//! Subresource Integrity (W3C SRI, gap batch D, D62): an `integrity`
//! attribute's or `fetch()` option's metadata, and whether a response's
//! body matches it. The hashes are those `csp` hashes a source with.

use crate::csp::{Hash, base64, digest, normalized_base64};

/// The most entries a metadata list is read to.
pub const MAX_INTEGRITY_ENTRIES: usize = 64;

/// Parsed integrity metadata: the entries of the strongest algorithm
/// named, each its base64 value. Empty when nothing is to be checked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Integrity {
    strongest: Option<Hash>,
    values: Vec<String>,
}

impl Integrity {
    /// The metadata `text` gives (SRI section 3.3.3, "parse metadata"):
    /// tokens split at white space, each `algorithm-value` with options
    /// after a `?` ignored, those of no hash this checks passed over; the
    /// strongest algorithm's kept.
    pub fn parse(text: &str) -> Integrity {
        let mut entries: Vec<(Hash, String)> = Vec::new();
        for token in text.split_ascii_whitespace().take(MAX_INTEGRITY_ENTRIES) {
            let token = token.split_once('?').map_or(token, |(before, _)| before);
            let Some((algorithm, value)) = token.split_once('-') else {
                continue;
            };
            let hash = match algorithm.to_ascii_lowercase().as_str() {
                "sha256" => Hash::Sha256,
                "sha384" => Hash::Sha384,
                "sha512" => Hash::Sha512,
                _ => continue,
            };
            if !value.is_empty() {
                entries.push((hash, normalized_base64(value)));
            }
        }
        let strongest = entries.iter().map(|(hash, _)| *hash).max();
        Integrity {
            strongest,
            values: entries
                .into_iter()
                .filter(|(hash, _)| Some(*hash) == strongest)
                .map(|(_, value)| value)
                .collect(),
        }
    }

    /// Whether there is nothing to check: every body matches.
    pub fn is_empty(&self) -> bool {
        self.strongest.is_none()
    }

    /// Whether `body` matches (section 3.3.5): its digest by the
    /// strongest algorithm is one of the values.
    pub fn matches(&self, body: &[u8]) -> bool {
        let Some(hash) = self.strongest else {
            return true;
        };
        let actual = base64(&digest(hash, body));
        self.values.contains(&actual)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strongest_algorithm_decides() {
        // "alert('Hello, world.');", SRI's own example.
        let body = b"alert('Hello, world.');";
        let sha384 = "sha384-H8BRh8j48O9oYatfu5AZzq6A9RINhZO5H16dQZngK7T62em8MUt1FLm52t+eX6xO";
        assert!(Integrity::parse(sha384).matches(body));
        assert!(!Integrity::parse(sha384).matches(b"alert(1)"));
        // A wrong weaker hash beside a right stronger one: the stronger.
        let both = format!("sha256-AAAA {sha384}?opt");
        assert!(Integrity::parse(&both).matches(body));
        // A wrong stronger hash fails, a right weaker one aside.
        let wrong =
            "sha512-AAAA sha384-H8BRh8j48O9oYatfu5AZzq6A9RINhZO5H16dQZngK7T62em8MUt1FLm52t+eX6xO";
        assert!(!Integrity::parse(wrong).matches(body));
        // Nothing to check: unknown algorithms, empty values, nothing.
        for empty in ["", "md5-abc", "sha256-", "  "] {
            assert!(Integrity::parse(empty).is_empty(), "{empty:?}");
            assert!(Integrity::parse(empty).matches(b"anything"));
        }
        // base64url is taken.
        let url = "sha384-H8BRh8j48O9oYatfu5AZzq6A9RINhZO5H16dQZngK7T62em8MUt1FLm52t-eX6xO";
        assert!(Integrity::parse(url).matches(body));
    }
}
