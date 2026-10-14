//! The adversarial site's pages again (M34.7, docs/design/processes.md,
//! "The compromised renderer"), each in a tab of renderers: the
//! attacker's and the victim's documents in renderers of their sites,
//! their frames across them, their fetches and cookies through the
//! browser. What one engine refuses, renderers refuse; what it lets
//! through with the defences gone, they do.

use std::sync::Arc;
use std::time::SystemTime;

use net::Url;
use net::transport::Config;

use super::{RemoteTab, ThreadLauncher};
use crate::loader::{adversarial_tests, security_tests};
use crate::view::View;

fn fonts() -> text::FontCollection {
    text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
}

/// A tab of renderers, scripts on, its network reaching hosts as
/// `config` says, `known` a host known as HTTPS only.
fn renderers(config: Config, known: Option<&Url>) -> Box<dyn View> {
    let launcher = Arc::new(ThreadLauncher::with_fonts(fonts()));
    let mut tab = RemoteTab::with_cap(launcher, fonts(), 16, false);
    tab.set_viewport(800, 600);
    tab.set_scripting(true);
    tab.set_transport(config);
    if let Some(known) = known {
        tab.hsts()
            .lock()
            .unwrap()
            .store(known, "max-age=1000", SystemTime::now());
    }
    Box::new(tab)
}

fn plain_renderers() -> Box<dyn View> {
    renderers(Config::default(), None)
}

#[test]
fn a_page_in_a_renderer_learns_nothing_of_an_origin_that_allows_nothing() {
    security_tests::learns_nothing(&plain_renderers);
}

#[test]
fn a_page_in_a_renderer_learns_by_cors_what_the_origin_allows_and_nothing_more() {
    security_tests::learns_by_cors(&plain_renderers);
}

#[test]
fn a_framing_page_in_a_renderer_reaches_nothing_its_frames_keep() {
    adversarial_tests::framing(&renderers);
}

#[test]
fn injected_markup_in_a_renderer_runs_nothing_and_reaches_nothing() {
    adversarial_tests::injected(&renderers);
}

#[test]
fn another_sites_renderer_carries_and_reads_none_of_the_victims_cookies() {
    adversarial_tests::cookies(&renderers);
}

#[test]
fn a_public_pages_renderer_sends_nothing_in_clear_or_to_the_local_network() {
    adversarial_tests::network(&renderers);
}
