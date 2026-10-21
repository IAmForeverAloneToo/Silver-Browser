//! The pages the browser makes (M37): `about:silver` the gallery of the
//! engine's demonstrations, each shown under `about:silver/<name>` with
//! its scripts and images and reached by no page of the web; the error
//! pages in light and dark, with a "Try again" where a retry can help.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use net::Url;
use net::test_server::Reply;

use crate::testing::*;
use crate::{ColorScheme, Engine, LoadError, LoadState};

/// Runs the engine until `done` holds of it.
fn until(tab: &mut Tab, mut done: impl FnMut(&mut Engine) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !done(&mut tab.engine) {
        assert!(std::time::Instant::now() < deadline, "in time");
        tab.engine.poll_network();
        tab.engine.run_tasks();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// The `href`s of the shown document's links.
fn links(engine: &Engine) -> Vec<String> {
    let document = engine.document();
    document
        .subtree(document.root())
        .filter(|&node| document.tag(node) == Some("a"))
        .filter_map(|node| document.attr(node, "href").map(str::to_owned))
        .collect()
}

/// Follows the shown document's link to `href`, as a click does.
fn follow(tab: &mut Tab, href: &str) {
    let document = tab.engine.document();
    let node = document
        .subtree(document.root())
        .find(|&node| document.attr(node, "href") == Some(href))
        .unwrap_or_else(|| panic!("a link to {href}"));
    let link = crate::Link {
        page: tab.engine.shown.page.id(),
        node,
        url: Url::parse(href).unwrap(),
    };
    tab.engine.follow_link(&link);
}

#[test]
fn the_gallery_links_each_demonstration() {
    let mut tab = Tab::new();
    tab.engine.navigate(Url::parse("about:silver").unwrap());
    let linked: Vec<String> = links(&tab.engine)
        .into_iter()
        .filter(|href| href.starts_with("about:silver/"))
        .collect();
    let demos: Vec<String> = crate::pages::DEMOS
        .iter()
        .map(|(name, _, _)| format!("about:silver/{name}"))
        .collect();
    assert_eq!(linked, demos);
    // Each is a page of its own.
    for (name, _, _) in crate::pages::DEMOS {
        assert!(crate::pages::demo(name).is_some(), "{name}");
    }
    assert_eq!(crate::pages::demo("nothing"), None);
}

#[test]
fn a_demonstration_is_shown_with_its_images_and_scripts() {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(Url::parse("about:silver").unwrap());
    follow(&mut tab, "about:silver/photo-grid");
    until(&mut tab, |engine| {
        *engine.load_state() == LoadState::Loaded && engine.shown.images.settled()
    });
    assert_eq!(
        tab.engine.url().map(Url::as_str),
        Some("about:silver/photo-grid")
    );
    assert!(tab.engine.can_go_back(), "a navigation of its own");
    let known = tab.engine.shown.images.known();
    assert!(known.len() >= 12, "every photo: {}", known.len());
    for url in &known {
        assert!(url.starts_with("data:image/"), "{}", &url[..20]);
        // Ready, not broken.
        assert_eq!(tab.engine.shown.images.outcome(url), Some(false));
    }
    // The icons, in SVG.
    tab.engine
        .navigate(Url::parse("about:silver/svg-icons").unwrap());
    until(&mut tab, |engine| {
        *engine.load_state() == LoadState::Loaded && engine.shown.images.settled()
    });
    let known = tab.engine.shown.images.known();
    assert!(known.len() >= 30, "every icon: {}", known.len());
    assert!(
        known
            .iter()
            .all(|url| tab.engine.shown.images.outcome(url) == Some(false))
    );
    // A demonstration's scripts run.
    tab.engine
        .navigate(Url::parse("about:silver/scripts").unwrap());
    until(&mut tab, |engine| engine.title() == "x");
    // Its workers too, made from data: URLs: no policy holds them back.
    tab.engine
        .navigate(Url::parse("about:silver/mandelbrot?workers=2&iterations=5").unwrap());
    let lines = tab.run_until(|lines| lines.iter().any(|line| line.starts_with("set ")));
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Content-Security-Policy")),
        "{lines:?}"
    );
    // An origin of its own: opaque.
    assert!(tab.engine.shown.page.origin().is_opaque());
}

#[test]
fn no_page_of_the_web_reaches_a_demonstration_and_one_not_there_is_an_error() {
    let server = serve(|_| {
        Some(html(
            "<a href=\"about:silver/particles\">particles</a>\
             <a href=\"about:silver-reload\">again</a>",
        ))
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.load(server.url("/"));
    tab.settle();
    follow(&mut tab, "about:silver/particles");
    tab.settle();
    assert_eq!(tab.engine.url(), Some(&server.url("/")));
    assert!(
        tab.engine
            .take_notice()
            .is_some_and(|notice| notice.contains("the browser's own pages"))
    );
    // Nor does its "Try again" of its own.
    follow(&mut tab, crate::pages::RELOAD);
    tab.settle();
    assert_eq!(tab.engine.url(), Some(&server.url("/")));
    assert_eq!(
        paths(&server).iter().filter(|path| *path == "/").count(),
        1,
        "not reloaded"
    );
    tab.engine
        .navigate(Url::parse("about:silver/nothing").unwrap());
    assert!(text(&tab.engine).contains("No page at about:silver/nothing"));
}

/// The colour of the shown page's heading.
fn heading(engine: &mut Engine) -> (f32, f32, f32) {
    engine.frame(400, 300);
    let node = element(engine, "h1");
    let color = engine.styles().get(node).expect("styled").color;
    (color.r, color.g, color.b)
}

#[test]
fn an_error_page_is_light_or_dark_after_the_setting() {
    let mut tab = Tab::new();
    tab.engine.navigate(Url::parse("about:nothing").unwrap());
    let light = heading(&mut tab.engine);
    assert!(light.0 < 0.2, "dark text: {light:?}");
    let mut preferences = tab.engine.preferences();
    preferences.color_scheme = ColorScheme::Dark;
    tab.engine.set_preferences(preferences);
    let dark = heading(&mut tab.engine);
    assert!(dark.0 > 0.8, "light text: {dark:?}");
}

#[test]
fn a_network_errors_page_tries_again_and_a_refusals_does_not() {
    // The first answer fails on the way; the second is the page.
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = asked.clone();
    let server = net::test_server::TestServer::plain(move |_| {
        match counted.fetch_add(1, Ordering::SeqCst) {
            0 => Reply::Close,
            _ => Reply::ok("text/html", b"<title>there</title>here"),
        }
    });
    let mut tab = Tab::new();
    let url = server.url("/");
    tab.engine.navigate(url.clone());
    until(&mut tab, |engine| {
        matches!(engine.load_state(), LoadState::Failed(_))
    });
    assert_eq!(tab.engine.url(), Some(&url));
    assert!(
        links(&tab.engine).contains(&crate::pages::RELOAD.to_owned()),
        "{}",
        text(&tab.engine)
    );
    follow(&mut tab, crate::pages::RELOAD);
    until(&mut tab, |engine| engine.title() == "there");
    assert_eq!(tab.engine.url(), Some(&url));
    assert_eq!(tab.engine.history.len(), 1, "the entry reloaded");
    // A port no browser connects to fails the same way again: no retry.
    let refused = Url::parse("http://localhost:6000/").unwrap();
    tab.engine.navigate(refused);
    until(&mut tab, |engine| {
        matches!(engine.load_state(), LoadState::Failed(_))
    });
    assert!(!links(&tab.engine).contains(&crate::pages::RELOAD.to_owned()));
    // A page lost in the browser may be tried again.
    let lost = crate::pages::error(&url, &LoadError::Lost);
    assert!(lost.text_content(lost.root()).contains("Try again"));
}
