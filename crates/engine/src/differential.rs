//! Differential checking (M13): what an engine shows after a sequence
//! of mutations against what a fresh engine shows for the document as
//! it now stands.
//!
//! An engine that mutates its document keeps what it computed for the
//! document before and reuses what the changes left valid: element
//! styles (M13.1), boxes' layouts (M13.2) and the last frame's pixels
//! (M13.3). A full pass over the same document is the answer it must
//! give. [`compare_with_fresh`] checks
//! that: the computed style of every element, every fragment and every
//! pixel. [`Mutator`] turns bytes into mutations, so that the engine's
//! tests drive it from seeded random bytes and the `mutation` fuzz
//! target from the fuzzer's; [`run_script`] lets a script make them, as
//! the `dom_api` target's do (M25).

use std::collections::HashMap;

use dom::{Document, NodeId};
use layout::{FragmentKind, Replaced};

use crate::Engine;

/// The tags a [`Mutator`] creates elements with: blocks, inlines, the
/// table parts, `style`, whose text is a stylesheet, the form elements
/// the form pseudo-classes read, and `meta`, whose pragma `:lang()`
/// reads.
const TAGS: [&str; 20] = [
    "div", "p", "span", "b", "ul", "li", "table", "tr", "td", "style", "a", "section", "input",
    "select", "option", "optgroup", "fieldset", "legend", "form", "meta",
];

/// The attributes a [`Mutator`] sets and removes, those selectors and
/// the cascade read and those table layout reads, each with the values
/// it is given: class names and ids a page's selectors can name, span
/// counts, directions, inline styles that reach every kind of box, with
/// transforms and opacities that change from one to another without
/// layout (M22.4), transitions and animations (M22), and what
/// `:lang()`, `:link` and the form pseudo-classes read. `class`
/// and `style`, the ones pages change most, come up most.
const ATTRIBUTES: [(&str, &[&str]); 23] = [
    ("class", CLASSES),
    ("class", CLASSES),
    ("style", STYLES),
    ("style", STYLES),
    ("id", &["x", "y", ""]),
    ("title", &["t", ""]),
    ("colspan", &["2", "0", "x"]),
    ("rowspan", &["2", "0"]),
    ("span", &["3"]),
    ("dir", &["rtl", "ltr"]),
    ("lang", &["fr", "de-CH", "", "en_US"]),
    ("href", &["#", ""]),
    ("type", &["radio", "checkbox", "text", "hidden"]),
    ("name", &["r", ""]),
    ("form", &["x", "y"]),
    ("checked", &[""]),
    ("selected", &[""]),
    ("disabled", &[""]),
    ("required", &[""]),
    ("multiple", &[""]),
    ("size", &["1", "2"]),
    ("http-equiv", &["content-language"]),
    ("content", &["fr", "de, fr", ""]),
];

const CLASSES: &[&str] = &["a", "b", "c", "a b", "c a", ""];

const STYLES: &[&str] = &[
    "",
    "display: none",
    "float: left; width: 30px; height: 10px",
    "float: right; width: 20px; height: 25px; clear: both",
    "position: absolute; top: 4px; left: 8px; width: 10px; height: 10px",
    "position: relative; top: 3px; z-index: 2",
    "position: sticky; top: 0",
    "overflow: auto; height: 20px",
    "display: inline-block; width: 12px; height: 12px",
    "display: table-cell; padding: 1px",
    "display: table; border-collapse: collapse",
    "width: 50%; padding: 2px",
    "margin: 6px; border: 2px solid #00f",
    "font-size: 20px; color: #f00",
    "background-color: #0f0; min-height: 5px",
    "color: inherit; padding: inherit",
    "rotate: 30deg; opacity: 0.5",
    "opacity: 0.7",
    "transform: translate(5px, 40px) scale(2)",
    "translate: 0 -8px; opacity: 0.2",
    "transition: all 1s; width: 40px; color: #0a0",
    "transition: margin 2s 1s, display 1s allow-discrete; margin-left: 9px",
    "animation: m 2s -0.5s infinite alternate",
    "animation: m 1s paused -0.25s, n 3s steps(3) backwards 1s",
];

/// The data a [`Mutator`] gives text nodes: empty, white space, words,
/// right-to-left letters, and stylesheets for a `style` element, two of
/// them with keyframes (M22).
const TEXTS: [&str; 19] = [
    "",
    " ",
    "x",
    "hello world",
    "a b c d e f",
    "\u{5d0}\u{5d1} \u{5d2}",
    "p { color: #00f }",
    ".a { float: right; width: 20px; height: 20px; background-color: #f0f }",
    "div { padding: 3px } .b { display: none }",
    "td { border: 1px solid #000 } .c { position: relative; left: 5px }",
    "li:first-child { font-size: 30px } span + b { display: block }",
    ":lang(fr) { color: #f00 } :lang('') { padding: 2px } :link { margin: 1px }",
    ":checked { display: block; width: 7px } :disabled + span { display: none }",
    "p::before { content: attr(title) } .a::after { content: 'x'; color: #f00 }",
    ".b::before { content: open-quote; display: block } q::after { display: table-cell }",
    "p::first-letter { float: left; font-size: 20px } div::first-letter { color: #f00 }",
    "div::first-line { font-size: 24px } p::first-line { background-color: #ff0; color: #00f }",
    "@keyframes m { to { width: 30px; rotate: 9deg; opacity: 0.3 } } .b { animation: m 2s -1s }",
    "@keyframes n { 50% { margin-top: 7px; display: none } } div { transition: padding 1s }",
];

/// Turns bytes into mutations of a document: each step reads an
/// operation and its operands from the front of a script and applies
/// it through the document's mutation API — create an element or a
/// text node and append it, append or insert a node where it is not,
/// remove one, set or remove an attribute, set a text node's data, or
/// free a node. Nodes are picked by a byte
/// among every node the mutator has known — those in the tree when it
/// was made and those it created since, removed and freed ones
/// included, so that the document's refusals are exercised too.
#[derive(Debug, Clone)]
pub struct Mutator {
    nodes: Vec<NodeId>,
}

impl Mutator {
    /// A mutator knowing every node in `document`'s tree.
    pub fn new(document: &Document) -> Self {
        Mutator {
            nodes: document.subtree(document.root()).collect(),
        }
    }

    /// Applies the mutation at the front of `script`, consuming its
    /// bytes, and returns whether there was one: `false` once the
    /// script is used up. A mutation the document refuses changes
    /// nothing, as the document promises.
    pub fn step(&mut self, document: &mut Document, script: &mut &[u8]) -> bool {
        let Some((&op, rest)) = script.split_first() else {
            return false;
        };
        *script = rest;
        let mut byte = || match script.split_first() {
            Some((&b, rest)) => {
                *script = rest;
                b as usize
            }
            None => 0,
        };
        match op % 9 {
            // A new node goes under a picked one at once, when that one
            // takes it: a node created and left aside changes nothing.
            0 => {
                if let Ok(id) = document.create_element(TAGS[byte() % TAGS.len()]) {
                    self.nodes.push(id);
                    let _ = document.append_child(self.pick(byte()), id);
                }
            }
            1 => {
                if let Ok(id) = document.create_text(TEXTS[byte() % TEXTS.len()]) {
                    self.nodes.push(id);
                    let _ = document.append_child(self.pick(byte()), id);
                }
            }
            2 => {
                let (parent, child) = (self.pick(byte()), self.pick(byte()));
                let _ = document.append_child(parent, child);
            }
            3 => {
                let (parent, child) = (self.pick(byte()), self.pick(byte()));
                let reference = match byte() {
                    b if b % 2 == 0 => None,
                    b => Some(self.pick(b / 2)),
                };
                let _ = document.insert_before(parent, child, reference);
            }
            4 => {
                let child = self.pick(byte());
                if let Some(parent) = document.parent(child) {
                    let _ = document.remove_child(parent, child);
                }
            }
            5 => {
                let element = self.pick(byte());
                let (name, values) = ATTRIBUTES[byte() % ATTRIBUTES.len()];
                let value = values[byte() % values.len()];
                let _ = document.set_attribute(element, name, value);
            }
            6 => {
                let element = self.pick(byte());
                let (name, _) = ATTRIBUTES[byte() % ATTRIBUTES.len()];
                let _ = document.remove_attribute(element, name);
            }
            7 => {
                let node = self.pick(byte());
                let _ = document.set_text(node, TEXTS[byte() % TEXTS.len()]);
            }
            _ => {
                let _ = document.free(self.pick(byte()));
            }
        }
        true
    }

    /// The node `byte` picks among those known.
    fn pick(&self, byte: usize) -> NodeId {
        self.nodes[byte % self.nodes.len()]
    }
}

/// How many frames `engine`'s tab has, and how deep they nest (M33.2):
/// what the frames target holds to the limits.
pub fn frames_of(engine: &Engine) -> (usize, usize) {
    let tree = crate::frames::lock(&engine.frame.tree);
    (tree.frames(), tree.depth())
}

/// Runs `source` in the document `engine` shows as a classic script, as
/// a `javascript:` URL's runs (M24), the page's policy aside, and takes
/// what it changed for the next frame: a script's changes after a frame
/// has been drawn, which the events of M26 will run.
pub fn run_script(engine: &mut Engine, source: &str) {
    engine.run_classic(source);
}

/// Takes the result of every fetch `engine` has in flight, waiting for
/// those still on their way (M27): what a script's `fetch()` or XHR
/// asked of a `data:` URL then waits as a task, due when the engine's
/// clock says it came, so that a target's run does not depend on how
/// fast the network's threads were. It yields between looks rather
/// than sleeping, which on Windows is never shorter than a tick of the
/// system's timer, some 15 ms.
pub fn settle_network(engine: &mut Engine) {
    loop {
        engine.poll_network();
        if engine.network.in_flight() == 0 {
            return;
        }
        std::thread::yield_now();
    }
}

/// Draws a `width` by `height` frame with `engine` and compares what it
/// keeps with what `fresh` computes for a snapshot of its document,
/// scrolled as `engine` is: the computed style of every element, every
/// fragment of the layout, and every pixel of the frame. The snapshot
/// is given the state no markup holds (M26): each element's state
/// flags and control record, the focus, whether the window has it, and
/// a drop-down's open list. `engine`'s
/// transitions are finished first, as a script's `finish()` would
/// finish them, those of the changes no frame has drawn yet with them,
/// since `fresh` starts none (M22); its animations, drawn
/// without a time as `fresh`'s are, show where `fresh`'s do. `fresh` is
/// any engine with the same fonts; its document is replaced. `Err`
/// describes the first difference found.
pub fn compare_with_fresh(
    engine: &mut Engine,
    fresh: &mut Engine,
    width: u32,
    height: u32,
) -> Result<(), String> {
    // A frame first, so that the changes no frame has drawn yet start
    // their transitions, and they are finished too.
    engine.frame(width, height);
    engine.finish_transitions();
    settle_frames(engine);
    let pixels = engine.frame(width, height);
    fresh.set_document(Document::from_node(&engine.shown.document.to_node()));
    let order = tree_order(&engine.shown.document);
    let fresh_order = tree_order(&fresh.shown.document);
    if order.len() != fresh_order.len() {
        return Err(format!(
            "the snapshot has {} nodes, the document {}",
            fresh_order.len(),
            order.len()
        ));
    }
    give_state(engine, fresh, &order, &fresh_order);
    fresh.frame(width, height);
    settle_frames(fresh);
    // The same offsets: the document's, and each scroll container's,
    // node for node in tree order.
    fresh.set_scroll(engine.scroll_y());
    for (&node, &fresh_node) in order.iter().zip(&fresh_order) {
        let offset = engine.element_scroll(node);
        if offset != 0.0 {
            fresh.set_element_scroll(fresh_node, offset);
        }
    }
    // Rasterized whole, not from the frame before: the engine's raster
    // keeps its last frame (M13.3), and the fresh one's must not.
    fresh.backend.forget_frame();
    let fresh_pixels = fresh.frame(width, height);

    engine.cached_styles();
    fresh.cached_styles();
    let styles = &engine.shown.styled.as_ref().expect("cascaded").styles;
    let fresh_styles = &fresh.shown.styled.as_ref().expect("cascaded").styles;
    for (i, (&node, &fresh_node)) in order.iter().zip(&fresh_order).enumerate() {
        if engine.shown.document.data(node) != fresh.shown.document.data(fresh_node) {
            return Err(format!("node {i} differs from the snapshot's"));
        }
        if styles.get(node) != fresh_styles.get(fresh_node) {
            return Err(format!(
                "node {i} ({:?}): computed style {:?}, a full cascade gives {:?}",
                engine.shown.document.tag(node),
                styles.get(node),
                fresh_styles.get(fresh_node)
            ));
        }
        if styles.pseudo_styles(node) != fresh_styles.pseudo_styles(fresh_node) {
            return Err(format!(
                "node {i} ({:?}): pseudo-element styles {:?}, a full cascade gives {:?}",
                engine.shown.document.tag(node),
                styles.pseudo_styles(node),
                fresh_styles.pseudo_styles(fresh_node)
            ));
        }
    }

    let fragments = fragment_lines(engine, width, &order)?;
    let fresh_fragments = fragment_lines(fresh, width, &fresh_order)?;
    if let Some(at) = (0..fragments.len().max(fresh_fragments.len()))
        .find(|&k| fragments.get(k) != fresh_fragments.get(k))
    {
        return Err(format!(
            "fragment {at}: {:?}, a full layout gives {:?}",
            fragments.get(at),
            fresh_fragments.get(at)
        ));
    }

    if pixels != fresh_pixels {
        let at = pixels
            .chunks(4)
            .zip(fresh_pixels.chunks(4))
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        let (x, y) = (at % width as usize, at / width as usize);
        return Err(format!(
            "pixel ({x}, {y}): {:?}, a full pipeline draws {:?}",
            pixels.chunks(4).nth(at),
            fresh_pixels.chunks(4).nth(at)
        ));
    }
    Ok(())
}

/// Polls `engine`'s network until its frames' documents have loaded, a
/// hundred times at most (M33): a `srcdoc` loads at a poll, and the
/// fresh engine's frames load again what the engine's did.
fn settle_frames(engine: &mut Engine) {
    for _ in 0..100 {
        if engine.frames_settled() {
            return;
        }
        engine.poll_network();
    }
}

/// Every node of `document`'s tree, in tree order.
fn tree_order(document: &Document) -> Vec<NodeId> {
    document.subtree(document.root()).collect()
}

/// Gives `fresh`'s snapshot of `engine`'s document, node for node in
/// tree order, the state no markup holds (M26): each element's state
/// flags and control record, the focus, whether the window has it, and
/// a drop-down's open list. Before its first frame, which cascades it
/// whole.
fn give_state(engine: &Engine, fresh: &mut Engine, order: &[NodeId], fresh_order: &[NodeId]) {
    let places: HashMap<NodeId, NodeId> = order
        .iter()
        .copied()
        .zip(fresh_order.iter().copied())
        .collect();
    let document = &engine.shown.document;
    let snapshot = &mut fresh.shown.document;
    snapshot.set_focus(
        document
            .focused()
            .and_then(|focused| places.get(&focused).copied()),
        false,
    );
    for (&node, &fresh_node) in order.iter().zip(fresh_order) {
        // A text node's state is refused.
        let _ = snapshot.set_state(fresh_node, document.state(node));
        if let Some(control) = document.control(node) {
            let _ = snapshot.set_control(fresh_node, control.clone());
        }
    }
    // A canvas's bitmap, which no markup holds, is the engine's (M33:
    // found by dom_api, a canvas drawn in view).
    for (&node, &fresh_node) in order.iter().zip(fresh_order) {
        if let Some(image) = engine.shown.scripts.canvas_image(node) {
            fresh.shown.scripts.show_canvas(fresh_node, image);
        }
    }
    fresh.window_focused = engine.window_focused;
    fresh.shown.popup = engine
        .shown
        .popup
        .as_ref()
        .and_then(|popup| popup.mapped(&places));
}

/// `engine`'s layout at `width` as one line per fragment in tree order,
/// a fragment's node named by its place in `order`: what two layouts
/// of the same tree compare by. The layout's height and whether it
/// depends on the viewport's lead.
fn fragment_lines(
    engine: &mut Engine,
    width: u32,
    order: &[NodeId],
) -> Result<Vec<String>, String> {
    let places: HashMap<NodeId, usize> = order.iter().enumerate().map(|(i, &n)| (n, i)).collect();
    let layout = engine.layout(width as f32);
    let mut lines = vec![format!(
        "height {} depends on the viewport's height {}",
        layout.height, layout.depends_on_viewport_height
    )];
    for placed in layout.placed() {
        let f = placed.fragment;
        let Some(number) = places.get(&f.node) else {
            return Err(format!(
                "a fragment of {}, which is not in the tree",
                f.node
            ));
        };
        // A canvas's bitmap, an outer svg's drawing, a frame and a video
        // name their element, by its place too: the two engines number
        // their nodes as they made them (M31, M32, M33, M40).
        let kind = match &f.kind {
            FragmentKind::Replaced(Replaced::Canvas {
                node,
                width,
                height,
            }) => format!(
                "Replaced(Canvas of node {:?} {width} by {height})",
                places.get(node)
            ),
            FragmentKind::Replaced(Replaced::Svg { node, ratio }) => format!(
                "Replaced(Svg of node {:?} ratio {ratio:?})",
                places.get(node)
            ),
            FragmentKind::Replaced(Replaced::Frame { node }) => {
                format!("Replaced(Frame of node {:?})", places.get(node))
            }
            FragmentKind::Replaced(Replaced::Video { node, video }) => {
                format!("Replaced(Video of node {:?} {video:?})", places.get(node))
            }
            kind => format!("{kind:?}"),
        };
        lines.push(format!(
            "node {number} {kind} at ({}, {}) {} by {} ascent {} scroll height {} style {:?}",
            placed.x, placed.y, f.w, f.h, f.ascent, f.scroll_height, f.style
        ));
    }
    Ok(lines)
}
