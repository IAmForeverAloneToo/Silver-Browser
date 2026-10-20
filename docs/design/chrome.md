# Design note: browser chrome v1

Roadmap M37. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ends: where they
disagree, the code wins, and section 4 lists the places, marked
*(corrected)*. What ships is described in `docs/architecture.md`
"Current state", and the limits it adds live in `docs/security.md`.

The browser is one window holding one tab. A link with
`target=_blank` navigates the tab itself; a middle click on a link says
"Links open in this tab: there are no other tabs yet"; `window.open`
does not exist. A response that would be a download leaves the page
shown with a notice, "Not saved: ... saving files comes in a later
version", and the browser has no code to parse `Content-Disposition`'s
file name or to write a file the user asked for. There are no settings:
scripts, the colour scheme and the profile are command-line flags,
`Accept-Language` is fixed at `en-US,en;q=0.9` (M14), text typed in
the address field that is no address is refused rather than searched
for, and the start page is always `about:silver`. There is one profile
per process, and nothing keeps the tabs a user had open. Underneath,
each `RemoteTab` makes its own cookie jar, HTTP cache, HSTS hosts,
permissions and storage (M34), which is right for one tab and wrong
for two: two tabs of one profile must share them, as must a tab and
the one it opened.

This milestone makes the browser something to browse with daily: tabs
in a strip drawn by the engine's own parts, pages opening tabs,
downloads saved where the user's downloads go, settings kept in the
profile and a page to change them, the browser's own pages and error
pages finished, and profiles in windows of their own, a private window
among them. Its acceptance (M37.6) is the maintainer browsing with it
for half an hour a week and filing what fails; what the code can do for
that is said in section 1, "The acceptance".

The documents this note follows, at the versions current when it was
written (21 October 2026):

- the HTML Standard (WHATWG, Living Standard): browsing contexts and
  their groups, `window.open` and the rules for choosing a navigable,
  `noopener` and `noreferrer`, the `download` attribute and "allowed
  to download", transient user activation, and `sessionStorage`'s copy
  for an auxiliary browsing context;
- RFC 6266 (`Content-Disposition` in HTTP) and RFC 8187 (the `filename*`
  parameter's encoding);
- the Storage Standard: storage keys and the `storage` event across
  every document of a key in the user agent;
- the Fetch Standard's `Accept-Language` and HTML's
  `navigator.languages`;
- Media Queries 5's `prefers-color-scheme`, which settings now choose.

## 1. Decisions

### What the tabs of a profile share: the context

**A context per profile.** A new type in `engine`, `Context`, holds what
every tab of one profile shares: the cookie jar, the HTTP cache, the
HSTS hosts, the permissions, the storage (`localStorage`, IndexedDB and
the profile's writer), the downloads, the settings, and the numbering
of renderers. A tab is made in a context, in either build:
`RemoteTab::in_context` in many processes, and `Engine::in_context` in
one; the constructors of today make a context of their own, so every
test and tool that makes one tab goes on as it is. The DevTools panel's
tab keeps a context of its own, empty, as it keeps no storage.

**What stays the tab's**: its renderers, its network service (whose
client is the context's jar, cache, HSTS and permissions with the tab's
own network log), its session history and segments, its
`sessionStorage` areas, its frames' registry, its DevTools panel and
its notices. `Storage` keeps `sessionStorage` by tab, so that one
`Storage`, the context's, serves every tab's service.

**Renderers are numbered by the context**, not from 1 in each tab: a
renderer's number names its opaque origins and its grants, and two
tabs' renderers must not share either. Renderers stay each tab's own
(a site in two tabs is two renderers, as the reference browser keeps
two tabs apart under memory pressure); the browser caps them across
tabs (below, "Tabs").

**`storage` events cross renderers and tabs.** M34 left them among
one renderer's engines (`processes.md` section 4). The context's
storage now tells every renderer of every tab of the profile, by a new
`ToRenderer::StorageChanged`, the change of a key whose documents it
may hold (its lock's site), and the renderer fires it at its
documents of that key but the one that made it. In one process the
context hands the change to each tab's engine.

### Tabs (M37.1)

**The session holds tabs.** `Session` holds a list of tabs, each a view
(`RemoteTab` or `Engine`) with its own DevTools panel, toolbar state
(the field's text while it is edited) and notice, and the index of the
one shown. The tab shown is the only one drawn and the only one given
input; the others are told they are hidden (`set_hidden`, as an
occluded window's page is: `visibilitychange`, timers at most once a
second, M31), and are drawn again when shown, from their renderers'
last frame.

**The tab strip is the `ui` crate's**, a new `TabStrip` beside the
`Toolbar`, drawn the same way (a display list rasterized in software,
text shaped by `text`; D9), in a row of `STRIP_HEIGHT` (32 CSS px)
above the toolbar, so the page's top is 72. Each tab shows its title
(or its URL while it has none), a mark while it loads, a close button,
and the tab shown drawn joined to the toolbar; a new-tab button follows
the last. Tabs are `TAB_WIDTH` (220) wide and shrink to `MIN_TAB_WIDTH`
(48) as more come; past that the strip scrolls with the wheel and keeps
the tab shown in sight. A tab is dragged along the strip to move it. A
middle click on a tab closes it.

**Keys**: Ctrl+T a new tab (Cmd on macOS, as every key below), Ctrl+W
and Ctrl+F4 close the tab shown, Ctrl+Shift+T opens the last tab
closed again (the last `MAX_CLOSED`, 10, kept by URL and title),
Ctrl+Tab and Ctrl+PageDown the next tab, Ctrl+Shift+Tab and
Ctrl+PageUp the one before, Ctrl+1 to Ctrl+8 the tab of that number,
Ctrl+9 the last. A new tab opens on the start page (settings, below)
with the address field focused. Closing the last tab closes the window.

**The window's title** is the tab shown's, as today's is the one tab's.

**Renderers across tabs**: each tab keeps at most `MAX_RENDERERS` (16),
as today, and the browser at most `MAX_ALL_RENDERERS` (48) across its
tabs. When a tab needs a renderer past that, the browser *discards* the
hidden tab least recently shown: its renderers end, and the tab keeps
its URL, title and history's URLs, showing its page again by loading
it when it is next shown, its documents' state lost (as a segment's is
past its renderer today, G117). A tab is never discarded while shown.
`MAX_TABS` (500) bounds a window's tabs.

**Crash placeholders** stay each tab's: a renderer that ends shows its
page in its tab (M34.4), the tab's title saying so, and a reload brings
it back.

### Pages opening tabs

**From the user**: a middle click on a link, or a click with Ctrl,
opens it in a new tab after the one shown, not shown (a click with
Ctrl+Shift shows it); the link's referrer policy and `noreferrer` hold,
and the new tab's document is never given an opener.

**From the page** (HTML's "rules for choosing a navigable"): `_blank`,
or a name no navigable of the tab has, as a link's, a form's or
`window.open`'s target, opens a new tab, shown, after the one it came
from. `window.open` is defined now; it opens a tab only with transient
user activation, as the reference browser's popup blocker allows,
otherwise the notice says a popup was blocked and it returns `null`.
Every new tab is opened as with `noopener` (HTML's default for `_blank`
since 2021): `window.open` returns `null`, the new document's `opener`
is `null`, and it gets no copy of `sessionStorage`. A `WindowProxy` for
a tab in another renderer, and the opener relation itself, are not in
this milestone (G90's and G122's); `window.open` with a features string
of `popup` opens a tab as well (no popup windows). A sandboxed frame
without `allow-popups` is refused as today; `allow-popups-to-escape-
sandbox` is honoured, the new tab having no sandbox of its own then.
`window.close` closes a tab a script opened, as the standard allows,
and no other.

### Downloads (M37.2)

**What the response says**: `net` gains `content_disposition`, RFC
6266's grammar read leniently as the reference browser reads it: its
type (`attachment` or `inline`), and a file name from `filename*`
(RFC 8187: UTF-8 or ISO-8859-1, percent-decoded) over `filename`
(a token or a quoted string, read as UTF-8 and then as Latin-1 when
not UTF-8). A fuzz target, `content_disposition`, holds it to not
panicking and to a name past which the sanitizing below is all that
stands.

**Where it is saved, and as what**: the user's downloads folder (the
known folder on Windows, `~/Downloads` elsewhere), under a name made
safe: the response's name, else the `download` attribute's, else the
URL's last path segment, else `download`; path separators, control
characters and characters Windows refuses replaced by `_`, leading
dots and trailing dots and spaces removed, Windows' reserved device
names (`CON`, `NUL`, `COM1`, ...) given a `_`, cut to `MAX_FILE_NAME`
(200 bytes, its extension kept); an extension taken from the MIME type
when there is none; and ` (1)`, ` (2)`, ... before the extension when
the name is taken. It is written as `<name>.silverdownload` and renamed
when complete. On Windows each file carries the mark of the web, the
`Zone.Identifier` stream (`ZoneId=3`, the internet zone, with its URL),
so that Windows treats it as from the internet.

**Who writes it**: the browser alone. A download is a navigation's
response that commits no document (as today's `Outcome::Download`), or
a link with a `download` attribute of its own origin, or of `data:` or
`blob:`. The network service in the browser already keeps another
site's download's body from the renderer (M34); it now streams any
download's body to the file itself. A `blob:` URL's bytes are the
renderer's, sent to the browser in pieces under `MAX_CHUNK` by a new
`ToBrowser::Download`, at most `MAX_PAGE_DOWNLOAD` (256 MiB), with a
user activation the renderer reports; the browser trusts it no further
than to write a file into the downloads folder under the mark of the
web. In one process the engine hands the same to the context's
downloads.

**Dangerous files are held**: a file whose extension runs code when
opened (`exe`, `msi`, `bat`, `cmd`, `com`, `scr`, `ps1`, `vbs`, `js`,
`jse`, `wsf`, `hta`, `lnk`, `reg`, `jar`, `dll`, `cpl`, `msc`, `app`,
`dmg`, `pkg`, `sh`, ...) stays `.silverdownload` when complete until the
user keeps it in the downloads page; until then it cannot be opened.

**Shown as**: the status bubble's notice "Saving <name>" and "Saved
<name>" in the tab that asked, and `about:downloads` (below): each
download's name, URL, size, progress and state (in progress, complete,
held, cancelled, failed), with Open, Show in folder, Cancel, Keep and
Remove from the list. Ctrl+J opens it. The list keeps the last
`MAX_DOWNLOADS` (100), in the profile. A download in progress when the
browser ends is cancelled and its partial file removed.

### The browser's pages

**Internal pages generalize the DevTools panel's bridge.** M36 made
`devtools:panel` a page of the browser's own, made from a response of
its own, in a renderer under `Lock::DevTools` refused every fetch and
all storage, its global `devtools` speaking JSON to a router in the
shell. The same makes `about:settings`, `about:downloads` and
`about:profiles`: each a page bundled in the shell's executable,
under a CSP of `default-src 'none'` with its inline script and style
alone, in a renderer under a new `Lock::Internal`, with a global
`silver` (`send`, `onmessage`) whose requests the shell's router
answers from the browser's own records (`settings.get`, `settings.set`,
`downloads.list`, `downloads.open`, `profiles.list`, ...), checked
against the page's name. No page reaches them: a link, a script or a
redirect to one is refused as `devtools:` is; the user reaches them by
typing the address, by the menu keys, or from another internal page.

**`about:silver` becomes the gallery** M15 promised: the engine's
demonstration pages (`crates/engine/tests/demos/`) bundled and linked,
each under `about:silver/<name>` with the page kept as today's made
page with an opaque origin. `about:blank` is as today. `about:version`
gives the browser's version, the engine's commit, the build and the
profile's folder.

**Error pages, finished**: one style for every made page (the error
pages, the crash page, the TLS failure, the blocked frame), in light
and dark after the setting; a title and a sentence in plain words, the
details (the error's name and the URL) under them, and a "Try again"
button where a retry can help (a network error, a crash), which reloads
through the made page's `about:silver-reload` as `about:silver-go-back`
goes back today.

### Settings (M37.3)

**Kept in the profile**: a `settings` table, `SCHEMA` 4, each setting
a key and a JSON value, sealed as every value is; read when the profile
is opened and written as it changes. A profile of `SCHEMA` 3 gains the
table. With no profile (`--temporary-profile`, or a private window) the
settings are the defaults, kept in memory. A command-line flag given
overrides its setting for that run.

**The settings, v1**:

- *On start*: the start page (`about:silver`, a blank page, or a URL),
  or the tabs open when the browser last ended (the session restored:
  each window's tabs' URLs and titles, written to the profile as they
  change, at most `MAX_TABS`).
- *New tabs open* the start page or a blank page.
- *Search*: the engine text typed in the address field goes to when it
  is no address, from a list (DuckDuckGo, the default; Google; Bing;
  Startpage; Brave Search), each a URL template the text is put into,
  percent-encoded. A search sends nothing as it is typed: no
  suggestions.
- *Scripts*: on or off for every site (no-scripts mode); the flags
  still override.
- *Appearance*: system, light or dark: the toolbar, the tab strip and
  the made pages drawn dark, and the pages' `prefers-color-scheme`.
- *Languages*: an ordered list from the languages the engine's fonts
  can show, sent as `Accept-Language` with descending `q` values and
  given to `navigator.language` and `navigator.languages`.
- *Third-party cookies*: sent (the default, D25) or blocked (G93): a
  frame or a request of another site than the top neither sends nor
  sets cookies, partitioned or not.
- *Downloads*: the folder, chosen by typing its path, and whether to
  ask where each goes (no: the shell has no file dialog yet).
- *Engine*: the GPU backend on or off, taking effect at the next start;
  the layer budget.

The roadmap's "CSP toggle" is not a setting: turning a page's policy off
removes a defence the page asked for, and is not offered.

### Profiles (M37.5)

**A window per profile, and windows.** The browser holds several
windows, each a session of tabs in one context; windows of the same
profile share its context. Ctrl+N opens a window of the profile shown,
Ctrl+Shift+N a *private* window: a context of a profile held in memory
(`profile::Memory`), its cookies, storage, history and downloads list
gone when its last window closes (its downloaded files stay where they
were saved), its windows marked in the tab strip. The browser ends
when its last window closes.

**Profiles are folders**: the default profile stays where it is
(`%LOCALAPPDATA%\SilverBrowser\Profile`, M28); others are
`...\SilverBrowser\Profiles\<name>`, a name of letters, digits, spaces,
dashes and underscores, at most 64 characters. `about:profiles` lists
them, makes one and opens one in a new window; `--profile-name <name>`
starts in one. A profile is opened by one browser process at a time
(the profile's own rule, M28): a second browser started on it says so
and ends, as today (handing the request to the running one is a G item).

### Bounds

`MAX_TABS` (500) a window, `MAX_WINDOWS` (50), `MAX_ALL_RENDERERS` (48)
across tabs, `MAX_CLOSED` (10), `MAX_DOWNLOADS` (100) kept,
`MAX_FILE_NAME` (200 bytes), `MAX_PAGE_DOWNLOAD` (256 MiB) a `blob:`
download, `MAX_SETTING_BYTES` (64 KiB) a setting's value,
`MAX_PROFILE_NAME` (64), and the internal pages' messages bounded as
the panel's (`MAX_MESSAGE_BYTES`).

### Fuzzing

`content_disposition` (the header's parse, and the name made safe:
never a separator, a reserved name, a control character or past
`MAX_FILE_NAME`); `ipc_decode` gains the new messages' seeds.

### The acceptance (M37.6)

The roadmap's gate is the maintainer's own browsing: half an hour of
real sites each week, its failures filed. What the code does for it:
a session test that browses as a person does across tabs on local
servers (a link opened in a new tab, a search from the address field,
a file downloaded and found in the folder, a setting changed and kept
across a restart, the session restored, a private window keeping
nothing); `docs/dogfood.md`, saying how to run the browser for it,
what to try, and how to write a failure down (a G item, or a fix); and
the milestone's report naming the first session as the maintainer's to
do.

## 2. Not in this milestone, said rather than hidden

- *An opener across tabs*: `window.open` returns `null` and `opener` is
  `null`; a script cannot reach the tab it opened, nor be reached
  (OAuth popups that post back to their opener fail). G item.
- *Popup windows*, and a window's `features` (size, position).
- *Favicons* in the tab strip. G item.
- *A file dialog* to choose where a download goes, and the shell's
  dialogs generally. G item.
- *Asking before a site downloads many files*, and resuming a download
  that failed. G items.
- *A second browser process handing its request to the first* on one
  profile. G item.
- *Bookmarks and a browsing history* kept across sessions (the session
  restored keeps only open tabs). G item.
- *Sync, extensions, password saving*: not planned.
- *Tabs dragged out into windows, and windows' tabs dragged between
  them.*
- *The `color-scheme` property, `light-dark()` and system colours*:
  the dark setting sets `prefers-color-scheme` and draws the browser's
  own parts dark; pages without dark styles stay light. G item.

## 3. Order of work

Each step a vertical slice, with its tests, a sabotage pass and the
gate.

1. *The context*: what a profile's tabs share, in both builds;
   renderers numbered by it; `storage` events across renderers and
   tabs.
2. *Tabs*: the session's tabs, `ui::TabStrip`, the keys and the
   pointer, hidden tabs, the window's title, the cap across tabs and
   discarding.
3. *Pages opening tabs*: middle and Ctrl clicks, `_blank` and names,
   `window.open` with activation, `window.close`, the closed tabs
   reopened.
4. *Downloads*: `Content-Disposition`, names made safe, the browser's
   writer, the mark of the web, the `download` attribute, `data:` and
   `blob:`, held files; the `content_disposition` fuzz target.
5. *Internal pages*: `Lock::Internal`, the `silver` bridge and its
   router, `about:downloads` and Ctrl+J, Open and Show in folder.
6. *Settings*: the profile's table, `about:settings`, and each setting
   applied: start and the session restored, search, scripts,
   appearance, languages, third-party cookies, downloads, engine.
7. *The made pages*: the gallery, `about:version`, the error pages'
   style and "Try again".
8. *Profiles and windows*: several windows, Ctrl+N, private windows,
   `about:profiles`, `--profile-name`.
9. *Acceptance*: the session test and `docs/dogfood.md`.
10. *T4.3 and the fuzz runs*.
11. *The documents*.

## 4. Corrected to the code

Filled in at the last step.
