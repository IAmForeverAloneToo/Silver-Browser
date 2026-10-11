# Design note: Canvas 2D

Roadmap M30. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ends: where they
disagree, the code wins, and the places are marked *(corrected)*. What
ships is described in `docs/architecture.md` "Current state".

A page could not draw. `<canvas>` was an inline element whose
children, the fallback content meant for browsers without scripts,
were laid out and painted as ordinary content; `HTMLCanvasElement`
reflected `width` and `height` (300 and 150) and had no `getContext`.
Every chart library, every game loop and every image editor on the web
starts by asking a canvas for its 2D context, and here got a
`TypeError`.

This milestone gave the engine the 2D context of the HTML Standard:
paths, fills and strokes in colours, gradients and patterns, text
through the engine's own text stack, images drawn from `<img>`, other
canvases and bitmaps, transforms, clipping, compositing, shadows, the
canvas state's stack, and the pixels themselves, read and written as
`ImageData` and exported as PNG. It gives `<canvas>` its rendering as a
replaced element whose bitmap a frame shows, changed by scripts and
shown again at the next frame without the page being laid out again;
an `OffscreenCanvas` that draws without an element; and the
same-origin rule for pixels: a canvas that has drawn another origin's
image without its consent keeps its pixels from scripts. Its
acceptance is the roadmap's: a chart and a particle demo drawn
correctly, at 60 frames a second.

The standards are read at the versions current when this note was
written (11 October 2026):

- the HTML Standard (Living Standard of 9 October 2026): 4.12.5 (the
  `canvas` element, its 2D context, `ImageData`, `Path2D`,
  `OffscreenCanvas`, the pixel manipulation and serialization of
  bitmaps), 8.10 (`ImageBitmap` and `createImageBitmap()`), 15.4.1
  (the rendering of embedded content), 2.5.4 (CORS settings attributes)
  and 4.8.4.3 (the image's request, for `crossorigin`);
- Compositing and Blending Level 1 for `globalCompositeOperation`;
- CSS Fonts Level 4 for the `font` attribute's syntax and
  serialization; CSS Color Level 4 for colours;
- Geometry Interfaces Module Level 1 for `DOMMatrix`, which
  `getTransform()` returns and `setTransform()` and `Path2D.addPath()`
  take;
- the PNG Specification (third edition) for `toDataURL()`.

What the standards leave to the implementation, or state but are worth
checking, was put to the reference browser (Chrome 154.0.8037.99,
headless) before this note was written (scratchpad `m30/p1-canvas.html`,
and `m30/pages/p2-taint.html` served on two origins):

- a canvas is 300 by 150 and `display: inline`; `getContext("2d")`
  gives the same object each time, `getContext("webgl")` `null`; a
  second context type after the first is `null`;
- the state's defaults: `fillStyle` and `strokeStyle` `#000000`,
  `lineWidth` 1, `lineCap` `butt`, `lineJoin` `miter`, `miterLimit` 10,
  `font` `10px sans-serif`, `textAlign` `start`, `textBaseline`
  `alphabetic`, `direction` `ltr`, `globalAlpha` 1,
  `globalCompositeOperation` `source-over`, `shadowColor`
  `rgba(0, 0, 0, 0)`, `imageSmoothingEnabled` true,
  `imageSmoothingQuality` `low`, `filter` `none`, `letterSpacing` and
  `wordSpacing` `0px`;
- a colour reads back as `#rrggbb` when opaque and as
  `rgba(r, g, b, a)` otherwise (`rgba(255,0,0,0.5)` gives
  `rgba(255, 0, 0, 0.5)`, `hsl(120, 100%, 50%)` gives `#00ff00`); a
  value that does not parse, or is not a string, changes nothing;
  `currentcolor` is the canvas's `color`, black off the document;
- a number that is not finite or not positive, for `lineWidth`,
  `miterLimit` and the rest, changes nothing; `globalAlpha` outside 0 to
  1 changes nothing; an enumeration's unknown value changes nothing;
- `font` serializes in the shorthand's order with the size in pixels:
  `italic small-caps 300 16px/2 Georgia, serif` gives
  `italic 300 small-caps 16px Georgia, serif`, `12pt monospace`
  `16px monospace`, `1em sans-serif` (off the document) `10px
  sans-serif`, `50% serif` `5px serif`; one that does not parse
  changes nothing;
- the bitmap is premultiplied 8-bit RGBA: `putImageData` of
  `(255, 128, 7, 1)` reads back as `(255, 255, 0, 1)`, of
  `(200, 100, 50, 128)` as `(199, 100, 50, 128)`;
- edges are anti-aliased by coverage: a rectangle from 2.5 to 5.5 is
  half-covered at its edges (alpha 128); a one-pixel line on an integer
  row covers two rows by half; a circle's edge pixels are partial
  (alpha 249 and 35 at the probed points);
- composite operations give, for `rgba(0, 0, 255, 0.5)` over
  `rgba(255, 0, 0, 0.75)`: `source-over` `(109, 0, 146, 223)`, `copy`
  `(0, 0, 255, 128)` and nothing outside the drawn shape, `lighter`
  `(191, 0, 128, 255)`, `multiply` `(109, 0, 37, 223)`, `xor`
  `(191, 0, 64, 127)`, `source-in` `(0, 0, 255, 96)` and nothing
  outside, `destination-out` `(255, 0, 0, 95)`, `difference`
  `(218, 0, 146, 223)`; `clear` is taken too, and clears the shape
  alone, where `plus-lighter` and `plus-darker` are not *(corrected:
  found by the web-platform tests, probe `m30/p8-wpt.html`)*;
- a linear gradient from black to white over ten pixels gives 12, 140
  and 242 at pixels 0, 5 and 9 (interpolated in premultiplied sRGB, at
  pixel centres); a radial one between radii 2 and 8 gives
  `(105, 0, 150)` at 5 pixels from its centre;
- a shadow with an offset of 5 and no blur is the shape in the shadow's
  colour, under the shape; with a blur of 4 it falls off as a Gaussian
  of deviation 2 (alpha 186, 130, 50, 23 and 3 at 0, 3, 5, 10 and 12
  pixels across its edge);
- drawing an image scaled up interpolates bilinearly (32, 96, 159
  between a black and a white pixel scaled four times) and, with
  `imageSmoothingEnabled` false, by the nearest pixel;
- `getTransform()` is a `DOMMatrix` (`matrix(2, 0, 0, 2, 2, 3)` after
  `translate(2, 3)` and `scale(2, 2)`); a `restore()` with nothing saved
  does nothing; setting `width` clears the bitmap and the state, even to
  the same value;
- `setLineDash` with a negative or non-finite value changes nothing,
  with an odd count doubles the list; `getLineDash()` gives a copy;
- the errors and their words: `addColorStop(2, ...)` an
  `IndexSizeError` ("The provided value (2) is outside the range (0.0,
  1.0)."), a colour that does not parse a `SyntaxError`, a non-finite
  gradient coordinate a `TypeError` ("The provided double value is
  non-finite."), a negative radius an `IndexSizeError` ("The radius
  provided (-1) is negative."), `getImageData` of no width an
  `IndexSizeError` ("The source width is 0."), an `ImageData` whose
  data is not a multiple of 4 an `InvalidStateError`, `drawImage` of
  what is not an image a `TypeError`, of an image not yet loaded
  nothing at all, an unknown pattern repetition a `SyntaxError`;
- `toDataURL()` gives `data:image/png;base64,...`, and for a type it
  does not know, PNG; `toDataURL("image/jpeg")` gives JPEG there;
- a canvas that drew an image of another origin, fetched without CORS,
  refuses `getImageData` ("The canvas has been tainted by cross-origin
  data.") and `toDataURL` ("Tainted canvases may not be exported."),
  both `SecurityError`s; one fetched with `crossorigin=anonymous` and
  allowed by `Access-Control-Allow-Origin` does not; a canvas that drew
  a tainted canvas is tainted; setting `width` makes it clean again;
  `crossorigin` on an image whose server does not allow it fails the
  image's load;
- `CanvasRenderingContext2D.prototype` has 74 own properties; the
  interfaces `Path2D`, `TextMetrics`, `ImageData`, `OffscreenCanvas`,
  `OffscreenCanvasRenderingContext2D`, `ImageBitmap` and `DOMMatrix`
  are all there.

## 1. Decisions

**Drawing on the processor, in tiny-skia (D22, M30.1, M30.3).** A 2D
context draws into a bitmap of its own, premultiplied 8-bit RGBA, with
`tiny-skia`, the rasterizer the software backend is built on (D8): its
paths, strokes with joins, caps and dashes, fills by both rules,
anti-aliasing, masks for clipping, gradients and patterns as shaders,
and every blend mode Compositing and Blending names, each composite
operation of the 2D context mapped onto one of them. Rather than drawing
on the GPU (D3): a canvas is read back by scripts (`getImageData`, the
pixels of the next frame in a game's collision test) as often as it is
drawn, and a readback from the GPU stalls; the software path is the one
every machine has, which a page's pixels must not depend on; and
tiny-skia is a port of Skia, the reference browser's rasterizer, so
that an edge's coverage and a gradient's steps come out as the
reference browser's do, within a level or two. *(corrected: inside a
shape and along a gradient within a level; at a curve's or a stroke's
edge tiny-skia's four-by-four samples stand apart from the reference
browser's coverage, a sixteenth in the crate's tests and up to 78
levels on the golden pages, where the exact coverage lies between
them.)* What M30.3 shares with
M19's split is the showing: the bitmap is an image to both backends,
drawn by the software backend and uploaded as a texture by the GPU
backend. A canvas drawn on the GPU is a later question (section 2).

- *`tiny-skia` 0.12.* The workspace moves from 0.11 to 0.12 (released
  February 2026): its radial gradient takes two circles, as
  `createRadialGradient` does, where 0.11's takes one and a focal
  point, and it adds the sweep gradient `createConicGradient` needs.
  Nothing else the render crate uses changes; the reference pages say
  whether any pixel did.
- *A crate of its own.* A new crate, `canvas`, below `engine` beside
  `render`, holds the context: its state and stack, its paths, its
  rasterization, its text, its pixels in and out, and the parsing of
  what scripts give it as strings, colours and fonts. It depends on
  `tiny-skia`, `text`, `images` (whose `Image` is the bitmap's shape,
  premultiplied RGBA, and whose decoders give `drawImage` its pixels)
  and `style` (whose colour parser becomes public for it) and `png` for
  the export, and `css`, whose tokenizer reads fonts and lengths
  *(corrected)*; it forbids `unsafe`. It knows no DOM and no script: a
  context is a Rust value the engine owns and the bindings drive, so
  that it is tested, fuzzed and benchmarked alone.
- *Colours.* `style`'s parser, made public as `style::parse_color_text`
  *(corrected)*, gains `hsl()`, `hsla()` and `hwb()`, which canvas code
  writes as often as hexadecimal (a particle's hue); CSS gains them with
  it, a part of G16. A function the text leaves open is closed at its
  end, as CSS Syntax closes one *(corrected: the web-platform tests)*.
  `currentcolor` is the canvas element's computed `color`. The host
  keeps the colours it parsed, by their text (512 at most, not one
  naming `currentcolor`): a page sets the same few each frame
  *(corrected: found at the acceptance)*.

**The element (M30.1, M30.3).** With scripts on, a `canvas` is a
replaced element (HTML 15.4.1): its children are not rendered, and its
natural size is its `width` and `height` attributes, 300 by 150 when
they are absent or not a valid non-negative integer. With scripts off
it represents its fallback content, as now. Layout gains
`Replaced::Canvas`, sized as an image of that size; `object-fit`,
`object-position` and `image-rendering` apply to it as to an image.
Paint draws it as `DisplayItem::Image` whose `ImageRef` names the
canvas (a key apart from images' and gradients' ids: bit 62 and the
node's bits *(corrected)*) and, as its frame,
the bitmap's generation, which every change to the bitmap moves on: the
display list's diff then damages the canvas's rectangle, the software
backend draws its tiles again, and the GPU backend, which keys its
textures by id and frame, learns a new path, writing a canvas's new
generation into its texture in place rather than making a texture a
frame. The engine fills the item's pixels from the canvas at drawing
time, as it fills an image's (`show_pixels`), a copy of the bitmap
taken when its generation moved.

- *Changed without layout.* A context's drawing marks the document
  changed for painting only, as an image's pixels arriving does
  (`image_read`), and wants a frame; nothing is laid out again. Setting
  `width` or `height` is an attribute change, which lays the canvas out
  again.
- *A layer of its own.* A canvas whose bitmap changed in the last
  frames is given a composited layer (M19's policy gains
  `LayerReason::Canvas`), so that a game's redraw redraws the canvas's
  layer and composites it, rather than drawing again the document's
  tiles beneath it. A canvas that stops changing falls back into the
  tiles at the next layerization, as a finished animation does.
  *(corrected: every canvas is a layer, its image wrapped in a group of
  its own, `GroupKey::Canvas`, so that the policy stays a function of
  the style and the fragment as M19's is; a still canvas costs a layer
  of its size. The document's tiles under it are still drawn again each
  frame on the software backend, until a layer tree composites over
  them, M35.10: 6.8 ms of a frame whose canvas changes whole.)*

**The bindings (M30.1).** The bootstrap's `HTMLCanvasElement` gains
`getContext`, `toDataURL` and `toBlob`; new classes
`CanvasRenderingContext2D`, `CanvasGradient`, `CanvasPattern`,
`TextMetrics`, `ImageData`, `Path2D`, `OffscreenCanvas`,
`OffscreenCanvasRenderingContext2D` and `ImageBitmap`, and
`createImageBitmap()`, with Web IDL's conversions and the reference
browser's words. `DOMMatrixReadOnly`, `DOMMatrix`, `DOMPointReadOnly`
and `DOMPoint` (Geometry Interfaces) are written in the bootstrap:
they are arithmetic, and the context's transform is kept in the engine
and handed across as six numbers.

- *Calls.* Each method is a native call that the host answers from the
  canvas's context (`Call::Canvas`, its operation and arguments; one
  variant, not one per method), as the DOM's calls are. A state
  attribute is read from the engine too, so that the state has one
  owner; the bootstrap keeps only what has no meaning there (the
  objects `CanvasGradient` and `CanvasPattern` are, by handle). A call
  costs about a microsecond; a particle demo of 2,000 particles makes
  some 8,000 a frame, 8 ms, measured at the acceptance. Batching the
  calls in the bootstrap, a command buffer sent once a frame, is the
  next step if it is not enough (section 2). *(corrected: measured, a
  call costs 0.8 to 1.8 us, `beginPath` 0.83, `lineWidth` 0.95,
  `fillStyle` 1.32 with the colours kept, `moveTo` 1.39 once the
  bootstrap called the commonest path methods straight, without an
  array and `Reflect.apply`; the demo's 8,000 about 14 ms. It is not
  enough, and batching stays G81's: the acceptance below.)*
- *Pixels across.* `getImageData` answers bytes, which the bootstrap
  receives as a new `ArrayBuffer` (`rquickjs` makes one from a `Vec`
  safely) under a `Uint8ClampedArray`; `putImageData` sends them as
  base64, `Uint8Array.prototype.toBase64` at the speed of C, as M29's
  memory crosses (`rquickjs` lends no buffer's bytes safely). A new
  `Answer::Bytes` carries them.
- *Where contexts live.* The document's host keeps its canvases, by the
  element's node, and its offscreen canvases, by handle, for the
  document's life; the engine reads a canvas's bitmap from there when
  it paints. *(corrected: by `Owner`, an element's node or an offscreen
  canvas's number, the bindings naming the latter's context by the
  negative of it; bitmaps and the objects scripts hold by number, with
  the document's bounds of the limits' table. The host has no image
  store: the engine shares with it what each `img` can give a canvas,
  `Drawables`, kept with what the element was scanned for, its `src`
  and `crossorigin`, so that one changed since is not drawn until its
  image comes.)*

**What a context does (M30.1).** As the HTML Standard says, with these
choices where it leaves one:

- *State.* Everything 4.12.5.1.3 lists, with `save()`, `restore()` and
  `reset()`; a stack of at most `MAX_CANVAS_STATES` (a `save()` past it
  does nothing).
- *Paths.* `moveTo`, `lineTo`, `quadraticCurveTo`, `bezierCurveTo`,
  `arcTo`, `arc` and `ellipse` (flattened to curves as Skia does),
  `rect`, `roundRect` with its radii's forms, `closePath`; transformed
  as they are added, as the standard says; `Path2D` the same on an
  object, and from SVG path data. `fill` and `clip` by `nonzero` or
  `evenodd`; `isPointInPath` and `isPointInStroke`. *(corrected: an arc
  whose angles differ by whole turns the other way round is a whole
  circle, as the reference browser draws it, `arc(x, y, r, 0, 2 * PI,
  true)`; a `roundRect` given one negative side runs the other way
  round, each radius with its corner as given; a stroke prunes its
  lines and curves of no length first, and a `strokeRect` of no width
  or height is a line closed on itself, with joins and no caps. A shape
  reaching more than 8,192 pixels past what it is drawn on is cut to
  that first, as tiny-skia's clipping of edges fails far out; a stroke
  is traced no finer than `MAX_STROKE_RESOLUTION`, nor, where its
  outline reaches past `FAR_STROKE` (2^22 pixels), than single
  precision holds there. The fuzzer found all three.)*
- *Paint.* Colours; linear, radial (two circles) and conic gradients,
  their stops interpolated in premultiplied sRGB; patterns of an image,
  a canvas or a bitmap with their four repetitions and a transform;
  `globalAlpha`; every `globalCompositeOperation`, the ones whose effect
  reaches outside the drawn shape (`copy`, `source-in`, `source-out`,
  `destination-in`, `destination-atop`) applied over the whole clip as
  the standard says: drawn into a layer the size of the shape, the rest
  of the clip then cleared *(corrected)*. `clear` among them, bounded
  *(corrected)*.
- *Strokes.* `lineWidth`, `lineCap`, `lineJoin`, `miterLimit`, the dash
  list and its offset, all transformed with the path.
- *Shadows.* Drawn under the shape from its own coverage, offset and
  blurred by a Gaussian of deviation `shadowBlur / 2`, made of three box
  blurs, as Skia does. *(corrected: within six levels of the reference
  browser's, 186, 124, 53, 26 and 3 against 186, 130, 50, 23 and 3; the
  shadow's layer covers only what can land on the bitmap, so a far
  offset costs nothing.)*
- *Clipping.* A mask the size of the bitmap, intersected by each
  `clip()`, saved and restored with the state.
- *Images.* `drawImage` in its three forms from an `img`, a `canvas`,
  an `OffscreenCanvas` and an `ImageBitmap`, filtered by bilinear
  interpolation or the nearest pixel as `imageSmoothingEnabled` says
  (`imageSmoothingQuality` reads back and filters alike); an image not
  loaded draws nothing; an animated image draws its first frame, as the
  standard says. An `img`'s pixels are decoded then if they are not.
  *(corrected: a canvas or `OffscreenCanvas` without a context draws as
  transparent black of its size, which an operator such as `copy`
  shows; an `img` fetches whether in the document or not, as `new
  Image()` needs.)*
- *Text (M30.1).* `fillText`, `strokeText` and `measureText` through the
  `text` crate: the string shaped, each glyph's outline turned into a
  path and drawn as any path is, so that text turns, scales, strokes,
  clips and fills with a gradient as a path does. `font` parsed as the
  shorthand (style, variant, weight, stretch, size and line height,
  families), its size resolved against the canvas's computed
  `font-size` (10px off the document), serialized as the reference
  browser does. The engine has one face (G22): every family, weight and
  style draws with it, at the size asked. `textAlign`, `textBaseline`,
  `direction` (`inherit` the element's), `letterSpacing` and
  `wordSpacing`; `maxWidth` squeezes the text horizontally.
  `TextMetrics` from the shaped glyphs and the face's metrics.
  *(corrected: spaced text shapes each character alone, without
  ligatures as the reference browser does, but puts it where the run
  shaped whole does, its kerning kept, the width the run's and the
  spacing; a spacing reads back with its unit in lower case, `ic` an
  em; `lh` is refused, the engine having no `line-height`, G22;
  `direction` reads back resolved, as the reference browser has it.)*
- *Pixels (M30.2).* `createImageData`, `getImageData` and
  `putImageData` (with its dirty rectangle), unpremultiplied as they
  leave the bitmap and premultiplied as they enter it, rounded as the
  reference browser rounds; `ImageData` in sRGB only, `colorSpace`
  reads `srgb`.

**Exporting (M30.2).** `toDataURL()` and `toBlob()` encode PNG, through
`png` 0.18's encoder, with no other type: a type the engine does not
encode, JPEG and WebP among them, gives PNG, as the standard allows; an
empty bitmap gives `data:,`. `toBlob` calls back in a task, with a
`Blob` (M27's). *(corrected: in a microtask; a page sees the callback
after the call returns either way.)*

**`OffscreenCanvas` (M30.2).** Constructed with a size; its
`getContext("2d")` an `OffscreenCanvasRenderingContext2D`, the same
context without the element's members; `width` and `height`;
`convertToBlob()` a promise of a PNG `Blob`; `transferToImageBitmap()`.
Not transferable to a worker until M31 (M31.4), nor made by
`transferControlToOffscreen()`. *(corrected: `getContext` takes the
`OffscreenRenderingContextId` enumeration, a name outside it a
`TypeError`; the settings dictionary of both `getContext`s is read in
its members' order, a primitive giving the defaults as the reference
browser takes it.)*

**`ImageBitmap`.** `createImageBitmap()` of an `img`, a `canvas`, an
`OffscreenCanvas`, an `ImageBitmap`, `ImageData` or a `Blob` holding an
image, with a source rectangle, resolving in a task *(corrected: a
microtask)* to a bitmap of premultiplied pixels; `close()`. Its options (`imageOrientation`,
`premultiplyAlpha`, `colorSpaceConversion`, `resize*`) are read and
ignored but `resizeWidth`, `resizeHeight` and `resizeQuality`'s
nearest-or-not.

**Security: the origin-clean flag (M30.1, fetch.md's deferral).** An
`img`'s `crossorigin` attribute, left for this milestone at M27.3, is
read: an image fetched in CORS mode, with credentials as the attribute
says, fails to load when the response is not allowed. A canvas is
origin-clean until it draws an image, or a pattern of one, that is not:
an image of another origin fetched without CORS, or a canvas or bitmap
that is not clean itself. Then `getImageData`, `toDataURL`, `toBlob`,
`convertToBlob` and `createImageBitmap` of it refuse with the reference
browser's `SecurityError`s. Setting `width` or `height` makes it clean
again, as the reference browser does. `isPointInPath` and
`measureText` read nothing of the pixels and stay allowed.

**Limits (the bounds a hostile page cannot pass).**

| Constant | Value | Bounds |
|---|---|---|
| `MAX_CANVAS_PIXELS` | 33,554,432 (2^25) | a canvas's bitmap |
| `MAX_CANVAS_SIDE` | 16,384 | a bitmap's width or height |
| `MAX_CANVAS_MEMORY` | 512 MiB | a document's bitmaps, in all |
| `MAX_CANVAS_STATES` | 1,024 | a context's saved states |
| `MAX_PATH_POINTS` | 1,048,576 | a path's points |
| `MAX_SHADOW_BLUR` | 256 | `shadowBlur`, as drawn |
| `MAX_DASHES` | 1,024 | a dash list's length |
| `MAX_CANVAS_OBJECTS` | 262,144 (2^18) | a document's paths, gradients, patterns and bitmaps *(corrected)* |
| `MAX_CANVAS_POINTS` | 4,194,304 (2^22) | the points of a document's paths, in all *(corrected)* |
| `MAX_CANVAS_STOPS` | 1,048,576 (2^20) | a document's gradient stops *(corrected)* |
| `MAX_DRAWN_DECODED` | 256 MiB | images decoded for drawing, kept *(corrected)* |
| `MAX_STROKE_RESOLUTION` | 65,536 | the finest a stroke is traced, as a scale *(corrected)* |
| `MAX_KEPT_COLOURS` | 512 | colours kept parsed *(corrected)* |

`MAX_CANVAS_MEMORY`, `MAX_CANVAS_OBJECTS`, `MAX_CANVAS_POINTS`,
`MAX_CANVAS_STOPS`, `MAX_DRAWN_DECODED` and `MAX_KEPT_COLOURS` are the
host's, a document's; the rest are the `canvas` crate's *(corrected)*.

A canvas past a bound has no bitmap: it draws nothing, reads back
transparent, and exports `data:,`, as the standard says of a bitmap
that cannot be made. `getImageData` and `createImageData` past
`MAX_CANVAS_PIXELS` throw a `RangeError`. A drawing operation is work
charged to the run's budget by the pixels it touches (its bounds within
the clip) and its path's length, so that a loop of large fills ends as
a loop of scripts does (M24).

**Tests.** The `canvas` crate's own: each operation's pixels against
the reference browser's answers above, within a level where coverage is
involved; the state's parsing and serialization. The `js` crate's: the
bindings through scripts, as the realm's other tests are, each class,
method and conversion and their errors in the reference browser's
words. *(corrected: the bindings' tests are the engine's,
`scripts/canvas_tests.rs`, through a page's scripts and the host; the
crate's allow a sixteenth at a curve's edge.)* The engine's: a canvas laid out and shown, changed and shown
again without a layout, its generation and damage, the GPU texture
updated in place, tainting across origins with the test server's two
origins, `toBlob` and `createImageBitmap` in tasks.

**Fuzzing.** A new target, `canvas`: sequences of the context's
operations with arguments the input gives (non-finite numbers, huge
and negative sizes, degenerate paths, every composite operation and
style), on a small bitmap, against the crate alone: nothing panics,
nothing allocates past the bounds, and the bitmap stays the size it
is. `dom_api` gains the context's calls on a page's canvas.
*(corrected: it found tiny-skia's walk of the edges failing on a shape
reaching billions of pixels out, cut since; its 16-bit blend lanes
overflowing on purpose, so the dev and fuzz builds check no overflow
in tiny-skia; and a stroke traced to a billionth of a unit, capped
since. Then 444,886 inputs in ten minutes without a crash. At the
acceptance, 642,016 more found a stroke a billion pixels wide slow,
eight million points, traced since to what single precision holds
where it reaches: 266; then 388,380 without a crash or a slow
input.)*

**The web-platform tests.** The `html/canvas/element` tests (the 2D
context's own, a testharness test each, built by
`html/canvas/tools/gentest.py`) and those of `html/canvas/offscreen`
for a window, by the pull's rule; the `manual` directory's reftests by
the reftest rule. A test that needs a font of its own, a filter, a
video, a colour space past sRGB or a worker is left out by the rule.
*(corrected: the `manual` reftests are left out, as the reftest rule
leaves out pages that draw by script; so are `layers`, `filters`,
`canvas-host`, `wide-gamut-canvas`, `color-type`, `global-hdr-headroom`
and `video`, `examples/wpt.rs` saying why. 2,180 testharness tests
in all; the first run found thirteen bugs, fixed; 326 subtests are
expected to fail, each with its gap: TextMetrics's proposed members,
`@font-face`, colour objects and `color-mix()`, SVG images, the system
colours, pixel formats past 8 bits, kerning, small caps and `lh`.)*

**Golden cases** continue from 130: a chart drawn on a canvas (axes,
labels, bars in a gradient, a line with a dash and a shadow) and a
canvas of paths, composite operations and an image, captured from the
reference browser. Text is the bundled font's, as the reference pages'
is; antialiased edges compare within the tolerance a page declares.
*(corrected: 130 is the chart without text, the reference browser's
face not the engine's, its shadow on 131, a canvas of paths, paint,
images and pixels; their edges within 80 and 48 levels and inside
within one, as the edges of curves and strokes came out above.)*

**Acceptance (M30.4).** The roadmap's: a chart (a page drawing one
with the API a charting library uses, its bars, axes, labels, legend
and a line with points) and a particle demo (2,000 particles, each a
filled circle of its own colour, moved and drawn each animation frame
over a cleared canvas), drawn correctly, as their golden pages and the
real shell show, and at 60 frames a second: each frame of the demo,
its script and its drawing and the frame's showing, within 16.7 ms on
the Windows machine in release, measured by `benches/canvas.rs` with the
same in a page's DOM for comparison. *(corrected: both are drawn
correctly, in the engine's tests and in the real shell, and the chart
draws in 32 ms from its load to its first frame; but 2,000 particles
take 26.9 ms a frame on the GPU backend and 31.3 on the software one,
37 and 32 frames a second, against 116 and 120 ms for the same as
elements of the page. Sixty frames a second hold up to 1,000 particles
on the GPU backend and 700 on the software one. What remains is the
calls, 14 ms, and tiny-skia's fills of the circles, 5.3 ms: section
2's batching and a canvas drawn on the GPU, G81. The shell printed
nothing a frame's callbacks logged; it does now. T4.3 found the rest
within 10% but a realm made, 5.2 ms slower, the bindings every realm
compiles grown by 2,700 lines: M35.12's.)*

## 2. Not in this milestone, said rather than hidden

- *A canvas drawn on the GPU*, and the batching of the context's calls
  in the bootstrap, should the acceptance's frames need either. G81.
  *(They do: the acceptance above.)*
- *`filter`* (a canvas's CSS filters), *`ctx.canvas` over a
  `transferControlToOffscreen()`*, *colour spaces past sRGB*
  (`display-p3`, `colorType: "float16"`), *the HDR attributes*. G81.
- *WebGL and WebGPU*, `getContext("webgl")` and the rest: `null` here.
  A milestone of their own, after M34's sandbox.
- *`OffscreenCanvas` in a worker*, transferred: M31.4.
- *Text in other faces than the engine's one*, by family, weight and
  style: G22, with `@font-face` (G12). Text along bidirectional runs
  drawn in visual order: with G10.
- *Hit regions*, `drawFocusIfNeeded` beyond doing nothing,
  `scrollPathIntoView`: dropped by the standard or dead.
- *Encoding JPEG and WebP* for `toDataURL`: PNG stands in, as the
  standard allows.
- *(corrected: found by the web-platform tests.)* `TextMetrics`'s
  proposed members, `fillTextCluster` and the colour objects a style
  may be given, proposals past the standard, and pixel formats past 8
  bits: G81. The system colours: G6. `color-mix()` and relative
  colours: G16. `font-kerning`, small capitals and `lh`: G22.
- *`captureStream()`* and video as a `drawImage` source: with media.

## 3. Order of work

Each step a vertical slice, with its tests, a sabotage pass and the
gate.

1. *The `canvas` crate*: `tiny-skia` 0.12, the context's state and its
   parsing (colours with `hsl()` and `hwb()` in `style`), paths, fills,
   strokes, dashes, clipping, transforms, gradients, patterns of a
   bitmap, composite operations, shadows, pixels in and out; its tests
   against the reference browser's pixels.
2. *The bindings*: `getContext`, `CanvasRenderingContext2D` (all but
   text and images), `CanvasGradient`, `Path2D`, `ImageData`, the
   geometry interfaces, `Call::Canvas` and the host's canvases.
3. *The element shown*: `Replaced::Canvas`, the generation, the paint
   and both backends, the GPU texture written in place, the canvas's
   layer, a frame wanted at a change.
4. *Text*: `font` and the text attributes, `fillText`, `strokeText`,
   `measureText`.
5. *Images and their origins*: `drawImage`, `createPattern` of images,
   `crossorigin` on `img`, the origin-clean flag.
6. *Export and offscreen*: `toDataURL`, `toBlob`, `OffscreenCanvas`,
   `ImageBitmap` and `createImageBitmap`.
7. *Fuzzing*: the `canvas` target, `dom_api`'s calls.
8. *The web-platform tests*: `html/canvas`.
9. *Golden cases* from 130.
10. *Acceptance*: the chart and the particle demo in the engine and the
    real shell, `benches/canvas.rs`, T4.3, fuzz runs.
11. *The documents*.
