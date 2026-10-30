//! `<urange>` (CSS Syntax 3 section 7.1, `unicode-range`): read from the
//! text its tokens were made of, since a number's token keeps its value
//! and not how it was written (`U+00E9` is a number with an exponent).
//! A range read is kept in its declaration as an identifier, `U+E9` or
//! `U+0-7F`, which is also how the reference browser writes it back.

use crate::Token;

/// The highest code point.
const MAX_CODE_POINT: u32 = 0x10_FFFF;

/// One `<urange>` written as `text`: its first and last code points, or
/// `None` for text that is not one.
pub fn read(text: &str) -> Option<(u32, u32)> {
    let rest = text.strip_prefix(['u', 'U'])?.strip_prefix('+')?;
    let digits = rest.chars().take_while(char::is_ascii_hexdigit).count();
    let marks = rest[digits..].chars().take_while(|&c| c == '?').count();
    if digits + marks == 0 || digits + marks > 6 {
        return None;
    }
    let hex = &rest[..digits];
    let after = &rest[digits + marks..];
    let (start, end) = if marks > 0 {
        if !after.is_empty() {
            return None;
        }
        let low = format!("{hex}{}", "0".repeat(marks));
        let high = format!("{hex}{}", "F".repeat(marks));
        (
            u32::from_str_radix(&low, 16).ok()?,
            u32::from_str_radix(&high, 16).ok()?,
        )
    } else {
        let start = u32::from_str_radix(hex, 16).ok()?;
        if after.is_empty() {
            (start, start)
        } else {
            let end = after.strip_prefix('-')?;
            if end.is_empty() || end.len() > 6 || !end.chars().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            (start, u32::from_str_radix(end, 16).ok()?)
        }
    };
    (end <= MAX_CODE_POINT && start <= end).then_some((start, end))
}

/// Ranges separated by commas, each with white space around it allowed;
/// `None` when any is not one.
pub fn read_list(text: &str) -> Option<Vec<(u32, u32)>> {
    text.split(',')
        .map(|part| read(part.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n'))))
        .collect()
}

/// A range as the reference browser writes it: `U+E9`, or `U+0-7F`.
pub fn write((start, end): (u32, u32)) -> String {
    if start == end {
        format!("U+{start:X}")
    } else {
        format!("U+{start:X}-{end:X}")
    }
}

/// Ranges as a declaration keeps them: an identifier each, written as
/// [`write`] has it, separated by commas.
pub fn tokens(ranges: &[(u32, u32)]) -> Vec<Token> {
    let mut out = Vec::new();
    for (i, range) in ranges.iter().enumerate() {
        if i > 0 {
            out.push(Token::Comma);
            out.push(Token::Whitespace);
        }
        out.push(Token::Ident(write(*range)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_read_from_their_text() {
        assert_eq!(read("U+00E9"), Some((0xE9, 0xE9)));
        assert_eq!(read("u+0-7f"), Some((0, 0x7F)));
        assert_eq!(read("U+0025-00FF"), Some((0x25, 0xFF)));
        assert_eq!(read("U+4??"), Some((0x400, 0x4FF)));
        assert_eq!(read("U+??????"), None);
        assert_eq!(read("U+?????"), Some((0, 0xF_FFFF)));
        assert_eq!(read("U+10FFFF"), Some((0x10_FFFF, 0x10_FFFF)));
        assert_eq!(read("U+1E3"), Some((0x1E3, 0x1E3)));
        for bad in [
            "U+",
            "U+110000",
            "U+1234567",
            "U+7F-0",
            "U+1?2",
            "U+1?-2",
            "U+ 1",
            "U+1 - 2",
            "V+1",
            "U1",
            "U+1-",
            "U+g",
        ] {
            assert_eq!(read(bad), None, "{bad}");
        }
        assert_eq!(
            read_list("U+0-7F, u+e9 ,U+4??"),
            Some(vec![(0, 0x7F), (0xE9, 0xE9), (0x400, 0x4FF)])
        );
        assert_eq!(read_list("U+0-7F, junk"), None);
        assert_eq!(write((0xE9, 0xE9)), "U+E9");
        assert_eq!(write((0, 0x7F)), "U+0-7F");
    }

    #[test]
    fn a_declaration_keeps_its_ranges_as_written_back() {
        let sheet = crate::parse_stylesheet(
            "@font-face { unicode-range: U+00E9, u+0025-00ff !important; src: url(a) }",
        )
        .expect("never fails");
        let crate::Rule::At(at) = &sheet.rules[0] else {
            panic!("an at-rule");
        };
        let Some(crate::Block::Declarations(list)) = &at.block else {
            panic!("declarations");
        };
        assert_eq!(
            list[0].value,
            [
                Token::Ident("U+E9".into()),
                Token::Comma,
                Token::Whitespace,
                Token::Ident("U+25-FF".into())
            ]
        );
        assert!(list[0].important);
        assert_eq!(list[1].name, "src");
        // What is no range keeps its tokens, which say nothing of one.
        let sheet = crate::parse_stylesheet("@font-face { unicode-range: U+00E9-1 }").unwrap();
        let crate::Rule::At(at) = &sheet.rules[0] else {
            panic!("an at-rule");
        };
        let Some(crate::Block::Declarations(list)) = &at.block else {
            panic!("declarations");
        };
        assert!(
            list[0]
                .value
                .iter()
                .all(|t| !matches!(t, Token::Ident(i) if i.starts_with("U+")))
        );
    }
}
