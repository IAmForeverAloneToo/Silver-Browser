//! Text v1 (SVG 2 section 11): a `<text>`'s characters, `<tspan>`s and
//! `<a>`s among them, white space collapsed or kept by `white-space`,
//! placed from the `x`, `y`, `dx` and `dy` lists, shaped in the engine's
//! face at each one's `font-size`, moved by `text-anchor` chunk by chunk
//! and set on their `dominant-baseline`, as paths a shape fills and
//! strokes. `textPath`, `rotate`, `textLength`, vertical and
//! bidirectional text are G87's.

use std::borrow::Cow;
use std::sync::Arc;

use canvas::{Matrix, Path};
use dom::NodeId;
use style::computed::{
    ComputedStyle, Display, DominantBaseline, TextAnchor, TextTransform, WhiteSpace,
};
use text::FontCollection;

use crate::syntax::{Axis, Viewport, parse_lengths};
use crate::tree::{Builder, Glyphs, Outline};

/// One run of a text's characters laid out: an element's, shaped
/// together, its glyphs' outlines in `path` in the text's user units.
pub(crate) struct Run<'a> {
    /// The element whose characters they are.
    pub(crate) node: NodeId,
    /// Its style.
    pub(crate) style: Cow<'a, ComputedStyle>,
    /// The outlines.
    pub(crate) path: Path,
    /// The run's box: its advance by the font's ascent and descent.
    pub(crate) bounds: [f64; 4],
}

/// An element a text's characters are in, and where they are.
struct Owner<'a> {
    node: NodeId,
    style: Cow<'a, ComputedStyle>,
    /// Its characters, by their index among all of the text's.
    start: usize,
    end: usize,
}

/// A run being laid: its element, where it starts, its characters.
struct Pending {
    owner: usize,
    x: f64,
    y: f64,
    text: String,
}

impl<'a> Builder<'a> {
    /// The runs of the `<text>` element `node` of `style`: `None` for no
    /// fonts or no character to draw.
    pub(crate) fn text_runs(
        &mut self,
        node: NodeId,
        style: Cow<'a, ComputedStyle>,
        viewport: &Viewport,
        inherit: bool,
    ) -> Option<Vec<Run<'a>>> {
        let fonts = self.fonts?;
        let mut owners = Vec::new();
        let mut chars = Vec::new();
        self.collect(node, style, inherit, &mut owners, &mut chars);
        let (chars, owners) = collapse(chars, owners);
        if chars.is_empty() {
            return None;
        }
        // Each character's place in the lists, the innermost element's
        // own list winning: the owners are in tree order.
        let n = chars.len();
        let mut lists: [Vec<Option<f64>>; 4] = std::array::from_fn(|_| vec![None; n]);
        for owner in &owners {
            let viewport = &Viewport {
                font_size: f64::from(owner.style.font_size_px),
                ..*viewport
            };
            for (k, (name, axis)) in [
                ("x", Axis::X),
                ("y", Axis::Y),
                ("dx", Axis::X),
                ("dy", Axis::Y),
            ]
            .into_iter()
            .enumerate()
            {
                let Some(values) = self.document.attr(owner.node, name) else {
                    continue;
                };
                for (i, length) in parse_lengths(values).into_iter().enumerate() {
                    if owner.start + i >= owner.end {
                        break;
                    }
                    lists[k][owner.start + i] = Some(length.resolve(axis, viewport));
                }
            }
        }
        let [xs, ys, dxs, dys] = &lists;
        let mut runs: Vec<(Pending, f64)> = Vec::new();
        // Each chunk's first run, and its anchor.
        let mut chunks: Vec<(usize, TextAnchor)> = Vec::new();
        let mut pen = (0.0f64, 0.0f64);
        let mut pending: Option<Pending> = None;
        for (i, &(c, owner)) in chars.iter().enumerate() {
            let style = &owners[owner].style;
            let spaced = style.letter_spacing != 0.0 || style.word_spacing != 0.0;
            let placed = xs[i].is_some() || ys[i].is_some();
            let moved = placed || dxs[i].is_some() || dys[i].is_some();
            let breaks = pending
                .as_ref()
                .is_none_or(|p| p.owner != owner || moved || spaced);
            if breaks {
                if let Some(done) = pending.take() {
                    let advance = advance_of(fonts, &done, &owners);
                    pen = (done.x + advance, done.y);
                    runs.push((done, advance));
                }
                if i == 0 || placed {
                    chunks.push((runs.len(), style.svg().text_anchor));
                }
                pen.0 = xs[i].unwrap_or(pen.0) + dxs[i].unwrap_or(0.0);
                pen.1 = ys[i].unwrap_or(pen.1) + dys[i].unwrap_or(0.0);
                pending = Some(Pending {
                    owner,
                    x: pen.0,
                    y: pen.1,
                    text: String::new(),
                });
            }
            if let Some(p) = pending.as_mut() {
                p.text.push(c);
            }
        }
        if let Some(done) = pending.take() {
            let advance = advance_of(fonts, &done, &owners);
            runs.push((done, advance));
        }
        // Each chunk moved by its anchor: its runs' extent, from its
        // first run's start to the farthest end.
        let mut shift = vec![0.0; runs.len()];
        for (k, &(first, anchor)) in chunks.iter().enumerate() {
            let last = chunks.get(k + 1).map_or(runs.len(), |next| next.0);
            if first >= last {
                continue;
            }
            let start = runs[first..last]
                .iter()
                .map(|(r, _)| r.x)
                .fold(f64::INFINITY, f64::min);
            let end = runs[first..last]
                .iter()
                .map(|(r, a)| r.x + a)
                .fold(f64::NEG_INFINITY, f64::max);
            let by = match anchor {
                TextAnchor::Start => 0.0,
                TextAnchor::Middle => -(end - start) / 2.0,
                TextAnchor::End => -(end - start),
            };
            shift[first..last].fill(by);
        }
        let mut out = Vec::new();
        for ((run, advance), by) in runs.into_iter().zip(shift) {
            let owner = &owners[run.owner];
            let size = owner.style.font_size_px;
            let metrics = fonts.primary().metrics(size);
            let (ascent, descent) = (f64::from(metrics.ascent), f64::from(metrics.descent));
            let baseline = run.y
                + match owner.style.svg().dominant_baseline {
                    DominantBaseline::Auto | DominantBaseline::Alphabetic => 0.0,
                    DominantBaseline::Middle => f64::from(fonts.x_height(size)) / 2.0,
                    DominantBaseline::Central => (ascent - descent) / 2.0,
                    DominantBaseline::Hanging => 0.8 * ascent,
                    DominantBaseline::Mathematical => 0.5 * ascent,
                    DominantBaseline::TextTop => ascent,
                    DominantBaseline::TextBottom | DominantBaseline::Ideographic => -descent,
                };
            let x = run.x + by;
            let mut path = Path::new();
            for &(script, glyph) in self.shaped(fonts, &run.text, size).iter() {
                let Some(curves) = self.outline(fonts, script, glyph.glyph_id, size) else {
                    continue;
                };
                let at = Matrix::new(
                    1.0,
                    0.0,
                    0.0,
                    1.0,
                    x + f64::from(glyph.x),
                    baseline + f64::from(glyph.y),
                );
                path.add_outline(&curves, &at);
            }
            out.push(Run {
                node: owner.node,
                style: owner.style.clone(),
                path,
                bounds: [x, baseline - ascent, advance, ascent + descent],
            });
        }
        Some(out)
    }

    /// `text`'s glyphs at `size`, shaped once.
    fn shaped(&mut self, fonts: &FontCollection, text: &str, size: f32) -> Glyphs {
        let key = (text.to_owned(), size.to_bits());
        if let Some(glyphs) = self.glyphs.get(&key) {
            return Arc::clone(glyphs);
        }
        let glyphs: Glyphs = fonts.shape(text, size).into();
        self.glyphs.insert(key, Arc::clone(&glyphs));
        glyphs
    }

    /// The outline of the glyph `id` of `script`'s font at `size`, made
    /// once; `None` for one with none.
    fn outline(
        &mut self,
        fonts: &FontCollection,
        script: text::Script,
        id: u16,
        size: f32,
    ) -> Outline {
        self.outlines
            .entry((script, id, size.to_bits()))
            .or_insert_with(|| {
                let font = fonts.font_for(script).glyph_font();
                text::glyph_outline(font, id, size).map(Arc::from)
            })
            .clone()
    }

    /// The characters of `node` and of the `tspan`s and `a`s in it, each
    /// with the index of its element among `owners`.
    fn collect(
        &mut self,
        node: NodeId,
        style: Cow<'a, ComputedStyle>,
        inherit: bool,
        owners: &mut Vec<Owner<'a>>,
        chars: &mut Vec<(char, usize)>,
    ) {
        let me = owners.len();
        owners.push(Owner {
            node,
            style,
            start: chars.len(),
            end: chars.len(),
        });
        let document = self.document;
        for child in document.children(node) {
            if let Some(text) = document.text(child) {
                // `text-transform` (as HTML's text has it): a word starts
                // after white space, or at the text's start.
                let mut word_start = chars.last().is_none_or(|(c, _)| c.is_whitespace());
                for c in text.chars() {
                    match owners[me].style.text_transform {
                        TextTransform::Uppercase => chars.extend(c.to_uppercase().map(|c| (c, me))),
                        TextTransform::Lowercase => chars.extend(c.to_lowercase().map(|c| (c, me))),
                        TextTransform::Capitalize if word_start && !c.is_whitespace() => {
                            chars.extend(c.to_uppercase().map(|c| (c, me)));
                        }
                        _ => chars.push((c, me)),
                    }
                    word_start = c.is_whitespace();
                }
                continue;
            }
            if !matches!(document.svg_tag(child), Some("tspan" | "a")) || !self.passes(child) {
                continue;
            }
            let parent = inherit.then(|| &*owners[me].style);
            let Some(style) = self.style_of(child, parent) else {
                continue;
            };
            if style.display == Display::None {
                continue;
            }
            self.collect(child, style, inherit, owners, chars);
        }
        owners[me].end = chars.len();
    }
}

/// The characters with white space collapsed (SVG 2 section 11.4, as the
/// reference browser does): a run of white space one space, none at the
/// text's ends, where `white-space` collapses; kept, a newline or a tab
/// a space, where it is `pre` or `pre-wrap`. The owners' ranges are of
/// what is kept.
fn collapse<'a>(
    chars: Vec<(char, usize)>,
    mut owners: Vec<Owner<'a>>,
) -> (Vec<(char, usize)>, Vec<Owner<'a>>) {
    let keeps = |owner: usize, owners: &[Owner<'_>]| {
        matches!(
            owners[owner].style.white_space,
            WhiteSpace::Pre | WhiteSpace::PreWrap
        )
    };
    let mut kept = Vec::with_capacity(chars.len());
    // For each character as written, how many were kept before it.
    let mut before = Vec::with_capacity(chars.len() + 1);
    for &(c, owner) in &chars {
        before.push(kept.len());
        let space = matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c');
        if !space {
            kept.push((c, owner));
            continue;
        }
        if keeps(owner, &owners) {
            kept.push((' ', owner));
            continue;
        }
        let after_space = kept
            .last()
            .is_none_or(|&(last, o): &(char, usize)| last == ' ' && !keeps(o, &owners));
        if !after_space {
            kept.push((' ', owner));
        }
    }
    before.push(kept.len());
    // A collapsible space at the end goes.
    if let Some(&(' ', owner)) = kept.last()
        && !keeps(owner, &owners)
    {
        kept.pop();
    }
    let total = kept.len();
    for owner in &mut owners {
        owner.start = before[owner.start].min(total);
        owner.end = before[owner.end].min(total);
    }
    (kept, owners)
}

/// A run's advance: its shaped width, and its spacing after each
/// character.
fn advance_of(fonts: &FontCollection, run: &Pending, owners: &[Owner<'_>]) -> f64 {
    let style = &owners[run.owner].style;
    let mut advance = f64::from(fonts.measure(&run.text, style.font_size_px));
    for c in run.text.chars() {
        advance += f64::from(style.letter_spacing);
        if c == ' ' {
            advance += f64::from(style.word_spacing);
        }
    }
    advance
}
