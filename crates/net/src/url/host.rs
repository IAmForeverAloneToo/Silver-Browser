//! Hosts (URL Standard §3): the host parser with its IPv4, IPv6,
//! opaque-host and domain parsers, and the host serializer.

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

use super::ParseError;
use super::percent::{EncodeSet, percent_decode, utf8_percent_encode};

/// A URL's host (URL Standard §3.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Host {
    /// A domain: ASCII, lowercase, through IDNA if it was not ASCII.
    Domain(String),
    /// An IPv4 address.
    Ipv4(Ipv4Addr),
    /// An IPv6 address.
    Ipv6(Ipv6Addr),
    /// An opaque host: a non-special URL's host, percent-encoded.
    Opaque(String),
    /// The empty host: a `file:` URL's, or a non-special URL's `//`.
    Empty,
}

impl Host {
    /// The host's public suffix (URL Standard 3.2), by the Public Suffix
    /// List (D20), its trailing dot kept: `None` for a host that is not
    /// a domain. A domain the list does not know has its last label.
    pub fn public_suffix(&self) -> Option<&str> {
        let Host::Domain(name) = self else {
            return None;
        };
        std::str::from_utf8(psl::suffix(name.as_bytes())?.as_bytes()).ok()
    }

    /// The host's registrable domain (URL Standard 3.2): its public
    /// suffix and the label before it; `None` for a host that is not a
    /// domain or that is a public suffix itself.
    pub fn registrable_domain(&self) -> Option<&str> {
        let Host::Domain(name) = self else {
            return None;
        };
        std::str::from_utf8(psl::domain(name.as_bytes())?.as_bytes()).ok()
    }

    /// Whether `suffix` is a registrable domain suffix of, or equal to,
    /// the host (HTML 7.1.1.2), as `document.domain`'s setter asks: the
    /// host itself, or a domain the host ends in, label by label, that
    /// is not a public suffix.
    pub fn has_registrable_suffix(&self, suffix: &str) -> bool {
        // The empty string, which HTML refuses first, fails to parse.
        let Ok(parsed) = parse_host(suffix, false) else {
            return false;
        };
        if parsed == *self {
            return true;
        }
        let (Host::Domain(name), Host::Domain(wanted)) = (self, &parsed) else {
            return false;
        };
        let dotted = format!(".{wanted}");
        if !name.ends_with(&dotted) {
            return false;
        }
        let public = self.public_suffix().unwrap_or_default();
        parsed.public_suffix() != Some(wanted.as_str()) && !public.ends_with(&dotted)
    }
}

impl fmt::Display for Host {
    /// The host serializer (URL Standard §3.6).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Host::Domain(name) | Host::Opaque(name) => f.write_str(name),
            Host::Ipv4(address) => write!(f, "{address}"),
            Host::Ipv6(address) => {
                f.write_str("[")?;
                write_ipv6(&address.segments(), f)?;
                f.write_str("]")
            }
            Host::Empty => Ok(()),
        }
    }
}

/// A forbidden host code point (URL Standard §3.2).
pub(crate) fn is_forbidden_host_code_point(c: char) -> bool {
    matches!(
        c,
        '\0' | '\t'
            | '\n'
            | '\r'
            | ' '
            | '#'
            | '/'
            | ':'
            | '<'
            | '>'
            | '?'
            | '@'
            | '['
            | '\\'
            | ']'
            | '^'
            | '|'
    )
}

/// A forbidden domain code point: a forbidden host code point, a C0
/// control, `%` or U+007F DELETE.
fn is_forbidden_domain_code_point(c: char) -> bool {
    is_forbidden_host_code_point(c) || c <= '\u{1F}' || c == '%' || c == '\u{7F}'
}

/// The host parser. `is_opaque` is true for a URL that is not special.
pub(crate) fn parse_host(input: &str, is_opaque: bool) -> Result<Host, ParseError> {
    if let Some(rest) = input.strip_prefix('[') {
        let inner = rest.strip_suffix(']').ok_or(ParseError::InvalidIpv6)?;
        return parse_ipv6(inner).map(Host::Ipv6);
    }
    if is_opaque {
        return parse_opaque_host(input);
    }
    if input.is_empty() {
        return Err(ParseError::EmptyHost);
    }
    let decoded = percent_decode(input.as_bytes());
    let domain = String::from_utf8_lossy(&decoded);
    let ascii = domain_to_ascii(&domain)?;
    if ends_in_a_number(&ascii) {
        return parse_ipv4(&ascii).map(Host::Ipv4);
    }
    Ok(Host::Domain(ascii))
}

/// The domain parser with `beStrict` false (URL Standard §3.3). An
/// ASCII domain is lowercased and taken as it is, whatever Unicode's
/// ToASCII says of it; any other goes through ToASCII, with the URL
/// Standard's options: CheckHyphens, UseSTD3ASCIIRules and
/// VerifyDnsLength off, CheckBidi and CheckJoiners on, nontransitional.
fn domain_to_ascii(domain: &str) -> Result<String, ParseError> {
    let result = if domain.is_ascii() {
        domain.to_ascii_lowercase()
    } else {
        idna::uts46::Uts46::new()
            .to_ascii(
                domain.as_bytes(),
                idna::uts46::AsciiDenyList::EMPTY,
                idna::uts46::Hyphens::Allow,
                idna::uts46::DnsLength::Ignore,
            )
            .map_err(|_| ParseError::InvalidDomain)?
            .into_owned()
    };
    if result.is_empty() || result.chars().any(is_forbidden_domain_code_point) {
        return Err(ParseError::InvalidDomain);
    }
    Ok(result)
}

/// The ends in a number checker: whether the last label (a trailing dot
/// aside) is all digits or parses as an IPv4 number.
fn ends_in_a_number(input: &str) -> bool {
    let mut parts: Vec<&str> = input.split('.').collect();
    if parts.last() == Some(&"") {
        parts.pop();
    }
    let Some(last) = parts.last() else {
        return false;
    };
    if !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    parse_ipv4_number(last).is_some()
}

/// The IPv4 parser: one to four numbers, decimal, octal or hex, the
/// last filling the bytes the others leave.
fn parse_ipv4(input: &str) -> Result<Ipv4Addr, ParseError> {
    let mut parts: Vec<&str> = input.split('.').collect();
    if parts.last() == Some(&"") && parts.len() > 1 {
        parts.pop();
    }
    if parts.len() > 4 {
        return Err(ParseError::InvalidIpv4);
    }
    let numbers: Vec<u64> = parts
        .iter()
        .map(|part| parse_ipv4_number(part).ok_or(ParseError::InvalidIpv4))
        .collect::<Result<_, _>>()?;
    let (last, rest) = numbers.split_last().ok_or(ParseError::InvalidIpv4)?;
    if rest.iter().any(|&n| n > 255) {
        return Err(ParseError::InvalidIpv4);
    }
    // 256 ^ (5 - n): what is left for the last number to fill.
    let room = 1u64 << (8 * (5 - numbers.len()));
    if *last >= room {
        return Err(ParseError::InvalidIpv4);
    }
    let mut ipv4 = *last;
    for (counter, n) in rest.iter().enumerate() {
        ipv4 += n << (8 * (3 - counter));
    }
    Ok(Ipv4Addr::from(ipv4 as u32))
}

/// The IPv4 number parser: decimal, octal after a leading `0`, or hex
/// after `0x`. Values past `u32::MAX` saturate: they fail the IPv4
/// parser either way.
fn parse_ipv4_number(input: &str) -> Option<u64> {
    if input.is_empty() {
        return None;
    }
    let (digits, radix) =
        if input.len() >= 2 && (input.starts_with("0x") || input.starts_with("0X")) {
            (&input[2..], 16)
        } else if input.len() >= 2 && input.starts_with('0') {
            (&input[1..], 8)
        } else {
            (input, 10)
        };
    if digits.is_empty() {
        return Some(0);
    }
    let mut value: u64 = 0;
    for c in digits.chars() {
        let digit = c.to_digit(radix)?;
        value = value
            .saturating_mul(u64::from(radix))
            .saturating_add(u64::from(digit));
    }
    Some(value)
}

/// The IPv6 parser (URL Standard §3.5), its pointer arithmetic kept.
fn parse_ipv6(input: &str) -> Result<Ipv6Addr, ParseError> {
    let input: Vec<char> = input.chars().collect();
    let at = |i: usize| input.get(i).copied();
    let mut address = [0u16; 8];
    let mut piece_index = 0;
    let mut compress: Option<usize> = None;
    let mut pointer = 0;
    let err = Err(ParseError::InvalidIpv6);

    if at(pointer) == Some(':') {
        if at(pointer + 1) != Some(':') {
            return err;
        }
        pointer += 2;
        piece_index += 1;
        compress = Some(piece_index);
    }
    while at(pointer).is_some() {
        if piece_index == 8 {
            return err;
        }
        if at(pointer) == Some(':') {
            if compress.is_some() {
                return err;
            }
            pointer += 1;
            piece_index += 1;
            compress = Some(piece_index);
            continue;
        }
        let mut value: u32 = 0;
        let mut length = 0;
        while length < 4
            && let Some(digit) = at(pointer).and_then(|c| c.to_digit(16))
        {
            value = value * 0x10 + digit;
            pointer += 1;
            length += 1;
        }
        if at(pointer) == Some('.') {
            if length == 0 {
                return err;
            }
            pointer -= length;
            if piece_index > 6 {
                return err;
            }
            let mut numbers_seen = 0;
            while let Some(c) = at(pointer) {
                let mut ipv4_piece: Option<u32> = None;
                if numbers_seen > 0 {
                    if c == '.' && numbers_seen < 4 {
                        pointer += 1;
                    } else {
                        return err;
                    }
                }
                if !at(pointer).is_some_and(|c| c.is_ascii_digit()) {
                    return err;
                }
                while let Some(digit) = at(pointer).and_then(|c| c.to_digit(10)) {
                    ipv4_piece = match ipv4_piece {
                        None => Some(digit),
                        Some(0) => return err,
                        Some(piece) => Some(piece * 10 + digit),
                    };
                    if ipv4_piece > Some(255) {
                        return err;
                    }
                    pointer += 1;
                }
                let piece = ipv4_piece.expect("a digit was read") as u16;
                address[piece_index] = address[piece_index].wrapping_mul(0x100) + piece;
                numbers_seen += 1;
                if numbers_seen == 2 || numbers_seen == 4 {
                    piece_index += 1;
                }
            }
            if numbers_seen != 4 {
                return err;
            }
            break;
        } else if at(pointer) == Some(':') {
            pointer += 1;
            if at(pointer).is_none() {
                return err;
            }
        } else if at(pointer).is_some() {
            return err;
        }
        address[piece_index] = value as u16;
        piece_index += 1;
    }
    if let Some(compress) = compress {
        let mut swaps = piece_index - compress;
        piece_index = 7;
        while piece_index != 0 && swaps > 0 {
            address.swap(piece_index, compress + swaps - 1);
            piece_index -= 1;
            swaps -= 1;
        }
    } else if piece_index != 8 {
        return err;
    }
    Ok(Ipv6Addr::from(address))
}

/// The opaque-host parser: a host of a URL that is not special, refused
/// if it holds a forbidden host code point and otherwise percent-encoded
/// with the C0 control set. An empty one is the empty host.
fn parse_opaque_host(input: &str) -> Result<Host, ParseError> {
    if input.chars().any(is_forbidden_host_code_point) {
        return Err(ParseError::InvalidHost);
    }
    if input.is_empty() {
        return Ok(Host::Empty);
    }
    let mut out = String::with_capacity(input.len());
    utf8_percent_encode(input, EncodeSet::C0Control, &mut out);
    Ok(Host::Opaque(out))
}

/// The IPv6 serializer: lower-case hex pieces, the first longest run of
/// two or more zero pieces written `::` (RFC 5952). std's `Display`
/// writes some addresses with an embedded IPv4 address, which the URL
/// Standard does not.
fn write_ipv6(pieces: &[u16; 8], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let compress = compressed_piece_index(pieces);
    let mut ignore0 = false;
    for (index, &piece) in pieces.iter().enumerate() {
        if ignore0 && piece == 0 {
            continue;
        }
        ignore0 = false;
        if compress == Some(index) {
            f.write_str(if index == 0 { "::" } else { ":" })?;
            ignore0 = true;
            continue;
        }
        write!(f, "{piece:x}")?;
        if index != 7 {
            f.write_str(":")?;
        }
    }
    Ok(())
}

/// Find the IPv6 address compressed piece index: the start of the first
/// longest run of zero pieces, if it is longer than one.
fn compressed_piece_index(pieces: &[u16; 8]) -> Option<usize> {
    let mut longest_index = None;
    let mut longest_size = 1;
    let mut found_index = None;
    let mut found_size = 0;
    for (index, &piece) in pieces.iter().enumerate() {
        if piece != 0 {
            if found_size > longest_size {
                longest_index = found_index;
                longest_size = found_size;
            }
            found_index = None;
            found_size = 0;
        } else {
            if found_index.is_none() {
                found_index = Some(index);
            }
            found_size += 1;
        }
    }
    if found_size > longest_size {
        return found_index;
    }
    longest_index
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(input: &str, is_opaque: bool) -> Result<String, ParseError> {
        parse_host(input, is_opaque).map(|h| h.to_string())
    }

    #[test]
    fn the_standards_round_trips() {
        // URL Standard §3, the table of host parser results.
        // `None` stands for failure.
        let cases: [(&str, Option<&str>, Option<&str>); 13] = [
            ("EXAMPLE.COM", Some("example.com"), Some("EXAMPLE.COM")),
            ("example%2Ecom", Some("example.com"), Some("example%2Ecom")),
            (
                "fa\u{DF}.example",
                Some("xn--fa-hia.example"),
                Some("fa%C3%9F.example"),
            ),
            ("0", Some("0.0.0.0"), Some("0")),
            ("%30", Some("0.0.0.0"), Some("%30")),
            ("0x", Some("0.0.0.0"), Some("0x")),
            ("0xffffffff", Some("255.255.255.255"), Some("0xffffffff")),
            ("[0:0::1]", Some("[::1]"), Some("[::1]")),
            ("[0:0::1%5D", None, None),
            ("[0:0::%31]", None, None),
            ("09", None, Some("09")),
            ("example.255", None, Some("example.255")),
            ("example^example", None, None),
        ];
        for (input, domain, opaque) in cases {
            assert_eq!(host(input, false).ok().as_deref(), domain, "{input:?}");
            assert_eq!(
                host(input, true).ok().as_deref(),
                opaque,
                "{input:?} opaque"
            );
        }
    }

    #[test]
    fn ascii_domains_skip_to_ascii_but_not_the_forbidden_check() {
        assert_eq!(host("xn--", false), Ok("xn--".into()));
        assert_eq!(host("XN--8I7CAA", false), Ok("xn--8i7caa".into()));
        assert_eq!(host("a b", false), Err(ParseError::InvalidDomain));
        assert_eq!(host("a%25b", false), Err(ParseError::InvalidDomain));
        assert_eq!(
            host("\u{2615}.example", false),
            Ok("xn--53h.example".into())
        );
        // A domain that only percent-decoding makes non-ASCII.
        assert_eq!(
            host("%E2%98%95.example", false),
            Ok("xn--53h.example".into())
        );
    }

    #[test]
    fn ipv4_forms() {
        assert_eq!(host("192.168.0.1", false), Ok("192.168.0.1".into()));
        assert_eq!(host("0xC0.0250.1", false), Ok("192.168.0.1".into()));
        assert_eq!(host("3232235521", false), Ok("192.168.0.1".into()));
        assert_eq!(host("1.2.3.4.", false), Ok("1.2.3.4".into()));
        assert_eq!(host("1.2.3.4.5", false), Err(ParseError::InvalidIpv4));
        assert_eq!(host("256.0.0.1", false), Err(ParseError::InvalidIpv4));
        assert_eq!(host("1.2.65536", false), Err(ParseError::InvalidIpv4));
        assert_eq!(host("4294967296", false), Err(ParseError::InvalidIpv4));
        assert_eq!(
            host(&format!("0x{}", "f".repeat(100)), false),
            Err(ParseError::InvalidIpv4)
        );
    }

    #[test]
    fn ipv6_forms() {
        assert_eq!(host("[::]", false), Ok("[::]".into()));
        assert_eq!(host("[1:0:0:2:0:0:0:3]", false), Ok("[1:0:0:2::3]".into()));
        assert_eq!(
            host("[0:f:0:0:f:f:0:0]", false),
            Ok("[0:f::f:f:0:0]".into())
        );
        assert_eq!(
            host("[::ffff:192.168.0.1]", false),
            Ok("[::ffff:c0a8:1]".into())
        );
        assert_eq!(
            host("[1:2:3:4:5:6:7:8]", false),
            Ok("[1:2:3:4:5:6:7:8]".into())
        );
        for bad in [
            "[1:2:3:4:5:6:7:8:9]",
            "[1::2::3]",
            "[:1]",
            "[1:]",
            "[::1.2.3]",
            "[::01.2.3.4]",
            "[g::]",
        ] {
            assert_eq!(host(bad, false), Err(ParseError::InvalidIpv6), "{bad}");
        }
    }
}
