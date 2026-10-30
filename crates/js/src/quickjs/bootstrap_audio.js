// Web Audio (M40, Web Audio API 1.1): a binding chunk (D64), compiled
// apart from the bootstrap and read into a realm only when a page first
// uses it. A function of what of the bootstrap's it uses, as the
// bootstrap names it, giving what the bootstrap calls of it. The
// bootstrap's rules hold here: what is kept of the built-ins was kept
// before any page ran, nothing looked up on a built-in's prototype when a
// page calls in, no iterator.
(function (env) {
  "use strict";
  const {
    $ArrayIsArray, $DOMException, $Float32Array, $Map, $NumberIsFinite, $Promise, $RangeError,
    $TypeError, $Uint8Array, ArrayPush, Event, EventTarget, HTMLMediaElement, JSONParse,
    JSONStringify, KEY, MapDelete, MapGet, MapSet, MathAbs, MathFloor, MathFround, MathMax,
    MathMin, MathRandom, StringSlice, TypedArraySet,
    binaryString, bufferLength, defineHandlers, defineProperty, dispatching, domError, eventState,
    expose, failed, getOwnPropertyNames, illegalConstructor, include, listed, native, need,
    needToConstruct, performance, prefix, rejected, resolved, self, str, unsigned, viewBuffer,
    viewLength, viewOffset,
  } = env;

  // ---- Web Audio (M40, Web Audio API 1.1) ----------------------------------
  // The contexts and their nodes are the engine's (`native.audio`: an
  // operation as JSON, with any samples as bytes after it), named here
  // by numbers. These objects keep the numbers, check what a page passes
  // in the reference browser's words, and keep what the standard keeps on
  // the control thread. A window's alone.

  const audioContexts = new $Map();
  const audioPromises = new $Map();
  let audioPromiseNext = 0;
  const audioSources = new $Map();

  // Asks the engine `op`, with `bytes` (a Uint8Array) or null: its result
  // and the bytes it gave (an ArrayBuffer). Throws its error, after
  // "Failed to execute 'method' on 'iface': " or, for a null `iface`,
  // after `method` itself.
  function audioCall(op, bytes, method, iface) {
    let pair;
    try {
      pair = native.audio(JSONStringify(op), bytes === null ? null : binaryString(bytes));
    } catch (e) {
      throw iface === null ? e : failed(e, method, iface);
    }
    const answer = JSONParse(pair[0]);
    if (answer.error !== undefined) {
      const e = answer.error;
      const words = (iface === null ? method : prefix(method, iface)) + e.message;
      if (e.name === "TypeError") throw new $TypeError(words);
      if (e.name === "RangeError") throw new $RangeError(words);
      throw new $DOMException(words, e.name);
    }
    return { result: answer.result, bytes: pair[1] };
  }

  function setWords(property, iface) {
    return "Failed to set the '" + property + "' property on '" + iface + "': ";
  }

  // `extra`'s own members, put on `op`.
  function audioMerge(op, extra) {
    if (extra === undefined) return op;
    const keys = getOwnPropertyNames(extra);
    for (let i = 0; i < keys.length; i++) op[keys[i]] = extra[keys[i]];
    return op;
  }

  // Float32Arrays as one run of little-endian bytes.
  function floatBytes(arrays) {
    let total = 0;
    for (let i = 0; i < arrays.length; i++) total += viewLength(arrays[i]);
    const all = new $Uint8Array(total);
    let at = 0;
    for (let i = 0; i < arrays.length; i++) {
      const a = arrays[i];
      TypedArraySet(all, new $Uint8Array(viewBuffer(a), viewOffset(a), viewLength(a)), at);
      at += viewLength(a);
    }
    return all;
  }

  // Web IDL's float and double: finite, or a TypeError.
  // A float past its range is infinite once rounded, and throws too.
  function audioFloat(value, words, double) {
    const v = double ? +value : MathFround(+value);
    if (!$NumberIsFinite(v)) {
      throw new $TypeError(words + "The provided " + (double ? "double" : "float") + " value is non-finite.");
    }
    return v;
  }

  // Floats all converted before any is used: one non-finite throws
  // with none set (WPT's pannernode-setposition-throws.html).
  function audioFloats(values, words) {
    const out = [];
    for (let i = 0; i < values.length; i++) ArrayPush(out, audioFloat(values[i], words, false));
    return out;
  }

  function audioTime(value, what, words) {
    const t = audioFloat(value, words, true);
    if (t < 0) {
      throw new $RangeError(words + "The " + what + " provided (" + t + ") is less than the minimum bound (0).");
    }
    return t;
  }

  // A buffer's or offline context's options must have its length and
  // rate.
  function audioRequired(options, dictionary, words) {
    if (options.length === undefined || options.sampleRate === undefined) {
      const name = options.length === undefined ? "length" : "sampleRate";
      throw new $TypeError(words + "Failed to read the '" + name + "' property from '" + dictionary + "': Required member is undefined.");
    }
  }

  // ---- AudioBuffer ----

  let audioBufferPayload;
  let audioBufferFromEngine;
  let audioBufferFill;

  function audioBufferCheck(channels, length, rate, words) {
    if (channels < 1 || channels > 32) {
      throw new $DOMException(words + "The number of channels provided (" + channels + ") is outside the range [1, 32].", "NotSupportedError");
    }
    if (length < 1) {
      throw new $DOMException(words + "The number of frames provided (" + length + ") is less than or equal to the minimum bound (0).", "NotSupportedError");
    }
    if (!(rate >= 3000 && rate <= 768000)) {
      throw new $DOMException(words + "The sample rate provided (" + rate + ") is outside the range [3000, 768000].", "NotSupportedError");
    }
  }

  class AudioBuffer {
    #rate; #length; #data; #id = 0; #fresh = false;

    constructor(options) {
      needToConstruct(arguments.length, 1, "AudioBuffer");
      const given = options === null || options === undefined ? {} : options;
      const words = "Failed to construct 'AudioBuffer': ";
      audioRequired(given, "AudioBufferOptions", words);
      const channels = given.numberOfChannels === undefined ? 1 : unsigned(given.numberOfChannels);
      const length = unsigned(given.length);
      const rate = audioFloat(given.sampleRate, words, false);
      audioBufferCheck(channels, length, rate, words);
      this.#rate = rate;
      this.#length = length;
      this.#data = [];
      for (let i = 0; i < channels; i++) ArrayPush(this.#data, new $Float32Array(length));
    }

    static {
      // What the engine is given for `buffer`: its id while the engine
      // has its samples as they are, else the samples.
      audioBufferPayload = (buffer, samples) => {
        if (!samples && buffer.#id !== 0 && buffer.#fresh) return { op: { id: buffer.#id }, bytes: null };
        return {
          op: { channels: buffer.#data.length, length: buffer.#length, rate: buffer.#rate },
          bytes: floatBytes(buffer.#data),
        };
      };
      // A buffer the engine decoded or rendered, as id `id`.
      audioBufferFromEngine = (id, rate, channels, length, bytes) => {
        const buffer = new AudioBuffer({ numberOfChannels: 1, length: 1, sampleRate: rate });
        buffer.#length = length;
        buffer.#data = [];
        for (let i = 0; i < channels; i++) {
          ArrayPush(buffer.#data, bytes === null ? new $Float32Array(length) : new $Float32Array(bytes, i * length * 4, length));
        }
        buffer.#id = id;
        buffer.#fresh = true;
        return buffer;
      };
      // Channel `c` of `buffer` set to `floats`.
      audioBufferFill = (buffer, c, floats) => {
        TypedArraySet(buffer.#data[c], floats);
        buffer.#fresh = false;
      };
    }

    get sampleRate() { return this.#rate; }
    get length() { return this.#length; }
    get duration() { return this.#length / this.#rate; }
    get numberOfChannels() { return this.#data.length; }

    getChannelData(channel) {
      need(arguments.length, 1, "getChannelData", "AudioBuffer");
      const c = unsigned(channel);
      if (c >= this.#data.length) {
        throw domError("IndexSizeError", "channel index (" + c + ") exceeds number of channels (" + this.#data.length + ")", "getChannelData", "AudioBuffer");
      }
      // The page may change the samples now: the engine's copy is old.
      this.#fresh = false;
      return this.#data[c];
    }

    copyFromChannel(destination, channelNumber, bufferOffset) {
      need(arguments.length, 2, "copyFromChannel", "AudioBuffer");
      if (!(destination instanceof $Float32Array)) {
        throw new $TypeError(prefix("copyFromChannel", "AudioBuffer") + "parameter 1 is not of type 'Float32Array'.");
      }
      const c = unsigned(channelNumber);
      if (c >= this.#data.length) {
        throw domError("IndexSizeError", "The channelNumber provided (" + c + ") is outside the range [0, " + this.#data.length + ").", "copyFromChannel", "AudioBuffer");
      }
      const offset = bufferOffset === undefined ? 0 : unsigned(bufferOffset);
      const source = this.#data[c];
      const n = MathMax(0, MathMin(destination.length, this.#length - offset));
      for (let i = 0; i < n; i++) destination[i] = source[offset + i];
    }

    copyToChannel(source, channelNumber, bufferOffset) {
      need(arguments.length, 2, "copyToChannel", "AudioBuffer");
      if (!(source instanceof $Float32Array)) {
        throw new $TypeError(prefix("copyToChannel", "AudioBuffer") + "parameter 1 is not of type 'Float32Array'.");
      }
      const c = unsigned(channelNumber);
      if (c >= this.#data.length) {
        throw domError("IndexSizeError", "The channelNumber provided (" + c + ") is outside the range [0, " + this.#data.length + ").", "copyToChannel", "AudioBuffer");
      }
      const offset = bufferOffset === undefined ? 0 : unsigned(bufferOffset);
      const target = this.#data[c];
      const n = MathMax(0, MathMin(source.length, this.#length - offset));
      for (let i = 0; i < n; i++) target[offset + i] = source[i];
      this.#fresh = false;
    }
  }
  expose(AudioBuffer, "AudioBuffer");

  // ---- AudioParam ----

  let makeAudioParam;
  let audioParamTarget;

  class AudioParam {
    #ctx; #node; #index; #default; #min; #max; #rate; #fixed; #listener;

    constructor(key) {
      if (key !== KEY) throw illegalConstructor("AudioParam");
    }

    static {
      makeAudioParam = (ctx, node, index, info, fixed, listener) => {
        const p = new AudioParam(KEY);
        p.#ctx = ctx; p.#node = node; p.#index = index;
        p.#default = info.default; p.#min = info.min; p.#max = info.max; p.#rate = info.rate;
        p.#fixed = !!fixed; p.#listener = !!listener;
        return p;
      };
      audioParamTarget = (p) => ({ ctx: p.#ctx, node: p.#node, index: p.#index, listener: p.#listener });
    }

    #call(what, extra, bytes, method, iface) {
      const op = { op: "param", ctx: audioContextId(this.#ctx), node: this.#node, param: this.#index, listener: this.#listener, what };
      return audioCall(audioMerge(op, extra), bytes, method, iface === undefined ? "AudioParam" : iface).result;
    }

    get value() { return this.#call("get", undefined, null, "value"); }
    set value(v) {
      const words = setWords("value", "AudioParam");
      this.#call("value", { value: audioFloat(v, words, false) }, null, words, null);
    }
    get automationRate() { return this.#rate; }
    set automationRate(v) {
      const rate = str(v);
      if (rate !== "a-rate" && rate !== "k-rate") return;
      if (this.#fixed && rate !== this.#rate) {
        throw new $DOMException(setWords("automationRate", "AudioParam") + "automationRate cannot be changed", "InvalidStateError");
      }
      this.#rate = rate;
      this.#call("rate", { rate }, null, setWords("automationRate", "AudioParam"), null);
    }
    get defaultValue() { return this.#default; }
    get minValue() { return this.#min; }
    get maxValue() { return this.#max; }

    setValueAtTime(value, startTime) {
      need(arguments.length, 2, "setValueAtTime", "AudioParam");
      const words = prefix("setValueAtTime", "AudioParam");
      const v = audioFloat(value, words, false);
      const t = audioTime(startTime, "start time", words);
      this.#call("setValueAtTime", { value: v, time: t }, null, "setValueAtTime");
      return this;
    }

    linearRampToValueAtTime(value, endTime) {
      need(arguments.length, 2, "linearRampToValueAtTime", "AudioParam");
      const words = prefix("linearRampToValueAtTime", "AudioParam");
      const v = audioFloat(value, words, false);
      const t = audioTime(endTime, "end time", words);
      this.#call("linearRampToValueAtTime", { value: v, time: t }, null, "linearRampToValueAtTime");
      return this;
    }

    exponentialRampToValueAtTime(value, endTime) {
      need(arguments.length, 2, "exponentialRampToValueAtTime", "AudioParam");
      const words = prefix("exponentialRampToValueAtTime", "AudioParam");
      const v = audioFloat(value, words, false);
      if (MathAbs(v) < 1.401298464324817e-45) {
        throw new $RangeError(words + "The float target value provided (" + v + ") should not be in the range (-1.40130e-45, 1.40130e-45).");
      }
      const t = audioTime(endTime, "end time", words);
      this.#call("exponentialRampToValueAtTime", { value: v, time: t }, null, "exponentialRampToValueAtTime");
      return this;
    }

    setTargetAtTime(target, startTime, timeConstant) {
      need(arguments.length, 3, "setTargetAtTime", "AudioParam");
      const words = prefix("setTargetAtTime", "AudioParam");
      const v = audioFloat(target, words, false);
      const t = audioTime(startTime, "start time", words);
      const c = audioTime(timeConstant, "time constant", words);
      this.#call("setTargetAtTime", { value: v, time: t, constant: c }, null, "setTargetAtTime");
      return this;
    }

    setValueCurveAtTime(values, startTime, duration) {
      need(arguments.length, 3, "setValueCurveAtTime", "AudioParam");
      const words = prefix("setValueCurveAtTime", "AudioParam");
      const n = values.length >>> 0;
      const curve = new $Float32Array(n);
      for (let i = 0; i < n; i++) curve[i] = audioFloat(values[i], words, false);
      const t = audioTime(startTime, "start time", words);
      const d = audioFloat(duration, words, true);
      if (!(d > 0)) {
        throw new $RangeError(words + "The duration provided (" + d + ") is less than or equal to the minimum bound (0).");
      }
      if (n < 2) {
        throw new $DOMException(words + "The curve length provided (" + n + ") is less than the minimum bound (2).", "InvalidStateError");
      }
      this.#call("setValueCurveAtTime", { time: t, duration: d }, floatBytes([curve]), "setValueCurveAtTime");
      return this;
    }

    cancelScheduledValues(cancelTime) {
      need(arguments.length, 1, "cancelScheduledValues", "AudioParam");
      const t = audioTime(cancelTime, "cancel time", prefix("cancelScheduledValues", "AudioParam"));
      this.#call("cancelScheduledValues", { time: t }, null, "cancelScheduledValues");
      return this;
    }

    cancelAndHoldAtTime(cancelTime) {
      need(arguments.length, 1, "cancelAndHoldAtTime", "AudioParam");
      const t = audioTime(cancelTime, "cancel time", prefix("cancelAndHoldAtTime", "AudioParam"));
      this.#call("cancelAndHoldAtTime", { time: t }, null, "cancelAndHoldAtTime");
      return this;
    }
  }
  expose(AudioParam, "AudioParam");

  // ---- AudioNode ----

  let audioNodeId;
  let audioNodeContext;

  // The node kinds whose channel count is 1 or 2 and whose mode is never
  // "max".
  const AUDIO_TWO = { __proto__: null, stereoPanner: true, panner: true, convolver: true, compressor: true };

  class AudioNode extends EventTarget {
    #ctx; #node; #inputs; #outputs; #count; #mode; #interp; #kind;

    constructor(key, ctx, info, kind) {
      if (key !== KEY) throw illegalConstructor("AudioNode");
      super();
      this.#ctx = ctx; this.#node = info.node; this.#inputs = info.inputs; this.#outputs = info.outputs;
      this.#count = info.count; this.#mode = info.mode; this.#interp = info.interpretation; this.#kind = kind;
    }

    static {
      audioNodeId = (node) => node.#node;
      audioNodeContext = (node) => node.#ctx;
    }

    #channels(what, value, property) {
      const op = { op: "channels", ctx: audioContextId(this.#ctx), node: this.#node };
      op[what] = value;
      audioCall(op, null, setWords(property, "AudioNode"), null);
    }

    get context() { return this.#ctx; }
    get numberOfInputs() { return this.#inputs; }
    get numberOfOutputs() { return this.#outputs; }

    get channelCount() { return this.#count; }
    set channelCount(v) {
      const n = unsigned(v);
      const words = setWords("channelCount", "AudioNode");
      const kind = this.#kind;
      if (kind === "splitter" || kind === "merger") {
        if (n !== this.#count) {
          throw new $DOMException(words + "Cannot change channel count of " + (kind === "splitter" ? "ChannelSplitterNode" : "ChannelMergerNode") + ".", "InvalidStateError");
        }
        return;
      }
      if (n < 1 || n > 32) {
        throw new $DOMException(words + "The channel count provided (" + n + ") is outside the range [1, 32].", "NotSupportedError");
      }
      if (kind === "script" && n !== this.#count) {
        throw new $DOMException(words + "channelCount cannot be changed from " + this.#count + " to " + n, "NotSupportedError");
      }
      if (kind === "destination" && n > audioMaxChannels(this.#ctx)) {
        throw new $DOMException(words + "The channel count provided (" + n + ") is outside the range [1, " + audioMaxChannels(this.#ctx) + "].", "IndexSizeError");
      }
      if (AUDIO_TWO[kind] && n > 2) {
        throw new $DOMException(words + "The channel count provided (" + n + ") is outside the range [1, 2].", "NotSupportedError");
      }
      this.#count = n;
      this.#channels("count", n, "channelCount");
    }

    get channelCountMode() { return this.#mode; }
    set channelCountMode(v) {
      const mode = str(v);
      if (mode !== "max" && mode !== "clamped-max" && mode !== "explicit") return;
      const kind = this.#kind;
      const words = setWords("channelCountMode", "AudioNode");
      if ((kind === "splitter" || kind === "merger") && mode !== this.#mode) {
        throw new $DOMException(words + "Cannot change channel count mode.", "InvalidStateError");
      }
      if (kind === "script" && mode !== "explicit") {
        throw new $DOMException(words + "channelCountMode cannot be changed from 'explicit' to '" + mode + "'", "NotSupportedError");
      }
      if (AUDIO_TWO[kind] && mode === "max") {
        throw new $DOMException(words + "'max' is not allowed.", "NotSupportedError");
      }
      this.#mode = mode;
      this.#channels("mode", mode, "channelCountMode");
    }

    get channelInterpretation() { return this.#interp; }
    set channelInterpretation(v) {
      const interp = str(v);
      if (interp !== "speakers" && interp !== "discrete") return;
      if (this.#kind === "splitter" && interp !== this.#interp) {
        throw new $DOMException(setWords("channelInterpretation", "AudioNode") + "Cannot change channel interpretation.", "InvalidStateError");
      }
      this.#interp = interp;
      this.#channels("interpretation", interp, "channelInterpretation");
    }

    connect(destination, output, input) {
      need(arguments.length, 1, "connect", "AudioNode");
      const out = output === undefined ? 0 : unsigned(output);
      if (destination instanceof AudioParam) {
        const target = audioParamTarget(destination);
        if (target.ctx !== this.#ctx) {
          throw domError("InvalidAccessError", "cannot connect to an AudioParam belonging to a different audio context.", "connect", "AudioNode");
        }
        if (out >= this.#outputs) {
          throw domError("IndexSizeError", "output index (" + out + ") exceeds number of outputs (" + this.#outputs + ").", "connect", "AudioNode");
        }
        audioCall({ op: "connectParam", ctx: audioContextId(this.#ctx), from: this.#node, output: out, to: target.node, input: target.index }, null, "connect", "AudioNode");
        return undefined;
      }
      if (!(destination instanceof AudioNode)) {
        throw new $TypeError(prefix("connect", "AudioNode") + "parameter 1 is not of type 'AudioNode'.");
      }
      if (destination.#ctx !== this.#ctx) {
        throw domError("InvalidAccessError", "cannot connect to an AudioNode belonging to a different audio context.", "connect", "AudioNode");
      }
      if (out >= this.#outputs) {
        throw domError("IndexSizeError", "output index (" + out + ") exceeds number of outputs (" + this.#outputs + ").", "connect", "AudioNode");
      }
      const into = input === undefined ? 0 : unsigned(input);
      if (into >= destination.#inputs) {
        throw domError("IndexSizeError", "input index (" + into + ") exceeds number of inputs (" + destination.#inputs + ").", "connect", "AudioNode");
      }
      audioCall({ op: "connect", ctx: audioContextId(this.#ctx), from: this.#node, output: out, to: destination.#node, input: into }, null, "connect", "AudioNode");
      return destination;
    }

    disconnect(destinationOrOutput, output, input) {
      const op = { op: "disconnect", ctx: audioContextId(this.#ctx), from: this.#node };
      const n = arguments.length;
      if (n === 0) {
        audioCall(op, null, "disconnect", "AudioNode");
        return;
      }
      const outputAt = (value) => {
        const out = unsigned(value);
        if (out >= this.#outputs) {
          throw domError("IndexSizeError", "output index (" + out + ") exceeds number of outputs (" + this.#outputs + ").", "disconnect", "AudioNode");
        }
        return out;
      };
      if (typeof destinationOrOutput === "number") {
        op.output = outputAt(destinationOrOutput);
        audioCall(op, null, "disconnect", "AudioNode");
        return;
      }
      if (destinationOrOutput instanceof AudioParam) {
        const target = audioParamTarget(destinationOrOutput);
        op.to = target.node;
        op.input = target.index;
        op.param = true;
      } else if (destinationOrOutput instanceof AudioNode) {
        op.to = destinationOrOutput.#node;
      } else {
        throw new $TypeError(prefix("disconnect", "AudioNode") + "parameter 1 is not of type 'AudioNode'.");
      }
      if (n > 1) op.output = outputAt(output);
      if (n > 2 && destinationOrOutput instanceof AudioNode) {
        const into = unsigned(input);
        if (into >= destinationOrOutput.#inputs) {
          throw domError("IndexSizeError", "input index (" + into + ") exceeds number of inputs (" + destinationOrOutput.#inputs + ").", "disconnect", "AudioNode");
        }
        op.input = into;
      }
      if (audioCall(op, null, "disconnect", "AudioNode").result === 0) {
        throw domError("InvalidAccessError", "the given destination is not connected.", "disconnect", "AudioNode");
      }
    }
  }
  expose(AudioNode, "AudioNode");

  // Makes a node of `kind` in `ctx`: its number and shape.
  function audioMake(ctx, kind, extra, iface) {
    if (!(ctx instanceof BaseAudioContext)) {
      throw new $TypeError("Failed to construct '" + iface + "': parameter 1 is not of type 'BaseAudioContext'.");
    }
    const op = audioMerge({ op: "newNode", ctx: audioContextId(ctx), kind }, extra);
    return audioCall(op, null, "Failed to construct '" + iface + "': ", null).result;
  }

  // The node's params, made and given their names.
  function audioParams(node, ctx, info, names, fixed) {
    for (let i = 0; i < names.length; i++) {
      const param = makeAudioParam(ctx, info.node, i, info.params[i], fixed !== undefined && fixed[i], false);
      defineProperty(node, names[i], { value: param, enumerable: true, configurable: true });
    }
  }

  // The options every node takes: its channel count, mode and
  // interpretation.
  // Options that are not a dictionary, and a mode or interpretation
  // that is not of its enum, throw as the reference's (WPT, M40 step 9).
  function audioNodeOptions(node, options) {
    if (options === undefined || options === null) return;
    const iface = node.constructor.name;
    const words = "Failed to construct '" + iface + "': ";
    if (typeof options !== "object" && typeof options !== "function") {
      throw new $TypeError(words + "The provided value is not of type '" + StringSlice(iface, 0, -4) + "Options" + "'.");
    }
    const enumOf = (name, type, values) => {
      const value = options[name];
      if (value === undefined) return undefined;
      const text = str(value);
      if (!listed(values, text)) {
        throw new $TypeError(words + "Failed to read the '" + name + "' property from 'AudioNodeOptions': The provided value '" + text + "' is not a valid enum value of type " + type + ".");
      }
      return text;
    };
    const count = options.channelCount;
    const mode = enumOf("channelCountMode", "ChannelCountMode", ["max", "clamped-max", "explicit"]);
    const interp = enumOf("channelInterpretation", "ChannelInterpretation", ["speakers", "discrete"]);
    if (count !== undefined) node.channelCount = count;
    if (mode !== undefined) node.channelCountMode = mode;
    if (interp !== undefined) node.channelInterpretation = interp;
  }

  function audioInitParams(node, options, names) {
    if (options === undefined || options === null) return;
    for (let i = 0; i < names.length; i++) {
      if (options[names[i]] !== undefined) node[names[i]].value = options[names[i]];
    }
  }

  function audioSet(node, what, value, property, iface) {
    const op = { op: "set", ctx: audioContextId(audioNodeContext(node)), node: audioNodeId(node), what };
    if (what === "curve") op.curve = value; else op.value = value;
    audioCall(op, null, setWords(property, iface), null);
  }

  class AudioDestinationNode extends AudioNode {
    constructor(key, ctx, info) {
      if (key !== KEY) throw illegalConstructor("AudioDestinationNode");
      super(KEY, ctx, info, "destination");
    }
    get maxChannelCount() { return audioMaxChannels(this.context); }
  }
  expose(AudioDestinationNode, "AudioDestinationNode");

  // ---- Scheduled sources ----

  class AudioScheduledSourceNode extends AudioNode {
    #started = false;

    constructor(key, ctx, info, kind) {
      if (key !== KEY) throw illegalConstructor("AudioScheduledSourceNode");
      super(KEY, ctx, info, kind);
    }

    start(when, offset, duration) {
      const buffered = this instanceof AudioBufferSourceNode;
      const iface = buffered ? "AudioBufferSourceNode" : "AudioScheduledSourceNode";
      const words = prefix("start", iface);
      const t = when === undefined ? 0 : audioTime(when, "start time", words);
      const op = { op: "start", ctx: audioContextId(this.context), node: audioNodeId(this), when: t };
      if (buffered) {
        if (offset !== undefined) op.offset = audioTime(offset, "offset", words);
        if (duration !== undefined) op.duration = audioTime(duration, "duration", words);
      }
      if (this.#started) {
        throw new $DOMException(words + "cannot call start more than once.", "InvalidStateError");
      }
      this.#started = true;
      MapSet(audioSources, op.ctx + ":" + op.node, this);
      audioCall(op, null, "start", iface);
    }

    stop(when) {
      const words = prefix("stop", "AudioScheduledSourceNode");
      const t = when === undefined ? 0 : audioTime(when, "stop time", words);
      if (!this.#started) {
        throw new $DOMException(words + "cannot call stop without calling start first.", "InvalidStateError");
      }
      audioCall({ op: "stop", ctx: audioContextId(this.context), node: audioNodeId(this), when: t }, null, "stop", "AudioScheduledSourceNode");
    }
  }
  expose(AudioScheduledSourceNode, "AudioScheduledSourceNode");
  defineHandlers(AudioScheduledSourceNode.prototype, ["ended"], false);

  class ConstantSourceNode extends AudioScheduledSourceNode {
    constructor(context, options) {
      needToConstruct(arguments.length, 1, "ConstantSourceNode");
      const info = audioMake(context, "constant", undefined, "ConstantSourceNode");
      super(KEY, context, info, "constant");
      audioParams(this, context, info, ["offset"]);
      audioInitParams(this, options, ["offset"]);
    }
  }
  expose(ConstantSourceNode, "ConstantSourceNode");

  const OSCILLATOR_TYPES = { __proto__: null, sine: true, square: true, sawtooth: true, triangle: true };

  class OscillatorNode extends AudioScheduledSourceNode {
    #type = "sine";

    constructor(context, options) {
      needToConstruct(arguments.length, 1, "OscillatorNode");
      const info = audioMake(context, "oscillator", undefined, "OscillatorNode");
      super(KEY, context, info, "oscillator");
      audioParams(this, context, info, ["frequency", "detune"]);
      if (options !== undefined && options !== null) {
        audioNodeOptions(this, options);
        audioInitParams(this, options, ["frequency", "detune"]);
        if (options.type === "custom" && options.periodicWave === undefined) {
          throw new $DOMException("Failed to construct 'OscillatorNode': A PeriodicWave must be specified if the type is set to \"custom\"", "InvalidStateError");
        }
        if (options.periodicWave !== undefined) {
          this.setPeriodicWave(options.periodicWave);
        } else if (options.type !== undefined) {
          this.type = options.type;
        }
      }
    }

    get type() { return this.#type; }
    set type(v) {
      const type = str(v);
      if (type === "custom") {
        throw new $DOMException(setWords("type", "OscillatorNode") + "'type' cannot be set directly to 'custom'.  Use setPeriodicWave() to create a custom Oscillator type.", "InvalidStateError");
      }
      if (!OSCILLATOR_TYPES[type]) return;
      this.#type = type;
      audioSet(this, "type", type, "type", "OscillatorNode");
    }

    setPeriodicWave(periodicWave) {
      need(arguments.length, 1, "setPeriodicWave", "OscillatorNode");
      if (!(periodicWave instanceof PeriodicWave)) {
        throw new $TypeError(prefix("setPeriodicWave", "OscillatorNode") + "parameter 1 is not of type 'PeriodicWave'.");
      }
      this.#type = "custom";
      audioCall({ op: "set", ctx: audioContextId(this.context), node: audioNodeId(this), what: "wave", wave: periodicWaveId(periodicWave) }, null, "setPeriodicWave", "OscillatorNode");
    }
  }
  expose(OscillatorNode, "OscillatorNode");

  // Gives `buffer` (or none) to a source or a convolver.
  function audioGiveBuffer(node, buffer, normalize, words) {
    const op = { op: "buffer", ctx: audioContextId(node.context), node: audioNodeId(node), normalize };
    if (buffer === null) {
      op.none = true;
      audioCall(op, null, words, null);
      return;
    }
    const given = audioBufferPayload(buffer, false);
    if (audioCall(audioMerge(op, given.op), given.bytes, words, null).result === "unknown") {
      // The engine let its copy go: the samples, then.
      const again = audioBufferPayload(buffer, true);
      audioCall(audioMerge({ op: "buffer", ctx: op.ctx, node: op.node, normalize }, again.op), again.bytes, words, null);
    }
  }

  class AudioBufferSourceNode extends AudioScheduledSourceNode {
    #buffer = null;
    #loop = false;
    #loopStart = 0;
    #loopEnd = 0;

    constructor(context, options) {
      needToConstruct(arguments.length, 1, "AudioBufferSourceNode");
      const info = audioMake(context, "bufferSource", undefined, "AudioBufferSourceNode");
      super(KEY, context, info, "bufferSource");
      audioParams(this, context, info, ["playbackRate", "detune"], [true, true]);
      if (options !== undefined && options !== null) {
        audioInitParams(this, options, ["playbackRate", "detune"]);
        if (options.buffer !== undefined && options.buffer !== null) this.buffer = options.buffer;
        if (options.loop !== undefined) this.loop = options.loop;
        if (options.loopStart !== undefined) this.loopStart = options.loopStart;
        if (options.loopEnd !== undefined) this.loopEnd = options.loopEnd;
      }
    }

    get buffer() { return this.#buffer; }
    set buffer(v) {
      const words = setWords("buffer", "AudioBufferSourceNode");
      if (v !== null && !(v instanceof AudioBuffer)) {
        throw new $TypeError(words + "The provided value is not of type 'AudioBuffer'.");
      }
      if (v !== null && this.#buffer !== null) {
        throw new $DOMException(words + "Cannot set buffer to non-null after it has been already been set to a non-null buffer", "InvalidStateError");
      }
      this.#buffer = v;
      audioGiveBuffer(this, v, true, words);
    }

    get loop() { return this.#loop; }
    set loop(v) {
      this.#loop = !!v;
      audioSet(this, "loop", this.#loop, "loop", "AudioBufferSourceNode");
    }
    get loopStart() { return this.#loopStart; }
    set loopStart(v) {
      this.#loopStart = audioFloat(v, setWords("loopStart", "AudioBufferSourceNode"), true);
      audioSet(this, "loopStart", this.#loopStart, "loopStart", "AudioBufferSourceNode");
    }
    get loopEnd() { return this.#loopEnd; }
    set loopEnd(v) {
      this.#loopEnd = audioFloat(v, setWords("loopEnd", "AudioBufferSourceNode"), true);
      audioSet(this, "loopEnd", this.#loopEnd, "loopEnd", "AudioBufferSourceNode");
    }
  }
  expose(AudioBufferSourceNode, "AudioBufferSourceNode");

  // ---- Processing nodes ----

  class GainNode extends AudioNode {
    constructor(context, options) {
      needToConstruct(arguments.length, 1, "GainNode");
      const info = audioMake(context, "gain", undefined, "GainNode");
      super(KEY, context, info, "gain");
      audioParams(this, context, info, ["gain"]);
      audioNodeOptions(this, options);
      audioInitParams(this, options, ["gain"]);
    }
  }
  expose(GainNode, "GainNode");

  class DelayNode extends AudioNode {
    constructor(context, options) {
      needToConstruct(arguments.length, 1, "DelayNode");
      const words = "Failed to construct 'DelayNode': ";
      const max = options === undefined || options === null || options.maxDelayTime === undefined ? 1 : audioFloat(options.maxDelayTime, words, true);
      if (!(max > 0 && max < 180)) {
        throw new $DOMException(words + "The max delay time provided (" + max + ") is outside the range (0, 180).", "NotSupportedError");
      }
      const info = audioMake(context, "delay", { max }, "DelayNode");
      super(KEY, context, info, "delay");
      audioParams(this, context, info, ["delayTime"]);
      audioNodeOptions(this, options);
      audioInitParams(this, options, ["delayTime"]);
    }
  }
  expose(DelayNode, "DelayNode");

  const BIQUAD_TYPES = { __proto__: null, lowpass: true, highpass: true, bandpass: true, lowshelf: true, highshelf: true, peaking: true, notch: true, allpass: true };

  // `getFrequencyResponse()` of a biquad or an IIR filter.
  function audioResponse(node, frequencyHz, magResponse, phaseResponse, iface) {
    const arrays = [frequencyHz, magResponse, phaseResponse];
    for (let i = 0; i < 3; i++) {
      if (!(arrays[i] instanceof $Float32Array)) {
        throw new $TypeError(prefix("getFrequencyResponse", iface) + "parameter " + (i + 1) + " is not of type 'Float32Array'.");
      }
    }
    const n = frequencyHz.length;
    if (magResponse.length !== n) {
      throw domError("InvalidAccessError", "magResponse length (" + magResponse.length + ") is not equal to frequencyHz length (" + n + ").", "getFrequencyResponse", iface);
    }
    if (phaseResponse.length !== n) {
      throw domError("InvalidAccessError", "phaseResponse length (" + phaseResponse.length + ") is not equal to frequencyHz length (" + n + ").", "getFrequencyResponse", iface);
    }
    const { bytes } = audioCall({ op: "response", ctx: audioContextId(node.context), node: audioNodeId(node) }, floatBytes([frequencyHz]), "getFrequencyResponse", iface);
    const all = new $Float32Array(bytes);
    for (let i = 0; i < n && i < all.length; i++) {
      magResponse[i] = all[i];
      phaseResponse[i] = all[n + i];
    }
  }

  class BiquadFilterNode extends AudioNode {
    #type = "lowpass";

    constructor(context, options) {
      needToConstruct(arguments.length, 1, "BiquadFilterNode");
      const info = audioMake(context, "biquad", undefined, "BiquadFilterNode");
      super(KEY, context, info, "biquad");
      audioParams(this, context, info, ["frequency", "detune", "Q", "gain"]);
      audioNodeOptions(this, options);
      audioInitParams(this, options, ["frequency", "detune", "Q", "gain"]);
      if (options !== undefined && options !== null && options.type !== undefined) this.type = options.type;
    }

    get type() { return this.#type; }
    set type(v) {
      const type = str(v);
      if (!BIQUAD_TYPES[type]) return;
      this.#type = type;
      audioSet(this, "type", type, "type", "BiquadFilterNode");
    }

    getFrequencyResponse(frequencyHz, magResponse, phaseResponse) {
      need(arguments.length, 3, "getFrequencyResponse", "BiquadFilterNode");
      audioResponse(this, frequencyHz, magResponse, phaseResponse, "BiquadFilterNode");
    }
  }
  expose(BiquadFilterNode, "BiquadFilterNode");

  function audioCoefficients(list, name) {
    const words = "Failed to construct 'IIRFilterNode': ";
    if (list === undefined || list === null) {
      throw new $TypeError(words + "Failed to read the '" + name + "' property from 'IIRFilterOptions': Required member is undefined.");
    }
    const n = list.length >>> 0;
    if (n < 1 || n > 20) {
      throw new $DOMException(words + "The " + name + " array length provided (" + n + ") is outside the range [1, 20].", "NotSupportedError");
    }
    const out = [];
    for (let i = 0; i < n; i++) ArrayPush(out, audioFloat(list[i], words, true));
    return out;
  }

  class IIRFilterNode extends AudioNode {
    constructor(context, options) {
      needToConstruct(arguments.length, 2, "IIRFilterNode");
      const feedforward = audioCoefficients(options.feedforward, "feedforward");
      const feedback = audioCoefficients(options.feedback, "feedback");
      if (feedback[0] === 0) {
        throw new $DOMException("Failed to construct 'IIRFilterNode': First feedback coefficient cannot be zero.", "InvalidStateError");
      }
      let zero = true;
      for (let i = 0; i < feedforward.length; i++) if (feedforward[i] !== 0) zero = false;
      if (zero) {
        throw new $DOMException("Failed to construct 'IIRFilterNode': At least one feedforward coefficient must be non-zero.", "InvalidStateError");
      }
      const info = audioMake(context, "iir", { feedforward, feedback }, "IIRFilterNode");
      super(KEY, context, info, "iir");
      audioNodeOptions(this, options);
    }

    getFrequencyResponse(frequencyHz, magResponse, phaseResponse) {
      need(arguments.length, 3, "getFrequencyResponse", "IIRFilterNode");
      audioResponse(this, frequencyHz, magResponse, phaseResponse, "IIRFilterNode");
    }
  }
  expose(IIRFilterNode, "IIRFilterNode");

  class StereoPannerNode extends AudioNode {
    constructor(context, options) {
      needToConstruct(arguments.length, 1, "StereoPannerNode");
      const info = audioMake(context, "stereoPanner", undefined, "StereoPannerNode");
      super(KEY, context, info, "stereoPanner");
      audioParams(this, context, info, ["pan"]);
      audioNodeOptions(this, options);
      audioInitParams(this, options, ["pan"]);
    }
  }
  expose(StereoPannerNode, "StereoPannerNode");

  const PANNER_NUMBERS = ["refDistance", "maxDistance", "rolloffFactor", "coneInnerAngle", "coneOuterAngle", "coneOuterGain"];
  const PANNER_PARAMS = ["positionX", "positionY", "positionZ", "orientationX", "orientationY", "orientationZ"];

  class PannerNode extends AudioNode {
    #model = "equalpower";
    #distance = "inverse";
    #numbers = { refDistance: 1, maxDistance: 10000, rolloffFactor: 1, coneInnerAngle: 360, coneOuterAngle: 360, coneOuterGain: 0 };

    constructor(context, options) {
      needToConstruct(arguments.length, 1, "PannerNode");
      const info = audioMake(context, "panner", undefined, "PannerNode");
      super(KEY, context, info, "panner");
      audioParams(this, context, info, PANNER_PARAMS);
      audioNodeOptions(this, options);
      audioInitParams(this, options, PANNER_PARAMS);
      if (options !== undefined && options !== null) {
        if (options.panningModel !== undefined) this.panningModel = options.panningModel;
        if (options.distanceModel !== undefined) this.distanceModel = options.distanceModel;
        for (let i = 0; i < PANNER_NUMBERS.length; i++) {
          if (options[PANNER_NUMBERS[i]] !== undefined) this[PANNER_NUMBERS[i]] = options[PANNER_NUMBERS[i]];
        }
      }
    }

    // HRTF is kept as the page's choice; it renders as equal power.
    get panningModel() { return this.#model; }
    set panningModel(v) {
      const model = str(v);
      if (model === "equalpower" || model === "HRTF") this.#model = model;
    }
    get distanceModel() { return this.#distance; }
    set distanceModel(v) {
      const model = str(v);
      if (model !== "linear" && model !== "inverse" && model !== "exponential") return;
      this.#distance = model;
      audioSet(this, "distanceModel", model, "distanceModel", "PannerNode");
    }

    setPosition(x, y, z) {
      need(arguments.length, 3, "setPosition", "PannerNode");
      const values = audioFloats([x, y, z], prefix("setPosition", "PannerNode"));
      this.positionX.value = values[0];
      this.positionY.value = values[1];
      this.positionZ.value = values[2];
    }

    setOrientation(x, y, z) {
      need(arguments.length, 3, "setOrientation", "PannerNode");
      const values = audioFloats([x, y, z], prefix("setOrientation", "PannerNode"));
      this.orientationX.value = values[0];
      this.orientationY.value = values[1];
      this.orientationZ.value = values[2];
    }

    static {
      for (let i = 0; i < PANNER_NUMBERS.length; i++) {
        const name = PANNER_NUMBERS[i];
        defineProperty(PannerNode.prototype, name, {
          get() { return this.#numbers[name]; },
          set(v) {
            const words = setWords(name, "PannerNode");
            const value = audioFloat(v, words, true);
            if ((name === "refDistance" || name === "rolloffFactor") && value < 0) {
              throw new $RangeError(words + "The " + name + " provided (" + value + ") is less than the minimum bound (0).");
            }
            if (name === "maxDistance" && !(value > 0)) {
              throw new $RangeError(words + "The maxDistance provided (" + value + ") is less than or equal to the minimum bound (0).");
            }
            if (name === "coneOuterGain" && (value < 0 || value > 1)) {
              throw new $DOMException(words + "The coneOuterGain provided (" + value + ") is outside the range [0, 1].", "InvalidStateError");
            }
            this.#numbers[name] = value;
            audioSet(this, name, value, name, "PannerNode");
          },
          enumerable: true,
          configurable: true,
        });
      }
    }
  }
  expose(PannerNode, "PannerNode");

  const LISTENER_PARAMS = ["positionX", "positionY", "positionZ", "forwardX", "forwardY", "forwardZ", "upX", "upY", "upZ"];
  const LISTENER_DEFAULTS = [0, 0, 0, 0, 0, -1, 0, 1, 0];

  class AudioListener {
    constructor(key, ctx) {
      if (key !== KEY) throw illegalConstructor("AudioListener");
      for (let i = 0; i < LISTENER_PARAMS.length; i++) {
        const info = { default: LISTENER_DEFAULTS[i], min: -3.4028234663852886e38, max: 3.4028234663852886e38, rate: "a-rate" };
        defineProperty(this, LISTENER_PARAMS[i], { value: makeAudioParam(ctx, 0, i, info, false, true), enumerable: true, configurable: true });
      }
    }

    setPosition(x, y, z) {
      need(arguments.length, 3, "setPosition", "AudioListener");
      const values = audioFloats([x, y, z], prefix("setPosition", "AudioListener"));
      this.positionX.value = values[0];
      this.positionY.value = values[1];
      this.positionZ.value = values[2];
    }

    setOrientation(x, y, z, xUp, yUp, zUp) {
      need(arguments.length, 6, "setOrientation", "AudioListener");
      const values = audioFloats([x, y, z, xUp, yUp, zUp], prefix("setOrientation", "AudioListener"));
      this.forwardX.value = values[0];
      this.forwardY.value = values[1];
      this.forwardZ.value = values[2];
      this.upX.value = values[3];
      this.upY.value = values[4];
      this.upZ.value = values[5];
    }
  }
  expose(AudioListener, "AudioListener");

  // A splitter's or merger's count, from its options.
  function audioCount(options, member, iface) {
    const count = options === undefined || options === null || options[member] === undefined ? 6 : unsigned(options[member]);
    if (count < 1 || count > 32) {
      const what = member === "numberOfOutputs" ? "outputs" : "inputs";
      throw new $DOMException("Failed to construct '" + iface + "': The number of " + what + " provided (" + count + ") is outside the range [1, 32].", "IndexSizeError");
    }
    return count;
  }

  class ChannelSplitterNode extends AudioNode {
    constructor(context, options) {
      needToConstruct(arguments.length, 1, "ChannelSplitterNode");
      const count = audioCount(options, "numberOfOutputs", "ChannelSplitterNode");
      const info = audioMake(context, "splitter", { count }, "ChannelSplitterNode");
      super(KEY, context, info, "splitter");
      audioNodeOptions(this, options);
    }
  }
  expose(ChannelSplitterNode, "ChannelSplitterNode");

  class ChannelMergerNode extends AudioNode {
    constructor(context, options) {
      needToConstruct(arguments.length, 1, "ChannelMergerNode");
      const count = audioCount(options, "numberOfInputs", "ChannelMergerNode");
      const info = audioMake(context, "merger", { count }, "ChannelMergerNode");
      super(KEY, context, info, "merger");
      audioNodeOptions(this, options);
    }
  }
  expose(ChannelMergerNode, "ChannelMergerNode");

  class WaveShaperNode extends AudioNode {
    #curve = null;
    #oversample = "none";

    constructor(context, options) {
      needToConstruct(arguments.length, 1, "WaveShaperNode");
      const info = audioMake(context, "shaper", undefined, "WaveShaperNode");
      super(KEY, context, info, "shaper");
      audioNodeOptions(this, options);
      if (options !== undefined && options !== null) {
        if (options.curve !== undefined && options.curve !== null) this.curve = new $Float32Array(options.curve);
        if (options.oversample !== undefined) this.oversample = options.oversample;
      }
    }

    get curve() { return this.#curve; }
    set curve(v) {
      const words = setWords("curve", "WaveShaperNode");
      if (v !== null) {
        if (!(v instanceof $Float32Array)) {
          throw new $TypeError(words + "The provided value is not of type 'Float32Array'.");
        }
        if (v.length < 2) {
          throw new $DOMException(words + "The curve length provided (" + v.length + ") is less than the minimum bound (2).", "InvalidStateError");
        }
        if (this.#curve !== null) {
          throw new $DOMException(words + "A curve can only be set once.", "InvalidStateError");
        }
      }
      // The node keeps a copy: the page's array is its own.
      this.#curve = v === null ? null : new $Float32Array(v);
      const values = [];
      if (this.#curve !== null) for (let i = 0; i < this.#curve.length; i++) ArrayPush(values, this.#curve[i]);
      audioSet(this, "curve", this.#curve === null ? null : values, "curve", "WaveShaperNode");
    }

    get oversample() { return this.#oversample; }
    set oversample(v) {
      const value = str(v);
      if (value !== "none" && value !== "2x" && value !== "4x") return;
      this.#oversample = value;
      audioSet(this, "oversample", value, "oversample", "WaveShaperNode");
    }
  }
  expose(WaveShaperNode, "WaveShaperNode");

  const COMPRESSOR_PARAMS = ["threshold", "knee", "ratio", "attack", "release"];

  class DynamicsCompressorNode extends AudioNode {
    constructor(context, options) {
      needToConstruct(arguments.length, 1, "DynamicsCompressorNode");
      const info = audioMake(context, "compressor", undefined, "DynamicsCompressorNode");
      super(KEY, context, info, "compressor");
      audioParams(this, context, info, COMPRESSOR_PARAMS, [true, true, true, true, true]);
      audioNodeOptions(this, options);
      audioInitParams(this, options, COMPRESSOR_PARAMS);
    }

    get reduction() {
      return audioCall({ op: "reduction", ctx: audioContextId(this.context), node: audioNodeId(this) }, null, "reduction", "DynamicsCompressorNode").result;
    }
  }
  expose(DynamicsCompressorNode, "DynamicsCompressorNode");

  class ConvolverNode extends AudioNode {
    #buffer = null;
    #normalize = true;

    constructor(context, options) {
      needToConstruct(arguments.length, 1, "ConvolverNode");
      const info = audioMake(context, "convolver", undefined, "ConvolverNode");
      super(KEY, context, info, "convolver");
      audioNodeOptions(this, options);
      if (options !== undefined && options !== null) {
        if (options.disableNormalization !== undefined) this.#normalize = !options.disableNormalization;
        if (options.buffer !== undefined && options.buffer !== null) this.buffer = options.buffer;
      }
    }

    get buffer() { return this.#buffer; }
    set buffer(v) {
      const words = setWords("buffer", "ConvolverNode");
      if (v !== null && !(v instanceof AudioBuffer)) {
        throw new $TypeError(words + "The provided value is not of type 'AudioBuffer'.");
      }
      if (v !== null) {
        const n = v.numberOfChannels;
        if (n !== 1 && n !== 2 && n !== 4) {
          throw new $DOMException(words + "The buffer must have 1, 2, or 4 channels, not " + n, "NotSupportedError");
        }
        if (v.sampleRate !== this.context.sampleRate) {
          throw new $DOMException(words + "The buffer sample rate " + v.sampleRate + " does not match the context rate " + this.context.sampleRate + " Hz.", "NotSupportedError");
        }
      }
      this.#buffer = v;
      audioGiveBuffer(this, v, this.#normalize, words);
    }

    get normalize() { return this.#normalize; }
    set normalize(v) { this.#normalize = !!v; }
  }
  expose(ConvolverNode, "ConvolverNode");

  class AnalyserNode extends AudioNode {
    #fft = 2048;
    #min = -100;
    #max = -30;
    #smoothing = 0.8;

    constructor(context, options) {
      needToConstruct(arguments.length, 1, "AnalyserNode");
      const info = audioMake(context, "analyser", undefined, "AnalyserNode");
      super(KEY, context, info, "analyser");
      audioNodeOptions(this, options);
      if (options !== undefined && options !== null) {
        if (options.fftSize !== undefined) this.fftSize = options.fftSize;
        if (options.maxDecibels !== undefined) this.maxDecibels = options.maxDecibels;
        if (options.minDecibels !== undefined) this.minDecibels = options.minDecibels;
        if (options.smoothingTimeConstant !== undefined) this.smoothingTimeConstant = options.smoothingTimeConstant;
      }
    }

    get fftSize() { return this.#fft; }
    set fftSize(v) {
      const n = unsigned(v);
      const words = setWords("fftSize", "AnalyserNode");
      if (n < 32 || n > 32768) {
        throw new $DOMException(words + "The FFT size provided (" + n + ") is outside the range [32, 32768].", "IndexSizeError");
      }
      if ((n & (n - 1)) !== 0) {
        throw new $DOMException(words + "The value provided (" + n + ") is not a power of two.", "IndexSizeError");
      }
      this.#fft = n;
      audioSet(this, "fftSize", n, "fftSize", "AnalyserNode");
    }
    get frequencyBinCount() { return this.#fft / 2; }
    get minDecibels() { return this.#min; }
    set minDecibels(v) {
      const words = setWords("minDecibels", "AnalyserNode");
      const value = audioFloat(v, words, true);
      if (value >= this.#max) {
        throw new $DOMException(words + "The minDecibels provided (" + value + ") is greater than or equal to the maximum bound (" + this.#max + ").", "IndexSizeError");
      }
      this.#min = value;
      audioSet(this, "minDecibels", value, "minDecibels", "AnalyserNode");
    }
    get maxDecibels() { return this.#max; }
    set maxDecibels(v) {
      const words = setWords("maxDecibels", "AnalyserNode");
      const value = audioFloat(v, words, true);
      if (value <= this.#min) {
        throw new $DOMException(words + "The maxDecibels provided (" + value + ") is less than or equal to the minimum bound (" + this.#min + ").", "IndexSizeError");
      }
      this.#max = value;
      audioSet(this, "maxDecibels", value, "maxDecibels", "AnalyserNode");
    }
    get smoothingTimeConstant() { return this.#smoothing; }
    set smoothingTimeConstant(v) {
      const words = setWords("smoothingTimeConstant", "AnalyserNode");
      const value = audioFloat(v, words, true);
      if (value < 0 || value > 1) {
        throw new $DOMException(words + "The smoothing value provided (" + value + ") is outside the range [0, 1].", "IndexSizeError");
      }
      this.#smoothing = value;
      audioSet(this, "smoothingTimeConstant", value, "smoothingTimeConstant", "AnalyserNode");
    }

    #data(what, array, method, floatsWanted) {
      if (!(array instanceof (floatsWanted ? $Float32Array : $Uint8Array))) {
        throw new $TypeError(prefix(method, "AnalyserNode") + "parameter 1 is not of type '" + (floatsWanted ? "Float32Array" : "Uint8Array") + "'.");
      }
      const time = what === "floatTime" || what === "byteTime";
      const length = MathMin(array.length, time ? this.#fft : this.#fft / 2);
      const { bytes } = audioCall({ op: "analyser", ctx: audioContextId(this.context), node: audioNodeId(this), what, length }, null, method, "AnalyserNode");
      const got = floatsWanted ? new $Float32Array(bytes) : new $Uint8Array(bytes);
      for (let i = 0; i < length && i < got.length; i++) array[i] = got[i];
    }

    getFloatFrequencyData(array) {
      need(arguments.length, 1, "getFloatFrequencyData", "AnalyserNode");
      this.#data("floatFrequency", array, "getFloatFrequencyData", true);
    }
    getByteFrequencyData(array) {
      need(arguments.length, 1, "getByteFrequencyData", "AnalyserNode");
      this.#data("byteFrequency", array, "getByteFrequencyData", false);
    }
    getFloatTimeDomainData(array) {
      need(arguments.length, 1, "getFloatTimeDomainData", "AnalyserNode");
      this.#data("floatTime", array, "getFloatTimeDomainData", true);
    }
    getByteTimeDomainData(array) {
      need(arguments.length, 1, "getByteTimeDomainData", "AnalyserNode");
      this.#data("byteTime", array, "getByteTimeDomainData", false);
    }
  }
  expose(AnalyserNode, "AnalyserNode");

  let periodicWaveId;

  class PeriodicWave {
    #id;

    constructor(context, options) {
      needToConstruct(arguments.length, 1, "PeriodicWave");
      const words = "Failed to construct 'PeriodicWave': ";
      if (!(context instanceof BaseAudioContext)) {
        throw new $TypeError(words + "parameter 1 is not of type 'BaseAudioContext'.");
      }
      const given = options === undefined || options === null ? {} : options;
      let real = given.real;
      let imag = given.imag;
      if (real === undefined && imag === undefined) {
        real = [0, 0];
        imag = [0, 1];
      } else if (real === undefined) {
        real = new $Float32Array(imag.length >>> 0);
      } else if (imag === undefined) {
        imag = new $Float32Array(real.length >>> 0);
      }
      if (real.length !== imag.length) {
        throw new $DOMException(words + "real and imag arrays must be the same length (" + real.length + " vs " + imag.length + ").", "IndexSizeError");
      }
      if (real.length < 2) {
        throw new $DOMException(words + "The length of the real array provided (" + real.length + ") is less than the minimum bound (2).", "IndexSizeError");
      }
      const re = new $Float32Array(real);
      const im = new $Float32Array(imag);
      for (let i = 0; i < re.length; i++) {
        if (!$NumberIsFinite(re[i]) || !$NumberIsFinite(im[i])) {
          throw new $TypeError(words + "The provided float value is non-finite.");
        }
      }
      const op = { op: "wave", ctx: audioContextId(context), length: re.length, disableNormalization: !!given.disableNormalization };
      this.#id = audioCall(op, floatBytes([re, im]), words, null).result;
    }

    static {
      periodicWaveId = (wave) => wave.#id;
    }
  }
  expose(PeriodicWave, "PeriodicWave");

  // ---- Contexts ----

  let audioContextId;
  let audioMaxChannels;
  let audioContextState;
  let audioContextWait;

  class BaseAudioContext extends EventTarget {
    #id; #rate; #channels; #state; #destination; #listener;
    // The promises of resume(), suspend() and close() waiting for the
    // state they ask for.
    #waiting = [];

    constructor(key, info, channels) {
      if (key !== KEY) throw illegalConstructor("BaseAudioContext");
      super();
      this.#id = info.ctx;
      this.#rate = info.rate;
      this.#channels = channels;
      this.#state = info.state;
      MapSet(audioContexts, this.#id, this);
      const destination = audioCall({ op: "newNode", ctx: this.#id, kind: "destination" }, null, "constructor", "BaseAudioContext").result;
      this.#destination = new AudioDestinationNode(KEY, this, destination);
      this.#listener = new AudioListener(KEY, this);
    }

    static {
      audioContextId = (ctx) => ctx.#id;
      audioMaxChannels = (ctx) => ctx.#channels;
      // The context is in `state` now: whether that changed it. The
      // promises waiting for it settle before its `statechange`, as the
      // standard and the reference browser order them.
      audioContextState = (ctx, state) => {
        const changed = ctx.#state !== state;
        ctx.#state = state;
        return changed;
      };
      audioContextWait = (ctx, state, settle) => {
        if (settle !== undefined) {
          ArrayPush(ctx.#waiting, { state, settle });
          return;
        }
        const still = [];
        const done = [];
        for (let i = 0; i < ctx.#waiting.length; i++) {
          const w = ctx.#waiting[i];
          ArrayPush(w.state === state || state === "closed" ? done : still, w);
        }
        ctx.#waiting = still;
        for (let i = 0; i < done.length; i++) {
          if (done[i].state === state) done[i].settle.resolve(undefined);
          else done[i].settle.reject(new $DOMException("Cannot resume a closed AudioContext.", "InvalidStateError"));
        }
      };
    }

    get destination() { return this.#destination; }
    get sampleRate() { return this.#rate; }
    get currentTime() {
      return audioCall({ op: "time", ctx: this.#id }, null, "currentTime", "BaseAudioContext").result;
    }
    get listener() { return this.#listener; }
    get state() { return this.#state; }
    // The render quantum, the standard's and the reference's (probe p1).
    get renderQuantumSize() { return 128; }

    createBuffer(numberOfChannels, length, sampleRate) {
      need(arguments.length, 3, "createBuffer", "BaseAudioContext");
      const channels = unsigned(numberOfChannels);
      const frames = unsigned(length);
      const rate = audioFloat(sampleRate, prefix("createBuffer", "BaseAudioContext"), false);
      audioBufferCheck(channels, frames, rate, prefix("createBuffer", "BaseAudioContext"));
      return new AudioBuffer({ numberOfChannels: channels, length: frames, sampleRate: rate });
    }
    createBufferSource() { return new AudioBufferSourceNode(this); }
    createConstantSource() { return new ConstantSourceNode(this); }
    createGain() { return new GainNode(this); }
    createOscillator() { return new OscillatorNode(this); }
    createBiquadFilter() { return new BiquadFilterNode(this); }
    createIIRFilter(feedforward, feedback) {
      need(arguments.length, 2, "createIIRFilter", "BaseAudioContext");
      return new IIRFilterNode(this, { feedforward, feedback });
    }
    createDelay(maxDelayTime) {
      const words = prefix("createDelay", "BaseAudioContext");
      const max = maxDelayTime === undefined ? 1 : audioFloat(maxDelayTime, words, true);
      if (!(max > 0 && max < 180)) {
        throw new $DOMException(words + "The max delay time provided (" + max + ") is outside the range (0, 180).", "NotSupportedError");
      }
      return new DelayNode(this, { maxDelayTime: max });
    }
    createStereoPanner() { return new StereoPannerNode(this); }
    createPanner() { return new PannerNode(this); }
    createChannelSplitter(numberOfOutputs) {
      const count = numberOfOutputs === undefined ? 6 : unsigned(numberOfOutputs);
      if (count < 1 || count > 32) {
        throw domError("IndexSizeError", "The number of outputs provided (" + count + ") is outside the range [1, 32].", "createChannelSplitter", "BaseAudioContext");
      }
      return new ChannelSplitterNode(this, { numberOfOutputs: count });
    }
    createChannelMerger(numberOfInputs) {
      const count = numberOfInputs === undefined ? 6 : unsigned(numberOfInputs);
      if (count < 1 || count > 32) {
        throw domError("IndexSizeError", "The number of inputs provided (" + count + ") is outside the range [1, 32].", "createChannelMerger", "BaseAudioContext");
      }
      return new ChannelMergerNode(this, { numberOfInputs: count });
    }
    createWaveShaper() { return new WaveShaperNode(this); }
    createDynamicsCompressor() { return new DynamicsCompressorNode(this); }
    createConvolver() { return new ConvolverNode(this); }
    createAnalyser() { return new AnalyserNode(this); }
    createPeriodicWave(real, imag, constraints) {
      need(arguments.length, 2, "createPeriodicWave", "BaseAudioContext");
      return new PeriodicWave(this, {
        real, imag,
        disableNormalization: constraints !== undefined && constraints !== null && !!constraints.disableNormalization,
      });
    }

    // Decodes on a thread of the engine's; the bytes are copied, so the
    // page's buffer is not detached.
    decodeAudioData(audioData, successCallback, errorCallback) {
      let length;
      try {
        length = bufferLength(audioData);
      } catch (_) {
        return rejected(new $TypeError(prefix("decodeAudioData", "BaseAudioContext") + "parameter 1 is not of type 'ArrayBuffer'."));
      }
      const id = ++audioPromiseNext;
      let settle;
      const promise = new $Promise((resolve, reject) => { settle = { resolve, reject }; });
      settle.success = typeof successCallback === "function" ? successCallback : null;
      settle.error = typeof errorCallback === "function" ? errorCallback : null;
      try {
        audioCall({ op: "decode", ctx: this.#id, promise: id }, new $Uint8Array(audioData, 0, length), "decodeAudioData", "BaseAudioContext");
      } catch (e) {
        return rejected(e);
      }
      MapSet(audioPromises, id, settle);
      return promise;
    }
  }
  expose(BaseAudioContext, "BaseAudioContext");
  defineHandlers(BaseAudioContext.prototype, ["statechange"], false);

  // Asks the engine for state `state` (by `op`): a promise that settles
  // once the engine tells the change.
  function audioAskState(ctx, op, state, method, iface) {
    let asked;
    try {
      asked = audioCall({ op, ctx: audioContextId(ctx) }, null, method, iface).result;
    } catch (e) {
      return rejected(e);
    }
    if (asked === state && ctx.state === state) return resolved(undefined);
    return new $Promise((resolve, reject) => audioContextWait(ctx, state, { resolve, reject }));
  }

  class AudioContext extends BaseAudioContext {
    constructor(options) {
      const given = options === undefined || options === null ? {} : options;
      const op = { op: "newContext", offline: false };
      const words = "Failed to construct 'AudioContext': ";
      if (typeof given !== "object" && typeof given !== "function") {
        throw new $TypeError(words + "The provided value is not of type 'AudioContextOptions'.");
      }
      if (given.sampleRate !== undefined) {
        const rate = audioFloat(given.sampleRate, words, false);
        if (!(rate >= 3000 && rate <= 768000)) {
          throw new $DOMException(words + "The hardware sample rate provided (" + rate + ") is outside the range [3000, 768000].", "NotSupportedError");
        }
        op.rate = rate;
      }
      const info = audioCall(op, null, words, null).result;
      super(KEY, info, 2);
    }

    get baseLatency() { return 0.01; }
    get outputLatency() { return 0; }

    getOutputTimestamp() {
      return { contextTime: this.currentTime, performanceTime: performance.now() };
    }

    // Starts the context: at once if the user has activated the page,
    // else when they do.
    resume() {
      if (this.state === "closed") {
        return rejected(domError("InvalidStateError", "Cannot resume a closed AudioContext.", "resume", "AudioContext"));
      }
      return audioAskState(this, "resume", "running", "resume", "AudioContext");
    }

    suspend() {
      if (this.state === "closed") {
        return rejected(domError("InvalidStateError", "Cannot suspend a closed AudioContext.", "suspend", "AudioContext"));
      }
      return audioAskState(this, "suspend", "suspended", "suspend", "AudioContext");
    }

    close() {
      if (this.state === "closed") {
        return rejected(domError("InvalidStateError", "Cannot close a context that is being closed or has already been closed.", "close", "AudioContext"));
      }
      return audioAskState(this, "close", "closed", "close", "AudioContext");
    }
  }
  expose(AudioContext, "AudioContext");

  // An element's sound taken into a graph (M40): its sound goes to the
  // graph, not the output, from then on.
  class MediaElementAudioSourceNode extends AudioNode {
    #element;

    constructor(context, options) {
      needToConstruct(arguments.length, 2, "MediaElementAudioSourceNode");
      if (!(context instanceof AudioContext)) {
        throw new $TypeError("Failed to construct 'MediaElementAudioSourceNode': parameter 1 is not of type 'AudioContext'.");
      }
      const element = options === null || typeof options !== "object" ? undefined : options.mediaElement;
      if (element === undefined) {
        throw new $TypeError("Failed to construct 'MediaElementAudioSourceNode': Failed to read the 'mediaElement' property from 'MediaElementAudioSourceOptions': Required member is undefined.");
      }
      if (!(element instanceof HTMLMediaElement)) {
        throw new $TypeError("Failed to construct 'MediaElementAudioSourceNode': Failed to read the 'mediaElement' property from 'MediaElementAudioSourceOptions': Failed to convert value to 'HTMLMediaElement'.");
      }
      const info = audioMake(context, "mediaElementSource", { element: self(element) }, "MediaElementAudioSourceNode");
      super(KEY, context, info, "mediaElementSource");
      this.#element = element;
    }

    get mediaElement() { return this.#element; }
  }
  expose(MediaElementAudioSourceNode, "MediaElementAudioSourceNode");

  include(
    {
      createMediaElementSource(mediaElement) {
        need(arguments.length, 1, "createMediaElementSource", "AudioContext");
        if (!(mediaElement instanceof HTMLMediaElement)) {
          throw new $TypeError(prefix("createMediaElementSource", "AudioContext") + "parameter 1 is not of type 'HTMLMediaElement'.");
        }
        try {
          return new MediaElementAudioSourceNode(this, { mediaElement });
        } catch (e) {
          if (e instanceof $DOMException) {
            throw new $DOMException(prefix("createMediaElementSource", "AudioContext") + e.message.replace("Failed to construct 'MediaElementAudioSourceNode': ", ""), e.name);
          }
          throw e;
        }
      },
    },
    [AudioContext.prototype],
  );


  // ---- ScriptProcessorNode (Web Audio 1.1 section 1.29) ----
  // Its buffers fill on the rendering thread and come here, each an
  // `audioprocess` event; what the page leaves in the output buffer goes
  // back, played two buffers later, as the reference browser has it. An
  // offline rendering waits for each answer, so one is always given.

  const audioScripts = new $Map();
  let audioScriptProcess;

  class AudioProcessingEvent extends Event {
    #time; #input; #output;

    constructor(type, eventInitDict) {
      needToConstruct(arguments.length, 2, "AudioProcessingEvent");
      const words = "Failed to construct 'AudioProcessingEvent': ";
      const init = eventInitDict;
      if (init === null || (typeof init !== "object" && typeof init !== "function")) {
        throw new $TypeError(words + "The provided value is not of type 'AudioProcessingEventInit'.");
      }
      const names = ["inputBuffer", "outputBuffer", "playbackTime"];
      for (let i = 0; i < names.length; i++) {
        if (init[names[i]] === undefined) {
          throw new $TypeError(words + "Failed to read the '" + names[i] + "' property from 'AudioProcessingEventInit': Required member is undefined.");
        }
      }
      const input = init.inputBuffer;
      const output = init.outputBuffer;
      if ((input !== null && !(input instanceof AudioBuffer)) || (output !== null && !(output instanceof AudioBuffer))) {
        throw new $TypeError(words + "Failed to read the '" + (input instanceof AudioBuffer ? "outputBuffer" : "inputBuffer") + "' property from 'AudioProcessingEventInit': Failed to convert value to 'AudioBuffer'.");
      }
      const time = audioFloat(init.playbackTime, words, true);
      super(type, init);
      this.#time = time;
      this.#input = input;
      this.#output = output;
    }

    get playbackTime() { return this.#time; }
    get inputBuffer() { return this.#input; }
    get outputBuffer() { return this.#output; }
  }
  expose(AudioProcessingEvent, "AudioProcessingEvent");

  class ScriptProcessorNode extends AudioNode {
    #size; #input; #output;

    constructor(key, ctx, info, size, inputs, outputs) {
      if (key !== KEY) throw illegalConstructor("ScriptProcessorNode");
      info.count = inputs;
      super(KEY, ctx, info, "script");
      this.#size = size;
      const rate = ctx.sampleRate;
      // One input buffer and one output buffer, given to every event, as
      // the reference browser gives them; none of no channels.
      this.#input = inputs === 0 ? null : new AudioBuffer({ numberOfChannels: inputs, length: size, sampleRate: rate });
      this.#output = outputs === 0 ? null : new AudioBuffer({ numberOfChannels: outputs, length: size, sampleRate: rate });
      MapSet(audioScripts, audioContextId(ctx) + ":" + info.node, this);
    }

    static {
      // Buffer `told.seq` filled, its samples `bytes`: the event, then
      // the output back.
      audioScriptProcess = function* (node, told, bytes) {
        const input = node.#input;
        const output = node.#output;
        if (input !== null && bytes !== null) {
          const n = MathMin(told.channels, input.numberOfChannels);
          for (let c = 0; c < n; c++) {
            audioBufferFill(input, c, new $Float32Array(bytes, c * told.length * 4, told.length));
          }
        }
        try {
          const event = new AudioProcessingEvent("audioprocess", {
            bubbles: true, playbackTime: told.time, inputBuffer: input, outputBuffer: output,
          });
          eventState(event).trusted = true;
          yield* dispatching(node, event);
        } finally {
          const payload = output === null ? null : audioBufferPayload(output, true);
          audioCall({
            op: "scriptOutput", ctx: told.ctx, node: told.node, seq: told.seq,
            channels: payload === null ? 0 : payload.op.channels,
          }, payload === null ? new $Uint8Array(0) : payload.bytes, "audioprocess", "ScriptProcessorNode");
        }
      };
    }

    get bufferSize() { return this.#size; }
  }
  expose(ScriptProcessorNode, "ScriptProcessorNode");
  defineHandlers(ScriptProcessorNode.prototype, ["audioprocess"], false);

  include(
    {
      createScriptProcessor(bufferSize, numberOfInputChannels, numberOfOutputChannels) {
        let size = bufferSize === undefined ? 0 : unsigned(bufferSize);
        const inputs = numberOfInputChannels === undefined ? 2 : unsigned(numberOfInputChannels);
        const outputs = numberOfOutputChannels === undefined ? 2 : unsigned(numberOfOutputChannels);
        const fail = (message) => domError("IndexSizeError", message, "createScriptProcessor", "BaseAudioContext");
        if (inputs === 0 && outputs === 0) {
          throw fail("number of input channels and output channels cannot both be zero.");
        }
        if (inputs > 32) throw fail("number of input channels (" + inputs + ") exceeds maximum (32).");
        if (outputs > 32) throw fail("number of output channels (" + outputs + ") exceeds maximum (32).");
        if (size === 0) {
          // The reference's choice: four times its output's callback,
          // a power of two; a quantum's offline.
          size = this instanceof OfflineAudioContext ? 256 : 2048;
        } else if (size < 256 || size > 16384 || (size & (size - 1)) !== 0) {
          throw fail("buffer size (" + size + ") must be 0 or a power of two between 256 and 16384.");
        }
        const info = audioMake(this, "script", { size, inputs, outputs }, "ScriptProcessorNode");
        return new ScriptProcessorNode(KEY, this, info, size, inputs, outputs);
      },
    },
    [BaseAudioContext.prototype],
  );

  // ---- MediaStream and MediaStreamTrack (Media Capture and Streams) ----
  // As far as Web Audio makes and takes them: a
  // MediaStreamAudioDestinationNode's track, which a
  // MediaStreamAudioSourceNode reads; no camera, microphone or video. A
  // track is the engine's by number.

  const HEX = "0123456789abcdef";

  // A version 4 UUID, as the reference names streams and tracks.
  function audioUuid() {
    let text = "";
    for (let i = 0; i < 32; i++) {
      let digit = MathFloor(MathRandom() * 16);
      if (i === 12) digit = 4;
      if (i === 16) digit = 8 + (digit & 3);
      if (i === 8 || i === 12 || i === 16 || i === 20) text += "-";
      text += HEX[digit];
    }
    return text;
  }

  let trackNumber;
  let trackId;
  let trackLive;
  let streamAudioTracks;

  class MediaStreamTrack extends EventTarget {
    #id = audioUuid(); #kind; #label; #number; #enabled = true; #state = "live";
    #device = "WebAudio-" + audioUuid(); #rate;

    constructor(key, kind, label, number, rate) {
      if (key !== KEY) throw illegalConstructor("MediaStreamTrack");
      super();
      this.#kind = kind; this.#label = label; this.#number = number; this.#rate = rate;
    }

    static {
      trackNumber = (track) => track.#number;
      trackId = (track) => track.#id;
      trackLive = (track) => track.#state === "live";
    }

    get kind() { return this.#kind; }
    get id() { return this.#id; }
    get label() { return this.#label; }
    get enabled() { return this.#enabled; }
    set enabled(v) { this.#enabled = !!v; }
    get muted() { return false; }
    get readyState() { return this.#state; }
    get contentHint() { return ""; }

    getSettings() {
      return { channelCount: 2, deviceId: this.#device, latency: 0.01, sampleRate: this.#rate, sampleSize: 16 };
    }
    getCapabilities() { return {}; }
    getConstraints() { return {}; }

    // Ends the track: whatever reads it hears silence from now.
    stop() {
      if (this.#state === "ended") return;
      this.#state = "ended";
      if (this.#number === 0) return;
      audioCall({ op: "stopTrack", track: this.#number }, null, "stop", "MediaStreamTrack");
    }

    clone() {
      const copy = this.#copy();
      copy.#state = this.#state;
      copy.#enabled = this.#enabled;
      return copy;
    }

    #copy() {
      if (this instanceof CanvasCaptureMediaStreamTrack) return canvasTrackCopy(this);
      return new MediaStreamTrack(KEY, this.#kind, this.#label, this.#number, this.#rate);
    }
  }
  expose(MediaStreamTrack, "MediaStreamTrack");
  defineHandlers(MediaStreamTrack.prototype, ["ended", "mute", "unmute"], false);

  // ---- A canvas's capture (Media Capture from DOM Elements) -------------
  // As far as the reference browser's: a stream of one live video track,
  // its label a random 64 bytes in base64, as its settings' device; no
  // frames are made, and no audio node takes it.

  const BASE64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

  function captureDevice() {
    let text = "";
    for (let i = 0; i < 86; i++) text += BASE64[MathFloor(MathRandom() * 64)];
    return text + "==";
  }

  let canvasTrackCopy;

  class CanvasCaptureMediaStreamTrack extends MediaStreamTrack {
    #canvas; #settings;

    constructor(key, canvas, label, settings) {
      if (key !== KEY) throw illegalConstructor("CanvasCaptureMediaStreamTrack");
      super(KEY, "video", label, 0, 0);
      this.#canvas = canvas;
      this.#settings = settings;
    }

    static {
      canvasTrackCopy = (track) =>
        new CanvasCaptureMediaStreamTrack(KEY, track.#canvas, track.label, track.#settings);
    }

    get canvas() { return this.#canvas; }

    requestFrame() {}

    getSettings() {
      const s = this.#settings;
      return {
        aspectRatio: s.width / s.height, deviceId: s.deviceId, frameRate: s.frameRate,
        height: s.height, resizeMode: "none", width: s.width,
      };
    }
  }
  expose(CanvasCaptureMediaStreamTrack, "CanvasCaptureMediaStreamTrack");

  // HTMLCanvasElement's captureStream(frameRate), the canvas `this`.
  function captureStream(frameRate) {
    const where = prefix("captureStream", "HTMLCanvasElement");
    let rate = 60;
    if (arguments.length > 0 && frameRate !== undefined) {
      rate = +frameRate;
      if (!$NumberIsFinite(rate)) {
        throw new $TypeError(where + "The provided double value is non-finite.");
      }
      if (rate < 0) {
        throw new $DOMException(where + "Given frame rate is not supported.", "NotSupportedError");
      }
    }
    const label = captureDevice();
    const settings = {
      deviceId: label, frameRate: rate, width: this.width, height: this.height,
    };
    const track = new CanvasCaptureMediaStreamTrack(KEY, this, label, settings);
    return new MediaStream([track]);
  }

  class MediaStream extends EventTarget {
    #id = audioUuid(); #tracks = [];

    constructor(streamOrTracks) {
      super();
      if (arguments.length === 0 || streamOrTracks === undefined) return;
      let tracks;
      if (streamOrTracks instanceof MediaStream) {
        tracks = streamOrTracks.#tracks;
      } else if ($ArrayIsArray(streamOrTracks)) {
        tracks = streamOrTracks;
      } else {
        throw new $TypeError("Failed to construct 'MediaStream': The provided value cannot be converted to a sequence.");
      }
      for (let i = 0; i < tracks.length; i++) {
        if (!(tracks[i] instanceof MediaStreamTrack)) {
          throw new $TypeError("Failed to construct 'MediaStream': Failed to convert value to 'MediaStreamTrack'.");
        }
      }
      for (let i = 0; i < tracks.length; i++) this.#add(tracks[i]);
    }

    #add(track) {
      for (let i = 0; i < this.#tracks.length; i++) {
        if (this.#tracks[i] === track) return;
      }
      ArrayPush(this.#tracks, track);
    }

    get id() { return this.#id; }

    static {
      streamAudioTracks = (stream) => stream.#of("audio");
    }

    get active() {
      for (let i = 0; i < this.#tracks.length; i++) {
        if (trackLive(this.#tracks[i])) return true;
      }
      return false;
    }

    #of(kind) {
      const out = [];
      for (let i = 0; i < this.#tracks.length; i++) {
        if (kind === null || this.#tracks[i].kind === kind) ArrayPush(out, this.#tracks[i]);
      }
      return out;
    }

    getTracks() { return this.#of(null); }
    getAudioTracks() { return this.#of("audio"); }
    getVideoTracks() { return this.#of("video"); }

    getTrackById(wanted) {
      need(arguments.length, 1, "getTrackById", "MediaStream");
      const id = str(wanted);
      for (let i = 0; i < this.#tracks.length; i++) {
        if (trackId(this.#tracks[i]) === id) return this.#tracks[i];
      }
      return null;
    }

    addTrack(track) {
      need(arguments.length, 1, "addTrack", "MediaStream");
      if (!(track instanceof MediaStreamTrack)) {
        throw new $TypeError(prefix("addTrack", "MediaStream") + "parameter 1 is not of type 'MediaStreamTrack'.");
      }
      this.#add(track);
    }

    removeTrack(track) {
      need(arguments.length, 1, "removeTrack", "MediaStream");
      if (!(track instanceof MediaStreamTrack)) {
        throw new $TypeError(prefix("removeTrack", "MediaStream") + "parameter 1 is not of type 'MediaStreamTrack'.");
      }
      const kept = [];
      for (let i = 0; i < this.#tracks.length; i++) {
        if (this.#tracks[i] !== track) ArrayPush(kept, this.#tracks[i]);
      }
      this.#tracks = kept;
    }

    clone() {
      const copy = new MediaStream();
      for (let i = 0; i < this.#tracks.length; i++) copy.#add(this.#tracks[i].clone());
      return copy;
    }
  }
  expose(MediaStream, "MediaStream");
  defineHandlers(MediaStream.prototype, ["addtrack", "removetrack", "active", "inactive"], false);

  // A realtime context's node, or a TypeError in the reference's words.
  function audioRealtime(context, iface) {
    if (!(context instanceof AudioContext)) {
      throw new $TypeError("Failed to construct '" + iface + "': parameter 1 is not of type 'AudioContext'.");
    }
  }

  // A graph's sound as a stream's track (Web Audio 1.1 section 1.24).
  class MediaStreamAudioDestinationNode extends AudioNode {
    #stream;

    constructor(context, options) {
      needToConstruct(arguments.length, 1, "MediaStreamAudioDestinationNode");
      audioRealtime(context, "MediaStreamAudioDestinationNode");
      if (options !== undefined && options !== null && typeof options !== "object" && typeof options !== "function") {
        throw new $TypeError("Failed to construct 'MediaStreamAudioDestinationNode': The provided value is not of type 'AudioNodeOptions'.");
      }
      const info = audioMake(context, "streamDestination", undefined, "MediaStreamAudioDestinationNode");
      super(KEY, context, info, "streamDestination");
      audioNodeOptions(this, options);
      this.#stream = new MediaStream([
        new MediaStreamTrack(KEY, "audio", "MediaStreamAudioDestinationNode", info.track, context.sampleRate),
      ]);
    }

    get stream() { return this.#stream; }
  }
  expose(MediaStreamAudioDestinationNode, "MediaStreamAudioDestinationNode");

  // A stream's first audio track by id, taken into a graph (Web Audio
  // 1.1 section 1.23).
  class MediaStreamAudioSourceNode extends AudioNode {
    #stream;

    constructor(context, options) {
      needToConstruct(arguments.length, 2, "MediaStreamAudioSourceNode");
      audioRealtime(context, "MediaStreamAudioSourceNode");
      const words = "Failed to construct 'MediaStreamAudioSourceNode': ";
      const stream = options === null || typeof options !== "object" ? undefined : options.mediaStream;
      if (stream === undefined) {
        throw new $TypeError(words + "Failed to read the 'mediaStream' property from 'MediaStreamAudioSourceOptions': Required member is undefined.");
      }
      if (!(stream instanceof MediaStream)) {
        throw new $TypeError(words + "Failed to read the 'mediaStream' property from 'MediaStreamAudioSourceOptions': Failed to convert value to 'MediaStream'.");
      }
      const tracks = streamAudioTracks(stream);
      if (tracks.length === 0) {
        throw new $DOMException(words + "MediaStream has no audio track", "InvalidStateError");
      }
      let first = tracks[0];
      for (let i = 1; i < tracks.length; i++) {
        if (trackId(tracks[i]) < trackId(first)) first = tracks[i];
      }
      const track = trackLive(first) ? trackNumber(first) : 0;
      const info = audioMake(context, "streamSource", { track }, "MediaStreamAudioSourceNode");
      super(KEY, context, info, "streamSource");
      this.#stream = stream;
    }

    get mediaStream() { return this.#stream; }
  }
  expose(MediaStreamAudioSourceNode, "MediaStreamAudioSourceNode");

  // Rethrows a constructor's DOMException as the factory's.
  function audioFactory(make, method) {
    try {
      return make();
    } catch (e) {
      if (e instanceof $DOMException) {
        const words = "Failed to construct 'MediaStreamAudioSourceNode': ";
        throw new $DOMException(prefix(method, "AudioContext") + StringSlice(e.message, words.length), e.name);
      }
      throw e;
    }
  }

  include(
    {
      createMediaStreamDestination() {
        return new MediaStreamAudioDestinationNode(this);
      },
      createMediaStreamSource(mediaStream) {
        need(arguments.length, 1, "createMediaStreamSource", "AudioContext");
        if (!(mediaStream instanceof MediaStream)) {
          throw new $TypeError(prefix("createMediaStreamSource", "AudioContext") + "parameter 1 is not of type 'MediaStream'.");
        }
        return audioFactory(() => new MediaStreamAudioSourceNode(this, { mediaStream }), "createMediaStreamSource");
      },
    },
    [AudioContext.prototype],
  );

  let offlineRendered;
  let offlineSuspended;

  class OfflineAudioContext extends BaseAudioContext {
    #length; #rendering = null; #suspends = new $Map();

    constructor(numberOfChannelsOrOptions, length, sampleRate) {
      needToConstruct(arguments.length, 1, "OfflineAudioContext");
      const words = "Failed to construct 'OfflineAudioContext': ";
      let channels, frames, rate;
      if (typeof numberOfChannelsOrOptions === "object" && numberOfChannelsOrOptions !== null) {
        const o = numberOfChannelsOrOptions;
        audioRequired(o, "OfflineAudioContextOptions", words);
        channels = o.numberOfChannels === undefined ? 1 : unsigned(o.numberOfChannels);
        frames = unsigned(o.length);
        rate = audioFloat(o.sampleRate, words, false);
      } else {
        needToConstruct(arguments.length, 3, "OfflineAudioContext");
        channels = unsigned(numberOfChannelsOrOptions);
        frames = unsigned(length);
        rate = audioFloat(sampleRate, words, false);
      }
      if (channels < 1 || channels > 32) {
        throw new $DOMException(words + "The number of channels provided (" + channels + ") is outside the range [1, 32].", "NotSupportedError");
      }
      if (frames < 1) {
        throw new $DOMException(words + "The number of frames provided (" + frames + ") is less than the minimum bound (1).", "NotSupportedError");
      }
      if (!(rate >= 3000 && rate <= 768000)) {
        throw new $DOMException(words + "The sampleRate provided (" + rate + ") is outside the range [3000, 768000].", "NotSupportedError");
      }
      const info = audioCall({ op: "newContext", offline: true, channels, length: frames, rate }, null, words, null).result;
      super(KEY, info, channels);
      this.#length = frames;
    }

    static {
      // The rendering stopped at `frame`: the promise of its suspend().
      offlineSuspended = (ctx, frame) => {
        const settle = MapGet(ctx.#suspends, frame);
        MapDelete(ctx.#suspends, frame);
        return settle;
      };
      offlineRendered = (ctx) => {
        const settle = ctx.#rendering;
        ctx.#rendering = null;
        return settle;
      };
    }

    get length() { return this.#length; }

    startRendering() {
      if (this.#rendering !== null || this.state !== "suspended") {
        return rejected(domError("InvalidStateError", "cannot call startRendering more than once", "startRendering", "OfflineAudioContext"));
      }
      try {
        audioCall({ op: "render", ctx: audioContextId(this) }, null, "startRendering", "OfflineAudioContext");
      } catch (e) {
        return rejected(e);
      }
      return new $Promise((resolve, reject) => { this.#rendering = { resolve, reject }; });
    }

    suspend(suspendTime) {
      try {
        need(arguments.length, 1, "suspend", "OfflineAudioContext");
        const t = audioTime(suspendTime, "suspend time", prefix("suspend", "OfflineAudioContext"));
        const frame = audioCall({ op: "suspend", ctx: audioContextId(this), time: t }, null, "suspend", "OfflineAudioContext").result;
        return new $Promise((resolve, reject) => MapSet(this.#suspends, frame, { resolve, reject }));
      } catch (e) {
        return rejected(e);
      }
    }

    resume() {
      if (this.state === "closed") {
        return rejected(domError("InvalidStateError", "cannot resume a closed offline context", "resume", "OfflineAudioContext"));
      }
      return audioAskState(this, "resume", "running", "resume", "OfflineAudioContext");
    }
  }
  expose(OfflineAudioContext, "OfflineAudioContext");
  defineHandlers(OfflineAudioContext.prototype, ["complete"], false);

  class OfflineAudioCompletionEvent extends Event {
    #buffer;

    constructor(type, eventInitDict) {
      needToConstruct(arguments.length, 2, "OfflineAudioCompletionEvent");
      if (eventInitDict === null || typeof eventInitDict !== "object" || !(eventInitDict.renderedBuffer instanceof AudioBuffer)) {
        throw new $TypeError("Failed to construct 'OfflineAudioCompletionEvent': Failed to read the 'renderedBuffer' property from 'OfflineAudioCompletionEventInit': Required member is undefined.");
      }
      super(type, eventInitDict);
      this.#buffer = eventInitDict.renderedBuffer;
    }

    get renderedBuffer() { return this.#buffer; }
  }
  expose(OfflineAudioCompletionEvent, "OfflineAudioCompletionEvent");

  function* audioStateEvent(ctx, state) {
    const changed = audioContextState(ctx, state);
    audioContextWait(ctx, state);
    if (changed) yield* audioStateChanged(ctx);
  }

  function* audioStateChanged(ctx) {
    const event = new Event("statechange");
    eventState(event).trusted = true;
    yield* dispatching(ctx, event);
  }

  // What the engine tells the document's Web Audio (`Task::Audio`).
  function* audioTask(text, bytes) {
    const told = JSONParse(text);
    const ctx = MapGet(audioContexts, told.ctx);
    switch (told.type) {
      case "ended": {
        const key = told.ctx + ":" + told.node;
        const node = MapGet(audioSources, key);
        if (node === undefined) return false;
        MapDelete(audioSources, key);
        const event = new Event("ended");
        eventState(event).trusted = true;
        yield* dispatching(node, event);
        return false;
      }
      case "state": {
        if (ctx !== undefined) yield* audioStateEvent(ctx, told.state);
        return false;
      }
      case "process": {
        const node = MapGet(audioScripts, told.ctx + ":" + told.node);
        if (node !== undefined) {
          yield* audioScriptProcess(node, told, bytes);
        } else if (ctx !== undefined) {
          audioCall({ op: "scriptOutput", ctx: told.ctx, node: told.node, seq: told.seq, channels: 0 }, new $Uint8Array(0), "audioprocess", "ScriptProcessorNode");
        }
        return false;
      }
      case "decoded":
      case "decodeError": {
        const settle = MapGet(audioPromises, told.promise);
        if (settle === undefined) return false;
        MapDelete(audioPromises, told.promise);
        if (told.type === "decoded") {
          const buffer = audioBufferFromEngine(told.id, told.rate, told.channels, told.length, bytes);
          if (settle.success !== null) settle.success(buffer);
          settle.resolve(buffer);
        } else {
          const error = new $DOMException("Unable to decode audio data", "EncodingError");
          if (settle.error !== null) settle.error(error);
          settle.reject(error);
        }
        return false;
      }
      case "suspended": {
        if (ctx === undefined) return false;
        const settle = offlineSuspended(ctx, told.frame);
        if (settle !== undefined) settle.resolve(undefined);
        yield* audioStateEvent(ctx, "suspended");
        return false;
      }
      case "rendered": {
        if (ctx === undefined) return false;
        const settle = offlineRendered(ctx);
        const buffer = audioBufferFromEngine(told.id, told.rate, told.channels, told.length, bytes);
        // Closed, then `complete` and the rendering's promise, and only
        // then `statechange`, as the reference browser orders them.
        const changed = audioContextState(ctx, "closed");
        audioContextWait(ctx, "closed");
        const complete = new OfflineAudioCompletionEvent("complete", { renderedBuffer: buffer });
        eventState(complete).trusted = true;
        yield* dispatching(ctx, complete);
        if (settle !== null) settle.resolve(buffer);
        if (changed) yield* audioStateChanged(ctx);
        return false;
      }
    }
    return false;
  }

  return { audioTask, captureStream };
})
