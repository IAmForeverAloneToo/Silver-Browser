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
    $DOMException, $FinalizationRegistry, $Map, $Number, $Promise, $TypeError, ArrayPush,
    ArraySort, CharCodeAt, Document, EVENT_INIT, Element, Event, EventTarget, JSONParse,
    JSONStringify, KEY, MapDelete, MapGet, MapSet, ObjectCreate, PromiseResolve, PromiseThen,
    ReflectApply, ReflectGetOwnPropertyDescriptor, ReflectOwnKeys, Register, StringSlice,
    ToUpperCase, defineHandlers, defineProperty, dispatching, eventArgs, eventState, expose,
    handleOf, illegalConstructor, isNode, iteratorSymbol, kindOf, members, native, need,
    setStyleProperty, str, wrap,
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
      return ask(op(["op", "timeline.time", "timeline", this.#index]));
    }

    get duration() {
      if (!(#index in this)) throw new $TypeError("Illegal invocation");
      return null;
    }
  }
  expose(AnimationTimeline, "AnimationTimeline");

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

  // The document's timeline.
  const documentTimeline = new DocumentTimeline(KEY);

  function timelineFor(index) {
    if (index === null || index === undefined) return null;
    const known = MapGet(timelineObjects, index);
    return known === undefined ? null : known;
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
      return state(this).startTime;
    }

    set startTime(value) {
      const id = animationId(this);
      const time = finiteOrNull(value, "", "=startTime", "Animation");
      ask(op(["op", "animation.startTime", "id", id, "time", time]), "=startTime", "Animation");
    }

    get currentTime() {
      return state(this).currentTime;
    }

    set currentTime(value) {
      const id = animationId(this);
      const time = finiteOrNull(value, "", "=currentTime", "Animation");
      ask(op(["op", "animation.currentTime", "id", id, "time", time]), "=currentTime", "Animation");
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
      this.#currentTime = finiteOrNull(init.currentTime, "currentTime", "constructor", "AnimationPlaybackEvent");
      this.#timelineTime = finiteOrNull(init.timelineTime, "timelineTime", "constructor", "AnimationPlaybackEvent");
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
      currentTime: e.currentTime,
      timelineTime: e.timelineTime,
    });
    eventState(event).trusted = true;
    return yield* dispatching(target, event);
  }

  return { animationsTask, animationFor };
})
