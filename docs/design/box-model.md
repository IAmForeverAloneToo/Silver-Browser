# Design note: box model completion

Roadmap M10. Written before the code, as the record of the decisions;
what ships is described in `docs/architecture.md` "Current state" when
the code lands. Where this note and the code later disagree, the code
wins and this note is corrected: it has been, and §4 lists what the
code taught.

The problem: the box model of M6 is the CSS2 algorithm for one case,
`width` and `height` in pixels on a `display: block` box with margins,
padding and borders in pixels. A real page says `width: 50%`, `padding:
1em 5%`, `max-width: 60rem`, `box-sizing: border-box`, `height: 100vh`,
`display: inline-block`, `border-left: 4px double`, and `overflow: auto`
on a panel it means to scroll — and today each of those is either
dropped or wrong. This milestone makes the box model complete for
normal flow, so that the pages M11 (floats and positioning) and M12
(tables) build on compute the same numbers a reference browser does.

## 1. Decisions

**The parser first.** The tree builder implies `html`, `head` and
`body` the way the HTML standard's insertion modes do: a document that
opens with anything but `<html>` gets one; the head-only elements
(`title`, `style`, `script`, `meta`, `link`, `base`) before any body
content go into an implied `head`; the first body content (any other
element, or text that is not white space) closes the head and opens
`body`; `</head>`, `</body>` and `</html>` end tags switch nothing off,
so content after them still lands in the body, as in browsers. The
tokenizer gains the raw-text state for `style`, `script`, `xmp`,
`iframe`, `noembed` and `noframes` and the escapable raw-text state for
`textarea` and `title`, so a `<` inside a stylesheet or a script is
text, and a textarea's content decodes its entities. This is M2's debt
rather than M10's item, but `rem` needs a root element, the UA sheet's
`body { margin: 8px }` must apply to a page that never wrote `<body>`,
and every comparison with a reference browser needs the same tree. The
consequence is accepted: every parsed document now has three more
elements, element indices shift, and the tests that assumed otherwise
are corrected rather than the parser kept wrong.

**Units are resolved in the cascade, against a context.** `em` resolves
as today. `rem` is the root element's computed `font-size` (on the root
element itself, the initial value). `vw`, `vh`, `vmin` and `vmax` are
percentages of the viewport the cascade is given, and the styled
document records whether any were used, so the engine recascades on a
viewport change only when they were. `ex` and `ch` come from the font
through a trait the engine implements over its primary font (the
x-height from the OS/2 table, the advance of `0`), with the
specification's fallbacks (half an em each) when no provider is given,
which is what unit tests use. `style` keeps no dependency on `text`:
the capability is passed down, the layering contract stands.

**Percentages are kept as percentages until layout.** `width` already
is; `height`, `min-height`, `max-height`, `min-width`, `max-width`,
`margin-*` and `padding-*` follow. The used value is computed in layout
against the containing block: the four margins and the four paddings
against its *width* (CSS2 §8.3 and §8.4, vertical ones included);
heights against its height when that is definite (a length, or a
percentage that itself resolved), and otherwise `height: auto`,
`max-height: none`, `min-height: 0` (CSS2 §10.5, §10.7). `font-size: %`
is a percentage of the parent's font size and resolves in the cascade.
Border widths take no percentage; `letter-spacing` and `word-spacing`
take none.

**`box-sizing`.** `content-box` (the initial value) and `border-box`.
Under `border-box` the specified `width`, `height`, and the min and max
of each include the padding and the border; they are converted to
content-box values (never below zero) before the CSS2 algorithms run,
which otherwise do not change.

**`min-*` and `max-*`.** CSS2 §10.4 for widths: the tentative width is
computed, clamped to `max-width` then `min-width`, and the margins are
resolved again with the clamped width. CSS2 §10.7 for heights: an
`auto` height is the content's, then clamped; a definite one is
clamped. A box whose `min-height` is not zero, like one whose `height`
is not `auto`, is never self-collapsing.

**Borders, complete.** Each side has its own width, style and colour
(`border-top-color`, `border-left-style`, and so on, the `border-color`
and `border-style` shorthands with one to four values, and the
`border-top`/`-right`/`-bottom`/`-left` shorthands). The styles are the
CSS2 ten: `none` and `hidden` paint nothing and count as zero width;
`solid` and `double` paint exactly (double as two strips of a third of
the width with the gap between); `dashed` and `dotted` paint dashes of
three times the width and dots of the width, from each side's start,
without the fitting adjustment real browsers make; `groove`, `ridge`,
`inset` and `outset` paint their two tones as the colour and the colour
at two thirds. The last six are approximations, said so here, and no
reference case uses them. Sides meet at mitred corners, each side a
trapezoid filled without anti-aliasing so that neighbours tile without
a seam; a reference browser anti-aliases the mitre between two colours,
so no reference case has sides of different colours either. The root
element's background paints the canvas (CSS2 §14.2; the body's when the
root's is transparent), so a white page is white to the edges.

**Anonymous block boxes.** The box tree builder makes every block
container's children block-level: a run of inline-level children is
wrapped in an anonymous block box, and a block-level box found inside
an inline child is hoisted to the containing block with its inline
ancestors split around it (CSS2 §9.2.1.1), the halves keeping the
inline element's style and node. An anonymous block inherits from its
containing block and produces no fragment of its own: its content's
fragments go into its parent's, so the fragment tree, paint and hit
testing are unchanged. Block layout then stacks block-level children
only; the inline-run path and M7.6's block-in-inline path inside the
inline layout go away. The observable fix: a block inside an inline is
a sibling of the blocks around it, and its margins collapse with theirs
through the anonymous blocks exactly as the specification says.

**Block formatting context roots.** A box with `overflow` other than
`visible`, and an inline-block, establishes a block formatting context:
its children's margins do not collapse with its own, it is never
self-collapsing, and its own margins collapse with its siblings as
usual. M9's clipping boxes already were such roots in the
specification; this makes them so in the layout.

**`display: inline-block`.** An atomic inline-level box laid out as a
block formatting context root. With `width: auto` its width is
shrink-to-fit, `min(max(min-content, available), max-content)`, so the
milestone adds intrinsic widths: a pass over a subtree that measures
each word (the longest unbreakable piece is the min-content, the
longest line the max-content), adds each box's horizontal padding,
border and margins, uses a length `width` as given and treats a
percentage one as `auto` (CSS2 §10.3.5), a percentage margin, padding
or `min-width` as zero and a percentage `max-width` as `none`. On the
line the box is an item as wide as its margin box, whose ascent is the
distance from its margin-box top to its last line box's baseline, or to
its bottom margin edge when it has no line box or clips (CSS2 §10.8.1);
this is M7.7, vertical alignment of inline-blocks on the baseline,
delivered with it. Other `vertical-align` values stay out.

**The strut, and white space.** A line is at least as tall as its
containing block's font, css-inline-3's root inline box: a line of small
text, or of inline-blocks alone, has the height the container's font
implies, as in browsers. The line gap of a font sits wholly below the
baseline in this engine, not split as half-leading; an inline-block's
own last line gap is inside its margin box and is not added again under
it. White space between words is what css-text-3 §4.1.1 says: one gap
of a space's width and a wrap opportunity where collapsible white space
was, within a text node or across the boxes of a run, and nothing where
none was (`a<b>b</b>` is set flush and never broken); an atomic inline
is a wrap opportunity on both sides; white space at the start or end of
a block container's inline run is dropped by the box tree builder,
since a line would discard it. The old rule, a gap at every word
boundary, was M7's simplification and could not have matched a
reference browser's row of inline-blocks.

**Elements that scroll.** A block container with `overflow: auto` or
`scroll` is a scroll container (a `hidden` one too, scrollable only
programmatically). Layout records on the box's fragment the height of
its scrollable overflow area (css-overflow-3 §2.2): the content as if
its height were `auto`, or any descendant's box or line reaching lower
(one inside a nested clipping box excepted), then the bottom padding,
and never less than the padding box itself; the furthest the content
scrolls is that height past the padding box. Its scroll offset lives in
the engine, keyed by the element's index, reset when a document loads
and clamped to the content at every frame; layout knows nothing else
about it and is not rerun by a scroll. Scrollbars are overlays that
take no space (the engine's own choice over the classic reserved gutter,
recorded here because it is observable: a `scroll` panel's content is
not narrowed), eight pixels wide, drawn by paint inside the padding box
at its right edge after the content when the container can scroll
(`auto`: when its content overflows; `scroll`: always; `hidden`:
never), the thumb shaped by the same function as the viewport's. Paint
translates a container's content by its offset inside the clip; hit
testing adds the offset on the way down; the wheel, handed to the
engine at the pointer, goes to the innermost scroll container among
the clipping boxes around the element hit that can still move in that
direction, then to its ancestors, then to the viewport (scroll
chaining). Each container animates with the same fixed-tick lerp as the
viewport, and one tick advances them all. Element scrollbars are
indicators, not controls, until M26's input work; vertical only, as the
viewport is.

**Form controls, v0.** The UA sheet makes `input`, `button`, `select`
and `textarea` inline-blocks with a border, padding and a background,
their text 13.33px as in the common browsers: an input twenty characters
wide (`20ch`) and one line tall, `1.2em` standing in for the line height
until `line-height` lands; a textarea twenty characters by two lines,
keeping its line breaks and scrolling; a button fitting its text; a
select the size of an input showing nothing (its options are hidden).
`type=hidden` displays nothing; a checkbox or radio is a thirteen-pixel
box with the browsers' margins; the `type` attribute is matched whatever
its case. The `size`, `rows` and `cols` attributes are not read yet. No
behaviour, and no value shown.

**Acceptance against a reference browser.** Ten pages under
`crates/engine/tests/reference/`, built of boxes only — no text, since
text rasterizers differ by design — each giving the root element a
background. `capture.sh` captures each at 800 by 600, device scale one,
scrollbars hidden, sRGB, from a headless browser: the headless shell of
the Chromium this build environment ships, whose viewport is exactly the
window (the full browser's headless window includes its toolbar and lays
the page out short); the roadmap names Firefox, and the harness accepts
any browser's capture, but for CSS2 box geometry the two agree and this
is what is here. Nothing of the browser enters the engine: the captures
are committed as fixtures, and the browser is needed only to make new
ones. The integration test (`tests/reference.rs`, the `png` crate as a
dev-dependency) renders each page with the engine and compares the
pixels byte for byte, writing the engine's render and a difference
image under `target/reference/` when they differ. The cases: content-box
margins, padding and borders; `box-sizing: border-box`; percentage
widths, margins and paddings; `min-width` and `max-width` with `auto`
margins; percentage heights under a definite height with `min-height`
and `max-height`; margin collapsing through nested boxes with borders
and padding as stops, an empty box between siblings and a negative
margin; `overflow: hidden` clipping at the padding edge, nested;
inline-blocks in a row with margins, wrapping, bottoms on the baseline;
borders per side including `double`, hidden sides and the width
keywords; `vw`, `vh`, `vmin`, `vmax`, `rem` and `em`. Every geometry is
whole pixels: a browser snaps a fractional edge to a pixel where this
engine anti-aliases it, a difference that is M11's painting work, not
the box model's.

## 2. Not in this milestone, said rather than hidden

Inline boxes' own backgrounds, padding and borders (the decoration of
a `span`) wait for M11's painting order work. `overflow-x` and
`overflow-y`, horizontal scrolling and the two-value `overflow` are not
parsed. `vertical-align` beyond the baseline, `line-height` (the strut
is the font's own metrics), and the `size`/`rows`/`cols` attributes are
not read. Classic scrollbar gutters are not reserved. The dashed, dotted
and three-dimensional border styles are approximations; mitres between
sides of different colours are not anti-aliased; a fractional edge is.
A button input's `value` is not shown. The selector matcher does not
know HTML's list of attributes whose values compare case-insensitively
(`type` among them), so the UA sheet says `i` where it matters. A
percentage `height` on the root element resolves against the viewport,
as in browsers, but the body's percentage height against the root's
`auto` height is `auto`, as in browsers too.

## 3. Order of work

Each step is a vertical slice on its own: build, tests, clippy green,
smoke run, then the next. (1) the parser: implied `html`, `head`,
`body`; raw text and escapable raw text; the tests of every crate that
assumed the old tree. (2) units: the cascade context, `rem`, `ex`,
`ch`, the viewport units, percentages stored for heights, margins,
paddings and `font-size`; the engine's cascade cache keyed on what the
document used. (3) `box-sizing`, `min-*` and `max-*`, percentage
heights, in layout. (4) borders per side and by style; the canvas
background. (5) anonymous block boxes and block formatting context
roots. (6) `inline-block`, intrinsic widths, the baseline. (7) scroll
containers. (8) the form controls' UA rules. (9) the reference harness
and its ten pages, and whatever they find. (10) the documents. All ten
landed on 2026-10-01, one commit each.

## 4. What the code taught

Four things this note did not foresee, each corrected above where it
stands. The strut: without it a line of inline-blocks was exactly their
height, and a browser's is not; it came with step 6. White space: M7's
gap at every word boundary would have put a gap between adjacent
inline-blocks that a browser sets flush, so the css-text-3 model of
collapsible white space came with the same step, and a text run of
nothing but white space is now kept where it is a gap. The scrollable
height needed a definition before the engine could clamp an offset or
paint a thumb; css-overflow-3's is on the fragment, computed by layout,
so the engine and paint share one. The capture: the full browser's new
headless mode makes a window of the size asked and lays the page out
in the viewport that is left under its toolbar, 513 pixels of 600, which
two of the ten cases caught (a box cut short, `vh` units short); the
headless shell's viewport is the window, and the script says why.
