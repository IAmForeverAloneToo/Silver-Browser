// The 2D canvas (HTML section 4.12.5, M30), and OffscreenCanvas and
// ImageBitmap (HTML 4.12.5.3, 8.10, M30.2): a binding chunk (D64),
// compiled apart from the bootstrap and read into a realm only when a page
// first uses them. A function of what of the bootstrap's it uses, as the
// bootstrap names it, giving what the bootstrap calls of it. The
// bootstrap's rules hold here: what is kept of the built-ins was kept
// before any page ran, nothing looked up on a built-in's prototype when a
// page calls in, no iterator.
(function (env) {
  "use strict";
  const {
    $DOMException, $Number, $RangeError, $TypeError, $Uint8Array, $Uint8ClampedArray, $WeakMap,
    ArrayPush, CTX, DOMMatrix, GLOBAL_INTERFACE, KEY, MapGet, MathTrunc, ReflectApply,
    StringIndexOf, StringSlice, ToBase64, ToLowerCase, TypedArrayBuffer, TypedArrayLength,
    TypedArrayOffset, TypedArrayTag, WORKER, WeakMapGet, WeakMapSet, WeakRefDeref, bindCanvasObject,
    blobBytes, canvasObjects, canvasTargets, chunkInstall, defineProperty, domError, expose, failed,
    fromList, getOwnPropertyDescriptor, getOwnPropertyNames, gpuIsCanvasContext, gpuWebglVersion,
    handleOf, illegalConstructor, illegalInvocation, imageDatas, include, isBlob, isNode,
    iteratorSymbol, kindOf, makeMatrix, matrixInit, native, need, needToConstruct, pngBlob, prefix,
    rejected, resolved, str, targetOf,
  } = env;

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
      if (id === "webgpu") {
        if (t.context !== null) {
          return gpuIsCanvasContext(t.context) ? t.context : null;
        }
        t.context = chunkInstall("gpu").gpuCanvasContext(this, -t.id);
        return t.context;
      }
      if (id === "webgl" || id === "webgl2") {
        const version = id === "webgl2" ? 2 : 1;
        if (t.context !== null) {
          return gpuWebglVersion(t.context) === version ? t.context : null;
        }
        if (WORKER) return null;
        t.context = chunkInstall("gpu").webglContext(this, -t.id, version, arguments[1]);
        return t.context;
      }
      if (id !== "2d") {
        return null;
      }
      if (t.context !== null) {
        return gpuIsCanvasContext(t.context) || gpuWebglVersion(t.context) !== 0 ? null : t.context;
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
      } else if (isNode(image) && kindOf(image) === 1 && native.tag(handleOf(image)) === "video") {
        // A video's picture now (M40).
        source = [5, handleOf(image)];
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

  // An element's 2D context, which its `getContext("2d")` makes.
  function elementContext(element, handle, options) {
    const { alpha, willReadFrequently } = contextSettings(options, "HTMLCanvasElement");
    native.canvas("context", 0, handle, alpha);
    const context = new CanvasRenderingContext2D(KEY);
    WeakMapSet(canvasTargets, context, {
      kind: 0,
      id: handle,
      iface: CTX,
      element,
      alpha,
      willReadFrequently,
    });
    return context;
  }

  return { ImageData, OffscreenCanvas, createImageBitmap, elementContext, makeBitmap };
})
