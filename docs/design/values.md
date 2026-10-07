# Design note: custom properties, `calc()`, counters and generated content

Roadmap M21. Written before the code, as the record of the decisions;
what ships is described in `docs/architecture.md` "Current state" when
the code lands. Where this note and the code later disagree, the code
wins and this note is corrected, as the notes before it were.

The problem has four parts. A value is a single token or a keyword: a
declaration with `calc()`, `min()`, `max()` or `clamp()` in it is
dropped, so `width: calc(100% - 2rem)`, the commonest arithmetic on the
web, leaves the box at whatever an earlier rule said. Custom properties
are parsed and thrown away, and `var()` makes any declaration invalid,
so a page that keeps its colours, spacing and type scale in `--*`
properties on `:root` — most pages built in the last five years — loses
all of them; `env()` is unknown, and `attr()` works in `content` alone,
as a string. Lists have no markers: `li` is a block, `display:
list-item` is dropped, and `counter()` in `content` makes the
declaration invalid, so neither a bullet nor a number is drawn anywhere.
And text is never decorated — no `text-decoration` property exists, so
links are coloured but not underlined — nor justified: `text-align`
takes `left`, `right` and `center`, M7's debt. This milestone adds the
math functions, custom properties with `var()`, `env()`, and `attr()`
in any property; counters and counter styles; list items with their
markers; and `text-decoration`, `text-align`'s other values and
justification.

The standards are read at the versions current when this note was
written: CSS Custom Properties for Cascading Variables Level 1
(Editor's Draft of 5 May 2025) for custom properties and `var()`; CSS
Values and Units Level 4 (Editor's Draft of 20 August 2026) for the
math functions, and Level 5 (Editor's Draft of 4 September 2026) for
arbitrary substitution, the bound on it, and the typed `attr()`, its
`<syntax>` from CSS Properties and Values API Level 1; CSS Environment
Variables Level 1 (Editor's Draft of 28 May 2026); CSS Lists and
Counters Level 3 (Editor's Draft of 28 April 2026) for list items,
markers and counters; CSS Counter Styles Level 3 (Editor's Draft of
19 August 2026) for the predefined counter styles; CSS Pseudo-Elements
Level 4 (Editor's Draft of 10 September 2026) for `::marker`; CSS Text
Decoration Level 3 (Editor's Draft of 16 December 2025) for decorating
boxes and their propagation, and Level 4 (Editor's Draft of 17 August
2026) for thickness, offset and the painting order; CSS Text Level 3
(Editor's Draft of 14 August 2026) for alignment and justification;
CSS Syntax Level 3 for what a declaration's value holds; and HTML §15.3
for the user agent rules and presentational hints of lists, links and
the decorated phrasing elements. Where the reference browser and the
web-platform-tests agree with each other against the text of a draft,
this engine follows them, and says so where it does. Questions the
drafts leave open — where an outside marker goes, how thick an
underline is and where it lies, how justification distributes space —
were put to the reference browser before this note was written, with
this engine's font, DejaVu Sans, loaded before anything was measured,
and its answers are given where they decide something.

## 1. Decisions

**Declarations and their tokens.** The css crate's declaration parser
changes in three ways CSS Syntax 3 asks for and custom properties
expose. A function token opens a block as a parenthesis does, so a `;`
or `!` inside `var(…)` or `calc(…)` no longer ends the declaration or
is taken for `!important`. A declaration's name is ASCII-lowercased
unless it begins with two hyphens: a custom property's name is
case-sensitive and compared code point by code point (Variables 1 §2),
where today every name is lowercased with Unicode's rules. And a
custom property's value may hold a `{}` block, which every other
declaration still may not. The value stays the list of tokens with the
white space at its ends trimmed, an empty value allowed.

A value's parts are its component values: a token, or a function or
bracket with everything up to its match. The cascade's parsers take
parts where they took tokens: the splitting of a shorthand at white
space or at commas respects nesting, so `margin: calc(1px + 2px) 3px`
has two parts and not four, and every parser of a number, a length, a
percentage, an angle or an integer takes a math function where it took
one token.

**Math functions (M21.2).** `style::calc` parses `calc()`, `min()`,
`max()` and `clamp()`, their names case-insensitive, by Values 4 §10:
sums with `+` and `-`, which must have white space on both sides,
products with `*` and `/`, parenthesized sub-expressions and nested
math functions, numbers, dimensions and percentages, and the constants
`e`, `pi`, `infinity`, `-infinity` and `NaN`. Nesting is bounded at 32
levels, as selectors' and media queries' parentheses are; a deeper
expression is invalid. Every expression is typed as §10.9 types it — a
length, angle, time, frequency, resolution, number or percentage, with
the percent hint — and checked where it is parsed against what the
property takes: `<length-percentage>` takes a sum of a length and a
percentage, `<length>` a length alone, `<number>` a number, a
percentage where the property says what it is a percentage of. A
product needs a number on one side and a quotient a number on the
right; dividing by zero makes an infinity of the numerator's sign, as
§10.9.1 says. An expression that fails its types is invalid at parse
time, like any other bad value. Its range is not checked there: a
`calc()` is clamped to the property's range where it is computed or
used, so `width: calc(10px - 20px)` is a width of 0, where `width:
-10px` is dropped. Infinities clamp to the largest length the engine
holds, `MAX_LENGTH_PX`, 33,554,431 pixels, which is where the reference
browser clamps `calc(1px / 0)` too; a `NaN` that reaches the top of an
expression is 0. An integer property — `z-index`, `order`, `orphans`,
`widows`, a counter's value — rounds to the nearest integer, a half
towards positive infinity, as the reference browser rounds
`z-index: calc(2.5)` to 3.

An expression is simplified where the value is computed, by §10.10:
font-relative and viewport lengths become pixels against the same
bases a plain length uses, angles become degrees, numbers fold, and
`min()`, `max()` and `clamp()` of comparable terms are evaluated. What
is left with no percentage in it is a plain value, and the computed
style holds it as it holds any. What is left with a percentage keeps
the simplified expression — a sum of a length and a percentage, or a
`min()`, `max()` or `clamp()` around such sums — until the
percentage's basis is known, as the reference browser keeps
`min(10px, 5%)`. A percentage the value computes against, in
`font-size` or `opacity`, is resolved with the rest.

They are taken everywhere a number, length, percentage, angle or
integer is: every property the engine has and every shorthand, the
grid's track sizes and `repeat()` counts, the transform functions,
`@page`'s `size` and margins, the counters' integers, and the values of
media features, where `em` and `rem` are the initial font size as the
query's other lengths are, so `(min-width: calc(400px + 10em))` is
answered where M20 left `calc()` in a query to the reference browser.
`round()`, `mod()`, `rem()`, `abs()`, `sign()`, the trigonometric and
exponential functions and the rest are section 2's.

**Values that keep a percentage.** The computed types that may hold a
percentage — `LengthPercentage`, `LengthPercentageAuto`, `Size`,
`MaxSize`, `FlexBasis`, `VerticalAlign`, and `Offset`, which becomes a
`LengthPercentage` since its sum of a percentage and a length is one —
gain a variant for an expression, `Calc(Arc<Calc>)`, the simplified
tree with the property's range. A value resolves itself against a
basis wherever a percentage resolves today, and is clamped to its range
there. Where layout treats a percentage specially — of an indefinite
height, which is `auto`, `0` or `none` by the property; in an intrinsic
contribution, where a percentage size is `auto` and a percentage margin
or padding is a percentage of zero (Sizing 3 §5.2.1); in a grid track
before its basis is known — an expression with a percentage is treated
as a percentage, which by construction every such expression is.

This costs the types `Copy` and makes each 16 bytes where it was 8,
`ComputedStyle` about 180 bytes more for the 22 fields that hold one;
the size test says the new number and why. The alternatives were
weighed and refused: an index into a table of expressions kept on each
style keeps the types small and `Copy`, but a value copied out of its
style — into a grid's track list, a table's column measures, a
transform, a child's style by `inherit` — would need its table carried
along everywhere it goes, and a wrong table is a silent wrong answer;
a tagged pointer packed into 8 bytes needs `unsafe`, which the workspace
keeps to FFI; an interner shared by the process would grow with every
viewport width a `vw` in a `calc()` has met, and never shrink. A
fragment copies its style, so the cost is paid per fragment; T4.3 says
what it is.

**Custom properties (M21.1).** A declaration whose name begins with two
hyphens, other than `--` alone, is a custom property. Its value is any
token list without a bad string, a bad URL, an unmatched `)`, `]` or
`}`, or a `;` or `!` outside a block, and with every `var()` in it
valid by `var()`'s argument grammar; otherwise the declaration is
dropped at parse time. It applies to every element and every
pseudo-element, those with restricted lists of properties included,
and is inherited. Its computed value is its value with the arbitrary
substitution functions in it replaced, or the guaranteed-invalid
value, which `initial` gives too; the CSS-wide keywords are taken as
keywords, not kept as the value. `ComputedStyle` gains one pointer,
`custom: Option<Arc<CustomProperties>>`, a map from name to computed
token list in which an absent name is the guaranteed-invalid value; an
element shares its parent's map unless it declares a custom property,
and then copies it on write.

**Substitution: `var()`, `env()` and `attr()`.** The three are
arbitrary substitution functions, replaced while a style is computed
by Values 5's Appendix A, and computing an element's style becomes
three passes over its sorted declarations: the custom properties; then
`font-size`, which may use them; then the rest.

The custom properties are computed name by name, each reference to
another of the element's own resolved before the property that makes
it, with the stack of names being resolved as the guard: a name met
again while it is on the stack makes every name from it to the top
cyclic, and a cyclic name's value is the guaranteed-invalid value, a
fallback in the cycle notwithstanding. The walk keeps its own stack, so
a chain of ten thousand custom properties each naming the next is
resolved without recursion. `var()`'s first argument is substituted
before it is read as a name, so `var(var(--name))` works as Values 5
says and the web-platform-tests ask, although the reference browser
refuses it; its fallback is substituted only when it is used.

Any other declaration whose value holds a substitution function is
valid at parse time when its property is known and the functions are
valid by their argument grammars. Where it wins the cascade, its value
is substituted and then parsed by the property's own parser; if the
substitution is the guaranteed-invalid value or the parse fails, the
declaration is invalid at computed-value time and the property is
`unset` — its inherited or initial value, not the value of a
declaration it beat, as Values 5's example of a `background-color`
given a length by `var()` shows. A value that substitutes to a
lone CSS-wide keyword is that keyword. A shorthand is substituted and
parsed whole and gives its longhands their values then, or, invalid,
leaves every one of them `unset`; its longhands written after it in the
cascade still win, the cascade applying declarations in order as it
does today. `@supports (--x: y)` becomes true, and a declaration with
`var()` in it is supported where its property is.

Substitution works on tokens, as Values 5 says it must: `var(--gap)px`
is a number followed by an identifier, never one dimension.

A substitution is bounded, as Values 5 §"Safely Handling Overly-Long
Substitution" requires: a function whose result would be longer than
64 KiB of text, counting each token's text, is replaced by the
guaranteed-invalid value instead, so thirty custom properties each
doubling the last end at the bound and not at a billion tokens. A
kilobyte of text in a custom property is a case the draft defends, and
64 KiB is ample room for it. Two more bounds keep a page from
multiplying the work by its elements. An element whose winning custom property
declarations are the very declarations its parent's were, none a
CSS-wide keyword and none reading an attribute, shares its parent's
computed map without computing it, so a chain declared on `*` is
computed once and not once per element. And the substitutions of one
cascade together produce at most 64 MiB of text; past that every
further substitution is the guaranteed-invalid value, a limit no page
meets but one built to exhaust the engine.

**`env()` (M21.3).** The variables of Environment Variables 1 §2, as a
desktop browser's window has them: `safe-area-inset-top`, `-right`,
`-bottom` and `-left` and their `safe-area-max-inset-` forms are `0px`,
the window being a rectangle; `preferred-text-scale` is 1, the engine
having no text zoom; the `viewport-segment-` variables are not defined,
the viewport never being split. The names are case-sensitive, and a
variable given more or fewer indices than it has dimensions is not
defined; an undefined variable takes the fallback, or makes the
declaration invalid at computed-value time. These are the reference
browser's answers. `env()` is substituted in properties; in media
queries and descriptors it is section 2's.

**`attr()` (M21.5).** `attr()` becomes an arbitrary substitution
function in any property, by Values 5 §8.7: `attr(<attr-name>
<attr-type>?, <declaration-value>?)`, the name matched against the
element's attributes, or for a pseudo-element its originating
element's, as attribute selectors match it — ASCII case-insensitively
on an HTML element, as the reference browser matches `attr(DATA-W
px)`; a name with a namespace prefix names no attribute, HTML having
none. Without a type, or with `raw-string`, the value is a string of
the attribute's value, an empty one when there is no attribute and no
fallback, which is what `content: attr(x)` gave since M16 and still
gives, M16's parse-time `attr()` now being this. `number` reads the
value, its white space trimmed, as one number token; a unit name, or
`%`, makes that number a dimension, and a name that is no unit takes
the fallback. The reference browser refuses `number` and an unknown
unit in the declaration, and fails a value with white space around it;
the draft is followed. `type(<syntax>)` parses the value as CSS, with
its own substitution functions replaced under the guard
«attribute, name», against a syntax of `*` or of components joined by
`|`, each a data type with an optional `+` or `#` or an identifier:
`<length>`, `<number>`, `<percentage>`, `<length-percentage>`,
`<integer>`, `<angle>`, `<time>`, `<resolution>`, `<color>`,
`<custom-ident>`, `<string>`, `<transform-function>` and
`<transform-list>` are checked; `<image>` matches nothing until images
(M23) and `<url>` is not allowed. A value that fails takes the
fallback, or makes the declaration invalid at computed-value time. The
selector features record every attribute name an `attr()` reads, in
any declaration, so changing the attribute restyles the element, as
`content`'s names have since M16. Values 5's taint — an attribute's
value may never become a URL — has nothing to guard yet: a `url(` is
one token that no substitution reaches, and `src()` and `image()` are
not parsed. M23 carries the taint when it adds them.

**Counters (M21.4).** `counter-reset`, `counter-increment` and
`counter-set` (Lists 3 §4), each a list of names with integers or
`none`: a reset's integer defaults to 0, an increment's to 1, a set's
to 0; `reversed(<name>)` in a reset makes a reversed counter, its
integer optional. They are not inherited, and with the list properties
they share a new pointer, `lists: Option<Arc<Lists>>`, whose inherited
half is copied to a child as `fragmentation`'s is. A counter's value is
a 32-bit integer, a reset, set or increment past either end clamped to
it, as the web-platform-tests ask and the reference browser does.

The counters are the box tree's, as the quote depth is: the builder
walks the document in order once, and the counters an element or
pseudo-element sees are found as it is reached. An element that
generates no box — `display: none`, a pseudo-element whose `content` is
`none` — touches none (§4.5). Each element's set of counters is
inherited from its parent and preceding sibling by the scoping the
reference browser and the web-platform-tests' `counters-scope` tests
agree on: the parent's counters, then those of the preceding sibling
whose names the parent has none of, each with the value of the counter
of the same name and creator on the element just before it in tree
order. The draft of 28 April 2026 takes the preceding sibling's
counters whatever the parent has, which those tests contradict; this
engine follows the tests and the reference browser. A reset
instantiates a counter, replacing an innermost one of the same name
created by the element or a preceding sibling and nesting inside any
other (§4.4.2); an increment or set of a counter the element does not
have instantiates it at 0 first, as `counter()` does. A reversed
counter without an integer starts at the value §4.4.2 calculates from
the increments and sets in its scope, found by a walk of the scope
ahead of the builder's. Pseudo-elements count as elements in tree
order: `::marker` first, then `::before`, the children, `::after`.

`content` gains `counter(<name>, <counter-style>?)` and
`counters(<name>, <string>, <counter-style>?)`, kept as such in the
computed value and turned into text by the builder with the counters
the box sees, where `content`'s quote marks are turned into text today.
The style defaults to `decimal`. A box's laid-out text then depends on
counters set anywhere earlier in the document, which the layout cache
cannot see by its node and inputs alone; its `quote_depth` input gives
way to a fingerprint of the generated text in the box's subtree —
quote marks, counters, markers — which the builder computes as it
makes the text, so an entry is reused only where every generated text
inside it is what it was. Quote depth alone was a special case of
this.

**Counter styles.** `style::counter_style` writes a counter's value in
a style by Counter Styles 3 §3: the systems `cyclic`, `fixed`,
`symbolic`, `alphabetic`, `numeric` and `additive`, with a range, a
negative sign, a pad, a prefix and suffix, and a fallback to the style
named when the value is out of range. The styles are §6's simple
predefined styles, as data in the form of §6's normative rules, all of
them: the numeric styles from `decimal` and `decimal-leading-zero` to
`tibetan`, the additive `lower-roman`, `upper-roman`, `armenian`,
`georgian` and `hebrew`, the alphabetic styles from `lower-alpha` to
`katakana-iroha`, the symbolic `disc`, `circle`, `square`,
`disclosure-open` and `disclosure-closed`, and the fixed
`cjk-earthly-branch` and `cjk-heavenly-stem`; the roadmap's "disc/
decimal + 3 custom" is read as the predefined styles beyond those two.
A name that is not one of these behaves as `decimal` but computes to
itself (§5), which is also what the complex styles of §7 do here
(section 2), and `none` writes nothing. In `counter()` the symbolic
styles are their characters, `•` U+2022, `◦` U+25E6 and `▪` U+25AA;
in a marker they are drawn shapes, below.

**List items and markers (M21.6).** `display: list-item` joins
`Display`, with its block forms, `block list-item`, `list-item block`,
`flow list-item`, and blockified to itself; a list item increments
`list-item` by 1, or by -1 when the counter is reversed, unless its
`counter-increment` names `list-item` (§4.6). `::marker` becomes a
cascade target of its own, whose style inherits from the list item and
to which only the properties Lists 3 §3.1.1 lets apply are applied: the
inherited text and font properties the engine has, `color`, `content`,
`direction`, `white-space`, `text-transform` and the custom properties.
The user agent sheet gives it `white-space: pre` and `text-transform:
none`, the parts of §3.1.1's rule the engine has. A marker is made
only for a list item, before its `::before`, from the first of §3.2's
conditions that holds: the marker's `content` if not `normal`, as
`::before`'s; an image, which `list-style-image` never gives until
images (M23) and so falls through; the list item's `list-style-type`
— a counter style writes the list item's `list-item` counter between
the style's prefix and suffix, `1. ` for `decimal`, and a string is
itself — or no marker for `none`. `list-style-type`,
`list-style-position` and `list-style-image`, the last `none` or a
`url()` taken and never loaded, and the `list-style` shorthand with its
rule for `none` (§3.6), are inherited.

An inside marker is an inline box at the start of the list item's
content. An outside marker is placed as the reference browser places
it, CSS leaving it undefined. The marker's box ends at the list item's
border-box start edge, on the left for `ltr` and the right for `rtl`,
whatever the list item's padding, its content's margins, `text-align`
or a float in its first line. It sits in the first line box in the
list item, found by descending into in-flow block boxes as
`::first-line` finds its line, and takes part in that line's height as
an inline box of its own font does, a large marker making the line
taller. Where the list item's content begins with a flex, grid or table
box instead, the marker is a line of its own whose baseline is that
box's first baseline, as the reference browser aligns a marker with a
flex container's first line; it aligns one beside a table differently,
which section 2 says. A list item with neither — empty, or beginning
with a box that has no baseline — gives the marker a line at the top of
its content, and is at least that tall. The marker is a child of the
list item's box, so `overflow: hidden` on the list item clips it away,
as it does in the reference browser.

`disc`, `circle` and `square` in `list-style-type`, the marker's
`content` being `normal`, are drawn shapes, as Counter Styles 3 §6.3
lets them be and the reference browser draws them. With `A` the
marker's font's ascent rounded, the shape is a square of side
`⌊(⌊2A/3⌋ + 1) / 2⌋` — 5 pixels at 16 pixels, 10 at 32 — its top
`⌊3(A − ⌊2A/3⌋) / 2⌋` below the top of the marker's ascent, a filled
circle, a circle stroked one pixel wide on that square's edge, or the
square filled. Outside, its left edge is `⌊2A/3⌋ + 7` pixels before
the list item's border edge; inside, it starts at the content edge and
the content follows `side + 1em + 1` pixels on. These numbers are the
reference browser's, measured in DejaVu Sans at 16 and 32 pixels, and
the golden cases hold them.

HTML's rules (§15.3.7) replace the sheet's list rules: `li` is a list
item with `text-align: match-parent`; `ol` is `decimal`, `ul`, `menu`
and `dir` `disc`, nested ones `circle` and then `square`; `ol`, `ul`
and `menu` reset `list-item`; nested lists have no block margins. Its
presentational hints join `dir`, which the cascade applies at the
author level with no specificity today: `ol`'s `start` and `reversed`
reset `list-item` as §15.3.7 says, to `start − 1`, or
`reversed(list-item)` with `start + 1`; `li`'s `value` sets it; and
`type` on `ol`, `ul` and `li` sets `list-style-type`. The engine has
no quirks mode, so the quirks rules do not arise.

**Text decoration (M21.7).** The properties of Text Decoration 3 and 4:
`text-decoration-line`, `none` or any of `underline`, `overline`,
`line-through` and `blink`, which is taken and never blinks;
`text-decoration-style`, `solid`, `double`, `dotted`, `dashed` or
`wavy`; `text-decoration-color`, `currentcolor` by default and resolved
to the colour of the element the decoration comes from;
`text-decoration-thickness`, `auto`, `from-font` or a length or
percentage of the font size; the `text-decoration` shorthand of the
four, in any order; and the inherited `text-underline-offset`, `auto`
or a length or percentage, and `text-underline-position`, `auto`,
`under` or `from-font`, `left` and `right` taken and acting as `auto`
in horizontal text. The non-inherited four and the inherited two share
a new pointer, `decoration: Option<Arc<Decoration>>`, as `lists` does.

A decoration is propagated through the box tree, not inherited (Text
Decoration 3 §2): from the element that names it to every inline box
and run of text in its inline formatting context, and to in-flow block
descendants and theirs, but not into floats, absolutely positioned
boxes, inline-blocks, inline tables or replaced elements. Each box
carries the decorations in force over its text, shared through an
`Arc` and grown only where an element adds one. Each keeps its origin's
colour, style, thickness, underline offset and position and font, so a
descendant's own font size or colour does not change a decoration it
did not start, as the reference browser draws one underline of the
same thickness under a paragraph's 16-pixel text and a 32-pixel span in
it.

A decoration is drawn over each run of text it decorates on each line,
across the white space and letter spacing between its words but not
across white space at a line's ends, and not across an atomic inline in
the line; the space between two words is decorated as the text it
belongs to is, the first space of a collapsed run being the one white
space processing keeps. It is drawn from the line's baseline with the
origin's font, as the reference browser draws it, measured at 12, 16,
18, 20, 24, 28, 32 and 40 pixels: with `B` the baseline, `A` the
font's ascent rounded, `D` its descent rounded and `T` the thickness —
the font size over ten for `auto`, the font's underline thickness for
`from-font`, the value given otherwise — the line is
`t = max(1, ⌊T⌋)` pixels thick for `auto`, the user agent's choice,
and `t` is `T` rounded for a given thickness, as Level 4 says; an
underline's top is `B + max(1, ⌈T/2⌉)` for `auto`, `B` plus the rounded
offset for a given `text-underline-offset`, `B + D` plus it for
`under`, and `B` plus the font's underline position for `from-font`;
an overline's top is `B − A − t`; a line-through's is
`B − A/3 − T/2`, rounded; `double` draws a second line at the first's
top plus `T + 1`, rounded. `dotted`, `dashed` and `wavy` are drawn in
the shape the reference browser gives them — dots and dashes of the
line's thickness, a wave of it — but not pixel for pixel; the golden
cases hold `solid` and `double`. A decoration is ink overflow and
extends no scrollable area.

Painting follows Text Decoration 4 and CSS2 Appendix E: for each run of
text, its underlines, then its overlines, then the text, then its
line-throughs, the decorations of outer elements below those of inner
ones. The user agent sheet gains HTML §15.3.4's rules: `:link` and
`:visited` underlined, `ins` and `u` underlined, `del`, `s` and
`strike` struck through, and `abbr[title]` and `acronym[title]` with a
dotted underline.

**Alignment and justification (M21.7).** `text-align` takes Text 3's
values, `start`, `end`, `left`, `right`, `center`, `justify`,
`match-parent` and `justify-all`, as the shorthand of `text-align-all`
and `text-align-last`: `justify-all` sets both to `justify`,
`match-parent` both to `match-parent`, and every other value sets
`text-align-all` to itself and `text-align-last` to `auto`; both
longhands are properties too. `start`, the new initial value, and
`end` are resolved at layout by the block's direction, so a right-to-
left paragraph is now right-aligned by default where it was left-
aligned. `match-parent` computes to the parent's value with `start` and
`end` resolved by the parent's direction, and to `start` on the root.
`text-justify` takes `auto`, `none`, `inter-word`, `inter-character`
and `distribute`, which is `inter-character`.

A line is justified when its alignment is `justify`: the alignment of
`text-align-last` for a block's last line and for a line ended by a
forced break, `auto` being `text-align-all`'s value or, where that is
`justify`, `start`; and `text-align-all`'s for the rest. The space left
between the end of the line's content and the end of its line box is
shared equally among the line's justification opportunities, as the
reference browser shares it, each gap growing by the same fraction of a
pixel. With `auto` and `inter-word` the opportunities are the word
separators left after white space processing — each space between
words, each preserved space under `pre-wrap` — the spaces either side
of an atomic inline included; with `inter-character` they are also the
places between typographic character units within a word. A line with
no opportunity, or whose content overflows its line box, is aligned to
its start. Justification never compresses: a line is filled only by
growing its gaps. The justified gaps are part of the text, so a
decoration runs across them. The reference browser does not take
`inter-character`, and justifies such lines by word; the draft is
followed.

**The style's size.** `ComputedStyle` gains three pointers, `custom`,
`lists` and `decoration`; `text-align-last` and `text-justify` as a
byte each beside `text_align`; and the larger percentage types above.
Its size test states the new size, the reasons, and the history
before.

**Restyle and invalidation.** A change to a custom property restyles
the element and, the property being inherited, every descendant whose
style the change reaches, as an inherited property's change does
today. The attribute names `attr()` reads are selector features, above.
The counters and markers depend on the document's order, not on an
element's style, and the box tree is built again for every layout; the
layout cache sees what they did to a box through the generated text
fingerprint, above. `env()` has no change to follow.

**Security.** The bounds above are this milestone's defences: a math
function nested 32 deep, a substitution of 64 KiB, a cascade's
substitutions of 64 MiB, a chain of custom properties resolved without
recursion, counters clamped to 32 bits, and a counter style's
representation bounded by the value's digits or, for an additive style,
by the range it is defined over. `attr()` reads attributes the page
never meant for styling; it can put one on the screen, which is
allowed, and not in a URL, which M23 must keep so. Until
`docs/security.md` (M33), the architecture's M21 entry records these.

**The web-platform-tests.** The subset gains `css/css-variables`,
`css/css-values`, `css/css-env`, `css/css-lists`,
`css/css-counter-styles`, `css/css-content`, `css/css-text-decor`,
`css/css-text/text-align`, `css/css-text/text-justify`,
`css/CSS2/lists` and `css/CSS2/generated-content`, by M17's rule, whose
check of a test's declarations now takes custom properties and the
substitution functions as the cascade takes them, and the element
`li`. Most of the variables and values tests are script tests of
computed values, which wait for scripts (M24, M25); many of the rest
use the Ahem font, which the subset does not take, or features of
those modules this note leaves out, which the rule's check of
declarations leaves out with them. `expectations.txt` gains this
milestone's section, each failure with what it asks of the engine.

**The golden cases.** Box-only reference pages numbered from 91,
captured and compared as the others are: colours and sizes from custom
properties, with fallbacks, cycles, invalid substitutions, `inherit`
and `initial` in them; `env()`; `calc()`, `min()`, `max()` and
`clamp()` in widths, heights, margins, padding, insets, grid tracks and
transforms, against definite and indefinite bases; `attr()` typed into
widths and colours; list items' symbol markers, outside and inside, at
two sizes, coloured and clipped, the items' own text transparent so
only the shapes are compared; and every decoration line in `solid` and
`double`, of each thickness rule and position, over text with no
descenders, which the reference browser's ink skipping would cut, the
text transparent and the decorations not. Text — numbers, letters,
justified lines — is compared word by word by M20's print references:
print pages of counters in nested sections and lists, every predefined
style the font can draw, `start`, `reversed` and `value`, markers inside
and outside with their content, and justified paragraphs with spaces,
inline-blocks, forced breaks, `text-align-last` and
`inter-character`, each word where the reference browser's print puts
it within M20's slack.

**Acceptance (M21.8).** `about:silver` is themed by custom properties:
its colours are declared once on `:root`, for the light scheme and
again for the dark one under `prefers-color-scheme`, and every rule
reads them with `var()`; and its sections are numbered by counters,
`1.`, `1.1` and so on, by `counter-reset`, `counter-increment` and
`::before`. Its print is checked word by word as M20's is, the
numbers included, and a test holds its colours under each scheme to the
theme's. A theming demo of the milestone's own, a box-only page of
components whose colours and sizes come from custom properties with
fallbacks, switched by a class that redeclares them, is a reference
page drawn against the reference browser's captures both ways.

**Performance and fuzzing.** A benchmark, `benches/values.rs`,
measures the cascade of the mutation benchmark's page with a theme of
custom properties on `:root` read by `var()` in its rules, against the
same page with the values written out; the computation of a chain of
custom properties at the substitution bound; the layout of a thousand
boxes sized by `calc()` against the same with plain lengths; the box
tree and layout of an ordered list of a thousand items with nested
counters; and the layout of a thousand justified paragraphs against
left-aligned ones. T4.3 compares the benchmarks before this milestone
with its own by M17's method. A fuzz target, `values`, parses and
computes math functions, custom properties with substitutions, cycles
and fallbacks, typed `attr()` and counter styles from arbitrary bytes,
checking that every computation ends within the bounds; and the three
targets that lay pages out run fifteen minutes each with the new
functions and properties in their dictionary and the milestone's pages
among their seeds.

## 2. Not in this milestone, said rather than hidden

Each item here that no milestone of the roadmap holds is registered with
the deferred items, the G list, at step 12.

- *Registered custom properties*: `@property` and
  `CSS.registerProperty()`, a custom property with a syntax, an initial
  value and inheritance of its own, computed and animated by its type
  (CSS Properties and Values API 1). Every custom property here is
  unregistered: a token list.
- *The other math functions*: Values 4's `round()`, `mod()`, `rem()`,
  `abs()`, `sign()`, the trigonometric functions and `pow()`, `sqrt()`,
  `hypot()`, `log()` and `exp()`; Values 5's typed arithmetic,
  `progress()`, `random()`, `calc-size()`, `if()`, `inherit()`, custom
  functions (`@function`) and the spread syntax, which only those
  functions' arguments use.
- *Colours*: `rgb()`, `hsl()` and the other colour functions, which the
  engine has never parsed, and the `currentcolor` keyword outside
  `text-decoration-color`; a custom property can hold a hexadecimal or
  named colour.
- *`@counter-style` and `symbols()`*, author-defined counter styles; the
  complex predefined styles of Counter Styles 3 §7, the longhand
  Chinese, Japanese and Korean styles and `ethiopic-numeric`, which
  behave as `decimal`; and `speak-as`.
- *Images*: `content: url()`, `list-style-image` and `<image>` in a
  typed `attr()`, with images (M23), which also carries `attr()`'s
  taint.
- *`env()` outside properties*, in media queries and descriptors, which
  the draft has not settled and the reference browser does not take;
  author-defined environment variables.
- *Markers on `::before` and `::after`* (`::before::marker`), `display:
  inline list-item` and `flow-root list-item`, and `marker-side`.
- *A marker beside a table*, which the reference browser places off the
  table's first row, where this engine aligns it with the row.
- *Ink skipping*: `text-decoration-skip-ink`, by whose `auto` the
  reference browser breaks an underline around descenders, where this
  engine draws it through them; `text-decoration-skip`, emphasis marks,
  `text-shadow`, and the spelling and grammar error lines.
- *Justification beyond expanding word separators and characters*:
  compressing a line, the priorities and script-specific rules of
  Chinese, Japanese, Korean and the South-East Asian scripts, whose text
  the engine cannot yet break into lines (G10), and cursive
  elongation; `text-align` with a string, for table columns.
- *The fonts' remaining properties*: `font-weight`, `font-style`,
  `font-feature-settings`, `line-height` and the `font` shorthand. M5.6
  and M8.2 pointed them at this milestone and M20's note did too, but
  none of M21's items holds them; they are registered rather than
  taken in passing.
- *Serializing custom properties as written*, comments and all, which
  only the CSSOM observes (M24, M25).
- *Sharing computed styles between elements*, which would bound the
  memory of a substitution repeated over many elements by the size of
  one; the per-cascade bound stands in for it.

## 3. Order of work

Each step is a vertical slice: build, tests, clippy green and a smoke
run before the next, with its sabotage pass. A step is done when its
tests pass.

1. *Math functions.* The css crate's three changes to declarations;
   component values in the cascade's parsers; `style::calc`, its parser,
   types, simplification and evaluation, tested on their own against
   the drafts' examples; the `Calc` variants and their resolution
   wherever layout and paint resolve a percentage; every property,
   shorthand, grid track, transform, `@page` value and media feature
   taking them; the style's size stated by its test. M21.2.
2. *Custom properties and substitution.* The custom properties in the
   computed style and the cascade's three passes; `var()` with its
   guard, fallbacks and bounds; invalid at computed-value time;
   shorthands; `env()`; `@supports`; sharing the parent's map;
   invalidation. M21.1 and M21.3.
3. *`attr()`.* The typed `attr()` in every property, `content`'s
   folded into it, the syntax subset, the selector features. M21.5.
4. *Counters.* The three properties, `counter()` and `counters()` in
   `content`, the counters in the box tree with the scoping above and
   reversed counters, `style::counter_style` with §6's styles, and the
   layout cache's fingerprint. M21.4.
5. *List items and markers.* `display: list-item`, `::marker`'s
   cascade target, the list properties, inside and outside markers with
   their shapes and places, the HTML rules and hints. M21.6.
6. *Text decoration.* The properties and shorthand, propagation in the
   box tree, the lines per run of text and their geometry, the painting
   order, the user agent rules. M21.7.
7. *Alignment and justification.* `text-align`'s values and longhands,
   `match-parent`, `text-justify`, justification in the line breaker's
   placement. M21.7.
8. *The WPT subset*: the rule's checks, the eleven directories pulled,
   the expectations by what the steps left.
9. *The golden cases*: the reference pages from 91 and the print pages
   of counters, lists and justified text, and whatever they find.
10. *Acceptance*: `about:silver` themed by custom properties and
    numbered by counters, under both schemes and in print; the theming
    demo. M21.8.
11. *Performance and fuzzing*: the benchmark, T4.3, the `values` target
    and the fuzz runs.
12. *The documents*: this note corrected to the code, the
    architecture's M21 entry with its security notes, the roadmap's
    ticks (M21.1 to M21.8) and the items section 2 names registered,
    and the numbers in `docs/perf-budgets.md`.
