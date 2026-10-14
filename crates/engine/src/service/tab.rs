//! The session's proxy (M34.5, docs/design/processes.md, "The session's
//! proxy"): a tab whose page a renderer shows. It keeps what the
//! renderers must not reach (the network service with the cookie jar,
//! the cache, the HSTS hosts and the permissions, Web Storage and the
//! profile) and gives the session what it reads of the page as the
//! renderer last reported it. The browser never waits on a renderer:
//! every order is a message, and every answer an event taken when the
//! session polls.

use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use net::cookies::Jar;
use net::{Client, Hsts, Url};

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

/// The number of the tab's top-level view in its renderer, and the
/// browser's number for its frame.
const TOP_VIEW: u32 = 1;
const TOP_FRAME: u32 = 1;

/// A tab whose page a renderer shows.
pub struct RemoteTab {
    fonts: text::FontCollection,
    launcher: Arc<dyn Launcher>,
    grants: Arc<Grants>,
    network: Arc<NetworkService>,
    services: Arc<Services>,
    storage: Arc<Mutex<Storage>>,
    jar: Arc<Mutex<Jar>>,
    cache: Arc<net::Cache>,
    hsts: Arc<Mutex<Hsts>>,
    permissions: Arc<Permissions>,
    renderer: Option<Renderer>,
    next_renderer: u32,
    events: mpsc::Receiver<(u32, Event)>,
    sender: mpsc::Sender<(u32, Event)>,
    /// What wakes the session's loop when a renderer says something: the
    /// session's waker once given.
    waker: Arc<Mutex<Option<crate::loader::Waker>>>,
    clock: Option<Clock>,
    settings: Settings,
    /// The page's viewport in CSS pixels, and the raster size its frames
    /// are drawn at, as last sent.
    viewport: (u32, u32),
    raster: (u32, u32),
    state: ViewState,
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
    /// with `fonts`.
    pub fn new(launcher: Arc<dyn Launcher>, fonts: text::FontCollection) -> RemoteTab {
        let grants = Grants::new();
        let jar: Arc<Mutex<Jar>> = Arc::default();
        let cache: Arc<net::Cache> = Arc::default();
        let hsts: Arc<Mutex<Hsts>> = Arc::default();
        let permissions: Arc<Permissions> = Arc::default();
        let storage = Arc::new(Mutex::new(Storage::new()));
        let network = NetworkService::new(
            client(&jar, &cache, &hsts, &permissions, false),
            grants.clone(),
        );
        let services = Arc::new(Services(vec![
            network.clone(),
            StorageService::new(storage.clone(), grants.clone()),
        ]));
        let (sender, events) = mpsc::channel();
        RemoteTab {
            fonts,
            launcher,
            grants,
            network,
            services,
            storage,
            jar,
            cache,
            hsts,
            permissions,
            renderer: None,
            next_renderer: 1,
            events,
            sender,
            waker: Arc::default(),
            clock: None,
            settings: Settings::default(),
            viewport: (0, 0),
            raster: (0, 0),
            state: ViewState::default(),
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

    /// The renderer showing the page, started with its view if there is
    /// none, its frame locked to `url`'s site.
    fn renderer_for(&mut self, url: &Url) -> Option<&Renderer> {
        if self.renderer.is_none() {
            let number = self.next_renderer;
            self.next_renderer += 1;
            let waker = self.waker.clone();
            let wake: crate::loader::Waker = Arc::new(move || {
                if let Some(wake) = waker
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .as_ref()
                {
                    wake();
                }
            });
            let serve: Arc<dyn super::Serve> = self.services.clone();
            match Renderer::start(
                self.launcher.as_ref(),
                number,
                self.sender.clone(),
                wake,
                Some(serve),
            ) {
                Ok(renderer) => self.renderer = Some(renderer),
                Err(error) => {
                    self.notice = Some(format!("No renderer could be started: {error}"));
                    return None;
                }
            }
            let renderer = self.renderer.as_ref().expect("started");
            let _ = renderer.send(&ToRenderer::CreateView {
                view: TOP_VIEW,
                frame: TOP_FRAME,
                settings: self.settings.clone(),
            });
            self.send_size();
        }
        let renderer = self.renderer.as_ref()?;
        // Until a renderer of each site shows its documents (step 6), the
        // one renderer is locked to the site navigated to.
        self.grants.relock(renderer.number(), Lock::of(url));
        self.grants.grant_frame(
            renderer.number(),
            TOP_FRAME,
            FrameGrant {
                top: url.origin(),
                cross_site: false,
            },
        );
        Some(renderer)
    }

    fn send(&self, message: &ToRenderer) {
        if let Some(renderer) = &self.renderer {
            let _ = renderer.send(message);
        }
    }

    fn input(&self, input: Input) {
        self.send(&ToRenderer::Input {
            view: TOP_VIEW,
            input,
        });
    }

    fn send_settings(&self) {
        self.send(&ToRenderer::Settings {
            view: TOP_VIEW,
            settings: self.settings.clone(),
        });
    }

    fn send_size(&self) {
        self.send(&ToRenderer::Resize {
            view: TOP_VIEW,
            width: self.viewport.0,
            height: self.viewport.1,
            raster_width: self.raster.0,
            raster_height: self.raster.1,
        });
    }

    /// The time on the session's clock.
    fn now(&self) -> Duration {
        self.clock.as_ref().map_or(Duration::ZERO, |clock| clock())
    }

    /// Takes what the renderers said; whether anything changed.
    fn take_events(&mut self) -> bool {
        let mut changed = false;
        while let Ok((number, event)) = self.events.try_recv() {
            let current = self.renderer.as_ref().map(Renderer::number);
            if current != Some(number) {
                continue;
            }
            changed = true;
            match event {
                Event::Ready => {}
                Event::Message(message) => {
                    if let Err(why) = self.take(*message)
                        && let Some(renderer) = &self.renderer
                    {
                        renderer.bad_message(why);
                    }
                }
                Event::Gone(why) => self.gone(why),
            }
        }
        changed
    }

    /// The renderer is gone: the page shows what it last showed, and
    /// says why.
    fn gone(&mut self, why: Gone) {
        if let Some(renderer) = self.renderer.take() {
            self.grants.forget(renderer.number());
        }
        if why != Gone::Shutdown {
            self.state.load = LoadState::Failed(LoadError::Lost);
            self.notice = Some(format!("The page's renderer stopped ({why:?})"));
        }
    }

    /// Takes a message of the renderer's; Err for one it may not send.
    fn take(&mut self, message: ToBrowser) -> Result<(), String> {
        match message {
            ToBrowser::State { view, state } => {
                check_view(view)?;
                self.undecodable = state
                    .undecodable
                    .as_deref()
                    .and_then(encoding::Encoding::for_label)
                    .map(encoding::Encoding::name);
                self.state = *state;
            }
            ToBrowser::Pixels {
                view,
                frame: _,
                width,
                height,
                first_row,
                bytes,
            } => {
                check_view(view)?;
                let row = frame_size(width, height)?;
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
                check_view(view)?;
                frame_size(width, height)?;
                if self.incoming.len() != width as usize * height as usize * 4 {
                    return Err("a frame of no pixels".into());
                }
                self.pixels.clone_from(&self.incoming);
                self.size = (width, height);
                self.fresh = true;
                self.frames += 1;
                self.send(&ToRenderer::FrameTaken { view, frame });
            }
            ToBrowser::Console { view, entries } => {
                check_view(view)?;
                self.console.extend(entries);
            }
            ToBrowser::Notice { view, text } => {
                check_view(view)?;
                self.notice = Some(text);
            }
            ToBrowser::FocusLeft { view } => {
                check_view(view)?;
                self.focus_left = true;
            }
            ToBrowser::Copied { view, text, cut } => {
                check_view(view)?;
                if let Some(text) = text {
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

    /// Kills the renderer, as a crash would end it (M34.4, M34.7).
    pub fn kill_renderer(&self) {
        if let Some(renderer) = &self.renderer {
            renderer.kill();
        }
    }

    /// The renderer's process's number, if one runs: for the tests and
    /// the checks.
    pub fn renderer_process(&self) -> Option<u32> {
        self.renderer.as_ref().map(Renderer::process_id)
    }
}

impl Drop for RemoteTab {
    fn drop(&mut self) {
        if let Some(renderer) = self.renderer.take() {
            renderer.shutdown();
        }
    }
}

fn check_view(view: u32) -> Result<(), String> {
    match view {
        TOP_VIEW => Ok(()),
        _ => Err(format!("view {view}, not the renderer's")),
    }
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
        if self.renderer_for(&url).is_some() {
            self.state.load = LoadState::Loading;
            self.send(&ToRenderer::Navigate {
                view: TOP_VIEW,
                url,
            });
        }
    }

    fn go_back(&mut self) {
        self.send(&ToRenderer::Traverse {
            view: TOP_VIEW,
            delta: -1,
        });
    }

    fn go_forward(&mut self) {
        self.send(&ToRenderer::Traverse {
            view: TOP_VIEW,
            delta: 1,
        });
    }

    fn reload(&mut self) {
        self.send(&ToRenderer::Reload { view: TOP_VIEW });
    }

    fn stop(&mut self) {
        self.send(&ToRenderer::Stop { view: TOP_VIEW });
    }

    fn can_go_back(&self) -> bool {
        self.state.can_go_back
    }

    fn can_go_forward(&self) -> bool {
        self.state.can_go_forward
    }

    fn url(&self) -> Option<&Url> {
        self.state.url.as_ref()
    }

    fn title(&self) -> String {
        self.state.title.clone()
    }

    fn load_state(&self) -> &LoadState {
        &self.state.load
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
        let permissions_wake = waker.clone();
        self.permissions.set_waker(permissions_wake);
        *self.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }

    fn set_clock(&mut self, clock: Clock) {
        self.clock = Some(clock);
    }

    fn run_tasks(&mut self) -> bool {
        let changed = self.take_events();
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
        self.state.cursor
    }

    fn link_at(&mut self, _: f32, _: f32) -> Option<Link> {
        self.state.hover.clone().map(Link::remote)
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
        self.send(&ToRenderer::Copy {
            view: TOP_VIEW,
            cut: false,
        });
        None
    }

    fn cut(&mut self) -> Option<String> {
        self.send(&ToRenderer::Cut { view: TOP_VIEW });
        None
    }

    fn paste(&mut self, text: &str) -> bool {
        self.input(Input::Paste {
            text: text.to_owned(),
        });
        true
    }

    fn can_paste(&self) -> bool {
        self.state.can_paste
    }

    fn caret_area(&mut self) -> Option<(f32, f32, f32, f32)> {
        self.state.caret
    }

    fn popup_open(&self) -> bool {
        self.state.popup
    }

    fn scrollbar(&mut self) -> Option<Scrollbar> {
        self.state.scrollbar
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
        self.send(&ToRenderer::Copy {
            view: TOP_VIEW,
            cut: true,
        });
        None
    }

    fn take_copied(&mut self) -> Option<(String, bool)> {
        self.copied.take()
    }

    fn cut_copied(&mut self) {
        self.send(&ToRenderer::Cut { view: TOP_VIEW });
    }
}
