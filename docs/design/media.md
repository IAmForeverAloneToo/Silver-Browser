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
none)` is right to be told; and `display-mode` is browser. A feature
neither level defines, or one this note leaves out, is unknown; the next
paragraph says what that does to a query.

**Parsing and evaluating a query.** `style::media` parses a
`<media-query-list>` from the tokens the css crate keeps, by Media
Queries 4 §3: the types `all`, `print` and `screen`, the other types of
CSS2 recognised and matching nothing (§2.3), `not` and `only`, and
conditions of features in parentheses joined by `and`, `or` and `not`
with parentheses to any depth, the three forms of a feature — plain
(`min-width: 600px`, the `min-` and `max-` prefixes on range features
only), boolean (`(color)`) and range (`(400px < width <= 800px)`, either
way round). A query that does not match the grammar becomes `not all`
(§3.2), and the rest of its list stands. A feature whose name or value
this engine does not know, and anything in parentheses the grammar does
not, is `<general-enclosed>` and evaluates to unknown; the evaluation is
three-valued as §3.2 has it, `not` of unknown being unknown and a query
whose value is unknown false. Lengths in a query are absolute or
relative to the initial values (§1.3): `em` and `rem` are 16px, the
initial font size, `ex` and `ch` half that, as the cascade takes them
without a font, and the viewport units are the viewport's. Ratios take a
slash or a lone number, and a resolution `dppx`, its alias `x`, `dpi`
and `dpcm`. A list is held as parsed, with the features it reads, so
that the engine knows which changes can change its value.

**Conditional rules in the cascade.** The cascade stops dropping
at-rules. When a style sheet is prepared (`Stylesheets::new`), its rules
are walked to any depth, the walk carrying the conditions and the layer
it is inside, and each style rule is kept with them: the media query
lists it is inside, by index into the sheet's lists, and its layer.
`@supports` is decided as it is met, since its answer is the engine's
and does not change: a rule inside a false `@supports` is not kept at
all. A rule inside `@media` is kept with its lists, and the cascade
evaluates every list once against the environment it is given and passes
over a rule any of whose lists is false. A nested `@media` or
`@supports` must hold with every rule around it (Conditional 3). The
same walk gives the selector features M13.1's invalidation reads, from
every style rule whatever its conditions, so that a rule that starts to
apply after a resize finds its elements as any rule does. `@container`
and `@scope` blocks, whose conditions this engine cannot evaluate, keep
being passed over with their rules; a `@media` met inside a style rule
waits for CSS nesting, which the css crate does not parse (section 2).

**`@supports`.** A `<supports-condition>` by Conditional 3: `not`, `and`
and `or`, which may not be mixed without parentheses, over declarations
in parentheses and `selector()`. A declaration is supported when the
cascade accepts it: `style::supports`, which M17 made for the
web-platform-tests' rule, answers whether the property is one the engine
applies and the value one its parser takes, the CSS-wide keywords
included; a custom property is not supported until M21 parses them.
`selector()` (Conditional 4) is supported when the selector parser takes
the complex selector without its forgiving rules and the matcher knows
every part of it. Anything else in parentheses, `font-tech()` and
`font-format()` among it, is false. A condition that does not parse
makes the rule invalid, and it is dropped with its block, as Conditional
3 says.

**Cascade layers.** `@layer` in its three forms (Cascade 5): the
statement, which declares layers in order without content; the block
with a name, which puts its rules in the layer; and the block without
one, an anonymous layer of its own. A dotted name is a layer inside a
layer, and a block inside a layer's block names a layer inside it. A
layer's place is its first appearance in the author's sheets, taken in
the order the cascade takes them, so that a statement at the top of the
first sheet orders layers the later sheets fill. Every layered rule gets
its layer's place in that order, nested layers ordered inside their
parent, and the rules in no layer come after every layer at their level.
The cascade's order of declarations becomes origin and importance, then
layer, then specificity, then order of appearance, as Cascade 5's
cascade sort has it: among normal author declarations a later layer wins
and the rules in no layer win over all, and among important ones the
order of layers is reversed, the first layer winning and the rules in no
layer losing to every layer. The user agent's sheet has no layers, and
there is no user origin yet (section 2). `revert-layer`, like `revert`,
is not a keyword the cascade knows yet (section 2).

**`@import` and the `media` attribute.** An `@import` may carry, after
its URL, `layer` or `layer(<name>)`, `supports(<condition>)` and a media
query list, in that order (Cascade 5); one with a block, or one after a
style rule, is ignored as before. The loader fetches the imported sheet
whatever the conditions say, and gives the cascade its rules wrapped in
the conditions: an anonymous or named `@layer` block, a `@supports`
block and a `@media` block, so that the cascade's one walk treats an
imported sheet as it treats the rules written inside those blocks. A
`<link rel=stylesheet>` or `<style>` with a `media` attribute is wrapped
the same way in a `@media` block of the attribute's value, parsed as a
media query list (HTML: an empty or absent attribute is `all`). Every
such sheet is fetched, a print sheet included, since printing and a
resize need them, but only one whose media holds blocks rendering,
HTML's rules for what holds a document's rendering and its scripts both
asking whether the media matches; one that does not is applied when it
comes, and the page is not held for it. The `load` event waits for every
sheet, as before. The preferred and alternate sets stay as M15 left
them.

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
`Engine::set_preferences` changes the preferences and asks for a frame.

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
`objc2-app-kit` crates winit already brings in there, and on Linux the
option, the desktop portal that says it being a D-Bus service the shell
has no client for (section 2). The platform code is the shell's, as the
layering rule says, and is checked here by compiling the shell for
Windows and macOS. The engine reads no preference itself. The roadmap's
M20.3 says the built-in interface's dark theme exists; it does not, the
toolbar having one light theme until settings (M37.3), which the roadmap
is corrected to say. A page in the dark scheme under a light toolbar is
what this milestone gives. The `color-scheme` property and the dark
canvas the user agent paints under a page that asks for one are not this
milestone's (section 2).

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
margins, 741 by 981 pixels — `@page` not consulted either: the margins
are the difference, and no web-platform-test takes its side, so this
engine follows the draft and records it. The page prints with its
backgrounds, as the reference browser's print to PDF does;
`print-color-adjust` (Color Adjust 1) parses, inherited and `economy`
initially, and changes nothing, since the engine economises on nothing
(section 2). The shell gains `--print-to-pdf <path>`, which loads the
page given, waits for it, prints it and exits, as the reference
browser's headless mode does, and the engine's tests print through
`Engine::print`. A print dialog, and a shortcut that opens one, wait for
the shell's dialogs (M37).

**Print: the page box and `@page`.** The `@page` rule (Paged Media 3) is
taken from the author's sheets, inside the conditions and layers its
sheet puts it in, with the selectors `:first`, `:left`, `:right` and
`:blank`, and, in its block, the descriptors `size` and `margin` with
its longhands; the page-margin boxes inside it are dropped, and the css
crate's parser, which drops every at-rule inside a block, is left to do
so. Its cascade is the same as a style rule's, specificity being the
page selectors'. `size` takes `auto`, one or two lengths, and the named
page sizes with `portrait` or `landscape`; a margin a length or a
percentage of the page box's width or height. With no `size`, the page
is the reference browser's for its print to PDF, US Letter, 8.5 by 11
inches, and with no margins, its margins, measured from its output at
step 6; `PrintOptions` may give another default size, as the reference
browser's options do. The page area of each page is the page box less
its margins, and the first page's is the initial containing block, whose
width the document is laid out at. A page whose margins leave a page
area of another width than the first's — `:left` and `:right` with
uneven sides — shows the content laid out at the first page's width from
its own left margin: one inline size for the whole document, as section
2 says. Pages are right and left in turn from a right first page, the
document being left to right.

**Print: the break properties.** `break-before` and `break-after`
(Fragmentation 3), with `auto`, `avoid`, `avoid-page`, `page`, `left`,
`right`, `recto`, `verso` and `all`, and the column and region values
parsed and meaning nothing on a page, which has no columns or regions to
break; `break-inside` with `auto`, `avoid` and `avoid-page`; the legacy
`page-break-before`, `page-break-after` and `page-break-inside` as their
aliases, `always` being `page`; and `orphans` and `widows`, inherited,
two each. They live behind one pointer in the computed style, as M19's
effects do, `None` being every initial value: the style grows by eight
bytes, to 448, which its size test states, and the box and the fragment
by the same. M17's and M19's lesson — every byte of the style is copied
into every fragment — argues for nothing more.

**Print: pagination, v1.** The document is laid out once, as on the
screen, at the page area's width, the page area's height the viewport's
for the units and percentages that read it. Pagination is then a pass
over the fragment tree, `layout::paginate`, which decides where the
pages break and moves what comes after each break to the next page,
rather than a layout that knows its pages as it goes. A break is chosen,
in document order, as the content is filled into pages:

- The places a page may break are between two blocks in the normal flow
  of the same block container, and at the start of the first or the end
  of the last (Fragmentation 3's classes A and C), and between two lines
  of a block container (class B). A block's own content is descended
  into for its places when the block itself does not fit on what is left
  of the page.
- A block is monolithic, never broken inside, when it is a replaced
  element, a box whose `overflow` clips, a flex or grid container, a
  table, an inline-block on its line, a float, or an absolutely
  positioned box (Fragmentation 3 names replaced elements and scroll
  containers; the others are this milestone's, section 2). One that does
  not fit on what is left of a page goes to the next; one taller than a
  page is put at the top of a page and sliced at its bottom, the rest
  showing on the pages after, as Fragmentation 3 allows for content too
  big for a page.
- A forced break, `page`, `left`, `right`, `recto` or `verso` between
  blocks, begins a page there; `left` and `right` add a blank page when
  the next would be the wrong side, which `:blank` matches.
- Of the places that let the content before them fit, the last is taken,
  unless it breaks one of Fragmentation 3's rules — an `avoid` between
  the blocks, a `break-inside: avoid` of a box around it, fewer lines
  before it than `orphans` or after it than `widows` in its block — when
  the last that breaks none is taken; when none does, the rules are let
  go in the draft's order, `avoid` first and the line counts last, and
  when still none fits, the content is broken where the page ends, a
  break where none is allowed being better than content lost.
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
resolution, not scaled pixels. The PDF is written by the `pdf-writer`
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
and footer), and its PDF says, page by page, where each run of text is,
in the document's coordinates at its resolution, through
Flate-compressed content streams and the ToUnicode maps of its fonts. A
small reader of that output, `crates/engine/examples/print.rs capture`,
writes for a page the text on each page with its position, and the print
tests hold the engine's pagination to it: the same number of pages, and
each run of text on the same page within a pixel or two of the same
place. This reads the reference browser's PDF, not PDFs in general, and
says so; the engine's own PDF is read back by the same reader in the
tests, to hold its structure and its pages' sizes. The look of `@media
print` styles is checked apart from pagination: the reference browser
draws a page's screen layout with the print media type when told to
(`--blink-settings=mediaTypeOverride=print`), and the engine does the
same through a test setting of the media type, so that golden pages
compare print styles pixel by pixel as they compare screen ones. The
preferences are checked the same way, the reference browser taking
`preferredColorScheme` and `--force-prefers-reduced-motion`.

**The web-platform-tests.** The subset gains `css/mediaqueries`,
`css/css-conditional` and the layer tests of `css/css-cascade` by M17's
rule, whose check of a test's style sheets now lets `@media`,
`@supports`, `@layer` and `@import` through, the conditions being
evaluated rather than read as unsupported; `expectations.txt` gains this
milestone's section, emptied as the steps land. Most of
`css/mediaqueries` are script tests of `matchMedia`, which wait for
scripts (M24); the reftests are what the subset runs. The print reftests
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

**Acceptance (M20.5).** `about:silver` gains a print style — dark text
on white, the cards and tiles bordered rather than filled, a break
before the long paragraphs — and breakpoints that stack its cards below
a width and widen its tiles above one; it prints to a PDF of several
pages, the text on each where the reference browser's print of the same
page puts it, and relays out at each breakpoint as the window is resized
across it. A page of the milestone's own, responsive with layers and
`@supports`, is drawn at the widths on either side of its breakpoints
against the reference browser's captures.

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
