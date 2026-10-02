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

pub mod differential;
mod loader;
mod navigation;
#[cfg(test)]
mod testing;

use std::collections::HashMap;
use std::time::{Duration, Instant};

pub use loader::{LoadError, LoadState};
pub use net::{Client, NetworkError, Url};

use dom::{Change, Document, NodeId};
use layout::{DocumentLayout, Fragment, FragmentKind, LayoutCache};
use loader::{Incoming, Network, Page, PageId, Target};
use navigation::{History, StateId};
use paint::{
    DisplayItem, Rect, SCROLLBAR_THUMB, SCROLLBAR_TRACK, Step, Viewport, padding_box,
    paint_fragments_in, paint_order, scrollbar_thumb,
};
use render::{RenderBackend as _, SoftwareBackend};
use style::computed::Overflow;
use style::{CascadeContext, FontMeasure, Styles, Stylesheets};
use text::FontError;

/// The document rendered when no other is loaded.
const DOCUMENT_SOURCE: &str = "\
<html>\
  <head>\
    <style>\
      html { background-color: #1e1e2e; }\
      h1 { color: #cdd6f4; }\
      p { color: #a6adc8; }\
      .highlight { color: #f9e2af; font-size: 20px; }\
      .card {\
        display: block;\
        margin: 16px 0;\
        padding: 12px;\
        border-width: 2px;\
        border-style: solid;\
        border-color: #89b4fa;\
        background-color: #313244;\
      }\
      .clipped { height: 40px; overflow: hidden; }\
      .scrolls { height: 60px; overflow: auto; }\
      a { color: #89b4fa; }\
    </style>\
  </head>\
  <body>\
    <h1>Hello from Silver Browser</h1>\
    <p>A hobby browser engine in Rust — milestone 9:</p>\
    <p class=\"highlight\">this page is laid out by our own block formatting context — real width/margin resolution and margin collapsing, not a flat vertical list.</p>\
    <div class=\"card\">\
      <p>This card's background and border are painted from real fragment geometry (M6.8) — the box model actually exists now.</p>\
    </div>\
    <div class=\"card clipped\">\
      <p>This card is 40px tall with overflow: hidden, so this second sentence is clipped away at its padding edge rather than painted over whatever follows. The clip is a rectangle in the display list (M9.3).</p>\
    </div>\
    <div class=\"card scrolls\">\
      <p>This card is 60px tall with overflow: auto, so it scrolls on its own: turn the wheel over it and its content moves inside the clip, with the overlay scrollbar at its right edge showing where it is (M10.5). At the end of its content the wheel goes on to the page, which is scroll chaining.</p>\
      <p>A second paragraph, to give the card something more to scroll through.</p>\
    </div>\
    <p>Scroll with the wheel, the arrow keys, Page Up and Page Down, Home and End, the space bar, or by dragging the scrollbar at the right (M9.2). Click <a href=\"about:silver\">this link</a> and the engine finds the anchor under the pointer (M9.4; the shell only prints it — navigation is M15).</p>\
    <p>The paragraphs below only exist to make the page taller than the window.</p>\
    <p>One. The quick brown fox jumps over the lazy dog, and the layout is cached across scrolls (M9.5): only paint and raster run again.</p>\
    <p>Two. The quick brown fox jumps over the lazy dog.</p>\
    <p>Three. The quick brown fox jumps over the lazy dog.</p>\
    <p>Four. The quick brown fox jumps over the lazy dog.</p>\
    <p>Five. The quick brown fox jumps over the lazy dog.</p>\
    <p>Six. The quick brown fox jumps over the lazy dog.</p>\
    <p>Seven. The quick brown fox jumps over the lazy dog.</p>\
    <p>Eight. The quick brown fox jumps over the lazy dog.</p>\
    <p>Nine. The quick brown fox jumps over the lazy dog.</p>\
    <p>Ten. The end of the page.</p>\
  </body>\
</html>";

/// How much of the remaining distance to the scroll target the scroll
/// animation covers in one frame of sixty a second, [`FRAME_BUDGET`]
/// (M9.2's lerp, measured in time since M13.4): a 1000px scroll settles
/// in about twenty such frames, a third of a second.
const SCROLL_TICK_FRACTION: f32 = 1.0 / 3.0;
/// Within this distance of the target, the animation lands on it.
const SCROLL_SNAP_PX: f32 = 0.5;

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
    /// Rasterizing it.
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
    backend: SoftwareBackend,
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
    /// How many boxes placed in flow the layouts have laid out, and how
    /// many they took from the cache instead: what proves a change lays
    /// out what it reaches and no more.
    boxes_laid_out: u64,
    boxes_reused: u64,
    /// The last frame's size: what scrolling is clamped to.
    viewport: (u32, u32),
    /// The time of the last frame drawn while a scroll offset was on its
    /// way to its target (M13.4): the next frame advances the animation
    /// by the time since. `None` when none was, so that an animation
    /// begun since starts with one frame's step, however long the
    /// engine was idle.
    animation_clock: Option<Duration>,
    /// What the last frame took (M13.4).
    stats: FrameStats,
    /// The scroll offset the last frame showed: a frame at another moves
    /// everything in it, and is rasterized whole without comparing its
    /// display list with the last one's (M13.3).
    drawn_scroll_y: f32,
    /// Whether the document was replaced or changed, or a scroll offset
    /// set, since the last frame.
    changed: bool,
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
    /// left: HTML's "salvageable", which aborting its fetches ends.
    salvageable: bool,
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
            layout: None,
            layout_cache: LayoutCache::new(),
            scroll_y: 0.0,
            scroll_target: 0.0,
            element_scroll: HashMap::new(),
            initial: false,
            salvageable: true,
        }
    }
}

/// A navigation under way: its document's load, and what showing it
/// does to the session history.
struct Navigation {
    incoming: Incoming,
    handling: Handling,
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
#[derive(Debug, Clone, Copy, PartialEq)]
enum ScrollGoal {
    /// An entry's saved offset.
    Offset(f32),
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
    /// system's fonts.
    pub fn new() -> Result<Self, FontError> {
        Ok(Self::with_fonts(text::FontCollection::new()?))
    }

    /// Creates a session showing the initial `about:blank`, as HTML's
    /// new navigable does (M15), with fonts already loaded (a test's
    /// bundled font, say). Layout and the rasterizer share the
    /// collection by cloning its handles.
    pub fn with_fonts(fonts: text::FontCollection) -> Self {
        let mut network = Network::new();
        let mut page = Page::given(0);
        let document = parsed_or_empty("");
        page.begin(&document, &mut network);
        let mut shown = Shown::new(document, page);
        shown.initial = true;
        let blank = Url::parse("about:blank").expect("about:blank parses");
        Engine {
            backend: SoftwareBackend::with_fonts(fonts.clone()),
            layout_fonts: fonts,
            shown,
            history: History::new(Some(blank)),
            network,
            navigation: None,
            load_state: LoadState::Loaded,
            next_page: 1,
            goal: None,
            style_runs: 0,
            restyled_elements: 0,
            layout_runs: 0,
            boxes_laid_out: 0,
            boxes_reused: 0,
            viewport: (0, 0),
            animation_clock: None,
            stats: FrameStats::default(),
            drawn_scroll_y: 0.0,
            changed: true,
        }
    }

    /// Shows the built-in page as a document given to the engine: what
    /// the shell shows when its load fails, until M15's error pages.
    pub fn show_built_in_page(&mut self) {
        self.load_document(DOCUMENT_SOURCE);
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
        page.begin(&document, &mut self.network);
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
        self.backend.clear_glyph_cache();
        self.backend.forget_frame();
        self.drawn_scroll_y = self.shown.scroll_y;
        self.animation_clock = None;
        self.changed = true;
        left
    }

    /// Leaves `left`, the document of document state `state`: its
    /// fetches are aborted, and the back-forward cache keeps it if it is
    /// salvageable — no fetch was aborted for it, now or by a stop — and
    /// an entry still has its state.
    fn leave(&mut self, left: Shown, state: StateId) {
        let aborted = self.network.cancel(left.page.id(), None);
        if left.salvageable && !aborted && !left.initial {
            // What the cache lets go is dropped here.
            drop(self.history.keep(state, Box::new(left)));
        }
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
        let handling = if self.shown.initial || self.history.current().url.as_ref() == Some(&url) {
            Handling::Replace
        } else {
            Handling::Push
        };
        self.start_navigation(url, handling);
    }

    /// Fetches `url`'s document, for `handling` once it is shown.
    fn start_navigation(&mut self, url: Url, handling: Handling) {
        self.cancel_navigation();
        let id = self.new_page_id();
        let request = net::Request::get(url, net::Destination::Document);
        self.network.fetch(id, Target::Document, request);
        self.navigation = Some(Navigation {
            incoming: Incoming::Fetching(id),
            handling,
        });
        self.load_state = LoadState::Loading;
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
        } else if let Some(kept) = self.history.take_kept(state) {
            self.history.go_to(index);
            let left = self.show(*kept);
            self.leave(left, left_state);
            self.goal = Some(ScrollGoal::Offset(scroll));
            self.settle_load_state();
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
        while let Some(arrived) = self.network.take() {
            self.arrived(arrived);
        }
        self.needs_frame()
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
            Target::Sheet { owner, path } => {
                if page == self.shown.page.id() {
                    if self
                        .shown
                        .page
                        .sheet_arrived(owner, &path, id, result, &mut self.network)
                    {
                        self.sheets_changed();
                    }
                    self.update_load_state();
                } else if let Some(Navigation {
                    incoming: Incoming::Waiting { page: incoming, .. },
                    ..
                }) = &mut self.navigation
                    && incoming.id() == page
                {
                    incoming.sheet_arrived(owner, &path, id, result, &mut self.network);
                    self.commit_if_ready();
                }
            }
        }
    }

    /// Takes the response to the document fetch of the navigation under
    /// way: the navigation fails, or the document is parsed and its
    /// stylesheets fetched.
    fn document_arrived(&mut self, page: PageId, result: Result<net::Response, LoadError>) {
        let Some(navigation) = self.navigation.take() else {
            return;
        };
        if !matches!(navigation.incoming, Incoming::Fetching(id) if id == page) {
            self.navigation = Some(navigation);
            return;
        }
        let parsed = result.and_then(|response| {
            let parsed = loader::document_from(&response)?;
            let address_space = response.address_space.unwrap_or(net::AddressSpace::Public);
            Ok((
                parsed.document,
                Page::fetched(
                    page,
                    response.url().clone(),
                    parsed.encoding,
                    parsed.undecodable,
                    address_space,
                ),
            ))
        });
        match parsed {
            Ok((document, mut incoming)) => {
                incoming.begin(&document, &mut self.network);
                self.navigation = Some(Navigation {
                    incoming: Incoming::Waiting {
                        page: Box::new(incoming),
                        document: Box::new(document),
                    },
                    handling: navigation.handling,
                });
                self.commit_if_ready();
            }
            Err(error) => self.load_state = LoadState::Failed(error),
        }
    }

    /// Shows the incoming document once its blocking stylesheets have
    /// arrived or failed.
    fn commit_if_ready(&mut self) {
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
        }) = self.navigation.take()
        else {
            unreachable!("checked above");
        };
        self.commit(*document, *page, handling);
    }

    /// Shows a navigation's document, and puts it in the session history
    /// as `handling` says: in a new entry pushed or put in the current
    /// one's place, the document left kept or dropped; or in the entry
    /// reloaded or traversed to, at its saved scroll position.
    fn commit(&mut self, document: Document, page: Page, handling: Handling) {
        let url = page.url().cloned();
        self.history.current_mut().scroll = self.shown.scroll_target;
        let left_state = self.history.current().state;
        let mut goal = None;
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
        let left = self.show(Shown::new(document, page));
        if handling == Handling::Reload {
            // The entry's state has the new document; the old one is
            // gone, not kept.
            self.network.cancel(left.page.id(), None);
        } else {
            self.leave(left, left_state);
        }
        self.goal = goal;
        self.settle_load_state();
    }

    /// The load is done once its document is shown and every stylesheet
    /// it asked for has arrived or failed.
    fn update_load_state(&mut self) {
        if self.load_state == LoadState::Loading
            && self.navigation.is_none()
            && self.shown.page.settled()
        {
            self.load_state = LoadState::Loaded;
        }
    }

    /// The load state as the document shown and the navigation under way
    /// make it, after a document is shown or a load stopped.
    fn settle_load_state(&mut self) {
        self.load_state = if self.navigation.is_some() || !self.shown.page.settled() {
            LoadState::Loading
        } else {
            LoadState::Loaded
        };
    }

    /// Takes the stylesheets that apply as they now stand: when they
    /// differ from those the cascade has, the next frame cascades the
    /// whole document.
    fn sheets_changed(&mut self) {
        let sheets = self.shown.page.sheets();
        if sheets != self.shown.stylesheets.sheets() {
            self.shown.stylesheets = Stylesheets::new(sheets);
            self.shown.styled = None;
            self.shown.pending.clear();
            self.shown.layout = None;
            self.changed = true;
        }
    }

    /// Where the latest navigation stands. A document given to the
    /// engine, and the initial `about:blank`, are loaded.
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
        self.network.set_waker(std::sync::Arc::new(waker));
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
    /// restyles the
    /// elements the changes reach (M13.1) and lays out the boxes the
    /// changes and those restyles reach (M13.2). The scroll offsets of
    /// freed elements are dropped, and
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
        let mut sheets = None;
        if changes
            .iter()
            .any(|change| loader::may_touch_sheets(&self.shown.document, change))
        {
            self.shown
                .page
                .rescan(&self.shown.document, &mut self.network);
            self.update_load_state();
            sheets = Some(self.shown.page.sheets())
                .filter(|sheets| sheets != self.shown.stylesheets.sheets());
        }
        if let Some(sheets) = sheets {
            self.shown.stylesheets = Stylesheets::new(sheets);
            self.shown.styled = None;
            self.shown.pending.clear();
        } else if self.shown.styled.is_some()
            && self.shown.pending.first() != Some(&Change::Everything)
        {
            // Past the journal's own bound, the changes count as all.
            if self.shown.pending.len() + changes.len() > dom::MAX_JOURNAL {
                self.shown.pending = vec![Change::Everything];
            } else {
                self.shown.pending.extend(changes);
            }
        }
        self.shown.layout = None;
        let document = &self.shown.document;
        self.shown
            .element_scroll
            .retain(|node, _| document.contains(*node));
        self.changed = true;
    }

    /// Whether a frame is wanted: the document was replaced or changed,
    /// or a scroll offset set, since the last frame, or a scroll offset,
    /// the document's or a scroll container's, is on its way to its
    /// target. A new viewport size is the caller's to know: the frame
    /// takes it.
    pub fn needs_frame(&self) -> bool {
        self.changed || self.animating()
    }

    /// Whether a scroll offset is on its way to its target.
    fn animating(&self) -> bool {
        self.shown.scroll_y != self.shown.scroll_target
            || self.shown.element_scroll.values().any(|s| s.y != s.target)
    }

    /// The document's stylesheets that apply, in cascade order: those
    /// of its `<link>` and `<style>` elements in tree order, each one's
    /// imports before it.
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

    fn cascade(&self, viewport: (u32, u32)) -> Styles {
        let fonts = FontMeasurer(&self.layout_fonts);
        let context = CascadeContext {
            viewport_width: viewport.0 as f32,
            viewport_height: viewport.1 as f32,
            fonts: Some(&fonts),
        };
        self.shown
            .stylesheets
            .cascade(&self.shown.document, &context)
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
        let styles = self.cascade((viewport_width as u32, self.viewport.1));
        let boxes = layout::build_box_tree(&self.shown.document, &styles);
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
    /// none is cached, or when the viewport changed and the document's
    /// styles depend on it (M10.1); restyled when the document changed
    /// since (M13.1). Either drops the cached layout, and brings the
    /// layout cache up to date: emptied by a whole cascade, its entries
    /// the changes and the restyled elements reach forgotten by a
    /// restyle (M13.2).
    fn cached_styles(&mut self) -> &Styles {
        let stale = self.shown.styled.as_ref().is_none_or(|cached| {
            cached.styles.uses_viewport_units && cached.viewport != self.viewport
        });
        if stale {
            let styles = self.cascade(self.viewport);
            self.style_runs += 1;
            self.shown.pending.clear();
            self.shown.layout = None;
            self.shown.layout_cache.clear();
            self.shown.styled = Some(CachedStyle {
                viewport: self.viewport,
                styles,
            });
        } else if !self.shown.pending.is_empty() {
            let changes = std::mem::take(&mut self.shown.pending);
            let fonts = FontMeasurer(&self.layout_fonts);
            let context = CascadeContext {
                viewport_width: self.viewport.0 as f32,
                viewport_height: self.viewport.1 as f32,
                fonts: Some(&fonts),
            };
            let cached = self.shown.styled.as_mut().expect("not stale");
            let restyle = self.shown.stylesheets.restyle(
                &self.shown.document,
                &context,
                &mut cached.styles,
                &changes,
            );
            // Styles that did not depend on the viewport stand for this
            // one; any computed now used it.
            cached.viewport = self.viewport;
            if restyle.full {
                self.style_runs += 1;
                self.shown.layout_cache.clear();
            } else {
                self.restyled_elements += restyle.computed as u64;
                self.shown.layout_cache.invalidate(
                    &self.shown.document,
                    &changes,
                    &restyle.changed,
                );
            }
            self.shown.layout = None;
        }
        &self.shown.styled.as_ref().expect("just computed").styles
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
            let boxes = layout::build_box_tree(&self.shown.document, styles);
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

    /// Draws the frame for `time` (M13.4): the scroll animations are
    /// advanced to `time.now` — by the time since the last frame drawn
    /// while one ran, or by one frame of [`FRAME_BUDGET`] for one begun
    /// since — and the frame is drawn as [`Engine::frame`] draws it,
    /// overrunning when it ends past `time.deadline`. The pixels are lent
    /// out until the next frame; [`Engine::frame_stats`] says what it
    /// took. What it shows is a function of the document, the viewport,
    /// the scroll inputs and the times passed in.
    pub fn frame_at(&mut self, width: u32, height: u32, time: FrameTime) -> &[u8] {
        if self.animating() {
            let elapsed = self
                .animation_clock
                .map_or(FRAME_BUDGET, |then| time.now.saturating_sub(then));
            self.advance(elapsed);
        }
        self.animation_clock = self.animating().then_some(time.now);
        self.draw(width, height, time.deadline.saturating_sub(time.now))
    }

    /// Renders the current document into an RGBA8 framebuffer for a
    /// `width` x `height` viewport at the current scroll offset, with
    /// the scrollbar when the document is taller than the viewport. A
    /// viewport over `render::MAX_FRAME_PIXELS` gets an empty buffer
    /// rather than an allocation the machine cannot make. No animation
    /// is advanced: [`Engine::tick`] does that for a caller without a
    /// clock, and [`Engine::frame_at`] draws by time. The frame overruns
    /// past [`FRAME_BUDGET`].
    ///
    /// A pure function of (document, stylesheets, size, scroll): no
    /// platform state, no side effects outside the caches.
    pub fn frame(&mut self, width: u32, height: u32) -> Vec<u8> {
        self.draw(width, height, FRAME_BUDGET).to_vec()
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
        self.cached_styles();
        let style_time = start.elapsed();
        let stage = Instant::now();
        self.layout(width as f32);
        let layout_time = stage.elapsed();
        // The document may be shorter at this width than where the
        // offset was; so may a scroll container's content.
        let max = self.max_scroll();
        if let Some(ScrollGoal::Offset(y)) = self.goal {
            // Applied before painting, so that no frame shows the
            // document elsewhere first, and again at each frame until
            // the document's load is over, since a late stylesheet can
            // make the document longer.
            self.shown.scroll_target = y;
            self.shown.scroll_y = y;
            if self.shown.page.settled() {
                self.goal = None;
            }
        }
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
        let offsets: HashMap<NodeId, f32> = self
            .shown
            .element_scroll
            .iter()
            .map(|(node, scroll)| (*node, scroll.y))
            .collect();
        let stage = Instant::now();
        let viewport = Viewport {
            width: width as f32,
            height: height as f32,
            scroll_y: self.shown.scroll_y,
        };
        let scrollbar = self.scrollbar();
        if self.shown.scroll_y != self.drawn_scroll_y {
            self.backend.forget_frame();
            self.drawn_scroll_y = self.shown.scroll_y;
        }
        let layout = self.layout(width as f32);
        let mut list = paint_fragments_in(&layout.roots, viewport, &offsets);
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
        let pixels = self.backend.rasterize(&list, width, height);
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
        pixels
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
    /// ticks; the target is clamped to the document.
    pub fn scroll_by(&mut self, dy: f32) {
        let target = self.shown.scroll_target + dy;
        self.scroll_to(target);
    }

    /// Scrolls to offset `y`, animated over the next ticks; clamped to
    /// the document (so an infinite `y` means the bottom or the top),
    /// and a NaN is ignored.
    pub fn scroll_to(&mut self, y: f32) {
        if y.is_nan() {
            return;
        }
        // The user has scrolled: a position being restored is given up.
        self.goal = None;
        let max = self.max_scroll();
        self.shown.scroll_target = y.clamp(0.0, max);
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

    /// Advances the scroll animations by one frame of [`FRAME_BUDGET`],
    /// for a caller without a clock (see [`Engine::frame_at`]). Returns
    /// whether another frame is wanted (an offset has not reached its
    /// target).
    pub fn tick(&mut self) -> bool {
        self.advance(FRAME_BUDGET)
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
    /// next ticks and clamped to its content, as [`Engine::scroll_to`]
    /// does for the viewport; a NaN is ignored. An `overflow: hidden`
    /// box scrolls this way too, by program only (CSS2 §11.1.1).
    pub fn scroll_element_to(&mut self, node: NodeId, y: f32) {
        if y.is_nan() {
            return;
        }
        self.goal = None;
        let max = self.element_max_scroll(node);
        self.shown.element_scroll.entry(node).or_default().target = y.clamp(0.0, max);
    }

    /// Scrolls the element's content by `dy` px (down when positive),
    /// animated over the next ticks.
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
    /// point is compared as it is. A scrollbar step is passed over: the
    /// bar is an indicator over its container, not a control.
    fn hit(&mut self, x: f32, y: f32) -> Option<(NodeId, Vec<NodeId>)> {
        let (width, height) = self.viewport;
        if width == 0 || !x.is_finite() || !y.is_finite() {
            return None;
        }
        let offsets: HashMap<NodeId, f32> = self
            .shown
            .element_scroll
            .iter()
            .map(|(node, scroll)| (*node, scroll.y))
            .collect();
        let viewport = Viewport {
            width: width as f32,
            height: height as f32,
            scroll_y: self.shown.scroll_y,
        };
        let layout = self.layout(width as f32);
        // The clips in force, outermost first: the chain around a hit.
        let mut clips: Vec<(NodeId, Rect)> = Vec::new();
        let under = |rect: &Rect, clips: &[(NodeId, Rect)]| {
            rect.contains(x, y) && clips.last().is_none_or(|(_, clip)| clip.contains(x, y))
        };
        let chain = |clips: &[(NodeId, Rect)]| clips.iter().map(|(node, _)| *node).collect();
        let mut hit = None;
        for step in paint_order(&layout.roots, viewport, &offsets) {
            match step {
                Step::Box { fragment, rect } => {
                    if under(&rect, &clips) {
                        let mut chain: Vec<NodeId> = chain(&clips);
                        if fragment.clips() && padding_box(fragment, rect).contains(x, y) {
                            chain.push(fragment.node);
                        }
                        hit = Some((fragment.node, chain));
                    }
                }
                Step::Text { fragment, rect, .. } => {
                    if under(&rect, &clips) {
                        hit = Some((fragment.node, chain(&clips)));
                    }
                }
                Step::PushClip { node, rect } => clips.push((node, rect)),
                Step::PopClip => {
                    clips.pop();
                }
                Step::Scrollbar { .. } => {}
            }
        }
        hit
    }

    /// The `href` of the nearest `<a>` at or above the element under
    /// the viewport point, if any (M9.4's "link clicks"; following it
    /// is M15's navigation).
    pub fn link_at(&mut self, x: f32, y: f32) -> Option<String> {
        let hit = self.hit_test(x, y)?;
        let document = &self.shown.document;
        std::iter::successors(Some(hit), |&node| document.parent_element(node)).find_map(|node| {
            if document.tag(node) == Some("a") {
                document.attr(node, "href").map(str::to_string)
            } else {
                None
            }
        })
    }
}

/// The document HTML `source` parses to, or an empty one if the parser
/// refuses it.
fn parsed_or_empty(source: &str) -> Document {
    html::parse_document(source)
        .map_or_else(|_| Document::new(), |parsed| Document::from_node(&parsed))
}

/// The box fragment of the element `node`, if it has one.
fn fragment_of(fragments: &[Fragment], node: NodeId) -> Option<&Fragment> {
    for f in fragments {
        if matches!(f.kind, FragmentKind::Box) && f.node == node {
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

    /// A test engine showing the built-in page.
    fn builtin_engine() -> Engine {
        let mut engine = test_engine();
        engine.load_document(DOCUMENT_SOURCE);
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
                    layout::FragmentKind::Box | layout::FragmentKind::InlineBox => None,
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
        // M10.6: no author CSS at all. A row of controls on one line,
        // each the size the UA sheet gives it, with the engine's font
        // measuring the `ch` and the text.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 }</style>\
             <form><input id=i><input type=hidden id=h><input type=checkbox id=c>\
             <button id=b>Go</button><select id=s><option>a</option></select>\
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
        let near = |a: f32, b: f32| (a - b).abs() < 0.01;
        let fonts = &engine.layout_fonts;
        let size = 13.3333;
        // The input: 20 zeros wide inside 2px of padding and 1px of
        // border each side; 1.2em tall inside 1px and 1px.
        let input = fragment("i").expect("the input's box");
        assert!(
            near(input.w, 20.0 * fonts.zero_advance(size) + 6.0),
            "{}",
            input.w
        );
        assert!(near(input.h, 1.2 * size + 4.0), "{}", input.h);
        assert!(fragment("h").is_none(), "a hidden input makes no box");
        let checkbox = fragment("c").expect("the checkbox's box");
        assert_eq!((checkbox.w, checkbox.h), (13.0, 13.0));
        // The button fits its label: 6px of padding and 1px of border
        // each side of the word, one line tall.
        let button = fragment("b").expect("the button's box");
        assert!(
            near(button.w, fonts.measure("Go", size) + 14.0),
            "{}",
            button.w
        );
        assert!(near(
            button.h,
            fonts.primary().metrics(size).line_height() + 4.0
        ));
        let label = frags
            .iter()
            .find(|f| matches!(&f.kind, layout::FragmentKind::Text(t) if t == "Go"))
            .expect("the label is laid out");
        assert!(label.x > button.x && label.x + label.w < button.x + button.w);
        // The select is an input's size and shows none of its options.
        let select = fragment("s").expect("the select's box");
        assert_eq!((select.w, select.h), (input.w, input.h));
        assert!(
            !frags
                .iter()
                .any(|f| matches!(&f.kind, layout::FragmentKind::Text(t) if t == "a"))
        );
        // The textarea is two lines tall and keeps its line break.
        let textarea = fragment("t").expect("the textarea's box");
        assert!(near(textarea.h, 2.4 * size + 4.0), "{}", textarea.h);
        let line = |t: &str| {
            frags
                .iter()
                .find(|f| matches!(&f.kind, layout::FragmentKind::Text(x) if x == t))
                .expect("the line")
                .y
        };
        assert!(line("two") > line("one"));
        // All on one line: every control's bottom is on the baseline.
        let bottoms: Vec<f32> = ["i", "c", "b", "s", "t"]
            .iter()
            .map(|id| {
                let f = fragment(id).unwrap();
                f.y + f.h
            })
            .collect();
        assert!(
            near(bottoms[0], bottoms[3]),
            "input and select: {bottoms:?}"
        );
        assert!(
            bottoms[1] < bottoms[0],
            "the checkbox has a bottom margin: {bottoms:?}"
        );
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
        // The page is as tall as the box in flow, not where it moved to.
        assert_eq!(engine.layout(200.0).height, 50.0);
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
    fn an_inline_element_paints_its_background_and_is_hit_over_its_padding() {
        // <p><a href=/t>b</a></p>, the link with 10px of horizontal
        // padding and a red background (M11.6): its padding is red, is
        // the link for the pointer and for a click, and ends with its
        // content area, the rest of the line below being the
        // paragraph's.
        let mut engine = test_engine();
        engine.load_document(
            "<style>body { margin: 0 } p { margin: 0 }
                    a { padding: 0 10px; background-color: #ff0000 }</style>\
             <p id=p><a href=\"/t\" id=a>b</a></p>",
        );
        let pixels = engine.frame(200, 100);
        let (p, a) = (node_by_id(&engine, "p"), node_by_id(&engine, "a"));
        assert_eq!(px(&pixels, 200, 5, 8), [0xff, 0x00, 0x00, 0xff]);
        assert_eq!(engine.hit_test(5.0, 8.0), Some(a));
        assert_eq!(engine.link_at(5.0, 8.0).as_deref(), Some("/t"));
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
            engine.link_at(link.x + 1.0, link.top() + 1.0).as_deref(),
            Some("https://example.test/x")
        );
        assert_eq!(engine.link_at(plain.x + 1.0, plain.top() + 1.0), None);
        assert_eq!(engine.link_at(1.0, 90.0), None);
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
        let padding = engine.styles().get(p).expect("styled").padding;
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
        // A scroll moves everything: drawn whole, without comparing.
        engine.set_scroll(10.0);
        engine.frame(200, 30);
        assert_eq!(engine.redrawn(), &render::Redrawn::Whole);
        engine.frame(200, 30);
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

    #[test]
    fn after_random_mutations_the_engine_shows_what_a_fresh_engine_shows() {
        // Seeded runs of random mutations, each compared after every
        // step with a fresh engine's full pass over the document as it
        // stands (M13's differential check): what the engine restyles
        // and lays out again, and what it keeps, must come to what a
        // full pass computes.
        let mut fresh = test_engine();
        for (name, page) in [("first", DIFFERENTIAL_PAGE), ("nested", NESTED_PAGE)] {
            let changed: usize = (1..=8)
                .map(|seed| mutate_and_compare(&mut fresh, name, page, seed))
                .sum();
            // Enough of the steps did something, over all the runs.
            assert!(
                changed >= 8 * 15,
                "{name} page: only {changed} steps changed anything"
            );
        }
    }

    /// Fifty random mutations of `page` from `seed`, each followed by
    /// the differential check against `fresh`; returns how many changed
    /// anything.
    fn mutate_and_compare(fresh: &mut Engine, name: &str, page: &str, seed: u64) -> usize {
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
        // The layout cache was put to use.
        assert!(
            engine.boxes_reused() > 0,
            "{name} page, seed {seed}: no box reused"
        );
        changed
    }
}
