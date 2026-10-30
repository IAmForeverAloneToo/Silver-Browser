// WebAssembly (the WebAssembly JavaScript Interface, M29): a binding
// chunk (D64), compiled apart from the bootstrap and read into a realm
// only when a page first uses `WebAssembly`. A function of what of the
// bootstrap's it uses, as the bootstrap names it, giving what the
// bootstrap calls of it. The bootstrap's rules hold here: what is kept of
// the built-ins was kept before any page ran, nothing looked up on a
// built-in's prototype when a page calls in, no iterator.
(function (env) {
  "use strict";
  const {
    $Error, $Map, $RangeError, $TypeError, $Uint8Array, $WeakMap, ArrayPush, BigIntAsIntN,
    FunctionBind, MapDelete, MapForEach, MapGet, MapSet, MathTrunc, ObjectCreate, ObjectFreeze,
    ObjectPrototype, PromiseThen, ReflectApply, ResponseArrayBuffer, SetFromBase64, StringIndexOf,
    StringSlice, ToBase64, ToLowerCase, ToWellFormed, WeakMapGet, WeakMapSet, bufferBytes,
    bufferLength, combined, defineProperty, getOwnPropertyDescriptor, getOwnPropertyNames, global,
    headersState, isResponse, iteratorSymbol, native, rejected, resolved, responseState, toString,
    toStringTag, viewLength,
  } = env;

  // ---- WebAssembly (the WebAssembly JavaScript Interface, M29) ------------

  // The namespace, its members defined as Web IDL defines a namespace's:
  // its operations enumerable, its classes not.
  const WebAssemblyNamespace = ObjectCreate(ObjectPrototype);
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

  return {
    // The calls under way forgotten after a run its budget stopped.
    reset() {
      wasmCalls = 0;
    },
  };
})
