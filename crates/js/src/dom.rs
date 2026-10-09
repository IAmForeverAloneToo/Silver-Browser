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
    /// A text node's data.
    Data(Handle),
    /// The text of the node's subtree in tree order, or `None` for the
    /// document (the DOM's `textContent`).
    TextContent(Handle),
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
