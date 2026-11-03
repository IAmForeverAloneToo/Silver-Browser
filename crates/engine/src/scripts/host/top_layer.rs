//! The top layer's calls (gap batch H): what the bindings' algorithms
//! for dialogs, popovers, close watchers and interest invokers read and
//! change of the document's top layer (`dom::Document`'s), each an op of
//! [`js::Call::TopLayer`]. The algorithms are the bindings'; this keeps
//! their state where the cascade, layout and hit testing read it, and
//! answers what only the engine knows: what a focusing focuses.

use dom::{Dialog, ElementState, NodeId, Popover, PopoverList, PopoverMode, Watcher};
use js::{Answer, DomError, Handle};

use super::DocumentHost;
use crate::scripts::handle;

/// A list's or a mode's name, as the bindings write it.
fn list_name(list: PopoverList) -> &'static str {
    match list {
        PopoverList::Auto => "auto",
        PopoverList::Hint => "hint",
        PopoverList::None => "none",
    }
}

fn mode_name(mode: PopoverMode) -> &'static str {
    match mode {
        PopoverMode::Auto => "auto",
        PopoverMode::Hint => "hint",
        PopoverMode::Manual => "manual",
    }
}

fn list_of(word: &str) -> PopoverList {
    match word {
        "auto" => PopoverList::Auto,
        "hint" => PopoverList::Hint,
        _ => PopoverList::None,
    }
}

impl DocumentHost {
    /// Answers [`js::Call::ElementReference`].
    pub(super) fn element_reference(
        &mut self,
        op: &str,
        element: Handle,
        attribute: &str,
        elements: &[Handle],
    ) -> Result<Answer, DomError> {
        let element = self.node(element)?;
        if !dom::ELEMENT_REFERENCES.contains(&attribute) {
            return Err(DomError::not_supported());
        }
        Ok(match op {
            "associated" => {
                let found = self.document.associated_elements(element, attribute);
                self.work += found.len() as u64;
                Answer::Nodes(found.into_iter().map(handle).collect())
            }
            "referenced" => {
                let found = self.document.referenced_element(element, attribute);
                self.give(found)
            }
            "set" => {
                let nodes = elements
                    .iter()
                    .map(|&h| self.node(h))
                    .collect::<Result<Vec<NodeId>, DomError>>()?;
                self.document
                    .set_explicit_elements(element, attribute, Some(nodes));
                Answer::Nothing
            }
            "unset" => {
                self.document
                    .set_explicit_elements(element, attribute, None);
                Answer::Nothing
            }
            _ => return Err(DomError::not_supported()),
        })
    }

    /// The node `nodes[i]` names, or `None`.
    fn top_layer_node(
        &self,
        nodes: &[Option<Handle>; 4],
        i: usize,
    ) -> Result<Option<NodeId>, DomError> {
        nodes[i].map(|h| self.node(h)).transpose()
    }

    /// The node `nodes[i]` names, which the op needs.
    fn top_layer_needs(&self, nodes: &[Option<Handle>; 4], i: usize) -> Result<NodeId, DomError> {
        self.top_layer_node(nodes, i)?
            .ok_or_else(|| DomError::new("TypeError", "A node is needed."))
    }

    /// Answers [`js::Call::TopLayer`]: `op` on `nodes` and `word`, as its
    /// documentation lists them.
    pub(super) fn top_layer_call(
        &mut self,
        op: &str,
        nodes: &[Option<Handle>; 4],
        word: &str,
    ) -> Result<Answer, DomError> {
        let nodes_of =
            |list: &[NodeId]| Answer::Nodes(list.iter().map(|&node| handle(node)).collect());
        Ok(match op {
            "popover" => {
                let node = self.top_layer_needs(nodes, 0)?;
                Answer::Strings(
                    self.document
                        .popover(node)
                        .map(|p| vec![mode_name(p.mode).to_owned(), list_name(p.list).to_owned()])
                        .unwrap_or_default(),
                )
            }
            "invoker" => {
                let node = self.top_layer_needs(nodes, 0)?;
                let invoker = self.document.popover(node).and_then(|p| p.invoker);
                self.give(invoker)
            }
            "anchor" => {
                let node = self.top_layer_needs(nodes, 0)?;
                let anchor = self.document.popover(node).and_then(|p| p.anchor);
                self.give(anchor)
            }
            "list" => nodes_of(self.document.popover_list(list_of(word))),
            "show" => {
                let node = self.top_layer_needs(nodes, 0)?;
                let invoker = self.top_layer_node(nodes, 1)?;
                let anchor = self.top_layer_node(nodes, 3)?;
                let mut words = word.split(' ');
                let mode = match words.next() {
                    Some("auto") => PopoverMode::Auto,
                    Some("hint") => PopoverMode::Hint,
                    _ => PopoverMode::Manual,
                };
                let list = list_of(words.next().unwrap_or(""));
                self.document.show_popover_state(
                    node,
                    Popover {
                        mode,
                        list,
                        invoker,
                        previously_focused: None,
                        anchor,
                    },
                );
                Answer::Nothing
            }
            "hide" => {
                let node = self.top_layer_needs(nodes, 0)?;
                let previous = self
                    .document
                    .hide_popover_state(node)
                    .and_then(|p| p.previously_focused)
                    .filter(|&p| self.document.contains(p));
                self.give(previous)
            }
            "setFocus" => {
                let node = self.top_layer_needs(nodes, 0)?;
                let focused = self.top_layer_node(nodes, 1)?;
                self.document.set_popover_focus(node, focused);
                Answer::Nothing
            }
            "dialog" => {
                let node = self.top_layer_needs(nodes, 0)?;
                Answer::Strings(
                    self.document
                        .dialog(node)
                        .map(|d| vec![if d.modal { "modal" } else { "open" }.to_owned()])
                        .unwrap_or_default(),
                )
            }
            "dialogs" => {
                let open: Vec<NodeId> = self.document.open_dialogs().collect();
                nodes_of(&open)
            }
            "openDialog" => {
                let node = self.top_layer_needs(nodes, 0)?;
                let focused = self.top_layer_node(nodes, 1)?;
                self.document.open_dialog_state(
                    node,
                    Dialog {
                        modal: word == "modal",
                        previously_focused: focused,
                    },
                );
                Answer::Nothing
            }
            "closeDialog" => {
                let node = self.top_layer_needs(nodes, 0)?;
                let previous = self
                    .document
                    .close_dialog_state(node)
                    .and_then(|d| d.previously_focused)
                    .filter(|&p| self.document.contains(p));
                self.give(previous)
            }
            "watchers" => Answer::Numbers(
                self.document
                    .close_watcher_groups()
                    .last()
                    .into_iter()
                    .flatten()
                    .flat_map(|w| match *w {
                        Watcher::Element(node) => [1.0, handle(node).0 as f64],
                        Watcher::Script(id) => [2.0, f64::from(id)],
                    })
                    .collect(),
            ),
            "addWatcher" | "removeWatcher" => {
                let id: u32 = word
                    .parse()
                    .map_err(|_| DomError::new("TypeError", "Not a close watcher."))?;
                if op == "addWatcher" {
                    self.document.add_close_watcher(id);
                } else {
                    self.document.remove_close_watcher(id);
                }
                Answer::Nothing
            }
            "delegate" => {
                let node = self.top_layer_needs(nodes, 0)?;
                self.styles_now();
                let styles = self.computed.as_ref().map(|computed| &computed.styles);
                let found = if word == "dialog" {
                    crate::focus::focus_delegate(&self.document, styles, node)
                } else if self.document.attr(node, "autofocus").is_some() {
                    Some(node)
                } else {
                    crate::focus::autofocus_delegate(&self.document, styles, node)
                };
                self.give(found)
            }
            "interest" => {
                let source = self.top_layer_needs(nodes, 0)?;
                let target = self.top_layer_needs(nodes, 1)?;
                let gained = word == "gain";
                let document = &mut self.document;
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
                Answer::Nothing
            }
            // A drop-down's `showPicker()` (HTML section 4.10.7): the
            // error to throw, or none, its list then shown after the run.
            "showPicker" => {
                let node = self.top_layer_needs(nodes, 0)?;
                let error = if dom::forms::actually_disabled(&self.document, node) {
                    Some("InvalidStateError")
                } else if !self.transiently_active() {
                    Some("NotAllowedError")
                } else {
                    self.outputs.picker = Some(node);
                    None
                };
                Answer::MaybeString(error.map(str::to_owned))
            }
            _ => return Err(DomError::not_supported()),
        })
    }

    /// Whether the tab's documents have the user's transient activation
    /// (HTML section 6.4.3), which this does not consume.
    fn transiently_active(&self) -> bool {
        let Some(env) = self.window.frames.clone() else {
            return false;
        };
        let now = self
            .window
            .clock
            .as_ref()
            .map(|clock| clock())
            .unwrap_or_default();
        crate::frames::lock(&env.tree).transiently_active(now)
    }
}
