# Dogfooding Silver Browser

M37.6's gate is the maintainer's own browsing: half an hour of real
sites each week, with what fails written down. This page says how to
run the browser for it, what to try, and how to file what goes wrong.

## Running it

Build it once in release, then run it with nothing on the command line:
it opens on the start page the settings name (`about:silver` at first).

    cargo build --release -p silver-browser
    target/release/silver-browser

Scripts are on by default. The profile is the default one
(`%LOCALAPPDATA%\SilverBrowser\Profile` on Windows): cookies, storage,
settings and the tabs open last are kept there. To browse without
touching it, use a private window (Ctrl+Shift+N) or start with
`--temporary-profile`; to keep a profile of its own for dogfooding,
start with `--profile-name Dogfood` (the profiles are folders under
`SilverBrowser\Profiles`, and `about:profiles` lists and opens them).

## What to try

Each session, a little of each:

- **Reading.** News, documentation, a wiki, a blog: scroll long pages,
  follow links, go back and forward, reload.
- **Tabs.** Open links in new tabs (Ctrl+click or the middle button),
  switch with Ctrl+Tab and Ctrl+1..9, close with Ctrl+W, reopen with
  Ctrl+Shift+T; drag a tab along the strip.
- **Searching.** Type words that are not an address into the address
  field: they go to the search engine the settings name.
- **Forms.** Sign in somewhere, fill a search form, post a comment on a
  test page of your own.
- **Downloads.** Download a file; find it in `about:downloads` (Ctrl+J)
  and in the folder; open it from there.
- **Settings.** Change one in `about:settings` (the appearance, the
  languages, third-party cookies) and see it applied; restart and see
  it kept.
- **Sessions.** With "the tabs open when the browser last ended" as the
  start, close the browser with several tabs open and start it again.
- **Windows.** Ctrl+N for another window, Ctrl+Shift+N for a private
  one; close the private window and check that its cookies are gone.
- **When something breaks.** F12 opens the DevTools panel: the console,
  the elements, the network log.

## Writing a failure down

For each failure, one line in the session's notes first: the URL, what
was done, what was seen, what was expected. Then, for each:

1. Try it again with `--temporary-profile`, and with
   `--disable-scripts`: a failure that goes away with either says
   where to look.
2. If it is a page the engine draws wrongly, compare it with the
   reference browser and keep the smallest page that still shows it.
3. File it in `docs/roadmap.md`: a fix when it is small and clear, or a
   new G item (the next number) saying what fails, where it was found
   (the dogfood session's date) and what it would take.

A renderer that stops shows its page with "Try again" and a record of
what it was doing beside the profile (`crashes`); attach that record's
path to the failure, never its contents, unless it has been read first.

## The first session

The first session is the maintainer's to do; this file and the session
test (`a_day_of_browsing_on_local_servers`, in the shell's session
tests) are what the code does for it.
