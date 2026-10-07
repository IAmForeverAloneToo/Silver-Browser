# Design note: media queries and printing

Roadmap M20. Written before the code, as the record of the decisions;
what ships is described in `docs/architecture.md` "Current state" when
the code lands. Where this note and the code later disagree, the code
wins and this note is corrected, as the notes before it were.

The problem has three parts. A stylesheet's at-rules are dropped whole:
the cascade keeps the style rules at a sheet's top level and passes over
every `@media`, `@supports` and `@layer` block with all the rules inside
it, so a responsive page loses its breakpoints, a page that tests for a
feature loses what it tested for, and a page built on cascade layers
loses everything it put in them, which on today's pages is often the
whole design. What media logic exists is in the loader: a `<link>` or
`<style>` whose `media` is anything but `all` or `screen`, `@import`s
the same, and an `@import` with a layer or a condition, are left out,
`(min-width: 600px)` among them. The engine knows no user preference:
nothing says whether the user wants a dark page or less motion. And
nothing prints: the layout is one continuous column, and there is no way
to put a page on paper or in a file. This milestone adds media queries,
evaluated against the viewport and the user's preferences and evaluated
again when either changes; conditional and layered rules in the cascade,
at any depth and through `@import`; the preferences themselves, from the
operating system where it says; and printing: `@media print`, the page
box and `@page`, a paginated layout of the document that breaks it
between blocks and between lines, and a PDF of the pages.

The standards are read at the versions current when this note was
written: Media Queries Level 4 (Candidate Recommendation Draft of 14
July 2026) for the syntax, the error handling and the features of the
device; Media Queries Level 5 (Working Draft of 29 June 2026) for the
user-preference features; CSS Conditional Rules Level 3 (Editor's Draft
of 7 December 2025) for `@media` and `@supports` as conditional group
rules, and Level 4 (Editor's Draft of 22 October 2025) for `selector()`;
CSS Cascading and Inheritance Level 5 (Editor's Draft of 13 April 2026)
for cascade layers and `@import`'s conditions; CSS Fragmentation Level 3
(Editor's Draft of 22 October 2025) for breaks; CSS Paged Media Level 3
(Editor's Draft of 3 March 2026) for the page box and `@page`; CSS Color
Adjustment Level 1 (Candidate Recommendation Snapshot of 7 May 2026) for
`print-color-adjust`; CSS2 §13 and §9.6.1 for paged media and fixed
boxes on paper; and HTML for the `media` attribute and what a style
sheet blocks. Where the reference browser and the web-platform-tests
agree with each other against the text of a draft, this engine follows
them, and says so where it does. A few questions the drafts leave open
were put to the reference browser before this note was written, and its
answers are given where they decide something.

## 1. Decisions

**The media environment.** What a query is evaluated against is one
value, `style::Media`, which the engine builds and the cascade reads
through `CascadeContext`: the media type, `screen` or `print`; the width
and height of the viewport in CSS pixels, or of the page box on paper;
the resolution, in dots per CSS pixel; and the user's preferences. On
the screen the width and height are the frame the engine draws, the
toolbar's rows not counted and the overlay scrollbar counted, since it
takes no room (Media Queries 4 §4.1 counts a scrollbar in). The
resolution is one dot per pixel: the engine draws a CSS pixel to a pixel
of the window, whatever the display's scale factor, and the query says
what is drawn (section 2). The device's other features have the values
of a desktop browser drawing in colour with a mouse, fixed: `color` 8,
`color-index` and `monochrome` 0, `color-gamut` srgb, `pointer` and
`any-pointer` fine, `hover` and `any-hover` hover, `grid` 0, `update`
fast on the screen and none on paper, `overflow-block` scroll on the
screen and paged on paper, and `overflow-inline` none, the document not
scrolling across (M9). Media Queries 5's preferences have the user's
values where the engine has them, `prefers-color-scheme` and
`prefers-reduced-motion` (below), and otherwise the values that say
there is no preference, which is the truth: `prefers-contrast`
no-preference, `prefers-reduced-transparency` no-preference,
`forced-colors` none, `inverted-colors` none, `dynamic-range` and
`video-dynamic-range` standard; `scripting` is none until M24 runs
scripts, which a page that hides a script's controls under `(scripting:
none)` is right to be told; and `display-mode` is browser. The
Compatibility Standard's `-webkit-device-pixel-ratio`, with its
`-webkit-min-` and `-webkit-max-` prefixes, is the resolution as a
number of dots per pixel, as the reference browser has it. A feature
neither level defines, or one this note leaves out, is unknown; the next
paragraph says what that does to a query. The reference browser, asked
at step 1, answers otherwise where its own device or its own features
differ, and this engine keeps its answers: its headless shell has no
pointer and no hover, it runs scripts, its `overflow-inline` is scroll,
its `ex` is the font's x-height where this engine's is half the font
size, it takes `lh`, the container units and `calc()` in a query, which
this engine's cascade does not, and it knows no `video-dynamic-range`.

**Parsing and evaluating a query.** `style::media` parses a
`<media-query-list>` from the tokens the css crate keeps, by Media
Queries 4 §3: the types `all`, `print` and `screen`, the other types of
CSS2 recognised and matching nothing (§2.3), `not` and `only`, and
conditions of features in parentheses joined by `and`, `or` and `not`
with parentheses nested to 32 levels, as selectors' are
(`style::media::MAX_NESTING`), past which a condition is unknown, the
three forms of a feature — plain (`min-width: 600px`, the `min-` and
`max-` prefixes on range features only), boolean (`(color)`) and range
(`(400px < width <= 800px)`, either way round). A query that does not
match the grammar becomes `not all` (§3.2), and the rest of its list
stands; the commas that part a list are those outside every block, a
block closed by its own kind of bracket, and a block the tokens end
inside is closed by their end, as css-syntax closes it, so that `(color`
holds, as in the reference browser. A keyword needs no white space after
it before a parenthesis, `and(` being a function's name rather than
`and`, and an integer may be negative, `not (color: -1)` being a valid
query that holds. A feature whose name or value this engine does not
know, and anything in parentheses the grammar does not, is
`<general-enclosed>` and evaluates to unknown; the evaluation is
three-valued as §3.2 has it, `not` of unknown being unknown and a query
whose value is unknown false. Lengths in a query are absolute or
relative to the initial values (§1.3): `em` and `rem` are 16px, the
initial font size, `ex` and `ch` half that, as the cascade takes them
without a font, and the viewport units are the viewport's. Ratios take a
slash or a lone number, and a resolution `dppx`, its alias `x`, `dpi`
and `dpcm`. A list is held as parsed; whether a change of the
environment changes its answer is learnt by evaluating it again, a few
comparisons (below).

**Conditional rules in the cascade.** The cascade stops dropping
at-rules. When style sheets are prepared (`Stylesheets::new`), their
rules are read into a `RuleSet`, to the depth the css crate parses and
without recursion: each `@media` block with its parsed list, each layer
by its name, and each style rule with the blocks it is inside.
`@supports` is decided as it is met, since its answer is the engine's
and does not change: a rule inside a false `@supports` is not kept at
all. At each cascade every list is evaluated once against the
environment given, its answers kept in order (`RuleSet::answers`), and
the rules in force are those inside no list that is false
(`RuleSet::in_force`), with the layers they are in. A nested `@media` or
`@supports` must hold with every rule around it (Conditional 3). The
same walk gives the selector features M13.1's invalidation reads, from
every style rule whatever its conditions, so that a rule that starts to
apply after a resize finds its elements as any rule does. `@container`
and `@scope` blocks, whose conditions this engine cannot evaluate, keep
being passed over with their rules; a `@media` met inside a style rule
waits for CSS nesting, which the css crate does not parse (section 2).
The user agent's sheet is read the same way, once.

**`@supports`.** A `<supports-condition>` by Conditional 3: `not`, `and`
and `or`, which may not be mixed without parentheses, over declarations
in parentheses and `selector()`. A declaration is supported when the
cascade accepts it: `style::supports`, which M17 made for the
web-platform-tests' rule, answers whether the property is one the engine
applies and the value one its parser takes, the CSS-wide keywords
included; a custom property is not supported until M21 parses them.
A declaration's `!important` is taken off before it is asked, as the
reference browser takes it. `selector()` (Conditional 4) is supported
when the selector parser takes one complex selector without its
forgiving rules, `:is()` and `:where()` unforgiving inside it, and the
matcher knows every part of it: a `::-webkit-` pseudo-element is not
supported (Conditional 4), and neither are `:has()` and `&`, which this
engine's selector parser does not take and the reference browser's does
(section 2). Anything else in parentheses, `font-tech()` and
`font-format()` among it, is false, where the reference browser takes
those two and custom properties. Parentheses nest to the same 32 levels
as a media query's, past which a condition reads as a declaration that
is false. A condition that does not parse makes the rule invalid, and it
is dropped with its block, as Conditional 3 says.

**Cascade layers.** `@layer` in its three forms (Cascade 5): the
statement, which declares layers in order without content; the block
with a name, which puts its rules in the layer; and the block without
one, an anonymous layer of its own. A dotted name is a layer inside a
layer, and a block inside a layer's block names a layer inside it. A
layer's place is its first appearance in the author's sheets, taken in
the order the cascade takes them, so that a statement at the top of the
first sheet orders layers the later sheets fill; the appearances counted
are those of the rules in force, so that a layer named only inside an
`@media` that does not hold has no place, as the web-platform-tests have
it (`layer-media-query.html`), and the order is found again at each
cascade from the answers. The CSS-wide keywords are not layer names, as
Cascade 5 reserves them; the reference browser takes them, the
web-platform-tests have no case, and the draft is followed. Every
layered rule gets its layer's place in that order, nested layers ordered
inside their parent, and the rules in no layer come after every layer at
their level. The cascade's order of declarations becomes origin and
importance, then layer, then specificity, then order of appearance, as
Cascade 5's cascade sort has it: among normal author declarations a
later layer wins and the rules in no layer win over all, and among
important ones the order of layers is reversed, the first layer winning
and the rules in no layer losing to every layer. The user agent's sheet
has no layers, and there is no user origin yet (section 2).
`revert-layer`, like `revert`, is not a keyword the cascade knows yet
(section 2).

**`@import` and the `media` attribute.** An `@import` may carry, after
its URL, `layer` or `layer(<name>)`, `supports(<condition>)` and a media
query list, in that order (Cascade 5); one with a block, or one after a
style rule, is ignored as before. The loader fetches the imported sheet
whatever the conditions say, and gives the cascade its rules wrapped in
the conditions: a `@media` block outermost, a `@supports` block inside
it and an anonymous or named `@layer` block innermost, so that the
cascade's one reading treats an imported sheet as it treats the rules
written inside those blocks. The rules are put in place of the `@import`
rule in the importing sheet, so that the engine's sheets are still one
for each element that brings one. An import that is not there — one that
failed, one still coming, one in a cycle, one past the depth of 16 —
leaves its wrappers empty, so that its layer is named all the same, as
Cascade 5 says it is. A `<link rel=stylesheet>` or `<style>` with a
`media` attribute is wrapped the same way in a `@media` block of the
attribute's value, parsed as a media query list (HTML: an empty or
absent attribute is `all`). Every such sheet is fetched, a print sheet
included, since printing and a resize need them, but only one whose
media holds blocks rendering, HTML's rules for what holds a document's
rendering and its scripts both asking whether the media matches, decided
when the parser meets the sheet, against the medium then — for a
session's first page, before its window has a size, a viewport of none;
one that does not is applied when it comes, and the page is not held for
it. A change of a `media` attribute alone fetches nothing again, HTML
naming no such fetch; the wrapper changes. The `load` event waits for
every sheet, as before. The preferred and alternate sets stay as M15
left them.

**Evaluated again.** A sheet's media query lists are evaluated at each
cascade against the environment it is given; the engine keeps, with the
styles it computed, the answer each list gave. Before a frame, as it
already asks whether a style used a viewport unit and the viewport
changed, the engine evaluates the lists again against the environment
now — the frame's size, the preferences — and, when any answer differs,
cascades the document again and lays it out. A resize across a
breakpoint is then one cascade and one layout, as the resize itself
already costs; a resize that crosses none costs nothing more, since a
list is a few comparisons. The lists that read only the media type or
features fixed for the session never change answer on the screen.
`Stylesheets::media_changed` is the question, the user agent's lists and
the authors' answered again and compared with those the styles keep.
`Engine::set_preferences` changes the preferences and asks for a frame
when an answer changes. `Engine::set_media_type` styles the document
shown for another medium on the screen, as a browser's developer tools
emulate one, the window drawn with what a page's print styles say,
unpaginated: the reference pages use it as the reference browser's
`mediaTypeOverride`, and a print is not affected by it.

**The user's preferences (M20.3).** `prefers-color-scheme` is light or
dark, and `prefers-reduced-motion` no-preference or reduce. The shell
reads them from the operating system where winit or the platform says,
and takes `--prefers-color-scheme light|dark` and
`--prefers-reduced-motion` to override it: the colour scheme from
winit's window theme, which Windows and macOS report and report again
when it changes (`WindowEvent::ThemeChanged`), and which X11 does not
and Wayland reports only when the application set it, so that on Linux
the option is the way; reduced motion from Windows's
`SPI_GETCLIENTAREAANIMATION` and macOS's
`accessibilityDisplayShouldReduceMotion`, through the `windows-sys` and
`objc2-app-kit` crates winit already brings in there, read when the
shell starts and again when its window regains the focus, winit having
no event for a change of it, and on Linux the option, the desktop portal
that says it being a D-Bus service the shell has no client for (section
2). `--prefers-reduced-motion` forces `reduce`; no option forces
`no-preference` over the system's. The platform code is the shell's, as
the layering rule says, and is checked here by compiling it for
x86_64-pc-windows-msvc, aarch64-apple-darwin and x86_64-apple-darwin
with the shell's crates and features: the whole shell does not build
here for them, its TLS provider building C for the target, for which
there is no cross compiler here. The engine reads no preference itself.
The roadmap's M20.3 says the built-in interface's dark theme exists; it
does not, the toolbar having one light theme until settings (M37.3),
which the roadmap is corrected to say. A page in the dark scheme under a
light toolbar is what this milestone gives. The `color-scheme` property
and the dark canvas the user agent paints under a page that asks for one
are not this milestone's (section 2).

**Print: what prints.** `Engine::print(&PrintOptions) -> Result<Vec<u8>,
PrintError>` makes a PDF of the document as it now stands, without
touching what the window shows: its own cascade under the print
environment, its own layout and pagination, its own paint and raster on
a software backend of its own, the document's styles, layout, scroll
offsets and the window's backend left as they were. The environment is
`print`; its width and height the page box of the size the print is
asked at, its margins included, as Media Queries 4 §4.1 says, `@page`'s
own size not consulted, since `@page` rules may themselves sit inside
the queries; its resolution that of the raster (below); and the
preferences the screen's. The reference browser, asked, evaluates them
against the page area of the size it was asked at — Letter less its
margins, 741 by 981 pixels, rounded down — `@page` not consulted either:
the margins are the difference, and no web-platform-test takes its side
(`css-page/media-queries-001-print.html` takes either, the working
group's issue 5437 open), so this engine follows the draft and records
it. The page prints with its backgrounds, as the reference browser's
print to PDF does; `print-color-adjust` (Color Adjust 1) parses,
inherited and `economy` initially, and changes nothing, since the engine
economises on nothing (section 2). The shell gains `--print-to-pdf
<path>`, which loads the page given, waits for it, prints it and exits,
as the reference browser's headless mode does, and the engine's tests
print through `Engine::print` and its two steps, which are public:
`printed_pages`, each page's box and the display list of its area, which
the print tests read the text of, and `print_pages`, those pages drawn.
A print dialog, and a shortcut that opens one, wait for the shell's
dialogs (M37).

**Print: the page box and `@page`.** The `@page` rule (Paged Media 3) is
taken from the author's sheets, inside the conditions and layers its
sheet puts it in, with the selectors `:first`, `:left`, `:right` and
`:blank`, and, in its block, the descriptors `size` and `margin` with
its longhands; the page-margin boxes inside it are dropped, and the css
crate's parser, which drops every at-rule inside a block, is left to do
so. Its cascade is the same as a style rule's, specificity being the
page selectors'. A rule may have a list of selectors and a selector
several pseudo-classes, `@page :first:left` and `@page a, b`, as the
draft and the web-platform-tests have them; the reference browser drops
those, and the draft is followed. `size` takes `auto`, one or two
lengths, and the named page sizes with `portrait` or `landscape`, as the
reference browser does (`A0` no name, a percentage no size); a margin a
length or a percentage of the page box's width or height, `auto` being
zero. With no `size`, the page is the reference browser's for its print
to PDF, US Letter, 8.5 by 11 inches, and with no margins, its margins,
measured from its output at step 6: 28 points a side, 37.33 pixels,
which leave Letter a page area of 741.33 by 981.33 pixels whose top left
corner is placed at whole pixels, (37, 37); `PrintOptions` may give
another default size and margin, as the reference browser's options do.
The page area of each page is the page box less its margins, and the
first page's is the initial containing block, whose width the document
is laid out at. The area is rounded as the reference browser rounds it,
which the print references found at step 8: the document is laid out at
the first page's area width rounded up to a whole pixel
(`PageBox::layout_area`), 742 on Letter, and cut into pages each as tall
as its area rounded up, 982, the last two thirds of a pixel cut off by
the area's edge when the page is drawn; the viewport units measure the
area rounded down (`PageBox::viewport`), 741 by 981. The reference
browser's fixed and absolutely positioned boxes take the rounded-down
viewport as their containing block where this engine's take the
rounded-up one, the initial containing block being one rectangle here:
such a box anchored at the right or the bottom is a pixel apart. A page
whose margins leave a page area of another width than the first's —
`:left` and `:right` with uneven sides — shows the content laid out at
the first page's width from its own left margin: one inline size for the
whole document, as section 2 says. Pages are right and left in turn from
a right first page, the document being left to right.

**Print: the break properties.** `break-before` and `break-after`
(Fragmentation 3), with `auto`, `avoid`, `avoid-page`, `page`, `left`,
`right`, `recto` and `verso`, and the column and region values parsed,
as the web-platform-tests have them where the reference browser rejects
the region ones, and meaning nothing on a page, which has no columns or
regions to break; not `all` or `always`, which are Level 4's;
`break-inside` with `auto`, `avoid` and `avoid-page`; the legacy
`page-break-before` and `page-break-after` as their aliases with CSS2's
values, `auto`, `always`, `avoid`, `left` and `right`, `always` being
`page`, and `page-break-inside` with `auto` and `avoid`; `orphans` and
`widows`, inherited, two each; and `print-color-adjust`, with its legacy
name `-webkit-print-color-adjust`, which the reference browser has. They
live behind one pointer in the computed style, as M19's effects do,
`None` being every initial value: the style grows by eight bytes, to
448, which its size test states, and the box and the fragment by the
same. M17's and M19's lesson — every byte of the style is copied into
every fragment — argues for nothing more.

**Print: pagination, v1.** The document is laid out once, as on the
screen, at the page area's width, the page area's height the viewport's
for the units and percentages that read it, each rounded as the page box
says above. Pagination is then a pass over the fragment tree,
`layout::paginate`, which decides where the pages break and moves what
comes after each break to the next page, rather than a layout that knows
its pages as it goes. It reads the fragments, in document order and
without recursion, as pieces a break goes between — the lines of a block
container, found by the line's top every text fragment carries, a box
never broken inside, and empty space — and the places before the
pieces, and fills the pieces into pages:

- The places a page may break are between two blocks in the normal flow
  of the same block container, and at the start of the first or the end
  of the last (Fragmentation 3's classes A and C), and between two lines
  of a block container (class B). A place knows the boxes that start
  there, outermost first: a break there goes before the outermost when
  something is before it on the page (class A), or after the top border
  or padding of a box that has one (class C), each a level of the place.
- Empty space — a block with no content, and what is below a block's
  last line or child — is a piece a page may end anywhere in, as the
  reference browser cuts an empty block where its page ends; inside a
  box that avoids breaks it is not cut.
- A block is monolithic, never broken inside, when it is a replaced
  element, a box whose `overflow` clips, a flex or grid container, a
  table, or a transformed box, one that contains its descendants (M19);
  an inline-block moves with its line, a float with the line it is
  anchored to, and an absolutely positioned box with its containing
  block (Fragmentation 3 names replaced elements and scroll containers;
  the others are this milestone's, section 2). One that does not fit on
  what is left of a page goes to the next; one taller than a page is put
  at the top of a page and sliced at its bottom, the rest showing on the
  pages after, as Fragmentation 3 allows for content too big for a page.
- A forced break, `page`, `left`, `right`, `recto` or `verso` between
  blocks, begins a page there; `left` and `right` add a blank page when
  the next would be the wrong side, which `:blank` matches, as
  Fragmentation 3 says. The reference browser's print to PDF adds none,
  every side breaking as `page` there, which the print references found
  at step 8; the draft is followed, and the references ask only for the
  side each page already is.
- Of the places that let the content before them fit, the last is taken,
  at its latest level, unless it breaks one of Fragmentation 3's rules —
  an `avoid` between the blocks, a `break-inside: avoid` of a box around
  it, fewer lines before it than `orphans` or after it than `widows` in
  its block — when the latest that breaks none is taken, a place's
  levels tried from the innermost box's to the one before them all
  before an earlier place, so that a box avoiding breaks inside it moves
  whole rather than from after its padding; when none does, the rules
  are let go in the draft's order (§4.4), the line counts first and the
  avoids after, and when still none fits, the content is broken where
  the page ends, a break where none is allowed being better than content
  lost. A paragraph of fewer lines than `orphans` and `widows` together
  is so moved whole when the break before it is allowed, as CSS2 §13.3.3
  has it; the reference browser breaks it inside, keeping `orphans`,
  which the print references found too, and the draft is followed.
- The pages are `layout::MAX_PAGES`, 10,000, at most: what is below the
  last is on none. A hostile page of the tallest boxes the cascade
  allows, stacked, would otherwise be cut without end, the pages' tops
  in `f32` arithmetic no longer moving by a page's height far enough
  down, which was found at step 11 as the `pipeline` fuzz target was
  made to paginate.
- At a break the margins that meet it are truncated: an unforced break
  drops the top margin of the content after it and the bottom margin of
  the content before; a forced one keeps the margin after it. A box
  broken across pages has its borders and padding cut at the break,
  `box-decoration-break: slice`, its background going on.

Moving the content is a single walk over the fragments after the breaks
are known: each break lifts everything below it in the document's order
by its strut, the distance from where the content after it was to the
top of the next page's area, and a box that holds content on both sides
of a break grows by the strut and is drawn across the pages. This is how
a layout without fragmentation in it pages a document, and it is right
for what flows in blocks and lines. What is beside a break is not laid
out again: a float that the break passes through keeps its place and the
lines that flowed around it keep their widths, and text that wrapped
beside a float on one page is not rewrapped on the next. A layout that
knows its pages, laying out each fragment in the room its page leaves,
is CSS Fragmentation's whole model and is section 2's.

A `position: fixed` box is drawn on every page at its place in the page
area (CSS2 §9.6.1, which Paged Media leaves as it was, and the reference
browser does). An absolutely positioned box moves with the content its
containing block moved with. The viewport's scrollbar, a scroll
container's scrollbar and its scroll offset are not printed: a scroll
container prints its content at its top, clipped as on the screen.

**Print: the raster and the PDF.** Each page's display list is the
document's painted with the page's area as the viewport, at the page's
offset down the paginated document, the fixed boxes listed for each, and
the page's area as the clip. It is rasterized on the software backend at
300 dots per inch, the resolution the reference browser prints at, by a
group around the list scaling it by 300/96 — M19's groups draw text at
the size a scale makes, so the glyphs are sharp at the printer's
resolution, not scaled pixels. The groups of the page's list are drawn
through, none kept as a layer: a page is drawn once, and the document's
scroll group, a layer on the screen (M19), drawn into its tiles at a dot
a pixel and resampled to the print's resolution, blurred the text and
cost most of a page's time, which step 11's benchmark found. Each page's
raster is compressed as soon as it is drawn (`PdfPage::new`), so that a
print holds one raw raster at a time, not all of them until the file is
written. The PDF is written by the `pdf-writer`
crate (MIT or Apache-2.0, which `deny.toml` allows; of its dependencies
the tree lacks only `ryu`, which formats its numbers, under the same
licences): one page per page box at its size in points, the page's
raster an image, RGB, eight bits, Flate-compressed by `flate2`, which
`net` already builds, placed over the page area. The file has the
document's title, and no dates, so that a page prints to the same bytes
each time. Its text is pictures of text: the PDF is a faithful print but
not one whose text can be selected or searched, which a PDF drawn from
the display list as paths and fonts would be (section 2). The page area
of US Letter at 300 dots per inch is about 2,317 by 3,067 pixels, under
the backend's limit; a page past the limit is printed at the resolution
that fits it.

**Print: what is checked against the reference browser.** The reference
browser prints the same page to PDF (`--print-to-pdf`, with no header
and footer), and its PDF says, page by page, where each glyph is, in the
document's coordinates at its resolution, through Flate-compressed
content streams and the ToUnicode maps of its fonts: composite fonts
with two-byte codes, the simple ones it makes for a synthesized bold
with one, and marked content's `ActualText` standing for a ligature's
glyph. A small reader of that output, `crates/engine/tests/print/
reader.rs`, which the capture and the tests share, reads it as words,
the text between the white space CSS collapses, each at its first glyph;
`crates/engine/examples/print.rs capture` prints each print page through
the reference browser, giving it this engine's font by a style at the
top of a copy of the page, and writes beside the pages each page's box
and area and each word on it with its place; and the print tests hold
the engine's pagination to it: the same number of pages, each the same
size with its area in the same place within half a pixel, the reference
browser's page sizes being whole device pixels, and each word on the
same page within two pixels of the same place, the engine's text
fragments being words. A page the reference browser draws nothing on has
an empty content stream and no area to read; the capture says so. This
reads the reference browser's PDF, not PDFs in general, and says so; the
engine's own PDF is read back by the same reader in the tests, to hold
its structure and its pages' sizes. The look of `@media print` styles is
checked apart from pagination: the reference browser draws a page's
screen layout with the print media type when told to
(`--blink-settings=mediaTypeOverride=print`), and the engine does the
same through `Engine::set_media_type`, so that golden pages compare
print styles pixel by pixel as they compare screen ones. The preferences
are checked the same way, the reference browser taking
`preferredColorScheme` and `--force-prefers-reduced-motion`. A page says
in its head what it is drawn under — `reference-size`,
`reference-media`, `reference-color-scheme` and
`reference-reduced-motion` — which `capture.sh` and `tests/reference.rs`
both read, and the acceptance layouts' capture and runner read the size.

**The web-platform-tests.** The subset gains `css/mediaqueries`,
`css/css-conditional` and the layer tests of `css/css-cascade` by M17's
rule, whose check of a test's style sheets now lets `@media`,
`@supports`, `@layer` and `@import` through, the conditions being
evaluated rather than read as unsupported, and a test's `@import`s
followed sixteen deep; `expectations.txt` gains this milestone's
section, emptied as the steps land. Most of `css/mediaqueries` and of
the layer tests are script tests of `matchMedia` and the CSSOM, which
wait for scripts (M24); the reftests are what the subset runs: 68 of
them, 3 of `css/mediaqueries`, 63 of `css/css-conditional` and 2 of
`css/css-cascade`, 974 in the subset. Seventeen are expected to fail,
each for what it asks of a part of the engine other than its media
queries, cascade and loader: no `visibility` property, no images in a
`background` yet, no `@font-face`, Conditional 5's `at-rule()`, which
the reference browser does not take either, `:has()` and pseudo-elements
this engine does not know (`::details-content`, those of form controls)
in `selector()`, a tentative test that expects `::-webkit-slider-thumb`
supported where Conditional 4 says it is not, and an HTML parser
without the "in head noscript" insertion mode. The print reftests
of `css/css-break` and `css/css-page`, which compare paginated
renderings, need the harness to print, and wait (section 2).

**The golden cases.** Box-only reference pages, numbered from 80,
captured and compared as the others are: breakpoints in `min-width`,
`max-width` and range syntax, captured at widths on either side of them;
`orientation` and `aspect-ratio`; nested and combined conditions, `not`
and unknown features; `@supports` with declarations, `not`, `and`, `or`
and `selector()`; layers in every form, with `!important` reversing
them; and print styles, the dark scheme and reduced motion under the
reference browser's settings. Print pages, compared by the text on each
page: paragraphs across many pages, forced breaks before headings,
`avoid` around a figure, orphans and widows, a table and a flex
container too tall for what is left of a page, `@page` with a size,
margins and `:first`, and a fixed header on every page. Two acceptance
layouts with text check a responsive page at its breakpoints, as M17's
and M18's layouts check theirs.

As built: eleven reference pages, 80 to 90 — the breakpoints at 640 by
480 pixels with the viewport a pixel below them, on them and a pixel
above; orientation and aspect ratio in a portrait viewport; combined,
nested, unknown and malformed queries; `@supports`; layers, and layers
with `!important`; the print medium on the screen; a dark scheme and
reduced motion — each a row of cells coloured by the condition each
answers, checked cell by cell against what it asks; they found nothing.
Seven print pages under `tests/print/` and `about:silver`'s, every word
on each where the reference browser puts it, exactly; the table and the
flex container ask for `break-inside: avoid`, since the reference
browser breaks inside both and this engine neither (section 2), and the
sides asked for are the sides the pages already are, the reference
browser adding no blank page. And three acceptance layouts rather than
two, the responsive page at 560, 800 and 1100 pixels, either side of
both its breakpoints.

**Acceptance (M20.5).** `about:silver` gains a print style — dark text
on white, the cards and tiles bordered rather than filled, a break
before the long paragraphs — and breakpoints that stack its cards below
a width and widen its tiles above one; it prints to a PDF of several
pages, the text on each where the reference browser's print of the same
page puts it, and relays out at each breakpoint as the window is resized
across it. A page of the milestone's own, responsive with layers and
`@supports`, is drawn at the widths on either side of its breakpoints
against the reference browser's captures.

As built: `about:silver` is a file of its own, `src/about-silver.html`,
which the engine includes and the print references print. Narrower than
600 pixels its cards stack, from 1000 its tiles are 200 wide at least,
and on paper its cards and tiles are kept together besides, the
reference browser breaking flex and grid containers inside where this
engine does not. It prints to three pages, every word where the
reference browser's print puts it, exactly. Printing it found the user
agent sheet's pixel approximations of the HTML standard's `em` margins
and heading sizes, now the standard's, and that the engine wraps lines
only at white space where the reference browser also breaks after a
hyphen (UAX #14); the page writes its CSS names with non-breaking
hyphens, which neither breaks, and the gap is section 2's.

**Performance and fuzzing.** A benchmark, `benches/media.rs`, measures
the evaluation of a page's media query lists before a frame, a resize
across a breakpoint of the mutation benchmark's page, and the print of a
page of a thousand paragraphs to PDF: its pagination, its raster and its
PDF, and the file's size. T4.3 compares the benchmarks before this
milestone with its own by M17's method. A fuzz target, `media_query`,
parses and evaluates query lists and `@supports` conditions from
arbitrary bytes, and the pipeline target paginates and prints the
documents it lays out; the three targets that lay pages out run fifteen
minutes each with the media and print keywords in a dictionary and the
milestone's pages among their seeds.

As built: the benchmark's numbers are in `docs/perf-budgets.md`. A
breakpoint costs a resize a cascade, 6ms beside the 506ms the layout of
1,000 paragraphs at a new width costs any resize, and answering 1,000
lists again 15 to 20µs; the print of those paragraphs, 53 pages, takes
5.9s, three quarters of it compressing the rasters, and makes a file of
21.8 MiB. The benchmark found the layer the print drew through and the
rasters it held, above, and T4.3 found every other benchmark within its
10%. `media_query` also cascades a small document under every medium,
holding its styles unchanged under the same one, and resolves `@page`
rules for each page position; the `pipeline` target prints at the
width its `@page` rules give, checking the pages lie one under another
and are no more than the bound. `media_query` and the three that lay
pages out, `pipeline`, `style_document` and `mutation`, ran fifteen
minutes each, `pipeline` and `mutation`, which spent theirs replaying
their corpora, fifteen more from their smallest inputs; none failed.

## 2. Not in this milestone, said rather than hidden

Each item here that no milestone of the roadmap holds is registered with
the deferred items, the G list, at step 12.

- *A layout that knows its pages.* Pagination moves laid-out content to
  the next page; it does not lay content out again in the room a page
  leaves. A float a break passes through, and the lines beside it, keep
  the places and widths of the continuous layout; a page area of another
  width than the first page's shows the content at the first page's
  width; flex and grid containers, tables, inline-blocks and floats are
  not broken inside but moved whole or sliced; columns (`column-count`),
  named pages (`page`) and the regions of the break properties have no
  context to break in. A layout that fragments as it goes is CSS
  Fragmentation's whole model, and multi-column layout waits on it.
- *The page-margin boxes*, headers and footers drawn by `@page`'s
  `@top-left` and the rest, and the running headers and page numbers of
  `string-set` and `counter(page)`; the reference browser's own header
  and footer, which its print to PDF draws unless told not to, are not
  drawn either.
- *A PDF with text in it.* The pages are images; a PDF drawn from the
  display list, with the fonts embedded and the text selectable and
  searchable, is printing's next step.
- *A print dialog*, choosing a printer, and printing to one, with the
  shell's dialogs (M37).
- *Economy*: `print-color-adjust: economy` lets a printer drop
  backgrounds and darken text; the engine prints what the screen shows,
  as the reference browser's print to PDF does.
- *The `color-scheme` property*, the dark canvas and form controls a
  page asking for the dark scheme gets, `light-dark()`, and the system
  colours (CSS Color Adjustment 1).
- *The colour scheme and reduced motion on Linux from the desktop*: the
  XDG desktop portal says both, over D-Bus, for which the shell has no
  client; the options say them there. *A display's scale factor*: the
  resolution is one dot per pixel until the engine draws at the
  display's scale.
- *`revert` and `revert-layer`*, which roll a property back to an origin
  or a layer: the cascade knows `initial`, `inherit` and `unset`.
- *The user origin* and user style sheets, with the browser's settings
  (M37.3).
- *`@container` and `@scope`*, passed over with their rules; *`@media`
  inside a style rule*, which needs CSS nesting; *custom media queries*
  (`@custom-media`, Media Queries 5 §10).
- *`matchMedia` and `CSS.supports()`* from script, and the script tests
  of the subset that use them (M24, M25).
- *The print reftests* of the web-platform-tests, which compare
  paginated renderings: the harness renders the screen alone.
- *The dark interface*: the toolbar has one light theme (M37.3).
- *Line breaking by UAX #14*: a line wraps only at white space, where
  the reference browser also breaks after a hyphen; and
  `white-space: nowrap`, which M8.4 did not take. Found by the print of
  `about:silver` (step 10).
- *The root element's margins*: a child's top margin collapses through
  the root element's box here, which CSS2 §8.3.1 says it does not; the
  root's box starts below the margin where the reference browser's
  starts at the top. Its children are where the reference browser puts
  them. Found by the print references (step 8).
- *Fixed and absolutely positioned boxes on paper* take the initial
  containing block as their containing block, the page area rounded up;
  the reference browser's take its viewport, rounded down, a pixel
  apart at the right or the bottom.
- *Synthesized bold and oblique*, which the reference browser draws for
  a font it has no bold or italic face of: `font-weight` and
  `font-style` are M21's.
- *A raster PDF's size*: a page of text at 300 dots per inch is about
  420 KiB; a PDF of text and paths (above) is the remedy.

## 3. Order of work

Each step is a vertical slice: build, tests, clippy green and a smoke
run before the next, with its sabotage pass. A step is done when its
tests pass.

1. *Media queries.* `style::media`: the parser of query lists with its
   error handling, the three-valued evaluation, the features above
   against `style::Media`; `@supports` conditions with `selector()`;
   both tested on their own against the drafts' examples. Nothing reads
   them yet.
2. *Conditional rules and layers in the cascade.* The walk over a
   sheet's nested rules with their conditions and layers; `@supports`
   decided, `@media` evaluated at each cascade; the layer order and the
   cascade's sort with it; the selector features from every rule;
   `@import`'s conditions and the `media` attribute in the loader, every
   sheet fetched and the render-blocking rule by media; the answers kept
   with the styles and evaluated again before a frame. Here a responsive
   page first responds. M20.1 and M20.2.
3. *The preferences.* `Engine::set_preferences`; the shell's options,
   winit's theme and its changes, reduced motion from Windows and macOS,
   checked by compiling the shell for both. M20.3.
4. *The WPT subset*: the rule's check of at-rules, the three directories
   pulled, the expectations by the steps to come.
5. *Print style.* `@page` with its selectors, `size` and margins; the
   break properties, `orphans`, `widows` and `print-color-adjust` behind
   the pointer; the size of `ComputedStyle` stated by its test.
6. *Pagination.* `layout::paginate`: the places to break, the rules and
   their relaxing, forced breaks and blank pages, monolithic boxes,
   margins at breaks, the struts; the page geometry from `@page`, the
   reference browser's defaults measured; tested on fragment trees.
7. *The PDF.* `Engine::print`: the print environment, cascade, layout
   and pagination apart from the window's; each page's list with the
   fixed boxes, rasterized at 300 dots per inch; `render::pdf` on
   `pdf-writer`; the shell's `--print-to-pdf`. M20.4.
8. *The reader and the print references.* `examples/print.rs capture`
   and the print pages compared by the text on each page; the engine's
   PDF read back.
9. *The golden cases*: the reference pages from 80, with the media type
   and the preferences set as the reference browser's were, and whatever
   they find.
10. *Acceptance*: `about:silver`'s print style and breakpoints, its PDF
    against the reference browser's, the responsive layouts. M20.5.
11. *Performance and fuzzing*: the benchmark, T4.3, the `media_query`
    target and the fuzz runs.
12. *The documents*: this note corrected to the code, the architecture's
    M20 entry, the roadmap's ticks (M20.1 to M20.5), its M20.3 corrected
    and the items section 2 names registered, and the numbers in
    `docs/perf-budgets.md`.
