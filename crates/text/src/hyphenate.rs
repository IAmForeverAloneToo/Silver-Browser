//! Hyphenation by patterns (`hyphens: auto`, CSS Text 3 section 5.4,
//! gap batch D's text gaps): where a word of a language with patterns
//! may be hyphenated, by `hypher`'s Liang patterns.

/// The limits `hyphenate-limit-chars` sets: the fewest characters a
/// word, and the parts before and after a hyphen, may have; `None` for
/// `auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HyphenLimits {
    pub word: Option<u32>,
    pub before: Option<u32>,
    pub after: Option<u32>,
}

/// A word's fewest characters for `auto`, as the reference browser has
/// it.
const AUTO_WORD: usize = 5;

/// The pattern language of `language`, a BCP 47 tag: its primary
/// subtag's, where it is one of two letters `hypher` has.
fn language_of(language: &str) -> Option<hypher::Lang> {
    let primary = language.split(['-', '_']).next()?.as_bytes();
    let [a, b] = primary else {
        return None;
    };
    hypher::Lang::from_iso([a.to_ascii_lowercase(), b.to_ascii_lowercase()])
}

/// Whether words in `language` are hyphenated by patterns here.
pub fn hyphenates(language: &str) -> bool {
    language_of(language).is_some()
}

/// The byte offsets in `word`, a run of letters, where `hyphens: auto`
/// may hyphenate it in `language`, within `limits`: none for a language
/// without patterns or a word too short.
pub fn hyphenation_points(word: &str, language: &str, limits: HyphenLimits) -> Vec<usize> {
    let Some(lang) = language_of(language) else {
        return Vec::new();
    };
    let chars = word.chars().count();
    let fewest = |limit: Option<u32>, auto: usize| limit.map_or(auto, |n| n as usize);
    if chars < fewest(limits.word, AUTO_WORD).max(1) {
        return Vec::new();
    }
    let (left, right) = lang.bounds();
    let before = fewest(limits.before, left).max(1);
    let after = fewest(limits.after, right).max(1);
    if before + after > chars {
        return Vec::new();
    }
    let mut at = 0;
    let mut points = Vec::new();
    for syllable in hypher::hyphenate_bounded(word, lang, before, after) {
        if at > 0 {
            points.push(at);
        }
        at += syllable.len();
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_hyphenate_by_their_languages_patterns_within_the_limits() {
        let auto = HyphenLimits::default();
        assert_eq!(hyphenation_points("extensive", "en", auto), [2, 5]);
        assert_eq!(hyphenation_points("extensive", "en-US", auto), [2, 5]);
        assert!(hyphenation_points("extensive", "", auto).is_empty());
        assert!(hyphenation_points("extensive", "xx", auto).is_empty());
        // Too short a word, and limits of the parts.
        assert!(
            hyphenation_points(
                "exten",
                "en",
                HyphenLimits {
                    word: Some(6),
                    ..auto
                }
            )
            .is_empty()
        );
        let wide = HyphenLimits {
            before: Some(3),
            ..auto
        };
        assert_eq!(hyphenation_points("extensive", "en", wide), [5]);
        assert!(hyphenates("de-CH") && !hyphenates("ja"));
    }
}
