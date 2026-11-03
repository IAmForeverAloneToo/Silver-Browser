//! The natives of shadow trees (gap batch H, D68): `attachShadow` and
//! what a shadow root says of itself, `getRootNode()`, slots and their
//! assignment, the event path through shadow trees with its targets
//! retargeted, a host's focus delegate, and `getHTML()` and
//! `setHTMLUnsafe()` with declarative shadow roots. The trees and the
//! slots' assignment are the document's; the bindings make the objects
//! and dispatch along the path this gives them.

use dom::{MutationError, NodeId, ShadowRootInit, ShadowRootMode, SlotAssignmentMode};
use js::{Answer, Call, DomError};

use super::{DocumentHost, handle, refused};

/// A struct of an event path (DOM section 2.9): its invocation target,
/// its shadow-adjusted target, its related target, whether it is the
/// root of a closed tree, and whether it is a slot in a closed tree.
type PathStruct = (NodeId, Option<NodeId>, Option<NodeId>, bool, bool);

impl DocumentHost {
    /// Answers a call of the shadow trees' natives.
    pub(super) fn shadow_call(&mut self, call: Call) -> Result<Answer, DomError> {
        match call {
            Call::AttachShadow {
                host,
                mode,
                delegates_focus,
                slot_assignment,
                clonable,
                serializable,
                reference_target,
            } => {
                let host = self.node(host)?;
                let init = ShadowRootInit {
                    mode: match mode.as_str() {
                        "closed" => ShadowRootMode::Closed,
                        _ => ShadowRootMode::Open,
                    },
                    delegates_focus,
                    slot_assignment: match slot_assignment.as_str() {
                        "manual" => SlotAssignmentMode::Manual,
                        _ => SlotAssignmentMode::Named,
                    },
                    clonable,
                    serializable,
                };
                // A declarative root taken over keeps its reference target.
                let fresh = self.document.shadow_root(host).is_none();
                let answer = self.attach_shadow(host, init)?;
                if let Some(root) = self.document.shadow_root(host).filter(|_| fresh) {
                    self.document
                        .set_reference_target(root, reference_target.as_deref())
                        .map_err(refused)?;
                }
                Ok(answer)
            }
            Call::SetReferenceTarget(root, target) => {
                let root = self.node(root)?;
                self.document
                    .set_reference_target(root, target.as_deref())
                    .map_err(refused)?;
                Ok(Answer::Nothing)
            }
            Call::ShadowRoot(host) => {
                let host = self.node(host)?;
                Ok(self.give(self.document.shadow_root(host)))
            }
            Call::ShadowInfo(root) => {
                let root = self.node(root)?;
                let Some(init) = self.document.shadow_init(root) else {
                    return Ok(Answer::Strings(Vec::new()));
                };
                let flag = |on: bool| if on { "true" } else { "false" }.to_owned();
                Ok(Answer::Strings(
                    vec![
                        match init.mode {
                            ShadowRootMode::Open => "open",
                            ShadowRootMode::Closed => "closed",
                        }
                        .to_owned(),
                        match init.slot_assignment {
                            SlotAssignmentMode::Named => "named",
                            SlotAssignmentMode::Manual => "manual",
                        }
                        .to_owned(),
                        flag(init.delegates_focus),
                        flag(init.clonable),
                        flag(init.serializable),
                        flag(self.document.is_declarative_shadow(root)),
                    ]
                    .into_iter()
                    .chain(self.document.reference_target(root).map(str::to_owned))
                    .collect(),
                ))
            }
            Call::ShadowHost(root) => {
                let root = self.node(root)?;
                Ok(self.give(self.document.shadow_host(root)))
            }
            Call::RootNode(node, composed) => {
                let node = self.node(node)?;
                self.work += 1;
                Ok(self.give(Some(self.document.root_node(node, composed))))
            }
            Call::AssignedSlot(node) => {
                let node = self.node(node)?;
                // DOM's "find a slot" with its open flag: a slot in a closed
                // tree is not given.
                let slot = self.document.assigned_slot(node).filter(|&slot| {
                    let root = self.document.root_of(slot);
                    self.document
                        .shadow_init(root)
                        .is_some_and(|init| init.mode == ShadowRootMode::Open)
                });
                Ok(self.give(slot))
            }
            Call::AssignedNodes(slot, flatten) => {
                let slot = self.node(slot)?;
                let nodes = match flatten {
                    true => self.document.flattened_assigned_nodes(slot),
                    false => self.document.assigned_nodes(slot).to_vec(),
                };
                self.work += nodes.len() as u64;
                Ok(Answer::Nodes(nodes.into_iter().map(handle).collect()))
            }
            Call::AssignSlot(slot, nodes) => {
                let slot = self.node(slot)?;
                let nodes = nodes
                    .into_iter()
                    .map(|node| self.node(node))
                    .collect::<Result<Vec<NodeId>, DomError>>()?;
                self.work += nodes.len() as u64;
                self.document.assign_slot(slot, &nodes).map_err(refused)?;
                Ok(Answer::Nothing)
            }
            Call::TakeSlotChanges => {
                let slots = self.document.take_slot_changes();
                Ok(Answer::Nodes(
                    slots
                        .into_iter()
                        .filter(|&slot| self.document.contains(slot))
                        .map(handle)
                        .collect(),
                ))
            }
            Call::EventPath {
                target,
                related,
                composed,
            } => {
                let target = self.node(target)?;
                let related = related.map(|related| self.node(related)).transpose()?;
                Ok(Answer::Numbers(self.event_path(target, related, composed)))
            }
            Call::Retarget(a, b) => {
                let (a, b) = (self.node(a)?, self.node(b)?);
                Ok(self.give(Some(self.document.retarget(a, b))))
            }
            Call::FocusDelegate(host) => {
                let host = self.node(host)?;
                self.styles_now();
                let styles = self.computed.as_ref().map(|computed| &computed.styles);
                let delegate = crate::focus::focus_delegate(&self.document, styles, host);
                self.work += 1;
                Ok(self.give(delegate))
            }
            Call::GetHtml {
                node,
                serializable,
                roots,
            } => {
                let node = self.node(node)?;
                let roots = roots
                    .into_iter()
                    .filter_map(|root| self.node(root).ok())
                    .collect::<Vec<NodeId>>();
                let markup = html::serialize_children_with(
                    &self.document,
                    node,
                    true,
                    html::ShadowRoots {
                        serializable,
                        roots: &roots,
                    },
                );
                self.work += markup.len() as u64 / 64;
                Ok(Answer::String(markup))
            }
            _ => Err(DomError::not_supported()),
        }
    }

    /// `attachShadow` (DOM section 4.9): a root attached to `host` with
    /// `init`, or the declarative root it hosts, emptied, its children
    /// left for a script to hold or for the sweep.
    fn attach_shadow(&mut self, host: NodeId, init: ShadowRootInit) -> Result<Answer, DomError> {
        let existing = self.document.shadow_root(host);
        let emptied: Vec<NodeId> = existing
            .filter(|&root| self.document.is_declarative_shadow(root))
            .map(|root| self.document.children(root).collect())
            .unwrap_or_default();
        let root = self
            .document
            .attach_shadow(host, init, false)
            .map_err(|error| match error {
                MutationError::NotSupported | MutationError::WrongKind if existing.is_some() => {
                    DomError::new(
                        "NotSupportedError",
                        "Shadow root cannot be created on a host which already hosts a shadow \
                         tree.",
                    )
                }
                MutationError::NotSupported | MutationError::WrongKind => DomError::new(
                    "NotSupportedError",
                    "This element does not support attachShadow",
                ),
                other => refused(other),
            })?;
        for child in emptied {
            self.taken_out(child);
        }
        Ok(self.give(Some(root)))
    }

    /// DOM's event path (section 2.9, the dispatch's steps 5.1 to 5.7) of
    /// an event at `target` whose related target, a node, is `related`,
    /// `composed` or not, as [`Call::EventPath`] gives it; the window's
    /// struct is the bindings' to add after the document.
    fn event_path(&mut self, target: NodeId, related: Option<NodeId>, composed: bool) -> Vec<f64> {
        let document = &self.document;
        let id = |node: Option<NodeId>| node.map_or(-1.0, |node| handle(node).0 as f64);
        let closed_root = |node: NodeId| {
            document
                .shadow_init(node)
                .is_some_and(|init| init.mode == ShadowRootMode::Closed)
        };
        let target_tree = document.root_of(target);
        // DOM's "get the parent": an assigned slottable's slot, a shadow
        // root's host unless the event is not composed and the root is
        // the target's, or the parent.
        let parent_of = |node: NodeId| {
            if document.is_shadow_root(node) {
                return match !composed && node == target_tree {
                    true => None,
                    false => document.shadow_host(node),
                };
            }
            document
                .assigned_slot(node)
                .or_else(|| document.parent(node))
        };
        let retargeted = |node: NodeId| related.map(|related| document.retarget(related, node));
        let first_related = retargeted(target);
        if first_related == Some(target) && related != Some(target) {
            return Vec::new();
        }
        let mut path: Vec<PathStruct> = vec![(
            target,
            Some(target),
            first_related,
            closed_root(target),
            false,
        )];
        let mut slottable = document.assigned_slot(target).map(|_| target);
        let mut slot_in_closed = false;
        let mut current = target;
        let mut parent = parent_of(target);
        let mut steps = 0u64;
        while let Some(at) = parent {
            steps += 1;
            if slottable.take().is_some() {
                let root = document.root_of(at);
                slot_in_closed = closed_root(root);
            }
            if document.assigned_slot(at).is_some() {
                slottable = Some(at);
            }
            let related_here = retargeted(at);
            let current_tree = document.root_of(current);
            if document.is_shadow_including_inclusive_ancestor(current_tree, at) {
                path.push((at, None, related_here, closed_root(at), slot_in_closed));
            } else if Some(at) == related_here {
                break;
            } else {
                current = at;
                path.push((at, Some(at), related_here, closed_root(at), slot_in_closed));
            }
            parent = parent_of(at);
            slot_in_closed = false;
        }
        // The targets are cleared after the dispatch when the last struct
        // with a target names nodes in a shadow tree.
        let in_shadow = |node: Option<NodeId>| {
            node.is_some_and(|node| document.is_shadow_root(document.root_of(node)))
        };
        let clear = path
            .iter()
            .rev()
            .find(|entry| entry.1.is_some())
            .is_some_and(|entry| in_shadow(entry.1) || in_shadow(entry.2));
        let window_related = related.map(|related| document.retarget(related, document.root()));
        let mut out = Vec::with_capacity(path.len() * 6 + 2);
        for (node, adjusted, related, root_closed, slot_closed) in path {
            let in_shadow = document.is_shadow_root(document.root_of(node));
            out.extend([
                id(Some(node)),
                id(adjusted),
                id(related),
                f64::from(u8::from(root_closed)),
                f64::from(u8::from(slot_closed)),
                f64::from(u8::from(in_shadow)),
            ]);
        }
        out.push(f64::from(u8::from(clear)));
        out.push(id(window_related));
        self.work += steps;
        out
    }
}
