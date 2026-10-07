//! HTML's common microsyntaxes (HTML §2.3) that more than one crate
//! reads attributes with.

/// An attribute's value by HTML's rules for parsing non-negative
/// integers (§2.3.4.2): leading ASCII white space and a `+` are
/// skipped, the digits that follow are the number and whatever follows
/// them is ignored; no digit, or a negative number, is an error
/// (`None`). A number too large for a `u64` saturates.
pub fn parse_non_negative_integer(value: &str) -> Option<u64> {
    let s = value.trim_start_matches([' ', '\t', '\n', '\r', '\x0c']);
    let (negative, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let value = s[..digits].parse::<u64>().unwrap_or(u64::MAX);
    if negative && value != 0 {
        return None;
    }
    Some(value)
}

/// An attribute's value by HTML's rules for parsing integers
/// (§2.3.4.1): leading ASCII white space is skipped, then a `-` or `+`,
/// and the digits that follow are the number, whatever follows them
/// ignored; no digit is an error (`None`). A number too large for an
/// `i64` saturates.
pub fn parse_integer(value: &str) -> Option<i64> {
    let s = value.trim_start_matches([' ', '\t', '\n', '\r', '\x0c']);
    let (negative, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let magnitude = s[..digits].parse::<i64>().unwrap_or(i64::MAX);
    Some(if negative { -magnitude } else { magnitude })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_parse_as_html_says() {
        assert_eq!(parse_integer("  12abc"), Some(12));
        assert_eq!(parse_integer("\t-7"), Some(-7));
        assert_eq!(parse_integer("+3"), Some(3));
        assert_eq!(parse_integer("-0"), Some(0));
        assert_eq!(parse_integer("abc"), None);
        assert_eq!(parse_integer("-"), None);
        assert_eq!(parse_integer("--1"), None);
        assert_eq!(parse_integer("99999999999999999999999"), Some(i64::MAX));
        assert_eq!(parse_integer("-99999999999999999999999"), Some(-i64::MAX));
    }

    #[test]
    fn non_negative_integers_parse_as_html_says() {
        assert_eq!(parse_non_negative_integer("  12abc"), Some(12));
        assert_eq!(parse_non_negative_integer("\t\n+7"), Some(7));
        assert_eq!(parse_non_negative_integer("abc"), None);
        assert_eq!(parse_non_negative_integer(""), None);
        assert_eq!(parse_non_negative_integer("-3"), None);
        assert_eq!(parse_non_negative_integer("-0"), Some(0));
        assert_eq!(parse_non_negative_integer("+"), None);
        assert_eq!(parse_non_negative_integer("\u{a0}1"), None);
        assert_eq!(
            parse_non_negative_integer("99999999999999999999999"),
            Some(u64::MAX)
        );
    }
}
