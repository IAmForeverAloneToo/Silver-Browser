// The DOM's bindings (M25, docs/design/dom-scripting.md): the classes a
// script's nodes are objects of, run once in each new realm before any
// page's script. A function of the natives, which only this closure
// holds, and of the global object.
//
// What runs when a page calls in uses only what was kept below before
// any page ran: no method looked up on a built-in's prototype, no
// iterator (`for...of`, spreading, destructuring an array), so that a
// page that replaces a built-in changes nothing the bindings do.
(function (native, global) {
  "use strict";

  const uncurry = (f) => Function.prototype.call.bind(f);
  const defineProperty = Object.defineProperty;
  const getOwnPropertyNames = Object.getOwnPropertyNames;
  const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
  const setPrototypeOf = Object.setPrototypeOf;
  const $String = String;
  const $Number = Number;
  const $TypeError = TypeError;
  const $RangeError = RangeError;
  const $DOMException = DOMException;
  const $Map = Map;
  const $WeakMap = WeakMap;
  const $WeakRef = WeakRef;
  const $Proxy = Proxy;
  const $FinalizationRegistry = FinalizationRegistry;
  const ReflectApply = Reflect.apply;
  const ReflectGet = Reflect.get;
  const ReflectSet = Reflect.set;
  const ReflectHas = Reflect.has;
  const ReflectOwnKeys = Reflect.ownKeys;
  const ReflectDefineProperty = Reflect.defineProperty;
  const ReflectDeleteProperty = Reflect.deleteProperty;
  const ReflectGetOwnPropertyDescriptor = Reflect.getOwnPropertyDescriptor;
  const ArraySplice = uncurry(Array.prototype.splice);
  const StringIndexOf = uncurry(String.prototype.indexOf);
  const FromCharCode = String.fromCharCode;
  const MapGet = uncurry(Map.prototype.get);
  const MapSet = uncurry(Map.prototype.set);
  const MapDelete = uncurry(Map.prototype.delete);
  const MapHas = uncurry(Map.prototype.has);
  const ArraySlice = uncurry(Array.prototype.slice);
  const $eval = eval;
  const WeakMapGet = uncurry(WeakMap.prototype.get);
  const WeakMapSet = uncurry(WeakMap.prototype.set);
  const WeakRefDeref = uncurry(WeakRef.prototype.deref);
  const Register = uncurry(FinalizationRegistry.prototype.register);
  const ArrayPush = uncurry(Array.prototype.push);
  const ArrayValues = Array.prototype.values;
  const ArrayEntries = Array.prototype.entries;
  const ArrayKeys = Array.prototype.keys;
  const ArrayForEach = Array.prototype.forEach;
  const ToWellFormed = uncurry(String.prototype.toWellFormed);
  const ToUpperCase = uncurry(String.prototype.toUpperCase);
  const ToLowerCase = uncurry(String.prototype.toLowerCase);
  const StringIncludes = uncurry(String.prototype.includes);
  const StringSlice = uncurry(String.prototype.slice);
  const CharCodeAt = uncurry(String.prototype.charCodeAt);
  const iteratorSymbol = Symbol.iterator;
  const toStringTag = Symbol.toStringTag;

  // A key only the bindings have: a class's constructor run without it
  // is a script's `new`, which the DOM's interfaces refuse.
  const KEY = Symbol("key");

  // A DOMString: the value as a string, with any lone surrogate made
  // U+FFFD, which the host's strings cannot hold.
  function str(value) {
    return ToWellFormed($String(value));
  }

  // A DOMString that is "" for null ([LegacyNullToEmptyString]).
  function strOrEmpty(value) {
    return value === null ? "" : str(value);
  }

  // An `unsigned long`: ToUint32.
  function unsigned(value) {
    return value >>> 0;
  }

  function prefix(method, iface) {
    return "Failed to execute '" + method + "' on '" + iface + "': ";
  }

  // The exception a native's failure is, in the reference browser's
  // words: "Failed to execute 'm' on 'I': reason".
  function failed(error, method, iface) {
    if (error instanceof $DOMException) {
      return new $DOMException(prefix(method, iface) + error.message, error.name);
    }
    if (error instanceof $TypeError || error instanceof $RangeError) {
      error.message = prefix(method, iface) + error.message;
    }
    return error;
  }

  // The exception a native's failure is when a property was set:
  // "Failed to set the 'p' property on 'I': reason".
  function failedSet(error, property, iface) {
    const words = "Failed to set the '" + property + "' property on '" + iface + "': ";
    if (error instanceof $DOMException) {
      return new $DOMException(words + error.message, error.name);
    }
    if (error instanceof $TypeError || error instanceof $RangeError) {
      error.message = words + error.message;
    }
    return error;
  }

  function domError(name, message, method, iface) {
    return new $DOMException(prefix(method, iface) + message, name);
  }

  function illegalConstructor(iface) {
    return new $TypeError("Failed to construct '" + iface + "': Illegal constructor");
  }

  function illegalInvocation() {
    return new $TypeError("Illegal invocation");
  }

  // Web IDL's check of the arguments a call must have.
  function need(count, required, method, iface) {
    if (count < required) {
      throw new $TypeError(
        prefix(method, iface) + required + " argument" + (required === 1 ? "" : "s") +
          " required, but only " + count + " present.",
      );
    }
  }

  // Makes an interface look as Web IDL says: its members enumerable,
  // its prototype tagged with its name, its constructor a global
  // property that is writable and configurable but not enumerable.
  function expose(cls, name) {
    const proto = cls.prototype;
    const keys = getOwnPropertyNames(proto);
    for (let i = 0; i < keys.length; i++) {
      if (keys[i] === "constructor") {
        continue;
      }
      const desc = getOwnPropertyDescriptor(proto, keys[i]);
      desc.enumerable = true;
      defineProperty(proto, keys[i], desc);
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

  // Copies a mixin's members onto each prototype, as Web IDL's
  // `includes` does.
  function include(source, prototypes) {
    const keys = getOwnPropertyNames(source);
    for (let i = 0; i < keys.length; i++) {
      for (let j = 0; j < prototypes.length; j++) {
        const desc = getOwnPropertyDescriptor(source, keys[i]);
        desc.enumerable = true;
        defineProperty(prototypes[j], keys[i], desc);
      }
    }
  }

  // ---- Nodes and their objects --------------------------------------

  // One object per node while it lives: handle to a weak reference to
  // it, and to the object itself while the node is in the document, which
  // holds it as the document holds its nodes (so that what a script set
  // on it, its listeners and expandos, stays). When one out of the
  // document is collected and no newer one has been made since, the host
  // hears the handle is released, and may free the node once it is out
  // of the tree.
  const objects = new $Map();
  const registry = new $FinalizationRegistry((handle) => {
    const entry = MapGet(objects, handle);
    if (entry !== undefined && WeakRefDeref(entry.ref) !== undefined) {
      return;
    }
    MapDelete(objects, handle);
    native.released(handle);
  });

  // Holds or lets go of the objects of the nodes that came into the
  // document or left it, as the host says.
  function applyPins() {
    const pins = native.takePins();
    for (let i = 0; i < pins.length; i += 2) {
      const entry = MapGet(objects, pins[i]);
      if (entry === undefined) {
        continue;
      }
      const object = WeakRefDeref(entry.ref);
      entry.strong = pins[i + 1] && object !== undefined ? object : null;
    }
  }

  // The natives that may move nodes into or out of the document: each
  // call is followed by the pins it made.
  const MOVING = [
    "insert",
    "remove",
    "replace",
    "setTextContent",
    "setInnerHTML",
    "setOuterHTML",
    "insertAdjacentHTML",
    "setTitle",
    "write",
  ];
  for (let i = 0; i < MOVING.length; i++) {
    const call = native[MOVING[i]];
    native[MOVING[i]] = function (...args) {
      try {
        return ReflectApply(call, undefined, args);
      } finally {
        applyPins();
      }
    };
  }

  // The object for the node `handle` names, made if it has none; null
  // for null.
  function wrap(handle) {
    if (handle === null) {
      return null;
    }
    const entry = MapGet(objects, handle);
    if (entry !== undefined) {
      const object = WeakRefDeref(entry.ref);
      if (object !== undefined) {
        return object;
      }
    }
    const answer = native.wrap(handle);
    const object = make(handle, answer[0]);
    MapSet(objects, handle, {
      ref: new $WeakRef(object),
      strong: answer[1] === "" ? null : object,
    });
    Register(registry, object, handle);
    return object;
  }

  // A new object for a node, of the interface its name says. The host
  // hears it from `wrap`, and holds the node until it is released.
  function make(handle, name) {
    if (name === "#text") {
      return new Text(KEY, handle, 3, name);
    }
    if (name === "#document") {
      return new HTMLDocument(KEY, handle, 9, name);
    }
    return new (elementInterface(name))(KEY, handle, 1, name);
  }

  // An EventTarget keeps its listeners (DOM section 2.7), each a type,
  // a callback and a capture flag, none twice; nothing calls them until
  // events are dispatched (M26). The window's are kept apart: the global
  // object is no EventTarget made by a constructor.
  const windowListeners = [];
  let listenersOf;

  class EventTarget {
    #listeners = [];

    static {
      listenersOf = (target) => {
        if (typeof target === "object" && target !== null && #listeners in target) {
          return target.#listeners;
        }
        if (target === global) {
          return windowListeners;
        }
        throw illegalInvocation();
      };
    }

    addEventListener(type, callback, options) {
      const listeners = listenersOf(this);
      need(arguments.length, 2, "addEventListener", "EventTarget");
      if (callback === null || callback === undefined) {
        return;
      }
      const name = str(type);
      const capture = captureOf(options);
      for (let i = 0; i < listeners.length; i++) {
        const l = listeners[i];
        if (l.type === name && l.callback === callback && l.capture === capture) {
          return;
        }
      }
      ArrayPush(listeners, { type: name, callback, capture });
    }

    removeEventListener(type, callback, options) {
      const listeners = listenersOf(this);
      need(arguments.length, 2, "removeEventListener", "EventTarget");
      const name = str(type);
      const capture = captureOf(options);
      for (let i = 0; i < listeners.length; i++) {
        const l = listeners[i];
        if (l.type === name && l.callback === callback && l.capture === capture) {
          ArraySplice(listeners, i, 1);
          return;
        }
      }
    }
  }

  // A listener's capture flag, from its options: a boolean, or an
  // object's `capture`.
  function captureOf(options) {
    if (typeof options === "boolean") {
      return options;
    }
    return typeof options === "object" && options !== null && !!options.capture;
  }

  // The handle of `node`, which must be a Node; whether a value is one;
  // and a node's kind and name. Set from inside Node's body, where its
  // private fields are seen.
  let handleOf;
  let isNode;
  let kindOf;
  let nameOf;
  let childList;
  let elementList;
  let tokensOf;
  let styleOf;
  let datasetOf;

  class Node extends EventTarget {
    #handle;
    // `nodeType`, and the tag (lowercase) or `#text` or `#document`,
    // which never change.
    #kind;
    #name;
    // The `childNodes` and `children` collections, each made once; an
    // element's `classList`, `style` and `dataset`, the same.
    #childNodes = null;
    #children = null;
    #tokens = null;
    #style = null;
    #dataset = null;

    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        throw illegalConstructor("Node");
      }
      super();
      this.#handle = handle;
      this.#kind = kind;
      this.#name = name;
    }

    static {
      handleOf = (node) => node.#handle;
      isNode = (value) => typeof value === "object" && value !== null && #handle in value;
      kindOf = (node) => node.#kind;
      nameOf = (node) => node.#name;
      childList = (node) => {
        if (node.#childNodes === null) {
          const handle = node.#handle;
          node.#childNodes = collection(NodeList, node, () => native.children(handle), true);
        }
        return node.#childNodes;
      };
      tokensOf = (node) => {
        if (node.#tokens === null) {
          node.#tokens = tokenList(node, node.#handle, "class");
        }
        return node.#tokens;
      };
      styleOf = (node) => {
        if (node.#style === null) {
          node.#style = styleDeclaration(node, node.#handle);
        }
        return node.#style;
      };
      datasetOf = (node) => {
        if (node.#dataset === null) {
          node.#dataset = stringMap(node, node.#handle);
        }
        return node.#dataset;
      };
      elementList = (node) => {
        if (node.#children === null) {
          const handle = node.#handle;
          node.#children = collection(
            HTMLCollection,
            node,
            () => native.elementChildren(handle),
            true,
          );
        }
        return node.#children;
      };
    }

    get nodeType() {
      self(this);
      return kindOf(this);
    }

    get nodeName() {
      self(this);
      return kindOf(this) === 1 ? ToUpperCase(nameOf(this)) : nameOf(this);
    }

    get ownerDocument() {
      self(this);
      return kindOf(this) === 9 ? null : theDocument();
    }

    get parentNode() {
      return wrap(native.parent(self(this)));
    }

    get parentElement() {
      const parent = native.parent(self(this));
      return parent !== null && native.kind(parent) === 1 ? wrap(parent) : null;
    }

    get childNodes() {
      self(this);
      return childList(this);
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

    get isConnected() {
      return native.connected(self(this));
    }

    get nodeValue() {
      const handle = self(this);
      return kindOf(this) === 3 ? native.data(handle) : null;
    }

    set nodeValue(value) {
      const handle = self(this);
      if (kindOf(this) === 3) {
        native.setData(handle, strOrEmpty(value));
      }
    }

    get textContent() {
      return native.textContent(self(this));
    }

    set textContent(value) {
      const handle = self(this);
      const text = strOrEmpty(value);
      try {
        native.setTextContent(handle, text);
      } catch (error) {
        throw failed(error, "textContent", "Node");
      }
    }

    hasChildNodes() {
      return native.firstChild(self(this)) !== null;
    }

    getRootNode() {
      let at = self(this);
      for (let up = native.parent(at); up !== null; up = native.parent(at)) {
        at = up;
      }
      return wrap(at);
    }

    contains(other) {
      const handle = self(this);
      need(arguments.length, 1, "contains", "Node");
      if (other === null) {
        return false;
      }
      return native.contains(handle, nodeArgument(other, 1, "contains", "Node"));
    }

    isSameNode(other) {
      self(this);
      need(arguments.length, 1, "isSameNode", "Node");
      return this === other;
    }

    appendChild(node) {
      const parent = self(this);
      need(arguments.length, 1, "appendChild", "Node");
      const child = nodeArgument(node, 1, "appendChild", "Node");
      try {
        native.insert(parent, child, null);
      } catch (error) {
        throw failed(error, "appendChild", "Node");
      }
      return node;
    }

    insertBefore(node, child) {
      const parent = self(this);
      need(arguments.length, 2, "insertBefore", "Node");
      const handle = nodeArgument(node, 1, "insertBefore", "Node");
      const before = child === null ? null : nodeArgument(child, 2, "insertBefore", "Node");
      try {
        native.insert(parent, handle, before);
      } catch (error) {
        throw failed(error, "insertBefore", "Node");
      }
      return node;
    }

    replaceChild(node, child) {
      const parent = self(this);
      need(arguments.length, 2, "replaceChild", "Node");
      const handle = nodeArgument(node, 1, "replaceChild", "Node");
      const old = nodeArgument(child, 2, "replaceChild", "Node");
      try {
        native.replace(parent, handle, old);
      } catch (error) {
        throw failed(error, "replaceChild", "Node");
      }
      return child;
    }

    removeChild(child) {
      const parent = self(this);
      need(arguments.length, 1, "removeChild", "Node");
      const handle = nodeArgument(child, 1, "removeChild", "Node");
      try {
        native.remove(parent, handle);
      } catch (error) {
        throw failed(error, "removeChild", "Node");
      }
      return child;
    }

    cloneNode(deep) {
      const handle = self(this);
      if (kindOf(this) === 9) {
        throw domError(
          "NotSupportedError",
          "Cloning a document is not supported.",
          "cloneNode",
          "Node",
        );
      }
      try {
        return wrap(native.clone(handle, deep === undefined ? false : !!deep));
      } catch (error) {
        throw failed(error, "cloneNode", "Node");
      }
    }
  }

  const NODE_TYPES = [
    "ELEMENT_NODE", 1,
    "ATTRIBUTE_NODE", 2,
    "TEXT_NODE", 3,
    "CDATA_SECTION_NODE", 4,
    "ENTITY_REFERENCE_NODE", 5,
    "ENTITY_NODE", 6,
    "PROCESSING_INSTRUCTION_NODE", 7,
    "COMMENT_NODE", 8,
    "DOCUMENT_NODE", 9,
    "DOCUMENT_TYPE_NODE", 10,
    "DOCUMENT_FRAGMENT_NODE", 11,
    "NOTATION_NODE", 12,
  ];
  for (let i = 0; i < NODE_TYPES.length; i += 2) {
    defineProperty(Node, NODE_TYPES[i], { value: NODE_TYPES[i + 1], enumerable: true });
    defineProperty(Node.prototype, NODE_TYPES[i], { value: NODE_TYPES[i + 1], enumerable: true });
  }

  // `this` as a node's handle, or Illegal invocation.
  function self(node) {
    if (!isNode(node)) {
      throw illegalInvocation();
    }
    return handleOf(node);
  }

  // An argument that must be a Node, as its handle.
  function nodeArgument(value, position, method, iface) {
    if (!isNode(value)) {
      throw new $TypeError(
        prefix(method, iface) + "parameter " + position + " is not of type 'Node'.",
      );
    }
    return handleOf(value);
  }

  // Whether `handle` is one of `handles`.
  function among(handles, handle) {
    for (let i = 0; i < handles.length; i++) {
      if (handles[i] === handle) {
        return true;
      }
    }
    return false;
  }

  // The DOM's "convert nodes into a node", without a DocumentFragment:
  // each string made a Text node, and, when there is more than one node
  // (when the DOM makes a fragment of them), each node taken out of its
  // parent, as appending it to the fragment would; their handles in
  // order.
  function convert(nodes, method, iface) {
    const handles = [];
    for (let i = 0; i < nodes.length; i++) {
      const node = nodes[i];
      if (isNode(node)) {
        const handle = handleOf(node);
        const parent = native.parent(handle);
        if (parent !== null && nodes.length > 1) {
          native.remove(parent, handle);
        }
        ArrayPush(handles, handle);
      } else {
        const text = str(node);
        try {
          ArrayPush(handles, native.createText(text));
        } catch (error) {
          throw failed(error, method, iface);
        }
      }
    }
    return handles;
  }

  // Inserts each of `handles` under `parent` before `before`.
  function insertAll(parent, handles, before, method, iface) {
    try {
      for (let i = 0; i < handles.length; i++) {
        native.insert(parent, handles[i], before);
      }
    } catch (error) {
      throw failed(error, method, iface);
    }
  }

  // The ParentNode mixin (DOM section 4.2.6), on Document and Element.
  const ParentNode = {
    get children() {
      self(this);
      return elementList(this);
    },

    get firstElementChild() {
      let child = native.firstChild(self(this));
      while (child !== null && !isElementHandle(child)) {
        child = native.nextSibling(child);
      }
      return wrap(child);
    },

    get lastElementChild() {
      let child = native.lastChild(self(this));
      while (child !== null && !isElementHandle(child)) {
        child = native.previousSibling(child);
      }
      return wrap(child);
    },

    get childElementCount() {
      return native.elementChildren(self(this)).length;
    },

    querySelector(selectors) {
      const handle = self(this);
      const iface = kindOf(this) === 9 ? "Document" : "Element";
      need(arguments.length, 1, "querySelector", iface);
      const text = str(selectors);
      let found;
      try {
        found = native.query(handle, text, false);
      } catch (error) {
        throw failed(error, "querySelector", iface);
      }
      return found.length === 0 ? null : wrap(found[0]);
    },

    querySelectorAll(selectors) {
      const handle = self(this);
      const iface = kindOf(this) === 9 ? "Document" : "Element";
      need(arguments.length, 1, "querySelectorAll", iface);
      const text = str(selectors);
      let found;
      try {
        found = native.query(handle, text, true);
      } catch (error) {
        throw failed(error, "querySelectorAll", iface);
      }
      return snapshot(NodeList, found);
    },

    prepend(...nodes) {
      const parent = self(this);
      const iface = kindOf(this) === 9 ? "Document" : "Element";
      const handles = convert(nodes, "prepend", iface);
      insertAll(parent, handles, native.firstChild(parent), "prepend", iface);
    },

    append(...nodes) {
      const parent = self(this);
      const iface = kindOf(this) === 9 ? "Document" : "Element";
      const handles = convert(nodes, "append", iface);
      insertAll(parent, handles, null, "append", iface);
    },

    replaceChildren(...nodes) {
      const parent = self(this);
      const iface = kindOf(this) === 9 ? "Document" : "Element";
      const handles = convert(nodes, "replaceChildren", iface);
      let child = native.firstChild(parent);
      while (child !== null) {
        native.remove(parent, child);
        child = native.firstChild(parent);
      }
      insertAll(parent, handles, null, "replaceChildren", iface);
    },
  };

  // The lookups Document and Element both have: live collections of
  // the elements under the node with a tag, or with classes.
  const ElementLookups = {
    getElementsByTagName(qualifiedName) {
      const handle = self(this);
      need(
        arguments.length,
        1,
        "getElementsByTagName",
        kindOf(this) === 9 ? "Document" : "Element",
      );
      const name = str(qualifiedName);
      return collection(HTMLCollection, this, () => native.byTag(handle, name), true);
    },

    getElementsByClassName(classNames) {
      const handle = self(this);
      need(
        arguments.length,
        1,
        "getElementsByClassName",
        kindOf(this) === 9 ? "Document" : "Element",
      );
      const names = str(classNames);
      return collection(HTMLCollection, this, () => native.byClass(handle, names), true);
    },
  };

  // The ChildNode mixin (DOM section 4.2.8), on Element and
  // CharacterData.
  const ChildNode = {
    before(...nodes) {
      const handle = self(this);
      const iface = kindOf(this) === 1 ? "Element" : "CharacterData";
      const parent = native.parent(handle);
      if (parent === null) {
        return;
      }
      let viable = native.previousSibling(handle);
      while (viable !== null && amongNodes(nodes, viable)) {
        viable = native.previousSibling(viable);
      }
      const handles = convert(nodes, "before", iface);
      const before = viable === null ? native.firstChild(parent) : native.nextSibling(viable);
      insertAll(parent, handles, before, "before", iface);
    },

    after(...nodes) {
      const handle = self(this);
      const iface = kindOf(this) === 1 ? "Element" : "CharacterData";
      const parent = native.parent(handle);
      if (parent === null) {
        return;
      }
      let viable = native.nextSibling(handle);
      while (viable !== null && amongNodes(nodes, viable)) {
        viable = native.nextSibling(viable);
      }
      const handles = convert(nodes, "after", iface);
      insertAll(parent, handles, viable, "after", iface);
    },

    replaceWith(...nodes) {
      const handle = self(this);
      const iface = kindOf(this) === 1 ? "Element" : "CharacterData";
      const parent = native.parent(handle);
      if (parent === null) {
        return;
      }
      let viable = native.nextSibling(handle);
      while (viable !== null && amongNodes(nodes, viable)) {
        viable = native.nextSibling(viable);
      }
      const handles = convert(nodes, "replaceWith", iface);
      if (native.parent(handle) === parent) {
        const before = native.nextSibling(handle);
        native.remove(parent, handle);
        insertAll(parent, handles, before, "replaceWith", iface);
      } else {
        insertAll(parent, handles, viable, "replaceWith", iface);
      }
    },

    remove() {
      const handle = self(this);
      const parent = native.parent(handle);
      if (parent !== null) {
        native.remove(parent, handle);
      }
    },
  };

  // The NonDocumentTypeChildNode mixin, on Element and CharacterData.
  const NonDocumentTypeChildNode = {
    get previousElementSibling() {
      let sibling = native.previousSibling(self(this));
      while (sibling !== null && !isElementHandle(sibling)) {
        sibling = native.previousSibling(sibling);
      }
      return wrap(sibling);
    },

    get nextElementSibling() {
      let sibling = native.nextSibling(self(this));
      while (sibling !== null && !isElementHandle(sibling)) {
        sibling = native.nextSibling(sibling);
      }
      return wrap(sibling);
    },
  };

  // Whether a node of `nodes`, the arguments of a ChildNode method, is
  // the node `handle` names.
  function amongNodes(nodes, handle) {
    for (let i = 0; i < nodes.length; i++) {
      if (isNode(nodes[i]) && handleOf(nodes[i]) === handle) {
        return true;
      }
    }
    return false;
  }

  function isElementHandle(handle) {
    return native.kind(handle) === 1;
  }

  class CharacterData extends Node {
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        throw illegalConstructor("CharacterData");
      }
      super(key, handle, kind, name);
    }

    get data() {
      return native.data(self(this));
    }

    set data(value) {
      const handle = self(this);
      const data = strOrEmpty(value);
      try {
        native.setData(handle, data);
      } catch (error) {
        throw failed(error, "data", "CharacterData");
      }
    }

    get length() {
      return native.data(self(this)).length;
    }

    substringData(offset, count) {
      const handle = self(this);
      need(arguments.length, 2, "substringData", "CharacterData");
      const data = native.data(handle);
      const at = checkOffset(data, unsigned(offset), "substringData");
      return StringSlice(data, at, at + unsigned(count));
    }

    appendData(data) {
      const handle = self(this);
      need(arguments.length, 1, "appendData", "CharacterData");
      setData(handle, native.data(handle) + str(data), "appendData");
    }

    insertData(offset, data) {
      const handle = self(this);
      need(arguments.length, 2, "insertData", "CharacterData");
      const old = native.data(handle);
      const at = checkOffset(old, unsigned(offset), "insertData");
      setData(handle, StringSlice(old, 0, at) + str(data) + StringSlice(old, at), "insertData");
    }

    deleteData(offset, count) {
      const handle = self(this);
      need(arguments.length, 2, "deleteData", "CharacterData");
      const old = native.data(handle);
      const at = checkOffset(old, unsigned(offset), "deleteData");
      const rest = StringSlice(old, at + unsigned(count));
      setData(handle, StringSlice(old, 0, at) + rest, "deleteData");
    }

    replaceData(offset, count, data) {
      const handle = self(this);
      need(arguments.length, 3, "replaceData", "CharacterData");
      const old = native.data(handle);
      const at = checkOffset(old, unsigned(offset), "replaceData");
      const text = str(data);
      setData(
        handle,
        StringSlice(old, 0, at) + text + StringSlice(old, at + unsigned(count)),
        "replaceData",
      );
    }
  }

  // `offset` if it is within `data` (in UTF-16 code units, as the DOM
  // counts), else the IndexSizeError the reference browser throws.
  function checkOffset(data, offset, method) {
    if (offset > data.length) {
      throw domError(
        "IndexSizeError",
        "The offset " + offset + " is greater than the node's length (" + data.length + ").",
        method,
        "CharacterData",
      );
    }
    return offset;
  }

  function setData(handle, data, method) {
    try {
      native.setData(handle, data);
    } catch (error) {
      throw failed(error, method, "CharacterData");
    }
  }

  class Text extends CharacterData {
    // `new Text(data)` is a script's to make, as the DOM lets it.
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        const data = key === undefined ? "" : str(key);
        try {
          return wrap(native.createText(data));
        } catch (error) {
          throw failed(error, "Text", "Text");
        }
      }
      super(key, handle, kind, name);
    }

    splitText(offset) {
      const handle = self(this);
      need(arguments.length, 1, "splitText", "Text");
      const data = native.data(handle);
      const at = unsigned(offset);
      if (at > data.length) {
        throw domError(
          "IndexSizeError",
          "The offset " + at + " is larger than the Text node's length.",
          "splitText",
          "Text",
        );
      }
      let made;
      try {
        made = native.createText(StringSlice(data, at));
      } catch (error) {
        throw failed(error, "splitText", "Text");
      }
      const parent = native.parent(handle);
      if (parent !== null) {
        native.insert(parent, made, native.nextSibling(handle));
      }
      native.setData(handle, StringSlice(data, 0, at));
      return wrap(made);
    }

    get wholeText() {
      let first = self(this);
      for (let p = native.previousSibling(first); p !== null && native.kind(p) === 3; ) {
        first = p;
        p = native.previousSibling(p);
      }
      let text = "";
      for (let n = first; n !== null && native.kind(n) === 3; n = native.nextSibling(n)) {
        text += native.data(n);
      }
      return text;
    }
  }

  // Whether `name` is a valid attribute local name (DOM section 1.5):
  // not empty, and no ASCII white space, NUL, `/`, `=` or `>`.
  function validAttributeName(name) {
    if (name.length === 0) {
      return false;
    }
    for (let i = 0; i < name.length; i++) {
      const c = CharCodeAt(name, i);
      if (c === 9 || c === 10 || c === 12 || c === 13 || c === 32 || c === 0 || c === 47 ||
          c === 61 || c === 62) {
        return false;
      }
    }
    return true;
  }

  function badAttributeName(name, method) {
    return domError(
      "InvalidCharacterError",
      "'" + name + "' is not a valid attribute name.",
      method,
      "Element",
    );
  }

  class Element extends Node {
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        throw illegalConstructor("Element");
      }
      super(key, handle, kind, name);
    }

    get tagName() {
      self(this);
      return ToUpperCase(nameOf(this));
    }

    get localName() {
      self(this);
      return nameOf(this);
    }

    get id() {
      const value = native.attribute(self(this), "id");
      return value === null ? "" : value;
    }

    set id(value) {
      setAttribute(self(this), "id", str(value), "id");
    }

    get className() {
      const value = native.attribute(self(this), "class");
      return value === null ? "" : value;
    }

    set className(value) {
      setAttribute(self(this), "class", str(value), "className");
    }

    get classList() {
      self(this);
      return tokensOf(this);
    }

    set classList(value) {
      self(this);
      tokensOf(this).value = value;
    }

    get style() {
      self(this);
      return styleOf(this);
    }

    set style(value) {
      self(this);
      styleOf(this).cssText = value;
    }

    get dataset() {
      self(this);
      return datasetOf(this);
    }

    getAttribute(name) {
      const handle = self(this);
      need(arguments.length, 1, "getAttribute", "Element");
      return native.attribute(handle, ToLowerCase(str(name)));
    }

    getAttributeNames() {
      return native.attributeNames(self(this));
    }

    hasAttribute(name) {
      const handle = self(this);
      need(arguments.length, 1, "hasAttribute", "Element");
      return native.attribute(handle, ToLowerCase(str(name))) !== null;
    }

    hasAttributes() {
      return native.attributeNames(self(this)).length > 0;
    }

    setAttribute(name, value) {
      const handle = self(this);
      need(arguments.length, 2, "setAttribute", "Element");
      const key = str(name);
      if (!validAttributeName(key)) {
        throw badAttributeName(key, "setAttribute");
      }
      setAttribute(handle, ToLowerCase(key), str(value), "setAttribute");
    }

    removeAttribute(name) {
      const handle = self(this);
      need(arguments.length, 1, "removeAttribute", "Element");
      native.removeAttribute(handle, ToLowerCase(str(name)));
    }

    toggleAttribute(name, force) {
      const handle = self(this);
      need(arguments.length, 1, "toggleAttribute", "Element");
      const key = str(name);
      if (!validAttributeName(key)) {
        throw badAttributeName(key, "toggleAttribute");
      }
      const lower = ToLowerCase(key);
      if (native.attribute(handle, lower) === null) {
        if (force === undefined || force) {
          setAttribute(handle, lower, "", "toggleAttribute");
          return true;
        }
        return false;
      }
      if (force === undefined || !force) {
        native.removeAttribute(handle, lower);
        return false;
      }
      return true;
    }

    get innerHTML() {
      return native.innerHTML(self(this));
    }

    set innerHTML(value) {
      const handle = self(this);
      const markup = strOrEmpty(value);
      try {
        native.setInnerHTML(handle, markup);
      } catch (error) {
        throw failedSet(error, "innerHTML", "Element");
      }
    }

    get outerHTML() {
      return native.outerHTML(self(this));
    }

    set outerHTML(value) {
      const handle = self(this);
      const markup = strOrEmpty(value);
      try {
        native.setOuterHTML(handle, markup);
      } catch (error) {
        throw failedSet(error, "outerHTML", "Element");
      }
    }

    insertAdjacentHTML(position, text) {
      const handle = self(this);
      need(arguments.length, 2, "insertAdjacentHTML", "Element");
      const where = checkPosition(str(position), "insertAdjacentHTML");
      const markup = str(text);
      try {
        native.insertAdjacentHTML(handle, where, markup);
      } catch (error) {
        throw failed(error, "insertAdjacentHTML", "Element");
      }
    }

    matches(selectors) {
      const handle = self(this);
      need(arguments.length, 1, "matches", "Element");
      const text = str(selectors);
      try {
        return native.matches(handle, text);
      } catch (error) {
        throw failed(error, "matches", "Element");
      }
    }

    webkitMatchesSelector(selectors) {
      const handle = self(this);
      need(arguments.length, 1, "webkitMatchesSelector", "Element");
      const text = str(selectors);
      try {
        return native.matches(handle, text);
      } catch (error) {
        throw failed(error, "webkitMatchesSelector", "Element");
      }
    }

    closest(selectors) {
      const handle = self(this);
      need(arguments.length, 1, "closest", "Element");
      const text = str(selectors);
      try {
        return wrap(native.closest(handle, text));
      } catch (error) {
        throw failed(error, "closest", "Element");
      }
    }

    insertAdjacentElement(where, element) {
      const handle = self(this);
      need(arguments.length, 2, "insertAdjacentElement", "Element");
      const position = str(where);
      if (!isNode(element) || kindOf(element) !== 1) {
        throw new $TypeError(
          prefix("insertAdjacentElement", "Element") + "parameter 2 is not of type 'Element'.",
        );
      }
      return insertAdjacent(handle, position, handleOf(element), "insertAdjacentElement")
        ? element
        : null;
    }

    insertAdjacentText(where, data) {
      const handle = self(this);
      need(arguments.length, 2, "insertAdjacentText", "Element");
      const position = str(where);
      const text = str(data);
      checkPosition(position, "insertAdjacentText");
      let made;
      try {
        made = native.createText(text);
      } catch (error) {
        throw failed(error, "insertAdjacentText", "Element");
      }
      insertAdjacent(handle, position, made, "insertAdjacentText");
    }
  }

  function setAttribute(handle, name, value, method) {
    try {
      native.setAttribute(handle, name, value);
    } catch (error) {
      throw failed(error, method, "Element");
    }
  }

  // The position an insertAdjacent method names, lowercased; or the
  // SyntaxError the reference browser throws.
  function checkPosition(where, method) {
    const position = ToLowerCase(where);
    if (
      position !== "beforebegin" && position !== "afterbegin" && position !== "beforeend" &&
      position !== "afterend"
    ) {
      throw domError(
        "SyntaxError",
        "The value provided ('" + where + "') is not one of 'beforeBegin', 'afterBegin', " +
          "'beforeEnd', or 'afterEnd'.",
        method,
        "Element",
      );
    }
    return position;
  }

  // The DOM's "insert adjacent": whether the node went in (not before or
  // after an element with no parent).
  function insertAdjacent(element, where, node, method) {
    const position = checkPosition(where, method);
    let parent = element;
    let before = null;
    if (position === "beforebegin" || position === "afterend") {
      parent = native.parent(element);
      if (parent === null) {
        return false;
      }
      before = position === "beforebegin" ? element : native.nextSibling(element);
    } else if (position === "afterbegin") {
      before = native.firstChild(element);
    }
    try {
      native.insert(parent, node, before);
    } catch (error) {
      throw failed(error, method, "Element");
    }
    return true;
  }

  class HTMLElement extends Element {
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        throw illegalConstructor("HTMLElement");
      }
      super(key, handle, kind, name);
    }
  }

  class Document extends Node {
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        throw illegalConstructor("Document");
      }
      super(key, handle, kind, name);
    }

    get documentElement() {
      return wrap(documentElement(self(this)));
    }

    get head() {
      return wrap(childOfHtml(self(this), "head"));
    }

    get body() {
      return wrap(childOfHtml(self(this), "body"));
    }

    get defaultView() {
      self(this);
      return global;
    }

    get textContent() {
      self(this);
      return null;
    }

    set textContent(value) {
      self(this);
    }

    get title() {
      self(this);
      return native.title();
    }

    set title(value) {
      self(this);
      native.setTitle(str(value));
    }

    get currentScript() {
      self(this);
      return wrap(native.currentScript());
    }

    get URL() {
      self(this);
      return native.documentInfo()[0];
    }

    get documentURI() {
      self(this);
      return native.documentInfo()[0];
    }

    get characterSet() {
      self(this);
      return native.documentInfo()[1];
    }

    get charset() {
      self(this);
      return native.documentInfo()[1];
    }

    get inputEncoding() {
      self(this);
      return native.documentInfo()[1];
    }

    get contentType() {
      self(this);
      return native.documentInfo()[2];
    }

    get compatMode() {
      self(this);
      return native.documentInfo()[3];
    }

    get location() {
      self(this);
      return location;
    }

    set location(value) {
      self(this);
      location.href = value;
    }

    get readyState() {
      self(this);
      return native.readyState();
    }

    write(...text) {
      self(this);
      write(text, "", "write");
    }

    writeln(...text) {
      self(this);
      write(text, "\n", "writeln");
    }

    getElementById(elementId) {
      const handle = self(this);
      need(arguments.length, 1, "getElementById", "Document");
      return wrap(native.elementById(handle, str(elementId)));
    }

    createElement(localName) {
      self(this);
      need(arguments.length, 1, "createElement", "Document");
      const name = str(localName);
      try {
        return wrap(native.createElement(name));
      } catch (error) {
        throw failed(error, "createElement", "Document");
      }
    }

    createTextNode(data) {
      self(this);
      need(arguments.length, 1, "createTextNode", "Document");
      const text = str(data);
      try {
        return wrap(native.createText(text));
      } catch (error) {
        throw failed(error, "createTextNode", "Document");
      }
    }
  }

  // `document.write`'s text, its arguments joined and `end` after them,
  // put in at the parser's insertion point (HTML section 8.4.3).
  function write(text, end, method) {
    let markup = "";
    for (let i = 0; i < text.length; i++) {
      markup += str(text[i]);
    }
    markup += end;
    try {
      native.write(markup);
    } catch (error) {
      throw failed(error, method, "Document");
    }
  }

  // The document's element: its first element child.
  function documentElement(document) {
    let child = native.firstChild(document);
    while (child !== null && native.kind(child) !== 1) {
      child = native.nextSibling(child);
    }
    return child;
  }

  // The first child of the `html` element with local name `tag`
  // (`head`, or `body`, which `frameset` also is: HTML section 3.1.7).
  function childOfHtml(document, tag) {
    const html = documentElement(document);
    if (html === null || native.tag(html) !== "html") {
      return null;
    }
    for (let child = native.firstChild(html); child !== null; child = native.nextSibling(child)) {
      if (native.kind(child) !== 1) {
        continue;
      }
      const name = native.tag(child);
      if (name === tag || (tag === "body" && name === "frameset")) {
        return child;
      }
    }
    return null;
  }

  class HTMLDocument extends Document {
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        throw illegalConstructor("HTMLDocument");
      }
      super(key, handle, kind, name);
    }
  }

  include(ParentNode, [Document.prototype, Element.prototype]);
  include(ElementLookups, [Document.prototype, Element.prototype]);
  include(ChildNode, [Element.prototype, CharacterData.prototype]);
  include(NonDocumentTypeChildNode, [Element.prototype, CharacterData.prototype]);

  expose(EventTarget, "EventTarget");
  expose(Node, "Node");
  expose(CharacterData, "CharacterData");
  expose(Text, "Text");
  expose(Element, "Element");
  expose(HTMLElement, "HTMLElement");
  expose(Document, "Document");
  expose(HTMLDocument, "HTMLDocument");

  // ---- Collections ---------------------------------------------------

  // A collection's state, by the collection (its proxy) and by its
  // target: the node it is of, which it keeps alive; how its items are
  // found; whether it is live; and its items, handles, with the
  // document's version they are from.
  const collections = new $WeakMap();

  // A new collection of class `cls` over `fetch`'s handles: live ones
  // fetch again when the document has changed since.
  function collection(cls, owner, fetch, live) {
    const target = new cls(KEY);
    const state = { owner, fetch, live, version: -1, items: null };
    const proxy = new $Proxy(target, handlers);
    WeakMapSet(collections, proxy, state);
    WeakMapSet(collections, target, state);
    return proxy;
  }

  // A static collection of the nodes `handles` name, made now.
  function snapshot(cls, handles) {
    const target = new cls(KEY);
    const items = [];
    for (let i = 0; i < handles.length; i++) {
      ArrayPush(items, wrap(handles[i]));
    }
    const state = { owner: null, fetch: null, live: false, version: 0, items: handles };
    // The objects, kept with the collection: a static one holds its nodes.
    state.objects = items;
    const proxy = new $Proxy(target, handlers);
    WeakMapSet(collections, proxy, state);
    WeakMapSet(collections, target, state);
    return proxy;
  }

  function items(state) {
    if (state.live) {
      const version = native.version();
      if (state.items === null || state.version !== version) {
        state.items = state.fetch();
        state.version = version;
      }
    }
    return state.items;
  }

  function stateOf(list) {
    const state = WeakMapGet(collections, list);
    if (state === undefined) {
      throw illegalInvocation();
    }
    return state;
  }

  function itemOf(state, i) {
    const handles = items(state);
    return i < handles.length ? wrap(handles[i]) : null;
  }

  // An array index property's index, or -1.
  function arrayIndex(key) {
    if (typeof key !== "string") {
      return -1;
    }
    const n = key >>> 0;
    return $String(n) === key && n !== 4294967295 ? n : -1;
  }

  // The first element of `state`'s items whose ID or `name` is `name`.
  function namedItem(state, name) {
    if (name === "") {
      return null;
    }
    const handles = items(state);
    for (let i = 0; i < handles.length; i++) {
      if (native.attribute(handles[i], "id") === name) {
        return wrap(handles[i]);
      }
    }
    for (let i = 0; i < handles.length; i++) {
      if (native.attribute(handles[i], "name") === name) {
        return wrap(handles[i]);
      }
    }
    return null;
  }

  // The indexed properties of NodeList and HTMLCollection, and
  // HTMLCollection's named ones, read through each access.
  const handlers = {
    get(target, key, receiver) {
      const i = arrayIndex(key);
      const state = WeakMapGet(collections, target);
      if (i >= 0) {
        const item = itemOf(state, i);
        return item === null ? undefined : item;
      }
      if (
        typeof key === "string" && target instanceof HTMLCollection && !ReflectHas(target, key)
      ) {
        const item = namedItem(state, key);
        if (item !== null) {
          return item;
        }
      }
      return ReflectGet(target, key, receiver);
    },
    has(target, key) {
      const i = arrayIndex(key);
      if (i >= 0) {
        return i < items(WeakMapGet(collections, target)).length;
      }
      return ReflectHas(target, key);
    },
    ownKeys(target) {
      const keys = [];
      const count = items(WeakMapGet(collections, target)).length;
      for (let i = 0; i < count; i++) {
        ArrayPush(keys, $String(i));
      }
      const own = ReflectOwnKeys(target);
      for (let i = 0; i < own.length; i++) {
        ArrayPush(keys, own[i]);
      }
      return keys;
    },
    getOwnPropertyDescriptor(target, key) {
      const i = arrayIndex(key);
      if (i >= 0) {
        const item = itemOf(WeakMapGet(collections, target), i);
        if (item === null) {
          return undefined;
        }
        return { value: item, writable: false, enumerable: true, configurable: true };
      }
      return ReflectGetOwnPropertyDescriptor(target, key);
    },
    set(target, key, value) {
      if (arrayIndex(key) >= 0) {
        return false;
      }
      return ReflectSet(target, key, value);
    },
    defineProperty(target, key, desc) {
      if (arrayIndex(key) >= 0) {
        return false;
      }
      return ReflectDefineProperty(target, key, desc);
    },
    deleteProperty(target, key) {
      const i = arrayIndex(key);
      if (i >= 0) {
        return i >= items(WeakMapGet(collections, target)).length;
      }
      return ReflectDeleteProperty(target, key);
    },
  };

  class NodeList {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("NodeList");
      }
    }

    get length() {
      return items(stateOf(this)).length;
    }

    item(index) {
      const state = stateOf(this);
      need(arguments.length, 1, "item", "NodeList");
      return itemOf(state, unsigned(index));
    }
  }
  NodeList.prototype.forEach = ArrayForEach;
  NodeList.prototype.entries = ArrayEntries;
  NodeList.prototype.keys = ArrayKeys;
  NodeList.prototype.values = ArrayValues;
  expose(NodeList, "NodeList");
  defineProperty(NodeList.prototype, iteratorSymbol, {
    value: ArrayValues,
    writable: true,
    configurable: true,
  });

  class HTMLCollection {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("HTMLCollection");
      }
    }

    get length() {
      return items(stateOf(this)).length;
    }

    item(index) {
      const state = stateOf(this);
      need(arguments.length, 1, "item", "HTMLCollection");
      return itemOf(state, unsigned(index));
    }

    namedItem(name) {
      const state = stateOf(this);
      need(arguments.length, 1, "namedItem", "HTMLCollection");
      return namedItem(state, str(name));
    }
  }
  expose(HTMLCollection, "HTMLCollection");
  defineProperty(HTMLCollection.prototype, iteratorSymbol, {
    value: ArrayValues,
    writable: true,
    configurable: true,
  });

  // ---- DOMTokenList (DOM section 7.1) ---------------------------------

  // Whether `c`, a code unit, is ASCII white space.
  function isSpace(c) {
    return c === 9 || c === 10 || c === 12 || c === 13 || c === 32;
  }

  // The DOM's ordered set parser: the tokens of `text` between ASCII
  // white space, each once, in order.
  function orderedSet(text) {
    const tokens = [];
    let token = "";
    for (let i = 0; i <= text.length; i++) {
      const c = i < text.length ? CharCodeAt(text, i) : 32;
      if (isSpace(c)) {
        if (token !== "" && !includesString(tokens, token)) {
          ArrayPush(tokens, token);
        }
        token = "";
      } else {
        token += text[i];
      }
    }
    return tokens;
  }

  function includesString(list, value) {
    for (let i = 0; i < list.length; i++) {
      if (list[i] === value) {
        return true;
      }
    }
    return false;
  }

  // A token list's state: its element and attribute.
  const tokenLists = new $WeakMap();

  function tokenList(owner, handle, attribute) {
    const target = new DOMTokenList(KEY);
    const state = { owner, handle, attribute };
    const proxy = new $Proxy(target, tokenHandlers);
    WeakMapSet(tokenLists, proxy, state);
    WeakMapSet(tokenLists, target, state);
    return proxy;
  }

  function tokenState(list) {
    const state = WeakMapGet(tokenLists, list);
    if (state === undefined) {
      throw illegalInvocation();
    }
    return state;
  }

  function tokensIn(state) {
    const value = native.attribute(state.handle, state.attribute);
    return value === null ? [] : orderedSet(value);
  }

  // The DOM's update steps: the attribute set to the tokens, unless it
  // is absent and there are none.
  function updateTokens(state, tokens, method) {
    if (tokens.length === 0 && native.attribute(state.handle, state.attribute) === null) {
      return;
    }
    let text = "";
    for (let i = 0; i < tokens.length; i++) {
      text += (i === 0 ? "" : " ") + tokens[i];
    }
    setAttribute(state.handle, state.attribute, text, method);
  }

  // A token given to add, remove, toggle or replace: not empty, and
  // without white space.
  function checkToken(value, method) {
    const token = str(value);
    if (token === "") {
      throw domError("SyntaxError", "The token provided must not be empty.", method, "DOMTokenList");
    }
    for (let i = 0; i < token.length; i++) {
      if (isSpace(CharCodeAt(token, i))) {
        throw domError(
          "InvalidCharacterError",
          "The token provided ('" + token + "') contains HTML space characters, which are not " +
            "valid in tokens.",
          method,
          "DOMTokenList",
        );
      }
    }
    return token;
  }

  class DOMTokenList {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("DOMTokenList");
      }
    }

    get length() {
      return tokensIn(tokenState(this)).length;
    }

    item(index) {
      const state = tokenState(this);
      need(arguments.length, 1, "item", "DOMTokenList");
      const tokens = tokensIn(state);
      const i = unsigned(index);
      return i < tokens.length ? tokens[i] : null;
    }

    contains(token) {
      const state = tokenState(this);
      need(arguments.length, 1, "contains", "DOMTokenList");
      return includesString(tokensIn(state), str(token));
    }

    add(...tokens) {
      const state = tokenState(this);
      const checked = [];
      for (let i = 0; i < tokens.length; i++) {
        ArrayPush(checked, checkToken(tokens[i], "add"));
      }
      const set = tokensIn(state);
      for (let i = 0; i < checked.length; i++) {
        if (!includesString(set, checked[i])) {
          ArrayPush(set, checked[i]);
        }
      }
      updateTokens(state, set, "add");
    }

    remove(...tokens) {
      const state = tokenState(this);
      const checked = [];
      for (let i = 0; i < tokens.length; i++) {
        ArrayPush(checked, checkToken(tokens[i], "remove"));
      }
      const set = tokensIn(state);
      const kept = [];
      for (let i = 0; i < set.length; i++) {
        if (!includesString(checked, set[i])) {
          ArrayPush(kept, set[i]);
        }
      }
      updateTokens(state, kept, "remove");
    }

    toggle(token, force) {
      const state = tokenState(this);
      need(arguments.length, 1, "toggle", "DOMTokenList");
      const name = checkToken(token, "toggle");
      const set = tokensIn(state);
      if (includesString(set, name)) {
        if (force === undefined || !force) {
          const kept = [];
          for (let i = 0; i < set.length; i++) {
            if (set[i] !== name) {
              ArrayPush(kept, set[i]);
            }
          }
          updateTokens(state, kept, "toggle");
          return false;
        }
        return true;
      }
      if (force === undefined || force) {
        ArrayPush(set, name);
        updateTokens(state, set, "toggle");
        return true;
      }
      return false;
    }

    replace(token, newToken) {
      const state = tokenState(this);
      need(arguments.length, 2, "replace", "DOMTokenList");
      const old = checkToken(token, "replace");
      const made = checkToken(newToken, "replace");
      const set = tokensIn(state);
      if (!includesString(set, old)) {
        return false;
      }
      const result = [];
      for (let i = 0; i < set.length; i++) {
        const t = set[i] === old ? made : set[i];
        if (!includesString(result, t)) {
          ArrayPush(result, t);
        }
      }
      updateTokens(state, result, "replace");
      return true;
    }

    supports(token) {
      tokenState(this);
      throw new $TypeError(
        prefix("supports", "DOMTokenList") + "DOMTokenList has no supported tokens.",
      );
    }

    get value() {
      const state = tokenState(this);
      const value = native.attribute(state.handle, state.attribute);
      return value === null ? "" : value;
    }

    set value(text) {
      const state = tokenState(this);
      setAttribute(state.handle, state.attribute, str(text), "value");
    }

    toString() {
      return this.value;
    }
  }
  DOMTokenList.prototype.forEach = ArrayForEach;
  DOMTokenList.prototype.entries = ArrayEntries;
  DOMTokenList.prototype.keys = ArrayKeys;
  DOMTokenList.prototype.values = ArrayValues;
  expose(DOMTokenList, "DOMTokenList");
  defineProperty(DOMTokenList.prototype, iteratorSymbol, {
    value: ArrayValues,
    writable: true,
    configurable: true,
  });

  // A token list's indexed properties, read through each access.
  const tokenHandlers = indexedHandlers(
    (target) => tokensIn(WeakMapGet(tokenLists, target)),
  );

  // Proxy handlers for an object of `target`'s with indexed properties,
  // the items `items(target)` gives, read-only.
  function indexedHandlers(items) {
    return {
      get(target, key, receiver) {
        const i = arrayIndex(key);
        if (i >= 0) {
          const list = items(target);
          return i < list.length ? list[i] : undefined;
        }
        return ReflectGet(target, key, receiver);
      },
      has(target, key) {
        const i = arrayIndex(key);
        return i >= 0 ? i < items(target).length : ReflectHas(target, key);
      },
      ownKeys(target) {
        const keys = [];
        const count = items(target).length;
        for (let i = 0; i < count; i++) {
          ArrayPush(keys, $String(i));
        }
        const own = ReflectOwnKeys(target);
        for (let i = 0; i < own.length; i++) {
          ArrayPush(keys, own[i]);
        }
        return keys;
      },
      getOwnPropertyDescriptor(target, key) {
        const i = arrayIndex(key);
        if (i >= 0) {
          const list = items(target);
          if (i >= list.length) {
            return undefined;
          }
          return { value: list[i], writable: false, enumerable: true, configurable: true };
        }
        return ReflectGetOwnPropertyDescriptor(target, key);
      },
      set(target, key, value) {
        return arrayIndex(key) >= 0 ? false : ReflectSet(target, key, value);
      },
      defineProperty(target, key, desc) {
        return arrayIndex(key) >= 0 ? false : ReflectDefineProperty(target, key, desc);
      },
      deleteProperty(target, key) {
        const i = arrayIndex(key);
        return i >= 0 ? i >= items(target).length : ReflectDeleteProperty(target, key);
      },
    };
  }

  // ---- CSSStyleDeclaration (CSSOM section 6.6) -------------------------

  // A declaration block's state: the element whose `style` attribute it
  // reads and writes.
  const declarationBlocks = new $WeakMap();

  function styleDeclaration(owner, handle) {
    const target = new CSSStyleDeclaration(KEY);
    const state = { owner, handle };
    const proxy = new $Proxy(target, styleHandlers);
    WeakMapSet(declarationBlocks, proxy, state);
    WeakMapSet(declarationBlocks, target, state);
    return proxy;
  }

  function blockState(block) {
    const state = WeakMapGet(declarationBlocks, block);
    if (state === undefined) {
      throw illegalInvocation();
    }
    return state;
  }

  // The block's declarations: name, value, priority, three in a row.
  function declarationsOf(state) {
    const text = native.attribute(state.handle, "style");
    return text === null ? [] : native.parseDeclarations(text);
  }

  function writeDeclarations(state, flat, method) {
    let text = "";
    for (let i = 0; i < flat.length; i += 3) {
      text += (i === 0 ? "" : " ") + flat[i] + ": " + flat[i + 1];
      text += (flat[i + 2] === "" ? "" : " !important") + ";";
    }
    setAttribute(state.handle, "style", text, method);
  }

  // A property's name as a block keeps it: lowercase, but a custom one.
  function propertyName(name) {
    return name.length > 1 && name[0] === "-" && name[1] === "-" ? name : ToLowerCase(name);
  }

  function findDeclaration(flat, name) {
    for (let i = 0; i < flat.length; i += 3) {
      if (flat[i] === name) {
        return i;
      }
    }
    return -1;
  }

  class CSSStyleDeclaration {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("CSSStyleDeclaration");
      }
    }

    get cssText() {
      const state = blockState(this);
      if (state.computed) {
        return "";
      }
      const flat = declarationsOf(state);
      let text = "";
      for (let i = 0; i < flat.length; i += 3) {
        text += (i === 0 ? "" : " ") + flat[i] + ": " + flat[i + 1];
        text += (flat[i + 2] === "" ? "" : " !important") + ";";
      }
      return text;
    }

    set cssText(value) {
      const state = blockState(this);
      if (state.computed) {
        throw new $DOMException(
          "Failed to set the 'cssText' property on 'CSSStyleDeclaration': These styles are " +
            "computed, and therefore read-only.",
          "NoModificationAllowedError",
        );
      }
      writeDeclarations(state, native.parseDeclarations(strOrEmpty(value)), "cssText");
    }

    get length() {
      const state = blockState(this);
      if (state.computed) {
        return computedProperties().length;
      }
      return declarationsOf(state).length / 3;
    }

    item(index) {
      const state = blockState(this);
      need(arguments.length, 1, "item", "CSSStyleDeclaration");
      if (state.computed) {
        const names = computedProperties();
        const i = unsigned(index);
        return i < names.length ? names[i] : "";
      }
      const flat = declarationsOf(state);
      const i = unsigned(index) * 3;
      return i < flat.length ? flat[i] : "";
    }

    getPropertyValue(property) {
      const state = blockState(this);
      need(arguments.length, 1, "getPropertyValue", "CSSStyleDeclaration");
      if (state.computed) {
        return computedValue(state, lookupName(str(property)));
      }
      const flat = declarationsOf(state);
      const at = findDeclaration(flat, lookupName(str(property)));
      return at < 0 ? "" : flat[at + 1];
    }

    getPropertyPriority(property) {
      const state = blockState(this);
      need(arguments.length, 1, "getPropertyPriority", "CSSStyleDeclaration");
      if (state.computed) {
        return "";
      }
      const flat = declarationsOf(state);
      const at = findDeclaration(flat, lookupName(str(property)));
      return at < 0 ? "" : flat[at + 2];
    }

    setProperty(property, value, priority) {
      const state = blockState(this);
      need(arguments.length, 2, "setProperty", "CSSStyleDeclaration");
      const name = str(property);
      refuseComputed(state, "setProperty", name);
      setProperty(state, name, strOrEmpty(value), priority, "setProperty");
    }

    removeProperty(property) {
      const state = blockState(this);
      need(arguments.length, 1, "removeProperty", "CSSStyleDeclaration");
      const name = str(property);
      refuseComputed(state, "removeProperty", name);
      return removeProperty(state, lookupName(name));
    }

    get cssFloat() {
      return this.getPropertyValue("float");
    }

    set cssFloat(value) {
      const state = blockState(this);
      refuseComputed(state, "cssFloat", "float");
      setProperty(state, "float", strOrEmpty(value), "", "cssFloat");
    }
  }
  expose(CSSStyleDeclaration, "CSSStyleDeclaration");

  // A computed style (CSSOM's getComputedStyle): the element's values
  // read through the host as they stand when asked, nothing written.
  function computedStyle(owner, handle, pseudo) {
    const target = new CSSStyleDeclaration(KEY);
    const state = { owner, handle, pseudo, computed: true };
    const proxy = new $Proxy(target, styleHandlers);
    WeakMapSet(declarationBlocks, proxy, state);
    WeakMapSet(declarationBlocks, target, state);
    return proxy;
  }

  function computedValue(state, name) {
    const value = native.computedValue(state.handle, state.pseudo, name);
    return value === null ? "" : value;
  }

  // The properties a computed style lists, asked once.
  let computedNames = null;
  function computedProperties() {
    if (computedNames === null) {
      computedNames = native.computedProperties();
    }
    return computedNames;
  }

  // The NoModificationAllowedError a computed style's writes throw.
  function refuseComputed(state, method, name) {
    if (state.computed) {
      throw new $DOMException(
        "Failed to execute '" + method + "' on 'CSSStyleDeclaration': These styles are " +
          "computed, and therefore the '" + name + "' property is read-only.",
        "NoModificationAllowedError",
      );
    }
  }

  // CSSOM's setProperty: an unknown property, a bad priority or an
  // invalid value changes nothing; the empty value removes it.
  function setProperty(state, property, value, priority, method) {
    const name = canonicalProperty(property);
    if (name === null) {
      return;
    }
    if (value === "") {
      removeProperty(state, name);
      return;
    }
    const wanted = priority === undefined ? "" : ToLowerCase(str(priority));
    if (wanted !== "" && wanted !== "important") {
      return;
    }
    if (!native.cssValid(name, value)) {
      return;
    }
    const flat = declarationsOf(state);
    const at = findDeclaration(flat, name);
    const trimmed = trimSpace(value);
    if (at < 0) {
      ArrayPush(flat, name, trimmed, wanted);
    } else {
      flat[at + 1] = trimmed;
      flat[at + 2] = wanted;
    }
    writeDeclarations(state, flat, method);
  }

  function removeProperty(state, name) {
    const flat = declarationsOf(state);
    const at = findDeclaration(flat, name);
    if (at < 0) {
      return "";
    }
    const old = flat[at + 1];
    ArraySplice(flat, at, 3);
    writeDeclarations(state, flat, "removeProperty");
    return old;
  }

  function trimSpace(text) {
    let start = 0;
    let end = text.length;
    while (start < end && isSpace(CharCodeAt(text, start))) {
      start++;
    }
    while (end > start && isSpace(CharCodeAt(text, end - 1))) {
      end--;
    }
    return StringSlice(text, start, end);
  }

  // The name the cascade knows a property by, or null, asked once a
  // name.
  const canonicalNames = new $Map();
  function canonicalProperty(name) {
    let canonical = MapGet(canonicalNames, name);
    if (canonical === undefined) {
      canonical = native.cssCanonical(name);
      MapSet(canonicalNames, name, canonical);
    }
    return canonical;
  }

  // A name a block's declarations are looked up by: the canonical one,
  // or, for a property the cascade does not know, the name lowercased,
  // which matches none.
  function lookupName(name) {
    const canonical = canonicalProperty(name);
    return canonical === null ? propertyName(name) : canonical;
  }

  // The property a name in camel case stands for (CSSOM's IDL
  // attributes): `backgroundColor` for `background-color`, `webkitFoo`
  // and `WebkitFoo` for `-webkit-foo`; a dashed name for itself.
  function dashed(key) {
    if (StringIndexOf(key, "-") >= 0) {
      return ToLowerCase(key);
    }
    let name = "";
    for (let i = 0; i < key.length; i++) {
      const c = CharCodeAt(key, i);
      if (c >= 65 && c <= 90) {
        name += "-" + FromCharCode(c + 32);
      } else {
        name += key[i];
      }
    }
    if (StringIndexOf(name, "webkit-") === 0) {
      return "-" + name;
    }
    return name;
  }

  // The indexed, camel-cased and dashed properties of a declaration
  // block: its properties' names by index, and each property the cascade
  // knows by name.
  const styleHandlers = {
    get(target, key, receiver) {
      const state = WeakMapGet(declarationBlocks, target);
      const i = arrayIndex(key);
      if (i >= 0) {
        if (state.computed) {
          const names = computedProperties();
          return i < names.length ? names[i] : undefined;
        }
        const flat = declarationsOf(state);
        return i * 3 < flat.length ? flat[i * 3] : undefined;
      }
      if (typeof key !== "string" || ReflectHas(target, key)) {
        return ReflectGet(target, key, receiver);
      }
      const name = canonicalProperty(dashed(key));
      if (name === null) {
        return ReflectGet(target, key, receiver);
      }
      if (state.computed) {
        return computedValue(state, name);
      }
      const flat = declarationsOf(state);
      const at = findDeclaration(flat, name);
      return at < 0 ? "" : flat[at + 1];
    },
    set(target, key, value, receiver) {
      if (typeof key === "string" && !ReflectHas(target, key) && arrayIndex(key) < 0) {
        const name = dashed(key);
        if (canonicalProperty(name) !== null) {
          const state = WeakMapGet(declarationBlocks, target);
          if (state.computed) {
            throw new $DOMException(
              "Failed to set the '" + key + "' property on 'CSSStyleDeclaration': These " +
                "styles are computed, and therefore the '" + name + "' property is read-only.",
              "NoModificationAllowedError",
            );
          }
          setProperty(state, name, strOrEmpty(value), "", key);
          return true;
        }
      }
      return ReflectSet(target, key, value, receiver);
    },
    has(target, key) {
      if (typeof key === "string" && !ReflectHas(target, key) && arrayIndex(key) < 0) {
        return canonicalProperty(dashed(key)) !== null;
      }
      return ReflectHas(target, key);
    },
  };

  // ---- DOMStringMap (HTML section 3.2.6.6) ------------------------------

  const stringMaps = new $WeakMap();

  // A map holds its element, as long as it lives.
  const mapOwners = new $WeakMap();

  function stringMap(owner, handle) {
    const target = new DOMStringMap(KEY);
    WeakMapSet(stringMaps, target, handle);
    WeakMapSet(mapOwners, target, owner);
    return new $Proxy(target, stringMapHandlers);
  }

  class DOMStringMap {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("DOMStringMap");
      }
    }
  }
  expose(DOMStringMap, "DOMStringMap");

  // The `data-` attribute a name stands for: each ASCII capital made a
  // hyphen and its lowercase.
  function dataAttribute(name) {
    let attribute = "data-";
    for (let i = 0; i < name.length; i++) {
      const c = CharCodeAt(name, i);
      attribute += c >= 65 && c <= 90 ? "-" + FromCharCode(c + 32) : name[i];
    }
    return attribute;
  }

  // The name a `data-` attribute stands for, or null for one with an
  // ASCII capital: each hyphen before a lowercase letter dropped and the
  // letter made capital.
  function dataName(attribute) {
    if (StringIndexOf(attribute, "data-") !== 0) {
      return null;
    }
    let name = "";
    for (let i = 5; i < attribute.length; i++) {
      const c = CharCodeAt(attribute, i);
      if (c >= 65 && c <= 90) {
        return null;
      }
      const next = i + 1 < attribute.length ? CharCodeAt(attribute, i + 1) : 0;
      if (c === 45 && next >= 97 && next <= 122) {
        name += FromCharCode(next - 32);
        i++;
      } else {
        name += attribute[i];
      }
    }
    return name;
  }

  const stringMapHandlers = {
    get(target, key, receiver) {
      if (typeof key !== "string" || ReflectHas(target, key)) {
        return ReflectGet(target, key, receiver);
      }
      const value = native.attribute(WeakMapGet(stringMaps, target), dataAttribute(key));
      return value === null ? undefined : value;
    },
    set(target, key, value, receiver) {
      if (typeof key !== "string") {
        return ReflectSet(target, key, value, receiver);
      }
      for (let i = 0; i + 1 < key.length; i++) {
        const next = CharCodeAt(key, i + 1);
        if (key[i] === "-" && next >= 97 && next <= 122) {
          throw new $DOMException(
            "Failed to set a named property '" + key + "' on 'DOMStringMap': '" + key +
              "' is not a valid property name.",
            "SyntaxError",
          );
        }
      }
      const attribute = dataAttribute(key);
      if (!validAttributeName(attribute)) {
        throw badAttributeName(attribute, "setAttribute");
      }
      setAttribute(WeakMapGet(stringMaps, target), attribute, str(value), "setAttribute");
      return true;
    },
    deleteProperty(target, key) {
      if (typeof key !== "string") {
        return ReflectDeleteProperty(target, key);
      }
      native.removeAttribute(WeakMapGet(stringMaps, target), dataAttribute(key));
      return true;
    },
    has(target, key) {
      if (typeof key !== "string") {
        return ReflectHas(target, key);
      }
      return native.attribute(WeakMapGet(stringMaps, target), dataAttribute(key)) !== null;
    },
    ownKeys(target) {
      const names = native.attributeNames(WeakMapGet(stringMaps, target));
      const keys = [];
      for (let i = 0; i < names.length; i++) {
        const name = dataName(names[i]);
        if (name !== null) {
          ArrayPush(keys, name);
        }
      }
      return keys;
    },
    getOwnPropertyDescriptor(target, key) {
      if (typeof key !== "string") {
        return ReflectGetOwnPropertyDescriptor(target, key);
      }
      const value = native.attribute(WeakMapGet(stringMaps, target), dataAttribute(key));
      if (value === null) {
        return undefined;
      }
      return { value, writable: true, enumerable: true, configurable: true };
    },
  };

  // HTML's element interfaces (HTML section 3.2.2 and each element's
  // definition): the interface each tag's elements are objects of.
  const interfaces = new $Map();
  function element(name, tags, base) {
    const parent = base || HTMLElement;
    const cls = {
      [name]: class extends parent {
        constructor(key, handle, kind, tag) {
          if (key !== KEY) {
            throw illegalConstructor(name);
          }
          super(key, handle, kind, tag);
        }
      },
    }[name];
    expose(cls, name);
    for (let i = 0; i < tags.length; i++) {
      MapSet(interfaces, tags[i], cls);
    }
    return cls;
  }
  const HTMLMediaElement = element("HTMLMediaElement", []);
  const ELEMENTS = [
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
  ];
  for (let i = 0; i < ELEMENTS.length; i++) {
    element(ELEMENTS[i][0], ELEMENTS[i][1], ELEMENTS[i][2]);
  }
  const HTMLUnknownElement = element("HTMLUnknownElement", []);
  // A script element's members (HTML section 4.12.1): its text, and
  // `async`, which reads its "force async" flag as well as the attribute
  // and clears the flag when set.
  include(
    {
      get text() {
        return native.childText(self(this));
      },

      set text(value) {
        const handle = self(this);
        const text = str(value);
        try {
          native.setTextContent(handle, text);
        } catch (error) {
          throw failedSet(error, "text", "HTMLScriptElement");
        }
      },

      get async() {
        const handle = self(this);
        return native.forceAsync(handle) || native.attribute(handle, "async") !== null;
      },

      set async(value) {
        const handle = self(this);
        native.setForceAsync(handle, false);
        if (value) {
          setAttribute(handle, "async", "", "async");
        } else {
          native.removeAttribute(handle, "async");
        }
      },
    },
    [MapGet(interfaces, "script").prototype],
  );

  // ---- Reflected attributes (HTML section 2.6.1) ------------------------

  // HTML's rules for parsing integers: leading white space, a sign, and
  // the digits; null when there are none.
  function parseInteger(text, signed) {
    let i = 0;
    while (i < text.length && isSpace(CharCodeAt(text, i))) {
      i++;
    }
    let sign = 1;
    if (i < text.length && (text[i] === "-" || text[i] === "+")) {
      if (text[i] === "-") {
        if (!signed) {
          return null;
        }
        sign = -1;
      }
      i++;
    }
    let value = 0;
    let digits = 0;
    while (i < text.length) {
      const c = CharCodeAt(text, i);
      if (c < 48 || c > 57) {
        break;
      }
      value = value * 10 + (c - 48);
      digits++;
      i++;
    }
    return digits === 0 ? null : sign * value;
  }

  // Defines `property` on `prototype` reflecting the content attribute
  // `attribute`, of `kind`: a string, a URL resolved against the base
  // URL, a boolean, an unsigned or signed number with `fallback`, or the
  // enumerated `dir`.
  function reflect(prototype, property, attribute, kind, fallback) {
    defineProperty(prototype, property, {
      get() {
        const value = native.attribute(self(this), attribute);
        switch (kind) {
          case "boolean":
            return value !== null;
          case "url": {
            if (value === null) {
              if (fallback === "document") {
                const url = native.documentUrl();
                return url === null ? "" : url;
              }
              return "";
            }
            const url = native.resolveUrl(value);
            return url === null ? value : url;
          }
          case "unsigned":
          case "long": {
            const n = value === null ? null : parseInteger(value, kind === "long");
            if (n === null || n > 2147483647 || n < -2147483648) {
              return fallback;
            }
            return n;
          }
          case "dir": {
            const lower = value === null ? "" : ToLowerCase(value);
            return lower === "ltr" || lower === "rtl" || lower === "auto" ? lower : "";
          }
          default:
            return value === null ? "" : value;
        }
      },
      set(value) {
        const handle = self(this);
        switch (kind) {
          case "boolean":
            if (value) {
              setAttribute(handle, attribute, "", property);
            } else {
              native.removeAttribute(handle, attribute);
            }
            return;
          case "unsigned": {
            const n = value >>> 0;
            setAttribute(handle, attribute, $String(n <= 2147483647 ? n : fallback), property);
            return;
          }
          case "long":
            setAttribute(handle, attribute, $String(value | 0), property);
            return;
          default:
            setAttribute(handle, attribute, str(value), property);
        }
      },
      enumerable: true,
      configurable: true,
    });
  }

  // What each interface reflects: [property, attribute, kind, fallback].
  const REFLECTED = [
    ["HTMLElement", [
      ["title", "title", "string"],
      ["lang", "lang", "string"],
      ["dir", "dir", "dir"],
      ["hidden", "hidden", "boolean"],
      ["inert", "inert", "boolean"],
      ["accessKey", "accesskey", "string"],
    ]],
    ["HTMLAnchorElement", [
      ["href", "href", "url"],
      ["target", "target", "string"],
      ["download", "download", "string"],
      ["rel", "rel", "string"],
      ["hreflang", "hreflang", "string"],
      ["type", "type", "string"],
    ]],
    ["HTMLAreaElement", [
      ["href", "href", "url"],
      ["alt", "alt", "string"],
      ["target", "target", "string"],
      ["rel", "rel", "string"],
      ["coords", "coords", "string"],
      ["shape", "shape", "string"],
    ]],
    ["HTMLLinkElement", [
      ["href", "href", "url"],
      ["rel", "rel", "string"],
      ["media", "media", "string"],
      ["type", "type", "string"],
      ["hreflang", "hreflang", "string"],
      ["disabled", "disabled", "boolean"],
    ]],
    ["HTMLImageElement", [
      ["src", "src", "url"],
      ["alt", "alt", "string"],
      ["srcset", "srcset", "string"],
      ["sizes", "sizes", "string"],
      ["useMap", "usemap", "string"],
      ["isMap", "ismap", "boolean"],
      ["width", "width", "unsigned", 0],
      ["height", "height", "unsigned", 0],
    ]],
    ["HTMLScriptElement", [
      ["src", "src", "url"],
      ["type", "type", "string"],
      ["charset", "charset", "string"],
      ["defer", "defer", "boolean"],
      ["noModule", "nomodule", "boolean"],
    ]],
    ["HTMLIFrameElement", [
      ["src", "src", "url"],
      ["name", "name", "string"],
      ["width", "width", "string"],
      ["height", "height", "string"],
    ]],
    ["HTMLMediaElement", [
      ["src", "src", "url"],
      ["autoplay", "autoplay", "boolean"],
      ["loop", "loop", "boolean"],
      ["controls", "controls", "boolean"],
    ]],
    ["HTMLVideoElement", [
      ["poster", "poster", "url"],
      ["width", "width", "unsigned", 0],
      ["height", "height", "unsigned", 0],
    ]],
    ["HTMLCanvasElement", [
      ["width", "width", "unsigned", 300],
      ["height", "height", "unsigned", 150],
    ]],
    ["HTMLFormElement", [
      ["action", "action", "url", "document"],
      ["name", "name", "string"],
      ["target", "target", "string"],
      ["noValidate", "novalidate", "boolean"],
    ]],
    ["HTMLInputElement", [
      ["name", "name", "string"],
      ["placeholder", "placeholder", "string"],
      ["defaultValue", "value", "string"],
      ["disabled", "disabled", "boolean"],
      ["required", "required", "boolean"],
      ["readOnly", "readonly", "boolean"],
      ["multiple", "multiple", "boolean"],
      ["autofocus", "autofocus", "boolean"],
    ]],
    ["HTMLButtonElement", [
      ["name", "name", "string"],
      ["value", "value", "string"],
      ["disabled", "disabled", "boolean"],
    ]],
    ["HTMLSelectElement", [
      ["name", "name", "string"],
      ["disabled", "disabled", "boolean"],
      ["required", "required", "boolean"],
      ["multiple", "multiple", "boolean"],
    ]],
    ["HTMLTextAreaElement", [
      ["name", "name", "string"],
      ["placeholder", "placeholder", "string"],
      ["disabled", "disabled", "boolean"],
      ["required", "required", "boolean"],
      ["readOnly", "readonly", "boolean"],
    ]],
    ["HTMLOptionElement", [
      ["disabled", "disabled", "boolean"],
      ["label", "label", "string"],
    ]],
    ["HTMLMetaElement", [
      ["name", "name", "string"],
      ["content", "content", "string"],
      ["httpEquiv", "http-equiv", "string"],
    ]],
    ["HTMLStyleElement", [
      ["media", "media", "string"],
    ]],
    ["HTMLLabelElement", [
      ["htmlFor", "for", "string"],
    ]],
    ["HTMLOListElement", [
      ["start", "start", "long", 1],
      ["reversed", "reversed", "boolean"],
      ["type", "type", "string"],
    ]],
    ["HTMLLIElement", [
      ["value", "value", "long", 0],
    ]],
    ["HTMLTableCellElement", [
      ["colSpan", "colspan", "unsigned", 1],
      ["rowSpan", "rowspan", "unsigned", 1],
    ]],
    ["HTMLTimeElement", [
      ["dateTime", "datetime", "string"],
    ]],
    ["HTMLDataElement", [
      ["value", "value", "string"],
    ]],
    ["HTMLQuoteElement", [
      ["cite", "cite", "url"],
    ]],
    ["HTMLModElement", [
      ["cite", "cite", "url"],
      ["dateTime", "datetime", "string"],
    ]],
  ];
  for (let i = 0; i < REFLECTED.length; i++) {
    const iface = global[REFLECTED[i][0]];
    const members = REFLECTED[i][1];
    for (let j = 0; j < members.length; j++) {
      const m = members[j];
      reflect(iface.prototype, m[0], m[1], m[2], m[3]);
    }
  }

  // The tags HTML defines whose elements are plain HTMLElements.
  const PLAIN = [
    "abbr", "acronym", "address", "article", "aside", "b", "basefont", "bdi", "bdo", "big",
    "center", "cite", "code", "dd", "dfn", "dt", "em", "figcaption", "figure", "footer",
    "header", "hgroup", "i", "kbd", "main", "mark", "nav", "nobr", "noembed", "noframes",
    "noscript", "plaintext", "rb", "rp", "rt", "rtc", "ruby", "s", "samp", "search",
    "section", "small", "strike", "strong", "sub", "summary", "sup", "tt", "u", "var", "wbr",
  ];
  for (let i = 0; i < PLAIN.length; i++) {
    MapSet(interfaces, PLAIN[i], HTMLElement);
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

  // ---- Location (HTML section 7.2.4) ---------------------------------

  // A URL to navigate to, after the run; or the SyntaxError for one that
  // does not parse, in the reference browser's words.
  function navigate(url, replace, member, setter) {
    try {
      native.navigate(url, replace);
    } catch (error) {
      throw setter ? failedSet(error, member, "Location") : failed(error, member, "Location");
    }
  }

  // The current entry's URL in parts: href, origin, protocol, host,
  // hostname, port, pathname, search, hash.
  function locationPart(i) {
    return native.location()[i];
  }

  class Location {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("Location");
      }
    }

    get href() {
      return locationPart(0);
    }

    set href(value) {
      navigate(str(value), false, "href", true);
    }

    get origin() {
      return locationPart(1);
    }

    get protocol() {
      return locationPart(2);
    }

    get host() {
      return locationPart(3);
    }

    get hostname() {
      return locationPart(4);
    }

    get port() {
      return locationPart(5);
    }

    get pathname() {
      return locationPart(6);
    }

    get search() {
      return locationPart(7);
    }

    get hash() {
      return locationPart(8);
    }

    set hash(value) {
      let fragment = str(value);
      if (fragment !== "" && fragment[0] === "#") {
        fragment = StringSlice(fragment, 1);
      }
      const href = locationPart(0);
      const at = StringIndexOf(href, "#");
      const base = at < 0 ? href : StringSlice(href, 0, at);
      navigate(base + "#" + fragment, false, "hash", true);
    }

    assign(url) {
      need(arguments.length, 1, "assign", "Location");
      navigate(str(url), false, "assign", false);
    }

    replace(url) {
      need(arguments.length, 1, "replace", "Location");
      navigate(str(url), true, "replace", false);
    }

    reload() {
      native.reload();
    }

    toString() {
      return locationPart(0);
    }
  }
  expose(Location, "Location");
  const location = new Location(KEY);

  // ---- Navigator (HTML section 8.10.1), by D12 -------------------------

  // What the host says of the user agent: its string, the platform and
  // the compatibility mode, asked once.
  let agent = null;
  function agentInfo() {
    if (agent === null) {
      agent = native.navigator();
    }
    return agent;
  }

  class PluginArray {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("PluginArray");
      }
    }

    get length() {
      return 0;
    }

    item(index) {
      return null;
    }

    namedItem(name) {
      return null;
    }

    refresh() {}
  }
  expose(PluginArray, "PluginArray");
  defineProperty(PluginArray.prototype, iteratorSymbol, {
    value: ArrayValues,
    writable: true,
    configurable: true,
  });

  class MimeTypeArray {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("MimeTypeArray");
      }
    }

    get length() {
      return 0;
    }

    item(index) {
      return null;
    }

    namedItem(name) {
      return null;
    }
  }
  expose(MimeTypeArray, "MimeTypeArray");
  defineProperty(MimeTypeArray.prototype, iteratorSymbol, {
    value: ArrayValues,
    writable: true,
    configurable: true,
  });

  const plugins = new PluginArray(KEY);
  const mimeTypes = new MimeTypeArray(KEY);
  const languages = Object.freeze(["en-US", "en"]);

  class Navigator {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("Navigator");
      }
    }

    get userAgent() {
      return agentInfo()[0];
    }

    get appCodeName() {
      return "Mozilla";
    }

    get appName() {
      return "Netscape";
    }

    // HTML's appVersion: what follows "Mozilla/", whole in Chrome's mode;
    // in Gecko's, its first part closed.
    get appVersion() {
      const ua = agentInfo()[0];
      if (StringIndexOf(ua, "Mozilla/5.0 (") !== 0) {
        return "";
      }
      const trail = StringSlice(ua, 8);
      if (agentInfo()[2] === "chrome") {
        return trail;
      }
      if (StringIndexOf(trail, "5.0 (Windows") === 0) {
        return "5.0 (Windows)";
      }
      const semicolon = StringIndexOf(trail, ";");
      return semicolon < 0 ? trail : StringSlice(trail, 0, semicolon) + ")";
    }

    get platform() {
      return agentInfo()[1];
    }

    get product() {
      return "Gecko";
    }

    get productSub() {
      return agentInfo()[2] === "chrome" ? "20030107" : "20100101";
    }

    get vendor() {
      return agentInfo()[2] === "chrome" ? "Google Inc." : "";
    }

    get vendorSub() {
      return "";
    }

    get language() {
      return "en-US";
    }

    get languages() {
      return languages;
    }

    get onLine() {
      return true;
    }

    get cookieEnabled() {
      return false;
    }

    get webdriver() {
      return false;
    }

    get pdfViewerEnabled() {
      return false;
    }

    get plugins() {
      return plugins;
    }

    get mimeTypes() {
      return mimeTypes;
    }

    javaEnabled() {
      return false;
    }
  }
  expose(Navigator, "Navigator");
  const navigator = new Navigator(KEY);

  // ---- Screen (CSSOM View section 4.3) -----------------------------------

  // The screen is reported as the window (D12): its real size tells one
  // machine from another and helps no page.
  class Screen {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("Screen");
      }
    }

    get width() {
      return native.viewport()[0];
    }

    get height() {
      return native.viewport()[1];
    }

    get availWidth() {
      return native.viewport()[0];
    }

    get availHeight() {
      return native.viewport()[1];
    }

    get colorDepth() {
      return 24;
    }

    get pixelDepth() {
      return 24;
    }
  }
  expose(Screen, "Screen");
  const screen = new Screen(KEY);

  // A pseudo-element named as getComputedStyle takes it: `::before`,
  // `:before` (the older one-colon names), or nothing.
  function pseudoName(value) {
    if (value === undefined || value === null) {
      return "";
    }
    const text = ToLowerCase(str(value));
    if (text === "") {
      return "";
    }
    const name = StringIndexOf(text, "::") === 0 ? StringSlice(text, 2) :
      StringIndexOf(text, ":") === 0 ? StringSlice(text, 1) : null;
    if (name === null) {
      return "invalid";
    }
    return name;
  }

  // ---- The window ----------------------------------------------------

  class Window extends EventTarget {
    constructor() {
      throw illegalConstructor("Window");
    }

    get location() {
      return location;
    }

    set location(value) {
      location.href = value;
    }

    get navigator() {
      return navigator;
    }

    get screen() {
      return screen;
    }

    get innerWidth() {
      return native.viewport()[0];
    }

    get innerHeight() {
      return native.viewport()[1];
    }

    get outerWidth() {
      return native.viewport()[0];
    }

    get outerHeight() {
      return native.viewport()[1];
    }

    get devicePixelRatio() {
      return native.viewport()[2];
    }

    // No frames yet: the window is its own top and parent.
    get top() {
      return global;
    }

    get parent() {
      return global;
    }

    get frames() {
      return global;
    }

    get length() {
      return 0;
    }

    get opener() {
      return null;
    }

    get closed() {
      return false;
    }

    get performance() {
      return performance;
    }

    getComputedStyle(element, pseudoElement) {
      need(arguments.length, 1, "getComputedStyle", "Window");
      if (!isNode(element) || kindOf(element) !== 1) {
        throw new $TypeError(
          prefix("getComputedStyle", "Window") + "parameter 1 is not of type 'Element'.",
        );
      }
      return computedStyle(element, handleOf(element), pseudoName(pseudoElement));
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

  // ---- Timers (M26, HTML section 8.6) ------------------------------------

  // Each timer's callback (a function, or a string compiled when it
  // runs), its arguments and its delay, by id; the host keeps when each
  // is due, and runs it as a task through `runTimer`. Timeouts and
  // intervals share the ids.
  const timers = new $Map();
  let lastTimer = 0;

  // A Web IDL `long`.
  function long(value) {
    return $Number(value) | 0;
  }

  function startTimer(handler, timeout, args, repeat) {
    const callback = typeof handler === "function" ? handler : str(handler);
    let id = lastTimer;
    do {
      id = id >= 2147483647 ? 1 : id + 1;
    } while (MapHas(timers, id));
    lastTimer = id;
    const delay = long(timeout);
    native.setTimer(id, delay);
    MapSet(timers, id, { callback, args, repeat, delay });
    return id;
  }

  function stopTimer(handle) {
    const id = long(handle);
    if (MapHas(timers, id)) {
      MapDelete(timers, id);
      native.clearTimer(id);
    }
  }

  // Runs timer `id` (the task the host queued when it was due): its
  // callback with `this` the window and its arguments, or its string as
  // a classic script; an interval is set again once its callback has
  // run, unless the callback cleared it.
  function runTimer(id) {
    const timer = MapGet(timers, id);
    if (timer === undefined) {
      return;
    }
    if (!timer.repeat) {
      MapDelete(timers, id);
    }
    try {
      if (typeof timer.callback === "function") {
        ReflectApply(timer.callback, global, timer.args);
      } else {
        $eval(timer.callback);
      }
    } finally {
      if (timer.repeat && MapGet(timers, id) === timer) {
        native.setTimer(id, timer.delay);
      }
    }
  }

  const timerFunctions = {
    setTimeout(handler, timeout = 0) {
      need(arguments.length, 1, "setTimeout", "Window");
      return startTimer(handler, timeout, ArraySlice(arguments, 2), false);
    },
    setInterval(handler, timeout = 0) {
      need(arguments.length, 1, "setInterval", "Window");
      return startTimer(handler, timeout, ArraySlice(arguments, 2), true);
    },
    clearTimeout(id = 0) {
      stopTimer(id);
    },
    clearInterval(id = 0) {
      stopTimer(id);
    },
  };
  const timerNames = getOwnPropertyNames(timerFunctions);
  for (let i = 0; i < timerNames.length; i++) {
    defineProperty(global, timerNames[i], {
      value: timerFunctions[timerNames[i]],
      writable: true,
      enumerable: true,
      configurable: true,
    });
  }

  // ---- performance (High Resolution Time) --------------------------------

  // The document's clock: milliseconds since its time origin, rounded
  // by the host to a tenth of one.
  class Performance extends EventTarget {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("Performance");
      }
      super();
    }

    now() {
      return native.now();
    }

    get timeOrigin() {
      return native.timeOrigin();
    }

    toJSON() {
      return { timeOrigin: native.timeOrigin() };
    }
  }
  expose(Performance, "Performance");
  const performance = new Performance(KEY);

  // The entry points the realm's tasks call (M26).
  return {
    timer: runTimer,
  };
});
