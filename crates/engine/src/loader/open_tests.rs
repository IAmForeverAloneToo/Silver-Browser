//! Pages opening tabs (M37, docs/design/chrome.md, "Pages opening
//! tabs"): a link of target `_blank` or a name found nowhere, a link
//! clicked with Ctrl or the middle button, a form sent to a new tab,
//! `window.open` by the user's activation alone, and `window.close` of
//! a tab a script opened; each a request the browser takes.

use net::Url;

use super::tests::load_all;
use crate::testing::*;
use crate::{Button, LoadState, Modifiers, OpenTab, POPUP_BLOCKED};

/// The console's lines since they were last taken.
fn lines(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

fn click(tab: &mut Tab, x: f32, y: f32) {
    tab.engine.pointer_move(x, y);
    tab.engine.pointer_down(x, y, Button::Primary, 1);
    tab.engine.pointer_up(x, y, Button::Primary, 1);
}

/// A tab of `page`, `/next` the page its links go to.
fn tab_of(page: &'static str, scripts: bool) -> (Tab, Url) {
    let server = serve(move |path| match path {
        "/" => Some(html(page)),
        _ => Some(html("<title>next</title>")),
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(scripts);
    load_all(&mut tab, server.url("/"));
    tab.engine.frame(400, 300);
    let next = server.url("/next");
    // The server lives as long as the test.
    std::mem::forget(server);
    (tab, next)
}

fn opened(url: &Url, show: bool, by_script: bool) -> Vec<OpenTab> {
    vec![OpenTab {
        url: url.clone(),
        show,
        by_script,
    }]
}

const LINKS: &str = "<style>body { margin: 0 } a { display: block; height: 40px }</style>\
    <a href=/next target=_blank>blank</a><a href=/next target=nowhere>nowhere</a>\
    <a href=/next>plain</a>";

#[test]
fn a_link_of_target_blank_or_a_name_found_nowhere_opens_a_tab() {
    for scripts in [false, true] {
        let (mut tab, next) = tab_of(LINKS, scripts);
        click(&mut tab, 10.0, 20.0);
        assert_eq!(
            tab.engine.take_opened(),
            opened(&next, true, false),
            "{scripts}"
        );
        click(&mut tab, 10.0, 60.0);
        assert_eq!(
            tab.engine.take_opened(),
            opened(&next, true, false),
            "{scripts}"
        );
        // The page itself stays.
        assert_eq!(tab.engine.load_state(), &LoadState::Loaded);
        assert_ne!(tab.engine.url(), Some(&next));
    }
}

#[test]
fn a_link_clicked_with_ctrl_or_the_middle_button_opens_behind_or_with_shift_in_front() {
    // Scripts off, on, and on with a listener of clicks, whose activation
    // the bindings run.
    for (page, scripts) in [(LINKS, false), (LINKS, true), (LISTENED, true)] {
        let (mut tab, next) = tab_of(page, scripts);
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::default()
        };
        tab.engine.set_modifiers(ctrl);
        click(&mut tab, 10.0, 100.0);
        assert_eq!(
            tab.engine.take_opened(),
            opened(&next, false, false),
            "{scripts}"
        );
        tab.engine.set_modifiers(Modifiers {
            shift: true,
            ..ctrl
        });
        click(&mut tab, 10.0, 100.0);
        assert_eq!(
            tab.engine.take_opened(),
            opened(&next, true, false),
            "{scripts}"
        );
        tab.engine.set_modifiers(Modifiers::default());
        tab.engine.pointer_move(10.0, 100.0);
        tab.engine.pointer_down(10.0, 100.0, Button::Auxiliary, 1);
        tab.engine.pointer_up(10.0, 100.0, Button::Auxiliary, 1);
        assert_eq!(
            tab.engine.take_opened(),
            opened(&next, false, false),
            "{scripts}"
        );
        assert_ne!(tab.engine.url(), Some(&next), "this tab stays");
    }
}

const LISTENED: &str = "<style>body { margin: 0 } a { display: block; height: 40px }</style>    <a href=/next target=_blank>blank</a><a href=/next target=nowhere>nowhere</a>    <a href=/next>plain</a><script>addEventListener('click', () => {})</script>";

const OPENER: &str = "<style>body { margin: 0 } button { display: block; height: 40px }</style>\
    <button id=one onclick=\"console.log(window.open('/next'))\">one</button>\
    <button id=two onclick=\"window.open('/next'); window.open('/next')\">two</button>\
    <script>window.open('/next'); console.log('loaded')</script>";

#[test]
fn window_open_opens_a_tab_by_the_users_activation_alone_and_gives_back_no_window() {
    let (mut tab, next) = tab_of(OPENER, true);
    // Without an activation, the page's own call opens nothing.
    assert_eq!(lines(&mut tab), ["loaded"]);
    assert!(tab.engine.take_opened().is_empty());
    assert_eq!(tab.engine.take_notice().as_deref(), Some(POPUP_BLOCKED));
    // A click lets one tab open, a script's; the window is not given.
    click(&mut tab, 10.0, 20.0);
    assert_eq!(lines(&mut tab), ["null"]);
    assert_eq!(tab.engine.take_opened(), opened(&next, true, true));
    // One activation, one tab: the second call is blocked.
    click(&mut tab, 10.0, 60.0);
    assert_eq!(tab.engine.take_opened(), opened(&next, true, true));
    assert_eq!(tab.engine.take_notice().as_deref(), Some(POPUP_BLOCKED));
}

#[test]
fn window_open_of_a_named_frame_navigates_it_and_a_sandbox_without_popups_refuses() {
    let (mut tab, _) = tab_of(
        "<style>body { margin: 0 } button { display: block; height: 40px }</style>\
         <iframe name=f srcdoc='frame' style='display:block;border:0;height:40px'></iframe>\
         <iframe sandbox='allow-scripts' style='display:block;border:0;height:60px' srcdoc=\"<button onclick=window.open('https://example.com/') \
         style='width:100px;height:40px'>s</button>\"></iframe>\
         <button onclick=\"window.open('/next', 'f')\">named</button>",
        true,
    );
    // The named frame navigates; no tab opens.
    click(&mut tab, 10.0, 120.0);
    let named = |engine: &crate::Engine| {
        let document = engine.document();
        let first = document
            .subtree(document.root())
            .find(|&node| document.tag(node) == Some("iframe"))
            .expect("the named frame");
        engine.shown.frames.get(&first).is_some_and(|frame| {
            frame
                .engine
                .url()
                .is_some_and(|url| url.as_str().ends_with("/next"))
        })
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !named(&tab.engine) {
        assert!(
            std::time::Instant::now() < deadline,
            "the named frame navigates"
        );
        tab.engine.poll_network();
        tab.engine.run_tasks();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(tab.engine.take_opened().is_empty());
    // The sandboxed frame's call is refused, said on the console.
    click(&mut tab, 10.0, 60.0);
    assert!(tab.engine.take_opened().is_empty());
    let said = lines(&mut tab);
    assert!(
        said.iter()
            .any(|line| line.contains("'allow-popups' permission is not set")),
        "{said:?}"
    );
}

#[test]
fn window_close_closes_only_a_tab_a_script_opened() {
    use crate::view::View;
    let (mut tab, _) = tab_of(
        "<style>body { margin: 0 } button { display: block; height: 40px }</style>\
         <button onclick='window.close()'>close</button>",
        true,
    );
    click(&mut tab, 10.0, 20.0);
    assert!(!tab.engine.take_close_request(), "the user's own tab");
    tab.engine.set_opened_by_script(true);
    click(&mut tab, 10.0, 20.0);
    assert!(tab.engine.take_close_request(), "a script's");
}

#[test]
fn a_form_sent_to_a_new_tab_opens_one_by_get_and_posts_here() {
    let (mut tab, next) = tab_of(
        "<style>body { margin: 0 } button { display: block; height: 40px }</style>\
         <form action=/next target=_blank><button>get</button></form>\
         <form action=/next method=post target=_blank><button>post</button></form>",
        false,
    );
    click(&mut tab, 10.0, 20.0);
    let get = Url::parse(&format!("{next}?")).expect("parses");
    assert_eq!(tab.engine.take_opened(), opened(&get, true, false));
    click(&mut tab, 10.0, 60.0);
    assert!(tab.engine.take_opened().is_empty());
    tab.wait_until(|engine| engine.url() == Some(&next));
}

#[test]
fn only_pages_of_the_web_and_a_blank_one_open_in_a_tab() {
    let open = |url: &str| crate::frames::opens_in_tab(&Url::parse(url).unwrap());
    assert!(open("https://example.com/") && open("http://a.test/") && open("about:blank"));
    for url in [
        "javascript:alert(1)",
        "file:///c:/x",
        "data:text/html,x",
        "mailto:a@b.c",
        "about:silver",
        "devtools:panel",
    ] {
        assert!(!open(url), "{url}");
    }
}
