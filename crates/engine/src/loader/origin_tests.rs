//! The engine keeping a document's origin (M27.2,
//! docs/design/fetch.md): `self.origin`, `isSecureContext` and
//! `document.domain` for a served page and a `data:` one.

use net::Url;

use super::script_tests::scripted;
use super::tests::load_all;
use crate::testing::*;

/// What a page logs of its origin, then of `document.domain` set to
/// itself and to values that are not a registrable suffix of its host.
const PROBE: &str = "<script>console.log(self.origin, isSecureContext, \
                     JSON.stringify(document.domain)); \
                     for (const v of [document.domain, '0.0.1', '127', 'com', '']) { \
                     try { document.domain = v; console.log('set', JSON.stringify(v)); } \
                     catch (e) { console.log(e.name, JSON.stringify(v)); } } \
                     console.log(JSON.stringify(document.domain))</script>";

#[test]
fn a_served_page_has_its_servers_origin_and_its_host_for_a_domain() {
    let (tab, console) = scripted(PROBE, true);
    let port = tab.engine.url().and_then(Url::port).expect("a port");
    assert_eq!(
        console,
        [
            format!("http://127.0.0.1:{port} true \"127.0.0.1\""),
            "set \"127.0.0.1\"".to_owned(),
            "SecurityError \"0.0.1\"".to_owned(),
            "SecurityError \"127\"".to_owned(),
            "SecurityError \"com\"".to_owned(),
            "SecurityError \"\"".to_owned(),
            "\"127.0.0.1\"".to_owned(),
        ]
    );
}

#[test]
fn a_data_page_has_an_opaque_origin_and_no_domain() {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    load_all(
        &mut tab,
        Url::parse(&format!("data:text/html,{PROBE}")).unwrap(),
    );
    let console: Vec<String> = tab
        .engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect();
    assert_eq!(
        console,
        [
            "null true \"\"",
            "SecurityError \"\"",
            "SecurityError \"0.0.1\"",
            "SecurityError \"127\"",
            "SecurityError \"com\"",
            "SecurityError \"\"",
            "\"\"",
        ]
    );
}

#[test]
fn a_window_is_its_own_top_level_with_no_frames() {
    // A top-level window's relatives, which testharness.js walks (M27).
    let (_tab, console) = scripted(
        "<script>console.log(parent === window, top === window, frames === window, \
         opener, length, window.length)</script>",
        true,
    );
    assert_eq!(console, ["true true true null 0 0"]);
}
