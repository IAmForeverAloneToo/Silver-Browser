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

    constructor(type, eventInitDict) {
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
    constructor(type, eventInitDict) {
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
    constructor(type, eventInitDict) {
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
    constructor(type, eventInitDict) {
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
    constructor(type, eventInitDict) {
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
    constructor(type, eventInitDict) {
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
    constructor(type, eventInitDict) {
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
    constructor(type, eventInitDict) {
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
    constructor(type, eventInitDict) {
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
    constructor(type, eventInitDict) {
      const args = eventArgs(arguments.length, type, eventInitDict, SUBMIT_INIT, "SubmitEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= SUBMIT;
    }

    get submitter() {
      return fieldsOf(this, SUBMIT).submitter;
    }
  }

  class ErrorEvent extends Event {
    constructor(type, eventInitDict) {
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
    constructor(type, eventInitDict) {
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
    constructor(type, eventInitDict) {
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
    constructor(type, eventInitDict) {
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
  function sequence(value, method, iface) {
    const iterate = typeof value === "object" && value !== null ? value[iteratorSymbol] : undefined;
    if (typeof iterate !== "function") {
      throw new $TypeError(
        prefix(method, iface) + "The provided value cannot be converted to a sequence.",
      );
    }
    const iterator = ReflectApply(iterate, value, []);
    const items = [];
    for (;;) {
      const step = iterator.next();
      if (step.done) {
        return items;
      }
      ArrayPush(items, step.value);
    }
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
  // of the document's elements whose `name` is `elementName`.
  include(
    {
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
