# Design note: selectors v2 and pseudo-elements

Roadmap M16. Written before the code, as the record of the decisions;
what ships is described in `docs/architecture.md` "Current state" when
the code lands. Where this note and the code later disagree, the code
wins and this note is corrected, as the notes before it were.

The problem: the engine styles elements and nothing else. A rule whose
selector names a pseudo-element — `p::before`, `p:first-line`, a
clearfix's `::after` — fails to parse and is dropped whole, with every
other selector in its list, so generated content never appears and a
page's `.a, .b::before { … }` loses `.a` as well. Selectors the web
leans on are missing in the same way: `:lang()`, the form states
(`:disabled`, `:checked`, `:required`), `:link`, which never matches
(the UA sheet colours `a` instead), and HTML's rule that some
attributes' values compare without regard to case. `<q>` has no
quotation marks. This milestone adds the pseudo-elements CSS draws —
`::before` and `::after` with `content`, `::first-letter` and
`::first-line` — recognises the ones it cannot draw yet, and brings
the selector matcher up to Selectors 4 for what the web uses.

The standards are read at the versions current when this note was
written: Selectors Level 4 (Editor's Draft of 18 September 2026), CSS
Pseudo-Elements Level 4 (10 September 2026), and CSS 2.2 §12 for
`content` and `quotes`, since CSS Generated Content Level 3 calls
itself "a very rough draft … not ready for implementation" and is
followed only where browsers agree with it; HTML for the language of
an element, its case-insensitive attribute values and its form
pseudo-classes; RFC 4647 §3.3.2 for language ranges.

## 1. Decisions

**What is parsed.** A selector may end in one pseudo-element, after its
last compound selector:

- `::before`, `::after`, `::first-letter` and `::first-line`, and the
  same four with one colon, which CSS 2 wrote and Pseudo-Elements 4
  requires to be accepted;
- `::marker`, `::placeholder` and `::selection`, recognised and
  matched but drawing nothing yet (M16.3; see section 2);
- any `::-webkit-` name that is not a function, which Selectors 4's
  Appendix B makes valid at parse time and matching nothing, so that a
  list with `::-webkit-scrollbar` in it keeps its other selectors.

Any other pseudo-element makes the selector invalid, as now. After a
pseudo-element only the user-action pseudo-classes may follow
(`:hover`, `:focus`, `:focus-visible`, `:focus-within`, `:active`),
and `:is()`, `:where()` and `:not()` whose arguments are made of them,
since Selectors 4 §3.6.3 passes the restriction to their arguments
(`:is()` and `:where()` forgive an argument that breaks it). No
pseudo-element is hovered, focused or active until M26.4, so
`::before:hover` never matches and `::before:not(:hover)` always does.
Anything else after a pseudo-element, a combinator included, is
invalid. A pseudo-element inside `:is()`, `:where()`, `:not()` stays
invalid there, as Selectors 4 says (`:is()` and `:where()` forgive it,
as now). A pseudo-element counts in the third column of specificity,
as a type selector does.

The selector keeps its pseudo-element as a field of its own; the
compound selectors before it match the *originating element*, and a
rule whose selector matches contributes its declarations to that
element's pseudo-element instead of to the element.

**Case, as HTML and Selectors 4 say.** Every name the parser compares —
pseudo-class and pseudo-element names, attribute flags — is compared
in ASCII lowercase, not Unicode lowercase as now (`to_lowercase` turns
some non-ASCII letters into ASCII ones). Attribute selectors gain
HTML's rule: on an HTML element, the values of the forty-six
attributes HTML lists (`type`, `lang`, `dir`, `rel`, `checked`,
`disabled` among them) compare ASCII case-insensitively unless the
selector says `s`; `i` makes any value case-insensitive, `s` makes any
value case-sensitive. The flag becomes three states — none, `i`, `s` —
where it is one boolean now. Comparisons are made in place, with no
string allocated per attribute tested, as two are now. Every document
the engine has is an HTML document, so the rule always applies; the
UA sheet's `i` flags stay, harmless. The HTML tokenizer lowercases
attribute names in ASCII too, as HTML's tokenizer does, so that the
name a selector gives and the name a page wrote agree; and `class` and
the words `~=` compares are split on ASCII white space, as HTML splits
them, not on Unicode's.

**Specificity of `:nth-child(An+B of S)`.** It becomes what Selectors 4
says: one pseudo-class plus the most specific selector in S. It counts
one pseudo-class alone now.

**`:lang()`.** It takes a comma-separated list of language ranges, each
an identifier or a string. An element's language is HTML's: the
`lang` attribute of the element or its nearest ancestor that has one;
else the pragma-set default language, from the last
`<meta http-equiv="content-language">` in the document whose content
is one tag (the first run of non-white-space characters, none when
the content has a comma); else the response's `Content-Language`
field when it names one language; else unknown, the empty tag. The
`xml:lang` attribute in no namespace has no effect, as HTML says, and
there is no XML namespace in an HTML document. A range matches a tag
by RFC 4647's extended filtering, ASCII case-insensitively: the first
subtags must be equal or the range's `*`; each later subtag of the
range is looked for in order in the tag, skipping subtags but never a
singleton, `*` matching anything. `*` does not match an untagged
element; the empty range `:lang("")` matches only untagged elements.
A range or a tag that is not well-formed by BCP 47's syntax matches
nothing. Two deviations, said rather than hidden: tags are not
canonicalised (RFC 5646 §4.5, and the extlang form Selectors 4 asks
for), and a well-formed tag is not checked against the registry of
valid subtags; both need the IANA registry's data, which nothing
carries until the internationalisation work of D7 and M42.

**The form pseudo-classes (M16.4).** No form control has state yet
(interactive controls have no milestone; M26 wires events), so each of
these matches the state the element has when nothing has changed it,
which HTML defines from its attributes; that is exact for every page
until state exists:

- `:disabled` matches a `button`, `input`, `select`, `textarea`,
  `optgroup`, `option` or `fieldset` that is actually disabled: its
  own `disabled` attribute; for the first four and `fieldset`, being
  inside a `fieldset` with `disabled` and not inside that fieldset's
  first `legend` child; for an `option`, the nearest of its ancestors
  that is an `optgroup`, `select`, `hr`, `datalist` or `option` being
  an `optgroup` with `disabled`. `:enabled` matches the same elements
  when not disabled.
- `:checked` matches an `input` of type `checkbox` with a `checked`
  attribute; one of type `radio` with one that is the last with one in
  its radio button group in tree order, since each button checks
  itself and unchecks the rest of its group as it is parsed (a group
  is the radio buttons of one form owner with one non-empty `name`;
  the form owner is the form a `form` attribute names, for a button in
  the document, else the nearest `form` ancestor; a button with no
  name is alone); and an `option` that is selected, as HTML's
  selectedness setting algorithm makes it. In the list of options of a
  `select` without `multiple` — its `option` descendants, not looking
  inside a `select`, `hr`, `option` or `datalist`, nor an `optgroup`
  inside another — that is the last with a `selected` attribute, or,
  with none and a display size of 1 (`size`, else 1), the first that
  is not disabled; any other option is selected by its own attribute.
- `:required` matches an `input` whose type the `required` attribute
  applies to (every type but `hidden`, `range`, `color`, `submit`,
  `image`, `reset` and `button`) with that attribute, and a `select`
  or `textarea` with it; `:optional` matches the same elements without
  it.

The rest of HTML's form pseudo-classes (`:read-only`, `:read-write`,
`:placeholder-shown`, `:default`, `:indeterminate`, `:valid`,
`:invalid` and the range ones) wait for form controls with state.

**Links and the dynamic pseudo-classes.** `:link` and `:any-link` match
`a` and `area` elements with an `href`, as HTML says; there is no
history yet, so every link is unvisited and `:visited` keeps matching
nothing, which is also what keeps history from being read through
styles. The UA sheet's `a { color: blue; }` becomes HTML's
`:link { color: #0000EE; }` and `:visited { color: #551A8B; }`, with
its `:link:active` and `:visited:active` red, so an `a` without
`href` is no longer blue. `:focus-visible`, `:focus-within`
and `:target` are recognised and match nothing, as `:hover` and
`:focus` do now, so that a list holding one keeps its other selectors;
M26.4 makes the user-action ones match, and `:target` waits for the
engine to keep the indicated part it finds (M15).

**Pseudo-element styles.** `Styles` keeps, beside each element's style,
the styles of its pseudo-elements that some rule styles: `::before`,
`::after`, `::first-letter` and `::first-line`. A pseudo-element no
rule matches has no style kept, and so, for `::before` and `::after`,
no box, since `content`'s initial value computes to `none` on them.
Each is cascaded from its own matched declarations with its
originating element's style as the parent it inherits from, as
Pseudo-Elements 4 says of tree-abiding pseudo-elements; `::first-line`
and `::first-letter` inherit differently when laid out (below). An
element's `style` attribute styles the element alone. The rules for
`::marker`, `::placeholder` and `::selection` are parsed and set
aside, unmatched: nothing draws them yet. Restyling compares an
element's pseudo-element styles with its own, so that a change to
either counts as a change to the element, which is how layout hears
of it.

**`content` and `quotes`.** Two properties join the computed style,
growing it by 48 bytes, from 336 to 384:

- `content: normal | none | [ <string> | url() | attr(<name>) |
  open-quote | close-quote | no-open-quote | no-close-quote ]+`, not
  inherited, initial `normal`, which computes to `none` on `::before`
  and `::after`. Its computed value is the list with each `attr()`
  replaced by the attribute's value on the originating element (the
  empty string when it has none), as CSS 2.2 says. A `url()` is left
  out, as CSS 2.2 allows a user agent that cannot show the resource to
  do, until images (M23, M21.5): `content: url(x)` makes an empty box.
  `counter()` and `counters()` make the declaration invalid until
  counters exist (M21.4): leaving the number out would print "." where
  "3." was meant, and so do the typed and fallback forms of `attr()`
  (M21.5). css-content-3's alternative text after a `/` is accepted
  and set aside, as nothing reads it. `content` applies to `::before`
  and `::after` only, as in CSS 2.2: on an element, and on
  `::first-letter` and `::first-line`, it is not applied and stays
  `normal`, so that no element's own style reads an attribute through
  `attr()`; css-content-3's replacing of an element by an image waits
  with images.
- `quotes: auto | none | [ <string> <string> ]+`, inherited, initial
  `auto`. `auto` gives “ ” and ‘ ’ in every language: the choice by
  content language that css-content-3 describes needs locale data
  (M42). HTML's rendering section has been cut down to
  `q::before { content: open-quote }` and
  `q::after { content: close-quote }`, which the UA sheet gains.

The quote marks are chosen when the box tree is built, which walks the
document in order: the depth is the open quotes so far minus the close
quotes, never below zero, and a depth past the pairs given takes the
last pair, as CSS 2.2 §12.3.2 says.

**Generated boxes for `::before` and `::after`.** An element whose
`::before` or `::after` style has a `content` other than `none`, and
whose `display` there is not `none`, gets a box for it as its first or
last child, as Pseudo-Elements 4 says: the box its `display` makes
(inline by default; block, inline-block, a float, an absolutely
positioned box, a table, a table part, all as for an element), holding
one text box with the content's text, or none when the text is empty.
The text is text like any other: `white-space` and `text-transform`
treat it as they treat any text, so `"\A"` breaks a line only where
white space is preserved, and white space at the start of a block
container's run or between table parts makes no box — its quotes
still move the depth. The box tree builder walks *items* instead of
node ids: a node, or an element's generated box, or the text in one.
So generated boxes go through the same code as elements — anonymous
blocks around them, hoisting a block out of an inline, anonymous
table parts around one met in a table — with no second path to keep
in step; a generated table part takes none of its element's
`colspan`, `rowspan` or `span`. The builder builds every box in
document order, a cell where it stands among the content around it,
so the quote depth is right in tables too. Generated boxes are not
made for replaced elements, as Pseudo-Elements 4 says; in this engine
those are `img`, `input`, `textarea`, `select`, `iframe`, `video`,
`audio`, `canvas`, `embed` and `object`, and `br` and `wbr`, which
make no box of their own.

A generated box names its originating element and is marked with the
pseudo-element that generated it, and so is each box inside it and
each fragment made from them:

- hit testing answers with the originating element, so a click on an
  `a`'s `::before` follows the link, as in the browsers;
- the floats of a formatting context are told apart by node and
  pseudo-element, since an element's `::before` and `::after` floats
  name it both;
- the layout cache (M13.2) keeps and takes results by node, for boxes
  that stand for an element only; a generated box is anonymous to it.
  What a cached box drew can depend on the quotes before it, so a box
  the cache keeps records the quote depth it began at, and is taken
  only at that depth (M21's counters will join that key);
- the canvas background (CSS2 §14.2) is the root's box's, the first
  box of the document, or the body's, the first among the root's
  children that names another element: a box of the root's `::before`
  names the root and is passed over, one of the body's is inside the
  body's box, so neither becomes the canvas's source, and each keeps
  its background. For the root's box to be the first, the cascade now
  blockifies the root element's `display`, as CSS 2.2 §9.7 and
  css-display-3 §2.7 say and it did not before; a top-level change
  restyles the document node's element children, whose root may have
  changed;
- a generated box does not scroll: `overflow` other than `visible`
  clips it but gives it no scroll offset, since scroll offsets are kept
  by element and its element has its own. Its clip in the paint order
  names no element, so the wheel does not take it for its element's
  scroll container, and a fragment link scrolls to the element's own
  first box, not to one its `::before` generated. Browsers scroll one;
  no page the milestone knows of needs it, and it is said here;
- the differential checker (M13) and the paint-order walk need nothing
  new: the node a generated fragment names is in the tree.

The reference pages of generated boxes found three faults, two of
them M11's: the floats were told apart by node alone, so an element's
two generated floats were placed as one; a float in an empty box,
waiting for the flow position, was not placed before the next box
when the flow's own position was already known above the empty box,
so a context root after it did not avoid it; and a context root
beside a float had its margin added beyond the float, where the
browsers measure it from the containing block and let it lie under
the float (docs/design/floats-positioning.md is corrected).

**`::first-letter` (M16.2).** It applies to block containers — a
block, an inline-block, a float or an absolutely positioned box that
is not a table, a table cell or caption; not an inline element, nor a
table row, whose text is in an anonymous cell — and is found by a pass
over the box tree once it is built (`layout::box_tree::first_letter`):

- the first-letter text is looked for in the inline content at the
  start of the originating element's first formatted line: its own, or
  that of its first in-flow block-level child that has a line,
  recursively, so an empty block is passed over, as in the browsers.
  Floats and absolutely positioned boxes, a floated `::before` among
  them, are out of flow and passed over; a table, an inline-block or
  an inline table first has lines of its own, and there is none.
  Generated `::before` text counts, as Pseudo-Elements 4 says
  (`p::before { content: "Note: " }` makes "N" the first letter);
- white space at the start is passed over, whether it collapses or is
  kept (`white-space: pre`), and is left in a text box before the
  letter's; a line break kept by `pre`, `pre-wrap` or `pre-line` first
  makes the first line empty, and there is none, as Pseudo-Elements 4
  says of `<p><br>First`. In this engine `br` makes no box and breaks
  no line (since M7, section 2), so that example finds "F" here;
- the text is Pseudo-Elements 4's pattern within one text box:
  punctuation (Unicode P\*) with any spaces (Zs but U+3000) between,
  then one letter, number or symbol (L\*, N\*, S\*) with the combining
  marks after it, then any punctuation but opening punctuation and
  dashes (Ps, Pd), with spaces between that are not word separators.
  Punctuation in another element than the letter's (`“<b>T</b>he`) is
  not taken across it, and there is none. Typographic character units
  are taken as one scalar value with its combining marks; Dutch "ij"
  and other language tailorings wait for M42;
- the text box that holds it is split: the first-letter text goes into
  a box of its own, which names the element the text is in, as its
  text does, so a drop cap in a link is the link to the pointer, and is
  marked as the `::first-letter`'s. Its style is the
  `::first-letter`'s, cascaded again with the style of what the text
  is in as its parent (Pseudo-Elements 4 §2.2.3): the cascade keeps it
  with the originating element as parent (above), and layout asks
  `style` for it through the `Recascade` trait, which the engine makes
  from its sheets. That box is an inline box, or, with `float` other
  than `none`, a float holding the letter's line, which every browser
  honours for drop caps though Pseudo-Elements 4 leaves it to the user
  agent. When several ancestors whose first formatted line this is
  have `::first-letter` styles, their boxes nest outermost first, as
  the fictional tag sequence in Pseudo-Elements 4 nests them, each
  inheriting from the one outside it;
- the properties that apply are the ones Pseudo-Elements 4 lists that
  this engine has (font size and family, colour, background,
  `text-transform`, letter and word spacing, `vertical-align`,
  margins, padding, borders but a table's `border-collapse` and
  `border-spacing`) and `float`; the cascade leaves the others out of
  the `::first-letter` style, so `display`, `position` and `width` stay
  initial and the inherited ones the parent's;
- the inline layout already keeps text with no white space between it
  flush and unbroken, so "T" and "he" stay one word. `text-transform:
  capitalize` now takes a word split across boxes, by an inline
  element or by the first letter, as one word: "the" with its first
  letter split off is "The", not "THe", and so is `<b>t</b>he`;
- the blocks on the way down to the letter, from the originating
  element's box, lay out a style that is not their own: their
  element's restyle does not see the ancestor's `::first-letter`
  change. They are marked, and the layout cache (M13.2) neither keeps
  nor takes them.

Part of the first-letter text wrapped to the next line stays in the
pseudo-element (Pseudo-Elements 4 drops it out);
`::first-letter::prefix` and `::suffix`, and `initial-letter`, are not
built (section 2).

**`::first-line` (M16.2).** Its box is the first line, which only line
breaking finds, so it is applied by the inline layout:

- the first formatted line is found as for `::first-letter`: a block
  container's first line box, or its first in-flow block-level child's
  first formatted line, recursively. Block layout carries down to that
  child the chain of `::first-line` styles of the elements whose first
  formatted line it holds, outermost first;
- in that line, Pseudo-Elements 4's fictional tag sequence is followed
  as written: the `::first-line` pseudo-elements stand just inside the
  block whose line it is, the outermost first, each inheriting from
  the one outside it and the outermost from that block; the inline
  elements on the line inherit from the innermost, each restyled with
  its own rules and that new parent; text directly in the block takes
  the innermost's style. Only the properties Pseudo-Elements 4 lists
  apply on the `::first-line` pseudo-elements themselves (font size and
  family, colour, background, `text-transform`, letter and word
  spacing, `vertical-align`); the restyled inline elements keep all
  their own;
- the restyling needs the cascade during layout, through the
  `Recascade` that `::first-letter` brought (named so beside the
  `Restyle` that `Stylesheets::restyle` returns), which gains running
  an element's rules again with a given parent style, beside giving a
  pseudo-element's style with a given parent. It is called only for
  elements on a first line, and for the `::first-letter` box when one
  is on it (which then inherits from the first line, as
  Pseudo-Elements 4 says);
- the words are measured with their first-line styles while the first
  line is filled, and with their own after it: the first line breaks
  where its own text, in its own fonts, runs out of room, and the
  second starts with the next word in its normal style. The line's
  strut comes from the innermost `::first-line` style, since the root
  inline box's fragment on the first line inherits from it;
- a `::first-line` with a background gets an inline-box fragment
  around the line's content, named by its originating element and
  marked generated;
- the layout of a block whose first line takes styles from an
  ancestor's `::first-line` depends on that ancestor, so such a block
  is neither kept in the layout cache nor taken from it; the blocks
  that are, are invalidated by their own restyle as now.

**Invalidation (M13.1).** The selector features gain what the new
selectors read:

- the attributes the new pseudo-classes read on the element and its
  ancestors — `lang`, `href`, `disabled`, `checked`, `selected`,
  `type`, `multiple`, `size`, `required` — count as named by a
  selector, so a change to one restyles the element's subtree;
- `:lang()` reads the document's pragma: a change to a `meta`'s
  `http-equiv` or `content`, or a `meta` inserted or removed (or a
  node freed, whose kind is no longer known), cascades the document
  whole;
- `:disabled` and `:enabled` read which `legend` is a fieldset's
  first: a child inserted under or removed from a `fieldset` restyles
  the fieldset's subtree;
- `:checked` reads a select's other options and a radio button's
  group: a child inserted or removed in a `select`, or a change to an
  `option`'s or `optgroup`'s `selected` or `disabled`, restyles the
  select's subtree; any element inserted or removed, or a change to an
  `input`'s `checked`, `name`, `type` or `form`, or to any element's
  `id` (which a `form` attribute names), restyles every radio button
  in the document;
- `content: attr(x)` reads `x` on the originating element, so the
  names in `attr()` join the attribute names whose change restyles the
  element.

Each is added only when a sheet uses the selector or the function, as
now.

**Performance (M16.5).** The roadmap asks for a Bloom filter of
ancestors if the matching benchmark — 1,000 selectors over 10,003
elements — has regressed more than twice. Measured on this machine
before the work: the code as imported (M8) takes 269–347ms in release,
today's 340–400ms, so it has not. M16.5 measures again after the
milestone, which adds rules (`::before` ones are matched against every
element, as any rule is) and the case-insensitive comparison without
allocating, and builds the filter only if the threshold is passed;
M35.2's rule hashing and style sharing remain where the larger win
is. The number goes into `docs/perf-budgets.md`, which has none for
this benchmark yet, with the M8 code's beside it.

**Acceptance (M16.6).** `about:silver` shows each pseudo-element: a
drop cap by `::first-letter` floated left, a first line in capitals by
`::first-line`, `::before` and `::after` content on the cards, a `<q>`
with its quotation marks, and a `:lang()` rule giving a quotation in
another language its own marks. Beyond the page:

- unit tests for the parser (every pseudo-element, the legacy forms,
  `-webkit-`, what may follow, specificity), the matcher (HTML's
  case-insensitive attributes and the `i` and `s` flags, `:lang()`
  against RFC 4647's examples, the form pseudo-classes, `:link`), the
  cascade (pseudo-element styles, `content` with `attr()`, `quotes`,
  the restyle that a pseudo-element's change makes), the box tree
  (generated boxes of each display, in tables, on replaced elements,
  quote depth, the first-letter split and its pattern) and the inline
  layout (the first line restyled and measured, the nesting, the
  strut);
- reference pages for the boxes, built of boxes alone as the others
  are and captured from the reference browser: `::before` and
  `::after` as blocks, inline-blocks, floats, absolutely positioned
  boxes and tables; the clearfix (`::after { content: ""; display:
  table; clear: both }`); generated boxes on `html` and `body`; and the
  reference pages' mutation test, run over them;
- the fuzz targets that parse selectors, style documents and run the
  pipeline cover the new grammar and the new boxes; the selector
  target also matches each selector that ends in a pseudo-element
  against the element it would originate from.

**The layering contract.** Nothing moves. `style` keeps selectors,
the cascade and the pseudo-element styles; `layout` builds the
generated boxes, splits off the first letter and applies the first
line, through the `Recascade` trait `style` defines and implements over
the sheets (`Stylesheets::recascader`), which the engine hands to
layout with the styles it cascaded.

## 2. Not in this milestone, said rather than hidden

- *Counters* (`counter()`, `counters()`, `counter-reset`,
  `counter-increment`): M21.4. A `content` naming one is invalid until
  then.
- *Images in `content`* and the typed `attr()` of css-values-5: M23 and
  M21.5.
- *List items and their markers*: `display: list-item`, `::marker`'s
  box and its content, `list-style`: M21.6. `::marker` is recognised
  and draws nothing.
- *`::placeholder`*: the form controls draw no placeholder text yet.
  *`::selection`* and the other highlight pseudo-elements: there is no
  selection (M26).
- *`::first-letter::prefix` and `::suffix`*, *`initial-letter`*, and
  the language tailorings of a first letter (Dutch "ij"): not on the
  roadmap yet, and M42 for the tailorings.
- *`br`*: it makes no box and breaks no line in this engine, which
  this milestone found and does not change; it has no milestone yet.
  Once it breaks a line, a `br` first will end the search for a first
  letter as a kept line break does now.
- *Other pseudo-elements*: `::backdrop`, `::file-selector-button`,
  `::details-content`, `::part()`, `::slotted()`, `::cue`,
  `::view-transition` and the rest stay invalid, each with the feature
  it styles.
- *`:has()`*, *`:dir()`*, *`:scope`*, *`:defined`*, *`:state()`*: not on
  the roadmap yet; a selector using one stays invalid.
- *The form pseudo-classes beyond `:disabled`, `:enabled`, `:checked`,
  `:required` and `:optional`*, and *state* for any of them: with
  interactive form controls, which have no milestone yet, and M26.
- *Canonicalising language tags* and checking them against the
  registry, and *quotes by language*: M42.
- *A scrolling generated box*: see above.
- *`content` on an element*: with images (M23).
- *Quirks mode* (`.class` and `#id` matched case-insensitively): D14.

## 3. Order of work

Each step is a vertical slice: build, tests, clippy green and a smoke
run before the next. A step is done when its tests pass.

1. *Selector syntax.* Pseudo-elements in the selector (the four, the
   legacy forms, the three recognised ones, `-webkit-`), what may follow
   one, specificity with `:nth-child(… of S)` corrected; ASCII case
   throughout; the three-state attribute flag, HTML's case-insensitive
   attributes, comparisons without allocation.
2. *Pseudo-classes.* `:lang()` with the document's language (the
   `lang` attribute, `meta http-equiv="content-language"`, the
   response's `Content-Language`); `:link` and `:any-link` and the UA
   sheet's link colours; `:disabled`, `:enabled`, `:checked`,
   `:required`, `:optional`; the never-matching dynamic ones; their
   invalidation.
3. *Pseudo-element styles.* `Styles` and the cascade; `content` and
   `quotes`; the UA sheet's `q`; restyle comparing pseudo-element
   styles; `attr()` invalidation.
4. *Generated boxes.* The builder over items; `::before` and `::after`
   of every display, in tables, not on replaced elements; quote depth
   and its key in the layout cache; generated fragments in hit testing,
   the canvas background and scrolling; the reference pages.
5. *`::first-letter`.* Finding the text, the pattern, the split, the
   inline or floating box, nesting.
6. *`::first-line`.* The first formatted line through blocks, the
   chain, the `Recascade` element hook, measuring the first line in its styles,
   its fragment, the strut, the cache rule.
7. *Performance and fuzzing.* M16.5's measurement, and the filter if
   it is due; the fuzz targets run.
8. *Acceptance and the documents.* `about:silver`; this note corrected
   to the code; the architecture's M16 entry; the roadmap's ticks; the
   benchmark's numbers in `docs/perf-budgets.md`.
