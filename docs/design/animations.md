# Design note: transitions and animations

Roadmap M22. Written before the code, as the record of the decisions;
what ships is described in `docs/architecture.md` "Current state" when
the code lands. Where this note and the code later disagree, the code
wins and this note is corrected, as the notes before it were.

Nothing on a page moves unless its document changes. The css crate keeps
`@keyframes` as an unknown at-rule's raw block, and the cascade knows
none of the `transition-*` and `animation-*` properties, so their
declarations are dropped: a spinner stands still, a menu meant to fade
in is there at once, and content that starts at `opacity: 0` for an
animation with a `forwards` fill to bring in is never seen at all. The
only motion is the engine's own, the scroll animation (M9.2, timed by
the clock since M13.4), and the frame scheduler draws only when the
document or a scroll offset changes. When a change does come, a change
of `transform` or `opacity` alone — what nearly every animation on the
web changes — lays the box out again as any change of style does: the
card M19's acceptance turned through its `style` attribute spends most
of each frame in layout it did not need (`docs/perf-budgets.md`), and
the compositing note left a change that only repaints to M35.9 with the
remark that this milestone needs one. This milestone adds CSS
transitions and CSS animations with their easing functions, on the
timing model of Web Animations; the interpolation of every property the
engine has; a timeline for each document that the frame scheduler
drives; a change of a transform or an opacity composited without layout;
and the user's preference for reduced motion respected in the engine's
own motion.

The standards are read at the versions current when this note was
written: CSS Transitions Level 1 (Working Draft of 6 January 2026, which
its Editor's Draft still is) for transitions, and Level 2 (Editor's
Draft of 22 April 2026) for `transition-behavior`, transitions on
elements no longer rendered and the order of their events; CSS
Animations Level 1 (Editor's Draft of 13 September 2026) for
`@keyframes` and the animation properties, and Level 2 (Editor's Draft
of 18 August 2026) for `animation-duration: auto`, the processing of
keyframes, `animationcancel` and the table of events; Web Animations
Level 1 (Editor's Draft of 23 September 2026) for the timing model both
are defined by, the value of a keyframe effect and the side effects of
animation; CSS Easing Functions Level 1 (Editor's Draft of 16 December
2025) for the cubic Bézier and step easing functions, and Level 2
(Editor's Draft of 23 February 2026) for `linear()`; CSS Values and
Units Level 4 (Editor's Draft of 20 August 2026) §3 for interpolation;
CSS Color Level 4 (Editor's Draft of 7 October 2026) §13 for
interpolating colours; CSS Transforms Level 1 and Level 2 (Editor's
Drafts of 30 November 2025) for interpolating transforms and decomposing
matrices; CSS Grid Layout Level 2 (Candidate Recommendation Draft of 30
October 2023) §7.2.3.3 for interpolating track lists, which the grid
note pointed at this milestone; CSS Display Level 4 (Editor's Draft of 5
June 2026) §2.9 for animating `display`; CSS Cascade Level 5 (Editor's
Draft of 13 April 2026) for the two origins animations and transitions
add to the cascade and for `@keyframes` in layers; CSS Custom Properties
for Cascading Variables Level 1 (Editor's Draft of 5 May 2025) for
animating custom properties and the taint that puts on them; Media
Queries Level 5 (Working Draft of 29 June 2026) for
`prefers-reduced-motion`; and the Compatibility Standard (Living
Standard of 15 March 2026) for the `-webkit-` names. Where the reference
browser and the web-platform-tests agree with each other against the
text of a draft, this engine follows them, and says so where it does.
What the drafts leave open, and what the reference browser makes of
their corners, was put to it before this note was written — values read
from animations paused at a time, from transitions paused by a script,
and the scrolling of a box a running animation moves — and its answers
are given where they decide something.

## 1. Decisions

**The properties (M22.1, M22.2).** The cascade gains Transitions'
`transition-property`, `transition-duration`,
`transition-timing-function` and `transition-delay`, and Level 2's
`transition-behavior`, with their shorthand `transition`; and
Animations' `animation-name`, `animation-duration`,
`animation-timing-function`, `animation-iteration-count`,
`animation-direction`, `animation-play-state`, `animation-delay` and
`animation-fill-mode`, with their shorthand `animation`. None is
inherited, and each takes a comma-separated list. `transition-property`
takes `none` or a list of `all` and identifiers; an identifier that
names no property is kept and matches nothing, so that the lists' places
still line up (Transitions 1 §2.1), and `none`, or a CSS-wide keyword,
in a list of more than one makes the declaration invalid. Durations are
`<time [0s,∞]>`, a negative one invalid where it is written and clamped
to zero where a math function makes it; `animation-duration` also takes
Level 2's `auto`, which is zero for an animation on a document's
timeline, as the reference browser computes it. Delays are any `<time>`.
`animation-iteration-count` is `infinite` or a number not below zero, a
fraction included. `animation-name` is `none` or a name, an identifier
or a string: the draft and the web-platform-tests take `"foo"`, which
the reference browser drops, and the draft is followed. Times are held
as seconds in an `f64`: a math function's infinite time is the largest
finite one, as the reference browser computes `calc(infinity * 1s)` to
`1.79769e+308s`, and its infinite iteration count is `infinite`.

The shorthands follow their grammars to the letter. In one transition or
one animation the first time is the duration and the second the delay,
and a third makes the declaration invalid, as `transition: 1s 2s 3s` is
in the reference browser; `none` is a transition only alone,
`transition: none, opacity 1s` being invalid there too. A keyword that
another of an animation's properties takes is that property's before it
is a name: `animation: ease ease` is the easing `ease` and the animation
named `ease`, and `animation: paused running` is paused and named
`running`, as the reference browser reads both. A shorthand sets every
longhand for each of its items, those an item leaves out to their
initial values.

The animations of an element are as many as the items of its
`animation-name`, and its transitions as those of its
`transition-property`; the other properties of each group are lists
coordinated with that one, repeated or cut to its length (Values 4
Appendix A). The Compatibility Standard's names are aliases mapped
before the cascade looks a name up, as M17's and M19's are:
`-webkit-transition` and its four longhands, `-webkit-animation` and its
eight, and `@-webkit-keyframes` for `@keyframes`. The computed values of
the thirteen longhands live behind one pointer, `ComputedStyle::motion:
Option<Arc<Motion>>`, `None` being every initial value, as the effects
and the grid properties live behind theirs: the style grows by eight
bytes, to 656, which its size test states.

**Easing functions.** `style::easing` parses an `<easing-function>`:
`linear`; `ease`, `ease-in`, `ease-out`, `ease-in-out` and
`cubic-bezier()`, an x value written outside [0, 1] making it invalid
and a math function's clamped to it, as the draft and the
web-platform-tests have `cubic-bezier(calc(-2), …)` computing to 0,
where the reference browser rejects it; `steps()`, a count written below
one, or below two with `jump-none`, making it invalid, a math function's
rounded a half up and raised to that least, a `NaN` one invalid, with
each step position, and `step-start` and `step-end`; and
Level 2's `linear()`, whose control points are canonicalized where it is
computed (Easing 2 §2.1), as the web-platform-tests read them back.
Level 2's grammar takes a `linear()` of a single point, and its output
answers for one, but the web-platform-tests and the reference browser
both reject `linear(0)`, and are followed: two points at least. Any
number may be a math function, a `NaN` one being 0, as
`linear(calc(0/0), 1)` is `linear(0 0%, 1 100%)` in the tests. A cubic
Bézier is solved for its x by Newton's method from a close start,
falling back on bisection, to within 10⁻⁷, and extended past [0, 1]
along its tangents at the ends (Easing 1 §2.2.1); `cubic-bezier(0.1,
0.7, 1.0, 0.1)` takes a margin from 0 to 100px to 36.9156px three tenths
of the way through in the reference browser, which a test holds. A step
easing function takes the before flag (§2.3.1), so that `steps(4,
jump-start)` in a backwards-filled delay is at its bottom step, as in
the reference browser, and at the bottom of a step at the step's own
input when the flag is set. `linear()` extrapolates along its first or
last two points (Easing 2 §2.1.2). An input outside [0, 1] reaches a
keyframe's easing when the animation's own easing overshoots. An output
is kept within 10⁹ of zero (`style::easing::MAX_PROGRESS`) and is never
`NaN`, so that what interpolates by it stays finite however the numbers
were written; every interpolated value is clamped to its property's
range long before.

**`@keyframes` (M22.2).** The css crate parses an `@keyframes` block,
and an `@-webkit-keyframes` one, as a list of rules, as it parses
`@media`'s: each a qualified rule whose prelude is a list of keyframe
selectors and whose block holds declarations. `RuleSet` keeps the
keyframes rules by name as it keeps the `@page` rules: inside `@media`
blocks, whose lists decide whether a rule is in force for a medium, and
inside `@supports` and `@layer` blocks. A name is an identifier other
than `none` and the CSS-wide keywords, or a string, and is compared as
written. Of the rules of one name in force, the one in the layer last in
the order of normal layers wins, as a framework's keyframes are
overridden from a later layer (Cascade 5 §6.4), and of those the last in
the sheets' order (Animations 1 §3). A keyframe selector is `from`, `to`
or a percentage from 0% to 100%, Level 2's ranges of a timeline being
section 2's; a block one of whose selectors is otherwise, or does not
parse, is dropped. The declarations keep their tokens; one marked
`!important` is ignored (§3), as are those of the properties that are
not animatable but `animation-timing-function`, which is the keyframe's
easing, and those the cascade would not take, checked where the
keyframes are read as a style rule's are, so that a value that does not
parse sets nothing at its offset. Of a block's
`animation-timing-function`s the last valid one is its easing; one
that substitutes `var()` cannot be read apart from an element, and the
keyframe takes the animation's easing. The keyframes
are processed by Animations 2 §3.2: sorted by offset, the blocks of one
offset and one easing taken together, a property's last declaration
among them winning and each keyframe keeping its own easing, so that
`50% { width: 110px } 50% { height: 7px }` animates both, as in the
reference browser. A property that no keyframe at 0% or at 100%
names takes its underlying value there at each sample, so that `to {
width: 110px }` over a width of 10px is 60px half way, as in the
reference browser.

A keyframes rule is shared by every element that names it: nothing of it
is copied for an element. A change to the sheets that changes the rule
changes the animations using it at the next restyle without restarting
them, and one that removes it ends them (Animations 1 §2).

**The timing model.** `style::animation` is Web Animations' model for
the two kinds of animation CSS makes, with what CSS reaches of it: one
timeline, a playback rate of one, an iteration start of zero and
`replace` composition. An animation has a start time on the timeline, or
a hold time while it is paused; its effect has the delay, the iteration
duration, the iteration count, the direction, the fill mode and the
easing its properties give it. From the timeline's time a sample takes,
by Web Animations §4.6 and §4.7: the local time; the phase, before,
active or after; the active time, as the fill mode allows one in that
phase, or none when the effect is not in effect; the overall and simple
iteration progress, the end of an iteration being progress one of it and
not zero of the next; the current iteration; the directed progress, by
the direction; and the transformed progress, by the easing with the
before flag, which is set in the before phase going forwards and in the
after phase going backwards. The keyframe effect's value for a property
is §5.3.4's: the keyframes on either side of the transformed progress,
the first or the last two outside [0, 1], the distance between their
offsets turned into the interval's progress through the earlier
keyframe's own easing, and the property interpolated at that. An effect
not in effect gives no value. An iteration count of `infinite` makes the
active duration infinite, and a zero duration's iterations make it zero,
whatever their count; no step computes an infinity less an infinity or
zero times an infinity. Pausing holds the current time, and running
again sets the start time so that the current time goes on from where it
was held; an animation made paused holds the time zero, its delay
counting from there, as the reference browser shows `animation: m 1s
linear paused -0.25s` a quarter of the way through.

**The document timeline (M22.3).** Each document has one timeline. It
runs on the caller's clock: its time is a frame's `FrameTime::now` less
that of the first frame drawn for a time while the document was shown,
and zero before that frame, as a document's timeline begins at its time
origin. `Engine::tick` moves it on by one `FRAME_BUDGET`, as it moves
the scroll; `Engine::frame`, which advances nothing, leaves it where it
is; and it never goes back. A frame's time is the timeline's for
everything the frame does, and between frames the timeline keeps the
last frame's time, as a browser's keeps that of its last animation frame
through a task (HTML's update the rendering). An animation or a
transition takes as its start time the time of the first frame drawn
after it is made — the frame whose style makes it, or the next one after
a change of style between frames — as Web Animations' pending play task
resolves at the next animation frame, and until then it shows its value
at time zero. So what a frame shows stays a function of the document,
the viewport, the scroll inputs and the times passed in, which the tests
drive: a document drawn by `Engine::frame` alone, as the reference tests
draw theirs, has every animation at its start, which a negative delay or
a pause at a time moves. A document the back-forward cache brings back
finds its timeline where the clock has gone.

**Animation types and interpolation.** `style::animate` holds a table of
the engine's longhands, each with its animation type, from its own
definition and Web Animations Appendix A, and the code that reads,
writes, compares and interpolates its computed value. A value is
interpolated by its type (Values 4 §3) and the result clamped to the
property's range, so that an easing that overshoots takes an opacity to
1 and a width to zero and no further, as the reference browser does:

- *Numbers and lengths* interpolate linearly. Integers — `z-index`,
  `order`, `orphans`, `widows` — round to the nearest, a half towards
  positive infinity, so that `z-index` from 1 to 2 is 2 half way, as in
  the reference browser. Font-relative and viewport lengths are already
  pixels in the computed style.
- *A length and a percentage*, or any value that is a math function,
  interpolate to the math function `calc((1 - p) * A + p * B)` in M21's
  types, simplified, which layout resolves with its basis: a width from
  10px to 50% of a containing block 800px wide is 205px half way, as in
  the reference browser.
- *Colours* interpolate in sRGB with premultiplied alpha (Color 4
  §13.4), every colour the engine holds being a legacy sRGB one: from
  opaque red to transparent blue is `rgba(255, 0, 0, 0.5)` half way, red
  and not purple, as in the reference browser.
- *Transforms* interpolate by Transforms 1 §9. Two `none`s give `none`;
  the shorter list is extended by the identity functions of the longer
  one's; from the start, pairs of functions of one name or of one
  primitive interpolate as that primitive (Transforms 2 §14), a rotation
  about two different axes by quaternions; the three skews, `skewX`,
  `skewY` and `skew`, are each a primitive of its own, paired only by
  name, as the reference browser pairs them, where §14 makes them one;
  at the first pair that differ, the rest of each list becomes one
  matrix and the two are interpolated by decomposition in three
  dimensions (Transforms 2 §13), a two-dimensional matrix too, as the
  reference browser and the web-platform-tests have it, rather than by
  Transforms 1 §11.2's decomposition in two: translation, scale, skew,
  perspective and a rotation interpolated spherically. `matrix()`,
  `matrix3d()` and `perspective()` are matrices whatever they meet. A
  matrix that does not decompose, one with no inverse, makes the
  interpolation discrete (Transforms 1 §11). The reference browser
  agrees at each point the tests below hold: `scale(2) rotate(0deg)` to
  `rotate(90deg) translate(10px)` half way is `matrix(1.06066, 1.06066,
  -1.06066, 1.06066, 0, 5)`; `rotate(0deg)` to `rotate(720deg)` a
  quarter of the way is a half turn, the turns kept; `scale(0)` to
  `rotate(90deg)` is discrete; `perspective(100px) translateZ(0)` to
  `perspective(200px) translateZ(50px)` half way has the perspective of
  133⅓px, the reciprocals interpolated by the matrix.
- *A translation's percentage that meets a matrix* is known only with
  the box. Such an interpolation is kept as its two lists and its
  progress, a transform function of its own that the computed style
  holds and that is resolved where the box's matrix is made:
  `translateX(50%) scale(1)` to `rotate(90deg)` half way over a box
  100px wide is `matrix(0.707107, 0.707107, -0.707107, 0.707107, 25, 0)`
  in the reference browser.
- *`translate`, `rotate` and `scale`* interpolate by Transforms 2 §5,
  `none` being the identity, and a rotation about two axes spherically,
  as `rotate: x 90deg` to `rotate: y 90deg` half way is 70.5288° about
  `0.707107 0.707107 0` in the reference browser.
- *Values of several parts* interpolate part by part:
  `transform-origin`, `perspective-origin`, `border-spacing`, and a
  grid's track list when the two lists have the same tracks and
  repetitions (Grid 2 §7.2.3.3), which is discrete otherwise, as `10px
  20px` to `30px 40px` is `20px 30px` half way and to `30px 40px 50px`
  discrete in the reference browser.
- *Discrete* is every other value, and every pair a type cannot
  interpolate — `auto` and a length, `none` and a length, `auto` and an
  integer: the first value before progress one half and the second from
  it, as `position` and `z-index: auto` do in the reference browser.
  `font-family`, `white-space`, `content` and the counters' properties
  animate so in the reference browser, and the engine's other keywords
  with them.
- *`display`* is discrete too, but by Display 4 §2.9 every progress
  strictly between 0 and 1 between `none` and another value takes the
  other, so that an element animated or transitioned out of view stays
  rendered to the end, as the reference browser keeps one. Display 3's
  current draft calls `display` not animatable; Level 4 and the
  reference browser are followed. The element is inert while its style
  without animations and transitions has `display: none` (§2.9), a flag
  of its computed style, `ComputedStyle::inert`, that its descendants
  inherit: hit testing passes over its boxes and its text and over what
  is inside them, which is all that inertness can mean in an engine
  without focus.
- *Custom properties* are discrete, their values lists of tokens
  (Variables 1 §2).
- *Not animatable* are the animation and transition properties
  themselves, `direction` and `will-change`, as in the reference
  browser: a keyframe's declaration of one is ignored, and no transition
  starts for one.

**Animations in the cascade.** Cascade 5 §6.1 orders the origins,
highest first: transitions; important user agent declarations; important
user ones, of which this engine has none; important author ones;
animations; normal author ones, inline styles among them; normal user
ones; normal user agent ones. `compute_style` applies the animations'
values in each of its passes after every normal declaration and before
the important ones, and the transitions' after everything, so that a
property an `!important` declaration sets is not animated (Animations 1
§2) and a transition outranks it. The custom properties pass takes the
animated custom properties, the font size pass the animated font size,
and the last pass every other property, so that a keyframe's `em` is of
the animated font size, its `currentcolor` the animated colour and its
`var()` the animated custom property: with `font-size` animated from
10px to 30px and `width: 10em` in another animation's keyframes, the
width half way is 200px, and a border's colour animated from
`currentcolor` while the colour itself animates is mixed from the colour
of that moment, both as the reference browser has them. A keyframe's
value is therefore computed at each sample, as the element's own
declaration of it would be (Web Animations §5.3.3 computes keyframes
"using the target element"); the underlying value an implicit keyframe
takes is the element's from the cascade below the animation and the
animations before it. A transition interpolates between its start and
end values, computed values fixed when it starts. Values from animations
and transitions inherit as any value does (Transitions 1 §4), so a
colour animated on a section reaches the text inside it. A transition's
value is not applied while a CSS animation animates the same property of
the element (§4). Of several animations of one property the last in
`animation-name` wins (Animations 1 §2).

An element's `::before`, `::after` and `::marker` are animated as the
element is, as the reference browser animates them, a marker only in the
properties a marker takes; `::first-letter` and `::first-line`, which it
does not animate, are not animated here either, their animation and
transition properties having no effect. Layout's cascade of an element
on a first line (M16's `Recascade`) applies the element's animations at
the timeline's time as the cascade does, and starts none. A custom
property an animation sets is tainted (Variables 1 §2, §3), a mark kept
with each value in the custom properties' map: a custom property whose
value substitutes a tainted one, by `var()` or its fallback, or that
inherits one, is tainted too; and `var()` of a tainted one in a property
that is not animatable substitutes the guaranteed-invalid value, its
fallback taken if it has one, so that `animation-duration: var(--d)`
cannot be animated by animating `--d`.

**CSS animations (M22.2).** An element's animations are those its
`animation-name` names whose keyframes are in force; a name that has
none makes no animation until a rule of that name comes, and then starts
one (Animations 1 §2). When the list changes, the animations are matched
to it as §2 says: the new list is walked from its last name to its
first, each name taking the last animation of that name not yet taken,
which keeps its time and takes the properties of its new place; a name
that finds none starts a new animation, and an animation no name takes
is cancelled. A change to the other properties applies as if the
animation had had them from its start, so that a shorter delay can end
it at once. An element whose style without animations and transitions
has `display: none`, or whose ancestor's has, has no animations: making
it so cancels them, and showing it again starts them anew (§2); taking
it out of the document cancels them too, and so moving it elsewhere in
the document starts them again, as the reference browser does, whatever
its place before and after. While an animation is current
or in effect, the engine acts as if the element's `will-change` named
each property the animation animates (Web Animations §5.6): an animated
transform or opacity makes the box a stacking context, a containing
block for the boxes inside it as a transform does, and a layer by the
compositing note's policy, for the animation's whole run, its delay with
it, so that its values never change what layout made of the box (below).
The computed `will-change` is not changed: the side effect is kept
beside it, in the effects (`Effects::animated`), the two together being
the `will-change` in effect. A CSS transition has no such side effect
here.

**CSS transitions (M22.1).** By Transitions 1 §3, to the letter, at each
change of style of an element that was rendered at the last change of
style and is now. Its before-change style is its style as the last
change left it, with its animations and transitions brought to the time
of this one, which in this engine is the style the cascade holds, every
element whose animated values change being restyled at each frame's
time; its after-change style is the new cascade's with the animations'
values and without the transitions', inheriting from its parent's
after-change style, so that a child's own transition of an inherited
property runs once, from the old value to the new, rather than chasing
its parent's transition, as in the reference browser. The after-change
style of an element whose transitions run is kept beside its style for
its children to inherit from; it is the cascade's, and layout does not
read it, so that a transition's start or end changing it alone lays
nothing out. For each longhand whose values in
the two differ, the last item of `transition-property` that names it —
by its name, by a shorthand of it, or by `all` — gives its duration,
delay, easing and behaviour; a transition starts when the duration's
positive part and the delay together are above zero and the two values
are transitionable: interpolable, or discrete with `allow-discrete`
(Transitions 2 §2.5); none starts for a property a CSS animation of the
element animates, or animated at its last change of style, as the
reference browser starts none when the animation is taken away either.
A running transition whose end value is no longer
the after-change value is cancelled, or replaced by one from its current
value, or reversed, its reversing-adjusted start value and shortening
factor making a change undone half way take half the time back (§3.1).
A completed transition is kept while the element's style before a change
and after it still differ in its property, which is when the rules ask
of it, and forgotten after, so that what is kept is bounded by what
runs. `all` names every animatable longhand but the
custom properties, as the `all` shorthand does, which the reference
browser agrees with; a custom property named by itself transitions,
discretely and so only with `allow-discrete`, as the drafts say, where
the reference browser does not transition it at all. A transition ends
at its end time. An element no longer rendered, judged by its style with
the transitions' values so that a `display` transitioning out keeps it
rendered to the end, loses its transitions, as does one taken out of the
document (Transitions 2 §3). A change of `transition-*` leaves a running
transition as it is, unless `transition-property` no longer names its
property, which ends it at once at its end value (Transitions 1 §3).

A whole cascade, made when the sheets change or a breakpoint is crossed,
is a restyle of every element against the styles it replaces, which are
their before-change styles, so that a change of sheets starts
transitions as a change of class does, as in the reference browser
(`Stylesheets::cascade_from`); the engine keeps the styles it shows
through a change of the sheets, the viewport or the medium, marking them
to be cascaded again rather than dropping them, so that there are styles
to replace. The cascade records whether each element is rendered, which
the next change of style asks.

**Events (M22.2).** Each sample compares an animation's phase and
iteration with the last sample's and queues the events of Animations 2
§5.1's table, `animationstart`, `animationiteration`, `animationend` and
`animationcancel`, and of Transitions 2 §5.1, `transitionrun`,
`transitionstart`, `transitionend` and `transitioncancel`, each with its
target and pseudo-element, its animation's name or its transition's
property, and the elapsed time the tables give. Nothing can listen to
them yet: `Engine::animation_events` gives the last frame's, for the
tests and for M26's event loop, whose rendering steps will dispatch them
(M26.1). `Engine::animations` lists the animations and transitions an
engine runs, with their targets, phases and times, for the tests.

**The frame (M22.3).** A frame drawn for a time does, in order: the
timeline takes the time, and the animations and transitions made since
the last frame their start times; each is sampled, its events queued,
and each target whose values can differ from the last sample's — one
with an animation playing in its active phase, or one whose animation
changed phase — is marked to be restyled; the style stage restyles those
with the document's changes, at the frame's time, starting what
animations and transitions the new styles ask for; layout runs if what
changed reaches it; the composited changes below are written into the
layout; and paint and raster follow as they do now. The sampling counts
in the frame's style time. `Engine::needs_frame` is also true while an
animation or a transition plays in its active phase, or waits for its
start time; `Engine::next_frame_at` says when a frame is next wanted
when none is now — the end of the nearest delay — so that the shell,
which asks for a frame at each refresh while one is needed, waits
through a delay with winit's `WaitUntil` rather than drawing frames that
show nothing new. An animation that never ends asks for a frame at every
refresh while its document is shown, as in every browser.

**Composited changes (M22.4).** The restyle sorts the changes it makes.
A change of an element's style, or of its pseudo-element's, in
`transform`, `translate`, `rotate`, `scale` and `opacity` alone, which
leaves the box what it was to layout, is composited and the layout is
kept. What it was to layout is three things, each of which decides a
change alone in the tests: a transform or none, since a transform makes
the box a containing block for every box inside it; a stacking context
or not, judged as for a box no transform applies to, since the restyle
cannot tell whether the box is transformable, so that an opacity from 1
to less is a change of layout unless the box is a stacking context
already, by `will-change` or an animation's side effect; and its 3D
content flattened or not. The other effects being equal, these imply
the rest, a containing block for absolutely and fixed positioned boxes
and a stacking context when the box is transformable. The element's
descendants are restyled still, since an explicit `inherit` can read
those properties, each change of theirs sorted by the same rule. A
restyle to the style an element had leaves that style in place, whose
effects the boxes carry: the fragments of the element's node and of its
pseudo-element, which no anonymous box's style shares, are found by
those effects and take the new ones, in the layout shown and in the
layout cache's entries that hold them, so that a later layout taking
them from the cache has them too, all of a restyle's changes in one walk
of each. When a transform changed, the box, or the outermost transformed
box around it that clips nothing in the same scroll container, whose
projection holds it, is measured before the change and after it, and
where it reached, or now reaches, the bottom of its scroll container or
of the document, to within half a pixel, the scrollable overflow is
measured again by laying the document out from the cache: a transform's
projection is part of the scrollable overflow (M19), and the reference
browser moves it with a running animation: a 100px scroller's
`scrollHeight` follows a 50px box an animation moves down inside it,
548px when the box is 498px down. A fixed box reaches no such edge. Hit
testing and painting read the fragments, so they follow the change.
Every frame of a CSS animation of a transform or an opacity makes such a
change, the side effect above keeping the box what it was to layout
throughout: the frame restyles the element and composites its layer,
laying nothing out and drawing the layer again only at a new raster
scale, as M19 draws it, though the document's tiles under it are drawn
again where it reaches until a layer tree composites it over them
(M35.10). A scale animation therefore draws its layer at each frame;
keeping a layer's raster scale through an animation is M35.10's, which
the roadmap holds. A transition from one transform to another is
composited from its start to its end; one from `none`, or of an opacity
from 1, lays the box out where it becomes a stacking context, a
transition having no side effect here. Changes that are not animated
are composited by the same rule: the card M19's acceptance turned
through its `style` attribute now lays nothing out, its transform
changing from one value to another. A frame whose changes reach nothing
layout reads — an attribute's change that restyles nothing, say, or a
transition's start, which changes only the after-change style — keeps
the layout as well, where before every change of the document dropped
it; a change of the tree or of its text, or of an attribute layout reads
itself, `colspan`, `rowspan` or `span`, lays it out whatever the styles,
and drops it when it is made, between frames, as every change did
before, so that the frame does not pay for freeing it.
`Engine::layouts_kept` counts the frames that kept it.

Everything else animates by restyle and layout, as the roadmap says: a
colour, a width, any property. A colour's change lays out again the
boxes it reaches though it only repaints, the paint-only change of
M35.9, for which fragments must refer to their styles rather than hold
copies of them.

**Reduced motion (M22.5).** CSS animations and transitions run as the
page wrote them under either preference. The `prefers-reduced-motion`
query (M20) is the page's means of honouring the user, as in every
browser; an engine that stopped animations would change what pages show
at their ends and when their events come, and Transitions 1 §10.1 leaves
stopping or slowing them to a user agent's settings, which section 2
registers. The engine's own motion is the system's that Media Queries 5
§12.1 asks to keep to what is essential: under `reduce`, a scroll by the
wheel or by the keys jumps to its target rather than animate there, the
document's and a scroll container's alike, and `Engine::needs_frame`
asks for no frames for it but the one that shows it; reduced motion
asked for while a scroll is on its way puts it at its target at once. A
scroll to a fragment, or one restored, jumped already whatever the
preference.

**Printing.** The print cascade (M20) takes the screen's animations and
transitions as they stand at the timeline's time and starts none of its
own, so that content an animation with a `forwards` fill brought in
prints with it; an animation that only the print styles name is drawn at
its start.

**The style's size.** `ComputedStyle` gains the pointer `motion`: 656
bytes where it was 648. The side effects — the properties animations act
as if `will-change` named — and the interpolated transform function live
in the effects, behind their pointer; inertness is a flag, `inert`,
which the padding held. Its size test states the new size and the
reason.

**Restyle and invalidation.** The animated targets join the restyle of
each frame as elements whose own style changed, restyled from their
parents' styles with their selectors matched again; a change of an
inherited property restyles the descendants it reaches, as any does, and
an animation of the root's font size cascades the document whole at each
frame, as any change of it does (M13.1). Keeping an animated element's
matched rules between frames, so that a frame of its animation does not
match its selectors again, is restyle work for M35.2. A page that
declares no animation or transition pays a few tests and a copy or two
of the larger style for each element computed, the tables of
animations and transitions not asked while they are empty: 4% more
work in a whole cascade under callgrind, which T4.3 measures within its
10% (`docs/perf-budgets.md`). The styles of elements taken out of the
document are forgotten with their animations and transitions, whose
`animationcancel` and `transitioncancel` are queued.

**Security.** A document runs at most 65,536 animations and transitions
together, `MAX_ANIMATIONS`; past that a new animation does not start and
a new transition does not run, its property taking its new value at once
— a limit no page meets, but one built to exhaust the engine. Keyframes
are kept once for each rule, however many elements name them, and a
sample computes the two keyframes around its progress for each animated
property, so a sample's work is bounded by what the sheets hold and the
bound above. An interpolated transform kept for its box nests inside
another when a transition is interrupted during one, at most eight deep
(`style::animate::transform::MAX_MIX_DEPTH`), past which the
interpolation is discrete. Times are `f64` seconds, infinities clamped
and a `NaN` zero, an easing's output is finite and within 10⁹ of zero,
and no step makes an infinity of finite inputs that the next step
cannot take. A frame's events are a few
for each animation. The layers animations make stay within M19's layer
budget, past which a box is drawn through: Transitions 1 §8 asks that a
page not exhaust graphics memory by animating many elements or large
ones. The new fuzz target below holds the engine to these from arbitrary
bytes, and until `docs/security.md` (M33) the architecture's M22 entry
records them.

**The web-platform-tests.** The subset gains `css/css-animations`,
`css/css-transitions` and `css/css-easing`, at the commit the subset is
pinned to, by M17's rule, whose check of declarations now takes the new
properties and `@keyframes`. Most of these directories' tests are script
tests, which wait for scripts (M24, M25); their reftests draw at time
zero with paused animations and negative delays, which `Engine::frame`
draws as the reference browser's first frame does. The tests of the
directories taken before that the rule left out for these properties —
34 for `@keyframes`, 10 for `transition`, 9 for `animation` and a few
for the longhands, by the first reason it found — are decided again by
it. As built, 36 tests join, 13 of them from the directories taken
before: most of those the rule had left out wait for scripts or use
properties the engine has not got, which their second reason says. One
leaves, `css-variables/variable-reference-30`, whose M21 manifest was
pulled before that milestone made the cascade drop its declaration. The
subset holds 1,738 tests, 1,610 passing. `expectations.txt` gains this
milestone's section, each failure with what it asks of the engine: the
four are gaps outside it, the root's overflow not carried to the
viewport (two), declarative shadow roots, and a layer's raster scale
capped at 64 under a scale of 100 (M35.10).

**The golden cases.** Box-only reference pages numbered from 100,
captured and compared as the others are, each animation paused at a time
or delayed negatively so that the reference browser's first frame and
the engine's show the same moment: 100, the types — lengths, percentages
and lengths mixed, integers, numbers, colours through transparency —
each at three points, and an easing's overshoot clamped; 101, every
easing keyword, `cubic-bezier()`, `steps()` at each position, the before
flag in a backwards-filled delay, and `linear()`, as bars side by side;
102, the timing: negative delays, each fill mode before and after, each
direction, fractional iteration counts and the moment of an iteration's
end; 103, transforms: pairs of one primitive, `none` and the identity,
matrices decomposed in two and three dimensions, perspective, the
discrete fallback, percentages resolved with the box, and `translate`,
`rotate` and `scale`; 104, keyframes: implicit ends, offsets repeated, a
keyframe's easing, `!important` ignored, the winning rule by layer and
by order, `@-webkit-keyframes`, `::before`, `::after` and `::marker`
animated, `display` and other discrete properties, and a custom property
read by `var()`. Transitions need a change after the first frame. A page
may ask for changes with `<meta name="reference-change">`, each an
element's id, an attribute and its new value, the changes separated by
semicolons, and for the times at which they are made and the page drawn
with `<meta name="reference-time">`, in milliseconds from the first
frame: `capture.sh`'s browser makes them by the page's
`reference-change.js`, which this engine does not run, holding every
running animation at each change's time while it makes the change, so
that the transitions it starts begin there, and at the drawing's time at
the end, an animation the page pauses keeping its own; and
`tests/reference.rs` makes them through `Engine::mutate` between frames
drawn at those times. 105 and 106 are transitions by those means: a
transition, its delay, one reversed half way, `all` and a shorthand
named, an inherited colour reaching the box inside, an animation
outranking a transition, `display` leaving with `allow-discrete` and
without, a transform and an opacity, and one retargeted half way. A
child's own transition of the colour it inherits is left out: holding
its parent's transition through the API is a change of style in which
the reference browser retargets the child's, which in real time it does
not, so that the capture cannot show what the engine draws. The pages
write colours in hexadecimal and take `currentcolor` from a border's
default, `rgb()` and a written `currentcolor` being G16's. The
change-and-restore test of M13 makes its steps before a page's own
changes, since an element its steps hide has its transitions cancelled,
and each page then draws as captured.

**Acceptance (M22.6).** Two demos of the milestone's own,
`tests/demos/spinner.html` and `colour-fade.html`, each a page drawn at
1280 by 800: a spinner, a square ring, `border-radius` being in no
milestone (section 2), turned by an infinite linear animation of
`rotate` on a page of text; and a colour fade, a card whose background,
text and border colours alternate between a light and a dark theme
among cards that stay, the dark border chosen so that its half way is a
whole colour. Each is drawn for ten seconds of frames sixty a second
through `Engine::frame_at`, on the software backend and on the GPU
backend where the machine has an adapter, and each frame's time is
recorded: sixty frames a second is every frame within `FRAME_BUDGET`,
and the jank budget is no frame over 20ms. `tests/demos.rs` holds what
makes that so whatever the machine: the spinner's frames lay nothing out
and composite its layer without drawing it again, the document's tiles
under it drawn again where it reaches (M35.10); under reduced motion it
draws one frame; and the colour fade's frames lay out seven boxes, the
card, the three inside it and the three around it, and take the header
and the other two cards whole from the layout cache. Pages 107 and 108
are the demos held at a moment, an eighth of a turn and half way between
the themes, and 109 the spinner under reduced motion, which the page's
own query stops (`reference-reduced-motion`), drawn against the
reference browser's captures.

**Performance and fuzzing.** A benchmark, `benches/animation.rs`, runs
the acceptance's demos and measures the frame of a transform animated on
the mutation benchmark's page of a thousand paragraphs, against a colour
animated there; a thousand elements animated at once; the cascade of
that page with `transition: all` on every element and a class changed,
against the same without transitions; and `composite.rs`'s turned card
again, now composited. T4.3 compares the benchmarks before this
milestone with its own by M17's method. A fuzz target, `animations`,
parses keyframes, easing functions and the new properties from arbitrary
bytes, evaluates the easing functions and the timing model at arbitrary
times, interpolates pairs of computed values the cascade made from the
bytes, checking that every result is finite, within its property's range
and its bounds; and draws a page with animations at a series of times,
checking each frame against a fresh engine drawn at the first time and
then at that one. The `mutation` target's styles take transforms,
opacities, animations and transitions, and two of its sheets keyframes;
its comparison draws a frame, so that the changes no frame has drawn
start their transitions, and finishes the mutated engine's transitions
before it compares, as a script's `finish()` would
(`Engine::finish_transitions`), since a fresh engine has none, the start
stepped down an ulp at a time where the time less the end time rounds
to a current time short of it; the targets that lay pages out run
fifteen minutes each with the new properties in their dictionary and
the milestone's pages among their seeds.

As built (step 12), the benchmark measures those and a class change
under `transition: all`; `docs/perf-budgets.md` has its numbers, and
the acceptance's on both backends. It found `layout::composite` walking
the layout and its cache once for each change, a thousand opacities
406ms a frame, now one walk for all of a restyle's changes; and the
restyle counting an element changed when only its after-change style
did, so that the frame starting a transition laid the page out, now
that layout reads only the style. T4.3 found three costs every page
paid, mended: the previous layout freed inside the next frame's style
stage, now when the change is made, as before, for a change layout
reads whatever the styles; a default `Motion`, thirteen lists, built to
compare every computed style with; and the empty tables of animations
and transitions hashed into for every element's pseudo-elements. The
cascade then does 4% more work than before M22, and the rows of T4.3
over 10% do the same work as before, which callgrind's counts of
instructions, cache misses and branches show. The `mutation` target
found, within a minute, the change that no frame had drawn, above. The
targets ran fifteen minutes each, `mutation` and `animations` again on
the code as committed; nothing else failed.

## 2. Not in this milestone, said rather than hidden

Each item here that no milestone of the roadmap holds is registered with
the deferred items, the G list, as step 13 did; the number follows each.

- *The Web Animations API*: `Element.animate()`, `getAnimations()`, the
  `Animation`, `KeyframeEffect`, `CSSAnimation` and `CSSTransition`
  objects, playback rates, seeking and reversing, iteration start, and
  the `add` and `accumulate` composite operations with Animations 2's
  `animation-composition`. The model here has room for them; nothing
  reaches it but CSS until scripts come (M24, M25). G26.
- *Scroll-driven animations*: `animation-timeline`, `scroll()` and
  `view()` timelines, `animation-range`, Level 2's keyframe selectors of
  a timeline's ranges, and Animations 2's `animation-delay-start` and
  `animation-delay-end`, which the reference browser does not take. G27.
- *Entry transitions and sizes*: Transitions 2's `@starting-style`,
  which gives an element just shown a style to transition from;
  `overlay`; and `interpolate-size`, by which a size transitions to and
  from `auto` through `calc-size()` (G15). An element shown here appears
  in its final style, and a size from `auto` changes at once. G28.
- *A compositor thread*: animations of a transform or an opacity
  composited by a thread of their own, keeping their rate while a page's
  own work takes longer than a frame. Here they are composited without
  layout but on the engine's one thread, between that work. G29.
- *A user's setting to stop or slow animations* (Transitions 1 §10.1),
  with the events still sent, which is a settings page's to offer. G30.
- *Dispatching the events* to listeners, and `requestAnimationFrame`'s
  callbacks in the frame: the event loop and its rendering steps (M26.1,
  M26.2), with the DOM objects of M25.
- *`getComputedStyle`* of an animated element, which reads the styles
  this milestone computes (M25.4).
- *Hover and focus starting transitions*: the dynamic pseudo-classes
  (M26.4). Until then a transition starts from a change of the document
  or of its sheets.
- *Keeping a layer's raster scale through an animation*, so that a scale
  animation composites its layer rather than drawing it at each frame
  (M35.10).
- *A paint-only change that only repaints*: fragments that refer to
  their styles, so that a colour's animation lays nothing out (M35.9).
- *Restyling an animated element without matching its selectors again*
  (M35.2).
- *Animated images* (M23), and SVG's SMIL animations, which M32.3
  rejects.
- *The properties the engine has not got* — `visibility`, `box-shadow`,
  `filter`, `background-position`, `line-height`, `font-weight` (G22)
  and the rest — animate by their types when they come; each declares
  its type in the table above. `border-radius`, `box-shadow` and
  `filter`, which no milestone holds, and for want of the first of which
  the acceptance's spinner is a square ring. G32.
- *What the subset asks of the rest of the engine*: shadow trees, which
  no milestone holds, `animation-name-in-nested-shadow` attaching its
  declarative shadow roots; and the root's or the body's overflow
  carried to the viewport (Overflow 3 §3.3), which tests since M17 have
  asked for. G31.
- *A transition's side effect*: a CSS transition does not act as if
  `will-change` named its property, as Web Animations §5.6 asks of every
  running animation, so that a transition of a transform from `none`,
  or of an opacity from 1, lays its box out where it becomes a stacking
  context; between two values that keep it what it was to layout it is
  composited throughout. G33.

## 3. Order of work

Each step is a vertical slice: build, tests, clippy green and a smoke
run before the next, with its sabotage pass. A step is done when its
tests pass.

1. *The properties and easing functions.* The css crate's `@keyframes`
   and `@-webkit-keyframes` blocks; the transition and animation
   longhands, their shorthands and aliases, parsed and computed into
   `Motion`; times; `style::easing`, parsed and evaluated, tested on its
   own against the drafts' examples and the reference browser's values;
   the style's size stated by its test. M22.1, M22.2.
2. *Interpolation.* `style::animate`'s table of the longhands with their
   animation types; interpolation of each type with clamping; transform
   lists, primitives, the decomposition of two- and three-dimensional
   matrices and the interpolated transform function resolved with its
   box, in `style::transform`; tested against the drafts and the
   reference browser's values. M22.1.
3. *Keyframes and timing.* `RuleSet`'s keyframes by name, with layers,
   media and order; the processing of keyframes; `style::animation`'s
   timing model, phases, progress, directions, fill modes and the before
   flag, tested against Web Animations' examples. M22.2. Built with
   step 4 as one, the keyframes having nothing to use them without the
   cascade's animations.
4. *CSS animations.* The two origins in `compute_style`'s passes;
   keyframes computed at each sample; implicit keyframes;
   pseudo-elements; the taint; matching `animation-name`'s changes;
   `display: none`; the side effects; the `Recascade`; the document
   timeline on `frame_at`, `tick` and `frame`, start times, the restyle
   of animated targets each frame, `needs_frame` and `next_frame_at`,
   and the shell's wait; printing. M22.2, M22.3.
5. *CSS transitions.* Before-change and after-change styles, Transitions
   1 §3's rules, reversing, `transition-behavior`, `display`'s rule and
   inertness, elements no longer rendered, and the whole cascade as a
   restyle of every element. M22.1.
6. *Events.* The tables of both drafts, the queue,
   `Engine::animation_events` and `Engine::animations`. M22.2.
7. *Composited changes.* The restyle's sorting of changes, the layout
   kept, the fragments in the layout and its cache given their new
   effects, the scrollable overflow measured again, frames whose changes
   reach nothing layout reads, and the turned card of M19. M22.4.
8. *Reduced motion.* The engine's scrolling instant under `reduce`.
   M22.5.
9. *The WPT subset*: the rule's checks, the three directories pulled,
   the expectations by what the steps left.
10. *The golden cases*: the change and the time a page may ask for, in
    `capture.sh` and `tests/reference.rs`, and the pages from 100, and
    whatever they find.
11. *Acceptance*: the spinner and the colour fade, their frames' times
    on both backends, the test of the work their frames do, and their
    pages. M22.6.
12. *Performance and fuzzing*: the benchmark, T4.3, the `animations`
    target, the `mutation` target's styles and comparison, and the fuzz
    runs.
13. *The documents*: this note corrected to the code, the architecture's
    M22 entry with its security notes, the roadmap's ticks (M22.1 to
    M22.6) and the items section 2 names registered, and the numbers in
    `docs/perf-budgets.md`.

The sabotage passes left three mutants no test can see, kept as guards
whose loss changes nothing observable: Transitions 1 §3's test, in the
rule that starts a transition, that the two values differ (step 5); the
composited change's test of `display: none`, since a change of it is
a change of layout of the element or of an ancestor (step 7); and the
restyle's asking for a composited change only where what layout reads
changed, since equal styles give an empty list (step 12). Step 7
found two faults of M19's raster, a group's opacity and its layer
missing from the state a partial redraw compares, mended in a commit of
their own.
