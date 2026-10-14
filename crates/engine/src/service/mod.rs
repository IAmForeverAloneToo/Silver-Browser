//! The browser process's side of the renderers (M34, docs/design/
//! processes.md): starting them and holding them ([`process`]). What the
//! browser keeps for every renderer and checks each one against grows
//! here step by step: the network and storage services, the frame tree,
//! the session history, the compositor and the session's proxy.

mod network;
#[cfg(test)]
mod network_tests;
mod orb;
mod process;
#[cfg(test)]
mod tests;

pub use network::{FrameGrant, Grant, Grants, Lock, MAX_RENDERER_FETCHES, NetworkService};
pub use process::{
    Event, Gone, HELLO_TIMEOUT, Launched, Launcher, Renderer, RendererProcess, Serve,
    ThreadLauncher,
};
