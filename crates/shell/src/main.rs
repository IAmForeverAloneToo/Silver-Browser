//! The shell: the crate that talks to the OS for windows, input and
//! the clipboard; `net` alone opens connections and reads resources by
//! URL.
//!
//! Owns the window, the event loop, and the present pipeline (winit +
//! softbuffer). What the window shows and what input does to it is the
//! session's (`session.rs`): the toolbar above the page, the status
//! bubble over it, and the keys (`keys.rs`), the pointer, the input
//! method and the clipboard made calls on the engine and the toolbar
//! (M15, docs/design/navigation.md). This file takes winit's events into
//! the session's own types, presents its frames, and gives the window
//! the title, the pointer's shape and the input method's state the
//! session asks for. Each frame is drawn for the time on the shell's
//! clock (M13.4), and a frame is asked for only when the session wants
//! one: after input that changed something, or while a scroll animates.
//!
//! It runs as `silver-browser [URL or path]` (M14, docs/design/net.md,
//! "The shell"): an argument naming an existing file loads that file,
//! and any other goes through the address field's rules; with none,
//! `about:silver` shows with the address field focused (M15). The
//! user's preferences that pages may ask about (M20.3) come from the
//! command line where it gives them, and otherwise from the system:
//! the colour scheme from the window's theme, as it changes, and
//! reduced motion from `platform`, read again when the window regains
//! the focus, since no event says it changed. With
//! `--print-to-pdf <path>` it opens no window: it loads the page,
//! waits for it, prints it to the file and exits (M20.4), as the
//! reference browser's headless mode does. Each
//! finished fetch wakes the event loop with a user event, on which the
//! engine takes what arrived; a load the command line asked for that
//! fails shows the engine's error page, and is reported on standard
//! error.

// `unsafe` in the platform module alone (M33.5), which allows it.
#![deny(unsafe_code)]

mod keys;
mod platform;
mod session;

use std::ffi::OsString;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use engine::address::Address;
use engine::{
    Backend, ColorScheme, Engine, FRAME_BUDGET, FrameTime, LoadState, Preferences, PrintOptions,
    Profile, Url,
};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{
    ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, StartCause, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::NamedKey;
use winit::window::{CursorIcon, Theme, Window, WindowId};

use keys::{Key, Modifiers, Platform};
use session::{Button, Clipboard, Session, Shape};

/// Window configuration.
const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 800;

/// The user event a finished fetch wakes the event loop with.
#[derive(Debug, Clone, Copy)]
struct NetworkWake;

/// The browser application: window + surface + the session.
///
/// The window lives behind an `Arc` because softbuffer's `Context` and
/// `Surface` each hold their own handle to it (raw-window-handle 0.6
/// provides the impls for `Arc<T>`).
struct Browser {
    window: Option<Arc<Window>>,
    context: Option<softbuffer::Context<Arc<Window>>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    /// The size the surface was last given.
    surface_size: SurfaceSize,
    session: Session,
    /// The clock frames are drawn by and clicks counted on: the time
    /// since the browser began.
    start: Instant,
    /// What the window was last given, so that it is given only what
    /// changed.
    shown: Shown,
    /// The preferences the command line gave.
    asked: Asked,
    /// The window's theme, when the system says it.
    theme: Option<Theme>,
    /// When the last frame began.
    drawn: Option<Instant>,
}

/// The size a surface was last given: it is given another only when
/// the window's changes, since a resize makes its buffers again.
#[derive(Debug, Default)]
struct SurfaceSize(Option<(NonZeroU32, NonZeroU32)>);

impl SurfaceSize {
    /// Whether the surface is to be resized to `size`, which it is
    /// taken to have from now on.
    fn wants(&mut self, size: (NonZeroU32, NonZeroU32)) -> bool {
        if self.0 == Some(size) {
            return false;
        }
        self.0 = Some(size);
        true
    }
}

/// The window's state the session decides.
#[derive(Debug, Default, PartialEq)]
struct Shown {
    title: String,
    shape: Option<Shape>,
    /// Whether the input method is on, and where the caret is.
    ime: Option<Option<(i32, i32, u32, u32)>>,
}

impl Browser {
    fn new(session: Session, asked: Asked, start: Instant) -> Self {
        Browser {
            window: None,
            context: None,
            surface: None,
            surface_size: SurfaceSize::default(),
            session,
            start,
            shown: Shown::default(),
            asked,
            theme: None,
            drawn: None,
        }
    }

    /// Gives the session the user's preferences as they now stand.
    fn read_preferences(&mut self) {
        let preferences = preferences(self.asked, self.theme, platform::reduced_motion());
        self.session.set_preferences(preferences);
    }

    /// Renders one frame and presents it, then draws ahead in the time
    /// the frame's budget leaves (M19.4). Failures skip the frame
    /// (logged) rather than taking the process down.
    fn draw(&mut self) {
        let began = Instant::now();
        self.drawn = Some(began);
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return; // Zero-sized window (e.g. minimized).
        };
        if self.surface_size.wants((w, h))
            && let Err(e) = surface.resize(w, h)
        {
            self.surface_size = SurfaceSize::default();
            eprintln!("surface resize failed: {e}");
            return;
        }
        match surface.buffer_mut() {
            Ok(mut buffer) => {
                let time = FrameTime::at(self.start.elapsed());
                self.session
                    .draw(size.width, size.height, time, &mut buffer);
                if let Err(e) = buffer.present() {
                    eprintln!("present failed: {e}");
                }
                self.session.draw_ahead(began + FRAME_BUDGET);
            }
            Err(e) => eprintln!("buffer acquisition failed: {e}"),
        }
    }

    /// Gives the window what the session now asks for — its title, the
    /// pointer's shape, the input method — and asks for a frame when
    /// the session wants one; prints what is to be reported.
    fn update(&mut self, window: &Window) {
        for report in self.session.take_reports() {
            eprintln!("silver-browser: {report}");
        }
        if self.session.title() != self.shown.title {
            self.shown.title = self.session.title().to_owned();
            window.set_title(&self.shown.title);
        }
        let shape = self.session.shape();
        if self.shown.shape != Some(shape) {
            self.shown.shape = Some(shape);
            window.set_cursor_visible(shape != Shape::Page(engine::Cursor::None));
            window.set_cursor(match shape {
                Shape::Arrow => CursorIcon::Default,
                Shape::Hand => CursorIcon::Pointer,
                Shape::Text => CursorIcon::Text,
                Shape::Page(cursor) => cursor_icon(cursor),
            });
        }
        // The session's CSS pixels as the window's device pixels.
        let scale = self.session.scale();
        let ime = self.session.ime_area().map(|(x, y, w, h)| {
            let (x, y, w, h) = (x * scale, y * scale, w * scale, h * scale);
            (
                x.round() as i32,
                y.round() as i32,
                w.ceil().max(1.0) as u32,
                h.ceil() as u32,
            )
        });
        let before = self.shown.ime.replace(ime);
        if before.is_none_or(|before| before.is_some() != ime.is_some()) {
            window.set_ime_allowed(ime.is_some());
        }
        if let Some((x, y, w, h)) = ime
            && before != Some(ime)
        {
            window.set_ime_cursor_area(PhysicalPosition::new(x, y), PhysicalSize::new(w, h));
        }
        if self.session.needs_frame() && frame_due(self.drawn, Instant::now()).is_none() {
            window.request_redraw();
        }
    }
}

/// When a frame the page wants may be drawn, if not now: a frame's
/// budget after the last began, sixty a second as a display shows them,
/// since a software surface presents at once, with no vertical sync to
/// wait on (M31: a light page's animation frames came 270 a second).
fn frame_due(drawn: Option<Instant>, now: Instant) -> Option<Instant> {
    drawn
        .map(|drawn| drawn + FRAME_BUDGET)
        .filter(|&due| due > now)
}

impl ApplicationHandler<NetworkWake> for Browser {
    /// The time the session asked to be woken at has come (M22.3): the
    /// end of an animation's delay, which wants a frame.
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        if let StartCause::ResumeTimeReached { .. } = cause
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
    }

    /// Runs the page's tasks that are due (M26), then waits for the next
    /// event, or, when the session wants no frame now but will at a time,
    /// or has a task due later, until then: an animation waiting out its
    /// delay draws no frame before it ends, and a timer runs when it is
    /// due.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.session.tick();
        if let Some(window) = self.window.clone() {
            self.update(&window);
        }
        // A frame wanted now waits for the next tick.
        let frame = match self.session.needs_frame() {
            true => frame_due(self.drawn, Instant::now()).map(|due| due - self.start),
            false => self.session.next_frame_at(),
        };
        let at = match (frame, self.session.next_task_at()) {
            (Some(frame), Some(task)) => Some(frame.min(task)),
            (frame, task) => frame.or(task),
        };
        let flow = match at {
            Some(at) => ControlFlow::WaitUntil(self.start + at),
            None => ControlFlow::Wait,
        };
        event_loop.set_control_flow(flow);
    }

    /// A fetch finished: the engine takes what arrived.
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _wake: NetworkWake) {
        self.session.network();
        if let Some(window) = self.window.clone() {
            self.update(&window);
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return; // Already running (e.g. resume after suspend).
        }

        let attributes = Window::default_attributes()
            .with_title(self.session.title())
            .with_inner_size(PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT))
            .with_resizable(true);

        match event_loop.create_window(attributes) {
            Ok(window) => {
                let window = Arc::new(window);
                match softbuffer::Context::new(window.clone()) {
                    Ok(context) => match softbuffer::Surface::new(&context, window.clone()) {
                        Ok(surface) => {
                            let size = window.inner_size();
                            self.session.set_scale(
                                window.scale_factor() as f32,
                                size.width,
                                size.height,
                            );
                            self.theme = window.theme();
                            self.read_preferences();
                            self.context = Some(context);
                            self.surface = Some(surface);
                            self.surface_size = SurfaceSize::default();
                            self.window = Some(window.clone());
                            self.update(&window);
                            window.request_redraw();
                        }
                        Err(e) => {
                            eprintln!("softbuffer surface creation failed: {e}");
                            event_loop.exit();
                        }
                    },
                    Err(e) => {
                        eprintln!("softbuffer context creation failed: {e}");
                        event_loop.exit();
                    }
                }
            }
            Err(e) => {
                eprintln!("window creation failed: {e}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.clone() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::Resized(size) => {
                if let (Some(w), Some(h)) =
                    (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                    && let Some(surface) = self.surface.as_mut()
                    && self.surface_size.wants((w, h))
                    && let Err(e) = surface.resize(w, h)
                {
                    self.surface_size = SurfaceSize::default();
                    eprintln!("surface resize failed: {e}");
                }
                self.session.resized(size.width, size.height);
                window.request_redraw();
            }
            // The window moved to a display of another scale, or the
            // display's changed (M32.4); its new size comes as a resize.
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let size = window.inner_size();
                self.session
                    .set_scale(scale_factor as f32, size.width, size.height);
                window.request_redraw();
            }
            WindowEvent::ThemeChanged(theme) => {
                self.theme = Some(theme);
                self.read_preferences();
            }
            WindowEvent::Focused(focused) => {
                self.session.set_window_focus(focused);
                if focused {
                    self.read_preferences();
                }
            }
            // Covered or minimized: the page is hidden (M26).
            WindowEvent::Occluded(occluded) => {
                self.session.set_hidden(occluded);
                window.request_redraw();
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                let state = modifiers.state();
                self.session.modifiers(Modifiers {
                    shift: state.shift_key(),
                    ctrl: state.control_key(),
                    alt: state.alt_key(),
                    logo: state.super_key(),
                });
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let page = page_key(&event);
                match event.state {
                    ElementState::Pressed => {
                        self.session.key_down(key_of(&event.logical_key), page);
                    }
                    ElementState::Released => self.session.key_up(page),
                }
            }
            WindowEvent::Ime(ime) => match ime {
                Ime::Preedit(text, _) => self.session.preedit(&text),
                Ime::Commit(text) => self.session.commit(&text),
                Ime::Enabled | Ime::Disabled => self.session.preedit(""),
            },
            WindowEvent::MouseWheel { delta, .. } => {
                // Positive deltas reveal content above (the content
                // moves down), so the scroll offset decreases.
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => -lines * session::LINE_PX,
                    // Device pixels, taken as CSS pixels by the scale.
                    MouseScrollDelta::PixelDelta(p) => -p.y as f32 / self.session.scale(),
                };
                self.session.wheel(dy);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.session
                    .pointer_moved(position.x as f32, position.y as f32);
            }
            WindowEvent::CursorLeft { .. } => self.session.pointer_left(),
            WindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    MouseButton::Left => Button::Left,
                    MouseButton::Middle => Button::Middle,
                    MouseButton::Right => Button::Right,
                    MouseButton::Back => Button::Back,
                    MouseButton::Forward => Button::Forward,
                    _ => Button::Other,
                };
                match state {
                    ElementState::Pressed => self.session.press(button, self.start.elapsed()),
                    ElementState::Released => self.session.release(button),
                }
            }
            _ => return,
        }
        self.update(&window);
    }
}

/// The OS's cursor for a page's `cursor` (M26.4); `none` hides it.
fn cursor_icon(cursor: engine::Cursor) -> CursorIcon {
    use engine::Cursor;
    match cursor {
        Cursor::Auto | Cursor::Default | Cursor::None => CursorIcon::Default,
        Cursor::ContextMenu => CursorIcon::ContextMenu,
        Cursor::Help => CursorIcon::Help,
        Cursor::Pointer => CursorIcon::Pointer,
        Cursor::Progress => CursorIcon::Progress,
        Cursor::Wait => CursorIcon::Wait,
        Cursor::Cell => CursorIcon::Cell,
        Cursor::Crosshair => CursorIcon::Crosshair,
        Cursor::Text => CursorIcon::Text,
        Cursor::VerticalText => CursorIcon::VerticalText,
        Cursor::Alias => CursorIcon::Alias,
        Cursor::Copy => CursorIcon::Copy,
        Cursor::Move => CursorIcon::Move,
        Cursor::NoDrop => CursorIcon::NoDrop,
        Cursor::NotAllowed => CursorIcon::NotAllowed,
        Cursor::Grab => CursorIcon::Grab,
        Cursor::Grabbing => CursorIcon::Grabbing,
        Cursor::EResize => CursorIcon::EResize,
        Cursor::NResize => CursorIcon::NResize,
        Cursor::NeResize => CursorIcon::NeResize,
        Cursor::NwResize => CursorIcon::NwResize,
        Cursor::SResize => CursorIcon::SResize,
        Cursor::SeResize => CursorIcon::SeResize,
        Cursor::SwResize => CursorIcon::SwResize,
        Cursor::WResize => CursorIcon::WResize,
        Cursor::EwResize => CursorIcon::EwResize,
        Cursor::NsResize => CursorIcon::NsResize,
        Cursor::NeswResize => CursorIcon::NeswResize,
        Cursor::NwseResize => CursorIcon::NwseResize,
        Cursor::ColResize => CursorIcon::ColResize,
        Cursor::RowResize => CursorIcon::RowResize,
        Cursor::AllScroll => CursorIcon::AllScroll,
        Cursor::ZoomIn => CursorIcon::ZoomIn,
        Cursor::ZoomOut => CursorIcon::ZoomOut,
    }
}

/// What the command line says of the user's preferences.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Asked {
    /// `--prefers-color-scheme light|dark`.
    color_scheme: Option<ColorScheme>,
    /// `--prefers-reduced-motion`: less motion, whatever the system says.
    reduced_motion: bool,
}

/// The user's preferences: what the command line `asked`, and else what
/// the system says, the window's `theme` and `reduced_motion`, and else
/// no preference.
fn preferences(asked: Asked, theme: Option<Theme>, reduced_motion: Option<bool>) -> Preferences {
    let system = theme.map(|theme| match theme {
        Theme::Light => ColorScheme::Light,
        Theme::Dark => ColorScheme::Dark,
    });
    Preferences {
        color_scheme: asked.color_scheme.or(system).unwrap_or_default(),
        reduced_motion: asked.reduced_motion || reduced_motion.unwrap_or(false),
    }
}

/// How long `--print-to-pdf` waits for the page to load before it
/// prints what there is.
const PRINT_LOAD_LIMIT: Duration = Duration::from_secs(60);

/// `--print-to-pdf`: loads `start`, or `about:silver`, with no window,
/// waits until it has loaded or failed, or for [`PRINT_LOAD_LIMIT`], and
/// prints it to `path` by the default print options; gives the exit
/// status, zero when the file is written. A page that failed to load
/// prints as the error page the window would show. It keeps nothing
/// but in a profile the command line names (M28), as the reference
/// browser's headless mode does.
fn print_to_pdf(
    start: Option<Url>,
    asked: Asked,
    scripts: bool,
    compat: bool,
    keep: &Keep,
    path: &Path,
) -> i32 {
    let mut engine = match Engine::new() {
        Ok(engine) => engine,
        Err(e) => {
            eprintln!("silver-browser: engine initialization failed: {e}");
            return 1;
        }
    };
    let keep = if *keep == Keep::Default {
        &Keep::Nothing
    } else {
        keep
    };
    if let Some(why) = open_profile(&mut engine, keep, None) {
        eprintln!("silver-browser: {why}");
    }
    engine.set_preferences(preferences(asked, None, platform::reduced_motion()));
    engine.set_scripting(scripts);
    if compat {
        engine.set_compat_user_agent(true);
    }
    let (wake, woken) = mpsc::channel();
    engine.set_waker(move || {
        // A receiver gone has nothing to wake.
        let _ = wake.send(());
    });
    let url = start.unwrap_or_else(|| Url::parse("about:silver").expect("about:silver parses"));
    engine.navigate(url);
    let deadline = Instant::now() + PRINT_LOAD_LIMIT;
    while *engine.load_state() == LoadState::Loading {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            eprintln!(
                "silver-browser: the page has not loaded in {} s; printing what there is",
                PRINT_LOAD_LIMIT.as_secs()
            );
            break;
        }
        let _ = woken.recv_timeout(left.min(Duration::from_millis(100)));
        engine.poll_network();
        engine.run_tasks();
        session::print_console(&mut engine);
    }
    if let LoadState::Failed(error) = engine.load_state() {
        eprintln!("silver-browser: {error}");
    }
    let pdf = match engine.print(&PrintOptions::default()) {
        Ok(pdf) => pdf,
        Err(e) => {
            eprintln!("silver-browser: {e}");
            return 1;
        }
    };
    if let Err(e) = engine.flush_profile() {
        eprintln!("silver-browser: the profile was not written: {e}");
    }
    match std::fs::write(path, pdf) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("silver-browser: {}: {e}", path.display());
            1
        }
    }
}

/// Gives `engine` the profile `keep` names, `default` for the
/// platform's place (M28, docs/design/storage.md). What went wrong, to
/// say: the engine then keeps nothing this run, and a profile it could
/// not open is left as it is.
fn open_profile(engine: &mut Engine, keep: &Keep, default: Option<PathBuf>) -> Option<String> {
    let dir = match keep {
        Keep::Nothing => return None,
        Keep::In(dir) => dir.clone(),
        Keep::Default => match default {
            Some(dir) => dir,
            None => return Some("no place for the profile; keeping nothing this time".to_owned()),
        },
    };
    let failed = |why: String| {
        Some(format!(
            "the profile in {}: {why}; keeping nothing this time",
            dir.display()
        ))
    };
    // Its values sealed under a key the system keeps for the user (D27).
    match Profile::open_with(&dir, platform::key_wrap()) {
        Ok(profile) => engine
            .set_profile(profile)
            .err()
            .and_then(|e| failed(e.to_string())),
        Err(e) => failed(e.to_string()),
    }
}

/// The session's key for winit's, or nothing for a modifier key, whose
/// state comes on its own.
fn key_of(key: &winit::keyboard::Key) -> Option<Key> {
    use winit::keyboard::Key as Winit;
    Some(match key {
        Winit::Named(named) => match named {
            NamedKey::Enter => Key::Enter,
            NamedKey::Escape => Key::Escape,
            NamedKey::Backspace => Key::Backspace,
            NamedKey::Delete => Key::Delete,
            NamedKey::Insert => Key::Insert,
            NamedKey::ArrowLeft => Key::Left,
            NamedKey::ArrowRight => Key::Right,
            NamedKey::ArrowUp => Key::Up,
            NamedKey::ArrowDown => Key::Down,
            NamedKey::Home => Key::Home,
            NamedKey::End => Key::End,
            NamedKey::PageUp => Key::PageUp,
            NamedKey::PageDown => Key::PageDown,
            NamedKey::Space => Key::Space,
            NamedKey::Tab => Key::Tab,
            NamedKey::F5 => Key::F5,
            NamedKey::F6 => Key::F6,
            NamedKey::Shift
            | NamedKey::Control
            | NamedKey::Alt
            | NamedKey::AltGraph
            | NamedKey::Super
            | NamedKey::Meta
            | NamedKey::Hyper
            | NamedKey::Fn
            | NamedKey::FnLock
            | NamedKey::CapsLock
            | NamedKey::NumLock
            | NamedKey::ScrollLock
            | NamedKey::Symbol
            | NamedKey::SymbolLock => return None,
            _ => Key::Other,
        },
        Winit::Character(text) => Key::Char(text.to_string()),
        Winit::Unidentified(_) | Winit::Dead(_) => Key::Other,
    })
}

/// The page's key for winit's (M26.6): UI Events' `key` value, a named
/// key's name or a character's text, and its `code`, the physical key's
/// name, which winit gives as UI Events does but the logo key's.
fn page_key(event: &KeyEvent) -> engine::Key {
    use winit::keyboard::{Key as Winit, KeyLocation, PhysicalKey};
    let key = match &event.logical_key {
        Winit::Character(text) => text.to_string(),
        Winit::Named(NamedKey::Space) => " ".to_owned(),
        Winit::Named(NamedKey::Super) => "Meta".to_owned(),
        Winit::Named(named) => format!("{named:?}"),
        Winit::Dead(_) => "Dead".to_owned(),
        Winit::Unidentified(_) => "Unidentified".to_owned(),
    };
    let code = match event.physical_key {
        PhysicalKey::Code(code) => match format!("{code:?}").as_str() {
            "SuperLeft" => "MetaLeft".to_owned(),
            "SuperRight" => "MetaRight".to_owned(),
            name => name.to_owned(),
        },
        PhysicalKey::Unidentified(_) => String::new(),
    };
    let location = match event.location {
        KeyLocation::Standard => 0,
        KeyLocation::Left => 1,
        KeyLocation::Right => 2,
        KeyLocation::Numpad => 3,
    };
    engine::Key {
        key,
        code,
        location,
        repeat: event.repeat,
        text: event.text.as_ref().map(|text| text.to_string()),
    }
}

/// The OS's clipboard, through `arboard`, opened when first used and
/// kept open: on X11 what was copied lasts as long as the clipboard
/// that copied it, unless a clipboard manager takes it.
#[derive(Default)]
struct SystemClipboard(Option<arboard::Clipboard>);

impl SystemClipboard {
    fn open(&mut self) -> Result<&mut arboard::Clipboard, String> {
        if self.0.is_none() {
            self.0 = Some(arboard::Clipboard::new().map_err(|error| error.to_string())?);
        }
        self.0.as_mut().ok_or_else(|| "it did not open".to_owned())
    }
}

impl Clipboard for SystemClipboard {
    fn get(&mut self) -> Result<String, String> {
        self.open()?.get_text().map_err(|error| error.to_string())
    }

    fn set(&mut self, text: &str) -> Result<(), String> {
        self.open()?
            .set_text(text)
            .map_err(|error| error.to_string())
    }
}

/// What the command line asks for: the page to show, and the options.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandLine {
    /// `None` for `about:silver`, or the URL to load.
    start: Option<Url>,
    /// `--layer-budget <MiB>`: how much the engine may hold in layers
    /// (M19.6), in bytes, when given.
    layer_budget: Option<usize>,
    /// `--software`: draw in software even where there is a GPU
    /// (M19.3, roadmap D3).
    software: bool,
    /// The user's preferences given (M20.3).
    asked: Asked,
    /// `--print-to-pdf <path>`: the file to print the page to, with no
    /// window (M20.4).
    print_to: Option<PathBuf>,
    /// `--enable-scripts`: run pages' scripts (M24), which are off
    /// otherwise until the isolation of M33 and M34 (roadmap risk R2).
    scripts: bool,
    /// `--compat-user-agent`: send the reference browser's user agent
    /// string, this browser named after it (D12's escape hatch, M25).
    compat: bool,
    /// Where the browsing keeps what it keeps (M28).
    profile: Keep,
}

/// The profile the command line asks for (M28).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Keep {
    /// The platform's place for it, or none for `--print-to-pdf`.
    Default,
    /// `--profile <dir>`.
    In(PathBuf),
    /// `--temporary-profile`: nothing kept.
    Nothing,
}

/// The command line parsed: at most one URL or path, and the options
/// in any order around it. An argument naming an existing file is that
/// file's `file:` URL; any other goes through the address field's
/// rules (`engine::address`, M15.4), so `example.com` is
/// `https://example.com/`. The file comes first: on Windows,
/// `C:\page.html` parses as a URL too, of the scheme `c`.
fn command_line(args: impl Iterator<Item = OsString>) -> Result<CommandLine, String> {
    let mut line = CommandLine {
        start: None,
        layer_budget: None,
        software: false,
        asked: Asked::default(),
        print_to: None,
        scripts: false,
        compat: false,
        profile: Keep::Default,
    };
    let mut page: Option<OsString> = None;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        if arg == "--software" {
            line.software = true;
            continue;
        }
        if arg == "--enable-scripts" {
            line.scripts = true;
            continue;
        }
        if arg == "--compat-user-agent" {
            line.compat = true;
            continue;
        }
        if arg == "--profile" || arg == "--temporary-profile" {
            if line.profile != Keep::Default {
                return Err("--profile and --temporary-profile: one at most".to_owned());
            }
            line.profile = if arg == "--temporary-profile" {
                Keep::Nothing
            } else {
                let dir = args
                    .next()
                    .ok_or_else(|| "--profile takes the directory of the profile".to_owned())?;
                Keep::In(PathBuf::from(dir))
            };
            continue;
        }
        if arg == "--print-to-pdf" {
            let path = args
                .next()
                .ok_or_else(|| "--print-to-pdf takes the path of the file to write".to_owned())?;
            line.print_to = Some(PathBuf::from(path));
            continue;
        }
        if arg == "--prefers-reduced-motion" {
            line.asked.reduced_motion = true;
            continue;
        }
        if arg == "--prefers-color-scheme" {
            let value = args
                .next()
                .ok_or_else(|| "--prefers-color-scheme takes light or dark".to_owned())?;
            line.asked.color_scheme = Some(match value.to_str() {
                Some("light") => ColorScheme::Light,
                Some("dark") => ColorScheme::Dark,
                _ => {
                    return Err(format!("{}: not light or dark", value.to_string_lossy()));
                }
            });
            continue;
        }
        if arg == "--layer-budget" {
            let value = args
                .next()
                .ok_or_else(|| "--layer-budget takes a size in MiB".to_owned())?;
            let mib: usize = value
                .to_str()
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| format!("{}: not a size in MiB", value.to_string_lossy()))?;
            line.layer_budget = Some(
                mib.checked_mul(1 << 20)
                    .ok_or_else(|| format!("{mib} MiB: too large"))?,
            );
            continue;
        }
        if page.replace(arg).is_some() {
            return Err("one URL or path at most".to_owned());
        }
    }
    line.start = match page {
        Some(arg) => start_page(arg)?,
        None => None,
    };
    Ok(line)
}

/// The page an argument names (see [`command_line`]).
fn start_page(arg: OsString) -> Result<Option<Url>, String> {
    let path = Path::new(&arg);
    if path.exists() {
        let absolute =
            std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
        return Url::from_file_path(&absolute)
            .map(Some)
            .ok_or_else(|| format!("{}: no file: URL names this path", path.display()));
    }
    let not_found = || format!("{}: not an address, and no such file", path.display());
    let text = arg.to_str().ok_or_else(not_found)?;
    match engine::address::parse(text) {
        Address::Url(url) => Ok(Some(url)),
        Address::Empty => Ok(None),
        Address::Unsupported(scheme) => Err(format!("{scheme}: addresses cannot be opened")),
        Address::NotAnAddress => Err(not_found()),
    }
}

fn main() {
    let line = match command_line(std::env::args_os().skip(1)) {
        Ok(line) => line,
        Err(message) => {
            eprintln!("silver-browser: {message}");
            eprintln!(
                "usage: silver-browser [--software] [--layer-budget <MiB>] \
                 [--prefers-color-scheme light|dark] [--prefers-reduced-motion] \
                 [--enable-scripts] [--compat-user-agent] [--print-to-pdf <path>] \
                 [--profile <dir> | --temporary-profile] [URL or path]"
            );
            std::process::exit(2);
        }
    };
    let start = line.start;
    let asked = line.asked;
    if let Some(path) = &line.print_to {
        std::process::exit(print_to_pdf(
            start,
            asked,
            line.scripts,
            line.compat,
            &line.profile,
            path,
        ));
    }

    let wanted = if line.software {
        Backend::Software
    } else {
        Backend::Gpu
    };
    let (mut engine, fallback) = match Engine::new_on(wanted) {
        Ok(made) => made,
        Err(e) => {
            eprintln!("engine initialization failed: {e}");
            return;
        }
    };
    if let Some(why) = fallback {
        eprintln!("silver-browser: {why}; drawing in software");
    }
    if let Some(bytes) = line.layer_budget {
        engine.set_layer_budget(bytes);
    }
    // The window's theme is known once there is a window.
    engine.set_preferences(preferences(asked, None, platform::reduced_motion()));
    engine.set_scripting(line.scripts);
    if line.compat {
        engine.set_compat_user_agent(true);
    }
    if let Some(why) = open_profile(&mut engine, &line.profile, platform::profile_directory()) {
        eprintln!("silver-browser: {why}");
    }

    let event_loop = match EventLoop::<NetworkWake>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(e) => {
            eprintln!("event loop creation failed: {e}");
            std::process::exit(1);
        }
    };

    // The proxy is sent across threads but need not be shared between
    // them on every platform, so a lock makes it so.
    let proxy = Mutex::new(event_loop.create_proxy());
    engine.set_waker(move || {
        let proxy = proxy.lock().unwrap_or_else(PoisonError::into_inner);
        // An event loop gone has no one to wake.
        let _ = proxy.send_event(NetworkWake);
    });
    // The engine's tasks and the frames read one clock (M26).
    let began = Instant::now();
    engine.set_clock(Arc::new(move || began.elapsed()));
    let clipboard = Box::new(SystemClipboard::default());
    let session = Session::new(engine, clipboard, Platform::current(), start);

    let mut browser = Browser::new(session, asked, began);
    let ran = event_loop.run_app(&mut browser);
    // The window has closed: what changed since the last write is
    // written now.
    if let Err(e) = browser.session.flush_profile() {
        eprintln!("silver-browser: the profile was not written: {e}");
    }
    if ran.is_err() {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    /// `line` without its string literals and its line comment.
    fn code_of(line: &str) -> String {
        let (mut code, mut quoted, mut escaped) = (String::new(), false, false);
        for c in line.chars() {
            match (quoted, c) {
                (false, '"') => quoted = true,
                (false, _) => code.push(c),
                (true, _) if escaped => escaped = false,
                (true, '\\') => escaped = true,
                (true, '"') => quoted = false,
                (true, _) => {}
            }
        }
        code.split("//").next().unwrap_or_default().to_owned()
    }

    /// Whether `code`, a line with its strings and comment cut off,
    /// writes `unsafe`: a block, a function, a trait or an impl, an
    /// extern block, or an unsafe attribute; not a word that only begins
    /// with it, as CSP's `'unsafe-inline'`.
    fn writes_unsafe(code: &str) -> bool {
        if code.contains(concat!("#[", "unsafe(")) {
            return true;
        }
        let mut rest = code;
        while let Some(at) = rest.find("unsafe") {
            let before = rest[..at].chars().next_back();
            let after = &rest[at + "unsafe".len()..];
            rest = after;
            let word = |c: char| c.is_alphanumeric() || c == '_' || c == '-' || c == '\'';
            if before.is_some_and(word) || after.starts_with(word) {
                continue;
            }
            let next = after.trim_start();
            if ["{", "fn", "impl", "trait", "extern"]
                .iter()
                .any(|kind| next.starts_with(kind))
            {
                return true;
            }
        }
        false
    }

    /// The `.rs` files under `dir`.
    fn sources(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                sources(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }

    /// T3.2 (M33.5, docs/architecture.md): the workspace writes `unsafe`
    /// in the shell's platform module alone, each block with its
    /// `// SAFETY:` argument just above it; every other crate root, test,
    /// bench, example and fuzz target forbids it.
    #[test]
    fn unsafe_is_written_in_the_platform_module_alone_each_block_argued() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        sources(&root.join("crates"), &mut files);
        sources(&root.join("fuzz").join("fuzz_targets"), &mut files);
        assert!(files.len() > 100, "the workspace's sources found");
        let mut blocks = 0;
        for file in &files {
            let name = file
                .strip_prefix(&root)
                .unwrap_or(file)
                .to_string_lossy()
                .replace('\\', "/");
            // M29's module is a crate of its own, built to WebAssembly
            // and run in the interpreter, whose exports `no_mangle` marks.
            if name.starts_with("crates/engine/tests/wasm/") {
                continue;
            }
            let text = std::fs::read_to_string(file).unwrap_or_default();
            let lines: Vec<&str> = text.lines().collect();
            for (n, line) in lines.iter().enumerate() {
                let code = code_of(line);
                if !writes_unsafe(&code) {
                    continue;
                }
                assert_eq!(
                    name,
                    "crates/shell/src/platform.rs",
                    "unsafe at line {}",
                    n + 1
                );
                // Its argument: the nearest comment above it, a statement
                // of a few lines at most between, begins `// SAFETY:`.
                let comment = |line: &str| line.trim_start().starts_with("//");
                let above = lines[..n]
                    .iter()
                    .rev()
                    .take(4)
                    .position(|line| comment(line));
                let argued = above.is_some_and(|skip| {
                    lines[..n - skip]
                        .iter()
                        .rev()
                        .take_while(|line| comment(line))
                        .last()
                        .is_some_and(|first| first.trim_start().starts_with("// SAFETY:"))
                });
                assert!(
                    argued,
                    "{name}:{}: an unsafe block without its argument",
                    n + 1
                );
                blocks += 1;
            }
            let root_file = name.ends_with("/src/lib.rs")
                || name.starts_with("fuzz/")
                || ["tests", "benches", "examples"].iter().any(|kind| {
                    let parts: Vec<&str> = name.split('/').collect();
                    parts.len() == 4 && parts[2] == *kind
                });
            if root_file {
                assert!(
                    text.contains("#![forbid(unsafe_code)]"),
                    "{name} does not forbid unsafe code"
                );
            }
        }
        assert!(blocks >= 4, "the platform module's blocks found: {blocks}");
        // The shell denies it but where the platform module allows it.
        let shell = std::fs::read_to_string(root.join("crates/shell/src/main.rs")).unwrap();
        assert!(shell.contains("\n#![deny(unsafe_code)]\n"));
    }

    use super::*;

    #[test]
    fn a_wanted_frame_waits_for_a_frames_budget_after_the_last() {
        let drawn = Instant::now();
        assert_eq!(frame_due(None, drawn), None);
        let soon = drawn + Duration::from_millis(5);
        assert_eq!(frame_due(Some(drawn), soon), Some(drawn + FRAME_BUDGET));
        assert_eq!(frame_due(Some(drawn), drawn + FRAME_BUDGET), None);
    }

    fn start(args: &[&str]) -> Result<Option<Url>, String> {
        command_line(args.iter().map(OsString::from)).map(|line| line.start)
    }

    #[test]
    fn the_surface_is_resized_only_when_the_window_is() {
        let size = |w, h| (NonZeroU32::new(w).unwrap(), NonZeroU32::new(h).unwrap());
        let mut surface = SurfaceSize::default();
        assert!(surface.wants(size(800, 600)), "a new surface has no size");
        assert!(!surface.wants(size(800, 600)));
        assert!(!surface.wants(size(800, 600)));
        assert!(surface.wants(size(1024, 600)));
        assert!(!surface.wants(size(1024, 600)));
        // After a failed resize the size is forgotten, and tried again.
        surface = SurfaceSize::default();
        assert!(surface.wants(size(1024, 600)));
    }

    #[test]
    fn the_compatibility_user_agent_is_sent_only_when_asked() {
        let compat =
            |args: &[&str]| command_line(args.iter().map(OsString::from)).map(|l| l.compat);
        assert_eq!(compat(&[]), Ok(false));
        assert_eq!(compat(&["--compat-user-agent", "example.com"]), Ok(true));
    }

    #[test]
    fn scripts_run_only_when_the_command_line_asks() {
        let scripts =
            |args: &[&str]| command_line(args.iter().map(OsString::from)).map(|l| l.scripts);
        assert_eq!(scripts(&[]), Ok(false));
        assert_eq!(scripts(&["--enable-scripts"]), Ok(true));
        assert_eq!(scripts(&["example.com", "--enable-scripts"]), Ok(true));
    }

    #[test]
    fn the_layer_budget_option_is_in_mebibytes_anywhere_on_the_line() {
        let line = |args: &[&str]| command_line(args.iter().map(OsString::from));
        assert_eq!(
            line(&["--layer-budget", "16", "https://example.com/"]),
            Ok(CommandLine {
                start: Some(Url::parse("https://example.com/").unwrap()),
                layer_budget: Some(16 << 20),
                software: false,
                asked: Asked::default(),
                print_to: None,
                scripts: false,
                compat: false,
                profile: Keep::Default,
            })
        );
        assert_eq!(
            line(&["https://example.com/", "--layer-budget", "0", "--software"]),
            Ok(CommandLine {
                start: Some(Url::parse("https://example.com/").unwrap()),
                layer_budget: Some(0),
                software: true,
                asked: Asked::default(),
                print_to: None,
                scripts: false,
                compat: false,
                profile: Keep::Default,
            })
        );
        assert_eq!(
            line(&["--layer-budget"]),
            Err("--layer-budget takes a size in MiB".to_owned())
        );
        assert_eq!(
            line(&["--layer-budget", "lots"]),
            Err("lots: not a size in MiB".to_owned())
        );
        assert_eq!(
            line(&["a", "--layer-budget", "1", "b"]),
            Err("one URL or path at most".to_owned())
        );
    }

    #[test]
    fn the_preferences_given_override_the_systems() {
        let asked = |args: &[&str]| command_line(args.iter().map(OsString::from)).map(|l| l.asked);
        assert_eq!(asked(&[]), Ok(Asked::default()));
        assert_eq!(
            asked(&[
                "--prefers-color-scheme",
                "dark",
                "example.com",
                "--prefers-reduced-motion"
            ]),
            Ok(Asked {
                color_scheme: Some(ColorScheme::Dark),
                reduced_motion: true,
            })
        );
        assert_eq!(
            asked(&["--prefers-color-scheme", "light"]).map(|a| a.color_scheme),
            Ok(Some(ColorScheme::Light))
        );
        assert_eq!(
            asked(&["--prefers-color-scheme"]),
            Err("--prefers-color-scheme takes light or dark".to_owned())
        );
        assert_eq!(
            asked(&["--prefers-color-scheme", "Dark"]),
            Err("Dark: not light or dark".to_owned())
        );

        // Given, they stand; not given, the system's; without either,
        // no preference.
        let none = Asked::default();
        let dark = Asked {
            color_scheme: Some(ColorScheme::Dark),
            reduced_motion: true,
        };
        assert_eq!(preferences(none, None, None), Preferences::default());
        assert_eq!(
            preferences(none, Some(Theme::Dark), Some(true)),
            Preferences {
                color_scheme: ColorScheme::Dark,
                reduced_motion: true,
            }
        );
        assert_eq!(
            preferences(none, Some(Theme::Light), Some(false)),
            Preferences::default()
        );
        assert_eq!(
            preferences(dark, Some(Theme::Light), Some(false)),
            Preferences {
                color_scheme: ColorScheme::Dark,
                reduced_motion: true,
            }
        );
    }

    #[test]
    fn print_to_pdf_writes_the_page_it_loads_and_says_what_failed() {
        let line = |args: &[&str]| command_line(args.iter().map(OsString::from));
        assert_eq!(
            line(&["--print-to-pdf", "out.pdf", "example.com"]).map(|l| l.print_to),
            Ok(Some(PathBuf::from("out.pdf")))
        );
        assert_eq!(
            line(&["--print-to-pdf"]).map(|l| l.print_to),
            Err("--print-to-pdf takes the path of the file to write".to_owned())
        );
        let dir = std::env::temp_dir().join(format!("silver-print-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let page = dir.join("page.html");
        std::fs::write(&page, "<style>@page { margin: 0 }</style><p>Printed</p>").unwrap();
        let url = Url::from_file_path(&std::fs::canonicalize(&page).unwrap()).unwrap();
        let out = dir.join("page.pdf");
        assert_eq!(
            print_to_pdf(
                Some(url),
                Asked::default(),
                false,
                false,
                &Keep::Default,
                &out
            ),
            0
        );
        let pdf = std::fs::read(&out).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        let letter = b"/MediaBox [0 0 612 792]";
        assert!(pdf.windows(letter.len()).any(|w| w == letter));
        // A file that cannot be written.
        let url = Url::from_file_path(&std::fs::canonicalize(&page).unwrap()).unwrap();
        assert_eq!(
            print_to_pdf(
                Some(url),
                Asked::default(),
                false,
                false,
                &Keep::Default,
                &dir.join("no/such/dir/x.pdf")
            ),
            1
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn print_to_pdf_keeps_what_it_keeps_in_the_profile_named_alone() {
        use net::test_server::{Reply, TestServer};
        let server = TestServer::plain(|_| {
            Reply::with_fields(
                200,
                &[
                    ("Content-Type", "text/html"),
                    ("Set-Cookie", "kept=1; Max-Age=3600"),
                ],
                b"<p>kept</p>",
            )
        });
        let dir = std::env::temp_dir().join(format!("silver-print-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let profile = dir.join("profile");
        let out = dir.join("page.pdf");
        let print = |keep: &Keep| {
            print_to_pdf(
                Some(server.url("/")),
                Asked::default(),
                false,
                false,
                keep,
                &out,
            )
        };
        assert_eq!(print(&Keep::Default), 0);
        assert!(!profile.exists());
        assert_eq!(print(&Keep::In(profile.clone())), 0);
        // Sealed under the key the platform keeps (D27): a copy opened
        // without it reads nothing.
        #[cfg(windows)]
        {
            let copy = dir.join("copy");
            std::fs::create_dir_all(&copy).unwrap();
            std::fs::copy(profile.join(profile::FILE), copy.join(profile::FILE)).unwrap();
            let unwrapped = Profile::open(&copy).unwrap();
            assert!(unwrapped.cookies().unwrap().is_empty());
        }
        let kept = Profile::open_with(&profile, platform::key_wrap()).unwrap();
        assert_eq!(kept.cookies().unwrap().len(), 1);
        drop(kept);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_profile_is_the_platforms_one_given_or_none() {
        let keep = |args: &[&str]| command_line(args.iter().map(OsString::from)).map(|l| l.profile);
        assert_eq!(keep(&[]), Ok(Keep::Default));
        assert_eq!(
            keep(&["--profile", "p", "example.com"]),
            Ok(Keep::In(PathBuf::from("p")))
        );
        assert_eq!(keep(&["--temporary-profile"]), Ok(Keep::Nothing));
        assert_eq!(
            keep(&["--profile"]),
            Err("--profile takes the directory of the profile".to_owned())
        );
        for both in [
            ["--profile", "p", "--temporary-profile"],
            ["--temporary-profile", "--profile", "p"],
        ] {
            assert_eq!(
                keep(&both),
                Err("--profile and --temporary-profile: one at most".to_owned())
            );
        }
    }

    #[test]
    fn a_profile_is_opened_or_why_not_said() {
        let dir = std::env::temp_dir().join(format!("silver-shell-profile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut engine = Engine::new().unwrap();
        assert_eq!(
            open_profile(&mut engine, &Keep::Nothing, Some(dir.clone())),
            None
        );
        assert!(!dir.exists());
        assert_eq!(
            open_profile(&mut engine, &Keep::Default, Some(dir.clone())),
            None
        );
        assert!(dir.join("profile.redb").is_file());
        // Held by the first engine, it is refused to a second.
        let mut second = Engine::new().unwrap();
        assert_eq!(
            open_profile(&mut second, &Keep::In(dir.clone()), None),
            Some(format!(
                "the profile in {}: another browser has it open; keeping nothing this time",
                dir.display()
            ))
        );
        assert_eq!(
            open_profile(&mut second, &Keep::Default, None),
            Some("no place for the profile; keeping nothing this time".to_owned())
        );
        drop(engine);
        assert_eq!(
            open_profile(&mut second, &Keep::In(dir.clone()), None),
            None
        );
        drop(second);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_argument_shows_the_built_in_page() {
        assert_eq!(start(&[]), Ok(None));
    }

    #[test]
    fn an_absolute_url_is_loaded() {
        assert_eq!(
            start(&["https://example.com/a?b#c"]),
            Ok(Some(Url::parse("https://example.com/a?b#c").unwrap()))
        );
    }

    #[test]
    fn an_existing_file_is_loaded_by_its_file_url() {
        let dir = std::env::temp_dir().join(format!("silver-shell-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a page#1.html");
        std::fs::write(&file, "<p>x</p>").unwrap();
        let url = start(&[file.to_str().unwrap()]).unwrap().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(url.scheme(), "file");
        assert!(
            url.as_str().ends_with("/a%20page%231.html"),
            "{}",
            url.as_str()
        );
        assert_eq!(url.fragment(), None);
    }

    #[test]
    fn an_address_without_a_scheme_is_loaded_as_the_address_field_would() {
        assert_eq!(
            start(&["example.com"]),
            Ok(Some(Url::parse("https://example.com/").unwrap()))
        );
        assert_eq!(
            start(&["localhost:8080"]),
            Ok(Some(Url::parse("http://localhost:8080/").unwrap()))
        );
        assert!(start(&["mailto:someone@example.com"]).is_err());
    }

    #[test]
    fn anything_else_is_refused() {
        assert!(start(&["no such page.html"]).is_err());
        assert!(start(&["https://example.com/", "https://example.org/"]).is_err());
    }
}
