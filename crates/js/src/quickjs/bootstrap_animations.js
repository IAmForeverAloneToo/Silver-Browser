// The Web Animations API (Web Animations 1 and 2, CSS Animations 2, CSS
// Transitions 2; D75): a binding chunk (D64), compiled apart from the
// bootstrap and read into a realm only when a page first uses it. A
// function of what of the bootstrap's it uses, as the bootstrap names
// it, giving what the bootstrap calls of it. The bootstrap's rules hold
// here: what is kept of the built-ins was kept before any page ran,
// nothing looked up on a built-in's prototype when a page calls in, no
// iterator but a page's own keyframes, which Web IDL iterates.
(function (env) {
  "use strict";
  const {
    $DOMException, $FinalizationRegistry, $Map, $Number, $Promise, $TypeError, ArrayIndexOf,
    ArrayJoin, ArrayPush, ArraySort, CSSNamespace, CharCodeAt, Document, EVENT_INIT, Element,
    Event, EventTarget, JSONParse, JSONStringify, KEY, MapDelete, MapGet, MapSet, ObjectCreate,
    PromiseResolve, PromiseThen, ReflectApply, ReflectGetOwnPropertyDescriptor, ReflectOwnKeys,
    Register, StringIndexOf, StringSlice, StringTrim, ToLowerCase, ToUpperCase, defineHandlers,
    defineProperty, dispatching, documentElement, eventArgs, eventState, expose, handleOf,
    illegalConstructor, isNode, iteratorSymbol, kindOf, members, native, need, setStyleProperty,
    str, wrap,
  } = env;

  // ---- The store (D75) ------------------------------------------------------

  // The animations and effects are the engine's, in one store with CSS's:
  // each object here is a view of a number there, every call crossing as
  // JSON through `native.animation`. What a call owes the promises a
  // script holds comes back with its answer, as notices, and is carried
  // out at once: a new promise, one settled, the finish notification
  // steps now or in a microtask (Web Animations 1 section 4.4).

  // `op` asked of the store: its result, the notices it owes carried
  // out first, or the exception it fails with, worded after `method` on
  // `iface`.
  function ask(op, method, iface) {
    const answer = JSONParse(native.animation(JSONStringify(op)));
    if (answer.notices !== undefined) owe(answer.notices);
    if (answer.error !== undefined) {
      const error = answer.error;
      const words = method === undefined ? "" : where(method, iface);
      if (error.name === "TypeError") throw new $TypeError(words + error.message);
      throw new $DOMException(words + error.message, error.name);
    }
    return answer.result;
  }

  function where(method, iface) {
    if (method[0] === "=") {
      return "Failed to set the '" + StringSlice(method, 1) + "' property on '" + iface + "': ";
    }
    return "Failed to execute '" + method + "' on '" + iface + "': ";
  }

  // An op: an object of no prototype, so that nothing a page puts on
  // Object.prototype is written with it.
  function op(pairs) {
    const o = ObjectCreate(null);
    for (let i = 0; i < pairs.length; i += 2) o[pairs[i]] = pairs[i + 1];
    return o;
  }

  // A number as it crosses: one JSON cannot hold (an infinity, NaN) as
  // the string JavaScript writes it, which the store reads back.
  function wired(n) {
    return typeof n === "number" && (n !== n || n === Infinity || n === -Infinity) ? "" + n : n;
  }

  // A number as the store gives it back: such a string read again.
  function unwired(v) {
    if (v === "Infinity") return Infinity;
    if (v === "-Infinity") return -Infinity;
    if (v === "NaN") return NaN;
    return v;
  }

  // ---- Promises ----

  // A promise an animation holds: made when a script first reads it, and
  // settled as its state says until then.
  function slot(state) {
    return { __proto__: null, promise: null, resolve: null, reject: null, state };
  }

  function abortError() {
    return new $DOMException("The user aborted a request.", "AbortError");
  }

  // `s`'s promise, made now if it was not, settled with `value`.
  function promiseOf(s, value) {
    if (s.promise === null) {
      s.promise = new $Promise((resolve, reject) => {
        s.resolve = resolve;
        s.reject = reject;
      });
      if (s.state === "resolved") s.resolve(value);
      if (s.state === "rejected") {
        PromiseThen(s.promise, undefined, () => {});
        s.reject(abortError());
      }
    }
    return s.promise;
  }

  function resolveSlot(s, value) {
    if (s.state !== "pending") return;
    s.state = "resolved";
    if (s.resolve !== null) s.resolve(value);
  }

  // Rejected with an AbortError, marked handled as the reference marks
  // its own.
  function rejectSlot(s) {
    if (s.state !== "pending") return;
    s.state = "rejected";
    if (s.reject !== null) {
      PromiseThen(s.promise, undefined, () => {});
      s.reject(abortError());
    }
  }

  // Each animation object's promises, by its number.
  const promised = new $Map();

  function promisesOf(id) {
    let p = MapGet(promised, id);
    if (p === undefined) {
      const state = ask(op(["op", "animation.state", "id", id]));
      p = {
        __proto__: null,
        ready: slot(state !== null && state.readyPending ? "pending" : "resolved"),
        finished: slot(state !== null && state.finishedResolved ? "resolved" : "pending"),
        queued: false,
      };
      MapSet(promised, id, p);
    }
    return p;
  }

  // Carries out the notices the store gave: [number, notice] pairs.
  function owe(notices) {
    for (let i = 0; i < notices.length; i++) {
      const id = notices[i][0];
      const notice = notices[i][1];
      const object = MapGet(animationObjects, id);
      if (object === undefined) continue;
      const p = promisesOf(id);
      switch (notice) {
        case "readyNew":
          if (p.ready.state !== "pending") p.ready = slot("pending");
          break;
        case "readyResolved":
          resolveSlot(p.ready, object);
          break;
        case "readyRejected":
          rejectSlot(p.ready);
          p.ready = slot("resolved");
          break;
        case "finishedNew":
          if (p.finished.state !== "pending") p.finished = slot("pending");
          break;
        case "finishedRejected":
          rejectSlot(p.finished);
          p.finished = slot("pending");
          break;
        case "notifySync":
          notify(id);
          break;
        case "notify":
          if (!p.queued) {
            p.queued = true;
            PromiseThen(PromiseResolve(undefined), () => {
              p.queued = false;
              notify(id);
            });
          }
          break;
      }
    }
  }

  // The finish notification steps (section 4.4.15): the finished promise
  // resolved and the `finish` event queued, if the animation is finished
  // still.
  function notify(id) {
    if (ask(op(["op", "animation.notify", "id", id])) === true) {
      const object = MapGet(animationObjects, id);
      if (object !== undefined) resolveSlot(promisesOf(id).finished, object);
    }
  }

  // ---- Objects ----

  // Each animation's and effect's object, while the script may reach it.
  const animationObjects = new $Map();
  const effectObjects = new $Map();
  const released = new $FinalizationRegistry((held) => {
    if (held.animation) {
      MapDelete(promised, held.id);
      native.animation(JSONStringify(op(["op", "animation.release", "id", held.id])));
    } else {
      native.animation(JSONStringify(op(["op", "effect.release", "effect", held.id])));
    }
  });

  // The number of `value`, an Animation, or Illegal invocation.
  let animationId;
  let effectId;
  let timelineIndex;

  // The object of animation `id`, made if it has none: a CSSAnimation, a
  // CSSTransition or an Animation as its kind says.
  function animationFor(id) {
    if (id === null || id === undefined) return null;
    const known = MapGet(animationObjects, id);
    if (known !== undefined) return known;
    const state = ask(op(["op", "animation.hold", "id", id]));
    if (state === null) return null;
    const object =
      state.kind === "css"
        ? new CSSAnimation(KEY, id)
        : state.kind === "transition"
          ? new CSSTransition(KEY, id)
          : new Animation(KEY, id);
    return object;
  }

  // The object of effect `id`, made if it has none.
  function effectFor(id) {
    if (id === null || id === undefined) return null;
    const known = MapGet(effectObjects, id);
    if (known !== undefined) return known;
    ask(op(["op", "effect.hold", "effect", id]));
    return new KeyframeEffect(KEY, id);
  }

  // ---- Web IDL's conversions ----

  // A `double`: a TypeError for a value not finite.
  function finite(value, member, method, iface) {
    const n = $Number(value);
    if (n !== n || n === Infinity || n === -Infinity) {
      throw new $TypeError(
        where(method, iface) + "The provided double value" +
          (member === "" ? "" : " for '" + member + "'") + " is non-finite.",
      );
    }
    return n;
  }

  // A `double?`.
  function finiteOrNull(value, member, method, iface) {
    return value === null || value === undefined ? null : finite(value, member, method, iface);
  }

  // An enum's value, or the TypeError its conversion throws.
  function enumValue(value, values, name, method, iface) {
    const s = str(value);
    for (let i = 0; i < values.length; i++) {
      if (values[i] === s) return s;
    }
    throw new $TypeError(
      where(method, iface) + "The provided value '" + s + "' is not a valid enum value of type " +
        name + ".",
    );
  }

  const FILLS = ["none", "forwards", "backwards", "both", "auto"];
  const DIRECTIONS = ["normal", "reverse", "alternate", "alternate-reverse"];
  const COMPOSITES = ["replace", "add", "accumulate"];
  const COMPOSITES_OR_AUTO = ["replace", "add", "accumulate", "auto"];

  // `(unrestricted double or DOMString)`'s duration: a number, or the
  // string, which must be `auto`.
  function durationOf(value, method, iface) {
    if (typeof value === "number") return value;
    const s = str(value);
    if (s !== "auto") {
      throw new $TypeError(where(method, iface) + "Invalid duration '" + s + "'.");
    }
    return "auto";
  }

  // An EffectTiming, or an OptionalEffectTiming when `optional`, read
  // from `value` as Web IDL reads a dictionary: its members in order,
  // each converted, a member left out absent (or its default).
  function effectTiming(value, optional, method, iface) {
    if (value !== undefined && value !== null && typeof value !== "object" && typeof value !== "function") {
      throw new $TypeError(where(method, iface) + "The provided value is not of type 'EffectTiming'.");
    }
    const given = value === undefined || value === null ? ObjectCreate(null) : value;
    const t = ObjectCreate(null);
    let v = given.delay;
    if (v !== undefined) t.delay = finite(v, "delay", method, iface);
    else if (!optional) t.delay = 0;
    v = given.direction;
    if (v !== undefined) t.direction = enumValue(v, DIRECTIONS, "PlaybackDirection", method, iface);
    else if (!optional) t.direction = "normal";
    v = given.duration;
    if (v !== undefined) t.duration = durationOf(v, method, iface);
    else if (!optional) t.duration = "auto";
    v = given.easing;
    if (v !== undefined) t.easing = str(v);
    else if (!optional) t.easing = "linear";
    v = given.endDelay;
    if (v !== undefined) t.endDelay = finite(v, "endDelay", method, iface);
    else if (!optional) t.endDelay = 0;
    v = given.fill;
    if (v !== undefined) t.fill = enumValue(v, FILLS, "FillMode", method, iface);
    else if (!optional) t.fill = "auto";
    v = given.iterationStart;
    if (v !== undefined) t.iterationStart = finite(v, "iterationStart", method, iface);
    else if (!optional) t.iterationStart = 0;
    v = given.iterations;
    if (v !== undefined) t.iterations = wired($Number(v));
    else if (!optional) t.iterations = 1;
    if (typeof t.duration === "number") t.duration = wired(t.duration);
    return { timing: t, given };
  }

  // A KeyframeEffectOptions, or a number that is its duration: the
  // timing, the composite operation and the pseudo-element.
  function effectOptions(options, method, iface) {
    if (typeof options === "number") {
      const t = effectTiming(undefined, false, method, iface).timing;
      t.duration = wired(options);
      return { timing: t, composite: "replace", pseudo: null };
    }
    const read = effectTiming(options, false, method, iface);
    const given = read.given;
    const composite = given.composite === undefined
      ? "replace"
      : enumValue(given.composite, COMPOSITES, "CompositeOperation", method, iface);
    const pseudo = given.pseudoElement === undefined || given.pseudoElement === null
      ? null
      : str(given.pseudoElement);
    return { timing: read.timing, composite, pseudo };
  }

  // ---- Keyframes (section 6.6, "process a keyframes argument") ----

  // The properties a keyframe may animate, by the names a keyframe
  // object gives them, and their CSS names: the store's, asked once.
  let animatable = null;

  // A property's CSS name as a keyframe object's member names it
  // (section 6.6.1): `cssFloat` is `float`, `cssOffset` is `offset`, a
  // camel-cased name hyphenated, a custom property as it is.
  function idlName(css) {
    if (css === "float") return "cssFloat";
    if (css === "offset") return "cssOffset";
    let out = "";
    let i = 0;
    if (StringSlice(css, 0, 8) === "-webkit-") {
      out = "webkit";
      i = 8;
      out += ToUpperCase(css[i]);
      i++;
    }
    for (; i < css.length; i++) {
      if (css[i] === "-" && i + 1 < css.length) {
        out += ToUpperCase(css[i + 1]);
        i++;
      } else {
        out += css[i];
      }
    }
    return out;
  }

  function animatableNames() {
    if (animatable === null) {
      animatable = ObjectCreate(null);
      const names = ask(op(["op", "properties"]));
      for (let i = 0; i < names.length; i++) {
        animatable[idlName(names[i])] = names[i];
      }
    }
    return animatable;
  }

  // Whether `name` is a custom property's name.
  function isCustom(name) {
    return name.length > 2 && name[0] === "-" && name[1] === "-";
  }

  // The animated properties of keyframe-like object `input`, by member
  // name, in code point order (section 6.6.1, step 5).
  function animationProperties(input) {
    const names = animatableNames();
    const keys = ReflectOwnKeys(input);
    const found = [];
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      if (typeof key !== "string") continue;
      const desc = ReflectGetOwnPropertyDescriptor(input, key);
      if (desc === undefined || !desc.enumerable) continue;
      if (names[key] !== undefined || isCustom(key)) ArrayPush(found, key);
    }
    ArraySort(found, (a, b) => {
      const n = a.length < b.length ? a.length : b.length;
      for (let i = 0; i < n; i++) {
        const d = CharCodeAt(a, i) - CharCodeAt(b, i);
        if (d !== 0) return d;
      }
      return a.length - b.length;
    });
    return found;
  }

  // A keyframe's offset (Web Animations 2's `(double or DOMString)?`): a
  // number, or a string the store reads as a `<number>`, `calc()` among
  // them.
  function offsetOf(value, method, iface) {
    if (value === null || value === undefined) return null;
    if (typeof value === "string") {
      return ask(op(["op", "offset", "text", value]), method, iface);
    }
    return finite(value, "offset", method, iface);
  }

  // Whether `value` is iterable: an object with a method at
  // Symbol.iterator, which is then returned.
  function iteratorMethod(value) {
    if (value === null || (typeof value !== "object" && typeof value !== "function")) {
      return undefined;
    }
    const method = value[iteratorSymbol];
    if (method === undefined || method === null) return undefined;
    if (typeof method !== "function") {
      throw new $TypeError("Failed to construct 'KeyframeEffect': The object must have a callable @@iterator property.");
    }
    return method;
  }

  // The values of an iterable, as a list.
  function listOf(value, method) {
    const iterator = ReflectApplyCall(method, value);
    const next = iterator.next;
    const out = [];
    for (;;) {
      const step = ReflectApplyCall(next, iterator);
      if (step === null || typeof step !== "object") {
        throw new $TypeError("Iterator result is not an object.");
      }
      if (step.done) break;
      ArrayPush(out, step.value);
    }
    return out;
  }

  function ReflectApplyCall(f, self) {
    return ReflectApply(f, self, []);
  }

  // A keyframe-like object (section 6.6.1): its offset, easing and
  // composite as a BaseKeyframe or, with `lists`, as a
  // BasePropertyIndexedKeyframe has them, and its properties' values,
  // each a string or, with `lists`, a list of strings.
  function keyframeLike(input, lists, method, iface) {
    if (input !== undefined && input !== null && typeof input !== "object" && typeof input !== "function") {
      throw new $TypeError(where(method, iface) + "Keyframes must be objects.");
    }
    const given = input === undefined || input === null ? ObjectCreate(null) : input;
    const k = { __proto__: null, values: [] };
    // The members in Web IDL's order: composite, easing, offset.
    let v = given.composite;
    if (lists) {
      if (v === undefined) k.composite = [];
      else if (typeof v === "object" && v !== null && iteratorMethod(v) !== undefined) {
        const list = listOf(v, iteratorMethod(v));
        k.composite = [];
        for (let i = 0; i < list.length; i++) {
          ArrayPush(k.composite, enumValue(list[i], COMPOSITES_OR_AUTO, "CompositeOperationOrAuto", method, iface));
        }
      } else {
        k.composite = [enumValue(v, COMPOSITES_OR_AUTO, "CompositeOperationOrAuto", method, iface)];
      }
    } else {
      k.composite = v === undefined ? "auto" : enumValue(v, COMPOSITES_OR_AUTO, "CompositeOperationOrAuto", method, iface);
    }
    v = given.easing;
    if (lists) {
      if (v === undefined) k.easing = [];
      else if (typeof v === "object" && v !== null && iteratorMethod(v) !== undefined) {
        const list = listOf(v, iteratorMethod(v));
        k.easing = [];
        for (let i = 0; i < list.length; i++) ArrayPush(k.easing, str(list[i]));
      } else {
        k.easing = [str(v)];
      }
    } else {
      k.easing = v === undefined ? "linear" : str(v);
    }
    v = given.offset;
    if (lists) {
      if (v === undefined) k.offset = [];
      else if (v === null) k.offset = [null];
      else if (typeof v === "object" && iteratorMethod(v) !== undefined) {
        const list = listOf(v, iteratorMethod(v));
        k.offset = [];
        for (let i = 0; i < list.length; i++) {
          ArrayPush(k.offset, offsetOf(list[i], method, iface));
        }
      } else {
        k.offset = [offsetOf(v, method, iface)];
      }
    } else {
      k.offset = offsetOf(v, method, iface);
    }
    const names = animatableNames();
    const properties = animationProperties(given);
    for (let i = 0; i < properties.length; i++) {
      const key = properties[i];
      const raw = given[key];
      const css = isCustom(key) ? key : names[key];
      let values;
      if (lists && raw !== null && typeof raw === "object" && iteratorMethod(raw) !== undefined) {
        const list = listOf(raw, iteratorMethod(raw));
        values = [];
        for (let j = 0; j < list.length; j++) ArrayPush(values, str(list[j]));
      } else {
        values = lists ? [str(raw)] : str(raw);
      }
      ArrayPush(k.values, [css, values]);
    }
    return k;
  }

  // The keyframes of `object`, as the store takes them: each an offset
  // or null, an easing, a composite operation and its properties'
  // values, as names and values.
  function processKeyframes(object, method, iface) {
    if (object === null || object === undefined) return [];
    if (typeof object !== "object" && typeof object !== "function") {
      throw new $TypeError(where(method, iface) + "The provided value is not of type 'object'.");
    }
    const iterate = iteratorMethod(object);
    if (iterate !== undefined) {
      const items = listOf(object, iterate);
      const out = [];
      for (let i = 0; i < items.length; i++) {
        const item = items[i];
        if (item !== undefined && item !== null && typeof item !== "object" && typeof item !== "function") {
          throw new $TypeError(where(method, iface) + "Keyframes must be objects.");
        }
        const k = keyframeLike(item, false, method, iface);
        ArrayPush(out, {
          offset: k.offset,
          easing: k.easing,
          composite: k.composite,
          values: k.values,
        });
      }
      return out;
    }
    // A property-indexed keyframe (section 6.6.1, step 7).
    const k = keyframeLike(object, true, method, iface);
    const frames = [];
    for (let i = 0; i < k.values.length; i++) {
      const name = k.values[i][0];
      const values = k.values[i][1];
      const n = values.length;
      for (let j = 0; j < n; j++) {
        const offset = n === 1 ? 1 : j / (n - 1);
        let frame = null;
        for (let f = 0; f < frames.length; f++) {
          if (frames[f].computed === offset) frame = frames[f];
        }
        if (frame === null) {
          frame = { computed: offset, values: [] };
          ArrayPush(frames, frame);
        }
        ArrayPush(frame.values, [name, values[j]]);
      }
    }
    ArraySort(frames, (a, b) => a.computed - b.computed);
    const out = [];
    for (let i = 0; i < frames.length; i++) {
      ArrayPush(out, {
        offset: i < k.offset.length ? k.offset[i] : null,
        easing: "linear",
        composite: "auto",
        values: frames[i].values,
      });
    }
    // Offsets for keyframes there are, beyond them unused but checked.
    if (k.offset.length > out.length) {
      for (let i = out.length; i < k.offset.length; i++) {
        const o = k.offset[i];
        if (o !== null && (o < 0 || o > 1)) {
          throw new $TypeError(where(method, iface) + "Offsets must be null or in the range [0,1].");
        }
      }
    }
    const easings = k.easing.length === 0 ? ["linear"] : k.easing;
    for (let i = 0; i < out.length; i++) out[i].easing = easings[i % easings.length];
    // Every easing given is checked, those no keyframe takes among them.
    const unused = [];
    for (let i = out.length; i < easings.length; i++) ArrayPush(unused, easings[i]);
    if (k.composite.length > 0) {
      for (let i = 0; i < out.length; i++) out[i].composite = k.composite[i % k.composite.length];
    }
    if (unused.length > 0) {
      ask(op(["op", "easings", "easings", unused]), method, iface);
    }
    return out;
  }

  // ---- Timelines (section 6.2 and 6.3) ----

  const timelineObjects = new $Map();

  class AnimationTimeline {
    #index = 0;

    constructor(key, index) {
      if (key !== KEY) throw illegalConstructor("AnimationTimeline");
      this.#index = index;
      MapSet(timelineObjects, index, this);
    }

    static {
      timelineIndex = (value) => {
        if (value === null || value === undefined) return null;
        if (typeof value !== "object" || !(#index in value)) return undefined;
        return value.#index;
      };
    }

    get currentTime() {
      if (!(#index in this)) throw new $TypeError("Illegal invocation");
      const time = ask(op(["op", "timeline.time", "timeline", this.#index]));
      return this.#index !== 0 && timelineInfo(this.#index).kind !== "document"
        ? percentValue(time)
        : time;
    }

    // A progress-based timeline's is 100% (Web Animations 2 section
    // 4.2.2.1), a document timeline's none.
    get duration() {
      if (!(#index in this)) throw new $TypeError("Illegal invocation");
      return this.#index !== 0 && timelineInfo(this.#index).kind !== "document"
        ? new CSSUnitValue(100, "percent")
        : null;
    }
  }
  expose(AnimationTimeline, "AnimationTimeline");

  // What timeline `index` is, as the store says.
  function timelineInfo(index) {
    const info = ask(op(["op", "timeline.info", "timeline", index]));
    return info === null ? { __proto__: null, kind: "none" } : info;
  }

  class DocumentTimeline extends AnimationTimeline {
    constructor(options = undefined) {
      if (options === KEY) {
        super(KEY, 0);
        return;
      }
      if (options !== undefined && options !== null && typeof options !== "object" && typeof options !== "function") {
        throw new $TypeError("Failed to construct 'DocumentTimeline': The provided value is not of type 'DocumentTimelineOptions'.");
      }
      const given = options === undefined || options === null ? undefined : options.originTime;
      const origin = given === undefined ? 0 : finite(given, "originTime", "constructor", "DocumentTimeline");
      super(KEY, ask(op(["op", "timeline.create", "origin", origin])));
    }
  }
  expose(DocumentTimeline, "DocumentTimeline");


  // ---- Scroll-driven Animations 1's timelines (sections 2.2 and 3.2) ----

  const AXES = ["block", "inline", "x", "y"];

  // A ScrollTimelineOptions' or a ViewTimelineOptions' axis.
  function axisOf(options, iface) {
    const axis = options === null || options === undefined ? undefined : options.axis;
    return axis === undefined ? "block" : enumValue(axis, AXES, "ScrollAxis", "constructor", iface);
  }

  class ScrollTimeline extends AnimationTimeline {
    constructor(options = undefined, index = undefined) {
      if (options === KEY) {
        super(KEY, index);
        return;
      }
      if (options !== undefined && options !== null && typeof options !== "object" && typeof options !== "function") {
        throw new $TypeError("Failed to construct 'ScrollTimeline': The provided value is not of type 'ScrollTimelineOptions'.");
      }
      const given = options === undefined || options === null ? ObjectCreate(null) : options;
      let source;
      if (given.source === undefined) {
        // The document's scrolling element, as at the time it is made.
        source = scrollingHandle();
      } else if (given.source === null) {
        source = null;
      } else {
        if (!isNode(given.source) || kindOf(given.source) !== 1) {
          throw new $TypeError("Failed to construct 'ScrollTimeline': Failed to read the 'source' property from 'ScrollTimelineOptions': parameter 1 is not of type 'Element'.");
        }
        source = handleOf(given.source);
      }
      const axis = axisOf(given, "ScrollTimeline");
      super(KEY, ask(op(["op", "timeline.scroll", "source", source, "axis", axis]),
        "constructor", "ScrollTimeline"));
    }

    get source() {
      const index = timelineIndex(this);
      if (index === undefined || index === null) throw new $TypeError("Illegal invocation");
      return wrap(timelineInfo(index).source);
    }

    get axis() {
      const index = timelineIndex(this);
      if (index === undefined || index === null) throw new $TypeError("Illegal invocation");
      return timelineInfo(index).axis;
    }
  }
  expose(ScrollTimeline, "ScrollTimeline");

  // The document's scrolling element's handle, `null` for none.
  function scrollingHandle() {
    const root = documentElement(native.document());
    return root === undefined ? null : root;
  }

  class ViewTimeline extends ScrollTimeline {
    constructor(options = undefined, index = undefined) {
      if (options === KEY) {
        super(KEY, index);
        return;
      }
      if (options !== undefined && options !== null && typeof options !== "object" && typeof options !== "function") {
        throw new $TypeError("Failed to construct 'ViewTimeline': The provided value is not of type 'ViewTimelineOptions'.");
      }
      const given = options === undefined || options === null ? ObjectCreate(null) : options;
      let subject = null;
      if (given.subject !== undefined && given.subject !== null) {
        if (!isNode(given.subject) || kindOf(given.subject) !== 1) {
          throw new $TypeError("Failed to construct 'ViewTimeline': Failed to read the 'subject' property from 'ViewTimelineOptions': parameter 1 is not of type 'Element'.");
        }
        subject = handleOf(given.subject);
      }
      const axis = axisOf(given, "ViewTimeline");
      let inset = null;
      const v = given.inset;
      if (v !== undefined) {
        if (typeof v === "string") {
          inset = v;
        } else if (v !== null && typeof v === "object" && iteratorMethod(v) !== undefined) {
          const sides = listOf(v, iteratorMethod(v));
          if (sides.length < 1 || sides.length > 2) {
            throw new $TypeError("Failed to construct 'ViewTimeline': Invalid inset");
          }
          inset = ArrayJoin(mapped(sides, (side) => {
            if (side === "auto") return "auto";
            const unit = unitOf(side);
            const text = numericText(side);
            if (text === null || (unit !== null && unit[1] !== "px" && unit[1] !== "percent")) {
              throw new $TypeError("Failed to construct 'ViewTimeline': Invalid inset");
            }
            return text;
          }), " ");
        } else {
          inset = str(v);
        }
      }
      super(KEY, ask(op(["op", "timeline.view", "subject", subject, "axis", axis, "inset", inset]),
        "constructor", "ViewTimeline"));
    }

    get subject() {
      const index = timelineIndex(this);
      if (index === undefined || index === null) throw new $TypeError("Illegal invocation");
      return wrap(timelineInfo(index).subject);
    }

    // Where the subject's range starts and ends, as scroll offsets.
    get startOffset() {
      const index = timelineIndex(this);
      if (index === undefined || index === null) throw new $TypeError("Illegal invocation");
      const start = timelineInfo(index).start;
      return start === null ? null : new CSSUnitValue(start, "px");
    }

    get endOffset() {
      const index = timelineIndex(this);
      if (index === undefined || index === null) throw new $TypeError("Illegal invocation");
      const end = timelineInfo(index).end;
      return end === null ? null : new CSSUnitValue(end, "px");
    }
  }
  expose(ViewTimeline, "ViewTimeline");

  // The document's timeline.
  const documentTimeline = new DocumentTimeline(KEY);

  // Timeline `index`'s object: made now for one a style made (`scroll()`,
  // `view()`, a name), of the kind it is; none for a name that names no
  // timeline.
  function timelineFor(index) {
    if (index === null || index === undefined) return null;
    const known = MapGet(timelineObjects, index);
    if (known !== undefined) return known;
    const info = timelineInfo(index);
    if (info.kind !== "scroll" && info.kind !== "view") return null;
    // Styles that name the same timeline share its object; an anonymous
    // one is its element's own.
    if (!info.named) {
      const own = info.kind === "scroll" ? new ScrollTimeline(KEY, index) : new ViewTimeline(KEY, index);
      MapSet(timelineObjects, index, own);
      return own;
    }
    const same = info.kind + " " + info.source + " " + info.subject + " " + info.axis;
    let made = MapGet(styleTimelines, same);
    if (made === undefined) {
      made = info.kind === "scroll" ? new ScrollTimeline(KEY, index) : new ViewTimeline(KEY, index);
      MapSet(styleTimelines, same, made);
    }
    MapSet(timelineObjects, index, made);
    return made;
  }

  // The timeline objects of the timelines styles made, by what they
  // follow.
  const styleTimelines = new $Map();


  // ---- CSS numeric values (CSS Typed OM 1, as the times and offsets of
  // scroll-driven animations need them; D75) ----
  // What Web Animations 2 and Scroll-driven Animations 1 give and take
  // as a CSSNumberish: a number with its unit, or a sum of them, with
  // `CSS.number()`, `CSS.percent()`, `CSS.px()` and the other units'
  // factories, and `CSSNumericValue.parse()` of one value.

  // The units the factories make, by their factories' names.
  const UNITS = [
    "number", "percent", "em", "rem", "ex", "ch", "lh", "vw", "vh", "vmin", "vmax", "cm",
    "mm", "q", "in", "pt", "pc", "px", "deg", "grad", "rad", "turn", "s", "ms", "hz",
    "khz", "dpi", "dpcm", "dppx", "fr",
  ];

  // The value and unit of a CSSUnitValue, and the values of a
  // CSSMathSum; `null` for another value. Set by their classes.
  let unitOf = null;
  let sumOf = null;

  class CSSStyleValue {
    constructor(key) {
      if (key !== KEY) throw illegalConstructor("CSSStyleValue");
    }
  }
  expose(CSSStyleValue, "CSSStyleValue");

  class CSSNumericValue extends CSSStyleValue {
    constructor(key) {
      if (key !== KEY) throw illegalConstructor("CSSNumericValue");
      super(KEY);
    }

    // One number, percentage or dimension (section 4.2.1's parse, of the
    // values these need).
    static parse(cssText) {
      const read = numericParts(StringTrim(str(cssText)));
      if (read === null) {
        throw new $DOMException(
          "Failed to execute 'parse' on 'CSSNumericValue': Invalid math expression",
          "SyntaxError",
        );
      }
      return new CSSUnitValue(read[0], read[1]);
    }
  }
  expose(CSSNumericValue, "CSSNumericValue");

  // A number, a percentage or a dimension as text: its value and unit,
  // `null` for anything else.
  function numericParts(text) {
    let i = 0;
    const at = (k) => (k < text.length ? CharCodeAt(text, k) : -1);
    const digit = (c) => c >= 48 && c <= 57;
    if (at(i) === 43 || at(i) === 45) i++;
    const digitsFrom = i;
    while (digit(at(i))) i++;
    if (at(i) === 46) {
      i++;
      while (digit(at(i))) i++;
    }
    if (i === digitsFrom || (i === digitsFrom + 1 && at(digitsFrom) === 46)) return null;
    if ((at(i) === 101 || at(i) === 69) && (digit(at(i + 1)) || ((at(i + 1) === 43 || at(i + 1) === 45) && digit(at(i + 2))))) {
      i += 2;
      while (digit(at(i))) i++;
    }
    const value = $Number(StringSlice(text, 0, i));
    const rest = StringSlice(text, i);
    if (rest === "") return [value, "number"];
    if (rest === "%") return [value, "percent"];
    const unit = ToLowerCase(rest);
    return ArrayIndexOf(UNITS, unit) < 0 ? null : [value, unit];
  }

  // Each of `list` made by `f`, as a new list.
  function mapped(list, f) {
    const out = [];
    for (let i = 0; i < list.length; i++) ArrayPush(out, f(list[i]));
    return out;
  }

  class CSSUnitValue extends CSSNumericValue {
    #value = 0;
    #unit = "number";

    constructor(value, unit) {
      super(KEY);
      need(arguments.length, 2, "constructor", "CSSUnitValue");
      this.#value = finite(value, "", "constructor", "CSSUnitValue");
      const u = ToLowerCase(str(unit));
      if (ArrayIndexOf(UNITS, u) < 0) {
        throw new $TypeError(
          "Failed to construct 'CSSUnitValue': Invalid unit: " + str(unit),
        );
      }
      this.#unit = u;
    }

    static {
      unitOf = (value) =>
        value !== null && typeof value === "object" && #unit in value ? [value.#value, value.#unit] : null;
    }

    get value() {
      if (!(#value in this)) throw new $TypeError("Illegal invocation");
      return this.#value;
    }

    set value(v) {
      if (!(#value in this)) throw new $TypeError("Illegal invocation");
      this.#value = finite(v, "", "=value", "CSSUnitValue");
    }

    get unit() {
      if (!(#value in this)) throw new $TypeError("Illegal invocation");
      return this.#unit;
    }

    toString() {
      if (!(#value in this)) throw new $TypeError("Illegal invocation");
      return unitText(this.#value, this.#unit);
    }
  }
  expose(CSSUnitValue, "CSSUnitValue");

  // A unit value's text: `50%`, `3px`, a number alone.
  function unitText(value, unit) {
    if (unit === "number") return "" + value;
    if (unit === "percent") return value + "%";
    return value + unit;
  }


  class CSSMathValue extends CSSNumericValue {
    constructor(key) {
      if (key !== KEY) throw illegalConstructor("CSSMathValue");
      super(KEY);
    }
  }
  expose(CSSMathValue, "CSSMathValue");

  // A sum of numeric values, written `calc(a + b)`.
  class CSSMathSum extends CSSMathValue {
    #values = [];

    constructor(...values) {
      super(KEY);
      if (values.length === 0) {
        throw new $DOMException(
          "Failed to construct 'CSSMathSum': Must specify at least one argument.",
          "SyntaxError",
        );
      }
      for (let i = 0; i < values.length; i++) {
        const v = values[i];
        ArrayPush(this.#values, typeof v === "number" ? new CSSUnitValue(v, "number") : v);
      }
    }

    static {
      sumOf = (value) =>
        value !== null && typeof value === "object" && #values in value ? value.#values : null;
    }

    get values() {
      if (!(#values in this)) throw new $TypeError("Illegal invocation");
      return mapped(this.#values, (v) => v);
    }

    toString() {
      if (!(#values in this)) throw new $TypeError("Illegal invocation");
      return numericText(this);
    }
  }
  expose(CSSMathSum, "CSSMathSum");


  // A numeric value's text as CSS writes it, `null` for what is none.
  function numericText(value) {
    const unit = unitOf(value);
    if (unit !== null) return unitText(unit[0], unit[1]);
    const sum = sumOf(value);
    if (sum !== null) {
      return "calc(" + ArrayJoin(mapped(sum, (v) => numericText(v)), " + ") + ")";
    }
    return null;
  }

  // `CSS.number()`, `CSS.percent()`, `CSS.px()` and the rest (section
  // 5.3), on the CSS namespace.
  for (let i = 0; i < UNITS.length; i++) {
    const unit = UNITS[i];
    defineProperty(CSSNamespace, unit, {
      value: {
        [unit](value) {
          need(arguments.length, 1, unit, "CSS");
          return new CSSUnitValue(value, unit);
        },
      }[unit],
      writable: true,
      enumerable: true,
      configurable: true,
    });
  }

  // A percentage as a scroll-driven animation's time.
  function percentValue(n) {
    return n === null || n === undefined ? null : new CSSUnitValue(unwired(n), "percent");
  }

  // A CSSNumberish set as a time (Web Animations 2 section 6.4): a
  // percentage on a progress-based timeline, else milliseconds, a
  // number, or seconds or milliseconds of a unit value.
  function timeFrom(value, progress, method, iface) {
    if (value === null || value === undefined) return null;
    const unit = unitOf(value);
    if (unit === null) {
      if (progress) {
        throw new $TypeError(where(method, iface) + "The provided value is not a percentage.");
      }
      return finite(value, "", method, iface);
    }
    if (progress) {
      if (unit[1] !== "percent") {
        throw new $TypeError(where(method, iface) + "The provided value is not a percentage.");
      }
      return unit[0];
    }
    if (unit[1] === "ms") return unit[0];
    if (unit[1] === "s") return unit[0] * 1000;
    if (unit[1] === "number") return unit[0];
    throw new $TypeError(where(method, iface) + "The provided value is not a time.");
  }

  // A range's edge a script gives (`rangeStart`, `rangeEnd`, and
  // `animate()`'s): `normal`, a string as `animation-range-start` takes
  // one, a numeric value, or a TimelineRangeOffset; as text.
  function rangeText(value, method, iface) {
    if (typeof value === "string") return value;
    const numeric = numericText(value);
    if (numeric !== null) return numeric;
    if (value !== null && typeof value === "object") {
      const name = value.rangeName === undefined || value.rangeName === null
        ? ""
        : str(value.rangeName);
      let offset = "";
      if (value.offset !== undefined) {
        offset = numericText(value.offset);
        if (offset === null) {
          throw new $TypeError(where(method, iface) + "The offset is not a CSSNumericValue.");
        }
      }
      if (name === "" && offset === "") return "normal";
      if (name !== "" && offset === "") return name;
      return name === "" ? offset : name + " " + offset;
    }
    return str(value);
  }

  // A range's edge as `rangeStart` gives it: `normal`, or its name and
  // offset.
  function rangeObject(text, end) {
    if (text === "normal") return "normal";
    const space = StringIndexOf(text, " ");
    const first = space < 0 ? text : StringSlice(text, 0, space);
    if (ArrayIndexOf(RANGE_NAMES, first) < 0) {
      return { rangeName: "none", offset: parseOffset(text) };
    }
    const offset = space < 0
      ? new CSSUnitValue(end ? 100 : 0, "percent")
      : parseOffset(StringSlice(text, space + 1));
    return { rangeName: first, offset };
  }

  const RANGE_NAMES = ["cover", "contain", "entry", "exit", "entry-crossing", "exit-crossing"];

  // An offset's text as a numeric value: a unit value, or a sum of
  // them as `calc()` writes it.
  function parseOffset(text) {
    const read = numericParts(text);
    if (read !== null) return new CSSUnitValue(read[0], read[1]);
    if (StringSlice(text, 0, 5) !== "calc(" || StringSlice(text, text.length - 1) !== ")") {
      return new CSSUnitValue(0, "percent");
    }
    const inner = StringSlice(text, 5, text.length - 1);
    const parts = [];
    let from = 0;
    for (;;) {
      const plus = StringIndexOf(inner, " + ", from);
      const piece = numericParts(StringSlice(inner, from, plus < 0 ? inner.length : plus));
      if (piece === null) return new CSSUnitValue(0, "percent");
      ArrayPush(parts, new CSSUnitValue(piece[0], piece[1]));
      if (plus < 0) break;
      from = plus + 3;
    }
    return ReflectApply(sumMaker, null, parts);
  }

  function sumMaker(...parts) {
    return new CSSMathSum(...parts);
  }

  // ---- Effects (section 6.5 and 6.6) ----

  // The timing an effect's getTiming() gives, from the store's.
  function timingObject(t) {
    return {
      delay: t.delay,
      direction: t.direction,
      duration: t.duration === "auto" ? "auto" : unwired(t.duration),
      easing: t.easing,
      endDelay: t.endDelay,
      fill: t.fill,
      iterationStart: t.iterationStart,
      iterations: unwired(t.iterations),
    };
  }

  class AnimationEffect {
    #id = 0;

    constructor(key, id) {
      if (key !== KEY) throw illegalConstructor("AnimationEffect");
      this.#id = id;
    }

    static {
      effectId = (value) => {
        if (value === null || typeof value !== "object" || !(#id in value)) return undefined;
        return value.#id;
      };
    }

    getTiming() {
      const id = effectId(this);
      if (id === undefined) throw new $TypeError("Illegal invocation");
      return timingObject(ask(op(["op", "effect.timing", "effect", id])));
    }

    getComputedTiming() {
      const id = effectId(this);
      if (id === undefined) throw new $TypeError("Illegal invocation");
      const t = ask(op(["op", "effect.computed", "effect", id]));
      const out = timingObject(t);
      out.startTime = t.startTime;
      out.endTime = unwired(t.endTime);
      out.activeDuration = unwired(t.activeDuration);
      out.localTime = t.localTime;
      out.progress = t.progress;
      out.currentIteration = unwired(t.currentIteration);
      // On a progress-based timeline its times are percentages.
      if (t.progressBased) {
        const times = ["duration", "startTime", "endTime", "activeDuration", "localTime"];
        for (let i = 0; i < times.length; i++) {
          out[times[i]] = percentValue(out[times[i]]);
        }
      }
      return out;
    }

    updateTiming(timing = undefined) {
      const id = effectId(this);
      if (id === undefined) throw new $TypeError("Illegal invocation");
      const t = effectTiming(timing, true, "updateTiming", "AnimationEffect").timing;
      ask(op(["op", "effect.update", "effect", id, "timing", t]), "updateTiming", "AnimationEffect");
    }
  }
  expose(AnimationEffect, "AnimationEffect");

  // The target an effect is given: an Element, or null.
  function targetHandle(value, method, iface) {
    if (value === null || value === undefined) return null;
    if (!isNode(value) || kindOf(value) !== 1) {
      throw new $TypeError(where(method, iface) + "The provided value is not of type 'Element'.");
    }
    return handleOf(value);
  }

  class KeyframeEffect extends AnimationEffect {
    constructor(target, keyframes = undefined, options = undefined) {
      if (target === KEY) {
        super(KEY, keyframes);
        MapSet(effectObjects, keyframes, this);
        Register(released, this, { animation: false, id: keyframes });
        return;
      }
      need(arguments.length, 1, "constructor", "KeyframeEffect");
      let id;
      if (arguments.length === 1) {
        const source = effectId(target);
        if (source === undefined) {
          throw new $TypeError(
            "Failed to construct 'KeyframeEffect': parameter 1 is not of type 'KeyframeEffect'.",
          );
        }
        id = ask(op(["op", "effect.copy", "effect", source]), "constructor", "KeyframeEffect");
      } else {
        const handle = targetHandle(target, "constructor", "KeyframeEffect");
        const frames = processKeyframes(keyframes, "constructor", "KeyframeEffect");
        const o = effectOptions(options, "constructor", "KeyframeEffect");
        id = ask(op([
          "op", "effect.create", "target", handle, "pseudo", o.pseudo, "keyframes", frames,
          "timing", o.timing, "composite", o.composite,
        ]), "constructor", "KeyframeEffect");
      }
      super(KEY, id);
      MapSet(effectObjects, id, this);
      Register(released, this, { animation: false, id });
    }

    get target() {
      const id = effectId(this);
      if (id === undefined) throw new $TypeError("Illegal invocation");
      const t = ask(op(["op", "effect.target", "effect", id]));
      return t.target === null ? null : wrap(t.target);
    }

    set target(value) {
      const id = effectId(this);
      if (id === undefined) throw new $TypeError("Illegal invocation");
      const handle = targetHandle(value, "=target", "KeyframeEffect");
      const t = ask(op(["op", "effect.target", "effect", id]));
      ask(op(["op", "effect.setTarget", "effect", id, "target", handle, "pseudo", t.pseudo]));
    }

    get pseudoElement() {
      const id = effectId(this);
      if (id === undefined) throw new $TypeError("Illegal invocation");
      return ask(op(["op", "effect.target", "effect", id])).pseudo;
    }

    set pseudoElement(value) {
      const id = effectId(this);
      if (id === undefined) throw new $TypeError("Illegal invocation");
      const pseudo = value === null ? null : str(value);
      const t = ask(op(["op", "effect.target", "effect", id]));
      ask(op(["op", "effect.setTarget", "effect", id, "target", t.target, "pseudo", pseudo]),
        "=pseudoElement", "KeyframeEffect");
    }

    get composite() {
      const id = effectId(this);
      if (id === undefined) throw new $TypeError("Illegal invocation");
      return ask(op(["op", "effect.composite", "effect", id]));
    }

    set composite(value) {
      const id = effectId(this);
      if (id === undefined) throw new $TypeError("Illegal invocation");
      let s;
      try {
        s = enumValue(value, COMPOSITES, "CompositeOperation", "=composite", "KeyframeEffect");
      } catch (e) {
        // An attribute's invalid enum value is ignored (Web IDL 3.7.6).
        return;
      }
      ask(op(["op", "effect.setComposite", "effect", id, "composite", s]));
    }

    getKeyframes() {
      const id = effectId(this);
      if (id === undefined) throw new $TypeError("Illegal invocation");
      const frames = ask(op(["op", "effect.keyframes", "effect", id]));
      const out = [];
      for (let i = 0; i < frames.length; i++) {
        const f = frames[i];
        const k = {
          offset: f.offset,
          computedOffset: f.computedOffset,
          easing: f.easing,
          composite: f.composite,
        };
        for (let j = 0; j < f.values.length; j++) {
          const name = f.values[j][0];
          k[isCustom(name) ? name : idlName(name)] = f.values[j][1];
        }
        ArrayPush(out, k);
      }
      return out;
    }

    setKeyframes(keyframes) {
      const id = effectId(this);
      if (id === undefined) throw new $TypeError("Illegal invocation");
      need(arguments.length, 1, "setKeyframes", "KeyframeEffect");
      const frames = processKeyframes(keyframes, "setKeyframes", "KeyframeEffect");
      ask(op(["op", "effect.setKeyframes", "effect", id, "keyframes", frames]),
        "setKeyframes", "KeyframeEffect");
    }
  }
  expose(KeyframeEffect, "KeyframeEffect");

  // ---- Animations (section 6.4) ----

  class Animation extends EventTarget {
    #id = 0;
    #name = "";

    constructor(effect = undefined, timeline = undefined) {
      super();
      if (effect === KEY) {
        this.#id = timeline;
      } else {
        const e = effect === undefined || effect === null ? null : effectId(effect);
        if (e === undefined) {
          throw new $TypeError(
            "Failed to construct 'Animation': parameter 1 is not of type 'AnimationEffect'.",
          );
        }
        let t = 0;
        if (timeline !== undefined) {
          t = timelineIndex(timeline);
          if (t === undefined) {
            throw new $TypeError(
              "Failed to construct 'Animation': parameter 2 is not of type 'AnimationTimeline'.",
            );
          }
        }
        this.#id = ask(op(["op", "animation.create", "effect", e, "timeline", t]),
          "constructor", "Animation");
      }
      MapSet(animationObjects, this.#id, this);
      Register(released, this, { animation: true, id: this.#id });
    }

    static {
      animationId = (value) => {
        if (value === null || typeof value !== "object" || !(#id in value)) {
          throw new $TypeError("Illegal invocation");
        }
        return value.#id;
      };
    }

    get id() {
      animationId(this);
      return this.#name;
    }

    set id(value) {
      animationId(this);
      this.#name = str(value);
    }

    get effect() {
      return effectFor(state(this).effect);
    }

    set effect(value) {
      const id = animationId(this);
      const e = value === null || value === undefined ? null : effectId(value);
      if (e === undefined) {
        throw new $TypeError("Failed to set the 'effect' property on 'Animation': The provided value is not of type 'AnimationEffect'.");
      }
      ask(op(["op", "animation.effect", "id", id, "effect", e]));
    }

    get timeline() {
      return timelineFor(state(this).timeline);
    }

    set timeline(value) {
      const id = animationId(this);
      const t = timelineIndex(value);
      if (t === undefined) {
        throw new $TypeError("Failed to set the 'timeline' property on 'Animation': The provided value is not of type 'AnimationTimeline'.");
      }
      ask(op(["op", "animation.timeline", "id", id, "timeline", t]));
    }

    get startTime() {
      const s = state(this);
      return s.progress ? percentValue(s.startTime) : s.startTime;
    }

    set startTime(value) {
      const id = animationId(this);
      const time = timeFrom(value, state(this).progress, "=startTime", "Animation");
      ask(op(["op", "animation.startTime", "id", id, "time", time]), "=startTime", "Animation");
    }

    get currentTime() {
      const s = state(this);
      return s.progress ? percentValue(s.currentTime) : s.currentTime;
    }

    set currentTime(value) {
      const id = animationId(this);
      const time = timeFrom(value, state(this).progress, "=currentTime", "Animation");
      ask(op(["op", "animation.currentTime", "id", id, "time", time]), "=currentTime", "Animation");
    }

    // The range of its timeline it is attached to (Web Animations 2
    // section 6.4.1).
    get rangeStart() {
      return rangeObject(state(this).rangeStart, false);
    }

    set rangeStart(value) {
      const id = animationId(this);
      const text = rangeText(value, "=rangeStart", "Animation");
      ask(op(["op", "animation.range", "id", id, "start", text]), "=rangeStart", "Animation");
    }

    get rangeEnd() {
      return rangeObject(state(this).rangeEnd, true);
    }

    set rangeEnd(value) {
      const id = animationId(this);
      const text = rangeText(value, "=rangeEnd", "Animation");
      ask(op(["op", "animation.range", "id", id, "end", text]), "=rangeEnd", "Animation");
    }

    get playbackRate() {
      return state(this).playbackRate;
    }

    set playbackRate(value) {
      const id = animationId(this);
      const rate = finite(value, "", "=playbackRate", "Animation");
      ask(op(["op", "animation.rate", "id", id, "rate", rate]), "=playbackRate", "Animation");
    }

    get playState() {
      return state(this).playState;
    }

    get replaceState() {
      return state(this).replaceState;
    }

    get pending() {
      return state(this).pending;
    }

    get overallProgress() {
      return state(this).overallProgress;
    }

    // Each reads the store first, which takes what styles changed of a
    // CSS animation and gives what that owes the promises.
    get ready() {
      const id = animationId(this);
      state(this);
      return promiseOf(promisesOf(id).ready, this);
    }

    get finished() {
      const id = animationId(this);
      state(this);
      return promiseOf(promisesOf(id).finished, this);
    }

    play() {
      const id = animationId(this);
      promisesOf(id);
      ask(op(["op", "animation.play", "id", id]), "play", "Animation");
    }

    pause() {
      const id = animationId(this);
      promisesOf(id);
      ask(op(["op", "animation.pause", "id", id]), "pause", "Animation");
    }

    finish() {
      const id = animationId(this);
      promisesOf(id);
      ask(op(["op", "animation.finish", "id", id]), "finish", "Animation");
    }

    cancel() {
      const id = animationId(this);
      promisesOf(id);
      ask(op(["op", "animation.cancel", "id", id]), "cancel", "Animation");
    }

    reverse() {
      const id = animationId(this);
      promisesOf(id);
      ask(op(["op", "animation.reverse", "id", id]), "reverse", "Animation");
    }

    updatePlaybackRate(playbackRate) {
      const id = animationId(this);
      need(arguments.length, 1, "updatePlaybackRate", "Animation");
      const rate = finite(playbackRate, "", "updatePlaybackRate", "Animation");
      promisesOf(id);
      ask(op(["op", "animation.updateRate", "id", id, "rate", rate]), "updatePlaybackRate", "Animation");
    }

    persist() {
      const id = animationId(this);
      ask(op(["op", "animation.persist", "id", id]));
    }

    commitStyles() {
      const id = animationId(this);
      const made = ask(op(["op", "animation.commit", "id", id]), "commitStyles", "Animation");
      const element = wrap(made.target);
      for (let i = 0; i < made.values.length; i++) {
        setStyleProperty(element, made.values[i][0], made.values[i][1]);
      }
    }
  }
  defineHandlers(Animation.prototype, ["finish", "cancel", "remove"], false);
  expose(Animation, "Animation");

  // What the store says of animation `this`.
  function state(animation) {
    const id = animationId(animation);
    const s = ask(op(["op", "animation.state", "id", id]));
    if (s === null) {
      return {
        __proto__: null, effect: null, timeline: null, startTime: null, currentTime: null,
        playbackRate: 1, playState: "idle", replaceState: "active", pending: false,
        overallProgress: null, name: "",
      };
    }
    return s;
  }

  // CSS Animations 2 section 4.1.
  class CSSAnimation extends Animation {
    constructor(key, id) {
      if (key !== KEY) throw illegalConstructor("CSSAnimation");
      super(KEY, id);
    }

    get animationName() {
      return state(this).name;
    }
  }
  expose(CSSAnimation, "CSSAnimation");

  // CSS Transitions 2 section 6.1.
  class CSSTransition extends Animation {
    constructor(key, id) {
      if (key !== KEY) throw illegalConstructor("CSSTransition");
      super(KEY, id);
    }

    get transitionProperty() {
      return state(this).name;
    }
  }
  expose(CSSTransition, "CSSTransition");

  // ---- AnimationPlaybackEvent (section 6.8) ----

  const PLAYBACK_INIT = members(EVENT_INIT, [
    ["currentTime", "any", null],
    ["timelineTime", "any", null],
  ], "AnimationPlaybackEventInit");

  class AnimationPlaybackEvent extends Event {
    #currentTime = null;
    #timelineTime = null;

    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, PLAYBACK_INIT, "AnimationPlaybackEvent");
      super(args[0], args[1]);
      const init = args[1];
      // A CSSNumberish each (Web Animations 2 section 6.11): a numeric
      // value as it is, else a double.
      const numberish = (value, member) =>
        numericText(value) !== null
          ? value
          : finiteOrNull(value, member, "constructor", "AnimationPlaybackEvent");
      this.#currentTime = numberish(init.currentTime, "currentTime");
      this.#timelineTime = numberish(init.timelineTime, "timelineTime");
    }

    get currentTime() {
      if (!(#currentTime in this)) throw new $TypeError("Illegal invocation");
      return this.#currentTime;
    }

    get timelineTime() {
      if (!(#currentTime in this)) throw new $TypeError("Illegal invocation");
      return this.#timelineTime;
    }
  }
  expose(AnimationPlaybackEvent, "AnimationPlaybackEvent");

  // ---- Animatable (section 6.9) and DocumentOrShadowRoot (6.10) ----

  // `Element.animate()`.
  function animate(keyframes, options = undefined) {
    if (!isNode(this) || kindOf(this) !== 1) throw new $TypeError("Illegal invocation");
    need(arguments.length, 1, "animate", "Element");
    const frames = processKeyframes(keyframes, "animate", "Element");
    const o = effectOptions(options, "animate", "Element");
    let timeline = 0;
    let name = "";
    if (typeof options === "object" && options !== null) {
      if (options.id !== undefined) name = str(options.id);
      if (options.timeline !== undefined) {
        timeline = timelineIndex(options.timeline);
        if (timeline === undefined) {
          throw new $TypeError("Failed to execute 'animate' on 'Element': The provided value is not of type 'AnimationTimeline'.");
        }
      }
    }
    const made = ask(op([
      "op", "animate", "target", handleOf(this), "pseudo", o.pseudo, "keyframes", frames,
      "timing", o.timing, "composite", o.composite, "timeline", timeline,
    ]), "animate", "Element");
    const effect = new KeyframeEffect(KEY, made.effect);
    const animation = new Animation(KEY, made.animation);
    animation.id = name;
    // Its range, before it plays and aligns with it.
    if (typeof options === "object" && options !== null) {
      const start = options.rangeStart === undefined ? null : rangeText(options.rangeStart, "animate", "Element");
      const end = options.rangeEnd === undefined ? null : rangeText(options.rangeEnd, "animate", "Element");
      if (start !== null || end !== null) {
        ask(op(["op", "animation.range", "id", made.animation, "start", start, "end", end]),
          "animate", "Element");
      }
    }
    promisesOf(made.animation);
    ask(op(["op", "animation.play", "id", made.animation]), "animate", "Element");
    return animation;
  }

  // `Element.getAnimations()` and `Document.getAnimations()`, of a
  // pseudo-element of the element when `pseudo` names one.
  function animationsOf(target, subtree, pseudo) {
    const ids = ask(op([
      "op", "animations", "target", target, "subtree", subtree, "pseudo", pseudo,
    ]), "getAnimations", target === null ? "Document" : "Element");
    const out = [];
    for (let i = 0; i < ids.length; i++) {
      const object = animationFor(ids[i]);
      if (object !== null) ArrayPush(out, object);
    }
    return out;
  }

  defineProperty(Element.prototype, "animate", {
    value: { animate }.animate,
    writable: true,
    enumerable: true,
    configurable: true,
  });
  defineProperty(Element.prototype, "getAnimations", {
    value: {
      getAnimations(options = undefined) {
        if (!isNode(this) || kindOf(this) !== 1) throw new $TypeError("Illegal invocation");
        const given = options === undefined || options === null ? undefined : options;
        // GetAnimationsOptions' members in order: pseudoElement, subtree.
        const pseudo = given === undefined || given.pseudoElement === undefined ||
          given.pseudoElement === null ? null : str(given.pseudoElement);
        const subtree = given !== undefined && !!given.subtree;
        return animationsOf(handleOf(this), subtree, pseudo);
      },
    }.getAnimations,
    writable: true,
    enumerable: true,
    configurable: true,
  });
  defineProperty(Document.prototype, "getAnimations", {
    value: {
      getAnimations() {
        if (!isNode(this) || kindOf(this) !== 9) throw new $TypeError("Illegal invocation");
        return animationsOf(null, true, null);
      },
    }.getAnimations,
    writable: true,
    enumerable: true,
    configurable: true,
  });
  // `ShadowRoot.getAnimations()` (DocumentOrShadowRoot): the animations
  // of the elements of its tree (D68).
  defineProperty(ShadowRoot.prototype, "getAnimations", {
    value: {
      getAnimations() {
        if (!isNode(this) || kindOf(this) !== 11) throw new $TypeError("Illegal invocation");
        return animationsOf(handleOf(this), true, null);
      },
    }.getAnimations,
    writable: true,
    enumerable: true,
    configurable: true,
  });
  defineProperty(Document.prototype, "timeline", {
    get: ReflectGetOwnPropertyDescriptor({
      get timeline() {
        if (!isNode(this) || kindOf(this) !== 9) throw new $TypeError("Illegal invocation");
        return documentTimeline;
      },
    }, "timeline").get,
    enumerable: true,
    configurable: true,
  });

  // ---- The frame's update (HTML's "update animations and send events") ----

  // What the engine tells the animations at a frame: the notices the
  // pending tasks it ran owe, or a playback event to dispatch at an
  // animation's object.
  function* animationsTask(text) {
    const told = JSONParse(text);
    if (told.notices !== undefined) {
      owe(told.notices);
      return false;
    }
    const e = told.event;
    if (e === undefined) return false;
    const target = MapGet(animationObjects, e.id);
    if (target === undefined) return false;
    const event = new AnimationPlaybackEvent(e.type, {
      currentTime: e.progress ? percentValue(e.currentTime) : e.currentTime,
      timelineTime: e.progress ? percentValue(e.timelineTime) : e.timelineTime,
    });
    eventState(event).trusted = true;
    return yield* dispatching(target, event);
  }

  return { animationsTask, animationFor };
})
