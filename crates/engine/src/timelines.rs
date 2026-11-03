//! The scroll-driven animations' timelines measured from a frame's layout
//! (Scroll-driven Animations 1 sections 2.1, 3.1 and 4.4; D75): for each
//! progress-based timeline, the scroll container it follows, how far it
//! is scrolled along the timeline's axis, and the offsets at which each
//! of the timeline's named ranges starts and ends. A scroll timeline's
//! ranges are all its scroll range; a view timeline's are where its
//! subject comes into its scroll container's scrollport, less the
//! timeline's inset, and leaves it. A named timeline is the one the
//! element nearest its looker up the tree declares by that name, a
//! `timeline-scope` reaching down to a declaration in its subtree.
//!
//! The engine lays out horizontal text only (the design note's section
//! 2), so a timeline's `block` axis is the vertical one and `inline` the
//! horizontal.

use std::collections::HashSet;

use dom::{Document, NodeId};
use layout::DocumentLayout;
use style::Styles;
use style::animation::{Measured, TimelineKind, TimelineSource};
use style::computed::{Axis, Inset, Insets, LengthPercentage, Scope};

/// What a timeline is measured from: the document, its styles and its
/// layout, the viewport's size and the scroll offsets the frame shows.
pub(crate) struct Scene<'a> {
    pub(crate) document: &'a Document,
    pub(crate) styles: Option<&'a Styles>,
    pub(crate) layout: &'a DocumentLayout,
    /// The viewport's width and height, in CSS pixels.
    pub(crate) viewport: (f32, f32),
    /// The viewport's scroll offset down and across, and each scroll
    /// container's.
    pub(crate) scroll: (f32, f32),
    pub(crate) offsets: &'a paint::Offsets,
}

/// A scroll container: an element's box that clips, or the viewport
/// (`None`).
type Scroller = Option<NodeId>;

impl Scene<'_> {
    /// What the progress-based timeline of `kind` is at this frame;
    /// `None` while it is inactive: no scroll container to follow, no
    /// subject laid out, or no timeline by its name.
    pub(crate) fn measure(&self, kind: &TimelineKind) -> Option<Measured> {
        match kind {
            TimelineKind::Document { .. } => None,
            TimelineKind::Scroll { source, axis } => {
                let scroller = self.source(source, *axis)?;
                self.scroll_timeline(scroller, *axis)
            }
            TimelineKind::View {
                subject,
                axis,
                inset,
            } => self.view_timeline((*subject)?, *axis, inset),
            TimelineKind::Named { name, from } => self.named(name, *from),
        }
    }

    /// Whether `node`'s box clips: a scroll container.
    fn clips(&self, node: NodeId) -> bool {
        self.layout
            .iter()
            .any(|f| f.node == node && f.generated.is_none() && f.clips())
    }

    /// Whether `node` stands for the viewport as a scroll container: the
    /// root element, or the body whose overflow the root's `visible`
    /// gives the viewport (Overflow 3 section 3.3).
    fn is_viewport(&self, node: NodeId) -> bool {
        let root = self.document.document_element();
        if Some(node) == root {
            return true;
        }
        let root_visible = root.is_some_and(|r| {
            self.styles
                .and_then(|s| s.get(r))
                .is_some_and(|s| s.overflow() == style::computed::Overflow::Visible)
        });
        root_visible
            && root.is_some_and(|r| self.document.parent(node) == Some(r))
            && self.document.tag(node) == Some("body")
    }

    /// The scroll container nearest `node` among its ancestors: the
    /// viewport where none clips.
    fn nearest(&self, node: NodeId, axis: Axis) -> Scroller {
        self.document
            .flat_ancestors(node)
            .filter(|&a| self.document.is_element(a))
            .find(|&a| !self.is_viewport(a) && self.clips(a) && self.scrolls_along(a, axis))
    }

    /// Whether `element`, a box that clips, scrolls along `axis`: its
    /// `overflow` there neither `visible` nor `clip` (Scroll-driven
    /// Animations 1 section 2.2.1, a single-axis scroll container skipped
    /// along its other axis).
    fn scrolls_along(&self, element: NodeId, axis: Axis) -> bool {
        use style::computed::Overflow;
        self.styles.and_then(|s| s.get(element)).is_none_or(|s| {
            let overflow = if axis.vertical() {
                s.overflow_y
            } else {
                s.overflow_x
            };
            !matches!(overflow, Overflow::Visible | Overflow::Clip)
        })
    }

    /// The scroll container `source` names; `None` for none.
    fn source(&self, source: &TimelineSource, axis: Axis) -> Option<Scroller> {
        match source {
            TimelineSource::Element(None) => None,
            TimelineSource::Element(Some(node)) | TimelineSource::Own(node) => {
                if self.is_viewport(*node) {
                    Some(None)
                } else if self.clips(*node) {
                    Some(Some(*node))
                } else {
                    None
                }
            }
            TimelineSource::Nearest(node) => Some(self.nearest(*node, axis)),
            TimelineSource::Root => Some(None),
        }
    }

    /// Where `scroller` is scrolled along `axis`, how far it can be, and
    /// its scrollport's size along it.
    fn scroll_state(&self, scroller: Scroller, axis: Axis) -> (f64, f64, f64) {
        let vertical = axis.vertical();
        match scroller {
            None => {
                let (width, height) = self.viewport;
                if vertical {
                    let bottom = crate::geometry::document_height(self.layout);
                    (
                        f64::from(self.scroll.0),
                        f64::from((bottom - height).max(0.0)),
                        f64::from(height),
                    )
                } else {
                    let right = crate::geometry::document_width(self.layout);
                    (
                        f64::from(self.scroll.1),
                        f64::from((right - width).max(0.0)),
                        f64::from(width),
                    )
                }
            }
            Some(node) => {
                let own = || {
                    self.layout
                        .iter()
                        .filter(move |f| f.node == node && f.generated.is_none() && f.clips())
                };
                let port = self.unscrolled(node).map_or(0.0, |m| {
                    let [bt, br, bb, bl] = m.border;
                    if vertical {
                        m.bounds[3] - bt - bb
                    } else {
                        m.bounds[2] - bl - br
                    }
                });
                if vertical {
                    (
                        f64::from(self.offsets.y.get(&node).copied().unwrap_or(0.0)),
                        f64::from(own().map(|f| f.max_scroll()).fold(0.0f32, f32::max)),
                        f64::from(port),
                    )
                } else {
                    (
                        f64::from(self.offsets.x.get(&node).copied().unwrap_or(0.0)),
                        f64::from(
                            own()
                                .map(crate::geometry::max_scroll_x)
                                .fold(0.0f32, f32::max),
                        ),
                        f64::from(port),
                    )
                }
            }
        }
    }

    /// `node`'s boxes measured with nothing scrolled.
    fn unscrolled(&self, node: NodeId) -> Option<crate::scripts::Measured> {
        let viewport = paint::Viewport {
            width: self.viewport.0,
            height: self.viewport.1,
            scroll_y: 0.0,
        };
        let targets: HashSet<NodeId> = std::iter::once(node).collect();
        crate::geometry::measure(self.layout, viewport, &paint::Offsets::default(), &targets)
            .remove(&node)
    }

    /// A scroll progress timeline of `scroller` along `axis`: its whole
    /// scroll range every range.
    fn scroll_timeline(&self, scroller: Scroller, axis: Axis) -> Option<Measured> {
        let (offset, max, _) = self.scroll_state(scroller, axis);
        Some(Measured {
            view: false,
            scroller,
            subject: None,
            axis,
            offset,
            ranges: [(0.0, max); 6],
        })
    }

    /// A view progress timeline of `subject` along `axis`, its scroll
    /// container's scrollport narrowed by `inset` (section 3.1): `auto`
    /// the container's `scroll-padding` there.
    fn view_timeline(
        &self,
        subject: NodeId,
        axis: Axis,
        inset: &Insets<LengthPercentage>,
    ) -> Option<Measured> {
        if !self.document.contains(subject) {
            return None;
        }
        let scroller = self.nearest(subject, axis);
        let vertical = axis.vertical();
        let (offset, _, port) = self.scroll_state(scroller, axis);
        let own = self.unscrolled(subject)?;
        // The subject's place in its scroll container's content, scrolled
        // to its start.
        let origin = match scroller {
            None => 0.0,
            Some(node) => {
                let m = self.unscrolled(node)?;
                if vertical {
                    m.bounds[1] + m.border[0]
                } else {
                    m.bounds[0] + m.border[3]
                }
            }
        };
        let (at, size) = if vertical {
            (own.bounds[1] - origin, own.bounds[3])
        } else {
            (own.bounds[0] - origin, own.bounds[2])
        };
        let (at, size) = (f64::from(at), f64::from(size));
        let padding = self
            .styles
            .and_then(|s| s.get(scroller.or_else(|| self.document.document_element())?))
            .map(|s| s.effects().scroll_padding.clone());
        let side = |i: usize, padding_side: usize| -> f64 {
            match &inset[i] {
                Inset::Length(l) => f64::from(l.resolve(port as f32)),
                Inset::Auto => padding.as_ref().map_or(0.0, |p| {
                    f64::from(p[padding_side].resolve(port as f32, 0.0))
                }),
            }
        };
        let (start_inset, end_inset) = if vertical {
            (side(0, 0), side(1, 2))
        } else {
            (side(0, 3), side(1, 1))
        };
        let seen = (port - start_inset - end_inset).max(0.0);
        // Where the subject's start edge meets the scrollport's end, and
        // its end edge the scrollport's start.
        let cover = (at - start_inset - seen, at + size - start_inset);
        let contain = if size <= seen {
            (at + size - start_inset - seen, at - start_inset)
        } else {
            (at - start_inset, at + size - start_inset - seen)
        };
        Some(Measured {
            view: true,
            scroller,
            subject: Some(subject),
            axis,
            offset,
            ranges: [
                cover,
                contain,
                (cover.0, contain.0),
                (contain.1, cover.1),
                (cover.0, cover.0 + size),
                (cover.1 - size, cover.1),
            ],
        })
    }

    /// The timeline named `name` in scope at `from` (section 4.4): the
    /// one `from` or the nearest of its flat tree ancestors declares, or,
    /// where an ancestor's `timeline-scope` takes the name, the last one
    /// an element of its subtree declares; the nearest such ancestor
    /// decides.
    fn named(&self, name: &str, from: NodeId) -> Option<Measured> {
        self.styles?;
        for element in std::iter::once(from).chain(self.document.flat_ancestors(from)) {
            if !self.document.is_element(element) {
                continue;
            }
            if let Some(found) = self.declared(element, name) {
                return found;
            }
            if self.scopes(element, name) {
                // The last in tree order that declares it, as the reference
                // browser resolves a conflict, an inner scope's left out.
                let mut found = Vec::new();
                self.declarers(element, name, &mut found);
                return found
                    .into_iter()
                    .rev()
                    .find_map(|n| self.declared(n, name))
                    .flatten();
            }
        }
        None
    }

    /// Whether `element`'s `timeline-scope` takes `name`.
    fn scopes(&self, element: NodeId, name: &str) -> bool {
        self.styles
            .and_then(|s| s.get(element))
            .is_some_and(|s| match &s.motion().timeline_scope {
                Scope::None => false,
                Scope::All => true,
                Scope::Names(names) => names.iter().any(|n| &**n == name),
            })
    }

    /// The elements under `element` in the flat tree, in tree order, that
    /// may declare `name` for its scope: those under an inner scope of the
    /// name left out.
    fn declarers(&self, element: NodeId, name: &str, out: &mut Vec<NodeId>) {
        let children: Vec<NodeId> = self.document.flat_children(element).collect();
        for child in children {
            if !self.document.is_element(child) {
                continue;
            }
            if self.scopes(child, name) {
                continue;
            }
            out.push(child);
            self.declarers(child, name, out);
        }
    }

    /// The timeline `element` declares by `name`: `None` where it
    /// declares none so named, else what it is.
    fn declared(&self, element: NodeId, name: &str) -> Option<Option<Measured>> {
        let motion = self.styles?.get(element)?.motion();
        let at = |names: &[Option<std::sync::Arc<str>>]| {
            names.iter().position(|n| n.as_deref() == Some(name))
        };
        if let Some(i) = at(&motion.scroll_timeline_name) {
            let axis = motion.scroll_timeline_axis[i % motion.scroll_timeline_axis.len()];
            let scroller = self.source(&TimelineSource::Own(element), axis);
            return Some(scroller.and_then(|s| self.scroll_timeline(s, axis)));
        }
        if let Some(i) = at(&motion.view_timeline_name) {
            let axis = motion.view_timeline_axis[i % motion.view_timeline_axis.len()];
            let inset = &motion.view_timeline_inset[i % motion.view_timeline_inset.len()];
            return Some(self.view_timeline(element, axis, inset));
        }
        None
    }
}
