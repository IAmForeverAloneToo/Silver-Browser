//! Composited changes (M22.4): a change of an element's style in its
//! transform or its opacity alone, which leaves its box what it was to
//! layout, written into the fragments that carry the element's effects —
//! in the layout shown and in the layout cache's entries — rather than
//! laid out again. A restyle's changes are written together, in one walk
//! over the fragments, each found by the element and pseudo-element it
//! is of: a frame of a thousand animations walks them once.
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

use std::collections::HashMap;
use std::sync::Arc;

use dom::NodeId;
use style::computed::{Effects, Position, TOP};
use style::selector_matching::PseudoElement;

use crate::block::{DocumentLayout, Fragment, FragmentKind};

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
}

impl<'c, 'a> Changes<'c, 'a> {
    fn new(changes: &'c [EffectsChange<'a>]) -> Self {
        let by_box: HashMap<_, _> = changes
            .iter()
            .enumerate()
            .map(|(i, change)| ((change.node, change.pseudo), i))
            .collect();
        debug_assert_eq!(by_box.len(), changes.len(), "a change for each box");
        Changes { changes, by_box }
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
        fragment.style.effects = Some(Arc::clone(self.changes[i].new));
    }
}

impl DocumentLayout {
    /// Gives the fragments `changes` reach their new effects, the
    /// document laid out `viewport_width` wide (M22.4).
    pub fn composite(&mut self, changes: &[EffectsChange<'_>], viewport_width: f32) -> Composited {
        let mut walk = Walk {
            changes: Changes::new(changes),
            patched: vec![false; changes.len()],
            overflow: false,
        };
        let height = self.height;
        for root in &mut self.roots {
            // A fixed box does not scroll: the document's height leaves
            // it out.
            let edge = (root.style.position != Position::Fixed).then_some(height);
            walk.visit(root, viewport_width, edge, false);
        }
        if walk.patched.contains(&false) {
            Composited::Missed
        } else if walk.overflow {
            Composited::Overflow
        } else {
            Composited::Done
        }
    }
}

/// Gives the fragments of `roots` and under them that `changes` reach
/// their new effects: the layout cache's entries'.
pub(crate) fn patch<'f>(
    roots: impl Iterator<Item = &'f mut Fragment>,
    changes: &[EffectsChange<'_>],
) {
    fn visit(fragment: &mut Fragment, changes: &Changes<'_, '_>) {
        if let Some(i) = changes.reaching(fragment) {
            changes.write(fragment, i);
        }
        for child in &mut fragment.children {
            visit(child, changes);
        }
    }
    let changes = Changes::new(changes);
    for root in roots {
        visit(root, &changes);
    }
}

/// A walk over a layout's fragments for a restyle's changes.
struct Walk<'c, 'a> {
    changes: Changes<'c, 'a>,
    /// Which changes found a fragment.
    patched: Vec<bool>,
    overflow: bool,
}

impl Walk<'_, '_> {
    /// Visits `fragment`, whose containing block is `cb_width` wide, in a
    /// scroll container whose scrollable overflow ends at `edge`, `None`
    /// when it counts toward none; `covered` when a box around it in
    /// that container is measured for the changes, which measures it
    /// too.
    fn visit(&mut self, fragment: &mut Fragment, cb_width: f32, edge: Option<f32>, covered: bool) {
        let reached = self.changes.reaching(fragment);
        // A box whose transform changed is measured, or the outermost
        // transformed box around one, which projects it.
        let measured = edge.is_some()
            && !covered
            && matches!(fragment.kind, FragmentKind::Box)
            && (reached.is_some_and(|i| self.changes.changes[i].transformed)
                || (!fragment.clips()
                    && fragment.matrix(cb_width).is_some()
                    && self.holds(fragment)));
        let before = measured.then(|| reach(fragment, cb_width));
        if let Some(i) = reached {
            self.changes.write(fragment, i);
            self.patched[i] = true;
        }
        let content_width = fragment.content_width(cb_width);
        let (inner_edge, inner_covered) = if fragment.clips() {
            let border_top = fragment.style.used_border_width()[TOP];
            (
                Some(fragment.y + border_top + fragment.scroll_height),
                false,
            )
        } else {
            (edge, covered || measured)
        };
        for child in &mut fragment.children {
            self.visit(child, content_width, inner_edge, inner_covered);
        }
        if let (Some(before), Some(edge)) = (before, edge)
            && before.max(reach(fragment, cb_width)) >= edge - EDGE
        {
            self.overflow = true;
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

/// The bottom of `fragment`'s box, or of anything inside it reaching
/// lower, in the absolute coordinates of a finished layout: `extent_bottom`
/// of `crate::block`, for a tree whose fragments are all in the
/// document's coordinates.
fn reach(fragment: &Fragment, cb_width: f32) -> f32 {
    let own = fragment.top() + fragment.h;
    let untransformed = if fragment.clips() || fragment.children.is_empty() {
        own
    } else {
        let content_width = fragment.content_width(cb_width);
        fragment
            .children
            .iter()
            .map(|child| reach(child, content_width))
            .fold(own, f32::max)
    };
    match fragment.matrix(cb_width) {
        None => untransformed,
        Some(matrix) => match matrix.bounds(0.0, 0.0, fragment.w, untransformed - fragment.y) {
            Some((_, _, _, bottom)) if matrix.inverse().is_some() => fragment.y + bottom,
            _ => fragment.y,
        },
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
        Laid { document, layout }
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
        laid.iter()
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
        };
        laid.layout.composite(&[change], VIEWPORT)
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
        };
        assert_eq!(laid.layout.composite(&[change], VIEWPORT), Composited::Done);
        assert!(Arc::ptr_eq(&effects(&laid, "a"), &new));
        assert!(Arc::ptr_eq(&effects(&laid, "b"), &b));
        // The paragraph inside has effects of its own, none.
        let p = node(&laid, "p");
        assert!(
            laid.iter()
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
        assert_eq!(
            laid.layout.composite(&[change], VIEWPORT),
            Composited::Missed
        );
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
        };
        // Two in the middle: both written, the overflow as it was.
        let changes = [
            change("a", &a, &new_a, false),
            change("b", &b, &new_b, true),
        ];
        assert_eq!(laid.layout.composite(&changes, VIEWPORT), Composited::Done);
        assert!(Arc::ptr_eq(&effects(&laid, "a"), &new_a));
        assert!(Arc::ptr_eq(&effects(&laid, "b"), &new_b));
        // One finding no box: missed, though the other is written.
        let (newer_a, stale) = (moved(&new_a, 0.0, 1.0), Arc::new((*b).clone()));
        let changes = [
            change("a", &new_a, &newer_a, true),
            change("b", &stale, &b, true),
        ];
        assert_eq!(
            laid.layout.composite(&changes, VIEWPORT),
            Composited::Missed
        );
        assert!(Arc::ptr_eq(&effects(&laid, "a"), &newer_a));
        // One at the bottom: the overflow to measure again.
        let new_c = moved(&c, 5.0, 1.0);
        let changes = [
            change("c", &c, &new_c, true),
            change("a", &newer_a, &new_a, false),
        ];
        assert_eq!(
            laid.layout.composite(&changes, VIEWPORT),
            Composited::Overflow
        );
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
        };
        assert_eq!(laid.layout.composite(&[change], VIEWPORT), Composited::Done);
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
