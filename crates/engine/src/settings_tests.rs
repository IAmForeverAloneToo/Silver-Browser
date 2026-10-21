//! The browser's settings (M37): read and written as JSON, each value
//! held to what it may be, kept in the profile through the context, and
//! applied there: a third party's cookies blocked, the languages sent as
//! `Accept-Language` and given to `navigator.languages`.

use devtools::Json;
use net::Url;
use profile::{Memory, Profile};

use crate::Context;
use crate::settings::{Appearance, Search, Settings, Start};
use crate::testing::*;

fn json(text: &str) -> Json {
    Json::parse(text).expect("JSON")
}

#[test]
fn settings_are_written_and_read_back_and_what_is_not_understood_keeps_its_default() {
    let mut settings = Settings::default();
    settings
        .set("start", &json("\"https://example.com/home\""))
        .unwrap();
    settings.set("search", &json("\"Startpage\"")).unwrap();
    settings.set("scripts", &json("false")).unwrap();
    settings.set("appearance", &json("\"dark\"")).unwrap();
    settings
        .set("languages", &json("[\"fr\", \"en-US\", \"fr\", \"xx\"]"))
        .unwrap();
    settings
        .set("blockThirdPartyCookies", &json("true"))
        .unwrap();
    let read = Settings::from_bytes(&settings.to_bytes());
    assert_eq!(read, settings);
    assert_eq!(
        read.start,
        Start::Page("https://example.com/home".to_owned())
    );
    assert_eq!(
        read.languages,
        ["fr", "en-US"],
        "unknown and repeated dropped"
    );
    assert_eq!(read.appearance, Appearance::Dark);
    // Junk, or a member of another version, keeps the defaults.
    assert_eq!(Settings::from_bytes(b"not json"), Settings::default());
    let other = Settings::from_bytes(br#"{"scripts":"yes","future":1,"search":"Bing"}"#);
    assert!(other.scripts, "a value not understood keeps its default");
    assert_eq!(other.search, Search::Bing);
}

#[test]
fn each_setting_holds_to_what_it_may_be() {
    let mut settings = Settings::default();
    for (name, value) in [
        ("start", "\"javascript:alert(1)\""),
        ("start", "\"file:///c:/x\""),
        ("search", "\"Nowhere\""),
        ("scripts", "\"yes\""),
        ("languages", "[]"),
        ("languages", "[\"xx\"]"),
        ("downloadsFolder", "\"relative/folder\""),
        ("nothing", "1"),
    ] {
        assert!(settings.set(name, &json(value)).is_err(), "{name} {value}");
    }
    assert_eq!(settings, Settings::default());
    let long = format!("\"https://a.test/{}\"", "a".repeat(70_000));
    assert!(settings.set("start", &json(&long)).is_err());
    settings.set("downloadsFolder", &Json::Null).unwrap();
    assert_eq!(settings.downloads_folder, None);
}

#[test]
fn a_search_puts_the_text_in_its_engines_query() {
    assert_eq!(
        Search::DuckDuckGo.url_for(" two words? ").as_str(),
        "https://duckduckgo.com/?q=two+words%3F"
    );
    assert_eq!(
        Search::Google.url_for("a&b=c#d").as_str(),
        "https://www.google.com/search?q=a%26b%3Dc%23d"
    );
    assert_eq!(
        Search::Bing.url_for("\u{e9}").as_str(),
        "https://www.bing.com/search?q=%C3%A9"
    );
}

#[test]
fn the_languages_weigh_less_in_order() {
    let mut settings = Settings::default();
    assert_eq!(settings.accept_language(), "en-US,en;q=0.9");
    settings
        .set("languages", &json("[\"fr\", \"de\", \"en\"]"))
        .unwrap();
    assert_eq!(settings.accept_language(), "fr,de;q=0.9,en;q=0.8");
}

#[test]
fn the_settings_are_kept_in_the_profile_through_the_context() {
    let memory = Memory::new();
    let context = Context::new();
    context
        .set_profile(Profile::in_memory(&memory).unwrap())
        .unwrap();
    let before = context.settings_changes();
    context.update_settings(|settings| settings.search = Search::Brave);
    assert!(context.settings_changes() > before);
    // The session's tabs kept quietly: no change readers act on.
    let changes = context.settings_changes();
    context.keep_session(vec![
        "https://a.test/".to_owned(),
        "about:silver".to_owned(),
    ]);
    assert_eq!(context.settings_changes(), changes);
    context
        .flush_profile(std::time::Duration::from_secs(10))
        .unwrap();
    let again = Context::new();
    again
        .set_profile(Profile::in_memory(&memory).unwrap())
        .unwrap();
    let read = again.settings();
    assert_eq!(read.search, Search::Brave);
    assert_eq!(
        read.session,
        ["https://a.test/"],
        "pages of the web's alone"
    );
}

#[test]
fn the_languages_go_with_requests_and_to_navigator_languages() {
    let server = serve(|_| {
        Some(html(
            "<script>console.log(navigator.language + ' ' + navigator.languages.join())</script>",
        ))
    });
    let context = Context::new();
    context.update_settings(|settings| {
        settings
            .set("languages", &json("[\"fr\", \"en-US\"]"))
            .unwrap()
    });
    let mut tab = Tab::new();
    tab.engine.join(&context);
    tab.engine.set_scripting(true);
    tab.engine.set_languages(context.settings().languages);
    tab.engine.navigate(server.url("/"));
    let lines = tab.run_until(|lines| !lines.is_empty());
    assert_eq!(lines, ["fr fr,en-US"]);
    let sent = server.requests()[0]
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("Accept-Language"))
        .map(|(_, value)| value.clone());
    assert_eq!(sent.as_deref(), Some("fr,en-US;q=0.9"));
}

#[test]
fn a_third_partys_cookies_are_blocked_by_the_setting() {
    let context = Context::new();
    let url = Url::parse("https://tracker.test/").unwrap();
    let third = net::cookies::Context {
        same_site: false,
        top_level: false,
        safe_method: true,
        partition: None,
    };
    let store = |context: &Context| {
        context.jar().lock().unwrap().store(
            b"id=1; SameSite=None; Secure",
            &url,
            net::cookies::Api::Http,
            third.clone(),
            std::time::SystemTime::now(),
        )
    };
    assert!(store(&context), "sent by default (D25)");
    context.update_settings(|settings| settings.block_third_party_cookies = true);
    assert!(!store(&context), "not set");
    let header = context.jar().lock().unwrap().header(
        &url,
        net::cookies::Api::Http,
        third.clone(),
        std::time::SystemTime::now(),
    );
    assert_eq!(header, None, "nor sent");
    // A top-level navigation from another site is no third party's: its
    // cookies are sent.
    let navigation = net::cookies::Context {
        same_site: false,
        top_level: true,
        safe_method: true,
        partition: None,
    };
    let header = context.jar().lock().unwrap().header(
        &url,
        net::cookies::Api::Http,
        navigation,
        std::time::SystemTime::now(),
    );
    assert!(header.is_some(), "a top-level navigation's");
    // The site's own, at the top, still are.
    let header = context.jar().lock().unwrap().header(
        &url,
        net::cookies::Api::Http,
        net::cookies::Context::SAME_SITE,
        std::time::SystemTime::now(),
    );
    assert!(header.is_some());
}
