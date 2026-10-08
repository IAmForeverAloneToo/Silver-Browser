# Design note: images

Roadmap M23. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ended: where they
disagreed, the code won, and the places are marked *(corrected)*. What
ships is described in `docs/architecture.md` "Current state".

No page shows a picture. An `<img>` is an empty inline box that makes
no line; `background` keeps its colour and drops the whole shorthand
when it names an image; `list-style-image` is parsed and never drawn;
`content: url()` is dropped. Nothing fetches an image but the favicon,
which nothing shows. There is no decoder, no image type, no display
item that carries pixels, and no path by which an image arriving could
change a layout. This milestone adds decoding of PNG (with APNG), JPEG,
GIF and WebP; `<img>` as a replaced element with its alt text; CSS
backgrounds, border images, list-style images, `content: url()`,
`image-set()` and linear and radial gradients; animated images on the
document timeline; and the bounds that keep hostile images in check.

The standards are read at the versions current when this note was
written:

- the HTML Standard (Living Standard of 7 October 2026), sections
  4.8.3-4.8.4 for `img` and the image processing model, 2.5.7 for lazy
  loading, 15.4.2-15.4.3 for rendering images and their dimension
  attributes;
- CSS Images Level 3 (Editor's Draft of 15 December 2025) for gradients,
  the sizing of replaced content, `object-fit`, `object-position`,
  `image-orientation` and `image-rendering`;
- CSS Images Level 4 (Editor's Draft of 18 September 2026) for
  `image-set()`;
- CSS Backgrounds and Borders Level 3 (Editor's Draft of 16 December
  2025) for the `background-*` and `border-image-*` properties;
- CSS Lists Level 3 (Editor's Draft of 28 April 2026) for
  `list-style-image`;
- CSS Generated Content Level 3 (Editor's Draft of 19 August 2026) for
  `content: url()`;
- CSS Color Level 4 (Editor's Draft of 7 October 2026) section 13.2 for
  interpolating legacy colours;
- PNG Third Edition (Recommendation of 24 June 2025) for APNG;
- GIF89a (1990) and RFC 9649 (November 2024) for GIF and WebP frames;
- the Fetch Standard (Living Standard of 6 October 2026) and Mixed
  Content (Candidate Recommendation Draft of 23 February 2023) for
  fetching images.

The texts are kept in `silver-handover/specs` beside the repository.

What the drafts leave to rendering was put to the reference browser
(Chrome 154.0.8037.98, headless) before this note was written:

- a loaded 200 by 100 image is 200 by 100;
- with `width="100"` alone it is 100 by 50, and with `height="80"`
  alone 160 by 80: one attribute keeps the image's ratio;
- with both attributes, the computed `aspect-ratio` is `auto 300 / 300`;
- an image that is broken, has no `src`, or has an empty `src`, and
  has alt text, is an inline box of the text with a 16 by 16 icon
  before it; its `width` and `height` attributes do not apply;
- with no `alt` it is a 16 by 16 icon, and with `alt=""` nothing;
- a gradient between hex colours interpolates in gamma-encoded sRGB with
  premultiplied alpha (midway from `#f00` to `#00f` is 127, 0, 128),
  and in Oklab only when asked to;
- a GIF frame delay of 10ms or less is shown for 100ms. No standard says
  so: the three engines' sources do, and so does this one.

What the code found the reference browser does besides, probed while it
was written:

- a broken image with no `alt` is 16 by 16 unless both its width and its
  height are given, by attribute or style; one side is not enough;
- `image-set()` takes the option of the smallest resolution at or above
  1x, else the largest, the first of equal ones;
- an image marker sits with its bottom on the baseline, 7px from the
  text, inside or outside; a gradient's marker is a square half the
  font's ascent;
- `image-orientation: none` does nothing to an image of another origin,
  and a `file:` page's images are all of another origin;
- a `pixelated` sample exactly between two texels takes the lower;
- a colour hint is drawn as nine stops along its curve, Blink's
  approximation, not the curve itself;
- a border image still coming draws nothing where the border is; a
  broken one gives the border.

## 1. Decisions

**A crate of its own (M23.1).** A new leaf crate, `images`, holds the
decoders, the `Image` type, frame composition, the decode bounds, frame
timing (`frame_at`) and the box filter of downscaled copies
(`downscale`). *(corrected)* There is no `ImageDecoder` trait: D6's
"per-format crates behind a trait" is two functions, `read_header` and
`decode`, which dispatch on the sniffed format, and that was enough. It depends on no crate of the
engine's. Layout reads an image's natural size and orientation, paint
puts an image in a display item, and render draws its pixels, so the
type must sit below all three. The layering becomes
`layout -> {style, text, images}` and `paint`, `render -> images`, and
`docs/architecture.md` says so. `engine` owns fetching and decoding.

**The decoders.** PNG and APNG by `png` 0.18; JPEG by `zune-jpeg` 0.5
with `default-features = false, features = ["std"]`, which leaves out
its SIMD code and with it every `unsafe` block; GIF by `gif` 0.14 with
`std` alone; WebP, lossy, lossless and animated, by `image-webp` 0.2.
All are MIT or Apache-2.0, carry no advisory, and add no second version
of any crate the tree has, once tiny-skia's own PNG feature, which
nothing calls, is turned off (`default-features = false, features =
["std", "simd"]`). `jpeg-decoder`, which the roadmap named, is in
maintenance mode in favour of `zune-jpeg`. EXIF orientation is read by
a short reader of the engine's own: tag 0x0112 of IFD0, nothing else.

AVIF is not decoded. `ravif`, which the roadmap named, only encodes.
Every AV1 decoder in Rust either fails `deny.toml` (a CC0 licence and an
unmaintained-crate advisory among `rav1d`'s dependencies), offers only
an unsafe C interface, or needs NASM to build on Windows. An AVIF image
is broken, and a `type("image/avif")` option of `image-set()` is
skipped as unsupported, so pages that offer a fallback get it.

**Decoding is isolated.** The decoders are third-party and are known to
panic on hostile input, so every decode runs on a decode thread inside
`catch_unwind`. A panic, an error or a bound exceeded makes the image
broken. Nothing a decoder does reaches the main thread but its result.

**Bounds.** Named constants, in `images` but for the last two, which
are the engine's *(corrected)*:

- `MAX_IMAGE_BYTES`, 32 MiB: an encoded image. The fetch stops reading
  past it, rather than reading up to the 64 MiB body limit first.
- `MAX_IMAGE_SIDE`, 16,384 pixels: either side.
- `MAX_IMAGE_PIXELS`, 2^25 (about 33.5 million): width times height,
  checked from the header before anything is allocated.
- `MAX_FRAMES`, 1,000: frames of an animation.
- `MAX_ANIMATION_BYTES`, 256 MiB: the composed frames of an animation.
  An animation over it shows its first frame alone.
- `MAX_DECODED_BYTES`, 512 MiB: decoded pixels kept by the engine,
  dropped least recently drawn first and decoded again when needed.
- `MAX_DECODE_THREADS`, 4.

The decoders' own limits are set from these too: `png`'s byte limit,
`gif`'s memory limit and `zune-jpeg`'s maximum sides. `image-webp`'s
limit is partial, so its header is checked first like every other's.

**Two decodes (M23.5).** When an image arrives, only its header is read:
its size and its orientation *(corrected: not whether it animates,
which nothing needs before the pixels)*. That is enough for layout. Its pixels are decoded the first time a frame draws it, off the
main thread. That frame draws nothing in its place, and the next one
draws the image. This is `decoding="async"` for every image. A script
could tell `sync` from `async`; nothing here can.

Downscaling on decode is narrower than the roadmap hoped. `zune-jpeg`
has no scaled decode. Instead *(corrected)*, the frame that first draws
an image knows the size it is drawn at, its tile mapped through its
groups' matrices, and an image drawn at less than half its natural size
both ways is decoded on the decode thread and kept downscaled to twice
that size, by a box filter, the full decode never kept. "Largest" is the
largest ever drawn: a copy since drawn larger than it was made for is
decoded again, drawn stretched meanwhile. That bounds what a page of
huge thumbnails keeps, but not the peak of one decode, which
`MAX_IMAGE_PIXELS` bounds. A print decodes whole.

The decoded budget is checked after each frame: the pixels drawn
longest ago are dropped, never those the frame drew, so one frame's
images may hold more than it. `Engine::decoding()` says a decode is
under way, for a caller that wants a frame with the images drawn.

**Fetching (M23.1, M14).** Images are fetched by the loader, with a new
target and the destination "image", in no-cors mode, with Fetch's
`Accept` value for images.
- **Mixed content:** an `http:` image of an `https:` page is upgraded to
  `https:` unless its host is an IP address (Mixed Content section 4.1),
  as `docs/design/net.md` left to this milestone. Other `http:`
  subresources stay blocked.
- **Content type:** the image's type is sniffed from its bytes. The
  sniffer's check of a supported image type, a stub until now, is
  filled in. `nosniff` and the HTTP status do not decide anything: HTML
  says the status is ignored, and Fetch says enforcing `nosniff` for
  images was not web-compatible.
- **Sharing:** one URL is fetched once per document, and every element
  naming it shares one image, as HTML's list of available images does.
- **Back and forward:** a page kept in the back-forward cache keeps its
  images.
- **Loaded:** `LoadState::Loaded` now also waits for the document's
  images, as the load event does. The page is still shown as soon as
  its document and its render-blocking sheets are in. Images arrive
  later and lay their boxes out again. The tests of the web-platform
  suite wait for `Loaded`, so they see images loaded.

`loading="lazy"` loads at once. HTML defines lazy loading as off when
scripting is disabled (section 2.5.7), and scripts are M24.

**An image arriving.** An `<img>`, a `content: url()` or a list-style
image changes its box's size when its image arrives or breaks.
- **Invalidation:** the engine marks each `img` showing that URL as
  changed in a way layout reads, and the layout cache drops those boxes.
  *(corrected)* Which boxes show a marker's or a `content`'s image is not
  kept: such an arrival, rarer, drops the whole layout cache.
- **No re-layout for backgrounds:** a background or border image changes
  no layout. Paint reads the image when it paints, so its arrival draws
  a frame and lays nothing out.

**`<img>` (M23.2).** An `img` is a replaced element. Which content it
shows follows HTML section 15.4.2, as the reference browser draws it:

| The element | What is drawn |
| --- | --- |
| Has an image | The image, a replaced box sized by its natural size |
| Image still coming | An empty replaced box of the size its attributes and style give |
| Broken or no `src`, with alt text | A non-replaced inline box of the text, after a 16 by 16 icon; `width` and `height` do not apply |
| Broken, no `alt` | A 16 by 16 replaced box of the icon, unless both its width and height are given |
| `alt=""`, or no `src` and no `alt` | Nothing: a replaced box of 0 by 0 |

- **The icon** is drawn by the engine, a box and a cross: the reference
  browser's is its own artwork.
- **The attributes:** `width` and `height` are presentational hints for
  `width` and `height`. When both parse, they also give `aspect-ratio:
  auto w / h` (section 15.4.3).
- **A known difference:** a broken `img` with no `alt` and `display:
  block` is 16 wide here, the line's width in the reference browser,
  whose icon sits in a block container.

**`aspect-ratio`.** The property is added for replaced elements, with
`auto`, `<ratio>` and both together. A natural ratio wins under `auto`.
On other boxes it computes but does not size: that is a gap of its
own, listed in section 2.

**Sizing replaced elements.** CSS Images 3 section 4.3's default sizing
algorithm, with a default object size of 300 by 150, everywhere a box
is sized:
- **Block and inline:** as an atomic inline.
- **Flex:** its case B, which `docs/design/flexbox.md` left to this
  milestone.
- **Grid:** items that are replaced.
- **Intrinsic sizes:** a replaced box contributes its size.

**Orientation and fit.**
- **`image-orientation`:** `from-image` (the initial value) and `none`.
  Natural sizes are taken after rotation. The deprecated angles and
  `flip` are not taken. *(corrected)* `none` applies to an image of the
  document's origin alone (section 5.3); an image of another origin keeps
  its orientation. It does not apply to backgrounds, which are drawn
  oriented.
- **`object-fit` and `object-position`:** as section 4.5-4.6 says, the
  image clipped to the content box.
- **Printing:** a replaced box is monolithic when paginated, as
  `docs/design/media.md` said it is and `is_monolithic` forgot.

**CSS image values (M23.3).** `<image>` computes to a URL, a gradient
or an `image-set()`.
- **URLs:** a URL is resolved against its stylesheet's URL, or the
  document's for a `style` attribute. *(corrected)* `css::Stylesheet` is
  unchanged: the loader makes every `url()` and `image-set()` string of a
  fetched sheet absolute when it parses it, custom properties' values
  included; the style crate keeps a URL as it gets it, and the engine
  resolves what reaches it against the document's base with `net`.
- **`image-set()`:** *(corrected)* chooses the option of the smallest
  resolution at or above 1x, else the largest, the first of equal ones,
  after dropping options with an unsupported `type()` (Images 4 section
  2.4), as the reference browser does. An option's resolution is its
  image's natural resolution: a 40 by 20 image at 2x is 20 by 10.
  `-webkit-image-set()` is its alias.
- **The `attr()` taint:** an attribute's value never becomes a URL
  (`docs/design/values.md`). *(corrected)* The rule is per value: an
  `attr()` whose value parses to a URL fails where it is substituted; a
  declaration is invalid when an `attr()`, or a `var()` of a custom
  property that read one (found to a fixed point), sits in an
  `image-set()`. A literal `url()` beside an `attr()` stands.

**Backgrounds.**
- **The longhands:** `background-image`, `-repeat` (`repeat`, `space`,
  `round`, `no-repeat`, per axis), `-position` with its 3- and 4-value
  forms, `-size` (`auto`, `cover`, `contain`, lengths), `-origin`,
  `-clip` and `-attachment`. Not Backgrounds 4's `background-position-x`
  and `-y` (G40).
- **The shorthand:** `background`, in full.
- **Layers:** the first is on top. The colour goes under them all,
  clipped by the bottom layer's `background-clip`. The root's and the
  body's backgrounds propagate to the canvas, images with colours.
  *(corrected)* The canvas's images are painted first in the document's
  group, placed by the root's boxes, over the document from its top to
  its last top-level box and a viewport more: the same area whatever the
  scroll, so that a scroll composites them from the document's layer.
- **Attachment:** *(corrected)* `fixed` is placed against the viewport at
  the scroll offset painted, inside the document's group, not in a group
  of its own: a scroll draws it again rather than compositing it. `local`
  is placed in a scroll container's content, moved by its offset and as
  tall as it scrolls.

**Repeated images.**
- **One item, not one per tile:** a repeated image is one display item,
  a pattern, not one item per tile. A 1px image repeated over a long
  page stays one item.
- **`space` and `round`** are solved in paint to a tile size and gap.
  Tiles with gaps are drawn one by one, up to 4,096 a pattern; past
  that the gaps are left out.
- **Number of layers:** `MAX_IMAGE_LAYERS`, 256, bounds the layers a
  box may have.

**`border-image`.** `border-image-source`, `-slice` (with `fill`),
`-width`, `-outset` and `-repeat`, and the shorthand, drawn by
Backgrounds 3 section 6's nine-slice algorithm *(corrected)* in the
border's place. `border` resets them; `border-top` and the others do
not. A broken or `none` image gives the box its ordinary borders, and
one still coming draws nothing there. The outset is ink overflow: it
affects no layout. A slice is a display item's crop of the image
(`ImageRef::crop`, fractions of the image, put on whole pixels by the
backends), so a gradient's one-pixel strip cuts too; a raster image's
slices are whole image pixels.

**Markers and content.**
- **`list-style-image`:** an `<image>`. *(corrected)* Its marker is the
  image at its natural size, its bottom on the baseline, 7px from the
  text, inside or outside; a gradient's is a square half the font's
  ascent. `list-style-type` stands in until the image loads or when it
  breaks; a marker's own `content` wins.
- **`content: url()`:**
  - **Alone,** with its alt after `/`, it makes the element replaced,
    a table display taken as a block's. An image that cannot be shown is
    0 by 0.
  - **In a list of content,** the image is an inline replaced box, and
    one that cannot be shown is skipped. Content 3 allows the skip or an
    icon; the skip is the simpler of the two.
- **`<image>` in a typed `attr()`:** stays invalid, for the taint.

**Gradients.** `linear-gradient()`, `radial-gradient()` and their
`repeating-` forms, with Images 3 section 3's geometry and colour-stop
fixup, and transition hints; *(corrected)* the reference browser's
`-webkit-linear-gradient()` and `-webkit-repeating-linear-gradient()`
too, not `-webkit-radial-gradient()`. A stop takes two positions.
- **Interpolation:** colours interpolate in gamma-encoded sRGB with
  premultiplied alpha, as CSS Color 4 section 13.2 says for legacy
  colours and as the reference browser draws them. Legacy colours are
  the only ones the engine parses (G16). A
  `<color-interpolation-method>` makes the gradient invalid.
- **Rendering:** a gradient is rendered by the engine's own code into
  an image the size of its tile, once, and drawn as an image by both
  backends. So both draw the same pixels, and the equivalence suite
  holds them to it without a tolerance for shaders. *(corrected)* A
  straight linear one is rendered one pixel thick and stretched; any
  other at most `MAX_GRADIENT_PIXELS` (2^22), stretched; a large one a
  band of rows a thread (scoped threads, at most 8). The engine keeps
  them by gradient and size under 64 MiB. Within one level of the
  reference browser's, two at a transparent end.
- **Hints:** *(corrected)* drawn as the reference browser draws them,
  nine stops along the curve (Blink's
  `ReplaceColorHintsWithColorStops`), which differs from the curve by
  up to 14 levels.
- **Degenerate gradients:** a repeating gradient whose period is too
  small to draw paints its average colour (section 3.3).
- **Bound:** `MAX_GRADIENT_STOPS`, 256, bounds a gradient's stops.

**Paint and render.**
- **Display items:** paint gains `DisplayItem::Image`, an image placed
  and clipped, with its rendering mode, and `DisplayItem::Pattern`, an
  image tiled over an area with a tile size and gap. Both carry an
  `ImageRef`, the image's number and the frame shown, so two display
  lists compare cheaply and an animation's next frame is a change of
  one item.
- **Software backend:** draws images and patterns through tiny-skia's
  pattern shader, bilinear, or nearest for `image-rendering: pixelated`
  and `crisp-edges`, filling the pixels whose centres are inside.
  *(corrected)* A nearest sample exactly between two texels takes the
  lower, as the reference browser's does (`NEAREST_NUDGE`, 1/1024 of a
  texel, on both backends).
- **GPU backend:** uploads each image to a texture, kept within its own
  least-recently-used budget of 256 MiB, never dropping one the frame
  drew, and draws it *(corrected)* through the composite pipeline, not a
  fifth.
- **Colour:** decoded pixels are premultiplied 8-bit sRGB. Embedded
  colour profiles, `gAMA` and `cICP` are not applied: section 2.
- **Printing:** the print path draws images at its 300 dpi, from the
  same decoded pixels, decoded at once; *(corrected)* a gradient is
  rendered at its tile's CSS size and scaled, and an animation shows its
  first frame.

**Animated images (M23.1).**
- **Composition:** GIF, APNG and animated WebP frames are composed in
  `images`, honouring each format's disposal and blending.
- **Timing:** every element showing the same image in a document shares
  one timeline, which starts when the first of them is added (HTML
  section 15.4.2): *(corrected)* when its header is read, on the
  document timeline of M22. The frame shown is chosen when a frame is
  drawn, not in paint.
- **Delays and loops:** a frame's delay of 10ms or less counts as
  100ms. Loop counts are honoured, and a finished animation rests on
  its last frame.
- **Cost:** a frame change is a change of one display item. It lays
  nothing out, and its raster is a partial redraw of the image's area.
- **Waking the scheduler:** *(corrected)* `next_frame_at` includes the
  next frame change of every animated image in view, its rectangle
  mapped through its groups' matrices (clips not looked at), and `tick`
  keeps wanting frames while one has some to come; `needs_frame` does
  not, an image's frame being a timed wake. Images out of view still
  advance in time but wake nothing.
- **Reduced motion:** the reference browser does not stop animated
  images under `prefers-reduced-motion`, and neither does this engine.
  A user's setting to stop them is G30's.

**SVG as an image (M23.4).** Not in this milestone. The roadmap draws it
"via M32 subset", and M32 is not done. `resvg` would serve, but it would
force a second tiny-skia or hold the tree at old font crates, and its
default resolver reads local files. An SVG image is broken until M32.

**Security.**
- **Bounds:** decoding happens off the main thread under the bounds
  above, and a panic is caught.
- **Fuzzing:** a fuzz target, `image_decode`, feeds each decoder,
  APNG, GIF and WebP composition, and the EXIF reader. The `pipeline`
  target gains pages with images as data URLs, and `css_stylesheet`
  gains the new properties.
- **Isolation:** the network-isolation rule holds. `images` opens
  nothing and reads no file: its decoders are given bytes.
- **Opaque responses:** image responses are opaque (no-cors), and
  nothing reads their pixels back: there are no scripts.

**Tests.**
- **Unit tests:** each decoder's bounds, every row of the `img` table,
  the sizing algorithm, the nine slices, gradient geometry and stop
  fixup, frame timing, and the taint.
- **Web-platform tests:** the subset gains `css/css-images`,
  `css/css-backgrounds` and the `img` element's reftests. `wpt pull` now
  copies the images a test uses, which it never needed before, and the
  runner serves their types. *(corrected)* Not done when the milestone's
  code was: the checkout the subset is pulled from was not on the
  machine; see the roadmap's M23 entry.
- **Golden pages:** 110 to 115, with images beside them under `images/`.
  A page with images says so with `<meta name="reference-load"
  content="file">` and is loaded by its `file:` URL, then drawn once its
  images are decoded. All match the reference browser byte for byte but
  the gradients' page, within 2.
- **Acceptance:** the photo grid, `tests/demos/photo-grid.html`, and
  `benches/images.rs`; `docs/perf-budgets.md` has the numbers.

## 2. Not in this milestone, said rather than hidden

Each item that no milestone holds is registered with the G list at the
documents step; the number follows each.

- *AVIF*, until an AV1 decoder meets `deny.toml` and builds without
  NASM. G34.
- *SVG as an image*, with M32.
- *`srcset`, `sizes` and `<picture>`*: the roadmap does not list them,
  and an `img`'s `src` is its fallback. G35.
- *`aspect-ratio` on boxes that are not replaced*. G36.
- *Conic gradients, `<color-interpolation-method>`, `image()`,
  `cross-fade()`, `element()` and `image-resolution`* (Images 4). G37.
- *Colour management*: embedded ICC profiles, `gAMA`, `cHRM` and
  `cICP`. G38.
- *Progressive display* of an image not yet completely loaded, which
  HTML allows. G39.
- *Downscaling by mipmaps*, `image-rendering: high-quality` and
  `smooth` beyond bilinear. M35.10's raster quality.
- *EXIF density correction* (HTML section 4.8.4.3.6). G35, with
  `srcset`.
- *Images of `cursor`, `mask-image`, `shape-outside` and `filter`*: with
  their properties (G32 and later).
- *The `decode()` method, `load` and `error` events, `naturalWidth`*:
  with scripts, M25 and M26.
- *Opaque response blocking*, which matters once scripts can read
  responses: M27.
- *Images in the PDF as images rather than raster*: G5.
- *`background-position-x` and `-y`* (Backgrounds 4). G40.

## 3. Order of work

Each step is a vertical slice: build, tests, clippy green and a smoke
run before the next, with its sabotage pass.

1. *The `images` crate.* The trait, the four decoders, header reading,
   EXIF orientation, frame composition, the bounds, `catch_unwind`; the
   dependency changes; the `image_decode` fuzz target. M23.1.
2. *Fetching and decoding.* The loader's image target, sniffing, the
   mixed-content upgrade, the body bound; the engine's image store, its
   decode threads and budget; `Loaded`; the invalidation of boxes
   showing an image. M23.1, M23.5.
3. *`<img>`.* Replaced boxes and the table of contents shown; the icon;
   the dimension hints and `aspect-ratio`; the default sizing algorithm
   in block, inline, flex, grid and intrinsic sizing; `object-fit`,
   `object-position`, `image-orientation`; pagination;
   `DisplayItem::Image` drawn by both backends. M23.2.
4. *CSS image values and backgrounds.* `<image>` with base URLs and
   `image-set()`; the background longhands and shorthand; layers,
   repeat, position, size, origin, clip, attachment; propagation to the
   canvas; `DisplayItem::Pattern`. M23.3.
5. *Gradients.* Parsing, geometry, stop fixup, interpolation, the
   engine's renderer and its use by both backends. M23.3.
6. *Border images, list-style images and `content: url()`.* M23.3.
7. *Animated images.* The shared timelines, delays, loops, the frame
   scheduler and partial redraws. M23.1.
8. *Memory.* Decoding on first draw, the decoded budget, downscaled
   copies, the GPU's texture budget. M23.5.
9. *The web-platform tests.* The new directories, copying images, and
   every failure in `expectations.txt` with its reason.
10. *Golden cases.* Pages from 110, captured from the reference browser.
11. *Acceptance.* A photo grid of PNG, JPEG, GIF and WebP with gradients
    and an animated GIF, scrolled at 60 frames a second, measured on
    both backends; a benchmark, `benches/images.rs`; T4.3; fuzz runs of
    `image_decode` and the targets that lay pages out. M23.6.
12. *The documents.* This note corrected to the code; the architecture's
    entry and layering; the roadmap's items ticked, M23.1's crates and
    M23.4's move to M32 recorded, and G34-G39 registered;
    `docs/perf-budgets.md`'s numbers.
