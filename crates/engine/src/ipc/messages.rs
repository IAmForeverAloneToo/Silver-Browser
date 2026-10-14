//! The messages between the browser and a renderer (M34.2): what the
//! browser sends a renderer, [`ToRenderer`], and what a renderer sends
//! the browser, [`ToBrowser`]. A variant's tag never changes meaning
//! within an [`IPC_VERSION`](super::IPC_VERSION).

use super::codec::wire_enum;

wire_enum! {
    /// What the browser sends a renderer.
    #[derive(Debug, Clone, PartialEq)]
    pub enum ToRenderer {
        /// The first message: the schema's version; the renderer's
        /// number, which also numbers its opaque origins apart from every
        /// other process's.
        0 => Hello { version: u32, process: u32 },
        /// The renderer ends.
        1 => Shutdown {},
    }
}

wire_enum! {
    /// What a renderer sends the browser.
    #[derive(Debug, Clone, PartialEq)]
    pub enum ToBrowser {
        /// The answer to the hello: the renderer's schema version.
        0 => Hello { version: u32 },
    }
}
