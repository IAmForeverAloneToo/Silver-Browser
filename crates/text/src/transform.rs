//! `text-transform` (CSS Text 3 section 2.1, gap batch D's text gaps):
//! case mapping tailored by the content language (Turkish and Azeri
//! dotted and dotless i, Greek uppercase without accents, Dutch IJ),
//! titlecase for `capitalize`, and the `full-width` and
//! `full-size-kana` mappings.

/// The case `text-transform` maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Case {
    #[default]
    None,
    Upper,
    Lower,
    /// The first typographic letter unit of each word in titlecase.
    Capitalize,
}

/// A `text-transform` to apply: a case, the width and kana mappings,
/// and the content language that tailors them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Transform<'a> {
    pub case: Case,
    pub full_width: bool,
    pub full_size_kana: bool,
    /// The content language, lowercase, `""` when unknown.
    pub language: &'a str,
}

impl Transform<'_> {
    /// Whether it changes nothing.
    pub fn is_none(&self) -> bool {
        self.case == Case::None && !self.full_width && !self.full_size_kana
    }
}

/// Whether `language` (a BCP 47 tag, lowercase) is `code` or one of its
/// variants, in the script its tailoring is of: a script subtag naming
/// another overrides the language (`tr-Cyrl` is no Turkish to case).
fn is_language(language: &str, code: &str) -> bool {
    let Some(rest) = language.strip_prefix(code) else {
        return false;
    };
    if !(rest.is_empty() || rest.starts_with('-')) {
        return false;
    }
    let script = if code == "el" { "grek" } else { "latn" };
    rest.split('-')
        .filter(|subtag| subtag.len() == 4 && subtag.chars().all(|c| c.is_ascii_alphabetic()))
        .all(|subtag| subtag.eq_ignore_ascii_case(script))
}

/// `text` transformed. `at_word_start` says whether the text starts a
/// word, the text before it, in another box, having ended one; it is
/// left saying whether the text after it would.
pub fn transform(text: &str, how: Transform<'_>, at_word_start: &mut bool) -> String {
    let turkic = is_language(how.language, "tr") || is_language(how.language, "az");
    let mut out = match how.case {
        Case::None => text.to_owned(),
        Case::Upper if is_language(how.language, "el") => greek_upper(text),
        Case::Upper if turkic => text.replace('i', "\u{130}").to_uppercase(),
        Case::Upper => text.to_uppercase(),
        Case::Lower if turkic => text
            .replace('I', "\u{131}")
            .replace('\u{130}', "i")
            .to_lowercase(),
        Case::Lower => text.to_lowercase(),
        Case::Capitalize => capitalize(text, how.language, at_word_start),
    };
    if how.case != Case::Capitalize {
        *at_word_start = ends_word(text, *at_word_start);
    }
    if how.full_size_kana {
        out = out.chars().map(full_size_kana).collect();
    }
    if how.full_width {
        out = out.chars().map(full_width).collect();
    }
    out
}

/// Whether a word ends with `text`: its last character is none a word
/// goes on through.
pub fn ends_word(text: &str, at_word_start: bool) -> bool {
    text.chars().next_back().map_or(at_word_start, |c| {
        !(c.is_alphanumeric() || c == '_' || is_mark(c))
    })
}

/// The first letter of each word in titlecase: a word starts at a letter
/// or digit after anything but a letter, a digit, an underscore, or an
/// apostrophe or a stop between two letters (`john's`, `e.g`).
fn capitalize(text: &str, language: &str, at_word_start: &mut bool) -> String {
    let mut out = String::with_capacity(text.len());
    capitalize_into(text, language, at_word_start, &mut out);
    out
}

fn capitalize_into(text: &str, language: &str, at_word_start: &mut bool, out: &mut String) {
    let dutch = is_language(language, "nl");
    let turkic = is_language(language, "tr") || is_language(language, "az");
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let joins = matches!(c, '\'' | '\u{2019}' | '.' | ':' | '\u{b7}')
            && i > 0
            && chars[i - 1].is_alphabetic()
            && chars.get(i + 1).is_some_and(|n| n.is_alphabetic());
        if c.is_alphanumeric() {
            if *at_word_start && c.is_alphabetic() {
                if dutch && c == 'i' && chars.get(i + 1) == Some(&'j') {
                    out.push_str("IJ");
                    i += 2;
                    *at_word_start = false;
                    continue;
                }
                if turkic && c == 'i' {
                    out.push('\u{130}');
                } else {
                    out.push_str(&titlecase(c));
                }
            } else {
                out.push(c);
            }
            *at_word_start = false;
        } else if is_mark(c) || c == '_' || joins {
            out.push(c);
        } else {
            out.push(c);
            *at_word_start = true;
        }
        i += 1;
    }
}

/// Whether `c` is a combining mark, which belongs to the letter before.
fn is_mark(c: char) -> bool {
    matches!(c,
        '\u{300}'..='\u{36f}'
        | '\u{483}'..='\u{489}'
        | '\u{591}'..='\u{5bd}'
        | '\u{610}'..='\u{61a}'
        | '\u{64b}'..='\u{65f}'
        | '\u{900}'..='\u{903}'
        | '\u{93a}'..='\u{94f}'
        | '\u{b82}'..='\u{bcd}'
        | '\u{1ab0}'..='\u{1aff}'
        | '\u{1dc0}'..='\u{1dff}'
        | '\u{200c}' | '\u{200d}'
        | '\u{20d0}'..='\u{20ff}'
        | '\u{fe20}'..='\u{fe2f}')
}

/// `c` in titlecase, which is not always its uppercase: a ligature's
/// first letter alone is capital, and the digraphs have forms of their
/// own.
fn titlecase(c: char) -> String {
    match c {
        '\u{fb00}' => "Ff".to_owned(),
        '\u{fb01}' => "Fi".to_owned(),
        '\u{fb02}' => "Fl".to_owned(),
        '\u{fb03}' => "Ffi".to_owned(),
        '\u{fb04}' => "Ffl".to_owned(),
        '\u{fb05}' | '\u{fb06}' => "St".to_owned(),
        '\u{1c4}'..='\u{1c6}' => "\u{1c5}".to_owned(),
        '\u{1c7}'..='\u{1c9}' => "\u{1c8}".to_owned(),
        '\u{1ca}'..='\u{1cc}' => "\u{1cb}".to_owned(),
        '\u{1f1}'..='\u{1f3}' => "\u{1f2}".to_owned(),
        '\u{587}' => "\u{535}\u{582}".to_owned(),
        '\u{df}' => "Ss".to_owned(),
        _ => c.to_uppercase().collect(),
    }
}

/// Greek in uppercase as ICU writes it (CSS Text 3 section 2.1.1's
/// language-sensitive mapping): the accents dropped, but a diaeresis,
/// which a vowel keeps, and which an iota or upsilon after an accented
/// vowel gains, the two being no diphthong; and the disjunctive eta, a
/// word of its own, keeping its accent.
fn greek_upper(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let alone = |i: usize| {
            (i == 0 || !chars[i - 1].is_alphabetic())
                && chars
                    .get(i + 1)
                    .is_none_or(|n| !n.is_alphabetic() && !is_mark(*n))
        };
        // The disjunctive eta.
        if matches!(c, '\u{3ae}' | '\u{389}') && alone(i) {
            out.push('\u{389}');
            i += 1;
            continue;
        }
        let Some((base, accent, diaeresis)) = greek_parts(c) else {
            if is_mark(c)
                && matches!(
                    c,
                    '\u{301}' | '\u{300}' | '\u{342}' | '\u{313}' | '\u{314}' | '\u{345}'
                )
            {
                i += 1;
                continue;
            }
            out.extend(c.to_uppercase());
            i += 1;
            continue;
        };
        let upper = base.to_uppercase().next().unwrap_or(base);
        out.push(if diaeresis {
            with_diaeresis(upper)
        } else {
            upper
        });
        // An iota or upsilon after an accented vowel takes a diaeresis.
        if accent
            && let Some(&next) = chars.get(i + 1)
            && matches!(next, '\u{3b9}' | '\u{3c5}' | '\u{399}' | '\u{3a5}')
        {
            let upper = next.to_uppercase().next().unwrap_or(next);
            out.push(with_diaeresis(upper));
            i += 2;
            continue;
        }
        i += 1;
    }
    out
}

/// A Greek vowel's base letter, and whether it has an accent and a
/// diaeresis; `None` for any other character.
fn greek_parts(c: char) -> Option<(char, bool, bool)> {
    Some(match c {
        '\u{3ac}' | '\u{386}' => ('\u{3b1}', true, false),
        '\u{3ad}' | '\u{388}' => ('\u{3b5}', true, false),
        '\u{3ae}' | '\u{389}' => ('\u{3b7}', true, false),
        '\u{3af}' | '\u{38a}' => ('\u{3b9}', true, false),
        '\u{3cc}' | '\u{38c}' => ('\u{3bf}', true, false),
        '\u{3cd}' | '\u{38e}' => ('\u{3c5}', true, false),
        '\u{3ce}' | '\u{38f}' => ('\u{3c9}', true, false),
        '\u{390}' => ('\u{3b9}', true, true),
        '\u{3b0}' => ('\u{3c5}', true, true),
        '\u{3ca}' | '\u{3aa}' => ('\u{3b9}', false, true),
        '\u{3cb}' | '\u{3ab}' => ('\u{3c5}', false, true),
        '\u{3b1}'..='\u{3c9}' | '\u{391}'..='\u{3a9}' => (c, false, false),
        _ => return None,
    })
}

/// A capital iota or upsilon with a diaeresis; any other letter as it is.
fn with_diaeresis(c: char) -> char {
    match c {
        '\u{399}' => '\u{3aa}',
        '\u{3a5}' => '\u{3ab}',
        other => other,
    }
}

/// `c` at its full size where it is a small kana (CSS Text 3 Appendix
/// G).
fn full_size_kana(c: char) -> char {
    let full = match c {
        '\u{3041}' => 0x3042,
        '\u{3043}' => 0x3044,
        '\u{3045}' => 0x3046,
        '\u{3047}' => 0x3048,
        '\u{3049}' => 0x304a,
        '\u{3063}' => 0x3064,
        '\u{3083}' => 0x3084,
        '\u{3085}' => 0x3086,
        '\u{3087}' => 0x3088,
        '\u{308e}' => 0x308f,
        '\u{3095}' => 0x304b,
        '\u{3096}' => 0x3051,
        '\u{30a1}' => 0x30a2,
        '\u{30a3}' => 0x30a4,
        '\u{30a5}' => 0x30a6,
        '\u{30a7}' => 0x30a8,
        '\u{30a9}' => 0x30aa,
        '\u{30c3}' => 0x30c4,
        '\u{30e3}' => 0x30e4,
        '\u{30e5}' => 0x30e6,
        '\u{30e7}' => 0x30e8,
        '\u{30ee}' => 0x30ef,
        '\u{30f5}' => 0x30ab,
        '\u{30f6}' => 0x30b1,
        '\u{31f0}' => 0x30af,
        '\u{31f1}' => 0x30b7,
        '\u{31f2}' => 0x30b9,
        '\u{31f3}' => 0x30c8,
        '\u{31f4}' => 0x30cc,
        '\u{31f5}' => 0x30cf,
        '\u{31f6}' => 0x30d2,
        '\u{31f7}' => 0x30d5,
        '\u{31f8}' => 0x30d8,
        '\u{31f9}' => 0x30db,
        '\u{31fa}' => 0x30e0,
        '\u{31fb}' => 0x30e9,
        '\u{31fc}' => 0x30ea,
        '\u{31fd}' => 0x30eb,
        '\u{31fe}' => 0x30ec,
        '\u{31ff}' => 0x30ed,
        '\u{1b132}' => 0x3053,
        '\u{1b150}' => 0x3090,
        '\u{1b151}' => 0x3091,
        '\u{1b152}' => 0x3092,
        '\u{1b155}' => 0x30b3,
        '\u{1b164}' => 0x30f0,
        '\u{1b165}' => 0x30f1,
        '\u{1b166}' => 0x30f2,
        '\u{1b167}' => 0x30f3,
        '\u{ff67}' => 0xff71,
        '\u{ff68}' => 0xff72,
        '\u{ff69}' => 0xff73,
        '\u{ff6a}' => 0xff74,
        '\u{ff6b}' => 0xff75,
        '\u{ff6c}' => 0xff94,
        '\u{ff6d}' => 0xff95,
        '\u{ff6e}' => 0xff96,
        '\u{ff6f}' => 0xff82,
        _ => return c,
    };
    char::from_u32(full).unwrap_or(c)
}

/// `c` in its fullwidth form, where it has one: the printable ASCII, the
/// space as the ideographic space, the halfwidth katakana and the other
/// halfwidth forms.
fn full_width(c: char) -> char {
    const KATAKANA: [u16; 59] = [
        0x3002, 0x300c, 0x300d, 0x3001, 0x30fb, 0x30f2, 0x30a1, 0x30a3, 0x30a5, 0x30a7, 0x30a9,
        0x30e3, 0x30e5, 0x30e7, 0x30c3, 0x30fc, 0x30a2, 0x30a4, 0x30a6, 0x30a8, 0x30aa, 0x30ab,
        0x30ad, 0x30af, 0x30b1, 0x30b3, 0x30b5, 0x30b7, 0x30b9, 0x30bb, 0x30bd, 0x30bf, 0x30c1,
        0x30c4, 0x30c6, 0x30c8, 0x30ca, 0x30cb, 0x30cc, 0x30cd, 0x30ce, 0x30cf, 0x30d2, 0x30d5,
        0x30d8, 0x30db, 0x30de, 0x30df, 0x30e0, 0x30e1, 0x30e2, 0x30e4, 0x30e6, 0x30e8, 0x30e9,
        0x30ea, 0x30eb, 0x30ec, 0x30ed,
    ];
    let full = match c {
        ' ' => 0x3000,
        '!'..='~' => c as u32 - 0x21 + 0xff01,
        '\u{ff61}'..='\u{ff9b}' => u32::from(KATAKANA[c as usize - 0xff61]),
        '\u{ff9c}' => 0x30ef,
        '\u{ff9d}' => 0x30f3,
        '\u{ff9e}' => 0x3099,
        '\u{ff9f}' => 0x309a,
        '\u{ffe8}' => 0x2502,
        '\u{ffe9}' => 0x2190,
        '\u{ffea}' => 0x2191,
        '\u{ffeb}' => 0x2192,
        '\u{ffec}' => 0x2193,
        '\u{ffed}' => 0x25a0,
        '\u{ffee}' => 0x25cb,
        _ => return c,
    };
    char::from_u32(full).unwrap_or(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, case: Case, language: &str) -> String {
        let how = Transform {
            case,
            language,
            ..Transform::default()
        };
        transform(text, how, &mut true)
    }

    #[test]
    fn capitalize_finds_each_words_first_letter() {
        assert_eq!(apply("hello world", Case::Capitalize, ""), "Hello World");
        assert_eq!(apply("foo-bar", Case::Capitalize, ""), "Foo-Bar");
        assert_eq!(apply("john's apple", Case::Capitalize, ""), "John's Apple");
        assert_eq!(apply("foo_bar", Case::Capitalize, ""), "Foo_bar");
        assert_eq!(
            apply("({[-\"'.<?!transform", Case::Capitalize, ""),
            "({[-\"'.<?!Transform"
        );
        assert_eq!(apply("3d x", Case::Capitalize, ""), "3d X");
        // Titlecase, not uppercase.
        assert_eq!(apply("\u{fb02}ying", Case::Capitalize, ""), "Flying");
        assert_eq!(apply("\u{1c6}a", Case::Capitalize, ""), "\u{1c5}a");
        // A word goes on into the next box.
        let how = Transform {
            case: Case::Capitalize,
            ..Transform::default()
        };
        let mut start = false;
        assert_eq!(transform("bc d", how, &mut start), "bc D");
        assert!(!start);
        assert_eq!(transform("x ", how, &mut start), "x ");
        assert!(start);
    }

    #[test]
    fn the_language_tailors_the_case() {
        assert_eq!(apply("ijsland", Case::Capitalize, "nl"), "IJsland");
        assert_eq!(apply("ijsland", Case::Capitalize, "en"), "Ijsland");
        assert_eq!(apply("istanbul", Case::Upper, "tr"), "\u{130}STANBUL");
        assert_eq!(apply("I", Case::Lower, "tr-tr"), "\u{131}");
        assert_eq!(apply("I", Case::Lower, "tr-latn"), "\u{131}");
        // A script subtag of another script overrides the language.
        assert_eq!(apply("I", Case::Lower, "tr-cyrl"), "i");
        assert_eq!(apply("i", Case::Upper, "en"), "I");
        assert_eq!(
            apply(
                "\u{3ba}\u{3b1}\u{3bb}\u{3b7}\u{3bc}\u{3ad}\u{3c1}\u{3b1}",
                Case::Upper,
                "el"
            ),
            "\u{39a}\u{391}\u{39b}\u{397}\u{39c}\u{395}\u{3a1}\u{391}"
        );
        // An accented vowel's iota gains a diaeresis: Nerai'da.
        assert_eq!(
            apply(
                "\u{3bd}\u{3b5}\u{3c1}\u{3ac}\u{3b9}\u{3b4}\u{3b1}",
                Case::Upper,
                "el"
            ),
            "\u{39d}\u{395}\u{3a1}\u{391}\u{3aa}\u{394}\u{391}"
        );
        // The disjunctive eta keeps its accent.
        assert_eq!(
            apply("\u{3ae} \u{3b5}", Case::Upper, "el"),
            "\u{389} \u{395}"
        );
    }

    #[test]
    fn full_width_and_full_size_kana() {
        let how = Transform {
            full_width: true,
            full_size_kana: true,
            ..Transform::default()
        };
        assert_eq!(
            transform("a 1\u{3041}\u{ff76}", how, &mut true),
            "\u{ff41}\u{3000}\u{ff11}\u{3042}\u{30ab}"
        );
    }
}
