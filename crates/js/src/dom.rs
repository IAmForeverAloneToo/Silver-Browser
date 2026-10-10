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

/// A script to run nested, inside the native call that made it ready
/// (HTML: a script a script inserted, or a parser-inserted one in what
/// `document.write` wrote, runs before the call returns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NestedScript {
    pub source: String,
    /// The URL its errors name.
    pub url: String,
    /// The line its text starts on.
    pub line: u32,
}

/// A request a script's `fetch()` asks for (M27.1), as the bindings
/// made it: the engine trusts none of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchCall {
    pub id: u32,
    pub method: String,
    /// Already parsed against the document's base URL.
    pub url: String,
    /// Names and values, one after the other.
    pub headers: Vec<String>,
    pub body: Option<Vec<u8>>,
    pub mode: String,
    pub credentials: String,
    pub redirect: String,
    /// Fetch's use-CORS-preflight flag.
    pub preflight: bool,
}

/// A call the bindings make of the host, one per native function.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    /// The document node.
    Document,
    /// The bindings make an object for the node, which the host then
    /// holds until it is released: the node's name, its local name for
    /// an element, `#text` or `#document`; `connected` or empty; then the
    /// names of its attributes that start with `on`, which may be event
    /// handlers (M26), as strings.
    Wrap(Handle),
    /// The nodes scripts hold whose place in the document changed since
    /// this was last asked, each with whether it is now in the document:
    /// the bindings hold the object of a node in the document as the
    /// document does, and let go of it when it leaves.
    TakePins,
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
    /// `document.write`: the text put into the parser's input at its
    /// insertion point, and parsed there (HTML section 8.4.3).
    Write(String),
    /// The script element whose script is running, if any.
    CurrentScript,
    /// `document.readyState`: `loading`, `interactive` or `complete`.
    ReadyState,
    /// The text of an element's text children alone (HTML's "child text
    /// content", a script's `text`).
    ChildText(Handle),
    /// A script element's "force async" flag.
    ForceAsync(Handle),
    /// Sets a script element's "force async" flag (its `async` setter).
    SetForceAsync(Handle, bool),
    /// The declarations of a declaration block's text that the cascade
    /// would take, as name, value and `important` or empty, each three in
    /// a row: what a `style` object reads.
    ParseDeclarations(String),
    /// The name the cascade knows a property by: an alias's property, a
    /// name in any case lowercased, a custom property's as it is; `None`
    /// for one it does not know.
    CssCanonical(String),
    /// Whether a value is valid for a property.
    CssValid(String, String),
    /// A URL parsed against the document's base URL, or `None` when it
    /// does not parse (a URL-reflecting attribute's getter).
    ResolveUrl(String),
    /// The document's URL, if it has one.
    DocumentUrl,
    /// The serialization of the document's origin (M27.2), what
    /// `self.origin` gives.
    Origin,
    /// Whether the document is a secure context (`isSecureContext`).
    SecureContext,
    /// `fetch()` (M27.1): the request a script asks for, which the
    /// engine checks again and starts once the run ends; its answer is a
    /// `Task::Fetched` of the same `id`. A `TypeError` for one it refuses.
    Fetch(Box<FetchCall>),
    /// Fetch `id` aborted by its signal: the engine stops it.
    AbortFetch(u32),
    /// Whether a URL includes credentials, which a Request refuses.
    UrlHasCredentials(String),
    /// Whether a URL is of the document's origin (a Request's referrer).
    SameOriginUrl(String),
    /// A form's entry list, as `FormData`'s constructor makes it
    /// (M27): each entry's name, `text` or `file`, and value; a
    /// `TypeError` for an element that is not a form or a submitter that
    /// is not a submit button, a `NotFoundError` for a submitter the
    /// form does not own.
    FormEntries {
        form: Handle,
        submitter: Option<Handle>,
    },
    /// The platform's line ending, for a Blob's `native` endings.
    PlatformNewline,
    /// `document.cookie` (HTML 3.1.4, M27.4): the cookies a script may
    /// read for the document's URL, the empty string for a document
    /// whose URL is not `http:` or `https:`.
    Cookie,
    /// `document.cookie` set: one cookie stored as a script stores it,
    /// nothing for a document whose URL is not `http:` or `https:`.
    SetCookie(String),
    /// `document.domain`: the origin's host, or the empty string for an
    /// opaque origin.
    DocumentDomain,
    /// `document.domain` set: a `SecurityError` unless the value is the
    /// host or a registrable suffix of it, and then nothing, as in an
    /// origin-keyed agent cluster (HTML 7.1.1.2).
    SetDocumentDomain(String),
    /// The document's URL in parts, as `Location` gives them: `href`,
    /// `origin`, `protocol`, `host`, `hostname`, `port`, `pathname`,
    /// `search`, `hash`.
    Location,
    /// A navigation to a URL parsed against the document's base URL,
    /// which the engine starts after the run; a `SyntaxError` for one that
    /// does not parse.
    Navigate {
        url: String,
        replace: bool,
    },
    /// The document fetched again, after the run.
    Reload,
    /// What `navigator` reports (D12): its user agent string, its
    /// platform, and its compatibility mode, `gecko` or `chrome`.
    NavigatorInfo,
    /// The viewport's width and height in CSS pixels, and the device
    /// pixel ratio.
    Viewport,
    /// The document's URL, character encoding, content type and
    /// compatibility mode.
    DocumentInfo,
    /// An element's, or its pseudo-element's (`before`, `after`,
    /// `first-letter`, `first-line`, `marker`; empty for the element),
    /// computed value of a property, serialized; `None` when there is
    /// none to give.
    ComputedValue {
        node: Handle,
        pseudo: String,
        property: String,
    },
    /// The properties a computed style lists.
    ComputedProperties,
    /// The script holds no object for the node any longer.
    Released(Handle),
    /// Timer `id` is set (M26, HTML section 8.6): its callback is to
    /// run as a task once `delay` milliseconds have passed, the host
    /// clamping the delay as HTML says. A timer set again with its id
    /// replaces itself.
    SetTimer {
        id: u32,
        delay: i32,
    },
    /// Timer `id` is cleared, if it is set.
    ClearTimer(u32),
    /// `performance.now()`: the milliseconds since the document's time
    /// origin.
    Now,
    /// `performance.timeOrigin`: the time origin, in milliseconds since
    /// the Unix epoch.
    TimeOrigin,
    /// The node's ancestors, its parent first (M26): an event's path.
    Ancestors(Handle),
    /// The nearest inclusive ancestor of the node with an activation
    /// behaviour (DOM section 2.9, HTML section 6.5), if any.
    Activator(Handle),
    /// Runs the element's activation behaviour, a click at it not
    /// canceled: a link is followed after the run.
    Activate(Handle),
    /// Whether the element is a form control that is disabled (HTML
    /// section 4.16.3), which `click()` does nothing for.
    Disabled(Handle),
    /// The element's form owner (HTML section 4.10.18.3), if it is
    /// form-associated and has one: an event handler's scope.
    FormOwner(Handle),
    /// A page asked for an animation frame callback: a frame is wanted
    /// (HTML section 8.10).
    WantFrame,
    /// Whether the element is a focusable area (HTML section 6.6.2, M26):
    /// it can take the focus, and is being rendered.
    Focusable(Handle),
    /// Whether the element can take the focus as it is, with no
    /// `tabindex`: `tabIndex`'s default.
    Tabbable(Handle),
    /// The focused element, if any.
    Focused,
    /// Focuses the element, or nothing; the host decides whether the
    /// focus is to be shown, and scrolls the element into view after the
    /// run unless `prevent_scroll`. The bindings fire the events.
    SetFocus {
        node: Option<Handle>,
        prevent_scroll: bool,
    },
    /// The focused element loses the focus on its way to the element
    /// (M26.11): `focus-within` stays with the ancestors they share
    /// while `blur` and `focusout` are dispatched.
    BlurToward(Handle),
    /// `document.hasFocus()`: whether the window has the system's focus.
    HasFocus,
    /// A form control's `value` (HTML section 4.10, M26.7): an
    /// `input`'s by its value mode, a `textarea`'s API value, a
    /// `select`'s first selected option's, an option's.
    ControlValue(Handle),
    /// An `input`'s value mode: `value`, `default`, `default/on` or
    /// `filename`; `value` for a `textarea`, empty for anything else.
    ValueMode(Handle),
    /// Sets an `input`'s value in the value mode, sanitized, or a
    /// `textarea`'s, as a script's `value` does.
    SetControlValue {
        node: Handle,
        value: String,
    },
    /// An `input`'s type, its keyword in lowercase.
    InputType(Handle),
    /// Whether an `input` is checked.
    Checkedness(Handle),
    /// Checks or unchecks an `input` as a script does: dirty, a radio
    /// button checked unchecking its group.
    SetCheckedness {
        node: Handle,
        checked: bool,
    },
    Indeterminate(Handle),
    SetIndeterminate {
        node: Handle,
        indeterminate: bool,
    },
    /// Whether an option is selected.
    Selectedness(Handle),
    /// Selects an option or not, as a script does.
    SetSelectedness {
        node: Handle,
        selected: bool,
    },
    /// A `select`'s list of options.
    Options(Handle),
    /// The `select` an option is in the list of, if any.
    OwnerSelect(Handle),
    /// A `select`'s first selected option's index, or -1.
    SelectedIndex(Handle),
    /// An option's text: its descendant text, stripped and collapsed.
    OptionText(Handle),
    /// A text control's selection, in UTF-16 code units: its start, its
    /// end and its direction (0 none, 1 forward, 2 backward).
    TextSelection(Handle),
    /// Sets a text control's selection, in UTF-16 code units.
    SetTextSelection {
        node: Handle,
        start: u32,
        end: u32,
        direction: u8,
    },
    /// A form's listed controls (HTML section 4.10.3), in tree order.
    FormControls(Handle),
    /// The control a `label` labels, if any.
    LabeledControl(Handle),
    /// The labels of a labelable element, in tree order.
    Labels(Handle),
    /// Whether the user changed a text control's value since it took the
    /// focus, the edit committed now (M26.8): `change` is to fire.
    CommitEdit(Handle),
    /// A radio button's group, in tree order.
    RadioGroup(Handle),
    /// Submits a form with its submitter, if any, the events before it
    /// the bindings' (M26.9): the navigation its plan makes, after the
    /// run.
    SubmitForm {
        form: Handle,
        submitter: Option<Handle>,
    },
    /// Resets a form's controls, its `reset` event the bindings'.
    ResetForm(Handle),
    /// A control's validity states as bits, `valueMissing` the lowest
    /// (M26.9b).
    Validity(Handle),
    /// Whether a control is a candidate for constraint validation.
    WillValidate(Handle),
    /// The reference browser's message for a control's first failure.
    ValidationMessage(Handle),
    /// Sets a control's custom validity message.
    SetCustomValidity {
        node: Handle,
        message: String,
    },
    /// A form's controls that fail their constraints, in tree order.
    InvalidControls(Handle),
    /// A form's submission tried: each control's user validity set.
    MarkSubmitted(Handle),
    /// A control's user validity set: the user changed it.
    SetUserValidity(Handle),
    /// A control's validation message given as the browser's notice.
    ReportInvalid(Handle),
    /// Whether the document is hidden (HTML section 6.2): its window is
    /// minimized or covered.
    Hidden,
    /// `alert`, `confirm` or `prompt` (`kind`) was called with `message`:
    /// the host shows it; the dialog blocks nothing and is answered as a
    /// browser that suppresses dialogs answers it.
    Dialog {
        kind: String,
        message: String,
    },
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
    /// Nodes, each with a flag.
    Pins(Vec<(Handle, bool)>),
    Numbers(Vec<f64>),
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
