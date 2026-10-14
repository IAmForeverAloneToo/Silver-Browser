//! The messages between the browser and a renderer (M34.2): what the
//! browser sends a renderer, [`ToRenderer`], and what a renderer sends
//! the browser, [`ToBrowser`]. A variant's tag never changes meaning
//! within an [`IPC_VERSION`](super::IPC_VERSION).

use net::cookies::Context;
use net::csp::Violation;
use net::{NetworkError, Request, Response, Url};

use super::codec::{Bytes, encode, wire_enum};

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
    }
}

wire_enum! {
    /// The browser's answer to a question a renderer waits on.
    #[derive(Debug)]
    pub enum Answer {
        /// `document.cookie`'s cookies, if any.
        0 => Cookies { cookies: Option<Bytes> },
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

equal_by_encoding!(ToRenderer, ToBrowser, Answer);
