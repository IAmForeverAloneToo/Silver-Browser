# Design note: compositing, transforms and accelerated scroll

Roadmap M19. Written before the code, as the record of the decisions;
what ships is described in `docs/architecture.md` "Current state" when
the code lands. Where this note and the code later disagree, the code
wins and this note is corrected, as the notes before it were.

The problem has three parts. `transform` and `opacity` are declarations
this engine drops: a card rotated a few degrees stands straight, a
box translated across the page sits where layout put it, and a faded
element is as opaque as any other; the pages that do this are
everywhere, and the ones that animate it (M22) cannot be drawn well
without a way to draw their boxes again cheaply. Scrolling redraws the
frame whole: every item of the display list rasterized again for a
one-pixel move, 2.7ms of a scroll frame at 1280 by 800 in release, all
of it work that was done the frame before. And every pixel is drawn by
the CPU, which is right for the lightweight machines this engine is
for and wrong as the only way: the roadmap decided at D3 that a GPU
backend lands behind the same trait, with the software path a
first-class fallback forever. This milestone adds the compositing
model that serves all three: a display list with groups, each a
transform or an opacity, drawn through the parent's raster when that
is cheap and exact, and kept as a layer of its own, rasterized once
and composited by its matrix, when it is not; the document's content
as a layer of tiles that a scroll moves rather than redraws; and a
second backend, on wgpu, that draws the same lists and composites the
same layers, with a suite that holds the two within a tolerance of
each other.

The standards are read at the versions current when this note was
written: CSS Transforms Module Level 1 (Editor's Draft of 30 November
2025) for `transform`, `transform-origin`, the 2D functions and the
rendering model; CSS Transforms Module Level 2 (Editor's Draft of the
same day) for the 3D functions, `transform-style`, `perspective`,
`perspective-origin`, `backface-visibility` and the individual
properties `translate`, `rotate` and `scale`; CSS Will Change Module
Level 1 (Editor's Draft of 29 April 2022); CSS Color Module Level 4
for `opacity`; CSS2 Appendix E, as M11 reads it, for the painting
order the groups fit into; and CSS Overflow Module Level 3 for what a
transformed box adds to the scrollable overflow. Where the reference
browser and the web-platform-tests agree with each other against the
text of a draft, this engine follows them, and says so where it does.

## 1. Decisions

**The properties.** These arrive in the cascade, none of them
inherited:

- `transform: none | <transform-list>` (Transforms 1 §7), a list of
  the functions `matrix()`, `translate()`, `translateX()`,
  `translateY()`, `scale()`, `scaleX()`, `scaleY()`, `rotate()`,
  `skew()`, `skewX()`, `skewY()`, and of Transforms 2 §13's
  `matrix3d()`, `translate3d()`, `translateZ()`, `scale3d()`,
  `scaleZ()`, `rotate3d()`, `rotateX()`, `rotateY()`, `rotateZ()` and
  `perspective()`. Lengths are absolute once computed, as everywhere;
  percentages, which only the translations take, are kept for layout
  and paint, where the box is known. Angles are kept in degrees; `0`
  is accepted where a `<zero>` is. A function the engine does not know
  makes the declaration invalid, as the grammar says, and so does a
  `<number>` where a `<length>` must be.
- `transform-origin` (Transforms 1 §8), initial `50% 50%`, with its
  three forms and its optional third value, a length along z.
- `transform-box` (Transforms 1 §9): `border-box` and `view-box`,
  which for a box without an SVG viewport is the border box, are
  parsed; the others are dropped, there being no SVG (M32). Every
  transform here is of the border box.
- `translate`, `rotate` and `scale` (Transforms 2 §2), initial `none`,
  which compose before `transform`: the transformation matrix is the
  translate, then the rotate, then the scale, then the list (§2.4).
- `transform-style: flat | preserve-3d` (§9), `perspective: none |
  <length [0,∞]>` (§10), `perspective-origin: <position>` (§11),
  initial `50% 50%`, and `backface-visibility: visible | hidden`
  (§12).
- `opacity: <alpha-value>` (Color 4 §3.2), initial `1`, clamped to
  `[0, 1]`; a percentage is a fraction of one.
- `will-change: auto | <animateable-feature>#` (Will Change 1 §2),
  each feature `scroll-position`, `contents` or a property name. The
  engine keeps whether it names `transform`, `translate`, `rotate`,
  `scale`, `opacity`, `perspective`, `transform-style`,
  `scroll-position` or `contents`; other names parse and mean nothing.

All of them live behind one pointer, `ComputedStyle::effects:
Option<Arc<Effects>>`, as the grid properties live behind
`GridStyle` (M18): `None` is every initial value, and the style grows
by eight bytes, to 440, which its size test states. The lesson of
M17's T4.3 holds: a transform list and two origins would be fifty
bytes on every box and fragment, and most elements have none.

**Where a transform's matrix comes from.** `style::transform` owns a
four-by-four matrix of `f32`, column-major as `matrix3d()` is written,
and the arithmetic over it: multiplication, inversion, the matrix of
each function (Transforms 2 §13 gives each), the perspective matrix,
and the projection of a point. It lives in `style` because the matrix
is the transformation matrix of the computed value (Transforms 1 §2),
and because layout and paint, above `style`, both need it. The
matrix of a box is resolved where the box is known, from its border
box: the identity; translated by the computed `transform-origin` (its
percentages of the border box's width and height, its third value
along z); multiplied by `translate`, `rotate`, `scale` and then each
function of `transform` in order, percentages in the translations
against the border box's width and height; and translated back by the
negated origin (Transforms 1 §2's four steps, Transforms 2 §2.4's
order). A singular matrix, `scale(0)` or a `matrix()` of zeros,
renders the box and its content not at all, as the reference browser
does, and is not hit.

**What a transform changes, and what it does not.** A transform does
not affect layout (Transforms 1 §2): the box is laid out where it
would be, and the matrix is applied when it is painted. Two things it
does change in layout:

- A box whose `transform` is not `none`, whose `perspective` is not
  `none`, or whose `will-change` names either, is a containing block
  for all its descendants, the fixed-positioned ones included
  (Transforms 1 §2, Will Change 1 §2.2): a `fixed` box inside it is
  positioned against its padding box, as an `absolute` one is, and
  scrolls with it. The box tree finds the containing block of a fixed
  box as it finds an absolute one's, stopping at such an ancestor.
  This is the item M11 recorded for M19.
- A transformed box extends the scrollable overflow area of the scroll
  containers around it by the rectangle around its transformed border
  box, and never shrinks it (Transforms 1 §2, Overflow 3 §2.2): a box
  translated below the document's end makes the document scroll to
  it. Layout unions the transformed rectangle into the overflow it
  records for each scroll container and for the document, by the
  matrix resolved from the box's own border box; a 3D matrix is
  projected first.

Such a box is also a stacking context, and so is one with `opacity`
below one, `transform-style: preserve-3d` or a `will-change` naming
any of these (Transforms 1 §2, Transforms 2 §9, Color 4 §3.2, Will
Change 1 §2.2): the paint walk's rule for a stacking context (M11),
which today reads `z-index` and `position`, reads these too, and the
box takes its own positioned descendants with it, which is what makes
a transformed subtree one unit. `overflow` other than `visible` or
`clip`, and `opacity` below one, are grouping properties (Transforms
2 §9.1): a box with one has a used `transform-style` of `flat`.

**The display list gains groups.** The list stays flat, in paint order,
and gains two items around a subtree: `PushGroup` and `PopGroup`. A
group carries what its box asked for: its matrix, or none; its
opacity; the key that names it across frames, the box's node and
pseudo-element; the rectangle its content can reach, in its own
coordinates; whether it is a layer and why (below); and, for 3D, its
perspective and whether it preserves 3D and hides its back face. The
items inside a group are in the group's own coordinates, which are
the coordinates the box would have had with no transform at all: a
rectangle inside a rotated group is written as the axis-aligned
rectangle layout gave it, and the group's matrix is what turns it. A
group inside a group is in the outer group's coordinates likewise, so
the matrices compose from the inside out. A clip inside a group is an
axis-aligned rectangle in the group's coordinates, as every clip is
today. The paint walk emits a group around each stacking context that
has a matrix or an opacity below one, at the point `stacking()` paints
the box, so that the box's background, its content, and its positioned
descendants are all inside it; `Step` gains the same pair, so that hit
testing reads it.

**Two ways to draw a group.** The software backend draws a group one
of two ways, and the policy below says which.

*Drawn through.* The group's items are drawn into the raster the group
sits in, under the group's matrix, when that matrix only translates
and scales. A rectangle is a path filled under a tiny-skia transform,
anti-aliased at its transformed edges as the reference browser
anti-aliases them; a quad and a polygon are their points mapped; a
clip is its rectangle mapped, still axis-aligned; text is shaped at
its size and its glyphs rasterized at the size the scale makes,
positioned by the mapped origin, so that scaled text is as sharp as
text of that size; a non-uniform scale rasterizes the glyph's outline
through a path under the transform instead. The pixels are those a
raster of the box at that place and size would give, so a translated
box matches the reference browser as a box laid out there does, and
the golden pages can hold it byte for byte. An opacity group drawn
through is drawn into a scratch pixmap the size of its reach, cleared,
and the scratch is drawn over the raster with the opacity: group
opacity (Color 4 §3.2), where the group's items do not show through
each other. The scratch is not kept.

*As a layer.* The group's content is rasterized in its own coordinates
into a pixmap of its own, the layer, which is kept from frame to frame
under the group's key, and drawn into the raster above it by the
group's matrix and opacity: an affine matrix by tiny-skia's pixmap
drawing with bilinear filtering, a projective one by the backend's own
inverse mapping of each destination pixel (below). A layer's content
is redrawn only where its own list changed, by M13.3's machinery run
on the layer's pixmap in the layer's coordinates: the list diff, the
damage, the partial redraw kept exact. A change of the matrix or the
opacity alone redraws nothing: the layer is composited again, which is
what an animated transform (M22) and a scroll cost. Rotated and skewed
content is resampled from its own plane, as every compositor's layers
are; it is not re-rasterized through the transform, which the
reference browser does for a transform that does not animate, so a
rotated edge or a rotated glyph differs from the reference browser's
by the resampling, and the golden pages with rotation compare within a
tolerance, as the multi-colour page of M11 does.

**The layerization policy (M19.2).** A group is a layer when any of
these holds, and is drawn through otherwise:

- its matrix rotates, skews, or is 3D or projective: the software
  backend's clips are axis-aligned, and inside a layer they stay so,
  while in the frame they would not; and the GPU backend's scissor is
  likewise axis-aligned;
- it takes part in a 3D rendering context (below);
- its box's `will-change` names `transform`, `translate`, `rotate`,
  `scale`, `opacity`, `perspective` or `transform-style`, the author's
  word that the matrix or the opacity is about to change;
- it is the scrolled content of a scroll container: the viewport's
  always, an element's when its `will-change` names `scroll-position`;
- and the layer budget (below) has room for it; a group the budget
  refuses is drawn through, a rotated one through a scratch of its
  reach, resampled, so that the page is right if slower.

A group whose box's `will-change` names `contents` is never a layer.
Nothing else is a layer: a box translated by a few pixels, a card
scaled on hover (M26), a faded panel, are all drawn through, exact and
cheap, and the common page makes no layer but the document's. The
policy is a function of the computed style and the matrix alone, so a
page's layers are the same from frame to frame until its styles
change, which is what keeps them cached. The reference browser's
policy is richer, and changes between versions; this one is written
down here, and `Engine::layers` tells a test what it decided.

**The document as a layer of tiles; composited scrolling (M19.4).**
The display list is in document coordinates inside a group that is the
document's content, translated by the negated scroll offset; the fixed
boxes, which do not scroll, and the viewport's scrollbar are outside
it, in viewport coordinates, as they are now. The group is a layer,
always, but one of tiles rather than one pixmap: strips the viewport's
width and 256 pixels tall, in document coordinates, each a pixmap of
its own, so that the layer does not hold a page of sixteen thousand
pixels at once. A frame composites the tiles the viewport meets, which
at a whole-pixel offset is a copy of their rows; a tile missing is
rasterized first, from the layer's list clipped to it. Paint lists the
content of the viewport and one viewport's height on either side of
it, the pre-paint margin, rather than the viewport alone, so that the
tiles a scroll brings in next are drawn before they show: after the
frame's own tiles are composited, while the frame has time before its
deadline (M13.4), the backend rasterizes the nearest missing tile in
the margin, and stops when it has none, which is the deferrable work
M13.4 reserved its deadline for. A change in the document damages, in
document coordinates, the tiles it reaches, and each is redrawn there
by M13.3's partial redraw; a tile outside the margin is dropped. A
new viewport width drops every tile.

The scroll offset the frame is composited at is a whole pixel: the
engine rounds the offset it draws, as the reference browser snaps a
composited scroll to the pixel grid, while the animation keeps its
fractional state. Hit testing and the fragments' places read the same
rounded offset. This changes what a frame mid-animation shows by less
than a pixel; a settled scroll is a whole number of pixels already.

An element that is a scroll container is painted as it is now, its
content moved by its offset inside its clip, unless its `will-change`
names `scroll-position`, when its content is a group that is a layer:
one pixmap of the content's extent, or tiles of it past a viewport's
height, the same as the document's. A scroll of either is a composite.

**The layer budget and eviction (M19.6).** The backend holds at most
`layer_budget` bytes of layer pixmaps, tiles counted singly, 64 MiB
unless the engine is told otherwise (`Engine::set_layer_budget`, which
the shell exposes as `--layer-budget`): sixteen frames of 1280 by 800,
modest on purpose. When a frame needs more, the layers and tiles
least recently composited go first, and a tile the viewport shows is
never evicted for one it does not; a group the budget cannot hold at
all is drawn through, and the document's own visible tiles, which the
frame cannot do without, are held under a budget however small, the
frame falling back to today's whole raster below one viewport of
tiles. `Engine::layer_bytes` reports what is held, and tests hold a
page of a thousand `will-change: transform` boxes to the cap.

**Hit testing.** A point is hit in viewport coordinates; inside a group
it is mapped into the group's coordinates by the inverse of the
group's matrix before the group's boxes are tested, the inverses
composing from the outside in as the walk enters groups; a projective
matrix is inverted on the plane `z = 0`, which is where the group's
content lies. A group whose matrix is singular, or whose back face is
hidden and turned away, is passed over. A scroll container's chain and
the link under the pointer read the same mapping.

**3D (M19.5).** A 3D function or `perspective()` makes a 3D matrix; a
box's matrix with `transform-style: flat` on its parent is projected to
the plane, so a `rotateY(60deg)` on a flat page is the parallelogram
the projection gives. A `perspective` on a parent applies to its
children's matrices: the perspective matrix, translated to the
`perspective-origin` and back (Transforms 2 §10), is multiplied before
each child's matrix, so the children recede. A `transform-style:
preserve-3d` box opens a 3D rendering context: its children's matrices
accumulate through it (§5, the accumulated matrix: each one's own
matrix, the offset from its parent, and the parent's perspective, up
to the context's root), and the children are composited into the
context by their accumulated matrices, ordered by the depth of their
transformed centres, furthest first, with no splitting where two
planes cross: a painter's order, v1, as the roadmap asks. Each child
of a 3D context is a layer. A layer whose `backface-visibility` is
`hidden` is not composited, nor hit, when its accumulated matrix has a
negative component in row 3, column 3 (§12). A projective composite in
the software backend maps each pixel of the destination, within the
projected quadrilateral, back through the inverse into the layer and
samples it bilinearly; the GPU backend's vertex shader does the same
with perspective-correct interpolation.

**The GPU backend (M19.3, D3, D8).** `render::gpu` is a second
`RenderBackend` on `wgpu`, behind a cargo feature `gpu` that is on by
default, chosen by the shell when an adapter exists and
`--software` is not given, and by the engine's tests when one exists.
It draws the same display list: a solid pipeline for rectangles, quads
and polygons, as triangles (a polygon fanned from its ears), the
rectangles' edges anti-aliased by the fragment shader from their
distance to the edge, the quads not at all (the GPU's pixel-centre
rule is the software backend's), the polygons by multisampling; a text
pipeline that draws each glyph as a quad over an atlas texture holding
the same masks the software backend blits, from the same rasterizer,
so that text is the same coverage in both; a composite pipeline that
draws a layer's texture by a four-by-four matrix, with its opacity and
bilinear sampling; clips as scissor rectangles, rounded as the
software clip rounds them; and layers as textures drawn in their own
render passes, kept under the same keys and the same budget, in
texture bytes. In this version the backend renders into a texture and
reads it back into the frame the trait returns, so the shell's present
path, softbuffer, stays as it is and the toolbar is drawn as it is
(D9); presenting the texture to a surface of the window, without the
readback, waits for the toolbar to draw through the same backend (M37)
and is written down in section 2. On an integrated GPU the readback of
a 1280 by 800 frame is a fraction of a millisecond.

The two backends are held together by an equivalence suite: every
reference page and every page of this milestone rendered by both,
compared pixel by pixel. A pixel whose neighbours in the software frame
are all its own colour, the inside of a shape, must match within two
levels, which is the rounding of blending; a pixel on an edge, where
the neighbours differ, within sixty-four, which is the difference
between tiny-skia's supersampled coverage and the shader's. The suite
runs where an adapter exists, on Mesa's software Vulkan driver in this
environment, and says so and passes where none does; CI installs the
driver on its Linux job. The software path keeps passing every golden
test byte for byte, the acceptance M19.7 asks for.

**The paint-order walk and the engine.** The walk's `Step` gains
`PushGroup` and `PopGroup`; the engine's hit test keeps a stack of
inverse matrices; `Engine::redrawn` says, besides the rectangles a
frame drew anew, which layers it composited and which it rasterized;
`Engine::layers` lists the frame's layers with their reasons, for the
tests of the policy. The engine no longer forgets the backend's frame
on a scroll. The `ui` crate, which draws the toolbar with its own
software backend, is untouched.

**Performance and the acceptance (M19.7).** A benchmark,
`benches/composite.rs`, measures at 1280 by 800: a scroll of the
mutation benchmark's page of a thousand paragraphs, 120 frames, which
must be composites, no tile rasterized once the margin is drawn; a
page of twelve cards, one rotating by two degrees a frame through
`Engine::mutate` on its `style` attribute, with and without
`will-change: transform`, where the layer's frame must be a composite
and the drawn-through one a partial redraw of its reach; a page of 200
translated and faded cards, drawn through; and the same three on the
GPU backend where there is an adapter. Sixty frames a second is 16.7ms
a frame, and the scroll and the layered rotation must be inside it on
both backends; the numbers go in `docs/perf-budgets.md`, with T4.3's
comparison against the commit before this milestone by M17's method.
The three fuzz targets that lay pages out run fifteen minutes each
with the transform and opacity keywords in a dictionary and the
milestone's pages among their seeds; the pipeline target's check that
a partial redraw equals a whole raster now covers groups, layers and
tiles. Unit tests hold the backend to its budget under a thousand
layers, to finite matrices under `scale(1e30)` and `matrix3d` of
infinities, and to a bounded time for a page of ten thousand
transformed boxes.

**The web-platform-tests.** The subset gains `css/css-transforms` by
M17's rule, with nothing added to it for this milestone, and
`expectations.txt` gains a section for each step that will make
failing tests pass, emptied as the steps land. The directory's
reftests compare a transformed box with an untransformed one laid out
where the transform puts it, which a drawn-through translation matches
exactly and a layer at a whole-pixel offset does too; the ones with
rotation compare against references that are themselves rotated, and
both sides go through the same layer, or carry the directory's own
fuzziness. `css/css-will-change` is small and comes too.

**The golden cases.** Ten box-only pages join `tests/reference/`,
numbered 70 to 79, captured and compared as the others are, the pages
with rotation or 3D at a stated tolerance: translations and scales of
boxes with borders, drawn through; `transform-origin` in its forms;
`opacity` over backgrounds and over other boxes, nested; a rotated and
a skewed layer; `perspective` on a parent and `rotateX` children;
`preserve-3d` with two planes; `fixed` inside a transformed box; a
transformed box extending the scroll; `will-change` layers that must
look no different from drawn-through boxes; and clips inside transforms.
Four check-layout pages join `tests/layouts/`: a carousel of cards
translated along a row, a stack of panels rotated about an edge, a
faded modal over a page, and a page whose header is a `fixed` box
inside a transformed wrapper.

## 2. Not in this milestone, said rather than hidden

- *Animations and transitions* (M22) are what makes most of this
  matter; this milestone draws a changed matrix cheaply and leaves
  changing it over time to M22.
- *Raster at the device transform*: a layer that rotates, or that a 3D
  matrix scales up, is resampled from its own plane; rasterizing it
  through the transform, which the reference browser does for a
  transform that does not animate, and re-rasterizing at a new scale
  when an animation settles, is M35's raster work.
- *Presenting the GPU frame to the window's surface*, without the
  readback, waits for the toolbar to draw through the same backend
  (M37) and the shell's present path to take a surface (M35).
- *Plane splitting* where two layers of a 3D context intersect: v1
  orders whole layers by depth. *Nested 3D contexts* beyond a
  `preserve-3d` box inside another are projected flat.
- *`filter`, `mix-blend-mode`, `isolation`, `clip-path`, `mask`* and
  the other grouping properties of Transforms 2 §9.1: none is parsed;
  only `overflow` and `opacity` group here.
- *`transform-box` other than the border box*: `fill-box`,
  `stroke-box` and a real `view-box` are SVG's (M32).
- *`will-change: contents`* only keeps a group from being a layer;
  nothing is cached less because of it, since nothing but layers is
  cached.
- *Tiles of an element scroll container across* its width: tiles are
  strips the content's width; a scroller wider than the budget's share
  is drawn through.
- *Scroll snapping, `scroll-behavior`, overscroll* (M26, M37).
- *Composited layers in the ui crate's toolbar* (D9, M37).
- *Fragmentation* of transformed boxes (M20) and the resolved values
  of `transform` for `getComputedStyle` (M25).
- *What the WPT subset leaves out*, among it the Ahem font, writing
  modes, scripts, images, animations and `filter`: the tests that need
  them wait for them, and the rule picks them up when they land.

## 3. Order of work

Each step is a vertical slice: build, tests, clippy green and a smoke
run before the next, with its sabotage pass. A step is done when its
tests pass.

1. *Style.* The properties and functions above into `Effects` behind
   the pointer; `style::transform`'s matrix, the matrix of each
   function, the origin, the perspective matrix, inversion and
   projection, tested on their own; the size of `ComputedStyle` stated
   by its test; `supports()` answering for the new names. Nothing
   draws yet.
2. *Layout.* The containing block of fixed and absolute boxes under a
   transformed, perspective or `will-change` ancestor; the scrollable
   overflow extended by transformed bounds; the stacking-context rule
   extended, in one predicate paint and layout share.
3. *Groups, drawn through.* `PushGroup` and `PopGroup` in the display
   list and in `Step`; the walk emitting them; the software backend
   drawing a translated or scaled group through, text rasterized at
   its scale, and an opacity group through a scratch; the damage of
   a changed item mapped by its groups; the exactness tests over
   groups. Here `transform: translate()` and `opacity` first draw.
   M19.1 and half of M19.2.
4. *Hit testing* through groups: the inverse stack in the engine, the
   scroll chain and the link under the pointer.
5. *Layers.* The layer cache under the group's key; a rotated or
   skewed group as a layer, composited by tiny-skia; the layerization
   policy and `Engine::layers`; the document's content as a layer of
   tiles and the whole-pixel scroll; the pre-paint margin and the
   deadline; `will-change` promotions, the element scroller's layer;
   the budget and eviction; `Engine::redrawn` extended. M19.2, M19.4
   and M19.6.
6. *3D.* The 3D functions in the matrix; perspective on a parent;
   `preserve-3d` contexts with their accumulated matrices and depth
   order; `backface-visibility`; the projective composite and the
   planar inverse for hit testing. M19.5.
7. *The WPT subset*: `css/css-transforms` and `css/css-will-change`
   pulled by the rule, the expectations in sections by the steps to
   come.
8. *The GPU backend*: `render::gpu` on wgpu behind the `gpu` feature;
   the four pipelines, the atlas, the layers as textures, the readback;
   the shell's choice and `--software`; the equivalence suite over
   every reference page, run where an adapter exists; CI's driver.
   M19.3.
9. *The golden cases* and the check-layout pages, and whatever they
   find.
10. *Performance and fuzzing*: `benches/composite.rs`, the 60fps
    acceptance on both backends, T4.3, the fuzz runs with the new
    dictionary and seeds. M19.7.
11. *The documents*: this note corrected to the code, the
    architecture's M19 entry, the roadmap's ticks (M19.1 to M19.7, D3
    and D8 decided), and the numbers in `docs/perf-budgets.md`.
