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
- `transform-box` (Transforms 1 §9), initial `view-box`: all five
  values parse. `content-box`, and `fill-box`, which for a box with a
  CSS layout box is its content box, put the reference box at the
  content box; `border-box`, and `stroke-box` and `view-box`, which
  without SVG (M32) are the border box, at the border box.
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
  `scroll-position` or `contents`, and, since a name must have the
  effect of the property's other values (§2.2), the properties those
  values make a stacking context or a containing block by: `filter`,
  `backdrop-filter`, `contain` and `offset-path` (both), `position`
  (a containing block for absolute boxes, not fixed ones, and a
  stacking context), `z-index` (a stacking context on a positioned box
  or an item) and the composited properties the engine has not got,
  `clip-path`, `mask` and the rest; the `-webkit-` aliases name what
  they alias. Other names parse and mean nothing.

All of them live behind one pointer, `ComputedStyle::effects:
Option<Arc<Effects>>`, as the grid properties live behind
`GridStyle` (M18): `None` is every initial value, and the style grows
by eight bytes, to 440, which its size test states. The lesson of
M17's T4.3 holds: a transform list and two origins would be fifty
bytes on every box and fragment, and most elements have none. The
parsers follow the grammar of their drafts and the reference browser
where the drafts leave room: `<position>` has no three-value form,
`transform-origin` takes no z after one value, a percentage in `scale`
is a number, and a function with the wrong count of arguments makes
the declaration invalid. Numbers are clamped to the largest length's
magnitude, so that `scale(1e30)` draws what the largest scale draws
and every matrix stays finite.

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

- A transformable box whose `transform` (or `translate`, `rotate` or
  `scale`) is not `none`, whose `perspective` is not `none`, or whose
  `transform-style` is `preserve-3d`, as the reference browser has
  it, or any box whose `will-change` names one of them or another
  such property (above), is a containing block for all its
  descendants, the fixed-positioned ones included (Transforms 1 §2,
  Transforms 2 §9, Will Change 1 §2.2): a `fixed` box inside it is
  positioned against its padding box, as an `absolute` one is, hangs
  under its fragment and scrolls with it. The box tree finds the
  containing block of a fixed box as it finds an absolute one's,
  stopping at such an ancestor; a transformed cell, row, row group or
  caption is one for the positioned boxes in it, and a box in a 3D
  rendering context whose back face is hidden is one too. This is the
  item M11 recorded for M19.
- The scroll extent reaches whatever the content reaches (Transforms 1
  §2, Overflow 3 §2.2), the document's as a scroll container's: a box
  overflowing its parent, a relatively positioned box where it moved
  to, an inline box's border box, and a transformed box by the bottom
  of its bounds through its matrix, the box taken as wide as its
  border box and as tall as what it reaches untransformed, a 3D
  matrix projected first; a box whose matrix is singular adds
  nothing, and the flow's own height still counts, as the reference
  browser has it. Only the bottom is tracked: there is no horizontal
  scrolling of the document (M9), and a scroll container's horizontal
  extent waits with it. Layout measures the document's extent on the
  same walk that makes the fragments' places absolute, so that a page
  of twenty thousand fragments is walked once a layout, not twice.

Such a box is also a stacking context, and so is one with `opacity`
below one or a `will-change` naming any of these (Transforms 1 §2,
Transforms 2 §9, Color 4 §3.2, Will Change 1 §2.2) — the transform
properties on a transformable box alone, so that a `perspective` on a
span makes none — and a box in a 3D rendering context whose back face
is hidden: the paint walk's rule for a stacking context (M11), which
read `z-index` and `position`, reads these too, in one predicate that
layout and paint share, and the box takes its own positioned
descendants with it, which is what makes a transformed subtree one
unit. A positioned descendant whose containing block lies above the
context, an absolutely positioned box inside a faded box that is not
positioned, hangs under the containing block's fragment and names the
context it belongs to, which the paint order hands it to. `overflow`
other than `visible` or `clip`, and `opacity` below one, are grouping
properties (Transforms 2 §9.1): a box with one has a used
`transform-style` of `flat`.

**The display list gains groups.** The list stays flat, in paint order,
and gains two items around a subtree: `PushGroup` and `PopGroup`. A
group carries its matrix, flattened onto the plane, about the box
where the walk has it; its opacity; the key that names it across
frames (`GroupKey`: a stacking context's box by its node and
pseudo-element, the scrolled content of an element, or a piece of the
document's content, below); and whether it is a layer and why
(below). Nothing else: the backends find a group's reach from its
items, and what 3D needs is settled by the walk, which orders a 3D
context's members by depth, flattens their matrices and leaves a
hidden back face out of the steps. The items inside a group are in
the group's own coordinates, which are the coordinates the box would
have had with no transform at all: a rectangle inside a rotated group
is written as the axis-aligned rectangle layout gave it, and the
group's matrix is what turns it. A group inside a group is in the
outer group's coordinates likewise, so the matrices compose from the
inside out. A clip inside a group is an axis-aligned rectangle in the
group's coordinates, intersected with the clips of the same group
only, the consumers intersecting across groups themselves. The paint
walk emits a group around each stacking context that has a matrix, an
opacity below one or a `will-change` that makes a layer, at the point
`stacking()` paints the box, so that the box's background, its
content, and its positioned descendants are all inside it; a box
whose matrix is singular has no steps at all. `Step` gains the same
pair, so that hit testing reads it. Whether the viewport meets a box
is decided through the groups around it, so that a box translated
into view from far below is painted: in a group that only translates
and scales, the document's among them, against what is listed mapped
into the group once, a hair wider for the rounding, rather than each
fragment mapped out.

The list also gains `DisplayItem::Border`, a border solid in one
colour all round as one band between two rectangles: under a scale
its edges fall between pixels, and four strips overlapping at the
corners covered each corner pixel twice. Both backends fill it as one
shape.

**Two ways to draw a group.** The software backend draws a group one
of two ways, and the policy below says which.

*Drawn through.* The group's items are drawn into the raster the group
sits in, under the group's matrix, when that matrix only translates
and scales and the opacity is one. A rectangle is mapped and filled
anti-aliased at its new edges; a quad and a polygon are their points
mapped; a clip is its rectangle mapped, still axis-aligned; text is
shaped at its size and its glyphs rasterized at the size the scale
makes, positioned by the mapped origin, so that scaled text is as
sharp as text of that size; a non-uniform scale fills the glyph's
outline under the scale instead. The pixels are those a raster of the
box at that place and size would give, so a translated box matches
the reference browser as a box laid out there does, and the golden
pages hold it byte for byte. Any other group that is no layer is drawn
from a scratch, cleared, which is not kept: an opacity group whose
matrix only translates and scales into a scratch in the raster's
coordinates, its items drawn through as above, laid over the raster
pixel for pixel with the opacity, which is group opacity (Color 4
§3.2), the group's items not showing through each other; a group
whose matrix does more into a scratch in its own coordinates, a clear
texel all round, resampled into the raster as a layer is (below). A
scratch's sampling does not depend on where the scratch starts, so
that a region of it composited alone, as the partial redraw does, is
the whole's to the bit.

*As a layer.* The group's content is rasterized in its own coordinates
into a pixmap of its own, the layer, which is kept from frame to frame
under the group's key, and drawn into the raster above it by the
group's matrix and opacity: a translation by whole pixels lays it over
the raster pixel for pixel, and any other matrix, affine or
projective, by the backend's own inverse mapping of each destination
pixel into the layer, sampled bilinearly between texel centres, rather
than tiny-skia's pixmap drawing, so that the software and GPU backends
sample alike and a region composited alone is the whole's to the bit.
A layer is rasterized at the scale its matrix gives the plane when
the matrix scales the axes and then turns or reflects them, keeping
them square (rounded to a sixty-fourth, between 1/64 and 64), and
composited with that scale taken out, so that a scaled `will-change`
box is as sharp as one drawn through and a scale with a quarter turn
or a mirror is laid over the raster texel for pixel; a skew and a
projective matrix are rasterized at one. A layer's content is redrawn
only where its own list changed, by M13.3's machinery run on the
layer's pixmap in the layer's coordinates: the list diff, the damage,
the partial redraw kept exact; an unchanged list is kept rather than
copied. A change of the matrix or the opacity alone redraws nothing:
the layer is composited again, which is what an animated transform
(M22) and a scroll cost; a new scale draws it again, so that the
frame stays a whole raster's whatever came before. A layer made anew,
at another place, size or scale, is damage outside it as a layer that
comes or goes is, since its texels can differ from the last ones by a
hair. Rotated and skewed content is resampled from its own plane, as
every compositor's layers are; it is not re-rasterized through the
transform, which the reference browser does for a transform that does
not animate, so a rotated edge or a rotated glyph differs from the
reference browser's by the resampling, and the golden pages with
rotation compare within a tolerance at edges.

**The layerization policy (M19.2).** A group is a layer when any of
these holds, and is drawn through otherwise:

- its matrix, flattened, rotates, skews, reflects, or is projective:
  the software backend's clips are axis-aligned, and inside a layer
  they stay so, while in the frame they would not; and the GPU
  backend's scissor is likewise axis-aligned. A member of a 3D
  rendering context is no layer for being one: a member whose
  flattened matrix only scales, a `translateZ()` under a perspective,
  is drawn through, sharp, where a layer would be resampled;
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
The display list is in document coordinates inside groups that are
the document's content, translated by the negated scroll offset; the
fixed boxes, which do not scroll, and the viewport's scrollbar are
outside them, in viewport coordinates, as they were. The content is
cut into pieces where a fixed box is painted among it, each piece a
group of its own (`GroupKey::Document(n)`, numbered in paint order),
so that a scroll changes the groups' matrices alone; a fixed box
inside a box that contains it scrolls with that box, and one inside
an opacity group, which it cannot leave, is listed in the group at
the place the offset gives it. Each piece is a layer, always, but one
of tiles rather than one pixmap: strips as wide as what of it can
show and 256 pixels tall (`paint::TILE_HEIGHT`), in document
coordinates, each a pixmap of its own, so that the layer does not
hold a page of sixteen thousand pixels at once. A frame composites
the tiles the viewport meets, which at a whole-pixel offset is a copy
of their rows; a tile missing is rasterized first, drawing only the
items whose extent meets it. Paint lists the content of the viewport
and one viewport's height on either side of it, the pre-paint margin,
rounded out to the rows of tiles, so that a scroll within a row lists
the same content; the tiles of the margin not yet drawn are drawn
ahead, nearest first, once the frame is presented, in the time before
the next: the shell calls `Engine::draw_ahead` with the frame's
deadline (M13.4), the deferrable work M13.4 reserved that time for,
and a tile is not begun with less time left than the last one took,
nor when the budget has no room for it without letting anything go;
what is left waits for a later call or for a frame that shows it.
Drawing ahead is outside the frame and its stages. A change in the
document damages, in document coordinates, the tiles it reaches, and
each is redrawn there by M13.3's partial redraw; a tile outside the
margin is dropped. A new viewport width drops every tile.

The damage between two lists now matches their items by a longest
common subsequence (Myers), bounded by an edit limit, so that content
entering or leaving the margin damages the tiles it reaches alone.
And a scroll by whole pixels does not draw the frame again: when the
only groups whose state changed between two lists are layers of tiles
held, all moved by the same whole pixels under the same clip, the
last frame's pixels inside that clip are moved along with them, and
what is drawn again is the strip the move uncovers, what changed
inside the groups where it is now, and whatever else the clip holds
that does not move, under the content or over it — the canvas, a
fixed box, the scrollbar — where it is and where the move took its
pixels, a rectangle by its edges swept along the move alone, its
pixels being its own colour wherever they move. A frame of a scroll
then draws a few rows, not the viewport.

The scroll offset the frame is composited at is a whole pixel: the
engine rounds the offset it draws, as the reference browser snaps a
composited scroll to the pixel grid, while the animation keeps its
fractional state. Hit testing and the fragments' places read the same
rounded offset. This changes what a frame mid-animation shows by less
than a pixel; a settled scroll is a whole number of pixels already.

An element that is a scroll container is painted as it was, its
content moved by its offset inside its clip, unless its `will-change`
names `scroll-position`, when its content is a group that is a layer
of tiles, the same as the document's (`GroupKey::Scroll`). A scroll of
either is a composite.

**The layer budget and eviction (M19.6).** The backend holds at most
`layer_budget` bytes of layer pixmaps, tiles counted singly, 64 MiB
unless the engine is told otherwise (`Engine::set_layer_budget`, which
the shell exposes as `--layer-budget <MiB>`): sixteen frames of 1280
by 800, modest on purpose. When a frame needs more, the layers and
tiles least recently composited go first, never one the frame shows;
a group the budget cannot hold, a layer of tiles whose shown rows do
not fit among them, is drawn through as a group that is no layer, and
the page is right if slower. A lowered budget is kept from the next
frame. `Engine::layer_bytes` reports what is held, and tests hold a
page of a thousand `will-change: transform` boxes to the cap. Which
groups are held under a budget too small for all of them depends on
what was composited when, and on how many tiles of the margin there
was time to draw ahead; a group drawn through rather than composited
can differ at its anti-aliased edges by a level, so under such a
budget two runs of the same frames need not agree to the bit.

**Hit testing.** A point is hit in viewport coordinates; inside a group
it is mapped into the group's coordinates by the inverse of the
group's matrix before the group's boxes are tested, the inverses
composing from the outside in as the walk enters groups; a projective
matrix is inverted on the plane `z = 0`, which is where the group's
content lies. A group whose matrix is singular, or that maps the
point behind its eye, is passed over whole, and a member whose back
face is hidden and turned away has no steps to hit. A clip is checked
in the coordinates of the group it was met in. A scroll container's
chain and the link under the pointer read the same mapping, so that a
scroll container moved by a transform takes the wheel where it is
drawn. The engine hit tests at the offsets it draws at, rounded to
whole pixels.

**3D (M19.5).** A 3D function or `perspective()` makes a 3D matrix; a
box's matrix with `transform-style: flat` on its parent is projected to
the plane, so a `rotateY(60deg)` on a flat page is the parallelogram
the projection gives. A `perspective` on a parent applies to its
children's matrices: the perspective matrix, translated to the
`perspective-origin` and back (Transforms 2 §10), is multiplied before
each child's matrix, so the children recede. The parent is the box's
parent in the document, which each box names (a block hoisted out of
an inline keeps the inline), so that a perspective and a 3D context
reach the box's own children alone, wherever layout put their
fragments. A `transform-style: preserve-3d` box whose parent's is
flat opens a 3D rendering context, and a `preserve-3d` child extends
it: the stacking contexts among the children of either are its
members, each with its accumulated matrix (§5: its own matrix, under
its parent's perspective, under the matrices of the members above
it, up to the context's root; a root with no transform of its own
puts its parent's perspective ahead of its children's), and the walk
takes them from the contexts that would paint them and paints them at
the root's level, around the root's own plane, ordered by the depth
of their transformed centres, furthest first, with no splitting where
two planes cross: a painter's order, v1, as the roadmap asks. Members
at the plane's own depth keep their places among the plane's
positioned descendants, as the reference browser orders what lies in
one plane, and nest in the root's group when their accumulated matrix
is 2D. Every group's matrix is flattened onto the plane for the
steps, the depth kept for the order alone, so that a `translateZ()`
under a perspective reads as the scale it is. A clip flattens the
context under it. A member whose `backface-visibility` is `hidden`
has no steps, so it is neither painted nor hit, when its accumulated
matrix turns it away (§12); such a member is a stacking context and a
containing block. A projective composite in the software backend
maps each pixel of the destination, within what the group's plane
lands on, back through the inverse into the layer and samples it
bilinearly; a plane that the perspective turns past its horizon
inside the frame lands on everything on that side of the horizon,
rather than on nothing, as the frame's far corners would have it. The
GPU backend's shader does the same with perspective-correct
interpolation, clipping a corner behind the eye where the software
leaves it undrawn.

**The GPU backend (M19.3, D3, D8).** `render::gpu::GpuBackend` is a
second `RenderBackend` on `wgpu` (Vulkan, Metal or Direct3D 12),
behind a cargo feature `gpu` that is on by default; the shell draws on
it unless `--software` is given, falling back to the software backend
with a line on the standard error where there is no adapter
(`Engine::new_on`, `Engine::backend`, `Engine::with_backend`), and the
fuzz targets build without it. It draws the same display list with
four pipelines: rectangles as instanced quads whose edges the fragment
shader anti-aliases by the exact share of each pixel inside them,
which the software rasterizer approximates by supersampling, a
`Border` as its outer rectangle less its inner; a border's quads as
plain triangles without anti-aliasing, so that they tile as the
pixel-centre rule tiles them; glyphs and the mitred polygons of
borders as masks over an atlas of 2048-texel pages, the glyph masks
from the shared rasterizer and a polygon's coverage from the same
exact-area accumulation the software backend now blends by, rather
than by multisampling, so that text and mitres are the same coverage
in both; and a layer's or a scratch's texture composited by a
four-by-four matrix with its opacity, sampled bilinearly between
texel centres as the software resamples. The clip is the scissor,
rounded for a glyph and a composite as the software rounds them, and
a rectangle, a quad and a polygon are cut to the clip's geometry
first, as the software cuts them. Layers are textures kept under the
same keys and the same budget, in texture bytes; a whole layer is
drawn again whole where its items changed, a tile where the damage
reaches it. What the two backends share is shared code: the fonts,
the shaped runs and the glyph masks (`GlyphCache`); the layers, the
budget, the marking and the eviction (`Layers`, generic over where a
layer keeps its pixels); the reach, bounds and damage of a list; and
a polygon's coverage. A frame is drawn whole into a texture, every
group that composites drawn into its texture in a pass of its own
before the pass that draws it, the frame's passes submitted at once,
and read back into the frame the trait returns, so the shell's
present path, softbuffer, stays as it is and the toolbar is drawn as
it is (D9); presenting the texture to a surface of the window,
without the readback, waits for the toolbar to draw through the same
backend (M37) and is written down in section 2.

The two backends are held together by an equivalence suite,
`crates/engine/tests/equivalence.rs`: every reference page, every
acceptance layout and twelve pages of the milestone's own (text under
translations, scales, opacity, rotation, skew, perspective,
`preserve-3d`, `will-change`, mitred borders, a fixed header),
rendered by both as loaded, scrolled and scrolled back, and compared
pixel by pixel by `render::gpu::compare`. A pixel whose neighbours in
the software frame are all its own colour, the inside of a shape,
must match within two levels, which is the rounding of blending; a
pixel on an edge, where the neighbours differ, within sixty-four,
which is the difference between tiny-skia's supersampled coverage and
the shader's. The suite runs where an adapter exists, on Mesa's
software Vulkan driver in this environment, and says so and passes
where none does; CI installs the driver on its Linux job. The software
path keeps passing every golden test, the acceptance M19.7 asks for:
byte for byte where the pages draw through, and, for the pages with
rotation or 3D, within the edge tolerance a page declares, a pixel on
an edge in the capture differing by up to it and nothing, or one
level of opacity's rounding, inside.

**The paint-order walk and the engine.** The walk's `Step` gains
`PushGroup` and `PopGroup`; the engine's hit test keeps a stack of
inverse matrices; `Engine::redrawn` says which rectangles a frame drew
anew, and `Engine::layered` which groups it composited from a layer,
which layers it drew into and how many tiles of each, how many tiles
were drawn ahead since, and by how many whole pixels it moved the
last frame's pixels; `Engine::layers` lists the backend's layers with
their reasons and sizes, for the tests of the policy, and
`Engine::layer_bytes` and `Engine::set_layer_budget` read and set the
budget. `Engine::draw_ahead` does the work a frame could do without,
for the caller to call once the frame is presented. The engine rounds
the offsets it draws and hit tests at to whole pixels, while the
scroll animation keeps its fraction, and gives the backend each
frame's list rather than lending it, so that the list the backend
keeps for the next frame's diff is not a copy. The engine no longer
forgets the backend's frame on a scroll. The `ui` crate, which draws
the toolbar with its own software backend, is untouched.

**Performance and the acceptance (M19.7).** A benchmark,
`benches/composite.rs`, measures at 1280 by 800 on both backends: a
scroll of the mutation benchmark's page of a thousand paragraphs, 120
frames, each followed by drawing ahead as the shell does, which must
be composites, no tile rasterized in a frame; a card of twelve, turned
two degrees or moved two pixels a frame through `Engine::mutate` on
its `style` attribute, with and without `will-change: transform`, the
turned one a composite of the layer kept and the moved one drawn
through a partial redraw of its reach; and 200 translated and faded
cards, drawn through, their first frame and a scroll. On the software
backend the scroll takes 2.0–2.6ms a frame, every frame a composite,
and the turned card's frames 11–12ms, most of it the card's layout,
since a change to its `style` lays it out again (section 2). On the
GPU backend, on Mesa's software Vulkan driver, the one adapter here,
the scroll takes 11–13ms a frame and a card's frame 21ms; an
integrated GPU is M19.8. T4.3 against the commit before this
milestone, by M17's method, found the frame drawing ahead to its
deadline, the paint walk projecting every fragment, layout walking
every fragment twice and tiles drawing the whole list, mended; what
remains over its line, the first frame of a page and an appended
paragraph's frame, is written down in `docs/perf-budgets.md` with
M35.10 and M35.9 and M35.4 to undo it. The three fuzz targets that
lay pages out ran fifteen minutes each with the transform and opacity
keywords in a dictionary and the milestone's pages among their seeds;
the pipeline and mutation targets draw ahead after each frame, so
that a tile drawn ahead is held to a whole raster with the rest. Unit
tests hold the backend to its budget under a thousand layers, to
finite matrices under `scale(1e30)` and `matrix3d` of values past
`f32`'s range, and to a bounded time for a page of ten thousand
transformed boxes.

**The web-platform-tests.** The subset gains `css/css-transforms` and
`css/css-will-change` by M17's rule, manual tests left out as
wptrunner leaves them: 325 more tests, 896 in all, of which 866 pass.
The directory's reftests compare a transformed box with an
untransformed one laid out where the transform puts it, which a
drawn-through translation matches exactly and a layer at a
whole-pixel offset does too; the ones with rotation compare against
references that are themselves rotated, and both sides go through the
same layer, or carry the directory's own fuzziness. Of the 18 tests
of the two directories that fail, `expectations.txt` names five as
this milestone's (section 2): three where the planes of a 3D context
cross or form a cycle, which v1 paints whole by depth, and two where
a box resampled from its own plane, a turned one and a `will-change`
layer under a scale, differs from the reference browser's raster
through the transform; and thirteen gaps named at other milestones:
the body's overflow not carried to the viewport, the parser's missing
`tbody`, `vertical-align: top`, inline boxes as containing blocks and
the empty inline box.

**The golden cases.** Ten box-only pages join `tests/reference/`,
numbered 70 to 79, captured and compared as the others are:
translations and scales of bordered boxes and the individual
properties, drawn through; `transform-origin` in its forms; `opacity`
over the canvas, over boxes and nested; rotated and skewed layers;
`perspective` on a parent with its origin and boxes moved along z;
3D rendering contexts; fixed boxes in transformed wrappers; a
transformed box extending the scroll; `will-change` layers beside the
same boxes drawn through; and clips inside transforms. The reference
test takes a declared edge tolerance besides the one everywhere: the
pages with rotation or 3D declare 128 at edges, a layer resampled
from its own plane moving an edge's coverage against the browser's
raster through the transform and never the colour inside, and
nothing, or one level of opacity's rounding, inside; the page that
scrolls leaves the engine's scrollbar out of the comparison, the
capture having none. Four check-layout pages join `tests/layouts/`: a
carousel of cards translated along a row, a stack of panels rotated
about an edge, a faded modal over a page, and a page whose header is
a `fixed` box inside a transformed wrapper; the checker's
`offsetParent` follows CSSOM View as the reference browser does, an
ancestor that contains absolute boxes by a transform or `will-change`
being one.

## 2. Not in this milestone, said rather than hidden

- *Animations and transitions* (M22) are what makes most of this
  matter; this milestone draws a changed matrix cheaply and leaves
  changing it over time to M22. A change of `transform` or `opacity`
  alone, which a transition makes every frame, is today a style
  change like any other: the box is laid out again, as M13's
  invalidation knows no change that only repaints, and the frame of a
  card turned through its `style` attribute spends most of its time
  there (`docs/perf-budgets.md`). A paint-only change that only
  repaints is M35.9's, and M22 needs it.
- *Raster at the device transform*: a layer that a skew or a
  projective matrix carries, or that turns without scaling, is
  resampled from its own plane at its own scale; rasterizing it
  through the transform, which the reference browser does for a
  transform that does not animate, and keeping a layer's raster scale
  through an animation, which the reference browser does for
  `will-change: transform`, are M35.10's and M22's to weigh.
- *Presenting the GPU frame to the window's surface*, without the
  readback, waits for the toolbar to draw through the same backend
  (M37) and the shell's present path to take a surface (M35).
- *The GPU backend's edges*: a texture over the device's longest side,
  which the software backend's pixmap may have, is not drawn; a layer
  of tiles under a matrix that is not a translation is sampled tile by
  tile, each clamped to its own edges, rather than across the seams as
  one; a corner of a projective group behind the eye is clipped by the
  GPU where the software leaves it undrawn; and a device lost
  mid-session is reported, and the backend keeps drawing on it.
- *Sixty frames a second on an integrated GPU*, as M19.7 words it: the
  one adapter here is Mesa's software Vulkan driver, and the GPU
  backend's numbers are taken on it and say so; a run on a machine
  with an integrated GPU is M19.8.
- *Layers above the document's tiles*: a `will-change` box in the
  document's content is a layer composited into the document's tiles,
  so a frame that moves it draws those tiles again where it reaches,
  old and new, and composites them; a layer tree beside the tiles,
  compositing such a layer over them directly, is M35.10.
- *The first frame of a page*, and the one after going back to it,
  draws the rows of tiles the viewport meets, a margin's content
  listed, and composites them, which costs more than drawing the
  viewport once did (`docs/perf-budgets.md`, T4.3). Drawing the group
  through until its tiles are drawn ahead would make a frame's pixels
  depend on how much time there was to draw ahead, since a group
  drawn into transparent tiles and laid over the frame can differ at
  anti-aliased edges by a level from one drawn on it. Tiles drawn
  opaque over the canvas colour, composited as copies, would draw the
  same pixels either way, and are M35.10.
- *The margin is listed with every frame*, not only while the
  document scrolls; on the mutation benchmark's page it costs about a
  tenth of a millisecond of paint and as much of raster.
- *A fragment is 32 bytes larger*, for the stacking context a
  positioned box belongs to, its parent in the document and whether it
  is in a 3D context, and the layout cache copies every fragment of a
  page at each layout; with each fragment carrying its computed style,
  the copies pass the last-level cache on the mutation benchmark's
  page (`docs/perf-budgets.md`). Sharing the computed style rather
  than copying it is M35.9.
- *Plane splitting* where two layers of a 3D context intersect: v1
  orders whole layers by depth. *Nested 3D contexts* beyond a
  `preserve-3d` box inside another are projected flat.
- *`filter`, `mix-blend-mode`, `isolation`, `clip-path`, `mask`* and
  the other grouping properties of Transforms 2 §9.1: none is parsed,
  and `will-change` names them only for the containing blocks and
  stacking contexts their values make; only `overflow` and `opacity`
  group here.
- *`fill-box`, `stroke-box` and a real `view-box`*, which are SVG's
  (M32), stand for the content and border boxes.
- *`will-change: contents`* only keeps a group from being a layer;
  nothing is cached less because of it, since nothing but layers is
  cached.
- *The horizontal scroll extent*: what content reaches is measured
  down the page alone.
- *Tiles of an element scroll container across* its width: tiles are
  strips as wide as what of the content can show; a scroller wider
  than a partial redraw's side is drawn through.
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
