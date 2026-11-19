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
    // Of its parent's origin (G90), its element and its parent's document
    // are its own to reach; sandboxed, of an opaque origin, neither. As
    // the reference browser. The parent runs a script of its own: one
    // that runs none has no realm to be reached by until phase 3.
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<script>0</script>\
         <iframe name=first srcdoc=\"<script>console.log(parent !== window, \
             top === parent, window.name, length, String(frameElement)); \
             try { console.log(parent.document ? 'read' : 'none') } \
             catch (e) { console.log(e.name) }</script>\"></iframe>\
         <iframe name=boxed sandbox=allow-scripts srcdoc=\"<script>console.log(parent !== window, \
             top === parent, window.name, length, String(frameElement)); \
             try { console.log(parent.document ? 'read' : 'none') } \
             catch (e) { console.log(e.name) }</script>\"></iframe>",
    )];
    let server = sites(PAGES);
    let mut lines = run(&server, server.url("/page.html"), 4);
    lines.sort();
    assert_eq!(
        lines,
        [
            "[frame about:srcdoc] SecurityError",
            "[frame about:srcdoc] read",
            "[frame about:srcdoc] true true boxed 0 null",
            "[frame about:srcdoc] true true first 0 [object HTMLIFrameElement]",
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
    // the sibling of `a`'s (the reference browser's rule, p6). The
    // navigator acts once its sibling's document answers it: until that
    // loads, the frame holds its first, of the top's origin, and is
    // refused (a slow machine's 50 ms were not enough).
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
            "<script>\
             let ready = false;\
             onmessage = e => {\
               if (ready || e.origin !== location.origin) return;\
               ready = true;\
               try { parent.frames.a.location = 'y.html' } \
               catch (e) { console.log(e.name, e.message.replace(/'http.*'/, 'URL')) }\
               parent.frames.b.location.href = 'y.html';\
             };\
             const ping = () => {\
               if (ready) return;\
               parent.frames.b.postMessage('here?', '*');\
               setTimeout(ping, 10);\
             };\
             onload = ping</script>",
        ),
        (
            "/x.html",
            "<p>x</p><script>onmessage = e => e.source.postMessage('here', '*')</script>",
        ),
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
fn a_frame_reads_another_frames_document_only_of_its_own_origin() {
    // HTML lets frames of one origin read each other's documents (G90),
    // and no others. A top of `a` reads its own origin's frame, not one
    // of `sub.a` (its site's, another origin) nor a sandboxed one of its
    // URL's origin (opaque, no `allow-same-origin`); a reader of `b` its
    // sibling and its child of `b`, not its parent and top of `a`; that
    // child its parent of `b`, not its top. As the reference browser.
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<iframe name=same src=x.html></iframe>\
             <iframe src='http://b.localhost:PORT/reader.html'></iframe>\
             <iframe name=b src='http://b.localhost:PORT/x.html'></iframe>\
             <iframe name=sub src='http://sub.a.localhost:PORT/x.html'></iframe>\
             <iframe name=boxed sandbox=allow-scripts src=x.html></iframe><script>\
             for (const f of document.querySelectorAll('iframe')) {\
               f.src = f.src.replace('PORT', location.port);\
             }\
             const read = w => { try { return w.document ? 'read' : 'none' } \
               catch (e) { return e.name } };\
             const doc = name => document.querySelector(`iframe[name=${name}]`).contentDocument;\
             onload = () => console.log('top: same', read(frames.same), doc('same'), \
               'sub', read(frames.sub), doc('sub'), 'boxed', read(frames.boxed), doc('boxed'));\
             </script>",
        ),
        (
            "/reader.html",
            "<iframe name=child src=grandchild.html></iframe><script>\
             const read = w => { try { return w.document ? 'read' : 'none' } \
               catch (e) { return e.name } };\
             const heard = new Set();\
             onmessage = e => {\
               if (e.origin !== location.origin || heard.size === 2) return;\
               heard.add(e.source);\
               if (heard.size < 2) return;\
               console.log('reader: sibling', read(parent.frames.b), 'parent', read(parent), \
                 'top', read(top), 'child', read(frames.child));\
             };\
             const ping = () => {\
               if (heard.size === 2) return;\
               parent.frames.b.postMessage('here?', '*');\
               frames.child.postMessage('here?', '*');\
               setTimeout(ping, 10);\
             };\
             onload = ping</script>",
        ),
        (
            "/grandchild.html",
            "<script>\
             const read = w => { try { return w.document ? 'read' : 'none' } \
               catch (e) { return e.name } };\
             let told = false;\
             onmessage = e => {\
               e.source.postMessage('here', '*');\
               if (told) return;\
               told = true;\
               console.log('grandchild: parent', read(parent), 'top', read(top));\
             };</script>",
        ),
        (
            "/x.html",
            "<p>x</p><script>onmessage = e => e.source.postMessage('here', '*')</script>",
        ),
    ];
    let server = sites(PAGES);
    let mut lines = run(&server, at(&server, "a.localhost", "/page.html"), 3);
    lines.sort();
    assert_eq!(
        lines,
        [
            "[frame http://b.localhost:PORT/grandchild.html] grandchild: parent read top \
             SecurityError",
            "[frame http://b.localhost:PORT/reader.html] reader: sibling read parent \
             SecurityError top SecurityError child read",
            "top: same read [object HTMLDocument] sub SecurityError null boxed SecurityError \
             null",
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
             const reach = () => { try { return typeof w.document; } catch (e) { return e.name; } };\
             setTimeout(() => console.log(frames[0] === w, f.contentWindow === w, reach()), 50);\
             </script>",
    )];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 2);
    // Its `about:blank`, which runs no script, has its realm made as it
    // is first reached (G90 phase 3 (b)). As the reference browser.
    assert_eq!(lines, ["true 1 true", "true true object"]);
}

#[test]
fn a_shadow_trees_frames_are_not_the_windows() {
    // HTML's document-tree child navigables (D68): a frame whose iframe
    // is in a shadow tree has its window, but its parent window has it
    // neither by index nor by name.
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<iframe name=main></iframe><div id=host></div>             <script>             const root = host.attachShadow({mode: 'closed'});             root.innerHTML = '<iframe name=inner></iframe>';             const w = root.firstChild.contentWindow;             onload = () => console.log(length, frames.main === frames[0], frames.inner,                w !== null, w.parent === window);             </script>",
    )];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 1);
    assert_eq!(lines, ["1 true undefined true true"]);
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

/// G90 phase 2: a frame of its creator's origin is reached as itself,
/// its own built-ins (the anti-bot "fresh frame" trick, Kasada's on
/// Twitch): at the top, and in a frame of another origin, which makes
/// its own; as the reference browser does.
#[test]
fn a_frame_of_its_creators_origin_is_reached_as_itself() {
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<iframe src='http://b.localhost:PORT/child.html'></iframe>\
             <script>\
             const f = document.querySelector('iframe');\
             f.src = f.src.replace('PORT', location.port);\
             const blank = document.createElement('iframe');\
             document.body.appendChild(blank);\
             onload = () => {\
               const w = blank.contentWindow;\
               console.log('top', typeof w.TextEncoder, w.TextEncoder === TextEncoder,\
                 Function.prototype.toString.call(w.TextEncoder).includes('[native code]'),\
                 w.Array !== Array, w.parent === window);\
             };\
             </script>",
        ),
        (
            "/child.html",
            "<script>\
             const blank = document.createElement('iframe');\
             document.documentElement.appendChild(blank);\
             onload = () => {\
               try {\
                 const w = blank.contentWindow;\
                 console.log('child', typeof w.TextEncoder, w.TextEncoder === TextEncoder);\
               } catch (e) { console.log('child threw', e.name); }\
             };\
             </script>",
        ),
    ];
    let server = sites(PAGES);
    let mut lines = run(&server, at(&server, "a.localhost", "/page.html"), 2);
    lines.sort();
    // The `about:blank` runs no script: its realm is made as the run
    // first reaches it (G90 phase 3 (b)), and its `TextEncoder` is its
    // own, native. As the reference browser.
    assert_eq!(
        lines,
        [
            "[frame http://b.localhost:PORT/child.html] child function false",
            "top function false true true true",
        ]
    );
}

#[test]
fn a_frame_navigates_its_parent_of_its_own_origin_by_its_location() {
    // A grandchild sets its parent's `location.href` (G90: the parent's
    // own `Location`, its frame of the grandchild's origin): the parent
    // navigates, and the document it shows tells the top. As the
    // reference browser (WPT iframe_sandbox_navigate_ancestor-2).
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<script>onmessage = e => console.log('top heard', e.data)</script>\
             <iframe src=child.html></iframe>",
        ),
        (
            "/child.html",
            "<p>child</p><iframe src=grandchild.html></iframe>",
        ),
        (
            "/grandchild.html",
            "<script>onload = () => {\
               try { parent.location.href = 'data:text/html,<script>parent.postMessage(1, \"*\")</scr' + 'ipt>'; }\
               catch (e) { console.log('threw', e.name); }\
             };</script>",
        ),
    ];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 1);
    assert_eq!(lines, ["top heard 1"]);
}

#[test]
fn a_frame_reaches_its_scriptless_parent() {
    // A parent that runs no script has its realm made as its frame of
    // its origin first reaches it (G90 phase 3 (b)), which reads its
    // document; a sandboxed frame is refused. As the reference browser.
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<title>T</title>\
         <iframe srcdoc=\"<script>\
           try { console.log('same', parent.document.title); } \
           catch (e) { console.log('same', e.name); }</script>\"></iframe>\
         <iframe sandbox=allow-scripts srcdoc=\"<script>\
           try { parent.document; } catch (e) { console.log('boxed', e.name); }</script>\">\
         </iframe>",
    )];
    let server = sites(PAGES);
    let mut lines = run(&server, server.url("/page.html"), 2);
    lines.sort();
    assert_eq!(
        lines,
        [
            "[frame about:srcdoc] boxed SecurityError",
            "[frame about:srcdoc] same T",
        ]
    );
}

#[test]
fn a_frame_of_another_origin_makes_its_scriptless_parent_no_realm() {
    // The twin of the one above: frames of `b` and of `sub.a` (the
    // parent's site, another origin) under a parent of `a` that runs no
    // script reach neither its document nor their own element, and make
    // it no realm (G90). As the reference browser.
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<title>T</title>\
             <iframe src='http://b.localhost:PORT/child.html'></iframe>\
             <iframe src='http://sub.a.localhost:PORT/child.html'></iframe>",
        ),
        (
            "/child.html",
            "<script>\
             let read; try { read = parent.document.title; } catch (e) { read = e.name; }\
             console.log(location.hostname, read, String(frameElement));</script>",
        ),
    ];
    // The page runs no script, so its frames' URLs carry the children's
    // server's port as served, not written in by a script.
    let server = sites(PAGES);
    let port = server.url("/").port().expect("a port");
    let page = PAGES[0].1.replace("PORT", &port.to_string());
    let server_page = serve(move |path| (path == "/page.html").then(|| html(&page)));
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine
        .navigate(at(&server_page, "a.localhost", "/page.html"));
    let mut lines = tab
        .run_until(|lines| lines.len() >= 2)
        .into_iter()
        .map(|line| line.replace(&port.to_string(), "PORT"))
        .collect::<Vec<_>>();
    lines.sort();
    assert_eq!(
        lines,
        [
            "[frame http://b.localhost:PORT/child.html] b.localhost SecurityError null",
            "[frame http://sub.a.localhost:PORT/child.html] sub.a.localhost SecurityError null",
        ]
    );
    assert!(
        !tab.engine.shown.scripts.has_realm(),
        "the parent made no realm"
    );
}

#[test]
fn a_frames_document_on_its_way_is_reached_as_itself() {
    // A frame of the page's origin parses its head, a stylesheet holding
    // it back from being shown, and tells its parent; the parent reads
    // the frame's document in a task of its own meanwhile (G90: the
    // document on its way is the frame's, and lent as the shown one is).
    // As the reference browser.
    let server = serve(|path| match path {
        "/page.html" => Some(html(
            "<iframe src=child.html></iframe><script>\
             onmessage = () => console.log('parent reads', frames[0].document.title, \
               frames[0].document.readyState);</script>",
        )),
        "/child.html" => Some(html(
            "<title>child</title><script>parent.postMessage('on its way', '*')</script>\
             <link rel=stylesheet href=slow.css><body>body</body>",
        )),
        "/slow.css" => {
            std::thread::sleep(std::time::Duration::from_millis(400));
            Some(css("p { color: green }"))
        }
        _ => None,
    });
    let port = server.url("/").port().expect("a port").to_string();
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    let lines: Vec<String> = tab
        .run_until(|lines| !lines.is_empty())
        .into_iter()
        .map(|line| line.replace(&port, "PORT"))
        .collect();
    assert_eq!(lines, ["parent reads child loading"]);
    // The frame was still on its way when its parent read it.
    assert!(
        tab.engine
            .shown
            .frames
            .values()
            .all(|frame| frame.engine.navigation.is_some())
    );
}

#[test]
fn a_frame_of_its_parents_origin_that_runs_no_script_is_reached() {
    // A document that runs no script, `about:blank` among them, has its
    // realm made as its parent of its origin first reaches it (G90 phase
    // 3 (b)). As the reference browser.
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<iframe src=quiet.html></iframe><iframe></iframe><script>\
             const read = w => { try { return w.document ? 'read' : 'none' } \
               catch (e) { return e.name } };\
             onload = () => console.log('quiet', read(frames[0]), 'blank', read(frames[1]));\
             </script>",
        ),
        ("/quiet.html", "<p>no script here</p>"),
    ];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 1);
    assert_eq!(lines, ["quiet read blank read"]);
}

#[test]
fn a_reached_blank_keeps_its_creators_policy_and_tells_its_violations() {
    // G90 phase 3 (b): a parent under a `script-src` without
    // 'unsafe-eval' writes a script into its `about:blank` frame, whose
    // realm is made as it is reached: the blank's policy is its creator's,
    // so `eval` throws `EvalError`, and the violation's event reaches the
    // listeners the written script added (written without `open()`, its
    // implicit open erases before the written scripts run). As the
    // reference browser (WPT eval-blocked-in-about-blank-iframe).
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<meta http-equiv=Content-Security-Policy content=\"script-src 'self' 'unsafe-inline'\">\
         <iframe src=about:blank></iframe><script>\
         addEventListener('message', e => console.log('parent heard', JSON.stringify(e.data)));\
         onload = () => { const d = frames[0].document;\
           d.write('<script>addEventListener(`securitypolicyviolation`, e => \
             parent.postMessage({ directive: e.violatedDirective }, `*`));\
             try { eval(`1`); parent.postMessage(`FAIL`, `*`); }\
             catch (e) { parent.postMessage(e instanceof EvalError ? `PASS` : e.name, `*`); }</scr' + 'ipt>');\
           d.close(); };\
         </script>",
    )];
    let server = sites(PAGES);
    let mut lines = run(&server, server.url("/page.html"), 3);
    lines.sort();
    assert_eq!(
        lines,
        [
            "[frame about:blank] Content-Security-Policy refused a string as code: \
             script-src 'self' 'unsafe-inline'",
            "parent heard \"PASS\"",
            "parent heard {\"directive\":\"script-src\"}",
        ]
    );
}

#[test]
fn a_documents_implicit_open_erases_before_the_scripts_it_writes_run() {
    // `document.write()` after the load opens the document again, which
    // erases the window's listeners (HTML 8.4.2.1): before the scripts it
    // writes run, so that theirs stay. As the reference browser.
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<script>addEventListener('x', () => console.log('old'));\
         onload = () => setTimeout(() => { document.write('<script>\
           addEventListener(`x`, () => console.log(`new`));\
           dispatchEvent(new Event(`x`));</scr' + 'ipt>'); document.close(); }, 0);\
         </script>",
    )];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 1);
    assert_eq!(lines, ["new"]);
}

#[test]
fn a_realm_made_as_it_is_reached_gives_every_output_of_that_run() {
    // G90 phase 3 (b): two scriptless frames of the parent's origin have
    // their realms made as the parent's run reaches them, and in that same
    // run they log, set a timer, fetch, post to the parent, write storage
    // (which the parent hears), refuse code by their policy (their
    // listener told), and navigate: each output arrives, as it does from
    // a realm made by a script of its own.
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<iframe src=quiet.html></iframe><iframe src=quiet2.html></iframe><script>\
             addEventListener('message', e => console.log('parent heard', e.data, e.source === window));\
             addEventListener('storage', e => console.log('parent storage', e.key, e.newValue));\
             onload = () => {\
               const w = frames[0];\
               w.console.log('console');\
               w.setTimeout(() => w.console.log('timer'), 0);\
               w.fetch('/data.txt').then(r => r.text()).then(t => w.console.log('fetched', t));\
               w.addEventListener('securitypolicyviolation', () => w.console.log('violation'));\
               try { w.eval('1'); } catch (e) { w.console.log('refused', e.name); }\
               w.parent.postMessage('posted', '*');\
               w.localStorage.setItem('k', 'v');\
               frames[1].location.href = '/next.html';\
             };\
             </script>",
        ),
        (
            "/quiet.html",
            "<meta http-equiv=Content-Security-Policy content=\"script-src 'self' 'unsafe-inline'\"><p>quiet",
        ),
        ("/quiet2.html", "<p>quiet too"),
        ("/next.html", "<script>console.log('arrived')</script>"),
        ("/data.txt", "hello"),
    ];
    let server = sites(PAGES);
    let mut lines = run(&server, server.url("/page.html"), 9);
    lines.sort();
    // The post is the parent's own (its source the caller's window).
    assert_eq!(
        lines,
        [
            "[frame http://127.0.0.1:PORT/next.html] arrived",
            "[frame http://127.0.0.1:PORT/quiet.html] Content-Security-Policy refused a string as code: script-src 'self' 'unsafe-inline'",
            "[frame http://127.0.0.1:PORT/quiet.html] console",
            "[frame http://127.0.0.1:PORT/quiet.html] fetched hello",
            "[frame http://127.0.0.1:PORT/quiet.html] refused EvalError",
            "[frame http://127.0.0.1:PORT/quiet.html] timer",
            "[frame http://127.0.0.1:PORT/quiet.html] violation",
            "parent heard posted true",
            "parent storage k v",
        ]
    );
}

/// One battery of what a realm does, run in a frame's realm made by a
/// script of its own and in one made as its parent reaches it (G90 phase
/// 3 (b)): engine events at the window, the document and an element, a
/// listener and a synthetic event, a mutation observer, a promise job, a
/// custom element's upgrade, a node's document and root.
const BATTERY: &str = "(() => { const log = (...a) => console.log(a.join(' ')); \
    addEventListener('hashchange', () => log('window hashchange')); \
    document.addEventListener('selectionchange', () => log('document selectionchange'), { once: true }); \
    const img = document.createElement('img'); \
    img.addEventListener('error', () => log('element error')); \
    img.src = '/missing.png'; document.body.append(img); \
    const p = document.createElement('p'); \
    p.addEventListener('ping', e => log('synthetic', e.type, e.isTrusted)); \
    p.dispatchEvent(new Event('ping')); \
    new MutationObserver(r => log('mutations', r.length, r[0].type)).observe(document.body, { childList: true }); \
    document.body.append(p); \
    Promise.resolve().then(() => log('promise job')); \
    const later = document.createElement('x-later'); document.body.append(later); \
    customElements.define('x-later', class extends HTMLElement { connectedCallback() { log('upgraded', this.isConnected); } }); \
    log('owner', p.ownerDocument === document, p.getRootNode() === document); \
    getSelection().selectAllChildren(document.body); \
    location.hash = '#moved'; \
    log('ran'); })()";

/// The lines a frame's realm gave, its frame's name taken off.
fn frame_lines(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .filter_map(|line| {
            line.strip_prefix("[frame ")
                .and_then(|rest| rest.split_once("] "))
                .map(|(_, text)| text.to_owned())
        })
        .collect()
}

#[test]
fn a_realm_made_as_it_is_reached_behaves_as_one_made_by_its_own_script() {
    let eager = format!(
        "<iframe srcdoc=\"<body><script>{}</script>\"></iframe>",
        BATTERY.replace('"', "&quot;")
    );
    let reached = format!(
        "<iframe src=quiet.html></iframe><script>onload = () => frames[0].eval({});</script>",
        serde_json_string(BATTERY)
    );
    let pages: &'static [(&'static str, &'static str)] = Box::leak(Box::new([
        ("/eager.html", &*Box::leak(eager.into_boxed_str())),
        ("/reached.html", &*Box::leak(reached.into_boxed_str())),
        ("/quiet.html", "<body>"),
    ]));
    let server = sites(pages);
    let count = 9;
    let eager = frame_lines(run(&server, server.url("/eager.html"), count));
    let reached = frame_lines(run(&server, server.url("/reached.html"), count));
    for (n, (a, b)) in eager.iter().zip(&reached).enumerate() {
        assert_eq!(
            a, b,
            "line {n} differs: eager {eager:?}, reached {reached:?}"
        );
    }
    assert_eq!(
        eager.len(),
        reached.len(),
        "eager {eager:?}, reached {reached:?}"
    );
}

/// `text` as a JavaScript string literal.
fn serde_json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
