//! A renderer's views (M34.5, docs/design/processes.md): each an engine
//! showing a tab's page for the browser, fetching and keeping its
//! cookies and storage through the browser, told the input and settings
//! the session gives, and reporting what the session reads of it: its
//! frames, row by row as they change, its state, its console and
//! notices. A view draws when its engine wants a frame, at most one
//! frame a [`FRAME_INTERVAL`], and never while the browser has not taken
//! the last one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::link::Link;
use crate::ipc::{Bytes, Hole, Input, MAX_CHUNK, Settings, ToBrowser, ToRenderer, ViewState};
use crate::{Engine, FrameTime, KeyOutcome};

/// The least time between two frames of a view.
pub(crate) const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// The most raster pixels a view's frame has (docs/design/processes.md,
/// `MAX_SURFACE_PIXELS`): one bigger is drawn clipped.
pub const MAX_SURFACE_PIXELS: u32 = 4096 * 4096;

struct View {
    engine: Engine,
    /// Whether frames of other renderers were in its last frame (M34):
    /// their holes are sent with each frame until none are.
    holes: bool,
    /// The history last reported.
    history: (Vec<net::Url>, usize),
    /// The raster size the browser draws the page at.
    raster: (u32, u32),
    /// The last frame sent, row by row, its size and number, and whether
    /// the browser took it.
    sent: Vec<u8>,
    sent_size: (u32, u32),
    frame: u64,
    taken: bool,
    drawn: Option<Instant>,
    /// A frame is owed: a resize, or the browser lost the last.
    owed: bool,
    /// The state last reported, and where the pointer last was.
    state: ViewState,
    pointer: Option<(f32, f32)>,
}

/// A renderer's views and what they share.
pub(crate) struct Host {
    link: Arc<Link>,
    fonts: Option<text::FontCollection>,
    began: Instant,
    views: HashMap<u32, View>,
    wake: Arc<dyn Fn() + Send + Sync>,
    /// Since when its views have all been hidden (M35.5).
    idle: Idle,
    /// The files of the browser's fonts, when it sent them and no fonts
    /// were given (M35.6), read at the first view.
    font_files: Option<text::FontFiles>,
    /// What every view's engine tells its changes of Web Storage areas
    /// through (M37), so that the renderer's documents hear of each
    /// other's; the areas themselves are the browser's.
    storage: Arc<Mutex<crate::storage::Storage>>,
}

/// Since when a renderer's views have all been hidden, and whether it
/// has trimmed since (M35.5): it trims once they have been for
/// `after`, and again only after one is shown and hidden.
#[derive(Debug)]
pub(crate) struct Idle {
    since: Option<Instant>,
    trimmed: bool,
    after: Duration,
}

impl Idle {
    pub(crate) fn new(after: Duration) -> Idle {
        Idle {
            since: None,
            trimmed: false,
            after,
        }
    }

    /// Whether to trim now, at `now`, the views all hidden or not.
    pub(crate) fn tick(&mut self, now: Instant, hidden: bool) -> bool {
        if !hidden {
            self.since = None;
            self.trimmed = false;
            return false;
        }
        let since = *self.since.get_or_insert(now);
        if self.trimmed || now.duration_since(since) < self.after {
            return false;
        }
        self.trimmed = true;
        true
    }

    /// When the trim is due, while one is to come.
    pub(crate) fn due(&self) -> Option<Instant> {
        self.since
            .filter(|_| !self.trimmed)
            .map(|since| since + self.after)
    }
}

impl Host {
    /// A host whose views draw with `fonts` (the system's when none,
    /// loaded at the first view), waking the renderer's loop with `wake`.
    pub(crate) fn new(
        link: Arc<Link>,
        fonts: Option<text::FontCollection>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Host {
        Host {
            link,
            fonts,
            began: Instant::now(),
            views: HashMap::new(),
            wake,
            idle: Idle::new(crate::IDLE_TRIM_AFTER),
            font_files: None,
            storage: Arc::new(Mutex::new(crate::storage::Storage::new())),
        }
    }

    fn send(&self, message: &ToBrowser) {
        // A browser gone ends the renderer through its input.
        let _ = self.link.send(message);
    }

    fn fonts(&mut self) -> Result<text::FontCollection, String> {
        if self.fonts.is_none() {
            // The files the browser found, read alone (M35.6); else the
            // system's, scanned.
            let given = self
                .font_files
                .take()
                .and_then(|files| text::FontCollection::from_files(&files).ok());
            let fonts = match given {
                Some(fonts) => fonts,
                None => {
                    text::FontCollection::system().map_err(|error| format!("fonts: {error}"))?
                }
            };
            self.fonts = Some(fonts);
        }
        Ok(self.fonts.clone().expect("loaded"))
    }

    /// A view of `frame`, the browser's number for its frame, shown under
    /// `settings`.
    fn create(&mut self, view: u32, frame: u32, settings: &Settings) -> Result<(), String> {
        let mut engine = Engine::with_fonts(self.fonts()?);
        engine.set_link(self.link.clone(), frame);
        self.add(view, engine, settings)
    }

    /// A view of the frame root `key`, under `ancestors` (M34).
    fn create_frame(
        &mut self,
        view: u32,
        key: u32,
        settings: &Settings,
        ancestors: Vec<crate::ipc::Ancestor>,
        sandbox: u32,
    ) -> Result<(), String> {
        let ancestors: Vec<_> = ancestors
            .into_iter()
            .map(|ancestor| (ancestor.key, ancestor.origin, ancestor.url))
            .collect();
        let engine = Engine::frame_root(
            self.fonts()?,
            self.link.clone(),
            key,
            &ancestors,
            crate::sandbox::Sandbox::from_bits(sandbox),
        );
        self.add(view, engine, settings)
    }

    fn add(&mut self, view: u32, mut engine: Engine, settings: &Settings) -> Result<(), String> {
        if self.views.contains_key(&view) {
            return Err(format!("view {view} again"));
        }
        engine.share_storage(self.storage.clone(), 0);
        let wake = self.wake.clone();
        engine.set_waker(move || wake());
        let began = self.began;
        engine.set_clock(Arc::new(move || began.elapsed()));
        apply(&mut engine, settings);
        self.views.insert(
            view,
            View {
                engine,
                holes: false,
                history: (Vec::new(), 0),
                raster: (0, 0),
                sent: Vec::new(),
                sent_size: (0, 0),
                frame: 0,
                taken: true,
                drawn: None,
                owed: true,
                state: ViewState::default(),
                pointer: None,
            },
        );
        Ok(())
    }

    fn view(&mut self, view: u32) -> Result<&mut View, String> {
        self.views
            .get_mut(&view)
            .ok_or_else(|| format!("no view {view}"))
    }

    /// Carries out `message`; false when the renderer is to end. Err for
    /// what the browser never sends.
    pub(crate) fn handle(&mut self, message: ToRenderer) -> Result<bool, String> {
        match message {
            ToRenderer::Shutdown {} => return Ok(false),
            ToRenderer::Hello { .. } => return Err("a second hello".into()),
            ToRenderer::Fonts { primary, fallbacks } => {
                if self.fonts.is_none() {
                    self.font_files = Some(text::FontFiles { primary, fallbacks });
                }
            }
            ToRenderer::CreateView {
                view,
                frame,
                settings,
            } => self.create(view, frame, &settings)?,
            ToRenderer::DestroyView { view } => {
                self.views.remove(&view);
            }
            // The DevTools panel's page is shown by its URL from the
            // browser alone (M36): no navigation reaches it.
            ToRenderer::Navigate { view, url } if url.as_str() == devtools::PANEL_URL => {
                self.view(view)?.engine.show_devtools_panel();
            }
            ToRenderer::Navigate { view, url } => self.view(view)?.engine.navigate(url),
            ToRenderer::DevTools { view, message } => {
                self.view(view)?.engine.devtools_send(message);
            }
            ToRenderer::Inspect { view, message } => {
                self.view(view)?.engine.inspect(&message);
            }
            ToRenderer::StorageChanged {
                local,
                origin,
                partition,
                key,
                old,
                new,
                url,
            } => {
                let area = crate::storage::AreaKey {
                    origin: origin.clone(),
                    partition: partition.clone(),
                };
                self.storage
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .broadcast(crate::storage::Change {
                        scope: crate::storage::Scope::of(local, 0),
                        origin: area.serialization(),
                        key,
                        old,
                        new,
                        url,
                        page: 0,
                        source: crate::storage::Source::Elsewhere,
                        owner: origin,
                        partition,
                    });
            }
            ToRenderer::NavigateHandedOver {
                view,
                url,
                handover,
            } => {
                let view = self.view(view)?;
                view.engine.hand_over(&url, handover);
                view.engine.navigate(url);
            }
            ToRenderer::DropForward { view } => self.view(view)?.engine.drop_forward(),
            ToRenderer::CreateFrameView {
                view,
                key,
                settings,
                ancestors,
                sandbox,
            } => self.create_frame(view, key, &settings, ancestors, sandbox)?,
            ToRenderer::FrameRemote { key } => {
                for view in self.views.values_mut() {
                    if view.engine.mark_remote(key) {
                        view.owed = true;
                        break;
                    }
                }
            }
            ToRenderer::FrameLoaded { key } => {
                for view in self.views.values_mut() {
                    if view.engine.remote_loaded(key) {
                        break;
                    }
                }
            }
            ToRenderer::FrameTask { from, to, task } => self.deliver(from, to, task),
            ToRenderer::Restore {
                view,
                entries,
                index,
            } => {
                self.view(view)?
                    .engine
                    .restore_history(entries, index as usize);
            }
            ToRenderer::Traverse { view, delta } => {
                self.view(view)?.engine.traverse(delta as isize);
            }
            ToRenderer::Reload { view } => self.view(view)?.engine.reload(),
            ToRenderer::ReloadBypassingCache { view } => {
                self.view(view)?.engine.reload_bypassing_cache();
            }
            ToRenderer::Stop { view } => self.view(view)?.engine.stop(),
            ToRenderer::Resize {
                view,
                width,
                height,
                raster_width,
                raster_height,
            } => {
                let view = self.view(view)?;
                view.engine.set_viewport(width, height);
                view.raster = clipped(raster_width, raster_height);
                view.owed = true;
            }
            ToRenderer::Settings { view, settings } => {
                let view = self.view(view)?;
                apply(&mut view.engine, &settings);
                view.owed = true;
            }
            ToRenderer::Input { view, input } => {
                let link = self.link.clone();
                let target = self.view(view)?;
                if let Some(answer) = give(target, input) {
                    let _ = link.send(&answer(view));
                }
            }
            ToRenderer::FrameTaken { view, frame } => {
                let view = self.view(view)?;
                if frame == view.frame {
                    view.taken = true;
                }
            }
            ToRenderer::Copy { view, cut } => {
                let text = self.view(view)?.engine.copy();
                self.send(&ToBrowser::Copied { view, text, cut });
            }
            ToRenderer::Cut { view } => {
                self.view(view)?.engine.cut();
            }
            other => return Err(format!("{other:?} out of place")),
        }
        Ok(true)
    }

    /// Puts what frame `from` of another renderer posted to frame `to` in
    /// that frame's inbox, the frame standing for the sender as its source.
    fn deliver(&mut self, from: Option<u32>, to: u32, task: crate::ipc::WireTask) {
        for view in self.views.values_mut() {
            let Some(engine) = view.engine.engine_of_key(to) else {
                continue;
            };
            let tree = engine.frame.tree.clone();
            let posted = {
                let mut tree = crate::frames::lock(&tree);
                let Some(local) = tree.local_of(to) else {
                    return;
                };
                let source = from
                    .and_then(|from| tree.local_of(from))
                    .unwrap_or(crate::frames::TOP);
                tree.post(local, task.into_task(source))
            };
            if posted {
                (self.wake)();
            }
            return;
        }
    }

    /// The views' turn: what arrived and the tasks due, then the frames
    /// and the state the browser is to have.
    pub(crate) fn turn(&mut self) {
        let now = Instant::now();
        let time = self.began.elapsed();
        let ids: Vec<u32> = self.views.keys().copied().collect();
        for id in ids {
            let Some(view) = self.views.get_mut(&id) else {
                continue;
            };
            view.engine.poll_network();
            if view.engine.next_task_at().is_some_and(|due| due <= time) {
                view.engine.run_tasks();
            }
            let mut out = Vec::new();
            let console = view.engine.take_console();
            if !console.is_empty() {
                out.push(ToBrowser::Console {
                    view: id,
                    entries: console,
                });
            }
            if let Some(text) = view.engine.take_notice() {
                out.push(ToBrowser::Notice { view: id, text });
            }
            // The DevTools' messages (M36): the panel's, or the answers of
            // the page it inspects.
            for message in view.engine.take_devtools() {
                out.push(ToBrowser::DevTools { view: id, message });
            }
            for message in view.engine.take_inspected() {
                out.push(ToBrowser::Inspected { view: id, message });
            }
            // The tabs its documents asked for, and its closing (M37).
            for tab in view.engine.take_opened() {
                out.push(ToBrowser::OpenTab {
                    view: id,
                    url: tab.url,
                    show: tab.show,
                    by_script: tab.by_script,
                });
            }
            if view.engine.take_close_asked() {
                out.push(ToBrowser::CloseTab { view: id });
            }
            for key in view.engine.take_remote_drops() {
                out.push(ToBrowser::FrameDropped { key });
            }
            draw(view, id, now, time, &mut out);
            let history = view.engine.history_urls();
            if history != view.history {
                view.history = history.clone();
                out.push(ToBrowser::History {
                    view: id,
                    entries: history.0,
                    index: history.1 as u32,
                });
            }
            let state = state_of(view);
            if state != view.state {
                view.state = state.clone();
                out.push(ToBrowser::State {
                    view: id,
                    state: Box::new(state),
                });
            }
            for message in &out {
                self.send(message);
            }
        }
        // A renderer whose views have all been hidden for a while lets
        // go of what it can make again when one is shown (M35.5).
        let hidden =
            !self.views.is_empty() && self.views.values().all(|view| view.engine.is_hidden());
        if self.idle.tick(now, hidden) {
            for view in self.views.values_mut() {
                view.engine.trim();
            }
        }
    }

    /// How long the loop may wait for a message before the next turn:
    /// the soonest task or frame due; none when nothing is.
    pub(crate) fn wait(&self) -> Option<Duration> {
        let time = self.began.elapsed();
        self.views
            .values()
            .filter_map(|view| {
                let task = view.engine.next_task_at();
                let frame = match view.engine.needs_frame() || view.owed {
                    true if view.taken => {
                        let next = view.drawn.map_or(Duration::ZERO, |drawn| {
                            FRAME_INTERVAL.saturating_sub(drawn.elapsed())
                        });
                        Some(time + next)
                    }
                    _ => view.engine.next_frame_at(),
                };
                [task, frame].into_iter().flatten().min()
            })
            .chain(
                self.idle
                    .due()
                    .map(|due| due.saturating_duration_since(self.began)),
            )
            .min()
            .map(|due| due.saturating_sub(time))
    }
}

/// `width` by `height` within [`MAX_SURFACE_PIXELS`].
fn clipped(width: u32, height: u32) -> (u32, u32) {
    let width = width.min(MAX_SURFACE_PIXELS);
    let most = MAX_SURFACE_PIXELS / width.max(1);
    (width, height.min(most))
}

fn apply(engine: &mut Engine, settings: &Settings) {
    engine.set_preferences(settings.preferences);
    engine.set_scripting(settings.scripting);
    engine.set_compat_user_agent(settings.compat_user_agent);
    engine.set_hidden(settings.hidden);
    engine.set_window_focus(settings.focused);
    if settings.scale.is_finite() && settings.scale > 0.0 {
        engine.set_scale_factor(settings.scale);
    }
    engine.set_modifiers(settings.modifiers);
    if let Some(bytes) = settings.layer_budget {
        engine.set_layer_budget(usize::try_from(bytes).unwrap_or(usize::MAX));
    }
    engine.set_languages(settings.languages.clone());
}

/// Gives `input` to `view`'s engine; what the browser is to be told of
/// it, made for the view's number.
fn give(view: &mut View, input: Input) -> Option<fn(u32) -> ToBrowser> {
    let engine = &mut view.engine;
    match input {
        Input::PointerMove { x, y } => {
            engine.pointer_move(x, y);
            view.pointer = Some((x, y));
        }
        Input::PointerLeave {} => {
            engine.pointer_leave();
            view.pointer = None;
        }
        Input::PointerDown {
            x,
            y,
            button,
            count,
        } => engine.pointer_down(x, y, button, count),
        Input::PointerUp {
            x,
            y,
            button,
            count,
        } => engine.pointer_up(x, y, button, count),
        Input::Wheel { x, y, dy } => engine.wheel(x, y, dy),
        Input::KeyDown { key } => {
            if engine.key_down(&key) == KeyOutcome::FocusLeft {
                return Some(|view| ToBrowser::FocusLeft { view });
            }
        }
        Input::KeyUp { key } => engine.key_up(&key),
        Input::FocusNext { backward } => {
            engine.focus_next(backward);
        }
        Input::InsertText { text } => {
            engine.insert_text(&text);
        }
        Input::Paste { text } => {
            engine.paste(&text);
        }
        Input::ScrollBy { dy } => engine.scroll_by(dy),
        Input::ScrollToFraction { fraction } => engine.scroll_to_fraction(fraction),
    }
    None
}

/// What the session reads of `view` now.
fn state_of(view: &mut View) -> ViewState {
    let engine = &mut view.engine;
    let (cursor, hover) = match view.pointer {
        Some((x, y)) => (
            engine.cursor_at(x, y),
            engine.link_at(x, y).map(|link| link.url().clone()),
        ),
        None => (crate::Cursor::Default, None),
    };
    ViewState {
        url: engine.url().cloned(),
        title: engine.title(),
        load: engine.load_state().clone(),
        undecodable: engine.undecodable_encoding().map(str::to_owned),
        can_go_back: engine.can_go_back(),
        can_go_forward: engine.can_go_forward(),
        popup: engine.popup_open().is_some(),
        can_paste: engine.can_paste(),
        caret: engine.caret_area(),
        scrollbar: engine.scrollbar(),
        cursor,
        hover,
    }
}

/// Draws `view` if a frame is wanted and may be drawn now, and puts in
/// `out` its rows that changed since the last frame sent, and its end.
fn draw(view: &mut View, id: u32, now: Instant, time: Duration, out: &mut Vec<ToBrowser>) {
    let wanted = view.owed || view.engine.needs_frame();
    let (width, height) = view.raster;
    let paced = view
        .drawn
        .is_none_or(|drawn| now.duration_since(drawn) >= FRAME_INTERVAL);
    if !wanted || !view.taken || !paced || width == 0 || height == 0 {
        return;
    }
    view.drawn = Some(now);
    view.owed = false;
    // The frames of other renderers it shows, before its pixels (M34).
    let scale = view.engine.scale_factor();
    let holes: Vec<Hole> = view
        .engine
        .remote_holes()
        .into_iter()
        .map(|(key, [x, y, w, h])| Hole {
            key,
            x: (x * scale).round() as i32,
            y: (y * scale).round() as i32,
            width: (w * scale).round().max(0.0) as u32,
            height: (h * scale).round().max(0.0) as u32,
        })
        .collect();
    let pixels = view.engine.frame_at(width, height, FrameTime::at(time));
    if pixels.len() != width as usize * height as usize * 4 {
        return;
    }
    let row = width as usize * 4;
    let resized = view.sent_size != (width, height);
    if resized {
        view.sent = vec![0; pixels.len()];
        view.sent_size = (width, height);
    }
    view.frame += 1;
    view.taken = false;
    if !holes.is_empty() || view.holes {
        view.holes = !holes.is_empty();
        out.push(ToBrowser::Holes {
            view: id,
            frame: view.frame,
            holes,
        });
    }
    let rows_per_piece = (MAX_CHUNK / row).max(1);
    let mut first = None;
    let rows = height as usize;
    for y in 0..=rows {
        let changed = y < rows && {
            let span = y * row..(y + 1) * row;
            resized || pixels[span.clone()] != view.sent[span]
        };
        match (changed, first) {
            (true, None) => first = Some(y),
            (false, Some(start)) => {
                let mut at = start;
                while at < y {
                    let end = (at + rows_per_piece).min(y);
                    out.push(ToBrowser::Pixels {
                        view: id,
                        frame: view.frame,
                        width,
                        height,
                        first_row: at as u32,
                        bytes: Bytes(pixels[at * row..end * row].to_vec()),
                    });
                    at = end;
                }
                first = None;
            }
            _ => {}
        }
    }
    view.sent.copy_from_slice(pixels);
    out.push(ToBrowser::Frame {
        view: id,
        frame: view.frame,
        width,
        height,
    });
}

#[cfg(test)]
mod tests {
    use super::Idle;
    use std::time::{Duration, Instant};

    #[test]
    fn a_renderer_trims_once_its_views_have_all_been_hidden_a_while() {
        // M35.5.
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut idle = Idle::new(Duration::from_secs(30));
        assert!(!idle.tick(at(0), false));
        assert_eq!(idle.due(), None, "shown: nothing due");
        assert!(!idle.tick(at(1), true));
        assert_eq!(idle.due(), Some(at(31)));
        assert!(!idle.tick(at(30), true), "not yet");
        assert!(idle.tick(at(31), true), "now");
        assert!(!idle.tick(at(90), true), "once");
        assert_eq!(idle.due(), None);
        // Shown, and hidden again: another trim, a while after.
        assert!(!idle.tick(at(100), false));
        assert!(!idle.tick(at(101), true));
        assert!(idle.tick(at(131), true));
    }

    #[test]
    fn a_host_reads_the_font_files_the_browser_sent() {
        // M35.6: those alone, not the system's scanned; and none when it
        // was given its fonts.
        use super::{Host, Link};
        use crate::ipc::{Sender, ToRenderer};
        use std::sync::Arc;
        let font = concat!(env!("CARGO_MANIFEST_DIR"), "/../text/fonts/DejaVuSans.ttf");
        let sent = || ToRenderer::Fonts {
            primary: (font.to_owned(), 0),
            fallbacks: Vec::new(),
        };
        let host = |fonts| {
            Host::new(
                Link::new(Sender::new(std::io::sink())),
                fonts,
                Arc::new(|| {}),
            )
        };
        let mut sent_to = host(None);
        sent_to.handle(sent()).expect("taken");
        let files = sent_to.fonts().expect("read").files().expect("of a file");
        assert_eq!(files.primary, (font.to_owned(), 0));
        let given = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("parses");
        let mut given = host(Some(given));
        given.handle(sent()).expect("taken");
        assert_eq!(given.fonts().expect("given").files(), None);
        // A file gone: the system's, scanned, where it has fonts.
        if let Ok(system) = text::FontCollection::system() {
            let mut gone = host(None);
            let lost = ToRenderer::Fonts {
                primary: ("no such font file".to_owned(), 0),
                fallbacks: Vec::new(),
            };
            gone.handle(lost).expect("taken");
            assert_eq!(gone.fonts().expect("scanned").files(), system.files());
        }
    }

    #[test]
    fn a_host_trims_when_its_views_are_all_hidden_and_wakes_for_it() {
        // M35.5: the idle clock is the views', and the loop's wait.
        use super::{Host, Link};
        use crate::ipc::{Sender, Settings, ToRenderer};
        use std::sync::Arc;
        let host = |after| {
            let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
                .expect("the bundled font parses");
            let link = Link::new(Sender::new(std::io::sink()));
            let mut host = Host::new(link, Some(fonts), Arc::new(|| {}));
            host.idle = Idle::new(after);
            let hidden = Settings {
                hidden: true,
                ..Settings::default()
            };
            for (view, settings) in [(1, Settings::default()), (2, hidden.clone())] {
                let create = ToRenderer::CreateView {
                    view,
                    frame: view,
                    settings,
                };
                host.handle(create).expect("made");
                let resize = ToRenderer::Resize {
                    view,
                    width: 40,
                    height: 30,
                    raster_width: 40,
                    raster_height: 30,
                };
                host.handle(resize).expect("sized");
            }
            // Their first frames drawn, nothing else is due.
            host.turn();
            assert_eq!(host.idle.due(), None, "one shown");
            let hide = ToRenderer::Settings {
                view: 1,
                settings: hidden,
            };
            host.handle(hide).expect("hidden");
            host.turn();
            host
        };
        let layers = |host: &Host| {
            let mut bytes = host.views.values().map(|view| view.engine.layer_bytes());
            (bytes.next().unwrap_or(0), bytes.next().unwrap_or(0))
        };
        let trimmed = host(Duration::ZERO);
        assert!(trimmed.idle.trimmed, "at once");
        assert_eq!(layers(&trimmed), (0, 0), "each view's engine trimmed");
        let host = host(Duration::from_secs(3600));
        assert!(!host.idle.trimmed, "not yet");
        let (one, two) = layers(&host);
        assert!(one > 0 && two > 0, "its frames held until then");
        let wait = host.wait().expect("woken for the trim");
        assert!(wait > Duration::from_secs(3500), "{wait:?}");
    }
}
