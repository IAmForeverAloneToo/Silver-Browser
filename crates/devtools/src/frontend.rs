//! The panel's page (M36, `docs/design/devtools.md`, "Where the panel
//! is"): one HTML document the browser makes, its style and its scripts
//! put inline, so that it fetches nothing. Each tool is a script of its
//! own, registered with the frame's `tool()`; the last script starts
//! the panel.

/// The URL the panel's page is shown under. No navigation reaches it:
/// the browser shows it by asking for the panel.
pub const PANEL_URL: &str = "devtools:panel";

/// The policy the panel's page is shown under: nothing fetched, and no
/// code but its own inline scripts.
pub const PANEL_CSP: &str =
    "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:";

const HTML: &str = include_str!("../frontend/panel.html");
const STYLE: &str = include_str!("../frontend/panel.css");

/// The scripts, in the order they run: the frame, the tools, the start.
const SCRIPTS: &[&str] = &[
    include_str!("../frontend/panel.js"),
    include_str!("../frontend/elements.js"),
    include_str!("../frontend/console.js"),
    include_str!("../frontend/start.js"),
];

/// The panel's page.
pub fn page() -> String {
    HTML.replacen("/*STYLE*/", STYLE, 1)
        .replacen("/*SCRIPT*/", &SCRIPTS.join("\n"), 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_holds_its_style_and_scripts_whole() {
        let page = page();
        assert!(!page.contains("/*STYLE*/") && !page.contains("/*SCRIPT*/"));
        assert!(page.contains("devtools.onmessage"));
        assert!(page.contains("--accent"));
        // Nothing inline ends its element early.
        for part in SCRIPTS.iter().chain([&STYLE]) {
            let lower = part.to_ascii_lowercase();
            assert!(!lower.contains("</script") && !lower.contains("</style"));
        }
        assert_eq!(page.matches("<script>").count(), 1);
    }
}
