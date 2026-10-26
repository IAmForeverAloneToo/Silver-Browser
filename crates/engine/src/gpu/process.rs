//! The GPU process's life (M41 step 1, D38): over its channel with the
//! browser (its standard input and output, or pipes in memory for a
//! thread), the hello first, then each client's batches handed to the
//! [`GpuService`] and its replies sent back. A batch the service refuses,
//! or one that panics in it, ends that client alone.

use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;

use super::service::GpuService;
use super::wire::{self, FromGpu, GPU_VERSION, ToGpu};
use crate::ipc::{self, Bytes, Sender};

/// Runs the GPU service over `input` and `output` until the browser
/// goes; Err for a browser that breaks the protocol.
pub fn run(
    input: impl Read + Send + 'static,
    output: impl Write + Send + 'static,
) -> Result<(), String> {
    let sender: Sender<FromGpu> = Sender::new(output);
    let (to_loop, inputs) = mpsc::channel::<Option<ToGpu>>();
    let closing = to_loop.clone();
    ipc::spawn_reader::<ToGpu, _>(
        "gpu-reader",
        input,
        move |message| {
            let _ = to_loop.send(Some(message));
        },
        move |_| {
            let _ = closing.send(None);
        },
    )
    .map_err(|error| format!("no reader thread: {error}"))?;
    let fallback_only = match inputs.recv() {
        Ok(Some(ToGpu::Hello {
            version,
            fallback_only,
        })) if version == GPU_VERSION => fallback_only,
        Ok(Some(ToGpu::Hello { version, .. })) => {
            return Err(format!("a hello of version {version}, not {GPU_VERSION}"));
        }
        Ok(Some(message)) => return Err(format!("{message:?} before the hello")),
        Ok(None) | Err(_) => return Ok(()),
    };
    sender
        .send(&FromGpu::Hello {
            version: GPU_VERSION,
        })
        .map_err(|_| "the browser is gone".to_owned())?;
    let mut service = GpuService::new(fallback_only);
    while let Ok(Some(message)) = inputs.recv() {
        let mut answers = Vec::new();
        match message {
            ToGpu::Client { client, bytes } => {
                let handled = catch_unwind(AssertUnwindSafe(|| service.handle(client, &bytes.0)));
                match handled {
                    Ok(Ok(replies)) => answers.extend(batches(client, replies)),
                    Ok(Err(why)) => {
                        service.gone(client);
                        answers.push(FromGpu::Bad { client, why });
                    }
                    Err(_) => {
                        service.gone(client);
                        answers.push(FromGpu::Bad {
                            client,
                            why: "its request panicked the GPU service".to_owned(),
                        });
                    }
                }
            }
            ToGpu::Gone { client } => service.gone(client),
            ToGpu::Hello { .. } => return Err("a second hello".to_owned()),
        }
        // What the devices told meanwhile, each to its client.
        for (client, event) in service.events() {
            answers.extend(batches(client, vec![event]));
        }
        for answer in answers {
            if sender.send(&answer).is_err() {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// `replies` for `client`, in messages of bounded size.
fn batches(client: u32, replies: Vec<wire::Reply>) -> impl Iterator<Item = FromGpu> {
    wire::reply_batches(replies)
        .into_iter()
        .map(move |bytes| FromGpu::Client {
            client,
            bytes: Bytes(bytes),
        })
}
