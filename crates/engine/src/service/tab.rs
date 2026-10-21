//! The session's proxy (M34.5, docs/design/processes.md, "The session's
//! proxy"): a tab whose pages renderers show, a renderer for each site
//! (M34.1, D13). It keeps what the renderers must not reach (the network
//! service with the cookie jar, the cache, the HSTS hosts and the
//! permissions, Web Storage and the profile) and gives the session what
//! it reads of the page shown as its renderer last reported it. The
//! browser never waits on a renderer: every order is a message, and
//! every answer an event taken when the session polls.
//!
//! The tab's history is the browser's, in segments (D28): runs of
//! entries one view of one renderer made, a site's that followed one
//! another. A navigation to another site starts a segment in that
//! site's renderer, whether the address bar asked for it or the network
//! service handed a page's own navigation over; going back and forward
//! across segments shows the other view again, or restores its entries
//! in a new renderer when its own is gone. At most `cap` renderers live
//! at once; the one least recently shown of those that show nothing
//! makes room.
//!
//! A frame whose document is of another site than its parent's is shown
//! by a view of its own in that site's renderer, a frame root (D33): its
//! parent draws a hole where it is, and the browser composites the
//! frame's pixels into the hole, gives the frame the pointer's events
//! that fall in it, and its keys once it is clicked.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use net::cookies::Jar;
use net::{Client, Hsts, Url};

use super::frames::{FrameEvent, FrameInfo, FramesService, SharedRegistry};
use super::handover::{HandoverNotice, Handovers};
use super::network::{FrameGrant, Grants, Lock, NetworkService};
use super::process::{Event, Gone, Launcher, Renderer, Services};
use super::storage::StorageService;
use crate::frames::HOLE;
use crate::ipc::{Ancestor, Hole, Input, Settings, ToBrowser, ToRenderer, ViewState};
use crate::permissions::Permissions;
use crate::renderer::MAX_SURFACE_PIXELS;
use crate::storage::Storage;
use crate::view::View;
use crate::{
    Button, Clock, ConsoleEntry, Cursor, Engine, FrameTime, Key, KeyOutcome, Link, LoadError,
    LoadState, Modifiers, PermissionPrompt, Preferences, Profile, Scrollbar,
};

/// The most renderers a tab has alive at once, the spare among them
/// (docs/design/processes.md, "The cap and eviction").
pub const MAX_RENDERERS: usize = 16;

/// A view of a renderer: the renderer's number and the view's.
type ViewId = (u32, u32);

/// A renderer the tab holds, and the site it is locked to.
struct Slot {
    renderer: Renderer,
    lock: Option<Lock>,
    /// When one of its views was last shown.
    shown: Instant,
}

/// A run of the tab's history in one view.
struct Segment {
    /// Its renderer, while it lives.
    renderer: Option<u32>,
    view: u32,
    frame: u32,
    lock: Lock,
    /// Its entries' URLs and the current one, as its renderer last said.
    entries: Vec<Url>,
    index: usize,
    state: ViewState,
}

/// A frame root's view: the frame the browser numbers so, and what its
/// renderer last said of it.
struct FrameView {
    key: u32,
    state: ViewState,
    /// The URL of the document whose load its parent was last told of.
    told: Option<Url>,
    /// The raster size it was last given.
    raster: (u32, u32),
}

impl FrameView {
    /// The view made for frame `key`'s navigation handed over. Its
    /// initial `about:blank`, which it may say it loaded before it takes
    /// the navigation, is never told: no `load` for it when a navigation
    /// follows (HTML's "process the iframe attributes").
    fn handed_over(key: u32) -> FrameView {
        FrameView {
            key,
            state: ViewState::default(),
            told: Url::parse("about:blank").ok(),
            raster: (0, 0),
        }
    }

    /// Whether its parent is to be told of the load `state` says: one
    /// done, of a document it was not told of.
    fn tells(&self, state: &ViewState) -> bool {
        let loaded = matches!(state.load, LoadState::Loaded | LoadState::Failed(_));
        loaded && state.url.is_some() && self.told != state.url
    }
}

/// A view's frames as they come: the frame being received, the last
/// whole one with its size, and where the frames of other renderers are
/// in it.
#[derive(Default)]
struct Buffer {
    incoming: Vec<u8>,
    pixels: Vec<u8>,
    size: (u32, u32),
    holes: Vec<Hole>,
}

impl Buffer {
    /// The pixel at `(x, y)`, if the frame has one there.
    fn pixel(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        pixel_in(&self.pixels, self.size, x, y)
    }
}

/// The pixel at `(x, y)` of a frame of `size`, if it has one there.
fn pixel_in(pixels: &[u8], size: (u32, u32), x: i32, y: i32) -> Option<[u8; 4]> {
    let (w, h) = (size.0 as i32, size.1 as i32);
    if x < 0 || y < 0 || x >= w || y >= h {
        return None;
    }
    let at = (y as usize * w as usize + x as usize) * 4;
    pixels.get(at..at + 4)?.try_into().ok()
}

/// The browser's page saying a frame's renderer stopped (M34.4), shown
/// in the frame's hole, and its pixels at the hole's size.
struct Stopped {
    engine: Engine,
    pixels: Vec<u8>,
    size: (u32, u32),
}

/// Why a renderer went, for the page saying so: how, and its exit code
/// when its process has one yet.
fn stopped_why(why: &Gone, code: Option<i64>) -> String {
    let how = match why {
        Gone::Ended => "its process ended".to_owned(),
        Gone::BadMessage(what) => format!("it sent what it may not ({what}) and was stopped"),
        Gone::NoHello => "it did not start".to_owned(),
        Gone::Shutdown => "the browser ended it".to_owned(),
    };
    match code {
        Some(code) if code >= 0 => format!("{how}, with exit code {code:#x}"),
        Some(code) => format!("{how}, with exit code {code}"),
        None => how,
    }
}

/// A tab whose pages renderers show.
pub struct RemoteTab {
    fonts: text::FontCollection,
    launcher: Arc<dyn Launcher>,
    grants: Arc<Grants>,
    network: Arc<NetworkService>,
    handovers: Arc<Handovers>,
    registry: SharedRegistry,
    services: Arc<Services>,
    storage: Arc<Mutex<Storage>>,
    jar: Arc<Mutex<Jar>>,
    cache: Arc<net::Cache>,
    hsts: Arc<Mutex<Hsts>>,
    permissions: Arc<Permissions>,
    renderers: BTreeMap<u32, Slot>,
    /// What the tab shares with the other tabs of its profile (M37): its
    /// renderers are numbered by it. The tab's number in it, and its own
    /// as one that hears of changes of Web Storage areas.
    context: crate::Context,
    tab: u64,
    listener: u64,
    next_view: u32,
    /// The most renderers alive at once, and whether one waits, unlocked,
    /// for the next site.
    cap: usize,
    spare: bool,
    segments: Vec<Segment>,
    current: usize,
    frame_views: HashMap<ViewId, FrameView>,
    buffers: HashMap<ViewId, Buffer>,
    events: mpsc::Receiver<(u32, Event)>,
    sender: mpsc::Sender<(u32, Event)>,
    handed: mpsc::Receiver<HandoverNotice>,
    frame_events: mpsc::Receiver<FrameEvent>,
    /// What wakes the session's loop when a renderer says something: the
    /// session's waker once given.
    waker: Arc<Mutex<Option<crate::loader::Waker>>>,
    clock: Option<Clock>,
    settings: Settings,
    /// The page's viewport in CSS pixels, and the raster size its frames
    /// are drawn at, as last sent.
    viewport: (u32, u32),
    raster: (u32, u32),
    /// What the session reads when no segment shows anything.
    blank: ViewState,
    undecodable: Option<&'static str>,
    /// What `frame_at` returns when the frame shown is not the page's
    /// alone, or not of the size asked.
    out: Vec<u8>,
    fresh: bool,
    /// How many frames came: for the tests of the renderer's pace.
    frames: u64,
    /// The view the pointer was last over, and the one a press was in
    /// while its button is held, with its offset in the page.
    pointer_view: Option<ViewId>,
    captured: Option<(ViewId, (f32, f32))>,
    /// The view the keys go to: the one last pressed in.
    focused_view: Option<ViewId>,
    console: Vec<ConsoleEntry>,
    notice: Option<String>,
    focus_left: bool,
    copied: Option<(String, bool)>,
    /// The page saying a segment's renderer stopped (M34.4), by the
    /// segment's index; and the frames' whose renderer did, by their
    /// numbers.
    stopped: Option<(usize, Engine)>,
    stopped_frames: HashMap<u32, Stopped>,
    /// Each time a frame's parent was told it loaded, and why: for the
    /// tests.
    #[cfg(test)]
    told_loaded: Vec<String>,
    /// Whether the frames that come are read but not taken, and the last
    /// of them, taken once they are no longer held: for the tests of the
    /// renderer's pace.
    #[cfg(test)]
    holding: bool,
    #[cfg(test)]
    held: Option<(ViewId, u64)>,
    /// How many changes of Web Storage areas it told its renderers: for
    /// the tests.
    #[cfg(test)]
    relayed: u64,
    /// Whether the tab shows the DevTools panel (M36), and what its page
    /// sent the browser; or, a page's tab, what its page answered the
    /// panel. Each kind is taken from its own kind of tab alone.
    panel: bool,
    devtools: Vec<String>,
    inspected: Vec<String>,
    /// The tab's network log (M36.3), which its client tells.
    log: Arc<crate::network_log::NetworkLog>,
    /// The tabs its pages asked for (M37), whether its page asked to close
    /// it, and whether a script opened it, which lets it.
    opened: Vec<crate::frames::OpenTab>,
    close_asked: bool,
    opened_by_script: bool,
}

/// The most tabs a tab's pages may have asked for that the browser has
/// not yet opened (M37).
const MAX_OPENING: usize = 16;

impl RemoteTab {
    /// A tab whose renderers `launcher` starts, its own interface drawn
    /// with `fonts`, at most [`MAX_RENDERERS`] of them, one spare.
    pub fn new(launcher: Arc<dyn Launcher>, fonts: text::FontCollection) -> RemoteTab {
        RemoteTab::with_cap(launcher, fonts, MAX_RENDERERS, true)
    }

    /// A tab of `context` (M37), sharing its cookies, cache, HSTS hosts,
    /// permissions and storage with the context's other tabs.
    pub fn in_context(
        launcher: Arc<dyn Launcher>,
        fonts: text::FontCollection,
        context: &crate::Context,
    ) -> RemoteTab {
        RemoteTab::in_context_with_cap(launcher, fonts, MAX_RENDERERS, true, context)
    }

    /// A tab of at most `cap` renderers, one of them spare if `spare`.
    pub fn with_cap(
        launcher: Arc<dyn Launcher>,
        fonts: text::FontCollection,
        cap: usize,
        spare: bool,
    ) -> RemoteTab {
        RemoteTab::in_context_with_cap(launcher, fonts, cap, spare, &crate::Context::new())
    }

    /// A tab of `context` of at most `cap` renderers, one of them spare if
    /// `spare`.
    pub fn in_context_with_cap(
        launcher: Arc<dyn Launcher>,
        fonts: text::FontCollection,
        cap: usize,
        spare: bool,
        context: &crate::Context,
    ) -> RemoteTab {
        let grants = Grants::new();
        let jar: Arc<Mutex<Jar>> = context.jar().clone();
        let cache: Arc<net::Cache> = context.cache().clone();
        let hsts: Arc<Mutex<Hsts>> = context.hsts().clone();
        let permissions: Arc<Permissions> = context.permissions().clone();
        let log: Arc<crate::network_log::NetworkLog> = Arc::default();
        let storage = context.shared_storage().clone();
        let tab = context.new_tab();
        let handovers = Handovers::new();
        let registry: SharedRegistry = Arc::default();
        let network = NetworkService::new(
            client(
                &jar,
                &cache,
                &hsts,
                &permissions,
                &log,
                context.accept_language(),
                false,
            ),
            grants.clone(),
        );
        network.hand_over_with(handovers.clone());
        let waker: Arc<Mutex<Option<crate::loader::Waker>>> = Arc::default();
        let wake = waking(&waker);
        let (sender, events) = mpsc::channel();
        let (notify, handed) = mpsc::channel();
        let (frame_sender, frame_events) = mpsc::channel();
        let notify = Mutex::new(notify);
        let notified = wake.clone();
        network.on_handover(Arc::new(move |notice| {
            let _ = notify
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .send(notice);
            notified();
        }));
        let (asked, seen) = (registry.clone(), registry.clone());
        network.watch_frames(
            Arc::new(move |key| {
                super::frames::lock(&asked)
                    .frames
                    .get(&key)
                    .is_some_and(|info| info.root.is_some())
            }),
            Arc::new(move |renderer, key, url, _| {
                let mut registry = super::frames::lock(&seen);
                if let Some(info) = registry.frames.get_mut(&key)
                    && super::frames::lock_matches(info, renderer)
                {
                    info.origin = Some(url.origin());
                    info.url = Some(url.clone());
                }
            }),
        );
        let listener = {
            let mut shared = storage.lock().unwrap_or_else(PoisonError::into_inner);
            let listener = shared.listen();
            shared.set_waker(listener, wake.clone());
            listener
        };
        // Downloads saved by the browser (M37), of a frame whose sandbox
        // lets it start them.
        let sandboxes = registry.clone();
        network.save_downloads(
            context.downloads().clone(),
            Arc::new(move |key| {
                super::frames::lock(&sandboxes)
                    .frames
                    .get(&key)
                    .is_none_or(|info| {
                        !crate::sandbox::Sandbox::from_bits(info.sandbox)
                            .has(crate::sandbox::Sandbox::DOWNLOADS)
                    })
            }),
        );
        let services = Arc::new(Services(vec![
            network.clone(),
            StorageService::new(storage.clone(), grants.clone(), tab),
            FramesService::new(registry.clone(), grants.clone(), frame_sender, wake),
        ]));
        RemoteTab {
            fonts,
            launcher,
            grants,
            network,
            handovers,
            registry,
            services,
            storage,
            jar,
            cache,
            hsts,
            permissions,
            renderers: BTreeMap::new(),
            context: context.clone(),
            tab,
            listener,
            next_view: 1,
            cap: cap.max(1),
            spare,
            segments: Vec::new(),
            current: 0,
            frame_views: HashMap::new(),
            buffers: HashMap::new(),
            events,
            sender,
            handed,
            frame_events,
            waker,
            clock: None,
            settings: Settings::default(),
            viewport: (0, 0),
            raster: (0, 0),
            blank: ViewState::default(),
            undecodable: None,
            out: Vec::new(),
            fresh: false,
            frames: 0,
            pointer_view: None,
            captured: None,
            focused_view: None,
            console: Vec::new(),
            notice: None,
            focus_left: false,
            copied: None,
            stopped: None,
            stopped_frames: HashMap::new(),
            #[cfg(test)]
            told_loaded: Vec::new(),
            #[cfg(test)]
            holding: false,
            #[cfg(test)]
            held: None,
            #[cfg(test)]
            relayed: 0,
            panel: false,
            devtools: Vec::new(),
            inspected: Vec::new(),
            log,
            opened: Vec::new(),
            close_asked: false,
            opened_by_script: false,
        }
    }

    /// A tab of one renderer showing the DevTools panel (M36), on this
    /// tab's launcher, fonts, waker, clock and settings, its scripts on.
    pub fn devtools_panel(&self) -> RemoteTab {
        let mut panel = RemoteTab::with_cap(self.launcher.clone(), self.fonts.clone(), 1, false);
        panel.panel = true;
        panel.settings = self.settings.clone();
        panel.settings.scripting = true;
        panel.clock = self.clock.clone();
        let waker = self
            .waker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        *panel.waker.lock().unwrap_or_else(PoisonError::into_inner) = waker;
        panel.navigate(Url::parse(devtools::PANEL_URL).expect("the panel's URL parses"));
        panel
    }

    /// The state the session reads: the current segment's.
    fn state(&self) -> &ViewState {
        self.segments
            .get(self.current)
            .map_or(&self.blank, |segment| &segment.state)
    }

    /// The state of the view under the pointer: a frame root's, or the
    /// page's.
    fn pointed_state(&self) -> &ViewState {
        self.pointer_view
            .and_then(|view| self.frame_views.get(&view))
            .map_or_else(|| self.state(), |frame| &frame.state)
    }

    fn segment(&self) -> Option<&Segment> {
        self.segments.get(self.current)
    }

    /// The current segment's view, if its renderer lives.
    fn shown_view(&self) -> Option<ViewId> {
        let segment = self.segment()?;
        let renderer = segment.renderer?;
        self.renderers
            .contains_key(&renderer)
            .then_some((renderer, segment.view))
    }

    /// The renderer of the current segment, and its view.
    fn shown(&self) -> Option<(&Renderer, u32)> {
        let (renderer, view) = self.shown_view()?;
        Some((&self.renderers.get(&renderer)?.renderer, view))
    }

    fn send(&self, message: impl FnOnce(u32) -> ToRenderer) {
        if let Some((renderer, view)) = self.shown() {
            let _ = renderer.send(&message(view));
        }
    }

    fn send_to(&self, (renderer, view): ViewId, message: impl FnOnce(u32) -> ToRenderer) {
        if let Some(slot) = self.renderers.get(&renderer) {
            let _ = slot.renderer.send(&message(view));
        }
    }

    /// `input` for the view the keys go to: the one last pressed in, or
    /// the page's.
    fn input(&self, input: Input) {
        match self
            .focused_view
            .filter(|view| self.frame_views.contains_key(view))
        {
            Some(view) => self.send_to(view, |view| ToRenderer::Input { view, input }),
            None => self.send(|view| ToRenderer::Input { view, input }),
        }
    }

    fn send_settings(&self) {
        let settings = self.settings.clone();
        self.send(|view| ToRenderer::Settings { view, settings });
        let mut framed = self.settings.clone();
        for (view, frame) in &self.frame_views {
            framed.focused = self.settings.focused && self.focused_view == Some(*view);
            let settings = framed.clone();
            let _ = frame;
            self.send_to(*view, |view| ToRenderer::Settings { view, settings });
        }
    }

    fn send_size(&self) {
        let (viewport, raster) = (self.viewport, self.raster);
        self.send(|view| ToRenderer::Resize {
            view,
            width: viewport.0,
            height: viewport.1,
            raster_width: raster.0,
            raster_height: raster.1,
        });
    }

    /// The wake the renderers' events call.
    fn wake(&self) -> crate::loader::Waker {
        waking(&self.waker)
    }

    /// Starts a renderer, room made for it; its number.
    fn launch(&mut self) -> Option<u32> {
        if self.renderers.len() >= self.cap && !self.evict() {
            self.notice = Some(format!(
                "No room for another renderer: {} are open",
                self.renderers.len()
            ));
            return None;
        }
        let number = self.context.new_renderer();
        let serve: Arc<dyn super::Serve> = self.services.clone();
        match Renderer::start(
            self.launcher.as_ref(),
            number,
            self.sender.clone(),
            self.wake(),
            Some(serve),
        ) {
            Ok(renderer) => {
                // The files of the browser's fonts, which the renderer
                // reads rather than scanning the system's (M35.6).
                if let Some(files) = self.fonts.files() {
                    let _ = renderer.send(&ToRenderer::Fonts {
                        primary: files.primary,
                        fallbacks: files.fallbacks,
                    });
                }
                super::frames::lock(&self.registry).reach(number, renderer.sender());
                self.renderers.insert(
                    number,
                    Slot {
                        renderer,
                        lock: None,
                        shown: Instant::now(),
                    },
                );
                Some(number)
            }
            Err(error) => {
                self.notice = Some(format!("No renderer could be started: {error}"));
                None
            }
        }
    }

    /// Shuts down the renderer least recently shown of those that show
    /// nothing now; whether there was one.
    fn evict(&mut self) -> bool {
        let shown = self.segment().and_then(|segment| segment.renderer);
        let framing: Vec<u32> = self
            .frame_views
            .keys()
            .map(|(renderer, _)| *renderer)
            .collect();
        let idle = self
            .renderers
            .iter()
            .filter(|(number, _)| Some(**number) != shown && !framing.contains(number))
            .min_by_key(|(_, slot)| (slot.lock.is_some(), slot.shown))
            .map(|(number, _)| *number);
        let Some(number) = idle else {
            return false;
        };
        self.forget(number);
        true
    }

    /// Lets go of renderer `number`: its segments keep their entries, to
    /// be restored in another.
    fn forget(&mut self, number: u32) {
        if let Some(slot) = self.renderers.remove(&number) {
            slot.renderer.shutdown();
        }
        self.left(number);
    }

    /// What renderer `number` showed is gone with it. A frame it showed
    /// stays another renderer's than its parent's: its next document is
    /// handed over to a new view.
    fn left(&mut self, number: u32) {
        self.grants.forget(number);
        for segment in &mut self.segments {
            if segment.renderer == Some(number) {
                segment.renderer = None;
            }
        }
        self.buffers.retain(|(renderer, _), _| *renderer != number);
        let gone: Vec<ViewId> = self
            .frame_views
            .keys()
            .filter(|(renderer, _)| *renderer == number)
            .copied()
            .collect();
        super::frames::lock(&self.registry).unreach(number);
        for view in gone {
            self.frame_views.remove(&view);
        }
    }

    /// The renderer for `lock`'s documents: the one locked to it, the
    /// spare, or a new one; a spare is started for the next site.
    fn renderer_for(&mut self, lock: &Lock) -> Option<u32> {
        let locked = self
            .renderers
            .iter()
            .find(|(_, slot)| slot.lock.as_ref() == Some(lock))
            .map(|(number, _)| *number);
        let number = match locked {
            Some(number) => number,
            None => {
                let spare = self
                    .renderers
                    .iter()
                    .find(|(_, slot)| slot.lock.is_none())
                    .map(|(number, _)| *number);
                let number = match spare {
                    Some(number) => number,
                    None => self.launch()?,
                };
                let slot = self.renderers.get_mut(&number)?;
                slot.lock = Some(lock.clone());
                self.grants.set_lock(number, lock.clone());
                number
            }
        };
        self.start_spare();
        Some(number)
    }

    /// Starts a spare renderer for the next site when the tab keeps one
    /// and has none, once a frame is shown (M35.6): not while the first
    /// page waits for its renderer to start.
    fn start_spare(&mut self) {
        if self.spare
            && !self.settings.hidden
            && self.frames > 0
            && self.renderers.values().all(|slot| slot.lock.is_some())
            && self.renderers.len() < self.cap
        {
            self.launch();
        }
    }

    /// A number for a tab's top-level frame in renderer `number`.
    fn top_frame(&mut self, number: u32) -> u32 {
        let mut registry = super::frames::lock(&self.registry);
        let key = registry.number();
        registry.frames.insert(
            key,
            FrameInfo {
                renderer: number,
                parent: None,
                origin: None,
                url: None,
                sandbox: 0,
                root: None,
            },
        );
        drop(registry);
        self.grants.grant_frame(
            number,
            key,
            FrameGrant {
                top: net::Origin::new_opaque(),
                cross_site: false,
            },
        );
        key
    }

    /// A new view in `lock`'s renderer, after the current segment, which
    /// becomes the current one; whether there is one.
    fn new_segment(&mut self, lock: Lock) -> bool {
        let Some(number) = self.renderer_for(&lock) else {
            return false;
        };
        let view = self.next_view;
        self.next_view += 1;
        let frame = self.top_frame(number);
        let renderer = &self.renderers[&number].renderer;
        let _ = renderer.send(&ToRenderer::CreateView {
            view,
            frame,
            settings: self.settings.clone(),
        });
        // The segments after the current one go, and its own entries
        // after its current one.
        if self.segments.get(self.current).is_some() {
            if let Some((renderer, view)) = self.shown() {
                let _ = renderer.send(&ToRenderer::DropForward { view });
            }
            self.hide();
            self.segments.truncate(self.current + 1);
            if let Some(left) = self.segments.last_mut() {
                left.entries.truncate(left.index + 1);
            }
        }
        self.segments.push(Segment {
            renderer: Some(number),
            view,
            frame,
            lock,
            entries: Vec::new(),
            index: 0,
            state: ViewState::default(),
        });
        self.current = self.segments.len() - 1;
        self.shown_now();
        true
    }

    /// The current segment's view stops being shown.
    fn hide(&self) {
        let mut settings = self.settings.clone();
        settings.hidden = true;
        self.send(|view| ToRenderer::Settings { view, settings });
    }

    /// The current segment's view is shown: its settings, its size, and a
    /// frame wanted from it.
    fn shown_now(&mut self) {
        if let Some(number) = self.segment().and_then(|segment| segment.renderer)
            && let Some(slot) = self.renderers.get_mut(&number)
        {
            slot.shown = Instant::now();
        }
        self.focused_view = None;
        self.pointer_view = None;
        self.captured = None;
        self.send_settings();
        self.send_size();
        self.undecodable = undecodable(self.state());
        self.fresh = false;
    }

    /// Shows segment `index`: its view again, or its entries restored in
    /// a renderer of its site when its own is gone.
    fn switch_to(&mut self, index: usize) {
        if index >= self.segments.len() || index == self.current {
            return;
        }
        self.hide();
        self.current = index;
        let alive = self.segments[index]
            .renderer
            .is_some_and(|number| self.renderers.contains_key(&number));
        if !alive {
            self.stopped = None;
            let lock = self.segments[index].lock.clone();
            let Some(number) = self.renderer_for(&lock) else {
                return;
            };
            let view = self.next_view;
            self.next_view += 1;
            let frame = self.top_frame(number);
            let segment = &mut self.segments[index];
            segment.renderer = Some(number);
            segment.view = view;
            segment.frame = frame;
            segment.state.load = LoadState::Loading;
            let renderer = &self.renderers[&number].renderer;
            let _ = renderer.send(&ToRenderer::CreateView {
                view,
                frame,
                settings: self.settings.clone(),
            });
            let _ = renderer.send(&ToRenderer::Restore {
                view,
                entries: segment.entries.clone(),
                index: segment.index as u32,
            });
        }
        self.shown_now();
    }

    /// The time on the session's clock.
    fn now(&self) -> Duration {
        self.clock.as_ref().map_or(Duration::ZERO, |clock| clock())
    }

    /// Takes what the renderers and the services said; whether anything
    /// changed.
    fn take_events(&mut self) -> bool {
        let mut changed = self.relay_storage_changes();
        while let Ok(notice) = self.handed.try_recv() {
            changed = true;
            self.hand_over(notice);
        }
        while let Ok(event) = self.frame_events.try_recv() {
            changed = true;
            match event {
                FrameEvent::Dropped(key) => self.drop_frame(key),
            }
        }
        while let Ok((number, event)) = self.events.try_recv() {
            if !self.renderers.contains_key(&number) {
                continue;
            }
            changed = true;
            match event {
                Event::Ready => {}
                Event::Message(message) => {
                    if let Err(why) = self.take(number, *message)
                        && let Some(slot) = self.renderers.get(&number)
                    {
                        slot.renderer.bad_message(why);
                    }
                }
                Event::Gone(why) => self.gone(number, why),
            }
        }
        changed
    }

    /// A document of another site than its renderer's was handed over:
    /// the current segment's top-level one starts a segment of that site,
    /// a frame's goes to a frame root in that site's renderer.
    fn hand_over(&mut self, notice: HandoverNotice) {
        let current = self
            .segment()
            .and_then(|segment| Some((segment.renderer?, segment.frame)));
        if current == Some((notice.renderer, notice.frame)) {
            self.hand_over_top(notice);
            return;
        }
        let framed = super::frames::lock(&self.registry)
            .frames
            .get(&notice.frame)
            .is_some_and(|info| {
                info.parent.is_some()
                    && (info.renderer == notice.renderer
                        || info
                            .root
                            .is_some_and(|(renderer, _)| renderer == notice.renderer))
            });
        if framed {
            self.hand_over_frame(notice);
        }
    }

    fn hand_over_top(&mut self, notice: HandoverNotice) {
        if !self.new_segment(notice.lock.clone()) {
            return;
        }
        let segment = &self.segments[self.current];
        let (number, view, frame) = (segment.renderer, segment.view, segment.frame);
        let Some(number) = number else {
            return;
        };
        self.handovers.assign(notice.handover, number, frame);
        if let Some(slot) = self.renderers.get(&number) {
            let _ = slot.renderer.send(&ToRenderer::NavigateHandedOver {
                view,
                url: notice.url,
                handover: notice.handover,
            });
        }
    }

    /// Frame `notice.frame`'s document goes to a view of its own in the
    /// renderer of its site: the frame's view there if it has one, else a
    /// new one, its parent told it is another renderer's.
    fn hand_over_frame(&mut self, notice: HandoverNotice) {
        let key = notice.frame;
        let (root, parent_renderer, sandbox) = {
            let registry = super::frames::lock(&self.registry);
            let Some(info) = registry.frames.get(&key) else {
                return;
            };
            (info.root, info.renderer, info.sandbox)
        };
        let reuse = root.filter(|(renderer, _)| {
            self.renderers
                .get(renderer)
                .is_some_and(|slot| slot.lock.as_ref() == Some(&notice.lock))
        });
        let view = match reuse {
            Some(view) => view,
            None => {
                if let Some(old) = root {
                    self.destroy_view(old);
                }
                let Some(number) = self.renderer_for(&notice.lock) else {
                    // No renderer for it (G118): it stays empty, and its
                    // parent's load waits for it no more.
                    if root.is_none()
                        && let Some(slot) = self.renderers.get(&parent_renderer)
                    {
                        let _ = slot.renderer.send(&ToRenderer::FrameRemote { key });
                        let _ = slot.renderer.send(&ToRenderer::FrameLoaded { key });
                        #[cfg(test)]
                        self.told_loaded.push(format!("{key}: no renderer"));
                    }
                    return;
                };
                let view = (number, self.next_view);
                self.next_view += 1;
                let (ancestors, grant) = self.place_of(key, &notice.url);
                self.grants.grant_frame(number, key, grant);
                let mut settings = self.settings.clone();
                settings.focused = false;
                self.send_to(view, |id| ToRenderer::CreateFrameView {
                    view: id,
                    key,
                    settings,
                    ancestors,
                    sandbox,
                });
                self.frame_views.insert(view, FrameView::handed_over(key));
                if let Some(info) = super::frames::lock(&self.registry).frames.get_mut(&key) {
                    info.root = Some(view);
                }
                self.stopped_frames.remove(&key);
                // A hole already drawn for the frame sizes its new view.
                let hole = self
                    .buffers
                    .values()
                    .flat_map(|buffer| buffer.holes.iter())
                    .find(|hole| hole.key == key)
                    .cloned();
                if let Some(hole) = hole {
                    self.size_frame(&hole);
                }
                if root.is_none()
                    && let Some(slot) = self.renderers.get(&parent_renderer)
                {
                    let _ = slot.renderer.send(&ToRenderer::FrameRemote { key });
                }
                view
            }
        };
        self.handovers.assign(notice.handover, view.0, key);
        self.send_to(view, |id| ToRenderer::NavigateHandedOver {
            view: id,
            url: notice.url,
            handover: notice.handover,
        });
    }

    /// Frame `key`'s ancestors as its view is told of them, and its place
    /// in the tab for a document of `url`: the top-level document's
    /// origin, and whether it or an ancestor is of another site.
    fn place_of(&self, key: u32, url: &Url) -> (Vec<Ancestor>, FrameGrant) {
        let registry = super::frames::lock(&self.registry);
        let ancestors = registry.ancestors(key);
        let top = ancestors
            .last()
            .map_or_else(net::Origin::new_opaque, |(_, origin, _)| origin.clone());
        let origin = url.origin();
        let cross_site = !top.same_site(&origin)
            || ancestors
                .iter()
                .rev()
                .skip(1)
                .any(|(_, ancestor, _)| !top.same_site(ancestor));
        (
            ancestors
                .into_iter()
                .map(|(key, origin, url)| Ancestor { key, origin, url })
                .collect(),
            FrameGrant { top, cross_site },
        )
    }

    /// A frame root's view goes, and what its renderer showed of it.
    fn destroy_view(&mut self, view: ViewId) {
        self.send_to(view, |id| ToRenderer::DestroyView { view: id });
        if let Some(frame) = self.frame_views.remove(&view) {
            self.grants.revoke_frame(view.0, frame.key);
        }
        self.buffers.remove(&view);
        if self.focused_view == Some(view) {
            self.focused_view = None;
        }
        if self.pointer_view == Some(view) {
            self.pointer_view = None;
        }
        if self.captured.is_some_and(|(captured, _)| captured == view) {
            self.captured = None;
        }
    }

    /// Frame `key` left its parent's page: its view and its frames' go.
    fn drop_frame(&mut self, key: u32) {
        let gone: Vec<(u32, Option<ViewId>)> = {
            let registry = super::frames::lock(&self.registry);
            registry
                .subtree(key)
                .into_iter()
                .map(|key| (key, registry.frames.get(&key).and_then(|info| info.root)))
                .collect()
        };
        for (key, root) in gone {
            if let Some(view) = root {
                self.destroy_view(view);
            }
            self.stopped_frames.remove(&key);
            super::frames::lock(&self.registry).frames.remove(&key);
        }
    }

    /// The browser's page saying the renderer of `url`'s document stopped,
    /// and why, in an engine of the browser's own.
    fn stopped_page(&self, url: Option<Url>, why: &str, dump: Option<&str>) -> Engine {
        let mut engine = Engine::with_fonts(self.fonts.clone());
        engine.set_preferences(self.settings.preferences);
        if self.settings.scale.is_finite() && self.settings.scale > 0.0 {
            engine.set_scale_factor(self.settings.scale);
        }
        engine.show_stopped(url, why, dump);
        engine
    }

    /// Renderer `number` is gone (M34.4, D34): its segments wait to be
    /// restored, and the page it showed says why, as do the frames it
    /// showed under other renderers' pages, whose parents' `load` waits
    /// for them no more; the frames of its pages that other renderers
    /// showed go with them.
    fn gone(&mut self, number: u32, why: Gone) {
        let showed = self.segment().and_then(|segment| segment.renderer) == Some(number);
        let (code, dump) = self
            .renderers
            .get(&number)
            .map(|slot| (slot.renderer.exit_code(), slot.renderer.dump()))
            .unwrap_or_default();
        self.renderers.remove(&number);
        let orphans: Vec<u32> = super::frames::lock(&self.registry)
            .frames
            .iter()
            .filter(|(_, info)| info.renderer == number && info.parent.is_some())
            .map(|(key, _)| *key)
            .collect();
        for key in orphans {
            self.drop_frame(key);
        }
        let shown_here: Vec<(u32, Option<Url>, bool)> = self
            .frame_views
            .iter()
            .filter(|((renderer, _), _)| *renderer == number)
            .map(|(_, frame)| {
                let loading = !matches!(frame.state.load, LoadState::Loaded | LoadState::Failed(_));
                (frame.key, frame.state.url.clone(), loading)
            })
            .collect();
        self.left(number);
        if why == Gone::Shutdown {
            return;
        }
        let said = stopped_why(&why, code);
        let dump = dump.map(|path| path.display().to_string());
        for (key, url, loading) in shown_here {
            let parent = super::frames::lock(&self.registry)
                .frames
                .get(&key)
                .map(|info| info.renderer);
            let Some(parent) = parent else {
                continue;
            };
            let engine = self.stopped_page(url, &said, dump.as_deref());
            self.stopped_frames.insert(
                key,
                Stopped {
                    engine,
                    pixels: Vec::new(),
                    size: (0, 0),
                },
            );
            if loading && let Some(slot) = self.renderers.get(&parent) {
                let _ = slot.renderer.send(&ToRenderer::FrameLoaded { key });
                #[cfg(test)]
                self.told_loaded
                    .push(format!("{key}: its renderer stopped"));
            }
            self.fresh = true;
        }
        if showed {
            let url = self.segments.get_mut(self.current).and_then(|segment| {
                segment.state.load = LoadState::Failed(LoadError::Lost);
                segment.state.url.clone()
            });
            self.notice = Some(match &dump {
                Some(dump) => {
                    format!("The page's renderer stopped: {said}; a record of it is in {dump}")
                }
                None => format!("The page's renderer stopped: {said}"),
            });
            let mut engine = self.stopped_page(url, &said, dump.as_deref());
            engine.set_viewport(self.viewport.0.max(1), self.viewport.1.max(1));
            self.stopped = Some((self.current, engine));
            self.fresh = true;
        }
    }

    /// The segment of view `view` of renderer `number`; Err for a view
    /// the renderer was not given.
    fn segment_of(&self, number: u32, view: u32) -> Result<usize, String> {
        self.segments
            .iter()
            .position(|segment| segment.renderer == Some(number) && segment.view == view)
            .ok_or_else(|| format!("view {view}, not the renderer's"))
    }

    /// Takes a message of renderer `number`'s; Err for one it may not
    /// send.
    fn take(&mut self, number: u32, message: ToBrowser) -> Result<(), String> {
        let view_of = |message: &ToBrowser| match message {
            ToBrowser::State { view, .. }
            | ToBrowser::History { view, .. }
            | ToBrowser::Pixels { view, .. }
            | ToBrowser::Frame { view, .. }
            | ToBrowser::Holes { view, .. }
            | ToBrowser::Console { view, .. }
            | ToBrowser::Notice { view, .. }
            | ToBrowser::FocusLeft { view }
            | ToBrowser::Copied { view, .. }
            | ToBrowser::OpenTab { view, .. }
            | ToBrowser::CloseTab { view } => Some(*view),
            // The panel's messages from the panel's tab, the answers to it
            // from a page's (M36).
            ToBrowser::DevTools { view, .. } => Some(*view),
            ToBrowser::Inspected { view, .. } if !self.panel => Some(*view),
            _ => None,
        };
        let Some(view) = view_of(&message) else {
            return Err(format!("{message:?} out of place"));
        };
        // The panel's messages, or those of the browser's own pages (M37),
        // from a renderer locked to them alone; any other's is a lie.
        if matches!(message, ToBrowser::DevTools { .. })
            && !self.panel
            && !self
                .segment_of(number, view)
                .is_ok_and(|index| self.segments[index].lock == Lock::Internal)
        {
            return Err(format!("{message:?} out of place"));
        }
        let id = (number, view);
        let framed = self.frame_views.contains_key(&id);
        let segment = match framed {
            true => None,
            false => Some(self.segment_of(number, view)?),
        };
        let shown = segment == Some(self.current);
        match message {
            ToBrowser::State { state, .. } => match segment {
                Some(index) => {
                    self.segments[index].state = *state;
                    if shown {
                        self.undecodable = undecodable(self.state());
                    }
                }
                None => self.frame_state(id, *state),
            },
            ToBrowser::History { entries, index, .. } => {
                let index = index as usize;
                if index >= entries.len() {
                    return Err(format!("entry {index} of {}", entries.len()));
                }
                // A frame's own history is not the tab's (G92).
                if let Some(at) = segment {
                    let segment = &mut self.segments[at];
                    segment.entries = entries;
                    segment.index = index;
                }
            }
            ToBrowser::Pixels {
                width,
                height,
                first_row,
                bytes,
                ..
            } => {
                let row = frame_size(width, height)?;
                let buffer = self.buffers.entry(id).or_default();
                if buffer.incoming.len() != width as usize * height as usize * 4 {
                    buffer.incoming = vec![255; width as usize * height as usize * 4];
                }
                let start = first_row as usize * row;
                let end = start
                    .checked_add(bytes.0.len())
                    .filter(|end| bytes.0.len() % row == 0 && *end <= buffer.incoming.len())
                    .ok_or("pixels past the frame")?;
                buffer.incoming[start..end].copy_from_slice(&bytes.0);
            }
            ToBrowser::Holes { holes, .. } => {
                if holes.len() > crate::MAX_FRAMES {
                    return Err(format!("{} holes", holes.len()));
                }
                self.buffers.entry(id).or_default().holes = holes;
                self.size_frames(id);
            }
            ToBrowser::Frame {
                frame,
                width,
                height,
                ..
            } => {
                frame_size(width, height)?;
                let buffer = self.buffers.entry(id).or_default();
                if buffer.incoming.len() != width as usize * height as usize * 4 {
                    return Err("a frame of no pixels".into());
                }
                buffer.pixels.clone_from(&buffer.incoming);
                buffer.size = (width, height);
                if shown || framed {
                    self.fresh = true;
                    self.frames += 1;
                    if self.frames == 1 {
                        self.start_spare();
                    }
                }
                #[cfg(test)]
                if self.holding {
                    self.held = Some((id, frame));
                    return Ok(());
                }
                self.send_to(id, |view| ToRenderer::FrameTaken { view, frame });
            }
            ToBrowser::Console { entries, .. } => self.console.extend(entries),
            ToBrowser::Notice { text, .. } => {
                if shown {
                    self.notice = Some(text);
                }
            }
            ToBrowser::FocusLeft { .. } => {
                if framed {
                    self.focused_view = None;
                } else if shown {
                    self.focus_left = true;
                }
            }
            ToBrowser::Copied { text, cut, .. } => {
                if let Some(text) = text
                    && (shown || framed)
                {
                    self.copied = Some((text, cut));
                }
            }
            ToBrowser::DevTools { message, .. }
                if shown && self.devtools.len() < crate::MAX_INSPECTED_MESSAGES =>
            {
                self.devtools.push(message);
            }
            ToBrowser::Inspected { message, .. }
                if shown && self.inspected.len() < crate::MAX_INSPECTED_MESSAGES =>
            {
                self.inspected.push(message);
            }
            // A tab the page shown asked for (M37): a page of the web's or a
            // blank one, at most MAX_OPENING waiting; the renderer's word
            // for the user's activation, the browser's tabs bounded.
            ToBrowser::OpenTab {
                url,
                show,
                by_script,
                ..
            } if (shown || framed)
                && crate::frames::opens_in_tab(&url)
                && self.opened.len() < MAX_OPENING =>
            {
                self.opened.push(crate::frames::OpenTab {
                    url,
                    show,
                    by_script,
                });
            }
            ToBrowser::CloseTab { .. } if shown => self.close_asked = true,
            _ => {}
        }
        Ok(())
    }

    /// A frame root's state: its parent hears of each document it loads.
    fn frame_state(&mut self, id: ViewId, state: ViewState) {
        let Some(frame) = self.frame_views.get_mut(&id) else {
            return;
        };
        let tell = frame.tells(&state);
        if tell {
            frame.told.clone_from(&state.url);
        }
        let key = frame.key;
        #[cfg(test)]
        if tell {
            self.told_loaded
                .push(format!("{key}: {:?} {:?}", state.url, state.load));
        }
        frame.state = state;
        if tell {
            let parent = super::frames::lock(&self.registry)
                .frames
                .get(&key)
                .map(|info| info.renderer);
            if let Some(slot) = parent.and_then(|parent| self.renderers.get(&parent)) {
                let _ = slot.renderer.send(&ToRenderer::FrameLoaded { key });
            }
        }
    }

    /// The frame roots in view `id`'s holes are given their holes' sizes.
    fn size_frames(&mut self, id: ViewId) {
        let holes = self
            .buffers
            .get(&id)
            .map(|buffer| buffer.holes.clone())
            .unwrap_or_default();
        for hole in &holes {
            self.size_frame(hole);
        }
    }

    /// The frame root in `hole` is given the hole's size, if it has
    /// another.
    fn size_frame(&mut self, hole: &Hole) {
        let scale = self.settings.scale;
        let Some(view) = self.root_of(hole.key) else {
            return;
        };
        let Some(frame) = self.frame_views.get_mut(&view) else {
            return;
        };
        let raster = (hole.width.max(1), hole.height.max(1));
        if frame.raster == raster {
            return;
        }
        frame.raster = raster;
        let css = |v: u32| (v as f32 / scale).round().max(1.0) as u32;
        self.send_to(view, |view| ToRenderer::Resize {
            view,
            width: css(raster.0),
            height: css(raster.1),
            raster_width: raster.0,
            raster_height: raster.1,
        });
    }

    /// The view showing frame `key`'s documents, if another renderer's.
    fn root_of(&self, key: u32) -> Option<ViewId> {
        super::frames::lock(&self.registry)
            .frames
            .get(&key)
            .and_then(|info| info.root)
            .filter(|view| self.frame_views.contains_key(view))
    }

    /// The pages saying a frame's renderer stopped, drawn at the sizes of
    /// the holes they show in.
    fn draw_stopped_frames(&mut self) {
        if self.stopped_frames.is_empty() {
            return;
        }
        let scale = self.settings.scale;
        let holes: Vec<Hole> = self
            .buffers
            .values()
            .flat_map(|buffer| buffer.holes.iter().cloned())
            .filter(|hole| self.stopped_frames.contains_key(&hole.key))
            .collect();
        for hole in holes {
            let Some(stopped) = self.stopped_frames.get_mut(&hole.key) else {
                continue;
            };
            let size = (hole.width.max(1), hole.height.max(1));
            if stopped.size == size && !stopped.engine.needs_frame() {
                continue;
            }
            let css = |v: u32| (v as f32 / scale).round().max(1.0) as u32;
            stopped.engine.set_viewport(css(size.0), css(size.1));
            stopped.pixels = stopped
                .engine
                .frame_at(size.0, size.1, FrameTime::at(Duration::ZERO))
                .to_vec();
            stopped.size = size;
        }
    }

    /// Puts frame root `view`'s pixels into `out`, `size` big, where the
    /// frame drawn at `at` in it shows through a hole, and theirs into
    /// theirs, within `clip`.
    fn compose(
        &self,
        view: ViewId,
        out: &mut [u8],
        size: (u32, u32),
        at: (i32, i32),
        clip: [i32; 4],
        depth: usize,
    ) {
        let Some(buffer) = self.buffers.get(&view) else {
            return;
        };
        if depth > crate::MAX_FRAME_DEPTH {
            return;
        }
        for hole in &buffer.holes {
            // The frame root's pixels, or the page saying its renderer
            // stopped.
            let child = self.root_of(hole.key);
            let inner = match child {
                Some(child) => self
                    .buffers
                    .get(&child)
                    .map(|inner| (inner.pixels.as_slice(), inner.size)),
                None => self
                    .stopped_frames
                    .get(&hole.key)
                    .map(|stopped| (stopped.pixels.as_slice(), stopped.size)),
            };
            let Some((pixels, inner_size)) = inner else {
                continue;
            };
            let origin = (at.0 + hole.x, at.1 + hole.y);
            let area = [
                origin.0.max(clip[0]).max(0),
                origin.1.max(clip[1]).max(0),
                (origin.0 + hole.width as i32)
                    .min(clip[2])
                    .min(size.0 as i32),
                (origin.1 + hole.height as i32)
                    .min(clip[3])
                    .min(size.1 as i32),
            ];
            for y in area[1]..area[3] {
                for x in area[0]..area[2] {
                    if buffer.pixel(x - at.0, y - at.1) != Some(HOLE) {
                        continue;
                    }
                    let pixel = pixel_in(pixels, inner_size, x - origin.0, y - origin.1)
                        .unwrap_or([255, 255, 255, 255]);
                    let to = (y as usize * size.0 as usize + x as usize) * 4;
                    out[to..to + 4].copy_from_slice(&pixel);
                }
            }
            if let Some(child) = child {
                self.compose(child, out, size, origin, area, depth + 1);
            }
        }
    }

    /// The view under the page's point `(x, y)`, CSS pixels, and the point
    /// in it: a frame root's, through the holes it shows in, or the page's.
    fn view_at(&self, x: f32, y: f32) -> Option<(ViewId, f32, f32)> {
        let scale = self.settings.scale;
        let mut view = self.shown_view()?;
        let (mut rx, mut ry) = ((x * scale).round() as i32, (y * scale).round() as i32);
        for _ in 0..=crate::MAX_FRAME_DEPTH {
            let Some(buffer) = self.buffers.get(&view) else {
                break;
            };
            let inside = buffer.holes.iter().rev().find(|hole| {
                rx >= hole.x
                    && ry >= hole.y
                    && rx < hole.x + hole.width as i32
                    && ry < hole.y + hole.height as i32
                    && buffer.pixel(rx, ry) == Some(HOLE)
            });
            let Some(hole) = inside else {
                break;
            };
            let Some(child) = self.root_of(hole.key) else {
                break;
            };
            (rx, ry) = (rx - hole.x, ry - hole.y);
            view = child;
        }
        Some((view, rx as f32 / scale, ry as f32 / scale))
    }

    /// The offset of view `target`'s page in the tab's page, CSS pixels.
    fn offset_of(&self, target: ViewId) -> (f32, f32) {
        let scale = self.settings.scale;
        let Some(mut view) = self.shown_view() else {
            return (0.0, 0.0);
        };
        let (mut x, mut y) = (0, 0);
        for _ in 0..=crate::MAX_FRAME_DEPTH {
            if view == target {
                break;
            }
            let next = self.buffers.get(&view).and_then(|buffer| {
                buffer.holes.iter().find_map(|hole| {
                    let child = self.root_of(hole.key)?;
                    self.reaches(child, target)
                        .then_some((child, hole.x, hole.y))
                })
            });
            let Some((child, hx, hy)) = next else {
                break;
            };
            (x, y, view) = (x + hx, y + hy, child);
        }
        (x as f32 / scale, y as f32 / scale)
    }

    /// Whether view `target` is `view` or shows in its holes.
    fn reaches(&self, view: ViewId, target: ViewId) -> bool {
        let mut queue = vec![view];
        let mut seen = 0;
        while let Some(at) = queue.pop() {
            if at == target {
                return true;
            }
            seen += 1;
            if seen > crate::MAX_FRAMES {
                return false;
            }
            if let Some(buffer) = self.buffers.get(&at) {
                queue.extend(
                    buffer
                        .holes
                        .iter()
                        .filter_map(|hole| self.root_of(hole.key)),
                );
            }
        }
        false
    }

    /// The pointer's event at `(x, y)` goes to the view under it, or to the
    /// one a press is held in; the view it left hears so.
    fn pointer(&mut self, x: f32, y: f32, event: impl FnOnce(f32, f32) -> Input) {
        let target = match self.captured {
            Some((view, (dx, dy))) => Some((view, x - dx, y - dy)),
            None => self.view_at(x, y),
        };
        let Some((view, fx, fy)) = target else {
            return;
        };
        if self.pointer_view != Some(view) {
            if let Some(left) = self.pointer_view {
                self.send_to(left, |view| ToRenderer::Input {
                    view,
                    input: Input::PointerLeave {},
                });
            }
            self.pointer_view = Some(view);
        }
        let input = event(fx, fy);
        self.send_to(view, |view| ToRenderer::Input { view, input });
    }

    /// Keeps what the browsing keeps in `profile` from now on (M28): the
    /// browser's cookies, HSTS hosts, permissions and `localStorage`.
    pub fn set_profile(&mut self, profile: Profile) -> Result<(), profile::Error> {
        self.context.set_profile(profile)
    }

    /// The tab's number in its context (M37).
    pub fn tab(&self) -> u64 {
        self.tab
    }

    /// Tells the tab's renderers of the changes of Web Storage areas
    /// made elsewhere (M37): each renderer whose lock admits the area's
    /// origin, but the one that made it; a `sessionStorage` area's only
    /// in its own tab. Whether there were any.
    fn relay_storage_changes(&mut self) -> bool {
        let changes = self
            .storage
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take_changes(self.listener);
        for change in &changes {
            if let crate::storage::Scope::Session(tab) = change.scope
                && tab != self.tab
            {
                continue;
            }
            for (number, slot) in &self.renderers {
                if change.source == crate::storage::Source::Renderer(*number)
                    || change.owner.is_opaque()
                    || !slot
                        .lock
                        .as_ref()
                        .is_some_and(|lock| lock.admits(&change.owner))
                {
                    continue;
                }
                #[cfg(test)]
                {
                    self.relayed += 1;
                }
                let _ = slot.renderer.send(&ToRenderer::StorageChanged {
                    local: change.scope.is_local(),
                    origin: change.owner.clone(),
                    partition: change.partition.clone(),
                    key: change.key.clone(),
                    old: change.old.clone(),
                    new: change.new.clone(),
                    url: change.url.clone(),
                });
            }
        }
        !changes.is_empty()
    }

    /// The numbers of the tab's renderers alive.
    #[cfg(test)]
    pub(crate) fn renderer_numbers(&self) -> Vec<u32> {
        self.renderers.keys().copied().collect()
    }

    /// How many changes of Web Storage areas it told its renderers.
    #[cfg(test)]
    pub(crate) fn storage_changes_sent(&self) -> u64 {
        self.relayed
    }

    /// How many frames the renderer sent.
    #[cfg(test)]
    pub(crate) fn frames_received(&self) -> u64 {
        self.frames
    }

    /// Kills the renderer of the page shown, as a crash would end it
    /// (M34.4, M34.7).
    pub fn kill_renderer(&self) {
        if let Some((renderer, _)) = self.shown() {
            renderer.kill();
        }
    }

    /// The tab's network reaches hosts as `config` says: for the tests
    /// of the adversarial site's names and address spaces.
    #[cfg(test)]
    pub(crate) fn set_transport(&mut self, config: net::transport::Config) {
        self.network.replace_client(
            Client::new(config)
                .with_cookies(self.jar.clone())
                .with_cache(self.cache.clone())
                .with_hsts(self.hsts.clone())
                .with_local_network_gate(self.permissions.clone())
                .with_observer(self.log.clone()),
        );
    }

    /// The hosts the tab knows as HTTPS only.
    #[cfg(test)]
    pub(crate) fn hsts(&self) -> Arc<Mutex<Hsts>> {
        self.hsts.clone()
    }

    /// Reads the frames that come without taking them while `hold`, and
    /// takes the last one read once not.
    #[cfg(test)]
    pub(crate) fn hold_frames(&mut self, hold: bool) {
        self.holding = hold;
        if !hold && let Some((id, frame)) = self.held.take() {
            self.send_to(id, |view| ToRenderer::FrameTaken { view, frame });
        }
    }

    /// Each time a frame's parent was told it loaded, and why.
    #[cfg(test)]
    pub(crate) fn told_loaded(&self) -> &[String] {
        &self.told_loaded
    }

    /// The load state of each frame root, by the renderer showing it.
    #[cfg(test)]
    pub(crate) fn frame_loads(&self) -> Vec<(u32, LoadState)> {
        self.frame_views
            .iter()
            .map(|((renderer, _), frame)| (*renderer, frame.state.load.clone()))
            .collect()
    }

    /// Kills renderer `number`, as a crash would end it.
    #[cfg(test)]
    pub(crate) fn kill(&self, number: u32) {
        if let Some(slot) = self.renderers.get(&number) {
            slot.renderer.kill();
        }
    }

    /// The processes of the tab's renderers alive: for the checks.
    pub fn renderer_processes(&self) -> Vec<u32> {
        self.renderers
            .values()
            .map(|slot| slot.renderer.process_id())
            .collect()
    }

    /// The process of the renderer of the page shown, if one runs: for the
    /// tests and the checks.
    pub fn renderer_process(&self) -> Option<u32> {
        self.shown().map(|(renderer, _)| renderer.process_id())
    }

    /// The tab's renderers alive, by number, with the sites they are
    /// locked to: for the tests and the checks.
    pub fn renderers(&self) -> Vec<(u32, Option<String>)> {
        self.renderers
            .iter()
            .map(|(number, slot)| (*number, slot.lock.as_ref().map(Lock::to_string)))
            .collect()
    }

    /// The number of the renderer of the page shown.
    pub fn shown_renderer(&self) -> Option<u32> {
        self.segment().and_then(|segment| segment.renderer)
    }

    /// The frame roots of the tab, by the renderers that show them and the
    /// URLs they show: for the tests and the checks.
    pub fn frame_roots(&self) -> Vec<(u32, Option<Url>)> {
        let mut roots: Vec<_> = self
            .frame_views
            .iter()
            .map(|((renderer, _), frame)| (*renderer, frame.state.url.clone()))
            .collect();
        roots.sort_by_key(|(renderer, url)| (*renderer, url.as_ref().map(|url| url.to_string())));
        roots
    }
}

impl Drop for RemoteTab {
    fn drop(&mut self) {
        for slot in self.renderers.values() {
            slot.renderer.shutdown();
        }
        let mut storage = self.storage.lock().unwrap_or_else(PoisonError::into_inner);
        storage.forget(self.listener);
        storage.forget_tab(self.tab);
    }
}

/// The wake that calls the session's waker once it is given.
fn waking(waker: &Arc<Mutex<Option<crate::loader::Waker>>>) -> crate::loader::Waker {
    let waker = waker.clone();
    Arc::new(move || {
        if let Some(wake) = waker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            wake();
        }
    })
}

/// The static name of the encoding `state` says its document cannot be
/// decoded in.
fn undecodable(state: &ViewState) -> Option<&'static str> {
    state
        .undecodable
        .as_deref()
        .and_then(encoding::Encoding::for_label)
        .map(encoding::Encoding::name)
}

/// A frame's row length, for a size a frame may have.
fn frame_size(width: u32, height: u32) -> Result<usize, String> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || pixels > u64::from(MAX_SURFACE_PIXELS) {
        return Err(format!("a frame of {width} by {height}"));
    }
    Ok(width as usize * 4)
}

/// The browser's client, keeping the browser's jar, cache, HSTS hosts
/// and permissions.
fn client(
    jar: &Arc<Mutex<Jar>>,
    cache: &Arc<net::Cache>,
    hsts: &Arc<Mutex<Hsts>>,
    permissions: &Arc<Permissions>,
    log: &Arc<crate::network_log::NetworkLog>,
    languages: &Arc<Mutex<String>>,
    compat: bool,
) -> Client {
    let client = match compat {
        true => Client::default().with_user_agent(net::COMPAT_USER_AGENT),
        false => Client::default(),
    };
    client
        .with_cookies(jar.clone())
        .with_cache(cache.clone())
        .with_hsts(hsts.clone())
        .with_local_network_gate(permissions.clone())
        .with_observer(log.clone())
        .with_languages(languages.clone())
}

impl View for RemoteTab {
    fn fonts(&self) -> &text::FontCollection {
        &self.fonts
    }

    fn navigate(&mut self, url: Url) {
        let lock = Lock::of(&url);
        let same = self.segment().is_some_and(|segment| {
            segment.lock == lock
                && segment
                    .renderer
                    .is_some_and(|number| self.renderers.contains_key(&number))
        });
        if !same && !self.new_segment(lock) {
            return;
        }
        if let Some(segment) = self.segments.get_mut(self.current) {
            segment.state.load = LoadState::Loading;
        }
        self.send(|view| ToRenderer::Navigate { view, url });
    }

    fn go_back(&mut self) {
        let Some(segment) = self.segment() else {
            return;
        };
        if segment.index > 0 {
            self.send(|view| ToRenderer::Traverse { view, delta: -1 });
        } else if self.current > 0 {
            self.switch_to(self.current - 1);
        }
    }

    fn go_forward(&mut self) {
        let Some(segment) = self.segment() else {
            return;
        };
        if segment.index + 1 < segment.entries.len() {
            self.send(|view| ToRenderer::Traverse { view, delta: 1 });
        } else if self.current + 1 < self.segments.len() {
            self.switch_to(self.current + 1);
        }
    }

    fn reload(&mut self) {
        if self.shown_view().is_some() {
            self.send(|view| ToRenderer::Reload { view });
            return;
        }
        // A page whose renderer is gone comes back in a new one.
        let index = self.current;
        if index < self.segments.len() {
            self.segments[index].renderer = None;
            self.current = usize::MAX;
            self.switch_to(index);
        }
    }

    fn stop(&mut self) {
        self.send(|view| ToRenderer::Stop { view });
    }

    fn can_go_back(&self) -> bool {
        self.current > 0 || self.segment().is_some_and(|segment| segment.index > 0)
    }

    fn can_go_forward(&self) -> bool {
        self.current + 1 < self.segments.len()
            || self
                .segment()
                .is_some_and(|segment| segment.index + 1 < segment.entries.len())
    }

    fn url(&self) -> Option<&Url> {
        self.state().url.as_ref()
    }

    fn title(&self) -> String {
        self.state().title.clone()
    }

    fn load_state(&self) -> &LoadState {
        &self.state().load
    }

    fn undecodable_encoding(&self) -> Option<&'static str> {
        self.undecodable
    }

    fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    fn take_console(&mut self) -> Vec<ConsoleEntry> {
        std::mem::take(&mut self.console)
    }

    fn permission_prompt(&self) -> Option<PermissionPrompt> {
        self.permissions.prompt()
    }

    fn answer_prompt(&mut self, id: u64, answer: Option<bool>) {
        self.permissions.answer(id, answer);
    }

    fn set_preferences(&mut self, preferences: Preferences) {
        self.settings.preferences = preferences;
        self.send_settings();
    }

    fn set_scripting(&mut self, on: bool) {
        self.settings.scripting = on;
        self.send_settings();
    }

    fn set_compat_user_agent(&mut self, on: bool) {
        self.settings.compat_user_agent = on;
        self.network.replace_client(client(
            &self.jar,
            &self.cache,
            &self.hsts,
            &self.permissions,
            &self.log,
            self.context.accept_language(),
            on,
        ));
        self.send_settings();
    }

    fn set_hidden(&mut self, hidden: bool) {
        self.settings.hidden = hidden;
        self.send_settings();
        // A hidden tab keeps no spare renderer (M37): the tab shown has
        // the one a window keeps.
        if hidden {
            let spares: Vec<u32> = self
                .renderers
                .iter()
                .filter(|(_, slot)| slot.lock.is_none())
                .map(|(number, _)| *number)
                .collect();
            for number in spares {
                self.forget(number);
            }
        } else {
            self.start_spare();
        }
    }

    fn renderer_count(&self) -> usize {
        self.renderers.len()
    }

    fn take_opened(&mut self) -> Vec<crate::frames::OpenTab> {
        std::mem::take(&mut self.opened)
    }

    fn downloads(&self) -> Option<crate::downloads::Downloads> {
        Some(self.context.downloads().clone())
    }

    fn set_languages(&mut self, languages: Vec<String>) {
        if self.settings.languages != languages {
            self.settings.languages = languages;
            self.send_settings();
        }
    }

    fn take_close_request(&mut self) -> bool {
        std::mem::take(&mut self.close_asked) && self.opened_by_script
    }

    fn set_opened_by_script(&mut self, by_script: bool) {
        self.opened_by_script = by_script;
    }

    fn set_window_focus(&mut self, focused: bool) {
        if self.settings.focused != focused {
            self.settings.focused = focused;
            self.send_settings();
        }
    }

    fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.settings.modifiers = modifiers;
        self.send_settings();
    }

    fn set_scale_factor(&mut self, scale: f32) {
        if scale.is_finite() && scale > 0.0 {
            self.settings.scale = scale;
            self.send_settings();
        }
    }

    fn scale_factor(&self) -> f32 {
        self.settings.scale
    }

    fn set_viewport(&mut self, width: u32, height: u32) {
        if self.viewport != (width, height) {
            self.viewport = (width, height);
            self.send_size();
        }
    }

    fn set_layer_budget(&mut self, bytes: usize) {
        self.settings.layer_budget = Some(bytes as u64);
        self.send_settings();
    }

    fn set_waker(&mut self, waker: Arc<dyn Fn() + Send + Sync>) {
        self.permissions.set_waker(waker.clone());
        *self.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }

    fn set_clock(&mut self, clock: Clock) {
        self.clock = Some(clock);
    }

    fn devtools_panel(&self) -> Option<RemoteTab> {
        Some(RemoteTab::devtools_panel(self))
    }

    fn devtools_send(&mut self, message: String) {
        let internal = self
            .segment()
            .is_some_and(|segment| segment.lock == Lock::Internal);
        if self.panel || internal {
            self.send(|view| ToRenderer::DevTools { view, message });
        }
    }

    fn take_devtools(&mut self) -> Vec<String> {
        std::mem::take(&mut self.devtools)
    }

    fn inspect(&mut self, message: String) {
        if !self.panel {
            self.send(|view| ToRenderer::Inspect { view, message });
        }
    }

    fn take_inspected(&mut self) -> Vec<String> {
        std::mem::take(&mut self.inspected)
    }

    fn network_log(&self) -> Option<Arc<crate::network_log::NetworkLog>> {
        Some(self.log.clone())
    }

    fn storage_inspect(&mut self, message: &str) -> String {
        let url = self.state().url.clone();
        crate::storage::devtools::inspect(&self.storage, &self.jar, self.tab, url.as_ref(), message)
    }

    fn run_tasks(&mut self) -> bool {
        let changed = self.take_events();
        self.handovers.expire();
        let now = self.now();
        let due = {
            let storage = self.storage.lock().unwrap_or_else(PoisonError::into_inner);
            storage.flush_at(&self.jar).is_some_and(|at| at <= now)
        };
        if due && let Err(error) = self.flush_profile() {
            self.notice = Some(format!("The profile was not written: {error}"));
        }
        changed
    }

    fn poll_network(&mut self) -> bool {
        self.take_events()
    }

    fn next_task_at(&self) -> Option<Duration> {
        self.storage
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .flush_at(&self.jar)
    }

    fn needs_frame(&self) -> bool {
        self.fresh
    }

    fn next_frame_at(&self) -> Option<Duration> {
        None
    }

    fn frame_at(&mut self, width: u32, height: u32, time: FrameTime) -> &[u8] {
        if self.raster != (width, height) {
            self.raster = (width, height);
            self.send_size();
        }
        self.fresh = false;
        let view = self.shown_view();
        // The page saying the page's renderer stopped, the browser's own.
        let stopped = self
            .stopped
            .as_ref()
            .is_some_and(|(index, _)| *index == self.current);
        if view.is_none()
            && stopped
            && let Some((index, mut engine)) = self.stopped.take()
        {
            engine.set_viewport(self.viewport.0.max(1), self.viewport.1.max(1));
            self.out.clear();
            self.out
                .extend_from_slice(engine.frame_at(width, height, time));
            self.stopped = Some((index, engine));
            return &self.out;
        }
        self.draw_stopped_frames();
        let buffer = view.and_then(|view| self.buffers.get(&view));
        let plain =
            buffer.is_some_and(|buffer| buffer.size == (width, height) && buffer.holes.is_empty());
        if plain && let Some(buffer) = buffer {
            return &buffer.pixels;
        }
        // The page's last frame at the top left, the rest white until the
        // renderer draws it at this size; the frames of other renderers in
        // their holes.
        let (w, h) = (width as usize, height as usize);
        let mut out = std::mem::take(&mut self.out);
        out.clear();
        out.resize(w * h * 4, 255);
        if let (Some(view), Some(buffer)) = (view, buffer) {
            let (fw, fh) = (buffer.size.0 as usize, buffer.size.1 as usize);
            for y in 0..h.min(fh) {
                let n = w.min(fw) * 4;
                out[y * w * 4..y * w * 4 + n]
                    .copy_from_slice(&buffer.pixels[y * fw * 4..y * fw * 4 + n]);
            }
            let clip = [0, 0, width as i32, height as i32];
            self.compose(view, &mut out, (width, height), (0, 0), clip, 0);
        }
        self.out = out;
        &self.out
    }

    fn draw_ahead(&mut self, _: Instant) -> u32 {
        0
    }

    fn pointer_move(&mut self, x: f32, y: f32) {
        self.pointer(x, y, |x, y| Input::PointerMove { x, y });
    }

    fn pointer_leave(&mut self) {
        if let Some(view) = self.pointer_view.take() {
            self.send_to(view, |view| ToRenderer::Input {
                view,
                input: Input::PointerLeave {},
            });
        }
    }

    fn pointer_down(&mut self, x: f32, y: f32, button: Button, count: u32) {
        if let Some((view, fx, fy)) = self.view_at(x, y) {
            let framed = self.frame_views.contains_key(&view);
            let (dx, dy) = self.offset_of(view);
            self.captured = Some((view, (dx, dy)));
            let before = self.focused_view;
            self.focused_view = framed.then_some(view);
            if before != self.focused_view {
                self.send_settings();
            }
            let _ = (fx, fy);
        }
        self.pointer(x, y, |x, y| Input::PointerDown {
            x,
            y,
            button,
            count,
        });
    }

    fn pointer_up(&mut self, x: f32, y: f32, button: Button, count: u32) {
        self.pointer(x, y, |x, y| Input::PointerUp {
            x,
            y,
            button,
            count,
        });
        self.captured = None;
    }

    fn wheel(&mut self, x: f32, y: f32, dy: f32) {
        self.pointer(x, y, |x, y| Input::Wheel { x, y, dy });
    }

    fn cursor_at(&mut self, _: f32, _: f32) -> Cursor {
        self.pointed_state().cursor
    }

    fn link_at(&mut self, _: f32, _: f32) -> Option<Link> {
        self.pointed_state().hover.clone().map(Link::remote)
    }

    fn key_down(&mut self, key: &Key) -> KeyOutcome {
        self.input(Input::KeyDown { key: key.clone() });
        KeyOutcome::Taken
    }

    fn key_up(&mut self, key: &Key) {
        self.input(Input::KeyUp { key: key.clone() });
    }

    fn focus_next(&mut self, backward: bool) -> bool {
        self.focused_view = None;
        self.input(Input::FocusNext { backward });
        true
    }

    fn insert_text(&mut self, text: &str) -> bool {
        self.input(Input::InsertText {
            text: text.to_owned(),
        });
        true
    }

    fn copy(&self) -> Option<String> {
        match self
            .focused_view
            .filter(|view| self.frame_views.contains_key(view))
        {
            Some(view) => self.send_to(view, |view| ToRenderer::Copy { view, cut: false }),
            None => self.send(|view| ToRenderer::Copy { view, cut: false }),
        }
        None
    }

    fn cut(&mut self) -> Option<String> {
        self.cut_copied();
        None
    }

    fn paste(&mut self, text: &str) -> bool {
        self.input(Input::Paste {
            text: text.to_owned(),
        });
        true
    }

    fn can_paste(&self) -> bool {
        self.focused_view
            .and_then(|view| self.frame_views.get(&view))
            .map_or_else(|| self.state().can_paste, |frame| frame.state.can_paste)
    }

    fn caret_area(&mut self) -> Option<(f32, f32, f32, f32)> {
        if let Some(view) = self.focused_view
            && let Some(frame) = self.frame_views.get(&view)
        {
            let (dx, dy) = self.offset_of(view);
            return frame.state.caret.map(|(x, y, w, h)| (x + dx, y + dy, w, h));
        }
        self.state().caret
    }

    fn popup_open(&self) -> bool {
        self.state().popup
    }

    fn scrollbar(&mut self) -> Option<Scrollbar> {
        self.state().scrollbar
    }

    fn scroll_by(&mut self, dy: f32) {
        self.send(|view| ToRenderer::Input {
            view,
            input: Input::ScrollBy { dy },
        });
    }

    fn scroll_to_fraction(&mut self, fraction: f32) {
        self.send(|view| ToRenderer::Input {
            view,
            input: Input::ScrollToFraction { fraction },
        });
    }

    fn flush_profile(&mut self) -> Result<(), profile::Error> {
        let now = self.now();
        self.storage
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .flush(&self.jar, now)
    }

    fn set_profile(&mut self, profile: Profile) -> Result<(), profile::Error> {
        RemoteTab::set_profile(self, profile)
    }

    fn take_focus_left(&mut self) -> bool {
        std::mem::take(&mut self.focus_left)
    }

    fn copy_to_cut(&mut self) -> Option<String> {
        match self
            .focused_view
            .filter(|view| self.frame_views.contains_key(view))
        {
            Some(view) => self.send_to(view, |view| ToRenderer::Copy { view, cut: true }),
            None => self.send(|view| ToRenderer::Copy { view, cut: true }),
        }
        None
    }

    fn take_copied(&mut self) -> Option<(String, bool)> {
        self.copied.take()
    }

    fn cut_copied(&mut self) {
        match self
            .focused_view
            .filter(|view| self.frame_views.contains_key(view))
        {
            Some(view) => self.send_to(view, |view| ToRenderer::Cut { view }),
            None => self.send(|view| ToRenderer::Cut { view }),
        }
    }

    fn has_page_frame(&self) -> bool {
        self.shown_view()
            .and_then(|view| self.buffers.get(&view))
            .is_some_and(|buffer| buffer.size != (0, 0))
    }
}

#[cfg(test)]
mod frame_view_tests {
    use super::*;

    fn state(url: &str, load: LoadState) -> ViewState {
        ViewState {
            url: Url::parse(url).ok(),
            load,
            ..ViewState::default()
        }
    }

    #[test]
    fn a_frames_parent_is_told_of_each_document_loaded_but_the_initial_blank() {
        let mut view = FrameView::handed_over(7);
        // Said loaded before the navigation was taken (CI's runners).
        assert!(!view.tells(&state("about:blank", LoadState::Loaded)));
        assert!(!view.tells(&state("http://b.test/", LoadState::Loading)));
        let loaded = state("http://b.test/", LoadState::Loaded);
        assert!(view.tells(&loaded));
        view.told = loaded.url.clone();
        assert!(!view.tells(&loaded), "told once");
        // A blank its document goes to later is a document like any.
        assert!(view.tells(&state("about:blank", LoadState::Loaded)));
    }
}
