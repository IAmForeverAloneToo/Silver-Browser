//! The top layer as the user reaches it (gap batch H): an Escape's
//! close request, a press's light dismiss, a button's command or popover
//! target, a form's `dialog` method, and interest invokers' delays (HTML
//! sections 4.10.6, 4.11.4, 6.12, 6.13 and 6.14). With a realm, each is
//! a task of the bindings', whose algorithms fire the events; without
//! one, no listener can hear them, and the engine runs the algorithms
//! here without events, over the same state (`dom::Document`'s top
//! layer), as it submits a form for a page whose scripts are off.

use std::time::Duration;

use dom::{
    Dialog, Document, ElementState, NodeId, Popover, PopoverList, PopoverMode, Watcher, forms,
};

use crate::{Engine, focus, scripts};

/// The most popovers one algorithm hides, in case the lists change
/// under it.
const HIDING_ROUNDS: usize = 4096;

/// What the engine keeps of the user's reach into the top layer.
#[derive(Debug, Default)]
pub(crate) struct Overlays {
    /// The popover and the dialog a press began on (HTML's "popover
    /// pointerdown target" and "dialog pointerdown target").
    popover_pressed: Option<NodeId>,
    dialog_pressed: Option<NodeId>,
    /// Whether the user activated the page since a close request was
    /// last canceled: a close request's `cancel` is cancelable only then
    /// (HTML's history-action activation), so that a page cannot keep a
    /// dialog open against every Escape.
    pub(crate) activation: bool,
    /// The interest invokers whose targets have their interest, each with
    /// its target.
    held: Vec<(NodeId, NodeId)>,
    /// The interest to change once its delay passes: when, the invoker,
    /// its target, and whether it is gained.
    pending: Vec<(Duration, NodeId, NodeId, bool)>,
}

/// The state of an element's `popover` attribute, if it has one (an
/// invalid value's is manual).
pub(crate) fn popover_attribute(document: &Document, node: NodeId) -> Option<PopoverMode> {
    if !document.is_html(node) {
        return None;
    }
    let value = document.attr(node, "popover")?;
    Some(match value.to_ascii_lowercase().as_str() {
        "" | "auto" => PopoverMode::Auto,
        "hint" => PopoverMode::Hint,
        _ => PopoverMode::Manual,
    })
}

/// The element an element-reflecting attribute of `node` refers to: one
/// a script set, else the first in its tree with the attribute's value
/// as its ID, shadow roots' reference targets followed, as the bindings
/// take it.
fn named_element(document: &Document, node: NodeId, attribute: &str) -> Option<NodeId> {
    document.referenced_element(node, attribute)
}

/// Whether `node` is a button, or an input that is one, which a
/// `popovertarget` applies to.
fn is_button_like(document: &Document, node: NodeId) -> bool {
    match document.tag(node) {
        Some("button") => true,
        Some("input") => matches!(
            forms::input_type(document, node),
            "button" | "submit" | "reset" | "image"
        ),
        _ => false,
    }
}

/// HTML's "popover target element" of `node`.
pub(crate) fn popover_target(document: &Document, node: NodeId) -> Option<NodeId> {
    if !is_button_like(document, node) || forms::actually_disabled(document, node) {
        return None;
    }
    if forms::form_owner(document, node).is_some() && forms::is_submit_button(document, node) {
        return None;
    }
    named_element(document, node, "popovertarget")
        .filter(|&popover| popover_attribute(document, popover).is_some())
}

/// A `button`'s `commandfor` element, if it names one: none for one of a
/// form whose type is its auto state, which neither submits nor
/// commands, as the reference browser has it.
pub(crate) fn command_target(document: &Document, node: NodeId) -> Option<NodeId> {
    if document.tag(node) != Some("button") || forms::actually_disabled(document, node) {
        return None;
    }
    if forms::button_type_auto(document, node) && forms::form_owner(document, node).is_some() {
        return None;
    }
    named_element(document, node, "commandfor")
}

/// Whether `node` has an invoker's activation behaviour: a button with a
/// `commandfor` element or a popover target.
pub(crate) fn is_invoker(document: &Document, node: NodeId) -> bool {
    command_target(document, node).is_some() || popover_target(document, node).is_some()
}

/// A button's `command`: a built-in one in lowercase, a custom one as
/// written, `None` for none.
fn command_of(document: &Document, node: NodeId) -> Option<String> {
    let value = document.attr(node, "command")?;
    if value.starts_with("--") {
        return Some(value.to_owned());
    }
    let lower = value.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "toggle-popover"
            | "show-popover"
            | "hide-popover"
            | "close"
            | "request-close"
            | "show-modal"
    )
    .then_some(lower)
}

/// An interest invoker's target (HTML section 6.14): an `a`, `area` or
/// `button`, or an SVG `a`, with an `interestfor` that names an element.
fn interest_target(document: &Document, node: NodeId) -> Option<NodeId> {
    let invoker = matches!(document.tag(node), Some("a" | "area" | "button"))
        || document.svg_tag(node) == Some("a");
    if !invoker {
        return None;
    }
    named_element(document, node, "interestfor")
}

/// A dialog's computed closed-by state.
fn closed_by(document: &Document, dialog: NodeId) -> &'static str {
    let value = document
        .attr(dialog, "closedby")
        .unwrap_or("")
        .to_ascii_lowercase();
    match value.as_str() {
        "any" => "any",
        "closerequest" => "closerequest",
        "none" => "none",
        _ if document.dialog(dialog).is_some_and(|d| d.modal) => "closerequest",
        _ => "none",
    }
}

fn is_dialog(document: &Document, node: NodeId) -> bool {
    document.tag(node) == Some("dialog")
}

/// HTML's "nearest inclusive open popover".
fn nearest_open_popover(document: &Document, node: NodeId) -> Option<NodeId> {
    std::iter::once(node)
        .chain(document.flat_ancestors(node))
        .find(|&n| {
            document
                .popover(n)
                .is_some_and(|p| p.mode != PopoverMode::Manual)
        })
}

/// HTML's "topmost popover ancestor" of `node` in `list`, by its parent
/// and `source`; `node` above the list when it is a popover about to show.
fn topmost_popover_ancestor(
    document: &Document,
    node: NodeId,
    list: PopoverList,
    source: Option<NodeId>,
) -> Option<NodeId> {
    let popovers = document.popover_list(list);
    let at = |p: NodeId| popovers.iter().position(|&q| q == p);
    [document.flat_parent(node), source]
        .into_iter()
        .flatten()
        .filter_map(|candidate| nearest_open_popover(document, candidate))
        .filter_map(|ancestor| at(ancestor).map(|i| (i, ancestor)))
        .max_by_key(|&(i, _)| i)
        .map(|(_, ancestor)| ancestor)
}

impl Engine {
    /// Runs a task of the bindings' top layer when the shown document's
    /// scripts run, a realm made for it if its handlers need one: the
    /// task's answer, `None` when no realm runs it.
    fn top_layer_task(&mut self, task: js::Task) -> Option<bool> {
        if !self.scripts_run() || self.shown.made {
            return None;
        }
        let ready = self.ready_state();
        let window = self.window_env();
        let answer = self
            .shown
            .run_top_layer_task(&mut self.network, window, ready, task);
        if answer.is_some() {
            self.scripts_ran();
        }
        answer
    }

    /// Runs `expression` in the shown document as a script of its own, as
    /// the DevTools console does: the tests' way in.
    #[cfg(test)]
    pub(crate) fn evaluate_script(&mut self, expression: &str) {
        let ready = self.ready_state();
        let window = self.window_env();
        self.shown.evaluate(
            &mut self.network,
            window,
            ready,
            expression.to_owned(),
            None,
        );
        self.scripts_ran();
    }

    /// The styles brought up to date with the document.
    fn fresh_styles(&mut self) -> Option<&style::Styles> {
        self.state_changed();
        self.cached_styles();
        self.shown.styled.as_ref().map(|styled| &styled.styles)
    }

    /// The user's close request, an Escape no listener canceled (HTML
    /// section 6.13.2): the topmost close watcher asked to close. Whether
    /// there was one.
    pub(crate) fn close_request(&mut self) -> bool {
        let document = &self.shown.document;
        let Some(group) = document.close_watcher_groups().last().cloned() else {
            return false;
        };
        // A dialog closed by none has its watcher disabled (HTML's "get
        // enabled state"): a group of none but those processes nothing.
        let enabled = |w: &Watcher| match *w {
            Watcher::Element(e) if document.dialog(e).is_some() => closed_by(document, e) != "none",
            _ => true,
        };
        let processed = group.iter().any(enabled);
        let can_prevent = document.can_prevent_close(self.shown.overlays.activation);
        match self.top_layer_task(js::Task::CloseRequest { can_prevent }) {
            Some(canceled) => {
                if canceled {
                    self.shown.overlays.activation = false;
                }
            }
            // With no listener, every enabled watcher of the group closes,
            // the newest first.
            None => {
                for watcher in group.into_iter().rev() {
                    let Watcher::Element(element) = watcher else {
                        continue;
                    };
                    let document = &self.shown.document;
                    if document.dialog(element).is_some() {
                        if closed_by(document, element) != "none" {
                            self.close_dialog(element);
                        }
                    } else if document.popover(element).is_some() {
                        self.hide_popover(element, true);
                    }
                }
            }
        }
        self.shown.document.close_request_done();
        processed
    }

    /// Notes the user's activation of the page (a press, or a key other
    /// than Escape), which a close request's `cancel` may then use.
    pub(crate) fn note_activation(&mut self) {
        self.shown.overlays.activation = true;
        self.shown.document.note_user_activation();
    }

    /// The light dismiss of popovers and dialogs (HTML sections 6.12 and
    /// 4.11.4) after a `pointerdown` (`up` false) or `pointerup` at
    /// `target`, at `(x, y)` in the viewport.
    pub(crate) fn light_dismiss(&mut self, target: NodeId, up: bool, x: f32, y: f32) {
        let document = &self.shown.document;
        if document.popover_list(PopoverList::Auto).is_empty()
            && document.popover_list(PopoverList::Hint).is_empty()
            && document.open_dialogs().next().is_none()
        {
            return;
        }
        let task = js::Task::LightDismiss {
            target: scripts::handle(target),
            up,
            x: f64::from(x),
            y: f64::from(y),
        };
        if self.top_layer_task(task).is_some() {
            return;
        }
        self.light_dismiss_popovers(target, up);
        self.light_dismiss_dialogs(target, up, x, y);
    }

    fn light_dismiss_popovers(&mut self, target: NodeId, up: bool) {
        let document = &self.shown.document;
        if document.popover_list(PopoverList::Auto).is_empty()
            && document.popover_list(PopoverList::Hint).is_empty()
        {
            return;
        }
        let clicked = self.topmost_clicked_popover(target);
        if !up {
            self.shown.overlays.popover_pressed = clicked;
            return;
        }
        let same = clicked == self.shown.overlays.popover_pressed.take();
        if same {
            self.hide_stacked_until(clicked);
        }
    }

    /// HTML's "hide all popovers until" `endpoint` as a press's light
    /// dismiss and a dialog shown run it, the popovers stacked as the
    /// reference browser stacks them, the hint list above the auto list:
    /// until an auto popover, every hint goes too; until a hint, the auto
    /// popovers above the one its list hangs from, all of them when it
    /// hangs from none. Every popover for `None`, or one in neither list.
    fn hide_stacked_until(&mut self, endpoint: Option<NodeId>) {
        let list = endpoint
            .and_then(|e| self.shown.document.popover(e))
            .map(|p| p.list);
        match list {
            Some(PopoverList::Auto) => {
                self.hide_all_popovers_until(endpoint);
                self.close_popover_list(PopoverList::Hint);
            }
            Some(PopoverList::Hint) => {
                self.hide_all_popovers_until(endpoint);
                let document = &self.shown.document;
                let from = document
                    .popover_list(PopoverList::Hint)
                    .first()
                    .and_then(|&bottom| document.popover(bottom))
                    .and_then(|p| p.anchor);
                match from {
                    Some(from) => self.hide_all_popovers_until(Some(from)),
                    None => self.close_popover_list(PopoverList::Auto),
                }
            }
            _ => self.hide_all_popovers_until(None),
        }
    }

    /// HTML's "topmost clicked popover".
    fn topmost_clicked_popover(&self, node: NodeId) -> Option<NodeId> {
        let document = &self.shown.document;
        let clicked = nearest_open_popover(document, node);
        let targeted = std::iter::once(node)
            .chain(document.flat_ancestors(node))
            .filter_map(|n| popover_target(document, n).or_else(|| command_target(document, n)))
            .find(|&p| {
                document
                    .popover(p)
                    .is_some_and(|p| p.mode != PopoverMode::Manual)
            });
        let position = |p: Option<NodeId>| {
            let Some(p) = p else {
                return 0;
            };
            let autos = document.popover_list(PopoverList::Auto);
            let hints = document.popover_list(PopoverList::Hint);
            if let Some(i) = hints.iter().position(|&q| q == p) {
                return autos.len() + i + 1;
            }
            autos.iter().position(|&q| q == p).map_or(0, |i| i + 1)
        };
        if position(clicked) > position(targeted) {
            clicked
        } else {
            targeted
        }
    }

    fn light_dismiss_dialogs(&mut self, target: NodeId, up: bool, x: f32, y: f32) {
        let document = &self.shown.document;
        let Some(topmost) = document.open_dialogs().last() else {
            return;
        };
        // A press on a modal dialog's backdrop, outside its box, is on no
        // dialog (HTML's "nearest clicked dialog").
        let modal = is_dialog(document, target) && document.dialog(target).is_some_and(|d| d.modal);
        let on_backdrop = modal
            && self
                .border_box(target)
                .is_some_and(|(r, _)| x < r.x || y < r.y || x >= r.x + r.w || y >= r.y + r.h);
        let document = &self.shown.document;
        let clicked = if on_backdrop {
            None
        } else {
            std::iter::once(target)
                .chain(document.flat_ancestors(target))
                .find(|&n| is_dialog(document, n) && document.attr(n, "open").is_some())
        };
        if !up {
            self.shown.overlays.dialog_pressed = clicked;
            return;
        }
        let same = clicked == self.shown.overlays.dialog_pressed.take();
        if same && clicked != Some(topmost) && closed_by(document, topmost) == "any" {
            self.close_dialog(topmost);
        }
    }

    /// A button's activation behaviour past its form's, with no listener
    /// (HTML section 4.10.6): its command at its `commandfor` element,
    /// else its popover target toggled, as its `popovertargetaction`
    /// says. Whether it had either.
    pub(crate) fn invoke(&mut self, button: NodeId) -> bool {
        let document = &self.shown.document;
        if let Some(target) = command_target(document, button) {
            self.run_command(button, target);
            return true;
        }
        let Some(popover) = popover_target(document, button) else {
            return false;
        };
        let action = document
            .attr(button, "popovertargetaction")
            .unwrap_or("")
            .to_ascii_lowercase();
        let shown = document.popover(popover).is_some();
        match (action.as_str(), shown) {
            ("show", true) | ("hide", false) => {}
            (_, true) => self.hide_popover(popover, true),
            (_, false) => self.show_popover(popover, Some(button)),
        }
        true
    }

    /// A button's command at `target`, its `command` event aside.
    fn run_command(&mut self, button: NodeId, target: NodeId) {
        let document = &self.shown.document;
        let Some(command) = command_of(document, button) else {
            return;
        };
        let popover = popover_attribute(document, target).is_some();
        let shown = document.popover(target).is_some();
        match command.as_str() {
            "hide-popover" if popover && shown => self.hide_popover(target, true),
            "show-popover" if popover && !shown => self.show_popover(target, Some(button)),
            "toggle-popover" if popover => {
                if shown {
                    self.hide_popover(target, true);
                } else {
                    self.show_popover(target, Some(button));
                }
            }
            "show-modal" if is_dialog(document, target) => {
                if document.attr(target, "open").is_none() && !shown {
                    self.show_modal(target);
                }
            }
            "close" | "request-close" if is_dialog(document, target) => self.close_dialog(target),
            _ => {}
        }
    }

    /// Whether `popover` may be shown (`showing` false) or hidden: HTML's
    /// "check popover validity", with nothing thrown.
    fn popover_valid(&self, popover: NodeId, showing: bool) -> bool {
        let document = &self.shown.document;
        popover_attribute(document, popover).is_some()
            && document.popover(popover).is_some() == showing
            && document.in_page(popover)
            && !(is_dialog(document, popover) && document.dialog(popover).is_some_and(|d| d.modal))
    }

    /// HTML's "show popover", stacked as the bindings' is, no event.
    pub(crate) fn show_popover(&mut self, popover: NodeId, source: Option<NodeId>) {
        if !self.popover_valid(popover, false) {
            return;
        }
        let document = &self.shown.document;
        let Some(mode) = popover_attribute(document, popover) else {
            return;
        };
        let mut list = PopoverList::None;
        let mut anchor = None;
        if mode != PopoverMode::Manual {
            let hint_ancestor =
                topmost_popover_ancestor(document, popover, PopoverList::Hint, source);
            if let Some(hint_ancestor) = hint_ancestor {
                let from = document
                    .popover_list(PopoverList::Hint)
                    .first()
                    .and_then(|&h| document.popover(h))
                    .and_then(|p| p.anchor);
                match from {
                    Some(from) => self.hide_all_popovers_until(Some(from)),
                    None => self.close_popover_list(PopoverList::Auto),
                }
                self.hide_all_popovers_until(Some(hint_ancestor));
                list = PopoverList::Hint;
            } else {
                self.close_popover_list(PopoverList::Hint);
                let document = &self.shown.document;
                let auto_ancestor =
                    topmost_popover_ancestor(document, popover, PopoverList::Auto, source);
                if mode == PopoverMode::Auto {
                    self.hide_all_popovers_until(auto_ancestor);
                    list = PopoverList::Auto;
                } else {
                    anchor = auto_ancestor;
                    list = PopoverList::Hint;
                }
            }
            if !self.popover_valid(popover, false) {
                return;
            }
        }
        let document = &self.shown.document;
        // The focus given back on hiding for the first auto or hint
        // popover alone (HTML's shouldRestoreFocus).
        let first = mode != PopoverMode::Manual
            && document.popover_list(PopoverList::Auto).is_empty()
            && document.popover_list(PopoverList::Hint).is_empty();
        let focused = document.focused();
        self.shown.document.show_popover_state(
            popover,
            Popover {
                mode,
                list,
                invoker: source,
                previously_focused: None,
                anchor,
            },
        );
        self.focusing(popover, false);
        if first {
            self.shown.document.set_popover_focus(popover, focused);
        }
        self.state_changed();
    }

    /// HTML's "hide popover algorithm", no event: those above it hidden
    /// first, and the focus given back when what is in it has it.
    pub(crate) fn hide_popover(&mut self, popover: NodeId, focus_previous: bool) {
        if !self.popover_valid(popover, true) {
            return;
        }
        let Some(state) = self.shown.document.popover(popover).copied() else {
            return;
        };
        if state.mode != PopoverMode::Manual {
            self.hide_all_popovers_until(Some(popover));
        }
        if state.list == PopoverList::Auto {
            let document = &self.shown.document;
            let hangs = document
                .popover_list(PopoverList::Hint)
                .first()
                .and_then(|&h| document.popover(h))
                .is_some_and(|p| p.anchor == Some(popover));
            if hangs {
                self.close_popover_list(PopoverList::Hint);
            }
        }
        let previous = self
            .shown
            .document
            .hide_popover_state(popover)
            .and_then(|p| p.previously_focused);
        let focused = self.shown.document.focused();
        if focus_previous
            && let Some(previous) = previous.filter(|&p| self.shown.document.contains(p))
            && focused.is_some_and(|f| {
                self.shown
                    .document
                    .is_shadow_including_inclusive_ancestor(popover, f)
            })
        {
            self.focus(Some(previous), false, false);
        }
        self.fixup_hidden(popover);
        self.state_changed();
    }

    /// HTML's focus fixup for what was hidden: the focus left in it goes
    /// to the viewport, with no event, as the reference browser has it.
    fn fixup_hidden(&mut self, hidden: NodeId) {
        let document = &mut self.shown.document;
        if document
            .focused()
            .is_some_and(|f| document.is_shadow_including_inclusive_ancestor(hidden, f))
        {
            document.set_focus(None, false);
        }
    }

    /// HTML's "close entire popover list".
    fn close_popover_list(&mut self, list: PopoverList) {
        for _ in 0..HIDING_ROUNDS {
            let Some(&top) = self.shown.document.popover_list(list).last() else {
                return;
            };
            self.hide_popover(top, false);
            if self.shown.document.popover_list(list).last() == Some(&top) {
                return;
            }
        }
    }

    /// HTML's "hide all popovers until" `endpoint`, every popover for
    /// `None`.
    fn hide_all_popovers_until(&mut self, endpoint: Option<NodeId>) {
        let Some(endpoint) = endpoint else {
            self.close_popover_list(PopoverList::Hint);
            self.close_popover_list(PopoverList::Auto);
            return;
        };
        let Some(list) = self.shown.document.popover(endpoint).map(|p| p.list) else {
            return;
        };
        for _ in 0..HIDING_ROUNDS {
            let popovers = self.shown.document.popover_list(list);
            let Some(at) = popovers.iter().position(|&p| p == endpoint) else {
                self.close_popover_list(list);
                return;
            };
            let Some(&top) = popovers.last().filter(|_| at + 1 < popovers.len()) else {
                return;
            };
            self.hide_popover(top, false);
            if self.shown.document.popover_list(list).last() == Some(&top) {
                return;
            }
        }
    }

    /// The popover or dialog focusing steps, no event but the focus's:
    /// a dialog's focus delegate, else the dialog; a popover's, if any.
    fn focusing(&mut self, subject: NodeId, dialog: bool) {
        // A dialog shown as a popover with `autofocus` takes the focus
        // itself, as the reference browser has it.
        let dialog = dialog
            || (is_dialog(&self.shown.document, subject)
                && self.shown.document.attr(subject, "autofocus").is_none());
        let styles = self.fresh_styles().cloned();
        let document = &self.shown.document;
        let control = if dialog {
            focus::focus_delegate(document, styles.as_ref(), subject)
        } else if document.attr(subject, "autofocus").is_some() {
            Some(subject)
        } else {
            focus::autofocus_delegate(document, styles.as_ref(), subject)
        };
        match control {
            Some(control) => self.focus(Some(control), false, false),
            None if dialog => self.focus(Some(subject), false, false),
            None => return,
        }
        // The document's own autofocus is then not done (its autofocus
        // processed flag).
        self.shown.document.set_autofocus_processed();
    }

    /// HTML's "show a modal dialog", no event.
    pub(crate) fn show_modal(&mut self, dialog: NodeId) {
        let document = &self.shown.document;
        if document.attr(dialog, "open").is_some()
            || document.popover(dialog).is_some()
            || !document.in_page(dialog)
        {
            return;
        }
        let focused = document.focused();
        let until = topmost_popover_ancestor(document, dialog, PopoverList::Hint, None)
            .or_else(|| topmost_popover_ancestor(document, dialog, PopoverList::Auto, None));
        let _ = self.shown.document.set_attribute(dialog, "open", "");
        self.shown.document.open_dialog_state(
            dialog,
            Dialog {
                modal: true,
                previously_focused: focused,
            },
        );
        self.hide_stacked_until(until);
        self.focusing(dialog, true);
        self.state_changed();
    }

    /// HTML's "close the dialog", no event: the focus given back where it
    /// was, when it was modal or what is in it has it.
    pub(crate) fn close_dialog(&mut self, dialog: NodeId) {
        let document = &mut self.shown.document;
        if document.attr(dialog, "open").is_none() {
            return;
        }
        // What was kept of it, before the attribute's going lets it go.
        let was = document.close_dialog_state(dialog);
        let _ = document.remove_attribute(dialog, "open");
        let focused = document.focused();
        if let Some(was) = was
            && let Some(previous) = was.previously_focused.filter(|&p| document.contains(p))
            && (was.modal
                || focused.is_some_and(|f| {
                    // Or in it in the flat tree, as an element slotted
                    // into it is, as the reference browser has it.
                    document.is_shadow_including_inclusive_ancestor(dialog, f)
                        || document.flat_ancestors(f).any(|a| a == dialog)
                }))
        {
            self.focus(Some(previous), false, false);
        }
        self.fixup_hidden(dialog);
        self.state_changed();
    }

    /// A form submitted by the `dialog` method (HTML section 4.10.21.3):
    /// the nearest dialog around it closed. Whether the method was it.
    pub(crate) fn submit_to_dialog(&mut self, form: NodeId, submitter: Option<NodeId>) -> bool {
        let document = &self.shown.document;
        let method = submitter
            .and_then(|s| document.attr(s, "formmethod"))
            .or_else(|| document.attr(form, "method"))
            .unwrap_or("");
        if !method.eq_ignore_ascii_case("dialog") {
            return false;
        }
        let dialog = document.ancestors(form).find(|&n| is_dialog(document, n));
        if let Some(dialog) = dialog {
            self.close_dialog(dialog);
        }
        true
    }

    // ---- Interest invokers (HTML section 6.14) ------------------------

    /// When the next change of interest is due.
    pub(crate) fn interest_due(&self) -> Option<Duration> {
        self.shown.overlays.pending.iter().map(|p| p.0).min()
    }

    /// The interest invokers the user's pointer and focus are on now,
    /// and whether each of those with interest still is, scheduled to
    /// gain or lose it after their delays: called as the hover and the
    /// focus change.
    pub(crate) fn interest_changed(&mut self) {
        let document = &self.shown.document;
        if self.shown.overlays.held.is_empty()
            && self.shown.overlays.pending.is_empty()
            && !self
                .shown
                .hovered
                .iter()
                .chain(document.focused().as_ref())
                .any(|&n| interest_target(document, n).is_some())
        {
            return;
        }
        let now = self.now();
        let focused = document.focused();
        let hovered = &self.shown.hovered;
        let mut wanted: Vec<(NodeId, NodeId)> = Vec::new();
        if let Some(found) = hovered
            .iter()
            .find_map(|&n| interest_target(document, n).map(|t| (n, t)))
        {
            wanted.push(found);
        }
        if let Some(found) = focused.and_then(|f| interest_target(document, f).map(|t| (f, t)))
            && !wanted.contains(&found)
        {
            wanted.push(found);
        }
        let styles = self.shown.styled.as_ref().map(|s| &s.styles);
        let delay = |node: NodeId, end: bool| {
            let seconds = styles
                .and_then(|s| s.get(node))
                .map_or(if end { 0.25 } else { 0.5 }, |s| s.interest_delay(end));
            Duration::from_secs_f64(seconds.clamp(0.0, 3600.0))
        };
        let overlays = &mut self.shown.overlays;
        // A wish no longer wanted is dropped.
        overlays
            .pending
            .retain(|&(_, source, target, gain)| !gain || wanted.contains(&(source, target)));
        for &(source, target) in &wanted {
            let held = overlays.held.contains(&(source, target));
            // Pointed at again, it keeps its interest.
            overlays
                .pending
                .retain(|&(_, s, t, gain)| gain || (s, t) != (source, target));
            let pending = overlays
                .pending
                .iter()
                .any(|&(_, s, t, gain)| gain && (s, t) == (source, target));
            if !held && !pending {
                overlays
                    .pending
                    .push((now + delay(source, false), source, target, true));
            }
        }
        // One with interest the user left, invoker and target both, loses
        // it after its delay.
        for &(source, target) in &overlays.held {
            let kept = wanted.contains(&(source, target))
                || hovered.iter().any(|&n| {
                    n == target || document.is_shadow_including_inclusive_ancestor(target, n)
                })
                || focused
                    .is_some_and(|f| document.is_shadow_including_inclusive_ancestor(target, f));
            let pending = overlays
                .pending
                .iter()
                .any(|&(_, s, t, gain)| !gain && (s, t) == (source, target));
            if kept {
                overlays
                    .pending
                    .retain(|&(_, s, t, gain)| gain || (s, t) != (source, target));
            } else if !pending {
                overlays
                    .pending
                    .push((now + delay(source, true), source, target, false));
            }
        }
    }

    /// Runs the changes of interest due: `interest` or `loseinterest`
    /// through the bindings, or with no realm the target's states and its
    /// popover's showing or hiding here.
    pub(crate) fn run_interest(&mut self) {
        let now = self.now();
        let due: Vec<(Duration, NodeId, NodeId, bool)> = self
            .shown
            .overlays
            .pending
            .iter()
            .copied()
            .filter(|p| p.0 <= now)
            .collect();
        self.shown.overlays.pending.retain(|p| p.0 > now);
        for (_, source, target, gained) in due {
            let document = &self.shown.document;
            if !document.contains(source) || !document.contains(target) {
                self.shown.overlays.held.retain(|&(s, _)| s != source);
                continue;
            }
            // A target has one invoker's interest at a time.
            if gained {
                let others: Vec<(NodeId, NodeId)> = self
                    .shown
                    .overlays
                    .held
                    .iter()
                    .copied()
                    .filter(|&(s, t)| t == target && s != source)
                    .collect();
                for (other, _) in others {
                    self.change_interest(other, target, false);
                }
            }
            self.change_interest(source, target, gained);
        }
    }

    /// `source`'s interest in `target` gained or lost now.
    fn change_interest(&mut self, source: NodeId, target: NodeId, gained: bool) {
        let task = js::Task::Interest {
            source: scripts::handle(source),
            target: scripts::handle(target),
            gained,
        };
        let canceled = match self.top_layer_task(task) {
            Some(canceled) => canceled,
            None => {
                let document = &mut self.shown.document;
                let _ = document.set_state(
                    source,
                    document
                        .state(source)
                        .with(ElementState::INTEREST_SOURCE, gained),
                );
                let _ = document.set_state(
                    target,
                    document
                        .state(target)
                        .with(ElementState::INTEREST_TARGET, gained),
                );
                if popover_attribute(document, target).is_some() {
                    let shown = document.popover(target).copied();
                    match shown {
                        None if gained => self.show_popover(target, Some(source)),
                        Some(p) if !gained && p.invoker == Some(source) => {
                            self.hide_popover(target, true)
                        }
                        _ => {}
                    }
                }
                self.state_changed();
                false
            }
        };
        let held = &mut self.shown.overlays.held;
        if canceled {
            return;
        }
        if gained {
            if !held.contains(&(source, target)) {
                held.push((source, target));
            }
        } else {
            held.retain(|&pair| pair != (source, target));
        }
    }
}
