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
  const $Error = Error;
  const ToBase64 = uncurry(Uint8Array.prototype.toBase64);
  const ObjectFreeze = Object.freeze;
  const FunctionBind = uncurry(Function.prototype.bind);
  const BigIntAsIntN = BigInt.asIntN;
  const SetFromBase64 = uncurry(Uint8Array.prototype.setFromBase64);
  const PromiseThen = uncurry(Promise.prototype.then);
  const $DOMException = DOMException;
  const $Map = Map;
  const $WeakMap = WeakMap;
  const $WeakRef = WeakRef;
  const $Proxy = Proxy;
  const $FinalizationRegistry = FinalizationRegistry;
  const ReflectApply = Reflect.apply;
  const ReflectConstruct = Reflect.construct;
  const $EvalError = EvalError;
  // The originals of what compiles code from strings (M33.1), which
  // only this closure holds once they are guarded.
  const OriginalEval = global.eval;
  const OriginalFunction = Function;
  const ReflectGet = Reflect.get;
  const ReflectSet = Reflect.set;
  const ReflectHas = Reflect.has;
  const ReflectOwnKeys = Reflect.ownKeys;
  const ReflectDefineProperty = Reflect.defineProperty;
  const ReflectDeleteProperty = Reflect.deleteProperty;
  const ReflectGetOwnPropertyDescriptor = Reflect.getOwnPropertyDescriptor;
  const ReflectGetPrototypeOf = Reflect.getPrototypeOf;
  const ArraySplice = uncurry(Array.prototype.splice);
  const StringIndexOf = uncurry(String.prototype.indexOf);
  const FromCharCode = String.fromCharCode;
  const MapGet = uncurry(Map.prototype.get);
  const MapSet = uncurry(Map.prototype.set);
  const MapDelete = uncurry(Map.prototype.delete);
  const MapHas = uncurry(Map.prototype.has);
  const ArraySlice = uncurry(Array.prototype.slice);
  const ObjectCreate = Object.create;
  const $WeakSet = WeakSet;
  const WeakSetHas = uncurry(WeakSet.prototype.has);
  const WeakSetAdd = uncurry(WeakSet.prototype.add);
  const MapClear = uncurry(Map.prototype.clear);
  const MapForEach = uncurry(Map.prototype.forEach);
  const GeneratorNext = uncurry(Object.getPrototypeOf(function* () {}).prototype.next);
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

  // ECMAScript's ToString, as Web IDL's conversions call it: a Symbol
  // throws, where String() would describe it.
  function toString(value) {
    if (typeof value === "symbol") {
      throw new $TypeError("Cannot convert a Symbol value to a string");
    }
    return $String(value);
  }

  // A DOMString: the value as a string, with any lone surrogate made
  // U+FFFD, which the host's strings cannot hold.
  function str(value) {
    return ToWellFormed(toString(value));
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

  // The prototypes of the interfaces the bindings expose: an object
  // with one in its chain is a platform object, which a structured
  // clone serializes only if its interface says how (M31).
  const platformPrototypes = new $WeakSet();

  // The kind of realm (M31): a document's window, or a dedicated
  // worker's global scope, which has only the interfaces Web IDL
  // exposes in a dedicated worker, and the name its creator gave it.
  const WORKER = native.realmKind === "worker";
  const WORKER_NAME = WORKER ? native.workerName : "";

  // The interface of the members the window and the workers share,
  // which they name in their errors, and the global's own.
  const GLOBAL_INTERFACE = WORKER ? "WorkerGlobalScope" : "Window";
  const GLOBAL_CLASS = WORKER ? "DedicatedWorkerGlobalScope" : "Window";

  // Of the interfaces the bindings make, those a dedicated worker has
  // too (true) or alone ("worker"); the others are the window's.
  const WORKER_INTERFACES = {
    __proto__: null,
    AbortController: true, AbortSignal: true, Blob: true, CanvasGradient: true,
    CanvasPattern: true, CustomEvent: true, DOMMatrix: true, DOMMatrixReadOnly: true,
    DOMPoint: true, DOMPointReadOnly: true, DedicatedWorkerGlobalScope: "worker",
    ErrorEvent: true, Event: true, EventTarget: true, File: true, FormData: true,
    Headers: true, ImageBitmap: true, ImageData: true, MessageEvent: true,
    OffscreenCanvas: true, OffscreenCanvasRenderingContext2D: true, Path2D: true,
    Performance: true, ProgressEvent: true, QuotaExceededError: true, Request: true,
    Response: true, TextMetrics: true, URLSearchParams: true, Worker: true,
    MessageChannel: true, MessagePort: true,
    WorkerGlobalScope: "worker", WorkerLocation: "worker", WorkerNavigator: "worker",
    XMLHttpRequest: true, XMLHttpRequestEventTarget: true, XMLHttpRequestUpload: true,
  };

  // Every interface the bindings make, by name, exposed or not.
  const madeInterfaces = { __proto__: null };

  // Defines a property of the window's that a worker's global has not.
  function windowOnly(name, desc) {
    if (!WORKER) {
      defineProperty(global, name, desc);
    }
  }

  // Makes an interface look as Web IDL says: its members enumerable,
  // its prototype tagged with its name, its constructor a global
  // property that is writable and configurable but not enumerable.
  function expose(cls, name) {
    const proto = cls.prototype;
    WeakSetAdd(platformPrototypes, proto);
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
    madeInterfaces[name] = cls;
    const where = WORKER_INTERFACES[name];
    if (WORKER ? where === undefined : where === "worker") {
      return cls;
    }
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
    const object = make(handle, answer[0], answer[2]);
    MapSet(objects, handle, {
      ref: new $WeakRef(object),
      strong: answer[1] === "" ? null : object,
    });
    Register(registry, object, handle);
    for (let i = 3; i < answer.length; i++) {
      attributeHandler(object, answer[i], true);
    }
    return object;
  }

  // A new object for a node, of the interface its name and namespace
  // say. The host hears it from `wrap`, and holds the node until it is
  // released.
  function make(handle, name, ns) {
    if (name === "#text") {
      return new Text(KEY, handle, 3, name);
    }
    if (name === "#document") {
      return new HTMLDocument(KEY, handle, 9, name);
    }
    const object = new (elementInterface(name, ns))(KEY, handle, 1, name);
    setNsOf(object, ns === "" ? null : ns);
    return object;
  }

  // The namespaces (Infra section 8) and a qualified name's parts.
  const HTML_NS = "http://www.w3.org/1999/xhtml";
  const SVG_NS = "http://www.w3.org/2000/svg";
  const MATHML_NS = "http://www.w3.org/1998/Math/MathML";
  const XLINK_NS = "http://www.w3.org/1999/xlink";
  const XML_NS = "http://www.w3.org/XML/1998/namespace";
  const XMLNS_NS = "http://www.w3.org/2000/xmlns/";
  function localPart(qualified) {
    const at = StringIndexOf(qualified, ":");
    return at < 0 ? qualified : StringSlice(qualified, at + 1);
  }
  function prefixPart(qualified) {
    const at = StringIndexOf(qualified, ":");
    return at < 0 ? null : StringSlice(qualified, 0, at);
  }

  // ---- Events (DOM section 2, M26) ----------------------------------------

  // An EventTarget keeps its listeners (DOM section 2.7), each a type, a
  // callback, its capture, passive and once flags, and the removed flag
  // a dispatch under way reads; and its event handlers (HTML section
  // 8.1.8), by event type, made when first set. The window's are kept
  // apart: the global object is no EventTarget made by a constructor.
  const windowListeners = [];
  const windowHandlers = new $Map();
  let listenersOf;
  let handlersOf;
  let isTarget;

  class EventTarget {
    #listeners = [];
    #handlers = null;

    static {
      // A method called with no `this`, as a global function, is the
      // window's (Web IDL section 3.7.5).
      listenersOf = (target) => {
        if (typeof target === "object" && target !== null && #listeners in target) {
          return target.#listeners;
        }
        if (target === global || target === undefined || target === null) {
          return windowListeners;
        }
        throw illegalInvocation();
      };
      handlersOf = (target) => {
        if (target === global) {
          return windowHandlers;
        }
        if (target.#handlers === null) {
          target.#handlers = new $Map();
        }
        return target.#handlers;
      };
      isTarget = (value) =>
        value === global || (typeof value === "object" && value !== null && #listeners in value);
    }

    addEventListener(type, callback, options) {
      const listeners = listenersOf(this);
      need(arguments.length, 2, "addEventListener", "EventTarget");
      const name = str(type);
      checkCallback(callback, "addEventListener");
      const flags = addOptions(options);
      addListener(this === undefined || this === null ? global : this, listeners, name, callback, flags);
    }

    removeEventListener(type, callback, options) {
      const listeners = listenersOf(this);
      need(arguments.length, 2, "removeEventListener", "EventTarget");
      const name = str(type);
      checkCallback(callback, "removeEventListener");
      const capture = captureOf(options);
      for (let i = 0; i < listeners.length; i++) {
        const l = listeners[i];
        if (l.type === name && l.callback === callback && l.capture === capture) {
          removeListener(listeners, l);
          return;
        }
      }
    }

    dispatchEvent(event) {
      listenersOf(this);
      need(arguments.length, 1, "dispatchEvent", "EventTarget");
      if (!isEvent(event)) {
        throw new $TypeError(
          prefix("dispatchEvent", "EventTarget") + "parameter 1 is not of type 'Event'.",
        );
      }
      const s = eventState(event);
      if (s.dispatching) {
        throw domError(
          "InvalidStateError",
          "The event is already being dispatched.",
          "dispatchEvent",
          "EventTarget",
        );
      }
      if (!s.initialized) {
        throw domError(
          "InvalidStateError",
          "The event provided is uninitialized.",
          "dispatchEvent",
          "EventTarget",
        );
      }
      s.trusted = false;
      return !drain(dispatching(this === undefined || this === null ? global : this, event));
    }
  }

  // A listener's callback must be an object, a function, or null.
  function checkCallback(callback, method) {
    if (
      callback !== null && callback !== undefined && typeof callback !== "object" &&
      typeof callback !== "function"
    ) {
      throw new $TypeError(prefix(method, "EventTarget") + "parameter 2 is not of type 'Object'.");
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

  // `addEventListener`'s options (DOM section 2.7's "flatten more"): a
  // boolean is `capture`; a dictionary's members are read in Web IDL's
  // order. `passive` is null when not given.
  function addOptions(options) {
    if (typeof options !== "object" && typeof options !== "function") {
      return { capture: !!options, passive: null, once: false, signal: null };
    }
    if (options === null) {
      return { capture: false, passive: null, once: false, signal: null };
    }
    const capture = !!options.capture;
    const once = !!options.once;
    const passive = options.passive;
    const given = options.signal;
    if (given !== undefined && !isSignal(given)) {
      throw new $TypeError(
        prefix("addEventListener", "EventTarget") +
          "Failed to read the 'signal' property from 'AddEventListenerOptions': " +
          "Failed to convert value to 'AbortSignal'.",
      );
    }
    return {
      capture,
      passive: passive === undefined ? null : !!passive,
      once,
      signal: given === undefined ? null : given,
    };
  }

  // The DOM's "add an event listener": none once its signal is aborted
  // or for a null callback, none twice; a `wheel` or touch listener of
  // the window, the document, its element or its body passive unless it
  // says otherwise; removed when its signal is aborted.
  function addListener(target, listeners, type, callback, flags) {
    if (flags.signal !== null && signalOf(flags.signal).aborted) {
      return;
    }
    if (callback === null || callback === undefined) {
      return;
    }
    for (let i = 0; i < listeners.length; i++) {
      const l = listeners[i];
      if (l.type === type && l.callback === callback && l.capture === flags.capture) {
        return;
      }
    }
    const listener = {
      type,
      callback,
      capture: flags.capture,
      passive: flags.passive === null ? defaultPassive(target, type) : flags.passive,
      once: flags.once,
      removed: false,
    };
    ArrayPush(listeners, listener);
    if (flags.signal !== null) {
      ArrayPush(signalOf(flags.signal).algorithms, () => removeListener(listeners, listener));
    }
  }

  // The DOM's "remove an event listener": its removed flag set, so that
  // a dispatch under way calls it no more.
  function removeListener(listeners, listener) {
    listener.removed = true;
    for (let i = 0; i < listeners.length; i++) {
      if (listeners[i] === listener) {
        ArraySplice(listeners, i, 1);
        return;
      }
    }
  }

  // The DOM's "default passive value".
  function defaultPassive(target, type) {
    if (type !== "touchstart" && type !== "touchmove" && type !== "wheel" && type !== "mousewheel") {
      return false;
    }
    if (target === global) {
      return true;
    }
    if (!isNode(target)) {
      return false;
    }
    if (kindOf(target) === 9) {
      return true;
    }
    const handle = handleOf(target);
    const doc = native.document();
    return handle === documentElement(doc) || handle === childOfHtml(doc, "body");
  }

  // An event's state (DOM section 2.2), out of a script's reach: its type
  // and flags, what a dispatch sets, the interfaces it is an object of
  // (bits of `kinds`), and its dictionary's members (`fields`).
  let eventState;
  let isEvent;
  const EVENT = 1;
  const UI = 2;
  const FOCUS = 4;
  const MOUSE = 8;
  const POINTER = 16;
  const WHEEL = 32;
  const KEYBOARD = 64;
  const INPUT = 128;
  const CUSTOM = 256;
  const SUBMIT = 512;
  const ERROR = 1024;
  const PAGE_TRANSITION = 2048;

  // The dictionaries (each member a name, a type, a default and the
  // dictionary that has it), the inherited members first, each
  // dictionary's in lexicographic order, as Web IDL reads them.
  function members(base, own, dictionary) {
    const all = [];
    for (let i = 0; i < base.length; i++) {
      ArrayPush(all, base[i]);
    }
    for (let i = 0; i < own.length; i++) {
      ArrayPush(all, [own[i][0], own[i][1], own[i][2], dictionary]);
    }
    return all;
  }
  const EVENT_INIT = members([], [
    ["bubbles", "boolean", false],
    ["cancelable", "boolean", false],
    ["composed", "boolean", false],
  ], "EventInit");
  const CUSTOM_INIT = members(EVENT_INIT, [["detail", "any", null]], "CustomEventInit");
  const UI_INIT = members(EVENT_INIT, [
    ["detail", "long", 0],
    ["view", "Window?", null],
    ["which", "unsigned long", 0],
  ], "UIEventInit");
  const FOCUS_INIT = members(UI_INIT, [["relatedTarget", "EventTarget?", null]], "FocusEventInit");
  const MODIFIERS = [
    "altKey", "ctrlKey", "metaKey", "modifierAltGraph", "modifierCapsLock", "modifierFn",
    "modifierFnLock", "modifierHyper", "modifierNumLock", "modifierScrollLock", "modifierSuper",
    "modifierSymbol", "modifierSymbolLock", "shiftKey",
  ];
  const MODIFIER_INIT = members(UI_INIT, [], "EventModifierInit");
  for (let i = 0; i < MODIFIERS.length; i++) {
    ArrayPush(MODIFIER_INIT, [MODIFIERS[i], "boolean", false, "EventModifierInit"]);
  }
  const MOUSE_INIT = members(MODIFIER_INIT, [
    ["button", "short", 0],
    ["buttons", "unsigned short", 0],
    ["clientX", "double", 0],
    ["clientY", "double", 0],
    ["movementX", "double", 0],
    ["movementY", "double", 0],
    ["relatedTarget", "EventTarget?", null],
    ["screenX", "double", 0],
    ["screenY", "double", 0],
  ], "MouseEventInit");
  const POINTER_INIT = members(MOUSE_INIT, [
    ["altitudeAngle", "double", Math.PI / 2],
    ["azimuthAngle", "double", 0],
    ["height", "double", 1],
    ["isPrimary", "boolean", false],
    ["pointerId", "long", 0],
    ["pointerType", "DOMString", ""],
    ["pressure", "float", 0],
    ["tangentialPressure", "float", 0],
    ["tiltX", "long", 0],
    ["tiltY", "long", 0],
    ["twist", "long", 0],
    ["width", "double", 1],
  ], "PointerEventInit");
  const WHEEL_INIT = members(MOUSE_INIT, [
    ["deltaMode", "unsigned long", 0],
    ["deltaX", "double", 0],
    ["deltaY", "double", 0],
    ["deltaZ", "double", 0],
  ], "WheelEventInit");
  const KEYBOARD_INIT = members(MODIFIER_INIT, [
    ["charCode", "unsigned long", 0],
    ["code", "DOMString", ""],
    ["isComposing", "boolean", false],
    ["key", "DOMString", ""],
    ["keyCode", "unsigned long", 0],
    ["location", "unsigned long", 0],
    ["repeat", "boolean", false],
  ], "KeyboardEventInit");
  const INPUT_INIT = members(UI_INIT, [
    ["data", "DOMString?", null],
    ["inputType", "DOMString", ""],
    ["isComposing", "boolean", false],
  ], "InputEventInit");
  const SUBMIT_INIT = members(EVENT_INIT, [["submitter", "HTMLElement?", null]], "SubmitEventInit");
  const ERROR_INIT = members(EVENT_INIT, [
    ["colno", "unsigned long", 0],
    ["error", "any", undefined],
    ["filename", "USVString", ""],
    ["lineno", "unsigned long", 0],
    ["message", "DOMString", ""],
  ], "ErrorEventInit");
  const PAGE_TRANSITION_INIT = members(
    EVENT_INIT,
    [["persisted", "boolean", false]],
    "PageTransitionEventInit",
  );

  // The dictionaries the bindings converted or made themselves, which a
  // constructor takes as they are.
  const convertedDictionaries = new $WeakSet();

  // Web IDL's conversion of a dictionary argument: each member read from
  // `value` in order and converted, or its default; null and undefined
  // are the empty dictionary. An object with no prototype.
  function dictionary(value, list, iface) {
    if (
      value !== undefined && value !== null && typeof value !== "object" &&
      typeof value !== "function"
    ) {
      throw new $TypeError(
        "Failed to construct '" + iface + "': The provided value is not of type '" +
          list[list.length - 1][3] + "'.",
      );
    }
    const fields = ObjectCreate(null);
    for (let i = 0; i < list.length; i++) {
      const m = list[i];
      const given = value === undefined || value === null ? undefined : value[m[0]];
      fields[m[0]] = given === undefined ? m[2] : member(given, m, iface);
    }
    WeakSetAdd(convertedDictionaries, fields);
    return fields;
  }

  // A dictionary the bindings make: the defaults, then `pairs`, names
  // and values in turn.
  function made(list, pairs) {
    const fields = ObjectCreate(null);
    for (let i = 0; i < list.length; i++) {
      fields[list[i][0]] = list[i][2];
    }
    for (let i = 0; i < pairs.length; i += 2) {
      fields[pairs[i]] = pairs[i + 1];
    }
    WeakSetAdd(convertedDictionaries, fields);
    return fields;
  }

  // A dictionary member's value converted to its type, or the TypeError
  // the reference browser throws.
  function member(value, m, iface) {
    const where = "Failed to construct '" + iface + "': Failed to read the '" + m[0] +
      "' property from '" + m[3] + "': ";
    switch (m[1]) {
      case "boolean":
        return !!value;
      case "double":
      case "float": {
        const n = $Number(value);
        if (n !== n || n === Infinity || n === -Infinity) {
          throw new $TypeError(where + "The provided " + m[1] + " value is non-finite.");
        }
        return n;
      }
      case "long":
        return $Number(value) | 0;
      case "unsigned long":
        return $Number(value) >>> 0;
      case "short":
        return ($Number(value) << 16) >> 16;
      case "unsigned short":
        return $Number(value) & 0xffff;
      case "DOMString":
      case "USVString":
        return str(value);
      case "DOMString?":
        return value === null ? null : str(value);
      case "Window?":
        if (value === null || value === global) {
          return value;
        }
        throw new $TypeError(where + "Failed to convert value to 'Window'.");
      case "EventTarget?":
        if (value === null || isTarget(value)) {
          return value;
        }
        throw new $TypeError(where + "Failed to convert value to 'EventTarget'.");
      case "HTMLElement?":
        if (value === null || (isNode(value) && kindOf(value) === 1)) {
          return value;
        }
        throw new $TypeError(where + "Failed to convert value to 'HTMLElement'.");
      case "Storage?":
        if (value === null || WeakMapGet(areaKinds, value) !== undefined) {
          return value;
        }
        throw new $TypeError(where + "Failed to convert value to 'Storage'.");
      case "MessageEventSource?":
        if (value === null || value === global || isPort(value)) {
          return value;
        }
        throw new $TypeError(where +
          "The provided value is not of type '(MessagePort or ServiceWorker or Window)'.");
      case "sequence<MessagePort>":
        return sequence(value, "", m[3], (item) => {
          if (!isPort(item)) {
            throw new $TypeError(where + "Failed to convert value to 'MessagePort'.");
          }
          return item;
        });
      default:
        return value;
    }
  }

  // An event constructor's arguments: its type and dictionary converted,
  // unless the bindings made them.
  function eventArgs(count, type, init, list, iface) {
    if (typeof init === "object" && init !== null && WeakSetHas(convertedDictionaries, init)) {
      return [type, init];
    }
    if (count < 1) {
      throw new $TypeError(
        "Failed to construct '" + iface + "': 1 argument required, but only 0 present.",
      );
    }
    const name = str(type);
    return [name, dictionary(init, list, iface)];
  }

  // The state of `event`, which must be an object of the interface
  // `bit` names.
  function eventOf(event, bit) {
    const s = eventState(event);
    if ((s.kinds & bit) === 0) {
      throw illegalInvocation();
    }
    return s;
  }

  function fieldsOf(event, bit) {
    return eventOf(event, bit).fields;
  }

  // The DOM's "set the canceled flag".
  function cancel(s) {
    if (s.cancelable && !s.passive) {
      s.canceled = true;
    }
  }

  // The DOM's "initialize" an event.
  function initialize(s, type, bubbles, cancelable) {
    s.initialized = true;
    s.stop = false;
    s.stopImmediate = false;
    s.canceled = false;
    s.trusted = false;
    s.target = null;
    s.type = type;
    s.bubbles = bubbles;
    s.cancelable = cancelable;
  }

  function isTrusted() {
    return eventState(this).trusted;
  }

  class Event {
    #state;

    static {
      eventState = (event) => {
        if (typeof event === "object" && event !== null && #state in event) {
          return event.#state;
        }
        throw illegalInvocation();
      };
      isEvent = (value) => typeof value === "object" && value !== null && #state in value;
    }

    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, EVENT_INIT, "Event");
      const fields = args[1];
      this.#state = {
        type: args[0],
        fields,
        kinds: EVENT,
        bubbles: fields.bubbles,
        cancelable: fields.cancelable,
        composed: fields.composed,
        target: null,
        currentTarget: null,
        phase: 0,
        path: [],
        stop: false,
        stopImmediate: false,
        canceled: false,
        passive: false,
        initialized: true,
        dispatching: false,
        trusted: false,
        timeStamp: native.now(),
        page: null,
        offset: null,
      };
      defineProperty(this, "isTrusted", { get: isTrusted, enumerable: true, configurable: false });
    }

    get type() {
      return eventState(this).type;
    }

    get target() {
      return eventState(this).target;
    }

    get srcElement() {
      return eventState(this).target;
    }

    get currentTarget() {
      return eventState(this).currentTarget;
    }

    composedPath() {
      return ArraySlice(eventState(this).path);
    }

    get eventPhase() {
      return eventState(this).phase;
    }

    stopPropagation() {
      eventState(this).stop = true;
    }

    get cancelBubble() {
      return eventState(this).stop;
    }

    set cancelBubble(value) {
      if (value) {
        eventState(this).stop = true;
      }
    }

    stopImmediatePropagation() {
      const s = eventState(this);
      s.stop = true;
      s.stopImmediate = true;
    }

    get bubbles() {
      return eventState(this).bubbles;
    }

    get cancelable() {
      return eventState(this).cancelable;
    }

    get returnValue() {
      return !eventState(this).canceled;
    }

    set returnValue(value) {
      if (!value) {
        cancel(eventState(this));
      }
    }

    preventDefault() {
      cancel(eventState(this));
    }

    get defaultPrevented() {
      return eventState(this).canceled;
    }

    get composed() {
      return eventState(this).composed;
    }

    get timeStamp() {
      return eventState(this).timeStamp;
    }

    initEvent(type, bubbles = false, cancelable = false) {
      const s = eventState(this);
      need(arguments.length, 1, "initEvent", "Event");
      if (!s.dispatching) {
        initialize(s, str(type), !!bubbles, !!cancelable);
      }
    }
  }

  class CustomEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, CUSTOM_INIT, "CustomEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= CUSTOM;
    }

    get detail() {
      return fieldsOf(this, CUSTOM).detail;
    }

    initCustomEvent(type, bubbles = false, cancelable = false, detail = null) {
      const s = eventOf(this, CUSTOM);
      need(arguments.length, 1, "initCustomEvent", "CustomEvent");
      if (!s.dispatching) {
        initialize(s, str(type), !!bubbles, !!cancelable);
        s.fields.detail = detail;
      }
    }
  }

  class UIEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, UI_INIT, "UIEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= UI;
    }

    get view() {
      return fieldsOf(this, UI).view;
    }

    get detail() {
      return fieldsOf(this, UI).detail;
    }

    // A keyboard event's `which` is its key code, a mouse event's its
    // button and one, as in the reference browser.
    get which() {
      const s = eventOf(this, UI);
      if ((s.kinds & KEYBOARD) !== 0) {
        return s.fields.keyCode;
      }
      if ((s.kinds & MOUSE) !== 0) {
        return s.fields.button + 1;
      }
      return s.fields.which;
    }

    initUIEvent(type, bubbles = false, cancelable = false, view = null, detail = 0) {
      const s = eventOf(this, UI);
      need(arguments.length, 1, "initUIEvent", "UIEvent");
      if (!s.dispatching) {
        initialize(s, str(type), !!bubbles, !!cancelable);
        s.fields.view = view === global ? global : null;
        s.fields.detail = $Number(detail) | 0;
      }
    }
  }

  class FocusEvent extends UIEvent {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, FOCUS_INIT, "FocusEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= FOCUS;
    }

    get relatedTarget() {
      return fieldsOf(this, FOCUS).relatedTarget;
    }
  }

  // `getModifierState`'s answer for `key` (UI Events section 6.3.1).
  function modifierState(fields, key) {
    switch (str(key)) {
      case "Alt":
        return fields.altKey;
      case "Control":
      case "Accel":
        return fields.ctrlKey;
      case "Shift":
        return fields.shiftKey;
      case "Meta":
        return fields.metaKey;
      case "AltGraph":
      case "CapsLock":
      case "Fn":
      case "FnLock":
      case "Hyper":
      case "NumLock":
      case "ScrollLock":
      case "Super":
      case "Symbol":
      case "SymbolLock":
        return fields["modifier" + str(key)];
      default:
        return false;
    }
  }

  class MouseEvent extends UIEvent {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, MOUSE_INIT, "MouseEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= MOUSE;
    }

    get screenX() {
      return fieldsOf(this, MOUSE).screenX | 0;
    }

    get screenY() {
      return fieldsOf(this, MOUSE).screenY | 0;
    }

    get clientX() {
      return fieldsOf(this, MOUSE).clientX | 0;
    }

    get clientY() {
      return fieldsOf(this, MOUSE).clientY | 0;
    }

    get x() {
      return fieldsOf(this, MOUSE).clientX | 0;
    }

    get y() {
      return fieldsOf(this, MOUSE).clientY | 0;
    }

    // The point on the page and from the target's padding edge, which an
    // event the engine makes is given; a script's has its client point.
    get pageX() {
      const s = eventOf(this, MOUSE);
      return (s.page === null ? s.fields.clientX : s.page[0]) | 0;
    }

    get pageY() {
      const s = eventOf(this, MOUSE);
      return (s.page === null ? s.fields.clientY : s.page[1]) | 0;
    }

    get offsetX() {
      const s = eventOf(this, MOUSE);
      return (s.offset === null ? s.fields.clientX : s.offset[0]) | 0;
    }

    get offsetY() {
      const s = eventOf(this, MOUSE);
      return (s.offset === null ? s.fields.clientY : s.offset[1]) | 0;
    }

    get movementX() {
      return fieldsOf(this, MOUSE).movementX | 0;
    }

    get movementY() {
      return fieldsOf(this, MOUSE).movementY | 0;
    }

    get ctrlKey() {
      return fieldsOf(this, MOUSE).ctrlKey;
    }

    get shiftKey() {
      return fieldsOf(this, MOUSE).shiftKey;
    }

    get altKey() {
      return fieldsOf(this, MOUSE).altKey;
    }

    get metaKey() {
      return fieldsOf(this, MOUSE).metaKey;
    }

    get button() {
      return fieldsOf(this, MOUSE).button;
    }

    get buttons() {
      return fieldsOf(this, MOUSE).buttons;
    }

    get relatedTarget() {
      return fieldsOf(this, MOUSE).relatedTarget;
    }

    getModifierState(keyArg) {
      const fields = fieldsOf(this, MOUSE);
      need(arguments.length, 1, "getModifierState", "MouseEvent");
      return modifierState(fields, keyArg);
    }

    initMouseEvent(
      type, bubbles = false, cancelable = false, view = null, detail = 0, screenX = 0,
      screenY = 0, clientX = 0, clientY = 0, ctrlKey = false, altKey = false, shiftKey = false,
      metaKey = false, button = 0, relatedTarget = null,
    ) {
      const s = eventOf(this, MOUSE);
      need(arguments.length, 1, "initMouseEvent", "MouseEvent");
      if (s.dispatching) {
        return;
      }
      initialize(s, str(type), !!bubbles, !!cancelable);
      const f = s.fields;
      f.view = view === global ? global : null;
      f.detail = $Number(detail) | 0;
      f.screenX = $Number(screenX) | 0;
      f.screenY = $Number(screenY) | 0;
      f.clientX = $Number(clientX) | 0;
      f.clientY = $Number(clientY) | 0;
      f.ctrlKey = !!ctrlKey;
      f.altKey = !!altKey;
      f.shiftKey = !!shiftKey;
      f.metaKey = !!metaKey;
      f.button = ($Number(button) << 16) >> 16;
      f.relatedTarget = isTarget(relatedTarget) ? relatedTarget : null;
    }
  }

  class PointerEvent extends MouseEvent {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, POINTER_INIT, "PointerEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= POINTER;
    }

    get pointerId() {
      return fieldsOf(this, POINTER).pointerId;
    }

    get width() {
      return fieldsOf(this, POINTER).width;
    }

    get height() {
      return fieldsOf(this, POINTER).height;
    }

    get pressure() {
      return fieldsOf(this, POINTER).pressure;
    }

    get tangentialPressure() {
      return fieldsOf(this, POINTER).tangentialPressure;
    }

    get tiltX() {
      return fieldsOf(this, POINTER).tiltX;
    }

    get tiltY() {
      return fieldsOf(this, POINTER).tiltY;
    }

    get twist() {
      return fieldsOf(this, POINTER).twist;
    }

    get altitudeAngle() {
      return fieldsOf(this, POINTER).altitudeAngle;
    }

    get azimuthAngle() {
      return fieldsOf(this, POINTER).azimuthAngle;
    }

    get pointerType() {
      return fieldsOf(this, POINTER).pointerType;
    }

    get isPrimary() {
      return fieldsOf(this, POINTER).isPrimary;
    }

    get persistentDeviceId() {
      fieldsOf(this, POINTER);
      return 0;
    }

    getCoalescedEvents() {
      fieldsOf(this, POINTER);
      return [];
    }

    getPredictedEvents() {
      fieldsOf(this, POINTER);
      return [];
    }
  }

  class WheelEvent extends MouseEvent {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, WHEEL_INIT, "WheelEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= WHEEL;
    }

    get deltaX() {
      return fieldsOf(this, WHEEL).deltaX;
    }

    get deltaY() {
      return fieldsOf(this, WHEEL).deltaY;
    }

    get deltaZ() {
      return fieldsOf(this, WHEEL).deltaZ;
    }

    get deltaMode() {
      return fieldsOf(this, WHEEL).deltaMode;
    }
  }

  class KeyboardEvent extends UIEvent {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(
        arguments.length,
        type,
        eventInitDict,
        KEYBOARD_INIT,
        "KeyboardEvent",
      );
      super(args[0], args[1]);
      eventState(this).kinds |= KEYBOARD;
    }

    get key() {
      return fieldsOf(this, KEYBOARD).key;
    }

    get code() {
      return fieldsOf(this, KEYBOARD).code;
    }

    get location() {
      return fieldsOf(this, KEYBOARD).location;
    }

    get ctrlKey() {
      return fieldsOf(this, KEYBOARD).ctrlKey;
    }

    get shiftKey() {
      return fieldsOf(this, KEYBOARD).shiftKey;
    }

    get altKey() {
      return fieldsOf(this, KEYBOARD).altKey;
    }

    get metaKey() {
      return fieldsOf(this, KEYBOARD).metaKey;
    }

    get repeat() {
      return fieldsOf(this, KEYBOARD).repeat;
    }

    get isComposing() {
      return fieldsOf(this, KEYBOARD).isComposing;
    }

    get charCode() {
      return fieldsOf(this, KEYBOARD).charCode;
    }

    get keyCode() {
      return fieldsOf(this, KEYBOARD).keyCode;
    }

    getModifierState(keyArg) {
      const fields = fieldsOf(this, KEYBOARD);
      need(arguments.length, 1, "getModifierState", "KeyboardEvent");
      return modifierState(fields, keyArg);
    }

    initKeyboardEvent(
      type, bubbles = false, cancelable = false, view = null, key = "", location = 0,
      ctrlKey = false, altKey = false, shiftKey = false, metaKey = false,
    ) {
      const s = eventOf(this, KEYBOARD);
      need(arguments.length, 1, "initKeyboardEvent", "KeyboardEvent");
      if (s.dispatching) {
        return;
      }
      initialize(s, str(type), !!bubbles, !!cancelable);
      const f = s.fields;
      f.view = view === global ? global : null;
      f.key = str(key);
      f.location = $Number(location) >>> 0;
      f.ctrlKey = !!ctrlKey;
      f.altKey = !!altKey;
      f.shiftKey = !!shiftKey;
      f.metaKey = !!metaKey;
    }
  }

  class InputEvent extends UIEvent {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, INPUT_INIT, "InputEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= INPUT;
    }

    get data() {
      return fieldsOf(this, INPUT).data;
    }

    get isComposing() {
      return fieldsOf(this, INPUT).isComposing;
    }

    get inputType() {
      return fieldsOf(this, INPUT).inputType;
    }

    get dataTransfer() {
      fieldsOf(this, INPUT);
      return null;
    }

    getTargetRanges() {
      fieldsOf(this, INPUT);
      return [];
    }
  }

  class SubmitEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, SUBMIT_INIT, "SubmitEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= SUBMIT;
    }

    get submitter() {
      return fieldsOf(this, SUBMIT).submitter;
    }
  }

  class ErrorEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, ERROR_INIT, "ErrorEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= ERROR;
    }

    get message() {
      return fieldsOf(this, ERROR).message;
    }

    get filename() {
      return fieldsOf(this, ERROR).filename;
    }

    get lineno() {
      return fieldsOf(this, ERROR).lineno;
    }

    get colno() {
      return fieldsOf(this, ERROR).colno;
    }

    get error() {
      return fieldsOf(this, ERROR).error;
    }
  }

  class PageTransitionEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(
        arguments.length,
        type,
        eventInitDict,
        PAGE_TRANSITION_INIT,
        "PageTransitionEvent",
      );
      super(args[0], args[1]);
      eventState(this).kinds |= PAGE_TRANSITION;
    }

    get persisted() {
      return fieldsOf(this, PAGE_TRANSITION).persisted;
    }
  }

  const ANIMATION = 4096;
  const TRANSITION = 8192;
  const ANIMATION_INIT = members(EVENT_INIT, [
    ["animationName", "DOMString", ""],
    ["elapsedTime", "double", 0],
    ["pseudoElement", "DOMString", ""],
  ], "AnimationEventInit");
  const TRANSITION_INIT = members(EVENT_INIT, [
    ["elapsedTime", "double", 0],
    ["propertyName", "DOMString", ""],
    ["pseudoElement", "DOMString", ""],
  ], "TransitionEventInit");

  // CSS Animations' and Transitions' events (M26): what an animation or a
  // transition's change of phase queued (M22.2).
  class AnimationEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(
        arguments.length,
        type,
        eventInitDict,
        ANIMATION_INIT,
        "AnimationEvent",
      );
      super(args[0], args[1]);
      eventState(this).kinds |= ANIMATION;
    }

    get animationName() {
      return fieldsOf(this, ANIMATION).animationName;
    }

    get elapsedTime() {
      return fieldsOf(this, ANIMATION).elapsedTime;
    }

    get pseudoElement() {
      return fieldsOf(this, ANIMATION).pseudoElement;
    }
  }

  class TransitionEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(
        arguments.length,
        type,
        eventInitDict,
        TRANSITION_INIT,
        "TransitionEvent",
      );
      super(args[0], args[1]);
      eventState(this).kinds |= TRANSITION;
    }

    get propertyName() {
      return fieldsOf(this, TRANSITION).propertyName;
    }

    get elapsedTime() {
      return fieldsOf(this, TRANSITION).elapsedTime;
    }

    get pseudoElement() {
      return fieldsOf(this, TRANSITION).pseudoElement;
    }
  }

  // SecurityPolicyViolationEvent (CSP section 5.1, M33.1).
  const SECURITY = 131072;
  const SECURITY_INIT = members(EVENT_INIT, [
    ["blockedURI", "USVString", ""],
    ["columnNumber", "unsigned long", 0],
    ["disposition", "DOMString", "enforce"],
    ["documentURI", "USVString", ""],
    ["effectiveDirective", "DOMString", ""],
    ["lineNumber", "unsigned long", 0],
    ["originalPolicy", "DOMString", ""],
    ["referrer", "USVString", ""],
    ["sample", "DOMString", ""],
    ["sourceFile", "USVString", ""],
    ["statusCode", "unsigned short", 0],
    ["violatedDirective", "DOMString", ""],
  ], "SecurityPolicyViolationEventInit");

  class SecurityPolicyViolationEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(
        arguments.length,
        type,
        eventInitDict,
        SECURITY_INIT,
        "SecurityPolicyViolationEvent",
      );
      super(args[0], args[1]);
      eventState(this).kinds |= SECURITY;
    }

    get documentURI() {
      return fieldsOf(this, SECURITY).documentURI;
    }

    get referrer() {
      return fieldsOf(this, SECURITY).referrer;
    }

    get blockedURI() {
      return fieldsOf(this, SECURITY).blockedURI;
    }

    get effectiveDirective() {
      return fieldsOf(this, SECURITY).effectiveDirective;
    }

    get violatedDirective() {
      return fieldsOf(this, SECURITY).violatedDirective;
    }

    get originalPolicy() {
      return fieldsOf(this, SECURITY).originalPolicy;
    }

    get sourceFile() {
      return fieldsOf(this, SECURITY).sourceFile;
    }

    get sample() {
      return fieldsOf(this, SECURITY).sample;
    }

    get disposition() {
      return fieldsOf(this, SECURITY).disposition;
    }

    get statusCode() {
      return fieldsOf(this, SECURITY).statusCode;
    }

    get lineNumber() {
      return fieldsOf(this, SECURITY).lineNumber;
    }

    get columnNumber() {
      return fieldsOf(this, SECURITY).columnNumber;
    }
  }

  // ProgressEvent (XMLHttpRequest 5, M27.6).
  const PROGRESS = 16384;
  const PROGRESS_INIT = members(EVENT_INIT, [
    ["lengthComputable", "boolean", false],
    ["loaded", "double", 0],
    ["total", "double", 0],
  ], "ProgressEventInit");

  class ProgressEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, PROGRESS_INIT, "ProgressEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= PROGRESS;
    }

    get lengthComputable() {
      return fieldsOf(this, PROGRESS).lengthComputable;
    }

    get loaded() {
      return fieldsOf(this, PROGRESS).loaded;
    }

    get total() {
      return fieldsOf(this, PROGRESS).total;
    }
  }

  // StorageEvent (HTML section 12.2.4, M28): a change of a Web Storage
  // area another document of the origin made.
  const STORAGE = 32768;
  const STORAGE_INIT = members(EVENT_INIT, [
    ["key", "DOMString?", null],
    ["newValue", "DOMString?", null],
    ["oldValue", "DOMString?", null],
    ["storageArea", "Storage?", null],
    ["url", "USVString", ""],
  ], "StorageEventInit");

  class StorageEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, STORAGE_INIT, "StorageEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= STORAGE;
    }

    get key() {
      return fieldsOf(this, STORAGE).key;
    }

    get oldValue() {
      return fieldsOf(this, STORAGE).oldValue;
    }

    get newValue() {
      return fieldsOf(this, STORAGE).newValue;
    }

    get url() {
      return fieldsOf(this, STORAGE).url;
    }

    get storageArea() {
      return fieldsOf(this, STORAGE).storageArea;
    }

    initStorageEvent(
      type,
      bubbles = false,
      cancelable = false,
      key = null,
      oldValue = null,
      newValue = null,
      url = "",
      storageArea = null,
    ) {
      const s = eventOf(this, STORAGE);
      need(arguments.length, 1, "initStorageEvent", "StorageEvent");
      const name = str(type);
      const text = (value) => (value === null ? null : str(value));
      const fields = [text(key), text(oldValue), text(newValue), str(url)];
      if (storageArea !== null && WeakMapGet(areaKinds, storageArea) === undefined) {
        throw new $TypeError(
          prefix("initStorageEvent", "StorageEvent") + "parameter 8 is not of type 'Storage'.",
        );
      }
      if (!s.dispatching) {
        initialize(s, name, !!bubbles, !!cancelable);
        s.fields.key = fields[0];
        s.fields.oldValue = fields[1];
        s.fields.newValue = fields[2];
        s.fields.url = fields[3];
        s.fields.storageArea = storageArea;
      }
    }
  }

  // An interface's constants, on it and its prototype, as Web IDL has
  // them.
  function constants(cls, pairs) {
    for (let i = 0; i < pairs.length; i += 2) {
      const desc = { value: pairs[i + 1], writable: false, enumerable: true, configurable: false };
      defineProperty(cls, pairs[i], desc);
      defineProperty(cls.prototype, pairs[i], desc);
    }
  }

  // The event interfaces, by name, with their dictionaries: those the
  // engine makes events of.
  const eventInterfaces = new $Map();
  const EVENT_INTERFACES = [
    ["Event", Event, EVENT_INIT],
    ["CustomEvent", CustomEvent, CUSTOM_INIT],
    ["UIEvent", UIEvent, UI_INIT],
    ["FocusEvent", FocusEvent, FOCUS_INIT],
    ["MouseEvent", MouseEvent, MOUSE_INIT],
    ["PointerEvent", PointerEvent, POINTER_INIT],
    ["WheelEvent", WheelEvent, WHEEL_INIT],
    ["KeyboardEvent", KeyboardEvent, KEYBOARD_INIT],
    ["InputEvent", InputEvent, INPUT_INIT],
    ["SubmitEvent", SubmitEvent, SUBMIT_INIT],
    ["ErrorEvent", ErrorEvent, ERROR_INIT],
    ["PageTransitionEvent", PageTransitionEvent, PAGE_TRANSITION_INIT],
    ["AnimationEvent", AnimationEvent, ANIMATION_INIT],
    ["TransitionEvent", TransitionEvent, TRANSITION_INIT],
    ["ProgressEvent", ProgressEvent, PROGRESS_INIT],
    ["StorageEvent", StorageEvent, STORAGE_INIT],
    ["SecurityPolicyViolationEvent", SecurityPolicyViolationEvent, SECURITY_INIT],
  ];
  for (let i = 0; i < EVENT_INTERFACES.length; i++) {
    const entry = EVENT_INTERFACES[i];
    expose(entry[1], entry[0]);
    MapSet(eventInterfaces, entry[0], entry);
  }
  constants(Event, ["NONE", 0, "CAPTURING_PHASE", 1, "AT_TARGET", 2, "BUBBLING_PHASE", 3]);
  constants(WheelEvent, ["DOM_DELTA_PIXEL", 0, "DOM_DELTA_LINE", 1, "DOM_DELTA_PAGE", 2]);
  constants(KeyboardEvent, [
    "DOM_KEY_LOCATION_STANDARD", 0, "DOM_KEY_LOCATION_LEFT", 1, "DOM_KEY_LOCATION_RIGHT", 2,
    "DOM_KEY_LOCATION_NUMPAD", 3,
  ]);

  // `document.createEvent`'s names (DOM section 4.5), lowercased: those
  // of the interfaces there are.
  const legacyEvents = new $Map();
  const LEGACY_EVENTS = [
    ["customevent", CustomEvent, CUSTOM_INIT],
    ["event", Event, EVENT_INIT],
    ["events", Event, EVENT_INIT],
    ["focusevent", FocusEvent, FOCUS_INIT],
    ["htmlevents", Event, EVENT_INIT],
    ["keyboardevent", KeyboardEvent, KEYBOARD_INIT],
    ["mouseevent", MouseEvent, MOUSE_INIT],
    ["mouseevents", MouseEvent, MOUSE_INIT],
    ["storageevent", StorageEvent, STORAGE_INIT],
    ["svgevents", Event, EVENT_INIT],
    ["uievent", UIEvent, UI_INIT],
    ["uievents", UIEvent, UI_INIT],
  ];
  for (let i = 0; i < LEGACY_EVENTS.length; i++) {
    MapSet(legacyEvents, LEGACY_EVENTS[i][0], LEGACY_EVENTS[i]);
  }

  // ---- Dispatch (DOM section 2.9) -----------------------------------------

  // The event whose listener runs: `window.event` (DOM section 2.9).
  let currentEvent = undefined;

  // Runs a dispatch through, with no checkpoint between its listeners,
  // as a script's dispatch is run; what it ends with.
  function drain(run) {
    let step;
    do {
      step = GeneratorNext(run);
    } while (!step.done);
    return step.value;
  }

  // The body's object, made if it has none, so that the handlers of the
  // window its attributes hold are the window's before an event reaches
  // it.
  function windowBody() {
    const body = childOfHtml(native.document(), "body");
    if (body !== null) {
      wrap(body);
    }
  }

  // The DOM's dispatch of `event` at `target` (section 2.9), with no
  // shadow trees and so no retargeting, its target `override` if given
  // (HTML's legacy target override): a generator that yields after
  // each listener it calls, so that the realm performs a microtask
  // checkpoint there when the engine dispatches; a script's dispatch is
  // run through at once. It ends with whether the event was canceled.
  //
  // The path is the target, its ancestors and, from the document, the
  // window, but for a `load` event. A click (a MouseEvent) runs the
  // activation behaviour of the target, or of the nearest ancestor that
  // has one when it bubbles, unless a listener canceled it.
  function* dispatching(target, event, override) {
    const s = eventState(event);
    s.dispatching = true;
    s.target = override === undefined ? target : override;
    const path = [target];
    let activator = null;
    if (isNode(target)) {
      const handle = handleOf(target);
      const ancestors = native.ancestors(handle);
      for (let i = 0; i < ancestors.length; i++) {
        ArrayPush(path, wrap(ancestors[i]));
      }
      if (kindOf(path[path.length - 1]) === 9 && s.type !== "load") {
        windowBody();
        ArrayPush(path, global);
      }
      if ((s.kinds & MOUSE) !== 0 && s.type === "click") {
        const found = native.activator(handle);
        if (found !== null && (found === handle || s.bubbles)) {
          activator = found;
        }
      }
    } else if (target === global && !WORKER) {
      windowBody();
    }
    // A checkbox's or a radio button's legacy-pre-activation behaviour
    // (HTML section 4.10.5.1.15, M26.8): its new state before the
    // listeners see the click.
    const pre = activator === null ? null : preActivate(activator);
    s.path = path;
    try {
      for (let i = path.length - 1; i >= 0; i--) {
        if (s.stop) {
          break;
        }
        s.phase = path[i] === target ? 2 : 1;
        s.currentTarget = path[i];
        const listeners = ArraySlice(listenersOf(path[i]));
        for (let j = 0; j < listeners.length; j++) {
          const l = listeners[j];
          if (l.removed || l.type !== s.type || !l.capture) {
            continue;
          }
          invoke(path[i], l, s, event);
          yield;
          if (s.stopImmediate) {
            break;
          }
        }
      }
      for (let i = 0; i < path.length; i++) {
        if (path[i] === target) {
          s.phase = 2;
        } else if (!s.bubbles) {
          continue;
        } else {
          s.phase = 3;
        }
        if (s.stop) {
          break;
        }
        s.currentTarget = path[i];
        const listeners = ArraySlice(listenersOf(path[i]));
        for (let j = 0; j < listeners.length; j++) {
          const l = listeners[j];
          if (l.removed || l.type !== s.type || l.capture) {
            continue;
          }
          invoke(path[i], l, s, event);
          yield;
          if (s.stopImmediate) {
            break;
          }
        }
      }
    } finally {
      s.phase = 0;
      s.currentTarget = null;
      s.path = [];
      s.dispatching = false;
      s.stop = false;
      s.stopImmediate = false;
    }
    if (activator !== null && s.canceled && pre !== null) {
      cancelActivation(activator, pre);
    } else if (activator !== null && !s.canceled) {
      yield* activate(activator, pre, target);
    }
    return s.canceled;
  }

  // The state a checkbox or a radio button was in before a click's
  // pre-activation changed it, or null for another element: a checkbox
  // toggled, its indeterminate flag cleared; a radio button checked.
  function preActivate(handle) {
    if (native.tag(handle) !== "input") {
      return null;
    }
    const kind = native.inputType(handle);
    if (kind === "checkbox") {
      const old = [native.checkedness(handle), native.indeterminate(handle)];
      native.setCheckedness(handle, !old[0]);
      native.setIndeterminate(handle, false);
      return old;
    }
    if (kind === "radio") {
      const group = native.radioGroup(handle);
      let old = null;
      for (let i = 0; i < group.length; i++) {
        if (native.checkedness(group[i])) {
          old = group[i];
        }
      }
      native.setCheckedness(handle, true);
      return [old];
    }
    return null;
  }

  // The legacy-canceled-activation behaviour: the state put back.
  function cancelActivation(handle, pre) {
    if (native.inputType(handle) === "checkbox") {
      native.setCheckedness(handle, pre[0]);
      native.setIndeterminate(handle, pre[1]);
    } else if (pre[0] !== null) {
      native.setCheckedness(pre[0], true);
    } else {
      native.setCheckedness(handle, false);
    }
  }

  // An element's activation behaviour, after a click not canceled: a
  // link's followed by the host; a checkbox's or a radio button's
  // `input` and `change` (HTML section 4.10.5.1.15), at one in the
  // document; a label's click given its control, focused first, unless
  // the click came from inside the control.
  function* activate(handle, pre, target) {
    const tag = native.tag(handle);
    if (tag === "input" && pre !== null) {
      if (!native.connected(handle)) {
        return;
      }
      const element = wrap(handle);
      native.setUserValidity(handle);
      yield* fireTrusted(element, "input", true);
      yield* fireTrusted(element, "change", false);
      return;
    }
    if (tag === "label") {
      const control = native.labeledControl(handle);
      if (control === null || (isNode(target) && wrap(control).contains(target))) {
        return;
      }
      const element = wrap(control);
      focusElement(control, false);
      const click = new MouseEvent("click", made(MOUSE_INIT, [
        "bubbles", true,
        "cancelable", true,
        "composed", true,
        "view", global,
      ]));
      eventState(click).trusted = true;
      yield* dispatching(element, click);
      return;
    }
    // A submit or reset button's: its form submitted or reset (M26.9).
    const form = native.formOwner(handle);
    if (form !== null && isSubmitButton(handle)) {
      yield* submitForm(form, handle, false);
      return;
    }
    if (form !== null && isResetButton(handle)) {
      yield* resetForm(form);
      return;
    }
    native.activate(handle);
  }

  // A `button`'s type, `submit` by default (HTML section 4.10.6).
  function buttonType(handle) {
    const value = native.attribute(handle, "type");
    const lower = value === null ? "" : ToLowerCase(value);
    return lower === "reset" || lower === "button" ? lower : "submit";
  }

  function isSubmitButton(handle) {
    const tag = native.tag(handle);
    if (tag === "button") {
      return buttonType(handle) === "submit";
    }
    const kind = tag === "input" ? native.inputType(handle) : "";
    return kind === "submit" || kind === "image";
  }

  function isResetButton(handle) {
    const tag = native.tag(handle);
    if (tag === "button") {
      return buttonType(handle) === "reset";
    }
    return tag === "input" && native.inputType(handle) === "reset";
  }

  // HTML's form submission from the bindings (section 4.10.21.3,
  // M26.9): unless `submit()` asked, a cancelable `submit` naming the
  // submitter; then the host's navigation, after the run.
  function* submitForm(form, submitter, fromMethod) {
    // Interactive validation first (M26.9b), unless the form or its
    // submitter says `novalidate`.
    if (
      !fromMethod &&
      native.attribute(form, "novalidate") === null &&
      (submitter === null || native.attribute(submitter, "formnovalidate") === null)
    ) {
      native.markSubmitted(form);
      if (!(yield* validateInteractively(form))) {
        return;
      }
    }
    if (!fromMethod) {
      const event = new SubmitEvent("submit", made(SUBMIT_INIT, [
        "bubbles", true,
        "cancelable", true,
        "submitter", submitter === null ? null : wrap(submitter),
      ]));
      eventState(event).trusted = true;
      if (yield* dispatching(wrap(form), event)) {
        return;
      }
    }
    native.submitForm(form, submitter);
  }

  // HTML's reset (section 4.10.22): a cancelable `reset`, then the
  // controls back to their defaults.
  function* resetForm(form) {
    const event = new Event("reset", made(EVENT_INIT, ["bubbles", true, "cancelable", true]));
    eventState(event).trusted = true;
    if (yield* dispatching(wrap(form), event)) {
      return;
    }
    native.resetForm(form);
  }

  // A trusted Event of `type` at `element`, bubbling, composed if asked.
  function* fireTrusted(element, type, composed) {
    const event = new Event(type, made(EVENT_INIT, [
      "bubbles", true,
      "composed", composed,
    ]));
    eventState(event).trusted = true;
    yield* dispatching(element, event);
  }

  // The DOM's "inner invoke" for one listener: removed first if it is
  // once; called with `window.event` the event and, when passive, its
  // canceling refused; what it throws reported.
  function invoke(target, listener, s, event) {
    if (listener.once) {
      removeListener(listenersOf(target), listener);
    }
    const previous = currentEvent;
    currentEvent = event;
    if (listener.passive) {
      s.passive = true;
    }
    try {
      const callback = listener.callback;
      if (typeof callback === "function") {
        ReflectApply(callback, s.currentTarget, [event]);
      } else {
        const handleEvent = callback.handleEvent;
        if (typeof handleEvent !== "function") {
          throw new $TypeError("The 'handleEvent' property of the event listener is not callable.");
        }
        ReflectApply(handleEvent, callback, [event]);
      }
    } catch (error) {
      reportException(error);
    } finally {
      s.passive = false;
      currentEvent = previous;
    }
  }

  // ---- Errors (HTML section 8.1.4.6) ---------------------------------------

  // Whether an `error` event is being fired: an exception one of its
  // listeners throws goes to the console alone.
  let reporting = false;

  // Fires the `error` event at the window for `value`, an uncaught
  // exception the console reports as `message` at `url`, `line` and
  // `column`; whether a listener canceled it. A muted error's event
  // tells nothing of it.
  function fireError(value, message, url, line, column, muted) {
    if (reporting) {
      return false;
    }
    reporting = true;
    try {
      const event = new ErrorEvent("error", made(ERROR_INIT, [
        "cancelable", true,
        "message", muted ? "Script error." : "Uncaught " + message,
        "filename", muted ? "" : url,
        "lineno", muted ? 0 : line,
        "colno", muted ? 0 : column,
        "error", muted ? null : value,
      ]));
      eventState(event).trusted = true;
      return drain(dispatching(global, event));
    } finally {
      reporting = false;
    }
  }

  // HTML's "report an exception" for `value`, which a listener, an
  // event handler or a timer's callback threw: the `error` event, then,
  // unless it was canceled, the console.
  function reportException(value) {
    const report = native.describe(value);
    if (!fireError(value, report[0], report[1], report[2], report[3], false)) {
      native.reportError(report[0], report[1], report[2], report[3]);
    }
  }

  // ---- Aborting (DOM section 3) --------------------------------------------

  let signalOf;
  let isSignal;

  class AbortSignal extends EventTarget {
    #state;

    static {
      signalOf = (signal) => {
        if (typeof signal === "object" && signal !== null && #state in signal) {
          return signal.#state;
        }
        throw illegalInvocation();
      };
      isSignal = (value) => typeof value === "object" && value !== null && #state in value;
    }

    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("AbortSignal");
      }
      super();
      this.#state = { aborted: false, reason: undefined, algorithms: [], dependents: [] };
    }

    static abort(reason) {
      const signal = new AbortSignal(KEY);
      const s = signalOf(signal);
      s.aborted = true;
      s.reason = reason === undefined ? abortError() : reason;
      return signal;
    }

    static timeout(milliseconds) {
      need(arguments.length, 1, "timeout", "AbortSignal");
      const ms = $Number(milliseconds);
      if (ms !== ms || ms < 0 || ms === Infinity) {
        throw new $TypeError(
          prefix("timeout", "AbortSignal") +
            "Value is outside the 'unsigned long long' value range.",
        );
      }
      const signal = new AbortSignal(KEY);
      const timedOut = () =>
        signalAbort(signal, new $DOMException("signal timed out", "TimeoutError"));
      startTimer(timedOut, ms > 2147483647 ? 2147483647 : ms, [], false);
      return signal;
    }

    static any(signals) {
      need(arguments.length, 1, "any", "AbortSignal");
      const list = sequence(signals, "any", "AbortSignal");
      for (let i = 0; i < list.length; i++) {
        if (!isSignal(list[i])) {
          throw new $TypeError(
            prefix("any", "AbortSignal") + "Failed to convert value to 'AbortSignal'.",
          );
        }
      }
      const result = new AbortSignal(KEY);
      for (let i = 0; i < list.length; i++) {
        const s = signalOf(list[i]);
        if (s.aborted) {
          const r = signalOf(result);
          r.aborted = true;
          r.reason = s.reason;
          return result;
        }
      }
      for (let i = 0; i < list.length; i++) {
        ArrayPush(signalOf(list[i]).dependents, result);
      }
      return result;
    }

    get aborted() {
      return signalOf(this).aborted;
    }

    get reason() {
      return signalOf(this).reason;
    }

    throwIfAborted() {
      const s = signalOf(this);
      if (s.aborted) {
        throw s.reason;
      }
    }
  }

  function abortError() {
    return new $DOMException("signal is aborted without reason", "AbortError");
  }

  // The DOM's "signal abort": its algorithms run (the listeners it
  // removes), an `abort` event fired at it, then its dependents
  // aborted with the same reason.
  function signalAbort(signal, reason) {
    const s = signalOf(signal);
    if (s.aborted) {
      return;
    }
    s.aborted = true;
    s.reason = reason;
    const algorithms = s.algorithms;
    s.algorithms = [];
    for (let i = 0; i < algorithms.length; i++) {
      algorithms[i]();
    }
    const event = new Event("abort", made(EVENT_INIT, []));
    eventState(event).trusted = true;
    drain(dispatching(signal, event));
    const dependents = s.dependents;
    s.dependents = [];
    for (let i = 0; i < dependents.length; i++) {
      signalAbort(dependents[i], reason);
    }
  }

  // Web IDL's conversion of an iterable to a sequence, through the
  // value's own iterator, as Web IDL says.
  // Web IDL's sequence from an iterable (3.2.18): its values, each
  // converted by `convert`, if given, as the iteration reaches it, so
  // that what converting one does shows in the next. `iterate` is the
  // iterable's @@iterator when the caller has read it already. With no
  // `method`, a failure is the constructor of `iface`'s.
  function sequence(value, method, iface, convert, iterate) {
    const where = method === "" ? "Failed to construct '" + iface + "': " : prefix(method, iface);
    const object = (typeof value === "object" && value !== null) || typeof value === "function";
    if (iterate === undefined && object) {
      iterate = value[iteratorSymbol];
    }
    if (!object || typeof iterate !== "function") {
      throw new $TypeError(where + "The provided value cannot be converted to a sequence.");
    }
    const iterator = ReflectApply(iterate, value, []);
    if ((typeof iterator !== "object" || iterator === null) && typeof iterator !== "function") {
      throw new $TypeError(where + "The iterator is not an object.");
    }
    const next = iterator.next;
    const items = [];
    for (;;) {
      const step = ReflectApply(next, iterator, []);
      if ((typeof step !== "object" || step === null) && typeof step !== "function") {
        throw new $TypeError(where + "The iterator's result is not an object.");
      }
      if (step.done) {
        return items;
      }
      const item = step.value;
      ArrayPush(items, convert === undefined ? item : convert(item));
    }
  }

  // Web IDL's record from an object (3.2.19): its own enumerable
  // properties in order, each key converted by `key` and each value by
  // `value`, a key seen again taking the later value in the earlier
  // place; a Symbol key that is enumerable fails, as ToString does.
  function record(object, key, value) {
    const keys = ReflectOwnKeys(object);
    const out = [];
    const at = new $Map();
    for (let i = 0; i < keys.length; i++) {
      const name = keys[i];
      const desc = ReflectGetOwnPropertyDescriptor(object, name);
      if (desc === undefined || !desc.enumerable) {
        continue;
      }
      const typedKey = key(name);
      const typedValue = value(object[name]);
      if (MapHas(at, typedKey)) {
        out[MapGet(at, typedKey)][1] = typedValue;
      } else {
        MapSet(at, typedKey, out.length);
        ArrayPush(out, [typedKey, typedValue]);
      }
    }
    return out;
  }

  let signalOfController;

  class AbortController {
    #signal;

    static {
      signalOfController = (controller) => {
        if (typeof controller === "object" && controller !== null && #signal in controller) {
          return controller.#signal;
        }
        throw illegalInvocation();
      };
    }

    constructor() {
      this.#signal = new AbortSignal(KEY);
    }

    get signal() {
      return signalOfController(this);
    }

    abort(reason) {
      signalAbort(signalOfController(this), reason === undefined ? abortError() : reason);
    }
  }
  expose(AbortSignal, "AbortSignal");
  expose(AbortController, "AbortController");

  // ---- Bytes and the bodies scripts make (M27, docs/design/fetch.md) ----

  const $Uint8Array = Uint8Array;
  const $ArrayBuffer = ArrayBuffer;
  const $Promise = Promise;
  const DateNow = Date.now;
  const ArraySort = uncurry(Array.prototype.sort);
  const TypedArrayPrototype = Object.getPrototypeOf(Uint8Array.prototype);
  const TypedArraySet = uncurry(TypedArrayPrototype.set);
  const TypedArraySlice = uncurry(TypedArrayPrototype.slice);
  const viewBuffer = uncurry(getOwnPropertyDescriptor(TypedArrayPrototype, "buffer").get);
  const viewOffset = uncurry(getOwnPropertyDescriptor(TypedArrayPrototype, "byteOffset").get);
  const viewLength = uncurry(getOwnPropertyDescriptor(TypedArrayPrototype, "byteLength").get);
  const dataViewBuffer = uncurry(getOwnPropertyDescriptor(DataView.prototype, "buffer").get);
  const dataViewOffset = uncurry(getOwnPropertyDescriptor(DataView.prototype, "byteOffset").get);
  const dataViewLength = uncurry(getOwnPropertyDescriptor(DataView.prototype, "byteLength").get);
  const bufferLength = uncurry(getOwnPropertyDescriptor(ArrayBuffer.prototype, "byteLength").get);
  const ArrayBufferSlice = uncurry(ArrayBuffer.prototype.slice);
  const goneGetter = getOwnPropertyDescriptor(ArrayBuffer.prototype, "detached");
  const bufferGone = goneGetter === undefined ? () => false : uncurry(goneGetter.get);
  const IsView = ArrayBuffer.isView;
  const MathFloor = Math.floor;
  const MathMax = Math.max;
  const MathMin = Math.min;
  const MathTrunc = Math.trunc;

  // A promise of `value`, resolved; one of `error`, rejected.
  function resolved(value) {
    return new $Promise((resolve) => resolve(value));
  }

  function rejected(error) {
    return new $Promise((_, reject) => reject(error));
  }

  // Web IDL's check of a constructor's arguments.
  function needToConstruct(count, required, iface) {
    if (count < required) {
      throw new $TypeError(
        "Failed to construct '" + iface + "': " + required + " argument" +
          (required === 1 ? "" : "s") + " required, but only " + count + " present.",
      );
    }
  }

  // Whether `value` is an ArrayBuffer, by its own internal slot.
  function isBuffer(value) {
    try {
      bufferLength(value);
      return true;
    } catch (e) {
      return false;
    }
  }

  // Whether `value` is a typed array, as against a DataView.
  function isTypedArray(value) {
    try {
      viewLength(value);
      return true;
    } catch (e) {
      return false;
    }
  }

  // A copy of the bytes a BufferSource holds (Web IDL's "get a copy of
  // the buffer source"), or undefined for a value that is none.
  function bufferBytes(value) {
    if (isBuffer(value)) {
      return bufferGone(value) ? new $Uint8Array(0) : new $Uint8Array(ArrayBufferSlice(value, 0));
    }
    if (typeof value === "object" && value !== null && IsView(value)) {
      const typed = isTypedArray(value);
      const buffer = typed ? viewBuffer(value) : dataViewBuffer(value);
      // A detached buffer's copy is empty.
      if (bufferGone(buffer)) {
        return new $Uint8Array(0);
      }
      const offset = typed ? viewOffset(value) : dataViewOffset(value);
      const length = typed ? viewLength(value) : dataViewLength(value);
      return new $Uint8Array(ArrayBufferSlice(buffer, offset, offset + length));
    }
    return undefined;
  }

  // Bytes from a list of numbers, made without the list's iterator.
  function bytesFrom(numbers) {
    const out = new $Uint8Array(numbers.length);
    for (let i = 0; i < numbers.length; i++) {
      out[i] = numbers[i];
    }
    return out;
  }

  // The bytes of `parts`, a list of byte arrays, one after another.
  function concat(parts) {
    let length = 0;
    for (let i = 0; i < parts.length; i++) {
      length += viewLength(parts[i]);
    }
    const out = new $Uint8Array(length);
    let at = 0;
    for (let i = 0; i < parts.length; i++) {
      TypedArraySet(out, parts[i], at);
      at += viewLength(parts[i]);
    }
    return out;
  }

  // UTF-8 encode (Encoding 4.2): a lone surrogate becomes U+FFFD.
  function utf8Encode(text) {
    const out = [];
    for (let i = 0; i < text.length; i++) {
      let c = CharCodeAt(text, i);
      if (c >= 0xd800 && c <= 0xdbff && i + 1 < text.length) {
        const d = CharCodeAt(text, i + 1);
        if (d >= 0xdc00 && d <= 0xdfff) {
          c = 0x10000 + ((c - 0xd800) << 10) + (d - 0xdc00);
          i++;
        } else {
          c = 0xfffd;
        }
      } else if (c >= 0xd800 && c <= 0xdfff) {
        c = 0xfffd;
      }
      if (c < 0x80) {
        ArrayPush(out, c);
      } else if (c < 0x800) {
        ArrayPush(out, 0xc0 | (c >> 6), 0x80 | (c & 0x3f));
      } else if (c < 0x10000) {
        ArrayPush(out, 0xe0 | (c >> 12), 0x80 | ((c >> 6) & 0x3f), 0x80 | (c & 0x3f));
      } else {
        ArrayPush(
          out,
          0xf0 | (c >> 18),
          0x80 | ((c >> 12) & 0x3f),
          0x80 | ((c >> 6) & 0x3f),
          0x80 | (c & 0x3f),
        );
      }
    }
    return bytesFrom(out);
  }

  // UTF-8 decode (Encoding 4.1, the decoder's error mode replacement),
  // a leading byte order mark dropped when `bom` says so.
  function utf8Decode(bytes, bom) {
    const n = viewLength(bytes);
    let i = 0;
    if (bom && n >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) {
      i = 3;
    }
    let out = "";
    let units = [];
    const flush = () => {
      out += ReflectApply(FromCharCode, null, units);
      units = [];
    };
    const push = (c) => {
      if (c >= 0x10000) {
        c -= 0x10000;
        ArrayPush(units, 0xd800 + (c >> 10), 0xdc00 + (c & 0x3ff));
      } else {
        ArrayPush(units, c);
      }
      if (units.length >= 4096) {
        flush();
      }
    };
    let need = 0;
    let seen = 0;
    let point = 0;
    let lower = 0x80;
    let upper = 0xbf;
    for (; i < n; i++) {
      const b = bytes[i];
      if (need === 0) {
        if (b <= 0x7f) {
          push(b);
        } else if (b >= 0xc2 && b <= 0xdf) {
          need = 1;
          point = b & 0x1f;
        } else if (b >= 0xe0 && b <= 0xef) {
          lower = b === 0xe0 ? 0xa0 : 0x80;
          upper = b === 0xed ? 0x9f : 0xbf;
          need = 2;
          point = b & 0xf;
        } else if (b >= 0xf0 && b <= 0xf4) {
          lower = b === 0xf0 ? 0x90 : 0x80;
          upper = b === 0xf4 ? 0x8f : 0xbf;
          need = 3;
          point = b & 0x7;
        } else {
          push(0xfffd);
        }
        continue;
      }
      if (b < lower || b > upper) {
        need = seen = point = 0;
        lower = 0x80;
        upper = 0xbf;
        push(0xfffd);
        i--;
        continue;
      }
      lower = 0x80;
      upper = 0xbf;
      point = (point << 6) | (b & 0x3f);
      seen++;
      if (seen === need) {
        push(point);
        need = seen = point = 0;
      }
    }
    if (need !== 0) {
      push(0xfffd);
    }
    flush();
    return out;
  }

  // `text` with every line ending made the platform's, as a Blob's
  // `native` endings say (File API's "convert line endings to native").
  function nativeEndings(text) {
    const ending = native.platformNewline();
    let out = "";
    for (let i = 0; i < text.length; i++) {
      const c = CharCodeAt(text, i);
      if (c === 0x0d) {
        out += ending;
        if (i + 1 < text.length && CharCodeAt(text, i + 1) === 0x0a) {
          i++;
        }
      } else if (c === 0x0a) {
        out += ending;
      } else {
        out += FromCharCode(c);
      }
    }
    return out;
  }

  // A Blob's type as File API keeps it: lower case, or the empty string
  // when it holds a code point outside U+0020 to U+007E.
  function blobType(type) {
    for (let i = 0; i < type.length; i++) {
      const c = CharCodeAt(type, i);
      if (c < 0x20 || c > 0x7e) {
        return "";
      }
    }
    return ToLowerCase(type);
  }

  // A Blob's or a File's options, read as their dictionaries say.
  function blobOptions(options, iface, file) {
    if (options !== undefined && options !== null && typeof options !== "object" && typeof options !== "function") {
      throw new $TypeError(
        "Failed to construct '" + iface + "': The provided value is not of type '" +
          (file ? "FilePropertyBag" : "BlobPropertyBag") + "'.",
      );
    }
    const get = (name) => (options === undefined || options === null ? undefined : options[name]);
    const endings = get("endings");
    const out = { endings: "transparent", type: "", lastModified: undefined };
    if (endings !== undefined) {
      const value = str(endings);
      if (value !== "transparent" && value !== "native") {
        throw new $TypeError(
          "Failed to construct '" + iface + "': Failed to read the 'endings' property from '" +
            (file ? "FilePropertyBag" : "BlobPropertyBag") + "': The provided value '" + value +
            "' is not a valid enum value of type EndingType.",
        );
      }
      out.endings = value;
    }
    if (file) {
      const lastModified = get("lastModified");
      if (lastModified !== undefined) {
        out.lastModified = $Number(lastModified);
      }
    }
    const type = get("type");
    if (type !== undefined) {
      out.type = blobType(str(type));
    }
    return out;
  }

  // Blob and File (File API, M27): bytes and a type.
  let blobBytes;
  let isBlob;
  // The parts of a Blob or a File (sequence<BlobPart>), each converted
  // as the iteration reaches it: a Blob's bytes, a buffer's, or a
  // USVString.
  function blobParts(parts, iface) {
    if (parts === undefined) {
      return [];
    }
    return sequence(parts, "", iface, (part) => {
      if (isBlob(part)) {
        return blobBytes(part);
      }
      const bytes = bufferBytes(part);
      return bytes !== undefined ? bytes : str(part);
    });
  }

  // The bytes of blobParts' items, each string UTF-8 encoded, its line
  // endings made the platform's if `endings` is native.
  function blobPartBytes(items, endings) {
    const list = [];
    for (let i = 0; i < items.length; i++) {
      const item = items[i];
      if (typeof item === "string") {
        ArrayPush(list, utf8Encode(endings === "native" ? nativeEndings(item) : item));
      } else {
        ArrayPush(list, item);
      }
    }
    return concat(list);
  }

  class Blob {
    #bytes;
    #type;

    constructor(parts = undefined, options = undefined) {
      if (parts === KEY) {
        this.#bytes = options.bytes;
        this.#type = options.type;
        return;
      }
      // Web IDL converts the arguments in order: the parts, each as the
      // iteration reaches it, then the options.
      const items = blobParts(parts, "Blob");
      const read = blobOptions(options, "Blob", false);
      this.#bytes = blobPartBytes(items, read.endings);
      this.#type = read.type;
    }

    static {
      blobBytes = (blob) => blob.#bytes;
      isBlob = (value) => typeof value === "object" && value !== null && #bytes in value;
    }

    get size() {
      if (!isBlob(this)) throw illegalInvocation();
      return viewLength(this.#bytes);
    }

    get type() {
      if (!isBlob(this)) throw illegalInvocation();
      return this.#type;
    }

    slice(start, end, contentType) {
      if (!isBlob(this)) throw illegalInvocation();
      const size = viewLength(this.#bytes);
      const at = (value, fallback) => {
        if (value === undefined) {
          return fallback;
        }
        let n = $Number(value);
        if (n !== n) {
          n = 0;
        } else if (n !== Infinity && n !== -Infinity) {
          const floor = MathFloor(n);
          const rest = n - floor;
          n = rest > 0.5 || (rest === 0.5 && floor % 2 !== 0) ? floor + 1 : floor;
        }
        return n < 0 ? MathMax(size + n, 0) : MathMin(n, size);
      };
      const from = at(start, 0);
      const to = at(end, size);
      const span = MathMax(to - from, 0);
      const type = contentType === undefined ? "" : blobType(str(contentType));
      return new Blob(KEY, {
        bytes: TypedArraySlice(this.#bytes, from, from + span),
        type,
      });
    }

    text() {
      if (!isBlob(this)) return rejected(illegalInvocation());
      return resolved(utf8Decode(this.#bytes, true));
    }

    arrayBuffer() {
      if (!isBlob(this)) return rejected(illegalInvocation());
      return resolved(viewBuffer(TypedArraySlice(this.#bytes)));
    }

    bytes() {
      if (!isBlob(this)) return rejected(illegalInvocation());
      return resolved(TypedArraySlice(this.#bytes));
    }
  }
  expose(Blob, "Blob");

  let isFile;
  class File extends Blob {
    #name;
    #lastModified;

    constructor(bits, name, options = undefined) {
      if (bits === KEY) {
        super(KEY, name);
        this.#name = name.name;
        this.#lastModified = name.lastModified;
        return;
      }
      needToConstruct(arguments.length, 2, "File");
      const items = blobParts(bits, "File");
      const fileName = str(name);
      const read = blobOptions(options, "File", true);
      super(KEY, { bytes: blobPartBytes(items, read.endings), type: read.type });
      this.#name = fileName;
      this.#lastModified =
        read.lastModified === undefined ? DateNow() : MathTrunc(read.lastModified) || 0;
    }

    static {
      isFile = (value) => typeof value === "object" && value !== null && #name in value;
    }

    get name() {
      if (!isFile(this)) throw illegalInvocation();
      return this.#name;
    }

    get lastModified() {
      if (!isFile(this)) throw illegalInvocation();
      return this.#lastModified;
    }
  }
  expose(File, "File");

  // A File of `blob`'s bytes and type named `name`, made by the bindings.
  function fileOf(blob, name, lastModified) {
    return new File(KEY, {
      bytes: blobBytes(blob),
      type: blob.type,
      name,
      lastModified: lastModified === undefined ? DateNow() : lastModified,
    });
  }

  // Web IDL's pair iterators: an object whose `next` reads the pairs
  // its target has at each step, so that changes made while iterating
  // show, its prototype named `iface + " Iterator"`.
  function pairIterator(iface, pairsOf) {
    const prototype = ObjectCreate(Object.getPrototypeOf(Object.getPrototypeOf([][iteratorSymbol]())));
    const states = new $WeakMap();
    defineProperty(prototype, "next", {
      value: function next() {
        const state = WeakMapGet(states, this);
        if (state === undefined) {
          throw new $TypeError("Illegal invocation");
        }
        const pairs = pairsOf(state.target);
        if (state.index >= pairs.length) {
          return { value: undefined, done: true };
        }
        const pair = pairs[state.index++];
        const value =
          state.kind === "key" ? pair[0] : state.kind === "value" ? pair[1] : [pair[0], pair[1]];
        return { value, done: false };
      },
      writable: true,
      enumerable: true,
      configurable: true,
    });
    defineProperty(prototype, toStringTag, { value: iface + " Iterator", configurable: true });
    const make = (target, kind) => {
      const iterator = ObjectCreate(prototype);
      WeakMapSet(states, iterator, { target, kind, index: 0 });
      return iterator;
    };
    return {
      install(cls, brand) {
        const proto = cls.prototype;
        const method = (name, kind) =>
          defineProperty(proto, name, {
            value: {
              [name]() {
                brand(this);
                return make(this, kind);
              },
            }[name],
            writable: true,
            enumerable: true,
            configurable: true,
          });
        method("entries", "key+value");
        method("keys", "key");
        method("values", "value");
        defineProperty(proto, iteratorSymbol, {
          value: proto.entries,
          writable: true,
          configurable: true,
        });
        defineProperty(proto, "forEach", {
          value: function forEach(callback, thisArg) {
            brand(this);
            if (typeof callback !== "function") {
              throw new $TypeError(
                "Failed to execute 'forEach' on '" + iface +
                  "': The callback provided as parameter 1 is not a function.",
              );
            }
            // The pairs read again at each step, as Web IDL's forEach.
            for (let i = 0; ; i++) {
              const pairs = pairsOf(this);
              if (i >= pairs.length) {
                return;
              }
              ReflectApply(callback, thisArg, [pairs[i][1], pairs[i][0], this]);
            }
          },
          writable: true,
          enumerable: true,
          configurable: true,
        });
      },
    };
  }

  // FormData (XMLHttpRequest 4, M27): an entry list of names and values,
  // each a string or a File.
  let entriesOf;
  let isFormData;
  class FormData {
    #entries = [];

    constructor(form = undefined, submitter = undefined) {
      if (form !== undefined) {
        if (!isNode(form)) {
          throw new $TypeError(
            "Failed to construct 'FormData': parameter 1 is not of type 'HTMLFormElement'.",
          );
        }
        const by =
          submitter === undefined || submitter === null ? null : isNode(submitter) ? handleOf(submitter) : -1;
        if (by === -1) {
          throw new $TypeError(
            "Failed to construct 'FormData': parameter 2 is not of type 'HTMLElement'.",
          );
        }
        let list;
        try {
          list = native.formEntries(handleOf(form), by);
        } catch (e) {
          throw failedConstruct(e, "FormData");
        }
        for (let i = 0; i < list.length; i += 3) {
          const value = list[i + 1] === "file" ? fileOf(new Blob(KEY, { bytes: new $Uint8Array(0), type: "application/octet-stream" }), "") : list[i + 2];
          ArrayPush(this.#entries, [list[i], value]);
        }
      }
    }

    static {
      entriesOf = (data) => data.#entries;
      isFormData = (value) => typeof value === "object" && value !== null && #entries in value;
    }

    append(name, value, filename) {
      formData(this);
      need(arguments.length, 2, "append", "FormData");
      ArrayPush(this.#entries, entry(name, value, filename, arguments.length, "append"));
    }

    delete(name) {
      formData(this);
      need(arguments.length, 1, "delete", "FormData");
      const wanted = str(name);
      const kept = [];
      for (let i = 0; i < this.#entries.length; i++) {
        if (this.#entries[i][0] !== wanted) {
          ArrayPush(kept, this.#entries[i]);
        }
      }
      this.#entries = kept;
    }

    get(name) {
      formData(this);
      need(arguments.length, 1, "get", "FormData");
      const wanted = str(name);
      for (let i = 0; i < this.#entries.length; i++) {
        if (this.#entries[i][0] === wanted) {
          return this.#entries[i][1];
        }
      }
      return null;
    }

    getAll(name) {
      formData(this);
      need(arguments.length, 1, "getAll", "FormData");
      const wanted = str(name);
      const out = [];
      for (let i = 0; i < this.#entries.length; i++) {
        if (this.#entries[i][0] === wanted) {
          ArrayPush(out, this.#entries[i][1]);
        }
      }
      return out;
    }

    has(name) {
      formData(this);
      need(arguments.length, 1, "has", "FormData");
      const wanted = str(name);
      for (let i = 0; i < this.#entries.length; i++) {
        if (this.#entries[i][0] === wanted) {
          return true;
        }
      }
      return false;
    }

    set(name, value, filename) {
      formData(this);
      need(arguments.length, 2, "set", "FormData");
      const made = entry(name, value, filename, arguments.length, "set");
      const kept = [];
      let placed = false;
      for (let i = 0; i < this.#entries.length; i++) {
        if (this.#entries[i][0] !== made[0]) {
          ArrayPush(kept, this.#entries[i]);
        } else if (!placed) {
          ArrayPush(kept, made);
          placed = true;
        }
      }
      if (!placed) {
        ArrayPush(kept, made);
      }
      this.#entries = kept;
    }
  }
  pairIterator("FormData", (data) => entriesOf(data)).install(FormData, (value) => {
    if (!isFormData(value)) throw illegalInvocation();
  });
  expose(FormData, "FormData");

  function formData(value) {
    if (!isFormData(value)) throw illegalInvocation();
  }

  // An entry of a FormData (XMLHttpRequest's "create an entry"): a Blob
  // value made a File named `filename`, or "blob" for one that is not a
  // File; a File renamed by `filename`; anything else a string.
  function entry(name, value, filename, count, method) {
    const key = str(name);
    if (isBlob(value)) {
      if (!isFile(value) || count > 2) {
        const named = count > 2 ? str(filename) : isFile(value) ? value.name : "blob";
        return [key, fileOf(value, named, isFile(value) ? value.lastModified : undefined)];
      }
      return [key, value];
    }
    if (count > 2) {
      throw new $TypeError(
        "Failed to execute '" + method + "' on 'FormData': parameter 2 is not of type 'Blob'.",
      );
    }
    return [key, str(value)];
  }

  // The exception a native's failure is in a constructor:
  // "Failed to construct 'I': reason".
  function failedConstruct(error, iface) {
    const words = "Failed to construct '" + iface + "': ";
    if (error instanceof $DOMException) {
      return new $DOMException(words + error.message, error.name);
    }
    if (error instanceof $TypeError || error instanceof $RangeError) {
      error.message = words + error.message;
    }
    return error;
  }

  // application/x-www-form-urlencoded (URL Standard 5): the parser, of a
  // string's UTF-8 bytes, and the serializer.
  function hexValue(c) {
    if (c >= 0x30 && c <= 0x39) return c - 0x30;
    if (c >= 0x41 && c <= 0x46) return c - 0x37;
    if (c >= 0x61 && c <= 0x66) return c - 0x57;
    return -1;
  }

  function percentDecode(bytes, from, to) {
    const out = [];
    for (let i = from; i < to; i++) {
      const b = bytes[i];
      if (b === 0x2b) {
        ArrayPush(out, 0x20);
      } else if (
        b === 0x25 && i + 2 < to && hexValue(bytes[i + 1]) >= 0 && hexValue(bytes[i + 2]) >= 0
      ) {
        ArrayPush(out, hexValue(bytes[i + 1]) * 16 + hexValue(bytes[i + 2]));
        i += 2;
      } else {
        ArrayPush(out, b);
      }
    }
    return utf8Decode(bytesFrom(out), false);
  }

  function formParse(text) {
    return formParseBytes(utf8Encode(text));
  }

  function formParseBytes(bytes) {
    const n = viewLength(bytes);
    const pairs = [];
    let start = 0;
    for (let i = 0; i <= n; i++) {
      if (i < n && bytes[i] !== 0x26) {
        continue;
      }
      if (i > start) {
        let equals = i;
        for (let j = start; j < i; j++) {
          if (bytes[j] === 0x3d) {
            equals = j;
            break;
          }
        }
        const name = percentDecode(bytes, start, equals);
        const value = equals < i ? percentDecode(bytes, equals + 1, i) : "";
        ArrayPush(pairs, [name, value]);
      }
      start = i + 1;
    }
    return pairs;
  }

  const HEX = "0123456789ABCDEF";

  function formEncode(text) {
    const bytes = utf8Encode(text);
    let out = "";
    for (let i = 0; i < viewLength(bytes); i++) {
      const b = bytes[i];
      if (
        (b >= 0x30 && b <= 0x39) || (b >= 0x41 && b <= 0x5a) || (b >= 0x61 && b <= 0x7a) ||
        b === 0x2a || b === 0x2d || b === 0x2e || b === 0x5f
      ) {
        out += FromCharCode(b);
      } else if (b === 0x20) {
        out += "+";
      } else {
        out += "%" + HEX[b >> 4] + HEX[b & 15];
      }
    }
    return out;
  }

  function formSerialize(pairs) {
    let out = "";
    for (let i = 0; i < pairs.length; i++) {
      if (i > 0) out += "&";
      out += formEncode(pairs[i][0]) + "=" + formEncode(pairs[i][1]);
    }
    return out;
  }

  // URLSearchParams (URL Standard 6, M27): a list of names and values.
  let paramsOf;
  let isParams;
  class URLSearchParams {
    #list = [];

    constructor(init) {
      if (init === undefined || init === null) {
        if (init === null) this.#list = formParse("null");
        return;
      }
      if (typeof init === "object" || typeof init === "function") {
        const iterate = init[iteratorSymbol];
        if (iterate !== undefined && iterate !== null) {
          this.#list = sequence(init, "", "URLSearchParams", (item) => {
            const pair = sequence(item, "", "URLSearchParams", str);
            if (pair.length !== 2) {
              throw new $TypeError(
                "Failed to construct 'URLSearchParams': Sequence initializer must only contain pair elements",
              );
            }
            return pair;
          }, iterate);
          return;
        }
        this.#list = record(init, str, str);
        return;
      }
      let text = str(init);
      if (CharCodeAt(text, 0) === 0x3f) {
        text = StringSlice(text, 1);
      }
      this.#list = formParse(text);
    }

    static {
      paramsOf = (params) => params.#list;
      isParams = (value) => typeof value === "object" && value !== null && #list in value;
    }

    get size() {
      return this.#list.length;
    }

    append(name, value) {
      need(arguments.length, 2, "append", "URLSearchParams");
      ArrayPush(this.#list, [str(name), str(value)]);
    }

    delete(name, value) {
      need(arguments.length, 1, "delete", "URLSearchParams");
      const wanted = str(name);
      const also = value === undefined ? undefined : str(value);
      const kept = [];
      for (let i = 0; i < this.#list.length; i++) {
        const pair = this.#list[i];
        if (pair[0] !== wanted || (also !== undefined && pair[1] !== also)) {
          ArrayPush(kept, pair);
        }
      }
      this.#list = kept;
    }

    get(name) {
      need(arguments.length, 1, "get", "URLSearchParams");
      const wanted = str(name);
      for (let i = 0; i < this.#list.length; i++) {
        if (this.#list[i][0] === wanted) return this.#list[i][1];
      }
      return null;
    }

    getAll(name) {
      need(arguments.length, 1, "getAll", "URLSearchParams");
      const wanted = str(name);
      const out = [];
      for (let i = 0; i < this.#list.length; i++) {
        if (this.#list[i][0] === wanted) ArrayPush(out, this.#list[i][1]);
      }
      return out;
    }

    has(name, value) {
      need(arguments.length, 1, "has", "URLSearchParams");
      const wanted = str(name);
      const also = value === undefined ? undefined : str(value);
      for (let i = 0; i < this.#list.length; i++) {
        const pair = this.#list[i];
        if (pair[0] === wanted && (also === undefined || pair[1] === also)) return true;
      }
      return false;
    }

    set(name, value) {
      need(arguments.length, 2, "set", "URLSearchParams");
      const pair = [str(name), str(value)];
      const kept = [];
      let placed = false;
      for (let i = 0; i < this.#list.length; i++) {
        if (this.#list[i][0] !== pair[0]) {
          ArrayPush(kept, this.#list[i]);
        } else if (!placed) {
          ArrayPush(kept, pair);
          placed = true;
        }
      }
      if (!placed) ArrayPush(kept, pair);
      this.#list = kept;
    }

    sort() {
      // Stable, by the names' code units.
      ArraySort(this.#list, (a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
    }

    toString() {
      return formSerialize(this.#list);
    }
  }
  pairIterator("URLSearchParams", (params) => paramsOf(params)).install(URLSearchParams, (value) => {
    if (!isParams(value)) throw illegalInvocation();
  });
  expose(URLSearchParams, "URLSearchParams");

  // ---- Fetch: Headers, Request, Response, fetch() (M27.1) ----

  const JSONParse = JSON.parse;
  const JSONStringify = JSON.stringify;
  const MathRandom = Math.random;

  // A signal that aborts when `signal` does (a request's, which follows
  // the one it was given, as AbortSignal.any's would).
  function followingSignal(signal) {
    const result = new AbortSignal(KEY);
    const source = signalOf(signal);
    if (source.aborted) {
      const own = signalOf(result);
      own.aborted = true;
      own.reason = source.reason;
      return result;
    }
    ArrayPush(source.dependents, result);
    return result;
  }

  // A ByteString (Web IDL): a string of code points up to U+00FF.
  function byteString(value, method, iface) {
    const text = toString(value);
    for (let i = 0; i < text.length; i++) {
      if (CharCodeAt(text, i) > 0xff) {
        throw new $TypeError(
          prefix(method, iface) + "String contains non ISO-8859-1 code point.",
        );
      }
    }
    return text;
  }

  // An HTTP token code point.
  function isTokenCode(c) {
    return (
      (c >= 0x30 && c <= 0x39) || (c >= 0x41 && c <= 0x5a) || (c >= 0x61 && c <= 0x7a) ||
      c === 0x21 || (c >= 0x23 && c <= 0x27) || c === 0x2a || c === 0x2b || c === 0x2d ||
      c === 0x2e || c === 0x5e || c === 0x5f || c === 0x60 || c === 0x7c || c === 0x7e
    );
  }

  function isToken(text) {
    if (text.length === 0) return false;
    for (let i = 0; i < text.length; i++) {
      if (!isTokenCode(CharCodeAt(text, i))) return false;
    }
    return true;
  }

  // A header value normalized (Fetch 2.2.2): HTTP whitespace off both ends.
  function normalizeValue(value) {
    const space = (c) => c === 0x09 || c === 0x0a || c === 0x0d || c === 0x20;
    let start = 0;
    let end = value.length;
    while (start < end && space(CharCodeAt(value, start))) start++;
    while (end > start && space(CharCodeAt(value, end - 1))) end--;
    return StringSlice(value, start, end);
  }

  function isHeaderValue(value) {
    for (let i = 0; i < value.length; i++) {
      const c = CharCodeAt(value, i);
      if (c === 0 || c === 0x0a || c === 0x0d) return false;
    }
    return true;
  }

  const FORBIDDEN_REQUEST = [
    "accept-charset", "accept-encoding", "access-control-request-headers",
    "access-control-request-method", "connection", "content-length", "cookie", "cookie2",
    "date", "dnt", "expect", "host", "keep-alive", "origin", "referer", "set-cookie", "te",
    "trailer", "transfer-encoding", "upgrade", "via",
  ];

  function within(list, value) {
    for (let i = 0; i < list.length; i++) {
      if (list[i] === value) return true;
    }
    return false;
  }

  function isForbiddenMethod(method) {
    const upper = ToUpperCase(method);
    return upper === "CONNECT" || upper === "TRACE" || upper === "TRACK";
  }

  // Fetch 2.2.2's forbidden request-header; the engine checks again.
  function isForbiddenRequestHeader(name, value) {
    const lower = ToLowerCase(name);
    if (within(FORBIDDEN_REQUEST, lower) || StringSlice(lower, 0, 6) === "proxy-" || StringSlice(lower, 0, 4) === "sec-") {
      return true;
    }
    if (lower === "x-http-method" || lower === "x-http-method-override" || lower === "x-method-override") {
      const methods = splitList(value);
      for (let i = 0; i < methods.length; i++) {
        if (isForbiddenMethod(methods[i])) return true;
      }
    }
    return false;
  }

  // A header value split at its commas, each piece trimmed of spaces and
  // tabs (Fetch's get, decode, and split, quotes aside).
  function splitList(value) {
    const out = [];
    let start = 0;
    for (let i = 0; i <= value.length; i++) {
      if (i === value.length || CharCodeAt(value, i) === 0x2c) {
        let piece = StringSlice(value, start, i);
        let a = 0;
        let b = piece.length;
        while (a < b && (CharCodeAt(piece, a) === 0x20 || CharCodeAt(piece, a) === 0x09)) a++;
        while (b > a && (CharCodeAt(piece, b - 1) === 0x20 || CharCodeAt(piece, b - 1) === 0x09)) b--;
        ArrayPush(out, StringSlice(piece, a, b));
        start = i + 1;
      }
    }
    return out;
  }

  function isCorsUnsafeCode(c) {
    return (
      (c < 0x20 && c !== 0x09) || c === 0x22 || c === 0x28 || c === 0x29 || c === 0x3a ||
      c === 0x3c || c === 0x3e || c === 0x3f || c === 0x40 || c === 0x5b || c === 0x5c ||
      c === 0x5d || c === 0x7b || c === 0x7d || c === 0x7f
    );
  }

  // A MIME type's essence, as far as safelisting reads it: what comes
  // before its parameters, trimmed and lower case.
  function essenceOf(value) {
    const semicolon = StringIndexOf(value, ";");
    return ToLowerCase(normalizeValue(semicolon < 0 ? value : StringSlice(value, 0, semicolon)));
  }

  // Fetch 2.2.2's no-CORS-safelisted request-header.
  function isNoCorsSafelisted(name, value) {
    if (value.length > 128) return false;
    const lower = ToLowerCase(name);
    if (lower === "accept") {
      for (let i = 0; i < value.length; i++) {
        if (isCorsUnsafeCode(CharCodeAt(value, i))) return false;
      }
      return true;
    }
    if (lower === "accept-language" || lower === "content-language") {
      for (let i = 0; i < value.length; i++) {
        const c = CharCodeAt(value, i);
        const ok =
          (c >= 0x30 && c <= 0x39) || (c >= 0x41 && c <= 0x5a) || (c >= 0x61 && c <= 0x7a) ||
          c === 0x20 || c === 0x2a || c === 0x2c || c === 0x2d || c === 0x2e || c === 0x3b || c === 0x3d;
        if (!ok) return false;
      }
      return true;
    }
    if (lower === "content-type") {
      for (let i = 0; i < value.length; i++) {
        if (isCorsUnsafeCode(CharCodeAt(value, i))) return false;
      }
      const essence = essenceOf(value);
      return (
        essence === "application/x-www-form-urlencoded" || essence === "multipart/form-data" ||
        essence === "text/plain"
      );
    }
    return false;
  }

  // Headers (Fetch 5.1): a header list and its guard.
  let headersState;
  let isHeaders;
  class Headers {
    #list = [];
    #guard = "none";

    constructor(init) {
      if (init === KEY) {
        return;
      }
      if (init !== undefined) {
        fillHeaders(this, init, "Headers", true);
      }
    }

    static {
      headersState = (headers) => ({
        get list() {
          return headers.#list;
        },
        set list(value) {
          headers.#list = value;
        },
        get guard() {
          return headers.#guard;
        },
        set guard(value) {
          headers.#guard = value;
        },
      });
      isHeaders = (value) => typeof value === "object" && value !== null && #list in value;
    }

    append(name, value) {
      need(arguments.length, 2, "append", "Headers");
      appendHeader(this, byteString(name, "append", "Headers"), byteString(value, "append", "Headers"), "append");
    }

    delete(name) {
      need(arguments.length, 1, "delete", "Headers");
      const key = byteString(name, "delete", "Headers");
      if (!validHeader(this, key, "", "delete")) return;
      const lower = ToLowerCase(key);
      if (this.#guard === "request-no-cors" && !isNoCorsSafelisted(key, "") &&
        lower !== "accept" && lower !== "accept-language" && lower !== "content-language" &&
        lower !== "content-type" && lower !== "range") {
        return;
      }
      const kept = [];
      for (let i = 0; i < this.#list.length; i++) {
        if (ToLowerCase(this.#list[i][0]) !== lower) ArrayPush(kept, this.#list[i]);
      }
      this.#list = kept;
      if (this.#guard === "request-no-cors") removePrivileged(this);
    }

    get(name) {
      need(arguments.length, 1, "get", "Headers");
      const key = byteString(name, "get", "Headers");
      if (!isToken(key)) {
        throw new $TypeError(prefix("get", "Headers") + "Invalid name");
      }
      return combined(this.#list, ToLowerCase(key));
    }

    getSetCookie() {
      const out = [];
      for (let i = 0; i < this.#list.length; i++) {
        if (ToLowerCase(this.#list[i][0]) === "set-cookie") ArrayPush(out, this.#list[i][1]);
      }
      return out;
    }

    has(name) {
      need(arguments.length, 1, "has", "Headers");
      const key = byteString(name, "has", "Headers");
      if (!isToken(key)) {
        throw new $TypeError(prefix("has", "Headers") + "Invalid name");
      }
      return combined(this.#list, ToLowerCase(key)) !== null;
    }

    set(name, value) {
      need(arguments.length, 2, "set", "Headers");
      const key = byteString(name, "set", "Headers");
      const normal = normalizeValue(byteString(value, "set", "Headers"));
      if (!validHeader(this, key, normal, "set")) return;
      if (this.#guard === "request-no-cors" && !isNoCorsSafelisted(key, normal)) return;
      const lower = ToLowerCase(key);
      const kept = [];
      let placed = false;
      for (let i = 0; i < this.#list.length; i++) {
        if (ToLowerCase(this.#list[i][0]) !== lower) {
          ArrayPush(kept, this.#list[i]);
        } else if (!placed) {
          ArrayPush(kept, [this.#list[i][0], normal]);
          placed = true;
        }
      }
      if (!placed) ArrayPush(kept, [key, normal]);
      this.#list = kept;
      if (this.#guard === "request-no-cors") removePrivileged(this);
    }
  }

  // A header list's combined value of `lower`, or null.
  function combined(list, lower) {
    let out = null;
    for (let i = 0; i < list.length; i++) {
      if (ToLowerCase(list[i][0]) === lower) {
        out = out === null ? list[i][1] : out + ", " + list[i][1];
      }
    }
    return out;
  }

  // Fetch's "validate" of a header for `headers`: a TypeError for a name
  // or value that is not one, or immutable headers; false for one the
  // guard leaves out.
  function validHeader(headers, name, value, method) {
    if (!isToken(name)) {
      throw new $TypeError(prefix(method, "Headers") + "Invalid name");
    }
    if (!isHeaderValue(value)) {
      throw new $TypeError(prefix(method, "Headers") + "Invalid value");
    }
    const state = headersState(headers);
    if (state.guard === "immutable") {
      throw new $TypeError(prefix(method, "Headers") + "Headers are immutable");
    }
    if (state.guard === "request" && isForbiddenRequestHeader(name, value)) return false;
    if (state.guard === "response") {
      const lower = ToLowerCase(name);
      if (lower === "set-cookie" || lower === "set-cookie2") return false;
    }
    return true;
  }

  // Fetch's "append" to a Headers.
  function appendHeader(headers, name, value, method) {
    const normal = normalizeValue(value);
    if (!validHeader(headers, name, normal, method)) return;
    const state = headersState(headers);
    if (state.guard === "request-no-cors") {
      const before = combined(state.list, ToLowerCase(name));
      const temporary = before === null ? normal : before + ", " + normal;
      if (!isNoCorsSafelisted(name, temporary)) return;
    }
    ArrayPush(state.list, [name, normal]);
    if (state.guard === "request-no-cors") removePrivileged(headers);
  }

  function removePrivileged(headers) {
    const state = headersState(headers);
    const kept = [];
    for (let i = 0; i < state.list.length; i++) {
      if (ToLowerCase(state.list[i][0]) !== "range") ArrayPush(kept, state.list[i]);
    }
    state.list = kept;
  }

  // Fetch's "fill" of a Headers from a HeadersInit: pairs, a record, or
  // another Headers.
  function fillHeaders(headers, init, iface, constructing) {
    const fail = (message) =>
      new $TypeError(
        (constructing ? "Failed to construct '" + iface + "': " : "") + message,
      );
    if (typeof init !== "object" && typeof init !== "function") {
      throw fail("The provided value is not of type '(record<ByteString, ByteString> or sequence<sequence<ByteString>>)'.");
    }
    if (init === null) {
      throw fail("The provided value is not of type '(record<ByteString, ByteString> or sequence<sequence<ByteString>>)'.");
    }
    const toByteString = (value) => byteString(value, "append", "Headers");
    const iterate = init[iteratorSymbol];
    let pairs;
    if (iterate !== undefined && iterate !== null) {
      pairs = sequence(init, "", iface, (item) => {
        const pair = sequence(item, "", iface, toByteString);
        if (pair.length !== 2) {
          throw fail("Invalid value");
        }
        return pair;
      }, iterate);
    } else {
      pairs = record(init, toByteString, toByteString);
    }
    for (let i = 0; i < pairs.length; i++) {
      appendHeader(headers, pairs[i][0], pairs[i][1], "append");
    }
  }

  // Headers made by the bindings, with `guard`.
  function makeHeaders(guard, list) {
    const headers = new Headers(KEY);
    const state = headersState(headers);
    state.guard = guard;
    if (list !== undefined) state.list = list;
    return headers;
  }

  // Fetch 2.2.2's sort and combine, what Headers iterates.
  function sortAndCombine(list) {
    const names = [];
    for (let i = 0; i < list.length; i++) {
      const lower = ToLowerCase(list[i][0]);
      if (!within(names, lower)) ArrayPush(names, lower);
    }
    ArraySort(names, (a, b) => (a < b ? -1 : a > b ? 1 : 0));
    const out = [];
    for (let i = 0; i < names.length; i++) {
      if (names[i] === "set-cookie") {
        for (let j = 0; j < list.length; j++) {
          if (ToLowerCase(list[j][0]) === "set-cookie") ArrayPush(out, ["set-cookie", list[j][1]]);
        }
      } else {
        ArrayPush(out, [names[i], combined(list, names[i])]);
      }
    }
    return out;
  }
  pairIterator("Headers", (headers) => sortAndCombine(headersState(headers).list)).install(Headers, (value) => {
    if (!isHeaders(value)) throw illegalInvocation();
  });
  expose(Headers, "Headers");

  // The body of a Request or a Response: its bytes, or null for none,
  // and whether it was read.
  function bodyFrom(bytes) {
    return { bytes, used: false };
  }

  // Fetch's "extract" of a BodyInit (5.2): its bytes and the type it
  // gives, null for a buffer or a Blob of none.
  function extract(value, iface) {
    if (isBlob(value)) {
      const type = value.type;
      return { bytes: TypedArraySlice(blobBytes(value)), type: type === "" ? null : type };
    }
    const bytes = bufferBytes(value);
    if (bytes !== undefined) {
      return { bytes, type: null };
    }
    if (isFormData(value)) {
      const encoded = multipartEncode(entriesOf(value));
      return { bytes: encoded.bytes, type: "multipart/form-data; boundary=" + encoded.boundary };
    }
    if (isParams(value)) {
      return {
        bytes: utf8Encode(formSerialize(paramsOf(value))),
        type: "application/x-www-form-urlencoded;charset=UTF-8",
      };
    }
    return { bytes: utf8Encode(str(value)), type: "text/plain;charset=UTF-8" };
  }

  // multipart/form-data (HTML 4.10.22.8): each entry a part, names and
  // file names escaped as HTML says, a boundary of the bindings.
  function multipartEncode(entries) {
    let boundary = "----SilverFormBoundary";
    const letters = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    for (let i = 0; i < 16; i++) {
      boundary += letters[MathFloor(MathRandom() * letters.length)];
    }
    const escape = (text) => {
      let out = "";
      for (let i = 0; i < text.length; i++) {
        const c = CharCodeAt(text, i);
        out += c === 0x0a ? "%0A" : c === 0x0d ? "%0D" : c === 0x22 ? "%22" : FromCharCode(c);
      }
      return out;
    };
    const parts = [];
    for (let i = 0; i < entries.length; i++) {
      const name = entries[i][0];
      const value = entries[i][1];
      let head = "--" + boundary + "\r\nContent-Disposition: form-data; name=\"" + escape(name) + "\"";
      if (typeof value === "string") {
        ArrayPush(parts, utf8Encode(head + "\r\n\r\n" + value + "\r\n"));
      } else {
        const type = value.type === "" ? "application/octet-stream" : value.type;
        head += "; filename=\"" + escape(value.name) + "\"\r\nContent-Type: " + type + "\r\n\r\n";
        ArrayPush(parts, utf8Encode(head), blobBytes(value), utf8Encode("\r\n"));
      }
    }
    ArrayPush(parts, utf8Encode("--" + boundary + "--\r\n"));
    return { bytes: concat(parts), boundary };
  }

  // A multipart/form-data body parsed (RFC 7578) into entries, or null
  // when it is not one.
  function multipartParse(bytes, boundary) {
    const delimiter = utf8Encode("--" + boundary);
    const n = viewLength(bytes);
    const at = (i, pattern) => {
      if (i + viewLength(pattern) > n) return false;
      for (let j = 0; j < viewLength(pattern); j++) {
        if (bytes[i + j] !== pattern[j]) return false;
      }
      return true;
    };
    const find = (from, pattern) => {
      for (let i = from; i + viewLength(pattern) <= n; i++) {
        if (at(i, pattern)) return i;
      }
      return -1;
    };
    const crlf = utf8Encode("\r\n");
    const next = utf8Encode("\r\n--" + boundary);
    const entries = [];
    if (!at(0, delimiter)) return null;
    let i = viewLength(delimiter);
    // Transport padding (RFC 2046): spaces and tabs after a delimiter.
    const padding = (from) => {
      while (from < n && (bytes[from] === 0x20 || bytes[from] === 0x09)) from++;
      return from;
    };
    for (;;) {
      if (at(i, utf8Encode("--"))) {
        // The close delimiter, then the end or a line break before any
        // epilogue.
        const end = padding(i + 2);
        return end === n || at(end, crlf) ? entries : null;
      }
      i = padding(i);
      if (!at(i, crlf)) return null;
      i += 2;
      const headEnd = find(i, utf8Encode("\r\n\r\n"));
      if (headEnd < 0) return null;
      const head = utf8Decode(TypedArraySlice(bytes, i, headEnd), false);
      const bodyEnd = find(headEnd + 4, next);
      if (bodyEnd < 0) return null;
      const body = TypedArraySlice(bytes, headEnd + 4, bodyEnd);
      let name = null;
      let filename = null;
      let type = "text/plain";
      const lines = splitLines(head);
      for (let k = 0; k < lines.length; k++) {
        const colon = StringIndexOf(lines[k], ":");
        if (colon < 0) continue;
        const field = ToLowerCase(normalizeValue(StringSlice(lines[k], 0, colon)));
        const value = normalizeValue(StringSlice(lines[k], colon + 1));
        if (field === "content-disposition") {
          name = parameter(value, "name");
          filename = parameter(value, "filename");
        } else if (field === "content-type") {
          type = value;
        }
      }
      if (name === null) return null;
      if (filename === null) {
        ArrayPush(entries, [name, utf8Decode(body, false)]);
      } else {
        ArrayPush(entries, [name, fileOf(new Blob(KEY, { bytes: body, type: blobType(type) }), filename)]);
      }
      i = bodyEnd + viewLength(next);
    }
  }

  function splitLines(text) {
    const out = [];
    let start = 0;
    for (let i = 0; i + 1 <= text.length; i++) {
      if (CharCodeAt(text, i) === 0x0d && CharCodeAt(text, i + 1) === 0x0a) {
        ArrayPush(out, StringSlice(text, start, i));
        start = i + 2;
        i++;
      }
    }
    ArrayPush(out, StringSlice(text, start));
    return out;
  }

  // A parameter's value in a Content-Disposition, quoted or not.
  function parameter(value, wanted) {
    const pieces = splitParameters(value);
    for (let i = 1; i < pieces.length; i++) {
      const equals = StringIndexOf(pieces[i], "=");
      if (equals < 0) continue;
      if (ToLowerCase(normalizeValue(StringSlice(pieces[i], 0, equals))) !== wanted) continue;
      let v = normalizeValue(StringSlice(pieces[i], equals + 1));
      if (v.length >= 2 && CharCodeAt(v, 0) === 0x22 && CharCodeAt(v, v.length - 1) === 0x22) {
        v = StringSlice(v, 1, v.length - 1);
      }
      return v;
    }
    return null;
  }

  function splitParameters(value) {
    const out = [];
    let start = 0;
    let quoted = false;
    for (let i = 0; i <= value.length; i++) {
      const c = i < value.length ? CharCodeAt(value, i) : -1;
      if (c === 0x22) quoted = !quoted;
      if (c === -1 || (c === 0x3b && !quoted)) {
        ArrayPush(out, StringSlice(value, start, i));
        start = i + 1;
      }
    }
    return out;
  }

  // The Body mixin (Fetch 5.3) on `cls`, whose bodies `bodyState` gives
  // and whose headers `headersOf` gives.
  function installBody(cls, iface, brand, bodyState, headersOf) {
    const consume = (object, method, convert) => {
      if (!brand(object)) return rejected(illegalInvocation());
      const body = bodyState(object);
      if (body !== null && body.used) {
        return rejected(new $TypeError(prefix(method, iface) + "body stream already read"));
      }
      let bytes = new $Uint8Array(0);
      if (body !== null) {
        body.used = true;
        if (body.bytes !== null) bytes = body.bytes;
      }
      try {
        return resolved(convert(bytes, object));
      } catch (e) {
        return rejected(e);
      }
    };
    const members = {
      get bodyUsed() {
        if (!brand(this)) throw illegalInvocation();
        const body = bodyState(this);
        return body !== null && body.bytes !== null && body.used;
      },
      arrayBuffer() {
        return consume(this, "arrayBuffer", (bytes) => viewBuffer(TypedArraySlice(bytes)));
      },
      blob() {
        return consume(this, "blob", (bytes, object) => {
          const type = combined(headersState(headersOf(object)).list, "content-type");
          return new Blob(KEY, { bytes: TypedArraySlice(bytes), type: type === null ? "" : blobType(type) });
        });
      },
      bytes() {
        return consume(this, "bytes", (bytes) => TypedArraySlice(bytes));
      },
      formData() {
        return consume(this, "formData", (bytes, object) => {
          const type = combined(headersState(headersOf(object)).list, "content-type") || "";
          const essence = essenceOf(type);
          const data = new FormData();
          let entries = null;
          if (essence === "multipart/form-data") {
            const boundary = parameter(";" + StringSlice(type, StringIndexOf(type, ";") + 1), "boundary");
            entries = boundary === null ? null : multipartParse(bytes, boundary);
          } else if (essence === "application/x-www-form-urlencoded") {
            entries = formParseBytes(bytes);
          }
          if (entries === null) {
            throw new $TypeError(prefix("formData", iface) + "Failed to parse body as FormData.");
          }
          for (let i = 0; i < entries.length; i++) ArrayPush(entriesOf(data), entries[i]);
          return data;
        });
      },
      json() {
        return consume(this, "json", (bytes) => JSONParse(utf8Decode(bytes, true)));
      },
      text() {
        return consume(this, "text", (bytes) => utf8Decode(bytes, true));
      },
    };
    const names = getOwnPropertyNames(members);
    for (let i = 0; i < names.length; i++) {
      defineProperty(cls.prototype, names[i], getOwnPropertyDescriptor(members, names[i]));
    }
  }

  const REQUEST_MODES = ["same-origin", "no-cors", "cors", "navigate"];
  const REQUEST_CREDENTIALS = ["omit", "same-origin", "include"];
  const REQUEST_CACHES = ["default", "no-store", "reload", "no-cache", "force-cache", "only-if-cached"];
  // A request's priority: checked, as Fetch's IDL has it, and kept by
  // nothing, the engine having no priorities to give.
  const REQUEST_PRIORITIES = ["high", "low", "auto"];
  const REQUEST_REDIRECTS = ["follow", "error", "manual"];
  const ADDRESS_SPACES = ["public", "local", "loopback"];
  const REFERRER_POLICIES = [
    "", "no-referrer", "no-referrer-when-downgrade", "same-origin", "origin",
    "strict-origin", "origin-when-cross-origin", "strict-origin-when-cross-origin", "unsafe-url",
  ];

  // An enum member of `init`, checked.
  function enumMember(init, name, values, type) {
    const value = init[name];
    if (value === undefined) return undefined;
    const text = str(value);
    if (!within(values, text)) {
      throw new $TypeError(
        "Failed to construct 'Request': Failed to read the '" + name + "' property from 'RequestInit': The provided value '" +
          text + "' is not a valid enum value of type " + type + ".",
      );
    }
    return text;
  }

  // Request (Fetch 5.4).
  let requestState;
  let isRequest;
  class Request {
    #state;

    constructor(input, init) {
      if (input === KEY) {
        this.#state = init;
        return;
      }
      needToConstruct(arguments.length, 1, "Request");
      const fail = (message) => new $TypeError("Failed to construct 'Request': " + message);
      if (init !== undefined && init !== null && typeof init !== "object" && typeof init !== "function") {
        throw fail("The provided value is not of type 'RequestInit'.");
      }
      const given = init === undefined || init === null ? {} : init;
      let state;
      let fallbackMode = null;
      let signal = null;
      const inputIsRequest = isRequest(input);
      if (!inputIsRequest) {
        const text = str(input);
        const url = native.resolveUrl(text);
        if (url === null) {
          throw fail("Failed to parse URL from " + text);
        }
        if (native.urlHasCredentials(url)) {
          throw fail("Request cannot be constructed from a URL that includes credentials: " + text);
        }
        state = {
          method: "GET", url, headers: null, body: null, mode: "cors", credentials: "same-origin",
          cache: "default", redirect: "follow", referrer: "about:client", referrerPolicy: "",
          integrity: "", keepalive: false, duplex: "half", targetAddressSpace: "",
        };
        fallbackMode = "cors";
      } else {
        const from = input.#state;
        state = {
          method: from.method, url: from.url, headers: null, body: from.body, mode: from.mode,
          credentials: from.credentials, cache: from.cache, redirect: from.redirect,
          referrer: from.referrer, referrerPolicy: from.referrerPolicy, integrity: from.integrity,
          keepalive: from.keepalive, duplex: "half", targetAddressSpace: from.targetAddressSpace,
        };
        signal = from.signal;
      }
      const has = (name) => given[name] !== undefined;
      const any = has("body") || has("cache") || has("credentials") || has("headers") ||
        has("integrity") || has("keepalive") || has("method") || has("mode") || has("priority") ||
        has("redirect") || has("referrer") || has("referrerPolicy") || has("signal") || has("window");
      if (given.window !== undefined && given.window !== null) {
        throw fail("'window' must be null.");
      }
      if (any) {
        if (state.mode === "navigate") state.mode = "same-origin";
        state.referrer = "about:client";
        state.referrerPolicy = "";
      }
      if (has("referrer")) {
        const referrer = str(given.referrer);
        if (referrer === "") {
          state.referrer = "no-referrer";
        } else {
          const url = native.resolveUrl(referrer);
          if (url === null) throw fail("The referrer URL " + referrer + " is invalid.");
          state.referrer = url === "about:client" || native.sameOriginUrl(url) === false ? "about:client" : url;
        }
      }
      const policy = enumMember(given, "referrerPolicy", REFERRER_POLICIES, "ReferrerPolicy");
      if (policy !== undefined) state.referrerPolicy = policy;
      // Local Network Access (M33.4): the space the server is to be in.
      const space = enumMember(given, "targetAddressSpace", ADDRESS_SPACES, "IPAddressSpace");
      if (space !== undefined) state.targetAddressSpace = space;
      const mode = enumMember(given, "mode", REQUEST_MODES, "RequestMode");
      if (mode === "navigate") {
        throw fail("Cannot construct a Request with a RequestInit whose mode member is set as 'navigate'.");
      }
      if (mode !== undefined) state.mode = mode;
      else if (fallbackMode !== null) state.mode = fallbackMode;
      const credentials = enumMember(given, "credentials", REQUEST_CREDENTIALS, "RequestCredentials");
      if (credentials !== undefined) state.credentials = credentials;
      const cache = enumMember(given, "cache", REQUEST_CACHES, "RequestCache");
      if (cache !== undefined) state.cache = cache;
      if (state.cache === "only-if-cached" && state.mode !== "same-origin") {
        throw fail("'only-if-cached' can be set only with 'same-origin' mode");
      }
      const redirect = enumMember(given, "redirect", REQUEST_REDIRECTS, "RequestRedirect");
      if (redirect !== undefined) state.redirect = redirect;
      enumMember(given, "priority", REQUEST_PRIORITIES, "RequestPriority");
      if (has("integrity")) state.integrity = str(given.integrity);
      if (has("keepalive")) state.keepalive = !!given.keepalive;
      if (has("method")) {
        const method = byteString(given.method, "", "");
        if (!isToken(method)) throw fail("'" + method + "' is not a valid HTTP method.");
        if (isForbiddenMethod(method)) throw fail("'" + method + "' HTTP method is unsupported.");
        const upper = ToUpperCase(method);
        state.method =
          upper === "DELETE" || upper === "GET" || upper === "HEAD" || upper === "OPTIONS" ||
          upper === "POST" || upper === "PUT" ? upper : method;
      }
      if (has("signal")) {
        const given_signal = given.signal;
        if (given_signal !== null && !isSignal(given_signal)) {
          throw fail("Failed to read the 'signal' property from 'RequestInit': Failed to convert value to 'AbortSignal'.");
        }
        signal = given_signal;
      }
      // The request's own signal follows the one given.
      state.signal = signal === null ? new AbortSignal(KEY) : followingSignal(signal);
      // Headers.
      const headers = makeHeaders("request");
      state.headers = headers;
      if (state.mode === "no-cors") {
        if (state.method !== "GET" && state.method !== "HEAD" && state.method !== "POST") {
          throw fail("'" + state.method + "' is unsupported in no-cors mode.");
        }
        headersState(headers).guard = "request-no-cors";
      }
      if (has("headers")) {
        fillHeaders(headers, given.headers, "Request", true);
      } else if (inputIsRequest) {
        const list = headersState(input.#state.headers).list;
        for (let i = 0; i < list.length; i++) {
          appendHeader(headers, list[i][0], list[i][1], "append");
        }
      }
      // The body.
      const inputBody = inputIsRequest ? input.#state.body : null;
      const initBody = given.body !== undefined && given.body !== null ? given.body : null;
      if ((initBody !== null || (inputBody !== null && inputBody.bytes !== null)) &&
        (state.method === "GET" || state.method === "HEAD")) {
        throw fail("Request with GET/HEAD method cannot have body.");
      }
      if (initBody !== null) {
        const extracted = extract(initBody, "Request");
        state.body = bodyFrom(extracted.bytes);
        if (extracted.type !== null && combined(headersState(headers).list, "content-type") === null) {
          appendHeader(headers, "Content-Type", extracted.type, "append");
        }
      } else if (inputBody !== null && inputBody.bytes !== null) {
        if (inputBody.used) {
          throw fail("Cannot construct a Request with a Request object that has already been used.");
        }
        state.body = bodyFrom(inputBody.bytes);
        inputBody.used = true;
      } else {
        state.body = bodyFrom(null);
      }
      this.#state = state;
    }

    static {
      requestState = (request) => request.#state;
      isRequest = (value) => typeof value === "object" && value !== null && #state in value;
    }

    get method() { return this.#state.method; }
    get url() { return this.#state.url; }
    get headers() { return this.#state.headers; }
    get destination() { this.#state; return ""; }
    get referrer() {
      const referrer = this.#state.referrer;
      return referrer === "no-referrer" ? "" : referrer;
    }
    get referrerPolicy() { return this.#state.referrerPolicy; }
    get mode() { return this.#state.mode; }
    get credentials() { return this.#state.credentials; }
    get cache() { return this.#state.cache; }
    get redirect() { return this.#state.redirect; }
    get integrity() { return this.#state.integrity; }
    get keepalive() { return this.#state.keepalive; }
    get isReloadNavigation() { this.#state; return false; }
    get isHistoryNavigation() { this.#state; return false; }
    get signal() { return this.#state.signal; }
    get duplex() { this.#state; return "half"; }

    clone() {
      const from = this.#state;
      if (from.body.bytes !== null && from.body.used) {
        throw new $TypeError(prefix("clone", "Request") + "Request body is already used");
      }
      const headers = makeHeaders(headersState(from.headers).guard, cloneList(headersState(from.headers).list));
      const state = {
        method: from.method, url: from.url, headers,
        body: bodyFrom(from.body.bytes === null ? null : TypedArraySlice(from.body.bytes)),
        mode: from.mode, credentials: from.credentials, cache: from.cache, redirect: from.redirect,
        referrer: from.referrer, referrerPolicy: from.referrerPolicy, integrity: from.integrity,
        keepalive: from.keepalive, duplex: "half", signal: followingSignal(from.signal),
      };
      return new Request(KEY, state);
    }
  }
  installBody(Request, "Request", (value) => isRequest(value), (request) => requestState(request).body, (request) => requestState(request).headers);
  expose(Request, "Request");

  function cloneList(list) {
    const out = [];
    for (let i = 0; i < list.length; i++) ArrayPush(out, [list[i][0], list[i][1]]);
    return out;
  }

  // A status text: HTAB, SP, VCHAR or obs-text, as reason-phrase is.
  function isReasonPhrase(text) {
    for (let i = 0; i < text.length; i++) {
      const c = CharCodeAt(text, i);
      if (!(c === 0x09 || (c >= 0x20 && c <= 0x7e) || (c >= 0x80 && c <= 0xff))) return false;
    }
    return true;
  }

  function isNullBodyStatus(status) {
    return status === 101 || status === 103 || status === 204 || status === 205 || status === 304;
  }

  // Response (Fetch 5.5).
  let responseState;
  let isResponse;
  class Response {
    #state;

    constructor(body, init) {
      if (body === KEY) {
        this.#state = init;
        return;
      }
      const fail = (message, Error) => new (Error || $TypeError)("Failed to construct 'Response': " + message);
      if (init !== undefined && init !== null && typeof init !== "object" && typeof init !== "function") {
        throw fail("The provided value is not of type 'ResponseInit'.");
      }
      const given = init === undefined || init === null ? {} : init;
      const status = given.status === undefined ? 200 : unsignedShort(given.status);
      if (status < 200 || status > 599) {
        throw fail("The status provided (" + status + ") is outside the range [200, 599].", $RangeError);
      }
      const statusText = given.statusText === undefined ? "" : byteString(given.statusText, "", "");
      if (!isReasonPhrase(statusText)) {
        throw fail("Invalid statusText");
      }
      const headers = makeHeaders("response");
      if (given.headers !== undefined) {
        fillHeaders(headers, given.headers, "Response", true);
      }
      let bytes = null;
      if (body !== undefined && body !== null) {
        if (isNullBodyStatus(status)) {
          throw fail("Response with null body status cannot have body");
        }
        const extracted = extract(body, "Response");
        bytes = extracted.bytes;
        if (extracted.type !== null && combined(headersState(headers).list, "content-type") === null) {
          appendHeader(headers, "Content-Type", extracted.type, "append");
        }
      }
      this.#state = {
        type: "default", urlList: [], status, statusText, headers, body: bodyFrom(bytes),
      };
    }

    static {
      responseState = (response) => response.#state;
      isResponse = (value) => typeof value === "object" && value !== null && #state in value;
    }

    static error() {
      return new Response(KEY, {
        type: "error", urlList: [], status: 0, statusText: "", headers: makeHeaders("immutable"),
        body: bodyFrom(null),
      });
    }

    static redirect(url, status) {
      need(arguments.length, 1, "redirect", "Response");
      const text = str(url);
      const parsed = native.resolveUrl(text);
      if (parsed === null) {
        throw new $TypeError(prefix("redirect", "Response") + "Failed to parse URL from " + text);
      }
      const code = status === undefined ? 302 : unsignedShort(status);
      if (code !== 301 && code !== 302 && code !== 303 && code !== 307 && code !== 308) {
        throw new $RangeError(prefix("redirect", "Response") + "Invalid status code");
      }
      return new Response(KEY, {
        type: "default", urlList: [], status: code, statusText: "",
        headers: makeHeaders("immutable", [["Location", parsed]]), body: bodyFrom(null),
      });
    }

    static json(data, init) {
      const text = JSONStringify(data);
      if (text === undefined) {
        throw new $TypeError(prefix("json", "Response") + "The data is not JSON serializable");
      }
      const response = new Response(null, init);
      const state = response.#state;
      if (isNullBodyStatus(state.status)) {
        throw new $TypeError(prefix("json", "Response") + "Response with null body status cannot have body");
      }
      state.body = bodyFrom(utf8Encode(text));
      if (combined(headersState(state.headers).list, "content-type") === null) {
        appendHeader(state.headers, "Content-Type", "application/json", "append");
      }
      return response;
    }

    get type() { return this.#state.type; }
    get url() {
      const list = this.#state.urlList;
      if (list.length === 0) return "";
      const url = list[list.length - 1];
      const hash = StringIndexOf(url, "#");
      return hash < 0 ? url : StringSlice(url, 0, hash);
    }
    get redirected() { return this.#state.urlList.length > 1; }
    get status() { return this.#state.status; }
    get ok() { return this.#state.status >= 200 && this.#state.status <= 299; }
    get statusText() { return this.#state.statusText; }
    get headers() { return this.#state.headers; }

    clone() {
      const from = this.#state;
      if (from.body.bytes !== null && from.body.used) {
        throw new $TypeError(prefix("clone", "Response") + "Response body is already used");
      }
      return new Response(KEY, {
        type: from.type, urlList: ArraySlice(from.urlList), status: from.status,
        statusText: from.statusText,
        headers: makeHeaders(headersState(from.headers).guard, cloneList(headersState(from.headers).list)),
        body: bodyFrom(from.body.bytes === null ? null : TypedArraySlice(from.body.bytes)),
      });
    }
  }
  installBody(Response, "Response", (value) => isResponse(value), (response) => responseState(response).body, (response) => responseState(response).headers);
  expose(Response, "Response");

  // An `unsigned short` (Web IDL's conversion).
  function unsignedShort(value) {
    const n = $Number(value);
    if (n !== n || n === Infinity || n === -Infinity) return 0;
    const t = n < 0 ? -MathFloor(-n) : MathFloor(n);
    return ((t % 65536) + 65536) % 65536;
  }

  // The fetches under way, by number: their promises' settlers and their
  // signals' abort steps.
  const fetches = new $Map();
  let nextFetch = 1;

  // fetch() (Fetch 5.6).
  function fetch(input, init) {
    let request;
    try {
      request = new Request(input, init);
    } catch (e) {
      if (e instanceof $TypeError || e instanceof $RangeError) {
        const was = "Failed to construct 'Request': ";
        if (StringSlice(e.message, 0, was.length) === was) {
          e.message = "Failed to execute 'fetch' on 'Window': " + StringSlice(e.message, was.length);
        }
      }
      return rejected(e);
    }
    const state = requestState(request);
    const signal = state.signal;
    if (signal.aborted) {
      return rejected(signal.reason);
    }
    const id = nextFetch++;
    return new $Promise((resolve, reject) => {
      const body = state.body.bytes;
      if (body !== null) state.body.used = true;
      try {
        native.fetch(
          id, state.method, state.url, flatten(headersState(state.headers).list),
          body === null ? null : binaryString(body),
          state.mode, state.credentials, state.redirect, false, state.cache,
          state.referrer, state.referrerPolicy, state.targetAddressSpace,
        );
      } catch (e) {
        reject(failed(e, "fetch", GLOBAL_INTERFACE));
        return;
      }
      const abort = () => {
        if (!MapHas(fetches, id)) return;
        MapDelete(fetches, id);
        native.abortFetch(id);
        reject(signal.reason);
      };
      ArrayPush(signalOf(signal).algorithms, abort);
      MapSet(fetches, id, { resolve, reject, signal, abort });
    });
  }

  // Bytes as a string of code units 0 to 255, as the natives take them.
  function binaryString(bytes) {
    let out = "";
    const n = viewLength(bytes);
    for (let i = 0; i < n; i += 4096) {
      const chunk = [];
      const end = MathMin(n, i + 4096);
      for (let j = i; j < end; j++) ArrayPush(chunk, bytes[j]);
      out += ReflectApply(FromCharCode, null, chunk);
    }
    return out;
  }

  // A header list as the natives take it: name, value, name, value...
  function flatten(list) {
    const out = [];
    for (let i = 0; i < list.length; i++) ArrayPush(out, list[i][0], list[i][1]);
    return out;
  }

  // A fetch's answer, from the engine: a response, or null for a network
  // error.
  function fetched(id, answer) {
    const pending = MapGet(fetches, id);
    if (pending === undefined) return;
    MapDelete(fetches, id);
    if (pending.deliver !== undefined) {
      pending.deliver(answer);
      return;
    }
    const algorithms = signalOf(pending.signal).algorithms;
    for (let i = 0; i < algorithms.length; i++) {
      if (algorithms[i] === pending.abort) {
        ArraySplice(algorithms, i, 1);
        break;
      }
    }
    if (answer === null) {
      pending.reject(new $TypeError("Failed to fetch"));
      return;
    }
    const list = [];
    for (let i = 0; i < answer.headers.length; i += 2) {
      ArrayPush(list, [answer.headers[i], answer.headers[i + 1]]);
    }
    pending.resolve(
      new Response(KEY, {
        type: answer.type, urlList: answer.urlList, status: answer.status,
        statusText: answer.statusText, headers: makeHeaders("immutable", list),
        body: bodyFrom(answer.body === null ? null : new $Uint8Array(answer.body)),
      }),
    );
  }

  defineProperty(global, "fetch", {
    value: fetch,
    writable: true,
    enumerable: true,
    configurable: true,
  });

  // ---- XMLHttpRequest (XMLHttpRequest Standard, M27.6) ----

  // Whether `target` has a listener (XHR's upload listener flag).
  function hasListeners(target) {
    return listenersOf(target).length > 0;
  }

  // An event of the request's own, dispatched at once, trusted.
  function fireAt(target, event) {
    eventState(event).trusted = true;
    drain(dispatching(target, event));
  }

  function progressEvent(type, loaded, total, computable) {
    return new ProgressEvent(type, { lengthComputable: computable, loaded, total });
  }

  class XMLHttpRequestEventTarget extends EventTarget {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("XMLHttpRequestEventTarget");
      }
      super();
    }
  }

  class XMLHttpRequestUpload extends XMLHttpRequestEventTarget {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("XMLHttpRequestUpload");
      }
      super(KEY);
    }
  }

  const UNSENT = 0;
  const OPENED = 1;
  const HEADERS_RECEIVED = 2;
  const LOADING = 3;
  const DONE = 4;
  const RESPONSE_TYPES = ["", "arraybuffer", "blob", "document", "json", "text"];

  let isXhr;
  class XMLHttpRequest extends XMLHttpRequestEventTarget {
    #state = UNSENT;
    #send = false;
    #method = "GET";
    #url = "";
    #async = true;
    #headers = [];
    #timeout = 0;
    #credentials = false;
    #upload = new XMLHttpRequestUpload(KEY);
    #uploadComplete = false;
    #uploadListened = false;
    #response = null;
    #responseType = "";
    #override = null;
    #fetch = 0;
    #timer = 0;
    #cache = undefined;

    constructor() {
      super(KEY);
    }

    static {
      isXhr = (value) => typeof value === "object" && value !== null && #state in value;
    }

    get readyState() {
      return this.#state;
    }

    get upload() {
      return this.#upload;
    }

    get timeout() {
      return this.#timeout;
    }

    set timeout(value) {
      this.#state;
      const ms = unsigned(value);
      if (!WORKER && !this.#async && this.#state !== UNSENT) {
        throw new $DOMException(
          "Failed to set the 'timeout' property on 'XMLHttpRequest': Timeouts cannot be set for synchronous requests made from a document.",
          "InvalidAccessError",
        );
      }
      this.#timeout = ms;
    }

    get withCredentials() {
      return this.#credentials;
    }

    set withCredentials(value) {
      if ((this.#state !== UNSENT && this.#state !== OPENED) || this.#send) {
        throw new $DOMException(
          "Failed to set the 'withCredentials' property on 'XMLHttpRequest': The value may only be set if the object's state is UNSENT or OPENED.",
          "InvalidStateError",
        );
      }
      this.#credentials = !!value;
    }

    open(method, url, async, username, password) {
      need(arguments.length, 2, "open", "XMLHttpRequest");
      const m = byteString(method, "open", "XMLHttpRequest");
      if (!isToken(m)) {
        throw domError("SyntaxError", "'" + m + "' is not a valid HTTP method.", "open", "XMLHttpRequest");
      }
      if (isForbiddenMethod(m)) {
        throw domError("SecurityError", "'" + m + "' HTTP method is unsupported.", "open", "XMLHttpRequest");
      }
      const resolved = native.resolveUrl(str(url));
      if (resolved === null) {
        throw domError("SyntaxError", "Invalid URL", "open", "XMLHttpRequest");
      }
      const isAsync = arguments.length < 3 ? true : !!async;
      if (username !== undefined && username !== null) {
        str(username);
      }
      if (password !== undefined && password !== null) {
        str(password);
      }
      if (!WORKER && !isAsync && (this.#timeout !== 0 || this.#responseType !== "")) {
        throw domError(
          "InvalidAccessError",
          "Synchronous requests from a document must not set a timeout or a response type.",
          "open",
          "XMLHttpRequest",
        );
      }
      this.#terminate();
      const upper = ToUpperCase(m);
      this.#method =
        upper === "DELETE" || upper === "GET" || upper === "HEAD" || upper === "OPTIONS" ||
        upper === "POST" || upper === "PUT" ? upper : m;
      this.#url = resolved;
      this.#async = isAsync;
      this.#headers = [];
      this.#send = false;
      this.#response = null;
      this.#cache = undefined;
      if (this.#state !== OPENED) {
        this.#state = OPENED;
        fireAt(this, new Event("readystatechange"));
      }
    }

    setRequestHeader(name, value) {
      need(arguments.length, 2, "setRequestHeader", "XMLHttpRequest");
      if (this.#state !== OPENED || this.#send) {
        throw domError("InvalidStateError", "The object's state must be OPENED.", "setRequestHeader", "XMLHttpRequest");
      }
      const key = byteString(name, "setRequestHeader", "XMLHttpRequest");
      const normal = normalizeValue(byteString(value, "setRequestHeader", "XMLHttpRequest"));
      if (!isToken(key)) {
        throw domError("SyntaxError", "'" + key + "' is not a valid HTTP header field name.", "setRequestHeader", "XMLHttpRequest");
      }
      if (!isHeaderValue(normal)) {
        throw domError("SyntaxError", "'" + normal + "' is not a valid HTTP header field value.", "setRequestHeader", "XMLHttpRequest");
      }
      if (isForbiddenRequestHeader(key, normal)) {
        return;
      }
      // Combined with a field of the same name (XMLHttpRequest 3.5.2).
      const lower = ToLowerCase(key);
      for (let i = 0; i < this.#headers.length; i++) {
        if (ToLowerCase(this.#headers[i][0]) === lower) {
          this.#headers[i][1] += ", " + normal;
          return;
        }
      }
      ArrayPush(this.#headers, [key, normal]);
    }

    send(body) {
      if (this.#state !== OPENED || this.#send) {
        throw domError("InvalidStateError", "The object's state must be OPENED.", "send", "XMLHttpRequest");
      }
      let bytes = null;
      if (body !== undefined && body !== null && this.#method !== "GET" && this.#method !== "HEAD") {
        const extracted = extract(body, "XMLHttpRequest");
        bytes = extracted.bytes;
        const set = combined(this.#headers, "content-type");
        if (set === null && extracted.type !== null) {
          ArrayPush(this.#headers, ["Content-Type", extracted.type]);
        }
      }
      this.#uploadComplete = bytes === null;
      this.#uploadListened = hasListeners(this.#upload);
      this.#send = true;
      const credentials = this.#credentials ? "include" : "same-origin";
      if (!this.#async) {
        let answer;
        try {
          answer = native.fetchNow(
            this.#method, this.#url, flatten(this.#headers),
            bytes === null ? null : binaryString(bytes), "cors", credentials, "follow", false,
          );
        } catch (e) {
          answer = null;
        }
        if (answer === null) {
          this.#state = DONE;
          this.#send = false;
          this.#response = { error: true };
          throw domError("NetworkError", "Failed to load '" + this.#url + "'.", "send", "XMLHttpRequest");
        }
        this.#response = answer;
        this.#state = DONE;
        this.#send = false;
        fireAt(this, new Event("readystatechange"));
        const length = answer.body === null ? 0 : bufferLength(answer.body);
        fireAt(this, progressEvent("load", length, length, true));
        fireAt(this, progressEvent("loadend", length, length, true));
        return;
      }
      fireAt(this, progressEvent("loadstart", 0, 0, false));
      if (this.#uploadListened && !this.#uploadComplete) {
        fireAt(this.#upload, progressEvent("loadstart", 0, viewLength(bytes), true));
      }
      if (this.#state !== OPENED || !this.#send) {
        return;
      }
      const id = nextFetch++;
      this.#fetch = id;
      try {
        native.fetch(
          id, this.#method, this.#url, flatten(this.#headers),
          bytes === null ? null : binaryString(bytes), "cors", credentials, "follow",
          this.#uploadListened, "default", "about:client", "", "",
        );
      } catch (e) {
        this.#fetch = 0;
        this.#errorSteps("error");
        return;
      }
      const sent = bytes === null ? 0 : viewLength(bytes);
      MapSet(fetches, id, {
        deliver: (answer) => this.#answered(answer, sent),
      });
      if (this.#timeout > 0) {
        this.#timer = startTimer(() => this.#timedOut(id), this.#timeout, [], false);
      }
    }

    // The response came, or a network error (XMLHttpRequest 3.5.6's
    // process response, end of body, and request error steps).
    #answered(answer, sent) {
      this.#fetch = 0;
      this.#stopTimer();
      if (answer === null) {
        this.#errorSteps("error");
        return;
      }
      if (this.#uploadListened && !this.#uploadComplete) {
        this.#uploadComplete = true;
        fireAt(this.#upload, progressEvent("progress", sent, sent, true));
        fireAt(this.#upload, progressEvent("load", sent, sent, true));
        fireAt(this.#upload, progressEvent("loadend", sent, sent, true));
      }
      this.#uploadComplete = true;
      this.#response = answer;
      this.#state = HEADERS_RECEIVED;
      fireAt(this, new Event("readystatechange"));
      if (this.#state !== HEADERS_RECEIVED) return;
      const length = answer.body === null ? 0 : bufferLength(answer.body);
      if (length > 0) {
        this.#state = LOADING;
        fireAt(this, new Event("readystatechange"));
        if (this.#state !== LOADING) return;
      }
      fireAt(this, progressEvent("progress", length, length, true));
      this.#state = DONE;
      this.#send = false;
      fireAt(this, new Event("readystatechange"));
      fireAt(this, progressEvent("load", length, length, true));
      fireAt(this, progressEvent("loadend", length, length, true));
    }

    // The request error steps of `event`: error, abort or timeout.
    #errorSteps(event) {
      this.#state = DONE;
      this.#send = false;
      this.#response = { error: true };
      fireAt(this, new Event("readystatechange"));
      if (!this.#uploadComplete) {
        this.#uploadComplete = true;
        if (this.#uploadListened) {
          fireAt(this.#upload, progressEvent(event, 0, 0, false));
          fireAt(this.#upload, progressEvent("loadend", 0, 0, false));
        }
      }
      fireAt(this, progressEvent(event, 0, 0, false));
      fireAt(this, progressEvent("loadend", 0, 0, false));
    }

    #stopTimer() {
      if (this.#timer !== 0) {
        stopTimer(this.#timer);
        this.#timer = 0;
      }
    }

    #timedOut(id) {
      this.#timer = 0;
      if (this.#fetch !== id) return;
      MapDelete(fetches, id);
      native.abortFetch(id);
      this.#fetch = 0;
      this.#errorSteps("timeout");
    }

    // Stops a fetch under way, quietly (open's terminate).
    #terminate() {
      if (this.#fetch !== 0) {
        MapDelete(fetches, this.#fetch);
        native.abortFetch(this.#fetch);
        this.#fetch = 0;
      }
      this.#stopTimer();
    }

    abort() {
      const pending = (this.#state === OPENED && this.#send) || this.#state === HEADERS_RECEIVED ||
        this.#state === LOADING;
      this.#terminate();
      if (pending) {
        this.#errorSteps("abort");
      }
      if (this.#state === DONE) {
        this.#state = UNSENT;
        this.#response = null;
      }
    }

    get responseURL() {
      const response = this.#response;
      if (response === null || response.error || response.urlList.length === 0) return "";
      const url = response.urlList[response.urlList.length - 1];
      const hash = StringIndexOf(url, "#");
      return hash < 0 ? url : StringSlice(url, 0, hash);
    }

    get status() {
      const response = this.#response;
      return response === null || response.error ? 0 : response.status;
    }

    get statusText() {
      const response = this.#response;
      return response === null || response.error ? "" : response.statusText;
    }

    getResponseHeader(name) {
      need(arguments.length, 1, "getResponseHeader", "XMLHttpRequest");
      const response = this.#response;
      if (response === null || response.error) return null;
      return combined(pairsFrom(response.headers), ToLowerCase(byteString(name, "getResponseHeader", "XMLHttpRequest")));
    }

    getAllResponseHeaders() {
      const response = this.#response;
      if (response === null || response.error) return "";
      const pairs = sortAndCombine(pairsFrom(response.headers));
      let out = "";
      for (let i = 0; i < pairs.length; i++) {
        out += pairs[i][0] + ": " + pairs[i][1] + "\r\n";
      }
      return out;
    }

    overrideMimeType(mime) {
      need(arguments.length, 1, "overrideMimeType", "XMLHttpRequest");
      if (this.#state === LOADING || this.#state === DONE) {
        throw domError("InvalidStateError", "MimeType cannot be overridden when the state is LOADING or DONE.", "overrideMimeType", "XMLHttpRequest");
      }
      this.#override = str(mime);
    }

    get responseType() {
      return this.#responseType;
    }

    set responseType(value) {
      const type = str(value);
      if (!within(RESPONSE_TYPES, type)) return;
      if (WORKER && type === "document") return;
      if (this.#state === LOADING || this.#state === DONE) {
        throw new $DOMException(
          "Failed to set the 'responseType' property on 'XMLHttpRequest': The response type cannot be set if the object's state is LOADING or DONE.",
          "InvalidStateError",
        );
      }
      if (!WORKER && !this.#async && this.#state !== UNSENT) {
        throw new $DOMException(
          "Failed to set the 'responseType' property on 'XMLHttpRequest': The response type cannot be changed for synchronous requests made from a document.",
          "InvalidAccessError",
        );
      }
      this.#responseType = type;
    }

    // The final MIME type: the override's, or the response's.
    #mime() {
      if (this.#override !== null) return this.#override;
      const response = this.#response;
      return combined(pairsFrom(response.headers), "content-type") || "text/xml";
    }

    #text() {
      const response = this.#response;
      if (response === null || response.error || response.body === null) return "";
      const mime = this.#mime();
      const charset = parameter(";" + StringSlice(mime, StringIndexOf(mime, ";") + 1), "charset");
      const bytes = new $Uint8Array(response.body);
      if (charset !== null && StringIndexOf(mime, ";") >= 0) {
        const decoded = native.decode(binaryString(bytes), charset);
        if (decoded !== null) return decoded;
      }
      return utf8Decode(bytes, true);
    }

    get responseText() {
      if (this.#responseType !== "" && this.#responseType !== "text") {
        throw new $DOMException(
          "Failed to read the 'responseText' property from 'XMLHttpRequest': The value is only accessible if the object's 'responseType' is '' or 'text' (was '" +
            this.#responseType + "').",
          "InvalidStateError",
        );
      }
      if (this.#state !== LOADING && this.#state !== DONE) return "";
      return this.#text();
    }

    get responseXML() {
      if (this.#responseType !== "" && this.#responseType !== "document") {
        throw new $DOMException(
          "Failed to read the 'responseXML' property from 'XMLHttpRequest': The value is only accessible if the object's 'responseType' is '' or 'document' (was '" +
            this.#responseType + "').",
          "InvalidStateError",
        );
      }
      return null;
    }

    get response() {
      const type = this.#responseType;
      if (type === "" || type === "text") {
        return this.responseText;
      }
      if (this.#state !== DONE) return null;
      const response = this.#response;
      if (response === null || response.error) return null;
      if (this.#cache !== undefined) return this.#cache;
      const bytes = response.body === null ? new $Uint8Array(0) : new $Uint8Array(response.body);
      let value = null;
      if (type === "arraybuffer") {
        value = viewBuffer(TypedArraySlice(bytes));
      } else if (type === "blob") {
        value = new Blob(KEY, { bytes: TypedArraySlice(bytes), type: blobType(this.#mime()) });
      } else if (type === "json") {
        try {
          value = JSONParse(utf8Decode(bytes, true));
        } catch (e) {
          value = null;
        }
      }
      this.#cache = value;
      return value;
    }
  }
  constants(XMLHttpRequest, ["UNSENT", 0, "OPENED", 1, "HEADERS_RECEIVED", 2, "LOADING", 3, "DONE", 4]);
  defineHandlers(
    XMLHttpRequestEventTarget.prototype,
    ["loadstart", "progress", "abort", "error", "load", "timeout", "loadend"],
    false,
  );
  defineHandlers(XMLHttpRequest.prototype, ["readystatechange"], false);
  expose(XMLHttpRequestEventTarget, "XMLHttpRequestEventTarget");
  expose(XMLHttpRequestUpload, "XMLHttpRequestUpload");
  expose(XMLHttpRequest, "XMLHttpRequest");
  // `responseXML` is the window's alone (XHR Standard 4.8).
  if (WORKER) {
    ReflectDeleteProperty(XMLHttpRequest.prototype, "responseXML");
  }

  // A header list as pairs, from the natives' names and values.
  function pairsFrom(flat) {
    const out = [];
    for (let i = 0; i < flat.length; i += 2) ArrayPush(out, [flat[i], flat[i + 1]]);
    return out;
  }

  // The handle of `node`, which must be a Node; whether a value is one;
  // and a node's kind and name. Set from inside Node's body, where its
  // private fields are seen.
  let handleOf;
  let isNode;
  let kindOf;
  let nameOf;
  let nsOf;
  let setNsOf;
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
    // An element's namespace's URI, null for none (M32).
    #ns = null;
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
      nsOf = (node) => node.#ns;
      setNsOf = (node, ns) => {
        node.#ns = ns;
      };
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
      return kindOf(this) === 1 ? qualifiedTag(this) : nameOf(this);
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

    getElementsByTagNameNS(namespace, localName) {
      const handle = self(this);
      need(
        arguments.length,
        2,
        "getElementsByTagNameNS",
        kindOf(this) === 9 ? "Document" : "Element",
      );
      const ns = nullable(namespace);
      const local = str(localName);
      return collection(
        HTMLCollection,
        this,
        () => native.byTagNS(handle, ns === null ? "" : ns, local),
        true,
      );
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

  // An element's `tagName`: an HTML element's name in upper case,
  // another's as it is (DOM section 4.9).
  function qualifiedTag(element) {
    const name = nameOf(element);
    return nsOf(element) === HTML_NS ? ToUpperCase(name) : name;
  }

  // An attribute's name as an element keeps it: lowercased on an HTML
  // element, as given on another.
  function attrKey(element, name) {
    return nsOf(element) === HTML_NS ? ToLowerCase(name) : name;
  }

  // A nullable namespace argument: null, undefined and the empty string
  // are none.
  function nullable(value) {
    if (value === null || value === undefined) {
      return null;
    }
    const text = str(value);
    return text === "" ? null : text;
  }

  // The namespace of an element's attribute named `qualified`: its
  // prefix's, where that is XLink's, XML's or XMLNS's, on an element
  // that is not HTML's; none otherwise.
  function attributeNamespace(element, qualified) {
    if (nsOf(element) === HTML_NS) {
      return null;
    }
    if (qualified === "xmlns") {
      return XMLNS_NS;
    }
    switch (prefixPart(qualified)) {
      case "xlink":
        return XLINK_NS;
      case "xml":
        return XML_NS;
      case "xmlns":
        return XMLNS_NS;
      default:
        return null;
    }
  }

  // The qualified name of an element's attribute of namespace `ns` and
  // local name `local`, or null.
  function attributeNamed(element, ns, local) {
    const names = native.attributeNames(handleOf(element));
    for (let i = 0; i < names.length; i++) {
      const name = names[i];
      const namespace = attributeNamespace(element, name);
      const own = namespace === null ? name : localPart(name);
      if (namespace === ns && own === local) {
        return name;
      }
    }
    return null;
  }

  // DOM's "validate and extract" (section 1.5): `qualified` a name with
  // at most one prefix, the namespace one its prefix allows; the
  // namespace and the name, or the reference browser's error.
  function validateAndExtract(ns, qualified, method, iface) {
    if (qualified.length === 0) {
      throw domError("InvalidCharacterError", "The qualified name provided is empty.", method, iface);
    }
    const at = StringIndexOf(qualified, ":");
    const local = at < 0 ? qualified : StringSlice(qualified, at + 1);
    const prefix = at < 0 ? null : StringSlice(qualified, 0, at);
    const bad = (c) => c === 9 || c === 10 || c === 12 || c === 13 || c === 32 || c === 0 ||
      c === 47 || c === 62;
    const first = CharCodeAt(qualified, 0);
    const startsWell = (first >= 65 && first <= 90) || (first >= 97 && first <= 122) ||
      first === 95 || first === 58 || first >= 128;
    let invalid = startsWell ? -1 : 0;
    for (let i = 0; invalid < 0 && i < qualified.length; i++) {
      if (bad(CharCodeAt(qualified, i))) {
        invalid = i;
      }
    }
    if (invalid >= 0 || local.length === 0 || (prefix !== null && prefix.length === 0)) {
      const c = invalid >= 0 ? qualified[invalid] : ":";
      throw domError(
        "InvalidCharacterError",
        "The qualified name provided ('" + qualified + "') contains the invalid character '" + c +
          "'.",
        method,
        iface,
      );
    }
    if (
      (prefix !== null && ns === null) ||
      (prefix === "xml" && ns !== XML_NS) ||
      ((qualified === "xmlns" || prefix === "xmlns") && ns !== XMLNS_NS) ||
      (ns === XMLNS_NS && qualified !== "xmlns" && prefix !== "xmlns")
    ) {
      throw domError(
        "NamespaceError",
        "The namespace URI provided ('" + (ns === null ? "" : ns) +
          "') is not valid for the qualified name provided ('" + qualified + "').",
        method,
        iface,
      );
    }
    return [ns, qualified];
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
      return qualifiedTag(this);
    }

    get localName() {
      self(this);
      return localPart(nameOf(this));
    }

    get namespaceURI() {
      self(this);
      return nsOf(this);
    }

    get prefix() {
      self(this);
      return prefixPart(nameOf(this));
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
      return native.attribute(handle, attrKey(this, str(name)));
    }

    getAttributeNames() {
      return native.attributeNames(self(this));
    }

    hasAttribute(name) {
      const handle = self(this);
      need(arguments.length, 1, "hasAttribute", "Element");
      return native.attribute(handle, attrKey(this, str(name))) !== null;
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
      setAttribute(handle, attrKey(this, key), str(value), "setAttribute");
    }

    removeAttribute(name) {
      const handle = self(this);
      need(arguments.length, 1, "removeAttribute", "Element");
      removeAttribute(handle, attrKey(this, str(name)));
    }

    // The namespaced attribute methods (DOM section 4.9): an attribute's
    // namespace is its prefix's, XLink's, XML's or XMLNS's, on an
    // element that is not HTML's (M32; G51 for the rest).
    getAttributeNS(namespace, localName) {
      const handle = self(this);
      need(arguments.length, 2, "getAttributeNS", "Element");
      const name = attributeNamed(this, nullable(namespace), str(localName));
      return name === null ? null : native.attribute(handle, name);
    }

    hasAttributeNS(namespace, localName) {
      self(this);
      need(arguments.length, 2, "hasAttributeNS", "Element");
      return attributeNamed(this, nullable(namespace), str(localName)) !== null;
    }

    setAttributeNS(namespace, qualifiedName, value) {
      const handle = self(this);
      need(arguments.length, 3, "setAttributeNS", "Element");
      const [ns, qualified] = validateAndExtract(
        nullable(namespace),
        str(qualifiedName),
        "setAttributeNS",
        "Element",
      );
      const before = attributeNamed(this, ns, localPart(qualified));
      if (before !== null && before !== qualified) {
        removeAttribute(handle, before);
      }
      setAttribute(handle, qualified, str(value), "setAttributeNS");
    }

    removeAttributeNS(namespace, localName) {
      const handle = self(this);
      need(arguments.length, 2, "removeAttributeNS", "Element");
      const name = attributeNamed(this, nullable(namespace), str(localName));
      if (name !== null) {
        removeAttribute(handle, name);
      }
    }

    toggleAttribute(name, force) {
      const handle = self(this);
      need(arguments.length, 1, "toggleAttribute", "Element");
      const key = str(name);
      if (!validAttributeName(key)) {
        throw badAttributeName(key, "toggleAttribute");
      }
      const lower = attrKey(this, key);
      if (native.attribute(handle, lower) === null) {
        if (force === undefined || force) {
          setAttribute(handle, lower, "", "toggleAttribute");
          return true;
        }
        return false;
      }
      if (force === undefined || !force) {
        removeAttribute(handle, lower);
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
    attributeHandler(wrap(handle), name, true);
  }

  function removeAttribute(handle, name) {
    native.removeAttribute(handle, name);
    attributeHandler(wrap(handle), name, false);
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

  // The elements a `click()` is dispatching at, which a click inside it
  // does not click again.
  const clicking = new $Map();

  class HTMLElement extends Element {
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        throw illegalConstructor("HTMLElement");
      }
      super(key, handle, kind, name);
    }

    // HTML's click() (section 6.5.4): a synthetic click, a PointerEvent
    // as in the reference browser, unless the element is a disabled
    // form control or already being clicked.
    click() {
      const handle = self(this);
      if (native.disabled(handle) || MapHas(clicking, handle)) {
        return;
      }
      MapSet(clicking, handle, true);
      try {
        const event = new PointerEvent("click", made(POINTER_INIT, [
          "bubbles", true,
          "cancelable", true,
          "composed", true,
          "view", global,
          "pointerId", -1,
        ]));
        drain(dispatching(this, event));
      } finally {
        MapDelete(clicking, handle);
      }
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

    // The focused element, or the body when none is (HTML section 6.6.6),
    // and whether the window has the system's focus.
    get activeElement() {
      const handle = self(this);
      const focused = native.focused();
      return focused === null ? wrap(childOfHtml(handle, "body")) : wrap(focused);
    }

    hasFocus() {
      self(this);
      return native.hasFocus();
    }

    // Page visibility (HTML section 6.2): hidden while the window is
    // minimized or covered.
    get visibilityState() {
      self(this);
      return native.hidden() ? "hidden" : "visible";
    }

    get hidden() {
      self(this);
      return native.hidden();
    }

    // The DOM's createEvent (section 4.5): an uninitialized event of the
    // interface a legacy name gives.
    createEvent(interfaceName) {
      self(this);
      need(arguments.length, 1, "createEvent", "Document");
      const name = str(interfaceName);
      const known = MapGet(legacyEvents, ToLowerCase(name));
      if (known === undefined) {
        throw domError(
          "NotSupportedError",
          "The provided event type ('" + name + "') is invalid.",
          "createEvent",
          "Document",
        );
      }
      const event = new known[1]("", made(known[2], []));
      eventState(event).initialized = false;
      return event;
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

    // The referrer of the request that brought the document (M33.3).
    get referrer() {
      self(this);
      return native.documentInfo()[4];
    }

    get cookie() {
      self(this);
      return native.cookie();
    }

    set cookie(value) {
      self(this);
      native.setCookie(str(value));
    }

    get domain() {
      self(this);
      return native.documentDomain();
    }

    set domain(value) {
      self(this);
      native.setDocumentDomain(str(value));
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

    createElementNS(namespace, qualifiedName) {
      self(this);
      need(arguments.length, 2, "createElementNS", "Document");
      const [ns, qualified] = validateAndExtract(
        nullable(namespace),
        str(qualifiedName),
        "createElementNS",
        "Document",
      );
      try {
        return wrap(native.createElementNS(ns === null ? "" : ns, qualified));
      } catch (error) {
        throw failed(error, "createElementNS", "Document");
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

  // `getElementsByName` (HTML section 3.1.5, M26.11): a live NodeList
  // of the document's elements whose `name` is `elementName`; `forms`
  // (M26.13), a live HTMLCollection of its forms.
  include(
    {
      get forms() {
        const handle = self(this);
        return controlList(this, "forms", () =>
          collection(HTMLCollection, this, () => native.byTag(handle, "form"), true),
        );
      },

      getElementsByName(elementName) {
        const handle = self(this);
        need(arguments.length, 1, "getElementsByName", "Document");
        const name = str(elementName);
        return collection(
          NodeList,
          this,
          () => {
            const all = native.byTag(handle, "*");
            const named = [];
            for (let i = 0; i < all.length; i++) {
              if (native.attribute(all[i], "name") === name) {
                ArrayPush(named, all[i]);
              }
            }
            return named;
          },
          true,
        );
      },
    },
    [Document.prototype],
  );
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
        const item = state.named ? state.named(state, key) : namedItem(state, key);
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

  // `supported` the tokens `supports()` knows, lowercase; none for a
  // list that has no supported tokens.
  function tokenList(owner, handle, attribute, supported) {
    const target = new DOMTokenList(KEY);
    const state = { owner, handle, attribute, supported };
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
      const state = tokenState(this);
      need(arguments.length, 1, "supports", "DOMTokenList");
      if (state.supported === undefined) {
        throw new $TypeError(
          prefix("supports", "DOMTokenList") + "DOMTokenList has no supported tokens.",
        );
      }
      const lower = ToLowerCase(str(token));
      for (let i = 0; i < state.supported.length; i++) {
        if (state.supported[i] === lower) {
          return true;
        }
      }
      return false;
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
    // The CSSOM's, which the policies let through (M33.1).
    native.cssomStyled(state.handle);
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

  // ---- QuotaExceededError (Web IDL section 2.8.1, M28) ------------------

  // A DOMException of its own interface, with the quota and the amount
  // asked for where they are known: the storage areas know neither.
  const QUOTA_OPTIONS = members([], [
    ["quota", "double", null],
    ["requested", "double", null],
  ], "QuotaExceededErrorOptions");
  const quotas = new $WeakMap();

  class QuotaExceededError extends $DOMException {
    constructor(message = "", options = undefined) {
      const text = str(message);
      const given = dictionary(options, QUOTA_OPTIONS, "QuotaExceededError");
      const where = "Failed to construct 'QuotaExceededError': ";
      if (given.quota !== null && given.quota < 0) {
        throw new $RangeError(where + "The quota must not be negative.");
      }
      if (given.requested !== null && given.requested < 0) {
        throw new $RangeError(where + "The requested amount must not be negative.");
      }
      if (given.quota !== null && given.requested !== null && given.requested < given.quota) {
        throw new $RangeError(where + "The requested amount must not be less than the quota.");
      }
      super(text, "QuotaExceededError");
      WeakMapSet(quotas, this, given);
    }

    get quota() {
      return quotaOf(this).quota;
    }

    get requested() {
      return quotaOf(this).requested;
    }
  }
  expose(QuotaExceededError, "QuotaExceededError");

  function quotaOf(value) {
    const given = WeakMapGet(quotas, value);
    if (given === undefined) {
      throw illegalInvocation();
    }
    return given;
  }

  // ---- Web Storage (HTML section 12.2, M28) ------------------------------

  // Each area, its object and the proxy scripts hold, to whether it is
  // the window's `localStorage` (true) or `sessionStorage` (false); and
  // each object's proxy.
  const areaKinds = new $WeakMap();
  const areaProxies = new $WeakMap();
  let localArea = null;
  let sessionArea = null;

  function areaOf(value) {
    const local = WeakMapGet(areaKinds, value);
    if (local === undefined) {
      throw illegalInvocation();
    }
    return local;
  }

  // Its members in the reference browser's order.
  class Storage {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("Storage");
      }
    }

    get length() {
      return native.storageLength(areaOf(this));
    }

    clear() {
      native.storageClear(areaOf(this));
    }

    getItem(key) {
      const local = areaOf(this);
      need(arguments.length, 1, "getItem", "Storage");
      return native.storageGet(local, str(key));
    }

    key(index) {
      const local = areaOf(this);
      need(arguments.length, 1, "key", "Storage");
      return native.storageKey(local, unsigned(index));
    }

    removeItem(key) {
      const local = areaOf(this);
      need(arguments.length, 1, "removeItem", "Storage");
      native.storageRemove(local, str(key));
    }

    setItem(key, value) {
      const local = areaOf(this);
      need(arguments.length, 2, "setItem", "Storage");
      const name = str(key);
      if (!native.storageSet(local, name, str(value))) {
        throw new QuotaExceededError(
          prefix("setItem", "Storage") + "Setting the value of '" + name +
            "' exceeded the quota.",
        );
      }
    }
  }
  expose(Storage, "Storage");

  // The window's `localStorage` (`local`) or `sessionStorage` (HTML
  // sections 12.2.2 and 12.2.3): the same object at every reading, made
  // at the first; a SecurityError for a document whose origin is opaque.
  function storageArea(local) {
    const made = local ? localArea : sessionArea;
    if (made !== null) {
      return made;
    }
    const refused = native.storageOpen(local);
    if (refused !== "") {
      throw new $DOMException(
        "Failed to read the '" + (local ? "localStorage" : "sessionStorage") +
          "' property from 'Window': " + refused,
        "SecurityError",
      );
    }
    const target = new Storage(KEY);
    const area = new $Proxy(target, storageHandlers);
    WeakMapSet(areaKinds, target, local);
    WeakMapSet(areaKinds, area, local);
    WeakMapSet(areaProxies, target, area);
    if (local) {
      localArea = area;
    } else {
      sessionArea = area;
    }
    return area;
  }

  // Whether an item named `key` shows as a property of the area
  // `target` (Web IDL's named property visibility algorithm): not when
  // the object or a prototype has a property of that name, `Storage`
  // not overriding built-ins.
  function visible(target, key) {
    if (ReflectGetOwnPropertyDescriptor(target, key) !== undefined) {
      return false;
    }
    const proto = ReflectGetPrototypeOf(target);
    return proto === null || !ReflectHas(proto, key);
  }

  // The value of the item `key` if it shows as a property, or null.
  function shown(target, key) {
    if (typeof key !== "string" || !visible(target, key)) {
      return null;
    }
    return native.storageGet(WeakMapGet(areaKinds, target), str(key));
  }

  // The named property setter: `setItem` in the words of a property.
  function namedSet(target, key, value) {
    const name = str(key);
    if (!native.storageSet(WeakMapGet(areaKinds, target), name, str(value))) {
      throw new QuotaExceededError(
        "Failed to set a named property '" + name + "' on 'Storage': Setting the value of '" +
          name + "' exceeded the quota.",
      );
    }
  }

  // A legacy platform object's internal methods (Web IDL section 3.9)
  // for an interface with a named getter, setter and deleter.
  const storageHandlers = {
    get(target, key, receiver) {
      const value = shown(target, key);
      return value === null ? ReflectGet(target, key, receiver) : value;
    },
    set(target, key, value, receiver) {
      if (typeof key === "string" && receiver === WeakMapGet(areaProxies, target)) {
        namedSet(target, key, value);
        return true;
      }
      return ReflectSet(target, key, value, receiver);
    },
    defineProperty(target, key, desc) {
      if (typeof key === "string" && ReflectGetOwnPropertyDescriptor(target, key) === undefined) {
        if (!ReflectHas(desc, "value") && !ReflectHas(desc, "writable")) {
          throw new $TypeError(
            "Failed to set a named property '" + key +
              "' on 'Storage': Accessor properties are not allowed.",
          );
        }
        namedSet(target, key, desc.value);
        return true;
      }
      return ReflectDefineProperty(target, key, desc);
    },
    deleteProperty(target, key) {
      if (shown(target, key) !== null) {
        native.storageRemove(WeakMapGet(areaKinds, target), str(key));
        return true;
      }
      return ReflectDeleteProperty(target, key);
    },
    has(target, key) {
      return shown(target, key) !== null || ReflectHas(target, key);
    },
    ownKeys(target) {
      const keys = native.storageKeys(WeakMapGet(areaKinds, target));
      const out = [];
      for (let i = 0; i < keys.length; i++) {
        if (visible(target, keys[i])) {
          ArrayPush(out, keys[i]);
        }
      }
      const own = ReflectOwnKeys(target);
      for (let i = 0; i < own.length; i++) {
        ArrayPush(out, own[i]);
      }
      return out;
    },
    getOwnPropertyDescriptor(target, key) {
      const value = shown(target, key);
      if (value === null) {
        return ReflectGetOwnPropertyDescriptor(target, key);
      }
      return { value, writable: true, enumerable: true, configurable: true };
    },
    preventExtensions() {
      throw new $TypeError("Cannot prevent extensions");
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
  // URL (against the document's own for a `<base>`'s, `fallback` of
  // "fallback base"), a boolean, an unsigned or signed number with
  // `fallback`, or the enumerated `dir`.
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
            const url = fallback === "fallback base"
              ? native.resolveUrlFromDocument(value)
              : native.resolveUrl(value);
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
          // A CORS settings attribute (HTML 2.5.4): null when absent.
          case "cors":
            if (value === null) {
              return null;
            }
            return ToLowerCase(value) === "use-credentials" ? "use-credentials" : "anonymous";
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
          case "cors":
            if (value === null) {
              native.removeAttribute(handle, attribute);
            } else {
              setAttribute(handle, attribute, str(value), property);
            }
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
    ["HTMLBaseElement", [
      ["href", "href", "url", "fallback base"],
      ["target", "target", "string"],
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
      ["crossOrigin", "crossorigin", "cors"],
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
      ["srcdoc", "srcdoc", "string"],
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
      ["defaultChecked", "checked", "boolean"],
      ["disabled", "disabled", "boolean"],
      ["required", "required", "boolean"],
      ["readOnly", "readonly", "boolean"],
      ["multiple", "multiple", "boolean"],
      ["autofocus", "autofocus", "boolean"],
      ["size", "size", "unsigned", 20],
      ["min", "min", "string"],
      ["max", "max", "string"],
      ["step", "step", "string"],
      ["pattern", "pattern", "string"],
      ["alt", "alt", "string"],
      ["src", "src", "url"],
      ["accept", "accept", "string"],
      ["autocomplete", "autocomplete", "string"],
      ["formNoValidate", "formnovalidate", "boolean"],
    ]],
    ["HTMLButtonElement", [
      ["name", "name", "string"],
      ["value", "value", "string"],
      ["disabled", "disabled", "boolean"],
      ["formNoValidate", "formnovalidate", "boolean"],
    ]],
    ["HTMLSelectElement", [
      ["name", "name", "string"],
      ["disabled", "disabled", "boolean"],
      ["required", "required", "boolean"],
      ["multiple", "multiple", "boolean"],
      ["size", "size", "unsigned", 0],
      ["autofocus", "autofocus", "boolean"],
    ]],
    ["HTMLTextAreaElement", [
      ["name", "name", "string"],
      ["placeholder", "placeholder", "string"],
      ["disabled", "disabled", "boolean"],
      ["required", "required", "boolean"],
      ["readOnly", "readonly", "boolean"],
      ["cols", "cols", "unsigned", 20],
      ["rows", "rows", "unsigned", 2],
      ["wrap", "wrap", "string"],
      ["autofocus", "autofocus", "boolean"],
    ]],
    ["HTMLOptionElement", [
      ["disabled", "disabled", "boolean"],
      ["defaultSelected", "selected", "boolean"],
    ]],
    ["HTMLOptGroupElement", [
      ["disabled", "disabled", "boolean"],
      ["label", "label", "string"],
    ]],
    ["HTMLFieldSetElement", [
      ["disabled", "disabled", "boolean"],
      ["name", "name", "string"],
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
    const iface = madeInterfaces[REFLECTED[i][0]];
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

  // SVG's element interfaces (SVG 2's IDL, M32): SVGElement over
  // Element, with `className` an SVGAnimatedString and its nearest
  // `svg`; the graphics and geometry elements' own above it, each
  // tag's interface its own. The geometry methods come with drawing
  // (docs/design/svg.md).
  const animatedStrings = new $WeakMap();
  class SVGAnimatedString {
    #element;
    #attribute;

    constructor(key, element, attribute) {
      if (key !== KEY) {
        throw illegalConstructor("SVGAnimatedString");
      }
      this.#element = element;
      this.#attribute = attribute;
    }

    get baseVal() {
      const value = native.attribute(handleOf(this.#element), this.#attribute);
      return value === null ? "" : value;
    }

    set baseVal(value) {
      setAttribute(handleOf(this.#element), this.#attribute, str(value), "baseVal");
    }

    get animVal() {
      return this.baseVal;
    }
  }
  expose(SVGAnimatedString, "SVGAnimatedString");

  class SVGElement extends Element {
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        throw illegalConstructor("SVGElement");
      }
      super(key, handle, kind, name);
    }

    get className() {
      self(this);
      let animated = WeakMapGet(animatedStrings, this);
      if (animated === undefined) {
        animated = new SVGAnimatedString(KEY, this, "class");
        WeakMapSet(animatedStrings, this, animated);
      }
      return animated;
    }

    // The nearest `svg` ancestor; null for the outermost.
    get ownerSVGElement() {
      let at = native.parent(self(this));
      while (at !== null && native.kind(at) === 1) {
        const element = wrap(at);
        if (nsOf(element) === SVG_NS && nameOf(element) === "svg") {
          return element;
        }
        at = native.parent(at);
      }
      return null;
    }

    get viewportElement() {
      return this.ownerSVGElement;
    }
  }
  expose(SVGElement, "SVGElement");

  // HTMLOrSVGElement's `nonce` (M33.1): the element's nonce, which its
  // attribute stops showing once the document hides it.
  include(
    {
      get nonce() {
        return native.nonce(self(this));
      },
      set nonce(value) {
        native.setNonce(self(this), str(value));
      },
    },
    [HTMLElement.prototype, SVGElement.prototype],
  );

  const svgInterfaces = new $Map();
  function svgElement(name, tags, base) {
    const cls = {
      [name]: class extends base {
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
      MapSet(svgInterfaces, tags[i], cls);
    }
    return cls;
  }
  const SVGGraphicsElement = svgElement("SVGGraphicsElement", [], SVGElement);
  const SVGGeometryElement = svgElement("SVGGeometryElement", [], SVGGraphicsElement);
  const SVGTextContentElement = svgElement("SVGTextContentElement", [], SVGGraphicsElement);
  const SVGTextPositioningElement = svgElement(
    "SVGTextPositioningElement",
    [],
    SVGTextContentElement,
  );
  const SVGGradientElement = svgElement("SVGGradientElement", [], SVGElement);

  // The geometry scripts ask of SVG (M32): the host answers from the
  // render tree of the element's outer svg, at the size its style gives.
  function svgAsk(element, query, a, b) {
    const answer = native.svgQuery(self(element), query, a, b);
    return answer.length === 0 ? null : answer;
  }
  function svgMethod(cls, name, fn) {
    defineProperty(cls.prototype, name, {
      value: fn,
      writable: true,
      enumerable: true,
      configurable: true,
    });
  }
  svgMethod(SVGGraphicsElement, "getBBox", function getBBox() {
    const n = svgAsk(this, "bbox", 0, 0);
    return n === null ? new DOMRect(0, 0, 0, 0) : new DOMRect(n[0], n[1], n[2], n[3]);
  });
  svgMethod(SVGGraphicsElement, "getCTM", function getCTM() {
    const n = svgAsk(this, "ctm", 0, 0);
    return n === null ? null : new DOMMatrix([n[0], n[1], n[2], n[3], n[4], n[5]]);
  });
  svgMethod(SVGTextContentElement, "getComputedTextLength", function getComputedTextLength() {
    const n = svgAsk(this, "textLength", 0, 0);
    return n === null ? 0 : n[0];
  });
  svgMethod(SVGGeometryElement, "getTotalLength", function getTotalLength() {
    const n = svgAsk(this, "totalLength", 0, 0);
    return n === null ? 0 : n[0];
  });
  svgMethod(SVGGeometryElement, "getPointAtLength", function getPointAtLength(distance) {
    need(arguments.length, 1, "getPointAtLength", "SVGGeometryElement");
    const n = svgAsk(this, "pointAtLength", $Number(distance), 0);
    return n === null ? new DOMPoint(0, 0) : new DOMPoint(n[0], n[1]);
  });
  svgMethod(SVGGeometryElement, "isPointInFill", function isPointInFill(point) {
    const p = pointInit(point, "isPointInFill", "SVGGeometryElement");
    const n = svgAsk(this, "inFill", p[0], p[1]);
    return n !== null && n[0] === 1;
  });
  svgMethod(SVGGeometryElement, "isPointInStroke", function isPointInStroke(point) {
    const p = pointInit(point, "isPointInStroke", "SVGGeometryElement");
    const n = svgAsk(this, "inStroke", p[0], p[1]);
    return n !== null && n[0] === 1;
  });
  const SVG_ELEMENTS = [
    ["SVGSVGElement", ["svg"], SVGGraphicsElement],
    ["SVGGElement", ["g"], SVGGraphicsElement],
    ["SVGDefsElement", ["defs"], SVGGraphicsElement],
    ["SVGUseElement", ["use"], SVGGraphicsElement],
    ["SVGSwitchElement", ["switch"], SVGGraphicsElement],
    ["SVGAElement", ["a"], SVGGraphicsElement],
    ["SVGImageElement", ["image"], SVGGraphicsElement],
    ["SVGForeignObjectElement", ["foreignObject"], SVGGraphicsElement],
    ["SVGRectElement", ["rect"], SVGGeometryElement],
    ["SVGCircleElement", ["circle"], SVGGeometryElement],
    ["SVGEllipseElement", ["ellipse"], SVGGeometryElement],
    ["SVGLineElement", ["line"], SVGGeometryElement],
    ["SVGPolylineElement", ["polyline"], SVGGeometryElement],
    ["SVGPolygonElement", ["polygon"], SVGGeometryElement],
    ["SVGPathElement", ["path"], SVGGeometryElement],
    ["SVGTextElement", ["text"], SVGTextPositioningElement],
    ["SVGTSpanElement", ["tspan"], SVGTextPositioningElement],
    ["SVGTextPathElement", ["textPath"], SVGTextContentElement],
    ["SVGLinearGradientElement", ["linearGradient"], SVGGradientElement],
    ["SVGRadialGradientElement", ["radialGradient"], SVGGradientElement],
    ["SVGStopElement", ["stop"], SVGElement],
    ["SVGSymbolElement", ["symbol"], SVGElement],
    ["SVGClipPathElement", ["clipPath"], SVGElement],
    ["SVGMaskElement", ["mask"], SVGElement],
    ["SVGPatternElement", ["pattern"], SVGElement],
    ["SVGMarkerElement", ["marker"], SVGElement],
    ["SVGFilterElement", ["filter"], SVGElement],
    ["SVGTitleElement", ["title"], SVGElement],
    ["SVGDescElement", ["desc"], SVGElement],
    ["SVGMetadataElement", ["metadata"], SVGElement],
    ["SVGStyleElement", ["style"], SVGElement],
    ["SVGScriptElement", ["script"], SVGElement],
    ["SVGViewElement", ["view"], SVGElement],
  ];
  for (let i = 0; i < SVG_ELEMENTS.length; i++) {
    svgElement(SVG_ELEMENTS[i][0], SVG_ELEMENTS[i][1], SVG_ELEMENTS[i][2]);
  }

  // MathML's elements are all MathMLElements (MathML Core).
  class MathMLElement extends Element {
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        throw illegalConstructor("MathMLElement");
      }
      super(key, handle, kind, name);
    }
  }
  expose(MathMLElement, "MathMLElement");

  // The interface an element with local name `tag` of the namespace
  // `ns` is an object of: SVG's for its name, else SVGElement; MathML's
  // MathMLElement; another namespace's Element; HTML's for its tag,
  // HTMLElement for a name with a hyphen (a custom element's),
  // HTMLUnknownElement for the rest.
  function elementInterface(tag, ns) {
    if (ns === SVG_NS) {
      return MapGet(svgInterfaces, localPart(tag)) || SVGElement;
    }
    if (ns === MATHML_NS) {
      return MathMLElement;
    }
    if (ns !== undefined && ns !== HTML_NS) {
      return Element;
    }
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
      return true;
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

  // ---- A worker's location and navigator (M31, HTML 10.3.1, 10.3.2) ------

  // The worker's script URL in parts, as the window's location reads its
  // document's: the worker's host answers.
  class WorkerLocation {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("WorkerLocation");
      }
    }

    get href() {
      return locationPart(0);
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

    toString() {
      return locationPart(0);
    }
  }
  expose(WorkerLocation, "WorkerLocation");
  const workerLocation = new WorkerLocation(KEY);

  // The members of the window's navigator a worker's has too.
  class WorkerNavigator {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("WorkerNavigator");
      }
    }
  }
  {
    const shared = ["appCodeName", "appName", "appVersion", "language", "languages", "onLine",
      "platform", "product", "userAgent"];
    for (let i = 0; i < shared.length; i++) {
      defineProperty(WorkerNavigator.prototype, shared[i],
        getOwnPropertyDescriptor(Navigator.prototype, shared[i]));
    }
  }
  expose(WorkerNavigator, "WorkerNavigator");
  const workerNavigator = new WorkerNavigator(KEY);

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

    get origin() {
      return native.origin();
    }

    get isSecureContext() {
      return native.secureContext();
    }

    get navigator() {
      return navigator;
    }

    get localStorage() {
      return storageArea(true);
    }

    get sessionStorage() {
      return storageArea(false);
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

    // The window's relatives among the tab's frames (M33.2): itself at
    // the top.
    get top() {
      return windowFor(native.frameTop(selfFrame()));
    }

    get parent() {
      const parent = native.frameParent(selfFrame());
      return parent < 0 ? global : windowFor(parent);
    }

    get frames() {
      return global;
    }

    get length() {
      return native.frameLength(selfFrame());
    }

    get opener() {
      return null;
    }

    // No realm here holds another frame's element (D24).
    get frameElement() {
      return null;
    }

    // The navigable's name: its iframe's `name`, until a script sets it.
    get name() {
      return native.windowName();
    }

    set name(value) {
      native.setWindowName(str(value));
    }

    get closed() {
      return false;
    }

    get performance() {
      return performance;
    }

    // HTML's simple dialogs (section 8.8.1), which block nothing (M26):
    // the message goes to the host, and the answer is a browser's that
    // does not show dialogs.
    alert() {
      native.dialog("alert", arguments.length === 0 ? "" : str(arguments[0]));
    }

    confirm() {
      native.dialog("confirm", arguments[0] === undefined ? "" : str(arguments[0]));
      return false;
    }

    prompt() {
      native.dialog("prompt", arguments[0] === undefined ? "" : str(arguments[0]));
      return null;
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

  // ---- Frames (M33.2, docs/design/hardening.md) -------------------------

  // Every frame is another origin's to this realm, its own origin's
  // too (D24): its window answers what HTML 7.2.3 lets another origin
  // ask, and nothing else. A frame is named by its number in the tab;
  // a native answers -1 for none.

  const hasInstanceSymbol = Symbol.hasInstance;
  const concatSpreadableSymbol = Symbol.isConcatSpreadable;

  // This window's frame.
  let selfFrameId;
  function selfFrame() {
    if (selfFrameId === undefined) {
      try {
        selfFrameId = native.frameSelf();
      } catch {
        // A host that has no frames: the top.
        selfFrameId = 0;
      }
    }
    return selfFrameId;
  }

  // The windows of other frames, one object a frame.
  const frameWindows = new $Map();
  const frameLocations = new $Map();

  // The window of frame `id`: this one's own, another's object, or
  // `null` for none.
  function windowFor(id) {
    if (id < 0) {
      return null;
    }
    if (id === selfFrame()) {
      return global;
    }
    let window = MapGet(frameWindows, id);
    if (window === undefined) {
      window = frameWindow(id);
      MapSet(frameWindows, id, window);
    }
    return window;
  }

  // The reference browser's words for what another origin may not do.
  function blocked(verb, name, iface) {
    const what = typeof name === "symbol" ? "a named property" : "a named property '" + name + "'";
    const where = verb === "read" ? " from '" : " on '";
    return new $DOMException(
      "Failed to " + verb + " " + what + where + iface + "': Blocked a frame with origin \"" +
        native.origin() + "\" from accessing a cross-origin frame.",
      "SecurityError",
    );
  }

  // Navigates frame `id` to `url`, as setting its location does: a
  // `SecurityError` when this window may not navigate it.
  function navigateFrame(id, url, replace, method) {
    try {
      native.frameNavigate(id, str(url), replace);
    } catch (error) {
      if (method !== undefined) {
        throw failed(error, method, "Location");
      }
      if (error instanceof $DOMException) {
        throw new $DOMException(
          "Failed to set a named property 'href' on 'Location': " + error.message,
          error.name,
        );
      }
      throw error;
    }
  }

  // A cross-origin Location (HTML 7.2.3): `href` to set and `replace`.
  function frameLocation(id) {
    let location = MapGet(frameLocations, id);
    if (location !== undefined) {
      return location;
    }
    const replace = function replace(url) {
      need(arguments.length, 1, "replace", "Location");
      navigateFrame(id, url, true, "replace");
    };
    const target = ObjectCreate(null);
    const read = (prop) => {
      if (prop === "replace") {
        return replace;
      }
      if (prop === "then" || prop === toStringTag || prop === hasInstanceSymbol ||
        prop === concatSpreadableSymbol) {
        return undefined;
      }
      throw blocked("read", prop, "Location");
    };
    location = new $Proxy(target, {
      get(_, prop) {
        return read(prop);
      },
      set(_, prop, value) {
        if (prop === "href") {
          navigateFrame(id, value, false);
          return true;
        }
        throw blocked("set", prop, "Location");
      },
      has(_, prop) {
        if (prop === "href" || prop === "replace" || prop === "then") {
          return true;
        }
        throw blocked("read", prop, "Location");
      },
      getOwnPropertyDescriptor(_, prop) {
        if (prop === "href") {
          return {
            get: undefined,
            set: function (value) {
              navigateFrame(id, value, false);
            },
            enumerable: false,
            configurable: true,
          };
        }
        return { value: read(prop), writable: false, enumerable: false, configurable: true };
      },
      ownKeys() {
        return ["href", "replace", "then", toStringTag, hasInstanceSymbol,
          concatSpreadableSymbol];
      },
      defineProperty(_, prop) {
        throw blocked("set", prop, "Location");
      },
      deleteProperty(_, prop) {
        throw blocked("set", prop, "Location");
      },
      getPrototypeOf() {
        return null;
      },
      setPrototypeOf(_, prototype) {
        return prototype === null;
      },
      preventExtensions() {
        return false;
      },
    });
    MapSet(frameLocations, id, location);
    return location;
  }

  // The names another origin's window has (the reference browser's
  // order).
  const FRAME_WINDOW_KEYS = ["window", "self", "location", "closed", "frames", "length", "top",
    "opener", "parent", "blur", "close", "focus", "postMessage", "then"];

  // A cross-origin WindowProxy (HTML 7.2.3) for frame `id`.
  function frameWindow(id) {
    const methods = {
      blur: function blur() {},
      close: function close() {},
      focus: function focus() {},
      postMessage: function postMessage(message) {
        const posted = postArguments(arguments, "postMessage", "Window");
        native.framePost(id, posted[0], posted[1]);
      },
    };
    let window;
    const read = (prop) => {
      switch (prop) {
        case "window":
        case "self":
        case "frames":
          return window;
        case "location":
          return frameLocation(id);
        case "closed":
          return native.frameClosed(id);
        case "length":
          return native.frameLength(id);
        case "top":
          return windowFor(native.frameTop(id));
        case "parent": {
          const parent = native.frameParent(id);
          return windowFor(parent < 0 ? id : parent);
        }
        case "opener":
          return null;
        case "then":
          return undefined;
        case "blur":
        case "close":
        case "focus":
        case "postMessage":
          return methods[prop];
      }
      if (prop === toStringTag || prop === hasInstanceSymbol ||
        prop === concatSpreadableSymbol) {
        return undefined;
      }
      if (typeof prop === "string") {
        const index = arrayIndex(prop);
        const child = index >= 0 ? native.frameChild(id, index) : native.frameNamed(id, prop);
        if (child >= 0) {
          return windowFor(child);
        }
      }
      throw blocked("read", prop, "Window");
    };
    window = new $Proxy(ObjectCreate(null), {
      get(_, prop) {
        return read(prop);
      },
      set(_, prop, value) {
        if (prop === "location") {
          navigateFrame(id, value, false);
          return true;
        }
        throw blocked("set", prop, "Window");
      },
      has(_, prop) {
        if (typeof prop === "string" && includesString(FRAME_WINDOW_KEYS, prop)) {
          return true;
        }
        return read(prop) !== undefined;
      },
      getOwnPropertyDescriptor(_, prop) {
        if (prop === "location") {
          return {
            get: function () {
              return frameLocation(id);
            },
            set: function (value) {
              navigateFrame(id, value, false);
            },
            enumerable: false,
            configurable: true,
          };
        }
        return { value: read(prop), writable: false, enumerable: false, configurable: true };
      },
      ownKeys() {
        const keys = [];
        const length = native.frameLength(id);
        for (let i = 0; i < length; i++) {
          ArrayPush(keys, $String(i));
        }
        for (let i = 0; i < FRAME_WINDOW_KEYS.length; i++) {
          ArrayPush(keys, FRAME_WINDOW_KEYS[i]);
        }
        ArrayPush(keys, toStringTag, hasInstanceSymbol, concatSpreadableSymbol);
        return keys;
      },
      defineProperty(_, prop) {
        throw blocked("set", prop, "Window");
      },
      deleteProperty(_, prop) {
        throw blocked("set", prop, "Window");
      },
      getPrototypeOf() {
        return null;
      },
      setPrototypeOf(_, prototype) {
        return prototype === null;
      },
      preventExtensions() {
        return false;
      },
    });
    return window;
  }

  // This window's frames by index and by name (HTML's WindowProperties
  // object, between Window.prototype and EventTarget.prototype): its
  // properties the engine's list of the window's child frames makes
  // after each change (`frameList`), each answering as the tab now is.
  const windowProperties = ObjectCreate(EventTarget.prototype);
  let frameKeys = [];
  if (!WORKER) {
    setPrototypeOf(Window.prototype, windowProperties);
  }

  // The window's child frames are now as many as `names`, those names
  // their elements' (M33.2).
  function frameList(names) {
    for (let i = 0; i < frameKeys.length; i++) {
      ReflectDeleteProperty(windowProperties, frameKeys[i]);
    }
    frameKeys = [];
    const property = (key, ask) => {
      if (ReflectGetOwnPropertyDescriptor(windowProperties, key) !== undefined) {
        return;
      }
      defineProperty(windowProperties, key, {
        get: function () {
          const id = ask();
          return id < 0 ? undefined : windowFor(id);
        },
        enumerable: false,
        configurable: true,
      });
      ArrayPush(frameKeys, key);
    };
    for (let i = 0; i < names.length; i++) {
      property($String(i), () => native.frameChild(selfFrame(), i));
    }
    for (let i = 0; i < names.length; i++) {
      const name = names[i];
      if (name !== "" && arrayIndex(name) < 0) {
        property(name, () => native.frameNamed(selfFrame(), name));
      }
    }
    return nothing();
  }

  // The `sandbox` tokens the reference browser knows (M33.2, p2).
  const SANDBOX_TOKENS = [
    "allow-downloads",
    "allow-forms",
    "allow-modals",
    "allow-orientation-lock",
    "allow-pointer-lock",
    "allow-popups",
    "allow-popups-to-escape-sandbox",
    "allow-presentation",
    "allow-same-origin",
    "allow-scripts",
    "allow-top-navigation",
    "allow-top-navigation-by-user-activation",
    "allow-storage-access-by-user-activation",
  ];
  const sandboxLists = new $WeakMap();

  // An iframe's frame's window, another origin's to this realm (D24),
  // and its document, which no realm here reaches into; its `sandbox`,
  // the same list at every reading, which setting sets its value.
  include(
    {
      get contentWindow() {
        return windowFor(native.frameOf(self(this)));
      },
      get contentDocument() {
        self(this);
        return null;
      },
      get sandbox() {
        const handle = self(this);
        let list = WeakMapGet(sandboxLists, this);
        if (list === undefined) {
          list = tokenList(this, handle, "sandbox", SANDBOX_TOKENS);
          WeakMapSet(sandboxLists, this, list);
        }
        return list;
      },
      set sandbox(value) {
        const handle = self(this);
        setAttribute(handle, "sandbox", str(value), "sandbox");
      },
    },
    [MapGet(interfaces, "iframe").prototype],
  );

  // ---- A dedicated worker's global scope (M31, HTML section 10.2) -----

  // WorkerGlobalScope's members are its prototype's and the dedicated
  // scope's the global's own, as the reference browser has them.
  class WorkerGlobalScope extends EventTarget {
    constructor() {
      throw illegalConstructor("WorkerGlobalScope");
    }

    get self() {
      return global;
    }

    get origin() {
      return native.origin();
    }

    get isSecureContext() {
      return native.secureContext();
    }

    get crossOriginIsolated() {
      return false;
    }

    get performance() {
      return performance;
    }

    get location() {
      return workerLocation;
    }

    get navigator() {
      return workerNavigator;
    }

    // importScripts (HTML section 10.3.3): every URL parsed against the
    // worker's, then each script fetched and run in order, what it
    // throws thrown on; one that fails to load a NetworkError.
    importScripts() {
      const a = arguments;
      const where = prefix("importScripts", "WorkerGlobalScope");
      const urls = [];
      for (let i = 0; i < a.length; i++) {
        const text = str(a[i]);
        const url = native.resolveUrl(text);
        if (url === null) {
          throw new $DOMException(where + "The URL '" + text + "' is invalid.", "SyntaxError");
        }
        ArrayPush(urls, url);
      }
      for (let i = 0; i < urls.length; i++) {
        const script = native.importScript(urls[i]);
        if (script.length === 0) {
          throw new $DOMException(where + "The script at '" + urls[i] + "' failed to load.",
            "NetworkError");
        }
        native.evaluate(script[0], script[1]);
      }
    }
  }
  expose(WorkerGlobalScope, "WorkerGlobalScope");

  class DedicatedWorkerGlobalScope extends WorkerGlobalScope {
    constructor() {
      throw illegalConstructor("DedicatedWorkerGlobalScope");
    }
  }
  expose(DedicatedWorkerGlobalScope, "DedicatedWorkerGlobalScope");

  setPrototypeOf(global, WORKER ? DedicatedWorkerGlobalScope.prototype : Window.prototype);
  // The global is tagged by its prototype, not by QuickJS-ng's own tag.
  ReflectDeleteProperty(global, toStringTag);
  if (WORKER) {
    // [Replaceable] (HTML section 10.2.1.2): a script's own `name`
    // replaces it.
    defineProperty(global, "name", {
      get() {
        return WORKER_NAME;
      },
      set(value) {
        defineProperty(global, "name", {
          value,
          writable: true,
          enumerable: true,
          configurable: true,
        });
      },
      enumerable: true,
      configurable: true,
    });
  }

  // The document's object, made the first time a script asks: the
  // realm is made before any host is there to ask.
  let document = null;
  function theDocument() {
    if (document === null) {
      document = wrap(native.document());
    }
    return document;
  }
  windowOnly("document", {
    get: theDocument,
    enumerable: true,
    configurable: false,
  });

  // `window.event` (HTML section 7.2.2.1): the event whose listener
  // runs; replaceable.
  windowOnly("event", {
    get() {
      return currentEvent;
    },
    set(value) {
      defineProperty(global, "event", {
        value,
        writable: true,
        enumerable: true,
        configurable: true,
      });
    },
    enumerable: true,
    configurable: true,
  });

  // `reportError` (HTML section 8.1.4.6): `e` reported as an uncaught
  // exception is.
  defineProperty(global, "reportError", {
    value: function reportError(e) {
      need(arguments.length, 1, "reportError", GLOBAL_INTERFACE);
      reportException(e);
    },
    writable: true,
    enumerable: true,
    configurable: true,
  });

  // ---- Event handlers (HTML section 8.1.8, M26) ----------------------------

  // The event types of GlobalEventHandlers' handlers (and of those other
  // specifications add to it), of WindowEventHandlers', and of
  // DocumentAndElementEventHandlers'.
  const GLOBAL_HANDLERS = [
    "abort", "animationcancel", "animationend", "animationiteration", "animationstart",
    "auxclick", "beforeinput", "beforematch", "beforetoggle", "blur", "cancel", "canplay",
    "canplaythrough", "change", "click", "close", "contextlost", "contextmenu",
    "contextrestored", "cuechange", "dblclick", "drag", "dragend", "dragenter", "dragleave",
    "dragover", "dragstart", "drop", "durationchange", "emptied", "ended", "error", "focus",
    "formdata", "gotpointercapture", "input", "invalid", "keydown", "keypress", "keyup", "load",
    "loadeddata", "loadedmetadata", "loadstart", "lostpointercapture", "mousedown",
    "mouseenter", "mouseleave", "mousemove", "mouseout", "mouseover", "mouseup", "pause",
    "play", "playing", "pointercancel", "pointerdown", "pointerenter", "pointerleave",
    "pointermove", "pointerout", "pointerover", "pointerup", "progress", "ratechange", "reset",
    "resize", "scroll", "scrollend", "securitypolicyviolation", "seeked", "seeking", "select",
    "selectionchange", "selectstart", "slotchange", "stalled", "submit", "suspend",
    "timeupdate", "toggle", "transitioncancel", "transitionend", "transitionrun",
    "transitionstart", "volumechange", "waiting", "webkitanimationend",
    "webkitanimationiteration", "webkitanimationstart", "webkittransitionend", "wheel",
  ];
  const WINDOW_HANDLERS = [
    "afterprint", "beforeprint", "beforeunload", "hashchange", "languagechange", "message",
    "messageerror", "offline", "online", "pagehide", "pagereveal", "pageshow", "pageswap",
    "popstate", "rejectionhandled", "storage", "unhandledrejection", "unload",
  ];
  const DOCUMENT_ELEMENT_HANDLERS = ["copy", "cut", "paste"];
  // The handlers a `body` or `frameset` element's attributes set on its
  // window (HTML section 8.1.8.2): the window-reflecting body element
  // event handler set and WindowEventHandlers'.
  const WINDOW_REFLECTING = ["blur", "error", "focus", "load", "resize", "scroll"];

  function typeSet(lists) {
    const set = new $Map();
    for (let i = 0; i < lists.length; i++) {
      for (let j = 0; j < lists[i].length; j++) {
        MapSet(set, lists[i][j], true);
      }
    }
    return set;
  }
  const elementHandlers = typeSet([GLOBAL_HANDLERS, DOCUMENT_ELEMENT_HANDLERS]);
  const bodyWindowHandlers = typeSet([WINDOW_REFLECTING, WINDOW_HANDLERS]);

  // A target's handler for `type` (HTML's event handler): its value, a
  // callback or null; the element whose content attribute's text it is
  // until that is compiled; and its listener while it is active.
  function handlerOf(target, type) {
    const handlers = handlersOf(target);
    let handler = MapGet(handlers, type);
    if (handler === undefined) {
      handler = { value: null, element: null, listener: null };
      MapSet(handlers, type, handler);
    }
    return handler;
  }

  // HTML's "getting the current value of the event handler": its
  // attribute's text compiled first, if it has not been.
  function currentHandler(target, type) {
    const handler = MapGet(handlersOf(target), type);
    if (handler === undefined) {
      return null;
    }
    if (handler.element !== null) {
      const element = handler.element;
      handler.element = null;
      handler.value = compileHandler(target, type, element);
    }
    return handler.value;
  }

  // Sets the handler to `value`, or to `element`'s attribute's text
  // when `element` is given: its listener is added the first time, and
  // keeps its place among the target's listeners after; with neither,
  // the handler is deactivated, its listener removed.
  function setHandler(target, type, value, element) {
    const handler = handlerOf(target, type);
    handler.value = value;
    handler.element = element;
    if (value === null && element === null) {
      if (handler.listener !== null) {
        removeListener(listenersOf(target), handler.listener);
        handler.listener = null;
      }
      return;
    }
    if (handler.listener === null) {
      const listener = {
        type,
        callback: handlerCallback(target, type),
        capture: false,
        passive: defaultPassive(target, type),
        once: false,
        removed: false,
      };
      handler.listener = listener;
      ArrayPush(listenersOf(target), listener);
    }
  }

  // HTML's event handler processing algorithm, as the handler's
  // listener: the handler called with the event, or, for an ErrorEvent
  // at the window, with its five members; a return value of false
  // cancels the event, and of true the window's `error` event.
  function handlerCallback(target, type) {
    return function (event) {
      const callback = currentHandler(target, type);
      if (typeof callback !== "function") {
        return;
      }
      const s = eventState(event);
      const special = type === "error" && (s.kinds & ERROR) !== 0 && s.currentTarget === global;
      if (special) {
        const f = s.fields;
        const result = ReflectApply(callback, s.currentTarget, [
          f.message, f.filename, f.lineno, f.colno, f.error,
        ]);
        if (result === true) {
          cancel(s);
        }
      } else if (ReflectApply(callback, s.currentTarget, [event]) === false) {
        cancel(s);
      }
    };
  }

  // A content attribute's handler compiled (HTML section 8.1.8.1): a
  // function of `event` (of five arguments for the window's `error`)
  // whose body is the attribute's text, with the document, the
  // element's form owner and the element on its scope chain. Text that
  // does not compile is reported, and the handler is null.
  function compileHandler(target, type, element) {
    const handle = handleOf(element);
    const body = native.attribute(handle, "on" + type);
    if (body === null) {
      return null;
    }
    const params = type === "error" && target === global
      ? "event, source, lineno, colno, error"
      : "event";
    // The document's policies may refuse it (M33.1): no handler then.
    if (native.codeAllowed("handler", body, handle) !== "") {
      return null;
    }
    const owner = native.formOwner(handle);
    const url = native.documentUrl();
    const source = "(function () { with (arguments[0]) with (arguments[1]) " +
      "with (arguments[2]) return function on" + type + "(" + params + ") {\n" + body +
      "\n}; })";
    try {
      const scope = native.evaluate(source, url === null ? "" : url);
      return ReflectApply(scope, undefined, [
        theDocument(),
        owner === null ? ObjectCreate(null) : wrap(owner),
        element,
      ]);
    } catch (error) {
      reportException(error);
      return null;
    }
  }

  // An attribute named `name` of `element` was set (`present`) or
  // removed: an `on...` one is the text of the element's handler, or,
  // on a body or frameset, of its window's.
  function attributeHandler(element, name, present) {
    if (CharCodeAt(name, 0) !== 111 || CharCodeAt(name, 1) !== 110) {
      return;
    }
    const type = StringSlice(name, 2);
    const tag = nameOf(element);
    let target = null;
    if ((tag === "body" || tag === "frameset") && MapHas(bodyWindowHandlers, type)) {
      target = global;
    } else if (MapHas(elementHandlers, type)) {
      target = element;
    }
    if (target !== null) {
      setHandler(target, type, null, present ? element : null);
    }
  }

  // The IDL attributes of `types`' handlers on `prototype`: the
  // object's own, or its window's when `windows`. A value that is not an
  // object is null ([LegacyTreatNonObjectAsNull]).
  function defineHandlers(prototype, types, windows) {
    for (let i = 0; i < types.length; i++) {
      const type = types[i];
      defineProperty(prototype, "on" + type, {
        get() {
          if (windows) {
            self(this);
          }
          return currentHandler(windows ? global : handlerTarget(this), type);
        },
        set(value) {
          if (windows) {
            self(this);
          }
          const callback =
            value !== null && (typeof value === "object" || typeof value === "function")
              ? value
              : null;
          setHandler(windows ? global : handlerTarget(this), type, callback, null);
        },
        enumerable: true,
        configurable: true,
      });
    }
  }

  // `this` as a target with handlers, or Illegal invocation.
  function handlerTarget(target) {
    listenersOf(target);
    return target;
  }

  defineHandlers(HTMLElement.prototype, GLOBAL_HANDLERS, false);
  defineHandlers(HTMLElement.prototype, DOCUMENT_ELEMENT_HANDLERS, false);
  defineHandlers(SVGElement.prototype, GLOBAL_HANDLERS, false);
  defineHandlers(MathMLElement.prototype, GLOBAL_HANDLERS, false);
  defineHandlers(Document.prototype, GLOBAL_HANDLERS, false);
  defineHandlers(Document.prototype, DOCUMENT_ELEMENT_HANDLERS, false);
  defineHandlers(Document.prototype, ["readystatechange", "visibilitychange"], false);
  defineHandlers(Window.prototype, GLOBAL_HANDLERS, false);
  defineHandlers(Window.prototype, WINDOW_HANDLERS, false);
  // A worker's (M31): WorkerGlobalScope's, and the dedicated scope's
  // message handlers, the global's own.
  defineHandlers(WorkerGlobalScope.prototype,
    ["error", "languagechange", "rejectionhandled", "unhandledrejection"], false);
  if (WORKER) {
    defineHandlers(global, ["message", "messageerror"], false);
  }
  defineHandlers(AbortSignal.prototype, ["abort"], false);
  const BODIES = ["body", "frameset"];
  for (let i = 0; i < BODIES.length; i++) {
    defineHandlers(MapGet(interfaces, BODIES[i]).prototype, WINDOW_REFLECTING, true);
    defineHandlers(MapGet(interfaces, BODIES[i]).prototype, WINDOW_HANDLERS, true);
  }

  // ---- Form controls (HTML section 4.10, M26.7) ---------------------------

  // The collections a control or a form keeps, made at the first ask:
  // the same object each time.
  const controlLists = new $WeakMap();

  function controlList(element, name, make) {
    let lists = WeakMapGet(controlLists, element);
    if (lists === undefined) {
      lists = ObjectCreate(null);
      WeakMapSet(controlLists, element, lists);
    }
    if (lists[name] === undefined) {
      lists[name] = make();
    }
    return lists[name];
  }

  // A `select`'s options, live (HTML section 4.10.7): a collection that
  // also adds, removes and selects.
  class HTMLOptionsCollection extends HTMLCollection {
    get length() {
      return items(stateOf(this)).length;
    }

    set length(value) {
      const state = stateOf(this);
      setOptionsLength(state.owner, unsigned(value));
    }

    get selectedIndex() {
      return native.selectedIndex(self(stateOf(this).owner));
    }

    set selectedIndex(value) {
      setSelectedIndex(self(stateOf(this).owner), value | 0);
    }

    add(element, before) {
      need(arguments.length, 1, "add", "HTMLOptionsCollection");
      addOption(stateOf(this).owner, element, before, "HTMLOptionsCollection");
    }

    remove(index) {
      need(arguments.length, 1, "remove", "HTMLOptionsCollection");
      removeOption(stateOf(this).owner, index | 0);
    }
  }
  expose(HTMLOptionsCollection, "HTMLOptionsCollection");

  // A form's listed controls, live (HTML section 4.10.3): by index, and
  // by name, a RadioNodeList where more than one has it.
  class HTMLFormControlsCollection extends HTMLCollection {
    namedItem(name) {
      const state = stateOf(this);
      need(arguments.length, 1, "namedItem", "HTMLFormControlsCollection");
      return formNamedItem(state, str(name));
    }
  }
  expose(HTMLFormControlsCollection, "HTMLFormControlsCollection");

  // The controls of a form named `name`, by ID or `name`: none, one, or
  // a RadioNodeList of them all, live.
  function formNamedItem(state, name) {
    if (name === "") {
      return null;
    }
    const named = () => {
      const all = items(state);
      const found = [];
      for (let i = 0; i < all.length; i++) {
        if (native.attribute(all[i], "id") === name || native.attribute(all[i], "name") === name) {
          ArrayPush(found, all[i]);
        }
      }
      return found;
    };
    const found = named();
    if (found.length === 0) {
      return null;
    }
    if (found.length === 1) {
      return wrap(found[0]);
    }
    return collection(RadioNodeList, state.owner, named, true);
  }

  // A form's controls of one name (HTML section 2.7.3): its `value` is
  // the checked radio button's.
  class RadioNodeList extends NodeList {
    get value() {
      const handles = items(stateOf(this));
      for (let i = 0; i < handles.length; i++) {
        const h = handles[i];
        if (isRadio(h) && native.checkedness(h)) {
          return native.controlValue(h);
        }
      }
      return "";
    }

    set value(value) {
      const text = str(value);
      const handles = items(stateOf(this));
      for (let i = 0; i < handles.length; i++) {
        const h = handles[i];
        if (isRadio(h) && native.controlValue(h) === text) {
          native.setCheckedness(h, true);
          return;
        }
      }
    }
  }
  expose(RadioNodeList, "RadioNodeList");

  function isRadio(handle) {
    return native.tag(handle) === "input" && native.inputType(handle) === "radio";
  }

  // Sets a select's selected index (HTML section 4.10.7): that option
  // selected, the rest not; one out of range selects none.
  function setSelectedIndex(handle, index) {
    const options = native.options(handle);
    for (let i = 0; i < options.length; i++) {
      if (i !== index && native.selectedness(options[i])) {
        native.setSelectedness(options[i], false);
      }
    }
    if (index >= 0 && index < options.length) {
      native.setSelectedness(options[index], true);
    }
  }

  // HTMLOptionsCollection's length setter: options added at the end, or
  // the last ones removed.
  function setOptionsLength(select, length) {
    const handle = self(select);
    let options = native.options(handle);
    if (length > 100000) {
      return;
    }
    while (options.length < length) {
      select.appendChild(select.ownerDocument.createElement("option"));
      options = native.options(handle);
    }
    for (let i = options.length - 1; i >= length; i--) {
      wrap(options[i]).remove();
    }
  }

  // `add(element, before)` of a select or its options (HTML section
  // 4.10.7): an option or an optgroup, before an element or the option
  // at an index, else at the end.
  function addOption(select, element, before, iface) {
    const tag = isNode(element) ? native.tag(self(element)) : "";
    if (tag !== "option" && tag !== "optgroup") {
      throw new $TypeError(
        prefix("add", iface) +
          "The provided value is not of type '(HTMLOptGroupElement or HTMLOptionElement)'.",
      );
    }
    let reference = null;
    if (typeof before === "number") {
      const options = native.options(self(select));
      reference = before >= 0 && before < options.length ? wrap(options[before]) : null;
    } else if (before !== null && before !== undefined) {
      reference = before;
    }
    const parent = reference === null ? select : reference.parentNode;
    if (reference !== null && !select.contains(reference)) {
      throw domError(
        "NotFoundError",
        "The node before which the new node is to be inserted is not a child of this node.",
        "add",
        iface,
      );
    }
    parent.insertBefore(element, reference);
  }

  function removeOption(select, index) {
    const options = native.options(self(select));
    if (index >= 0 && index < options.length) {
      wrap(options[index]).remove();
    }
  }

  // An `input`'s, a `textarea`'s, a `select`'s or an `option`'s `value`.
  function valueSetter(handle, value, iface) {
    switch (native.valueMode(handle)) {
      case "value":
        native.setControlValue(handle, value);
        return;
      case "default":
      case "default/on":
        setAttribute(handle, "value", value, "value");
        return;
      case "filename":
        if (value !== "") {
          throw new $DOMException(
            "Failed to set the 'value' property on '" + iface +
              "': This input element accepts a filename, which may only be programmatically set to the empty string.",
            "InvalidStateError",
          );
        }
        return;
      default:
        setAttribute(handle, "value", value, "value");
    }
  }

  // The selection API of a text control (HTML section 4.10.20), in
  // UTF-16 code units: `null` where it does not apply.
  function selectionApplies(handle) {
    if (native.tag(handle) === "textarea") {
      return true;
    }
    const kind = native.inputType(handle);
    return kind === "text" || kind === "search" || kind === "url" || kind === "tel" ||
      kind === "password";
  }

  function noSelection(handle, property) {
    return new $DOMException(
      "Failed to set the '" + property + "' property on 'HTMLInputElement': The input " +
        "element's type ('" + native.inputType(handle) + "') does not support selection.",
      "InvalidStateError",
    );
  }

  // No direction reads as forward, as on Windows and Linux in the
  // reference browser, where the platform knows no other (M26.8).
  const DIRECTIONS = ["forward", "forward", "backward"];

  // The host clamps both ends to the value.
  function setSelection(handle, start, end, direction) {
    start = start > end ? end : start;
    const d = direction === "forward" ? 1 : direction === "backward" ? 2 : 0;
    native.setTextSelection(handle, start, end, d);
  }

  const textControlMembers = {
    get value() {
      const handle = self(this);
      return native.controlValue(handle);
    },

    set value(value) {
      const handle = self(this);
      const iface = native.tag(handle) === "textarea" ? "HTMLTextAreaElement" : "HTMLInputElement";
      valueSetter(handle, strOrEmpty(value), iface);
    },

    get form() {
      return wrap(native.formOwner(self(this)));
    },

    get selectionStart() {
      const handle = self(this);
      return selectionApplies(handle) ? native.textSelection(handle)[0] : null;
    },

    set selectionStart(value) {
      const handle = self(this);
      if (!selectionApplies(handle)) {
        throw noSelection(handle, "selectionStart");
      }
      const s = native.textSelection(handle);
      const start = unsigned(value);
      setSelection(handle, start, s[1] < start ? start : s[1], DIRECTIONS[s[2]]);
    },

    get selectionEnd() {
      const handle = self(this);
      return selectionApplies(handle) ? native.textSelection(handle)[1] : null;
    },

    set selectionEnd(value) {
      const handle = self(this);
      if (!selectionApplies(handle)) {
        throw noSelection(handle, "selectionEnd");
      }
      const s = native.textSelection(handle);
      setSelection(handle, s[0], unsigned(value), DIRECTIONS[s[2]]);
    },

    get selectionDirection() {
      const handle = self(this);
      return selectionApplies(handle) ? DIRECTIONS[native.textSelection(handle)[2]] : null;
    },

    set selectionDirection(value) {
      const handle = self(this);
      if (!selectionApplies(handle)) {
        throw noSelection(handle, "selectionDirection");
      }
      const s = native.textSelection(handle);
      setSelection(handle, s[0], s[1], str(value));
    },

    setSelectionRange(start, end, direction) {
      const handle = self(this);
      need(arguments.length, 2, "setSelectionRange", "HTMLInputElement");
      if (!selectionApplies(handle)) {
        throw domError(
          "InvalidStateError",
          "The input element's type ('" + native.inputType(handle) +
            "') does not support selection.",
          "setSelectionRange",
          "HTMLInputElement",
        );
      }
      setSelection(handle, unsigned(start), unsigned(end),
        direction === undefined ? "none" : str(direction));
    },

    select() {
      const handle = self(this);
      if (!selectionApplies(handle)) {
        return;
      }
      setSelection(handle, 0, native.controlValue(handle).length, "none");
    },

    get labels() {
      const element = this;
      const handle = self(this);
      return controlList(element, "labels", () =>
        collection(NodeList, element, () => native.labels(handle), true));
    },
  };
  include(textControlMembers, [madeInterfaces.HTMLInputElement.prototype, madeInterfaces.HTMLTextAreaElement.prototype]);

  include(
    {
      get type() {
        return native.inputType(self(this));
      },

      set type(value) {
        setAttribute(self(this), "type", str(value), "type");
      },

      get checked() {
        return native.checkedness(self(this));
      },

      set checked(value) {
        native.setCheckedness(self(this), !!value);
      },

      get indeterminate() {
        return native.indeterminate(self(this));
      },

      set indeterminate(value) {
        native.setIndeterminate(self(this), !!value);
      },
    },
    [madeInterfaces.HTMLInputElement.prototype],
  );

  include(
    {
      get type() {
        self(this);
        return "textarea";
      },

      get defaultValue() {
        return this.textContent;
      },

      set defaultValue(value) {
        this.textContent = strOrEmpty(value);
      },

      get textLength() {
        return native.controlValue(self(this)).length;
      },
    },
    [madeInterfaces.HTMLTextAreaElement.prototype],
  );

  include(
    {
      get type() {
        return this.multiple ? "select-multiple" : "select-one";
      },

      get options() {
        const element = this;
        const handle = self(this);
        return controlList(element, "options", () =>
          collection(HTMLOptionsCollection, element, () => native.options(handle), true));
      },

      get selectedOptions() {
        const element = this;
        const handle = self(this);
        return controlList(element, "selectedOptions", () =>
          collection(HTMLCollection, element, () => {
            const all = native.options(handle);
            const chosen = [];
            for (let i = 0; i < all.length; i++) {
              if (native.selectedness(all[i])) {
                ArrayPush(chosen, all[i]);
              }
            }
            return chosen;
          }, true));
      },

      get length() {
        return native.options(self(this)).length;
      },

      set length(value) {
        setOptionsLength(this, unsigned(value));
      },

      get selectedIndex() {
        return native.selectedIndex(self(this));
      },

      set selectedIndex(value) {
        setSelectedIndex(self(this), value | 0);
      },

      get value() {
        return native.controlValue(self(this));
      },

      set value(value) {
        const handle = self(this);
        const text = str(value);
        const options = native.options(handle);
        let found = -1;
        for (let i = 0; i < options.length; i++) {
          if (found < 0 && native.controlValue(options[i]) === text) {
            found = i;
          }
        }
        setSelectedIndex(handle, found);
      },

      get form() {
        return wrap(native.formOwner(self(this)));
      },

      get labels() {
        const element = this;
        const handle = self(this);
        return controlList(element, "labels", () =>
          collection(NodeList, element, () => native.labels(handle), true));
      },

      item(index) {
        const options = native.options(self(this));
        const i = unsigned(index);
        return i < options.length ? wrap(options[i]) : null;
      },

      namedItem(name) {
        return this.options.namedItem(name);
      },

      add(element, before) {
        self(this);
        need(arguments.length, 1, "add", "HTMLSelectElement");
        addOption(this, element, before, "HTMLSelectElement");
      },

      remove(index) {
        self(this);
        if (arguments.length === 0) {
          // ChildNode's remove(), which the select's own overloads.
          const parent = this.parentNode;
          if (parent !== null) {
            parent.removeChild(this);
          }
          return;
        }
        removeOption(this, index | 0);
      },
    },
    [madeInterfaces.HTMLSelectElement.prototype],
  );

  include(
    {
      get selected() {
        return native.selectedness(self(this));
      },

      set selected(value) {
        native.setSelectedness(self(this), !!value);
      },

      get index() {
        const handle = self(this);
        const select = native.ownerSelect(handle);
        if (select === null) {
          return 0;
        }
        const options = native.options(select);
        for (let i = 0; i < options.length; i++) {
          if (options[i] === handle) {
            return i;
          }
        }
        return 0;
      },

      get text() {
        return native.optionText(self(this));
      },

      set text(value) {
        this.textContent = str(value);
      },

      get label() {
        const handle = self(this);
        const label = native.attribute(handle, "label");
        return label === null || label === "" ? native.optionText(handle) : label;
      },

      set label(value) {
        setAttribute(self(this), "label", str(value), "label");
      },

      get value() {
        return native.controlValue(self(this));
      },

      set value(value) {
        setAttribute(self(this), "value", str(value), "value");
      },

      get form() {
        const select = native.ownerSelect(self(this));
        return select === null ? null : wrap(native.formOwner(select));
      },
    },
    [madeInterfaces.HTMLOptionElement.prototype],
  );

  include(
    {
      get elements() {
        const element = this;
        const handle = self(this);
        return controlList(element, "elements", () => {
          const list = collection(HTMLFormControlsCollection, element,
            () => native.formControls(handle), true);
          stateOf(list).named = (state, name) => formNamedItem(state, name);
          return list;
        });
      },

      get length() {
        return native.formControls(self(this)).length;
      },

      submit() {
        drain(submitForm(self(this), null, true));
      },

      requestSubmit(submitter) {
        const handle = self(this);
        let by = null;
        if (submitter !== undefined && submitter !== null) {
          if (!isNode(submitter) || !isSubmitButton(handleOf(submitter))) {
            throw new $TypeError(
              prefix("requestSubmit", "HTMLFormElement") +
                "The specified element is not a submit button.",
            );
          }
          by = handleOf(submitter);
          if (native.formOwner(by) !== handle) {
            throw domError(
              "NotFoundError",
              "The specified element is not owned by this form element.",
              "requestSubmit",
              "HTMLFormElement",
            );
          }
        }
        drain(submitForm(handle, by, false));
      },

      reset() {
        drain(resetForm(self(this)));
      },
    },
    [madeInterfaces.HTMLFormElement.prototype],
  );

  include(
    {
      get control() {
        return wrap(native.labeledControl(self(this)));
      },

      get form() {
        const control = native.labeledControl(self(this));
        return control === null ? null : wrap(native.formOwner(control));
      },
    },
    [madeInterfaces.HTMLLabelElement.prototype],
  );

  include(
    {
      get type() {
        return buttonType(self(this));
      },

      set type(value) {
        setAttribute(self(this), "type", str(value), "type");
      },

      get form() {
        return wrap(native.formOwner(self(this)));
      },

      get labels() {
        const element = this;
        const handle = self(this);
        return controlList(element, "labels", () =>
          collection(NodeList, element, () => native.labels(handle), true));
      },
    },
    [madeInterfaces.HTMLButtonElement.prototype],
  );

  // ---- Constraint validation (HTML section 4.10.20, M26.9b) ---------------

  // A control's validity, live (HTML section 4.10.20.3): each state read
  // from the host when asked.
  const validityHandles = new $WeakMap();

  class ValidityState {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("ValidityState");
      }
    }
  }
  const VALIDITY_STATES = [
    "valueMissing", "typeMismatch", "patternMismatch", "tooLong", "tooShort",
    "rangeUnderflow", "rangeOverflow", "stepMismatch", "badInput", "customError",
  ];
  function validityBits(state) {
    const handle = WeakMapGet(validityHandles, state);
    if (handle === undefined) {
      throw illegalInvocation();
    }
    return native.validity(handle);
  }
  for (let i = 0; i < VALIDITY_STATES.length; i++) {
    const bit = 1 << i;
    defineProperty(ValidityState.prototype, VALIDITY_STATES[i], {
      get() {
        return (validityBits(this) & bit) !== 0;
      },
      enumerable: true,
      configurable: true,
    });
  }
  defineProperty(ValidityState.prototype, "valid", {
    get() {
      return validityBits(this) === 0;
    },
    enumerable: true,
    configurable: true,
  });
  expose(ValidityState, "ValidityState");

  // Fires a trusted, cancelable `invalid` at `handle`: whether it was
  // canceled.
  function* fireInvalid(handle) {
    const event = new Event("invalid", made(EVENT_INIT, ["cancelable", true]));
    eventState(event).trusted = true;
    return yield* dispatching(wrap(handle), event);
  }

  // HTML's interactive validation of `form` (section 4.10.20.2): an
  // `invalid` at each control that fails; the first not canceled focused
  // and its message given as the browser's notice. Whether all pass.
  function* validateInteractively(form) {
    const invalid = native.invalidControls(form);
    let first = null;
    for (let i = 0; i < invalid.length; i++) {
      const canceled = yield* fireInvalid(invalid[i]);
      if (!canceled && first === null && native.connected(invalid[i])) {
        first = invalid[i];
      }
    }
    if (first !== null) {
      focusElement(first, false);
      native.reportInvalid(first);
    }
    return invalid.length === 0;
  }

  include(
    {
      get validity() {
        const element = this;
        const handle = self(this);
        return controlList(element, "validity", () => {
          const state = new ValidityState(KEY);
          WeakMapSet(validityHandles, state, handle);
          return state;
        });
      },

      get willValidate() {
        return native.willValidate(self(this));
      },

      get validationMessage() {
        return native.validationMessage(self(this));
      },

      setCustomValidity(message) {
        const handle = self(this);
        need(arguments.length, 1, "setCustomValidity", "HTMLElement");
        native.setCustomValidity(handle, str(message));
      },

      checkValidity() {
        const handle = self(this);
        if (!native.willValidate(handle) || native.validity(handle) === 0) {
          return true;
        }
        drain(fireInvalid(handle));
        return false;
      },

      reportValidity() {
        const handle = self(this);
        if (!native.willValidate(handle) || native.validity(handle) === 0) {
          return true;
        }
        if (!drain(fireInvalid(handle)) && native.connected(handle)) {
          focusElement(handle, false);
          native.reportInvalid(handle);
        }
        return false;
      },
    },
    [
      madeInterfaces.HTMLInputElement.prototype,
      madeInterfaces.HTMLSelectElement.prototype,
      madeInterfaces.HTMLTextAreaElement.prototype,
      madeInterfaces.HTMLButtonElement.prototype,
    ],
  );

  include(
    {
      checkValidity() {
        const invalid = native.invalidControls(self(this));
        for (let i = 0; i < invalid.length; i++) {
          drain(fireInvalid(invalid[i]));
        }
        return invalid.length === 0;
      },

      reportValidity() {
        return drain(validateInteractively(self(this)));
      },
    },
    [madeInterfaces.HTMLFormElement.prototype],
  );

  // ---- Focus (HTML section 6.6, M26.6) ------------------------------------

  // Fires a focus event of `type` at `target`, `related` its related
  // target: `focus` and `blur` do not bubble, `focusin` and `focusout`
  // do; none can be canceled. Trusted: the focusing steps fire them.
  function fireFocus(type, target, related) {
    const bubbles = type === "focusin" || type === "focusout";
    const event = new FocusEvent(type, made(FOCUS_INIT, [
      "bubbles", bubbles,
      "composed", true,
      "view", global,
      "relatedTarget", related,
    ]));
    eventState(event).trusted = true;
    drain(dispatching(target, event));
  }

  // A text control's edit committed as it loses the focus (HTML section
  // 4.10.5.5, M26.8): `change` at it when the user changed its value.
  // Whether it still has the focus after, for the unfocusing to go on.
  function commitChange(handle) {
    if (native.commitEdit(handle)) {
      drain(fireTrusted(wrap(handle), "change", false));
    }
    return native.focused() === handle;
  }

  // HTML's focusing steps for the element `handle` names, as `focus()`
  // runs them: the element focused loses it first, with `blur` and
  // `focusout`, then this one gains it, with `focus` and `focusin`, each
  // naming the other. Nothing for an element that cannot take the
  // focus, or has it.
  function focusElement(handle, preventScroll) {
    if (!native.focusable(handle)) {
      return;
    }
    const old = native.focused();
    if (old === handle) {
      return;
    }
    const element = wrap(handle);
    const before = old === null ? null : wrap(old);
    if (before !== null) {
      if (!commitChange(old)) {
        return;
      }
      // The ancestors the two share keep `:focus-within` meanwhile, as
      // in the reference browser (M26.11).
      native.blurToward(handle);
      fireFocus("blur", before, element);
      fireFocus("focusout", before, element);
    }
    // A listener may have moved the focus, or the element; else what
    // `:focus-within` kept goes, unless the element takes it.
    if (native.focused() !== null) {
      return;
    }
    if (!native.focusable(handle)) {
      native.setFocus(null, true);
      return;
    }
    native.setFocus(handle, preventScroll);
    fireFocus("focus", element, before);
    fireFocus("focusin", element, before);
  }

  // HTML's unfocusing steps: the element loses the focus, to the
  // viewport.
  function blurElement(handle) {
    if (native.focused() !== handle || !commitChange(handle)) {
      return;
    }
    native.setFocus(null, true);
    const element = wrap(handle);
    fireFocus("blur", element, null);
    fireFocus("focusout", element, null);
  }

  include(
    {
      focus(options) {
        const handle = self(this);
        const prevent = typeof options === "object" && options !== null && !!options.preventScroll;
        focusElement(handle, prevent);
      },

      blur() {
        blurElement(self(this));
      },

      // `tabIndex` (HTML section 6.6.3): the attribute's integer, else 0
      // for an element that can take the focus as it is and -1 for one
      // that cannot.
      get tabIndex() {
        const handle = self(this);
        const value = native.attribute(handle, "tabindex");
        const n = value === null ? null : parseInteger(value, true);
        if (n !== null && n >= -2147483648 && n <= 2147483647) {
          return n;
        }
        return native.tabbable(handle) ? 0 : -1;
      },

      set tabIndex(value) {
        setAttribute(self(this), "tabindex", $String($Number(value) | 0), "tabIndex");
      },
    },
    [HTMLElement.prototype, SVGElement.prototype, MathMLElement.prototype],
  );

  // ---- Geometry (Geometry Interfaces Module Level 1, M30) ------------------

  const MathSin = Math.sin;
  const MathCos = Math.cos;
  const MathTan = Math.tan;
  const MathAtan2 = Math.atan2;
  const MathSqrt = Math.sqrt;
  const PI = Math.PI;
  const $Uint8ClampedArray = Uint8ClampedArray;
  const $Float32Array = Float32Array;
  const $Float64Array = Float64Array;
  // A typed array's kind, buffer, offset and length, by the getters no
  // page reaches: undefined for what is not a typed array.
  const TYPED_ARRAY = ReflectGetPrototypeOf(Uint8Array.prototype);
  const TypedArrayTag = uncurry(getOwnPropertyDescriptor(TYPED_ARRAY, toStringTag).get);
  const TypedArrayBuffer = uncurry(getOwnPropertyDescriptor(TYPED_ARRAY, "buffer").get);
  const TypedArrayOffset = uncurry(getOwnPropertyDescriptor(TYPED_ARRAY, "byteOffset").get);
  const TypedArrayLength = uncurry(getOwnPropertyDescriptor(TYPED_ARRAY, "byteLength").get);

  // A point's numbers, x, y, z and w; a matrix's sixteen, column by
  // column (m11, m12, m13, m14, m21, ...), and whether it is 2D; each
  // where no page reaches them.
  const points = new $WeakMap();
  const matrices = new $WeakMap();

  function pointOf(object) {
    const p = WeakMapGet(points, object);
    if (p === undefined) {
      throw illegalInvocation();
    }
    return p;
  }

  function matrixOf(object) {
    const m = WeakMapGet(matrices, object);
    if (m === undefined) {
      throw illegalInvocation();
    }
    return m;
  }

  class DOMPointReadOnly {
    constructor() {
      const a = arguments;
      WeakMapSet(points, this, [
        a[0] === undefined ? 0 : $Number(a[0]),
        a[1] === undefined ? 0 : $Number(a[1]),
        a[2] === undefined ? 0 : $Number(a[2]),
        a[3] === undefined ? 1 : $Number(a[3]),
      ]);
    }

    static fromPoint() {
      const p = pointInit(arguments[0], "fromPoint", "DOMPointReadOnly");
      return new DOMPointReadOnly(p[0], p[1], p[2], p[3]);
    }

    get x() {
      return pointOf(this)[0];
    }

    get y() {
      return pointOf(this)[1];
    }

    get z() {
      return pointOf(this)[2];
    }

    get w() {
      return pointOf(this)[3];
    }

    matrixTransform() {
      const p = pointOf(this);
      return transformed(matrixInit(arguments[0], "matrixTransform", "DOMPointReadOnly", true), p);
    }

    toJSON() {
      const p = pointOf(this);
      return { x: p[0], y: p[1], z: p[2], w: p[3] };
    }
  }

  class DOMPoint extends DOMPointReadOnly {
    static fromPoint() {
      const p = pointInit(arguments[0], "fromPoint", "DOMPoint");
      return new DOMPoint(p[0], p[1], p[2], p[3]);
    }
  }
  const POINT_FIELDS = ["x", "y", "z", "w"];
  for (let i = 0; i < 4; i++) {
    defineProperty(DOMPoint.prototype, POINT_FIELDS[i], {
      get() {
        return pointOf(this)[i];
      },
      set(value) {
        pointOf(this)[i] = $Number(value);
      },
      enumerable: true,
      configurable: true,
    });
  }

  // A DOMPointInit's numbers.
  function pointInit(value, method, iface) {
    if (value !== undefined && value !== null && typeof value !== "object" &&
      typeof value !== "function") {
      throw new $TypeError(prefix(method, iface) +
        "The provided value is not of type 'DOMPointInit'.");
    }
    const field = (name, fallback) => {
      const given = value === undefined || value === null ? undefined : value[name];
      return given === undefined ? fallback : $Number(given);
    };
    return [field("x", 0), field("y", 0), field("z", 0), field("w", 1)];
  }

  // The point `p` transformed by the matrix `m`, a new DOMPoint.
  function transformed(m, p) {
    const s = m.s;
    const x = p[0], y = p[1], z = p[2], w = p[3];
    return new DOMPoint(
      s[0] * x + s[4] * y + s[8] * z + s[12] * w,
      s[1] * x + s[5] * y + s[9] * z + s[13] * w,
      s[2] * x + s[6] * y + s[10] * z + s[14] * w,
      s[3] * x + s[7] * y + s[11] * z + s[15] * w,
    );
  }

  const IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];

  // The identity, 2D or not.
  function identity(is2D) {
    return { s: ArraySlice(IDENTITY, 0), is2D };
  }

  // A matrix's numbers from six (2D) or sixteen.
  function fromList(list, is2D) {
    const s = ArraySlice(IDENTITY, 0);
    if (is2D) {
      s[0] = list[0];
      s[1] = list[1];
      s[4] = list[2];
      s[5] = list[3];
      s[12] = list[4];
      s[13] = list[5];
    } else {
      for (let i = 0; i < 16; i++) {
        s[i] = list[i];
      }
    }
    return { s, is2D };
  }

  // The product a x b, b applied to a point first.
  function multiplied(a, b) {
    const s = [];
    for (let col = 0; col < 4; col++) {
      for (let row = 0; row < 4; row++) {
        let sum = 0;
        for (let k = 0; k < 4; k++) {
          sum += a.s[k * 4 + row] * b.s[col * 4 + k];
        }
        s[col * 4 + row] = sum;
      }
    }
    return { s, is2D: a.is2D && b.is2D };
  }

  function sameValueZero(a, b) {
    return a === b || (a !== a && b !== b);
  }

  // Geometry's "validate and fixup" of a DOMMatrix2DInit, or of a
  // DOMMatrixInit when `full`: the numbers, `a` and `m11` and the rest
  // agreeing, and whether it is 2D.
  function matrixInit(value, method, iface, full) {
    if (value !== undefined && value !== null && typeof value !== "object" &&
      typeof value !== "function") {
      throw new $TypeError(prefix(method, iface) + "The provided value is not of type '" +
        (full ? "DOMMatrixInit" : "DOMMatrix2DInit") + "'.");
    }
    const get = (name) => {
      const given = value === undefined || value === null ? undefined : value[name];
      return given === undefined ? undefined : $Number(given);
    };
    const pairs = [["a", "m11", 1], ["b", "m12", 0], ["c", "m21", 0], ["d", "m22", 1],
      ["e", "m41", 0], ["f", "m42", 0]];
    const two = [];
    for (let i = 0; i < 6; i++) {
      const short = get(pairs[i][0]);
      const long = get(pairs[i][1]);
      if (short !== undefined && long !== undefined && !sameValueZero(short, long)) {
        throw new $TypeError(prefix(method, iface) + "Property mismatch on matrix initialization.");
      }
      two[i] = long !== undefined ? long : short !== undefined ? short : pairs[i][2];
    }
    const m = fromList(two, true);
    if (!full) {
      return m;
    }
    const rest = [["m13", 2, 0], ["m14", 3, 0], ["m23", 6, 0], ["m24", 7, 0], ["m31", 8, 0],
      ["m32", 9, 0], ["m33", 10, 1], ["m34", 11, 0], ["m43", 14, 0], ["m44", 15, 1]];
    let flat = true;
    for (let i = 0; i < rest.length; i++) {
      const given = get(rest[i][0]);
      const v = given === undefined ? rest[i][2] : given;
      m.s[rest[i][1]] = v;
      if (v !== rest[i][2]) {
        flat = false;
      }
    }
    const is2D = value === undefined || value === null ? undefined : value.is2D;
    if (is2D === undefined) {
      m.is2D = flat;
    } else if (!!is2D && !flat) {
      throw new $TypeError(prefix(method, iface) +
        "The is2D member is set to true but the input matrix is a 3d matrix.");
    } else {
      m.is2D = !!is2D;
    }
    return m;
  }

  // A matrix of the class `cls` holding `m`.
  function makeMatrix(cls, m) {
    const object = new cls(KEY);
    WeakMapSet(matrices, object, m);
    return object;
  }

  function translation(tx, ty, tz) {
    const m = identity(tz === 0);
    m.s[12] = tx;
    m.s[13] = ty;
    m.s[14] = tz;
    return m;
  }

  function scaling(sx, sy, sz) {
    const m = identity(sz === 1);
    m.s[0] = sx;
    m.s[5] = sy;
    m.s[10] = sz;
    return m;
  }

  // The sine and cosine of `degrees`, exact at quarter turns.
  function sinCos(degrees) {
    const turns = degrees / 90;
    if (turns === MathFloor(turns) && turns === turns) {
      const q = ((turns % 4) + 4) % 4;
      return [[0, 1, 0, -1][q], [1, 0, -1, 0][q]];
    }
    const radians = (degrees * PI) / 180;
    return [MathSin(radians), MathCos(radians)];
  }

  // A rotation of `degrees` about the axis (x, y, z): about one of the
  // three axes by its sine and cosine, else as CSS Transforms 2 writes it.
  function rotation(x, y, z, degrees) {
    const m = identity(false);
    const s = m.s;
    const length = MathSqrt(x * x + y * y + z * z);
    if (length === 0 || length !== length) {
      m.is2D = x === 0 && y === 0;
      return m;
    }
    x /= length;
    y /= length;
    z /= length;
    const sc = sinCos(degrees);
    const sin = sc[0], cos = sc[1];
    if (x === 0 && y === 0) {
      s[0] = cos;
      s[1] = z * sin;
      s[4] = -z * sin;
      s[5] = cos;
      m.is2D = true;
      return m;
    }
    if (y === 0 && z === 0) {
      s[5] = cos;
      s[6] = x * sin;
      s[9] = -x * sin;
      s[10] = cos;
      return m;
    }
    if (x === 0 && z === 0) {
      s[0] = cos;
      s[2] = -y * sin;
      s[8] = y * sin;
      s[10] = cos;
      return m;
    }
    const half = sinCos(degrees / 2);
    const scHalf = half[0] * half[1];
    const sq = half[0] * half[0];
    s[0] = 1 - 2 * (y * y + z * z) * sq;
    s[1] = 2 * (x * y * sq + z * scHalf);
    s[2] = 2 * (x * z * sq - y * scHalf);
    s[4] = 2 * (x * y * sq - z * scHalf);
    s[5] = 1 - 2 * (x * x + z * z) * sq;
    s[6] = 2 * (y * z * sq + x * scHalf);
    s[8] = 2 * (x * z * sq + y * scHalf);
    s[9] = 2 * (y * z * sq - x * scHalf);
    s[10] = 1 - 2 * (x * x + y * y) * sq;
    return m;
  }

  function skewing(xDegrees, yDegrees) {
    const m = identity(true);
    m.s[4] = MathTan((xDegrees * PI) / 180);
    m.s[1] = MathTan((yDegrees * PI) / 180);
    return m;
  }

  function nans() {
    const s = [];
    for (let i = 0; i < 16; i++) {
      s[i] = NaN;
    }
    return s;
  }

  // The inverse of `m`, or NaN everywhere when it has none.
  function inverted(m) {
    const s = m.s;
    if (m.is2D) {
      const det = s[0] * s[5] - s[1] * s[4];
      if (det === 0 || det !== det) {
        return { s: nans(), is2D: false };
      }
      return fromList([
        s[5] / det, -s[1] / det, -s[4] / det, s[0] / det,
        (s[4] * s[13] - s[5] * s[12]) / det, (s[1] * s[12] - s[0] * s[13]) / det,
      ], true);
    }
    const v = [];
    v[0] = s[5] * s[10] * s[15] - s[5] * s[11] * s[14] - s[9] * s[6] * s[15] +
      s[9] * s[7] * s[14] + s[13] * s[6] * s[11] - s[13] * s[7] * s[10];
    v[4] = -s[4] * s[10] * s[15] + s[4] * s[11] * s[14] + s[8] * s[6] * s[15] -
      s[8] * s[7] * s[14] - s[12] * s[6] * s[11] + s[12] * s[7] * s[10];
    v[8] = s[4] * s[9] * s[15] - s[4] * s[11] * s[13] - s[8] * s[5] * s[15] +
      s[8] * s[7] * s[13] + s[12] * s[5] * s[11] - s[12] * s[7] * s[9];
    v[12] = -s[4] * s[9] * s[14] + s[4] * s[10] * s[13] + s[8] * s[5] * s[14] -
      s[8] * s[6] * s[13] - s[12] * s[5] * s[10] + s[12] * s[6] * s[9];
    v[1] = -s[1] * s[10] * s[15] + s[1] * s[11] * s[14] + s[9] * s[2] * s[15] -
      s[9] * s[3] * s[14] - s[13] * s[2] * s[11] + s[13] * s[3] * s[10];
    v[5] = s[0] * s[10] * s[15] - s[0] * s[11] * s[14] - s[8] * s[2] * s[15] +
      s[8] * s[3] * s[14] + s[12] * s[2] * s[11] - s[12] * s[3] * s[10];
    v[9] = -s[0] * s[9] * s[15] + s[0] * s[11] * s[13] + s[8] * s[1] * s[15] -
      s[8] * s[3] * s[13] - s[12] * s[1] * s[11] + s[12] * s[3] * s[9];
    v[13] = s[0] * s[9] * s[14] - s[0] * s[10] * s[13] - s[8] * s[1] * s[14] +
      s[8] * s[2] * s[13] + s[12] * s[1] * s[10] - s[12] * s[2] * s[9];
    v[2] = s[1] * s[6] * s[15] - s[1] * s[7] * s[14] - s[5] * s[2] * s[15] +
      s[5] * s[3] * s[14] + s[13] * s[2] * s[7] - s[13] * s[3] * s[6];
    v[6] = -s[0] * s[6] * s[15] + s[0] * s[7] * s[14] + s[4] * s[2] * s[15] -
      s[4] * s[3] * s[14] - s[12] * s[2] * s[7] + s[12] * s[3] * s[6];
    v[10] = s[0] * s[5] * s[15] - s[0] * s[7] * s[13] - s[4] * s[1] * s[15] +
      s[4] * s[3] * s[13] + s[12] * s[1] * s[7] - s[12] * s[3] * s[5];
    v[14] = -s[0] * s[5] * s[14] + s[0] * s[6] * s[13] + s[4] * s[1] * s[14] -
      s[4] * s[2] * s[13] - s[12] * s[1] * s[6] + s[12] * s[2] * s[5];
    v[3] = -s[1] * s[6] * s[11] + s[1] * s[7] * s[10] + s[5] * s[2] * s[11] -
      s[5] * s[3] * s[10] - s[9] * s[2] * s[7] + s[9] * s[3] * s[6];
    v[7] = s[0] * s[6] * s[11] - s[0] * s[7] * s[10] - s[4] * s[2] * s[11] +
      s[4] * s[3] * s[10] + s[8] * s[2] * s[7] - s[8] * s[3] * s[6];
    v[11] = -s[0] * s[5] * s[11] + s[0] * s[7] * s[9] + s[4] * s[1] * s[11] -
      s[4] * s[3] * s[9] - s[8] * s[1] * s[7] + s[8] * s[3] * s[5];
    v[15] = s[0] * s[5] * s[10] - s[0] * s[6] * s[9] - s[4] * s[1] * s[10] +
      s[4] * s[2] * s[9] + s[8] * s[1] * s[6] - s[8] * s[2] * s[5];
    const det = s[0] * v[0] + s[1] * v[4] + s[2] * v[8] + s[3] * v[12];
    if (det === 0 || det !== det) {
      return { s: nans(), is2D: false };
    }
    for (let i = 0; i < 16; i++) {
      v[i] /= det;
    }
    return { s: v, is2D: false };
  }

  // The absolute lengths and the angles a transform list may hold, in
  // pixels and degrees; the lengths it may not, relative to a font, a
  // viewport or a box.
  const LENGTHS = ["px", 1, "cm", 96 / 2.54, "mm", 96 / 25.4, "q", 96 / 101.6, "in", 96,
    "pt", 4 / 3, "pc", 16];
  const ANGLES = ["deg", 1, "grad", 0.9, "rad", 180 / PI, "turn", 360];
  const RELATIVE = ["%", "em", "ex", "ch", "rem", "vw", "vh", "vmin", "vmax", "lh", "rlh",
    "cap", "ic", "vi", "vb", "svw", "svh", "lvw", "lvh", "dvw", "dvh", "cqw", "cqh"];

  function unitScale(list, unit) {
    for (let i = 0; i < list.length; i += 2) {
      if (list[i] === unit) {
        return list[i + 1];
      }
    }
    return undefined;
  }

  // A transform list (CSS Transforms 1 and 2) as a matrix, as DOMMatrix's
  // string reads it: its functions multiplied in order, `none` or
  // nothing the identity. One that does not parse is a SyntaxError, as is
  // one with a length that is not absolute.
  function fromTransformList(text, iface, method) {
    const fail = (message) => new $DOMException(
      "Failed to " + method + " '" + iface + "': " + message, "SyntaxError");
    const parse = () => fail("Failed to parse '" + text + "'.");
    const n = text.length;
    let i = 0;
    const space = () => {
      while (i < n && (text[i] === " " || text[i] === "\t" || text[i] === "\n" ||
        text[i] === "\r" || text[i] === "\f")) {
        i++;
      }
    };
    const letter = (c) => (c >= "a" && c <= "z") || (c >= "A" && c <= "Z") || c === "-";
    const digit = (c) => c >= "0" && c <= "9";
    space();
    let result = identity(true);
    const start = i;
    while (i < n && letter(text[i])) {
      i++;
    }
    if (ToLowerCase(StringSlice(text, start, i)) === "none") {
      space();
      if (i === n) {
        return result;
      }
      throw parse();
    }
    i = start;
    while (i < n) {
      const nameStart = i;
      while (i < n && (letter(text[i]) || digit(text[i]))) {
        i++;
      }
      const name = ToLowerCase(StringSlice(text, nameStart, i));
      if (name === "" || text[i] !== "(") {
        throw parse();
      }
      i++;
      const args = [];
      space();
      while (i < n && text[i] !== ")") {
        const numberStart = i;
        if (text[i] === "+" || text[i] === "-") {
          i++;
        }
        let digits = 0;
        while (i < n && digit(text[i])) {
          i++;
          digits++;
        }
        if (text[i] === ".") {
          i++;
          while (i < n && digit(text[i])) {
            i++;
            digits++;
          }
        }
        if (digits === 0) {
          throw parse();
        }
        if ((text[i] === "e" || text[i] === "E") &&
          (digit(text[i + 1]) || ((text[i + 1] === "+" || text[i + 1] === "-") && digit(text[i + 2])))) {
          i += 2;
          while (i < n && digit(text[i])) {
            i++;
          }
        }
        const value = $Number(StringSlice(text, numberStart, i));
        const unitStart = i;
        while (i < n && (letter(text[i]) || text[i] === "%")) {
          i++;
        }
        ArrayPush(args, [value, ToLowerCase(StringSlice(text, unitStart, i))]);
        space();
        if (text[i] === ",") {
          i++;
          space();
          if (text[i] === ")") {
            throw parse();
          }
        }
      }
      if (i >= n) {
        throw parse();
      }
      i++;
      space();
      result = multiplied(result, transformFunction(name, args, parse, fail));
    }
    return result;
  }

  // One function of a transform list as a matrix.
  function transformFunction(name, args, parse, fail) {
    const count = args.length;
    const numberAt = (k) => {
      if (args[k][1] !== "") {
        throw parse();
      }
      return args[k][0];
    };
    const lengthAt = (k) => {
      const v = args[k][0], unit = args[k][1];
      if (unit === "" && v === 0) {
        return 0;
      }
      const scale = unitScale(LENGTHS, unit);
      if (scale !== undefined) {
        return v * scale;
      }
      for (let r = 0; r < RELATIVE.length; r++) {
        if (RELATIVE[r] === unit) {
          throw fail("Values must be resolvable at parse time");
        }
      }
      throw parse();
    };
    const angleAt = (k) => {
      const v = args[k][0], unit = args[k][1];
      if (unit === "" && v === 0) {
        return 0;
      }
      const scale = unitScale(ANGLES, unit);
      if (scale === undefined) {
        throw parse();
      }
      return v * scale;
    };
    const want = (lo, hi) => {
      if (count < lo || count > hi) {
        throw parse();
      }
    };
    switch (name) {
      case "matrix": {
        want(6, 6);
        const v = [];
        for (let k = 0; k < 6; k++) {
          v[k] = numberAt(k);
        }
        return fromList(v, true);
      }
      case "matrix3d": {
        want(16, 16);
        const v = [];
        for (let k = 0; k < 16; k++) {
          v[k] = numberAt(k);
        }
        return fromList(v, false);
      }
      case "translate":
        want(1, 2);
        return translation(lengthAt(0), count > 1 ? lengthAt(1) : 0, 0);
      case "translatex":
        want(1, 1);
        return translation(lengthAt(0), 0, 0);
      case "translatey":
        want(1, 1);
        return translation(0, lengthAt(0), 0);
      case "translatez": {
        want(1, 1);
        const m = translation(0, 0, lengthAt(0));
        m.is2D = false;
        return m;
      }
      case "translate3d": {
        want(3, 3);
        const m = translation(lengthAt(0), lengthAt(1), lengthAt(2));
        m.is2D = false;
        return m;
      }
      case "scale":
        want(1, 2);
        return scaling(numberAt(0), count > 1 ? numberAt(1) : numberAt(0), 1);
      case "scalex":
        want(1, 1);
        return scaling(numberAt(0), 1, 1);
      case "scaley":
        want(1, 1);
        return scaling(1, numberAt(0), 1);
      case "scalez": {
        want(1, 1);
        const m = scaling(1, 1, numberAt(0));
        m.is2D = false;
        return m;
      }
      case "scale3d": {
        want(3, 3);
        const m = scaling(numberAt(0), numberAt(1), numberAt(2));
        m.is2D = false;
        return m;
      }
      case "rotate":
      case "rotatez": {
        want(1, 1);
        const m = rotation(0, 0, 1, angleAt(0));
        m.is2D = name === "rotate";
        return m;
      }
      case "rotatex":
        want(1, 1);
        return rotation(1, 0, 0, angleAt(0));
      case "rotatey":
        want(1, 1);
        return rotation(0, 1, 0, angleAt(0));
      case "rotate3d": {
        want(4, 4);
        const m = rotation(numberAt(0), numberAt(1), numberAt(2), angleAt(3));
        m.is2D = false;
        return m;
      }
      case "skew":
        want(1, 2);
        return skewing(angleAt(0), count > 1 ? angleAt(1) : 0);
      case "skewx":
        want(1, 1);
        return skewing(angleAt(0), 0);
      case "skewy":
        want(1, 1);
        return skewing(0, angleAt(0));
      case "perspective": {
        want(1, 1);
        const m = identity(false);
        const d = lengthAt(0);
        if (d !== 0) {
          m.s[11] = -1 / d;
        }
        return m;
      }
      default:
        throw parse();
    }
  }

  // A matrix's numbers from a list given to the constructor: six for a
  // 2D one, sixteen for one in 3D.
  function fromSequence(list, iface) {
    if (list.length === 6) {
      return fromList(list, true);
    }
    if (list.length === 16) {
      return fromList(list, false);
    }
    throw new $TypeError("Failed to construct '" + iface +
      "': The sequence must contain 6 elements for a 2D matrix or 16 elements for a 3D matrix.");
  }

  // The constructor's argument as a matrix: none the identity, a string
  // a transform list, anything iterable its numbers.
  function constructed(init, iface) {
    if (init === undefined) {
      return identity(true);
    }
    if (typeof init === "string") {
      return fromTransformList(init, iface, "construct");
    }
    if (init === null || (typeof init !== "object" && typeof init !== "function") ||
      typeof init[iteratorSymbol] !== "function") {
      throw new $TypeError("Failed to construct '" + iface +
        "': The provided value cannot be converted to a sequence.");
    }
    const list = [];
    const iterator = init[iteratorSymbol]();
    for (;;) {
      const step = iterator.next();
      if (step.done) {
        break;
      }
      ArrayPush(list, $Number(step.value));
    }
    return fromSequence(list, iface);
  }

  // A matrix from a typed array's numbers.
  function typedMatrix(array, method, iface, type) {
    if (array === null || typeof array !== "object" || TypedArrayTag(array) !== type) {
      throw new $TypeError(prefix(method, iface) + "parameter 1 is not of type '" + type + "'.");
    }
    const list = [];
    for (let i = 0; i < array.length; i++) {
      list[i] = array[i];
    }
    if (list.length !== 6 && list.length !== 16) {
      throw new $TypeError(prefix(method, iface) +
        "The sequence must contain 6 elements for a 2D matrix or 16 elements for a 3D matrix.");
    }
    return fromList(list, list.length === 6);
  }

  // A number argument, its default when undefined.
  function or(value, fallback) {
    return value === undefined ? fallback : $Number(value);
  }

  function translated(m, tx, ty, tz) {
    return multiplied(m, translation(or(tx, 0), or(ty, 0), or(tz, 0)));
  }

  function scaled(m, sx, sy, sz, ox, oy, oz) {
    sx = or(sx, 1);
    sy = or(sy, sx);
    sz = or(sz, 1);
    ox = or(ox, 0);
    oy = or(oy, 0);
    oz = or(oz, 0);
    let out = multiplied(m, translation(ox, oy, oz));
    out = multiplied(out, scaling(sx, sy, sz));
    out = multiplied(out, translation(-ox, -oy, -oz));
    out.is2D = m.is2D && sz === 1 && oz === 0;
    return out;
  }

  function rotated(m, rx, ry, rz) {
    rx = or(rx, 0);
    if (ry === undefined && rz === undefined) {
      rz = rx;
      rx = 0;
      ry = 0;
    } else {
      ry = or(ry, 0);
      rz = or(rz, 0);
    }
    let out = m;
    if (rz !== 0 || (rx === 0 && ry === 0)) {
      out = multiplied(out, rotation(0, 0, 1, rz));
    }
    if (ry !== 0) {
      out = multiplied(out, rotation(0, 1, 0, ry));
    }
    if (rx !== 0) {
      out = multiplied(out, rotation(1, 0, 0, rx));
    }
    return out;
  }

  function rotatedFromVector(m, x, y) {
    x = or(x, 0);
    y = or(y, 0);
    const degrees = x === 0 && y === 0 ? 0 : (MathAtan2(y, x) * 180) / PI;
    return multiplied(m, rotation(0, 0, 1, degrees));
  }

  function axisAngle(m, x, y, z, angle) {
    return multiplied(m, rotation(or(x, 0), or(y, 0), or(z, 0), or(angle, 0)));
  }

  class DOMMatrixReadOnly {
    constructor() {
      if (arguments[0] === KEY) {
        return;
      }
      const iface = new.target === DOMMatrix ? "DOMMatrix" : "DOMMatrixReadOnly";
      WeakMapSet(matrices, this, constructed(arguments[0], iface));
    }

    static fromMatrix() {
      return makeMatrix(DOMMatrixReadOnly,
        matrixInit(arguments[0], "fromMatrix", "DOMMatrixReadOnly", true));
    }

    static fromFloat32Array(array) {
      return makeMatrix(DOMMatrixReadOnly,
        typedMatrix(array, "fromFloat32Array", "DOMMatrixReadOnly", "Float32Array"));
    }

    static fromFloat64Array(array) {
      return makeMatrix(DOMMatrixReadOnly,
        typedMatrix(array, "fromFloat64Array", "DOMMatrixReadOnly", "Float64Array"));
    }

    get is2D() {
      return matrixOf(this).is2D;
    }

    get isIdentity() {
      const s = matrixOf(this).s;
      for (let i = 0; i < 16; i++) {
        if (s[i] !== IDENTITY[i]) {
          return false;
        }
      }
      return true;
    }

    translate() {
      const a = arguments;
      return makeMatrix(DOMMatrix, translated(matrixOf(this), a[0], a[1], a[2]));
    }

    scale() {
      const a = arguments;
      return makeMatrix(DOMMatrix, scaled(matrixOf(this), a[0], a[1], a[2], a[3], a[4], a[5]));
    }

    scaleNonUniform() {
      const a = arguments;
      return makeMatrix(DOMMatrix, scaled(matrixOf(this), or(a[0], 1), or(a[1], 1), 1, 0, 0, 0));
    }

    scale3d() {
      const a = arguments;
      const s = or(a[0], 1);
      return makeMatrix(DOMMatrix, scaled(matrixOf(this), s, s, s, a[1], a[2], a[3]));
    }

    rotate() {
      const a = arguments;
      return makeMatrix(DOMMatrix, rotated(matrixOf(this), a[0], a[1], a[2]));
    }

    rotateFromVector() {
      const a = arguments;
      return makeMatrix(DOMMatrix, rotatedFromVector(matrixOf(this), a[0], a[1]));
    }

    rotateAxisAngle() {
      const a = arguments;
      return makeMatrix(DOMMatrix, axisAngle(matrixOf(this), a[0], a[1], a[2], a[3]));
    }

    skewX() {
      return makeMatrix(DOMMatrix, multiplied(matrixOf(this), skewing(or(arguments[0], 0), 0)));
    }

    skewY() {
      return makeMatrix(DOMMatrix, multiplied(matrixOf(this), skewing(0, or(arguments[0], 0))));
    }

    multiply() {
      const m = matrixOf(this);
      return makeMatrix(DOMMatrix,
        multiplied(m, matrixInit(arguments[0], "multiply", "DOMMatrixReadOnly", true)));
    }

    flipX() {
      return makeMatrix(DOMMatrix, multiplied(matrixOf(this), fromList([-1, 0, 0, 1, 0, 0], true)));
    }

    flipY() {
      return makeMatrix(DOMMatrix, multiplied(matrixOf(this), fromList([1, 0, 0, -1, 0, 0], true)));
    }

    inverse() {
      return makeMatrix(DOMMatrix, inverted(matrixOf(this)));
    }

    transformPoint() {
      const m = matrixOf(this);
      return transformed(m, pointInit(arguments[0], "transformPoint", "DOMMatrixReadOnly"));
    }

    toFloat32Array() {
      return new $Float32Array(matrixOf(this).s);
    }

    toFloat64Array() {
      return new $Float64Array(matrixOf(this).s);
    }

    toJSON() {
      const m = matrixOf(this);
      const out = {};
      for (let i = 0; i < MATRIX_FIELDS.length; i++) {
        out[MATRIX_FIELDS[i][0]] = m.s[MATRIX_FIELDS[i][1]];
      }
      out.is2D = m.is2D;
      out.isIdentity = this.isIdentity;
      return out;
    }

    toString() {
      const m = matrixOf(this);
      const s = m.s;
      for (let i = 0; i < 16; i++) {
        const n = s[i];
        if (n !== n || n === Infinity || n === -Infinity) {
          throw new $DOMException(
            "Failed to execute 'toString' on 'DOMMatrixReadOnly': " +
              "DOMMatrix cannot be serialized with NaN or Infinity values.",
            "InvalidStateError",
          );
        }
      }
      if (m.is2D) {
        return "matrix(" + $String(s[0]) + ", " + $String(s[1]) + ", " +
          $String(s[4]) + ", " + $String(s[5]) + ", " + $String(s[12]) + ", " +
          $String(s[13]) + ")";
      }
      let text = "matrix3d(";
      for (let i = 0; i < 16; i++) {
        text += (i === 0 ? "" : ", ") + $String(s[i]);
      }
      return text + ")";
    }
  }

  // a..f and m11..m44: getters on the read-only class, with setters on
  // DOMMatrix, where a value set off the 2D plane makes it 3D.
  const MATRIX_FIELDS = [["a", 0], ["b", 1], ["c", 4], ["d", 5], ["e", 12], ["f", 13]];
  for (let i = 0; i < 16; i++) {
    ArrayPush(MATRIX_FIELDS, ["m" + ((i >> 2) + 1) + ((i & 3) + 1), i]);
  }
  for (let i = 0; i < MATRIX_FIELDS.length; i++) {
    const at = MATRIX_FIELDS[i][1];
    defineProperty(DOMMatrixReadOnly.prototype, MATRIX_FIELDS[i][0], {
      get() {
        return matrixOf(this).s[at];
      },
      enumerable: true,
      configurable: true,
    });
  }

  class DOMMatrix extends DOMMatrixReadOnly {
    static fromMatrix() {
      return makeMatrix(DOMMatrix, matrixInit(arguments[0], "fromMatrix", "DOMMatrix", true));
    }

    static fromFloat32Array(array) {
      return makeMatrix(DOMMatrix,
        typedMatrix(array, "fromFloat32Array", "DOMMatrix", "Float32Array"));
    }

    static fromFloat64Array(array) {
      return makeMatrix(DOMMatrix,
        typedMatrix(array, "fromFloat64Array", "DOMMatrix", "Float64Array"));
    }

    multiplySelf() {
      const other = arguments[0];
      return becomes(this, (m) => multiplied(m, matrixInit(other, "multiplySelf", "DOMMatrix", true)));
    }

    preMultiplySelf() {
      const other = arguments[0];
      return becomes(this,
        (m) => multiplied(matrixInit(other, "preMultiplySelf", "DOMMatrix", true), m));
    }

    translateSelf() {
      const a = arguments;
      return becomes(this, (m) => translated(m, a[0], a[1], a[2]));
    }

    scaleSelf() {
      const a = arguments;
      return becomes(this, (m) => scaled(m, a[0], a[1], a[2], a[3], a[4], a[5]));
    }

    scale3dSelf() {
      const a = arguments;
      const s = or(a[0], 1);
      return becomes(this, (m) => scaled(m, s, s, s, a[1], a[2], a[3]));
    }

    rotateSelf() {
      const a = arguments;
      return becomes(this, (m) => rotated(m, a[0], a[1], a[2]));
    }

    rotateFromVectorSelf() {
      const a = arguments;
      return becomes(this, (m) => rotatedFromVector(m, a[0], a[1]));
    }

    rotateAxisAngleSelf() {
      const a = arguments;
      return becomes(this, (m) => axisAngle(m, a[0], a[1], a[2], a[3]));
    }

    skewXSelf() {
      const sx = or(arguments[0], 0);
      return becomes(this, (m) => multiplied(m, skewing(sx, 0)));
    }

    skewYSelf() {
      const sy = or(arguments[0], 0);
      return becomes(this, (m) => multiplied(m, skewing(0, sy)));
    }

    invertSelf() {
      return becomes(this, inverted);
    }

    setMatrixValue(transformList) {
      const text = str(transformList);
      return becomes(this,
        () => fromTransformList(text, "DOMMatrix", "execute 'setMatrixValue' on"));
    }
  }

  // The matrix `object` holds replaced by what `change` makes of it.
  function becomes(object, change) {
    const m = matrixOf(object);
    const next = change(m);
    m.s = next.s;
    m.is2D = next.is2D;
    return object;
  }

  for (let i = 0; i < MATRIX_FIELDS.length; i++) {
    const at = MATRIX_FIELDS[i][1];
    const flat = at === 10 || at === 15 ? 1 : 0;
    const plane = at === 0 || at === 1 || at === 4 || at === 5 || at === 12 || at === 13;
    defineProperty(DOMMatrix.prototype, MATRIX_FIELDS[i][0], {
      get() {
        return matrixOf(this).s[at];
      },
      set(value) {
        const m = matrixOf(this);
        const v = $Number(value);
        m.s[at] = v;
        if (!plane && v !== flat) {
          m.is2D = false;
        }
      },
      enumerable: true,
      configurable: true,
    });
  }

  // A rectangle (Geometry Interfaces 1 section 4, M32: what `getBBox()`
  // gives): its x, y, width and height, the rest made from them.
  const rects = new $WeakMap();
  function rectOf(object) {
    const r = WeakMapGet(rects, object);
    if (r === undefined) {
      throw illegalInvocation();
    }
    return r;
  }

  class DOMRectReadOnly {
    constructor() {
      const a = arguments;
      WeakMapSet(rects, this, [
        a[0] === undefined ? 0 : $Number(a[0]),
        a[1] === undefined ? 0 : $Number(a[1]),
        a[2] === undefined ? 0 : $Number(a[2]),
        a[3] === undefined ? 0 : $Number(a[3]),
      ]);
    }

    static fromRect(other) {
      const r = rectInit(other);
      return new DOMRectReadOnly(r[0], r[1], r[2], r[3]);
    }

    get x() {
      return rectOf(this)[0];
    }

    get y() {
      return rectOf(this)[1];
    }

    get width() {
      return rectOf(this)[2];
    }

    get height() {
      return rectOf(this)[3];
    }

    get top() {
      const r = rectOf(this);
      return MathMin(r[1], r[1] + r[3]);
    }

    get right() {
      const r = rectOf(this);
      return MathMax(r[0], r[0] + r[2]);
    }

    get bottom() {
      const r = rectOf(this);
      return MathMax(r[1], r[1] + r[3]);
    }

    get left() {
      const r = rectOf(this);
      return MathMin(r[0], r[0] + r[2]);
    }

    toJSON() {
      return {
        x: this.x,
        y: this.y,
        width: this.width,
        height: this.height,
        top: this.top,
        right: this.right,
        bottom: this.bottom,
        left: this.left,
      };
    }
  }

  class DOMRect extends DOMRectReadOnly {
    static fromRect(other) {
      const r = rectInit(other);
      return new DOMRect(r[0], r[1], r[2], r[3]);
    }
  }
  const RECT_FIELDS = ["x", "y", "width", "height"];
  for (let i = 0; i < 4; i++) {
    defineProperty(DOMRect.prototype, RECT_FIELDS[i], {
      get() {
        return rectOf(this)[i];
      },
      set(value) {
        rectOf(this)[i] = $Number(value);
      },
      enumerable: true,
      configurable: true,
    });
  }

  function rectInit(value) {
    const field = (name) => {
      const given = value === undefined || value === null ? undefined : value[name];
      return given === undefined ? 0 : $Number(given);
    };
    return [field("x"), field("y"), field("width"), field("height")];
  }

  expose(DOMPointReadOnly, "DOMPointReadOnly");
  expose(DOMPoint, "DOMPoint");
  expose(DOMRectReadOnly, "DOMRectReadOnly");
  expose(DOMRect, "DOMRect");
  expose(DOMMatrixReadOnly, "DOMMatrixReadOnly");
  expose(DOMMatrix, "DOMMatrix");

  // ---- Images for a canvas (HTML section 4.8.4, M30) ----------------------

  // An `img`'s image as a canvas would draw it: [0 coming or none, 1
  // there, 2 broken; its natural width; its height].
  function imageState(element) {
    return native.canvas("imageState", 1, self(element));
  }

  include(
    {
      get complete() {
        const handle = self(this);
        const src = native.attribute(handle, "src");
        return src === null || src === "" || imageState(this)[0] !== 0;
      },

      get naturalWidth() {
        return imageState(this)[1];
      },

      get naturalHeight() {
        return imageState(this)[2];
      },
    },
    [MapGet(interfaces, "img").prototype],
  );

  // The legacy factory function `Image(width, height)` (HTML 4.8.4.1):
  // a new `img`, its size attributes set when given.
  const ImageFactory = function Image(width, height) {
    if (new.target === undefined) {
      throw new $TypeError("Failed to construct 'Image': Please use the 'new' operator, " +
        "this DOM object constructor cannot be called as a function.");
    }
    const element = wrap(native.createElement("img"));
    if (width !== undefined) {
      element.width = width;
    }
    if (height !== undefined) {
      element.height = height;
    }
    return element;
  };
  defineProperty(ImageFactory, "prototype", { value: MapGet(interfaces, "img").prototype });
  windowOnly("Image", { value: ImageFactory, writable: true, configurable: true });

  // ---- Canvas: the 2D context (HTML section 4.12.5, M30) ------------------

  // What a context, a Path2D, a gradient or a pattern is to the host:
  // the kind it numbers it by (0 an element's context, by the element's
  // handle; 2 a Path2D, 3 a CanvasGradient, 4 a CanvasPattern, by their
  // numbers), which, and the interface its errors name.
  const canvasTargets = new $WeakMap();
  const CTX = "CanvasRenderingContext2D";

  function targetOf(object) {
    const t = WeakMapGet(canvasTargets, object);
    if (t === undefined) {
      throw illegalInvocation();
    }
    return t;
  }

  // The objects the host numbers, by their numbers, while they live; the
  // registry tells the host when one is gone and no newer object holds
  // its number.
  const canvasObjects = new $Map();
  const canvasRegistry = new $FinalizationRegistry((held) => {
    const ref = MapGet(canvasObjects, held[1]);
    if (ref !== undefined && WeakRefDeref(ref) !== undefined) {
      return;
    }
    MapDelete(canvasObjects, held[1]);
    native.canvas("release", held[0], held[1]);
  });

  function bindCanvasObject(object, kind, id, iface) {
    WeakMapSet(canvasTargets, object, { kind, id, iface });
    MapSet(canvasObjects, id, new $WeakRef(object));
    Register(canvasRegistry, object, [kind, id]);
    return object;
  }

  // The object for a gradient or pattern the host names, the one the
  // script holds or, when it let that go, a new one.
  function canvasObject(kind, id) {
    const ref = MapGet(canvasObjects, id);
    const object = ref === undefined ? undefined : WeakRefDeref(ref);
    if (object !== undefined) {
      return object;
    }
    return bindCanvasObject(
      kind === 3 ? new CanvasGradient(KEY) : new CanvasPattern(KEY),
      kind,
      id,
      kind === 3 ? "CanvasGradient" : "CanvasPattern",
    );
  }

  // Web IDL's `unrestricted double`: ECMAScript's ToNumber, which a
  // Symbol or a BigInt refuses.
  function unrestricted(value, method, iface) {
    if (typeof value === "number") {
      return value;
    }
    if (typeof value === "symbol" || typeof value === "bigint") {
      throw new $TypeError(prefix(method, iface) + "Cannot convert a " +
        (typeof value === "symbol" ? "Symbol" : "BigInt") + " value to a number");
    }
    return $Number(value);
  }

  // The same for an attribute's setter, in its words.
  function unrestrictedSet(value, property, iface) {
    if (typeof value === "number") {
      return value;
    }
    if (typeof value === "symbol" || typeof value === "bigint") {
      throw new $TypeError("Failed to set the '" + property + "' property on '" + iface +
        "': Cannot convert a " + (typeof value === "symbol" ? "Symbol" : "BigInt") +
        " value to a number");
    }
    return $Number(value);
  }

  // Web IDL's `double`: a value not finite a TypeError.
  function restricted(value, method, iface) {
    const n = unrestricted(value, method, iface);
    if (n !== n || n === Infinity || n === -Infinity) {
      throw new $TypeError(prefix(method, iface) + "The provided double value is non-finite.");
    }
    return n;
  }

  // Web IDL's `[EnforceRange] long`.
  function enforcedLong(value, method, iface) {
    const n = unrestricted(value, method, iface);
    if (n !== n || n === Infinity || n === -Infinity) {
      throw new $TypeError(prefix(method, iface) + "Value is not a finite number.");
    }
    const whole = MathTrunc(n);
    if (whole < -2147483648 || whole > 2147483647) {
      throw new $TypeError(prefix(method, iface) + "Value is outside the 'long' value range.");
    }
    return whole;
  }

  // A CanvasFillRule, its default `nonzero` for undefined: 1 for
  // `evenodd`, 0 for `nonzero`.
  function fillRule(value, method, iface) {
    if (value === undefined) {
      return 0;
    }
    const text = str(value);
    if (text === "nonzero") {
      return 0;
    }
    if (text === "evenodd") {
      return 1;
    }
    throw new $TypeError(prefix(method, iface) + "The provided value '" + text +
      "' is not a valid enum value of type CanvasFillRule.");
  }

  function finite(n) {
    return n === n && n !== Infinity && n !== -Infinity;
  }

  function isPath2D(value) {
    if (value === null || typeof value !== "object") {
      return false;
    }
    const t = WeakMapGet(canvasTargets, value);
    return t !== undefined && t.kind === 2;
  }

  // The functions `filter` takes (Filter Effects 1), by name: a value
  // of them, or `none`, is kept; any other changes nothing. It is not
  // drawn (G81).
  const FILTERS = ["blur", "brightness", "contrast", "drop-shadow", "grayscale",
    "hue-rotate", "invert", "opacity", "saturate", "sepia", "url"];
  function validFilter(text) {
    let i = 0;
    const n = text.length;
    const space = () => {
      while (i < n && (text[i] === " " || text[i] === "\t" || text[i] === "\n")) {
        i++;
      }
    };
    space();
    if (StringSlice(text, i, i + 4) === "none") {
      const start = i;
      i += 4;
      space();
      if (i === n) {
        return true;
      }
      i = start;
    }
    let count = 0;
    while (i < n) {
      const open = StringIndexOf(text, "(", i);
      if (open < 0) {
        return false;
      }
      const name = ToLowerCase(StringSlice(text, i, open));
      let known = false;
      for (let k = 0; k < FILTERS.length; k++) {
        if (FILTERS[k] === name) {
          known = true;
        }
      }
      const close = StringIndexOf(text, ")", open);
      if (!known || close < 0) {
        return false;
      }
      i = close + 1;
      count++;
      space();
    }
    return count > 0;
  }

  // The contexts of canvas elements, by the element's object, which a
  // context holds so that its element lives as long as it does.
  const canvasContexts = new $WeakMap();

  class CanvasRenderingContext2D {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor(CTX);
      }
    }

    get canvas() {
      return targetOf(this).element;
    }

    getContextAttributes() {
      const t = targetOf(this);
      return {
        alpha: t.alpha,
        colorSpace: "srgb",
        colorType: "unorm8",
        desynchronized: false,
        toneMapping: { mode: "standard" },
        willReadFrequently: t.willReadFrequently,
      };
    }

    isContextLost() {
      targetOf(this);
      return false;
    }

    save() {
      native.canvas("save", 0, targetOf(this).id);
    }

    restore() {
      native.canvas("restore", 0, targetOf(this).id);
    }

    reset() {
      native.canvas("reset", 0, targetOf(this).id);
    }

    scale(x, y) {
      const id = targetOf(this).id;
      need(arguments.length, 2, "scale", CTX);
      native.canvas("scale", 0, id, unrestricted(x, "scale", CTX), unrestricted(y, "scale", CTX));
    }

    rotate(angle) {
      const id = targetOf(this).id;
      need(arguments.length, 1, "rotate", CTX);
      native.canvas("rotate", 0, id, unrestricted(angle, "rotate", CTX));
    }

    translate(x, y) {
      const id = targetOf(this).id;
      need(arguments.length, 2, "translate", CTX);
      native.canvas("translate", 0, id, unrestricted(x, "translate", CTX),
        unrestricted(y, "translate", CTX));
    }

    transform(a, b, c, d, e, f) {
      const id = targetOf(this).id;
      need(arguments.length, 6, "transform", CTX);
      native.canvas("transform", 0, id, unrestricted(a, "transform", CTX),
        unrestricted(b, "transform", CTX), unrestricted(c, "transform", CTX),
        unrestricted(d, "transform", CTX), unrestricted(e, "transform", CTX),
        unrestricted(f, "transform", CTX));
    }

    getTransform() {
      const m = native.canvas("getTransform", 0, targetOf(this).id);
      return makeMatrix(DOMMatrix, fromList(m, true));
    }

    setTransform() {
      const id = targetOf(this).id;
      if (arguments.length >= 6) {
        const v = [];
        for (let i = 0; i < 6; i++) {
          v[i] = unrestricted(arguments[i], "setTransform", CTX);
        }
        native.canvas("setTransform", 0, id, v[0], v[1], v[2], v[3], v[4], v[5]);
        return;
      }
      if (arguments.length > 1) {
        throw new $TypeError(prefix("setTransform", CTX) +
          "The provided value is not of type 'DOMMatrixInit'.");
      }
      const m = matrixInit(arguments[0], "setTransform", CTX, false).s;
      native.canvas("setTransform", 0, id, m[0], m[1], m[4], m[5], m[12], m[13]);
    }

    resetTransform() {
      native.canvas("resetTransform", 0, targetOf(this).id);
    }

    get globalAlpha() {
      return native.canvas("globalAlpha", 0, targetOf(this).id);
    }

    set globalAlpha(value) {
      const id = targetOf(this).id;
      native.canvas("globalAlpha", 0, id, unrestrictedSet(value, "globalAlpha", CTX));
    }

    get globalCompositeOperation() {
      return native.canvas("globalCompositeOperation", 0, targetOf(this).id);
    }

    set globalCompositeOperation(value) {
      native.canvas("globalCompositeOperation", 0, targetOf(this).id, str(value));
    }

    get imageSmoothingEnabled() {
      return native.canvas("imageSmoothingEnabled", 0, targetOf(this).id);
    }

    set imageSmoothingEnabled(value) {
      native.canvas("imageSmoothingEnabled", 0, targetOf(this).id, !!value);
    }

    get imageSmoothingQuality() {
      return native.canvas("imageSmoothingQuality", 0, targetOf(this).id);
    }

    set imageSmoothingQuality(value) {
      native.canvas("imageSmoothingQuality", 0, targetOf(this).id, str(value));
    }

    get strokeStyle() {
      return paintOf(native.canvas("strokeStyle", 0, targetOf(this).id));
    }

    set strokeStyle(value) {
      setPaint(targetOf(this).id, "strokeStyle", value);
    }

    get fillStyle() {
      return paintOf(native.canvas("fillStyle", 0, targetOf(this).id));
    }

    set fillStyle(value) {
      setPaint(targetOf(this).id, "fillStyle", value);
    }

    createLinearGradient(x0, y0, x1, y1) {
      targetOf(this);
      need(arguments.length, 4, "createLinearGradient", CTX);
      const m = "createLinearGradient";
      const id = native.canvas("linearGradient", restricted(x0, m, CTX), restricted(y0, m, CTX),
        restricted(x1, m, CTX), restricted(y1, m, CTX));
      return bindCanvasObject(new CanvasGradient(KEY), 3, id, "CanvasGradient");
    }

    createRadialGradient(x0, y0, r0, x1, y1, r1) {
      targetOf(this);
      const m = "createRadialGradient";
      need(arguments.length, 6, m, CTX);
      const v = [restricted(x0, m, CTX), restricted(y0, m, CTX), restricted(r0, m, CTX),
        restricted(x1, m, CTX), restricted(y1, m, CTX), restricted(r1, m, CTX)];
      let id;
      try {
        id = native.canvas("radialGradient", v[0], v[1], v[2], v[3], v[4], v[5]);
      } catch (error) {
        throw failed(error, m, CTX);
      }
      return bindCanvasObject(new CanvasGradient(KEY), 3, id, "CanvasGradient");
    }

    createConicGradient(angle, x, y) {
      targetOf(this);
      const m = "createConicGradient";
      need(arguments.length, 3, m, CTX);
      const id = native.canvas("conicGradient", restricted(angle, m, CTX), restricted(x, m, CTX),
        restricted(y, m, CTX));
      return bindCanvasObject(new CanvasGradient(KEY), 3, id, "CanvasGradient");
    }

    get shadowOffsetX() {
      return native.canvas("shadowOffsetX", 0, targetOf(this).id);
    }

    set shadowOffsetX(value) {
      const id = targetOf(this).id;
      native.canvas("shadowOffsetX", 0, id, unrestrictedSet(value, "shadowOffsetX", CTX));
    }

    get shadowOffsetY() {
      return native.canvas("shadowOffsetY", 0, targetOf(this).id);
    }

    set shadowOffsetY(value) {
      const id = targetOf(this).id;
      native.canvas("shadowOffsetY", 0, id, unrestrictedSet(value, "shadowOffsetY", CTX));
    }

    get shadowBlur() {
      return native.canvas("shadowBlur", 0, targetOf(this).id);
    }

    set shadowBlur(value) {
      const id = targetOf(this).id;
      native.canvas("shadowBlur", 0, id, unrestrictedSet(value, "shadowBlur", CTX));
    }

    get shadowColor() {
      return native.canvas("shadowColor", 0, targetOf(this).id);
    }

    set shadowColor(value) {
      native.canvas("shadowColor", 0, targetOf(this).id, str(value));
    }

    get filter() {
      return native.canvas("filter", 0, targetOf(this).id);
    }

    set filter(value) {
      const id = targetOf(this).id;
      const text = str(value);
      if (validFilter(text)) {
        native.canvas("filter", 0, id, text);
      }
    }

    clearRect(x, y, w, h) {
      rectangle(this, "clearRect", arguments.length, x, y, w, h);
    }

    fillRect(x, y, w, h) {
      rectangle(this, "fillRect", arguments.length, x, y, w, h);
    }

    strokeRect(x, y, w, h) {
      rectangle(this, "strokeRect", arguments.length, x, y, w, h);
    }

    beginPath() {
      native.canvas("beginPath", 0, targetOf(this).id);
    }

    fill() {
      const id = targetOf(this).id;
      const a = arguments;
      if (a.length > 0 && isPath2D(a[0])) {
        native.canvas("fill", 0, id, targetOf(a[0]).id, fillRule(a[1], "fill", CTX));
      } else {
        native.canvas("fill", 0, id, -1, fillRule(a[0], "fill", CTX));
      }
    }

    stroke() {
      const id = targetOf(this).id;
      const a = arguments;
      if (a.length > 0 && a[0] !== undefined) {
        if (!isPath2D(a[0])) {
          throw new $TypeError(prefix("stroke", CTX) + "parameter 1 is not of type 'Path2D'.");
        }
        native.canvas("stroke", 0, id, targetOf(a[0]).id);
      } else {
        native.canvas("stroke", 0, id, -1);
      }
    }

    clip() {
      const id = targetOf(this).id;
      const a = arguments;
      if (a.length > 0 && isPath2D(a[0])) {
        native.canvas("clip", 0, id, targetOf(a[0]).id, fillRule(a[1], "clip", CTX));
      } else {
        native.canvas("clip", 0, id, -1, fillRule(a[0], "clip", CTX));
      }
    }

    isPointInPath(a0, a1) {
      const id = targetOf(this).id;
      const a = arguments;
      const m = "isPointInPath";
      need(a.length, 2, m, CTX);
      if (isPath2D(a[0])) {
        need(a.length, 3, m, CTX);
        return native.canvas(m, 0, id, targetOf(a[0]).id, unrestricted(a[1], m, CTX),
          unrestricted(a[2], m, CTX), fillRule(a[3], m, CTX));
      }
      return native.canvas(m, 0, id, -1, unrestricted(a[0], m, CTX), unrestricted(a[1], m, CTX),
        fillRule(a[2], m, CTX));
    }

    isPointInStroke(a0, a1) {
      const id = targetOf(this).id;
      const a = arguments;
      const m = "isPointInStroke";
      need(a.length, 2, m, CTX);
      if (isPath2D(a[0])) {
        need(a.length, 3, m, CTX);
        return native.canvas(m, 0, id, targetOf(a[0]).id, unrestricted(a[1], m, CTX),
          unrestricted(a[2], m, CTX));
      }
      return native.canvas(m, 0, id, -1, unrestricted(a[0], m, CTX), unrestricted(a[1], m, CTX));
    }

    drawFocusIfNeeded(element) {
      targetOf(this);
      need(arguments.length, 1, "drawFocusIfNeeded", CTX);
    }

    get lineWidth() {
      return native.canvas("lineWidth", 0, targetOf(this).id);
    }

    set lineWidth(value) {
      const id = targetOf(this).id;
      native.canvas("lineWidth", 0, id, unrestrictedSet(value, "lineWidth", CTX));
    }

    get lineCap() {
      return native.canvas("lineCap", 0, targetOf(this).id);
    }

    set lineCap(value) {
      native.canvas("lineCap", 0, targetOf(this).id, str(value));
    }

    get lineJoin() {
      return native.canvas("lineJoin", 0, targetOf(this).id);
    }

    set lineJoin(value) {
      native.canvas("lineJoin", 0, targetOf(this).id, str(value));
    }

    get miterLimit() {
      return native.canvas("miterLimit", 0, targetOf(this).id);
    }

    set miterLimit(value) {
      const id = targetOf(this).id;
      native.canvas("miterLimit", 0, id, unrestrictedSet(value, "miterLimit", CTX));
    }

    setLineDash(segments) {
      const id = targetOf(this).id;
      need(arguments.length, 1, "setLineDash", CTX);
      const list = numberSequence(segments, "setLineDash", CTX);
      const args = ["setLineDash", 0, id];
      for (let i = 0; i < list.length; i++) {
        ArrayPush(args, list[i]);
      }
      ReflectApply(native.canvas, undefined, args);
    }

    getLineDash() {
      return native.canvas("getLineDash", 0, targetOf(this).id);
    }

    get lineDashOffset() {
      return native.canvas("lineDashOffset", 0, targetOf(this).id);
    }

    set lineDashOffset(value) {
      const id = targetOf(this).id;
      native.canvas("lineDashOffset", 0, id, unrestrictedSet(value, "lineDashOffset", CTX));
    }

    get font() {
      return native.canvas("font", 0, targetOf(this).id);
    }

    set font(value) {
      native.canvas("font", 0, targetOf(this).id, str(value));
    }

    get textAlign() {
      return native.canvas("textAlign", 0, targetOf(this).id);
    }

    set textAlign(value) {
      native.canvas("textAlign", 0, targetOf(this).id, str(value));
    }

    get textBaseline() {
      return native.canvas("textBaseline", 0, targetOf(this).id);
    }

    set textBaseline(value) {
      native.canvas("textBaseline", 0, targetOf(this).id, str(value));
    }

    get direction() {
      return native.canvas("direction", 0, targetOf(this).id);
    }

    set direction(value) {
      native.canvas("direction", 0, targetOf(this).id, str(value));
    }

    get letterSpacing() {
      return native.canvas("letterSpacing", 0, targetOf(this).id);
    }

    set letterSpacing(value) {
      native.canvas("letterSpacing", 0, targetOf(this).id, str(value));
    }

    get wordSpacing() {
      return native.canvas("wordSpacing", 0, targetOf(this).id);
    }

    set wordSpacing(value) {
      native.canvas("wordSpacing", 0, targetOf(this).id, str(value));
    }

    get fontKerning() {
      return native.canvas("fontKerning", 0, targetOf(this).id);
    }

    set fontKerning(value) {
      native.canvas("fontKerning", 0, targetOf(this).id, str(value));
    }

    get fontStretch() {
      return native.canvas("fontStretch", 0, targetOf(this).id);
    }

    set fontStretch(value) {
      native.canvas("fontStretch", 0, targetOf(this).id, str(value));
    }

    get fontVariantCaps() {
      return native.canvas("fontVariantCaps", 0, targetOf(this).id);
    }

    set fontVariantCaps(value) {
      native.canvas("fontVariantCaps", 0, targetOf(this).id, str(value));
    }

    get textRendering() {
      return native.canvas("textRendering", 0, targetOf(this).id);
    }

    set textRendering(value) {
      native.canvas("textRendering", 0, targetOf(this).id, str(value));
    }

    get lang() {
      return native.canvas("lang", 0, targetOf(this).id);
    }

    set lang(value) {
      native.canvas("lang", 0, targetOf(this).id, str(value));
    }

    fillText(text, x, y) {
      drawText(this, "fillText", arguments);
    }

    strokeText(text, x, y) {
      drawText(this, "strokeText", arguments);
    }

    measureText(text) {
      const id = targetOf(this).id;
      need(arguments.length, 1, "measureText", CTX);
      const m = native.canvas("measureText", 0, id, str(text));
      const metrics = new TextMetrics(KEY);
      WeakMapSet(textMetrics, metrics, m);
      return metrics;
    }

    drawImage(image, a, b) {
      const id = targetOf(this).id;
      const m = "drawImage";
      const n = arguments.length;
      need(n, 3, m, CTX);
      if (n !== 3 && n !== 5 && n < 9) {
        throw new $TypeError(prefix(m, CTX) + "Overload resolution failed.");
      }
      const source = imageSource(image, m);
      const count = n >= 9 ? 8 : n - 1;
      const args = [m, 0, id, source[0], source[1]];
      for (let i = 1; i <= count; i++) {
        ArrayPush(args, unrestricted(arguments[i], m, CTX));
      }
      try {
        ReflectApply(native.canvas, undefined, args);
      } catch (error) {
        throw failed(error, m, CTX);
      }
    }

    createPattern(image, repetition) {
      const id = targetOf(this).id;
      const m = "createPattern";
      need(arguments.length, 2, m, CTX);
      const source = imageSource(image, m);
      const type = repetition === null ? "" : str(repetition);
      if (type !== "" && type !== "repeat" && type !== "repeat-x" && type !== "repeat-y" &&
        type !== "no-repeat") {
        throw domError("SyntaxError", "The provided type ('" + type +
          "') is not one of 'repeat', 'no-repeat', 'repeat-x', or 'repeat-y'.", m, CTX);
      }
      let made;
      try {
        made = native.canvas(m, 0, id, source[0], source[1], type);
      } catch (error) {
        throw failed(error, m, CTX);
      }
      if (made === null) {
        return null;
      }
      return bindCanvasObject(new CanvasPattern(KEY), 4, made, "CanvasPattern");
    }

    createImageData(a0) {
      targetOf(this);
      const m = "createImageData";
      need(arguments.length, 1, m, CTX);
      if (arguments.length === 1) {
        const data = WeakMapGet(imageDatas, a0);
        if (data === undefined) {
          throw new $TypeError(prefix(m, CTX) + "parameter 1 is not of type 'ImageData'.");
        }
        return blankImageData(data.width, data.height, m, CTX, data.colorSpace);
      }
      const w = enforcedLong(arguments[0], m, CTX);
      const h = enforcedLong(arguments[1], m, CTX);
      return blankImageData(w < 0 ? -w : w, h < 0 ? -h : h, m, CTX,
        imageDataColorSpace(arguments[2], m, CTX));
    }

    getImageData(sx, sy, sw, sh) {
      const id = targetOf(this).id;
      const m = "getImageData";
      need(arguments.length, 4, m, CTX);
      const v = [enforcedLong(sx, m, CTX), enforcedLong(sy, m, CTX), enforcedLong(sw, m, CTX),
        enforcedLong(sh, m, CTX)];
      let buffer;
      try {
        buffer = native.canvas(m, 0, id, v[0], v[1], v[2], v[3]);
      } catch (error) {
        throw failed(error, m, CTX);
      }
      const data = new $Uint8ClampedArray(buffer);
      return makeImageData(data, v[2] < 0 ? -v[2] : v[2], v[3] < 0 ? -v[3] : v[3], "srgb");
    }

    putImageData(imagedata, dx, dy) {
      const id = targetOf(this).id;
      const m = "putImageData";
      need(arguments.length, 3, m, CTX);
      const data = WeakMapGet(imageDatas, imagedata);
      if (data === undefined) {
        throw new $TypeError(prefix(m, CTX) + "parameter 1 is not of type 'ImageData'.");
      }
      const a = arguments;
      if (a.length > 3 && a.length < 7) {
        throw new $TypeError(prefix(m, CTX) + "Overload resolution failed.");
      }
      const x = enforcedLong(dx, m, CTX);
      const y = enforcedLong(dy, m, CTX);
      const bytes = ToBase64(new $Uint8Array(TypedArrayBuffer(data.data),
        TypedArrayOffset(data.data), TypedArrayLength(data.data)));
      if (a.length >= 7) {
        native.canvasData(m, bytes, 0, id, data.width, data.height, x, y,
          enforcedLong(a[3], m, CTX), enforcedLong(a[4], m, CTX), enforcedLong(a[5], m, CTX),
          enforcedLong(a[6], m, CTX));
      } else {
        native.canvasData(m, bytes, 0, id, data.width, data.height, x, y);
      }
    }
  }

  // fillRect, strokeRect and clearRect.
  function rectangle(context, m, count, x, y, w, h) {
    const id = targetOf(context).id;
    need(count, 4, m, CTX);
    native.canvas(m, 0, id, unrestricted(x, m, CTX), unrestricted(y, m, CTX),
      unrestricted(w, m, CTX), unrestricted(h, m, CTX));
  }

  // A style the host answers as a colour's text, or a gradient's or a
  // pattern's kind and number.
  function paintOf(answer) {
    return typeof answer === "string" ? answer : canvasObject(answer[0], answer[1]);
  }

  function setPaint(id, property, value) {
    if (value !== null && typeof value === "object") {
      const t = WeakMapGet(canvasTargets, value);
      if (t !== undefined && (t.kind === 3 || t.kind === 4)) {
        native.canvas(property, 0, id, t.kind, t.id);
        return;
      }
    }
    native.canvas(property, 0, id, str(value));
  }

  // A sequence of unrestricted doubles, from anything iterable.
  function numberSequence(value, method, iface) {
    if (value === null || (typeof value !== "object" && typeof value !== "function") ||
      typeof value[iteratorSymbol] !== "function") {
      throw new $TypeError(prefix(method, iface) +
        "The provided value cannot be converted to a sequence.");
    }
    const list = [];
    const iterator = value[iteratorSymbol]();
    for (;;) {
      const step = iterator.next();
      if (step.done) {
        break;
      }
      ArrayPush(list, unrestricted(step.value, method, iface));
    }
    return list;
  }

  // CanvasPath (HTML section 4.12.5.1.5), on contexts and Path2D alike.
  const CANVAS_PATH = {
    closePath() {
      const t = targetOf(this);
      native.canvas("closePath", t.kind, t.id);
    },

    moveTo(x, y) {
      pathCall(this, "moveTo", 2, arguments);
    },

    lineTo(x, y) {
      pathCall(this, "lineTo", 2, arguments);
    },

    quadraticCurveTo(cpx, cpy, x, y) {
      pathCall(this, "quadraticCurveTo", 4, arguments);
    },

    bezierCurveTo(cp1x, cp1y, cp2x, cp2y, x, y) {
      pathCall(this, "bezierCurveTo", 6, arguments);
    },

    arcTo(x1, y1, x2, y2, radius) {
      pathCall(this, "arcTo", 5, arguments);
    },

    rect(x, y, w, h) {
      pathCall(this, "rect", 4, arguments);
    },

    roundRect(x, y, w, h) {
      const t = targetOf(this);
      const m = "roundRect";
      need(arguments.length, 4, m, t.iface);
      const v = [unrestricted(x, m, t.iface), unrestricted(y, m, t.iface),
        unrestricted(w, m, t.iface), unrestricted(h, m, t.iface)];
      const radii = roundRectRadii(arguments[4], m, t.iface);
      if (radii === null) {
        return;
      }
      for (let i = 0; i < radii.length; i++) {
        ArrayPush(v, radii[i]);
      }
      const args = [m, t.kind, t.id];
      for (let i = 0; i < v.length; i++) {
        ArrayPush(args, v[i]);
      }
      try {
        ReflectApply(native.canvas, undefined, args);
      } catch (error) {
        throw failed(error, m, t.iface);
      }
    },

    arc(x, y, radius, startAngle, endAngle) {
      pathCall(this, "arc", 5, arguments, 6);
    },

    ellipse(x, y, radiusX, radiusY, rotation, startAngle, endAngle) {
      pathCall(this, "ellipse", 7, arguments, 8);
    },
  };

  // A CanvasPath method of `count` numbers, and when `flag` is given, a
  // boolean after them (`counterclockwise`) to make that many.
  function pathCall(object, m, count, a, flag) {
    const t = targetOf(object);
    need(a.length, count, m, t.iface);
    // moveTo and lineTo, the most made, called straight (M30.4).
    if (count === 2 && flag === undefined) {
      const x = unrestricted(a[0], m, t.iface);
      const y = unrestricted(a[1], m, t.iface);
      try {
        native.canvas(m, t.kind, t.id, x, y);
      } catch (error) {
        throw failed(error, m, t.iface);
      }
      return;
    }
    // arc and arcTo, five numbers and arc's flag.
    if (count === 5) {
      const x = unrestricted(a[0], m, t.iface);
      const y = unrestricted(a[1], m, t.iface);
      const p = unrestricted(a[2], m, t.iface);
      const q = unrestricted(a[3], m, t.iface);
      const r = unrestricted(a[4], m, t.iface);
      try {
        if (flag === undefined) {
          native.canvas(m, t.kind, t.id, x, y, p, q, r);
        } else {
          native.canvas(m, t.kind, t.id, x, y, p, q, r, !!a[5]);
        }
      } catch (error) {
        throw failed(error, m, t.iface);
      }
      return;
    }
    const args = [m, t.kind, t.id];
    for (let i = 0; i < count; i++) {
      ArrayPush(args, unrestricted(a[i], m, t.iface));
    }
    if (flag !== undefined) {
      ArrayPush(args, !!a[count]);
    }
    try {
      ReflectApply(native.canvas, undefined, args);
    } catch (error) {
      throw failed(error, m, t.iface);
    }
  }

  // roundRect's radii (HTML section 4.12.5.1.5) as (x, y) pairs, in the
  // reference browser's words when they are refused; null when one is
  // not finite, which draws nothing.
  function roundRectRadii(value, m, iface) {
    let list;
    if (value === undefined) {
      list = [0];
    } else if (value !== null && (typeof value === "object" || typeof value === "function") &&
      typeof value[iteratorSymbol] === "function") {
      list = [];
      const iterator = value[iteratorSymbol]();
      for (;;) {
        const step = iterator.next();
        if (step.done) {
          break;
        }
        ArrayPush(list, step.value);
      }
    } else {
      list = [value];
    }
    if (list.length < 1 || list.length > 4) {
      throw new $RangeError(prefix(m, iface) + list.length +
        " radii provided. Between one and four radii are necessary.");
    }
    const out = [];
    for (let i = 0; i < list.length; i++) {
      const r = list[i];
      if (r !== null && (typeof r === "object" || typeof r === "function")) {
        const x = r.x === undefined ? 0 : unrestricted(r.x, m, iface);
        const y = r.y === undefined ? 0 : unrestricted(r.y, m, iface);
        if (!finite(x) || !finite(y)) {
          return null;
        }
        if (x < 0) {
          throw new $RangeError(prefix(m, iface) + "X-radius value " + x + " is negative.");
        }
        if (y < 0) {
          throw new $RangeError(prefix(m, iface) + "Y-radius value " + y + " is negative.");
        }
        ArrayPush(out, x);
        ArrayPush(out, y);
      } else {
        const v = unrestricted(r, m, iface);
        if (!finite(v)) {
          return null;
        }
        if (v < 0) {
          throw new $RangeError(prefix(m, iface) + "Radius value " + v + " is negative.");
        }
        ArrayPush(out, v);
        ArrayPush(out, v);
      }
    }
    return out;
  }

  class CanvasGradient {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("CanvasGradient");
      }
    }

    addColorStop(offset, color) {
      const t = targetOf(this);
      const m = "addColorStop";
      need(arguments.length, 2, m, "CanvasGradient");
      const at = restricted(offset, m, "CanvasGradient");
      const text = str(color);
      try {
        native.canvas(m, 3, t.id, at, text);
      } catch (error) {
        throw failed(error, m, "CanvasGradient");
      }
    }
  }

  class CanvasPattern {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("CanvasPattern");
      }
    }

    setTransform(transform) {
      const t = targetOf(this);
      const s = matrixInit(transform, "setTransform", "CanvasPattern", false).s;
      native.canvas("patternTransform", 4, t.id, s[0], s[1], s[4], s[5], s[12], s[13]);
    }
  }

  class Path2D {
    constructor() {
      const a = arguments;
      let id;
      if (a.length === 0 || a[0] === undefined) {
        id = native.canvas("path");
      } else if (isPath2D(a[0])) {
        id = native.canvas("path", targetOf(a[0]).id);
      } else {
        id = native.canvas("path", str(a[0]));
      }
      bindCanvasObject(this, 2, id, "Path2D");
    }

    addPath(path) {
      const t = targetOf(this);
      need(arguments.length, 1, "addPath", "Path2D");
      if (!isPath2D(path)) {
        throw new $TypeError(prefix("addPath", "Path2D") + "parameter 1 is not of type 'Path2D'.");
      }
      const s = matrixInit(arguments[1], "addPath", "Path2D", false).s;
      native.canvas("addPath", 2, t.id, targetOf(path).id, s[0], s[1], s[4], s[5], s[12], s[13]);
    }
  }

  include(CANVAS_PATH, [CanvasRenderingContext2D.prototype, Path2D.prototype]);

  // ---- ImageData (HTML section 4.12.5.1.16) ---------------------------------

  // Each ImageData's pixels, its size and its colour space.
  const imageDatas = new $WeakMap();

  function makeImageData(data, width, height, colorSpace) {
    const object = new ImageData(KEY);
    WeakMapSet(imageDatas, object, { data, width, height, colorSpace });
    return object;
  }

  // An ImageDataSettings' colour space.
  // The settings a 2D context is made with (HTML 4.12.5.1.1), read as
  // Web IDL converts the dictionary: its members in the order of their
  // names, the colour space and type as their enumerations. Anything not
  // an object gives the defaults, as the reference browser takes it.
  function contextSettings(options, iface) {
    const settings = { alpha: true, willReadFrequently: false };
    if (options === undefined || options === null ||
        (typeof options !== "object" && typeof options !== "function")) {
      return settings;
    }
    const wrong = (member, type, text) => new $TypeError(prefix("getContext", iface) +
      "Failed to read the '" + member + "' property from 'CanvasRenderingContext2DSettings': " +
      "The provided value '" + text + "' is not a valid enum value of type " + type + ".");
    const alpha = options.alpha;
    if (alpha !== undefined) {
      settings.alpha = !!alpha;
    }
    const colorSpace = options.colorSpace;
    if (colorSpace !== undefined) {
      const text = str(colorSpace);
      if (text !== "srgb" && text !== "display-p3") {
        throw wrong("colorSpace", "PredefinedColorSpace", text);
      }
    }
    const colorType = options.colorType;
    if (colorType !== undefined) {
      const text = str(colorType);
      if (text !== "unorm8" && text !== "float16") {
        throw wrong("colorType", "CanvasColorType", text);
      }
    }
    // Read, as the dictionary is, though nothing is drawn apart.
    void options.desynchronized;
    const frequently = options.willReadFrequently;
    if (frequently !== undefined) {
      settings.willReadFrequently = !!frequently;
    }
    return settings;
  }

  function imageDataColorSpace(settings, m, iface) {
    if (settings === undefined || settings === null) {
      return "srgb";
    }
    // A dictionary converts from an object alone.
    if (typeof settings !== "object" && typeof settings !== "function") {
      throw new $TypeError(prefix(m, iface) +
        "The provided value is not of type 'ImageDataSettings'.");
    }
    const given = settings.colorSpace;
    if (given === undefined) {
      return "srgb";
    }
    const text = str(given);
    if (text !== "srgb" && text !== "display-p3") {
      throw new $TypeError(prefix(m, iface) + "Failed to read the 'colorSpace' property from " +
        "'ImageDataSettings': The provided value '" + text +
        "' is not a valid enum value of type PredefinedColorSpace.");
    }
    return text;
  }

  // A transparent ImageData, its size within the canvas's bound.
  function blankImageData(width, height, m, iface, colorSpace) {
    if (width === 0) {
      throw domError("IndexSizeError", "The source width is zero or not a number.", m, iface);
    }
    if (height === 0) {
      throw domError("IndexSizeError", "The source height is zero or not a number.", m, iface);
    }
    if (width * height > 33554432) {
      throw domError("IndexSizeError", "The requested image size exceeds the supported range.",
        m, iface);
    }
    return makeImageData(new $Uint8ClampedArray(width * height * 4), width, height, colorSpace);
  }

  class ImageData {
    constructor(a0, a1) {
      if (a0 === KEY) {
        return;
      }
      const a = arguments;
      const c = "constructor";
      const where = (message) => "Failed to construct 'ImageData': " + message;
      // Web IDL's `unsigned long`: ToUint32, NaN 0.
      const unsignedLong = (value) => {
        if (typeof value === "symbol" || typeof value === "bigint") {
          throw new $TypeError(where("Cannot convert a " +
            (typeof value === "symbol" ? "Symbol" : "BigInt") + " value to a number"));
        }
        return $Number(value) >>> 0;
      };
      if (a.length < 2) {
        throw new $TypeError(where(
          "2 arguments required, but only " + a.length + " present."));
      }
      if (a0 !== null && typeof a0 === "object" && TypedArrayTag(a0) === "Uint8ClampedArray") {
        const length = TypedArrayLength(a0);
        const width = unsignedLong(a1);
        const given = a.length > 2 && a[2] !== undefined ? unsignedLong(a[2]) : undefined;
        const colorSpace = imageDataColorSpace(a[3], c, "ImageData");
        if (length === 0 || length % 4 !== 0) {
          throw new $DOMException(where("The input data length is not a multiple of 4."),
            "InvalidStateError");
        }
        if (width === 0) {
          throw new $DOMException(where("The source width is zero or not a number."),
            "IndexSizeError");
        }
        if ((length / 4) % width !== 0) {
          throw new $DOMException(where("The input data length is not a multiple of (4 * width)."),
            "IndexSizeError");
        }
        const height = length / 4 / width;
        if (given !== undefined && given !== height) {
          throw new $DOMException(
            where("The input data length is not equal to (4 * width * height)."),
            "IndexSizeError");
        }
        WeakMapSet(imageDatas, this, {
          data: a0,
          width,
          height,
          colorSpace,
        });
        return;
      }
      const width = unsignedLong(a0);
      const height = unsignedLong(a1);
      const colorSpace = imageDataColorSpace(a[2], c, "ImageData");
      if (width === 0) {
        throw new $DOMException(where("The source width is zero or not a number."),
          "IndexSizeError");
      }
      if (height === 0) {
        throw new $DOMException(where("The source height is zero or not a number."),
          "IndexSizeError");
      }
      if (width * height > 33554432) {
        throw new $DOMException(where("The requested image size exceeds the supported range."),
          "IndexSizeError");
      }
      WeakMapSet(imageDatas, this, {
        data: new $Uint8ClampedArray(width * height * 4),
        width,
        height,
        colorSpace,
      });
    }

    get width() {
      return imageDataOf(this).width;
    }

    get height() {
      return imageDataOf(this).height;
    }

    get data() {
      return imageDataOf(this).data;
    }

    get colorSpace() {
      return imageDataOf(this).colorSpace;
    }

    get pixelFormat() {
      imageDataOf(this);
      return "rgba-unorm8";
    }
  }

  function imageDataOf(object) {
    const data = WeakMapGet(imageDatas, object);
    if (data === undefined) {
      throw illegalInvocation();
    }
    return data;
  }

  // A CanvasImageSource as the host knows it: its kind (1 an img, 2 a
  // canvas, 5 a video) and its node's handle; anything else, a
  // TypeError in the reference browser's words.
  function imageSource(value, m) {
    const t = value !== null && typeof value === "object" ? WeakMapGet(canvasTargets, value) : undefined;
    if (t !== undefined && t.kind === 1) {
      // One transferred away is detached (HTML's "check the usability
      // of the image argument").
      if (t.detached) {
        throw new $DOMException(prefix(m, CTX) + "The image source is detached.", "InvalidStateError");
      }
      return [3, t.id];
    }
    if (t !== undefined && t.kind === 5) {
      return [4, t.id];
    }
    if (isNode(value) && kindOf(value) === 1) {
      const handle = handleOf(value);
      const tag = native.tag(handle);
      if (tag === "img") {
        return [1, handle];
      }
      if (tag === "canvas") {
        return [2, handle];
      }
      if (tag === "video") {
        return [5, handle];
      }
    }
    throw new $TypeError(prefix(m, CTX) + "The provided value is not of type '(CSSImageValue " +
      "or HTMLCanvasElement or HTMLImageElement or HTMLVideoElement or ImageBitmap or " +
      "OffscreenCanvas or SVGImageElement or VideoFrame)'.");
  }

  // fillText and strokeText: the text, the point, and the maximum
  // width when one is given.
  function drawText(context, m, a) {
    const id = targetOf(context).id;
    need(a.length, 3, m, CTX);
    const text = str(a[0]);
    const x = unrestricted(a[1], m, CTX);
    const y = unrestricted(a[2], m, CTX);
    if (a.length > 3 && a[3] !== undefined) {
      native.canvas(m, 0, id, text, x, y, unrestricted(a[3], m, CTX));
    } else {
      native.canvas(m, 0, id, text, x, y);
    }
  }

  // Each TextMetrics' numbers, in the order the host gives them.
  const textMetrics = new $WeakMap();

  class TextMetrics {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("TextMetrics");
      }
    }
  }
  const TEXT_METRICS = ["width", "actualBoundingBoxLeft", "actualBoundingBoxRight",
    "actualBoundingBoxAscent", "actualBoundingBoxDescent", "fontBoundingBoxAscent",
    "fontBoundingBoxDescent", "alphabeticBaseline", "hangingBaseline", "ideographicBaseline"];
  for (let i = 0; i < TEXT_METRICS.length; i++) {
    defineProperty(TextMetrics.prototype, TEXT_METRICS[i], {
      get() {
        const m = WeakMapGet(textMetrics, this);
        if (m === undefined) {
          throw illegalInvocation();
        }
        return m[i];
      },
      enumerable: true,
      configurable: true,
    });
  }

  expose(TextMetrics, "TextMetrics");
  expose(CanvasRenderingContext2D, CTX);
  expose(CanvasGradient, "CanvasGradient");
  expose(CanvasPattern, "CanvasPattern");
  expose(Path2D, "Path2D");
  expose(ImageData, "ImageData");

  // A PNG file's bytes, as the host gives them, a Blob.
  function pngBlob(buffer) {
    return new Blob(KEY, { bytes: new $Uint8Array(buffer), type: "image/png" });
  }

  // ---- OffscreenCanvas and ImageBitmap (HTML 4.12.5.3, 8.10, M30.2) ----

  // Web IDL's `[EnforceRange] unsigned long long`, for a size.
  function enforcedSize(value, where) {
    const n = typeof value === "number" ? value : $Number(value);
    if (n !== n || n === Infinity || n === -Infinity || MathTrunc(n) < 0 ||
      MathTrunc(n) > 4294967295) {
      throw new $TypeError(where + "Value is outside the 'unsigned long' value range.");
    }
    return MathTrunc(n);
  }

  class OffscreenCanvas {
    constructor(width, height) {
      needToConstruct(arguments.length, 2, "OffscreenCanvas");
      const where = "Failed to construct 'OffscreenCanvas': ";
      const w = enforcedSize(width, where);
      const h = enforcedSize(height, where);
      const id = native.canvas("offscreen", w, h);
      bindCanvasObject(this, 1, id, "OffscreenCanvas");
      const t = targetOf(this);
      t.width = w;
      t.height = h;
      t.context = null;
    }

    get width() {
      return targetOf(this).width;
    }

    set width(value) {
      const t = targetOf(this);
      t.width = enforcedSize(value, "Failed to set the 'width' property on 'OffscreenCanvas': ");
      native.canvas("offscreenResize", 1, t.id, t.width, t.height);
    }

    get height() {
      return targetOf(this).height;
    }

    set height(value) {
      const t = targetOf(this);
      t.height = enforcedSize(value, "Failed to set the 'height' property on 'OffscreenCanvas': ");
      native.canvas("offscreenResize", 1, t.id, t.width, t.height);
    }

    getContext(contextId) {
      const t = targetOf(this);
      need(arguments.length, 1, "getContext", "OffscreenCanvas");
      const id = str(contextId);
      // OffscreenRenderingContextId, an enumeration: a name outside it
      // a TypeError, one in it but "2d" no context.
      if (id !== "2d" && id !== "bitmaprenderer" && id !== "webgl" && id !== "webgl2" &&
          id !== "webgpu") {
        throw new $TypeError(prefix("getContext", "OffscreenCanvas") + "The provided value '" +
          id + "' is not a valid enum value of type OffscreenRenderingContextId.");
      }
      if (id !== "2d") {
        return null;
      }
      if (t.context !== null) {
        return t.context;
      }
      const { alpha, willReadFrequently } = contextSettings(arguments[1], "OffscreenCanvas");
      native.canvas("context", 0, -t.id, alpha);
      const context = new OffscreenCanvasRenderingContext2D(KEY);
      WeakMapSet(canvasTargets, context, {
        kind: 0,
        id: -t.id,
        iface: "OffscreenCanvasRenderingContext2D",
        element: this,
        alpha,
        willReadFrequently,
      });
      t.context = context;
      return context;
    }

    convertToBlob() {
      const t = targetOf(this);
      let bytes;
      try {
        bytes = native.canvas("encode", 0, -t.id);
      } catch (error) {
        return rejected(failed(error, "convertToBlob", "OffscreenCanvas"));
      }
      return resolved(pngBlob(bytes));
    }

    transferToImageBitmap() {
      const t = targetOf(this);
      let made;
      try {
        made = native.canvas("transfer", 0, -t.id);
      } catch (error) {
        throw failed(error, "transferToImageBitmap", "OffscreenCanvas");
      }
      return makeBitmap(made);
    }
  }

  // The context of an OffscreenCanvas: the 2D context's members but
  // those of an element's alone.
  class OffscreenCanvasRenderingContext2D {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("OffscreenCanvasRenderingContext2D");
      }
    }
  }
  {
    const names = getOwnPropertyNames(CanvasRenderingContext2D.prototype);
    for (let i = 0; i < names.length; i++) {
      if (names[i] === "constructor" || names[i] === "drawFocusIfNeeded") {
        continue;
      }
      defineProperty(OffscreenCanvasRenderingContext2D.prototype, names[i],
        getOwnPropertyDescriptor(CanvasRenderingContext2D.prototype, names[i]));
    }
  }

  class ImageBitmap {
    constructor() {
      if (arguments[0] !== KEY) {
        throw illegalConstructor("ImageBitmap");
      }
    }

    get width() {
      const t = targetOf(this);
      return t.closed ? 0 : t.width;
    }

    get height() {
      const t = targetOf(this);
      return t.closed ? 0 : t.height;
    }

    close() {
      const t = targetOf(this);
      t.closed = true;
      native.canvas("closeBitmap", 5, t.id);
    }
  }

  // A new ImageBitmap from the host's [number, width, height].
  function makeBitmap(made) {
    const bitmap = bindCanvasObject(new ImageBitmap(KEY), 5, made[0], "ImageBitmap");
    const t = targetOf(bitmap);
    t.width = made[1];
    t.height = made[2];
    t.closed = false;
    return bitmap;
  }

  // createImageBitmap() (HTML 8.10): its source, crop and options for the
  // host, which makes the bitmap; every refusal a rejected promise.
  function createImageBitmap(image) {
    const a = arguments;
    const where = prefix("createImageBitmap", GLOBAL_INTERFACE);
    try {
      if (a.length < 1) {
        throw new $TypeError(where + "1 argument required, but only 0 present.");
      }
      if (a.length > 2 && a.length < 5) {
        throw new $TypeError(where + "Overload resolution failed.");
      }
      let source;
      let bytes = null;
      let imageDataHeight = 0;
      const t = image !== null && typeof image === "object" ? WeakMapGet(canvasTargets, image) : undefined;
      const data = image !== null && typeof image === "object" ? WeakMapGet(imageDatas, image) : undefined;
      if (t !== undefined && t.kind === 1) {
        source = [3, t.id];
      } else if (t !== undefined && t.kind === 5) {
        if (t.closed) {
          throw new $DOMException(where + "The image source is not usable.", "InvalidStateError");
        }
        source = [4, t.id];
      } else if (data !== undefined) {
        source = [6, data.width];
        imageDataHeight = data.height;
        bytes = ToBase64(new $Uint8Array(TypedArrayBuffer(data.data),
          TypedArrayOffset(data.data), TypedArrayLength(data.data)));
      } else if (isBlob(image)) {
        source = [7, 0];
        bytes = ToBase64(blobBytes(image));
      } else if (isNode(image) && kindOf(image) === 1 &&
        (native.tag(handleOf(image)) === "img" || native.tag(handleOf(image)) === "canvas")) {
        source = [native.tag(handleOf(image)) === "img" ? 1 : 2, handleOf(image)];
      } else {
        throw new $TypeError(where + "The provided value is not of type '(Blob or " +
          "HTMLCanvasElement or HTMLImageElement or HTMLVideoElement or ImageBitmap or ImageData " +
          "or OffscreenCanvas or SVGImageElement or VideoFrame)'.");
      }
      let crop = [0, 0, 0, 0, 0];
      let options = a[1];
      if (a.length >= 5) {
        const m = "createImageBitmap";
        crop = [1, enforcedLong(a[1], m, GLOBAL_INTERFACE), enforcedLong(a[2], m, GLOBAL_INTERFACE),
          enforcedLong(a[3], m, GLOBAL_INTERFACE), enforcedLong(a[4], m, GLOBAL_INTERFACE)];
        if (crop[3] === 0) {
          throw new $RangeError(where + "The crop rect width is 0.");
        }
        if (crop[4] === 0) {
          throw new $RangeError(where + "The crop rect height is 0.");
        }
        options = a[5];
      }
      const option = (name) =>
        options === undefined || options === null || typeof options !== "object" ? undefined
          : options[name];
      const size = (name) => {
        const value = option(name);
        return value === undefined ? -1 : enforcedSize(value, where);
      };
      const rw = size("resizeWidth");
      const rh = size("resizeHeight");
      if (rw === 0 || rh === 0) {
        throw new $DOMException(where + "The options's resizeWidth or resizeHeight is 0.",
          "InvalidStateError");
      }
      const quality = option("resizeQuality");
      const orientation = option("imageOrientation");
      const numbers = [source[0], source[1], crop[0], crop[1], crop[2], crop[3], crop[4], rw, rh,
        quality !== undefined && str(quality) === "pixelated" ? 1 : 0,
        orientation !== undefined && str(orientation) === "flipY" ? 1 : 0, imageDataHeight];
      const args = bytes === null ? ["imageBitmap"] : ["imageBitmap", bytes];
      for (let i = 0; i < numbers.length; i++) {
        ArrayPush(args, numbers[i]);
      }
      let made;
      try {
        made = ReflectApply(bytes === null ? native.canvas : native.canvasData, undefined, args);
      } catch (error) {
        // The reference browser words a Blob it cannot decode without
        // the method.
        if (error instanceof $DOMException && error.message === "The source image could not be decoded.") {
          throw error;
        }
        throw failed(error, "createImageBitmap", GLOBAL_INTERFACE);
      }
      return resolved(makeBitmap(made));
    } catch (error) {
      return rejected(error);
    }
  }

  expose(OffscreenCanvas, "OffscreenCanvas");
  expose(OffscreenCanvasRenderingContext2D, "OffscreenCanvasRenderingContext2D");
  expose(ImageBitmap, "ImageBitmap");
  defineProperty(global, "createImageBitmap", {
    value: createImageBitmap,
    writable: true,
    configurable: true,
    enumerable: true,
  });

  // HTMLCanvasElement's own members (HTML section 4.12.5): its context
  // and its bitmap exported.
  include(
    {
      toDataURL() {
        const handle = self(this);
        let bytes;
        try {
          bytes = native.canvas("encode", 0, handle);
        } catch (error) {
          throw failed(error, "toDataURL", "HTMLCanvasElement");
        }
        if (typeof bytes === "string") {
          return "data:,";
        }
        return "data:image/png;base64," + ToBase64(new $Uint8Array(bytes));
      },

      toBlob(callback) {
        const handle = self(this);
        need(arguments.length, 1, "toBlob", "HTMLCanvasElement");
        if (typeof callback !== "function") {
          throw new $TypeError(prefix("toBlob", "HTMLCanvasElement") +
            "parameter 1 is not of type 'Function'.");
        }
        let bytes;
        try {
          bytes = native.canvas("encode", 0, handle);
        } catch (error) {
          throw failed(error, "toBlob", "HTMLCanvasElement");
        }
        const blob = typeof bytes === "string" ? null : pngBlob(bytes);
        PromiseThen(resolved(undefined), () => callback(blob));
      },

      getContext(contextId) {
        const handle = self(this);
        need(arguments.length, 1, "getContext", "HTMLCanvasElement");
        const id = str(contextId);
        const existing = WeakMapGet(canvasContexts, this);
        if (id !== "2d") {
          return null;
        }
        if (existing !== undefined) {
          return existing;
        }
        const { alpha, willReadFrequently } = contextSettings(arguments[1], "HTMLCanvasElement");
        native.canvas("context", 0, handle, alpha);
        const context = new CanvasRenderingContext2D(KEY);
        WeakMapSet(canvasTargets, context, {
          kind: 0,
          id: handle,
          iface: CTX,
          element: this,
          alpha,
          willReadFrequently,
        });
        WeakMapSet(canvasContexts, this, context);
        return context;
      },
    },
    [MapGet(interfaces, "canvas").prototype],
  );

  // ---- Structured data (HTML section 2.7, M31) ------------------------------
  //
  // StructuredSerializeWithTransfer makes a record of a value, and
  // StructuredDeserializeWithTransfer the value again from it, in this
  // realm (structuredClone, a message to the window itself) or in
  // another, the record then crossing as a string (`wire`). A record is
  // [root, entries]: the root a value's code, and an entry for each
  // object met, which codes name by index, so that cycles and shared
  // buffers come out as they went in. A code is the value itself for a
  // string, a boolean, null and a finite number but -0; [0] for
  // undefined, [1, text] for NaN, the infinities and -0, [2, digits]
  // for a BigInt, [3, index] for an object.
  const BooleanValueOf = uncurry(Boolean.prototype.valueOf);
  const NumberValueOf = uncurry(Number.prototype.valueOf);
  const StringValueOf = uncurry(String.prototype.valueOf);
  const BigIntValueOf = uncurry(BigInt.prototype.valueOf);
  const BigIntToString = uncurry(BigInt.prototype.toString);
  const $BigInt = BigInt;
  const $Boolean = Boolean;
  const $Object = Object;
  const BlobType = uncurry(getOwnPropertyDescriptor(Blob.prototype, "type").get);
  const FileName = uncurry(getOwnPropertyDescriptor(File.prototype, "name").get);
  const FileLastModified = uncurry(getOwnPropertyDescriptor(File.prototype, "lastModified").get);
  const $Date = Date;
  const DateGetTime = uncurry(Date.prototype.getTime);
  const RegExpPrototype = RegExp.prototype;
  const $RegExp = RegExp;
  const RegExpSource = uncurry(getOwnPropertyDescriptor(RegExp.prototype, "source").get);
  const RegExpFlags = uncurry(getOwnPropertyDescriptor(RegExp.prototype, "flags").get);
  const ArrayBufferByteLength = uncurry(
    getOwnPropertyDescriptor(ArrayBuffer.prototype, "byteLength").get);
  const detachedGetter = getOwnPropertyDescriptor(ArrayBuffer.prototype, "detached");
  const ArrayBufferDetached = detachedGetter === undefined ? undefined : uncurry(detachedGetter.get);
  const ArrayBufferIsView = ArrayBuffer.isView;
  const TypedArrayCount = uncurry(getOwnPropertyDescriptor(TYPED_ARRAY, "length").get);
  const $DataView = DataView;
  const DataViewBuffer = uncurry(getOwnPropertyDescriptor(DataView.prototype, "buffer").get);
  const DataViewOffset = uncurry(getOwnPropertyDescriptor(DataView.prototype, "byteOffset").get);
  const DataViewLength = uncurry(getOwnPropertyDescriptor(DataView.prototype, "byteLength").get);
  // A resizable buffer's bound, and whether a view is out of its
  // buffer's bounds: the methods that validate a view throw for one.
  const resizableGetter = getOwnPropertyDescriptor(ArrayBuffer.prototype, "resizable");
  const ArrayBufferResizable = resizableGetter === undefined ? undefined
    : uncurry(resizableGetter.get);
  const maxGetter = getOwnPropertyDescriptor(ArrayBuffer.prototype, "maxByteLength");
  const ArrayBufferMax = maxGetter === undefined ? undefined : uncurry(maxGetter.get);
  const TypedArrayKeys = uncurry(TYPED_ARRAY.keys);
  const MapSize = uncurry(getOwnPropertyDescriptor(Map.prototype, "size").get);
  const SetSize = uncurry(getOwnPropertyDescriptor(Set.prototype, "size").get);
  const SetForEach = uncurry(Set.prototype.forEach);
  const SetAdd = uncurry(Set.prototype.add);
  const $Set = Set;
  const WeakMapHasOf = uncurry(WeakMap.prototype.has);
  const WeakSetHasOf = uncurry(WeakSet.prototype.has);
  const WeakRefDerefOf = uncurry(WeakRef.prototype.deref);
  const FunctionToString = uncurry(Function.prototype.toString);
  const SymbolToString = uncurry(Symbol.prototype.toString);
  const ArrayIsArray = Array.isArray;
  const NumberIsNaN = Number.isNaN;
  const FromBase64 = Uint8Array.fromBase64;
  const domExceptionName = getOwnPropertyDescriptor($DOMException.prototype, "name");
  const DOMExceptionName = domExceptionName !== undefined && domExceptionName.get !== undefined
    ? uncurry(domExceptionName.get) : undefined;
  // The engine keeps an error's stack in an accessor of Error.prototype
  // where the reference browser keeps it on the error itself.
  const errorStack = getOwnPropertyDescriptor(Error.prototype, "stack");
  const ErrorStack = errorStack !== undefined && errorStack.get !== undefined
    ? uncurry(errorStack.get) : undefined;
  // The typed arrays, by the names their tag gives.
  const TYPED_ARRAYS = new $Map();
  {
    const kinds = [Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array,
      Int32Array, Uint32Array, Float32Array, Float64Array, BigInt64Array, BigUint64Array];
    if (typeof Float16Array === "function") {
      ArrayPush(kinds, Float16Array);
    }
    for (let i = 0; i < kinds.length; i++) {
      MapSet(TYPED_ARRAYS, kinds[i].name, kinds[i]);
    }
  }
  // The errors whose names a clone keeps (HTML section 2.7.3).
  const ERROR_TYPES = new $Map([["Error", $Error], ["EvalError", EvalError],
    ["RangeError", $RangeError], ["ReferenceError", ReferenceError],
    ["SyntaxError", SyntaxError], ["TypeError", $TypeError], ["URIError", URIError]]);

  // The kinds of entry.
  const OBJECT = 0;
  const ARRAY = 1;
  const BOOLEAN_OBJECT = 2;
  const NUMBER_OBJECT = 3;
  const STRING_OBJECT = 4;
  const BIGINT_OBJECT = 5;
  const DATE = 6;
  const REGEXP = 7;
  const BUFFER = 8;
  const VIEW = 9;
  const MAP = 10;
  const SET = 11;
  const ERROR_ENTRY = 12;
  const DOM_EXCEPTION = 13;
  const BLOB = 14;
  const FILE = 15;
  const IMAGE_DATA = 16;
  const IMAGE_BITMAP = 17;
  const MATRIX = 18;
  const POINT = 19;
  const PORT = 20;
  const OFFSCREEN_CANVAS = 21;
  const IMAGE_BITMAP_PIXELS = 22;

  // What `native.cloneKind` says of an object: what no script can tell.
  const KIND_FUNCTION = 1;
  const KIND_PROXY = 2;
  const KIND_PROMISE = 3;
  const KIND_ERROR = 4;

  function dataCloneError(where, what) {
    return new $DOMException(where + what, "DataCloneError");
  }

  // The name an object's class goes by in the reference browser's words.
  function className(value) {
    let proto = ReflectGetPrototypeOf(value);
    while (proto !== null) {
      const ctor = ReflectGetOwnPropertyDescriptor(proto, "constructor");
      if (ctor !== undefined && typeof ctor.value === "function" && typeof ctor.value.name === "string" &&
        ctor.value.name !== "") {
        return ctor.value.name;
      }
      proto = ReflectGetPrototypeOf(proto);
    }
    return "Object";
  }

  // The interface of a platform object, the most derived the bindings
  // exposed, or undefined for an object that is none.
  function platformInterface(value) {
    if (isNode(value)) {
      return className(value);
    }
    let proto = ReflectGetPrototypeOf(value);
    while (proto !== null) {
      if (WeakSetHas(platformPrototypes, proto)) {
        const tag = ReflectGetOwnPropertyDescriptor(proto, toStringTag);
        return tag !== undefined && typeof tag.value === "string" ? tag.value : className(value);
      }
      proto = ReflectGetPrototypeOf(proto);
    }
    return undefined;
  }

  // Whether `value` passes a brand check that throws for what fails it.
  function branded(check, value) {
    try {
      check(value);
      return true;
    } catch {
      return false;
    }
  }

  function numberCode(n) {
    if (n !== n) {
      return [1, "NaN"];
    }
    if (n === Infinity) {
      return [1, "Infinity"];
    }
    if (n === -Infinity) {
      return [1, "-Infinity"];
    }
    if (n === 0 && 1 / n < 0) {
      return [1, "-0"];
    }
    return n;
  }

  function codeNumber(code) {
    if (typeof code === "number") {
      return code;
    }
    switch (code[1]) {
      case "NaN":
        return NaN;
      case "Infinity":
        return Infinity;
      case "-Infinity":
        return -Infinity;
      default:
        return -0;
    }
  }

  // StructuredSerializeWithTransfer: `value` serialized with the
  // objects of `transfer` (already a list) transferred, its refusals
  // worded after `where`. The buffers transferred are detached once the
  // value is serialized, as the standard says.
  function serialize(value, transfer, where, source) {
    const memory = new $Map();
    const entries = [];
    // Each index and object of the list, buffers apart.
    const buffers = [];
    const others = [];
    // The list first: what can be transferred, and nothing twice. What
    // went before (detached, neutered) is found once the value is
    // serialized, which may make it so, as the standard orders it.
    for (let i = 0; i < transfer.length; i++) {
      const t = transfer[i];
      if (MapHas(memory, t)) {
        const what = branded(ArrayBufferByteLength, t) ? "ArrayBuffer" : "Message port";
        throw dataCloneError(where, what + " at index " + i + " is a duplicate of an earlier " +
          (what === "ArrayBuffer" ? "ArrayBuffer." : "port."));
      }
      if (branded(ArrayBufferByteLength, t)) {
        ArrayPush(buffers, i, t);
      } else if (transferable(t, i, where, source)) {
        ArrayPush(others, i, t);
      } else {
        throw dataCloneError(where, "Value at index " + i + " does not have a transferable type.");
      }
      MapSet(memory, t, entries.length);
      ArrayPush(entries, null);
    }
    const root = codeOf(value, memory, entries, where);
    for (let k = 0; k < buffers.length; k += 2) {
      if (isDetached(buffers[k + 1])) {
        throw dataCloneError(where, "ArrayBuffer at index " + buffers[k] + " is already detached.");
      }
    }
    for (let k = 0; k < others.length; k += 2) {
      entries[MapGet(memory, others[k + 1])] = transferEntry(others[k + 1], others[k], where);
    }
    // The transferred buffers' bytes, then the buffers detached.
    for (let k = 0; k < buffers.length; k += 2) {
      const at = MapGet(memory, buffers[k + 1]);
      entries[at] = [BUFFER, TypedArraySlice(new $Uint8Array(buffers[k + 1])),
        bufferMax(buffers[k + 1])];
    }
    for (let k = 0; k < buffers.length; k += 2) {
      native.wasmDetach(buffers[k + 1]);
    }
    transferred(entries);
    return [root, entries];
  }

  // Whether what is not a buffer can be transferred (its type, and not
  // the port posting), and its entry once the value is serialized, or
  // the refusal of one that went: ports (M31.5) and offscreen canvases
  // (M31.4) define them, the steps that make them.
  let transferable = function () {
    return false;
  };
  let transferEntry = function () {
    return undefined;
  };
  // What a transfer does once the record is made: the steps that
  // define transferable objects hook in.
  let transferred = function () {};

  // A buffer's `maxByteLength` when it is resizable, else null: what
  // its entry keeps beside its bytes.
  function bufferMax(buffer) {
    if (ArrayBufferResizable === undefined || !ArrayBufferResizable(buffer)) {
      return null;
    }
    return ArrayBufferMax(buffer);
  }

  // Whether a view is out of its buffer's bounds, its buffer detached or
  // shrunk under it (ECMAScript's IsArrayBufferViewOutOfBounds).
  function outOfBounds(view, typed) {
    try {
      if (typed) {
        TypedArrayKeys(view);
      } else {
        DataViewLength(view);
      }
      return false;
    } catch {
      return true;
    }
  }

  function isDetached(buffer) {
    if (ArrayBufferDetached !== undefined) {
      return ArrayBufferDetached(buffer);
    }
    return false;
  }

  // The code of `value`, its object entered in `entries` if it is not
  // there yet.
  function codeOf(value, memory, entries, where) {
    switch (typeof value) {
      case "undefined":
        return [0];
      case "boolean":
      case "string":
        return value;
      case "number":
        return numberCode(value);
      case "bigint":
        return [2, BigIntToString(value)];
      case "symbol":
        throw dataCloneError(where, SymbolToString(value) + " could not be cloned.");
      default:
        break;
    }
    if (value === null) {
      return null;
    }
    const known = MapGet(memory, value);
    if (known !== undefined) {
      return [3, known];
    }
    const at = entries.length;
    MapSet(memory, value, at);
    ArrayPush(entries, null);
    entries[at] = entryOf(value, memory, entries, where);
    return [3, at];
  }

  // The entry of object `value`, its own codes made as the standard's
  // steps reach them.
  function entryOf(value, memory, entries, where) {
    if (typeof value === "function") {
      throw dataCloneError(where, FunctionToString(value) + " could not be cloned.");
    }
    if (value === global) {
      throw dataCloneError(where, "#<" + GLOBAL_CLASS + "> could not be cloned.");
    }
    const kind = native.cloneKind(value);
    if (kind === KIND_PROXY) {
      throw dataCloneError(where, "#<Object> could not be cloned.");
    }
    if (kind === KIND_PROMISE) {
      throw dataCloneError(where, "#<" + className(value) + "> could not be cloned.");
    }
    if (branded(BooleanValueOf, value)) {
      return [BOOLEAN_OBJECT, BooleanValueOf(value)];
    }
    if (branded(NumberValueOf, value)) {
      return [NUMBER_OBJECT, numberCode(NumberValueOf(value))];
    }
    if (branded(BigIntValueOf, value)) {
      return [BIGINT_OBJECT, BigIntToString(BigIntValueOf(value))];
    }
    if (branded(StringValueOf, value)) {
      return [STRING_OBJECT, StringValueOf(value)];
    }
    if (branded(DateGetTime, value)) {
      return [DATE, numberCode(DateGetTime(value))];
    }
    if (value !== RegExpPrototype && branded(RegExpSource, value)) {
      return [REGEXP, RegExpSource(value), RegExpFlags(value)];
    }
    if (branded(ArrayBufferByteLength, value)) {
      if (isDetached(value)) {
        throw dataCloneError(where, "An ArrayBuffer is detached and could not be cloned.");
      }
      return [BUFFER, TypedArraySlice(new $Uint8Array(value)), bufferMax(value)];
    }
    if (ArrayBufferIsView(value)) {
      const tag = TypedArrayTag(value);
      if (outOfBounds(value, tag !== undefined)) {
        throw dataCloneError(where, "An ArrayBuffer is detached and could not be cloned.");
      }
      if (tag !== undefined) {
        const buffer = codeOf(TypedArrayBuffer(value), memory, entries, where);
        return [VIEW, tag, buffer[1], TypedArrayOffset(value), TypedArrayCount(value)];
      }
      const buffer = codeOf(DataViewBuffer(value), memory, entries, where);
      return [VIEW, "DataView", buffer[1], DataViewOffset(value), DataViewLength(value)];
    }
    if (branded(MapSize, value)) {
      const pairs = [];
      MapForEach(value, (v, k) => {
        ArrayPush(pairs, k, v);
      });
      const codes = [];
      for (let i = 0; i < pairs.length; i++) {
        ArrayPush(codes, codeOf(pairs[i], memory, entries, where));
      }
      return [MAP, codes];
    }
    if (branded(SetSize, value)) {
      const items = [];
      SetForEach(value, (v) => {
        ArrayPush(items, v);
      });
      const codes = [];
      for (let i = 0; i < items.length; i++) {
        ArrayPush(codes, codeOf(items[i], memory, entries, where));
      }
      return [SET, codes];
    }
    if (DOMExceptionName !== undefined && branded(DOMExceptionName, value)) {
      return [DOM_EXCEPTION, DOMExceptionName(value), value.message];
    }
    if (kind === KIND_ERROR) {
      return errorEntry(value, memory, entries, where);
    }
    if (ArrayIsArray(value)) {
      const length = value.length;
      return [ARRAY, length, ownCodes(value, memory, entries, where)];
    }
    const platform = platformInterface(value);
    if (platform !== undefined) {
      return platformEntry(value, platform, where);
    }
    if (branded((v) => WeakMapHasOf(v, value), value) || branded((v) => WeakSetHasOf(v, value), value) ||
      branded(WeakRefDerefOf, value)) {
      throw dataCloneError(where, "#<" + className(value) + "> could not be cloned.");
    }
    return [OBJECT, ownCodes(value, memory, entries, where)];
  }

  // The codes of an object's own enumerable string-keyed properties,
  // name then value, read in the order of its keys.
  function ownCodes(value, memory, entries, where) {
    const keys = ReflectOwnKeys(value);
    const codes = [];
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      if (typeof key !== "string") {
        continue;
      }
      const desc = ReflectGetOwnPropertyDescriptor(value, key);
      if (desc === undefined || !desc.enumerable) {
        continue;
      }
      const v = value[key];
      ArrayPush(codes, key, codeOf(v, memory, entries, where));
    }
    return codes;
  }

  // An error's entry: its name if the standard keeps it, its own
  // message, its own cause, and its stack, which the reference browser
  // keeps too.
  function errorEntry(value, memory, entries, where) {
    let name = value.name;
    name = typeof name === "string" && MapHas(ERROR_TYPES, name) ? name : "Error";
    const messageDesc = ReflectGetOwnPropertyDescriptor(value, "message");
    const message = messageDesc !== undefined && "value" in messageDesc ? str(messageDesc.value) : [0];
    const causeDesc = ReflectGetOwnPropertyDescriptor(value, "cause");
    const hasCause = causeDesc !== undefined && "value" in causeDesc;
    const cause = hasCause ? codeOf(causeDesc.value, memory, entries, where) : null;
    const stackDesc = ReflectGetOwnPropertyDescriptor(value, "stack");
    let stack = stackDesc !== undefined && typeof stackDesc.value === "string" ? stackDesc.value : null;
    if (stackDesc === undefined && ErrorStack !== undefined) {
      const own = ErrorStack(value);
      stack = typeof own === "string" ? own : null;
    }
    return [ERROR_ENTRY, name, message, hasCause, cause, stack];
  }

  // The entry of a platform object: a serializable one's state, or the
  // refusal the reference browser words.
  function platformEntry(value, iface, where) {
    if (isPort(value)) {
      throw dataCloneError(where, "A MessagePort could not be cloned because it was not transferred.");
    }
    const canvasTarget = WeakMapGet(canvasTargets, value);
    if (canvasTarget !== undefined && canvasTarget.kind === 1) {
      throw dataCloneError(where,
        "An OffscreenCanvas could not be cloned because it was not transferred.");
    }
    if (isBlob(value)) {
      if (isFile(value)) {
        return [FILE, TypedArraySlice(blobBytes(value)), BlobType(value), FileName(value),
          FileLastModified(value)];
      }
      return [BLOB, TypedArraySlice(blobBytes(value)), BlobType(value)];
    }
    const data = WeakMapGet(imageDatas, value);
    if (data !== undefined) {
      return [IMAGE_DATA, data.width, data.height,
        TypedArraySlice(new $Uint8Array(TypedArrayBuffer(data.data), TypedArrayOffset(data.data),
          TypedArrayLength(data.data))), data.colorSpace];
    }
    const m = WeakMapGet(matrices, value);
    if (m !== undefined) {
      return [MATRIX, iface, m.is2D, ArraySlice(m.s, 0)];
    }
    const p = WeakMapGet(points, value);
    if (p !== undefined) {
      return [POINT, iface, ArraySlice(p, 0)];
    }
    const t = WeakMapGet(canvasTargets, value);
    if (t !== undefined && t.kind === 5) {
      if (t.closed) {
        throw dataCloneError(where, "An ImageBitmap is detached and could not be cloned.");
      }
      return [IMAGE_BITMAP, bitmapCopy(t)];
    }
    throw dataCloneError(where, iface + " object could not be cloned.");
  }

  // A copy of an ImageBitmap's pixels for a clone, kept as the host's
  // [number, width, height]: here a new bitmap of this realm's.
  let bitmapCopy = function (t) {
    return native.canvas("imageBitmap", 4, t.id, 0, 0, 0, 0, 0, -1, -1, 0, 0, 0);
  };

  // StructuredDeserializeWithTransfer: the value of a record.
  function deserialize(record) {
    return deserializeWithPorts(record)[0];
  }

  // The value of a record and the ports transferred with it, in the
  // order of its transfer list (M31.5): a message event's `ports`.
  function deserializeWithPorts(record) {
    const entries = record[1];
    const made = [];
    made.length = entries.length;
    const ports = [];
    for (let i = 0; i < entries.length; i++) {
      const e = entries[i];
      if (e !== null && typeof e === "object" && e[0] === PORT) {
        ArrayPush(ports, objectAt(i, entries, made));
      }
    }
    return [valueOf(record[0], entries, made), ports];
  }

  function valueOf(code, entries, made) {
    if (typeof code !== "object" || code === null) {
      return code;
    }
    switch (code[0]) {
      case 0:
        return undefined;
      case 1:
        return codeNumber(code);
      case 2:
        return $BigInt(code[1]);
      default:
        return objectAt(code[1], entries, made);
    }
  }

  // The object of entry `at`, made once: its shell first, then what it
  // holds, so that what refers back to it finds it.
  function objectAt(at, entries, made) {
    if (made[at] !== undefined) {
      return made[at];
    }
    const e = entries[at];
    let object;
    switch (e[0]) {
      case OBJECT:
        object = {};
        made[at] = object;
        fillOwn(object, e[1], entries, made);
        return object;
      case ARRAY:
        object = [];
        object.length = e[1];
        made[at] = object;
        fillOwn(object, e[2], entries, made);
        return object;
      case BOOLEAN_OBJECT:
        object = new $Boolean(e[1]);
        break;
      case NUMBER_OBJECT:
        object = new $Number(codeNumber(e[1]));
        break;
      case STRING_OBJECT:
        object = new $String(e[1]);
        break;
      case BIGINT_OBJECT:
        object = $Object($BigInt(e[1]));
        break;
      case DATE:
        object = new $Date(codeNumber(e[1]));
        break;
      case REGEXP:
        object = new $RegExp(e[1], e[2]);
        break;
      case BUFFER: {
        object = e[2] === null || e[2] === undefined ? new $ArrayBuffer(TypedArrayLength(e[1]))
          : new $ArrayBuffer(TypedArrayLength(e[1]), { maxByteLength: e[2] });
        TypedArraySet(new $Uint8Array(object), e[1]);
        break;
      }
      case VIEW: {
        const buffer = objectAt(e[2], entries, made);
        if (e[1] === "DataView") {
          object = new $DataView(buffer, e[3], e[4]);
        } else {
          const cls = MapGet(TYPED_ARRAYS, e[1]);
          object = new cls(buffer, e[3], e[4]);
        }
        break;
      }
      case MAP: {
        object = new $Map();
        made[at] = object;
        const codes = e[1];
        for (let i = 0; i < codes.length; i += 2) {
          MapSet(object, valueOf(codes[i], entries, made), valueOf(codes[i + 1], entries, made));
        }
        return object;
      }
      case SET: {
        object = new $Set();
        made[at] = object;
        const codes = e[1];
        for (let i = 0; i < codes.length; i++) {
          SetAdd(object, valueOf(codes[i], entries, made));
        }
        return object;
      }
      case ERROR_ENTRY: {
        const cls = MapGet(ERROR_TYPES, e[1]);
        object = new cls();
        made[at] = object;
        if (typeof e[2] === "string") {
          defineProperty(object, "message", { value: e[2], writable: true, configurable: true, enumerable: false });
        }
        if (e[3]) {
          defineProperty(object, "cause", {
            value: valueOf(e[4], entries, made), writable: true, configurable: true, enumerable: false,
          });
        }
        if (typeof e[5] === "string") {
          defineProperty(object, "stack", { value: e[5], writable: true, configurable: true, enumerable: false });
        }
        return object;
      }
      case DOM_EXCEPTION:
        // Made again without the stack its construction gave it, as
        // the reference browser makes it.
        object = new $DOMException(e[2], e[1]);
        ReflectDeleteProperty(object, "stack");
        break;
      case BLOB:
        object = new Blob(KEY, { bytes: e[1], type: e[2] });
        break;
      case FILE:
        object = new File(KEY, { bytes: e[1], type: e[2], name: e[3], lastModified: e[4] });
        break;
      case IMAGE_DATA: {
        object = new ImageData(KEY);
        const bytes = new $Uint8ClampedArray(TypedArrayLength(e[3]));
        TypedArraySet(bytes, e[3]);
        WeakMapSet(imageDatas, object, { data: bytes, width: e[1], height: e[2], colorSpace: e[4] });
        break;
      }
      case IMAGE_BITMAP:
        object = bitmapMade(e[1]);
        break;
      case IMAGE_BITMAP_PIXELS:
        object = makeBitmap(native.canvasData("bitmapFrom", e[3], e[1], e[2], e[4] ? 1 : 0));
        break;
      case MATRIX:
        object = makeMatrix(e[1] === "DOMMatrix" ? DOMMatrix : DOMMatrixReadOnly,
          { s: ArraySlice(e[3], 0), is2D: e[2] });
        break;
      case POINT:
        object = e[1] === "DOMPoint" ? new DOMPoint(e[2][0], e[2][1], e[2][2], e[2][3])
          : new DOMPointReadOnly(e[2][0], e[2][1], e[2][2], e[2][3]);
        break;
      default:
        object = transferMade(e);
        break;
    }
    made[at] = object;
    return object;
  }

  // The ImageBitmap of a clone's copy; the transferred objects' made by
  // the steps that define them.
  let bitmapMade = function (made) {
    return makeBitmap(made);
  };
  let transferMade = function () {
    throw new $TypeError("The message holds what this realm cannot make.");
  };

  function fillOwn(object, codes, entries, made) {
    for (let i = 0; i < codes.length; i += 2) {
      defineProperty(object, codes[i], {
        value: valueOf(codes[i + 1], entries, made),
        writable: true,
        enumerable: true,
        configurable: true,
      });
    }
  }

  // The kinds of entry whose bytes cross as base64, and where they are.
  const BYTES_AT = new $Map([[BUFFER, 1], [BLOB, 1], [FILE, 1], [IMAGE_DATA, 3]]);

  // A record as the string another realm takes: its JSON, bytes in
  // base64.
  function wire(record) {
    const entries = record[1];
    const out = [];
    for (let i = 0; i < entries.length; i++) {
      const e = entries[i];
      if (e !== null && e[0] === IMAGE_BITMAP) {
        // A bitmap of this realm's host crosses as its pixels (M31).
        const id = e[1][0];
        const bytes = new $Uint8Array(native.canvas("bitmapBytes", 4, id));
        const clean = native.canvas("bitmapClean", 4, id);
        native.canvas("release", 5, id);
        ArrayPush(out, [IMAGE_BITMAP_PIXELS, e[1][1], e[1][2], ToBase64(bytes), clean]);
        continue;
      }
      const at = MapGet(BYTES_AT, e[0]);
      if (at === undefined) {
        ArrayPush(out, e);
        continue;
      }
      const copy = ArraySlice(e, 0);
      copy[at] = ToBase64(e[at]);
      ArrayPush(out, copy);
    }
    return JSONStringify([record[0], out]);
  }

  // The record of a string `wire` made.
  function unwire(text) {
    const record = JSONParse(text);
    const entries = record[1];
    for (let i = 0; i < entries.length; i++) {
      const e = entries[i];
      const at = MapGet(BYTES_AT, e[0]);
      if (at !== undefined) {
        e[at] = FromBase64(e[at]);
      }
    }
    return record;
  }

  // The transfer list of `options`' `transfer` member (a dictionary's,
  // StructuredSerializeOptions and the post messages' options), or a
  // `transfer` argument.
  function transferList(value, method, iface, dictionaryName) {
    const where = prefix(method, iface);
    if (value === undefined) {
      return [];
    }
    return sequence(value, method, iface, (item) => {
      if ((typeof item !== "object" || item === null) && typeof item !== "function") {
        throw new $TypeError(where + (dictionaryName === undefined ? ""
          : "Failed to read the 'transfer' property from '" + dictionaryName + "': ") +
          "Failed to convert value to 'object'.");
      }
      return item;
    });
  }

  function serializeOptions(options, method, iface, dictionaryName) {
    if (options === undefined || options === null) {
      return [];
    }
    if (typeof options !== "object" && typeof options !== "function") {
      throw new $TypeError(prefix(method, iface) + "The provided value is not of type '" +
        dictionaryName + "'.");
    }
    return transferList(options.transfer, method, iface, dictionaryName);
  }

  // structuredClone() (HTML section 2.7.6).
  defineProperty(global, "structuredClone", {
    value: function structuredClone(value) {
      const where = prefix("structuredClone", GLOBAL_INTERFACE);
      if (arguments.length < 1) {
        throw new $TypeError(where + "1 argument required, but only 0 present.");
      }
      const transfer = serializeOptions(arguments[1], "structuredClone", GLOBAL_INTERFACE,
        "StructuredSerializeOptions");
      return deserialize(serialize(value, transfer, where, null));
    },
    writable: true,
    configurable: true,
    enumerable: true,
  });

  // MessageEvent (HTML section 9.3, M31): a message's data, its origin,
  // its last event ID, its source and the ports sent with it, frozen.
  const MESSAGE = 65536;
  const MESSAGE_INIT = members(EVENT_INIT, [
    ["data", "any", null],
    ["lastEventId", "DOMString", ""],
    ["origin", "USVString", ""],
    ["ports", "sequence<MessagePort>", []],
    ["source", "MessageEventSource?", null],
  ], "MessageEventInit");

  class MessageEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, MESSAGE_INIT, "MessageEvent");
      super(args[0], args[1]);
      const s = eventState(this);
      s.kinds |= MESSAGE;
      s.fields.ports = ObjectFreeze(ArraySlice(s.fields.ports, 0));
    }

    get data() {
      return fieldsOf(this, MESSAGE).data;
    }

    get origin() {
      return fieldsOf(this, MESSAGE).origin;
    }

    get lastEventId() {
      return fieldsOf(this, MESSAGE).lastEventId;
    }

    get source() {
      return fieldsOf(this, MESSAGE).source;
    }

    get ports() {
      return fieldsOf(this, MESSAGE).ports;
    }

    initMessageEvent(type, bubbles = false, cancelable = false, data = null, origin = "",
      lastEventId = "", source = null, ports = []) {
      const s = eventOf(this, MESSAGE);
      if (arguments.length < 1) {
        throw new $TypeError(prefix("initMessageEvent", "MessageEvent") +
          "1 argument required, but only 0 present.");
      }
      if (s.dispatching) {
        return;
      }
      const list = MESSAGE_INIT;
      const fields = s.fields;
      fields.data = data;
      fields.origin = str(origin);
      fields.lastEventId = str(lastEventId);
      fields.source = member(source, list[list.length - 1], "MessageEvent");
      fields.ports = ObjectFreeze(member(ports, list[list.length - 2], "MessageEvent"));
      initialize(s, str(type), !!bubbles, !!cancelable);
    }
  }
  ArrayPush(EVENT_INTERFACES, ["MessageEvent", MessageEvent, MESSAGE_INIT]);
  expose(MessageEvent, "MessageEvent");
  MapSet(eventInterfaces, "MessageEvent", EVENT_INTERFACES[EVENT_INTERFACES.length - 1]);

  // Whether `value` is a MessagePort, which M31.5's ports define.
  let isPort = function () {
    return false;
  };

  // The message event a message's arrival fires at `target`: its data
  // made again, or, when it cannot be, a `messageerror` (HTML section
  // 9.5.3); `origin` the sender's, `source` the window for a message
  // to the window.
  function messageEvent(target, text, origin, source) {
    let data = null;
    let ports = [];
    let type = "message";
    try {
      const both = deserializeWithPorts(unwire(text));
      data = both[0];
      ports = both[1];
    } catch {
      type = "messageerror";
    }
    const event = new MessageEvent(type, made(MESSAGE_INIT, [
      "data", data,
      "origin", origin,
      "source", source !== undefined ? source : target === global && !WORKER ? global : null,
      "ports", ports,
    ]));
    eventState(event).trusted = true;
    return dispatching(target, event);
  }

  // window.postMessage (HTML section 9.4.3, M31): to this window, the
  // only one there is until frames (M33.2). `targetOrigin` "*" for any,
  // "/" for the window's own, else a URL whose origin it must be; the
  // message dropped when it is not.
  defineProperty(Window.prototype, "postMessage", {
    value: function postMessage(message) {
      const posted = postArguments(arguments, "postMessage", "Window");
      native.postMessage(posted[0], posted[1]);
    },
    writable: true,
    configurable: true,
    enumerable: true,
  });

  // A window's `postMessage`'s arguments (HTML section 9.4.3): the target
  // origin serialized, and the message's record wired; this window's
  // and another frame's (M33.2).
  function postArguments(a, method, iface) {
    const where = prefix(method, iface);
    if (a.length < 1) {
      throw new $TypeError(where + "1 argument required, but only 0 present.");
    }
    let targetOrigin;
    let transfer;
    const second = a[1];
    if (a.length >= 3 || (a.length === 2 && second !== undefined && second !== null &&
      typeof second !== "object" && typeof second !== "function")) {
      targetOrigin = str(second);
      transfer = transferList(a[2], "postMessage", "Window", undefined);
    } else {
      if (second !== undefined && second !== null && typeof second !== "object" &&
        typeof second !== "function") {
        throw new $TypeError(where + "The provided value is not of type 'WindowPostMessageOptions'.");
      }
      transfer = serializeOptions(second, "postMessage", "Window", "WindowPostMessageOptions");
      const given = second === undefined || second === null ? undefined : second.targetOrigin;
      targetOrigin = given === undefined ? "/" : str(given);
    }
    let origin;
    try {
      origin = native.targetOrigin(targetOrigin);
    } catch (error) {
      throw failed(error, method, iface);
    }
    const record = serialize(a[0], transfer, where, global);
    return [origin, wire(record)];
  }

  // ---- Dedicated workers (M31, HTML section 10.2.6) ----------------------

  // The workers this realm made and has not seen end, by number: each
  // is kept while it may still hear from its worker.
  const workerObjects = new $Map();
  let workerNumber;

  // A message's transfer list from the argument after it: a sequence of
  // objects, or StructuredSerializeOptions (Web IDL's overload
  // resolution of postMessage(message, transfer) and
  // postMessage(message, options)).
  function postTransfer(second, method, iface) {
    const object = (typeof second === "object" && second !== null) || typeof second === "function";
    if (object && second[iteratorSymbol] !== undefined) {
      return transferList(second, method, iface, undefined);
    }
    return serializeOptions(second, method, iface, "StructuredSerializeOptions");
  }

  // One of an enumeration's values, or the reference browser's error.
  function workerEnum(value, values, member, type) {
    const text = str(value);
    for (let i = 0; i < values.length; i++) {
      if (values[i] === text) {
        return text;
      }
    }
    throw new $TypeError("Failed to construct 'Worker': Failed to read the '" + member +
      "' property from 'WorkerOptions': The provided value '" + text +
      "' is not a valid enum value of type " + type + ".");
  }

  // WorkerOptions (HTML section 10.2.6.3), its members read in Web
  // IDL's order: [credentials, name, type].
  function workerOptions(options) {
    if (options !== undefined && options !== null && typeof options !== "object" &&
      typeof options !== "function") {
      throw new $TypeError(
        "Failed to construct 'Worker': The provided value is not of type 'WorkerOptions'.");
    }
    const given = options === undefined || options === null ? null : options;
    const credentials = given === null ? undefined : given.credentials;
    const name = given === null ? undefined : given.name;
    const type = given === null ? undefined : given.type;
    return [
      credentials === undefined ? "same-origin"
        : workerEnum(credentials, ["omit", "same-origin", "include"], "credentials",
          "RequestCredentials"),
      name === undefined ? "" : str(name),
      type === undefined ? "classic" : workerEnum(type, ["classic", "module"], "type", "WorkerType"),
    ];
  }

  class Worker extends EventTarget {
    #id = 0;

    static {
      workerNumber = (worker) => {
        if (typeof worker === "object" && worker !== null && #id in worker) {
          return worker.#id;
        }
        throw illegalInvocation();
      };
    }

    constructor(scriptURL, options = undefined) {
      if (arguments.length < 1) {
        throw new $TypeError(
          "Failed to construct 'Worker': 1 argument required, but only 0 present.");
      }
      super();
      const url = str(scriptURL);
      const o = workerOptions(options);
      let id;
      try {
        id = native.workerStart(url, o[1], o[2] === "module", o[0]);
      } catch (error) {
        if (error instanceof $DOMException) {
          throw new $DOMException("Failed to construct 'Worker': " + error.message, error.name);
        }
        throw error;
      }
      this.#id = id;
      MapSet(workerObjects, id, this);
    }

    postMessage(message) {
      const id = workerNumber(this);
      const a = arguments;
      need(a.length, 1, "postMessage", "Worker");
      const transfer = postTransfer(a[1], "postMessage", "Worker");
      const record = serialize(message, transfer, prefix("postMessage", "Worker"), null);
      try {
        native.workerPost(id, wire(record));
      } catch (error) {
        throw failed(error, "postMessage", "Worker");
      }
    }

    terminate() {
      const id = workerNumber(this);
      native.workerTerminate(id);
      MapDelete(workerObjects, id);
    }
  }
  // The reference browser's Worker has no onmessageerror.
  defineHandlers(Worker.prototype, ["error", "message"], false);
  expose(Worker, "Worker");

  // A dedicated worker's own members (HTML section 10.2.1.2): a message
  // to its owner, and close().
  if (WORKER) {
    defineProperty(global, "postMessage", {
      value: function postMessage(message) {
        const a = arguments;
        const iface = "DedicatedWorkerGlobalScope";
        need(a.length, 1, "postMessage", iface);
        const transfer = postTransfer(a[1], "postMessage", iface);
        const record = serialize(message, transfer, prefix("postMessage", iface), null);
        try {
          native.postToOwner(wire(record));
        } catch (error) {
          throw failed(error, "postMessage", iface);
        }
      },
      writable: true,
      enumerable: true,
      configurable: true,
    });
    defineProperty(global, "close", {
      value: function close() {
        native.closeWorker();
      },
      writable: true,
      enumerable: true,
      configurable: true,
    });
  }

  // ---- The console (Console Standard; M36) ---------------------------------

  // The console namespace: each method's arguments formatted by the
  // natives into the text the shell prints, as before, and each given a
  // preview for the DevTools panel (docs/design/devtools.md, "The
  // console"): a snapshot taken when it was logged, as JSON, objects to
  // PREVIEW_DEPTH levels and PREVIEW_PROPERTIES own properties each, no
  // getter called, at most PREVIEW_NODES values in all.
  const consoleMessage = native.consoleMessage;
  const PREVIEW_DEPTH = 3;
  const PREVIEW_PROPERTIES = 100;
  const PREVIEW_NODES = 1000;
  const PREVIEW_STRING = 10000;
  const CONSOLE_LOG = 0;
  const CONSOLE_INFO = 1;
  const CONSOLE_WARN = 2;
  const CONSOLE_ERROR = 3;
  const CONSOLE_DEBUG = 4;
  const $ErrorIsError = Error.isError;
  // An error's stack is an accessor of Error.prototype's in QuickJS-ng.
  const ErrorStackGet = getOwnPropertyDescriptor(Error.prototype, "stack").get;
  const RegExpSourceGet = getOwnPropertyDescriptor(RegExp.prototype, "source").get;
  const $RegExpPrototype = RegExp.prototype;
  const $ArrayIsArray = Array.isArray;
  const $DateGetTime = uncurry(Date.prototype.getTime);
  const $DateToISOString = uncurry(Date.prototype.toISOString);
  const $RegExpToString = uncurry(RegExp.prototype.toString);
  const $FunctionToString = uncurry(Function.prototype.toString);
  const $SymbolToString = uncurry(Symbol.prototype.toString);
  const $BigIntToString = uncurry(BigInt.prototype.toString);
  const $MapSize = uncurry(getOwnPropertyDescriptor(Map.prototype, "size").get);
  const $SetSize = uncurry(getOwnPropertyDescriptor(Set.prototype, "size").get);
  const $SetForEach = uncurry(Set.prototype.forEach);
  const $MapForEach = uncurry(Map.prototype.forEach);
  const $ObjectIs = Object.is;
  const ObjectHasOwn = Object.hasOwn;
  const $NumberToString = uncurry(Number.prototype.toString);
  const $NumberIsFinite = Number.isFinite;

  // `text` as a JSON string.
  function quote(text) {
    let out = '"';
    for (let i = 0; i < text.length; i++) {
      const c = CharCodeAt(text, i);
      if (c === 0x22) {
        out += '\\"';
      } else if (c === 0x5c) {
        out += "\\\\";
      } else if (c < 0x20 || c === 0x2028 || c === 0x2029) {
        out += "\\u" + StringSlice("000" + $NumberToString(c, 16), -4);
      } else {
        out += text[i];
      }
    }
    return out + '"';
  }

  // The class an object is of, as the reference browser names it: its
  // constructor's own name where its prototype has one, else its tag.
  function classOf(value) {
    let proto;
    try {
      proto = ReflectGetPrototypeOf(value);
    } catch (e) {
      return "Object";
    }
    if (proto === null) {
      return "Object";
    }
    const desc = ReflectGetOwnPropertyDescriptor(proto, "constructor");
    if (desc !== undefined && ObjectHasOwn(desc, "value") && typeof desc.value === "function") {
      const name = ReflectGetOwnPropertyDescriptor(desc.value, "name");
      if (name !== undefined && typeof name.value === "string" && name.value !== "") {
        return name.value;
      }
    }
    return "Object";
  }

  // An error's stack, as the original getter reads it.
  function stackOf(error) {
    try {
      const stack = ReflectApply(ErrorStackGet, error, []);
      return typeof stack === "string" ? stack : "";
    } catch (e) {
      return "";
    }
  }

  // The preview of `value` as JSON, `depth` levels of its properties
  // more to go, `state` the values made so far and the objects on the
  // way down.
  function preview(value, depth, state) {
    state.nodes++;
    switch (typeof value) {
      case "undefined":
        return '{"type":"undefined"}';
      case "boolean":
        return '{"type":"boolean","value":' + (value ? "true" : "false") + "}";
      case "number":
        if ($NumberIsFinite(value) && !$ObjectIs(value, -0)) {
          return '{"type":"number","value":' + $String(value) + "}";
        }
        return '{"type":"number","description":' + quote($ObjectIs(value, -0) ? "-0" : $String(value)) + "}";
      case "string":
        return '{"type":"string","value":' + quote(StringSlice(value, 0, PREVIEW_STRING)) +
          (value.length > PREVIEW_STRING ? ',"more":true' : "") + "}";
      case "bigint":
        return '{"type":"bigint","description":' + quote($BigIntToString(value) + "n") + "}";
      case "symbol":
        return '{"type":"symbol","description":' + quote($SymbolToString(value)) + "}";
      case "function": {
        const name = ReflectGetOwnPropertyDescriptor(value, "name");
        return '{"type":"function","name":' +
          quote(name !== undefined && typeof name.value === "string" ? name.value : "") + "}";
      }
    }
    if (value === null) {
      return '{"type":"null"}';
    }
    if (isNode(value)) {
      return nodePreview(value);
    }
    for (let i = 0; i < state.path.length; i++) {
      if (state.path[i] === value) {
        return '{"type":"object","class":' + quote(classOf(value)) + ',"circular":true}';
      }
    }
    const cls = classOf(value);
    if ($ErrorIsError(value)) {
      return errorPreview(value, cls);
    }
    if (branded($DateGetTime, value)) {
      const time = $DateGetTime(value);
      return '{"type":"date","description":' +
        quote(time === time ? $DateToISOString(value) : "Invalid Date") + "}";
    }
    if (branded((v) => ReflectApply(RegExpSourceGet, v, []), value) &&
        value !== $RegExpPrototype) {
      return '{"type":"regexp","description":' + quote($RegExpToString(value)) + "}";
    }
    if (depth === 0 || state.nodes >= PREVIEW_NODES) {
      return '{"type":"object","class":' + quote(cls) + ',"collapsed":true}';
    }
    ArrayPush(state.path, value);
    let out;
    const map = branded($MapSize, value);
    if (map || branded($SetSize, value)) {
      out = entriesPreview(value, map, cls, depth, state);
    } else {
      out = propertiesPreview(value, cls, depth, state);
    }
    state.path.length--;
    return out;
  }

  // An object's own properties, or an array's items and own named
  // properties: an accessor's value is not read.
  function propertiesPreview(value, cls, depth, state) {
    const array = $ArrayIsArray(value);
    let keys;
    try {
      keys = ReflectOwnKeys(value);
    } catch (e) {
      return '{"type":"object","class":' + quote(cls) + ',"collapsed":true}';
    }
    let out = array
      ? '{"type":"array","class":' + quote(cls) + ',"length":' + $String(value.length) + ',"properties":['
      : '{"type":"object","class":' + quote(cls) + ',"properties":[';
    let shown = 0;
    let more = false;
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      if (array && key === "length") {
        continue;
      }
      if (shown === PREVIEW_PROPERTIES || state.nodes >= PREVIEW_NODES) {
        more = true;
        break;
      }
      let desc;
      try {
        desc = ReflectGetOwnPropertyDescriptor(value, key);
      } catch (e) {
        desc = undefined;
      }
      if (desc === undefined) {
        continue;
      }
      const name = typeof key === "symbol" ? $SymbolToString(key) : key;
      const item = ObjectHasOwn(desc, "value")
        ? preview(desc.value, depth - 1, state)
        : '{"type":"accessor"}';
      out += (shown > 0 ? "," : "") + "[" + quote(name) + "," + item + "]";
      shown++;
    }
    return out + "]" + (more ? ',"more":true' : "") + "}";
  }

  // A map's entries, or a set's values.
  function entriesPreview(value, map, cls, depth, state) {
    const size = map ? $MapSize(value) : $SetSize(value);
    let out = '{"type":' + (map ? '"map"' : '"set"') + ',"class":' + quote(cls) +
      ',"size":' + $String(size) + ',"entries":[';
    let shown = 0;
    const each = (v, k) => {
      if (shown === PREVIEW_PROPERTIES || state.nodes >= PREVIEW_NODES) {
        return;
      }
      const item = map
        ? "[" + preview(k, depth - 1, state) + "," + preview(v, depth - 1, state) + "]"
        : preview(v, depth - 1, state);
      out += (shown > 0 ? "," : "") + item;
      shown++;
    };
    if (map) {
      $MapForEach(value, each);
    } else {
      $SetForEach(value, each);
    }
    return out + "]" + (shown < size ? ',"more":true' : "") + "}";
  }

  // An `Error`: its class, message and stack, read as data and never by
  // a getter.
  function errorPreview(value, cls) {
    const own = (name) => {
      const desc = ReflectGetOwnPropertyDescriptor(value, name);
      return desc !== undefined && typeof desc.value === "string" ? desc.value : "";
    };
    return '{"type":"error","class":' + quote(cls) + ',"message":' + quote(own("message")) +
      ',"stack":' + quote(stackOf(value)) + "}";
  }

  // A node: what the reference browser shows of one, its tag, id and
  // classes, from the document rather than the object, and its handle,
  // which the element inspector names it by.
  function nodePreview(node) {
    const handle = handleOf(node);
    const kind = native.kind(handle);
    let out = '{"type":"node","node":' + $String(handle) + ',"nodeType":' + $String(kind);
    if (kind === 1) {
      const id = native.attribute(handle, "id");
      const cls = native.attribute(handle, "class");
      out += ',"tag":' + quote(native.tag(handle)) +
        (id !== null ? ',"id":' + quote(id) : "") +
        (cls !== null ? ',"classes":' + quote(cls) : "");
    } else {
      out += ',"name":' + quote(nameOf(node));
    }
    return out + "}";
  }

  // The previews of `args` from `first` on, a JSON array.
  function previews(args, first) {
    const state = { __proto__: null, nodes: 0, path: [] };
    let out = "[";
    for (let i = first; i < args.length; i++) {
      out += (i > first ? "," : "") + preview(args[i], PREVIEW_DEPTH, state);
    }
    return out + "]";
  }

  // The stack where the console was called, the bootstrap's own frames
  // left out by the native.
  function stackHere() {
    return stackOf(new $Error());
  }

  // Logs `args` (Console Standard's Logger) at `level` as a call of
  // `kind`, with the stack where it was called: its place, and all of
  // it for an error or a trace.
  function logger(level, kind, args, first) {
    const call = [level, kind, previews(args, first), stackHere()];
    for (let i = first; i < args.length; i++) {
      ArrayPush(call, args[i]);
    }
    ReflectApply(consoleMessage, undefined, call);
  }

  const consoleCounts = new $Map();
  const consoleTimers = new $Map();
  let groupDepth = 0;

  function label(value) {
    return value === undefined ? "default" : toString(value);
  }

  const consoleMethods = {
    log(...args) {
      logger(CONSOLE_LOG, "log", args, 0);
    },
    info(...args) {
      logger(CONSOLE_INFO, "log", args, 0);
    },
    warn(...args) {
      logger(CONSOLE_WARN, "log", args, 0);
    },
    error(...args) {
      logger(CONSOLE_ERROR, "log", args, 0);
    },
    debug(...args) {
      logger(CONSOLE_DEBUG, "log", args, 0);
    },
    assert(condition, ...args) {
      if (condition) {
        return;
      }
      // "Assertion failed", then the format and its arguments.
      if (args.length === 0) {
        args = ["Assertion failed"];
      } else if (typeof args[0] === "string") {
        args[0] = "Assertion failed: " + args[0];
      } else {
        ArraySplice(args, 0, 0, "Assertion failed:");
      }
      logger(CONSOLE_ERROR, "log", args, 0);
    },
    clear() {
      groupDepth = 0;
      logger(CONSOLE_LOG, "clear", [], 0);
    },
    count(name) {
      const key = label(name);
      const count = (MapGet(consoleCounts, key) || 0) + 1;
      MapSet(consoleCounts, key, count);
      logger(CONSOLE_INFO, "log", [key + ": " + count], 0);
    },
    countReset(name) {
      const key = label(name);
      if (MapHas(consoleCounts, key)) {
        MapSet(consoleCounts, key, 0);
      } else {
        logger(CONSOLE_WARN, "log", ["Count for '" + key + "' does not exist"], 0);
      }
    },
    dir(item) {
      logger(CONSOLE_LOG, "dir", [item], 0);
    },
    dirxml(...args) {
      logger(CONSOLE_LOG, "log", args, 0);
    },
    group(...args) {
      groupDepth++;
      logger(CONSOLE_LOG, "group", args.length === 0 ? ["console.group"] : args, 0);
    },
    groupCollapsed(...args) {
      groupDepth++;
      logger(CONSOLE_LOG, "groupCollapsed", args.length === 0 ? ["console.groupCollapsed"] : args, 0);
    },
    groupEnd() {
      if (groupDepth > 0) {
        groupDepth--;
        logger(CONSOLE_LOG, "groupEnd", [], 0);
      }
    },
    table(data, properties) {
      // The panel draws the table of the preview; the text is a log's.
      logger(CONSOLE_LOG, "table", [data], 0);
    },
    time(name) {
      const key = label(name);
      if (MapHas(consoleTimers, key)) {
        logger(CONSOLE_WARN, "log", ["Timer '" + key + "' already exists"], 0);
        return;
      }
      MapSet(consoleTimers, key, native.now());
    },
    timeLog(name, ...args) {
      const key = label(name);
      const start = MapGet(consoleTimers, key);
      if (start === undefined) {
        logger(CONSOLE_WARN, "log", ["Timer '" + key + "' does not exist"], 0);
        return;
      }
      ArraySplice(args, 0, 0, key + ": " + (native.now() - start) + " ms");
      logger(CONSOLE_LOG, "log", args, 0);
    },
    timeEnd(name) {
      const key = label(name);
      const start = MapGet(consoleTimers, key);
      if (start === undefined) {
        logger(CONSOLE_WARN, "log", ["Timer '" + key + "' does not exist"], 0);
        return;
      }
      MapDelete(consoleTimers, key);
      logger(CONSOLE_INFO, "log", [key + ": " + (native.now() - start) + " ms - timer ended"], 0);
    },
    trace(...args) {
      logger(CONSOLE_LOG, "trace", args.length === 0 ? ["console.trace"] : args, 0);
    },
  };

  defineProperty(consoleMethods, toStringTag, { value: "console", configurable: true });
  defineProperty(global, "console", {
    value: consoleMethods,
    writable: true,
    enumerable: false,
    configurable: true,
  });

  // The DevTools console's expression (M36), run as a script of its own
  // in the global scope, the page's policies aside, with `$0` the node
  // the element inspector selected unless the page has a `$0` of its
  // own: its value's preview, or its exception's, given to the host.
  function* evaluateExpression(text, selected) {
    const own = ReflectGetOwnPropertyDescriptor(global, "$0");
    if (own === undefined) {
      defineProperty(global, "$0", {
        value: selected === null ? undefined : wrap(selected),
        writable: true,
        configurable: true,
      });
    }
    const state = { __proto__: null, nodes: 0, path: [] };
    let answer;
    try {
      const value = OriginalEval(text);
      answer = '{"result":' + preview(value, PREVIEW_DEPTH, state) + "}";
    } catch (e) {
      answer = '{"exception":' + preview(e, PREVIEW_DEPTH, state) + "}";
    } finally {
      if (own === undefined) {
        ReflectDeleteProperty(global, "$0");
      }
    }
    native.devtoolsAnswer(answer);
    return false;
  }

  // An entry's run that dispatches nothing.
  function* nothing() {
    return false;
  }

  // The DevTools panel's bridge (M36, docs/design/devtools.md): in the
  // panel's page alone, `devtools.send(message)` queues a message for
  // the browser, and each of the browser's calls `devtools.onmessage`.
  const devtoolsObject = native.realmKind === "devtools" ? {
    send(message) {
      native.devtoolsSend(toString(message));
    },
    onmessage: null,
  } : undefined;
  if (devtoolsObject !== undefined) {
    defineProperty(global, "devtools", {
      value: devtoolsObject,
      writable: false,
      enumerable: false,
      configurable: false,
    });
  }

  // A message from the browser to the panel's page: what its handler
  // throws is reported as an uncaught exception.
  function* devtoolsMessage(text) {
    const handler = devtoolsObject === undefined ? undefined : devtoolsObject.onmessage;
    if (typeof handler === "function") {
      try {
        handler.call(devtoolsObject, text);
      } catch (e) {
        reportException(e);
      }
    }
    return false;
  }

  // Worker `id`'s uncaught error (HTML section 10.2.4, "runtime script
  // errors"): an ErrorEvent at its object, cancelable, then, not
  // canceled, reported here as an error of this realm's own, with no
  // error object; for a script that failed to load (`message` null), a
  // plain `error` event. Whether it was canceled.
  function* workerError(id, message, url, line, column) {
    const worker = MapGet(workerObjects, id);
    if (worker === undefined) {
      return false;
    }
    const event = message === null
      ? new Event("error", made(EVENT_INIT, ["cancelable", true]))
      : new ErrorEvent("error", made(ERROR_INIT, [
        "cancelable", true,
        "message", "Uncaught " + message,
        "filename", url,
        "lineno", line,
        "colno", column,
        "error", null,
      ]));
    eventState(event).trusted = true;
    const canceled = yield* dispatching(worker, event);
    if (message !== null && !canceled && !fireError(null, message, url, line, column, false)) {
      native.reportError(message, url, line, column);
    }
    return canceled;
  }

  // ---- Channel messaging (M31, HTML section 9.5) ---------------------------

  // The ports this realm holds, by number. A port keeps its messages
  // until it is started, by start() or by setting onmessage, as the
  // host's router keeps them for it.
  const portObjects = new $Map();
  let portNumber;
  let portGone;
  let startPort;
  let neuterPort;

  class MessagePort extends EventTarget {
    #id = 0;
    // 0 not started, 1 started, 2 transferred or closed.
    #state = 0;

    static {
      portNumber = (port) => {
        if (typeof port === "object" && port !== null && #id in port) {
          return port.#id;
        }
        throw illegalInvocation();
      };
      isPort = (value) => typeof value === "object" && value !== null && #id in value;
      portGone = (port) => port.#state === 2;
      startPort = (port) => {
        if (port.#state === 0) {
          port.#state = 1;
          native.portStart(port.#id);
        }
      };
      neuterPort = (port) => {
        port.#state = 2;
        MapDelete(portObjects, port.#id);
      };
    }

    constructor(key, id) {
      if (key !== KEY) {
        throw illegalConstructor("MessagePort");
      }
      super();
      this.#id = id;
      MapSet(portObjects, id, this);
    }

    postMessage(message) {
      const id = portNumber(this);
      const a = arguments;
      need(a.length, 1, "postMessage", "MessagePort");
      const transfer = postTransfer(a[1], "postMessage", "MessagePort");
      const record = serialize(message, transfer, prefix("postMessage", "MessagePort"), this);
      if (this.#state === 2) {
        return;
      }
      try {
        native.portPost(id, wire(record));
      } catch (error) {
        throw failed(error, "postMessage", "MessagePort");
      }
    }

    start() {
      portNumber(this);
      startPort(this);
    }

    close() {
      const id = portNumber(this);
      if (this.#state !== 2) {
        neuterPort(this);
        native.portClose(id);
      }
    }
  }
  defineHandlers(MessagePort.prototype, ["message", "messageerror"], false);
  // Setting onmessage starts the port (HTML section 9.5.3).
  {
    const desc = getOwnPropertyDescriptor(MessagePort.prototype, "onmessage");
    const setHandler = desc.set;
    desc.set = function (value) {
      ReflectApply(setHandler, this, [value]);
      startPort(this);
    };
    defineProperty(MessagePort.prototype, "onmessage", desc);
  }
  expose(MessagePort, "MessagePort");

  class MessageChannel {
    #port1;
    #port2;

    constructor() {
      let ids;
      try {
        ids = native.portChannel();
      } catch (error) {
        if (error instanceof $RangeError) {
          error.message = "Failed to construct 'MessageChannel': " + error.message;
        }
        throw error;
      }
      this.#port1 = new MessagePort(KEY, ids[0]);
      this.#port2 = new MessagePort(KEY, ids[1]);
    }

    get port1() {
      if (!(#port1 in this)) {
        throw illegalInvocation();
      }
      return this.#port1;
    }

    get port2() {
      if (!(#port2 in this)) {
        throw illegalInvocation();
      }
      return this.#port2;
    }
  }
  expose(MessageChannel, "MessageChannel");

  // A port in a message's transfer list: its entry, unless it is the
  // port posting or one already transferred or closed.
  const transferableOther = transferable;
  transferable = function (t, i, where, source) {
    if (!isPort(t)) {
      return transferableOther(t, i, where, source);
    }
    if (t === source) {
      throw dataCloneError(where, "Port at index " + i + " contains the source port.");
    }
    return true;
  };
  const transferOther = transferEntry;
  transferEntry = function (t, i, where) {
    if (!isPort(t)) {
      return transferOther(t, i, where);
    }
    if (portGone(t)) {
      throw dataCloneError(where, "Port at index " + i + " is already neutered.");
    }
    return [PORT, portNumber(t)];
  };
  // Once the record is made, the ports in it are this realm's no more.
  const transferredOther = transferred;
  transferred = function (entries) {
    transferredOther(entries);
    for (let i = 0; i < entries.length; i++) {
      const e = entries[i];
      if (e !== null && e[0] === PORT) {
        const port = MapGet(portObjects, e[1]);
        if (port !== undefined) {
          neuterPort(port);
        }
        native.portAway(e[1]);
      }
    }
  };
  // A port taken from a message: this realm's from now on.
  const madeOther = transferMade;
  transferMade = function (e) {
    if (e[0] !== PORT) {
      return madeOther(e);
    }
    if (!native.portAdopt(e[1])) {
      throw new $TypeError("The message holds a port it may not.");
    }
    return new MessagePort(KEY, e[1]);
  };

  // ---- An OffscreenCanvas from a page's canvas (M31, HTML 4.12.5.3) ------

  // The canvas elements that gave their control away: placeholders,
  // which show what their OffscreenCanvas draws, wherever it is.
  const placeholders = new $WeakSet();

  // An OffscreenCanvas of the host's offscreen `made`, [number, width,
  // height], as transferControlToOffscreen and a transfer make one.
  function madeOffscreen(made) {
    const canvas = bindCanvasObject(ObjectCreate(OffscreenCanvas.prototype), 1, made[0],
      "OffscreenCanvas");
    const t = targetOf(canvas);
    t.width = made[1];
    t.height = made[2];
    t.context = null;
    return canvas;
  }

  {
    const proto = MapGet(interfaces, "canvas").prototype;
    const iface = "HTMLCanvasElement";
    defineProperty(proto, "transferControlToOffscreen", {
      value: function transferControlToOffscreen() {
        const handle = self(this);
        const method = "transferControlToOffscreen";
        if (WeakMapGet(canvasContexts, this) !== undefined) {
          throw domError("InvalidStateError",
            "Cannot transfer control from a canvas that has a rendering context.", method, iface);
        }
        if (WeakSetHas(placeholders, this)) {
          throw domError("InvalidStateError",
            "Cannot transfer control from a canvas for more than one time.", method, iface);
        }
        const made = native.canvas("transferControl", 0, handle);
        WeakSetAdd(placeholders, this);
        return madeOffscreen(made);
      },
      writable: true,
      enumerable: true,
      configurable: true,
    });
    // A placeholder has no context of its own, and keeps its size.
    const elementContext = proto.getContext;
    defineProperty(proto, "getContext", {
      value: function getContext(contextId) {
        if (WeakSetHas(placeholders, this)) {
          throw domError("InvalidStateError",
            "Cannot get context from a canvas that has transferred its control to offscreen.",
            "getContext", iface);
        }
        return ReflectApply(elementContext, this, arguments);
      },
      writable: true,
      enumerable: true,
      configurable: true,
    });
    const sizes = ["width", "height"];
    for (let i = 0; i < sizes.length; i++) {
      const name = sizes[i];
      const desc = getOwnPropertyDescriptor(proto, name);
      const set = desc.set;
      desc.set = function (value) {
        if (WeakSetHas(placeholders, this)) {
          throw new $DOMException("Failed to set the '" + name + "' property on '" + iface +
            "': Cannot resize canvas after call to transferControlToOffscreen().",
            "InvalidStateError");
        }
        ReflectApply(set, this, [value]);
      };
      defineProperty(proto, name, desc);
    }
  }

  // An OffscreenCanvas in a transfer list: its entry, unless it has a
  // context or went already. Once the record is made it is detached
  // here, its entry naming its placeholder, and made again where the
  // message arrives.
  const transferableBefore = transferable;
  transferable = function (t, i, where, source) {
    const target = WeakMapGet(canvasTargets, t);
    return (target !== undefined && target.kind === 1) || transferableBefore(t, i, where, source);
  };
  const transferBefore = transferEntry;
  transferEntry = function (t, i, where) {
    const target = WeakMapGet(canvasTargets, t);
    if (target === undefined || target.kind !== 1) {
      return transferBefore(t, i, where);
    }
    if (target.detached) {
      throw dataCloneError(where, "OffscreenCanvas at index " + i + " is already detached.");
    }
    if (target.context !== null) {
      throw new $DOMException(where +
        "An OffscreenCanvas could not be transferred because it had a rendering context.",
        "InvalidStateError");
    }
    return [OFFSCREEN_CANVAS, target.id, target.width, target.height];
  };
  const transferredBefore = transferred;
  transferred = function (entries) {
    transferredBefore(entries);
    for (let i = 0; i < entries.length; i++) {
      const e = entries[i];
      if (e !== null && e[0] === OFFSCREEN_CANVAS) {
        const ref = MapGet(canvasObjects, e[1]);
        const canvas = ref === undefined ? undefined : WeakRefDeref(ref);
        if (canvas !== undefined) {
          const t = targetOf(canvas);
          t.detached = true;
          t.width = 0;
          t.height = 0;
        }
        e[1] = native.canvas("offscreenAway", 1, e[1]);
      }
    }
  };
  const madeBefore = transferMade;
  transferMade = function (e) {
    if (e[0] !== OFFSCREEN_CANVAS) {
      return madeBefore(e);
    }
    return madeOffscreen(native.canvas("offscreenAdopt", 1, e[1], e[2], e[3]));
  };

  // ---- Code from strings (M33.1, CSP's 'unsafe-eval') ---------------------

  // Once the document's policies would refuse or report a string
  // compiled as code, `eval`, `Function` and the constructors of async,
  // generator and async generator functions (reached through their
  // prototypes' `constructor`) are replaced by functions that ask the
  // host first, then compile through the originals: an `EvalError` when
  // refused. They keep the originals' names, lengths and prototypes, so
  // that `eval === window.eval`, `Function.prototype.constructor ===
  // Function` and `instanceof Function` hold. An `eval` through them is
  // indirect: a direct `eval` loses its caller's scope while a policy
  // is there (a deviation, docs/design/hardening.md).
  let codeGuarded = false;

  function codeRefused(source) {
    const refused = native.codeAllowed("eval", source, -1);
    if (refused !== "") {
      throw new $EvalError(refused);
    }
  }

  // The source a function constructor would compile, as the reference
  // browser samples it.
  function functionSource(prefix, args) {
    let params = "";
    for (let i = 0; i < args.length - 1; i++) {
      params += (i === 0 ? "" : ",") + str(args[i]);
    }
    const body = args.length === 0 ? "" : str(args[args.length - 1]);
    return "(" + prefix + " anonymous(" + params + "\n) {\n" + body + "\n})";
  }

  function guardConstructor(Original, prefix) {
    const guard = function () {
      const args = [];
      for (let i = 0; i < arguments.length; i++) {
        ArrayPush(args, arguments[i]);
      }
      codeRefused(functionSource(prefix, args));
      return ReflectConstruct(Original, args, new.target === undefined ? guard : new.target);
    };
    defineProperty(guard, "name", { value: Original.name, configurable: true });
    defineProperty(guard, "length", { value: 1, configurable: true });
    defineProperty(guard, "prototype", { value: Original.prototype, writable: false });
    setPrototypeOf(guard, ReflectGetPrototypeOf(Original));
    defineProperty(Original.prototype, "constructor", {
      value: guard,
      writable: true,
      configurable: true,
    });
    return guard;
  }

  function guardCode() {
    if (codeGuarded) {
      return;
    }
    codeGuarded = true;
    const guarded = {
      eval(x) {
        if (typeof x === "string") {
          codeRefused(x);
        }
        return OriginalEval(x);
      },
    }.eval;
    defineProperty(global, "eval", { value: guarded, writable: true, configurable: true });
    const FunctionGuard = guardConstructor(OriginalFunction, "function");
    defineProperty(global, "Function", {
      value: FunctionGuard,
      writable: true,
      configurable: true,
    });
    guardConstructor(ReflectGetPrototypeOf(async function () {}).constructor, "async function");
    guardConstructor(ReflectGetPrototypeOf(function* () {}).constructor, "function*");
    guardConstructor(
      ReflectGetPrototypeOf(async function* () {}).constructor,
      "async function*",
    );
  }

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
    // A string the policies refuse to compile (M33.1): an id, and
    // nothing runs.
    if (typeof callback === "string" && native.codeAllowed("eval", callback, -1) !== "") {
      return id;
    }
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
        const url = native.documentUrl();
        native.evaluate(timer.callback, url === null ? "" : url);
      }
    } catch (error) {
      reportException(error);
    } finally {
      if (timer.repeat && MapGet(timers, id) === timer) {
        native.setTimer(id, timer.delay);
      }
    }
  }

  const timerFunctions = {
    setTimeout(handler, timeout = 0) {
      need(arguments.length, 1, "setTimeout", GLOBAL_INTERFACE);
      return startTimer(handler, timeout, ArraySlice(arguments, 2), false);
    },
    setInterval(handler, timeout = 0) {
      need(arguments.length, 1, "setInterval", GLOBAL_INTERFACE);
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

  // ---- Animation frames (HTML section 8.10, M26) ---------------------------

  // The callbacks a page asked to run before the next frame, by handle,
  // in the order asked; asking tells the host a frame is wanted.
  const frameCallbacks = new $Map();
  let lastFrame = 0;

  const frameFunctions = {
    requestAnimationFrame(callback) {
      need(arguments.length, 1, "requestAnimationFrame", GLOBAL_CLASS);
      if (typeof callback !== "function") {
        throw new $TypeError(
          prefix("requestAnimationFrame", GLOBAL_CLASS) +
            "The callback provided as parameter 1 is not a function.",
        );
      }
      lastFrame = lastFrame >= 2147483647 ? 1 : lastFrame + 1;
      MapSet(frameCallbacks, lastFrame, callback);
      native.wantFrame();
      return lastFrame;
    },
    cancelAnimationFrame(handle) {
      need(arguments.length, 1, "cancelAnimationFrame", GLOBAL_CLASS);
      MapDelete(frameCallbacks, long(handle));
    },
  };
  const frameNames = getOwnPropertyNames(frameFunctions);
  for (let i = 0; i < frameNames.length; i++) {
    defineProperty(global, frameNames[i], {
      value: frameFunctions[frameNames[i]],
      writable: true,
      enumerable: true,
      configurable: true,
    });
  }

  // HTML's "run the animation frame callbacks" for a frame at `now`: those
  // asked for before it, in order, each given `now`; those they ask for
  // wait for the next frame. A generator, as a dispatch is, that yields
  // after each callback for the realm's microtask checkpoint.
  function* runFrames(now) {
    const handles = [];
    MapForEach(frameCallbacks, (callback, handle) => {
      ArrayPush(handles, handle);
    });
    for (let i = 0; i < handles.length; i++) {
      const callback = MapGet(frameCallbacks, handles[i]);
      if (callback === undefined) {
        continue;
      }
      MapDelete(frameCallbacks, handles[i]);
      try {
        ReflectApply(callback, undefined, [now]);
      } catch (error) {
        reportException(error);
      }
      yield;
    }
    return false;
  }

  // ---- WebAssembly (the WebAssembly JavaScript Interface, M29) ------------

  // The namespace, its members defined as Web IDL defines a namespace's:
  // its operations enumerable, its classes not.
  const WebAssemblyNamespace = ObjectCreate(Object.prototype);
  defineProperty(WebAssemblyNamespace, toStringTag, { value: "WebAssembly", configurable: true });
  defineProperty(global, "WebAssembly", {
    value: WebAssemblyNamespace,
    writable: true,
    configurable: true,
    enumerable: false,
  });

  // An interface of the namespace: as `expose` makes one, its class
  // string `WebAssembly.` and its name, its operations and attributes,
  // static ones too, enumerable.
  function exposeWasm(cls, name) {
    const proto = cls.prototype;
    const own = [proto, cls];
    for (let j = 0; j < own.length; j++) {
      const keys = getOwnPropertyNames(own[j]);
      for (let i = 0; i < keys.length; i++) {
        if (j === 0 ? keys[i] === "constructor" : keys[i] === "length" || keys[i] === "name" || keys[i] === "prototype") {
          continue;
        }
        const desc = getOwnPropertyDescriptor(own[j], keys[i]);
        desc.enumerable = true;
        defineProperty(own[j], keys[i], desc);
      }
    }
    defineProperty(proto, toStringTag, { value: "WebAssembly." + name, configurable: true });
    defineProperty(WebAssemblyNamespace, name, {
      value: cls,
      writable: true,
      configurable: true,
      enumerable: false,
    });
  }

  function wasmOperation(name, f) {
    defineProperty(WebAssemblyNamespace, name, {
      value: f,
      writable: true,
      configurable: true,
      enumerable: true,
    });
  }

  // The interface's errors, native errors of its own as `TypeError` is
  // one of the language's: a name on the prototype, no class string.
  function wasmError(name) {
    const cls = class extends $Error {};
    defineProperty(cls, "name", { value: name, configurable: true });
    defineProperty(cls.prototype, "name", { value: name, writable: true, configurable: true });
    defineProperty(cls.prototype, "message", { value: "", writable: true, configurable: true });
    defineProperty(WebAssemblyNamespace, name, {
      value: cls,
      writable: true,
      configurable: true,
      enumerable: false,
    });
    return cls;
  }
  const CompileError = wasmError("CompileError");
  const LinkError = wasmError("LinkError");
  const RuntimeError = wasmError("RuntimeError");

  // A buffer source's bytes, copied, as base64 for the natives; a
  // TypeError in the reference browser's words for anything else.
  function moduleBytes(value, where) {
    const bytes = bufferBytes(value);
    if (bytes === undefined) {
      throw new $TypeError(where + ": Argument 0 must be a buffer source");
    }
    if (viewLength(bytes) > native.wasmModuleLimit) {
      throw new $RangeError(
        where + ": the module is larger than " + native.wasmModuleLimit / 1048576 + " MiB",
      );
    }
    return ToBase64(bytes);
  }

  // A module compiled from `bytes`: its handle, or a CompileError.
  function compileModule(bytes, where) {
    return compileText(moduleBytes(bytes, where), where);
  }

  // A module compiled from its bytes as base64, if the policies allow
  // (M33.1: 'wasm-unsafe-eval' or 'unsafe-eval').
  function compileText(text, where) {
    const refused = native.codeAllowed("wasm", "", -1);
    if (refused !== "") {
      throw new CompileError(where + ": " + refused);
    }
    const handle = native.wasmCompile(text, where);
    if (typeof handle === "string") {
      throw new CompileError(where + ": " + handle);
    }
    return handle;
  }

  wasmOperation("validate", function validate(bytes) {
    return native.wasmValidate(moduleBytes(bytes, "WebAssembly.validate()"));
  });

  // Each module object's handle.
  const modules = new $WeakMap();

  function moduleOf(value, where) {
    const handle = typeof value === "object" && value !== null ? WeakMapGet(modules, value) : undefined;
    if (handle === undefined) {
      throw new $TypeError(where + ": Argument 0 must be a WebAssembly.Module");
    }
    return handle;
  }

  // A module object for a handle the natives gave.
  function moduleObject(handle) {
    const module = ObjectCreate(Module.prototype);
    WeakMapSet(modules, module, handle);
    return module;
  }

  class Module {
    constructor(bytes) {
      WeakMapSet(modules, this, compileModule(bytes, "WebAssembly.Module()"));
    }

    static exports(module) {
      const list = native.wasmModuleExports(moduleOf(module, "WebAssembly.Module.exports()"));
      const out = [];
      for (let i = 0; i < list.length; i += 2) {
        ArrayPush(out, { name: list[i], kind: list[i + 1] });
      }
      return out;
    }

    static imports(module) {
      const list = native.wasmModuleImports(moduleOf(module, "WebAssembly.Module.imports()"));
      const out = [];
      for (let i = 0; i < list.length; i += 4) {
        ArrayPush(out, { module: list[i], name: list[i + 1], kind: list[i + 2] });
      }
      return out;
    }

    static customSections(module, sectionName) {
      const handle = moduleOf(module, "WebAssembly.Module.customSections()");
      if (arguments.length < 2) {
        throw new $TypeError(
          "WebAssembly.Module.customSections(): Argument 1 is required",
        );
      }
      // A DOMString: a lone surrogate names no section, whose names are
      // UTF-8.
      const name = toString(sectionName);
      return ToWellFormed(name) === name ? native.wasmModuleCustomSections(handle, name) : [];
    }
  }
  exposeWasm(Module, "Module");

  // What the natives say failed, as the interface's error.
  function wasmFailure(name, message) {
    switch (name) {
      case "LinkError":
        return new LinkError(message);
      case "RuntimeError":
        return new RuntimeError(message);
      case "CompileError":
        return new CompileError(message);
      case "RangeError":
        return new $RangeError(message);
    }
    return new $TypeError(message);
  }

  function incompatible() {
    return new $TypeError("type incompatibility when transforming from/to JS");
  }

  // The values a module's external references hold, by their indices.
  const externValues = new $Map();
  const externIndices = new $Map();

  function externIndex(value) {
    let index = MapGet(externIndices, value);
    if (index === undefined) {
      index = externValues.size;
      MapSet(externValues, index, value);
      MapSet(externIndices, value, index);
    }
    return index;
  }

  // A script's value as a WebAssembly value of the type `letter`, raw for
  // the natives (the interface's ToWebAssemblyValue): an `i32` by
  // ToInt32, an `i64` by ToBigInt64, a float by ToNumber, a function
  // reference an exported function's key, an external one any value's
  // index.
  function toWasm(value, letter) {
    if (typeof value === "bigint" && letter !== "I" && letter !== "e") {
      throw new $TypeError("Cannot convert a BigInt value to a number");
    }
    switch (letter) {
      case "i":
        return +value | 0;
      case "I":
        if (typeof value === "number") {
          throw new $TypeError("Cannot convert " + value + " to a BigInt");
        }
        return BigIntAsIntN(64, value);
      case "f":
      case "F":
        return +value;
      case "r": {
        if (value === null) {
          return null;
        }
        const key = typeof value === "function" ? WeakMapGet(exportedKeys, value) : undefined;
        if (key === undefined) {
          throw incompatible();
        }
        return key;
      }
      case "e":
        return value === null ? null : externIndex(value);
    }
    throw incompatible();
  }

  // A raw WebAssembly value as a script's (ToJSValue).
  function fromWasm(raw, letter) {
    if (raw === null) {
      return null;
    }
    if (letter === "r") {
      return exportedFunction(raw);
    }
    if (letter === "e") {
      return MapGet(externValues, raw);
    }
    return raw;
  }

  // A function's results, raw, as a script's: none undefined, one its
  // value, several an array.
  function resultsToJS(raw, letters, from) {
    if (letters.length === 0) {
      return undefined;
    }
    if (letters.length === 1) {
      return fromWasm(raw[from], letters[0]);
    }
    const out = [];
    for (let i = 0; i < letters.length; i++) {
      ArrayPush(out, fromWasm(raw[from + i], letters[i]));
    }
    return out;
  }

  // A script's function's return value as the results `letters`, raw:
  // several taken from an iterable of as many.
  function resultsFromJS(value, letters) {
    const out = [];
    if (letters.length === 1) {
      ArrayPush(out, toWasm(value, letters[0]));
    } else if (letters.length > 1) {
      const method = value === null || value === undefined ? undefined : value[iteratorSymbol];
      if (typeof method !== "function") {
        throw new $TypeError("multi-return value must be iterable");
      }
      const iterator = ReflectApply(method, value, []);
      const next = iterator.next;
      const values = [];
      for (;;) {
        const step = ReflectApply(next, iterator, []);
        if (step.done) {
          break;
        }
        ArrayPush(values, step.value);
      }
      if (values.length !== letters.length) {
        throw new $TypeError("multi-return length mismatch");
      }
      for (let i = 0; i < letters.length; i++) {
        ArrayPush(out, toWasm(values[i], letters[i]));
      }
    }
    return out;
  }

  // The scripts' callables imported, by their slots: each one with its
  // import's parameters and results.
  const hostFunctions = [];

  // A script's function called as a module's import, with raw arguments:
  // its results, raw.
  function callHost(slot, raw, from) {
    const host = hostFunctions[slot];
    const args = [];
    for (let i = 0; i < host.params.length; i++) {
      ArrayPush(args, fromWasm(raw[from + i], host.params[i]));
    }
    return resultsFromJS(ReflectApply(host.callable, undefined, args), host.results);
  }

  // A call into the store driven to its end: each time it stops at a
  // script's function, the function called and the call resumed with
  // what it returned; what the function throws ends the call and goes
  // on, the same value.
  function drive(answer) {
    for (;;) {
      leave();
      const status = answer[0];
      if (status === 0) {
        return answer;
      }
      if (status === 2) {
        throw wasmFailure(answer[1], answer[2]);
      }
      const id = answer[1];
      let results;
      try {
        results = callHost(answer[2], answer, 3);
      } catch (error) {
        native.wasmAbandon(id);
        throw error;
      }
      enter();
      answer = native.wasmResume(id, results);
    }
  }

  // The calls into the store under way, nested.
  let wasmCalls = 0;

  // Each function of the store a script has, by its key, and each one's
  // key: the same function object each time (the interface's cache of
  // exported functions).
  const exportedByKey = new $Map();
  const exportedKeys = new $WeakMap();

  // The exported function of the store's function `key`: a built-in as
  // the interface makes one, named by its index in its module, its
  // length its parameters', with no prototype and no [[Construct]].
  function exportedFunction(key) {
    let f = MapGet(exportedByKey, key);
    if (f !== undefined) {
      return f;
    }
    const info = native.wasmFuncInfo(key);
    const signature = info[0];
    const colon = StringIndexOf(signature, ":");
    const params = StringSlice(signature, 0, colon);
    const results = StringSlice(signature, colon + 1);
    const slot = info[2];
    const target = {
      run() {
        const raw = [];
        for (let i = 0; i < params.length; i++) {
          ArrayPush(raw, toWasm(arguments[i], params[i]));
        }
        if (slot >= 0) {
          // A script's function imported and exported again: called as
          // a module would call it.
          return resultsToJS(callHost(slot, raw, 0), results, 0);
        }
        const top = wasmCalls === 0;
        wasmCalls++;
        try {
          enter();
          return resultsToJS(drive(native.wasmCall(key, raw, top)), results, 1);
        } finally {
          wasmCalls--;
        }
      },
    }.run;
    f = FunctionBind(target, undefined);
    defineProperty(f, "length", { value: params.length, configurable: true });
    defineProperty(f, "name", { value: info[1], configurable: true });
    MapSet(exportedByKey, key, f);
    WeakMapSet(exportedKeys, f, key);
    return f;
  }

  function isObject(value) {
    return value !== null && (typeof value === "object" || typeof value === "function");
  }

  // The interface's instantiation of the module `handle` with the import
  // object `importObject`: the imports read in the module's order, the
  // instance made, its start function called; its exports object,
  // frozen, with no prototype.
  function instantiateModule(handle, importObject, where) {
    return makeInstance(handle, readImports(handle, importObject, where), where);
  }

  // The imports of the module `handle` read from `importObject`, in the
  // module's order: a tag and a value each, for the natives.
  function readImports(handle, importObject, where) {
    if (importObject !== undefined && !isObject(importObject)) {
      throw new $TypeError(where + ": Argument 1 must be an object");
    }
    const list = native.wasmModuleImports(handle);
    if (list.length > 0 && importObject === undefined) {
      throw new $TypeError(where + ": Imports argument must be present and must be an object");
    }
    const given = [];
    for (let i = 0; i < list.length; i += 4) {
      const label = "Import #" + i / 4 + ' "' + list[i] + '"';
      const from = importObject[list[i]];
      if (!isObject(from)) {
        throw new $TypeError(where + ": " + label + ": module is not an object or function");
      }
      const value = from[list[i + 1]];
      const named = where + ": " + label + ' "' + list[i + 1] + '": ';
      const kind = list[i + 2];
      const type = list[i + 3];
      if (kind === "function") {
        if (typeof value !== "function") {
          throw new LinkError(named + "function import requires a callable");
        }
        const key = WeakMapGet(exportedKeys, value);
        if (key !== undefined) {
          ArrayPush(given, "f", key);
        } else {
          const colon = StringIndexOf(type, ":");
          ArrayPush(given, "h", hostFunctions.length);
          ArrayPush(hostFunctions, {
            callable: value,
            params: StringSlice(type, 0, colon),
            results: StringSlice(type, colon + 1),
          });
        }
      } else if (kind === "global" && WeakMapGet(globalInfo, value) !== undefined) {
        ArrayPush(given, "G", WeakMapGet(globalInfo, value).key);
      } else if (kind === "global") {
        const letter = type[0];
        const fits =
          letter === "I"
            ? typeof value === "bigint"
            : letter === "r" || letter === "e" || typeof value === "number";
        if (!fits) {
          throw new LinkError(
            named + "global import must be a number, valid Wasm reference, or WebAssembly.Global object",
          );
        }
        if (type[1] === "m") {
          throw new LinkError(named + "imported mutable global must be a WebAssembly.Global object");
        }
        ArrayPush(given, "g", toWasm(value, letter));
      } else if (kind === "memory") {
        const key = isObject(value) ? WeakMapGet(memoryKeys, value) : undefined;
        if (key === undefined) {
          throw new LinkError(named + "memory import must be a WebAssembly.Memory object");
        }
        ArrayPush(given, "m", key);
      } else if (kind === "table") {
        const info = isObject(value) ? WeakMapGet(tableInfo, value) : undefined;
        if (info === undefined) {
          throw new LinkError(named + "table import requires a WebAssembly.Table");
        }
        ArrayPush(given, "t", info.key);
      } else {
        throw new LinkError(named + "tag import requires a WebAssembly.Tag");
      }
    }
    return given;
  }

  // An instance of the module `handle` with the imports `given`, its
  // start function called: its exports object.
  function makeInstance(handle, given, where) {
    enter();
    const made = native.wasmInstantiate(handle, given);
    leave();
    if (made[0] === 2) {
      throw wasmFailure(made[1], where + ": " + made[2]);
    }
    if (made[2] >= 0) {
      // The start function, which may be a script's function imported.
      ReflectApply(exportedFunction(made[2]), undefined, []);
    }
    const exports = ObjectCreate(null);
    const named = native.wasmInstanceExports(made[1]);
    for (let i = 0; i < named.length; i += 3) {
      const kind = named[i + 1];
      const key = named[i + 2];
      defineProperty(exports, named[i], {
        value:
          kind === "function"
            ? exportedFunction(key)
            : kind === "memory"
              ? memoryObject(key)
              : kind === "table"
                ? tableObject(key)
                : globalObject(key),
        writable: true,
        enumerable: true,
        configurable: true,
      });
    }
    return ObjectFreeze(exports);
  }

  // Each instance object's exports.
  const instanceExports = new $WeakMap();

  function instanceObject(exports) {
    const instance = ObjectCreate(Instance.prototype);
    WeakMapSet(instanceExports, instance, exports);
    return instance;
  }

  class Instance {
    constructor(module) {
      const importObject = arguments[1];
      const where = "WebAssembly.Instance()";
      const handle = moduleOf(module, where);
      WeakMapSet(instanceExports, this, instantiateModule(handle, importObject, where));
    }

    get exports() {
      const exports = isObject(this) ? WeakMapGet(instanceExports, this) : undefined;
      if (exports === undefined) {
        throw new $TypeError("Receiver is not a WebAssembly.Instance");
      }
      return exports;
    }
  }
  exposeWasm(Instance, "Instance");

  // ---- The asynchronous interface --------------------------------------------

  // `work` run in a promise job, the promise settled by what it returns
  // or throws: the interface's "in parallel", here on the realm's own
  // thread (a worker's is M31's).
  function later(work) {
    return PromiseThen(resolved(undefined), work);
  }

  // The interface's operations return promises: what they would throw
  // rejects them.
  function promising(work) {
    try {
      return work();
    } catch (error) {
      return rejected(error);
    }
  }

  // A promise of the result of instantiating the module `handle` with
  // the imports read from `importObject` now.
  function instantiateLater(handle, importObject, where) {
    const given = readImports(handle, importObject, where);
    return later(() => instanceObject(makeInstance(handle, given, where)));
  }

  // The module of bytes as base64 and its instance, as `instantiate`
  // resolves them.
  function moduleAndInstance(text, importObject, where) {
    const handle = compileText(text, where);
    const module = moduleObject(handle);
    const instance = instanceObject(instantiateModule(handle, importObject, where));
    return { module, instance };
  }

  wasmOperation("compile", function compile(bytes) {
    return promising(() => {
      const text = moduleBytes(bytes, "WebAssembly.compile()");
      return later(() => moduleObject(compileText(text, "WebAssembly.compile()")));
    });
  });

  wasmOperation("instantiate", function instantiate(source) {
    const importObject = arguments[1];
    const where = "WebAssembly.instantiate()";
    return promising(() => {
      const handle = isObject(source) ? WeakMapGet(modules, source) : undefined;
      if (handle !== undefined) {
        return instantiateLater(handle, importObject, where);
      }
      if (bufferBytes(source) === undefined) {
        throw new $TypeError(where + ": Argument 0 must be a buffer source or a WebAssembly.Module object");
      }
      const text = moduleBytes(source, where);
      return later(() => moduleAndInstance(text, importObject, where));
    });
  });

  const ResponseArrayBuffer = Response.prototype.arrayBuffer;

  // The bytes of a response, or of the promise of one, as base64: its
  // MIME type, its origin and its status checked as the WebAssembly Web
  // API says, in the reference browser's words.
  function responseText(source) {
    return PromiseThen(resolved(source), (response) => {
      const fail = (message) => new $TypeError("Failed to execute 'compile' on 'WebAssembly': " + message);
      if (!isResponse(response)) {
        throw fail("An argument must be provided, which must be a Response or Promise<Response> object");
      }
      const state = responseState(response);
      const type = combined(headersState(state.headers).list, "content-type");
      if (type === null || ToLowerCase(type) !== "application/wasm") {
        throw fail("Incorrect response MIME type. Expected 'application/wasm'.");
      }
      if (state.type === "opaque" || state.type === "opaqueredirect") {
        throw fail("Response is not CORS-same-origin");
      }
      if (state.status < 200 || state.status > 299) {
        throw fail("HTTP status code is not ok");
      }
      return PromiseThen(ReflectApply(ResponseArrayBuffer, response, []), (buffer) => {
        if (bufferLength(buffer) > native.wasmModuleLimit) {
          throw new $RangeError(
            "WebAssembly.compileStreaming(): the module is larger than " + native.wasmModuleLimit / 1048576 + " MiB",
          );
        }
        return ToBase64(new $Uint8Array(buffer));
      });
    });
  }

  wasmOperation("compileStreaming", function compileStreaming(source) {
    return promising(() =>
      PromiseThen(responseText(source), (text) =>
        moduleObject(compileText(text, "WebAssembly.compileStreaming()")),
      ),
    );
  });

  wasmOperation("instantiateStreaming", function instantiateStreaming(source) {
    const importObject = arguments[1];
    const where = "WebAssembly.instantiateStreaming()";
    return promising(() =>
      PromiseThen(responseText(source), (text) => moduleAndInstance(text, importObject, where)),
    );
  });

  // ---- Memory, Table and Global --------------------------------------------

  // A descriptor's property as Web IDL's [EnforceRange] unsigned long.
  function rangeProperty(value, where, property) {
    const number = +value;
    if (number !== number || number === Infinity || number === -Infinity) {
      throw new $TypeError(where + ": Property '" + property + "' must be convertible to a valid number");
    }
    const whole = MathTrunc(number);
    if (whole < 0 || whole > 4294967295) {
      throw new $TypeError(
        where + ": Property '" + property + "': value " + whole + " is outside the unsigned long range",
      );
    }
    return whole;
  }

  // An argument as Web IDL's [EnforceRange] unsigned long.
  function rangeArgument(value, where, n) {
    const number = +value;
    if (number !== number || number === Infinity || number === -Infinity) {
      throw new $TypeError(where + ": Argument " + n + " must be convertible to a valid number");
    }
    const whole = MathTrunc(number);
    if (whole < 0 || whole > 4294967295) {
      throw new $TypeError(where + ": Argument " + n + " must be in the unsigned long range");
    }
    return whole + 0;
  }

  // A descriptor's limits: its initial and its maximum (-1 for none),
  // within `bound`.
  function limits(descriptor, where, bound) {
    const initialValue = descriptor.initial;
    if (initialValue === undefined) {
      throw new $TypeError(where + ": Property 'initial' is required");
    }
    const initial = rangeProperty(initialValue, where, "initial");
    const maximumValue = descriptor.maximum;
    const maximum = maximumValue === undefined ? -1 : rangeProperty(maximumValue, where, "maximum");
    if (initial > bound) {
      throw new $RangeError(
        where + ": Property 'initial': value " + initial + " is above the upper bound " + bound,
      );
    }
    if (maximum > bound) {
      throw new $RangeError(
        where + ": Property 'maximum': value " + maximum + " is above the upper bound " + bound,
      );
    }
    if (maximum !== -1 && maximum < initial) {
      throw new $RangeError(
        where + ": Property 'maximum': value " + maximum + " is below the lower bound " + initial,
      );
    }
    return [initial, maximum];
  }

  // The memories a script has the buffer of, by their keys: each one's
  // buffer, a copy kept the same as the memory at the calls' edges.
  const mirrors = new $Map();

  // Before a call enters the store: what scripts wrote in the buffers,
  // into the memories.
  function enter() {
    if (mirrors.size === 0) {
      return;
    }
    const list = [];
    MapForEach(mirrors, (buffer, key) => {
      ArrayPush(list, key, ToBase64(new $Uint8Array(buffer)));
    });
    native.wasmSyncIn(list);
  }

  // After it leaves: what the modules wrote, into the buffers; a memory
  // that grew, its buffer detached.
  function leave() {
    if (mirrors.size === 0) {
      return;
    }
    const changes = native.wasmSyncOut();
    for (let i = 0; i < changes.length; i++) {
      const change = changes[i];
      const buffer = MapGet(mirrors, change[0]);
      if (change.length === 1) {
        MapDelete(mirrors, change[0]);
        native.wasmDetach(buffer);
        continue;
      }
      for (let j = 1; j < change.length; j += 2) {
        SetFromBase64(new $Uint8Array(buffer, change[j]), change[j + 1]);
      }
    }
  }

  // A memory's buffer detached and forgotten, as the memory grew.
  function refresh(key) {
    const buffer = MapGet(mirrors, key);
    if (buffer !== undefined) {
      MapDelete(mirrors, key);
      native.wasmDetach(buffer);
    }
  }

  // Each memory object's key, and each key's object (the interface's
  // cache of memory objects); the same for tables and globals, with each
  // one's type.
  const memoryKeys = new $WeakMap();
  const memoryByKey = new $Map();
  const tableInfo = new $WeakMap();
  const tableByKey = new $Map();
  const globalInfo = new $WeakMap();
  const globalByKey = new $Map();

  function memoryOf(value, where) {
    const key = isObject(value) ? WeakMapGet(memoryKeys, value) : undefined;
    if (key === undefined) {
      throw new $TypeError(where + ": Receiver is not a WebAssembly.Memory");
    }
    return key;
  }

  function memoryObject(key) {
    let memory = MapGet(memoryByKey, key);
    if (memory === undefined) {
      memory = ObjectCreate(Memory.prototype);
      WeakMapSet(memoryKeys, memory, key);
      MapSet(memoryByKey, key, memory);
    }
    return memory;
  }

  class Memory {
    constructor(descriptor) {
      const where = "WebAssembly.Memory()";
      if (!isObject(descriptor)) {
        throw new $TypeError(where + ": Argument 0 must be a memory descriptor");
      }
      const range = limits(descriptor, where, 65536);
      if (descriptor.shared) {
        throw new $TypeError(where + ": shared memories are not supported");
      }
      const made = native.wasmMemoryNew(range[0], range[1]);
      if (made[0] === 2) {
        throw wasmFailure(made[1], where + ": " + made[2]);
      }
      WeakMapSet(memoryKeys, this, made[1]);
      MapSet(memoryByKey, made[1], this);
    }

    grow(delta) {
      const where = "WebAssembly.Memory.grow()";
      const key = memoryOf(this, where);
      const pages = rangeArgument(delta, where, 0);
      enter();
      const made = native.wasmMemoryGrow(key, pages);
      leave();
      if (made[0] === 2) {
        throw wasmFailure(made[1], where + ": " + made[2]);
      }
      refresh(key);
      return made[1];
    }

    get buffer() {
      const key = memoryOf(this, "WebAssembly.Memory.buffer");
      let buffer = MapGet(mirrors, key);
      if (buffer === undefined) {
        buffer = native.wasmMemoryBuffer(key);
        MapSet(mirrors, key, buffer);
      }
      return buffer;
    }
  }
  exposeWasm(Memory, "Memory");

  // A reference type's letter, as a table's element names it.
  function elementLetter(element, where) {
    const name = toString(element);
    if (name === "anyfunc" || name === "funcref") {
      return "r";
    }
    if (name === "externref") {
      return "e";
    }
    throw new $TypeError(where + ": Descriptor property 'element' must be a WebAssembly reference type");
  }

  // The value a table's or a global's missing value is (DefaultValue).
  function defaultValue(letter) {
    switch (letter) {
      case "I":
        return 0n;
      case "r":
        return null;
      case "e":
        return externIndex(undefined);
    }
    return 0;
  }

  // A script's value for a table of `letter`'s references, raw.
  // `present` when the argument was given, undefined or not.
  function tableValue(present, value, letter, where, n) {
    if (!present) {
      return defaultValue(letter);
    }
    if (letter === "r" && value !== null && WeakMapGet(exportedKeys, value) === undefined) {
      throw new $TypeError(
        where +
          ": Argument " +
          n +
          " is invalid for table: function-typed object must be null (if nullable) or a Wasm function object",
      );
    }
    return toWasm(value, letter);
  }

  function tableOf(value, where) {
    const info = isObject(value) ? WeakMapGet(tableInfo, value) : undefined;
    if (info === undefined) {
      throw new $TypeError(where + ": Receiver is not a WebAssembly.Table");
    }
    return info;
  }

  function tableObject(key) {
    let table = MapGet(tableByKey, key);
    if (table === undefined) {
      table = ObjectCreate(Table.prototype);
      WeakMapSet(tableInfo, table, { key, letter: native.wasmTableType(key) });
      MapSet(tableByKey, key, table);
    }
    return table;
  }

  class Table {
    constructor(descriptor) {
      const value = arguments[1];
      const where = "WebAssembly.Table()";
      if (!isObject(descriptor)) {
        throw new $TypeError(where + ": Argument 0 must be a table descriptor");
      }
      const letter = elementLetter(descriptor.element, where);
      const range = limits(descriptor, where, 10000000);
      const made = native.wasmTableNew(letter, range[0], range[1], tableValue(value !== undefined, value, letter, where, 1));
      if (made[0] === 2) {
        throw wasmFailure(made[1], where + ": " + made[2]);
      }
      WeakMapSet(tableInfo, this, { key: made[1], letter });
      MapSet(tableByKey, made[1], this);
    }

    get length() {
      return native.wasmTableLength(tableOf(this, "WebAssembly.Table.length").key);
    }

    get(index) {
      const where = "WebAssembly.Table.get()";
      const info = tableOf(this, where);
      const made = native.wasmTableGet(info.key, rangeArgument(index, where, 0));
      if (made[0] === 2) {
        throw wasmFailure(made[1], where + ": " + made[2]);
      }
      return fromWasm(made[1], info.letter);
    }

    set(index) {
      const value = arguments[1];
      const where = "WebAssembly.Table.set()";
      const info = tableOf(this, where);
      const at = rangeArgument(index, where, 0);
      const made = native.wasmTableSet(info.key, at, tableValue(arguments.length > 1, value, info.letter, where, 1));
      if (made[0] === 2) {
        throw wasmFailure(made[1], where + ": " + made[2]);
      }
    }

    grow(delta) {
      const value = arguments[1];
      const where = "WebAssembly.Table.grow()";
      const info = tableOf(this, where);
      const by = rangeArgument(delta, where, 0);
      const made = native.wasmTableGrow(info.key, by, tableValue(value !== undefined, value, info.letter, where, 1));
      if (made[0] === 2) {
        throw wasmFailure(made[1], where + ": " + made[2]);
      }
      return made[1];
    }
  }
  exposeWasm(Table, "Table");

  // A value type's letter, as a global's descriptor names it.
  function valueLetter(value, where) {
    switch (toString(value)) {
      case "i32":
        return "i";
      case "i64":
        return "I";
      case "f32":
        return "f";
      case "f64":
        return "F";
      case "anyfunc":
      case "funcref":
        return "r";
      case "externref":
        return "e";
    }
    throw new $TypeError(where + ": Descriptor property 'value' must be a WebAssembly type");
  }

  function globalOf(value, where) {
    const info = isObject(value) ? WeakMapGet(globalInfo, value) : undefined;
    if (info === undefined) {
      throw new $TypeError(where + ": Receiver is not a WebAssembly.Global");
    }
    return info;
  }

  function globalObject(key) {
    let global = MapGet(globalByKey, key);
    if (global === undefined) {
      global = ObjectCreate(Global.prototype);
      const type = native.wasmGlobalType(key);
      WeakMapSet(globalInfo, global, { key, letter: type[0], mutable: type[1] === "m" });
      MapSet(globalByKey, key, global);
    }
    return global;
  }

  function globalValue(global, where) {
    const info = globalOf(global, where);
    return fromWasm(native.wasmGlobalGet(info.key), info.letter);
  }

  class Global {
    constructor(descriptor) {
      const value = arguments[1];
      const where = "WebAssembly.Global()";
      if (!isObject(descriptor)) {
        throw new $TypeError(where + ": Argument 0 must be a global descriptor");
      }
      const mutable = !!descriptor.mutable;
      const letter = valueLetter(descriptor.value, where);
      const raw = value === undefined ? defaultValue(letter) : toWasm(value, letter);
      const key = native.wasmGlobalNew(letter, mutable, raw);
      WeakMapSet(globalInfo, this, { key, letter, mutable });
      MapSet(globalByKey, key, this);
    }

    get value() {
      return globalValue(this, "get WebAssembly.Global.value");
    }

    set value(value) {
      const info = globalOf(this, "set WebAssembly.Global.value");
      if (!info.mutable) {
        throw new $TypeError("set WebAssembly.Global.value): Can't set the value of an immutable global.");
      }
      native.wasmGlobalSet(info.key, toWasm(value, info.letter));
    }

    valueOf() {
      return globalValue(this, "WebAssembly.Global.valueOf()");
    }
  }
  exposeWasm(Global, "Global");

  // The entry points the realm calls (M26): a timer's task; an event
  // the engine makes, dispatched at the node `handle` names or, for
  // null, at the window, as a generator `resume` steps through, which
  // ends with whether the event was canceled; the animation frame
  // callbacks, stepped through the same way; a script's uncaught
  // exception reported; and the bootstrap's own state put back after a
  // run its budget stopped.
  return {
    timer: runTimer,
    dispatch(handle, iface, type, init) {
      const known = MapGet(eventInterfaces, iface);
      if (typeof init.relatedTarget === "number") {
        init.relatedTarget = wrap(init.relatedTarget);
      }
      if (typeof init.submitter === "number") {
        init.submitter = wrap(init.submitter);
      }
      // A `storage` event's area, which the engine names (M28).
      if (typeof init.storageArea === "boolean") {
        init.storageArea = storageArea(init.storageArea);
      }
      const event = new known[1](type, dictionary(init, known[2], iface));
      const s = eventState(event);
      s.trusted = true;
      // A mouse event the engine makes knows its point on the page and
      // from its target's padding edge (M26.5).
      if (typeof init.pageX === "number") {
        s.page = [init.pageX, init.pageY];
      }
      if (typeof init.offsetX === "number") {
        s.offset = [init.offsetX, init.offsetY];
      }
      if (handle === null) {
        // The window's `load` names the document its target.
        return dispatching(global, event, type === "load" ? theDocument() : undefined);
      }
      return dispatching(wrap(handle), event);
    },
    frames: runFrames,
    frameList,
    guardCode,
    devtools: devtoolsMessage,
    evaluate: evaluateExpression,
    fetched,
    // A message's arrival (M31): at the global (`kind` 0), at worker
    // `n`'s object (1) or at port `n` (2).
    message(kind, n, text, origin) {
      // From frame `n`'s window (3, M33.2): at the window, its source.
      if (kind === 3) {
        return messageEvent(global, text, origin, windowFor(n));
      }
      const target = kind === 0 ? global : MapGet(kind === 1 ? workerObjects : portObjects, n);
      return target === undefined ? nothing() : messageEvent(target, text, origin);
    },
    workerError,
    // Worker `id` ended: its object is let go.
    workerEnded(id) {
      MapDelete(workerObjects, id);
      return nothing();
    },
    resume(run) {
      const step = GeneratorNext(run);
      return step.done ? step.value : null;
    },
    report: fireError,
    reset() {
      native.wasmReset();
      wasmCalls = 0;
      reporting = false;
      currentEvent = undefined;
      MapClear(clicking);
    },
  };
});
