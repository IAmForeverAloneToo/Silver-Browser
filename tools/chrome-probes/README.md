# Chrome probes

The pages and outputs that `docs/chrome-surface.md` cites: what the
reference browser (the installed Chrome stable) does, recorded with the
version it ran on. A probe whose version is not the installed Chrome's
(`chrome.exe --version`) is re-run before anything leans on it.

Each driver runs Chrome headless, with a throwaway profile that it
removes after, never the user's.

- `probe.mjs <page url> <out.json> <seconds>`: the page's
  `window.__done` and its console, with the version (CDP's
  `Browser.getVersion`).
- `read-out.mjs <page url> <out file> <seconds> [time zone]`: the text
  of the page's `#out`.
- `ua/ua_server.py <out-headers.jsonl> <port>`: serves the UA page and
  records each request's headers; read the page with `read-out.mjs`.

Web Crypto needs a secure context, so serve the pages from 127.0.0.1,
for example:

    cd tools/chrome-probes/webcrypto
    python -I -m http.server 8796 --bind 127.0.0.1
    node ../probe.mjs http://127.0.0.1:8796/chacha.html chacha.json 5

A page that needs SharedArrayBuffer (`buffer-kinds`, `buffer-members`,
`supports-resizable`) needs cross-origin isolation: `coi_server.py`
serves its directory with COOP and COEP, for example:

    python -I tools/chrome-probes/webcrypto/coi_server.py 8799
    node tools/chrome-probes/probe.mjs http://127.0.0.1:8799/buffer-kinds.html buffer-kinds.json 30
