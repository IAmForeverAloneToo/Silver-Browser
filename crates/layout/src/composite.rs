//! Composited changes (M22.4): a change of an element's style in its
//! transform or its opacity alone, which leaves its box what it was to
//! layout, written into the fragments that carry the element's effects —
//! in the layout shown and in the layout cache's entries — rather than
//! laid out again. A restyle's changes are written together, in one walk
//! over the fragments, each found by the element and pseudo-element it
//! is of: a frame of a thousand animations walks them once. The lists of
//! children are shared between the layout and the cache's entries
//! (M35.9): the walk copies those on the way to a change, once each, and
//! leaves the rest shared.
//!
//! The fragments a change reaches are the element's own boxes, or its
//! pseudo-element's: those of its node and pseudo-element whose style
//! carries the element's effects, the very ones, shared by the style the
//! change replaces; an anonymous box's style has none of the element's.
//! A transform is part of the scrollable overflow (M19), so a box whose
//! transform changed is measured again, before the change and after it,
//! where it reached, or now reaches, the bottom of the scroll container
//! it is in, or of the document: there the overflow can have changed,
//! and the caller lays the document out again, from the cache, the
//! container's entry forgotten. Anywhere else the overflow is as it was.
//! A box inside a transformed box, in the same scroll container, counts
//! toward the overflow through that box's projection: the outermost such
//! box around it is measured instead.
//!
//! A change of an element's style in what painting reads alone (M35.9,
//! [`repaints_only`]), a colour, a cursor, an outline, a block's
//! background, is written the same way: its fragments take the new
//! style. Only when every fragment of the element carries its style, the
//! one the change replaces: one carrying another, an anonymous box's or
//! a first line's, made from it at layout, would keep the old; and when
//! an element with no fragment would make none with its new style. Any
//! other change is laid out.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use dom::NodeId;
use style::ComputedStyle;
use style::computed::{Content, Display, Effects, Position, TOP};
use style::selector_matching::PseudoElement;

use crate::block::{DocumentLayout, Fragment, FragmentKind, extent_bottom};
use crate::children::Children;

/// How far from a scroll container's bottom edge a box's transformed
/// extent counts as reaching it: what the two ways of measuring the
/// extent, in layout and here, can differ by.
const EDGE: f32 = 0.5;

/// A composited change of one element's, or pseudo-element's, effects.
#[derive(Debug, Clone, Copy)]
pub struct EffectsChange<'a> {
    pub node: NodeId,
    pub pseudo: Option<PseudoElement>,
    /// The effects its fragments carry, which the change replaces.
    pub old: &'a Arc<Effects>,
    /// Its new effects.
    pub new: &'a Arc<Effects>,
    /// Whether its transform changed, which can change the scrollable
    /// overflow its boxes are part of.
    pub transformed: bool,
    /// The style its fragments carry and the new one, when known, which
    /// differ in these effects alone: a fragment that carries the very
    /// style takes the new one, shared, rather than a copy of its own
    /// with the new effects (T4.3, M35), which costs a style's copy a
    /// fragment a frame where the lists are shared with the cache.
    pub styles: Option<(&'a Arc<ComputedStyle>, &'a Arc<ComputedStyle>)>,
}

/// A change of an element's, or pseudo-element's, computed style in
/// what painting reads alone (M35.9, [`repaints_only`]): its fragments
/// take the new style rather than being laid out again.
#[derive(Debug, Clone, Copy)]
pub struct StyleChange<'a> {
    pub node: NodeId,
    pub pseudo: Option<PseudoElement>,
    /// Its new style, the cascade's.
    pub new: &'a Arc<ComputedStyle>,
}

/// Whether `new` differs from `old`, an element's or pseudo-element's
/// style before a restyle, only in what painting reads and layout does
/// not (M35.9): the colour, the cursor, the outline and the custom
/// properties; and the background and the border colours (a border's
/// `currentcolor` the colour) but of a table or a table part's box,
/// whose layout reads them (a row's background, a collapsed border's
/// colour). A box that draws a text decoration takes its colour at
/// layout.
pub fn repaints_only(old: &ComputedStyle, new: &ComputedStyle) -> bool {
    if old.color != new.color && (old.decoration().line.draws() || new.decoration().line.draws()) {
        return false;
    }
    let mut probe = old.clone();
    probe.color = new.color;
    probe.cursor = new.cursor;
    probe.outline = new.outline.clone();
    probe.custom = new.custom.clone();
    let laid_from_colours = matches!(
        old.display,
        Display::Table
            | Display::InlineTable
            | Display::TableRowGroup
            | Display::TableHeaderGroup
            | Display::TableFooterGroup
            | Display::TableRow
            | Display::TableColumnGroup
            | Display::TableColumn
            | Display::TableCell
            | Display::TableCaption
    );
    if !laid_from_colours {
        probe.background_color = new.background_color;
        probe.background = new.background.clone();
        probe.border_color = new.border_color;
        probe.border_current_color = new.border_current_color;
    }
    probe == *new
}

/// Whether a box of `pseudo`'s, or of its element when `None`, with
/// `style` makes no fragment: what a change found none for must still
/// be, laid out again otherwise.
fn makes_none(pseudo: Option<PseudoElement>, style: &ComputedStyle) -> bool {
    match pseudo {
        None => style.display == Display::None,
        Some(PseudoElement::Before | PseudoElement::After) => {
            style.display == Display::None
                || matches!(style.content, Content::Normal | Content::None)
        }
        Some(_) => false,
    }
}

/// What a restyle's composited changes did to a layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Composited {
    /// Their fragments carry the new effects, and the scrollable
    /// overflow is as it was.
    Done,
    /// Their fragments carry the new effects, but a transformed box
    /// reached, or now reaches, the bottom of its scroll container or
    /// of the document: the overflow is to be measured again.
    Overflow,
    /// A change found no fragment carrying its old effects.
    Missed,
}

/// A restyle's changes, found by the element and pseudo-element each is
/// of: a restyle makes one for each at most.
struct Changes<'c, 'a> {
    changes: &'c [EffectsChange<'a>],
    by_box: HashMap<(NodeId, Option<PseudoElement>), usize>,
    /// The changes of styles in what painting reads alone (M35.9), by
    /// element and pseudo-element, and the elements they are of.
    styles: &'c [StyleChange<'a>],
    by_style: HashMap<(NodeId, Option<PseudoElement>), usize>,
    restyled: HashSet<NodeId>,
    /// The style each replaces, as its fragments carry it, once found.
    olds: Vec<Option<Arc<ComputedStyle>>>,
}

impl<'c, 'a> Changes<'c, 'a> {
    fn new(changes: &'c [EffectsChange<'a>]) -> Self {
        let by_box: HashMap<_, _> = changes
            .iter()
            .enumerate()
            .map(|(i, change)| ((change.node, change.pseudo), i))
            .collect();
        debug_assert_eq!(by_box.len(), changes.len(), "a change for each box");
        Changes {
            changes,
            by_box,
            styles: &[],
            by_style: HashMap::new(),
            restyled: HashSet::new(),
            olds: Vec::new(),
        }
    }

    /// The change of style that reaches `fragment`, if one does: one of
    /// its element's, or its pseudo-element's, whose old style it
    /// carries.
    fn restyling(&self, fragment: &Fragment) -> Option<usize> {
        let &i = self.by_style.get(&(fragment.node, fragment.generated))?;
        let old = self.olds[i].as_ref()?;
        Arc::ptr_eq(&fragment.style, old).then_some(i)
    }

    /// Gives `fragment` the new style of the change `i`.
    fn restyle(&self, fragment: &mut Fragment, i: usize) {
        fragment.style = Arc::clone(self.styles[i].new);
    }

    /// The change that reaches `fragment`, if one does: one of its
    /// element's, or its pseudo-element's, whose old effects it carries.
    fn reaching(&self, fragment: &Fragment) -> Option<usize> {
        let &i = self.by_box.get(&(fragment.node, fragment.generated))?;
        fragment
            .style
            .effects
            .as_ref()
            .is_some_and(|effects| Arc::ptr_eq(effects, self.changes[i].old))
            .then_some(i)
    }

    /// Gives `fragment` the new effects of the change `i`.
    fn write(&self, fragment: &mut Fragment, i: usize) {
        let change = &self.changes[i];
        match change.styles {
            Some((old, new)) if Arc::ptr_eq(&fragment.style, old) => {
                fragment.style = Arc::clone(new);
            }
            _ => Arc::make_mut(&mut fragment.style).effects = Some(Arc::clone(change.new)),
        }
    }
}

impl DocumentLayout {
    /// Gives the fragments `patch`'s changes reach their new effects,
    /// the document laid out `viewport_width` wide (M22.4): the boxes
    /// whose extent counts toward a scroll container's overflow, or the
    /// document's, measured before and after, the lists of children on
    /// the way to a change copied once (M35.9).
    pub fn composite(&mut self, patch: &mut Patch<'_, '_>, viewport_width: f32) -> Composited {
        let mut walk = Walk {
            changes: &patch.changes,
            path: Vec::new(),
            measured: Vec::new(),
            olds: vec![None; patch.changes.styles.len()],
            mixed: false,
        };
        let height = self.height;
        for (i, root) in self.roots.iter().enumerate() {
            // A fixed box does not scroll: the document's height leaves
            // it out.
            let edge = (root.style.position != Position::Fixed).then_some(height);
            walk.path.push(i);
            walk.visit(root, viewport_width, edge, false, 0.0);
            walk.path.pop();
        }
        let (measured, olds) = (walk.measured, walk.olds);
        // A change of style is written only where it is what painting
        // reads alone, every fragment of its element carrying the style
        // it replaces; else the layout is to be made again.
        if walk.mixed {
            return Composited::Missed;
        }
        for (change, old) in patch.changes.styles.iter().zip(&olds) {
            let alike = match old {
                Some(old) => repaints_only(old, change.new),
                None => makes_none(change.pseudo, change.new),
            };
            if !alike {
                return Composited::Missed;
            }
        }
        patch.changes.olds = olds;
        for root in &mut self.roots {
            patch.fragment(root);
        }
        if patch.patched.contains(&false) {
            return Composited::Missed;
        }
        let overflow = measured.iter().any(|m| {
            let mut fragment = &self.roots[m.path[0]];
            for &i in &m.path[1..] {
                fragment = &fragment.children[i];
            }
            m.before.max(m.top + extent_bottom(fragment, m.cb_width)) >= m.edge - EDGE
        });
        if overflow {
            Composited::Overflow
        } else {
            Composited::Done
        }
    }
}

/// A restyle's composited changes being written (M22.4, M35.9): into
/// the layout shown, then the layout cache's entries. Each list of
/// children on the way to a change is copied once, the copy given
/// wherever the list was shared, so that the layout and the entries
/// share the lists still; a list nothing reaches under is left as it
/// is, and is met once however many share it.
pub struct Patch<'c, 'a> {
    changes: Changes<'c, 'a>,
    /// Which changes found a fragment.
    patched: Vec<bool>,
    /// Every list met, by its address: the list, kept so that no other
    /// takes its address while the patch lasts, and its copy with the
    /// changes written, `None` when nothing under it changed.
    lists: HashMap<*const Vec<Fragment>, (ListOf, Option<ListOf>)>,
}

/// A list of children, shared.
type ListOf = Arc<Vec<Fragment>>;

impl<'c, 'a> Patch<'c, 'a> {
    /// The patch of `changes`, nothing written yet.
    pub fn new(changes: &'c [EffectsChange<'a>]) -> Self {
        Patch {
            changes: Changes::new(changes),
            patched: vec![false; changes.len()],
            lists: HashMap::new(),
        }
    }

    /// The patch with `styles`, changes of styles in what painting reads
    /// alone, to write too (M35.9): written only by a layout that finds
    /// them so ([`DocumentLayout::composite`]), and then into the
    /// cache's entries.
    pub fn restyled(mut self, styles: &'c [StyleChange<'a>]) -> Self {
        self.changes.by_style = styles
            .iter()
            .enumerate()
            .map(|(i, change)| ((change.node, change.pseudo), i))
            .collect();
        self.changes.restyled = styles.iter().map(|change| change.node).collect();
        self.changes.styles = styles;
        self.changes.olds = vec![None; styles.len()];
        self
    }

    /// Writes the changes into `fragment` and the fragments under it;
    /// whether any reached.
    pub(crate) fn fragment(&mut self, fragment: &mut Fragment) -> bool {
        let restyled = self.changes.restyling(fragment);
        if let Some(i) = restyled {
            self.changes.restyle(fragment, i);
        }
        let reached = self.changes.reaching(fragment);
        if let Some(i) = reached {
            self.changes.write(fragment, i);
            self.patched[i] = true;
        }
        let below = self.children(&mut fragment.children);
        restyled.is_some() || reached.is_some() || below
    }

    /// Writes the changes under `children`, a copy of the list in its
    /// place when any reached; whether any did.
    fn children(&mut self, children: &mut Children) -> bool {
        let Some(list) = children.shared() else {
            return false;
        };
        let key = Arc::as_ptr(list);
        if let Some((_, done)) = self.lists.get(&key) {
            return match done {
                Some(copy) => {
                    children.share(Arc::clone(copy));
                    true
                }
                None => false,
            };
        }
        let list = Arc::clone(list);
        let mut copy: Option<Vec<Fragment>> = None;
        for (i, child) in list.iter().enumerate() {
            let restyled = self.changes.restyling(child);
            let reached = self.changes.reaching(child);
            let mut below = child.children.clone();
            let changed = self.children(&mut below);
            if restyled.is_none() && reached.is_none() && !changed {
                continue;
            }
            let copy = copy.get_or_insert_with(|| list.to_vec());
            if let Some(r) = restyled {
                self.changes.restyle(&mut copy[i], r);
            }
            if let Some(r) = reached {
                self.changes.write(&mut copy[i], r);
                self.patched[r] = true;
            }
            copy[i].children = below;
        }
        let copy = copy.map(Arc::new);
        if let Some(copy) = &copy {
            children.share(Arc::clone(copy));
        }
        let changed = copy.is_some();
        self.lists.insert(key, (list, copy));
        changed
    }
}

/// A box measured for a change: where it is in the tree, by its places
/// among its parents' children from a root's, where it reached before,
/// where its parent is on the page, its containing block's width, and
/// the bottom of the scroll container it is in.
struct Measured {
    path: Vec<usize>,
    before: f32,
    top: f32,
    cb_width: f32,
    edge: f32,
}

/// A walk over a layout's fragments, before a restyle's changes are
/// written, for the boxes to measure.
struct Walk<'w, 'c, 'a> {
    changes: &'w Changes<'c, 'a>,
    /// Where the walk is: the places among their parents' children of
    /// the fragments it is in, from a root's.
    path: Vec<usize>,
    measured: Vec<Measured>,
    /// The style each change of style replaces, as the first fragment of
    /// its element met carries it.
    olds: Vec<Option<Arc<ComputedStyle>>>,
    /// Whether a fragment of a restyled element carries another style,
    /// or one of a pseudo-element no change is of.
    mixed: bool,
}

impl Walk<'_, '_, '_> {
    /// Visits `fragment`, whose containing block is `cb_width` wide and
    /// whose parent is at `top` on the page (its offset is from there,
    /// M35.9), in a scroll container whose scrollable overflow ends at
    /// `edge` on the page, `None` when it counts toward none; `covered`
    /// when a box around it in that container is measured for the
    /// changes, which measures it too.
    fn visit(
        &mut self,
        fragment: &Fragment,
        cb_width: f32,
        edge: Option<f32>,
        covered: bool,
        top: f32,
    ) {
        if self.changes.restyled.contains(&fragment.node) {
            match self
                .changes
                .by_style
                .get(&(fragment.node, fragment.generated))
            {
                Some(&i) => match &self.olds[i] {
                    None => self.olds[i] = Some(Arc::clone(&fragment.style)),
                    Some(old) => self.mixed |= !Arc::ptr_eq(old, &fragment.style),
                },
                None => self.mixed = true,
            }
        }
        let reached = self.changes.reaching(fragment);
        // A box whose transform changed is measured, or the outermost
        // transformed box around one, which projects it.
        let measured = !covered
            && matches!(fragment.kind, FragmentKind::Box)
            && (reached.is_some_and(|i| self.changes.changes[i].transformed)
                || (!fragment.clips()
                    && fragment.matrix(cb_width).is_some()
                    && self.holds(fragment)));
        let measured = match edge {
            Some(edge) if measured => {
                self.measured.push(Measured {
                    path: self.path.clone(),
                    before: top + extent_bottom(fragment, cb_width),
                    top,
                    cb_width,
                    edge,
                });
                true
            }
            _ => false,
        };
        let content_width = fragment.content_width(cb_width);
        let (inner_edge, inner_covered) = if fragment.clips() {
            let border_top = fragment.style.used_border_width()[TOP];
            (
                Some(top + fragment.y + border_top + fragment.scroll_height),
                false,
            )
        } else {
            (edge, covered || measured)
        };
        let inner_top = top + fragment.y;
        for (i, child) in fragment.children.iter().enumerate() {
            self.path.push(i);
            self.visit(child, content_width, inner_edge, inner_covered, inner_top);
            self.path.pop();
        }
    }

    /// Whether a box a change of its transform reaches is inside
    /// `fragment` where its extent counts toward `fragment`'s: not
    /// inside a box that clips.
    fn holds(&self, fragment: &Fragment) -> bool {
        fragment.children.iter().any(|child| {
            (matches!(child.kind, FragmentKind::Box)
                && self
                    .changes
                    .reaching(child)
                    .is_some_and(|i| self.changes.changes[i].transformed))
                || (!child.clips() && self.holds(child))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::layout_document;
    use crate::testing::{Laid, boxes_of};
    use style::computed::{LengthPercentage, Translate};
    use text::FontCollection;

    const VIEWPORT: f32 = 400.0;

    /// The document `html` parses to, styled under `css` with the body's
    /// margin taken away, laid out against a 400px viewport.
    fn laid(html: &str, css: &str) -> Laid {
        let fonts = FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses");
        let doc = html::parse_document(html).expect("html parsing never fails");
        let sheet = css::parse_stylesheet(&format!("body {{ margin: 0 }} {css}"))
            .expect("test css is valid");
        let (document, boxes) = boxes_of(&doc, &[sheet]);
        let layout = layout_document(&boxes, VIEWPORT, &fonts);
        Laid::new(document, layout)
    }

    /// `changes` composited into `layout`.
    fn composite(layout: &mut DocumentLayout, changes: &[EffectsChange<'_>]) -> Composited {
        layout.composite(&mut Patch::new(changes), VIEWPORT)
    }

    /// The element whose `id` is `id`.
    fn node(laid: &Laid, id: &str) -> NodeId {
        let document = &laid.document;
        document
            .subtree(document.root())
            .find(|&n| document.attr(n, "id") == Some(id))
            .unwrap_or_else(|| panic!("no element #{id}"))
    }

    /// The effects the box of the element `id` carries.
    fn effects(laid: &Laid, id: &str) -> Arc<Effects> {
        let node = node(laid, id);
        laid.layout
            .iter()
            .find(|f| f.node == node && matches!(f.kind, FragmentKind::Box))
            .and_then(|f| f.style.effects.clone())
            .unwrap_or_else(|| panic!("no effects for #{id}"))
    }

    /// `old` translated by `dy` down and at `opacity`.
    fn moved(old: &Effects, dy: f32, opacity: f32) -> Arc<Effects> {
        Arc::new(Effects {
            translate: Some(Translate {
                x: LengthPercentage::Px(0.0),
                y: LengthPercentage::Px(dy),
                z: 0.0,
            }),
            opacity,
            ..old.clone()
        })
    }

    /// Composites the element `id`'s change from the effects its box
    /// carries to them translated by `dy`, a transform's change.
    fn translate(laid: &mut Laid, id: &str, dy: f32) -> Composited {
        let old = effects(laid, id);
        let new = moved(&old, dy, old.opacity);
        let change = EffectsChange {
            node: node(laid, id),
            pseudo: None,
            old: &old,
            new: &new,
            transformed: true,
            styles: None,
        };
        composite(&mut laid.layout, &[change])
    }

    #[test]
    fn a_change_gives_its_elements_boxes_the_new_effects_and_no_others() {
        let mut laid = laid(
            "<div id=a><p id=p>one</p></div><div id=b></div><div id=tall></div>",
            "#a, #b { translate: 0 0; height: 20px } #tall { height: 500px }",
        );
        let (a, b) = (effects(&laid, "a"), effects(&laid, "b"));
        let new = moved(&a, 5.0, 0.5);
        let change = EffectsChange {
            node: node(&laid, "a"),
            pseudo: None,
            old: &a,
            new: &new,
            transformed: true,
            styles: None,
        };
        assert_eq!(composite(&mut laid.layout, &[change]), Composited::Done);
        assert!(Arc::ptr_eq(&effects(&laid, "a"), &new));
        assert!(Arc::ptr_eq(&effects(&laid, "b"), &b));
        // The paragraph inside has effects of its own, none.
        let p = node(&laid, "p");
        assert!(
            laid.layout
                .iter()
                .filter(|f| f.node == p)
                .all(|f| f.style.effects.is_none())
        );
        // A change from effects no box carries finds none.
        let equal = Arc::new((*new).clone());
        let change = EffectsChange {
            old: &equal,
            new: &a,
            ..change
        };
        assert_eq!(composite(&mut laid.layout, &[change]), Composited::Missed);
        assert!(Arc::ptr_eq(&effects(&laid, "a"), &new));
    }

    #[test]
    fn a_restyles_changes_are_written_together() {
        let mut laid = laid(
            "<div id=a></div><div id=b></div><div id=c></div>",
            "div { translate: 0 0; height: 100px }",
        );
        let [a, b, c] = ["a", "b", "c"].map(|id| effects(&laid, id));
        let nodes = ["a", "b", "c"].map(|id| (id, node(&laid, id)));
        let (new_a, new_b) = (moved(&a, 5.0, 0.5), moved(&b, 5.0, 1.0));
        let change = |id: &str, old, new, transformed| EffectsChange {
            node: nodes.iter().find(|(name, _)| *name == id).expect("a box").1,
            pseudo: None,
            old,
            new,
            transformed,
            styles: None,
        };
        // Two in the middle: both written, the overflow as it was.
        let changes = [
            change("a", &a, &new_a, false),
            change("b", &b, &new_b, true),
        ];
        assert_eq!(composite(&mut laid.layout, &changes), Composited::Done);
        assert!(Arc::ptr_eq(&effects(&laid, "a"), &new_a));
        assert!(Arc::ptr_eq(&effects(&laid, "b"), &new_b));
        // One finding no box: missed, though the other is written.
        let (newer_a, stale) = (moved(&new_a, 0.0, 1.0), Arc::new((*b).clone()));
        let changes = [
            change("a", &new_a, &newer_a, true),
            change("b", &stale, &b, true),
        ];
        assert_eq!(composite(&mut laid.layout, &changes), Composited::Missed);
        assert!(Arc::ptr_eq(&effects(&laid, "a"), &newer_a));
        // One at the bottom: the overflow to measure again.
        let new_c = moved(&c, 5.0, 1.0);
        let changes = [
            change("c", &c, &new_c, true),
            change("a", &newer_a, &new_a, false),
        ];
        assert_eq!(composite(&mut laid.layout, &changes), Composited::Overflow);
        assert!(Arc::ptr_eq(&effects(&laid, "c"), &new_c));
    }

    #[test]
    fn a_box_whose_transform_reached_or_reaches_the_documents_bottom_measures_it_again() {
        let mut laid = laid(
            "<div id=top></div><div id=middle></div><div id=bottom></div>",
            "div { translate: 0 0; height: 100px }",
        );
        // Away from the bottom, the overflow is as it was.
        assert_eq!(translate(&mut laid, "top", 50.0), Composited::Done);
        assert_eq!(translate(&mut laid, "middle", 10.0), Composited::Done);
        // Down from the bottom, back up to it, and up from it.
        assert_eq!(translate(&mut laid, "bottom", 5.0), Composited::Overflow);
        assert_eq!(translate(&mut laid, "bottom", 0.0), Composited::Overflow);
        assert_eq!(translate(&mut laid, "bottom", -50.0), Composited::Overflow);
        // Down to it from above, and away again.
        assert_eq!(translate(&mut laid, "middle", 100.0), Composited::Overflow);
        assert_eq!(translate(&mut laid, "middle", 10.0), Composited::Overflow);
        // An opacity changes no overflow, at the bottom too.
        let old = effects(&laid, "bottom");
        let new = moved(&old, -50.0, 0.25);
        let change = EffectsChange {
            node: node(&laid, "bottom"),
            pseudo: None,
            old: &old,
            new: &new,
            transformed: false,
            styles: None,
        };
        assert_eq!(composite(&mut laid.layout, &[change]), Composited::Done);
    }

    #[test]
    fn a_fragment_carrying_the_very_style_takes_the_new_one_shared() {
        // T4.3 (M35): an opacity animated on 1,000 paragraphs copied a
        // style a fragment a frame, 8us each, the lists being shared.
        let mut laid = laid("<div id=a></div>", "div { opacity: 0.5; height: 10px }");
        let with = |style: &Arc<ComputedStyle>, opacity: f32| {
            let mut effects = (**style.effects.as_ref().expect("effects")).clone();
            effects.opacity = opacity;
            let effects = Arc::new(effects);
            let mut changed = (**style).clone();
            changed.effects = Some(Arc::clone(&effects));
            (effects, Arc::new(changed))
        };
        let old = style_of(&laid, "a");
        let (new_effects, new) = with(&old, 0.25);
        let change = EffectsChange {
            node: node(&laid, "a"),
            pseudo: None,
            old: old.effects.as_ref().expect("effects"),
            new: &new_effects,
            transformed: false,
            styles: Some((&old, &new)),
        };
        assert_eq!(composite(&mut laid.layout, &[change]), Composited::Done);
        assert!(Arc::ptr_eq(&style_of(&laid, "a"), &new), "shared");
        // A fragment that carries another style takes a copy of its own
        // with the new effects.
        let stranger = Arc::new((*new).clone());
        let (newer_effects, newer) = with(&new, 0.75);
        let change = EffectsChange {
            node: node(&laid, "a"),
            pseudo: None,
            old: &new_effects,
            new: &newer_effects,
            transformed: false,
            styles: Some((&stranger, &newer)),
        };
        assert_eq!(composite(&mut laid.layout, &[change]), Composited::Done);
        let carried = style_of(&laid, "a");
        assert!(!Arc::ptr_eq(&carried, &newer));
        assert_eq!(carried, newer, "the same values");
    }

    /// The style the box of the element `id` carries.
    fn style_of(laid: &Laid, id: &str) -> Arc<ComputedStyle> {
        let node = node(laid, id);
        laid.layout
            .iter()
            .find(|f| {
                f.node == node && matches!(f.kind, FragmentKind::Box | FragmentKind::InlineBox)
            })
            .map(|f| Arc::clone(&f.style))
            .unwrap_or_else(|| panic!("no box for #{id}"))
    }

    /// `old` changed by `change`.
    fn changed(old: &ComputedStyle, change: impl FnOnce(&mut ComputedStyle)) -> Arc<ComputedStyle> {
        let mut new = old.clone();
        change(&mut new);
        Arc::new(new)
    }

    /// The change of the element `id`'s style to `new`, written.
    fn restyle(laid: &mut Laid, id: &str, new: &Arc<ComputedStyle>) -> Composited {
        let change = StyleChange {
            node: node(laid, id),
            pseudo: None,
            new,
        };
        laid.layout
            .composite(&mut Patch::new(&[]).restyled(&[change]), VIEWPORT)
    }

    #[test]
    fn what_painting_reads_alone_is_a_colour_an_outline_a_cursor_and_a_boxs_background() {
        // M35.9.
        let plain = style::ComputedStyle::default();
        let red = style::computed::Color::rgb8(255, 0, 0);
        let alone = |change: &dyn Fn(&mut ComputedStyle)| {
            let mut new = plain.clone();
            change(&mut new);
            repaints_only(&plain, &new)
        };
        assert!(alone(&|s| s.color = red));
        assert!(alone(&|s| s.background_color = red));
        assert!(alone(&|s| s.border_color = [red; 4]));
        assert!(alone(&|s| s.cursor = style::computed::Cursor::Pointer));
        assert!(!alone(&|s| s.font_size_px = 20.0));
        assert!(!alone(&|s| s.display = Display::Block));
        // A row's background is laid out, a decorating box's colour.
        let row = ComputedStyle {
            display: Display::TableRow,
            ..plain.clone()
        };
        let mut new = row.clone();
        new.background_color = red;
        assert!(!repaints_only(&row, &new));
        let mut underlined = plain.clone();
        underlined.decoration_mut().line.underline = true;
        let mut new = underlined.clone();
        new.color = red;
        assert!(!repaints_only(&underlined, &new));
    }

    #[test]
    fn a_change_that_only_repaints_gives_the_elements_fragments_its_new_style() {
        // M35.9: the paragraph's box and its text take the new style,
        // the span's, its own, keep theirs; nothing is laid out.
        let mut laid = laid("<p id=p>one <span id=s>two</span></p>", "");
        let old = style_of(&laid, "p");
        let span = style_of(&laid, "s");
        let new = changed(&old, |s| s.color = style::computed::Color::rgb8(255, 0, 0));
        assert_eq!(restyle(&mut laid, "p", &new), Composited::Done);
        let p = node(&laid, "p");
        let mine: Vec<_> = laid.layout.iter().filter(|f| f.node == p).collect();
        assert!(mine.len() >= 2 && mine.iter().all(|f| Arc::ptr_eq(&f.style, &new)));
        assert!(Arc::ptr_eq(&style_of(&laid, "s"), &span));
    }

    #[test]
    fn a_change_laid_out_or_whose_element_has_a_style_made_from_it_is_missed() {
        // M35.9: a size is laid out; a first line's style is made from
        // its element's at layout, and would keep the old colour; an
        // element with no box, which its new style would give one. An
        // anonymous block's lines are its element's, which carry its
        // style.
        let mut laid = laid(
            "<p id=p>one</p><p id=d>first<br>line</p><div id=n></div><div id=a>text<p>block</p></div>",
            "#n { display: none } #d::first-line { letter-spacing: 1px }",
        );
        let old = style_of(&laid, "p");
        let bigger = changed(&old, |s| s.font_size_px = 30.0);
        assert_eq!(restyle(&mut laid, "p", &bigger), Composited::Missed);
        assert!(Arc::ptr_eq(&style_of(&laid, "p"), &old), "nothing written");
        let div = style_of(&laid, "d");
        let red = changed(&div, |s| s.color = style::computed::Color::rgb8(255, 0, 0));
        assert_eq!(restyle(&mut laid, "d", &red), Composited::Missed);
        let shown = changed(&div, |s| s.display = Display::Block);
        assert_eq!(restyle(&mut laid, "n", &shown), Composited::Missed);
        let hidden = changed(&div, |s| s.display = Display::None);
        assert_eq!(restyle(&mut laid, "n", &hidden), Composited::Done);
        let anonymous = style_of(&laid, "a");
        let red = changed(&anonymous, |s| {
            s.color = style::computed::Color::rgb8(255, 0, 0)
        });
        assert_eq!(restyle(&mut laid, "a", &red), Composited::Done);
    }

    #[test]
    fn a_box_in_a_scroll_container_counts_toward_its_overflow_alone() {
        let mut laid = laid(
            "<div id=s><div id=a></div><div id=fill></div><div id=b></div></div>",
            "#s { overflow: auto; height: 100px; border-top: 10px solid #000; \
             padding-bottom: 8px } #a, #b { height: 20px; translate: 0 0 } \
             #fill { height: 300px }",
        );
        // The scroller is the document's bottom; its content is not.
        assert_eq!(translate(&mut laid, "a", 50.0), Composited::Done);
        // Its overflow ends at its bottom padding, below the last box.
        assert_eq!(translate(&mut laid, "b", 5.0), Composited::Done);
        assert_eq!(translate(&mut laid, "b", 20.0), Composited::Overflow);
        // The same further down the page, inside another box: its edge
        // is where it is on the page, its offset added to its parent's
        // place (M35.9).
        let mut lower = self::laid(
            "<div id=above></div><div><div id=s><div id=a></div><div id=fill></div>             <div id=b></div></div></div>",
            "#above { height: 40px } #s { overflow: auto; height: 100px;              border-top: 10px solid #000; padding-bottom: 8px }              #a, #b { height: 20px; translate: 0 0 } #fill { height: 300px }",
        );
        assert_eq!(translate(&mut lower, "b", 5.0), Composited::Done);
        assert_eq!(translate(&mut lower, "b", 20.0), Composited::Overflow);
    }

    #[test]
    fn a_box_in_a_transformed_box_is_measured_through_it() {
        let css = ".t { rotate: 10deg } .s { overflow: hidden; height: 50px } \
                   .tall { height: 1000px } #a, #b, #c, #d { height: 20px; translate: 0 0 } \
                   #fill { height: 300px } #up { translate: 0 -200px }";
        // Far from the bottom, the transformed box around it, whose reach
        // the content of a box clipping it does not extend; or inside a
        // box that clips, inside a transformed box at the bottom: that
        // box's overflow is its own.
        let mut far = laid(
            "<div class=t><div id=a></div><div class=s><div class=tall></div></div></div>\
             <div id=fill></div>\
             <div class=t><div class=s><div id=d></div><div class=tall></div></div></div>",
            css,
        );
        assert_eq!(translate(&mut far, "a", 1.0), Composited::Done);
        assert_eq!(translate(&mut far, "d", 1.0), Composited::Done);
        // At the bottom, its projection moved by what moved inside it.
        let mut bottom = laid(
            "<div id=fill></div><div class=t><div id=b></div></div>",
            css,
        );
        assert_eq!(translate(&mut bottom, "b", -1.0), Composited::Overflow);
        // A box at the bottom of the flow, but projected far above it.
        let mut up = laid("<div id=fill></div><div id=up><div id=c></div></div>", css);
        assert_eq!(translate(&mut up, "c", 1.0), Composited::Done);
    }

    #[test]
    fn a_fixed_box_counts_toward_no_overflow() {
        let mut laid = laid(
            "<div id=f></div><div id=fill></div>",
            "#f { position: fixed; bottom: 0; height: 20px; width: 20px; translate: 0 0 } \
             #fill { height: 50px }",
        );
        assert_eq!(translate(&mut laid, "f", 400.0), Composited::Done);
    }
}
