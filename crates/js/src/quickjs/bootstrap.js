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
  const ArrayShift = uncurry(Array.prototype.shift);
  const StringIndexOf = uncurry(String.prototype.indexOf);
  const StringSplit = uncurry(String.prototype.split);
  const StringTrim = uncurry(String.prototype.trim);
  const PromiseReject = (value) => ReflectApply(Promise.reject, Promise, [value]);
  const PromiseResolve = (value) => ReflectApply(Promise.resolve, Promise, [value]);
  const FromCharCode = String.fromCharCode;
  const MapGet = uncurry(Map.prototype.get);
  const MapSet = uncurry(Map.prototype.set);
  const MapDelete = uncurry(Map.prototype.delete);
  const MapHas = uncurry(Map.prototype.has);
  const ArraySlice = uncurry(Array.prototype.slice);
  const ObjectCreate = Object.create;
  const ObjectPrototype = Object.prototype;
  const $WeakSet = WeakSet;
  const WeakSetHas = uncurry(WeakSet.prototype.has);
  const WeakSetAdd = uncurry(WeakSet.prototype.add);
  const MapClear = uncurry(Map.prototype.clear);
  const MapForEach = uncurry(Map.prototype.forEach);
  const GeneratorNext = uncurry(Object.getPrototypeOf(function* () {}).prototype.next);
  const WeakMapGet = uncurry(WeakMap.prototype.get);
  const WeakMapSet = uncurry(WeakMap.prototype.set);
  const WeakMapDelete = uncurry(WeakMap.prototype.delete);
  const WeakRefDeref = uncurry(WeakRef.prototype.deref);
  const Register = uncurry(FinalizationRegistry.prototype.register);
  const ArrayPush = uncurry(Array.prototype.push);
  const ArrayIncludes = uncurry(Array.prototype.includes);
  const ArrayValues = Array.prototype.values;
  const MathRound = Math.round;
  const ArrayEntries = Array.prototype.entries;
  const ArrayKeys = Array.prototype.keys;
  const ArrayForEach = Array.prototype.forEach;
  const ToWellFormed = uncurry(String.prototype.toWellFormed);
  const ToUpperCase = uncurry(String.prototype.toUpperCase);
  const ToLowerCase = uncurry(String.prototype.toLowerCase);
  const StringIncludes = uncurry(String.prototype.includes);
  const StringSlice = uncurry(String.prototype.slice);
  const CharCodeAt = uncurry(String.prototype.charCodeAt);
  const CodePointAt = uncurry(String.prototype.codePointAt);
  const StringEndsWith = uncurry(String.prototype.endsWith);
  const NumberToString = uncurry(Number.prototype.toString);
  const iteratorSymbol = Symbol.iterator;
  const toStringTag = Symbol.toStringTag;
  // %IteratorPrototype%, which the built-ins' iterators inherit from.
  const IteratorPrototype = Object.getPrototypeOf(Object.getPrototypeOf([][iteratorSymbol]()));

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

  // A binding's toString() (D47, gap batch A): the bindings are read
  // without their source, so QuickJS writes each as it writes its own
  // functions, as native code; here on one line, as the reference
  // writes a native function. Any other function as QuickJS writes it.
  const QuickToString = uncurry(Function.prototype.toString);
  const NATIVE_TAIL = "() {\n    [native code]\n}";
  defineProperty(Function.prototype, "toString", {
    value: {
      toString() {
        const text = QuickToString(this);
        if (!StringEndsWith(text, NATIVE_TAIL)) {
          return text;
        }
        return StringSlice(text, 0, text.length - NATIVE_TAIL.length) + "() { [native code] }";
      },
    }.toString,
    writable: true,
    configurable: true,
  });

  // The current time (ECMA-262 21.4): `Date.now()`, `new Date()` and
  // `Date()` read the system's precise clock in whole milliseconds, as
  // the reference browser's do. QuickJS reads Windows' coarse clock, which
  // steps by about 16 ms, so that two calls milliseconds apart read the
  // same. The constructor is QuickJS's behind one of the same shape,
  // which keeps its prototype; given a time, it is QuickJS's.
  {
    const QuickDate = Date;
    const QuickDateString = uncurry(QuickDate.prototype.toString);
    const wall = () => native.wallClock();
    const PreciseDate = function Date(year, month, date, hours, minutes, seconds, ms) {
      if (new.target === undefined) {
        return QuickDateString(new QuickDate(wall()));
      }
      const args = arguments.length === 0 ? [wall()] : arguments;
      return ReflectConstruct(QuickDate, args, new.target);
    };
    defineProperty(PreciseDate, "prototype", {
      value: QuickDate.prototype,
      writable: false,
      enumerable: false,
      configurable: false,
    });
    defineProperty(PreciseDate, "now", {
      value: {
        now() {
          return wall();
        },
      }.now,
      writable: true,
      configurable: true,
    });
    for (const name of ["parse", "UTC"]) {
      defineProperty(PreciseDate, name, getOwnPropertyDescriptor(QuickDate, name));
    }
    defineProperty(QuickDate.prototype, "constructor", {
      value: PreciseDate,
      writable: true,
      configurable: true,
    });
    defineProperty(global, "Date", {
      value: PreciseDate,
      writable: true,
      configurable: true,
    });
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
  // A service worker's (M39) is a worker's too.
  const DEDICATED = native.realmKind === "worker";
  const SERVICE = native.realmKind === "serviceworker";
  const WORKER = DEDICATED || SERVICE;
  const WORKER_NAME = DEDICATED ? native.workerName : "";

  // The interface of the members the window and the workers share,
  // which they name in their errors, and the global's own.
  const GLOBAL_INTERFACE = WORKER ? "WorkerGlobalScope" : "Window";
  const GLOBAL_CLASS = SERVICE
    ? "ServiceWorkerGlobalScope"
    : WORKER
      ? "DedicatedWorkerGlobalScope"
      : "Window";

  // Whether the realm is a secure context's (M39): the interfaces Web
  // IDL marks `[SecureContext]` are its alone.
  const SECURE = native.secure === true;
  const SECURE_ONLY = {
    __proto__: null, Cache: true, CacheStorage: true, ServiceWorker: true,
    ServiceWorkerContainer: true, ServiceWorkerRegistration: true,
  };

  // Of the interfaces the bindings make, those every worker has too
  // (true), the workers alone ("worker"), a dedicated worker alone
  // ("dedicated"), a service worker alone ("service", M39), or the
  // window and a dedicated worker ("notservice"); the others are the
  // window's.
  const WORKER_INTERFACES = {
    __proto__: null,
    AbortController: true, AbortSignal: true, Blob: true, CanvasGradient: true,
    CanvasPattern: true, CustomEvent: true, DOMMatrix: true, DOMMatrixReadOnly: true,
    DOMPoint: true, DOMPointReadOnly: true, DedicatedWorkerGlobalScope: "dedicated",
    ErrorEvent: true, Event: true, EventTarget: true, File: true, FontFace: true,
    FontFaceSet: true, FontFaceSetLoadEvent: true, FormData: true,
    Headers: true, ImageBitmap: true, ImageData: true, MessageEvent: true,
    OffscreenCanvas: true, OffscreenCanvasRenderingContext2D: true, Path2D: true,
    Performance: true, ProgressEvent: "notservice", QuotaExceededError: true, Request: true,
    Response: true, TextDecoder: true, TextEncoder: true, TextMetrics: true, URL: true,
    URLSearchParams: true, Worker: "notservice", FileReader: true,
    MessageChannel: true, MessagePort: true, Cache: true, CacheStorage: true,
    WorkerGlobalScope: "worker", WorkerLocation: "worker", WorkerNavigator: "worker",
    XMLHttpRequest: "notservice", XMLHttpRequestEventTarget: "notservice",
    XMLHttpRequestUpload: "notservice", ServiceWorker: true, ServiceWorkerRegistration: true,
    ServiceWorkerContainer: true, ServiceWorkerGlobalScope: "service", ExtendableEvent: "service",
    InstallEvent: "service", ExtendableMessageEvent: "service", Clients: "service", Client: "service",
    WindowClient: "service", FetchEvent: "service", FontFace: true, FontFaceSet: true,
  };

  // Whether an interface `where` says so is the realm's.
  function exposedHere(where) {
    if (SERVICE) return where === true || where === "worker" || where === "service";
    if (WORKER) return where === true || where === "worker" || where === "dedicated" || where === "notservice";
    return where === undefined || where === true || where === "notservice";
  }

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
    if (!exposedHere(WORKER_INTERFACES[name])) {
      return cls;
    }
    if (SECURE_ONLY[name] && !SECURE) {
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

  // ---- Binding chunks, made at their first use (D64) -----------------------
  // A few big chunks of the bindings most realms never use: WebGPU and
  // WebGL (M41), Intl, IndexedDB and `navigator.storage`, Web Audio,
  // WebAssembly, the 2D canvas and OffscreenCanvas, and the Web
  // Animations API (D75). Made at once,
  // they cost every realm that made them: WebGPU and WebGL a third more
  // time (T4.3), the others a fifth more again (D64).
  // Each is a module of its own, compiled once a process and read by
  // `native.chunk`: a function of the bootstrap's names it uses
  // (`CHUNK_ENVS`), giving what the rest of the bindings call of it.
  // Until a page reaches one, its names on the global, and on the
  // prototypes that read it, are accessors that make it, made where the
  // chunk was and as this realm's kind exposes them; `chunkInstall` makes
  // it once.

  // What each chunk made gave, by its name.
  const chunkParts = { __proto__: null };
  // Each chunk's accessors, by its name: target, name and descriptor.
  const chunkAccessors = { __proto__: null };

  // Chunk `chunk`'s parts, made now if they were not.
  function chunkInstall(chunk) {
    const parts = chunkParts[chunk];
    if (parts !== undefined) return parts;
    // What a page put in place of an accessor (or its deletion) stays.
    const accessors = chunkAccessors[chunk] || [];
    const kept = [];
    for (let i = 0; i < accessors.length; i += 3) {
      if (!chunkPending(accessors[i], accessors[i + 1], accessors[i + 2])) {
        ArrayPush(kept, i, getOwnPropertyDescriptor(accessors[i], accessors[i + 1]));
      }
    }
    const before = getOwnPropertyNames(madeInterfaces);
    // The chunk, read now, given the bootstrap's names it uses.
    const made = native.chunk(chunk)(CHUNK_ENVS[chunk]());
    chunkParts[chunk] = made;
    for (let k = 0; k < kept.length; k += 2) {
      const i = kept[k];
      if (kept[k + 1] === undefined) {
        ReflectDeleteProperty(accessors[i], accessors[i + 1]);
      } else {
        ReflectDefineProperty(accessors[i], accessors[i + 1], kept[k + 1]);
      }
    }
    // Every interface made here and exposed here is one of the accessors'.
    const now = getOwnPropertyNames(madeInterfaces);
    for (let i = 0; i < now.length; i++) {
      const name = now[i];
      if (ArrayIncludes(before, name) || !chunkExposed(name)) continue;
      let found = false;
      for (let j = 0; j < accessors.length; j += 3) {
        if (accessors[j] === global && accessors[j + 1] === name) found = true;
      }
      if (!found) throw new $Error("the bindings' lazy names lack " + name);
    }
    return made;
  }

  // Whether this realm exposes interface `name`, as `expose` decides.
  function chunkExposed(name) {
    return exposedHere(WORKER_INTERFACES[name]) && !(SECURE_ONLY[name] && !SECURE);
  }

  // Whether `desc`, an accessor of a chunk's, is still `name` on `target`.
  function chunkPending(target, name, desc) {
    const now = getOwnPropertyDescriptor(target, name);
    return now !== undefined && now.get === desc.get;
  }

  // An accessor of chunk `chunk` for `name` on `target`, enumerable or
  // not: reading it makes the chunk and gives what the chunk put there,
  // read as `this`; for a data property (`data`), setting it makes the
  // chunk, then sets it.
  function chunkLazy(chunk, target, name, enumerable, data) {
    const desc = {
      get() {
        chunkInstall(chunk);
        // Not made after all: no second try.
        if (chunkPending(target, name, desc)) return undefined;
        const made = getOwnPropertyDescriptor(target, name);
        if (made === undefined) return undefined;
        return made.get !== undefined ? ReflectApply(made.get, this, []) : made.value;
      },
      set: data ? function (value) {
        chunkInstall(chunk);
        if (!chunkPending(target, name, desc)) ReflectSet(target, name, value, this);
      } : undefined,
      enumerable,
      configurable: true,
    };
    defineProperty(target, name, desc);
    if (chunkAccessors[chunk] === undefined) chunkAccessors[chunk] = [];
    ArrayPush(chunkAccessors[chunk], target, name, desc);
  }

  // Accessors of chunk `chunk` for the interfaces of `names` this realm
  // exposes, as `expose` would put them on the global.
  function chunkInterfaces(chunk, names) {
    for (let i = 0; i < names.length; i++) {
      if (chunkExposed(names[i])) chunkLazy(chunk, global, names[i], false, true);
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
    "replaceAll",
    "setTextContent",
    "setInnerHTML",
    "setOuterHTML",
    "insertAdjacentHTML",
    "setTitle",
    "write",
    "normalize",
    "splitText",
    "attachShadow",
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
    const object = make(handle, answer[0], answer[2], +answer[3], answer[4]);
    MapSet(objects, handle, {
      ref: new $WeakRef(object),
      strong: answer[1] === "" ? null : object,
    });
    Register(registry, object, handle);
    for (let i = 5; i < answer.length; i++) {
      attributeHandler(object, answer[i], true);
    }
    return object;
  }

  // Whether a document beside the page's has had an object made (D67):
  // until one has, every node's document is the page's, or an inert one
  // of its type.
  let otherDocuments = false;

  // A new object for a node, of the interface its kind, name and
  // namespace say; `extra` is an element's prefix, or a document's
  // interface when it is not the page's. The host hears it from `wrap`,
  // and holds the node until it is released.
  function make(handle, name, ns, kind, extra) {
    switch (kind) {
      case 3:
        return new Text(KEY, handle, 3, name);
      case 4:
        return new CDATASection(KEY, handle, 4, name);
      case 7:
        return new ProcessingInstruction(KEY, handle, 7, name);
      case 8:
        return new Comment(KEY, handle, 8, name);
      case 9:
        if (extra !== "") {
          otherDocuments = true;
        }
        if (extra === "Document") {
          return new Document(KEY, handle, 9, name);
        }
        if (extra === "XMLDocument") {
          return new XMLDocument(KEY, handle, 9, name);
        }
        return new HTMLDocument(KEY, handle, 9, name);
      case 10:
        return new DocumentType(KEY, handle, 10, name);
      case 11:
        if (extra === "ShadowRoot") {
          return new ShadowRoot(KEY, handle, 11, name);
        }
        return new DocumentFragment(KEY, handle, 11, name);
    }
    const object = new (elementInterface(name, ns))(KEY, handle, 1, name);
    setNsOf(object, ns === "" ? null : ns);
    if (extra !== "") {
      setPrefixOf(object, extra);
    }
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

  // HTML's "erase all event listeners and handlers": the target's event
  // handlers deactivated and every listener removed.
  function eraseListeners(target) {
    MapForEach(handlersOf(target), (handler) => {
      handler.value = null;
      handler.element = null;
      handler.listener = null;
    });
    const listeners = listenersOf(target);
    for (let i = 0; i < listeners.length; i++) {
      listeners[i].removed = true;
    }
    ArraySplice(listeners, 0, listeners.length);
  }

  // `document.open()`'s erasing (HTML section 8.4.2.1): the window's
  // listeners, and those of the nodes `handles` names, the document and
  // its descendants a script holds; none when it opened nothing.
  function eraseOpened(handles) {
    if (handles === null || handles === undefined || handles.length === 0) {
      return;
    }
    eraseListeners(global);
    for (let i = 0; i < handles.length; i++) {
      eraseListeners(wrap(handles[i]));
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
      case "unsigned long long?":
        if (value === null) {
          return null;
        }
      // Falls through.
      case "unsigned long long": {
        const n = $Number(value);
        if (n !== n || n === Infinity || n === -Infinity) {
          return 0;
        }
        const whole = n - (n % 1);
        return whole < 0 ? 0 : whole;
      }
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
      case "Element?":
        if (value === null || (isNode(value) && kindOf(value) === 1)) {
          return value;
        }
        throw new $TypeError(where + "Failed to convert value to 'Element'.");
      case "Storage?":
        if (value === null || WeakMapGet(areaKinds, value) !== undefined) {
          return value;
        }
        throw new $TypeError(where + "Failed to convert value to 'Storage'.");
      case "MessageEventSource?":
        if (value === null || value === global || isPort(value) || WeakSetHas(swSources, value)) {
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

    // DOM's `composedPath()` (section 2.2): the path's invocation
    // targets, those in a closed shadow tree the current target is not
    // in left out (D68).
    composedPath() {
      const s = eventState(this);
      const path = s.path;
      if (path.length === 0) {
        return [];
      }
      const current = s.currentTarget;
      const out = [current];
      let currentIndex = 0;
      let currentLevel = 0;
      for (let i = path.length - 1; i >= 0; i--) {
        if (path[i].rootClosed) {
          currentLevel++;
        }
        if (path[i].item === current) {
          currentIndex = i;
          break;
        }
        if (path[i].slotClosed) {
          currentLevel--;
        }
      }
      let level = currentLevel;
      let max = currentLevel;
      const before = [];
      for (let i = currentIndex - 1; i >= 0; i--) {
        if (path[i].rootClosed) {
          level++;
        }
        if (level <= max) {
          ArrayPush(before, path[i].item);
        }
        if (path[i].slotClosed) {
          level--;
          if (level < max) {
            max = level;
          }
        }
      }
      level = currentLevel;
      max = currentLevel;
      for (let i = currentIndex + 1; i < path.length; i++) {
        if (path[i].slotClosed) {
          level++;
        }
        if (level <= max) {
          ArrayPush(out, path[i].item);
        }
        if (path[i].rootClosed) {
          level--;
          if (level < max) {
            max = level;
          }
        }
      }
      const result = [];
      for (let i = before.length - 1; i >= 0; i--) {
        ArrayPush(result, before[i]);
      }
      for (let i = 0; i < out.length; i++) {
        ArrayPush(result, out[i]);
      }
      return result;
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

  // A script's MouseEvent's `offsetX` (`axis` 0) or `offsetY` (CSSOM
  // View section 12): while it is dispatched, its client coordinate from
  // the padding edge of its target as the listener sees it, retargeted
  // (D68), or of the block box an inline target is in, as the reference
  // browser measures it; else the client coordinate.
  function dispatchedOffset(s, axis) {
    const client = axis === 0 ? s.fields.clientX : s.fields.clientY;
    if (!s.dispatching || !isNode(s.target) || kindOf(s.target) !== 1) {
      return client;
    }
    let box = handleOf(s.target);
    for (let up = native.parent(box); up !== null && native.kind(up) === 1;
      up = native.parent(box)) {
      const display = native.computedValue(box, "", "display");
      if (display !== "inline" && display !== "contents") {
        break;
      }
      box = up;
    }
    const g = native.geometry(box, false);
    if (g.length === 0) {
      return client;
    }
    return axis === 0 ? client - g[0] - g[G_BORDER + 3] : client - g[1] - g[G_BORDER];
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
      return (s.offset === null ? dispatchedOffset(s, 0) : s.offset[0]) | 0;
    }

    get offsetY() {
      const s = eventOf(this, MOUSE);
      return (s.offset === null ? dispatchedOffset(s, 1) : s.offset[1]) | 0;
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

  // The session history's events (HTML section 7.2.2): `popstate` with an
  // entry's state, `hashchange` with the URLs a fragment changed between.
  const POP_STATE = 1048576;
  const HASH_CHANGE = 2097152;
  const POP_STATE_INIT = members(EVENT_INIT, [
    ["hasUAVisualTransition", "boolean", false],
    ["state", "any", null],
  ], "PopStateEventInit");
  const HASH_CHANGE_INIT = members(EVENT_INIT, [
    ["newURL", "USVString", ""],
    ["oldURL", "USVString", ""],
  ], "HashChangeEventInit");

  class PopStateEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, POP_STATE_INIT,
        "PopStateEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= POP_STATE;
    }

    get state() {
      return fieldsOf(this, POP_STATE).state;
    }

    get hasUAVisualTransition() {
      return fieldsOf(this, POP_STATE).hasUAVisualTransition;
    }
  }

  class HashChangeEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, HASH_CHANGE_INIT,
        "HashChangeEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= HASH_CHANGE;
    }

    get oldURL() {
      return fieldsOf(this, HASH_CHANGE).oldURL;
    }

    get newURL() {
      return fieldsOf(this, HASH_CHANGE).newURL;
    }
  }

  const ANIMATION = 4096;
  const TRANSITION = 8192;
  const ANIMATION_INIT = members(EVENT_INIT, [
    ["animation", "CSSAnimation?", null],
    ["animationName", "DOMString", ""],
    ["elapsedTime", "double", 0],
    ["pseudoElement", "DOMString", ""],
  ], "AnimationEventInit");
  const TRANSITION_INIT = members(EVENT_INIT, [
    ["animation", "CSSTransition?", null],
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

    // The CSSAnimation it is of (Animations 2 section 5.1).
    get animation() {
      return fieldsOf(this, ANIMATION).animation;
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

    // The CSSTransition it is of (Transitions 2 section 5.1).
    get animation() {
      return fieldsOf(this, TRANSITION).animation;
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

  // The events of the top layer and the invokers (HTML sections 4.11.4,
  // 6.12 and 6.14, gap batch H): a dialog's and a popover's
  // `beforetoggle` and `toggle`, a command button's `command` at its
  // target, and an interest invoker's `interest` and `loseinterest`.
  const TOGGLE = 1 << 27;
  const COMMAND = 1 << 28;
  const INTEREST = 1 << 29;
  const TOGGLE_INIT = members(EVENT_INIT, [
    ["newState", "DOMString", ""],
    ["oldState", "DOMString", ""],
    ["source", "Element?", null],
  ], "ToggleEventInit");
  const COMMAND_INIT = members(EVENT_INIT, [
    ["command", "DOMString", ""],
    ["source", "Element?", null],
  ], "CommandEventInit");
  const INTEREST_INIT = members(EVENT_INIT, [["source", "Element?", null]], "InterestEventInit");

  class ToggleEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, TOGGLE_INIT, "ToggleEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= TOGGLE;
    }

    get oldState() {
      return fieldsOf(this, TOGGLE).oldState;
    }

    get newState() {
      return fieldsOf(this, TOGGLE).newState;
    }

    get source() {
      return sourceSeen(this, TOGGLE);
    }
  }

  class CommandEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, COMMAND_INIT, "CommandEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= COMMAND;
    }

    get command() {
      return fieldsOf(this, COMMAND).command;
    }

    get source() {
      return sourceSeen(this, COMMAND);
    }
  }

  class InterestEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, INTEREST_INIT, "InterestEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= INTEREST;
    }

    get source() {
      return sourceSeen(this, INTEREST);
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

  // What an event at an object of no tree goes on to after it (M36.7):
  // IndexedDB's request to its transaction, and that to its database.
  const eventParents = new $WeakMap();

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
    ["ToggleEvent", ToggleEvent, TOGGLE_INIT],
    ["CommandEvent", CommandEvent, COMMAND_INIT],
    ["InterestEvent", InterestEvent, INTEREST_INIT],
    ["ErrorEvent", ErrorEvent, ERROR_INIT],
    ["PageTransitionEvent", PageTransitionEvent, PAGE_TRANSITION_INIT],
    ["PopStateEvent", PopStateEvent, POP_STATE_INIT],
    ["HashChangeEvent", HashChangeEvent, HASH_CHANGE_INIT],
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
    ["hashchangeevent", HashChangeEvent, HASH_CHANGE_INIT],
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

  // The source of a toggle, command or interest event dispatched, or the
  // submitter of a submit event, which the reference browser takes as
  // one: a node, or null.
  function sourceOf(s) {
    let source = null;
    if ((s.kinds & (TOGGLE | COMMAND | INTEREST)) !== 0) {
      source = s.fields.source;
    } else if ((s.kinds & SUBMIT) !== 0) {
      source = s.fields.submitter;
    }
    return source !== null && source !== undefined && isNode(source) ? source : null;
  }

  // An event's `source` as its listener sees it: retargeted against the
  // object the event is at, or its target once dispatched (DOM's
  // retargeting, gap batch H), so that a source in a shadow tree is its
  // host outside it.
  function sourceSeen(event, bit) {
    const s = eventOf(event, bit);
    const source = s.fields.source;
    if (source === null || source === undefined || !isNode(source)) {
      return source === undefined ? null : source;
    }
    // Once dispatched, against its document, as the reference browser
    // keeps it: a source in a shadow tree is the outermost host.
    const at = s.currentTarget !== null && isNode(s.currentTarget) ? handleOf(s.currentTarget) :
      s.dispatchedOnce === true ? native.ownerDocument(handleOf(source)) : null;
    return at === null ? source : wrap(native.retarget(handleOf(source), at));
  }

  // The event's related target, for the events that have one: a
  // FocusEvent's or a MouseEvent's.
  function relatedOf(s) {
    if ((s.kinds & (FOCUS | MOUSE)) === 0) {
      return null;
    }
    const related = s.fields.relatedTarget;
    return related === undefined ? null : related;
  }

  // One struct of an event's path (DOM section 2.9): its invocation
  // target, its shadow-adjusted target (null for none), its related
  // target, whether it is the root of a closed tree or a slot in a
  // closed tree, and whether its invocation target is in a shadow tree.
  function pathStruct(item, adjusted, related, rootClosed, slotClosed, inShadow) {
    return { item, target: adjusted, related, rootClosed, slotClosed, inShadow };
  }

  // The event path of an event at `target` (D68): for a node, the host's
  // (`native.eventPath`), its targets and related targets retargeted at
  // each shadow boundary, then the window from the document, but for a
  // `load` event; for another target, it and its parents. Null when the
  // related target, retargeted, is the target, and nothing is
  // dispatched. Its last member says whether the targets are cleared
  // after the dispatch.
  function eventPath(target, s, override) {
    const related = relatedOf(s);
    const path = [];
    if (!isNode(target)) {
      ArrayPush(path, pathStruct(target, override === undefined ? target : override, related,
        false, false, false));
      if (target === global && !WORKER) {
        windowBody();
      } else {
        // An IndexedDB request's, transaction's and database's parents
        // (M36.7, its section 2.2's "get the parent").
        for (let p = WeakMapGet(eventParents, target); p !== undefined;
          p = WeakMapGet(eventParents, p)) {
          ArrayPush(path, pathStruct(p, null, related, false, false, false));
        }
      }
      return [path, false];
    }
    const relatedNode = related !== null && isNode(related) ? handleOf(related) : null;
    const answer = native.eventPath(handleOf(target), relatedNode, s.composed);
    if (answer.length === 0) {
      return null;
    }
    const count = (answer.length - 2) / 6;
    const node = (n) => (n < 0 ? null : wrap(n));
    for (let i = 0; i < count; i++) {
      const at = i * 6;
      ArrayPush(path, pathStruct(
        wrap(answer[at]),
        node(answer[at + 1]),
        relatedNode === null ? related : node(answer[at + 2]),
        answer[at + 3] === 1,
        answer[at + 4] === 1,
        answer[at + 5] === 1,
      ));
    }
    if (override !== undefined) {
      path[0].target = override;
    }
    // An event of a source (a toggle's, a command's, an interest's) goes
    // no further than the tree of the source: its path ends at the first
    // root that is a shadow-including ancestor of the source, as the
    // reference browser has it with reference targets (gap batch H).
    const source = sourceOf(s);
    if (source !== null) {
      const from = handleOf(source);
      for (let i = 0; i < path.length; i++) {
        const kind = kindOf(path[i].item);
        if ((kind === 9 || kind === 11) && shadowIncludes(handleOf(path[i].item), from)) {
          path.length = i + 1;
          break;
        }
      }
    }
    if (kindOf(path[path.length - 1].item) === 9 && s.type !== "load") {
      windowBody();
      const related = relatedNode === null ? relatedOf(s) : node(answer[count * 6 + 1]);
      ArrayPush(path, pathStruct(global, null, related, false, false, false));
    }
    return [path, answer[count * 6] === 1];
  }

  // The DOM's dispatch of `event` at `target` (section 2.9), its target
  // `override` if given (HTML's legacy target override): a generator
  // that yields after each listener it calls, so that the realm
  // performs a microtask checkpoint there when the engine dispatches; a
  // script's dispatch is run through at once. It ends with whether the
  // event was canceled.
  //
  // The path ([`eventPath`]) goes through shadow trees as the event's
  // `composed` says, each listener seeing the target and the related
  // target retargeted to its tree (D68). A click (a MouseEvent) runs
  // the activation behaviour of the target, or of the nearest ancestor
  // that has one when it bubbles, unless a listener canceled it.
  function* dispatching(target, event, override) {
    const s = eventState(event);
    s.dispatching = true;
    s.target = override === undefined ? target : override;
    const made = eventPath(target, s, override);
    if (made === null) {
      s.dispatching = false;
      return s.canceled;
    }
    const path = made[0];
    let activator = null;
    if (isNode(target) && (s.kinds & MOUSE) !== 0 && s.type === "click") {
      const handle = handleOf(target);
      const found = native.activator(handle);
      if (found !== null && (found === handle || s.bubbles)) {
        activator = found;
      }
    }
    // A checkbox's or a radio button's legacy-pre-activation behaviour
    // (HTML section 4.10.5.1.15, M26.8): its new state before the
    // listeners see the click.
    const pre = activator === null ? null : preActivate(activator);
    s.path = path;
    try {
      for (let i = path.length - 1; i >= 0; i--) {
        s.phase = path[i].target !== null ? 2 : 1;
        if (structTargets(path, i, s)) {
          break;
        }
        const item = path[i].item;
        s.currentTarget = item;
        const listeners = ArraySlice(listenersOf(item));
        for (let j = 0; j < listeners.length; j++) {
          const l = listeners[j];
          if (l.removed || l.type !== s.type || !l.capture) {
            continue;
          }
          invoke(item, l, s, event);
          yield;
          if (s.stopImmediate) {
            break;
          }
        }
      }
      for (let i = 0; i < path.length; i++) {
        if (path[i].target !== null) {
          s.phase = 2;
        } else if (!s.bubbles) {
          continue;
        } else {
          s.phase = 3;
        }
        if (structTargets(path, i, s)) {
          break;
        }
        const item = path[i].item;
        s.currentTarget = item;
        const listeners = ArraySlice(listenersOf(item));
        for (let j = 0; j < listeners.length; j++) {
          const l = listeners[j];
          if (l.removed || l.type !== s.type || l.capture) {
            continue;
          }
          invoke(item, l, s, event);
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
      s.inShadow = false;
      s.dispatching = false;
      s.stop = false;
      s.stopImmediate = false;
      // What a source is retargeted against from now on (gap batch H).
      s.dispatchedOnce = true;
      // A target in a shadow tree is not told outside it (step 10).
      if (made[1]) {
        s.target = null;
        if ((s.kinds & (FOCUS | MOUSE)) !== 0) {
          s.fields.relatedTarget = null;
        }
      }
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
    // A button's command or its popover target's (HTML sections 4.10.6
    // and 6.12.3, gap batch H).
    if (invokerActivation(handle, target)) {
      return;
    }
    // The summary of its `details`: the `details` opened or closed (HTML
    // section 4.11.2, gap batch H).
    if (tag === "summary") {
      const details = summaryDetails(handle);
      if (details !== null) {
        if (hasOpen(details)) {
          removeAttribute(details, "open");
        } else {
          setAttribute(details, "open", "", "click");
        }
      }
      return;
    }
    native.activate(handle);
  }

  // A `button`'s type, `submit` by default (HTML section 4.10.6).
  // A button's type: with none or an invalid one, `button` for one with
  // a `command` or `commandfor` attribute, else `submit` (gap batch H).
  function buttonType(handle) {
    const value = native.attribute(handle, "type");
    const lower = value === null ? "" : ToLowerCase(value);
    if (lower === "reset" || lower === "button" || lower === "submit") return lower;
    return native.attribute(handle, "command") !== null ||
      native.attribute(handle, "commandfor") !== null ? "button" : "submit";
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

  // A form-associated element's `form` IDL attribute: the form owner
  // of `of`, which a reference target may have put in a shadow tree,
  // retargeted against `against`, so the host there (gap batch H).
  function formSeen(of, against) {
    const owner = native.formOwner(of);
    return owner === null ? null : wrap(native.retarget(owner, against));
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
      // One with a submitter is composed, its path no further than the
      // submitter's tree, as the reference browser has it (gap batch H).
      const event = new SubmitEvent("submit", made(SUBMIT_INIT, [
        "bubbles", true,
        "cancelable", true,
        "composed", submitter !== null,
        "submitter", submitter === null ? null : wrap(submitter),
      ]));
      eventState(event).trusted = true;
      if (yield* dispatching(wrap(form), event)) {
        return;
      }
    }
    // The dialog method closes the form's dialog, its result the
    // submitter's value (HTML section 4.10.21.3, gap batch H).
    if (formMethod(form, submitter) === "dialog") {
      const dialog = dialogAround(form);
      if (dialog !== null) {
        const value = submitter === null ? null : native.attribute(submitter, "value");
        closeDialog(dialog, value, null);
      }
      return;
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

  // The DOM's "invoke" for the struct at `i` of `path`, its first steps:
  // the event's target the shadow-adjusted target of the nearest struct
  // at or before it that has one, its related target the struct's.
  // Whether propagation was stopped, and nothing is to be invoked.
  function structTargets(path, i, s) {
    for (let j = i; j >= 0; j--) {
      if (path[j].target !== null) {
        s.target = path[j].target;
        break;
      }
    }
    if ((s.kinds & (FOCUS | MOUSE)) !== 0) {
      s.fields.relatedTarget = path[i].related;
    }
    s.inShadow = path[i].inShadow;
    return s.stop;
  }

  // The DOM's "inner invoke" for one listener: removed first if it is
  // once; called with `window.event` the event and, when passive, its
  // canceling refused; what it throws reported.
  function invoke(target, listener, s, event) {
    if (listener.once) {
      removeListener(listenersOf(target), listener);
    }
    const previous = currentEvent;
    // A listener in a shadow tree does not see it as `window.event`
    // (HTML section 7.2.2.1, D68).
    currentEvent = s.inShadow ? undefined : event;
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

  // URLSearchParams (URL Standard 6, M27): a list of names and values,
  // and the URL whose `searchParams` it is, if any (gap batch A).
  let paramsOf;
  let isParams;
  let paramsLink;
  let paramsReset;
  let urlQueryChanged;
  class URLSearchParams {
    #list = [];
    #url = null;

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
      paramsLink = (params, url) => {
        params.#url = url;
      };
      paramsReset = (params, query) => {
        params.#list = formParse(query);
      };
    }

    // A URL's object gives its URL the list as the query (URL Standard
    // 6.2's update steps).
    #update() {
      if (this.#url !== null) {
        urlQueryChanged(this.#url, formSerialize(this.#list));
      }
    }

    get size() {
      return this.#list.length;
    }

    append(name, value) {
      need(arguments.length, 2, "append", "URLSearchParams");
      ArrayPush(this.#list, [str(name), str(value)]);
      this.#update();
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
      this.#update();
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
      this.#update();
    }

    sort() {
      // Stable, by the names' code units.
      ArraySort(this.#list, (a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
      this.#update();
    }

    toString() {
      return formSerialize(this.#list);
    }
  }
  pairIterator("URLSearchParams", (params) => paramsOf(params)).install(URLSearchParams, (value) => {
    if (!isParams(value)) throw illegalInvocation();
  });
  expose(URLSearchParams, "URLSearchParams");

  // URL (URL Standard 6.1, gap batch A): the values of its getters, which
  // the engine's parser hands back as `href`, `origin`, `protocol`,
  // `username`, `password`, `host`, `hostname`, `port`, `pathname`,
  // `search` and `hash`, and the URLSearchParams object of its query.
  const SEARCH = 9;

  // The parts of `url` against `base`, or null, as URL.parse() gives.
  function urlParse(url, base, constructing) {
    const input = str(url);
    if (base === undefined) {
      return native.urlParse(input, null);
    }
    const against = str(base);
    if (native.urlParse(against, null) === null) {
      if (constructing) {
        throw new $TypeError("Failed to construct 'URL': Invalid base URL");
      }
      return null;
    }
    return native.urlParse(input, against);
  }

  class URL {
    #parts;
    #query = new URLSearchParams();

    constructor(url, base = undefined) {
      needToConstruct(arguments.length, 1, "URL");
      const parts = urlParse(url, base, true);
      if (parts === null) {
        throw new $TypeError("Failed to construct 'URL': Invalid URL");
      }
      this.#parts = parts;
      paramsReset(this.#query, StringSlice(parts[SEARCH], 1));
      paramsLink(this.#query, this);
    }

    static {
      urlQueryChanged = (url, query) => {
        url.#parts = native.urlSet(url.#parts[0], "searchParams", query);
      };
    }

    static parse(url, base = undefined) {
      need(arguments.length, 1, "parse", "URL");
      const parts = urlParse(url, base, false);
      return parts === null ? null : new URL(parts[0]);
    }

    static canParse(url, base = undefined) {
      need(arguments.length, 1, "canParse", "URL");
      return urlParse(url, base, false) !== null;
    }

    // A setter's run, which leaves the URL as it was for a value the
    // part cannot take; `search`'s and `href`'s give the query's object
    // its list again.
    #set(part, value) {
      const parts = native.urlSet(this.#parts[0], part, str(value));
      if (parts === null) {
        throw new $TypeError("Failed to set the '" + part + "' property on 'URL': Invalid URL");
      }
      this.#parts = parts;
      if (part === "href" || part === "search") {
        paramsReset(this.#query, StringSlice(parts[SEARCH], 1));
      }
    }

    get searchParams() {
      return this.#query;
    }

    toJSON() {
      return this.#parts[0];
    }

    toString() {
      return this.#parts[0];
    }

    get href() {
      return this.#parts[0];
    }

    set href(value) {
      this.#set("href", value);
    }

    get origin() {
      return this.#parts[1];
    }

    get protocol() {
      return this.#parts[2];
    }

    set protocol(value) {
      this.#set("protocol", value);
    }

    get username() {
      return this.#parts[3];
    }

    set username(value) {
      this.#set("username", value);
    }

    get password() {
      return this.#parts[4];
    }

    set password(value) {
      this.#set("password", value);
    }

    get host() {
      return this.#parts[5];
    }

    set host(value) {
      this.#set("host", value);
    }

    get hostname() {
      return this.#parts[6];
    }

    set hostname(value) {
      this.#set("hostname", value);
    }

    get port() {
      return this.#parts[7];
    }

    set port(value) {
      this.#set("port", value);
    }

    get pathname() {
      return this.#parts[8];
    }

    set pathname(value) {
      this.#set("pathname", value);
    }

    get search() {
      return this.#parts[9];
    }

    set search(value) {
      this.#set("search", value);
    }

    get hash() {
      return this.#parts[10];
    }

    set hash(value) {
      this.#set("hash", value);
    }
  }
  expose(URL, "URL");

  // URL.createObjectURL() and revokeObjectURL() (File API 8, gap batch
  // A): a `blob:` URL the process's store answers with the blob's bytes
  // for the realm's origin, until revoked or the realm ends. No service
  // worker has them.
  if (!SERVICE) {
    defineProperty(URL, "createObjectURL", {
      value: function createObjectURL(obj) {
        need(arguments.length, 1, "createObjectURL", "URL");
        if (!isBlob(obj)) {
          throw new $TypeError(prefix("createObjectURL", "URL") + "Overload resolution failed.");
        }
        return native.blobUrlCreate(binaryString(blobBytes(obj)), BlobType(obj));
      },
      writable: true,
      configurable: true,
    });
    defineProperty(URL, "revokeObjectURL", {
      value: function revokeObjectURL(url) {
        need(arguments.length, 1, "revokeObjectURL", "URL");
        native.blobUrlRevoke(str(url));
      },
      writable: true,
      configurable: true,
    });
  }

  // ---- TextEncoder and TextDecoder (Encoding 7, gap batch A) ----

  // UTF-8 written here, from the string's code points; any other
  // encoding read by the engine's decoders, a stream's decoder kept
  // there between its calls by a number of the realm's.
  class TextEncoder {
    #utf8 = true;

    get encoding() {
      return this.#utf8 ? "utf-8" : "";
    }

    encode(input = "") {
      this.#utf8;
      return utf8Encode(str(input));
    }

    encodeInto(source, destination) {
      this.#utf8;
      need(arguments.length, 2, "encodeInto", "TextEncoder");
      const text = str(source);
      if (destination === null || typeof destination !== "object" || !isTypedArray(destination) ||
        TypedArrayTag(destination) !== "Uint8Array") {
        throw new $TypeError(prefix("encodeInto", "TextEncoder") +
          "parameter 2 is not of type 'Uint8Array'.");
      }
      const room = viewLength(destination);
      let read = 0;
      let written = 0;
      while (read < text.length) {
        const c = CodePointAt(text, read);
        const size = c < 0x80 ? 1 : c < 0x800 ? 2 : c < 0x10000 ? 3 : 4;
        if (written + size > room) {
          break;
        }
        if (size === 1) {
          destination[written] = c;
        } else if (size === 2) {
          destination[written] = 0xc0 | (c >> 6);
          destination[written + 1] = 0x80 | (c & 0x3f);
        } else if (size === 3) {
          destination[written] = 0xe0 | (c >> 12);
          destination[written + 1] = 0x80 | ((c >> 6) & 0x3f);
          destination[written + 2] = 0x80 | (c & 0x3f);
        } else {
          destination[written] = 0xf0 | (c >> 18);
          destination[written + 1] = 0x80 | ((c >> 12) & 0x3f);
          destination[written + 2] = 0x80 | ((c >> 6) & 0x3f);
          destination[written + 3] = 0x80 | (c & 0x3f);
        }
        read += c > 0xffff ? 2 : 1;
        written += size;
      }
      return { read, written };
    }
  }
  expose(TextEncoder, "TextEncoder");

  let nextTextStream = 1;
  class TextDecoder {
    #encoding;
    #fatal;
    #ignoreBOM;
    // The stream being decoded, 0 for none; whether its first code point
    // has been seen.
    #stream = 0;
    #bomSeen = false;

    constructor(label = "utf-8", options = undefined) {
      const given = str(label);
      const name = native.textDecoderName(given);
      if (name === null) {
        throw new $RangeError("Failed to construct 'TextDecoder': The encoding label provided ('" +
          given + "') is invalid.");
      }
      const read = dictionary(options, TEXT_DECODER_OPTIONS, "TextDecoder");
      this.#encoding = name;
      this.#fatal = read.fatal;
      this.#ignoreBOM = read.ignoreBOM;
    }

    get encoding() {
      return this.#encoding;
    }

    get fatal() {
      return this.#fatal;
    }

    get ignoreBOM() {
      return this.#ignoreBOM;
    }

    decode(input = undefined, options = undefined) {
      this.#encoding;
      let bytes = new $Uint8Array(0);
      if (input !== undefined) {
        bytes = bufferBytes(input);
        if (bytes === undefined) {
          throw new $TypeError(prefix("decode", "TextDecoder") +
            "parameter 1 is not of type 'ArrayBuffer'.");
        }
      }
      if (options !== undefined && options !== null && typeof options !== "object" &&
        typeof options !== "function") {
        throw new $TypeError(prefix("decode", "TextDecoder") +
          "The provided value is not of type 'TextDecodeOptions'.");
      }
      const stream = options === undefined || options === null ? false : !!options.stream;
      // The bytes as they are once the options are read: a getter of
      // theirs may have detached the buffer.
      if (input !== undefined) {
        bytes = bufferBytes(input) ?? new $Uint8Array(0);
      }
      if (this.#stream === 0) {
        this.#stream = nextTextStream++;
        this.#bomSeen = false;
      }
      const id = this.#stream;
      if (!stream) {
        this.#stream = 0;
      }
      let text = native.textDecode(id, this.#encoding, binaryString(bytes), this.#fatal, stream);
      if (text === null) {
        this.#stream = 0;
        throw new $TypeError(prefix("decode", "TextDecoder") + "The encoded data was not valid.");
      }
      // A byte order mark the stream begins with is dropped, unless asked
      // for.
      if (!this.#bomSeen && text.length > 0) {
        this.#bomSeen = true;
        const unicode = this.#encoding === "utf-8" || this.#encoding === "utf-16le" ||
          this.#encoding === "utf-16be";
        if (unicode && !this.#ignoreBOM && CharCodeAt(text, 0) === 0xfeff) {
          text = StringSlice(text, 1);
        }
      }
      return text;
    }
  }
  const TEXT_DECODER_OPTIONS = [
    ["fatal", "boolean", false, "TextDecoderOptions"],
    ["ignoreBOM", "boolean", false, "TextDecoderOptions"],
  ];
  expose(TextDecoder, "TextDecoder");

  // ---- The CSS namespace (CSSOM 9, gap batch A) ----

  // `supports()` asks the style crate as `@supports` does; `escape()`
  // serializes an identifier. A window's alone.
  const CSSNamespace = ObjectCreate(Object.prototype);
  defineProperty(CSSNamespace, toStringTag, { value: "CSS", configurable: true });
  defineProperty(CSSNamespace, "supports", {
    value: function supports(conditionText, value = undefined) {
      need(arguments.length, 1, "supports", "CSS");
      if (arguments.length > 1) {
        return native.cssSupports(str(conditionText), str(value));
      }
      return native.cssSupports(str(conditionText), null);
    },
    writable: true,
    enumerable: true,
    configurable: true,
  });
  defineProperty(CSSNamespace, "escape", {
    value: function escape(ident) {
      need(arguments.length, 1, "escape", "CSS");
      // A DOMString: lone surrogates kept, as they are not escaped.
      const text = toString(ident);
      let out = "";
      for (let i = 0; i < text.length; i++) {
        const c = CharCodeAt(text, i);
        const first = CharCodeAt(text, 0);
        if (c === 0) {
          out += "\ufffd";
        } else if ((c >= 0x1 && c <= 0x1f) || c === 0x7f ||
          (i === 0 && c >= 0x30 && c <= 0x39) ||
          (i === 1 && c >= 0x30 && c <= 0x39 && first === 0x2d)) {
          out += "\\" + NumberToString(c, 16) + " ";
        } else if (i === 0 && c === 0x2d && text.length === 1) {
          out += "\\-";
        } else if (c >= 0x80 || c === 0x2d || c === 0x5f || (c >= 0x30 && c <= 0x39) ||
          (c >= 0x41 && c <= 0x5a) || (c >= 0x61 && c <= 0x7a)) {
          out += text[i];
        } else {
          out += "\\" + text[i];
        }
      }
      return out;
    },
    writable: true,
    enumerable: true,
    configurable: true,
  });
  // `registerProperty()` (Properties and Values API section 3.1, D73): a
  // custom property registered for the document by the host, its
  // `PropertyDefinition` read in its members' order.
  defineProperty(CSSNamespace, "registerProperty", {
    value: function registerProperty(definition) {
      need(arguments.length, 1, "registerProperty", "CSS");
      if (definition !== undefined && definition !== null && typeof definition !== "object" &&
        typeof definition !== "function") {
        throw new $TypeError(prefix("registerProperty", "CSS") +
          "The provided value is not of type 'PropertyDefinition'.");
      }
      const member = (key) => (definition === undefined || definition === null ? undefined : definition[key]);
      const required = (key) => new $TypeError(prefix("registerProperty", "CSS") +
        "Failed to read the '" + key + "' property from 'PropertyDefinition': Required member is undefined.");
      const inherits = member("inherits");
      if (inherits === undefined) {
        throw required("inherits");
      }
      const initial = member("initialValue");
      const initialValue = initial === undefined ? undefined : str(initial);
      const named = member("name");
      if (named === undefined) {
        throw required("name");
      }
      const name = str(named);
      const given = member("syntax");
      const syntax = given === undefined ? "*" : str(given);
      let text = name + "\u0000" + syntax;
      if (initialValue !== undefined) {
        text += "\u0000" + initialValue;
      }
      try {
        native.cssom("register", null, [], text, inherits ? 1 : 0);
      } catch (error) {
        throw failed(error, "registerProperty", "CSS");
      }
    },
    writable: true,
    enumerable: true,
    configurable: true,
  });
  windowOnly("CSS", { value: CSSNamespace, writable: true, configurable: true });

  // ---- Mutation observers (DOM 4.3, gap batch A, D42) ----

  // An observer is the engine's by a number: it keeps the registrations,
  // matches the document's changes against them and queues the records,
  // and wakes the realm when one is queued, which then queues its
  // mutation observer microtask. An observer registered is kept, as its
  // nodes keep it.
  const mutationObservers = new $Map();
  let lastObserver = 0;
  let mutationsQueued = false;
  let observerCallback;

  class MutationRecord {
    #fields;

    constructor(key, fields) {
      if (key !== KEY) {
        throw illegalConstructor("MutationRecord");
      }
      this.#fields = fields;
    }

    get type() {
      return this.#fields.type;
    }

    get target() {
      return this.#fields.target;
    }

    get addedNodes() {
      return this.#fields.added;
    }

    get removedNodes() {
      return this.#fields.removed;
    }

    get previousSibling() {
      return this.#fields.previous;
    }

    get nextSibling() {
      return this.#fields.next;
    }

    get attributeName() {
      return this.#fields.attribute;
    }

    get attributeNamespace() {
      return this.#fields.namespace;
    }

    get oldValue() {
      return this.#fields.old;
    }
  }
  expose(MutationRecord, "MutationRecord");

  // The records the engine queued for observer `id`, made objects.
  function mutationRecords(id) {
    const given = JSONParse(native.mutationTake(id));
    const records = [];
    for (let i = 0; i < given.length; i++) {
      const r = given[i];
      ArrayPush(records, new MutationRecord(KEY, {
        type: r.type,
        target: wrap(r.target),
        added: snapshot(NodeList, r.added),
        removed: snapshot(NodeList, r.removed),
        previous: r.previous === null ? null : wrap(r.previous),
        next: r.next === null ? null : wrap(r.next),
        attribute: r.attribute,
        namespace: r.namespace,
        old: r.old,
      }));
    }
    return records;
  }

  const OBSERVE_WORDS = "Failed to execute 'observe' on 'MutationObserver': The options object ";

  class MutationObserver {
    #id;
    #callback;

    constructor(callback) {
      needToConstruct(arguments.length, 1, "MutationObserver");
      if (typeof callback !== "function") {
        throw new $TypeError("Failed to construct 'MutationObserver': The callback provided " +
          "as parameter 1 is not a function.");
      }
      this.#id = ++lastObserver;
      this.#callback = callback;
    }

    static {
      observerCallback = (observer) => observer.#callback;
    }

    observe(target, options = undefined) {
      this.#id;
      need(arguments.length, 1, "observe", "MutationObserver");
      if (!isNode(target)) {
        throw new $TypeError(prefix("observe", "MutationObserver") +
          "parameter 1 is not of type 'Node'.");
      }
      if (options !== undefined && options !== null && typeof options !== "object" &&
        typeof options !== "function") {
        throw new $TypeError(prefix("observe", "MutationObserver") +
          "The provided value is not of type 'MutationObserverInit'.");
      }
      // The members in Web IDL's order, those not given undefined.
      const given = options === undefined || options === null ? {} : options;
      let filter = given.attributeFilter;
      if (filter !== undefined) {
        filter = sequence(filter, "observe", "MutationObserver", str);
      }
      const read = (value) => value === undefined ? undefined : !!value;
      const attributeOldValue = read(given.attributeOldValue);
      let attributes = read(given.attributes);
      let characterData = read(given.characterData);
      const characterDataOldValue = read(given.characterDataOldValue);
      const childList = !!given.childList;
      const subtree = !!given.subtree;
      if ((attributeOldValue !== undefined || filter !== undefined) && attributes === undefined) {
        attributes = true;
      }
      if (characterDataOldValue !== undefined && characterData === undefined) {
        characterData = true;
      }
      // The reference checks these first, then that something is asked.
      if (attributeOldValue && !attributes) {
        throw new $TypeError(OBSERVE_WORDS + "may only set 'attributeOldValue' to true when " +
          "'attributes' is true or not present.");
      }
      if (filter !== undefined && !attributes) {
        throw new $TypeError(OBSERVE_WORDS + "may only set 'attributeFilter' when " +
          "'attributes' is true or not present.");
      }
      if (characterDataOldValue && !characterData) {
        throw new $TypeError(OBSERVE_WORDS + "may only set 'characterDataOldValue' to true when " +
          "'characterData' is true or not present.");
      }
      if (!childList && !attributes && !characterData) {
        throw new $TypeError(OBSERVE_WORDS + "must set at least one of 'attributes', " +
          "'characterData', or 'childList' to true.");
      }
      const flags = (childList ? 1 : 0) | (attributes ? 2 : 0) | (characterData ? 4 : 0) |
        (subtree ? 8 : 0) | (attributeOldValue ? 16 : 0) | (characterDataOldValue ? 32 : 0);
      MapSet(mutationObservers, this.#id, this);
      native.mutationObserve(this.#id, handleOf(target), flags, filter === undefined ? null : filter);
    }

    disconnect() {
      native.mutationDisconnect(this.#id);
      MapDelete(mutationObservers, this.#id);
    }

    takeRecords() {
      return mutationRecords(this.#id);
    }
  }
  expose(MutationObserver, "MutationObserver");

  // The slots signaled (DOM's signal slots), their objects held until
  // their `slotchange`: the list holds them, so a slot of a tree out of
  // the document, which only its own listeners hold, is not let go of
  // before the event. Taken from the host as soon as it signals them.
  let signaledSlots = [];

  // Whether the slots and toggles are being taken: each native call
  // made then tells of what is still waiting, which the taking under way
  // takes.
  let takingSignals = false;

  function holdSignaledSlots() {
    if (takingSignals) {
      return;
    }
    takingSignals = true;
    try {
      // The `details` toggled, taken as promptly (gap batch H).
      takeDetailsToggles();
      const slots = native.takeSlotChanges();
      for (let i = 0; i < slots.length; i++) {
        const slot = wrap(slots[i]);
        if (!ArrayIncludes(signaledSlots, slot)) {
          ArrayPush(signaledSlots, slot);
        }
      }
    } finally {
      takingSignals = false;
    }
  }

  // DOM's "notify mutation observers": each observer with records, in
  // the order they were made, its callback given them; then a
  // `slotchange` at each slot signaled (D68).
  function notifyMutationObservers() {
    mutationsQueued = false;
    holdSignaledSlots();
    const slots = signaledSlots;
    signaledSlots = [];
    const ids = native.mutationNotify();
    for (let i = 0; i < ids.length; i++) {
      const observer = MapGet(mutationObservers, ids[i]);
      const records = mutationRecords(ids[i]);
      if (observer === undefined || records.length === 0) {
        continue;
      }
      try {
        ReflectApply(observerCallback(observer), observer, [records, observer]);
      } catch (error) {
        reportException(error);
      }
    }
    for (let i = 0; i < slots.length; i++) {
      const event = new Event("slotchange", made(EVENT_INIT, ["bubbles", true]));
      eventState(event).trusted = true;
      drain(dispatching(slots[i], event));
    }
  }

  // The engine queued a record or signaled a slot: the microtask that
  // tells the observers, unless one is queued already.
  function queueMutationObserverMicrotask() {
    holdSignaledSlots();
    if (!mutationsQueued) {
      mutationsQueued = true;
      PromiseThen(resolved(undefined), notifyMutationObservers);
    }
  }

  // The engine's `mutations` task: records of the parser's insertions.
  function* mutationsTask() {
    notifyMutationObservers();
    return false;
  }

  // ---- Resize and intersection observers (gap batch A, D45) ----

  // An observer is the engine's by a number: each frame, after layout,
  // the engine measures what the observers watch and gives the realm the
  // entries that makes as a task (`observations`). An observer watching
  // an element is kept, as the element keeps it.
  const ArrayIndexOf = uncurry(Array.prototype.indexOf);
  const isElementNode = (value) => isNode(value) && kindOf(value) === 1;
  const resizeObservers = new $Map();
  const intersectionObservers = new $Map();
  let lastLayoutObserver = 0;
  let resizeCallback;
  let intersectionCallback;

  class ResizeObserverSize {
    #inline;
    #block;

    constructor(key, inline, block) {
      if (key !== KEY) {
        throw illegalConstructor("ResizeObserverSize");
      }
      this.#inline = inline;
      this.#block = block;
    }

    get inlineSize() {
      return this.#inline;
    }

    get blockSize() {
      return this.#block;
    }
  }
  expose(ResizeObserverSize, "ResizeObserverSize");

  class ResizeObserverEntry {
    #fields;

    constructor(key, fields) {
      if (key !== KEY) {
        throw illegalConstructor("ResizeObserverEntry");
      }
      this.#fields = fields;
    }

    get target() {
      return this.#fields.target;
    }

    get contentRect() {
      return this.#fields.contentRect;
    }

    get borderBoxSize() {
      return this.#fields.borderBoxSize;
    }

    get contentBoxSize() {
      return this.#fields.contentBoxSize;
    }

    get devicePixelContentBoxSize() {
      return this.#fields.devicePixelContentBoxSize;
    }
  }
  expose(ResizeObserverEntry, "ResizeObserverEntry");

  const RESIZE_BOXES = ["content-box", "border-box", "device-pixel-content-box"];

  class ResizeObserver {
    #id;
    #callback;

    constructor(callback) {
      needToConstruct(arguments.length, 1, "ResizeObserver");
      if (typeof callback !== "function") {
        throw new $TypeError("Failed to construct 'ResizeObserver': The callback provided as " +
          "parameter 1 is not a function.");
      }
      this.#id = ++lastLayoutObserver;
      this.#callback = callback;
    }

    static {
      resizeCallback = (observer) => observer.#callback;
    }

    observe(target, options = undefined) {
      this.#id;
      need(arguments.length, 1, "observe", "ResizeObserver");
      if (!isElementNode(target)) {
        throw new $TypeError(prefix("observe", "ResizeObserver") +
          "parameter 1 is not of type 'Element'.");
      }
      const given = options === undefined || options === null ? undefined : options.box;
      const box = given === undefined ? "content-box" : str(given);
      const observed = ArrayIndexOf(RESIZE_BOXES, box);
      if (observed < 0) {
        throw new $TypeError(prefix("observe", "ResizeObserver") + "Failed to read the 'box' " +
          "property from 'ResizeObserverOptions': The provided value '" + box +
          "' is not a valid enum value of type ResizeObserverBoxOptions.");
      }
      MapSet(resizeObservers, this.#id, this);
      native.resizeObserve(this.#id, handleOf(target), observed);
    }

    unobserve(target) {
      this.#id;
      need(arguments.length, 1, "unobserve", "ResizeObserver");
      if (!isElementNode(target)) {
        throw new $TypeError(prefix("unobserve", "ResizeObserver") +
          "parameter 1 is not of type 'Element'.");
      }
      native.resizeUnobserve(this.#id, handleOf(target));
    }

    disconnect() {
      native.resizeUnobserve(this.#id, null);
      MapDelete(resizeObservers, this.#id);
    }
  }
  expose(ResizeObserver, "ResizeObserver");

  class IntersectionObserverEntry {
    #fields;

    constructor(key, fields) {
      if (key !== KEY) {
        throw illegalConstructor("IntersectionObserverEntry");
      }
      this.#fields = fields;
    }

    get time() {
      return this.#fields.time;
    }

    get rootBounds() {
      return this.#fields.rootBounds;
    }

    get boundingClientRect() {
      return this.#fields.boundingClientRect;
    }

    get intersectionRect() {
      return this.#fields.intersectionRect;
    }

    get isIntersecting() {
      return this.#fields.isIntersecting;
    }

    get isVisible() {
      return false;
    }

    get intersectionRatio() {
      return this.#fields.intersectionRatio;
    }

    get target() {
      return this.#fields.target;
    }
  }
  expose(IntersectionObserverEntry, "IntersectionObserverEntry");

  // CSS's absolute lengths, by their units, in pixels.
  const ABSOLUTE_UNITS = [
    ["px", 1], ["cm", 96 / 2.54], ["mm", 96 / 25.4], ["in", 96], ["pt", 4 / 3], ["pc", 16],
    ["q", 96 / 101.6],
  ];

  // `rootMargin` or `scrollMargin`: one to four absolute lengths or
  // percentages, each side as a value in pixels or percent and whether
  // it is a percentage; null for anything else.
  function parseMargin(text) {
    const parts = [];
    let word = "";
    for (let i = 0; i <= text.length; i++) {
      const c = i < text.length ? CharCodeAt(text, i) : 32;
      if (c === 32 || c === 9 || c === 10 || c === 12 || c === 13) {
        if (word.length > 0) {
          ArrayPush(parts, word);
          word = "";
        }
      } else {
        word += text[i];
      }
    }
    if (parts.length === 0) {
      ArrayPush(parts, "0px");
    }
    if (parts.length > 4) {
      return null;
    }
    const sides = [];
    for (let i = 0; i < parts.length; i++) {
      const lower = ToLowerCase(parts[i]);
      const percent = StringEndsWith(lower, "%");
      // A percentage, or an absolute length in pixels.
      let unit = percent ? 1 : 0;
      let scale = 1;
      for (let j = 0; j < ABSOLUTE_UNITS.length && unit === 0; j++) {
        if (StringEndsWith(lower, ABSOLUTE_UNITS[j][0])) {
          unit = ABSOLUTE_UNITS[j][0].length;
          scale = ABSOLUTE_UNITS[j][1];
        }
      }
      const digits = StringSlice(lower, 0, lower.length - unit);
      const value = $Number(digits);
      if (digits === "" || value !== value || (unit === 0 && value !== 0)) {
        return null;
      }
      // Another unit's pixels whole, as the reference browser keeps them.
      ArrayPush(sides, [scale === 1 ? value : MathTrunc(value * scale), percent]);
    }
    // CSS's margin shorthand: top, right, bottom, left.
    const top = sides[0];
    const right = sides.length > 1 ? sides[1] : top;
    const bottom = sides.length > 2 ? sides[2] : top;
    const left = sides.length > 3 ? sides[3] : right;
    return [top, right, bottom, left];
  }

  function serializeMargin(sides) {
    let out = "";
    for (let i = 0; i < 4; i++) {
      out += (i > 0 ? " " : "") + sides[i][0] + (sides[i][1] ? "%" : "px");
    }
    return out;
  }

  class IntersectionObserver {
    #id;
    #callback;
    #root;
    #margin;
    #scrollMargin;
    #thresholds;

    constructor(callback, options = undefined) {
      needToConstruct(arguments.length, 1, "IntersectionObserver");
      if (typeof callback !== "function") {
        throw new $TypeError("Failed to construct 'IntersectionObserver': The callback " +
          "provided as parameter 1 is not a function.");
      }
      const words = "Failed to construct 'IntersectionObserver': ";
      const given = options === undefined || options === null ? {} : options;
      const root = given.root === undefined ? null : given.root;
      if (root !== null && !isElementNode(root) && !(isNode(root) && kindOf(root) === 9)) {
        throw new $TypeError(words + "Failed to read the 'root' property from " +
          "'IntersectionObserverInit': The provided value is not of type '(Document or Element)'.");
      }
      const margin = parseMargin(given.rootMargin === undefined ? "0px" : str(given.rootMargin));
      if (margin === null) {
        throw new $DOMException(words + "rootMargin must be specified in absolute length " +
          "units or percent.", "SyntaxError");
      }
      const scrollMargin = parseMargin(given.scrollMargin === undefined ? "0px" : str(given.scrollMargin));
      if (scrollMargin === null) {
        throw new $DOMException(words + "scrollMargin must be specified in absolute length " +
          "units or percent.", "SyntaxError");
      }
      let thresholds = given.threshold === undefined ? [0] : given.threshold;
      thresholds = typeof thresholds === "object" && thresholds !== null
        ? sequence(thresholds, "", "IntersectionObserver", $Number)
        : [$Number(thresholds)];
      for (let i = 0; i < thresholds.length; i++) {
        const t = thresholds[i];
        if (!(t >= 0 && t <= 1)) {
          throw new $RangeError(words + "Threshold values must be numbers between 0 and 1");
        }
      }
      ArraySort(thresholds, (a, b) => a - b);
      if (thresholds.length === 0) {
        ArrayPush(thresholds, 0);
      }
      this.#id = ++lastLayoutObserver;
      this.#callback = callback;
      this.#root = root;
      this.#margin = margin;
      this.#scrollMargin = scrollMargin;
      this.#thresholds = ObjectFreeze(thresholds);
    }

    static {
      intersectionCallback = (observer) => observer.#callback;
    }

    get root() {
      return this.#root;
    }

    get rootMargin() {
      return serializeMargin(this.#margin);
    }

    get scrollMargin() {
      return serializeMargin(this.#scrollMargin);
    }

    get thresholds() {
      return this.#thresholds;
    }

    get delay() {
      this.#id;
      return 0;
    }

    get trackVisibility() {
      this.#id;
      return false;
    }

    observe(target) {
      this.#id;
      need(arguments.length, 1, "observe", "IntersectionObserver");
      if (!isElementNode(target)) {
        throw new $TypeError(prefix("observe", "IntersectionObserver") +
          "parameter 1 is not of type 'Element'.");
      }
      // A document root is the viewport's, as the implicit root is.
      const root = this.#root !== null && isElementNode(this.#root) ? handleOf(this.#root) : null;
      const margin = [];
      for (let i = 0; i < 4; i++) {
        ArrayPush(margin, this.#margin[i][0], this.#margin[i][1] ? 1 : 0);
      }
      MapSet(intersectionObservers, this.#id, this);
      native.intersectionObserve(this.#id, handleOf(target), root, margin,
        ArraySlice(this.#thresholds, 0));
    }

    unobserve(target) {
      this.#id;
      need(arguments.length, 1, "unobserve", "IntersectionObserver");
      if (!isElementNode(target)) {
        throw new $TypeError(prefix("unobserve", "IntersectionObserver") +
          "parameter 1 is not of type 'Element'.");
      }
      native.intersectionUnobserve(this.#id, handleOf(target));
    }

    disconnect() {
      native.intersectionUnobserve(this.#id, null);
      MapDelete(intersectionObservers, this.#id);
    }

    // The engine gives the entries as the frame makes them: none wait.
    takeRecords() {
      this.#id;
      return [];
    }
  }
  expose(IntersectionObserver, "IntersectionObserver");

  function observedRect(r) {
    return new DOMRectReadOnly(r[0], r[1], r[2], r[3]);
  }

  // The engine's `observations` task: a frame's entries, each observer's
  // callback called with its own, a microtask checkpoint after each.
  function* observationsTask(text) {
    const told = JSONParse(text);
    for (let i = 0; i < told.resize.length; i++) {
      const group = told.resize[i];
      const observer = MapGet(resizeObservers, group.observer);
      if (observer === undefined) {
        continue;
      }
      const entries = [];
      for (let j = 0; j < group.entries.length; j++) {
        const e = group.entries[j];
        const size = (pair) => ObjectFreeze([new ResizeObserverSize(KEY, pair[0], pair[1])]);
        ArrayPush(entries, new ResizeObserverEntry(KEY, {
          target: wrap(e.target),
          contentRect: observedRect(e.content),
          contentBoxSize: size([e.content[2], e.content[3]]),
          borderBoxSize: size(e.border),
          devicePixelContentBoxSize: size(e.device),
        }));
      }
      try {
        ReflectApply(resizeCallback(observer), observer, [entries, observer]);
      } catch (error) {
        reportException(error);
      }
      yield;
    }
    for (let i = 0; i < told.intersection.length; i++) {
      const group = told.intersection[i];
      const observer = MapGet(intersectionObservers, group.observer);
      if (observer === undefined) {
        continue;
      }
      const entries = [];
      for (let j = 0; j < group.entries.length; j++) {
        const e = group.entries[j];
        ArrayPush(entries, new IntersectionObserverEntry(KEY, {
          time: e.time,
          rootBounds: e.root === null ? null : observedRect(e.root),
          boundingClientRect: observedRect(e.bounds),
          intersectionRect: observedRect(e.rect),
          isIntersecting: e.intersecting,
          intersectionRatio: e.ratio,
          target: wrap(e.target),
        }));
      }
      try {
        ReflectApply(intersectionCallback(observer), observer, [entries, observer]);
      } catch (error) {
        reportException(error);
      }
      yield;
    }
    return false;
  }

  // ---- BroadcastChannel (HTML 9.6, gap batch A, D46) ----

  // A message goes to every other channel of the name and storage key in
  // the browser: through the engine's context in one process, and the
  // browser between renderers. A channel is kept while it is open, as
  // the standard keeps one that can still hear.
  const openChannels = new $Map();
  let channelOf;
  // The realm's channels in the order they were made, and the clock of
  // that order at each post of its own still to come back by sender: a
  // message goes to the channels there were when it was posted.
  let channelClock = 0;
  const postClocks = new $Map();

  class BroadcastChannel extends EventTarget {
    #name;
    #id;
    #made = ++channelClock;
    #closed = false;

    constructor(name) {
      needToConstruct(arguments.length, 1, "BroadcastChannel");
      super();
      this.#name = str(name);
      this.#id = native.broadcastId();
      MapSet(openChannels, this, true);
    }

    static {
      channelOf = (channel) => [channel.#name, channel.#id, channel.#closed, channel.#made];
    }

    get name() {
      return this.#name;
    }

    postMessage(message) {
      need(arguments.length, 1, "postMessage", "BroadcastChannel");
      if (this.#closed) {
        throw new $DOMException(prefix("postMessage", "BroadcastChannel") + "Channel is closed",
          "InvalidStateError");
      }
      const record = serialize(message, [], prefix("postMessage", "BroadcastChannel"), null);
      native.broadcastPost(this.#name, wire(record), this.#id);
      let clocks = MapGet(postClocks, this.#id);
      if (clocks === undefined) {
        clocks = [];
        MapSet(postClocks, this.#id, clocks);
      }
      ArrayPush(clocks, channelClock);
    }

    close() {
      this.#closed = true;
      MapDelete(openChannels, this);
    }
  }
  defineHandlers(BroadcastChannel.prototype, ["message", "messageerror"], false);
  expose(BroadcastChannel, "BroadcastChannel");

  // The engine's `broadcast` task: a `message` event at each open channel
  // of `name` but the one that sent it, in the order they were made.
  function* broadcastTask(name, text, sender, origin) {
    // Posted here: to the channels made by then (another realm's, to all).
    let posted = Infinity;
    const clocks = MapGet(postClocks, sender);
    if (clocks !== undefined) {
      posted = ArrayShift(clocks);
      if (clocks.length === 0) {
        MapDelete(postClocks, sender);
      }
    }
    const channels = [];
    MapForEach(openChannels, (_, channel) => ArrayPush(channels, channel));
    for (let i = 0; i < channels.length; i++) {
      const [own, id, closed, made] = channelOf(channels[i]);
      if (own === name && id !== sender && !closed && made <= posted) {
        yield* messageEvent(channels[i], text, origin);
      }
    }
    return false;
  }

  // ---- matchMedia() and MediaQueryList (CSSOM View 4.2, gap batch A) ----

  // A list is the engine's by a number: parsed once, answered against the
  // window's medium when `matches` is read, and evaluated again in each
  // frame's update, which fires `change` at those whose answer changed.
  // An object let go, the engine forgets its list.
  const mediaLists = new $Map();
  const mediaGone = new $FinalizationRegistry((id) => {
    MapDelete(mediaLists, id);
    native.mediaForget(id);
  });
  const EventTargetAdd = uncurry(EventTarget.prototype.addEventListener);
  const EventTargetRemove = uncurry(EventTarget.prototype.removeEventListener);

  class MediaQueryList extends EventTarget {
    #id;
    #media;

    constructor(key, id, media) {
      if (key !== KEY) {
        throw illegalConstructor("MediaQueryList");
      }
      super();
      this.#id = id;
      this.#media = media;
    }

    get media() {
      return this.#media;
    }

    get matches() {
      return native.mediaMatches(this.#id);
    }

    addListener(callback) {
      this.#id;
      need(arguments.length, 1, "addListener", "MediaQueryList");
      if (callback !== null) {
        EventTargetAdd(this, "change", callback);
      }
    }

    removeListener(callback) {
      this.#id;
      need(arguments.length, 1, "removeListener", "MediaQueryList");
      if (callback !== null) {
        EventTargetRemove(this, "change", callback);
      }
    }
  }
  defineHandlers(MediaQueryList.prototype, ["change"], false);
  expose(MediaQueryList, "MediaQueryList");

  const MEDIA_QUERY = 524288;
  const MEDIA_QUERY_INIT = members(EVENT_INIT, [
    ["matches", "boolean", false],
    ["media", "DOMString", ""],
  ], "MediaQueryListEventInit");

  class MediaQueryListEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, MEDIA_QUERY_INIT,
        "MediaQueryListEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= MEDIA_QUERY;
    }

    get media() {
      return fieldsOf(this, MEDIA_QUERY).media;
    }

    get matches() {
      return fieldsOf(this, MEDIA_QUERY).matches;
    }
  }
  expose(MediaQueryListEvent, "MediaQueryListEvent");

  windowOnly("matchMedia", {
    value: function matchMedia(query) {
      need(arguments.length, 1, "matchMedia", "Window");
      // Its serialization and its number.
      const made = native.matchMedia(str(query));
      const id = $Number(made[1]);
      const list = new MediaQueryList(KEY, id, made[0]);
      MapSet(mediaLists, id, new $WeakRef(list));
      Register(mediaGone, list, id);
      return list;
    },
    writable: true,
    enumerable: true,
    configurable: true,
  });

  // The frame's report (the engine's `mediaChanged` task): a `change`
  // at each list whose answer changed, in the order they were made.
  function* mediaChanged(ids) {
    for (let i = 0; i < ids.length; i++) {
      const ref = MapGet(mediaLists, ids[i]);
      const list = ref === undefined ? undefined : WeakRefDeref(ref);
      if (list === undefined) {
        continue;
      }
      const event = new MediaQueryListEvent("change", { media: list.media, matches: list.matches });
      eventState(event).trusted = true;
      yield* dispatching(list, event);
    }
    return false;
  }

  // ---- requestIdleCallback() (Cooperative Scheduling, gap batch A) ----

  // The callbacks wait for an idle period: a task of the bindings' own,
  // queued when the first is asked for, that runs those asked for while
  // its deadline lasts (the engine's next timer, the next frame, or 50
  // ms on). A callback whose timeout passes first runs as a task of its
  // own, `didTimeout` true.
  const idleCallbacks = new $Map();
  let lastIdle = 0;
  let idleQueued = false;

  class IdleDeadline {
    #end;
    #timedOut;

    constructor(key, end, timedOut) {
      if (key !== KEY) {
        throw illegalConstructor("IdleDeadline");
      }
      this.#end = end;
      this.#timedOut = timedOut;
    }

    timeRemaining() {
      // At most the 50 ms an idle period lasts, whatever the rounding.
      const left = this.#end - native.now();
      return left > 0 ? MathMin(left, 50) : 0;
    }

    get didTimeout() {
      return this.#timedOut;
    }
  }
  expose(IdleDeadline, "IdleDeadline");

  function queueIdle() {
    if (!idleQueued) {
      idleQueued = true;
      queueTask(runIdle, 0);
    }
  }

  function runIdle() {
    idleQueued = false;
    const end = native.idleDeadline();
    const ids = [];
    MapForEach(idleCallbacks, (_, id) => ArrayPush(ids, id));
    for (let i = 0; i < ids.length; i++) {
      if (native.now() >= end) {
        break;
      }
      const entry = MapGet(idleCallbacks, ids[i]);
      if (entry === undefined) {
        continue;
      }
      MapDelete(idleCallbacks, ids[i]);
      if (entry.timer !== 0) {
        cancelTask(entry.timer);
      }
      try {
        ReflectApply(entry.callback, global, [new IdleDeadline(KEY, end, false)]);
      } catch (error) {
        reportException(error);
      }
    }
    if (MapSize(idleCallbacks) > 0) {
      queueIdle();
    }
  }

  const IDLE_OPTIONS = [["timeout", "unsigned long", 0, "IdleRequestOptions"]];
  windowOnly("requestIdleCallback", {
    value: function requestIdleCallback(callback, options = undefined) {
      need(arguments.length, 1, "requestIdleCallback", "Window");
      if (typeof callback !== "function") {
        throw new $TypeError(prefix("requestIdleCallback", "Window") +
          "The callback provided as parameter 1 is not a function.");
      }
      const read = dictionary(options, IDLE_OPTIONS, "IdleRequestOptions");
      const id = ++lastIdle;
      const entry = { callback, timer: 0 };
      if (read.timeout > 0) {
        entry.timer = queueTask(() => {
          if (MapGet(idleCallbacks, id) !== entry) {
            return;
          }
          MapDelete(idleCallbacks, id);
          ReflectApply(callback, global, [new IdleDeadline(KEY, -Infinity, true)]);
        }, read.timeout);
      }
      MapSet(idleCallbacks, id, entry);
      queueIdle();
      return id;
    },
    writable: true,
    enumerable: true,
    configurable: true,
  });
  windowOnly("cancelIdleCallback", {
    value: function cancelIdleCallback(handle) {
      need(arguments.length, 1, "cancelIdleCallback", "Window");
      const id = handle >>> 0;
      const entry = MapGet(idleCallbacks, id);
      if (entry !== undefined) {
        MapDelete(idleCallbacks, id);
        if (entry.timer !== 0) {
          cancelTask(entry.timer);
        }
      }
    },
    writable: true,
    enumerable: true,
    configurable: true,
  });

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
        const url = native.resolveUrlUtf8(text);
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
          const url = native.resolveUrlUtf8(referrer);
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
    get destination() { return this.#state.destination || ""; }
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
      const parsed = native.resolveUrlUtf8(text);
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
          state.referrer, state.referrerPolicy, state.targetAddressSpace, state.integrity,
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

  // ---- The Cache API (Service Workers 5, M39) ----

  // A request as the engine's store takes it.
  function cacheRequestOf(request) {
    const state = requestState(request);
    return {
      method: state.method, url: state.url,
      headers: cloneList(headersState(state.headers).list),
    };
  }

  // The request a `RequestInfo` names, as `method` of `iface` reads it.
  function cacheRequest(info, method, iface) {
    if (isRequest(info)) return info;
    try {
      return new Request(info);
    } catch (e) {
      const was = "Failed to construct 'Request': ";
      if ((e instanceof $TypeError) && StringSlice(e.message, 0, was.length) === was) {
        e.message = prefix(method, iface) + StringSlice(e.message, was.length);
      }
      throw e;
    }
  }

  // A `CacheQueryOptions` dictionary.
  function cacheOptions(options) {
    const given = options === undefined || options === null ? {} : options;
    return {
      ignoreSearch: !!given.ignoreSearch,
      ignoreMethod: !!given.ignoreMethod,
      ignoreVary: !!given.ignoreVary,
    };
  }

  // Asks the engine's store `op`, the bodies in `bodies` appended after
  // one another and named in `op` by offset and length: the answer's
  // result, and the bytes its bodies are in. Throws the answer's error.
  function cacheCall(op, bodies, method, iface) {
    let total = 0;
    for (let i = 0; i < bodies.length; i++) total += viewLength(bodies[i]);
    const all = new $Uint8Array(total);
    let at = 0;
    for (let i = 0; i < bodies.length; i++) {
      TypedArraySet(all, bodies[i], at);
      at += viewLength(bodies[i]);
    }
    let pair;
    try {
      pair = native.caches(JSONStringify(op), total === 0 ? null : binaryString(all));
    } catch (e) {
      throw failed(e, method, iface);
    }
    const answer = JSONParse(pair[0]);
    if (answer.error !== undefined) {
      const error = answer.error;
      if (error.name === "TypeError") throw new $TypeError(prefix(method, iface) + error.message);
      throw new $DOMException(prefix(method, iface) + error.message, error.name);
    }
    return { result: answer.result, bytes: pair[1] };
  }

  // A response of the store's, with its body.
  function cachedResponse(json, bytes) {
    const at = json.body[0];
    const length = json.body[1];
    return new Response(KEY, {
      type: json.type, urlList: json.urls, status: json.status, statusText: json.statusText,
      headers: makeHeaders("immutable", json.headers),
      body: bodyFrom(TypedArraySlice(new $Uint8Array(bytes), at, at + length)),
    });
  }

  // A request of the store's.
  function cachedRequest(json) {
    return new Request(KEY, {
      method: json.method, url: json.url, headers: makeHeaders("request", json.headers),
      body: bodyFrom(null), mode: "cors", credentials: "same-origin", cache: "default",
      redirect: "follow", referrer: "about:client", referrerPolicy: "", integrity: "",
      keepalive: false, duplex: "half", signal: new AbortSignal(KEY), targetAddressSpace: "",
    });
  }

  // Whether a `Vary` value names `*` among its comma-separated names.
  function varyHasStar(value) {
    let start = 0;
    while (start <= value.length) {
      let end = StringIndexOf(value, ",", start);
      if (end < 0) end = value.length;
      let a = start;
      let b = end;
      while (a < b && (CharCodeAt(value, a) === 0x20 || CharCodeAt(value, a) === 0x09)) a++;
      while (b > a && (CharCodeAt(value, b - 1) === 0x20 || CharCodeAt(value, b - 1) === 0x09)) b--;
      if (StringSlice(value, a, b) === "*") return true;
      start = end + 1;
    }
    return false;
  }

  // The checks `put` and `addAll` make of a request and of the response
  // it is to be kept with (Service Workers 5.4.6), thrown as `method`
  // of `Cache` words them.
  function checkCacheRequest(request, method) {
    const state = requestState(request);
    const scheme = StringSlice(state.url, 0, StringIndexOf(state.url, ":"));
    if (scheme !== "http" && scheme !== "https") {
      throw new $TypeError(prefix(method, "Cache") + "Request scheme '" + scheme + "' is unsupported");
    }
    if (state.method !== "GET") {
      throw new $TypeError(prefix(method, "Cache") + "Request method '" + state.method + "' is unsupported");
    }
  }

  function checkCacheResponse(response, method) {
    const state = responseState(response);
    if (state.status === 206) {
      throw new $TypeError(prefix(method, "Cache") + "Partial response (status code 206) is unsupported");
    }
    const vary = combined(headersState(state.headers).list, "vary");
    if (vary !== null && varyHasStar(vary)) {
      throw new $TypeError(prefix(method, "Cache") + "Vary header contains *");
    }
    if (state.body.bytes !== null && state.body.used) {
      throw new $TypeError(prefix(method, "Cache") + "Response body is already used");
    }
  }

  // A put of a batch, the response's body taken.
  function cachePut(request, response, bodies, offset) {
    const state = responseState(response);
    const bytes = state.body.bytes === null ? new $Uint8Array(0) : state.body.bytes;
    if (state.body.bytes !== null) state.body.used = true;
    ArrayPush(bodies, bytes);
    return {
      put: {
        request: cacheRequestOf(request),
        response: {
          type: state.type, urls: ArraySlice(state.urlList), status: state.status,
          statusText: state.statusText, headers: cloneList(headersState(state.headers).list),
          body: [offset, viewLength(bytes)],
        },
      },
    };
  }

  let cacheNumber;
  class Cache {
    #cache;

    constructor(key, number) {
      if (key !== KEY) throw new $TypeError("Illegal constructor");
      this.#cache = number;
    }

    static {
      cacheNumber = (cache) => cache.#cache;
    }

    match(request, options) {
      try {
        need(arguments.length, 1, "match", "Cache");
        const query = cacheRequestOf(cacheRequest(request, "match", "Cache"));
        const { result, bytes } = cacheCall(
          { op: "match", cache: this.#cache, request: query, options: cacheOptions(options), all: false },
          [], "match", "Cache",
        );
        return resolved(result.length === 0 ? undefined : cachedResponse(result[0], bytes));
      } catch (e) {
        return rejected(e);
      }
    }

    matchAll(request, options) {
      try {
        const query = request === undefined ? null : cacheRequestOf(cacheRequest(request, "matchAll", "Cache"));
        const { result, bytes } = cacheCall(
          { op: "match", cache: this.#cache, request: query, options: cacheOptions(options), all: true },
          [], "matchAll", "Cache",
        );
        const out = [];
        for (let i = 0; i < result.length; i++) ArrayPush(out, cachedResponse(result[i], bytes));
        return resolved(out);
      } catch (e) {
        return rejected(e);
      }
    }

    add(request) {
      try {
        need(arguments.length, 1, "add", "Cache");
        return this.#addAll([cacheRequest(request, "add", "Cache")], "add");
      } catch (e) {
        return rejected(e);
      }
    }

    addAll(requests) {
      try {
        need(arguments.length, 1, "addAll", "Cache");
        const list = [];
        for (const request of requests) ArrayPush(list, cacheRequest(request, "addAll", "Cache"));
        return this.#addAll(list, "addAll");
      } catch (e) {
        return rejected(e);
      }
    }

    // Add All (Service Workers 5.4.4): each request fetched, then all
    // put in one batch, or none.
    #addAll(requests, method) {
      const seen = [];
      for (let i = 0; i < requests.length; i++) {
        checkCacheRequest(requests[i], method);
        const url = requestState(requests[i]).url;
        if (ArrayIncludes(seen, url)) {
          throw new $DOMException(
            prefix(method, "Cache") + "Cache.addAll(): duplicate requests (" + url + ")",
            "InvalidStateError",
          );
        }
        ArrayPush(seen, url);
      }
      const cache = this.#cache;
      const fetching = [];
      for (let i = 0; i < requests.length; i++) ArrayPush(fetching, fetch(requests[i]));
      return $Promise.all(fetching).then(
        (responses) => {
          const ops = [];
          const bodies = [];
          let offset = 0;
          for (let i = 0; i < responses.length; i++) {
            const state = responseState(responses[i]);
            if (state.type === "error" || state.status < 200 || state.status > 299) {
              throw new $TypeError(prefix(method, "Cache") + "Request failed");
            }
            checkCacheResponse(responses[i], method);
            const op = cachePut(requests[i], responses[i], bodies, offset);
            offset += op.put.response.body[1];
            ArrayPush(ops, op);
          }
          cacheCall({ op: "batch", cache, ops }, bodies, method, "Cache");
          return undefined;
        },
        () => {
          throw new $TypeError(prefix(method, "Cache") + "Request failed");
        },
      );
    }

    put(request, response) {
      try {
        need(arguments.length, 2, "put", "Cache");
        const key = cacheRequest(request, "put", "Cache");
        if (!isResponse(response)) {
          throw new $TypeError(prefix("put", "Cache") + "parameter 2 is not of type 'Response'.");
        }
        checkCacheRequest(key, "put");
        checkCacheResponse(response, "put");
        const bodies = [];
        const op = cachePut(key, response, bodies, 0);
        cacheCall({ op: "batch", cache: this.#cache, ops: [op] }, bodies, "put", "Cache");
        return resolved(undefined);
      } catch (e) {
        return rejected(e);
      }
    }

    delete(request, options) {
      try {
        need(arguments.length, 1, "delete", "Cache");
        const query = cacheRequestOf(cacheRequest(request, "delete", "Cache"));
        const { result } = cacheCall(
          { op: "batch", cache: this.#cache, ops: [{ delete: { request: query, options: cacheOptions(options) } }] },
          [], "delete", "Cache",
        );
        return resolved(result[0]);
      } catch (e) {
        return rejected(e);
      }
    }

    keys(request, options) {
      try {
        const query = request === undefined ? null : cacheRequestOf(cacheRequest(request, "keys", "Cache"));
        const { result } = cacheCall(
          { op: "requests", cache: this.#cache, request: query, options: cacheOptions(options) },
          [], "keys", "Cache",
        );
        const out = [];
        for (let i = 0; i < result.length; i++) ArrayPush(out, cachedRequest(result[i]));
        return resolved(out);
      } catch (e) {
        return rejected(e);
      }
    }
  }
  expose(Cache, "Cache");

  class CacheStorage {
    constructor(key) {
      if (key !== KEY) throw new $TypeError("Illegal constructor");
    }

    match(request, options) {
      try {
        need(arguments.length, 1, "match", "CacheStorage");
        const query = cacheRequestOf(cacheRequest(request, "match", "CacheStorage"));
        const given = options === undefined || options === null ? {} : options;
        const name = given.cacheName === undefined ? null : str(given.cacheName);
        const { result, bytes } = cacheCall(
          { op: "matchAny", request: query, options: cacheOptions(options), name },
          [], "match", "CacheStorage",
        );
        return resolved(result === null ? undefined : cachedResponse(result, bytes));
      } catch (e) {
        return rejected(e);
      }
    }

    has(name) {
      try {
        need(arguments.length, 1, "has", "CacheStorage");
        return resolved(cacheCall({ op: "has", name: str(name) }, [], "has", "CacheStorage").result);
      } catch (e) {
        return rejected(e);
      }
    }

    open(name) {
      try {
        need(arguments.length, 1, "open", "CacheStorage");
        const number = cacheCall({ op: "open", name: str(name) }, [], "open", "CacheStorage").result;
        return resolved(new Cache(KEY, number));
      } catch (e) {
        return rejected(e);
      }
    }

    delete(name) {
      try {
        need(arguments.length, 1, "delete", "CacheStorage");
        return resolved(cacheCall({ op: "delete", name: str(name) }, [], "delete", "CacheStorage").result);
      } catch (e) {
        return rejected(e);
      }
    }

    keys() {
      try {
        return resolved(cacheCall({ op: "names" }, [], "keys", "CacheStorage").result);
      } catch (e) {
        return rejected(e);
      }
    }
  }
  expose(CacheStorage, "CacheStorage");

  // `caches`: the realm's one `CacheStorage`, a secure context's alone;
  // reading it in a context whose storage is refused (a sandbox without
  // `allow-same-origin`, an opaque origin) throws.
  if (SECURE) {
    const storage = new CacheStorage(KEY);
    defineProperty(global, "caches", {
      get() {
        try {
          cacheCall({ op: "check" }, [], "", "");
        } catch (e) {
          const was = "Failed to execute '' on '': ";
          throw new $DOMException(
            "Failed to read the 'caches' property from '" + GLOBAL_INTERFACE + "': " +
              (StringSlice(e.message, 0, was.length) === was ? StringSlice(e.message, was.length) : e.message),
            e.name,
          );
        }
        return storage;
      },
      enumerable: true,
      configurable: true,
    });
  }

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

  // `fireAt` as a stepped task's step: the microtask checkpoint runs
  // between the listeners, as for the engine's own events.
  function* firingAt(target, event) {
    eventState(event).trusted = true;
    return yield* dispatching(target, event);
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
        // A document is sent serialized, as UTF-8 (XMLHttpRequest
        // section 3.5.6, step 4.1).
        const extracted = isNode(body) && kindOf(body) === 9
          ? {
            bytes: utf8Encode(native.serializeDocument(handleOf(body))),
            type: native.inHtmlDocument(handleOf(body))
              ? "text/html;charset=UTF-8"
              : "application/xml;charset=UTF-8",
          }
          : extract(body, "XMLHttpRequest");
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
      // UTF-8, a UTF-16 byte order mark overriding it.
      if (bytes.length >= 2 && ((bytes[0] === 0xff && bytes[1] === 0xfe) ||
        (bytes[0] === 0xfe && bytes[1] === 0xff))) {
        return native.decode(binaryString(bytes), "utf-8");
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
      if (this.#state !== DONE) return null;
      return this.#document();
    }

    // The "document response" (XMLHttpRequest section 4.8.4): the body
    // parsed into a document of its own (D67), once: an HTML one only
    // for the `document` response type, an XML one for an XML type;
    // null for another type, or XML that is not well formed.
    #document() {
      if (this.#cache !== undefined) return this.#cache;
      const response = this.#response;
      let made = null;
      if (response !== null && !response.error && response.body !== null) {
        const mime = this.#mime();
        const at = StringIndexOf(mime, ";");
        const essence = ToLowerCase(StringTrim(at < 0 ? mime : StringSlice(mime, 0, at)));
        const html = essence === "text/html";
        const xml = essence === "text/xml" || essence === "application/xml" ||
          StringEndsWith(essence, "+xml");
        if ((html && this.#responseType === "document") || xml) {
          const charset = at < 0 ? null : parameter(";" + StringSlice(mime, at + 1), "charset");
          const urls = response.urlList;
          const url = urls.length === 0 ? "about:blank" : urls[urls.length - 1];
          made = wrap(native.parseResponse(
            binaryString(new $Uint8Array(response.body)), html, essence, charset, url,
          ));
        }
      }
      this.#cache = made;
      return made;
    }

    get response() {
      const type = this.#responseType;
      if (type === "" || type === "text") {
        return this.responseText;
      }
      if (this.#state !== DONE) return null;
      const response = this.#response;
      if (response === null || response.error) return null;
      if (type === "document") return this.#document();
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

  // ---- FileReader (File API 6, gap batch A) ----

  // A blob's bytes are in the realm already: a read takes them at once,
  // and its events come as the standard queues them, each a task.
  const FILE_READER_STATES = { EMPTY: 0, LOADING: 1, DONE: 2 };
  class FileReader extends EventTarget {
    #state = 0;
    #result = null;
    #error = null;
    // The read under way, which `abort()` ends: its tasks check it.
    #read = null;

    constructor() {
      super();
    }

    get readyState() {
      return this.#state;
    }

    get result() {
      return this.#result;
    }

    get error() {
      return this.#error;
    }

    readAsArrayBuffer(blob) {
      need(arguments.length, 1, "readAsArrayBuffer", "FileReader");
      this.#start(blob, "readAsArrayBuffer", (bytes) => TypedArraySlice(bytes).buffer);
    }

    readAsBinaryString(blob) {
      need(arguments.length, 1, "readAsBinaryString", "FileReader");
      this.#start(blob, "readAsBinaryString", (bytes) => binaryString(bytes));
    }

    readAsText(blob, encoding = undefined) {
      need(arguments.length, 1, "readAsText", "FileReader");
      const label = encoding === undefined ? undefined : str(encoding);
      this.#start(blob, "readAsText", (bytes, type) => {
        // The label given, else the type's charset, else UTF-8; a byte
        // order mark overrides them.
        let fallback = label === undefined ? null : native.textDecoderName(label);
        if (fallback === null) {
          fallback = native.mimeCharset(type);
        }
        return native.decode(binaryString(bytes), fallback === null ? "utf-8" : fallback);
      });
    }

    readAsDataURL(blob) {
      need(arguments.length, 1, "readAsDataURL", "FileReader");
      this.#start(blob, "readAsDataURL", (bytes, type) =>
        "data:" + (type === "" ? "application/octet-stream" : type) + ";base64," + ToBase64(bytes));
    }

    abort() {
      if (this.#state !== 1) {
        this.#result = null;
        return;
      }
      this.#state = 2;
      this.#result = null;
      this.#read = null;
      fireAt(this, progressEvent("abort", 0, 0, false));
      if (this.#state !== 1) {
        fireAt(this, progressEvent("loadend", 0, 0, false));
      }
    }

    // The standard's read operation, `pack` the result's package.
    #start(blob, method, pack) {
      if (!isBlob(blob)) {
        throw new $TypeError(prefix(method, "FileReader") + "parameter 1 is not of type 'Blob'.");
      }
      if (this.#state === 1) {
        throw new $DOMException(prefix(method, "FileReader") +
          "The object is already busy reading Blobs.", "InvalidStateError");
      }
      this.#state = 1;
      this.#result = null;
      this.#error = null;
      const read = {};
      this.#read = read;
      const bytes = blobBytes(blob);
      const type = BlobType(blob);
      const total = viewLength(bytes);
      const current = () => this.#read === read;
      queueTask(() => {
        if (current()) {
          fireAt(this, progressEvent("loadstart", 0, total, true));
        }
      });
      if (total > 0) {
        queueTask(() => {
          if (current()) {
            fireAt(this, progressEvent("progress", total, total, true));
          }
        });
      }
      // Its load and loadend in one task, a microtask checkpoint between.
      const reader = this;
      queueTask(function* () {
        if (!current()) {
          return;
        }
        reader.#read = null;
        reader.#state = 2;
        reader.#result = pack(bytes, type);
        yield* firingAt(reader, progressEvent("load", total, total, true));
        if (reader.#state !== 1) {
          yield* firingAt(reader, progressEvent("loadend", total, total, true));
        }
      }, 0, true);
    }
  }
  for (const name in FILE_READER_STATES) {
    const desc = { value: FILE_READER_STATES[name], enumerable: true };
    defineProperty(FileReader, name, desc);
    defineProperty(FileReader.prototype, name, desc);
  }
  defineHandlers(FileReader.prototype, ["loadstart", "progress", "load", "abort", "error", "loadend"],
    false);
  expose(FileReader, "FileReader");
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
  let prefixOf;
  let setPrefixOf;
  let childList;
  let elementList;
  let tokensOf;
  let styleOf;
  let datasetOf;

  class Node extends EventTarget {
    #handle;
    // `nodeType`, and the `nodeName` (an element's tag, lowercase),
    // which never change.
    #kind;
    #name;
    // An element's namespace's URI, null for none (M32), and its
    // namespace prefix, null for none (D67).
    #ns = null;
    #prefix = null;
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
      prefixOf = (node) => node.#prefix;
      setPrefixOf = (node, prefix) => {
        node.#prefix = prefix;
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

    // The node's document (D67): the page's, another a parser or a
    // script made, or the inert one a template's contents are in.
    get ownerDocument() {
      const handle = self(this);
      switch (kindOf(this)) {
        case 9:
          return null;
        // An attribute's is its element's, or the page's.
        case 2: {
          const element = attrElement(this);
          return element === null ? theDocument() : element.ownerDocument;
        }
      }
      return wrap(native.ownerDocument(handle));
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

    // The document's base URL (gap batch A, G105); another document's
    // its own (D67).
    get baseURI() {
      const handle = self(this);
      if (otherDocuments && kindOf(this) !== 2) {
        const owner = kindOf(this) === 9 ? handle : native.ownerDocument(handle);
        if (owner !== native.document()) {
          return native.documentInfo(owner)[5];
        }
      }
      return native.baseUrl();
    }

    get nodeValue() {
      const handle = self(this);
      return isCharacterData(this) ? native.data(handle) : null;
    }

    set nodeValue(value) {
      const handle = self(this);
      if (isCharacterData(this)) {
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

    // DOM's `getRootNode()`: the root, or with `composed` the
    // shadow-including root (D68).
    getRootNode(options) {
      const handle = self(this);
      const composed = typeof options === "object" && options !== null && !!options.composed;
      return wrap(native.rootNode(handle, composed));
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

    // A document's copy is a document beside the page's (D67).
    cloneNode(deep) {
      const handle = self(this);
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
    // `compareDocumentPosition`'s bits (DOM section 4.4).
    "DOCUMENT_POSITION_DISCONNECTED", 0x01,
    "DOCUMENT_POSITION_PRECEDING", 0x02,
    "DOCUMENT_POSITION_FOLLOWING", 0x04,
    "DOCUMENT_POSITION_CONTAINS", 0x08,
    "DOCUMENT_POSITION_CONTAINED_BY", 0x10,
    "DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC", 0x20,
  ];
  for (let i = 0; i < NODE_TYPES.length; i += 2) {
    defineProperty(Node, NODE_TYPES[i], { value: NODE_TYPES[i + 1], enumerable: true });
    defineProperty(Node.prototype, NODE_TYPES[i], { value: NODE_TYPES[i + 1], enumerable: true });
  }

  // Whether `node` is a Text, ProcessingInstruction or Comment node.
  function isCharacterData(node) {
    const kind = kindOf(node);
    return kind === 3 || kind === 4 || kind === 7 || kind === 8;
  }

  // The interface a ParentNode or ChildNode member of `node` is named
  // by in its errors.
  function parentIface(node) {
    const kind = kindOf(node);
    return kind === 9 ? "Document" : kind === 11 ? "DocumentFragment" : "Element";
  }
  function childIface(node) {
    const kind = kindOf(node);
    return kind === 1 ? "Element" : kind === 10 ? "DocumentType" : "CharacterData";
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

  // The DOM's "convert nodes into a node": each string made a Text
  // node, and more than one node put in a new DocumentFragment; the
  // handle of the node, in an array, marked `made` for a fragment.
  function convert(nodes, method, iface) {
    const handles = [];
    for (let i = 0; i < nodes.length; i++) {
      const node = nodes[i];
      if (isNode(node)) {
        ArrayPush(handles, handleOf(node));
      } else {
        const text = str(node);
        try {
          ArrayPush(handles, native.createText(text));
        } catch (error) {
          throw failed(error, method, iface);
        }
      }
    }
    if (handles.length <= 1) {
      return handles;
    }
    try {
      const fragment = native.createFragment();
      for (let i = 0; i < handles.length; i++) {
        native.insert(fragment, handles[i], null);
      }
      const made = [fragment];
      made.made = true;
      return made;
    } catch (error) {
      throw failed(error, method, iface);
    }
  }

  // Inserts each of `handles` under `parent` before `before`, one
  // operation for mutation observers.
  function insertAll(parent, handles, before, method, iface) {
    native.mutationScope(true);
    try {
      for (let i = 0; i < handles.length; i++) {
        native.insert(parent, handles[i], before);
      }
    } catch (error) {
      // The nodes of a fragment made for them are left in no parent, as
      // the reference browser leaves them.
      if (handles.made) {
        for (let child = native.firstChild(handles[0]); child !== null;
          child = native.firstChild(handles[0])) {
          native.remove(handles[0], child);
        }
      }
      throw failed(error, method, iface);
    } finally {
      native.mutationScope(false);
    }
  }

  // The ParentNode mixin (DOM section 4.2.6), on Document,
  // DocumentFragment and Element.
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
      const iface = parentIface(this);
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
      const iface = parentIface(this);
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
      const iface = parentIface(this);
      const handles = convert(nodes, "prepend", iface);
      insertAll(parent, handles, native.firstChild(parent), "prepend", iface);
    },

    append(...nodes) {
      const parent = self(this);
      const iface = parentIface(this);
      const handles = convert(nodes, "append", iface);
      insertAll(parent, handles, null, "append", iface);
    },

    // DOM's `moveBefore` (section 4.2.6): `node` moved before `child`
    // keeping its state.
    moveBefore(node, child) {
      const parent = self(this);
      const iface = parentIface(this);
      need(arguments.length, 2, "moveBefore", iface);
      const handle = nodeArgument(node, 1, "moveBefore", iface);
      const before = child === null ? null : nodeArgument(child, 2, "moveBefore", iface);
      try {
        native.moveBefore(parent, handle, before);
      } catch (error) {
        throw failed(error, "moveBefore", iface);
      }
    },

    replaceChildren(...nodes) {
      const parent = self(this);
      const iface = parentIface(this);
      const handles = convert(nodes, "replaceChildren", iface);
      // The DOM's "replace all", after "ensure pre-insert validity": one
      // record for mutation observers.
      try {
        native.replaceAll(parent, handles.length === 0 ? null : handles[0]);
      } catch (error) {
        // As `insertAll` leaves them.
        if (handles.made) {
          for (let child = native.firstChild(handles[0]); child !== null;
            child = native.firstChild(handles[0])) {
            native.remove(handles[0], child);
          }
        }
        throw failed(error, "replaceChildren", iface);
      }
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

  // The ChildNode mixin (DOM section 4.2.8), on Element, DocumentType
  // and CharacterData.
  const ChildNode = {
    before(...nodes) {
      const handle = self(this);
      const iface = childIface(this);
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
      const iface = childIface(this);
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
      const iface = childIface(this);
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
      const text = str(data);
      replaceData(handle, native.data(handle).length, 0, text, "appendData");
    }

    insertData(offset, data) {
      const handle = self(this);
      need(arguments.length, 2, "insertData", "CharacterData");
      const at = checkOffset(native.data(handle), unsigned(offset), "insertData");
      replaceData(handle, at, 0, str(data), "insertData");
    }

    deleteData(offset, count) {
      const handle = self(this);
      need(arguments.length, 2, "deleteData", "CharacterData");
      const at = checkOffset(native.data(handle), unsigned(offset), "deleteData");
      replaceData(handle, at, unsigned(count), "", "deleteData");
    }

    replaceData(offset, count, data) {
      const handle = self(this);
      need(arguments.length, 3, "replaceData", "CharacterData");
      const at = checkOffset(native.data(handle), unsigned(offset), "replaceData");
      const n = unsigned(count);
      replaceData(handle, at, n, str(data), "replaceData");
    }
  }

  // The DOM's "replace data" (section 4.10), which moves the live ranges
  // in the node as it changes.
  function replaceData(handle, offset, count, data, method) {
    try {
      native.replaceData(handle, offset, count, data);
    } catch (error) {
      throw failed(error, method, "CharacterData");
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
      try {
        return wrap(native.splitText(handle, at));
      } catch (error) {
        throw failed(error, "splitText", "Text");
      }
    }

    get wholeText() {
      let first = self(this);
      for (let p = native.previousSibling(first); p !== null && isTextHandle(p); ) {
        first = p;
        p = native.previousSibling(p);
      }
      let text = "";
      for (let n = first; n !== null && isTextHandle(n); n = native.nextSibling(n)) {
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

  // Whether the document is an HTML document, of type text/html, and
  // not an XML one, whose names keep their case; asked once.
  let htmlDocument;
  function isHtmlDocument() {
    if (htmlDocument === undefined) {
      // A host that says nothing of its document has an HTML one.
      try {
        htmlDocument = native.documentInfo()[2] === "text/html";
      } catch {
        htmlDocument = true;
      }
    }
    return htmlDocument;
  }

  // Whether `node`'s document is an HTML one: the page's type until a
  // document beside it is made (D67), then the host's answer.
  function inHtmlDocument(node) {
    return otherDocuments ? native.inHtmlDocument(handleOf(node)) : isHtmlDocument();
  }

  // An element's `tagName`: an HTML element's name in upper case in an
  // HTML document, another's as it is (DOM section 4.9).
  function qualifiedTag(element) {
    const name = nameOf(element);
    return nsOf(element) === HTML_NS && inHtmlDocument(element) ? ToUpperCase(name) : name;
  }

  // An attribute's name as an element keeps it: lowercased on an HTML
  // element of an HTML document, as given on another.
  function attrKey(element, name) {
    return nsOf(element) === HTML_NS && inHtmlDocument(element) ? ToLowerCase(name) : name;
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

  // The namespace of an element's first attribute named `qualified`, as
  // the host keeps it (D67): the one it was given or its name implies;
  // null for none.
  function attributeNamespace(element, qualified) {
    const handle = handleOf(element);
    const names = native.attributeNames(handle);
    for (let i = 0; i < names.length; i++) {
      if (names[i] === qualified) {
        const ns = native.attributeNamespaces(handle)[i];
        return ns === "" ? null : ns;
      }
    }
    return null;
  }

  // The qualified name of an element's attribute of namespace `ns` and
  // local name `local`, or null.
  function attributeNamed(element, ns, local) {
    const handle = handleOf(element);
    const names = native.attributeNames(handle);
    const namespaces = native.attributeNamespaces(handle);
    for (let i = 0; i < names.length; i++) {
      const name = names[i];
      const namespace = namespaces[i] === "" ? null : namespaces[i];
      const own = namespace === null ? name : localPart(name);
      if (namespace === ns && own === local) {
        return name;
      }
    }
    return null;
  }

  // `setAttributeNS`'s change (DOM section 4.9), the name validated
  // against the namespace.
  function setAttributeNS(handle, ns, qualified, value, method) {
    try {
      native.setAttributeNS(handle, ns, qualified, value);
    } catch (error) {
      throw failed(error, method, "Element");
    }
    if (ns === null) {
      attributeHandler(wrap(handle), qualified, true, true);
    }
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

    // A name with a colon is a prefix and a local name only when it was
    // made so (`createElementNS`, the XML parser); `createElement`'s and
    // the HTML parser's are local names whole (D67).
    get localName() {
      self(this);
      return prefixOf(this) === null ? nameOf(this) : localPart(nameOf(this));
    }

    get namespaceURI() {
      self(this);
      return nsOf(this);
    }

    get prefix() {
      self(this);
      return prefixOf(this);
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

    // The namespaced attribute methods (DOM section 4.9), over the
    // namespaces the host keeps each attribute's in (D67).
    getAttributeNS(namespace, localName) {
      const handle = self(this);
      need(arguments.length, 2, "getAttributeNS", "Element");
      return native.attributeNS(handle, nullable(namespace), str(localName));
    }

    hasAttributeNS(namespace, localName) {
      const handle = self(this);
      need(arguments.length, 2, "hasAttributeNS", "Element");
      return native.attributeNS(handle, nullable(namespace), str(localName)) !== null;
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
      setAttributeNS(handle, ns, qualified, str(value), "setAttributeNS");
    }

    removeAttributeNS(namespace, localName) {
      const handle = self(this);
      need(arguments.length, 2, "removeAttributeNS", "Element");
      const ns = nullable(namespace);
      const local = str(localName);
      native.removeAttributeNS(handle, ns, local);
      if (ns === null) {
        attributeHandler(this, local, false);
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
    attributeHandler(wrap(handle), name, true, true);
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

  // Whether `handle` names the page's document rather than one beside
  // it (D67).
  function isPage(handle) {
    return !otherDocuments || handle === native.document();
  }

  // What `DocumentInfo` says of a document: its URL, encoding, content
  // type, mode and referrer, and another's base URL.
  function documentInfo(handle) {
    return isPage(handle) ? native.documentInfo() : native.documentInfo(handle);
  }

  class Document extends Node {
    // `new Document()` (DOM section 4.5) is a script's to make: an XML
    // document of its own, with nothing in it.
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        try {
          return wrap(native.createDocument("plain", ""));
        } catch (error) {
          throw failed(error, "Document", "Document");
        }
      }
      super(key, handle, kind, name);
    }

    get documentElement() {
      return wrap(documentElement(self(this)));
    }

    // SVG's `rootElement` (SVG 2 section 5.14.2): the document element
    // when it is an SVG `svg` element.
    get rootElement() {
      const element = wrap(documentElement(self(this)));
      return element !== null && nsOf(element) === SVG_NS && element.localName === "svg"
        ? element
        : null;
    }

    // The document's fonts (CSS Font Loading 3, D54).
    get fonts() {
      self(this);
      return fontSetOf(this);
    }

    get head() {
      return wrap(childOfHtml(self(this), "head"));
    }

    get body() {
      return wrap(childOfHtml(self(this), "body"));
    }

    // The focused element, or the body when none is (HTML section 6.6.6),
    // and whether the window has the system's focus; a document beside
    // the page's has no focus of its own.
    get activeElement() {
      const handle = self(this);
      const focused = isPage(handle) ? native.focused() : null;
      // An element in a shadow tree is its outermost host here (D68).
      return focused === null
        ? wrap(childOfHtml(handle, "body"))
        : wrap(native.retarget(focused, handle));
    }

    hasFocus() {
      const handle = self(this);
      return isPage(handle) && native.hasFocus();
    }

    // Page visibility (HTML section 6.2): hidden while the window is
    // minimized or covered; a document with no window is hidden.
    get visibilityState() {
      const handle = self(this);
      return !isPage(handle) || native.hidden() ? "hidden" : "visible";
    }

    get hidden() {
      const handle = self(this);
      return !isPage(handle) || native.hidden();
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
      const handle = self(this);
      return isPage(handle) ? global : null;
    }

    get textContent() {
      self(this);
      return null;
    }

    set textContent(value) {
      self(this);
    }

    get title() {
      const handle = self(this);
      return isPage(handle) ? native.title() : native.documentTitle(handle);
    }

    set title(value) {
      const handle = self(this);
      if (isPage(handle)) {
        native.setTitle(str(value));
      } else {
        native.setDocumentTitle(handle, str(value));
      }
    }

    get currentScript() {
      const handle = self(this);
      return isPage(handle) ? wrap(native.currentScript()) : null;
    }

    get URL() {
      return documentInfo(self(this))[0];
    }

    get documentURI() {
      return documentInfo(self(this))[0];
    }

    get characterSet() {
      return documentInfo(self(this))[1];
    }

    get charset() {
      return documentInfo(self(this))[1];
    }

    get inputEncoding() {
      return documentInfo(self(this))[1];
    }

    get contentType() {
      return documentInfo(self(this))[2];
    }

    get compatMode() {
      return documentInfo(self(this))[3];
    }

    // The referrer of the request that brought the document (M33.3).
    get referrer() {
      return documentInfo(self(this))[4];
    }

    // A document with no window is cookie-averse (HTML section 3.1.4).
    get cookie() {
      const handle = self(this);
      return isPage(handle) ? native.cookie() : "";
    }

    set cookie(value) {
      const handle = self(this);
      if (isPage(handle)) {
        native.setCookie(str(value));
      }
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
      const handle = self(this);
      return isPage(handle) ? location : null;
    }

    set location(value) {
      const handle = self(this);
      if (isPage(handle)) {
        location.href = value;
      }
    }

    // A document a parser or a script made is complete once made.
    get readyState() {
      const handle = self(this);
      return isPage(handle) ? native.readyState() : "complete";
    }

    write(...text) {
      const handle = self(this);
      if (isPage(handle)) {
        write(text, "", "write");
      } else {
        writeElsewhere(handle, "write");
      }
    }

    writeln(...text) {
      const handle = self(this);
      if (isPage(handle)) {
        write(text, "\n", "writeln");
      } else {
        writeElsewhere(handle, "writeln");
      }
    }

    getElementById(elementId) {
      const handle = self(this);
      need(arguments.length, 1, "getElementById", "Document");
      return wrap(native.elementById(handle, str(elementId)));
    }

    // Each node a document makes is its (D67).
    createElement(localName) {
      const handle = self(this);
      need(arguments.length, 1, "createElement", "Document");
      const name = str(localName);
      try {
        return wrap(native.createElement(name, handle));
      } catch (error) {
        throw failed(error, "createElement", "Document");
      }
    }

    createElementNS(namespace, qualifiedName) {
      const handle = self(this);
      need(arguments.length, 2, "createElementNS", "Document");
      const [ns, qualified] = validateAndExtract(
        nullable(namespace),
        str(qualifiedName),
        "createElementNS",
        "Document",
      );
      try {
        return wrap(native.createElementNS(ns === null ? "" : ns, qualified, handle));
      } catch (error) {
        throw failed(error, "createElementNS", "Document");
      }
    }

    createTextNode(data) {
      const handle = self(this);
      need(arguments.length, 1, "createTextNode", "Document");
      const text = str(data);
      try {
        return wrap(native.createText(text, handle));
      } catch (error) {
        throw failed(error, "createTextNode", "Document");
      }
    }

    // `createCDATASection` (DOM section 4.5): an XML document's alone.
    createCDATASection(data) {
      const handle = self(this);
      need(arguments.length, 1, "createCDATASection", "Document");
      const text = str(data);
      if (inHtmlDocument(this)) {
        throw domError(
          "NotSupportedError",
          "This operation is not supported for HTML documents.",
          "createCDATASection",
          "Document",
        );
      }
      if (StringIndexOf(text, "]]>") >= 0) {
        throw domError(
          "InvalidCharacterError",
          "String cannot contain ']]>' since that is the end delimiter of a CData section.",
          "createCDATASection",
          "Document",
        );
      }
      try {
        return wrap(native.createCDATASection(text, handle));
      } catch (error) {
        throw failed(error, "createCDATASection", "Document");
      }
    }

    // `importNode` (DOM section 4.5): a copy of `node`, this document's.
    importNode(node, deep) {
      const handle = self(this);
      need(arguments.length, 1, "importNode", "Document");
      if (!isNode(node)) {
        throw new $TypeError(prefix("importNode", "Document") + "parameter 1 is not of type 'Node'.");
      }
      if (kindOf(node) === 9) {
        throw domError(
          "NotSupportedError",
          "The node provided is a document, which may not be imported.",
          "importNode",
          "Document",
        );
      }
      if (kindOf(node) === 2) {
        return node.cloneNode();
      }
      let copy;
      try {
        copy = native.clone(handleOf(node), deep === undefined ? false : !!deep);
        native.adopt(copy, handle);
      } catch (error) {
        throw failed(error, "importNode", "Document");
      }
      return wrap(copy);
    }

    // `adoptNode` (DOM section 4.5): `node` taken out of where it is and
    // made this document's.
    adoptNode(node) {
      const handle = self(this);
      need(arguments.length, 1, "adoptNode", "Document");
      if (!isNode(node)) {
        throw new $TypeError(prefix("adoptNode", "Document") + "parameter 1 is not of type 'Node'.");
      }
      if (kindOf(node) === 9) {
        throw domError(
          "NotSupportedError",
          "The node provided is a document, which may not be adopted.",
          "adoptNode",
          "Document",
        );
      }
      if (kindOf(node) === 2) {
        const element = attrElement(node);
        if (element !== null) {
          removeAttr(element, node.name);
        }
        return node;
      }
      try {
        native.adopt(handleOf(node), handle);
      } catch (error) {
        throw failed(error, "adoptNode", "Document");
      }
      applyPins();
      return node;
    }

    // `createAttribute` and `createAttributeNS` (DOM section 4.5): an
    // attribute of no element, its name lowercased in an HTML document.
    createAttribute(localName) {
      self(this);
      need(arguments.length, 1, "createAttribute", "Document");
      let name = str(localName);
      if (!validAttributeName(name)) {
        throw domError(
          "InvalidCharacterError",
          "The localName provided ('" + name + "') contains an invalid character.",
          "createAttribute",
          "Document",
        );
      }
      if (inHtmlDocument(this)) {
        name = ToLowerCase(name);
      }
      return new Attr(KEY, null, name, null, "");
    }

    createAttributeNS(namespace, qualifiedName) {
      self(this);
      need(arguments.length, 2, "createAttributeNS", "Document");
      const [ns, qualified] = validateAndExtract(
        nullable(namespace),
        str(qualifiedName),
        "createAttributeNS",
        "Document",
      );
      return new Attr(KEY, null, qualified, ns, "");
    }
  }

  // `write` and `writeln` of a document beside the page's (HTML section
  // 8.4.3), which has no parser to write to: an XML one throws, an HTML
  // one takes nothing.
  function writeElsewhere(handle, method) {
    if (native.inHtmlDocument(handle)) {
      return;
    }
    throw domError(
      "InvalidStateError",
      "Only HTML documents support " + method + "().",
      method,
      "Document",
    );
  }

  // `document.write`'s text, its arguments joined and `end` after them,
  // put in at the parser's insertion point (HTML section 8.4.3).
  function write(text, end, method) {
    let markup = "";
    for (let i = 0; i < text.length; i++) {
      markup += str(text[i]);
    }
    markup += end;
    let erased;
    try {
      erased = native.write(markup);
    } catch (error) {
      throw failed(error, method, "Document");
    }
    eraseOpened(erased);
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

  // An XML document a parser or `createDocument` made (D67).
  class XMLDocument extends Document {
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        throw illegalConstructor("XMLDocument");
      }
      super(key, handle, kind, name);
    }
  }

  // A CDATA section (DOM section 4.13): text of its own kind, which an
  // XML document's parser or `createCDATASection` makes (D67).
  class CDATASection extends Text {
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        throw illegalConstructor("CDATASection");
      }
      super(key, handle, kind, name);
    }
  }

  // ---- Comments, processing instructions, doctypes and fragments ------
  // (D58), and attributes as Attr objects over their elements'.

  class Comment extends CharacterData {
    // `new Comment(data)` is a script's to make, as the DOM lets it.
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        const data = key === undefined ? "" : str(key);
        try {
          return wrap(native.createComment(data));
        } catch (error) {
          throw failed(error, "Comment", "Comment");
        }
      }
      super(key, handle, kind, name);
    }
  }

  class ProcessingInstruction extends CharacterData {
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        throw illegalConstructor("ProcessingInstruction");
      }
      super(key, handle, kind, name);
    }

    get target() {
      self(this);
      return nameOf(this);
    }
  }

  class DocumentType extends Node {
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        throw illegalConstructor("DocumentType");
      }
      super(key, handle, kind, name);
    }

    get name() {
      self(this);
      return nameOf(this);
    }

    get publicId() {
      return native.doctypeIds(self(this))[1];
    }

    get systemId() {
      return native.doctypeIds(self(this))[2];
    }
  }

  class DocumentFragment extends Node {
    // `new DocumentFragment()` is a script's to make.
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        try {
          return wrap(native.createFragment());
        } catch (error) {
          throw failed(error, "DocumentFragment", "DocumentFragment");
        }
      }
      super(key, handle, kind, name);
    }

    getElementById(elementId) {
      const handle = self(this);
      need(arguments.length, 1, "getElementById", "DocumentFragment");
      return wrap(native.elementById(handle, str(elementId)));
    }
  }

  // ---- Shadow trees (DOM section 4.8, gap batch H, D68) --------------------

  // What a shadow root was made with: its mode, its slot assignment, and
  // whether it delegates focus, is clonable, is serializable and is a
  // declarative one.
  function shadowInfo(handle) {
    const info = native.shadowInfo(handle);
    return {
      mode: info[0],
      slotAssignment: info[1],
      delegatesFocus: info[2] === "true",
      clonable: info[3] === "true",
      serializable: info[4] === "true",
      referenceTarget: info.length > 6 ? info[6] : null,
    };
  }

  // Whether `value` is a shadow root's object.
  function isShadowRoot(value) {
    return isNode(value) && kindOf(value) === 11 && value instanceof ShadowRoot;
  }

  // `getHTML()`'s options (HTML section 8.5.1): whether serializable
  // shadow roots are written, and the shadow roots to write whatever
  // they say.
  function htmlOptions(options, iface) {
    const roots = [];
    if (options === undefined || options === null) {
      return [false, roots];
    }
    if (typeof options !== "object" && typeof options !== "function") {
      throw new $TypeError(
        prefix("getHTML", iface) + "The provided value is not of type 'GetHTMLOptions'.",
      );
    }
    const serializable = !!options.serializableShadowRoots;
    const given = options.shadowRoots;
    if (given !== undefined) {
      const list = sequence(given, "getHTML", iface, (item) => {
        if (!isShadowRoot(item)) {
          throw new $TypeError(
            prefix("getHTML", iface) + "Failed to read the 'shadowRoots' property from " +
              "'GetHTMLOptions': Failed to convert value to 'ShadowRoot'.",
          );
        }
        return handleOf(item);
      });
      for (let i = 0; i < list.length; i++) {
        ArrayPush(roots, list[i]);
      }
    }
    return [serializable, roots];
  }

  // DocumentOrShadowRoot's `activeElement` (HTML section 6.6.6) of the
  // shadow root `handle`: the focused element retargeted against it,
  // when that is in its tree.
  function shadowActiveElement(handle) {
    const focused = native.focused();
    if (focused === null) {
      return null;
    }
    const candidate = native.retarget(focused, handle);
    return native.rootNode(candidate, false) === handle ? wrap(candidate) : null;
  }

  // The elements hit at a point, the topmost first, each retargeted
  // against `root`, a document or a shadow root, those of its tree once
  // each (D68): the first alone unless `all`.
  function pointElements(root, x, y, all) {
    const found = native.elementsFromPoint(x, y, true);
    const out = [];
    for (let i = 0; i < found.length; i++) {
      const element = native.retarget(found[i], root);
      if (native.rootNode(element, false) !== root) {
        continue;
      }
      const object = wrap(element);
      if (!ArrayIncludes(out, object)) {
        ArrayPush(out, object);
        if (!all) {
          break;
        }
      }
    }
    return out;
  }

  class ShadowRoot extends DocumentFragment {
    constructor(key, handle, kind, name) {
      if (key !== KEY) {
        throw illegalConstructor("ShadowRoot");
      }
      super(key, handle, kind, name);
    }

    get mode() {
      return shadowInfo(self(this)).mode;
    }

    get delegatesFocus() {
      return shadowInfo(self(this)).delegatesFocus;
    }

    get slotAssignment() {
      return shadowInfo(self(this)).slotAssignment;
    }

    get clonable() {
      return shadowInfo(self(this)).clonable;
    }

    get serializable() {
      return shadowInfo(self(this)).serializable;
    }

    get host() {
      return wrap(native.shadowHost(self(this)));
    }

    // The ID of the element in its tree that stands for its host in the
    // host's tree's references (Reference Target), or null.
    get referenceTarget() {
      return shadowInfo(self(this)).referenceTarget;
    }

    set referenceTarget(value) {
      native.setReferenceTarget(self(this), value === null ? null : str(value));
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
        throw failedSet(error, "innerHTML", "ShadowRoot");
      }
    }

    getHTML(options) {
      const handle = self(this);
      const parsed = htmlOptions(options, "ShadowRoot");
      return native.getHTML(handle, parsed[0], parsed[1]);
    }

    // HTML's setHTML() and setHTMLUnsafe() of a shadow root (section
    // 8.5.1, the Sanitizer API): `html` parsed in the context of its host,
    // sanitized as `options` say, safe or not, in place of its children.
    setHTML(html, options = undefined) {
      const handle = self(this);
      need(arguments.length, 1, "setHTML", "ShadowRoot");
      setAndFilterHTML(handle, wrap(native.shadowHost(handle)), str(html), options, true,
        "setHTML", "ShadowRoot");
    }

    setHTMLUnsafe(html, options = undefined) {
      const handle = self(this);
      need(arguments.length, 1, "setHTMLUnsafe", "ShadowRoot");
      setAndFilterHTML(handle, wrap(native.shadowHost(handle)), str(html), options, false,
        "setHTMLUnsafe", "ShadowRoot");
    }

    get activeElement() {
      return shadowActiveElement(self(this));
    }

    // The elements at a point of the viewport (CSSOM View section 5.1),
    // retargeted against the shadow root, those of its tree.
    elementFromPoint(x, y) {
      const handle = self(this);
      need(arguments.length, 2, "elementFromPoint", "ShadowRoot");
      const found = pointElements(handle, $Number(x), $Number(y), false);
      return found.length === 0 ? null : found[0];
    }

    elementsFromPoint(x, y) {
      const handle = self(this);
      need(arguments.length, 2, "elementsFromPoint", "ShadowRoot");
      return pointElements(handle, $Number(x), $Number(y), true);
    }

    get fullscreenElement() {
      self(this);
      return null;
    }

    get pointerLockElement() {
      self(this);
      return null;
    }

    get pictureInPictureElement() {
      self(this);
      return null;
    }
  }
  expose(ShadowRoot, "ShadowRoot");

  // `ShadowRootInit`'s members (DOM section 4.9), read in their order,
  // as `attachShadow` takes them.
  const SHADOW_MODES = ["open", "closed"];
  const SLOT_ASSIGNMENTS = ["named", "manual"];
  function shadowInit(init) {
    const where = prefix("attachShadow", "Element");
    if (init === undefined || init === null || (typeof init !== "object" && typeof init !== "function")) {
      throw new $TypeError(where +
        (init === undefined || init === null
          ? "Failed to read the 'mode' property from 'ShadowRootInit': Required member is undefined."
          : "The provided value is not of type 'ShadowRootInit'."));
    }
    const clonable = !!init.clonable;
    const delegatesFocus = !!init.delegatesFocus;
    const modeValue = init.mode;
    if (modeValue === undefined) {
      throw new $TypeError(where +
        "Failed to read the 'mode' property from 'ShadowRootInit': Required member is undefined.");
    }
    const mode = str(modeValue);
    if (!ArrayIncludes(SHADOW_MODES, mode)) {
      throw new $TypeError(where + "Failed to read the 'mode' property from 'ShadowRootInit': " +
        "The provided value '" + mode + "' is not a valid enum value of type ShadowRootMode.");
    }
    const referenceValue = init.referenceTarget;
    const referenceTarget =
      referenceValue === undefined || referenceValue === null ? null : str(referenceValue);
    const serializable = !!init.serializable;
    const assignmentValue = init.slotAssignment;
    const slotAssignment = assignmentValue === undefined ? "named" : str(assignmentValue);
    if (!ArrayIncludes(SLOT_ASSIGNMENTS, slotAssignment)) {
      throw new $TypeError(where +
        "Failed to read the 'slotAssignment' property from 'ShadowRootInit': The provided " +
        "value '" + slotAssignment + "' is not a valid enum value of type SlotAssignmentMode.");
    }
    return [mode, delegatesFocus, slotAssignment, clonable, serializable, referenceTarget];
  }

  // The part names of an element, a token list of its `part` (CSS Shadow
  // Parts 1), made once.
  const partLists = new $WeakMap();

  // A host's shadow root's object, held while the host's is, out of the
  // document too, so that what a script set on it (its expandos, its
  // adopted sheets) stays with it as the host does (D68).
  const heldRoots = new $WeakMap();
  function holdRoot(host, root) {
    if (root !== null) {
      WeakMapSet(heldRoots, host, root);
    }
    return root;
  }

  include(
    {
      attachShadow(init) {
        const handle = self(this);
        need(arguments.length, 1, "attachShadow", "Element");
        const parsed = shadowInit(init);
        try {
          return holdRoot(this, wrap(native.attachShadow(handle, parsed[0], parsed[1],
            parsed[2], parsed[3], parsed[4], parsed[5])));
        } catch (error) {
          throw failed(error, "attachShadow", "Element");
        }
      },

      // The element's shadow root, when it is open.
      get shadowRoot() {
        const root = native.shadowRoot(self(this));
        return root === null || shadowInfo(root).mode !== "open"
          ? null
          : holdRoot(this, wrap(root));
      },

      get assignedSlot() {
        return wrap(native.assignedSlot(self(this)));
      },

      get slot() {
        const value = native.attribute(self(this), "slot");
        return value === null ? "" : value;
      },

      set slot(value) {
        setAttribute(self(this), "slot", str(value), "slot");
      },

      get part() {
        const handle = self(this);
        let list = WeakMapGet(partLists, this);
        if (list === undefined) {
          list = tokenList(this, handle, "part");
          WeakMapSet(partLists, this, list);
        }
        return list;
      },

      set part(value) {
        setAttribute(self(this), "part", str(value), "part");
      },

      getHTML(options) {
        const handle = self(this);
        const parsed = htmlOptions(options, "Element");
        return native.getHTML(handle, parsed[0], parsed[1]);
      },
    },
    [Element.prototype],
  );

  // Slottable's `assignedSlot` on text (DOM section 4.2.9).
  include(
    {
      get assignedSlot() {
        return wrap(native.assignedSlot(self(this)));
      },
    },
    [Text.prototype],
  );

  // A processing instruction's target or a doctype's name: a name the
  // DOM takes, or the reference browser's error.
  function checkName(name, method, iface) {
    const first = name.length > 0 ? CharCodeAt(name, 0) : 48;
    if (!validAttributeName(name) || (first >= 48 && first <= 57) || first === 45 || first === 46) {
      throw domError(
        "InvalidCharacterError",
        "The " + (iface === "Document" ? "target" : "qualified name") + " provided ('" + name +
          "') is not a valid name.",
        method,
        iface,
      );
    }
  }

  include(
    {
      createComment(data) {
        const handle = self(this);
        need(arguments.length, 1, "createComment", "Document");
        const text = str(data);
        try {
          return wrap(native.createComment(text, handle));
        } catch (error) {
          throw failed(error, "createComment", "Document");
        }
      },

      createDocumentFragment() {
        const handle = self(this);
        try {
          return wrap(native.createFragment(handle));
        } catch (error) {
          throw failed(error, "createDocumentFragment", "Document");
        }
      },

      createProcessingInstruction(target, data) {
        const handle = self(this);
        need(arguments.length, 2, "createProcessingInstruction", "Document");
        const name = str(target);
        const text = str(data);
        checkName(name, "createProcessingInstruction", "Document");
        if (StringIndexOf(text, "?>") >= 0) {
          throw domError(
            "InvalidCharacterError",
            "The data provided ('" + text + "') contains '?>'.",
            "createProcessingInstruction",
            "Document",
          );
        }
        try {
          return wrap(native.createProcessingInstruction(name, text, handle));
        } catch (error) {
          throw failed(error, "createProcessingInstruction", "Document");
        }
      },

      get doctype() {
        const handle = self(this);
        return wrap(native.doctype(handle));
      },

      // `document.open()` (HTML section 8.4.2.1); with three arguments,
      // the window's `open`. A document beside the page's has no parser
      // to open.
      open(...args) {
        const handle = self(this);
        if (args.length >= 3) {
          return isPage(handle) ? global.open(args[0], args[1], args[2]) : null;
        }
        if (!isPage(handle)) {
          writeElsewhere(handle, "open");
          return this;
        }
        let erased;
        try {
          erased = native.documentOpen();
        } catch (error) {
          throw failed(error, "open", "Document");
        }
        eraseOpened(erased);
        return this;
      },

      close() {
        const handle = self(this);
        if (!isPage(handle)) {
          writeElsewhere(handle, "close");
          return;
        }
        try {
          native.documentClose();
        } catch (error) {
          throw failed(error, "close", "Document");
        }
      },

      get implementation() {
        self(this);
        let implementation = WeakMapGet(implementations, this);
        if (implementation === undefined) {
          implementation = new DOMImplementation(KEY, this);
          WeakMapSet(implementations, this, implementation);
        }
        return implementation;
      },
    },
    [Document.prototype],
  );

  // Each document's `implementation`, which makes doctypes for it and
  // documents beside it (DOM section 4.5.1, D67).
  const implementations = new $WeakMap();
  let implementationDocument;
  class DOMImplementation {
    #document;

    constructor(key, document) {
      if (key !== KEY) {
        throw illegalConstructor("DOMImplementation");
      }
      this.#document = document;
    }

    static {
      implementationDocument = (implementation) => {
        if (typeof implementation !== "object" || implementation === null ||
            !(#document in implementation)) {
          throw illegalInvocation();
        }
        return handleOf(implementation.#document);
      };
    }

    createDocumentType(qualifiedName, publicId, systemId) {
      const document = implementationDocument(this);
      need(arguments.length, 3, "createDocumentType", "DOMImplementation");
      const name = str(qualifiedName);
      validateAndExtract(null, name, "createDocumentType", "DOMImplementation");
      try {
        return wrap(native.createDoctype(name, str(publicId), str(systemId), document));
      } catch (error) {
        throw failed(error, "createDocumentType", "DOMImplementation");
      }
    }

    // An XML document, with the element and doctype given (DOM section
    // 4.5.1): its content type by the element's namespace.
    createDocument(namespace, qualifiedName, doctype) {
      implementationDocument(this);
      need(arguments.length, 2, "createDocument", "DOMImplementation");
      const ns = nullable(namespace);
      const name = strOrEmpty(qualifiedName);
      if (doctype !== undefined && doctype !== null &&
          !(isNode(doctype) && kindOf(doctype) === 10)) {
        throw new $TypeError(
          prefix("createDocument", "DOMImplementation") +
            "parameter 3 is not of type 'DocumentType'.",
        );
      }
      const element = name === ""
        ? null
        : validateAndExtract(ns, name, "createDocument", "DOMImplementation");
      const type = ns === HTML_NS ? "application/xhtml+xml" : ns === SVG_NS ? "image/svg+xml"
        : "application/xml";
      try {
        const document = native.createDocument("xml", type);
        if (doctype !== undefined && doctype !== null) {
          native.insert(document, handleOf(doctype), null);
        }
        if (element !== null) {
          const made = native.createElementNS(element[0] === null ? "" : element[0], element[1],
            document);
          native.insert(document, made, null);
        }
        return wrap(document);
      } catch (error) {
        throw failed(error, "createDocument", "DOMImplementation");
      }
    }

    // An HTML document with a doctype, `html`, `head`, the `title` given
    // and `body` (DOM section 4.5.1).
    createHTMLDocument(title) {
      implementationDocument(this);
      try {
        const document = native.createDocument("html", "text/html");
        native.insert(document, native.createDoctype("html", "", "", document), null);
        const html = native.createElement("html", document);
        native.insert(document, html, null);
        const head = native.createElement("head", document);
        native.insert(html, head, null);
        if (title !== undefined) {
          const element = native.createElement("title", document);
          native.insert(head, element, null);
          native.insert(element, native.createText(str(title), document), null);
        }
        native.insert(html, native.createElement("body", document), null);
        return wrap(document);
      } catch (error) {
        throw failed(error, "createHTMLDocument", "DOMImplementation");
      }
    }

    hasFeature() {
      implementationDocument(this);
      return true;
    }
  }

  // ---- DOMParser and XMLSerializer (HTML section 8.5.1, DOM Parsing
  // section 3, D67) ------------------------------------------------------

  const PARSER_TYPES = [
    "text/html",
    "text/xml",
    "application/xml",
    "application/xhtml+xml",
    "image/svg+xml",
  ];

  class DOMParser {
    #brand = true;

    parseFromString(string, type) {
      if (typeof this !== "object" || this === null || !(#brand in this)) {
        throw illegalInvocation();
      }
      need(arguments.length, 2, "parseFromString", "DOMParser");
      const markup = str(string);
      const mime = str(type);
      if (!ArrayIncludes(PARSER_TYPES, mime)) {
        throw new $TypeError(
          prefix("parseFromString", "DOMParser") + "The provided value '" + mime +
            "' is not a valid enum value of type DOMParserSupportedType.",
        );
      }
      try {
        return wrap(native.parseDocument(markup, mime));
      } catch (error) {
        throw failed(error, "parseFromString", "DOMParser");
      }
    }
  }

  class XMLSerializer {
    #brand = true;

    serializeToString(root) {
      if (typeof this !== "object" || this === null || !(#brand in this)) {
        throw illegalInvocation();
      }
      need(arguments.length, 1, "serializeToString", "XMLSerializer");
      if (!isNode(root)) {
        throw new $TypeError(
          prefix("serializeToString", "XMLSerializer") + "parameter 1 is not of type 'Node'.",
        );
      }
      // An attribute's serialization is the empty string.
      if (kindOf(root) === 2) {
        return "";
      }
      return native.serializeXml(handleOf(root));
    }
  }

  // Each element's Attr objects, by qualified name, made when asked for;
  // and each NamedNodeMap's element.
  const attrsOf = new $WeakMap();
  const attrMapOwners = new $WeakMap();
  const namedMaps = new $WeakMap();
  let attrElement;
  let attrBind;

  class Attr extends Node {
    // The element it is an attribute of, or null; its qualified name and
    // namespace; and its value as last seen, which it keeps once it is
    // no attribute of the element.
    #element;
    #name;
    #ns;
    #value;

    constructor(key, element, name, ns, value) {
      if (key !== KEY) {
        throw illegalConstructor("Attr");
      }
      super(KEY, -1, 2, name);
      this.#element = element;
      this.#name = name;
      this.#ns = ns;
      this.#value = value;
    }

    static {
      // The element `attr` is still an attribute of, its value read.
      attrElement = (attr) => {
        const element = attr.#element;
        if (element === null) {
          return null;
        }
        const value = native.attribute(handleOf(element), attr.#name);
        if (value === null) {
          attr.#element = null;
          return null;
        }
        attr.#value = value;
        return element;
      };
      attrBind = (attr, element) => {
        attr.#element = element;
      };
    }

    get name() {
      return this.#name;
    }

    get nodeName() {
      return this.#name;
    }

    get localName() {
      return this.#ns === null ? this.#name : localPart(this.#name);
    }

    get prefix() {
      return this.#ns === null ? null : prefixPart(this.#name);
    }

    get namespaceURI() {
      return this.#ns;
    }

    get ownerElement() {
      return attrElement(this);
    }

    get specified() {
      return true;
    }

    get value() {
      attrElement(this);
      return this.#value;
    }

    set value(value) {
      const text = str(value);
      const element = attrElement(this);
      if (element !== null) {
        setAttribute(handleOf(element), this.#name, text, "value");
      }
      this.#value = text;
    }

    get nodeValue() {
      return this.value;
    }

    set nodeValue(value) {
      this.value = strOrEmpty(value);
    }

    get textContent() {
      return this.value;
    }

    set textContent(value) {
      this.value = strOrEmpty(value);
    }

    get parentNode() {
      return null;
    }

    get parentElement() {
      return null;
    }

    get firstChild() {
      return null;
    }

    get lastChild() {
      return null;
    }

    get previousSibling() {
      return null;
    }

    get nextSibling() {
      return null;
    }

    get childNodes() {
      return snapshot(NodeList, []);
    }

    get isConnected() {
      const element = attrElement(this);
      return element !== null && element.isConnected;
    }

    hasChildNodes() {
      return false;
    }

    getRootNode() {
      return this;
    }

    contains(other) {
      return other === this;
    }

    cloneNode() {
      return new Attr(KEY, null, this.#name, this.#ns, this.value);
    }
  }

  // The Attr object for `element`'s attribute `name`, the same each time
  // while it is one.
  function attrFor(element, name) {
    let attrs = WeakMapGet(attrsOf, element);
    if (attrs === undefined) {
      attrs = new $Map();
      WeakMapSet(attrsOf, element, attrs);
    }
    let attr = MapGet(attrs, name);
    if (attr === undefined || attrElement(attr) !== element) {
      const value = native.attribute(handleOf(element), name);
      attr = new Attr(KEY, element, name, attributeNamespace(element, name), value);
      MapSet(attrs, name, attr);
    }
    return attr;
  }

  // An argument that must be an Attr.
  function attrArgument(value, method, iface) {
    if (!(value instanceof Attr)) {
      throw new $TypeError(
        prefix(method, iface) + "parameter 1 is not of type 'Attr'.",
      );
    }
    return value;
  }

  // Sets `attr` on `element`: the attribute it replaces, as an Attr, or
  // null; an InUseAttributeError if another element has it.
  function setAttr(element, attr, method, iface) {
    const owner = attrElement(attr);
    if (owner !== null && owner !== element) {
      throw domError(
        "InUseAttributeError",
        "The node provided is an attribute node that is already an attribute of another " +
          "Element; attribute nodes must be explicitly cloned.",
        method,
        iface,
      );
    }
    if (owner === element) {
      return attr;
    }
    const ns = attr.namespaceURI;
    const before = attributeNamed(element, ns, attr.localName);
    const old = before === null ? null : attrFor(element, before);
    if (old !== null) {
      attrElement(old);
      attrBind(old, null);
      if (before !== attr.name) {
        removeAttribute(handleOf(element), before);
      }
    }
    if (ns === null) {
      setAttribute(handleOf(element), attr.name, attr.value, method);
    } else {
      setAttributeNS(handleOf(element), ns, attr.name, attr.value, method);
    }
    attrBind(attr, element);
    let attrs = WeakMapGet(attrsOf, element);
    if (attrs === undefined) {
      attrs = new $Map();
      WeakMapSet(attrsOf, element, attrs);
    }
    MapSet(attrs, attr.name, attr);
    return old;
  }

  // Takes `element`'s attribute `name` out, its Attr kept with its value.
  function removeAttr(element, name) {
    const attr = attrFor(element, name);
    attrElement(attr);
    attrBind(attr, null);
    removeAttribute(handleOf(element), name);
    return attr;
  }

  class NamedNodeMap {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("NamedNodeMap");
      }
    }

    get length() {
      return native.attributeNames(handleOf(mapOwner(this))).length;
    }

    item(index) {
      const element = mapOwner(this);
      need(arguments.length, 1, "item", "NamedNodeMap");
      const names = native.attributeNames(handleOf(element));
      const i = unsigned(index);
      return i < names.length ? attrFor(element, names[i]) : null;
    }

    getNamedItem(qualifiedName) {
      const element = mapOwner(this);
      need(arguments.length, 1, "getNamedItem", "NamedNodeMap");
      const name = attrKey(element, str(qualifiedName));
      return native.attribute(handleOf(element), name) === null ? null : attrFor(element, name);
    }

    getNamedItemNS(namespace, localName) {
      const element = mapOwner(this);
      need(arguments.length, 2, "getNamedItemNS", "NamedNodeMap");
      const name = attributeNamed(element, nullable(namespace), str(localName));
      return name === null ? null : attrFor(element, name);
    }

    setNamedItem(attr) {
      const element = mapOwner(this);
      need(arguments.length, 1, "setNamedItem", "NamedNodeMap");
      return setAttr(element, attrArgument(attr, "setNamedItem", "NamedNodeMap"), "setNamedItem", "NamedNodeMap");
    }

    setNamedItemNS(attr) {
      const element = mapOwner(this);
      need(arguments.length, 1, "setNamedItemNS", "NamedNodeMap");
      return setAttr(element, attrArgument(attr, "setNamedItemNS", "NamedNodeMap"), "setNamedItemNS", "NamedNodeMap");
    }

    removeNamedItem(qualifiedName) {
      const element = mapOwner(this);
      need(arguments.length, 1, "removeNamedItem", "NamedNodeMap");
      const given = str(qualifiedName);
      const name = attrKey(element, given);
      if (native.attribute(handleOf(element), name) === null) {
        throw domError(
          "NotFoundError",
          "No item with name '" + given + "' was found.",
          "removeNamedItem",
          "NamedNodeMap",
        );
      }
      return removeAttr(element, name);
    }

    removeNamedItemNS(namespace, localName) {
      const element = mapOwner(this);
      need(arguments.length, 2, "removeNamedItemNS", "NamedNodeMap");
      const name = attributeNamed(element, nullable(namespace), str(localName));
      if (name === null) {
        throw domError(
          "NotFoundError",
          "No item with namespace '" + nullable(namespace) + "' and local name '" + str(localName) +
            "' was found.",
          "removeNamedItemNS",
          "NamedNodeMap",
        );
      }
      return removeAttr(element, name);
    }
  }

  function mapOwner(map) {
    const element = WeakMapGet(attrMapOwners, map);
    if (element === undefined) {
      throw illegalInvocation();
    }
    return element;
  }

  // A NamedNodeMap's indices and its attributes' names as properties.
  const namedMapHandlers = {
    get(target, key, receiver) {
      const element = WeakMapGet(attrMapOwners, target);
      const i = arrayIndex(key);
      if (i >= 0) {
        const names = native.attributeNames(handleOf(element));
        return i < names.length ? attrFor(element, names[i]) : undefined;
      }
      if (typeof key === "string" && !ReflectHas(target, key) &&
          native.attribute(handleOf(element), key) !== null) {
        return attrFor(element, key);
      }
      return ReflectGet(target, key, receiver);
    },
    has(target, key) {
      const element = WeakMapGet(attrMapOwners, target);
      const i = arrayIndex(key);
      if (i >= 0) {
        return i < native.attributeNames(handleOf(element)).length;
      }
      return ReflectHas(target, key) ||
        (typeof key === "string" && native.attribute(handleOf(element), key) !== null);
    },
    ownKeys(target) {
      const element = WeakMapGet(attrMapOwners, target);
      const names = native.attributeNames(handleOf(element));
      const keys = [];
      for (let i = 0; i < names.length; i++) {
        ArrayPush(keys, $String(i));
      }
      for (let i = 0; i < names.length; i++) {
        ArrayPush(keys, names[i]);
      }
      const own = ReflectOwnKeys(target);
      for (let i = 0; i < own.length; i++) {
        ArrayPush(keys, own[i]);
      }
      return keys;
    },
    getOwnPropertyDescriptor(target, key) {
      const element = WeakMapGet(attrMapOwners, target);
      const i = arrayIndex(key);
      const names = native.attributeNames(handleOf(element));
      if (i >= 0) {
        return i < names.length
          ? { value: attrFor(element, names[i]), writable: false, enumerable: true, configurable: true }
          : undefined;
      }
      if (typeof key === "string" && native.attribute(handleOf(element), key) !== null) {
        return { value: attrFor(element, key), writable: false, enumerable: false, configurable: true };
      }
      return ReflectGetOwnPropertyDescriptor(target, key);
    },
    set(target, key, value, receiver) {
      if (arrayIndex(key) >= 0) {
        return false;
      }
      return ReflectSet(target, key, value, receiver);
    },
  };

  include(
    {
      get attributes() {
        self(this);
        let map = WeakMapGet(namedMaps, this);
        if (map === undefined) {
          const target = new NamedNodeMap(KEY);
          map = new $Proxy(target, namedMapHandlers);
          WeakMapSet(attrMapOwners, target, this);
          WeakMapSet(attrMapOwners, map, this);
          WeakMapSet(namedMaps, this, map);
        }
        return map;
      },

      getAttributeNode(qualifiedName) {
        const handle = self(this);
        need(arguments.length, 1, "getAttributeNode", "Element");
        const name = attrKey(this, str(qualifiedName));
        return native.attribute(handle, name) === null ? null : attrFor(this, name);
      },

      getAttributeNodeNS(namespace, localName) {
        self(this);
        need(arguments.length, 2, "getAttributeNodeNS", "Element");
        const name = attributeNamed(this, nullable(namespace), str(localName));
        return name === null ? null : attrFor(this, name);
      },

      setAttributeNode(attr) {
        self(this);
        need(arguments.length, 1, "setAttributeNode", "Element");
        return setAttr(this, attrArgument(attr, "setAttributeNode", "Element"), "setAttributeNode", "Element");
      },

      setAttributeNodeNS(attr) {
        self(this);
        need(arguments.length, 1, "setAttributeNodeNS", "Element");
        return setAttr(this, attrArgument(attr, "setAttributeNodeNS", "Element"), "setAttributeNodeNS", "Element");
      },

      removeAttributeNode(attr) {
        self(this);
        need(arguments.length, 1, "removeAttributeNode", "Element");
        attrArgument(attr, "removeAttributeNode", "Element");
        if (attrElement(attr) !== this) {
          throw domError(
            "NotFoundError",
            "The node provided is owned by another element.",
            "removeAttributeNode",
            "Element",
          );
        }
        return removeAttr(this, attr.name);
      },
    },
    [Element.prototype],
  );

  include(ParentNode, [Document.prototype, DocumentFragment.prototype, Element.prototype]);

  // ---- Ranges, traversal and the selection (D59) -------------------------

  // Whether the node `handle` names is a Text node: text, or a CDATA
  // section (D67).
  function isTextHandle(handle) {
    const kind = native.kind(handle);
    return kind === 3 || kind === 4;
  }

  // Whether the node `handle` names is character data.
  function isCharacterHandle(handle) {
    const kind = native.kind(handle);
    return kind === 3 || kind === 4 || kind === 7 || kind === 8;
  }

  // Whether `ancestor` is `node` or one of its ancestors.
  function inclusiveAncestor(ancestor, node) {
    for (let at = node; at !== null; at = native.parent(at)) {
      if (at === ancestor) {
        return true;
      }
    }
    return false;
  }

  // The root of `handle`'s tree.
  function rootHandle(handle) {
    let at = handle;
    for (let up = native.parent(at); up !== null; up = native.parent(at)) {
      at = up;
    }
    return at;
  }

  // A boundary point's node and offset, checked as "set the start or
  // end" checks them (DOM section 5.5): no doctype, no offset past its
  // length.
  function checkPoint(node, offset, method, iface) {
    const handle = nodeArgument(node, 1, method, iface);
    if (node instanceof Attr) {
      throw domError("InvalidNodeTypeError", "The node provided is of type 'Attr'.", method, iface);
    }
    if (native.kind(handle) === 10) {
      throw domError(
        "InvalidNodeTypeError",
        "The node provided is of type 'DocumentType'.",
        method,
        iface,
      );
    }
    const at = unsigned(offset);
    const length = native.nodeLength(handle);
    if (at > length) {
      throw domError(
        "IndexSizeError",
        isCharacterHandle(handle)
          ? "The offset " + at + " is larger than the node's length (" + length + ")."
          : "There is no child at offset " + at + ".",
        method,
        iface,
      );
    }
    return [handle, at];
  }

  // A node's parent, or the InvalidNodeTypeError of one with none.
  function parentOf(handle, method) {
    // An Attr (handle -1) has none.
    const parent = handle === -1 ? null : native.parent(handle);
    if (parent === null) {
      throw domError("InvalidNodeTypeError", "the given Node has no parent.", method, "Range");
    }
    return parent;
  }

  const rangeIds = new $WeakMap();
  const staticBounds = new $WeakMap();

  // The host's live ranges and node iterators, by their ids, each with
  // a weak reference to its object. One whose object is gone is let go
  // of when the registry hears of it, or sooner when the host has as
  // many as it keeps and a script makes another: the objects a script
  // dropped within one run are gone at once, the registry hearing of
  // them only after it. An id the host gave out again is another
  // object's, which the late news of the first leaves alone.
  function liveObjects(drop) {
    const refs = new $Map();
    const gone = new $FinalizationRegistry((id) => {
      const ref = MapGet(refs, id);
      if (ref !== undefined && WeakRefDeref(ref) === undefined) {
        MapDelete(refs, id);
        drop(id);
      }
    });
    return {
      // The id `make` gives, made for `object`, after the dropped ones
      // are let go of if it fails.
      make(object, make) {
        let id;
        try {
          id = make();
        } catch (error) {
          const dead = [];
          MapForEach(refs, (ref, id) => {
            if (WeakRefDeref(ref) === undefined) {
              ArrayPush(dead, id);
            }
          });
          if (dead.length === 0) {
            throw error;
          }
          for (let i = 0; i < dead.length; i++) {
            MapDelete(refs, dead[i]);
            drop(dead[i]);
          }
          id = make();
        }
        MapSet(refs, id, new $WeakRef(object));
        Register(gone, object, id);
        return id;
      },
    };
  }
  const liveRanges = liveObjects((id) => native.dropRange(id));

  // A new live range of the document `document` names, collapsed at its
  // start (`createRange`, the Range constructor).
  function newRange(range, document) {
    let id;
    try {
      id = liveRanges.make(range, () => native.createRange(document));
    } catch (error) {
      throw failed(error, "Range", "Range");
    }
    WeakMapSet(rangeIds, range, id);
  }

  // A range's boundary points: start node, start offset, end node, end
  // offset.
  function boundsOf(range) {
    const id = WeakMapGet(rangeIds, range);
    if (id !== undefined) {
      return native.rangeOf(id);
    }
    const fixed = WeakMapGet(staticBounds, range);
    if (fixed === undefined) {
      throw illegalInvocation();
    }
    return fixed;
  }

  function rangeId(range) {
    const id = WeakMapGet(rangeIds, range);
    if (id === undefined) {
      throw illegalInvocation();
    }
    return id;
  }

  // The Attr a range is in, when an Attr is its root: an Attr is in no
  // tree and has no children, so the range is at (attr, 0) at both ends.
  const attrRooted = new $WeakMap();

  function attrOf(range) {
    return WeakMapGet(attrRooted, range);
  }

  // Puts `range` at (attr, 0), or back in a tree when `attr` is undefined.
  function rootInAttr(range, attr) {
    if (attr === undefined) {
      WeakMapDelete(attrRooted, range);
    } else {
      WeakMapSet(attrRooted, range, attr);
    }
  }

  // "Set the start or end" of a range to (attr, offset): any other root
  // moves both ends there.
  function setInAttr(range, attr, offset, method) {
    if (unsigned(offset) > 0) {
      throw domError(
        "IndexSizeError",
        "There is no child at offset " + unsigned(offset) + ".",
        method,
        "Range",
      );
    }
    rootInAttr(range, attr);
  }

  // The root of `node`, an Attr being its own; `range`'s, likewise.
  function rootOfNode(node) {
    return node instanceof Attr ? node : rootHandle(handleOf(node));
  }

  function rootOfRange(range) {
    const attr = attrOf(range);
    return attr === undefined ? rootHandle(boundsOf(range)[0]) : attr;
  }

  class AbstractRange {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("AbstractRange");
      }
    }

    get startContainer() {
      const attr = attrOf(this);
      return attr === undefined ? wrap(boundsOf(this)[0]) : attr;
    }

    get startOffset() {
      return attrOf(this) === undefined ? boundsOf(this)[1] : 0;
    }

    get endContainer() {
      const attr = attrOf(this);
      return attr === undefined ? wrap(boundsOf(this)[2]) : attr;
    }

    get endOffset() {
      return attrOf(this) === undefined ? boundsOf(this)[3] : 0;
    }

    get collapsed() {
      if (attrOf(this) !== undefined) {
        return true;
      }
      const b = boundsOf(this);
      return b[0] === b[2] && b[1] === b[3];
    }
  }

  class StaticRange extends AbstractRange {
    constructor(init) {
      need(arguments.length, 1, "StaticRange", "StaticRange");
      super(KEY);
      if (init === null || typeof init !== "object") {
        throw new $TypeError(
          "Failed to construct 'StaticRange': The provided value is not of type 'StaticRangeInit'.",
        );
      }
      const points = [];
      const names = ["startContainer", "startOffset", "endContainer", "endOffset"];
      for (let i = 0; i < 4; i++) {
        const value = init[names[i]];
        if (value === undefined) {
          throw new $TypeError(
            "Failed to construct 'StaticRange': Failed to read the '" + names[i] +
              "' property from 'StaticRangeInit': Required member is undefined.",
          );
        }
        if (i % 2 === 0) {
          if (!isNode(value)) {
            throw new $TypeError(
              "Failed to construct 'StaticRange': Failed to read the '" + names[i] +
                "' property from 'StaticRangeInit': Failed to convert value to 'Node'.",
            );
          }
          const kind = kindOf(value);
          if (kind === 2 || kind === 10) {
            throw domError(
              "InvalidNodeTypeError",
              "The supplied node is of type " + (kind === 2 ? "'Attr'." : "'DocumentType'."),
              "StaticRange",
              "StaticRange",
            );
          }
          ArrayPush(points, handleOf(value));
        } else {
          ArrayPush(points, unsigned(value));
        }
      }
      WeakMapSet(staticBounds, this, points);
    }
  }

  class Range extends AbstractRange {
    // `new Range()`: collapsed at the start of the document; a
    // document's `createRange()` gives its own (D67).
    constructor(key, document) {
      super(KEY);
      newRange(this, key === KEY ? document : native.document());
    }

    // DOM Parsing's `createContextualFragment()`: `fragment` parsed in
    // the context of the start's element, its scripts not started.
    createContextualFragment(fragment) {
      need(arguments.length, 1, "createContextualFragment", "Range");
      const b = boundsOf(this);
      const markup = str(fragment);
      try {
        return wrap(native.contextualFragment(b[0], markup));
      } catch (error) {
        throw failed(error, "createContextualFragment", "Range");
      }
    }

    // CSSOM View's: the boxes of what the range holds, in viewport
    // coordinates.
    getClientRects() {
      const b = boundsOf(this);
      const g = native.rangeRects(b[0], b[1], b[2], b[3]);
      const rects = [];
      for (let i = 0; i + 3 < g.length; i += 4) {
        ArrayPush(rects, new DOMRect(g[i], g[i + 1], g[i + 2], g[i + 3]));
      }
      return new DOMRectList(KEY, rects);
    }

    // Their union, those of no size left out unless all are; the first
    // of them then.
    getBoundingClientRect() {
      const b = boundsOf(this);
      const g = native.rangeRects(b[0], b[1], b[2], b[3]);
      if (g.length < 4) {
        return new DOMRect(0, 0, 0, 0);
      }
      let found = false;
      let [left, top, right, bottom] = [0, 0, 0, 0];
      for (let i = 0; i + 3 < g.length; i += 4) {
        if (g[i + 2] === 0 && g[i + 3] === 0) {
          continue;
        }
        if (!found) {
          [left, top, right, bottom] = [g[i], g[i + 1], g[i] + g[i + 2], g[i + 1] + g[i + 3]];
          found = true;
        } else {
          left = MathMin(left, g[i]);
          top = MathMin(top, g[i + 1]);
          right = MathMax(right, g[i] + g[i + 2]);
          bottom = MathMax(bottom, g[i + 1] + g[i + 3]);
        }
      }
      if (!found) {
        return new DOMRect(g[0], g[1], g[2], g[3]);
      }
      return new DOMRect(left, top, right - left, bottom - top);
    }

    get commonAncestorContainer() {
      const attr = attrOf(this);
      if (attr !== undefined) {
        return attr;
      }
      const b = boundsOf(this);
      let common = b[0];
      while (!inclusiveAncestor(common, b[2])) {
        common = native.parent(common);
      }
      return wrap(common);
    }

    setStart(node, offset) {
      rangeId(this);
      need(arguments.length, 2, "setStart", "Range");
      if (isNode(node) && node instanceof Attr) {
        return setInAttr(this, node, offset, "setStart");
      }
      const [handle, at] = checkPoint(node, offset, "setStart", "Range");
      setStart(this, handle, at);
    }

    setEnd(node, offset) {
      rangeId(this);
      need(arguments.length, 2, "setEnd", "Range");
      if (isNode(node) && node instanceof Attr) {
        return setInAttr(this, node, offset, "setEnd");
      }
      const [handle, at] = checkPoint(node, offset, "setEnd", "Range");
      setEnd(this, handle, at);
    }

    setStartBefore(node) {
      rangeId(this);
      need(arguments.length, 1, "setStartBefore", "Range");
      const handle = nodeArgument(node, 1, "setStartBefore", "Range");
      setStart(this, parentOf(handle, "setStartBefore"), native.indexOf(handle));
    }

    setStartAfter(node) {
      rangeId(this);
      need(arguments.length, 1, "setStartAfter", "Range");
      const handle = nodeArgument(node, 1, "setStartAfter", "Range");
      setStart(this, parentOf(handle, "setStartAfter"), native.indexOf(handle) + 1);
    }

    setEndBefore(node) {
      rangeId(this);
      need(arguments.length, 1, "setEndBefore", "Range");
      const handle = nodeArgument(node, 1, "setEndBefore", "Range");
      setEnd(this, parentOf(handle, "setEndBefore"), native.indexOf(handle));
    }

    setEndAfter(node) {
      rangeId(this);
      need(arguments.length, 1, "setEndAfter", "Range");
      const handle = nodeArgument(node, 1, "setEndAfter", "Range");
      setEnd(this, parentOf(handle, "setEndAfter"), native.indexOf(handle) + 1);
    }

    collapse(toStart = false) {
      const id = rangeId(this);
      if (attrOf(this) !== undefined) {
        return;
      }
      const b = native.rangeOf(id);
      if (toStart) {
        native.setRange(id, b[0], b[1], b[0], b[1]);
      } else {
        native.setRange(id, b[2], b[3], b[2], b[3]);
      }
    }

    selectNode(node) {
      const id = rangeId(this);
      need(arguments.length, 1, "selectNode", "Range");
      const handle = nodeArgument(node, 1, "selectNode", "Range");
      const parent = parentOf(handle, "selectNode");
      const index = native.indexOf(handle);
      native.setRange(id, parent, index, parent, index + 1);
      rootInAttr(this, undefined);
    }

    selectNodeContents(node) {
      const id = rangeId(this);
      need(arguments.length, 1, "selectNodeContents", "Range");
      const handle = nodeArgument(node, 1, "selectNodeContents", "Range");
      if (node instanceof Attr) {
        return rootInAttr(this, node);
      }
      if (native.kind(handle) === 10) {
        throw domError(
          "InvalidNodeTypeError",
          "The node provided is of type 'DocumentType'.",
          "selectNodeContents",
          "Range",
        );
      }
      native.setRange(id, handle, 0, handle, native.nodeLength(handle));
      rootInAttr(this, undefined);
    }

    compareBoundaryPoints(how, sourceRange) {
      rangeId(this);
      need(arguments.length, 2, "compareBoundaryPoints", "Range");
      const code = unsignedShort(how);
      if (!(sourceRange instanceof Range)) {
        throw new $TypeError(
          prefix("compareBoundaryPoints", "Range") + "parameter 2 is not of type 'Range'.",
        );
      }
      if (code > 3) {
        throw domError(
          "NotSupportedError",
          "The comparison method provided must be one of 'START_TO_START', 'START_TO_END', " +
            "'END_TO_END', or 'END_TO_START'.",
          "compareBoundaryPoints",
          "Range",
        );
      }
      if (rootOfRange(this) !== rootOfRange(sourceRange)) {
        throw domError(
          "WrongDocumentError",
          "The two Ranges are not in the same tree.",
          "compareBoundaryPoints",
          "Range",
        );
      }
      // Two in one Attr are both at (attr, 0).
      if (attrOf(this) !== undefined) {
        return 0;
      }
      const a = boundsOf(this);
      const b = boundsOf(sourceRange);
      // START_TO_START, START_TO_END, END_TO_END, END_TO_START: which of
      // this range's points against which of the source's.
      const mine = code === 0 || code === 3 ? 0 : 2;
      const theirs = code === 0 || code === 1 ? 0 : 2;
      const order = native.compareBoundaries(a[mine], a[mine + 1], b[theirs], b[theirs + 1]);
      if (order === 2) {
        throw domError(
          "WrongDocumentError",
          "The two Ranges are not in the same tree.",
          "compareBoundaryPoints",
          "Range",
        );
      }
      return order;
    }

    isPointInRange(node, offset) {
      const b = boundsOf(this);
      need(arguments.length, 2, "isPointInRange", "Range");
      const handle = nodeArgument(node, 1, "isPointInRange", "Range");
      if (rootOfNode(node) !== rootOfRange(this)) {
        return false;
      }
      if (node instanceof Attr) {
        setInAttr(new Range(), node, offset, "isPointInRange");
        return true;
      }
      const [, at] = checkPoint(node, offset, "isPointInRange", "Range");
      return native.compareBoundaries(handle, at, b[0], b[1]) !== -1 &&
        native.compareBoundaries(handle, at, b[2], b[3]) !== 1;
    }

    comparePoint(node, offset) {
      const b = boundsOf(this);
      need(arguments.length, 2, "comparePoint", "Range");
      const handle = nodeArgument(node, 1, "comparePoint", "Range");
      if (rootOfNode(node) !== rootOfRange(this)) {
        throw domError(
          "WrongDocumentError",
          "The node provided and the Range are not in the same tree.",
          "comparePoint",
          "Range",
        );
      }
      if (node instanceof Attr) {
        setInAttr(new Range(), node, offset, "comparePoint");
        return 0;
      }
      const [, at] = checkPoint(node, offset, "comparePoint", "Range");
      if (native.compareBoundaries(handle, at, b[0], b[1]) === -1) {
        return -1;
      }
      return native.compareBoundaries(handle, at, b[2], b[3]) === 1 ? 1 : 0;
    }

    intersectsNode(node) {
      const b = boundsOf(this);
      need(arguments.length, 1, "intersectsNode", "Range");
      const handle = nodeArgument(node, 1, "intersectsNode", "Range");
      if (rootOfNode(node) !== rootOfRange(this)) {
        return false;
      }
      // An Attr has no parent.
      if (node instanceof Attr) {
        return true;
      }
      const parent = native.parent(handle);
      if (parent === null) {
        return true;
      }
      const index = native.indexOf(handle);
      return native.compareBoundaries(parent, index, b[2], b[3]) === -1 &&
        native.compareBoundaries(parent, index + 1, b[0], b[1]) === 1;
    }

    cloneRange() {
      const b = boundsOf(this);
      const copy = new Range();
      native.setRange(rangeId(copy), b[0], b[1], b[2], b[3]);
      rootInAttr(copy, attrOf(this));
      return copy;
    }

    detach() {
      rangeId(this);
    }

    deleteContents() {
      const id = rangeId(this);
      if (attrOf(this) !== undefined) {
        return;
      }
      try {
        deleteBounds(id, native.rangeOf(id));
      } catch (error) {
        throw failed(error, "deleteContents", "Range");
      }
    }

    extractContents() {
      const id = rangeId(this);
      if (attrOf(this) !== undefined) {
        return wrap(native.createFragment());
      }
      try {
        return wrap(extractBounds(native.rangeOf(id), id));
      } catch (error) {
        throw failed(error, "extractContents", "Range");
      }
    }

    cloneContents() {
      rangeId(this);
      if (attrOf(this) !== undefined) {
        return wrap(native.createFragment());
      }
      try {
        return wrap(cloneBounds(boundsOf(this)));
      } catch (error) {
        throw failed(error, "cloneContents", "Range");
      }
    }

    insertNode(node) {
      const id = rangeId(this);
      need(arguments.length, 1, "insertNode", "Range");
      const handle = nodeArgument(node, 1, "insertNode", "Range");
      // Nothing goes into an Attr, and an Attr goes nowhere.
      if (attrOf(this) !== undefined || node instanceof Attr) {
        throw domError(
          "HierarchyRequestError",
          "The node to be inserted is not of a type that may be inserted here.",
          "insertNode",
          "Range",
        );
      }
      try {
        insertInRange(id, handle);
      } catch (error) {
        throw failed(error, "insertNode", "Range");
      }
    }

    surroundContents(newParent) {
      const id = rangeId(this);
      need(arguments.length, 1, "surroundContents", "Range");
      const handle = nodeArgument(newParent, 1, "surroundContents", "Range");
      if (attrOf(this) !== undefined) {
        throw domError(
          "HierarchyRequestError",
          "The node to be inserted is not of a type that may be inserted here.",
          "surroundContents",
          "Range",
        );
      }
      const b = native.rangeOf(id);
      if (partlyHoldsNonText(b)) {
        throw domError(
          "InvalidStateError",
          "The Range has partially selected a non-Text node.",
          "surroundContents",
          "Range",
        );
      }
      const kind = native.kind(handle);
      if (kind === 9 || kind === 10 || kind === 11) {
        throw domError(
          "InvalidNodeTypeError",
          "The node provided is of type '" + nameOf(newParent) + "'.",
          "surroundContents",
          "Range",
        );
      }
      try {
        const fragment = extractBounds(b, id);
        for (let child = native.firstChild(handle); child !== null;
          child = native.firstChild(handle)) {
          native.remove(handle, child);
        }
        insertInRange(id, handle);
        native.insert(handle, fragment, null);
        const parent = native.parent(handle);
        const index = native.indexOf(handle);
        native.setRange(id, parent, index, parent, index + 1);
      } catch (error) {
        throw failed(error, "surroundContents", "Range");
      }
    }

    toString() {
      const b = boundsOf(this);
      if (attrOf(this) !== undefined) {
        return "";
      }
      if (b[0] === b[2] && isTextHandle(b[0])) {
        return StringSlice(native.data(b[0]), b[1], b[3]);
      }
      let text = "";
      if (isTextHandle(b[0])) {
        text += StringSlice(native.data(b[0]), b[1]);
      }
      const nodes = containedNodes(b);
      for (let i = 0; i < nodes.length; i++) {
        if (isTextHandle(nodes[i])) {
          text += native.data(nodes[i]);
        }
      }
      if (isTextHandle(b[2])) {
        text += StringSlice(native.data(b[2]), 0, b[3]);
      }
      return text;
    }
  }

  const RANGE_CONSTANTS = ["START_TO_START", 0, "START_TO_END", 1, "END_TO_END", 2, "END_TO_START", 3];
  for (let i = 0; i < RANGE_CONSTANTS.length; i += 2) {
    defineProperty(Range, RANGE_CONSTANTS[i], { value: RANGE_CONSTANTS[i + 1], enumerable: true });
    defineProperty(Range.prototype, RANGE_CONSTANTS[i], {
      value: RANGE_CONSTANTS[i + 1],
      enumerable: true,
    });
  }

  // "Set the start" and "set the end" of a range (DOM section 5.5): a
  // point in another tree, or past the other end, moves both ends.
  function setStart(range, handle, offset) {
    const id = rangeId(range);
    if (attrOf(range) !== undefined) {
      native.setRange(id, handle, offset, handle, offset);
      return rootInAttr(range, undefined);
    }
    const b = native.rangeOf(id);
    const order = native.compareBoundaries(handle, offset, b[2], b[3]);
    if (order === 2 || order === 1) {
      native.setRange(id, handle, offset, handle, offset);
    } else {
      native.setRange(id, handle, offset, b[2], b[3]);
    }
  }

  function setEnd(range, handle, offset) {
    const id = rangeId(range);
    if (attrOf(range) !== undefined) {
      native.setRange(id, handle, offset, handle, offset);
      return rootInAttr(range, undefined);
    }
    const b = native.rangeOf(id);
    const order = native.compareBoundaries(handle, offset, b[0], b[1]);
    if (order === 2 || order === -1) {
      native.setRange(id, handle, offset, handle, offset);
    } else {
      native.setRange(id, b[0], b[1], handle, offset);
    }
  }

  // Whether `handle` is contained in the range `b` bounds: wholly after
  // its start and before its end.
  function contained(b, handle) {
    return rootHandle(handle) === rootHandle(b[0]) &&
      native.compareBoundaries(handle, 0, b[0], b[1]) === 1 &&
      native.compareBoundaries(handle, native.nodeLength(handle), b[2], b[3]) === -1;
  }

  // The nodes `b` contains, in tree order.
  function containedNodes(b) {
    const out = [];
    let common = b[0];
    while (!inclusiveAncestor(common, b[2])) {
      common = native.parent(common);
    }
    // Every node after the start, up to the end node.
    let at = common;
    while (at !== null) {
      if (at !== common && contained(b, at)) {
        ArrayPush(out, at);
      }
      if (at === b[2] && native.firstChild(at) === null) {
        break;
      }
      let next = native.firstChild(at);
      while (next === null && at !== null && at !== common) {
        next = native.nextSibling(at);
        if (next === null) {
          at = native.parent(at);
        }
      }
      if (next === null) {
        break;
      }
      at = next;
    }
    return out;
  }

  // Where the range of `b` collapses to when its contents go: the start,
  // or just after the start's ancestor that is not the end's.
  function collapsePoint(b) {
    if (inclusiveAncestor(b[0], b[2])) {
      return [b[0], b[1]];
    }
    let reference = b[0];
    while (!inclusiveAncestor(native.parent(reference), b[2])) {
      reference = native.parent(reference);
    }
    return [native.parent(reference), native.indexOf(reference) + 1];
  }

  // The range's common ancestor's children that partly hold it, first
  // and last, each null when the end it would hold is the ancestor.
  function partialChildren(b, common) {
    let first = null;
    let last = null;
    if (!inclusiveAncestor(b[0], b[2])) {
      first = b[0];
      while (native.parent(first) !== common) {
        first = native.parent(first);
      }
    }
    if (!inclusiveAncestor(b[2], b[0])) {
      last = b[2];
      while (native.parent(last) !== common) {
        last = native.parent(last);
      }
    }
    return [first, last];
  }

  function commonOf(b) {
    let common = b[0];
    while (!inclusiveAncestor(common, b[2])) {
      common = native.parent(common);
    }
    return common;
  }

  // Whether a node other than text is partly in `b`: an ancestor of one
  // end and not of the other.
  function partlyHoldsNonText(b) {
    for (let at = b[0]; at !== null; at = native.parent(at)) {
      if (!inclusiveAncestor(at, b[2]) && native.kind(at) !== 3) {
        return true;
      }
    }
    for (let at = b[2]; at !== null; at = native.parent(at)) {
      if (!inclusiveAncestor(at, b[0]) && native.kind(at) !== 3) {
        return true;
      }
    }
    return false;
  }

  // A shallow copy of character data `handle` holding UTF-16 units
  // `from` to `to` of its data.
  function partCopy(handle, from, to) {
    const copy = native.clone(handle, false);
    native.setData(copy, StringSlice(native.data(handle), from, to));
    return copy;
  }

  // The DOM's "extract" a live range's contents (section 5.5) into a
  // new fragment; `id`, when given, the range collapsed after.
  function extractBounds(b, id) {
    const fragment = native.createFragment();
    if (b[0] === b[2] && b[1] === b[3]) {
      return fragment;
    }
    if (b[0] === b[2] && isCharacterHandle(b[0])) {
      native.insert(fragment, partCopy(b[0], b[1], b[3]), null);
      native.replaceData(b[0], b[1], b[3] - b[1], "");
      return fragment;
    }
    const common = commonOf(b);
    const [first, last] = partialChildren(b, common);
    const children = [];
    for (let c = native.firstChild(common); c !== null; c = native.nextSibling(c)) {
      if (contained(b, c)) {
        if (native.kind(c) === 10) {
          throw domError("HierarchyRequestError", "The Range contains a doctype node.");
        }
        ArrayPush(children, c);
      }
    }
    const [newNode, newOffset] = collapsePoint(b);
    if (first !== null && isCharacterHandle(first)) {
      const length = native.nodeLength(b[0]);
      native.insert(fragment, partCopy(b[0], b[1], length), null);
      native.replaceData(b[0], b[1], length - b[1], "");
    } else if (first !== null) {
      const copy = native.clone(first, false);
      native.insert(fragment, copy, null);
      const inner = extractBounds([b[0], b[1], first, native.nodeLength(first)]);
      native.insert(copy, inner, null);
    }
    for (let i = 0; i < children.length; i++) {
      native.insert(fragment, children[i], null);
    }
    if (last !== null && isCharacterHandle(last)) {
      native.insert(fragment, partCopy(b[2], 0, b[3]), null);
      native.replaceData(b[2], 0, b[3], "");
    } else if (last !== null) {
      const copy = native.clone(last, false);
      native.insert(fragment, copy, null);
      const inner = extractBounds([last, 0, b[2], b[3]]);
      native.insert(copy, inner, null);
    }
    if (id !== undefined) {
      native.setRange(id, newNode, newOffset, newNode, newOffset);
    }
    return fragment;
  }

  // The DOM's "clone the contents" of a live range (section 5.5).
  function cloneBounds(b) {
    const fragment = native.createFragment();
    if (b[0] === b[2] && b[1] === b[3]) {
      return fragment;
    }
    if (b[0] === b[2] && isCharacterHandle(b[0])) {
      native.insert(fragment, partCopy(b[0], b[1], b[3]), null);
      return fragment;
    }
    const common = commonOf(b);
    const [first, last] = partialChildren(b, common);
    const children = [];
    for (let c = native.firstChild(common); c !== null; c = native.nextSibling(c)) {
      if (contained(b, c)) {
        if (native.kind(c) === 10) {
          throw domError("HierarchyRequestError", "The Range contains a doctype node.");
        }
        ArrayPush(children, c);
      }
    }
    if (first !== null && isCharacterHandle(first)) {
      native.insert(fragment, partCopy(b[0], b[1], native.nodeLength(b[0])), null);
    } else if (first !== null) {
      const copy = native.clone(first, false);
      native.insert(fragment, copy, null);
      native.insert(copy, cloneBounds([b[0], b[1], first, native.nodeLength(first)]), null);
    }
    for (let i = 0; i < children.length; i++) {
      native.insert(fragment, native.clone(children[i], true), null);
    }
    if (last !== null && isCharacterHandle(last)) {
      native.insert(fragment, partCopy(b[2], 0, b[3]), null);
    } else if (last !== null) {
      const copy = native.clone(last, false);
      native.insert(fragment, copy, null);
      native.insert(copy, cloneBounds([last, 0, b[2], b[3]]), null);
    }
    return fragment;
  }

  // `deleteContents()` (DOM section 5.5).
  function deleteBounds(id, b) {
    if (b[0] === b[2] && b[1] === b[3]) {
      return;
    }
    if (b[0] === b[2] && isCharacterHandle(b[0])) {
      native.replaceData(b[0], b[1], b[3] - b[1], "");
      return;
    }
    // The contained nodes whose parents are not: the tops of what goes.
    const nodes = containedNodes(b);
    const tops = [];
    for (let i = 0; i < nodes.length; i++) {
      const parent = native.parent(nodes[i]);
      if (parent === null || !contained(b, parent)) {
        ArrayPush(tops, nodes[i]);
      }
    }
    const [newNode, newOffset] = collapsePoint(b);
    if (isCharacterHandle(b[0])) {
      native.replaceData(b[0], b[1], native.nodeLength(b[0]) - b[1], "");
    }
    for (let i = 0; i < tops.length; i++) {
      const parent = native.parent(tops[i]);
      if (parent !== null) {
        native.remove(parent, tops[i]);
      }
    }
    if (isCharacterHandle(b[2])) {
      native.replaceData(b[2], 0, b[3], "");
    }
    native.setRange(id, newNode, newOffset, newNode, newOffset);
  }

  // The DOM's "insert" into a live range (section 5.5).
  function insertInRange(id, node) {
    const b = native.rangeOf(id);
    const start = b[0];
    const kind = native.kind(start);
    const text = kind === 3 || kind === 4;
    if (kind === 7 || kind === 8 || (text && native.parent(start) === null) ||
        start === node) {
      throw domError("HierarchyRequestError", "The Range's start node can not hold the node.");
    }
    let reference = null;
    if (text) {
      reference = start;
    } else {
      reference = native.firstChild(start);
      for (let i = 0; i < b[1] && reference !== null; i++) {
        reference = native.nextSibling(reference);
      }
    }
    const parent = reference === null ? start : native.parent(reference);
    if (text) {
      reference = native.splitText(start, b[1]);
    }
    if (node === reference) {
      reference = native.nextSibling(reference);
    }
    const old = native.parent(node);
    if (old !== null) {
      native.remove(old, node);
    }
    let offset = reference === null ? native.nodeLength(parent) : native.indexOf(reference);
    offset += native.kind(node) === 11 ? native.nodeLength(node) : 1;
    native.insert(parent, node, reference);
    const after = native.rangeOf(id);
    if (after[0] === after[2] && after[1] === after[3]) {
      native.setRange(id, after[0], after[1], parent, offset);
    }
  }

  // ---- The selection (Selection API) ------------------------------------

  // The document's one selection: its range, or null, and whether its
  // anchor is the range's end.
  let selectionRange = null;
  let selectionBackward = false;
  let theSelection = null;

  class Selection {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("Selection");
      }
    }

    get anchorNode() {
      selectionOf(this);
      return selectionRange === null ? null
        : selectionBackward ? selectionRange.endContainer : selectionRange.startContainer;
    }

    get anchorOffset() {
      selectionOf(this);
      return selectionRange === null ? 0
        : selectionBackward ? selectionRange.endOffset : selectionRange.startOffset;
    }

    get focusNode() {
      selectionOf(this);
      return selectionRange === null ? null
        : selectionBackward ? selectionRange.startContainer : selectionRange.endContainer;
    }

    get focusOffset() {
      selectionOf(this);
      return selectionRange === null ? 0
        : selectionBackward ? selectionRange.startOffset : selectionRange.endOffset;
    }

    get isCollapsed() {
      selectionOf(this);
      return selectionRange === null || selectionRange.collapsed;
    }

    get rangeCount() {
      selectionOf(this);
      return selectionRange === null ? 0 : 1;
    }

    get type() {
      selectionOf(this);
      return selectionRange === null ? "None" : selectionRange.collapsed ? "Caret" : "Range";
    }

    get direction() {
      selectionOf(this);
      if (selectionRange === null || selectionRange.collapsed) {
        return "none";
      }
      return selectionBackward ? "backward" : "forward";
    }

    getRangeAt(index) {
      selectionOf(this);
      need(arguments.length, 1, "getRangeAt", "Selection");
      const i = unsigned(index);
      if (selectionRange === null || i > 0) {
        throw domError(
          "IndexSizeError",
          "The index provided (" + i + ") is greater than or equal to the maximum bound (" +
            (selectionRange === null ? 0 : 1) + ").",
          "getRangeAt",
          "Selection",
        );
      }
      return selectionRange;
    }

    addRange(range) {
      selectionOf(this);
      need(arguments.length, 1, "addRange", "Selection");
      if (!(range instanceof Range)) {
        throw new $TypeError(prefix("addRange", "Selection") + "parameter 1 is not of type 'Range'.");
      }
      if (selectionRange !== null) {
        return;
      }
      if (attrOf(range) !== undefined || rootHandle(boundsOf(range)[0]) !== native.document()) {
        return;
      }
      selectionRange = range;
      selectionBackward = false;
    }

    removeRange(range) {
      selectionOf(this);
      need(arguments.length, 1, "removeRange", "Selection");
      if (!(range instanceof Range)) {
        throw new $TypeError(
          prefix("removeRange", "Selection") + "parameter 1 is not of type 'Range'.",
        );
      }
      if (range !== selectionRange) {
        throw domError(
          "NotFoundError",
          "The given range isn't in document.",
          "removeRange",
          "Selection",
        );
      }
      selectionRange = null;
    }

    removeAllRanges() {
      selectionOf(this);
      selectionRange = null;
    }

    empty() {
      selectionOf(this);
      selectionRange = null;
    }

    collapse(node, offset = 0) {
      selectionOf(this);
      need(arguments.length, 1, "collapse", "Selection");
      if (node === null) {
        selectionRange = null;
        return;
      }
      const [handle, at] = checkPoint(node, offset, "collapse", "Selection");
      if (rootHandle(handle) !== native.document()) {
        return;
      }
      const range = new Range();
      native.setRange(rangeId(range), handle, at, handle, at);
      selectionRange = range;
      selectionBackward = false;
    }

    setPosition(node, offset = 0) {
      selectionOf(this);
      need(arguments.length, 1, "setPosition", "Selection");
      return this.collapse(node, offset);
    }

    collapseToStart() {
      selectionOf(this);
      if (selectionRange === null) {
        throw domError("InvalidStateError", "there is no selection.", "collapseToStart", "Selection");
      }
      const b = boundsOf(selectionRange);
      const range = new Range();
      native.setRange(rangeId(range), b[0], b[1], b[0], b[1]);
      selectionRange = range;
      selectionBackward = false;
    }

    collapseToEnd() {
      selectionOf(this);
      if (selectionRange === null) {
        throw domError("InvalidStateError", "there is no selection.", "collapseToEnd", "Selection");
      }
      const b = boundsOf(selectionRange);
      const range = new Range();
      native.setRange(rangeId(range), b[2], b[3], b[2], b[3]);
      selectionRange = range;
      selectionBackward = false;
    }

    // The Selection API's `extend()`: a node of another tree (another
    // document's, a detached one, an attribute) aborts before anything
    // is checked of the selection or of the point.
    extend(node, offset = 0) {
      selectionOf(this);
      need(arguments.length, 1, "extend", "Selection");
      const target = nodeArgument(node, 1, "extend", "Selection");
      if (node instanceof Attr || rootHandle(target) !== native.document()) {
        return;
      }
      if (selectionRange === null) {
        throw domError(
          "InvalidStateError",
          "This Selection object doesn't have any Ranges.",
          "extend",
          "Selection",
        );
      }
      const [handle, at] = checkPoint(node, offset, "extend", "Selection");
      const b = boundsOf(selectionRange);
      const anchor = selectionBackward ? [b[2], b[3]] : [b[0], b[1]];
      const range = new Range();
      const order = native.compareBoundaries(anchor[0], anchor[1], handle, at);
      if (order === 2 || order !== 1) {
        native.setRange(rangeId(range), anchor[0], anchor[1], handle, at);
        selectionBackward = false;
      } else {
        native.setRange(rangeId(range), handle, at, anchor[0], anchor[1]);
        selectionBackward = true;
      }
      selectionRange = range;
    }

    setBaseAndExtent(anchorNode, anchorOffset, focusNode, focusOffset) {
      selectionOf(this);
      need(arguments.length, 4, "setBaseAndExtent", "Selection");
      const [a, ao] = checkPoint(anchorNode, anchorOffset, "setBaseAndExtent", "Selection");
      const [f, fo] = checkPoint(focusNode, focusOffset, "setBaseAndExtent", "Selection");
      const document = native.document();
      if (rootHandle(a) !== document || rootHandle(f) !== document) {
        return;
      }
      const range = new Range();
      if (native.compareBoundaries(a, ao, f, fo) === 1) {
        native.setRange(rangeId(range), f, fo, a, ao);
        selectionBackward = true;
      } else {
        native.setRange(rangeId(range), a, ao, f, fo);
        selectionBackward = false;
      }
      selectionRange = range;
    }

    selectAllChildren(node) {
      selectionOf(this);
      need(arguments.length, 1, "selectAllChildren", "Selection");
      const handle = nodeArgument(node, 1, "selectAllChildren", "Selection");
      if (native.kind(handle) === 10) {
        throw domError(
          "InvalidNodeTypeError",
          "The node provided is of type 'DocumentType'.",
          "selectAllChildren",
          "Selection",
        );
      }
      if (rootHandle(handle) !== native.document()) {
        return;
      }
      // From before its first child to after its last: a character data
      // node's children are none, whatever its length.
      const range = new Range();
      native.setRange(rangeId(range), handle, 0, handle, native.children(handle).length);
      selectionRange = range;
      selectionBackward = false;
    }

    deleteFromDocument() {
      selectionOf(this);
      if (selectionRange !== null) {
        selectionRange.deleteContents();
      }
    }

    containsNode(node, allowPartialContainment = false) {
      selectionOf(this);
      need(arguments.length, 1, "containsNode", "Selection");
      const handle = nodeArgument(node, 1, "containsNode", "Selection");
      if (selectionRange === null || rootHandle(handle) !== native.document()) {
        return false;
      }
      const b = boundsOf(selectionRange);
      const length = native.nodeLength(handle);
      const startsIn = native.compareBoundaries(b[0], b[1], handle, 0) !== 1;
      const endsIn = native.compareBoundaries(b[2], b[3], handle, length) !== -1;
      if (!allowPartialContainment) {
        return startsIn && endsIn;
      }
      return native.compareBoundaries(b[0], b[1], handle, length) !== 1 &&
        native.compareBoundaries(b[2], b[3], handle, 0) !== -1;
    }

    toString() {
      selectionOf(this);
      if (selectionRange === null) {
        return "";
      }
      // The rendered text, as the reference browser gives it.
      const b = boundsOf(selectionRange);
      const text = native.selectedText(b[0], b[1], b[2], b[3]);
      return text === null ? selectionRange.toString() : text;
    }
  }

  function selectionOf(value) {
    if (value !== theSelection || theSelection === null) {
      throw illegalInvocation();
    }
    return value;
  }

  // The document's selection, made at its first ask.
  function getSelection() {
    if (theSelection === null) {
      theSelection = new Selection(KEY);
    }
    return theSelection;
  }

  // ---- Traversal (DOM section 6) ----------------------------------------

  const NodeFilter = {
    FILTER_ACCEPT: 1,
    FILTER_REJECT: 2,
    FILTER_SKIP: 3,
    SHOW_ALL: 0xffffffff,
    SHOW_ELEMENT: 0x1,
    SHOW_ATTRIBUTE: 0x2,
    SHOW_TEXT: 0x4,
    SHOW_CDATA_SECTION: 0x8,
    SHOW_ENTITY_REFERENCE: 0x10,
    SHOW_ENTITY: 0x20,
    SHOW_PROCESSING_INSTRUCTION: 0x40,
    SHOW_COMMENT: 0x80,
    SHOW_DOCUMENT: 0x100,
    SHOW_DOCUMENT_TYPE: 0x200,
    SHOW_DOCUMENT_FRAGMENT: 0x400,
    SHOW_NOTATION: 0x800,
  };
  if (!WORKER) {
    defineProperty(global, "NodeFilter", { value: NodeFilter, writable: true, configurable: true });
  }

  // A traverser's state: its root's object, what it shows, its filter
  // and whether the filter runs.
  const traversers = new $WeakMap();

  function traverser(value, iface) {
    const state = WeakMapGet(traversers, value);
    if (state === undefined || state.iface !== iface) {
      throw illegalInvocation();
    }
    return state;
  }

  // The DOM's "filter" a node (section 6.1): FILTER_SKIP for a kind it
  // does not show, else what its filter says.
  function filterNode(state, handle, method) {
    if (state.active) {
      throw domError(
        "InvalidStateError",
        "The filter is already running.",
        method,
        state.iface,
      );
    }
    const n = native.kind(handle) - 1;
    if (((state.whatToShow >>> n) & 1) === 0) {
      return 3;
    }
    const filter = state.filter;
    if (filter === null) {
      return 1;
    }
    state.active = true;
    try {
      let result;
      if (typeof filter === "function") {
        result = ReflectApply(filter, undefined, [wrap(handle)]);
      } else {
        const accept = filter.acceptNode;
        if (typeof accept !== "function") {
          throw new $TypeError("Failed to execute '" + method + "' on '" + state.iface +
            "': The provided callback is not callable.");
        }
        result = ReflectApply(accept, filter, [wrap(handle)]);
      }
      return $Number(result) & 0xffff;
    } finally {
      state.active = false;
    }
  }

  function makeTraverser(root, whatToShow, filter, iface) {
    const handle = nodeArgument(root, 1, "create" + iface, "Document");
    return {
      iface,
      root,
      rootHandle: handle,
      whatToShow: whatToShow === undefined ? 0xffffffff : whatToShow >>> 0,
      filter: filter === undefined ? null : filter,
      active: false,
    };
  }

  class TreeWalker {
    #current;

    constructor(key, state) {
      if (key !== KEY) {
        throw illegalConstructor("TreeWalker");
      }
      WeakMapSet(traversers, this, state);
      this.#current = state.root;
    }

    get root() {
      return traverser(this, "TreeWalker").root;
    }

    get whatToShow() {
      return traverser(this, "TreeWalker").whatToShow;
    }

    get filter() {
      return traverser(this, "TreeWalker").filter;
    }

    get currentNode() {
      traverser(this, "TreeWalker");
      return this.#current;
    }

    set currentNode(node) {
      traverser(this, "TreeWalker");
      nodeArgument(node, 1, "currentNode", "TreeWalker");
      this.#current = node;
    }

    parentNode() {
      const state = traverser(this, "TreeWalker");
      let node = handleOf(this.#current);
      while (node !== null && node !== state.rootHandle) {
        node = native.parent(node);
        if (node !== null && filterNode(state, node, "parentNode") === 1) {
          this.#current = wrap(node);
          return this.#current;
        }
      }
      return null;
    }

    firstChild() {
      return this.#children(true, "firstChild");
    }

    lastChild() {
      return this.#children(false, "lastChild");
    }

    previousSibling() {
      return this.#siblings(false, "previousSibling");
    }

    nextSibling() {
      return this.#siblings(true, "nextSibling");
    }

    // The DOM's "traverse children" (section 6.2).
    #children(first, method) {
      const state = traverser(this, "TreeWalker");
      const start = (h) => (first ? native.firstChild(h) : native.lastChild(h));
      const step = (h) => (first ? native.nextSibling(h) : native.previousSibling(h));
      const top = handleOf(this.#current);
      let node = start(top);
      while (node !== null) {
        const result = filterNode(state, node, method);
        if (result === 1) {
          this.#current = wrap(node);
          return this.#current;
        }
        if (result === 3) {
          const child = start(node);
          if (child !== null) {
            node = child;
            continue;
          }
        }
        while (node !== null) {
          const sibling = step(node);
          if (sibling !== null) {
            node = sibling;
            break;
          }
          const parent = native.parent(node);
          if (parent === null || parent === state.rootHandle || parent === top) {
            return null;
          }
          node = parent;
        }
      }
      return null;
    }

    // The DOM's "traverse siblings".
    #siblings(next, method) {
      const state = traverser(this, "TreeWalker");
      const step = (h) => (next ? native.nextSibling(h) : native.previousSibling(h));
      const into = (h) => (next ? native.firstChild(h) : native.lastChild(h));
      let node = handleOf(this.#current);
      if (node === state.rootHandle) {
        return null;
      }
      while (true) {
        let sibling = step(node);
        while (sibling !== null) {
          node = sibling;
          const result = filterNode(state, node, method);
          if (result === 1) {
            this.#current = wrap(node);
            return this.#current;
          }
          sibling = into(node);
          if (result === 2 || sibling === null) {
            sibling = step(node);
          }
        }
        node = native.parent(node);
        if (node === null || node === state.rootHandle) {
          return null;
        }
        if (filterNode(state, node, method) === 1) {
          return null;
        }
      }
    }

    previousNode() {
      const state = traverser(this, "TreeWalker");
      let node = handleOf(this.#current);
      while (node !== state.rootHandle) {
        let sibling = native.previousSibling(node);
        while (sibling !== null) {
          node = sibling;
          let result = filterNode(state, node, "previousNode");
          while (result !== 2 && native.firstChild(node) !== null) {
            node = native.lastChild(node);
            result = filterNode(state, node, "previousNode");
          }
          if (result === 1) {
            this.#current = wrap(node);
            return this.#current;
          }
          sibling = native.previousSibling(node);
        }
        const parent = native.parent(node);
        if (node === state.rootHandle || parent === null) {
          return null;
        }
        node = parent;
        if (filterNode(state, node, "previousNode") === 1) {
          this.#current = wrap(node);
          return this.#current;
        }
      }
      return null;
    }

    nextNode() {
      const state = traverser(this, "TreeWalker");
      let node = handleOf(this.#current);
      let result = 1;
      while (true) {
        while (result !== 2 && native.firstChild(node) !== null) {
          node = native.firstChild(node);
          result = filterNode(state, node, "nextNode");
          if (result === 1) {
            this.#current = wrap(node);
            return this.#current;
          }
        }
        // The next node after `node`'s subtree, under the root.
        let sibling = null;
        let temporary = node;
        while (temporary !== null) {
          if (temporary === state.rootHandle) {
            return null;
          }
          sibling = native.nextSibling(temporary);
          if (sibling !== null) {
            node = sibling;
            break;
          }
          temporary = native.parent(temporary);
        }
        if (temporary === null) {
          return null;
        }
        result = filterNode(state, node, "nextNode");
        if (result === 1) {
          this.#current = wrap(node);
          return this.#current;
        }
      }
    }
  }

  const liveIterators = liveObjects((id) => native.dropIterator(id));

  class NodeIterator {
    #id;

    constructor(key, state) {
      if (key !== KEY) {
        throw illegalConstructor("NodeIterator");
      }
      WeakMapSet(traversers, this, state);
      this.#id = liveIterators.make(this, () => native.createIterator(state.rootHandle));
    }

    get root() {
      return traverser(this, "NodeIterator").root;
    }

    get whatToShow() {
      return traverser(this, "NodeIterator").whatToShow;
    }

    get filter() {
      return traverser(this, "NodeIterator").filter;
    }

    get referenceNode() {
      traverser(this, "NodeIterator");
      return wrap(native.iteratorOf(this.#id)[0]);
    }

    get pointerBeforeReferenceNode() {
      traverser(this, "NodeIterator");
      return native.iteratorOf(this.#id)[1] === 1;
    }

    nextNode() {
      return this.#traverse(true, "nextNode");
    }

    previousNode() {
      return this.#traverse(false, "previousNode");
    }

    detach() {
      traverser(this, "NodeIterator");
    }

    // The DOM's "traverse" (section 6.1).
    #traverse(next, method) {
      const state = traverser(this, "NodeIterator");
      const now = native.iteratorOf(this.#id);
      let node = now[0];
      let before = now[1] === 1;
      const root = state.rootHandle;
      while (true) {
        if (next) {
          if (!before) {
            node = following(node, root);
            if (node === null) {
              return null;
            }
          } else {
            before = false;
          }
        } else if (before) {
          node = preceding(node, root);
          if (node === null) {
            return null;
          }
        } else {
          before = true;
        }
        const result = filterNode(state, node, method);
        if (result === 1) {
          break;
        }
      }
      native.setIterator(this.#id, node, before);
      return wrap(node);
    }
  }

  // The node after `handle` in tree order, within `root`; null past it.
  function following(handle, root) {
    const child = native.firstChild(handle);
    if (child !== null) {
      return child;
    }
    for (let at = handle; at !== null && at !== root; at = native.parent(at)) {
      const sibling = native.nextSibling(at);
      if (sibling !== null) {
        return sibling;
      }
    }
    return null;
  }

  // The node before `handle` in tree order, within `root`.
  function preceding(handle, root) {
    if (handle === root) {
      return null;
    }
    let sibling = native.previousSibling(handle);
    if (sibling === null) {
      return native.parent(handle);
    }
    for (let last = native.lastChild(sibling); last !== null; last = native.lastChild(sibling)) {
      sibling = last;
    }
    return sibling;
  }

  include(
    {
      createRange() {
        const handle = self(this);
        return new Range(KEY, handle);
      },

      createTreeWalker(root, whatToShow = undefined, filter = null) {
        self(this);
        need(arguments.length, 1, "createTreeWalker", "Document");
        return new TreeWalker(KEY, makeTraverser(root, whatToShow, filter, "TreeWalker"));
      },

      createNodeIterator(root, whatToShow = undefined, filter = null) {
        self(this);
        need(arguments.length, 1, "createNodeIterator", "Document");
        try {
          return new NodeIterator(KEY, makeTraverser(root, whatToShow, filter, "NodeIterator"));
        } catch (error) {
          throw failed(error, "createNodeIterator", "Document");
        }
      },

      getSelection() {
        self(this);
        return getSelection();
      },
    },
    [Document.prototype],
  );
  windowOnly("getSelection", {
    value: function getSelection() {
      return getSelectionOf();
    },
    writable: true,
    enumerable: true,
    configurable: true,
  });
  function getSelectionOf() {
    return getSelection();
  }

  // Node's comparisons and `normalize()` (DOM section 4.4).
  include(
    {
      compareDocumentPosition(other) {
        self(this);
        need(arguments.length, 1, "compareDocumentPosition", "Node");
        nodeArgument(other, 1, "compareDocumentPosition", "Node");
        if (this === other) {
          return 0;
        }
        let node1 = other;
        let node2 = this;
        let attr1 = null;
        if (node1 instanceof Attr) {
          attr1 = node1;
          node1 = attr1.ownerElement;
        }
        if (node2 instanceof Attr) {
          const attr2 = node2;
          node2 = attr2.ownerElement;
          if (attr1 !== null && node1 !== null && node1 === node2) {
            // Two attributes of one element: by their order on it.
            const names = native.attributeNames(handleOf(node2));
            return ArrayIndexOf(names, attr1.name) < ArrayIndexOf(names, attr2.name) ? 34 : 36;
          }
        }
        if (node1 === null || node2 === null) {
          return 1 | 32 | 4;
        }
        if (node1 === node2) {
          // An element and an attribute of its.
          return attr1 !== null ? 20 : 10;
        }
        return native.comparePosition(handleOf(node2), handleOf(node1));
      },

      isEqualNode(otherNode) {
        self(this);
        need(arguments.length, 1, "isEqualNode", "Node");
        if (otherNode === null) {
          return false;
        }
        nodeArgument(otherNode, 1, "isEqualNode", "Node");
        if (this instanceof Attr || otherNode instanceof Attr) {
          return this instanceof Attr && otherNode instanceof Attr &&
            this.namespaceURI === otherNode.namespaceURI &&
            this.localName === otherNode.localName && this.value === otherNode.value;
        }
        return native.isEqualNode(handleOf(this), handleOf(otherNode));
      },

      normalize() {
        const handle = self(this);
        if (this instanceof Attr) {
          return;
        }
        native.normalize(handle);
      },
    },
    [Node.prototype],
  );
  include(ElementLookups, [Document.prototype, Element.prototype]);

  // `getElementsByName` (HTML section 3.1.5, M26.11): a live NodeList
  // of the document's elements whose `name` is `elementName`; `forms`
  // (M26.13), a live HTMLCollection of its forms; `scripts` and `links`
  // (gap batch A, G105), its scripts and its `a` and `area` elements
  // with an `href`.
  include(
    {
      get scripts() {
        const handle = self(this);
        return controlList(this, "scripts", () =>
          collection(HTMLCollection, this, () => native.byTag(handle, "script"), true),
        );
      },

      get links() {
        const handle = self(this);
        return controlList(this, "links", () =>
          collection(HTMLCollection, this, () => native.query(handle, "a[href], area[href]", true), true),
        );
      },

      get forms() {
        const handle = self(this);
        return controlList(this, "forms", () =>
          collection(HTMLCollection, this, () => native.byTag(handle, "form"), true),
        );
      },

      // `images`, `embeds` (and `plugins`, the same collection),
      // `anchors` and `applets` (HTML section 3.1.5): its `img`
      // elements, its `embed` elements, its `a` elements with a `name`,
      // and nothing; those of its tree, never a shadow tree's.
      get images() {
        const handle = self(this);
        return controlList(this, "images", () =>
          collection(HTMLCollection, this, () => native.byTag(handle, "img"), true),
        );
      },

      get embeds() {
        const handle = self(this);
        return controlList(this, "embeds", () =>
          collection(HTMLCollection, this, () => native.byTag(handle, "embed"), true),
        );
      },

      get plugins() {
        return this.embeds;
      },

      get anchors() {
        const handle = self(this);
        return controlList(this, "anchors", () =>
          collection(HTMLCollection, this, () => native.query(handle, "a[name]", true), true),
        );
      },

      get applets() {
        self(this);
        return controlList(this, "applets", () => collection(HTMLCollection, this, () => [], true));
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
  include(ChildNode, [Element.prototype, DocumentType.prototype, CharacterData.prototype]);
  include(NonDocumentTypeChildNode, [Element.prototype, CharacterData.prototype]);

  expose(EventTarget, "EventTarget");
  expose(Node, "Node");
  expose(CharacterData, "CharacterData");
  expose(Text, "Text");
  expose(CDATASection, "CDATASection");
  expose(Comment, "Comment");
  expose(ProcessingInstruction, "ProcessingInstruction");
  expose(DocumentType, "DocumentType");
  expose(DocumentFragment, "DocumentFragment");
  expose(DOMImplementation, "DOMImplementation");
  expose(Attr, "Attr");
  expose(NamedNodeMap, "NamedNodeMap");
  expose(AbstractRange, "AbstractRange");
  expose(StaticRange, "StaticRange");
  expose(Range, "Range");
  expose(Selection, "Selection");
  expose(TreeWalker, "TreeWalker");
  expose(NodeIterator, "NodeIterator");
  defineProperty(NamedNodeMap.prototype, iteratorSymbol, {
    value: ArrayValues,
    writable: true,
    configurable: true,
  });
  expose(Element, "Element");
  expose(HTMLElement, "HTMLElement");
  expose(Document, "Document");
  expose(HTMLDocument, "HTMLDocument");
  expose(XMLDocument, "XMLDocument");
  expose(DOMParser, "DOMParser");
  expose(XMLSerializer, "XMLSerializer");

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
    definePropertyAttributes();
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
    if (state.rule !== undefined) {
      const data = ruleData(state.rule);
      // An `@font-face` rule's are descriptors, not properties.
      if (data[0] === "page") {
        return native.parsePageDescriptors(data[3]);
      }
      return data[0] === "font-face" ? native.fonts("parse " + data[3], null) : native.parseDeclarations(data[3]);
    }
    const text = native.attribute(state.handle, "style");
    // One the policies refuse is no declarations (M33.1).
    if (text === null || !native.styleAllowed(state.handle)) return [];
    return native.parseDeclarations(text);
  }

  // The longhands a block keeps a shorthand as (G50), asked once a name;
  // empty for a property kept as it is.
  const shorthandLonghands = new $Map();
  function longhandsOf(name) {
    let longhands = MapGet(shorthandLonghands, name);
    if (longhands === undefined) {
      longhands = native.cssLonghands(name);
      MapSet(shorthandLonghands, name, longhands);
    }
    return longhands;
  }

  // A shorthand's value from the block's longhands; "" for a longhand.
  function shorthandOf(name, flat) {
    return longhandsOf(name).length === 0 ? "" : native.cssShorthand(name, flat);
  }

  function writeDeclarations(state, flat, method) {
    const text = native.cssBlockText(flat);
    if (state.rule !== undefined) {
      setRule(state.rule, text, false);
      return;
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
      return native.cssBlockText(declarationsOf(state));
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
        return computedNamesOf(state).length;
      }
      return declarationsOf(state).length / 3;
    }

    item(index) {
      const state = blockState(this);
      need(arguments.length, 1, "item", "CSSStyleDeclaration");
      if (state.computed) {
        const names = computedNamesOf(state);
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
      const name = lookupName(str(property));
      const at = findDeclaration(flat, name);
      if (at >= 0) return flat[at + 1];
      // A shorthand kept as its longhands is said from them, a longhand
      // read from a shorthand kept as written.
      return flat.length === 0 ? "" : native.cssShorthand(name, flat);
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
  // Iterable by its indexed properties, as WebIDL makes it.
  defineProperty(CSSStyleDeclaration.prototype, iteratorSymbol, {
    value: ArrayValues,
    writable: true,
    configurable: true,
  });

  // ---- Style sheets and their rules (CSSOM sections 6.1 to 6.4, D66) ----

  // A sheet's state: its owner element's object and handle, the imports
  // down to it, its serial number, and its rule list.
  const sheetStates = new $WeakMap();
  // Each owner element's sheet object, with the serial it is of.
  const ownerSheets = new $WeakMap();

  function sheetState(sheet) {
    const state = WeakMapGet(sheetStates, sheet);
    if (state === undefined) {
      throw illegalInvocation();
    }
    return state;
  }

  // The native path of a rule list: the imports' count, the imports, the
  // rule path; a constructed sheet's serial number first.
  function cssomPath(state, rulePath) {
    const out = state.constructed ? [state.serial, 0] : [state.imports.length];
    for (let i = 0; i < state.imports.length; i++) {
      ArrayPush(out, state.imports[i]);
    }
    for (let i = 0; i < rulePath.length; i++) {
      ArrayPush(out, rulePath[i]);
    }
    return out;
  }

  // The sheet object of `owner`'s sheet `serial` (or of an import of it
  // at `imports`, under `parent`), made once.
  function sheetFor(owner, serial, imports, parent, ownerRule) {
    if (imports.length === 0) {
      const known = WeakMapGet(ownerSheets, owner);
      if (known !== undefined && known.serial === serial) {
        return known.sheet;
      }
    }
    const sheet = new CSSStyleSheet(KEY);
    const state = {
      owner,
      handle: handleOf(owner),
      imports,
      serial,
      parent,
      ownerRule,
      list: null,
    };
    state.list = ruleList(state, [], null);
    WeakMapSet(sheetStates, sheet, state);
    if (imports.length === 0) {
      WeakMapSet(ownerSheets, owner, { serial, sheet });
    }
    return sheet;
  }

  // A rule list's state: its sheet, its path, the grouping rule holding
  // it, and its rule objects, kept so that one rule is one object.
  const listStates = new $WeakMap();

  function ruleList(sheet, path, parentRule) {
    const list = new CSSRuleList(KEY);
    const state = { sheet, path, parentRule, objects: [] };
    const proxy = new $Proxy(list, ruleListHandlers);
    WeakMapSet(listStates, list, state);
    WeakMapSet(listStates, proxy, state);
    state.proxy = proxy;
    return state;
  }

  // The rules of a list as the native answers: four strings a rule.
  function listData(state) {
    const sheet = state.sheet;
    return native.cssom("rules", sheet.handle, cssomPath(sheet, state.path), "", 0);
  }

  // The list's rule objects brought up to the rules now there: one of the
  // same kind at the same place kept, another made.
  function listObjects(state) {
    const data = listData(state);
    const count = data.length / 4;
    if (state.objects.length > count) {
      state.objects.length = count;
    }
    for (let i = 0; i < count; i++) {
      const kind = data[i * 4];
      const kept = state.objects[i];
      if (kept === undefined || WeakMapGet(ruleStates, kept).kind !== kind) {
        state.objects[i] = makeRule(kind, state);
      }
    }
    return state.objects;
  }

  const ruleListHandlers = {
    get(target, key, receiver) {
      const i = arrayIndex(key);
      if (i >= 0) {
        const objects = listObjects(WeakMapGet(listStates, target));
        return i < objects.length ? objects[i] : undefined;
      }
      return ReflectGet(target, key, receiver);
    },
    has(target, key) {
      const i = arrayIndex(key);
      if (i >= 0) {
        return i < listObjects(WeakMapGet(listStates, target)).length;
      }
      return ReflectHas(target, key);
    },
    ownKeys(target) {
      const keys = [];
      const count = listObjects(WeakMapGet(listStates, target)).length;
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
        const objects = listObjects(WeakMapGet(listStates, target));
        return i < objects.length
          ? { value: objects[i], writable: false, enumerable: true, configurable: true }
          : undefined;
      }
      return ReflectGetOwnPropertyDescriptor(target, key);
    },
    set(target, key, value, receiver) {
      if (arrayIndex(key) >= 0) {
        return false;
      }
      return ReflectSet(target, key, value, receiver);
    },
  };

  class CSSRuleList {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("CSSRuleList");
      }
    }

    get length() {
      return listObjects(listStateOf(this)).length;
    }

    item(index) {
      need(arguments.length, 1, "item", "CSSRuleList");
      const objects = listObjects(listStateOf(this));
      const i = unsigned(index);
      return i < objects.length ? objects[i] : null;
    }
  }
  defineProperty(CSSRuleList.prototype, iteratorSymbol, {
    value: ArrayValues,
    writable: true,
    configurable: true,
  });

  function listStateOf(list) {
    const state = WeakMapGet(listStates, list);
    if (state === undefined) {
      throw illegalInvocation();
    }
    return state;
  }

  // Inserts `text` as a rule at `index` of the list, its objects moved
  // to make room: CSSOM's "insert a CSS rule".
  function insertInList(state, text, index, method, iface) {
    listObjects(state);
    const sheet = state.sheet;
    let at;
    try {
      at = native.cssom("insert", sheet.handle, cssomPath(sheet, state.path), text, index);
    } catch (error) {
      throw failed(error, method, iface);
    }
    ArraySplice(state.objects, at, 0, undefined);
    listObjects(state);
    return at;
  }

  function deleteFromList(state, index, method, iface) {
    listObjects(state);
    const sheet = state.sheet;
    try {
      native.cssom("delete", sheet.handle, cssomPath(sheet, state.path), "", index);
    } catch (error) {
      throw failed(error, method, iface);
    }
    const gone = state.objects[index];
    ArraySplice(state.objects, index, 1);
    if (gone !== undefined) {
      WeakMapGet(ruleStates, gone).list = null;
    }
  }

  // ---- The rules -------------------------------------------------------

  const ruleStates = new $WeakMap();

  function ruleState(rule) {
    const state = WeakMapGet(ruleStates, rule);
    if (state === undefined) {
      throw illegalInvocation();
    }
    return state;
  }

  // A rule's place: its list's path and its index there; null once it
  // was taken out.
  function rulePath(state) {
    if (state.list === null) {
      return null;
    }
    const at = ArrayIndexOf(state.list.objects, state.object);
    if (at < 0) {
      return null;
    }
    const path = [];
    for (let i = 0; i < state.list.path.length; i++) {
      ArrayPush(path, state.list.path[i]);
    }
    ArrayPush(path, at);
    return path;
  }

  // The four strings of a rule: kind, cssText, and two by kind; empty
  // for one taken out.
  function ruleData(state) {
    const path = rulePath(state);
    if (path === null) {
      return state.last;
    }
    const data = listData(state.list);
    const i = path[path.length - 1] * 4;
    state.last = [data[i], data[i + 1], data[i + 2], data[i + 3]];
    return state.last;
  }

  const RULE_CLASSES = { __proto__: null };

  function makeRule(kind, list) {
    const cls = RULE_CLASSES[kind];
    const rule = new cls(KEY);
    const state = { kind, list, object: rule, last: [kind, "", "", ""], children: null };
    WeakMapSet(ruleStates, rule, state);
    return rule;
  }

  // Sets a rule's declarations or selector by the native.
  function setRule(state, text, selector) {
    const path = rulePath(state);
    if (path === null) {
      return;
    }
    const sheet = state.list.sheet;
    native.cssom("set", sheet.handle, cssomPath(sheet, path), text, selector ? 1 : 0);
  }

  // A media or import rule's media list, made once, its setters the
  // rule's media.
  function ruleMedia(rule, part) {
    const state = ruleState(rule);
    if (state.media === undefined) {
      state.media = mediaList(
        () => StringSplit(ruleData(state)[part], "\u0000")[0],
        (text) => {
          const path = rulePath(state);
          if (path !== null) {
            const sheet = state.list.sheet;
            native.cssom("set", sheet.handle, cssomPath(sheet, path), text, 2);
          }
        },
      );
    }
    return state.media;
  }

  // The import rule's conditions after its media: its layer, then its
  // `supports()` condition, each null when it has none.
  function importCondition(rule, i) {
    const part = StringSplit(ruleData(ruleState(rule))[3], "\u0000")[i];
    return part === undefined || part === "" ? null : StringSlice(part, 1);
  }

  class CSSRule {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("CSSRule");
      }
    }

    get type() {
      return RULE_TYPES[ruleState(this).kind];
    }

    get cssText() {
      return ruleData(ruleState(this))[1];
    }

    set cssText(value) {
      ruleState(this);
    }

    get parentRule() {
      const state = ruleState(this);
      return state.list === null ? null : state.list.parentRule;
    }

    get parentStyleSheet() {
      const state = ruleState(this);
      return state.list === null ? null : sheetObject(state.list.sheet);
    }
  }
  const RULE_TYPES = {
    __proto__: null,
    style: 1,
    import: 3,
    media: 4,
    "font-face": 5,
    page: 6,
    keyframes: 7,
    keyframe: 8,
    supports: 12,
    layer: 0,
    "layer-statement": 0,
    namespace: 10,
    container: 0,
    "counter-style": 11,
    "font-feature-values": 14,
    "font-palette-values": 0,
    "nested-declarations": 0,
    scope: 0,
    property: 0,
  };
  const CSSRULE_CONSTANTS = [
    "STYLE_RULE", 1, "CHARSET_RULE", 2, "IMPORT_RULE", 3, "MEDIA_RULE", 4, "FONT_FACE_RULE", 5,
    "PAGE_RULE", 6, "KEYFRAMES_RULE", 7, "KEYFRAME_RULE", 8, "MARGIN_RULE", 9,
    "NAMESPACE_RULE", 10, "COUNTER_STYLE_RULE", 11, "SUPPORTS_RULE", 12,
    "FONT_FEATURE_VALUES_RULE", 14,
  ];
  for (let i = 0; i < CSSRULE_CONSTANTS.length; i += 2) {
    defineProperty(CSSRule, CSSRULE_CONSTANTS[i], { value: CSSRULE_CONSTANTS[i + 1], enumerable: true });
    defineProperty(CSSRule.prototype, CSSRULE_CONSTANTS[i], {
      value: CSSRULE_CONSTANTS[i + 1],
      enumerable: true,
    });
  }

  // The sheet object of a sheet's state.
  function sheetObject(state) {
    if (state.imports.length === 0) {
      return sheetFor(state.owner, state.serial, [], null, null);
    }
    return state.object;
  }

  // A rule's `style`: a declaration block over its declarations.
  function ruleStyle(rule) {
    const state = ruleState(rule);
    if (state.style === undefined) {
      const page = state.kind === "page";
      const target = page ? new CSSPageDescriptors(KEY) : new CSSStyleDeclaration(KEY);
      const block = { owner: rule, rule: state, page };
      definePropertyAttributes();
      const proxy = new $Proxy(target, styleHandlers);
      WeakMapSet(declarationBlocks, proxy, block);
      WeakMapSet(declarationBlocks, target, block);
      state.style = proxy;
    }
    return state.style;
  }

  // A grouping rule's list of rules, made once.
  function ruleChildren(rule) {
    const state = ruleState(rule);
    if (state.children === null) {
      const path = rulePath(state) || [];
      state.children = ruleList(state.list.sheet, path, rule);
    }
    const path = rulePath(state);
    if (path !== null) {
      state.children.path = path;
    }
    return state.children;
  }

  class CSSImportRule extends CSSRule {
    get href() {
      return ruleData(ruleState(this))[2];
    }

    get media() {
      return ruleMedia(this, 3);
    }

    set media(text) {
      ruleMedia(this, 3).mediaText = text;
    }

    get styleSheet() {
      const state = ruleState(this);
      if (state.list === null || state.list.path.length > 0) {
        return null;
      }
      // Its place among its sheet's imports.
      const objects = listObjects(state.list);
      let n = 0;
      for (let i = 0; i < objects.length && objects[i] !== this; i++) {
        if (ruleState(objects[i]).kind === "import") {
          n++;
        }
      }
      const parent = state.list.sheet;
      const imports = [];
      for (let i = 0; i < parent.imports.length; i++) {
        ArrayPush(imports, parent.imports[i]);
      }
      ArrayPush(imports, n);
      const serial = native.cssom("sheet", parent.handle, cssomPath({ imports }, []), "", 0);
      if (serial.length === 0) {
        return null;
      }
      if (state.sheet === undefined || state.sheetSerial !== serial[0]) {
        const sheet = sheetFor(parent.owner, serial[0], imports, sheetObject(parent), this);
        WeakMapGet(sheetStates, sheet).object = sheet;
        state.sheet = sheet;
        state.sheetSerial = serial[0];
      }
      return state.sheet;
    }

    get layerName() {
      return importCondition(this, 1);
    }

    get supportsText() {
      return importCondition(this, 2);
    }
  }

  class CSSGroupingRule extends CSSRule {
    get cssRules() {
      return ruleChildren(this).proxy;
    }

    insertRule(rule, index = 0) {
      need(arguments.length, 1, "insertRule", "CSSGroupingRule");
      return insertInList(ruleChildren(this), str(rule), unsigned(index), "insertRule", "CSSGroupingRule");
    }

    deleteRule(index) {
      need(arguments.length, 1, "deleteRule", "CSSGroupingRule");
      deleteFromList(ruleChildren(this), unsigned(index), "deleteRule", "CSSGroupingRule");
    }
  }

  // A style rule is a grouping rule (CSS Nesting 1 section 3.3): its
  // `cssRules` are the rules nested in it.
  class CSSStyleRule extends CSSGroupingRule {
    get selectorText() {
      return ruleData(ruleState(this))[2];
    }

    set selectorText(value) {
      setRule(ruleState(this), str(value), true);
    }

    get style() {
      return ruleStyle(this);
    }

    set style(value) {
      ruleStyle(this).cssText = value;
    }
  }

  // A run of declarations among nested rules (CSS Nesting 1 section
  // 3.3): its declarations alone.
  class CSSNestedDeclarations extends CSSRule {
    get style() {
      return ruleStyle(this);
    }

    set style(value) {
      ruleStyle(this).cssText = value;
    }
  }

  // `@property` (Properties and Values API section 4.1): its name, and
  // its syntax, inheritance and initial value, parted by NULs, the last
  // after a `1` when it has one.
  class CSSPropertyRule extends CSSRule {
    get name() {
      return ruleData(ruleState(this))[2];
    }

    get syntax() {
      return StringSplit(ruleData(ruleState(this))[3], "\u0000")[0];
    }

    get inherits() {
      return StringSplit(ruleData(ruleState(this))[3], "\u0000")[1] === "1";
    }

    get initialValue() {
      const part = StringSplit(ruleData(ruleState(this))[3], "\u0000")[2];
      return part === undefined || part === "" ? null : StringSlice(part, 1);
    }
  }

  // `@scope` (Cascade 6 section 2.5.3): its roots' and limits'
  // selectors, null for those it has not.
  class CSSScopeRule extends CSSGroupingRule {
    get start() {
      const part = ruleData(ruleState(this))[2];
      return part === "" ? null : StringSlice(part, 1);
    }

    get end() {
      const part = ruleData(ruleState(this))[3];
      return part === "" ? null : StringSlice(part, 1);
    }
  }

  // `@starting-style` (Transitions 2 section 3.1), a grouping rule of
  // nothing more.
  class CSSStartingStyleRule extends CSSGroupingRule {}

  class CSSConditionRule extends CSSGroupingRule {
    get conditionText() {
      return ruleData(ruleState(this))[2];
    }
  }

  class CSSMediaRule extends CSSConditionRule {
    get media() {
      return ruleMedia(this, 2);
    }

    set media(text) {
      ruleMedia(this, 2).mediaText = text;
    }
  }

  class CSSSupportsRule extends CSSConditionRule {}

  // `@container` (Conditional 5 section 7.1): its queries' names and
  // conditions, parted by NULs; the name and the query those of its one
  // query, empty for a list of more.
  function containerConditions(rule) {
    const parts = StringSplit(ruleData(ruleState(rule))[3], "\u0000");
    const out = [];
    for (let i = 0; i + 1 < parts.length; i += 2) {
      ArrayPush(out, { name: parts[i], query: parts[i + 1] });
    }
    return out;
  }

  class CSSContainerRule extends CSSConditionRule {
    get containerName() {
      const conditions = containerConditions(this);
      return conditions.length === 1 ? conditions[0].name : "";
    }

    get containerQuery() {
      const conditions = containerConditions(this);
      return conditions.length === 1 ? conditions[0].query : "";
    }

    get conditions() {
      return containerConditions(this);
    }
  }

  // `@counter-style` (Counter Styles 3 section 9): its name and its
  // descriptors as written.
  class CSSCounterStyleRule extends CSSRule {
    get name() {
      return ruleData(ruleState(this))[2];
    }
  }
  const COUNTER_DESCRIPTORS = [
    "system", "symbols", "additive-symbols", "negative", "prefix", "suffix", "range", "pad",
    "speak-as", "fallback",
  ];
  for (let i = 0; i < COUNTER_DESCRIPTORS.length; i++) {
    const name = COUNTER_DESCRIPTORS[i];
    const dash = StringIndexOf(name, "-");
    const key = dash < 0
      ? name
      : StringSlice(name, 0, dash) + ToUpperCase(name[dash + 1]) + StringSlice(name, dash + 2);
    defineProperty(CSSCounterStyleRule.prototype, key, {
      get() {
        const parts = StringSplit(ruleData(ruleState(this))[3], ";");
        for (let j = 0; j < parts.length; j++) {
          const colon = StringIndexOf(parts[j], ":");
          if (colon >= 0 && trimSpace(StringSlice(parts[j], 0, colon)) === name) {
            return trimSpace(StringSlice(parts[j], colon + 1));
          }
        }
        return "";
      },
      configurable: true,
      enumerable: true,
    });
  }

  // `@font-feature-values` (Fonts 4 section 6.10): its families and a
  // map of names to numbers for each feature type, which writes the
  // rule back.
  class CSSFontFeatureValuesRule extends CSSRule {
    get fontFamily() {
      return ruleData(ruleState(this))[2];
    }

    set fontFamily(value) {
      setRule(ruleState(this), str(value), true);
    }

    get stylistic() {
      return featureMap(this, "stylistic");
    }

    get historicalForms() {
      return featureMap(this, "historical-forms");
    }

    get styleset() {
      return featureMap(this, "styleset");
    }

    get characterVariant() {
      return featureMap(this, "character-variant");
    }

    get swash() {
      return featureMap(this, "swash");
    }

    get ornaments() {
      return featureMap(this, "ornaments");
    }

    get annotation() {
      return featureMap(this, "annotation");
    }
  }

  const featureMaps = new $WeakMap();

  // The rule's entries, each its type, name and numbers.
  function featureEntries(rule) {
    const lines = StringSplit(ruleData(ruleState(rule))[3], "\n");
    const out = [];
    for (let i = 0; i < lines.length; i++) {
      const parts = StringSplit(lines[i], "\t");
      if (parts.length === 3) {
        const numbers = [];
        const words = StringSplit(parts[2], " ");
        for (let j = 0; j < words.length; j++) {
          ArrayPush(numbers, +words[j]);
        }
        ArrayPush(out, [parts[0], parts[1], numbers]);
      }
    }
    return out;
  }

  function writeFeatureEntries(rule, entries) {
    let text = "";
    for (let i = 0; i < entries.length; i++) {
      let numbers = "";
      for (let j = 0; j < entries[i][2].length; j++) {
        numbers += (j === 0 ? "" : " ") + entries[i][2][j];
      }
      text += entries[i][0] + "\t" + entries[i][1] + "\t" + numbers + "\n";
    }
    setRule(ruleState(rule), text, false);
  }

  function featureMap(rule, feature) {
    let maps = WeakMapGet(featureMaps, rule);
    if (maps === undefined) {
      maps = { __proto__: null };
      WeakMapSet(featureMaps, rule, maps);
    }
    if (maps[feature] === undefined) {
      const map = new CSSFontFeatureValuesMap(KEY);
      WeakMapSet(featureMaps, map, { rule, feature });
      maps[feature] = map;
    }
    return maps[feature];
  }

  function mapState(map) {
    const state = WeakMapGet(featureMaps, map);
    if (state === undefined || state.feature === undefined) {
      throw illegalInvocation();
    }
    return state;
  }

  // This map's entries, as [name, numbers] pairs.
  function featurePairs(map) {
    const state = mapState(map);
    const all = featureEntries(state.rule);
    const out = [];
    for (let i = 0; i < all.length; i++) {
      if (all[i][0] === state.feature) {
        ArrayPush(out, [all[i][1], all[i][2]]);
      }
    }
    return out;
  }

  class CSSFontFeatureValuesMap {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("CSSFontFeatureValuesMap");
      }
    }

    get size() {
      return featurePairs(this).length;
    }

    get(name) {
      const pairs = featurePairs(this);
      const key = str(name);
      for (let i = 0; i < pairs.length; i++) {
        if (pairs[i][0] === key) return pairs[i][1];
      }
      return undefined;
    }

    has(name) {
      return this.get(name) !== undefined;
    }

    set(name, value) {
      const state = mapState(this);
      const numbers = [];
      if (typeof value === "object" && value !== null) {
        for (const n of value) {
          ArrayPush(numbers, unsigned(n));
        }
      } else {
        ArrayPush(numbers, unsigned(value));
      }
      const all = featureEntries(state.rule);
      const key = str(name);
      let found = false;
      for (let i = 0; i < all.length; i++) {
        if (all[i][0] === state.feature && all[i][1] === key) {
          all[i][2] = numbers;
          found = true;
        }
      }
      if (!found) {
        ArrayPush(all, [state.feature, key, numbers]);
      }
      writeFeatureEntries(state.rule, all);
      return this;
    }

    delete(name) {
      const state = mapState(this);
      const all = featureEntries(state.rule);
      const key = str(name);
      const kept = [];
      for (let i = 0; i < all.length; i++) {
        if (!(all[i][0] === state.feature && all[i][1] === key)) ArrayPush(kept, all[i]);
      }
      if (kept.length === all.length) return false;
      writeFeatureEntries(state.rule, kept);
      return true;
    }

    clear() {
      const state = mapState(this);
      const all = featureEntries(state.rule);
      const kept = [];
      for (let i = 0; i < all.length; i++) {
        if (all[i][0] !== state.feature) ArrayPush(kept, all[i]);
      }
      writeFeatureEntries(state.rule, kept);
    }

    forEach(callback, thisArg) {
      const pairs = featurePairs(this);
      for (let i = 0; i < pairs.length; i++) {
        callback.call(thisArg, pairs[i][1], pairs[i][0], this);
      }
    }

    entries() {
      return featurePairs(this)[iteratorSymbol]();
    }

    keys() {
      const pairs = featurePairs(this);
      const names = [];
      for (let i = 0; i < pairs.length; i++) ArrayPush(names, pairs[i][0]);
      return names[iteratorSymbol]();
    }

    values() {
      const pairs = featurePairs(this);
      const values = [];
      for (let i = 0; i < pairs.length; i++) ArrayPush(values, pairs[i][1]);
      return values[iteratorSymbol]();
    }

    [iteratorSymbol]() {
      return this.entries();
    }
  }

  // `@font-palette-values` (Fonts 4 section 9.1): its name and its
  // descriptors as the rule keeps them, each "" where it has none.
  class CSSFontPaletteValuesRule extends CSSRule {
    get name() {
      return ruleData(ruleState(this))[2];
    }

    get fontFamily() {
      return paletteDescriptor(this, 0);
    }

    get basePalette() {
      return paletteDescriptor(this, 1);
    }

    get overrideColors() {
      return paletteDescriptor(this, 2);
    }
  }

  // The descriptor at `at` of a palette values rule, parted and flagged
  // as an import rule's conditions are; "" when there is none.
  function paletteDescriptor(rule, at) {
    const part = importCondition(rule, at);
    return part === null ? "" : part;
  }

  class CSSLayerBlockRule extends CSSGroupingRule {
    get name() {
      return ruleData(ruleState(this))[2];
    }
  }

  // `@layer a, b;`: its names, a frozen array.
  class CSSLayerStatementRule extends CSSRule {
    get nameList() {
      const text = ruleData(ruleState(this))[2];
      const names = [];
      if (text !== "") {
        const parts = StringSplit(text, ", ");
        for (let i = 0; i < parts.length; i++) {
          ArrayPush(names, parts[i]);
        }
      }
      return ObjectFreeze(names);
    }
  }

  // `@namespace` (CSSOM section 6.4.8): its prefix and its URL.
  class CSSNamespaceRule extends CSSRule {
    get namespaceURI() {
      return ruleData(ruleState(this))[3];
    }

    get prefix() {
      return ruleData(ruleState(this))[2];
    }
  }

  // An `@page` rule's block (CSS Paged Media 3 section 7): its
  // descriptors only, each by its name and in camel case.
  const PAGE_DESCRIPTORS = [
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "size",
    "page-orientation",
  ];

  function pageDescriptor(name) {
    const lower = ToLowerCase(str(name));
    return ArrayIncludes(PAGE_DESCRIPTORS, lower) ? lower : null;
  }

  class CSSPageDescriptors extends CSSStyleDeclaration {}
  for (let i = 0; i < PAGE_DESCRIPTORS.length; i++) {
    const name = PAGE_DESCRIPTORS[i];
    const accessor = {
      get() {
        return this.getPropertyValue(name);
      },
      set(value) {
        this.setProperty(name, value);
      },
      configurable: true,
      enumerable: true,
    };
    defineProperty(CSSPageDescriptors.prototype, name, accessor);
    const dash = StringIndexOf(name, "-");
    if (dash >= 0) {
      const camel = StringSlice(name, 0, dash) + ToUpperCase(name[dash + 1]) +
        StringSlice(name, dash + 2);
      defineProperty(CSSPageDescriptors.prototype, camel, accessor);
    }
  }
  expose(CSSPageDescriptors, "CSSPageDescriptors");

  class CSSPageRule extends CSSGroupingRule {
    get selectorText() {
      return ruleData(ruleState(this))[2];
    }

    set selectorText(value) {
      setRule(ruleState(this), str(value), true);
    }

    get style() {
      return ruleStyle(this);
    }
  }

  class CSSFontFaceRule extends CSSRule {
    get style() {
      return ruleStyle(this);
    }
  }

  class CSSKeyframeRule extends CSSRule {
    get keyText() {
      return ruleData(ruleState(this))[2];
    }

    set keyText(value) {
      setRule(ruleState(this), str(value), true);
    }

    get style() {
      return ruleStyle(this);
    }
  }

  class CSSKeyframesRule extends CSSRule {
    get name() {
      return ruleData(ruleState(this))[2];
    }

    get cssRules() {
      return ruleChildren(this).proxy;
    }

    get length() {
      return listObjects(ruleChildren(this)).length;
    }

    appendRule(rule) {
      need(arguments.length, 1, "appendRule", "CSSKeyframesRule");
      const list = ruleChildren(this);
      insertInList(list, str(rule), listObjects(list).length, "appendRule", "CSSKeyframesRule");
    }

    findRule(select) {
      need(arguments.length, 1, "findRule", "CSSKeyframesRule");
      const at = keyframeAt(this, str(select));
      return at < 0 ? null : listObjects(ruleChildren(this))[at];
    }

    deleteRule(select) {
      need(arguments.length, 1, "deleteRule", "CSSKeyframesRule");
      const at = keyframeAt(this, str(select));
      if (at >= 0) {
        deleteFromList(ruleChildren(this), at, "deleteRule", "CSSKeyframesRule");
      }
    }
  }

  // The last keyframe whose key is `select`, `from` and `to` as 0% and
  // 100%; -1 for none.
  function keyframeAt(rule, select) {
    const want = ToLowerCase(StringTrim(select));
    const key = want === "from" ? "0%" : want === "to" ? "100%" : want;
    const objects = listObjects(ruleChildren(rule));
    for (let i = objects.length - 1; i >= 0; i--) {
      if (ruleData(ruleState(objects[i]))[2] === key) {
        return i;
      }
    }
    return -1;
  }

  RULE_CLASSES.style = CSSStyleRule;
  RULE_CLASSES.import = CSSImportRule;
  RULE_CLASSES.media = CSSMediaRule;
  RULE_CLASSES["font-face"] = CSSFontFaceRule;
  RULE_CLASSES.page = CSSPageRule;
  RULE_CLASSES.keyframes = CSSKeyframesRule;
  RULE_CLASSES.keyframe = CSSKeyframeRule;
  RULE_CLASSES.supports = CSSSupportsRule;
  RULE_CLASSES.layer = CSSLayerBlockRule;
  RULE_CLASSES["layer-statement"] = CSSLayerStatementRule;
  RULE_CLASSES.namespace = CSSNamespaceRule;
  RULE_CLASSES.container = CSSContainerRule;
  RULE_CLASSES["counter-style"] = CSSCounterStyleRule;
  RULE_CLASSES["font-feature-values"] = CSSFontFeatureValuesRule;
  RULE_CLASSES["font-palette-values"] = CSSFontPaletteValuesRule;
  RULE_CLASSES["nested-declarations"] = CSSNestedDeclarations;
  RULE_CLASSES.scope = CSSScopeRule;
  RULE_CLASSES["starting-style"] = CSSStartingStyleRule;
  RULE_CLASSES.property = CSSPropertyRule;

  // A media list (CSSOM section 4.1) over where its queries are kept:
  // `get` reads their text and `set` writes it, the element's `media`
  // attribute, a rule's or a constructed sheet's own.
  const mediaStates = new $WeakMap();

  function mediaListState(list) {
    const state = WeakMapGet(mediaStates, list);
    if (state === undefined) {
      throw illegalInvocation();
    }
    return state;
  }

  // The queries of a media query list's text, each serialized.
  function mediaQueries(text) {
    return native.cssom("media", null, [], text, 0);
  }

  function mediaItems(state) {
    return mediaQueries(state.get());
  }

  function joinMedia(items) {
    let out = "";
    for (let i = 0; i < items.length; i++) {
      out += (i === 0 ? "" : ", ") + items[i];
    }
    return out;
  }

  class MediaList {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("MediaList");
      }
    }

    get mediaText() {
      return joinMedia(mediaItems(mediaListState(this)));
    }

    set mediaText(text) {
      const state = mediaListState(this);
      state.set(joinMedia(mediaQueries(text === null ? "" : str(text))));
    }

    get length() {
      return mediaItems(mediaListState(this)).length;
    }

    item(index) {
      need(arguments.length, 1, "item", "MediaList");
      const items = mediaItems(mediaListState(this));
      const i = unsigned(index);
      return i < items.length ? items[i] : null;
    }

    // Adds one query, unless it is there already or the text is a list.
    appendMedium(medium) {
      const state = mediaListState(this);
      need(arguments.length, 1, "appendMedium", "MediaList");
      const query = mediaQueries(str(medium));
      if (query.length !== 1) {
        return;
      }
      const items = mediaItems(state);
      if (ArrayIndexOf(items, query[0]) >= 0) {
        return;
      }
      ArrayPush(items, query[0]);
      state.set(joinMedia(items));
    }

    // Removes each query that is the one given, which must be there.
    deleteMedium(medium) {
      const state = mediaListState(this);
      need(arguments.length, 1, "deleteMedium", "MediaList");
      const query = mediaQueries(str(medium));
      const items = mediaItems(state);
      const kept = [];
      for (let i = 0; i < items.length; i++) {
        if (query.length !== 1 || items[i] !== query[0]) {
          ArrayPush(kept, items[i]);
        }
      }
      if (kept.length === items.length) {
        throw domError(
          "NotFoundError",
          "Failed to delete '" + str(medium) + "'.",
          "deleteMedium",
          "MediaList",
        );
      }
      state.set(joinMedia(kept));
    }

    toString() {
      return this.mediaText;
    }
  }

  const mediaHandlers = indexedHandlers((target) => mediaItems(WeakMapGet(mediaStates, target)));

  function mediaList(get, set) {
    const list = new MediaList(KEY);
    const proxy = new $Proxy(list, mediaHandlers);
    const state = { get, set };
    WeakMapSet(mediaStates, list, state);
    WeakMapSet(mediaStates, proxy, state);
    return proxy;
  }

  // ---- The sheets ------------------------------------------------------

  class StyleSheet {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("StyleSheet");
      }
    }

    get type() {
      sheetState(this);
      return "text/css";
    }

    get href() {
      const state = sheetState(this);
      const href = sheetInfo(state)[0];
      return href === "" ? null : href;
    }

    // Null once the element is out of the document or has another
    // sheet.
    get ownerNode() {
      const state = sheetState(this);
      if (state.imports.length !== 0 || state.owner === null) {
        return null;
      }
      if (state.elsewhere) {
        return state.owner;
      }
      return sheetInfo(state)[3] === "1" ? state.owner : null;
    }

    get parentStyleSheet() {
      return sheetState(this).parent;
    }

    get title() {
      const title = sheetInfo(sheetState(this))[1];
      return title === "" ? null : title;
    }

    // An imported sheet's is its rule's; an element's sheet's its `media`
    // attribute; a constructed sheet's its own.
    get media() {
      const state = sheetState(this);
      if (state.ownerRule !== null && state.ownerRule !== undefined) {
        return state.ownerRule.media;
      }
      if (state.media === undefined) {
        state.media = state.constructed
          ? mediaList(
              () => (state.mediaText === undefined ? "" : state.mediaText),
              (text) => {
                state.mediaText = text;
                native.cssom("constructedMedia", null, cssomPath(state, []), text, 0);
              },
            )
          : mediaList(
              () => sheetInfo(state)[2],
              (text) => {
                if (state.owner !== null) {
                  setAttribute(state.handle, "media", text, "media");
                }
              },
            );
      }
      return state.media;
    }

    set media(text) {
      this.media.mediaText = text;
    }

    get disabled() {
      return sheetState(this).disabled === true;
    }

    // The cascade leaves a disabled sheet out (CSSOM section 6.1).
    set disabled(value) {
      const state = sheetState(this);
      state.disabled = !!value;
      if (state.imports.length === 0 && (state.constructed || state.owner !== null)) {
        try {
          native.cssom("disable", state.handle, cssomPath(state, []), "", state.disabled ? 1 : 0);
        } catch (error) {
          // A sheet gone: its flag is the object's alone.
        }
      }
    }
  }

  function sheetInfo(state) {
    try {
      return native.cssom("info", state.handle, cssomPath(state, []), "", 0);
    } catch (error) {
      return ["", "", ""];
    }
  }

  // A constructed sheet (CSSOM 6.1.2's constructed flag), empty, whose
  // rules' URLs resolve against `base` (the document's base URL when
  // empty): its state.
  function constructSheet(sheet, base) {
    const serial = native.cssom("construct", null, [], base, 0)[0];
    const state = {
      owner: null,
      handle: null,
      imports: [],
      serial,
      parent: null,
      ownerRule: null,
      list: null,
      constructed: true,
    };
    state.list = ruleList(state, [], null);
    WeakMapSet(sheetStates, sheet, state);
    return state;
  }

  class CSSStyleSheet extends StyleSheet {
    // Its options (CSSOM section 6.1.2): `media`, as a list or as text,
    // and `disabled`.
    constructor(options) {
      super(KEY);
      if (options !== KEY) {
        // Its `baseURL`, against the document's base URL (CSSOM 6.1.2).
        let base = "";
        if (options !== undefined && options !== null && typeof options === "object" &&
          options.baseURL !== undefined) {
          const url = native.resolveUrl(str(options.baseURL));
          if (url === null) {
            throw domError(
              "NotAllowedError",
              "The base URL is not valid.",
              "constructor",
              "CSSStyleSheet",
            );
          }
          base = url;
        }
        const state = constructSheet(this, base);
        if (options !== undefined && options !== null && typeof options === "object") {
          const media = options.media;
          if (media !== undefined) {
            state.mediaText =
              typeof media === "object" && media !== null && WeakMapGet(mediaStates, media)
                ? media.mediaText
                : joinMedia(mediaQueries(str(media)));
          }
          if (state.mediaText !== undefined) {
            native.cssom("constructedMedia", null, cssomPath(state, []), state.mediaText, 0);
          }
          if (options.disabled !== undefined) {
            state.disabled = !!options.disabled;
            if (state.disabled) {
              native.cssom("disable", null, cssomPath(state, []), "", 1);
            }
          }
        }
      }
    }

    get ownerRule() {
      return sheetState(this).ownerRule;
    }

    get cssRules() {
      return sheetState(this).list.proxy;
    }

    get rules() {
      return sheetState(this).list.proxy;
    }

    insertRule(rule, index = 0) {
      const state = sheetState(this);
      need(arguments.length, 1, "insertRule", "CSSStyleSheet");
      const text = str(rule);
      // A constructed sheet takes no `@import` (CSSOM section 6.1.2's
      // insertRule(), step 3 of "insert a CSS rule").
      if (state.constructed && StringSlice(ToLowerCase(StringTrim(text)), 0, 7) === "@import") {
        throw domError(
          "SyntaxError",
          "Can't insert @import rules into a constructed stylesheet.",
          "insertRule",
          "CSSStyleSheet",
        );
      }
      return insertInList(state.list, text, unsigned(index), "insertRule", "CSSStyleSheet");
    }

    deleteRule(index) {
      const state = sheetState(this);
      need(arguments.length, 1, "deleteRule", "CSSStyleSheet");
      deleteFromList(state.list, unsigned(index), "deleteRule", "CSSStyleSheet");
    }

    // The legacy members (CSSOM section 6.1.2.1): a rule of a selector
    // and a block, at the end unless placed; -1 always.
    addRule(selector = "undefined", style = "undefined", index) {
      const state = sheetState(this);
      const text = str(style) === ""
        ? str(selector) + " { }"
        : str(selector) + " { " + str(style) + " }";
      const at = index === undefined ? listObjects(state.list).length : unsigned(index);
      insertInList(state.list, text, at, "addRule", "CSSStyleSheet");
      return -1;
    }

    removeRule(index = 0) {
      const state = sheetState(this);
      deleteFromList(state.list, unsigned(index), "removeRule", "CSSStyleSheet");
    }

    replace(text) {
      const state = sheetState(this);
      if (state.constructed && !state.elsewhere) {
        native.cssom("replace", null, cssomPath(state, []), str(text), 0);
        return ReflectApply(Promise.resolve, Promise, [this]);
      }
      return PromiseReject(
        domError(
          "NotAllowedError",
          "Can't call replace on non-constructed CSSStyleSheets.",
          "replace",
          "CSSStyleSheet",
        ),
      );
    }

    replaceSync(text) {
      const state = sheetState(this);
      if (state.constructed && !state.elsewhere) {
        native.cssom("replace", null, cssomPath(state, []), str(text), 0);
        return;
      }
      throw domError(
        "NotAllowedError",
        "Can't call replaceSync on non-constructed CSSStyleSheets.",
        "replaceSync",
        "CSSStyleSheet",
      );
    }
  }

  // A document's `styleSheets`: one list a document, its sheets read at
  // each ask.
  const sheetListDocuments = new $WeakMap();
  class StyleSheetList {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("StyleSheetList");
      }
    }

    get length() {
      return documentSheets(listDocument(this)).length;
    }

    item(index) {
      const document = listDocument(this);
      need(arguments.length, 1, "item", "StyleSheetList");
      const sheets = documentSheets(document);
      const i = unsigned(index);
      return i < sheets.length ? sheets[i] : null;
    }
  }
  defineProperty(StyleSheetList.prototype, iteratorSymbol, {
    value: ArrayValues,
    writable: true,
    configurable: true,
  });

  // The document a sheet list is of.
  function listDocument(list) {
    const document = WeakMapGet(sheetListDocuments, list);
    if (document === undefined) {
      throw illegalInvocation();
    }
    return document;
  }

  function documentSheets(document) {
    // A shadow root's are its tree's (D69).
    const shadow = native.kind(document) === 11;
    if (!shadow && !isPage(document)) {
      return otherDocumentSheets(document);
    }
    const flat = native.cssom("sheets", shadow ? document : null, [], "", 0);
    const out = [];
    for (let i = 0; i < flat.length; i += 2) {
      ArrayPush(out, sheetFor(wrap(flat[i]), flat[i + 1], [], null, null));
    }
    return out;
  }

  // The sheets of a document beside the page's (D67), which renders
  // nothing: each of its `style` elements' text as a sheet of its own,
  // made when first asked for and again once the text has changed.
  const otherSheets = new $WeakMap();
  function otherDocumentSheets(document) {
    const styles = native.query(document, "style", true);
    const out = [];
    for (let i = 0; i < styles.length; i++) {
      const element = wrap(styles[i]);
      const ns = nsOf(element);
      if (ns !== HTML_NS && ns !== SVG_NS) {
        continue;
      }
      const text = native.childText(styles[i]);
      let known = WeakMapGet(otherSheets, element);
      if (known === undefined || known.text !== text) {
        const sheet = new CSSStyleSheet(KEY);
        // Kept by the host as a constructed sheet is, but not one: it
        // cannot be replaced or adopted.
        const state = constructSheet(sheet, documentInfo(document)[5]);
        state.owner = element;
        state.elsewhere = true;
        native.cssom("replace", null, cssomPath(state, []), text, 0);
        known = { text, sheet };
        WeakMapSet(otherSheets, element, known);
      }
      ArrayPush(out, known.sheet);
    }
    return out;
  }

  const sheetLists = new $WeakMap();
  const sheetListHandlers = {
    get(target, key, receiver) {
      const i = arrayIndex(key);
      if (i >= 0) {
        const sheets = documentSheets(WeakMapGet(sheetListDocuments, target));
        return i < sheets.length ? sheets[i] : undefined;
      }
      return ReflectGet(target, key, receiver);
    },
    has(target, key) {
      const i = arrayIndex(key);
      return i >= 0
        ? i < documentSheets(WeakMapGet(sheetListDocuments, target)).length
        : ReflectHas(target, key);
    },
  };

  include(
    {
      get styleSheets() {
        const handle = self(this);
        let list = WeakMapGet(sheetLists, this);
        if (list === undefined) {
          const target = new StyleSheetList(KEY);
          list = new $Proxy(target, sheetListHandlers);
          WeakMapSet(sheetListDocuments, target, handle);
          WeakMapSet(sheetListDocuments, list, handle);
          WeakMapSet(sheetLists, this, list);
        }
        return list;
      },
    },
    [Document.prototype, ShadowRoot.prototype],
  );

  // `document.adoptedStyleSheets` (CSSOM 6.2): the constructed sheets
  // the document's styles take after its own, by document, and a shadow
  // root's its tree's (D69); an array the page sets whole.
  const adoptedSheets = new $WeakMap();
  include(
    {
      get adoptedStyleSheets() {
        self(this);
        let sheets = WeakMapGet(adoptedSheets, this);
        if (sheets === undefined) {
          sheets = [];
          WeakMapSet(adoptedSheets, this, sheets);
        }
        return sheets;
      },
      set adoptedStyleSheets(value) {
        const handle = self(this);
        const iface = kindOf(this) === 11 ? "ShadowRoot" : "Document";
        const sheets = [];
        const serials = [];
        if (value === null || (typeof value !== "object" && typeof value !== "function")) {
          throw new $TypeError(
            "Failed to set the 'adoptedStyleSheets' property on '" + iface + "': The provided " +
              "value cannot be converted to a sequence.",
          );
        }
        for (const sheet of value) {
          const state = WeakMapGet(sheetStates, sheet);
          if (state === undefined) {
            throw new $TypeError(
              "Failed to set the 'adoptedStyleSheets' property on '" + iface + "': Failed to " +
                "convert value to 'CSSStyleSheet'.",
            );
          }
          if (!state.constructed || state.elsewhere) {
            throw new $DOMException(
              "Failed to set the 'adoptedStyleSheets' property on '" + iface + "': Can't adopt " +
                "non-constructed stylesheets.",
              "NotAllowedError",
            );
          }
          ArrayPush(sheets, sheet);
          ArrayPush(serials, state.serial);
        }
        if (this === theDocument()) {
          native.cssom("adopt", null, serials, "", 0);
        } else if (iface === "ShadowRoot") {
          native.cssom("adopt", handle, serials, "", 0);
        }
        WeakMapSet(adoptedSheets, this, sheets);
      },
    },
    [Document.prototype, ShadowRoot.prototype],
  );

  // A CSS module script's style sheet (HTML's "create a CSS module
  // script"): constructed of `text`, its URLs resolving against `base`.
  function cssModule(text, base) {
    const sheet = new CSSStyleSheet(KEY);
    const state = constructSheet(sheet, base);
    native.cssom("replace", null, cssomPath(state, []), text, 0);
    return sheet;
  }

  // `<style>.sheet` and `<link>.sheet`: the element's sheet, or null.
  const elementSheet = {
    get sheet() {
      const handle = self(this);
      const serial = native.cssom("sheet", handle, [0], "", 0);
      return serial.length === 0 ? null : sheetFor(this, serial[0], [], null, null);
    },
  };

  expose(StyleSheet, "StyleSheet");
  expose(CSSStyleSheet, "CSSStyleSheet");
  expose(StyleSheetList, "StyleSheetList");
  expose(CSSRuleList, "CSSRuleList");
  expose(MediaList, "MediaList");
  expose(CSSRule, "CSSRule");
  expose(CSSStyleRule, "CSSStyleRule");
  expose(CSSImportRule, "CSSImportRule");
  expose(CSSGroupingRule, "CSSGroupingRule");
  expose(CSSConditionRule, "CSSConditionRule");
  expose(CSSMediaRule, "CSSMediaRule");
  expose(CSSSupportsRule, "CSSSupportsRule");
  expose(CSSLayerBlockRule, "CSSLayerBlockRule");
  expose(CSSLayerStatementRule, "CSSLayerStatementRule");
  expose(CSSNamespaceRule, "CSSNamespaceRule");
  expose(CSSContainerRule, "CSSContainerRule");
  expose(CSSCounterStyleRule, "CSSCounterStyleRule");
  expose(CSSFontFeatureValuesRule, "CSSFontFeatureValuesRule");
  expose(CSSFontPaletteValuesRule, "CSSFontPaletteValuesRule");
  expose(CSSNestedDeclarations, "CSSNestedDeclarations");
  expose(CSSScopeRule, "CSSScopeRule");
  expose(CSSStartingStyleRule, "CSSStartingStyleRule");
  expose(CSSPropertyRule, "CSSPropertyRule");
  expose(CSSFontFeatureValuesMap, "CSSFontFeatureValuesMap");
  expose(CSSPageRule, "CSSPageRule");
  expose(CSSFontFaceRule, "CSSFontFaceRule");
  expose(CSSKeyframeRule, "CSSKeyframeRule");
  expose(CSSKeyframesRule, "CSSKeyframesRule");

  // A computed style (CSSOM's getComputedStyle): the element's values
  // read through the host as they stand when asked, nothing written.
  function computedStyle(owner, handle, pseudo) {
    const target = new CSSStyleDeclaration(KEY);
    const state = { owner, handle, pseudo, computed: true };
    definePropertyAttributes();
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

  // The properties a computed style lists: none for a pseudo-element
  // that is not one (CSSOM's getComputedStyle(), step 4); else those,
  // then the element's custom properties with a value, which the host
  // gives for the name `--` parted by NULs (D73).
  function computedNamesOf(state) {
    const names = [];
    if (state.pseudo === "invalid") {
      return names;
    }
    const listed = computedProperties();
    for (let i = 0; i < listed.length; i++) {
      ArrayPush(names, listed[i]);
    }
    const custom = native.computedValue(state.handle, state.pseudo, "--");
    if (custom !== null && custom !== "") {
      const parts = StringSplit(custom, "\u0000");
      for (let i = 0; i < parts.length; i++) {
        ArrayPush(names, parts[i]);
      }
    }
    return names;
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
    const name = state.page ? pageDescriptor(property) : canonicalProperty(property);
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
    if (state.page
      ? native.parsePageDescriptors(name + ": " + value).length === 0
      : !native.cssValid(name, value)) {
      return;
    }
    const flat = declarationsOf(state);
    const at = findDeclaration(flat, name);
    const trimmed = trimSpace(value);
    if (at < 0) {
      ArrayPush(flat, name, trimmed, wanted);
    } else if (flat[at + 1] === trimmed && flat[at + 2] === wanted) {
      // Unchanged: the style attribute is left as it is.
      return;
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
      // A shorthand kept as its longhands: they go.
      const longhands = longhandsOf(name);
      if (longhands.length === 0) return "";
      const old = native.cssShorthand(name, flat);
      let removed = false;
      for (let i = 0; i < longhands.length; i++) {
        const found = findDeclaration(flat, longhands[i]);
        if (found >= 0) {
          ArraySplice(flat, found, 3);
          removed = true;
        }
      }
      if (removed) writeDeclarations(state, flat, "removeProperty");
      return old;
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

  // A property's value in a declaration block: computed, or declared,
  // a shorthand's from its longhands.
  function propertyValue(state, name) {
    if (state.computed) {
      return computedValue(state, name);
    }
    const flat = declarationsOf(state);
    const at = findDeclaration(flat, name);
    if (at >= 0) return flat[at + 1];
    return flat.length === 0 ? "" : native.cssShorthand(name, flat);
  }

  // Each property a computed style lists is an attribute of the
  // interface, dashed and in camel case, as the reference browser has
  // them (CSSOM section 6.6.1): its getter and setter the block's, on
  // the prototype, made once the first block is.
  let propertyAttributes = false;
  function definePropertyAttributes() {
    if (propertyAttributes) {
      return;
    }
    propertyAttributes = true;
    // One attribute, `key`, of the property `name`.
    const define = (key, name) => {
      if (ReflectHas(CSSStyleDeclaration.prototype, key)) {
        return;
      }
      defineProperty(CSSStyleDeclaration.prototype, key, {
        get() {
          return propertyValue(blockState(this), name);
        },
        set(value) {
          const state = blockState(this);
          if (state.computed) {
            throw new $DOMException(
              "Failed to set the '" + key + "' property on 'CSSStyleDeclaration': These " +
                "styles are computed, and therefore the '" + name + "' property is read-only.",
              "NoModificationAllowedError",
            );
          }
          setProperty(state, name, strOrEmpty(value), "", key);
        },
        enumerable: true,
        configurable: true,
      });
    };
    const listed = computedProperties();
    for (let i = 0; i < listed.length; i++) {
      const name = listed[i];
      let camel = "";
      let upper = false;
      for (let j = StringIndexOf(name, "-webkit-") === 0 ? 1 : 0; j < name.length; j++) {
        if (name[j] === "-") {
          upper = true;
        } else {
          camel += upper ? ToUpperCase(name[j]) : name[j];
          upper = false;
        }
      }
      define(name, name);
      define(camel, name);
    }
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
          const names = computedNamesOf(state);
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
      return propertyValue(state, name);
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
  // A template's contents (D58), a fragment the element owns; and what
  // its attributes make of a declarative shadow root (D68): its
  // `shadowrootmode`, `open` or `closed` in any case, or else empty.
  include(
    {
      get content() {
        return wrap(native.templateContent(self(this)));
      },

      get shadowRootMode() {
        const value = native.attribute(self(this), "shadowrootmode");
        const lower = value === null ? "" : ToLowerCase(value);
        return lower === "open" || lower === "closed" ? lower : "";
      },

      set shadowRootMode(value) {
        setAttribute(self(this), "shadowrootmode", str(value), "shadowRootMode");
      },

      // `shadowrootslotassignment`: `manual` in any case, else `named`.
      get shadowRootSlotAssignment() {
        const value = native.attribute(self(this), "shadowrootslotassignment");
        return value !== null && ToLowerCase(value) === "manual" ? "manual" : "named";
      },

      set shadowRootSlotAssignment(value) {
        setAttribute(self(this), "shadowrootslotassignment", str(value),
          "shadowRootSlotAssignment");
      },

      // `shadowrootreferencetarget`, null when absent.
      get shadowRootReferenceTarget() {
        return native.attribute(self(this), "shadowrootreferencetarget");
      },

      set shadowRootReferenceTarget(value) {
        const handle = self(this);
        if (value === null) {
          native.removeAttribute(handle, "shadowrootreferencetarget");
        } else {
          setAttribute(handle, "shadowrootreferencetarget", str(value),
            "shadowRootReferenceTarget");
        }
      },
    },
    [MapGet(interfaces, "template").prototype],
  );
  // ---- Tables (HTML section 4.9) ---------------------------------------

  // Whether `handle` is an HTML element whose local name is one of
  // `names`.
  function isHtmlNamed(handle, names) {
    if (native.kind(handle) !== 1) {
      return false;
    }
    const element = wrap(handle);
    return nsOf(element) === HTML_NS && includesString(names, element.localName);
  }

  // The children of `handle` that are HTML elements named one of `names`.
  function childrenNamed(handle, names) {
    const found = [];
    const children = native.elementChildren(handle);
    for (let i = 0; i < children.length; i++) {
      if (isHtmlNamed(children[i], names)) {
        ArrayPush(found, children[i]);
      }
    }
    return found;
  }

  function firstChildNamed(handle, name) {
    const children = native.elementChildren(handle);
    for (let i = 0; i < children.length; i++) {
      if (isHtmlNamed(children[i], [name])) {
        return children[i];
      }
    }
    return null;
  }

  // A table's `rows` (section 4.9.1): the rows of its `thead`s, then its
  // own and its `tbody`s', then its `tfoot`s', each in tree order.
  function tableRows(table) {
    const head = [];
    const body = [];
    const foot = [];
    const children = native.elementChildren(table);
    for (let i = 0; i < children.length; i++) {
      const child = children[i];
      if (isHtmlNamed(child, ["tr"])) {
        ArrayPush(body, child);
      } else if (isHtmlNamed(child, ["thead", "tbody", "tfoot"])) {
        const rows = childrenNamed(child, ["tr"]);
        const into = native.tag(child) === "thead" ? head : native.tag(child) === "tbody"
          ? body : foot;
        for (let j = 0; j < rows.length; j++) {
          ArrayPush(into, rows[j]);
        }
      }
    }
    for (let i = 0; i < body.length; i++) {
      ArrayPush(head, body[i]);
    }
    for (let i = 0; i < foot.length; i++) {
      ArrayPush(head, foot[i]);
    }
    return head;
  }

  // A new HTML element named `name` of the document `handle` is of.
  function createIn(handle, name) {
    return native.createElementNS(HTML_NS, name, native.ownerDocument(handle));
  }

  // The live collection of `fetch`'s handles that `name` of `element`
  // is, made at its first ask and the same object after.
  const tableCollections = new $WeakMap();
  function tableCollection(element, name, fetch) {
    let made = WeakMapGet(tableCollections, element);
    if (made === undefined) {
      made = {};
      WeakMapSet(tableCollections, element, made);
    }
    if (made[name] === undefined) {
      const handle = self(element);
      made[name] = collection(HTMLCollection, element, () => fetch(handle), true);
    }
    return made[name];
  }

  // A `long` argument, as Web IDL converts it.
  function longArgument(value) {
    return $Number(value) | 0;
  }

  // Removes `handle` from its parent, if it has one.
  function removeFromParent(handle) {
    const parent = native.parent(handle);
    if (parent !== null) {
      native.remove(parent, handle);
    }
  }

  // The `tr` a section's or a table's `insertRow` made, put before the
  // `index`th of `rows` in its parent, or after the last in `parent`.
  function insertRowAt(rows, index, parent, owner) {
    const row = createIn(owner, "tr");
    if (index === -1 || index === rows.length) {
      native.insert(parent, row, null);
    } else {
      native.insert(native.parent(rows[index]), row, rows[index]);
    }
    return wrap(row);
  }

  // A slot's members (HTML section 4.12.4, D68): its assigned nodes and
  // elements, flattened when asked, and `assign()`.
  function assignedOf(handle, options) {
    const flatten = typeof options === "object" && options !== null && !!options.flatten;
    return native.assignedNodes(handle, flatten);
  }
  include(
    {
      assignedNodes(options) {
        const handle = self(this);
        const nodes = assignedOf(handle, options);
        const out = [];
        for (let i = 0; i < nodes.length; i++) {
          ArrayPush(out, wrap(nodes[i]));
        }
        return out;
      },

      assignedElements(options) {
        const handle = self(this);
        const nodes = assignedOf(handle, options);
        const out = [];
        for (let i = 0; i < nodes.length; i++) {
          if (native.kind(nodes[i]) === 1) {
            ArrayPush(out, wrap(nodes[i]));
          }
        }
        return out;
      },

      assign(...nodes) {
        const handle = self(this);
        const handles = [];
        for (let i = 0; i < nodes.length; i++) {
          const node = nodes[i];
          if (!isNode(node) || (kindOf(node) !== 1 && kindOf(node) !== 3)) {
            throw new $TypeError(prefix("assign", "HTMLSlotElement") + "parameter " + (i + 1) +
              " is not of type '(Element or Text)'.");
          }
          ArrayPush(handles, handleOf(node));
        }
        native.assignSlot(handle, handles);
      },
    },
    [MapGet(interfaces, "slot").prototype],
  );

  include(
    {
      get caption() {
        return wrap(firstChildNamed(self(this), "caption"));
      },

      set caption(value) {
        const handle = self(this);
        if (value !== null && !(value instanceof MapGet(interfaces, "caption"))) {
          throw new $TypeError("Failed to set the 'caption' property on 'HTMLTableElement': " +
            "Failed to convert value to 'HTMLTableCaptionElement'.");
        }
        const old = firstChildNamed(handle, "caption");
        if (old !== null) {
          native.remove(handle, old);
        }
        if (value !== null) {
          try {
            native.insert(handle, self(value), native.firstChild(handle));
          } catch (error) {
            throw failedSet(error, "caption", "HTMLTableElement");
          }
        }
      },

      createCaption() {
        const handle = self(this);
        let caption = firstChildNamed(handle, "caption");
        if (caption === null) {
          caption = createIn(handle, "caption");
          native.insert(handle, caption, native.firstChild(handle));
        }
        return wrap(caption);
      },

      deleteCaption() {
        const handle = self(this);
        const caption = firstChildNamed(handle, "caption");
        if (caption !== null) {
          native.remove(handle, caption);
        }
      },

      get tHead() {
        return wrap(firstChildNamed(self(this), "thead"));
      },

      set tHead(value) {
        tableSection(this, value, "thead", "tHead");
      },

      createTHead() {
        const handle = self(this);
        let head = firstChildNamed(handle, "thead");
        if (head === null) {
          head = createIn(handle, "thead");
          native.insert(handle, head, afterCaptions(handle));
        }
        return wrap(head);
      },

      deleteTHead() {
        const handle = self(this);
        const head = firstChildNamed(handle, "thead");
        if (head !== null) {
          native.remove(handle, head);
        }
      },

      get tFoot() {
        return wrap(firstChildNamed(self(this), "tfoot"));
      },

      set tFoot(value) {
        tableSection(this, value, "tfoot", "tFoot");
      },

      createTFoot() {
        const handle = self(this);
        let foot = firstChildNamed(handle, "tfoot");
        if (foot === null) {
          foot = createIn(handle, "tfoot");
          native.insert(handle, foot, null);
        }
        return wrap(foot);
      },

      deleteTFoot() {
        const handle = self(this);
        const foot = firstChildNamed(handle, "tfoot");
        if (foot !== null) {
          native.remove(handle, foot);
        }
      },

      get tBodies() {
        self(this);
        return tableCollection(this, "tBodies", (handle) => childrenNamed(handle, ["tbody"]));
      },

      // A new `tbody` after the table's last, or at its end.
      createTBody() {
        const handle = self(this);
        const bodies = childrenNamed(handle, ["tbody"]);
        const body = createIn(handle, "tbody");
        const before = bodies.length === 0 ? null
          : native.nextSibling(bodies[bodies.length - 1]);
        native.insert(handle, body, before);
        return wrap(body);
      },

      get rows() {
        self(this);
        return tableCollection(this, "rows", tableRows);
      },

      // A new row (section 4.9.1): in a new `tbody` of a table with no
      // rows and no `tbody`, in its last `tbody` when it has no rows, else
      // before the `index`th row or after the last in that row's parent.
      insertRow(index = -1) {
        const handle = self(this);
        const at = longArgument(index);
        const rows = tableRows(handle);
        if (at < -1) {
          throw domError("IndexSizeError", "The index provided (" + at + ") is less than -1.",
            "insertRow", "HTMLTableElement");
        }
        if (at > rows.length) {
          throw domError("IndexSizeError", "The index provided (" + at +
            ") is greater than the number of rows in the table (" + rows.length + ").",
          "insertRow", "HTMLTableElement");
        }
        if (rows.length === 0) {
          const bodies = childrenNamed(handle, ["tbody"]);
          const row = createIn(handle, "tr");
          if (bodies.length === 0) {
            const body = createIn(handle, "tbody");
            native.insert(body, row, null);
            native.insert(handle, body, null);
          } else {
            native.insert(bodies[bodies.length - 1], row, null);
          }
          return wrap(row);
        }
        const last = native.parent(rows[rows.length - 1]);
        return insertRowAt(rows, at, last, handle);
      },

      deleteRow(index) {
        const handle = self(this);
        need(arguments.length, 1, "deleteRow", "HTMLTableElement");
        const at = longArgument(index);
        const rows = tableRows(handle);
        if (at < -1) {
          throw domError("IndexSizeError", "The index provided (" + at + ") is less than -1.",
            "deleteRow", "HTMLTableElement");
        }
        if (at >= rows.length) {
          throw domError("IndexSizeError", "The index provided (" + at +
            ") is greater than the number of rows in the table (" + rows.length + ").",
          "deleteRow", "HTMLTableElement");
        }
        if (at === -1) {
          if (rows.length > 0) {
            removeFromParent(rows[rows.length - 1]);
          }
          return;
        }
        removeFromParent(rows[at]);
      },
    },
    [MapGet(interfaces, "table").prototype],
  );

  // The first child of `table` that is neither a `caption` nor a
  // `colgroup`, where a new `thead` goes.
  function afterCaptions(table) {
    let child = native.firstChild(table);
    while (child !== null && (native.kind(child) !== 1 ||
      isHtmlNamed(child, ["caption", "colgroup"]))) {
      child = native.nextSibling(child);
    }
    return child;
  }

  // The `tHead` and `tFoot` setters: `value` in place of the table's
  // first `name`, a `thead` after its captions and column groups, a
  // `tfoot` at its end.
  function tableSection(table, value, name, member) {
    const handle = self(table);
    if (value !== null && !(value instanceof MapGet(interfaces, "tbody"))) {
      throw new $TypeError("Failed to set the '" + member + "' property on " +
        "'HTMLTableElement': Failed to convert value to 'HTMLTableSectionElement'.");
    }
    if (value !== null && value.localName !== name) {
      throw new $DOMException("Failed to set the '" + member + "' property on " +
        "'HTMLTableElement': Not a " + name + " element.", "HierarchyRequestError");
    }
    const old = firstChildNamed(handle, name);
    if (old !== null) {
      native.remove(handle, old);
    }
    if (value !== null) {
      const before = name === "thead" ? afterCaptions(handle) : null;
      try {
        native.insert(handle, self(value), before);
      } catch (error) {
        throw failedSet(error, member, "HTMLTableElement");
      }
    }
  }

  include(
    {
      get rows() {
        self(this);
        return tableCollection(this, "rows", (handle) => childrenNamed(handle, ["tr"]));
      },

      insertRow(index = -1) {
        const handle = self(this);
        const at = longArgument(index);
        const rows = childrenNamed(handle, ["tr"]);
        if (at < -1 || at > rows.length) {
          // The reference browser's words, its parenthesis unclosed.
          throw domError("IndexSizeError", "The provided index (" + at +
            " is outside the range [-1, " + rows.length + "].",
          "insertRow", "HTMLTableSectionElement");
        }
        return insertRowAt(rows, at, handle, handle);
      },

      deleteRow(index) {
        const handle = self(this);
        need(arguments.length, 1, "deleteRow", "HTMLTableSectionElement");
        const at = longArgument(index);
        const rows = childrenNamed(handle, ["tr"]);
        if (at === -1) {
          if (rows.length > 0) {
            native.remove(handle, rows[rows.length - 1]);
          }
          return;
        }
        if (at < -1 || at >= rows.length) {
          throw domError("IndexSizeError", "The provided index (" + at +
            " is outside the range [-1, " + rows.length + "].",
          "deleteRow", "HTMLTableSectionElement");
        }
        native.remove(handle, rows[at]);
      },
    },
    [MapGet(interfaces, "tbody").prototype],
  );

  // A row's index in `rows`, or -1.
  function indexIn(rows, handle) {
    for (let i = 0; i < rows.length; i++) {
      if (rows[i] === handle) {
        return i;
      }
    }
    return -1;
  }

  include(
    {
      // Its index in its table's `rows`, its parent the table or one of
      // the table's sections; else -1.
      get rowIndex() {
        const handle = self(this);
        const parent = native.parent(handle);
        if (parent === null) {
          return -1;
        }
        if (isHtmlNamed(parent, ["table"])) {
          return indexIn(tableRows(parent), handle);
        }
        if (isHtmlNamed(parent, ["thead", "tbody", "tfoot"])) {
          const table = native.parent(parent);
          if (table !== null && isHtmlNamed(table, ["table"])) {
            return indexIn(tableRows(table), handle);
          }
        }
        return -1;
      },

      // Its index in its parent's `rows`, a table's or a section's.
      get sectionRowIndex() {
        const handle = self(this);
        const parent = native.parent(handle);
        if (parent === null) {
          return -1;
        }
        if (isHtmlNamed(parent, ["table"])) {
          return indexIn(tableRows(parent), handle);
        }
        if (isHtmlNamed(parent, ["thead", "tbody", "tfoot"])) {
          return indexIn(childrenNamed(parent, ["tr"]), handle);
        }
        return -1;
      },

      get cells() {
        self(this);
        return tableCollection(this, "cells", (handle) => childrenNamed(handle, ["td", "th"]));
      },

      insertCell(index = -1) {
        const handle = self(this);
        const at = longArgument(index);
        const cells = childrenNamed(handle, ["td", "th"]);
        if (at < -1 || at > cells.length) {
          throw domError("IndexSizeError", "The value provided (" + at +
            ") is outside the range [-1, " + cells.length + "].",
          "insertCell", "HTMLTableRowElement");
        }
        const cell = createIn(handle, "td");
        native.insert(handle, cell, at === -1 || at === cells.length ? null : cells[at]);
        return wrap(cell);
      },

      deleteCell(index) {
        const handle = self(this);
        need(arguments.length, 1, "deleteCell", "HTMLTableRowElement");
        const at = longArgument(index);
        const cells = childrenNamed(handle, ["td", "th"]);
        if (at === -1) {
          if (cells.length > 0) {
            native.remove(handle, cells[cells.length - 1]);
          }
          return;
        }
        if (at < -1 || at >= cells.length) {
          throw domError("IndexSizeError", "The value provided (" + at +
            ") is outside the range [0, " + cells.length + ").",
          "deleteCell", "HTMLTableRowElement");
        }
        native.remove(handle, cells[at]);
      },
    },
    [MapGet(interfaces, "tr").prototype],
  );

  include(
    {
      // Its index in its row's `cells`, or -1.
      get cellIndex() {
        const handle = self(this);
        const parent = native.parent(handle);
        if (parent === null || !isHtmlNamed(parent, ["tr"])) {
          return -1;
        }
        return indexIn(childrenNamed(parent, ["td", "th"]), handle);
      },
    },
    [MapGet(interfaces, "td").prototype],
  );

  // ---- The Sanitizer API (HTML section 8.5.3, as Chrome 156 ships it) ----

  // A configuration is kept canonical: each element a name and a
  // namespace (null for none), with its own `attributes` or
  // `removeAttributes`; each attribute a name and a namespace; each
  // processing instruction a target; an allow list or a remove list of
  // elements, of attributes and of processing instructions, never both;
  // `comments`, and `dataAttributes` with an allow list of attributes.

  // The built-in safe default configuration, the reference browser's:
  // the elements of each namespace and their own attributes, and the
  // attributes allowed on any element.
  const SAFE_DEFAULT_ELEMENTS = [
    [MATHML_NS, "math merror mfrac mi mmultiscripts mn mo mover mpadded mphantom " +
      "mprescripts mroot mrow ms mspace msqrt mstyle msub msubsup msup mtable mtd mtext mtr " +
      "munder munderover semantics", {
      __proto__: null,
      mo: "fence form largeop lspace maxsize minsize movablelimits rspace separator " +
        "stretchy symmetric",
      mover: "accent",
      mpadded: "depth height lspace voffset width",
      mspace: "depth height width",
      mtd: "columnspan rowspan",
      munder: "accentunder",
      munderover: "accent accentunder",
    }],
    [HTML_NS, "a abbr address article aside b bdi bdo blockquote body br caption cite " +
      "code col colgroup data dd del dfn div dl dt em figcaption figure footer h1 h2 h3 h4 h5 " +
      "h6 head header hgroup hr html i ins kbd li main mark menu nav ol p pre q rp rt ruby s " +
      "samp search section small span strong sub sup table tbody td tfoot th thead time " +
      "title tr u ul var wbr", {
      __proto__: null,
      a: "href hreflang type",
      blockquote: "cite",
      col: "span",
      colgroup: "span",
      data: "value",
      del: "cite datetime",
      ins: "cite datetime",
      li: "value",
      ol: "reversed start type",
      td: "colspan headers rowspan",
      th: "abbr colspan headers rowspan scope",
      time: "datetime",
    }],
    [SVG_NS, "a circle defs desc ellipse foreignObject g line marker metadata path " +
      "polygon polyline rect svg text textPath title tspan", {
      __proto__: null,
      a: "href hreflang type",
      circle: "cx cy pathLength r",
      ellipse: "cx cy pathLength rx ry",
      foreignObject: "height width x y",
      line: "pathLength x1 x2 y1 y2",
      marker: "markerHeight markerUnits markerWidth orient preserveAspectRatio refX refY " +
        "viewBox",
      path: "d pathLength",
      polygon: "pathLength points",
      polyline: "pathLength points",
      rect: "height pathLength rx ry width x y",
      svg: "height preserveAspectRatio viewBox width x y",
      text: "dx dy lengthAdjust rotate textLength x y",
      textPath: "lengthAdjust method path side spacing startOffset textLength",
      tspan: "dx dy lengthAdjust rotate textLength x y",
    }],
  ];
  const SAFE_DEFAULT_ATTRIBUTES = "alignment-baseline baseline-shift clip-path clip-rule " +
    "color color-interpolation cursor dir direction display displaystyle dominant-baseline " +
    "fill fill-opacity fill-rule font-family font-size font-size-adjust font-stretch " +
    "font-style font-variant font-weight lang letter-spacing marker-end marker-mid " +
    "marker-start mathbackground mathcolor mathsize opacity paint-order pointer-events " +
    "scriptlevel shape-rendering stop-color stop-opacity stroke stroke-dasharray " +
    "stroke-dashoffset stroke-linecap stroke-linejoin stroke-miterlimit stroke-opacity " +
    "stroke-width text-anchor text-decoration text-overflow text-rendering title transform " +
    "transform-origin unicode-bidi vector-effect visibility white-space word-spacing " +
    "writing-mode";

  // The built-in safe baseline: the elements that run script or load a
  // document, and the event handler content attributes, the reference
  // browser's.
  const SAFE_BASELINE_ELEMENTS = [
    [HTML_NS, "base"], [HTML_NS, "embed"], [HTML_NS, "frame"], [HTML_NS, "iframe"],
    [HTML_NS, "object"], [HTML_NS, "script"], [SVG_NS, "script"], [SVG_NS, "use"],
  ];
  const EVENT_HANDLER_ATTRIBUTES = "onabort onactivate onafterprint onanimationcancel " +
    "onanimationend onanimationiteration onanimationstart onautofill onauxclick onbeforecopy " +
    "onbeforecut onbeforefilter onbeforeinput onbeforepaste onbeforeprint onbeforetoggle " +
    "onbeforeunload onbegin onblur oncancel oncanplay oncanplaythrough onchange onclick " +
    "onclose oncommand oncontentvisibilityautostatechange oncontextlost oncontextmenu " +
    "oncontextrestored oncopy oncuechange oncut ondblclick ondrag ondragend ondragenter " +
    "ondragleave ondragover ondragstart ondrop ondurationchange onemptied onend onended " +
    "onerror onfocus onfocusin onfocusout onformdata ongotpointercapture onhashchange " +
    "oninput oninstallresult oninvalid onkeydown onkeypress onkeyup onlanguagechange onload " +
    "onloadeddata onloadedmetadata onloadstart onlocation onlostpointercapture onmessage " +
    "onmessageerror onmousedown onmouseenter onmouseleave onmousemove onmouseout " +
    "onmouseover onmouseup onmousewheel onmove onoffline ononline onorientationchange " +
    "onpagehide onpageshow onpaste onpause onplay onplaying onpointercancel onpointerdown " +
    "onpointerenter onpointerleave onpointermove onpointerout onpointerover " +
    "onpointerrawupdate onpointerup onpopstate onprogress onpromptaction onpromptdismiss " +
    "onratechange onrepeat onreset onresize onscroll onscrollend onscrollsnapchange " +
    "onscrollsnapchanging onsearch onsecuritypolicyviolation onseeked onseeking onselect " +
    "onselectionchange onselectstart onshow onslotchange onstalled onstorage onstream " +
    "onsubmit onsuspend ontimeupdate ontimezonechange ontoggle ontouchcancel ontouchend " +
    "ontouchmove ontouchstart ontransitionend onunload onvalidationstatuschange " +
    "onvolumechange onwaiting onwebkitanimationend onwebkitanimationiteration " +
    "onwebkitanimationstart onwebkitfullscreenchange onwebkitfullscreenerror " +
    "onwebkittransitionend onwheel";

  // The words of `text`, split at spaces.
  function words(text) {
    const out = [];
    let word = "";
    for (let i = 0; i <= text.length; i++) {
      if (i === text.length || text[i] === " ") {
        if (word !== "") {
          ArrayPush(out, word);
        }
        word = "";
      } else {
        word += text[i];
      }
    }
    return out;
  }

  function noNamespace(names) {
    const out = [];
    const list = words(names);
    for (let i = 0; i < list.length; i++) {
      ArrayPush(out, { name: list[i], namespace: null });
    }
    return out;
  }

  // The built-in safe default configuration, made once; each Sanitizer
  // made of it is given a copy.
  let safeDefault = null;
  function safeDefaultConfig() {
    if (safeDefault === null) {
      const elements = [];
      for (let i = 0; i < SAFE_DEFAULT_ELEMENTS.length; i++) {
        const [ns, names, own] = SAFE_DEFAULT_ELEMENTS[i];
        const list = words(names);
        for (let j = 0; j < list.length; j++) {
          const attributes = own[list[j]] !== undefined ? noNamespace(own[list[j]]) : [];
          ArrayPush(elements, { name: list[j], namespace: ns, attributes });
        }
      }
      safeDefault = {
        elements,
        attributes: noNamespace(SAFE_DEFAULT_ATTRIBUTES),
        processingInstructions: [],
        comments: false,
        dataAttributes: false,
      };
    }
    return safeDefault;
  }

  // ---- Names and lists

  function sameName(a, b) {
    return a.name === b.name && a.namespace === b.namespace;
  }

  function sameTarget(a, b) {
    return a.target === b.target;
  }

  function indexOfItem(list, item, same) {
    for (let i = 0; i < list.length; i++) {
      if (same(list[i], item)) {
        return i;
      }
    }
    return -1;
  }

  function hasItem(list, item, same = sameName) {
    return list !== undefined && indexOfItem(list, item, same) >= 0;
  }

  // Removes `item` from `list`; whether it was there.
  function removeItem(list, item, same = sameName) {
    if (list === undefined) {
      return false;
    }
    const at = indexOfItem(list, item, same);
    if (at < 0) {
      return false;
    }
    ArraySplice(list, at, 1);
    return true;
  }

  function hasDuplicates(list, same = sameName) {
    if (list === undefined) {
      return false;
    }
    for (let i = 0; i < list.length; i++) {
      for (let j = i + 1; j < list.length; j++) {
        if (same(list[i], list[j])) {
          return true;
        }
      }
    }
    return false;
  }

  function intersects(a, b) {
    if (a === undefined || b === undefined) {
      return false;
    }
    for (let i = 0; i < a.length; i++) {
      if (hasItem(b, a[i])) {
        return true;
      }
    }
    return false;
  }

  // A custom data attribute's name (HTML section 3.2.6.6), of no
  // namespace.
  function isDataAttribute(attribute) {
    return attribute.namespace === null && attribute.name.length > 5 &&
      StringSlice(attribute.name, 0, 5) === "data-";
  }

  function copyList(list) {
    if (list === undefined) {
      return undefined;
    }
    const out = [];
    for (let i = 0; i < list.length; i++) {
      const item = list[i];
      const copy = item.target !== undefined ? { target: item.target }
        : { name: item.name, namespace: item.namespace };
      if (item.attributes !== undefined) {
        copy.attributes = copyList(item.attributes);
      }
      if (item.removeAttributes !== undefined) {
        copy.removeAttributes = copyList(item.removeAttributes);
      }
      ArrayPush(out, copy);
    }
    return out;
  }

  const CONFIG_LISTS = [
    "attributes", "elements", "processingInstructions", "removeAttributes", "removeElements",
    "removeProcessingInstructions", "replaceWithChildrenElements",
  ];

  function copyConfig(config) {
    const out = {};
    for (let i = 0; i < CONFIG_LISTS.length; i++) {
      if (config[CONFIG_LISTS[i]] !== undefined) {
        out[CONFIG_LISTS[i]] = copyList(config[CONFIG_LISTS[i]]);
      }
    }
    out.comments = config.comments;
    if (config.dataAttributes !== undefined) {
      out.dataAttributes = config.dataAttributes;
    }
    return out;
  }

  // ---- Converting a configuration (Web IDL's dictionaries and unions)

  function isObjectValue(value) {
    return (typeof value === "object" && value !== null) || typeof value === "function";
  }

  // A sequence of `value`, each item converted by `convert`; `at` the
  // words its errors start with.
  function sanitizerSequence(value, at, convert) {
    const iterate = isObjectValue(value) ? value[iteratorSymbol] : undefined;
    if (typeof iterate !== "function") {
      throw new $TypeError(at + "The provided value cannot be converted to a sequence.");
    }
    const iterator = ReflectApply(iterate, value, []);
    const next = iterator.next;
    const items = [];
    for (;;) {
      const step = ReflectApply(next, iterator, []);
      if (!isObjectValue(step)) {
        throw new $TypeError(at + "The iterator's result is not an object.");
      }
      if (step.done) {
        return items;
      }
      ArrayPush(items, convert(step.value, at));
    }
  }

  function readMember(at, member, dictionary) {
    return at + "Failed to read the '" + member + "' property from '" + dictionary + "': ";
  }

  // A namespace member: `fallback` when missing, null for null or empty.
  function namespaceMember(value, fallback) {
    if (value === undefined) {
      return fallback;
    }
    if (value === null) {
      return null;
    }
    const ns = str(value);
    return ns === "" ? null : ns;
  }

  function requiredName(value, at, dictionary, member) {
    if (value === undefined) {
      throw new $TypeError(readMember(at, member, dictionary) + "Required member is undefined.");
    }
    return str(value);
  }

  // A SanitizerAttribute, canonical: a string is of no namespace.
  function canonicalAttribute(value, at) {
    if (!isObjectValue(value) && value !== null && value !== undefined) {
      return { name: str(value), namespace: null };
    }
    const dict = value === null || value === undefined ? {} : value;
    const name = requiredName(dict.name, at, "SanitizerAttributeNamespace", "name");
    return { name, namespace: namespaceMember(dict.namespace, null) };
  }

  function attributeList(value, at, member, dictionary) {
    return sanitizerSequence(value, readMember(at, member, dictionary), canonicalAttribute);
  }

  // A SanitizerElement, canonical: a string is HTML's; with
  // `attributes` its own attributes, given or not.
  function canonicalElement(value, at, attributes) {
    if (!isObjectValue(value) && value !== null && value !== undefined) {
      const element = { name: str(value), namespace: HTML_NS };
      if (attributes) {
        element.removeAttributes = [];
      }
      return element;
    }
    const dict = value === null || value === undefined ? {} : value;
    const name = requiredName(dict.name, at, "SanitizerElementNamespace", "name");
    const element = { name, namespace: namespaceMember(dict.namespace, HTML_NS) };
    if (attributes) {
      const where = "SanitizerElementNamespaceWithAttributes";
      if (dict.attributes !== undefined) {
        element.attributes = attributeList(dict.attributes, at, "attributes", where);
      }
      if (dict.removeAttributes !== undefined) {
        element.removeAttributes = attributeList(dict.removeAttributes, at, "removeAttributes",
          where);
      }
      if (element.attributes === undefined && element.removeAttributes === undefined) {
        element.removeAttributes = [];
      }
    }
    return element;
  }

  function canonicalInstruction(value, at) {
    if (!isObjectValue(value) && value !== null && value !== undefined) {
      return { target: str(value) };
    }
    const dict = value === null || value === undefined ? {} : value;
    return { target: requiredName(dict.target, at, "SanitizerProcessingInstruction", "target") };
  }

  // A SanitizerConfig dictionary, read in its members' order and made
  // canonical: the lists not given are empty remove lists, `comments`,
  // and `dataAttributes` with an allow list of attributes, are `allow`
  // when not given (HTML's "set a configuration").
  function configFrom(value, at, allow) {
    const dict = value === null || value === undefined ? {} : value;
    const config = {};
    const lists = [
      ["attributes", (v, w) => attributeList(v, w, "attributes", "SanitizerConfig")],
      ["comments", null],
      ["dataAttributes", null],
      ["elements", (v, w) => sanitizerSequence(v, readMember(w, "elements", "SanitizerConfig"),
        (item, a) => canonicalElement(item, a, true))],
      ["processingInstructions", (v, w) => sanitizerSequence(v,
        readMember(w, "processingInstructions", "SanitizerConfig"), canonicalInstruction)],
      ["removeAttributes", (v, w) => attributeList(v, w, "removeAttributes", "SanitizerConfig")],
      ["removeElements", (v, w) => sanitizerSequence(v,
        readMember(w, "removeElements", "SanitizerConfig"),
        (item, a) => canonicalElement(item, a, false))],
      ["removeProcessingInstructions", (v, w) => sanitizerSequence(v,
        readMember(w, "removeProcessingInstructions", "SanitizerConfig"), canonicalInstruction)],
      ["replaceWithChildrenElements", (v, w) => sanitizerSequence(v,
        readMember(w, "replaceWithChildrenElements", "SanitizerConfig"),
        (item, a) => canonicalElement(item, a, false))],
    ];
    for (let i = 0; i < lists.length; i++) {
      const [member, convert] = lists[i];
      const given = dict[member];
      if (given === undefined) {
        continue;
      }
      config[member] = convert === null ? !!given : convert(given, at);
    }
    if (config.elements === undefined && config.removeElements === undefined) {
      config.removeElements = [];
    }
    if (config.processingInstructions === undefined &&
      config.removeProcessingInstructions === undefined) {
      config.removeProcessingInstructions = [];
    }
    if (config.attributes === undefined && config.removeAttributes === undefined) {
      config.removeAttributes = [];
    }
    if (config.comments === undefined) {
      config.comments = allow;
    }
    if (config.attributes !== undefined && config.dataAttributes === undefined) {
      config.dataAttributes = allow;
    }
    return config;
  }

  // Whether a canonical configuration is valid (HTML's "valid" Sanitizer
  // configuration): one list of each kind, none with a name twice, no
  // element both allowed or removed and replaced by its children, no
  // `html` replaced, and each element's own attributes in keeping with
  // the configuration's.
  function validConfig(config) {
    if ((config.elements !== undefined) === (config.removeElements !== undefined) ||
      (config.attributes !== undefined) === (config.removeAttributes !== undefined) ||
      (config.processingInstructions !== undefined) ===
        (config.removeProcessingInstructions !== undefined)) {
      return false;
    }
    if (hasDuplicates(config.elements) || hasDuplicates(config.removeElements) ||
      hasDuplicates(config.replaceWithChildrenElements) || hasDuplicates(config.attributes) ||
      hasDuplicates(config.removeAttributes) ||
      hasDuplicates(config.processingInstructions, sameTarget) ||
      hasDuplicates(config.removeProcessingInstructions, sameTarget)) {
      return false;
    }
    if (intersects(config.elements !== undefined ? config.elements : config.removeElements,
      config.replaceWithChildrenElements)) {
      return false;
    }
    if (hasItem(config.replaceWithChildrenElements, { name: "html", namespace: HTML_NS })) {
      return false;
    }
    if (config.attributes === undefined && config.dataAttributes !== undefined) {
      return false;
    }
    const elements = config.elements !== undefined ? config.elements : [];
    for (let i = 0; i < elements.length; i++) {
      const element = elements[i];
      if (element.attributes !== undefined && element.removeAttributes !== undefined) {
        return false;
      }
      if (hasDuplicates(element.attributes) || hasDuplicates(element.removeAttributes)) {
        return false;
      }
      if (config.attributes !== undefined) {
        if (intersects(element.attributes, config.attributes)) {
          return false;
        }
        const removed = element.removeAttributes !== undefined ? element.removeAttributes : [];
        for (let j = 0; j < removed.length; j++) {
          if (!hasItem(config.attributes, removed[j])) {
            return false;
          }
        }
        if (config.dataAttributes === true && element.attributes !== undefined) {
          for (let j = 0; j < element.attributes.length; j++) {
            if (isDataAttribute(element.attributes[j])) {
              return false;
            }
          }
        }
      } else if (intersects(element.attributes, config.removeAttributes) ||
        intersects(element.removeAttributes, config.removeAttributes)) {
        return false;
      }
    }
    return true;
  }

  // ---- Changing a configuration (the Sanitizer's methods), each saying
  // whether it changed anything

  function sameElementEntry(a, b) {
    if (!sameName(a, b)) {
      return false;
    }
    const sameList = (x, y) => {
      if (x === undefined || y === undefined) {
        return x === y;
      }
      if (x.length !== y.length) {
        return false;
      }
      for (let i = 0; i < x.length; i++) {
        if (!sameName(x[i], y[i])) {
          return false;
        }
      }
      return true;
    };
    return sameList(a.attributes, b.attributes) &&
      sameList(a.removeAttributes, b.removeAttributes);
  }

  function withoutDuplicates(list) {
    const out = [];
    for (let i = 0; i < list.length; i++) {
      if (!hasItem(out, list[i])) {
        ArrayPush(out, list[i]);
      }
    }
    return out;
  }

  function without(list, removed, keep) {
    const out = [];
    for (let i = 0; i < list.length; i++) {
      if (hasItem(removed, list[i]) === keep) {
        ArrayPush(out, list[i]);
      }
    }
    return out;
  }

  function allowElementIn(config, element) {
    if (element.attributes !== undefined && element.removeAttributes !== undefined) {
      return false;
    }
    if (config.elements !== undefined) {
      if (config.attributes !== undefined) {
        if (element.attributes !== undefined) {
          let own = without(withoutDuplicates(element.attributes), config.attributes, false);
          if (config.dataAttributes === true) {
            const kept = [];
            for (let i = 0; i < own.length; i++) {
              if (!isDataAttribute(own[i])) {
                ArrayPush(kept, own[i]);
              }
            }
            own = kept;
          }
          element.attributes = own;
        }
        if (element.removeAttributes !== undefined) {
          element.removeAttributes = without(withoutDuplicates(element.removeAttributes),
            config.attributes, true);
        }
      } else {
        if (element.attributes !== undefined) {
          element.attributes = without(withoutDuplicates(element.attributes),
            config.removeAttributes, false);
        }
        if (element.removeAttributes !== undefined) {
          element.removeAttributes = without(withoutDuplicates(element.removeAttributes),
            config.removeAttributes, false);
        }
      }
      const modified = removeItem(config.replaceWithChildrenElements, element);
      const at = indexOfItem(config.elements, element, sameName);
      if (at < 0) {
        ArrayPush(config.elements, element);
        return true;
      }
      if (sameElementEntry(config.elements[at], element)) {
        return modified;
      }
      ArraySplice(config.elements, at, 1);
      ArrayPush(config.elements, element);
      return true;
    }
    if (element.attributes !== undefined ||
      (element.removeAttributes !== undefined && element.removeAttributes.length > 0)) {
      return false;
    }
    const modified = removeItem(config.replaceWithChildrenElements, element);
    return removeItem(config.removeElements, element) || modified;
  }

  function removeElementIn(config, element) {
    const modified = removeItem(config.replaceWithChildrenElements, element);
    if (config.elements !== undefined) {
      return removeItem(config.elements, element) || modified;
    }
    if (hasItem(config.removeElements, element)) {
      return modified;
    }
    ArrayPush(config.removeElements, element);
    return true;
  }

  function replaceElementIn(config, element) {
    if (element.name === "html" && element.namespace === HTML_NS) {
      return false;
    }
    if (hasItem(config.replaceWithChildrenElements, element)) {
      return false;
    }
    removeItem(config.removeElements, element);
    removeItem(config.elements, element);
    if (config.replaceWithChildrenElements === undefined) {
      config.replaceWithChildrenElements = [];
    }
    ArrayPush(config.replaceWithChildrenElements, element);
    return true;
  }

  function allowAttributeIn(config, attribute) {
    if (config.attributes !== undefined) {
      if (config.dataAttributes === true && isDataAttribute(attribute)) {
        return false;
      }
      if (hasItem(config.attributes, attribute)) {
        return false;
      }
      const elements = config.elements !== undefined ? config.elements : [];
      for (let i = 0; i < elements.length; i++) {
        removeItem(elements[i].attributes, attribute);
      }
      ArrayPush(config.attributes, attribute);
      return true;
    }
    return removeItem(config.removeAttributes, attribute);
  }

  function removeAttributeIn(config, attribute) {
    const elements = config.elements !== undefined ? config.elements : [];
    if (config.attributes !== undefined) {
      let modified = removeItem(config.attributes, attribute);
      for (let i = 0; i < elements.length; i++) {
        modified = removeItem(elements[i].attributes, attribute) || modified;
        modified = removeItem(elements[i].removeAttributes, attribute) || modified;
      }
      return modified;
    }
    if (hasItem(config.removeAttributes, attribute)) {
      return false;
    }
    for (let i = 0; i < elements.length; i++) {
      removeItem(elements[i].attributes, attribute);
      removeItem(elements[i].removeAttributes, attribute);
    }
    ArrayPush(config.removeAttributes, attribute);
    return true;
  }

  function allowInstructionIn(config, instruction) {
    if (config.processingInstructions !== undefined) {
      if (hasItem(config.processingInstructions, instruction, sameTarget)) {
        return false;
      }
      ArrayPush(config.processingInstructions, instruction);
      return true;
    }
    return removeItem(config.removeProcessingInstructions, instruction, sameTarget);
  }

  function removeInstructionIn(config, instruction) {
    if (config.processingInstructions !== undefined) {
      return removeItem(config.processingInstructions, instruction, sameTarget);
    }
    if (hasItem(config.removeProcessingInstructions, instruction, sameTarget)) {
      return false;
    }
    ArrayPush(config.removeProcessingInstructions, instruction);
    return true;
  }

  // HTML's "remove unsafe": the built-in safe baseline's elements and
  // attributes removed from the configuration.
  function removeUnsafeIn(config) {
    for (let i = 0; i < SAFE_BASELINE_ELEMENTS.length; i++) {
      const [namespace, name] = SAFE_BASELINE_ELEMENTS[i];
      removeElementIn(config, { name, namespace });
    }
    const handlers = words(EVENT_HANDLER_ATTRIBUTES);
    for (let i = 0; i < handlers.length; i++) {
      removeAttributeIn(config, { name: handlers[i], namespace: null });
    }
  }

  // ---- The Sanitizer interface

  const sanitizerConfigs = new $WeakMap();

  function sanitizerOf(value) {
    const config = WeakMapGet(sanitizerConfigs, value);
    if (config === undefined) {
      throw illegalInvocation();
    }
    return config;
  }

  // A list of a configuration's, sorted by namespace, no namespace
  // first, then by name, or by target, as `get()` gives them.
  function sortedList(list) {
    const out = copyList(list);
    const before = (a, b) => {
      if (a.target !== undefined) {
        return a.target < b.target;
      }
      if (a.namespace !== b.namespace) {
        return a.namespace === null || (b.namespace !== null && a.namespace < b.namespace);
      }
      return a.name < b.name;
    };
    for (let i = 1; i < out.length; i++) {
      const item = out[i];
      let j = i - 1;
      while (j >= 0 && before(item, out[j])) {
        out[j + 1] = out[j];
        j--;
      }
      out[j + 1] = item;
    }
    for (let i = 0; i < out.length; i++) {
      if (out[i].attributes !== undefined) {
        out[i].attributes = sortedList(out[i].attributes);
      }
      if (out[i].removeAttributes !== undefined) {
        out[i].removeAttributes = sortedList(out[i].removeAttributes);
      }
    }
    return out;
  }

  class Sanitizer {
    constructor(configuration = "default") {
      const where = "Failed to construct 'Sanitizer': ";
      let config;
      if (isObjectValue(configuration) || configuration === null) {
        config = configFrom(configuration, where, true);
        if (!validConfig(config)) {
          throw new $TypeError(where + "Invalid Sanitizer configuration.");
        }
      } else {
        const preset = str(configuration);
        if (preset !== "default") {
          throw new $TypeError(where + "The provided value '" + preset +
            "' is not a valid enum value of type SanitizerPresets.");
        }
        config = copyConfig(safeDefaultConfig());
      }
      WeakMapSet(sanitizerConfigs, this, config);
    }

    // The configuration, its keys and lists in order.
    get() {
      const config = sanitizerOf(this);
      const out = {};
      if (config.attributes !== undefined) {
        out.attributes = sortedList(config.attributes);
      }
      out.comments = config.comments;
      if (config.dataAttributes !== undefined) {
        out.dataAttributes = config.dataAttributes;
      }
      const rest = [
        "elements", "processingInstructions", "removeAttributes", "removeElements",
        "removeProcessingInstructions", "replaceWithChildrenElements",
      ];
      for (let i = 0; i < rest.length; i++) {
        if (config[rest[i]] !== undefined) {
          out[rest[i]] = sortedList(config[rest[i]]);
        }
      }
      return out;
    }

    allowElement(element) {
      const config = sanitizerOf(this);
      need(arguments.length, 1, "allowElement", "Sanitizer");
      return allowElementIn(config, canonicalElement(element, prefix("allowElement",
        "Sanitizer"), true));
    }

    removeElement(element) {
      const config = sanitizerOf(this);
      need(arguments.length, 1, "removeElement", "Sanitizer");
      return removeElementIn(config, canonicalElement(element, prefix("removeElement",
        "Sanitizer"), false));
    }

    replaceElementWithChildren(element) {
      const config = sanitizerOf(this);
      need(arguments.length, 1, "replaceElementWithChildren", "Sanitizer");
      return replaceElementIn(config, canonicalElement(element,
        prefix("replaceElementWithChildren", "Sanitizer"), false));
    }

    allowAttribute(attribute) {
      const config = sanitizerOf(this);
      need(arguments.length, 1, "allowAttribute", "Sanitizer");
      return allowAttributeIn(config, canonicalAttribute(attribute,
        prefix("allowAttribute", "Sanitizer")));
    }

    removeAttribute(attribute) {
      const config = sanitizerOf(this);
      need(arguments.length, 1, "removeAttribute", "Sanitizer");
      return removeAttributeIn(config, canonicalAttribute(attribute,
        prefix("removeAttribute", "Sanitizer")));
    }

    setComments(allow) {
      const config = sanitizerOf(this);
      need(arguments.length, 1, "setComments", "Sanitizer");
      config.comments = !!allow;
    }

    setDataAttributes(allow) {
      const config = sanitizerOf(this);
      need(arguments.length, 1, "setDataAttributes", "Sanitizer");
      config.dataAttributes = !!allow;
    }

    removeUnsafe() {
      removeUnsafeIn(sanitizerOf(this));
    }

    allowProcessingInstruction(pi) {
      const config = sanitizerOf(this);
      need(arguments.length, 1, "allowProcessingInstruction", "Sanitizer");
      return allowInstructionIn(config, canonicalInstruction(pi,
        prefix("allowProcessingInstruction", "Sanitizer")));
    }

    removeProcessingInstruction(pi) {
      const config = sanitizerOf(this);
      need(arguments.length, 1, "removeProcessingInstruction", "Sanitizer");
      return removeInstructionIn(config, canonicalInstruction(pi,
        prefix("removeProcessingInstruction", "Sanitizer")));
    }
  }
  expose(Sanitizer, "Sanitizer");

  // ---- Sanitizing (HTML's "sanitize" and "set and filter HTML")

  // The configuration `options`' `sanitizer` member gives (HTML's "get a
  // sanitizer instance from options"): a Sanitizer's, a dictionary's, in
  // which comments and data attributes are allowed by an unsafe method
  // alone, or the built-in safe default's; for a safe method, with the
  // safe baseline removed from a copy. Null for an unsafe method given
  // none: nothing is removed.
  function sanitizerConfigFrom(options, safe, method, iface) {
    const where = prefix(method, iface);
    const dictionary = safe ? "SetHTMLOptions" : "SetHTMLUnsafeOptions";
    if (options !== undefined && options !== null && !isObjectValue(options)) {
      throw new $TypeError(where + "The provided value is not of type '" + dictionary + "'.");
    }
    const spec = options === undefined || options === null ? undefined : options.sanitizer;
    let config;
    if (spec === undefined) {
      if (!safe) {
        return null;
      }
      return safeDefaultConfig();
    }
    const own = isObjectValue(spec) ? WeakMapGet(sanitizerConfigs, spec) : undefined;
    if (own !== undefined) {
      config = own;
    } else if (isObjectValue(spec) || spec === null) {
      config = configFrom(spec, readMember(where, "sanitizer", dictionary), !safe);
      if (!validConfig(config)) {
        throw new $TypeError(where + "Invalid Sanitizer configuration.");
      }
    } else {
      const preset = str(spec);
      if (preset !== "default") {
        throw new $TypeError(readMember(where, "sanitizer", dictionary) + "The provided value '" +
          preset + "' is not a valid enum value of type SanitizerPresets.");
      }
      config = safeDefaultConfig();
    }
    // The built-in default holds nothing unsafe.
    if (safe && config !== safeDefault) {
      config = copyConfig(config);
      removeUnsafeIn(config);
    }
    return config;
  }

  // A `javascript:` URL, as the URL parser would find its scheme: the
  // value with its leading and trailing C0 controls and spaces, and its
  // tabs and newlines, gone.
  function isJavascriptUrl(value) {
    let start = 0;
    let end = value.length;
    while (start < end && CharCodeAt(value, start) <= 32) {
      start++;
    }
    while (end > start && CharCodeAt(value, end - 1) <= 32) {
      end--;
    }
    let scheme = "";
    for (let i = start; i < end && scheme.length < 11; i++) {
      const c = CharCodeAt(value, i);
      if (c !== 9 && c !== 10 && c !== 13) {
        scheme += value[i];
      }
    }
    return ToLowerCase(scheme) === "javascript:";
  }

  // The built-in navigating URL attributes list: an element's attribute
  // whose `javascript:` URL a safe method removes.
  const NAVIGATING_URL_ATTRIBUTES = [
    [HTML_NS, "a", null, "href"], [HTML_NS, "area", null, "href"],
    [HTML_NS, "base", null, "href"], [HTML_NS, "button", null, "formaction"],
    [HTML_NS, "form", null, "action"], [HTML_NS, "input", null, "formaction"],
    [SVG_NS, "a", null, "href"], [SVG_NS, "a", XLINK_NS, "href"],
  ];

  // The built-in animating URL attributes list: SVG's animations' target
  // attributes, which may not name `href`.
  const ANIMATING_ELEMENTS = ["animate", "animateMotion", "animateTransform", "set"];

  // Whether a safe method removes the attribute of namespace `ns` and
  // local name `local` of value `value` from the element of namespace
  // `elementNs` and local name `name` (HTML's "sanitize core", step
  // 4.8.5): a `javascript:` URL that navigates, or an animation of an
  // `href`.
  function unsafeUrl(elementNs, name, ns, local, value) {
    for (let i = 0; i < NAVIGATING_URL_ATTRIBUTES.length; i++) {
      const [e, n, a, l] = NAVIGATING_URL_ATTRIBUTES[i];
      if (e === elementNs && n === name && a === ns && l === local) {
        return isJavascriptUrl(value);
      }
    }
    if (elementNs === MATHML_NS && local === "href" && (ns === null || ns === XLINK_NS)) {
      return isJavascriptUrl(value);
    }
    return elementNs === SVG_NS && ns === null && local === "attributeName" &&
      includesString(ANIMATING_ELEMENTS, name) && (value === "href" || value === "xlink:href");
  }

  function configEntry(list, name, namespace) {
    if (list === undefined) {
      return undefined;
    }
    for (let i = 0; i < list.length; i++) {
      if (list[i].name === name && list[i].namespace === namespace) {
        return list[i];
      }
    }
    return undefined;
  }

  // HTML's "sanitize core": `handle`'s children kept, removed or replaced
  // by their own children as `config` says, their attributes likewise,
  // template contents and what is under each element sanitized too; the
  // `javascript:` URLs that navigate removed when `safe`. Of a
  // document's own children, only its element is sanitized.
  function sanitizeTree(handle, config, safe) {
    const top = native.kind(handle) === 9;
    const children = native.children(handle);
    for (let i = 0; i < children.length; i++) {
      const child = children[i];
      const kind = native.kind(child);
      if (top && kind !== 1) {
        continue;
      }
      if (kind === 8) {
        if (config.comments !== true) {
          native.remove(handle, child);
        }
        continue;
      }
      if (kind === 7) {
        const pi = { target: wrap(child).target };
        if ((config.processingInstructions !== undefined &&
          !hasItem(config.processingInstructions, pi, sameTarget)) ||
          hasItem(config.removeProcessingInstructions, pi, sameTarget)) {
          native.remove(handle, child);
        }
        continue;
      }
      if (kind !== 1) {
        continue;
      }
      const element = wrap(child);
      const name = element.localName;
      const namespace = nsOf(element);
      const named = { name, namespace };
      if (hasItem(config.replaceWithChildrenElements, named)) {
        sanitizeTree(child, config, safe);
        for (let grand = native.firstChild(child); grand !== null;
          grand = native.firstChild(child)) {
          native.insert(handle, grand, child);
        }
        native.remove(handle, child);
        continue;
      }
      if (hasItem(config.removeElements, named) ||
        (config.elements !== undefined && !hasItem(config.elements, named))) {
        native.remove(handle, child);
        continue;
      }
      if (name === "template" && namespace === HTML_NS) {
        sanitizeTree(native.templateContent(child), config, safe);
      }
      sanitizeAttributes(child, name, namespace, config, safe);
      sanitizeTree(child, config, safe);
    }
  }

  // The attributes of element `child` the configuration does not keep,
  // removed (HTML's "sanitize core", step 4.8).
  function sanitizeAttributes(child, name, namespace, config, safe) {
    const own = configEntry(config.elements, name, namespace);
    const names = native.attributeNames(child);
    const namespaces = native.attributeNamespaces(child);
    const gone = [];
    for (let i = 0; i < names.length; i++) {
      const ns = namespaces[i] === "" ? null : namespaces[i];
      const local = ns === null ? names[i] : localPart(names[i]);
      const attribute = { name: local, namespace: ns };
      let remove;
      if (own !== undefined && hasItem(own.removeAttributes, attribute)) {
        remove = true;
      } else if (config.attributes !== undefined) {
        remove = !hasItem(config.attributes, attribute) &&
          !(own !== undefined && hasItem(own.attributes, attribute)) &&
          !(config.dataAttributes === true && isDataAttribute(attribute));
      } else if (own !== undefined && own.attributes !== undefined) {
        remove = !hasItem(own.attributes, attribute);
      } else {
        remove = hasItem(config.removeAttributes, attribute);
      }
      if (!remove && safe) {
        const value = native.attributeNS(child, ns, local);
        remove = value !== null && unsafeUrl(namespace, name, ns, local, value);
      }
      if (remove) {
        ArrayPush(gone, [ns, local]);
      }
    }
    for (let i = 0; i < gone.length; i++) {
      native.removeAttributeNS(child, gone[i][0], gone[i][1]);
    }
  }

  // HTML's "set and filter HTML": `html` parsed as a fragment in the
  // context of the element `context`, sanitized as `options` say, safe
  // or not, and put in place of `target`'s children in one operation.
  // A safe method leaves a script element alone. The core of `setHTML`
  // and `setHTMLUnsafe` whatever `target` is: an element, a template's
  // contents, or a shadow root, its host the context.
  function setAndFilterHTML(target, context, html, options, safe, method, iface) {
    const contextHandle = handleOf(context);
    if (safe && context.localName === "script" &&
      (nsOf(context) === HTML_NS || nsOf(context) === SVG_NS)) {
      return;
    }
    const config = sanitizerConfigFrom(options, safe, method, iface);
    let fragment;
    try {
      fragment = native.parseHtmlFragment(contextHandle, html);
    } catch (error) {
      throw failed(error, method, iface);
    }
    if (config !== null) {
      sanitizeTree(fragment, config, safe);
    }
    try {
      native.replaceAll(target, fragment);
    } catch (error) {
      throw failed(error, method, iface);
    }
  }

  // The node an element's markup goes into: a template's contents.
  function markupTarget(handle) {
    const contents = native.templateContent(handle);
    return contents === null ? handle : contents;
  }

  include(
    {
      setHTML(html, options = undefined) {
        const handle = self(this);
        need(arguments.length, 1, "setHTML", "Element");
        setAndFilterHTML(markupTarget(handle), this, str(html), options, true, "setHTML",
          "Element");
      },

      setHTMLUnsafe(html, options = undefined) {
        const handle = self(this);
        need(arguments.length, 1, "setHTMLUnsafe", "Element");
        setAndFilterHTML(markupTarget(handle), this, str(html), options, false,
          "setHTMLUnsafe", "Element");
      },
    },
    [Element.prototype],
  );

  // `Document.parseHTML` and `parseHTMLUnsafe` (HTML section 8.5.1): `html`
  // parsed as a document of its own, scripting off, and sanitized as
  // `options` say, safe or not.
  function parseHTMLDocument(html, options, safe, method) {
    const config = sanitizerConfigFrom(options, safe, method, "Document");
    let document;
    try {
      document = native.parseDocument(html, "text/html", true);
    } catch (error) {
      throw failed(error, method, "Document");
    }
    if (config !== null) {
      sanitizeTree(document, config, safe);
    }
    return wrap(document);
  }

  defineProperty(Document, "parseHTML", {
      __proto__: null,
    value: {
      parseHTML(html, options = undefined) {
        need(arguments.length, 1, "parseHTML", "Document");
        return parseHTMLDocument(str(html), options, true, "parseHTML");
      },
    }.parseHTML,
    writable: true,
    enumerable: true,
    configurable: true,
  });
  defineProperty(Document, "parseHTMLUnsafe", {
      __proto__: null,
    value: {
      parseHTMLUnsafe(html, options = undefined) {
        need(arguments.length, 1, "parseHTMLUnsafe", "Document");
        return parseHTMLDocument(str(html), options, false, "parseHTMLUnsafe");
      },
    }.parseHTMLUnsafe,
    writable: true,
    enumerable: true,
    configurable: true,
  });

  // `<style>.sheet` and `<link>.sheet` (D66).
  include(elementSheet, [
    MapGet(interfaces, "style").prototype,
    MapGet(interfaces, "link").prototype,
  ]);
  // `HTMLScriptElement.supports()` (HTML section 4.12.1): the script
  // types the engine runs, speculation rules not among them.
  defineProperty(MapGet(interfaces, "script"), "supports", {
    value: {
      supports(type) {
        need(arguments.length, 1, "supports", "HTMLScriptElement");
        const kind = str(type);
        return kind === "classic" || kind === "module" || kind === "importmap";
      },
    }.supports,
    writable: true,
    enumerable: true,
    configurable: true,
  });

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

  // `value` parsed against the base URL of the document of the node
  // `handle`: the page's, or a document's beside it its own (D67), so
  // that a relative URL in one at `about:blank` does not parse; null
  // when it does not.
  function resolveFor(handle, value) {
    if (otherDocuments) {
      const owner = native.ownerDocument(handle);
      if (owner !== native.document()) {
        const parts = native.urlParse(value, native.documentInfo(owner)[5]);
        return parts === null ? null : parts[0];
      }
    }
    return native.resolveUrl(value);
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
              : resolveFor(self(this), value);
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
          // A referrer policy attribute: a policy's name, else empty.
          case "referrer": {
            const lower = value === null ? "" : ToLowerCase(value);
            return ArrayIncludes(REFERRER_POLICIES, lower) ? lower : "";
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
      ["name", "name", "string"],
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
      // M38: what a preload is fetched as, and how.
      ["as", "as", "string"],
      ["crossOrigin", "crossorigin", "cors"],
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
      ["crossOrigin", "crossorigin", "cors"],
      ["referrerPolicy", "referrerpolicy", "referrer"],
      ["integrity", "integrity", "string"],
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
      ["crossOrigin", "crossorigin", "cors"],
      ["defaultMuted", "muted", "boolean"],
    ]],
    ["HTMLSourceElement", [
      ["src", "src", "url"],
      ["type", "type", "string"],
      ["media", "media", "string"],
    ]],
    ["HTMLVideoElement", [
      ["playsInline", "playsinline", "boolean"],
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
    ["HTMLSlotElement", [
      ["name", "name", "string"],
    ]],
    ["HTMLTemplateElement", [
      ["shadowRootDelegatesFocus", "shadowrootdelegatesfocus", "boolean"],
      ["shadowRootClonable", "shadowrootclonable", "boolean"],
      ["shadowRootSerializable", "shadowrootserializable", "boolean"],
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

  // HTML's HTMLHyperlinkElementUtils (section 4.6.6, gap batch A): an
  // `a`'s and an `area`'s URL by its parts, parsed from its `href`
  // against the base URL, a part set written back to the attribute.
  const LINK_PARTS = [
    "origin", "protocol", "username", "password", "host", "hostname", "port", "pathname",
    "search", "hash",
  ];
  function linkUrl(handle) {
    const value = native.attribute(handle, "href");
    if (value === null) {
      return null;
    }
    const resolved = resolveFor(handle, value);
    return resolved === null ? null : native.urlParse(resolved, null);
  }
  function linkPart(prototype, part, at) {
    const descriptor = {
      get() {
        const parts = linkUrl(self(this));
        if (parts === null) {
          return part === "protocol" ? ":" : "";
        }
        return parts[at];
      },
      enumerable: true,
      configurable: true,
    };
    if (part !== "origin") {
      descriptor.set = function (value) {
        const handle = self(this);
        const parts = linkUrl(handle);
        if (parts === null) {
          return;
        }
        const changed = native.urlSet(parts[0], part, str(value));
        if (changed !== null) {
          setAttribute(handle, "href", changed[0], part);
        }
      };
    }
    defineProperty(prototype, part, descriptor);
  }
  for (const name of ["HTMLAnchorElement", "HTMLAreaElement"]) {
    for (let i = 0; i < LINK_PARTS.length; i++) {
      linkPart(madeInterfaces[name].prototype, LINK_PARTS[i], i + 1);
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
  // An SVG script's `type` and `crossOrigin` (SVG 2's SVGScriptElement).
  reflect(MapGet(svgInterfaces, "script").prototype, "type", "type", "string");
  reflect(MapGet(svgInterfaces, "script").prototype, "crossOrigin", "crossorigin", "cors");

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
  // A MathML element is an HTMLOrSVGElement too (MathML Core section
  // 2.1.2): its `nonce` is HTML's and SVG's.
  defineProperty(
    MathMLElement.prototype,
    "nonce",
    getOwnPropertyDescriptor(HTMLElement.prototype, "nonce"),
  );

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
    let fragment;
    try {
      fragment = native.navigate(url, replace);
    } catch (error) {
      throw setter ? failedSet(error, member, "Location") : failed(error, member, "Location");
    }
    // To a fragment of the document's URL, the navigation is done when
    // the call returns (HTML's "navigate to a fragment"): `popstate` is
    // fired with the new entry's state, null.
    if (fragment === true) {
      historyState = null;
      historyStateKnown = true;
      const event = new PopStateEvent("popstate", made(POP_STATE_INIT, ["state", null]));
      eventState(event).trusted = true;
      drain(dispatching(global, event));
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
      // The fragment the URL has already: nothing to navigate to.
      if (at >= 0 && StringSlice(href, at + 1) === fragment) {
        return;
      }
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

  // ---- History (HTML section 7.2.2) --------------------------------------

  // The document's history object state, what `history.state` gives: read
  // from the entry the document was shown for at its first ask, then set
  // by `pushState`, `replaceState` and the traversals between its
  // entries, each as its serialized state made again.
  let historyStateKnown = false;
  let historyState = null;

  function setHistoryState(text) {
    try {
      historyState = deserialize(unwire(text));
    } catch {
      historyState = null;
    }
    historyStateKnown = true;
  }

  // HTML's "shared history push/replace state steps": the state
  // serialized for storage, the URL checked and set by the host, which
  // puts a new entry after the current one or the current one's place.
  function pushHistory(data, unused, url, replace, method) {
    str(unused);
    const given = url === null ? null : str(url);
    const text = wire(serialize(data, [], prefix(method, "History"), null));
    try {
      native.historyPush(text, given, replace);
    } catch (error) {
      throw failed(error, method, "History");
    }
    setHistoryState(text);
  }

  class History {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("History");
      }
    }

    get length() {
      historyOf(this);
      return native.historyLength();
    }

    get scrollRestoration() {
      historyOf(this);
      return native.historyScroll() ? "manual" : "auto";
    }

    // An enumeration's attribute: a value not of it is ignored.
    set scrollRestoration(value) {
      historyOf(this);
      const mode = str(value);
      if (mode === "auto" || mode === "manual") {
        native.historyScroll(mode === "manual");
      }
    }

    get state() {
      historyOf(this);
      if (!historyStateKnown) {
        const text = native.historyState();
        if (text === null) {
          historyStateKnown = true;
        } else {
          setHistoryState(text);
        }
      }
      return historyState;
    }

    go(delta = 0) {
      historyOf(this);
      native.historyGo($Number(delta) | 0);
    }

    back() {
      historyOf(this);
      native.historyGo(-1);
    }

    forward() {
      historyOf(this);
      native.historyGo(1);
    }

    pushState(data, unused, url = null) {
      historyOf(this);
      need(arguments.length, 2, "pushState", "History");
      pushHistory(data, unused, url === undefined ? null : url, false, "pushState");
    }

    replaceState(data, unused, url = null) {
      historyOf(this);
      need(arguments.length, 2, "replaceState", "History");
      pushHistory(data, unused, url === undefined ? null : url, true, "replaceState");
    }
  }
  expose(History, "History");
  const history = new History(KEY);

  function historyOf(value) {
    if (value !== history) {
      throw illegalInvocation();
    }
  }

  // A traversal to another entry of the document's (the engine's
  // `popstate` task): its state is the document's history object state,
  // and `popstate` is fired at the window with it.
  function* popStateTask(text) {
    if (text === null) {
      historyState = null;
      historyStateKnown = true;
    } else {
      setHistoryState(text);
    }
    const event = new PopStateEvent("popstate", made(POP_STATE_INIT, ["state", historyState]));
    eventState(event).trusted = true;
    return yield* dispatching(global, event);
  }

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
  // The user's languages (M37): what the host says after the user
  // agent's three, the default two when it says none.
  let languages = null;
  function languageList() {
    if (languages === null) {
      const said = ArraySlice(agentInfo(), 3);
      languages = Object.freeze(said.length ? said : ["en-US", "en"]);
    }
    return languages;
  }

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
      return languageList()[0];
    }

    get languages() {
      return languageList();
    }

    get onLine() {
      return true;
    }

    // The Storage Standard's StorageManager (M36.8), of a secure
    // context's window.
    get storage() {
      return chunkInstall("idb").storageManager;
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
    // What does not start with a colon names no pseudo-element: the
    // element's own style (CSSOM section 9).
    if (name === null) {
      return "";
    }
    // One colon is CSS 2's four pseudo-elements' alone (Selectors 4
    // section 3.6.1); `::part()` and `::slotted()` name elements of
    // other trees, not a pseudo-element of this one: an empty
    // declaration, as the reference browser gives (D69).
    const legacy = name === "before" || name === "after" || name === "first-letter" ||
      name === "first-line";
    if (
      (StringIndexOf(text, "::") !== 0 && !legacy) ||
      StringIndexOf(name, "part(") === 0 ||
      StringIndexOf(name, "slotted(") === 0
    ) {
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

    get history() {
      return history;
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

    // A new tab, or the navigable `target` names (M37): opened as with
    // `noopener`, so no window is given back.
    open(url = "", target = "_blank", features = "") {
      void features;
      native.openWindow(str(url), str(target) || "_blank");
      return null;
    }

    // The tab closed, when a script opened it (M37).
    close() {
      native.closeWindow();
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

    // HTML's simple dialogs (section 8.8.1): the host asks the user, the
    // script waiting (gap batch A, D43); null is Cancel, or no dialog.
    alert() {
      native.dialog("alert", arguments.length === 0 ? "" : str(arguments[0]), "");
    }

    confirm() {
      const message = arguments[0] === undefined ? "" : str(arguments[0]);
      return native.dialog("confirm", message, "") !== null;
    }

    prompt() {
      const message = arguments[0] === undefined ? "" : str(arguments[0]);
      const given = arguments[1] === undefined ? "" : str(arguments[1]);
      return native.dialog("prompt", message, given);
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
  // With them (gap batch A, D44) the document's named elements: an
  // element of that id, or of that name among `embed`, `form`, `img` and
  // `object`, or a collection of several, an accessor of each name the
  // engine says the document has (`namedSync`), a frame's name first.
  // Real properties, not a proxy: QuickJS would take a proxy's answer of
  // nothing for a global that is there, and throw no ReferenceError.
  const ArrayIncludesOf = uncurry(Array.prototype.includes);
  const windowProperties = ObjectCreate(EventTarget.prototype);
  let frameKeys = [];
  if (!WORKER) {
    setPrototypeOf(Window.prototype, windowProperties);
  }

  // The names the document has, and the last answers, while the document
  // is as it was.
  const namedWanted = new $Map();
  const namedFound = new $Map();
  let namedVersion = -1;
  function namedElement(name) {
    const version = native.version();
    if (version !== namedVersion) {
      MapClear(namedFound);
      namedVersion = version;
    }
    let found = MapGet(namedFound, name);
    if (found === undefined) {
      found = native.namedItems(name);
      MapSet(namedFound, name, found);
    }
    if (found.length === 0) {
      return undefined;
    }
    return found.length === 1 ? wrap(found[0]) : snapshot(HTMLCollection, found);
  }

  // The accessor of named property `name`, unless a frame has the name
  // or what the window inherits (Web IDL's named property visibility:
  // `addEventListener` is not hidden by an element of that id).
  function defineNamed(name) {
    if (ArrayIncludesOf(frameKeys, name) || ReflectHas(EventTarget.prototype, name)) {
      return;
    }
    defineProperty(windowProperties, name, {
      get: function () {
        return namedElement(name);
      },
      // A script's own value of the name, on the window.
      set: function (value) {
        defineProperty(this, name, { value, writable: true, enumerable: true, configurable: true });
      },
      enumerable: false,
      configurable: true,
    });
  }

  // The engine's word of the names that came (true) or went, in pairs;
  // all of them first dropped when `reset`.
  function namedSync(reset, changes) {
    if (WORKER) {
      return;
    }
    if (reset) {
      MapForEach(namedWanted, (_, name) => {
        if (!ArrayIncludesOf(frameKeys, name)) {
          ReflectDeleteProperty(windowProperties, name);
        }
      });
      MapClear(namedWanted);
    }
    for (let i = 0; i + 1 < changes.length; i += 2) {
      const name = changes[i];
      if (changes[i + 1]) {
        MapSet(namedWanted, name, true);
        defineNamed(name);
      } else {
        MapDelete(namedWanted, name);
        if (!ArrayIncludesOf(frameKeys, name)) {
          ReflectDeleteProperty(windowProperties, name);
        }
      }
    }
  }

  // The window's child frames are now as many as `names`, those names
  // their elements' (M33.2).
  function frameList(names) {
    for (let i = 0; i < frameKeys.length; i++) {
      ReflectDeleteProperty(windowProperties, frameKeys[i]);
    }
    frameKeys = [];
    const property = (key, ask) => {
      if (ReflectGetOwnPropertyDescriptor(windowProperties, key) !== undefined &&
        !MapHas(namedWanted, key)) {
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
    // The named elements a frame no longer hides.
    MapForEach(namedWanted, (_, name) => {
      if (ReflectGetOwnPropertyDescriptor(windowProperties, name) === undefined) {
        defineNamed(name);
      }
    });
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

    // The worker's own face set (CSS Font Loading 3, D54), which its
    // canvases' text is drawn in.
    get fonts() {
      return fontSetOf(global);
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
        // Another origin's script, not by CORS, throws a NetworkError for
        // whatever it threw (HTML's "run a classic script" with muted
        // errors), which tells nothing of it.
        if (script[2] !== "muted") {
          native.evaluate(script[0], script[1]);
          continue;
        }
        // Its `import()`s resolve against about:blank (HTML's "create a
        // classic script" with muted errors).
        try {
          native.evaluate(script[0], script[1], "about:blank");
        } catch (_) {
          throw new $DOMException(where + "The script at '" + urls[i] + "' failed to load.",
            "NetworkError");
        }
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

  // ---- Service workers (M39, Service Workers section 3) ---------------

  // The realm's objects of the registrations and versions it has seen,
  // one each (the standard's "get the service worker registration
  // object"), by the browser's numbers; and the promises of the jobs it
  // scheduled.
  const swRegistrations = new $Map();
  const swWorkers = new $Map();
  // The ServiceWorker and Client objects, which a message's source may
  // be.
  const swSources = new $WeakSet();
  // The document's controller, once it asked (M39 step 4).
  let swController = null;
  let swContainer = null;
  const swPromises = new $Map();
  let swNextPromise = 1;
  let swSetState;
  let swSetSlot;

  // A request of the service workers' host: its result, or its error
  // thrown, in the host's words.
  function swCall(request) {
    const answer = JSONParse(native.serviceWorker(JSONStringify(request)));
    if (answer.error !== undefined) {
      throw answer.error.name === "TypeError"
        ? new $TypeError(answer.error.message)
        : new $DOMException(answer.error.message, answer.error.name);
    }
    return answer.result;
  }

  // The object of version `shown` ({id, scriptURL, state}), made the
  // first time; null for none.
  function swWorker(shown) {
    if (shown === null || shown === undefined) return null;
    let worker = MapGet(swWorkers, shown.id);
    if (worker === undefined) {
      worker = new ServiceWorker(KEY, shown);
      MapSet(swWorkers, shown.id, worker);
    }
    return worker;
  }

  // The object of the registration `view` shows, made the first time
  // with its slots as the view has them.
  function swRegistration(view) {
    let reg = MapGet(swRegistrations, view.id);
    if (reg === undefined) {
      reg = new ServiceWorkerRegistration(KEY, view);
      MapSet(swRegistrations, view.id, reg);
    }
    return reg;
  }

  // A job scheduled: its promise, settled by the host's task.
  function swJob(request) {
    const number = swNextPromise++;
    request.promise = number;
    let settle;
    const promise = new $Promise((resolve, reject) => {
      settle = { resolve, reject };
    });
    swCall(request);
    MapSet(swPromises, number, settle);
    return promise;
  }

  class ServiceWorker extends EventTarget {
    #id;
    #scriptURL;
    #state;

    constructor(key, shown) {
      if (key !== KEY) throw illegalConstructor("ServiceWorker");
      super();
      this.#id = shown.id;
      this.#scriptURL = shown.scriptURL;
      this.#state = shown.state;
      WeakSetAdd(swSources, this);
    }

    // A message for the version (M39 step 4): a structured clone, which
    // the version hears as an ExtendableMessageEvent. Ports do not cross
    // to a service worker here.
    postMessage(message, transfer) {
      const iface = "ServiceWorker";
      need(arguments.length, 1, "postMessage", iface);
      const ports = postTransfer(transfer, "postMessage", iface);
      if (ports.length > 0) {
        throw new $DOMException(prefix("postMessage", iface) +
          "A MessagePort could not be transferred to a service worker.", "DataCloneError");
      }
      const record = serialize(message, ports, prefix("postMessage", iface), null);
      swCall({ op: "postMessage", version: this.#id, data: wire(record) });
    }

    static {
      swSetState = (worker, state) => {
        const changed = worker.#state !== state;
        worker.#state = state;
        return changed;
      };
    }

    get scriptURL() {
      return this.#scriptURL;
    }

    get state() {
      return this.#state;
    }
  }
  defineHandlers(ServiceWorker.prototype, ["statechange", "error"], false);
  expose(ServiceWorker, "ServiceWorker");

  class ServiceWorkerRegistration extends EventTarget {
    #id;
    #scope;
    #updateViaCache;
    #slots;

    constructor(key, view) {
      if (key !== KEY) throw illegalConstructor("ServiceWorkerRegistration");
      super();
      this.#id = view.id;
      this.#scope = view.scope;
      this.#updateViaCache = view.updateViaCache;
      this.#slots = {
        __proto__: null,
        installing: swWorker(view.installing),
        waiting: swWorker(view.waiting),
        active: swWorker(view.active),
      };
    }

    static {
      swSetSlot = (reg, slot, worker, updateViaCache) => {
        reg.#slots[slot] = worker;
        reg.#updateViaCache = updateViaCache;
      };
    }

    get installing() {
      return this.#slots.installing;
    }

    get waiting() {
      return this.#slots.waiting;
    }

    get active() {
      return this.#slots.active;
    }

    get scope() {
      return this.#scope;
    }

    get updateViaCache() {
      return this.#updateViaCache;
    }

    update() {
      try {
        return swJob({ op: "update", registration: this.#id, scope: this.#scope });
      } catch (e) {
        return rejected(e);
      }
    }

    unregister() {
      try {
        return swJob({ op: "unregister", registration: this.#id, scope: this.#scope });
      } catch (e) {
        return rejected(e);
      }
    }
  }
  defineHandlers(ServiceWorkerRegistration.prototype, ["updatefound"], false);
  expose(ServiceWorkerRegistration, "ServiceWorkerRegistration");

  // A RegistrationOptions member that is one of an enumeration's values.
  function swEnum(value, fallback, values, member, type) {
    if (value === undefined) return fallback;
    const text = str(value);
    if (!ArrayIncludes(values, text)) {
      throw new $TypeError(
        prefix("register", "ServiceWorkerContainer") + "Failed to read the '" + member +
          "' property from 'RegistrationOptions': The provided value '" + text +
          "' is not a valid enum value of type " + type + ".",
      );
    }
    return text;
  }

  // `ready`'s promise, settled once a registration of the document's
  // URL has an active version.
  let swReady = null;

  // The registry may be ahead of the notices the realm has run: the
  // registration's object, as they made it, decides.
  function swCheckReady() {
    if (swReady === null || swReady.done) return;
    let view;
    try {
      view = swCall({ op: "getRegistration", url: null });
    } catch {
      return;
    }
    if (view === null) return;
    const reg = swRegistration(view);
    if (reg.active !== null) {
      swReady.done = true;
      swReady.resolve(reg);
    }
  }

  class ServiceWorkerContainer extends EventTarget {
    constructor(key) {
      if (key !== KEY) throw illegalConstructor("ServiceWorkerContainer");
      super();
    }

    get controller() {
      return swController;
    }

    get ready() {
      if (swReady === null) {
        let resolve;
        const promise = new $Promise((r) => {
          resolve = r;
        });
        swReady = { promise, resolve, done: false };
        swCheckReady();
      }
      return swReady.promise;
    }

    register(scriptURL, options) {
      try {
        need(arguments.length, 1, "register", "ServiceWorkerContainer");
        const given = options === undefined || options === null ? {} : options;
        const scope = given.scope === undefined ? null : str(given.scope);
        const type = swEnum(given.type, "classic", ["classic", "module"], "type", "WorkerType");
        const updateViaCache = swEnum(given.updateViaCache, "imports", ["imports", "all", "none"],
          "updateViaCache", "ServiceWorkerUpdateViaCache");
        return swJob({ op: "register", script: str(scriptURL), scope, type, updateViaCache });
      } catch (e) {
        return rejected(e);
      }
    }

    getRegistration(clientURL) {
      try {
        const url = clientURL === undefined ? null : str(clientURL);
        const view = swCall({ op: "getRegistration", url });
        return resolved(view === null ? undefined : swRegistration(view));
      } catch (e) {
        return rejected(e);
      }
    }

    getRegistrations() {
      try {
        const views = swCall({ op: "getRegistrations" });
        const out = [];
        for (let i = 0; i < views.length; i++) ArrayPush(out, swRegistration(views[i]));
        return resolved(out);
      } catch (e) {
        return rejected(e);
      }
    }

    startMessages() {}
  }
  defineHandlers(ServiceWorkerContainer.prototype, ["controllerchange", "message", "messageerror"], false);
  expose(ServiceWorkerContainer, "ServiceWorkerContainer");

  // `navigator.serviceWorker`: the document's one container, a secure
  // context's alone; reading it where storage is refused (a sandbox
  // without `allow-same-origin`) throws.
  if (!WORKER && SECURE) {
    let container = null;
    defineProperty(Navigator.prototype, "serviceWorker", {
      get() {
        let asked;
        try {
          asked = swCall({ op: "check" });
        } catch (e) {
          throw new $DOMException(
            "Failed to read the 'serviceWorker' property from 'Navigator': " + e.message, e.name);
        }
        if (container === null) {
          container = new ServiceWorkerContainer(KEY);
          swContainer = container;
          swController = asked === null ? null : swWorker(asked.controller);
        }
        return container;
      },
      enumerable: true,
      configurable: true,
    });
  }

  // ---- Extendable events (Service Workers 4.4) -----------------------

  // An extendable event's state: the promises its waitUntil holds,
  // whether one rejected, its dispatch done, and the host's number for
  // it (0 for one a script made).
  const extendStates = new $WeakMap();

  function extendState(event) {
    let state = WeakMapGet(extendStates, event);
    if (state === undefined) {
      state = { pending: 0, rejected: false, dispatched: false, id: 0, ended: false };
      WeakMapSet(extendStates, event, state);
    }
    return state;
  }

  // The host's event is done once dispatched with nothing pending: the
  // host hears whether every promise fulfilled.
  function extendEnd(state) {
    if (state.ended || !state.dispatched || state.pending > 0 || state.id === 0) return;
    state.ended = true;
    swCall({ op: "eventDone", id: state.id, ok: !state.rejected });
  }

  class ExtendableEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, EVENT_INIT, "ExtendableEvent");
      super(args[0], args[1]);
    }

    waitUntil(promise) {
      const where = prefix("waitUntil", "ExtendableEvent");
      need(arguments.length, 1, "waitUntil", "ExtendableEvent");
      if (!eventState(this).trusted) {
        throw new $DOMException(where + "Can not call waitUntil on a script constructed ExtendableEvent.",
          "InvalidStateError");
      }
      const state = extendState(this);
      if (state.dispatched && state.pending === 0) {
        throw new $DOMException(
          where + "The event handler is already finished and no extend lifetime promises are outstanding.",
          "InvalidStateError");
      }
      state.pending++;
      const settled = () => {
        state.pending--;
        extendEnd(state);
      };
      PromiseThen(resolved(promise), settled, () => {
        state.rejected = true;
        settled();
      });
    }
  }
  expose(ExtendableEvent, "ExtendableEvent");

  class InstallEvent extends ExtendableEvent {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, EVENT_INIT, "InstallEvent");
      super(args[0], args[1]);
    }
  }
  expose(InstallEvent, "InstallEvent");

  // Dispatches the host's extendable event `type` numbered `id` at the
  // global.
  function* swExtendable(type, id, data) {
    let event;
    if (type === "fetch") {
      event = new FetchEvent("fetch", {
        request: swRequest(data.request),
        clientId: data.clientId,
        resultingClientId: data.resultingClientId,
      });
      WeakMapGet(fetchStates, event).id = id;
    } else if (type === "message") {
      let message = null;
      let kind = "message";
      try {
        message = deserialize(unwire(data.data));
      } catch {
        kind = "messageerror";
      }
      event = new ExtendableMessageEvent(kind, {
        data: message,
        origin: data.origin,
        source: swClient(data.source),
      });
    } else {
      event = type === "install" ? new InstallEvent(type) : new ExtendableEvent(type);
    }
    eventState(event).trusted = true;
    const state = extendState(event);
    state.id = id;
    yield* dispatching(global, event);
    state.dispatched = true;
    // A request no handler answered goes to the network.
    if (type === "fetch" && !WeakMapGet(fetchStates, event).called) {
      swRespond(id, "network");
    }
    extendEnd(state);
    return false;
  }

  // ---- Clients (Service Workers 4.2, 4.3; M39 step 4) -----------------

  let swClientOf;

  class Client {
    #client;

    constructor(key, client) {
      if (key !== KEY) throw illegalConstructor("Client");
      this.#client = client;
      WeakSetAdd(swSources, this);
    }

    static {
      swClientOf = (client) => client.#client;
    }

    get url() {
      return this.#client.url;
    }

    get frameType() {
      return this.#client.frameType;
    }

    get id() {
      return this.#client.id;
    }

    get type() {
      return this.#client.type;
    }

    postMessage(message, transfer) {
      const iface = "Client";
      need(arguments.length, 1, "postMessage", iface);
      const ports = postTransfer(transfer, "postMessage", iface);
      if (ports.length > 0) {
        throw new $DOMException(prefix("postMessage", iface) +
          "A MessagePort could not be transferred from a service worker.", "DataCloneError");
      }
      const record = serialize(message, ports, prefix("postMessage", iface), null);
      swCall({ op: "clients.postMessage", client: this.#client.id, data: wire(record) });
    }
  }
  expose(Client, "Client");

  class WindowClient extends Client {
    constructor(key, client) {
      if (key !== KEY) throw illegalConstructor("WindowClient");
      super(KEY, client);
    }

    get visibilityState() {
      return swClientOf(this).visibilityState;
    }

    get focused() {
      return swClientOf(this).focused;
    }

    get ancestorOrigins() {
      return ObjectFreeze([]);
    }

    focus() {
      try {
        swCall({ op: "clients.focus", client: swClientOf(this).id });
        return resolved(this);
      } catch (e) {
        return rejected(e);
      }
    }

    navigate(url) {
      try {
        need(arguments.length, 1, "navigate", "WindowClient");
        swCall({ op: "clients.navigate", client: swClientOf(this).id, url: str(url) });
        return resolved(null);
      } catch (e) {
        return rejected(e);
      }
    }
  }
  expose(WindowClient, "WindowClient");

  function swClient(client) {
    return client === null || client === undefined ? null : new WindowClient(KEY, client);
  }

  class Clients {
    constructor(key) {
      if (key !== KEY) throw illegalConstructor("Clients");
    }

    get(id) {
      try {
        need(arguments.length, 1, "get", "Clients");
        return resolved(swClient(swCall({ op: "clients.get", client: str(id) })) ?? undefined);
      } catch (e) {
        return rejected(e);
      }
    }

    matchAll(options) {
      try {
        const given = options === undefined || options === null ? {} : options;
        const type = given.type === undefined ? "window" : str(given.type);
        const found = swCall({
          op: "clients.matchAll",
          includeUncontrolled: !!given.includeUncontrolled,
          type,
        });
        const out = [];
        for (let i = 0; i < found.length; i++) ArrayPush(out, swClient(found[i]));
        return resolved(out);
      } catch (e) {
        return rejected(e);
      }
    }

    openWindow(url) {
      try {
        need(arguments.length, 1, "openWindow", "Clients");
        swCall({ op: "clients.openWindow", url: str(url) });
        return resolved(null);
      } catch (e) {
        return rejected(e);
      }
    }

    claim() {
      try {
        swCall({ op: "clients.claim" });
        return resolved(undefined);
      } catch (e) {
        return rejected(e);
      }
    }
  }
  expose(Clients, "Clients");

  // An ExtendableMessageEvent's members (Service Workers 4.6).
  const extendableMessages = new $WeakMap();

  class ExtendableMessageEvent extends ExtendableEvent {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, EVENT_INIT, "ExtendableMessageEvent");
      super(args[0], args[1]);
      const given = eventInitDict === undefined || eventInitDict === null ? {} : eventInitDict;
      WeakMapSet(extendableMessages, this, {
        data: given.data === undefined ? null : given.data,
        origin: given.origin === undefined ? "" : str(given.origin),
        lastEventId: given.lastEventId === undefined ? "" : str(given.lastEventId),
        source: given.source === undefined ? null : given.source,
        ports: ObjectFreeze(given.ports === undefined ? [] : ArraySlice(given.ports, 0)),
      });
    }

    get data() {
      return WeakMapGet(extendableMessages, this).data;
    }

    get origin() {
      return WeakMapGet(extendableMessages, this).origin;
    }

    get lastEventId() {
      return WeakMapGet(extendableMessages, this).lastEventId;
    }

    get source() {
      return WeakMapGet(extendableMessages, this).source;
    }

    get ports() {
      return WeakMapGet(extendableMessages, this).ports;
    }
  }
  expose(ExtendableMessageEvent, "ExtendableMessageEvent");

  const swClients = SERVICE ? new Clients(KEY) : null;

  // ---- FetchEvent (Service Workers 4.5; M39 step 5) -------------------

  // A FetchEvent's members, and its answer's state: whether respondWith
  // was called, and the host's number for it (0 for one a script made).
  const fetchStates = new $WeakMap();

  // The request a FetchEvent is given, from the host's JSON.
  function swRequest(json) {
    const list = [];
    for (let i = 0; i < json.headers.length; i++) ArrayPush(list, json.headers[i]);
    return new Request(KEY, {
      method: json.method, url: json.url, headers: makeHeaders("request", list),
      body: bodyFrom(json.body === null ? null : new $Uint8Array(json.body)), mode: json.mode,
      credentials: json.credentials, cache: json.cache, redirect: json.redirect,
      referrer: json.referrer === "" ? "no-referrer" : json.referrer, referrerPolicy: "",
      integrity: "", keepalive: false, duplex: "half", signal: new AbortSignal(KEY),
      targetAddressSpace: "", destination: json.destination,
    });
  }

  // The host hears a FetchEvent's answer: none (the network's), a
  // network error, or a response, its body's bytes apart.
  function swRespond(id, answer, response) {
    if (id === 0) return;
    if (answer !== "response") {
      native.serviceWorkerRespond(JSONStringify({ id, answer }), null);
      return;
    }
    if (!isResponse(response)) {
      swRespond(id, "error");
      return;
    }
    const state = responseState(response);
    if (state.type === "error" || (state.body.bytes !== null && state.body.used)) {
      swRespond(id, "error");
      return;
    }
    const bytes = state.body.bytes === null ? new $Uint8Array(0) : state.body.bytes;
    if (state.body.bytes !== null) state.body.used = true;
    native.serviceWorkerRespond(JSONStringify({
      id, answer: "response",
      response: {
        type: state.type, urls: ArraySlice(state.urlList), status: state.status,
        statusText: state.statusText, headers: cloneList(headersState(state.headers).list),
      },
    }), viewLength(bytes) === 0 ? null : binaryString(bytes));
  }

  class FetchEvent extends ExtendableEvent {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, EVENT_INIT, "FetchEvent");
      super(args[0], args[1]);
      const given = eventInitDict === undefined || eventInitDict === null ? {} : eventInitDict;
      if (!isRequest(given.request)) {
        throw new $TypeError("Failed to construct 'FetchEvent': Failed to read the 'request' property from 'FetchEventInit': " +
          (given.request === undefined ? "Required member is undefined." : "Failed to convert value to 'Request'."));
      }
      WeakMapSet(fetchStates, this, {
        request: given.request,
        clientId: given.clientId === undefined ? "" : str(given.clientId),
        resultingClientId: given.resultingClientId === undefined ? "" : str(given.resultingClientId),
        handled: resolved(undefined),
        called: false,
        id: 0,
      });
    }

    get request() {
      return WeakMapGet(fetchStates, this).request;
    }

    get clientId() {
      return WeakMapGet(fetchStates, this).clientId;
    }

    get resultingClientId() {
      return WeakMapGet(fetchStates, this).resultingClientId;
    }

    get replacesClientId() {
      return "";
    }

    get handled() {
      return WeakMapGet(fetchStates, this).handled;
    }

    get preloadResponse() {
      return resolved(undefined);
    }

    respondWith(r) {
      const where = prefix("respondWith", "FetchEvent");
      need(arguments.length, 1, "respondWith", "FetchEvent");
      const state = WeakMapGet(fetchStates, this);
      if (extendState(this).dispatched) {
        throw new $DOMException(where + "The event handler is already finished.", "InvalidStateError");
      }
      if (state.called) {
        throw new $DOMException(where + "respondWith() was already called.", "InvalidStateError");
      }
      state.called = true;
      // Its promise keeps the event, as waitUntil's do.
      if (eventState(this).trusted) this.waitUntil(r);
      this.stopImmediatePropagation();
      const id = state.id;
      PromiseThen(resolved(r), (response) => swRespond(id, "response", response),
        () => swRespond(id, "error"));
    }
  }
  expose(FetchEvent, "FetchEvent");

  // ---- A service worker's global scope (Service Workers 4.1) ----------

  // The version's own registration and worker, made the first time from
  // the host's view.
  let swOwn = null;

  function swSelf() {
    if (swOwn === null) {
      const own = swCall({ op: "self" });
      swOwn = { registration: swRegistration(own.view), worker: swWorker(own.worker) };
    }
    return swOwn;
  }

  class ServiceWorkerGlobalScope extends WorkerGlobalScope {
    constructor() {
      throw illegalConstructor("ServiceWorkerGlobalScope");
    }

    get registration() {
      return swSelf().registration;
    }

    get serviceWorker() {
      return swSelf().worker;
    }

    get clients() {
      return swClients;
    }

    skipWaiting() {
      try {
        swCall({ op: "skipWaiting" });
        return resolved(undefined);
      } catch (e) {
        return rejected(e);
      }
    }
  }
  expose(ServiceWorkerGlobalScope, "ServiceWorkerGlobalScope");

  // What the host tells the realm (M39): a job's promise settled, a
  // registration's slot or a version's state changed, `updatefound`, an
  // extendable event, or a question of the events the global listens
  // for.
  function* swTask(text) {
    const told = JSONParse(text);
    switch (told.type) {
      case "settle": {
        const settle = MapGet(swPromises, told.promise);
        if (settle === undefined) return false;
        MapDelete(swPromises, told.promise);
        if (told.error !== undefined) {
          settle.reject(told.error.name === "TypeError"
            ? new $TypeError(told.error.message)
            : new $DOMException(told.error.message, told.error.name));
        } else if (told.view !== undefined) {
          settle.resolve(swRegistration(told.view));
        } else {
          settle.resolve(told.result);
        }
        return false;
      }
      case "slot": {
        const reg = MapGet(swRegistrations, told.registration);
        if (reg !== undefined) {
          swSetSlot(reg, told.slot, swWorker(told.view[told.slot]), told.view.updateViaCache);
        }
        return false;
      }
      case "state": {
        const worker = MapGet(swWorkers, told.version);
        if (worker !== undefined && swSetState(worker, told.state)) {
          const event = new Event("statechange");
          eventState(event).trusted = true;
          yield* dispatching(worker, event);
        }
        // `ready` settles once the active version's state is told.
        if (!WORKER) swCheckReady();
        return false;
      }
      case "updatefound": {
        const reg = MapGet(swRegistrations, told.registration);
        if (reg !== undefined) {
          const event = new Event("updatefound");
          eventState(event).trusted = true;
          yield* dispatching(reg, event);
        }
        return false;
      }
      case "event":
        return yield* swExtendable(told.kind, told.id, told.data);
      // A version's message for the document (M39 step 4).
      case "message":
        if (swContainer === null) return false;
        return yield* messageEvent(swContainer, told.data, told.origin, swWorker(told.source));
      // The document's controller changed: claimed, or a new version
      // activated.
      case "controller": {
        swController = swWorker(told.version);
        if (swContainer !== null) {
          const event = new Event("controllerchange");
          eventState(event).trusted = true;
          yield* dispatching(swContainer, event);
        }
        return false;
      }
      case "navigate":
        if (!WORKER) global.location.href = told.url;
        return false;
      case "listened": {
        const listeners = listenersOf(global);
        const types = [];
        for (let i = 0; i < listeners.length; i++) {
          if (!ArrayIncludes(types, listeners[i].type)) ArrayPush(types, listeners[i].type);
        }
        swCall({ op: "listened", types });
        return false;
      }
    }
    return false;
  }

  setPrototypeOf(
    global,
    SERVICE
      ? ServiceWorkerGlobalScope.prototype
      : WORKER
        ? DedicatedWorkerGlobalScope.prototype
        : Window.prototype,
  );
  // The global is tagged by its prototype, not by QuickJS-ng's own tag.
  ReflectDeleteProperty(global, toStringTag);
  if (DEDICATED) {
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
    "canplaythrough", "change", "click", "close", "command", "contextlost", "contextmenu",
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
    // The document's policies refused it when it was set (M33.1): no
    // handler then.
    const refused = WeakMapGet(refusedHandlers, element);
    if (refused !== undefined && RefusedHas(refused, type)) {
      return null;
    }
    const owner = native.formOwner(handle);
    const url = native.documentUrl();
    const source = "(function () { with (arguments[0]) with (arguments[1]) " +
      "with (arguments[2]) return function on" + type + "(" + params + ") {\n" + body +
      "\n}; })";
    try {
      // Named apart from the document's scripts: its `import()`s resolve
      // against the document's base URL, with the default options.
      const scope = native.evaluate(source, (url === null ? "" : url) + "\u0001handler");
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

  // The types of each element's handlers its document's policies
  // refused as their attributes were set.
  const refusedHandlers = new $WeakMap();
  const RefusedSet = Set;
  const RefusedHas = uncurry(Set.prototype.has);
  const RefusedAdd = uncurry(Set.prototype.add);
  const RefusedDelete = uncurry(Set.prototype.delete);

  // The line and column of the page's code that called in: the first
  // frame of a stack here, the bindings' own left out; 0 0 for none.
  function callerPosition() {
    const limit = ReflectApply(stackLimit.get, $Error, []);
    const prepare = ReflectApply(prepareStack.get, $Error, []);
    let stack = "";
    try {
      ReflectApply(prepareStack.set, $Error, [undefined]);
      ReflectApply(stackLimit.set, $Error, [STACK_PLACE]);
      stack = stackOf(new $Error());
    } catch (e) {
      stack = "";
    } finally {
      ReflectApply(stackLimit.set, $Error, [limit]);
      ReflectApply(prepareStack.set, $Error, [prepare]);
    }
    const lines = StringSplit(stack, "\n");
    for (let i = 0; i < lines.length; i++) {
      let line = StringTrim(lines[i]);
      if (line === "") continue;
      if (StringEndsWith(line, ")")) line = StringSlice(line, 0, line.length - 1);
      const parts = StringSplit(line, ":");
      if (parts.length < 3) continue;
      // The bindings' frames, by their file's name.
      const file = parts[parts.length - 3];
      if (StringEndsWith(file, "bootstrap") || StringIncludes(file, "bootstrap-")) continue;
      const row = $Number(parts[parts.length - 2]);
      const column = $Number(parts[parts.length - 1]);
      return [row === row ? row : 0, column === column ? column : 0];
    }
    return [0, 0];
  }

  // An attribute named `name` of `element` was set (`present`) or
  // removed: an `on...` one is the text of the element's handler, or,
  // on a body or frameset, of its window's. One set is held to the
  // document's policies then (HTML's attribute change steps), at the
  // place of the script that set it (`scripted`), else at none.
  function attributeHandler(element, name, present, scripted) {
    if (name === "popover") {
      popoverAttributeChanged(handleOf(element));
      return;
    }
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
    if (target === null) {
      return;
    }
    let refused = WeakMapGet(refusedHandlers, element);
    if (refused !== undefined) RefusedDelete(refused, type);
    if (present) {
      const handle = handleOf(element);
      const body = native.attribute(handle, name);
      const at = scripted ? callerPosition() : [0, 0];
      if (body !== null && native.codeAllowed("handler", body, handle, at[0], at[1]) !== "") {
        if (refused === undefined) {
          refused = new RefusedSet();
          WeakMapSet(refusedHandlers, element, refused);
        }
        RefusedAdd(refused, type);
      }
    }
    setHandler(target, type, null, present ? element : null);
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
  defineHandlers(ShadowRoot.prototype, ["slotchange"], false);
  defineHandlers(Document.prototype, DOCUMENT_ELEMENT_HANDLERS, false);
  defineHandlers(Document.prototype, ["readystatechange", "visibilitychange"], false);
  defineHandlers(Window.prototype, GLOBAL_HANDLERS, false);
  defineHandlers(Window.prototype, WINDOW_HANDLERS, false);
  // A worker's (M31): WorkerGlobalScope's, and the dedicated scope's
  // message handlers, the global's own.
  defineHandlers(WorkerGlobalScope.prototype,
    ["error", "languagechange", "rejectionhandled", "unhandledrejection"], false);
  if (DEDICATED) {
    defineHandlers(global, ["message", "messageerror"], false);
  }
  // A service worker's (M39).
  if (SERVICE) {
    defineHandlers(global, ["install", "activate", "fetch", "message", "messageerror"], false);
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
      return formSeen(self(this), self(this));
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
  // The other form-associated elements' `form` (HTML section
  // 4.10.18.3): their form owner, found in their own tree (D68).
  include(
    {
      get form() {
        return formSeen(self(this), self(this));
      },
    },
    [
      madeInterfaces.HTMLFieldSetElement.prototype,
      madeInterfaces.HTMLOutputElement.prototype,
      madeInterfaces.HTMLObjectElement.prototype,
    ],
  );
  // The input types the `list` attribute does not apply to.
  const LIST_IGNORED = [
    "hidden", "checkbox", "radio", "file", "submit", "image", "reset", "button", "password",
  ];

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

      // `list` (HTML section 4.10.5.3.10): the `datalist` its `list`
      // attribute names in its tree, a shadow tree's own (D68), for a
      // type the attribute applies to.
      get list() {
        const handle = self(this);
        const id = native.attribute(handle, "list");
        if (id === null || id === "" || ArrayIncludes(LIST_IGNORED, native.inputType(handle))) {
          return null;
        }
        const found = native.elementById(native.rootNode(handle, false), id);
        return found !== null && native.tag(found) === "datalist" ? wrap(found) : null;
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
        return formSeen(self(this), self(this));
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
        return select === null ? null : formSeen(select, self(this));
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
      // Its control, retargeted against it: a host whose reference
      // target the control is, rather than the control (gap batch H).
      get control() {
        const handle = self(this);
        const control = native.labeledControl(handle);
        return control === null ? null : wrap(native.retarget(control, handle));
      },

      get form() {
        const control = native.labeledControl(self(this));
        return control === null ? null : formSeen(control, self(this));
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
        return formSeen(self(this), self(this));
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

  // A form's indexed and named properties (HTML section 4.10.3): its
  // controls by index (`form[0]`) and by name or ID (`form.user`),
  // found through an object between HTMLFormElement.prototype and
  // HTMLElement.prototype, which a property neither has reaches with
  // the form as its receiver; what HTMLElement's chain has wins.
  if (!WORKER) {
    const formPrototype = madeInterfaces.HTMLFormElement.prototype;
    const above = ReflectGetPrototypeOf(formPrototype);
    const control = (form, key) => {
      if (typeof key !== "string" || !isNode(form) || ReflectHas(above, key)) {
        return undefined;
      }
      const i = arrayIndex(key);
      if (i >= 0) {
        const controls = native.formControls(handleOf(form));
        return i < controls.length ? wrap(controls[i]) : undefined;
      }
      const found = ReflectGet(formPrototype, "elements", form).namedItem(key);
      return found === null ? undefined : found;
    };
    setPrototypeOf(
      formPrototype,
      new $Proxy(above, {
        get(target, key, receiver) {
          const found = control(receiver, key);
          return found === undefined ? ReflectGet(target, key, receiver) : found;
        },
        getPrototypeOf() {
          return above;
        },
      }),
    );
  }

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
      // The user's change committed: `:user-invalid` may show, as when
      // the engine's own unfocusing commits it.
      native.setUserValidity(handle);
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
    // A shadow host that delegates focus gives it to its focus delegate,
    // unless the focus is in it already (the focusing steps' step 2, D68).
    if (delegatesFocus(handle)) {
      const focused = native.focused();
      if (focused !== null && shadowIncludes(handle, focused)) {
        return;
      }
      const delegate = native.focusDelegate(handle);
      if (delegate === null) {
        return;
      }
      handle = delegate;
    }
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

  // Whether the element `handle` hosts a shadow root that delegates
  // focus.
  function delegatesFocus(handle) {
    const root = native.shadowRoot(handle);
    return root !== null && shadowInfo(root).delegatesFocus;
  }

  // Whether `node` is a shadow-including inclusive descendant of
  // `ancestor`, its parents and hosts walked up.
  function shadowIncludes(ancestor, node) {
    for (let at = node; at !== null; ) {
      if (at === ancestor) {
        return true;
      }
      const parent = native.parent(at);
      at = parent !== null ? parent : native.kind(at) === 11 ? native.shadowHost(at) : null;
    }
    return false;
  }

  // HTML's unfocusing steps: the element loses the focus, to the
  // viewport; a shadow host's blur blurs the focus in its tree (D68).
  function blurElement(handle) {
    const focused = native.focused();
    if (focused !== null && focused !== handle && native.shadowRoot(handle) !== null &&
      native.retarget(focused, handle) === handle) {
      handle = focused;
    }
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

  // ---- The top layer, dialogs, popovers and invokers (gap batch H) ---------

  // HTML sections 4.11.4 (dialogs), 6.12 (popovers), 6.13 (close
  // watchers) and 6.14 (interest invokers), and the invokers' activation
  // (section 4.10.6). The engine keeps the top layer, the popover lists,
  // the open dialogs and the close watchers (`dom::Document`'s), which
  // the cascade, layout and hit testing read, and runs these algorithms
  // itself for a page whose scripts are off; with scripts on they are
  // these, with their events, reaching that state through
  // `native.topLayer`.
  function topLayer(op, a = null, b = null, c = null, d = null, word = "") {
    return native.topLayer(op, a, b, c, d, word);
  }

  // "hide all popovers until" the document: every popover.
  const ALL_POPOVERS = -1;
  // The most rounds a loop hiding popovers takes, in case listeners
  // show one again each time.
  const HIDING_ROUNDS = 4096;

  // The elements whose popover is being shown or hidden (HTML's
  // "popover showing or hiding"), and the `toggle` tasks queued, each's
  // id and the old state it carries ("toggle task tracker"), by handle.
  const popoverBusy = new $Map();
  const toggleTasks = new $Map();
  // A dialog's `returnValue`, by its object.
  const returnValues = new $WeakMap();

  function isHtmlElement(handle) {
    return native.kind(handle) === 1 && nsOf(wrap(handle)) === HTML_NS;
  }

  function isDialog(handle) {
    return isHtmlElement(handle) && native.tag(handle) === "dialog";
  }

  // The state of an element's `popover` attribute: `auto`, `hint`,
  // `manual` (an invalid value's too), or null for none.
  function popoverAttribute(handle) {
    if (!isHtmlElement(handle)) return null;
    const value = native.attribute(handle, "popover");
    if (value === null) return null;
    const lower = ToLowerCase(value);
    if (lower === "" || lower === "auto") return "auto";
    return lower === "hint" ? "hint" : "manual";
  }

  // A showing popover's mode and list (`auto`, `hint` or `none`), or
  // null for a hidden one.
  function popoverShown(handle) {
    const state = topLayer("popover", handle);
    return state.length === 0 ? null : state;
  }

  function popoverList(list) {
    return topLayer("list", null, null, null, null, list);
  }

  // An open dialog's state, `modal` or `open`; null for a closed one.
  function dialogShown(handle) {
    const state = topLayer("dialog", handle);
    return state.length === 0 ? null : state[0];
  }

  function hasOpen(handle) {
    return native.attribute(handle, "open") !== null;
  }

  // HTML's "check popover validity": whether `handle` is a popover in
  // the state `showing` says, in the page's document and not a modal
  // dialog, else false or, when `throws`, the reference browser's
  // exception.
  function popoverValid(handle, showing, throws, method) {
    // One shown is hidden whatever its attribute says now: the type it
    // was shown as is what counts, as the reference browser keeps it.
    if (popoverAttribute(handle) === null && !(showing && popoverShown(handle) !== null)) {
      if (throws) {
        throw domError(
          "NotSupportedError",
          "Not supported on elements that do not have a valid value for the 'popover' attribute.",
          method,
          "HTMLElement",
        );
      }
      return false;
    }
    if ((popoverShown(handle) !== null) !== showing) return false;
    const unfit = !native.connected(handle) ||
      !isPage(native.ownerDocument(handle)) ||
      (isDialog(handle) && (dialogShown(handle) === "modal" || (!showing && hasOpen(handle))));
    if (unfit) {
      if (throws) {
        throw domError("InvalidStateError", "Invalid on disconnected popover elements.", method,
          "HTMLElement");
      }
      return false;
    }
    return true;
  }

  // Fires `beforetoggle` at `handle`: whether it was not canceled. One
  // with a source is composed, as the reference browser's is.
  function fireBeforeToggle(handle, oldState, newState, source, cancelable) {
    const event = new ToggleEvent("beforetoggle", made(TOGGLE_INIT, [
      "cancelable", cancelable,
      "composed", source !== null,
      "newState", newState,
      "oldState", oldState,
      "source", source === null ? null : wrap(source),
    ]));
    eventState(event).trusted = true;
    return !drain(dispatching(wrap(handle), event));
  }

  // HTML's "queue a popover toggle event task" and "queue a dialog toggle
  // event task": a `toggle` queued, one queued already taken back and
  // its old state kept, so that a run of changes makes one event.
  function queueToggle(handle, oldState, newState, source) {
    const queued = MapGet(toggleTasks, handle);
    if (queued !== undefined) {
      oldState = queued.oldState;
      cancelTask(queued.id);
    }
    const element = wrap(handle);
    const from = source === null ? null : wrap(source);
    const id = queueTask(function* () {
      MapDelete(toggleTasks, handle);
      const event = new ToggleEvent("toggle", made(TOGGLE_INIT, [
        "composed", from !== null,
        "newState", newState,
        "oldState", oldState,
        "source", from,
      ]));
      eventState(event).trusted = true;
      yield* dispatching(element, event);
    }, 0, true);
    MapSet(toggleTasks, handle, { id, oldState });
  }

  // The `toggle` tasks of `details` elements, by element: HTML's details
  // toggle task tracker, its task and its old state.
  const detailsToggleTasks = new $Map();

  // The toggles the document's `details` owe, each queued as HTML's
  // "queue a details toggle event task" queues one: a task still waiting
  // gives way to the new one, which keeps its old state.
  function takeDetailsToggles() {
    const toggles = native.takeDetailsToggles();
    for (let i = 0; i < toggles.length; i += 2) {
      const handle = toggles[i];
      let oldState = toggles[i + 1] ? "open" : "closed";
      const newState = native.attribute(handle, "open") !== null ? "open" : "closed";
      const queued = MapGet(detailsToggleTasks, handle);
      if (queued !== undefined) {
        oldState = queued.oldState;
        cancelTask(queued.id);
      }
      const element = wrap(handle);
      const id = queueTask(function* () {
        MapDelete(detailsToggleTasks, handle);
        const event = new ToggleEvent("toggle", made(TOGGLE_INIT, [
          "newState", newState,
          "oldState", oldState,
        ]));
        eventState(event).trusted = true;
        yield* dispatching(element, event);
      }, 0, true);
      MapSet(detailsToggleTasks, handle, { id, oldState });
    }
  }

  // The `details` the `summary` is the summary for: its parent, when it
  // is that parent's first `summary` child; or null.
  function summaryDetails(handle) {
    const parent = native.parent(handle);
    if (parent === null || native.kind(parent) !== 1 || native.tag(parent) !== "details" ||
        !isHtmlElement(parent) || !isHtmlElement(handle)) {
      return null;
    }
    for (let n = native.firstChild(parent); n !== null; n = native.nextSibling(n)) {
      if (native.kind(n) === 1 && native.tag(n) === "summary" && isHtmlElement(n)) {
        return n === handle ? parent : null;
      }
    }
    return null;
  }

  // HTML's "nearest inclusive open popover": the nearest inclusive
  // ancestor that is an auto or hint popover showing.
  function nearestOpenPopover(handle) {
    for (let n = handle; n !== null; n = flatParent(n)) {
      if (native.kind(n) !== 1) continue;
      const shown = popoverShown(n);
      if (shown !== null && (shown[0] === "auto" || shown[0] === "hint")) return n;
    }
    return null;
  }

  // HTML's "topmost popover ancestor" of `handle` among `list`, by its
  // parent and its `source`: the one highest in the list, or null.
  // `handle` counts as above them when it is a popover about to show.
  function topmostPopoverAncestor(handle, list, source, isPopover) {
    const positions = new $Map();
    for (let i = 0; i < list.length; i++) MapSet(positions, list[i], i);
    if (isPopover) MapSet(positions, handle, list.length);
    let topmost = null;
    const check = (candidate) => {
      if (candidate === null) return;
      const ancestor = nearestOpenPopover(candidate);
      const at = ancestor === null ? undefined : MapGet(positions, ancestor);
      if (at !== undefined && (topmost === null || MapGet(positions, topmost) < at)) {
        topmost = ancestor;
      }
    };
    check(flatParent(handle));
    check(source);
    return topmost;
  }

  // HTML's "close entire popover list": each hidden, the topmost first.
  function closeEntireList(list, focusPrevious, fireEvents) {
    for (let round = 0; round < HIDING_ROUNDS; round++) {
      const popovers = popoverList(list);
      if (popovers.length === 0) return;
      hidePopover(popovers[popovers.length - 1], focusPrevious, fireEvents, false, null, "");
    }
  }

  // HTML's "hide popover stack until": the popovers of `list` above
  // `endpoint` hidden, the topmost first; the whole list when it does
  // not hold it.
  function hideStackUntil(endpoint, list, focusPrevious, fireEvents) {
    for (let round = 0; round < HIDING_ROUNDS; round++) {
      let lastToHide = null;
      let found = false;
      const popovers = popoverList(list);
      for (let i = 0; i < popovers.length; i++) {
        if (popovers[i] === endpoint) {
          found = true;
        } else if (found) {
          lastToHide = popovers[i];
          break;
        }
      }
      if (!found) {
        closeEntireList(list, focusPrevious, fireEvents);
        return;
      }
      for (let i = 0; i < HIDING_ROUNDS && lastToHide !== null && popoverShown(lastToHide) !== null;
        i++) {
        const now = popoverList(list);
        if (now.length === 0) break;
        hidePopover(now[now.length - 1], focusPrevious, fireEvents, false, null, "");
      }
      const after = popoverList(list);
      const repeating = ArrayIncludes(after, endpoint) && after[after.length - 1] !== endpoint;
      if (!repeating) return;
      fireEvents = false;
    }
  }

  // HTML's "hide all popovers until": those above `endpoint` in its list,
  // or every one for ALL_POPOVERS, the hint list first.
  function hideAllPopoversUntil(endpoint, focusPrevious, fireEvents) {
    if (endpoint === ALL_POPOVERS) {
      closeEntireList("hint", focusPrevious, fireEvents);
      closeEntireList("auto", focusPrevious, fireEvents);
      return;
    }
    const shown = popoverShown(endpoint);
    if (shown === null || shown[1] === "none") return;
    hideStackUntil(endpoint, shown[1], focusPrevious, fireEvents);
  }

  // HTML's "show popover", as the reference browser stacks a hint: an
  // auto popover closes the hint list and the auto popovers it is not
  // in; a hint closes the other hints, and hangs from the auto popover
  // it is in, if any; either, shown in a hint, joins the hint list,
  // closing what is above that hint and the auto popovers the hint list
  // does not hang from.
  function showPopover(handle, throws, source, method) {
    if (!popoverValid(handle, false, throws, method)) return;
    const nested = MapHas(popoverBusy, handle);
    const fireEvents = !nested;
    MapSet(popoverBusy, handle, true);
    try {
      if (!fireBeforeToggle(handle, "closed", "open", source, true)) return;
      if (!popoverValid(handle, false, throws, method)) return;
      const original = popoverAttribute(handle);
      let list = "none";
      let anchor = null;
      if (original === "auto" || original === "hint") {
        const hints = popoverList("hint");
        const hintAncestor = topmostPopoverAncestor(handle, hints, source, true);
        if (hintAncestor !== null) {
          // The auto popovers above the one the hint list hangs from go,
          // all of them when it hangs from none.
          const from = topLayer("anchor", hints[0]);
          if (from === null) {
            closeEntireList("auto", false, fireEvents);
          } else {
            hideAllPopoversUntil(from, false, fireEvents);
          }
          hideAllPopoversUntil(hintAncestor, false, fireEvents);
          list = "hint";
        } else {
          closeEntireList("hint", false, fireEvents);
          const autoAncestor = topmostPopoverAncestor(handle, popoverList("auto"), source, true);
          if (original === "auto") {
            hideAllPopoversUntil(autoAncestor === null ? ALL_POPOVERS : autoAncestor, false,
              fireEvents);
            list = "auto";
          } else {
            anchor = autoAncestor;
            list = "hint";
          }
        }
        // A type changed while the others hid is an error (HTML's "show
        // popover", its original type).
        if (popoverAttribute(handle) !== original) {
          if (throws) {
            throw domError("InvalidStateError", "The popover's type changed while others hid.",
              method, "HTMLElement");
          }
          return;
        }
        if (!popoverValid(handle, false, throws, method)) return;
      }
      // The focus is given back on hiding for the first auto or hint
      // popover alone, a manual one keeping none (HTML's
      // shouldRestoreFocus).
      const first = (original === "auto" || original === "hint") &&
        popoverList("auto").length === 0 && popoverList("hint").length === 0;
      const focused = native.focused();
      topLayer("show", handle, source, null, anchor, original + " " + list);
      popoverFocusing(handle);
      if (first && popoverAttribute(handle) !== null) {
        topLayer("setFocus", handle, focused);
      }
      queueToggle(handle, "closed", "open", source);
    } finally {
      if (!nested) MapDelete(popoverBusy, handle);
    }
  }

  // HTML's "hide popover algorithm": those above it hidden first, then
  // it, its `beforetoggle` and `toggle` when `fireEvents`, and the focus
  // given back to what had it before when what is in it has it.
  function hidePopover(handle, focusPrevious, fireEvents, throws, source, method) {
    if (!popoverValid(handle, true, throws, method)) return;
    const nested = MapHas(popoverBusy, handle);
    MapSet(popoverBusy, handle, true);
    if (nested) fireEvents = false;
    try {
      const shown = popoverShown(handle);
      if (shown[0] === "auto" || shown[0] === "hint") {
        hideAllPopoversUntil(handle, focusPrevious, fireEvents);
        if (!popoverValid(handle, true, throws, method)) return;
      }
      // The hint list hanging from it goes with it.
      if (shown[1] === "auto") {
        const hints = popoverList("hint");
        if (hints.length > 0 && topLayer("anchor", hints[0]) === handle) {
          closeEntireList("hint", focusPrevious, fireEvents);
          if (!popoverValid(handle, true, throws, method)) return;
        }
      }
      const autos = popoverList("auto");
      const wasTop = autos.length > 0 && autos[autos.length - 1] === handle;
      if (fireEvents) {
        fireBeforeToggle(handle, "open", "closed", source, false);
        const now = popoverList("auto");
        if (wasTop && (now.length === 0 || now[now.length - 1] !== handle)) {
          hideAllPopoversUntil(handle, focusPrevious, false);
        }
        if (!popoverValid(handle, true, throws, method)) return;
      }
      const previous = topLayer("hide", handle);
      if (fireEvents) queueToggle(handle, "open", "closed", source);
      if (previous !== null && focusPrevious) {
        const focused = native.focused();
        if (focused !== null && shadowIncludes(handle, focused)) focusElement(previous, true);
      }
      focusFixup(handle);
    } finally {
      if (!nested) MapDelete(popoverBusy, handle);
    }
  }

  // HTML's "toggle popover": whether it is showing after.
  function togglePopover(handle, force, throws, source, method) {
    if (popoverShown(handle) !== null && force !== true) {
      hidePopover(handle, true, true, throws, source, method);
    } else if (force !== false) {
      showPopover(handle, throws, source, method);
    } else {
      popoverValid(handle, popoverShown(handle) !== null, throws, method);
    }
    return popoverShown(handle) !== null;
  }

  // HTML's "popover focusing steps": a dialog's own; else the popover
  // itself with `autofocus`, or the first focusable descendant with it.
  function popoverFocusing(handle) {
    if (isDialog(handle)) {
      dialogFocusing(handle);
      return;
    }
    const control = topLayer("delegate", handle, null, null, null, "popover");
    if (control !== null) focusElement(control, false);
  }

  // HTML's "dialog focusing steps", as the reference browser has them:
  // its focus delegate, else the dialog.
  function dialogFocusing(handle) {
    const control = topLayer("delegate", handle, null, null, null, "dialog");
    focusElement(control === null ? handle : control, false);
  }

  // The popover attribute changed (HTML's popover attribute change
  // steps): a popover shown in another state hidden.
  function popoverAttributeChanged(handle) {
    const shown = popoverShown(handle);
    if (shown !== null && shown[0] !== popoverAttribute(handle)) {
      hidePopover(handle, true, true, false, null, "");
    }
  }

  // Where showing a dialog hides popovers until: the topmost hint, else
  // auto, popover it is in, else every popover.
  function dialogHideUntil(handle) {
    let until = topmostPopoverAncestor(handle, popoverList("hint"), null, false);
    if (until === null) until = topmostPopoverAncestor(handle, popoverList("auto"), null, false);
    return until === null ? ALL_POPOVERS : until;
  }

  // `show()`.
  function showDialog(handle) {
    if (hasOpen(handle) && dialogShown(handle) !== "modal") return;
    if (hasOpen(handle)) {
      throw domError(
        "InvalidStateError",
        "The dialog is already open as a modal dialog, and therefore cannot be opened as a non-modal dialog.",
        "show",
        "HTMLDialogElement",
      );
    }
    if (!fireBeforeToggle(handle, "closed", "open", null, true) || hasOpen(handle)) return;
    queueToggle(handle, "closed", "open", null);
    setAttribute(handle, "open", "", "show");
    topLayer("openDialog", handle, native.focused(), null, null, "open");
    hideAllPopoversUntil(dialogHideUntil(handle), false, true);
    dialogFocusing(handle);
  }

  // HTML's "show a modal dialog".
  function showModal(handle, source) {
    const open = hasOpen(handle);
    if (open && dialogShown(handle) === "modal") return;
    const refuse = (message) =>
      domError("InvalidStateError", message, "showModal", "HTMLDialogElement");
    if (open) {
      throw refuse(
        "The dialog is already open as a non-modal dialog, and therefore cannot be opened as a modal dialog.",
      );
    }
    if (!native.connected(handle) || !isPage(native.ownerDocument(handle))) {
      throw refuse("The element is not in a Document.");
    }
    if (popoverShown(handle) !== null) {
      throw refuse(
        "The dialog is already open as a Popover, and therefore cannot be opened as a modal dialog.",
      );
    }
    if (!fireBeforeToggle(handle, "closed", "open", source, true)) return;
    if (hasOpen(handle) || !native.connected(handle) || popoverShown(handle) !== null) return;
    queueToggle(handle, "closed", "open", source);
    setAttribute(handle, "open", "", "showModal");
    topLayer("openDialog", handle, native.focused(), null, null, "modal");
    hideAllPopoversUntil(dialogHideUntil(handle), false, true);
    dialogFocusing(handle);
  }

  // HTML's "close the dialog": `result`, unless null, its return value;
  // the focus back where it was when what is in it has it or it was
  // modal; `close` queued.
  function closeDialog(handle, result, source) {
    if (!hasOpen(handle)) return;
    fireBeforeToggle(handle, "open", "closed", source, false);
    if (!hasOpen(handle)) return;
    queueToggle(handle, "open", "closed", source);
    removeAttribute(handle, "open");
    const wasModal = dialogShown(handle) === "modal";
    const previous = topLayer("closeDialog", handle);
    const element = wrap(handle);
    if (result !== null) WeakMapSet(returnValues, element, result);
    if (previous !== null) {
      const focused = native.focused();
      if (wasModal || (focused !== null && shadowIncludes(handle, focused))) {
        focusElement(previous, true);
      }
    }
    focusFixup(handle);
    // `close` comes with the next frame's rendering, before its
    // animation frame callbacks, as the reference browser fires it.
    ArrayPush(frameEvents, element);
    native.wantFrame();
  }

  // The dialogs a `close` waits for the next frame for, in order.
  const frameEvents = [];

  // Fires the `close` events the frame owes (its rendering steps).
  function* fireFrameEvents() {
    const due = ArraySplice(frameEvents, 0, frameEvents.length);
    for (let i = 0; i < due.length; i++) {
      const event = new Event("close", made(EVENT_INIT, []));
      eventState(event).trusted = true;
      yield* dispatching(due[i], event);
    }
  }

  // HTML's focus fixup for what was hidden: the focus left in it, no
  // longer rendered, goes to the viewport, with no event, as the
  // reference browser has it.
  function focusFixup(handle) {
    const focused = native.focused();
    if (focused !== null && shadowIncludes(handle, focused)) native.setFocus(null, true);
  }

  // A dialog's computed closed-by state: its `closedby`, or by default
  // `closerequest` for a modal dialog and `none` for another.
  function closedByState(handle) {
    const value = native.attribute(handle, "closedby");
    const lower = value === null ? "" : ToLowerCase(value);
    if (lower === "any" || lower === "closerequest" || lower === "none") return lower;
    return dialogShown(handle) === "modal" ? "closerequest" : "none";
  }

  // Fires `cancel` at a dialog: whether it was not canceled.
  function fireCancel(handle, cancelable) {
    const event = new Event("cancel", made(EVENT_INIT, ["cancelable", cancelable]));
    eventState(event).trusted = true;
    return !drain(dispatching(wrap(handle), event));
  }

  // HTML's "request close the dialog": its close watcher asked to close,
  // `cancel` first; a dialog the parser opened, which has none, closed.
  function requestCloseDialog(handle, result, source) {
    if (!hasOpen(handle) || !native.connected(handle)) return;
    // A request while its `cancel` runs does nothing (the close
    // watcher's "is running cancel action").
    if (MapHas(cancelling, handle)) return;
    // One its `open` attribute opened has its `cancel` too, as the
    // reference browser gives it.
    MapSet(cancelling, handle, true);
    let go;
    try {
      go = fireCancel(handle, true);
    } finally {
      MapDelete(cancelling, handle);
    }
    if (go) closeDialog(handle, result, source);
  }

  // The dialogs whose `cancel` a request to close is running.
  const cancelling = new $Map();

  // Whether `handle` is a button, or an input that is one, which a
  // `popovertarget` applies to.
  function isButtonLike(handle) {
    const tag = native.tag(handle);
    if (tag === "button") return isHtmlElement(handle);
    if (tag !== "input") return false;
    const kind = native.inputType(handle);
    return kind === "button" || kind === "submit" || kind === "reset" || kind === "image";
  }

  // The element an element-reflecting attribute refers to, as what acts
  // on it takes it: one a script set, while in a tree around the
  // element, else the first of the attribute's ID in the element's tree,
  // shadow roots' reference targets followed (the engine's, which its
  // own algorithms read too).
  function reflectedElement(handle, attribute) {
    return native.elementReference("referenced", handle, attribute);
  }

  // The elements an element-reflecting attribute names, as its IDL
  // attribute gives them: a host, not what its reference target names.
  function associatedElements(handle, attribute) {
    return native.elementReference("associated", handle, attribute);
  }

  // Sets an element-reflecting attribute to `value`, an Element or null:
  // the content attribute the empty string, or none.
  function setAttrElement(handle, attribute, value, property, iface) {
    if (value !== null && !(isNode(value) && kindOf(value) === 1)) {
      throw new $TypeError("Failed to set the '" + property + "' property on '" + iface +
        "': Failed to convert value to 'Element'.");
    }
    if (value === null) {
      removeAttribute(handle, attribute);
      native.elementReference("unset", handle, attribute);
      return;
    }
    setAttribute(handle, attribute, "", property);
    native.elementReference("set", handle, attribute, [handleOf(value)]);
  }

  // The same of a list of elements (`ariaLabelledByElements`, ...).
  function setAttrElements(handle, attribute, value, property) {
    if (value === null || value === undefined) {
      removeAttribute(handle, attribute);
      native.elementReference("unset", handle, attribute);
      return;
    }
    const list = [];
    const length = value.length >>> 0;
    for (let i = 0; i < length; i++) {
      const item = value[i];
      if (!isNode(item) || kindOf(item) !== 1) {
        throw new $TypeError("Failed to set the '" + property +
          "' property on 'Element': Failed to convert value to 'Element'.");
      }
      ArrayPush(list, handleOf(item));
    }
    setAttribute(handle, attribute, "", property);
    native.elementReference("set", handle, attribute, list);
  }

  // HTML's "popover target element" of a button.
  function popoverTargetOf(handle) {
    if (!isButtonLike(handle) || native.disabled(handle)) return null;
    if (native.formOwner(handle) !== null && isSubmitButton(handle)) return null;
    const popover = reflectedElement(handle, "popovertarget");
    return popover !== null && popoverAttribute(popover) !== null ? popover : null;
  }

  function popoverTargetAction(handle) {
    const value = native.attribute(handle, "popovertargetaction");
    const lower = value === null ? "" : ToLowerCase(value);
    return lower === "show" || lower === "hide" ? lower : "toggle";
  }

  // The commands a `command` attribute takes, past a custom `--` one.
  const COMMANDS = [
    "toggle-popover", "show-popover", "hide-popover", "close", "request-close", "show-modal",
  ];

  // A button's `command`: a built-in command in lowercase, a custom one
  // as written, or the empty string for none.
  function commandOf(handle) {
    const value = native.attribute(handle, "command");
    if (value === null) return "";
    if (StringSlice(value, 0, 2) === "--") return value;
    const lower = ToLowerCase(value);
    return ArrayIncludes(COMMANDS, lower) ? lower : "";
  }

  // An element's optional value: its `value` attribute, or null.
  function optionalValue(handle) {
    return native.attribute(handle, "value");
  }

  // A button's activation past its form's (HTML section 4.10.6): its
  // command at its `commandfor` element, else its popover target's
  // (section 6.12.3), `target` the click's. Whether it had either.
  function invokerActivation(handle, target) {
    if (native.disabled(handle)) return false;
    if (native.tag(handle) === "button" && isHtmlElement(handle)) {
      const commanded = reflectedElement(handle, "commandfor");
      if (commanded !== null) {
        // One of a form whose type is its auto state neither submits nor
        // commands, as the reference browser has it.
        const type = native.attribute(handle, "type");
        const lower = type === null ? "" : ToLowerCase(type);
        const auto = lower !== "reset" && lower !== "button" && lower !== "submit";
        if (!(auto && native.formOwner(handle) !== null)) runCommand(handle, commanded);
        return true;
      }
    }
    const popover = popoverTargetOf(handle);
    if (popover === null) return false;
    // A press in the popover the button holds does nothing; a button that
    // is its own popover holds none (a shadow-including descendant, not
    // an inclusive one).
    if (isNode(target) && shadowIncludes(popover, handleOf(target)) && popover !== handle &&
      shadowIncludes(handle, popover)) {
      return true;
    }
    const action = popoverTargetAction(handle);
    const shown = popoverShown(popover) !== null;
    if ((action === "show" && shown) || (action === "hide" && !shown)) return true;
    if (shown) {
      hidePopover(popover, true, true, false, handle, "");
    } else if (popoverValid(popover, false, false, "")) {
      showPopover(popover, false, handle, "");
    }
    return true;
  }

  // A button's command at `target` (HTML section 4.10.6's steps): a
  // cancelable `command` first, then a popover's or a dialog's own.
  function runCommand(button, target) {
    const command = commandOf(button);
    if (command === "") return;
    const custom = StringSlice(command, 0, 2) === "--";
    const popover = popoverAttribute(target) !== null;
    const dialogCommand = command === "show-modal" || command === "close" ||
      command === "request-close";
    // The popover commands are any HTML element's, which acts on them
    // when it is a popover; the dialog commands a dialog's alone.
    const popoverCommand = command === "show-popover" || command === "hide-popover" ||
      command === "toggle-popover";
    if (!custom && !(popoverCommand && isHtmlElement(target)) &&
      !(isDialog(target) && dialogCommand)) {
      return;
    }
    const event = new CommandEvent("command", made(COMMAND_INIT, [
      "cancelable", true,
      "command", command,
      "composed", true,
      "source", wrap(button),
    ]));
    eventState(event).trusted = true;
    if (drain(dispatching(wrap(target), event))) return;
    if (!native.connected(target) || custom) return;
    if (popover && command === "hide-popover") {
      if (popoverValid(target, true, false, "")) hidePopover(target, true, true, false, button, "");
    } else if (popover && command === "toggle-popover") {
      togglePopover(target, null, false, button, "");
    } else if (popover && command === "show-popover") {
      if (popoverValid(target, false, false, "")) showPopover(target, false, button, "");
    } else if (isDialog(target) && command === "show-modal") {
      if (!hasOpen(target) && popoverShown(target) === null) showModal(target, button);
    } else if (isDialog(target) && command === "close") {
      closeDialog(target, optionalValue(button), button);
    } else if (isDialog(target) && command === "request-close") {
      requestCloseDialog(target, optionalValue(button), button);
    }
  }

  // A form's method as its submission takes it: its submitter's
  // `formmethod`, else its own `method`, lowercased.
  function formMethod(form, submitter) {
    let value = submitter === null ? null : native.attribute(submitter, "formmethod");
    if (value === null) value = native.attribute(form, "method");
    return value === null ? "" : ToLowerCase(value);
  }

  // The nearest dialog around `handle`, if any.
  function dialogAround(handle) {
    for (let n = native.parent(handle); n !== null; n = native.parent(n)) {
      if (isDialog(n)) return n;
    }
    return null;
  }

  // ---- Close requests and light dismiss --------------------------------

  // The engine's close request (HTML section 6.13.2, the user's Escape):
  // the topmost close watcher asked to close, its `cancel` cancelable
  // when `canPrevent`; a dialog whose closed-by state is `none` takes
  // none. Whether its closing was canceled.
  function* closeRequestTask(canPrevent) {
    // HTML's "process close watchers": the last group's, the newest
    // first, each enabled one asked to close until one is kept open.
    const watchers = topLayer("watchers");
    for (let i = watchers.length - 2; i >= 0; i -= 2) {
      if (!requestCloseOf(watchers[i], watchers[i + 1], canPrevent)) return true;
    }
    return false;
  }

  // HTML's "request to close" the close watcher of `kind` (1 an
  // element's, 2 a script's) and `which`, with history-action activation
  // required: whether it closed, or was not enabled; false when its
  // `cancel` was canceled.
  function requestCloseOf(kind, which, canPrevent) {
    if (kind === 2) {
      const watcher = MapGet(scriptWatchers, which);
      return watcher === undefined || watcherRequestClose(watcher, canPrevent);
    }
    if (isDialog(which) && dialogShown(which) !== null) {
      if (closedByState(which) === "none") return true;
      if (!fireCancel(which, canPrevent)) return false;
      closeDialog(which, null, null);
      return true;
    }
    if (popoverShown(which) !== null) hidePopover(which, true, true, false, null, "");
    return true;
  }

  // The popover and the dialog a press began on, as light dismiss
  // knows them (HTML's "popover pointerdown target" and "dialog
  // pointerdown target").
  let popoverPressed = null;
  let dialogPressed = null;

  // The engine's light dismiss (HTML sections 6.12 and 4.11.4), the
  // trusted `pointerdown` (`up` false) or `pointerup` at `target`, at
  // `(x, y)` in the viewport.
  function* lightDismissTask(target, up, x, y) {
    lightDismissPopovers(target, up);
    lightDismissDialogs(target, up, x, y);
    return false;
  }

  function lightDismissPopovers(target, up) {
    if (popoverList("hint").length === 0 && popoverList("auto").length === 0) return;
    if (!up) {
      popoverPressed = topmostClickedPopover(target);
      return;
    }
    const ancestor = topmostClickedPopover(target);
    const same = ancestor === popoverPressed;
    popoverPressed = null;
    if (same) hideAllPopoversUntil(ancestor === null ? ALL_POPOVERS : ancestor, false, true);
  }

  // HTML's "topmost clicked popover".
  function topmostClickedPopover(node) {
    const clicked = nearestOpenPopover(node);
    const targeted = nearestTargetPopover(node);
    return stackPosition(clicked) > stackPosition(targeted) ? clicked : targeted;
  }

  // HTML's "popover stack position": past the auto list in the hint
  // list, from 1; 0 for none.
  function stackPosition(popover) {
    if (popover === null) return 0;
    const autos = popoverList("auto");
    const hints = popoverList("hint");
    for (let i = 0; i < hints.length; i++) {
      if (hints[i] === popover) return autos.length + i + 1;
    }
    for (let i = 0; i < autos.length; i++) {
      if (autos[i] === popover) return i + 1;
    }
    return 0;
  }

  // HTML's "nearest inclusive target popover for invoker": the auto or
  // hint popover showing that the nearest invoker around `node` targets.
  function nearestTargetPopover(node) {
    for (let n = node; n !== null; n = flatParent(n)) {
      if (native.kind(n) !== 1) continue;
      let popover = popoverTargetOf(n);
      // A command button counts when it would run a popover command: not
      // a submit or reset button of a form, nor one of a form of no type.
      if (popover === null && native.tag(n) === "button" && isHtmlElement(n)) {
        const command = commandOf(n);
        const popoverCommand = command === "show-popover" || command === "hide-popover" ||
          command === "toggle-popover";
        const type = native.attribute(n, "type");
        const explicit = type !== null && ToLowerCase(type) === "button";
        if (popoverCommand && !native.disabled(n) && (explicit || native.formOwner(n) === null)) {
          popover = reflectedElement(n, "commandfor");
        }
      }
      const shown = popover === null ? null : popoverShown(popover);
      if (shown !== null && (shown[0] === "auto" || shown[0] === "hint")) return popover;
    }
    return null;
  }

  function lightDismissDialogs(target, up, x, y) {
    const dialogs = topLayer("dialogs");
    if (dialogs.length === 0) return;
    const ancestor = nearestClickedDialog(target, x, y);
    if (!up) {
      dialogPressed = ancestor;
      return;
    }
    const same = ancestor === dialogPressed;
    dialogPressed = null;
    const topmost = dialogs[dialogs.length - 1];
    if (!same || ancestor === topmost || closedByState(topmost) !== "any") return;
    if (fireCancel(topmost, true)) closeDialog(topmost, null, null);
  }

  // HTML's "nearest clicked dialog": none for a press on a modal
  // dialog's backdrop, outside its box; else the nearest open dialog
  // around the target.
  function nearestClickedDialog(target, x, y) {
    if (isDialog(target) && hasOpen(target) && dialogShown(target) === "modal") {
      const box = native.geometry(target, true);
      if (box.length >= 4 && (x < box[0] || y < box[1] || x >= box[0] + box[2] ||
        y >= box[1] + box[3])) {
        return null;
      }
    }
    for (let n = target; n !== null; n = flatParent(n)) {
      if (isDialog(n) && hasOpen(n)) return n;
    }
    return null;
  }

  // The engine's change of interest (HTML section 6.14): `interest` or
  // `loseinterest` at `target`, cancelable, its source `source`; then
  // its state, and a popover target shown or hidden. Whether it was
  // canceled.
  function* interestTask(source, target, gained) {
    const event = new InterestEvent(gained ? "interest" : "loseinterest", made(INTEREST_INIT, [
      "cancelable", true,
      "composed", true,
      "source", wrap(source),
    ]));
    eventState(event).trusted = true;
    if (yield* dispatching(wrap(target), event)) return true;
    topLayer("interest", source, target, null, null, gained ? "gain" : "lose");
    if (popoverAttribute(target) !== null) {
      if (gained && popoverShown(target) === null) {
        showPopover(target, false, source, "");
      } else if (!gained && popoverShown(target) !== null &&
        topLayer("invoker", target) === source) {
        hidePopover(target, true, true, false, source, "");
      }
    }
    return false;
  }

  // ---- Script close watchers (HTML section 6.13.1) ---------------------

  // Each `CloseWatcher`'s state, by object, and the objects by number.
  const watcherStates = new $WeakMap();
  const scriptWatchers = new $Map();
  let lastWatcher = 0;

  function watcherOf(object) {
    const state = WeakMapGet(watcherStates, object);
    if (state === undefined) throw illegalInvocation();
    return state;
  }

  // Destroys a close watcher: no longer active, and the engine's
  // forgotten.
  function watcherDestroy(object) {
    const state = watcherOf(object);
    if (!state.active) return;
    state.active = false;
    MapDelete(scriptWatchers, state.id);
    topLayer("removeWatcher", null, null, null, null, $String(state.id));
  }

  // HTML's "close" a close watcher: destroyed, then `close` at it.
  function watcherClose(object) {
    const state = watcherOf(object);
    if (!state.active) return;
    watcherDestroy(object);
    const event = new Event("close", made(EVENT_INIT, []));
    eventState(event).trusted = true;
    drain(dispatching(object, event));
  }

  // HTML's "request to close": `cancel`, cancelable when `canPrevent`,
  // then its closing. Whether it closed (or was not active).
  function watcherRequestClose(object, canPrevent) {
    const state = watcherOf(object);
    if (!state.active || state.running) return true;
    state.running = true;
    let go;
    try {
      const event = new Event("cancel", made(EVENT_INIT, ["cancelable", canPrevent]));
      eventState(event).trusted = true;
      go = !drain(dispatching(object, event));
    } finally {
      state.running = false;
    }
    if (!go) return false;
    watcherClose(object);
    return true;
  }

  class CloseWatcher extends EventTarget {
    constructor(options = undefined) {
      super();
      const dict = options === undefined || options === null ? {} : options;
      const signal = dict.signal;
      const id = ++lastWatcher;
      WeakMapSet(watcherStates, this, { id, active: true, running: false });
      if (signal !== undefined && signal !== null) {
        const s = signalOf(signal);
        if (s.aborted) {
          WeakMapGet(watcherStates, this).active = false;
          return;
        }
        ArrayPush(s.algorithms, () => watcherDestroy(this));
      }
      MapSet(scriptWatchers, id, this);
      topLayer("addWatcher", null, null, null, null, $String(id));
    }

    requestClose() {
      watcherRequestClose(this, true);
    }

    close() {
      watcherClose(this);
    }

    destroy() {
      watcherDestroy(this);
    }
  }
  defineHandlers(CloseWatcher.prototype, ["cancel", "close"], false);
  expose(CloseWatcher, "CloseWatcher");

  // ---- The members (HTML sections 4.11.4, 6.12.1, 4.10.6, 6.14) ----------

  include(
    {
      get open() {
        return hasOpen(self(this));
      },

      set open(value) {
        const handle = self(this);
        if (value) {
          setAttribute(handle, "open", "", "open");
        } else {
          removeAttribute(handle, "open");
        }
      },

      get returnValue() {
        self(this);
        const value = WeakMapGet(returnValues, this);
        return value === undefined ? "" : value;
      },

      set returnValue(value) {
        self(this);
        WeakMapSet(returnValues, this, str(value));
      },

      get closedBy() {
        return closedByState(self(this));
      },

      set closedBy(value) {
        setAttribute(self(this), "closedby", str(value), "closedBy");
      },

      close(result = undefined) {
        closeDialog(self(this), result === undefined ? null : str(result), null);
      },

      requestClose(result = undefined) {
        requestCloseDialog(self(this), result === undefined ? null : str(result), null);
      },

      show() {
        showDialog(self(this));
      },

      showModal() {
        showModal(self(this), null);
      },
    },
    [madeInterfaces.HTMLDialogElement.prototype],
  );

  // HTMLDetailsElement (HTML section 4.11.1, gap batch H): `open` and
  // `name` reflected.
  include(
    {
      get open() {
        return native.attribute(self(this), "open") !== null;
      },

      set open(value) {
        const handle = self(this);
        if (value) {
          setAttribute(handle, "open", "", "open");
        } else {
          removeAttribute(handle, "open");
        }
      },

      get name() {
        const value = native.attribute(self(this), "name");
        return value === null ? "" : value;
      },

      set name(value) {
        setAttribute(self(this), "name", str(value), "name");
      },
    },
    [madeInterfaces.HTMLDetailsElement.prototype],
  );

  // HTMLSelectElement's `showPicker()` (HTML section 4.10.7, gap batch
  // H): a drop-down not disabled, with the user's activation, has its
  // list shown once the script is done.
  include(
    {
      showPicker() {
        const handle = self(this);
        const error = topLayer("showPicker", handle);
        if (error === "InvalidStateError") {
          throw domError("InvalidStateError", "Select cannot be used if it is disabled.",
            "showPicker", "HTMLSelectElement");
        }
        if (error === "NotAllowedError") {
          throw domError("NotAllowedError", "HTMLSelectElement::showPicker() requires a user gesture.",
            "showPicker", "HTMLSelectElement");
        }
      },
    },
    [madeInterfaces.HTMLSelectElement.prototype],
  );

  // A `showPopover()` or `togglePopover()` option's `source`, as an
  // element's handle, or null.
  function sourceOption(options, method) {
    if (options === undefined || options === null || typeof options !== "object") return null;
    const source = options.source;
    if (source === undefined) return null;
    // Not nullable: null is no HTMLElement (WebIDL section 3.2.10).
    if (source === null || !isNode(source) || kindOf(source) !== 1) {
      throw new $TypeError(prefix(method, "HTMLElement") +
        "Failed to read the 'source' property from '" +
        (method === "showPopover" ? "ShowPopoverOptions" : "TogglePopoverOptions") +
        "': Failed to convert value to 'HTMLElement'.");
    }
    return handleOf(source);
  }

  include(
    {
      get popover() {
        return popoverAttribute(self(this));
      },

      set popover(value) {
        const handle = self(this);
        // A nullable DOMString: undefined is null (WebIDL section 3.2.24).
        if (value === null || value === undefined) {
          removeAttribute(handle, "popover");
        } else {
          setAttribute(handle, "popover", str(value), "popover");
        }
      },

      showPopover(options = undefined) {
        const handle = self(this);
        showPopover(handle, true, sourceOption(options, "showPopover"), "showPopover");
      },

      hidePopover() {
        hidePopover(self(this), true, true, true, null, "hidePopover");
      },

      togglePopover(options = undefined) {
        const handle = self(this);
        let force = null;
        if (typeof options === "boolean") {
          force = options;
        } else if (options !== undefined && options !== null && typeof options === "object" &&
          options.force !== undefined) {
          force = !!options.force;
        }
        const source = typeof options === "boolean" ? null : sourceOption(options, "togglePopover");
        return togglePopover(handle, force, true, source, "togglePopover");
      },
    },
    [HTMLElement.prototype],
  );

  // `popoverTargetElement` and `popoverTargetAction` (HTML section
  // 6.12.3), of a button and an input.
  include(
    {
      get popoverTargetElement() {
        const found = associatedElements(self(this), "popovertarget");
        return found.length === 0 ? null : wrap(found[0]);
      },

      set popoverTargetElement(value) {
        setAttrElement(self(this), "popovertarget", value, "popoverTargetElement",
          nameOfInterface(this));
      },

      get popoverTargetAction() {
        return popoverTargetAction(self(this));
      },

      set popoverTargetAction(value) {
        setAttribute(self(this), "popovertargetaction", str(value), "popoverTargetAction");
      },
    },
    [madeInterfaces.HTMLButtonElement.prototype, madeInterfaces.HTMLInputElement.prototype],
  );

  // `commandForElement` and `command` (HTML section 4.10.6), a button's.
  include(
    {
      get commandForElement() {
        const found = associatedElements(self(this), "commandfor");
        return found.length === 0 ? null : wrap(found[0]);
      },

      set commandForElement(value) {
        setAttrElement(self(this), "commandfor", value, "commandForElement",
          "HTMLButtonElement");
      },

      get command() {
        return commandOf(self(this));
      },

      set command(value) {
        setAttribute(self(this), "command", str(value), "command");
      },
    },
    [madeInterfaces.HTMLButtonElement.prototype],
  );

  // `interestForElement` (HTML section 6.14), an interest invoker's.
  include(
    {
      get interestForElement() {
        const found = associatedElements(self(this), "interestfor");
        return found.length === 0 ? null : wrap(found[0]);
      },

      set interestForElement(value) {
        setAttrElement(self(this), "interestfor", value, "interestForElement",
          nameOfInterface(this));
      },
    },
    [
      madeInterfaces.HTMLButtonElement.prototype,
      madeInterfaces.HTMLAnchorElement.prototype,
      madeInterfaces.HTMLAreaElement.prototype,
      madeInterfaces.SVGAElement.prototype,
    ],
  );

  // ARIA's element references (ARIA 1.3 section 6.1, as the reference
  // browser reflects them): one element, or a list, each `null` with no
  // attribute and none a script set.
  const ARIA_ELEMENT = [["ariaActiveDescendantElement", "aria-activedescendant"]];
  const ARIA_ELEMENTS = [
    ["ariaControlsElements", "aria-controls"],
    ["ariaDescribedByElements", "aria-describedby"],
    ["ariaDetailsElements", "aria-details"],
    ["ariaErrorMessageElements", "aria-errormessage"],
    ["ariaFlowToElements", "aria-flowto"],
    ["ariaLabelledByElements", "aria-labelledby"],
  ];
  // Whether the attribute is there, or a script set elements for it.
  function referenceSet(handle, attribute) {
    return native.attribute(handle, attribute) !== null;
  }
  for (let i = 0; i < ARIA_ELEMENT.length; i++) {
    const [property, attribute] = ARIA_ELEMENT[i];
    defineProperty(Element.prototype, property, {
      get() {
        const found = associatedElements(self(this), attribute);
        return found.length === 0 ? null : wrap(found[0]);
      },
      set(value) {
        setAttrElement(self(this), attribute, value === undefined ? null : value, property,
          "Element");
      },
      enumerable: true,
      configurable: true,
    });
  }
  for (let i = 0; i < ARIA_ELEMENTS.length; i++) {
    const [property, attribute] = ARIA_ELEMENTS[i];
    defineProperty(Element.prototype, property, {
      get() {
        const handle = self(this);
        if (!referenceSet(handle, attribute)) return null;
        const found = associatedElements(handle, attribute);
        const out = [];
        for (let j = 0; j < found.length; j++) ArrayPush(out, wrap(found[j]));
        return out;
      },
      set(value) {
        setAttrElements(self(this), attribute, value, property);
      },
      enumerable: true,
      configurable: true,
    });
  }

  // An element's interface's name, as an exception's words give it.
  function nameOfInterface(object) {
    const tag = native.tag(self(object));
    if (tag === "input") return "HTMLInputElement";
    if (tag === "a") return isHtmlElement(self(object)) ? "HTMLAnchorElement" : "SVGAElement";
    if (tag === "area") return "HTMLAreaElement";
    return "HTMLButtonElement";
  }

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

  // ---- CSSOM View: what scripts ask of the layout (D65) -----------------

  // An element's geometry as `native.geometry` answers: bounds x, y,
  // width, height; border top, right, bottom, left; padding; margins;
  // scroll height; then each box's x, y, width, height. Empty with no box.
  const G_BORDER = 4;
  const G_PADDING = 8;
  const G_MARGIN = 12;
  const G_SCROLL_HEIGHT = 16;
  // The first box's corner with no transform applied (CSSOM View
  // section 7's offsets), then each box's bounds.
  const G_ORIGIN = 17;
  const G_RECTS = 19;

  function geometryOf(element, scrolled) {
    return native.geometry(self(element), scrolled);
  }

  class DOMRectList {
    #rects;

    constructor(key, rects) {
      if (key !== KEY) {
        throw illegalConstructor("DOMRectList");
      }
      this.#rects = rects;
      for (let i = 0; i < rects.length; i++) {
        defineProperty(this, $String(i), { value: rects[i], enumerable: true });
      }
    }

    get length() {
      return this.#rects.length;
    }

    item(index) {
      need(arguments.length, 1, "item", "DOMRectList");
      const i = unsigned(index);
      return i < this.#rects.length ? this.#rects[i] : null;
    }
  }
  defineProperty(DOMRectList.prototype, iteratorSymbol, {
    value: ArrayValues,
    writable: true,
    configurable: true,
  });
  expose(DOMRectList, "DOMRectList");

  // Whether `element` lays out as an inline box, which has no client
  // area (CSSOM View section 6): an inline element, an inline list item
  // and a ruby container or its parts (D77).
  function inlineBox(element) {
    switch (native.computedValue(self(element), "", "display")) {
      case "inline":
      case "inline list-item":
      case "ruby":
      case "ruby-base":
      case "ruby-text":
      case "ruby-base-container":
      case "ruby-text-container":
        return true;
      default:
        return false;
    }
  }

  // Whether `element` is the document's element, which stands for the
  // viewport in the scroll members (the scrolling element).
  function scrollingRoot(element) {
    return handleOf(element) === documentElement(native.document());
  }

  // A scroll offset asked for as CSSOM View normalizes it: not finite
  // is 0.
  function finiteOrZero(value) {
    const n = $Number(value);
    return n === n && n !== Infinity && n !== -Infinity ? n : 0;
  }

  // `scrollTo()`'s and the like's arguments (CSSOM View sections 5 and
  // 7): x and y, or a `ScrollToOptions` dictionary; as [x, y, behavior],
  // NaN for an axis not given. Throws a TypeError for one argument that
  // is not a dictionary, or a behavior that is none.
  function scrollOptions(args) {
    let x = NaN;
    let y = NaN;
    let behavior = "auto";
    if (args.length === 1) {
      const options = args[0];
      if (options !== null && options !== undefined) {
        if (typeof options !== "object" && typeof options !== "function") {
          throw new $TypeError("The provided value is not of type 'ScrollToOptions'.");
        }
        if (options.left !== undefined) {
          x = finiteOrZero(options.left);
        }
        if (options.top !== undefined) {
          y = finiteOrZero(options.top);
        }
        if (options.behavior !== undefined) {
          behavior = str(options.behavior);
          if (behavior !== "auto" && behavior !== "instant" && behavior !== "smooth") {
            throw new $TypeError(
              "The provided value '" + behavior + "' is not a valid enum value of type ScrollBehavior.",
            );
          }
        }
      }
    } else if (args.length >= 2) {
      x = finiteOrZero(args[0]);
      y = finiteOrZero(args[1]);
    }
    return [x, y, behavior];
  }

  // Whether a scroll of `element`'s box (the viewport's for the root)
  // with `behavior` is smooth: asked, or `auto` with the box's
  // `scroll-behavior` smooth.
  function smoothScroll(element, behavior) {
    if (behavior !== "auto") {
      return behavior === "smooth";
    }
    const handle = element === null ? documentElement(native.document()) : handleOf(element);
    return handle !== null && native.computedValue(handle, "", "scroll-behavior") === "smooth";
  }

  // ---- The scroll methods' promises (as the reference browser has
  // them: CSSOM View's scroll methods return a promise) ----
  // Each scroll a method made, until it settles, by the scrollers it
  // moves: the viewport (-1) or an element's handle. A scroll settles
  // with `{ interrupted }`: false once it is where it was going (a
  // smooth one over the frames to come), true once another scroll of a
  // scroller it moves, or the scroller's leaving the document, cut it
  // short.
  const scrollsUnderWay = new $Map();
  let scrollsWatched = false;

  function scrollerKey(target) {
    return target === null ? -1 : target;
  }

  // The scrolls under way of the scroller `target` settle, cut short: a
  // new scroll of it is made.
  function interruptScrolls(target) {
    const list = MapGet(scrollsUnderWay, scrollerKey(target));
    if (list === undefined) return;
    MapDelete(scrollsUnderWay, scrollerKey(target));
    for (let i = 0; i < list.length; i++) settleScroll(list[i], true);
  }

  // Scroll `scroll` settles, `interrupted` or not, and is forgotten.
  function settleScroll(scroll, interrupted) {
    if (scroll.settled) return;
    scroll.settled = true;
    for (let i = 0; i < scroll.targets.length; i++) {
      const key = scrollerKey(scroll.targets[i]);
      const list = MapGet(scrollsUnderWay, key);
      if (list === undefined) continue;
      const kept = [];
      for (let j = 0; j < list.length; j++) {
        if (list[j] !== scroll) ArrayPush(kept, list[j]);
      }
      if (kept.length === 0) MapDelete(scrollsUnderWay, key);
      else MapSet(scrollsUnderWay, key, kept);
    }
    scroll.resolve({ interrupted });
  }

  // A promise of a scroll of the scrollers `targets` under way, settled
  // once `arrived()` says it is where it was going, checked at each
  // frame.
  function scrollPromise(targets, arrived) {
    let resolve;
    const promise = new $Promise((r) => {
      resolve = r;
    });
    const scroll = { __proto__: null, targets, arrived, resolve, settled: false };
    for (let i = 0; i < targets.length; i++) {
      const key = scrollerKey(targets[i]);
      const list = MapGet(scrollsUnderWay, key);
      if (list === undefined) MapSet(scrollsUnderWay, key, [scroll]);
      else ArrayPush(list, scroll);
    }
    watchScrolls();
    return promise;
  }

  // The scrolls under way checked at the next frame, and the frames after
  // while some are.
  function watchScrolls() {
    if (scrollsWatched) return;
    scrollsWatched = true;
    frameFunctions.requestAnimationFrame(() => {
      scrollsWatched = false;
      const all = [];
      MapForEach(scrollsUnderWay, (list) => {
        for (let i = 0; i < list.length; i++) {
          if (ArrayIndexOf(all, list[i]) < 0) ArrayPush(all, list[i]);
        }
      });
      for (let i = 0; i < all.length; i++) {
        const scroll = all[i];
        let gone = false;
        for (let j = 0; j < scroll.targets.length; j++) {
          const target = scroll.targets[j];
          if (target !== null && !native.connected(target)) gone = true;
        }
        if (gone) settleScroll(scroll, true);
        else if (scroll.arrived()) settleScroll(scroll, false);
      }
      let left = false;
      MapForEach(scrollsUnderWay, () => {
        left = true;
      });
      if (left) watchScrolls();
    });
  }

  // A scroll of the viewport (`target` null) or an element by the
  // arguments `args` of `method`, by them or to them: a promise settled
  // once it is where it goes, or cut short; rejected for arguments it
  // cannot take.
  function scrollWith(element, target, args, relative) {
    let options;
    try {
      options = scrollOptions(args);
    } catch (error) {
      return PromiseReject(error);
    }
    let [x, y] = options;
    const now = native.scrollOffset(target);
    if (relative) {
      x = x === x ? now[2] + x : NaN;
      y = y === y ? now[0] + y : NaN;
    }
    interruptScrolls(target);
    const smooth = smoothScroll(element, options[2]);
    native.scrollTo(target, x, y, smooth);
    if (!smooth) return PromiseResolve({ interrupted: false });
    // Where it goes, as far as the scroller can.
    const clamp = (v, max) => (v < 0 ? 0 : v > max ? max : v);
    const toY = y === y ? MathRound(clamp(y, now[1])) : NaN;
    const toX = x === x ? MathRound(clamp(x, now[3])) : NaN;
    return scrollPromise([target], () => {
      const at = native.scrollOffset(target);
      return (toY !== toY || MathRound(at[0]) === toY) && (toX !== toX || MathRound(at[2]) === toX);
    });
  }

  // The element's parent in the flat tree (D68): its slot when it is
  // assigned one, its shadow root's host for a child of one, else its
  // parent; `null` at the top.
  function flatParent(handle) {
    const slot = native.assignedSlot(handle);
    if (slot !== null) {
      return slot;
    }
    const parent = native.parent(handle);
    if (parent !== null && native.kind(parent) === 11) {
      return native.shadowHost(parent);
    }
    return parent;
  }

  include(
    {
      // CSSOM View's checkVisibility(): false when the element has no
      // box; with `checkOpacity` (or `opacityProperty`), when it or an
      // ancestor in the flat tree has an opacity of 0; with
      // `checkVisibilityCSS` (or `visibilityProperty`), when its own
      // `visibility` is not `visible`.
      checkVisibility(options) {
        const o = options === undefined || options === null ? {} : options;
        if (geometryOf(this, false).length === 0) {
          return false;
        }
        if (o.checkOpacity || o.opacityProperty) {
          for (let at = self(this); at !== null && native.kind(at) === 1; at = flatParent(at)) {
            if (Number(native.computedValue(at, "", "opacity")) === 0) {
              return false;
            }
          }
        }
        if (o.checkVisibilityCSS || o.visibilityProperty) {
          return native.computedValue(self(this), "", "visibility") === "visible";
        }
        return true;
      },

      getBoundingClientRect() {
        const g = geometryOf(this, true);
        return g.length === 0 ? new DOMRect(0, 0, 0, 0) : new DOMRect(g[0], g[1], g[2], g[3]);
      },

      getClientRects() {
        const g = geometryOf(this, true);
        const rects = [];
        for (let i = G_RECTS; i + 3 < g.length; i += 4) {
          ArrayPush(rects, new DOMRect(g[i], g[i + 1], g[i + 2], g[i + 3]));
        }
        return new DOMRectList(KEY, rects);
      },

      get clientTop() {
        const g = geometryOf(this, false);
        return g.length === 0 || inlineBox(this) ? 0 : MathRound(g[G_BORDER]);
      },

      get clientLeft() {
        const g = geometryOf(this, false);
        return g.length === 0 || inlineBox(this) ? 0 : MathRound(g[G_BORDER + 3]);
      },

      get clientWidth() {
        if (scrollingRoot(this)) {
          return native.viewport()[0];
        }
        const g = geometryOf(this, false);
        if (g.length === 0 || inlineBox(this)) {
          return 0;
        }
        return MathRound(g[2] - g[G_BORDER + 1] - g[G_BORDER + 3]);
      },

      get clientHeight() {
        if (scrollingRoot(this)) {
          return native.viewport()[1];
        }
        const g = geometryOf(this, false);
        if (g.length === 0 || inlineBox(this)) {
          return 0;
        }
        return MathRound(g[3] - g[G_BORDER] - g[G_BORDER + 2]);
      },

      get scrollTop() {
        const handle = self(this);
        return native.scrollOffset(scrollingRoot(this) ? null : handle)[0];
      },

      set scrollTop(value) {
        const handle = self(this);
        interruptScrolls(scrollingRoot(this) ? null : handle);
        native.scrollTo(
          scrollingRoot(this) ? null : handle,
          NaN,
          finiteOrZero(value),
          smoothScroll(this, "auto"),
        );
      },

      get scrollLeft() {
        const handle = self(this);
        return native.scrollOffset(scrollingRoot(this) ? null : handle)[2];
      },

      set scrollLeft(value) {
        const handle = self(this);
        interruptScrolls(scrollingRoot(this) ? null : handle);
        native.scrollTo(
          scrollingRoot(this) ? null : handle,
          finiteOrZero(value),
          NaN,
          smoothScroll(this, "auto"),
        );
      },

      // The client width and how far the content reaches past it,
      // whether the box scrolls or not.
      get scrollWidth() {
        const handle = self(this);
        const target = scrollingRoot(this) ? null : handle;
        return this.clientWidth + MathRound(native.scrollOffset(target)[5]);
      },

      get scrollHeight() {
        const handle = self(this);
        if (scrollingRoot(this)) {
          return MathRound(native.scrollOffset(null)[1] + native.viewport()[1]);
        }
        const g = geometryOf(this, false);
        if (g.length === 0) {
          return 0;
        }
        const own = g[G_SCROLL_HEIGHT];
        if (own > 0) {
          return MathRound(own);
        }
        return this.clientHeight + MathRound(native.scrollOffset(handle)[4]);
      },

      scrollTo(...args) {
        const handle = self(this);
        return scrollWith(this, scrollingRoot(this) ? null : handle, args, false);
      },

      scroll(...args) {
        const handle = self(this);
        return scrollWith(this, scrollingRoot(this) ? null : handle, args, false);
      },

      scrollBy(...args) {
        const handle = self(this);
        return scrollWith(this, scrollingRoot(this) ? null : handle, args, true);
      },

      // A boolean or a `ScrollIntoViewOptions` (CSSOM View section 7):
      // the block start (end for false), the nearest inline place.
      scrollIntoView(arg) {
        const handle = self(this);
        let block = "start";
        let inline = "nearest";
        let behavior = "auto";
        if (arg === false) {
          block = "end";
        } else if (arg !== null && typeof arg === "object") {
          const pick = (value, fallback, names) => {
            if (value === undefined) return fallback;
            const text = str(value);
            if (ArrayIndexOf(names, text) < 0) {
              throw new $TypeError("The provided value '" + text + "' is not a valid enum value.");
            }
            return text;
          };
          const places = ["start", "center", "end", "nearest"];
          block = pick(arg.block, "start", places);
          inline = pick(arg.inline, "nearest", places);
          behavior = pick(arg.behavior, "auto", ["auto", "instant", "smooth"]);
        }
        // Every scroller around it may move: their scrolls under way are
        // cut short, and its own settles once it stays where it is.
        const scrollers = [null];
        for (let a = native.parent(handle); a !== null; a = native.parent(a)) {
          if (native.kind(a) === 1) ArrayPush(scrollers, a);
        }
        for (let i = 0; i < scrollers.length; i++) interruptScrolls(scrollers[i]);
        native.scrollIntoView(handle, block, inline, behavior);
        if (behavior === "instant") return PromiseResolve({ interrupted: false });
        let last = null;
        return scrollPromise(scrollers, () => {
          const g = native.geometry(handle, true);
          const now = g.length === 0 ? "" : g[0] + "," + g[1];
          const still = now === last && native.scrollOffset(null)[6] === 0;
          last = now;
          return still;
        });
      },
    },
    [Element.prototype],
  );

  include(
    {
      get offsetParent() {
        return wrap(native.offsetParent(self(this)));
      },

      get offsetTop() {
        return MathRound(offsetOf(this)[1]);
      },

      get offsetLeft() {
        return MathRound(offsetOf(this)[0]);
      },

      get offsetWidth() {
        const g = geometryOf(this, false);
        return g.length === 0 ? 0 : MathRound(g[2]);
      },

      get offsetHeight() {
        const g = geometryOf(this, false);
        return g.length === 0 ? 0 : MathRound(g[3]);
      },

      get innerText() {
        return native.innerText(self(this));
      },

      set innerText(value) {
        const handle = self(this);
        setRenderedText(handle, strOrEmpty(value), "innerText");
      },

      get outerText() {
        return native.innerText(self(this));
      },

      set outerText(value) {
        const handle = self(this);
        const parent = native.parent(handle);
        if (parent === null) {
          throw domError(
            "NoModificationAllowedError",
            "The element has no parent.",
            "outerText",
            "HTMLElement",
          );
        }
        // Its place taken by the text, lines parted by `br`s.
        const fragment = renderedFragment(strOrEmpty(value));
        native.mutationScope(true);
        try {
          native.insert(parent, fragment, handle);
          native.remove(parent, handle);
        } finally {
          native.mutationScope(false);
        }
      },
    },
    [HTMLElement.prototype],
  );

  // An element's offset from its offsetParent's padding edge, or from
  // the document's origin with none (CSSOM View section 7).
  function offsetOf(element) {
    const g = geometryOf(element, false);
    if (g.length === 0) {
      return [0, 0];
    }
    // Measured as laid out, "ignoring any transforms".
    const x = g[G_ORIGIN];
    const y = g[G_ORIGIN + 1];
    const parent = native.offsetParent(handleOf(element));
    if (parent === null || native.tag(parent) === "body") {
      return [x, y];
    }
    const p = native.geometry(parent, false);
    if (p.length === 0) {
      return [x, y];
    }
    // The reference browser takes a fieldset's border off twice for an
    // absolutely positioned box it holds.
    const position =
      native.tag(parent) === "fieldset"
        ? native.computedValue(handleOf(element), "", "position")
        : "";
    const twice = position === "absolute" || position === "fixed" ? 2 : 1;
    return [
      x - p[G_ORIGIN] - twice * p[G_BORDER + 3],
      y - p[G_ORIGIN + 1] - twice * p[G_BORDER],
    ];
  }

  // HTML's "rendered text fragment": `text` as text nodes, each line
  // break a `br`.
  function renderedFragment(text) {
    const fragment = native.createFragment();
    let at = 0;
    while (at <= text.length) {
      let end = at;
      while (end < text.length && text[end] !== "\n" && text[end] !== "\r") {
        end++;
      }
      if (end > at) {
        native.insert(fragment, native.createText(StringSlice(text, at, end)), null);
      }
      if (end >= text.length) {
        break;
      }
      native.insert(fragment, native.createElement("br"), null);
      at = end + (text[end] === "\r" && text[end + 1] === "\n" ? 2 : 1);
    }
    return fragment;
  }

  // The `innerText` setter: the children replaced by the text.
  function setRenderedText(handle, text, method) {
    try {
      const fragment = renderedFragment(text);
      native.mutationScope(true);
      try {
        for (let child = native.firstChild(handle); child !== null;
          child = native.firstChild(handle)) {
          native.remove(handle, child);
        }
        native.insert(handle, fragment, null);
      } finally {
        native.mutationScope(false);
      }
    } catch (error) {
      throw failed(error, method, "HTMLElement");
    }
  }

  include(
    {
      // An element in a shadow tree is its host here (D68).
      elementFromPoint(x, y) {
        const handle = self(this);
        need(arguments.length, 2, "elementFromPoint", "Document");
        const found = pointElements(handle, $Number(x), $Number(y), false);
        return found.length === 0 ? null : found[0];
      },

      elementsFromPoint(x, y) {
        const handle = self(this);
        need(arguments.length, 2, "elementsFromPoint", "Document");
        return pointElements(handle, $Number(x), $Number(y), true);
      },

      get scrollingElement() {
        return wrap(documentElement(self(this)));
      },
    },
    [Document.prototype],
  );

  // The window's scroll members (CSSOM View section 5): the viewport
  // scrolls down and across.
  const viewportScrollY = {
    get() {
      return native.scrollOffset(null)[0];
    },
    enumerable: true,
    configurable: true,
  };
  const viewportScrollX = {
    get() {
      return native.scrollOffset(null)[2];
    },
    enumerable: true,
    configurable: true,
  };
  windowOnly("scrollY", viewportScrollY);
  windowOnly("pageYOffset", viewportScrollY);
  windowOnly("scrollX", viewportScrollX);
  windowOnly("pageXOffset", viewportScrollX);
  const windowScrollTo = function scrollTo(...args) {
    return scrollWith(null, null, args, false);
  };
  windowOnly("scrollTo", { value: windowScrollTo, writable: true, enumerable: true, configurable: true });
  windowOnly("scroll", {
    value: function scroll(...args) {
      return scrollWith(null, null, args, false);
    },
    writable: true,
    enumerable: true,
    configurable: true,
  });
  windowOnly("scrollBy", {
    value: function scrollBy(...args) {
      return scrollWith(null, null, args, true);
    },
    writable: true,
    enumerable: true,
    configurable: true,
  });

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

  // `width` and `height` (HTML 4.8.4.4): the attribute's value, else the
  // image's natural size (an image not rendered, as `new Image()`'s;
  // a rendered one's box: G-list).
  {
    const proto = MapGet(interfaces, "img").prototype;
    const sides = [["width", 1], ["height", 2]];
    for (let i = 0; i < sides.length; i++) {
      const [name, index] = sides[i];
      const desc = getOwnPropertyDescriptor(proto, name);
      const reflected = desc.get;
      desc.get = function () {
        const handle = self(this);
        if (native.attribute(handle, name) !== null) return ReflectApply(reflected, this, []);
        return imageState(this)[index];
      };
      defineProperty(proto, name, desc);
    }
  }

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

  // The contexts of canvas elements, by the element's object, which a
  // context holds so that its element lives as long as it does.
  const canvasContexts = new $WeakMap();

  // Each ImageData's pixels, its size and its colour space.
  const imageDatas = new $WeakMap();

  // A PNG file's bytes, as the host gives them, a Blob.
  function pngBlob(buffer) {
    return new Blob(KEY, { bytes: new $Uint8Array(buffer), type: "image/png" });
  }

  // The rest of the 2D canvas, and OffscreenCanvas and ImageBitmap, are a
  // chunk made at its first use (D64): their interfaces are its accessors
  // until a page reaches them; `createImageBitmap` is the bootstrap's own,
  // which calls the chunk's.
  chunkInterfaces("canvas", [
    "TextMetrics", "CanvasRenderingContext2D", "CanvasGradient", "CanvasPattern", "Path2D",
    "ImageData", "OffscreenCanvas", "OffscreenCanvasRenderingContext2D", "ImageBitmap",
  ]);
  defineProperty(global, "createImageBitmap", {
    value: function createImageBitmap(image) {
      return ReflectApply(chunkInstall("canvas").createImageBitmap, this, arguments);
    },
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
        // A canvas has one context mode: a "2d" or a "webgpu" (M41).
        if (id === "webgpu") {
          if (existing !== undefined) {
            return gpuIsCanvasContext(existing) ? existing : null;
          }
          const made = chunkInstall("gpu").gpuCanvasContext(this, handle);
          if (made !== null) WeakMapSet(canvasContexts, this, made);
          return made;
        }
        if (id === "webgl" || id === "experimental-webgl" || id === "webgl2") {
          const version = id === "webgl2" ? 2 : 1;
          if (existing !== undefined) {
            return gpuWebglVersion(existing) === version ? existing : null;
          }
          const made = chunkInstall("gpu").webglContext(this, handle, version, arguments[1]);
          if (made !== null) WeakMapSet(canvasContexts, this, made);
          return made;
        }
        if (id !== "2d") {
          return null;
        }
        if (existing !== undefined) {
          return gpuIsCanvasContext(existing) || gpuWebglVersion(existing) !== 0 ? null : existing;
        }
        const context = chunkInstall("canvas").elementContext(this, handle, arguments[1]);
        WeakMapSet(canvasContexts, this, context);
        return context;
      },

      // A stream of the canvas's video (Media Capture from DOM Elements),
      // made by the audio chunk, which keeps the streams.
      captureStream() {
        self(this);
        return ReflectApply(chunkInstall("audio").captureStream, this, arguments);
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
        object = new (chunkInstall("canvas").ImageData)(KEY);
        const bytes = new $Uint8ClampedArray(TypedArrayLength(e[3]));
        TypedArraySet(bytes, e[3]);
        WeakMapSet(imageDatas, object, { data: bytes, width: e[1], height: e[2], colorSpace: e[4] });
        break;
      }
      case IMAGE_BITMAP:
        object = bitmapMade(e[1]);
        break;
      case IMAGE_BITMAP_PIXELS:
        object = chunkInstall("canvas").makeBitmap(native.canvasData("bitmapFrom", e[3], e[1], e[2], e[4] ? 1 : 0));
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
    return chunkInstall("canvas").makeBitmap(made);
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
  if (DEDICATED) {
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
  // So are Error.stackTraceLimit and Error.prepareStackTrace, of the
  // realm's own state: a call is logged with the originals' values set,
  // so that what the page put there runs nothing of its own, and the
  // previews' brand checks, which throw for most objects, build no stack.
  const stackLimit = getOwnPropertyDescriptor($Error, "stackTraceLimit");
  const prepareStack = getOwnPropertyDescriptor($Error, "prepareStackTrace");
  // The frames a call's stack is built with: its place, past the
  // bindings' own frames, or all of it for an error or a trace.
  const STACK_PLACE = 8;
  const STACK_WHOLE = 64;
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
  const $NumberIsFinite = Number.isFinite;

  // `text`, a string, as a JSON string: the original stringify, which
  // runs nothing of the page's for a string.
  function quote(text) {
    return JSONStringify(text);
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
  // A call whose previews or stack cannot be made, its heap spent, has
  // its text alone.
  function logger(level, kind, args, first) {
    let made = "";
    let stack = "";
    const limit = ReflectApply(stackLimit.get, $Error, []);
    const prepare = ReflectApply(prepareStack.get, $Error, []);
    try {
      ReflectApply(prepareStack.set, $Error, [undefined]);
      ReflectApply(stackLimit.set, $Error, [0]);
      made = previews(args, first);
      const whole = level === CONSOLE_ERROR || kind === "trace";
      ReflectApply(stackLimit.set, $Error, [whole ? STACK_WHOLE : STACK_PLACE]);
      stack = stackHere();
    } catch (e) {
      made = "";
    } finally {
      ReflectApply(stackLimit.set, $Error, [limit]);
      ReflectApply(prepareStack.set, $Error, [prepare]);
    }
    consoleMessage(level, kind, made, stack, args, first);
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

  // ---- IndexedDB and StorageManager, a chunk made at its first use (D64) ----
  // Their interfaces are the chunk's accessors until a page reaches them;
  // `indexedDB` and `navigator.storage` are the bootstrap's own getters,
  // which make the chunk.
  chunkInterfaces("idb", [
    "IDBKeyRange", "IDBVersionChangeEvent", "IDBRequest", "IDBOpenDBRequest", "IDBTransaction",
    "DOMStringList", "IDBDatabase", "IDBObjectStore", "IDBIndex", "IDBCursor", "IDBCursorWithValue",
    "IDBFactory",
  ]);
  windowOnly("indexedDB", {
    get() {
      return chunkInstall("idb").indexedDBObject;
    },
    enumerable: true,
    configurable: true,
  });
  chunkInterfaces("idb", ["StorageManager"]);

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
    const canvas = bindCanvasObject(ObjectCreate(chunkInstall("canvas").OffscreenCanvas.prototype), 1, made[0],
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
    // A string the policies refuse to compile (M33.1): nothing runs,
    // and the id is 0, as the reference browser gives it.
    if (typeof callback === "string" && native.codeAllowed("eval", callback, -1) !== "") {
      return 0;
    }
    // A string runs as the script that set it, which its `import()`s
    // resolve against (HTML's timer initialization steps).
    const script = typeof callback === "string" ? native.activeScript() : "";
    let id = lastTimer;
    do {
      id = id >= 2147483647 ? 1 : id + 1;
    } while (MapHas(timers, id));
    lastTimer = id;
    const delay = long(timeout);
    native.setTimer(id, delay);
    MapSet(timers, id, { callback, args, repeat, delay, script });
    return id;
  }

  // A task of the bindings' own (as File API's reads queue theirs): a
  // timer, of no delay unless `delay` milliseconds are given, its id past
  // those of scripts' timers, which `cancelTask` takes.
  let lastTask = 2147483647;
  function queueTask(work, delay = 0, stepped = false) {
    lastTask = lastTask >= 4294967295 ? 2147483648 : lastTask + 1;
    MapSet(timers, lastTask, { callback: work, args: [], repeat: false, delay, stepped });
    native.setTimer(lastTask, delay);
    return lastTask;
  }

  function cancelTask(id) {
    if (MapHas(timers, id)) {
      MapDelete(timers, id);
      native.clearTimer(id);
    }
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
  // run, unless the callback cleared it. The id comes as a 32-bit
  // integer, which a task's (past 2^31) is not until made unsigned.
  // A timer's task, a stepped entry (M26): a script's callback or string,
  // or the bindings' own task, which as a generator may dispatch events
  // the way the engine does, the microtask checkpoint run between their
  // listeners (gap batch A).
  function* runTimer(given) {
    const id = given >>> 0;
    const timer = MapGet(timers, id);
    if (timer === undefined) {
      return false;
    }
    if (!timer.repeat) {
      MapDelete(timers, id);
    }
    try {
      if (timer.stepped) {
        yield* timer.callback();
      } else if (typeof timer.callback === "function") {
        ReflectApply(timer.callback, global, timer.args);
      } else {
        const url = timer.script !== "" ? timer.script : native.documentUrl();
        native.evaluate(timer.callback, url === null ? "" : url);
      }
    } catch (error) {
      reportException(error);
    } finally {
      if (timer.repeat && MapGet(timers, id) === timer) {
        native.setTimer(id, timer.delay);
      }
    }
    return false;
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

  // ---- fonts (CSS Font Loading 3, gap batch D, D54) ----------------------

  // Each FontFace's state: its descriptors as strings, its status, its
  // sources and data once had, the sets it is in, and its `loaded`
  // promise with what settles it. A CSS-connected face (one of the
  // document's `@font-face` rules) knows its rule's number, `css`.
  const ArrayJoin = uncurry(Array.prototype.join);
  const fontFaces = new $WeakMap();
  let nextFontFace = 1;

  function faceOf(face) {
    const state = WeakMapGet(fontFaces, face);
    if (state === undefined) throw illegalInvocation();
    return state;
  }

  // A descriptor's value as an `@font-face` rule would have it written
  // back, or null where it does not parse.
  function descriptorText(name, value) {
    const text = native.fonts("descriptor " + name + " " + value, null);
    return text === undefined ? null : text;
  }

  // The face's status set, its promise settled, and its sets told.
  function settleFace(face, state, status, error) {
    if (state.status === "loaded" || state.status === "error") return;
    state.status = status;
    if (status === "loaded") {
      state.resolve(face);
    } else if (status === "error") {
      state.reject(error || new $DOMException("A network error occurred.", "NetworkError"));
    }
    for (let i = 0; i < state.sets.length; i++) {
      const set = state.sets[i];
      const s = setOf(set);
      if (status === "loaded") ArrayPush(s.loaded, face);
      if (status === "error") ArrayPush(s.failed, face);
      queueTask(() => fontsChanged(set, true));
    }
  }

  // The face's data had: a font, loaded; else an error.
  function faceData(face, state, bytes) {
    if (bytes !== null && native.fonts("valid", binaryString(bytes))) {
      state.bytes = bytes;
      for (let i = 0; i < state.sets.length; i++) addFace(face, state);
      settleFace(face, state, "loaded");
    } else {
      settleFace(face, state, "error");
    }
  }

  // A FontFace's url fetched as a font is (CSS Font Loading 3 section
  // 2.2: the font destination, which `font-src` rules, in cors mode):
  // its bytes, or null.
  function fetchFont(url) {
    return new $Promise((resolve) => {
      const id = nextFetch++;
      try {
        native.fetch(
          id, "GET", url, [], null, "cors", "same-origin", "follow", false, "default",
          "about:client", "", "", "", "font",
        );
      } catch (e) {
        resolve(null);
        return;
      }
      MapSet(fetches, id, {
        deliver(answer) {
          const ok = answer !== null && answer.body !== null && answer.status >= 200 &&
            answer.status < 300;
          resolve(ok ? new $Uint8Array(answer.body) : null);
        },
      });
    });
  }

  // Tries the face's sources from `at` in order: a system face by its
  // name, then each url fetched; the first that is a font is the face's.
  function trySources(face, state, at) {
    if (at >= state.sources.length) {
      faceData(face, state, null);
      return;
    }
    const kind = state.sources[at];
    const value = state.sources[at + 1];
    if (kind === "local") {
      const data = native.fonts("local " + value, null);
      if (data !== undefined && native.fonts("valid", binaryString(new $Uint8Array(data)))) {
        faceData(face, state, new $Uint8Array(data));
      } else {
        trySources(face, state, at + 2);
      }
      return;
    }
    const url = native.resolveUrl(value);
    if (url === null) {
      trySources(face, state, at + 2);
      return;
    }
    PromiseThen(fetchFont(url), (bytes) => {
      if (bytes !== null && native.fonts("valid", binaryString(bytes))) {
        faceData(face, state, bytes);
      } else {
        trySources(face, state, at + 2);
      }
    });
  }

  // A descriptor's range as the natives take it: two numbers.
  function faceRange(text, keywords, fallback) {
    const words = [];
    let word = "";
    for (let i = 0; i <= text.length; i++) {
      if (i === text.length || text[i] === " ") {
        if (word !== "") ArrayPush(words, word);
        word = "";
      } else {
        word += text[i];
      }
    }
    const one = (w) => {
      const lower = ToLowerCase(w);
      if (keywords[lower] !== undefined) return keywords[lower];
      const n = $Number(StringEndsWith(w, "%") ? StringSlice(w, 0, w.length - 1) : w);
      return n === n ? n : fallback;
    };
    if (words.length === 0) return fallback + " " + fallback;
    return one(words[0]) + " " + one(words.length > 1 ? words[1] : words[0]);
  }

  const WEIGHTS = { __proto__: null, normal: 400, bold: 700 };
  const STRETCHES = {
    __proto__: null, "ultra-condensed": 50, "extra-condensed": 62.5, condensed: 75,
    "semi-condensed": 87.5, normal: 100, "semi-expanded": 112.5, expanded: 125,
    "extra-expanded": 150, "ultra-expanded": 200,
  };

  // A unicode-range as the natives take it: `-` for every code point,
  // else each range's first and last in hex, parted by commas.
  function faceRanges(text) {
    const out = [];
    const parts = StringSplit(text, ",");
    for (let i = 0; i < parts.length; i++) {
      const part = StringTrim(parts[i]);
      if (StringSlice(part, 0, 2) !== "U+") continue;
      const range = StringSlice(part, 2);
      const dash = StringIndexOf(range, "-");
      const lo = dash < 0 ? range : StringSlice(range, 0, dash);
      const hi = dash < 0 ? range : StringSlice(range, dash + 1);
      if (lo === "0" && hi === "10FFFF") return "-";
      ArrayPush(out, lo + "-" + hi);
    }
    return out.length === 0 ? "-" : ArrayJoin(out, ",");
  }

  // Gives the document face `face`, whose data is had.
  function addFace(face, state) {
    if (state.css >= 0 || state.bytes === null) return;
    const style = ToLowerCase(state.style);
    const slant = StringIncludes(style, "italic") ? "italic" : StringIncludes(style, "oblique") ? "oblique" : "normal";
    native.fonts(
      "add " + state.id + " " + faceRange(state.weight, WEIGHTS, 400) + " " + slant + " " +
        faceRange(state.stretch, STRETCHES, 100) + " " + faceRanges(state.unicodeRange) + " " +
        state.family,
      binaryString(state.bytes),
    );
  }

  // A face's state, its descriptors given as text, none checked yet.
  function newFaceState(family, d) {
    const state = {
      id: nextFontFace++,
      family,
      style: d.style,
      weight: d.weight,
      stretch: d.stretch,
      unicodeRange: d.unicodeRange,
      featureSettings: d.featureSettings,
      display: d.display,
      status: "unloaded",
      css: -1,
      // The face sets it is in.
      sets: [],
      bytes: null,
      sources: [],
      resolve: null,
      reject: null,
      loaded: null,
    };
    state.loaded = new $Promise((resolve, reject) => {
      state.resolve = resolve;
      state.reject = reject;
    });
    // A face that fails with nobody listening is not reported.
    PromiseThen(state.loaded, undefined, () => {});
    return state;
  }

  // The descriptors FontFace's attributes are, by their dictionary's
  // names, with their initial values.
  const FACE_DESCRIPTORS = [
    ["style", "font-style", "normal"],
    ["weight", "font-weight", "normal"],
    ["stretch", "font-stretch", "normal"],
    ["unicodeRange", "unicode-range", "U+0-10FFFF"],
    ["featureSettings", "font-feature-settings", "normal"],
    ["display", "font-display", "auto"],
  ];

  class FontFace {
    constructor(family, source, descriptors) {
      needToConstruct(arguments.length, 2, "FontFace");
      const d = descriptors === undefined || descriptors === null ? {} : descriptors;
      const given = { __proto__: null };
      let bad = false;
      for (let i = 0; i < FACE_DESCRIPTORS.length; i++) {
        const entry = FACE_DESCRIPTORS[i];
        const value = d[entry[0]] === undefined ? entry[2] : $String(d[entry[0]]);
        const text = descriptorText(entry[1], value);
        if (text === null) bad = true;
        given[entry[0]] = text === null ? "" : text;
      }
      const state = newFaceState($String(family), given);
      WeakMapSet(fontFaces, this, state);
      const syntax = () =>
        new $DOMException("Failed to construct 'FontFace': The source provided could not be parsed as a value list.", "SyntaxError");
      if (bad) {
        state.status = "error";
        state.reject(syntax());
        return;
      }
      const bytes = bufferBytes(source);
      if (bytes !== undefined) {
        state.status = "loading";
        faceData(this, state, bytes);
        return;
      }
      const text = $String(source);
      if (descriptorText("src", text) === null) {
        state.status = "error";
        state.reject(syntax());
        return;
      }
      state.sources = native.fonts("sources " + text, null);
    }

    get family() { return faceOf(this).family; }
    set family(value) { faceOf(this).family = $String(value); }
    get style() { return faceOf(this).style; }
    set style(value) { setDescriptor(this, "style", "font-style", value); }
    get weight() { return faceOf(this).weight; }
    set weight(value) { setDescriptor(this, "weight", "font-weight", value); }
    get stretch() { return faceOf(this).stretch; }
    set stretch(value) { setDescriptor(this, "stretch", "font-stretch", value); }
    get unicodeRange() { return faceOf(this).unicodeRange; }
    set unicodeRange(value) { setDescriptor(this, "unicodeRange", "unicode-range", value); }
    get featureSettings() { return faceOf(this).featureSettings; }
    set featureSettings(value) { setDescriptor(this, "featureSettings", "font-feature-settings", value); }
    get display() { return faceOf(this).display; }
    set display(value) { setDescriptor(this, "display", "font-display", value); }
    get status() { return faceOf(this).status; }
    get loaded() { return faceOf(this).loaded; }

    load() {
      const state = faceOf(this);
      if (state.status !== "unloaded") return state.loaded;
      state.status = "loading";
      for (let i = 0; i < state.sets.length; i++) fontsChanged(state.sets[i]);
      if (state.css >= 0) {
        native.fonts("load " + state.css, null);
      } else {
        trySources(this, state, 0);
      }
      return state.loaded;
    }
  }
  expose(FontFace, "FontFace");

  // A FontFace attribute set: a value that does not parse throws.
  function setDescriptor(face, key, name, value) {
    const state = faceOf(face);
    const text = descriptorText(name, $String(value));
    if (text === null) {
      throw new $DOMException(
        "Failed to set the '" + key + "' property on 'FontFace': Failed to set '" + $String(value) +
          "' as a property value.",
        "SyntaxError",
      );
    }
    state[key] = text;
  }

  // ---- FontFaceSetLoadEvent ----

  const loadEventFaces = new $WeakMap();
  class FontFaceSetLoadEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      needToConstruct(arguments.length, 1, "FontFaceSetLoadEvent");
      super(type, eventInitDict);
      const faces = [];
      const given = eventInitDict === undefined || eventInitDict === null ? undefined : eventInitDict.fontfaces;
      if (given !== undefined) {
        for (const face of given) {
          faceOf(face);
          ArrayPush(faces, face);
        }
      }
      WeakMapSet(loadEventFaces, this, ObjectFreeze(faces));
    }

    get fontfaces() {
      const faces = WeakMapGet(loadEventFaces, this);
      if (faces === undefined) throw illegalInvocation();
      return faces;
    }
  }
  expose(FontFaceSetLoadEvent, "FontFaceSetLoadEvent");

  // ---- FontFaceSet ----

  // Each set's state: the faces scripts added, in order; whether it is
  // the document's, whose CSS-connected faces come first; whether it
  // last said it was loading; the faces had and failed since; and its
  // `ready` promise.
  const fontSets = new $WeakMap();

  function setOf(set) {
    const state = WeakMapGet(fontSets, set);
    if (state === undefined) throw illegalInvocation();
    return state;
  }

  // The document's faces of its `@font-face` rules, as the engine last
  // said: one FontFace each, kept while its rule is the same, wherever
  // rules come or go before it. A face whose rule went while it loaded
  // fails, so that a `load()` waiting for it settles.
  let cssFaces = [];
  let documentSet = null;

  function syncCssFaces() {
    if (WORKER) return cssFaces;
    const flat = native.fonts("rules", null);
    const next = [];
    const unused = ArraySlice(cssFaces);
    for (let i = 0, n = 0; i + 8 <= flat.length; i += 8, n++) {
      const key = ArrayJoin(ArraySlice(flat, i, i + 7), "\u0000");
      let face;
      for (let k = 0; k < unused.length; k++) {
        if (WeakMapGet(fontFaces, unused[k]).key === key) {
          face = unused[k];
          ArraySplice(unused, k, 1);
          break;
        }
      }
      let state = face === undefined ? undefined : WeakMapGet(fontFaces, face);
      if (state === undefined) {
        face = ObjectCreate(FontFace.prototype);
        state = newFaceState(flat[i], {
          style: flat[i + 1], weight: flat[i + 2], stretch: flat[i + 3],
          unicodeRange: flat[i + 4], featureSettings: flat[i + 5], display: flat[i + 6],
        });
        state.key = key;
        if (documentSet !== null) ArrayPush(state.sets, documentSet);
        WeakMapSet(fontFaces, face, state);
      }
      state.css = n;
      const status = flat[i + 7];
      if (status === "loaded" || status === "error") {
        settleFace(face, state, status);
      } else if (status === "loading" && state.status === "unloaded") {
        state.status = "loading";
      }
      ArrayPush(next, face);
    }
    cssFaces = next;
    for (let k = 0; k < unused.length; k++) {
      const state = WeakMapGet(fontFaces, unused[k]);
      state.css = -1;
      if (state.status === "loading") settleFace(unused[k], state, "error");
    }
    return next;
  }

  // Every face of the set: the CSS-connected ones first.
  function allFaces(set) {
    const own = setOf(set).faces;
    if (set !== documentSet) return ArraySlice(own);
    const all = ArraySlice(syncCssFaces());
    for (let i = 0; i < own.length; i++) ArrayPush(all, own[i]);
    return all;
  }

  // Whether the set is loading: a face of it is, or one the document's
  // layout asked for.
  function setLoading(set, faceOnly) {
    const faces = allFaces(set);
    for (let i = 0; i < faces.length; i++) {
      if (WeakMapGet(fontFaces, faces[i]).status === "loading") return true;
    }
    return !faceOnly && set === documentSet && native.fonts("status", null) === "loading";
  }

  // A set's event, fired in a task of its own.
  function fireSoon(set, event) {
    queueTask(() => fireAt(set, event));
  }

  // The set's status changed, perhaps: `loading` fired when it began to
  // load, `loadingdone` (and `loadingerror` for faces that failed) when
  // it is done, and `ready` settled then (section 2.4).
  function fontsChanged(set, faceOnly) {
    const s = setOf(set);
    const loading = setLoading(set, faceOnly);
    if (loading && !s.loading) {
      s.loading = true;
      if (s.readyDone) {
        s.readyDone = false;
        s.ready = new $Promise((resolve) => { s.readyResolve = resolve; });
      }
      fireSoon(set, new FontFaceSetLoadEvent("loading"));
    } else if (!loading && s.loading) {
      s.loading = false;
      const loaded = s.loaded;
      const failed = s.failed;
      s.loaded = [];
      s.failed = [];
      fireSoon(set, new FontFaceSetLoadEvent("loadingdone", { fontfaces: loaded }));
      if (failed.length > 0) {
        fireSoon(set, new FontFaceSetLoadEvent("loadingerror", { fontfaces: failed }));
      }
    }
    if (!loading && !s.readyDone) {
      s.readyDone = true;
      s.readyResolve(set);
    }
  }

  // What the engine says of the document's faces: loading began, or
  // ended with the rules' faces of those numbers had and failed.
  function fontsTold(type, init) {
    if (documentSet === null) return;
    const s = setOf(documentSet);
    const faces = syncCssFaces();
    const numbers = (text) => (text === undefined || text === "" ? [] : StringSplit(text, ","));
    if (type === "loadingdone" && s.loading) {
      const loaded = numbers(init.loaded);
      for (let i = 0; i < loaded.length; i++) {
        const face = faces[$Number(loaded[i])];
        if (face !== undefined && ArrayIndexOf(s.loaded, face) < 0) ArrayPush(s.loaded, face);
      }
      const failed = numbers(init.failed);
      for (let i = 0; i < failed.length; i++) {
        const face = faces[$Number(failed[i])];
        if (face !== undefined && ArrayIndexOf(s.failed, face) < 0) ArrayPush(s.failed, face);
      }
    }
    if (type === "loading" && !s.loading) {
      s.loading = true;
      if (s.readyDone) {
        s.readyDone = false;
        s.ready = new $Promise((resolve) => { s.readyResolve = resolve; });
      }
      fireAt(documentSet, new FontFaceSetLoadEvent("loading"));
      return;
    }
    fontsChanged(documentSet, type === "loadingdone");
  }

  // `fontsTold` as an engine's task steps it.
  function* fontsToldSteps(type, init) {
    fontsTold(type, init);
    return false;
  }

  class FontFaceSet extends EventTarget {
    constructor(key) {
      if (key !== KEY) throw illegalConstructor("FontFaceSet");
      super();
      const state = {
        faces: [], loading: false, loaded: [], failed: [], ready: null, readyResolve: null,
        readyDone: false,
      };
      state.ready = new $Promise((resolve) => { state.readyResolve = resolve; });
      WeakMapSet(fontSets, this, state);
    }

    add(face) {
      const faces = setOf(this).faces;
      const state = faceOf(face);
      if (state.css >= 0) {
        if (this === documentSet) return this;
        throw new $DOMException(
          "Failed to execute 'add' on 'FontFaceSet': Cannot add a CSS-connected FontFace.",
          "InvalidModificationError",
        );
      }
      if (ArrayIndexOf(faces, face) < 0) {
        ArrayPush(faces, face);
        ArrayPush(state.sets, this);
        if (state.status === "loaded") addFace(face, state);
        fontsChanged(this);
      }
      return this;
    }

    delete(face) {
      const faces = setOf(this).faces;
      const state = faceOf(face);
      const at = ArrayIndexOf(faces, face);
      if (at < 0) return false;
      ArraySplice(faces, at, 1);
      ArraySplice(state.sets, ArrayIndexOf(state.sets, this), 1);
      native.fonts("delete " + state.id, null);
      return true;
    }

    clear() {
      const faces = setOf(this).faces;
      while (faces.length > 0) this.delete(faces[0]);
    }

    has(face) {
      return ArrayIndexOf(allFaces(this), face) >= 0;
    }

    get size() {
      return allFaces(this).length;
    }

    forEach(callback, thisArg) {
      const faces = allFaces(this);
      for (let i = 0; i < faces.length; i++) ReflectApply(callback, thisArg, [faces[i], faces[i], this]);
    }

    values() {
      return ReflectApply(ArrayValues, allFaces(this), []);
    }

    keys() {
      return ReflectApply(ArrayValues, allFaces(this), []);
    }

    entries() {
      const faces = allFaces(this);
      const pairs = [];
      for (let i = 0; i < faces.length; i++) ArrayPush(pairs, [faces[i], faces[i]]);
      return ReflectApply(ArrayValues, pairs, []);
    }

    get status() {
      return setLoading(this) ? "loading" : "loaded";
    }

    // Settled when the set is done loading. What the task asking did to
    // the document (a sheet with faces, an element needing one) its
    // layout sees later, so the document's set looks again in a later
    // task, and each frame while it loads.
    get ready() {
      const set = this;
      const s = setOf(set);
      if (s.readyDone && set === documentSet) {
        s.readyDone = false;
        s.ready = new $Promise((resolve) => { s.readyResolve = resolve; });
      }
      if (!s.readyDone && !s.polling) {
        s.polling = true;
        const check = () => {
          if (setLoading(set)) {
            queueTask(check, 16);
            return;
          }
          s.polling = false;
          fontsChanged(set);
        };
        if (set === documentSet) queueTask(check, 16);
        else check();
      }
      return s.ready;
    }

    check(font, text) {
      need(arguments.length, 1, "check", "FontFaceSet");
      const faces = matchingFaces(this, $String(font), "check");
      for (let i = 0; i < faces.length; i++) {
        if (WeakMapGet(fontFaces, faces[i]).status !== "loaded") return false;
      }
      return true;
    }

    load(font, text) {
      need(arguments.length, 1, "load", "FontFaceSet");
      let faces;
      try {
        faces = matchingFaces(this, $String(font), "load");
      } catch (e) {
        return PromiseReject(e);
      }
      const promises = [];
      for (let i = 0; i < faces.length; i++) ArrayPush(promises, faces[i].load());
      return PromiseThen(ReflectApply($Promise.all, $Promise, [promises]), () => faces);
    }
  }
  defineProperty(FontFaceSet.prototype, iteratorSymbol, {
    value: FontFaceSet.prototype.values,
    writable: true,
    configurable: true,
  });
  defineHandlers(FontFaceSet.prototype, ["loading", "loadingdone", "loadingerror"], false);
  expose(FontFaceSet, "FontFaceSet");

  // The faces of `set` whose family `font` (a `font` shorthand) names:
  // what `load()` and `check()` look at. A font that does not parse
  // throws.
  function matchingFaces(set, font, method) {
    const names = native.fonts("families " + font, null);
    if (names.length === 0) {
      throw new $DOMException(
        "Failed to execute '" + method + "' on 'FontFaceSet': Could not resolve '" + font +
          "' as a font.",
        "SyntaxError",
      );
    }
    const wanted = [];
    for (let i = 1; i < names.length; i++) ArrayPush(wanted, ToLowerCase(names[i]));
    const out = [];
    const faces = allFaces(set);
    for (let i = 0; i < faces.length; i++) {
      if (ArrayIndexOf(wanted, ToLowerCase(WeakMapGet(fontFaces, faces[i]).family)) >= 0) {
        ArrayPush(out, faces[i]);
      }
    }
    return out;
  }

  // The document's face set, made when first asked for; a worker's
  // (`self.fonts`), its own.
  const documentFonts = new $WeakMap();
  function fontSetOf(document) {
    let set = WeakMapGet(documentFonts, document);
    if (set === undefined) {
      set = new FontFaceSet(KEY);
      WeakMapSet(documentFonts, document, set);
      if (!WORKER && document === theDocument()) {
        documentSet = set;
        const faces = syncCssFaces();
        for (let i = 0; i < faces.length; i++) {
          const state = WeakMapGet(fontFaces, faces[i]);
          if (ArrayIndexOf(state.sets, set) < 0) ArrayPush(state.sets, set);
        }
      }
    }
    return set;
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
    yield* fireFrameEvents();
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
    // WebGPU's canvases drawn into are presented (M41).
    if (chunkParts.gpu !== undefined) {
      chunkParts.gpu.gpuPresent();
      chunkParts.gpu.webglPresent();
    }
    return false;
  }

  // ---- WebAssembly, a chunk made at its first use (M29, D64) ----------------
  // `WebAssembly` is the chunk's accessor until a page reaches it.
  chunkLazy("wasm", global, "WebAssembly", false, true);
  // What the chunk keeps of what a page may change, kept before it ran.
  const ResponseArrayBuffer = Response.prototype.arrayBuffer;

  // ---- Web Audio, a chunk made at its first use (M40, D64) ------------------
  // Its interfaces are the chunk's accessors until a page reaches them.
  chunkInterfaces("audio", [
    "AudioBuffer", "AudioParam", "AudioNode", "AudioDestinationNode", "AudioScheduledSourceNode",
    "ConstantSourceNode", "OscillatorNode", "AudioBufferSourceNode", "GainNode", "DelayNode",
    "BiquadFilterNode", "IIRFilterNode", "StereoPannerNode", "PannerNode", "AudioListener",
    "ChannelSplitterNode", "ChannelMergerNode", "WaveShaperNode", "DynamicsCompressorNode",
    "ConvolverNode", "AnalyserNode", "PeriodicWave", "BaseAudioContext", "AudioContext",
    "MediaElementAudioSourceNode", "OfflineAudioContext", "OfflineAudioCompletionEvent",
    "ScriptProcessorNode", "AudioProcessingEvent", "MediaStreamAudioDestinationNode",
    "MediaStreamAudioSourceNode", "MediaStream", "MediaStreamTrack", "CanvasCaptureMediaStreamTrack",
  ]);

  // What the chunk keeps of the built-ins, kept before any page ran.
  const MathAbs = Math.abs;
  const MathFround = Math.fround;

  // ---- Media elements (M40, HTML 4.8.11) ------------------------------------
  // An element's state is the engine's (`native.media`, JSON both ways);
  // what its algorithms queue comes back as `media` tasks, events and
  // play() promises settled in order.

  const mediaPromises = new $Map();
  let mediaPromiseNext = 0;
  const mediaErrors = new $WeakMap();
  const mediaPitch = new $WeakMap();

  function mediaCall(op, words) {
    const answer = JSONParse(native.media(JSONStringify(op)));
    if (answer.error !== undefined) {
      const e = answer.error;
      if (e.name === "TypeError") throw new $TypeError(words + e.message);
      throw new $DOMException(e.message, e.name);
    }
    return answer.result;
  }

  function mediaState(element) {
    return mediaCall({ op: "state", node: self(element) }, "");
  }

  class MediaError {
    #code; #message;

    constructor(key, code, message) {
      if (key !== KEY) throw illegalConstructor("MediaError");
      this.#code = code;
      this.#message = message;
    }

    get code() { return this.#code; }
    get message() { return this.#message; }
  }
  expose(MediaError, "MediaError");

  class TimeRanges {
    #ranges;

    constructor(key, ranges) {
      if (key !== KEY) throw illegalConstructor("TimeRanges");
      this.#ranges = ranges;
    }

    get length() { return this.#ranges.length; }

    #at(index, method) {
      const i = unsigned(index);
      if (i >= this.#ranges.length) {
        throw domError("IndexSizeError", "The index provided (" + i + ") is greater than or equal to the maximum bound (" + this.#ranges.length + ").", method, "TimeRanges");
      }
      return this.#ranges[i];
    }

    start(index) {
      need(arguments.length, 1, "start", "TimeRanges");
      return this.#at(index, "start")[0];
    }

    end(index) {
      need(arguments.length, 1, "end", "TimeRanges");
      return this.#at(index, "end")[1];
    }
  }
  expose(TimeRanges, "TimeRanges");

  const MEDIA_CONSTANTS = [
    ["NETWORK_EMPTY", 0], ["NETWORK_IDLE", 1], ["NETWORK_LOADING", 2], ["NETWORK_NO_SOURCE", 3],
    ["HAVE_NOTHING", 0], ["HAVE_METADATA", 1], ["HAVE_CURRENT_DATA", 2], ["HAVE_FUTURE_DATA", 3],
    ["HAVE_ENOUGH_DATA", 4],
  ];
  const ERROR_CONSTANTS = [
    ["MEDIA_ERR_ABORTED", 1], ["MEDIA_ERR_NETWORK", 2], ["MEDIA_ERR_DECODE", 3],
    ["MEDIA_ERR_SRC_NOT_SUPPORTED", 4],
  ];
  function mediaConstants(target, list) {
    for (let i = 0; i < list.length; i++) {
      defineProperty(target, list[i][0], { value: list[i][1], enumerable: true });
      defineProperty(target.prototype, list[i][0], { value: list[i][1], enumerable: true });
    }
  }
  mediaConstants(HTMLMediaElement, MEDIA_CONSTANTS);
  mediaConstants(MediaError, ERROR_CONSTANTS);

  function timeRanges(list) {
    const ranges = [];
    for (let i = 0; i < list.length; i++) {
      ArrayPush(ranges, [$Number(list[i][0]), $Number(list[i][1])]);
    }
    return new TimeRanges(KEY, ranges);
  }

  const mediaSet = (property) => "Failed to set the '" + property + "' property on 'HTMLMediaElement': ";

  include(
    {
      get networkState() { return mediaState(this).networkState; },
      get readyState() { return mediaState(this).readyState; },
      get paused() { return mediaState(this).paused; },
      get seeking() { return mediaState(this).seeking; },
      get ended() { return mediaState(this).ended; },
      get currentSrc() { return mediaState(this).currentSrc; },
      get duration() { return $Number(mediaState(this).duration); },
      get currentTime() { return $Number(mediaState(this).currentTime); },
      set currentTime(value) {
        const time = +value;
        if (!$NumberIsFinite(time)) {
          throw new $TypeError(mediaSet("currentTime") + "The provided double value is non-finite.");
        }
        mediaCall({ op: "seek", node: self(this), time }, mediaSet("currentTime"));
      },
      get volume() { return mediaState(this).volume; },
      set volume(value) {
        const volume = +value;
        if (!$NumberIsFinite(volume)) {
          throw new $TypeError(mediaSet("volume") + "The provided double value is non-finite.");
        }
        if (volume < 0 || volume > 1) {
          throw new $DOMException(mediaSet("volume") + "The volume provided (" + volume + ") is outside the range [0, 1].", "IndexSizeError");
        }
        mediaCall({ op: "volume", node: self(this), value: volume }, mediaSet("volume"));
      },
      get muted() { return mediaState(this).muted; },
      set muted(value) {
        mediaCall({ op: "muted", node: self(this), value: !!value }, mediaSet("muted"));
      },
      get playbackRate() { return mediaState(this).playbackRate; },
      set playbackRate(value) {
        const rate = +value;
        if (!$NumberIsFinite(rate)) {
          throw new $TypeError(mediaSet("playbackRate") + "The provided double value is non-finite.");
        }
        const size = rate < 0 ? -rate : rate;
        if (rate !== 0 && (size < 0.0625 || size > 16)) {
          throw new $DOMException(mediaSet("playbackRate") + "The provided playback rate (" + rate + ") is not in the supported playback range.", "NotSupportedError");
        }
        mediaCall({ op: "rate", node: self(this), value: rate }, mediaSet("playbackRate"));
      },
      get defaultPlaybackRate() { return mediaState(this).defaultPlaybackRate; },
      set defaultPlaybackRate(value) {
        const rate = +value;
        if (!$NumberIsFinite(rate)) {
          throw new $TypeError(mediaSet("defaultPlaybackRate") + "The provided double value is non-finite.");
        }
        mediaCall({ op: "defaultRate", node: self(this), value: rate }, mediaSet("defaultPlaybackRate"));
      },
      // Kept and reflected; the sound's pitch moves with its rate yet.
      get preservesPitch() {
        self(this);
        const kept = WeakMapGet(mediaPitch, this);
        return kept === undefined ? true : kept;
      },
      set preservesPitch(value) {
        self(this);
        WeakMapSet(mediaPitch, this, !!value);
      },
      get preload() {
        const value = native.attribute(self(this), "preload");
        if (value === null) return "metadata";
        const lower = ToLowerCase(value);
        if (lower === "none" || lower === "metadata" || lower === "auto") return lower;
        return lower === "" ? "auto" : "metadata";
      },
      set preload(value) {
        setAttribute(self(this), "preload", str(value), "preload");
      },
      get error() {
        const error = mediaState(this).error;
        if (error === null) return null;
        const kept = WeakMapGet(mediaErrors, this);
        if (kept !== undefined && kept.code === error.code && kept.message === error.message) {
          return kept;
        }
        const made = new MediaError(KEY, error.code, error.message);
        WeakMapSet(mediaErrors, this, made);
        return made;
      },
      get buffered() { return timeRanges(mediaState(this).buffered); },
      get seekable() { return timeRanges(mediaState(this).seekable); },
      get played() { return timeRanges(mediaState(this).played); },

      load() {
        mediaCall({ op: "load", node: self(this) }, prefix("load", "HTMLMediaElement"));
      },

      canPlayType(type) {
        self(this);
        need(arguments.length, 1, "canPlayType", "HTMLMediaElement");
        return mediaCall({ op: "canPlayType", type: str(type) }, prefix("canPlayType", "HTMLMediaElement"));
      },

      play() {
        let handle;
        try {
          handle = self(this);
        } catch (e) {
          return rejected(e);
        }
        const id = ++mediaPromiseNext;
        let settle;
        const promise = new $Promise((resolve, reject) => { settle = { resolve, reject }; });
        MapSet(mediaPromises, id, settle);
        try {
          mediaCall({ op: "play", node: handle, promise: id }, prefix("play", "HTMLMediaElement"));
        } catch (e) {
          MapDelete(mediaPromises, id);
          return rejected(e);
        }
        return promise;
      },

      pause() {
        mediaCall({ op: "pause", node: self(this) }, prefix("pause", "HTMLMediaElement"));
      },
    },
    [HTMLMediaElement.prototype],
  );

  const HTMLVideoElement = MapGet(interfaces, "video");
  include(
    {
      get videoWidth() { return mediaState(this).videoWidth; },
      get videoHeight() { return mediaState(this).videoHeight; },
    },
    [HTMLVideoElement.prototype],
  );

  // `new Audio(src)`: an `<audio>` element of the window's document, its
  // preload `auto`, as the reference makes it.
  const HTMLAudioElement = MapGet(interfaces, "audio");
  function Audio(src) {
    if (new.target === undefined) {
      throw new $TypeError("Failed to construct 'Audio': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
    }
    const element = theDocument().createElement("audio");
    element.setAttribute("preload", "auto");
    if (src !== undefined) element.setAttribute("src", str(src));
    return element;
  }
  defineProperty(Audio, "prototype", { value: HTMLAudioElement.prototype });
  windowOnly("Audio", { value: Audio, writable: true, configurable: true });

  // What the engine's media elements queue, or a pipeline's report to
  // hear (its actions then carried out in this task).
  function* mediaTask(text) {
    const told = JSONParse(text);
    // A media key's action (M40): the session's handler.
    if (told.session !== undefined) {
      return yield* sessionAction(told.session);
    }
    let actions = told.actions;
    if (told.report !== undefined) {
      actions = mediaCall({ op: "report", node: told.node, generation: told.generation, report: told.report }, "");
    }
    for (let i = 0; i < actions.length; i++) {
      const action = actions[i];
      if (action.promise !== undefined) {
        const settle = MapGet(mediaPromises, action.promise);
        if (settle === undefined) continue;
        MapDelete(mediaPromises, action.promise);
        if (action.name === undefined) {
          settle.resolve(undefined);
        } else {
          settle.reject(new $DOMException(action.message, action.name));
        }
        // Its reactions run before the next event, which the reference
        // fires in a task of its own.
        yield;
        continue;
      }
      const target = wrap(action.node);
      if (target === null || target === undefined) continue;
      const event = new Event(action.event);
      eventState(event).trusted = true;
      yield* dispatching(target, event);
    }
    return false;
  }

  // ---- Media Session (M40) --------------------------------------------------
  // The document's session: its metadata, its playback state and its
  // action handlers, which the media keys call (the engine is told which
  // there are), as the reference's.

  const SESSION_ACTIONS = [
    "play", "pause", "seekbackward", "seekforward", "previoustrack", "nexttrack", "skipad", "stop",
    "seekto", "togglemicrophone", "togglecamera", "hangup", "previousslide", "nextslide",
    "enterpictureinpicture",
  ];
  const SESSION_STATES = ["none", "paused", "playing"];

  let sessionHandlers = null;

  function listed(list, value) {
    for (let i = 0; i < list.length; i++) {
      if (list[i] === value) return true;
    }
    return false;
  }

  function sessionTell(state) {
    const actions = [];
    if (sessionHandlers !== null) {
      MapForEach(sessionHandlers, (_, action) => ArrayPush(actions, action));
    }
    mediaCall({ op: "session", state, actions }, "");
  }

  class MediaMetadata {
    #title = ""; #artist = ""; #album = ""; #artwork = [];

    constructor(init) {
      const given = init === undefined || init === null ? {} : init;
      if (given.title !== undefined) this.#title = str(given.title);
      if (given.artist !== undefined) this.#artist = str(given.artist);
      if (given.album !== undefined) this.#album = str(given.album);
      if (given.artwork !== undefined) this.artwork = given.artwork;
    }

    get title() { return this.#title; }
    set title(v) { this.#title = str(v); }
    get artist() { return this.#artist; }
    set artist(v) { this.#artist = str(v); }
    get album() { return this.#album; }
    set album(v) { this.#album = str(v); }
    get artwork() { return this.#artwork; }
    set artwork(list) {
      const out = [];
      const n = list === null || list === undefined ? 0 : list.length >>> 0;
      for (let i = 0; i < n; i++) {
        const image = list[i];
        if (image === null || typeof image !== "object" || image.src === undefined) {
          throw new $TypeError("Failed to set the 'artwork' property on 'MediaMetadata': Failed to read the 'src' property from 'MediaImage': Required member is undefined.");
        }
        const src = native.resolveUrl(str(image.src));
        if (src === null) {
          throw new $TypeError("Failed to set the 'artwork' property on 'MediaMetadata': '" + str(image.src) + "' can't be resolved to a valid URL.");
        }
        ArrayPush(out, ObjectFreeze({
          sizes: image.sizes === undefined ? "" : str(image.sizes),
          src,
          type: image.type === undefined ? "" : str(image.type),
        }));
      }
      this.#artwork = ObjectFreeze(out);
    }
    get chapterInfo() { return ObjectFreeze([]); }
  }
  expose(MediaMetadata, "MediaMetadata");

  class MediaSession {
    #metadata = null;
    #state = "none";

    constructor(key) {
      if (key !== KEY) throw illegalConstructor("MediaSession");
      sessionHandlers = new $Map();
    }

    get metadata() { return this.#metadata; }
    set metadata(v) {
      if (v !== null && !(v instanceof MediaMetadata)) {
        throw new $TypeError("Failed to set the 'metadata' property on 'MediaSession': Failed to convert value to 'MediaMetadata'.");
      }
      this.#metadata = v;
    }

    get playbackState() { return this.#state; }
    set playbackState(v) {
      const state = str(v);
      if (!listed(SESSION_STATES, state)) return;
      this.#state = state;
      sessionTell(state);
    }

    setActionHandler(action, handler) {
      need(arguments.length, 2, "setActionHandler", "MediaSession");
      const name = str(action);
      if (!listed(SESSION_ACTIONS, name)) {
        throw new $TypeError(prefix("setActionHandler", "MediaSession") + "The provided value '" + name + "' is not a valid enum value of type MediaSessionAction.");
      }
      if (handler === null) {
        MapDelete(sessionHandlers, name);
      } else if (typeof handler === "function") {
        MapSet(sessionHandlers, name, handler);
      } else {
        throw new $TypeError(prefix("setActionHandler", "MediaSession") + "The callback provided as parameter 2 is not a function.");
      }
      sessionTell(this.#state);
    }

    setPositionState(state) {
      const words = prefix("setPositionState", "MediaSession");
      if (state === undefined || state === null) return;
      if (typeof state !== "object") {
        throw new $TypeError(words + "The provided value is not of type 'MediaPositionState'.");
      }
      if (state.duration === undefined && state.position === undefined && state.playbackRate === undefined) {
        return;
      }
      const duration = state.duration === undefined ? undefined : +state.duration;
      if (duration === undefined) {
        throw new $TypeError(words + "The duration must be provided and must be greater than or equal to zero.");
      }
      if (duration < 0) {
        throw new $TypeError(words + "The provided duration cannot be less than zero.");
      }
      if (state.position !== undefined && +state.position > duration) {
        throw new $TypeError(words + "The provided position cannot be greater than the duration.");
      }
      if (state.position !== undefined && +state.position < 0) {
        throw new $TypeError(words + "The provided position cannot be less than zero.");
      }
      if (state.playbackRate !== undefined && +state.playbackRate === 0) {
        throw new $TypeError(words + "The provided playbackRate cannot be equal to zero.");
      }
    }
  }
  expose(MediaSession, "MediaSession");

  if (!WORKER) {
    let session = null;
    defineProperty(Navigator.prototype, "mediaSession", {
      get() {
        if (session === null) session = new MediaSession(KEY);
        return session;
      },
      enumerable: true,
      configurable: true,
    });
  }

  // An action of the session, from a media key: its handler called.
  function* sessionAction(action) {
    const handler = sessionHandlers === null ? undefined : MapGet(sessionHandlers, action);
    if (handler !== undefined) {
      ReflectApply(handler, undefined, [{ action }]);
    }
    return false;
  }

  // ---- Never DRM (M40) ------------------------------------------------------
  // Encrypted Media Extensions are never here: every key system is
  // refused as a browser without a content decryption module refuses
  // it, so that a page falls back; MediaKeys and its kin are absent.

  if (!WORKER) {
    defineProperty(Navigator.prototype, "requestMediaKeySystemAccess", {
      value: function requestMediaKeySystemAccess(keySystem, supportedConfigurations) {
        const words = prefix("requestMediaKeySystemAccess", "Navigator");
        if (arguments.length < 2) {
          return rejected(new $TypeError(words + "2 arguments required, but only " + arguments.length + " present."));
        }
        if (str(keySystem) === "") {
          return rejected(new $TypeError(words + "The keySystem parameter is empty."));
        }
        if (supportedConfigurations === null || supportedConfigurations === undefined || (supportedConfigurations.length >>> 0) === 0) {
          return rejected(new $TypeError(words + "The supportedConfigurations parameter is empty."));
        }
        return rejected(new $DOMException("Unsupported keySystem or supportedConfigurations.", "NotSupportedError"));
      },
      writable: true,
      enumerable: true,
      configurable: true,
    });
  }

  include(
    {
      get mediaKeys() {
        self(this);
        return null;
      },
      setMediaKeys(mediaKeys) {
        try {
          self(this);
          need(arguments.length, 1, "setMediaKeys", "HTMLMediaElement");
        } catch (e) {
          return rejected(e);
        }
        if (mediaKeys === null) return resolved(undefined);
        return rejected(new $TypeError(prefix("setMediaKeys", "HTMLMediaElement") + "parameter 1 is not of type 'MediaKeys'."));
      },
    },
    [HTMLMediaElement.prototype],
  );

  // ---- WebGPU and WebGL, a chunk made at its first use (M41, D64) ------
  // Until a page reaches them (a name of theirs on the global,
  // `navigator.gpu`, a canvas's context of theirs, a GPU task), those
  // names are the chunk's accessors.
  const GPU_LAZY_NAMES = [
    "GPUSupportedFeatures", "WGSLLanguageFeatures", "GPUSupportedLimits", "GPUAdapterInfo",
    "GPUError", "GPUValidationError", "GPUOutOfMemoryError", "GPUInternalError",
    "GPUUncapturedErrorEvent", "GPUDeviceLostInfo", "GPU", "GPUAdapter", "GPUDevice", "GPUQueue",
    "GPUCompilationMessage", "GPUCompilationInfo", "GPUPipelineError", "GPUBuffer",
    "GPUShaderModule", "GPUTexture", "GPUTextureView", "GPUSampler", "GPUBindGroupLayout",
    "GPUPipelineLayout", "GPUBindGroup", "GPUComputePipeline", "GPURenderPipeline", "GPUQuerySet",
    "GPUCommandBuffer", "GPURenderBundle", "GPUCommandEncoder", "GPURenderPassEncoder",
    "GPUComputePassEncoder", "GPURenderBundleEncoder", "GPUCanvasContext", "WebGLBuffer",
    "WebGLShader", "WebGLProgram", "WebGLTexture", "WebGLRenderbuffer", "WebGLFramebuffer",
    "WebGLUniformLocation", "WebGLVertexArrayObject", "WebGLQuery", "WebGLSync",
    "WebGLTransformFeedback", "WebGLSampler", "WebGLActiveInfo", "WebGLShaderPrecisionFormat",
    "WebGLContextEvent", "WebGLRenderingContext", "WebGL2RenderingContext",
  ];
  const GPU_LAZY_NAMESPACES = [
    "GPUBufferUsage", "GPUMapMode", "GPUTextureUsage", "GPUShaderStage", "GPUColorWrite",
  ];
  chunkInterfaces("gpu", GPU_LAZY_NAMES);
  if (!WORKER) {
    for (let i = 0; i < GPU_LAZY_NAMESPACES.length; i++) {
      chunkLazy("gpu", global, GPU_LAZY_NAMESPACES[i], false, true);
    }
    chunkLazy("gpu", Navigator.prototype, "gpu", true, false);
  }

  // What the chunk keeps of the built-ins, kept before any page ran.
  const GpuSetHas = uncurry(Set.prototype.has);
  const GpuSetValues = uncurry(Set.prototype.values);
  const GpuSetEntries = uncurry(Set.prototype.entries);
  const TypedArraySubarray = uncurry(TypedArrayPrototype.subarray);
  const GpuCharCodeAt = uncurry(String.prototype.charCodeAt);
  const GpuLastIndexOf = uncurry(String.prototype.lastIndexOf);
  const GpuIndexOf = uncurry(String.prototype.indexOf);
  const GpuUint32Array = Uint32Array;
  const GpuArrayIndexOf = uncurry(Array.prototype.indexOf);

  // A canvas's context of WebGPU's, or WebGL's version (0 for any other
  // context), without making them.
  function gpuIsCanvasContext(context) {
    return chunkParts.gpu !== undefined && context instanceof chunkParts.gpu.GPUCanvasContext;
  }
  function gpuWebglVersion(context) {
    return chunkParts.gpu === undefined ? 0 : chunkParts.gpu.webglVersionOf(context);
  }


  // ---- Intl, a chunk made at its first use (D63, D64) -----------------------
  // `Intl` is the chunk's accessor until a page reaches it; the locale
  // forms of the language's methods are the bootstrap's own, which call
  // the chunk's.
  chunkLazy("intl", global, "Intl", false, true);
  // What the chunk keeps of the built-ins, kept before any page ran.
  const DateGetTimezoneOffset = uncurry(Date.prototype.getTimezoneOffset);

  // A locale form: a built-in's data property, writable, configurable,
  // not enumerable.
  function intlLocaleForm(target, name, value) {
    defineProperty(target, name, { value, writable: true, configurable: true, enumerable: false });
  }

  intlLocaleForm(Number.prototype, "toLocaleString", {
    toLocaleString() {
      return ReflectApply(chunkInstall("intl").numberToLocaleString, this, arguments);
    },
  }.toLocaleString);
  intlLocaleForm(BigInt.prototype, "toLocaleString", {
    toLocaleString() {
      return ReflectApply(chunkInstall("intl").bigintToLocaleString, this, arguments);
    },
  }.toLocaleString);
  intlLocaleForm(Date.prototype, "toLocaleString", {
    toLocaleString() {
      return ReflectApply(chunkInstall("intl").dateToLocaleString, this, arguments);
    },
  }.toLocaleString);
  intlLocaleForm(Date.prototype, "toLocaleDateString", {
    toLocaleDateString() {
      return ReflectApply(chunkInstall("intl").dateToLocaleDateString, this, arguments);
    },
  }.toLocaleDateString);
  intlLocaleForm(Date.prototype, "toLocaleTimeString", {
    toLocaleTimeString() {
      return ReflectApply(chunkInstall("intl").dateToLocaleTimeString, this, arguments);
    },
  }.toLocaleTimeString);
  intlLocaleForm(String.prototype, "localeCompare", {
    localeCompare(that) {
      return ReflectApply(chunkInstall("intl").localeCompare, this, arguments);
    },
  }.localeCompare);

  // ---- The Web Animations API, a chunk made at its first use (D64, D75) ----
  // Its interfaces, `animate()` and `getAnimations()` on elements, and
  // `getAnimations()` and `timeline` on the document, are the chunk's
  // accessors until a page reaches one of them.
  if (!WORKER) {
    chunkInterfaces("animations", [
      "Animation", "AnimationEffect", "AnimationPlaybackEvent", "AnimationTimeline",
      "CSSAnimation", "CSSMathSum", "CSSMathValue", "CSSNumericValue", "CSSStyleValue",
      "CSSTransition", "CSSUnitValue", "DocumentTimeline", "KeyframeEffect", "ScrollTimeline",
      "ViewTimeline",
    ]);
    // The numeric factories of the CSS namespace (CSS Typed OM 1 section
    // 5.3), which the chunk's numeric values make.
    const CSS_UNITS = [
      "number", "percent", "em", "rem", "ex", "ch", "lh", "vw", "vh", "vmin", "vmax", "cm",
      "mm", "q", "in", "pt", "pc", "px", "deg", "grad", "rad", "turn", "s", "ms", "hz",
      "khz", "dpi", "dpcm", "dppx", "fr",
    ];
    for (let i = 0; i < CSS_UNITS.length; i++) {
      chunkLazy("animations", CSSNamespace, CSS_UNITS[i], true, true);
    }
    chunkLazy("animations", Element.prototype, "animate", true, true);
    chunkLazy("animations", Element.prototype, "getAnimations", true, true);
    chunkLazy("animations", Document.prototype, "getAnimations", true, true);
    chunkLazy("animations", ShadowRoot.prototype, "getAnimations", true, true);
    chunkLazy("animations", Document.prototype, "timeline", true, false);
  }

  // A property set in an element's `style` attribute as a script's
  // `style.setProperty()` sets it, by the setter kept before any page ran:
  // what `commitStyles()` writes (D75).
  const StyleSetProperty = CSSStyleDeclaration.prototype.setProperty;
  function setStyleProperty(element, name, value) {
    ReflectApply(StyleSetProperty, styleOf(element), [name, value]);
  }

  // Each chunk's names of the bootstrap's (D64), as `chunkInstall` gives
  // them to it.
  const CHUNK_ENVS = {
    __proto__: null,
    animations: () => ({
      $DOMException, $FinalizationRegistry, $Map, $Number, $Promise, $TypeError, ArrayIndexOf,
      ArrayJoin, ArrayPush, ArraySort, CSSNamespace, CharCodeAt, Document, EVENT_INIT, Element,
      Event, EventTarget, JSONParse, JSONStringify, KEY, MapDelete, MapGet, MapSet, ObjectCreate,
      PromiseResolve, PromiseThen, ReflectApply, ReflectGetOwnPropertyDescriptor, ReflectOwnKeys,
      Register, StringIndexOf, StringSlice, StringTrim, ToLowerCase, ToUpperCase, defineHandlers,
      defineProperty, dispatching, documentElement, eventArgs, eventState, expose, handleOf,
      illegalConstructor, isNode, iteratorSymbol, kindOf, members, native, need, setStyleProperty,
      str, wrap,
    }),
    gpu: () => ({
      $ArrayBuffer, $DOMException, $DataView, $Map, $MapForEach, $Number, $NumberIsFinite, $Promise,
      $RangeError, $Set, $TypeError, $Uint8Array, $WeakMap, ArrayBufferIsView, ArrayIsArray,
      ArrayPush, ArraySlice, DataViewBuffer, DataViewLength, DataViewOffset, Event, EventTarget,
      FromCharCode, GpuArrayIndexOf, GpuCharCodeAt, GpuIndexOf, GpuLastIndexOf, GpuSetEntries,
      GpuSetHas, GpuSetValues, GpuUint32Array, JSONParse, JSONStringify, KEY, MapDelete, MapGet,
      MapSet, MathFloor, MathMax, MathMin, MathTrunc, Navigator, ObjectFreeze, PromiseThen,
      ReflectApply, SetAdd, SetForEach, SetSize, StringSlice, ToBase64, TypedArrayBuffer,
      TypedArrayCount, TypedArrayLength, TypedArrayOffset, TypedArraySet, TypedArraySubarray,
      TypedArrayTag, WORKER, WeakMapGet, WeakMapSet, binaryString, canvasTargets, defineHandlers,
      defineProperty, dispatching, domError, eventState, expose, fireAt, getOwnPropertyDescriptor,
      global, handleOf, illegalConstructor, illegalInvocation, imageDatas, interfaces, isNode,
      iteratorSymbol, kindOf, listed, native, need, needToConstruct, prefix, record, rejected,
      resolved, sequence, startTimer, str, toStringTag, viewLength,
    }),
    intl: () => ({
      $Date, $Number, $Object, $RangeError, $String, $TypeError, $WeakMap, ArrayIndexOf, ArrayPush,
      ArraySlice, ArraySort, BigIntToString, BigIntValueOf, CharCodeAt, DateGetTime,
      DateGetTimezoneOffset, DateNow, IteratorPrototype, MathAbs, MathFloor, MathMax, MathMin,
      MathTrunc, NumberValueOf, ObjectCreate, ObjectPrototype, StringIndexOf, StringSlice,
      ToLowerCase, ToUpperCase, ToWellFormed, WeakMapGet, WeakMapSet, defineProperty,
      getOwnPropertyDescriptor, getOwnPropertyNames, global, iteratorSymbol, languageList,
      native, toString, toStringTag,
    }),
    wasm: () => ({
      $Error, $Map, $RangeError, $TypeError, $Uint8Array, $WeakMap, ArrayPush, BigIntAsIntN,
      FunctionBind, MapDelete, MapForEach, MapGet, MapSet, MathTrunc, ObjectCreate, ObjectFreeze,
      ObjectPrototype, PromiseThen, ReflectApply, ResponseArrayBuffer, SetFromBase64, StringIndexOf,
      StringSlice, ToBase64, ToLowerCase, ToWellFormed, WeakMapGet, WeakMapSet, bufferBytes,
      bufferLength, combined, defineProperty, getOwnPropertyDescriptor, getOwnPropertyNames, global,
      headersState, isResponse, iteratorSymbol, native, rejected, resolved, responseState, toString,
      toStringTag, viewLength,
    }),
    audio: () => ({
      $ArrayIsArray, $DOMException, $Float32Array, $Map, $NumberIsFinite, $Promise, $RangeError,
      $TypeError, $Uint8Array, ArrayPush, Event, EventTarget, HTMLMediaElement, JSONParse,
      JSONStringify, KEY, MapDelete, MapGet, MapSet, MathAbs, MathFloor, MathFround, MathMax,
      MathMin, MathRandom, StringSlice, TypedArraySet,
      binaryString, bufferLength, defineHandlers, defineProperty, dispatching, domError, eventState,
      expose, failed, getOwnPropertyNames, illegalConstructor, include, listed, native, need,
      needToConstruct, performance, prefix, rejected, resolved, self, str, unsigned, viewBuffer,
      viewLength, viewOffset,
    }),
    idb: () => ({
      $ArrayIsArray, $DOMException, $Date, $DateGetTime, $Map, $Number, $Promise, $TypeError,
      ArrayIncludes, ArrayPush, ArraySlice, ArraySort, ArraySplice, EVENT_INIT, Event, EventTarget,
      FromBase64, GeneratorNext, JSONParse, JSONStringify, KEY, MapDelete, MapGet, MapSet,
      ObjectCreate, ObjectHasOwn, StringIndexOf, StringSlice, ToBase64, WeakMapDelete, WeakMapSet,
      branded, bufferBytes, defineHandlers, defineProperty, deserialize, dispatching, domError,
      drain, eventArgs, eventParents, eventState, expose, fieldsOf, illegalConstructor, made,
      members, native, nothing, prefix, serialize, toString, unsigned, unwire, wire,
    }),
    canvas: () => ({
      $DOMException, $Number, $RangeError, $TypeError, $Uint8Array, $Uint8ClampedArray, $WeakMap,
      ArrayPush, CTX, DOMMatrix, GLOBAL_INTERFACE, KEY, MapGet, MathTrunc, ReflectApply,
      StringIndexOf, StringSlice, ToBase64, ToLowerCase, TypedArrayBuffer, TypedArrayLength,
      TypedArrayOffset, TypedArrayTag, WORKER, WeakMapGet, WeakMapSet, WeakRefDeref,
      bindCanvasObject, blobBytes, canvasObjects, canvasTargets, chunkInstall, defineProperty,
      domError, expose, failed, fromList, getOwnPropertyDescriptor, getOwnPropertyNames,
      gpuIsCanvasContext, gpuWebglVersion, handleOf, illegalConstructor, illegalInvocation,
      imageDatas, include, isBlob, isNode, iteratorSymbol, kindOf, makeMatrix, matrixInit, native,
      need, needToConstruct, pngBlob, prefix, rejected, resolved, str, targetOf,
    }),
  };

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
      // What the engine says of the document's web fonts is for its
      // `document.fonts` (D54), which fires its own events.
      if (iface === "FontFaceSetLoadEvent") {
        return fontsToldSteps(type, init);
      }
      const known = MapGet(eventInterfaces, iface);
      if (typeof init.relatedTarget === "number") {
        init.relatedTarget = wrap(init.relatedTarget);
      }
      if (typeof init.submitter === "number") {
        init.submitter = wrap(init.submitter);
      }
      // A CSS animation's or transition's event names its animation by
      // its number (D75).
      if (typeof init.animation === "number") {
        init.animation = chunkInstall("animations").animationFor(init.animation);
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
        // The window's `load`, `pageshow` and `pagehide` name the document
        // their target.
        const named = type === "load" || type === "pageshow" || type === "pagehide";
        return dispatching(global, event, named ? theDocument() : undefined);
      }
      return dispatching(wrap(handle), event);
    },
    frames: runFrames,
    frameList,
    guardCode,
    devtools: devtoolsMessage,
    idb: () => chunkInstall("idb").idbTask(),
    serviceWorker: swTask,
    audio: (text, bytes) => chunkInstall("audio").audioTask(text, bytes),
    gpu: (text, bytes) => chunkInstall("gpu").gpuTask(text, bytes),
    media: mediaTask,
    // What the engine tells the animations scripts hold at a frame (D75):
    // nothing to tell before a page reached them.
    animations(text) {
      const part = chunkParts.animations;
      return part === undefined ? nothing() : part.animationsTask(text);
    },
    mediaChanged,
    // The top layer's (gap batch H): the user's close request, a press's
    // light dismiss, and a change of interest.
    closeRequest: closeRequestTask,
    lightDismiss: lightDismissTask,
    interest: interestTask,
    mutations: mutationsTask,
    observations: observationsTask,
    popState: popStateTask,
    broadcast: broadcastTask,
    namedSync,
    mutationsQueued: queueMutationObserverMicrotask,
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
    // Module scripts (D61): a module script's evaluation, whose
    // rejection is reported as an uncaught exception; and an `import()`'s
    // promise, settled as its graph's `promise` is.
    moduleEvaluated(promise) {
      PromiseThen(promise, undefined, reportException);
    },
    moduleSettle(promise, resolve, reject) {
      PromiseThen(promise, resolve, reject);
    },
    cssModule,
    // A JSON, CSS or text module the engine gave an `import()`, imported
    // by its name with its type.
    importWith(name, type) {
      return import(name, { with: { type } });
    },
    reset() {
      native.wasmReset();
      if (chunkParts.wasm !== undefined) chunkParts.wasm.reset();
      reporting = false;
      currentEvent = undefined;
      MapClear(clicking);
    },
  };
});
