// WebGPU and WebGL's bindings (M41, docs/design/gpu.md): compiled apart
// from the bootstrap and read into a realm only when a page first uses
// them (T4.3: read with the bootstrap, they cost every realm a
// millisecond more). A function of what of the bootstrap's they use, as
// the bootstrap names it, giving what the bootstrap calls of them. The
// bootstrap's rules hold here: nothing looked up on a built-in's
// prototype when a page calls in, no iterator.
(function (env) {
  "use strict";
  const {
    $ArrayBuffer, $DOMException, $DataView, $Map, $MapForEach, $Number, $NumberIsFinite, $Promise,
    $RangeError, $Set, $TypeError, $Uint8Array, $WeakMap, ArrayBufferIsView, ArrayIsArray,
    ArrayPush, ArraySlice, DataViewBuffer, DataViewLength, DataViewOffset, Event, EventTarget,
    FromCharCode, JSONParse, JSONStringify, KEY, MapDelete, MapGet, MapSet, MathFloor, MathMax,
    MathMin, MathTrunc, Navigator, ObjectFreeze, PromiseThen, ReflectApply, STORAGE, SetAdd,
    SetForEach, SetSize, StringSlice, ToBase64, TypedArrayBuffer, TypedArrayCount,
    TypedArrayLength, TypedArrayOffset, TypedArrayPrototype, TypedArraySet, TypedArrayTag, WORKER,
    WeakMapGet, WeakMapSet, binaryString, canvasTargets, constants, defineHandlers, defineProperty,
    dispatching, domError, entry, eventState, expose, fireAt, getOwnPropertyDescriptor, global,
    handleOf, illegalConstructor, illegalInvocation, imageDatas, interfaces, isNode, items,
    iteratorSymbol, kindOf, label, limits, listed, location, made, make, member, native, need,
    needToConstruct, nullable, objects, prefix, record, rejected, resolved, sequence, startTimer,
    str, toStringTag, uncurry, viewLength, write,
  } = env;


  // ---- WebGPU (M41 step 2, D39, docs/design/gpu.md) ----------------------
  // The objects are the engine's numbers for the GPU process's; each call
  // goes through one native, `native.gpu(op, bytes)`, and what the GPU
  // process answers later comes back as a task (`gpuTask`).

  const GpuSetHas = uncurry(Set.prototype.has);
  const GpuSetValues = uncurry(Set.prototype.values);
  const GpuSetEntries = uncurry(Set.prototype.entries);
  const TypedArraySubarray = uncurry(TypedArrayPrototype.subarray);

  // An ArrayBufferView's bytes, and the size of its elements.
  function gpuViewBytes(view) {
    if (view instanceof $DataView) {
      return [new $Uint8Array(DataViewBuffer(view), DataViewOffset(view), DataViewLength(view)), 1];
    }
    const length = viewLength(view);
    const count = TypedArrayCount(view);
    return [new $Uint8Array(TypedArrayBuffer(view), TypedArrayOffset(view), length),
      count === 0 ? 1 : length / count];
  }

  function gpuCall(op, bytes) {
    const pair = native.gpu(JSONStringify(op), bytes === null || bytes === undefined ? null : binaryString(bytes));
    const answer = JSONParse(pair[0]);
    if (answer.error !== undefined) {
      throw new $DOMException(answer.error.message, answer.error.name);
    }
    return { result: answer.result, bytes: pair[1] };
  }

  // The promises waiting for the GPU process, by number.
  const gpuPromises = new $Map();
  let gpuNextPromise = 1;
  function gpuPromise(op) {
    const promise = gpuNextPromise++;
    op.promise = promise;
    return new $Promise((resolve, reject) => {
      MapSet(gpuPromises, promise, { resolve, reject });
      try {
        gpuCall(op, null);
      } catch (e) {
        MapDelete(gpuPromises, promise);
        reject(e);
      }
    });
  }

  // `promise` with what `then` makes of its value.
  function gpuThen(promise, then) {
    return PromiseThen(promise, then);
  }

  // Web IDL's [EnforceRange] for an integer of `type`'s range.
  function gpuEnforced(value, where, type, max) {
    const n = typeof value === "number" ? value : $Number(value);
    if (n !== n || n === Infinity || n === -Infinity || MathTrunc(n) < 0 || MathTrunc(n) > max) {
      throw new $TypeError(where + "Value is outside the '" + type + "' value range.");
    }
    return MathTrunc(n);
  }
  const GPU_LONG = 4294967295;
  const GPU_LONG_LONG = 9007199254740991;

  // A dictionary member's words, as the reference's.
  function memberWords(method, iface, member, dict) {
    return prefix(method, iface) + "Failed to read the '" + member + "' property from '" + dict + "': ";
  }

  function gpuDictionary(value, method, iface, dict) {
    if (value === undefined || value === null) return {};
    if (typeof value !== "object" && typeof value !== "function") {
      throw new $TypeError(prefix(method, iface) + "The provided value is not of type '" + dict + "'.");
    }
    return value;
  }

  // ---- Step 3's objects and dictionaries --------------------------------
  // Each object's record: its kind, its number in the GPU process (null
  // for an encoder, or an invalid command buffer), its label, and what
  // else its kind keeps.
  const gpuThings = new $WeakMap();
  function gpuThing(object, kind, id, record) {
    const r = record === undefined ? {} : record;
    r.kind = kind;
    r.id = id;
    r.label = "";
    WeakMapSet(gpuThings, object, r);
    return object;
  }

  // `object`'s record, when it is one of `kinds` (a name or a list).
  function gpuRecordOf(object, kinds) {
    const r = (typeof object === "object" && object !== null) ? WeakMapGet(gpuThings, object) : undefined;
    if (r === undefined || (typeof kinds === "string" ? r.kind !== kinds : !listed(kinds, r.kind))) {
      throw illegalInvocation();
    }
    return r;
  }

  // `value`'s number, when it is an object of interface `kind`.
  function gpuNumberOf(value, kind, words) {
    const r = (typeof value === "object" && value !== null) ? WeakMapGet(gpuThings, value) : undefined;
    if (r === undefined || r.kind !== kind) {
      throw new $TypeError(words + "Failed to convert value to '" + kind + "'.");
    }
    return r.id;
  }

  // Interface `kind`'s label (GPUObjectBase), and the interface exposed.
  function gpuLabelled(cls, kind) {
    defineProperty(cls.prototype, "label", {
      get() { return gpuRecordOf(this, kind).label; },
      set(value) { gpuRecordOf(this, kind).label = str(value); },
      enumerable: true,
      configurable: true,
    });
    expose(cls, kind);
  }

  // A descriptor's label given to the object made of it.
  function gpuNamed(object, r) {
    if (r.d.label !== undefined) WeakMapGet(gpuThings, object).label = str(r.d.label);
    return object;
  }

  // A validation error the bootstrap found on `device`, which the GPU
  // process reports as `wgpu`'s.
  function gpuInvalid(device, message) {
    gpuCall({ op: "invalid", device, message }, null);
  }

  // A dictionary being read, `words` before its members' errors.
  function gpuReader(value, words, dict) {
    if (value !== undefined && value !== null && typeof value !== "object" && typeof value !== "function") {
      throw new $TypeError(words + "The provided value is not of type '" + dict + "'.");
    }
    return { d: value === undefined || value === null ? {} : value, words, dict };
  }

  function gpuWords(r, member) {
    return r.words + "Failed to read the '" + member + "' property from '" + r.dict + "': ";
  }

  function gpuNeed(r, member) {
    const value = r.d[member];
    if (value === undefined) throw new $TypeError(gpuWords(r, member) + "Required member is undefined.");
    return value;
  }

  // A member's value: required when `fallback` is undefined, else
  // `fallback` when it is absent (null for an optional member).
  function gpuValue(r, member, fallback) {
    return fallback === undefined ? gpuNeed(r, member) : r.d[member];
  }

  function gpuU32(r, member, fallback) {
    const value = gpuValue(r, member, fallback);
    return value === undefined ? fallback : gpuEnforced(value, gpuWords(r, member), "unsigned long", GPU_LONG);
  }

  function gpuU64(r, member, fallback) {
    const value = gpuValue(r, member, fallback);
    return value === undefined ? fallback : gpuEnforced(value, gpuWords(r, member), "unsigned long long", GPU_LONG_LONG);
  }

  // Web IDL's [EnforceRange] long.
  function gpuSigned(value, words) {
    const n = typeof value === "number" ? value : $Number(value);
    if (n !== n || n === Infinity || n === -Infinity || MathTrunc(n) < -2147483648 || MathTrunc(n) > 2147483647) {
      throw new $TypeError(words + "Value is outside the 'long' value range.");
    }
    return MathTrunc(n);
  }

  function gpuI32(r, member, fallback) {
    const value = gpuValue(r, member, fallback);
    return value === undefined ? fallback : gpuSigned(value, gpuWords(r, member));
  }

  // A restricted float or double.
  function gpuFinite(value, words, type) {
    const n = typeof value === "number" ? value : $Number(value);
    if (!$NumberIsFinite(n)) throw new $TypeError(words + "The provided " + type + " value is non-finite.");
    return n;
  }

  function gpuFloat(r, member, fallback, type) {
    const value = gpuValue(r, member, fallback);
    return value === undefined ? fallback : gpuFinite(value, gpuWords(r, member), type === undefined ? "double" : type);
  }

  function gpuBool(r, member, fallback) {
    const value = r.d[member];
    return value === undefined ? fallback : !!value;
  }

  function gpuEnumValue(value, list, words, type) {
    const s = str(value);
    if (!listed(list, s)) {
      throw new $TypeError(words + "The provided value '" + s + "' is not a valid enum value of type " + type + ".");
    }
    return s;
  }

  function gpuEnum(r, member, list, type, fallback) {
    const value = gpuValue(r, member, fallback);
    return value === undefined ? fallback : gpuEnumValue(value, list, gpuWords(r, member), type);
  }

  // A member that is an object of interface `kind`: its number, or null
  // when optional and absent.
  function gpuMemberNumber(r, member, kind, optional) {
    const value = r.d[member];
    if (value === undefined && optional) return null;
    return gpuNumberOf(gpuNeed(r, member), kind, gpuWords(r, member));
  }

  // A member that is a dictionary: its reader, or null when absent.
  function gpuSub(r, member, dict, required) {
    const value = required ? gpuNeed(r, member) : r.d[member];
    return value === undefined ? null : gpuReader(value, gpuWords(r, member), dict);
  }

  // Web IDL's sequence, with `words` before its errors.
  function gpuSequence(value, words) {
    const object = (typeof value === "object" && value !== null) || typeof value === "function";
    const iterate = object ? value[iteratorSymbol] : undefined;
    if (typeof iterate !== "function") {
      throw new $TypeError(words + "The provided value cannot be converted to a sequence.");
    }
    const iterator = ReflectApply(iterate, value, []);
    if ((typeof iterator !== "object" || iterator === null) && typeof iterator !== "function") {
      throw new $TypeError(words + "The iterator is not an object.");
    }
    const next = iterator.next;
    const items = [];
    for (;;) {
      const step = ReflectApply(next, iterator, []);
      if ((typeof step !== "object" || step === null) && typeof step !== "function") {
        throw new $TypeError(words + "The iterator's result is not an object.");
      }
      if (step.done) return items;
      ArrayPush(items, step.value);
    }
  }

  function gpuList(r, member, required) {
    const value = required ? gpuNeed(r, member) : r.d[member];
    return value === undefined ? [] : gpuSequence(value, gpuWords(r, member));
  }

  // Whether a union's value is its sequence (it has @@iterator).
  function gpuIsSequence(value) {
    return ((typeof value === "object" && value !== null) || typeof value === "function") &&
      value[iteratorSymbol] !== undefined;
  }

  // A GPUExtent3D, as [width, height, depthOrArrayLayers].
  function gpuExtent(value, words) {
    if (gpuIsSequence(value)) {
      const list = gpuSequence(value, words);
      for (let i = 0; i < list.length; i++) list[i] = gpuEnforced(list[i], words, "unsigned long", GPU_LONG);
      if (list.length < 1 || list.length > 3) {
        throw new $TypeError(words + "A sequence of number used as a GPUExtent3D must have between 1 and 3 elements.");
      }
      return [list[0], list.length > 1 ? list[1] : 1, list.length > 2 ? list[2] : 1];
    }
    const r = gpuReader(value, words, "GPUExtent3DDict");
    const depth = gpuU32(r, "depthOrArrayLayers", 1);
    const height = gpuU32(r, "height", 1);
    return [gpuU32(r, "width"), height, depth];
  }

  // A GPUOrigin3D, as [x, y, z].
  function gpuOrigin(value, words) {
    if (value === undefined) return [0, 0, 0];
    if (gpuIsSequence(value)) {
      const list = gpuSequence(value, words);
      for (let i = 0; i < list.length; i++) list[i] = gpuEnforced(list[i], words, "unsigned long", GPU_LONG);
      if (list.length > 3) {
        throw new $TypeError(words + "A sequence of number used as a GPUOrigin3D must have at most 3 elements.");
      }
      return [list.length > 0 ? list[0] : 0, list.length > 1 ? list[1] : 0, list.length > 2 ? list[2] : 0];
    }
    const r = gpuReader(value, words, "GPUOrigin3DDict");
    return [gpuU32(r, "x", 0), gpuU32(r, "y", 0), gpuU32(r, "z", 0)];
  }

  // A GPUColor, as [r, g, b, a].
  function gpuColor(value, words) {
    if (gpuIsSequence(value)) {
      const list = gpuSequence(value, words);
      for (let i = 0; i < list.length; i++) list[i] = gpuFinite(list[i], words, "double");
      if (list.length !== 4) {
        throw new $TypeError(words + "A sequence of number used as a GPUColor must have exactly 4 elements.");
      }
      return list;
    }
    const r = gpuReader(value, words, "GPUColorDict");
    const a = gpuFloat(r, "a");
    const b = gpuFloat(r, "b");
    const g = gpuFloat(r, "g");
    return [gpuFloat(r, "r"), g, b, a];
  }

  // WebGPU's enums, as `gpu::convert` reads them in the GPU process.
  const GPU_FORMATS = [
    "r8unorm", "r8snorm", "r8uint", "r8sint", "r16unorm", "r16snorm", "r16uint", "r16sint",
    "r16float", "rg8unorm", "rg8snorm", "rg8uint", "rg8sint", "r32uint", "r32sint", "r32float",
    "rg16unorm", "rg16snorm", "rg16uint", "rg16sint", "rg16float", "rgba8unorm",
    "rgba8unorm-srgb", "rgba8snorm", "rgba8uint", "rgba8sint", "bgra8unorm", "bgra8unorm-srgb",
    "rgb9e5ufloat", "rgb10a2uint", "rgb10a2unorm", "rg11b10ufloat", "rg32uint", "rg32sint",
    "rg32float", "rgba16unorm", "rgba16snorm", "rgba16uint", "rgba16sint", "rgba16float",
    "rgba32uint", "rgba32sint", "rgba32float", "stencil8", "depth16unorm", "depth24plus",
    "depth24plus-stencil8", "depth32float", "depth32float-stencil8", "bc1-rgba-unorm",
    "bc1-rgba-unorm-srgb", "bc2-rgba-unorm", "bc2-rgba-unorm-srgb", "bc3-rgba-unorm",
    "bc3-rgba-unorm-srgb", "bc4-r-unorm", "bc4-r-snorm", "bc5-rg-unorm", "bc5-rg-snorm",
    "bc6h-rgb-ufloat", "bc6h-rgb-float", "bc7-rgba-unorm", "bc7-rgba-unorm-srgb",
    "etc2-rgb8unorm", "etc2-rgb8unorm-srgb", "etc2-rgb8a1unorm", "etc2-rgb8a1unorm-srgb",
    "etc2-rgba8unorm", "etc2-rgba8unorm-srgb", "eac-r11unorm", "eac-r11snorm", "eac-rg11unorm",
    "eac-rg11snorm", "astc-4x4-unorm", "astc-4x4-unorm-srgb", "astc-5x4-unorm",
    "astc-5x4-unorm-srgb", "astc-5x5-unorm", "astc-5x5-unorm-srgb", "astc-6x5-unorm",
    "astc-6x5-unorm-srgb", "astc-6x6-unorm", "astc-6x6-unorm-srgb", "astc-8x5-unorm",
    "astc-8x5-unorm-srgb", "astc-8x6-unorm", "astc-8x6-unorm-srgb", "astc-8x8-unorm",
    "astc-8x8-unorm-srgb", "astc-10x5-unorm", "astc-10x5-unorm-srgb", "astc-10x6-unorm",
    "astc-10x6-unorm-srgb", "astc-10x8-unorm", "astc-10x8-unorm-srgb", "astc-10x10-unorm",
    "astc-10x10-unorm-srgb", "astc-12x10-unorm", "astc-12x10-unorm-srgb", "astc-12x12-unorm",
    "astc-12x12-unorm-srgb",
  ];
  const GPU_VERTEX_FORMATS = [
    "uint8", "uint8x2", "uint8x4", "sint8", "sint8x2", "sint8x4", "unorm8", "unorm8x2",
    "unorm8x4", "snorm8", "snorm8x2", "snorm8x4", "uint16", "uint16x2", "uint16x4", "sint16",
    "sint16x2", "sint16x4", "unorm16", "unorm16x2", "unorm16x4", "snorm16", "snorm16x2",
    "snorm16x4", "float16", "float16x2", "float16x4", "float32", "float32x2", "float32x3",
    "float32x4", "uint32", "uint32x2", "uint32x3", "uint32x4", "sint32", "sint32x2", "sint32x3",
    "sint32x4", "unorm10-10-10-2", "unorm8x4-bgra",
  ];
  const GPU_DEPTH_FORMATS = ["depth16unorm", "depth24plus", "depth24plus-stencil8", "depth32float",
    "depth32float-stencil8"];
  const GPU_STENCIL_FORMATS = ["stencil8", "depth24plus-stencil8", "depth32float-stencil8"];
  const GPU_TEXTURE_DIMENSIONS = ["1d", "2d", "3d"];
  const GPU_VIEW_DIMENSIONS = ["1d", "2d", "2d-array", "cube", "cube-array", "3d"];
  const GPU_ASPECTS = ["all", "stencil-only", "depth-only"];
  const GPU_ADDRESS_MODES = ["clamp-to-edge", "repeat", "mirror-repeat"];
  const GPU_FILTERS = ["nearest", "linear"];
  const GPU_COMPARES = ["never", "less", "equal", "less-equal", "greater", "not-equal",
    "greater-equal", "always"];
  const GPU_BUFFER_BINDINGS = ["uniform", "storage", "read-only-storage"];
  const GPU_SAMPLER_BINDINGS = ["filtering", "non-filtering", "comparison"];
  const GPU_SAMPLE_TYPES = ["float", "unfilterable-float", "depth", "sint", "uint"];
  const GPU_STORAGE_ACCESSES = ["write-only", "read-only", "read-write"];
  const GPU_TOPOLOGIES = ["point-list", "line-list", "line-strip", "triangle-list", "triangle-strip"];
  const GPU_INDEX_FORMATS = ["uint16", "uint32"];
  const GPU_FRONT_FACES = ["ccw", "cw"];
  const GPU_CULL_MODES = ["none", "front", "back"];
  const GPU_STEP_MODES = ["vertex", "instance"];
  const GPU_BLEND_FACTORS = ["zero", "one", "src", "one-minus-src", "src-alpha",
    "one-minus-src-alpha", "dst", "one-minus-dst", "dst-alpha", "one-minus-dst-alpha",
    "src-alpha-saturated", "constant", "one-minus-constant", "src1", "one-minus-src1",
    "src1-alpha", "one-minus-src1-alpha"];
  const GPU_BLEND_OPERATIONS = ["add", "subtract", "reverse-subtract", "min", "max"];
  const GPU_STENCIL_OPERATIONS = ["keep", "zero", "replace", "invert", "increment-clamp",
    "decrement-clamp", "increment-wrap", "decrement-wrap"];
  const GPU_LOAD_OPS = ["load", "clear"];
  const GPU_STORE_OPS = ["store", "discard"];
  const GPU_QUERY_TYPES = ["occlusion", "timestamp"];

  // ---- The set-likes: features, WGSL's language features ----
  class GPUSupportedFeatures {
    #names;
    constructor(key, names) {
      if (key !== KEY) throw illegalConstructor("GPUSupportedFeatures");
      this.#names = new $Set();
      for (let i = 0; i < names.length; i++) SetAdd(this.#names, names[i]);
    }
    get size() { return SetSize(this.#names); }
    has(name) { return GpuSetHas(this.#names, str(name)); }
    forEach(callback, thisArg) {
      need(arguments.length, 1, "forEach", "GPUSupportedFeatures");
      SetForEach(this.#names, (name) => ReflectApply(callback, thisArg, [name, name, this]));
    }
    values() { return GpuSetValues(this.#names); }
    keys() { return GpuSetValues(this.#names); }
    entries() { return GpuSetEntries(this.#names); }
    [iteratorSymbol]() { return GpuSetValues(this.#names); }
  }
  expose(GPUSupportedFeatures, "GPUSupportedFeatures");

  class WGSLLanguageFeatures {
    #names;
    constructor(key, names) {
      if (key !== KEY) throw illegalConstructor("WGSLLanguageFeatures");
      this.#names = new $Set();
      for (let i = 0; i < names.length; i++) SetAdd(this.#names, names[i]);
    }
    get size() { return SetSize(this.#names); }
    has(name) { return GpuSetHas(this.#names, str(name)); }
    forEach(callback, thisArg) {
      need(arguments.length, 1, "forEach", "WGSLLanguageFeatures");
      SetForEach(this.#names, (name) => ReflectApply(callback, thisArg, [name, name, this]));
    }
    values() { return GpuSetValues(this.#names); }
    keys() { return GpuSetValues(this.#names); }
    entries() { return GpuSetEntries(this.#names); }
    [iteratorSymbol]() { return GpuSetValues(this.#names); }
  }
  expose(WGSLLanguageFeatures, "WGSLLanguageFeatures");

  // The limits, by WebGPU's names, in the standard's order.
  const GPU_LIMITS = [
    "maxTextureDimension1D", "maxTextureDimension2D", "maxTextureDimension3D",
    "maxTextureArrayLayers", "maxBindGroups", "maxBindGroupsPlusVertexBuffers",
    "maxBindingsPerBindGroup", "maxDynamicUniformBuffersPerPipelineLayout",
    "maxDynamicStorageBuffersPerPipelineLayout", "maxSampledTexturesPerShaderStage",
    "maxSamplersPerShaderStage", "maxStorageBuffersPerShaderStage",
    "maxStorageTexturesPerShaderStage", "maxUniformBuffersPerShaderStage",
    "maxUniformBufferBindingSize", "maxStorageBufferBindingSize",
    "minUniformBufferOffsetAlignment", "minStorageBufferOffsetAlignment",
    "maxVertexBuffers", "maxBufferSize", "maxVertexAttributes",
    "maxVertexBufferArrayStride", "maxInterStageShaderVariables", "maxColorAttachments",
    "maxColorAttachmentBytesPerSample", "maxComputeWorkgroupStorageSize",
    "maxComputeInvocationsPerWorkgroup", "maxComputeWorkgroupSizeX",
    "maxComputeWorkgroupSizeY", "maxComputeWorkgroupSizeZ",
    "maxComputeWorkgroupsPerDimension",
  ];
  // The limits whose smaller value is the better (alignments).
  const GPU_MIN_LIMITS = ["minUniformBufferOffsetAlignment", "minStorageBufferOffsetAlignment"];

  class GPUSupportedLimits {
    #values;
    constructor(key, pairs) {
      if (key !== KEY) throw illegalConstructor("GPUSupportedLimits");
      this.#values = new $Map();
      for (let i = 0; i < pairs.length; i++) MapSet(this.#values, pairs[i][0], pairs[i][1]);
    }
    static {
      for (let i = 0; i < GPU_LIMITS.length; i++) {
        const name = GPU_LIMITS[i];
        defineProperty(GPUSupportedLimits.prototype, name, {
          get() {
            const value = MapGet(this.#values, name);
            return value === undefined ? 0 : value;
          },
          enumerable: true,
          configurable: true,
        });
      }
    }
  }
  expose(GPUSupportedLimits, "GPUSupportedLimits");

  class GPUAdapterInfo {
    #vendor; #fallback;
    constructor(key, vendor, fallback) {
      if (key !== KEY) throw illegalConstructor("GPUAdapterInfo");
      this.#vendor = vendor;
      this.#fallback = fallback;
    }
    get vendor() { return this.#vendor; }
    get architecture() { return ""; }
    get device() { return ""; }
    get description() { return ""; }
    get subgroupMinSize() { return 4; }
    get subgroupMaxSize() { return 128; }
    get isFallbackAdapter() { return this.#fallback; }
  }
  expose(GPUAdapterInfo, "GPUAdapterInfo");

  // ---- The errors ----
  class GPUError {
    #message;
    constructor(message) {
      if (new.target === GPUError) throw illegalConstructor("GPUError");
      this.#message = str(message);
    }
    get message() { return this.#message; }
  }
  expose(GPUError, "GPUError");

  class GPUValidationError extends GPUError {
    constructor(message) {
      needToConstruct(arguments.length, 1, "GPUValidationError");
      super(message);
    }
  }
  expose(GPUValidationError, "GPUValidationError");

  class GPUOutOfMemoryError extends GPUError {
    constructor(message) {
      needToConstruct(arguments.length, 1, "GPUOutOfMemoryError");
      super(message);
    }
  }
  expose(GPUOutOfMemoryError, "GPUOutOfMemoryError");

  class GPUInternalError extends GPUError {
    constructor(message) {
      needToConstruct(arguments.length, 1, "GPUInternalError");
      super(message);
    }
  }
  expose(GPUInternalError, "GPUInternalError");

  // An error the GPU process told of, as the page sees it.
  function gpuError(told) {
    const message = told.message + "\n";
    if (told.kind === 1) return new GPUOutOfMemoryError(message);
    if (told.kind === 2) return new GPUInternalError(message);
    return new GPUValidationError(message);
  }

  class GPUUncapturedErrorEvent extends Event {
    #error;
    constructor(type, init) {
      needToConstruct(arguments.length, 2, "GPUUncapturedErrorEvent");
      const words = "Failed to construct 'GPUUncapturedErrorEvent': Failed to read the 'error' property from 'GPUUncapturedErrorEventInit': ";
      if (init === null || typeof init !== "object" || init.error === undefined) {
        throw new $TypeError(words + "Required member is undefined.");
      }
      if (!(init.error instanceof GPUError)) {
        throw new $TypeError(words + "Failed to convert value to 'GPUError'.");
      }
      super(type, init);
      this.#error = init.error;
    }
    get error() { return this.#error; }
  }
  expose(GPUUncapturedErrorEvent, "GPUUncapturedErrorEvent");

  class GPUDeviceLostInfo {
    #reason; #message;
    constructor(key, reason, message) {
      if (key !== KEY) throw illegalConstructor("GPUDeviceLostInfo");
      this.#reason = reason;
      this.#message = message;
    }
    get reason() { return this.#reason; }
    get message() { return this.#message; }
  }
  expose(GPUDeviceLostInfo, "GPUDeviceLostInfo");

  // ---- The flags ----
  const GPU_NAMESPACES = [
    ["GPUBufferUsage", {
      MAP_READ: 0x1, MAP_WRITE: 0x2, COPY_SRC: 0x4, COPY_DST: 0x8, INDEX: 0x10,
      VERTEX: 0x20, UNIFORM: 0x40, STORAGE: 0x80, INDIRECT: 0x100, QUERY_RESOLVE: 0x200,
    }],
    ["GPUMapMode", { READ: 0x1, WRITE: 0x2 }],
    ["GPUTextureUsage", {
      COPY_SRC: 0x1, COPY_DST: 0x2, TEXTURE_BINDING: 0x4, STORAGE_BINDING: 0x8,
      RENDER_ATTACHMENT: 0x10,
    }],
    ["GPUShaderStage", { VERTEX: 0x1, FRAGMENT: 0x2, COMPUTE: 0x4 }],
    ["GPUColorWrite", { RED: 0x1, GREEN: 0x2, BLUE: 0x4, ALPHA: 0x8, ALL: 0xF }],
  ];
  if (!WORKER) {
    for (let i = 0; i < GPU_NAMESPACES.length; i++) {
      defineProperty(global, GPU_NAMESPACES[i][0], {
        value: ObjectFreeze(GPU_NAMESPACES[i][1]),
        writable: true,
        configurable: true,
      });
    }
  }

  // ---- GPU and GPUAdapter ----
  // What naga, this build's WGSL front end, implements.
  const WGSL_FEATURES = [
    "packed_4x8_integer_dot_product", "pointer_composite_access",
    "readonly_and_readwrite_storage_textures",
  ];

  const GPU_FEATURES = [
    "core-features-and-limits", "depth-clip-control", "depth32float-stencil8",
    "texture-compression-bc", "texture-compression-bc-sliced-3d", "texture-compression-etc2",
    "texture-compression-astc", "texture-compression-astc-sliced-3d", "timestamp-query",
    "indirect-first-instance", "shader-f16", "rg11b10ufloat-renderable", "bgra8unorm-storage",
    "float32-filterable", "float32-blendable", "dual-source-blending", "clip-distances",
    "subgroups", "texture-formats-tier1", "texture-formats-tier2", "primitive-index",
    "texture-component-swizzle",
  ];

  class GPU {
    #wgsl;
    constructor(key) {
      if (key !== KEY) throw illegalConstructor("GPU");
      this.#wgsl = new WGSLLanguageFeatures(KEY, WGSL_FEATURES);
    }
    requestAdapter(options) {
      let o;
      try {
        o = gpuDictionary(options, "requestAdapter", "GPU", "GPURequestAdapterOptions");
      } catch (e) {
        return rejected(e);
      }
      const preference = o.powerPreference === undefined ? "" : str(o.powerPreference);
      if (preference !== "" && preference !== "low-power" && preference !== "high-performance") {
        return rejected(new $TypeError(memberWords("requestAdapter", "GPU", "powerPreference", "GPURequestAdapterOptions") +
          "The provided value '" + preference + "' is not a valid enum value of type GPUPowerPreference."));
      }
      return gpuPromise({
        op: "requestAdapter",
        power: preference === "low-power" ? 1 : preference === "high-performance" ? 2 : 0,
        fallback: !!o.forceFallbackAdapter,
      });
    }
    getPreferredCanvasFormat() { return "bgra8unorm"; }
    get wgslLanguageFeatures() { return this.#wgsl; }
  }
  expose(GPU, "GPU");

  let gpuObject = null;
  if (!WORKER) {
    defineProperty(Navigator.prototype, "gpu", {
      get() {
        if (!native.secureContext()) return undefined;
        if (gpuObject === null) gpuObject = new GPU(KEY);
        return gpuObject;
      },
      enumerable: true,
      configurable: true,
    });
  }

  let gpuDeviceId;
  let gpuDeviceLost;
  let gpuBufferId;

  class GPUAdapter {
    #id; #features; #limits; #info; #consumed = false;
    constructor(key, told) {
      if (key !== KEY) throw illegalConstructor("GPUAdapter");
      this.#id = told.id;
      this.#features = new GPUSupportedFeatures(KEY, told.features);
      this.#limits = new GPUSupportedLimits(KEY, told.limits);
      this.#info = new GPUAdapterInfo(KEY, told.vendor, told.fallback);
    }
    get features() { return this.#features; }
    get limits() { return this.#limits; }
    get info() { return this.#info; }
    get isFallbackAdapter() { return this.#info.isFallbackAdapter; }

    requestDevice(descriptor) {
      try {
        return this.#requestDevice(descriptor);
      } catch (e) {
        return rejected(e);
      }
    }

    #requestDevice(descriptor) {
      const words = prefix("requestDevice", "GPUAdapter");
      const d = gpuDictionary(descriptor, "requestDevice", "GPUAdapter", "GPUDeviceDescriptor");
      const features = [];
      if (d.requiredFeatures !== undefined) {
        const names = sequence(d.requiredFeatures, "requestDevice", "GPUAdapter", str);
        for (let i = 0; i < names.length; i++) {
          if (!listed(GPU_FEATURES, names[i])) {
            throw new $TypeError(words + "Failed to read the 'requiredFeatures' property from 'GPUDeviceDescriptor': The provided value '" +
              names[i] + "' is not a valid enum value of type GPUFeatureName.");
          }
          if (!this.#features.has(names[i])) {
            throw new $TypeError(words + "Unsupported feature: " + names[i]);
          }
          ArrayPush(features, names[i]);
        }
      }
      const limits = [];
      if (d.requiredLimits !== undefined && d.requiredLimits !== null) {
        const pairs = record(d.requiredLimits, str, (value) => value);
        for (let i = 0; i < pairs.length; i++) {
          const name = pairs[i][0];
          if (pairs[i][1] === undefined) continue;
          if (!listed(GPU_LIMITS, name)) {
            throw domError("OperationError", "The limit '" + name + "' is not recognized.", "requestDevice", "GPUAdapter");
          }
          const n = gpuEnforced(pairs[i][1], words, "unsigned long long", GPU_LONG_LONG);
          const supported = this.#limits[name];
          const alignment = listed(GPU_MIN_LIMITS, name);
          if (alignment ? n < supported : n > supported) {
            throw domError("OperationError", "Required limit (" + n + ") is " +
              (alignment ? "lower" : "greater") + " than the supported limit (" + supported +
              ").\n - While validating " + name + "\n - While validating required limits", "requestDevice", "GPUAdapter");
          }
          if (alignment && (n & (n - 1)) !== 0) {
            throw domError("OperationError", "Required limit (" + n + ") is not a power of two.", "requestDevice", "GPUAdapter");
          }
          ArrayPush(limits, [name, n]);
        }
      }
      if (this.#consumed) {
        throw domError("OperationError", "adapter is \"consumed\": it has already been used to create a device", "requestDevice", "GPUAdapter");
      }
      this.#consumed = true;
      const info = this.#info;
      const label = d.label === undefined ? "" : str(d.label);
      return gpuThen(gpuPromise({ op: "requestDevice", adapter: this.#id, features, limits }),
        (told) => {
          const device = new GPUDevice(KEY, told, info);
          device.label = label;
          return device;
        });
    }
  }
  expose(GPUAdapter, "GPUAdapter");

  // ---- GPUDevice and GPUQueue ----
  const gpuDevices = new $Map();

  class GPUDevice extends EventTarget {
    #id; #features; #limits; #info; #queue; #lost; #resolveLost; #destroyed = false; #label = "";
    constructor(key, told, info) {
      if (key !== KEY) throw illegalConstructor("GPUDevice");
      super();
      this.#id = told.id;
      this.#features = new GPUSupportedFeatures(KEY, told.features);
      this.#limits = new GPUSupportedLimits(KEY, told.limits);
      this.#info = info;
      this.#queue = new GPUQueue(KEY, this.#id);
      this.#lost = new $Promise((resolve) => { this.#resolveLost = resolve; });
      MapSet(gpuDevices, this.#id, this);
    }
    static {
      gpuDeviceId = (device) => device.#id;
      gpuDeviceLost = (device, reason, message) => {
        if (device.#resolveLost === null) return;
        const resolve = device.#resolveLost;
        device.#resolveLost = null;
        MapDelete(gpuDevices, device.#id);
        resolve(new GPUDeviceLostInfo(KEY, reason, message));
      };
    }
    get features() { return this.#features; }
    get limits() { return this.#limits; }
    get adapterInfo() { return this.#info; }
    get queue() { return this.#queue; }
    get lost() { return this.#lost; }
    get label() { return this.#label; }
    set label(value) { this.#label = str(value); }

    destroy() {
      if (this.#destroyed) return;
      this.#destroyed = true;
      gpuCall({ op: "destroyDevice", device: this.#id }, null);
    }

    pushErrorScope(filter) {
      need(arguments.length, 1, "pushErrorScope", "GPUDevice");
      const f = str(filter);
      const n = f === "validation" ? 0 : f === "out-of-memory" ? 1 : f === "internal" ? 2 : -1;
      if (n < 0) {
        throw new $TypeError(prefix("pushErrorScope", "GPUDevice") + "The provided value '" + f +
          "' is not a valid enum value of type GPUErrorFilter.");
      }
      gpuCall({ op: "pushErrorScope", device: this.#id, filter: n }, null);
    }

    popErrorScope() {
      return gpuThen(gpuPromise({ op: "popErrorScope", device: this.#id }),
        (told) => told === null ? null : gpuError(told));
    }

    createBuffer(descriptor) {
      need(arguments.length, 1, "createBuffer", "GPUDevice");
      const d = gpuDictionary(descriptor, "createBuffer", "GPUDevice", "GPUBufferDescriptor");
      const sizeWords = memberWords("createBuffer", "GPUDevice", "size", "GPUBufferDescriptor");
      const usageWords = memberWords("createBuffer", "GPUDevice", "usage", "GPUBufferDescriptor");
      if (d.size === undefined) throw new $TypeError(sizeWords + "Required member is undefined.");
      if (d.usage === undefined) throw new $TypeError(usageWords + "Required member is undefined.");
      const size = gpuEnforced(d.size, sizeWords, "unsigned long long", GPU_LONG_LONG);
      const usage = gpuEnforced(d.usage, usageWords, "unsigned long", GPU_LONG);
      const mapped = !!d.mappedAtCreation;
      if (mapped && size % 4 !== 0) {
        throw new $RangeError(prefix("createBuffer", "GPUDevice") + "createBuffer failed, size (" + size +
          ") is not aligned to 4 when mappedAtCreation is true.");
      }
      const told = gpuCall({ op: "createBuffer", device: this.#id, size, usage, mapped }, null).result;
      const buffer = new GPUBuffer(KEY, told.id, size, usage, mapped);
      if (d.label !== undefined) buffer.label = d.label;
      return buffer;
    }

    createTexture(descriptor) {
      need(arguments.length, 1, "createTexture", "GPUDevice");
      return gpuCreateTexture(this.#id, descriptor);
    }

    createSampler(descriptor) {
      return gpuCreateSampler(this.#id, descriptor);
    }

    createShaderModule(descriptor) {
      need(arguments.length, 1, "createShaderModule", "GPUDevice");
      return gpuCreateShaderModule(this.#id, descriptor);
    }

    createBindGroupLayout(descriptor) {
      need(arguments.length, 1, "createBindGroupLayout", "GPUDevice");
      return gpuCreateBindGroupLayout(this.#id, descriptor);
    }

    createPipelineLayout(descriptor) {
      need(arguments.length, 1, "createPipelineLayout", "GPUDevice");
      return gpuCreatePipelineLayout(this.#id, descriptor);
    }

    createBindGroup(descriptor) {
      need(arguments.length, 1, "createBindGroup", "GPUDevice");
      return gpuCreateBindGroup(this.#id, descriptor);
    }

    createComputePipeline(descriptor) {
      need(arguments.length, 1, "createComputePipeline", "GPUDevice");
      return gpuCreatePipeline(this.#id, descriptor, true, false);
    }

    createRenderPipeline(descriptor) {
      need(arguments.length, 1, "createRenderPipeline", "GPUDevice");
      return gpuCreatePipeline(this.#id, descriptor, false, false);
    }

    createComputePipelineAsync(descriptor) {
      try {
        need(arguments.length, 1, "createComputePipelineAsync", "GPUDevice");
        return gpuCreatePipeline(this.#id, descriptor, true, true);
      } catch (e) {
        return rejected(e);
      }
    }

    createRenderPipelineAsync(descriptor) {
      try {
        need(arguments.length, 1, "createRenderPipelineAsync", "GPUDevice");
        return gpuCreatePipeline(this.#id, descriptor, false, true);
      } catch (e) {
        return rejected(e);
      }
    }

    createCommandEncoder(descriptor) {
      return gpuCreateCommandEncoder(this.#id, descriptor);
    }

    createRenderBundleEncoder(descriptor) {
      need(arguments.length, 1, "createRenderBundleEncoder", "GPUDevice");
      return gpuCreateRenderBundleEncoder(this.#id, descriptor);
    }

    createQuerySet(descriptor) {
      need(arguments.length, 1, "createQuerySet", "GPUDevice");
      return gpuCreateQuerySet(this.#id, descriptor);
    }
  }
  defineHandlers(GPUDevice.prototype, ["uncapturederror"], false);
  expose(GPUDevice, "GPUDevice");

  // The bytes `data` holds from `offset` (in its elements) for `size`
  // elements, as `writeBuffer` reads them.
  function gpuDataBytes(data, offset, size, method, iface) {
    let bytes;
    let unit = 1;
    if (data instanceof $ArrayBuffer) {
      bytes = new $Uint8Array(data);
    } else if (ArrayBufferIsView(data)) {
      const view = gpuViewBytes(data);
      unit = view[1];
      bytes = view[0];
    } else {
      throw new $TypeError(prefix(method, iface) + "parameter 3 is not of type 'ArrayBuffer' or 'ArrayBufferView'.");
    }
    const elements = bytes.length / unit;
    const start = offset === undefined ? 0 : gpuEnforced(offset, prefix(method, iface), "unsigned long long", GPU_LONG_LONG);
    if (start > elements) {
      throw domError("OperationError", "dataOffset (" + start + ") is larger than data's size (" + elements + ").", method, iface);
    }
    const count = size === undefined ? elements - start : gpuEnforced(size, prefix(method, iface), "unsigned long long", GPU_LONG_LONG);
    if (start + count > elements) {
      throw domError("OperationError", "dataOffset (" + start + ") + size (" + count + ") is larger than data's size (" + elements + ").", method, iface);
    }
    if ((count * unit) % 4 !== 0) {
      throw domError("OperationError", "size (" + count * unit + " bytes) is not a multiple of 4 bytes.", method, iface);
    }
    return TypedArraySubarray(bytes, start * unit, (start + count) * unit);
  }

  // The most bytes one texture write sends: a larger one goes in pieces
  // of whole rows (the GPU process's GPU_CHUNK).
  const GPU_PIECE = 4194304;

  // `bytes` written into texture copy `dest` of `size` texels.
  function gpuWriteTexture(device, dest, bytes, layout, size, blockHeight) {
    const bytesPerRow = layout.bytesPerRow;
    const rows = MathTrunc((size[1] + blockHeight - 1) / blockHeight);
    const write = (destination, piece, rowsPerImage, extent) => gpuCall({
      op: "writeTexture",
      device,
      desc: {
        dataLayout: { bytesPerRow, rowsPerImage },
        destination,
        size: extent,
      },
    }, piece);
    if (bytes.length <= GPU_PIECE || bytesPerRow === null || rows === 0 || size[0] === 0 || size[2] === 0) {
      write(dest, bytes, layout.rowsPerImage, size);
      return;
    }
    const perImage = layout.rowsPerImage === null ? rows : layout.rowsPerImage;
    const step = MathMax(1, MathTrunc(GPU_PIECE / bytesPerRow));
    for (let z = 0; z < size[2]; z++) {
      for (let row = 0; row < rows; row += step) {
        const count = MathMin(step, rows - row);
        const start = MathMin(bytes.length, (z * perImage + row) * bytesPerRow);
        const piece = TypedArraySubarray(bytes, start, MathMin(bytes.length, start + count * bytesPerRow));
        const destination = {
          aspect: dest.aspect,
          mipLevel: dest.mipLevel,
          origin: [dest.origin[0], dest.origin[1] + row * blockHeight, dest.origin[2] + z],
          texture: dest.texture,
        };
        write(destination, piece, null, [size[0], MathMin(count * blockHeight, size[1] - row * blockHeight), 1]);
      }
    }
  }

  class GPUQueue {
    #device; #label = "";
    constructor(key, device) {
      if (key !== KEY) throw illegalConstructor("GPUQueue");
      this.#device = device;
    }
    get label() { return this.#label; }
    set label(value) { this.#label = str(value); }

    submit(buffers) {
      need(arguments.length, 1, "submit", "GPUQueue");
      const list = sequence(buffers, "submit", "GPUQueue");
      const records = [];
      for (let i = 0; i < list.length; i++) {
        const r = (typeof list[i] === "object" && list[i] !== null) ? WeakMapGet(gpuThings, list[i]) : undefined;
        if (r === undefined || r.kind !== "GPUCommandBuffer") {
          throw new $TypeError(prefix("submit", "GPUQueue") + "Failed to convert value to 'GPUCommandBuffer'.");
        }
        ArrayPush(records, r);
      }
      // A command buffer is submitted once; one invalid, none is.
      const ids = [];
      let invalid = false;
      for (let i = 0; i < records.length; i++) {
        if (records[i].id === null || records[i].submitted) invalid = true;
        records[i].submitted = true;
        ArrayPush(ids, records[i].id);
      }
      if (invalid) {
        gpuInvalid(this.#device, "A command buffer submitted is invalid, or was submitted before.");
        return;
      }
      if (ids.length !== 0) gpuCall({ op: "submit", device: this.#device, buffers: ids }, null);
    }

    copyExternalImageToTexture(source, destination, copySize) {
      need(arguments.length, 3, "copyExternalImageToTexture", "GPUQueue");
      const words = prefix("copyExternalImageToTexture", "GPUQueue");
      const src = gpuReader(source, words, "GPUCopyExternalImageSourceInfo");
      const flipY = gpuBool(src, "flipY", false);
      let origin = [0, 0];
      if (src.d.origin !== undefined) {
        const o = gpuOrigin(src.d.origin, gpuWords(src, "origin"));
        origin = [o[0], o[1]];
      }
      const image = gpuImageSource(gpuNeed(src, "source"), words);
      const dest = gpuTexelTexture(destination, words);
      const texture = gpuTexelRecord;
      const premultiplied = !!destination.premultipliedAlpha;
      const extent = gpuExtent(copySize, words);
      if (extent[2] !== 1) {
        gpuInvalid(this.#device, "copySize.depthOrArrayLayers must be 1.");
        return;
      }
      const format = texture.desc.format;
      const bgra = format === "bgra8unorm" || format === "bgra8unorm-srgb";
      if (!bgra && format !== "rgba8unorm" && format !== "rgba8unorm-srgb") {
        gpuInvalid(this.#device, "Copies of external images into " + format + " are not supported.");
        return;
      }
      if (extent[0] === 0 || extent[1] === 0) return;
      // The source's pixels: its size checked against the copy, then the
      // rectangle copied, flipped as asked.
      const whole = gpuBitmapOf(image, null, false);
      let made = null;
      try {
        if (!native.canvas("bitmapClean", 4, whole[0])) {
          throw new $DOMException(words + "The external image is cross-origin.", "SecurityError");
        }
        if (origin[0] + extent[0] > whole[1] || origin[1] + extent[1] > whole[2]) {
          throw new $DOMException(words + "The copy rectangle is outside the source image.",
            "OperationError");
        }
        made = gpuBitmapOf([4, whole[0], null, 0], [origin[0], origin[1], extent[0], extent[1]], flipY);
        const bytes = new $Uint8Array(native.canvas("bitmapPixels", 4, made[0], bgra, premultiplied));
        gpuWriteTexture(this.#device, dest, bytes, { bytesPerRow: extent[0] * 4, rowsPerImage: extent[1] },
          extent, 1);
      } finally {
        native.canvas("closeBitmap", 4, whole[0]);
        if (made !== null) native.canvas("closeBitmap", 4, made[0]);
      }
    }

    writeTexture(destination, data, dataLayout, size) {
      need(arguments.length, 4, "writeTexture", "GPUQueue");
      const words = prefix("writeTexture", "GPUQueue");
      const dest = gpuTexelTexture(destination, words);
      let bytes;
      if (data instanceof $ArrayBuffer) {
        bytes = new $Uint8Array(data);
      } else if (ArrayBufferIsView(data)) {
        bytes = gpuViewBytes(data)[0];
      } else {
        throw new $TypeError(words + "parameter 2 is not of type 'ArrayBuffer' or 'ArrayBufferView'.");
      }
      const r = gpuReader(dataLayout, words, "GPUTexelCopyBufferLayout");
      const layout = {
        bytesPerRow: gpuU32(r, "bytesPerRow", null),
        offset: gpuU64(r, "offset", 0),
        rowsPerImage: gpuU32(r, "rowsPerImage", null),
      };
      const extent = gpuExtent(size, words);
      if (layout.offset > bytes.length) {
        gpuInvalid(this.#device, "The data layout's offset (" + layout.offset + ") is past the data's size (" +
          bytes.length + ").");
        return;
      }
      const texture = gpuTexelRecord;
      gpuWriteTexture(this.#device, dest, TypedArraySubarray(bytes, layout.offset), layout, extent,
        gpuBlockHeight(texture.desc.format));
    }

    onSubmittedWorkDone() {
      return gpuThen(gpuPromise({ op: "workDone", device: this.#device }), () => undefined);
    }

    writeBuffer(buffer, bufferOffset, data, dataOffset, size) {
      need(arguments.length, 3, "writeBuffer", "GPUQueue");
      if (!(buffer instanceof GPUBuffer)) {
        throw new $TypeError(prefix("writeBuffer", "GPUQueue") + "parameter 1 is not of type 'GPUBuffer'.");
      }
      const offset = gpuEnforced(bufferOffset, prefix("writeBuffer", "GPUQueue"), "unsigned long long", GPU_LONG_LONG);
      const bytes = gpuDataBytes(data, dataOffset, size, "writeBuffer", "GPUQueue");
      gpuCall({ op: "writeBuffer", device: this.#device, buffer: gpuBufferId(buffer), offset }, bytes);
    }
  }
  expose(GPUQueue, "GPUQueue");

  // ---- Shader modules (M41 step 3) ----
  const GpuCharCodeAt = uncurry(String.prototype.charCodeAt);
  const GpuLastIndexOf = uncurry(String.prototype.lastIndexOf);
  const GpuIndexOf = uncurry(String.prototype.indexOf);
  const GpuUint32Array = Uint32Array;

  // The UTF-16 index of UTF-8 byte `at` of `code`.
  function gpuUtf16At(code, at) {
    let bytes = 0;
    let i = 0;
    while (i < code.length && bytes < at) {
      const c = GpuCharCodeAt(code, i);
      if (c < 0x80) {
        bytes += 1;
      } else if (c < 0x800) {
        bytes += 2;
      } else if (c >= 0xD800 && c <= 0xDBFF && i + 1 < code.length) {
        bytes += 4;
        i++;
      } else {
        bytes += 3;
      }
      i++;
    }
    return i;
  }

  class GPUCompilationMessage {
    #told;
    constructor(key, told) {
      if (key !== KEY) throw illegalConstructor("GPUCompilationMessage");
      this.#told = told;
    }
    get message() { return this.#told.message; }
    get type() { return this.#told.type; }
    get lineNum() { return this.#told.lineNum; }
    get linePos() { return this.#told.linePos; }
    get offset() { return this.#told.offset; }
    get length() { return this.#told.length; }
  }
  expose(GPUCompilationMessage, "GPUCompilationMessage");

  class GPUCompilationInfo {
    #messages;
    constructor(key, messages) {
      if (key !== KEY) throw illegalConstructor("GPUCompilationInfo");
      this.#messages = ObjectFreeze(messages);
    }
    get messages() { return this.#messages; }
  }
  expose(GPUCompilationInfo, "GPUCompilationInfo");

  // The GPU process's messages of `code`, their places in UTF-16 units
  // as WebGPU counts them (the GPU process counts bytes).
  function gpuMessages(code, told) {
    const messages = [];
    for (let i = 0; i < told.length; i++) {
      const m = told[i];
      let offset = 0;
      let length = 0;
      let linePos = 0;
      if (m.line !== 0) {
        offset = gpuUtf16At(code, m.offset);
        length = gpuUtf16At(code, m.offset + m.length) - offset;
        linePos = offset - (offset === 0 ? 0 : GpuLastIndexOf(code, "\n", offset - 1) + 1) + 1;
      }
      ArrayPush(messages, new GPUCompilationMessage(KEY, {
        message: m.message,
        type: m.kind === 0 ? "error" : m.kind === 1 ? "warning" : "info",
        lineNum: m.line,
        linePos,
        offset,
        length,
      }));
    }
    return messages;
  }

  class GPUShaderModule {
    constructor(key, id, code) {
      if (key !== KEY) throw illegalConstructor("GPUShaderModule");
      gpuThing(this, "GPUShaderModule", id, { code });
    }
    getCompilationInfo() {
      let m;
      try {
        m = gpuRecordOf(this, "GPUShaderModule");
      } catch (e) {
        return rejected(e);
      }
      return gpuThen(gpuPromise({ op: "compilationInfo", module: m.id }),
        (told) => new GPUCompilationInfo(KEY, gpuMessages(m.code, told)));
    }
  }
  gpuLabelled(GPUShaderModule, "GPUShaderModule");

  function gpuCreateShaderModule(device, descriptor) {
    const r = gpuReader(descriptor, prefix("createShaderModule", "GPUDevice"), "GPUShaderModuleDescriptor");
    const code = str(gpuNeed(r, "code"));
    const id = gpuCall({ op: "createShaderModule", device, code }, null).result.id;
    return gpuNamed(new GPUShaderModule(KEY, id, code), r);
  }

  // ---- Textures, views and samplers ----
  class GPUTexture {
    constructor(key, id, device, desc) {
      if (key !== KEY) throw illegalConstructor("GPUTexture");
      gpuThing(this, "GPUTexture", id, { device, desc });
    }
    get width() { return gpuRecordOf(this, "GPUTexture").desc.size[0]; }
    get height() { return gpuRecordOf(this, "GPUTexture").desc.size[1]; }
    get depthOrArrayLayers() { return gpuRecordOf(this, "GPUTexture").desc.size[2]; }
    get mipLevelCount() { return gpuRecordOf(this, "GPUTexture").desc.mipLevelCount; }
    get sampleCount() { return gpuRecordOf(this, "GPUTexture").desc.sampleCount; }
    get dimension() { return gpuRecordOf(this, "GPUTexture").desc.dimension; }
    get format() { return gpuRecordOf(this, "GPUTexture").desc.format; }
    get usage() { return gpuRecordOf(this, "GPUTexture").desc.usage; }
    createView(descriptor) {
      return gpuCreateView(gpuRecordOf(this, "GPUTexture"), descriptor);
    }
    destroy() {
      gpuCall({ op: "destroyTexture", texture: gpuRecordOf(this, "GPUTexture").id }, null);
    }
  }
  gpuLabelled(GPUTexture, "GPUTexture");

  class GPUTextureView {
    constructor(key, id, device, format, aspect, sampleCount) {
      if (key !== KEY) throw illegalConstructor("GPUTextureView");
      gpuThing(this, "GPUTextureView", id, { device, format, aspect, sampleCount });
    }
  }
  gpuLabelled(GPUTextureView, "GPUTextureView");

  function gpuCreateTexture(device, descriptor) {
    const r = gpuReader(descriptor, prefix("createTexture", "GPUDevice"), "GPUTextureDescriptor");
    const dimension = gpuEnum(r, "dimension", GPU_TEXTURE_DIMENSIONS, "GPUTextureDimension", "2d");
    const format = gpuEnum(r, "format", GPU_FORMATS, "GPUTextureFormat");
    const mipLevelCount = gpuU32(r, "mipLevelCount", 1);
    const sampleCount = gpuU32(r, "sampleCount", 1);
    const size = gpuExtent(gpuNeed(r, "size"), gpuWords(r, "size"));
    const usage = gpuU32(r, "usage");
    const named = gpuList(r, "viewFormats", false);
    const viewFormats = [];
    for (let i = 0; i < named.length; i++) {
      ArrayPush(viewFormats, gpuEnumValue(named[i], GPU_FORMATS, gpuWords(r, "viewFormats"), "GPUTextureFormat"));
    }
    const desc = { dimension, format, mipLevelCount, sampleCount, size, usage, viewFormats };
    const id = gpuCall({ op: "createTexture", device, desc }, null).result.id;
    return gpuNamed(new GPUTexture(KEY, id, device, desc), r);
  }

  function gpuCreateView(t, descriptor) {
    const r = gpuReader(descriptor, prefix("createView", "GPUTexture"), "GPUTextureViewDescriptor");
    const desc = {
      arrayLayerCount: gpuU32(r, "arrayLayerCount", null),
      aspect: gpuEnum(r, "aspect", GPU_ASPECTS, "GPUTextureAspect", "all"),
      baseArrayLayer: gpuU32(r, "baseArrayLayer", 0),
      baseMipLevel: gpuU32(r, "baseMipLevel", 0),
      dimension: gpuEnum(r, "dimension", GPU_VIEW_DIMENSIONS, "GPUTextureViewDimension", null),
      format: gpuEnum(r, "format", GPU_FORMATS, "GPUTextureFormat", null),
      mipLevelCount: gpuU32(r, "mipLevelCount", null),
      usage: gpuU32(r, "usage", 0),
    };
    // A usage of 0 is the texture's.
    if (desc.usage === 0) desc.usage = null;
    const id = gpuCall({ op: "createView", texture: t.id, desc }, null).result.id;
    const format = desc.format === null ? t.desc.format : desc.format;
    return gpuNamed(new GPUTextureView(KEY, id, t.device, format, desc.aspect, t.desc.sampleCount), r);
  }

  class GPUSampler {
    constructor(key, id) {
      if (key !== KEY) throw illegalConstructor("GPUSampler");
      gpuThing(this, "GPUSampler", id);
    }
  }
  gpuLabelled(GPUSampler, "GPUSampler");

  function gpuCreateSampler(device, descriptor) {
    const r = gpuReader(descriptor, prefix("createSampler", "GPUDevice"), "GPUSamplerDescriptor");
    const mode = (member) => gpuEnum(r, member, GPU_ADDRESS_MODES, "GPUAddressMode", "clamp-to-edge");
    const filter = (member, type) => gpuEnum(r, member, GPU_FILTERS, type, "nearest");
    const desc = {
      addressModeU: mode("addressModeU"),
      addressModeV: mode("addressModeV"),
      addressModeW: mode("addressModeW"),
      compare: gpuEnum(r, "compare", GPU_COMPARES, "GPUCompareFunction", null),
      lodMaxClamp: gpuFloat(r, "lodMaxClamp", 32, "float"),
      lodMinClamp: gpuFloat(r, "lodMinClamp", 0, "float"),
      magFilter: filter("magFilter", "GPUFilterMode"),
      maxAnisotropy: 1,
      minFilter: filter("minFilter", "GPUFilterMode"),
      mipmapFilter: filter("mipmapFilter", "GPUMipmapFilterMode"),
    };
    // [Clamp] unsigned short.
    if (r.d.maxAnisotropy !== undefined) {
      const n = $Number(r.d.maxAnisotropy);
      desc.maxAnisotropy = n !== n ? 0 : MathMin(65535, MathMax(0, MathFloor(n + 0.5)));
    }
    const id = gpuCall({ op: "createSampler", device, desc }, null).result.id;
    return gpuNamed(new GPUSampler(KEY, id), r);
  }

  // ---- Layouts and bind groups ----
  class GPUBindGroupLayout {
    constructor(key, id) {
      if (key !== KEY) throw illegalConstructor("GPUBindGroupLayout");
      gpuThing(this, "GPUBindGroupLayout", id);
    }
  }
  gpuLabelled(GPUBindGroupLayout, "GPUBindGroupLayout");

  class GPUPipelineLayout {
    constructor(key, id) {
      if (key !== KEY) throw illegalConstructor("GPUPipelineLayout");
      gpuThing(this, "GPUPipelineLayout", id);
    }
  }
  gpuLabelled(GPUPipelineLayout, "GPUPipelineLayout");

  class GPUBindGroup {
    constructor(key, id) {
      if (key !== KEY) throw illegalConstructor("GPUBindGroup");
      gpuThing(this, "GPUBindGroup", id);
    }
  }
  gpuLabelled(GPUBindGroup, "GPUBindGroup");

  function gpuCreateBindGroupLayout(device, descriptor) {
    const r = gpuReader(descriptor, prefix("createBindGroupLayout", "GPUDevice"), "GPUBindGroupLayoutDescriptor");
    const list = gpuList(r, "entries", true);
    const entries = [];
    let error = null;
    for (let i = 0; i < list.length; i++) {
      const e = gpuReader(list[i], gpuWords(r, "entries"), "GPUBindGroupLayoutEntry");
      const entry = { binding: gpuU32(e, "binding") };
      let kinds = 0;
      const buffer = gpuSub(e, "buffer", "GPUBufferBindingLayout");
      if (buffer !== null) {
        kinds++;
        entry.buffer = {
          hasDynamicOffset: gpuBool(buffer, "hasDynamicOffset", false),
          minBindingSize: gpuU64(buffer, "minBindingSize", 0),
          type: gpuEnum(buffer, "type", GPU_BUFFER_BINDINGS, "GPUBufferBindingType", "uniform"),
        };
      }
      if (gpuSub(e, "externalTexture", "GPUExternalTextureBindingLayout") !== null) {
        kinds++;
        if (error === null) error = "External textures are not supported.";
      }
      const sampler = gpuSub(e, "sampler", "GPUSamplerBindingLayout");
      if (sampler !== null) {
        kinds++;
        entry.sampler = {
          type: gpuEnum(sampler, "type", GPU_SAMPLER_BINDINGS, "GPUSamplerBindingType", "filtering"),
        };
      }
      const storage = gpuSub(e, "storageTexture", "GPUStorageTextureBindingLayout");
      if (storage !== null) {
        kinds++;
        entry.storageTexture = {
          access: gpuEnum(storage, "access", GPU_STORAGE_ACCESSES, "GPUStorageTextureAccess", "write-only"),
          format: gpuEnum(storage, "format", GPU_FORMATS, "GPUTextureFormat"),
          viewDimension: gpuEnum(storage, "viewDimension", GPU_VIEW_DIMENSIONS, "GPUTextureViewDimension", "2d"),
        };
      }
      const texture = gpuSub(e, "texture", "GPUTextureBindingLayout");
      if (texture !== null) {
        kinds++;
        entry.texture = {
          multisampled: gpuBool(texture, "multisampled", false),
          sampleType: gpuEnum(texture, "sampleType", GPU_SAMPLE_TYPES, "GPUTextureSampleType", "float"),
          viewDimension: gpuEnum(texture, "viewDimension", GPU_VIEW_DIMENSIONS, "GPUTextureViewDimension", "2d"),
        };
      }
      entry.visibility = gpuU32(e, "visibility");
      if (kinds !== 1) {
        if (error === null) {
          error = "The entry of binding " + entry.binding + " has " + (kinds === 0 ? "no" : "more than one") +
            " binding layout.";
        }
        continue;
      }
      ArrayPush(entries, entry);
    }
    const id = gpuCall({ op: "createBindGroupLayout", device, desc: { entries } }, null).result.id;
    if (error !== null) gpuInvalid(device, error);
    return gpuNamed(new GPUBindGroupLayout(KEY, id), r);
  }

  function gpuCreatePipelineLayout(device, descriptor) {
    const r = gpuReader(descriptor, prefix("createPipelineLayout", "GPUDevice"), "GPUPipelineLayoutDescriptor");
    const list = gpuList(r, "bindGroupLayouts", true);
    const layouts = [];
    for (let i = 0; i < list.length; i++) {
      // A null layout is an empty one, numbered 0.
      ArrayPush(layouts, list[i] === null || list[i] === undefined ? 0
        : gpuNumberOf(list[i], "GPUBindGroupLayout", gpuWords(r, "bindGroupLayouts")));
    }
    const id = gpuCall({ op: "createPipelineLayout", device, layouts }, null).result.id;
    return gpuNamed(new GPUPipelineLayout(KEY, id), r);
  }

  // A bind group entry's resource, as the GPU process reads it.
  function gpuResource(binding, value, words) {
    const r = (typeof value === "object" && value !== null) ? WeakMapGet(gpuThings, value) : undefined;
    if (r !== undefined) {
      if (r.kind === "GPUSampler") return { binding, sampler: r.id };
      if (r.kind === "GPUTextureView") return { binding, view: r.id };
      if (r.kind === "GPUBuffer") return { binding, buffer: r.id, offset: 0, size: null };
      if (r.kind === "GPUTexture") {
        return { binding, view: gpuNumberOf(gpuCreateView(r, undefined), "GPUTextureView", words) };
      }
    }
    if (r === undefined && typeof value === "object" && value !== null) {
      const b = gpuReader(value, words, "GPUBufferBinding");
      const buffer = gpuMemberNumber(b, "buffer", "GPUBuffer");
      return { binding, buffer, offset: gpuU64(b, "offset", 0), size: gpuU64(b, "size", null) };
    }
    throw new $TypeError(words + "The provided value is not of type '(GPUBuffer or GPUBufferBinding or " +
      "GPUExternalTexture or GPUSampler or GPUTexture or GPUTextureView)'.");
  }

  function gpuCreateBindGroup(device, descriptor) {
    const r = gpuReader(descriptor, prefix("createBindGroup", "GPUDevice"), "GPUBindGroupDescriptor");
    const list = gpuList(r, "entries", true);
    const entries = [];
    for (let i = 0; i < list.length; i++) {
      const e = gpuReader(list[i], gpuWords(r, "entries"), "GPUBindGroupEntry");
      const binding = gpuU32(e, "binding");
      ArrayPush(entries, gpuResource(binding, gpuNeed(e, "resource"), gpuWords(e, "resource")));
    }
    const layout = gpuMemberNumber(r, "layout", "GPUBindGroupLayout");
    const id = gpuCall({ op: "createBindGroup", device, desc: { entries, layout } }, null).result.id;
    return gpuNamed(new GPUBindGroup(KEY, id), r);
  }

  // ---- Pipelines ----
  class GPUPipelineError extends $DOMException {
    #reason;
    constructor(message, options) {
      const words = "Failed to construct 'GPUPipelineError': ";
      needToConstruct(arguments.length, 2, "GPUPipelineError");
      const r = gpuReader(options, words, "GPUPipelineErrorInit");
      const reason = gpuEnum(r, "reason", ["validation", "internal"], "GPUPipelineErrorReason");
      super(message === undefined ? "" : str(message), "GPUPipelineError");
      this.#reason = reason;
    }
    get reason() { return this.#reason; }
  }
  expose(GPUPipelineError, "GPUPipelineError");

  // A pipeline's bind group layout `index`, a new object each time.
  function gpuGetBindGroupLayout(p, count, index, iface) {
    need(count, 1, "getBindGroupLayout", iface);
    const n = gpuEnforced(index, prefix("getBindGroupLayout", iface), "unsigned long", GPU_LONG);
    const id = gpuCall({ op: "getBindGroupLayout", pipeline: p.id, index: n }, null).result.id;
    return new GPUBindGroupLayout(KEY, id);
  }

  class GPUComputePipeline {
    constructor(key, id) {
      if (key !== KEY) throw illegalConstructor("GPUComputePipeline");
      gpuThing(this, "GPUComputePipeline", id);
    }
    getBindGroupLayout(index) {
      return gpuGetBindGroupLayout(gpuRecordOf(this, "GPUComputePipeline"), arguments.length, index, "GPUComputePipeline");
    }
  }
  gpuLabelled(GPUComputePipeline, "GPUComputePipeline");

  class GPURenderPipeline {
    constructor(key, id) {
      if (key !== KEY) throw illegalConstructor("GPURenderPipeline");
      gpuThing(this, "GPURenderPipeline", id);
    }
    getBindGroupLayout(index) {
      return gpuGetBindGroupLayout(gpuRecordOf(this, "GPURenderPipeline"), arguments.length, index, "GPURenderPipeline");
    }
  }
  gpuLabelled(GPURenderPipeline, "GPURenderPipeline");

  // A pipeline descriptor's layout: a layout's number, or "auto".
  function gpuLayoutOf(r) {
    const value = gpuNeed(r, "layout");
    const t = (typeof value === "object" && value !== null) ? WeakMapGet(gpuThings, value) : undefined;
    if (t !== undefined && t.kind === "GPUPipelineLayout") return t.id;
    return gpuEnumValue(value, ["auto"], gpuWords(r, "layout"), "GPUAutoLayoutMode");
  }

  // A programmable stage: its module, entry point and constants.
  function gpuStage(s) {
    const stage = {};
    if (s.d.constants !== undefined && s.d.constants !== null) {
      const words = gpuWords(s, "constants");
      const pairs = record(gpuReader(s.d.constants, words, "record<USVString, GPUPipelineConstantValue>").d,
        str, (value) => gpuFinite(value, words, "double"));
      stage.constants = {};
      for (let i = 0; i < pairs.length; i++) stage.constants[pairs[i][0]] = pairs[i][1];
    }
    if (s.d.entryPoint !== undefined) stage.entryPoint = str(s.d.entryPoint);
    stage.module = gpuMemberNumber(s, "module", "GPUShaderModule");
    return stage;
  }

  function gpuComputeDesc(r) {
    const compute = gpuStage(gpuSub(r, "compute", "GPUProgrammableStage", true));
    return { compute, layout: gpuLayoutOf(r) };
  }

  function gpuBlendComponent(b, member) {
    const c = gpuSub(b, member, "GPUBlendComponent", true);
    return {
      dstFactor: gpuEnum(c, "dstFactor", GPU_BLEND_FACTORS, "GPUBlendFactor", "zero"),
      operation: gpuEnum(c, "operation", GPU_BLEND_OPERATIONS, "GPUBlendOperation", "add"),
      srcFactor: gpuEnum(c, "srcFactor", GPU_BLEND_FACTORS, "GPUBlendFactor", "one"),
    };
  }

  function gpuStencilFace(ds, member) {
    const f = gpuSub(ds, member, "GPUStencilFaceState") || gpuReader(undefined, "", "");
    const op = (name) => gpuEnum(f, name, GPU_STENCIL_OPERATIONS, "GPUStencilOperation", "keep");
    return {
      compare: gpuEnum(f, "compare", GPU_COMPARES, "GPUCompareFunction", "always"),
      depthFailOp: op("depthFailOp"),
      failOp: op("failOp"),
      passOp: op("passOp"),
    };
  }

  function gpuRenderDesc(r) {
    const desc = {};
    const ds = gpuSub(r, "depthStencil", "GPUDepthStencilState");
    if (ds !== null) {
      desc.depthStencil = {
        depthBias: gpuI32(ds, "depthBias", 0),
        depthBiasClamp: gpuFloat(ds, "depthBiasClamp", 0, "float"),
        depthBiasSlopeScale: gpuFloat(ds, "depthBiasSlopeScale", 0, "float"),
        depthCompare: gpuEnum(ds, "depthCompare", GPU_COMPARES, "GPUCompareFunction", null),
        depthWriteEnabled: ds.d.depthWriteEnabled === undefined ? null : !!ds.d.depthWriteEnabled,
        format: gpuEnum(ds, "format", GPU_FORMATS, "GPUTextureFormat"),
        stencilBack: gpuStencilFace(ds, "stencilBack"),
        stencilFront: gpuStencilFace(ds, "stencilFront"),
        stencilReadMask: gpuU32(ds, "stencilReadMask", GPU_LONG),
        stencilWriteMask: gpuU32(ds, "stencilWriteMask", GPU_LONG),
      };
    }
    const fragment = gpuSub(r, "fragment", "GPUFragmentState");
    if (fragment !== null) {
      const stage = gpuStage(fragment);
      const list = gpuList(fragment, "targets", true);
      stage.targets = [];
      for (let i = 0; i < list.length; i++) {
        if (list[i] === null || list[i] === undefined) {
          ArrayPush(stage.targets, null);
          continue;
        }
        const t = gpuReader(list[i], gpuWords(fragment, "targets"), "GPUColorTargetState");
        const target = {};
        const blend = gpuSub(t, "blend", "GPUBlendState");
        if (blend !== null) {
          target.blend = { alpha: gpuBlendComponent(blend, "alpha"), color: gpuBlendComponent(blend, "color") };
        }
        target.format = gpuEnum(t, "format", GPU_FORMATS, "GPUTextureFormat");
        target.writeMask = gpuU32(t, "writeMask", 0xF);
        ArrayPush(stage.targets, target);
      }
      desc.fragment = stage;
    }
    desc.layout = gpuLayoutOf(r);
    const ms = gpuSub(r, "multisample", "GPUMultisampleState");
    if (ms !== null) {
      desc.multisample = {
        alphaToCoverageEnabled: gpuBool(ms, "alphaToCoverageEnabled", false),
        count: gpuU32(ms, "count", 1),
        mask: gpuU32(ms, "mask", GPU_LONG),
      };
    }
    const p = gpuSub(r, "primitive", "GPUPrimitiveState");
    if (p !== null) {
      desc.primitive = {
        cullMode: gpuEnum(p, "cullMode", GPU_CULL_MODES, "GPUCullMode", "none"),
        frontFace: gpuEnum(p, "frontFace", GPU_FRONT_FACES, "GPUFrontFace", "ccw"),
        stripIndexFormat: gpuEnum(p, "stripIndexFormat", GPU_INDEX_FORMATS, "GPUIndexFormat", null),
        topology: gpuEnum(p, "topology", GPU_TOPOLOGIES, "GPUPrimitiveTopology", "triangle-list"),
        unclippedDepth: gpuBool(p, "unclippedDepth", false),
      };
    }
    const v = gpuSub(r, "vertex", "GPUVertexState", true);
    const vertex = gpuStage(v);
    const list = gpuList(v, "buffers", false);
    vertex.buffers = [];
    for (let i = 0; i < list.length; i++) {
      if (list[i] === null || list[i] === undefined) {
        ArrayPush(vertex.buffers, null);
        continue;
      }
      const b = gpuReader(list[i], gpuWords(v, "buffers"), "GPUVertexBufferLayout");
      const buffer = { arrayStride: gpuU64(b, "arrayStride"), attributes: [] };
      const attributes = gpuList(b, "attributes", true);
      for (let j = 0; j < attributes.length; j++) {
        const a = gpuReader(attributes[j], gpuWords(b, "attributes"), "GPUVertexAttribute");
        ArrayPush(buffer.attributes, {
          format: gpuEnum(a, "format", GPU_VERTEX_FORMATS, "GPUVertexFormat"),
          offset: gpuU64(a, "offset"),
          shaderLocation: gpuU32(a, "shaderLocation"),
        });
      }
      buffer.stepMode = gpuEnum(b, "stepMode", GPU_STEP_MODES, "GPUVertexStepMode", "vertex");
      ArrayPush(vertex.buffers, buffer);
    }
    desc.vertex = vertex;
    return desc;
  }

  // A pipeline of `device`; the asynchronous kind's promise settles once
  // the GPU process has made it.
  function gpuCreatePipeline(device, descriptor, compute, async) {
    const method = (compute ? "createComputePipeline" : "createRenderPipeline") + (async ? "Async" : "");
    const r = gpuReader(descriptor, prefix(method, "GPUDevice"),
      compute ? "GPUComputePipelineDescriptor" : "GPURenderPipelineDescriptor");
    const op = {
      op: compute ? "createComputePipeline" : "createRenderPipeline",
      device,
      desc: compute ? gpuComputeDesc(r) : gpuRenderDesc(r),
    };
    const make = (id) => gpuNamed(compute ? new GPUComputePipeline(KEY, id) : new GPURenderPipeline(KEY, id), r);
    if (!async) return make(gpuCall(op, null).result.id);
    const promise = gpuNextPromise++;
    op.promise = promise;
    return new $Promise((resolve, reject) => {
      let pipeline = null;
      MapSet(gpuPromises, promise, { resolve: () => resolve(pipeline), reject });
      try {
        pipeline = make(gpuCall(op, null).result.id);
      } catch (e) {
        MapDelete(gpuPromises, promise);
        reject(e);
      }
    });
  }

  // ---- Query sets, command buffers, render bundles ----
  class GPUQuerySet {
    constructor(key, id, type, count) {
      if (key !== KEY) throw illegalConstructor("GPUQuerySet");
      gpuThing(this, "GPUQuerySet", id, { type, count });
    }
    get type() { return gpuRecordOf(this, "GPUQuerySet").type; }
    get count() { return gpuRecordOf(this, "GPUQuerySet").count; }
    destroy() {
      gpuCall({ op: "destroyQuerySet", set: gpuRecordOf(this, "GPUQuerySet").id }, null);
    }
  }
  gpuLabelled(GPUQuerySet, "GPUQuerySet");

  function gpuCreateQuerySet(device, descriptor) {
    const r = gpuReader(descriptor, prefix("createQuerySet", "GPUDevice"), "GPUQuerySetDescriptor");
    const count = gpuU32(r, "count");
    const type = gpuEnum(r, "type", GPU_QUERY_TYPES, "GPUQueryType");
    const id = gpuCall({ op: "createQuerySet", device, kind: type === "occlusion" ? 0 : 1, count }, null).result.id;
    return gpuNamed(new GPUQuerySet(KEY, id, type, count), r);
  }

  // A command buffer: `id` null when its encoder was invalid.
  class GPUCommandBuffer {
    constructor(key, id) {
      if (key !== KEY) throw illegalConstructor("GPUCommandBuffer");
      gpuThing(this, "GPUCommandBuffer", id, { submitted: false });
    }
  }
  gpuLabelled(GPUCommandBuffer, "GPUCommandBuffer");

  class GPURenderBundle {
    constructor(key, id, layout) {
      if (key !== KEY) throw illegalConstructor("GPURenderBundle");
      gpuThing(this, "GPURenderBundle", id, { layout });
    }
  }
  gpuLabelled(GPURenderBundle, "GPURenderBundle");

  // ---- Encoders ----
  // An encoder's record (a command encoder's, a pass's, a render bundle
  // encoder's): its device, "open", "locked" while a pass of it is open,
  // or "ended"; what it recorded; the first error that makes it invalid,
  // reported at its end; its debug groups' depth.
  function gpuEncoder(device) {
    return { device, state: "open", commands: [], error: null, depth: 0 };
  }

  // Whether encoder `s` may record: a locked encoder's use makes it
  // invalid, an ended one's is a validation error.
  function gpuOpen(s) {
    if (s.state === "open") return true;
    if (s.state === "locked") {
      if (s.error === null) s.error = "The " + s.kind + " is locked while a pass of it is open.";
    } else {
      gpuInvalid(s.device, "The " + s.kind + " has already ended.");
    }
    return false;
  }

  function gpuRecord(s, command) {
    if (gpuOpen(s)) ArrayPush(s.commands, command);
  }

  // A draw: without a pipeline set, it makes its encoder invalid.
  function gpuDraw(s, command) {
    if (s.state === "open" && !s.pipeline) gpuInvalidate(s, "A draw was made with no pipeline set.");
    gpuRecord(s, command);
  }

  function gpuInvalidate(s, message) {
    if (s.error === null) s.error = message;
  }

  // A pass of command encoder `s`, of `kind`, its descriptor `desc`.
  function gpuBeginPass(s, pass, kind, desc, error, layout) {
    const p = gpuEncoder(s.device);
    p.parent = s;
    p.desc = desc;
    p.layout = layout;
    p.pipeline = false;
    p.error = error;
    gpuThing(pass, kind, null, p);
    if (gpuOpen(s)) {
      s.state = "locked";
    } else {
      // A pass of an encoder that could not begin one is invalid.
      p.parent = null;
      gpuInvalidate(p, "The pass's encoder could not begin it.");
    }
    return pass;
  }

  // Pass `s` ends: its parent records it.
  function gpuEndPass(s, command) {
    if (s.state !== "open") {
      gpuInvalid(s.device, "The " + s.kind + " has already ended.");
      return;
    }
    s.state = "ended";
    if (s.depth !== 0) gpuInvalidate(s, "A debug group of the pass was not popped.");
    if (s.occluding) gpuInvalidate(s, "An occlusion query of the pass was not ended.");
    const parent = s.parent;
    if (parent === null) return;
    parent.state = "open";
    if (s.error !== null) gpuInvalidate(parent, s.error);
    ArrayPush(parent.commands, command);
  }

  const GPU_RENDER_KINDS = ["GPURenderPassEncoder", "GPURenderBundleEncoder"];
  const GPU_PASS_KINDS = ["GPURenderPassEncoder", "GPURenderBundleEncoder", "GPUComputePassEncoder"];
  const GPU_ENCODER_KINDS = ["GPURenderPassEncoder", "GPURenderBundleEncoder", "GPUComputePassEncoder",
    "GPUCommandEncoder"];

  // An argument of `method` on `s`'s interface.
  function gpuArg(s, method, value, fallback) {
    return value === undefined ? fallback : gpuEnforced(value, prefix(method, s.kind), "unsigned long", GPU_LONG);
  }

  function gpuWideArg(s, method, value, fallback) {
    return value === undefined ? fallback : gpuEnforced(value, prefix(method, s.kind), "unsigned long long", GPU_LONG_LONG);
  }

  function gpuObjectArg(s, method, value, kind, index) {
    const r = (typeof value === "object" && value !== null) ? WeakMapGet(gpuThings, value) : undefined;
    if (r === undefined || r.kind !== kind) {
      throw new $TypeError(prefix(method, s.kind) + "parameter " + index + " is not of type '" + kind + "'.");
    }
    return r.id;
  }

  // Methods on `prototypes`, as operations are (enumerable).
  function gpuMethods(prototypes, methods) {
    for (let i = 0; i < prototypes.length; i++) {
      for (let j = 0; j < methods.length; j++) {
        defineProperty(prototypes[i], methods[j][0], {
          value: methods[j][1],
          writable: true,
          enumerable: true,
          configurable: true,
        });
      }
    }
  }

  class GPUCommandEncoder {
    constructor(key, device) {
      if (key !== KEY) throw illegalConstructor("GPUCommandEncoder");
      gpuThing(this, "GPUCommandEncoder", null, gpuEncoder(device));
    }

    beginRenderPass(descriptor) {
      const s = gpuRecordOf(this, "GPUCommandEncoder");
      need(arguments.length, 1, "beginRenderPass", "GPUCommandEncoder");
      const checked = gpuRenderPassDesc(descriptor);
      return gpuNamed(gpuBeginPass(s, new GPURenderPassEncoder(KEY), "GPURenderPassEncoder", checked.desc,
        checked.error, checked.layout), checked.reader);
    }

    beginComputePass(descriptor) {
      const s = gpuRecordOf(this, "GPUCommandEncoder");
      const r = gpuReader(descriptor, prefix("beginComputePass", "GPUCommandEncoder"), "GPUComputePassDescriptor");
      return gpuNamed(gpuBeginPass(s, new GPUComputePassEncoder(KEY), "GPUComputePassEncoder", null, null, null), r);
    }

    copyBufferToBuffer(source, a, b, c, d) {
      const s = gpuRecordOf(this, "GPUCommandEncoder");
      const method = "copyBufferToBuffer";
      need(arguments.length, 2, method, "GPUCommandEncoder");
      // (source, destination, size) or (source, sourceOffset,
      // destination, destinationOffset, size).
      const short = arguments.length <= 3;
      const command = {
        c: method,
        source: gpuObjectArg(s, method, source, "GPUBuffer", 1),
        sourceOffset: short ? 0 : gpuWideArg(s, method, a),
        destination: gpuObjectArg(s, method, short ? a : b, "GPUBuffer", short ? 2 : 3),
        destinationOffset: short ? 0 : gpuWideArg(s, method, c),
        size: gpuWideArg(s, method, short ? b : d, null),
      };
      if (!short) need(arguments.length, 4, method, "GPUCommandEncoder");
      gpuRecord(s, command);
    }

    copyBufferToTexture(source, destination, copySize) {
      const s = gpuRecordOf(this, "GPUCommandEncoder");
      need(arguments.length, 3, "copyBufferToTexture", "GPUCommandEncoder");
      const words = prefix("copyBufferToTexture", "GPUCommandEncoder");
      gpuRecord(s, {
        c: "copyBufferToTexture",
        desc: {
          source: gpuTexelBuffer(source, words),
          destination: gpuTexelTexture(destination, words),
          size: gpuExtent(copySize, words),
        },
      });
    }

    copyTextureToBuffer(source, destination, copySize) {
      const s = gpuRecordOf(this, "GPUCommandEncoder");
      need(arguments.length, 3, "copyTextureToBuffer", "GPUCommandEncoder");
      const words = prefix("copyTextureToBuffer", "GPUCommandEncoder");
      gpuRecord(s, {
        c: "copyTextureToBuffer",
        desc: {
          source: gpuTexelTexture(source, words),
          destination: gpuTexelBuffer(destination, words),
          size: gpuExtent(copySize, words),
        },
      });
    }

    copyTextureToTexture(source, destination, copySize) {
      const s = gpuRecordOf(this, "GPUCommandEncoder");
      need(arguments.length, 3, "copyTextureToTexture", "GPUCommandEncoder");
      const words = prefix("copyTextureToTexture", "GPUCommandEncoder");
      gpuRecord(s, {
        c: "copyTextureToTexture",
        desc: {
          source: gpuTexelTexture(source, words),
          destination: gpuTexelTexture(destination, words),
          size: gpuExtent(copySize, words),
        },
      });
    }

    clearBuffer(buffer, offset, size) {
      const s = gpuRecordOf(this, "GPUCommandEncoder");
      need(arguments.length, 1, "clearBuffer", "GPUCommandEncoder");
      gpuRecord(s, {
        c: "clearBuffer",
        buffer: gpuObjectArg(s, "clearBuffer", buffer, "GPUBuffer", 1),
        offset: gpuWideArg(s, "clearBuffer", offset, 0),
        size: gpuWideArg(s, "clearBuffer", size, null),
      });
    }

    resolveQuerySet(querySet, firstQuery, queryCount, destination, destinationOffset) {
      const s = gpuRecordOf(this, "GPUCommandEncoder");
      const method = "resolveQuerySet";
      need(arguments.length, 5, method, "GPUCommandEncoder");
      gpuRecord(s, {
        c: method,
        set: gpuObjectArg(s, method, querySet, "GPUQuerySet", 1),
        first: gpuArg(s, method, firstQuery),
        count: gpuArg(s, method, queryCount),
        destination: gpuObjectArg(s, method, destination, "GPUBuffer", 4),
        offset: gpuWideArg(s, method, destinationOffset),
      });
    }

    finish(descriptor) {
      const s = gpuRecordOf(this, "GPUCommandEncoder");
      const r = gpuReader(descriptor, prefix("finish", "GPUCommandEncoder"), "GPUCommandBufferDescriptor");
      let error = s.error;
      if (s.state === "locked") error = "The GPUCommandEncoder is locked while a pass of it is open.";
      if (s.state === "ended") error = "The GPUCommandEncoder has already finished.";
      if (error === null && s.depth !== 0) error = "A debug group of the encoder was not popped.";
      s.state = "ended";
      if (error !== null) {
        gpuInvalid(s.device, error);
        return gpuNamed(new GPUCommandBuffer(KEY, null), r);
      }
      const commands = s.commands;
      s.commands = [];
      const id = gpuCall({ op: "finish", device: s.device, commands }, null).result.id;
      return gpuNamed(new GPUCommandBuffer(KEY, id), r);
    }
  }
  gpuLabelled(GPUCommandEncoder, "GPUCommandEncoder");

  function gpuCreateCommandEncoder(device, descriptor) {
    const r = gpuReader(descriptor, prefix("createCommandEncoder", "GPUDevice"), "GPUCommandEncoderDescriptor");
    return gpuNamed(new GPUCommandEncoder(KEY, device), r);
  }

  // A GPUTexelCopyBufferInfo.
  function gpuTexelBuffer(value, words) {
    const r = gpuReader(value, words, "GPUTexelCopyBufferInfo");
    return {
      buffer: gpuMemberNumber(r, "buffer", "GPUBuffer"),
      bytesPerRow: gpuU32(r, "bytesPerRow", null),
      offset: gpuU64(r, "offset", 0),
      rowsPerImage: gpuU32(r, "rowsPerImage", null),
    };
  }

  // A GPUTexelCopyTextureInfo; its texture's record is left in
  // `gpuTexelRecord`.
  let gpuTexelRecord = null;
  function gpuTexelTexture(value, words) {
    const r = gpuReader(value, words, "GPUTexelCopyTextureInfo");
    const aspect = gpuEnum(r, "aspect", GPU_ASPECTS, "GPUTextureAspect", "all");
    const mipLevel = gpuU32(r, "mipLevel", 0);
    const origin = gpuOrigin(r.d.origin, gpuWords(r, "origin"));
    const texture = gpuNeed(r, "texture");
    const id = gpuNumberOf(texture, "GPUTexture", gpuWords(r, "texture"));
    gpuTexelRecord = WeakMapGet(gpuThings, texture);
    return { aspect, mipLevel, origin, texture: id };
  }

  // A render pass descriptor, and the first error the standard finds in
  // its attachments that `wgpu` would not (their load and store ops).
  function gpuRenderPassDesc(descriptor) {
    const r = gpuReader(descriptor, prefix("beginRenderPass", "GPUCommandEncoder"), "GPURenderPassDescriptor");
    let error = null;
    const list = gpuList(r, "colorAttachments", true);
    const colors = [];
    const layout = { colors: [], depth: null, samples: null };
    for (let i = 0; i < list.length; i++) {
      if (list[i] === null || list[i] === undefined) {
        ArrayPush(colors, null);
        ArrayPush(layout.colors, null);
        continue;
      }
      const a = gpuReader(list[i], gpuWords(r, "colorAttachments"), "GPURenderPassColorAttachment");
      const color = {};
      if (a.d.clearValue !== undefined) color.clearValue = gpuColor(a.d.clearValue, gpuWords(a, "clearValue"));
      color.depthSlice = gpuU32(a, "depthSlice", null);
      color.loadOp = gpuEnum(a, "loadOp", GPU_LOAD_OPS, "GPULoadOp");
      color.resolveTarget = gpuAttachmentView(a, "resolveTarget", true);
      color.storeOp = gpuEnum(a, "storeOp", GPU_STORE_OPS, "GPUStoreOp");
      color.view = gpuAttachmentView(a, "view", false);
      ArrayPush(colors, color);
      const shape = gpuShape(a.d.view);
      ArrayPush(layout.colors, shape[0]);
      layout.samples = shape[1];
    }
    const desc = { colorAttachments: colors };
    const ds = gpuSub(r, "depthStencilAttachment", "GPURenderPassDepthStencilAttachment");
    if (ds !== null) {
      const d = {
        depthClearValue: gpuFloat(ds, "depthClearValue", null, "float"),
        depthLoadOp: gpuEnum(ds, "depthLoadOp", GPU_LOAD_OPS, "GPULoadOp", null),
        depthReadOnly: gpuBool(ds, "depthReadOnly", false),
        depthStoreOp: gpuEnum(ds, "depthStoreOp", GPU_STORE_OPS, "GPUStoreOp", null),
        stencilClearValue: gpuU32(ds, "stencilClearValue", 0),
        stencilLoadOp: gpuEnum(ds, "stencilLoadOp", GPU_LOAD_OPS, "GPULoadOp", null),
        stencilReadOnly: gpuBool(ds, "stencilReadOnly", false),
        stencilStoreOp: gpuEnum(ds, "stencilStoreOp", GPU_STORE_OPS, "GPUStoreOp", null),
      };
      d.view = gpuAttachmentView(ds, "view", false);
      const shape = gpuShape(ds.d.view);
      layout.depth = shape[0];
      layout.samples = shape[1];
      const view = WeakMapGet(gpuThings, ds.d.view);
      const format = view.kind === "GPUTexture" ? view.desc.format : view.format;
      const aspect = view.kind === "GPUTexture" ? "all" : view.aspect;
      const depth = listed(GPU_DEPTH_FORMATS, format) && aspect !== "stencil-only";
      const stencil = listed(GPU_STENCIL_FORMATS, format) && aspect !== "depth-only";
      const ops = (has, prefixName, readOnly) => {
        const given = d[prefixName + "LoadOp"] !== null || d[prefixName + "StoreOp"] !== null;
        if (has && !readOnly && (d[prefixName + "LoadOp"] === null || d[prefixName + "StoreOp"] === null)) {
          return "The " + prefixName + " load and store operations must be given for the attachment's " + format + ".";
        }
        if ((!has || readOnly) && given) {
          return "The " + prefixName + " load and store operations must not be given for the attachment.";
        }
        return null;
      };
      error = ops(depth, "depth", d.depthReadOnly) || ops(stencil, "stencil", d.stencilReadOnly);
      if (error === null && d.depthLoadOp === "clear" &&
        (d.depthClearValue === null || d.depthClearValue < 0 || d.depthClearValue > 1)) {
        error = "The depth clear value must be given, between 0 and 1.";
      }
      desc.depthStencilAttachment = d;
    }
    desc.maxDrawCount = gpuU64(r, "maxDrawCount", 50000000);
    desc.occlusionQuerySet = gpuMemberNumber(r, "occlusionQuerySet", "GPUQuerySet", true);
    if (r.d.timestampWrites !== undefined && error === null) {
      error = "Timestamp writes are not supported.";
    }
    return { desc, error, layout, reader: r };
  }

  // An attachment's format and sample count, of a view or a texture.
  function gpuShape(value) {
    const r = WeakMapGet(gpuThings, value);
    return r.kind === "GPUTexture" ? [r.desc.format, r.desc.sampleCount] : [r.format, r.sampleCount];
  }

  // Whether a render bundle's layout is a pass's: the same color formats
  // (trailing nulls aside), depth-stencil format and sample count.
  function gpuSameLayout(a, b) {
    let n = a.colors.length;
    while (n > 0 && a.colors[n - 1] === null) n--;
    let m = b.colors.length;
    while (m > 0 && b.colors[m - 1] === null) m--;
    if (n !== m || a.depth !== b.depth || a.samples !== b.samples) return false;
    for (let i = 0; i < n; i++) {
      if (a.colors[i] !== b.colors[i]) return false;
    }
    return true;
  }

  // An attachment's view: a view's number, or a texture's default view's.
  function gpuAttachmentView(a, member, optional) {
    const value = a.d[member];
    if (value === undefined && optional) return null;
    const t = (typeof value === "object" && value !== null) ? WeakMapGet(gpuThings, value) : undefined;
    if (t !== undefined && t.kind === "GPUTexture") {
      return WeakMapGet(gpuThings, gpuCreateView(t, undefined)).id;
    }
    return gpuMemberNumber(a, member, "GPUTextureView");
  }

  class GPURenderPassEncoder {
    constructor(key) {
      if (key !== KEY) throw illegalConstructor("GPURenderPassEncoder");
    }

    setViewport(x, y, width, height, minDepth, maxDepth) {
      const s = gpuRecordOf(this, "GPURenderPassEncoder");
      need(arguments.length, 6, "setViewport", "GPURenderPassEncoder");
      const words = prefix("setViewport", "GPURenderPassEncoder");
      gpuRecord(s, {
        c: "setViewport",
        x: gpuFinite(x, words, "float"),
        y: gpuFinite(y, words, "float"),
        width: gpuFinite(width, words, "float"),
        height: gpuFinite(height, words, "float"),
        minDepth: gpuFinite(minDepth, words, "float"),
        maxDepth: gpuFinite(maxDepth, words, "float"),
      });
    }

    setScissorRect(x, y, width, height) {
      const s = gpuRecordOf(this, "GPURenderPassEncoder");
      need(arguments.length, 4, "setScissorRect", "GPURenderPassEncoder");
      const m = "setScissorRect";
      gpuRecord(s, { c: m, x: gpuArg(s, m, x), y: gpuArg(s, m, y), width: gpuArg(s, m, width), height: gpuArg(s, m, height) });
    }

    setBlendConstant(color) {
      const s = gpuRecordOf(this, "GPURenderPassEncoder");
      need(arguments.length, 1, "setBlendConstant", "GPURenderPassEncoder");
      const c = gpuColor(color, prefix("setBlendConstant", "GPURenderPassEncoder"));
      gpuRecord(s, { c: "setBlendConstant", r: c[0], g: c[1], b: c[2], a: c[3] });
    }

    setStencilReference(reference) {
      const s = gpuRecordOf(this, "GPURenderPassEncoder");
      need(arguments.length, 1, "setStencilReference", "GPURenderPassEncoder");
      gpuRecord(s, { c: "setStencilReference", reference: gpuArg(s, "setStencilReference", reference) });
    }

    beginOcclusionQuery(queryIndex) {
      const s = gpuRecordOf(this, "GPURenderPassEncoder");
      need(arguments.length, 1, "beginOcclusionQuery", "GPURenderPassEncoder");
      const index = gpuArg(s, "beginOcclusionQuery", queryIndex);
      if (s.state === "open") s.occluding = true;
      gpuRecord(s, { c: "beginOcclusionQuery", index });
    }

    endOcclusionQuery() {
      const s = gpuRecordOf(this, "GPURenderPassEncoder");
      if (s.state === "open") s.occluding = false;
      gpuRecord(s, { c: "endOcclusionQuery" });
    }

    executeBundles(bundles) {
      const s = gpuRecordOf(this, "GPURenderPassEncoder");
      need(arguments.length, 1, "executeBundles", "GPURenderPassEncoder");
      const words = prefix("executeBundles", "GPURenderPassEncoder");
      const list = gpuSequence(bundles, words);
      const ids = [];
      let invalid = false;
      let other = false;
      for (let i = 0; i < list.length; i++) {
        const id = gpuNumberOf(list[i], "GPURenderBundle", words);
        if (id === null) invalid = true;
        else ArrayPush(ids, id);
        if (!gpuSameLayout(WeakMapGet(gpuThings, list[i]).layout, s.layout)) other = true;
      }
      if (s.state === "open") {
        if (invalid) gpuInvalidate(s, "A render bundle executed is invalid.");
        if (other) gpuInvalidate(s, "A render bundle's formats or sample count are not the pass's.");
        s.pipeline = false;
      }
      gpuRecord(s, { c: "executeBundles", bundles: ids });
    }

    end() {
      const s = gpuRecordOf(this, "GPURenderPassEncoder");
      gpuEndPass(s, { c: "renderPass", desc: s.desc, commands: s.commands });
    }
  }
  gpuLabelled(GPURenderPassEncoder, "GPURenderPassEncoder");

  class GPUComputePassEncoder {
    constructor(key) {
      if (key !== KEY) throw illegalConstructor("GPUComputePassEncoder");
    }

    setPipeline(pipeline) {
      const s = gpuRecordOf(this, "GPUComputePassEncoder");
      need(arguments.length, 1, "setPipeline", "GPUComputePassEncoder");
      gpuRecord(s, { c: "setPipeline", pipeline: gpuObjectArg(s, "setPipeline", pipeline, "GPUComputePipeline", 1) });
    }

    dispatchWorkgroups(x, y, z) {
      const s = gpuRecordOf(this, "GPUComputePassEncoder");
      const m = "dispatchWorkgroups";
      need(arguments.length, 1, m, "GPUComputePassEncoder");
      gpuRecord(s, { c: "dispatch", x: gpuArg(s, m, x), y: gpuArg(s, m, y, 1), z: gpuArg(s, m, z, 1) });
    }

    dispatchWorkgroupsIndirect(indirectBuffer, indirectOffset) {
      const s = gpuRecordOf(this, "GPUComputePassEncoder");
      const m = "dispatchWorkgroupsIndirect";
      need(arguments.length, 2, m, "GPUComputePassEncoder");
      gpuRecord(s, {
        c: "dispatchIndirect",
        buffer: gpuObjectArg(s, m, indirectBuffer, "GPUBuffer", 1),
        offset: gpuWideArg(s, m, indirectOffset),
      });
    }

    end() {
      const s = gpuRecordOf(this, "GPUComputePassEncoder");
      gpuEndPass(s, { c: "computePass", commands: s.commands });
    }
  }
  gpuLabelled(GPUComputePassEncoder, "GPUComputePassEncoder");

  class GPURenderBundleEncoder {
    constructor(key) {
      if (key !== KEY) throw illegalConstructor("GPURenderBundleEncoder");
    }

    finish(descriptor) {
      const s = gpuRecordOf(this, "GPURenderBundleEncoder");
      const r = gpuReader(descriptor, prefix("finish", "GPURenderBundleEncoder"), "GPURenderBundleDescriptor");
      let error = s.error;
      if (s.state !== "open") error = "The GPURenderBundleEncoder has already finished.";
      if (error === null && s.depth !== 0) error = "A debug group of the encoder was not popped.";
      s.state = "ended";
      if (error !== null) {
        gpuInvalid(s.device, error);
        return gpuNamed(new GPURenderBundle(KEY, null, s.layout), r);
      }
      const op = { op: "createRenderBundle", device: s.device, desc: s.desc, commands: s.commands };
      s.commands = [];
      return gpuNamed(new GPURenderBundle(KEY, gpuCall(op, null).result.id, s.layout), r);
    }
  }
  gpuLabelled(GPURenderBundleEncoder, "GPURenderBundleEncoder");

  function gpuCreateRenderBundleEncoder(device, descriptor) {
    const r = gpuReader(descriptor, prefix("createRenderBundleEncoder", "GPUDevice"), "GPURenderBundleEncoderDescriptor");
    const named = gpuList(r, "colorFormats", true);
    const colorFormats = [];
    for (let i = 0; i < named.length; i++) {
      ArrayPush(colorFormats, named[i] === null || named[i] === undefined ? null
        : gpuEnumValue(named[i], GPU_FORMATS, gpuWords(r, "colorFormats"), "GPUTextureFormat"));
    }
    const desc = {
      colorFormats,
      depthReadOnly: gpuBool(r, "depthReadOnly", false),
      depthStencilFormat: gpuEnum(r, "depthStencilFormat", GPU_FORMATS, "GPUTextureFormat", null),
      sampleCount: gpuU32(r, "sampleCount", 1),
      stencilReadOnly: gpuBool(r, "stencilReadOnly", false),
    };
    const encoder = new GPURenderBundleEncoder(KEY);
    const s = gpuEncoder(device);
    s.desc = desc;
    s.layout = { colors: colorFormats, depth: desc.depthStencilFormat, samples: desc.sampleCount };
    s.pipeline = false;
    gpuThing(encoder, "GPURenderBundleEncoder", null, s);
    return gpuNamed(encoder, r);
  }

  // GPUDebugCommandsMixin.
  gpuMethods([GPUCommandEncoder.prototype, GPURenderPassEncoder.prototype, GPUComputePassEncoder.prototype,
    GPURenderBundleEncoder.prototype], [
    ["pushDebugGroup", function pushDebugGroup(groupLabel) {
      const s = gpuRecordOf(this, GPU_ENCODER_KINDS);
      need(arguments.length, 1, "pushDebugGroup", s.kind);
      str(groupLabel);
      if (gpuOpen(s)) s.depth++;
    }],
    ["popDebugGroup", function popDebugGroup() {
      const s = gpuRecordOf(this, GPU_ENCODER_KINDS);
      if (!gpuOpen(s)) return;
      if (s.depth === 0) {
        gpuInvalidate(s, "A debug group was popped that was not pushed.");
      } else {
        s.depth--;
      }
    }],
    ["insertDebugMarker", function insertDebugMarker(markerLabel) {
      const s = gpuRecordOf(this, GPU_ENCODER_KINDS);
      need(arguments.length, 1, "insertDebugMarker", s.kind);
      str(markerLabel);
      gpuOpen(s);
    }],
  ]);

  // GPUBindingCommandsMixin.
  gpuMethods([GPURenderPassEncoder.prototype, GPUComputePassEncoder.prototype, GPURenderBundleEncoder.prototype], [
    ["setBindGroup", function setBindGroup(index, bindGroup, offsets, start, length) {
      const s = gpuRecordOf(this, GPU_PASS_KINDS);
      const m = "setBindGroup";
      need(arguments.length, 2, m, s.kind);
      const n = gpuArg(s, m, index);
      const group = bindGroup === null || bindGroup === undefined ? null : gpuObjectArg(s, m, bindGroup, "GPUBindGroup", 2);
      const list = [];
      if (arguments.length >= 5) {
        // (index, group, Uint32Array data, start, length).
        if (!(offsets instanceof GpuUint32Array)) {
          throw new $TypeError(prefix(m, s.kind) + "parameter 3 is not of type 'Uint32Array'.");
        }
        const first = gpuWideArg(s, m, start);
        const count = gpuArg(s, m, length);
        if (first + count > TypedArrayCount(offsets)) {
          throw new $RangeError(prefix(m, s.kind) + "dynamicOffsetsDataStart + dynamicOffsetsDataLength (" +
            (first + count) + ") is larger than dynamicOffsetsData's length (" + TypedArrayCount(offsets) + ").");
        }
        for (let i = 0; i < count; i++) ArrayPush(list, offsets[first + i]);
      } else if (offsets !== undefined) {
        const given = gpuSequence(offsets, prefix(m, s.kind));
        for (let i = 0; i < given.length; i++) ArrayPush(list, gpuArg(s, m, given[i]));
      }
      gpuRecord(s, { c: m, index: n, group, offsets: list });
    }],
  ]);

  // GPURenderCommandsMixin.
  gpuMethods([GPURenderPassEncoder.prototype, GPURenderBundleEncoder.prototype], [
    ["setPipeline", function setPipeline(pipeline) {
      const s = gpuRecordOf(this, GPU_RENDER_KINDS);
      need(arguments.length, 1, "setPipeline", s.kind);
      const id = gpuObjectArg(s, "setPipeline", pipeline, "GPURenderPipeline", 1);
      if (s.state === "open") s.pipeline = true;
      gpuRecord(s, { c: "setPipeline", pipeline: id });
    }],
    ["setIndexBuffer", function setIndexBuffer(buffer, indexFormat, offset, size) {
      const s = gpuRecordOf(this, GPU_RENDER_KINDS);
      const m = "setIndexBuffer";
      need(arguments.length, 2, m, s.kind);
      const id = gpuObjectArg(s, m, buffer, "GPUBuffer", 1);
      const format = gpuEnumValue(indexFormat, GPU_INDEX_FORMATS, prefix(m, s.kind), "GPUIndexFormat");
      gpuRecord(s, {
        c: m,
        buffer: id,
        format: format === "uint16" ? 0 : 1,
        offset: gpuWideArg(s, m, offset, 0),
        size: gpuWideArg(s, m, size, null),
      });
    }],
    ["setVertexBuffer", function setVertexBuffer(slot, buffer, offset, size) {
      const s = gpuRecordOf(this, GPU_RENDER_KINDS);
      const m = "setVertexBuffer";
      need(arguments.length, 2, m, s.kind);
      const n = gpuArg(s, m, slot);
      gpuRecord(s, {
        c: m,
        slot: n,
        buffer: buffer === null ? null : gpuObjectArg(s, m, buffer, "GPUBuffer", 2),
        offset: gpuWideArg(s, m, offset, 0),
        size: gpuWideArg(s, m, size, null),
      });
    }],
    ["draw", function draw(vertexCount, instanceCount, firstVertex, firstInstance) {
      const s = gpuRecordOf(this, GPU_RENDER_KINDS);
      need(arguments.length, 1, "draw", s.kind);
      gpuDraw(s, {
        c: "draw",
        vertices: gpuArg(s, "draw", vertexCount),
        instances: gpuArg(s, "draw", instanceCount, 1),
        firstVertex: gpuArg(s, "draw", firstVertex, 0),
        firstInstance: gpuArg(s, "draw", firstInstance, 0),
      });
    }],
    ["drawIndexed", function drawIndexed(indexCount, instanceCount, firstIndex, baseVertex, firstInstance) {
      const s = gpuRecordOf(this, GPU_RENDER_KINDS);
      const m = "drawIndexed";
      need(arguments.length, 1, m, s.kind);
      gpuDraw(s, {
        c: m,
        indices: gpuArg(s, m, indexCount),
        instances: gpuArg(s, m, instanceCount, 1),
        firstIndex: gpuArg(s, m, firstIndex, 0),
        baseVertex: baseVertex === undefined ? 0 : gpuSigned(baseVertex, prefix(m, s.kind)),
        firstInstance: gpuArg(s, m, firstInstance, 0),
      });
    }],
    ["drawIndirect", function drawIndirect(indirectBuffer, indirectOffset) {
      const s = gpuRecordOf(this, GPU_RENDER_KINDS);
      need(arguments.length, 2, "drawIndirect", s.kind);
      gpuDraw(s, {
        c: "drawIndirect",
        buffer: gpuObjectArg(s, "drawIndirect", indirectBuffer, "GPUBuffer", 1),
        offset: gpuWideArg(s, "drawIndirect", indirectOffset),
      });
    }],
    ["drawIndexedIndirect", function drawIndexedIndirect(indirectBuffer, indirectOffset) {
      const s = gpuRecordOf(this, GPU_RENDER_KINDS);
      need(arguments.length, 2, "drawIndexedIndirect", s.kind);
      gpuDraw(s, {
        c: "drawIndexedIndirect",
        buffer: gpuObjectArg(s, "drawIndexedIndirect", indirectBuffer, "GPUBuffer", 1),
        offset: gpuWideArg(s, "drawIndexedIndirect", indirectOffset),
      });
    }],
  ]);

  // A block's height in texels for a texture format.
  function gpuBlockHeight(format) {
    const head = StringSlice(format, 0, 3);
    if (StringSlice(format, 0, 2) === "bc" || head === "etc" || head === "eac") return 4;
    if (StringSlice(format, 0, 5) === "astc-") {
      // "astc-WxH-unorm...".
      const dims = StringSlice(format, 5, GpuIndexOf(format, "-", 5));
      return $Number(StringSlice(dims, GpuIndexOf(dims, "x") + 1));
    }
    return 1;
  }

  // ---- GPUBuffer ----
  class GPUBuffer {
    #id; #size; #usage; #state; #label = "";
    // While mapped: the range's start and bytes, whether it is written
    // back, and the array buffers handed out of it.
    #mapped = null;
    constructor(key, id, size, usage, mapped) {
      if (key !== KEY) throw illegalConstructor("GPUBuffer");
      this.#id = id;
      gpuThing(this, "GPUBuffer", id);
      this.#size = size;
      this.#usage = usage;
      this.#state = mapped ? "mapped" : "unmapped";
      if (mapped) {
        this.#mapped = { offset: 0, bytes: new $Uint8Array(size), write: true, ranges: [] };
      }
    }
    static {
      gpuBufferId = (buffer) => buffer.#id;
    }
    get size() { return this.#size; }
    get usage() { return this.#usage; }
    get mapState() { return this.#state; }
    get label() { return this.#label; }
    set label(value) { this.#label = str(value); }

    mapAsync(mode, offset, size) {
      try {
        return this.#mapAsync(arguments.length, mode, offset, size);
      } catch (e) {
        return rejected(e);
      }
    }

    #mapAsync(count, mode, offset, size) {
      const words = prefix("mapAsync", "GPUBuffer");
      need(count, 1, "mapAsync", "GPUBuffer");
      const m = gpuEnforced(mode, words, "unsigned long", GPU_LONG);
      const start = offset === undefined ? 0 : gpuEnforced(offset, words, "unsigned long long", GPU_LONG_LONG);
      const length = size === undefined ? MathMax(0, this.#size - start) : gpuEnforced(size, words, "unsigned long long", GPU_LONG_LONG);
      if (this.#state === "pending") {
        throw domError("OperationError", "Buffer already has a pending map.", "mapAsync", "GPUBuffer");
      }
      // The rest is the device's to validate (WebGPU 5.2.2): a buffer
      // already mapped, destroyed or invalid, a mode, offset or size not
      // its own, is a validation error and a rejection; a mapped buffer
      // stays mapped.
      const mappedBefore = this.#state === "mapped";
      if (!mappedBefore) this.#state = "pending";
      const buffer = this;
      const mapping = gpuPromise({ op: "mapAsync", buffer: this.#id, mode: m, offset: start, size: length });
      return PromiseThen(mapping, (bytes) => {
        if (mappedBefore) {
          throw domError("OperationError", "Buffer is already mapped.", "mapAsync", "GPUBuffer");
        }
        if (buffer.#state !== "pending") {
          throw domError("AbortError", "Buffer was unmapped before the mapping was resolved.", "mapAsync", "GPUBuffer");
        }
        buffer.#state = "mapped";
        const view = new $Uint8Array(length);
        if (bytes !== null) TypedArraySet(view, new $Uint8Array(bytes));
        buffer.#mapped = { offset: start, bytes: view, write: m === 2, ranges: [] };
        return undefined;
      }, (error) => {
        if (buffer.#state === "pending") buffer.#state = "unmapped";
        throw error;
      });
    }

    getMappedRange(offset, size) {
      const words = prefix("getMappedRange", "GPUBuffer");
      const start = offset === undefined ? 0 : gpuEnforced(offset, words, "unsigned long long", GPU_LONG_LONG);
      if (this.#state !== "mapped" || this.#mapped === null) {
        throw new $DOMException(words + "getMappedRange failed", "OperationError");
      }
      const mapped = this.#mapped;
      const end = mapped.offset + mapped.bytes.length;
      const length = size === undefined ? MathMax(0, end - start) : gpuEnforced(size, words, "unsigned long long", GPU_LONG_LONG);
      if (start % 8 !== 0 || length % 4 !== 0 || start < mapped.offset || start + length > end) {
        throw new $DOMException(words + "getMappedRange failed", "OperationError");
      }
      for (let i = 0; i < mapped.ranges.length; i++) {
        const range = mapped.ranges[i];
        if (start < range[0] + range[1] && range[0] < start + length) {
          throw new $DOMException(words + "getMappedRange failed", "OperationError");
        }
      }
      // A buffer of its own, read back at unmap when written.
      const out = new $ArrayBuffer(length);
      TypedArraySet(new $Uint8Array(out), TypedArraySubarray(mapped.bytes, start - mapped.offset, start - mapped.offset + length));
      ArrayPush(mapped.ranges, [start, length, out]);
      return out;
    }

    unmap() {
      if (this.#state === "pending") {
        this.#state = "unmapped";
        return;
      }
      if (this.#state !== "mapped" || this.#mapped === null) return;
      const mapped = this.#mapped;
      this.#mapped = null;
      this.#state = "unmapped";
      // What the page wrote goes back; then the ranges handed out are
      // detached.
      if (mapped.write) {
        for (let i = 0; i < mapped.ranges.length; i++) {
          const range = mapped.ranges[i];
          const at = range[0] - mapped.offset;
          TypedArraySet(TypedArraySubarray(mapped.bytes, at, at + range[1]), new $Uint8Array(range[2]));
        }
      }
      gpuCall({ op: "unmap", buffer: this.#id, offset: mapped.offset, write: mapped.write },
        mapped.write ? mapped.bytes : null);
      for (let i = 0; i < mapped.ranges.length; i++) native.wasmDetach(mapped.ranges[i][2]);
    }

    destroy() {
      if (this.#state !== "unmapped") this.unmap();
      gpuCall({ op: "destroyBuffer", buffer: this.#id }, null);
    }
  }
  expose(GPUBuffer, "GPUBuffer");

  const GpuArrayIndexOf = uncurry(Array.prototype.indexOf);
  // ---- The canvas (M41 step 4) ----
  // A configured context has a texture of its canvas's size, the same
  // until its frame presents it: after the frame's callbacks, it is read
  // back to the canvas's bitmap (the GPU process sends its rows) and
  // destroyed.
  const GPU_CANVAS_FORMATS = ["rgba8unorm", "bgra8unorm", "rgba16float"];
  let gpuDrawn = [];
  let gpuPresentContext;

  // A canvas's size, an element's or an OffscreenCanvas's.
  let gpuCanvasGetters = null;
  function gpuCanvasSize(canvas) {
    const t = WeakMapGet(canvasTargets, canvas);
    if (t !== undefined) return [t.width, t.height];
    if (gpuCanvasGetters === null) {
      const proto = MapGet(interfaces, "canvas").prototype;
      gpuCanvasGetters = [getOwnPropertyDescriptor(proto, "width").get, getOwnPropertyDescriptor(proto, "height").get];
    }
    return [ReflectApply(gpuCanvasGetters[0], canvas, []), ReflectApply(gpuCanvasGetters[1], canvas, [])];
  }

  class GPUCanvasContext {
    #canvas; #handle; #config = null; #texture = null;
    constructor(key, canvas, handle) {
      if (key !== KEY) throw illegalConstructor("GPUCanvasContext");
      this.#canvas = canvas;
      this.#handle = handle;
    }
    static {
      gpuPresentContext = (context) => {
        const texture = context.#texture;
        if (texture === null) return;
        context.#texture = null;
        const t = WeakMapGet(gpuThings, texture);
        const c = context.#config;
        const code = c === null ? -1 : GpuArrayIndexOf(GPU_CANVAS_FORMATS, c.format);
        if (code >= 0 && t.desc.size[0] > 0 && t.desc.size[1] > 0) {
          gpuCall({
            op: "present",
            device: t.device,
            texture: t.id,
            canvas: context.#handle,
            width: t.desc.size[0],
            height: t.desc.size[1],
            format: code,
            premultiplied: c.alphaMode === "premultiplied",
            opaque: c.alphaMode === "opaque",
          }, null);
        }
        gpuCall({ op: "destroyTexture", texture: t.id }, null);
      };
    }
    get canvas() { return this.#canvas; }

    configure(configuration) {
      need(arguments.length, 1, "configure", "GPUCanvasContext");
      const r = gpuReader(configuration, prefix("configure", "GPUCanvasContext"), "GPUCanvasConfiguration");
      const alphaMode = gpuEnum(r, "alphaMode", ["opaque", "premultiplied"], "GPUCanvasAlphaMode", "opaque");
      const colorSpace = gpuEnum(r, "colorSpace", ["srgb", "display-p3"], "PredefinedColorSpace", "srgb");
      const device = gpuNeed(r, "device");
      if (!(device instanceof GPUDevice)) {
        throw new $TypeError(gpuWords(r, "device") + "Failed to convert value to 'GPUDevice'.");
      }
      const format = gpuEnum(r, "format", GPU_FORMATS, "GPUTextureFormat");
      const tone = gpuSub(r, "toneMapping", "GPUCanvasToneMapping");
      const mode = tone === null ? "standard"
        : gpuEnum(tone, "mode", ["standard", "extended"], "GPUCanvasToneMappingMode", "standard");
      const usage = gpuU32(r, "usage", 16);
      const named = gpuList(r, "viewFormats", false);
      const viewFormats = [];
      for (let i = 0; i < named.length; i++) {
        ArrayPush(viewFormats, gpuEnumValue(named[i], GPU_FORMATS, gpuWords(r, "viewFormats"), "GPUTextureFormat"));
      }
      // A format not the canvas's throws; the texture's descriptor is
      // validated on the device's timeline (WebGPU 21.2.5): the canvas is
      // configured even so, its textures invalid.
      if (!listed(GPU_CANVAS_FORMATS, format)) {
        throw new $TypeError(prefix("configure", "GPUCanvasContext") + "Unsupported canvas context format '" +
          format + "'.");
      }
      this.#texture = null;
      const size = gpuCanvasSize(this.#canvas);
      let invalid = null;
      if (usage === 0 || (usage & ~0x1f) !== 0) {
        invalid = "The canvas's usage " + usage + " is not a valid texture usage.";
      } else if (format === "bgra8unorm" && (usage & 0x8) !== 0 && !device.features.has("bgra8unorm-storage")) {
        invalid = "The canvas's format bgra8unorm with STORAGE_BINDING needs the bgra8unorm-storage feature.";
      } else {
        for (let i = 0; i < viewFormats.length; i++) {
          const view = viewFormats[i];
          if (view !== format && !(format !== "rgba16float" && view === format + "-srgb")) {
            invalid = "The canvas's view format " + view + " is not compatible with " + format + ".";
            break;
          }
        }
      }
      if (invalid !== null) {
        gpuInvalid(gpuDeviceId(device), invalid);
      } else if (size[0] === 0 || size[1] === 0) {
        // The size is the canvas's now, not the configuration's.
        gpuInvalid(gpuDeviceId(device), "The canvas's size is zero.");
      }
      this.#config = { device, format, usage, viewFormats, colorSpace, toneMapping: { mode }, alphaMode, invalid };
    }

    unconfigure() {
      this.#config = null;
      this.#texture = null;
    }

    getConfiguration() {
      const c = this.#config;
      if (c === null) return null;
      return {
        device: c.device,
        format: c.format,
        usage: c.usage,
        viewFormats: ArraySlice(c.viewFormats, 0),
        colorSpace: c.colorSpace,
        toneMapping: { mode: c.toneMapping.mode },
        alphaMode: c.alphaMode,
      };
    }

    getCurrentTexture() {
      const c = this.#config;
      if (c === null) {
        throw domError("InvalidStateError", "context is not configured.", "getCurrentTexture",
          "GPUCanvasContext");
      }
      if (this.#texture === null) {
        const size = gpuCanvasSize(this.#canvas);
        const device = gpuDeviceId(c.device);
        const desc = {
          dimension: "2d",
          format: c.format,
          mipLevelCount: 1,
          sampleCount: 1,
          size: [size[0], size[1], 1],
          usage: c.usage,
          viewFormats: c.viewFormats,
        };
        // A configuration that was invalid makes an invalid texture,
        // never presented: its error again, and the page's descriptor.
        if (c.invalid !== null) {
          gpuInvalid(device, c.invalid);
          const id = gpuCall({ op: "createTexture", device, desc }, null).result.id;
          this.#texture = new GPUTexture(KEY, id, device, desc);
          return this.#texture;
        }
        // Copied from to present it: COPY_SRC besides the page's usage.
        const made = {
          dimension: "2d",
          format: c.format,
          mipLevelCount: 1,
          sampleCount: 1,
          size: desc.size,
          usage: c.usage | 1,
          viewFormats: c.viewFormats,
        };
        const id = gpuCall({ op: "createTexture", device, desc: made }, null).result.id;
        this.#texture = new GPUTexture(KEY, id, device, desc);
        ArrayPush(gpuDrawn, this);
        native.wantFrame();
      }
      return this.#texture;
    }
  }
  expose(GPUCanvasContext, "GPUCanvasContext");

  // The contexts drawn since the last frame, presented (its callbacks
  // have run).
  function gpuPresent() {
    if (gpuDrawn.length === 0) return;
    const drawn = gpuDrawn;
    gpuDrawn = [];
    for (let i = 0; i < drawn.length; i++) gpuPresentContext(drawn[i]);
  }

  // A canvas's WebGPU context, made at its first asking; null on a
  // context that is not secure, as `navigator.gpu` is not there.
  function gpuCanvasContext(canvas, handle) {
    if (WORKER || !native.secureContext()) return null;
    return new GPUCanvasContext(KEY, canvas, handle);
  }

  // An image source of copyExternalImageToTexture, as the host's
  // `imageBitmap` names it: [kind, which, base64 bytes or null, an
  // ImageData's height].
  function gpuImageSource(image, words) {
    const t = image !== null && typeof image === "object" ? WeakMapGet(canvasTargets, image) : undefined;
    const data = image !== null && typeof image === "object" ? WeakMapGet(imageDatas, image) : undefined;
    if (t !== undefined && t.kind === 1) return [3, t.id, null, 0];
    if (t !== undefined && t.kind === 5) {
      if (t.closed) {
        throw new $DOMException(words + "The image source is not usable.", "InvalidStateError");
      }
      return [4, t.id, null, 0];
    }
    if (data !== undefined) {
      return [6, data.width, ToBase64(new $Uint8Array(TypedArrayBuffer(data.data),
        TypedArrayOffset(data.data), TypedArrayLength(data.data))), data.height];
    }
    if (isNode(image) && kindOf(image) === 1) {
      const tag = native.tag(handleOf(image));
      if (tag === "img") return [1, handleOf(image), null, 0];
      if (tag === "canvas") return [2, handleOf(image), null, 0];
      if (tag === "video") return [5, handleOf(image), null, 0];
    }
    throw new $TypeError(words + "Failed to read the 'source' property from 'GPUCopyExternalImageSourceInfo': " +
      "The provided value is not of type '(HTMLCanvasElement or HTMLImageElement or HTMLVideoElement or " +
      "ImageBitmap or ImageData or OffscreenCanvas or VideoFrame)'.");
  }

  // A bitmap of `source`, cropped to `crop` ([x, y, w, h]) or whole:
  // the host's [number, width, height].
  function gpuBitmapOf(source, crop, flipY) {
    const numbers = [source[0], source[1], crop === null ? 0 : 1, crop === null ? 0 : crop[0],
      crop === null ? 0 : crop[1], crop === null ? 0 : crop[2], crop === null ? 0 : crop[3], -1, -1, 0,
      flipY ? 1 : 0, source[3]];
    const args = source[2] === null ? ["imageBitmap"] : ["imageBitmap", source[2]];
    for (let i = 0; i < numbers.length; i++) ArrayPush(args, numbers[i]);
    return ReflectApply(source[2] === null ? native.canvas : native.canvasData, undefined, args);
  }

  // ---- WebGL (M41 step 6, D40) -------------------------------------------
  // The contexts and their objects are the engine's numbers; every call
  // goes through `native.webgl(op, context, ...)`, GL's state there.
  // Web IDL's conversions and TypeErrors are here; GL's errors are the
  // engine's, read by getError.
  const WEBGL1_CONSTANTS = [
    "DEPTH_BUFFER_BIT",256, "STENCIL_BUFFER_BIT",1024, "COLOR_BUFFER_BIT",16384, "POINTS",0,
    "LINES",1, "LINE_LOOP",2, "LINE_STRIP",3, "TRIANGLES",4, "TRIANGLE_STRIP",5, "TRIANGLE_FAN",6,
    "ZERO",0, "ONE",1, "SRC_COLOR",768, "ONE_MINUS_SRC_COLOR",769, "SRC_ALPHA",770,
    "ONE_MINUS_SRC_ALPHA",771, "DST_ALPHA",772, "ONE_MINUS_DST_ALPHA",773, "DST_COLOR",774,
    "ONE_MINUS_DST_COLOR",775, "SRC_ALPHA_SATURATE",776, "FUNC_ADD",32774, "BLEND_EQUATION",32777,
    "BLEND_EQUATION_RGB",32777, "BLEND_EQUATION_ALPHA",34877, "FUNC_SUBTRACT",32778,
    "FUNC_REVERSE_SUBTRACT",32779, "BLEND_DST_RGB",32968, "BLEND_SRC_RGB",32969,
    "BLEND_DST_ALPHA",32970, "BLEND_SRC_ALPHA",32971, "CONSTANT_COLOR",32769,
    "ONE_MINUS_CONSTANT_COLOR",32770, "CONSTANT_ALPHA",32771, "ONE_MINUS_CONSTANT_ALPHA",32772,
    "BLEND_COLOR",32773, "ARRAY_BUFFER",34962, "ELEMENT_ARRAY_BUFFER",34963,
    "ARRAY_BUFFER_BINDING",34964, "ELEMENT_ARRAY_BUFFER_BINDING",34965, "STREAM_DRAW",35040,
    "STATIC_DRAW",35044, "DYNAMIC_DRAW",35048, "BUFFER_SIZE",34660, "BUFFER_USAGE",34661,
    "CURRENT_VERTEX_ATTRIB",34342, "FRONT",1028, "BACK",1029, "FRONT_AND_BACK",1032,
    "CULL_FACE",2884, "BLEND",3042, "DITHER",3024, "STENCIL_TEST",2960, "DEPTH_TEST",2929,
    "SCISSOR_TEST",3089, "POLYGON_OFFSET_FILL",32823, "SAMPLE_ALPHA_TO_COVERAGE",32926,
    "SAMPLE_COVERAGE",32928, "NO_ERROR",0, "INVALID_ENUM",1280, "INVALID_VALUE",1281,
    "INVALID_OPERATION",1282, "OUT_OF_MEMORY",1285, "CW",2304, "CCW",2305, "LINE_WIDTH",2849,
    "ALIASED_POINT_SIZE_RANGE",33901, "ALIASED_LINE_WIDTH_RANGE",33902, "CULL_FACE_MODE",2885,
    "FRONT_FACE",2886, "DEPTH_RANGE",2928, "DEPTH_WRITEMASK",2930, "DEPTH_CLEAR_VALUE",2931,
    "DEPTH_FUNC",2932, "STENCIL_CLEAR_VALUE",2961, "STENCIL_FUNC",2962, "STENCIL_FAIL",2964,
    "STENCIL_PASS_DEPTH_FAIL",2965, "STENCIL_PASS_DEPTH_PASS",2966, "STENCIL_REF",2967,
    "STENCIL_VALUE_MASK",2963, "STENCIL_WRITEMASK",2968, "STENCIL_BACK_FUNC",34816,
    "STENCIL_BACK_FAIL",34817, "STENCIL_BACK_PASS_DEPTH_FAIL",34818,
    "STENCIL_BACK_PASS_DEPTH_PASS",34819, "STENCIL_BACK_REF",36003,
    "STENCIL_BACK_VALUE_MASK",36004, "STENCIL_BACK_WRITEMASK",36005, "VIEWPORT",2978,
    "SCISSOR_BOX",3088, "COLOR_CLEAR_VALUE",3106, "COLOR_WRITEMASK",3107, "UNPACK_ALIGNMENT",3317,
    "PACK_ALIGNMENT",3333, "MAX_TEXTURE_SIZE",3379, "MAX_VIEWPORT_DIMS",3386, "SUBPIXEL_BITS",3408,
    "RED_BITS",3410, "GREEN_BITS",3411, "BLUE_BITS",3412, "ALPHA_BITS",3413, "DEPTH_BITS",3414,
    "STENCIL_BITS",3415, "POLYGON_OFFSET_UNITS",10752, "POLYGON_OFFSET_FACTOR",32824,
    "TEXTURE_BINDING_2D",32873, "SAMPLE_BUFFERS",32936, "SAMPLES",32937,
    "SAMPLE_COVERAGE_VALUE",32938, "SAMPLE_COVERAGE_INVERT",32939,
    "COMPRESSED_TEXTURE_FORMATS",34467, "DONT_CARE",4352, "FASTEST",4353, "NICEST",4354,
    "GENERATE_MIPMAP_HINT",33170, "BYTE",5120, "UNSIGNED_BYTE",5121, "SHORT",5122,
    "UNSIGNED_SHORT",5123, "INT",5124, "UNSIGNED_INT",5125, "FLOAT",5126, "DEPTH_COMPONENT",6402,
    "ALPHA",6406, "RGB",6407, "RGBA",6408, "LUMINANCE",6409, "LUMINANCE_ALPHA",6410,
    "UNSIGNED_SHORT_4_4_4_4",32819, "UNSIGNED_SHORT_5_5_5_1",32820, "UNSIGNED_SHORT_5_6_5",33635,
    "FRAGMENT_SHADER",35632, "VERTEX_SHADER",35633, "MAX_VERTEX_ATTRIBS",34921,
    "MAX_VERTEX_UNIFORM_VECTORS",36347, "MAX_VARYING_VECTORS",36348,
    "MAX_COMBINED_TEXTURE_IMAGE_UNITS",35661, "MAX_VERTEX_TEXTURE_IMAGE_UNITS",35660,
    "MAX_TEXTURE_IMAGE_UNITS",34930, "MAX_FRAGMENT_UNIFORM_VECTORS",36349, "SHADER_TYPE",35663,
    "DELETE_STATUS",35712, "LINK_STATUS",35714, "VALIDATE_STATUS",35715, "ATTACHED_SHADERS",35717,
    "ACTIVE_UNIFORMS",35718, "ACTIVE_ATTRIBUTES",35721, "SHADING_LANGUAGE_VERSION",35724,
    "CURRENT_PROGRAM",35725, "NEVER",512, "LESS",513, "EQUAL",514, "LEQUAL",515, "GREATER",516,
    "NOTEQUAL",517, "GEQUAL",518, "ALWAYS",519, "KEEP",7680, "REPLACE",7681, "INCR",7682,
    "DECR",7683, "INVERT",5386, "INCR_WRAP",34055, "DECR_WRAP",34056, "VENDOR",7936,
    "RENDERER",7937, "VERSION",7938, "NEAREST",9728, "LINEAR",9729, "NEAREST_MIPMAP_NEAREST",9984,
    "LINEAR_MIPMAP_NEAREST",9985, "NEAREST_MIPMAP_LINEAR",9986, "LINEAR_MIPMAP_LINEAR",9987,
    "TEXTURE_MAG_FILTER",10240, "TEXTURE_MIN_FILTER",10241, "TEXTURE_WRAP_S",10242,
    "TEXTURE_WRAP_T",10243, "TEXTURE_2D",3553, "TEXTURE",5890, "TEXTURE_CUBE_MAP",34067,
    "TEXTURE_BINDING_CUBE_MAP",34068, "TEXTURE_CUBE_MAP_POSITIVE_X",34069,
    "TEXTURE_CUBE_MAP_NEGATIVE_X",34070, "TEXTURE_CUBE_MAP_POSITIVE_Y",34071,
    "TEXTURE_CUBE_MAP_NEGATIVE_Y",34072, "TEXTURE_CUBE_MAP_POSITIVE_Z",34073,
    "TEXTURE_CUBE_MAP_NEGATIVE_Z",34074, "MAX_CUBE_MAP_TEXTURE_SIZE",34076, "TEXTURE0",33984,
    "TEXTURE1",33985, "TEXTURE2",33986, "TEXTURE3",33987, "TEXTURE4",33988, "TEXTURE5",33989,
    "TEXTURE6",33990, "TEXTURE7",33991, "TEXTURE8",33992, "TEXTURE9",33993, "TEXTURE10",33994,
    "TEXTURE11",33995, "TEXTURE12",33996, "TEXTURE13",33997, "TEXTURE14",33998, "TEXTURE15",33999,
    "TEXTURE16",34000, "TEXTURE17",34001, "TEXTURE18",34002, "TEXTURE19",34003, "TEXTURE20",34004,
    "TEXTURE21",34005, "TEXTURE22",34006, "TEXTURE23",34007, "TEXTURE24",34008, "TEXTURE25",34009,
    "TEXTURE26",34010, "TEXTURE27",34011, "TEXTURE28",34012, "TEXTURE29",34013, "TEXTURE30",34014,
    "TEXTURE31",34015, "ACTIVE_TEXTURE",34016, "REPEAT",10497, "CLAMP_TO_EDGE",33071,
    "MIRRORED_REPEAT",33648, "FLOAT_VEC2",35664, "FLOAT_VEC3",35665, "FLOAT_VEC4",35666,
    "INT_VEC2",35667, "INT_VEC3",35668, "INT_VEC4",35669, "BOOL",35670, "BOOL_VEC2",35671,
    "BOOL_VEC3",35672, "BOOL_VEC4",35673, "FLOAT_MAT2",35674, "FLOAT_MAT3",35675,
    "FLOAT_MAT4",35676, "SAMPLER_2D",35678, "SAMPLER_CUBE",35680,
    "VERTEX_ATTRIB_ARRAY_ENABLED",34338, "VERTEX_ATTRIB_ARRAY_SIZE",34339,
    "VERTEX_ATTRIB_ARRAY_STRIDE",34340, "VERTEX_ATTRIB_ARRAY_TYPE",34341,
    "VERTEX_ATTRIB_ARRAY_NORMALIZED",34922, "VERTEX_ATTRIB_ARRAY_POINTER",34373,
    "VERTEX_ATTRIB_ARRAY_BUFFER_BINDING",34975, "IMPLEMENTATION_COLOR_READ_TYPE",35738,
    "IMPLEMENTATION_COLOR_READ_FORMAT",35739, "COMPILE_STATUS",35713, "LOW_FLOAT",36336,
    "MEDIUM_FLOAT",36337, "HIGH_FLOAT",36338, "LOW_INT",36339, "MEDIUM_INT",36340,
    "HIGH_INT",36341, "FRAMEBUFFER",36160, "RENDERBUFFER",36161, "RGBA4",32854, "RGB5_A1",32855,
    "RGB565",36194, "DEPTH_COMPONENT16",33189, "STENCIL_INDEX8",36168, "DEPTH_STENCIL",34041,
    "RENDERBUFFER_WIDTH",36162, "RENDERBUFFER_HEIGHT",36163, "RENDERBUFFER_INTERNAL_FORMAT",36164,
    "RENDERBUFFER_RED_SIZE",36176, "RENDERBUFFER_GREEN_SIZE",36177, "RENDERBUFFER_BLUE_SIZE",36178,
    "RENDERBUFFER_ALPHA_SIZE",36179, "RENDERBUFFER_DEPTH_SIZE",36180,
    "RENDERBUFFER_STENCIL_SIZE",36181, "FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE",36048,
    "FRAMEBUFFER_ATTACHMENT_OBJECT_NAME",36049, "FRAMEBUFFER_ATTACHMENT_TEXTURE_LEVEL",36050,
    "FRAMEBUFFER_ATTACHMENT_TEXTURE_CUBE_MAP_FACE",36051, "COLOR_ATTACHMENT0",36064,
    "DEPTH_ATTACHMENT",36096, "STENCIL_ATTACHMENT",36128, "DEPTH_STENCIL_ATTACHMENT",33306,
    "NONE",0, "FRAMEBUFFER_COMPLETE",36053, "FRAMEBUFFER_INCOMPLETE_ATTACHMENT",36054,
    "FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT",36055, "FRAMEBUFFER_INCOMPLETE_DIMENSIONS",36057,
    "FRAMEBUFFER_UNSUPPORTED",36061, "FRAMEBUFFER_BINDING",36006, "RENDERBUFFER_BINDING",36007,
    "MAX_RENDERBUFFER_SIZE",34024, "INVALID_FRAMEBUFFER_OPERATION",1286,
    "UNPACK_FLIP_Y_WEBGL",37440, "UNPACK_PREMULTIPLY_ALPHA_WEBGL",37441,
    "CONTEXT_LOST_WEBGL",37442, "UNPACK_COLORSPACE_CONVERSION_WEBGL",37443,
    "BROWSER_DEFAULT_WEBGL",37444,
  ];
  const WEBGL2_CONSTANTS = [
    "READ_BUFFER",3074, "UNPACK_ROW_LENGTH",3314, "UNPACK_SKIP_ROWS",3315,
    "UNPACK_SKIP_PIXELS",3316, "PACK_ROW_LENGTH",3330, "PACK_SKIP_ROWS",3331,
    "PACK_SKIP_PIXELS",3332, "COLOR",6144, "DEPTH",6145, "STENCIL",6146, "RED",6403, "RGB8",32849,
    "RGBA8",32856, "RGB10_A2",32857, "TEXTURE_BINDING_3D",32874, "UNPACK_SKIP_IMAGES",32877,
    "UNPACK_IMAGE_HEIGHT",32878, "TEXTURE_3D",32879, "TEXTURE_WRAP_R",32882,
    "MAX_3D_TEXTURE_SIZE",32883, "UNSIGNED_INT_2_10_10_10_REV",33640,
    "MAX_ELEMENTS_VERTICES",33000, "MAX_ELEMENTS_INDICES",33001, "TEXTURE_MIN_LOD",33082,
    "TEXTURE_MAX_LOD",33083, "TEXTURE_BASE_LEVEL",33084, "TEXTURE_MAX_LEVEL",33085, "MIN",32775,
    "MAX",32776, "DEPTH_COMPONENT24",33190, "MAX_TEXTURE_LOD_BIAS",34045,
    "TEXTURE_COMPARE_MODE",34892, "TEXTURE_COMPARE_FUNC",34893, "CURRENT_QUERY",34917,
    "QUERY_RESULT",34918, "QUERY_RESULT_AVAILABLE",34919, "STREAM_READ",35041, "STREAM_COPY",35042,
    "STATIC_READ",35045, "STATIC_COPY",35046, "DYNAMIC_READ",35049, "DYNAMIC_COPY",35050,
    "MAX_DRAW_BUFFERS",34852, "DRAW_BUFFER0",34853, "DRAW_BUFFER1",34854, "DRAW_BUFFER2",34855,
    "DRAW_BUFFER3",34856, "DRAW_BUFFER4",34857, "DRAW_BUFFER5",34858, "DRAW_BUFFER6",34859,
    "DRAW_BUFFER7",34860, "DRAW_BUFFER8",34861, "DRAW_BUFFER9",34862, "DRAW_BUFFER10",34863,
    "DRAW_BUFFER11",34864, "DRAW_BUFFER12",34865, "DRAW_BUFFER13",34866, "DRAW_BUFFER14",34867,
    "DRAW_BUFFER15",34868, "MAX_FRAGMENT_UNIFORM_COMPONENTS",35657,
    "MAX_VERTEX_UNIFORM_COMPONENTS",35658, "SAMPLER_3D",35679, "SAMPLER_2D_SHADOW",35682,
    "FRAGMENT_SHADER_DERIVATIVE_HINT",35723, "PIXEL_PACK_BUFFER",35051,
    "PIXEL_UNPACK_BUFFER",35052, "PIXEL_PACK_BUFFER_BINDING",35053,
    "PIXEL_UNPACK_BUFFER_BINDING",35055, "SRGB",35904, "SRGB8",35905, "SRGB8_ALPHA8",35907,
    "COMPARE_REF_TO_TEXTURE",34894, "RGBA32F",34836, "RGB32F",34837, "RGBA16F",34842,
    "RGB16F",34843, "VERTEX_ATTRIB_ARRAY_INTEGER",35069, "MAX_ARRAY_TEXTURE_LAYERS",35071,
    "MIN_PROGRAM_TEXEL_OFFSET",35076, "MAX_PROGRAM_TEXEL_OFFSET",35077,
    "MAX_VARYING_COMPONENTS",35659, "TEXTURE_2D_ARRAY",35866, "TEXTURE_BINDING_2D_ARRAY",35869,
    "R11F_G11F_B10F",35898, "UNSIGNED_INT_10F_11F_11F_REV",35899, "RGB9_E5",35901,
    "UNSIGNED_INT_5_9_9_9_REV",35902, "TRANSFORM_FEEDBACK_BUFFER_MODE",35967,
    "MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS",35968, "TRANSFORM_FEEDBACK_VARYINGS",35971,
    "TRANSFORM_FEEDBACK_BUFFER_START",35972, "TRANSFORM_FEEDBACK_BUFFER_SIZE",35973,
    "TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN",35976, "RASTERIZER_DISCARD",35977,
    "MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS",35978,
    "MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS",35979, "INTERLEAVED_ATTRIBS",35980,
    "SEPARATE_ATTRIBS",35981, "TRANSFORM_FEEDBACK_BUFFER",35982,
    "TRANSFORM_FEEDBACK_BUFFER_BINDING",35983, "RGBA32UI",36208, "RGB32UI",36209, "RGBA16UI",36214,
    "RGB16UI",36215, "RGBA8UI",36220, "RGB8UI",36221, "RGBA32I",36226, "RGB32I",36227,
    "RGBA16I",36232, "RGB16I",36233, "RGBA8I",36238, "RGB8I",36239, "RED_INTEGER",36244,
    "RGB_INTEGER",36248, "RGBA_INTEGER",36249, "SAMPLER_2D_ARRAY",36289,
    "SAMPLER_2D_ARRAY_SHADOW",36292, "SAMPLER_CUBE_SHADOW",36293, "UNSIGNED_INT_VEC2",36294,
    "UNSIGNED_INT_VEC3",36295, "UNSIGNED_INT_VEC4",36296, "INT_SAMPLER_2D",36298,
    "INT_SAMPLER_3D",36299, "INT_SAMPLER_CUBE",36300, "INT_SAMPLER_2D_ARRAY",36303,
    "UNSIGNED_INT_SAMPLER_2D",36306, "UNSIGNED_INT_SAMPLER_3D",36307,
    "UNSIGNED_INT_SAMPLER_CUBE",36308, "UNSIGNED_INT_SAMPLER_2D_ARRAY",36311,
    "DEPTH_COMPONENT32F",36012, "DEPTH32F_STENCIL8",36013, "FLOAT_32_UNSIGNED_INT_24_8_REV",36269,
    "FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING",33296, "FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE",33297,
    "FRAMEBUFFER_ATTACHMENT_RED_SIZE",33298, "FRAMEBUFFER_ATTACHMENT_GREEN_SIZE",33299,
    "FRAMEBUFFER_ATTACHMENT_BLUE_SIZE",33300, "FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE",33301,
    "FRAMEBUFFER_ATTACHMENT_DEPTH_SIZE",33302, "FRAMEBUFFER_ATTACHMENT_STENCIL_SIZE",33303,
    "FRAMEBUFFER_DEFAULT",33304, "UNSIGNED_INT_24_8",34042, "DEPTH24_STENCIL8",35056,
    "UNSIGNED_NORMALIZED",35863, "DRAW_FRAMEBUFFER_BINDING",36006, "READ_FRAMEBUFFER",36008,
    "DRAW_FRAMEBUFFER",36009, "READ_FRAMEBUFFER_BINDING",36010, "RENDERBUFFER_SAMPLES",36011,
    "FRAMEBUFFER_ATTACHMENT_TEXTURE_LAYER",36052, "MAX_COLOR_ATTACHMENTS",36063,
    "COLOR_ATTACHMENT1",36065, "COLOR_ATTACHMENT2",36066, "COLOR_ATTACHMENT3",36067,
    "COLOR_ATTACHMENT4",36068, "COLOR_ATTACHMENT5",36069, "COLOR_ATTACHMENT6",36070,
    "COLOR_ATTACHMENT7",36071, "COLOR_ATTACHMENT8",36072, "COLOR_ATTACHMENT9",36073,
    "COLOR_ATTACHMENT10",36074, "COLOR_ATTACHMENT11",36075, "COLOR_ATTACHMENT12",36076,
    "COLOR_ATTACHMENT13",36077, "COLOR_ATTACHMENT14",36078, "COLOR_ATTACHMENT15",36079,
    "FRAMEBUFFER_INCOMPLETE_MULTISAMPLE",36182, "MAX_SAMPLES",36183, "HALF_FLOAT",5131, "RG",33319,
    "RG_INTEGER",33320, "R8",33321, "RG8",33323, "R16F",33325, "R32F",33326, "RG16F",33327,
    "RG32F",33328, "R8I",33329, "R8UI",33330, "R16I",33331, "R16UI",33332, "R32I",33333,
    "R32UI",33334, "RG8I",33335, "RG8UI",33336, "RG16I",33337, "RG16UI",33338, "RG32I",33339,
    "RG32UI",33340, "VERTEX_ARRAY_BINDING",34229, "R8_SNORM",36756, "RG8_SNORM",36757,
    "RGB8_SNORM",36758, "RGBA8_SNORM",36759, "SIGNED_NORMALIZED",36764, "COPY_READ_BUFFER",36662,
    "COPY_WRITE_BUFFER",36663, "COPY_READ_BUFFER_BINDING",36662, "COPY_WRITE_BUFFER_BINDING",36663,
    "UNIFORM_BUFFER",35345, "UNIFORM_BUFFER_BINDING",35368, "UNIFORM_BUFFER_START",35369,
    "UNIFORM_BUFFER_SIZE",35370, "MAX_VERTEX_UNIFORM_BLOCKS",35371,
    "MAX_FRAGMENT_UNIFORM_BLOCKS",35373, "MAX_COMBINED_UNIFORM_BLOCKS",35374,
    "MAX_UNIFORM_BUFFER_BINDINGS",35375, "MAX_UNIFORM_BLOCK_SIZE",35376,
    "MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS",35377,
    "MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS",35379, "UNIFORM_BUFFER_OFFSET_ALIGNMENT",35380,
    "ACTIVE_UNIFORM_BLOCKS",35382, "UNIFORM_TYPE",35383, "UNIFORM_SIZE",35384,
    "UNIFORM_BLOCK_INDEX",35386, "UNIFORM_OFFSET",35387, "UNIFORM_ARRAY_STRIDE",35388,
    "UNIFORM_MATRIX_STRIDE",35389, "UNIFORM_IS_ROW_MAJOR",35390, "UNIFORM_BLOCK_BINDING",35391,
    "UNIFORM_BLOCK_DATA_SIZE",35392, "UNIFORM_BLOCK_ACTIVE_UNIFORMS",35394,
    "UNIFORM_BLOCK_ACTIVE_UNIFORM_INDICES",35395,
    "UNIFORM_BLOCK_REFERENCED_BY_VERTEX_SHADER",35396,
    "UNIFORM_BLOCK_REFERENCED_BY_FRAGMENT_SHADER",35398, "INVALID_INDEX",4294967295,
    "MAX_VERTEX_OUTPUT_COMPONENTS",37154, "MAX_FRAGMENT_INPUT_COMPONENTS",37157,
    "MAX_SERVER_WAIT_TIMEOUT",37137, "OBJECT_TYPE",37138, "SYNC_CONDITION",37139,
    "SYNC_STATUS",37140, "SYNC_FLAGS",37141, "SYNC_FENCE",37142,
    "SYNC_GPU_COMMANDS_COMPLETE",37143, "UNSIGNALED",37144, "SIGNALED",37145,
    "ALREADY_SIGNALED",37146, "TIMEOUT_EXPIRED",37147, "CONDITION_SATISFIED",37148,
    "WAIT_FAILED",37149, "SYNC_FLUSH_COMMANDS_BIT",1, "VERTEX_ATTRIB_ARRAY_DIVISOR",35070,
    "ANY_SAMPLES_PASSED",35887, "ANY_SAMPLES_PASSED_CONSERVATIVE",36202, "SAMPLER_BINDING",35097,
    "RGB10_A2UI",36975, "INT_2_10_10_10_REV",36255, "TRANSFORM_FEEDBACK",36386,
    "TRANSFORM_FEEDBACK_PAUSED",36387, "TRANSFORM_FEEDBACK_ACTIVE",36388,
    "TRANSFORM_FEEDBACK_BINDING",36389, "TEXTURE_IMMUTABLE_FORMAT",37167,
    "MAX_ELEMENT_INDEX",36203, "TEXTURE_IMMUTABLE_LEVELS",33503,
    "MAX_CLIENT_WAIT_TIMEOUT_WEBGL",37447,
  ];
  const webglRecords = new $WeakMap();
  const webglObjects = new $WeakMap();
  let webglDrawn = [];
  // The documents' contexts, for those lost to make room.
  const webglLive = [];

  function webglRecord(context) {
    const r = (typeof context === "object" && context !== null) ? WeakMapGet(webglRecords, context) : undefined;
    if (r === undefined) throw illegalInvocation();
    return r;
  }

  // A WebGL context's version, 0 for anything else.
  function webglVersionOf(context) {
    const r = (typeof context === "object" && context !== null) ? WeakMapGet(webglRecords, context) : undefined;
    return r === undefined ? 0 : r.version;
  }

  // GL's integer types, as Web IDL converts them (no [EnforceRange]).
  const glu = (v) => $Number(v) >>> 0;
  const gli = (v) => $Number(v) | 0;
  const glf = (v) => $Number(v);
  const glll = (v) => {
    const n = $Number(v);
    return n !== n || n === Infinity || n === -Infinity ? 0 : MathTrunc(n);
  };

  // The WebGL objects, each a number of its context's.
  function webglObjectClass(name) {
    const cls = class {
      constructor(key) {
        if (key !== KEY) throw illegalConstructor(name);
      }
    };
    defineProperty(cls, "name", { value: name });
    expose(cls, name);
    return cls;
  }
  const WebGLBuffer = webglObjectClass("WebGLBuffer");
  const WebGLShader = webglObjectClass("WebGLShader");
  const WebGLProgram = webglObjectClass("WebGLProgram");
  const WebGLTexture = webglObjectClass("WebGLTexture");
  const WebGLRenderbuffer = webglObjectClass("WebGLRenderbuffer");
  const WebGLFramebuffer = webglObjectClass("WebGLFramebuffer");
  const WebGLUniformLocation = webglObjectClass("WebGLUniformLocation");
  const WebGLVertexArrayObject = webglObjectClass("WebGLVertexArrayObject");
  const WebGLQuery = webglObjectClass("WebGLQuery");
  const WebGLSync = webglObjectClass("WebGLSync");
  const WebGLTransformFeedback = webglObjectClass("WebGLTransformFeedback");
  const WebGLSampler = webglObjectClass("WebGLSampler");
  // OES_vertex_array_object's, with no interface object of its own.
  const WebGLVertexArrayObjectOES = class {
    constructor(key) {
      if (key !== KEY) throw illegalConstructor("WebGLVertexArrayObjectOES");
    }
  };
  defineProperty(WebGLVertexArrayObjectOES.prototype, toStringTag,
    { value: "WebGLVertexArrayObjectOES", configurable: true });
  const WEBGL_KINDS = {
    V: [WebGLVertexArrayObject, "WebGLVertexArrayObject"],
    Q: [WebGLQuery, "WebGLQuery"],
    Y: [WebGLSync, "WebGLSync"],
    X: [WebGLTransformFeedback, "WebGLTransformFeedback"],
    M: [WebGLSampler, "WebGLSampler"],
    O: [WebGLVertexArrayObjectOES, "WebGLVertexArrayObjectOES"],
    B: [WebGLBuffer, "WebGLBuffer"],
    S: [WebGLShader, "WebGLShader"],
    P: [WebGLProgram, "WebGLProgram"],
    T: [WebGLTexture, "WebGLTexture"],
    R: [WebGLRenderbuffer, "WebGLRenderbuffer"],
    F: [WebGLFramebuffer, "WebGLFramebuffer"],
    L: [WebGLUniformLocation, "WebGLUniformLocation"],
  };

  // A new object of kind `k` numbered `id` (none for 0 or nothing).
  function webglMake(r, k, id) {
    if (typeof id !== "number" || id <= 0) return null;
    const object = new WEBGL_KINDS[k][0](KEY);
    WeakMapSet(webglObjects, object, { id, context: r, kind: k, generation: r.generation, fresh: false });
    MapSet(r.objects, id, object);
    return object;
  }

  class WebGLActiveInfo {
    #name; #size; #type;
    constructor(key, name, size, type) {
      if (key !== KEY) throw illegalConstructor("WebGLActiveInfo");
      this.#name = name;
      this.#size = size;
      this.#type = type;
    }
    get name() { return this.#name; }
    get size() { return this.#size; }
    get type() { return this.#type; }
  }
  expose(WebGLActiveInfo, "WebGLActiveInfo");

  class WebGLShaderPrecisionFormat {
    #min; #max; #precision;
    constructor(key, min, max, precision) {
      if (key !== KEY) throw illegalConstructor("WebGLShaderPrecisionFormat");
      this.#min = min;
      this.#max = max;
      this.#precision = precision;
    }
    get rangeMin() { return this.#min; }
    get rangeMax() { return this.#max; }
    get precision() { return this.#precision; }
  }
  expose(WebGLShaderPrecisionFormat, "WebGLShaderPrecisionFormat");

  class WebGLContextEvent extends Event {
    #message;
    constructor(type, init) {
      needToConstruct(arguments.length, 1, "WebGLContextEvent");
      super(type, init);
      const m = init !== undefined && init !== null ? init.statusMessage : undefined;
      this.#message = m === undefined ? "" : str(m);
    }
    get statusMessage() { return this.#message; }
  }
  expose(WebGLContextEvent, "WebGLContextEvent");

  // An argument's conversion by its letter: e/u GLenum or GLuint, i
  // GLint, f GLfloat, b GLboolean, l GLintptr, s DOMString; B S P T R F
  // L an object of that kind, nullable when followed by "?".
  // An object of another context makes GL's INVALID_OPERATION and the
  // call does nothing (`undefined` here).
  function webglArgs(r, spec, args, method, iface) {
    const out = [];
    let a = 0;
    for (let k = 0; k < spec.length; k++) {
      const c = spec[k];
      if (c === "?") continue;
      const v = args[a++];
      switch (c) {
        case "e": case "u": ArrayPush(out, glu(v)); break;
        case "i": ArrayPush(out, gli(v)); break;
        case "f": ArrayPush(out, glf(v)); break;
        case "b": ArrayPush(out, !!v); break;
        case "l": ArrayPush(out, glll(v)); break;
        case "s": ArrayPush(out, str(v)); break;
        default: {
          const nullable = spec[k + 1] === "?";
          if (v === null || v === undefined) {
            if (!nullable) {
              throw new $TypeError(prefix(method, iface) + "parameter " + a + " is not of type '" + WEBGL_KINDS[c][1] + "'.");
            }
            ArrayPush(out, 0);
            break;
          }
          const o = typeof v === "object" ? WeakMapGet(webglObjects, v) : undefined;
          if (o === undefined || o.kind !== c) {
            throw new $TypeError(prefix(method, iface) + "parameter " + a + " is not of type '" + WEBGL_KINDS[c][1] + "'.");
          }
          // Another context's, or one from before a restoration (an is*
          // call's is false, with no error).
          if (o.context !== r || o.generation !== r.generation) {
            if (StringSlice(method, 0, 2) !== "is") native.webgl("error", r.id, 0x0502);
            return null;
          }
          ArrayPush(out, o.id);
        }
      }
    }
    return out;
  }

  // The calls that convert their arguments and pass them on: name,
  // arguments, the least count, and what they give (v nothing, n a
  // number, b a boolean, s a string or null, or an object's letter).
  const WEBGL_SIMPLE = [
    ["activeTexture", "e", 1, "v"], ["attachShader", "PS", 2, "v"], ["bindAttribLocation", "Pus", 3, "v"],
    ["bindBuffer", "eB?", 2, "v"], ["blendColor", "ffff", 4, "v"], ["blendEquation", "e", 1, "v"],
    ["blendEquationSeparate", "ee", 2, "v"], ["blendFunc", "ee", 2, "v"], ["blendFuncSeparate", "eeee", 4, "v"],
    ["clearColor", "ffff", 4, "v"], ["clearDepth", "f", 1, "v"], ["clearStencil", "i", 1, "v"],
    ["colorMask", "bbbb", 4, "v"], ["compileShader", "S", 1, "v"], ["cullFace", "e", 1, "v"],
    ["deleteBuffer", "B?", 1, "v"], ["deleteProgram", "P?", 1, "v"], ["deleteShader", "S?", 1, "v"],
    ["depthFunc", "e", 1, "v"], ["depthMask", "b", 1, "v"], ["depthRange", "ff", 2, "v"],
    ["detachShader", "PS", 2, "v"], ["disable", "e", 1, "v"], ["disableVertexAttribArray", "u", 1, "v"],
    ["enable", "e", 1, "v"], ["enableVertexAttribArray", "u", 1, "v"], ["finish", "", 0, "v"], ["flush", "", 0, "v"],
    ["frontFace", "e", 1, "v"], ["getAttribLocation", "Ps", 2, "n"], ["getBufferParameter", "ee", 2, "n"],
    ["getProgramInfoLog", "P", 1, "s"], ["getShaderInfoLog", "S", 1, "s"], ["getShaderSource", "S", 1, "s"],
    ["getVertexAttribOffset", "ue", 2, "n"], ["hint", "ee", 2, "v"], ["isBuffer", "B?", 1, "b"],
    ["isEnabled", "e", 1, "b"], ["isProgram", "P?", 1, "b"], ["isShader", "S?", 1, "b"], ["lineWidth", "f", 1, "v"],
    ["linkProgram", "P", 1, "v"], ["pixelStorei", "ei", 2, "v"], ["polygonOffset", "ff", 2, "v"],
    ["sampleCoverage", "fb", 2, "v"], ["scissor", "iiii", 4, "v"], ["shaderSource", "Ss", 2, "v"],
    ["stencilFunc", "eiu", 3, "v"], ["stencilFuncSeparate", "eeiu", 4, "v"], ["stencilMask", "u", 1, "v"],
    ["stencilMaskSeparate", "eu", 2, "v"], ["stencilOp", "eee", 3, "v"], ["stencilOpSeparate", "eeee", 4, "v"],
    ["useProgram", "P?", 1, "v"], ["validateProgram", "P", 1, "v"], ["viewport", "iiii", 4, "v"],
    ["vertexAttribPointer", "uiebil", 6, "v"], ["createBuffer", "", 0, "B"], ["createProgram", "", 0, "P"],
    ["createShader", "e", 1, "S"], ["getUniformLocation", "Ps", 2, "L"],
    ["createTexture", "", 0, "T"], ["createRenderbuffer", "", 0, "R"], ["createFramebuffer", "", 0, "F"],
    ["bindTexture", "eT?", 2, "v"], ["bindRenderbuffer", "eR?", 2, "v"], ["bindFramebuffer", "eF?", 2, "v"],
    ["deleteTexture", "T?", 1, "v"], ["deleteRenderbuffer", "R?", 1, "v"], ["deleteFramebuffer", "F?", 1, "v"],
    ["isTexture", "T?", 1, "b"], ["isRenderbuffer", "R?", 1, "b"], ["isFramebuffer", "F?", 1, "b"],
    ["texParameteri", "eei", 3, "v"], ["texParameterf", "eef", 3, "v"], ["getTexParameter", "ee", 2, "n"],
    ["generateMipmap", "e", 1, "v"], ["checkFramebufferStatus", "e", 1, "n"],
    ["framebufferTexture2D", "eeeT?i", 5, "v"], ["framebufferRenderbuffer", "eeeR?", 4, "v"],
    ["renderbufferStorage", "eeii", 4, "v"], ["getRenderbufferParameter", "ee", 2, "n"],
    ["copyTexImage2D", "eieiiiii", 8, "v"], ["copyTexSubImage2D", "eiiiiiii", 8, "v"],
  ];

  // Drawing calls, whose canvas is presented after the frame.
  const WEBGL_DRAWS = [["clear", "u", 1], ["drawArrays", "eii", 3], ["drawElements", "eiel", 4]];

  function webglDrew(r) {
    if (!r.drawn) {
      r.drawn = true;
      ArrayPush(webglDrawn, r);
      native.wantFrame();
    }
  }

  // The WebGL contexts drawn since the last frame, presented.
  function webglPresent() {
    if (webglDrawn.length === 0) return;
    const drawn = webglDrawn;
    webglDrawn = [];
    for (let i = 0; i < drawn.length; i++) {
      drawn[i].drawn = false;
      native.webgl("present", drawn[i].id);
    }
  }

  // A typed array's or a sequence's numbers.
  function webglNumbers(v, method, iface) {
    if (ArrayBufferIsView(v) && !(v instanceof $DataView)) {
      const out = [];
      for (let i = 0; i < v.length; i++) ArrayPush(out, v[i]);
      return out;
    }
    return sequence(v, method, iface, (x) => $Number(x));
  }

  // The bytes of a BufferSource, or null for none.
  function webglBytes(v, method, iface, index) {
    if (v instanceof $ArrayBuffer) return new $Uint8Array(v);
    if (ArrayBufferIsView(v)) return gpuViewBytes(v)[0];
    throw new $TypeError(prefix(method, iface) + "parameter " + index + " is not of type 'ArrayBuffer' or 'ArrayBufferView'.");
  }

  const WEBGL_ATTRIBUTES = ["alpha", "antialias", "depth", "desynchronized", "failIfMajorPerformanceCaveat",
    "powerPreference", "premultipliedAlpha", "preserveDrawingBuffer", "stencil", "xrCompatible"];

  // The context attributes a getContext asks for, read as Web IDL reads
  // the dictionary.
  function webglAttributes(options) {
    const a = { alpha: true, antialias: true, depth: true, desynchronized: false,
      failIfMajorPerformanceCaveat: false, powerPreference: "default", premultipliedAlpha: true,
      preserveDrawingBuffer: false, stencil: false, xrCompatible: false };
    if (options === undefined || options === null || (typeof options !== "object" && typeof options !== "function")) {
      return a;
    }
    for (let i = 0; i < WEBGL_ATTRIBUTES.length; i++) {
      const name = WEBGL_ATTRIBUTES[i];
      const v = options[name];
      if (v === undefined) continue;
      if (name === "powerPreference") {
        const s = str(v);
        if (s !== "default" && s !== "low-power" && s !== "high-performance") {
          throw new $TypeError("Failed to execute 'getContext' on 'HTMLCanvasElement': Failed to read the 'powerPreference' property from 'WebGLContextAttributes': The provided value '" + s + "' is not a valid enum value of type WebGLPowerPreference.");
        }
        a.powerPreference = s;
      } else {
        a[name] = !!v;
      }
    }
    return a;
  }

  // A context of `version` for `canvas` (its handle), or null without a
  // GPU.
  function webglContext(canvas, handle, version, options) {
    const attributes = webglAttributes(options);
    const id = native.webgl("create", handle, version, attributes.alpha, attributes.depth, attributes.stencil,
      attributes.antialias, attributes.premultipliedAlpha, attributes.preserveDrawingBuffer);
    if (typeof id !== "number" || id < 0) {
      if (canvas !== null && typeof canvas === "object" && isNode(canvas)) {
        fireAt(canvas, new WebGLContextEvent("webglcontextcreationerror", { statusMessage: "" }));
      }
      return null;
    }
    const context = new (version === 2 ? WebGL2RenderingContext : WebGLRenderingContext)(KEY);
    const r = { id, canvas, version, attributes, objects: new $Map(), extensions: new $Map(), drawn: false,
      generation: 0, lostFired: false, restorable: false };
    WeakMapSet(webglRecords, context, r);
    webglSweepLost();
    ArrayPush(webglLive, r);
    return context;
  }

  // The extensions' constants, and the calls ANGLE_instanced_arrays adds.
  const WEBGL_EXTENSIONS = {
    ANGLE_instanced_arrays: ["VERTEX_ATTRIB_ARRAY_DIVISOR_ANGLE", 0x88FE],
    EXT_blend_minmax: ["MIN_EXT", 0x8007, "MAX_EXT", 0x8008],
    EXT_color_buffer_half_float: ["RGBA16F_EXT", 0x881A, "RGB16F_EXT", 0x881B,
      "FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE_EXT", 0x8211, "UNSIGNED_NORMALIZED_EXT", 0x8C17],
    EXT_sRGB: ["SRGB_EXT", 0x8C40, "SRGB_ALPHA_EXT", 0x8C42, "SRGB8_ALPHA8_EXT", 0x8C43,
      "FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING_EXT", 0x8210],
    EXT_texture_filter_anisotropic: ["TEXTURE_MAX_ANISOTROPY_EXT", 0x84FE, "MAX_TEXTURE_MAX_ANISOTROPY_EXT", 0x84FF],
    OES_standard_derivatives: ["FRAGMENT_SHADER_DERIVATIVE_HINT_OES", 0x8B8B],
    OES_vertex_array_object: ["VERTEX_ARRAY_BINDING_OES", 0x85B5],
    WEBGL_draw_buffers: (() => {
      const list = ["MAX_COLOR_ATTACHMENTS_WEBGL", 0x8CDF, "MAX_DRAW_BUFFERS_WEBGL", 0x8824];
      for (let i = 0; i < 16; i++) {
        ArrayPush(list, "COLOR_ATTACHMENT" + i + "_WEBGL", 0x8CE0 + i, "DRAW_BUFFER" + i + "_WEBGL", 0x8825 + i);
      }
      return list;
    })(),
    OES_texture_half_float: ["HALF_FLOAT_OES", 0x8D61],
    WEBGL_color_buffer_float: ["RGBA32F_EXT", 0x8814, "FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE_EXT", 0x8211,
      "UNSIGNED_NORMALIZED_EXT", 0x8C17],
    WEBGL_compressed_texture_s3tc: ["COMPRESSED_RGB_S3TC_DXT1_EXT", 0x83F0, "COMPRESSED_RGBA_S3TC_DXT1_EXT", 0x83F1,
      "COMPRESSED_RGBA_S3TC_DXT3_EXT", 0x83F2, "COMPRESSED_RGBA_S3TC_DXT5_EXT", 0x83F3],
    WEBGL_depth_texture: ["UNSIGNED_INT_24_8_WEBGL", 0x84FA],
  };

  // An extension's object for a context: its interface's name its
  // tag, as the reference's (no global interface object).
  function webglExtension(name, r) {
    const cls = class {
      constructor(key) {
        if (key !== KEY) throw illegalConstructor(name);
      }
    };
    const proto = cls.prototype;
    defineProperty(proto, toStringTag, { value: name, configurable: true });
    const constants = WEBGL_EXTENSIONS[name] || [];
    for (let i = 0; i < constants.length; i += 2) {
      defineProperty(proto, constants[i], { value: constants[i + 1], enumerable: true });
    }
    const method = (n, f) => defineProperty(proto, n, { value: f, writable: true, enumerable: true, configurable: true });
    if (name === "ANGLE_instanced_arrays") {
      method("drawArraysInstancedANGLE", function (mode, first, count, primcount) {
        need(arguments.length, 4, "drawArraysInstancedANGLE", name);
        native.webgl("drawArraysInstanced", r.id, glu(mode), gli(first), gli(count), gli(primcount));
        webglDrew(r);
      });
      method("drawElementsInstancedANGLE", function (mode, count, type, offset, primcount) {
        need(arguments.length, 5, "drawElementsInstancedANGLE", name);
        native.webgl("drawElementsInstanced", r.id, glu(mode), gli(count), glu(type), glll(offset), gli(primcount));
        webglDrew(r);
      });
      method("vertexAttribDivisorANGLE", function (index, divisor) {
        need(arguments.length, 2, "vertexAttribDivisorANGLE", name);
        native.webgl("vertexAttribDivisor", r.id, glu(index), glu(divisor));
      });
    }
    if (name === "OES_vertex_array_object") {
      const iface = name;
      method("createVertexArrayOES", function () {
        return webglMake(r, "O", native.webgl("createVertexArray", r.id));
      });
      for (const [n, call, gives] of [["deleteVertexArrayOES", "deleteVertexArray", "v"],
        ["isVertexArrayOES", "isVertexArray", "b"], ["bindVertexArrayOES", "bindVertexArray", "v"]]) {
        method(n, function (arrayObject) {
          need(arguments.length, n === "deleteVertexArrayOES" || n === "isVertexArrayOES" ? 0 : 1, n, iface);
          const args = webglArgs(r, "O?", [arrayObject], n, iface);
          if (args === null) return gives === "b" ? false : undefined;
          const v = native.webgl(call, r.id, args[0]);
          return gives === "b" ? v === true : undefined;
        });
      }
    }
    if (name === "WEBGL_draw_buffers") {
      method("drawBuffersWEBGL", function (buffers) {
        need(arguments.length, 1, "drawBuffersWEBGL", name);
        const list = sequence(buffers, "drawBuffersWEBGL", name, glu);
        ReflectApply(native.webgl, undefined, ["drawBuffers", r.id, ...list]);
      });
    }
    if (name === "WEBGL_lose_context") {
      method("loseContext", function () {
        if (native.webgl("isContextLost", r.id)) return;
        native.webgl("loseContext", r.id);
        webglLose(r);
      });
      method("restoreContext", function () {
        if (!native.webgl("isContextLost", r.id)) {
          native.webgl("error", r.id, 0x0502);
          return;
        }
        webglRestore(r);
      });
    }
    return new cls(KEY);
  }

  // An internal task (a timer of no delay): what WebGL says happens
  // once control returns to the event loop.
  function webglTask(work) {
    startTimer(work, 0, [], false);
  }

  // A context lost: its lost event fired in a task; when the page
  // prevents its default, the context may be restored.
  function webglLose(r) {
    if (r.lostFired) return;
    r.lostFired = true;
    webglTask(() => {
      const event = new WebGLContextEvent("webglcontextlost", { cancelable: true, statusMessage: "" });
      if (r.canvas !== null && typeof r.canvas === "object") fireAt(r.canvas, event);
      r.restorable = event.defaultPrevented;
    });
  }

  // A lost context made again: its old objects stale, its extensions
  // enabled again, the restored event fired.
  function webglRestore(r) {
    webglTask(() => {
      if (!native.webgl("restoreContext", r.id)) return;
      r.generation++;
      r.objects = new $Map();
      r.lostFired = false;
      r.restorable = false;
      $MapForEach(r.extensions, (_, name) => native.webgl("getExtension", r.id, name));
      const event = new WebGLContextEvent("webglcontextrestored", { statusMessage: "" });
      if (r.canvas !== null && typeof r.canvas === "object") fireAt(r.canvas, event);
    });
  }

  // Contexts lost to make room for a new one, their events fired.
  function webglSweepLost() {
    for (let i = 0; i < webglLive.length; i++) {
      const r = webglLive[i];
      if (!r.lostFired && native.webgl("isContextLost", r.id)) webglLose(r);
    }
  }

  // WebGL 2's methods (step 8): the calls that convert and pass on,
  // then the rest.
  const WEBGL2_SIMPLE = [
    ["bindBufferBase", "euB?", 3, "v"], ["bindBufferRange", "euB?ll", 5, "v"],
    ["copyBufferSubData", "eelll", 5, "v"],
    ["createVertexArray", "", 0, "V"], ["deleteVertexArray", "V?", 1, "v"], ["isVertexArray", "V?", 1, "b"],
    ["bindVertexArray", "V?", 1, "v"],
    ["getUniformBlockIndex", "Ps", 2, "n"], ["uniformBlockBinding", "Puu", 3, "v"],
    ["getActiveUniformBlockName", "Pu", 2, "s"],
    ["createQuery", "", 0, "Q"], ["deleteQuery", "Q?", 1, "v"], ["isQuery", "Q?", 1, "b"],
    ["beginQuery", "eQ", 2, "v"],
    ["deleteSync", "Y?", 1, "v"], ["isSync", "Y?", 1, "b"], ["waitSync", "Yul", 3, "v"],
    ["createTransformFeedback", "", 0, "X"], ["deleteTransformFeedback", "X?", 1, "v"],
    ["isTransformFeedback", "X?", 1, "b"], ["bindTransformFeedback", "eX?", 2, "v"],
    ["beginTransformFeedback", "e", 1, "v"], ["endTransformFeedback", "", 0, "v"],
    ["pauseTransformFeedback", "", 0, "v"], ["resumeTransformFeedback", "", 0, "v"],
    ["vertexAttribIPointer", "uieil", 5, "v"], ["vertexAttribDivisor", "uu", 2, "v"],
  ];
  const WEBGL2_DRAWS = [["drawArraysInstanced", "eiii", 4], ["drawElementsInstanced", "eieli", 5],
    ["drawRangeElements", "euuiel", 6]];

  function webgl2Methods(cls, iface) {
    const proto = cls.prototype;
    const method = (name, f) => defineProperty(proto, name, { value: f, writable: true, enumerable: true, configurable: true });
    webglSimple(method, WEBGL2_SIMPLE, iface);
    for (let i = 0; i < WEBGL2_DRAWS.length; i++) {
      const [name, spec, least] = WEBGL2_DRAWS[i];
      method(name, function (...args) {
        const r = webglRecord(this);
        need(args.length, least, name, iface);
        const converted = webglArgs(r, spec, args, name, iface);
        ReflectApply(native.webgl, undefined, [name, r.id, ...converted]);
        webglDrew(r);
      });
    }
    // The bindings' objects.
    method("getIndexedParameter", function (target, index) {
      const r = webglRecord(this);
      need(arguments.length, 2, "getIndexedParameter", iface);
      const t = glu(target);
      const v = native.webgl("getIndexedParameter", r.id, t, glu(index));
      if (t === 0x8A28 || t === 0x8C8F) return MapGet(r.objects, v) || null;
      return v === undefined ? null : v;
    });
    method("getBufferSubData", function (target, srcByteOffset, dstBuffer, dstOffset = 0, length = 0) {
      const r = webglRecord(this);
      need(arguments.length, 3, "getBufferSubData", iface);
      if (!ArrayBufferIsView(dstBuffer)) {
        throw new $TypeError(prefix("getBufferSubData", iface) + "parameter 3 is not of type 'ArrayBufferView'.");
      }
      const [bytes, size] = gpuViewBytes(dstBuffer);
      const first = glu(dstOffset) * size;
      const count = glu(length) === 0 ? bytes.length - first : glu(length) * size;
      if (first > bytes.length || first + count > bytes.length) {
        native.webgl("error", r.id, 0x0501);
        return;
      }
      const got = native.webgl("getBufferSubData", r.id, glu(target), glll(srcByteOffset), count);
      if (got === undefined) return;
      const from = new $Uint8Array(got);
      for (let i = 0; i < from.length; i++) bytes[first + i] = from[i];
    });
    method("getActiveUniformBlockParameter", function (program, index, pname) {
      const r = webglRecord(this);
      need(arguments.length, 3, "getActiveUniformBlockParameter", iface);
      const args = webglArgs(r, "Puu", [program, index, pname], "getActiveUniformBlockParameter", iface);
      if (args === null) return null;
      const v = ReflectApply(native.webgl, undefined, ["getActiveUniformBlockParameter", r.id, ...args]);
      if (ArrayIsArray(v)) return new Uint32Array(v);
      return v === undefined ? null : v;
    });
    method("getUniformIndices", function (program, names) {
      const r = webglRecord(this);
      need(arguments.length, 2, "getUniformIndices", iface);
      const args = webglArgs(r, "P", [program], "getUniformIndices", iface);
      const list = sequence(names, "getUniformIndices", iface, str);
      if (args === null) return null;
      const v = ReflectApply(native.webgl, undefined, ["getUniformIndices", r.id, args[0], ...list]);
      return ArrayIsArray(v) ? ArraySlice(v, 0) : null;
    });
    method("getActiveUniforms", function (program, indices, pname) {
      const r = webglRecord(this);
      need(arguments.length, 3, "getActiveUniforms", iface);
      const args = webglArgs(r, "P", [program], "getActiveUniforms", iface);
      const list = sequence(indices, "getActiveUniforms", iface, glu);
      const p = glu(pname);
      if (args === null) return null;
      const v = ReflectApply(native.webgl, undefined, ["getActiveUniforms", r.id, args[0], p, ...list]);
      if (!ArrayIsArray(v)) return null;
      if (p === 0x8A3E) {
        const out = [];
        for (let i = 0; i < v.length; i++) ArrayPush(out, v[i] !== 0);
        return out;
      }
      return ArraySlice(v, 0);
    });
    // Queries and fences: their results seen once control returned to
    // the event loop.
    const fresh = (object) => {
      const o = WeakMapGet(webglObjects, object);
      o.fresh = true;
      webglTask(() => { o.fresh = false; });
    };
    method("endQuery", function (target) {
      const r = webglRecord(this);
      need(arguments.length, 1, "endQuery", iface);
      const t = glu(target);
      const q = MapGet(r.objects, native.webgl("getQuery", r.id, t, 0x8865));
      native.webgl("endQuery", r.id, t);
      if (q !== undefined) fresh(q);
    });
    method("getQuery", function (target, pname) {
      const r = webglRecord(this);
      need(arguments.length, 2, "getQuery", iface);
      const v = native.webgl("getQuery", r.id, glu(target), glu(pname));
      return MapGet(r.objects, v) || null;
    });
    method("getQueryParameter", function (query, pname) {
      const r = webglRecord(this);
      need(arguments.length, 2, "getQueryParameter", iface);
      const args = webglArgs(r, "Qe", [query, pname], "getQueryParameter", iface);
      if (args === null) return null;
      const v = native.webgl("getQueryParameter", r.id, args[0], args[1]);
      if (v === undefined) return null;
      if (WeakMapGet(webglObjects, query).fresh) return args[1] === 0x8867 ? false : 0;
      return v;
    });
    method("fenceSync", function (condition, flags) {
      const r = webglRecord(this);
      need(arguments.length, 2, "fenceSync", iface);
      const sync = webglMake(r, "Y", native.webgl("fenceSync", r.id, glu(condition), glu(flags)));
      if (sync !== null) fresh(sync);
      return sync;
    });
    method("getSyncParameter", function (sync, pname) {
      const r = webglRecord(this);
      need(arguments.length, 2, "getSyncParameter", iface);
      const args = webglArgs(r, "Ye", [sync, pname], "getSyncParameter", iface);
      if (args === null) return null;
      const v = native.webgl("getSyncParameter", r.id, args[0], args[1]);
      if (v === undefined) return null;
      if (args[1] === 0x9114 && WeakMapGet(webglObjects, sync).fresh) return 0x9118;
      return v;
    });
    method("clientWaitSync", function (sync, flags, timeout) {
      const r = webglRecord(this);
      need(arguments.length, 3, "clientWaitSync", iface);
      const args = webglArgs(r, "Yul", [sync, flags, timeout], "clientWaitSync", iface);
      if (args === null) return 0x911D;
      const v = native.webgl("clientWaitSync", r.id, args[0], args[1], args[2]);
      if (v === 0x911A && WeakMapGet(webglObjects, sync).fresh) return 0x911B;
      return v;
    });
    method("transformFeedbackVaryings", function (program, varyings, bufferMode) {
      const r = webglRecord(this);
      need(arguments.length, 3, "transformFeedbackVaryings", iface);
      const args = webglArgs(r, "P", [program], "transformFeedbackVaryings", iface);
      const list = sequence(varyings, "transformFeedbackVaryings", iface, str);
      if (args === null) return;
      ReflectApply(native.webgl, undefined, ["transformFeedbackVaryings", r.id, args[0], glu(bufferMode), ...list]);
    });
    method("getTransformFeedbackVarying", function (program, index) {
      const r = webglRecord(this);
      need(arguments.length, 2, "getTransformFeedbackVarying", iface);
      const args = webglArgs(r, "Pu", [program, index], "getTransformFeedbackVarying", iface);
      if (args === null) return null;
      native.webgl("getTransformFeedbackVarying", r.id, args[0], args[1]);
      return null;
    });
    // The unsigned uniforms and the matrices of two sizes.
    const uniform = (name, kind, components, vector) => function (location, ...values) {
      const r = webglRecord(this);
      need(arguments.length, vector ? 2 : components + 1, name, iface);
      const args = webglArgs(r, "L?", [location], name, iface);
      if (args === null || args[0] === 0) return;
      let list = vector ? webglNumbers(values[0], name, iface) : ArraySlice(values, 0, components);
      if (vector) list = webglRange(r, list, values[1], values[2]);
      if (list === null) return;
      ReflectApply(native.webgl, undefined, ["uniform", r.id, args[0], kind, components, 0, 0, ...list]);
    };
    for (let c = 1; c <= 4; c++) {
      method("uniform" + c + "ui", uniform("uniform" + c + "ui", 2, c, false));
      method("uniform" + c + "uiv", uniform("uniform" + c + "uiv", 2, c, true));
      method("uniform" + c + "fv", uniform("uniform" + c + "fv", 0, c, true));
      method("uniform" + c + "iv", uniform("uniform" + c + "iv", 1, c, true));
    }
    const matrix = (name, cols, rows) => function (location, transpose, value, srcOffset, srcLength) {
      const r = webglRecord(this);
      need(arguments.length, 3, name, iface);
      const args = webglArgs(r, "L?", [location], name, iface);
      if (args === null || args[0] === 0) return;
      const list = webglRange(r, webglNumbers(value, name, iface), srcOffset, srcLength);
      if (list === null) return;
      ReflectApply(native.webgl, undefined, ["uniform", r.id, args[0], 0, rows, cols, !!transpose, ...list]);
    };
    for (const [c, rr] of [[2, 2], [3, 3], [4, 4], [2, 3], [3, 2], [2, 4], [4, 2], [3, 4], [4, 3]]) {
      const name = c === rr ? "uniformMatrix" + c + "fv" : "uniformMatrix" + c + "x" + rr + "fv";
      method(name, matrix(name, c, rr));
    }
    const attribI = (name, kind, vector) => function (index, ...values) {
      const r = webglRecord(this);
      need(arguments.length, vector ? 2 : 5, name, iface);
      const list = vector ? webglNumbers(values[0], name, iface) : values;
      if (vector && list.length < 4) {
        native.webgl("error", r.id, 0x0501);
        return;
      }
      const v = [0, 0, 0, 0];
      for (let i = 0; i < 4; i++) v[i] = kind === 0x1404 ? $Number(list[i]) | 0 : $Number(list[i]) >>> 0;
      native.webgl("vertexAttrib", r.id, glu(index), kind, v[0], v[1], v[2], v[3]);
    };
    method("vertexAttribI4i", attribI("vertexAttribI4i", 0x1404, false));
    method("vertexAttribI4ui", attribI("vertexAttribI4ui", 0x1405, false));
    method("vertexAttribI4iv", attribI("vertexAttribI4iv", 0x1404, true));
    method("vertexAttribI4uiv", attribI("vertexAttribI4uiv", 0x1405, true));
    // bufferData and bufferSubData of part of a view.
    const base = { bufferData: proto.bufferData, bufferSubData: proto.bufferSubData };
    method("bufferData", function (target, data, usage, srcOffset, length) {
      if (arguments.length < 4 || !ArrayBufferIsView(data)) return ReflectApply(base.bufferData, this, arguments);
      const r = webglRecord(this);
      const part = webglViewPart(r, data, srcOffset, length);
      if (part === null) return;
      native.webglData("bufferData", binaryString(part), r.id, glu(target), 0, glu(usage));
    });
    method("bufferSubData", function (target, offset, data, srcOffset, length) {
      if (arguments.length < 4 || !ArrayBufferIsView(data)) return ReflectApply(base.bufferSubData, this, arguments);
      const r = webglRecord(this);
      const part = webglViewPart(r, data, srcOffset, length);
      if (part === null) return;
      native.webglData("bufferSubData", binaryString(part), r.id, glu(target), glll(offset));
    });
  }

  // A view's elements from `srcOffset`, `length` of them (0: the rest),
  // as bytes; none (INVALID_VALUE) when past its end.
  function webglViewPart(r, view, srcOffset, length) {
    const [bytes, size] = gpuViewBytes(view);
    const first = glu(srcOffset) * size;
    const count = length === undefined || glu(length) === 0 ? bytes.length - first : glu(length) * size;
    if (first > bytes.length || first + count > bytes.length) {
      native.webgl("error", r.id, 0x0501);
      return null;
    }
    return new $Uint8Array(TypedArrayBuffer(bytes), TypedArrayOffset(bytes) + first, count);
  }

  // A uniform's values from `srcOffset`, `srcLength` of them (0: the
  // rest); none (INVALID_VALUE) when past the end.
  function webglRange(r, list, srcOffset, srcLength) {
    if (srcOffset === undefined && srcLength === undefined) return list;
    const first = glu(srcOffset === undefined ? 0 : srcOffset);
    const count = srcLength === undefined || glu(srcLength) === 0 ? list.length - first : glu(srcLength);
    if (first > list.length || first + count > list.length) {
      native.webgl("error", r.id, 0x0501);
      return null;
    }
    return ArraySlice(list, first, first + count);
  }

  // An image source's pixels for a texture: [RGBA bytes, width,
  // height], turned over and multiplied by alpha as the unpack state
  // says, cropped to `w` by `h` when given; a source not ready is an
  // empty image.
  function webglSourcePixels(r, source, method, iface, w, h) {
    const words = prefix(method, iface);
    const image = gpuImageSource(source, words);
    const flipY = native.webgl("getParameter", r.id, 0x9240) === true;
    const premultiply = native.webgl("getParameter", r.id, 0x9241) === true;
    // An ImageData's pixels as they are, its colour under alpha 0 kept
    // (a bitmap's are premultiplied, and lose it).
    const data = WeakMapGet(imageDatas, source);
    if (data !== undefined) {
      const { width, height } = data;
      const from = data.data;
      const bytes = new $Uint8Array(width * height * 4);
      for (let y = 0; y < height; y++) {
        const row = (flipY ? height - 1 - y : y) * width * 4;
        for (let i = 0; i < width * 4; i += 4) {
          const at = y * width * 4 + i;
          const a = from[row + i + 3];
          for (let k = 0; k < 3; k++) {
            const c = from[row + i + k];
            bytes[at + k] = premultiply ? MathFloor(c * a / 255 + 0.5) : c;
          }
          bytes[at + 3] = a;
        }
      }
      return [bytes, width, height];
    }
    let whole;
    try {
      whole = gpuBitmapOf(image, w > 0 && h > 0 ? [0, 0, w, h] : null, flipY);
    } catch (e) {
      if (e instanceof $DOMException && e.name === "InvalidStateError") return [new $Uint8Array(0), 0, 0];
      throw e;
    }
    try {
      if (!native.canvas("bitmapClean", 4, whole[0])) {
        throw new $DOMException(words + "The image element contains cross-origin data, and may not be loaded.",
          "SecurityError");
      }
      return [new $Uint8Array(native.canvas("bitmapPixels", 4, whole[0], false, premultiply)), whole[1], whole[2]];
    } finally {
      native.canvas("closeBitmap", 4, whole[0]);
    }
  }

  // An array's kind by the engine's code: 1 Uint8Array, 2 Uint16Array,
  // 3 Float32Array, 4 Uint32Array, 5 Int8Array, 6 Uint8ClampedArray, 7
  // Int16Array, 8 Int32Array, else 0.
  function webglViewKind(v) {
    const tag = ArrayBufferIsView(v) && !(v instanceof $DataView) ? TypedArrayTag(v) : "";
    const kinds = { Uint8Array: 1, Uint16Array: 2, Float32Array: 3, Uint32Array: 4, Int8Array: 5,
      Uint8ClampedArray: 6, Int16Array: 7, Int32Array: 8 };
    return kinds[tag] || 0;
  }

  // WebGL 2's textures, framebuffers and samplers (step 8).
  const WEBGL2_SIMPLE_B = [
    ["readBuffer", "e", 1, "v"], ["renderbufferStorageMultisample", "eieii", 5, "v"],
    ["framebufferTextureLayer", "eeT?ii", 5, "v"], ["texStorage2D", "eieii", 5, "v"],
    ["texStorage3D", "eieiii", 6, "v"], ["copyTexSubImage3D", "eiiiiiiii", 9, "v"],
    ["createSampler", "", 0, "M"], ["deleteSampler", "M?", 1, "v"], ["isSampler", "M?", 1, "b"],
    ["bindSampler", "uM?", 2, "v"], ["samplerParameteri", "Mei", 3, "v"], ["samplerParameterf", "Mef", 3, "v"],
    ["getSamplerParameter", "Me", 2, "n"],
  ];

  function webgl2Textures(cls, iface) {
    const proto = cls.prototype;
    const method = (name, f) => defineProperty(proto, name, { value: f, writable: true, enumerable: true, configurable: true });
    webglSimple(method, WEBGL2_SIMPLE_B, iface);
    const isSource = (v) => v !== null && typeof v === "object" && !ArrayBufferIsView(v);
    // An upload's last argument: a pixel unpack buffer's offset, an
    // array (from an offset of its elements), none, or an image source,
    // whole, with its size (the engine takes the rectangle of the unpack
    // state's skips).
    const pixels = (r, name, data, srcOffset, w, h, d) => {
      if (typeof data === "number" || typeof data === "bigint") return [3, 0, glll(data), null];
      if (data === null || data === undefined) return [1, 0, 0, null];
      if (isSource(data)) {
        const [bytes, sw, sh] = webglSourcePixels(r, data, name, iface, 0, 0);
        return [sw === 0 ? 1 : 2, sw, sh, bytes];
      }
      const part = srcOffset === undefined ? gpuViewBytes(data)[0] : webglViewPart(r, data, srcOffset, 0);
      if (part === null) return null;
      return [0, webglViewKind(data), 0, part];
    };
    const send = (op, r, numbers, got) => {
      const [source, view, offset, bytes] = got;
      const args = [...numbers, source, view, offset];
      if (bytes === null) {
        ReflectApply(native.webgl, undefined, [op, r.id, ...args]);
      } else {
        ReflectApply(native.webglData, undefined, [op, binaryString(bytes), r.id, ...args]);
      }
    };
    method("texImage2D", function (...a) {
      const r = webglRecord(this);
      need(a.length, 6, "texImage2D", iface);
      if (a.length < 9) {
        const [bytes, w, h] = webglSourcePixels(r, a[5], "texImage2D", iface, 0, 0);
        send("texImage2D", r, [glu(a[0]), gli(a[1]), gli(a[2]), w, h, 0, glu(a[3]), glu(a[4])],
          [w === 0 ? 1 : 2, w, h, bytes]);
        return;
      }
      const got = pixels(r, "texImage2D", a[8], a[9], gli(a[3]), gli(a[4]), 1);
      if (got === null) return;
      send("texImage2D", r, [glu(a[0]), gli(a[1]), gli(a[2]), gli(a[3]), gli(a[4]), gli(a[5]), glu(a[6]), glu(a[7])], got);
    });
    method("texSubImage2D", function (...a) {
      const r = webglRecord(this);
      need(a.length, 7, "texSubImage2D", iface);
      if (a.length < 9) {
        const [bytes, w, h] = webglSourcePixels(r, a[6], "texSubImage2D", iface, 0, 0);
        send("texSubImage2D", r, [glu(a[0]), gli(a[1]), gli(a[2]), gli(a[3]), w, h, glu(a[4]), glu(a[5])], [2, w, h, bytes]);
        return;
      }
      const got = pixels(r, "texSubImage2D", a[8], a[9], gli(a[4]), gli(a[5]), 1);
      if (got === null) return;
      send("texSubImage2D", r, [glu(a[0]), gli(a[1]), gli(a[2]), gli(a[3]), gli(a[4]), gli(a[5]), glu(a[6]), glu(a[7])], got);
    });
    method("texImage3D", function (...a) {
      const r = webglRecord(this);
      need(a.length, 10, "texImage3D", iface);
      const got = pixels(r, "texImage3D", a[9], a[10], gli(a[3]), gli(a[4]), gli(a[5]));
      if (got === null) return;
      send("texImage3D", r, [glu(a[0]), gli(a[1]), gli(a[2]), gli(a[3]), gli(a[4]), gli(a[5]), gli(a[6]), glu(a[7]), glu(a[8])], got);
    });
    method("texSubImage3D", function (...a) {
      const r = webglRecord(this);
      need(a.length, 11, "texSubImage3D", iface);
      const got = pixels(r, "texSubImage3D", a[10], a[11], gli(a[5]), gli(a[6]), gli(a[7]));
      if (got === null) return;
      send("texSubImage3D", r, [glu(a[0]), gli(a[1]), gli(a[2]), gli(a[3]), gli(a[4]), gli(a[5]), gli(a[6]), gli(a[7]), glu(a[8]), glu(a[9])], got);
    });
    const compressed = (name, count, dataAt) => method(name, function (...a) {
      const r = webglRecord(this);
      need(a.length, count, name, iface);
      const data = a[dataAt];
      if (!ArrayBufferIsView(data)) {
        throw new $TypeError(prefix(name, iface) + "parameter " + (dataAt + 1) + " is not of type 'ArrayBufferView'.");
      }
      const part = a.length > dataAt + 1 ? webglViewPart(r, data, a[dataAt + 1], a[dataAt + 2]) : gpuViewBytes(data)[0];
      if (part === null) return;
      const numbers = [];
      for (let i = 0; i < dataAt; i++) ArrayPush(numbers, i === 0 ? glu(a[i]) : gli(a[i]));
      ReflectApply(native.webglData, undefined, [name, binaryString(part), r.id, ...numbers]);
    });
    compressed("compressedTexImage2D", 7, 6);
    compressed("compressedTexSubImage2D", 8, 7);
    compressed("compressedTexImage3D", 8, 7);
    compressed("compressedTexSubImage3D", 10, 9);
    method("readPixels", function (x, y, width, height, format, type, dst, dstOffset) {
      const r = webglRecord(this);
      need(arguments.length, 7, "readPixels", iface);
      const numbers = [gli(x), gli(y), gli(width), gli(height), glu(format), glu(type)];
      if (typeof dst === "number" || typeof dst === "bigint") {
        ReflectApply(native.webgl, undefined, ["readPixels", r.id, ...numbers, 0, glll(dst)]);
        return;
      }
      if (dst === null) {
        native.webgl("error", r.id, 0x0501);
        return;
      }
      if (!ArrayBufferIsView(dst)) {
        throw new $TypeError(prefix("readPixels", iface) + "parameter 7 is not of type 'ArrayBufferView'.");
      }
      const bytes = dstOffset === undefined ? gpuViewBytes(dst)[0] : webglViewPart(r, dst, dstOffset, 0);
      if (bytes === null) return;
      const out = ReflectApply(native.webglData, undefined,
        ["readPixels", binaryString(bytes), r.id, ...numbers, webglViewKind(dst), 0]);
      if (out === undefined) return;
      const written = new $Uint8Array(out);
      for (let i = 0; i < written.length && i < bytes.length; i++) bytes[i] = written[i];
    });
    method("blitFramebuffer", function (...a) {
      const r = webglRecord(this);
      need(a.length, 10, "blitFramebuffer", iface);
      const c = webglArgs(r, "iiiiiiiiue", a, "blitFramebuffer", iface);
      ReflectApply(native.webgl, undefined, ["blitFramebuffer", r.id, ...c]);
      webglDrew(r);
    });
    method("drawBuffers", function (buffers) {
      const r = webglRecord(this);
      need(arguments.length, 1, "drawBuffers", iface);
      const list = sequence(buffers, "drawBuffers", iface, glu);
      ReflectApply(native.webgl, undefined, ["drawBuffers", r.id, ...list]);
    });
    const clearBuffer = (name, kind) => method(name, function (buffer, drawbuffer, values, srcOffset = 0) {
      const r = webglRecord(this);
      need(arguments.length, 3, name, iface);
      const list = webglNumbers(values, name, iface);
      const first = glu(srcOffset);
      if (first > list.length) {
        native.webgl("error", r.id, 0x0501);
        return;
      }
      ReflectApply(native.webgl, undefined, ["clearBuffer", r.id, glu(buffer), gli(drawbuffer), kind, ...ArraySlice(list, first)]);
      webglDrew(r);
    });
    clearBuffer("clearBufferfv", 0);
    clearBuffer("clearBufferiv", 1);
    clearBuffer("clearBufferuiv", 2);
    method("clearBufferfi", function (buffer, drawbuffer, depth, stencil) {
      const r = webglRecord(this);
      need(arguments.length, 4, "clearBufferfi", iface);
      native.webgl("clearBuffer", r.id, glu(buffer), gli(drawbuffer), 3, glf(depth), gli(stencil));
      webglDrew(r);
    });
    method("invalidateFramebuffer", function (target, attachments) {
      const r = webglRecord(this);
      need(arguments.length, 2, "invalidateFramebuffer", iface);
      const list = sequence(attachments, "invalidateFramebuffer", iface, glu);
      ReflectApply(native.webgl, undefined, ["invalidateFramebuffer", r.id, glu(target), ...list]);
    });
    method("invalidateSubFramebuffer", function (target, attachments, x, y, width, height) {
      const r = webglRecord(this);
      need(arguments.length, 6, "invalidateSubFramebuffer", iface);
      const list = sequence(attachments, "invalidateSubFramebuffer", iface, glu);
      ReflectApply(native.webgl, undefined,
        ["invalidateSubFramebuffer", r.id, glu(target), ...list, gli(x), gli(y), gli(width), gli(height)]);
    });
    method("getInternalformatParameter", function (target, internalformat, pname) {
      const r = webglRecord(this);
      need(arguments.length, 3, "getInternalformatParameter", iface);
      const v = native.webgl("getInternalformatParameter", r.id, glu(target), glu(internalformat), glu(pname));
      if (!ArrayIsArray(v)) return null;
      // The samples a renderbuffer of the format takes, most first (1, a
      // single sample, is not one of them).
      const out = [];
      for (let i = v.length - 1; i >= 0; i--) if (v[i] > 1) ArrayPush(out, v[i]);
      return new Int32Array(out);
    });
  }

  class WebGLRenderingContext {
    constructor(key) {
      if (key !== KEY) throw illegalConstructor("WebGLRenderingContext");
    }
  }

  class WebGL2RenderingContext {
    constructor(key) {
      if (key !== KEY) throw illegalConstructor("WebGL2RenderingContext");
    }
  }

  // The methods and constants both versions share.
  // The calls of `list` defined by `method`.
  function webglSimple(method, list, iface) {
    for (let i = 0; i < list.length; i++) {
      const [name, spec, least, gives] = list[i];
      method(name, function (...args) {
        const r = webglRecord(this);
        need(args.length, least, name, iface);
        const converted = webglArgs(r, spec, args, name, iface);
        if (converted === null) return gives === "b" ? false : gives === "n" ? (name === "getAttribLocation" ? -1 : null) : gives === "v" ? undefined : null;
        const answer = ReflectApply(native.webgl, undefined, [name, r.id, ...converted]);
        if (gives === "v") return undefined;
        if (gives === "n" || gives === "b") return answer === undefined ? (gives === "b" ? false : null) : answer;
        if (gives === "s") return answer === undefined ? null : answer;
        return webglMake(r, gives, answer);
      });
    }
  }

  function webglBase(cls, iface) {
    const proto = cls.prototype;
    const method = (name, f) => defineProperty(proto, name, { value: f, writable: true, enumerable: true, configurable: true });
    const getter = (name, f) => defineProperty(proto, name, { get: f, enumerable: true, configurable: true });
    webglSimple(method, WEBGL_SIMPLE, iface);
    for (let i = 0; i < WEBGL_DRAWS.length; i++) {
      const [name, spec, least] = WEBGL_DRAWS[i];
      method(name, function (...args) {
        const r = webglRecord(this);
        need(args.length, least, name, iface);
        const converted = webglArgs(r, spec, args, name, iface);
        ReflectApply(native.webgl, undefined, [name, r.id, ...converted]);
        webglDrew(r);
      });
    }
    getter("canvas", function () { return webglRecord(this).canvas; });
    getter("drawingBufferWidth", function () { return native.webgl("drawingBufferSize", webglRecord(this).id)[0]; });
    getter("drawingBufferHeight", function () { return native.webgl("drawingBufferSize", webglRecord(this).id)[1]; });
    getter("drawingBufferColorSpace", function () { webglRecord(this); return "srgb"; });
    getter("unpackColorSpace", function () { webglRecord(this); return "srgb"; });
    method("getContextAttributes", function () {
      const r = webglRecord(this);
      if (native.webgl("isContextLost", r.id)) return null;
      const a = r.attributes;
      return { alpha: a.alpha, antialias: a.antialias, depth: a.depth, desynchronized: false,
        failIfMajorPerformanceCaveat: a.failIfMajorPerformanceCaveat, powerPreference: a.powerPreference,
        premultipliedAlpha: a.premultipliedAlpha, preserveDrawingBuffer: a.preserveDrawingBuffer,
        stencil: a.stencil, xrCompatible: false };
    });
    method("isContextLost", function () { return native.webgl("isContextLost", webglRecord(this).id); });
    method("getError", function () { return native.webgl("getError", webglRecord(this).id); });
    method("getSupportedExtensions", function () {
      const r = webglRecord(this);
      if (native.webgl("isContextLost", r.id)) return null;
      return ArraySlice(native.webgl("getSupportedExtensions", r.id), 0);
    });
    method("getExtension", function (name) {
      const r = webglRecord(this);
      need(arguments.length, 1, "getExtension", iface);
      const found = native.webgl("getExtension", r.id, str(name));
      if (typeof found !== "string") return null;
      let extension = MapGet(r.extensions, found);
      if (extension === undefined) {
        extension = webglExtension(found, r);
        MapSet(r.extensions, found, extension);
      }
      return extension;
    });
    method("makeXRCompatible", function () { webglRecord(this); return resolved(undefined); });
    // The drawing buffer's tone mapping (ColorWeb-CG's): its mode kept;
    // an 8-bit drawing buffer shows "extended" as "standard".
    method("drawingBufferToneMapping", function (toneMapping) {
      const r = webglRecord(this);
      if (toneMapping !== undefined) {
        let mode = "standard";
        if (toneMapping !== null && (typeof toneMapping === "object" || typeof toneMapping === "function")) {
          const given = toneMapping.mode;
          if (given !== undefined) {
            mode = str(given);
            if (mode !== "standard" && mode !== "extended") {
              throw new $TypeError(prefix("drawingBufferToneMapping", iface) + "Failed to read the 'mode' property from 'GPUCanvasToneMapping': The provided value '" + mode + "' is not a valid enum value of type GPUCanvasToneMappingMode.");
            }
          }
        } else if (toneMapping !== null) {
          throw new $TypeError(prefix("drawingBufferToneMapping", iface) + "The provided value is not of type 'GPUCanvasToneMapping'.");
        }
        r.toneMapping = mode;
      }
      return { mode: r.toneMapping || "standard" };
    });
    method("getParameter", function (pname) {
      const r = webglRecord(this);
      need(arguments.length, 1, "getParameter", iface);
      const p = glu(pname);
      const answer = native.webgl("getParameter", r.id, p);
      // The bindings: their objects.
      if (p === 0x8894 || p === 0x8895) return MapGet(r.objects, answer) || null;
      if (p === 0x8B8D) return MapGet(r.objects, answer) || null;
      if (p === 0x8CA6 || p === 0x8CA7 || p === 0x8069 || p === 0x8514 || p === 0x85B5 || p === 0x8F36 ||
        p === 0x8F37 || p === 0x88ED || p === 0x88EF || p === 0x8A28 || p === 0x8C8F || p === 0x8E25 ||
        p === 0x806A || p === 0x8C1D || p === 0x8919 || p === 0x8CAA) {
        return MapGet(r.objects, answer) || null;
      }
      if (ArrayIsArray(answer)) {
        const kind = answer[0];
        const values = ArraySlice(answer, 1);
        if (kind === 1) return new Int32Array(values);
        if (kind === 2) return new Float32Array(values);
        const bools = [];
        for (let i = 0; i < values.length; i++) ArrayPush(bools, values[i] !== 0);
        return bools;
      }
      return answer === undefined ? null : answer;
    });
    method("getShaderParameter", function (shader, pname) {
      const r = webglRecord(this);
      need(arguments.length, 2, "getShaderParameter", iface);
      const args = webglArgs(r, "Se", [shader, pname], "getShaderParameter", iface);
      if (args === null) return null;
      const answer = native.webgl("getShaderParameter", r.id, args[0], args[1]);
      return answer === undefined ? null : answer;
    });
    method("getProgramParameter", function (program, pname) {
      const r = webglRecord(this);
      need(arguments.length, 2, "getProgramParameter", iface);
      const args = webglArgs(r, "Pe", [program, pname], "getProgramParameter", iface);
      if (args === null) return null;
      const answer = native.webgl("getProgramParameter", r.id, args[0], args[1]);
      return answer === undefined ? null : answer;
    });
    method("getAttachedShaders", function (program) {
      const r = webglRecord(this);
      need(arguments.length, 1, "getAttachedShaders", iface);
      const args = webglArgs(r, "P", [program], "getAttachedShaders", iface);
      if (args === null) return null;
      const ids = native.webgl("getAttachedShaders", r.id, args[0]);
      if (ids === undefined) return null;
      const out = [];
      for (let i = 0; i < ids.length; i++) ArrayPush(out, MapGet(r.objects, ids[i]));
      return out;
    });
    const activeInfo = (name) => function (program, index) {
      const r = webglRecord(this);
      need(arguments.length, 2, name, iface);
      const args = webglArgs(r, "Pu", [program, index], name, iface);
      if (args === null) return null;
      const pair = native.webgl(name, r.id, args[0], args[1]);
      if (pair === undefined) return null;
      const info = JSONParse(pair[0]);
      const bytes = new $Uint8Array(pair[1]);
      let text = "";
      for (let i = 0; i < bytes.length; i++) text += FromCharCode(bytes[i]);
      return new WebGLActiveInfo(KEY, text, info[0], info[1]);
    };
    method("getActiveAttrib", activeInfo("getActiveAttrib"));
    method("getActiveUniform", activeInfo("getActiveUniform"));
    method("getUniform", function (program, location) {
      const r = webglRecord(this);
      need(arguments.length, 2, "getUniform", iface);
      const args = webglArgs(r, "PL", [program, location], "getUniform", iface);
      if (args === null) return null;
      const pair = native.webgl("getUniform", r.id, args[0], args[1]);
      if (pair === undefined) return null;
      const told = JSONParse(pair[0]);
      const kind = told[0];
      const values = ArraySlice(told, 2);
      if (told[1] === 1) return kind === 3 ? values[0] !== 0 : values[0];
      if (kind === 0) return new Float32Array(values);
      if (kind === 1) return new Int32Array(values);
      if (kind === 2) return new Uint32Array(values);
      const bools = [];
      for (let i = 0; i < values.length; i++) ArrayPush(bools, values[i] !== 0);
      return bools;
    });
    method("getShaderPrecisionFormat", function (shadertype, precisiontype) {
      const r = webglRecord(this);
      need(arguments.length, 2, "getShaderPrecisionFormat", iface);
      const v = native.webgl("getShaderPrecisionFormat", r.id, glu(shadertype), glu(precisiontype));
      return ArrayIsArray(v) ? new WebGLShaderPrecisionFormat(KEY, v[0], v[1], v[2]) : null;
    });
    method("getVertexAttrib", function (index, pname) {
      const r = webglRecord(this);
      need(arguments.length, 2, "getVertexAttrib", iface);
      const p = glu(pname);
      const v = native.webgl("getVertexAttrib", r.id, glu(index), p);
      if (p === 0x889F) return MapGet(r.objects, v) || null;
      if (p === 0x8626 && ArrayIsArray(v)) return new Float32Array(v);
      return v === undefined ? null : v;
    });
    method("bufferData", function (target, data, usage) {
      const r = webglRecord(this);
      need(arguments.length, 3, "bufferData", iface);
      const t = glu(target);
      if (data === null) {
        native.webgl("error", r.id, 0x0501);
        return;
      }
      if (typeof data === "number" || typeof data === "bigint" ||
        !(data instanceof $ArrayBuffer || ArrayBufferIsView(data))) {
        native.webgl("bufferData", r.id, t, glll(data), glu(usage));
        return;
      }
      native.webglData("bufferData", binaryString(webglBytes(data, "bufferData", iface, 2)), r.id, t, 0, glu(usage));
    });
    method("bufferSubData", function (target, offset, data) {
      const r = webglRecord(this);
      need(arguments.length, 3, "bufferSubData", iface);
      if (data === null) return;
      const bytes = webglBytes(data, "bufferSubData", iface, 3);
      native.webglData("bufferSubData", binaryString(bytes), r.id, glu(target), glll(offset));
    });
    // The uniforms: [kind, components, columns, transpose, values...].
    const uniform = (name, kind, components, vector) => function (location, ...values) {
      const r = webglRecord(this);
      need(arguments.length, vector ? 2 : components + 1, name, iface);
      const args = webglArgs(r, "L?", [location], name, iface);
      if (args === null || args[0] === 0) return;
      const list = vector ? webglNumbers(values[0], name, iface) : ArraySlice(values, 0, components);
      ReflectApply(native.webgl, undefined, ["uniform", r.id, args[0], kind, components, 0, 0, ...list]);
    };
    for (let c = 1; c <= 4; c++) {
      method("uniform" + c + "f", uniform("uniform" + c + "f", 0, c, false));
      method("uniform" + c + "i", uniform("uniform" + c + "i", 1, c, false));
      method("uniform" + c + "fv", uniform("uniform" + c + "fv", 0, c, true));
      method("uniform" + c + "iv", uniform("uniform" + c + "iv", 1, c, true));
    }
    const matrix = (name, cols, rows) => function (location, transpose, value) {
      const r = webglRecord(this);
      need(arguments.length, 3, name, iface);
      const args = webglArgs(r, "L?", [location], name, iface);
      if (args === null || args[0] === 0) return;
      const list = webglNumbers(value, name, iface);
      ReflectApply(native.webgl, undefined, ["uniform", r.id, args[0], 0, rows, cols, !!transpose, ...list]);
    };
    for (let c = 2; c <= 4; c++) {
      method("uniformMatrix" + c + "fv", matrix("uniformMatrix" + c + "fv", c, c));
    }
    const attrib = (name, count, vector) => function (index, ...values) {
      const r = webglRecord(this);
      need(arguments.length, vector ? 2 : count + 1, name, iface);
      const list = vector ? webglNumbers(values[0], name, iface) : values;
      if (vector && list.length < count) {
        native.webgl("error", r.id, 0x0501);
        return;
      }
      const v = [0, 0, 0, 1];
      for (let i = 0; i < count; i++) v[i] = $Number(list[i]);
      native.webgl("vertexAttrib", r.id, glu(index), 0x1406, v[0], v[1], v[2], v[3]);
    };
    for (let c = 1; c <= 4; c++) {
      method("vertexAttrib" + c + "f", attrib("vertexAttrib" + c + "f", c, false));
      method("vertexAttrib" + c + "fv", attrib("vertexAttrib" + c + "fv", c, true));
    }
    // Textures, renderbuffers and framebuffers (step 7).
    const sourcePixels = (r, source, method) => webglSourcePixels(r, source, method, iface, 0, 0);
    const viewKind = webglViewKind;
    const viewOf = (v, method, index) => {
      if (!ArrayBufferIsView(v)) {
        throw new $TypeError(prefix(method, iface) + "parameter " + index + " is not of type 'ArrayBufferView'.");
      }
      return gpuViewBytes(v)[0];
    };
    method("texImage2D", function (...args) {
      const r = webglRecord(this);
      need(args.length, 6, "texImage2D", iface);
      const [target, level, internal] = [glu(args[0]), gli(args[1]), gli(args[2])];
      if (args.length >= 9) {
        const [w, h, border, format, type] = [gli(args[3]), gli(args[4]), gli(args[5]), glu(args[6]), glu(args[7])];
        const pixels = args[8];
        if (pixels === null || pixels === undefined) {
          native.webgl("texImage2D", r.id, target, level, internal, w, h, border, format, type, 1, 0);
          return;
        }
        const bytes = viewOf(pixels, "texImage2D", 9);
        native.webglData("texImage2D", binaryString(bytes), r.id, target, level, internal, w, h, border, format,
          type, 0, viewKind(pixels));
        return;
      }
      const [format, type] = [glu(args[3]), glu(args[4])];
      const [bytes, w, h] = sourcePixels(r, args[5], "texImage2D");
      native.webglData("texImage2D", binaryString(bytes), r.id, target, level, internal, w, h, 0, format, type,
        w === 0 ? 1 : 2, 0);
    });
    method("texSubImage2D", function (...args) {
      const r = webglRecord(this);
      need(args.length, 7, "texSubImage2D", iface);
      const [target, level, x, y] = [glu(args[0]), gli(args[1]), gli(args[2]), gli(args[3])];
      if (args.length >= 9) {
        const [w, h, format, type] = [gli(args[4]), gli(args[5]), glu(args[6]), glu(args[7])];
        const pixels = args[8];
        if (pixels === null || pixels === undefined) {
          native.webgl("texSubImage2D", r.id, target, level, x, y, w, h, format, type, 1, 0);
          return;
        }
        const bytes = viewOf(pixels, "texSubImage2D", 9);
        native.webglData("texSubImage2D", binaryString(bytes), r.id, target, level, x, y, w, h, format, type, 0,
          viewKind(pixels));
        return;
      }
      const [format, type] = [glu(args[4]), glu(args[5])];
      const [bytes, w, h] = sourcePixels(r, args[6], "texSubImage2D");
      native.webglData("texSubImage2D", binaryString(bytes), r.id, target, level, x, y, w, h, format, type, 2, 0);
    });
    method("compressedTexImage2D", function (target, level, internal, width, height, border, data) {
      const r = webglRecord(this);
      need(arguments.length, 7, "compressedTexImage2D", iface);
      const bytes = viewOf(data, "compressedTexImage2D", 7);
      native.webglData("compressedTexImage2D", binaryString(bytes), r.id, glu(target), gli(level), glu(internal),
        gli(width), gli(height), gli(border));
    });
    method("compressedTexSubImage2D", function (target, level, x, y, width, height, format, data) {
      const r = webglRecord(this);
      need(arguments.length, 8, "compressedTexSubImage2D", iface);
      const bytes = viewOf(data, "compressedTexSubImage2D", 8);
      native.webglData("compressedTexSubImage2D", binaryString(bytes), r.id, glu(target), gli(level), gli(x),
        gli(y), gli(width), gli(height), glu(format));
    });
    method("readPixels", function (x, y, width, height, format, type, pixels) {
      const r = webglRecord(this);
      need(arguments.length, 7, "readPixels", iface);
      const numbers = [gli(x), gli(y), gli(width), gli(height), glu(format), glu(type)];
      if (pixels === null) {
        native.webgl("error", r.id, 0x0501);
        return;
      }
      const bytes = viewOf(pixels, "readPixels", 7);
      const out = ReflectApply(native.webglData, undefined,
        ["readPixels", binaryString(bytes), r.id, ...numbers, viewKind(pixels)]);
      if (out === undefined) return;
      const written = new $Uint8Array(out);
      for (let i = 0; i < written.length && i < bytes.length; i++) bytes[i] = written[i];
    });
    method("getFramebufferAttachmentParameter", function (target, attachment, pname) {
      const r = webglRecord(this);
      need(arguments.length, 3, "getFramebufferAttachmentParameter", iface);
      const p = glu(pname);
      const v = native.webgl("getFramebufferAttachmentParameter", r.id, glu(target), glu(attachment), p);
      if (p === 0x8CD1) return MapGet(r.objects, v) || null;
      return v === undefined ? null : v;
    });
    expose(cls, iface);
  }

  // The constants, on the interface and its prototype alike.
  function webglConstants(cls, list) {
    for (let i = 0; i < list.length; i += 2) {
      const desc = { value: list[i + 1], enumerable: true };
      defineProperty(cls, list[i], desc);
      defineProperty(cls.prototype, list[i], desc);
    }
  }

  if (!WORKER) {
    webglBase(WebGLRenderingContext, "WebGLRenderingContext");
    webglConstants(WebGLRenderingContext, WEBGL1_CONSTANTS);
    webglBase(WebGL2RenderingContext, "WebGL2RenderingContext");
    webgl2Methods(WebGL2RenderingContext, "WebGL2RenderingContext");
    webgl2Textures(WebGL2RenderingContext, "WebGL2RenderingContext");
    webglConstants(WebGL2RenderingContext, WEBGL1_CONSTANTS);
    webglConstants(WebGL2RenderingContext, WEBGL2_CONSTANTS);
  }

  // What the GPU process told: a promise's answer, or a device's news.
  function* gpuTask(text, bytes) {
    const told = JSONParse(text);
    if (told.promise !== undefined) {
      const settle = MapGet(gpuPromises, told.promise);
      if (settle === undefined) return false;
      MapDelete(gpuPromises, told.promise);
      if (told.error !== undefined) {
        settle.reject(told.error.name === "TypeError" ? new $TypeError(told.error.message)
          : told.error.name === "GPUPipelineError" ? new GPUPipelineError(told.error.message, { reason: "validation" })
          : new $DOMException(told.error.message, told.error.name));
      } else if (told.adapter !== undefined) {
        settle.resolve(told.adapter === null ? null : new GPUAdapter(KEY, told.adapter));
      } else if (told.device !== undefined) {
        settle.resolve(told.device);
      } else if (told.scope !== undefined) {
        settle.resolve(told.scope);
      } else if (told.mapped !== undefined) {
        settle.resolve(bytes);
      } else if (told.compilation !== undefined) {
        settle.resolve(told.compilation);
      } else if (told.pipeline !== undefined) {
        settle.resolve(undefined);
      } else {
        settle.resolve(undefined);
      }
      return false;
    }
    const device = MapGet(gpuDevices, told.device);
    if (device === undefined) return false;
    if (told.uncaptured !== undefined) {
      const event = new GPUUncapturedErrorEvent("uncapturederror", {
        error: gpuError(told.uncaptured),
        cancelable: true,
      });
      eventState(event).trusted = true;
      yield* dispatching(device, event);
    } else if (told.lost !== undefined) {
      gpuDeviceLost(device, told.lost.reason, told.lost.message);
    }
    return false;
  }

  return { GPUCanvasContext, gpuCanvasContext, webglVersionOf, webglContext, gpuPresent, webglPresent, gpuTask };
})
