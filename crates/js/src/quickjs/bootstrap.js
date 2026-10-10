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
    for (let i = 2; i < answer.length; i++) {
      attributeHandler(object, answer[i], true);
    }
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
    } else if (target === global) {
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
      return new $Uint8Array(ArrayBufferSlice(value, 0));
    }
    if (typeof value === "object" && value !== null && IsView(value)) {
      const typed = isTypedArray(value);
      const buffer = typed ? viewBuffer(value) : dataViewBuffer(value);
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

    constructor(form, submitter) {
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
          integrity: "", keepalive: false, duplex: "half",
        };
        fallbackMode = "cors";
      } else {
        const from = input.#state;
        state = {
          method: from.method, url: from.url, headers: null, body: from.body, mode: from.mode,
          credentials: from.credentials, cache: from.cache, redirect: from.redirect,
          referrer: from.referrer, referrerPolicy: from.referrerPolicy, integrity: from.integrity,
          keepalive: from.keepalive, duplex: "half",
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
          state.mode, state.credentials, state.redirect, false,
        );
      } catch (e) {
        reject(failed(e, "fetch", "Window"));
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
      if (!this.#async && this.#state !== UNSENT) {
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
      if (!isAsync && (this.#timeout !== 0 || this.#responseType !== "")) {
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
          this.#uploadListened,
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
      if (this.#state === LOADING || this.#state === DONE) {
        throw new $DOMException(
          "Failed to set the 'responseType' property on 'XMLHttpRequest': The response type cannot be set if the object's state is LOADING or DONE.",
          "InvalidStateError",
        );
      }
      if (!this.#async && this.#state !== UNSENT) {
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
      removeAttribute(handle, ToLowerCase(str(name)));
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
  setPrototypeOf(global, Window.prototype);
  // The window is tagged by its prototype, as a Window, not by
  // QuickJS-ng's own tag.
  ReflectDeleteProperty(global, toStringTag);

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

  // `window.event` (HTML section 7.2.2.1): the event whose listener
  // runs; replaceable.
  defineProperty(global, "event", {
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
      need(arguments.length, 1, "reportError", "Window");
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
  defineHandlers(Document.prototype, GLOBAL_HANDLERS, false);
  defineHandlers(Document.prototype, DOCUMENT_ELEMENT_HANDLERS, false);
  defineHandlers(Document.prototype, ["readystatechange", "visibilitychange"], false);
  defineHandlers(Window.prototype, GLOBAL_HANDLERS, false);
  defineHandlers(Window.prototype, WINDOW_HANDLERS, false);
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
  include(textControlMembers, [HTMLInputElement.prototype, HTMLTextAreaElement.prototype]);

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
    [HTMLInputElement.prototype],
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
    [HTMLTextAreaElement.prototype],
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
    [HTMLSelectElement.prototype],
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
    [HTMLOptionElement.prototype],
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
    [HTMLFormElement.prototype],
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
    [HTMLLabelElement.prototype],
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
    [HTMLButtonElement.prototype],
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
      HTMLInputElement.prototype,
      HTMLSelectElement.prototype,
      HTMLTextAreaElement.prototype,
      HTMLButtonElement.prototype,
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
    [HTMLFormElement.prototype],
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
    [HTMLElement.prototype],
  );

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

  // ---- Animation frames (HTML section 8.10, M26) ---------------------------

  // The callbacks a page asked to run before the next frame, by handle,
  // in the order asked; asking tells the host a frame is wanted.
  const frameCallbacks = new $Map();
  let lastFrame = 0;

  const frameFunctions = {
    requestAnimationFrame(callback) {
      need(arguments.length, 1, "requestAnimationFrame", "Window");
      if (typeof callback !== "function") {
        throw new $TypeError(
          prefix("requestAnimationFrame", "Window") +
            "The callback provided as parameter 1 is not a function.",
        );
      }
      lastFrame = lastFrame >= 2147483647 ? 1 : lastFrame + 1;
      MapSet(frameCallbacks, lastFrame, callback);
      native.wantFrame();
      return lastFrame;
    },
    cancelAnimationFrame(handle) {
      need(arguments.length, 1, "cancelAnimationFrame", "Window");
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
    fetched,
    resume(run) {
      const step = GeneratorNext(run);
      return step.done ? step.value : null;
    },
    report: fireError,
    reset() {
      reporting = false;
      currentEvent = undefined;
      MapClear(clicking);
    },
  };
});
