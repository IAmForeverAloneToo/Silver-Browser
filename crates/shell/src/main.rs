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
//! the focus, since no event says it changed. Each
//! finished fetch wakes the event loop with a user event, on which the
//! engine takes what arrived; a load the command line asked for that
//! fails shows the engine's error page, and is reported on standard
//! error.

mod keys;
mod platform;
mod session;

use std::ffi::OsString;
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use engine::address::Address;
use engine::{Backend, ColorScheme, Engine, FRAME_BUDGET, FrameTime, Preferences, Url};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
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
    fn new(session: Session, asked: Asked) -> Self {
        Browser {
            window: None,
            context: None,
            surface: None,
            session,
            start: Instant::now(),
            shown: Shown::default(),
            asked,
            theme: None,
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
        // TODO(M2): only resize on size changes, not every frame.
        if let Err(e) = surface.resize(w, h) {
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
            window.set_cursor(match shape {
                Shape::Arrow => CursorIcon::Default,
                Shape::Hand => CursorIcon::Pointer,
                Shape::Text => CursorIcon::Text,
            });
        }
        let ime = self.session.ime_area().map(|(x, y, w, h)| {
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
        if self.session.needs_frame() {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler<NetworkWake> for Browser {
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
                            self.session.resized(size.width, size.height);
                            self.theme = window.theme();
                            self.read_preferences();
                            self.context = Some(context);
                            self.surface = Some(surface);
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
                    && let Err(e) = surface.resize(w, h)
                {
                    eprintln!("surface resize failed: {e}");
                }
                self.session.resized(size.width, size.height);
                window.request_redraw();
            }
            WindowEvent::ThemeChanged(theme) => {
                self.theme = Some(theme);
                self.read_preferences();
            }
            WindowEvent::Focused(true) => self.read_preferences(),
            WindowEvent::ModifiersChanged(modifiers) => {
                let state = modifiers.state();
                self.session.modifiers(Modifiers {
                    shift: state.shift_key(),
                    ctrl: state.control_key(),
                    alt: state.alt_key(),
                    logo: state.super_key(),
                });
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        text,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                if let Some(key) = key_of(&logical_key) {
                    self.session.key(key, text.as_deref());
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
                    MouseScrollDelta::PixelDelta(p) => -p.y as f32,
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
    };
    let mut page: Option<OsString> = None;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        if arg == "--software" {
            line.software = true;
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
                 [--prefers-color-scheme light|dark] [--prefers-reduced-motion] [URL or path]"
            );
            std::process::exit(2);
        }
    };
    let start = line.start;
    let asked = line.asked;

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
    let clipboard = Box::new(SystemClipboard::default());
    let session = Session::new(engine, clipboard, Platform::current(), start);

    let mut browser = Browser::new(session, asked);
    if event_loop.run_app(&mut browser).is_err() {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(args: &[&str]) -> Result<Option<Url>, String> {
        command_line(args.iter().map(OsString::from)).map(|line| line.start)
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
            })
        );
        assert_eq!(
            line(&["https://example.com/", "--layer-budget", "0", "--software"]),
            Ok(CommandLine {
                start: Some(Url::parse("https://example.com/").unwrap()),
                layer_budget: Some(0),
                software: true,
                asked: Asked::default(),
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
