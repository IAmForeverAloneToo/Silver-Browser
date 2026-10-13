//! Referrer Policy (M33.3, W3C Referrer Policy, Editor's Draft): the
//! policies, as a `Referrer-Policy` field, a `<meta name=referrer>` and
//! a `referrerpolicy` attribute give them, and "determine request's
//! referrer" (section 8.3), which the fetch runs at each hop to set the
//! `Referer` field.

use crate::headers::HeaderList;
use crate::url::{Origin, Url};

/// The longest referrer sent whole: past it, the origin alone (section
/// 8.3, step 6).
pub const MAX_REFERRER: usize = 4096;

/// A referrer policy (section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReferrerPolicy {
    NoReferrer,
    NoReferrerWhenDowngrade,
    SameOrigin,
    Origin,
    StrictOrigin,
    OriginWhenCrossOrigin,
    /// The default (section 3).
    #[default]
    StrictOriginWhenCrossOrigin,
    UnsafeUrl,
}

/// What a request's referrer is (Fetch's request referrer): none, its
/// client's (the document or worker that made it, its initiator's
/// URL), or a URL.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Referrer {
    None,
    #[default]
    Client,
    Url(Box<Url>),
}

impl ReferrerPolicy {
    /// The policy a token names, in any case; `None` for one that names
    /// none, the empty string included.
    pub fn parse(token: &str) -> Option<ReferrerPolicy> {
        Some(match token.to_ascii_lowercase().as_str() {
            "no-referrer" => ReferrerPolicy::NoReferrer,
            "no-referrer-when-downgrade" => ReferrerPolicy::NoReferrerWhenDowngrade,
            "same-origin" => ReferrerPolicy::SameOrigin,
            "origin" => ReferrerPolicy::Origin,
            "strict-origin" => ReferrerPolicy::StrictOrigin,
            "origin-when-cross-origin" => ReferrerPolicy::OriginWhenCrossOrigin,
            "strict-origin-when-cross-origin" => ReferrerPolicy::StrictOriginWhenCrossOrigin,
            "unsafe-url" => ReferrerPolicy::UnsafeUrl,
            _ => return None,
        })
    }

    /// *Parse a referrer policy from a Referrer-Policy header* (section
    /// 8.1): the last of its comma-split tokens that names a policy.
    pub fn from_headers(headers: &HeaderList) -> Option<ReferrerPolicy> {
        headers
            .get_decode_split(b"Referrer-Policy")
            .unwrap_or_default()
            .iter()
            .rev()
            .find_map(|token| ReferrerPolicy::parse(token.trim()))
    }

    /// A `<meta name=referrer>`'s policy (HTML section 4.2.5.1): its
    /// value, ASCII-lowercased, with the legacy names mapped; `None` for
    /// one that names none.
    pub fn from_meta(content: &str) -> Option<ReferrerPolicy> {
        match content.trim().to_ascii_lowercase().as_str() {
            "never" => Some(ReferrerPolicy::NoReferrer),
            "default" => Some(ReferrerPolicy::StrictOriginWhenCrossOrigin),
            "always" => Some(ReferrerPolicy::UnsafeUrl),
            "origin-when-crossorigin" => Some(ReferrerPolicy::OriginWhenCrossOrigin),
            other => ReferrerPolicy::parse(other),
        }
    }

    /// Its name, as `Request.referrerPolicy` and the field say it.
    pub fn name(self) -> &'static str {
        match self {
            ReferrerPolicy::NoReferrer => "no-referrer",
            ReferrerPolicy::NoReferrerWhenDowngrade => "no-referrer-when-downgrade",
            ReferrerPolicy::SameOrigin => "same-origin",
            ReferrerPolicy::Origin => "origin",
            ReferrerPolicy::StrictOrigin => "strict-origin",
            ReferrerPolicy::OriginWhenCrossOrigin => "origin-when-cross-origin",
            ReferrerPolicy::StrictOriginWhenCrossOrigin => "strict-origin-when-cross-origin",
            ReferrerPolicy::UnsafeUrl => "unsafe-url",
        }
    }
}

/// *Determine request's referrer* (section 8.3) for a request from
/// `source` (the document's URL, or the URL a request names) to
/// `target`, by `policy`: the `Referer` value, or none. A local URL's
/// (`about:`, `blob:`, `data:`), and an opaque origin's, is none.
pub fn determine(
    source: &Url,
    source_origin: &Origin,
    policy: ReferrerPolicy,
    target: &Url,
) -> Option<String> {
    if !matches!(source.scheme(), "http" | "https") || source_origin.is_opaque() {
        return None;
    }
    let url = strip(source, false)?;
    let origin = strip(source, true)?;
    let url = match url.len() > MAX_REFERRER {
        true => origin.clone(),
        false => url,
    };
    let same_origin = source.origin() == target.origin();
    let downgrade = source.is_potentially_trustworthy() && !target.is_potentially_trustworthy();
    match policy {
        ReferrerPolicy::NoReferrer => None,
        ReferrerPolicy::Origin => Some(origin),
        ReferrerPolicy::UnsafeUrl => Some(url),
        ReferrerPolicy::StrictOrigin => (!downgrade).then_some(origin),
        ReferrerPolicy::StrictOriginWhenCrossOrigin => match (same_origin, downgrade) {
            (true, _) => Some(url),
            (false, true) => None,
            (false, false) => Some(origin),
        },
        ReferrerPolicy::SameOrigin => same_origin.then_some(url),
        ReferrerPolicy::OriginWhenCrossOrigin => match same_origin {
            true => Some(url),
            false => Some(origin),
        },
        ReferrerPolicy::NoReferrerWhenDowngrade => (!downgrade).then_some(url),
    }
}

/// *Strip url for use as a referrer* (section 8.4): without its
/// credentials and fragment, and, `origin_only`, its path and query.
fn strip(url: &Url, origin_only: bool) -> Option<String> {
    let origin = url.origin();
    let Origin::Tuple { .. } = origin else {
        return None;
    };
    if origin_only {
        return Some(format!("{}/", origin.serialization()));
    }
    let mut text = url.without_fragment().to_owned();
    if !url.username().is_empty() || !url.password().is_empty() {
        let at = text.find('@')?;
        let scheme_end = text.find("://")? + 3;
        text.replace_range(scheme_end..=at, "");
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).expect("a URL")
    }

    fn sent(source: &str, policy: ReferrerPolicy, target: &str) -> Option<String> {
        let source = url(source);
        determine(&source, &source.origin(), policy, &url(target))
    }

    #[test]
    fn each_policy_sends_what_section_8_3_says() {
        use ReferrerPolicy::*;
        let page = "https://a.test/dir/page.html?q=1#frag";
        let same = "https://a.test/other";
        let cross = "https://b.test/x";
        let down = "http://b.test/x";
        let whole = "https://a.test/dir/page.html?q=1";
        let origin = "https://a.test/";
        let table: [(ReferrerPolicy, [Option<&str>; 3]); 8] = [
            (NoReferrer, [None, None, None]),
            (NoReferrerWhenDowngrade, [Some(whole), Some(whole), None]),
            (SameOrigin, [Some(whole), None, None]),
            (Origin, [Some(origin), Some(origin), Some(origin)]),
            (StrictOrigin, [Some(origin), Some(origin), None]),
            (
                OriginWhenCrossOrigin,
                [Some(whole), Some(origin), Some(origin)],
            ),
            (
                StrictOriginWhenCrossOrigin,
                [Some(whole), Some(origin), None],
            ),
            (UnsafeUrl, [Some(whole), Some(whole), Some(whole)]),
        ];
        for (policy, wanted) in table {
            for (target, wanted) in [same, cross, down].iter().zip(wanted) {
                assert_eq!(
                    sent(page, policy, target).as_deref(),
                    wanted,
                    "{policy:?} to {target}"
                );
            }
        }
        // Credentials go; a local URL and an opaque origin send nothing.
        assert_eq!(
            sent("https://u:p@a.test/x#f", UnsafeUrl, cross).as_deref(),
            Some("https://a.test/x")
        );
        assert_eq!(sent("data:text/html,x", UnsafeUrl, cross), None);
        assert_eq!(sent("blob:https://a.test/0b1d", UnsafeUrl, cross), None);
        assert_eq!(sent("about:blank", UnsafeUrl, cross), None);
        let page = url(page);
        assert_eq!(
            determine(
                &page,
                &crate::url::Origin::new_opaque(),
                UnsafeUrl,
                &url(cross)
            ),
            None
        );
        // Past 4,096 bytes, the origin alone.
        let long = format!("https://a.test/{}", "x".repeat(MAX_REFERRER));
        assert_eq!(
            sent(&long, UnsafeUrl, cross).as_deref(),
            Some("https://a.test/")
        );
    }

    #[test]
    fn a_field_and_a_meta_give_their_policies() {
        let field = |value: &str| {
            let mut headers = HeaderList::new();
            headers.append(b"Referrer-Policy", value.as_bytes());
            ReferrerPolicy::from_headers(&headers)
        };
        assert_eq!(field("no-referrer, origin"), Some(ReferrerPolicy::Origin));
        assert_eq!(field("origin, bogus"), Some(ReferrerPolicy::Origin));
        assert_eq!(field("bogus"), None);
        assert_eq!(field("UNSAFE-URL"), Some(ReferrerPolicy::UnsafeUrl));
        assert_eq!(
            ReferrerPolicy::from_meta("never"),
            Some(ReferrerPolicy::NoReferrer)
        );
        assert_eq!(
            ReferrerPolicy::from_meta("origin-when-crossorigin"),
            Some(ReferrerPolicy::OriginWhenCrossOrigin)
        );
        assert_eq!(ReferrerPolicy::from_meta("bogus"), None);
        assert_eq!(ReferrerPolicy::from_meta(""), None);
        for policy in [
            ReferrerPolicy::NoReferrer,
            ReferrerPolicy::NoReferrerWhenDowngrade,
            ReferrerPolicy::SameOrigin,
            ReferrerPolicy::Origin,
            ReferrerPolicy::StrictOrigin,
            ReferrerPolicy::OriginWhenCrossOrigin,
            ReferrerPolicy::StrictOriginWhenCrossOrigin,
            ReferrerPolicy::UnsafeUrl,
        ] {
            assert_eq!(ReferrerPolicy::parse(policy.name()), Some(policy));
        }
        assert_eq!(
            ReferrerPolicy::default(),
            ReferrerPolicy::StrictOriginWhenCrossOrigin
        );
    }
}
