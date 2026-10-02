//! Language tags for `:lang()` (M16): which tags and ranges are
//! well-formed by BCP 47's syntax (RFC 5646 §2.1), RFC 4647's extended
//! filtering of a tag by a range (§3.3.2), and HTML's pragma-set
//! default language from a `<meta http-equiv="content-language">`.
//!
//! Selectors 4 asks for tags and ranges to be canonicalised, put in
//! extlang form and checked against the IANA registry before they are
//! compared; this does none of that (docs/design/selectors-pseudo.md),
//! so `:lang(zh-yue)` and a page's `lang=yue` do not match, and
//! `lang=qq-ZZ` is taken as a tag. Every comparison is ASCII
//! case-insensitive.

/// The tags RFC 5646 keeps whole, which its syntax would not otherwise
/// allow ("irregular") or would read differently ("regular").
const GRANDFATHERED: [&str; 26] = [
    "en-GB-oed",
    "i-ami",
    "i-bnn",
    "i-default",
    "i-enochian",
    "i-hak",
    "i-klingon",
    "i-lux",
    "i-mingo",
    "i-navajo",
    "i-pwn",
    "i-tao",
    "i-tay",
    "i-tsu",
    "sgn-BE-FR",
    "sgn-BE-NL",
    "sgn-CH-DE",
    "art-lojban",
    "cel-gaulish",
    "no-bok",
    "no-nyn",
    "zh-guoyu",
    "zh-hakka",
    "zh-min",
    "zh-min-nan",
    "zh-xiang",
];

fn alpha(subtag: &str) -> bool {
    subtag.bytes().all(|b| b.is_ascii_alphabetic())
}

fn alphanum(subtag: &str) -> bool {
    subtag.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn digits(subtag: &str) -> bool {
    subtag.bytes().all(|b| b.is_ascii_digit())
}

/// Whether `tag` is a well-formed language tag (RFC 5646 §2.1):
/// `language` (2–3 letters with up to three extlangs, or 4–8 letters),
/// then an optional script, region, any variants, any extensions and a
/// private-use part; or a private-use tag alone; or one of the
/// grandfathered tags.
pub(crate) fn is_well_formed_tag(tag: &str) -> bool {
    if GRANDFATHERED.iter().any(|g| g.eq_ignore_ascii_case(tag)) {
        return true;
    }
    let mut subtags = tag.split('-').peekable();
    let Some(language) = subtags.next() else {
        return false;
    };
    if language.eq_ignore_ascii_case("x") {
        return private_use(subtags);
    }
    if !(2..=8).contains(&language.len()) || !alpha(language) {
        return false;
    }
    let mut next_if = |test: &dyn Fn(&str) -> bool| subtags.next_if(|s| test(s)).is_some();
    // `extlang`, after a two- or three-letter language only: three
    // letters, which nothing else after a language is.
    if language.len() <= 3 {
        for _ in 0..3 {
            if !next_if(&|s| s.len() == 3 && alpha(s)) {
                break;
            }
        }
    }
    // `script`, then `region`.
    next_if(&|s| s.len() == 4 && alpha(s));
    next_if(&|s| (s.len() == 2 && alpha(s)) || (s.len() == 3 && digits(s)));
    // `variant`: 5–8 letters and digits, or a digit and three more.
    while next_if(&|s| {
        ((5..=8).contains(&s.len()) && alphanum(s))
            || (s.len() == 4 && s.as_bytes()[0].is_ascii_digit() && alphanum(s))
    }) {}
    // `extension`: a singleton other than `x`, then one or more
    // subtags of 2–8 letters and digits.
    while next_if(&|s| s.len() == 1 && alphanum(s) && !s.eq_ignore_ascii_case("x")) {
        let mut any = false;
        while next_if(&|s| (2..=8).contains(&s.len()) && alphanum(s)) {
            any = true;
        }
        if !any {
            return false;
        }
    }
    if next_if(&|s| s.eq_ignore_ascii_case("x")) {
        return private_use(subtags);
    }
    subtags.next().is_none()
}

/// The subtags after a private-use `x`: one or more, each 1–8 letters
/// and digits.
fn private_use<'a>(subtags: impl Iterator<Item = &'a str>) -> bool {
    let mut any = false;
    for subtag in subtags {
        if !(1..=8).contains(&subtag.len()) || !alphanum(subtag) {
            return false;
        }
        any = true;
    }
    any
}

/// Whether `range` is a language range `:lang()` can match with
/// (Selectors 4 §7.2): the empty range, or one that would be a
/// well-formed tag were an initial `*` replaced by a valid subtag. A
/// `*` anywhere else makes it match nothing.
pub(crate) fn is_well_formed_range(range: &str) -> bool {
    if range.is_empty() || range == "*" {
        return true;
    }
    match range.strip_prefix("*-") {
        // "en" stands for any primary language subtag.
        Some(rest) => is_well_formed_tag(&format!("en-{rest}")),
        None => is_well_formed_tag(range),
    }
}

/// RFC 4647 §3.3.2's extended filtering: whether the well-formed,
/// non-empty `range` matches the well-formed, non-empty `tag`. The first
/// subtags must be equal, or the range's `*`; each later subtag of the
/// range must then be found in order in the tag, which may skip
/// subtags to reach it, but never a singleton. Ranges here have no `*`
/// after the first subtag (see [`is_well_formed_range`]).
pub(crate) fn extended_filter(range: &str, tag: &str) -> bool {
    let mut range = range.split('-');
    let mut tag = tag.split('-');
    let (Some(first_range), Some(first_tag)) = (range.next(), tag.next()) else {
        return false;
    };
    if first_range != "*" && !first_range.eq_ignore_ascii_case(first_tag) {
        return false;
    }
    'range: for wanted in range {
        for subtag in tag.by_ref() {
            if subtag.eq_ignore_ascii_case(wanted) {
                continue 'range;
            }
            if subtag.len() == 1 {
                return false;
            }
        }
        return false;
    }
    true
}

/// HTML's pragma-set default language from a
/// `<meta http-equiv="content-language">` whose `content` is `content`
/// (HTML §4.2.5.3): none when it is empty or holds a comma; else its
/// first run of characters other than ASCII white space, if any.
pub(crate) fn pragma_language(content: &str) -> Option<&str> {
    if content.contains(',') {
        return None;
    }
    content.split_ascii_whitespace().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_formed_tags_follow_rfc_5646() {
        for tag in [
            "en",
            "EN-us",
            "de-CH-1901",
            "sl-rozaj-biske",
            "sr-Latn-RS",
            "zh-Hant-TW",
            "es-419",
            "zh-yue-HK",
            "zh-cmn-Hans-CN",
            "abc-def-ghi-jkl",
            "en-a-bbb-x-a-ccc",
            "en-u-ca-gregory-t-ja",
            "x-whatever",
            "X-1-22-333",
            "qq",
            "und",
            "abcdefgh",
            "en-GB-oed",
            "I-KLINGON",
            "zh-min-nan",
            "de-x-DE",
            "en-US-x-twain",
        ] {
            assert!(is_well_formed_tag(tag), "{tag}");
        }
        for tag in [
            "",
            "e",
            "abcdefghi",
            "åå",
            "en_US",
            "en--US",
            "en-",
            "-en",
            "en-US-",
            "en-a",
            "en-a-x-y",
            "en-a-b",
            "x",
            "x-",
            "x-abcdefghi",
            "en-x",
            "abc-def-ghi-jkl-mno",
            "abcd-def",
            "en-Latn-Latn",
            "en-US-US",
            "en-123456789",
            "en-a1b",
            "1en",
            "en-ä",
            "*",
            "en-*",
        ] {
            assert!(!is_well_formed_tag(tag), "{tag}");
        }
    }

    #[test]
    fn a_range_may_start_with_a_wildcard_and_nowhere_else() {
        for range in ["", "*", "*-CH", "*-Latn", "de", "de-DE", "x-a", "*-x-a"] {
            assert!(is_well_formed_range(range), "{range:?}");
        }
        for range in ["de-*-DE", "de-*", "*-*", "**", "*CH", "åå", "-", "de-"] {
            assert!(!is_well_formed_range(range), "{range:?}");
        }
    }

    #[test]
    fn extended_filtering_follows_rfc_4647_and_selectors_4() {
        // Selectors 4's example: `de-DE` matches these and not `de`.
        for tag in [
            "de-DE",
            "de-de",
            "de-DE-1996",
            "de-Latn-DE",
            "de-Latf-DE",
            "de-Latn-DE-1996",
        ] {
            assert!(extended_filter("de-DE", tag), "{tag}");
        }
        for tag in [
            "de",
            "de-Deva",
            "de-x-DE",
            "de-Latn-x-DE",
            "en-DE",
            "dex-DE",
        ] {
            assert!(!extended_filter("de-DE", tag), "{tag}");
        }
        // A singleton is never skipped, but a range may name it.
        assert!(extended_filter("de-x-DE", "de-x-DE"));
        assert!(extended_filter("en-a-bbb", "en-US-a-bbb-ccc"));
        assert!(!extended_filter("en-bbb", "en-US-a-bbb"));
        // The primary language subtag by wildcard.
        for tag in ["de-CH", "it-CH", "fr-CH", "rm-CH", "gsw-Latn-CH"] {
            assert!(extended_filter("*-CH", tag), "{tag}");
        }
        assert!(!extended_filter("*-CH", "de-DE"));
        assert!(extended_filter("*", "und"));
        assert!(extended_filter("*", "x-private"));
        assert!(extended_filter("EN", "en-gb"));
        assert!(!extended_filter("en-GB", "en"));
        // Order counts.
        assert!(!extended_filter("de-DE-Latn", "de-Latn-DE"));
    }

    #[test]
    fn the_pragma_takes_one_tag() {
        assert_eq!(pragma_language("fr"), Some("fr"));
        assert_eq!(pragma_language(" \t de-AT  "), Some("de-AT"));
        // The first run only: "en" of "en fr".
        assert_eq!(pragma_language("en fr"), Some("en"));
        assert_eq!(pragma_language("en, fr"), None);
        assert_eq!(pragma_language(""), None);
        assert_eq!(pragma_language("   "), None);
        // Not ASCII white space: part of the tag, which is ill-formed.
        assert_eq!(pragma_language("\u{a0}fr"), Some("\u{a0}fr"));
    }
}
