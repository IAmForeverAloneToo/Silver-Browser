//! The browser process's side of the renderers (M34, docs/design/
//! processes.md): starting them and holding them ([`process`]). What the
//! browser keeps for every renderer and checks each one against grows
//! here step by step: the network and storage services, the frame tree,
//! the session history, the compositor and the session's proxy.

mod handover;
mod network;
#[cfg(test)]
mod network_tests;
mod orb;
mod process;
#[cfg(test)]
mod sites_tests;
mod storage;
#[cfg(test)]
mod storage_tests;
mod tab;
#[cfg(test)]
mod tab_tests;
#[cfg(test)]
mod tests;

pub use handover::{HANDOVER_TIMEOUT, HandoverNotice, Handovers, MAX_HANDOVER_BUFFER};
pub use network::{FrameGrant, Grant, Grants, Lock, MAX_RENDERER_FETCHES, NetworkService};
pub use process::{
    Event, Gone, HELLO_TIMEOUT, Launched, Launcher, Renderer, RendererProcess, Serve, Services,
    ThreadLauncher,
};
pub use storage::StorageService;
pub use tab::{MAX_RENDERERS, RemoteTab};
