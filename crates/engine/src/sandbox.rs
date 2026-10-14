//! Sandboxing and refusing to be framed (M33.2, `docs/design/hardening.md`):
//! HTML 7.1's sandboxing flag set, parsed from an `iframe`'s `sandbox`
//! attribute and a response's CSP `sandbox` directives, and the checks a
//! frame's response passes before anything of it is parsed: CSP's
//! `frame-ancestors`, else `X-Frame-Options` (HTML 7.4.2.3.7).

use net::csp::Csp;
use net::{HeaderList, Origin, Url};

/// A document's sandboxing flags (HTML 7.1.3): each one set takes away
/// what it names. None set for a document that is not sandboxed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Sandbox(u32);

impl Sandbox {
    /// No navigating other frames but itself, its descendants and the
    /// top (the top as the flags below allow).
    pub(crate) const NAVIGATION: Sandbox = Sandbox(1);
    /// No new tabs or windows: `_blank` and a name found nowhere.
    pub(crate) const AUXILIARY_NAVIGATION: Sandbox = Sandbox(1 << 1);
    /// No navigating the top without the user's activation.
    pub(crate) const TOP_NAVIGATION: Sandbox = Sandbox(1 << 2);
    /// No navigating the top with the user's activation either.
    pub(crate) const TOP_NAVIGATION_BY_USER: Sandbox = Sandbox(1 << 3);
    // 1 << 4: no plugins, which the engine has none of.
    /// The document's origin is a new opaque one.
    pub(crate) const ORIGIN: Sandbox = Sandbox(1 << 5);
    /// No form is submitted.
    pub(crate) const FORMS: Sandbox = Sandbox(1 << 6);
    pub(crate) const POINTER_LOCK: Sandbox = Sandbox(1 << 7);
    /// No script runs: none of its elements, event handler attributes or
    /// `javascript:` URLs.
    pub(crate) const SCRIPTS: Sandbox = Sandbox(1 << 8);
    pub(crate) const AUTOMATIC_FEATURES: Sandbox = Sandbox(1 << 9);
    /// `document.domain` may not be set.
    pub(crate) const DOCUMENT_DOMAIN: Sandbox = Sandbox(1 << 10);
    pub(crate) const PROPAGATES_TO_AUXILIARY: Sandbox = Sandbox(1 << 11);
    /// `alert`, `confirm` and `prompt` do nothing.
    pub(crate) const MODALS: Sandbox = Sandbox(1 << 12);
    pub(crate) const ORIENTATION_LOCK: Sandbox = Sandbox(1 << 13);
    pub(crate) const PRESENTATION: Sandbox = Sandbox(1 << 14);
    /// No download is started.
    pub(crate) const DOWNLOADS: Sandbox = Sandbox(1 << 15);
    // 1 << 16: no custom protocols' handlers, which it has none of.
    pub(crate) const STORAGE_ACCESS: Sandbox = Sandbox(1 << 17);
    /// Every flag: a `sandbox` attribute with no token.
    pub(crate) const ALL: Sandbox = Sandbox((1 << 18) - 1);
    pub(crate) const NONE: Sandbox = Sandbox(0);

    /// *Parse a sandboxing directive* (HTML 7.1.3): every flag but those
    /// the tokens, ASCII case-insensitively, take away; tokens it does
    /// not know are ignored.
    /// The flags as bits, for another process (M34).
    pub(crate) fn bits(self) -> u32 {
        self.0
    }

    /// The flags `bits` names; bits no flag has are dropped.
    pub(crate) fn from_bits(bits: u32) -> Sandbox {
        Sandbox(bits & Sandbox::ALL.0)
    }

    pub(crate) fn parse(value: &str) -> Sandbox {
        let mut flags = Sandbox::ALL;
        for token in value
            .split(['\t', '\n', '\u{c}', '\r', ' '])
            .filter(|token| !token.is_empty())
        {
            let lifted = match token.to_ascii_lowercase().as_str() {
                "allow-popups" => Sandbox::AUXILIARY_NAVIGATION,
                "allow-top-navigation" => {
                    Sandbox::TOP_NAVIGATION.with(Sandbox::TOP_NAVIGATION_BY_USER)
                }
                "allow-top-navigation-by-user-activation" => Sandbox::TOP_NAVIGATION_BY_USER,
                "allow-same-origin" => Sandbox::ORIGIN,
                "allow-forms" => Sandbox::FORMS,
                "allow-pointer-lock" => Sandbox::POINTER_LOCK,
                "allow-scripts" => Sandbox::SCRIPTS.with(Sandbox::AUTOMATIC_FEATURES),
                "allow-popups-to-escape-sandbox" => Sandbox::PROPAGATES_TO_AUXILIARY,
                "allow-modals" => Sandbox::MODALS,
                "allow-orientation-lock" => Sandbox::ORIENTATION_LOCK,
                "allow-presentation" => Sandbox::PRESENTATION,
                "allow-downloads" => Sandbox::DOWNLOADS,
                "allow-storage-access-by-user-activation" => Sandbox::STORAGE_ACCESS,
                _ => Sandbox::NONE,
            };
            flags = Sandbox(flags.0 & !lifted.0);
        }
        flags
    }

    /// The flags of the `sandbox` directives of `csp`'s policies (CSP
    /// 6.3.2), each parsed as the attribute is: a response's enforced
    /// ones, as no `<meta>` delivers one.
    pub(crate) fn from_csp(csp: &Csp) -> Sandbox {
        csp.sandboxes().iter().fold(Sandbox::NONE, |flags, value| {
            flags.with(Sandbox::parse(value))
        })
    }

    /// These flags and `other`'s.
    pub(crate) fn with(self, other: Sandbox) -> Sandbox {
        Sandbox(self.0 | other.0)
    }

    /// Whether every flag of `flag` is set.
    pub(crate) fn has(self, flag: Sandbox) -> bool {
        self.0 & flag.0 == flag.0
    }

    /// Whether any flag is set: the document is sandboxed.
    pub(crate) fn any(self) -> bool {
        self.0 != 0
    }
}

/// The `SecurityError` a sandboxed document's storage and cookies give
/// without `allow-same-origin`.
pub(crate) const NO_SAME_ORIGIN: &str =
    "The document is sandboxed and lacks the 'allow-same-origin' flag.";

/// Why a frame may not show the response from `url`, whose headers are
/// `headers`, under ancestors of `ancestors`' origins, the parent's
/// first: the console's line, or `None` when it may. A `frame-ancestors`
/// directive of an enforced policy decides when there is one (CSP
/// 4.2.5); else `X-Frame-Options` (HTML 7.4.2.3.7).
pub(crate) fn framing_refused(
    url: &Url,
    headers: &HeaderList,
    ancestors: &[Origin],
) -> Option<String> {
    let csp = Csp::from_headers(headers, url);
    match csp.check_frame_ancestors(ancestors) {
        Some(Ok(())) => return None,
        Some(Err(violation)) => {
            return Some(format!(
                "Refused to frame '{url}' because an ancestor violates the following Content \
                 Security Policy directive: \"{violation}\"."
            ));
        }
        None => {}
    }
    let why = x_frame_options(headers, &url.origin(), ancestors)?;
    Some(format!(
        "Refused to display '{url}' in a frame because {why}."
    ))
}

/// HTML 7.4.2.3.7, *check a navigation response's adherence to
/// `X-Frame-Options`*, for a frame's response of `origin`: why its values
/// refuse it, or `None`. Every field's values, split on commas, trimmed
/// and lowercased, are a set; several that include `deny`, `allowall` or
/// `sameorigin` refuse, several others allow; `deny` refuses;
/// `sameorigin` refuses unless every ancestor is of `origin`; anything
/// else, and nothing, allows.
pub(crate) fn x_frame_options(
    headers: &HeaderList,
    origin: &Origin,
    ancestors: &[Origin],
) -> Option<String> {
    let raw = headers
        .get_decode_split(b"X-Frame-Options")
        .unwrap_or_default();
    let mut values: Vec<String> = Vec::new();
    for value in &raw {
        let value = value.to_ascii_lowercase();
        if !values.contains(&value) {
            values.push(value);
        }
    }
    let known = |value: &String| matches!(value.as_str(), "deny" | "allowall" | "sameorigin");
    if values.len() > 1 {
        return values.iter().any(known).then(|| {
            format!(
                "it set multiple 'X-Frame-Options' headers with conflicting values ('{}'). \
                 Falling back to 'deny'",
                raw.join(", ")
            )
        });
    }
    match values.first().map(String::as_str) {
        Some("deny") => Some("it set 'X-Frame-Options' to 'deny'".to_owned()),
        Some("sameorigin") if ancestors.iter().any(|ancestor| ancestor != origin) => {
            Some("it set 'X-Frame-Options' to 'sameorigin'".to_owned())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tokens `sandbox.supports()` knows: the reference browser's
    /// thirteen (p2).
    const SUPPORTED: [&str; 13] = [
        "allow-downloads",
        "allow-forms",
        "allow-modals",
        "allow-orientation-lock",
        "allow-pointer-lock",
        "allow-popups",
        "allow-popups-to-escape-sandbox",
        "allow-presentation",
        "allow-same-origin",
        "allow-scripts",
        "allow-top-navigation",
        "allow-top-navigation-by-user-activation",
        "allow-storage-access-by-user-activation",
    ];

    fn url(s: &str) -> Url {
        Url::parse(s).expect("a URL")
    }

    fn headers(fields: &[(&str, &str)]) -> HeaderList {
        let mut list = HeaderList::new();
        for (name, value) in fields {
            list.append(name.as_bytes(), value.as_bytes());
        }
        list
    }

    #[test]
    fn the_attributes_tokens_lift_their_flags_in_any_case() {
        assert_eq!(Sandbox::parse(""), Sandbox::ALL);
        let flags = Sandbox::parse("  allow-scripts\tbogus ALLOW-SAME-ORIGIN ");
        assert!(!flags.has(Sandbox::SCRIPTS));
        assert!(!flags.has(Sandbox::AUTOMATIC_FEATURES));
        assert!(!flags.has(Sandbox::ORIGIN));
        assert!(flags.has(Sandbox::FORMS));
        assert!(flags.has(Sandbox::NAVIGATION));
        assert!(flags.has(Sandbox::DOCUMENT_DOMAIN));
        let top = Sandbox::parse("allow-top-navigation");
        assert!(!top.has(Sandbox::TOP_NAVIGATION));
        assert!(!top.has(Sandbox::TOP_NAVIGATION_BY_USER));
        let by_user = Sandbox::parse("allow-top-navigation-by-user-activation");
        assert!(by_user.has(Sandbox::TOP_NAVIGATION));
        assert!(!by_user.has(Sandbox::TOP_NAVIGATION_BY_USER));
        // Every supported token lifts something.
        for token in SUPPORTED {
            assert_ne!(Sandbox::parse(token), Sandbox::ALL, "{token}");
        }
        // The reference browser does not know this one.
        assert_eq!(
            Sandbox::parse("allow-top-navigation-to-custom-protocols"),
            Sandbox::ALL
        );
        assert!(!Sandbox::NONE.any());
        assert!(Sandbox::NONE.with(Sandbox::FORMS).has(Sandbox::FORMS));
    }

    #[test]
    fn a_policys_sandbox_directives_are_flags() {
        let page = url("http://a.test/");
        let csp = Csp::from_headers(
            &headers(&[
                ("Content-Security-Policy", "sandbox allow-scripts"),
                ("Content-Security-Policy", "script-src 'self'"),
            ]),
            &page,
        );
        let flags = Sandbox::from_csp(&csp);
        assert!(flags.has(Sandbox::ORIGIN));
        assert!(!flags.has(Sandbox::SCRIPTS));
        // Two policies' flags are both kept.
        let csp = Csp::from_headers(
            &headers(&[(
                "Content-Security-Policy",
                "sandbox allow-scripts, sandbox allow-same-origin",
            )]),
            &page,
        );
        let flags = Sandbox::from_csp(&csp);
        assert!(flags.has(Sandbox::ORIGIN) && flags.has(Sandbox::SCRIPTS));
        // A `<meta>` delivers none, and no directive is no sandbox.
        let mut csp = Csp::none(page.origin());
        csp.add_meta("sandbox");
        assert_eq!(Sandbox::from_csp(&csp), Sandbox::NONE);
    }

    #[test]
    fn x_frame_options_refuses_as_html_says() {
        let a = url("http://a.test/").origin();
        let b = url("http://b.test/").origin();
        let refused = |fields: &[(&str, &str)], ancestors: &[Origin]| {
            x_frame_options(&headers(fields), &a, ancestors).is_some()
        };
        let field = |value| [("X-Frame-Options", value)];
        assert!(!refused(&[], std::slice::from_ref(&b)));
        assert!(refused(&field("DENY"), std::slice::from_ref(&a)));
        assert!(!refused(&field("SAMEORIGIN"), &[a.clone(), a.clone()]));
        assert!(refused(&field("sameorigin"), &[a.clone(), b.clone()]));
        assert!(refused(&field("SAMEORIGIN"), std::slice::from_ref(&b)));
        assert!(!refused(&field("ALLOWALL"), std::slice::from_ref(&b)));
        assert!(!refused(
            &field("ALLOW-FROM http://b.test"),
            std::slice::from_ref(&b)
        ));
        assert!(!refused(&field("bogus"), std::slice::from_ref(&b)));
        assert!(refused(
            &field("DENY, SAMEORIGIN"),
            std::slice::from_ref(&a)
        ));
        assert!(refused(&field("DENY, bogus"), std::slice::from_ref(&a)));
        assert!(refused(
            &[
                ("X-Frame-Options", "deny"),
                ("X-Frame-Options", "SAMEORIGIN")
            ],
            std::slice::from_ref(&a)
        ));
        assert!(!refused(
            &field("SAMEORIGIN, sameorigin"),
            std::slice::from_ref(&a)
        ));
        assert!(!refused(&field("foo, bar"), std::slice::from_ref(&b)));
        let why = x_frame_options(
            &headers(&field("DENY, SAMEORIGIN")),
            &a,
            std::slice::from_ref(&a),
        );
        assert_eq!(
            why.as_deref(),
            Some(
                "it set multiple 'X-Frame-Options' headers with conflicting values \
                 ('DENY, SAMEORIGIN'). Falling back to 'deny'"
            )
        );
    }

    #[test]
    fn frame_ancestors_decides_over_x_frame_options() {
        let page = url("http://a.test/f.html");
        let a = page.origin();
        let b = url("http://b.test/").origin();
        let refused = |fields: &[(&str, &str)], ancestors: &[Origin]| {
            framing_refused(&page, &headers(fields), ancestors)
        };
        let csp = |value| ("Content-Security-Policy", value);
        assert_eq!(
            refused(&[csp("frame-ancestors 'none'")], std::slice::from_ref(&a)).as_deref(),
            Some(
                "Refused to frame 'http://a.test/f.html' because an ancestor violates the \
                 following Content Security Policy directive: \"frame-ancestors 'none'\"."
            )
        );
        assert!(refused(&[csp("frame-ancestors 'self'")], std::slice::from_ref(&a)).is_none());
        assert!(refused(&[csp("frame-ancestors 'self'")], &[a.clone(), b.clone()]).is_some());
        assert!(
            refused(
                &[csp("frame-ancestors http://b.test")],
                std::slice::from_ref(&b)
            )
            .is_none()
        );
        // An opaque ancestor matches nothing.
        assert!(refused(&[csp("frame-ancestors *")], &[Origin::new_opaque()]).is_some());
        // A directive that allows lets X-Frame-Options say nothing.
        assert!(
            refused(
                &[csp("frame-ancestors *"), ("X-Frame-Options", "DENY")],
                std::slice::from_ref(&b)
            )
            .is_none()
        );
        // A report-only policy refuses nothing.
        assert!(
            refused(
                &[(
                    "Content-Security-Policy-Report-Only",
                    "frame-ancestors 'none'"
                )],
                std::slice::from_ref(&b)
            )
            .is_none()
        );
        assert_eq!(
            refused(&[("X-Frame-Options", "deny")], std::slice::from_ref(&a)).as_deref(),
            Some(
                "Refused to display 'http://a.test/f.html' in a frame because it set \
                 'X-Frame-Options' to 'deny'."
            )
        );
    }
}
