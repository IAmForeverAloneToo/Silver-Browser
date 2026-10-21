//! The browser's own pages (M37, docs/design/chrome.md, "The browser's
//! pages"): `about:downloads` (and, with the settings and the profiles,
//! the pages for them), each made by the engine from the HTML bundled
//! here under the DevTools panel's policy, its scripts on, its window
//! given the panel's bridge to the browser, which answers its requests
//! from its own records. A page reaches none of them: a link, a script
//! or a frame's navigation to an `about:` page other than `about:blank`
//! is refused but from a page the browser made, and in many processes a
//! renderer that shows one is locked to them alone.

use net::Url;

/// The downloads page's URL.
pub const DOWNLOADS: &str = "about:downloads";
/// The settings page's URL.
pub const SETTINGS: &str = "about:settings";

/// Each page's URL and markup.
const PAGES: &[(&str, &str)] = &[
    (DOWNLOADS, include_str!("internal/downloads.html")),
    (SETTINGS, include_str!("internal/settings.html")),
];

/// Whether `url` is one of the browser's own pages.
pub fn is_internal(url: &Url) -> bool {
    page(url).is_some()
}

/// The markup of the browser's own page at `url`, if it is one.
pub(crate) fn page(url: &Url) -> Option<&'static str> {
    PAGES
        .iter()
        .find(|(at, _)| *at == url.as_str())
        .map(|(_, markup)| *markup)
}
