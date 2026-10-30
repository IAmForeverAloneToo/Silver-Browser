//! Line breaking (UAX #14 by ICU4X's line segmenter, gap batch D, D55):
//! where a line may break within a run of text.

use std::sync::OnceLock;

use icu_segmenter::LineSegmenter;
use icu_segmenter::options::{LineBreakOptions, LineBreakStrictness, LineBreakWordOption};

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

/// How strict the rules are (`line-break`, CSS Text 3 section 5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Strictness {
    /// UAX #14's own rules, small kana and the prolonged sound mark kept
    /// off a line's start: `strict`, and `auto` here.
    #[default]
    Strict,
    /// Small kana and the prolonged sound mark may start a line.
    Normal,
    /// More breaks besides: before iteration marks, between leaders,
    /// and in Chinese and Japanese around some punctuation.
    Loose,
    /// Between any two typographic character units.
    Anywhere,
}

/// The rules [`line_breaks`] follows: `word-break`'s, `line-break`'s,
/// and whether the text's language is Chinese or Japanese, which
/// `normal` and `loose` tailor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BreakRules {
    pub words: WordBreaking,
    pub strictness: Strictness,
    pub chinese_or_japanese: bool,
}

impl From<WordBreaking> for BreakRules {
    fn from(words: WordBreaking) -> Self {
        BreakRules {
            words,
            ..BreakRules::default()
        }
    }
}

/// The byte offsets in `text` where a line may break, before the
/// character there: none at its start or its end, nor before a line
/// feed, whose break is forced. Between two printable ASCII characters
/// the reference browser keeps a table of its own (Blink's ASCII line
/// break table, [`ascii_pair_breaks`]) rather than UAX #14's, so that
/// `width/margin`, `a|b` or `a.b` stay whole, as `well-known` does not.
/// The strictness does not change where ASCII breaks, but `anywhere`.
pub fn line_breaks(text: &str, rules: impl Into<BreakRules>) -> Vec<usize> {
    let rules = rules.into();
    if rules.words == WordBreaking::Normal
        && rules.strictness != Strictness::Anywhere
        && text.bytes().all(|b| (b' '..=b'~').contains(&b))
    {
        return ascii_breaks(text.as_bytes());
    }
    by_segmenter(text, rules)
}

/// The byte offsets in `text` between its grapheme clusters (UAX #29),
/// its typographic character units: none at its start or its end.
pub fn grapheme_boundaries(text: &str) -> Vec<usize> {
    if text.is_ascii() && !text.contains('\r') {
        return (1..text.len()).collect();
    }
    icu_segmenter::GraphemeClusterSegmenter::new()
        .segment_str(text)
        .filter(|&at| at > 0 && at < text.len())
        .collect()
}

/// Whether the reference browser breaks between the printable ASCII
/// characters `before` and `after`, `earlier` the one before `before`,
/// as its table has it (measured from Chrome 156): after a hyphen but
/// before what closes, ends or joins, and before a digit only where the
/// hyphen follows a letter or digit; after a question mark the same
/// way, but before quotes; after other punctuation that ends, only
/// before what opens.
pub(crate) fn ascii_pair_breaks(earlier: Option<u8>, before: u8, after: u8) -> bool {
    match before {
        b'-' if after.is_ascii_digit() => earlier.is_some_and(|e| e.is_ascii_alphanumeric()),
        b'-' => !matches!(
            after,
            b'!' | b'$' | b')' | b',' | b'.' | b'/' | b':' | b';' | b'?' | b']' | b'}'
        ),
        b'?' => !matches!(
            after,
            b'!' | b'"' | b'\'' | b')' | b',' | b'.' | b'/' | b':' | b';' | b'?' | b']' | b'}'
        ),
        b'!' | b'"' | b'#' | b'%' | b'&' | b')' | b'*' | b'+' | b',' | b'.' | b':' | b';'
        | b'=' | b'>' | b'\\' | b']' | b'|' | b'}' | b'~' => {
            matches!(after, b'(' | b'<' | b'[' | b'{')
        }
        _ => false,
    }
}

/// [`line_breaks`] for text of spaces and printable ASCII, as UAX #14
/// and the table give it, without the segmenter: after spaces, unless
/// what follows closes, ends or joins (classes CL, CP, EX, IS and SY,
/// rule LB13), or what the spaces follow opens (OP, rule LB14); and
/// between two other characters where [`ascii_pair_breaks`] says.
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
            after != b' ' && ascii_pair_breaks(at.checked_sub(2).map(|e| bytes[e]), before, after)
        };
        if allowed {
            breaks.push(at);
        }
    }
    breaks
}

/// [`line_breaks`] by ICU4X's segmenter, whatever the text.
fn by_segmenter(text: &str, rules: impl Into<BreakRules>) -> Vec<usize> {
    /// One segmenter for each set of rules, made when first asked for.
    static SEGMENTERS: [OnceLock<icu_segmenter::LineSegmenterBorrowed<'static>>; 24] =
        [const { OnceLock::new() }; 24];
    let rules = rules.into();
    let words = rules.words;
    let (word_slot, option) = match words {
        WordBreaking::Normal => (0, LineBreakWordOption::Normal),
        WordBreaking::BreakAll => (1, LineBreakWordOption::BreakAll),
        WordBreaking::KeepAll => (2, LineBreakWordOption::KeepAll),
    };
    let (strict_slot, strictness) = match rules.strictness {
        Strictness::Strict => (0, LineBreakStrictness::Strict),
        Strictness::Normal => (1, LineBreakStrictness::Normal),
        Strictness::Loose => (2, LineBreakStrictness::Loose),
        Strictness::Anywhere => (3, LineBreakStrictness::Anywhere),
    };
    let slot = (word_slot * 4 + strict_slot) * 2 + usize::from(rules.chinese_or_japanese);
    let segmenter = SEGMENTERS[slot].get_or_init(|| {
        // The language matters to the segmenter only as Chinese or
        // Japanese or neither, and only while it is made.
        let japanese = "ja".parse().ok();
        let mut options = LineBreakOptions::default();
        options.word_option = Some(option);
        options.strictness = Some(strictness);
        options.content_locale = japanese.as_ref().filter(|_| rules.chinese_or_japanese);
        LineSegmenter::new_auto(options)
    });
    let anywhere = rules.strictness == Strictness::Anywhere;
    let bytes = text.as_bytes();
    let printable = |b: u8| (b'!'..=b'~').contains(&b);
    let by_table = words != WordBreaking::BreakAll && !anywhere;
    // Between two printable ASCII characters the table decides, the
    // segmenter elsewhere.
    let pair = |at: usize| by_table && printable(bytes[at - 1]) && printable(bytes[at]);
    let mut breaks: Vec<usize> = segmenter
        .segment_str(text)
        .filter(|&at| at > 0 && at < text.len() && !text[..at].ends_with('\n'))
        .filter(|&at| !pair(at))
        .collect();
    if by_table {
        let from = breaks.len();
        breaks.extend((1..bytes.len()).filter(|&at| {
            pair(at)
                && ascii_pair_breaks(
                    at.checked_sub(2).map(|e| bytes[e]),
                    bytes[at - 1],
                    bytes[at],
                )
        }));
        if breaks.len() > from {
            breaks.sort_unstable();
        }
    }
    tailored(text, rules, breaks)
}

/// `breaks` as the reference browser's ICU tailors them for CSS: under
/// `loose`, before an inseparable character (a leader or an ellipsis)
/// that follows a letter too; in Chinese and Japanese under `normal` and
/// `strict`, never before a wide prefix (a currency or a plus-minus
/// sign), which stays with what it follows.
fn tailored(text: &str, rules: BreakRules, mut breaks: Vec<usize>) -> Vec<usize> {
    let inseparable = |c: char| matches!(c, '\u{2024}'..='\u{2026}' | '\u{22ef}' | '\u{fe19}');
    let wide_prefix = |c: char| {
        matches!(
            c,
            '\u{a4}'
                | '\u{b1}'
                | '\u{20ac}'
                | '\u{2116}'
                | '\u{fe69}'
                | '\u{ff04}'
                | '\u{ffe1}'
                | '\u{ffe5}'
                | '\u{ffe6}'
        )
    };
    match rules.strictness {
        Strictness::Loose
            if rules.words != WordBreaking::KeepAll && text.chars().any(inseparable) =>
        {
            let mut chars = text.char_indices().peekable();
            while let Some((_, c)) = chars.next() {
                if let Some(&(at, next)) = chars.peek()
                    && inseparable(next)
                    && !c.is_whitespace()
                    && !inseparable(c)
                {
                    breaks.push(at);
                }
            }
            breaks.sort_unstable();
            breaks.dedup();
        }
        Strictness::Normal | Strictness::Strict if rules.chinese_or_japanese => {
            breaks.retain(|&at| !text[at..].chars().next().is_some_and(wide_prefix));
        }
        _ => {}
    }
    breaks
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
        for whole in ["width/margin", "a.b,c", "-1", "a|b", "a'(b", "a$(b"] {
            assert_eq!(pieces(whole, WordBreaking::Normal), [whole], "{whole}");
        }
        let chrome: [(&str, &[&str]); 9] = [
            ("x--y", &["x-", "-", "y"]),
            ("50%-off", &["50%-", "off"]),
            ("a-1", &["a-", "1"]),
            ("a--1", &["a-", "-1"]),
            ("-x", &["-", "x"]),
            ("a!(b", &["a!", "(b"]),
            ("a?1", &["a?", "1"]),
            ("a?-b", &["a?", "-", "b"]),
            ("a-?b", &["a-?", "b"]),
        ];
        for (text, expected) in chrome {
            assert_eq!(pieces(text, WordBreaking::Normal), expected, "{text}");
            // So in text the segmenter breaks too.
            let wide = format!("\u{5b57} {text}");
            assert_eq!(
                pieces(&wide, WordBreaking::Normal)[1..],
                *expected,
                "{wide}"
            );
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

    fn with(text: &str, strictness: Strictness, chinese_or_japanese: bool) -> Vec<&str> {
        let rules = BreakRules {
            strictness,
            chinese_or_japanese,
            ..BreakRules::default()
        };
        let mut out = Vec::new();
        let mut start = 0;
        for at in line_breaks(text, rules).into_iter().chain([text.len()]) {
            out.push(&text[start..at]);
            start = at;
        }
        out
    }

    #[test]
    fn grapheme_clusters_keep_their_marks() {
        assert_eq!(grapheme_boundaries("ab"), [1]);
        // A Tamil letter and its vowel signs are one cluster.
        let tamil = "\u{0ba8}\u{0bbf}\u{0ba8}";
        assert_eq!(grapheme_boundaries(tamil), [6]);
        assert_eq!(grapheme_boundaries("a\r\nb"), [1, 3]);
    }

    #[test]
    fn line_break_is_as_strict_as_it_says() {
        // A small kana starts a line only under `normal` and `loose`.
        let kana = "\u{6587}\u{3041}";
        assert_eq!(with(kana, Strictness::Strict, false), [kana]);
        assert_eq!(
            with(kana, Strictness::Normal, false),
            ["\u{6587}", "\u{3041}"]
        );
        // An iteration mark only under `loose`.
        let mark = "\u{6587}\u{3005}";
        assert_eq!(with(mark, Strictness::Normal, false), [mark]);
        assert_eq!(
            with(mark, Strictness::Loose, false),
            ["\u{6587}", "\u{3005}"]
        );
        // A wave dash under `normal` in Chinese or Japanese alone.
        let dash = "\u{6587}\u{301c}";
        assert_eq!(with(dash, Strictness::Normal, false), [dash]);
        assert_eq!(
            with(dash, Strictness::Normal, true),
            ["\u{6587}", "\u{301c}"]
        );
        // A leader may start a line under `loose`.
        let leader = "\u{6587}\u{2026}";
        assert_eq!(with(leader, Strictness::Normal, false), [leader]);
        assert_eq!(
            with(leader, Strictness::Loose, false),
            ["\u{6587}", "\u{2026}"]
        );
        // A wide prefix may not, in Chinese or Japanese, but under `loose`.
        let euro = "\u{6587}\u{20ac}";
        assert_eq!(
            with(euro, Strictness::Normal, false),
            ["\u{6587}", "\u{20ac}"]
        );
        assert_eq!(with(euro, Strictness::Normal, true), [euro]);
        assert_eq!(with(euro, Strictness::Strict, true), [euro]);
        assert_eq!(
            with(euro, Strictness::Loose, true),
            ["\u{6587}", "\u{20ac}"]
        );
        // `anywhere` between any two characters, ASCII ones too.
        assert_eq!(
            with("ab c", Strictness::Anywhere, false),
            ["a", "b", " ", "c"]
        );
        assert_eq!(with("ab c", Strictness::Loose, false), ["ab ", "c"]);
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
