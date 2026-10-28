# Design note: DOM scripting

Roadmap M25. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ended: where they
disagreed, the code won, and the places are marked *(corrected)*. What
ships is described in `docs/architecture.md` "Current state".

A script can do almost nothing to its page. M24 gave it ECMAScript, a
`console`, and a `document` with a `title` and nothing else: it cannot
find an element, make one, read or change an attribute, or write into
the parser's input. The document it would reach is ready for it: since
M13 it is an arena of nodes named by generational ids, changed only
through `dom::Document`'s methods, every change journaled and restyled
and laid out incrementally by the next frame; since M24 a run lends the
document to the realm's host and takes it back, its changes processed
as any other change. This milestone gives scripts the DOM: node objects
for the arena's nodes, the tree read and written through them, the
selectors engine behind `querySelector`, the HTML parser behind
`innerHTML` and `document.write`, a `style` object, `classList`, and a
`window` with `location`, `navigator`, `screen` and
`getComputedStyle`; and it frees the nodes a script lets go of.

The standards are read at the versions current when this note was
written:

- the DOM Standard (Living Standard of 5 October 2026): nodes, the
  mutation algorithms, `ParentNode` and `ChildNode`, collections,
  `DOMTokenList`, `Element`'s attributes, the selectors' entry points;
- the HTML Standard (Living Standard of 9 October 2026): sections
  3.1 (`Document`), 3.2.2 (elements in the DOM), 2.6.1 (reflecting
  content attributes), 4.12.1.1 (preparing a script, now for scripts
  inserted by scripts), 8.4 (dynamic markup insertion: `document.write`
  and the insertion point), 8.5 (`innerHTML`, `outerHTML`,
  `insertAdjacentHTML`), 13.2.6.4.8 (a script end tag when scripts
  nest), 13.3 (serializing HTML fragments), 13.4 (parsing HTML
  fragments), 7.2.4 (`Location`) and 8.10.1 (`Navigator`);
- CSSOM (Editor's Draft of 31 August 2026): `CSSStyleDeclaration`,
  `getComputedStyle`, serializing CSS values; and CSSOM View for
  `screen`, `innerWidth` and `devicePixelRatio`;
- Web IDL for the shapes the objects take: interface objects,
  prototypes, `[LegacyUnenumerableNamedProperties]` aside.

The texts are kept in `silver-handover/specs` beside the repository
(`dom.txt`, `cssom-1.txt`, `html-system-state.txt`,
`html-nav-history-apis.txt`, and the HTML sections M24 kept).

What the standards leave to the implementation, or state but are worth
checking, was put to the reference browser (Chrome 154.0.8037.99,
headless) before this note was written:

- an inline script written by `document.write` runs before the call
  returns, and its own writes land just after its end tag, before the
  rest of what the outer write wrote; an external script written runs
  once it has arrived, after the writing script, the parser pausing at
  it, and later writes in the same script queue behind it, unparsed
  until it has run;
- `document.write` from an `async` external script does nothing;
- a script made by `createElement`, given text and appended, runs
  before `appendChild` returns; one parsed by `innerHTML` never runs;
- `childNodes` and `getElementsByTagName` are live, `querySelectorAll`
  is not; `textContent` of the document is `null`;
- the errors are `DOMException`s with messages of the form
  `Failed to execute 'appendChild' on 'Node': The new child element
  contains the parent.`; `new Node()` throws `TypeError: Failed to
  construct 'Node': Illegal constructor`;
- `el.style.color = "RED"` reads back `red` and writes `color: red;`
  into the attribute; `style.margin = "1px 2px"` makes `marginTop`
  read `1px`; an invalid value (`width = "banana"`) is dropped;
  `#ABC` reads back `rgb(170, 187, 204)`;
- `getComputedStyle` gives `rgb(10, 20, 30)`, `24px` for `1.5em` at
  16px, `383px` for a width of `50%` (the used value), `0.25`,
  `Arial, sans-serif`, `400`, `normal`, `rgba(0, 0, 0, 0)`, `auto`
  for `z-index` and `top`, `none` for `transform`, and `inline` for a
  `noscript` while scripts run;
- `innerHTML` escapes `&`, `<`, `>` and no-break spaces in text, `&`,
  `"` and no-break spaces in attribute values, writes void elements
  without an end tag, and leaves the text of `style` and `script` as
  it is; `cloneNode(true)` copies attributes and descendants;
- an element made by `createElement("DIV")` is an `HTMLDivElement`,
  `[object HTMLDivElement]` to `Object.prototype.toString`;
- `navigator`: `appName` `Netscape`, `product` `Gecko`, `webdriver`
  `false`; `screen.colorDepth` and `pixelDepth` 24.

## 1. Decisions

**Node objects made in JavaScript, over numbers (M25.1).** A script's
`Node` is an object of the realm's whose class the realm defines, in a
bootstrap script of `js`'s own run when the realm is made, before any
page's. Each node object holds its node's *handle*, a number, in a
private field; its methods call native functions that take handles,
strings and numbers and give back the same, through the host. The
alternative, classes defined in Rust through `rquickjs`'s class
machinery, needs `unsafe impl JsLifetime` for every class, which `js`'s
`#![forbid(unsafe_code)]` rules out, and would put every method's
argument conversion in Rust; the JavaScript classes keep `js` free of
`unsafe`, keep Web IDL's shapes (interface objects, prototype chains,
accessors on prototypes, `Symbol.toStringTag`) where they are easiest
to get right, and keep the boundary as M24 drew it: plain data, no
object of the engine's reachable from the realm.

- *The natives* are functions of one object that the bootstrap's
  closure receives and that no page can reach: it is not on the global
  object, and the bootstrap keeps what it uses of the built-ins
  (`Map.prototype.get`, `WeakRef.prototype.deref`,
  `Array.prototype.push` and the rest) from before any page runs, so a
  page that replaces a built-in changes nothing the bindings do.
- *A handle* is a node's id as one number: its slot's index plus its
  generation times 2^32. A number holds integers to 2^53, so `dom`
  retires a slot after 2^21 generations instead of 2^32
  (`MAX_GENERATION`): a slot reused two million times is let go of,
  a few dozen bytes, rather than a handle ever naming the wrong node.
- *Identity.* One node, one object: the bootstrap keeps a map from
  handle to a `WeakRef` of the node's object, and gives the same object
  back while it lives (`a.firstChild === a.firstChild`). A
  `FinalizationRegistry` hears when one is collected; if no newer
  object for the node has been made since, the entry goes and the host
  is told the handle is *released*. *(corrected)* The object of a node
  in the document is held strongly as well (a *pin*), as the document
  holds its nodes: held weakly, a temporary `document.body`'s object,
  and with it what a script put on it (expandos, listeners, its
  `style`), died between two reads. The host notes which held nodes
  came into or left the document at each insertion or removal by a
  script, and the bootstrap takes the notes after every native that
  moves nodes. The host marks a node held when the bootstrap first
  makes its object (`Call::Wrap`, which also gives the node's kind and
  name, kept in the object since they never change), not at every
  handle it gives: a handle in a live collection's cache is no hold.
  A finalizer runs as a job, in the checkpoint, where the host is.
- *Interfaces.* `EventTarget`, `Node`, `CharacterData`, `Text`,
  `Document`, `Element`, `HTMLElement` and HTML's element interfaces
  (`HTMLDivElement`, `HTMLAnchorElement` and the rest of HTML's list,
  `HTMLUnknownElement` for a tag HTML does not define, `HTMLElement`
  for one with a hyphen), `NodeList`, `HTMLCollection`, `DOMTokenList`,
  `DOMStringMap`, `CSSStyleDeclaration`, `Window`, `Location`,
  `Navigator`, `Screen`, `PluginArray` and `MimeTypeArray`, each an
  interface object on the global object whose constructor throws
  "Illegal constructor" (but `EventTarget` and `Text`, which the DOM
  lets scripts construct). The global object's prototype becomes
  `Window.prototype`. Each method checks that `this` is one of its own
  (a private field's brand check) and throws `TypeError: Illegal
  invocation` otherwise.
- *Errors* are `DOMException`s (QuickJS-ng has the class) with the
  DOM's names, `HierarchyRequestError`, `NotFoundError`,
  `InvalidCharacterError`, `SyntaxError`, `IndexSizeError`,
  `NotSupportedError`, and the reference browser's messages, which the
  bootstrap writes from the method's name and interface and the reason
  the host gives.
- *Strings* cross as UTF-8. A lone surrogate, which a JavaScript string
  can hold and a Rust string cannot, crosses as U+FFFD; the DOM's
  strings are UTF-16 and keep it (a deviation no page should notice).
  *(corrected)* The bootstrap makes every string well formed
  (`String.prototype.toWellFormed`, kept before any page runs) before
  it crosses: `rquickjs` refuses a lone surrogate, which QuickJS gives
  as WTF-8. The realm's own `DOMException` class is the one thrown
  (`Exception::throw_dom`), whatever a page did to the global's.
  Offsets the DOM gives in UTF-16 code units (`length`, `substringData`
  and the other `CharacterData` methods) are worked out in the
  bootstrap from the string, never by the host.

**The host grows (M24.4's boundary).** `ScriptingHost` gains a method
for each thing a native does: read a node's kind, tag, parent,
children, siblings, text and attributes; create, insert, remove and
clone; parse and serialize markup; match selectors; write into the
parser; read the URL, navigate, and compute styles. Arguments and
results are handles, strings, numbers and small enums of `js`'s own;
a failure is a `DomError` naming the DOM exception and the reason.
*(corrected)* One method, `ScriptingHost::dom`, takes a `Call` and gives
an `Answer` (a default refusing every call, so M24's hosts compile),
rather than a method per call: the engine's host matches every call,
and `js`'s tests answer the ones they make from a small tree of their
own (`test_tree.rs`). Three more, `take_script`, `script_ran` and
`report`, run nested scripts (below). The page and the parser are lent
from step 5 and the window from step 7, as they were needed; the
document's title setter moved into the host, which frees the children
it replaces. The
engine's host, `DocumentHost`, keeps living in a document's `Scripts`
between runs (it holds what must last: the released and started sets
below), and for each run is *lent* what a run may touch, moved in and
moved back as the document is now: the document, the page (its URL,
base URL, encoding, Content Security Policy, the style sheets it has),
the parser while the document is still parsing, and what the window
reports (the viewport, the scripting setting, the fonts the cascade
measures with). Nothing it does reaches the network during a run: a
script it decides to fetch, a navigation, the style sheets and images
a change brings, are recorded and acted on by the engine after the run,
which no script can tell from doing them at once.

**Native work is charged (M24.5).** M24's step budget counts
QuickJS-ng's operations, and a native function's time is not among
them: `querySelectorAll("*")` on a document of four million nodes is
one operation. So the host counts its work, a unit for every node it
visits and every 64 bytes it copies, and the realm adds a step for
every 10,000 units (`WORK_PER_STEP`) after each native call. A loop of
expensive calls runs out of steps as a loop of cheap operations does,
and is stopped the same way. *(corrected)* The charge alone cannot stop
a short loop, which may end before the interrupt is checked again: once
the budget is spent the natives refuse (an internal error), and a run
that spent its budget ends as a budget error even when the script
caught a refusal and ended first.

**The DOM's data is bounded.** `MAX_NODES` bounds how many nodes a
script can keep, but not how much they hold: a script can append a
megabyte string a million times while its heap holds it once. `dom`
counts the bytes of text and of attribute names and values its live
nodes hold and refuses a change past `MAX_DATA_BYTES`, 256 MiB, what a
realm's heap may hold (`MutationError::TooMuchData`); the native throws
a `RangeError`, and the parser drops what does not fit, as it drops a
node past `MAX_NODES`. No document parsed from a body within net's
limit (64 MiB) comes near it.

**Freeing detached nodes (M24 left it to M25).** A node a script
removes, or makes and never inserts, stays in the arena while anything
can still reach it, and only a node object can. After every run and
its checkpoint the host looks at the subtrees scripts detached: a root
still detached none of whose nodes has a live node object (a handle
given out and not released) is freed, its slots reused. The parser may
hold an element a script took out of the tree among its open elements;
once freed, the parser's insertions into it fail and are dropped, as
they would be invisible anyway. The engine's own tables keyed by
`NodeId` (styles, scroll offsets, animations, sheet owners) already
drop or ignore ids that no longer resolve. *(corrected)* An object that
caches its element's `classList`, `style` or `dataset` is in a cycle
with it (each holds its element, which can outlive every other hold);
a removed node in such a cycle is freed when the realm's garbage is
next collected, not at the end of the run.

**DOM Core (M25.1, M25.2).** On `Node`: `nodeType`, `nodeName`,
`ownerDocument`, `parentNode`, `parentElement`, `childNodes`,
`firstChild`, `lastChild`, `previousSibling`, `nextSibling`,
`hasChildNodes`, `isConnected`, `contains`, `nodeValue`,
`textContent`, `appendChild`, `insertBefore`, `removeChild`,
`replaceChild`, `cloneNode` and the constants. `ParentNode` and
`ChildNode`: `children`, `firstElementChild`, `lastElementChild`,
`childElementCount`, `previousElementSibling`,
`nextElementSibling`, `append`, `prepend`, `replaceChildren`, `before`,
`after`, `replaceWith`, `remove`, `querySelector`, `querySelectorAll`.
On `Element`: `tagName`, `localName`, `id`, `className`, `classList`,
`getAttribute`, `setAttribute`, `removeAttribute`, `hasAttribute`,
`hasAttributes`, `toggleAttribute`, `getAttributeNames`, `matches`,
`closest`, `getElementsByTagName`, `getElementsByClassName`,
`innerHTML`, `outerHTML`, `insertAdjacentHTML`,
`insertAdjacentElement`, `insertAdjacentText`. On `CharacterData` and
`Text`: `data`, `length`, `appendData`, `insertData`, `deleteData`,
`replaceData`, `substringData`. On `Document`: `documentElement`,
`head`, `body`, `title` (M24's), `URL`, `documentURI`, `characterSet`,
`contentType`, `compatMode`, `readyState`, `currentScript`,
`defaultView`, `location`, `getElementById`, `createElement`,
`createTextNode`, `write` and `writeln`.

- The mutations are the DOM's: "pre-insert" with its hierarchy checks
  (no node into itself or its descendant, no text under the document,
  no second element under it) before anything changes, a node inserted
  taken from where it was first, as `dom`'s methods do now.
  *(corrected)* Pre-insert's one-element check counts any element
  child, the node itself among them
  (`document.appendChild(document.documentElement)` throws, as in the
  reference browser); replace's leaves out the child replaced. The
  DOM's "convert nodes into a node" makes no fragment for one node, so
  `document.append(x)` failing leaves `x` where it was; with several,
  each is taken out first and stays out if the insertion fails.
- `createElement` checks the DOM's "valid element local name" and
  lowercases in an HTML document; `setAttribute` checks a valid
  attribute name and lowercases it.
- Live collections (`childNodes`, `children`, `getElementsByTagName`,
  `getElementsByClassName`) are `Proxy`s over the collection object
  whose items are fetched again when the document has changed since:
  `dom` gains a version, a count of its changes, and the collection
  keeps the one its items are from. `querySelectorAll` gives a static
  `NodeList`. *(corrected)* A static list holds its objects;
  `childNodes` and `children` are made once a node and kept in the
  node's object; `getElementsByTagName` and `getElementsByClassName`
  make a new collection each call (the reference browser gives the same
  one; the DOM does not ask). An indexed set on a collection in strict
  code throws QuickJS's `TypeError` for a refused set, not the
  reference browser's message.
- `getElementById` walks the tree, as the cascade's id lookups do; an
  index waits until a profile asks for it.

**Selectors (M25.1).** `querySelector`, `querySelectorAll`, `matches`
and `closest` parse their argument with `css`'s tokenizer and `style`'s
selector parser and match with its `Matcher`, so they know exactly what
style rules know (M4, M17, M20): a selector the cascade cannot parse is
a `SyntaxError` here too. A pseudo-element in the list matches nothing.
The matcher gains `:scope`, the element the method was called on (the
root element in a style rule), which `el.querySelectorAll(":scope >
p")` needs. The dynamic pseudo-classes match nothing, as in style,
until M26 gives them state. *(corrected)* For a query of the document
`:scope` is the root element, as in the reference browser
(`document.querySelector(':scope')` is the `html` element). `:has()`
is not parsed (G12): a selector with it is a `SyntaxError` here, where
the reference browser matches it. An empty selector is "The provided
selector is empty.".

**Markup (M25.1).** `innerHTML` and `outerHTML` read as HTML section
13.3 serializes: escaping, void elements, raw text elements left as
they are, `noscript` raw while scripts run; the serializer goes in
`html`. Set, they parse as section 13.4 does: a fragment parser, which
`html` gains, whose tree builder starts in the body with the context
element deciding the tokenizer's first state (`title` and `textarea`
RCDATA, `style`, `xmp`, `iframe`, `noembed` and `noframes` RAWTEXT,
`noscript` RAWTEXT with scripting, `script` script data, `plaintext`
PLAINTEXT) and builds into a detached container in the document itself,
whose children then take the place of the element's (or of the element,
for `outerHTML`). Its scripts are marked already started and never
run. The fragment parser has the tree builder's subset (G44): no
`tbody` is made for a `<td>` in a fragment, which the reference browser
makes. `insertAdjacentHTML` parses the same way, its context the
element or its parent. *(corrected)* The fragment's tokenizer starts
with no start tag emitted, so no end tag is appropriate: `a</textarea>b`
set into a `textarea` is all text. An `html` context starts the builder
before the head, so `html.innerHTML = '<p>x'` makes a head and a body;
every other context starts in the body. A `body` is the context of
`insertAdjacentHTML` at the document element's edges (HTML 8.5.6).
`outerHTML` set on the document's element is a
`NoModificationAllowedError`. The serializer escapes `<` and `>` in
attribute values as well (the Standard of 9 October 2026, and the
reference browser).

**Scripts that scripts insert (HTML 4.12.1.1, left by M24).** A
`script` element prepared becomes *already started* (the host keeps
the set, by id); one made by a script is prepared when it becomes
connected, when a connected one with no `src` gains a child, and when
a connected one gains a `src`: as HTML says, an inline one runs at
once, before the call that connected it returns, and an external one
is fetched and runs as soon as it arrives, in order with the others
whose `async` was set to `false` and as each arrives for the rest.
Running at once from inside a native function is a *nested* run: the
native returns, and the bootstrap asks the host for the scripts to run
now and runs each with `eval` in the realm, inside the call, its errors
reported as the page's, with no checkpoint (HTML's checkpoint waits for
the stack to empty, which it is not). `document.currentScript` is the
element whose script is running, restored when a nested one ends;
`null` for a `javascript:` URL. *(corrected)* Only a script made by
`createElement` or `cloneNode` runs when connected (the host's *made*
set): HTML's "not parser-inserted" also covers a parser's empty script
whose preparation returned early, which here never runs when it is
given text later. A made script not yet started is prepared after each
change a script makes that could connect it or give it text, in tree
order; one whose type is not a script's and changes runs at the next
such change, where HTML waits for it to be inserted again. The natives
after which the bootstrap asks for scripts to run are those that may
make one ready: the insertions, `textContent`, `setAttribute`, the
markup setters and `write`. A nested script stopped by the budget
rethrows its uncatchable error, so the script it is nested in stops
too.

**`document.write` and the insertion point (M25.1, HTML 8.4.3).** The
parser's input gains the insertion point HTML defines: a position in
the tokenizer's input, kept in a stack, since scripts nest. The engine
sets it, just after the end tag, while a parser-inserted script runs
at its end tag or as the pending parsing-blocking script, and unsets it
after. `document.write(text)` with the insertion point defined puts the
text before it (moving it past the text, so the next write follows)
and, when no parsing-blocking script is pending, has the parser run
over what was written, stopping at the insertion point: the tokenizer
treats the insertion point as the end of its input for that parse. A
script end tag in what was written is handled as section 13.2.6.4.8
says when the nesting level is above zero: an inline script runs there
and then, nested, with the insertion point moved to just after its own
end tag for its writes; an external one becomes the pending
parsing-blocking script, and the parse stops, the rest of what was
written waiting in the input until it has run. `writeln` adds a line
feed. Writes are bounded as everything else: the text counts against
the heap it came from and the work charged; nesting deeper than 32
scripts (`MAX_SCRIPT_NESTING`) runs no further nested script and says
so on the console, where the reference browser would run out of stack.
*(corrected)* The cap is 6, not 32: a level of nesting takes some 30
KiB of the 256 KiB stack, and 8 levels overflowed it. It is checked
where a nested script is queued, so a refused one's text is consumed,
not parsed again a level lower. The insertion points are positions in
the tokenizer's input; a write splices its text in before the innermost
and moves the points at or after it. A written inline script is never
held by a style sheet: a script runs only when none blocks scripts, and
the sheets a write brings are found after the run.

- With the insertion point undefined, a write from an external script
  (`async` or `defer`, or one inserted by a script) does nothing, as
  HTML's ignore-destructive-writes counter has it; from anything else
  HTML would open the document again, discarding it, which M25 does not
  do (G48): the write does nothing, and the console says why.
- A `meta` written that declares an encoding is not acted on during the
  write; the parse acts on it, if at all, when it next runs, which
  restarts the document as M24.8 does.

**`readyState`.** `loading` while the parser runs, `interactive` once
it has stopped, `complete` once the document has loaded
(`LoadState::Loaded`). Its `readystatechange` event waits for events,
M26. *(corrected)* The engine gives it to the host at each run.

**`classList`, `style`, `dataset` and reflection (M25.2).**

- `classList` is a `DOMTokenList` over `class`: the DOM's ordered set
  parser and serializer (ASCII white space, duplicates dropped),
  `add`, `remove`, `toggle`, `replace`, `contains`, `item`, `length`,
  `value`, iteration, `supports` throwing `TypeError` as for an
  attribute with no supported tokens.
- `style` is a `CSSStyleDeclaration` over the `style` attribute: its
  declarations parsed with `css`'s parser, each value checked with
  `style::supports` (an invalid one dropped, as the reference browser
  drops `width: banana`), `cssText`, `length`, `item`,
  `getPropertyValue`, `getPropertyPriority`, `setProperty`,
  `removeProperty`, and a property for each name `style` knows, in
  camel case (`backgroundColor`, `cssFloat`, `webkitTransform`) as
  well as hyphenated, through a `Proxy`; a change writes the attribute
  again, which the cascade reads as since M5. *A deviation:* values
  keep the text they were given, trimmed and with keywords' case kept,
  rather than CSSOM's serialization of the specified value (`RED`
  stays `RED`, `#ABC` stays `#ABC`), and a shorthand is not expanded
  (`margin` set leaves `marginTop` empty) (G50). *(corrected)* The
  object reads the attribute each time, splitting the block's text
  (strings, comments and brackets kept whole, comments dropped first,
  as CSS's tokenizer drops them) and taking an alias's canonical name
  (`style::property_name`, new).
- `dataset` is a `DOMStringMap` over the `data-` attributes, through a
  `Proxy`, with the DOM's name conversions.
- Reflected attributes (HTML 2.6.1) for what pages read most: `id`,
  `className`, `title`, `lang`, `dir`, `hidden` on every element;
  `href`, `src` and `action` as URLs resolved against the document's
  base URL; `name`, `type`, `rel`, `alt`, `target`, `media`, `charset`;
  `async` and `defer` on `script`, and `text`; `width` and `height`
  on images as numbers of their attributes (not the drawn size, G46).
  What a form control's IDL state is (`value`, `checked`) waits for
  M26.7, which gives controls state. *(corrected)* The bootstrap's
  `REFLECTED` list holds them, string, URL, boolean, unsigned, signed
  and `dir`; a form's `action` with none is the document's URL.
- `addEventListener` and `removeEventListener` on every `EventTarget`
  keep their listeners, in order and without duplicates, as the DOM
  says, and nothing calls them yet: events are M26's. A script that
  registers a `load` listener runs to its end rather than failing on
  a missing method. `dispatchEvent` and the `on` handler properties
  are M26.2's.

**`window` v1 (M25.4).**

- `location` (and `document.location`): `href`, `origin`, `protocol`,
  `host`, `hostname`, `port`, `pathname`, `search`, `hash`,
  `toString`, from the document's URL as net parses it; `assign`,
  `replace`, `reload`, and setting `href`, `hash` (or `location`
  itself) navigate. A navigation from a script is queued and the engine
  starts it after the run, through the rules a followed link has (M15:
  `http:`, `https:` and `about:blank` from anywhere, `file:` from
  `file:`, `javascript:` run in the document): with HTML's handling,
  *replace* for `replace` and while the document has not completely
  loaded, *push* otherwise; one to the same document's fragment
  scrolls to it. A navigation from a document still on its way (its
  scripts run before it is shown) takes the place of the navigation
  that brought it, with that navigation's handling, so that a page that
  redirects by script leaves one history entry. *(corrected)* As built;
  the last navigation a run asks for wins, and a followed link and a
  script's navigation share one check (`refusal`). A document on its
  way that asks for a `javascript:` URL or a reload does nothing.
- `navigator`, by the D12 decision below: `userAgent` (the header's
  string), `appCodeName` `Mozilla`, `appName` `Netscape`, `appVersion`
  as HTML derives it, `platform` (`Win32`, `MacIntel` or `Linux
  x86_64`, the values every browser freezes to), `product` `Gecko`,
  `productSub`, `vendor` and `vendorSub` as HTML gives them for the
  Gecko compatibility mode (`20100101`, empty, empty), `language` and
  `languages` (`en-US` and `en-US, en`, what `Accept-Language` sends),
  `onLine` `true`, `cookieEnabled` `false` (no cookies until M27),
  `webdriver` `false`, `pdfViewerEnabled` `false` with empty `plugins`
  and `mimeTypes` (HTML's values without a PDF viewer), `javaEnabled()`
  `false`. Nothing else: no `hardwareConcurrency`, `deviceMemory`,
  `connection` or `userAgentData`.
- `screen`: `width`, `height`, `availWidth` and `availHeight` the
  viewport's size in CSS pixels, `colorDepth` and `pixelDepth` 24. The
  screen's real size identifies a machine and helps no page; the window
  is what the page is shown in. `innerWidth` and `innerHeight` are the
  viewport's, `devicePixelRatio` the engine's scale (1 today).
- `getComputedStyle(element, pseudo)`: a read-only
  `CSSStyleDeclaration` whose values are read when asked, the element's
  computed style serialized as CSSOM says for the properties M25
  serializes, about seventy of the commonly read ones (colors as
  `rgb()` and `rgba()`, lengths in `px` at their shortest, keywords,
  numbers, font families as written, `auto` and `none` where computed
  values say so), and custom properties through `getPropertyValue`;
  `""` for the others, and for an element not in the document.
  Serializing computed values goes in `style` (`style::serialize`).
  The style is cascaded, at the first call in a run, from the
  document as it stands and the page's sheets, and restyled at later
  calls from the journal since (M13.1), on a copy of the host's own.
  *Deviations*, each G46: a sheet the running script added or changed
  is not seen until the run ends; transitions and animations neither
  start nor show (the copy is not the engine's style, so a
  `getComputedStyle` between two style changes does not start the
  transition it would in the reference browser); `width`, `height`
  and the other properties whose resolved value is the used value read
  as computed (`auto`, or a percentage as written) rather than the
  laid-out size (`383px`). *(corrected)* The copy is restyled by
  the journal's changes since it was made (`Document::changes`, new).
  The properties are the v1 `ComputedStyle`'s fields, with
  `transform-box` (step 10); one the cascade does not keep
  (`font-weight`, `line-height`: G22) is absent. Two things found on
  the way: the sheets the parser made before a script's end tag were
  found only after the script was prepared, in the same slice of the
  parse, and are found before it now, so that one still loading holds
  the script and the script's `getComputedStyle` reads them; and a
  script writing back a colour it read failed, the cascade reading no
  `rgb()`, which it reads now, `rgba()` too, in both syntaxes (part of
  G16). The engine knew the viewport's size only once it drew a frame,
  so a page loaded before its first gave its scripts a 0 by 0 window:
  `Engine::set_viewport` gives it before (the shell, when its window
  is resized, and the reference test); `--print-to-pdf` still loads at
  0 by 0 (G55).
- `noscript`'s computed `display` reads `inline` while scripts run, as
  the reference browser's does: the UA sheet's `display: none` for it
  goes, and the box tree builder makes no box for a `noscript` when the
  styles were cascaded with scripting on (M24 left this for M25).

**D12, the user agent string and the fingerprint policy.** The
roadmap's decision is taken as written: an honest, minimal user agent
string, `Mozilla/5.0 (compatible; SilverBrowser/<version>)` (since
M14), the same in `navigator.userAgent`, and a minimal API surface,
each value above either fixed or the viewport's, nothing that tells
one machine from another beyond what the window's size already does.
The compatibility escape hatch exists and is not the default: the
shell's `--compat-user-agent` makes requests and `navigator` say what
the reference browser on the same system says (its user agent string,
with `SilverBrowser/<version>` appended, and HTML's Chrome
compatibility mode: `vendor` `Google Inc.`, `productSub` `20030107`),
for a page that refuses an engine it does not recognize. Net takes the
string from the request rather than a constant.

**Mutations reach the frame (M25.3).** Nothing new is needed to make a
script's change visible: it goes through `dom::Document`'s methods,
the journal records it, and after the run the engine takes the
changes as it takes any (M24), the next frame restyling what selector
invalidation says a change reaches (M13.1), laying out again what the
restyle and the change reach (M13.2), and compositing a change to
`transform` or `opacity` alone (M22.4). What M25.3 adds is the
measurement: `benches/dom.rs` builds a table of 1,000 rows from a
script, toggles a class on it, and edits one cell's text, each followed
by a frame; the frame after a change must stay within the 16.7 ms
budget on the reference machine, and what the profile shows is fixed
in this milestone if it does not. *(corrected)* That budget was too
much to ask of a change that restyles and lays out every cell: the
reference browser itself takes 33 ms over style and layout alone for
the toggle at 1,000 rows (49-60 ms for the first layout of the table,
2 ms for the edit). The benchmark builds tables of 100 and 1,000 rows
through the page's links (`javascript:` URLs, as a click runs them).
The profile's first finding was text shaping: every word shaped again
at every pass, the 1,000-row table's layout 1,135 ms. A cache of
widths in `text::FontCollection`, by size and text, 65,536 kept (half
of M35.8), brought it to 68 ms. At 100 rows the edit's frame is 4 ms,
within the budget; the toggle's 20 ms and the build's 22 ms, past it,
a full raster in software taking 10 ms of each. At 1,000 rows the
edit's is 46-58 ms: the box tree built again (17 ms) and every fragment
copied (12 ms) are M35.9's, and the table's cells laid out again with
it (32 ms) M35.9's too now; the toggle's 105-116 ms is a restyle of
7-8 us an element (M35.11) beside those. Every realm now compiles the
bootstrap, 3,300 lines, which takes 5 ms where a realm took 0.12
(M35.12). The incremental engine does what it
should: no whole cascade, the toggle restyling the table's elements and
the edit none, three boxes laid out and the rest taken from the
cache (`docs/perf-budgets.md`).

**Limits, all named.** `WORK_PER_STEP` 10,000 work units per step, in
`js`; `MAX_DATA_BYTES` 256 MiB and `MAX_GENERATION` 2^21 in `dom`;
`MAX_SCRIPT_NESTING` 32 in the engine. The heap, steps and stack are
M24's. *(corrected)* `MAX_SCRIPT_NESTING` is 6; and `text` keeps
`MAX_MEASURES` widths, 65,536.

**Tests.** `js` tests against a host that records its calls: the
bootstrap's shapes (prototype chains, `toString` tags, illegal
constructors and invocations), identity and release, the work charge,
nested runs. `dom` tests for the data count and the generation cap;
`html` tests for the serializer, the fragment parser, the insertion
point and nested writes; `style` tests for `:scope` and for each value
serialized. Engine tests through pages with scripts: every method in
section 1 against what the reference browser gave, the order of
written and inserted scripts, freeing, the D12 values, the
navigations. A new fuzz target, `dom_api` (T2.4): structured input,
a sequence of DOM calls drawn from section 1's surface with handles
chosen from those made so far, turned into a script and run in a page
with scripts on, then a frame drawn; the document must stay valid
(`Document::validate`), and nothing may panic or exceed its limits.
*(corrected)* Calls run as the page is parsed meet no invalidation:
before the first frame nothing is invalidated, and the first frame
cascades whole. So up to 15 calls run in the parse, and the rest in
runs of 8 after a frame each, through `engine::differential::run_script`
(new), which runs a script in the document shown as a `javascript:`
URL's runs; beside `validate`, the frame is compared with a fresh
engine's before every fourth run and at the end, as the `mutation`
target does. The calls are written out as scripts, each in its own
`try`, their strings escaped a code unit at a time, lone surrogates
among them. The web-platform tests' rule takes a layout query read as
a statement of its own (a style flush in the reference browser; its
value unused here), and holds what a script names for its writes to
the rule as markup is (`data-*` names, `style` properties).

## 2. Not in this milestone, said rather than hidden

Each item that no milestone holds is registered on the G list at the
documents step; the number follows each.

- *Events*: dispatch, listeners called, `on` handler attributes and
  properties, `load`, `DOMContentLoaded`, `readystatechange`; timers;
  `requestAnimationFrame`. M26.
- *A style and layout flush for scripts*: `getComputedStyle` that
  starts transitions and shows animations and the sheets a script just
  added, the resolved values of `width`, `height` and the rest, and the
  layout queries of CSSOM View (`offsetWidth`, `clientWidth`,
  `getBoundingClientRect`, `scrollTop`, `scrollTo`, `innerText`). G46.
- *More kinds of node*: `Comment`, `DocumentFragment`, `DocumentType`,
  `ProcessingInstruction`, `CDATASection`, `Attr` and `attributes`,
  with `createComment`, `createDocumentFragment`, `template`'s content;
  the parser's comments and doctypes (G44) would make them. G47.
- *`document.open` and `document.close`*, and a write with no insertion
  point replacing the document; a `javascript:` URL's string result
  replacing it is M26.6's. G48.
- *`MutationObserver`*, which M13 expected of M24 or M25. G49. *(Later:
  done in gap batch A.)*
- *CSSOM's serialization of specified values and shorthands* in
  `style` and `cssText`; `document.styleSheets` and the CSSOM rule
  objects. G50.
- *Namespaces*: `createElementNS`, SVG and MathML elements as their own
  interfaces. M32 for SVG; the rest G51.
- *Traversal and ranges*: `TreeWalker`, `NodeIterator`, `Range`,
  `Selection`, `compareDocumentPosition`, `isEqualNode`, `normalize`.
  G52.
- *Form controls' IDL state* (`value`, `checked`, `selectedIndex`).
  M26.7.
- *`fetch`, `XMLHttpRequest`, cookies, storage*. M27, M28.
- *Element ids indexed* for `getElementById`. M35.
- *Custom elements and shadow trees.* G53.
- *(corrected)* *The window's named properties* (HTML 7.2.2.3): an
  element by its id as a global, which most of the selector
  invalidation reftests use. G54.
- *(corrected)* *A viewport for `--print-to-pdf`'s load*: its scripts
  read a 0 by 0 window. G55.

## 3. Order of work

Each step is a vertical slice: build, tests, clippy green and a smoke
run before the next, with its sabotage pass.

1. *Handles and the binding.* `dom`'s generation cap, data count and
   version; `js`'s bootstrap, the natives, handles, identity and
   release, the interfaces' skeleton, the work charge; the engine's
   host kept per document and lent the page and the parser; freeing
   detached nodes after a run. M25.1.
2. *The tree read and written.* The `Node`, `ParentNode`, `ChildNode`,
   `Element`, `CharacterData` and `Document` members of section 1 but
   selectors and markup; attributes, `id` and `className`; the
   exceptions; the live collections. M25.1, M25.2.
3. *Selectors.* `getElementById`, `querySelector(All)`, `matches`,
   `closest`, `getElementsByTagName` and `ClassName`; `:scope`. M25.1.
4. *Markup.* The serializer and the fragment parser in `html`;
   `innerHTML`, `outerHTML`, `insertAdjacentHTML`. M25.1.
5. *Inserted scripts and `document.write`.* Already started, preparing
   on connection, nested runs, `currentScript`, `readyState`; the
   insertion point, `write` and `writeln`, written scripts inline and
   external, ignore-destructive-writes. M25.1.
6. *`classList`, `style`, `dataset`, reflection, listeners.* M25.2.
7. *`window` v1.* `location` and navigation from scripts, `navigator`
   and D12 with `--compat-user-agent`, `screen` and the viewport,
   `getComputedStyle` with `style::serialize`, `noscript`'s display.
   M25.4.
8. *Invalidation measured.* `benches/dom.rs`, the frame after each
   kind of change, and what the profile asks for. M25.3.
9. *API fuzzing.* The `dom_api` target, seeded. T2.4.
10. *The web-platform tests.* The subset's rule taking tests whose
    scripts use M25's surface, each failure in `expectations.txt` with
    its reason.
11. *Golden cases.* Pages from 119 with scripts on: a table built by a
    script, `innerHTML` and `document.write` output, a class toggled,
    captured from the reference browser.
12. *Acceptance.* The page that builds a table and styles it by a
    class toggle, in the shell; the DOM mutation benchmark within the
    frame budget; T4.3; fuzz runs of `dom_api`, `script`, `html_parse`
    and the targets that lay pages out. M25.5.
13. *The documents.* This note corrected to the code; the
    architecture's entry; the roadmap's items ticked, D12 recorded,
    T2.4 ticked, G46-G53 registered; `docs/perf-budgets.md`'s
    numbers.
