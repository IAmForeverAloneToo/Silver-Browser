//! Sandboxed frames and frames refused (M33.2, `docs/design/hardening.md`):
//! the `sandbox` attribute's flags, the parent's and a response's CSP
//! `sandbox`, what each guards, and `X-Frame-Options` and
//! `frame-ancestors`, as the reference browser has them (p2, p3).

use net::Url;
use net::test_server::{Reply, TestServer};

use crate::testing::*;

/// Pages by path, each with its header fields and body.
type Pages = &'static [(
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static str,
)];

/// A server of `pages` under two sites: a page at
/// `http://a.localhost:port/path` or `http://b.localhost:port/path`, a
/// query left aside.
fn sites(pages: Pages) -> TestServer {
    serve(move |target| {
        let path = target.split('?').next().unwrap_or_default();
        let (_, fields, body) = pages.iter().find(|(at, ..)| *at == path)?;
        let mut all = vec![("Content-Type", "text/html")];
        all.extend_from_slice(fields);
        Some(Reply::with_fields(200, &all, body.as_bytes()))
    })
}

fn at(server: &TestServer, host: &str, path: &str) -> Url {
    let port = server.url("/").port().expect("a port");
    Url::parse(&format!("http://{host}:{port}{path}")).expect("a URL")
}

/// A tab with scripts on, `url` loaded, run until a line of the console
/// ends with `last`: the lines, the port written `PORT`, in order.
fn run_to(server: &TestServer, url: Url, last: &str) -> Vec<String> {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(url);
    let port = server.url("/").port().expect("a port").to_string();
    let mut lines: Vec<String> = tab
        .run_until(|lines| lines.iter().any(|line| line.ends_with(last)))
        .into_iter()
        .map(|line| line.replace(&port, "PORT"))
        .collect();
    lines.sort();
    lines
}

/// What a frame's document posts its parent: its hash, its origin, and
/// what its cookies, its storage and `document.domain` answer.
const REPORT: &str = "<script>\
    function t(f) { try { return String(f()); } catch (e) { return e.name + ': ' + e.message; } }\
    parent.postMessage([location.hash, self.origin, t(() => document.cookie),\
      t(() => localStorage.length), t(() => document.domain),\
      t(() => { document.domain = 'a.localhost'; return 'set'; })].join(' | '), '*');\
    </script>";

const FORBIDDEN: &str = "SecurityError: Failed to set the 'domain' property on 'Document': \
                         Assignment is forbidden for sandboxed iframes.";

const LACKING: &str = "The document is sandboxed and lacks the 'allow-same-origin' flag.";

/// What `REPORT` says of a frame of origin `null` at `hash`.
fn opaque(hash: &str) -> String {
    format!(
        "null {hash} | null | SecurityError: Failed to read the 'cookie' property from 'Document': \
         {LACKING} | SecurityError: Failed to read the 'localStorage' property from 'Window': \
         {LACKING} |  | {FORBIDDEN}"
    )
}

/// What `REPORT` says of a frame of `http://a.localhost` at `hash`
/// sandboxed with `allow-same-origin`.
fn same_origin(hash: &str) -> String {
    format!(
        "http://a.localhost:PORT {hash} | http://a.localhost:PORT |  | 0 | a.localhost | \
         {FORBIDDEN}"
    )
}

/// A tab with scripts on, `url` loaded, run until the console holds
/// `count` lines: the lines, the port written `PORT`, in order.
fn run_count(server: &TestServer, url: Url, count: usize) -> Vec<String> {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(url);
    let port = server.url("/").port().expect("a port").to_string();
    let mut lines: Vec<String> = tab
        .run_until(|lines| lines.len() >= count)
        .into_iter()
        .map(|line| line.replace(&port, "PORT"))
        .collect();
    lines.sort();
    lines
}

#[test]
fn a_sandboxed_frame_runs_no_script_and_is_of_an_opaque_origin() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[],
            "<script>onmessage = e => console.log(e.origin + ' ' + e.data);\
             onload = () => console.log('loaded');</script>\
             <iframe sandbox src=report.html#none></iframe>\
             <iframe sandbox=allow-scripts src=report.html#scripts></iframe>\
             <iframe sandbox='allow-scripts bogus ALLOW-SAME-ORIGIN' src=report.html#same>\
             </iframe>",
        ),
        ("/report.html", &[], REPORT),
    ];
    let server = sites(PAGES);
    let lines = run_to(&server, at(&server, "a.localhost", "/page.html"), "loaded");
    assert_eq!(
        lines,
        [
            same_origin("#same"),
            "loaded".to_owned(),
            opaque("#scripts"),
        ]
    );
}

#[test]
fn the_flags_go_down_to_frames_and_srcdoc_and_come_from_csp() {
    // A frame in a sandboxed frame, a sandboxed srcdoc, and a response's
    // `Content-Security-Policy: sandbox`, at a frame and at the top.
    static PAGES: Pages = &[
        (
            "/page.html",
            &[],
            "<script>onmessage = e => console.log(e.origin + ' ' + e.data);\
             onload = () => console.log('loaded');</script>\
             <iframe sandbox=allow-scripts src=nest.html></iframe>\
             <iframe sandbox=allow-scripts srcdoc=\"<script>parent.postMessage(\
               [location.href, self.origin].join(' | '), '*')</script>\"></iframe>\
             <iframe src=csp.html#csp></iframe>",
        ),
        (
            "/nest.html",
            &[],
            "<script>onmessage = e => top.postMessage(e.data, '*')</script>\
             <iframe src=report.html#inner></iframe>",
        ),
        ("/report.html", &[], REPORT),
        (
            "/csp.html",
            &[("Content-Security-Policy", "sandbox allow-scripts")],
            REPORT,
        ),
        (
            "/top.html",
            &[("Content-Security-Policy", "sandbox allow-scripts")],
            "<script>console.log('top ' + self.origin)</script>",
        ),
    ];
    let server = sites(PAGES);
    let lines = run_to(&server, at(&server, "a.localhost", "/page.html"), "loaded");
    assert_eq!(
        lines,
        [
            "loaded".to_owned(),
            opaque("#csp"),
            opaque("#inner"),
            "null about:srcdoc | null".to_owned(),
        ]
    );
    let lines = run_to(&server, at(&server, "a.localhost", "/top.html"), "null");
    assert_eq!(lines, ["top null"]);
}

#[test]
fn a_sandboxed_frame_submits_no_form_and_shows_no_dialog() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[],
            "<iframe sandbox=allow-scripts src=form.html?no></iframe>\
             <iframe sandbox='allow-scripts allow-forms allow-modals' src=form.html?yes>\
             </iframe>",
        ),
        (
            "/form.html",
            &[],
            "<form action=/landed.html><input name=a value=1></form>\
             <script>alert('hi'); console.log(confirm('sure?'), prompt('name?'));\
             onload = () => document.forms[0].submit()</script>",
        ),
        (
            "/landed.html",
            &[],
            "<script>console.log('landed', location.search)</script>",
        ),
    ];
    let server = sites(PAGES);
    let lines = run_count(&server, at(&server, "a.localhost", "/page.html"), 10);
    let no = "[frame http://a.localhost:PORT/form.html?no]";
    let yes = "[frame http://a.localhost:PORT/form.html?yes]";
    let ignored = |call: &str| {
        format!(
            "{no} Ignored call to '{call}()'. The document is sandboxed, and the \
             'allow-modals' keyword is not set."
        )
    };
    assert_eq!(
        lines,
        [
            format!(
                "{no} Blocked form submission to 'http://a.localhost:PORT/landed.html' \
                 because the form's frame is sandboxed and the 'allow-forms' permission is \
                 not set."
            ),
            ignored("alert"),
            ignored("confirm"),
            ignored("prompt"),
            format!("{no} false null"),
            format!("{yes} alert: hi"),
            format!("{yes} confirm: sure?"),
            format!("{yes} false null"),
            format!("{yes} prompt: name?"),
            "[frame http://a.localhost:PORT/landed.html?a=1] landed ?a=1".to_owned(),
        ]
    );
}

#[test]
fn a_sandboxed_frame_navigates_the_top_only_as_its_flags_allow() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[],
            "<iframe sandbox=allow-scripts src=nav.html></iframe>\
             <script>onload = () => console.log('loaded')</script>",
        ),
        (
            "/allowed.html",
            &[],
            "<iframe sandbox='allow-scripts allow-top-navigation' src=nav.html></iframe>",
        ),
        (
            "/by-user.html",
            &[],
            "<iframe sandbox='allow-scripts allow-top-navigation-by-user-activation' \
             src=nav.html></iframe><script>onload = () => console.log('loaded')</script>",
        ),
        (
            "/nav.html",
            &[],
            "<a target=_top href=/landed.html>top</a><a target=_blank href=/landed.html>new</a>\
             <script>\
             try { top.location.href = '/landed.html'; } catch (e) { console.log(e.name, e.message) }\
             const links = document.querySelectorAll('a');\
             links[0].click(); onload = () => links[1].click();\
             </script>",
        ),
        (
            "/landed.html",
            &[],
            "<script>console.log('landed')</script>",
        ),
    ];
    let server = sites(PAGES);
    let frame = "[frame http://a.localhost:PORT/nav.html]";
    let unsafe_top = format!(
        "{frame} Unsafe attempt to initiate navigation for frame with origin \
         'http://a.localhost:PORT' from frame with URL 'http://a.localhost:PORT/nav.html'. \
         The frame attempting navigation of the top-level window is sandboxed, but the flag \
         of 'allow-top-navigation' or 'allow-top-navigation-by-user-activation' is not set."
    );
    let popup = format!(
        "{frame} Blocked opening 'http://a.localhost:PORT/landed.html' in a new window \
         because the request was made in a sandboxed frame whose 'allow-popups' permission \
         is not set."
    );
    let denied = format!(
        "{frame} SecurityError Failed to set a named property 'href' on 'Location': The \
         current window does not have permission to navigate the target frame to \
         'http://a.localhost:PORT/landed.html'."
    );
    let lines = run_to(&server, at(&server, "a.localhost", "/page.html"), "loaded");
    assert_eq!(
        lines,
        [
            popup.clone(),
            denied.clone(),
            unsafe_top.clone(),
            "loaded".to_owned()
        ]
    );
    // A script's click is no user's activation.
    let lines = run_to(
        &server,
        at(&server, "a.localhost", "/by-user.html"),
        "loaded",
    );
    assert_eq!(lines, [popup, denied, unsafe_top, "loaded".to_owned()]);
    let lines = run_to(
        &server,
        at(&server, "a.localhost", "/allowed.html"),
        "landed",
    );
    assert_eq!(lines.last().map(String::as_str), Some("landed"));
}

#[test]
fn a_change_of_the_attribute_applies_at_the_next_navigation() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[],
            "<script>onmessage = e => console.log(e.origin + ' ' + e.data);</script>\
             <iframe src=again.html#before></iframe>\
             <script>\
             const f = document.querySelector('iframe');\
             onload = () => {\
               f.sandbox = 'allow-scripts';\
               f.contentWindow.postMessage('again', '*');\
               setTimeout(() => { f.src = 'again.html#after'; }, 50);\
             };\
             </script>",
        ),
        (
            "/again.html",
            &[],
            "<script>parent.postMessage(location.hash + ' ' + self.origin, '*');\
             onmessage = () => parent.postMessage(location.hash + ' still ' + self.origin, '*');\
             </script>",
        ),
    ];
    let server = sites(PAGES);
    let lines = run_to(
        &server,
        at(&server, "a.localhost", "/page.html"),
        "#after null",
    );
    assert_eq!(
        lines,
        [
            "http://a.localhost:PORT #before http://a.localhost:PORT",
            "http://a.localhost:PORT #before still http://a.localhost:PORT",
            "null #after null",
        ]
    );
}

#[test]
fn the_elements_sandbox_is_a_token_list_that_knows_the_reference_browsers_tokens() {
    static PAGES: Pages = &[(
        "/page.html",
        &[],
        "<iframe sandbox='allow-scripts bogus ALLOW-SAME-ORIGIN'></iframe><script>\
         const f = document.querySelector('iframe');\
         const s = f.sandbox;\
         console.log(s === f.sandbox, s.value, s.length, s.contains('bogus'));\
         console.log(['allow-forms', 'ALLOW-MODALS', 'allow-top-navigation-to-custom-protocols',\
           'allow-bogus'].map(t => s.supports(t)).join());\
         f.sandbox = 'allow-forms';\
         console.log(f.getAttribute('sandbox'), s.value);\
         try { document.body.classList.supports('x') } catch (e) { console.log(e.name) }\
         console.log('done');\
         </script>",
    )];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine
        .navigate(at(&server, "a.localhost", "/page.html"));
    let lines = tab.run_until(|lines| lines.iter().any(|line| line == "done"));
    assert_eq!(
        lines,
        [
            "true allow-scripts bogus ALLOW-SAME-ORIGIN 3 true",
            "true,true,false,false",
            "allow-forms allow-forms",
            "TypeError",
            "done",
        ]
    );
}

#[test]
fn x_frame_options_and_frame_ancestors_refuse_a_frame() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[],
            "<script>let loads = 0; onmessage = e => console.log('shown ' + e.data);\
             onload = () => console.log('loaded', loads, \
               [...document.querySelectorAll('iframe')].map(f => f.contentWindow.length).join());\
             </script>\
             <iframe onload=loads++ src=deny.html></iframe>\
             <iframe onload=loads++ src=same.html></iframe>\
             <iframe onload=loads++ src='http://b.localhost:PORT/same.html'></iframe>\
             <iframe onload=loads++ src=conflict.html></iframe>\
             <iframe onload=loads++ src=bogus.html></iframe>\
             <iframe onload=loads++ src=none.html></iframe>\
             <iframe onload=loads++ src=override.html></iframe>\
             <iframe onload=loads++ src=report-only.html></iframe>\
             <iframe onload=loads++ src=meta.html></iframe>\
             <script>\
             for (const f of document.querySelectorAll('iframe')) {\
               f.src = f.src.replace('PORT', location.port);\
             }\
             </script>",
        ),
        (
            "/deny.html",
            &[("X-Frame-Options", "DENY")],
            "<script>parent.postMessage('deny', '*')</script>",
        ),
        (
            "/same.html",
            &[("X-Frame-Options", "SAMEORIGIN")],
            "<script>parent.postMessage('same ' + self.origin, '*')</script>",
        ),
        (
            "/conflict.html",
            &[
                ("X-Frame-Options", "DENY"),
                ("X-Frame-Options", "SAMEORIGIN"),
            ],
            "<script>parent.postMessage('conflict', '*')</script>",
        ),
        (
            "/bogus.html",
            &[("X-Frame-Options", "ALLOW-FROM http://a.localhost")],
            "<script>parent.postMessage('bogus', '*')</script>",
        ),
        (
            "/none.html",
            &[("Content-Security-Policy", "frame-ancestors 'none'")],
            "<script>parent.postMessage('none', '*')</script>",
        ),
        (
            "/override.html",
            &[
                ("Content-Security-Policy", "frame-ancestors 'self'"),
                ("X-Frame-Options", "DENY"),
            ],
            "<script>parent.postMessage('override', '*')</script>",
        ),
        (
            "/report-only.html",
            &[(
                "Content-Security-Policy-Report-Only",
                "frame-ancestors 'none'",
            )],
            "<script>parent.postMessage('report-only', '*')</script>",
        ),
        (
            "/meta.html",
            &[],
            "<meta http-equiv=Content-Security-Policy content=\"frame-ancestors 'none'\">\
             <script>parent.postMessage('meta', '*')</script>",
        ),
    ];
    let server = sites(PAGES);
    let lines = run_to(
        &server,
        at(&server, "a.localhost", "/page.html"),
        "0,0,0,0,0,0,0,0,0",
    );
    let refused = |path: &str, why: &str| {
        format!(
            "[frame http://a.localhost:PORT/{path}] Refused to display \
             'http://a.localhost:PORT/{path}' in a frame because {why}."
        )
    };
    assert_eq!(
        lines,
        [
            refused(
                "conflict.html",
                "it set multiple 'X-Frame-Options' headers with conflicting values ('DENY, \
                 SAMEORIGIN'). Falling back to 'deny'"
            ),
            refused("deny.html", "it set 'X-Frame-Options' to 'deny'"),
            "[frame http://a.localhost:PORT/none.html] Refused to frame \
             'http://a.localhost:PORT/none.html' because an ancestor violates the following \
             Content Security Policy directive: \"frame-ancestors 'none'\"."
                .to_owned(),
            "[frame http://b.localhost:PORT/same.html] Refused to display \
             'http://b.localhost:PORT/same.html' in a frame because it set 'X-Frame-Options' \
             to 'sameorigin'."
                .to_owned(),
            "loaded 9 0,0,0,0,0,0,0,0,0".to_owned(),
            "shown bogus".to_owned(),
            "shown meta".to_owned(),
            "shown override".to_owned(),
            "shown report-only".to_owned(),
            "shown same http://a.localhost:PORT".to_owned(),
        ]
    );
}

#[test]
fn sameorigin_looks_at_every_ancestor() {
    // a, b inside it, a inside b: refused, as the probes found.
    static PAGES: Pages = &[
        (
            "/page.html",
            &[],
            "<iframe src='http://b.localhost:PORT/middle.html'></iframe><script>\
             const f = document.querySelector('iframe');\
             f.src = f.src.replace('PORT', location.port);\
             </script>",
        ),
        (
            "/middle.html",
            &[],
            "<iframe src='http://a.localhost:PORT/same.html'></iframe><script>\
             const f = document.querySelector('iframe');\
             f.src = f.src.replace('PORT', location.port);\
             </script>",
        ),
        (
            "/same.html",
            &[("X-Frame-Options", "SAMEORIGIN")],
            "<script>console.log('shown')</script>",
        ),
    ];
    let server = sites(PAGES);
    let lines = run_to(
        &server,
        at(&server, "a.localhost", "/page.html"),
        "'sameorigin'.",
    );
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("Refused to display 'http://a.localhost:PORT/same.html'"));
}

#[test]
fn a_top_level_document_is_never_refused() {
    static PAGES: Pages = &[(
        "/deny.html",
        &[
            ("X-Frame-Options", "DENY"),
            ("Content-Security-Policy", "frame-ancestors 'none'"),
        ],
        "<script>console.log('shown')</script>",
    )];
    let server = sites(PAGES);
    let lines = run_to(&server, at(&server, "a.localhost", "/deny.html"), "shown");
    assert_eq!(lines, ["shown"]);
}

#[test]
fn a_sandboxed_frames_requests_are_an_opaque_origins_and_it_parses_as_with_scripts_off() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[],
            "<iframe sandbox src=frame.html></iframe><iframe src=frame.html?scripts></iframe>\
             <script>onload = () => console.log('loaded')</script>",
        ),
        (
            "/frame.html",
            &[],
            "<noscript><img src=/seen.png></noscript><img src=/always.png>",
        ),
    ];
    let server = sites(PAGES);
    run_to(&server, at(&server, "a.localhost", "/page.html"), "loaded");
    let sites_of = |path: &str| {
        let mut sites: Vec<String> = server
            .requests()
            .into_iter()
            .filter(|request| request.target == path)
            .map(|request| {
                request
                    .header("Sec-Fetch-Site")
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect();
        sites.sort();
        sites
    };
    assert_eq!(sites_of("/seen.png"), ["cross-site"]);
    assert_eq!(sites_of("/always.png"), ["cross-site", "same-origin"]);
}

#[test]
fn a_sandboxed_frame_is_styled_as_with_scripts_off() {
    // Its `noscript` makes boxes and its media queries see `scripting`
    // as `none` (M33.2), where a frame allowed scripts sees neither.
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    let frame = "<style>@media (scripting: none) { body { background: rgb(0, 0, 255) } }</style>\
                 <body style='margin:0'><noscript>\
                 <div style='height:20px; background: rgb(255, 0, 0)'></div></noscript>";
    tab.engine.load_document(&format!(
        "<body style='margin:0; background: rgb(0, 128, 0)'>\
         <iframe sandbox style='display:block; border:0' srcdoc=\"{frame}\"></iframe>\
         <iframe sandbox=allow-scripts style='display:block; border:0' srcdoc=\"{frame}\"></iframe>"
    ));
    tab.wait_until(|engine| engine.frames_settled());
    let shown = tab.engine.frame(400, 400);
    let pixel = |x: usize, y: usize| {
        let at = (y * 400 + x) * 4;
        [shown[at], shown[at + 1], shown[at + 2]]
    };
    assert_eq!(pixel(10, 10), [255, 0, 0], "the noscript's div");
    assert_eq!(pixel(10, 100), [0, 0, 255], "scripting: none");
    assert_eq!(pixel(10, 160), [0, 128, 0], "allowed scripts: neither");
}

#[test]
fn a_sandboxed_frame_navigates_no_frame_but_its_own_and_its_descendants() {
    // Of its parent's origin and allowed its origin, it still may not
    // navigate its sibling, which an unsandboxed one may.
    static PAGES: Pages = &[
        (
            "/page.html",
            &[],
            "<iframe name=sib src=sib.html></iframe>\
             <iframe sandbox='allow-scripts allow-same-origin' src=nav.html></iframe>\
             <script>onload = () => console.log('loaded')</script>",
        ),
        (
            "/free.html",
            &[],
            "<iframe name=sib src=sib.html></iframe><iframe src=nav.html></iframe>",
        ),
        ("/sib.html", &[], "<p>sibling</p>"),
        (
            "/nav.html",
            &[],
            "<a target=sib href=/landed.html>sib</a>\
             <script>onload = () => document.querySelector('a').click()</script>",
        ),
        (
            "/landed.html",
            &[],
            "<script>console.log('landed')</script>",
        ),
    ];
    let server = sites(PAGES);
    let lines = run_to(&server, at(&server, "a.localhost", "/page.html"), "loaded");
    assert_eq!(
        lines,
        [
            "[frame http://a.localhost:PORT/nav.html] Unsafe attempt to initiate navigation for \
             frame with origin 'http://a.localhost:PORT' from frame with URL \
             'http://a.localhost:PORT/nav.html'. The frame attempting navigation is sandboxed, \
             and is therefore disallowed from navigating its ancestors.",
            "loaded",
        ]
    );
    let lines = run_to(&server, at(&server, "a.localhost", "/free.html"), "landed");
    assert_eq!(
        lines,
        ["[frame http://a.localhost:PORT/landed.html] landed"]
    );
}

#[test]
fn a_sandboxed_frames_svg_javascript_url_and_download_do_nothing() {
    static PAGES: Pages = &[
        (
            "/page.html",
            &[],
            "<script>onmessage = e => console.log('posted ' + e.data);\
             onload = () => console.log('loaded')</script>\
             <iframe sandbox src=image.svg></iframe>\
             <iframe sandbox src=\"javascript:parent.postMessage('js', '*')\"></iframe>\
             <iframe sandbox src=file.zip></iframe>\
             <iframe src=image.svg?free></iframe>",
        ),
        (
            "/image.svg",
            &[("Content-Type", "image/svg+xml")],
            "<svg xmlns='http://www.w3.org/2000/svg'>\
             <script>parent.postMessage('svg ' + location.search, '*')</script></svg>",
        ),
        (
            "/top.svg",
            &[
                ("Content-Type", "image/svg+xml"),
                ("Content-Security-Policy", "sandbox"),
            ],
            "<svg xmlns='http://www.w3.org/2000/svg'><script>console.log('ran')</script>\
             <image href='/after.png'/></svg>",
        ),
        ("/file.zip", &[("Content-Type", "application/zip")], "PK"),
    ];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine
        .navigate(at(&server, "a.localhost", "/page.html"));
    let port = server.url("/").port().expect("a port").to_string();
    let mut lines: Vec<String> = tab
        .run_until(|lines| lines.iter().any(|line| line == "loaded"))
        .into_iter()
        .map(|line| line.replace(&port, "PORT"))
        .collect();
    lines.sort();
    assert_eq!(
        lines,
        [
            "[frame about:blank] Download is disallowed. The frame initiating or instantiating \
             the download is sandboxed, but the flag 'allow-downloads' is not set.",
            "loaded",
            "posted svg ?free",
        ]
    );
    assert_eq!(tab.engine.take_notice(), None);
    // A response's own CSP sandboxes an SVG document at the top too.
    tab.load(at(&server, "a.localhost", "/top.svg"));
    let lines: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert!(lines.is_empty(), "{lines:?}");
    assert_eq!(tab.engine.scripts(), crate::ScriptState::Off);
}
