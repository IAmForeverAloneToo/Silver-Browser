//! Content Security Policy (M24.6, M33.1; CSP Level 3, Editor's Draft of
//! 16 September 2026): a document's policies, enforced and report-only,
//! parsed from its response's `Content-Security-Policy` and
//! `Content-Security-Policy-Report-Only` fields and its `<meta>`
//! elements; the checks a request passes before each hop, by its
//! destination's fetch directive; and those an inline script and a
//! `javascript:` URL pass before they run.
//!
//! Every policy is checked, not the first that refuses alone: each that
//! does not allow gives a violation, an enforced one's refusing and a
//! report-only one's reporting. A request's check records its violations
//! where the document that made it takes them (its sink). An expression
//! not understood matches nothing; a directive not understood is kept and
//! consulted by nothing. A policy is never looser than the page asked
//! for. A response's `sandbox` and `frame-ancestors` directives are read
//! for its frame (M33.2).

use std::sync::{Arc, Mutex, PoisonError};

use crate::fetch::Destination;
use crate::headers::HeaderList;
use crate::url::{Host, Origin, Url, default_port, percent_decode};

/// The most policies a document keeps; one past them refuses everything a
/// check looks at, so that no page escapes its policy by adding more.
pub const MAX_POLICIES: usize = 64;

/// The most violations a sink keeps until its document takes them; past
/// them the newest are dropped.
pub const MAX_RECORDED: usize = 256;

/// A document's policies and the origin `'self'` means: HTML's policy
/// container's CSP list, with where the violations of its requests'
/// checks wait for it.
#[derive(Debug, Clone)]
pub struct Csp {
    policies: Vec<Policy>,
    self_origin: Origin,
    /// More policies came than are kept: nothing is allowed.
    overflowed: bool,
    sink: Option<Sink>,
}

/// A policy list as plain values (M34.2): [`Csp::parts`] and
/// [`Csp::from_parts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CspParts {
    /// Each policy's serialization, disposition, and whether it came
    /// from a `<meta>`.
    pub policies: Vec<(String, Disposition, bool)>,
    pub self_origin: Origin,
    pub overflowed: bool,
}

/// Where a document's violations wait for it, shared by the copies of its
/// policies its requests carry.
type Sink = Arc<Mutex<Vec<Violation>>>;

/// Whether a policy refuses what it does not allow, or reports it alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Enforce,
    Report,
}

impl Disposition {
    /// Its name, as a violation's report says it.
    pub fn name(self) -> &'static str {
        match self {
            Disposition::Enforce => "enforce",
            Disposition::Report => "report",
        }
    }
}

/// One policy: its directives, each name once, the first kept; its
/// disposition; and its serialization, as a report quotes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    directives: Vec<Directive>,
    disposition: Disposition,
    serialization: String,
    /// Whether a `<meta>` delivered it.
    from_meta: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Directive {
    /// Lowercase.
    name: String,
    value: Vec<String>,
}

/// What a policy did not allow: the directive that decided, as the policy
/// wrote it, and its value; the check's effective directive (section
/// 6.8.4), which the reference browser reports as the violated one too;
/// what was blocked (a URL, or `inline`); what the check was for, as a
/// console line words it; and the policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub directive: String,
    pub value: String,
    pub effective: String,
    pub blocked: String,
    pub what: String,
    pub disposition: Disposition,
    pub policy: String,
    /// The first [`MAX_SAMPLE`] characters of an inline script, style,
    /// attribute, `javascript:` URL or code string, when its directive
    /// has `'report-sample'`; empty otherwise.
    pub sample: String,
    /// Where its policy's `report-uri` sends reports, as it wrote them.
    pub report_uri: Vec<String>,
}

/// How many characters of what was blocked a violation's sample keeps.
pub const MAX_SAMPLE: usize = 40;

/// What an inline check is for (section 6.7.3.3's types): an inline
/// script or style element's text, an event handler attribute's or a
/// `style` attribute's value, or a `javascript:` URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inline {
    Script,
    ScriptAttribute,
    Navigation,
    Style,
    StyleAttribute,
}

impl Inline {
    /// The directives its check consults, the most relevant first.
    fn directives(self) -> &'static [&'static str] {
        match self {
            Inline::Script | Inline::Navigation => SCRIPT_ELEM,
            Inline::ScriptAttribute => &["script-src-attr", "script-src", "default-src"],
            Inline::Style => &["style-src-elem", "style-src", "default-src"],
            Inline::StyleAttribute => &["style-src-attr", "style-src", "default-src"],
        }
    }

    /// What a console line calls it.
    fn what(self) -> &'static str {
        match self {
            Inline::Script => "an inline script",
            Inline::ScriptAttribute => "an event handler",
            Inline::Navigation => "a javascript: URL",
            Inline::Style => "an inline style",
            Inline::StyleAttribute => "a style attribute",
        }
    }

    /// Whether it is an element's, which a nonce, and a hash without
    /// `'unsafe-hashes'`, may allow.
    fn element(self) -> bool {
        matches!(self, Inline::Script | Inline::Style)
    }

    /// Whether it is a script's, which `'strict-dynamic'` concerns.
    fn script(self) -> bool {
        matches!(
            self,
            Inline::Script | Inline::ScriptAttribute | Inline::Navigation
        )
    }
}

/// A hash source's algorithm (section 2.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hash {
    Sha256,
    Sha384,
    Sha512,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.value.is_empty() {
            write!(f, "{}", self.directive)
        } else {
            write!(f, "{} {}", self.directive, self.value)
        }
    }
}

/// The directives an inline script's and a script element's checks
/// consult, the most relevant first (section 6.8.3).
const SCRIPT_ELEM: &[&str] = &["script-src-elem", "script-src", "default-src"];

/// The fetch directive a request of `destination` is checked against, and
/// those it falls back to (sections 6.1 and 6.8.4), and what a console
/// line calls what it fetches; none for a document's own.
fn fetch_directives(destination: Destination) -> (&'static [&'static str], &'static str) {
    match destination {
        Destination::Script => (SCRIPT_ELEM, "the script"),
        Destination::Style => (
            &["style-src-elem", "style-src", "default-src"],
            "the stylesheet",
        ),
        Destination::Image => (&["img-src", "default-src"], "the image"),
        Destination::Audio | Destination::Video => (&["media-src", "default-src"], "media from"),
        Destination::Iframe => (&["frame-src", "child-src", "default-src"], "the frame"),
        Destination::Worker | Destination::ServiceWorker => (
            &["worker-src", "child-src", "script-src", "default-src"],
            "the worker",
        ),
        Destination::Empty => (&["connect-src", "default-src"], "the connection to"),
        Destination::Document | Destination::Report => (&[], ""),
    }
}

impl Policy {
    /// *Parse a serialized CSP* (section 2.2.1) as an enforced policy.
    pub fn parse(serialized: &str) -> Policy {
        Policy::parse_as(serialized, Disposition::Enforce)
    }

    /// *Parse a serialized CSP* (section 2.2.1): directives split on
    /// `;`, each a lowercased name and its whitespace-separated value; a
    /// token not ASCII, empty, or naming a directive already seen is
    /// skipped.
    pub fn parse_as(serialized: &str, disposition: Disposition) -> Policy {
        let mut directives: Vec<Directive> = Vec::new();
        for token in serialized.split(';') {
            let token = token.trim_matches(is_ascii_whitespace);
            if token.is_empty() || !token.is_ascii() {
                continue;
            }
            let mut parts = token.split(is_ascii_whitespace).filter(|p| !p.is_empty());
            let name = parts.next().unwrap_or_default().to_ascii_lowercase();
            if directives.iter().any(|d| d.name == name) {
                continue;
            }
            directives.push(Directive {
                name,
                value: parts.map(str::to_owned).collect(),
            });
        }
        Policy {
            directives,
            disposition,
            serialization: serialized.trim_matches(is_ascii_whitespace).to_owned(),
            from_meta: false,
        }
    }

    /// Whether the policy has no directive, as one that failed to parse.
    pub fn is_empty(&self) -> bool {
        self.directives.is_empty()
    }

    /// The directive a check of `names` applies (section 6.8.4): the
    /// first of them the policy has.
    fn directive(&self, names: &[&str]) -> Option<&Directive> {
        names
            .iter()
            .find_map(|name| self.directives.iter().find(|d| d.name == *name))
    }

    /// Removes the directives a `<meta>` may not deliver (HTML section
    /// 4.2.5.3): `report-uri`, `frame-ancestors` and `sandbox`.
    fn for_meta(mut self) -> Policy {
        self.from_meta = true;
        self.directives.retain(|d| {
            !matches!(
                d.name.as_str(),
                "report-uri" | "frame-ancestors" | "sandbox"
            )
        });
        self
    }
}

impl Csp {
    /// No policy: everything allowed, `'self'` meaning `self_origin`.
    pub fn none(self_origin: Origin) -> Csp {
        Csp {
            policies: Vec::new(),
            self_origin,
            overflowed: false,
            sink: None,
        }
    }

    /// The same policies, recording the violations its requests' checks
    /// find where [`Csp::take_violations`] gives them: a document's.
    pub fn with_sink(mut self) -> Csp {
        self.sink = Some(Arc::default());
        self
    }

    /// A copy for a document that inherits these policies (HTML's policy
    /// container cloned, M33.1), recording its own violations.
    pub fn inherited(&self) -> Csp {
        Csp {
            sink: None,
            ..self.clone()
        }
        .with_sink()
    }

    /// *Parse a response's Content Security Policies* (section 2.2.2):
    /// each `Content-Security-Policy` value, split on commas, an enforced
    /// policy, and each `Content-Security-Policy-Report-Only` one a
    /// report-only policy, those with no directive left out; `'self'` is
    /// `url`'s origin.
    pub fn from_headers(headers: &HeaderList, url: &Url) -> Csp {
        let mut csp = Csp::none(url.origin());
        let fields: [(&[u8], Disposition); 2] = [
            (b"Content-Security-Policy", Disposition::Enforce),
            (b"Content-Security-Policy-Report-Only", Disposition::Report),
        ];
        for (name, disposition) in fields {
            for serialized in headers.get_decode_split(name).unwrap_or_default() {
                csp.add(Policy::parse_as(&serialized, disposition));
            }
        }
        csp
    }

    /// Adds the policy of a `<meta http-equiv="Content-Security-Policy">`
    /// the parser inserted into the head (HTML section 4.2.5.3): it
    /// applies to what comes after it.
    pub fn add_meta(&mut self, content: &str) {
        self.add(Policy::parse(content).for_meta());
    }

    fn add(&mut self, policy: Policy) {
        if policy.is_empty() {
            return;
        }
        if self.policies.len() == MAX_POLICIES {
            self.overflowed = true;
            return;
        }
        self.policies.push(policy);
    }

    /// The violations of a request of `destination` for `url`, after
    /// `redirects` redirects, carrying `nonce` and inserted by the parser
    /// or not (section 6.7.1, its fetch directive's pre-request check):
    /// one for each policy whose directive does not allow it, naming
    /// `reported` (the request's first URL after a redirect to another
    /// origin, which a report may not reveal). A nonce matches for a
    /// script's or a stylesheet's element; `'strict-dynamic'` in a
    /// script's or a worker's list leaves its URLs aside and allows what
    /// the parser did not insert.
    pub fn request_violations(
        &self,
        destination: Destination,
        (url, reported): (&Url, &Url),
        redirects: usize,
        nonce: &str,
        parser_inserted: bool,
    ) -> Vec<Violation> {
        let (names, noun) = fetch_directives(destination);
        let Some(&effective) = names.first() else {
            return Vec::new();
        };
        let nonceable = matches!(effective, "script-src-elem" | "style-src-elem");
        let dynamic = matches!(effective, "script-src-elem" | "worker-src");
        let what = format!("{noun} {reported}");
        self.violations(names, (reported.as_str(), ""), &what, |list| {
            if nonceable
                && !nonce.is_empty()
                && list.iter().any(|source| nonce_of(source) == Some(nonce))
            {
                return true;
            }
            if dynamic
                && list
                    .iter()
                    .any(|source| source.eq_ignore_ascii_case("'strict-dynamic'"))
            {
                return !parser_inserted;
            }
            url_matches_list(url, list, &self.self_origin, redirects)
        })
    }

    /// Whether a parser-inserted script with no nonce may be fetched from
    /// `url` after `redirects` redirects: the first enforced violation,
    /// if one.
    pub fn check_script_url(&self, url: &Url, redirects: usize) -> Result<(), Box<Violation>> {
        enforced(self.request_violations(Destination::Script, (url, url), redirects, "", true))
    }

    /// The violations of inline content of `kind` (section 6.7.3.3, "does
    /// element match source list for type and source"), its element's
    /// `nonce` (empty for none, or for an element that is not nonceable)
    /// and its `source`: each policy whose directive neither allows all
    /// inline behavior, nor a nonce of the element's (an element's), nor
    /// the hash of its source (an element's, or with `'unsafe-hashes'`).
    pub fn inline_violations(&self, kind: Inline, nonce: &str, source: &str) -> Vec<Violation> {
        self.violations(kind.directives(), ("inline", source), kind.what(), |list| {
            inline_allowed(list, kind, nonce, source)
        })
    }

    /// Whether an inline script with no nonce and no text may run: the
    /// first enforced violation, if one.
    pub fn check_inline_script(&self) -> Result<(), Box<Violation>> {
        enforced(self.inline_violations(Inline::Script, "", ""))
    }

    /// Whether a `javascript:` URL of no text may run: the first enforced
    /// violation, if one.
    pub fn check_javascript_url(&self) -> Result<(), Box<Violation>> {
        enforced(self.inline_violations(Inline::Navigation, "", ""))
    }

    /// The violations of a string compiled as code, `source` (section
    /// 4.4, "EnsureCSPDoesNotBlockStringCompilation"): each policy whose
    /// `script-src` (else `default-src`) has no `'unsafe-eval'`.
    pub fn eval_violations(&self, source: &str) -> Vec<Violation> {
        self.violations(
            &["script-src", "default-src"],
            ("eval", source),
            "a string as code",
            |list| has_keyword(list, "'unsafe-eval'"),
        )
    }

    /// The violations of a WebAssembly module compiled (section 4.5):
    /// each policy whose `script-src` (else `default-src`) has neither
    /// `'wasm-unsafe-eval'` nor `'unsafe-eval'`.
    pub fn wasm_violations(&self) -> Vec<Violation> {
        self.violations(
            &["script-src", "default-src"],
            ("wasm-eval", ""),
            "a WebAssembly module",
            |list| has_keyword(list, "'wasm-unsafe-eval'") || has_keyword(list, "'unsafe-eval'"),
        )
    }

    /// The violations of a form's submission to `url`, after `redirects`
    /// redirects (section 6.4.1): each policy whose `form-action`, which
    /// falls back to nothing, does not match it.
    pub fn form_action_violations(
        &self,
        (url, reported): (&Url, &Url),
        redirects: usize,
    ) -> Vec<Violation> {
        let what = format!("the form submission to {reported}");
        self.violations(&["form-action"], (reported.as_str(), ""), &what, |list| {
            url_matches_list(url, list, &self.self_origin, redirects)
        })
    }

    /// The violations of a `<base>` whose URL is `url` (section 6.3.1):
    /// each policy whose `base-uri`, which falls back to nothing, does not
    /// match it.
    pub fn base_uri_violations(&self, url: &Url) -> Vec<Violation> {
        let what = format!("the base URL {url}");
        self.violations(&["base-uri"], (url.as_str(), ""), &what, |list| {
            url_matches_list(url, list, &self.self_origin, 0)
        })
    }

    /// Whether an enforced policy has `upgrade-insecure-requests`
    /// (Upgrade Insecure Requests section 3).
    pub fn upgrades_insecure_requests(&self) -> bool {
        self.enforcing()
            .any(|policy| policy.directive(&["upgrade-insecure-requests"]).is_some())
    }

    /// Whether the document has no policy: no check refuses or reports.
    pub fn is_empty(&self) -> bool {
        self.policies.is_empty() && !self.overflowed
    }

    /// Whether a policy came from a response's header rather than a
    /// `<meta>` (HTML's nonce attribute steps hide nonces only then).
    pub fn has_header_policy(&self) -> bool {
        self.policies.iter().any(|policy| !policy.from_meta)
    }

    /// The policies as plain values, for another process (M34.2): each
    /// one's serialization, disposition and whether it came from a
    /// `<meta>`, the origin they protect, whether more came than are kept.
    pub fn parts(&self) -> CspParts {
        CspParts {
            policies: self
                .policies
                .iter()
                .map(|policy| {
                    (
                        policy.serialization.clone(),
                        policy.disposition,
                        policy.from_meta,
                    )
                })
                .collect(),
            self_origin: self.self_origin.clone(),
            overflowed: self.overflowed,
        }
    }

    /// The policies `parts` describes, parsed again, with no sink; at most
    /// as many as one list keeps.
    pub fn from_parts(parts: CspParts) -> Csp {
        let overflowed = parts.overflowed || parts.policies.len() > MAX_POLICIES;
        let policies = parts
            .policies
            .into_iter()
            .take(MAX_POLICIES)
            .map(|(serialized, disposition, from_meta)| {
                let policy = Policy::parse_as(&serialized, disposition);
                if from_meta { policy.for_meta() } else { policy }
            })
            .collect();
        Csp {
            policies,
            self_origin: parts.self_origin,
            overflowed,
            sink: None,
        }
    }

    /// Records `violations` where the document of these policies takes
    /// them, when it has a sink.
    pub fn record(&self, violations: &[Violation]) {
        if let Some(sink) = &self.sink {
            let mut sink = sink.lock().unwrap_or_else(PoisonError::into_inner);
            let room = MAX_RECORDED.saturating_sub(sink.len());
            sink.extend(violations.iter().take(room).cloned());
        }
    }

    /// The violations recorded since the last time, oldest first.
    pub fn take_violations(&self) -> Vec<Violation> {
        match &self.sink {
            Some(sink) => std::mem::take(&mut *sink.lock().unwrap_or_else(PoisonError::into_inner)),
            None => Vec::new(),
        }
    }

    /// The enforced policies, which alone deliver `sandbox` and
    /// `frame-ancestors` (sections 6.3.2 and 6.4.2).
    fn enforcing(&self) -> impl Iterator<Item = &Policy> {
        self.policies
            .iter()
            .filter(|policy| policy.disposition == Disposition::Enforce)
    }

    /// The values of the enforced policies' `sandbox` directives (section
    /// 6.3.2, M33.2): a response's, as no `<meta>` delivers one.
    pub fn sandboxes(&self) -> Vec<String> {
        self.enforcing()
            .filter_map(|policy| {
                let directive = policy.directive(&["sandbox"])?;
                Some(directive.value.join(" "))
            })
            .collect()
    }

    /// Whether a frame may show the response these policies came with,
    /// under ancestors of `ancestors`' origins, the parent's first
    /// (section 4.2.5, "does a navigation response violate
    /// frame-ancestors", M33.2): each enforced policy's `frame-ancestors`
    /// must match every ancestor's origin as a URL, which an opaque one is
    /// not. `None` when no policy has the directive; the first that
    /// refuses, if one.
    pub fn check_frame_ancestors(&self, ancestors: &[Origin]) -> Option<Result<(), Violation>> {
        let mut found = None;
        for policy in self.enforcing() {
            let Some(directive) = policy.directive(&["frame-ancestors"]) else {
                continue;
            };
            found = Some(Ok(()));
            let refused = ancestors.iter().any(|ancestor| {
                Url::parse(&ancestor.serialization())
                    .ok()
                    .is_none_or(|url| {
                        !url_matches_list(&url, &directive.value, &self.self_origin, 0)
                    })
            });
            if refused {
                return Some(Err(violation(policy, directive, "frame-ancestors", "", "")));
            }
        }
        if found.is_some() && self.overflowed {
            return Some(Err(too_many("frame-ancestors", "", "")));
        }
        found
    }

    /// One violation for each policy whose directive among `names` does
    /// not allow what `allows` looks at, `blocked` and `what` saying what
    /// it is; and one, enforced, when the document has too many policies.
    fn violations(
        &self,
        names: &[&str],
        (blocked, source): (&str, &str),
        what: &str,
        allows: impl Fn(&[String]) -> bool,
    ) -> Vec<Violation> {
        let effective = names.first().copied().unwrap_or_default();
        let mut found = Vec::new();
        if self.overflowed {
            found.push(too_many(effective, blocked, what));
        }
        for policy in &self.policies {
            if let Some(directive) = policy.directive(names)
                && !allows(&directive.value)
            {
                let mut violation = violation(policy, directive, effective, blocked, what);
                if has_keyword(&directive.value, "'report-sample'") {
                    violation.sample = source.chars().take(MAX_SAMPLE).collect();
                }
                found.push(violation);
            }
        }
        found
    }
}

/// `policy`'s violation by `directive`, for a check of `effective`.
fn violation(
    policy: &Policy,
    directive: &Directive,
    effective: &str,
    blocked: &str,
    what: &str,
) -> Violation {
    Violation {
        directive: directive.name.clone(),
        value: directive.value.join(" "),
        effective: effective.to_owned(),
        blocked: blocked.to_owned(),
        what: what.to_owned(),
        disposition: policy.disposition,
        policy: policy.serialization.clone(),
        sample: String::new(),
        report_uri: policy
            .directive(&["report-uri"])
            .map(|directive| directive.value.clone())
            .unwrap_or_default(),
    }
}

/// The violation of a document with more policies than are kept.
fn too_many(effective: &str, blocked: &str, what: &str) -> Violation {
    Violation {
        directive: "too many policies".to_owned(),
        value: String::new(),
        effective: effective.to_owned(),
        blocked: blocked.to_owned(),
        what: what.to_owned(),
        disposition: Disposition::Enforce,
        policy: String::new(),
        sample: String::new(),
        report_uri: Vec::new(),
    }
}

/// Whether `list` has the keyword source `keyword`, in any case.
fn has_keyword(list: &[String], keyword: &str) -> bool {
    list.iter()
        .any(|source| source.eq_ignore_ascii_case(keyword))
}

/// Whether `list` allows inline content of `kind` with `nonce` and
/// `source` (section 6.7.3.3).
fn inline_allowed(list: &[String], kind: Inline, nonce: &str, source: &str) -> bool {
    if allows_all_inline(list, kind.script()) {
        return true;
    }
    if kind.element()
        && !nonce.is_empty()
        && list
            .iter()
            .any(|expression| nonce_of(expression) == Some(nonce))
    {
        return true;
    }
    if !kind.element() && !has_keyword(list, "'unsafe-hashes'") {
        return false;
    }
    list.iter().any(|expression| {
        hash_of(expression).is_some_and(|(hash, value)| {
            normalized_base64(value) == base64(&digest(hash, source.as_bytes()))
        })
    })
}

/// A hash source's algorithm and base64 value, `'sha256-...'` and the
/// others (section 2.3.1), its prefix in any case.
fn hash_of(expression: &str) -> Option<(Hash, &str)> {
    let inner = expression.strip_prefix('\'')?.strip_suffix('\'')?;
    let (algorithm, value) = inner.split_once('-')?;
    let hash = match algorithm.to_ascii_lowercase().as_str() {
        "sha256" => Hash::Sha256,
        "sha384" => Hash::Sha384,
        "sha512" => Hash::Sha512,
        _ => return None,
    };
    (!value.is_empty()).then_some((hash, value))
}

/// `bytes`' digest by `hash`.
fn digest(hash: Hash, bytes: &[u8]) -> Vec<u8> {
    let algorithm = match hash {
        Hash::Sha256 => &aws_lc_rs::digest::SHA256,
        Hash::Sha384 => &aws_lc_rs::digest::SHA384,
        Hash::Sha512 => &aws_lc_rs::digest::SHA512,
    };
    aws_lc_rs::digest::digest(algorithm, bytes)
        .as_ref()
        .to_vec()
}

/// `bytes` in base64 with padding (RFC 4648 section 4).
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (u32::from(b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A hash source's value with base64url's two characters as base64's,
/// as the reference browser takes it.
fn normalized_base64(value: &str) -> String {
    value.replace('-', "+").replace('_', "/")
}

/// The first violation of `violations` an enforced policy gave, which
/// refuses: `Ok` when there is none.
pub fn enforced(violations: Vec<Violation>) -> Result<(), Box<Violation>> {
    match violations
        .into_iter()
        .find(|violation| violation.disposition == Disposition::Enforce)
    {
        Some(violation) => Err(Box::new(violation)),
        None => Ok(()),
    }
}

/// The value of a nonce source, `'nonce-...'` (section 2.3.1), its
/// prefix in any case.
fn nonce_of(expression: &str) -> Option<&str> {
    let inner = expression.strip_suffix('\'')?;
    let prefix = inner.get(..7)?;
    let value = &inner[7..];
    (prefix.eq_ignore_ascii_case("'nonce-") && !value.is_empty()).then_some(value)
}

/// *Does a source list allow all inline behavior* (section 6.7.3.2):
/// `'unsafe-inline'`, with no nonce or hash, nor for a script
/// `'strict-dynamic'`.
fn allows_all_inline(list: &[String], script: bool) -> bool {
    let mut unsafe_inline = false;
    for expression in list {
        let lower = expression.to_ascii_lowercase();
        if lower.starts_with("'nonce-")
            || ["'sha256-", "'sha384-", "'sha512-"]
                .iter()
                .any(|hash| lower.starts_with(hash))
            || (script && lower == "'strict-dynamic'")
        {
            return false;
        }
        if lower == "'unsafe-inline'" {
            unsafe_inline = true;
        }
    }
    unsafe_inline
}

/// *Does url match source list* (section 6.7.2.7): an empty list, or
/// `'none'`, matches nothing, as `'none'` matches no URL as an
/// expression.
fn url_matches_list(url: &Url, list: &[String], origin: &Origin, redirects: usize) -> bool {
    list.iter()
        .any(|expression| url_matches_expression(url, expression, origin, redirects))
}

/// *Does url match expression* (section 6.7.2.8).
fn url_matches_expression(url: &Url, expression: &str, origin: &Origin, redirects: usize) -> bool {
    let scheme = url.scheme();
    let origin_scheme = match origin {
        Origin::Tuple { scheme, .. } => Some(scheme.as_str()),
        Origin::Opaque(_) => None,
    };
    if expression == "*" {
        return matches!(scheme, "http" | "https") || origin_scheme == Some(scheme);
    }
    if expression.eq_ignore_ascii_case("'self'") {
        return scheme != "blob" && matches_self(url, origin);
    }
    if let Some(source_scheme) = scheme_source(expression) {
        return scheme_part_matches(source_scheme, scheme);
    }
    let Some(source) = HostSource::parse(expression) else {
        return false;
    };
    match source.scheme {
        Some(source_scheme) if !scheme_part_matches(source_scheme, scheme) => return false,
        None if !origin_scheme.is_some_and(|own| scheme_part_matches(own, scheme)) => {
            return false;
        }
        _ => {}
    }
    let Some(Host::Domain(host)) = url.host() else {
        return false;
    };
    if !host_part_matches(source.host, host) || !port_part_matches(source.port, url) {
        return false;
    }
    match source.path {
        Some(path) if !path.is_empty() && redirects == 0 => {
            path_part_matches(path, &url.path_str())
        }
        _ => true,
    }
}

/// `'self'` (section 6.7.2.8): the origin, or the same host and port,
/// or default ports, upgraded to a secure scheme.
fn matches_self(url: &Url, origin: &Origin) -> bool {
    if url.origin() == *origin {
        return true;
    }
    let Origin::Tuple {
        scheme: own_scheme,
        host: own_host,
        port: own_port,
    } = origin
    else {
        return false;
    };
    let scheme = url.scheme();
    url.host() == Some(own_host)
        && url.port() == *own_port
        && (matches!(scheme, "https" | "wss")
            || (own_scheme == "http" && matches!(scheme, "http" | "ws")))
}

/// The scheme of a scheme-source, `https:`: what comes before a final
/// colon. One that is no scheme matches no URL's, and nothing ending in a
/// colon is a host-source.
fn scheme_source(expression: &str) -> Option<&str> {
    expression.strip_suffix(':')
}

/// RFC 3986's scheme: a letter, then letters, digits, `+`, `-` and `.`.
fn is_scheme(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// A host-source's parts: `[scheme://]host[:port][path]`.
struct HostSource<'a> {
    scheme: Option<&'a str>,
    host: &'a str,
    port: Option<&'a str>,
    path: Option<&'a str>,
}

impl<'a> HostSource<'a> {
    fn parse(expression: &'a str) -> Option<HostSource<'a>> {
        let (scheme, rest) = match expression.split_once("://") {
            Some((scheme, rest)) if is_scheme(scheme) => (Some(scheme), rest),
            Some(_) => return None,
            None => (None, expression),
        };
        let end = rest.find([':', '/']).unwrap_or(rest.len());
        let (host, rest) = rest.split_at(end);
        if !is_host_part(host) {
            return None;
        }
        let (port, path) = match rest.strip_prefix(':') {
            Some(rest) => {
                let end = rest.find('/').unwrap_or(rest.len());
                let (port, path) = rest.split_at(end);
                if port != "*" && (port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit())) {
                    return None;
                }
                (Some(port), path)
            }
            None => (None, rest),
        };
        if path.contains([';', ',']) {
            return None;
        }
        Some(HostSource {
            scheme,
            host,
            port,
            path: (!path.is_empty()).then_some(path),
        })
    }
}

/// The host-part grammar: `*`, or `[*.]` labels of letters, digits and
/// `-` joined by dots, a final dot allowed.
fn is_host_part(host: &str) -> bool {
    if host == "*" {
        return true;
    }
    let host = host.strip_prefix("*.").unwrap_or(host);
    let host = host.strip_suffix('.').unwrap_or(host);
    !host.is_empty()
        && host.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// *scheme-part matching* (section 6.7.2.9), secure upgrades allowed.
fn scheme_part_matches(a: &str, b: &str) -> bool {
    // A URL's scheme is lowercase already.
    let a = a.to_ascii_lowercase();
    a == b
        || (a == "http" && b == "https")
        || (a == "ws" && matches!(b, "wss" | "http" | "https"))
        || (a == "wss" && b == "https")
}

/// *host-part matching* (section 6.7.2.10) against a domain.
fn host_part_matches(pattern: &str, host: &str) -> bool {
    // A URL's domain is lowercase already; `*` alone leaves nothing, which
    // every host ends with.
    if let Some(remaining) = pattern.strip_prefix('*') {
        return host.ends_with(&remaining.to_ascii_lowercase());
    }
    pattern.eq_ignore_ascii_case(host)
}

/// *port-part matching* (section 6.7.2.11): `*`, the URL's port, or its
/// scheme's default when it has none; no port-part matches only that.
fn port_part_matches(input: Option<&str>, url: &Url) -> bool {
    if input == Some("*") {
        return true;
    }
    let input = match input {
        Some(digits) => match digits.parse::<u32>() {
            Ok(port) => Some(port),
            Err(_) => return false,
        },
        None => None,
    };
    let port = url.port().map(u32::from);
    input == port || (port.is_none() && input == default_port(url.scheme()).map(u32::from))
}

/// *path-part matching* (section 6.7.2.12).
fn path_part_matches(a: &str, b: &str) -> bool {
    if a.is_empty() || (a == "/" && b.is_empty()) {
        return true;
    }
    let exact = !a.ends_with('/');
    let mut list_a: Vec<&str> = a.split('/').collect();
    let list_b: Vec<&str> = b.split('/').collect();
    if list_a.len() > list_b.len() || (exact && list_a.len() != list_b.len()) {
        return false;
    }
    if !exact {
        list_a.pop();
    }
    list_a
        .iter()
        .zip(&list_b)
        .all(|(a, b)| percent_decode(a.as_bytes()) == percent_decode(b.as_bytes()))
}

fn is_ascii_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{c}' | '\r' | ' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    /// The policies of `values`, each a `Content-Security-Policy` field,
    /// of a page at https://example.com/dir/page.html.
    fn csp(values: &[&str]) -> Csp {
        let mut headers = HeaderList::new();
        for value in values {
            headers.append(b"Content-Security-Policy", value.as_bytes());
        }
        Csp::from_headers(&headers, &url("https://example.com/dir/page.html"))
    }

    fn allows(policy: &str, script: &str) -> bool {
        csp(&[policy]).check_script_url(&url(script), 0).is_ok()
    }

    #[test]
    fn a_policy_parses_as_section_2_2_1_says() {
        let policy = Policy::parse(" Script-SRC  'self'\thttps:; ;script-src 'none'; img-src * ;");
        assert_eq!(
            policy.directives,
            [
                Directive {
                    name: "script-src".to_owned(),
                    value: vec!["'self'".to_owned(), "https:".to_owned()],
                },
                Directive {
                    name: "img-src".to_owned(),
                    value: vec!["*".to_owned()],
                },
            ]
        );
        // A token not ASCII is skipped; nothing left, no policy.
        assert!(Policy::parse("script-src caf\u{e9}.example").is_empty());
        assert!(Policy::parse(" ; ").is_empty());
        // A meta element may not deliver these.
        let meta =
            Policy::parse("sandbox; report-uri /r; frame-ancestors 'none'; script-src 'none'")
                .for_meta();
        assert_eq!(meta.directives.len(), 1);
    }

    #[test]
    fn each_header_value_and_each_comma_is_a_policy_and_every_one_must_allow() {
        let both = csp(&[
            "script-src 'self'",
            "script-src https://example.com, default-src https:",
        ]);
        assert_eq!(both.policies.len(), 3);
        assert!(
            both.check_script_url(&url("https://example.com/a.js"), 0)
                .is_ok()
        );
        let blocked = both
            .check_script_url(&url("https://other.example/a.js"), 0)
            .unwrap_err();
        assert_eq!(blocked.to_string(), "script-src 'self'");
        // Policies with no directive are not kept, nor report-only ones.
        assert!(csp(&[" ; ", ""]).policies.is_empty());
        let mut headers = HeaderList::new();
        headers.append(b"Content-Security-Policy-Report-Only", b"script-src 'none'");
        let report = Csp::from_headers(&headers, &url("https://example.com/"));
        assert!(report.check_inline_script().is_ok());
    }

    #[test]
    fn script_src_elem_then_script_src_then_default_src_decide() {
        let elem =
            "script-src-elem https://a.example; script-src https://b.example; default-src 'none'";
        assert!(allows(elem, "https://a.example/x.js"));
        assert!(!allows(elem, "https://b.example/x.js"));
        let src = "script-src https://b.example; default-src https://a.example";
        assert!(allows(src, "https://b.example/x.js"));
        assert!(!allows(src, "https://a.example/x.js"));
        assert!(allows(
            "default-src https://a.example",
            "https://a.example/x.js"
        ));
        assert!(!allows(
            "default-src https://a.example",
            "https://b.example/x.js"
        ));
        // A policy with no script directive leaves scripts alone.
        assert!(allows(
            "img-src 'none'; style-src 'none'",
            "https://b.example/x.js"
        ));
    }

    #[test]
    fn none_and_an_empty_list_match_nothing_and_self_its_origin() {
        assert!(!allows("script-src 'none'", "https://example.com/a.js"));
        assert!(!allows("script-src", "https://example.com/a.js"));
        assert!(allows(
            "script-src 'none' https://example.com",
            "https://example.com/a.js"
        ));
        assert!(allows(
            "script-src 'SELF'",
            "https://example.com/other/a.js"
        ));
        assert!(!allows(
            "script-src 'self'",
            "https://example.com:8443/a.js"
        ));
        assert!(!allows("script-src 'self'", "http://example.com/a.js"));
        assert!(!allows("script-src 'self'", "https://sub.example.com/a.js"));
        assert!(!allows("script-src 'self'", "blob:https://example.com/x"));
        // An http: page's 'self' takes https: on the same host.
        let mut headers = HeaderList::new();
        headers.append(b"Content-Security-Policy", b"script-src 'self'");
        let insecure = Csp::from_headers(&headers, &url("http://example.com/"));
        assert!(
            insecure
                .check_script_url(&url("https://example.com/a.js"), 0)
                .is_ok()
        );
        assert!(
            insecure
                .check_script_url(&url("http://example.com/a.js"), 0)
                .is_ok()
        );
        assert!(
            insecure
                .check_script_url(&url("https://example.com:8443/a.js"), 0)
                .is_err()
        );
        // Another scheme's origin: itself, and `*` its own scheme.
        let mut headers = HeaderList::new();
        headers.append(b"Content-Security-Policy", b"script-src 'self'");
        let ws = Csp::from_headers(&headers, &url("ws://example.com/"));
        assert!(ws.check_script_url(&url("ws://example.com/a"), 0).is_ok());
        let mut headers = HeaderList::new();
        headers.append(b"Content-Security-Policy", b"script-src *");
        let ftp = Csp::from_headers(&headers, &url("ftp://example.com/"));
        assert!(ftp.check_script_url(&url("ftp://example.com/a"), 0).is_ok());
        assert!(ftp.check_script_url(&url("ws://example.com/a"), 0).is_err());
    }

    #[test]
    fn schemes_paths_and_hosts_match_as_their_parts_say() {
        for (a, b, matches) in [
            ("http", "http", true),
            ("http", "https", true),
            ("https", "http", false),
            ("ws", "wss", true),
            ("ws", "http", true),
            ("ws", "https", true),
            ("wss", "https", true),
            ("wss", "http", false),
            ("HTTPS", "https", true),
        ] {
            assert_eq!(scheme_part_matches(a, b), matches, "{a} {b}");
        }
        assert!(path_part_matches("", "/anything"));
        assert!(path_part_matches("/", ""));
        assert!(path_part_matches("/", "/a/b"));
        assert!(!path_part_matches("/a/b/", "/a/"));
        assert!(host_part_matches("*", "anything.example"));
        assert!(host_part_matches("EXAMPLE.com", "example.com"));
        assert!(allows("script-src https://*", "https://any.example/a.js"));
        assert!(allows("script-src EXAMPLE.COM", "https://example.com/a.js"));
    }

    #[test]
    fn schemes_and_stars_match_as_section_6_7_2_8_says() {
        assert!(allows("script-src https:", "https://any.example/a.js"));
        assert!(!allows("script-src https:", "http://any.example/a.js"));
        assert!(allows("script-src http:", "https://any.example/a.js"));
        assert!(allows("script-src data:", "data:text/javascript,1"));
        assert!(allows("script-src *", "http://any.example/a.js"));
        assert!(!allows("script-src *", "data:text/javascript,1"));
        assert!(!allows("script-src *", "blob:https://example.com/x"));
    }

    #[test]
    fn hosts_ports_and_paths_match_as_section_6_7_2_8_says() {
        assert!(allows("script-src example.com", "https://EXAMPLE.com/a.js"));
        assert!(!allows("script-src example.com", "http://example.com/a.js"));
        assert!(allows(
            "script-src http://example.com",
            "https://example.com/a.js"
        ));
        assert!(!allows(
            "script-src https://example.com",
            "http://example.com/a.js"
        ));
        assert!(allows(
            "script-src *.example.com",
            "https://a.b.example.com/a.js"
        ));
        assert!(!allows(
            "script-src *.example.com",
            "https://example.com/a.js"
        ));
        assert!(allows("script-src *", "https://127.0.0.1/a.js"));
        assert!(!allows("script-src 127.0.0.1", "https://127.0.0.1/a.js"));
        assert!(!allows(
            "script-src https://example.com",
            "https://example.com:8443/a.js"
        ));
        assert!(allows(
            "script-src https://example.com:8443",
            "https://example.com:8443/a.js"
        ));
        assert!(allows(
            "script-src https://example.com:*",
            "https://example.com:9/a.js"
        ));
        assert!(allows(
            "script-src https://example.com:443",
            "https://example.com/a.js"
        ));
        assert!(allows(
            "script-src https://example.com/js/",
            "https://example.com/js/a/b.js"
        ));
        assert!(!allows(
            "script-src https://example.com/js/",
            "https://example.com/css/a.js"
        ));
        assert!(allows(
            "script-src https://example.com/js/a.js",
            "https://example.com/js/a.js"
        ));
        assert!(!allows(
            "script-src https://example.com/js/a.js",
            "https://example.com/js/a.jsx"
        ));
        assert!(allows(
            "script-src https://example.com/a%20b.js",
            "https://example.com/a b.js"
        ));
        // After a redirect the path is not looked at.
        let policy = csp(&["script-src https://example.com/js/"]);
        let elsewhere = url("https://example.com/other.js");
        assert!(policy.check_script_url(&elsewhere, 0).is_err());
        assert!(policy.check_script_url(&elsewhere, 1).is_ok());
        // What does not parse matches nothing.
        for broken in [
            "script-src https://",
            "script-src a..b",
            "script-src exa_mple.com",
            "script-src x:y",
        ] {
            assert!(!allows(broken, "https://example.com/a.js"), "{broken}");
        }
        // A host the grammar does not allow matches no URL, even its own.
        assert!(!allows(
            "script-src exa_mple.com",
            "https://exa_mple.com/a.js"
        ));
        // Paths compare percent-decoded.
        assert!(allows(
            "script-src https://example.com/a%41.js",
            "https://example.com/aA.js"
        ));
    }

    #[test]
    fn inline_scripts_need_unsafe_inline_with_no_nonce_hash_or_strict_dynamic() {
        assert!(csp(&["img-src 'none'"]).check_inline_script().is_ok());
        assert!(csp(&["script-src 'self'"]).check_inline_script().is_err());
        assert!(
            csp(&["script-src 'unsafe-inline'"])
                .check_inline_script()
                .is_ok()
        );
        assert!(
            csp(&["default-src 'UNSAFE-INLINE'"])
                .check_inline_script()
                .is_ok()
        );
        for overriding in ["'nonce-abc'", "'sha256-abc='", "'strict-dynamic'"] {
            let policy = format!("script-src 'unsafe-inline' {overriding}");
            assert!(
                csp(&[&policy]).check_inline_script().is_err(),
                "{overriding}"
            );
        }
        let two = csp(&["script-src 'unsafe-inline'", "default-src 'self'"]);
        assert_eq!(
            two.check_javascript_url().unwrap_err().to_string(),
            "default-src 'self'"
        );
    }

    #[test]
    fn a_meta_policy_adds_to_those_of_the_header_and_too_many_block_all() {
        let mut policy = csp(&["script-src 'unsafe-inline' https:"]);
        policy.add_meta("script-src https://example.com; report-uri /r");
        assert!(
            policy
                .check_script_url(&url("https://example.com/a.js"), 0)
                .is_ok()
        );
        assert!(
            policy
                .check_script_url(&url("https://other.example/a.js"), 0)
                .is_err()
        );
        assert!(policy.check_inline_script().is_err());
        let mut many = Csp::none(url("https://example.com/").origin());
        for _ in 0..MAX_POLICIES {
            many.add_meta("img-src 'none'");
        }
        assert!(many.check_inline_script().is_ok());
        many.add_meta("img-src 'none'");
        assert!(many.check_inline_script().is_err());
        assert!(
            many.check_script_url(&url("https://example.com/a.js"), 0)
                .is_err()
        );
    }

    fn request(policy: &Csp, destination: Destination, target: &str) -> Vec<Violation> {
        let target = url(target);
        policy.request_violations(destination, (&target, &target), 0, "", true)
    }

    #[test]
    fn each_destination_is_held_to_its_fetch_directive_falling_back_as_section_6_8_4_says() {
        let policy = csp(&["default-src 'none'; img-src *; style-src 'self'; \
             connect-src https://api.example; child-src https://child.example; \
             script-src https://scripts.example"]);
        let allowed = |destination, target: &str| request(&policy, destination, target).is_empty();
        assert!(allowed(
            Destination::Image,
            "https://anywhere.example/i.png"
        ));
        assert!(allowed(Destination::Style, "https://example.com/s.css"));
        assert!(!allowed(Destination::Style, "https://other.example/s.css"));
        assert!(allowed(Destination::Empty, "https://api.example/x"));
        assert!(!allowed(Destination::Empty, "https://example.com/x"));
        // Frames and workers fall back to child-src, a worker then to
        // script-src.
        assert!(allowed(Destination::Iframe, "https://child.example/"));
        assert!(allowed(Destination::Worker, "https://child.example/w.js"));
        assert!(!allowed(
            Destination::Worker,
            "https://scripts.example/w.js"
        ));
        let no_child = csp(&["script-src https://scripts.example; default-src 'none'"]);
        assert!(
            request(
                &no_child,
                Destination::Worker,
                "https://scripts.example/w.js"
            )
            .is_empty()
        );
        assert!(!request(&no_child, Destination::Iframe, "https://scripts.example/").is_empty());
        // A document's own navigation is held to none.
        assert!(allowed(Destination::Document, "https://other.example/"));
        let refused = request(&policy, Destination::Empty, "https://example.com/x");
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].to_string(), "connect-src https://api.example");
        assert_eq!(refused[0].effective, "connect-src");
        assert_eq!(refused[0].blocked, "https://example.com/x");
        assert_eq!(refused[0].what, "the connection to https://example.com/x");
        assert_eq!(refused[0].disposition, Disposition::Enforce);
        assert!(
            refused[0]
                .policy
                .starts_with("default-src 'none'; img-src *")
        );
        let framed = request(&policy, Destination::Iframe, "https://x.example/");
        assert_eq!(
            (framed[0].directive.as_str(), framed[0].effective.as_str()),
            ("child-src", "frame-src")
        );
    }

    #[test]
    fn nonces_and_strict_dynamic_decide_a_scripts_and_a_stylesheets_request() {
        let policy = csp(&[
            "script-src 'nonce-abc' 'strict-dynamic' https://example.com; \
             style-src 'NONCE-xyz'; img-src 'nonce-abc'",
        ]);
        let allowed = |destination, nonce: &str, parser: bool| {
            let target = url("https://other.example/a");
            policy
                .request_violations(destination, (&target, &target), 0, nonce, parser)
                .is_empty()
        };
        assert!(allowed(Destination::Script, "abc", true));
        assert!(!allowed(Destination::Script, "abd", true));
        // 'strict-dynamic': the URLs aside, what the parser did not insert
        // runs.
        assert!(allowed(Destination::Script, "", false));
        assert!(!request(&policy, Destination::Script, "https://example.com/a.js").is_empty());
        assert!(allowed(Destination::Style, "xyz", true));
        assert!(!allowed(Destination::Style, "abc", true));
        // An image's request has no nonce that matches.
        assert!(!allowed(Destination::Image, "abc", true));
    }

    #[test]
    fn report_only_policies_are_kept_and_report_without_refusing() {
        let page = url("https://example.com/");
        let mut headers = HeaderList::new();
        headers.append(
            b"Content-Security-Policy-Report-Only",
            b"img-src 'none', script-src 'self'",
        );
        headers.append(b"Content-Security-Policy", b"img-src https:");
        let both = Csp::from_headers(&headers, &page);
        let found = request(&both, Destination::Image, "http://example.com/i.png");
        assert_eq!(
            found
                .iter()
                .map(|violation| (violation.value.as_str(), violation.disposition))
                .collect::<Vec<_>>(),
            [
                ("https:", Disposition::Enforce),
                ("'none'", Disposition::Report)
            ]
        );
        let reported = request(&both, Destination::Image, "https://example.com/i.png");
        assert_eq!(reported.len(), 1);
        assert!(enforced(reported).is_ok());
        assert!(
            both.inline_violations(Inline::Script, "", "")
                .iter()
                .all(|violation| violation.disposition == Disposition::Report)
        );
        assert!(both.check_inline_script().is_ok());
        // A report-only policy delivers no sandbox, and refuses no frame.
        let mut headers = HeaderList::new();
        headers.append(
            b"Content-Security-Policy-Report-Only",
            b"sandbox; frame-ancestors 'none'",
        );
        let report = Csp::from_headers(&headers, &page);
        assert!(report.sandboxes().is_empty());
        assert!(
            report
                .check_frame_ancestors(&[url("https://a.example/").origin()])
                .is_none()
        );
    }

    #[test]
    fn a_documents_violations_wait_in_its_sink_and_an_inheritor_has_its_own() {
        let page = csp(&["img-src 'none'"]).with_sink();
        let copy = page.clone();
        let found = request(&copy, Destination::Image, "https://example.com/i.png");
        copy.record(&found);
        assert_eq!(page.take_violations().len(), 1);
        assert!(page.take_violations().is_empty());
        let child = page.inherited();
        child.record(&found);
        assert!(page.take_violations().is_empty());
        assert_eq!(child.take_violations().len(), 1);
        // Without a sink nothing is kept; past the bound the newest go.
        let bare = csp(&["img-src 'none'"]);
        bare.record(&found);
        assert!(bare.take_violations().is_empty());
        for _ in 0..MAX_RECORDED + 5 {
            page.record(&found);
        }
        assert_eq!(page.take_violations().len(), MAX_RECORDED);
    }

    #[test]
    fn base64_is_rfc_4648s() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xfb, 0xff]), "+/8=");
    }

    #[test]
    fn inline_content_is_allowed_by_nonce_hash_or_unsafe_inline_as_section_6_7_3_3_says() {
        let script = "console.log('hashed')";
        let policy = csp(&["script-src 'nonce-abc' \
             'sha256-Ly+EvtafdANT0xlOxh/fd3NF3f5hCoaFad0hrg25DRI=' \
             'SHA384-rzh2fR62o6squOaj7F0JlhI4ddmHGG9ZrFVO1/6ozP8UwCu0TAsfAzztuofNtZcy' \
             'report-sample'; style-src 'unsafe-hashes' \
             'sha512-ocSA23LaMKg3bDI5FncQNvuQfjXdujM4i74BJygee6ohNiax3HNTrCdGP7SNV7qePx2OuVoViiM0bLOu0-AC9g=='"]);
        let allowed = |kind, nonce: &str, source: &str| {
            policy.inline_violations(kind, nonce, source).is_empty()
        };
        assert!(allowed(Inline::Script, "abc", "anything"));
        assert!(allowed(Inline::Script, "", script));
        // Each algorithm digests as its name says.
        let sha384 = csp(&["script-src \
             'sha384-rzh2fR62o6squOaj7F0JlhI4ddmHGG9ZrFVO1/6ozP8UwCu0TAsfAzztuofNtZcy'"]);
        assert!(
            sha384
                .inline_violations(Inline::Script, "", script)
                .is_empty()
        );
        assert!(!allowed(Inline::Script, "", "console.log('hashed') "));
        assert!(!allowed(Inline::Script, "abd", "anything"));
        // An attribute's or a javascript: URL's: no nonce, and a hash only
        // with 'unsafe-hashes'.
        assert!(!allowed(Inline::ScriptAttribute, "abc", script));
        assert!(!allowed(Inline::Navigation, "", script));
        assert!(allowed(Inline::StyleAttribute, "", "color: red"));
        assert!(allowed(Inline::Style, "", "color: red"));
        assert!(!allowed(Inline::StyleAttribute, "", "color: blue"));
        // 'report-sample' keeps the first 40 characters.
        let long = "x".repeat(MAX_SAMPLE + 10);
        let found = policy.inline_violations(Inline::Script, "", &long);
        assert_eq!(found[0].sample, "x".repeat(MAX_SAMPLE));
        assert_eq!(found[0].blocked, "inline");
        assert_eq!(found[0].effective, "script-src-elem");
        // 'unsafe-inline' is all inline behavior with no nonce or hash, and
        // for a script with no 'strict-dynamic'.
        let open = csp(&["script-src 'unsafe-inline' 'strict-dynamic'; \
             style-src 'unsafe-inline' 'strict-dynamic'"]);
        assert!(!open.inline_violations(Inline::Script, "", "1").is_empty());
        assert!(open.inline_violations(Inline::Style, "", "p {}").is_empty());
        let attr = csp(&["script-src-attr 'unsafe-inline'; script-src 'none'"]);
        assert!(
            attr.inline_violations(Inline::ScriptAttribute, "", "1")
                .is_empty()
        );
        assert!(!attr.inline_violations(Inline::Script, "", "1").is_empty());
    }

    #[test]
    fn eval_needs_unsafe_eval_and_webassembly_wasm_unsafe_eval_or_it() {
        let strict = csp(&["script-src 'self' 'report-sample'"]);
        let found = strict.eval_violations("1 + 1");
        assert_eq!(found.len(), 1);
        assert_eq!(
            (
                found[0].blocked.as_str(),
                found[0].effective.as_str(),
                found[0].sample.as_str()
            ),
            ("eval", "script-src", "1 + 1")
        );
        assert_eq!(strict.wasm_violations()[0].blocked, "wasm-eval");
        let wasm = csp(&["default-src 'wasm-unsafe-eval'"]);
        assert!(wasm.wasm_violations().is_empty());
        assert!(!wasm.eval_violations("1").is_empty());
        let both = csp(&["script-src 'unsafe-eval'"]);
        assert!(both.eval_violations("1").is_empty() && both.wasm_violations().is_empty());
        // Only script-src and default-src are asked.
        assert!(
            csp(&["script-src-elem 'none'"])
                .eval_violations("1")
                .is_empty()
        );
        // base-uri and form-action fall back to nothing.
        let none = csp(&["default-src 'none'"]);
        let elsewhere = url("https://other.example/");
        assert!(none.base_uri_violations(&elsewhere).is_empty());
        assert!(
            none.form_action_violations((&elsewhere, &elsewhere), 0)
                .is_empty()
        );
        // A policy from a <meta> is no header's.
        let mut meta = Csp::none(url("https://example.com/").origin());
        meta.add_meta("script-src 'none'");
        assert!(!meta.has_header_policy());
        assert!(strict.has_header_policy());
    }
}
