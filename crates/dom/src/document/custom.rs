//! Custom elements as the document keeps them (gap batch H, D70): each
//! element's custom element state, definition and `is` value (HTML
//! section 4.13), the definitions' names and what their reactions need
//! (observed attributes, which lifecycle callbacks exist), the registry
//! each document and shadow root looks definitions up in, an element's
//! custom states (`:state()`), and the custom element reactions the tree's
//! changes queue.
//!
//! The registries, constructors and callbacks are the bindings': a
//! registry here is a number, a definition a number with its names. The
//! document queues the reactions its own operations cause, in order, as
//! HTML's "enqueue a custom element callback reaction" and "try to
//! upgrade an element" say: an element connected, disconnected, moved,
//! adopted, or an observed attribute of it changed, and an upgrade when
//! an undefined element is connected, cloned or made by a fragment's
//! parser after its definition. The bindings take them after each call
//! ([`Document::take_reactions`]) and run them before returning to the
//! script, as `[CEReactions]` says.
//!
//! `:defined` and `:state()` are restyled by two flags of
//! [`ElementState`] that flip at each change of what they match
//! ([`ElementState::DEFINED_CHANGED`], [`ElementState::STATES_CHANGED`]).

use std::collections::HashMap;

use super::{Document, ElementState, MutationError, NodeData, NodeId};
use crate::Namespace;

/// The registry of the page's window (`window.customElements`): the
/// page's document looks definitions up in it.
pub const WINDOW_REGISTRY: u32 = 1;

/// The most definitions a document keeps, every registry's together.
pub const MAX_DEFINITIONS: usize = 1 << 16;

/// The most custom states (`ElementInternals.states`) one element has.
pub const MAX_CUSTOM_STATES: usize = 1 << 10;

/// An element's custom element state (HTML section 4.13.1): what
/// `:defined` reads and whether reactions are queued for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CustomElementState {
    /// A valid custom element name, or an `is` value, with no definition
    /// yet.
    Undefined,
    /// Its construction or upgrade threw.
    Failed,
    /// Any other element.
    Uncustomized,
    /// Being upgraded: its constructor is running.
    Precustomized,
    /// Constructed or upgraded by its definition.
    Custom,
}

/// The lifecycle callbacks a definition has, as [`Definition::callbacks`]
/// flags: a reaction is queued only for a callback that exists.
pub mod callbacks {
    pub const CONNECTED: u8 = 1;
    pub const DISCONNECTED: u8 = 2;
    pub const ADOPTED: u8 = 4;
    pub const ATTRIBUTE_CHANGED: u8 = 8;
    pub const CONNECTED_MOVE: u8 = 16;
    pub const FORM_ASSOCIATED: u8 = 32;
    pub const FORM_RESET: u8 = 64;
    pub const FORM_DISABLED: u8 = 128;
}

/// A custom element definition as the document keeps it: its registry,
/// its name and local name (the same for an autonomous custom element,
/// the extended element's for a customized built-in one), its observed
/// attributes' local names, which callbacks it has, and whether it is
/// form-associated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    pub registry: u32,
    pub name: String,
    pub local_name: String,
    pub observed: Vec<String>,
    pub callbacks: u8,
    pub form_associated: bool,
    /// Whether its elements may have no shadow root (`disabledFeatures`'
    /// `shadow`).
    pub disable_shadow: bool,
}

/// A custom element reaction (HTML section 4.13.6), queued by the
/// document's operations for the bindings to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reaction {
    /// Upgrade `element` by definition `definition`.
    Upgrade {
        element: NodeId,
        definition: u32,
    },
    Connected(NodeId),
    Disconnected(NodeId),
    /// Moved by `moveBefore` within a connected tree: its
    /// `connectedMoveCallback`, or its disconnected and connected ones.
    ConnectedMove(NodeId),
    /// Adopted from document `old` into `new`.
    Adopted {
        element: NodeId,
        old: NodeId,
        new: NodeId,
    },
    /// Observed attribute `name` (its local name) of namespace
    /// `namespace` changed from `old` to `value`, each `None` for none.
    AttributeChanged {
        element: NodeId,
        name: String,
        old: Option<String>,
        value: Option<String>,
        namespace: Option<String>,
    },
    /// A form-associated element's form owner changed to `form`.
    FormAssociated {
        element: NodeId,
        form: Option<NodeId>,
    },
    /// Its form was reset.
    FormReset(NodeId),
    /// It became disabled or enabled by an ancestor or its attribute.
    FormDisabled {
        element: NodeId,
        disabled: bool,
    },
}

impl Reaction {
    /// The element the reaction is queued for.
    pub fn element(&self) -> NodeId {
        match *self {
            Reaction::Upgrade { element, .. }
            | Reaction::Adopted { element, .. }
            | Reaction::AttributeChanged { element, .. }
            | Reaction::FormAssociated { element, .. }
            | Reaction::FormDisabled { element, .. } => element,
            Reaction::Connected(element)
            | Reaction::Disconnected(element)
            | Reaction::ConnectedMove(element)
            | Reaction::FormReset(element) => element,
        }
    }
}

/// What the document keeps of custom elements; most documents have
/// nothing here.
#[derive(Debug, Clone, Default)]
pub(super) struct Custom {
    /// The definitions, numbered from 1 by their place.
    definitions: Vec<Definition>,
    /// Each registry's definitions by name.
    by_name: HashMap<(u32, String), u32>,
    /// The elements whose state is not the one their names imply (see
    /// [`Document::custom_element_state`]), with their definitions.
    states: HashMap<NodeId, (CustomElementState, Option<u32>)>,
    /// The elements' `is` values.
    is_values: HashMap<NodeId, String>,
    /// The registry of each document, shadow root or element that is not
    /// the one it would have by default: none, or a scoped one.
    registries: HashMap<NodeId, Option<u32>>,
    /// The elements that keep no registry when inserted (DOM's "keep
    /// custom element registry null"): those a parser made with a
    /// `customelementregistry` attribute.
    keep_null: std::collections::HashSet<NodeId>,
    /// Each element's custom states, in the order they were added.
    custom_states: HashMap<NodeId, Vec<String>>,
    /// The reactions queued and not yet taken.
    reactions: Vec<Reaction>,
    /// The form-associated custom elements' submission values, validity
    /// and the form owner and disabledness last told them.
    faces: HashMap<NodeId, Face>,
    /// The tree changed since the form owners and disabledness were
    /// last compared with what the elements were told.
    faces_dirty: bool,
}

/// A form-associated custom element's submission value (HTML section
/// 4.13.7.3, `setFormValue()`): none, one value (`None` for a file), or a
/// `FormData`'s entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FormValue {
    #[default]
    None,
    One(Option<String>),
    Entries(Vec<(String, Option<String>)>),
}

impl FormValue {
    fn bytes(&self) -> usize {
        match self {
            FormValue::None => 0,
            FormValue::One(value) => value.as_ref().map_or(0, String::len),
            FormValue::Entries(entries) => entries
                .iter()
                .map(|(name, value)| name.len() + value.as_ref().map_or(0, String::len))
                .sum(),
        }
    }
}

/// What the document keeps of a form-associated custom element.
#[derive(Debug, Clone, Default)]
struct Face {
    value: FormValue,
    /// Its validity flags, as [`crate::validity::Validity::bits`], and its
    /// validation message (`setValidity()`).
    validity: u32,
    message: String,
    /// The form owner and disabledness it was last told of.
    owner: Option<NodeId>,
    disabled: bool,
}

impl Face {
    fn bytes(&self) -> usize {
        self.value.bytes() + self.message.len()
    }
}

impl Custom {
    /// Whether any element is custom or any definition exists: until
    /// then no operation queues a reaction.
    fn active(&self) -> bool {
        !self.definitions.is_empty() || !self.states.is_empty()
    }
}

impl Document {
    // ---- Definitions -------------------------------------------------------

    /// Records a definition (HTML's `define()`, after the bindings'
    /// checks); its number. A name already defined in the registry is
    /// refused, as are definitions past [`MAX_DEFINITIONS`].
    pub fn define_custom_element(&mut self, definition: Definition) -> Result<u32, MutationError> {
        let custom = &mut self.rare.custom;
        let key = (definition.registry, definition.name.clone());
        if custom.by_name.contains_key(&key) || custom.definitions.len() >= MAX_DEFINITIONS {
            return Err(MutationError::NotSupported);
        }
        let size = definition.name.len()
            + definition.local_name.len()
            + definition.observed.iter().map(String::len).sum::<usize>();
        self.check_data(size)?;
        self.data_bytes += size;
        let custom = &mut self.rare.custom;
        custom.definitions.push(definition);
        let id = custom.definitions.len() as u32;
        custom.by_name.insert(key, id);
        Ok(id)
    }

    /// Definition `id`.
    pub fn custom_definition(&self, id: u32) -> Option<&Definition> {
        let index = (id as usize).checked_sub(1)?;
        self.rare.custom.definitions.get(index)
    }

    /// HTML's "look up a custom element definition" (section 4.13.4): in
    /// `registry`, an HTML element named `local_name` with `is` value
    /// `is` is an autonomous custom element of that name, or a customized
    /// built-in one of the name `is` that extends `local_name`.
    pub fn look_up_custom_element(
        &self,
        registry: Option<u32>,
        ns: &Namespace,
        local_name: &str,
        is: Option<&str>,
    ) -> Option<u32> {
        let registry = registry?;
        let custom = &self.rare.custom;
        if *ns != Namespace::Html || custom.definitions.is_empty() {
            return None;
        }
        let found = |name: &str| {
            let id = *custom.by_name.get(&(registry, name.to_owned()))?;
            let definition = self.custom_definition(id)?;
            (definition.local_name == local_name).then_some(id)
        };
        found(local_name).or_else(|| is.and_then(found))
    }

    /// The elements definition `id` upgrades when it is defined (HTML's
    /// `define()` step 17): those of its local name, and its name as
    /// their `is` value for a customized built-in one, among the
    /// shadow-including descendants of the documents whose registry is
    /// its, in shadow-including tree order.
    pub fn upgrade_candidates(&self, id: u32) -> Vec<NodeId> {
        let Some(definition) = self.custom_definition(id) else {
            return Vec::new();
        };
        let autonomous = definition.name == definition.local_name;
        let mut documents = vec![self.root];
        documents.extend(self.other_documents());
        documents.sort_by_key(|&document| document != self.root);
        let mut out = Vec::new();
        for document in documents {
            for node in self.shadow_including_subtree(document) {
                let Some((ns, local)) = self.custom_name(node) else {
                    continue;
                };
                if *ns != Namespace::Html
                    || local != definition.local_name
                    || self.custom_element_registry(node) != Some(definition.registry)
                {
                    continue;
                }
                if autonomous || self.is_value(node) == Some(definition.name.as_str()) {
                    out.push(node);
                }
            }
        }
        out
    }

    /// Whether `host` may have no shadow root by its definition
    /// (`disabledFeatures`' `shadow`), looked up as DOM's "attach a shadow
    /// root" step 3 looks it up.
    pub fn shadow_disabled(&self, host: NodeId) -> bool {
        let Some((ns, local)) = self.custom_name(host) else {
            return false;
        };
        let registry = self.custom_element_registry(host);
        self.look_up_custom_element(registry, ns, local, self.is_value(host))
            .and_then(|id| self.custom_definition(id))
            .is_some_and(|definition| definition.disable_shadow)
    }

    // ---- Registries --------------------------------------------------------

    /// The registry `node` looks definitions up in (HTML's "custom element
    /// registry" of a document, a shadow root or an element): one set for
    /// it, or for a document its default, the window's for the page's
    /// and none for another; for a shadow root its host's document's; for
    /// an element its owner document's.
    pub fn custom_element_registry(&self, node: NodeId) -> Option<u32> {
        if let Some(&registry) = self.rare.custom.registries.get(&node) {
            return registry;
        }
        if node == self.root {
            return Some(WINDOW_REGISTRY);
        }
        if self.is_document(node) {
            return None;
        }
        // A template's contents' document has none.
        let (owner, contents) = self.owner_and_contents(node);
        if contents {
            return None;
        }
        match self.rare.custom.registries.get(&owner) {
            Some(&registry) => registry,
            None => (owner == self.root).then_some(WINDOW_REGISTRY),
        }
    }

    /// Sets the registry `node` looks definitions up in.
    pub fn set_custom_element_registry(&mut self, node: NodeId, registry: Option<u32>) {
        if self.contains(node) {
            self.rare.custom.registries.insert(node, registry);
        }
    }

    /// HTML's `initialize()` of `registry` for `root` (section 4.13.4):
    /// its registry, when it is a document or a shadow root with none,
    /// set to `registry`, and that of each element of its inclusive
    /// descendants with none; those of them whose registry `registry` is
    /// that it defines, each with its definition, to try upgrading, the
    /// elements it was the registry of already among them.
    pub fn initialize_registry(&mut self, root: NodeId, registry: u32) -> Vec<(NodeId, u32)> {
        let elements: Vec<NodeId> = self
            .subtree(root)
            .filter(|&node| self.is_element(node))
            .collect();
        if (self.is_document(root) || self.is_shadow_root(root))
            && self.custom_element_registry(root).is_none()
        {
            self.rare.custom.registries.insert(root, Some(registry));
        }
        let mut found = Vec::new();
        for element in elements {
            match self.custom_element_registry(element) {
                None => {
                    self.rare.custom.registries.insert(element, Some(registry));
                }
                Some(own) if own != registry => continue,
                Some(_) => {}
            }
            let Some((ns, local)) = self.custom_name(element) else {
                continue;
            };
            let is = self.is_value(element);
            if let Some(definition) = self.look_up_custom_element(Some(registry), ns, local, is) {
                found.push((element, definition));
            }
        }
        found
    }

    /// Gives a copy `made` of a node its registry (DOM's "clone a single
    /// node" step 2): the original's, or with none `fallback`, a global
    /// one its document's own; the upgrade a definition of it owes the
    /// copy queued.
    pub fn set_copied_registry(&mut self, made: NodeId, original: NodeId, fallback: Option<u32>) {
        if !self.is_element(made) {
            return;
        }
        let registry = self.custom_element_registry(original).or(fallback);
        let document = self.owner_document(made);
        let registry = match registry {
            Some(WINDOW_REGISTRY) => self.effective_global_registry(document),
            other => other,
        };
        if registry != self.custom_element_registry(made) || registry.is_none() {
            self.rare.custom.registries.insert(made, registry);
        }
        self.upgrade_if_defined(made);
    }

    /// A clonable shadow root's copy's registry (DOM's "clone a node" step
    /// 6): the original's, a global one its document's own, and its
    /// keeping none as the original keeps it.
    pub fn set_copied_shadow_registry(&mut self, copy: NodeId, original: NodeId) {
        let registry = match self.custom_element_registry(original) {
            Some(WINDOW_REGISTRY) => self.effective_global_registry(self.owner_document(copy)),
            other => other,
        };
        self.rare.custom.registries.insert(copy, registry);
        if self.rare.custom.keep_null.contains(&original) {
            self.rare.custom.keep_null.insert(copy);
        }
    }

    /// A document's registry when it is a global one (DOM's "effective
    /// global custom element registry"): the window's, of the page's
    /// document; none of a document beside it, or one given a scoped one.
    pub fn effective_global_registry(&self, document: NodeId) -> Option<u32> {
        self.custom_element_registry(document)
            .filter(|&registry| registry == WINDOW_REGISTRY)
    }

    /// What DOM's "adopt" step 3 gives each shadow root and element of
    /// `node`'s shadow-including inclusive descendants for its registry,
    /// read before `node` is `document`'s: a global one the document's,
    /// none the document's too for a shadow root that does not keep none,
    /// and for an element its parent's (the document's at the top), each
    /// as a global registry, and a scoped one kept. Nothing when `node`
    /// stays in its document, which it leaves too going into or out of a
    /// template's contents (`crossed`), theirs in DOM's terms; nor when
    /// no node was given a registry, all of them their documents'.
    pub(super) fn registries_before_adoption(
        &self,
        node: NodeId,
        old: NodeId,
        document: NodeId,
        crossed: bool,
    ) -> Vec<(NodeId, bool)> {
        if (old == document && !crossed) || self.rare.custom.registries.is_empty() {
            return Vec::new();
        }
        let mut changed = Vec::new();
        for at in self.shadow_including_subtree(node) {
            let registry = self.custom_element_registry(at);
            let global = registry == Some(WINDOW_REGISTRY);
            if self.is_shadow_root(at) {
                if global || (registry.is_none() && !self.rare.custom.keep_null.contains(&at)) {
                    changed.push((at, true));
                }
            } else if self.is_element(at) && (registry.is_none() || global) {
                // Its parent's, unless it is the top, under a fragment, or
                // had one: then the document's.
                let parent = self.parent(at).filter(|&parent| {
                    registry.is_none()
                        && !matches!(self.data(parent), Some(NodeData::DocumentFragment))
                });
                changed.push((at, parent.is_none()));
            }
        }
        changed
    }

    /// Gives the nodes [`Document::registries_before_adoption`] found
    /// their registries, now they are `document`'s: the document's, or
    /// their parents', as a global registry.
    pub(super) fn registries_adopted(&mut self, nodes: Vec<(NodeId, bool)>, document: NodeId) {
        crate::hook!("dom::custom::registries_adopted");
        // A template's contents' document has none.
        let contents = nodes
            .first()
            .is_some_and(|&(at, _)| self.owner_and_contents(at).1);
        for (at, of_document) in nodes {
            let registry = match of_document {
                true if contents => None,
                true => self.custom_element_registry(document),
                false => self
                    .parent(at)
                    .and_then(|parent| self.custom_element_registry(parent)),
            };
            let registry = registry.filter(|&registry| registry == WINDOW_REGISTRY);
            self.rare.custom.registries.insert(at, registry);
        }
    }

    /// Whether moving `node` under `parent` takes it into or out of a
    /// template's contents, which DOM has as another document's; only
    /// asked once a node was given a registry.
    pub(super) fn crosses_contents(&self, node: NodeId, parent: NodeId) -> bool {
        !self.rare.custom.registries.is_empty()
            && self.owner_and_contents(node).1 != self.owner_and_contents(parent).1
    }

    /// An element's namespace and local name, which a definition is
    /// looked up by: its prefix, if it has one, left out.
    pub fn custom_name(&self, element: NodeId) -> Option<(&Namespace, &str)> {
        let (ns, _) = self.element_name(element)?;
        Some((ns, self.local_name(element)?))
    }

    /// Gives `element` no registry, kept when it is inserted (DOM's "keep
    /// custom element registry null").
    pub fn keep_custom_element_registry_null(&mut self, element: NodeId) {
        if self.contains(element) {
            self.rare.custom.registries.insert(element, None);
            self.rare.custom.keep_null.insert(element);
        }
    }

    /// Whether `node`'s registry was set for it rather than by default.
    pub fn has_own_custom_element_registry(&self, node: NodeId) -> bool {
        self.rare.custom.registries.contains_key(&node)
    }

    // ---- Elements ----------------------------------------------------------

    /// An element's custom element state: the one it was given, or for an
    /// element never given one, "undefined" for an HTML element whose
    /// local name is a valid custom element name or that has an `is`
    /// value, and "uncustomized" for the rest (HTML's "create an element"
    /// step 7).
    pub fn custom_element_state(&self, element: NodeId) -> CustomElementState {
        if let Some(&(state, _)) = self.rare.custom.states.get(&element) {
            return state;
        }
        match self.custom_name(element) {
            Some((Namespace::Html, local))
                if super::shadow::is_valid_custom_element_name(local)
                    || self.rare.custom.is_values.contains_key(&element) =>
            {
                CustomElementState::Undefined
            }
            _ => CustomElementState::Uncustomized,
        }
    }

    /// The definition an element is custom (or being upgraded) by.
    pub fn custom_element_definition(&self, element: NodeId) -> Option<u32> {
        self.rare
            .custom
            .states
            .get(&element)
            .and_then(|&(_, definition)| definition)
    }

    /// Sets an element's custom element state and definition: `:defined`
    /// is restyled when its answer changes.
    pub fn set_custom_element_state(
        &mut self,
        element: NodeId,
        state: CustomElementState,
        definition: Option<u32>,
    ) -> Result<(), MutationError> {
        if !self.is_element(element) {
            return Err(MutationError::WrongKind);
        }
        let was = self.is_defined(element);
        self.rare.custom.states.insert(element, (state, definition));
        if was != self.is_defined(element) {
            self.flip_state(element, ElementState::DEFINED_CHANGED);
        }
        if state == CustomElementState::Custom {
            self.form_associated_custom(element);
        }
        Ok(())
    }

    /// Whether `element` matches `:defined` (HTML section 4.16.3): its
    /// state is "uncustomized" or "custom".
    pub fn is_defined(&self, element: NodeId) -> bool {
        matches!(
            self.custom_element_state(element),
            CustomElementState::Uncustomized | CustomElementState::Custom
        )
    }

    /// Whether `element` is custom: reactions are queued for it.
    pub fn is_custom(&self, element: NodeId) -> bool {
        matches!(
            self.rare.custom.states.get(&element),
            Some((CustomElementState::Custom, _))
        )
    }

    /// An element's `is` value (DOM's), which a customized built-in
    /// element is looked up by and serialized with.
    pub fn is_value(&self, element: NodeId) -> Option<&str> {
        self.rare.custom.is_values.get(&element).map(String::as_str)
    }

    /// Sets an element's `is` value, which only its making does.
    pub fn set_is_value(&mut self, element: NodeId, is: Option<&str>) -> Result<(), MutationError> {
        if !self.is_element(element) {
            return Err(MutationError::WrongKind);
        }
        let was = self.is_defined(element);
        let old = self
            .rare
            .custom
            .is_values
            .get(&element)
            .map_or(0, String::len);
        match is {
            Some(is) => {
                self.check_data(is.len().saturating_sub(old))?;
                self.data_bytes = self.data_bytes + is.len() - old;
                self.rare.custom.is_values.insert(element, is.to_owned());
            }
            None => {
                self.data_bytes -= old;
                self.rare.custom.is_values.remove(&element);
            }
        }
        if was != self.is_defined(element) {
            self.flip_state(element, ElementState::DEFINED_CHANGED);
        }
        Ok(())
    }

    /// HTML's "create an element" with the synchronous custom elements
    /// flag unset, after the element was made (a copy, a fragment
    /// parser's element): with a definition for it in its registry, it is
    /// upgraded by a reaction.
    pub fn upgrade_if_defined(&mut self, element: NodeId) {
        let Some((ns, local)) = self.custom_name(element) else {
            return;
        };
        let registry = self.custom_element_registry(element);
        let found = self.look_up_custom_element(registry, ns, local, self.is_value(element));
        if let Some(definition) = found {
            self.rare.custom.reactions.push(Reaction::Upgrade {
                element,
                definition,
            });
        }
    }

    // ---- Custom states -----------------------------------------------------

    /// Whether `element` has custom state `name` (`:state(name)`).
    pub fn has_custom_state(&self, element: NodeId, name: &str) -> bool {
        self.rare
            .custom
            .custom_states
            .get(&element)
            .is_some_and(|states| states.iter().any(|state| state == name))
    }

    /// An element's custom states, in the order they were added.
    pub fn custom_states(&self, element: NodeId) -> &[String] {
        self.rare
            .custom
            .custom_states
            .get(&element)
            .map_or(&[], Vec::as_slice)
    }

    /// Adds custom state `name` to `element`, or with `on` false takes it
    /// away; whether that changed anything. `:state()` is restyled.
    pub fn set_custom_state(
        &mut self,
        element: NodeId,
        name: &str,
        on: bool,
    ) -> Result<bool, MutationError> {
        if !self.is_element(element) {
            return Err(MutationError::WrongKind);
        }
        let had = self.has_custom_state(element, name);
        if had == on {
            return Ok(false);
        }
        if on {
            if self.custom_states(element).len() >= MAX_CUSTOM_STATES {
                return Err(MutationError::TooMuchData);
            }
            self.check_data(name.len())?;
            self.data_bytes += name.len();
            self.rare
                .custom
                .custom_states
                .entry(element)
                .or_default()
                .push(name.to_owned());
        } else {
            self.data_bytes -= name.len();
            if let Some(states) = self.rare.custom.custom_states.get_mut(&element) {
                states.retain(|state| state != name);
                if states.is_empty() {
                    self.rare.custom.custom_states.remove(&element);
                }
            }
        }
        self.flip_state(element, ElementState::STATES_CHANGED);
        Ok(true)
    }

    /// Takes every custom state of `element` away.
    pub fn clear_custom_states(&mut self, element: NodeId) {
        if let Some(states) = self.rare.custom.custom_states.remove(&element) {
            self.data_bytes -= states.iter().map(String::len).sum::<usize>();
            self.flip_state(element, ElementState::STATES_CHANGED);
        }
    }

    /// Journals that `:defined` or `:state()` may answer otherwise for
    /// `element`: a change of its state flags from flags with `flag` set
    /// ([`ElementState::CUSTOM_CHANGES`]).
    fn flip_state(&mut self, element: NodeId, flag: ElementState) {
        let old = self.state(element).with(flag, true);
        self.record_change(super::Change::StateChanged { element, old });
    }

    // ---- Reactions ---------------------------------------------------------

    /// The reactions queued since they were last taken, in order.
    pub fn take_reactions(&mut self) -> Vec<Reaction> {
        self.settle_form_associated();
        std::mem::take(&mut self.rare.custom.reactions)
    }

    /// Whether reactions are queued: those of the tree's changes, and
    /// those a form-associated custom element's new form owner or
    /// disabledness makes.
    pub fn has_reactions(&mut self) -> bool {
        self.settle_form_associated();
        !self.rare.custom.reactions.is_empty()
    }

    // ---- Form-associated custom elements -----------------------------------

    /// Whether `element` is a form-associated custom element (HTML section
    /// 4.13.7.3): custom by a definition that is form-associated.
    pub fn is_form_associated_custom(&self, element: NodeId) -> bool {
        self.rare.custom.faces.contains_key(&element)
    }

    /// A custom element became custom, upgraded or constructed: one of a
    /// form-associated definition is told its form owner and that it is
    /// disabled, when it is (HTML's "upgrade an element" step 12), and
    /// from then on whenever they change.
    fn form_associated_custom(&mut self, element: NodeId) {
        let is_face = self
            .custom_element_definition(element)
            .and_then(|id| self.custom_definition(id))
            .is_some_and(|definition| definition.form_associated);
        if !is_face || self.rare.custom.faces.contains_key(&element) {
            return;
        }
        self.rare.custom.faces.insert(element, Face::default());
        self.rare.custom.faces_dirty = true;
        self.settle_form_associated();
    }

    /// A form-associated custom element's submission value.
    pub fn form_value(&self, element: NodeId) -> &FormValue {
        static NONE: FormValue = FormValue::None;
        self.rare
            .custom
            .faces
            .get(&element)
            .map_or(&NONE, |face| &face.value)
    }

    /// Sets a form-associated custom element's submission value.
    pub fn set_form_value(
        &mut self,
        element: NodeId,
        value: FormValue,
    ) -> Result<(), MutationError> {
        let Some(face) = self.rare.custom.faces.get(&element) else {
            return Err(MutationError::WrongKind);
        };
        let old = face.value.bytes();
        let new = value.bytes();
        self.check_data(new.saturating_sub(old))?;
        self.data_bytes = self.data_bytes + new - old;
        if let Some(face) = self.rare.custom.faces.get_mut(&element) {
            face.value = value;
        }
        Ok(())
    }

    /// A form-associated custom element's validity flags and validation
    /// message, as `setValidity()` set them.
    pub fn form_validity(&self, element: NodeId) -> Option<(u32, &str)> {
        self.rare
            .custom
            .faces
            .get(&element)
            .map(|face| (face.validity, face.message.as_str()))
    }

    /// Sets a form-associated custom element's validity: `:valid` and
    /// `:invalid` are restyled.
    pub fn set_form_validity(
        &mut self,
        element: NodeId,
        flags: u32,
        message: &str,
    ) -> Result<(), MutationError> {
        let Some(face) = self.rare.custom.faces.get(&element) else {
            return Err(MutationError::WrongKind);
        };
        let old = face.message.len();
        self.check_data(message.len().saturating_sub(old))?;
        self.data_bytes = self.data_bytes + message.len() - old;
        if let Some(face) = self.rare.custom.faces.get_mut(&element) {
            face.validity = flags;
            face.message = message.to_owned();
        }
        self.record_change(super::Change::ControlChanged { element });
        Ok(())
    }

    /// A form owning `element` was reset (HTML section 4.10.22): its
    /// `formResetCallback`.
    pub(crate) fn form_reset_custom(&mut self, element: NodeId) {
        if self.is_form_associated_custom(element)
            && self.callback_of(element, callbacks::FORM_RESET).is_some()
        {
            self.rare
                .custom
                .reactions
                .push(Reaction::FormReset(element));
        }
    }

    /// Where `node` stands in shadow-including tree order: its
    /// shadow-including root, then each step down from it, a host's shadow
    /// root before its children. Compared, the positions of a tree's
    /// nodes are in that order.
    fn tree_position(&self, node: NodeId) -> (u32, Vec<u32>) {
        let mut steps = Vec::new();
        let mut at = node;
        loop {
            if let Some(parent) = self.parent(at) {
                let index = self.children(parent).position(|child| child == at);
                steps.push(index.map_or(0, |i| i as u32 + 1));
                at = parent;
            } else if let Some(host) = self.shadow_host(at) {
                steps.push(0);
                at = host;
            } else {
                break;
            }
        }
        steps.reverse();
        (at.index() as u32, steps)
    }

    /// The tree changed: what form-associated custom elements were told
    /// is compared again before reactions are next taken.
    pub(super) fn custom_changed(&mut self) {
        if !self.rare.custom.faces.is_empty() {
            self.rare.custom.faces_dirty = true;
        }
    }

    /// Tells each form-associated custom element whose form owner or
    /// disabledness is other than it was told (HTML's "reset the form
    /// owner", and a change of disabledness), in shadow-including tree
    /// order.
    fn settle_form_associated(&mut self) {
        if !std::mem::take(&mut self.rare.custom.faces_dirty) {
            return;
        }
        let mut elements: Vec<NodeId> = self.rare.custom.faces.keys().copied().collect();
        elements.sort_by_cached_key(|&node| self.tree_position(node));
        for element in elements {
            let owner = crate::forms::form_owner(self, element);
            let disabled = crate::forms::actually_disabled(self, element);
            let Some(face) = self.rare.custom.faces.get_mut(&element) else {
                continue;
            };
            let moved = face.owner != owner;
            let toggled = face.disabled != disabled;
            face.owner = owner;
            face.disabled = disabled;
            if moved
                && self
                    .callback_of(element, callbacks::FORM_ASSOCIATED)
                    .is_some()
            {
                self.rare.custom.reactions.push(Reaction::FormAssociated {
                    element,
                    form: owner,
                });
            }
            if toggled
                && self
                    .callback_of(element, callbacks::FORM_DISABLED)
                    .is_some()
            {
                self.rare
                    .custom
                    .reactions
                    .push(Reaction::FormDisabled { element, disabled });
            }
        }
    }

    /// Queues a reaction the bindings asked for: one a form's change
    /// causes, or an upgrade.
    pub fn queue_reaction(&mut self, reaction: Reaction) {
        if self.contains(reaction.element()) {
            self.rare.custom.reactions.push(reaction);
        }
    }

    /// Forgets every definition, state and queued reaction: the realm
    /// whose registries they were is gone. The elements keep the states
    /// their names imply.
    pub fn forget_custom_elements(&mut self) {
        let custom = std::mem::take(&mut self.rare.custom);
        let size = custom
            .definitions
            .iter()
            .map(|d| {
                d.name.len()
                    + d.local_name.len()
                    + d.observed.iter().map(String::len).sum::<usize>()
            })
            .sum::<usize>()
            + custom.is_values.values().map(String::len).sum::<usize>()
            + custom
                .custom_states
                .values()
                .flatten()
                .map(String::len)
                .sum::<usize>()
            + custom.faces.values().map(Face::bytes).sum::<usize>();
        self.data_bytes -= size;
        // What `is` values say of an element's interface stays.
        let mut kept = Custom {
            is_values: custom.is_values,
            ..Custom::default()
        };
        self.data_bytes += kept.is_values.values().map(String::len).sum::<usize>();
        std::mem::swap(&mut self.rare.custom, &mut kept);
    }

    /// The definition `element` is custom by, when it has the callback
    /// `flag`.
    fn callback_of(&self, element: NodeId, flag: u8) -> Option<&Definition> {
        let (state, definition) = self.rare.custom.states.get(&element)?;
        if *state != CustomElementState::Custom {
            return None;
        }
        let definition = self.custom_definition((*definition)?)?;
        (definition.callbacks & flag != 0).then_some(definition)
    }

    /// After `node` was inserted (DOM's "insert" step 7.7, and "adopt"):
    /// each custom element of its shadow-including inclusive descendants
    /// adopted from `old_owner` when that was another document, then,
    /// connected, each custom one's `connectedCallback` and each other
    /// one tried for an upgrade, in shadow-including tree order.
    pub(super) fn custom_inserted(&mut self, node: NodeId, old_owner: Option<NodeId>) {
        crate::hook!("dom::custom::custom_inserted");
        if old_owner.is_none() && !self.rare.custom.registries.is_empty() {
            self.registries_inserted(node);
        }
        if !self.rare.custom.active() {
            return;
        }
        let nodes = self.shadow_including_subtree(node);
        if let Some(old) = old_owner {
            self.custom_adopted(&nodes, old);
        }
        if !self.is_connected(node) {
            return;
        }
        for element in nodes {
            if !self.is_element(element) {
                continue;
            }
            match self.custom_element_state(element) {
                CustomElementState::Custom => {
                    if self.callback_of(element, callbacks::CONNECTED).is_some() {
                        self.rare
                            .custom
                            .reactions
                            .push(Reaction::Connected(element));
                    }
                }
                CustomElementState::Undefined => self.upgrade_if_defined(element),
                _ => {}
            }
        }
    }

    /// A node a parser made and inserts at once (`old_owner` none): each
    /// element of its shadow-including inclusive descendants with no
    /// registry of its own takes its parent's, as it is made with its
    /// intended parent's (HTML's "create an element for a token" step
    /// 2). A shadow root keeps its own. DOM's "insert" gives a node a
    /// script inserts none: its registry changes as it is adopted alone
    /// ([`Document::registries_before_adoption`]).
    fn registries_inserted(&mut self, node: NodeId) {
        for element in self.shadow_including_subtree(node) {
            // One given a registry, or none, keeps it: a copy's.
            if !self.is_element(element) || self.rare.custom.registries.contains_key(&element) {
                continue;
            }
            let parent = self
                .host_or_parent(element)
                .and_then(|parent| self.custom_element_registry(parent));
            if parent != self.custom_element_registry(element) {
                self.rare.custom.registries.insert(element, parent);
            }
        }
    }

    /// A fragment's parser made the elements under `container` (HTML
    /// section 13.4), in the context of a node whose registry is
    /// `registry` (HTML's "create an element for a token" step 2): each
    /// takes its intended parent's, that one under the container, and is
    /// upgraded by a reaction when it defines it.
    pub fn fragment_parsed(&mut self, container: NodeId, registry: Option<u32>) {
        let elements: Vec<NodeId> = self
            .subtree(container)
            .skip(1)
            .filter(|&node| self.is_element(node))
            .collect();
        for element in elements {
            // One the parser gave none keeps none; another takes its
            // parent's, the context's under the container.
            if !self.has_own_custom_element_registry(element) {
                let parent = self.parent(element).filter(|&parent| parent != container);
                let wanted = match parent {
                    Some(parent) => self.custom_element_registry(parent),
                    None => registry,
                };
                if self.custom_element_registry(element) != wanted {
                    self.rare.custom.registries.insert(element, wanted);
                }
            }
            self.upgrade_if_defined(element);
        }
    }

    /// `nodes`, a subtree in shadow-including tree order, were adopted
    /// from document `old` (DOM's "adopt" step 3.3): each custom element's
    /// `adoptedCallback`, when its document is another now.
    pub(super) fn custom_adopted(&mut self, nodes: &[NodeId], old: NodeId) {
        let Some(&first) = nodes.first() else {
            return;
        };
        let new = self.owner_document(first);
        if new == old {
            return;
        }
        for &element in nodes {
            if self.callback_of(element, callbacks::ADOPTED).is_some() {
                self.rare
                    .custom
                    .reactions
                    .push(Reaction::Adopted { element, old, new });
            }
        }
    }

    /// `node` was removed from a connected tree (DOM's "remove" steps 15
    /// and 16): each custom element of its shadow-including inclusive
    /// descendants hears `disconnectedCallback`.
    pub(super) fn custom_removed(&mut self, node: NodeId) {
        crate::hook!("dom::custom::custom_removed");
        if self.rare.custom.states.is_empty() {
            return;
        }
        for element in self.shadow_including_subtree(node) {
            if self.callback_of(element, callbacks::DISCONNECTED).is_some() {
                self.rare
                    .custom
                    .reactions
                    .push(Reaction::Disconnected(element));
            }
        }
    }

    /// `node` was moved within a connected tree (DOM's "move" step 13):
    /// each custom element of its shadow-including inclusive descendants
    /// hears `connectedMoveCallback`, or `disconnectedCallback` and
    /// `connectedCallback` without one.
    pub(super) fn custom_moved(&mut self, node: NodeId) {
        crate::hook!("dom::custom::custom_moved");
        if self.rare.custom.states.is_empty() || !self.is_connected(node) {
            return;
        }
        let any = callbacks::CONNECTED_MOVE | callbacks::CONNECTED | callbacks::DISCONNECTED;
        for element in self.shadow_including_subtree(node) {
            if self.callback_of(element, any).is_some() {
                self.rare
                    .custom
                    .reactions
                    .push(Reaction::ConnectedMove(element));
            }
        }
    }

    /// Attribute `name` (as the element keeps it) of namespace
    /// `namespace` of `element` changed from `old` to `value` (DOM's
    /// "handle attribute changes" step 2): its `attributeChangedCallback`
    /// when it observes the attribute's local name.
    pub(super) fn custom_attribute_changed(
        &mut self,
        element: NodeId,
        name: &str,
        namespace: Option<&str>,
        old: Option<&str>,
        value: Option<&str>,
    ) {
        crate::hook!("dom::custom::custom_attribute_changed");
        if self.rare.custom.states.is_empty() {
            return;
        }
        let local = super::attributes::attribute_local_name(namespace, name);
        let Some(definition) = self.callback_of(element, callbacks::ATTRIBUTE_CHANGED) else {
            return;
        };
        if !definition.observed.iter().any(|observed| observed == local) {
            return;
        }
        let reaction = Reaction::AttributeChanged {
            element,
            name: local.to_owned(),
            old: old.map(str::to_owned),
            value: value.map(str::to_owned),
            namespace: namespace.map(str::to_owned),
        };
        self.rare.custom.reactions.push(reaction);
    }

    /// Forgets what is kept of the nodes `doomed`, which are being freed.
    pub(super) fn forget_custom(&mut self, doomed: &[NodeId]) {
        crate::hook!("dom::custom::forget_custom");
        let custom = &mut self.rare.custom;
        if custom.states.is_empty()
            && custom.is_values.is_empty()
            && custom.registries.is_empty()
            && custom.custom_states.is_empty()
            && custom.faces.is_empty()
        {
            return;
        }
        let mut freed = 0;
        for node in doomed {
            custom.keep_null.remove(node);
            if let Some(face) = custom.faces.remove(node) {
                freed += face.bytes();
            }
            custom.states.remove(node);
            custom.registries.remove(node);
            if let Some(is) = custom.is_values.remove(node) {
                freed += is.len();
            }
            if let Some(states) = custom.custom_states.remove(node) {
                freed += states.iter().map(String::len).sum::<usize>();
            }
        }
        let gone: std::collections::HashSet<NodeId> = doomed.iter().copied().collect();
        custom
            .reactions
            .retain(|reaction| !gone.contains(&reaction.element()));
        self.data_bytes -= freed;
    }

    /// The bytes the custom element data holds, for
    /// [`Document::validate`].
    pub(super) fn custom_data_bytes(&self) -> usize {
        let custom = &self.rare.custom;
        custom
            .definitions
            .iter()
            .map(|d| {
                d.name.len()
                    + d.local_name.len()
                    + d.observed.iter().map(String::len).sum::<usize>()
            })
            .sum::<usize>()
            + custom.is_values.values().map(String::len).sum::<usize>()
            + custom
                .custom_states
                .values()
                .flatten()
                .map(String::len)
                .sum::<usize>()
            + custom.faces.values().map(Face::bytes).sum::<usize>()
    }

    /// Checks that what is kept of custom elements names live elements,
    /// for [`Document::validate`].
    pub(super) fn validate_custom(&self) -> Result<(), String> {
        let custom = &self.rare.custom;
        for node in custom
            .states
            .keys()
            .chain(custom.is_values.keys())
            .chain(custom.custom_states.keys())
            .chain(custom.faces.keys())
        {
            if !matches!(self.data(*node), Some(NodeData::Element { .. })) {
                return Err(format!(
                    "custom element data names {node:?}, not an element"
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defined(document: &mut Document, name: &str, local: &str, callbacks: u8) -> u32 {
        document
            .define_custom_element(Definition {
                registry: WINDOW_REGISTRY,
                name: name.to_owned(),
                local_name: local.to_owned(),
                observed: vec!["title".to_owned()],
                callbacks,
                form_associated: false,
                disable_shadow: false,
            })
            .unwrap()
    }

    #[test]
    fn a_name_implies_undefined_until_a_state_is_given() {
        let mut document = Document::new();
        let custom = document.create_element("x-a").unwrap();
        let plain = document.create_element("div").unwrap();
        assert_eq!(
            document.custom_element_state(custom),
            CustomElementState::Undefined
        );
        assert!(!document.is_defined(custom));
        assert!(document.is_defined(plain));
        document.set_is_value(plain, Some("x-b")).unwrap();
        assert!(!document.is_defined(plain), "an is value is undefined too");
        document.take_changes();
        document
            .set_custom_element_state(custom, CustomElementState::Custom, None)
            .unwrap();
        assert!(document.is_defined(custom));
        assert_eq!(
            document.take_changes(),
            [crate::Change::StateChanged {
                element: custom,
                old: ElementState::DEFINED_CHANGED
            }],
            "journaled for :defined's restyle"
        );
        assert_eq!(document.state(custom), ElementState::NONE);
        assert!(document.validate().is_ok());
    }

    #[test]
    fn definitions_are_looked_up_by_registry_name_and_is_value() {
        let mut document = Document::new();
        let a = defined(&mut document, "x-a", "x-a", 0);
        let b = defined(&mut document, "x-b", "button", 0);
        let html = Namespace::Html;
        let window = Some(WINDOW_REGISTRY);
        assert_eq!(
            document.look_up_custom_element(window, &html, "x-a", None),
            Some(a)
        );
        assert_eq!(
            document.look_up_custom_element(window, &html, "button", Some("x-b")),
            Some(b)
        );
        assert_eq!(
            document.look_up_custom_element(window, &html, "div", Some("x-b")),
            None
        );
        assert_eq!(
            document.look_up_custom_element(None, &html, "x-a", None),
            None
        );
        assert_eq!(
            document.look_up_custom_element(Some(2), &html, "x-a", None),
            None
        );
        assert_eq!(
            document.look_up_custom_element(window, &Namespace::Svg, "x-a", None),
            None
        );
        assert!(
            document
                .define_custom_element(Definition {
                    registry: WINDOW_REGISTRY,
                    name: "x-a".into(),
                    local_name: "x-a".into(),
                    observed: Vec::new(),
                    callbacks: 0,
                    form_associated: false,
                    disable_shadow: false,
                })
                .is_err(),
            "a name is defined once in a registry"
        );
    }

    #[test]
    fn tree_changes_queue_the_reactions_of_custom_elements_alone() {
        let mut document = Document::new();
        let all = callbacks::CONNECTED
            | callbacks::DISCONNECTED
            | callbacks::ATTRIBUTE_CHANGED
            | callbacks::ADOPTED;
        let a = defined(&mut document, "x-a", "x-a", all);
        let html = document.create_element("html").unwrap();
        let root = document.root();
        document.append_child(root, html).unwrap();
        let custom = document.create_element("x-a").unwrap();
        let waiting = document.create_element("x-a").unwrap();
        document
            .set_custom_element_state(custom, CustomElementState::Custom, Some(a))
            .unwrap();
        document.set_attribute(custom, "title", "t").unwrap();
        document.set_attribute(custom, "lang", "en").unwrap();
        document.append_child(html, custom).unwrap();
        document.append_child(custom, waiting).unwrap();
        document.set_attribute(custom, "title", "t").unwrap();
        document.remove_child(html, custom).unwrap();
        assert_eq!(
            document.take_reactions(),
            [
                Reaction::AttributeChanged {
                    element: custom,
                    name: "title".into(),
                    old: None,
                    value: Some("t".into()),
                    namespace: None,
                },
                Reaction::Connected(custom),
                Reaction::Upgrade {
                    element: waiting,
                    definition: a
                },
                Reaction::AttributeChanged {
                    element: custom,
                    name: "title".into(),
                    old: Some("t".into()),
                    value: Some("t".into()),
                    namespace: None,
                },
                Reaction::Disconnected(custom),
            ]
        );
        // Detached, an insertion connects nothing.
        let detached = document.create_element("div").unwrap();
        document.append_child(detached, custom).unwrap();
        assert!(!document.has_reactions());
    }

    #[test]
    fn freeing_an_element_forgets_its_custom_data() {
        let mut document = Document::new();
        let a = defined(&mut document, "x-a", "x-a", callbacks::DISCONNECTED);
        let element = document.create_element("x-a").unwrap();
        document
            .set_custom_element_state(element, CustomElementState::Custom, Some(a))
            .unwrap();
        document.set_custom_state(element, "on", true).unwrap();
        document.set_is_value(element, Some("x-a")).unwrap();
        let bytes = document.data_bytes();
        document.free(element).unwrap();
        assert!(document.data_bytes() < bytes);
        assert!(document.validate().is_ok());
        assert_eq!(document.custom_states(element), &[] as &[String]);
    }
}
