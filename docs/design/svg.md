# Design note: SVG

Roadmap M32. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ends: where they
disagree, the code wins, and the places are marked *(corrected)*. What
ships is described in `docs/architecture.md` "Current state".

The engine could not draw a vector graphic. An `<svg>` in a page was
an ordinary element: the HTML parser honoured `/>` inside it and
nothing else of foreign content (G44), lowercased its names
(`viewBox` became `viewbox`, `linearGradient` `lineargradient`), and
gave it no namespace, since the DOM has none (G51); its elements were
laid out as unknown inline boxes, the text of a `<text>` drawn as a
line of the page's, its shapes as nothing; scripts saw an
`HTMLUnknownElement`. An SVG file was a broken image (M23 deferred it
here) and, opened by itself, its source shown as text. Icons, logos,
charts and diagrams, a large part of what the web draws, were missing
or garbled.

This milestone gives the engine SVG 2's static rendering: shapes and
paths, fills and strokes, gradients, transforms, `viewBox` and
`preserveAspectRatio`, groups and opacity, `<use>` and `<symbol>`,
nested viewports, clipping paths, and text v1, drawn by a new `svg`
crate through the `canvas` crate's context. An SVG file is an image
again, in `<img>`, a CSS `background-image`, a list marker and a border
image, parsed by an XML parser of the engine's own, styled by the
engine's cascade and drawn at the size it lands on in pixels, so that
it stays sharp however it is scaled. An `<svg>` in a page is a replaced
element sized as the reference browser sizes it, its elements in the
SVG namespace, styled by the page's CSS and by their presentation
attributes, drawn again without the page laid out again when only its
contents change, its shapes the targets of the pointer. An SVG file
opened by itself is shown as the reference browser shows one. SMIL
animation is rejected, with its reasons (M32.3). The engine learns to
draw at a display's scale factor, which the acceptance's "all DPIs"
needs and which this machine's 125% display has wanted since M1. The
acceptance is the roadmap's: an icon set and a statistical chart drawn
correctly at every device pixel ratio.

The standards are read at the versions current when this note was
written (12 October 2026):

- Scalable Vector Graphics (SVG) 2 (W3C Candidate Recommendation, and
  the Editor's Draft where it has moved on): chapters 5 (document
  structure, `<use>`, `<symbol>`, `<switch>`), 6 (styling and
  presentation attributes), 7 (geometry properties), 8 (coordinate
  systems, `viewBox`, `preserveAspectRatio`, units), 9 (paths), 10
  (basic shapes), 11 (text, the parts v1 takes), 13 (painting), 14
  (paint servers: gradients), and the DOM interfaces of each;
- CSS Masking Level 1 for `clip-path` and `<clipPath>`;
- Extensible Markup Language (XML) 1.0 (Fifth Edition) and Namespaces
  in XML 1.0 (Third Edition), for the parser;
- the HTML Standard (Living Standard of 9 October 2026): 13.2.6.5 (the
  rules for parsing tokens in foreign content), 13.2.6.3 (creating and
  inserting nodes, adjusting SVG attributes and foreign attributes),
  4.8.16 (SVG in HTML), 15.4.1 (the rendering of embedded content),
  and "load a document" for XML types;
- DOM Standard for namespaces (`createElementNS`, "validate and
  extract", the namespaced attribute methods);
- CSS Namespaces Level 3 for `@namespace` and `ns|type` selectors;
- CSS Images Level 3, 5.2 (the default sizing algorithm), and CSS 2's
  10.3.2 for a replaced box with a ratio and no size;
- CSS Transforms Level 1 for the `transform` attribute as a property;
  CSS Color Level 4.

What the standards leave to the implementation, or state but are worth
checking, was put to the reference browser (Chrome 154.0.8037.99,
headless) before this note was written (scratchpad `m32/p1-inline.html`
to `p7-text.html`, and `p5-top.svg`):

- an inline `<svg>` is `display: inline` and `overflow: hidden`; with no
  `width`, `height` or `viewBox` it is 300 by 150, its line 154 high in
  16px sans-serif (the inline box's descent below the baseline); with
  a `viewBox` alone it takes the container's width (400) and the
  `viewBox`'s ratio (200 high); with a width or a height and a
  `viewBox`, the other follows the ratio (100 wide gives 50; 40 high
  gives 80); a percentage resolves against the container (`50%` of 400
  is 200); CSS `width` wins over the attribute (77 over 10); a negative
  width is 0 and a height that does not parse is the default 150; a
  `viewBox` of no area is ignored;
- presentation attributes are the author's at the lowest specificity:
  any matching CSS rule wins over `fill="red"`, and the `style`
  attribute wins over both; an attribute value that does not parse
  (`fill="bogus"`, `fill="red !important"`, `stroke-width="-1"`) is
  ignored, white space around one is not an error, `opacity="2"` is
  clamped to 1 and `fill-opacity="-1"` to 0; of two attributes the
  parser lowercases alike (`FILL`, `Fill`), the first wins;
- computed values: `fill` `rgb(0, 0, 0)`, `stroke` `none`,
  `stroke-width` `1px` (`2em` gives `32px`), `stroke-miterlimit` 4,
  `stroke-dasharray` `none` (`1, 2 3` gives `1px, 2px, 3px`),
  `stroke-dashoffset` `0px`, `fill-rule` `nonzero`, `opacity` 1, the
  geometry properties as lengths (`x` `0px`, `width` `10px`, `rx` and
  `ry` `auto`, `cx`, `cy` and `r` in pixels); `transform="translate(3
  4)"` computes to `matrix(1, 0, 0, 1, 3, 4)`; `fill="currentColor"`
  takes `color`, from a `color` presentation attribute too; a paint
  `url(#nope) yellow` keeps its fallback (`url("#nope") rgb(255, 255,
  0)`) and `url(#nope)` alone keeps the reference; a `<text>`'s
  `font-size` is 16px and `font-family` the page's, inherited, and it
  computes `display: block`;
- interfaces: `SVGSVGElement`, `SVGRectElement`, `SVGCircleElement`,
  `SVGGElement`, `SVGTextElement`, `SVGLinearGradientElement`, the
  chain `SVGRectElement` > `SVGGeometryElement` > `SVGGraphicsElement` >
  `SVGElement` > `Element`; `createElementNS` with the SVG namespace
  makes them, `createElement("circle")` an `HTMLUnknownElement`;
  `className` is an `SVGAnimatedString`; `style` and `dataset` are
  there; `getBBox()` gives the geometry without stroke or transform
  (5, 6, 7, 8 for that rect), of a `<use>` its own box, of an element
  under `display: none` its geometry still; `getCTM()` takes the
  transform; `getComputedTextLength()` measures (45.6 for "Hello" at
  20px); `SVGAnimateElement` and `pauseAnimations` exist (SMIL);
- `elementFromPoint` finds the shape under the point: a filled rect's
  inside, a stroked rect's stroke and not its unfilled inside (the
  `<svg>` there), the `<use>` for its symbol's content, the `<svg>`
  between shapes and over `display: none`;
- an SVG image (`<img>` of `image/svg+xml`): `width="40" height="20"`
  gives a natural size of 40 by 20, `2em` and `1in` 32 and 96 (16px to
  the em), no size 300 by 150, a percentage width 300; with a `viewBox`
  alone the natural size reads 300 by 150 but the image is laid out by
  the ratio alone (800 by 400 in an 800-wide body); XML that is not
  well formed, an `<svg>` without the SVG namespace, or another root
  element give a broken image (`error`);
- pixels, an SVG image drawn on a canvas: a gradient from opaque red to
  transparent blue is purple at its middle (`128, 0, 130, 126`):
  interpolated unpremultiplied; a rect edge at a half pixel is half
  covered (alpha 128); a one-pixel line on an integer row covers two
  rows by half; an image scaled ten times is drawn again at its new
  size (a circle's edge sharp across one pixel), not stretched;
  `opacity` on a group composites the group first (blue over red at
  half: `0, 0, 255, 128`), where each child's own `opacity` mixes them
  (`85, 0, 170, 192`);
- an SVG file opened by itself is an `XMLDocument` of type
  `image/svg+xml` with no `body`, in no-quirks mode; its root `<svg>`,
  with no size, is `display: block` and `overflow: visible`, as large as
  the viewport (800 by 600 at 0, 0), on a transparent canvas the
  window's white shows through; its scripts run, inside a CDATA section;
- the XML parser (`DOMParser` with `image/svg+xml`): entities an
  internal DTD subset declares are expanded, a namespace named by one
  too (Illustrator's `xmlns="&ns_svg;"`); the five predefined entities
  and character references are; an entity not declared, an HTML one
  like `&nbsp;` among them, is an error; an external DTD is not
  fetched, and its document parses; a CDATA section stays one, and
  comments and processing instructions stay nodes; a byte order mark is
  skipped; attribute values are normalized (a newline and a tab become
  spaces); a prefixed attribute keeps its namespace and its qualified
  name (`xlink:href`); errors, each failing the whole document: an
  element left open, an end tag that does not match, an attribute
  twice, an unquoted value, a `<` in a value, a prefix not declared,
  content after the root element; and the billion laughs, refused as
  soon as the expansion grows past a factor ("Maximum entity
  amplification factor exceeded"); white space in text is kept as
  written;
- text (20px sans-serif): the first glyph starts at `x` on the
  baseline `y`; `text-anchor: middle` and `end` move the run by half
  and all of its advance (45.58 for "Hello"); a list of `x` places each
  character in turn, the rest following the last; a `<tspan>`'s `dx`
  and `dy` move the pen for it and for what follows, the shift of `dy`
  staying; white space collapses as HTML's does and is trimmed at the
  ends (`"  a\n     b   "` is three characters, "a b"),
  `xml:space="preserve"` and `white-space: pre` keep it (a newline as a
  space); `letter-spacing` applies; `dominant-baseline: middle` moves
  the glyphs down by about a quarter of the size; `getBBox()` of a text
  is its advance by the font's ascent and descent (22 high at 20px).

## 1. Decisions

**An `svg` crate of our own, drawing through `canvas` (D23, M32.1).**
`resvg` and `usvg` were weighed and set aside again (M23 did so first):
they bring their own XML parser, their own CSS, their own font stack and
a second copy of `tiny-skia`, so an inline `<svg>` styled by the page's
cascade could not be drawn by them without serializing the page's
computed styles back into a document; and their resolver reads local
files. A new crate, `svg`, below `layout`, over `canvas`, `style`,
`text` and `dom`, holds SVG: its microsyntaxes (lengths, number and
point lists, `viewBox`, `preserveAspectRatio`, the transform list),
the geometry of its shapes, and a *render tree* it builds from a
document and its computed styles: groups with a transform, an opacity
and a clip, shapes as `canvas::Path`s with their fill and stroke
resolved (a colour, a gradient, nothing), text as runs with their
positions. The tree holds no node of the document (only each shape's
node id, for hit testing) and is drawn at any size and scale by
walking it with a `canvas::Canvas`, whose context already does what
SVG's painting is (fills by both rules, strokes with joins, caps,
miter limits and dashes, gradients, clipping, transforms, text in the
engine's face) and already holds the bounds its fuzzing taught it
(shapes reaching billions of pixels out cut, strokes traced no finer
than single precision holds). The canvas crate gains what SVG needs and
2D contexts do not: a gradient's spread (`pad`, `reflect`, `repeat`)
and its own transform (`gradientTransform`, and an `objectBoundingBox`
gradient's box), a stroke's default miter limit of 4 being SVG's to
set, and `Path` exposed to its new neighbour where a shape's geometry
is asked for (`getBBox`, hit testing). `svg` forbids `unsafe`.

- *Group opacity.* A group with `opacity` below 1 is drawn into a
  canvas of its own and composited at its opacity, as the reference
  browser does (`0, 0, 255, 128` above); so is a shape with both a fill
  and a stroke, which overlap; a shape with one paint takes its opacity
  into that paint's alpha. Layers nest at most `MAX_SVG_LAYERS` deep;
  past it, the opacity is applied to each child, recorded.
- *Gradient interpolation* is tiny-skia's, unpremultiplied, as the
  reference browser's SVG is (`128, 0, 130, 126` above).

**Namespaces in the DOM, and foreign content in the parser (M32.2,
G44, G51 in part).** An element gains its namespace: `dom::Namespace`
is `Html`, `Svg`, `MathMl` or another URI, kept with its local name, in
its own case for any namespace but HTML's. `Document::tag()` keeps its
meaning, the tag of an HTML element, and answers `None` for any other:
the 360 places the engine asks an element's tag ask whether it is that
HTML element, and an SVG `<a>`, `<style>`, `<script>`, `<title>` or
`<image>` must not be taken for one. A new `element_name()` gives any
element's namespace and local name, for the places that mean any
element (selector matching, `getElementsByTagName`, serialization, the
bindings' names). An attribute keeps its qualified name as now
(`xlink:href`, `xml:space`); its namespace is that of its prefix's
(the XML and XLink namespaces, the only ones SVG uses), not stored.
`createElementNS`, `namespaceURI`, `prefix`, `getAttributeNS`,
`setAttributeNS`, `hasAttributeNS`, `removeAttributeNS` and
`getElementsByTagNameNS` come with it, attributes in other namespaces
than those two being G51's still.

The tree builder takes foreign content as the HTML Standard writes it:
an `<svg>` or `<math>` start tag inserts a foreign element, the SVG
element names and attributes adjusted to their case (`foreignObject`,
`linearGradient`, `viewBox`, `preserveAspectRatio` and the rest of the
standard's tables), `xlink:`, `xml:` and `xmlns:` attributes adjusted;
in foreign content a start tag is foreign unless it is one of the
breakout tags (`b`, `big`, `blockquote`, ... `font` with `color`,
`face` or `size`), which close the foreign elements; the HTML
integration points (`foreignObject`, `desc`, `title` in SVG,
`annotation-xml` with an HTML encoding in MathML) and the MathML text
integration points parse HTML within them; a self-closing foreign
element closes at once; CDATA sections are taken (the tokenizer's
states exist, unused till now); `</script>` in SVG runs the script as
HTML's does. The serializer writes foreign elements with their case
and their self-closed empty elements as `<rect/>`. Fragments
(`innerHTML` on an SVG element) parse in the context's namespace.

**Namespaces in CSS (M32.2).** Type selectors match an HTML element's
name in any case and another element's exactly (`linearGradient`
matches `linearGradient`, as the reference browser does in an HTML
document). `@namespace` (default and prefixed) and `ns|type`, `*|type`
and `|type` selectors are taken (CSS Namespaces 3); `[ns|attr]`
attribute selectors stay unsupported (G51). The user agent sheet
splits in two: its HTML rules under a default HTML namespace, so that
an SVG `<a>` takes no link colour and an SVG `<title>` is not the
document's; and SVG's own, under the SVG namespace, as the reference
browser's `svg.css` has them (`svg:not(:root)`, `symbol`, `image`,
`marker`, `pattern` and `foreignObject` `overflow: hidden`, the root
`<svg>` 100% wide and high, `text` and `foreignObject` `display:
block`, `text` `white-space: nowrap`, `transform-origin: 0 0` for SVG
elements but an outer `<svg>`).

**SVG's properties, and presentation attributes (M32.2).** `style`
gains the properties SVG paints with: `fill` and `stroke` (a paint:
`none`, a colour, `currentcolor`, `url(#id)` with a fallback colour or
`none`; `context-fill` and the rest as `none`), `fill-opacity`,
`stroke-opacity`, `fill-rule`, `clip-rule`, `stroke-width`,
`stroke-linecap`, `stroke-linejoin`, `stroke-miterlimit`,
`stroke-dasharray`, `stroke-dashoffset`, `stop-color`, `stop-opacity`,
`text-anchor`, `dominant-baseline`, `paint-order`, `shape-rendering`,
`vector-effect` (parsed; `non-scaling-stroke` drawn), `clip-path`
(`url(#id)` and `none` for SVG elements), and the geometry properties
`x`, `y`, `cx`, `cy`, `r`, `rx`, `ry` (`width` and `height` are CSS's
already), inherited as SVG 2's table says, computed as the reference
browser computes them above, serialized by `getComputedStyle`.
Presentation attributes are presentational hints, as `img`'s
`width` and `height` are since M23: on an SVG element, each attribute
that names one of these properties (and `color`, `display`,
`visibility`, `opacity`, `overflow`, `transform`, `font-family`,
`font-size`, `font-style`, `font-weight`, `letter-spacing`,
`word-spacing`, `white-space`) becomes a declaration of the author's
level at no specificity, its value parsed by the property's own
grammar with SVG's leniencies (a number without a unit is pixels, white
space at the ends is no error), one that does not parse ignored. The
`transform` attribute's grammar is SVG's (unitless numbers, commas or
spaces, `rotate(a x y)`), turned into the property's matrix; an outer
`<svg>`'s `width` and `height` are its CSS size's hints.

**The render tree, and what v1 draws (M32.1).** SVG 2 sections 13 and
14, for these elements: `svg` (outer and nested, its viewport clipped
when `overflow` says so), `g`, `defs`, `symbol` (drawn through `use`),
`use` (of an element of the same document; a reference to another
document, or one that reaches its own ancestor, draws nothing), `switch`
(its first child whose `requiredExtensions` and `systemLanguage` pass),
`a` (drawn as a group), `rect` (with `rx` and `ry`), `circle`,
`ellipse`, `line`, `polyline`, `polygon`, `path` (the path data grammar
of the canvas crate's `Path2D`, its errors keeping what came before),
`text` and `tspan`, `linearGradient` and `radialGradient`
(`gradientUnits`, `gradientTransform`, `spreadMethod`, `href`
inheritance of stops and attributes, `fx`, `fy` and `fr`), `stop`, and
`clipPath` (`clipPathUnits`, its children's union, `clip-rule`). `title`, `desc`, `metadata` and
`style` draw nothing; `<style>` in SVG is a style sheet, as HTML's is.
Painting follows SVG's order (fill, then stroke, or as `paint-order`
says), `display: none` draws nothing under it, `visibility: hidden` the
element alone. A `viewBox` maps to the viewport by
`preserveAspectRatio` (`xMidYMid meet` by default, `none` stretching,
`slice` cutting). Lengths: a percentage of the nearest viewport's width,
height or normalized diagonal, `em` and `ex` of the element's font,
the absolute units at 96 per inch. An element in error (a negative
radius, a degenerate gradient) draws as SVG 2 says, mostly nothing.

- *Text v1.* A `<text>`'s characters, white space collapsed as SVG 2
  and the reference browser do (or kept, by `xml:space` or
  `white-space`), shaped in the engine's face (its only one: family,
  weight and style are G22's) at the `font-size`, placed from `x` and
  `y` with `dx`, `dy` and per-character lists, moved by
  `text-anchor`, the baseline by `dominant-baseline` (`auto`,
  `middle`, `central`, `hanging`, `text-top`, `text-bottom`), filled
  and stroked as a path is, `letter-spacing` and `word-spacing` taken;
  `<tspan>`s nest within it. `textPath`, `rotate`, `textLength`,
  vertical text and bidirectional runs are G87's.

**Inline `<svg>` (M32.2).** An outer `<svg>` (an SVG `svg` whose parent
is not an SVG element) is a replaced element, `Replaced::Svg`: its
children make no boxes. Its natural size is its `width` and `height`
attributes when they are absolute lengths (`em` of its font), its
ratio its `viewBox`'s or else that size's; the CSS size, the attributes'
hints among it, decides the rest as for any replaced box. A ratio with
no size takes the containing block's width, as CSS 2's 10.3.2 suggests
and the reference browser does (400 by 200 above), where layout's
default sizing contained it in 300 by 150; an image of an SVG file
with only a `viewBox` is laid out so too. Paint draws it as an image
whose `ImageRef` names the `<svg>` (bit 61 and the node's bits) and,
as its frame, a generation the engine moves when anything under the
`<svg>` changes or its style does. The engine builds the render tree at
that time and draws it at the size the image lands on in raster pixels
(`drawn_size`, which already decodes images at their drawn size),
through the group transforms above it and the device scale, and keeps
that bitmap while neither the generation nor the size moves. A change
under an outer `<svg>` that does not change its own size or style
lays nothing out again.

- *Hit testing.* A point in an outer `<svg>`'s content box is put to
  its render tree, topmost first: a shape whose fill (by its rule) or
  stroke contains it, as `pointer-events: visiblePainted` says, gives
  its element (a `<use>`'s content gives the `<use>`); else the
  `<svg>`. Events dispatch to that element and bubble through the SVG
  tree into the page.
- *Scripts.* Every SVG element gets an interface of SVG's: `SVGElement`
  over `Element` with `style`, `dataset`, `className` (an
  `SVGAnimatedString`), `ownerSVGElement`, `viewportElement`, and its
  own (`SVGSVGElement`, `SVGGElement`, `SVGRectElement` and the rest,
  through `SVGGraphicsElement` and `SVGGeometryElement`), with
  `getBBox()`, `getCTM()`, `getScreenCTM()`, `getComputedTextLength()`,
  `getNumberOfChars()`, `isPointInFill()` and `isPointInStroke()`, and
  `getTotalLength()` and `getPointAtLength()` on geometry. The animated
  values (`x.baseVal`, `SVGLength`, `SVGTransformList`) are G87's: the
  attributes are what scripts set.

**SVG images (M32.1).** A response of `image/svg+xml`, or whose type the
image request leaves unknown and whose bytes begin like SVG (MIME
Sniffing does not sniff SVG; the engine's image store does, for a
`file:` URL), goes to the image store's SVG path: parsed by the XML
parser on a decode thread, its root an SVG `svg` or the image broken,
styled by its own sheets (the user agent's and its `<style>`
elements) in a cascade of its own, its render tree built once; it runs
no script, loads nothing outside itself (an external `<use>`, `<image>`
or `@import` draws nothing, a `data:` image or font included), plays no
animation (the reference browser's secure static mode, SVG Integration).
Its natural size and ratio are as above (300 by 150 reported with no
size); `images::Header` learns a size that may be missing, so layout's
`natural()` reads the size or the ratio it has. It is drawn at the size
it lands on in raster pixels, a background tile at its tile's, a
border image's slices from one drawing; a drawing is kept per size,
under the image store's budget. `drawImage` of an SVG `img` on a canvas
draws it at the size drawn; printing at the print's resolution.

**SVG documents (M32.1).** A navigation's response of `image/svg+xml`
makes an XML document of the XML parser (other XML types and XHTML
keep M15's stated deviations: XHTML through the HTML parser, other XML
as text): `document.contentType` its type, no `body`, no-quirks mode,
its scripts run as an HTML document's do, its `<style>`s applied; its
root `<svg>` is laid out as an outer `<svg>` that is the root element:
`display: block`, 100% of the viewport by the user agent sheet, its
`overflow` visible. A document that is not well formed shows the
parser's error as the reference browser does, a short page naming the
line and column, rather than nothing.

**The XML parser (M32.1).** A new module of the `html` crate, `xml`,
bytes to a `dom` tree as the HTML parser's are: XML 1.0 (Fifth Edition)
with Namespaces, non-validating. The encoding from a byte order mark or
the declaration's `encoding` (the Encoding Standard's labels), else
UTF-8. Elements, attributes (normalized as the standard says), text,
CDATA sections (as text: the DOM has no CDATA node, nor comment or
processing-instruction nodes yet, which an XML document's keep:
recorded), character references, the five predefined entities, and
the internal subset's general entities (text and markup in them), with
the reference browser's guard: an expansion past `MAX_ENTITY_EXPANSION`
characters or a factor of `MAX_ENTITY_FACTOR` over the document is an
error, as is an entity that refers to itself. An external subset or
entity is never fetched. Every well-formedness error ends the parse
with its line and column; there is no recovery, as there is none in
the reference browser's. Namespace prefixes are resolved through
`xmlns` declarations; an undeclared prefix is an error; `xml:` is
bound.

**Drawing at a display's scale (M32.4, G9 in part).** The engine gains
a device scale factor, `Engine::set_scale_factor`: the viewport, in CSS
pixels, is the frame's size over it; the page's display list is drawn
under a scale by it, as a print's is at its resolution; the
`resolution` media feature and `devicePixelRatio` say it; images and
SVG are drawn at the pixels they land on (the canvas's bitmap, by its
nature, is scaled). The shell gives the window's scale factor (winit's)
and takes it again when it changes, the pointer's position over it,
the toolbar drawn at it too. Edges are snapped to CSS pixels as now,
which at a scale that is not whole leaves a border's edge between
device pixels where the reference browser snaps to them: recorded.

**Limits (the bounds a hostile document cannot pass).**

| Constant | Value | Bounds |
|---|---|---|
| `MAX_XML_DEPTH` | 512 | elements open at once in the XML parser |
| `MAX_ENTITY_EXPANSION` | 1,048,576 | characters entities expand to, in a document |
| `MAX_ENTITY_FACTOR` | 10 | the expansion over the document's own size |
| `MAX_SVG_ELEMENTS` | 262,144 (2^18) | the elements a render tree is built from, `<use>`s' instances counted |
| `MAX_USE_DEPTH` | 32 | `<use>` within `<use>` |
| `MAX_SVG_LAYERS` | 16 | groups drawn into layers of their own, nested |
| `MAX_GRADIENT_STOPS` | 1,024 | a gradient's stops |
| `MAX_SVG_PIXELS` | 33,554,432 (2^25) | an SVG drawing's bitmap, as a canvas's |
| `MAX_SVG_IMAGE_BYTES` | 32 MiB | an SVG image's bytes, as any image's |

A render tree past its elements' bound draws what was built; a `<use>`
past its depth draws nothing; a drawing past its pixels is drawn at the
largest scale that fits and stretched. The canvas crate's bounds
(points of a path, dashes, the reach of a shape) apply to what the
tree draws through it.

**Tests.** The `svg` crate's own: each microsyntax, each shape's
geometry, each painting rule's pixels against the reference browser's
answers above, through a small canvas. The `html` crate's: the XML
parser against XML's grammar and the probes above; foreign content
against the HTML Standard's tree construction (the html5lib
tree-construction tests of foreign content, vendored). The `style`
crate's: namespaces in selectors and sheets, SVG's properties and the
presentation attributes. The engine's: an inline `<svg>` laid out,
drawn, changed and drawn again without a layout, hit, scripted; an SVG
image in each place an image goes, its natural sizes, its static mode;
an SVG document opened; a page drawn at a scale.

**Fuzzing.** Two new targets: `xml` (any bytes to the XML parser:
nothing panics, entity expansion bounded) and `svg` (any bytes as an
SVG image: parsed, styled, its tree built and drawn small: nothing
panics, nothing allocates past the bounds). `dom_api` gains inline SVG:
elements made with `createElementNS`, attributes set and removed, a
`<use>` pointed at its ancestor.

**The web-platform tests.** The `svg` directory's reftests and
testharness tests by the pull's rule, `.svg` tests opened as SVG
documents, their `<h:link rel="match">` read; the CSS tests the rule
left out for an `<svg>` element or an SVG image taken again; the
html5lib foreign content tests. A test that needs SMIL, a filter, a
mask, a marker, a pattern, `textPath`, `foreignObject`'s HTML or the
animated values is left out by the rule.

**Golden cases** continue from 133: shapes, strokes and dashes;
gradients and their spreads; transforms, `viewBox` and
`preserveAspectRatio`; `use`, `symbol` and clipping; an SVG image in
`<img>` and as a background; captured from the reference browser.
Text is left out, the reference browser's face not the engine's, as at
M30; the acceptance's chart has its labels in the engine's tests.

**Acceptance (M32.4).** The roadmap's: an icon set (thirty icons of a
stroke-drawn and a filled family, as inline SVG through `<use>` of
`<symbol>`s and as `<img>`s, at 16, 24 and 48 pixels) and a statistical
chart (axes with ticks and labels, gridlines, bars, a line with points,
an area under a gradient, a legend, as a charting library writes it)
drawn correctly at device pixel ratios 1, 1.25, 1.5, 2 and 3: at each
ratio the drawing is the same picture at that many pixels, edges as
sharp as at 1 (compared with the same SVG drawn at the larger size at
1), in the engine's tests and in the real shell at this machine's
125%. `benches/svg.rs` measures the icon page and the chart's first
frame, a change to the chart, and an SVG image drawn at a new size.
T4.3 and fuzz runs.

## 2. Not in this milestone, said rather than hidden

- *SMIL animation* (`<animate>`, `<set>`, `<animateTransform>`,
  `<animateMotion>`): rejected (M32.3). The reference browser has it,
  but it is a second animation model beside CSS animations and Web
  Animations (M22), with its own timing graph, events and interfaces,
  and the web's animated SVG moved to CSS and scripts long ago; its
  elements draw nothing and change nothing here, as in a browser
  without SMIL. To be weighed again at T4 by what the web-platform
  tests and the pages of the time ask.
- *Filters* (`<filter>`, `filter` on SVG elements), *masks* (`<mask>`,
  `mask`), *markers* (`<marker>`, `marker-*`), *patterns* (`<pattern>`)
  and *`<image>`* in SVG: G87, with `foreignObject`'s HTML content,
  `textPath`, `rotate`, `textLength`, vertical and bidirectional text,
  the animated DOM values (`SVGAnimatedLength`, `SVGTransformList`),
  `<view>` and fragment identifiers into an SVG image, and
  `vector-effect`'s other values.
- *CSS `clip-path` on HTML boxes* and `mask-image`: CSS Masking for
  the page's boxes, a milestone's work with compositing; SVG elements'
  `clip-path` is here.
- *An SVG font* (`<font>`, removed from SVG 2), and fonts by family,
  weight and style for SVG text: G22.
- *Attribute namespaces past XLink and XML*, `[ns|attr]` selectors, and
  comment, processing-instruction and CDATA nodes in the DOM: G51.
- *XHTML through an XML parser*: M15's deviation stays; the parser is
  there to take it, a later step.
- *Snapping to device pixels* at a scale that is not whole, and the
  display's scale in the `ui` crate's text metrics past drawing: with
  M35's layout polish.

## 3. Order of work

Each step a vertical slice, with its tests, a sabotage pass and the
gate.

1. *Namespaces in the DOM and foreign content in the parser*:
   `dom::Namespace`, `tag()` for HTML elements, `element_name()`,
   the tree builder's foreign content and adjustments, CDATA, the
   serializer.
2. *Namespaces in CSS and the bindings*: `@namespace` and namespace
   selectors, case-sensitive type selectors for foreign elements, the
   user agent sheet split; `createElementNS` and the namespaced
   methods, SVG's interface names.
3. *The XML parser*.
4. *SVG's properties and presentation attributes*.
5. *The `svg` crate*: microsyntaxes, the render tree, shapes and
   paths, fill and stroke, transforms, `viewBox`, groups and opacity,
   drawn through `canvas`.
6. *Paint servers and structure*: gradients (the canvas crate's spread
   and transform), `use` and `symbol`, nested `svg`, `clipPath`,
   `switch`.
7. *Text v1*.
8. *Inline `<svg>`*: `Replaced::Svg`, sizing with a ratio alone,
   drawing at its raster size, changes without layout, hit testing,
   the geometry methods for scripts.
9. *SVG images*: the image store's SVG path, natural sizes, the static
   mode, drawing at the drawn size, backgrounds, border images,
   markers, print, `drawImage`.
10. *SVG documents*: navigation of `image/svg+xml`, the parser's error
    page.
11. *Drawing at a display's scale*: the engine's scale factor and the
    shell's.
12. *Fuzzing*: `xml`, `svg`, `dom_api`'s SVG.
13. *The web-platform tests*: `svg`, the CSS tests taken again,
    html5lib's foreign content.
14. *Golden cases* from 133.
15. *Acceptance*: the icon set and the chart at every ratio, in the
    engine and the real shell, `benches/svg.rs`, T4.3, fuzz runs.
16. *The documents*.
