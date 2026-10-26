//! WebGL and WebGPU's way to the GPU (M41, docs/design/gpu.md): the GPU
//! process and its protocols (D38), the service it runs, and the
//! clients that reach it, a renderer's through the browser, the browser's
//! own engines' in one process through a thread of the service.

pub mod client;
#[cfg(feature = "gpu")]
pub mod convert;
#[cfg(feature = "gpu")]
pub mod names;
#[cfg(feature = "gpu")]
pub mod process;
#[cfg(feature = "gpu")]
pub mod service;
pub mod wire;

#[cfg(test)]
mod tests;

use std::sync::Arc;

pub use client::GpuClient;

/// The memory the GPU process may commit, where its sandbox can limit
/// it: past it, its allocations fail.
pub const MAX_GPU_MEMORY: u64 = 4 << 30;

/// A new client of a GPU service in this process, run on a thread of its
/// own, started at the first client (one process, and the engine's
/// tests). `None` without the `gpu` feature.
pub fn local_client() -> Option<Arc<GpuClient>> {
    #[cfg(feature = "gpu")]
    {
        Some(local::client())
    }
    #[cfg(not(feature = "gpu"))]
    {
        None
    }
}

#[cfg(feature = "gpu")]
mod local {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak, mpsc};

    use super::client::GpuClient;
    use super::service::GpuService;
    use super::wire;

    /// A batch of a client's, with where its replies go.
    struct Job {
        client: u32,
        bytes: Vec<u8>,
        to: Weak<GpuClient>,
    }

    /// The service's thread, started at the first client.
    fn service() -> &'static Mutex<mpsc::Sender<Job>> {
        static SERVICE: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
        SERVICE.get_or_init(|| {
            let (sender, jobs) = mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("gpu-service".to_owned())
                .spawn(move || {
                    let mut service = GpuService::new(false);
                    // Where each client's replies go, for its devices'
                    // news too.
                    let mut clients: HashMap<u32, Weak<GpuClient>> = HashMap::new();
                    while let Ok(job) = jobs.recv() {
                        clients.insert(job.client, job.to.clone());
                        let handled =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                service.handle(job.client, &job.bytes)
                            }));
                        let Some(to) = job.to.upgrade() else {
                            service.gone(job.client);
                            clients.remove(&job.client);
                            continue;
                        };
                        match handled {
                            Ok(Ok(replies)) => {
                                for batch in wire::reply_batches(replies) {
                                    let _ = to.deliver(&batch);
                                }
                            }
                            Ok(Err(why)) => {
                                service.gone(job.client);
                                to.lose(why);
                            }
                            Err(_) => {
                                service.gone(job.client);
                                to.lose("its request panicked the GPU service".to_owned());
                            }
                        }
                        for (client, event) in service.events() {
                            if let Some(to) = clients.get(&client).and_then(Weak::upgrade) {
                                let _ = to.deliver(&wire::replies(&[event]));
                            }
                        }
                    }
                })
                .expect("the GPU service's thread");
            Mutex::new(sender)
        })
    }

    pub(super) fn client() -> Arc<GpuClient> {
        static CLIENTS: AtomicU32 = AtomicU32::new(1);
        let number = CLIENTS.fetch_add(1, Ordering::Relaxed);
        Arc::new_cyclic(|this: &Weak<GpuClient>| {
            let to = this.clone();
            GpuClient::new(Box::new(move |bytes| {
                let job = Job {
                    client: number,
                    bytes,
                    to: to.clone(),
                };
                let sent = service()
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .send(job);
                if sent.is_err()
                    && let Some(to) = to.upgrade()
                {
                    to.lose("the GPU service is gone".to_owned());
                }
            }))
        })
    }
}
