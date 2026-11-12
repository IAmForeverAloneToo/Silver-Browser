//! The top layer and what is kept of the elements it holds (gap batch
//! H): the document's top layer (CSS Position 4 section 4), its showing
//! popovers and their two lists, auto and hint (HTML section 6.12), its
//! open dialogs (HTML section 4.11.4) and the close watchers they and
//! scripts' `CloseWatcher`s make (HTML section 6.13). The algorithms
//! that open and close them, with their events, are the bindings' and,
//! for a page whose scripts are off, the engine's; both keep their state
//! here, where the cascade, layout and hit testing read it.
//!
//! An element that leaves the document leaves all of it at once,
//! without events, as HTML's removing steps for a popover and a dialog
//! say: a moved one (`moveBefore`) keeps it.

use super::{Change, Document, ElementState, NodeId};

/// The most elements the top layer, the popover lists and the open
/// dialogs hold each, and the most close watchers: past it the oldest
/// go, so that a script showing popovers in a loop holds the document's
/// memory within a bound.
pub const MAX_TOP_LAYER: usize = 1 << 12;

/// How a popover was opened (HTML's "opened in popover mode"): the
/// state of its `popover` attribute when it was shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopoverMode {
    Auto,
    Hint,
    Manual,
}

/// The list a showing popover is in, as its mode and nesting put it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopoverList {
    /// The showing auto popover list.
    Auto,
    /// The showing hint popover list.
    Hint,
    /// Neither: a manual popover.
    None,
}

/// What is kept of a showing popover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Popover {
    pub mode: PopoverMode,
    pub list: PopoverList,
    /// Its popover invoker: the element that showed it, if one did.
    pub invoker: Option<NodeId>,
    /// The element focused before it was shown, given the focus back
    /// when it hides, if it was the first of the auto and hint popovers.
    pub previously_focused: Option<NodeId>,
    /// For a hint popover at the bottom of the hint list, the auto
    /// popover it was shown inside, whose hiding closes the hint list,
    /// as the reference browser has it.
    pub anchor: Option<NodeId>,
}

/// What is kept of an open dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dialog {
    /// Shown by `showModal()`.
    pub modal: bool,
    /// The element focused before it was shown.
    pub previously_focused: Option<NodeId>,
}

/// A close watcher (HTML section 6.13): a dialog's or a popover's, by
/// its element, or a script's `CloseWatcher`, by its number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Watcher {
    Element(NodeId),
    Script(u32),
}

/// A document's top layer and what goes with it.
#[derive(Debug, Clone)]
pub(super) struct TopLayer {
    /// The top layer, bottom first.
    order: Vec<NodeId>,
    /// Each showing popover.
    popovers: Vec<(NodeId, Popover)>,
    /// The showing auto popover list and the showing hint popover list,
    /// bottom first.
    auto: Vec<NodeId>,
    hint: Vec<NodeId>,
    /// The open dialogs, in the order they were opened.
    dialogs: Vec<(NodeId, Dialog)>,
    /// The window's close watcher manager (HTML section 6.13.1): its
    /// groups of close watchers, oldest first, each group's oldest first;
    /// how many groups it allows; and whether the next user activation
    /// allows one more.
    groups: Vec<Vec<Watcher>>,
    allowed_groups: usize,
    next_allows_group: bool,
    /// Whether an HTML `dialog` was ever given an `open` attribute, which
    /// an insertion then has to look for.
    open_dialogs_made: bool,
}

impl Default for TopLayer {
    fn default() -> Self {
        TopLayer {
            order: Vec::new(),
            popovers: Vec::new(),
            auto: Vec::new(),
            hint: Vec::new(),
            dialogs: Vec::new(),
            groups: Vec::new(),
            allowed_groups: 1,
            next_allows_group: true,
            open_dialogs_made: false,
        }
    }
}

impl TopLayer {
    fn is_empty(&self) -> bool {
        self.order.is_empty()
            && self.popovers.is_empty()
            && self.dialogs.is_empty()
            && self.groups.is_empty()
    }

    /// HTML's "establish a close watcher": a group of its own while the
    /// manager allows one more, else in the last group; the next user
    /// activation then allows another. Past [`MAX_TOP_LAYER`] watchers
    /// the oldest goes.
    fn establish(&mut self, watcher: Watcher) {
        self.forget(watcher);
        if self.groups.iter().map(Vec::len).sum::<usize>() >= MAX_TOP_LAYER
            && let Some(&oldest) = self.groups.first().and_then(|g| g.first())
        {
            self.forget(oldest);
        }
        let room = self.groups.len() < self.allowed_groups;
        match self.groups.last_mut() {
            Some(last) if !room => last.push(watcher),
            _ => self.groups.push(vec![watcher]),
        }
        self.next_allows_group = true;
    }

    /// A close watcher destroyed: out of its group, an empty group gone.
    fn forget(&mut self, watcher: Watcher) {
        for group in &mut self.groups {
            group.retain(|&w| w != watcher);
        }
        self.groups.retain(|group| !group.is_empty());
    }
}

/// Pushes `item` on `list`, the oldest going past [`MAX_TOP_LAYER`].
fn push_bounded<T>(list: &mut Vec<T>, item: T) {
    if list.len() >= MAX_TOP_LAYER {
        list.remove(0);
    }
    list.push(item);
}

impl Document {
    /// The top layer, bottom first: what is rendered above the rest of
    /// the document, in this order.
    pub fn top_layer(&self) -> &[NodeId] {
        &self.rare.top_layer.order
    }

    /// Whether `element` is in the top layer.
    pub fn in_top_layer(&self, element: NodeId) -> bool {
        self.state(element).contains(ElementState::TOP_LAYER)
    }

    /// The topmost dialog shown by `showModal()`, if any: what blocks the
    /// rest of the document (HTML's "blocked by a modal dialog"), every
    /// node but it and what is in it being inert.
    pub fn topmost_modal(&self) -> Option<NodeId> {
        self.rare
            .top_layer
            .order
            .iter()
            .rev()
            .copied()
            .find(|&element| self.state(element).contains(ElementState::MODAL))
    }

    /// Adds `element` to the top layer, at its top: taken out first if
    /// it is in it (CSS Position 4's "add an element to the top layer").
    /// Nothing for a node that is not an element.
    pub fn add_to_top_layer(&mut self, element: NodeId) {
        if !self.is_element(element) {
            return;
        }
        let order = &mut self.rare.top_layer.order;
        order.retain(|&e| e != element);
        if order.len() >= MAX_TOP_LAYER {
            let gone = order.remove(0);
            let _ = self.set_state(gone, self.state(gone).with(ElementState::TOP_LAYER, false));
        }
        self.rare.top_layer.order.push(element);
        let _ = self.set_state(
            element,
            self.state(element).with(ElementState::TOP_LAYER, true),
        );
    }

    /// Takes `element` out of the top layer, if it is in it.
    pub fn remove_from_top_layer(&mut self, element: NodeId) {
        self.rare.top_layer.order.retain(|&e| e != element);
        let _ = self.set_state(
            element,
            self.state(element).with(ElementState::TOP_LAYER, false),
        );
    }

    /// A showing popover's state; `None` for a hidden one.
    pub fn popover(&self, element: NodeId) -> Option<&Popover> {
        self.rare
            .top_layer
            .popovers
            .iter()
            .find(|(e, _)| *e == element)
            .map(|(_, p)| p)
    }

    /// The showing auto popover list (`Auto`) or hint popover list, bottom
    /// first.
    pub fn popover_list(&self, list: PopoverList) -> &[NodeId] {
        match list {
            PopoverList::Auto => &self.rare.top_layer.auto,
            PopoverList::Hint => &self.rare.top_layer.hint,
            PopoverList::None => &[],
        }
    }

    /// `element` is shown as a popover: kept with `popover`, put at the
    /// top of its list and of the top layer, `:popover-open`, and a
    /// close watcher made for it unless it is manual. One showing
    /// already is hidden first.
    pub fn show_popover_state(&mut self, element: NodeId, popover: Popover) {
        if !self.is_element(element) {
            return;
        }
        self.hide_popover_state(element);
        let layer = &mut self.rare.top_layer;
        push_bounded(&mut layer.popovers, (element, popover));
        match popover.list {
            PopoverList::Auto => push_bounded(&mut layer.auto, element),
            PopoverList::Hint => push_bounded(&mut layer.hint, element),
            PopoverList::None => {}
        }
        if popover.mode != PopoverMode::Manual {
            layer.establish(Watcher::Element(element));
        }
        self.add_to_top_layer(element);
        let _ = self.set_state(
            element,
            self.state(element).with(ElementState::POPOVER_OPEN, true),
        );
    }

    /// `element` is hidden as a popover: out of its list, the top layer
    /// and the close watchers. What was kept of it, `None` when it was
    /// not showing.
    pub fn hide_popover_state(&mut self, element: NodeId) -> Option<Popover> {
        let layer = &mut self.rare.top_layer;
        let at = layer.popovers.iter().position(|(e, _)| *e == element)?;
        let (_, popover) = layer.popovers.remove(at);
        layer.auto.retain(|&e| e != element);
        layer.hint.retain(|&e| e != element);
        layer.forget(Watcher::Element(element));
        self.remove_from_top_layer(element);
        let _ = self.set_state(
            element,
            self.state(element).with(ElementState::POPOVER_OPEN, false),
        );
        Some(popover)
    }

    /// Whether `element` is an HTML `dialog`.
    fn is_html_dialog(&self, element: NodeId) -> bool {
        self.tag(element) == Some("dialog") && self.is_html(element)
    }

    /// A change to the document: HTML's dialog setup steps for a `dialog`
    /// that comes to be open in the page's document other than by
    /// `show()` and `showModal()`, as one the parser opens or an `open`
    /// attribute set (section 4.11.4): in the open dialogs, non-modal,
    /// with a close watcher; and its cleanup steps when its `open` goes.
    pub(super) fn dialogs_changed(&mut self, change: &Change) {
        match change {
            Change::AttributeChanged { element, name, old }
                if name == "open" && self.is_html_dialog(*element) =>
            {
                let open = self.attr(*element, "open").is_some();
                if open {
                    self.rare.top_layer.open_dialogs_made = true;
                }
                match (old.is_some(), open) {
                    (false, true) => self.dialog_setup(*element),
                    (true, false) => {
                        self.close_dialog_state(*element);
                    }
                    _ => {}
                }
            }
            Change::Inserted { child, .. } if self.rare.top_layer.open_dialogs_made => {
                let inserted: Vec<NodeId> = self
                    .shadow_including_subtree(*child)
                    .into_iter()
                    .filter(|&n| self.is_html_dialog(n) && self.attr(n, "open").is_some())
                    .collect();
                for element in inserted {
                    self.dialog_setup(element);
                }
            }
            _ => {}
        }
    }

    /// HTML's "dialog setup steps": an open `dialog` in the page's
    /// document and not yet in its open dialogs is put there.
    fn dialog_setup(&mut self, element: NodeId) {
        if self.dialog(element).is_some() || !self.in_page(element) {
            return;
        }
        self.open_dialog_state(
            element,
            Dialog {
                modal: false,
                previously_focused: None,
            },
        );
    }

    /// An HTML `dialog` made with its attributes, as a parser or a clone
    /// makes one: an open one is looked for when it is inserted.
    pub(super) fn dialog_made(&mut self, element: NodeId) {
        if self.is_html_dialog(element) && self.attr(element, "open").is_some() {
            self.rare.top_layer.open_dialogs_made = true;
        }
    }

    /// Sets a showing popover's previously focused element.
    pub fn set_popover_focus(&mut self, element: NodeId, previously_focused: Option<NodeId>) {
        if let Some((_, popover)) = self
            .rare
            .top_layer
            .popovers
            .iter_mut()
            .find(|(e, _)| *e == element)
        {
            popover.previously_focused = previously_focused;
        }
    }

    /// An open dialog's state; `None` for a closed one.
    pub fn dialog(&self, element: NodeId) -> Option<&Dialog> {
        self.rare
            .top_layer
            .dialogs
            .iter()
            .find(|(e, _)| *e == element)
            .map(|(_, d)| d)
    }

    /// The open dialogs, in the order they were opened.
    pub fn open_dialogs(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.rare.top_layer.dialogs.iter().map(|(e, _)| *e)
    }

    /// `element`, a dialog, is opened: kept with `dialog`, given a close
    /// watcher, and when modal put at the top of the top layer and made
    /// `:modal`. One open already is closed first.
    pub fn open_dialog_state(&mut self, element: NodeId, dialog: Dialog) {
        if !self.is_element(element) {
            return;
        }
        self.close_dialog_state(element);
        let layer = &mut self.rare.top_layer;
        push_bounded(&mut layer.dialogs, (element, dialog));
        layer.establish(Watcher::Element(element));
        if dialog.modal {
            self.add_to_top_layer(element);
            let _ = self.set_state(element, self.state(element).with(ElementState::MODAL, true));
        }
    }

    /// `element`, a dialog, is closed: out of the open dialogs, the close
    /// watchers and the top layer, and no longer `:modal`. What was kept
    /// of it, `None` when it was not open.
    pub fn close_dialog_state(&mut self, element: NodeId) -> Option<Dialog> {
        let layer = &mut self.rare.top_layer;
        let at = layer.dialogs.iter().position(|(e, _)| *e == element)?;
        let (_, dialog) = layer.dialogs.remove(at);
        layer.forget(Watcher::Element(element));
        if self.popover(element).is_none() {
            self.remove_from_top_layer(element);
        }
        let _ = self.set_state(
            element,
            self.state(element).with(ElementState::MODAL, false),
        );
        Some(dialog)
    }

    /// The close watchers, oldest first, their groups run together.
    pub fn close_watchers(&self) -> Vec<Watcher> {
        self.rare.top_layer.groups.concat()
    }

    /// The close watcher manager's groups (HTML section 6.13.1), oldest
    /// first: what a close request asks of is the last.
    pub fn close_watcher_groups(&self) -> &[Vec<Watcher>] {
        &self.rare.top_layer.groups
    }

    /// A script's close watcher `id` is made: established as HTML's
    /// "establish a close watcher" says.
    pub fn add_close_watcher(&mut self, id: u32) {
        self.rare.top_layer.establish(Watcher::Script(id));
    }

    /// A script's close watcher `id` is destroyed.
    pub fn remove_close_watcher(&mut self, id: u32) {
        self.rare.top_layer.forget(Watcher::Script(id));
    }

    /// HTML's "notify the close watcher manager about user activation":
    /// the first activation since a watcher was established allows one
    /// more group.
    pub fn note_user_activation(&mut self) {
        let layer = &mut self.rare.top_layer;
        if layer.next_allows_group {
            layer.allowed_groups = layer.allowed_groups.saturating_add(1).min(MAX_TOP_LAYER);
        }
        layer.next_allows_group = false;
    }

    /// Whether a close request's `cancel` may be canceled (HTML's "request
    /// to close", `canPreventClose`): the manager has fewer groups than it
    /// allows and the window has history-action activation, `activation`.
    pub fn can_prevent_close(&self, activation: bool) -> bool {
        let layer = &self.rare.top_layer;
        activation && layer.groups.len() < layer.allowed_groups
    }

    /// The end of HTML's "process close watchers": one group fewer
    /// allowed, but always one.
    pub fn close_request_done(&mut self) {
        let layer = &mut self.rare.top_layer;
        if layer.allowed_groups > 1 {
            layer.allowed_groups -= 1;
        }
    }

    /// What leaving the document does to `removed`'s subtree, taken out
    /// of the page's tree (HTML's removing steps for a popover and a
    /// dialog): the popovers in it hidden and the dialogs in it out of
    /// the top layer and the open dialogs, without events; a dialog keeps
    /// its `open` attribute.
    pub(super) fn top_layer_removing(&mut self, removed: NodeId) {
        crate::hook!("dom::top_layer::top_layer_removing");
        if self.rare.top_layer.is_empty() {
            return;
        }
        let inside = |document: &Document, element: NodeId| {
            document.is_shadow_including_inclusive_ancestor(removed, element)
        };
        let popovers: Vec<NodeId> = self
            .rare
            .top_layer
            .popovers
            .iter()
            .map(|(e, _)| *e)
            .filter(|&e| inside(self, e))
            .collect();
        for element in popovers {
            self.hide_popover_state(element);
        }
        let dialogs: Vec<NodeId> = self.open_dialogs().filter(|&e| inside(self, e)).collect();
        for element in dialogs {
            self.close_dialog_state(element);
        }
        let left: Vec<NodeId> = self
            .rare
            .top_layer
            .order
            .iter()
            .copied()
            .filter(|&e| inside(self, e))
            .collect();
        for element in left {
            self.remove_from_top_layer(element);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Change, Element, Node};

    /// A body holding a dialog with a button in it, two divs and a
    /// section holding a div.
    fn page() -> (Document, [NodeId; 5]) {
        let body = Element::new("body")
            .push(Element::new("dialog").push(Element::new("button")))
            .push(Element::new("div"))
            .push(Element::new("div"))
            .push(Element::new("section").push(Element::new("div")));
        let d = Document::from_node(&Node::document(vec![
            Element::new("html").push(body).into(),
        ]));
        let html = d.document_element().unwrap();
        let body = d.first_child(html).unwrap();
        let kids: Vec<NodeId> = d.element_children(body).collect();
        let inner = d.first_child(kids[3]).unwrap();
        (d, [kids[0], kids[1], kids[2], kids[3], inner])
    }

    fn auto(invoker: Option<NodeId>) -> Popover {
        Popover {
            mode: PopoverMode::Auto,
            list: PopoverList::Auto,
            invoker,
            previously_focused: None,
            anchor: None,
        }
    }

    #[test]
    fn an_element_added_again_goes_to_the_top_and_its_flag_is_journaled() {
        let (mut d, [_, a, b, ..]) = page();
        d.take_changes();
        d.add_to_top_layer(a);
        d.add_to_top_layer(b);
        d.add_to_top_layer(a);
        assert_eq!(d.top_layer(), [b, a]);
        assert!(d.in_top_layer(a) && d.in_top_layer(b));
        assert!(d.take_changes().contains(&Change::StateChanged {
            element: a,
            old: ElementState::NONE
        }));
        d.remove_from_top_layer(a);
        assert_eq!(d.top_layer(), [b]);
        assert!(!d.in_top_layer(a));
        // Only an element goes in.
        let root = d.root();
        d.add_to_top_layer(root);
        assert_eq!(d.top_layer(), [b]);
    }

    #[test]
    fn a_popover_shown_is_in_its_list_the_top_layer_and_the_close_watchers() {
        let (mut d, [_, a, b, section, _]) = page();
        d.show_popover_state(a, auto(Some(section)));
        let hint = Popover {
            mode: PopoverMode::Hint,
            list: PopoverList::Hint,
            ..auto(None)
        };
        d.show_popover_state(b, hint);
        let manual = Popover {
            mode: PopoverMode::Manual,
            list: PopoverList::None,
            ..auto(None)
        };
        d.show_popover_state(section, manual);
        assert_eq!(d.popover_list(PopoverList::Auto), [a]);
        assert_eq!(d.popover_list(PopoverList::Hint), [b]);
        assert_eq!(d.top_layer(), [a, b, section]);
        // A manual popover has no close watcher.
        assert_eq!(
            d.close_watchers(),
            [Watcher::Element(a), Watcher::Element(b)]
        );
        assert!(d.state(a).contains(ElementState::POPOVER_OPEN));
        assert_eq!(d.popover(a).and_then(|p| p.invoker), Some(section));
        assert_eq!(d.hide_popover_state(a), Some(auto(Some(section))));
        assert_eq!(d.hide_popover_state(a), None);
        assert!(!d.state(a).contains(ElementState::POPOVER_OPEN));
        assert!(!d.in_top_layer(a));
        assert!(d.popover_list(PopoverList::Auto).is_empty());
        assert_eq!(d.close_watchers(), [Watcher::Element(b)]);
    }

    #[test]
    fn a_modal_dialog_is_in_the_top_layer_and_a_shown_one_is_not() {
        let (mut d, [dialog, a, ..]) = page();
        let shown = Dialog {
            modal: false,
            previously_focused: Some(a),
        };
        d.open_dialog_state(dialog, shown);
        assert_eq!(d.dialog(dialog), Some(&shown));
        assert!(d.top_layer().is_empty());
        assert_eq!(d.topmost_modal(), None);
        d.close_dialog_state(dialog);
        let modal = Dialog {
            modal: true,
            previously_focused: None,
        };
        d.open_dialog_state(dialog, modal);
        d.show_popover_state(a, auto(None));
        assert_eq!(d.top_layer(), [dialog, a]);
        assert_eq!(d.topmost_modal(), Some(dialog));
        assert!(d.state(dialog).contains(ElementState::MODAL));
        assert_eq!(d.open_dialogs().collect::<Vec<_>>(), [dialog]);
        assert_eq!(d.close_dialog_state(dialog), Some(modal));
        assert!(!d.state(dialog).contains(ElementState::MODAL));
        assert_eq!(d.top_layer(), [a]);
        assert_eq!(d.topmost_modal(), None);
    }

    #[test]
    fn a_script_close_watcher_is_kept_by_its_number() {
        let (mut d, [_, a, ..]) = page();
        d.add_close_watcher(7);
        d.show_popover_state(a, auto(None));
        d.add_close_watcher(7);
        assert_eq!(
            d.close_watchers(),
            [Watcher::Element(a), Watcher::Script(7)]
        );
        d.remove_close_watcher(7);
        assert_eq!(d.close_watchers(), [Watcher::Element(a)]);
    }

    #[test]
    fn leaving_the_document_hides_and_closes_without_a_move_doing_so() {
        let (mut d, [dialog, a, b, section, inner]) = page();
        d.open_dialog_state(
            dialog,
            Dialog {
                modal: true,
                previously_focused: None,
            },
        );
        d.show_popover_state(inner, auto(None));
        d.show_popover_state(a, auto(None));
        // Moved, it keeps its state.
        let body = d.parent(a).unwrap();
        d.move_before(body, inner, Some(b)).unwrap();
        assert!(d.popover(inner).is_some());
        d.move_before(section, inner, None).unwrap();
        // The section taken out takes the popover in it.
        d.remove_child(body, section).unwrap();
        assert!(d.popover(inner).is_none());
        assert!(!d.state(inner).contains(ElementState::POPOVER_OPEN));
        assert_eq!(d.popover_list(PopoverList::Auto), [a]);
        d.remove_child(body, dialog).unwrap();
        assert!(d.dialog(dialog).is_none());
        assert!(!d.state(dialog).contains(ElementState::MODAL));
        assert_eq!(d.top_layer(), [a]);
        assert_eq!(d.close_watchers(), [Watcher::Element(a)]);
    }

    #[test]
    fn close_watchers_are_grouped_by_the_users_activation() {
        let (mut d, [dialog, a, b, section, _]) = page();
        // Made with no activation between, they share a group.
        d.show_popover_state(a, auto(None));
        d.add_close_watcher(1);
        assert_eq!(
            d.close_watcher_groups(),
            [vec![Watcher::Element(a), Watcher::Script(1)]]
        );
        assert!(!d.can_prevent_close(true), "as many groups as allowed");
        // An activation allows one more, once.
        d.note_user_activation();
        d.note_user_activation();
        assert!(d.can_prevent_close(true) && !d.can_prevent_close(false));
        d.open_dialog_state(
            dialog,
            Dialog {
                modal: true,
                previously_focused: None,
            },
        );
        d.add_close_watcher(2);
        assert_eq!(
            d.close_watcher_groups(),
            [
                vec![Watcher::Element(a), Watcher::Script(1)],
                vec![Watcher::Element(dialog), Watcher::Script(2)]
            ]
        );
        // A close request takes one group's allowance back: the next
        // activation gives it again, and no more groups than allowed.
        d.close_request_done();
        d.note_user_activation();
        d.show_popover_state(b, auto(None));
        // A group emptied goes.
        d.remove_close_watcher(1);
        d.hide_popover_state(a);
        assert_eq!(
            d.close_watcher_groups(),
            [vec![
                Watcher::Element(dialog),
                Watcher::Script(2),
                Watcher::Element(b)
            ]]
        );
        // The allowance never falls below one group.
        for _ in 0..4 {
            d.close_request_done();
        }
        d.note_user_activation();
        d.show_popover_state(section, auto(None));
        assert_eq!(d.close_watcher_groups().len(), 2);
    }

    #[test]
    fn the_lists_hold_at_most_their_bound() {
        let (mut d, [_, root, ..]) = page();
        let mut made = Vec::new();
        for _ in 0..MAX_TOP_LAYER + 2 {
            let e = d.create_element("div").unwrap();
            d.append_child(root, e).unwrap();
            d.show_popover_state(e, auto(None));
            made.push(e);
        }
        assert_eq!(d.top_layer().len(), MAX_TOP_LAYER);
        assert_eq!(d.popover_list(PopoverList::Auto).len(), MAX_TOP_LAYER);
        assert!(!d.in_top_layer(made[0]));
        assert!(d.in_top_layer(made[MAX_TOP_LAYER + 1]));
    }
}
