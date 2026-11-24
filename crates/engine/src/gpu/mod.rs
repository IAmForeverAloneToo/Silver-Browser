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

/// Whether a GPU service here has an adapter, as a page's would: the
/// tests needing one say so and pass where there is none (CI's Windows
/// runners). False without the `gpu` feature.
pub fn adapter_here() -> bool {
    #[cfg(feature = "gpu")]
    {
        service::GpuService::new(false).has_adapter()
    }
    #[cfg(not(feature = "gpu"))]
    {
        false
    }
}

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

    /// What the service's thread takes.
    enum Job {
        /// A batch of a client's, with where its replies go.
        Batch {
            client: u32,
            bytes: Vec<u8>,
            to: Weak<GpuClient>,
        },
        /// A client gone: its objects go, its device with them (a
        /// renderer's are told so by the browser as the renderer goes).
        Gone(u32),
        /// How many devices the service holds (the tests').
        #[cfg(test)]
        Devices(mpsc::Sender<usize>),
    }

    /// The service's thread, started at the first client.
    fn service() -> &'static Mutex<mpsc::Sender<Job>> {
        static SERVICE: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
        SERVICE.get_or_init(|| {
            let (sender, jobs) = mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("gpu-service".to_owned())
                .spawn(move || {
                    let service = GpuService::new(false);
                    service.warm();
                    run(service, &jobs);
                })
                .expect("the GPU service's thread");
            Mutex::new(sender)
        })
    }

    /// The service's loop: each job in turn, until no client can send.
    fn run(mut service: GpuService, jobs: &mpsc::Receiver<Job>) {
        // Where each client's replies go, for its devices' news too.
        let mut clients: HashMap<u32, Weak<GpuClient>> = HashMap::new();
        while let Ok(job) = jobs.recv() {
            let (client, bytes, to) = match job {
                Job::Batch { client, bytes, to } => (client, bytes, to),
                Job::Gone(client) => {
                    service.gone(client);
                    clients.remove(&client);
                    continue;
                }
                #[cfg(test)]
                Job::Devices(answer) => {
                    let _ = answer.send(service.devices());
                    continue;
                }
            };
            clients.insert(client, to.clone());
            let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                service.handle(client, &bytes)
            }));
            let Some(to) = to.upgrade() else {
                service.gone(client);
                clients.remove(&client);
                continue;
            };
            match handled {
                Ok(Ok(replies)) => {
                    for batch in wire::reply_batches(replies) {
                        let _ = to.deliver(&batch);
                    }
                }
                Ok(Err(why)) => {
                    service.gone(client);
                    to.lose(why);
                }
                Err(_) => {
                    service.gone(client);
                    to.lose("its request panicked the GPU service".to_owned());
                }
            }
            for (client, event) in service.events() {
                if let Some(to) = clients.get(&client).and_then(Weak::upgrade) {
                    let _ = to.deliver(&wire::replies(&[event]));
                }
            }
        }
    }

    pub(super) fn client() -> Arc<GpuClient> {
        // The service started with its first client, not its first
        // request: making its instance (the backends loaded, more than a
        // second here) and its adapter go on while the page loads.
        let _ = service();
        static CLIENTS: AtomicU32 = AtomicU32::new(1);
        let number = CLIENTS.fetch_add(1, Ordering::Relaxed);
        client_of(number, |job| {
            service()
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .send(job)
                .is_ok()
        })
    }

    /// Client `number` of the service `send` reaches; its end tells the
    /// service so.
    fn client_of(
        number: u32,
        send: impl Fn(Job) -> bool + Clone + Send + Sync + 'static,
    ) -> Arc<GpuClient> {
        let end = send.clone();
        Arc::new_cyclic(|this: &Weak<GpuClient>| {
            let to = this.clone();
            GpuClient::with_end(
                Box::new(move |bytes| {
                    let job = Job::Batch {
                        client: number,
                        bytes,
                        to: to.clone(),
                    };
                    if !send(job)
                        && let Some(to) = to.upgrade()
                    {
                        to.lose("the GPU service is gone".to_owned());
                    }
                }),
                move || {
                    end(Job::Gone(number));
                },
            )
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::webgl::Harness;

        #[test]
        fn a_gone_clients_shared_device_goes_with_it() {
            // CI's fuzzing suppresses leaks inside Mesa's software Vulkan
            // (fuzz/lsan-suppressions.txt), which a device of ours never
            // dropped would hide behind: the devices are counted here.
            if !super::super::adapter_here() {
                eprintln!("no adapter here: the devices are not counted");
                return;
            }
            let (sender, jobs) = mpsc::channel::<Job>();
            let thread = std::thread::spawn(move || run(GpuService::new(false), &jobs));
            let sender = Arc::new(Mutex::new(sender));
            let send = {
                let sender = sender.clone();
                move |job| {
                    sender
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .send(job)
                        .is_ok()
                }
            };
            let devices = || {
                let (answer, count) = mpsc::channel();
                sender.lock().unwrap().send(Job::Devices(answer)).unwrap();
                count.recv().unwrap()
            };
            assert_eq!(devices(), 0);
            for number in 1..=3 {
                let client = client_of(number, send.clone());
                let mut h = Harness::new(client.clone());
                assert!(h.create(1), "client {number}: no context");
                h.call("createBuffer", &[], Vec::new(), None);
                assert!(h.create(2), "client {number}: no second context");
                // Two contexts, one device: the client's.
                assert_eq!(devices(), 1, "client {number}");
                // Its contexts gone, the client keeps its device for the
                // next it makes (one a client, never more).
                h.call("loseContext", &[], Vec::new(), None);
                drop(h);
                assert_eq!(devices(), 1, "client {number}");
                // The client gone: the device goes.
                drop(client);
                assert_eq!(devices(), 0, "client {number}");
            }
            drop(send);
            drop(sender);
            thread.join().unwrap();
        }
    }
}
