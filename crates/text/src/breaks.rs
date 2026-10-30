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
    if words == WordBreaking::Normal && text.bytes().all(|b| (b' '..=b'~').contains(&b)) {
        return ascii_breaks(text.as_bytes());
    }
    by_segmenter(text, words)
}

/// [`line_breaks`] for text of spaces and printable ASCII, as UAX #14
/// and the table give it, without the segmenter: after spaces, unless
/// what follows closes, ends or joins (classes CL, CP, EX, IS and SY,
/// rule LB13), or what the spaces follow opens (OP, rule LB14); and
/// after a hyphen between a letter or digit and a letter.
fn ascii_breaks(bytes: &[u8]) -> Vec<usize> {
    let mut breaks = Vec::new();
    // The last character that is not a space.
    let mut last = None;
    for at in 1..bytes.len() {
        let (before, after) = (bytes[at - 1], bytes[at]);
        if before != b' ' {
            last = Some(before);
        }
        let allowed = if before == b' ' {
            after != b' '
                && !matches!(
                    after,
                    b')' | b']' | b'}' | b'!' | b'?' | b',' | b'.' | b':' | b';' | b'/'
                )
                && !matches!(last, Some(b'(' | b'[' | b'{'))
        } else {
            before == b'-'
                && after.is_ascii_alphabetic()
                && at >= 2
                && bytes[at - 2].is_ascii_alphanumeric()
        };
        if allowed {
            breaks.push(at);
        }
    }
    breaks
}

/// [`line_breaks`] by ICU4X's segmenter, whatever the text.
fn by_segmenter(text: &str, words: WordBreaking) -> Vec<usize> {
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

    #[test]
    fn ascii_breaks_as_the_segmenter_does() {
        // Every text of up to five of these, then longer ones made from
        // them.
        const ALPHABET: &[u8] = b"a1 -(.\"'/)?%$[!,";
        let mut texts = vec![Vec::new()];
        let mut last = vec![Vec::new()];
        for _ in 0..5 {
            last = last
                .iter()
                .flat_map(|t: &Vec<u8>| {
                    ALPHABET.iter().map(move |&c| {
                        let mut t = t.clone();
                        t.push(c);
                        t
                    })
                })
                .collect();
            texts.extend(last.iter().cloned());
        }
        let mut seed = 7u32;
        for _ in 0..20_000 {
            let len = 6 + seed as usize % 20;
            let text = (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    if seed >> 30 == 0 {
                        b' '
                    } else {
                        b' ' + (seed >> 16) as u8 % 95
                    }
                })
                .collect();
            texts.push(text);
        }
        for text in texts {
            let text = std::str::from_utf8(&text).unwrap();
            for words in [WordBreaking::Normal, WordBreaking::KeepAll] {
                assert_eq!(
                    line_breaks(text, words),
                    by_segmenter(text, words),
                    "{text:?} {words:?}"
                );
            }
        }
    }
}
