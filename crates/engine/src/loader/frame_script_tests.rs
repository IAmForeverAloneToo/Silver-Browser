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

/// The `iframe` a script inserts and reaches in the same task, before its
/// parent's engine makes its frame: its initial `about:blank`, made then
/// with its realm (G90 phase 3 (b), `blank_frame`), is the frame's.
#[test]
fn a_same_task_reach_of_an_inserted_iframe_gives_its_blank() {
    // The way of anti-bot scripts (Kasada's reads `TextEncoder` of a
    // child it makes) and of `iframe-nosrc`. As the reference browser.
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<script>\
         const f = document.createElement('iframe');\
         document.documentElement.appendChild(f);\
         const w = f.contentWindow;\
         console.log('A', typeof w.document, w.document.readyState, w.location.href,\
           w.document.URL, String(w.document.body), new w.TextEncoder().encode('ab').length,\
           w.document === f.contentDocument, w.origin === origin, w.document.characterSet,\
           w.innerWidth, w.innerHeight);\
         w.document.body.innerHTML = '<p id=p style=\"height:20px;margin:0\">made</p>';\
         const made = w.document.getElementById('p');\
         setTimeout(() => console.log('A later', frames[0] === w, f.contentWindow === w,\
           f.contentDocument.getElementById('p').textContent,\
           f.contentDocument.getElementById('p').offsetHeight,\
           made === f.contentDocument.getElementById('p'), w.document.readyState), 50);\
         </script>",
    )];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 2);
    assert_eq!(
        lines,
        [
            "A object complete about:blank about:blank [object HTMLBodyElement] 2 true true \
             UTF-8 300 150",
            "A later true true made 20 true complete",
        ]
    );
}

#[test]
fn a_same_task_reach_of_a_sandboxed_insert_is_refused_unless_of_its_origin() {
    // A sandbox without `allow-same-origin` makes the blank's origin
    // opaque: another agent's, reached as its window. As the reference
    // browser.
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<script>\
         const b = document.createElement('iframe');\
         b.sandbox = 'allow-scripts';\
         document.documentElement.appendChild(b);\
         console.log('B', b.contentDocument === null,\
           (() => { try { return typeof b.contentWindow.document; } catch (e) { return e.name; } })(),\
           b.contentWindow !== null);\
         const s = document.createElement('iframe');\
         s.sandbox = 'allow-same-origin';\
         document.documentElement.appendChild(s);\
         console.log('B2', s.contentDocument !== null,\
           (() => { try { return s.contentWindow.document.URL; } catch (e) { return e.name; } })());\
         </script>",
    )];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 2);
    assert_eq!(lines, ["B true SecurityError true", "B2 true about:blank"]);
}

#[test]
fn a_hundred_inserts_reached_in_one_run_are_each_their_blank() {
    // Each blank's realm made in the run, the agent's heap fitted to them
    // as they come: every read succeeds, and the agent lives on. As the
    // reference browser.
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<script>\
         let ok = 0;\
         for (let i = 0; i < 100; i++) {\
           const g = document.createElement('iframe');\
           g.style.display = 'none';\
           document.documentElement.appendChild(g);\
           try { if (g.contentWindow.document.body) ok++; }\
           catch (e) { console.log(e.name, e.message); }\
         }\
         console.log('C', ok);\
         setTimeout(() => {\
           let later = 0;\
           for (let i = 0; i < frames.length; i++) {\
             try { if (frames[i].document.body) later++; } catch (e) { console.log(e.name); }\
           }\
           console.log('C later', frames.length, later, frames[99].eval('1 + 1'));\
         }, 50);\
         </script>",
    )];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 2);
    assert_eq!(lines, ["C 100", "C later 100 100 2"]);
}

#[test]
fn a_reached_blank_inserts_and_reaches_its_own_iframe() {
    // The blank's run is its parent's: an `iframe` inserted in the blank
    // is reached as its own blank too, and is the blank's frame once
    // both are made. As the reference browser.
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<script>\
         const f = document.createElement('iframe');\
         document.documentElement.appendChild(f);\
         const w = f.contentWindow;\
         const inner = w.document.createElement('iframe');\
         w.document.body.appendChild(inner);\
         console.log('E', typeof inner.contentWindow.document, inner.contentWindow.parent === w,\
           w.length);\
         setTimeout(() => console.log('E later', w.length, w.frames[0] === inner.contentWindow,\
           typeof w.frames[0].document), 50);\
         </script>",
    )];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 2);
    assert_eq!(lines, ["E object true 1", "E later 1 true object"]);
}

#[test]
fn a_same_task_reach_keeps_its_creators_policy() {
    // The blank's policy is its creator's from its first reach: code from
    // strings throws `EvalError` and is told to the blank's listeners, an
    // inline script its policy allows runs. As the reference browser.
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<meta http-equiv=Content-Security-Policy content=\"script-src 'self' 'unsafe-inline'\">\
         <script>\
         const f = document.createElement('iframe');\
         document.documentElement.appendChild(f);\
         const w = f.contentWindow;\
         w.addEventListener('securitypolicyviolation',\
           e => console.log('frame heard', e.violatedDirective));\
         try { w.eval('1'); console.log('FAIL'); } catch (e) { console.log('eval', e.name); }\
         try { new w.Function('return 1'); console.log('FAIL'); }\
         catch (e) { console.log('Function', e.name); }\
         const s = w.document.createElement('script');\
         s.textContent = 'parent.console.log(`inline ran`)';\
         w.document.documentElement.appendChild(s);\
         console.log('after inline');\
         </script>",
    )];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 8);
    // The run's lines in order; the frame's console and the events queued
    // for it after, in an order of their own.
    assert_eq!(
        lines[..4],
        [
            "eval EvalError",
            "Function EvalError",
            "inline ran",
            "after inline"
        ]
    );
    let mut later = lines[4..].to_vec();
    later.sort();
    assert_eq!(
        later,
        [
            "[frame about:blank] Content-Security-Policy refused a string as code: \
             script-src 'self' 'unsafe-inline'",
            "[frame about:blank] Content-Security-Policy refused a string as code: \
             script-src 'self' 'unsafe-inline'",
            "frame heard script-src",
            "frame heard script-src",
        ]
    );
}

#[test]
fn a_same_task_reach_of_an_insert_with_a_source_gives_the_blank_then_what_came() {
    // A `src` or `srcdoc` is navigated to after the run: the run reaches
    // the initial blank, a later task what came through the same window
    // (the blank's global, which follows its frame). Another origin's
    // document, of another site or of a subdomain of the page's, is
    // refused then, a property set on the blank before too. As the
    // reference browser.
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<script>\
             const read = w => { try { return w.document.URL + ' ' + w.document.body.innerHTML; }\
               catch (e) { return e.name; } };\
             const kept = w => { try { return String(w.kept); } catch (e) { return e.name; } };\
             const make = (attr, value) => {\
               const f = document.createElement('iframe');\
               f[attr] = value;\
               document.documentElement.appendChild(f);\
               const w = f.contentWindow;\
               w.kept = 1;\
               console.log(attr, 'now', read(w));\
               return [f, w];\
             };\
             const port = location.port;\
             const all = [make('srcdoc', '<p>s</p>'),\
               make('src', 'http://b.localhost:' + port + '/child.html'),\
               make('src', 'http://sub.a.localhost:' + port + '/child.html')];\
             onload = () => all.forEach(([f, w]) =>\
               console.log('later', read(w), w === f.contentWindow, kept(w)));\
             </script>",
        ),
        ("/child.html", "<p>child</p>"),
    ];
    let server = sites(PAGES);
    let lines = run(&server, at(&server, "a.localhost", "/page.html"), 6);
    assert_eq!(
        lines,
        [
            "srcdoc now about:blank ",
            "src now about:blank ",
            "src now about:blank ",
            "later about:srcdoc <p>s</p> true undefined",
            "later SecurityError true SecurityError",
            "later SecurityError true SecurityError",
        ]
    );
}

#[test]
fn a_blank_reached_and_removed_in_one_run_leaves_no_realm() {
    // Its realm made as it was reached goes with its element, before any
    // frame was made of it: it would keep its agent. Its window kept
    // answers as a removed frame's from the next task, as the reference
    // browser's; in the run that removed it, as open still (a known
    // deviation: the reference browser's is closed at once).
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<script>\
         const r = document.createElement('iframe');\
         document.documentElement.appendChild(r);\
         const rw = r.contentWindow;\
         rw.document.body.textContent = 'gone';\
         r.remove();\
         console.log('D', rw.closed);\
         setTimeout(() => console.log('D later', rw.closed, rw.parent === null, rw.length), 50);\
         </script>",
    )];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    let lines = tab.run_until(|lines| lines.len() >= 2);
    assert_eq!(lines, ["D false", "D later true true 0"]);
    let agent = tab.engine.shown.scripts.agent().expect("its agent").clone();
    assert_eq!(agent.realm_count(), 1, "the page's realm alone");
    assert!(!agent.has_made(1));
}

/// A window a script holds is its frame's across the frame's navigations
/// (G90 phase 3, HTML's WindowProxy): the global its first document's
/// realm had answers, once that document is left, as the frame's window
/// now, of its origin or another's.
#[test]
fn a_held_window_follows_its_frame_and_stays_its_message_source() {
    // The parent holds the window of its frame, which loads a page of its
    // origin, then another site's, then its own again; the frame's
    // messages come from that same window each time. As the reference
    // browser.
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<iframe src=one.html></iframe><script>\
             const f = document.querySelector('iframe');\
             let w = null, step = 0;\
             const read = () => { try { return w.document.title; } catch (e) { return e.name; } };\
             addEventListener('message', e => console.log('message', e.data,\
               e.source === w, e.source === f.contentWindow));\
             f.onload = () => {\
               step++;\
               if (w === null) w = f.contentWindow;\
               console.log('load', step, read(), w === f.contentWindow, w === frames[0]);\
               if (step === 1) w.location.href = 'two.html';\
               else if (step === 2) f.src = 'http://b.localhost:' + location.port + '/other.html';\
               else if (step === 3) f.src = 'three.html';\
             };\
             </script>",
        ),
        (
            "/one.html",
            "<title>one</title><script>parent.postMessage('one', '*')</script>",
        ),
        (
            "/two.html",
            "<title>two</title><script>parent.postMessage('two', '*')</script>",
        ),
        (
            "/other.html",
            "<title>other</title><script>parent.postMessage('other', '*')</script>",
        ),
        (
            "/three.html",
            "<title>three</title><script>parent.postMessage('three', '*')</script>",
        ),
    ];
    let server = sites(PAGES);
    let lines = run(&server, at(&server, "a.localhost", "/page.html"), 8);
    assert_eq!(
        lines,
        [
            "message one false true",
            "load 1 one true true",
            "message two true true",
            "load 2 two true true",
            "message other true true",
            "load 3 SecurityError true true",
            "message three true true",
            "load 4 three true true",
        ]
    );
}

#[test]
fn a_left_documents_realm_runs_nothing_and_its_functions_read_the_frames_document() {
    // Once its frame went on, nothing of the old document runs: its
    // timers and its listeners (they write to the parent at once, so that
    // nothing queued before the navigation is counted). A function of it
    // a script kept still runs when called, as in the reference browser,
    // its own globals found; its `document` is the frame's now (a known
    // deviation: the reference browser's reads the old document).
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<iframe src=old.html></iframe><script>\
             var ticks = 0, oldRan = [];\
             const f = document.querySelector('iframe');\
             let w = null, readTitle = null, before = 0;\
             addEventListener('message', e => console.log('message', e.data));\
             f.onload = () => {\
               if (w === null) { w = f.contentWindow; readTitle = w.readTitle;\
                 setTimeout(() => { w.location.href = 'new.html'; }, 20); return; }\
               before = ticks;\
               console.log('kept function reads', readTitle());\
               w.addEventListener('message', e => console.log('listener added through it', e.data));\
               w.postMessage('hello', '*');\
               setTimeout(() => console.log('old ran', ticks > 0, ticks - before, oldRan.join()), 60);\
             };\
             </script>",
        ),
        (
            "/old.html",
            "<title>old</title><body><script>\
             setInterval(() => parent.ticks++, 2);\
             addEventListener('message', () => parent.oldRan.push('heard'));\
             function readTitle() { return document.title; }\
             </script>",
        ),
        (
            "/new.html",
            "<title>new</title><body><script>\
             addEventListener('message', e => parent.postMessage('new heard ' + e.data, '*'));\
             </script>",
        ),
    ];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 4);
    assert_eq!(
        lines,
        [
            "kept function reads new",
            "listener added through it hello",
            "message new heard hello",
            "old ran true 0 ",
        ]
    );
}

#[test]
fn a_held_window_of_a_frame_gone_to_another_origin_answers_as_its_window() {
    // What a held window answers once its frame shows another origin's
    // document: what that origin's window answers (the cross-origin
    // window's allowlist, `SecurityError` for the rest), as the reference
    // browser. Known deviations, the held object being the old global:
    // its own properties stay its own. The old page's own `var` and
    // `function` globals, of the holder's origin, read as they were (the
    // reference browser: `SecurityError`); the language's globals
    // (`JSON`...) stay, so that its functions still work; its own keys
    // are those, not the allowlist; its prototype is the forwarder, not
    // null; `location` has no own descriptor. `instanceof Window` is
    // false, as there.
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<iframe src=mine.html></iframe><script>\
             const f = document.querySelector('iframe');\
             let w = null;\
             const t = g => { try { return String(g()); } catch (e) { return e.name; } };\
             f.onload = () => {\
               if (w === null) { w = f.contentWindow;\
                 f.src = 'http://b.localhost:' + location.port + '/other.html'; return; }\
               console.log(t(() => 'location' in w), t(() => 'postMessage' in w),\
                 t(() => 'alert' in w), t(() => w.mine), t(() => 'mine' in w),\
                 t(() => w.document), t(() => w.alert), t(() => w.closed),\
                 t(() => w.length), t(() => w.parent === window), t(() => w.top === window));\
               const names = Object.getOwnPropertyNames(w);\
               console.log(['mine', 'JSON', 'document', 'alert', 'location', 'postMessage']\
                   .map(n => n + ':' + names.includes(n)).join(' '),\
                 t(() => Object.getPrototypeOf(w) === null), t(() => w instanceof Window),\
                 t(() => Object.getOwnPropertyDescriptor(w, 'location')));\
             };\
             </script>",
        ),
        ("/mine.html", "<script>var mine = 'own data';</script>"),
        ("/other.html", "<p>other</p>"),
    ];
    let server = sites(PAGES);
    let lines = run(&server, at(&server, "a.localhost", "/page.html"), 2);
    assert_eq!(
        lines,
        [
            "true true SecurityError own data true SecurityError SecurityError false 0 true true",
            "mine:true JSON:true document:true alert:false location:false postMessage:false \
             false false undefined",
        ]
    );
}

#[test]
fn a_held_window_keeps_one_old_realm_however_often_its_frame_navigates() {
    // Ad and refresh frames reload again and again while their parent
    // holds their window: the first global, held, keeps its realm's
    // context and what it could not let go; each realm after goes.
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<iframe src=child.html></iframe><script>\
             const f = document.querySelector('iframe');\
             let w = null, n = 0;\
             f.onload = () => {\
               if (w === null) w = f.contentWindow;\
               n++;\
               if (n === 10 || n === 50) console.log('loads', n, w.document.URL.endsWith(n - 1));\
               if (n < 50) w.location.href = 'child.html?' + n;\
             };\
             </script>",
        ),
        (
            "/child.html",
            "<script>var big = new Array(200000).fill(1).map((x, i) => ({ i }));</script>",
        ),
    ];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    let lines = tab.run_until(|lines| !lines.is_empty());
    assert_eq!(lines, ["loads 10 true"]);
    tab.engine.shown.scripts.collect();
    let agent = tab.engine.shown.scripts.agent().expect("its agent").clone();
    let (heap_ten, realms_ten) = (tab.engine.shown.scripts.heap(), agent.realm_count());
    let lines = tab.run_until(|lines| !lines.is_empty());
    assert_eq!(lines, ["loads 50 true"]);
    tab.engine.shown.scripts.collect();
    let (heap_fifty, realms_fifty) = (tab.engine.shown.scripts.heap(), agent.realm_count());
    assert_eq!(
        (realms_ten, realms_fifty),
        (2, 2),
        "the page's and the frame's"
    );
    assert!(
        heap_fifty < heap_ten + (4 << 20),
        "the heap grew from {heap_ten} to {heap_fifty} over 40 navigations"
    );
}

#[test]
fn a_held_window_finds_its_frames_reloaded_document() {
    // A reload leaves its old document too: the parent's window of the
    // frame posts to the reloaded one, and what the old one posted to
    // its own frame as it went is not delivered (WPT
    // postMessage-to-target-not-fully-active-on-reload). As the
    // reference browser.
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<body><script>\
             window.ran = false;\
             const s = document.createElement('iframe');\
             s.src = 'mid.html';\
             let n = 0;\
             s.addEventListener('load', () => {\
               n++;\
               if (n === 1) s.contentWindow.location.reload();\
               else s.contentWindow.postMessage('reloaded', '*');\
             });\
             addEventListener('message', e => console.log('heard', e.data, window.ran));\
             document.body.appendChild(s);\
             </script>",
        ),
        (
            "/mid.html",
            "<body><script>\
             addEventListener('unload', () => { if (window[0]) window[0].postMessage('foo', '*'); });\
             addEventListener('message', () => parent.postMessage('done', '*'));\
             </script><iframe src=leaf.html></iframe>",
        ),
        (
            "/leaf.html",
            "<script>addEventListener('message', () => { parent.parent.ran = true; });</script>",
        ),
    ];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 1);
    assert_eq!(lines, ["heard done false"]);
}

#[test]
fn an_iframes_src_set_to_its_own_value_navigates_it_again() {
    // HTML's "process the iframe attributes" runs as `src` is set, to
    // the value it had too: the frame loads again, and its element
    // hears `load` again (WPT webstorage event_body_attribute). As the
    // reference browser.
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<body><script>\
             const f = document.createElement('iframe');\
             document.body.appendChild(f);\
             let loads = 0;\
             f.onload = () => {\
               loads++;\
               console.log('load', loads, f.contentWindow.document.body.textContent);\
               if (loads < 3) f.src = 'child.html';\
             };\
             f.src = 'child.html';\
             </script>",
        ),
        ("/child.html", "<body>child"),
    ];
    let server = sites(PAGES);
    let lines = run(&server, server.url("/page.html"), 3);
    assert_eq!(lines, ["load 1 child", "load 2 child", "load 3 child"]);
}

#[test]
fn the_sweep_counts_a_blanks_window_state_lost_to_its_frames_first_document() {
    // A script sets a property, or adds a listener, on a fresh frame's
    // window before its first document of the page's origin comes, as
    // sign-in and widget loaders do. HTML gives that document the
    // blank's `Window`, so the reference browser keeps them; this
    // version does not, and the sweep counts it. A frame left alone, or
    // whose first document is of another origin, counts nothing.
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<script>\
             const make = src => { const f = document.createElement('iframe'); f.src = src;\
               document.documentElement.appendChild(f); return f.contentWindow; };\
             make('child.html').expando = 1;\
             make('child.html').addEventListener('message', () => {});\
             make('child.html');\
             make('http://b.localhost:' + location.port + '/child.html').other = 1;\
             onload = () => console.log('loaded', frames.length);\
             </script>",
        ),
        ("/child.html", "<p>child"),
    ];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine
        .navigate(at(&server, "a.localhost", "/page.html"));
    let lines = tab.run_until(|lines| !lines.is_empty());
    assert_eq!(lines, ["loaded 4"]);
    tab.engine.inspect(r#"{"id":1,"method":"page.sweep"}"#);
    let answers = tab.engine.take_inspected();
    let answer = devtools::Json::parse(&answers[0]).expect("JSON");
    let counts = answer.get("result").expect("a result");
    assert_eq!(
        counts
            .get("blankWindowStateLost")
            .and_then(devtools::Json::as_u64),
        Some(2),
        "{answer:?}"
    );
}

#[test]
fn the_sweep_counts_a_documents_call_into_one_of_its_agent_not_lent() {
    // Every document of the agent is lent for a run since G90 phase 3,
    // so the tests withhold the frames' to reach one not lent: the call
    // into it throws, and the sweep counts it (documentsNotLent).
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<iframe srcdoc=\"<title>child</title><script>var made = 1;</script>\"></iframe><script>\
         onload = () => {\
           console.log('loaded', frames[0].document.title);\
           setTimeout(() => {\
             try { console.log('read', frames[0].document.title); }\
             catch (e) { console.log('refused', e.message); }\
           }, 100);\
         };\
         </script>",
    )];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    let lines = tab.run_until(|lines| !lines.is_empty());
    assert_eq!(lines, ["loaded child"]);
    // The frame's engine lends its document for the page's runs.
    for frame in tab.engine.shown.frames.values_mut() {
        frame.engine.withhold_lending = true;
    }
    let lines = tab.run_until(|lines| !lines.is_empty());
    assert_eq!(lines, ["refused no script is running"]);
    tab.engine.inspect(r#"{"id":1,"method":"page.sweep"}"#);
    let answers = tab.engine.take_inspected();
    let answer = devtools::Json::parse(&answers[0]).expect("JSON");
    let counts = answer.get("result").expect("a result");
    assert!(
        counts
            .get("documentsNotLent")
            .and_then(devtools::Json::as_u64)
            >= Some(1),
        "{answer:?}"
    );
}

/// The clean-realm pattern (1Password, weather.com's ads): an `iframe`
/// made, its window's functions taken, the `iframe` removed, the
/// functions kept. Its realm goes with its document; what its functions
/// do then is a detached window's (G90).
#[test]
fn a_removed_frames_functions_answer_as_a_detached_windows() {
    // `URL` parses as before, its objects its realm's; `fetch` rejects
    // with a TypeError; `setTimeout` gives an id and never fires; a call
    // that needs the document throws InvalidStateError. Nothing counts
    // as a document not lent. As the reference browser, but for the
    // last, whose detached document still answers.
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<script>\
         const f = document.createElement('iframe');\
         document.documentElement.appendChild(f);\
         const w = f.contentWindow;\
         const { URL: CleanURL, fetch: cleanFetch, setTimeout: cleanTimeout } = w;\
         const cleanCreate = w.document.createElement.bind(w.document);\
         f.remove();\
         setTimeout(() => {\
           const u = new CleanURL('/a?b=1#c', 'https://example.test/x/');\
           console.log('url', u.href, u.searchParams.get('b'),\
             Object.getPrototypeOf(u) === CleanURL.prototype, u instanceof URL);\
           const id = cleanTimeout(() => console.log('FIRED'), 1);\
           console.log('timer', typeof id, id > 0);\
           try { cleanCreate('p'); console.log('created'); }\
           catch (e) { console.log('document', e.name); }\
           cleanFetch('/page.html').then(() => console.log('fetched'),\
             e => console.log('fetch', e.constructor === w.TypeError || e instanceof TypeError || e.name));\
           setTimeout(() => console.log('done'), 50);\
         }, 10);\
         </script>",
    )];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/page.html"));
    let lines = tab.run_until(|lines| lines.iter().any(|line| line == "done"));
    assert_eq!(
        lines,
        [
            "url https://example.test/a?b=1#c 1 true false",
            "timer number true",
            "document InvalidStateError",
            "fetch true",
            "done",
        ]
    );
    tab.engine.inspect(r#"{"id":1,"method":"page.sweep"}"#);
    let answers = tab.engine.take_inspected();
    let answer = devtools::Json::parse(&answers[0]).expect("JSON");
    let counts = answer.get("result").expect("a result");
    assert_eq!(
        counts
            .get("documentsNotLent")
            .and_then(devtools::Json::as_u64),
        Some(0),
        "{answer:?}"
    );
}

/// A frame's window's size is its element's content box as its parent is
/// laid out now (the frame-size gap): read in the same task as the
/// parent made or changed the element, with no frame drawn, and so are
/// its viewport units and media queries; a grandchild's after its
/// parent's. As the reference browser (probed, Chrome 156).
#[test]
fn a_frames_size_is_its_box_as_its_parent_is_laid_out_now() {
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<body style='margin:0'><div id=host style='width:600px'></div><script>\
         const host = document.getElementById('host');\
         const f = document.createElement('iframe');\
         f.style.cssText = 'width:123px;height:45px;border:0';\
         host.appendChild(f);\
         const w = f.contentWindow;\
         console.log('made', w.innerWidth, w.innerHeight);\
         f.style.width = '50%';\
         console.log('changed', w.innerWidth, w.innerHeight);\
         const d = w.document.createElement('div');\
         d.style.cssText = 'width:100vw;height:10px';\
         w.document.body.appendChild(d);\
         f.style.width = '200px';\
         console.log('inside', d.offsetWidth, w.getComputedStyle(d).width,\
           w.matchMedia('(min-width: 199px)').matches, w.matchMedia('(min-width: 201px)').matches);\
         const hidden = document.createElement('iframe');\
         hidden.style.cssText = 'display:none;width:77px;height:33px';\
         host.appendChild(hidden);\
         console.log('hidden', hidden.contentWindow.innerWidth, hidden.contentWindow.innerHeight);\
         const outer = document.createElement('iframe');\
         outer.style.cssText = 'width:400px;height:100px;border:0';\
         host.appendChild(outer);\
         const inner = outer.contentWindow.document;\
         inner.body.style.margin = '0';\
         const g = inner.createElement('iframe');\
         g.style.cssText = 'width:50%;height:20px;border:0';\
         inner.body.appendChild(g);\
         console.log('nested', g.contentWindow.innerWidth, g.contentWindow.innerHeight);\
         outer.style.width = '300px';\
         console.log('nested later', g.contentWindow.innerWidth);\
         </script>",
    )];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_viewport(800, 600);
    tab.engine.navigate(server.url("/page.html"));
    let lines = tab.run_until(|lines| lines.len() >= 6);
    assert_eq!(
        lines,
        [
            "made 123 45",
            "changed 300 45",
            "inside 200 200px true false",
            "hidden 0 0",
            "nested 200 20",
            "nested later 150",
        ]
    );
}

/// A frame's size read after each of a thousand changes of its element
/// is right every time, at one layout of its parent a change (the
/// frame-size gap): as the reference browser's.
#[test]
fn a_frames_size_read_after_each_change_lays_its_parent_out_once_a_change() {
    static PAGES: &[(&str, &str)] = &[(
        "/page.html",
        "<body style='margin:0'><script>\
         const f = document.createElement('iframe');\
         f.style.cssText = 'height:40px;border:0';\
         document.body.appendChild(f);\
         const w = f.contentWindow;\
         let wrong = 0;\
         for (let i = 0; i < 1000; i++) {\
           const width = 100 + i % 50;\
           f.style.width = width + 'px';\
           if (w.innerWidth !== width || w.innerWidth !== width) wrong++;\
         }\
         console.log('wrong', wrong);\
         </script>",
    )];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.set_viewport(800, 600);
    tab.engine.navigate(server.url("/page.html"));
    let started = std::time::Instant::now();
    let lines = tab.run_until(|lines| !lines.is_empty());
    let spent = started.elapsed();
    assert_eq!(lines, ["wrong 0"]);
    let layouts = tab.engine.shown.scripts.frame_box_layouts();
    eprintln!("1000 changes: {layouts} layouts for the frame, {spent:?} with the load");
    assert_eq!(layouts, 1000, "one layout a change, the second read none");
}

/// A frame of another origin is never sized by a layout of its parent
/// its own scripts force (the frame-size gap): it reads the size the
/// parent's last layout gave it, though the parent changed its element
/// since, and the sweep counts that read.
#[test]
fn a_frame_of_another_origin_reads_the_size_its_parents_last_layout_gave() {
    static PAGES: &[(&str, &str)] = &[
        (
            "/page.html",
            "<body style='margin:0'><iframe style='width:200px;height:100px;border:0'></iframe>\
             <script>\
             const f = document.querySelector('iframe');\
             f.src = 'http://b.localhost:' + location.port + '/child.html';\
             onmessage = e => {\
               if (e.data === 'ready') {\
                 console.log('ready');\
                 setTimeout(() => {\
                   f.style.width = '321px';\
                   f.contentWindow.postMessage('read', '*');\
                 }, 300);\
               } else {\
                 console.log('read', e.data);\
               }\
             };\
             </script>",
        ),
        (
            "/child.html",
            "<script>\
             onmessage = () => parent.postMessage(innerWidth + ' ' + innerHeight, '*');\
             parent.postMessage('ready', '*');\
             </script>",
        ),
    ];
    let server = sites(PAGES);
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine
        .navigate(at(&server, "a.localhost", "/page.html"));
    assert_eq!(tab.run_until(|lines| !lines.is_empty()), ["ready"]);
    // The parent's layout gives the frame its size.
    tab.engine
        .frame_at(800, 600, crate::FrameTime::at(std::time::Duration::ZERO));
    assert_eq!(tab.run_until(|lines| !lines.is_empty()), ["read 200 100"]);
    assert_eq!(tab.engine.shown.scripts.frame_box_layouts(), 0);
    tab.engine.inspect(r#"{"id":1,"method":"page.sweep"}"#);
    let answers = tab.engine.take_inspected();
    let answer = devtools::Json::parse(&answers[0]).expect("JSON");
    let counts = answer.get("result").expect("a result");
    assert_eq!(
        counts
            .get("frameViewportStale")
            .and_then(devtools::Json::as_u64),
        Some(1),
        "{answer:?}"
    );
}
