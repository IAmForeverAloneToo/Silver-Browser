# Design note: web components, the cascade and animations (gap batch H)

The fourth batch of the G-list worked through by its priorities
(`silver-work/notes/gap-priorities.md`): the priorities' groups H (web
components), G (the modern cascade) and N (animations), with the
leftovers that sit beside them: a second document in a realm, which
template contents, `DOMParser` and `responseXML` wait on, and what gap
batch E left of colours and boxes. Written before the code, as the
record of the decisions, and corrected to the code when the batch ends:
where they disagree, the code wins, and section 4 lists the places.
What ships is described in `docs/architecture.md` "Current state", and
the limits it adds live in `docs/security.md`.

A step is checked by its crates' tests, clippy and its sabotage cases,
and committed signed but kept local; the whole gate, the acceptance and
the push come at the end of the batch. A gap found while closing these
is closed in the batch, not listed.

There are no shadow trees and no custom elements (G53): a design
system's components, GitHub's and YouTube's among them, are inert
elements whose content never appears, and a declarative shadow root
(`<template shadowrootmode>`) stays a template (G31). A page has one
document: `document.implementation.createHTMLDocument`, `DOMParser`
and XHR's `responseXML` are missing (G70), template contents share the
page's document (G47's rest), and attributes of other namespaces,
`[ns|attr]` and MathML's interfaces are not there (G51's rest).

The cascade passes over nesting, `@container`, `@scope` and
`@custom-media` (G8), which current stylesheets lean on more each year;
`revert` and `revert-layer` are unknown (G7); `@property` is dropped
(G14); `env()` is a property's alone (G18); logical properties map for
left-to-right horizontal text only, `direction` is not a property and
`:dir()` is not a selector (G24); `:has()` is invalid, and
`::details-content` and form controls' pseudo-elements are missing
(G12's rest). `display: contents`, `ruby`, `run-in` and `math` are
unknown (G230). Relative colours, `contrast-color()` and custom colour
profiles are refused (G224); infinite and NaN calculations are clamped
where they should serialize, and a relative length in a colour's
component is refused (G228).

Animations are CSS's alone: no `Element.animate()`, `getAnimations()`
or animation objects (G26), no scroll-driven timelines (G27), no
`@starting-style`, `overlay` or `interpolate-size` (G28), no user
setting that stops them (G30), and a transition of a transform from
`none` is laid out rather than composited from its start (G33). The
root's or body's overflow reaches the viewport's scrollbar but the
element still clips as its own box (G31).

This batch gives the engine all of them but what section 2 names. Its
acceptance is WPT's directories for them (fetched for it, at the
checkout's commit, 3 November 2026: `shadow-dom`, `custom-elements`,
`domparsing`, `css/css-nesting`, `css/css-conditional/container-queries`,
`css/css-cascade`, `css/css-logical`, `css/css-properties-values-api`,
`css/css-env`, `css/selectors/invalidation`, `css/css-animations`,
`css/css-transitions`, `web-animations`, `scroll-animations`), with
expectations.

The documents this note follows, at their versions of 3 November 2026:
DOM (shadow trees, slots, the flat tree, event retargeting, the
`Document` constructor, `adoptNode`), HTML (custom elements and their
reactions, declarative shadow roots, `DOMParser` and `parseHTMLUnsafe`,
template contents' owner document), DOM Parsing; CSS Scoping 1
(`:host`, `::slotted`, `::part`), CSS Nesting 1, CSS Conditional 5
(container queries), CSS Cascade 5 and 6 (`revert`, `revert-layer`,
`@scope`), Media Queries 5 (`@custom-media`), CSS Properties and Values
API 1, CSS Environment Variables 1, CSS Logical Properties 1, CSS
Writing Modes 4 (`direction`), Selectors 4 (`:has()`, `:dir()`), CSS
Display 3 and 4, CSS Ruby 1, CSS Color 5, CSS Values 4 (infinity and
NaN); Web Animations 1 and 2, Scroll-driven Animations 1, CSS
Animations 2, CSS Transitions 2 (`@starting-style`, `overlay`), CSS
Values 5 (`calc-size()`, `interpolate-size`). The reference browser,
Chrome 156, decides what the documents leave open and what is not
shipped: what it does not take, the engine does not either.

## 1. Decisions

### D67: a second document in the same arena

A realm's documents share the page's node arena: another document is a
`Document` node allocated beside the page's root, with its own
subtree, and the page's document is the one the engine renders. A
node's owner document is its tree's root document, or for a node not
in a tree the one recorded at its creation (a map kept in the arena's
rare state, for the nodes whose owner is not the page's). `adoptNode`
and moves between documents rewrite the owner; a node handle stays one
number, so a script's wrapper of a node of another document is made as
any other. Template contents are owned by an inert document of their
own, made with the first template, as HTML says.

`DOMParser.parseFromString`, `Document.parseHTMLUnsafe`,
`document.implementation.createHTMLDocument` and `createDocument`, the
`Document` constructor, and XHR's `document` response type and
`responseXML` make such documents with the tree builder and the XML
parser gap batch D made, scripts off as HTML says; a `Document` sent
as an XHR body is serialized. Rejected: a second arena a document (a
handle would have to name its arena, and every native would take two
numbers), and documents as fragments of the page's tree (`ownerDocument`
and `documentElement` would lie).

Attributes keep their namespace and prefix (gap batch D kept the name
alone for most), so `getAttributeNS` of any namespace and `[ns|attr]`
selectors with `@namespace` work; MathML's elements get
`MathMLElement`'s interface, as Chrome has it.

### D68: shadow trees in the arena, the flat tree a view of it

A shadow root is a node of its own kind (`NodeData::ShadowRoot` with its
mode, `delegatesFocus`, `slotAssignment`, `clonable`, `serializable`),
allocated in the arena and attached to its host by a host's link kept in
the rare state; its children are its tree, and its parent is none, as
DOM has it. `host` and the shadow root's of a host are those links.

The flat tree is a view the cascade, layout, hit testing and rendering
walk instead of the DOM's children: a host's flat children are its
shadow root's children; a slot's are its assigned nodes, or its own
children when none is assigned; a host's own children appear only where
they are slotted. Slot assignment (named, and manual by `assign()`) is
computed when a tree changes, as DOM's "assign slots", and kept by the
document, so `assignedNodes()`, `assignedSlot` and `slotchange` read it.

Events are dispatched along DOM's event path, retargeted at each shadow
boundary, with `composed` deciding whether one leaves a shadow root and
`composedPath()` hiding closed trees: the path is built natively, the
retargeting in the bindings beside the dispatch they already have.
Focus follows `delegatesFocus`; `:focus` and `:focus-within` match the
hosts on the way. Selection, `innerText` and the layout queries read the
flat tree.

Rejected: shadow roots as a host's hidden children (every walk of
children would have to skip them, and the DOM's tree order would lie),
and expanding the flat tree into a copy (every change would have to be
copied).

### D69: a shadow tree's styles scoped by tree

Each tree's sheets (a shadow root's `<style>` and `<link>`s and its
`adoptedStyleSheets`) form a rule set of their own. An element is
matched by its own tree's rules; its host's tree's rules reach it only
through `::slotted()` (on the tree its slot is in) and `::part()`
(`part` and `exportparts`), and a shadow root's rules reach its host by
`:host`, `:host()` and `:host-context()`. The cascade orders the trees'
declarations as CSS Scoping 1 does: for normal declarations the outer
tree wins, for important ones the inner. Inheritance follows the flat
tree. A tree's rule sets are built once and shared by its every
instance (a component of a thousand rows has one).

### D70: custom elements in the bindings, their reactions queued

The registry, definitions, upgrades and reactions are the bindings',
as the DOM objects are: `customElements.define`, `get`, `getName`,
`whenDefined`, `upgrade`, autonomous and customized built-in elements
(`is`, which Chrome ships), `ElementInternals` with form association and
`states` (`:state()`), and the lifecycle callbacks. The host reports
the reactions a native's DOM operation causes (an element connected,
disconnected, adopted, or an observed attribute changed) with the
operation's result, and the bindings run them before returning to the
script, as `[CEReactions]` says; the parser's elements are created as
undefined and upgraded in tree order when their definition is known,
synchronously for an element the parser makes after `define`, as HTML's
"create an element for a token" says, the XML parser's as well.

A `[CEReactions]` member the bindings carry out with several natives
opens one element queue for them all, run as it returns; script it
calls into (a constructor, a callback, a listener) opens its own. An
algorithm no such member runs, a reset button's activation, queues on
the backup element queue. Every node's object lives while its node can
be reached: the objects of one tree (a shadow-including root's, a
template's contents with their template's) are a group, which each
holds and which holds them all, the page's document's held for good
and each tree's holding its owner document's, so that a tree goes
whole with the collector's pass once nothing holds any object of it.
A custom element's object is that rule's like any other, with one
hold more: a tree such an object joins has its group held besides,
until a sweep after a task finds nothing else reaches the group and
the host holds no node of the tree (a mutation record's, a live
range's, an event's to fire, a media element's with events to come),
as the host may come to hold one, and the collector run, between two
sweeps. The sweep lets those holds go, collects the garbage, and holds
again the groups that lived. It runs once as many groups were held
since the last as after it (64 at least), else at every sixteenth
task, and whenever
the host asks for a collection, which it does as its document's nodes
double; that collection is the one that frees the trees no custom
element holds.

### D71: nesting, `@scope` and `@custom-media` desugared when the sheet is built

The CSS parser keeps a style rule's nested rules and at-rules (gap
batch D's parser dropped them); building the rule set desugars a nested
rule into a selector whose `&` is its parent's selector list taken as
`:is()` (with the specificity Nesting 1 gives), and nested at-rules into
the conditional blocks the rule set has. `@scope` keeps its root and
limit selectors on each rule inside it: matching walks to the nearest
scoping root, and the cascade orders by scope proximity before order of
appearance. `@custom-media` names are resolved when the sheet is built,
across sheets in order, cycles invalid.

### D72: container queries by laying out first

A `@container` block's rules are kept with their condition, and an
element's matching depends on its nearest container's size or style.
The cascade evaluates size queries against the container's last laid
out content box; after layout, the containers whose size changed the
answer of a query are restyled with their subtrees and laid out again,
at most a bounded number of passes (each resolves one more level of
nested containers, as size containment makes a container's size
independent of its content). Style queries (`style(--x: y)`) read the
container's computed custom properties in the same cascade. `container-
type` gives size containment, as Conditional 5 says.

### D73: `revert`, `revert-layer` and registered properties in the cascade

Revert keywords keep the declarations of a property ordered by origin,
layer and specificity: `revert` takes the property's value from the
origin below (author to user agent; there is no user origin), and
`revert-layer` from the layer below, the unlayered declarations being
the last layer. `@property` and `CSS.registerProperty` give a custom
property a syntax, an initial value and inheritance: its value is
computed by its syntax, so that it animates by its type, and a value
not of its syntax is the initial one. `env()` resolves in media queries
and `@page` too, where Chrome takes it.

### D74: `:has()` by searching, invalidated by its anchors

`:has()` takes a relative selector list, matched by searching the
anchor's descendants or following siblings as its combinators say, its
answers kept per anchor for a style pass. Invalidation records, for each
`:has()` in the sheets, what a change below or beside an anchor must
reach: a changed node invalidates the ancestors (and their preceding
siblings) that could anchor a `:has()` its features match, not every
element. `:dir()` reads directionality from `dir`, as HTML gives it
(`auto` from the text); `direction` becomes a property, and the logical
properties, values and shorthands map by it.

### D75: animations as one store, scripts holding their objects

Gap batch D's `Animations` store keeps CSS animations and transitions;
it now keeps script animations beside them, each a keyframe effect on a
target with its timing and composite operation, and every animation
gets a number. The bindings' `Animation`, `KeyframeEffect`,
`CSSAnimation`, `CSSTransition` and `DocumentTimeline` objects are
views of those numbers: a call reads or changes the store, and a frame
samples it as it samples CSS's. `getAnimations()` lists the store's
animations of a target in composite order. Composite operations (`add`,
`accumulate`) and `animation-composition` compose an effect onto the
value below, by each property's type.

Timelines are the document's or a scroll's: `scroll()` and `view()`
timelines (and `ScrollTimeline`, `ViewTimeline`) are progress through a
scroll container's range, read from the engine's scroll offsets at each
frame, so that a scroll-driven animation runs without script.

### D76: `@starting-style`, `overlay` and sizes from `auto`

`@starting-style` rules are cascaded only for an element's before-change
style when it had none (it was not rendered), so that its first style
transitions from them. `overlay` and `display` transition with
`allow-discrete` as Transitions 2 says. `calc-size()` and
`interpolate-size: allow-keywords` let a size transition to and from
`auto` and the other intrinsic keywords: the keyword is resolved to its
length for each frame. A transition makes its property act as if
`will-change` named it (G33), so a transform or an opacity animates on
the compositor from its start. A user preference stops animations and
transitions at their end (their events still sent), and
`prefers-reduced-motion` reports it.

### D77: `display: contents`, ruby, and the rest

`display: contents` makes no box: the box tree walks its flat children
in its place, its style inherited by them. Ruby (`ruby`, `ruby-base`,
`ruby-text` and HTML's `ruby`, `rt`, `rp`) lays its annotations above
(or by `ruby-position` below) their bases as inline boxes, as Chrome
does. `run-in` is not taken, as Chrome does not take it. `display: math`
gives MathML's elements MathML Core's layout of the elements Chrome
lays out (`math`, `mrow`, `mi`, `mn`, `mo`, `mtext`, `mspace`,
`msup`, `msub`, `msubsup`, `mfrac`, `msqrt`, `mroot`, `munder`,
`mover`, `munderover`, `mtable`), from their fonts' MATH tables where a
font has one. The root's or body's `overflow` is the viewport's, and
the element then does not clip (Overflow 3 section 3.3).

### D78: relative colours, and values past the finite

Relative colour syntax (`rgb(from c r g b)` and every function's `from`)
computes from its origin colour's channels, keeping `currentcolor` and
`calc()` of the channels to the used value; `contrast-color()` is taken
if Chrome 156 takes it, and `@color-profile` likewise. Calculations keep
infinity and NaN through to serialization (`calc(infinity * 1px)`), and
a colour's channel may hold a relative length, resolved with the
element's font when the colour computes.

## 2. Not in this batch, said rather than hidden

- Vertical writing modes (`writing-mode`'s vertical values and
  `text-orientation`), which logical properties would map by: the
  engine lays out horizontal text only, and `direction` alone decides
  the inline sides here.
- What Chrome 156 does not ship, which the engine refuses as it does:
  `display: run-in`, author-defined environment variables, and whatever
  of `contrast-color()`, `@color-profile` and `env()` in descriptors a
  probe of it finds missing.

## 3. Order of work

1. *A second document* (D67): documents in the arena, owner documents,
   `DOMParser`, `createHTMLDocument`, `responseXML`, template contents'
   document, namespaced attributes and `[ns|attr]`, `MathMLElement`.
2. *Shadow trees in the DOM* (D68): shadow roots, slots and their
   assignment, the flat tree, event paths retargeted, focus.
3. *Shadow trees styled and laid out* (D69): scoped rule sets, `:host`,
   `::slotted`, `::part`, the flat tree in the cascade and layout,
   declarative shadow roots, `adoptedStyleSheets` in shadow roots.
4. *Custom elements* (D70).
5. *Nesting, `@scope`, `@custom-media`* (D71).
6. *Container queries* (D72).
7. *`revert`, `revert-layer`, `@property`, `env()`* (D73).
8. *`:has()`, `:dir()`, `direction` and logical properties,
   `::details-content` and form controls' pseudo-elements* (D74).
9. *`display: contents`, ruby, `display: math`, the root's overflow*
   (D77).
10. *Relative colours and values past the finite* (D78).
11. *The Web Animations API* (D75).
12. *Scroll-driven animations* (D75).
13. *`@starting-style`, `overlay`, `interpolate-size`, composited
    transitions, reduced motion* (D76).
14. *The acceptance*: WPT's directories, the whole gate, T4.3, fuzzing,
    documents, then the push.

## 4. Corrected to the code

(Written at the end of the batch.)
