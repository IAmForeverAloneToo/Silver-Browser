//! The engine glue: document + pipeline orchestration.
//!
//! `Engine` owns the session state — the document, its stylesheets, the
//! scroll position (M9) and the render backend — and drives the
//! per-frame pipeline (`style` -> `layout` -> `paint` -> `render`).
//! Documents enter as HTML source through `html::parse_document` and
//! are held as a [`dom::Document`] (M13), whose nodes keep their
//! [`NodeId`]s; `<style>` element contents are parsed into
//! [`css::Stylesheet`]s and cascaded by `style::cascade_document` (M5)
//! into `style::Styles`; `layout::build_box_tree` +
//! `layout::layout_document` (M6, M9.1) resolve the document and its
//! styles into a real, positioned fragment tree in document
//! coordinates — actual box-model geometry (width/margin resolution,
//! margin collapsing), not a per-tag guess or a flat vertical list —
//! which `paint::paint_fragments` consumes each frame for the current
//! [`paint::Viewport`]. As the engine grows, this crate gains navigation
//! and resource orchestration (`net`). It owns no platform state:
//! windowing is `shell`'s job.
//!
//! The cascade is computed once per document and the layout once per
//! (document, viewport width) (M9.5): a scroll, or another frame at the
//! same width, reruns paint and raster only, and [`Engine::layout_runs`]
//! counts the layouts actually done, so a test can prove it. The
//! document changes in place through [`Engine::mutate`] (M13.5), which
//! reads the document's journal afterwards: a change is seen by the
//! next frame, which restyles the elements it reaches (M13.1) and lays
//! out the boxes it reaches, taking the others from the layout cache
//! (M13.2); [`Engine::boxes_laid_out`] and [`Engine::boxes_reused`]
//! count which. The rasterizer keeps the last frame and draws again
//! only where the new display list differs from the last one (M13.3);
//! [`Engine::redrawn`] says where. A frame is drawn for a time the
//! caller passes, [`Engine::frame_at`] (M13.4), which also gives it a
//! deadline; [`Engine::frame_stats`] says what each stage took and
//! whether it overran, and [`Engine::needs_frame`] whether one is
//! wanted. Scrolling (M9.2) is a target the shown offset approaches by
//! a fixed fraction of the distance every sixtieth of a second of that
//! time, however many frames share it, or every [`Engine::tick`] for a
//! caller without a clock; compositor-smooth scrolling is M19. The
//! scrollbar is
//! in-engine chrome (decision D9): drawn into the frame after the
//! document, its geometry exposed through [`Engine::scrollbar`] for the
//! shell to map presses and drags. Hit testing (M9.4) answers with the
//! [`NodeId`] of the element under a viewport point, the groundwork for
//! M26's events; [`Engine::link_at`] walks the document's parent links
//! up from it to the nearest `<a href>`.
//!
//! Elements scroll too (M10.5): a box whose `overflow` is `auto` or
//! `scroll` is a scroll container (a `hidden` one as well, by program
//! only), its offset kept here by the element's id and reset when a
//! document loads — layout knows nothing of it, paint moves the content
//! inside the clip, hit testing adds it on the way down. The wheel
//! ([`Engine::wheel`]) goes to the innermost container under the pointer
//! that can still move that way, then to its ancestors, then to the
//! viewport: scroll chaining. Each container animates as the viewport
//! does.
//!
//! Documents also come by URL (M14.6, docs/design/net.md, "The engine's
//! loader"). [`Engine::navigate`] fetches one through `net` on a thread
//! of its own, at most 32 fetches in flight; each result comes back over a
//! channel and calls the waker the shell gives with
//! [`Engine::set_waker`], and [`Engine::poll_network`] takes them on
//! the engine's thread. The page shown stays until the new document and
//! the stylesheets the parser met before its `<body>` have arrived or
//! failed, so a page first appears styled. A document's stylesheets
//! come from its `<link>` and `<style>` elements in tree order, each
//! with its `@import`s before it, and are found again when the journal
//! says an element that owns one changed; a sheet that arrives later
//! restyles the page. [`Engine::load_state`] says where
//! the latest load stands. A document given to the engine as source or
//! as a tree has no URL and fetches nothing.
//!
//! Navigation keeps a session history (M15, docs/design/navigation.md):
//! a new engine shows the initial `about:blank`, each navigation pushes
//! an entry or replaces the current one, and [`Engine::go_back`],
//! [`Engine::go_forward`], [`Engine::reload`] and [`Engine::stop`] act
//! on it as HTML's traversal, reload and "stop loading" do. A document
//! left with nothing on its way is kept whole, with its cascade, layout
//! and scroll offsets, by the back-forward cache of six, and shown again
//! without being fetched; any other is fetched again for its entry,
//! which is shown at its saved scroll position.
//!
//! CSS animations and transitions run on the document's timeline (M22,
//! docs/design/animations.md): its time is a frame's [`FrameTime::now`]
//! less that of the first frame drawn for a time while the document was
//! shown, [`Engine::tick`] moves it on by a frame for a caller without a
//! clock, and it never goes back. A frame starts the animations and
//! transitions made since the last one at its time and restyles the
//! elements whose animations give something new at it;
//! [`Engine::needs_frame`] is true while one plays, and
//! [`Engine::next_frame_at`] says when the end of a delay will want a
//! frame. The cascade keeps them (see `style::animation`), every change
//! of style, a change of the sheets or of the medium among them, a
//! change that transitions; a print takes them as they stand.

pub mod address;
pub mod differential;
mod focus;
mod image_store;
mod keys;
mod loader;
mod navigation;
mod pages;
mod print;
mod scripts;
#[cfg(test)]
mod testing;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

pub use dom::ElementState;
pub use image_store::MAX_DECODE_THREADS;
pub use js::ConsoleLevel;
pub use keys::{Key, KeyOutcome};
pub use loader::{LoadError, LoadState};
pub use net::{Client, NetworkError, Url};
pub use print::{PrintError, PrintOptions, PrintedPage};
pub use scripts::{
    ConsoleEntry, MAX_CONSOLE_ENTRIES, MAX_CONSOLE_MESSAGE, MAX_SCRIPT_BYTES, ScriptState,
};
pub use style::animation::{AnimationEvent, EventKind, Running as RunningAnimation};
pub use style::computed::Cursor;
pub use style::{ColorScheme, MediaType, Preferences};

use dom::{Change, Document, NodeId};
use image_store::{Decoder, ImageStore};
use layout::{DocumentLayout, Fragment, FragmentKind, LayoutCache};
use loader::{Incoming, Network, Page, PageId, Target};
use navigation::{History, StateId};
use paint::{
    DisplayItem, Rect, SCROLLBAR_THUMB, SCROLLBAR_TRACK, Step, Viewport, padding_box,
    paint_fragments_with, paint_order, scrollbar_thumb,
};
pub use render::Backend;
use render::{RenderBackend, SoftwareBackend};
use style::animation::Animations;
use style::computed::Overflow;
use style::{CascadeContext, FontMeasure, Media, Styles, Stylesheets};
use text::FontError;

/// How much of the remaining distance to the scroll target the scroll
/// animation covers in one frame of sixty a second, [`FRAME_BUDGET`]
/// (M9.2's lerp, measured in time since M13.4): a 1000px scroll settles
/// in about twenty such frames, a third of a second.
const SCROLL_TICK_FRACTION: f32 = 1.0 / 3.0;
/// Within this distance of the target, the animation lands on it.
const SCROLL_SNAP_PX: f32 = 0.5;

/// The stack a thread that runs the engine needs: a page nested as deep
/// as the parser lets (`dom::MAX_DEPTH`) is built into boxes and laid out
/// by recursion, about 7 KiB a level in release and 33 KiB in a debug
/// build. Windows gives a program's main thread 1 MiB, which a page of
/// 160 nested elements overflowed (found at M25); the shell's main thread
/// is given this much (its build script), and the engine's tests hold a
/// page at the cap to it.
pub const STACK_SIZE: usize = 32 << 20;

/// The clock the engine's tasks, timers and scripts read (M26,
/// `docs/design/events.md`): the time since a start of the caller's. The
/// shell gives the one its frames' times come from; tests give one they
/// move by hand.
pub type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;

/// How long [`Engine::run_tasks`] runs tasks before it leaves the rest
/// for its next call, so that a page's timers cannot keep the window
/// from drawing.
pub const TASK_SLICE: Duration = Duration::from_millis(50);

/// A mouse button (M26.5), by UI Events' `button` numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// The main button, usually the left.
    Primary,
    /// The middle button, or the wheel's.
    Auxiliary,
    /// The context menu's, usually the right.
    Secondary,
    Back,
    Forward,
}

impl Button {
    /// Its `button` number.
    fn number(self) -> i16 {
        match self {
            Button::Primary => 0,
            Button::Auxiliary => 1,
            Button::Secondary => 2,
            Button::Back => 3,
            Button::Forward => 4,
        }
    }

    /// Its bit in `buttons`.
    fn bit(self) -> u16 {
        match self {
            Button::Primary => 1,
            Button::Secondary => 2,
            Button::Auxiliary => 4,
            Button::Back => 8,
            Button::Forward => 16,
        }
    }
}

/// The modifier keys held (M26.5), which the pointer's and the keys'
/// events say.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

/// What a middle click on a link says: tabs are M37's.
pub const NO_NEW_TAB: &str = "Links open in this tab: there are no other tabs yet";

/// The least time between two timers of a hidden document (M26): its
/// timers run once a second at most, as the reference browser throttles
/// a hidden page's.
pub const HIDDEN_TIMER_PERIOD: Duration = Duration::from_secs(1);

/// The time a frame may take: one of sixty a second (M13.4,
/// `docs/perf-budgets.md`). The scroll animation's rate is given per
/// such a frame too.
pub const FRAME_BUDGET: Duration = Duration::from_nanos(16_666_667);

/// When a frame is drawn, on the caller's clock (M13.4): any clock that
/// does not go backwards, the times only compared with each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTime {
    /// The time the frame is drawn for: what it shows is advanced to it.
    pub now: Duration,
    /// When the frame should be done; one done later has overrun. Work
    /// that could wait for a later frame would stop at it, but there is
    /// none yet (see `docs/perf-budgets.md`).
    pub deadline: Duration,
}

impl FrameTime {
    /// A frame drawn for `now`, given [`FRAME_BUDGET`].
    pub fn at(now: Duration) -> Self {
        FrameTime {
            now,
            deadline: now + FRAME_BUDGET,
        }
    }
}

/// What a frame took, stage by stage (M13.4), on the engine's own clock,
/// which it reads for nothing else.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Bringing the computed styles up to date: a restyle, a whole
    /// cascade, or nothing.
    pub style: Duration,
    /// Bringing the layout up to date.
    pub layout: Duration,
    /// Building the display list.
    pub paint: Duration,
    /// Rasterizing it, until its pixels are done.
    pub raster: Duration,
    /// The whole frame: the stages and what holds them together.
    pub total: Duration,
    /// Whether the frame took longer than its deadline allowed.
    pub overran: bool,
}
/// The scrollbar track's width, in px, at the viewport's right edge.
pub const SCROLLBAR_WIDTH: f32 = 12.0;

/// A browsing session: the document shown, the session history and the
/// documents it keeps, the network, and the rasterizer.
pub struct Engine {
    /// The rasterizer (M19.3, roadmap D3): the software backend unless
    /// the session was made on the GPU's.
    backend: Box<dyn RenderBackend>,
    /// Loaded once and reused every frame — `text::FontCollection::new()`
    /// runs real system-font queries plus file reads/parses (the primary
    /// font and, since M8.1, its per-script fallbacks), not something to
    /// redo per frame. The backend holds a clone of the same collection
    /// (a `FontCollection` clones by sharing its face data).
    layout_fonts: text::FontCollection,
    /// The document shown, with everything made from it.
    shown: Shown,
    /// The session history (M15): its entries, the current one, whose
    /// document is the one shown, and the documents kept for others.
    history: History<Box<Shown>>,
    /// The fetches, the threads they run on and the client they share.
    network: Network,
    /// The navigation whose document is not shown yet, if one is under
    /// way.
    navigation: Option<Navigation>,
    load_state: LoadState,
    /// The waker the caller gave, kept to wake the caller while a parse
    /// has more to do (M24.8).
    waker: Option<loader::Waker>,
    /// Whether requests and `navigator` give D12's compatibility user
    /// agent string (M25).
    compat_user_agent: bool,
    /// Whether scripts run: off unless asked (M24, risk R2).
    scripting: bool,
    /// What a document's realm may use, the defaults but in tests.
    script_limits: js::Limits,
    /// What the user should be told of a navigation that showed no page:
    /// a download not saved (M15).
    notice: Option<String>,
    /// The number the next page is given.
    next_page: PageId,
    /// Where the next frames scroll the viewport to, once there is a
    /// layout to find it in: an entry's saved offset, shown again.
    goal: Option<ScrollGoal>,
    /// How many full cascades have run: what proves a resize without
    /// viewport units does not.
    style_runs: u64,
    /// How many element styles restyles have computed (M13.1): what
    /// proves a change restyles what it touches and no more.
    restyled_elements: u64,
    /// How many layouts have run: what proves a scroll does not.
    layout_runs: u64,
    /// How many restyles left the layout standing (M22.4): their changes
    /// reached nothing layout reads, or only the transforms and
    /// opacities of boxes, which were written into it.
    layouts_kept: u64,
    /// How many boxes placed in flow the layouts have laid out, and how
    /// many they took from the cache instead: what proves a change lays
    /// out what it reaches and no more.
    boxes_laid_out: u64,
    boxes_reused: u64,
    /// The last frame's size: what scrolling is clamped to.
    viewport: (u32, u32),
    /// The user's preferences that media queries ask about (M20).
    preferences: Preferences,
    /// The medium the shown document is styled for: `screen` unless
    /// emulated otherwise.
    media_type: MediaType,
    /// The time of the last frame drawn while a scroll offset was on its
    /// way to its target (M13.4): the next frame advances the animation
    /// by the time since. `None` when none was, so that an animation
    /// begun since starts with one frame's step, however long the
    /// engine was idle.
    animation_clock: Option<Duration>,
    /// What the last frame took (M13.4).
    stats: FrameStats,
    /// Whether the document was replaced or changed, or a scroll offset
    /// set, since the last frame.
    changed: bool,
    /// The animation and transition events the last frame queued, with
    /// those queued between it and the frame before (M22.2).
    events: Vec<AnimationEvent>,
    /// The threads images are read and decoded on (M23).
    decoder: Decoder,
    /// The number the next image is given.
    next_image: u64,
    /// The clock tasks, timers and scripts read (M26).
    clock: Clock,
    /// Whether the window is hidden (M26): minimized or covered.
    hidden: bool,
    /// When a hidden document's timer last ran.
    hidden_ran: Option<Duration>,
    /// The animation and transition events the frames queued that the
    /// next frame's rendering steps dispatch (M26).
    pending_events: Vec<AnimationEvent>,
    /// What a document wrote to its console as it was left (M26), for
    /// the next [`Engine::take_console`].
    departed: Vec<ConsoleEntry>,
    /// Where the pointer is over the page, in its coordinates, while it
    /// is (M26.4).
    pointer: Option<(f32, f32)>,
    /// The buttons held, as `buttons` has them, and the modifier keys
    /// (M26.5).
    buttons: u16,
    modifiers: Modifiers,
    /// Whether the window has the system's focus, and whether the last
    /// input was a key rather than the pointer (M26.6).
    window_focused: bool,
    keyboard: bool,
}

/// A document and everything the engine made from it (M15): what the
/// back-forward cache keeps whole, so that a document shown again needs
/// no cascade and no layout.
struct Shown {
    document: Document,
    /// The document's stylesheets, element by element, with what is
    /// fetched for them, and its URL.
    page: Page,
    /// The stylesheets that apply, prepared for the cascade.
    stylesheets: Stylesheets,
    /// The cascade of the document, computed on first use and dropped
    /// when its stylesheets change, or when the viewport changes and the
    /// document used viewport units (M10.1); a change to the document
    /// restyles it (M13.1).
    styled: Option<CachedStyle>,
    /// The changes made to the document since the cached cascade, in
    /// order, for the next frame to restyle by.
    pending: Vec<Change>,
    /// Whether the stylesheets changed since the cached cascade, which
    /// the next frame cascades the document whole for, as a change of
    /// style from the cached one (M22.1).
    recascade: bool,
    /// The layout of the document for one viewport width, dropped when
    /// the document changes, or the width changes (M9.5).
    layout: Option<CachedLayout>,
    /// The boxes' layouts the next layout may take rather than redo
    /// (M13.2): forgotten where a change reaches, and all of them when
    /// the document is cascaded whole.
    layout_cache: LayoutCache,
    /// The scroll offset shown, and the one being approached.
    scroll_y: f32,
    scroll_target: f32,
    /// Each scroll container's offset (M10.5), by its element's id; one
    /// never scrolled has no entry, and one freed loses its entry.
    element_scroll: HashMap<NodeId, ElementScroll>,
    /// The initial `about:blank` of a new engine, which the first
    /// navigation replaces (HTML's "is initial about:blank").
    initial: bool,
    /// Whether the back-forward cache may keep the document when it is
    /// left: HTML's "salvageable", which aborting its fetches ends. No
    /// page the browser made is kept.
    salvageable: bool,
    /// For an error page, the error it says.
    failure: Option<LoadError>,
    /// Whether the browser made the page: `about:silver`, or a page in
    /// place of a response. Only such a page may open `about:` pages.
    made: bool,
    /// The document's icon, once it has arrived.
    icon: Option<Icon>,
    /// The animations the document runs and its timeline (M22.3), which
    /// the cascade keeps.
    animations: RefCell<Animations>,
    /// The caller's time of the first frame drawn for a time while the
    /// document was shown: its timeline's origin.
    timeline_origin: Option<Duration>,
    /// The elements whose animations gave something new at the last
    /// frame's time, for the style stage to restyle.
    animated: Vec<NodeId>,
    /// The document's images (M23).
    images: ImageStore,
    /// When on the timeline the next frame of an animated image the last
    /// frame showed in the viewport comes (M23.1).
    next_image_frame: Option<f64>,
    /// The document's parse, while its body is still arriving or its
    /// parse has more to do (M24.8).
    parsing: Option<Box<loader::Parsing>>,
    /// The document's scripts (M24).
    scripts: scripts::Scripts,
    /// Its lifecycle events fired so far (M26).
    lifecycle: Lifecycle,
    /// The viewport size and the scroll offsets of the last frame drawn
    /// for a time: a frame that finds them changed fires `resize` and
    /// `scroll` (M26).
    rendered: Option<Rendered>,
    /// The `img` elements told whether their image came, each with the
    /// URL it was told of (M26).
    told: HashMap<NodeId, String>,
    /// The elements hovered, the one under the pointer and its ancestors
    /// (M26.4), and the one the URL's fragment indicates.
    hovered: Vec<NodeId>,
    target: Option<NodeId>,
    /// The element each button went down on, by its number (M26.5); the
    /// elements active while the primary one is held; and whether the
    /// mouse events stand down, a `pointerdown` having been canceled,
    /// until the buttons are all up.
    pressed: [Option<NodeId>; 5],
    active: Vec<NodeId>,
    no_mouse: bool,
    /// The element a Space press is to click when it comes up (M26.6);
    /// whether an `autofocus` element has had the focus, and the version
    /// of the document the styles were last brought up to, and was when
    /// one was last looked for.
    space: Option<NodeId>,
    autofocused: bool,
    styled_at: u64,
    autofocus_seen: Option<u64>,
}

/// The lifecycle events a document has had (HTML section 8.4.6, "the
/// end"): `readystatechange` as its parse ends, `DOMContentLoaded`, and
/// those of its load.
#[derive(Debug, Clone, Copy, Default)]
struct Lifecycle {
    interactive: bool,
    content_loaded: bool,
    loaded: bool,
}

/// What a frame was drawn at, for the next one's rendering steps.
#[derive(Debug, Clone, Default)]
struct Rendered {
    size: (u32, u32),
    scroll_y: f32,
    element_scroll: HashMap<NodeId, f32>,
}

impl Shown {
    /// `document` with `page` its stylesheets, nothing made from it yet,
    /// at the top.
    fn new(mut document: Document, page: Page) -> Shown {
        document.take_changes();
        let stylesheets = Stylesheets::new(page.sheets());
        Shown {
            document,
            page,
            stylesheets,
            styled: None,
            pending: Vec::new(),
            recascade: false,
            layout: None,
            layout_cache: LayoutCache::new(),
            scroll_y: 0.0,
            scroll_target: 0.0,
            element_scroll: HashMap::new(),
            initial: false,
            salvageable: true,
            failure: None,
            made: false,
            icon: None,
            animations: RefCell::new(Animations::new()),
            timeline_origin: None,
            animated: Vec::new(),
            images: ImageStore::default(),
            next_image_frame: None,
            parsing: None,
            scripts: scripts::Scripts::default(),
            lifecycle: Lifecycle::default(),
            rendered: None,
            told: HashMap::new(),
            hovered: Vec::new(),
            target: None,
            pressed: [None; 5],
            active: Vec::new(),
            no_mouse: false,
            space: None,
            autofocused: false,
            styled_at: 0,
            autofocus_seen: None,
        }
    }

    /// Dispatches `event` at `target` in this document, with what the
    /// run needs lent (M26): whether a listener canceled it.
    fn fire(
        &mut self,
        network: &mut loader::Network,
        window: scripts::WindowEnv,
        ready: scripts::ReadyState,
        target: js::Target,
        event: js::EventInit,
    ) -> Option<bool> {
        let mut lent = scripts::Lent {
            document: &mut self.document,
            page: &mut self.page,
            parser: self.parsing.as_mut().map(|parsing| &mut parsing.load),
            network,
            ready,
            window,
        };
        self.scripts.dispatch(&mut lent, target, event)
    }
}

/// A navigation under way: the URL navigated to, its document's load,
/// and what showing it does to the session history.
struct Navigation {
    url: Url,
    incoming: Incoming,
    handling: Handling,
    /// When it started: its document's time origin (M26).
    started: scripts::TimeOrigin,
}

/// What a navigation's document does to the session history when it is
/// shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handling {
    /// A new entry after the current one (HTML's "push").
    Push,
    /// A new entry in the current one's place (HTML's "replace").
    Replace,
    /// The current entry's document, fetched again.
    Reload,
    /// The document of the entry at this index, fetched again since none
    /// was kept: a traversal.
    Traverse(usize),
}

/// Where the viewport is to be scrolled once the document is laid out.
#[derive(Debug, Clone, PartialEq)]
enum ScrollGoal {
    /// An entry's saved offset.
    Offset(f32),
    /// The part of the document a URL's fragment indicates (HTML's
    /// "scroll to the fragment").
    Fragment(String),
}

/// What a fragment indicates in a document (HTML's "indicated part").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Indicated {
    /// The top of the document.
    Top,
    Element(NodeId),
}

/// The most bytes an icon is kept with: no icon needs more.
pub const MAX_ICON_BYTES: usize = 1 << 20;

/// The status bubble's notice when a document's realm runs out of
/// memory (M24.7).
const SCRIPTS_STOPPED: &str = "Scripts stopped on this page: out of memory";

/// The tokens a document is parsed by between polls while its bytes
/// arrive (M24.8): the window draws between slices.
const PARSE_SLICE: usize = 4096;

/// The page of a document parsed from `head`'s response with `load`,
/// its encoding as the load has found it (M24.8).
fn parsed_page(id: PageId, head: &net::Response, load: &html::Load) -> Page {
    let address_space = head.address_space.unwrap_or(net::AddressSpace::Public);
    let encoding = load.encoding().unwrap_or(encoding::UTF_8);
    Page::fetched(
        id,
        head.url().clone(),
        encoding,
        load.undecodable(),
        address_space,
    )
    .with_language(loader::content_language(head))
    .with_csp(net::csp::Csp::from_headers(&head.headers, head.url()))
}

/// Whether `document` has a body: HTML's "allows adding render-blocking
/// elements" no longer holds (section 3.1.6).
fn has_body(document: &Document) -> bool {
    document.document_element().is_some_and(|html| {
        document
            .element_children(html)
            .any(|child| document.tag(child) == Some("body"))
    })
}

/// A document's icon (docs/design/navigation.md, "The icon"): fetched
/// and kept, not shown until images are decoded (M23).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icon {
    /// Where it came from, after any redirects.
    pub url: Url,
    /// Its computed MIME type's essence: an image type.
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

/// A link under the pointer (docs/design/navigation.md, "Links"): an `a`
/// element with an `href`, in the document shown when it was found, and
/// the URL its `href` parses to.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    page: PageId,
    node: NodeId,
    url: Url,
}

impl Link {
    /// The URL the link goes to: its `href` parsed against the
    /// document's base URL, with the document's encoding.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The link's element.
    pub fn node(&self) -> NodeId {
        self.node
    }
}

/// A scroll container's offset shown, and the one being approached.
#[derive(Debug, Clone, Copy, Default)]
struct ElementScroll {
    y: f32,
    target: f32,
}

struct CachedLayout {
    width: f32,
    /// The viewport height the layout was given; it matters only when
    /// the layout says it depends on it.
    height: f32,
    layout: DocumentLayout,
}

struct CachedStyle {
    /// The viewport the cascade resolved its units against.
    viewport: (u32, u32),
    styles: Styles,
}

/// The engine's fonts, lent to the cascade for `ex` and `ch`.
struct FontMeasurer<'a>(&'a text::FontCollection);

/// The size in raster pixels each way that `image`, one tile `tile` of
/// it in the coordinates `matrix` maps to the raster's, is drawn at
/// (M23.5): the whole image's, of which a slice is drawn, as its pixels
/// lie before they are turned.
fn drawn_size(
    matrix: &style::transform::Matrix,
    tile: [f32; 4],
    image: &paint::ImageRef,
) -> (f32, f32) {
    let (mut w, mut h) = matrix
        .bounds(tile[0], tile[1], tile[2], tile[3])
        .map_or((tile[2], tile[3]), |(l, t, r, b)| (r - l, b - t));
    if let Some([x0, y0, x1, y1]) = image.crop {
        w /= (x1 - x0).max(f32::EPSILON);
        h /= (y1 - y0).max(f32::EPSILON);
    }
    if image.orientation.swaps_sides() {
        (w, h) = (h, w);
    }
    (w, h)
}

/// Whether `change` may change the images the document's `img` elements
/// show: nodes in or out, a `src`, or an `href` that may be a base URL's.
fn may_touch_images(change: &Change) -> bool {
    match change {
        Change::Inserted { .. } | Change::Removed { .. } | Change::Everything => true,
        Change::AttributeChanged { name, .. } => matches!(name.as_str(), "src" | "href"),
        Change::TextChanged { .. }
        | Change::StateChanged { .. }
        | Change::ControlChanged { .. } => false,
    }
}

/// Whether layout reads `change` whatever the styles: a change of the
/// document's structure or text, of a table's spans (M22.4), or of what
/// a form control shows (M26.7).
fn read_by_layout(change: &Change) -> bool {
    match change {
        Change::AttributeChanged { name, .. } => {
            matches!(
                name.as_str(),
                "colspan" | "rowspan" | "span" | "src" | "alt"
            ) || layout::shown_by_controls(name)
        }
        _ => true,
    }
}

/// What a cascade of a document knows from outside it: the `viewport`,
/// the engine's fonts as `fonts` measures them, the `language` its
/// response gave, the `media` it is shown on, and the `animations` it
/// runs.
fn cascade_context<'a>(
    viewport: (u32, u32),
    fonts: &'a FontMeasurer<'a>,
    language: Option<&'a str>,
    media: Media,
    animations: &'a RefCell<Animations>,
) -> CascadeContext<'a> {
    CascadeContext {
        viewport_width: viewport.0 as f32,
        viewport_height: viewport.1 as f32,
        fonts: Some(fonts),
        language,
        media,
        animations: Some(animations),
    }
}

impl FontMeasure for FontMeasurer<'_> {
    fn x_height(&self, font_size_px: f32) -> f32 {
        self.0.x_height(font_size_px)
    }

    fn zero_advance(&self, font_size_px: f32) -> f32 {
        self.0.zero_advance(font_size_px)
    }
}

/// The scrollbar's geometry for the last frame (M9.2), in viewport
/// pixels: the track at the right edge and the thumb within it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scrollbar {
    /// The track: `x` is its left edge, `w` its width, `h` the viewport's
    /// height (it starts at the top).
    pub x: f32,
    pub w: f32,
    pub h: f32,
    /// The thumb's top and height within the track.
    pub thumb_y: f32,
    pub thumb_h: f32,
}

impl Scrollbar {
    /// Whether a viewport point is on the track (thumb included).
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= 0.0 && y < self.h
    }

    /// Whether a viewport point is on the thumb.
    pub fn thumb_contains(&self, x: f32, y: f32) -> bool {
        self.contains(x, y) && y >= self.thumb_y && y < self.thumb_y + self.thumb_h
    }
}

impl Engine {
    /// Creates a session showing the initial `about:blank`, with the
    /// system's fonts, drawing in software.
    pub fn new() -> Result<Self, FontError> {
        Ok(Self::with_fonts(text::FontCollection::new()?))
    }

    /// Creates a session with the system's fonts on `backend` (M19.3):
    /// the GPU's where the machine has an adapter and the build has
    /// the backend, else the software's, with why the GPU's could not
    /// be had. [`Engine::backend`] says which it is on.
    pub fn new_on(backend: Backend) -> Result<(Self, Option<String>), FontError> {
        let fonts = text::FontCollection::new()?;
        match backend {
            Backend::Software => Ok((Self::with_fonts(fonts), None)),
            Backend::Gpu => match Self::on_gpu(fonts.clone()) {
                Ok(engine) => Ok((engine, None)),
                Err(why) => Ok((Self::with_fonts(fonts), Some(why))),
            },
        }
    }

    /// Creates a session with `fonts` on the GPU backend (M19.3), or
    /// says why there is none to be had.
    #[cfg(feature = "gpu")]
    pub fn on_gpu(fonts: text::FontCollection) -> Result<Self, String> {
        let backend =
            render::gpu::GpuBackend::with_fonts(fonts.clone()).map_err(|e| e.to_string())?;
        Ok(Self::with_backend(fonts, Box::new(backend)))
    }

    /// Creates a session with `fonts` on the GPU backend (M19.3), or
    /// says why there is none to be had: this build has none.
    #[cfg(not(feature = "gpu"))]
    pub fn on_gpu(fonts: text::FontCollection) -> Result<Self, String> {
        let _ = fonts;
        Err("this build has no GPU backend".to_owned())
    }

    /// Creates a session showing the initial `about:blank`, as HTML's
    /// new navigable does (M15), with fonts already loaded (a test's
    /// bundled font, say), drawing in software. Layout and the
    /// rasterizer share the collection by cloning its handles.
    pub fn with_fonts(fonts: text::FontCollection) -> Self {
        Self::with_backend(fonts.clone(), Box::new(SoftwareBackend::with_fonts(fonts)))
    }

    /// Creates a session showing the initial `about:blank` with
    /// `fonts`, drawing on `backend`, which must be over the same
    /// fonts.
    pub fn with_backend(fonts: text::FontCollection, backend: Box<dyn RenderBackend>) -> Self {
        let mut network = Network::new();
        let mut page = Page::given(0);
        let document = parsed_or_empty("");
        page.begin(&document, &mut network, &Media::default());
        let mut shown = Shown::new(document, page);
        shown.initial = true;
        let blank = Url::parse("about:blank").expect("about:blank parses");
        Engine {
            backend,
            layout_fonts: fonts,
            shown,
            history: History::new(Some(blank)),
            network,
            navigation: None,
            load_state: LoadState::Loaded,
            waker: None,
            compat_user_agent: false,
            scripting: false,
            script_limits: js::Limits::default(),
            notice: None,
            next_page: 1,
            goal: None,
            style_runs: 0,
            restyled_elements: 0,
            layout_runs: 0,
            layouts_kept: 0,
            boxes_laid_out: 0,
            boxes_reused: 0,
            viewport: (0, 0),
            preferences: Preferences::default(),
            media_type: MediaType::Screen,
            animation_clock: None,
            stats: FrameStats::default(),
            changed: true,
            events: Vec::new(),
            decoder: Decoder::new(),
            next_image: 0,
            clock: {
                let made = Instant::now();
                Arc::new(move || made.elapsed())
            },
            hidden: false,
            hidden_ran: None,
            pending_events: Vec::new(),
            departed: Vec::new(),
            pointer: None,
            buttons: 0,
            modifiers: Modifiers::default(),
            window_focused: true,
            keyboard: false,
        }
    }

    /// The fonts the engine draws with, for the browser's own interface
    /// to draw with too (M15): a clone shares their handles.
    pub fn fonts(&self) -> &text::FontCollection {
        &self.layout_fonts
    }

    /// Replaces the session document by parsing HTML `source` and
    /// installing it as [`Engine::set_document`] does. A source the
    /// parser refuses leaves the session as it was.
    pub fn load_document(&mut self, source: &str) {
        if let Ok(parsed) = html::parse_document(source) {
            self.set_document(Document::from_node(&parsed));
        }
    }

    /// Makes `document` the session document (M13.5), the only entry
    /// of a new session history, with no URL (M15): its `<style>`
    /// elements are parsed into stylesheets, the cascade and layout
    /// caches, the layout cache's boxes among them, and the glyph cache
    /// start over, every scroll offset is dropped and the document
    /// starts at the top. Its journal of changes is emptied, since the
    /// engine has seen nothing of it yet. The document has no URL and
    /// fetches nothing, so its `<link>` elements and `@import` rules
    /// bring no sheets. A navigation under way, the fetches of the page
    /// shown and the documents the history kept are dropped, and the
    /// load state is [`LoadState::Loaded`].
    pub fn set_document(&mut self, document: Document) {
        self.cancel_navigation();
        let mut page = Page::given(self.new_page_id());
        let media = self.media(self.viewport);
        page.begin(&document, &mut self.network, &media);
        let left = self.show(Shown::new(document, page));
        self.network.cancel(left.page.id(), None);
        self.history = History::new(None);
        self.goal = None;
        self.load_state = LoadState::Loaded;
    }

    /// Shows `shown` in place of the document shown, which it returns:
    /// the frame and the glyph cache start over, and the scroll
    /// animation's clock.
    fn show(&mut self, shown: Shown) -> Shown {
        let left = std::mem::replace(&mut self.shown, shown);
        let origin = self.time_origin();
        self.shown.scripts.set_time_origin(origin);
        self.backend.clear_glyph_cache();
        self.backend.forget_frame();
        self.animation_clock = None;
        self.events.clear();
        self.changed = true;
        left
    }

    /// Leaves `left`, the document of document state `state`: its
    /// fetches are aborted, and the back-forward cache keeps it if it is
    /// salvageable — no fetch was aborted for it, now or by a stop — and
    /// an entry still has its state. Its realm goes with it, its garbage
    /// collected, unless the realm died (M24).
    fn leave(&mut self, mut left: Shown, state: StateId) {
        let aborted = self.network.cancel(left.page.id(), None);
        let decoding = self.decoder.busy(left.page.id());
        self.decoder.forget(left.page.id());
        let kept = left.salvageable
            && !aborted
            && !decoding
            && !left.initial
            && left.parsing.is_none()
            && left.scripts.keepable();
        self.unload(&mut left, kept);
        if kept {
            left.scripts.collect();
            // What the cache lets go is dropped here.
            drop(self.history.keep(state, Box::new(left)));
        }
    }

    /// HTML's "unload a document" for `left`, a document no longer shown
    /// (M26): `pagehide`, saying whether the cache keeps it, and `unload`
    /// if not; what it writes to its console as it goes reaches the
    /// engine's.
    fn unload(&mut self, left: &mut Shown, kept: bool) {
        if !self.scripting || left.made {
            return;
        }
        let window = self.window_env();
        let ready = if left.lifecycle.loaded {
            scripts::ReadyState::Complete
        } else {
            scripts::ReadyState::Interactive
        };
        left.fire(
            &mut self.network,
            window.clone(),
            ready,
            js::Target::Window,
            js::EventInit::new("PageTransitionEvent", "pagehide")
                .with("persisted", js::Field::Bool(kept)),
        );
        if !kept {
            left.fire(
                &mut self.network,
                window,
                ready,
                js::Target::Window,
                js::EventInit::new("Event", "unload"),
            );
        }
        left.document.take_changes();
        self.departed.extend(left.scripts.take_console());
    }

    fn new_page_id(&mut self) -> PageId {
        let id = self.next_page;
        self.next_page += 1;
        id
    }

    /// Navigates to `url`, as the user asks with the address field or
    /// the command line (docs/design/navigation.md, "Navigating"): HTML's
    /// navigate, with no source document. The new entry replaces the
    /// current one when `url` is the current entry's URL, or the
    /// document shown is the initial `about:blank`; otherwise it is
    /// pushed after it. The document is fetched on a thread of its own,
    /// in place of any navigation under way, which is aborted; the page
    /// shown stays until the new document has arrived and so have the
    /// stylesheets the parser met before its `<body>`, or they have
    /// failed. [`Engine::poll_network`] takes the results, and
    /// [`Engine::load_state`] says where the navigation stands.
    pub fn navigate(&mut self, url: Url) {
        self.navigate_replacing(url, false);
    }

    /// [`Engine::navigate`], the new entry in the current one's place when
    /// `replace`, as `location.replace` asks (M25).
    fn navigate_replacing(&mut self, url: Url, replace: bool) {
        let current = self.history.current().url.clone();
        let handling = if replace || self.shown.initial || current.as_ref() == Some(&url) {
            Handling::Replace
        } else {
            Handling::Push
        };
        // A URL that differs from the current entry's only in having a
        // fragment is a navigation to that fragment, unless the page
        // shown is an error page, which no fragment could name a part
        // of: there the user wants the URL tried again.
        let same_document = url.fragment().is_some()
            && self.shown.failure.is_none()
            && current
                .as_ref()
                .is_some_and(|current| current.without_fragment() == url.without_fragment());
        if same_document {
            self.navigate_to_fragment(url, handling);
        } else {
            self.start_navigation(url, handling);
        }
    }

    /// HTML's "navigate to a fragment": a new entry for `url`, pushed or
    /// put in the current one's place, with the current entry's document
    /// state, so the document stays and only its URL changes; the next
    /// frame scrolls to the fragment. Nothing is fetched, and a
    /// navigation under way goes on, unless it fetches the current
    /// entry's document again for a reload or a traversal, which the
    /// new entry would leave without a place.
    fn navigate_to_fragment(&mut self, url: Url, handling: Handling) {
        if matches!(
            self.navigation
                .as_ref()
                .map(|navigation| navigation.handling),
            Some(Handling::Reload | Handling::Traverse(_))
        ) {
            self.cancel_navigation();
        }
        let state = self.history.current().state;
        self.history.current_mut().scroll = self.shown.scroll_target;
        if handling == Handling::Push {
            self.history.push(Some(url.clone()), state);
        } else {
            self.history.replace(Some(url.clone()), state);
        }
        let fragment = url.fragment().unwrap_or_default().to_owned();
        self.shown.page.set_url(url);
        self.goal = Some(ScrollGoal::Fragment(fragment));
        self.changed = true;
        self.update_target();
    }

    /// Follows `link`, a click on it having activated it (HTML's "follow
    /// the hyperlink", docs/design/navigation.md, "Links"): its `href`
    /// is parsed again as it now stands, and the page navigates to it,
    /// as [`Engine::navigate`] does, unless the link's document is no
    /// longer shown or the element has left it (HTML's "cannot
    /// navigate"), or the page may not navigate to that URL:
    ///
    /// - `http:`, `https:` and `about:blank` from any page;
    /// - `file:` only from a `file:` page;
    /// - other `about:` pages only from a page the browser made;
    /// - `data:` from no page, as the browsers refuse it;
    /// - `javascript:` runs its script in the document shown, with
    ///   scripting on (M24), and does nothing otherwise;
    /// - any other scheme would go to another program, which this
    ///   version cannot do.
    ///
    /// A refusal leaves a notice saying why, but for `javascript:`.
    pub fn follow_link(&mut self, link: &Link) {
        let document = &self.shown.document;
        if link.page != self.shown.page.id() || !connected(document, link.node) {
            return;
        }
        let Some(href) = document.attr(link.node, "href") else {
            return;
        };
        let Some(url) = self.shown.page.resolve(document, href) else {
            return;
        };
        if url.scheme() == "javascript" {
            if self.scripting {
                self.run_javascript_url(&url);
            }
            return;
        }
        let source = self.shown.page.url().map(Url::scheme);
        match refusal(&url, source, self.shown.made) {
            Some(why) => self.notice = Some(why),
            None => self.navigate(url),
        }
    }

    /// Starts the navigation the shown document's scripts asked for
    /// (`location`, M25.4), as a followed link's is, with HTML's handling:
    /// in the current entry's place for `replace`, and while the document
    /// has not loaded completely; a reload fetches it again.
    fn script_navigation(&mut self, navigation: scripts::ScriptNavigation) {
        let Some(url) = navigation.url.filter(|_| !navigation.reload) else {
            self.reload();
            return;
        };
        if url.scheme() == "javascript" {
            self.run_javascript_url(&url);
            return;
        }
        let source = self.shown.page.url().map(Url::scheme);
        match refusal(&url, source, self.shown.made) {
            Some(why) => self.notice = Some(why),
            // A followed link navigates as a click on it does (M26).
            None if navigation.link => self.navigate(url),
            None => {
                let replace = navigation.replace || self.load_state != LoadState::Loaded;
                self.navigate_replacing(url, replace);
            }
        }
    }

    /// Starts the navigation the scripts of the document on its way asked
    /// for: in place of the navigation that brought it, with its handling,
    /// so that a page that sends its reader on by script leaves one entry.
    /// Its `javascript:` URLs and reloads wait for it to be shown: they do
    /// nothing.
    fn incoming_navigation(&mut self, navigation: scripts::ScriptNavigation) {
        let Some(url) = navigation.url.filter(|_| !navigation.reload) else {
            return;
        };
        let Some(current) = &self.navigation else {
            return;
        };
        let handling = current.handling;
        let source = current.url.scheme().to_owned();
        if url.scheme() == "javascript" {
            return;
        }
        match refusal(&url, Some(&source), false) {
            Some(why) => self.notice = Some(why),
            None => self.start_navigation(url, handling),
        }
    }

    /// Runs a `javascript:` URL in the document shown (HTML section 7.4,
    /// "evaluate a javascript: URL"): what follows the scheme,
    /// percent-decoded as UTF-8, run as a classic script with a budget of
    /// its own, its errors naming the document. A string it ends with
    /// would replace the document, which waits for M26.6.
    fn run_javascript_url(&mut self, url: &Url) {
        let csp = self.shown.page.csp(&self.shown.document);
        if let Err(violation) = csp.check_javascript_url() {
            self.shown.scripts.refused("a javascript: URL", &violation);
            return;
        }
        let encoded = url.as_str().get("javascript:".len()..).unwrap_or_default();
        let source =
            String::from_utf8_lossy(&net::url::percent_decode(encoded.as_bytes())).into_owned();
        if let Some(js::Completion::String(html)) = self.run_classic(&source) {
            self.show_javascript_result(html);
        }
    }

    /// A `javascript:` URL's result (HTML section 7.4.2.3.2, M26.5): a
    /// string it ended with is a new document of that HTML at the same
    /// URL, in the current entry's place, as a response with that body
    /// would be, its scripts run.
    fn show_javascript_result(&mut self, html: String) {
        let Some(url) = self.shown.page.url().cloned() else {
            return;
        };
        self.cancel_navigation();
        let response = net::Response {
            url_list: vec![url.clone()],
            status: 200,
            status_message: b"OK".to_vec(),
            headers: net::HeaderList::default(),
            body: Vec::new(),
            mime_type: net::MimeType::parse("text/html;charset=utf-8"),
            address_space: None,
        };
        let id = self.new_page_id();
        self.navigation = Some(Navigation {
            url,
            incoming: Incoming::Receiving {
                id,
                head: Box::new(response),
                body: html.into_bytes(),
                sniffed: false,
            },
            handling: Handling::Replace,
            started: self.time_origin(),
        });
        self.load_state = LoadState::Loading;
        self.sniff_incoming(true);
    }

    /// Runs `source` in the document shown as a classic script with a
    /// budget of its own, its errors naming the document, and takes what
    /// it changed: what a `javascript:` URL runs, and a script
    /// [`differential::run_script`] is given.
    fn run_classic(&mut self, source: &str) -> Option<js::Completion> {
        let document = self
            .shown
            .page
            .url()
            .map(Url::to_string)
            .unwrap_or_default();
        let script = js::Script {
            source,
            url: &document,
            line: 1,
            muted: false,
        };
        let ready = self.ready_state();
        let window = self.window_env();
        let shown = &mut self.shown;
        let mut lent = scripts::Lent {
            document: &mut shown.document,
            page: &mut shown.page,
            parser: shown.parsing.as_mut().map(|parsing| &mut parsing.load),
            network: &mut self.network,
            ready,
            window: window.clone(),
        };
        let completion = shown
            .scripts
            .run(&mut lent, &script, scripts::How::default());
        self.scripts_ran();
        completion
    }

    /// What the window reports to a run's scripts (M25.4).
    fn window_env(&self) -> scripts::WindowEnv {
        scripts::WindowEnv {
            viewport: self.viewport,
            media: self.media(self.viewport),
            fonts: Some(self.layout_fonts.clone()),
            compat: self.compat_user_agent,
            clock: Some(self.clock.clone()),
            hidden: self.hidden,
            focused: self.window_focused,
            keyboard: self.keyboard,
        }
    }

    /// Dispatches `event` at `target` in the document shown, with scripts
    /// on (M26), as a task: its changes are taken after. Whether a
    /// listener canceled it; `None` when nothing was dispatched, scripts
    /// being off or the document having no listener at all.
    fn fire(&mut self, target: js::Target, event: js::EventInit) -> Option<bool> {
        if !self.scripting || self.shown.made {
            return None;
        }
        let ready = self.ready_state();
        let window = self.window_env();
        let canceled = self
            .shown
            .fire(&mut self.network, window, ready, target, event);
        self.scripts_ran();
        canceled
    }

    /// The modifier keys held from now on (M26.5), which the pointer's
    /// events say.
    pub fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.modifiers = modifiers;
    }

    /// The pointer moved to `(x, y)` in the page's coordinates, CSS
    /// pixels from the viewport's top left (M26.4, M26.5): the element
    /// there and its ancestors are hovered, with scripts on or off; with
    /// scripts on, the boundary events of the elements left and entered
    /// come first, then `pointermove` and `mousemove`.
    pub fn pointer_move(&mut self, x: f32, y: f32) {
        let target = self.pointer_at(x, y);
        if let Some(target) = target {
            let event = self.mouse_event("PointerEvent", "pointermove", target, -1, 0, None);
            self.fire(js::Target::Node(scripts::handle(target)), event);
            if !self.shown.no_mouse && self.shown.document.contains(target) {
                let event = self.mouse_event("MouseEvent", "mousemove", target, 0, 0, None);
                self.fire(js::Target::Node(scripts::handle(target)), event);
            }
        }
    }

    /// The pointer left the page: nothing is hovered, and the elements it
    /// was over hear it leave.
    pub fn pointer_leave(&mut self) {
        let left = self.shown.hovered.clone();
        self.set_hovered(Vec::new());
        self.boundary(&left, &[]);
        self.pointer = None;
    }

    /// A button went down at `(x, y)` (M26.5), the `count`th press of a
    /// run of quick ones: the element there is active while the primary
    /// one is held, with scripts on or off; with scripts on, `pointerdown`
    /// and, unless it was canceled, `mousedown`.
    pub fn pointer_down(&mut self, x: f32, y: f32, button: Button, count: u32) {
        self.keyboard = false;
        let target = self.pointer_at(x, y);
        self.buttons |= button.bit();
        self.shown.pressed[button.number() as usize] = target;
        if button == Button::Primary {
            let chain = target.map(|node| self.chain(node)).unwrap_or_default();
            self.set_flag(ElementState::ACTIVE, chain);
        }
        let Some(target) = target else {
            // Past the root's box: the viewport takes the focus.
            if matches!(button, Button::Primary | Button::Secondary) {
                self.focus(None, false, false);
            }
            return;
        };
        let event = self.mouse_event(
            "PointerEvent",
            "pointerdown",
            target,
            button.number(),
            0,
            None,
        );
        if self.fire(js::Target::Node(scripts::handle(target)), event) == Some(true) {
            self.shown.no_mouse = true;
        }
        let mut canceled = None;
        if !self.shown.no_mouse && self.shown.document.contains(target) {
            let event = self.mouse_event(
                "MouseEvent",
                "mousedown",
                target,
                button.number(),
                count,
                None,
            );
            canceled = self.fire(js::Target::Node(scripts::handle(target)), event);
        }
        // mousedown's default action (HTML section 6.6.4): the nearest
        // focusable area around the target takes the focus, a text
        // control's shown; with none, the viewport takes it.
        if canceled != Some(true)
            && matches!(button, Button::Primary | Button::Secondary)
            && self.shown.document.contains(target)
        {
            self.cached_styles();
            let styles = self.shown.styled.as_ref().map(|styled| &styled.styles);
            let area = focus::click_focus_target(&self.shown.document, styles, target);
            let visible =
                area.is_some_and(|area| dom::forms::is_text_control(&self.shown.document, area));
            self.focus(area, visible, false);
        }
    }

    /// The element focused in the document shown, if any (M26.6).
    pub fn focused(&self) -> Option<NodeId> {
        self.shown.document.focused()
    }

    /// HTML's focusing steps (section 6.6.4) for `area`, or its
    /// unfocusing steps for none (M26.6): the element focused loses it,
    /// with `blur` and `focusout` naming the new one, then `area` gains
    /// it, with `focus` and `focusin` naming the old, shown when
    /// `visible`, and scrolled into view when `scroll`. Nothing when it
    /// has the focus already; a listener that moves the focus meanwhile
    /// has the last word.
    fn focus(&mut self, area: Option<NodeId>, visible: bool, scroll: bool) {
        let old = self.shown.document.focused();
        if old == area {
            return;
        }
        if let Some(old) = old {
            self.shown.document.set_focus(None, false);
            self.state_changed();
            for (kind, bubbles) in [("blur", false), ("focusout", true)] {
                if self.shown.document.contains(old) {
                    let event = focus_event(kind, bubbles, area);
                    self.fire(js::Target::Node(scripts::handle(old)), event);
                }
            }
        }
        let Some(area) = area.filter(|&area| self.shown.document.contains(area)) else {
            return;
        };
        if self.shown.document.focused().is_some() {
            return;
        }
        self.shown.document.set_focus(Some(area), visible);
        self.state_changed();
        if scroll {
            self.scroll_focus_into_view(area);
        }
        for (kind, bubbles) in [("focus", false), ("focusin", true)] {
            if self.shown.document.contains(area) {
                let event = focus_event(kind, bubbles, old);
                self.fire(js::Target::Node(scripts::handle(area)), event);
            }
        }
    }

    /// Scrolls an element given the focus into view (M26.6), unless the
    /// top of its first box is in the viewport already and no scroll
    /// container holds it, as the reference browser's focus does.
    fn scroll_focus_into_view(&mut self, node: NodeId) {
        let height = self.viewport.1 as f32;
        let at = self.shown.scroll_target;
        if let Some((top, chain)) = self.locate(node)
            && chain.is_empty()
            && (at..at + height).contains(&top)
        {
            return;
        }
        self.scroll_into_view(node);
    }

    /// The document's state changes are taken, restyling what they reach.
    fn state_changed(&mut self) {
        let changes = self.shown.document.take_changes();
        if !changes.is_empty() {
            self.document_changed(changes);
        }
    }

    /// Moves the focus to the next area of the sequential focus order
    /// (HTML section 6.6.3), or the previous for `backward`: Tab and
    /// Shift+Tab. From nothing focused, the first or the last. Past the
    /// end, the page gives up the focus, and the answer is false: the
    /// browser's own interface is to take it.
    pub fn focus_next(&mut self, backward: bool) -> bool {
        self.keyboard = true;
        self.cached_styles();
        let styles = self.shown.styled.as_ref().map(|styled| &styled.styles);
        let order = focus::sequential_order(&self.shown.document, styles);
        let current = self.shown.document.focused();
        let at = current.and_then(|current| order.iter().position(|&node| node == current));
        let next = match (at, backward) {
            (Some(i), false) => order.get(i + 1).copied(),
            (Some(i), true) => i.checked_sub(1).map(|i| order[i]),
            (None, false) => order.first().copied(),
            (None, true) => order.last().copied(),
        };
        match next {
            Some(next) => {
                self.focus(Some(next), true, true);
                true
            }
            None => {
                self.focus(None, false, false);
                false
            }
        }
    }

    /// Whether the window has the system's focus (M26.6): when it loses it,
    /// the element focused hears `blur`, then the window; when it comes
    /// back, the window hears `focus`, then the element.
    pub fn set_window_focus(&mut self, focused: bool) {
        if self.window_focused == focused {
            return;
        }
        self.window_focused = focused;
        let element = self.shown.document.focused();
        let kind = if focused { "focus" } else { "blur" };
        let at_element = |engine: &mut Engine| {
            if let Some(element) = element {
                let event = focus_event(kind, false, None);
                engine.fire(js::Target::Node(scripts::handle(element)), event);
            }
        };
        if !focused {
            at_element(self);
        }
        self.fire(js::Target::Window, focus_event(kind, false, None));
        if focused {
            at_element(self);
        }
    }

    /// A key went down (M26.6): `keydown` at the element focused, or the
    /// body; then, unless a listener canceled it, its default action: Tab
    /// moves the focus, Enter activates a link or a button, Space arms a
    /// button for its release, and the scrolling keys scroll the page when
    /// no text control has the focus.
    pub fn key_down(&mut self, key: &Key) -> KeyOutcome {
        self.keyboard = true;
        if let Some(target) = self.key_target() {
            let event = self.key_event("keydown", key, None);
            if self.fire(js::Target::Node(scripts::handle(target)), event) == Some(true) {
                return KeyOutcome::Taken;
            }
        }
        let outcome = self.key_default(key);
        self.key_press(key);
        outcome
    }

    /// A key's default action, after its `keydown` (M26.6).
    fn key_default(&mut self, key: &Key) -> KeyOutcome {
        let m = self.modifiers;
        let plain = !m.ctrl && !m.alt && !m.meta;
        let focused = self.shown.document.focused();
        let document = &self.shown.document;
        let text_control = focused.is_some_and(|node| dom::forms::is_text_control(document, node));
        let clicks = |space| focused.filter(|&node| key_clicks(document, node, space));
        match key.key.as_str() {
            "Tab" if plain => {
                if self.focus_next(m.shift) {
                    KeyOutcome::Taken
                } else {
                    KeyOutcome::FocusLeft
                }
            }
            "Enter" if plain && clicks(false).is_some() => {
                if let Some(element) = clicks(false) {
                    self.key_click(element);
                }
                KeyOutcome::Taken
            }
            " " if plain && clicks(true).is_some() => {
                self.shown.space = clicks(true);
                KeyOutcome::Taken
            }
            _ if text_control => KeyOutcome::Ignored,
            "ArrowDown" | "ArrowUp" | "PageDown" | "PageUp" | " " | "Home" | "End"
                if plain || m.shift =>
            {
                let page = self.page_step();
                match key.key.as_str() {
                    "ArrowDown" => self.scroll_by(SCROLL_STEP),
                    "ArrowUp" => self.scroll_by(-SCROLL_STEP),
                    "PageDown" => self.scroll_by(page),
                    "PageUp" => self.scroll_by(-page),
                    " " if m.shift => self.scroll_by(-page),
                    " " => self.scroll_by(page),
                    "Home" => self.scroll_to(0.0),
                    _ => self.scroll_to(f32::INFINITY),
                }
                KeyOutcome::Taken
            }
            _ => KeyOutcome::Ignored,
        }
    }

    /// A page's scroll by a key: the viewport's height less a line, so a
    /// line of context stays.
    fn page_step(&self) -> f32 {
        (self.viewport.1 as f32 - SCROLL_STEP).max(SCROLL_STEP)
    }

    /// The `keypress` for each character a key types (UI Events section
    /// 5.6.4, the reference browser's legacy event): none for a control
    /// character but Enter's, nor with Ctrl or Meta held but as AltGr;
    /// its `charCode` and `keyCode` the character's.
    fn key_press(&mut self, key: &Key) {
        let m = self.modifiers;
        if (m.ctrl && !m.alt) || m.meta {
            return;
        }
        let Some(text) = key.text.clone() else {
            return;
        };
        for c in text.chars() {
            if c.is_control() && c != '\r' {
                continue;
            }
            let Some(target) = self.key_target() else {
                return;
            };
            let press = Key {
                key: if c == '\r' {
                    "Enter".to_owned()
                } else {
                    c.to_string()
                },
                text: None,
                ..key.clone()
            };
            let event = self.key_event("keypress", &press, Some(c as u32));
            self.fire(js::Target::Node(scripts::handle(target)), event);
        }
    }

    /// A key came up (M26.6): `keyup`; Space's release clicks the button
    /// its press armed.
    pub fn key_up(&mut self, key: &Key) {
        if let Some(target) = self.key_target() {
            let event = self.key_event("keyup", key, None);
            self.fire(js::Target::Node(scripts::handle(target)), event);
        }
        if key.key == " "
            && let Some(armed) = self.shown.space.take()
            && self.shown.document.focused() == Some(armed)
        {
            self.key_click(armed);
        }
    }

    /// Where a key's events go: the element focused, or the body, or the
    /// document's element.
    fn key_target(&self) -> Option<NodeId> {
        let document = &self.shown.document;
        document.focused().or_else(|| {
            let html = document.element_children(document.root()).next()?;
            document
                .element_children(html)
                .find(|&child| matches!(document.tag(child), Some("body" | "frameset")))
                .or(Some(html))
        })
    }

    /// A `KeyboardEvent` of `kind` for `key` (UI Events section 5.6):
    /// bubbling, cancelable and composed, its legacy `keyCode` the
    /// reference browser's, or for a `keypress` its `charCode`'s.
    fn key_event(&self, kind: &str, key: &Key, char_code: Option<u32>) -> js::EventInit {
        use js::Field::{Bool, Number, String as Text, Window};
        let m = self.modifiers;
        let key_code = char_code.unwrap_or_else(|| keys::key_code(key));
        js::EventInit::new("KeyboardEvent", kind)
            .with("bubbles", Bool(true))
            .with("cancelable", Bool(true))
            .with("composed", Bool(true))
            .with("view", Window)
            .with("key", Text(key.key.clone()))
            .with("code", Text(key.code.clone()))
            .with("location", Number(f64::from(key.location)))
            .with("repeat", Bool(key.repeat))
            .with("keyCode", Number(f64::from(key_code)))
            .with("charCode", Number(f64::from(char_code.unwrap_or(0))))
            .with("shiftKey", Bool(m.shift))
            .with("ctrlKey", Bool(m.ctrl))
            .with("altKey", Bool(m.alt))
            .with("metaKey", Bool(m.meta))
    }

    /// The click a key gives an element (Enter on a link or a button,
    /// Space's release on a button): a trusted `click`, at no point, its
    /// detail 0, as the reference browser's; with no listener, the
    /// activation behaviour itself.
    fn key_click(&mut self, element: NodeId) {
        let event = js::EventInit::new("PointerEvent", "click")
            .with("bubbles", js::Field::Bool(true))
            .with("cancelable", js::Field::Bool(true))
            .with("composed", js::Field::Bool(true))
            .with("view", js::Field::Window)
            .with("pointerId", js::Field::Number(-1.0));
        if self
            .fire(js::Target::Node(scripts::handle(element)), event)
            .is_none()
        {
            self.activate(element);
        }
    }

    /// A button came up at `(x, y)` (M26.5): `pointerup` and `mouseup`;
    /// then, at the nearest element over both the press and the release,
    /// `click` for the primary button, its activation behaviour run unless
    /// it was canceled (a link is followed), and `dblclick` after a second
    /// press; `auxclick` for another, and `contextmenu` after it for the
    /// secondary one. With scripts off, the activation alone.
    pub fn pointer_up(&mut self, x: f32, y: f32, button: Button, count: u32) {
        let target = self.pointer_at(x, y);
        self.buttons &= !button.bit();
        let pressed = self.shown.pressed[button.number() as usize].take();
        if button == Button::Primary {
            self.set_flag(ElementState::ACTIVE, Vec::new());
        }
        let no_mouse = self.shown.no_mouse;
        if self.buttons == 0 {
            self.shown.no_mouse = false;
        }
        let Some(target) = target else {
            return;
        };
        let event = self.mouse_event(
            "PointerEvent",
            "pointerup",
            target,
            button.number(),
            0,
            None,
        );
        self.fire(js::Target::Node(scripts::handle(target)), event);
        if !no_mouse && self.shown.document.contains(target) {
            let event = self.mouse_event(
                "MouseEvent",
                "mouseup",
                target,
                button.number(),
                count,
                None,
            );
            self.fire(js::Target::Node(scripts::handle(target)), event);
        }
        let Some(clicked) = pressed.and_then(|pressed| self.common_ancestor(pressed, target))
        else {
            return;
        };
        let at = js::Target::Node(scripts::handle(clicked));
        match button {
            Button::Primary => {
                let event = self.mouse_event("PointerEvent", "click", clicked, 0, count, None);
                if self.fire(at, event).is_none() {
                    // Nothing listens: the activation behaviour, as the
                    // dispatch would run it.
                    self.activate(clicked);
                }
                if count == 2 && self.shown.document.contains(clicked) {
                    let event = self.mouse_event("MouseEvent", "dblclick", clicked, 0, 2, None);
                    self.fire(at, event);
                }
            }
            Button::Auxiliary | Button::Secondary => {
                let event = self.mouse_event(
                    "PointerEvent",
                    "auxclick",
                    clicked,
                    button.number(),
                    count,
                    None,
                );
                let canceled = self.fire(at, event) == Some(true);
                if button == Button::Auxiliary && !canceled && self.link_around(clicked).is_some() {
                    self.notice = Some(NO_NEW_TAB.to_owned());
                }
                if button == Button::Secondary && self.shown.document.contains(clicked) {
                    let event =
                        self.mouse_event("PointerEvent", "contextmenu", clicked, 2, 0, None);
                    self.fire(at, event);
                }
            }
            Button::Back | Button::Forward => {}
        }
    }

    /// The element under `(x, y)`, the pointer moved there: the hovered
    /// elements are those under it now, and, with scripts on, the
    /// elements left and entered hear it (UI Events section 4.3.4,
    /// Pointer Events section 4.1, in the reference browser's order).
    fn pointer_at(&mut self, x: f32, y: f32) -> Option<NodeId> {
        self.pointer = Some((x, y));
        let before = self.shown.hovered.clone();
        self.update_hover();
        let after = self.shown.hovered.clone();
        if before.first() != after.first() {
            self.boundary(&before, &after);
        }
        self.shown
            .hovered
            .first()
            .copied()
            .filter(|&node| self.shown.document.contains(node))
    }

    /// The boundary events of the pointer moving from the element first in
    /// `before` to the one first in `after`, each list it and its
    /// ancestors: `pointerout` at the one left, `pointerleave` at each
    /// left, innermost first, `pointerover` at the one entered,
    /// `pointerenter` at each entered, outermost first; then the mouse
    /// events of the same.
    fn boundary(&mut self, before: &[NodeId], after: &[NodeId]) {
        if !self.scripting {
            return;
        }
        let (old, new) = (before.first().copied(), after.first().copied());
        for (interface, prefix) in [("PointerEvent", "pointer"), ("MouseEvent", "mouse")] {
            if let Some(old) = old.filter(|&node| self.shown.document.contains(node)) {
                let event = self.mouse_event(interface, &format!("{prefix}out"), old, -1, 0, new);
                self.fire(js::Target::Node(scripts::handle(old)), event);
            }
            for &node in before.iter().filter(|node| !after.contains(node)) {
                if self.shown.document.contains(node) {
                    let event =
                        self.mouse_event(interface, &format!("{prefix}leave"), node, -1, 0, new);
                    self.fire(js::Target::Node(scripts::handle(node)), event);
                }
            }
            if let Some(new) = new.filter(|&node| self.shown.document.contains(node)) {
                let event = self.mouse_event(interface, &format!("{prefix}over"), new, -1, 0, old);
                self.fire(js::Target::Node(scripts::handle(new)), event);
            }
            for &node in after.iter().rev().filter(|node| !before.contains(node)) {
                if self.shown.document.contains(node) {
                    let event =
                        self.mouse_event(interface, &format!("{prefix}enter"), node, -1, 0, old);
                    self.fire(js::Target::Node(scripts::handle(node)), event);
                }
            }
        }
    }

    /// A pointer or mouse event of `kind` at `target` (M26.5): bubbling,
    /// cancelable and composed but for the enter and leave events; at the
    /// pointer's point, on the page and from the target's padding edge;
    /// with the buttons held, the modifier keys and `button`, which a
    /// mouse event's move makes 0 and a pointer event's -1; `detail` the
    /// press's count; the mouse's pointer, id 1 and primary, its pressure
    /// a half while a button is held.
    fn mouse_event(
        &mut self,
        interface: &'static str,
        kind: &str,
        target: NodeId,
        button: i16,
        detail: u32,
        related: Option<NodeId>,
    ) -> js::EventInit {
        use js::Field::{Bool, Node, Number, String as Text, Window};
        let (x, y) = self.pointer.unwrap_or_default();
        let (ox, oy) = self.padding_offset(target, x, y);
        let edge = kind.ends_with("enter") || kind.ends_with("leave");
        let pointer = interface == "PointerEvent";
        let button = if !pointer && button < 0 { 0 } else { button };
        let m = self.modifiers;
        let mut event = js::EventInit::new(interface, kind)
            .with("bubbles", Bool(!edge))
            .with("cancelable", Bool(!edge))
            .with("composed", Bool(!edge))
            .with("view", Window)
            .with("detail", Number(f64::from(detail)))
            .with("clientX", Number(f64::from(x)))
            .with("clientY", Number(f64::from(y)))
            .with("screenX", Number(f64::from(x)))
            .with("screenY", Number(f64::from(y)))
            .with("pageX", Number(f64::from(x)))
            .with("pageY", Number(f64::from(y + self.shown.scroll_y)))
            .with("offsetX", Number(f64::from(ox)))
            .with("offsetY", Number(f64::from(oy)))
            .with("button", Number(f64::from(button)))
            .with("buttons", Number(f64::from(self.buttons)))
            .with("shiftKey", Bool(m.shift))
            .with("ctrlKey", Bool(m.ctrl))
            .with("altKey", Bool(m.alt))
            .with("metaKey", Bool(m.meta))
            .with("relatedTarget", Node(related.map(scripts::handle)));
        if pointer {
            event = event
                .with("pointerId", Number(1.0))
                .with("pointerType", Text("mouse".to_owned()))
                .with("isPrimary", Bool(true))
                .with(
                    "pressure",
                    Number(if self.buttons != 0 { 0.5 } else { 0.0 }),
                );
        }
        event
    }

    /// The point `(x, y)` from `node`'s padding edge: from its first box's,
    /// in that box's coordinates, as the paint order places it; the
    /// point itself when it has none.
    fn padding_offset(&mut self, node: NodeId, x: f32, y: f32) -> (f32, f32) {
        let (width, height) = self.viewport;
        if width == 0 {
            return (x, y);
        }
        let (scroll_y, offsets) = self.drawn_offsets();
        let viewport = Viewport {
            width: width as f32,
            height: height as f32,
            scroll_y,
        };
        let layout = self.layout(width as f32);
        let mut points: Vec<(f32, f32)> = vec![(x, y)];
        let mut passed = 0u32;
        for step in paint_order(&layout.roots, viewport, &offsets) {
            if passed > 0 {
                match step {
                    Step::PushGroup { .. } => passed += 1,
                    Step::PopGroup => passed -= 1,
                    _ => {}
                }
                continue;
            }
            match step {
                Step::Box { fragment, rect, .. }
                    if fragment.node == node && fragment.generated.is_none() =>
                {
                    let (px, py) = points[points.len() - 1];
                    let padding = padding_box(fragment, rect);
                    return (px - padding.x, py - padding.y);
                }
                Step::PushGroup { matrix, .. } => {
                    let (px, py) = points[points.len() - 1];
                    match matrix.inverse().and_then(|inverse| inverse.project(px, py)) {
                        Some(inside) => points.push(inside),
                        None => passed = 1,
                    }
                }
                Step::PopGroup if points.len() > 1 => {
                    points.pop();
                }
                _ => {}
            }
        }
        (x, y)
    }

    /// The element and its element ancestors.
    fn chain(&self, node: NodeId) -> Vec<NodeId> {
        let document = &self.shown.document;
        std::iter::once(node)
            .chain(document.ancestors(node))
            .filter(|&node| document.is_element(node))
            .collect()
    }

    /// The nearest element that is an inclusive ancestor of both, where a
    /// click goes (UI Events section 4.3.3).
    fn common_ancestor(&self, a: NodeId, b: NodeId) -> Option<NodeId> {
        let document = &self.shown.document;
        if !document.contains(a) || !document.contains(b) {
            return None;
        }
        let around_b = self.chain(b);
        self.chain(a)
            .into_iter()
            .find(|node| around_b.contains(node))
    }

    /// Sets `flag` on the elements of `chain`, clearing it from those
    /// that had it in the chain before: the active elements, the only
    /// ones kept so.
    fn set_flag(&mut self, flag: ElementState, chain: Vec<NodeId>) {
        let old = std::mem::replace(&mut self.shown.active, chain);
        let document = &mut self.shown.document;
        for &node in old.iter().filter(|node| !self.shown.active.contains(node)) {
            let _ = document.set_state(node, document.state(node).with(flag, false));
        }
        for &node in &self.shown.active {
            let _ = document.set_state(node, document.state(node).with(flag, true));
        }
        let changes = self.shown.document.take_changes();
        if !changes.is_empty() {
            self.document_changed(changes);
        }
    }

    /// The link `node` is in: the nearest `a` or `area` with an `href` at
    /// or above it, and where it goes.
    fn link_around(&self, node: NodeId) -> Option<Link> {
        let document = &self.shown.document;
        let (node, href) = std::iter::once(node)
            .chain(document.ancestors(node))
            .find_map(|node| {
                matches!(document.tag(node), Some("a" | "area"))
                    .then(|| document.attr(node, "href").map(|href| (node, href)))
                    .flatten()
            })?;
        let url = self.shown.page.resolve(document, href)?;
        Some(Link {
            page: self.shown.page.id(),
            node,
            url,
        })
    }

    /// A click's activation behaviour (HTML section 6.5) when no script
    /// runs it: the link it is in is followed.
    fn activate(&mut self, node: NodeId) {
        if let Some(link) = self.link_around(node) {
            self.follow_link(&link);
        }
    }

    /// The hovered elements as the pointer and the page now stand: the
    /// element under the pointer and its ancestors.
    fn update_hover(&mut self) {
        let hit = self.pointer.and_then(|(x, y)| self.hit_test(x, y));
        let document = &self.shown.document;
        let chain: Vec<NodeId> = hit
            .into_iter()
            .flat_map(|node| std::iter::once(node).chain(document.ancestors(node)))
            .filter(|&node| document.is_element(node))
            .collect();
        self.set_hovered(chain);
    }

    /// Hovers `chain`, unhovering the elements hovered before that are
    /// not in it; the restyle follows from the state's changes.
    fn set_hovered(&mut self, chain: Vec<NodeId>) {
        if chain == self.shown.hovered {
            return;
        }
        let old = std::mem::replace(&mut self.shown.hovered, chain);
        let document = &mut self.shown.document;
        for &node in old.iter().filter(|node| !self.shown.hovered.contains(node)) {
            let state = document.state(node).with(ElementState::HOVER, false);
            let _ = document.set_state(node, state);
        }
        for &node in &self.shown.hovered {
            let state = document.state(node).with(ElementState::HOVER, true);
            let _ = document.set_state(node, state);
        }
        let changes = self.shown.document.take_changes();
        if !changes.is_empty() {
            self.document_changed(changes);
        }
    }

    /// `:target` (M26.4): the element the document's URL's fragment
    /// indicates, as it now stands.
    fn update_target(&mut self) {
        let target =
            self.shown.page.url().and_then(Url::fragment).and_then(
                |fragment| match indicated_part(&self.shown.document, fragment) {
                    Some(Indicated::Element(element)) => Some(element),
                    _ => None,
                },
            );
        if target == self.shown.target {
            return;
        }
        let document = &mut self.shown.document;
        if let Some(old) = self.shown.target {
            let _ = document.set_state(old, document.state(old).with(ElementState::TARGET, false));
        }
        if let Some(new) = target {
            let _ = document.set_state(new, document.state(new).with(ElementState::TARGET, true));
        }
        self.shown.target = target;
        let changes = self.shown.document.take_changes();
        if !changes.is_empty() {
            self.document_changed(changes);
        }
    }

    /// The cursor over the viewport point (CSS UI 4 section 4.1, M26.4):
    /// the `cursor` of the element there; `auto` an I-beam over a text
    /// control, the arrow elsewhere and off the page.
    pub fn cursor_at(&mut self, x: f32, y: f32) -> Cursor {
        let Some(node) = self.hit_test(x, y) else {
            return Cursor::Default;
        };
        let cursor = self
            .cached_styles()
            .get(node)
            .map_or(Cursor::Auto, |style| style.cursor);
        match cursor {
            Cursor::Auto if dom::forms::is_text_control(&self.shown.document, node) => Cursor::Text,
            Cursor::Auto => Cursor::Default,
            other => other,
        }
    }

    /// HTML's focus fixup (section 6.6.4) at a rendering (M26.6): an
    /// element focused that can no longer take the focus, disabled or no
    /// longer rendered by the last frame's styles, loses it, with `blur`.
    /// The styles are not brought up to date here, so that the frame's
    /// own style stage makes its animations.
    fn focus_fixup(&mut self) {
        let Some(focused) = self.shown.document.focused() else {
            return;
        };
        let Some(styled) = &self.shown.styled else {
            return;
        };
        if !focus::focusable(&self.shown.document, Some(&styled.styles), focused) {
            self.focus(None, false, false);
        }
    }

    /// HTML's autofocus (section 6.6.5) at a rendering (M26.6): the first
    /// element with `autofocus` that the last frame's styles render and
    /// that can take the focus takes it, shown, once a document, unless
    /// something has had it first. Looked for again only when the styles
    /// have been brought up to a new version of the document.
    fn autofocus(&mut self) {
        if self.shown.autofocused {
            return;
        }
        if self.shown.document.focused().is_some() {
            self.shown.autofocused = true;
            return;
        }
        let Some(styled) = &self.shown.styled else {
            return;
        };
        if self.shown.autofocus_seen == Some(self.shown.styled_at) {
            return;
        }
        self.shown.autofocus_seen = Some(self.shown.styled_at);
        let document = &self.shown.document;
        let candidate = document.subtree(document.root()).find(|&node| {
            document.attr(node, "autofocus").is_some()
                && focus::focusable(document, Some(&styled.styles), node)
        });
        if let Some(candidate) = candidate {
            self.shown.autofocused = true;
            self.focus(Some(candidate), true, true);
        }
    }

    /// The document node as an event's target.
    fn document_target(&self) -> js::Target {
        js::Target::Node(scripts::handle(self.shown.document.root()))
    }

    /// The shown document's lifecycle events (HTML section 8.4.6, "the
    /// end", M26), each once, as its load goes on and in the reference
    /// browser's order: `readystatechange` as its parse ends, before its
    /// deferred scripts run; `DOMContentLoaded` once they have; and, once
    /// it has loaded, `readystatechange` again, `load` at the window, its
    /// target the document, and `pageshow`.
    fn lifecycle(&mut self) {
        if !self.scripting || self.shown.made || self.shown.parsing.is_some() {
            return;
        }
        let document = self.document_target();
        if !self.shown.lifecycle.interactive {
            self.shown.lifecycle.interactive = true;
            self.fire(document, js::EventInit::new("Event", "readystatechange"));
        }
        if !self.shown.lifecycle.content_loaded && self.shown.scripts.deferred_settled() {
            self.shown.lifecycle.content_loaded = true;
            self.fire(
                document,
                js::EventInit::new("Event", "DOMContentLoaded")
                    .with("bubbles", js::Field::Bool(true)),
            );
        }
        if !self.shown.lifecycle.loaded
            && self.shown.lifecycle.content_loaded
            && self.load_state == LoadState::Loaded
        {
            self.shown.lifecycle.loaded = true;
            self.fire(document, js::EventInit::new("Event", "readystatechange"));
            self.fire(js::Target::Window, js::EventInit::new("Event", "load"));
            self.fire(
                js::Target::Window,
                js::EventInit::new("PageTransitionEvent", "pageshow")
                    .with("persisted", js::Field::Bool(false)),
            );
        }
    }

    /// Says whether the window is hidden, minimized or covered (HTML
    /// section 6.2, M26): the document gets `visibilitychange`; while
    /// hidden it draws no animation frame callbacks, and its timers run
    /// once every [`HIDDEN_TIMER_PERIOD`] at most.
    pub fn set_hidden(&mut self, hidden: bool) {
        if self.hidden == hidden {
            return;
        }
        self.hidden = hidden;
        self.hidden_ran = None;
        let document = self.document_target();
        self.fire(
            document,
            js::EventInit::new("Event", "visibilitychange").with("bubbles", js::Field::Bool(true)),
        );
    }

    /// HTML's "update the rendering" before a frame at `now` (section
    /// 8.1.7.3, M26), in its order: `resize` at the window when the size
    /// changed since the last frame, `scroll` at the document and at each
    /// element that scrolled, the animation and transition events the
    /// last frames queued, then the animation frame callbacks, given the
    /// frame's time on the document's clock. The first frame fires none
    /// of the first two: it sets what the next compares with.
    fn update_the_rendering(&mut self, width: u32, height: u32, now: Duration) {
        // The window's size is the frame's from now on.
        self.viewport = (width, height);
        let events = std::mem::take(&mut self.pending_events);
        self.focus_fixup();
        self.autofocus();
        if !self.scripting || self.shown.made {
            return;
        }
        let rendered = Rendered {
            size: (width, height),
            scroll_y: self.shown.scroll_y,
            element_scroll: self
                .shown
                .element_scroll
                .iter()
                .map(|(&node, scroll)| (node, scroll.y))
                .collect(),
        };
        let last = self.shown.rendered.replace(rendered.clone());
        if let Some(last) = last {
            if last.size != rendered.size {
                self.fire(js::Target::Window, js::EventInit::new("Event", "resize"));
            }
            if last.scroll_y != rendered.scroll_y {
                let document = self.document_target();
                self.fire(
                    document,
                    js::EventInit::new("Event", "scroll").with("bubbles", js::Field::Bool(true)),
                );
            }
            let mut scrolled: Vec<NodeId> = rendered
                .element_scroll
                .iter()
                .filter(|&(node, y)| last.element_scroll.get(node).copied().unwrap_or(0.0) != *y)
                .map(|(&node, _)| node)
                .collect();
            scrolled.sort_by_key(|node| node.to_bits());
            for node in scrolled {
                if self.shown.document.contains(node) {
                    self.fire(
                        js::Target::Node(scripts::handle(node)),
                        js::EventInit::new("Event", "scroll"),
                    );
                }
            }
        }
        for event in events {
            if self.shown.document.contains(event.node) {
                self.fire(
                    js::Target::Node(scripts::handle(event.node)),
                    animation_event(&event),
                );
            }
        }
        if !self.hidden && self.shown.scripts.wants_frame() {
            let origin = self
                .shown
                .scripts
                .origin()
                .map_or(Duration::ZERO, |origin| origin.at);
            let time = now.saturating_sub(origin).as_secs_f64() * 1000.0;
            let ready = self.ready_state();
            let window = self.window_env();
            let shown = &mut self.shown;
            let mut lent = scripts::Lent {
                document: &mut shown.document,
                page: &mut shown.page,
                parser: shown.parsing.as_mut().map(|parsing| &mut parsing.load),
                network: &mut self.network,
                ready,
                window,
            };
            shown.scripts.run_frames(&mut lent, time);
            self.scripts_ran();
        }
    }

    /// Gives the engine the clock its tasks, timers and scripts read
    /// (M26): the shell's, which its frames' times come from too.
    pub fn set_clock(&mut self, clock: Clock) {
        self.clock = clock;
    }

    /// The clock now.
    fn now(&self) -> Duration {
        (self.clock)()
    }

    /// A time origin for a document made now: the clock, and the time
    /// since the Unix epoch.
    fn time_origin(&self) -> scripts::TimeOrigin {
        let epoch = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        scripts::TimeOrigin {
            at: self.now(),
            epoch_ms: epoch.as_secs_f64() * 1000.0,
        }
    }

    /// Runs the tasks that are due (HTML's event loop, M26.1): the timers
    /// of the document shown and of the one on its way, the soonest due
    /// first, each followed by its microtask checkpoint and by what the
    /// engine does after any run, until none is due or [`TASK_SLICE`] has
    /// passed, the rest being left for the next call. Whether a frame is
    /// wanted.
    pub fn run_tasks(&mut self) -> bool {
        let start = self.now();
        loop {
            let now = self.now();
            if now.saturating_sub(start) >= TASK_SLICE {
                self.wake();
                break;
            }
            let shown = self.shown_task_at().filter(|due| *due <= now);
            let incoming = self
                .incoming_scripts()
                .and_then(scripts::Scripts::next_task_at)
                .filter(|due| *due <= now);
            match (shown, incoming) {
                (None, None) => break,
                (Some(shown), Some(incoming)) if shown <= incoming => self.run_shown_timer(),
                (Some(_), None) => self.run_shown_timer(),
                (_, Some(_)) => self.run_incoming_timer(),
            }
        }
        self.needs_frame()
    }

    /// When the next task is due (M26): the soonest timer of the
    /// document shown or of the one on its way, for the caller to wake
    /// at and call [`Engine::run_tasks`].
    pub fn next_task_at(&self) -> Option<Duration> {
        let shown = self.shown_task_at();
        let incoming = self
            .incoming_scripts()
            .and_then(scripts::Scripts::next_task_at);
        match (shown, incoming) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// When the shown document's next timer is due: no sooner than
    /// [`HIDDEN_TIMER_PERIOD`] after the last while the window is hidden.
    fn shown_task_at(&self) -> Option<Duration> {
        let due = self.shown.scripts.next_task_at()?;
        match self.hidden_ran {
            Some(ran) if self.hidden => Some(due.max(ran + HIDDEN_TIMER_PERIOD)),
            _ => Some(due),
        }
    }

    /// The scripts of the document on its way, while it is parsed.
    fn incoming_scripts(&self) -> Option<&scripts::Scripts> {
        match &self.navigation {
            Some(Navigation {
                incoming: Incoming::Parsing { parsing, .. },
                ..
            }) => Some(&parsing.scripts),
            _ => None,
        }
    }

    /// Runs the shown document's timer due first as a task.
    fn run_shown_timer(&mut self) {
        let Some((id, nesting)) = self.shown.scripts.take_next_timer() else {
            return;
        };
        if self.hidden {
            self.hidden_ran = Some(self.now());
        }
        let ready = self.ready_state();
        let window = self.window_env();
        let shown = &mut self.shown;
        let mut lent = scripts::Lent {
            document: &mut shown.document,
            page: &mut shown.page,
            parser: shown.parsing.as_mut().map(|parsing| &mut parsing.load),
            network: &mut self.network,
            ready,
            window,
        };
        shown
            .scripts
            .enter(&mut lent, &js::Task::Timer(id), nesting);
        self.scripts_ran();
    }

    /// Runs the timer due first of the document on its way as
    /// a task, as its parse runs its scripts: a navigation it asks for
    /// takes the navigation's place, and the sheets it makes are found.
    fn run_incoming_timer(&mut self) {
        let media = self.media(self.viewport);
        let window = self.window_env();
        let Some(Navigation {
            incoming:
                Incoming::Parsing {
                    page,
                    document,
                    parsing,
                },
            ..
        }) = &mut self.navigation
        else {
            return;
        };
        let loader::Parsing { load, scripts, .. } = &mut **parsing;
        let Some((id, nesting)) = scripts.take_next_timer() else {
            return;
        };
        let mut lent = scripts::Lent {
            document,
            page,
            parser: Some(load),
            network: &mut self.network,
            ready: scripts::ReadyState::Loading,
            window,
        };
        scripts.enter(&mut lent, &js::Task::Timer(id), nesting);
        let asked = scripts.take_navigation();
        let changes = document.take_changes();
        if changes
            .iter()
            .any(|change| loader::may_touch_sheets(document, change))
        {
            page.rescan(document, &mut self.network, Some(&media));
        }
        if let Some(navigation) = asked {
            self.incoming_navigation(navigation);
        }
    }

    /// The shown document's `readyState` (HTML section 3.1.5): `loading`
    /// while it is parsed, `interactive` until its load event, then
    /// `complete` (M26: as its lifecycle events have it).
    fn ready_state(&self) -> scripts::ReadyState {
        if self.shown.parsing.is_some() {
            scripts::ReadyState::Loading
        } else if self.shown.lifecycle.loaded
            || (!self.scripting && self.load_state == LoadState::Loaded)
        {
            scripts::ReadyState::Complete
        } else {
            scripts::ReadyState::Interactive
        }
    }

    /// Fetches `url`'s document, for `handling` once it is shown. The
    /// browser's own `about:` pages are made at once instead: HTML leaves
    /// every `about:` URL but `about:blank` to the user agent, and this
    /// one has `about:silver`, and an error page for any other.
    fn start_navigation(&mut self, url: Url, handling: Handling) {
        self.cancel_navigation();
        if url.scheme() == "about" && url.path_str() != "blank" {
            let shown = if url.path_str() == "silver" {
                let mut page = Page::fetched(
                    self.new_page_id(),
                    url.clone(),
                    encoding::UTF_8,
                    None,
                    net::AddressSpace::Public,
                );
                let document = parsed_or_empty(pages::ABOUT_SILVER);
                let media = self.media(self.viewport);
                page.begin(&document, &mut self.network, &media);
                let mut shown = Shown::new(document, page);
                shown.salvageable = false;
                shown.made = true;
                shown
            } else {
                let error = LoadError::Network(NetworkError::BadScheme("about".to_owned()));
                self.made(pages::error(&url, &error), Some(error))
            };
            self.commit(shown, Some(url), handling);
            return;
        }
        let id = self.new_page_id();
        let request = net::Request::get(url.clone(), net::Destination::Document);
        self.network.fetch(id, Target::Document, request);
        self.navigation = Some(Navigation {
            url,
            incoming: Incoming::Fetching(id),
            handling,
            started: self.time_origin(),
        });
        self.load_state = LoadState::Loading;
    }

    /// A page the browser made, `document`, with no URL of its own so
    /// that nothing in it resolves against the URL it is shown under,
    /// and never kept by the back-forward cache; `failure` for an error
    /// page.
    fn made(&mut self, document: Document, failure: Option<LoadError>) -> Shown {
        let mut page = Page::given(self.new_page_id());
        let media = self.media(self.viewport);
        page.begin(&document, &mut self.network, &media);
        let mut shown = Shown::new(document, page);
        shown.salvageable = false;
        shown.failure = failure;
        shown.made = true;
        shown
    }

    /// Takes the response to the shown document's icon: kept when its
    /// status is ok, its computed type an image type and its body no
    /// larger than [`MAX_ICON_BYTES`]. HTML's `load` event does not wait
    /// for it, and neither does the load state.
    fn icon_arrived(&mut self, page: PageId, result: Result<net::Response, LoadError>) {
        if page != self.shown.page.id() {
            return;
        }
        let Ok(response) = result else {
            return;
        };
        let computed = loader::computed_type(&response);
        if response.ok() && computed.is_image() && response.body.len() <= MAX_ICON_BYTES {
            self.shown.icon = Some(Icon {
                url: response.url().clone(),
                mime_type: computed.essence().to_owned(),
                bytes: response.body,
            });
        }
    }

    /// The document's icon (HTML's `icon` link type, else
    /// `/favicon.ico`), once it has arrived; none for a document without
    /// one, or whose icon failed.
    pub fn icon(&self) -> Option<&Icon> {
        self.shown.icon.as_ref()
    }

    /// The document's title, as `document.title` reads it: the text of
    /// the first `title` element in tree order, its own text children
    /// joined, with ASCII whitespace stripped from its ends and each run
    /// of it inside collapsed to one space. Empty when there is none.
    pub fn title(&self) -> String {
        scripts::document_title(&self.shown.document)
    }

    /// Runs scripts from now on, or not (M24): they are off unless asked
    /// for, since no untrusted script is to run before the isolation of
    /// M33 and M34 (roadmap risk R2). A document already loaded keeps how
    /// it was parsed; the `scripting` media feature follows at once.
    pub fn set_scripting(&mut self, on: bool) {
        self.scripting = on;
        let media = self.media(self.viewport);
        if self
            .shown
            .styled
            .as_ref()
            .is_some_and(|cached| self.shown.stylesheets.media_changed(&cached.styles, &media))
        {
            self.changed = true;
        }
    }

    /// Whether scripts run.
    pub fn scripting(&self) -> bool {
        self.scripting
    }

    /// Sends D12's compatibility user agent string in requests and
    /// `navigator`, for a page that refuses an engine it does not know
    /// (the shell's `--compat-user-agent`); never the default. Takes the
    /// place of the client: a test's own goes.
    pub fn set_compat_user_agent(&mut self, on: bool) {
        self.compat_user_agent = on;
        let agent = if on {
            net::COMPAT_USER_AGENT
        } else {
            net::USER_AGENT
        };
        self.network
            .set_client(net::Client::default().with_user_agent(agent));
    }

    /// The shown document's console since it was last taken (M24.7):
    /// what its scripts wrote, and the errors they reported; after what
    /// a document left wrote as it was left (M26).
    pub fn take_console(&mut self) -> Vec<ConsoleEntry> {
        let mut entries = std::mem::take(&mut self.departed);
        entries.extend(self.shown.scripts.take_console());
        entries
    }

    /// Where the shown document's scripting stands (M24.7).
    pub fn scripts(&self) -> ScriptState {
        if self.scripting {
            self.shown.scripts.state()
        } else {
            ScriptState::Off
        }
    }

    /// What the user should be told of the latest navigation that
    /// showed no page, once: a download this version cannot save. The
    /// shell shows it in the status bubble.
    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    /// Aborts the navigation under way, if there is one.
    fn cancel_navigation(&mut self) {
        if let Some(navigation) = self.navigation.take() {
            self.network.cancel(navigation.incoming.id(), None);
        }
    }

    /// Goes back one entry in the session history, if there is one
    /// before the current one: [`Engine::traverse`] by -1.
    pub fn go_back(&mut self) {
        self.traverse(-1);
    }

    /// Goes forward one entry, if there is one after the current one.
    pub fn go_forward(&mut self) {
        self.traverse(1);
    }

    /// Whether there is an entry before the current one.
    pub fn can_go_back(&self) -> bool {
        self.history.can_go_back()
    }

    /// Whether there is an entry after the current one.
    pub fn can_go_forward(&self) -> bool {
        self.history.can_go_forward()
    }

    /// Makes the entry `delta` away from the current one current, as
    /// HTML's "traverse the history by a delta" does; a delta past
    /// either end does nothing, and 0 reloads. A navigation under way is
    /// aborted, and the current entry's scroll position saved. An entry
    /// of the same document state shows the same document, at the
    /// entry's URL; another shows the document the back-forward cache
    /// kept for it, whole, or, when it kept none, its URL is fetched
    /// again for it. Either way the entry's scroll position is restored.
    pub fn traverse(&mut self, delta: isize) {
        if delta == 0 {
            self.reload();
            return;
        }
        let Some((index, entry)) = self.history.by_delta(delta) else {
            return;
        };
        let (state, url, scroll) = (entry.state, entry.url.clone(), entry.scroll);
        self.cancel_navigation();
        self.history.current_mut().scroll = self.shown.scroll_target;
        let left_state = self.history.current().state;
        if state == left_state {
            self.history.go_to(index);
            if let Some(url) = url {
                self.shown.page.set_url(url);
            }
            self.goal = Some(ScrollGoal::Offset(scroll));
            self.changed = true;
            self.settle_load_state();
            self.update_target();
        } else if let Some(kept) = self.history.take_kept(state) {
            self.history.go_to(index);
            let left = self.show(*kept);
            self.leave(left, left_state);
            self.goal = Some(ScrollGoal::Offset(scroll));
            self.settle_load_state();
            // Shown again from the cache (M26).
            self.fire(
                js::Target::Window,
                js::EventInit::new("PageTransitionEvent", "pageshow")
                    .with("persisted", js::Field::Bool(true)),
            );
        } else if let Some(url) = url {
            self.start_navigation(url, Handling::Traverse(index));
        }
    }

    /// Fetches the current entry's URL again and shows the new document
    /// in the same entry, at its scroll position, as HTML's reload does.
    /// A document given to the engine has no URL, and is not reloaded.
    pub fn reload(&mut self) {
        if let Some(url) = self.history.current().url.clone() {
            self.start_navigation(url, Handling::Reload);
        }
    }

    /// HTML's "stop loading": the navigation under way is aborted, and so
    /// are the shown document's own fetches. Its stylesheets aborted
    /// that way count as failed, and the load is over. A document whose
    /// fetches were aborted is not kept by the back-forward cache.
    pub fn stop(&mut self) {
        self.cancel_navigation();
        if let Some(parsing) = &mut self.shown.parsing {
            parsing.load.end();
            self.wake();
        }
        self.shown.scripts.abort();
        if self.shown.page.abort(&mut self.network) {
            self.shown.salvageable = false;
            self.sheets_changed();
        }
        self.settle_load_state();
    }

    /// Takes every fetch result that has arrived and applies it: a
    /// document is parsed, and shown once its blocking stylesheets have
    /// arrived or failed; a stylesheet takes its place among the
    /// document's, which restyles it. Says whether a frame is wanted,
    /// as [`Engine::needs_frame`] does. The shell calls it when the
    /// waker wakes it.
    pub fn poll_network(&mut self) -> bool {
        while let Some(taken) = self.network.take() {
            match taken {
                loader::Taken::Arrived(arrived) => self.arrived(arrived),
                loader::Taken::Head { page, response } => self.document_head(page, response),
                loader::Taken::Body { page, bytes } => self.document_body(page, &bytes),
            }
        }
        self.parse_incoming();
        self.parse_shown();
        self.run_scripts();
        while let Some(finished) = self.decoder.take() {
            self.image_read(finished);
        }
        self.needs_frame()
    }

    /// Whether an image a frame drew is being decoded (M23.5): a frame
    /// drawn once it is shows it, as one drawn now does not. The waker
    /// wakes the caller when it is done.
    pub fn decoding(&self) -> bool {
        self.shown.images.decoding()
    }

    /// Fetches the images the shown document's `img` elements show that
    /// are not yet wanted (M23.1). A page with no URL fetches nothing,
    /// nor does one the browser made.
    fn request_images(&mut self) {
        if self.shown.made {
            return;
        }
        let Some(initiator) = self.shown.page.initiator() else {
            return;
        };
        let shown = &mut self.shown;
        let wanted = shown
            .images
            .scan(&shown.document, &shown.page, &mut self.next_image);
        for url in wanted {
            let key = url.as_str().to_owned();
            let request = net::Request::get(url, net::Destination::Image)
                .initiated_by(initiator.clone())
                .with_body_limit(images::MAX_IMAGE_BYTES);
            self.network
                .fetch(shown.page.id(), Target::Image { url: key }, request);
        }
        // An element showing an image that came before it was made is
        // told now (M26).
        for url in self.shown.images.known() {
            self.tell_images(&url);
        }
    }

    /// Fetches the images the shown document's styles name that are not
    /// yet wanted (M23.3), as [`Engine::request_images`] does its `img`s'.
    fn request_css_images(&mut self) {
        if self.shown.made {
            return;
        }
        let Some(initiator) = self.shown.page.initiator() else {
            return;
        };
        let shown = &mut self.shown;
        let Some(styled) = &shown.styled else {
            return;
        };
        let wanted = shown.images.scan_styles(
            &shown.document,
            &styled.styles,
            &shown.page,
            &mut self.next_image,
        );
        for url in wanted {
            let key = url.as_str().to_owned();
            let request = net::Request::get(url, net::Destination::Image)
                .initiated_by(initiator.clone())
                .with_body_limit(images::MAX_IMAGE_BYTES);
            self.network
                .fetch(shown.page.id(), Target::Image { url: key }, request);
        }
    }

    /// Takes the response to one of the shown document's images: its
    /// header is read next, off this thread.
    fn image_arrived(&mut self, page: PageId, url: &str, result: Result<net::Response, LoadError>) {
        if page != self.shown.page.id() {
            return;
        }
        match self
            .shown
            .images
            .fetched(url, result.ok().map(|response| response.body))
        {
            Some(work) => self.decoder.start(page, url.to_owned(), work),
            None => self.image_changed(url),
        }
    }

    /// Takes a decode thread's result for the shown document.
    fn image_read(&mut self, finished: image_store::Finished) {
        if finished.page != self.shown.page.id() {
            return;
        }
        let image_store::Finished {
            url, done, bytes, ..
        } = finished;
        let now = self.shown.animations.borrow().time();
        if self.shown.images.done(&url, done, Some(bytes), now) {
            self.image_changed(&url);
        } else {
            // New pixels to draw; nothing to lay out.
            self.changed = true;
        }
    }

    /// An image's size or brokenness changed: the boxes showing it are
    /// laid out again.
    fn image_changed(&mut self, url: &str) {
        self.tell_images(url);
        let users = self.shown.images.users(url);
        if !users.is_empty() {
            self.shown
                .layout_cache
                .invalidate(&self.shown.document, &[], &users);
            self.shown.layout = None;
        }
        // A marker's or a `content`'s image (M23.3): which boxes show it
        // is not kept, and such an arrival is rare: all are laid out.
        if self.shown.images.lays_out(url) {
            self.shown.layout_cache.clear();
            self.shown.layout = None;
        }
        self.changed = true;
        self.update_load_state();
    }

    /// Tells the `img` elements showing `url` whether it came, once it is
    /// known (M26): `load` once its size is, `error` if it is broken;
    /// each element once for each URL.
    fn tell_images(&mut self, url: &str) {
        let Some(broken) = self.shown.images.outcome(url) else {
            return;
        };
        let kind = if broken { "error" } else { "load" };
        for node in self.shown.images.users(url) {
            if self.shown.document.tag(node) != Some("img")
                || self.shown.told.get(&node).is_some_and(|told| told == url)
            {
                continue;
            }
            self.shown.told.insert(node, url.to_owned());
            self.fire(
                js::Target::Node(scripts::handle(node)),
                js::EventInit::new("Event", kind),
            );
        }
    }

    /// The pixels of the shown document's image numbered `id`, if
    /// decoded; asked for the first time, they are decoded off this
    /// thread (M23.5), and a frame drawn once they are shows them.
    fn image_pixels(&mut self, id: u64) -> Option<std::sync::Arc<images::Image>> {
        let url = self.shown.images.url_of(id)?.to_owned();
        let (pixels, work) = self.shown.images.pixels(&url);
        if let Some(work) = work {
            self.decoder.start(self.shown.page.id(), url, work);
        }
        pixels
    }

    /// Gives each image `items` draw its pixels where they are decoded,
    /// and has the others decoded (M23.2); and an animation the frame its
    /// timeline is at (M23.1). Notes when the next frame comes of those
    /// in the viewport, `width` by `height`, through their groups'
    /// matrices, or whose place cannot be told; one elsewhere moves on
    /// unseen.
    fn show_pixels(&mut self, items: &mut [DisplayItem], width: f32, height: f32) {
        let now = self.shown.animations.borrow().time();
        let frame = self.shown.images.next_frame();
        // First the size each image is drawn at and whether in view, so
        // that a decode asked for knows how large (M23.5).
        let mut seen = Vec::new();
        let mut groups = vec![style::transform::Matrix::IDENTITY];
        for item in items.iter() {
            let (image, rect, tile) = match item {
                DisplayItem::PushGroup { matrix, .. } => {
                    let outer = groups.last().expect("the viewport's");
                    groups.push(outer.then(matrix));
                    continue;
                }
                DisplayItem::PopGroup => {
                    if groups.len() > 1 {
                        groups.pop();
                    }
                    continue;
                }
                DisplayItem::Image {
                    x, y, w, h, image, ..
                } => (image, [*x, *y, *w, *h], [*x, *y, *w, *h]),
                DisplayItem::Pattern {
                    x,
                    y,
                    w,
                    h,
                    tile,
                    image,
                    ..
                } => (image, [*x, *y, *w, *h], *tile),
                _ => continue,
            };
            let matrix = groups.last().expect("the viewport's");
            seen.push(
                matrix
                    .bounds(rect[0], rect[1], rect[2], rect[3])
                    .is_none_or(|(l, t, r, b)| l < width && r > 0.0 && t < height && b > 0.0),
            );
            let size = drawn_size(matrix, tile, image);
            self.shown.images.drew(image.id, frame, size);
        }
        let mut next: Option<f64> = None;
        let mut seen = seen.into_iter();
        for item in items {
            let image = match item {
                DisplayItem::Image { image, .. } | DisplayItem::Pattern { image, .. } => image,
                _ => continue,
            };
            let seen = seen.next().unwrap_or(true);
            if image.pixels.is_none() {
                image.pixels = self.image_pixels(image.id);
            }
            let Some(pixels) = image.pixels.as_deref().filter(|p| p.is_animated()) else {
                continue;
            };
            let Some(started) = self.shown.images.started(image.id) else {
                continue;
            };
            let (shown, change) = images::frame_at(pixels, (now - started) * 1000.0);
            image.frame = shown as u32;
            if let Some(change) = change.filter(|_| seen) {
                let at = started + change / 1000.0;
                next = Some(next.map_or(at, |n| n.min(at)));
            }
        }
        self.shown.next_image_frame = next;
        self.shown.images.keep_within_budget(frame);
    }

    /// Applies one fetch's result.
    fn arrived(&mut self, arrived: loader::Arrived) {
        let loader::Arrived {
            id,
            page,
            target,
            result,
        } = arrived;
        match target {
            Target::Document => self.document_arrived(page, result),
            Target::Icon => self.icon_arrived(page, result),
            Target::Image { url } => self.image_arrived(page, &url, result),
            Target::Script => {
                // What is ready runs as the poll goes on.
                if page == self.shown.page.id() {
                    self.shown.scripts.arrived(&self.shown.page, id, result);
                } else if let Some(Navigation {
                    incoming:
                        Incoming::Parsing {
                            page: incoming,
                            parsing,
                            ..
                        },
                    ..
                }) = &mut self.navigation
                    && incoming.id() == page
                {
                    parsing.scripts.arrived(incoming, id, result);
                }
            }
            Target::Sheet { owner, path } => {
                if page == self.shown.page.id() {
                    let ok = result.as_ref().is_ok_and(loader::is_stylesheet);
                    let wanted = self.shown.page.awaits(owner, &path, id);
                    if self
                        .shown
                        .page
                        .sheet_arrived(owner, &path, id, result, &mut self.network)
                    {
                        self.sheets_changed();
                    }
                    // A link element hears whether its sheet came (M26).
                    if wanted && path.is_empty() && self.shown.document.tag(owner) == Some("link") {
                        let kind = if ok { "load" } else { "error" };
                        self.fire(
                            js::Target::Node(scripts::handle(owner)),
                            js::EventInit::new("Event", kind),
                        );
                    }
                    self.update_load_state();
                } else {
                    self.incoming_sheet_arrived(page, owner, &path, id, result);
                }
            }
        }
    }

    /// Takes the result of fetch `id` for a sheet of the document on its
    /// way, as [`Engine::arrived`] does the shown one's: a link element
    /// of a document being parsed hears whether its sheet came (M26), and
    /// the document is shown if it is now ready.
    fn incoming_sheet_arrived(
        &mut self,
        page: PageId,
        owner: NodeId,
        path: &[usize],
        id: loader::FetchId,
        result: Result<net::Response, LoadError>,
    ) {
        let window = self.window_env();
        let scripting = self.scripting;
        let mut asked = None;
        match &mut self.navigation {
            Some(Navigation {
                incoming: Incoming::Waiting { page: incoming, .. },
                ..
            }) if incoming.id() == page => {
                incoming.sheet_arrived(owner, path, id, result, &mut self.network);
            }
            Some(Navigation {
                incoming:
                    Incoming::Parsing {
                        page: incoming,
                        document,
                        parsing,
                    },
                ..
            }) if incoming.id() == page => {
                let ok = result.as_ref().is_ok_and(loader::is_stylesheet);
                let wanted = incoming.awaits(owner, path, id);
                incoming.sheet_arrived(owner, path, id, result, &mut self.network);
                if scripting && wanted && path.is_empty() && document.tag(owner) == Some("link") {
                    let loader::Parsing { load, scripts, .. } = &mut **parsing;
                    let mut lent = scripts::Lent {
                        document,
                        page: incoming,
                        parser: Some(load),
                        network: &mut self.network,
                        ready: scripts::ReadyState::Loading,
                        window,
                    };
                    let kind = if ok { "load" } else { "error" };
                    scripts.dispatch(
                        &mut lent,
                        js::Target::Node(scripts::handle(owner)),
                        js::EventInit::new("Event", kind),
                    );
                    asked = scripts.take_navigation();
                }
            }
            _ => return,
        }
        if let Some(navigation) = asked {
            self.incoming_navigation(navigation);
            return;
        }
        self.commit_if_ready();
    }

    /// The head of the response to the navigation under way's document
    /// fetch: its body is to come (M24.8).
    fn document_head(&mut self, page: PageId, response: net::Response) {
        if let Some(navigation) = &mut self.navigation
            && matches!(navigation.incoming, Incoming::Fetching(id) if id == page)
        {
            navigation.incoming = Incoming::Receiving {
                id: page,
                head: Box::new(response),
                body: Vec::new(),
                sniffed: false,
            };
        }
    }

    /// A piece of the body of a document being fetched: of the navigation
    /// under way's, kept until it is known to be HTML and then parsed as
    /// it comes; or of the document shown, still arriving (M24.8).
    fn document_body(&mut self, page: PageId, bytes: &[u8]) {
        if self.shown.page.id() == page
            && let Some(parsing) = &mut self.shown.parsing
        {
            parsing.load.push(bytes);
            return;
        }
        let Some(navigation) = &mut self.navigation else {
            return;
        };
        match &mut navigation.incoming {
            Incoming::Receiving {
                id, body, sniffed, ..
            } if *id == page => {
                body.extend_from_slice(bytes);
                if !*sniffed && body.len() >= loader::RESOURCE_HEADER {
                    self.sniff_incoming(false);
                }
            }
            Incoming::Parsing {
                page: incoming,
                parsing,
                ..
            } if incoming.id() == page => {
                parsing.load.push(bytes);
            }
            _ => {}
        }
    }

    /// Whether the navigation under way's response, received so far, or
    /// whole if `ended`, is HTML: then it is parsed from here on as it
    /// arrives, into a document that is shown once it has a body and its
    /// blocking sheets (M24.8). Otherwise it is taken whole when it ends.
    fn sniff_incoming(&mut self, ended: bool) {
        let Some(Navigation {
            incoming:
                Incoming::Receiving {
                    id,
                    head,
                    body,
                    sniffed,
                },
            ..
        }) = &mut self.navigation
        else {
            return;
        };
        let mut response = (**head).clone();
        response.body = std::mem::take(body);
        let Some(transport) = loader::streams_as_html(&response) else {
            *body = response.body;
            *sniffed = true;
            return;
        };
        let id = *id;
        let mut load = html::Load::new(transport, self.scripting);
        load.push(&response.body);
        if ended {
            load.end();
        }
        response.body = Vec::new();
        let page = parsed_page(id, &response, &load);
        if let Some(navigation) = &mut self.navigation {
            let mut scripts = scripts::Scripts::with_limits(self.script_limits);
            scripts.set_time_origin(navigation.started);
            navigation.incoming = Incoming::Parsing {
                page: Box::new(page),
                document: Box::new(Document::new()),
                parsing: Box::new(loader::Parsing {
                    load,
                    head: response,
                    scripts,
                }),
            };
        }
        self.wake();
    }

    /// Parses a slice of the navigation under way's document: the sheets
    /// the parser makes are found and fetched as they come, those before
    /// `<body>` blocking, and the document is shown once it may be. A
    /// `<meta>` that changes the encoding starts it again (M24.8).
    fn parse_incoming(&mut self) {
        let media = self.media(self.viewport);
        let window = self.window_env();
        let Some(Navigation {
            incoming:
                Incoming::Parsing {
                    page,
                    document,
                    parsing,
                },
            ..
        }) = &mut self.navigation
        else {
            return;
        };
        // The parser waits for its pending parsing-blocking script (M24.3).
        // A realm that dies says so once its document is shown.
        let loader::Parsing { load, scripts, .. } = &mut **parsing;
        let mut lent = scripts::Lent {
            document,
            page,
            parser: Some(load),
            network: &mut self.network,
            ready: scripts::ReadyState::Loading,
            window: window.clone(),
        };
        let go_on = scripts.run_ready(&mut lent, false);
        if let Some(navigation) = scripts.take_navigation() {
            self.incoming_navigation(navigation);
            return;
        }
        if !go_on {
            return;
        }
        let mut asked = None;
        match parsing.load.parse(document, PARSE_SLICE) {
            html::Progress::Restarted => {
                // The sheets found so far go, the document fetch stays.
                page.rescan(&Document::new(), &mut self.network, None);
                **page = parsed_page(page.id(), &parsing.head, &parsing.load);
                **document = Document::new();
                parsing.scripts.restart();
                self.wake();
            }
            html::Progress::Parsed(stop) => {
                if let html::Stop::Script { element, line } = stop
                    && self.scripting
                {
                    // The sheets the parser made before the script are
                    // found first: one on its way blocks it, and its
                    // `getComputedStyle` reads them (M25).
                    let changes = document.take_changes();
                    if changes
                        .iter()
                        .any(|change| loader::may_touch_sheets(document, change))
                    {
                        page.rescan(document, &mut self.network, Some(&media));
                    }
                    let loader::Parsing { load, scripts, .. } = &mut **parsing;
                    let mut lent = scripts::Lent {
                        document,
                        page,
                        parser: Some(load),
                        network: &mut self.network,
                        ready: scripts::ReadyState::Loading,
                        window: window.clone(),
                    };
                    scripts.prepare(&mut lent, element, line);
                    asked = scripts.take_navigation();
                }
                let changes = document.take_changes();
                if changes
                    .iter()
                    .any(|change| loader::may_touch_sheets(document, change))
                {
                    page.rescan(document, &mut self.network, Some(&media));
                }
                if matches!(stop, html::Stop::Slice | html::Stop::Script { .. }) {
                    self.wake();
                }
            }
        }
        if let Some(navigation) = asked {
            self.incoming_navigation(navigation);
            return;
        }
        self.commit_if_ready();
    }

    /// Parses a slice of the document shown, still arriving: what the
    /// parser adds is a change like any other, restyled and laid out by
    /// the next frame, its sheets and images fetched (M24.8).
    fn parse_shown(&mut self) {
        let window = self.window_env();
        let Shown {
            document,
            page,
            parsing: Some(parsing),
            scripts: queue,
            ..
        } = &mut self.shown
        else {
            return;
        };
        // The parser waits for its pending parsing-blocking script (M24.3).
        let mut lent = scripts::Lent {
            document,
            page,
            parser: Some(&mut parsing.load),
            network: &mut self.network,
            ready: scripts::ReadyState::Loading,
            window: window.clone(),
        };
        if !queue.run_ready(&mut lent, false) {
            self.scripts_ran();
            return;
        }
        let Some(parsing) = &mut self.shown.parsing else {
            return;
        };
        match parsing.load.parse(&mut self.shown.document, PARSE_SLICE) {
            html::Progress::Restarted => {
                // Shown already: the document starts again in place.
                let head = parsing.head.clone();
                let page = parsed_page(self.shown.page.id(), &head, &parsing.load);
                self.shown
                    .page
                    .rescan(&Document::new(), &mut self.network, None);
                let parsing = self.shown.parsing.take();
                let mut shown = Shown::new(Document::new(), page);
                shown.parsing = parsing;
                shown.scripts = std::mem::take(&mut self.shown.scripts);
                shown.scripts.restart();
                let _ = self.show(shown);
                self.wake();
            }
            html::Progress::Parsed(stop) => {
                if matches!(stop, html::Stop::Script { .. }) && self.scripting {
                    // The parser's changes are taken before the script is
                    // prepared: the sheets it made may block it, and its
                    // `getComputedStyle` reads them (M25).
                    self.scripts_ran();
                }
                if let html::Stop::Script { element, line } = stop
                    && self.scripting
                    && let Shown {
                        document,
                        page,
                        parsing: Some(parsing),
                        scripts: queue,
                        ..
                    } = &mut self.shown
                {
                    let mut lent = scripts::Lent {
                        document,
                        page,
                        parser: Some(&mut parsing.load),
                        network: &mut self.network,
                        ready: scripts::ReadyState::Loading,
                        window: window.clone(),
                    };
                    queue.prepare(&mut lent, element, line);
                }
                self.scripts_ran();
                match stop {
                    html::Stop::Done => {
                        self.shown.parsing = None;
                        self.update_load_state();
                    }
                    html::Stop::Slice | html::Stop::Script { .. } => self.wake(),
                    html::Stop::Input => {}
                }
            }
        }
    }

    /// Runs the shown document's scripts that are ready once its parse
    /// is over: those run as soon as possible, and the deferred ones in
    /// order (M24.3). The load is over once none is left.
    fn run_scripts(&mut self) {
        if self.shown.parsing.is_some() || self.shown.scripts.settled() {
            return;
        }
        let ready = self.ready_state();
        let window = self.window_env();
        let shown = &mut self.shown;
        let mut lent = scripts::Lent {
            document: &mut shown.document,
            page: &mut shown.page,
            parser: None,
            network: &mut self.network,
            ready,
            window: window.clone(),
        };
        shown.scripts.run_ready(&mut lent, true);
        self.scripts_ran();
        self.update_load_state();
    }

    /// After the shown document's scripts ran: their changes are taken as
    /// any change is, and a realm that died says so.
    fn scripts_ran(&mut self) {
        let changes = self.shown.document.take_changes();
        if !changes.is_empty() {
            self.document_changed(changes);
        }
        if let Some(node) = self.shown.scripts.take_scroll() {
            self.scroll_focus_into_view(node);
        }
        if let Some(navigation) = self.shown.scripts.take_navigation() {
            self.script_navigation(navigation);
        }
        if self.shown.scripts.take_just_stopped() {
            self.notice = Some(SCRIPTS_STOPPED.to_owned());
        }
    }

    /// Takes the response to the document fetch of the navigation under
    /// way (docs/design/navigation.md, "What a response becomes"): a
    /// network error shows an error page; a document is parsed and its
    /// stylesheets fetched; an error status with nothing in it, or a type
    /// this version cannot show, shows the browser's page saying so; a
    /// download or a 204 or 205 ends the navigation with the page shown
    /// as it was.
    fn document_arrived(&mut self, page: PageId, result: Result<net::Response, LoadError>) {
        // A document parsed as it arrives ends with what came, even if the
        // fetch failed after it began (M24.8).
        if self.shown.page.id() == page
            && let Some(parsing) = &mut self.shown.parsing
        {
            parsing.load.end();
            self.wake();
            return;
        }
        if let Some(Navigation {
            incoming:
                Incoming::Parsing {
                    page: incoming,
                    parsing,
                    ..
                },
            ..
        }) = &mut self.navigation
            && incoming.id() == page
        {
            parsing.load.end();
            self.wake();
            return;
        }
        if result.is_ok() {
            self.sniff_incoming(true);
            if matches!(
                &self.navigation,
                Some(Navigation { incoming: Incoming::Parsing { page: incoming, .. }, .. })
                    if incoming.id() == page
            ) {
                return;
            }
        }
        let Some(navigation) = self.navigation.take() else {
            return;
        };
        // A streamed response is its head as it came, with the body that
        // arrived after it in pieces.
        let result = match (result, &navigation.incoming) {
            (Ok(_), Incoming::Receiving { head, body, .. }) => {
                let mut response = (**head).clone();
                response.body = body.clone();
                Ok(response)
            }
            (result, _) => result,
        };
        if !matches!(
            navigation.incoming,
            Incoming::Fetching(id) | Incoming::Receiving { id, .. } if id == page
        ) {
            self.navigation = Some(navigation);
            return;
        }
        let Navigation {
            url,
            handling,
            started,
            ..
        } = navigation;
        let response = match result {
            Ok(response) => response,
            Err(error) => {
                let shown = self.made(pages::error(&url, &error), Some(error));
                self.commit(shown, Some(url), handling);
                return;
            }
        };
        let at = response.url().clone();
        match loader::outcome(&response) {
            loader::Outcome::Document(parsed) => {
                let address_space = response.address_space.unwrap_or(net::AddressSpace::Public);
                let mut incoming =
                    Page::fetched(page, at, parsed.encoding, parsed.undecodable, address_space)
                        .with_language(parsed.language);
                let media = self.media(self.viewport);
                incoming.begin(&parsed.document, &mut self.network, &media);
                self.navigation = Some(Navigation {
                    url,
                    incoming: Incoming::Waiting {
                        page: Box::new(incoming),
                        document: Box::new(parsed.document),
                    },
                    handling,
                    started,
                });
                self.commit_if_ready();
            }
            loader::Outcome::Status => {
                let document = pages::status(&at, response.status, &response.status_message);
                let shown = self.made(document, None);
                self.commit(shown, Some(at), handling);
            }
            loader::Outcome::Unshowable(essence) => {
                let shown = self.made(pages::unshowable(&at, &essence), None);
                self.commit(shown, Some(at), handling);
            }
            loader::Outcome::Download(essence) => {
                self.notice = Some(format!(
                    "Not saved: {at} is a {essence} file, and saving files comes in a later version"
                ));
                self.settle_load_state();
            }
            loader::Outcome::Nothing => self.settle_load_state(),
        }
    }

    /// Shows the incoming document once its blocking stylesheets have
    /// arrived or failed.
    fn commit_if_ready(&mut self) {
        // A document being parsed is shown once it is no longer
        // render-blocked (HTML section 3.1.6): it has a body, or is parsed
        // whole, and its blocking sheets have come.
        let parsed_ready = matches!(
            &self.navigation,
            Some(Navigation { incoming: Incoming::Parsing { page, document, parsing }, .. })
                if page.blocking_settled() && (has_body(document) || parsing.load.is_done())
        );
        if parsed_ready {
            let Some(Navigation {
                incoming:
                    Incoming::Parsing {
                        page,
                        document,
                        parsing,
                    },
                handling,
                ..
            }) = self.navigation.take()
            else {
                unreachable!("checked above");
            };
            let url = page.url().cloned();
            let mut parsing = parsing;
            let mut shown = Shown::new(*document, *page);
            shown.scripts = std::mem::take(&mut parsing.scripts);
            shown.parsing = Some(parsing);
            self.commit(shown, url, handling);
            return;
        }
        let ready = matches!(
            &self.navigation,
            Some(Navigation { incoming: Incoming::Waiting { page, .. }, .. })
                if page.blocking_settled()
        );
        if !ready {
            return;
        }
        let Some(Navigation {
            incoming: Incoming::Waiting { page, document },
            handling,
            ..
        }) = self.navigation.take()
        else {
            unreachable!("checked above");
        };
        let url = page.url().cloned();
        self.commit(Shown::new(*document, *page), url, handling);
    }

    /// Shows a navigation's document, `shown`, at `url`, and puts it in
    /// the session history as `handling` says: in a new entry pushed or
    /// put in the current one's place, the document left kept or
    /// dropped; or in the entry reloaded or traversed to, at its saved
    /// scroll position.
    fn commit(&mut self, shown: Shown, url: Option<Url>, handling: Handling) {
        self.history.current_mut().scroll = self.shown.scroll_target;
        let left_state = self.history.current().state;
        // A new document at a URL with a fragment is scrolled to it.
        let mut goal = url
            .as_ref()
            .and_then(Url::fragment)
            .map(|fragment| ScrollGoal::Fragment(fragment.to_owned()));
        match handling {
            Handling::Push => {
                let state = self.history.new_state();
                self.history.push(url, state);
            }
            Handling::Replace => {
                let state = self.history.new_state();
                self.history.replace(url, state);
            }
            Handling::Reload => {
                let entry = self.history.current_mut();
                entry.url = url;
                goal = Some(ScrollGoal::Offset(entry.scroll));
            }
            Handling::Traverse(index) => {
                self.history.go_to(index);
                let entry = self.history.entry_mut(index);
                entry.url = url;
                goal = Some(ScrollGoal::Offset(entry.scroll));
            }
        }
        let left = self.show(shown);
        if !self.shown.made
            && let Some(request) = self.shown.page.icon_request(&self.shown.document)
        {
            self.network
                .fetch(self.shown.page.id(), Target::Icon, request);
        }
        self.request_images();
        if handling == Handling::Reload {
            // The entry's state has the new document; the old one is
            // gone, not kept.
            let mut left = left;
            self.unload(&mut left, false);
            self.network.cancel(left.page.id(), None);
        } else {
            self.leave(left, left_state);
        }
        self.goal = goal;
        self.settle_load_state();
    }

    /// The load is done once its document is shown and parsed, every
    /// stylesheet and image it asked for has arrived or failed, and its
    /// deferred and async scripts have run, as the load event waits.
    fn update_load_state(&mut self) {
        if self.load_state == LoadState::Loading
            && self.navigation.is_none()
            && self.shown.parsing.is_none()
            && self.shown.scripts.settled()
            && self.shown.page.settled()
            && self.shown.images.settled()
        {
            // The images the styles name are waited for too (M23.3): the
            // styles are computed now, if no frame has, to find them.
            self.cached_styles();
            if self.shown.images.settled() {
                self.load_state = LoadState::Loaded;
            }
        }
        self.update_target();
        self.lifecycle();
    }

    /// The load state as the document shown and the navigation under way
    /// make it, after a document is shown or a load stopped: failed while
    /// an error page is shown.
    fn settle_load_state(&mut self) {
        self.load_state = if self.navigation.is_some()
            || self.shown.parsing.is_some()
            || !self.shown.page.settled()
            || !self.shown.images.settled()
        {
            LoadState::Loading
        } else if let Some(failure) = &self.shown.failure {
            LoadState::Failed(failure.clone())
        } else {
            // Loaded once the images its styles name are in (M23.3).
            self.load_state = LoadState::Loading;
            self.update_load_state();
            return;
        };
    }

    /// Takes the stylesheets that apply as they now stand: when they
    /// differ from those the cascade has, the next frame cascades the
    /// whole document.
    fn sheets_changed(&mut self) {
        let sheets = self.shown.page.sheets();
        if sheets != self.shown.stylesheets.sheets() {
            self.shown.stylesheets = Stylesheets::new(sheets);
            self.shown.recascade = true;
            self.shown.pending.clear();
            self.shown.layout = None;
            self.changed = true;
        }
    }

    /// Where the latest navigation stands: failed while the error page
    /// it ended in is shown. A document given to the engine, and the
    /// initial `about:blank`, are loaded.
    pub fn load_state(&self) -> &LoadState {
        &self.load_state
    }

    /// The current entry's URL: the URL of the document shown, which a
    /// fragment navigation changes; none for a document given to the
    /// engine.
    pub fn url(&self) -> Option<&Url> {
        self.history.current().url.as_ref()
    }

    /// The name of the encoding the document shown is in, when the
    /// engine cannot decode it yet: a legacy CJK encoding, until a page
    /// needs one or M42 brings them (docs/design/net.md). Its bytes were
    /// decoded as UTF-8, which keeps its ASCII and shows the rest as
    /// U+FFFD; the shell says so.
    pub fn undecodable_encoding(&self) -> Option<&'static str> {
        self.shown
            .page
            .undecodable()
            .map(|encoding| encoding.name())
    }

    /// Calls `waker`, from a fetch's thread, whenever a fetch finishes:
    /// the shell's way to be woken to call [`Engine::poll_network`].
    pub fn set_waker(&mut self, waker: impl Fn() + Send + Sync + 'static) {
        let waker: loader::Waker = std::sync::Arc::new(waker);
        self.network.set_waker(waker.clone());
        self.decoder.set_waker(waker.clone());
        self.waker = Some(waker);
    }

    /// Wakes the caller to poll again: a parse has more to do, and the
    /// window draws between its slices.
    fn wake(&self) {
        if let Some(waker) = &self.waker {
            waker();
        }
    }

    /// Fetches from now on with `client`, in place of the one the
    /// engine makes at its first fetch, `Client::default()`: a test's
    /// client trusts its server's root, say. Fetches in flight finish
    /// with the client they began with.
    pub fn set_network(&mut self, client: Client) {
        self.network.set_client(client);
    }

    /// The current document.
    pub fn document(&self) -> &Document {
        &self.shown.document
    }

    /// Lends the document to `f` to change through its mutation API,
    /// and returns what `f` returns (M13.5). The engine then reads the
    /// document's journal. When a change could touch a stylesheet — a
    /// `<link>` or `<style>` inserted, removed or changed, or a
    /// `<style>`'s text — the document's sheets are found again: an
    /// element's sheet is kept while what made it is unchanged, made
    /// again or fetched again otherwise, and dropped with the element.
    /// If the sheets that apply changed, the next frame cascades the
    /// whole document and lays it out afresh; otherwise the next frame
    /// restyles the elements the changes reach (M13.1) and lays out the
    /// boxes the changes and those restyles reach (M13.2), or keeps the
    /// layout when they reach nothing layout reads, a change of a
    /// transform or an opacity alone written into it (M22.4). The
    /// scroll offsets of freed elements are dropped, and
    /// [`Engine::needs_frame`] says a frame is wanted; the document's
    /// own scroll offset stays, clamped to the new layout by the next
    /// frame. When it records none — a value set to the one already
    /// there, say — nothing is redone.
    pub fn mutate<R>(&mut self, f: impl FnOnce(&mut Document) -> R) -> R {
        let result = f(&mut self.shown.document);
        let changes = self.shown.document.take_changes();
        if !changes.is_empty() {
            self.document_changed(changes);
        }
        result
    }

    /// What `changes` to the document invalidate: the cascade, whole
    /// when the stylesheets changed and by restyle otherwise, the
    /// layout, and the scroll offsets of freed elements.
    fn document_changed(&mut self, changes: Vec<Change>) {
        // An element taken out of the document, put back or not, loses
        // its animations (M22.2): one put back starts them again, as in
        // the reference browser.
        let animations = self.shown.animations.get_mut();
        if !animations.is_empty() {
            let document = &self.shown.document;
            let taken_out: HashSet<NodeId> = changes
                .iter()
                .filter_map(|change| match change {
                    Change::Removed { child, .. } => Some(*child),
                    _ => None,
                })
                .flat_map(|child| document.subtree(child))
                .collect();
            animations.retain(|node| document.contains(node) && !taken_out.contains(&node));
        }
        let mut sheets = None;
        if changes
            .iter()
            .any(|change| loader::may_touch_sheets(&self.shown.document, change))
        {
            self.shown
                .page
                .rescan(&self.shown.document, &mut self.network, None);
            self.update_load_state();
            sheets = Some(self.shown.page.sheets())
                .filter(|sheets| sheets != self.shown.stylesheets.sheets());
        }
        if changes.iter().any(may_touch_images) {
            self.request_images();
        }
        // A change layout reads whatever the styles drops the layout now,
        // between frames, as every change did before M22: only the others
        // wait for the restyle to say what they reach (M22.4).
        if sheets.is_some() || changes.iter().any(read_by_layout) {
            self.shown.layout = None;
        }
        if let Some(sheets) = sheets {
            self.shown.stylesheets = Stylesheets::new(sheets);
            self.shown.recascade = true;
            self.shown.pending.clear();
        } else if self.shown.styled.is_some()
            && !self.shown.recascade
            && self.shown.pending.first() != Some(&Change::Everything)
        {
            // Past the journal's own bound, the changes count as all.
            if self.shown.pending.len() + changes.len() > dom::MAX_JOURNAL {
                self.shown.pending = vec![Change::Everything];
            } else {
                self.shown.pending.extend(changes);
            }
        }
        let document = &self.shown.document;
        self.shown
            .element_scroll
            .retain(|node, _| document.contains(*node));
        self.changed = true;
    }

    /// Whether a frame is wanted: the document was replaced or changed,
    /// or a scroll offset set, since the last frame, or a scroll offset,
    /// the document's or a scroll container's, is on its way to its
    /// target, or an animation plays in its active phase or waits for a
    /// frame to start it (M22.3). A new viewport size is the caller's to
    /// know: the frame takes it.
    pub fn needs_frame(&self) -> bool {
        self.changed
            || self.animating()
            || self.shown.animations.borrow().needs_frame()
            || (self.scripting && !self.hidden && self.shown.scripts.wants_frame())
    }

    /// When a frame is next wanted, on the clock [`Engine::frame_at`] is
    /// given, when none is now (M22.3): the end of the soonest delay of a
    /// playing animation, which shows nothing new before then, or the
    /// next frame of an animated image in view (M23.1), so that a caller
    /// waits for it rather than drawing frames. `None` when a
    /// frame is wanted now, when none will be without a change, or when
    /// no frame was drawn for a time.
    pub fn next_frame_at(&self) -> Option<Duration> {
        if self.needs_frame() {
            return None;
        }
        let origin = self.shown.timeline_origin?;
        // An animated image's next frame too (M23.1).
        let animation = self.shown.animations.borrow().next_change();
        let at = match (animation, self.shown.next_image_frame) {
            (Some(a), Some(b)) => a.min(b),
            (a, b) => a.or(b)?,
        };
        Some(origin + Duration::try_from_secs_f64(at).unwrap_or(Duration::MAX))
    }

    /// Finishes every transition running, as a script's `finish()` of
    /// each would (Web Animations 1 §4.4.13): the next frame ends it, its
    /// element showing the value it was going to, and a frame is wanted.
    /// The differential check does so before it compares the engine with
    /// a fresh one, which runs none (M22).
    pub fn finish_transitions(&mut self) {
        if self.shown.animations.get_mut().finish_transitions() {
            self.changed = true;
        }
    }

    /// Whether a scroll offset is on its way to its target.
    fn animating(&self) -> bool {
        self.shown.scroll_y != self.shown.scroll_target
            || self.shown.element_scroll.values().any(|s| s.y != s.target)
    }

    /// The animation and transition events of the last frame (M22.2):
    /// those its animation frame queued, at each change of phase of an
    /// animation or a transition (Animations 2 §5.1, Transitions 2 §5.1),
    /// and those of the animations and transitions cancelled since the
    /// frame before, in the order they were queued. Nothing dispatches
    /// them yet (M26.1).
    pub fn animation_events(&self) -> &[AnimationEvent] {
        &self.events
    }

    /// The CSS animations and transitions the document shown runs, with
    /// their targets, phases and times: for the tests.
    pub fn animations(&self) -> Vec<RunningAnimation> {
        self.shown.animations.borrow().running()
    }

    /// The document's stylesheets that apply, in cascade order: those
    /// of its `<link>` and `<style>` elements in tree order, the rules of
    /// the sheets each imports in place of its `@import` rules, and each
    /// inside the `@media`, `@supports` and `@layer` blocks its element's
    /// `media` and its imports' conditions make (M20).
    pub fn stylesheets(&self) -> &[css::Stylesheet] {
        self.shown.stylesheets.sheets()
    }

    /// The document's elements' computed styles, by node, as the frames
    /// use them: the cascade of its stylesheets over the built-in UA
    /// stylesheet (see `style::cascade_document`) against the last
    /// frame's viewport and the engine's fonts, computed if the cached
    /// one is stale.
    pub fn styles(&mut self) -> &Styles {
        self.cached_styles()
    }

    /// The medium the document is shown on (M20): a screen of the size
    /// of `viewport`, a CSS pixel to a pixel, with the user's
    /// preferences.
    fn media(&self, viewport: (u32, u32)) -> Media {
        Media {
            media_type: self.media_type,
            preferences: self.preferences,
            scripting: self.scripting,
            ..Media::screen(viewport.0 as f32, viewport.1 as f32)
        }
    }

    /// The document cascaded whole for `viewport`, its animations and
    /// transitions kept in `animations`: as a change of style from
    /// `before`, the styles it replaces, when there are some (M22.1).
    fn cascade(
        &self,
        viewport: (u32, u32),
        animations: &RefCell<Animations>,
        before: Option<&Styles>,
    ) -> Styles {
        let fonts = FontMeasurer(&self.layout_fonts);
        let context = cascade_context(
            viewport,
            &fonts,
            self.shown.page.language(),
            self.media(viewport),
            animations,
        );
        let (sheets, document) = (&self.shown.stylesheets, &self.shown.document);
        match before {
            Some(before) => sheets.cascade_from(document, &context, before),
            None => sheets.cascade(document, &context),
        }
    }

    /// How many full cascades this session has run.
    pub fn style_runs(&self) -> u64 {
        self.style_runs
    }

    /// How many element styles this session's restyles have computed
    /// (M13.1).
    pub fn restyled_elements(&self) -> u64 {
        self.restyled_elements
    }

    /// Lays the current document out against a `viewport_width`-wide
    /// initial containing block (M6.6) and returns its fragments in
    /// tree order, flat — a fresh computation for inspection; the
    /// frame pipeline uses [`Engine::layout`].
    pub fn fragments(&self, viewport_width: f32) -> Vec<Fragment> {
        let viewport = (viewport_width as u32, self.viewport.1);
        // The animations as they stand, changed by nothing done here.
        let animations = RefCell::new(self.shown.animations.borrow().clone());
        let styles = self.cascade(viewport, &animations, None);
        let fonts = FontMeasurer(&self.layout_fonts);
        let context = cascade_context(
            viewport,
            &fonts,
            self.shown.page.language(),
            self.media(viewport),
            &animations,
        );
        let recascade = self
            .shown
            .stylesheets
            .recascader(&self.shown.document, &styles, &context);
        let boxes = layout::build_box_tree_showing(
            &self.shown.document,
            &styles,
            &recascade,
            &self.shown.images,
        );
        layout::flatten(
            layout::layout_document_in(
                &boxes,
                viewport_width,
                Some(self.viewport.1 as f32),
                &self.layout_fonts,
            )
            .roots,
        )
    }

    /// The cached cascade for the current viewport: computed whole when
    /// none is cached, when the viewport changed and the document's
    /// styles depend on it (M10.1), or when an `@media` list answers
    /// otherwise for the medium now, a resize across a breakpoint
    /// (M20); restyled when the document changed since (M13.1), or an
    /// element's animations give something new (M22.3). Either drops the
    /// cached layout, and brings the
    /// layout cache up to date: emptied by a whole cascade, its entries
    /// the changes and the restyled elements reach forgotten by a
    /// restyle (M13.2).
    fn cached_styles(&mut self) -> &Styles {
        let media = self.media(self.viewport);
        let stale = self.shown.recascade
            || self.shown.styled.as_ref().is_none_or(|cached| {
                (cached.styles.uses_viewport_units && cached.viewport != self.viewport)
                    || self.shown.stylesheets.media_changed(&cached.styles, &media)
            });
        if stale {
            // A change of style from the styles cached, when there are
            // some: a change of the sheets, the viewport or the medium
            // starts transitions as a change of the document does.
            let before = self.shown.styled.take();
            let styles = self.cascade(
                self.viewport,
                &self.shown.animations,
                before.as_ref().map(|cached| &cached.styles),
            );
            self.shown.recascade = false;
            self.style_runs += 1;
            self.shown.pending.clear();
            self.shown.animated.clear();
            self.shown.layout = None;
            self.shown.layout_cache.clear();
            self.shown.styled = Some(CachedStyle {
                viewport: self.viewport,
                styles,
            });
            self.request_css_images();
        } else if !self.shown.pending.is_empty() || !self.shown.animated.is_empty() {
            let changes = std::mem::take(&mut self.shown.pending);
            let animated = std::mem::take(&mut self.shown.animated);
            let fonts = FontMeasurer(&self.layout_fonts);
            let context = cascade_context(
                self.viewport,
                &fonts,
                self.shown.page.language(),
                self.media(self.viewport),
                &self.shown.animations,
            );
            let cached = self.shown.styled.as_mut().expect("not stale");
            let restyle = self.shown.stylesheets.restyle_animated(
                &self.shown.document,
                &context,
                &mut cached.styles,
                &changes,
                &animated,
            );
            // Styles that did not depend on the viewport stand for this
            // one; any computed now used it.
            cached.viewport = self.viewport;
            if restyle.full {
                self.style_runs += 1;
                self.shown.layout_cache.clear();
                self.shown.layout = None;
            } else {
                self.restyled_elements += restyle.computed as u64;
                // The layout stands when nothing changed that layout
                // reads — the document's structure or text, a table's
                // spans, a style but in its composited properties — and
                // the composited changes are written into it (M22.4).
                let read = changes.iter().any(read_by_layout);
                let kept =
                    !read && restyle.changed.is_empty() && self.composite(&restyle.composited);
                if kept {
                    self.layouts_kept += 1;
                } else {
                    let mut restyled = restyle.changed;
                    restyled.extend(restyle.composited.iter().map(|c| c.node));
                    self.shown
                        .layout_cache
                        .invalidate(&self.shown.document, &changes, &restyled);
                    self.shown.layout = None;
                }
            }
            self.request_css_images();
        }
        self.shown.styled_at = self.shown.document.version();
        &self.shown.styled.as_ref().expect("just computed").styles
    }

    /// Writes the composited changes `composited` into the layout shown
    /// and the layout cache, all in one walk of each (M22.4,
    /// `layout::composite`): says whether there is a layout and it
    /// stands, every change having found the boxes it reaches and left
    /// the scrollable overflow as it was. When it does not, the caller
    /// lays the document out again.
    fn composite(&mut self, composited: &[style::Composited]) -> bool {
        let Some(cached) = self.shown.layout.as_mut() else {
            return false;
        };
        let changes: Vec<layout::EffectsChange<'_>> = composited
            .iter()
            .map(|change| layout::EffectsChange {
                node: change.node,
                pseudo: change.pseudo,
                old: &change.old,
                new: &change.new,
                transformed: change.transformed,
            })
            .collect();
        let done = cached.layout.composite(&changes, cached.width);
        self.shown.layout_cache.composite(&changes);
        done == layout::Composited::Done
    }

    /// The document's layout for `viewport_width`, computed once per
    /// document and width (M9.5) and reused by every frame, scroll and
    /// hit test at that width. Computing it takes from the layout cache
    /// what the changes since did not reach (M13.2).
    pub fn layout(&mut self, viewport_width: f32) -> &DocumentLayout {
        // A viewport change can invalidate the cascade, and with it the
        // layout — so the cascade is checked first.
        self.cached_styles();
        let viewport_height = self.viewport.1 as f32;
        let stale = self.shown.layout.as_ref().is_none_or(|cached| {
            cached.width != viewport_width
                || (cached.layout.depends_on_viewport_height && cached.height != viewport_height)
        });
        if stale {
            let styles = &self.shown.styled.as_ref().expect("just computed").styles;
            let fonts = FontMeasurer(&self.layout_fonts);
            let context = cascade_context(
                self.viewport,
                &fonts,
                self.shown.page.language(),
                self.media(self.viewport),
                &self.shown.animations,
            );
            let recascade =
                self.shown
                    .stylesheets
                    .recascader(&self.shown.document, styles, &context);
            let boxes = layout::build_box_tree_showing(
                &self.shown.document,
                styles,
                &recascade,
                &self.shown.images,
            );
            let layout = layout::layout_document_cached(
                &boxes,
                viewport_width,
                Some(viewport_height),
                &self.layout_fonts,
                &self.shown.layout_cache,
            );
            self.layout_runs += 1;
            let (laid_out, reused) = self.shown.layout_cache.counts();
            self.boxes_laid_out += laid_out as u64;
            self.boxes_reused += reused as u64;
            self.shown.layout = Some(CachedLayout {
                width: viewport_width,
                height: viewport_height,
                layout,
            });
        }
        &self.shown.layout.as_ref().expect("just computed").layout
    }

    /// How many layouts this session has run.
    pub fn layout_runs(&self) -> u64 {
        self.layout_runs
    }

    /// How many restyles this session has kept the layout through
    /// (M22.4): their changes reached nothing layout reads, or only the
    /// transforms and opacities of boxes, which were written into it.
    pub fn layouts_kept(&self) -> u64 {
        self.layouts_kept
    }

    /// How many boxes placed in flow this session's layouts have laid
    /// out (M13.2): those a change reached, or that the layout cache
    /// could not hold.
    pub fn boxes_laid_out(&self) -> u64 {
        self.boxes_laid_out
    }

    /// How many boxes placed in flow this session's layouts have taken
    /// from the layout cache, each with its subtree, instead of laying
    /// them out (M13.2).
    pub fn boxes_reused(&self) -> u64 {
        self.boxes_reused
    }

    /// What the last frame's raster drew anew (M13.3): the whole frame,
    /// or the rectangles where its display list differed from the
    /// frame's before — none when it did not.
    pub fn redrawn(&self) -> &render::Redrawn {
        self.backend.redrawn()
    }

    /// What the last frame did with the layers (M19): which groups it
    /// composited from a layer, which layers it drew into and how many
    /// tiles it drew ahead of a scroll.
    pub fn layered(&self) -> render::Layered {
        self.backend.layered()
    }

    /// The layers the backend holds (M19), in a fixed order: the
    /// compositing note's policy, as the backend decided it for the
    /// last frame, with what each costs.
    pub fn layers(&self) -> Vec<render::LayerInfo> {
        self.backend.layers()
    }

    /// Which rasterizer the session draws on (M19.3).
    pub fn backend(&self) -> Backend {
        self.backend.backend()
    }

    /// How many bytes of layer pixmaps the backend holds (M19.6).
    pub fn layer_bytes(&self) -> usize {
        self.backend.layer_bytes()
    }

    /// The user's preferences that media queries ask about (M20.3).
    pub fn preferences(&self) -> Preferences {
        self.preferences
    }

    /// Sets the user's preferences, as the shell reads them from the
    /// operating system or its own options (M20.3). A frame is wanted
    /// when an `@media` list of the document shown answers otherwise
    /// for them, and cascades the document again; a page that asks
    /// nothing of them is left alone. The engine reads one itself:
    /// under reduced motion its scrolls jump to their targets, one on
    /// its way there included (M22.5). Animations and transitions run
    /// as the page wrote them, which the page's own
    /// `prefers-reduced-motion` query is the means to change.
    pub fn set_preferences(&mut self, preferences: Preferences) {
        self.preferences = preferences;
        let media = self.media(self.viewport);
        if self
            .shown
            .styled
            .as_ref()
            .is_some_and(|cached| self.shown.stylesheets.media_changed(&cached.styles, &media))
        {
            self.changed = true;
        }
        self.settle_scrolls();
    }

    /// The medium the document shown is styled for.
    pub fn media_type(&self) -> MediaType {
        self.media_type
    }

    /// Styles the document shown for another medium on the screen, as a
    /// browser's developer tools emulate one: `print` draws the window
    /// with what the page's print styles say, at the window's size and
    /// unpaginated, as the reference browser draws a page told to with
    /// its `mediaTypeOverride` (M20, the reference pages). A frame is
    /// wanted when an `@media` list of the document answers otherwise;
    /// [`Engine::print`] is unaffected.
    pub fn set_media_type(&mut self, media_type: MediaType) {
        self.media_type = media_type;
        let media = self.media(self.viewport);
        if self
            .shown
            .styled
            .as_ref()
            .is_some_and(|cached| self.shown.stylesheets.media_changed(&cached.styles, &media))
        {
            self.changed = true;
        }
    }

    /// Sets how many bytes of layer pixmaps the backend may hold
    /// (M19.6), [`render::DEFAULT_LAYER_BUDGET`] unless set: the layers
    /// least recently composited go when a frame needs more, and a
    /// frame that cannot hold its layers draws them through.
    pub fn set_layer_budget(&mut self, bytes: usize) {
        self.backend.set_layer_budget(bytes);
    }

    /// The scroll offsets of the frame: the viewport's and each scroll
    /// container's, rounded to whole pixels (M19), since a composited
    /// scroll is snapped to the pixel grid as the reference browser
    /// snaps it; the animation keeps its fraction. Hit testing and the
    /// display list read the same offsets.
    fn drawn_offsets(&self) -> (f32, HashMap<NodeId, f32>) {
        let offsets = self
            .shown
            .element_scroll
            .iter()
            .map(|(node, scroll)| (*node, scroll.y.round()))
            .collect();
        (self.shown.scroll_y.round(), offsets)
    }

    /// Draws the frame for `time` (M13.4): the scroll animations are
    /// advanced to `time.now` — by the time since the last frame drawn
    /// while one ran, or by one frame of [`FRAME_BUDGET`] for one begun
    /// since — the document's timeline moves to `time.now` less the time
    /// of the first frame drawn for a time while it was shown, its
    /// origin (M22.3), and the frame is drawn as [`Engine::frame`] draws
    /// it, overrunning when it ends past `time.deadline`. The pixels are
    /// lent out until the next frame; [`Engine::frame_stats`] says what
    /// it took. What it shows is a function of the document, the
    /// viewport, the scroll inputs and the times passed in.
    pub fn frame_at(&mut self, width: u32, height: u32, time: FrameTime) -> &[u8] {
        if self.pointer.is_some() && (self.changed || self.animating()) {
            self.update_hover();
        }
        self.update_the_rendering(width, height, time.now);
        if self.animating() {
            let elapsed = self
                .animation_clock
                .map_or(FRAME_BUDGET, |then| time.now.saturating_sub(then));
            self.advance(elapsed);
        }
        self.animation_clock = self.animating().then_some(time.now);
        let origin = *self.shown.timeline_origin.get_or_insert(time.now);
        let timeline = time.now.saturating_sub(origin).as_secs_f64();
        self.shown.animations.get_mut().set_time(timeline);
        self.draw(width, height, time.deadline.saturating_sub(time.now))
    }

    /// Renders the current document into an RGBA8 framebuffer for a
    /// `width` x `height` viewport at the current scroll offset, with
    /// the scrollbar when the document is taller than the viewport. A
    /// viewport over `render::MAX_FRAME_PIXELS` gets an empty buffer
    /// rather than an allocation the machine cannot make. No animation
    /// is advanced, and the timeline keeps its time: [`Engine::tick`]
    /// moves them for a caller without a clock, and [`Engine::frame_at`]
    /// draws by time; the animations waiting for a frame start at the
    /// timeline's time. The frame overruns past [`FRAME_BUDGET`].
    ///
    /// A pure function of (document, stylesheets, size, scroll): no
    /// platform state, no side effects outside the caches.
    pub fn frame(&mut self, width: u32, height: u32) -> Vec<u8> {
        self.draw(width, height, FRAME_BUDGET).to_vec()
    }

    /// The viewport's size before a frame is drawn at it (M25): the
    /// window's size a page's scripts read, and its media queries match,
    /// while it loads. Each frame sets it to the size it is drawn at.
    pub fn set_viewport(&mut self, width: u32, height: u32) {
        self.viewport = (width, height);
    }

    /// What the last frame took, stage by stage (M13.4).
    pub fn frame_stats(&self) -> FrameStats {
        self.stats
    }

    /// Draws a frame and times its stages, overrunning past `budget`.
    fn draw(&mut self, width: u32, height: u32, budget: Duration) -> &[u8] {
        let start = Instant::now();
        if width as u64 * height as u64 > render::MAX_FRAME_PIXELS {
            self.stats = FrameStats::default();
            return &[];
        }
        self.changed = false;
        self.viewport = (width, height);
        // The frame's animations (M22.3): those made since the last frame
        // start at its time, and the elements whose animations give
        // something new at it are restyled with the document's changes;
        // those the style stage makes start at its time too.
        let animations = self.shown.animations.get_mut();
        if !animations.is_empty() {
            animations.start_pending();
            let moved = animations.sample();
            self.shown.animated.extend(moved);
        }
        self.cached_styles();
        let animations = self.shown.animations.get_mut();
        animations.start_pending();
        self.events = animations.take_events();
        if self.scripting {
            self.pending_events.extend(self.events.iter().cloned());
        }
        let style_time = start.elapsed();
        let stage = Instant::now();
        self.layout(width as f32);
        let layout_time = stage.elapsed();
        // The document may be shorter at this width than where the
        // offset was; so may a scroll container's content.
        let max = self.max_scroll();
        self.shown.scroll_target = self.shown.scroll_target.clamp(0.0, max);
        self.shown.scroll_y = self.shown.scroll_y.clamp(0.0, max);
        let mut elements = std::mem::take(&mut self.shown.element_scroll);
        {
            let layout = self.layout(width as f32);
            for (node, scroll) in elements.iter_mut() {
                let max = fragment_of(&layout.roots, *node).map_or(0.0, Fragment::max_scroll);
                scroll.target = scroll.target.clamp(0.0, max);
                scroll.y = scroll.y.clamp(0.0, max);
            }
        }
        self.shown.element_scroll = elements;
        if let Some(goal) = self.goal.clone() {
            // Applied before painting, so that no frame shows the
            // document elsewhere first, and again at each frame until
            // the document's load is over, since a late stylesheet can
            // move what it scrolls to.
            self.scroll_to_goal(&goal);
            if self.shown.page.settled() {
                self.goal = None;
            }
        }
        let (scroll_y, offsets) = self.drawn_offsets();
        let stage = Instant::now();
        let viewport = Viewport {
            width: width as f32,
            height: height as f32,
            scroll_y,
        };
        let scrollbar = self.scrollbar();
        self.layout(width as f32);
        let layout = &self.shown.layout.as_ref().expect("just laid out").layout;
        let mut list = paint_fragments_with(&layout.roots, viewport, &offsets, &self.shown.images);
        self.show_pixels(&mut list.items, width as f32, height as f32);
        if let Some(bar) = scrollbar {
            list.items.push(DisplayItem::Rect {
                x: bar.x,
                y: 0.0,
                w: bar.w,
                h: bar.h,
                color: SCROLLBAR_TRACK,
            });
            list.items.push(DisplayItem::Rect {
                x: bar.x + 2.0,
                y: bar.thumb_y,
                w: bar.w - 4.0,
                h: bar.thumb_h,
                color: SCROLLBAR_THUMB,
            });
        }
        let paint = stage.elapsed();
        let stage = Instant::now();
        self.backend.rasterize_owned(list, width, height);
        let raster = stage.elapsed();
        let total = start.elapsed();
        self.stats = FrameStats {
            style: style_time,
            layout: layout_time,
            paint,
            raster,
            total,
            overran: total > budget,
        };
        self.backend.last_frame()
    }

    /// Does the work the last frame could do without, until `until`
    /// (M19.4): the tiles of the document's layer, and of any other
    /// scrolled layer, that the rows next to the viewport need, so that
    /// a scroll's next frames composite them rather than draw them. For
    /// the caller to call once the frame is presented, with the time
    /// before the next one (M13.4 reserved it for such work); what it
    /// leaves waits for a later call or for a frame that shows it. How
    /// many tiles it drew.
    pub fn draw_ahead(&mut self, until: Instant) -> u32 {
        self.backend.draw_ahead(until)
    }

    // ---- Scrolling (M9.2) ---------------------------------------------------

    /// The scroll offset the current frame shows, in px.
    pub fn scroll_y(&self) -> f32 {
        self.shown.scroll_y
    }

    /// The scroll offset being approached (equal to [`Engine::scroll_y`]
    /// once the animation has settled).
    pub fn scroll_target(&self) -> f32 {
        self.shown.scroll_target
    }

    /// The furthest the last frame's viewport can scroll: the document's
    /// height past the viewport's, never negative. Zero before the first
    /// frame, since there is no viewport to measure against yet; only
    /// vertical scrolling exists (the document is laid out to the
    /// viewport's width, and a wider word overflows to the right).
    pub fn max_scroll(&mut self) -> f32 {
        let (width, height) = self.viewport;
        if width == 0 {
            return 0.0;
        }
        (self.layout(width as f32).height - height as f32).max(0.0)
    }

    /// Scrolls by `dy` px (down when positive), animated over the next
    /// ticks, or at once under reduced motion (M22.5); the target is
    /// clamped to the document.
    pub fn scroll_by(&mut self, dy: f32) {
        let target = self.shown.scroll_target + dy;
        self.scroll_to(target);
    }

    /// Scrolls to offset `y`, animated over the next ticks, or at once
    /// under reduced motion (M22.5); clamped to the document (so an
    /// infinite `y` means the bottom or the top), and a NaN is ignored.
    pub fn scroll_to(&mut self, y: f32) {
        if y.is_nan() {
            return;
        }
        // The user has scrolled: a position being restored is given up.
        self.goal = None;
        let max = self.max_scroll();
        self.shown.scroll_target = y.clamp(0.0, max);
        self.settle_scrolls();
    }

    /// Under reduced motion, puts every scroll offset, the document's
    /// and each scroll container's, at its target at once (M22.5): the
    /// engine's own motion is kept to what is essential (Media Queries
    /// 5 §12.1), and no frame is wanted for a scroll but the one that
    /// shows where it went.
    fn settle_scrolls(&mut self) {
        if !self.preferences.reduced_motion {
            return;
        }
        let shown = &mut self.shown;
        if shown.scroll_y != shown.scroll_target {
            shown.scroll_y = shown.scroll_target;
            self.changed = true;
        }
        for scroll in shown.element_scroll.values_mut() {
            if scroll.y != scroll.target {
                scroll.y = scroll.target;
                self.changed = true;
            }
        }
    }

    /// Jumps to offset `y` at once, no animation (a thumb drag follows
    /// the pointer); clamped to the document.
    pub fn set_scroll(&mut self, y: f32) {
        self.scroll_to(y);
        self.changed |= self.shown.scroll_y != self.shown.scroll_target;
        self.shown.scroll_y = self.shown.scroll_target;
    }

    /// Jumps to the fraction `f` of the scroll range (0 is the top, 1
    /// the bottom), as a thumb drag asks.
    pub fn scroll_to_fraction(&mut self, f: f32) {
        let max = self.max_scroll();
        self.set_scroll(f.clamp(0.0, 1.0) * max);
    }

    /// Advances the scroll animations and the document's timeline by one
    /// frame of [`FRAME_BUDGET`], for a caller without a clock (see
    /// [`Engine::frame_at`]). Returns whether another frame is wanted:
    /// an offset has not reached its target, an animation plays, or an
    /// animated image in view has frames to come.
    pub fn tick(&mut self) -> bool {
        let animations = self.shown.animations.get_mut();
        let time = animations.time();
        animations.set_time(time + FRAME_BUDGET.as_secs_f64());
        self.advance(FRAME_BUDGET)
            || self.shown.animations.get_mut().needs_frame()
            || self.shown.next_image_frame.is_some()
    }

    /// Advances the scroll animations by `elapsed`: each shown offset,
    /// the viewport's and every scroll container's, covers
    /// [`SCROLL_TICK_FRACTION`] of its distance to its target in every
    /// [`FRAME_BUDGET`] of it, so that how far it gets depends on the
    /// time alone, not on how many frames shared it, and lands on the
    /// target within [`SCROLL_SNAP_PX`]. Returns whether an offset has
    /// not reached its target.
    fn advance(&mut self, elapsed: Duration) -> bool {
        let frames = elapsed.as_secs_f64() / FRAME_BUDGET.as_secs_f64();
        let keep = (1.0 - f64::from(SCROLL_TICK_FRACTION)).powf(frames) as f32;
        let mut moving = approach(&mut self.shown.scroll_y, self.shown.scroll_target, keep);
        for scroll in self.shown.element_scroll.values_mut() {
            moving |= approach(&mut scroll.y, scroll.target, keep);
        }
        moving
    }

    /// The scrollbar for the last frame's viewport, or `None` when the
    /// document fits in it (or no frame has been drawn).
    pub fn scrollbar(&mut self) -> Option<Scrollbar> {
        let (width, height) = self.viewport;
        let max = self.max_scroll();
        if max <= 0.0 {
            return None;
        }
        let (width, height) = (width as f32, height as f32);
        let (thumb_y, thumb_h) = scrollbar_thumb(height, max, self.shown.scroll_y);
        Some(Scrollbar {
            x: width - SCROLLBAR_WIDTH,
            w: SCROLLBAR_WIDTH,
            h: height,
            thumb_y,
            thumb_h,
        })
    }

    // ---- Element scrolling (M10.5) ------------------------------------------

    /// The scroll offset a scroll container shows, in px: zero for an
    /// element never scrolled, or not a scroll container at all.
    pub fn element_scroll(&self, node: NodeId) -> f32 {
        self.shown.element_scroll.get(&node).map_or(0.0, |s| s.y)
    }

    /// The offset a scroll container is approaching.
    pub fn element_scroll_target(&self, node: NodeId) -> f32 {
        self.shown
            .element_scroll
            .get(&node)
            .map_or(0.0, |s| s.target)
    }

    /// The furthest the element's content can scroll in the last
    /// frame's layout: its scrollable height past its padding box
    /// (`layout::Fragment::max_scroll`); zero for an element that is
    /// not a scroll container, or before the first frame.
    pub fn element_max_scroll(&mut self, node: NodeId) -> f32 {
        let (width, _) = self.viewport;
        if width == 0 {
            return 0.0;
        }
        fragment_of(&self.layout(width as f32).roots, node).map_or(0.0, Fragment::max_scroll)
    }

    /// Scrolls the element's content to offset `y`, animated over the
    /// next ticks, or at once under reduced motion, and clamped to its
    /// content, as [`Engine::scroll_to`] does for the viewport; a NaN
    /// is ignored. An `overflow: hidden` box scrolls this way too, by
    /// program only (CSS2 §11.1.1).
    pub fn scroll_element_to(&mut self, node: NodeId, y: f32) {
        if y.is_nan() {
            return;
        }
        self.goal = None;
        let max = self.element_max_scroll(node);
        self.shown.element_scroll.entry(node).or_default().target = y.clamp(0.0, max);
        self.settle_scrolls();
    }

    /// Scrolls the element's content by `dy` px (down when positive),
    /// animated over the next ticks, or at once under reduced motion.
    pub fn scroll_element_by(&mut self, node: NodeId, dy: f32) {
        let target = self.element_scroll_target(node) + dy;
        self.scroll_element_to(node, target);
    }

    /// Jumps the element's content to offset `y` at once, no animation.
    pub fn set_element_scroll(&mut self, node: NodeId, y: f32) {
        self.scroll_element_to(node, y);
        if let Some(scroll) = self.shown.element_scroll.get_mut(&node) {
            self.changed |= scroll.y != scroll.target;
            scroll.y = scroll.target;
        }
    }

    /// A wheel turn of `dy` px (down when positive) at the viewport
    /// point `(x, y)`: scroll chaining — the innermost scroll container
    /// under the pointer whose target can still move that way takes it,
    /// else the next one out, else the viewport. Only `overflow: auto`
    /// and `scroll` containers take the wheel.
    pub fn wheel(&mut self, x: f32, y: f32, dy: f32) {
        if !dy.is_finite() || dy == 0.0 {
            return;
        }
        // A `wheel` event first (M26.5): canceled, nothing scrolls.
        if let Some(target) = self.pointer_at(x, y) {
            let event = self
                .mouse_event("WheelEvent", "wheel", target, 0, 0, None)
                .with("deltaY", js::Field::Number(f64::from(dy)));
            if self.fire(js::Target::Node(scripts::handle(target)), event) == Some(true) {
                return;
            }
        }
        for node in self.scroll_chain_at(x, y).into_iter().rev() {
            let max = self.element_max_scroll(node);
            let target = self.element_scroll_target(node);
            if (dy > 0.0 && target < max) || (dy < 0.0 && target > 0.0) {
                self.scroll_element_by(node, dy);
                return;
            }
        }
        self.scroll_by(dy);
    }

    /// The scroll containers that take the wheel under the viewport
    /// point, outermost first: the ones around the element hit there.
    fn scroll_chain_at(&mut self, x: f32, y: f32) -> Vec<NodeId> {
        let Some((_, chain)) = self.hit(x, y) else {
            return Vec::new();
        };
        let layout = self.layout(self.viewport.0 as f32);
        chain
            .into_iter()
            .filter(|node| {
                fragment_of(&layout.roots, *node)
                    .is_some_and(|f| matches!(f.style.overflow, Overflow::Auto | Overflow::Scroll))
            })
            .collect()
    }

    // ---- Hit testing (M9.4) -------------------------------------------------

    /// The node under the viewport point `(x, y)` at the current scroll
    /// offset: the last fragment in paint order whose box contains the
    /// point (a text fragment's box is its line, and answers with its
    /// containing element; an anonymous box's with the element it
    /// names, or the document node at the top level). Content clipped
    /// away by an ancestor's `overflow` is not hit. `None` over nothing
    /// — the viewport background, or the scrollbar, which the shell
    /// checks first.
    pub fn hit_test(&mut self, x: f32, y: f32) -> Option<NodeId> {
        self.hit(x, y).map(|(node, _)| node)
    }

    /// The element under the viewport point and the clipping boxes
    /// around it, outermost first (the box itself last, when the point
    /// is inside its padding box): what [`Engine::hit_test`] answers
    /// with and what the wheel chains through (M10.5).
    ///
    /// The steps of [`paint_order`] are read in paint order and the last
    /// one under the point wins (M11): a box's or a text's step counts
    /// when its rectangle holds the point and so does the innermost clip
    /// in force, since content clipped away is not hit. The steps are
    /// in viewport coordinates with every scroll offset applied, so the
    /// point is compared as it is — inside a group (M19), the point
    /// mapped by the inverse of the group's matrix, the inverses
    /// composing from the outside in, since the steps inside are in the
    /// group's own coordinates; a group whose matrix cannot be inverted,
    /// or that maps the point behind its eye, is passed over whole. A
    /// scrollbar step is passed over: the bar is an indicator over its
    /// container, not a control.
    fn hit(&mut self, x: f32, y: f32) -> Option<(NodeId, Vec<NodeId>)> {
        let (width, height) = self.viewport;
        if width == 0 || !x.is_finite() || !y.is_finite() {
            return None;
        }
        let (scroll_y, offsets) = self.drawn_offsets();
        let viewport = Viewport {
            width: width as f32,
            height: height as f32,
            scroll_y,
        };
        let layout = self.layout(width as f32);
        // The clips in force, outermost first, each with how many
        // groups deep it was met: the chain around a hit, which a
        // generated box's clip is not part of, as it does not scroll.
        let mut clips: Vec<(Option<NodeId>, Rect, usize)> = Vec::new();
        // The point in the coordinates of the groups being walked into
        // (M19), outermost first: a group's inverse matrix maps the
        // point outside it to the point inside it.
        let mut points: Vec<(f32, f32)> = vec![(x, y)];
        // How deep inside a group nothing of which is hit: its matrix
        // singular, or the point behind its eye.
        let mut passed = 0u32;
        // Whether the point is in `rect`, and inside the innermost clip
        // of every group around, each in that group's coordinates.
        let under =
            |rect: &Rect, clips: &[(Option<NodeId>, Rect, usize)], points: &[(f32, f32)]| {
                let (px, py) = points[points.len() - 1];
                if !rect.contains(px, py) {
                    return false;
                }
                let mut checked = usize::MAX;
                for (_, clip, depth) in clips.iter().rev() {
                    if *depth == checked {
                        continue;
                    }
                    checked = *depth;
                    let (cx, cy) = points[(*depth).min(points.len() - 1)];
                    if !clip.contains(cx, cy) {
                        return false;
                    }
                }
                true
            };
        let chain = |clips: &[(Option<NodeId>, Rect, usize)]| {
            clips.iter().filter_map(|(node, _, _)| *node).collect()
        };
        let mut hit = None;
        for step in paint_order(&layout.roots, viewport, &offsets) {
            if passed > 0 {
                match step {
                    Step::PushGroup { .. } => passed += 1,
                    Step::PopGroup => passed -= 1,
                    _ => {}
                }
                continue;
            }
            match step {
                // An inert box, and what is inside it, is passed over
                // (M22.1).
                Step::Box { fragment, .. } | Step::Text { fragment, .. }
                    if fragment.style.inert => {}
                Step::Box { fragment, rect, .. } => {
                    if under(&rect, &clips, &points) {
                        let (px, py) = points[points.len() - 1];
                        let mut chain: Vec<NodeId> = chain(&clips);
                        if fragment.clips()
                            && fragment.generated.is_none()
                            && padding_box(fragment, rect).contains(px, py)
                        {
                            chain.push(fragment.node);
                        }
                        hit = Some((fragment.node, chain));
                    }
                }
                Step::Text { fragment, rect, .. } => {
                    if under(&rect, &clips, &points) {
                        hit = Some((fragment.node, chain(&clips)));
                    }
                }
                Step::PushClip { node, rect } => clips.push((node, rect, points.len() - 1)),
                Step::PopClip => {
                    clips.pop();
                }
                Step::PushGroup { matrix, .. } => {
                    let (px, py) = points[points.len() - 1];
                    match matrix.inverse().and_then(|inverse| inverse.project(px, py)) {
                        Some(inside) => points.push(inside),
                        None => passed = 1,
                    }
                }
                Step::PopGroup => {
                    if points.len() > 1 {
                        points.pop();
                    }
                }
                Step::Scrollbar { .. } => {}
            }
        }
        hit
    }

    /// The link under the viewport point, if any (M9.4, M15): the
    /// nearest `<a>` with an `href` at or above the element there, and
    /// the URL its `href` parses to against the document's base URL,
    /// with the document's encoding, as HTML's "follow the hyperlink"
    /// parses it. An `href` that does not parse is no link to follow.
    pub fn link_at(&mut self, x: f32, y: f32) -> Option<Link> {
        let hit = self.hit_test(x, y)?;
        let document = &self.shown.document;
        let (node, href) = std::iter::successors(Some(hit), |&node| document.parent_element(node))
            .find_map(|node| {
                (document.tag(node) == Some("a"))
                    .then(|| document.attr(node, "href").map(|href| (node, href)))
                    .flatten()
            })?;
        let url = self.shown.page.resolve(document, href)?;
        Some(Link {
            page: self.shown.page.id(),
            node,
            url,
        })
    }

    /// Scrolls to `goal`, at once: an entry's saved offset, or the part
    /// of the document a fragment indicates.
    fn scroll_to_goal(&mut self, goal: &ScrollGoal) {
        match goal {
            ScrollGoal::Offset(y) => {
                let y = y.clamp(0.0, self.max_scroll());
                self.shown.scroll_y = y;
                self.shown.scroll_target = y;
            }
            ScrollGoal::Fragment(fragment) => {
                match indicated_part(&self.shown.document, fragment) {
                    Some(Indicated::Top) => {
                        self.shown.scroll_y = 0.0;
                        self.shown.scroll_target = 0.0;
                    }
                    Some(Indicated::Element(node)) => self.scroll_into_view(node),
                    None => {}
                }
            }
        }
    }

    /// Scrolls `node` into view as CSSOM View's "scroll into view" does
    /// with `block: start` and the behaviour "auto", that is at once:
    /// each scrolling box around it, innermost first, is scrolled so
    /// that the top of the element's first box is at the top of the
    /// box's padding edge, as far as it can go, and then the viewport.
    /// An element with no box is not scrolled to.
    fn scroll_into_view(&mut self, node: NodeId) {
        let Some((_, chain)) = self.locate(node) else {
            return;
        };
        for (container, _) in chain.into_iter().rev() {
            // Where things are now that the boxes inside have scrolled.
            let Some((top, chain)) = self.locate(node) else {
                return;
            };
            let Some(&(_, padding_top)) = chain.iter().find(|(c, _)| *c == container) else {
                return;
            };
            let max = self.element_max_scroll(container);
            let scroll = self.shown.element_scroll.entry(container).or_default();
            let wanted = (scroll.y + top - padding_top).clamp(0.0, max);
            scroll.y = wanted;
            scroll.target = wanted;
        }
        let Some((top, _)) = self.locate(node) else {
            return;
        };
        let wanted = top.clamp(0.0, self.max_scroll());
        self.shown.scroll_y = wanted;
        self.shown.scroll_target = wanted;
    }

    /// Where `node`'s first box is in the document, with the scroll
    /// containers scrolled as they are, and the clipping boxes around
    /// it, outermost first, each with the top of its padding box: from
    /// the paint order of the whole document, as a viewport as tall as
    /// it and at its top would see it.
    fn locate(&mut self, node: NodeId) -> Option<(f32, Vec<(NodeId, f32)>)> {
        let (width, height) = self.viewport;
        if width == 0 {
            return None;
        }
        let (_, offsets) = self.drawn_offsets();
        let layout = self.layout(width as f32);
        let whole = Viewport {
            width: width as f32,
            height: layout.height.max(0.0) + height as f32,
            scroll_y: 0.0,
        };
        let mut padding_tops: HashMap<NodeId, f32> = HashMap::new();
        // As in `hit`, with the clips of generated boxes, which do not
        // scroll, left out; and the first box is one of the element's
        // own, not one its `::before` generated.
        let mut chain: Vec<Option<NodeId>> = Vec::new();
        for step in paint_order(&layout.roots, whole, &offsets) {
            match step {
                Step::Box { fragment, rect, .. } | Step::Text { fragment, rect, .. } => {
                    if fragment.generated.is_some() {
                        continue;
                    }
                    if fragment.clips() {
                        padding_tops.insert(fragment.node, padding_box(fragment, rect).y);
                    }
                    if fragment.node == node {
                        let chain = chain
                            .iter()
                            .flatten()
                            .map(|c| (*c, padding_tops.get(c).copied().unwrap_or(0.0)))
                            .collect();
                        return Some((rect.y, chain));
                    }
                }
                Step::PushClip { node, .. } => chain.push(node),
                Step::PopClip => {
                    chain.pop();
                }
                Step::PushGroup { .. } | Step::PopGroup => {}
                Step::Scrollbar { .. } => {}
            }
        }
        None
    }
}

/// Whether `node` is in the document's tree: its ancestors reach the
/// document node. A node removed stays valid, detached.
fn connected(document: &Document, node: NodeId) -> bool {
    document.contains(node)
        && std::iter::successors(Some(node), |&node| document.parent(node)).last()
            == Some(document.root())
}

/// HTML's "select the indicated part" for a document at a URL whose
/// fragment is `fragment`: the top of the document for an empty one;
/// else the first element in tree order whose ID is the fragment, or
/// the first `a` whose `name` is; else the same two for the fragment
/// percent-decoded and decoded as UTF-8; else the top of the document
/// for `top` in any case; else nothing.
fn indicated_part(document: &Document, fragment: &str) -> Option<Indicated> {
    if fragment.is_empty() {
        return Some(Indicated::Top);
    }
    if let Some(element) = potential_indicated_element(document, fragment) {
        return Some(Indicated::Element(element));
    }
    let decoded =
        String::from_utf8_lossy(&net::url::percent_decode(fragment.as_bytes())).into_owned();
    if let Some(element) = potential_indicated_element(document, &decoded) {
        return Some(Indicated::Element(element));
    }
    decoded
        .eq_ignore_ascii_case("top")
        .then_some(Indicated::Top)
}

/// HTML's "find a potential indicated element": the first element in
/// tree order whose ID is `fragment`, else the first `a` whose `name`
/// is.
fn potential_indicated_element(document: &Document, fragment: &str) -> Option<NodeId> {
    let root = document.root();
    document
        .subtree(root)
        .find(|&node| document.tag(node).is_some() && document.attr(node, "id") == Some(fragment))
        .or_else(|| {
            document.subtree(root).find(|&node| {
                document.tag(node) == Some("a") && document.attr(node, "name") == Some(fragment)
            })
        })
}

/// Why a page whose URL's scheme is `source` may not navigate to `url`
/// (docs/design/navigation.md, "Links"; M25 for scripts): `http:`,
/// `https:` and `about:blank` from any page, `file:` from a `file:` page,
/// other `about:` pages from a page the browser `made`, `data:` from
/// none, and no scheme another program would open; `None` when it may.
/// `javascript:` is its caller's.
fn refusal(url: &Url, source: Option<&str>, made: bool) -> Option<String> {
    match url.scheme() {
        "http" | "https" => None,
        "about" if url.path_str() == "blank" => None,
        "about" if made => None,
        "about" => Some("This page may not open the browser's own pages".to_owned()),
        "file" if source == Some("file") => None,
        "file" => Some("This page may not open files on this computer".to_owned()),
        "data" => Some(
            "This page may not open a data: address; type it into the address field to open it"
                .to_owned(),
        ),
        scheme => Some(format!(
            "{scheme}: links open in another program, which this version cannot do"
        )),
    }
}

/// The document HTML `source` parses to, or an empty one if the parser
/// refuses it.
fn parsed_or_empty(source: &str) -> Document {
    html::parse_document(source)
        .map_or_else(|_| Document::new(), |parsed| Document::from_node(&parsed))
}

/// The box fragment of the element `node`, if it has one of its own: a
/// box its `::before` or `::after` generated is not (M16).
fn fragment_of(fragments: &[Fragment], node: NodeId) -> Option<&Fragment> {
    for f in fragments {
        if matches!(f.kind, FragmentKind::Box) && f.node == node && f.generated.is_none() {
            return Some(f);
        }
        if let Some(found) = fragment_of(&f.children, node) {
            return Some(found);
        }
    }
    None
}

/// One step of a scroll animation: `y` moves to `keep` of its distance
/// from `target`, landing on it within [`SCROLL_SNAP_PX`]. Returns
/// whether it is still short of it.
fn approach(y: &mut f32, target: f32, keep: f32) -> bool {
    *y = target - (target - *y) * keep;
    if (target - *y).abs() <= SCROLL_SNAP_PX {
        *y = target;
        return false;
    }
    true
}

impl Drop for Engine {
    /// Aborts the fetches in flight, so that their threads end soon
    /// rather than read on for an engine gone.
    fn drop(&mut self) {
        self.network.cancel_all();
    }
}

/// How far an arrow key scrolls the page (M26.6): 40px, as the shell's
/// keys did.
const SCROLL_STEP: f32 = 40.0;

/// Whether a key clicks the element focused (M26.6), as the reference
/// browser's keys do: Enter a link, a button or a button-like `input`;
/// Space, on its release, a button, a button-like `input`, a checkbox or
/// a radio button.
fn key_clicks(document: &Document, element: NodeId, space: bool) -> bool {
    let kind = document
        .attr(element, "type")
        .unwrap_or("")
        .to_ascii_lowercase();
    match document.tag(element) {
        Some("a" | "area") => !space && document.attr(element, "href").is_some(),
        Some("button") => true,
        Some("input") => match kind.as_str() {
            "button" | "submit" | "reset" | "image" => true,
            "checkbox" | "radio" => space,
            _ => false,
        },
        _ => false,
    }
}

/// A `FocusEvent` of `kind` (M26.6): bubbling for `focusin` and
/// `focusout`, never cancelable, `related` its related target.
fn focus_event(kind: &str, bubbles: bool, related: Option<NodeId>) -> js::EventInit {
    js::EventInit::new("FocusEvent", kind)
        .with("bubbles", js::Field::Bool(bubbles))
        .with("composed", js::Field::Bool(true))
        .with("view", js::Field::Window)
        .with(
            "relatedTarget",
            js::Field::Node(related.map(scripts::handle)),
        )
}

/// The event an animation's or a transition's change of phase queued,
/// as a script sees it (M26): an AnimationEvent or a TransitionEvent,
/// bubbling and not cancelable, with its name, elapsed time and
/// pseudo-element.
fn animation_event(event: &AnimationEvent) -> js::EventInit {
    let transition = matches!(
        event.kind,
        EventKind::TransitionRun
            | EventKind::TransitionStart
            | EventKind::TransitionEnd
            | EventKind::TransitionCancel
    );
    let (interface, name) = if transition {
        ("TransitionEvent", "propertyName")
    } else {
        ("AnimationEvent", "animationName")
    };
    let pseudo = match event.pseudo {
        Some(style::selector_matching::PseudoElement::Before) => "::before",
        Some(style::selector_matching::PseudoElement::After) => "::after",
        Some(style::selector_matching::PseudoElement::Marker) => "::marker",
        _ => "",
    };
    js::EventInit::new(interface, event.kind.name())
        .with("bubbles", js::Field::Bool(true))
        .with(name, js::Field::String(event.name.to_string()))
        .with("elapsedTime", js::Field::Number(event.elapsed))
        .with("pseudoElement", js::Field::String(pseudo.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An engine over the bundled test font: the same text metrics on
    /// every machine, and a machine without fonts still runs the tests
    /// rather than passing them on nothing.
    fn test_engine() -> Engine {
        Engine::with_fonts(
            text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
                .expect("the bundled font parses"),
        )
    }

    #[test]
    fn an_image_is_drawn_at_its_tiles_size_on_the_raster_whole_and_unturned() {
        let image = |crop, orientation| paint::ImageRef {
            id: 1,
            frame: 0,
            orientation,
            crop,
            pixels: None,
        };
        let plain = image(None, images::Orientation::Normal);
        let tile = [5.0, 5.0, 40.0, 10.0];
        let identity = style::transform::Matrix::IDENTITY;
        assert_eq!(drawn_size(&identity, tile, &plain), (40.0, 10.0));
        let doubled = style::transform::Matrix::scale(2.0, 3.0, 1.0);
        assert_eq!(drawn_size(&doubled, tile, &plain), (80.0, 30.0));
        let slice = image(Some([0.0, 0.5, 0.25, 1.0]), images::Orientation::Normal);
        assert_eq!(drawn_size(&identity, tile, &slice), (160.0, 20.0));
        let turned = image(None, images::Orientation::Rotate90);
        assert_eq!(drawn_size(&identity, tile, &turned), (10.0, 40.0));
    }

    #[test]
    fn a_gradient_background_is_drawn_with_no_fetch() {
        let mut engine = test_engine();
        engine.load_document(
            "<body style='margin: 0'><div style='width: 100px; height: 10px;              background: linear-gradient(to right, red, blue)'></div>",
        );
        let frame = engine.frame(120, 20);
        let at = |x: usize, y: usize| {
            let i = (y * 120 + x) * 4;
            [frame[i], frame[i + 1], frame[i + 2]]
        };
        assert_eq!(at(0, 5), [254, 0, 1]);
        assert_eq!(at(99, 5), [1, 0, 254]);
        assert_eq!(at(110, 5), [255, 255, 255]);
    }

    /// A test engine showing the built-in page.
    fn builtin_engine() -> Engine {
        let mut engine = test_engine();
        engine.load_document(pages::ABOUT_SILVER);
        engine
    }

    #[test]
    fn a_new_engine_shows_the_initial_about_blank() {
        let engine = test_engine();
        assert_eq!(engine.url().map(Url::as_str), Some("about:blank"));
        assert_eq!(engine.document().text_content(engine.document().root()), "");
        assert_eq!(engine.load_state(), &LoadState::Loaded);
        assert!(!engine.can_go_back() && !engine.can_go_forward());
    }

    #[test]
    fn a_root_with_no_box_leaves_the_canvas_white() {
        let mut engine = test_engine();
        engine.load_document("<html style='display: none'><p>hidden</p>");
        let frame = engine.frame(64, 32);
        assert!(frame.chunks(4).all(|p| p == [0xff, 0xff, 0xff, 0xff]));
    }

    #[test]
    fn frame_produces_full_size_buffer() {
        let mut engine = test_engine();
        let buf = engine.frame(64, 32);
        assert_eq!(buf.len(), 64 * 32 * 4);
    }

    #[test]
    fn builtin_document_parses_to_expected_text() {
        let engine = builtin_engine();
        assert!(
            engine
                .document()
                .text_content(engine.document().root())
                .contains("Hello from Silver Browser")
        );
    }

    #[test]
    fn loading_a_document_replaces_the_tree() {
        let mut engine = test_engine();
        engine.load_document("<h1>Alpha</h1>");
        assert_eq!(
            engine.document().text_content(engine.document().root()),
            "Alpha"
        );
    }

    #[test]
    fn different_documents_render_differently() {
        let mut engine = test_engine();
        let before = engine.frame(128, 256);
        engine.load_document("<p>XX</p>");
        let after = engine.frame(128, 256);
        assert_ne!(before, after);
    }

    #[test]
    fn builtin_style_element_parses_into_stylesheet() {
        let engine = builtin_engine();
        assert_eq!(engine.stylesheets().len(), 1);
        let rules = engine.stylesheets()[0].rules.len();
        assert!(rules >= 2, "expected at least h1 and p rules, got {rules}");
    }

    #[test]
    fn loading_a_document_replaces_stylesheets() {
        let mut engine = builtin_engine();
        assert!(!engine.stylesheets().is_empty());
        engine.load_document("<p>no styles here</p>");
        assert!(engine.stylesheets().is_empty());
    }

    #[test]
    fn style_elements_accumulate_into_sheets() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>p { color: red; }</style><p>x</p><style>p { margin: 0; }</style>",
        );
        assert_eq!(engine.stylesheets().len(), 2);
    }

    // ---- M5 acceptance (roadmap M5.9) --------------------------------------

    #[test]
    fn deleting_the_style_block_still_yields_ua_styling() {
        let mut engine = test_engine();
        // No `<style>` at all: zero author stylesheets, but the cascade
        // always includes the built-in UA sheet underneath.
        engine.load_document("<html><body><h1>Title</h1><p>Body</p></body></html>");
        assert!(engine.stylesheets().is_empty());
        let h1 = first(&engine, "h1");
        let h1 = engine.styles().get(h1).expect("the h1 is styled");
        // ua.css: `h1 { display: block; font-size: 32px; ... }`.
        assert_eq!(h1.display, style::computed::Display::Block);
        assert_eq!(h1.font_size_px, 32.0);
    }

    #[test]
    fn adding_a_color_rule_visibly_changes_rendered_pixels() {
        // The literal M5.9 acceptance scenario: a screenshot-diff v0 —
        // compare frame buffers before and after a color-changing rule.
        let mut engine = test_engine();
        engine.load_document("<p>Some text to render</p>");
        let before = engine.frame(200, 100);
        engine.load_document("<style>p { color: red; }</style><p>Some text to render</p>");
        let after = engine.frame(200, 100);
        assert_ne!(
            before, after,
            "changing `color` must change the rendered pixels"
        );
    }

    #[test]
    fn a_lists_markers_are_drawn_as_the_reference_browser_draws_them() {
        // M21.6: a disc, a circle stroked a pixel wide along the square's
        // inscribed circle and a square, 5 pixels at 16, their left edge
        // 17 before the list item's, 7 below the line's top — rows of 30.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body, ul { margin: 0 } li { height: 30px }</style>\
             <ul><li>&nbsp;</li></ul><ul style='list-style-type: circle'><li>&nbsp;</li></ul>\
             <ul style='list-style-type: square'><li>&nbsp;</li></ul>",
        );
        let pixels = engine.frame(200, 100);
        let red = |x: usize, y: usize| pixels[(y * 200 + x) * 4];
        let (dark, white) = (100, 250);
        // The disc's middle, and nothing past its square.
        assert!(red(25, 9) < dark);
        assert!(red(22, 9) > white && red(28, 9) > white);
        // The circle's ring, open in the middle.
        assert!(red(22, 39) < 200 && red(28, 39) < 200);
        assert!(red(25, 39) > white);
        // The square, corner to corner.
        assert!(red(23, 67) < dark && red(27, 71) < dark);
        assert!(red(22, 67) > white && red(28, 71) > white);
    }

    #[test]
    fn text_decorations_are_drawn_where_the_reference_browser_draws_them() {
        // M21.7, the rows measured in the reference browser at 16 pixels,
        // each line a pixel thick: an underline a pixel below the
        // baseline, an overline its thickness above the ascent, a
        // line-through a third of the ascent up, a double underline's
        // second line three below the first. The text is transparent;
        // the lines are black.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 } div { height: 40px; padding-left: 10px; color: transparent }\
             </style><div style='text-decoration: underline #000'>xxxxxx</div>\
             <div style='text-decoration: overline #000'>xxxxxx</div>\
             <div style='text-decoration: line-through #000'>xxxxxx</div>\
             <div style='text-decoration: underline double #000'>xxxxxx</div>",
        );
        let pixels = engine.frame(200, 160);
        let dark_rows: Vec<usize> = (0..160)
            .filter(|&y| (0..200).any(|x| pixels[(y * 200 + x) * 4] < 100))
            .collect();
        assert_eq!(dark_rows, [16, 39, 89, 136, 139]);
        // From the text's start to its end.
        let dark = |y: usize| -> Vec<usize> {
            (0..200)
                .filter(|&x| pixels[(y * 200 + x) * 4] < 100)
                .collect()
        };
        let row = dark(16);
        assert_eq!(row.first(), Some(&10));
        assert!(row.windows(2).all(|w| w[1] == w[0] + 1));
    }

    #[test]
    fn display_none_removes_the_element_from_the_frame() {
        let mut engine = test_engine();
        engine.load_document("<p>Visible</p>");
        let with_p = engine.frame(200, 100);
        engine.load_document("<style>p { display: none; }</style><p>Visible</p>");
        let without_p = engine.frame(200, 100);
        assert_ne!(with_p, without_p);
    }

    // ---- M6 acceptance (roadmap M6.10) -------------------------------------

    #[test]
    fn fragments_resolve_real_box_model_geometry() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>div { display: block; width: 100px; height: 50px;
                          margin: 10px; padding: 5px;
                          border-width: 2px; border-style: solid; }</style>\
             <div></div>",
        );
        let boxes = boxes_of(&engine, "div");
        let div = &boxes[0];
        // border-box: 100 + 2*5 + 2*2 = 114 wide, 50 + 2*5 + 2*2 = 64
        // tall. The implied body's 8px margin (the UA sheet) puts it at
        // x = 18; its top margin and the div's adjoin, so y is their
        // collapsed 10, not 18.
        assert_eq!((div.x, div.y, div.w, div.h), (18.0, 10.0, 114.0, 64.0));
    }

    /// The box fragments of every `tag` element, in tree order.
    fn boxes_of(engine: &Engine, tag: &str) -> Vec<layout::Fragment> {
        let document = engine.document();
        engine
            .fragments(800.0)
            .into_iter()
            .filter(|f| {
                matches!(f.kind, layout::FragmentKind::Box) && document.tag(f.node) == Some(tag)
            })
            .collect()
    }

    /// The first element named `tag` in the engine's document, in tree
    /// order.
    fn first(engine: &Engine, tag: &str) -> NodeId {
        let document = engine.document();
        document
            .subtree(document.root())
            .find(|&node| document.tag(node) == Some(tag))
            .unwrap_or_else(|| panic!("a {tag} element"))
    }

    #[test]
    fn adjacent_margins_collapse_through_the_real_pipeline() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>p { display: block; margin: 20px 0; }</style>\
             <p>one</p><p>two</p>",
        );
        let boxes = boxes_of(&engine, "p");
        assert_eq!(boxes.len(), 2);
        // The gap between the two border-boxes is max(20, 20) = 20, not
        // the sum (40) — `boxes[0].h` is its own real content height
        // (one line of text), unrelated to the collapsing itself.
        assert_eq!(boxes[1].y, boxes[0].y + boxes[0].h + 20.0);
    }

    #[test]
    fn builtin_document_renders_the_card_background_and_border() {
        let engine = builtin_engine();
        let fragments = engine.fragments(800.0);
        let has_card_background = fragments.iter().any(|f| {
            matches!(f.kind, layout::FragmentKind::Box) && f.style.background_color.a > 0.0
        });
        assert!(
            has_card_background,
            "the built-in demo's .card background should paint (M6.8)"
        );
    }

    // ---- M16 acceptance (roadmap M16.6) -------------------------------------

    #[test]
    fn the_builtin_page_shows_each_pseudo_element() {
        use style::selector_matching::PseudoElement::{After, Before, FirstLetter};
        let engine = builtin_engine();
        let fragments = engine.fragments(800.0);
        let text = |f: &layout::Fragment| match &f.kind {
            layout::FragmentKind::Text(t) => Some(t.clone()),
            _ => None,
        };
        let generated = |pseudo| -> Vec<String> {
            fragments
                .iter()
                .filter(|f| f.generated == Some(pseudo))
                .filter_map(text)
                .collect()
        };
        let texts: Vec<String> = fragments.iter().filter_map(text).collect();
        // A drop cap: the lead paragraph's first letter, floated.
        assert!(
            fragments.iter().any(|f| f.generated == Some(FirstLetter)
                && f.style.float == style::computed::Float::Left)
        );
        assert_eq!(generated(FirstLetter), ["P"]);
        // Its first line in capitals, and its last in its own case.
        assert!(texts.iter().any(|t| t == "SEUDO-ELEMENTS"));
        assert!(texts.iter().any(|t| t == "citation"));
        // The cards labelled from their attributes, and closed.
        let before = generated(Before);
        for label in ["MODEL", "HIDDEN", "AUTO"] {
            assert!(before.iter().any(|t| t == label), "{label}");
        }
        let after = generated(After);
        assert!(after.iter().any(|t| t == "\u{a7}"));
        // Quotation marks, a quotation in a quotation, and French ones.
        for mark in ["\u{201c}", "\u{2018}", "\u{ab}\u{a0}"] {
            assert!(before.iter().any(|t| t == mark), "{mark}");
        }
        for mark in ["\u{2019}", "\u{201d}", "\u{a0}\u{bb}"] {
            assert!(after.iter().any(|t| t == mark), "{mark}");
        }
    }

    // ---- M17 acceptance (roadmap M17.10) ------------------------------------

    #[test]
    fn the_builtin_page_lays_its_cards_out_with_flexbox() {
        // The cards are flex items: at 800px three share the first row,
        // side by side and grown alike, and the fourth wraps onto a row
        // of its own, which it fills; at 400px each has a row.
        let engine = builtin_engine();
        let cards = |width: f32| -> Vec<(f32, f32, f32)> {
            let document = engine.document();
            engine
                .fragments(width)
                .into_iter()
                .filter(|f| {
                    matches!(f.kind, layout::FragmentKind::Box)
                        && f.generated.is_none()
                        && document
                            .attr(f.node, "class")
                            .is_some_and(|c| c.split(' ').any(|c| c == "card"))
                })
                .map(|f| (f.x, f.y, f.w))
                .collect()
        };
        let wide = cards(800.0);
        assert_eq!(wide.len(), 4);
        assert!(wide[0].1 == wide[1].1 && wide[1].1 == wide[2].1, "{wide:?}");
        assert!(wide[0].0 < wide[1].0 && wide[1].0 < wide[2].0, "{wide:?}");
        assert!(wide[0].2 == wide[1].2 && wide[1].2 == wide[2].2, "{wide:?}");
        assert!(wide[3].1 > wide[0].1 && wide[3].2 > wide[0].2, "{wide:?}");
        let narrow = cards(400.0);
        assert!(
            narrow
                .windows(2)
                .all(|p| p[1].1 > p[0].1 && p[1].0 == p[0].0),
            "{narrow:?}"
        );
    }

    // ---- M18 acceptance (roadmap M18.7) ------------------------------------

    #[test]
    fn the_builtin_page_lays_its_tiles_out_with_grid() {
        // The tiles are grid items in as many 150px-or-wider columns as
        // the page has room for: five at 800px and two at 400px, the
        // first tile across them all and the wide ones over two columns
        // and the gap between. Dense packing puts a later narrow tile in
        // a hole a wide one left: at 400px, M12 beside M9.
        let engine = builtin_engine();
        let tiles = |width: f32| -> Vec<(f32, f32, f32)> {
            let document = engine.document();
            engine
                .fragments(width)
                .into_iter()
                .filter(|f| {
                    matches!(f.kind, layout::FragmentKind::Box)
                        && f.generated.is_none()
                        && document
                            .attr(f.node, "class")
                            .is_some_and(|c| c.split(' ').any(|c| c == "tile"))
                })
                .map(|f| (f.x, f.y, f.w))
                .collect()
        };
        for (width, columns) in [(800.0, 5), (400.0, 2)] {
            let tiles = tiles(width);
            assert_eq!(tiles.len(), 14, "{tiles:?}");
            let (all, rest) = tiles.split_first().expect("a tile");
            let mut lefts: Vec<f32> = rest.iter().map(|t| t.0).collect();
            lefts.sort_by(f32::total_cmp);
            lefts.dedup();
            assert_eq!(lefts.len(), columns, "{width}: {tiles:?}");
            let right = rest.iter().map(|t| t.0 + t.2).fold(0.0, f32::max);
            assert!(
                all.0 == lefts[0] && all.0 + all.2 == right,
                "{width}: {tiles:?}"
            );
            assert!(rest.iter().all(|t| t.1 > all.1), "{width}: {tiles:?}");
            let narrow = rest[0].2;
            for wide in [rest[1], rest[2], rest[5], rest[11]] {
                assert!(
                    (wide.2 - (2.0 * narrow + 8.0)).abs() < 0.01,
                    "{width}: {tiles:?}"
                );
            }
        }
        let narrow = tiles(400.0);
        let (m9, m10, m12) = (narrow[1], narrow[2], narrow[4]);
        assert!(m12.1 == m9.1 && m12.0 > m9.0 && m10.1 > m9.1, "{narrow:?}");
    }

    #[test]
    fn the_user_agent_sheets_margins_are_in_ems_of_the_elements_own_font() {
        // HTML's rendering section: a heading's size in ems of its
        // parent's font, and its margins and a paragraph's in ems of its
        // own, whatever font size the author gives it.
        let mut engine = test_engine();
        engine.load_document(
            "<h1>a</h1><h1 style=\"font-size: 24px\">b</h1><h3>c</h3>\
             <p style=\"font-size: 20px\">d</p><div style=\"font-size: 10px\"><h2>e</h2></div>",
        );
        let styles: Vec<(f32, f32)> = {
            let document = engine.document().clone();
            let elements: Vec<NodeId> = document
                .subtree(document.root())
                .filter(|&n| matches!(document.tag(n), Some("h1" | "h2" | "h3" | "p")))
                .collect();
            let styles = engine.styles();
            elements
                .into_iter()
                .map(|n| {
                    let style = styles.get(n).expect("styled");
                    let margin = match &style.margin[style::computed::TOP] {
                        style::computed::LengthPercentageAuto::Px(px) => *px,
                        other => panic!("{other:?}"),
                    };
                    (style.font_size_px, margin)
                })
                .collect()
        };
        let near =
            |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01;
        let expected = [
            (32.0, 21.44),
            (24.0, 16.08),
            (18.72, 18.72),
            (20.0, 20.0),
            (15.0, 12.45),
        ];
        assert!(
            styles.iter().zip(expected).all(|(s, e)| near(*s, e)),
            "{styles:?}"
        );
    }

    // ---- M21 acceptance (roadmap M21.8) ------------------------------------

    #[test]
    fn the_builtin_page_takes_its_themes_colours_under_each_scheme() {
        // Its colours are custom properties declared once on :root, for a
        // light scheme and again for a dark one, which every rule reads.
        use style::computed::Color;
        let hex = |rgb: u32| Color::rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
        for (scheme, page, heading, text, surface, link) in [
            (
                ColorScheme::Light,
                0xeff1f5,
                0x4c4f69,
                0x5c5f77,
                0xccd0da,
                0x1e66f5,
            ),
            (
                ColorScheme::Dark,
                0x1e1e2e,
                0xcdd6f4,
                0xa6adc8,
                0x313244,
                0x89b4fa,
            ),
        ] {
            let mut engine = test_engine();
            engine.set_preferences(Preferences {
                color_scheme: scheme,
                ..Preferences::default()
            });
            engine.load_document(pages::ABOUT_SILVER);
            let pixels = engine.frame(800, 600);
            let document = engine.document().clone();
            let styles = engine.styles();
            let first = |tag: &str, class: Option<&str>| {
                let node = document
                    .subtree(document.root())
                    .find(|&n| {
                        document.tag(n) == Some(tag)
                            && class.is_none_or(|c| {
                                document
                                    .attr(n, "class")
                                    .is_some_and(|v| v.split(' ').any(|v| v == c))
                            })
                    })
                    .unwrap_or_else(|| panic!("no {tag}"));
                styles.get(node).expect("styled").clone()
            };
            let html = first("html", None);
            assert_eq!(html.background_color, hex(page), "{scheme:?}");
            // The canvas is painted in it.
            let canvas = Color::rgb8(pixels[0], pixels[1], pixels[2]);
            assert_eq!(canvas, hex(page), "{scheme:?}");
            assert_eq!(first("h1", None).color, hex(heading), "{scheme:?}");
            assert_eq!(first("h2", None).color, hex(heading), "{scheme:?}");
            assert_eq!(first("p", None).color, hex(text), "{scheme:?}");
            let card = first("div", Some("card"));
            assert_eq!(card.background_color, hex(surface), "{scheme:?}");
            assert_eq!(card.border_color[0], hex(link), "{scheme:?}");
            assert_eq!(first("a", None).color, hex(link), "{scheme:?}");
        }
    }

    #[test]
    fn the_builtin_pages_sections_are_numbered_by_counters() {
        use style::selector_matching::PseudoElement::Before;
        let engine = builtin_engine();
        let document = engine.document().clone();
        let numbers: Vec<String> = engine
            .fragments(800.0)
            .iter()
            .filter(|f| {
                f.generated == Some(Before) && matches!(document.tag(f.node), Some("h2" | "h3"))
            })
            .filter_map(|f| match &f.kind {
                layout::FragmentKind::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            numbers,
            [
                "1.", "1.1", "1.2", "1.3", "2.", "3.", "3.1", "3.2", "4.", "5."
            ]
        );
    }

    #[test]
    fn the_builtin_page_relays_out_at_its_breakpoints() {
        // M20.5: below 600px the cards stack, from 1000px the tiles are
        // at least 200px wide; a resize across a breakpoint cascades the
        // page again, one between them does not.
        let mut engine = builtin_engine();
        let boxes = |engine: &mut Engine, width: u32, class: &str| -> Vec<(f32, f32)> {
            engine.frame(width, 600);
            let document = engine.document().clone();
            engine
                .fragments(width as f32)
                .into_iter()
                .filter(|f| {
                    matches!(f.kind, layout::FragmentKind::Box)
                        && f.generated.is_none()
                        && document
                            .attr(f.node, "class")
                            .is_some_and(|c| c.split(' ').any(|c| c == class))
                })
                .map(|f| (f.x, f.w))
                .collect()
        };
        // A row at 800px: the cards start at more than one place.
        let cards = boxes(&mut engine, 800, "card");
        assert_eq!(cards.len(), 4);
        assert!(cards.iter().any(|c| c.0 != cards[0].0), "{cards:?}");
        let runs = engine.style_runs();
        boxes(&mut engine, 700, "card");
        assert_eq!(engine.style_runs(), runs, "no breakpoint between");
        // A column at 500px: each card the column's width.
        let cards = boxes(&mut engine, 500, "card");
        assert_eq!(engine.style_runs(), runs + 1);
        assert!(
            cards.iter().all(|c| *c == cards[0]) && cards[0].1 > 450.0,
            "{cards:?}"
        );
        // The tiles: at least 150px wide at 800px, 200px at 1100px.
        let narrowest = |tiles: &[(f32, f32)]| tiles.iter().map(|t| t.1).fold(f32::MAX, f32::min);
        let tiles = boxes(&mut engine, 800, "tile");
        assert!((150.0..200.0).contains(&narrowest(&tiles)), "{tiles:?}");
        let tiles = boxes(&mut engine, 1100, "tile");
        assert!(narrowest(&tiles) >= 200.0, "{tiles:?}");
    }

    // ---- M7 acceptance (roadmap M7.8) --------------------------------------

    #[test]
    fn narrow_viewport_wraps_text_onto_more_lines_than_a_wide_one() {
        let mut engine = test_engine();
        engine
            .load_document("<p>one two three four five six seven eight nine ten eleven twelve</p>");
        // f32 isn't `Ord` (NaN), so count distinct baselines by rounding
        // to whole pixels and deduping, rather than putting raw floats
        // in a set.
        let distinct_baselines = |viewport_width: f32| -> usize {
            let mut ys: Vec<i32> = engine
                .fragments(viewport_width)
                .iter()
                .filter_map(|f| match f.kind {
                    layout::FragmentKind::Text(_) => Some(f.y.round() as i32),
                    layout::FragmentKind::Box
                    | layout::FragmentKind::InlineBox
                    | layout::FragmentKind::Marker(_)
                    | layout::FragmentKind::Replaced(_) => None,
                })
                .collect();
            ys.sort_unstable();
            ys.dedup();
            ys.len()
        };

        let wide_lines = distinct_baselines(5000.0);
        let narrow_lines = distinct_baselines(60.0);
        assert!(
            narrow_lines > wide_lines,
            "a narrow viewport ({narrow_lines} lines) should wrap onto more lines than a wide one ({wide_lines})"
        );
    }

    #[test]
    fn text_align_center_shifts_text_right_of_a_left_aligned_equivalent() {
        let mut engine = test_engine();
        engine.load_document("<p>hi</p>");
        let left_x = engine
            .fragments(800.0)
            .into_iter()
            .find(|f| matches!(f.kind, layout::FragmentKind::Text(_)))
            .map(|f| f.x)
            .expect("a text fragment");

        engine.load_document("<style>p { text-align: center; }</style><p>hi</p>");
        let center_x = engine
            .fragments(800.0)
            .into_iter()
            .find(|f| matches!(f.kind, layout::FragmentKind::Text(_)))
            .map(|f| f.x)
            .expect("a text fragment");

        assert!(
            center_x > left_x,
            "centered text should start further right than left-aligned text"
        );
    }

    #[test]
    fn a_frame_past_the_pixel_cap_is_empty_rather_than_gigabytes() {
        let mut engine = test_engine();
        assert!(engine.frame(100_000, 100_000).is_empty());
        assert_eq!(engine.frame(64, 32).len(), 64 * 32 * 4);
    }

    #[test]
    fn a_hostile_font_size_renders_without_taking_the_process_down() {
        // 1e9px used to overflow the mask arithmetic (debug) or ask for
        // gigabytes (release); 1e39px is infinity in f32.
        let mut engine = test_engine();
        for size in ["1e9px", "1e39px", "50000px", "-16px"] {
            engine.load_document(&format!("<p style=\"font-size:{size};margin:0\">M</p>"));
            assert_eq!(engine.frame(200, 100).len(), 200 * 100 * 4, "{size}");
        }
    }

    #[test]
    fn non_finite_lengths_do_not_put_text_at_the_corner() {
        // `1e39px` margins used to collapse to NaN and every fragment
        // landed at y = NaN, which the rasterizer drew at the top-left.
        let mut engine = test_engine();
        engine.load_document(
            "<div style=\"margin-top:1e39px;margin-bottom:-1e39px\"></div><p style=\"margin:0\">hi</p>",
        );
        for f in engine.fragments(800.0) {
            assert!(f.x.is_finite() && f.y.is_finite(), "{f:?}");
        }
    }

    // ---- M9 ---------------------------------------------------------------

    /// Enough paragraphs to be far taller than a 100px viewport.
    const TALL: &str = "<style>p { margin: 0; font-size: 20px; }</style>\
        <p>one</p><p>two</p><p>three</p><p>four</p><p>five</p><p>six</p>\
        <p>seven</p><p>eight</p><p>nine</p><p>ten</p><p>eleven</p><p>twelve</p>";

    /// The pixel at `(x, y)` of a `width`-wide RGBA8 buffer.
    fn px(buf: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * width + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    #[test]
    fn scrolling_is_clamped_to_the_document_and_animates_to_its_target() {
        let mut engine = test_engine();
        engine.load_document(TALL);
        // Before a frame there is no viewport to scroll in.
        engine.scroll_by(100.0);
        assert_eq!(engine.scroll_target(), 0.0);
        engine.frame(200, 100);
        let max = engine.max_scroll();
        assert!(max > 100.0, "twelve 20px lines overflow a 100px viewport");
        engine.scroll_by(1e9);
        assert_eq!(engine.scroll_target(), max);
        assert_eq!(engine.scroll_y(), 0.0, "the shown offset moves per tick");
        let mut ticks = 0;
        while engine.tick() {
            ticks += 1;
            assert!(ticks < 100, "the animation settles");
        }
        assert_eq!(engine.scroll_y(), max);
        assert!(ticks >= 3, "it took more than one frame ({ticks})");
        engine.scroll_by(-1e9);
        assert_eq!(engine.scroll_target(), 0.0);
        engine.scroll_to(f32::INFINITY);
        assert_eq!(engine.scroll_target(), max);
        engine.scroll_to(f32::NAN);
        assert_eq!(engine.scroll_target(), max, "a NaN is ignored");
        engine.scroll_to(f32::NEG_INFINITY);
        assert_eq!(engine.scroll_target(), 0.0);
        engine.set_scroll(50.0);
        assert_eq!((engine.scroll_y(), engine.scroll_target()), (50.0, 50.0));
        assert!(!engine.tick());
        engine.scroll_to_fraction(1.0);
        assert_eq!(engine.scroll_y(), max);
        engine.scroll_to_fraction(0.0);
        assert_eq!(engine.scroll_y(), 0.0);
        // A new document starts at the top.
        engine.set_scroll(50.0);
        engine.load_document(TALL);
        assert_eq!(engine.scroll_y(), 0.0);
    }

    /// Draws `engine`'s 200 by 100 frame for `ms` milliseconds on the
    /// test's clock, and returns the offset it shows.
    fn frame_at_ms(engine: &mut Engine, ms: f64) -> f32 {
        let now = Duration::from_secs_f64(ms / 1000.0);
        engine.frame_at(200, 100, FrameTime::at(now));
        engine.scroll_y()
    }

    #[test]
    fn the_scroll_animation_runs_on_time_whatever_the_frame_rate() {
        // Two engines scroll the same way, one drawing sixty frames a
        // second, the other thirty (M13.4): after the same time both
        // show the same offset, a third of the remaining distance gone
        // every sixtieth of a second.
        let (mut fast, mut slow) = (test_engine(), test_engine());
        for engine in [&mut fast, &mut slow] {
            engine.load_document(TALL);
            frame_at_ms(engine, 0.0);
            engine.scroll_by(1e9);
        }
        let max = fast.scroll_target();
        assert!(max > 100.0);
        // Ten idle seconds later, the first frame: one frame's step, a
        // third of the way, not the time since the last frame.
        let start = 10_000.0;
        let third = frame_at_ms(&mut fast, start);
        assert!((third - max / 3.0).abs() < 1e-3, "{third} of {max}");
        frame_at_ms(&mut slow, start);
        // A tenth of a second more: six frames, or three.
        for k in 1..=6 {
            frame_at_ms(&mut fast, start + k as f64 * 1000.0 / 60.0);
        }
        for k in 1..=3 {
            frame_at_ms(&mut slow, start + k as f64 * 1000.0 / 30.0);
        }
        let expected = max - max * (2.0f32 / 3.0).powi(7);
        assert!(
            (fast.scroll_y() - expected).abs() < 0.01,
            "{}",
            fast.scroll_y()
        );
        assert!((slow.scroll_y() - fast.scroll_y()).abs() < 0.01);
        assert!(fast.needs_frame(), "still on its way");
        // A frame drawn late makes up the time in one step: here, all
        // the way.
        frame_at_ms(&mut fast, start + 2100.0);
        assert_eq!(fast.scroll_y(), max);
        assert!(!fast.needs_frame());
    }

    #[test]
    fn a_frame_records_its_stages_and_whether_it_overran() {
        let mut engine = test_engine();
        engine.load_document(TALL);
        let now = Duration::from_secs(1);
        // Due when it begins: overrun.
        engine.frame_at(200, 100, FrameTime { now, deadline: now });
        let stats = engine.frame_stats();
        assert!(stats.overran);
        assert!(stats.layout > Duration::ZERO && stats.raster > Duration::ZERO);
        assert!(stats.total >= stats.style + stats.layout + stats.paint + stats.raster);
        // An hour to spare: on time.
        let deadline = now + Duration::from_secs(3600);
        engine.frame_at(200, 100, FrameTime { now, deadline });
        assert!(!engine.frame_stats().overran);
        // Past the frame cap: nothing drawn, nothing taken.
        engine.frame_at(100_000, 100_000, FrameTime::at(now));
        assert_eq!(engine.frame_stats(), FrameStats::default());
    }

    #[test]
    fn a_burst_of_mutations_between_frames_costs_one_pass() {
        // Changes are applied as they come and their work is left to
        // the next frame (M13.4): ten paragraphs appended one by one,
        // one layout, each paragraph restyled once.
        let mut engine = test_engine();
        engine.load_document("<p>one</p>");
        engine.frame(200, 100);
        let (layouts, restyled) = (engine.layout_runs(), engine.restyled_elements());
        let body = first(&engine, "body");
        for i in 0..10 {
            engine.mutate(|document| {
                let p = document.create_element("p").expect("an element");
                let text = document.create_text(&format!("{i}")).expect("a text node");
                document.append_child(p, text).expect("appended");
                document.append_child(body, p).expect("appended");
            });
        }
        assert_eq!(engine.layout_runs(), layouts, "nothing before the frame");
        assert!(engine.needs_frame());
        engine.frame(200, 100);
        assert_eq!(engine.layout_runs(), layouts + 1);
        assert_eq!(engine.restyled_elements(), restyled + 10);
        assert!(!engine.needs_frame());
    }

    #[test]
    fn an_offset_set_at_once_wants_a_frame_and_the_frame_lent_is_the_frame() {
        let mut engine = test_engine();
        engine.load_document(
            "<div id=s style=\"height: 20px; overflow: auto\"><p>a</p><p>b</p><p>c</p></div>\
             <p>one</p><p>two</p><p>three</p><p>four</p><p>five</p><p>six</p>",
        );
        let owned = engine.frame(200, 100);
        assert!(!engine.needs_frame());
        let lent = engine.frame_at(200, 100, FrameTime::at(Duration::ZERO));
        assert_eq!(lent, owned.as_slice());
        engine.set_scroll(30.0);
        assert!(engine.needs_frame(), "the offset shown changed");
        engine.frame(200, 100);
        assert!(!engine.needs_frame());
        engine.set_scroll(30.0);
        assert!(!engine.needs_frame(), "the same offset changes nothing");
        let s = node_by_id(&engine, "s");
        engine.set_element_scroll(s, 5.0);
        assert!(engine.needs_frame(), "a container's offset changed");
        engine.frame(200, 100);
        assert!(!engine.needs_frame());
    }

    #[test]
    fn a_scroll_reruns_paint_and_raster_but_not_layout() {
        let mut engine = test_engine();
        engine.load_document(TALL);
        assert_eq!(engine.layout_runs(), 0);
        engine.frame(200, 100);
        assert_eq!(engine.layout_runs(), 1);
        engine.scroll_by(40.0);
        while engine.tick() {}
        engine.frame(200, 100);
        engine.frame(200, 100);
        engine.hit_test(10.0, 10.0);
        assert_eq!(
            engine.layout_runs(),
            1,
            "scrolling and hit testing reuse the layout"
        );
        engine.frame(200, 150);
        assert_eq!(
            engine.layout_runs(),
            1,
            "a taller viewport changes nothing about layout"
        );
        engine.frame(300, 150);
        assert_eq!(engine.layout_runs(), 2, "a wider viewport does");
        engine.load_document("<p>another</p>");
        engine.frame(300, 150);
        assert_eq!(engine.layout_runs(), 3, "a new document does");
    }

    #[test]
    fn a_scrolled_frame_shows_the_content_further_down() {
        // A 100px red box at the top of the document, text, then a tall
        // spacer so there is room to scroll. Column 70 is right of the
        // text and left of the scrollbar.
        let mut engine = test_engine();
        engine.load_document(
            "<div style=\"height: 100px; background-color: #ff0000; margin: 0\"></div>\
             <p style=\"margin:0\">below</p>\
             <div style=\"height: 1000px; margin: 0\"></div>",
        );
        let top = engine.frame(100, 100);
        assert_eq!(px(&top, 100, 70, 50), [0xff, 0, 0, 0xff]);
        engine.set_scroll(60.0);
        assert_eq!(engine.scroll_y(), 60.0);
        let scrolled = engine.frame(100, 100);
        assert_eq!(
            px(&scrolled, 100, 70, 30),
            [0xff, 0, 0, 0xff],
            "the box moved up by 60"
        );
        assert_ne!(
            px(&scrolled, 100, 70, 50),
            [0xff, 0, 0, 0xff],
            "its bottom edge is now at 40"
        );
        assert_eq!(
            px(&scrolled, 100, 70, 0),
            [0xff, 0, 0, 0xff],
            "the top row is still inside it"
        );
    }

    #[test]
    fn hit_test_finds_the_innermost_box_or_text_under_the_point() {
        let mut engine = test_engine();
        // outer: 0..200 x 0..100; inner: at (20, 10), 100 x 50 with text.
        // The body's UA margin is zeroed so the numbers stay these.
        engine.load_document(
            "<style>body { margin: 0 }</style>\
             <div class=outer style=\"width: 200px; height: 100px; margin: 0; padding: 10px 0 0 20px\">\
               <div class=inner style=\"width: 100px; height: 50px; margin: 0\">hi</div>\
             </div>",
        );
        engine.frame(400, 300);
        let document = engine.document().clone();
        let tag = |node: Option<NodeId>| node.and_then(|n| document.tag(n).map(str::to_string));
        let class =
            |node: Option<NodeId>| node.and_then(|n| document.attr(n, "class").map(str::to_string));
        // On the inner box's text line.
        assert_eq!(
            class(engine.hit_test(25.0, 15.0)),
            Some("inner".to_string())
        );
        // On the inner box past the text, still the inner box.
        assert_eq!(
            class(engine.hit_test(110.0, 55.0)),
            Some("inner".to_string())
        );
        // In the outer box's padding.
        assert_eq!(class(engine.hit_test(5.0, 5.0)), Some("outer".to_string()));
        // Right of both boxes: the implied body, as wide as the viewport.
        assert_eq!(tag(engine.hit_test(300.0, 50.0)).as_deref(), Some("body"));
        // Below everything, and off the viewport: nothing.
        assert_eq!(engine.hit_test(50.0, 250.0), None);
        assert_eq!(engine.hit_test(-1.0, 5.0), None);
        assert_eq!(engine.hit_test(f32::NAN, 5.0), None);
    }

    #[test]
    fn overlapping_flex_items_are_hit_in_the_order_they_paint() {
        // css-flexbox-1 §4.3 (M17): items paint as units in order-modified
        // document order, and hit testing reads the same steps. #b overlaps
        // #a by 30px and paints over it; given a `z-index`, #a makes a
        // stacking context though it is not positioned, and is on top.
        for (extra, expected) in [("", "b"), ("#a { z-index: 1 }", "a")] {
            let mut engine = test_engine();
            engine.load_document(&format!(
                "<style>body {{ margin: 0 }} .f {{ display: flex }}
                        .f > div {{ width: 60px; height: 50px; flex: none }}
                        #b {{ margin-left: -30px }} {extra}</style>\
                 <div class=f><div id=a></div><div id=b></div></div>"
            ));
            engine.frame(200, 100);
            let document = engine.document().clone();
            let id = |node: Option<NodeId>| {
                node.and_then(|n| document.attr(n, "id").map(str::to_string))
            };
            assert_eq!(
                id(engine.hit_test(45.0, 25.0)).as_deref(),
                Some(expected),
                "{extra}"
            );
            assert_eq!(
                id(engine.hit_test(10.0, 25.0)).as_deref(),
                Some("a"),
                "{extra}"
            );
        }
    }

    #[test]
    fn grid_items_in_the_same_cell_are_hit_in_the_order_they_paint() {
        // css-grid-2 §6.5 (M18): items in the same cell overlap in
        // order-modified document order, and hit testing reads the same
        // steps. #b, after #a, is on top; `order` puts #a after it, and a
        // `z-index` makes #a a stacking context though it is not
        // positioned, on top of #b.
        for (extra, expected) in [
            ("", "b"),
            ("#a { order: 1 }", "a"),
            ("#a { z-index: 1 }", "a"),
        ] {
            let mut engine = test_engine();
            engine.load_document(&format!(
                "<style>body {{ margin: 0 }} .g {{ display: grid; grid-template-columns: 60px 60px }}
                        .g > div {{ grid-area: 1 / 1; height: 50px }} {extra}</style>\
                 <div class=g><div id=a></div><div id=b></div></div>"
            ));
            engine.frame(200, 100);
            let document = engine.document().clone();
            let id = |node: Option<NodeId>| {
                node.and_then(|n| document.attr(n, "id").map(str::to_string))
            };
            assert_eq!(
                id(engine.hit_test(30.0, 25.0)).as_deref(),
                Some(expected),
                "{extra}"
            );
        }
    }

    #[test]
    fn inline_blocks_are_hit_and_painted_through_the_real_pipeline() {
        // Two 50px inline-blocks on one line of a font-less row (M10.4):
        // the pointer over the second one answers with it, over its
        // text with it too, and the row is as tall as they are.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0; font-size: 0 }
                    .ib { display: inline-block; width: 50px; height: 50px;
                          background-color: #ff0000; font-size: 16px }</style>\
             <div class=row><span class=ib id=a>a</span><span class=ib id=b>b</span></div>",
        );
        engine.frame(200, 100);
        let document = engine.document().clone();
        let id =
            |node: Option<NodeId>| node.and_then(|n| document.attr(n, "id").map(str::to_string));
        assert_eq!(id(engine.hit_test(25.0, 25.0)), Some("a".to_string()));
        assert_eq!(id(engine.hit_test(75.0, 45.0)), Some("b".to_string()));
        assert_eq!(
            id(engine.hit_test(52.0, 8.0)),
            Some("b".to_string()),
            "its text"
        );
        assert_eq!(id(engine.hit_test(150.0, 25.0)), None, "the row");
        let row = boxes_of(&engine, "div");
        assert_eq!((row[0].w, row[0].h), (800.0, 50.0));
        let pixels = engine.frame(200, 100);
        let px = |x: usize, y: usize| &pixels[(y * 200 + x) * 4..(y * 200 + x) * 4 + 3];
        assert_eq!(px(10, 40), [0xff, 0x00, 0x00], "the first box's background");
        assert_eq!(px(90, 40), [0xff, 0x00, 0x00], "the second's");
        assert_eq!(px(150, 40), [0xff, 0xff, 0xff], "the canvas beside them");
    }

    /// The element with `id`.
    fn node_by_id(engine: &Engine, id: &str) -> NodeId {
        let document = engine.document();
        document
            .subtree(document.root())
            .find(|&node| document.attr(node, "id") == Some(id))
            .unwrap_or_else(|| panic!("an element with id {id}"))
    }

    #[test]
    fn the_wheel_scrolls_the_container_under_the_pointer_then_its_ancestors_then_the_viewport() {
        // An outer 50px scroll container holding an inner one and a
        // 200px block, then another 200px block below: the inner
        // scrolls 150, the outer 200, the viewport 150.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 } .s { overflow: auto; height: 50px; margin: 0 }
                    .tall { height: 200px; margin: 0 }</style>\
             <div class=s id=outer><div class=s id=inner><div class=tall></div></div>\
               <div class=tall></div></div>\
             <div class=tall></div>",
        );
        engine.frame(200, 100);
        let (outer, inner) = (node_by_id(&engine, "outer"), node_by_id(&engine, "inner"));
        assert_eq!(engine.element_max_scroll(inner), 150.0);
        assert_eq!(engine.element_max_scroll(outer), 200.0);
        assert_eq!(engine.max_scroll(), 150.0);
        // Over the inner box: it takes the turn.
        engine.wheel(10.0, 5.0, 40.0);
        assert_eq!(engine.element_scroll_target(inner), 40.0);
        assert_eq!(engine.element_scroll_target(outer), 0.0);
        assert_eq!(engine.scroll_target(), 0.0);
        // Three more reach its end, the last clamped (160 to 150).
        for _ in 0..3 {
            engine.wheel(10.0, 5.0, 40.0);
        }
        assert_eq!(engine.element_scroll_target(inner), 150.0);
        // The next goes to the outer, the inner being at its end.
        engine.wheel(10.0, 5.0, 40.0);
        assert_eq!(engine.element_scroll_target(outer), 40.0);
        assert_eq!(engine.element_scroll_target(inner), 150.0);
        // With both at their ends, the viewport takes it.
        engine.scroll_element_to(outer, 1e9);
        assert_eq!(engine.element_scroll_target(outer), 200.0);
        engine.wheel(10.0, 5.0, 40.0);
        assert_eq!(engine.scroll_target(), 40.0);
        // Back up: the inner can still move that way.
        engine.wheel(10.0, 5.0, -40.0);
        assert_eq!(engine.element_scroll_target(inner), 110.0);
        assert_eq!(engine.scroll_target(), 40.0);
        // Over no scroll container: the viewport.
        engine.wheel(10.0, 90.0, 40.0);
        assert_eq!(engine.scroll_target(), 80.0);
        // A turn of nothing, or of nonsense, does nothing.
        engine.wheel(10.0, 5.0, 0.0);
        engine.wheel(10.0, 5.0, f32::NAN);
        assert_eq!(engine.element_scroll_target(inner), 110.0);
        assert_eq!(engine.scroll_target(), 80.0);
    }

    #[test]
    fn a_scrolled_container_shows_and_hits_its_content_further_down() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }
                    .s { overflow: auto; height: 50px; width: 100px; margin: 0 }
                    .r { height: 30px; margin: 0; background-color: #ff0000 }
                    .b { height: 100px; margin: 0; background-color: #0000ff }</style>\
             <div class=s id=s><p class=r id=r></p><p class=b id=b></p></div>",
        );
        let id = |engine: &Engine, node: Option<NodeId>| {
            node.and_then(|n| engine.document().attr(n, "id").map(str::to_string))
        };
        let red = [0xff, 0x00, 0x00, 0xff];
        let blue = [0x00, 0x00, 0xff, 0xff];
        let top = engine.frame(200, 100);
        let s = node_by_id(&engine, "s");
        assert_eq!(px(&top, 200, 10, 10), red);
        let hit = engine.hit_test(10.0, 10.0);
        assert_eq!(id(&engine, hit), Some("r".to_string()));
        assert_eq!(engine.element_max_scroll(s), 80.0);

        engine.set_element_scroll(s, 30.0);
        assert_eq!(engine.element_scroll(s), 30.0);
        let scrolled = engine.frame(200, 100);
        assert_eq!(
            px(&scrolled, 200, 10, 10),
            blue,
            "the blue box moved up to the top"
        );
        let hit = engine.hit_test(10.0, 10.0);
        assert_eq!(id(&engine, hit), Some("b".to_string()));
        assert_eq!(
            px(&scrolled, 200, 10, 60),
            [0xff, 0xff, 0xff, 0xff],
            "below the box the content is clipped: the canvas"
        );
        // The overlay scrollbar at the padding box's right edge: the
        // track column, and the thumb 24px tall from 9.75 down.
        assert_eq!(px(&scrolled, 200, 95, 2), [0x31, 0x32, 0x44, 0xff]);
        assert_eq!(px(&scrolled, 200, 95, 20), [0x6c, 0x70, 0x86, 0xff]);
        assert_eq!(px(&scrolled, 200, 95, 45), [0x31, 0x32, 0x44, 0xff]);

        // Animated like the viewport: the shown offset approaches the
        // target over ticks, and a tick reports it.
        engine.scroll_element_to(s, 80.0);
        assert_eq!(engine.element_scroll(s), 30.0);
        let mut ticks = 0;
        while engine.tick() {
            ticks += 1;
            assert!(ticks < 100);
        }
        assert!(ticks >= 3);
        assert_eq!(engine.element_scroll(s), 80.0);
        // Past the end: clamped; a NaN: ignored; an element that is no
        // container: nowhere.
        engine.scroll_element_by(s, 1e9);
        assert_eq!(engine.element_scroll_target(s), 80.0);
        engine.scroll_element_to(s, f32::NAN);
        assert_eq!(engine.element_scroll_target(s), 80.0);
        let r = node_by_id(&engine, "r");
        engine.scroll_element_to(r, 10.0);
        assert_eq!(
            (
                engine.element_max_scroll(r),
                engine.element_scroll_target(r)
            ),
            (0.0, 0.0)
        );
        // A new document starts every container at the top.
        engine.load_document("<p>x</p>");
        assert_eq!(engine.element_scroll(s), 0.0);
    }

    #[test]
    fn an_offset_is_clamped_when_the_content_gets_shorter_and_hidden_scrolls_by_program_only() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 } .s { overflow: hidden; height: 20px; margin: 0 }
                    .c { height: 100vw; margin: 0 }</style>\
             <div class=s id=s><div class=c></div></div>\
             <div style=\"height: 500px; margin: 0\"></div>",
        );
        engine.frame(100, 100);
        let s = node_by_id(&engine, "s");
        assert_eq!(engine.element_max_scroll(s), 80.0);
        // `hidden` ignores the wheel: the viewport scrolls instead.
        engine.wheel(5.0, 5.0, 40.0);
        assert_eq!(engine.element_scroll_target(s), 0.0);
        assert_eq!(engine.scroll_target(), 40.0);
        // But scrolls by program.
        engine.set_element_scroll(s, 80.0);
        assert_eq!(engine.element_scroll(s), 80.0);
        // At a narrower viewport the content is 10px tall: nothing to
        // scroll, and the offset is clamped at the next frame.
        engine.frame(10, 100);
        assert_eq!(engine.element_max_scroll(s), 0.0);
        assert_eq!(
            (engine.element_scroll(s), engine.element_scroll_target(s)),
            (0.0, 0.0)
        );
    }

    #[test]
    fn form_controls_lay_out_as_sized_boxes_through_the_ua_sheet() {
        // M26.7: no author CSS at all. A row of controls on one line,
        // each the reference browser's size at its default font, showing
        // its value, label or chosen option.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }</style>\
             <form><input id=i value=hi><input id=i10 size=10><input type=hidden id=h>\
             <input type=checkbox id=c><button id=b>Go</button>\
             <select id=s><option>a</option><option selected>bee</option></select>\
             <textarea id=t>one\ntwo</textarea></form>",
        );
        engine.frame(800, 100);
        let frags = engine.fragments(800.0);
        let fragment = |id: &str| {
            let node = node_by_id(&engine, id);
            frags
                .iter()
                .find(|f| matches!(f.kind, layout::FragmentKind::Box) && f.node == node)
                .cloned()
        };
        let text = |t: &str| {
            frags
                .iter()
                .find(|f| matches!(&f.kind, layout::FragmentKind::Text(x) if x == t))
                .cloned()
        };
        let near = |a: f32, b: f32| (a - b).abs() < 0.02;
        let fonts = &engine.layout_fonts;
        let size = 13.3333;
        // A text field: 169px by 15 inside 2px and 1px of padding and a
        // 2px border; 99px wide at size 10. Its value is in it.
        let input = fragment("i").expect("the input's box");
        assert!(near(input.w, 177.0), "{}", input.w);
        assert!(near(input.h, 21.0), "{}", input.h);
        assert!(near(fragment("i10").expect("the small one").w, 107.0));
        let value = text("hi").expect("the value is shown");
        assert!(value.x >= input.x + 4.0 && value.x + value.w < input.x + input.w);
        assert!(fragment("h").is_none(), "a hidden input makes no box");
        let checkbox = fragment("c").expect("the checkbox's box");
        assert_eq!((checkbox.w, checkbox.h), (13.0, 13.0));
        assert!(frags.iter().any(|f| f.node == checkbox.node
            && matches!(
                f.kind,
                layout::FragmentKind::Replaced(layout::Replaced::Mark(layout::Mark::Checkbox {
                    checked: false,
                    ..
                }))
            )));
        // The button fits its label: 6px of padding and a 2px border
        // each side of the word, one line tall.
        let button = fragment("b").expect("the button's box");
        assert!(
            near(button.w, fonts.measure("Go", size) + 16.0),
            "{}",
            button.w
        );
        assert!(near(
            button.h,
            fonts.primary().metrics(size).line_height() + 6.0
        ));
        let label = text("Go").expect("the label is laid out");
        assert!(label.x > button.x && label.x + label.w < button.x + button.w);
        // The select is 19px tall and shows its chosen option alone, with
        // its arrow at the end.
        let select = fragment("s").expect("the select's box");
        assert!(near(select.h, 19.0), "{}", select.h);
        assert!(text("bee").is_some() && text("a").is_none());
        let arrow = frags
            .iter()
            .find(|f| {
                matches!(
                    f.kind,
                    layout::FragmentKind::Replaced(layout::Replaced::Mark(layout::Mark::Arrow))
                )
            })
            .expect("the arrow");
        assert!(arrow.x > text("bee").unwrap().x && arrow.x + arrow.w <= select.x + select.w);
        // The textarea is two lines tall and keeps its line break.
        let textarea = fragment("t").expect("the textarea's box");
        assert!(near(textarea.h, 36.0), "{}", textarea.h);
        assert!(near(textarea.w, 168.0), "{}", textarea.w);
        assert!(text("two").unwrap().y > text("one").unwrap().y);
        // The checkbox's bottom margin keeps it above the baseline the
        // field's text sits on.
        assert!(checkbox.y + checkbox.h < input.y + input.h);
    }

    #[test]
    fn hit_test_follows_the_scroll_offset_and_respects_clipping() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }</style>\
             <div class=spacer style=\"height: 100px; margin: 0\"></div>\
             <div class=clip style=\"height: 10px; overflow: hidden; margin: 0\">\
               <p style=\"margin: 0; font-size: 40px; width: 50px\">tall</p>\
             </div>\
             <div class=tail style=\"height: 1000px; margin: 0\"></div>",
        );
        engine.frame(200, 100);
        let document = engine.document().clone();
        let name = |node: Option<NodeId>| {
            node.map(|n| {
                document
                    .attr(n, "class")
                    .or(document.tag(n))
                    .unwrap_or_default()
                    .to_string()
            })
        };
        // The clipping box is at document y 100..110, off the unscrolled
        // viewport; after scrolling by 100 it is at viewport y 0..10,
        // and the visible top of its text is what is under the pointer.
        assert_eq!(name(engine.hit_test(5.0, 5.0)), Some("spacer".to_string()));
        engine.set_scroll(100.0);
        assert_eq!(engine.scroll_y(), 100.0);
        assert_eq!(name(engine.hit_test(5.0, 5.0)), Some("p".to_string()));
        // Right of the 50px-wide p, still inside the clipping box.
        assert_eq!(name(engine.hit_test(150.0, 5.0)), Some("clip".to_string()));
        // The 40px text line extends past the 10px box, but is clipped
        // there: viewport y 20 (document 120) is the tail, not the text.
        assert_eq!(name(engine.hit_test(5.0, 20.0)), Some("tail".to_string()));
    }

    #[test]
    fn floats_are_laid_out_painted_and_hit_through_the_real_pipeline() {
        // A red 50px left float, then a paragraph: the paragraph's text
        // starts beside the float, the float's pixels are red, and the
        // pointer over it answers with the float — over the part the
        // paragraph's box also covers too, floats being painted over
        // in-flow boxes (CSS2 Appendix E); the page is as tall as the
        // float, more than its one line (M11).
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }
                    .f { float: left; width: 50px; height: 50px; background-color: #ff0000 }
                    p { margin: 0 }</style>\
             <div class=f id=f></div><p id=p>beside</p>",
        );
        let pixels = engine.frame(200, 100);
        assert_eq!(px(&pixels, 200, 10, 10), [0xff, 0x00, 0x00, 0xff]);
        assert_eq!(px(&pixels, 200, 10, 45), [0xff, 0x00, 0x00, 0xff]);
        assert_eq!(px(&pixels, 200, 150, 45), [0xff, 0xff, 0xff, 0xff]);
        let f = node_by_id(&engine, "f");
        let p = node_by_id(&engine, "p");
        assert_eq!(engine.hit_test(10.0, 40.0), Some(f));
        assert_eq!(
            engine.hit_test(10.0, 5.0),
            Some(f),
            "under the paragraph's line"
        );
        assert_eq!(engine.hit_test(60.0, 5.0), Some(p));
        let text = engine
            .fragments(200.0)
            .into_iter()
            .find(|fr| matches!(&fr.kind, layout::FragmentKind::Text(t) if t == "beside"))
            .expect("the text is laid out");
        assert_eq!(text.x, 50.0);
        assert_eq!(engine.layout(200.0).height, 50.0);
    }

    #[test]
    fn a_relatively_positioned_box_is_painted_and_hit_where_it_moved_to() {
        // A 50px red box moved 100px right and 10px down: its pixels
        // and the pointer find it there, and where it was is the body.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }
                    .r { position: relative; left: 100px; top: 10px; width: 50px; height: 50px;
                         background-color: #ff0000 }</style>\
             <div class=r id=r></div>",
        );
        let pixels = engine.frame(200, 100);
        assert_eq!(px(&pixels, 200, 110, 20), [0xff, 0x00, 0x00, 0xff]);
        assert_eq!(px(&pixels, 200, 10, 20), [0xff, 0xff, 0xff, 0xff]);
        let r = node_by_id(&engine, "r");
        assert_eq!(engine.hit_test(110.0, 20.0), Some(r));
        assert_ne!(engine.hit_test(10.0, 20.0), Some(r));
        // The page reaches the box where it moved to (css-overflow-3
        // §2.2, as the reference browser has it: a relatively positioned
        // box counts at its offset position), not only where it is in
        // flow.
        assert_eq!(engine.layout(200.0).height, 60.0);
    }

    #[test]
    fn a_fixed_box_stays_put_under_scroll_and_an_absolute_one_scrolls_with_the_page() {
        // A tall page with a red fixed box at the viewport's bottom
        // right and a blue absolutely positioned box 150px down: both
        // are painted and hit where they are; scrolled by 100, the
        // fixed one has not moved and the absolute one is 100px higher,
        // while the page's height counts the absolute one only.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }
                    .tall { height: 1000px }
                    .f { position: fixed; right: 10px; bottom: 10px; width: 30px; height: 30px;
                         background-color: #ff0000 }
                    .a { position: absolute; left: 20px; top: 150px; width: 30px; height: 30px;
                         background-color: #0000ff }</style>\
             <div class=tall></div><div class=f id=f></div><div class=a id=a></div>",
        );
        let pixels = engine.frame(200, 100);
        let (f, a) = (node_by_id(&engine, "f"), node_by_id(&engine, "a"));
        assert_eq!(px(&pixels, 200, 170, 70), [0xff, 0x00, 0x00, 0xff]);
        assert_eq!(engine.hit_test(170.0, 70.0), Some(f));
        assert_ne!(
            engine.hit_test(30.0, 60.0),
            Some(a),
            "the absolute box is below the fold: the tall div is there"
        );
        engine.set_scroll(100.0);
        let pixels = engine.frame(200, 100);
        assert_eq!(
            px(&pixels, 200, 170, 70),
            [0xff, 0x00, 0x00, 0xff],
            "fixed: unmoved"
        );
        assert_eq!(engine.hit_test(170.0, 70.0), Some(f));
        assert_eq!(
            px(&pixels, 200, 30, 60),
            [0x00, 0x00, 0xff, 0xff],
            "absolute: scrolled up"
        );
        assert_eq!(engine.hit_test(30.0, 60.0), Some(a));
        assert_eq!(engine.layout(200.0).height, 1000.0);
    }

    #[test]
    fn a_fixed_box_inside_a_transformed_box_scrolls_with_the_page() {
        // css-transforms-1 §2 (M19): a transformed box is the containing
        // block of the fixed boxes inside it, so the red box, fixed
        // inside a translated wrapper, scrolls away with the page, while
        // the blue one, fixed outside it, stays put; painting and hit
        // testing agree.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }
                    .w { transform: translateX(0px); height: 40px }
                    .tall { height: 1000px }
                    .f { position: fixed; left: 10px; top: 10px; width: 30px; height: 30px;
                         background-color: #ff0000 }
                    .g { position: fixed; left: 100px; top: 10px; width: 30px; height: 30px;
                         background-color: #0000ff }</style>\
             <div class=w><div class=f id=f></div></div><div class=tall></div>\
             <div class=g id=g></div>",
        );
        let pixels = engine.frame(200, 100);
        let (f, g) = (node_by_id(&engine, "f"), node_by_id(&engine, "g"));
        assert_eq!(px(&pixels, 200, 20, 20), [0xff, 0x00, 0x00, 0xff]);
        assert_eq!(px(&pixels, 200, 110, 20), [0x00, 0x00, 0xff, 0xff]);
        assert_eq!(engine.hit_test(20.0, 20.0), Some(f));
        assert_eq!(engine.hit_test(110.0, 20.0), Some(g));
        engine.set_scroll(100.0);
        let pixels = engine.frame(200, 100);
        assert_ne!(
            px(&pixels, 200, 20, 20),
            [0xff, 0x00, 0x00, 0xff],
            "scrolled away with its wrapper"
        );
        assert_ne!(engine.hit_test(20.0, 20.0), Some(f));
        assert_eq!(
            px(&pixels, 200, 110, 20),
            [0x00, 0x00, 0xff, 0xff],
            "fixed to the viewport"
        );
        assert_eq!(engine.hit_test(110.0, 20.0), Some(g));
        assert_eq!(engine.layout(200.0).height, 1040.0);
    }

    #[test]
    fn transformed_and_faded_boxes_are_drawn_where_their_matrices_put_them() {
        // M19: a translated box's pixels are where the translation puts
        // them and not where layout did; a scaled box covers its scaled
        // area; a faded box blends with what is under it; a turned box
        // keeps its centre and bares its old corner; and a changed
        // transform is drawn again in part.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }
                    div { width: 20px; height: 20px; background-color: #ff0000 }
                    .t { transform: translate(100px, 10px) }
                    .s { transform: scale(2); transform-origin: 0 0; margin-top: 10px }
                    .o { opacity: 0.5; margin-top: 30px }
                    .r { transform: rotate(45deg); margin-top: 10px }</style>\
             <div class=t id=t></div><div class=s id=s></div><div class=o id=o></div>\
             <div class=r id=r></div>",
        );
        let pixels = engine.frame(200, 160);
        let at = |x, y| px(&pixels, 200, x, y);
        let red = [0xff, 0x00, 0x00, 0xff];
        let white = [0xff, 0xff, 0xff, 0xff];
        assert_eq!(at(110, 20), red, "translated");
        assert_eq!(at(10, 10), white, "not where layout put it");
        assert_eq!(at(30, 60), red, "scaled to 40 by 40 from (0, 30)");
        assert_eq!(at(45, 60), white);
        let faded = at(10, 90);
        assert!(
            faded[0] == 255 && (faded[1] as i32 - 128).abs() <= 1 && faded[1] == faded[2],
            "half red over white: {faded:?}"
        );
        // The turned box, 20 by 20 at (0, 110), about its centre (10,
        // 120): the centre red, the old corner bare, the point 12px
        // below the centre, inside the turned box, red.
        assert_eq!(at(10, 120), red);
        assert_eq!(at(1, 111), white);
        assert_eq!(at(10, 132), red);
        // A new transform redraws the box's old and new places, not the
        // frame.
        let t = node_by_id(&engine, "t");
        engine.mutate(|document| {
            document
                .set_attribute(t, "style", "transform: translate(50px, 10px)")
                .expect("an element");
        });
        let pixels = engine.frame(200, 160);
        let at = |x, y| px(&pixels, 200, x, y);
        assert_eq!(at(60, 20), red);
        assert_eq!(at(110, 20), white);
        match engine.redrawn() {
            render::Redrawn::Rects(rects) => {
                let covers = |x: u32, y: u32| {
                    rects
                        .iter()
                        .any(|r| r.x0 <= x && x < r.x1 && r.y0 <= y && y < r.y1)
                };
                assert!(covers(60, 20) && covers(110, 20), "{rects:?}");
                assert!(!covers(10, 120), "the turned box is left alone: {rects:?}");
            }
            render::Redrawn::Whole => panic!("drawn whole"),
        }
    }

    #[test]
    fn transformed_boxes_are_hit_where_they_are_drawn() {
        // M19: the pointer finds a box where its matrix put it, through
        // the inverse of the matrix, not where layout did; a scaled box
        // across its scaled area; a turned box inside its turned edges;
        // a box whose matrix is singular never; a clip inside a group in
        // the group's coordinates; a box translated into view from far
        // below, drawn and hit; and the wheel over a scroll container
        // moved by a transform scrolls it.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }
                    div { width: 20px; height: 20px; background-color: #ff0000 }
                    .t { transform: translate(100px, 10px) }
                    .s { transform: scale(2); transform-origin: 0 0; margin-top: 10px }
                    .r { transform: rotate(45deg); margin-top: 10px }
                    .z { transform: scale(0); margin-top: 10px }
                    .c { transform: translate(60px, 0); overflow: hidden; margin-top: 10px }
                    .c div { width: 50px; height: 10px; background-color: #0000ff }
                    .sc { transform: translateX(100px); overflow: auto; margin-top: 10px }
                    .sc div { height: 100px }
                    .far { transform: translateY(-1000px); margin-top: 1000px }</style>\
             <div class=t id=t></div><div class=s id=s></div><div class=r id=r></div>\
             <div class=z id=z></div><div class=c id=c><div id=in></div></div>\
             <div class=sc id=sc><div></div></div><div class=far id=far></div>",
        );
        let pixels = engine.frame(200, 200);
        let node = |id: &str| node_by_id(&engine, id);
        let (t, s, r, z, c, inner, sc, far) = (
            node("t"),
            node("s"),
            node("r"),
            node("z"),
            node("c"),
            node("in"),
            node("sc"),
            node("far"),
        );
        assert_eq!(engine.hit_test(110.0, 20.0), Some(t));
        assert_ne!(engine.hit_test(10.0, 10.0), Some(t));
        assert_eq!(engine.hit_test(35.0, 65.0), Some(s), "scaled past its box");
        assert_eq!(
            engine.hit_test(10.0, 70.0),
            Some(r),
            "the centre of the turned box"
        );
        assert_ne!(
            engine.hit_test(1.0, 61.0),
            Some(r),
            "its old corner is bare"
        );
        assert_ne!(
            engine.hit_test(10.0, 100.0),
            Some(z),
            "singular: not displayed"
        );
        assert_eq!(engine.hit_test(70.0, 125.0), Some(inner));
        assert_ne!(
            engine.hit_test(85.0, 125.0),
            Some(inner),
            "clipped by its moved box"
        );
        assert_ne!(engine.hit_test(85.0, 125.0), Some(c));
        assert_eq!(px(&pixels, 200, 70, 125), [0x00, 0x00, 0xff, 0xff]);
        assert_eq!(px(&pixels, 200, 85, 125), [0xff, 0xff, 0xff, 0xff]);
        assert_eq!(
            px(&pixels, 200, 10, 180),
            [0xff, 0x00, 0x00, 0xff],
            "moved into view"
        );
        assert_eq!(engine.hit_test(10.0, 180.0), Some(far));
        // The scroll container, laid out at (0, 150) and drawn at
        // (100, 150): the wheel over it there scrolls it; over where it
        // was, the page.
        engine.wheel(110.0, 160.0, 40.0);
        assert_eq!(engine.element_scroll_target(sc), 40.0);
        assert_eq!(engine.scroll_target(), 0.0);
        engine.wheel(10.0, 160.0, 40.0);
        assert_eq!(engine.element_scroll_target(sc), 40.0);
        assert!(engine.scroll_target() > 0.0);
    }

    #[test]
    fn a_sticky_box_keeps_its_inset_from_its_scrollport_within_its_container() {
        // A 300px container holding a red 20px sticky header with
        // `top: 0`, then a tall div. Unscrolled, the header is at the
        // top; scrolled by 100 it is held at the viewport's top;
        // scrolled by 290 the container's bottom is at 10 and holds the
        // header 10px up, half gone. Painting and hit testing agree at
        // every step.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }
                    .c { height: 300px }
                    .s { position: sticky; top: 0; height: 20px; background-color: #ff0000 }
                    .tall { height: 1000px }</style>\
             <div class=c><div class=s id=s></div></div><div class=tall></div>",
        );
        let red = [0xff, 0x00, 0x00, 0xff];
        let white = [0xff, 0xff, 0xff, 0xff];
        let pixels = engine.frame(200, 100);
        let s = node_by_id(&engine, "s");
        assert_eq!(px(&pixels, 200, 10, 10), red);
        assert_eq!(engine.hit_test(10.0, 10.0), Some(s));
        engine.set_scroll(100.0);
        let pixels = engine.frame(200, 100);
        assert_eq!(px(&pixels, 200, 10, 10), red, "held at the top");
        assert_eq!(px(&pixels, 200, 10, 30), white);
        assert_eq!(engine.hit_test(10.0, 10.0), Some(s));
        engine.set_scroll(290.0);
        let pixels = engine.frame(200, 100);
        assert_eq!(
            px(&pixels, 200, 10, 5),
            red,
            "held by the container's bottom"
        );
        assert_eq!(px(&pixels, 200, 10, 15), white);
        assert_eq!(engine.hit_test(10.0, 5.0), Some(s));
        assert_ne!(engine.hit_test(10.0, 15.0), Some(s));
        // Inside a scroll container, the container's padding box is the
        // scrollport: the header stays at its top as its content
        // scrolls, and is hit there, being positioned and so painted
        // over the tall div that scrolled up under it (CSS2 Appendix E).
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }
                    .o { overflow: auto; height: 60px }
                    .s { position: sticky; top: 0; height: 20px; background-color: #ff0000 }
                    .t { height: 200px }</style>\
             <div class=o id=o><div class=s id=s></div><div class=t></div></div>",
        );
        engine.frame(200, 100);
        let (o, s) = (node_by_id(&engine, "o"), node_by_id(&engine, "s"));
        engine.set_element_scroll(o, 30.0);
        let pixels = engine.frame(200, 100);
        assert_eq!(px(&pixels, 200, 10, 10), red, "held at the container's top");
        assert_eq!(engine.hit_test(10.0, 10.0), Some(s));
        assert_eq!(px(&pixels, 200, 10, 40), white);
        assert_ne!(engine.hit_test(10.0, 40.0), Some(s));
    }

    #[test]
    fn z_index_decides_what_is_painted_and_hit_where_positioned_boxes_overlap() {
        // Three absolute boxes at the top left and an in-flow black
        // strip (CSS2 Appendix E, M11): a red 50px one with z-index 2
        // over a blue 100px one with 1, though the blue comes later in
        // tree order; a green 20px strip with -1 under the in-flow
        // content but over the canvas. Pixels and the pointer agree.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }
                    .a, .b, .n { position: absolute; top: 0; left: 0 }
                    .a { width: 50px; height: 50px; background-color: #ff0000; z-index: 2 }
                    .b { width: 100px; height: 100px; background-color: #0000ff; z-index: 1 }
                    .n { width: 200px; height: 20px; background-color: #00ff00; z-index: -1 }
                    .t { height: 10px; background-color: #000000 }</style>\
             <div class=a id=a></div><div class=b id=b></div><div class=n id=n></div>\
             <div class=t id=t></div>",
        );
        let pixels = engine.frame(200, 100);
        let (a, b) = (node_by_id(&engine, "a"), node_by_id(&engine, "b"));
        let (n, t) = (node_by_id(&engine, "n"), node_by_id(&engine, "t"));
        assert_eq!(
            px(&pixels, 200, 10, 10),
            [0xff, 0x00, 0x00, 0xff],
            "red over blue"
        );
        assert_eq!(engine.hit_test(10.0, 10.0), Some(a));
        assert_eq!(px(&pixels, 200, 75, 75), [0x00, 0x00, 0xff, 0xff]);
        assert_eq!(engine.hit_test(75.0, 75.0), Some(b));
        assert_eq!(
            px(&pixels, 200, 150, 5),
            [0x00, 0x00, 0x00, 0xff],
            "in flow over a negative z-index"
        );
        assert_eq!(engine.hit_test(150.0, 5.0), Some(t));
        assert_eq!(px(&pixels, 200, 150, 15), [0x00, 0xff, 0x00, 0xff]);
        assert_eq!(engine.hit_test(150.0, 15.0), Some(n));
    }

    #[test]
    fn a_table_paints_its_parts_under_its_cells_and_is_hit_through_its_grid() {
        // A fixed 100px table, 10px spacing, blue; a row group of two
        // red rows, each with two cells, the second green. The columns
        // are (100 - 3 * 10) / 2 = 35px; the rows 20px, at 10 and 40.
        // The row's red shows under its cells alone, never in the
        // spacing (CSS2 §17.6.1); the pointer finds the cell over a
        // cell, the row between two cells of it, the group between two
        // rows, and the table in its own spacing (M12).
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }
                    table { table-layout: fixed; width: 100px; border-spacing: 10px;
                            background-color: #0000ff }
                    td { padding: 0; height: 20px }
                    tr { background-color: #ff0000 }
                    .g { background-color: #00ff00 }</style>\
             <table id=t><tbody id=b><tr id=r><td id=a></td><td class=g id=c></td></tr>\
             <tr><td></td><td class=g></td></tr></tbody></table>",
        );
        let pixels = engine.frame(200, 100);
        let id = |name: &str| node_by_id(&engine, name);
        let (t, b, r, a, c) = (id("t"), id("b"), id("r"), id("a"), id("c"));
        let blue = [0x00, 0x00, 0xff, 0xff];
        assert_eq!(
            px(&pixels, 200, 20, 20),
            [0xff, 0x00, 0x00, 0xff],
            "the row under a cell"
        );
        assert_eq!(engine.hit_test(20.0, 20.0), Some(a));
        assert_eq!(
            px(&pixels, 200, 70, 20),
            [0x00, 0xff, 0x00, 0xff],
            "the green cell"
        );
        assert_eq!(engine.hit_test(70.0, 20.0), Some(c));
        assert_eq!(
            px(&pixels, 200, 50, 20),
            blue,
            "the spacing between two cells"
        );
        assert_eq!(engine.hit_test(50.0, 20.0), Some(r));
        assert_eq!(
            px(&pixels, 200, 20, 35),
            blue,
            "the spacing between two rows"
        );
        assert_eq!(engine.hit_test(20.0, 35.0), Some(b));
        assert_eq!(px(&pixels, 200, 5, 5), blue, "the table's own spacing");
        assert_eq!(engine.hit_test(5.0, 5.0), Some(t));
        assert_eq!(engine.hit_test(95.0, 65.0), Some(t));
        assert_eq!(
            px(&pixels, 200, 20, 75),
            [0xff, 0xff, 0xff, 0xff],
            "below the table"
        );
        assert_ne!(engine.hit_test(20.0, 75.0), Some(t));
    }

    #[test]
    fn collapsed_borders_paint_on_the_grid_lines_the_winner_on_top_at_joints() {
        // A collapsed 2-column table at (10, 10): the table's 4px blue
        // border on its outer lines, whole inside its border box, whose
        // own border is their half, 2px; the cells' 2px red one between
        // them at x = 60 (12 + 48), 59 to 61. At the joint with the top
        // line the blue border, which wins, is painted over the red; a
        // point on a line is the table's, one inside a cell the cell's.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 10px }
                    table { border-collapse: collapse; border: 4px solid #0000ff;
                            table-layout: fixed; width: 100px }
                    td { padding: 0; height: 20px; border: 2px solid #ff0000 }</style>\
             <table id=t><tr><td id=a></td><td></td></tr></table>",
        );
        let pixels = engine.frame(200, 100);
        let (t, a) = (node_by_id(&engine, "t"), node_by_id(&engine, "a"));
        let (blue, red) = ([0x00, 0x00, 0xff, 0xff], [0xff, 0x00, 0x00, 0xff]);
        let white = [0xff, 0xff, 0xff, 0xff];
        assert_eq!(px(&pixels, 200, 9, 20), white, "nothing in its margin");
        assert_eq!(
            px(&pixels, 200, 10, 20),
            blue,
            "the outer half of the left line"
        );
        assert_eq!(px(&pixels, 200, 13, 20), blue, "its inner half");
        assert_eq!(px(&pixels, 200, 14, 20), white, "the cell");
        assert_eq!(px(&pixels, 200, 59, 20), red);
        assert_eq!(px(&pixels, 200, 60, 20), red);
        assert_eq!(
            px(&pixels, 200, 60, 11),
            blue,
            "the joint, the top line over the inner one"
        );
        assert_eq!(px(&pixels, 200, 30, 20), white);
        assert_eq!(engine.hit_test(60.0, 20.0), Some(t));
        assert_eq!(engine.hit_test(30.0, 20.0), Some(a));
    }

    #[test]
    fn a_drop_cap_in_a_link_is_the_link_to_the_pointer() {
        // The p's first letter is in the link's text: its box is the
        // link's, and a click on it follows the link.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 } p { margin: 0 }
                    p::first-letter { float: left; font-size: 60px; color: #f00 }</style>\
             <p><a href=\"https://example.test/t\" id=a>Once</a> upon a time</p>",
        );
        engine.frame(200, 100);
        let a = node_by_id(&engine, "a");
        // Low in the 60px letter, below the first 16px line.
        assert_eq!(engine.hit_test(5.0, 40.0), Some(a));
        assert_eq!(engine.link_at(5.0, 40.0).map(|link| link.node()), Some(a));
    }

    #[test]
    fn a_generated_box_is_its_element_to_the_pointer_but_never_its_scroller() {
        // The link's `::before` is a 40px block above its text: the
        // pointer over it is over the link, and a click follows it.
        // #e's `::before` is absolutely positioned outside #e, at
        // document y 300, and clips: the wheel over it scrolls the
        // viewport, not #e, whose element it names.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 } p { margin: 0 }
                    a::before { display: block; height: 40px; content: '' }
                    #e { overflow: auto; height: 100px }
                    #e::before { position: absolute; top: 300px; left: 0; width: 100px;
                                 height: 50px; overflow: auto; content: 'x' }
                    .tall { height: 1000px }</style>\
             <p><a href=\"https://example.test/t\" id=a>b</a></p>\
             <div id=e><div class=tall></div></div><div class=tall></div>",
        );
        engine.frame(200, 100);
        let (a, e) = (node_by_id(&engine, "a"), node_by_id(&engine, "e"));
        assert_eq!(engine.hit_test(5.0, 20.0), Some(a));
        assert_eq!(engine.link_at(5.0, 20.0).map(|link| link.node()), Some(a));
        engine.set_scroll(260.0);
        // Over its text, and over the box below it.
        for y in [45.0, 80.0] {
            assert_eq!(engine.hit_test(5.0, y), Some(e));
            engine.wheel(5.0, y, 30.0);
            assert_eq!(engine.element_scroll_target(e), 0.0, "{y}");
        }
        assert_eq!(engine.scroll_target(), 320.0);
        // Over #e itself, the wheel scrolls #e.
        engine.set_scroll(0.0);
        engine.wheel(5.0, 80.0, 30.0);
        assert_eq!(engine.element_scroll_target(e), 30.0);
    }

    #[test]
    fn an_inline_element_paints_its_background_and_is_hit_over_its_padding() {
        // <p><a href=…>b</a></p>, the link with 10px of horizontal
        // padding and a red background (M11.6): its padding is red, is
        // the link for the pointer and for a click, and ends with its
        // content area, the rest of the line below being the
        // paragraph's. The link's font is smaller than the paragraph's,
        // whose strut makes the line 19px tall, the baseline at 15: the
        // link's content area runs from 6 to 17.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 } p { margin: 0 }
                    a { padding: 0 10px; background-color: #ff0000; font-size: 10px }</style>\
             <p id=p><a href=\"https://example.test/t\" id=a>b</a></p>",
        );
        let pixels = engine.frame(200, 100);
        let (p, a) = (node_by_id(&engine, "p"), node_by_id(&engine, "a"));
        assert_eq!(px(&pixels, 200, 5, 8), [0xff, 0x00, 0x00, 0xff]);
        assert_eq!(engine.hit_test(5.0, 8.0), Some(a));
        assert_eq!(engine.link_at(5.0, 8.0).map(|link| link.node()), Some(a));
        assert_eq!(
            px(&pixels, 200, 5, 18),
            [0xff, 0xff, 0xff, 0xff],
            "below the content area"
        );
        assert_eq!(engine.hit_test(5.0, 18.0), Some(p));
        assert_eq!(engine.hit_test(12.0, 8.0), Some(a), "over its text");
    }

    #[test]
    fn a_clipping_box_is_hit_on_its_border_but_scrolls_only_from_its_padding_box() {
        // A 100px scroll container with a 10px border and 300px of
        // content: the pointer on the border is over the box, since the
        // border is the box's own (what its step paints), but the wheel
        // there does not scroll it, the border lying outside what clips
        // and scrolls (CSS2 §11.1.1); inside the padding box, the
        // content is hit and the wheel scrolls the box.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }
                    .s { overflow: auto; height: 100px; width: 100px; margin: 0;
                         border: 10px solid #000000 }
                    .t { height: 300px; margin: 0 }</style>\
             <div class=s id=s><p class=t id=t></p></div>",
        );
        engine.frame(200, 200);
        let s = node_by_id(&engine, "s");
        let t = node_by_id(&engine, "t");
        assert_eq!(engine.hit_test(5.0, 50.0), Some(s), "on the left border");
        assert_eq!(engine.hit_test(50.0, 5.0), Some(s), "on the top border");
        assert_eq!(engine.hit_test(50.0, 50.0), Some(t), "inside: the content");
        assert_eq!(engine.hit_test(115.0, 50.0), Some(s), "on the right border");
        engine.wheel(5.0, 50.0, 40.0);
        assert_eq!(
            engine.element_scroll_target(s),
            0.0,
            "the border scrolls nothing"
        );
        assert_eq!(
            engine.scroll_target(),
            0.0,
            "and the page has nothing to scroll"
        );
        engine.wheel(50.0, 50.0, 40.0);
        assert_eq!(engine.element_scroll_target(s), 40.0);
    }

    #[test]
    fn link_at_walks_up_to_the_nearest_anchor() {
        let mut engine = test_engine();
        engine.load_document(
            "<p style=\"margin: 0; font-size: 20px\">\
               <a href=\"https://example.test/x\"><b>link</b></a> plain</p>",
        );
        engine.frame(400, 100);
        let frags = engine.fragments(400.0);
        let word = |w: &str| {
            frags
                .iter()
                .find(|f| matches!(&f.kind, layout::FragmentKind::Text(t) if t == w))
                .expect("the word is laid out")
                .clone()
        };
        let link = word("link");
        let plain = word("plain");
        assert_eq!(
            engine
                .link_at(link.x + 1.0, link.top() + 1.0)
                .map(|link| link.url().as_str().to_owned())
                .as_deref(),
            Some("https://example.test/x")
        );
        assert_eq!(engine.link_at(plain.x + 1.0, plain.top() + 1.0), None);
        assert_eq!(engine.link_at(1.0, 90.0), None);
    }

    #[test]
    fn the_users_preferences_restyle_the_pages_that_ask_for_them() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>@media (prefers-color-scheme: dark) { p { color: #fff } }\
             @media (prefers-reduced-motion) { p { background-color: #00f } }</style><p>x</p>",
        );
        let style = |engine: &mut Engine| {
            let p = first(engine, "p");
            let style = engine.styles().get(p).expect("styled");
            (style.color, style.background_color)
        };
        engine.frame(200, 100);
        let light = style(&mut engine);
        assert_eq!(light.0, style::computed::Color::rgb8(0, 0, 0));
        assert_eq!(engine.preferences(), Preferences::default());
        assert!(!engine.needs_frame());

        let dark = Preferences {
            color_scheme: ColorScheme::Dark,
            ..Preferences::default()
        };
        engine.set_preferences(dark);
        assert_eq!(engine.preferences(), dark);
        assert!(engine.needs_frame());
        engine.frame(200, 100);
        assert_eq!(engine.style_runs(), 2);
        assert_eq!(
            style(&mut engine).0,
            style::computed::Color::rgb8(255, 255, 255)
        );
        engine.set_preferences(dark);
        assert!(!engine.needs_frame(), "the same preferences");

        engine.set_preferences(Preferences {
            reduced_motion: true,
            ..dark
        });
        engine.frame(200, 100);
        assert_eq!(
            style(&mut engine).1,
            style::computed::Color::rgb8(0, 0, 255)
        );

        // A page that asks nothing of them is left alone.
        engine.load_document("<p>x</p>");
        engine.frame(200, 100);
        let runs = engine.style_runs();
        engine.set_preferences(Preferences::default());
        assert!(!engine.needs_frame());
        engine.frame(200, 100);
        assert_eq!(engine.style_runs(), runs);
    }

    #[test]
    fn an_emulated_medium_restyles_the_page_for_it() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>@media print { p { color: #f00 } } @media screen { p { color: #00f } }\
             </style><p>x</p>",
        );
        let color = |engine: &mut Engine| {
            let p = first(engine, "p");
            engine.styles().get(p).expect("styled").color
        };
        engine.frame(200, 100);
        assert_eq!(engine.media_type(), MediaType::Screen);
        assert_eq!(color(&mut engine), style::computed::Color::rgb8(0, 0, 255));
        engine.set_media_type(MediaType::Print);
        assert!(engine.needs_frame());
        engine.frame(200, 100);
        assert_eq!(color(&mut engine), style::computed::Color::rgb8(255, 0, 0));
        engine.set_media_type(MediaType::Print);
        assert!(!engine.needs_frame(), "the same medium");
        // Back to the screen.
        engine.set_media_type(MediaType::Screen);
        assert!(engine.needs_frame());
        engine.frame(200, 100);
        assert_eq!(color(&mut engine), style::computed::Color::rgb8(0, 0, 255));
    }

    #[test]
    fn a_resize_across_a_breakpoint_recascades_and_one_within_does_not() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>p { color: #000 } @media (min-width: 500px) { p { color: #f00 } }\
             @media (orientation: portrait) { p { background-color: #00f } }</style><p>x</p>",
        );
        let color = |engine: &mut Engine| {
            let p = first(engine, "p");
            engine.styles().get(p).expect("styled").color
        };
        engine.frame(400, 300);
        assert_eq!(color(&mut engine), style::computed::Color::rgb8(0, 0, 0));
        assert_eq!(engine.style_runs(), 1);
        engine.frame(450, 300);
        assert_eq!(engine.style_runs(), 1, "no list answers otherwise");
        engine.frame(600, 300);
        assert_eq!(engine.style_runs(), 2, "past the breakpoint");
        assert_eq!(color(&mut engine), style::computed::Color::rgb8(255, 0, 0));
        engine.frame(600, 700);
        assert_eq!(engine.style_runs(), 3, "turned to portrait");
        engine.frame(700, 900);
        assert_eq!(engine.style_runs(), 3);
        assert!(!engine.needs_frame());
    }

    #[test]
    fn viewport_units_recascade_only_when_the_document_uses_them() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }</style><div style=\"height: 50vh; width: 25vw\"></div>",
        );
        // The div's size in the engine's own cached layout at the
        // frame's width (`boxes_of` would lay out afresh at 800px).
        fn div_size(engine: &mut Engine) -> (f32, f32) {
            let div = first(engine, "div");
            let f = engine
                .layout(200.0)
                .iter()
                .find(|f| f.node == div)
                .expect("the div's fragment");
            (f.w, f.h)
        }
        engine.frame(200, 100);
        assert_eq!(div_size(&mut engine), (50.0, 50.0));
        assert_eq!(engine.style_runs(), 1);
        engine.frame(200, 200);
        assert_eq!(engine.style_runs(), 2, "the height changed and vh was used");
        assert_eq!(engine.layout_runs(), 2, "and the layout with it");
        assert_eq!(div_size(&mut engine), (50.0, 100.0));
        engine.frame(200, 200);
        assert_eq!(
            engine.style_runs(),
            2,
            "the same viewport: nothing recomputed"
        );

        engine.load_document("<style>body { margin: 0 }</style><div style=\"height: 50px\"></div>");
        engine.frame(200, 100);
        engine.frame(200, 300);
        assert_eq!(
            engine.style_runs(),
            3,
            "no viewport unit: a taller viewport recascades nothing"
        );
    }

    #[test]
    fn a_root_percentage_height_follows_the_viewport_and_only_then_relays_out() {
        let mut engine = test_engine();
        engine.load_document("<style>html, body { height: 100%; margin: 0 }</style><p>x</p>");
        engine.frame(200, 100);
        assert_eq!(boxes_of(&engine, "html")[0].h, 100.0);
        assert_eq!(engine.layout_runs(), 1);
        engine.frame(200, 300);
        assert_eq!(
            engine.layout_runs(),
            2,
            "the height changed and the root depends on it"
        );
        assert_eq!(boxes_of(&engine, "body")[0].h, 300.0);
        engine.load_document("<style>body { margin: 0 }</style><p>x</p>");
        engine.frame(200, 100);
        engine.frame(200, 300);
        assert_eq!(
            engine.layout_runs(),
            3,
            "no percentage height: a taller viewport relays out nothing"
        );
    }

    #[test]
    fn rem_ex_and_ch_resolve_through_the_engine_fonts() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>html { font-size: 20px } body { margin: 0 } \
                    p { margin: 0; padding: 1rem 2ch 1ex 0; font-size: 16px }</style><p>x</p>",
        );
        engine.frame(400, 100);
        let p = first(&engine, "p");
        let padding = engine.styles().get(p).expect("styled").padding.clone();
        let fonts = &engine.layout_fonts;
        use style::computed::LengthPercentage::Px;
        assert_eq!(padding[0], Px(20.0), "1rem of the root's 20px");
        assert_eq!(padding[1], Px(2.0 * fonts.zero_advance(16.0)));
        assert_eq!(padding[2], Px(fonts.x_height(16.0)));
    }

    #[test]
    fn the_scrollbar_appears_only_when_the_document_is_taller_than_the_viewport() {
        let mut engine = test_engine();
        engine.load_document("<p>short</p>");
        engine.frame(200, 100);
        assert_eq!(engine.scrollbar(), None);
        engine.load_document(TALL);
        engine.frame(200, 100);
        let bar = engine.scrollbar().expect("a scrollbar");
        assert_eq!(
            (bar.x, bar.w, bar.h),
            (200.0 - SCROLLBAR_WIDTH, SCROLLBAR_WIDTH, 100.0)
        );
        assert_eq!(bar.thumb_y, 0.0);
        assert!(bar.thumb_h >= paint::SCROLLBAR_MIN_THUMB && bar.thumb_h < 100.0);
        assert!(bar.contains(195.0, 50.0) && !bar.contains(100.0, 50.0));
        assert!(bar.thumb_contains(195.0, 1.0) && !bar.thumb_contains(195.0, 99.0));
        engine.scroll_to_fraction(1.0);
        let bar = engine.scrollbar().expect("a scrollbar");
        assert_eq!(
            bar.thumb_y + bar.thumb_h,
            100.0,
            "the thumb reaches the bottom"
        );
        // The frame draws it: the track column differs from the background.
        let frame = engine.frame(200, 100);
        assert_eq!(px(&frame, 200, 194, 50), [0x31, 0x32, 0x44, 0xff]);
    }

    #[test]
    fn a_mutation_is_seen_by_the_next_frame_and_one_that_changes_nothing_costs_nothing() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 } p { margin: 0; height: 10px; background-color: #ff0000 }</style>\
             <p id=p></p>\
             <div id=s style=\"overflow: auto; height: 20px\"><div style=\"height: 100px\"></div></div>",
        );
        assert!(engine.needs_frame(), "a new document");
        engine.frame(200, 100);
        assert!(!engine.needs_frame());
        let runs = (engine.style_runs(), engine.layout_runs());
        let p = node_by_id(&engine, "p");

        // Setting the value already there records nothing: no frame is
        // wanted and none recomputes anything.
        let same = engine.mutate(|document| document.set_attribute(p, "id", "p"));
        assert_eq!(same, Ok(()), "mutate returns what the closure does");
        assert!(!engine.needs_frame());
        engine.frame(200, 100);
        assert_eq!((engine.style_runs(), engine.layout_runs()), runs);

        // A real change is seen: the next frame restyles the one element
        // it reaches (M13.1), lays out again, and shows it.
        engine
            .mutate(|document| document.set_attribute(p, "style", "background-color: #0000ff"))
            .expect("p is an element");
        assert!(engine.needs_frame());
        let frame = engine.frame(200, 100);
        assert!(!engine.needs_frame());
        assert_eq!(
            (engine.style_runs(), engine.layout_runs()),
            (runs.0, runs.1 + 1)
        );
        assert_eq!(engine.restyled_elements(), 1);
        assert_eq!(px(&frame, 200, 5, 5), [0x00, 0x00, 0xff, 0xff]);

        // A stylesheet's text changed: the stylesheets are parsed again.
        let style = first(&engine, "style");
        let text = engine
            .document()
            .first_child(style)
            .expect("the sheet's text");
        engine
            .mutate(|document| document.set_text(text, "p { height: 30px; margin: 0 }"))
            .expect("a text node");
        engine.frame(200, 100);
        assert_eq!(boxes_of(&engine, "p")[0].h, 30.0);

        // A freed scroll container's offset is dropped with it; the
        // document's own offset stays, clamped to the new layout.
        let s = node_by_id(&engine, "s");
        engine.set_element_scroll(s, 30.0);
        assert_eq!(engine.element_scroll(s), 30.0);
        engine.mutate(|document| {
            let body = document.parent(s).expect("attached");
            document.remove_child(body, s).expect("a child");
            document.free(s).expect("detached");
        });
        assert!(engine.shown.element_scroll.is_empty());
        assert_eq!(engine.element_scroll(s), 0.0);
        engine.frame(200, 100);

        // A document installed whole has its journal emptied: the engine
        // has seen nothing of it, and it all counts as new.
        let mut document = Document::from_node(&html::parse_document("<p>a</p>").expect("parses"));
        let body = document
            .document_element()
            .and_then(|html| document.last_child(html));
        let extra = document.create_element("p").expect("an element");
        document
            .append_child(body.expect("a body"), extra)
            .expect("appended");
        assert!(document.has_changes());
        engine.set_document(document);
        assert!(!engine.document().has_changes());
        assert!(engine.needs_frame());
    }

    #[test]
    fn appending_a_paragraph_to_a_long_page_restyles_it_alone() {
        // No selector depends on siblings or children here, the UA
        // sheet's included: the new paragraph is the one element
        // restyled, and nothing is cascaded whole (M13.1).
        let mut engine = test_engine();
        let mut page = String::from("<style>p { margin: 0 } .x { color: #00f }</style>");
        for i in 0..100 {
            page.push_str(&format!("<p>{i}</p>"));
        }
        engine.load_document(&page);
        engine.frame(200, 100);
        let (runs, restyled) = (engine.style_runs(), engine.restyled_elements());
        let body = first(&engine, "body");
        engine.mutate(|document| {
            let p = document.create_element("p").expect("an element");
            let text = document.create_text("new").expect("a text node");
            document.append_child(p, text).expect("appended");
            document.append_child(body, p).expect("appended");
        });
        engine.frame(200, 100);
        assert_eq!(engine.style_runs(), runs);
        assert_eq!(engine.restyled_elements(), restyled + 1);
        // A class some selector names, on one paragraph: it alone.
        let p = first(&engine, "p");
        engine
            .mutate(|document| document.set_attribute(p, "class", "x"))
            .expect("an element");
        engine.frame(200, 100);
        assert_eq!(engine.restyled_elements(), restyled + 2);
        // A new stylesheet: the whole document, cascaded afresh.
        let style = first(&engine, "style");
        let text = engine.document().first_child(style).expect("the sheet");
        engine
            .mutate(|document| document.set_text(text, "p { margin: 1px }"))
            .expect("a text node");
        engine.frame(200, 100);
        assert_eq!(engine.style_runs(), runs + 1);
    }

    #[test]
    fn appending_a_paragraph_to_a_long_page_lays_out_it_and_its_ancestors_alone() {
        // After the first frame, appending a paragraph lays out the new
        // paragraph and the boxes around it that changed, the body and
        // the root, and takes every other from the layout cache (M13.2).
        let mut engine = test_engine();
        let mut page = String::from("<style>p { margin: 0 }</style>");
        for i in 0..100 {
            page.push_str(&format!("<p>{i}</p>"));
        }
        engine.load_document(&page);
        engine.frame(200, 100);
        // The root, the body and the paragraphs; the head has no box.
        assert_eq!((engine.boxes_laid_out(), engine.boxes_reused()), (102, 0));
        let body = first(&engine, "body");
        engine.mutate(|document| {
            let p = document.create_element("p").expect("an element");
            let text = document.create_text("new").expect("a text node");
            document.append_child(p, text).expect("appended");
            document.append_child(body, p).expect("appended");
        });
        engine.frame(200, 100);
        assert_eq!((engine.boxes_laid_out(), engine.boxes_reused()), (105, 100));
        // A text change: its paragraph and the two above it.
        let p = first(&engine, "p");
        let text = engine.document().first_child(p).expect("a text node");
        engine
            .mutate(|document| document.set_text(text, "changed"))
            .expect("a text node");
        engine.frame(200, 100);
        assert_eq!((engine.boxes_laid_out(), engine.boxes_reused()), (108, 200));
        // A new stylesheet cascades the document whole, and lays it out
        // whole: the cache is emptied with the cascade.
        let style = first(&engine, "style");
        let sheet = engine.document().first_child(style).expect("the sheet");
        engine
            .mutate(|document| document.set_text(sheet, "p { margin: 1px }"))
            .expect("a text node");
        engine.frame(200, 100);
        assert_eq!((engine.boxes_laid_out(), engine.boxes_reused()), (211, 200));
        // A new width: every paragraph's containing block is new.
        engine.frame(300, 100);
        assert_eq!((engine.boxes_laid_out(), engine.boxes_reused()), (314, 200));
    }

    #[test]
    fn a_frame_redraws_where_its_list_changed_and_nowhere_else() {
        // After the first frame, the raster draws again only where the
        // display list differs from the last frame's (M13.3), and the
        // frame is still what a fresh engine draws whole.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 } p { margin: 0 } .x { color: #00f }</style>\
             <p>one</p><p>two</p><p>three</p>",
        );
        engine.frame(200, 100);
        assert_eq!(engine.redrawn(), &render::Redrawn::Whole);
        engine.frame(200, 100);
        assert_eq!(engine.redrawn(), &render::Redrawn::Rects(Vec::new()));
        // The second paragraph's text in another colour: around its
        // line alone.
        let second = boxes_of(&engine, "p")[1].clone();
        engine
            .mutate(|document| document.set_attribute(second.node, "class", "x"))
            .expect("an element");
        engine.frame(200, 100);
        let render::Redrawn::Rects(rects) = engine.redrawn().clone() else {
            panic!("drawn whole: {:?}", engine.redrawn());
        };
        assert!(!rects.is_empty());
        for rect in &rects {
            assert!(
                rect.y0 as f32 >= second.y - 2.0 && rect.y1 as f32 <= second.y + second.h + 2.0,
                "{rect:?} reaches past the paragraph at {} by {}",
                second.y,
                second.h
            );
        }
        let mut fresh = test_engine();
        differential::compare_with_fresh(&mut engine, &mut fresh, 200, 100)
            .expect("the frame drawn in part is the whole raster");
        // A new size: drawn whole.
        engine.frame(200, 30);
        assert_eq!(engine.redrawn(), &render::Redrawn::Whole);
        // A scroll moves the document's group (M19.4): the last frame's
        // pixels move up with it, and what remains, the strip the move
        // uncovers and the scrollbar's thumb, is drawn again, composited
        // from the document's tiles, with nothing drawn into them.
        engine.load_document(
            "<style>body { margin: 0 } p { margin: 0 }</style>\
             <p>one</p><p>two</p><p>three</p><p>four</p><p>five</p><p>six</p><p>seven</p>",
        );
        engine.frame(200, 60);
        engine.set_scroll(10.0);
        engine.frame(200, 60);
        let render::Redrawn::Rects(rects) = engine.redrawn().clone() else {
            panic!("drawn whole: {:?}", engine.redrawn());
        };
        assert!(!rects.is_empty());
        let layered = engine.layered();
        assert_eq!(layered.moved, Some((0, -10)), "{layered:?}");
        assert!(layered.rasterized.is_empty(), "{layered:?}");
        assert!(
            layered.composited.contains(&paint::GroupKey::Document(0)),
            "{layered:?}"
        );
        let area: u64 = rects.iter().map(|r| r.area()).sum();
        assert!(area < 200 * 60 / 2, "{rects:?}");
        let mut fresh = test_engine();
        fresh.load_document(
            "<style>body { margin: 0 } p { margin: 0 }</style>\
             <p>one</p><p>two</p><p>three</p><p>four</p><p>five</p><p>six</p><p>seven</p>",
        );
        fresh.frame(200, 60);
        fresh.set_scroll(10.0);
        assert_eq!(engine.frame(200, 60), fresh.frame(200, 60));
        assert_eq!(engine.redrawn(), &render::Redrawn::Rects(Vec::new()));
    }

    /// A page with something of every kind the mutations reach: styled
    /// blocks and inlines, a float, a positioned box, an inline-block,
    /// a table and a scroll container, under selectors that make most
    /// styles depend on an element's siblings, its place among them or
    /// whether it has children, so that a restyle that misses any of
    /// them shows.
    const DIFFERENTIAL_PAGE: &str = "<style>\
          body { margin: 4px; font-size: 10px }\
          .a { color: #c00; padding: 2px; background-color: #eee }\
          .b { float: left; width: 30px; height: 10px; background-color: #0a0 }\
          .c { position: relative; top: 3px; border: 1px solid #00f }\
          * + * { margin-top: 1px }\
          .a ~ * { color: #060 }\
          :first-child { padding-left: 1px }\
          :last-child { padding-right: 2px }\
          :only-child { border: 1px solid #f0f }\
          :nth-child(2 of .a) { background-color: #ff0 }\
          #x { border: 3px solid #0ff } [title] > * { font-size: 12px }\
          li:first-child { font-size: 14px }\
          td { padding: 1px; border: 1px solid #000 }\
          :empty { min-height: 2px; background-color: #ccc }\
        </style>\
        <div class=a><p>one <b>two</b> three</p><p class=c>four</p></div>\
        <div class=b></div><ul><li>x</li><li style=\"display: inline-block; width: 20px\">y</li></ul>\
        <table><tr><td>1</td><td colspan=2>2</td></tr><tr><td>3</td></tr></table>\
        <div style=\"overflow: auto; height: 30px\"><p>scroll</p><p>more</p><p>and more</p></div>";

    /// A page whose boxes the layout cache can hold (M13.2): blocks
    /// nested a few deep, a table with a spanning cell, an anonymous
    /// table, absolutely positioned boxes settled by a positioned
    /// ancestor near and far, and its one float inside a context root
    /// of its own, so that until a mutation makes another nothing in
    /// the page's own context meets a float.
    const NESTED_PAGE: &str = "<style>\
          body { margin: 4px; font-size: 10px }\
          .a { padding: 2px; background-color: #eee }\
          .b { float: left; width: 30px; height: 10px; background-color: #0a0 }\
          .c { position: relative; top: 3px; border: 1px solid #00f }\
          section { margin: 3px 0; border-left: 2px solid #888 }\
          .cell { display: table-cell; padding: 1px; border: 1px solid #c0c }\
          .abs { position: absolute; top: 2px; right: 3px; width: 8px; height: 8px; background-color: #f80 }\
          td { padding: 1px; border: 1px solid #000 }\
          :empty { min-height: 2px; background-color: #ccc }\
        </style>\
        <section><div class=a><p>one <b>two</b></p><div><p>deep</p><p class=c>er<span class=abs></span></p></div></div>\
        <div><span class=cell>cell</span><span class=cell>next</span></div></section>\
        <section class=c><p>held <span class=abs></span></p><div><div><p>three levels</p></div></div></section>\
        <table><tr><td>1</td><td>2</td><td>3</td></tr><tr><td colspan=2>4</td><td>5</td></tr></table>\
        <div style=\"overflow: hidden\"><span class=b></span><p>beside</p><p>below</p></div>\
        <ul><li>x</li><li>y</li></ul>";

    /// A page of the selectors that read past an element (M16): the
    /// language from `lang` and the pragma, links, and the form states,
    /// over a fieldset with a legend, a select and radio groups.
    const FORMS_PAGE: &str = "<style>\
          body { margin: 4px; font-size: 10px }\
          :lang(fr) { color: #f00 } :lang('') { padding: 1px } :lang(\\*-CH) { margin-left: 3px }\
          :checked { display: block; width: 7px; height: 3px; background-color: #0a0 }\
          :disabled { color: #0f0 } :enabled + span { padding: 2px }\
          :link { padding: 1px } :required { border: 1px solid #f00 } :optional ~ b { display: block }\
          input, select, option, optgroup, fieldset, legend, form { display: block; min-height: 2px }\
          p::before { content: attr(title) '>' } .a::after, b:after { content: open-quote; color: #00f }\
          p::first-line { color: #f0f } div::first-letter { padding: 1px }\
        </style>\
        <meta http-equiv=content-language content=fr>\
        <form id=x><fieldset disabled><legend><input type=radio name=r checked></legend>\
        <input type=checkbox checked><span>s</span></fieldset>\
        <select><option>1</option><optgroup disabled><option selected>2</option></optgroup>\
        <option>3</option></select><input type=radio name=r checked><b>b</b></form>\
        <div lang=de-CH><p>x <a href=#>link</a></p><input type=radio name=r form=x checked required></div>\
        <form id=y><input type=radio name=r><span>t</span></form>\
        <p title=t class=a>quoted <q>words</q></p>";

    /// A page of generated boxes (M16) of each display, with quotes
    /// whose marks depend on what comes before them, on the classes the
    /// mutator sets.
    const GENERATED_PAGE: &str = "<style>\
          body { margin: 4px; font-size: 10px } q { quotes: '<' '>' '{' '}' }\
          .a::before { content: open-quote 'A'; display: block; height: 3px; background-color: #f00 }\
          .b::after { content: close-quote attr(title); display: inline-block; padding: 1px }\
          .c::before { content: 'C'; float: left; width: 5px; height: 5px; background-color: #0f0 }\
          .c::after { content: 'X'; position: absolute; top: 2px; right: 3px }\
          div::after { content: ''; display: table; clear: both }\
          td::before { content: no-open-quote 'T'; display: table-cell; width: 4px }\
          p::before { content: 'O'; display: block; overflow: hidden; height: 4px }\
          .a::first-letter { font-size: 14px; color: #00f } div::first-letter { padding: 1px }\
          td::first-letter { float: left; font-size: 20px }\
          .a::first-line { font-size: 12px; background-color: #ff0 } p::first-line { color: #f0f }\
        </style>\
        <div class=a><p class=b title=t>one <q>two <q>three</q></q></p></div>\
        <p class='b c' title=u><span class=c>x</span> y</p>\
        <div><div class=c>z</div></div>\
        <table><tr><td class=a>cell <q>q</q></td></tr></table>\
        <div class=c style='position: relative'>abs <q>r</q></div>";

    /// A page of transformed and translucent boxes (M19), whose classes
    /// and `style` attributes the mutator changes from one transform or
    /// opacity to another, which keeps the layout (M22.4): blocks, a
    /// pseudo-element, a table's wrapper, an inline-block, a box near a
    /// scroll container's bottom and one at the document's, and a fixed
    /// box.
    const TRANSFORMED_PAGE: &str = "<style>\
          body { margin: 4px; font-size: 10px }\
          div { rotate: 0deg } p { opacity: 0.9 }\
          .a { rotate: 20deg; opacity: 0.6 }\
          .b { translate: 3px 12px; scale: 1.5 }\
          .c { transform: skewX(10deg) translateY(-4px); opacity: 0.3 }\
          p.a::before { content: 'A'; display: inline-block; rotate: 45deg }\
          p.b::after { content: 'B'; display: block; translate: 0 6px; opacity: 0.5 }\
          .s { overflow: auto; height: 30px }\
          .f { position: fixed; bottom: 0; right: 0; width: 10px; height: 10px; background-color: #f80 }\
        </style>\
        <div class=a><p>one <b>two</b></p><p class=b>three</p></div>\
        <div class=s><p>scroll</p><div class=b>moved</div></div>\
        <table class=c><tr><td>1</td><td>2</td></tr></table>\
        <span class=a style='display: inline-block'>ib</span>\
        <div class='f a'></div><div class=c>last</div>";

    #[test]
    fn after_random_mutations_the_engine_shows_what_a_fresh_engine_shows() {
        // Seeded runs of random mutations, each compared after every
        // step with a fresh engine's full pass over the document as it
        // stands (M13's differential check): what the engine restyles
        // and lays out again, and what it keeps, must come to what a
        // full pass computes.
        let mut fresh = test_engine();
        for (name, page) in [
            ("first", DIFFERENTIAL_PAGE),
            ("nested", NESTED_PAGE),
            ("forms", FORMS_PAGE),
            ("generated", GENERATED_PAGE),
            ("transformed", TRANSFORMED_PAGE),
        ] {
            let (mut changed, mut reused, mut kept) = (0, 0, 0);
            for seed in 1..=8 {
                let run = mutate_and_compare(&mut fresh, name, page, seed);
                changed += run.0;
                reused += run.1;
                kept += run.2;
            }
            // Enough of the steps did something, over all the runs.
            assert!(
                changed >= 8 * 15,
                "{name} page: only {changed} steps changed anything"
            );
            // The layout cache was put to use, and so was a layout kept
            // through changes that reached nothing it reads (M22.4).
            assert!(reused > 0, "{name} page: no box reused");
            assert!(kept > 0, "{name} page: no layout kept");
        }
    }

    /// Fifty random mutations of `page` from `seed`, each followed by
    /// the differential check against `fresh`; returns how many changed
    /// anything, how many boxes the layouts reused and how many restyles
    /// kept the layout.
    fn mutate_and_compare(
        fresh: &mut Engine,
        name: &str,
        page: &str,
        seed: u64,
    ) -> (usize, u64, u64) {
        let mut engine = test_engine();
        engine.load_document(page);
        let mut mutator = differential::Mutator::new(engine.document());
        let mut rng = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let mut changed = 0;
        for step in 0..50 {
            // xorshift64: a sequence a failing seed replays.
            let script: Vec<u8> = (0..6)
                .map(|_| {
                    rng ^= rng << 13;
                    rng ^= rng >> 7;
                    rng ^= rng << 17;
                    (rng >> 32) as u8
                })
                .collect();
            engine.mutate(|document| mutator.step(document, &mut script.as_slice()));
            changed += usize::from(engine.needs_frame());
            if step % 10 == 5 {
                engine.set_scroll(step as f32 * 2.0);
            }
            if let Err(difference) = differential::compare_with_fresh(&mut engine, fresh, 160, 120)
            {
                panic!("{name} page, seed {seed}, step {step}: {difference}");
            }
        }
        (changed, engine.boxes_reused(), engine.layouts_kept())
    }

    #[test]
    fn the_document_is_a_layer_of_tiles_that_a_scroll_composites() {
        // M19.4: the document's content is a layer of tiles kept from
        // frame to frame; a scroll composites them, drawing nothing
        // into them while the rows it shows are held, and the frame is
        // what a fresh engine draws at that offset. The offset drawn is
        // a whole pixel: a fraction is rounded, for painting and hit
        // testing alike.
        // The paragraphs are all within the margin at the top, and the
        // tall box after them has no pixels of its own, so a scroll
        // lists the same items.
        let page = "<style>body { margin: 0 } p { margin: 0; height: 30px }
                           .r { background-color: #f00 } .tall { height: 2000px }</style>\
                    <p>one</p><p class=r id=r>two</p><p>three</p><p>four</p><p>five</p>\
                    <div class=tall></div>";
        let mut engine = test_engine();
        engine.load_document(page);
        engine.frame(200, 100);
        let layers = engine.layers();
        assert!(
            layers.iter().any(|l| l.key == paint::GroupKey::Document(0)
                && l.reason == paint::LayerReason::Scrolled
                && l.tiles >= 1),
            "{layers:?}"
        );
        assert!(
            engine
                .layered()
                .rasterized
                .iter()
                .any(|(key, _)| *key == paint::GroupKey::Document(0))
        );
        engine.set_scroll(60.0);
        let pixels = engine.frame(200, 100);
        let layered = engine.layered();
        assert!(layered.rasterized.is_empty(), "{layered:?}");
        assert!(layered.composited.contains(&paint::GroupKey::Document(0)));
        let mut fresh = test_engine();
        fresh.load_document(page);
        fresh.frame(200, 100);
        fresh.set_scroll(60.0);
        assert_eq!(pixels, fresh.frame(200, 100), "the composited scroll");
        // Scrolled far past the rows drawn ahead, the rows the viewport
        // shows are drawn now.
        engine.set_scroll(1000.0);
        engine.frame(200, 100);
        assert!(
            engine
                .layered()
                .rasterized
                .iter()
                .any(|(key, tiles)| *key == paint::GroupKey::Document(0) && *tiles >= 1),
            "{:?}",
            engine.layered()
        );
        // A fraction of a pixel is rounded: the frame at 29.6 is the
        // frame at 30, where the red box's top is the viewport's, and
        // the hit test agrees.
        engine.set_scroll(29.6);
        let rounded = engine.frame(200, 100);
        assert_eq!(engine.scroll_y(), 29.6, "the animation keeps its fraction");
        assert_eq!(px(&rounded, 200, 10, 0), [0xff, 0, 0, 0xff]);
        let r = node_by_id(&engine, "r");
        assert_eq!(
            engine.hit_test(10.0, 0.5),
            Some(r),
            "the red box's first row"
        );
        engine.set_scroll(30.0);
        assert_eq!(rounded, engine.frame(200, 100));
    }

    #[test]
    fn will_change_makes_a_layer_that_a_new_transform_composites() {
        // M19.2: a box whose `will-change` names `transform` is a layer
        // under its own key; a new transform on it composites the layer
        // where the matrix puts it, drawing nothing into it, and the
        // frame is what a fresh engine draws. Under a budget of nothing
        // (M19.6) the box is drawn through, the same.
        let page = "<style>body { margin: 0 }
                           .w { will-change: transform; width: 20px; height: 20px;
                                background-color: #00f }</style>\
                    <div class=w id=w></div>";
        let mut engine = test_engine();
        engine.load_document(page);
        let pixels = engine.frame(100, 60);
        assert_eq!(px(&pixels, 100, 10, 10), [0, 0, 0xff, 0xff]);
        let w = node_by_id(&engine, "w");
        let key = paint::GroupKey::Box((w, None));
        let layers = engine.layers();
        assert!(
            layers
                .iter()
                .any(|l| l.key == key && l.reason == paint::LayerReason::WillChange),
            "{layers:?}"
        );
        engine.mutate(|document| {
            document
                .set_attribute(w, "style", "transform: translate(30px, 20px)")
                .expect("an element");
        });
        let pixels = engine.frame(100, 60);
        let layered = engine.layered();
        assert!(
            !layered.rasterized.iter().any(|(k, _)| *k == key),
            "{layered:?}"
        );
        assert!(layered.composited.contains(&key), "{layered:?}");
        assert_eq!(px(&pixels, 100, 40, 30), [0, 0, 0xff, 0xff]);
        assert_eq!(px(&pixels, 100, 10, 10), [0xff, 0xff, 0xff, 0xff]);
        let mut fresh = test_engine();
        fresh.load_document(page);
        fresh.mutate(|document| {
            document
                .set_attribute(w, "style", "transform: translate(30px, 20px)")
                .expect("an element");
        });
        assert_eq!(pixels, fresh.frame(100, 60));
        engine.set_layer_budget(0);
        assert_eq!(engine.frame(100, 60), pixels);
        assert!(engine.layers().is_empty());
        assert_eq!(engine.layer_bytes(), 0);
    }

    #[test]
    fn a_3d_rendering_context_paints_and_hits_by_depth_as_the_reference_browser_does() {
        // M19.5, against the reference browser's answers for the same
        // pages: a 200px grey box with `transform-style: preserve-3d`
        // holding a red 100px box at (20, 20) and a blue one at (60,
        // 60), each positioned. The red box pushed back goes under the
        // grey plane, and the plane is hit where it covers it; brought
        // forward, it covers the blue one. With the plane transparent,
        // the red shows through from behind, but the plane's box is
        // still what is hit. A box turned away with its backface hidden
        // is neither painted nor hit; turned away with it shown, both.
        // Turned by 60 degrees about y with no perspective, the box is
        // half as wide about its centre; under a perspective of 300px
        // from the plane's centre, its near edge grows and its far edge
        // shrinks. A child with `preserve-3d` extends the context, so
        // its pushed-back box goes under the plane too, while one that
        // clips flattens it and keeps the box in its own plane, over
        // the grey.
        let page = |css: &str| {
            format!(
                "<style>body {{ margin: 0 }}
                        .ctx {{ transform-style: preserve-3d; position: absolute; left: 0; top: 0;
                                width: 200px; height: 200px; background-color: #eeeeee }}
                        .mid {{ position: absolute; left: 0; top: 0; width: 200px; height: 200px }}
                        .a {{ position: absolute; left: 20px; top: 20px; width: 100px;
                              height: 100px; background-color: #ff0000 }}
                        .b {{ position: absolute; left: 60px; top: 60px; width: 100px;
                              height: 100px; background-color: #0000ff }}
                        {css}</style>\
                 <div class=ctx id=ctx><div class=mid id=mid><div class=a id=a></div></div>\
                 <div class=b id=b></div></div>"
            )
        };
        let grey = [0xee, 0xee, 0xee, 0xff];
        let red = [0xff, 0, 0, 0xff];
        let blue = [0, 0, 0xff, 0xff];
        let white = [0xff, 0xff, 0xff, 0xff];
        // Each case: the CSS, then (x, y, the colour, the id hit).
        type Check = (usize, usize, [u8; 4], &'static str);
        let cases: [(&str, Vec<Check>); 9] = [
            (
                ".mid { transform-style: preserve-3d } .a { transform: translateZ(-10px) }",
                vec![(30, 30, grey, "mid"), (80, 80, blue, "b")],
            ),
            (
                ".mid { transform-style: preserve-3d } .a { transform: translateZ(10px) }",
                vec![(80, 80, red, "a"), (30, 30, red, "a")],
            ),
            (
                ".mid { transform-style: preserve-3d } .ctx { background-color: transparent }
                 .a { transform: translateZ(-10px) }",
                vec![(30, 30, red, "mid"), (80, 80, blue, "b")],
            ),
            (
                ".mid { transform-style: preserve-3d } .b { display: none }
                 .a { transform: rotateY(180deg); backface-visibility: hidden }",
                vec![(70, 70, grey, "mid")],
            ),
            (
                ".mid { transform-style: preserve-3d } .b { display: none }
                 .a { transform: rotateY(180deg) }",
                vec![(70, 70, red, "a")],
            ),
            (
                ".mid { transform-style: preserve-3d } .b { display: none }
                 .a { transform: rotateY(60deg) }",
                vec![
                    (50, 70, red, "a"),
                    (40, 70, grey, "mid"),
                    (94, 70, red, "a"),
                ],
            ),
            (
                ".mid { transform-style: preserve-3d; perspective: 300px } .b { display: none }
                 .a { transform: rotateY(60deg) }",
                vec![
                    (37, 70, red, "a"),
                    (34, 70, grey, "mid"),
                    (94, 70, red, "a"),
                    (97, 70, grey, "mid"),
                    (90, 40, red, "a"),
                    (90, 20, grey, "mid"),
                ],
            ),
            (
                ".mid { transform-style: preserve-3d } .a { transform: translateZ(-10px) }
                 .b { display: none }",
                vec![(30, 30, grey, "mid"), (150, 150, grey, "mid")],
            ),
            (
                ".mid { overflow: hidden; transform-style: preserve-3d }
                 .a { transform: translateZ(-10px) }",
                vec![(30, 30, red, "a"), (80, 80, blue, "b")],
            ),
        ];
        for (css, checks) in cases {
            let mut engine = test_engine();
            engine.load_document(&page(css));
            let pixels = engine.frame(200, 200);
            for (x, y, colour, id) in checks {
                assert_eq!(px(&pixels, 200, x, y), colour, "{css}: ({x}, {y})");
                let hit = engine.hit_test(x as f32 + 0.5, y as f32 + 0.5);
                assert_eq!(hit, Some(node_by_id(&engine, id)), "{css}: hit ({x}, {y})");
            }
            let mut fresh = test_engine();
            fresh.load_document(&page(css));
            assert_eq!(pixels, fresh.frame(200, 200));
        }
        let _ = white;
    }

    #[test]
    fn matrices_past_any_size_draw_and_hit_without_harm() {
        // M19: transforms whose matrices overflow, a scale of 1e30 in
        // another, a `matrix3d` of values past f32's range, a move and a
        // turn of 1e30, a perspective of 1e-30px, a scale to nothing: a
        // frame is drawn of the viewport's size, the same each time, a
        // point is hit or not, and the layers stay in their budget.
        let page = "<style>body { margin: 0 } div { width: 20px; height: 20px; \
                    background-color: #36c; margin: 4px; }</style>\
                    <div style=\"transform: scale(1e30)\">\
                      <div style=\"transform: scale(1e30); will-change: transform\"></div></div>\
                    <div style=\"transform: matrix3d(1e39, 0, 0, 0, 0, 1e39, 0, 0, 0, 0, 1, 0, \
                      1e39, 1e39, 0, 1)\"></div>\
                    <div style=\"transform: translate(1e30px, -1e30px) rotate(1e30deg)\"></div>\
                    <div style=\"perspective: 1e-30px\"><div style=\"transform: rotateY(89.9deg) \
                      translateZ(1e30px)\"></div></div>\
                    <div style=\"transform: scale(0); opacity: 0.5\"></div>\
                    <div style=\"transform: scale(-1e30, 1e-30) skew(89.99deg)\"></div>\
                    <p style=\"transform: rotate(3deg)\">text after them all</p>";
        let mut engine = test_engine();
        engine.load_document(page);
        let frame = engine.frame(200, 120);
        assert_eq!(frame.len(), 200 * 120 * 4);
        let mut fresh = test_engine();
        fresh.load_document(page);
        assert_eq!(frame, fresh.frame(200, 120));
        for (x, y) in [(0.0, 0.0), (10.0, 10.0), (100.0, 60.0), (199.0, 119.0)] {
            let _ = engine.hit_test(x, y);
        }
        engine.set_scroll(40.0);
        assert_eq!(engine.frame(200, 120).len(), 200 * 120 * 4);
        assert!(engine.layer_bytes() <= render::DEFAULT_LAYER_BUDGET);
    }

    #[test]
    fn ten_thousand_transformed_boxes_are_drawn_in_bounded_time() {
        // M19: a page of ten thousand small boxes, each turned, moved or
        // scaled, some faded: thousands of layers, scratches and groups
        // drawn through in one frame and one scroll, held to a time in
        // the unoptimised build and to the layer budget.
        let mut page = String::from(
            "<style>body { margin: 0 } i { display: inline-block; width: 10px; \
             height: 10px; background-color: #36c }</style><body>",
        );
        for k in 0..10_000 {
            let transform = ["rotate(10deg)", "translate(1px, 1px)", "scale(0.8)"][k % 3];
            let faded = if k % 5 == 0 { "; opacity: 0.5" } else { "" };
            page.push_str(&format!("<i style=\"transform: {transform}{faded}\"></i>"));
        }
        let started = Instant::now();
        let mut engine = test_engine();
        engine.load_document(&page);
        engine.frame(1280, 800);
        engine.set_scroll(400.0);
        engine.frame(1280, 800);
        let took = started.elapsed();
        println!("ten thousand transformed boxes: {took:?}");
        assert!(took.as_secs() < TEN_THOUSAND_SECONDS, "{took:?}");
        assert!(engine.layer_bytes() <= render::DEFAULT_LAYER_BUDGET);
    }

    /// The time the page of ten thousand transformed boxes may take, in
    /// the unoptimised build, which is what the tests run in.
    const TEN_THOUSAND_SECONDS: u64 = 20;

    /// The left margin of the element `id`, as the engine's styles hold
    /// it.
    fn margin_left_of(engine: &mut Engine, id: &str) -> f32 {
        let node = node_by_id(engine, id);
        engine.styles().get(node).expect("styled").margin[style::computed::LEFT]
            .resolve(0.0, f32::NAN)
    }

    /// Draws a frame for the caller's time `ms`.
    fn draw_at(engine: &mut Engine, ms: u64) {
        engine.frame_at(200, 100, FrameTime::at(Duration::from_millis(ms)));
    }

    const SLIDE: &str = "<style>@keyframes s { from { margin-left: 0px } \
                         to { margin-left: 100px } }</style>";

    /// Sets the attribute `name` of the element `id` through the engine.
    fn set(engine: &mut Engine, id: &str, name: &str, value: &str) {
        let node = node_by_id(engine, id);
        engine.mutate(|document| document.set_attribute(node, name, value).unwrap());
    }

    #[test]
    fn a_transition_runs_on_the_frames_after_a_change_and_waits_out_its_delay() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>#a { margin-left: 0px; transition: margin-left 1s linear 0.5s }
             #a.on { margin-left: 100px }</style><div id=a></div>",
        );
        draw_at(&mut engine, 1000);
        set(&mut engine, "a", "class", "on");
        assert!(engine.needs_frame());
        draw_at(&mut engine, 1100);
        assert_eq!(margin_left_of(&mut engine, "a"), 0.0);
        // In its delay: no frame wanted until it ends.
        assert!(!engine.needs_frame());
        assert_eq!(engine.next_frame_at(), Some(Duration::from_millis(1600)));
        draw_at(&mut engine, 2100);
        assert_eq!(margin_left_of(&mut engine, "a"), 50.0);
        assert!(engine.needs_frame());
        draw_at(&mut engine, 2600);
        assert_eq!(margin_left_of(&mut engine, "a"), 100.0);
        assert!(!engine.needs_frame());
        // A change of the sheets transitions as a change of the document
        // does.
        engine.mutate(|document| {
            let style = document.create_element("style").unwrap();
            let text = document
                .create_text("#a.on { margin-left: 200px }")
                .unwrap();
            document.append_child(style, text).unwrap();
            let body = document
                .subtree(document.root())
                .find(|&n| document.tag(n) == Some("body"))
                .unwrap();
            document.append_child(body, style).unwrap();
        });
        draw_at(&mut engine, 3000);
        assert_eq!(margin_left_of(&mut engine, "a"), 100.0);
        draw_at(&mut engine, 4000);
        assert_eq!(margin_left_of(&mut engine, "a"), 150.0);
    }

    #[test]
    fn finishing_the_transitions_shows_where_they_were_going() {
        // As a script's finish() of each: the next frame ends them, the
        // element showing the value it was going to, and the
        // differential check compares a fresh engine, which has none,
        // with the engine so finished.
        let mut engine = test_engine();
        engine.load_document(
            "<style>#a { margin-left: 0px; transition: margin-left 1s linear 0.5s }
             #a.on { margin-left: 100px }
             #b { transition: margin-left 0.1s linear } #b.on { margin-left: 50px }</style>
             <div id=a></div><div id=b></div>",
        );
        draw_at(&mut engine, 0);
        set(&mut engine, "a", "class", "on");
        draw_at(&mut engine, 100);
        assert_eq!(margin_left_of(&mut engine, "a"), 0.0);
        engine.finish_transitions();
        assert!(engine.needs_frame());
        draw_at(&mut engine, 100);
        assert_eq!(margin_left_of(&mut engine, "a"), 100.0);
        assert!(engine.animations().is_empty());
        assert!(!engine.needs_frame());
        // One started again, back, and the check made in its delay.
        set(&mut engine, "a", "class", "");
        draw_at(&mut engine, 200);
        assert_eq!(engine.animations().len(), 1);
        let mut fresh = test_engine();
        if let Err(difference) = differential::compare_with_fresh(&mut engine, &mut fresh, 200, 100)
        {
            panic!("{difference}");
        }
        assert_eq!(margin_left_of(&mut engine, "a"), 0.0);
        // A change no frame has drawn yet, its transition started by the
        // check's first frame: finished all the same, though at 0.7s the
        // time 0.1s before rounds to less than 0.1s before.
        draw_at(&mut engine, 700);
        set(&mut engine, "b", "class", "on");
        if let Err(difference) = differential::compare_with_fresh(&mut engine, &mut fresh, 200, 100)
        {
            panic!("{difference}");
        }
        assert_eq!(margin_left_of(&mut engine, "b"), 50.0);
        assert!(engine.animations().is_empty());
    }

    #[test]
    fn an_element_transitioning_out_of_view_is_drawn_and_not_hit() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 } div { width: 100px; height: 50px; background: #f00 }
             #a { transition: opacity 1s linear, display 1s allow-discrete }
             #a.gone { display: none; opacity: 0 }</style><div id=a><span>x</span></div>",
        );
        draw_at(&mut engine, 0);
        let a = node_by_id(&engine, "a");
        assert_eq!(engine.hit_test(10.0, 10.0), Some(a));
        set(&mut engine, "a", "class", "gone");
        draw_at(&mut engine, 100);
        draw_at(&mut engine, 600);
        // Drawn half faded, and passed over by the pointer, its text too.
        let frame = engine.frame(200, 100);
        let pixel = px(&frame, 200, 10, 10);
        assert!(pixel[1] > 100 && pixel[1] < 160, "{pixel:?}");
        let hit = engine.hit_test(10.0, 10.0);
        assert!(
            hit.is_none_or(|node| node != a && engine.document().parent(node) != Some(a)),
            "{hit:?}"
        );
        draw_at(&mut engine, 1200);
        assert_eq!(engine.hit_test(10.0, 10.0), None);
    }

    #[test]
    fn a_frame_gives_its_events_and_the_engine_lists_what_runs() {
        let mut engine = test_engine();
        engine.load_document(&format!(
            "{SLIDE}<style>#a {{ animation: s 1s linear }}
             #b {{ margin-left: 0px; transition: margin-left 1s linear }}
             #b.on {{ margin-left: 100px }}</style><div id=a></div><div id=b></div>"
        ));
        draw_at(&mut engine, 0);
        assert!(
            engine.animation_events().is_empty(),
            "made by this frame's style"
        );
        set(&mut engine, "b", "class", "on");
        draw_at(&mut engine, 100);
        let (a, b) = (node_by_id(&engine, "a"), node_by_id(&engine, "b"));
        let events: Vec<(NodeId, EventKind, &str)> = engine
            .animation_events()
            .iter()
            .map(|e| (e.node, e.kind, &*e.name))
            .collect();
        // The transition this frame's style made starts at its end, and
        // its events come with the next frame.
        assert_eq!(events, vec![(a, EventKind::AnimationStart, "s")]);
        let running = engine.animations();
        assert_eq!(running.len(), 2);
        assert_eq!((running[0].node, &*running[0].name), (a, "s"));
        assert_eq!(running[0].phase, Some(style::animation::Phase::Active));
        assert!((running[0].current_time - 0.1).abs() < 1e-9);
        assert!(running[1].transition && running[1].current_time == 0.0);
        draw_at(&mut engine, 200);
        let events: Vec<(NodeId, EventKind)> = engine
            .animation_events()
            .iter()
            .map(|e| (e.node, e.kind))
            .collect();
        assert_eq!(
            events,
            vec![
                (b, EventKind::TransitionRun),
                (b, EventKind::TransitionStart)
            ]
        );
        // Each frame gives its own.
        draw_at(&mut engine, 300);
        assert!(engine.animation_events().is_empty());
        draw_at(&mut engine, 1100);
        let kinds: Vec<EventKind> = engine.animation_events().iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![EventKind::AnimationEnd, EventKind::TransitionEnd]
        );
        // The transition is gone; the animation stays, finished, while
        // `animation-name` names it.
        let running = engine.animations();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].phase, Some(style::animation::Phase::After));
    }

    #[test]
    fn an_animation_starts_at_the_first_frame_after_it_is_made_and_runs_on_its_times() {
        let mut engine = test_engine();
        engine.load_document(&format!(
            "{SLIDE}<style>#a {{ animation: s 1s linear }}</style><div id=a></div>"
        ));
        // The timeline begins at the first frame drawn for a time, and
        // the animation, made by that frame's style, starts there.
        draw_at(&mut engine, 5000);
        assert_eq!(margin_left_of(&mut engine, "a"), 0.0);
        assert!(engine.needs_frame(), "it plays");
        draw_at(&mut engine, 5250);
        assert_eq!(margin_left_of(&mut engine, "a"), 25.0);
        // However many frames share the time.
        draw_at(&mut engine, 5500);
        draw_at(&mut engine, 5500);
        assert_eq!(margin_left_of(&mut engine, "a"), 50.0);
        assert_eq!(engine.next_frame_at(), None);
        // Ended, without a fill: the underlying value, and no frame.
        draw_at(&mut engine, 6200);
        assert_eq!(margin_left_of(&mut engine, "a"), 0.0);
        assert!(!engine.needs_frame());
        assert_eq!(engine.next_frame_at(), None);
        // An animation made between frames shows its start until the
        // next frame starts it.
        engine.mutate(|document| {
            let b = document.create_element("div").unwrap();
            document.set_attribute(b, "id", "b").unwrap();
            document
                .set_attribute(b, "style", "animation: s 1s linear")
                .unwrap();
            let body = document
                .subtree(document.root())
                .find(|&n| document.tag(n) == Some("body"))
                .unwrap();
            document.append_child(body, b).unwrap();
        });
        assert_eq!(margin_left_of(&mut engine, "b"), 0.0);
        assert!(engine.needs_frame());
        draw_at(&mut engine, 7000);
        assert_eq!(margin_left_of(&mut engine, "b"), 0.0);
        draw_at(&mut engine, 7100);
        assert_eq!(margin_left_of(&mut engine, "b"), 10.0);
    }

    #[test]
    fn a_delay_asks_for_no_frame_until_it_ends() {
        let mut engine = test_engine();
        engine.load_document(&format!(
            "{SLIDE}<style>#a {{ animation: s 1s linear 2s }}</style><div id=a></div>"
        ));
        draw_at(&mut engine, 1000);
        assert!(
            !engine.needs_frame(),
            "nothing to show before the delay ends"
        );
        assert_eq!(engine.next_frame_at(), Some(Duration::from_millis(3000)));
        draw_at(&mut engine, 2999);
        assert_eq!(margin_left_of(&mut engine, "a"), 0.0);
        draw_at(&mut engine, 3500);
        assert_eq!(margin_left_of(&mut engine, "a"), 50.0);
        assert!(engine.needs_frame());
        // A document shown again finds its timeline where the clock has
        // gone: a new one starts its own.
        engine.load_document(&format!(
            "{SLIDE}<style>#a {{ animation: s 1s linear 2s }}</style><div id=a></div>"
        ));
        draw_at(&mut engine, 9000);
        assert_eq!(engine.next_frame_at(), Some(Duration::from_millis(11000)));
    }

    #[test]
    fn frames_without_a_time_show_each_animation_at_its_start_and_tick_moves_them_on() {
        let mut engine = test_engine();
        engine.load_document(&format!(
            "{SLIDE}<style>#a {{ animation: s 1s linear }} \
             #b {{ animation: s 1s linear -0.5s paused }}</style><div id=a></div><div id=b></div>"
        ));
        for _ in 0..3 {
            engine.frame(200, 100);
        }
        assert_eq!(margin_left_of(&mut engine, "a"), 0.0);
        assert_eq!(margin_left_of(&mut engine, "b"), 50.0);
        // Fifteen ticks are a quarter of a second.
        for _ in 0..15 {
            assert!(engine.tick(), "the animation plays");
        }
        engine.frame(200, 100);
        assert!((margin_left_of(&mut engine, "a") - 25.0).abs() < 1e-3);
        assert_eq!(margin_left_of(&mut engine, "b"), 50.0);
    }

    #[test]
    fn a_frame_restyles_the_animated_elements_alone_and_a_moved_one_starts_again() {
        let mut engine = test_engine();
        let paragraphs = "<p>still</p>".repeat(20);
        engine.load_document(&format!(
            "{SLIDE}<style>#a {{ animation: s 1s linear }}</style>{paragraphs}\
             <div id=a></div><div id=b></div>"
        ));
        draw_at(&mut engine, 0);
        let restyled = engine.restyled_elements();
        draw_at(&mut engine, 100);
        assert_eq!(engine.restyled_elements(), restyled + 1);
        assert_eq!(margin_left_of(&mut engine, "a"), 10.0);
        // A frame at the same time restyles nothing.
        draw_at(&mut engine, 100);
        assert_eq!(engine.restyled_elements(), restyled + 1);
        // Moved, the element's animation starts again.
        engine.mutate(|document| {
            let a = document
                .subtree(document.root())
                .find(|&n| document.attr(n, "id") == Some("a"))
                .unwrap();
            let b = document
                .subtree(document.root())
                .find(|&n| document.attr(n, "id") == Some("b"))
                .unwrap();
            document.append_child(b, a).unwrap();
        });
        draw_at(&mut engine, 200);
        assert_eq!(margin_left_of(&mut engine, "a"), 0.0);
        draw_at(&mut engine, 300);
        assert_eq!(margin_left_of(&mut engine, "a"), 10.0);
        // Moved while the sheets change, the document cascaded whole: it
        // starts again all the same.
        engine.mutate(|document| {
            let find = |id: &str| {
                document
                    .subtree(document.root())
                    .find(|&n| document.attr(n, "id") == Some(id))
                    .unwrap()
            };
            let (a, b) = (find("a"), find("b"));
            let parent = document.parent(b).unwrap();
            document.append_child(parent, a).unwrap();
            let style = document.create_element("style").unwrap();
            let text = document.create_text("p { color: #00f }").unwrap();
            document.append_child(style, text).unwrap();
            document.append_child(parent, style).unwrap();
        });
        draw_at(&mut engine, 350);
        assert_eq!(margin_left_of(&mut engine, "a"), 0.0);
        // Taken out, it runs nothing.
        engine.mutate(|document| {
            let a = document
                .subtree(document.root())
                .find(|&n| document.attr(n, "id") == Some("a"))
                .unwrap();
            let parent = document.parent(a).unwrap();
            document.remove_child(parent, a).unwrap();
        });
        draw_at(&mut engine, 400);
        assert!(!engine.needs_frame());
    }

    /// The changes a page's classes and `style` attributes can make
    /// from one transform or opacity to another (M22.4).
    const COMPOSITED_CHANGES: [(&str, &str); 10] = [
        ("class", "a"),
        ("class", "b"),
        ("class", "c"),
        ("class", "b c"),
        ("style", "rotate: 30deg; opacity: 0.5"),
        ("style", "opacity: 0.3"),
        ("style", "opacity: 0.7"),
        ("style", "transform: translate(5px, 40px) scale(2)"),
        ("style", "translate: 0 -8px; opacity: 0.2"),
        ("style", ""),
    ];

    #[test]
    fn composited_changes_keep_the_layout_and_show_what_a_fresh_engine_shows() {
        // Each element of the transformed page given each change in
        // turn, compared after each with a fresh engine's full pass (the
        // differential check): the layouts kept through the composited
        // changes, their fragments given the new effects and the
        // scrollable overflow measured again, are those a full pass
        // computes, scrolled or not.
        let mut fresh = test_engine();
        let mut engine = test_engine();
        engine.load_document(TRANSFORMED_PAGE);
        let check = |engine: &mut Engine, fresh: &mut Engine, step: &str| {
            if let Err(difference) = differential::compare_with_fresh(engine, fresh, 160, 120) {
                panic!("{step}: {difference}");
            }
        };
        check(&mut engine, &mut fresh, "loaded");
        let document = engine.document();
        let elements: Vec<NodeId> = document
            .subtree(document.root())
            .filter(|&n| {
                document.is_element(n)
                    && !matches!(document.tag(n), Some("html" | "head" | "style"))
            })
            .collect();
        let (runs, kept) = (engine.layout_runs(), engine.layouts_kept());
        let mut steps = 0;
        for (i, &element) in elements.iter().enumerate() {
            // The changes, then the attributes put back as they were, so
            // that the next element's changes are made in the page as
            // it was loaded.
            let document = engine.document();
            let restore = ["class", "style"]
                .map(|name| (name, document.attr(element, name).map(str::to_string)));
            let changes = COMPOSITED_CHANGES
                .iter()
                .map(|&(name, value)| (name, Some(value.to_string())))
                .chain(restore);
            for (name, value) in changes {
                if engine.document().attr(element, name) == value.as_deref() {
                    continue; // no change
                }
                engine.mutate(|document| match &value {
                    Some(value) => document.set_attribute(element, name, value).unwrap(),
                    None => document.remove_attribute(element, name).unwrap(),
                });
                if steps % 7 == 3 {
                    engine.set_scroll(steps as f32);
                }
                check(
                    &mut engine,
                    &mut fresh,
                    &format!("element {i}, {name}={value:?}"),
                );
                steps += 1;
            }
        }
        let (laid, kept) = (engine.layout_runs() - runs, engine.layouts_kept() - kept);
        // A third of the changes and more kept the layout: those that
        // restyled nothing, and the composited changes but where a
        // transform moved the bottom of a scroll container's overflow or
        // the document's, which this small page's boxes are often near.
        // The others laid it out.
        assert_eq!(laid + kept, steps, "every step restyled");
        assert!(kept * 3 > steps, "{kept} of {steps} kept the layout");
        assert!(laid > 0, "no step laid out");
    }

    #[test]
    fn a_transform_or_opacity_animation_lays_nothing_out_and_is_drawn_and_hit_where_it_is() {
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 } div { width: 50px; height: 50px }
             @keyframes slide { from { translate: 0px } to { translate: 100px } }
             @keyframes fade { from { opacity: 1 } to { opacity: 0 } }
             #a { background: #f00; animation: slide 1s linear }
             #b { background: #00f; animation: fade 1s linear }
             #c { height: 400px }</style><div id=a></div><div id=b></div><div id=c></div>",
        );
        draw_at(&mut engine, 0);
        let a = node_by_id(&engine, "a");
        assert_ne!(engine.hit_test(60.0, 10.0), Some(a));
        let (runs, kept) = (engine.layout_runs(), engine.layouts_kept());
        let frame = engine
            .frame_at(200, 100, FrameTime::at(Duration::from_millis(500)))
            .to_vec();
        // Both changes were written into the layout standing.
        assert_eq!(engine.layout_runs(), runs);
        assert_eq!(engine.layouts_kept(), kept + 1);
        // The red box is drawn and hit 50px along, the blue one half
        // faded.
        assert_eq!(px(&frame, 200, 60, 10), [255, 0, 0, 255]);
        assert_eq!(px(&frame, 200, 40, 10), [255, 255, 255, 255]);
        assert_eq!(engine.hit_test(60.0, 10.0), Some(a));
        assert_ne!(engine.hit_test(40.0, 10.0), Some(a));
        let faded = px(&frame, 200, 10, 60);
        assert!(
            faded[0] > 100 && faded[0] < 160 && faded[2] == 255,
            "{faded:?}"
        );
        // Nor do the frames after.
        for ms in [600, 700, 800] {
            draw_at(&mut engine, ms);
        }
        assert_eq!(engine.layout_runs(), runs);
        assert_eq!(engine.layouts_kept(), kept + 4);
        assert_eq!(engine.hit_test(90.0, 10.0), Some(a));
    }

    #[test]
    fn a_transform_reaching_its_scroll_containers_bottom_or_the_documents_is_laid_out() {
        // A transform is part of the scrollable overflow (M19): a box an
        // animation moves down past its scroll container's bottom, or
        // the document's, makes them taller, as in the reference
        // browser, where a 100px scroller's `scrollHeight` is 548px when
        // its 50px box is 498px down.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 } #s { overflow: auto; height: 100px }
             #a, #b { height: 50px; animation: down 1s linear }
             #b { animation-name: up }
             @keyframes down { from { translate: 0 0 } to { translate: 0 996px } }
             @keyframes up { from { translate: 0 0 } to { translate: 0 100px } }
             </style><div id=s><div id=a></div></div><div id=b></div>",
        );
        draw_at(&mut engine, 0);
        let s = node_by_id(&engine, "s");
        assert_eq!(engine.element_max_scroll(s), 0.0);
        assert_eq!(engine.max_scroll(), 50.0);
        let runs = engine.layout_runs();
        draw_at(&mut engine, 500);
        assert_eq!(engine.layout_runs(), runs + 1);
        assert_eq!(engine.element_max_scroll(s), 448.0);
        assert_eq!(engine.max_scroll(), 100.0);
    }

    #[test]
    fn a_box_turned_through_its_style_attribute_lays_nothing_out() {
        // The card M19's acceptance turns, one of a dozen inline blocks,
        // turned two degrees more at each frame; and a change of an
        // attribute that restyles nothing.
        let mut engine = test_engine();
        let cards = "<div class=card><p>card</p></div>".repeat(11);
        engine.load_document(&format!(
            "<style>body {{ margin: 0 }} .card {{ display: inline-block; width: 60px;
             height: 40px; margin: 0 10px 10px 0; background: #0a0 }}</style>\
             <div><div class=card id=spin style='transform: rotate(0deg)'><p>card</p></div>\
             {cards}</div>"
        ));
        engine.frame(200, 100);
        let spin = node_by_id(&engine, "spin");
        let (runs, kept) = (engine.layout_runs(), engine.layouts_kept());
        for degrees in (2..=90).step_by(2) {
            set(
                &mut engine,
                "spin",
                "style",
                &format!("transform: rotate({degrees}deg)"),
            );
            engine.frame(200, 100);
        }
        assert_eq!(engine.layout_runs(), runs);
        assert_eq!(engine.layouts_kept(), kept + 45);
        // Turned a quarter about its centre (30, 20): 40px wide and 60px
        // tall, from -10px to 50px down.
        let in_spin = |engine: &mut Engine, x: f32, y: f32| {
            engine.hit_test(x, y).is_some_and(|hit| {
                hit == spin || engine.document().ancestors(hit).any(|n| n == spin)
            })
        };
        assert!(in_spin(&mut engine, 25.0, 45.0));
        assert!(!in_spin(&mut engine, 5.0, 5.0));
        set(&mut engine, "spin", "data-turned", "");
        engine.frame(200, 100);
        assert_eq!(
            (engine.layout_runs(), engine.layouts_kept()),
            (runs, kept + 46)
        );
        // A change to what layout reads lays it out.
        set(
            &mut engine,
            "spin",
            "style",
            "transform: rotate(90deg); width: 80px",
        );
        engine.frame(200, 100);
        assert_eq!(engine.layout_runs(), runs + 1);
    }

    #[test]
    fn a_restyle_to_the_same_style_leaves_the_boxes_effects_to_be_found() {
        // The element restyled for a selector's sake to the style it had:
        // the style in place stays, whose effects its boxes carry, and
        // the composited change after finds them.
        let mut engine = test_engine();
        engine.load_document(
            "<style>#t { rotate: 5deg; height: 20px } [data-on] #t { rotate: 5deg }
             #t.more { rotate: 10deg }</style><div id=d><div id=t></div></div><p>after</p>",
        );
        engine.frame(200, 100);
        let (runs, kept, restyled) = (
            engine.layout_runs(),
            engine.layouts_kept(),
            engine.restyled_elements(),
        );
        set(&mut engine, "d", "data-on", "");
        engine.frame(200, 100);
        assert!(engine.restyled_elements() > restyled, "restyled");
        set(&mut engine, "t", "class", "more");
        engine.frame(200, 100);
        assert_eq!(
            (engine.layout_runs(), engine.layouts_kept()),
            (runs, kept + 2)
        );
    }

    #[test]
    fn a_transition_of_a_transform_lays_nothing_out_from_its_start_to_its_end() {
        // Its start changes the after-change style the cascade keeps
        // beside the element's, which the children read and layout does
        // not; its frames and its end are composited changes, its boxes
        // found by the effects of the style left in place. A width's
        // start lays nothing out either; its frames do.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 } #t { rotate: 5deg; height: 20px; transition: rotate 1s linear }
             #t.on { rotate: 45deg } #w { width: 50px; height: 20px; transition: width 1s linear }
             #w.on { width: 150px } #c { height: 400px }</style>
             <div id=t><p>inside</p></div><div id=w></div><div id=c></div>",
        );
        draw_at(&mut engine, 0);
        let (runs, kept) = (engine.layout_runs(), engine.layouts_kept());
        set(&mut engine, "t", "class", "on");
        for ms in [100, 600, 1200] {
            draw_at(&mut engine, ms);
        }
        assert_eq!(engine.layout_runs(), runs);
        assert_eq!(engine.layouts_kept(), kept + 3);
        assert!(engine.animations().is_empty());
        let mut fresh = test_engine();
        if let Err(difference) = differential::compare_with_fresh(&mut engine, &mut fresh, 200, 100)
        {
            panic!("{difference}");
        }
        let runs = engine.layout_runs();
        set(&mut engine, "w", "class", "on");
        draw_at(&mut engine, 1300);
        assert_eq!(engine.layout_runs(), runs);
        draw_at(&mut engine, 1800);
        assert_eq!(engine.layout_runs(), runs + 1);
    }

    #[test]
    fn a_box_the_cache_keeps_through_a_composited_change_keeps_its_new_effects() {
        // A later change lays the document out again from the cache: the
        // turned box taken from it is turned as it was last shown.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 } #t { width: 100px; height: 20px; translate: 0px;
             background: #00f } section { margin: 0 }</style>\
             <section><div id=t></div><p>held</p></section><p id=text>words</p>",
        );
        engine.frame(200, 100);
        set(&mut engine, "t", "style", "translate: 50px");
        engine.frame(200, 100);
        let t = node_by_id(&engine, "t");
        let text = node_by_id(&engine, "text");
        let (runs, reused) = (engine.layout_runs(), engine.boxes_reused());
        engine.mutate(|document| {
            let word = document.first_child(text).unwrap();
            document.set_text(word, "other words").unwrap();
        });
        engine.frame(200, 100);
        assert_eq!(engine.layout_runs(), runs + 1);
        assert!(engine.boxes_reused() > reused, "the section was taken");
        assert_eq!(engine.hit_test(120.0, 10.0), Some(t));
        assert_ne!(engine.hit_test(20.0, 10.0), Some(t));
    }

    /// The preferences of a user who asked for reduced motion.
    const REDUCED: Preferences = Preferences {
        color_scheme: ColorScheme::Light,
        reduced_motion: true,
    };

    /// A 50px scroll container holding a 200px block, then another
    /// 200px block below: 150px of scrolling for each, the container
    /// and the viewport, at 200 by 100.
    const SCROLLERS: &str = "<style>body { margin: 0 } .s { overflow: auto; height: 50px }
         .tall { height: 200px }</style>\
         <div class=s id=s><div class=tall></div></div><div class=tall></div>";

    #[test]
    fn under_reduced_motion_a_scroll_jumps_to_its_target() {
        // M22.5: the engine's own motion kept to what is essential. A
        // scroll by the keys or the wheel, the document's or a scroll
        // container's, is where it goes at once, and the frame that
        // shows it is the one wanted.
        let mut engine = test_engine();
        engine.set_preferences(REDUCED);
        engine.load_document(SCROLLERS);
        engine.frame(200, 100);
        assert!(!engine.needs_frame());
        let s = node_by_id(&engine, "s");
        engine.scroll_by(40.0);
        assert_eq!((engine.scroll_y(), engine.scroll_target()), (40.0, 40.0));
        assert!(engine.needs_frame(), "the frame that shows it");
        engine.frame(200, 100);
        assert!(!engine.needs_frame());
        assert!(!engine.tick());
        engine.scroll_to(f32::INFINITY);
        assert_eq!(engine.scroll_y(), 150.0);
        engine.scroll_to(0.0);
        engine.frame(200, 100);
        engine.wheel(10.0, 5.0, 30.0);
        assert_eq!(
            (engine.element_scroll(s), engine.element_scroll_target(s)),
            (30.0, 30.0)
        );
        assert!(engine.needs_frame());
        engine.frame(200, 100);
        assert!(!engine.needs_frame());
        assert!(!engine.tick());
    }

    #[test]
    fn reduced_motion_asked_for_during_a_scroll_puts_it_where_it_goes() {
        let mut engine = test_engine();
        engine.load_document(SCROLLERS);
        engine.frame(200, 100);
        let s = node_by_id(&engine, "s");
        engine.scroll_by(100.0);
        engine.scroll_element_by(s, 30.0);
        assert!(engine.tick(), "on their way");
        assert!(engine.scroll_y() < 100.0 && engine.element_scroll(s) < 30.0);
        engine.set_preferences(REDUCED);
        assert_eq!((engine.scroll_y(), engine.element_scroll(s)), (100.0, 30.0));
        assert!(engine.needs_frame());
        engine.frame(200, 100);
        assert!(!engine.tick());
        // Asked no more, a scroll moves over the frames again.
        engine.set_preferences(Preferences::default());
        engine.scroll_by(-100.0);
        assert_eq!(engine.scroll_y(), 100.0);
        assert!(engine.tick());
    }

    #[test]
    fn animations_and_transitions_run_as_written_under_reduced_motion() {
        // M22.5: the page's own `prefers-reduced-motion` query is its
        // means of honouring the user, as in every browser; the engine
        // stops nothing, which would change what pages show at their
        // ends and when their events come.
        let mut engine = test_engine();
        engine.set_preferences(REDUCED);
        engine.load_document(&format!(
            "{SLIDE}<style>#a {{ animation: s 1s linear }}
             @media (prefers-reduced-motion: no-preference) {{ #b {{ animation: s 1s linear }} }}
             #c {{ margin-left: 0px; transition: margin-left 1s linear }}
             #c.on {{ margin-left: 100px }}</style>\
             <div id=a></div><div id=b></div><div id=c></div>"
        ));
        draw_at(&mut engine, 0);
        set(&mut engine, "c", "class", "on");
        draw_at(&mut engine, 500);
        // The animation half way, the one the page's query leaves out
        // not made.
        assert_eq!(margin_left_of(&mut engine, "a"), 50.0);
        assert_eq!(margin_left_of(&mut engine, "b"), 0.0);
        assert!(engine.needs_frame());
        // The transition, made by the frame at 500, half way at 1000.
        draw_at(&mut engine, 1000);
        assert_eq!(margin_left_of(&mut engine, "c"), 50.0);
    }
}
