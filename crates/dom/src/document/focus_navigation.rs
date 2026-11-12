//! What the document keeps for focus navigation (gap batch H): its
//! sequential focus navigation starting point (HTML section 6.6.3), the
//! node Tab starts from when it is not the focused element, kept where
//! that node was when it leaves the tree; and the focusgroups (the
//! scoped focusgroup of Open UI's explainer, which the reference browser
//! ships): the `focusgroup` attribute's tokens, the owner each element
//! belongs to, and each owner's last focused item, its memory; and HTML's
//! autofocus candidates (section 6.6.5).

use std::collections::HashMap;

use super::{Boundary, Document, LiveRange, NodeId};

/// Every token the `focusgroup` attribute takes, as the reference
/// browser's `focusGroup.supports()` answers.
pub const FOCUSGROUP_TOKENS: [&str; 22] = [
    "toolbar",
    "tablist",
    "radiogroup",
    "listbox",
    "menu",
    "menubar",
    "wrap",
    "nowrap",
    "inline",
    "block",
    "nomemory",
    "none",
    "feed",
    "grid",
    "itemcontrols",
    "noitemcontrols",
    "manual",
    "rowwrap",
    "colwrap",
    "flow",
    "rowflow",
    "colflow",
];

/// The most focusgroup owners whose memory a document keeps: past it
/// those that left the document go, then the oldest.
const MAX_MEMORIES: usize = 1 << 12;

/// A focusgroup's behavior, its first token that names one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusGroupBehavior {
    Toolbar,
    Tablist,
    Radiogroup,
    Listbox,
    Menu,
    Menubar,
    Grid,
    Feed,
}

/// What an owner's `focusgroup` attribute gives it: the axes the arrow
/// keys move along, whether they wrap at the ends, whether the group
/// remembers its last focused item, and a grid's wrapping and flowing
/// by rows and by columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusGroup {
    pub behavior: FocusGroupBehavior,
    /// Left and Right move the focus.
    pub inline: bool,
    /// Up and Down move it.
    pub block: bool,
    /// Past the last item the first, and before the first the last.
    pub wrap: bool,
    pub memory: bool,
    /// A grid's: past a row's end its start (`rowwrap`), or the next
    /// row's (`rowflow`); past a column's end its start or the next
    /// column's. A conflict of the two leaves the edge hard.
    pub row_wrap: bool,
    pub row_flow: bool,
    pub col_wrap: bool,
    pub col_flow: bool,
}

/// What a `focusgroup` attribute makes of its element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusGroupAttribute {
    /// A focusgroup owner.
    Owner(FocusGroup),
    /// `none` among its tokens: the element and its subtree are taken
    /// out of the focusgroup they are in.
    OptOut,
}

/// The `focusgroup` attribute `value`'s meaning: `none` anywhere opts
/// out; else the first token naming a behavior makes an owner, the
/// others modifying it, and with none the attribute is ignored. Tokens
/// are ASCII case-insensitive; one unknown is passed over.
pub fn parse_focusgroup(value: &str) -> Option<FocusGroupAttribute> {
    let tokens: Vec<String> = value
        .split_ascii_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .collect();
    let has = |name: &str| tokens.iter().any(|t| t == name);
    if has("none") {
        return Some(FocusGroupAttribute::OptOut);
    }
    let behavior = tokens.iter().find_map(|t| {
        Some(match t.as_str() {
            "toolbar" => FocusGroupBehavior::Toolbar,
            "tablist" => FocusGroupBehavior::Tablist,
            "radiogroup" => FocusGroupBehavior::Radiogroup,
            "listbox" => FocusGroupBehavior::Listbox,
            "menu" => FocusGroupBehavior::Menu,
            "menubar" => FocusGroupBehavior::Menubar,
            "grid" => FocusGroupBehavior::Grid,
            "feed" => FocusGroupBehavior::Feed,
            _ => return None,
        })
    })?;
    use FocusGroupBehavior as B;
    // Each behavior's axes and wrapping, as the reference browser gives
    // them: an explicit axis replaces the default ones.
    let (inline, block, wrap) = match behavior {
        B::Toolbar => (true, false, false),
        B::Tablist | B::Menubar => (true, false, true),
        B::Radiogroup => (true, true, true),
        B::Listbox | B::Feed => (false, true, false),
        B::Menu => (false, true, true),
        B::Grid => (true, true, false),
    };
    let (inline, block) = match (has("inline"), has("block")) {
        (false, false) => (inline, block),
        explicit => explicit,
    };
    let wrap = match (has("wrap"), has("nowrap")) {
        (_, true) => false,
        (true, false) => true,
        _ => wrap,
    };
    // A grid's per axis: wrapping and flowing at once, or `nowrap` with
    // flowing, leave that axis's edge hard.
    let nowrap = has("nowrap");
    let axis = |wraps: bool, flows: bool| {
        let wraps = wraps && !nowrap;
        if flows && (wraps || nowrap) {
            (false, false)
        } else {
            (wraps, flows)
        }
    };
    let (row_wrap, row_flow) = axis(has("wrap") || has("rowwrap"), has("flow") || has("rowflow"));
    let (col_wrap, col_flow) = axis(has("wrap") || has("colwrap"), has("flow") || has("colflow"));
    Some(FocusGroupAttribute::Owner(FocusGroup {
        behavior,
        inline,
        block,
        wrap,
        memory: !has("nomemory"),
        row_wrap,
        row_flow,
        col_wrap,
        col_flow,
    }))
}

/// The sequential focus navigation starting point, as the document
/// keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Start {
    /// A node: the element focused last, or what a click or a fragment
    /// set it to.
    Node(NodeId),
    /// Where such a node was when it left the tree: a live range,
    /// collapsed there and moved with the changes around it, and the
    /// slot it was assigned to, whose focus navigation scope it was in.
    Point { range: u32, slot: Option<NodeId> },
}

/// The sequential focus navigation starting point, as Tab reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartingPoint {
    /// A node in the tree.
    Node(NodeId),
    /// The point a node left: before child `offset` of `parent`; `slot`
    /// the slot the node was assigned to, if it was a host's child.
    Point {
        parent: NodeId,
        offset: u32,
        slot: Option<NodeId>,
    },
}

/// The starting point and the focusgroups' memories.
#[derive(Debug, Clone, Default)]
pub(super) struct FocusNavigation {
    start: Option<Start>,
    /// Each owner's last focused item, by owner.
    memory: HashMap<NodeId, NodeId>,
    /// HTML's autofocus candidates (section 6.6.5), in the order they
    /// were inserted, and its "autofocus processed" flag.
    autofocus: Vec<NodeId>,
    autofocus_processed: bool,
}

/// The most autofocus candidates a document keeps: past it the oldest go.
const MAX_AUTOFOCUS: usize = 1 << 12;

impl Document {
    /// What `element`'s `focusgroup` attribute makes of it, if it has one
    /// that means anything.
    pub fn focus_group(&self, element: NodeId) -> Option<FocusGroupAttribute> {
        parse_focusgroup(self.attr(element, "focusgroup")?)
    }

    /// The focusgroup owner `element` belongs to, with its focusgroup:
    /// its nearest ancestor in the flat tree that has a `focusgroup`
    /// attribute meaning anything, when that is an owner. None when that
    /// one opts out, when `element` opts out itself, or when the walk
    /// meets an element in the top layer first, a popover or a modal
    /// dialog shown being out of the focusgroup it is in, unless it is an
    /// owner of its own. An owner is its parent owner's item, not its own.
    pub fn focus_group_owner(&self, element: NodeId) -> Option<(NodeId, FocusGroup)> {
        if !self.is_element(element) {
            return None;
        }
        if matches!(self.focus_group(element), Some(FocusGroupAttribute::OptOut))
            || self.in_top_layer(element)
        {
            return None;
        }
        for node in self.flat_ancestors(element) {
            if !self.is_element(node) {
                continue;
            }
            match self.focus_group(node) {
                Some(FocusGroupAttribute::Owner(group)) => return Some((node, group)),
                Some(FocusGroupAttribute::OptOut) => return None,
                None => {}
            }
            if self.in_top_layer(node) {
                return None;
            }
        }
        None
    }

    /// The item `owner`'s focusgroup remembers as last focused, if it is
    /// still in the document.
    pub fn focus_group_memory(&self, owner: NodeId) -> Option<NodeId> {
        self.rare
            .focus
            .memory
            .get(&owner)
            .copied()
            .filter(|&item| self.contains(item))
    }

    /// `element` took the focus: it is the starting point, and the item
    /// its focusgroup remembers when that has memory.
    pub(super) fn focus_navigation_focused(&mut self, element: NodeId) {
        self.set_starting_point(element);
        if let Some((owner, group)) = self.focus_group_owner(element)
            && group.memory
        {
            let memory = &mut self.rare.focus.memory;
            if memory.len() >= MAX_MEMORIES && !memory.contains_key(&owner) {
                let gone: Vec<NodeId> = memory.keys().copied().collect();
                for key in gone {
                    if !self.contains(key) {
                        self.rare.focus.memory.remove(&key);
                    }
                }
                let memory = &mut self.rare.focus.memory;
                if memory.len() >= MAX_MEMORIES
                    && let Some(&any) = memory.keys().next()
                {
                    memory.remove(&any);
                }
            }
            self.rare.focus.memory.insert(owner, element);
        }
    }

    /// After the subtree at `top` went into the page's tree: each element
    /// with `autofocus` in it is an autofocus candidate, put last if it
    /// was one already, until the document has processed autofocus (HTML
    /// section 6.6.5's "autofocus steps" of the insertion steps).
    pub(super) fn autofocus_inserted(&mut self, top: NodeId) {
        crate::hook!("dom::focus::autofocus_inserted");
        if self.rare.focus.autofocus_processed {
            return;
        }
        let found: Vec<NodeId> = self
            .subtree(top)
            .filter(|&node| self.is_element(node) && self.attr(node, "autofocus").is_some())
            .collect();
        let candidates = &mut self.rare.focus.autofocus;
        for element in found {
            candidates.retain(|&c| c != element);
            if candidates.len() >= MAX_AUTOFOCUS {
                candidates.remove(0);
            }
            candidates.push(element);
        }
    }

    /// The autofocus candidates, oldest first, those still in the page.
    pub fn autofocus_candidates(&self) -> Vec<NodeId> {
        self.rare
            .focus
            .autofocus
            .iter()
            .copied()
            .filter(|&node| self.in_page(node))
            .collect()
    }

    /// Takes `element` out of the autofocus candidates.
    pub fn drop_autofocus_candidate(&mut self, element: NodeId) {
        self.rare.focus.autofocus.retain(|&c| c != element);
    }

    /// Whether the document has processed autofocus: an element took the
    /// focus by it, or something had it when the candidates were flushed.
    pub fn autofocus_processed(&self) -> bool {
        self.rare.focus.autofocus_processed
    }

    /// The document has processed autofocus: its candidates go, and no
    /// more are taken.
    pub fn set_autofocus_processed(&mut self) {
        self.rare.focus.autofocus.clear();
        self.rare.focus.autofocus_processed = true;
    }

    /// The sequential focus navigation starting point, if there is one
    /// still in the document.
    pub fn starting_point(&self) -> Option<StartingPoint> {
        match self.rare.focus.start? {
            Start::Node(node) => self.is_connected(node).then_some(StartingPoint::Node(node)),
            Start::Point { range, slot } => {
                let LiveRange { start, .. } = self.range(range)?;
                self.is_connected(start.node)
                    .then_some(StartingPoint::Point {
                        parent: start.node,
                        offset: start.offset,
                        slot: slot.filter(|&slot| self.is_connected(slot)),
                    })
            }
        }
    }

    /// Sets the sequential focus navigation starting point to `node`: a
    /// click's target, a fragment's, or the element focused.
    pub fn set_starting_point(&mut self, node: NodeId) {
        if let Some(Start::Point { range, .. }) = self.rare.focus.start {
            self.drop_range(range);
        }
        self.rare.focus.start = Some(Start::Node(node));
    }

    /// Clears the sequential focus navigation starting point: the focus
    /// left the page, which Tab comes back to from its start or end.
    pub fn clear_starting_point(&mut self) {
        if let Some(Start::Point { range, .. }) = self.rare.focus.start.take() {
            self.drop_range(range);
        }
    }

    /// Before `child` leaves `parent`, at `index`: a starting point in it
    /// is kept where it was, in the scope of the slot it was assigned to.
    pub(super) fn starting_point_leaving(&mut self, parent: NodeId, child: NodeId, index: u32) {
        crate::hook!("dom::focus::starting_point_leaving");
        let Some(Start::Node(node)) = self.rare.focus.start else {
            return;
        };
        if !self.is_shadow_including_inclusive_ancestor(child, node) {
            return;
        }
        let slot = self.assigned_slot(child);
        self.rare.focus.start = None;
        let Ok(range) = self.create_range(parent) else {
            return;
        };
        let at = Boundary {
            node: parent,
            offset: index,
        };
        if self
            .set_range(range, LiveRange { start: at, end: at })
            .is_err()
        {
            self.drop_range(range);
            return;
        }
        self.rare.focus.start = Some(Start::Point { range, slot });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(value: &str) -> FocusGroup {
        match parse_focusgroup(value) {
            Some(FocusGroupAttribute::Owner(group)) => group,
            other => panic!("{value}: {other:?}"),
        }
    }

    #[test]
    fn a_behavior_token_anywhere_makes_an_owner() {
        assert_eq!(owner("wrap toolbar").behavior, FocusGroupBehavior::Toolbar);
        assert_eq!(
            owner("invalid TabList").behavior,
            FocusGroupBehavior::Tablist
        );
        assert_eq!(parse_focusgroup(""), None);
        assert_eq!(parse_focusgroup("inline wrap"), None);
        assert_eq!(
            parse_focusgroup("toolbar wrap none"),
            Some(FocusGroupAttribute::OptOut)
        );
    }

    #[test]
    fn behaviors_give_axes_and_wrapping() {
        let toolbar = owner("toolbar");
        assert!(toolbar.inline && !toolbar.block && !toolbar.wrap && toolbar.memory);
        let menu = owner("menu");
        assert!(!menu.inline && menu.block && menu.wrap);
        let both = owner("toolbar inline block wrap nomemory");
        assert!(both.inline && both.block && both.wrap && !both.memory);
        assert!(!owner("menu nowrap").wrap);
        let radios = owner("radiogroup inline");
        assert!(radios.inline && !radios.block && radios.wrap);
        let feed = owner("feed");
        assert!(feed.block && !feed.inline && !feed.wrap);
    }

    #[test]
    fn grid_conflicts_leave_hard_edges() {
        let grid = owner("grid wrap rowflow");
        assert!(!grid.row_wrap && !grid.row_flow && grid.col_wrap && !grid.col_flow);
        let grid = owner("grid nowrap flow");
        assert!(!grid.row_wrap && !grid.row_flow && !grid.col_wrap && !grid.col_flow);
        let grid = owner("grid colwrap rowflow");
        assert!(!grid.row_wrap && grid.row_flow && grid.col_wrap && !grid.col_flow);
        let grid = owner("grid row-wrap col-wrap");
        assert!(!grid.row_wrap && !grid.col_wrap);
    }
}
