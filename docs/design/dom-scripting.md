# Design note: DOM scripting

Roadmap M25. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ends: where they disagree,
the code wins, and the places are marked *(corrected)*. What ships is
described in `docs/architecture.md` "Current state".

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
  is told the handle is *released*.
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
  Offsets the DOM gives in UTF-16 code units (`length`, `substringData`
  and the other `CharacterData` methods) are worked out in the
  bootstrap from the string, never by the host.

**The host grows (M24.4's boundary).** `ScriptingHost` gains a method
for each thing a native does: read a node's kind, tag, parent,
children, siblings, text and attributes; create, insert, remove and
clone; parse and serialize markup; match selectors; write into the
parser; read the URL, navigate, and compute styles. Arguments and
results are handles, strings, numbers and small enums of `js`'s own;
a failure is a `DomError` naming the DOM exception and the reason. The
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
and is stopped the same way.

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
drop or ignore ids that no longer resolve.

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
- `createElement` checks the DOM's "valid element local name" and
  lowercases in an HTML document; `setAttribute` checks a valid
  attribute name and lowercases it.
- Live collections (`childNodes`, `children`, `getElementsByTagName`,
  `getElementsByClassName`) are `Proxy`s over the collection object
  whose items are fetched again when the document has changed since:
  `dom` gains a version, a count of its changes, and the collection
  keeps the one its items are from. `querySelectorAll` gives a static
  `NodeList`.
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
until M26 gives them state.

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
element or its parent.

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
`null` for a `javascript:` URL.

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
M26.

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
  (`margin` set leaves `marginTop` empty) (G50).
- `dataset` is a `DOMStringMap` over the `data-` attributes, through a
  `Proxy`, with the DOM's name conversions.
- Reflected attributes (HTML 2.6.1) for what pages read most: `id`,
  `className`, `title`, `lang`, `dir`, `hidden` on every element;
  `href`, `src` and `action` as URLs resolved against the document's
  base URL; `name`, `type`, `rel`, `alt`, `target`, `media`, `charset`;
  `async` and `defer` on `script`, and `text`; `width` and `height`
  on images as numbers of their attributes (not the drawn size, G46).
  What a form control's IDL state is (`value`, `checked`) waits for
  M26.7, which gives controls state.
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
  redirects by script leaves one history entry.
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
  laid-out size (`383px`).
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
in this milestone if it does not.

**Limits, all named.** `WORK_PER_STEP` 10,000 work units per step, in
`js`; `MAX_DATA_BYTES` 256 MiB and `MAX_GENERATION` 2^21 in `dom`;
`MAX_SCRIPT_NESTING` 32 in the engine. The heap, steps and stack are
M24's.

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
- *`MutationObserver`*, which M13 expected of M24 or M25. G49.
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
