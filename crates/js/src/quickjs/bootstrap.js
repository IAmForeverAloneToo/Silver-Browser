// The DOM's bindings (M25, docs/design/dom-scripting.md): the classes a
// script's nodes are objects of, run once in each new realm before any
// page's script. A function of the natives, which only this closure
// holds, and of the global object.
(function (native, global) {
  "use strict";

  // What the bindings use of the built-ins, kept before any page runs,
  // so that a page that replaces one changes nothing they do.
  const uncurry = (f) => Function.prototype.call.bind(f);
  const $Object = Object;
  const defineProperty = Object.defineProperty;
  const getOwnPropertyNames = Object.getOwnPropertyNames;
  const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
  const setPrototypeOf = Object.setPrototypeOf;
  const $String = String;
  const $TypeError = TypeError;
  const $RangeError = RangeError;
  const $DOMException = DOMException;
  const $Map = Map;
  const $WeakRef = WeakRef;
  const $FinalizationRegistry = FinalizationRegistry;
  const MapGet = uncurry(Map.prototype.get);
  const MapSet = uncurry(Map.prototype.set);
  const MapDelete = uncurry(Map.prototype.delete);
  const WeakRefDeref = uncurry(WeakRef.prototype.deref);
  const Register = uncurry(FinalizationRegistry.prototype.register);
  const ToWellFormed = uncurry(String.prototype.toWellFormed);
  const ToUpperCase = uncurry(String.prototype.toUpperCase);
  const StringIncludes = uncurry(String.prototype.includes);
  const toStringTag = Symbol.toStringTag;

  // A key only the bindings have: a class's constructor run without it
  // is a script's `new`, which the DOM's interfaces refuse.
  const KEY = Symbol("key");

  // A DOMString: the value as a string, with any lone surrogate made
  // U+FFFD, which the host's strings cannot hold.
  function str(value) {
    return ToWellFormed($String(value));
  }

  // The exception a native's failure is, in the reference browser's
  // words: "Failed to execute 'm' on 'I': reason".
  function failed(error, method, iface) {
    if (error instanceof $DOMException) {
      return new $DOMException(
        "Failed to execute '" + method + "' on '" + iface + "': " + error.message,
        error.name,
      );
    }
    if (error instanceof $TypeError || error instanceof $RangeError) {
      error.message = "Failed to execute '" + method + "' on '" + iface + "': " + error.message;
    }
    return error;
  }

  function illegalConstructor(iface) {
    return new $TypeError("Failed to construct '" + iface + "': Illegal constructor");
  }

  function illegalInvocation() {
    return new $TypeError("Illegal invocation");
  }

  // Makes an interface look as Web IDL says: its members enumerable,
  // its prototype tagged with its name, its constructor a global
  // property that is writable and configurable but not enumerable.
  function expose(cls, name) {
    const proto = cls.prototype;
    for (const key of getOwnPropertyNames(proto)) {
      if (key === "constructor") {
        continue;
      }
      const desc = getOwnPropertyDescriptor(proto, key);
      desc.enumerable = true;
      defineProperty(proto, key, desc);
    }
    defineProperty(proto, toStringTag, { value: name, configurable: true });
    defineProperty(global, name, {
      value: cls,
      writable: true,
      configurable: true,
      enumerable: false,
    });
    return cls;
  }

  // ---- Nodes and their objects --------------------------------------

  // One object per node while it lives: handle to a weak reference to
  // it. When one is collected and no newer one has been made since,
  // the host hears the handle is released, and may free the node once
  // it is out of the tree.
  const objects = new $Map();
  const registry = new $FinalizationRegistry((handle) => {
    const ref = MapGet(objects, handle);
    if (ref !== undefined && WeakRefDeref(ref) !== undefined) {
      return;
    }
    MapDelete(objects, handle);
    native.released(handle);
  });

  // The object for the node `handle` names, made if it has none; null
  // for null.
  function wrap(handle) {
    if (handle === null) {
      return null;
    }
    const ref = MapGet(objects, handle);
    if (ref !== undefined) {
      const object = WeakRefDeref(ref);
      if (object !== undefined) {
        return object;
      }
    }
    const object = make(handle);
    MapSet(objects, handle, new $WeakRef(object));
    Register(registry, object, handle);
    return object;
  }

  // A new object for a node, of the interface its kind and tag say.
  function make(handle) {
    switch (native.kind(handle)) {
      case 1:
        return new (elementInterface(native.tag(handle)))(KEY, handle);
      case 3:
        return new Text(KEY, handle);
      default:
        return new HTMLDocument(KEY, handle);
    }
  }

  class EventTarget {}

  // The handle of `node`, which must be a Node; and whether a value is
  // one. Set from inside Node's body, where its private field is seen.
  let handleOf;
  let isNode;

  class Node extends EventTarget {
    #handle;

    constructor(key, handle) {
      if (key !== KEY) {
        throw illegalConstructor("Node");
      }
      super();
      this.#handle = handle;
    }

    static {
      handleOf = (node) => node.#handle;
      isNode = (value) => typeof value === "object" && value !== null && #handle in value;
    }

    get nodeType() {
      return native.kind(self(this));
    }

    get nodeName() {
      const handle = self(this);
      switch (native.kind(handle)) {
        case 1:
          return ToUpperCase(native.tag(handle));
        case 3:
          return "#text";
        default:
          return "#document";
      }
    }

    get ownerDocument() {
      const handle = self(this);
      return native.kind(handle) === 9 ? null : theDocument();
    }

    get parentNode() {
      return wrap(native.parent(self(this)));
    }

    get firstChild() {
      return wrap(native.firstChild(self(this)));
    }

    get lastChild() {
      return wrap(native.lastChild(self(this)));
    }

    get previousSibling() {
      return wrap(native.previousSibling(self(this)));
    }

    get nextSibling() {
      return wrap(native.nextSibling(self(this)));
    }

    get textContent() {
      return native.textContent(self(this));
    }

    appendChild(node) {
      const parent = self(this);
      const child = nodeArgument(node, "appendChild", "Node");
      try {
        native.insert(parent, child, null);
      } catch (error) {
        throw failed(error, "appendChild", "Node");
      }
      return node;
    }

    removeChild(child) {
      const parent = self(this);
      const handle = nodeArgument(child, "removeChild", "Node");
      try {
        native.remove(parent, handle);
      } catch (error) {
        throw failed(error, "removeChild", "Node");
      }
      return child;
    }
  }

  for (const [name, value] of [
    ["ELEMENT_NODE", 1],
    ["ATTRIBUTE_NODE", 2],
    ["TEXT_NODE", 3],
    ["CDATA_SECTION_NODE", 4],
    ["ENTITY_REFERENCE_NODE", 5],
    ["ENTITY_NODE", 6],
    ["PROCESSING_INSTRUCTION_NODE", 7],
    ["COMMENT_NODE", 8],
    ["DOCUMENT_NODE", 9],
    ["DOCUMENT_TYPE_NODE", 10],
    ["DOCUMENT_FRAGMENT_NODE", 11],
    ["NOTATION_NODE", 12],
  ]) {
    defineProperty(Node, name, { value, enumerable: true });
    defineProperty(Node.prototype, name, { value, enumerable: true });
  }

  // `this` as a node's handle, or Illegal invocation.
  function self(node) {
    if (!isNode(node)) {
      throw illegalInvocation();
    }
    return handleOf(node);
  }

  // An argument that must be a Node, as its handle.
  function nodeArgument(value, method, iface) {
    if (!isNode(value)) {
      throw new $TypeError(
        "Failed to execute '" + method + "' on '" + iface + "': parameter 1 is not of type 'Node'.",
      );
    }
    return handleOf(value);
  }

  class CharacterData extends Node {
    constructor(key, handle) {
      if (key !== KEY) {
        throw illegalConstructor("CharacterData");
      }
      super(key, handle);
    }

    get data() {
      return native.data(self(this));
    }

    get length() {
      return native.data(self(this)).length;
    }
  }

  class Text extends CharacterData {
    // `new Text(data)` is a script's to make, as the DOM lets it.
    constructor(key, handle) {
      if (key !== KEY) {
        return wrap(native.createText(key === undefined ? "" : str(key)));
      }
      super(key, handle);
    }
  }

  class Element extends Node {
    constructor(key, handle) {
      if (key !== KEY) {
        throw illegalConstructor("Element");
      }
      super(key, handle);
    }

    get tagName() {
      return ToUpperCase(native.tag(self(this)));
    }

    get localName() {
      return native.tag(self(this));
    }
  }

  class HTMLElement extends Element {
    constructor(key, handle) {
      if (key !== KEY) {
        throw illegalConstructor("HTMLElement");
      }
      super(key, handle);
    }
  }

  class Document extends Node {
    constructor(key, handle) {
      if (key !== KEY) {
        throw illegalConstructor("Document");
      }
      super(key, handle);
    }

    get documentElement() {
      self(this);
      let child = native.firstChild(native.document());
      while (child !== null && native.kind(child) !== 1) {
        child = native.nextSibling(child);
      }
      return wrap(child);
    }

    get textContent() {
      self(this);
      return null;
    }

    get title() {
      self(this);
      return native.title();
    }

    set title(value) {
      self(this);
      native.setTitle(str(value));
    }

    createElement(localName) {
      self(this);
      try {
        return wrap(native.createElement(str(localName)));
      } catch (error) {
        throw failed(error, "createElement", "Document");
      }
    }

    createTextNode(data) {
      self(this);
      return wrap(native.createText(str(data)));
    }
  }

  class HTMLDocument extends Document {
    constructor(key, handle) {
      if (key !== KEY) {
        throw illegalConstructor("HTMLDocument");
      }
      super(key, handle);
    }
  }

  expose(EventTarget, "EventTarget");
  expose(Node, "Node");
  expose(CharacterData, "CharacterData");
  expose(Text, "Text");
  expose(Element, "Element");
  expose(HTMLElement, "HTMLElement");
  expose(Document, "Document");
  expose(HTMLDocument, "HTMLDocument");

  // HTML's element interfaces (HTML section 3.2.2 and each element's
  // definition): the interface each tag's elements are objects of.
  const interfaces = new $Map();
  function element(name, tags, base) {
    const parent = base || HTMLElement;
    const cls = {
      [name]: class extends parent {
        constructor(key, handle) {
          if (key !== KEY) {
            throw illegalConstructor(name);
          }
          super(key, handle);
        }
      },
    }[name];
    expose(cls, name);
    for (const tag of tags) {
      MapSet(interfaces, tag, cls);
    }
    return cls;
  }
  const HTMLMediaElement = element("HTMLMediaElement", []);
  for (const [name, tags, base] of [
    ["HTMLAnchorElement", ["a"]],
    ["HTMLAreaElement", ["area"]],
    ["HTMLAudioElement", ["audio"], HTMLMediaElement],
    ["HTMLBaseElement", ["base"]],
    ["HTMLBodyElement", ["body"]],
    ["HTMLBRElement", ["br"]],
    ["HTMLButtonElement", ["button"]],
    ["HTMLCanvasElement", ["canvas"]],
    ["HTMLDataElement", ["data"]],
    ["HTMLDataListElement", ["datalist"]],
    ["HTMLDetailsElement", ["details"]],
    ["HTMLDialogElement", ["dialog"]],
    ["HTMLDirectoryElement", ["dir"]],
    ["HTMLDivElement", ["div"]],
    ["HTMLDListElement", ["dl"]],
    ["HTMLEmbedElement", ["embed"]],
    ["HTMLFieldSetElement", ["fieldset"]],
    ["HTMLFontElement", ["font"]],
    ["HTMLFormElement", ["form"]],
    ["HTMLFrameElement", ["frame"]],
    ["HTMLFrameSetElement", ["frameset"]],
    ["HTMLHeadElement", ["head"]],
    ["HTMLHeadingElement", ["h1", "h2", "h3", "h4", "h5", "h6"]],
    ["HTMLHRElement", ["hr"]],
    ["HTMLHtmlElement", ["html"]],
    ["HTMLIFrameElement", ["iframe"]],
    ["HTMLImageElement", ["img"]],
    ["HTMLInputElement", ["input"]],
    ["HTMLLabelElement", ["label"]],
    ["HTMLLegendElement", ["legend"]],
    ["HTMLLIElement", ["li"]],
    ["HTMLLinkElement", ["link"]],
    ["HTMLMapElement", ["map"]],
    ["HTMLMarqueeElement", ["marquee"]],
    ["HTMLMenuElement", ["menu"]],
    ["HTMLMetaElement", ["meta"]],
    ["HTMLMeterElement", ["meter"]],
    ["HTMLModElement", ["del", "ins"]],
    ["HTMLObjectElement", ["object"]],
    ["HTMLOListElement", ["ol"]],
    ["HTMLOptGroupElement", ["optgroup"]],
    ["HTMLOptionElement", ["option"]],
    ["HTMLOutputElement", ["output"]],
    ["HTMLParagraphElement", ["p"]],
    ["HTMLParamElement", ["param"]],
    ["HTMLPictureElement", ["picture"]],
    ["HTMLPreElement", ["pre", "listing", "xmp"]],
    ["HTMLProgressElement", ["progress"]],
    ["HTMLQuoteElement", ["blockquote", "q"]],
    ["HTMLScriptElement", ["script"]],
    ["HTMLSelectElement", ["select"]],
    ["HTMLSlotElement", ["slot"]],
    ["HTMLSourceElement", ["source"]],
    ["HTMLSpanElement", ["span"]],
    ["HTMLStyleElement", ["style"]],
    ["HTMLTableCaptionElement", ["caption"]],
    ["HTMLTableCellElement", ["td", "th"]],
    ["HTMLTableColElement", ["col", "colgroup"]],
    ["HTMLTableElement", ["table"]],
    ["HTMLTableRowElement", ["tr"]],
    ["HTMLTableSectionElement", ["tbody", "thead", "tfoot"]],
    ["HTMLTemplateElement", ["template"]],
    ["HTMLTextAreaElement", ["textarea"]],
    ["HTMLTimeElement", ["time"]],
    ["HTMLTitleElement", ["title"]],
    ["HTMLTrackElement", ["track"]],
    ["HTMLUListElement", ["ul"]],
    ["HTMLVideoElement", ["video"], HTMLMediaElement],
  ]) {
    element(name, tags, base);
  }
  const HTMLUnknownElement = element("HTMLUnknownElement", []);
  // The tags HTML defines whose elements are plain HTMLElements.
  for (const tag of [
    "abbr", "acronym", "address", "article", "aside", "b", "basefont", "bdi", "bdo", "big",
    "center", "cite", "code", "dd", "dfn", "dt", "em", "figcaption", "figure", "footer",
    "header", "hgroup", "i", "kbd", "main", "mark", "nav", "nobr", "noembed", "noframes",
    "noscript", "plaintext", "rb", "rp", "rt", "rtc", "ruby", "s", "samp", "search",
    "section", "small", "strike", "strong", "sub", "summary", "sup", "tt", "u", "var", "wbr",
  ]) {
    MapSet(interfaces, tag, HTMLElement);
  }

  // The interface an element with local name `tag` is an object of:
  // HTML's for its tag, HTMLElement for a name with a hyphen (a custom
  // element's), HTMLUnknownElement for the rest.
  function elementInterface(tag) {
    const known = MapGet(interfaces, tag);
    if (known !== undefined) {
      return known;
    }
    return StringIncludes(tag, "-") ? HTMLElement : HTMLUnknownElement;
  }

  // ---- The window ----------------------------------------------------

  class Window extends EventTarget {
    constructor() {
      throw illegalConstructor("Window");
    }
  }
  expose(Window, "Window");
  setPrototypeOf(global, Window.prototype);

  // The document's object, made the first time a script asks: the
  // realm is made before any host is there to ask.
  let document = null;
  function theDocument() {
    if (document === null) {
      document = wrap(native.document());
    }
    return document;
  }
  defineProperty(global, "document", {
    get: theDocument,
    enumerable: true,
    configurable: false,
  });
});
