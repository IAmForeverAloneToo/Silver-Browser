//! Line breaking (UAX #14 by ICU4X's line segmenter, gap batch D, D55):
//! where a line may break within a run of text.

use std::sync::OnceLock;

use icu_segmenter::LineSegmenter;
use icu_segmenter::options::{LineBreakOptions, LineBreakWordOption};

/// How words break (`word-break`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WordBreaking {
    /// As each script's custom has it.
    #[default]
    Normal,
    /// Between any two letters too.
    BreakAll,
    /// Not within words, in CJK text either.
    KeepAll,
}

/// The byte offsets in `text` where a line may break, before the
/// character there: none at its start or its end, nor before a line
/// feed, whose break is forced. Between two printable ASCII characters
/// the reference browser keeps a table of its own (Blink's ASCII line
/// break table) rather than UAX #14's: there a break is only after a
/// hyphen that follows a letter or digit and comes before a letter, so
/// that `width/margin` or `a.b` stay whole, as `well-known` does not.
pub fn line_breaks(text: &str, words: WordBreaking) -> Vec<usize> {
    static SEGMENTERS: [OnceLock<icu_segmenter::LineSegmenterBorrowed<'static>>; 3] =
        [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    let (slot, option) = match words {
        WordBreaking::Normal => (0, LineBreakWordOption::Normal),
        WordBreaking::BreakAll => (1, LineBreakWordOption::BreakAll),
        WordBreaking::KeepAll => (2, LineBreakWordOption::KeepAll),
    };
    let segmenter = SEGMENTERS[slot].get_or_init(|| {
        let mut options = LineBreakOptions::default();
        options.word_option = Some(option);
        LineSegmenter::new_auto(options)
    });
    let bytes = text.as_bytes();
    let printable = |b: u8| (b'!'..=b'~').contains(&b);
    segmenter
        .segment_str(text)
        .filter(|&at| at > 0 && at < text.len() && !text[..at].ends_with('\n'))
        .filter(|&at| {
            let (before, after) = (bytes[at - 1], bytes[at]);
            if words == WordBreaking::BreakAll || !printable(before) || !printable(after) {
                return true;
            }
            before == b'-'
                && after.is_ascii_alphabetic()
                && at >= 2
                && bytes[at - 2].is_ascii_alphanumeric()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces(text: &str, words: WordBreaking) -> Vec<&str> {
        let mut out = Vec::new();
        let mut start = 0;
        for at in line_breaks(text, words).into_iter().chain([text.len()]) {
            out.push(&text[start..at]);
            start = at;
        }
        out
    }

    #[test]
    fn lines_break_after_spaces_hyphens_and_between_ideographs() {
        assert_eq!(pieces("one two", WordBreaking::Normal), ["one ", "two"]);
        assert_eq!(
            pieces("well-known", WordBreaking::Normal),
            ["well-", "known"]
        );
        // ASCII as the reference browser breaks it.
        for whole in ["width/margin", "a.b,c", "-1", "x--y", "50%-off"] {
            assert_eq!(pieces(whole, WordBreaking::Normal), [whole], "{whole}");
        }
        assert_eq!(
            pieces("\u{65e5}\u{672c}\u{8a9e}", WordBreaking::Normal),
            ["\u{65e5}", "\u{672c}", "\u{8a9e}"]
        );
        assert_eq!(
            pieces("a\u{a0}b", WordBreaking::Normal),
            ["a\u{a0}b"],
            "no break at a no-break space"
        );
        assert_eq!(pieces("abc", WordBreaking::BreakAll), ["a", "b", "c"]);
        assert_eq!(
            pieces("\u{65e5}\u{672c}", WordBreaking::KeepAll),
            ["\u{65e5}\u{672c}"]
        );
        assert!(line_breaks("", WordBreaking::Normal).is_empty());
    }
}
