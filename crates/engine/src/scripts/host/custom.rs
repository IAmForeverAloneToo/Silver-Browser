//! The natives of custom elements (gap batch H, D70): definitions as the
//! document keeps them, elements' custom element states and `is` values,
//! the reactions the document's changes queue, which the bindings take
//! after each call, the custom states `:state()` matches, the elements
//! the parser constructs, and form-associated custom elements' values
//! and validity. The registries, constructors and callbacks are the
//! bindings'.

use dom::{CustomElementState, Definition, FormValue, NodeId, Reaction};
use js::{Answer, CustomCall, DomError};

use super::{DocumentHost, handle, refused};

/// A custom element state as the bindings number it.
fn state_number(state: CustomElementState) -> f64 {
    match state {
        CustomElementState::Undefined => 0.0,
        CustomElementState::Failed => 1.0,
        CustomElementState::Uncustomized => 2.0,
        CustomElementState::Precustomized => 3.0,
        CustomElementState::Custom => 4.0,
    }
}

/// The state the bindings' number names.
fn numbered_state(number: u8) -> Option<CustomElementState> {
    Some(match number {
        0 => CustomElementState::Undefined,
        1 => CustomElementState::Failed,
        2 => CustomElementState::Uncustomized,
        3 => CustomElementState::Precustomized,
        4 => CustomElementState::Custom,
        _ => return None,
    })
}

/// A string as a JSON string literal.
fn json(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A string or `null` as JSON.
fn maybe_json(text: Option<&str>) -> String {
    text.map_or_else(|| "null".to_owned(), json)
}

/// The reactions as the bindings take them: a JSON array of arrays, each
/// its kind (`u`, `c`, `d`, `m`, `a`, `t`, `f`, `r`, `x`), its element's
/// handle and its arguments.
fn reactions_json(reactions: &[Reaction]) -> String {
    let node = |id: NodeId| handle(id).0.to_string();
    let items: Vec<String> = reactions
        .iter()
        .map(|reaction| match reaction {
            Reaction::Upgrade {
                element,
                definition,
            } => format!("[\"u\",{},{definition}]", node(*element)),
            Reaction::Connected(element) => format!("[\"c\",{}]", node(*element)),
            Reaction::Disconnected(element) => format!("[\"d\",{}]", node(*element)),
            Reaction::ConnectedMove(element) => format!("[\"m\",{}]", node(*element)),
            Reaction::Adopted { element, old, new } => {
                format!("[\"a\",{},{},{}]", node(*element), node(*old), node(*new))
            }
            Reaction::AttributeChanged {
                element,
                name,
                old,
                value,
                namespace,
            } => format!(
                "[\"t\",{},{},{},{},{}]",
                node(*element),
                json(name),
                maybe_json(old.as_deref()),
                maybe_json(value.as_deref()),
                maybe_json(namespace.as_deref())
            ),
            Reaction::FormAssociated { element, form } => format!(
                "[\"f\",{},{}]",
                node(*element),
                form.map_or_else(|| "null".to_owned(), node)
            ),
            Reaction::FormReset(element) => format!("[\"r\",{}]", node(*element)),
            Reaction::FormDisabled { element, disabled } => {
                format!("[\"x\",{},{disabled}]", node(*element))
            }
        })
        .collect();
    format!("[{}]", items.join(","))
}

impl DocumentHost {
    /// Answers a call of the custom elements' natives.
    pub(super) fn custom_call(&mut self, call: CustomCall) -> Result<Answer, DomError> {
        match call {
            CustomCall::Define {
                registry,
                name,
                local_name,
                observed,
                callbacks,
                form_associated,
                disable_shadow,
            } => {
                let id = self
                    .document
                    .define_custom_element(Definition {
                        registry,
                        name,
                        local_name,
                        observed,
                        callbacks,
                        form_associated,
                        disable_shadow,
                    })
                    .map_err(refused)?;
                Ok(Answer::Number(f64::from(id)))
            }
            CustomCall::Candidates(id) => {
                let candidates = self.document.upgrade_candidates(id);
                self.work += candidates.len() as u64;
                Ok(Answer::Nodes(candidates.into_iter().map(handle).collect()))
            }
            CustomCall::State(element) => {
                let element = self.node(element)?;
                let state = self.document.custom_element_state(element);
                let definition = self.document.custom_element_definition(element);
                Ok(Answer::Numbers(vec![
                    state_number(state),
                    f64::from(definition.unwrap_or(0)),
                ]))
            }
            CustomCall::SetState {
                element,
                state,
                definition,
            } => {
                let element = self.node(element)?;
                let state = numbered_state(state)
                    .ok_or_else(|| DomError::new("TypeError", "not a custom element state"))?;
                let definition = (definition != 0).then_some(definition);
                self.document
                    .set_custom_element_state(element, state, definition)
                    .map_err(refused)?;
                Ok(Answer::Nothing)
            }
            CustomCall::SetIs(element, is) => {
                let element = self.node(element)?;
                self.document
                    .set_is_value(element, is.as_deref())
                    .map_err(refused)?;
                Ok(Answer::Nothing)
            }
            CustomCall::Is(element) => {
                let element = self.node(element)?;
                Ok(Answer::MaybeString(
                    self.document.is_value(element).map(str::to_owned),
                ))
            }
            CustomCall::TakeReactions => {
                let reactions = self.document.take_reactions();
                self.work += reactions.len() as u64;
                Ok(Answer::String(reactions_json(&reactions)))
            }
            CustomCall::Registry(node) => {
                let node = self.node(node)?;
                Ok(match self.document.custom_element_registry(node) {
                    Some(registry) => Answer::Number(f64::from(registry)),
                    None => Answer::Node(None),
                })
            }
            CustomCall::SetRegistry(node, registry) => {
                let node = self.node(node)?;
                self.document.set_custom_element_registry(node, registry);
                Ok(Answer::Nothing)
            }
            CustomCall::SetCustomState { element, name, on } => {
                let element = self.node(element)?;
                let changed = self
                    .document
                    .set_custom_state(element, &name, on)
                    .map_err(refused)?;
                Ok(Answer::Bool(changed))
            }
            CustomCall::ParserElement(element) => {
                let element = self.node(element)?;
                if let Some(load) = self.parser.as_mut() {
                    load.provide_element(element);
                }
                Ok(Answer::Nothing)
            }
            CustomCall::SetPrefix(element, prefix) => {
                let element = self.node(element)?;
                self.document
                    .set_element_prefix(element, &prefix)
                    .map_err(refused)?;
                Ok(Answer::Nothing)
            }
            CustomCall::Kept(roots) => {
                let mut held = self.mutations.held();
                held.extend(self.document.live_nodes());
                held.extend(self.outputs.events.iter().map(|&(node, _)| node));
                held.extend(self.media.active());
                let mut kept = Vec::new();
                for root in roots {
                    let Ok(node) = self.node(root) else {
                        continue;
                    };
                    // The tree the bindings group its objects by: its
                    // shadow trees and its templates' contents with it.
                    let subtree = self.document.subtree_with_contents(node);
                    self.work += subtree.len() as u64;
                    if subtree.iter().any(|node| held.contains(node)) {
                        kept.push(root);
                    }
                }
                Ok(Answer::Nodes(kept))
            }
            CustomCall::Upgradable(root, wanted) => {
                let root = self.node(root)?;
                let mut found = Vec::new();
                for node in self.document.shadow_including_subtree(root) {
                    self.work += 1;
                    let Some((ns, local)) = self.document.custom_name(node) else {
                        continue;
                    };
                    let registry = self.document.custom_element_registry(node);
                    if registry != Some(wanted) {
                        continue;
                    }
                    let is = self.document.is_value(node);
                    if let Some(definition) = self
                        .document
                        .look_up_custom_element(registry, ns, local, is)
                    {
                        found.push(handle(node).0 as f64);
                        found.push(f64::from(definition));
                    }
                }
                Ok(Answer::Numbers(found))
            }
            CustomCall::SetFormValue {
                element,
                kind,
                strings,
                files,
            } => {
                let element = self.node(element)?;
                let file = |i: usize| files.get(i).copied().unwrap_or(false);
                let value = match kind {
                    1 => FormValue::One(strings.into_iter().next()),
                    2 => FormValue::One(None),
                    3 => FormValue::Entries(
                        strings
                            .chunks(2)
                            .enumerate()
                            .map(|(i, pair)| {
                                let name = pair.first().cloned().unwrap_or_default();
                                let value = pair.get(1).cloned().filter(|_| !file(i));
                                (name, value)
                            })
                            .collect(),
                    ),
                    _ => FormValue::None,
                };
                self.document
                    .set_form_value(element, value)
                    .map_err(refused)?;
                Ok(Answer::Nothing)
            }
            CustomCall::Initialize { root, registry } => {
                let root = self.node(root)?;
                let found = self.document.initialize_registry(root, registry);
                self.work += found.len() as u64;
                Ok(Answer::Numbers(
                    found
                        .into_iter()
                        .flat_map(|(element, definition)| {
                            [handle(element).0 as f64, f64::from(definition)]
                        })
                        .collect(),
                ))
            }
            CustomCall::SetFormValidity {
                element,
                flags,
                message,
            } => {
                let element = self.node(element)?;
                self.document
                    .set_form_validity(element, flags, &message)
                    .map_err(refused)?;
                Ok(Answer::Nothing)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reactions_are_given_as_json_arrays_of_their_kind_and_arguments() {
        let mut document = dom::Document::new();
        let a = document.create_element("x-a").unwrap();
        let b = document.root();
        let given = reactions_json(&[
            Reaction::Connected(a),
            Reaction::AttributeChanged {
                element: a,
                name: "t\"".into(),
                old: None,
                value: Some("v\n".into()),
                namespace: None,
            },
            Reaction::FormAssociated {
                element: a,
                form: None,
            },
            Reaction::FormDisabled {
                element: a,
                disabled: true,
            },
            Reaction::Adopted {
                element: a,
                old: b,
                new: b,
            },
        ]);
        let (h, d) = (handle(a).0, handle(b).0);
        assert_eq!(
            given,
            format!(
                "[[\"c\",{h}],[\"t\",{h},\"t\\\"\",null,\"v\\u000a\",null],\
                 [\"f\",{h},null],[\"x\",{h},true],[\"a\",{h},{d},{d}]]"
            )
        );
    }
}
