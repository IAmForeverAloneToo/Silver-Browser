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

// The renderer's engines use it from step 5 (M34.5); its tests now.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod link;

use link::Link;

/// Where renderer `process`'s opaque origins are numbered from: each
/// process its own space of 2^40, the browser's the first.
pub fn opaque_origins_of(process: u32) -> u64 {
    (u64::from(process) + 1) << 40
}

/// What the renderer's reader thread hands its main thread.
enum Input {
    Message(Box<ToRenderer>),
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
    let link = Link::new(sender.clone());
    let (queue, inputs) = mpsc::channel();
    let closing = queue.clone();
    let (routing, ending) = (link.clone(), link.clone());
    ipc::spawn_reader::<ToRenderer, _>(
        "renderer-reader",
        input,
        move |message| match routing.route(message) {
            Ok(Some(message)) => {
                let _ = queue.send(Input::Message(Box::new(message)));
            }
            Ok(None) => {}
            Err(why) => {
                let _ = queue.send(Input::Closed(Some(why)));
            }
        },
        move |reason| {
            ending.close();
            let _ = closing.send(Input::Closed(reason.map(|reason| reason.to_string())));
        },
    )
    .map_err(|error| format!("no reader thread: {error}"))?;

    let first = match inputs.recv() {
        Ok(Input::Message(message)) => *message,
        Ok(Input::Closed(None)) | Err(_) => return Ok(()),
        Ok(Input::Closed(Some(why))) => return Err(why),
    };
    match first {
        ToRenderer::Hello { version, process } if version == IPC_VERSION => {
            net::start_opaque_origins_at(opaque_origins_of(process));
            sender
                .send(&ToBrowser::Hello {
                    version: IPC_VERSION,
                })
                .map_err(|_| "the browser is gone".to_owned())?;
        }
        ToRenderer::Hello { version, .. } => {
            return Err(format!("a hello of version {version}, not {IPC_VERSION}"));
        }
        message => return Err(format!("{message:?} before the hello")),
    }

    // A renderer with nothing to show waits for its end.
    let next = match inputs.recv() {
        Ok(Input::Message(message)) => *message,
        Ok(Input::Closed(None)) | Err(_) => return Ok(()),
        Ok(Input::Closed(Some(why))) => return Err(why),
    };
    match next {
        ToRenderer::Shutdown {} => Ok(()),
        ToRenderer::Hello { .. } => Err("a second hello".into()),
        message => Err(format!("{message:?} out of place")),
    }
}
