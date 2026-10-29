# Design note: boxes and colour (gap batch E)

The second batch of the G-list worked through by its priorities
(`silver-work/notes/gap-priorities.md`, 28 October 2026). Written before
the code, as the record of the decisions, and corrected to the code
when the batch ends: where they disagree, the code wins, and section 4
lists the places. What ships is described in `docs/architecture.md`
"Current state", and the limits it adds live in `docs/security.md`.

The engine lays out and draws most of an ordinary page, but the page
looks wrong in the ways a reader sees first. Every box is square:
`border-radius` is not a property here, so a round avatar, a pill
button and a card's corners come out as squares (G32), and M22's
spinner is a square ring. There is no `box-shadow` and no `filter`
(G32), so a card floats on nothing and a greyed-out image is in
colour. Colours past CSS Color 3 are dropped: `lab()`, `lch()`,
`oklab()`, `oklch()`, `color()` and `color-mix()` (G16), which design
systems now write their palettes in, and `currentcolor` outside the
few properties that take it. A page asking for the dark scheme gets
the light one: no `color-scheme`, no `light-dark()`, no system colours
(G6). The math functions past `calc()`, `min()`, `max()` and `clamp()`
are invalid (G15), and with them every declaration that writes one.
`display: flow-root` is dropped (G23); `aspect-ratio` sizes only images
(G36); a child's top margin collapses through the root (G11);
`background-position-x` and `-y` are unknown (G40); and a conic
gradient is not an image (G37, part).

This batch gives the engine all of them. Its acceptance is WPT's
directories for them (fetched for it, at the checkout's commit:
`css/css-backgrounds` whole, `css/filter-effects`, `css/css-color`,
`css/css-color-adjust`, `css/css-display`,
`css/css-sizing/aspect-ratio`, `css/css-values` whole and
`css/css-images` whole), their reftests taken
by the pull's rule as the properties land and their testharness
parsing and computed-value tests added, with expectations; and pages
drawn against the reference browser's captures.

The documents this note follows, at their versions of 29 October 2026:
CSS Color 4 (the colour functions, the predefined colour spaces,
`currentcolor`, system colours, interpolation, serialization) and CSS
Color 5's `color-mix()`; CSS Color Adjustment 1 (`color-scheme`, the
`color-scheme` meta element of HTML 4.2.5.4); CSS Color 5's
`light-dark()`; CSS Backgrounds 3 (`border-radius`, `box-shadow`, the
corner shaping of backgrounds, borders and clips) and Backgrounds 4's
`background-position-x` and `-y`; Filter Effects 1 (`filter` and its
functions, the filter region); CSS Values 4's math functions (section
10: stepped values, trigonometric, exponential and sign-related
functions); CSS Display 3 (`flow-root`); CSS Sizing 4's preferred
aspect ratios (section 5); CSS 2's section 8.3.1 at the root; CSS
Images 4's conic gradients and `<color-interpolation-method>`.

What the standards leave open, or say but are worth checking, was put
to the reference browser (Chrome 156.0.8078.48, headless, on this
machine) before this note was written (scratchpad `be/pages/probe.html`
and `be/pages/dark.html`). Its lines, which the engine's values follow:

- A colour keeps its form. `lab(50 20 30)` specifies and computes as
  itself; `lab(50% 20% -30% / 0.5)` as `lab(50 25 -37.5 / 0.5)`;
  `oklch(60% 0.2 400deg / 50%)` as `oklch(0.6 0.2 40 / 0.5)`; `none`
  is kept (`oklch(none 0.1 120)`); `color(xyz ...)` is
  `color(xyz-d65 ...)`; out-of-range components of `color()` are kept
  (`color(display-p3 1.5 0 0)`), while `lab()`'s lightness is clamped
  to 100, `oklab()`'s to 1 and a negative chroma to 0. `hwb()` and
  `hsl()` compute to `rgb()`, as before.
- `color-mix()` specifies as written (a missing second percentage
  filled in, `red 30%, blue 70%`) and computes to a colour of its
  interpolation space: `in srgb` a `color(srgb ...)`, `in oklch` an
  `oklch()`, `in lab` a `lab()`, `in hsl` a `color(srgb ...)`; with no
  space, `oklab`. Percentages summing under 100 scale the alpha
  (`red 20%, transparent 20%` is `color(srgb 1 0 0 / 0.2)`).
  `currentcolor` inside it is resolved at computed time.
- `light-dark(red, blue)` computes to `rgb(255, 0, 0)` under the light
  scheme and `rgb(0, 0, 255)` under the dark one. System colours
  specify lower-cased and compute to `rgb()`, each by the element's
  used scheme: `Canvas` white or `rgb(18, 18, 18)`, `CanvasText` black
  or white, `Field` white or `rgb(59, 59, 59)`, `ButtonFace`
  `rgb(240, 240, 240)` or `rgb(107, 107, 107)`, `LinkText`
  `rgb(0, 102, 204)` or `rgb(158, 158, 255)`, and the rest of the
  probe's table (`be/out-dark.txt`), the deprecated names mapped as
  css-color-4 appendix A maps them.
- `color-scheme` specifies and computes as written in order, `only` last
  (`dark only`), unknown words kept (`dark light foo`). An element's
  `color-scheme: dark` changes its system colours and its controls, not
  its inherited `color`; the root's (or a `color-scheme` meta element's
  `dark`) makes the root's `color` white, the initial value being
  `CanvasText`, and the canvas dark. With `light dark` the user's
  preference chooses.
- A dark text field computes `color` white, `background-color`
  `rgb(59, 59, 59)` and its border `rgb(133, 133, 133)`; a dark button
  `rgb(107, 107, 107)` with white text.
- `border-radius: 10px 20px / 5px` computes as written; a negative
  radius is invalid. `box-shadow: 1px 2px 3px 4px red` computes to
  `rgb(255, 0, 0) 1px 2px 3px 4px`, every length written out and
  `inset` last; a negative blur is invalid.
- `filter: grayscale()` computes to `grayscale(1)`, percentages to
  numbers (`saturate(2)`), `drop-shadow(1px 2px 3px red)` to
  `drop-shadow(rgb(255, 0, 0) 1px 2px 3px)`; a negative `blur()` is
  invalid.
- `round(55px, 10px)` specifies as `calc(60px)`, `rem(-55px, 10px)`
  as `calc(-5px)` (the width then clamped to 0); `round(55%, 10px)`
  stays as written until used. `pi` and `e` are kept to six figures.
- `display: flow-root` computes as itself, `block flow-root` as
  `flow-root`, `inline flow-root` as `inline-block`.
- `aspect-ratio: 2` computes to `2 / 1`, `auto 2` to `auto 2 / 1`.
- `background-position-x: right 10px` computes to
  `calc(100% - 10px)`; a list of positions computes to as many as the
  layers.
- `conic-gradient(from 90deg at 10% 20%, red 0deg, blue 0.5turn)`
  computes with its angles in degrees and its colours as `rgb()`;
  `linear-gradient(in oklch, red, blue)` keeps its method.

## 1. Decisions

### Colours keep their space (D48)

`style::computed::Color` stays a small `Copy` value, and gains the form
it was written in: its colour space (the legacy sRGB forms, `lab`,
`lch`, `oklab`, `oklch`, and the predefined spaces of `color()`), its
three components there with a bit each for `none`, and its alpha. The
sRGB channels that drawing reads are worked out once, when the colour
is made, through CIE XYZ by css-color-4's conversions (section 18's
sample code, the D50-D65 Bradford adaptation where the space needs
it). Out-of-gamut colours are clipped channel by channel when drawn,
as the reference browser draws them on a display taken as sRGB (G38);
css-color-4's gamut mapping by OKLCH chroma is not applied, a
departure the note records.

Serialization follows the probe: the legacy forms as `rgb()` and
`rgba()`, each other space in its own function, six significant
figures. Interpolation (transitions, animations, gradients and
`color-mix()`) follows css-color-4 section 12: in the space the method
names, with premultiplied alpha, hue arcs by `shorter`, `longer`,
`increasing` and `decreasing`, missing components carried over; by
default sRGB when both ends are legacy, else Oklab, as the probe and
section 12.1 say.

`color-mix()` and `light-dark()` are resolved where the value is
computed. So is `currentcolor` in the properties that are not
inherited, against the element's computed `color`, which is what
`getComputedStyle` shows; the properties that inherit a colour and take
`currentcolor` (`fill`, `stroke`, `caret-color`, `text-emphasis-color`)
keep it as a keyword, as `fill` and `stroke` already do.

### The scheme an element uses (D49)

`color-scheme` is an inherited property, its value the list as written.
An element's used scheme is the first of its schemes the user's
preference (`Preferences::color_scheme`, already read by media queries)
allows, or light. The root's value falls back, when it is `normal`, to
the document's `color-scheme` meta element's, read as HTML says (the
first such element in the head, its content parsed as the property).

System colours and `light-dark()` resolve where the value is computed,
by the used scheme of the element whose value it is; the two tables are
the reference browser's. `color`'s initial value becomes `CanvasText`,
resolved at the root, so that a dark root's text is white and the rest
inherit it. The canvas (the viewport's background where the root and
the body give none) is `Canvas` by the root's used scheme. The user
agent stylesheet writes the controls' colours as system colours and
`light-dark()` pairs, so that a dark control is the reference browser's
dark; the light ones keep the colours M26.7 chose, the grey borders
among them.

### Rounded boxes are drawn as paths (D50)

`border-radius` and its four longhands are computed as two
length-percentages a corner, resolved against the border box when the
box is painted and reduced by Backgrounds 3 section 5.5's factor where
adjacent radii overlap. Paint turns a rounded rectangle into a path:
each corner's quarter ellipse flattened into straight segments, few
enough that the chord is within a quarter of a device pixel of the
curve, so the shape is the same at any scale the frame is drawn at.

A new display item, `Path`, fills one or more such contours by the
nonzero rule, anti-aliased by the same exact-area coverage the
`Polygon` item has, which both backends already share (the GPU's
through its atlas). A rounded background is a `Path`; a border solid in
one colour all round is a `Path` of its outer and inner contours, the
inner reversed; a border whose sides differ is a `Path` a side, split
at each corner on the line from the outer curve's to the inner curve's
point at the corner's angle, as the reference browser splits them.
`double`, `groove`, `ridge`, `inset` and `outset` divide the band as
they do square, into paths. `dashed` and `dotted` sides keep their
dashes on the straight runs and draw the curved corners solid, a
departure section 2 records.

Boxes without radii keep the items they have, so a page without
`border-radius` draws as before.

### Rounded clips are masked groups (D51)

What a rounded box clips (an `overflow` other than `visible`, a
replaced element's content, a background image's layer by its
`background-clip`) cannot be clipped by the axis-aligned clip the
display list has. A new item, `PushRoundedClip`, a rectangle and its
eight radii, ended by `PopClip` like any clip, makes the items inside
it a group drawn into a scratch of its own and composited through the
rounded rectangle's coverage, in both backends, each of which already
draws groups through scratches. The axis-aligned clip around the shape
is in force too, so that nothing outside the rectangle is drawn into
the scratch.

### Blur, shadows and filters (D52)

`box-shadow` is a list of shadows, each painted as a new item,
`Shadow`: the shadow's rounded rectangle (the border box offset, grown
or shrunk by the spread, its radii by the spread too, as section 7.1.1
says), its blur, its colour, whether it is inset, and the box it is
clipped out of (outside the border box for an outer shadow, inside the
padding box for an inset one). Its coverage is a mask worked out by
code `render` shares between its backends: the shape's coverage, then
a Gaussian blur of standard deviation half the blur radius, made as
three box blurs a direction (the approximation the reference browser's
raster uses), then the clip; masks are cached by their shape's size,
radii and blur, so that a list of a hundred alike cards blurs once.
The GPU backend draws the mask from its atlas as it draws a polygon's.
Outer shadows paint below the background, inset ones above it and
below the border, as section 7.1 orders them.

`filter` is a list of filter functions; a box with one other than
`none` is a stacking context, its group drawn into a scratch and
filtered before it is composited (`PushGroup` gains the list). The
colour functions (`brightness`, `contrast`, `grayscale`, `hue-rotate`,
`invert`, `opacity`, `saturate`, `sepia`) are colour matrices, applied
to unpremultiplied colour in sRGB, which is what the reference browser
does for CSS's shorthand filters whatever `color-interpolation-filters`
says; consecutive ones compose into one matrix. `blur()` blurs the
group by the same Gaussian as shadows; `drop-shadow()` blurs the
group's alpha, tints it, offsets it and draws it below. The group's
extent grows by three standard deviations of each blur and by each
offset, which the backends' damage and reach take into account.

The software backend runs the filters on its scratch pixmap. The GPU
backend does the colour matrices in its composite shader and the blurs
in two render passes a blur, along each axis, by weights of the same
Gaussian; its pixels may differ from the software backend's by a level
or two of a channel where blurred, and the tests between the backends
say so. An SVG filter by `url()` is left with SVG's filters (G87), and
`backdrop-filter` is left too (section 2).

### Math functions

The parser of `calc.rs` takes the rest of Values 4's math functions:
`round()` with its strategy, `mod()`, `rem()`, `abs()`, `sign()`,
`sin()`, `cos()`, `tan()`, `asin()`, `acos()`, `atan()`, `atan2()`,
`pow()`, `sqrt()`, `hypot()`, `log()` and `exp()`, typed by section
10.9 (`sign()` and the trigonometric functions of an angle are numbers,
the inverse ones angles). Those whose arguments hold no percentage fold
where the value is computed, as the rest do; one whose arguments hold a
percentage of a length not yet known stays a node of the `Expression`
that layout evaluates. Values 5's `progress()`, `random()`,
`calc-size()`, `if()` and the rest remain in G15.

### Boxes

`display: flow-root` is a block box that establishes a block formatting
context; `inline flow-root` is `inline-block`. The root element's box
is the root of the initial block formatting context, so a child's
margin no longer collapses through it (G11). `aspect-ratio` gives a box
that is not replaced a preferred aspect ratio (css-sizing-4 section
5): an `auto` width or height is worked out from the other, through
the box's sizing (`box-sizing` choosing which box the ratio is of),
in block layout, for inline-blocks, for absolutely positioned boxes and
as flex and grid items' used sizes; its automatic minimum size is its
content's, as section 5.2 says. `background-position-x` and
`background-position-y` become the longhands of `background-position`,
each a list as the other background properties are.

### Conic gradients

`conic-gradient()` and `repeating-conic-gradient()` are rendered as the
other gradients are, by the engine's own code into an image both
backends draw (M23.3's design), each pixel by its angle about the
centre. Gradients take a `<color-interpolation-method>` and interpolate
in its space by D48's rules.

### Bounds

`MAX_SHADOW_MASK_PIXELS` (a shadow's mask; a larger one is worked out at
a smaller scale and stretched, as large gradients are),
`MAX_BLUR_RADIUS` (a blur's standard deviation in device pixels before
the group or mask is scaled down for it), the shadow mask cache's byte
budget, beside the glyph masks', and `MAX_PATH_POINTS` (a path's
flattened points, a radius's segments fewer past it). A filter's
scratch is within `MAX_FRAME_PIXELS`, as every scratch is.

### Fuzzing

`values` takes the colour functions, `color-mix()`, `light-dark()` and
the new math functions, each value it parses serializing to text that
parses to the same value. A new target, `paint_effects`, draws random
rounded boxes, shadows and filtered groups through the software
backend, holding the shared mask and blur code to its bounds and
finding no panics on extreme radii, blurs and spreads.

## 2. Not in this batch, said rather than hidden

- *Relative colour syntax* (`rgb(from red r g b)`) and Color 5's
  `contrast-color()`, `color()`'s custom colour profiles
  (`@color-profile`).
- *Gamut mapping* by OKLCH chroma reduction, and colour management of
  wide-gamut displays (G38).
- *Dashes and dots along a rounded corner* (corners drawn solid).
- *`backdrop-filter`*, *SVG filters by `url()`* (G87), and
  `mask`/`clip-path` shapes beyond SVG's references.
- *Values 5's math* (`progress()`, `random()`, `calc-size()`, `if()`).
- *`display: flow-root list-item`* and `inline list-item` (G19), and
  `display: contents`.
- *The canvas element's `filter`* attribute of 2D contexts (G81).

## 3. Order of work

1. *Colours* (D48): the spaces, `lab()` to `color()`, `color-mix()`,
   serialization, interpolation, `currentcolor` in every colour
   property, the system colours' names.
2. *The colour scheme* (D49): `color-scheme`, its meta element,
   `light-dark()`, the system colours by scheme, the canvas, the
   controls.
3. *Math functions*.
4. *Boxes*: `flow-root`, the root's margins, `aspect-ratio`,
   `background-position-x` and `-y`.
5. *Rounded boxes* (D50): `border-radius`, the `Path` item, backgrounds
   and borders.
6. *Rounded clips* (D51).
7. *Shadows* (D52): `box-shadow`, the shared mask and blur.
8. *Filters* (D52): `filter`, both backends.
9. *Conic gradients* and gradients' interpolation methods.
10. *The acceptance*: WPT's directories, pages against the reference
    browser's captures, T4.3, fuzzing, documents.

## 4. Corrected to the code

What the work found that this note did not say, or said otherwise, by
step:
