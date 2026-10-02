//! The window's title (docs/design/navigation.md, "The title").

/// The browser's name, which every window title ends with.
const NAME: &str = "Silver Browser";

/// The most characters of a page's title the window shows.
const MAX_CHARS: usize = 1024;

/// The window's title for a page titled `title` at `url`: "{title} —
/// Silver Browser", the URL standing in for an empty title, and the
/// browser's name alone with neither. A page writes its title as it
/// likes, so it loses its control characters and the bidirectional
/// formatting characters, which would let it reorder the browser's name
/// after it, and is cut at 1,024 characters.
pub fn window_title(title: &str, url: Option<&str>) -> String {
    let shown = clean(title);
    let shown = if shown.is_empty() {
        url.map(clean).unwrap_or_default()
    } else {
        shown
    };
    if shown.is_empty() {
        NAME.to_owned()
    } else {
        format!("{shown} — {NAME}")
    }
}

/// `text` without control characters and bidirectional formatting
/// characters, trimmed, at most [`MAX_CHARS`] long.
fn clean(text: &str) -> String {
    let kept: String = text
        .chars()
        .filter(|&c| !c.is_control() && !is_bidi_formatting(c))
        .collect();
    let cut: String = kept.trim().chars().take(MAX_CHARS).collect();
    cut.trim_end().to_owned()
}

/// The marks, embeddings, overrides and isolates of Unicode's
/// bidirectional algorithm.
fn is_bidi_formatting(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}
