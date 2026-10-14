//! The messages between the browser and a renderer (M34.2): what the
//! browser sends a renderer, [`ToRenderer`], and what a renderer sends
//! the browser, [`ToBrowser`]. A variant's tag never changes meaning
//! within an [`IPC_VERSION`](super::IPC_VERSION).

use net::cookies::Context;
use net::csp::Violation;
use net::{NetworkError, Origin, Partition, Request, Response, Url};

use super::codec::{Bytes, encode, wire_enum};
use super::view_wire::{Input, Settings, ViewState};
use crate::ConsoleEntry;

wire_enum! {
    /// What the browser sends a renderer.
    #[derive(Debug)]
    pub enum ToRenderer {
        /// The first message: the schema's version; the renderer's
        /// number, which also numbers its opaque origins apart from every
        /// other process's.
        0 => Hello { version: u32, process: u32 },
        /// The renderer ends.
        1 => Shutdown {},
        /// The answer to the question of that number.
        2 => Answer { call: u64, answer: Answer },
        /// The head of fetch `id`'s response, its body to come.
        3 => FetchHead { id: u64, response: Response },
        /// A piece of fetch `id`'s body, of at most `MAX_CHUNK` bytes.
        4 => FetchBody { id: u64, bytes: Bytes },
        /// Fetch `id`'s end: its response's head, or why it failed; and
        /// the violations of its policies.
        5 => FetchDone {
            id: u64,
            result: Result<Response, NetworkError>,
            violations: Vec<Violation>,
        },
        /// A view of the frame the browser numbered `frame`, shown under
        /// `settings`: an engine showing `about:blank`.
        6 => CreateView { view: u32, frame: u32, settings: Settings },
        /// The view goes.
        7 => DestroyView { view: u32 },
        /// The view navigates to `url`.
        8 => Navigate { view: u32, url: Url },
        /// The view's history goes `delta` entries back or forward.
        9 => Traverse { view: u32, delta: i32 },
        10 => Reload { view: u32 },
        11 => Stop { view: u32 },
        /// The view's viewport, in CSS pixels, and the size its frames are
        /// drawn at, in raster pixels.
        12 => Resize {
            view: u32,
            width: u32,
            height: u32,
            raster_width: u32,
            raster_height: u32,
        },
        /// The settings the view is shown under now.
        13 => Settings { view: u32, settings: Settings },
        /// Input given to the view.
        14 => Input { view: u32, input: Input },
        /// The browser took the view's frame `frame`: the next may come.
        15 => FrameTaken { view: u32, frame: u64 },
        /// The view's selection is wanted, to be copied, or cut once the
        /// clipboard has it.
        16 => Copy { view: u32, cut: bool },
        /// The view's selection is cut.
        17 => Cut { view: u32 },
    }
}

wire_enum! {
    /// What a renderer sends the browser.
    #[derive(Debug)]
    pub enum ToBrowser {
        /// The answer to the hello: the renderer's schema version.
        0 => Hello { version: u32 },
        /// Fetches `request` for the frame the browser numbered `frame`;
        /// `id` is the renderer's number for the fetch. A streamed fetch's
        /// body comes as it arrives.
        1 => Fetch { id: u64, frame: u32, streamed: bool, request: Box<Request> },
        /// The renderer gave up on fetch `id`.
        2 => CancelFetch { id: u64 },
        /// `document.cookie` of frame `frame`'s document, of `url`.
        3 => Cookies { call: u64, frame: u32, url: Url, context: Context },
        /// `document.cookie = value` of frame `frame`'s document.
        4 => SetCookie { frame: u32, url: Url, value: Bytes, context: Context },
        /// An operation on a Web Storage area of frame `frame`'s document,
        /// of `origin` in `partition`: `localStorage`'s if `local`, else
        /// `sessionStorage`'s.
        5 => Storage {
            call: u64,
            frame: u32,
            local: bool,
            origin: Origin,
            partition: Option<Partition>,
            op: StorageOp,
        },
        /// Rows of the view's frame `frame`, `width` by `height`, from
        /// `first_row`: those that changed since the last frame.
        6 => Pixels {
            view: u32,
            frame: u64,
            width: u32,
            height: u32,
            first_row: u32,
            bytes: Bytes,
        },
        /// The view's frame `frame` is whole.
        7 => Frame { view: u32, frame: u64, width: u32, height: u32 },
        /// What the session reads of the view, changed.
        8 => State { view: u32, state: Box<ViewState> },
        /// What the view's scripts wrote to the console.
        9 => Console { view: u32, entries: Vec<ConsoleEntry> },
        /// A notice the view has for the user.
        10 => Notice { view: u32, text: String },
        /// The keyboard's focus left the view's page.
        11 => FocusLeft { view: u32 },
        /// The view's selection, copied or to be cut.
        12 => Copied { view: u32, text: Option<String>, cut: bool },
    }
}

wire_enum! {
    /// What a script does to a Web Storage area.
    #[derive(Debug)]
    pub enum StorageOp {
        0 => Length {},
        1 => Key { index: u64 },
        2 => Keys {},
        3 => Get { item: String },
        4 => Set { item: String, value: String },
        5 => Remove { item: String },
        6 => Clear {},
    }
}

wire_enum! {
    /// The browser's answer to a question a renderer waits on.
    #[derive(Debug)]
    pub enum Answer {
        /// `document.cookie`'s cookies, if any.
        0 => Cookies { cookies: Option<Bytes> },
        /// An area's number of items.
        1 => StorageLength { length: u64 },
        /// An item's value or key, if there is one.
        2 => StorageValue { value: Option<String> },
        /// An area's keys, in their order.
        3 => StorageKeys { keys: Vec<String> },
        /// What `setItem` did: whether the item was stored (not past the
        /// quota), and whether it changed, from `old`.
        4 => StorageSet { stored: bool, old: Option<String>, changed: bool },
        /// Whether `clear()` removed anything.
        5 => StorageCleared { cleared: bool },
    }
}

/// Messages are equal when their encodings are, which are canonical.
macro_rules! equal_by_encoding {
    ($($name:ty),*) => {$(
        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                encode(self) == encode(other)
            }
        }
    )*};
}

equal_by_encoding!(ToRenderer, ToBrowser, Answer, StorageOp);
