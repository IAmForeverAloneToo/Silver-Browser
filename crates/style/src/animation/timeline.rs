//! The timelines a document's animations run on (Web Animations 1 section
//! 4.2, Scroll-driven Animations 1 sections 2 and 3; D75): document
//! timelines, whose time is the document's less their origin, and the
//! progress-based ones that follow a scroll container: a scroll progress
//! timeline through its scroll range, a view progress timeline through
//! the range in which its subject is seen in it.
//!
//! A progress-based timeline's time is a percentage of its range, read
//! from what the engine measured of the scroll container at the frame's
//! start ([`Measured`]): the offset it is scrolled to and the offsets at
//! which each named range starts and ends. A timeline the engine could
//! not measure, whose source is no scroll container or whose range is
//! empty, is inactive: it has no time. The engine finds what each
//! timeline follows from its [`Kind`]: a script's source, the scroll
//! container nearest an element, the viewport, or a name an element
//! looks up among the timelines its ancestors declare.

use std::sync::Arc;

use dom::NodeId;

use crate::computed::{Axis, Edge, Insets, LengthPercentage, RangeName};

/// Where a scroll progress timeline finds its scroll container.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// A script's source: an element, or none (`null`).
    Element(Option<NodeId>),
    /// The scroll container nearest the element, among its ancestors
    /// (`scroll(nearest)`).
    Nearest(NodeId),
    /// The element itself (`scroll(self)`).
    Own(NodeId),
    /// The document's viewport (`scroll(root)`).
    Root,
}

/// What a timeline follows.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// A document timeline, `origin` milliseconds on the document's.
    Document { origin: f64 },
    /// A scroll progress timeline (section 2).
    Scroll { source: Source, axis: Axis },
    /// A view progress timeline of `subject` (section 3), in the scroll
    /// container nearest it; none without a subject.
    View {
        subject: Option<NodeId>,
        axis: Axis,
        inset: Insets<LengthPercentage>,
    },
    /// The timeline named `name` in scope at the element `from` (section
    /// 4.4), the scroll or view timeline an element declares.
    Named { name: Arc<str>, from: NodeId },
}

impl Kind {
    /// Whether it is progress-based, its time a percentage.
    pub fn progress_based(&self) -> bool {
        !matches!(self, Kind::Document { .. })
    }
}

/// What the engine measured of a progress-based timeline at a frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Measured {
    /// Whether it is a view progress timeline, a named one's as the
    /// timeline it names is.
    pub view: bool,
    /// The scroll container it follows, `None` for the viewport.
    pub scroller: Option<NodeId>,
    /// A view timeline's subject.
    pub subject: Option<NodeId>,
    /// Its axis, a named one's that of the timeline it names.
    pub axis: Axis,
    /// The scroll container's offset along the axis, in CSS pixels.
    pub offset: f64,
    /// The offsets at which each named range starts and ends, in
    /// [`RANGES`]' order: a scroll timeline's are all its whole range.
    pub ranges: [(f64, f64); 6],
}

/// The named ranges, in [`Measured::ranges`]' order: `cover` first,
/// which is the timeline's whole range.
pub const RANGES: [RangeName; 6] = [
    RangeName::Cover,
    RangeName::Contain,
    RangeName::Entry,
    RangeName::Exit,
    RangeName::EntryCrossing,
    RangeName::ExitCrossing,
];

impl Measured {
    /// The offsets at which range `name` starts and ends.
    pub fn range(&self, name: RangeName) -> (f64, f64) {
        let i = RANGES.iter().position(|r| *r == name).unwrap_or(0);
        self.ranges[i]
    }

    /// The timeline's current time, a percentage of its whole range;
    /// `None` where the range is empty (section 2.1, an inactive
    /// timeline).
    pub fn current_time(&self) -> Option<f64> {
        let (start, end) = self.ranges[0];
        (end > start).then(|| (self.offset - start) / (end - start) * 100.0)
    }

    /// The percentage of the timeline's whole range at which `edge` is,
    /// `end` for a range's end (where `normal` is 100%): an offset in a
    /// named range, or in the whole of it.
    pub fn edge_percent(&self, edge: &Edge<LengthPercentage>, end: bool) -> f64 {
        let (start, stop) = self.ranges[0];
        let whole = stop - start;
        if whole <= 0.0 {
            return if end { 100.0 } else { 0.0 };
        }
        let at = match edge {
            Edge::Normal | Edge::Auto => return if end { 100.0 } else { 0.0 },
            Edge::At(name, offset) => {
                let (a, b) = name.map_or((start, stop), |n| self.range(n));
                a + f64::from(offset.resolve((b - a) as f32))
            }
        };
        (at - start) / whole * 100.0
    }
}

/// A timeline of the document's.
#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    pub kind: Kind,
    /// What the engine last measured of it, `None` while it is inactive.
    pub measured: Option<Measured>,
}

impl Timeline {
    /// One of `kind`, measured at no frame yet.
    pub fn new(kind: Kind) -> Self {
        Timeline {
            kind,
            measured: None,
        }
    }

    /// Its current time at the document timeline's time `now`, in
    /// milliseconds for a document timeline, a percentage for a
    /// progress-based one; `None` while it is inactive.
    pub fn current_time(&self, now: f64) -> Option<f64> {
        match &self.kind {
            Kind::Document { origin } => Some(now - origin),
            _ => self.measured.as_ref()?.current_time(),
        }
    }
}
