# Design note: navigation, session history and the address bar

Roadmap M15. Written before the code, as the record of the decisions;
what ships is described in `docs/architecture.md` "Current state" when
the code lands. Where this note and the code later disagree, the code
wins and this note is corrected, as the notes before it were.

The problem: the browser loads one page and stops there. `Engine::load`
fetches a URL and shows the document when it and its render-blocking
stylesheets have arrived; the shell calls it once, for its command-line
argument. A click on a link prints the link's `href` and goes nowhere.
There is no history, no way to type an address, and the window's title
is always "Silver Browser". A load that fails is reported on standard
error and the built-in page shows in its place; a response that is not
HTML fails the same way. This milestone makes the browser something a
person can browse with: an address field and back, forward and reload
buttons, links that are followed, a session history that goes back and
forward and keeps the documents it left, fragments that scroll, pages
that say what went wrong, and the page's title on the window. Its
acceptance is a real site over HTTPS, read in the browser.

The standards are read at the versions current when this note was
written: the HTML Standard of 2 October 2026, for its sections on
navigation and session history, on loading documents, on following
hyperlinks, on the `icon` link type and on `document.title`; and CSSOM
View for scrolling an element into view. URLs, fetching and MIME types
are as M14 built them (`docs/design/net.md`). Where a standard leaves a
choice to the user agent, the choice is stated below.

## 1. Decisions

**Where the code goes (D9).** Three parts, in three places:

- *Navigation and session history* go in the engine, a new module
  `engine::navigation` beside the loader. They are web-platform
  semantics: what HTML says a navigation does to the documents of a
  navigable. Scripts will drive them too (`location` is M25.4's), and
  they must not depend on how the browser's buttons are drawn.
- *The toolbar* — the address field, the buttons and the status bubble
  (below) — goes in a new crate, `ui`. It draws with the engine's own
  parts: a `paint::DisplayList` rasterized by a
  `render::SoftwareBackend` and text shaped by `text`, which is what
  D9's "the engine is the toolkit" asks; there is no other toolkit.
  `ui` depends on `paint`, `render` and `text`, and on nothing above
  them: not on `engine`, whose state the shell hands it, and not on
  `winit`, so that it never touches the OS (rule 2) and its every
  behaviour can be tested without a window. M37 builds tabs, downloads
  and settings on it.
- *The shell* stays the glue: the window, the event loop, the keys and
  the pointer, the clipboard, and putting the toolbar's pixels above
  the page's.

The toolbar could instead be a document the engine renders, as some
browsers draw theirs in HTML. It is not, because such a document would
need an editable text field, keyboard focus and a caret, which the
engine does not have (focus comes with M26.3, and editable form
controls have no milestone yet) and would have to borrow from outside
the document; the field's editing is cleaner as a small model of its
own. Nor is the toolbar a module of the shell: the shell is the
one crate that may touch the OS, and the UI code that M37 multiplies
should be kept from it by the dependency graph, not by care.

**The session history.** The engine keeps one, for the one navigable
it has. It is a list of entries and the index of the current one. An
entry holds:

- its URL, fragment included;
- its document state: which document it shows, shared by the entries a
  fragment navigation made from one another, as HTML's document state
  is shared, so that they show the same document;
- its scroll position: the viewport's scroll offset, saved when the
  entry stops being current (HTML's "save persisted state").

A document lives in its document state while the entry is current or
while it is kept in the back-forward cache (below); otherwise the state
holds none, and showing the entry again fetches its URL. The list holds
at most 50 entries, as Chromium's and Firefox's do; past that the
oldest goes. A new engine starts as HTML's new navigable does, with one
entry for the initial `about:blank`. A document given to the engine as
source or as a tree (`Engine::load_document`, `set_document`, which the
tests and fuzz targets use) becomes the only entry of a new history,
with no URL; a load under way is aborted, as now.

**The document's state as one value.** Everything the engine made from
the document shown moves into one value: the document, its page (the
stylesheets and their fetches, the URL, the encoding, the address
space), the stylesheets prepared for the cascade, the cached cascade
and the changes pending for it, the cached layout and the layout cache,
the viewport's and the scroll containers' offsets, and the icon. The
engine has one such value shown. The back-forward cache holds others
whole, so a document shown again needs no cascade and no layout, and
its scroll containers are where they were.

**Navigating.** A navigation starts in one of two ways:

- *by the user* (`Engine::navigate`): the address field, the command
  line. HTML calls this user involvement "browser UI", with no source
  document;
- *by the page* (`Engine::follow_link`): a link activated in the
  document shown, which is its source document, with user involvement
  "activation".

Then, as HTML's navigate algorithm has it:

1. *History handling* is push, except that it is replace when the URL
   equals the shown document's URL (for a page-initiated navigation,
   HTML also asks that the source be same origin with that document,
   which a link in the document shown always is), or when the document
   shown is the initial `about:blank` ("the navigation must be a
   replace").
2. If the URL differs from the current entry's URL only in its
   fragment, and has a fragment (not null; an empty one counts), the
   navigation is to a fragment: no fetch (below), and a navigation
   under way goes on, as HTML's does. One choice of this browser's:
   when the page shown is an error page, the navigation fetches
   instead, since an error page has nothing a fragment could name and
   the user who types the URL again wants it tried again.
3. Otherwise any navigation under way is aborted, and the URL is
   fetched as a document, with mode `navigate`, no initiator and so no
   address-space or mixed-content check, which apply to subresources;
   a top-level navigation the user sees is not one (Local Network
   Access leaves it alone too). The page shown stays until the new
   document commits, as M14 built it: when the document and its
   render-blocking stylesheets have arrived or failed.
4. On commit, a push removes the entries after the current one, and
   their documents with them, then appends the new entry; a replace
   puts the new document in the current entry, with the response's
   URL. The document left is kept in the back-forward cache or dropped.

A navigation that ends without a document leaves the history and the
page shown as they were (below: 204, 205 and downloads).

Which URLs the page may navigate to is the browser's choice; HTML
leaves it to the user agent, and the browsers agree on these:

- `http:` and `https:` from any page;
- `about:blank` from any page;
- `file:` only from a `file:` page, since a web page must not open, or
  learn of, files on the computer;
- `data:` from no page: the browsers refuse a page's top-level
  navigation to a `data:` URL, the tool of phishing pages that show
  any content under an address that names no site;
- `about:silver` only from a page the browser made;
- `javascript:` URLs run script, which there is none of yet (M24); the
  link does nothing;
- any other scheme (`mailto:`, `tel:`) is handed to another program in
  the browsers, which this version cannot do.

A refused navigation does nothing, and the status bubble says why. The
user may type any of the five schemes the browser fetches.

**What a response becomes.** A response with a network error becomes an
error page (below). Otherwise, in HTML's order:

- a `Content-Disposition` field whose disposition type is `attachment`
  (RFC 6266, matched without regard to case) makes the response a
  download. Downloads come with M37.2; until then the navigation ends
  there, the page shown stays, and the status bubble says the file was
  not saved;
- a 204 or 205 ends the navigation with nothing shown, as HTML says;
- otherwise the document is made by the response's computed MIME type
  (MIME Sniffing §7, as the loader computes it now), as HTML's "load a
  document" has it:
  - *HTML*: parsed as now.
  - *`application/xhtml+xml`*, and *any other XML type*: HTML loads
    these as XML documents. There is no XML parser, nor one on the
    roadmap, so XHTML goes through the HTML parser, which reads what
    the web's XHTML is written as, and other XML types are shown as
    text, below. Both are stated deviations.
  - *Text*: `text/plain`, `text/css`, `text/vtt`, the JavaScript types
    and the JSON types become a text document, as HTML's "load a text
    document" makes it: an HTML document whose body is one `<pre>`
    holding the text, decoded as HTML decodes a document's bytes (its
    encoding sniffing, prescan included, since HTML feeds the text to
    its parser). HTML lets the user agent add to the document's head;
    one rule is added there, `pre { white-space: pre-wrap }`, so a
    long line wraps and can be read.
  - *Image, audio and video types*: the browsers show these in the
    navigable as media documents; this one cannot yet (images at M23).
    HTML lets the user agent show "an error message because the
    specified type is not supported", and that is what it shows: a
    page the browser makes, saying what the type is.
  - *Anything else*, `application/pdf` among it since there is no PDF
    viewer: HTML hands it to external software, a download; until
    M37.2, as for `attachment`, the page shown stays and the status
    bubble says so.

A response whose status is not ok (outside 200–299) is still a
document, shown as the server sent it, as M14 has it and as browsers do;
a 404 page is the site's own. One with an empty body has nothing to
show, and gets a page the browser makes, naming the status: "HTTP 404
Not Found", as Chromium does. Firefox shows the empty document; an
empty window tells the user nothing.

**Pages the browser makes.** HTML calls them documents "for inline
content that doesn't have a DOM": the browser's own, under the URL
navigated to. Here they are HTML documents the engine builds as trees,
element by element, with an inline stylesheet and no URL of their own,
so that nothing in them resolves against the URL shown and they fetch
nothing. Their origin is opaque. Text that comes from outside — the
URL, the reason for an error, a MIME type — goes into them as text
nodes; there is no markup for it to be spliced into. `about:silver`
alone is parsed from the engine's own source, under its own URL, so
that its links to its own fragments resolve.

- *`about:silver`* is the built-in page, the one shown since M1: the
  page with no argument on the command line, and HTML's other
  `about:` URLs are the user agent's to define. Its text is brought up
  to date, and it gains a link to one of its own fragments, so that it
  shows a fragment navigation. M37.4 makes it a gallery.
- *`about:blank`* is fetched as now: an empty HTML document.
- *Any other `about:` URL* gets an error page: not a page the browser
  has.
- *Error pages.* A network error names what failed: the name did not
  resolve, no connection could be made, TLS failed (with the reason:
  expired, not yet valid, an issuer not trusted, another name), the
  server broke HTTP, the connection ended early, too long without a
  byte, too many redirects, a redirect where it may not go, a port or
  a scheme not fetched, a file not found, a `data:` URL that is not
  one, a body too large. There is no way past a TLS error: the page
  says what is wrong and offers nothing to proceed with. A load lost
  to a fetch thread that failed gets a page too. Each is shown under
  the URL that failed, as a history entry like any other, so reload
  tries the URL again and back leaves it. HTML makes these documents
  unsalvageable, so the back-forward cache never keeps one; going back
  to one fetches its URL again.
- The *type* and *status* pages above.

Each has a `<title>` saying what it is: "Can't reach example.invalid".

**Going back and forward, reloading, stopping.**

- `Engine::go_back`, `go_forward` and `traverse(delta)` move the
  current entry by a delta, as HTML's "traverse the history by a delta"
  does; a delta past either end does nothing. Any navigation under way
  is aborted first. The current entry's scroll position is saved. If
  the target entry shares the current one's document state, the
  traversal is within the document: its URL becomes the document's,
  and its scroll position is restored. Otherwise its document is
  shown from the back-forward cache, or, when there it has none, its
  URL is fetched as a navigation would fetch it and the entry gets the
  new document, which every entry sharing its document state then
  shows. A fetch that redirects updates the entry's URL to the
  response's. If the fetch fails, the entry shows an error page.
- `Engine::reload` fetches the current entry's URL again, as HTML's
  reload does, and shows the new document in the same entry, at the
  saved scroll position. There is no cache yet to bypass (M28.4), so
  there is one kind of reload. A reload of an error page tries the URL
  again; a reload of `about:silver` builds it again.
- `Engine::stop` is HTML's "stop loading": the navigation under way is
  aborted, and so are the shown document's own fetches. Stylesheets
  aborted that way count as failed, and the load is over. A document
  whose fetches were aborted is unsalvageable, as HTML says.

**The back-forward cache.** HTML keeps a document in its entry's
document state when it is salvageable; there is no script yet, so the
rules that remain are HTML's own: a document is kept when leaving it
aborts no fetch (aborting one makes it unsalvageable), and it is not an
error page. Nor is any other page the browser made kept: a page for a
status or a type is better asked for again, since the server may answer
differently, and `about:silver` costs nothing to build. So a document
still loading a stylesheet when it is left is dropped, and its entry
fetches its URL again when shown. The cache holds at most six
documents besides the one shown, as Chromium's default does; past that
the one shown least recently goes. A document
shown from the cache comes back whole — its cascade, its layout, its
scroll offsets — and the next frame only paints and rasterizes, unless
the viewport's width has changed since. There is no memory limit beyond
the count yet; M35 measures one.

**Scrolling: fragments and saved positions.** Showing a document can
bring a scroll goal with it:

- a URL with a fragment, after a navigation to it: HTML's "scroll to
  the fragment". The indicated part is found as HTML's "select the
  indicated part" has it: an empty fragment is the top of the
  document; otherwise the first element in tree order whose ID equals
  the fragment, else the first `a` whose `name` does, else the same
  two after the fragment is percent-decoded and decoded as UTF-8, else,
  if the decoded fragment is `top` in any case, the top. An element is
  scrolled into view as CSSOM View has it with `block: start`: each
  scroll container around it is scrolled so that its border box's top
  is at the container's top, innermost first, and then the viewport.
  The scroll is instant: "auto" behaviour, and `scroll-behavior` is
  not supported. A fragment that identifies nothing changes nothing.
- an entry's saved scroll position, after a traversal or a reload
  ("restore persisted state").

A goal needs a layout, so it is applied at the next frame, before the
frame is painted, so that no frame shows the document at the top and
then jumps. It is applied again at each frame until the load is over,
since a stylesheet that arrives late can move the target, and is
dropped as soon as the user scrolls: HTML's "try to scroll to the
fragment" stops when "the user is no longer interested", and restoring
a position may be skipped once "the user has scrolled". A navigation
to a fragment of the document shown applies its goal at once, as a new
entry pushed (or replaced) with the same document state; HTML fires
`hashchange` and `popstate` there, which have no listeners before
scripts. The entry restores only the viewport's offset when its
document was fetched again: the scroll containers' offsets live with
the document, and come back only with it.

**Links.** `Engine::link_at(x, y)` answers with the link under a point:
the nearest `a` element with an `href` at or above the element hit, and
its URL, parsed from the `href` against the document's base URL with
the document's encoding, as HTML's "follow the hyperlink" does when the
link is followed. That base URL is the frozen base URL of the first
`<base href>` in the document, else the document's URL; unlike a
stylesheet's, which is fetched when the parser inserts its `<link>`, a
link's URL is parsed when it is followed, after the whole document is
there. An `href` that does not parse gives no URL and is not followed.
An `area` element is a link too, but has no box until images give it
one (M23).

The shell follows a link on a click, as HTML's activation behaviour
waits for: the left button pressed on a link and released on the same
one. `Engine::follow_link` takes the link and navigates, unless the
link's document is no longer shown or the element is gone from it
(HTML's "cannot navigate"). `target` is not looked at: there is one
navigable, and `_blank` or a name navigates it (tabs are M37.1). The
`download` attribute is not looked at either: without downloads the
link is followed. `rel=noreferrer` changes nothing while no referrer
is sent (M33.3).

**The title (M15.5).** `Engine::title` is `document.title`: the text of
the first `title` element in tree order, its child text, with ASCII
whitespace stripped and collapsed. The window's title is the page's
title and the browser's name, "Example Domain — Silver Browser"; when
the title is empty, the entry's URL takes its place, and with no URL
the window says "Silver Browser". The title can be anything a page
writes, so before it goes to the window it loses its control
characters and the bidirectional formatting characters (U+061C,
U+200E, U+200F, U+202A–U+202E, U+2066–U+2069), which would otherwise
let a page reorder the text the browser puts after it, and what is
left, trimmed, is cut at 1,024 characters, more than any title bar
shows.

**The icon (M15.5).** HTML's `icon` link type, fetched and kept, not yet
shown (images are M23):

- the last `<link>` in tree order whose `rel` has the token `icon`,
  whose `type`, if present, is an image type, and whose `href` parses,
  as HTML says "the last one declared in tree order"; in its absence,
  for an `http:` or `https:` document, `/favicon.ico` resolved against
  the document's URL, as HTML allows;
- fetched once the document has committed, with destination `image`
  (net gains it, with Fetch's `Accept` for images,
  `image/png,image/svg+xml,image/*;q=0.8,*/*;q=0.5`), mode `no-cors`,
  and the document as initiator, so a public page's icon cannot come
  from a local address;
- kept when its status is ok, its computed type is an image type, and
  it is no larger than 1 MiB; `Engine::icon` gives its URL, type and
  bytes;
- not waited for by the load, as HTML's `load` event does not wait for
  it. The links are read once, at commit; a later change to them
  fetches nothing.

**What the engine tells the shell.** Beside what it has:

- `navigate(url)`, `follow_link(&link)`, `go_back`, `go_forward`,
  `traverse(delta)`, `reload`, `stop`, replacing `load`;
- `can_go_back`, `can_go_forward`;
- `url()`, now the current entry's URL, which an error page has too;
- `load_state()`: loading while a navigation is under way or the shown
  document waits for a stylesheet; failed, with the error, while an
  error page is shown; loaded otherwise;
- `title()`, `icon()`;
- `fonts()`, the fonts it draws with, which the toolbar draws with too;
- `link_at(x, y)`, which now answers with a `Link`: the element and its
  URL;
- a notice of what a navigation did that the user should be told and
  that shows no page: a navigation refused, a download not saved. The
  shell takes it with `take_notice`, and the status bubble shows it.

`show_built_in_page` goes: the shell navigates to `about:silver`.

**The address (M15.4).** What the user types becomes a URL in
`engine::address`. The input loses leading and trailing whitespace, and
any line breaks within it (a URL pasted from a wrapped line). Then,
first rule that applies:

1. Empty: nothing happens.
2. *A host and port* — something that looks like a host, a colon and
   digits, then the end or a `/`, `?` or `#`, as `localhost:8080` or
   `example.com:8443/a` — is a host, though it would parse as a URL
   whose scheme is the host. It goes on to rule 5.
3. *An absolute path* on the computer the browser runs on (`/home/a`,
   `C:\a`, `\\server\share`) is that file's `file:` URL
   (`Url::from_file_path`); whether the file exists is the fetch's to
   find out.
4. *A URL* — it parses as an absolute URL — is that URL. Its scheme
   decides the rest: one of the five the browser fetches is navigated
   to, any other is refused with a notice, as a link to it is.
5. *Something that looks like a host*, before an optional port, path,
   query and fragment, with no whitespace in it and no `@` (an email
   address is not one), gets a scheme put before it:
   - `http://` for `localhost`, a name ending in `.localhost`, an IPv4
     address in dotted-decimal form, an IPv6 address in brackets, and a
     name of one label given with a port (`devbox:3000`): addresses on
     the computer or on the local network, which rarely have
     certificates;
   - `https://` for a domain of two labels or more, each 1 to 63
     letters, digits or hyphens (or non-ASCII characters, for a name
     IDNA maps), not starting or ending with a hyphen, the last not
     all digits; a final dot is allowed. There is no public suffix
     list until cookies need one (M27), so `notes.txt` looks like a
     host as much as `example.com` does, and gets an error page when it
     does not resolve.
6. Anything else is not an address. The field keeps the text and the
   status bubble says so. Searching for it waits for the choice of
   search engine (M37.3).

There is no falling back to `http:` when `https:` fails: the error page
says what failed, and the user can type `http://` to mean it, as in the
browsers' HTTPS-only modes. The command line goes through the same
rules after its own first: an argument naming an existing file is that
file.

**The toolbar (M15.3).** A strip 40 pixels high across the top of the
window, above the page, which is laid out to the rest of the window.
From the left: back, forward, reload — which is stop while a load is
under way — and the address field, which takes the remaining width.
The buttons are drawn as shapes (`paint`'s anti-aliased polygons:
arrows, a circular arrow, a cross), not as glyphs a font may lack. A
button the history cannot use is dimmed and does nothing. A button
lights under the pointer, and acts when the left button is pressed and
released on it, as a native button does. There is one light theme; a
dark one waits for settings (M37.3). The toolbar is one display list,
rasterized into its own buffer of the strip's size, and drawn again
only when something in it changed.

**The address field.** `ui` holds its model: the text, the caret, and
the selection's other end. It is the field's alone, tested without
drawing.

- What it shows: when it is not being edited, the current entry's URL,
  serialized (below). While the user edits, what they typed, which a
  page's navigation does not overwrite: a commit replaces the text
  only when the field is not focused, or is focused but unedited. After
  Enter, it shows the URL the navigation was started with, until that
  navigation commits or ends; a page-initiated navigation shows nothing
  until it commits.
- Editing: insert at the caret, replacing the selection; Backspace and
  Delete; Left and Right, collapsing a selection or moving a
  character; Home and End; Ctrl+Left and Ctrl+Right by words, where a
  word is a run of letters and digits, so `/`, `.`, `:` and `-` stop
  the caret as they do in the browsers' address bars; Ctrl+Backspace
  and Ctrl+Delete by words; Shift with any of the moves extends the
  selection; Ctrl+A selects all. A click places the caret, Shift with a
  click selects to it, a drag selects, a double click selects a word
  and a triple click all of it; presses half a second and four pixels
  apart at most count as one click after another. A click that focuses
  the field selects all of it, as the browsers' address bars do, unless
  the pointer then moves four pixels or more with the button held,
  which selects from where it was pressed.
- Characters are Unicode scalar values: the caret moves by one, and
  never stands inside one. Grapheme clusters, so that an accented
  letter made of two code points moves as one, come with M42's text
  work, and so does bidirectional text in the field, which shows its
  characters in logical order meanwhile.
- The field holds at most 65,536 bytes. A paste that would pass that is
  refused whole, with a notice, rather than cut into a URL nobody
  wrote. Pasted text loses its line breaks and control characters.
- Drawing: the text at 15 pixels, clipped to the field and scrolled
  sideways to keep the caret in view; a selection's highlight and a
  caret one pixel wide, which does not blink, so the toolbar needs no
  timer, both only while the field has the keyboard, as in the browsers;
  a focus ring. The caret's places come from the text's shaping: `text`
  gains, for a shaped run, the x of every character boundary, from the
  shaper's clusters, a ligature's advance shared among the characters it
  joins. The field shapes its text once per change, and draws only the
  characters in view.

**The status bubble.** A small box at the bottom left of the page, over
it, as the browsers have: while the pointer is over a link, the link's
URL, so where a link goes can be read before it is followed; otherwise
the latest notice, until the next input: a key, a press of a mouse
button or a turn of the wheel, but not the pointer moving, or a notice
would go before it was read. It is at most half the window's width; a
longer text keeps its start, where a URL's scheme and host are, and
ends in an ellipsis. It is part of `ui`, drawn into its own buffer, and
the shell puts it over the page's pixels.

**Showing a URL.** The field shows a URL as the URL Standard serializes
it: an internationalized host as its ASCII (`xn--`) form, and the path
and query percent-encoded as they are. Showing the Unicode form safely
needs the browsers' rules for names that look like others (Unicode's
UTS #39 among them), which nothing on the roadmap builds yet; until
then the serialization shows exactly which host the page came from, at
the cost of readability. The status bubble shows a link's URL the same
way. When a URL is wider than the field and the field is not being
edited, it shows from its start, so the scheme and the host stay in
view.

**Keys and the pointer.** The keyboard goes to the address field when it
has focus, else to the page. On Linux and Windows:

- Ctrl+L, Alt+D and F6 focus the field and select all of it;
- in the field, Enter navigates to what it holds; Escape gives up an
  edit, showing the URL again and selecting all of it, or, with no
  edit, gives the focus back to the page;
- Alt+Left and Alt+Right go back and forward, and so do the mouse's
  back and forward buttons;
- F5 and Ctrl+R reload; Escape, when the page has focus and a load is
  under way, stops it;
- the scrolling keys of M9 scroll the page when it has focus;
- Ctrl+C, Ctrl+X and Ctrl+V copy, cut and paste in the field, and so
  do Ctrl+Insert, Shift+Delete and Shift+Insert;
- a key pressed with Ctrl or Alt alone types nothing into the field;
  with both, which is how Windows reports AltGr, it types.

On macOS, Cmd stands where Ctrl is, Cmd+[ and Cmd+] go back and
forward, and the field moves as macOS fields do: Option+Left and
Option+Right by words, Cmd+Left and Cmd+Right to the ends; Option
types characters, so Alt+D is not macOS's, and Cmd+Backspace, which
deletes to the line's start in macOS fields, does nothing until the
field can do that. The mapping
from keys to commands is the shell's (rule 6 keeps `cfg` there); `ui`
takes commands. A click on the page gives it the focus; one on the
field gives the field the focus. The pointer is a hand over a link, an
I-beam over the field, and an arrow elsewhere.

At start the page has the focus, except when nothing was asked for on
the command line: then `about:silver` shows and the field has the
focus, as a new window's address bar does.

**The clipboard and input methods.** The clipboard is the OS's, so the
shell reaches it, through the `arboard` crate with its default features
off. It adds eight small crates to the tree, under licences the
repository allows; one of them, `objc2-app-kit`, is a second generation
of a crate `winit` already brings on macOS, so the bans list skips it
by name, as it does `objc2`, until `winit` moves to the same
generation. A clipboard that cannot be reached gives a notice and
nothing else. Text an input method commits (`winit`'s `Ime::Commit`)
is inserted like typed text; the shell turns the input method on while
the field has focus and tells it where the caret is. Text still being
composed is not shown in the field until M45; the input method's own
window shows it.

**Tests.** None needs the network:

- *Navigation*, against the test server: a push and a replace; a
  navigation to the shown URL replaces; the initial `about:blank` is
  replaced; 50 entries, the oldest dropped; back and forward from the
  cache with no request made (the server counts them), and with the
  same frame as before; a seventh document left that drops the oldest
  kept, whose entry fetches again; a page left while a stylesheet is
  on its way, not kept; reload, with its scroll position; stop, with
  its stylesheets failed; a navigation started while another is under
  way, and a traversal during one.
- *Responses*: each type to the document it becomes; a text document's
  `<pre>` and its decoding; 204 and 205 leave the page; `attachment`
  and an unknown type leave the page with a notice; an empty 404 page
  and a 404 with a body; an error page for each class of error the
  test server can cause (a name under `.invalid` that does not
  resolve, a refused connection, a TLS certificate from a root not
  trusted, a broken response, a timeout), with its URL and title, a
  reload that refetches it, and text in it that would be markup shown
  as text.
- *Links and fragments*: the URL a link resolves to, with a `<base>`
  before and after it and in a legacy encoding; a click as press and
  release on the same link; each scheme rule; a link whose document is
  gone; a fragment navigation that fetches nothing and pushes an entry;
  the indicated part for an ID, a name, a percent-encoded fragment,
  `top` and the empty fragment; an element inside a scroll container;
  a fragment target moved by a stylesheet that arrives later; a goal
  dropped when the user scrolls; scroll positions restored on back.
- *Title and icon*: whitespace, the first `title`, the characters
  dropped and the cut; the last icon link, the default `/favicon.ico`,
  a refused type, a local icon for a public page refused.
- *The address*: a table of inputs to the URL or the refusal each
  becomes, each rule and its edges among them.
- *`ui`*: the field's editing, command by command, and a seeded random
  run of thousands of commands over text with multi-byte characters,
  after each of which the caret and the selection stand on character
  boundaries within the text; the caret's places against the shaped
  text; the toolbar's buttons hit by the pointer and dimmed; frames
  drawn with the bundled test font, pixel for pixel where the result is
  exact (a button's colours, the caret's column).
- *The shell*: its handling of input is a state machine over its own
  small types, apart from `winit`'s events, so it is driven in tests
  with an engine and the test server: focusing the field, typing an
  address, Enter, the load, the title, back. Only the window, the
  surface and the clipboard are left to the smoke run and to hand.
- *A live test*, `#[ignore]`d: the engine navigates to
  `https://example.com/` and finds its title, "Example Domain", and its
  text, and draws it. It trusts Mozilla's roots, and also
  the certificates of the PEM file `SILVER_EXTRA_ROOTS` names when it
  names one, for a network whose TLS is intercepted; the browser itself
  has no such option. It is run by
  hand. M15.6's acceptance is that test, and the browser itself pointed
  at the site on a machine with a direct connection.

**The layering contract.**

- `ui` joins the graph beside `engine`, under `shell`, above `paint`,
  `render` and `text`.
- Rule 2 gains a sentence: `ui` draws the browser's own interface and
  touches no OS; the shell gives it input and shows its pixels.

## 2. Not in this milestone, said rather than hidden

- *Tabs*, opening a link in a new one, and *named targets*: M37.1.
- *Downloads*: M37.2. A response that would be one leaves the page with
  a notice.
- *Search* from the address field, and *suggestions* as one types:
  M37.3, with the choice of search engine.
- *A history kept between sessions*, bookmarks, and a list of the
  history to choose from: not on the roadmap yet; M28.1's profile is
  where they would be stored.
- *Scripts' part*: `location` (M25.4), the History API, `hashchange`
  and `popstate`, and a click on a link as an event a listener can
  cancel (M26.6, which takes the click this milestone follows from the
  shell and routes it through event dispatch). The `:target`
  pseudo-class is not matched; the engine finds the indicated part but
  keeps no target element.
- *Form submission* and `POST` navigations: with interactive forms,
  which have no milestone yet.
- *Handing a URL to another program* (`mailto:` and the like): not on
  the roadmap yet.
- *An XML parser*: XHTML goes through the HTML parser, other XML is
  shown as text.
- *Images and media as documents*: M23 for images, M40 for audio and
  video; a page says each cannot be shown yet. *PDF*: no viewer is
  planned, so a PDF is a download.
- *Showing the icon*: M23 decodes it.
- *Internationalized hosts in Unicode* in the address field, and the
  checks that make it safe: not on the roadmap yet.
- *An `http:` fallback* for a host typed without a scheme: not planned;
  see above.
- *Grapheme clusters* and *bidirectional text* in the field: M42.
  *Preedit text* of an input method shown in the field: M45.
- *The X11 primary selection* (pasting with the middle button): not on
  the roadmap yet.
- *Scaling for high-density displays*: the page and the toolbar are
  drawn one pixel to one CSS pixel. Not on the roadmap yet.
- *Accessibility* of the toolbar, to a screen reader: with the
  accessibility tree, M44.
- *A blinking caret*: it would need a timer for a cosmetic effect.
- *A dark toolbar*: with settings, M37.3.
- *A memory limit* on the back-forward cache: M35.

## 3. Order of work

Each step is a vertical slice: build, tests, clippy green and a smoke
run before the next. A step is done when its tests pass.

1. *The session history.* The document's state as one value, the
   initial `about:blank`, entries and their document states,
   `navigate`, `traverse`, `reload` and `stop`, history handling, the
   back-forward cache and its limits, the load state; `load` goes.
2. *Responses and the browser's pages.* Dispatch by type, 204 and 205,
   `attachment`, text documents, `about:silver`, the error pages, the
   status and type pages, notices.
3. *Links and fragments.* `link_at` with the URL, `follow_link` and its
   scheme rules, fragment navigations, the indicated part, scrolling
   into view, scroll goals and their restoring.
4. *Title and icon.* `title`, `Destination::Image` in `net`, the icon's
   fetch and `icon`.
5. *The address.* `engine::address` and its table of tests.
6. *`ui`.* `text`'s character boundaries; the field's model; the
   toolbar and the status bubble, drawn.
7. *The shell.* The toolbar above the page, focus, the keys and the
   pointer, the clipboard and input methods, the pointer's shapes, the
   window's title, the start page; the shell's state machine and its
   tests; the bans list.
8. *Acceptance and the documents.* The live test, run here and by hand;
   this note corrected to the code; the architecture's M15 entry and
   the layering contract; the roadmap's ticks; and numbers for
   `docs/perf-budgets.md`: going back to a page of 1,000 paragraphs
   kept in the cache, and to one fetched again, each to its first
   frame.

## 4. What the code taught

Ten things this note did not foresee, each corrected above where it
stands or recorded here. Scrolling an element into view could not use
the paint-order walk as it was, since that walk leaves out what is
outside the viewport, which is where a fragment's target usually is; the
target is found in a walk of its own with a viewport as tall as the
document, and each scroll container around it is scrolled innermost
first, the target found again after each. A navigation to a fragment
lets a push or a replace under way go on, as the note says, but cancels
a reload or a traversal, whose entry would otherwise be shown in the
wrong place. A document given as source has no URL, so only an absolute
`href` in it resolves. A `text/plain` response with a NUL in it sniffs
as binary, as MIME Sniffing says, and becomes a download rather than a
text document; and a text document's `<pre>` is drawn in the
proportional face, since no monospace face is loaded yet. The icon,
which the load does not wait for, is a fetch of the document's own: a
page left before its icon arrives aborts that fetch, and HTML's rule
then keeps the page out of the back-forward cache. `example.com:port`,
which the rule for a host and port does not take, `port` being no
number, parses as a URL whose scheme is `example.com`, and was refused
as a scheme the browser does not open; a scheme with a dot in it is now
taken as no address. The field's caret stands at characters in logical
order, while the text it draws goes through `text`'s shaping, which lays
a right-to-left run out right to left: in a URL with Hebrew or Arabic in
it the caret stands apart from its character until M42. Escape on a page
stops only a load under way, as the note says, and the code had to look
at the load state to do so: `stop` on a page all there would still abort
its icon. And `example.com` no longer has a heading: the page this note
had in mind became a title, a paragraph and a link, and then, within the
day, lost the link, so the live test reads only the title and the
paragraph's words. Last, the load benchmark waited for the engine to
have a URL, which with the initial `about:blank` it always has, so its
wait ended at once and its run failed; benchmarks are built by the
checks but not run, and it was found only when step 8 ran it. It waits
for the load state now.
