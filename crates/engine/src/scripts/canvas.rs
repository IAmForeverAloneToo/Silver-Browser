//! The document's canvases (M30, `docs/design/canvas.md`): each
//! `canvas` element's 2D context, kept by its node while the node
//! lives, each `OffscreenCanvas` and its context, and the objects
//! scripts make for them, `Path2D`s, `CanvasGradient`s, `CanvasPattern`s
//! and `ImageBitmap`s, by number until the bindings let them go. The
//! bindings' calls of them, `Call::Canvas`, are answered here.
//!
//! A document's canvases are bounded together, past each canvas's own
//! bounds: their bitmaps by [`MAX_CANVAS_MEMORY`], the objects scripts
//! hold by [`MAX_CANVAS_OBJECTS`], and the points of every path and the
//! stops of every gradient by [`MAX_CANVAS_POINTS`] and
//! [`MAX_CANVAS_STOPS`], so that a loop making them ends in a refusal
//! or in nothing more drawn, not in the engine's memory spent.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use canvas::{
    Canvas, CanvasError, Caps, Composite, Direction, FillRule, FontBase, Gradient, GradientKind,
    Kerning, LineCap, LineJoin, Matrix, Paint, Path, Pattern, Quality, Rendering, Rgba, Spacing,
    Stretch, TextAlign, TextBaseline, parse_color, parse_font, serialize_color,
};
use dom::NodeId;
use js::{Answer, CanvasCall, DomError};

use crate::image_store::{Drawable, Scanned};

/// The most bytes a document's bitmaps hold in all: a canvas made or
/// resized past it has no bitmap, and draws nothing.
pub(crate) const MAX_CANVAS_MEMORY: u64 = 512 << 20;
/// The most `Path2D`s, gradients and patterns a document's scripts hold
/// at once: making one more is a `RangeError`.
pub(crate) const MAX_CANVAS_OBJECTS: usize = 1 << 18;
/// The most points a document's paths hold together, contexts' and
/// `Path2D`s': past it, a point is not added.
pub(crate) const MAX_CANVAS_POINTS: usize = 1 << 22;
/// The most colour stops a document's gradients hold together: past
/// it, a stop is not added.
pub(crate) const MAX_CANVAS_STOPS: usize = 1 << 20;

/// The most bytes of pixels decoded for drawing a document keeps
/// (M30): past it an image is decoded again for each draw.
pub(crate) const MAX_DRAWN_DECODED: usize = 256 << 20;

/// What the shown document's `img` elements give a canvas to draw
/// (M30), shared between the engine, which keeps it as images arrive,
/// and the scripts' host, which draws from it: for the page named, by
/// element; and the pixels decoded for drawing, by their bytes.
#[derive(Default)]
pub(crate) struct Drawables {
    pub(crate) page: Option<crate::loader::PageId>,
    by_node: HashMap<NodeId, (Scanned, Drawable)>,
    decoded: HashMap<usize, Arc<images::Image>>,
    bytes: usize,
}

/// The list as the engine and the host share it.
pub(crate) type SharedDrawables = Arc<Mutex<Drawables>>;

/// What an `img` gives a draw: its pixels and whether they leave a
/// canvas clean; that it is broken; or nothing yet.
pub(crate) enum Drawn {
    Pixels(Arc<images::Image>, bool),
    Broken,
    Coming,
}

impl Drawables {
    /// The engine's list, as it is now: decoded pixels whose bytes are
    /// gone let go.
    pub(crate) fn update(&mut self, by_node: HashMap<NodeId, (Scanned, Drawable)>) {
        let alive: std::collections::HashSet<usize> = by_node
            .values()
            .filter_map(|(_, drawable)| match drawable {
                Drawable::Ready { bytes, .. } => Some(Arc::as_ptr(bytes) as *const u8 as usize),
                Drawable::Broken => None,
            })
            .collect();
        let bytes = &mut self.bytes;
        self.decoded.retain(|key, image| {
            let keep = alive.contains(key);
            if !keep {
                *bytes -= image_bytes(image);
            }
            keep
        });
        self.by_node = by_node;
    }

    /// The image of the `img` `node` scanned for what it is `now`.
    fn of(&self, node: NodeId, now: &Scanned) -> Option<&Drawable> {
        self.by_node
            .get(&node)
            .filter(|(was, _)| was == now)
            .map(|(_, drawable)| drawable)
    }

    /// Where the `img` `node`'s image stands, for `complete` and its
    /// natural size: 0 coming or none, 1 there, 2 broken; and its size.
    pub(crate) fn state(&self, node: NodeId, now: &Scanned) -> (u8, u32, u32) {
        match self.of(node, now) {
            None => (0, 0, 0),
            Some(Drawable::Broken) => (2, 0, 0),
            Some(Drawable::Ready { size, .. }) => (1, size.0, size.1),
        }
    }

    /// What the `img` `node` gives a draw, decoded now if it must be.
    pub(crate) fn get(&mut self, node: NodeId, now: &Scanned) -> Drawn {
        let Some(drawable) = self.of(node, now) else {
            return Drawn::Coming;
        };
        let (bytes, decoded, clean, size) = match drawable {
            Drawable::Broken => return Drawn::Broken,
            Drawable::Ready {
                bytes,
                decoded,
                clean,
                size,
            } => (bytes, decoded, *clean, *size),
        };
        if let Some(image) = decoded {
            return Drawn::Pixels(Arc::clone(image), clean);
        }
        let key = Arc::as_ptr(bytes) as *const u8 as usize;
        if let Some(image) = self.decoded.get(&key) {
            return Drawn::Pixels(Arc::clone(image), clean);
        }
        // An SVG image at its natural size (M32), its text in no face:
        // recorded.
        let image = match images::decode(bytes) {
            Ok(image) => image,
            Err(_) => match crate::svg_images::draw(bytes, size, None) {
                Ok(image) => image,
                Err(_) => return Drawn::Broken,
            },
        };
        let image = Arc::new(image);
        let size = image_bytes(&image);
        if self.bytes + size <= MAX_DRAWN_DECODED {
            self.bytes += size;
            self.decoded.insert(key, Arc::clone(&image));
        }
        Drawn::Pixels(image, clean)
    }
}

/// The bytes an image's frames hold.
fn image_bytes(image: &images::Image) -> usize {
    image.frames.iter().map(|frame| frame.pixels.len()).sum()
}

/// The kinds of object a call names, as the bindings number them.
const CONTEXT: u8 = 0;
const OFFSCREEN: u8 = 1;
const PATH: u8 = 2;
const GRADIENT: u8 = 3;
const PATTERN: u8 = 4;
const BITMAP: u8 = 5;

/// Whose a context and its bitmap are: a `canvas` element's, or an
/// `OffscreenCanvas`'s, by its number. The bindings name the second by
/// the negative of its number where the first is named by the element's
/// handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Owner {
    Element(NodeId),
    Offscreen(u32),
}

/// A `canvas` element that gave its control to an `OffscreenCanvas`
/// (M31.4): its number, which a worker's frames name; the offscreen of
/// this document that draws for it, while it is here; the last frame a
/// worker sent, and the size it has.
struct Placeholder {
    number: u32,
    local: Option<u32>,
    frame: Option<(u64, Arc<images::Image>)>,
    size: (u32, u32),
}

/// What a worker commits to a placeholder after a task (M31.4): its
/// number, the offscreen's size and its pixels, none with no context.
pub(crate) type Commit = (u32, (u32, u32), Option<Arc<images::Image>>);

/// An `ImageBitmap`'s pixels and whether they are origin-clean; `None`
/// once it is closed.
type Bitmap = Option<(Arc<images::Image>, bool)>;

/// The refusal of a closed bitmap drawn.
fn detached() -> DomError {
    DomError::new("InvalidStateError", "The image source is detached")
}

/// What the host knows of the call that the canvases do not: the
/// element a context's call is made on and its size by its attributes,
/// its computed `color`, which `currentcolor` is, and for a call of
/// text what a font's relative sizes are of, whether the element's
/// direction is right to left, and the window's fonts.
pub(crate) struct Element {
    pub(crate) owner: Owner,
    pub(crate) size: (u32, u32),
    pub(crate) color: Rgba,
    pub(crate) font_base: FontBase,
    pub(crate) rtl: bool,
    pub(crate) fonts: Option<text::FontCollection>,
    /// The image a `drawImage()` or a `createPattern()` names, when it
    /// has pixels to draw.
    pub(crate) source: Option<canvas::Source>,
}

/// Whether a context's call of `op` names an image (its kind and which
/// after the context's): the host finds its pixels first.
pub(crate) fn needs_source(op: &str) -> bool {
    matches!(op, "drawImage" | "createPattern")
}

/// Whether a context's call of `op` reads what [`Element`] holds for
/// text: the host works it out only then.
pub(crate) fn needs_text(op: &str) -> bool {
    matches!(
        op,
        "font"
            | "letterSpacing"
            | "wordSpacing"
            | "direction"
            | "fillText"
            | "strokeText"
            | "measureText"
    )
}

#[derive(Default)]
pub(crate) struct Canvases {
    contexts: HashMap<Owner, Canvas>,
    /// Each `OffscreenCanvas`'s size, by its number.
    offscreen: HashMap<u32, (u32, u32)>,
    bitmaps: HashMap<u32, Bitmap>,
    paths: HashMap<u32, Path>,
    gradients: HashMap<u32, Arc<Mutex<Gradient>>>,
    patterns: HashMap<u32, Arc<Mutex<Pattern>>>,
    next: u32,
    /// The points every path holds, the contexts' and the `Path2D`s'.
    points: usize,
    /// The stops the gradients scripts hold have.
    stops: usize,
    /// A bitmap changed since the last frame took this.
    changed: bool,
    /// Each bitmap as an image, with the generation it was taken at.
    snapshots: HashMap<Owner, (u64, Arc<images::Image>)>,
    /// The work done since it was last taken.
    work: u64,
    /// Colours the styles were given, parsed, by their text (M30.4): a
    /// page sets the same few again and again, a particle's each frame.
    colours: RefCell<HashMap<String, Option<Rgba>>>,
    /// The placeholders (M31.4), by element; each offscreen with one,
    /// by number, with the placeholder's; the frames made for them, and
    /// the sizes their elements are to take.
    placeholders: HashMap<NodeId, Placeholder>,
    links: HashMap<u32, u32>,
    next_placeholder: u32,
    frames_made: u64,
    resizes: Vec<(NodeId, (u32, u32))>,
    /// What a worker last committed of each linked offscreen: its
    /// generation and size.
    committed: HashMap<u32, (u64, (u32, u32))>,
}

/// The most colours [`Canvases`] keeps parsed; past it they are let go.
pub(crate) const MAX_KEPT_COLOURS: usize = 512;

/// Whether a call of `op` names a context, its first number its kind:
/// those that make objects, or work on a gradient or a pattern, name
/// none.
pub(crate) fn names_context(op: &str) -> bool {
    !matches!(
        op,
        "path"
            | "linearGradient"
            | "radialGradient"
            | "conicGradient"
            | "release"
            | "addColorStop"
            | "patternTransform"
            | "imageState"
            | "imageBitmap"
            | "offscreen"
            | "offscreenResize"
            | "offscreenAway"
            | "offscreenAdopt"
            | "bitmapBytes"
            | "bitmapClean"
            | "bitmapFrom"
            | "closeBitmap"
    )
}

/// A canvas's refusal as the script's exception.
fn refused(error: CanvasError) -> DomError {
    DomError::new(error.name, error.message)
}

fn bad_call() -> DomError {
    DomError::new("TypeError", "Illegal invocation")
}

/// The number at `i`, NaN when there is none.
fn at(numbers: &[f64], i: usize) -> f64 {
    numbers.get(i).copied().unwrap_or(f64::NAN)
}

/// A fill rule as the bindings number it: 1 `evenodd`, else `nonzero`.
fn rule(number: f64) -> FillRule {
    if number == 1.0 {
        FillRule::EvenOdd
    } else {
        FillRule::Winding
    }
}

fn number(value: f64) -> Answer {
    Answer::Number(value)
}

/// The bytes a canvas of this size and bitmap holds.
fn bytes_of(canvas: &Canvas) -> u64 {
    canvas.bytes().map_or(0, |bytes| bytes.len() as u64)
}

impl Canvases {
    /// The work done since the last call, for the run's budget.
    pub(crate) fn take_work(&mut self) -> u64 {
        std::mem::take(&mut self.work)
    }

    /// Whether the canvas `node`'s bitmap is origin-clean: one with no
    /// context is.
    pub(crate) fn origin_clean(&self, owner: Owner) -> bool {
        self.contexts.get(&owner).is_none_or(Canvas::origin_clean)
    }

    /// The size of the `OffscreenCanvas` `id`.
    pub(crate) fn offscreen_size(&self, id: u32) -> Option<(u32, u32)> {
        self.offscreen.get(&id).copied()
    }

    /// The pixels of the bitmap `id`, the refusal of a closed one.
    pub(crate) fn bitmap(&self, id: u32) -> Result<(Arc<images::Image>, bool), DomError> {
        match self.bitmaps.get(&id) {
            Some(Some((image, clean))) => Ok((Arc::clone(image), *clean)),
            Some(None) => Err(detached()),
            None => Err(bad_call()),
        }
    }

    /// A new `ImageBitmap` of `image`: its number.
    pub(crate) fn make_bitmap(
        &mut self,
        image: Arc<images::Image>,
        clean: bool,
    ) -> Result<u32, DomError> {
        let id = self.new_id()?;
        self.bitmaps.insert(id, Some((image, clean)));
        Ok(id)
    }

    /// Whether a bitmap changed since the last frame took this (M30.3).
    pub(crate) fn changed(&self) -> bool {
        self.changed
    }

    pub(crate) fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// The bitmap of `node`'s canvas as an image, as it is now, for a
    /// print, which keeps nothing.
    pub(crate) fn image(&self, node: NodeId) -> Option<Arc<images::Image>> {
        if let Some(placeholder) = self.placeholders.get(&node) {
            return match placeholder.local {
                Some(local) => self
                    .contexts
                    .get(&Owner::Offscreen(local))
                    .and_then(Canvas::snapshot),
                None => placeholder
                    .frame
                    .as_ref()
                    .map(|(_, image)| Arc::clone(image)),
            };
        }
        let owner = Owner::Element(node);
        let canvas = self.contexts.get(&owner)?;
        match self.snapshots.get(&owner) {
            Some((taken, image)) if *taken == canvas.generation() => Some(Arc::clone(image)),
            _ => canvas.snapshot(),
        }
    }

    /// The bitmap of `node`'s canvas as an image, and its generation: a
    /// copy taken when the generation moved, the same until it moves
    /// again.
    pub(crate) fn pixels(&mut self, owner: Owner) -> Option<(u64, Arc<images::Image>)> {
        // A placeholder shows its offscreen's, or a worker's last frame.
        if let Owner::Element(node) = owner
            && let Some(placeholder) = self.placeholders.get(&node)
        {
            return match placeholder.local {
                Some(local) => self.pixels(Owner::Offscreen(local)),
                None => placeholder.frame.clone(),
            };
        }
        let canvas = self.contexts.get(&owner)?;
        let generation = canvas.generation();
        if let Some((taken, image)) = self.snapshots.get(&owner)
            && *taken == generation
        {
            return Some((generation, Arc::clone(image)));
        }
        let image = canvas.snapshot()?;
        self.snapshots
            .insert(owner, (generation, Arc::clone(&image)));
        Some((generation, image))
    }

    /// The points the `Path2D` `id` holds.
    #[cfg(test)]
    pub(crate) fn path_len(&self, id: u32) -> Option<usize> {
        self.paths.get(&id).map(Path::len)
    }

    /// A canvas of `size` that the document's bitmaps have room for, or
    /// one with none: `others` bytes are held by the rest.
    fn made(size: (u32, u32), opaque: bool, others: u64) -> Canvas {
        let bytes = u64::from(size.0) * u64::from(size.1) * 4;
        if others.saturating_add(bytes) > MAX_CANVAS_MEMORY {
            Canvas::new(0, 0, opaque)
        } else {
            Canvas::new(size.0, size.1, opaque)
        }
    }

    /// The bytes the bitmaps but `owner`'s hold.
    fn others(&self, owner: Owner) -> u64 {
        self.contexts
            .iter()
            .filter(|(o, _)| **o != owner)
            .map(|(_, canvas)| bytes_of(canvas))
            .sum()
    }

    /// The canvas `node`'s `width` or `height` attribute was set or
    /// removed: its bitmap made again at its size, cleared, and its
    /// context's state reset, as setting either does.
    pub(crate) fn resized(&mut self, owner: Owner, size: (u32, u32)) {
        if let Owner::Offscreen(id) = owner {
            self.offscreen.insert(id, size);
            // Its placeholder here takes its size.
            if let Some((node, placeholder)) = self
                .placeholders
                .iter_mut()
                .find(|(_, placeholder)| placeholder.local == Some(id))
                && placeholder.size != size
            {
                placeholder.size = size;
                self.resizes.push((*node, size));
            }
        }
        let others = self.others(owner);
        if let Some(canvas) = self.contexts.get_mut(&owner) {
            self.points -= canvas.path().len();
            let bytes = u64::from(size.0) * u64::from(size.1) * 4;
            if others.saturating_add(bytes) > MAX_CANVAS_MEMORY {
                canvas.resize(0, 0);
            } else {
                canvas.resize(size.0, size.1);
            }
            self.changed = true;
        }
    }

    /// The contexts of nodes no longer in the document's arena let go.
    pub(crate) fn retain(&mut self, mut alive: impl FnMut(NodeId) -> bool) {
        let points = &mut self.points;
        self.contexts.retain(|&owner, canvas| {
            let keep = match owner {
                Owner::Element(node) => alive(node),
                Owner::Offscreen(_) => true,
            };
            if !keep {
                *points -= canvas.path().len();
            }
            keep
        });
        let contexts = &self.contexts;
        self.snapshots.retain(|node, _| contexts.contains_key(node));
        self.placeholders.retain(|node, _| alive(*node));
    }

    /// A frame worker sent placeholder `number` (M31.4): shown at the
    /// next frame, its element given its size.
    pub(crate) fn placeholder_frame(
        &mut self,
        number: u32,
        size: (u32, u32),
        image: Option<Arc<images::Image>>,
    ) {
        let Some((node, placeholder)) = self
            .placeholders
            .iter_mut()
            .find(|(_, placeholder)| placeholder.number == number)
        else {
            return;
        };
        self.frames_made += 1;
        placeholder.frame = image.map(|image| ((1 << 40) + self.frames_made, image));
        if placeholder.size != size {
            placeholder.size = size;
            self.resizes.push((*node, size));
        }
        self.changed = true;
    }

    /// The sizes placeholders' elements are to take, taken.
    pub(crate) fn take_resizes(&mut self) -> Vec<(NodeId, (u32, u32))> {
        std::mem::take(&mut self.resizes)
    }

    /// What changed of each offscreen with a placeholder since it was
    /// last committed (M31.4), for a worker to send its document.
    pub(crate) fn commits(&mut self) -> Vec<Commit> {
        let links: Vec<(u32, u32)> = self.links.iter().map(|(id, n)| (*id, *n)).collect();
        let mut commits = Vec::new();
        for (id, number) in links {
            let size = self.offscreen_size(id).unwrap_or((0, 0));
            let pixels = self.pixels(Owner::Offscreen(id));
            let generation = pixels.as_ref().map_or(0, |(generation, _)| *generation);
            if self.committed.get(&id) == Some(&(generation, size)) {
                continue;
            }
            self.committed.insert(id, (generation, size));
            commits.push((number, size, pixels.map(|(_, image)| image)));
        }
        commits
    }

    /// The points the document's budget has room for yet.
    fn room(&self) -> usize {
        MAX_CANVAS_POINTS.saturating_sub(self.points)
    }

    /// A number for a new object, past [`MAX_CANVAS_OBJECTS`] none.
    fn new_id(&mut self) -> Result<u32, DomError> {
        let held = self.paths.len()
            + self.gradients.len()
            + self.patterns.len()
            + self.offscreen.len()
            + self.bitmaps.len();
        if held >= MAX_CANVAS_OBJECTS {
            return Err(DomError::new(
                "RangeError",
                format!("No more than {MAX_CANVAS_OBJECTS} canvas objects may be held."),
            ));
        }
        loop {
            self.next = self.next.wrapping_add(1);
            let id = self.next;
            if !self.paths.contains_key(&id)
                && !self.gradients.contains_key(&id)
                && !self.patterns.contains_key(&id)
                && !self.offscreen.contains_key(&id)
                && !self.bitmaps.contains_key(&id)
            {
                return Ok(id);
            }
        }
    }

    /// The number a gradient or a pattern a state holds goes by: its own
    /// if the bindings still hold it, a new one if they let it go.
    fn paint_answer(&mut self, paint: &Paint) -> Result<Answer, DomError> {
        Ok(match paint {
            Paint::Color(color) => Answer::String(serialize_color(*color)),
            Paint::Gradient(gradient) => {
                let found = self
                    .gradients
                    .iter()
                    .find(|(_, g)| Arc::ptr_eq(g, gradient))
                    .map(|(id, _)| *id);
                let id = match found {
                    Some(id) => id,
                    None => {
                        let id = self.new_id()?;
                        self.gradients.insert(id, gradient.clone());
                        id
                    }
                };
                Answer::Numbers(vec![f64::from(GRADIENT), f64::from(id)])
            }
            Paint::Pattern(pattern) => {
                let found = self
                    .patterns
                    .iter()
                    .find(|(_, p)| Arc::ptr_eq(p, pattern))
                    .map(|(id, _)| *id);
                let id = match found {
                    Some(id) => id,
                    None => {
                        let id = self.new_id()?;
                        self.patterns.insert(id, pattern.clone());
                        id
                    }
                };
                Answer::Numbers(vec![f64::from(PATTERN), f64::from(id)])
            }
        })
    }

    /// `text` as a colour, `current` the element's: kept once parsed,
    /// but for a text naming `currentcolor`, whose colour is the
    /// element's of the moment.
    pub(crate) fn colour(&self, text: &str, current: Rgba) -> Option<Rgba> {
        let names_current = text
            .as_bytes()
            .windows(12)
            .any(|w| w.eq_ignore_ascii_case(b"currentcolor"));
        if names_current {
            return parse_color(text, current);
        }
        if let Some(&kept) = self.colours.borrow().get(text) {
            return kept;
        }
        let parsed = parse_color(text, current);
        let mut colours = self.colours.borrow_mut();
        if colours.len() >= MAX_KEPT_COLOURS {
            colours.clear();
        }
        colours.insert(text.to_owned(), parsed);
        parsed
    }

    /// How many colours are kept parsed.
    #[cfg(test)]
    pub(crate) fn kept_colours(&self) -> usize {
        self.colours.borrow().len()
    }

    /// The paint a style setter's arguments name: a colour's text, or a
    /// gradient or pattern by its kind and number. `None` changes
    /// nothing, as a colour that does not parse does not.
    fn paint_of(&self, call: &CanvasCall, current: Rgba) -> Option<Paint> {
        if let Some(text) = call.strings.first() {
            return self.colour(text, current).map(Paint::Color);
        }
        let id = at(&call.numbers, 3) as u32;
        match at(&call.numbers, 2) as u8 {
            GRADIENT => self.gradients.get(&id).cloned().map(Paint::Gradient),
            PATTERN => self.patterns.get(&id).cloned().map(Paint::Pattern),
            _ => None,
        }
    }

    /// The bindings' call of a canvas or an object, `element` the
    /// element a context's call names.
    pub(crate) fn call(
        &mut self,
        call: CanvasCall,
        element: Option<Element>,
    ) -> Result<Answer, DomError> {
        let n = &call.numbers;
        // Calls that name no context.
        match call.op.as_str() {
            "path" => {
                let id = self.new_id()?;
                let path = match (call.strings.first(), n.first()) {
                    (Some(d), _) => Path::from_svg(d),
                    (None, Some(&from)) => {
                        self.paths.get(&(from as u32)).cloned().unwrap_or_default()
                    }
                    (None, None) => Path::new(),
                };
                let path = if path.len() > self.room() {
                    Path::new()
                } else {
                    path
                };
                self.work += path.len() as u64;
                self.points += path.len();
                self.paths.insert(id, path);
                return Ok(number(f64::from(id)));
            }
            "linearGradient" | "radialGradient" | "conicGradient" => {
                let kind = match call.op.as_str() {
                    "linearGradient" => GradientKind::Linear {
                        x0: at(n, 0),
                        y0: at(n, 1),
                        x1: at(n, 2),
                        y1: at(n, 3),
                    },
                    "radialGradient" => {
                        for (i, name) in [(2, "r0"), (5, "r1")] {
                            if at(n, i) < 0.0 {
                                return Err(DomError::new(
                                    "IndexSizeError",
                                    format!("The {name} provided is less than 0."),
                                ));
                            }
                        }
                        GradientKind::Radial {
                            x0: at(n, 0),
                            y0: at(n, 1),
                            r0: at(n, 2),
                            x1: at(n, 3),
                            y1: at(n, 4),
                            r1: at(n, 5),
                        }
                    }
                    _ => GradientKind::Conic {
                        angle: at(n, 0),
                        x: at(n, 1),
                        y: at(n, 2),
                    },
                };
                let id = self.new_id()?;
                self.gradients
                    .insert(id, Arc::new(Mutex::new(Gradient::new(kind))));
                return Ok(number(f64::from(id)));
            }
            "offscreen" => {
                let id = self.new_id()?;
                self.offscreen
                    .insert(id, (at(n, 0) as u32, at(n, 1) as u32));
                return Ok(number(f64::from(id)));
            }
            // HTMLCanvasElement's transferControlToOffscreen (M31.4): the
            // element a placeholder, an offscreen of its size drawing for
            // it.
            "transferControl" => {
                let Some(Element {
                    owner: Owner::Element(node),
                    size,
                    ..
                }) = element
                else {
                    return Err(bad_call());
                };
                if self.contexts.contains_key(&Owner::Element(node))
                    || self.placeholders.contains_key(&node)
                {
                    return Err(DomError::new(
                        "InvalidStateError",
                        "Cannot transfer control from this canvas.",
                    ));
                }
                let id = self.new_id()?;
                self.offscreen.insert(id, size);
                let which = self.next_placeholder;
                self.next_placeholder += 1;
                self.placeholders.insert(
                    node,
                    Placeholder {
                        number: which,
                        local: Some(id),
                        frame: None,
                        size,
                    },
                );
                self.links.insert(id, which);
                return Ok(Answer::Numbers(vec![
                    f64::from(id),
                    f64::from(size.0),
                    f64::from(size.1),
                ]));
            }
            // An offscreen transferred away: its placeholder's number,
            // or -1.
            "offscreenAway" => {
                let id = at(n, 1) as u32;
                if self.offscreen.remove(&id).is_none() {
                    return Err(bad_call());
                }
                let owner = Owner::Offscreen(id);
                if let Some(canvas) = self.contexts.remove(&owner) {
                    self.points -= canvas.path().len();
                }
                self.snapshots.remove(&owner);
                self.committed.remove(&id);
                let link = self.links.remove(&id);
                for placeholder in self.placeholders.values_mut() {
                    if placeholder.local == Some(id) {
                        placeholder.local = None;
                    }
                }
                return Ok(number(link.map_or(-1.0, f64::from)));
            }
            // An offscreen transferred here, of a size, drawing for the
            // placeholder it names: its number and size.
            "offscreenAdopt" => {
                let size = (at(n, 2) as u32, at(n, 3) as u32);
                let id = self.new_id()?;
                self.offscreen.insert(id, size);
                let link = at(n, 1);
                if (0.0..=f64::from(u32::MAX)).contains(&link) && link.fract() == 0.0 {
                    let which = link as u32;
                    self.links.insert(id, which);
                    if let Some(placeholder) = self
                        .placeholders
                        .values_mut()
                        .find(|placeholder| placeholder.number == which)
                    {
                        placeholder.local = Some(id);
                    }
                }
                return Ok(Answer::Numbers(vec![
                    f64::from(id),
                    f64::from(size.0),
                    f64::from(size.1),
                ]));
            }
            "offscreenResize" => {
                let id = at(n, 1) as u32;
                if !self.offscreen.contains_key(&id) {
                    return Err(bad_call());
                }
                self.resized(Owner::Offscreen(id), (at(n, 2) as u32, at(n, 3) as u32));
                return Ok(Answer::Nothing);
            }
            // A bitmap crossing to another realm as its pixels (M31),
            // premultiplied as it holds them, and made again there.
            "bitmapBytes" => {
                let (image, _) = self.bitmap(at(n, 1) as u32)?;
                let bytes = image
                    .frames
                    .first()
                    .map_or_else(Vec::new, |frame| frame.pixels.to_vec());
                return Ok(Answer::Bytes(bytes));
            }
            "bitmapClean" => {
                let (_, clean) = self.bitmap(at(n, 1) as u32)?;
                return Ok(Answer::Bool(clean));
            }
            "bitmapFrom" => {
                let (width, height) = (at(n, 0) as u32, at(n, 1) as u32);
                let bytes = call.bytes.clone().unwrap_or_default();
                if u64::from(width) * u64::from(height) * 4 != bytes.len() as u64 {
                    return Err(bad_call());
                }
                let image = Arc::new(images::Image {
                    width,
                    height,
                    frames: vec![images::Frame {
                        pixels: Arc::from(bytes),
                        delay: 0,
                    }],
                    plays: images::Plays::Forever,
                });
                let id = self.make_bitmap(image, at(n, 2) == 1.0)?;
                return Ok(Answer::Numbers(vec![
                    f64::from(id),
                    f64::from(width),
                    f64::from(height),
                ]));
            }
            "closeBitmap" => {
                if let Some(bitmap) = self.bitmaps.get_mut(&(at(n, 1) as u32)) {
                    *bitmap = None;
                }
                return Ok(Answer::Nothing);
            }
            "patternTransform" => {
                let m = Matrix::new(at(n, 2), at(n, 3), at(n, 4), at(n, 5), at(n, 6), at(n, 7));
                let pattern = self.patterns.get(&(at(n, 1) as u32)).ok_or_else(bad_call)?;
                if m.is_finite() {
                    pattern
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .transform = m;
                }
                return Ok(Answer::Nothing);
            }
            "release" => {
                let id = at(n, 1) as u32;
                match at(n, 0) as u8 {
                    PATH => {
                        if let Some(path) = self.paths.remove(&id) {
                            self.points -= path.len();
                        }
                    }
                    GRADIENT => {
                        if let Some(gradient) = self.gradients.remove(&id) {
                            let gradient = gradient.lock().unwrap_or_else(PoisonError::into_inner);
                            self.stops = self.stops.saturating_sub(gradient.stops.len());
                        }
                    }
                    PATTERN => {
                        self.patterns.remove(&id);
                    }
                    OFFSCREEN => {
                        self.offscreen.remove(&id);
                        let owner = Owner::Offscreen(id);
                        if let Some(canvas) = self.contexts.remove(&owner) {
                            self.points -= canvas.path().len();
                        }
                        self.snapshots.remove(&owner);
                    }
                    BITMAP => {
                        self.bitmaps.remove(&id);
                    }
                    _ => {}
                }
                return Ok(Answer::Nothing);
            }
            "addColorStop" => {
                let room = self.stops < MAX_CANVAS_STOPS;
                let gradient = self
                    .gradients
                    .get(&(at(n, 1) as u32))
                    .ok_or_else(bad_call)?;
                let text = call.strings.first().map_or("", String::as_str);
                let current = element.map_or(Rgba::BLACK, |e| e.color);
                let mut gradient = gradient.lock().unwrap_or_else(PoisonError::into_inner);
                let offset = at(n, 2);
                if !(0.0..=1.0).contains(&offset) {
                    // The crate words the range's refusal.
                    return gradient
                        .add_stop(offset, Rgba::BLACK)
                        .map(|()| Answer::Nothing)
                        .map_err(refused);
                }
                let Some(color) = parse_color(text, current) else {
                    return Err(DomError::new(
                        "SyntaxError",
                        format!("The value provided ('{text}') could not be parsed as a color."),
                    ));
                };
                if room {
                    gradient.add_stop(offset, color).map_err(refused)?;
                    drop(gradient);
                    self.stops += 1;
                }
                return Ok(Answer::Nothing);
            }
            _ => {}
        }
        let kind = at(n, 0) as u8;
        if kind == PATH {
            return self.path_call(&call);
        }
        if kind != CONTEXT {
            return Err(bad_call());
        }
        let element = element.ok_or_else(bad_call)?;
        let owner = element.owner;
        // An `OffscreenCanvas`'s size is kept here.
        let size = match owner {
            Owner::Element(_) => element.size,
            Owner::Offscreen(id) => self.offscreen_size(id).ok_or_else(bad_call)?,
        };
        if call.op == "context" {
            if !self.contexts.contains_key(&owner) {
                let canvas = Canvases::made(size, at(n, 2) == 0.0, self.others(owner));
                self.contexts.insert(owner, canvas);
            }
            return Ok(Answer::Nothing);
        }
        if call.op == "encode" {
            return self.encode(owner, size);
        }
        if call.op == "transfer" {
            return self.transfer(owner);
        }
        let room = self.room();
        let mut canvas = self.contexts.remove(&owner).ok_or_else(bad_call)?;
        let before = canvas.path().len();
        let generation = canvas.generation();
        let answer = self.context_call(&mut canvas, &call, &element, room);
        self.points = self.points - before + canvas.path().len();
        self.changed |= canvas.generation() != generation;
        self.work += canvas.take_work();
        self.contexts.insert(owner, canvas);
        answer
    }

    /// `toDataURL()`, `toBlob()` and `convertToBlob()`: the bitmap as a
    /// PNG file, a transparent one of the canvas's size for one with no
    /// context; an empty string for one of no area; refused for one that
    /// is not origin-clean, in the reference browser's words for each.
    fn encode(&mut self, owner: Owner, size: (u32, u32)) -> Result<Answer, DomError> {
        let offscreen = matches!(owner, Owner::Offscreen(_));
        let canvas = self.contexts.get(&owner);
        if canvas.is_some_and(|canvas| !canvas.origin_clean()) {
            return Err(DomError::new(
                "SecurityError",
                if offscreen {
                    "A tainted OffscreenCanvas may not be exported."
                } else {
                    "Tainted canvases may not be exported."
                },
            ));
        }
        if size.0 == 0 || size.1 == 0 {
            if offscreen {
                return Err(DomError::new(
                    "IndexSizeError",
                    "The size of the OffscreenCanvas is zero.",
                ));
            }
            return Ok(Answer::String(String::new()));
        }
        let png = match canvas {
            Some(canvas) => canvas.to_png(),
            None if u64::from(size.0) * u64::from(size.1) <= canvas::MAX_CANVAS_PIXELS => {
                Canvas::new(size.0, size.1, false).to_png()
            }
            None => None,
        };
        let Some(png) = png else {
            return Ok(Answer::String(String::new()));
        };
        self.work += u64::from(size.0) * u64::from(size.1) / 16;
        Ok(Answer::Bytes(png))
    }

    /// `transferToImageBitmap()`: the bitmap given to a new `ImageBitmap`,
    /// the canvas left a transparent one of its size: the bitmap's number
    /// and size.
    fn transfer(&mut self, owner: Owner) -> Result<Answer, DomError> {
        let Some(canvas) = self.contexts.get_mut(&owner) else {
            return Err(DomError::new(
                "InvalidStateError",
                "Cannot transfer an ImageBitmap from an OffscreenCanvas with no context",
            ));
        };
        let clean = canvas.origin_clean();
        let (width, height) = (canvas.width(), canvas.height());
        let image = canvas.snapshot().unwrap_or_else(|| {
            Arc::new(images::Image {
                width: 0,
                height: 0,
                frames: vec![images::Frame {
                    pixels: Arc::from(Vec::new()),
                    delay: 0,
                }],
                plays: images::Plays::Forever,
            })
        });
        canvas.clear_bitmap();
        self.changed = true;
        let id = self.make_bitmap(image, clean)?;
        Ok(Answer::Numbers(vec![
            f64::from(id),
            f64::from(width),
            f64::from(height),
        ]))
    }

    /// A call of a `Path2D` (kind 2, its number second).
    fn path_call(&mut self, call: &CanvasCall) -> Result<Answer, DomError> {
        let id = at(&call.numbers, 1) as u32;
        let room = self.room();
        if call.op == "addPath" {
            let other = self
                .paths
                .get(&(at(&call.numbers, 2) as u32))
                .cloned()
                .ok_or_else(bad_call)?;
            let n = &call.numbers[3..];
            let m = Matrix::new(at(n, 0), at(n, 1), at(n, 2), at(n, 3), at(n, 4), at(n, 5));
            let path = self.paths.get_mut(&id).ok_or_else(bad_call)?;
            let before = path.len();
            if m.is_finite() && other.len() <= room {
                path.add_path(&other, &m);
            }
            self.points = self.points - before + path.len();
            self.work += other.len() as u64;
            return Ok(Answer::Nothing);
        }
        let path = self.paths.get_mut(&id).ok_or_else(bad_call)?;
        let before = path.len();
        path.set_limit(Some(before + room));
        let answer = build(path, &call.op, &call.numbers[2..], &Matrix::IDENTITY);
        path.set_limit(None);
        self.points = self.points - before + path.len();
        self.work += path.len().saturating_sub(before) as u64;
        answer
            .ok_or_else(bad_call)?
            .map(|()| Answer::Nothing)
            .map_err(refused)
    }

    /// A call of an element's context.
    fn context_call(
        &mut self,
        canvas: &mut Canvas,
        call: &CanvasCall,
        element: &Element,
        room: usize,
    ) -> Result<Answer, DomError> {
        let n = &call.numbers[2.min(call.numbers.len())..];
        // A setter has its value after the target; a getter none.
        let set = !n.is_empty() || !call.strings.is_empty();
        let value = at(n, 0);
        let text = call.strings.first().map(String::as_str);
        let path_of = |paths: &HashMap<u32, Path>, id: f64| -> Result<Option<Path>, DomError> {
            if id < 0.0 {
                return Ok(None);
            }
            paths
                .get(&(id as u32))
                .cloned()
                .map(Some)
                .ok_or_else(bad_call)
        };
        let state = canvas.state_mut();
        let answer = match call.op.as_str() {
            "save" => {
                canvas.save();
                Answer::Nothing
            }
            "restore" => {
                canvas.restore();
                Answer::Nothing
            }
            "reset" => {
                canvas.reset();
                Answer::Nothing
            }
            "scale" => {
                canvas.scale(value, at(n, 1));
                Answer::Nothing
            }
            "rotate" => {
                canvas.rotate(value);
                Answer::Nothing
            }
            "translate" => {
                canvas.translate(value, at(n, 1));
                Answer::Nothing
            }
            "transform" | "setTransform" => {
                let m = Matrix::new(value, at(n, 1), at(n, 2), at(n, 3), at(n, 4), at(n, 5));
                if call.op == "transform" {
                    canvas.transform(&m);
                } else {
                    canvas.set_transform(&m);
                }
                Answer::Nothing
            }
            "resetTransform" => {
                canvas.set_transform(&Matrix::IDENTITY);
                Answer::Nothing
            }
            "getTransform" => {
                let m = canvas.state().transform;
                Answer::Numbers(vec![m.a, m.b, m.c, m.d, m.e, m.f])
            }
            "globalAlpha" if set => {
                state.set_global_alpha(value);
                Answer::Nothing
            }
            "globalAlpha" => number(state.global_alpha),
            "globalCompositeOperation" if set => {
                if let Some(composite) = text.and_then(Composite::from_name) {
                    state.composite = composite;
                }
                Answer::Nothing
            }
            "globalCompositeOperation" => Answer::String(state.composite.name().to_owned()),
            "imageSmoothingEnabled" if set => {
                state.image_smoothing = value != 0.0;
                Answer::Nothing
            }
            "imageSmoothingEnabled" => Answer::Bool(state.image_smoothing),
            "imageSmoothingQuality" if set => {
                if let Some(quality) = text.and_then(Quality::from_name) {
                    state.image_smoothing_quality = quality;
                }
                Answer::Nothing
            }
            "imageSmoothingQuality" => {
                Answer::String(state.image_smoothing_quality.name().to_owned())
            }
            "fillStyle" | "strokeStyle" if set => {
                if let Some(paint) = self.paint_of(call, element.color) {
                    let state = canvas.state_mut();
                    if call.op == "fillStyle" {
                        state.fill = paint;
                    } else {
                        state.stroke = paint;
                    }
                }
                Answer::Nothing
            }
            "fillStyle" => {
                let paint = state.fill.clone();
                self.paint_answer(&paint)?
            }
            "strokeStyle" => {
                let paint = state.stroke.clone();
                self.paint_answer(&paint)?
            }
            "shadowOffsetX" if set => {
                state.set_shadow_offset(Some(value), None);
                Answer::Nothing
            }
            "shadowOffsetX" => number(state.shadow_offset_x),
            "shadowOffsetY" if set => {
                state.set_shadow_offset(None, Some(value));
                Answer::Nothing
            }
            "shadowOffsetY" => number(state.shadow_offset_y),
            "shadowBlur" if set => {
                state.set_shadow_blur(value);
                Answer::Nothing
            }
            "shadowBlur" => number(state.shadow_blur),
            "shadowColor" if set => {
                if let Some(color) = text.and_then(|text| parse_color(text, element.color)) {
                    state.shadow_color = color;
                }
                Answer::Nothing
            }
            "shadowColor" => Answer::String(serialize_color(state.shadow_color)),
            "filter" if set => {
                if let Some(text) = text {
                    state.filter = text.to_owned();
                }
                Answer::Nothing
            }
            "filter" => Answer::String(state.filter.clone()),
            "font" if set => {
                if let Some(font) = text.and_then(|text| parse_font(text, &element.font_base)) {
                    state.set_font(font);
                }
                Answer::Nothing
            }
            "font" => Answer::String(state.font.serialize()),
            "textAlign" if set => {
                if let Some(align) = text.and_then(TextAlign::from_name) {
                    state.text_align = align;
                }
                Answer::Nothing
            }
            "textAlign" => Answer::String(state.text_align.name().to_owned()),
            "textBaseline" if set => {
                if let Some(baseline) = text.and_then(TextBaseline::from_name) {
                    state.text_baseline = baseline;
                }
                Answer::Nothing
            }
            "textBaseline" => Answer::String(state.text_baseline.name().to_owned()),
            "direction" if set => {
                if let Some(direction) = text.and_then(Direction::from_name) {
                    state.direction = direction;
                }
                Answer::Nothing
            }
            // `inherit` reads back as the element's, as the reference
            // browser has it.
            "direction" => Answer::String(
                if state.direction.is_rtl(element.rtl) {
                    "rtl"
                } else {
                    "ltr"
                }
                .to_owned(),
            ),
            "letterSpacing" | "wordSpacing" if set => {
                if let Some(spacing) =
                    text.and_then(|text| Spacing::parse(text, &element.font_base))
                {
                    if call.op == "letterSpacing" {
                        state.letter_spacing = spacing;
                    } else {
                        state.word_spacing = spacing;
                    }
                }
                Answer::Nothing
            }
            "letterSpacing" => Answer::String(state.letter_spacing.text.clone()),
            "wordSpacing" => Answer::String(state.word_spacing.text.clone()),
            "fontKerning" if set => {
                if let Some(kerning) = text.and_then(Kerning::from_name) {
                    state.font_kerning = kerning;
                }
                Answer::Nothing
            }
            "fontKerning" => Answer::String(state.font_kerning.name().to_owned()),
            "fontStretch" if set => {
                if let Some(stretch) = text.and_then(Stretch::from_name) {
                    state.font_stretch = stretch;
                }
                Answer::Nothing
            }
            "fontStretch" => Answer::String(state.font_stretch.name().to_owned()),
            "fontVariantCaps" if set => {
                if let Some(caps) = text.and_then(Caps::from_name) {
                    state.font_variant_caps = caps;
                }
                Answer::Nothing
            }
            "fontVariantCaps" => Answer::String(state.font_variant_caps.name().to_owned()),
            "textRendering" if set => {
                if let Some(rendering) = text.and_then(Rendering::from_name) {
                    state.text_rendering = rendering;
                }
                Answer::Nothing
            }
            "textRendering" => Answer::String(state.text_rendering.name().to_owned()),
            "lang" if set => {
                if let Some(text) = text {
                    state.lang = text.to_owned();
                }
                Answer::Nothing
            }
            "lang" => Answer::String(state.lang.clone()),
            // The source's kind and which first, then two, four or eight
            // numbers.
            "drawImage" => {
                if let Some(source) = &element.source {
                    let a = &n[2.min(n.len())..];
                    let (w, h) = (f64::from(source.width), f64::from(source.height));
                    let (src, dst) = match a.len() {
                        2 => (None, [at(a, 0), at(a, 1), w, h]),
                        4 => (None, [at(a, 0), at(a, 1), at(a, 2), at(a, 3)]),
                        _ => (
                            Some([at(a, 0), at(a, 1), at(a, 2), at(a, 3)]),
                            [at(a, 4), at(a, 5), at(a, 6), at(a, 7)],
                        ),
                    };
                    canvas.draw_image(source, src, dst);
                }
                Answer::Nothing
            }
            "createPattern" => match &element.source {
                Some(source) => {
                    let repetition = text
                        .and_then(canvas::Repetition::from_name)
                        .ok_or_else(bad_call)?;
                    let id = self.new_id()?;
                    let pattern = Pattern {
                        width: source.width,
                        height: source.height,
                        pixels: Arc::clone(&source.pixels),
                        repetition,
                        transform: Matrix::IDENTITY,
                        origin_clean: source.origin_clean,
                    };
                    self.patterns.insert(id, Arc::new(Mutex::new(pattern)));
                    number(f64::from(id))
                }
                // An image not yet decoded makes no pattern: null.
                None => Answer::MaybeString(None),
            },
            "fillText" | "strokeText" => {
                if let Some(fonts) = &element.fonts {
                    let max = (n.len() > 2).then(|| at(n, 2));
                    let stroke = call.op == "strokeText";
                    let text = text.unwrap_or_default();
                    canvas.draw_text(fonts, text, value, at(n, 1), max, stroke, element.rtl);
                }
                Answer::Nothing
            }
            "measureText" => {
                let m = match &element.fonts {
                    Some(fonts) => {
                        canvas.measure_text(fonts, text.unwrap_or_default(), element.rtl)
                    }
                    None => canvas::TextMetrics::default(),
                };
                Answer::Numbers(vec![
                    m.width,
                    m.actual_left,
                    m.actual_right,
                    m.actual_ascent,
                    m.actual_descent,
                    m.font_ascent,
                    m.font_descent,
                    m.alphabetic,
                    m.hanging,
                    m.ideographic,
                ])
            }
            "lineWidth" if set => {
                state.set_line_width(value);
                Answer::Nothing
            }
            "lineWidth" => number(state.line_width),
            "lineCap" if set => {
                if let Some(cap) = text.and_then(LineCap::from_name) {
                    state.line_cap = cap;
                }
                Answer::Nothing
            }
            "lineCap" => Answer::String(state.line_cap.name().to_owned()),
            "lineJoin" if set => {
                if let Some(join) = text.and_then(LineJoin::from_name) {
                    state.line_join = join;
                }
                Answer::Nothing
            }
            "lineJoin" => Answer::String(state.line_join.name().to_owned()),
            "miterLimit" if set => {
                state.set_miter_limit(value);
                Answer::Nothing
            }
            "miterLimit" => number(state.miter_limit),
            "lineDashOffset" if set => {
                state.set_dash_offset(value);
                Answer::Nothing
            }
            "lineDashOffset" => number(state.dash.offset),
            "setLineDash" => {
                state.dash.set(n);
                Answer::Nothing
            }
            "getLineDash" => Answer::Numbers(state.dash.list.clone()),
            "clearRect" | "fillRect" | "strokeRect" => {
                let (x, y, w, h) = (value, at(n, 1), at(n, 2), at(n, 3));
                match call.op.as_str() {
                    "clearRect" => canvas.clear_rect(x, y, w, h),
                    "fillRect" => canvas.fill_rect(x, y, w, h),
                    _ => canvas.stroke_rect(x, y, w, h),
                }
                Answer::Nothing
            }
            "beginPath" => {
                canvas.begin_path();
                Answer::Nothing
            }
            "fill" | "clip" => {
                let path = path_of(&self.paths, value)?;
                if call.op == "fill" {
                    canvas.fill(path.as_ref(), rule(at(n, 1)));
                } else {
                    canvas.clip(path.as_ref(), rule(at(n, 1)));
                }
                Answer::Nothing
            }
            "stroke" => {
                let path = path_of(&self.paths, value)?;
                canvas.stroke(path.as_ref());
                Answer::Nothing
            }
            "isPointInPath" => {
                let path = path_of(&self.paths, value)?;
                Answer::Bool(canvas.is_point_in_path(
                    path.as_ref(),
                    at(n, 1),
                    at(n, 2),
                    rule(at(n, 3)),
                ))
            }
            "isPointInStroke" => {
                let path = path_of(&self.paths, value)?;
                Answer::Bool(canvas.is_point_in_stroke(path.as_ref(), at(n, 1), at(n, 2)))
            }
            "getImageData" => {
                let [x, y, w, h] = [value, at(n, 1), at(n, 2), at(n, 3)].map(|v| v as i64);
                let bytes = canvas.get_image_data(x, y, w, h).map_err(refused)?;
                self.work += bytes.len() as u64 / 64;
                Answer::Bytes(bytes)
            }
            "putImageData" => {
                let data = call.bytes.as_deref().unwrap_or_default();
                let [w, h, dx, dy] = [value, at(n, 1), at(n, 2), at(n, 3)].map(|v| v as i64);
                let dirty = (n.len() >= 8).then(|| {
                    (
                        at(n, 4) as i64,
                        at(n, 5) as i64,
                        at(n, 6) as i64,
                        at(n, 7) as i64,
                    )
                });
                let (Ok(w), Ok(h)) = (u32::try_from(w), u32::try_from(h)) else {
                    return Err(bad_call());
                };
                self.work += data.len() as u64 / 64;
                canvas.put_image_data(data, w, h, dx, dy, dirty);
                Answer::Nothing
            }
            op => {
                let answer = canvas.with_path(|path, m| {
                    path.set_limit(Some(path.len() + room));
                    let answer = build(path, op, n, m);
                    path.set_limit(None);
                    answer
                });
                return answer
                    .ok_or_else(bad_call)?
                    .map(|()| Answer::Nothing)
                    .map_err(refused);
            }
        };
        Ok(answer)
    }
}

/// One of `CanvasPath`'s methods on `path`, its points through `m`:
/// `None` when `op` is none of them.
fn build(path: &mut Path, op: &str, n: &[f64], m: &Matrix) -> Option<Result<(), CanvasError>> {
    let a = |i: usize| at(n, i);
    Some(match op {
        "closePath" => {
            path.close();
            Ok(())
        }
        "moveTo" => {
            path.move_to(a(0), a(1), m);
            Ok(())
        }
        "lineTo" => {
            path.line_to(a(0), a(1), m);
            Ok(())
        }
        "quadraticCurveTo" => {
            path.quad_to(a(0), a(1), a(2), a(3), m);
            Ok(())
        }
        "bezierCurveTo" => {
            path.cubic_to(a(0), a(1), a(2), a(3), a(4), a(5), m);
            Ok(())
        }
        "arcTo" => path.arc_to(a(0), a(1), a(2), a(3), a(4), m),
        "rect" => {
            path.rect(a(0), a(1), a(2), a(3), m);
            Ok(())
        }
        "roundRect" => {
            let radii: Vec<(f64, f64)> = n
                .get(4..)
                .unwrap_or_default()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&[x, y]| (x, y))
                .collect();
            path.round_rect(a(0), a(1), a(2), a(3), &radii, m)
        }
        "arc" => path.arc(a(0), a(1), a(2), a(3), a(4), a(5) != 0.0, m),
        "ellipse" => path.ellipse(a(0), a(1), a(2), a(3), a(4), a(5), a(6), a(7) != 0.0, m),
        _ => return None,
    })
}
