//! The shell: the only crate that talks to the OS.
//!
//! Owns the window, the event loop, and the present pipeline (winit +
//! softbuffer). All rendering decisions live in `engine`; this file
//! pumps frames to the screen and turns input into engine calls (M9):
//! the wheel (at the pointer, so a scroll container under it takes it
//! first, M10.5), the keys and the scrollbar into scrolling, a click
//! into a hit test. A scroll animates over frames, so a redraw that
//! leaves the engine mid-animation asks for the next one.

use std::num::NonZeroU32;
use std::sync::Arc;

use engine::Engine;
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
}

impl Browser {
    fn new(engine: Option<Engine>) -> Self {
        Browser {
            window: None,
            context: None,
            surface: None,
            engine,
            cursor: (0.0, 0.0),
            thumb_grab: None,
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

        let rgba = engine.frame(size.width, size.height);
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
    /// thumb, jumps a page from its track, or hit-tests the document
    /// for a link (M9.4; following it is M15).
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
        if let Some(href) = engine.link_at(x, y) {
            println!("link: {href}");
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

impl ApplicationHandler for Browser {
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
                // One animation step per frame; while the scroll has
                // not settled, the next frame is wanted.
                let animating = self.engine.as_mut().is_some_and(Engine::tick);
                self.draw();
                if animating {
                    window.request_redraw();
                }
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
                window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
                if self.thumb_grab.is_some() {
                    self.drag();
                    window.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => match state {
                ElementState::Pressed => {
                    self.press();
                    window.request_redraw();
                }
                ElementState::Released => self.thumb_grab = None,
            },
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } if self.key_scroll(&logical_key) => window.request_redraw(),
            _ => {}
        }
    }
}

fn main() {
    let engine = match Engine::new() {
        Ok(engine) => Some(engine),
        Err(e) => {
            eprintln!("engine initialization failed: {e}");
            return;
        }
    };

    let event_loop = match EventLoop::new() {
        Ok(event_loop) => event_loop,
        Err(e) => {
            eprintln!("event loop creation failed: {e}");
            std::process::exit(1);
        }
    };

    let mut browser = Browser::new(engine);
    if event_loop.run_app(&mut browser).is_err() {
        std::process::exit(1);
    }
}
