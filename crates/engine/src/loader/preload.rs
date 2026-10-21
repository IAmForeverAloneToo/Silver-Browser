//! Preloads (M38, `docs/design/http.md`): the requests the preload
//! scanner's hints and `<link rel=preload>`s make, built as the page's
//! own requests for the same things are, so that those take them
//! ([`super::Network::preload`]).

use dom::{Document, NodeId};
use html::preload::{Hint, HintKind};
use net::csp::Csp;
use net::{Destination, ReferrerPolicy, Request, Url};

use super::{Network, Page, with_cors_settings};

/// What a hint or a `<link rel=preload>`'s `as` is fetched as: the
/// destinations the page's own requests have.
fn destination(as_: &str) -> Option<Destination> {
    match as_.to_ascii_lowercase().as_str() {
        "style" => Some(Destination::Style),
        "script" => Some(Destination::Script),
        "image" => Some(Destination::Image),
        _ => None,
    }
}

/// The policies the page's requests are held to now: its Content
/// Security Policy and its referrer policy, from its response and the
/// `<meta>`s parsed so far. The policy records nothing: a preload says
/// no violation, which the request that takes its place would say at
/// its own time and element; one its policy would find, enforced or
/// reported, is not made at all.
pub(crate) struct Policies {
    pub(crate) csp: Csp,
    pub(crate) referrer: ReferrerPolicy,
}

impl Policies {
    pub(crate) fn of(page: &mut Page, document: &Document) -> Policies {
        Policies {
            csp: Csp::from_parts(page.csp(document).parts()),
            referrer: page.referrer_policy(document),
        }
    }

    /// Whether `request` is one to preload: no violation of the policy
    /// at its first URL.
    fn allow(&self, request: &Request) -> bool {
        self.csp
            .request_violations(
                request.destination,
                (&request.url, &request.url),
                0,
                &request.nonce,
                request.parser_inserted,
            )
            .is_empty()
    }
}

/// The nonce of an element of the attributes `attrs`, as its request
/// carries it (M33.1): none when an attribute looks like markup a
/// `<script>` or `<style>` swallowed; white space around it not its.
fn nonce(attrs: &[(String, String)]) -> String {
    let injected = |text: &str| {
        let lower = text.to_ascii_lowercase();
        lower.contains("<script") || lower.contains("<style")
    };
    if attrs
        .iter()
        .any(|(name, value)| injected(name) || injected(value))
    {
        return String::new();
    }
    attrs
        .iter()
        .find(|(name, _)| name == "nonce")
        .map_or("", |(_, value)| value.as_str())
        .trim_matches(|c: char| c.is_ascii_whitespace())
        .to_owned()
}

/// The request for `url` as `destination` from `page`, with the CORS
/// settings `crossorigin` and the referrer policy `referrerpolicy`: as
/// a `<link rel=stylesheet>`'s, a classic `<script>`'s or an `<img>`'s
/// are made, held to `policies`. None for a page that fetches nothing.
fn request(
    page: &Page,
    policies: &Policies,
    url: Url,
    destination: Destination,
    crossorigin: Option<&str>,
    referrerpolicy: Option<&str>,
) -> Option<Request> {
    let mut request = Request::get(url, destination)
        .initiated_by(page.initiator()?)
        .with_csp(policies.csp.clone());
    request = match destination {
        Destination::Script => request.with_body_limit(crate::scripts::MAX_SCRIPT_BYTES),
        Destination::Image => request.with_body_limit(images::MAX_IMAGE_BYTES),
        _ => request,
    };
    request.referrer_policy = Some(
        referrerpolicy
            .and_then(ReferrerPolicy::parse)
            .unwrap_or(policies.referrer),
    );
    Some(with_cors_settings(request, crossorigin))
}

/// What the scanner's hints of a document still parsing need: the base
/// URL its first `<base href>` gave, once one has; and whether a
/// `<meta>` setting a policy came, after which nothing is preloaded.
#[derive(Default)]
pub(crate) struct Hints {
    base: Option<Url>,
    stopped: bool,
}

impl Hints {
    /// Preloads what `hints`, the scanner's for `page`, ask for: a
    /// stylesheet; a classic script, when scripts run; an image; and a
    /// `<link rel=preload>` as a style, script or image.
    pub(crate) fn preload(
        &mut self,
        hints: Vec<Hint>,
        page: &mut Page,
        document: &Document,
        scripting: bool,
        network: &mut Network,
    ) {
        let policies = Policies::of(page, document);
        for hint in hints {
            if self.stopped {
                return;
            }
            let resolve = |href: &str| match &self.base {
                Some(base) => Url::parse_with(href, Some(base), page.encoding()).ok(),
                None => page.resolve_from_document(href),
            };
            let (destination, explicit) = match hint.kind {
                HintKind::Base => {
                    self.base = page.resolve_from_document(&hint.url);
                    continue;
                }
                HintKind::Policy => {
                    self.stopped = true;
                    continue;
                }
                HintKind::Stylesheet => {
                    let css = hint.attr("type").is_none_or(|kind| {
                        kind.is_empty()
                            || net::MimeType::parse(kind).is_some_and(|t| t.essence() == "text/css")
                    });
                    if !css || hint.attr("disabled").is_some() {
                        continue;
                    }
                    (Destination::Style, false)
                }
                HintKind::Script => {
                    let classic =
                        crate::scripts::script_type_of(hint.attr("type"), hint.attr("language"))
                            == Some(crate::scripts::ScriptType::Classic);
                    if !scripting || !classic || hint.attr("nomodule").is_some() {
                        continue;
                    }
                    (Destination::Script, false)
                }
                HintKind::Image => (Destination::Image, false),
                HintKind::Preload => match hint.attr("as").and_then(destination) {
                    Some(destination) => (destination, true),
                    None => continue,
                },
            };
            let Some(url) = resolve(&hint.url) else {
                continue;
            };
            if let Some(mut request) = request(
                page,
                &policies,
                url,
                destination,
                hint.attr("crossorigin"),
                hint.attr("referrerpolicy"),
            ) {
                // As the element's own: its nonce, the parser's, and a
                // link's `noreferrer` (M33).
                request.nonce = nonce(&hint.attrs);
                request.parser_inserted = hint.kind == HintKind::Script;
                if hint.kind == HintKind::Stylesheet
                    && hint.attr("rel").is_some_and(|rel| {
                        rel.split(|c: char| c.is_ascii_whitespace())
                            .any(|t| t.eq_ignore_ascii_case("noreferrer"))
                    })
                {
                    request.referrer = net::Referrer::None;
                }
                if policies.allow(&request) {
                    network.preload(page.id(), request, explicit);
                }
            }
        }
    }
}

/// Preloads what the `<link rel=preload>`s of `document`, `page`'s, ask
/// for (M38): those a script made, or the scanner did not see.
pub(crate) fn preload_links(document: &Document, page: &mut Page, network: &mut Network) {
    let mut policies = None;
    let links = document.subtree(document.root()).filter(|&node| {
        document.tag(node) == Some("link")
            && document.attr(node, "rel").is_some_and(|rel| {
                rel.split(|c: char| c.is_ascii_whitespace())
                    .any(|t| t.eq_ignore_ascii_case("preload"))
            })
    });
    let links: Vec<NodeId> = links.collect();
    for node in links {
        let Some(destination) = document.attr(node, "as").and_then(destination) else {
            continue;
        };
        let Some(url) = document
            .attr(node, "href")
            .filter(|href| !href.is_empty())
            .and_then(|href| page.resolve(document, href))
        else {
            continue;
        };
        let policies = policies.get_or_insert_with(|| Policies::of(page, document));
        if let Some(request) = request(
            page,
            policies,
            url,
            destination,
            document.attr(node, "crossorigin"),
            document.attr(node, "referrerpolicy"),
        ) && policies.allow(&request)
        {
            network.preload(page.id(), request, true);
        }
    }
}
