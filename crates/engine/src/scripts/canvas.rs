//! The document's canvases (M30, `docs/design/canvas.md`): each
//! `canvas` element's 2D context, kept by its node while the node
//! lives, and the objects scripts make for them, `Path2D`s and
//! `CanvasGradient`s, by number until the bindings let them go. The
//! bindings' calls of them, `Call::Canvas`, are answered here.
//!
//! A document's canvases are bounded together, past each canvas's own
//! bounds: their bitmaps by [`MAX_CANVAS_MEMORY`], the objects scripts
//! hold by [`MAX_CANVAS_OBJECTS`], and the points of every path and the
//! stops of every gradient by [`MAX_CANVAS_POINTS`] and
//! [`MAX_CANVAS_STOPS`], so that a loop making them ends in a refusal
//! or in nothing more drawn, not in the engine's memory spent.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use canvas::{
    Canvas, CanvasError, Caps, Composite, Direction, FillRule, FontBase, Gradient, GradientKind,
    Kerning, LineCap, LineJoin, Matrix, Paint, Path, Pattern, Quality, Rendering, Rgba, Spacing,
    Stretch, TextAlign, TextBaseline, parse_color, parse_font, serialize_color,
};
use dom::NodeId;
use js::{Answer, CanvasCall, DomError};

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

/// The kinds of object a call names, as the bindings number them.
const CONTEXT: u8 = 0;
const PATH: u8 = 2;
const GRADIENT: u8 = 3;
const PATTERN: u8 = 4;

/// What the host knows of the call that the canvases do not: the
/// element a context's call is made on and its size by its attributes,
/// its computed `color`, which `currentcolor` is, and for a call of
/// text what a font's relative sizes are of, whether the element's
/// direction is right to left, and the window's fonts.
pub(crate) struct Element {
    pub(crate) node: NodeId,
    pub(crate) size: (u32, u32),
    pub(crate) color: Rgba,
    pub(crate) font_base: FontBase,
    pub(crate) rtl: bool,
    pub(crate) fonts: Option<text::FontCollection>,
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
    contexts: HashMap<NodeId, Canvas>,
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
    snapshots: HashMap<NodeId, (u64, Arc<images::Image>)>,
    /// The work done since it was last taken.
    work: u64,
}

/// Whether a call of `op` names a context, its first number its kind:
/// those that make objects, or work on a gradient, name none.
pub(crate) fn names_context(op: &str) -> bool {
    !matches!(
        op,
        "path" | "linearGradient" | "radialGradient" | "conicGradient" | "release" | "addColorStop"
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
        let canvas = self.contexts.get(&node)?;
        match self.snapshots.get(&node) {
            Some((taken, image)) if *taken == canvas.generation() => Some(Arc::clone(image)),
            _ => canvas.snapshot(),
        }
    }

    /// The bitmap of `node`'s canvas as an image, and its generation: a
    /// copy taken when the generation moved, the same until it moves
    /// again.
    pub(crate) fn pixels(&mut self, node: NodeId) -> Option<(u64, Arc<images::Image>)> {
        let canvas = self.contexts.get(&node)?;
        let generation = canvas.generation();
        if let Some((taken, image)) = self.snapshots.get(&node)
            && *taken == generation
        {
            return Some((generation, Arc::clone(image)));
        }
        let image = canvas.snapshot()?;
        self.snapshots
            .insert(node, (generation, Arc::clone(&image)));
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

    /// The bytes the bitmaps but `node`'s hold.
    fn others(&self, node: NodeId) -> u64 {
        self.contexts
            .iter()
            .filter(|(n, _)| **n != node)
            .map(|(_, canvas)| bytes_of(canvas))
            .sum()
    }

    /// The canvas `node`'s `width` or `height` attribute was set or
    /// removed: its bitmap made again at its size, cleared, and its
    /// context's state reset, as setting either does.
    pub(crate) fn resized(&mut self, node: NodeId, size: (u32, u32)) {
        let others = self.others(node);
        if let Some(canvas) = self.contexts.get_mut(&node) {
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
        self.contexts.retain(|&node, canvas| {
            let keep = alive(node);
            if !keep {
                *points -= canvas.path().len();
            }
            keep
        });
        let contexts = &self.contexts;
        self.snapshots.retain(|node, _| contexts.contains_key(node));
    }

    /// The points the document's budget has room for yet.
    fn room(&self) -> usize {
        MAX_CANVAS_POINTS.saturating_sub(self.points)
    }

    /// A number for a new object, past [`MAX_CANVAS_OBJECTS`] none.
    fn new_id(&mut self) -> Result<u32, DomError> {
        if self.paths.len() + self.gradients.len() + self.patterns.len() >= MAX_CANVAS_OBJECTS {
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

    /// The paint a style setter's arguments name: a colour's text, or a
    /// gradient or pattern by its kind and number. `None` changes
    /// nothing, as a colour that does not parse does not.
    fn paint_of(&self, call: &CanvasCall, current: Rgba) -> Option<Paint> {
        if let Some(text) = call.strings.first() {
            return parse_color(text, current).map(Paint::Color);
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
        if call.op == "context" {
            if !self.contexts.contains_key(&element.node) {
                let canvas =
                    Canvases::made(element.size, at(n, 2) == 0.0, self.others(element.node));
                self.contexts.insert(element.node, canvas);
            }
            return Ok(Answer::Nothing);
        }
        let room = self.room();
        let mut canvas = self.contexts.remove(&element.node).ok_or_else(bad_call)?;
        let before = canvas.path().len();
        let generation = canvas.generation();
        let answer = self.context_call(&mut canvas, &call, &element, room);
        self.points = self.points - before + canvas.path().len();
        self.changed |= canvas.generation() != generation;
        self.work += canvas.take_work();
        self.contexts.insert(element.node, canvas);
        answer
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
