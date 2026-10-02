//! The shell: the crate that talks to the OS for windows and input;
//! `net` alone opens connections and reads resources by URL.
//!
//! Owns the window, the event loop, and the present pipeline (winit +
//! softbuffer). All rendering decisions live in `engine`; this file
//! pumps frames to the screen and turns input into engine calls (M9):
//! the wheel (at the pointer, so a scroll container under it takes it
//! first, M10.5), the keys and the scrollbar into scrolling, a click
//! into a hit test. Each frame is drawn for the time on the shell's
//! clock (M13.4), and a frame is asked for only when the engine wants
//! one: after input that changed something, or while a scroll animates.
//!
//! It runs as `silver-browser [URL or path]` (M14, docs/design/net.md,
//! "The shell"): an argument naming an existing file loads that file,
//! and any other must be an absolute URL; with none, `about:silver`
//! shows (M15). Each finished fetch wakes the event loop with a user
//! event, on which the engine takes what arrived; a load that fails
//! shows the engine's error page, and is reported on standard error.

use std::ffi::OsString;
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use engine::{Engine, FrameTime, Link, LoadState, Url};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

/// Window configuration.
const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 800;
const WINDOW_TITLE: &str = "Silver Browser";
/// One wheel notch, or one arrow key, scrolls this far.
const LINE_PX: f32 = 40.0;

/// The user event a finished fetch wakes the event loop with.
#[derive(Debug, Clone, Copy)]
struct NetworkWake;

/// The browser application: window + surface + engine session.
///
/// The window lives behind an `Arc` because softbuffer's `Context` and
/// `Surface` each hold their own handle to it (raw-window-handle 0.6
/// provides the impls for `Arc<T>`).
struct Browser {
    window: Option<Arc<Window>>,
    context: Option<softbuffer::Context<Arc<Window>>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    engine: Option<Engine>,
    /// The pointer's last position, in window pixels.
    cursor: (f32, f32),
    /// While the scrollbar's thumb is held: how far below the thumb's
    /// top the pointer grabbed it, so the thumb follows without jumping.
    thumb_grab: Option<f32>,
    /// The clock frames are drawn by: the time since the browser began.
    start: Instant,
    /// The URL the command line asked for, until its load fails or its
    /// page is shown.
    loading: Option<Url>,
    /// The link the left button was pressed on: released on the same
    /// one, the press is a click, which follows it (M15).
    pressed_link: Option<Link>,
}

impl Browser {
    fn new(engine: Option<Engine>, loading: Option<Url>) -> Self {
        Browser {
            window: None,
            context: None,
            surface: None,
            engine,
            cursor: (0.0, 0.0),
            thumb_grab: None,
            start: Instant::now(),
            loading,
            pressed_link: None,
        }
    }

    /// Renders one frame and presents it. Failures skip the frame (logged)
    /// rather than taking the process down.
    fn draw(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let Some(engine) = self.engine.as_mut() else {
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

        let time = FrameTime::at(self.start.elapsed());
        let rgba = engine.frame_at(size.width, size.height, time);
        if rgba.is_empty() {
            // Past the engine's frame cap: the last frame stays shown.
            return;
        }
        match surface.buffer_mut() {
            Ok(mut buffer) => {
                // softbuffer buffers are 0RGB u32 (native endian);
                // convert from our RGBA8 bytes, assuming opaque pixels.
                let (pixels, _remainder) = rgba.as_chunks::<4>();
                for (dst, [r, g, b, _a]) in buffer.iter_mut().zip(pixels) {
                    *dst = (*r as u32) << 16 | (*g as u32) << 8 | *b as u32;
                }
                if let Err(e) = buffer.present() {
                    eprintln!("present failed: {e}");
                }
            }
            Err(e) => eprintln!("buffer acquisition failed: {e}"),
        }
    }

    /// Asks for a frame when the engine wants one.
    fn redraw_if_wanted(&self, window: &Window) {
        if self.engine.as_ref().is_some_and(Engine::needs_frame) {
            window.request_redraw();
        }
    }

    /// The window's height in pixels, for page-sized scrolls.
    fn viewport_height(&self) -> f32 {
        self.window
            .as_ref()
            .map_or(WINDOW_HEIGHT as f32, |w| w.inner_size().height as f32)
    }

    /// A page is the viewport less one line, so a line of context stays.
    fn page_px(&self) -> f32 {
        (self.viewport_height() - LINE_PX).max(LINE_PX)
    }

    /// A key press that scrolls (M9.2). Returns whether it did.
    fn key_scroll(&mut self, key: &Key) -> bool {
        let page = self.page_px();
        let Some(engine) = self.engine.as_mut() else {
            return false;
        };
        match key {
            Key::Named(NamedKey::ArrowDown) => engine.scroll_by(LINE_PX),
            Key::Named(NamedKey::ArrowUp) => engine.scroll_by(-LINE_PX),
            Key::Named(NamedKey::PageDown | NamedKey::Space) => engine.scroll_by(page),
            Key::Named(NamedKey::PageUp) => engine.scroll_by(-page),
            Key::Named(NamedKey::Home) => engine.scroll_to(0.0),
            Key::Named(NamedKey::End) => engine.scroll_to(f32::INFINITY),
            _ => return false,
        }
        true
    }

    /// The left button pressed at the pointer: grabs the scrollbar's
    /// thumb, jumps a page from its track, or notes the link under it,
    /// which the release follows if it is over the same link.
    fn press(&mut self) {
        let (x, y) = self.cursor;
        let page = self.page_px();
        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        if let Some(bar) = engine.scrollbar()
            && bar.contains(x, y)
        {
            if bar.thumb_contains(x, y) {
                self.thumb_grab = Some(y - bar.thumb_y);
            } else if y < bar.thumb_y {
                engine.scroll_by(-page);
            } else {
                engine.scroll_by(page);
            }
            return;
        }
        self.pressed_link = engine.link_at(x, y);
    }

    /// The left button released at the pointer: a press and a release on
    /// the same link are a click, which follows it (HTML's activation
    /// behaviour).
    fn release(&mut self) {
        self.thumb_grab = None;
        let (x, y) = self.cursor;
        let Some(pressed) = self.pressed_link.take() else {
            return;
        };
        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        if engine
            .link_at(x, y)
            .is_some_and(|link| link.node() == pressed.node())
        {
            engine.follow_link(&pressed);
            if let Some(notice) = engine.take_notice() {
                eprintln!("silver-browser: {notice}");
            }
        }
    }

    /// The pointer moved while the thumb is held: the thumb follows it.
    fn drag(&mut self) {
        let Some(grab) = self.thumb_grab else {
            return;
        };
        let (_, y) = self.cursor;
        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        let Some(bar) = engine.scrollbar() else {
            return;
        };
        let travel = bar.h - bar.thumb_h;
        if travel > 0.0 {
            engine.scroll_to_fraction((y - grab) / travel);
        }
    }
}

impl ApplicationHandler<NetworkWake> for Browser {
    /// A fetch finished: the engine takes what arrived. A failed load is
    /// reported, as the error page the engine shows says; a page shown in
    /// an encoding the engine cannot decode yet is reported too.
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _wake: NetworkWake) {
        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        engine.poll_network();
        if let Some(notice) = engine.take_notice() {
            eprintln!("silver-browser: {notice}");
        }
        if let LoadState::Failed(error) = engine.load_state().clone() {
            if let Some(url) = self.loading.take() {
                eprintln!("silver-browser: {url}: {error}");
            }
        } else if let Some(url) = engine.url()
            && self.loading.take().is_some()
            && let Some(encoding) = engine.undecodable_encoding()
        {
            eprintln!(
                "silver-browser: {url} is in {encoding}, which this version cannot decode; \
                 it is shown as UTF-8"
            );
        }
        if let Some(window) = self.window.clone() {
            self.redraw_if_wanted(&window);
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return; // Already running (e.g. resume after suspend).
        }

        let attributes = Window::default_attributes()
            .with_title(WINDOW_TITLE)
            .with_inner_size(winit::dpi::PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT))
            .with_resizable(true);

        match event_loop.create_window(attributes) {
            Ok(window) => {
                let window = Arc::new(window);
                match softbuffer::Context::new(window.clone()) {
                    Ok(context) => match softbuffer::Surface::new(&context, window.clone()) {
                        Ok(surface) => {
                            self.context = Some(context);
                            self.surface = Some(surface);
                            self.window = Some(window);
                            self.window.as_ref().expect("just set").request_redraw();
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
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                // While a scroll has not settled, the next frame is
                // wanted, which the engine advances by the time passed.
                self.draw();
                self.redraw_if_wanted(&window);
            }
            WindowEvent::Resized(size) => {
                if let (Some(w), Some(h)) =
                    (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                    && let Some(surface) = self.surface.as_mut()
                    && let Err(e) = surface.resize(w, h)
                {
                    eprintln!("surface resize failed: {e}");
                }
                window.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // Positive deltas reveal content above (the content
                // moves down), so the scroll offset decreases. The
                // engine chains the turn from the scroll container under
                // the pointer out to the viewport (M10.5).
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => -lines * LINE_PX,
                    MouseScrollDelta::PixelDelta(p) => -p.y as f32,
                };
                let (x, y) = self.cursor;
                if let Some(engine) = self.engine.as_mut() {
                    engine.wheel(x, y, dy);
                }
                self.redraw_if_wanted(&window);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
                if self.thumb_grab.is_some() {
                    self.drag();
                    self.redraw_if_wanted(&window);
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => match state {
                ElementState::Pressed => {
                    self.press();
                    self.redraw_if_wanted(&window);
                }
                ElementState::Released => {
                    self.release();
                    self.redraw_if_wanted(&window);
                }
            },
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } if self.key_scroll(&logical_key) => self.redraw_if_wanted(&window),
            _ => {}
        }
    }
}

/// What the command line asks to show: `None` for `about:silver`, or
/// the URL to load. An argument naming an existing file is that file's
/// `file:` URL, and any other must parse as an absolute URL. The file
/// comes first: on Windows, `C:\page.html` parses as a URL too, of the
/// scheme `c`.
fn start_page(mut args: impl Iterator<Item = OsString>) -> Result<Option<Url>, String> {
    let Some(arg) = args.next() else {
        return Ok(None);
    };
    if args.next().is_some() {
        return Err("one URL or path at most".to_owned());
    }
    let path = Path::new(&arg);
    if path.exists() {
        let absolute =
            std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
        return Url::from_file_path(&absolute)
            .map(Some)
            .ok_or_else(|| format!("{}: no file: URL names this path", path.display()));
    }
    let not_found = || format!("{}: not a URL, and no such file", path.display());
    let text = arg.to_str().ok_or_else(not_found)?;
    Url::parse(text).map(Some).map_err(|_| not_found())
}

fn main() {
    let start = match start_page(std::env::args_os().skip(1)) {
        Ok(start) => start,
        Err(message) => {
            eprintln!("silver-browser: {message}");
            eprintln!("usage: silver-browser [URL or path]");
            std::process::exit(2);
        }
    };

    let mut engine = match Engine::new() {
        Ok(engine) => engine,
        Err(e) => {
            eprintln!("engine initialization failed: {e}");
            return;
        }
    };

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
    let about_silver = || Url::parse("about:silver").expect("about:silver parses");
    engine.navigate(start.clone().unwrap_or_else(about_silver));

    let mut browser = Browser::new(Some(engine), start);
    if event_loop.run_app(&mut browser).is_err() {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(args: &[&str]) -> Result<Option<Url>, String> {
        start_page(args.iter().map(OsString::from))
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
    fn anything_else_is_refused() {
        assert!(start(&["no such page.html"]).is_err());
        assert!(start(&["https://example.com/", "https://example.org/"]).is_err());
    }
}
