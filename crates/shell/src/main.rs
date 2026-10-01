//! The shell: the only crate that talks to the OS.
//!
//! Owns the window, the event loop, and the present pipeline (winit +
//! softbuffer). All rendering decisions live in `engine`; this file just
//! pumps frames to the screen.

use std::num::NonZeroU32;
use std::sync::Arc;

use engine::Engine;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

/// Window configuration.
const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 800;
const WINDOW_TITLE: &str = "Random Browser";

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
}

impl Browser {
    fn new(engine: Option<Engine>) -> Self {
        Browser {
            window: None,
            context: None,
            surface: None,
            engine,
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
        let Some(window) = self.window.as_ref() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.draw(),
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
