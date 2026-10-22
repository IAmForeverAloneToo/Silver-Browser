//! The messages between the browser and a renderer (M34.2): what the
//! browser sends a renderer, [`ToRenderer`], and what a renderer sends
//! the browser, [`ToBrowser`]. A variant's tag never changes meaning
//! within an [`IPC_VERSION`](super::IPC_VERSION).

use net::cookies::Context;
use net::csp::Violation;
use net::{NetworkError, Origin, Partition, Request, Response, Url};

use super::codec::{Bytes, encode, wire_enum, wire_struct};
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
        /// The view navigates to `url`, whose response the browser holds
        /// as `handover`: its document's fetch asks for that one.
        18 => NavigateHandedOver { view: u32, url: Url, handover: u64 },
        /// The view's history drops its entries after the current one.
        19 => DropForward { view: u32 },
        /// The view's history is `entries`, the one at `index` current
        /// and loaded: a segment restored.
        20 => Restore { view: u32, entries: Vec<Url>, index: u32 },
        /// A view of the frame the browser numbers `key`, whose parent's
        /// document another renderer shows (M34): its ancestors, the parent
        /// first, and the sandboxing flags of its documents.
        21 => CreateFrameView {
            view: u32,
            key: u32,
            settings: Settings,
            ancestors: Vec<Ancestor>,
            sandbox: u32,
        },
        /// The frame the browser numbers `key`, of one of the renderer's
        /// pages, is shown by another renderer from now on.
        22 => FrameRemote { key: u32 },
        /// The frame of another renderer the browser numbers `key` loaded a
        /// document: its element hears `load`.
        23 => FrameLoaded { key: u32 },
        /// What frame `from` (the browser's number, if one) posted to frame
        /// `to`, one of the renderer's.
        24 => FrameTask { from: Option<u32>, to: u32, task: WireTask },
        /// The files of the fonts the browser found (M35.6,
        /// `text::FontFiles`), which the renderer reads rather than
        /// scanning the system's: sent after the hello, before any view.
        25 => Fonts { primary: (String, u32), fallbacks: Vec<(u8, (String, u32))> },
        /// The view, the DevTools panel's (M36), hears `message` from the
        /// browser.
        26 => DevTools { view: u32, message: String },
        /// The view's page is asked the panel's request `message` (M36).
        27 => Inspect { view: u32, message: String },
        /// A Web Storage area of `origin` in `partition` changed in
        /// another renderer (M37): `localStorage`'s if `local`, else the
        /// tab's `sessionStorage`'s; the renderer's documents of the area
        /// hear of it in `storage` events.
        28 => StorageChanged {
            local: bool,
            origin: Origin,
            partition: Option<Partition>,
            key: Option<String>,
            old: Option<String>,
            new: Option<String>,
            url: String,
        },
        /// The view reloads, bypassing the HTTP cache (M38).
        29 => ReloadBypassingCache { view: u32 },
        /// What the service workers tell the document `page` of frame
        /// `frame` (M39): a task of `js::Task::ServiceWorker`.
        30 => ServiceWorkerTask { frame: u32, page: u64, task: String },
        /// A worker renderer runs `version` (M39, D35) as frame `frame`:
        /// evaluated for its job if `evaluate`, else started again from
        /// its scripts kept (`stored`); `own` is its registration's view
        /// and its own, as the bindings read them.
        31 => StartServiceWorker {
            version: u64,
            frame: u32,
            registration: u64,
            key: String,
            scope: Box<Url>,
            script: Box<Url>,
            source: Bytes,
            stored: Vec<(Url, Bytes)>,
            own: String,
            partition: Option<Partition>,
            evaluate: bool,
        },
        /// The browser's extendable event `event` of type `kind` for
        /// `version`.
        32 => ServiceWorkerEvent { version: u64, event: u64, kind: String },
        /// The running versions of storage key `key` hear `task`.
        33 => ServiceWorkerNotice { key: String, task: String },
        /// `version` stops.
        34 => StopServiceWorker { version: u64 },
    }
}

wire_struct! {
    /// A frame root's ancestor (M34): the browser's number for it, and its
    /// document's origin and URL as far as the browser knows them.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Ancestor {
        pub key: u32,
        pub origin: Origin,
        pub url: Option<Url>,
    }
}

wire_struct! {
    /// Where a frame of another renderer is in a view's frame (M34): the
    /// browser's number for it, and its content box in the view's raster
    /// pixels.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Hole {
        pub key: u32,
        pub x: i32,
        pub y: i32,
        pub width: u32,
        pub height: u32,
    }
}

wire_enum! {
    /// What one frame asks of another of another renderer (M34): a message
    /// (its data serialized, the sender's origin, the target origin it was
    /// posted for), or a navigation.
    #[derive(Debug, Clone)]
    pub enum WireTask {
        0 => Message { data: String, origin: String, target_origin: String },
        1 => Navigate {
            url: Box<Url>,
            replace: bool,
            post: Option<(Bytes, String)>,
            origin: Origin,
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
        /// [`ToBrowser::Fetch`] of the response the browser holds as
        /// `handover` for this frame.
        13 => FetchHandedOver {
            id: u64,
            frame: u32,
            handover: u64,
            request: Box<Request>,
        },
        /// The view's history: its entries' URLs and the current one.
        14 => History { view: u32, entries: Vec<Url>, index: u32 },
        /// A number for a new frame under the renderer's frame `parent`
        /// (M34).
        15 => NewFrame { call: u64, parent: u32 },
        /// What the renderer's frame `from` posted to frame `to` of another
        /// renderer.
        16 => PostFrame { from: Option<u32>, to: u32, task: WireTask },
        /// The frame of another renderer the browser numbers `key` left
        /// the renderer's page, or became its again: its view goes.
        17 => FrameDropped { key: u32 },
        /// Where the frames of other renderers are in the view's frame
        /// `frame`, sent before it.
        18 => Holes { view: u32, frame: u64, holes: Vec<Hole> },
        /// The sandboxing flags the next document of the renderer's frame
        /// `key` is made with, as its parent says.
        19 => FramePolicy { key: u32, sandbox: u32 },
        /// What the view, the DevTools panel's (M36), sent the browser.
        20 => DevTools { view: u32, message: String },
        /// What the view's page answered the panel, or told it (M36).
        21 => Inspected { view: u32, message: String },
        /// Frame `frame`'s document changed a Web Storage area of `origin`
        /// in `partition` (M37), for the browser to tell the other
        /// renderers: `localStorage`'s if `local`, else `sessionStorage`'s.
        22 => StorageChanged {
            frame: u32,
            local: bool,
            origin: Origin,
            partition: Option<Partition>,
            key: Option<String>,
            old: Option<String>,
            new: Option<String>,
            url: String,
        },
        /// The view's documents asked for a tab of `url` (M37), shown at once
        /// if `show`, by a script if `by_script`.
        23 => OpenTab { view: u32, url: Url, show: bool, by_script: bool },
        /// The view's top-level document asked to close its tab (M37).
        24 => CloseTab { view: u32 },
        /// Frame `frame`'s document follows a link with a `download`
        /// attribute to `url` (M37): the browser saves it, under `name`
        /// when the response suggests none.
        25 => Download { frame: u32, url: Url, name: Option<String> },
        /// What `version`'s script did when it was evaluated (M39): the
        /// events it listens for and the scripts it imported, or none when
        /// it threw or did not run.
        26 => ServiceWorkerEvaluated {
            version: u64,
            listened: Option<Vec<String>>,
            imports: Vec<(Url, Bytes)>,
        },
        /// The browser's event `event` of `version` ended, its promises
        /// all fulfilled if `ok`; not `ran` when the version was not
        /// running, for the browser to start it again.
        27 => ServiceWorkerEventDone { version: u64, event: u64, ok: bool, ran: bool },
        /// `skipWaiting()` of `version`.
        28 => ServiceWorkerSkipWaiting { version: u64 },
        /// `version` stopped, idle or past an event's limit.
        29 => ServiceWorkerStopped { version: u64 },
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
        /// IndexedDB's request of the storage key's databases (M36.7), as
        /// `crate::idb::serve` reads it.
        7 => Idb { request: String },
        /// The bytes the storage key uses (M36.8), answered as a length.
        8 => Usage {},
        /// The Cache API's request of the storage key's caches (M39), the
        /// bodies it carries in `bytes`, as `crate::caches::serve` reads
        /// it.
        9 => Caches { request: String, bytes: Bytes },
        /// A request of the service workers' (M39) of document `page`, of
        /// `url`, its own URLs parsed against `base`, as
        /// `crate::service_worker::page::serve` reads it.
        10 => ServiceWorker { request: String, url: Box<Url>, base: Box<Url>, page: u64 },
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
        /// The browser's number for a new frame, if it gives one.
        6 => Frame { key: Option<u32> },
        /// IndexedDB's answer (M36.7).
        7 => Idb { answer: String },
        /// The Cache API's answer (M39), the bodies it gives in `bytes`.
        8 => Caches { answer: String, bytes: Bytes },
        /// The service workers' answer (M39).
        9 => ServiceWorker { answer: String },
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

equal_by_encoding!(ToRenderer, ToBrowser, Answer, StorageOp, WireTask);

impl WireTask {
    /// `task` on the wire: a message's source goes apart, as the browser's
    /// number of its frame.
    pub(crate) fn from_task(task: crate::frames::FrameTask) -> WireTask {
        match task {
            crate::frames::FrameTask::Message {
                data,
                origin,
                target_origin,
                ..
            } => WireTask::Message {
                data,
                origin,
                target_origin,
            },
            crate::frames::FrameTask::Navigate(navigation) => WireTask::Navigate {
                url: Box::new(navigation.url),
                replace: navigation.replace,
                post: navigation
                    .post
                    .map(|post| (Bytes(post.body), post.content_type)),
                origin: navigation.origin,
            },
        }
    }

    /// The task for a frame, its message from `source`, the frame of the
    /// receiving tree standing for the sender.
    pub(crate) fn into_task(self, source: crate::frames::FrameId) -> crate::frames::FrameTask {
        match self {
            WireTask::Message {
                data,
                origin,
                target_origin,
            } => crate::frames::FrameTask::Message {
                data,
                origin,
                source,
                target_origin,
            },
            WireTask::Navigate {
                url,
                replace,
                post,
                origin,
            } => crate::frames::FrameTask::Navigate(Box::new(crate::frames::FrameNavigation {
                url: *url,
                replace,
                post: post.map(|(body, content_type)| crate::submit::Post {
                    body: body.0,
                    content_type,
                }),
                origin,
            })),
        }
    }
}
