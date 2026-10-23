//! A renderer (M34.1, docs/design/processes.md): the engines of one
//! site, run for the browser over a channel. [`run`] is a renderer's
//! whole life: the shell's `--renderer` gives it the process's standard
//! input and output, and the engine's tests a thread's pipes. It
//! answers the browser's hello, numbers its opaque origins from the
//! space its number gives, and serves the browser's orders until it is
//! told to end or its input ends.

use std::io::{Read, Write};
use std::sync::{Arc, mpsc};

use crate::ipc::{self, IPC_VERSION, Sender, ToBrowser, ToRenderer};

mod host;
pub(crate) mod link;
mod workers;

use host::Host;
pub use host::MAX_SURFACE_PIXELS;
use link::Link;

/// The stack of the thread that compiles the bindings as a renderer
/// starts: room past the 1 MiB QuickJS's parser may use of it.
const BINDINGS_STACK: usize = 8 << 20;

/// Where renderer `process`'s opaque origins are numbered from: each
/// process its own space of 2^40, the browser's the first.
pub fn opaque_origins_of(process: u32) -> u64 {
    (u64::from(process) + 1) << 40
}

/// What the renderer's main thread is woken by.
/// The renderer's audio sink: its blocks sent to the browser, what was
/// played as the browser says.
pub(crate) struct BrowserSink(pub(crate) Arc<Link>);

impl crate::AudioSink for BrowserSink {
    fn write(&self, samples: &[f32]) {
        self.0.audio(samples);
    }

    fn played(&self) -> u64 {
        self.0.audio_played()
    }
}

enum Input {
    Message(Box<ToRenderer>),
    /// The browser's channel ended, or said what is no message.
    Closed(Option<String>),
    /// An engine has something to take: a fetch's result, a worker's
    /// message.
    Wake,
}

/// Runs a renderer over `input`, what the browser writes, and `output`,
/// what it reads, until the browser ends it. An error says why it ended
/// otherwise: a hello of another version, a message out of place, or a
/// channel that failed.
pub fn run(
    input: impl Read + Send + 'static,
    output: impl Write + Send + 'static,
) -> Result<(), String> {
    run_in(input, output, None, true)
}

/// [`run`], its views drawing with `fonts` rather than the system's: the
/// tests' bundled font.
pub fn run_with(
    input: impl Read + Send + 'static,
    output: impl Write + Send + 'static,
    fonts: Option<text::FontCollection>,
) -> Result<(), String> {
    run_in(input, output, fonts, false)
}

/// [`run_with`]; with `sound`, the process's sound goes to the browser
/// (a renderer's own process: in a thread, as the tests run renderers,
/// the process's mixer is not the renderer's to take).
fn run_in(
    input: impl Read + Send + 'static,
    output: impl Write + Send + 'static,
    fonts: Option<text::FontCollection>,
    sound: bool,
) -> Result<(), String> {
    // The bindings compiled while the renderer waits for its first page
    // (M35.12): every realm it makes reads them; one asked for first
    // waits for the compile rather than compiling again.
    let _ = std::thread::Builder::new()
        .name("renderer-bindings".to_owned())
        .stack_size(BINDINGS_STACK)
        .spawn(js::warm_bindings);
    let sender = Sender::<ToBrowser>::new(output);
    let link = Link::new(sender.clone());
    let (queue, inputs) = mpsc::channel();
    let closing = queue.clone();
    let waking = queue.clone();
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

    let first = loop {
        match inputs.recv() {
            Ok(Input::Message(message)) => break *message,
            Ok(Input::Wake) => {}
            Ok(Input::Closed(None)) | Err(_) => return Ok(()),
            Ok(Input::Closed(Some(why))) => return Err(why),
        }
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

    // The renderer's sound goes to the browser (M40, D37).
    if sound {
        crate::audio_output::set_audio_sink(Arc::new(BrowserSink(link.clone())));
    }
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let _ = waking.send(Input::Wake);
    });
    let mut host = Host::new(link, fonts, wake);
    loop {
        let first = match host.wait() {
            Some(wait) => match inputs.recv_timeout(wait) {
                Ok(input) => Some(input),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            },
            None => match inputs.recv() {
                Ok(input) => Some(input),
                Err(_) => return Ok(()),
            },
        };
        // What came meanwhile is taken before the views' turn.
        for input in first
            .into_iter()
            .chain(std::iter::from_fn(|| inputs.try_recv().ok()))
        {
            match input {
                Input::Message(message) => {
                    if !host.handle(*message)? {
                        return Ok(());
                    }
                }
                Input::Wake => {}
                Input::Closed(None) => return Ok(()),
                Input::Closed(Some(why)) => return Err(why),
            }
        }
        host.turn();
    }
}
