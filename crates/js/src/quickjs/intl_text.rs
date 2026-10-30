//! `Intl.Segmenter` (ECMA-402 section 18) and the lists of
//! `Intl.DurationFormat` (the Intl.DurationFormat proposal), as natives
//! of the bootstrap's `native` object: a string's grapheme clusters,
//! words or sentences by `icu_segmenter`'s rules (dictionaries for the
//! languages written without spaces, as the reference browser's ICU
//! breaks them), and a list of units joined by `icu_list`.

use icu_list::options::{ListFormatterOptions, ListLength};
use icu_list::{ListFormatter, ListFormatterPreferences};
use icu_locale::Locale;
use icu_segmenter::options::{SentenceBreakInvariantOptions, WordBreakInvariantOptions};
use icu_segmenter::{GraphemeClusterSegmenter, SentenceSegmenter, WordSegmenter};
use rquickjs::{Ctx, Function, Object};
use writeable::Writeable;

/// The boundaries of `text` at `granularity` ("grapheme", "word" or
/// "sentence") in UTF-16 units, each after the first twice its offset,
/// plus one where the segment it ends is word-like (a word's).
pub(super) fn segment(granularity: &str, text: &str) -> Vec<f64> {
    // UTF-8 offsets to UTF-16 ones, walking forward with the boundaries.
    let mut chars = text.char_indices().peekable();
    let mut units = 0usize;
    let mut to_utf16 = |offset: usize| -> usize {
        while let Some(&(at, c)) = chars.peek() {
            if at >= offset {
                break;
            }
            units += c.len_utf16();
            chars.next();
        }
        units
    };
    let mut out = Vec::new();
    match granularity {
        "word" => {
            let segmenter = WordSegmenter::new_dictionary(WordBreakInvariantOptions::default());
            let mut breaks = segmenter.segment_str(text);
            let mut start = 0;
            while let Some(offset) = breaks.next() {
                let like = offset > 0 && breaks.is_word_like();
                if like {
                    for (at, end) in chromium_word_stops(&text[start..offset]) {
                        out.push((to_utf16(start + at) * 2 + 1) as f64);
                        out.push((to_utf16(start + end) * 2) as f64);
                    }
                }
                out.push((to_utf16(offset) * 2 + usize::from(like)) as f64);
                start = offset;
            }
        }
        "sentence" => {
            let segmenter = SentenceSegmenter::new(SentenceBreakInvariantOptions::default());
            for offset in segmenter.segment_str(text) {
                out.push((to_utf16(offset) * 2) as f64);
            }
        }
        _ => {
            let segmenter = GraphemeClusterSegmenter::new();
            for offset in segmenter.segment_str(text) {
                out.push((to_utf16(offset) * 2) as f64);
            }
        }
    }
    out
}

/// The marks inside a word that the reference browser's ICU breaks
/// around, by their byte ranges in `word`: Chromium's word rules take
/// the full stops (`.`, U+FF0E) out of MidNumLet, keeping them between
/// digits, and the colons (`:`, U+FE55, U+FF1A) out of MidLetter, so
/// that `a.b` and `a:b` are three segments where UAX #29 makes one.
fn chromium_word_stops(word: &str) -> Vec<(usize, usize)> {
    let chars: Vec<(usize, char)> = word.char_indices().collect();
    let mut stops = Vec::new();
    for (i, &(at, c)) in chars.iter().enumerate() {
        let between_digits = || {
            let digit = |j: Option<&(usize, char)>| j.is_some_and(|(_, c)| c.is_numeric());
            i > 0 && digit(chars.get(i - 1)) && digit(chars.get(i + 1))
        };
        let stop = match c {
            ':' | '\u{fe55}' | '\u{ff1a}' => true,
            '.' | '\u{ff0e}' => !between_digits(),
            _ => false,
        };
        if stop && i > 0 && i + 1 < chars.len() {
            stops.push((at, at + c.len_utf8()));
        }
    }
    stops
}

/// `items` joined as a list of units in the locale `tag` at `width`
/// ("long", "short" or "narrow"), as pairs of part type ("element" or
/// "literal") and text.
pub(super) fn unit_list(tag: &str, width: &str, items: &[String]) -> Vec<String> {
    let preferences = Locale::try_from_str(tag)
        .map(|locale| ListFormatterPreferences::from(&locale))
        .unwrap_or_default();
    let length = match width {
        "narrow" => ListLength::Narrow,
        "short" => ListLength::Short,
        _ => ListLength::Wide,
    };
    let Ok(formatter) = ListFormatter::try_new_unit(
        preferences,
        ListFormatterOptions::default().with_length(length),
    ) else {
        let mut out = Vec::new();
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                out.extend(["literal".to_owned(), ", ".to_owned()]);
            }
            out.extend(["element".to_owned(), item.clone()]);
        }
        return out;
    };
    // Each item marked apart, so that the literals between them show.
    let marks: Vec<String> = (0..items.len())
        .map(|i| format!("\u{e000}{i}\u{e001}"))
        .collect();
    let joined = formatter
        .format(marks.iter().map(String::as_str))
        .write_to_string()
        .into_owned();
    let mut out = Vec::new();
    let mut rest = joined.as_str();
    while let Some(start) = rest.find('\u{e000}') {
        if start > 0 {
            out.extend(["literal".to_owned(), rest[..start].to_owned()]);
        }
        let after = &rest[start + '\u{e000}'.len_utf8()..];
        let Some(end) = after.find('\u{e001}') else {
            break;
        };
        let index: usize = after[..end].parse().unwrap_or(0);
        out.extend([
            "element".to_owned(),
            items.get(index).cloned().unwrap_or_default(),
        ]);
        rest = &after[end + '\u{e001}'.len_utf8()..];
    }
    if !rest.is_empty() {
        out.extend(["literal".to_owned(), rest.to_owned()]);
    }
    out
}

pub(super) fn install<'js>(ctx: &Ctx<'js>, natives: &Object<'js>) -> rquickjs::Result<()> {
    natives.set(
        "intlSegment",
        Function::new(ctx.clone(), |granularity: String, text: String| {
            segment(&granularity, &text)
        })?,
    )?;
    natives.set(
        "intlUnitList",
        Function::new(
            ctx.clone(),
            |tag: String, width: String, items: Vec<String>| unit_list(&tag, &width, &items),
        )?,
    )?;
    natives.set(
        "intlDisplayName",
        Function::new(
            ctx.clone(),
            |tag: String,
             kind: String,
             style: String,
             standard: bool,
             substitute: bool,
             code: String| {
                super::intl_names::display_name(&tag, &kind, &style, standard, substitute, &code)
            },
        )?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Segments as `text[start..end]` and the word-like flag.
    fn segments(granularity: &str, text: &str) -> Vec<(String, bool)> {
        let units: Vec<u16> = text.encode_utf16().collect();
        let marks = segment(granularity, text);
        marks
            .windows(2)
            .map(|pair| {
                let (start, end) = ((pair[0] as usize) / 2, (pair[1] as usize) / 2);
                (
                    String::from_utf16_lossy(&units[start..end]),
                    pair[1] as usize % 2 == 1,
                )
            })
            .collect()
    }

    #[test]
    fn words_sentences_and_clusters_break_as_the_reference_breaks_them() {
        // The reference browser's (Chrome 156) segments of the same text.
        let words = segments("word", "Hello, world! 3.14 is pi. Don't stop");
        let texts: Vec<&str> = words.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Hello", ",", " ", "world", "!", " ", "3.14", " ", "is", " ", "pi", ".", " ",
                "Don't", " ", "stop"
            ]
        );
        assert!(words[0].1 && !words[1].1 && words[6].1);
        let japanese = segments(
            "word",
            "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}\u{4e16}\u{754c}\u{3002}",
        );
        let texts: Vec<&str> = japanese.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(
            texts,
            [
                "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}",
                "\u{4e16}\u{754c}",
                "\u{3002}"
            ]
        );
        let sentences = segments(
            "sentence",
            "Hello, world! 3.14 is pi. Mr. Smith went home.  Yes? No...",
        );
        let texts: Vec<&str> = sentences.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Hello, world! ",
                "3.14 is pi. ",
                "Mr. ",
                "Smith went home.  ",
                "Yes? ",
                "No..."
            ]
        );
        let clusters = segment(
            "grapheme",
            "e\u{301}\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{1f1eb}\u{1f1f7}a\r\nb",
        );
        let starts: Vec<usize> = clusters.iter().map(|m| *m as usize / 2).collect();
        assert_eq!(starts, [0, 2, 10, 14, 15, 17, 18]);
    }

    #[test]
    fn units_are_listed_with_the_locales_literals() {
        let items = ["1 hr".to_owned(), "2 min".to_owned(), "3 sec".to_owned()];
        assert_eq!(
            unit_list("en", "short", &items),
            [
                "element", "1 hr", "literal", ", ", "element", "2 min", "literal", ", ", "element",
                "3 sec"
            ]
        );
        let items = ["1 hour".to_owned(), "2 minutes".to_owned()];
        assert_eq!(
            unit_list("en", "long", &items),
            ["element", "1 hour", "literal", ", ", "element", "2 minutes"]
        );
    }
}
