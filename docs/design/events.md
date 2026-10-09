# Design note: the event loop, events and input

Roadmap M26. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ends: where they disagree,
the code wins, and the places are marked *(corrected)*. What ships is
described in `docs/architecture.md` "Current state".

A page can be read but not used. M25 gave scripts the DOM, but nothing
calls a script after its document has been parsed: listeners are kept
and never called, there are no timers, and `requestAnimationFrame`
does not exist. The user's input never reaches the page: the shell
keeps the keyboard for its address field and its scrolling keys,
follows a link itself on a click, and draws a form control as an empty
box that no click or key can change. `:hover` and `:focus` match
nothing. A form cannot be filled in, let alone sent. This milestone
makes pages interactive: HTML's event loop with its tasks, microtasks,
timers and rendering steps; DOM events and their dispatch; the mouse,
the keyboard, focus and the wheel delivered to the page as events with
their default actions; the dynamic pseudo-classes; form controls that
show and edit their values; and form submission. Its acceptance is an
interactive to-do list built by a script, and a search form typed into
and submitted.

The standards are read at the versions current when this note was
written:

- the HTML Standard (Living Standard of 9 October 2026): 8.1.7 (event
  loops: tasks, the microtask checkpoint, updating the rendering),
  8.1.8 (event handlers, their attributes and the processing
  algorithm), 8.6 (timers), 8.10 (animation frames), 6.6 (focus:
  focusable areas, the focusing steps, sequential navigation,
  `autofocus`), 6.5 and 4.10 (the activation behaviours of links and
  controls, labels), 4.10.5 to 4.10.11 (`input`, `button`, `select`,
  `option`, `textarea` and their values and checkedness), 4.10.18 to
  4.10.22 (form owners, constraint validation, form submission, the
  form data set and its encodings), 4.10.5.5 (implicit submission),
  7.4 (a `javascript:` URL's result), 7.2.4 (the document's
  lifecycle: `readystatechange`, `DOMContentLoaded`, `load`,
  `pageshow`, `pagehide`, `unload`) and 6.2 (page visibility);
- the DOM Standard (Living Standard of 5 October 2026): 2 (events:
  `Event`, `CustomEvent`, `EventTarget`, dispatching, the activation
  behaviour hooks, `AbortController` and `AbortSignal`);
- UI Events (W3C Working Draft of 17 June 2026): the event classes,
  their order, `key` and `code`, the legacy `keyCode`, `charCode` and
  `which`;
- Pointer Events Level 3: pointer events for the mouse, fired before
  the mouse events they stand for;
- Input Events Level 2: `beforeinput` and `input`'s `inputType`;
- Selectors Level 4 and CSS Basic User Interface Level 4: the user
  action and input pseudo-classes, `outline` and `cursor`;
- CSSOM View: `scroll` and `resize` in the rendering steps, the mouse
  event coordinates.

The texts the repository's `silver-handover/specs` keeps (`dom.txt`,
`html-webappapis.txt`) are read there; the rest were read online.

What the standards leave to the implementation, or state but are worth
checking, was put to the reference browser (Chrome 154.0.8037.99,
headless) before this note was written (scratchpad `m26/probe1.html`,
`probe2.html`):

- the document's events come in this order: `readystatechange`
  (`interactive`), the deferred scripts, `DOMContentLoaded`,
  `readystatechange` (`complete`), `load` at the window (its target the
  document), `pageshow`; a microtask queued by a script runs before the
  next script;
- timeouts of equal delay run in the order they were set, a 0 before a
  1 set earlier; a microtask queued in a timer runs before the next
  timer; the sixth nested zero timeout and later wait 4ms;
- `dispatchEvent` runs the listeners before it returns, `isTrusted`
  false; handler attributes keep the place in the listener order of
  their first activation (HTML's example gives ONE, TWO, THREE, FOUR);
  a handler that returns `false` cancels the event;
- a checkbox's `checked` flips before the click's listeners run and
  flips back if one cancels it; a checkbox not in the document fires no
  `input` or `change`; a click on a label checks its control;
- a form's entries for a text field, a checked checkbox with no value,
  a checked radio, a select and a textarea are `q=hello&c=on&r=1&
  s=Second+option&ta=text&h=x`;
- a text field is 177 by 21 pixels: 13.3333px Arial, padding 1px 2px,
  a 2px inset border of `rgb(118, 118, 118)`, content box sizing; with
  `size=10` it is 107 wide. A checkbox or radio is a 13px border box
  with a 3px 3px 3px 4px margin (5px at a radio's left); a button
  `rgb(240, 240, 240)` with padding 1px 6px and a 2px outset border; a
  select 19px tall with a 1px solid border; a textarea monospace with
  padding 2px and a 1px solid border; a disabled field greys its text
  to `rgb(84, 84, 84)` and its border and background to 30% alpha.

## 1. Decisions

**The event loop, in the engine (M26.1).** HTML's event loop is the
engine's: one per engine, as there is one navigable. It is not a thread
of its own. The engine already runs on its caller's calls: the shell
calls `poll_network` when a fetch wakes it and draws a frame when one is
wanted. Two more calls make the loop:

- `Engine::run_tasks()` runs the tasks that are due, oldest first,
  each followed by a microtask checkpoint and then by what the engine
  does after any run (the journal taken, a navigation started, a dead
  realm noticed). It stops when none is left or after a slice of 50ms
  of its clock, leaving the rest for the next call so that a page's
  timers cannot keep the window from drawing; it says whether a frame
  is wanted.
- `Engine::next_task_at()` is when the next timer is due, for the
  shell to sleep until.

The shell calls `run_tasks` after every wake and when its sleep ends,
and sleeps until the sooner of `next_frame_at` and `next_task_at`.

- *Tasks* are kept per document, in the document's `Scripts`, so a
  document on its way (an incoming one, parsed before it is shown) runs
  its own and a document the back-forward cache keeps holds its own
  until it is shown again (HTML: a task whose document is not fully
  active is not runnable). The task sources are not kept apart: one
  queue, in the order tasks became runnable, which HTML allows.
- *The clock.* Timers, `performance.now()`, an event's `timeStamp` and
  an animation frame's time are read from one clock, which the engine
  is given (`Engine::set_clock`, a function giving a `Duration`): by
  default the time since the engine was made, and the shell's own, the
  one it gives `frame_at`, so that a frame's time and a timer's are on
  the same line. Tests give a clock they move by hand, so that no test
  waits for real time. A document's *time origin* is the clock's time
  when its navigation started; `performance.now()` is the clock less
  that, coarsened to 100 microseconds, and `performance.timeOrigin`
  the origin in milliseconds since the Unix epoch, as the reference
  browser gives it without cross-origin isolation. QuickJS-ng's own
  `performance`, which M24 took away, stays away; the bootstrap makes
  this one.
- *The microtask checkpoint* is the realm's (`Realm::checkpoint`, the
  promise jobs and `queueMicrotask`'s), performed after every task and
  after every callback the engine makes into the realm when no script
  is running, as HTML's "clean up after running a callback" says: so a
  promise resolved in a click listener settles before the next
  listener of a user's click runs, and after the listener in
  `el.click()`'s.

**Calling into the realm (M24 left it to M26).** The engine never calls
a function a page gave it: the functions stay in the realm, where the
bootstrap keeps them, and the engine names what it wants done in plain
data. `Realm` gains one method, `enter(task, host)`, which runs a
`js::Task`:

- `Task::Dispatch { target, event }`: dispatch an event the engine
  makes (a trusted one) at a node, the document or the window, the
  event's class, type, flags and fields given as data
  (`js::EventInit`). The answer is whether the event was canceled.
- `Task::Timer(id)`: run timer `id`'s callback.
- `Task::AnimationFrames(time)`: run the animation frame callbacks.
- `Task::ReportError { .. }`: an uncaught exception a classic script
  threw, for the `error` event at the window (below).

The bootstrap hands `js` its entry points when it runs, an object that
the realm keeps (a `Persistent`, dropped before the context, as M24's
rejections are). `enter` is `run`'s twin: the realm's host is moved in
and out the same way, the run has a step budget of its own, and a dead
realm runs nothing. A dispatch is a JavaScript generator that yields
after each listener it calls, so that the realm performs a microtask
checkpoint between listeners without the bootstrap knowing anything of
checkpoints; one budget covers the whole dispatch, as it is one task.

**Timers (M26.5).** `setTimeout`, `setInterval`, `clearTimeout` and
`clearInterval` as HTML 8.6 has them, in the bootstrap: a timer's
callback (or its string, compiled as a classic script when it runs,
the page's eval policy aside, as the realm has no other) and its
arguments stay in the realm, in a map by id; the host is told the id,
the delay and whether it repeats (`Call::SetTimer`, `Call::ClearTimer`),
and the engine keeps a list by due time. A timer's delay is clamped to
0 at least, and to 4ms once timers nest more than five deep, the
nesting counted from the task that set them; an interval is set again,
with the same id, after its callback runs. A due timer becomes a task.
Two timers due at once run in the order they were set. The engine
keeps at most `MAX_TIMERS` (100,000) a document; past it `setTimeout`
throws a `RangeError`, where the reference browser has no limit but its
memory. `queueMicrotask` is QuickJS-ng's and stays.

**Animation frames (M26.1).** `requestAnimationFrame` and
`cancelAnimationFrame`, their callbacks in the realm, and a frame wanted
while any is waiting. HTML's "update the rendering" runs inside
`frame_at`, before the style and the layout are brought up to date, in
its order: the `resize` steps (a `resize` event at the window when the
viewport's size changed since the last frame), the `scroll` steps
(`scroll` at the document and at each element that scrolled since),
the animation and transition events M22 collected and nothing
dispatched (`animationstart` and the rest, `transitionrun` and the
rest), then the animation frame callbacks, each given the frame's time
on the document's clock, and those registered by a callback waiting for
the next frame. A callback's changes reach the same frame. `frame` (with
no time) runs no callbacks: it is the tests' and the printer's.

**Events (M26.2).** The DOM's events, in the bootstrap:

- *Classes*: `Event`, `CustomEvent`, `UIEvent`, `FocusEvent`,
  `MouseEvent`, `PointerEvent`, `WheelEvent`, `KeyboardEvent`,
  `InputEvent`, `SubmitEvent`, `ErrorEvent`, `PageTransitionEvent`,
  with their constructors and dictionaries as the specifications give
  them, and `AbortController` and `AbortSignal` (`abort()`, `aborted`,
  `reason`, `throwIfAborted`, an `abort` event), which
  `addEventListener`'s `signal` needs.
- *Listeners* gain `once`, `passive`, `signal` and the removed flag the
  DOM's dispatch reads, and a callback that is an object is called
  through its `handleEvent`. As in the reference browser, a `wheel`
  listener on the window, the document or the body is passive unless
  it says otherwise.
- *Dispatch* is the DOM's algorithm: the path from the target through
  its ancestors to the document and, but for a `load` event, the
  window; the capture phase from the window down, the target, the
  bubbles; `stopPropagation`, `stopImmediatePropagation`,
  `preventDefault` (ignored for a passive listener or an event that is
  not cancelable), the legacy `returnValue` and `cancelBubble`,
  `composedPath`, `eventPhase`, `currentTarget`, `timeStamp` and
  `isTrusted`, true only for the events the engine makes. The path is
  read from the host once, at the dispatch's start. There is no shadow
  DOM, so no retargeting.
- *Activation behaviour.* A `click` dispatched at an element whose
  ancestor (or itself) has an activation behaviour (a link, a submit
  or reset button, a checkbox, a radio, a label, a `summary` aside)
  runs the DOM's legacy-pre-activation behaviour before the dispatch
  (a checkbox flips, a radio is checked) and, after it, the activation
  behaviour unless the event was canceled, or the legacy-canceled one
  that undoes the first if it was. The behaviours are the engine's,
  reached by natives (`Call::PreActivate`, `Call::Activate`,
  `Call::CancelActivation`), so `el.click()` and a user's click do the
  same thing.
- *Errors.* An exception a listener throws, and one a classic script
  or a timer's callback throws, is reported as HTML says: an
  `ErrorEvent` at the window, whose `onerror` handler gets the five
  arguments and cancels by returning `true`, and the console's
  "Uncaught ..." unless it was canceled; a muted error is "Script
  error." with no position.
- *Event handlers (HTML 8.1.8).* The `on...` attributes of
  `GlobalEventHandlers`, `WindowEventHandlers` and
  `DocumentAndElementEventHandlers`, as IDL attributes on the elements,
  the document and the window, and as content attributes: the
  attribute's text is kept uncompiled when it is set and compiled when
  the event first needs it, as a function of `event` with the
  document, the form owner and the element on its scope chain, the
  body's window-reflecting handlers (`onload`, `onerror` and the rest)
  on the window. A body that does not compile is reported and the
  handler stays null. A handler's listener takes its place in the
  listener order at its first activation, as HTML's examples show and
  the reference browser does. An element whose object the bootstrap
  makes for the first time activates the handlers its attributes
  already hold, so that a page's markup handlers run though no script
  ever touched the element.
- `window.alert`, `confirm` and `prompt` exist and block nothing: the
  message goes to the console, `confirm` answers `false` and `prompt`
  `null`, as a browser does when it suppresses dialogs. A dialog the
  user answers is G56.

**The document's lifecycle (M26.1).** As HTML's "the end" and the
reference browser above: when the parse ends, `readyState` becomes
`interactive` with a `readystatechange` event; the deferred scripts
run; a task fires `DOMContentLoaded` at the document; when the load is
complete (`LoadState::Loaded`, which already waits for the scripts, the
style sheets and the images) a task sets `readyState` to `complete`
with its event, fires `load` at the window, its target the document,
and `pageshow` with `persisted` false. A document shown again from the
back-forward cache gets `pageshow` with `persisted` true; one left gets
`pagehide`, `persisted` saying whether the cache keeps it, and, if it
does not, `unload`. `beforeunload`'s prompt is G57. `load` and `error`
fire at a `script` element whose script was fetched or failed, at an
`img` decoded or broken, and at a `link` whose style sheet arrived or
failed. `document.visibilityState` and `hidden` follow the window: the
shell says when it is hidden (minimized or covered, winit's
`Occluded`), the document gets `visibilitychange`, its animation frame
callbacks wait, and its timers are run no more than once a second, as
the reference browser throttles a hidden page's.

**Element state and the dynamic pseudo-classes (M26.4).** State is the
DOM's: `dom` keeps, for each element, the flags the user's actions set
(`hover`, `active`, `focus`, `focus-visible`, `focus-within`,
`target`), and a change to them is journaled (`Change::State`, with
the old flags) as an attribute change is, so that it is restyled the
way M13 restyles everything: style's invalidation records which of the
user action pseudo-classes its selectors use, where, and before which
combinators, and a change marks the element, its subtree or its later
siblings as an attribute change does. The matcher reads the flags:
`:hover`, `:active`, `:focus`, `:focus-visible`, `:focus-within` and
`:target` match; `:visited` matches nothing, as no history of visited
links is kept to answer it (and the reference browser hides it from
pages). The pseudo-classes of form state (below) read the controls'
state the same way. A pseudo-element's user action states are its
originating element's.

- *Hover* is the element under the pointer and its ancestors, from the
  engine's hit test (M9.4), with scripts on or off. It changes when the
  pointer moves, and when the page moves under a still pointer (a
  scroll, a layout), checked at each frame.
- *Active* is the element a primary button went down on and its
  ancestors, until the button comes up; a form control or a link held
  by the keyboard (Space on a button) is active too.
- *Focus*, *focus-visible* and *focus-within* follow the focused
  element (below). `:focus-visible` matches when the focus came from
  the keyboard, when it is a text field however it came, and when a
  script focused an element after the last input was a key, as the
  reference browser decides.
- *Target* is the element the document's URL's fragment indicates.

Two properties come with them, as the dynamic pseudo-classes are of
little use without: `outline` (`outline-style`, `-width`, `-color` and
`-offset`, and the shorthand), painted outside the border box and
taking no room, its `auto` style drawn as a 2px ring of the focus colour
(`-webkit-focus-ring-color`, `#101010`, the UA sheet's
`:focus-visible { outline: auto 1px -webkit-focus-ring-color }`, as the
reference browser's); and `cursor`'s keywords, which the engine reports
for the point under the pointer (`Engine::cursor_at`) and the shell
shows as the OS's cursor, `auto` being a hand over a link and an I-beam
over text that can be edited.

**The pointer (M26.3).** The engine is given the pointer's input in the
page's coordinates (CSS pixels from the viewport's top left):
`Engine::pointer_move(x, y)`, `pointer_down(x, y, button, buttons,
modifiers, count)`, `pointer_up(..)`, `pointer_leave()` and `wheel(x,
y, dx, dy, modifiers)`, `count` being the press's place in a run of
quick presses, which the shell counts as it does now. Each call is a
task: the engine hit-tests the point, updates the hover and active
state, dispatches the events and does their default actions, all
before it returns.

- *The events and their order* are Pointer Events' and UI Events',
  checked with the reference browser: moving onto an element,
  `pointerover`, `pointerenter` (at each newly hovered ancestor, from
  the outermost), `mouseover`, `mouseenter`, with `pointerout`,
  `pointerleave`, `mouseout` and `mouseleave` for those left first;
  then `pointermove` and `mousemove`. A press is `pointerdown` and,
  unless it was canceled, `mousedown`; a release `pointerup` and
  `mouseup`; then `click` (a `PointerEvent`, as in the reference
  browser) at the nearest common ancestor of the press's and the
  release's targets for the primary button, `auxclick` for another,
  `dblclick` after a second click, and `contextmenu` after a secondary
  release. `detail` is the press's count. The fields are the
  specifications': `clientX` and `clientY` the point, `pageX` and
  `pageY` with the document's scroll, `screenX` and `screenY` the
  client point (D12: the screen is the viewport), `offsetX` and
  `offsetY` from the target's padding edge, `button`, `buttons` and the
  modifier keys; the pointer events' `pointerId` 1, `pointerType`
  `mouse`, `isPrimary` true, a pressure of 0.5 while a button is down.
  Pointer capture is G58.
- *Default actions.* A `mousedown` not canceled focuses the nearest
  focusable ancestor of its target, or takes the focus from the element
  that has it; in a text control it places the caret, and a drag
  selects (two presses a word, three all). A `click` not canceled runs
  its activation behaviour: a link is followed. So the shell no longer
  follows links itself; it hands the engine the press and the release,
  and the engine's `follow_link` (M15's checks unchanged) runs as the
  link's activation behaviour, as M26.6 asks, a listener able to cancel
  it. A middle-button click on a link opens nothing: tabs are M37's, and
  the status bubble says so. A `wheel` not canceled scrolls, as M19's
  scroll chaining does now.
- *A `javascript:` URL's result* (HTML 7.4), left by M25: a string it
  ends with becomes the document, as a new document of that HTML at the
  same URL, replacing the entry.
- *Scripts off*, none of the events is dispatched and every default
  action is done: links are followed, controls are focused and edited,
  forms are submitted. A page without scripts loses nothing it had.

**Focus and the keyboard (M26.3).** Focus is HTML 6.6's, for one
document:

- *Focusable areas* are links with an `href`, controls that are not
  disabled (`input` but hidden ones, `button`, `select`, `textarea`),
  and elements with a `tabindex`, when they are rendered; the viewport
  when nothing else has focus. Editing hosts are G59, with
  `contenteditable`.
- *The focusing steps*, `focus()` (with `preventScroll`) and `blur()`,
  `document.activeElement` (the body when the viewport has focus),
  `document.hasFocus()`, and `autofocus`, whose candidate is focused at
  the first rendering after the document loads. The events are `blur`
  and `focusout` at the element that loses focus, `focus` and `focusin`
  at the one that gains it, `relatedTarget` naming the other, in the
  reference browser's order. An element focused is scrolled into view
  unless `preventScroll` asks otherwise. An element removed while it
  has focus gives it to the viewport at once and without events; one
  that becomes disabled or stops being rendered loses it at the next
  rendering, with `blur`, as HTML's fixup has it.
- *Sequential navigation.* Tab and Shift+Tab move the focus through
  the focusable areas in `tabindex` order (positive values first, in
  order, then the rest in tree order; a negative one is skipped). Past
  the last, the page gives the focus up and the shell takes it, to the
  address field; Shift+Tab in the address field gives it back to the
  page's last. That is the reference browser's way too.
- *The window's focus.* When the window loses the OS's focus the
  focused element gets `blur` and the window `blur`; when it comes back,
  `focus`.
- *Keys.* `Engine::key_down(key, modifiers)` and `key_up`, a key being
  its `key` and `code` strings, its `location` and whether it repeats,
  and `Engine::text(string)` for the text a key or an input method
  commits. The events go to the focused element, or the body: `keydown`;
  then, for a key that types, `keypress` (the legacy event, with
  `charCode`); then `keyup` at the release. `keyCode` and `which` take
  the reference browser's values for the keys whose values every
  browser agrees on (letters, digits, the named keys); others are 0.
  The default actions of a `keydown` not canceled: in a text control,
  editing (below); Enter on a link or a button activates it, Space on a
  button or a checkbox does at its `keyup`; the arrows in a `select`
  move its choice; Tab moves the focus; and, when no control takes
  them, the scrolling keys the shell handles now (the arrows, Page Up
  and Down, Space, Home and End) scroll the page, from the engine.
- *The shell* keeps the keys that are the browser's (focusing the
  address field, back and forward, reload, and copying and pasting
  in its own field) and hands the page every other key, its releases
  too (winit's released keys, which it drops now), when the page has
  the focus. Copy, cut and paste go to the page's focused control
  (`Engine::copy`, `cut`, `paste`), the shell keeping the clipboard.
  Composition events and showing an input method's text before it is
  committed are G60, as they are for the address field.

**Form controls (M26.7).** A control's state is the DOM's, as its
attributes are: `dom` keeps for an `input`, a `textarea` or an `option`
a record of HTML's state (the value and its dirty flag, the
checkedness and its dirty flag, an option's selectedness and its dirty
flag, the indeterminate flag, and a text control's selection: start,
end and direction), made at the first change and journaled
(`Change::Control`) so that what shows the state is restyled and laid
out again. It counts against `MAX_DATA_BYTES`; cloning copies it, as
HTML's cloning steps say; a form's reset puts it back to the defaults.

- *Types.* `text`, `search`, `tel`, `url`, `email`, `password` and
  `number` are text fields (a number's value sanitized as HTML says, no
  spin buttons); `checkbox`, `radio`, `submit`, `reset`, `button` and
  `hidden` are what they are; `image` is a submit button showing its
  `alt` (its picture, and the clicked point it sends, are G61); `file`,
  `color`, `range` and the date and time types, which need pickers,
  are text fields until G61 gives them their own (a value sanitized by
  its type all the same).
- *What a control shows.* The box tree gives a control the text it
  shows as an anonymous text box inside it, so that layout, painting
  and hit testing need nothing new: a text field's value, or its
  `placeholder` in the `::placeholder` colour (`#757575`, the reference
  browser's `darkgray` aside) when the value is empty; a password's
  value as bullets; a button's `value`, or "Submit" or "Reset"; a
  select's chosen option's label, with a down arrow drawn at its end; a
  textarea's value (its children only give its default value). A
  single-line field does not wrap; its text is clipped to its content
  box and scrolled so that the caret stays in view. A checkbox's and a
  radio's mark (a tick, a dot, a bar for an indeterminate checkbox) is
  drawn by paint, as list markers' shapes are (M21.6): a new fragment
  kind for it. A `select` with `multiple` or a `size` above 1 is a list
  box: its options as rows, the chosen ones highlighted, a scroll
  container.
- *The UA sheet* takes the reference browser's computed values above:
  13.3333px text, a text field's 2px inset border of `#767676` and its
  1px 2px padding, a button's `#f0f0f0` with an outset border, a
  select's 1px solid border, a textarea's monospace and 2px padding, a
  disabled control's greyed colours. A text field's width is `size`
  (20 by default) times the font's average character width, the
  textarea's `cols` and `rows` likewise, through the presentational
  hints M21.6 began. Controls are drawn as their computed style says;
  the reference browser's native look (a 1px rounded border where the
  computed style says 2px inset) is not copied.
- *The IDL* M25 left to this milestone (HTML 4.10): `value`,
  `defaultValue`, `checked`, `defaultChecked`, `indeterminate`, `type`,
  `form`, `labels`, `select()`, `selectionStart`, `selectionEnd`,
  `selectionDirection` and `setSelectionRange` on `input` and
  `textarea`; `selectedIndex`, `value`, `options` (an
  `HTMLOptionsCollection`), `selectedOptions`, `length`, `add`,
  `remove` and `item` on `select`; `selected`, `defaultSelected`,
  `index`, `text`, `label` and `value` on `option`; `elements` (an
  `HTMLFormControlsCollection`, by index and by name), `length`,
  `submit()`, `requestSubmit()` and `reset()` on `form`; `control` and
  `htmlFor` on `label`; and the constraint validation API below.
- *The form owner* is the `form` attribute's form when the control is
  connected, else the nearest `form` ancestor, as style's matcher works
  it out now. The parser keeps no form element pointer (G44's tree
  builder), so a form whose end tag a table moved does not own the
  controls after it, where the reference browser's parser does.

**Editing (M26.7, M26.3).** The focused text control is edited by the
engine:

- *The model* is the address field's: `ui::field::TextField`'s caret,
  selection and commands, moved to `text` (`text::edit`) so that the
  engine and `ui` share it (the engine does not depend on `ui`), and
  given lines: a textarea keeps its line feeds, and Up and Down move by
  the lines layout made, keeping the caret's x. A field's `maxlength`
  is counted in UTF-16 code units, as HTML counts it.
- *Keys and the pointer.* Typing inserts, Backspace and Delete delete
  (by word with Ctrl, Option on macOS), the arrows, Home and End move
  (Shift extends), Ctrl+A selects all, Enter in a textarea is a line
  feed and in a field submits (below); the pointer places and drags the
  selection. Undo is G62.
- *Events.* Each edit is a `beforeinput` (cancelable) with its
  `inputType` (`insertText`, `insertLineBreak`, `deleteContentBackward`
  and the rest, `insertFromPaste`, `deleteByCut`), then the change,
  then `input`; `change` at the next blur if the value changed since
  the focus came. A checkbox's or a radio's click gives `input` and
  `change` after the click's listeners, a select's choice gives them at
  once. A script setting `value` fires nothing and puts the caret at the
  end.
- *Drawing.* The caret is a 1px bar the colour of the text, and the
  selection a band of the selection colour behind the text, both drawn
  by the engine over the focused control's fragment from the positions
  `text`'s caret stops give (M15's way). The caret does not blink, as
  the address field's does not.
- *The select's list.* A click on a select, or Alt+Down or Space with
  it focused, opens its options as a list drawn by the engine over the
  page below the control (above, when there is no room), hit-tested
  before the page; the arrows move through it, Enter or a click chooses,
  Escape or a click elsewhere closes it. With it closed, the arrows
  change the choice.
- *Checkboxes and radios* are toggled by their click's activation
  behaviour; a radio unchecks the others of its group, the group being
  the radios of the same form owner and name.

**Form submission (M26.8).** HTML 4.10.21, in the engine:

- *Submitting.* A submit button's activation behaviour, Enter in a text
  field (implicit submission: the form's default button is clicked, or
  with none, the form is submitted if it has one field that blocks it),
  `form.requestSubmit(submitter)` and `form.submit()` (which skips the
  validation and the `submit` event). With interactive validation (no
  `novalidate`, no `formnovalidate` on the submitter), an invalid
  control stops it (below); then a `submit` event (a `SubmitEvent`
  naming the submitter) that a listener can cancel.
- *The entry list* is HTML's: the submittable controls of the form in
  tree order, but disabled ones, ones in a disabled fieldset, unchecked
  checkboxes and radios, buttons other than the submitter, and
  controls with no name; a checkbox's or radio's value `on` when it has
  none; a select's chosen options; a textarea's value with its line
  breaks made CR LF; a hidden `_charset_` field given the encoding's
  name. `dirname` is G63.
- *The method and encoding.* The submitter's `formaction`,
  `formmethod`, `formenctype` and `formtarget`, else the form's. A GET
  replaces the action URL's query with the entries
  `application/x-www-form-urlencoded`; a POST sends them as the
  request's body in the form's encoding (`accept-charset`'s first
  label `encoding` knows, else the document's): urlencoded,
  `multipart/form-data` (a boundary of `----SilverFormBoundary` and 16
  random letters and digits, names and file names escaped as HTML
  says) or `text/plain`. The serializers go in `net::url` beside the
  percent encoding sets they need (the urlencoded set, a space as
  `+`), and `net::Request` gains `post(url, body, content_type)` and the
  request header it sends for it.
- *The navigation* is a link's, with the method and body: a POST's
  `Navigation` and the history entry it makes carry its body. Reloading
  a POST's entry, or going back to one the cache did not keep, would
  send the form again, which the reference browser asks the user before
  doing; with no dialog to ask in, the engine shows a page of its own
  saying the page came from a sent form, as the reference browser's
  "Confirm Form Resubmission" page does without its button (G57). A
  `_blank` or other named target navigates this tab, as a link's does
  until M37. A `mailto:` action is refused, as a link's is; a
  `javascript:` one runs.
- *Reset.* A reset button's activation, or `form.reset()`, fires
  `reset` (cancelable) and puts every control back to its defaults.
- *Constraint validation v1.* The validity states `valueMissing`
  (`required`), `typeMismatch` (an `email` or `url` that is not one),
  `patternMismatch`, `tooLong` and `tooShort` (after the user's edit,
  as HTML says), `rangeUnderflow`, `rangeOverflow` and `stepMismatch`
  (a `number`), and `customError` (`setCustomValidity`), with
  `validity`, `willValidate`, `validationMessage`, `checkValidity()`,
  `reportValidity()`, the `invalid` event, and `:valid`, `:invalid`,
  `:user-valid`, `:user-invalid`, `:required`, `:optional`,
  `:in-range` and `:out-of-range`. A `pattern` is an ECMAScript regular
  expression with the `v` flag, so it is matched by QuickJS-ng whether
  the page's scripts run or not: `js` gains `pattern_matches(pattern,
  value)`, which compiles `^(?:pattern)$` in a small context of its
  own, with no DOM and a small step budget, the page's own scripts
  having no part in it; a pattern that does not compile is no
  constraint, as HTML says. Interactive validation fires `invalid` at
  each invalid control and, at the first not canceled, focuses it,
  scrolls it into view and says why in the status bubble, in the
  reference browser's words ("Please fill out this field." and the
  rest). The browser's own bubble over the field is the status bubble's
  text until a better place is found (G64).
- The pseudo-classes of form state the matcher gains: `:checked` and
  `:indeterminate` from the state, `:default`, `:placeholder-shown`,
  `:read-only` and `:read-write`, and the validity ones above; all are
  parsed now, where M25's parser refused them and dropped their rules.

**Limits, all named.** `MAX_TIMERS` 100,000 a document; a task slice of
50ms (`TASK_SLICE`); a hidden document's timers at most once a second
(`HIDDEN_TIMER_PERIOD`); `text::edit`'s 65,536 bytes a field, the
address field's; a select's list showing 20 options at most, scrolled.
Each task has M24's step budget, the heap and the stack as before. The
dispatch path is as deep as the tree, which `dom::MAX_DEPTH` bounds.

**Tests.** `js` tests against its recording host: the entry points,
timers' ids and clamping, a dispatch's phases, order, cancellation and
the checkpoints between listeners, handlers and their compiling,
errors. `dom` tests for the state flags, the control records, their
journal and their data count. `style` tests for each new pseudo-class,
its invalidation, `outline` and `cursor`. Engine tests through pages
with scripts and the engine's clock moved by hand: the task order, the
timers, the animation frames and the rendering steps' events, the
lifecycle events, every input sequence (moves, presses, releases, keys,
the wheel) with and without scripts, focus and sequential navigation,
every control's state, edits and events, the select's list, form
submission against `net`'s test server (the request line, headers and
body of each encoding, GET and POST, the history it makes), and
validation. Session tests drive the shell's window through the same
input. Fuzzing: the `dom_api` target gains events, listeners and timers
among its calls, and a new target, `input`, drives pages of controls
with pointer and key sequences, the engine's clock moved between them;
the document must stay valid, and nothing may panic or exceed its
limits.

## 2. Not in this milestone, said rather than hidden

Each item that no milestone holds is registered on the G list at the
documents step; the number follows each.

- *Dialogs*: `alert`, `confirm` and `prompt` answered by the user. G56.
- *Leaving a page*: `beforeunload`'s prompt, and the prompt before a
  form is sent again. G57.
- *Pointer capture*, `setPointerCapture` and its events; touch and pen
  input; drag and drop. G58.
- *Editing hosts*: `contenteditable`, `designMode`,
  `document.execCommand`, and selecting and copying a page's own text.
  G59.
- *Input methods*: composition events, and showing the text an input
  method composes before it commits it, in the page as in the address
  field. G60.
- *Pickers*: the `file` control (choosing a file, and the files in a
  form's body), `color`, `range`, the date and time types, and the
  image button's picture and clicked point. G61.
- *Undo and redo* in text controls. G62.
- *`dirname`* and the direction of a control's text. G63.
- *A validation message beside its control*, as the reference browser
  draws it. G64.
- *Tabs and windows*: `target=_blank`, a middle click, `window.open`.
  M37.
- *`fetch`, `XMLHttpRequest`, `FormData` and `URLSearchParams`*. M27.
- *Observers*: `MutationObserver` (G49), `IntersectionObserver`,
  `ResizeObserver`, `requestIdleCallback`. G65.
- *Messaging*: `postMessage`, `MessageChannel`, `BroadcastChannel`;
  workers. M31.
- *`:visited`*, by design (above).

## 3. Order of work

Each step is a vertical slice: build, tests, clippy green and a smoke
run before the next, with its sabotage pass.

1. *The loop's tasks and timers.* The engine's clock and
   `set_clock`; tasks per document, `run_tasks` and `next_task_at`;
   `Realm::enter` and the bootstrap's entry points; timers with their
   ids, nesting and clamping, `MAX_TIMERS`; `performance`; the shell
   sleeping until the next task. M26.1, M26.5.
2. *Events.* The event classes, `AbortController`, listeners' options,
   the dispatch with its checkpoints, handlers (IDL and content
   attributes), the `error` event, `alert` and its kin, `el.click()`
   with the activation hooks. M26.2.
3. *The rendering steps and the lifecycle.* Animation frames;
   `resize`, `scroll`, the animation and transition events;
   `readystatechange`, `DOMContentLoaded`, `load`, `pageshow`,
   `pagehide`, `unload`; elements' `load` and `error`; visibility and a
   hidden page's throttling. M26.1.
4. *State and the dynamic pseudo-classes.* `dom`'s state flags and
   their journal; the matcher and invalidation; `outline` and `cursor`;
   hover from the pointer. M26.4.
5. *The pointer.* The engine's pointer calls, the hover and active
   chains, the mouse and pointer events and their order, clicks and
   their counts, activation (links followed by the engine, the shell's
   own following gone), the middle click's notice, a `javascript:`
   URL's result, the cursor. M26.3, M26.6.
6. *Focus and the keyboard.* Focusable areas, the focusing steps and
   their events, sequential navigation and the shell's address field,
   the window's focus, `autofocus`, the key events and their default
   actions, the scrolling keys and the wheel moved into the engine.
   M26.3.
7. *Controls shown.* `dom`'s control records, the IDL, what each
   control shows, the UA sheet, the list box, the marks. M26.7.
8. *Editing.* `text::edit`, the caret and the selection drawn, the keys
   and the pointer, the clipboard, an input method's commits, the input
   events, checkboxes and radios, the select's list. M26.7, M26.3.
9. *Submission and validation.* The entry list, the encodings in `net`,
   POST navigations and their history, implicit submission, reset,
   constraint validation with `js::pattern_matches`. M26.8.
10. *Fuzzing.* `dom_api`'s new calls, the `input` target, seeded.
11. *The web-platform tests.* The runner waits for `reftest-wait` to be
    taken off and runs the page's timers and frames until it is; the
    rule takes tests whose scripts use M26's surface; each failure in
    `expectations.txt` with its reason.
12. *Golden cases.* Pages from 123 with scripts on: the document
    changed by timers, animation frames and the load event, and the
    controls' boxes, captured from the reference browser.
13. *Acceptance.* The to-do list built by a script, used through the
    shell's window: entries added with the keyboard and a button,
    styled by a click, removed; the search form typed into and sent,
    its results page loaded from `net`'s test server, with scripts on
    and off; T4.3; fuzz runs of `input`, `dom_api`, `script` and the
    targets that lay pages out. M26.9.
14. *The documents.* This note corrected to the code; the
    architecture's entry; the roadmap's items ticked, G56 to G65
    registered; `docs/perf-budgets.md`'s numbers.
