//! Canvas 2D (M30, decision D22; see `docs/design/canvas.md`): the
//! HTML Standard's 2D rendering context, drawing on the processor with
//! `tiny-skia` into a bitmap of premultiplied 8-bit RGBA, the shape of
//! `images::Image`. It knows no DOM and no script: a [`Canvas`] is a
//! value the engine owns and the bindings drive.

#![forbid(unsafe_code)]

mod blur;
mod color;
mod draw;
mod path;
mod pixels;
mod state;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use tiny_skia::Pixmap;

pub use color::{Rgba, parse_color, serialize_color};
pub use path::Path;
pub use pixels::{blank_image_data, too_large};
pub use state::{
    Composite, Dash, Gradient, GradientKind, LineCap, LineJoin, Matrix, Paint, Pattern, Repetition,
    State,
};
pub use tiny_skia::FillRule;

/// The most pixels a canvas's bitmap may have.
pub const MAX_CANVAS_PIXELS: u64 = 1 << 25;
/// The largest side of a canvas's bitmap.
pub const MAX_CANVAS_SIDE: u32 = 16_384;
/// The most states a context keeps saved.
pub const MAX_CANVAS_STATES: usize = 1_024;
/// The most points a path holds.
pub const MAX_PATH_POINTS: usize = 1 << 20;
/// The largest shadow blur drawn.
pub const MAX_SHADOW_BLUR: f64 = 256.0;
/// The longest dash list.
pub const MAX_DASHES: usize = 1_024;

/// Why an operation is refused: the DOM exception's name and the
/// reference browser's words, without the method's prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasError {
    pub name: &'static str,
    pub message: String,
}

impl CanvasError {
    pub(crate) fn new(name: &'static str, message: impl Into<String>) -> CanvasError {
        CanvasError {
            name,
            message: message.into(),
        }
    }
}

/// A canvas's bitmap and its 2D context: the state and its stack, the
/// current default path, and whether the bitmap is origin-clean.
pub struct Canvas {
    width: u32,
    height: u32,
    /// None when the bitmap is empty or past the bounds: such a canvas
    /// draws nothing and reads back transparent.
    pixmap: Option<Pixmap>,
    pub(crate) state: State,
    stack: Vec<State>,
    /// The current default path, in the bitmap's coordinates.
    pub(crate) path: Path,
    /// Moved on by every change to the bitmap.
    generation: u64,
    origin_clean: bool,
    /// The context's `alpha: false`: the bitmap is always opaque.
    opaque: bool,
    /// The work done since it was last taken.
    work: u64,
}

impl Canvas {
    /// A canvas of `width` by `height`, transparent black, or opaque
    /// black when `opaque`.
    pub fn new(width: u32, height: u32, opaque: bool) -> Canvas {
        let mut canvas = Canvas {
            width,
            height,
            pixmap: None,
            state: State::default(),
            stack: Vec::new(),
            path: Path::default(),
            generation: 0,
            origin_clean: true,
            opaque,
            work: 0,
        };
        canvas.reset_bitmap();
        canvas
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Whether the bitmap could be made: not empty and within the bounds.
    pub fn has_bitmap(&self) -> bool {
        self.pixmap.is_some()
    }

    /// The bitmap's size set, as setting `width` or `height` does: the
    /// bitmap cleared, the state and the path reset, the canvas clean
    /// again.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.reset();
        self.origin_clean = true;
    }

    /// `reset()`: the bitmap cleared, the state, its stack and the path
    /// as new.
    pub fn reset(&mut self) {
        self.state = State::default();
        self.stack.clear();
        self.path = Path::default();
        self.reset_bitmap();
    }

    fn reset_bitmap(&mut self) {
        let pixels = u64::from(self.width) * u64::from(self.height);
        self.pixmap = (self.width <= MAX_CANVAS_SIDE
            && self.height <= MAX_CANVAS_SIDE
            && pixels <= MAX_CANVAS_PIXELS)
            .then(|| Pixmap::new(self.width, self.height))
            .flatten();
        if self.opaque
            && let Some(pixmap) = &mut self.pixmap
        {
            pixmap.fill(tiny_skia::Color::BLACK);
        }
        self.changed();
    }

    /// The bitmap's generation, moved on by every change to it.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn changed(&mut self) {
        self.generation += 1;
    }

    /// Whether no image of another origin has been drawn without its
    /// consent.
    pub fn origin_clean(&self) -> bool {
        self.origin_clean
    }

    /// The bitmap no longer origin-clean.
    pub fn taint(&mut self) {
        self.origin_clean = false;
    }

    pub fn opaque(&self) -> bool {
        self.opaque
    }

    /// The bitmap's bytes, premultiplied RGBA, row by row.
    pub fn bytes(&self) -> Option<&[u8]> {
        self.pixmap.as_ref().map(|pixmap| pixmap.data())
    }

    /// A copy of the bitmap as an image, for a frame to show.
    pub fn snapshot(&self) -> Option<Arc<images::Image>> {
        let pixmap = self.pixmap.as_ref()?;
        Some(Arc::new(images::Image {
            width: self.width,
            height: self.height,
            frames: vec![images::Frame {
                pixels: Arc::from(pixmap.data()),
                delay: 0,
            }],
            plays: images::Plays::Forever,
        }))
    }

    /// The work done since the last call: pixels touched and points
    /// added, for the run's budget.
    pub fn take_work(&mut self) -> u64 {
        std::mem::take(&mut self.work)
    }

    pub(crate) fn charge(&mut self, units: u64) {
        self.work = self.work.saturating_add(units);
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// `save()`: the state pushed, past [`MAX_CANVAS_STATES`] not.
    pub fn save(&mut self) {
        if self.stack.len() < MAX_CANVAS_STATES {
            self.stack.push(self.state.clone());
        }
    }

    /// `restore()`: the last state saved taken back, nothing when none
    /// is.
    pub fn restore(&mut self) {
        if let Some(state) = self.stack.pop() {
            self.state = state;
        }
    }

    /// The current default path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `beginPath()`.
    pub fn begin_path(&mut self) {
        self.path = Path::default();
    }

    /// `f` given the current default path and the transform its points
    /// are added through, as `moveTo()` and the rest add them.
    pub fn with_path<R>(&mut self, f: impl FnOnce(&mut Path, &Matrix) -> R) -> R {
        let before = self.path.len();
        let out = f(&mut self.path, &self.state.transform);
        self.charge(self.path.len().saturating_sub(before) as u64);
        out
    }

    /// The current transform multiplied by `m`, as `transform()`,
    /// `scale()`, `rotate()` and `translate()` do: nothing when a value is
    /// not finite.
    pub fn transform(&mut self, m: &Matrix) {
        if m.is_finite() {
            self.state.transform = self.state.transform.then(m);
        }
    }

    /// `setTransform()`.
    pub fn set_transform(&mut self, m: &Matrix) {
        if m.is_finite() {
            self.state.transform = *m;
        }
    }

    pub fn scale(&mut self, x: f64, y: f64) {
        self.transform(&Matrix::new(x, 0.0, 0.0, y, 0.0, 0.0));
    }

    pub fn rotate(&mut self, angle: f64) {
        if angle.is_finite() {
            let (sin, cos) = angle.sin_cos();
            self.transform(&Matrix::new(cos, sin, -sin, cos, 0.0, 0.0));
        }
    }

    pub fn translate(&mut self, x: f64, y: f64) {
        self.transform(&Matrix::new(1.0, 0.0, 0.0, 1.0, x, y));
    }
}
