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

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use net::cookies::Jar;
use net::{Client, Hsts, Url};

use super::handover::{HandoverNotice, Handovers};
use super::network::{FrameGrant, Grants, Lock, NetworkService};
use super::process::{Event, Gone, Launcher, Renderer, Services};
use super::storage::StorageService;
use crate::ipc::{Input, Settings, ToBrowser, ToRenderer, ViewState};
use crate::permissions::Permissions;
use crate::renderer::MAX_SURFACE_PIXELS;
use crate::storage::Storage;
use crate::view::View;
use crate::{
    Button, Clock, ConsoleEntry, Cursor, FrameTime, Key, KeyOutcome, Link, LoadError, LoadState,
    Modifiers, PermissionPrompt, Preferences, Profile, Scrollbar,
};

/// The most renderers a tab has alive at once, the spare among them
/// (docs/design/processes.md, "The cap and eviction").
pub const MAX_RENDERERS: usize = 16;

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

/// A tab whose pages renderers show.
pub struct RemoteTab {
    fonts: text::FontCollection,
    launcher: Arc<dyn Launcher>,
    grants: Arc<Grants>,
    network: Arc<NetworkService>,
    handovers: Arc<Handovers>,
    services: Arc<Services>,
    storage: Arc<Mutex<Storage>>,
    jar: Arc<Mutex<Jar>>,
    cache: Arc<net::Cache>,
    hsts: Arc<Mutex<Hsts>>,
    permissions: Arc<Permissions>,
    renderers: BTreeMap<u32, Slot>,
    next_renderer: u32,
    next_view: u32,
    next_frame: u32,
    /// The most renderers alive at once, and whether one waits, unlocked,
    /// for the next site.
    cap: usize,
    spare: bool,
    segments: Vec<Segment>,
    current: usize,
    events: mpsc::Receiver<(u32, Event)>,
    sender: mpsc::Sender<(u32, Event)>,
    handed: mpsc::Receiver<HandoverNotice>,
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
    /// The frame being received, and the last whole one with its size.
    incoming: Vec<u8>,
    pixels: Vec<u8>,
    size: (u32, u32),
    /// What `frame_at` returns when the size asked is not the frame's.
    out: Vec<u8>,
    fresh: bool,
    /// How many frames came: for the tests of the renderer's pace.
    frames: u64,
    console: Vec<ConsoleEntry>,
    notice: Option<String>,
    focus_left: bool,
    copied: Option<(String, bool)>,
}

impl RemoteTab {
    /// A tab whose renderers `launcher` starts, its own interface drawn
    /// with `fonts`, at most [`MAX_RENDERERS`] of them, one spare.
    pub fn new(launcher: Arc<dyn Launcher>, fonts: text::FontCollection) -> RemoteTab {
        RemoteTab::with_cap(launcher, fonts, MAX_RENDERERS, true)
    }

    /// A tab of at most `cap` renderers, one of them spare if `spare`.
    pub fn with_cap(
        launcher: Arc<dyn Launcher>,
        fonts: text::FontCollection,
        cap: usize,
        spare: bool,
    ) -> RemoteTab {
        let grants = Grants::new();
        let jar: Arc<Mutex<Jar>> = Arc::default();
        let cache: Arc<net::Cache> = Arc::default();
        let hsts: Arc<Mutex<Hsts>> = Arc::default();
        let permissions: Arc<Permissions> = Arc::default();
        let storage = Arc::new(Mutex::new(Storage::new()));
        let handovers = Handovers::new();
        let network = NetworkService::new(
            client(&jar, &cache, &hsts, &permissions, false),
            grants.clone(),
        );
        network.hand_over_with(handovers.clone());
        let services = Arc::new(Services(vec![
            network.clone(),
            StorageService::new(storage.clone(), grants.clone()),
        ]));
        let (sender, events) = mpsc::channel();
        let (notify, handed) = mpsc::channel();
        let waker: Arc<Mutex<Option<crate::loader::Waker>>> = Arc::default();
        let waking = waker.clone();
        let notify = Mutex::new(notify);
        network.on_handover(Arc::new(move |notice| {
            let _ = notify
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .send(notice);
            if let Some(wake) = waking
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
            {
                wake();
            }
        }));
        RemoteTab {
            fonts,
            launcher,
            grants,
            network,
            handovers,
            services,
            storage,
            jar,
            cache,
            hsts,
            permissions,
            renderers: BTreeMap::new(),
            next_renderer: 1,
            next_view: 1,
            next_frame: 1,
            cap: cap.max(1),
            spare,
            segments: Vec::new(),
            current: 0,
            events,
            sender,
            handed,
            waker,
            clock: None,
            settings: Settings::default(),
            viewport: (0, 0),
            raster: (0, 0),
            blank: ViewState::default(),
            undecodable: None,
            incoming: Vec::new(),
            pixels: Vec::new(),
            size: (0, 0),
            out: Vec::new(),
            fresh: false,
            frames: 0,
            console: Vec::new(),
            notice: None,
            focus_left: false,
            copied: None,
        }
    }

    /// The state the session reads: the current segment's.
    fn state(&self) -> &ViewState {
        self.segments
            .get(self.current)
            .map_or(&self.blank, |segment| &segment.state)
    }

    fn segment(&self) -> Option<&Segment> {
        self.segments.get(self.current)
    }

    /// The renderer of the current segment, and its view.
    fn shown(&self) -> Option<(&Renderer, u32)> {
        let segment = self.segment()?;
        let slot = self.renderers.get(&segment.renderer?)?;
        Some((&slot.renderer, segment.view))
    }

    fn send(&self, message: impl FnOnce(u32) -> ToRenderer) {
        if let Some((renderer, view)) = self.shown() {
            let _ = renderer.send(&message(view));
        }
    }

    fn input(&self, input: Input) {
        self.send(|view| ToRenderer::Input { view, input });
    }

    fn send_settings(&self) {
        let settings = self.settings.clone();
        self.send(|view| ToRenderer::Settings { view, settings });
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
        let waker = self.waker.clone();
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

    /// Starts a renderer, room made for it; its number.
    fn launch(&mut self) -> Option<u32> {
        if self.renderers.len() >= self.cap && !self.evict() {
            self.notice = Some(format!(
                "No room for another renderer: {} are open",
                self.renderers.len()
            ));
            return None;
        }
        let number = self.next_renderer;
        self.next_renderer += 1;
        let serve: Arc<dyn super::Serve> = self.services.clone();
        match Renderer::start(
            self.launcher.as_ref(),
            number,
            self.sender.clone(),
            self.wake(),
            Some(serve),
        ) {
            Ok(renderer) => {
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
        let idle = self
            .renderers
            .iter()
            .filter(|(number, _)| Some(**number) != shown)
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
        self.grants.forget(number);
        for segment in &mut self.segments {
            if segment.renderer == Some(number) {
                segment.renderer = None;
            }
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
        if self.spare
            && self.renderers.values().all(|slot| slot.lock.is_some())
            && self.renderers.len() < self.cap
        {
            self.launch();
        }
        Some(number)
    }

    /// A new view in `lock`'s renderer, after the current segment, which
    /// becomes the current one; whether there is one.
    fn new_segment(&mut self, lock: Lock) -> bool {
        let Some(number) = self.renderer_for(&lock) else {
            return false;
        };
        let (view, frame) = (self.next_view, self.next_frame);
        self.next_view += 1;
        self.next_frame += 1;
        self.grants.grant_frame(
            number,
            frame,
            FrameGrant {
                top: net::Origin::new_opaque(),
                cross_site: false,
            },
        );
        let renderer = &self.renderers[&number].renderer;
        let _ = renderer.send(&ToRenderer::CreateView {
            view,
            frame,
            settings: self.settings.clone(),
        });
        // The segments after the current one go, and its own entries
        // after its current one.
        if let Some(left) = self.segments.get(self.current) {
            if let Some((renderer, view)) = self.shown() {
                let _ = renderer.send(&ToRenderer::DropForward { view });
            }
            let _ = left;
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
            let lock = self.segments[index].lock.clone();
            let Some(number) = self.renderer_for(&lock) else {
                return;
            };
            let (view, frame) = (self.next_view, self.next_frame);
            self.next_view += 1;
            self.next_frame += 1;
            self.grants.grant_frame(
                number,
                frame,
                FrameGrant {
                    top: net::Origin::new_opaque(),
                    cross_site: false,
                },
            );
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

    /// Takes what the renderers and the network service said; whether
    /// anything changed.
    fn take_events(&mut self) -> bool {
        let mut changed = false;
        while let Ok(notice) = self.handed.try_recv() {
            changed = true;
            self.hand_over(notice);
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

    /// A page's navigation was handed over to its site's renderer: if it
    /// was the current segment's top-level document's, a segment of that
    /// site takes the response.
    fn hand_over(&mut self, notice: HandoverNotice) {
        let current = self
            .segment()
            .and_then(|segment| Some((segment.renderer?, segment.frame)));
        if current != Some((notice.renderer, notice.frame)) {
            return;
        }
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

    /// Renderer `number` is gone: its segments wait to be restored, and
    /// the page it showed says why.
    fn gone(&mut self, number: u32, why: Gone) {
        let showed = self.segment().and_then(|segment| segment.renderer) == Some(number);
        self.renderers.remove(&number);
        self.grants.forget(number);
        for segment in &mut self.segments {
            if segment.renderer == Some(number) {
                segment.renderer = None;
            }
        }
        if showed && why != Gone::Shutdown {
            if let Some(segment) = self.segments.get_mut(self.current) {
                segment.state.load = LoadState::Failed(LoadError::Lost);
            }
            self.notice = Some(format!("The page's renderer stopped ({why:?})"));
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
        match message {
            ToBrowser::State { view, state } => {
                let index = self.segment_of(number, view)?;
                self.segments[index].state = *state;
                if index == self.current {
                    self.undecodable = undecodable(self.state());
                }
            }
            ToBrowser::History {
                view,
                entries,
                index,
            } => {
                let at = self.segment_of(number, view)?;
                let index = index as usize;
                if index >= entries.len() {
                    return Err(format!("entry {index} of {}", entries.len()));
                }
                let segment = &mut self.segments[at];
                segment.entries = entries;
                segment.index = index;
            }
            ToBrowser::Pixels {
                view,
                frame: _,
                width,
                height,
                first_row,
                bytes,
            } => {
                let index = self.segment_of(number, view)?;
                let row = frame_size(width, height)?;
                if index != self.current {
                    return Ok(());
                }
                if self.incoming.len() != width as usize * height as usize * 4 {
                    self.incoming = vec![255; width as usize * height as usize * 4];
                }
                let start = first_row as usize * row;
                let end = start
                    .checked_add(bytes.0.len())
                    .filter(|end| bytes.0.len() % row == 0 && *end <= self.incoming.len())
                    .ok_or("pixels past the frame")?;
                self.incoming[start..end].copy_from_slice(&bytes.0);
            }
            ToBrowser::Frame {
                view,
                frame,
                width,
                height,
            } => {
                let index = self.segment_of(number, view)?;
                frame_size(width, height)?;
                // A view not shown has its frames taken, unseen.
                if index == self.current {
                    if self.incoming.len() != width as usize * height as usize * 4 {
                        return Err("a frame of no pixels".into());
                    }
                    self.pixels.clone_from(&self.incoming);
                    self.size = (width, height);
                    self.fresh = true;
                    self.frames += 1;
                }
                if let Some(slot) = self.renderers.get(&number) {
                    let _ = slot.renderer.send(&ToRenderer::FrameTaken { view, frame });
                }
            }
            ToBrowser::Console { view, entries } => {
                self.segment_of(number, view)?;
                self.console.extend(entries);
            }
            ToBrowser::Notice { view, text } => {
                if self.segment_of(number, view)? == self.current {
                    self.notice = Some(text);
                }
            }
            ToBrowser::FocusLeft { view } => {
                if self.segment_of(number, view)? == self.current {
                    self.focus_left = true;
                }
            }
            ToBrowser::Copied { view, text, cut } => {
                if self.segment_of(number, view)? == self.current
                    && let Some(text) = text
                {
                    self.copied = Some((text, cut));
                }
            }
            other => return Err(format!("{other:?} out of place")),
        }
        Ok(())
    }

    /// Keeps what the browsing keeps in `profile` from now on (M28): the
    /// browser's cookies, HSTS hosts, permissions and `localStorage`.
    pub fn set_profile(&mut self, profile: Profile) -> Result<(), profile::Error> {
        let mut storage = self.storage.lock().unwrap_or_else(PoisonError::into_inner);
        storage.set_hsts(self.hsts.clone());
        storage.set_permissions(self.permissions.clone());
        storage.set_profile(profile, &self.jar)
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
}

impl Drop for RemoteTab {
    fn drop(&mut self) {
        for slot in self.renderers.values() {
            slot.renderer.shutdown();
        }
    }
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
        let alive = self
            .segment()
            .and_then(|segment| segment.renderer)
            .is_some_and(|number| self.renderers.contains_key(&number));
        if alive {
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
            on,
        ));
        self.send_settings();
    }

    fn set_hidden(&mut self, hidden: bool) {
        self.settings.hidden = hidden;
        self.send_settings();
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

    fn frame_at(&mut self, width: u32, height: u32, _: FrameTime) -> &[u8] {
        if self.raster != (width, height) {
            self.raster = (width, height);
            self.send_size();
        }
        self.fresh = false;
        if self.size == (width, height) {
            return &self.pixels;
        }
        // The last frame, of another size, at the top left, the rest of
        // the page white until the renderer draws it at this one.
        let (w, h) = (width as usize, height as usize);
        self.out.clear();
        self.out.resize(w * h * 4, 255);
        let (fw, fh) = (self.size.0 as usize, self.size.1 as usize);
        for y in 0..h.min(fh) {
            let n = w.min(fw) * 4;
            self.out[y * w * 4..y * w * 4 + n]
                .copy_from_slice(&self.pixels[y * fw * 4..y * fw * 4 + n]);
        }
        &self.out
    }

    fn draw_ahead(&mut self, _: Instant) -> u32 {
        0
    }

    fn pointer_move(&mut self, x: f32, y: f32) {
        self.input(Input::PointerMove { x, y });
    }

    fn pointer_leave(&mut self) {
        self.input(Input::PointerLeave {});
    }

    fn pointer_down(&mut self, x: f32, y: f32, button: Button, count: u32) {
        self.input(Input::PointerDown {
            x,
            y,
            button,
            count,
        });
    }

    fn pointer_up(&mut self, x: f32, y: f32, button: Button, count: u32) {
        self.input(Input::PointerUp {
            x,
            y,
            button,
            count,
        });
    }

    fn wheel(&mut self, x: f32, y: f32, dy: f32) {
        self.input(Input::Wheel { x, y, dy });
    }

    fn cursor_at(&mut self, _: f32, _: f32) -> Cursor {
        self.state().cursor
    }

    fn link_at(&mut self, _: f32, _: f32) -> Option<Link> {
        self.state().hover.clone().map(Link::remote)
    }

    fn key_down(&mut self, key: &Key) -> KeyOutcome {
        self.input(Input::KeyDown { key: key.clone() });
        KeyOutcome::Taken
    }

    fn key_up(&mut self, key: &Key) {
        self.input(Input::KeyUp { key: key.clone() });
    }

    fn focus_next(&mut self, backward: bool) -> bool {
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
        self.send(|view| ToRenderer::Copy { view, cut: false });
        None
    }

    fn cut(&mut self) -> Option<String> {
        self.send(|view| ToRenderer::Cut { view });
        None
    }

    fn paste(&mut self, text: &str) -> bool {
        self.input(Input::Paste {
            text: text.to_owned(),
        });
        true
    }

    fn can_paste(&self) -> bool {
        self.state().can_paste
    }

    fn caret_area(&mut self) -> Option<(f32, f32, f32, f32)> {
        self.state().caret
    }

    fn popup_open(&self) -> bool {
        self.state().popup
    }

    fn scrollbar(&mut self) -> Option<Scrollbar> {
        self.state().scrollbar
    }

    fn scroll_by(&mut self, dy: f32) {
        self.input(Input::ScrollBy { dy });
    }

    fn scroll_to_fraction(&mut self, fraction: f32) {
        self.input(Input::ScrollToFraction { fraction });
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
        self.send(|view| ToRenderer::Copy { view, cut: true });
        None
    }

    fn take_copied(&mut self) -> Option<(String, bool)> {
        self.copied.take()
    }

    fn cut_copied(&mut self) {
        self.send(|view| ToRenderer::Cut { view });
    }
}
