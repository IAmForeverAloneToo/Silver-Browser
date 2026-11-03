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
    /// Its base URL and fetch options, as [`crate::Script`]'s.
    pub base: String,
    pub options: u32,
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
    /// The request's cache mode (M28.4): `default`, `no-store`, ...
    pub cache: String,
    /// Its referrer (`about:client`, `no-referrer`, or a URL of the
    /// document's origin) and its referrer policy, empty for the
    /// document's (M33.3).
    pub referrer: String,
    pub referrer_policy: String,
    /// The address space its server is to be in (M33.4,
    /// `targetAddressSpace`): `local`, `loopback`, `public`, or empty.
    pub target_address_space: String,
    /// Its integrity metadata (D62), empty for none.
    pub integrity: String,
    /// Its destination: empty for a script's own fetch, `font` for a
    /// `FontFace`'s source (CSS Font Loading 3), which `font-src` rules.
    pub destination: String,
}

/// A call of a canvas's 2D context, or of an object scripts make for
/// one (M30, `docs/design/canvas.md`): the operation by its name, and
/// its arguments as the bindings converted them, numbers and strings
/// each in their order. An operation on an object names it first, by
/// two numbers: its kind (0 an element's context, by the element's
/// handle; 2 a `Path2D`, 3 a `CanvasGradient`, 4 a `CanvasPattern`, by
/// their numbers) and which.
#[derive(Debug, Clone, PartialEq)]
pub struct CanvasCall {
    pub op: String,
    pub numbers: Vec<f64>,
    pub strings: Vec<String>,
    /// The pixels `putImageData` writes.
    pub bytes: Option<Vec<u8>>,
}

/// A call the bindings make of the host, one per native function.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    /// The document node.
    Document,
    /// The bindings make an object for the node, which the host then
    /// holds until it is released: the node's name (its `nodeName`, an
    /// element's qualified name); `connected` or empty; an element's
    /// namespace's URI, or empty (M32); its `nodeType` (D58); for an
    /// element whose name has a prefix that prefix, for a document the
    /// interface its object is of (`Document`, `HTMLDocument` or
    /// `XMLDocument`, D67), else empty; then the names of its attributes
    /// that start with `on`, which may be event handlers (M26), as
    /// strings.
    Wrap(Handle),
    /// The nodes scripts hold whose place in the document changed since
    /// this was last asked, each with whether it is now in the document:
    /// the bindings hold the object of a node in the document as the
    /// document does, and let go of it when it leaves.
    TakePins,
    /// The node's `nodeType`.
    Kind(Handle),
    /// An element's qualified name: an HTML element's lowercase.
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
    /// A text, comment or processing instruction node's data.
    Data(Handle),
    /// Sets a text, comment or processing instruction node's data.
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
    /// lowercases in an HTML document. Each call that makes a node names
    /// the document it is made for last, `None` for the page's (D67).
    CreateElement(String, Option<Handle>),
    /// A new element of the namespace whose URI is given (none if
    /// `None`) with this qualified name, which the bindings validated
    /// (`createElementNS`, M32).
    CreateElementNs(Option<String>, String, Option<Handle>),
    /// A new text node.
    CreateText(String, Option<Handle>),
    /// A new comment (D58).
    CreateComment(String, Option<Handle>),
    /// A new CDATA section (`createCDATASection`, D67).
    CreateCdata(String, Option<Handle>),
    /// A new processing instruction: its target, which the bindings
    /// checked, and data.
    CreateProcessingInstruction(String, String, Option<Handle>),
    /// A new, empty document fragment.
    CreateFragment(Option<Handle>),
    /// `markup` parsed as a fragment in the context of the node's element
    /// (DOM Parsing's `createContextualFragment`, a range's start): a
    /// new document fragment, whose scripts run once inserted.
    ContextualFragment(Handle, String),
    /// A new doctype: its name, which the bindings checked, and public
    /// and system identifiers.
    CreateDoctype(String, String, String, Option<Handle>),
    /// A doctype's name, public identifier and system identifier.
    DoctypeIds(Handle),
    /// A template's contents, a fragment.
    TemplateContent(Handle),
    /// A document's doctype, or `None`: the page's for `None`.
    Doctype(Option<Handle>),
    /// A node's owner document (DOM's node document, D67), an inert
    /// template document made for a template's contents if need be.
    OwnerDocument(Handle),
    /// A new document beside the page's (D67): `html` for an HTML one
    /// (`createHTMLDocument`), `xml` for an XML one of the content type
    /// given (`createDocument`), `plain` for the `Document`
    /// constructor's.
    CreateDocument {
        kind: String,
        content_type: String,
    },
    /// `markup` parsed into a new document of the type its MIME type
    /// says (`DOMParser.parseFromString`, `Document.parseHTMLUnsafe`):
    /// `text/html` by the HTML parser, scripting off, another by the XML
    /// parser, an error reported in the document as the reference
    /// browser reports it.
    ParseDocument {
        markup: String,
        mime: String,
    },
    /// A response's body parsed into a new document (XHR's "document
    /// response"): HTML's (`html`) decoded by its `charset`, if one, or
    /// sniffed, XML's by its own; its content type and URL as given;
    /// `None` for XML that is not well formed.
    ParseResponse {
        body: Vec<u8>,
        html: bool,
        content_type: String,
        charset: Option<String>,
        url: String,
    },
    /// `XMLSerializer.serializeToString`: the node's XML serialization,
    /// not required to be well formed (DOM Parsing section 3.2).
    SerializeXml(Handle),
    /// A document serialized as a request body: an HTML document's HTML,
    /// another's XML (XHR's `send`).
    SerializeDocument(Handle),
    /// `adoptNode`: the node taken out of its parent and made the
    /// document's.
    Adopt {
        node: Handle,
        document: Handle,
    },
    /// Whether an element's node document is an HTML one, which decides
    /// the case of its HTML elements' names and attributes.
    InHtmlDocument(Handle),
    /// The value of an element's attribute of a namespace (`None` for
    /// none) and local name (`getAttributeNS`).
    AttributeNs(Handle, Option<String>, String),
    /// `setAttributeNS`: a namespace, a qualified name the bindings
    /// validated against it, and a value.
    SetAttributeNs(Handle, Option<String>, String, String),
    /// `removeAttributeNS`: a namespace and a local name.
    RemoveAttributeNs(Handle, Option<String>, String),
    /// The namespaces of an element's attributes, in order, the empty
    /// string for none.
    AttributeNamespaces(Handle),
    /// `compareDocumentPosition`'s bits of the second node against the
    /// first (D59).
    ComparePosition(Handle, Handle),
    /// Whether two nodes are equal, as `isEqualNode` says.
    IsEqualNode(Handle, Handle),
    /// `normalize()`.
    Normalize(Handle),
    /// The DOM's "replace data": UTF-16 offset, count, and new data.
    ReplaceData(Handle, u32, u32, String),
    /// `splitText` at a UTF-16 offset: the new text node.
    SplitText(Handle, u32),
    /// A node's length, as ranges count it.
    NodeLength(Handle),
    /// A node's index among its parent's children.
    IndexOf(Handle),
    /// A new live range at the start of the node: its id.
    CreateRange(Handle),
    /// Lets go of a live range.
    DropRange(u32),
    /// A live range's start node and offset and end node and offset.
    RangeOf(u32),
    /// Sets a live range's boundary points, which the bindings checked.
    SetRange(u32, Handle, u32, Handle, u32),
    /// Where the first boundary point is against the second: -1, 0, 1,
    /// or 2 for points in different trees.
    CompareBoundaries(Handle, u32, Handle, u32),
    /// A new node iterator over the node's subtree: its id.
    CreateIterator(Handle),
    /// Lets go of a node iterator.
    DropIterator(u32),
    /// A node iterator's reference node and whether its pointer is
    /// before it (1) or not (0).
    IteratorOf(u32),
    /// Moves a node iterator.
    SetIterator(u32, Handle, bool),
    /// `moveBefore`: the node moved under the parent before the child,
    /// or last, keeping its state.
    MoveBefore {
        parent: Handle,
        node: Handle,
        before: Option<Handle>,
    },
    /// An element's place in the layout (CSSOM View, D65): its border
    /// box in viewport coordinates (`scrolled`) or the document's, then
    /// its borders, padding and margins, its scroll height and each of
    /// its boxes; empty with no box.
    Geometry {
        node: Handle,
        scrolled: bool,
    },
    /// An element's `offsetParent`.
    OffsetParent(Handle),
    /// The viewport's scroll offset (`None`) or an element's, and its
    /// most, down then across.
    ScrollOffset(Option<Handle>),
    /// Scrolls the viewport (`None`) or an element across to an offset
    /// and down to another, either kept when NaN; smoothly or not.
    ScrollTo(Option<Handle>, f64, f64, bool),
    /// Scrolls an element into view, aligned in the block and the
    /// inline axes as `start`, `center`, `end` or `nearest`, with a
    /// `ScrollBehavior` (`auto`, `instant`, `smooth`).
    ScrollIntoView(Handle, String, String, String),
    /// The elements under a viewport point, topmost first, or the
    /// topmost alone.
    ElementsFromPoint {
        x: f64,
        y: f64,
        all: bool,
    },
    /// An element's `innerText`.
    InnerText(Handle),
    /// The rendered text between two boundary points, as `innerText`
    /// collects it, or `null` where nothing there is rendered: what
    /// `Selection.toString()` gives.
    SelectedText(Handle, u32, Handle, u32),
    /// The boxes of the range between two boundary points in viewport
    /// coordinates (CSSOM View section 6.2), four numbers each.
    RangeRects(Handle, u32, Handle, u32),
    /// The CSSOM's sheets and rules (D66): `op` on the sheet of `owner`
    /// (none for the document's list), `path` its imports' count, the
    /// imports, then the rule path; `text` and `index` what the op
    /// takes.
    Cssom {
        op: String,
        owner: Option<Handle>,
        path: Vec<f64>,
        text: String,
        index: f64,
    },
    /// `document.open()` (HTML section 8.4.2.1).
    DocumentOpen,
    /// `document.close()`.
    DocumentClose,
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
    /// The elements under the node of the namespace (`None` for none,
    /// `*` for any) and the local name (`*` for any), in tree order
    /// (`getElementsByTagNameNS`, M32).
    ElementsByTagNameNs(Handle, Option<String>, String),
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
    /// The descriptors of an `@page` rule's block (CSS Paged Media 3
    /// section 7): its margins, `size` and `page-orientation`, as
    /// `ParseDeclarations` gives them.
    ParsePageDescriptors(String),
    /// The name the cascade knows a property by: an alias's property, a
    /// name in any case lowercased, a custom property's as it is; `None`
    /// for one it does not know.
    CssCanonical(String),
    /// Whether a value is valid for a property.
    CssValid(String, String),
    /// A shorthand's value from a declaration block's declarations, as
    /// `ParseDeclarations` gives them: the empty string where it cannot
    /// be said.
    CssShorthand(String, Vec<String>),
    /// The longhands a declaration block keeps a shorthand as; none for
    /// a property it keeps as it is.
    CssLonghands(String),
    /// A declaration block's text from its declarations, as
    /// `ParseDeclarations` gives them, shorthands said where they can be.
    CssBlockText(Vec<String>),
    /// A URL parsed against the document's base URL, or `None` when it
    /// does not parse (a URL-reflecting attribute's getter).
    ResolveUrl(String),
    /// A URL parsed against the document's base URL by the UTF-8 URL
    /// parser, whatever the document's encoding: a `Request`'s and
    /// `Response.redirect()`'s (Fetch section 5.4).
    ResolveUrlUtf8(String),
    /// A URL parsed against the document's own URL, its fallback base
    /// URL, which a `<base>`'s `href` reads (M27).
    ResolveUrlFromDocument(String),
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
    /// A synchronous XHR's request (M27.6): fetched at once, the run
    /// waiting, its answer the response a script may see, or `null` for
    /// a network error.
    FetchNow(Box<FetchCall>),
    /// Bytes decoded with the encoding a label names (an XHR's charset),
    /// or `None` for a label no encoding has.
    Decode(Vec<u8>, String),
    /// Fetch `id` aborted by its signal: the engine stops it.
    AbortFetch(u32),
    /// The name, lower case, of the encoding a label names for
    /// `TextDecoder` (gap batch A), or `null`.
    TextDecoderName(String),
    /// `TextDecoder.decode()`: bytes in the encoding named, as stream
    /// `id`'s next piece unless not `stream`; `null` for bytes a `fatal`
    /// decoder fails on.
    TextDecode {
        id: u32,
        encoding: String,
        bytes: Vec<u8>,
        fatal: bool,
        stream: bool,
    },
    /// The encoding a MIME type's `charset` names, as `TextDecoderName`
    /// answers (`FileReader.readAsText()`).
    MimeCharset(String),
    /// `MutationObserver.observe()` (gap batch A, D42): observer
    /// `observer`'s registration on `node`, its options as flags
    /// (childList 1, attributes 2, characterData 4, subtree 8,
    /// attributeOldValue 16, characterDataOldValue 32) and its attribute
    /// filter, if any.
    MutationObserve {
        observer: u32,
        node: Handle,
        flags: u32,
        filter: Option<Vec<String>>,
    },
    /// `disconnect()`.
    MutationDisconnect(u32),
    /// The bindings begin (`true`) or end an operation of several calls
    /// (`replaceChildren()`, an insertion of several nodes), whose changes
    /// are grouped as one call's are.
    MutationScope(bool),
    /// `takeRecords()`: the observer's records, as JSON.
    MutationTake(u32),
    /// The mutation observer microtask's start: the observers to
    /// notify, in the order they were made.
    MutationNotify,
    /// The window's named property `name` (gap batch A, D44): the
    /// document's elements of that id, or of that name among `embed`,
    /// `form`, `img` and `object`, in tree order.
    NamedItems(String),
    /// The document's base URL (`Node.baseURI`), or `null` for none.
    BaseUrl,
    /// A number for a new `BroadcastChannel` (gap batch A, D46), no
    /// other channel's in the process.
    BroadcastId,
    /// `BroadcastChannel.postMessage()`: to the channels of `name` but
    /// `sender`, the message as serialized.
    BroadcastPost {
        name: String,
        data: String,
        sender: u64,
    },
    /// `ResizeObserver.observe()` (gap batch A, D45): observer's watch of
    /// `node`'s box, 0 its content box, 1 its border box, 2 its content
    /// box in device pixels.
    ResizeObserve {
        observer: u32,
        node: Handle,
        observed: u32,
    },
    /// `unobserve()`, or with no node `disconnect()`.
    ResizeUnobserve {
        observer: u32,
        node: Option<Handle>,
    },
    /// `IntersectionObserver.observe()`: observer's watch of `node`, with
    /// its root, its margin (each side a value and whether it is a
    /// percentage) and its thresholds.
    IntersectionObserve {
        observer: u32,
        node: Handle,
        root: Option<Handle>,
        margin: Vec<f64>,
        thresholds: Vec<f64>,
    },
    /// `unobserve()`, or with no node `disconnect()`.
    IntersectionUnobserve {
        observer: u32,
        node: Option<Handle>,
    },
    /// `matchMedia()` (gap batch A): the list's serialization and its
    /// number, as strings.
    MatchMedia(String),
    /// Whether list `id` holds now.
    MediaMatches(u32),
    /// List `id`'s object is gone.
    MediaForget(u32),
    /// When an idle period that starts now ends, on the document's clock
    /// (`requestIdleCallback()`).
    IdleDeadline,
    /// `CSS.supports()`: a property and a value, or a condition alone.
    CssSupports(String, Option<String>),
    /// `URL.createObjectURL()` (gap batch A): a `blob:` URL for a blob
    /// of these bytes and type.
    BlobUrlCreate(Vec<u8>, String),
    /// `URL.revokeObjectURL()`.
    BlobUrlRevoke(String),
    /// The URL parser for the URL API (gap batch A): the input against
    /// a base, if any, answered with the URL's getters' values or `null`.
    UrlParse(String, Option<String>),
    /// A URL API setter: the URL `href` after the setter `part` is given
    /// `value`, answered as `UrlParse` is.
    UrlSet {
        href: String,
        part: String,
        value: String,
    },
    /// Whether a URL includes credentials, which a Request refuses.
    UrlHasCredentials(String),
    /// Whether a URL is of the document's origin (a Request's referrer).
    SameOriginUrl(String),
    /// `window.postMessage`'s target origin (M31): `*`, or the origin
    /// `/` names (the document's) or a URL's, serialized; a
    /// `SyntaxError` for a URL that does not parse.
    TargetOrigin(String),
    /// A message to the window itself (M31), its data serialized, for
    /// the target origin `TargetOrigin` gave: delivered in a task, as a
    /// `Task::Message`, unless the origin is not the document's.
    PostMessage {
        target_origin: String,
        data: String,
    },
    /// `new Worker(url, options)` (M31): the worker started, its number
    /// answered; a `SecurityError` for a URL of another origin, a
    /// `SyntaxError` for one that does not parse.
    WorkerStart {
        url: String,
        name: String,
        module: bool,
        credentials: String,
    },
    /// A message for worker `worker`'s global, its data serialized.
    WorkerPost {
        worker: u32,
        data: String,
    },
    /// `worker.terminate()`.
    WorkerTerminate(u32),
    /// A worker's `postMessage`, to its `Worker` object (M31).
    PostToOwner(String),
    /// A worker's `close()`.
    CloseWorker,
    /// `new MessageChannel()` (M31.5): its two ports' numbers.
    PortChannel,
    /// A message from port `port` to its entangled port, its data
    /// serialized; a `DataCloneError` past the bounds.
    PortPost {
        port: u32,
        data: String,
    },
    /// A port started: the messages kept for it come.
    PortStart(u32),
    /// `port.close()`.
    PortClose(u32),
    /// A port in a message on its way: the realm holds it no more.
    PortAway(u32),
    /// A port taken from a message: whether the realm may hold it.
    PortAdopt(u32),
    /// A script `importScripts` runs (M31): its text and the URL it came
    /// from, or nothing when it failed to load.
    ImportScript(String),
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
    /// This window's frame (M33.2): its number in the tab, 0 at the top
    /// and for an engine that has no frames.
    FrameSelf,
    /// Frame `n`'s parent's number, or -1 at the top.
    FrameParent(u32),
    /// Frame `n`'s top's number.
    FrameTop(u32),
    /// How many child frames frame `n` has.
    FrameLength(u32),
    /// Frame `n`'s `index`th child frame, in its document's tree order,
    /// or -1.
    FrameChild(u32, u32),
    /// Frame `n`'s child frame whose element's `name` is `name`, or -1.
    FrameNamed(u32, String),
    /// Whether frame `n` is gone.
    FrameClosed(u32),
    /// The frame of this document's `iframe` `element`, or -1.
    FrameOf(Handle),
    /// A message for frame `frame`'s window, its data serialized, for the
    /// target origin `TargetOrigin` gave.
    FramePost {
        frame: u32,
        target_origin: String,
        data: String,
    },
    /// Frame `frame` navigated to `url`, parsed against this document's
    /// base URL, in place of its entry when `replace`: a `SecurityError`
    /// when this window may not navigate it.
    FrameNavigate {
        frame: u32,
        url: String,
        replace: bool,
    },
    /// `window.name`, and it set.
    WindowName,
    SetWindowName(String),
    /// `document.domain`: the origin's host, or the empty string for an
    /// opaque origin.
    DocumentDomain,
    /// `document.domain` set: a `SecurityError` unless the value is the
    /// host or a registrable suffix of it, and then nothing, as in an
    /// origin-keyed agent cluster (HTML 7.1.1.2).
    SetDocumentDomain(String),
    /// Whether the window's `localStorage` (`true`) or `sessionStorage`
    /// may be read (HTML 12.2.2 and 12.2.3, M28): the empty string, or
    /// why not, a `SecurityError`'s message. The calls below name the
    /// area the same way.
    StorageOpen(bool),
    /// An area's `length`.
    StorageLength(bool),
    /// An area's `key(index)`: the key, or `null`.
    StorageKey(bool, u32),
    /// An area's keys, in their order.
    StorageKeys(bool),
    /// An area's `getItem(key)`: the value, or `null`.
    StorageGet(bool, String),
    /// An area's `setItem(key, value)`: whether the area could hold it,
    /// within its quota.
    StorageSet(bool, String, String),
    /// An area's `removeItem(key)`.
    StorageRemove(bool, String),
    /// An area's `clear()`.
    StorageClear(bool),
    /// `devtools.send(message)` of the DevTools panel's page (M36): the
    /// message queued for the browser. Refused in any other document.
    DevToolsSend(String),
    /// The answer of a DevTools console's evaluation (M36,
    /// [`crate::Task::Evaluate`]): refused but while one runs.
    DevToolsAnswer(String),
    /// IndexedDB's request (M36.7), as JSON: its answer, as JSON.
    Idb(String),
    /// The Cache API's request (M39), as JSON, and the bodies it carries:
    /// its answer, as JSON, and the bodies it gives
    /// ([`Answer::Pair`]).
    Caches(String, Vec<u8>),
    /// A request of the service workers' host (M39), as JSON: its
    /// answer, as JSON.
    ServiceWorker(String),
    /// WebGPU's operation (M41), as JSON, and the bytes it carries.
    Gpu(String, Vec<u8>),
    /// WebGL's call (M41): the operation, its numbers and strings, and the
    /// bytes it carries, as a canvas's are.
    WebGl(Box<CanvasCall>),
    /// Web Audio's operation (M40), as JSON, and the samples it carries:
    /// its answer, as JSON, and the samples it gives ([`Answer::Pair`]).
    Audio(String, Vec<u8>),
    /// A media element's call (M40), as JSON: its answer, as JSON.
    Media(String),
    /// A call of the Web Animations API's (D75), as JSON: its answer, as
    /// JSON, with what it owes the promises the script holds.
    Animation(String),
    /// A `FetchEvent`'s answer (M39): its head as JSON and its body.
    ServiceWorkerRespond(String, Vec<u8>),
    /// The bytes the document's storage key uses (M36.8).
    StorageUsage,
    /// A request of IndexedDB's completed: its task is to be queued.
    IdbWake,
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
    /// `window.open(url, target)` (M37): a URL parsed against the
    /// document's base URL, `about:blank` for none, opened in a new tab
    /// or in the navigable `target` names; a `SyntaxError` for one that
    /// does not parse.
    OpenWindow {
        url: String,
        target: String,
    },
    /// `window.close()` (M37): the tab closed, if a script opened it.
    CloseWindow,
    /// What `navigator` reports (D12): its user agent string, its
    /// platform, and its compatibility mode, `gecko` or `chrome`.
    NavigatorInfo,
    /// The viewport's width and height in CSS pixels, and the device
    /// pixel ratio.
    Viewport,
    /// A document's URL, character encoding, content type,
    /// compatibility mode and referrer: the page's for `None`, another's
    /// as it was made (D67).
    DocumentInfo(Option<Handle>),
    /// A document's title (HTML section 3.1.7): the page's for `None`.
    DocumentTitle(Option<Handle>),
    /// Sets another document's title.
    SetDocumentTitle(Handle, String),
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
    /// What a script asks of an SVG element's geometry (M32): `query`
    /// names the question (`bbox`, `ctm`, `textLength`, `totalLength`,
    /// `pointAtLength`, `inFill`, `inStroke`), `args` its numbers; the
    /// answer's numbers, none when there is no answer.
    SvgQuery {
        node: Handle,
        query: String,
        args: Vec<f64>,
    },
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
    /// The document's fonts (CSS Font Loading 3, D54): an operation,
    /// its words separated by spaces, and a face's data. `status` says
    /// `loading` or `loaded`; `valid` whether the data is a font;
    /// `add <id> <weight> <weight> <style> <stretch> <stretch> <family>`
    /// adds a face of the data, `delete <id>` takes one away.
    Fonts(String, Vec<u8>),
    /// `alert`, `confirm` or `prompt` (`kind`) was called with `message`
    /// and, for `prompt`, `default` (gap batch A, D43): the host asks the
    /// user, the script waiting. The answer is a string for OK, what
    /// `prompt`'s field held, and null for Cancel or no dialog shown.
    Dialog {
        kind: String,
        message: String,
        default: String,
    },
    /// Whether code from a string may be compiled (M33.1): `kind` is
    /// `eval` (`eval`, the function constructors, a timer's string),
    /// `wasm` (a WebAssembly module) or `handler` (an event handler
    /// attribute of `element`, its text `source`). The answer is empty
    /// when it may, else what refuses it, as the exception says it.
    /// `position` is the line and column of the page's code that asked,
    /// 0 0 for none (a handler the parser set).
    CodeAllowed {
        kind: String,
        source: String,
        element: Option<Handle>,
        position: (u32, u32),
    },
    /// An element's nonce (M33.1, HTML's `[[CryptographicNonce]]`), which
    /// its `nonce` attribute no longer shows once hidden.
    Nonce(Handle),
    SetNonce(Handle, String),
    /// The CSSOM just wrote the element's `style` attribute (M33.1).
    CssomStyled(Handle),
    /// Whether the policies let the element's `style` attribute be its
    /// declarations (M33.1).
    StyleAllowed(Handle),
    /// HTML's "resolve a module specifier" (D61): `specifier`, of the
    /// script or module whose base URL is `base`, by the document's
    /// import map. The answer is the URL, or a `TypeError` throws.
    ResolveModule {
        base: String,
        specifier: String,
    },
    /// `import()` `id` wants the module at `url`, which the realm was
    /// not given (D61): the engine fetches its graph and gives it in a
    /// [`crate::Task::Imported`].
    ImportModule {
        id: u32,
        url: String,
        /// The fetch options of the script or module that called it, by
        /// the host's number ([`crate::Script::options`]).
        options: u32,
    },
    /// A canvas's operation (M30): one variant for them all.
    Canvas(Box<CanvasCall>),
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
    /// A synchronous fetch's answer, `None` for a network error (M27.6).
    Fetched(Option<Box<crate::FetchAnswer>>),
    /// Bytes, which the script receives as a new `ArrayBuffer` (M30: a
    /// canvas's pixels).
    Bytes(Vec<u8>),
    /// A string and bytes, which the script receives as an array of the
    /// string and a new `ArrayBuffer` (M39: the Cache API's answer).
    Pair(String, Vec<u8>),
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
