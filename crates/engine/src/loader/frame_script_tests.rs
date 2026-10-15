//! Scripts across frames (M33.2, `docs/design/hardening.md`): the
//! windows of other frames, which answer what another origin may ask,
//! messages and ports between frames, and navigating another frame.

use net::Url;
use net::test_server::TestServer;

use crate::testing::*;

/// A server of `pages`, path and body, under two sites: a page at
/// `http://a.localhost:port/path` or `http://b.localhost:port/path`.
fn sites(pages: &'static [(&'static str, &'static str)]) -> TestServer {
    serve(move |path| {
        pages
            .iter()
            .find(|(at, _)| *at == path)
            .map(|(_, body)| html(body))
    })
}

/// `path` of `server` under `host`.
fn at(server: &TestServer, host: &str, path: &str) -> Url {
    let port = server.url("/").port().expect("a port");
    Url::parse(&format!("http://{host}:{port}{path}")).expect("a URL")
}

/// A tab with scripts on, `url` loaded, run until the console holds
/// `count` lines: the lines, the port of `server` written `PORT`.
fn run(server: &TestServer, url: Url, count: usize) -> Vec<String> {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(url);
    let port = server.url("/").port().expect("a port").to_string();
    tab.run_until(|lines| lines.len() >= count)
        .into_iter()
        .map(|line| line.replace(&port, "PORT"))
        .collect()
}

#[test]
fn another_frames_window_answers_what_another_origin_may_ask() {
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<iframe name=bee src='http://b.localhost:PORT/child.html'></iframe>\
             <script>\
             const f = document.querySelector('iframe');\
             const port = location.port;\
             f.src = f.src.replace('PORT', port);\
             onload = () => {\
               const w = f.contentWindow;\
               console.log(length, frames[0] === w, frames.bee === w, f.contentDocument);\
               console.log(w.parent === window, w.top === window, w.length, w.closed, w.self === w);\
               for (const get of [() => w.document, () => w.name, () => w.location.href]) {\
                 try { get(); } catch (e) { console.log(e.name, e.message); }\
               }\
               try { w.foo = 1; } catch (e) { console.log(e.name, e.message); }\
               console.log(Object.getOwnPropertyNames(w).join());\
               console.log(Object.getOwnPropertyNames(w.location).join());\
             };\
             </script>",
        ),
        ("/child.html", "<p>child</p>"),
    ];
    let server = sites(PAGES);
    let lines = run(&server, at(&server, "a.localhost", "/page.html"), 8);
    let blocked = |verb: &str, what: &str, iface: &str| {
        format!(
            "SecurityError Failed to {verb} a named property '{what}' {} '{iface}': Blocked a \
             frame with origin \"http://a.localhost:PORT\" from accessing a cross-origin frame.",
            if verb == "read" { "from" } else { "on" }
        )
    };
    assert_eq!(
        lines,
        [
            "1 true true null".to_owned(),
            "true true 0 false true".to_owned(),
            blocked("read", "document", "Window"),
            blocked("read", "name", "Window"),
            blocked("read", "href", "Location"),
            blocked("set", "foo", "Window"),
            "window,self,location,closed,frames,length,top,opener,parent,blur,close,focus,\
             postMessage,then"
                .to_owned(),
            "href,replace,then".to_owned(),
        ]
    );
}

#[test]
fn a_frame_finds_its_parent_and_top_and_its_name() {
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<iframe name=first srcdoc=\"<script>console.log(parent !== window, \
             top === parent, window.name, length, frameElement); \
             try { parent.document } catch (e) { console.log(e.name) }</script>\"></iframe>",
    )];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 2);
    assert_eq!(
        lines,
        [
            "[frame about:srcdoc] true true first 0 null",
            "[frame about:srcdoc] SecurityError",
        ]
    );
}

#[test]
fn messages_go_between_frames_by_their_target_origins() {
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<iframe src=child.html></iframe><script>\
             onmessage = e => console.log('parent got', e.data, e.origin === location.origin, \
               e.source === frames[0]);\
             onload = () => {\
               frames[0].postMessage('to nobody', 'http://c.localhost');\
               frames[0].postMessage('ping', '*');\
               try { frames[0].postMessage('x', 'not a url') } catch (e) { console.log(e.name) }\
             };\
             </script>",
        ),
        (
            "/child.html",
            "<script>onmessage = e => {\
               console.log('child got', e.data, e.source === parent);\
               e.source.postMessage('pong', e.origin);\
             };</script>",
        ),
    ];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 3);
    let child = server.url("/child.html");
    let mut sorted = lines.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        [
            "SyntaxError".to_owned(),
            format!(
                "[frame {}] child got ping true",
                child
                    .as_str()
                    .replace(&server.url("/").port().unwrap().to_string(), "PORT")
            ),
            "parent got pong true true".to_owned(),
        ]
    );
}

#[test]
fn a_port_goes_to_a_frame_and_answers() {
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<iframe srcdoc=\"<script>onmessage = e => e.ports[0].postMessage('pong ' + \
             e.data)</script>\"></iframe><script>\
             onload = () => {\
               const channel = new MessageChannel();\
               channel.port1.onmessage = e => console.log(e.data);\
               frames[0].postMessage('ping', '*', [channel.port2]);\
             };</script>",
    )];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 1);
    assert_eq!(lines, ["pong ping"]);
}

#[test]
fn a_frame_navigates_what_it_may_and_is_refused_the_rest() {
    // `b`'s frame may navigate its own origin's sibling and the top, not
    // the sibling of `a`'s (the reference browser's rule, p6).
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<iframe id=n src='http://b.localhost:PORT/navigator.html'></iframe>\
             <iframe name=a src=x.html></iframe>\
             <iframe name=b id=third src='http://b.localhost:PORT/x.html'></iframe><script>\
             for (const f of document.querySelectorAll('iframe')) {\
               f.src = f.src.replace('PORT', location.port);\
             }</script>",
        ),
        (
            "/navigator.html",
            "<script>onload = () => setTimeout(() => {\
               try { parent.frames.a.location = 'y.html' } \
               catch (e) { console.log(e.name, e.message.replace(/'http.*'/, 'URL')) }\
               parent.frames.b.location.href = 'y.html';\
             }, 50)</script>",
        ),
        ("/x.html", "<p>x</p>"),
        ("/y.html", "<script>console.log('y here')</script>"),
    ];
    let server = sites(PAGES);
    let lines = run(&server, at(&server, "a.localhost", "/page.html"), 2);
    assert_eq!(
        lines,
        [
            "[frame http://b.localhost:PORT/navigator.html] SecurityError Failed to set a named \
             property 'href' on 'Location': The current window does not have permission to \
             navigate the target frame to URL.",
            "[frame http://b.localhost:PORT/y.html] y here",
        ]
    );
}

#[test]
fn links_and_forms_target_frames_by_name_parent_and_top() {
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<a href=named.html target=bee id=l>named</a>\
             <iframe name=bee src=x.html></iframe>\
             <iframe srcdoc=\"<a id=t href='/top.html' target=_top>top</a>\
             <script>onmessage = () => document.getElementById('t').click()</script>\"></iframe>\
             <script>onload = () => document.getElementById('l').click(); \
             onmessage = () => frames[1].postMessage('go', '*')</script>",
        ),
        ("/x.html", "<p>x</p>"),
        // The other frame's link to the top is followed once this ran.
        (
            "/named.html",
            "<script>console.log('named here', name); parent.postMessage('ran', '*')</script>",
        ),
        ("/top.html", "<script>console.log('top here')</script>"),
    ];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 2);
    let named = server
        .url("/named.html")
        .as_str()
        .replace(&server.url("/").port().unwrap().to_string(), "PORT");
    assert_eq!(
        lines,
        [
            format!("[frame {named}] named here bee"),
            "top here".to_owned()
        ]
    );
}

#[test]
fn an_inserted_iframe_has_its_window_at_once() {
    // Its number is reserved for the frame its parent's engine makes
    // after the run; its index on the window follows then.
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<script>\
             const f = document.createElement('iframe');\
             document.documentElement.appendChild(f);\
             const w = f.contentWindow;\
             console.log(w !== null, length, w.parent === window);\
             setTimeout(() => console.log(frames[0] === w, f.contentWindow === w), 50);\
             </script>",
    )];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 2);
    assert_eq!(lines, ["true 1 true", "true true"]);
}

#[test]
fn a_frames_name_its_script_set_stays_across_its_navigations() {
    static PAGES: &[(&str, &str)] = &[
        ("/page.html", "<iframe name=given src=one.html></iframe>"),
        (
            "/one.html",
            "<script>console.log(name); onload = () => { name = 'mine'; \
             location = 'two.html' }</script>",
        ),
        ("/two.html", "<script>console.log(name)</script>"),
    ];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 2);
    let port = server.url("/").port().unwrap().to_string();
    let url = |path: &str| server.url(path).as_str().replace(&port, "PORT");
    assert_eq!(
        lines,
        [
            format!("[frame {}] given", url("/one.html")),
            format!("[frame {}] mine", url("/two.html")),
        ]
    );
    // The frame's own navigation is an iframe's.
    let two = server
        .requests()
        .into_iter()
        .find(|request| request.target == "/two.html")
        .expect("asked");
    assert_eq!(two.header("Sec-Fetch-Dest"), Some("iframe"));
}

#[test]
fn an_iframe_taken_out_before_its_frame_is_made_leaves_no_frame() {
    // The number a script's question reserved goes with the element.
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<script>\
             const f = document.createElement('iframe');\
             document.documentElement.appendChild(f);\
             const w = f.contentWindow;\
             f.remove();\
             setTimeout(() => console.log(length, w.closed), 50);\
             </script>",
    )];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 1);
    assert_eq!(lines, ["0 true"]);
}

#[test]
fn a_form_submits_into_the_frame_its_target_names() {
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<form action=formed.html target=bee><input name=q value=1></form>\
             <iframe name=bee src=x.html></iframe>\
             <script>onload = () => document.forms[0].submit()</script>",
        ),
        ("/x.html", "<p>x</p>"),
        (
            "/formed.html?q=1",
            "<script>console.log('formed', location.search)</script>",
        ),
    ];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 1);
    let formed = server
        .url("/formed.html?q=1")
        .as_str()
        .replace(&server.url("/").port().unwrap().to_string(), "PORT");
    assert_eq!(lines, [format!("[frame {formed}] formed ?q=1")]);
}

#[test]
fn frames_answer_in_tree_order_every_time() {
    // Six frames each answer their parent's message; their work is done
    // in their elements' order (M33), so the answers come in it, as the
    // web-platform tests of messages between frames expect.
    static PAGES: &[(&str, &str)] = &[
        (
            "/top.html",
            "<script>var got = []; addEventListener('message', e => { got.push(e.data); \
             if (got.length === 6) console.log(got.join(' ')); }); \
             onload = () => { for (var i = 0; i < 6; i++) frames[i].postMessage(i, '*'); };</script>\
             <iframe src=/f.html></iframe><iframe src=/f.html></iframe><iframe src=/f.html></iframe>\
             <iframe src=/f.html></iframe><iframe src=/f.html></iframe><iframe src=/f.html></iframe>",
        ),
        (
            "/f.html",
            "<script>addEventListener('message', e => parent.postMessage(e.data, '*'));</script>",
        ),
    ];
    let server = sites(PAGES);
    for _ in 0..3 {
        assert_eq!(
            run(&server, at(&server, "a.localhost", "/top.html"), 1),
            ["0 1 2 3 4 5"]
        );
    }
}
