//! What crosses the boundary when a script reaches the document's nodes
//! (M25, `docs/design/dom-scripting.md`): handles, the calls the
//! bindings make of the host, and their answers. Plain data, as M24's
//! boundary is: no object of the engine's, no JavaScript value.

/// The work a step of the budget pays for: a step for every 10,000
/// units of work the host reports, a unit about a node visited or 64
/// bytes copied (M25), so that a loop of expensive native calls runs
/// out of steps as a loop of cheap operations does.
pub const WORK_PER_STEP: u64 = 10_000;

/// A node as a script holds it: a number below 2^53, which the host
/// makes from the node's id and resolves back, checking it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle(pub u64);

/// The largest number a handle may be: what a script's number holds
/// exactly.
pub const MAX_HANDLE: u64 = (1 << 53) - 1;

/// A call the bindings make of the host, one per native function.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    /// The document node.
    Document,
    /// The bindings make an object for the node, which the host then
    /// holds until it is released: the node's name, its local name for
    /// an element, `#text` or `#document`.
    Wrap(Handle),
    /// The node's `nodeType`: 1 for an element, 3 for text, 9 for the
    /// document.
    Kind(Handle),
    /// An element's tag name, lowercase.
    Tag(Handle),
    /// The node's parent, or `None`.
    Parent(Handle),
    FirstChild(Handle),
    LastChild(Handle),
    PreviousSibling(Handle),
    NextSibling(Handle),
    /// The node's children, in order.
    Children(Handle),
    /// The node's element children, in order.
    ElementChildren(Handle),
    /// The document's version, a count of its changes, which a live
    /// collection compares to know whether its items still stand.
    Version,
    /// Whether the node is in the document.
    Connected(Handle),
    /// Whether `other` is `node` or one of its descendants.
    Contains {
        node: Handle,
        other: Handle,
    },
    /// A text node's data.
    Data(Handle),
    /// Sets a text node's data.
    SetData(Handle, String),
    /// The text of the node's subtree in tree order, or `None` for the
    /// document (the DOM's `textContent`).
    TextContent(Handle),
    /// The DOM's `textContent` setter: an element's children replaced by
    /// one text node, or by none for the empty string; a text node's
    /// data set; nothing for the document.
    SetTextContent(Handle, String),
    /// An element's attribute by its name, which the bindings have
    /// lowercased, or `None`.
    Attribute(Handle, String),
    /// Sets an element's attribute, its name checked and lowercased.
    SetAttribute(Handle, String, String),
    /// Removes an element's attribute.
    RemoveAttribute(Handle, String),
    /// An element's attributes' names, in order.
    AttributeNames(Handle),
    /// A new element with this local name, which the host checks and
    /// lowercases.
    CreateElement(String),
    /// A new text node.
    CreateText(String),
    /// Inserts `node` under `parent` before `before`, or last: the DOM's
    /// "pre-insert", `node` taken from where it was first.
    Insert {
        parent: Handle,
        node: Handle,
        before: Option<Handle>,
    },
    /// Removes `node` from `parent`, whose child it must be.
    Remove {
        parent: Handle,
        node: Handle,
    },
    /// Puts `node` in the place of `child`, a child of `parent`: the
    /// DOM's "replace a child".
    Replace {
        parent: Handle,
        node: Handle,
        child: Handle,
    },
    /// A copy of the node, and of its descendants if `deep`, detached.
    Clone {
        node: Handle,
        deep: bool,
    },
    /// The first element in tree order under `root` whose ID is this.
    ElementById(Handle, String),
    /// The elements under `root`, in tree order, that a selector list
    /// matches, `:scope` being `root` if it is an element (the root
    /// element otherwise); the first alone unless `all`. A list that does
    /// not parse is a `SyntaxError`.
    Query {
        root: Handle,
        selectors: String,
        all: bool,
    },
    /// Whether a selector list matches the element, `:scope` being it.
    Matches(Handle, String),
    /// The element's nearest inclusive ancestor a selector list matches.
    Closest(Handle, String),
    /// The elements under the node with this tag (`*` for all), in tree
    /// order.
    ElementsByTagName(Handle, String),
    /// The elements under the node with every one of these classes
    /// (white-space separated), in tree order.
    ElementsByClassName(Handle, String),
    /// The markup of the node's children (HTML's fragment serialization).
    InnerHtml(Handle),
    /// The markup of the node and its children.
    OuterHtml(Handle),
    /// The element's children replaced by what the markup parses to as a
    /// fragment in its context.
    SetInnerHtml(Handle, String),
    /// The element replaced by what the markup parses to as a fragment in
    /// its parent's context; nothing without a parent, and a
    /// `NoModificationAllowedError` for the document's element.
    SetOuterHtml(Handle, String),
    /// What the markup parses to inserted at a position by the element
    /// (`beforebegin`, `afterbegin`, `beforeend` or `afterend`).
    InsertAdjacentHtml {
        node: Handle,
        position: String,
        markup: String,
    },
    /// The script holds no object for the node any longer.
    Released(Handle),
}

/// What the host answers a [`Call`] with.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// `undefined`.
    Nothing,
    Bool(bool),
    Number(f64),
    /// A node, or `null`.
    Node(Option<Handle>),
    Nodes(Vec<Handle>),
    String(String),
    /// A string, or `null`.
    MaybeString(Option<String>),
    Strings(Vec<String>),
}

/// Why a call failed: the exception the script gets, a `DOMException`
/// by its name (`HierarchyRequestError`, `NotFoundError`, ...) or, for
/// `TypeError` and `RangeError`, one of ECMAScript's; and the reason,
/// which the bindings put in the reference browser's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomError {
    pub name: &'static str,
    pub message: String,
}

impl DomError {
    pub fn new(name: &'static str, message: impl Into<String>) -> DomError {
        DomError {
            name,
            message: message.into(),
        }
    }

    /// The call is not one this host answers.
    pub fn not_supported() -> DomError {
        DomError::new("NotSupportedError", "This operation is not supported.")
    }
}
