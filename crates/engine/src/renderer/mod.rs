//! A renderer (M34.1, docs/design/processes.md): the engines of one
//! site, run for the browser over a channel. [`run`] is a renderer's
//! whole life: the shell's `--renderer` gives it the process's standard
//! input and output, and the engine's tests a thread's pipes. It
//! answers the browser's hello, numbers its opaque origins from the
//! space its number gives, and serves the browser's orders until it is
//! told to end or its input ends.

use std::io::{Read, Write};
use std::sync::mpsc;

use crate::ipc::{self, IPC_VERSION, Sender, ToBrowser, ToRenderer};

/// Where renderer `process`'s opaque origins are numbered from: each
/// process its own space of 2^40, the browser's the first.
pub fn opaque_origins_of(process: u32) -> u64 {
    (u64::from(process) + 1) << 40
}

/// What the renderer's reader thread hands its main thread.
enum Input {
    Message(ToRenderer),
    /// The browser's channel ended, or said what is no message.
    Closed(Option<String>),
}

/// Runs a renderer over `input`, what the browser writes, and `output`,
/// what it reads, until the browser ends it. An error says why it ended
/// otherwise: a hello of another version, a message out of place, or a
/// channel that failed.
pub fn run(
    input: impl Read + Send + 'static,
    output: impl Write + Send + 'static,
) -> Result<(), String> {
    let sender = Sender::<ToBrowser>::new(output);
    let (queue, inputs) = mpsc::channel();
    let closing = queue.clone();
    ipc::spawn_reader::<ToRenderer, _>(
        "renderer-reader",
        input,
        move |message| {
            let _ = queue.send(Input::Message(message));
        },
        move |reason| {
            let _ = closing.send(Input::Closed(reason.map(|reason| reason.to_string())));
        },
    )
    .map_err(|error| format!("no reader thread: {error}"))?;

    match inputs.recv() {
        Ok(Input::Message(ToRenderer::Hello { version, process })) if version == IPC_VERSION => {
            net::start_opaque_origins_at(opaque_origins_of(process));
            sender
                .send(&ToBrowser::Hello {
                    version: IPC_VERSION,
                })
                .map_err(|_| "the browser is gone".to_owned())?;
        }
        Ok(Input::Message(ToRenderer::Hello { version, .. })) => {
            return Err(format!("a hello of version {version}, not {IPC_VERSION}"));
        }
        Ok(Input::Message(message)) => return Err(format!("{message:?} before the hello")),
        Ok(Input::Closed(None)) | Err(_) => return Ok(()),
        Ok(Input::Closed(Some(why))) => return Err(why),
    }

    // A renderer with nothing to show waits for its end.
    match inputs.recv() {
        Ok(Input::Message(ToRenderer::Shutdown {})) => Ok(()),
        Ok(Input::Message(ToRenderer::Hello { .. })) => Err("a second hello".into()),
        Ok(Input::Closed(None)) | Err(_) => Ok(()),
        Ok(Input::Closed(Some(why))) => Err(why),
    }
}
