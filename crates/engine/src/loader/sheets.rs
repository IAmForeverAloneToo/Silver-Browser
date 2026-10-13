//! A document's stylesheets (docs/design/net.md, "The engine's
//! loader"): the `<link>` and `<style>` elements that own them, in tree
//! order, each with its sheet and the sheets that sheet imports, fetched
//! or failed or on their way. Every sheet is fetched whatever its media
//! or its imports' conditions, which wrap its rules for the cascade to
//! evaluate (docs/design/media.md); only one whose media holds when the
//! parser meets it holds the page back.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use dom::{Change, Document, NodeId};
use encoding::Encoding;
use net::csp::{Csp, Inline};
use net::{
    AddressSpace, Destination, Initiator, MimeType, Origin, Referrer, ReferrerPolicy, Request,
    Response, Url,
};
use style::Media;

use super::{FetchId, LoadError, Network, PageId, Target, rules};
use crate::sandbox::Sandbox;

/// How deep imports go: a sheet's imports are one deeper than it, and
/// one deeper than this is not fetched.
pub(crate) const MAX_IMPORT_DEPTH: usize = 16;

/// One document's stylesheets.
pub(crate) struct Page {
    context: Context,
    /// The encoding the document is in when the engine cannot decode
    /// it yet, and decoded it as UTF-8 instead.
    undecodable: Option<Encoding>,
    /// The language the response's `Content-Language` gave the
    /// document, when it named one (M16).
    language: Option<String>,
    /// The document's type when it is no HTML document (M32: an SVG
    /// document's `image/svg+xml`).
    content_type: Option<String>,
    /// The elements that may own a sheet, `<link>` and `<style>`, in
    /// tree order.
    owners: Vec<Owner>,
    /// The document's Content Security Policy (M24.6): its response's,
    /// and its `<meta>` elements' as they come.
    csp: Csp,
    /// The `<meta>` elements whose policies are in `csp`.
    metas: Vec<NodeId>,
    /// The document's sandboxing flags (M33.2).
    sandbox: Sandbox,
    /// Each element's nonce once its attribute was hidden or a script set
    /// it (M33.1, HTML's `[[CryptographicNonce]]`).
    nonces: HashMap<NodeId, String>,
    /// The elements whose `style` attribute the CSSOM wrote last, which
    /// the policies do not look at (M33.1); what they said of each other
    /// element's, by its value; the `<style>` elements they refused,
    /// whose `error` is to fire; and the `<base>` URL they last refused.
    cssom_styles: HashSet<NodeId>,
    style_checks: RefCell<HashMap<NodeId, (String, bool)>>,
    refused_styles: Vec<NodeId>,
    refused_base: RefCell<Option<Url>>,
    /// Its response's `Referrer-Policy` (M33.3), or its creator's for a
    /// local document; and the referrer of the request that brought it,
    /// `document.referrer`.
    referrer_policy: Option<ReferrerPolicy>,
    referrer: String,
}

/// What a page's fetches are made with.
struct Context {
    id: PageId,
    /// The document's URL. A document given to the engine rather than
    /// fetched has none, and fetches nothing.
    url: Option<Url>,
    /// The document's character encoding: what its URLs are parsed
    /// with, and a stylesheet's environment encoding.
    encoding: Encoding,
    /// The address space of the server the document came from.
    address_space: AddressSpace,
    /// The document's origin (M27.2), fixed when the page is made: its
    /// URL's for a network URL, else an opaque origin of its own, so
    /// that it is same origin with itself and with nothing else.
    origin: Origin,
    /// For a frame's `about:blank` or `about:srcdoc` document (M33.2),
    /// what it took from the document that made the frame: the base URL
    /// its own URL cannot give (HTML's "about base URL"), the URL its
    /// requests are made from, and whether it is a secure context.
    fallback_base: Option<Url>,
    creator_url: Option<Url>,
    secure: Option<bool>,
}

impl Context {
    /// The document's fallback base URL (HTML section 2.4.1): the about
    /// base URL a frame's document took, or its own URL.
    fn fallback(&self) -> Option<Url> {
        self.fallback_base.clone().or_else(|| self.url.clone())
    }
}

/// What a frame's document takes from the document that made the frame
/// (M33.2, `docs/design/hardening.md`): an `about:blank` or
/// `about:srcdoc` document its origin, its base URL, the URL its
/// requests are made from and its being a secure context; every local
/// document, a `data:` URL's too, its Content Security Policy (HTML's
/// policy container).
#[derive(Debug, Clone)]
pub(crate) struct Creator {
    pub(crate) origin: Origin,
    pub(crate) url: Option<Url>,
    pub(crate) base_url: Option<Url>,
    pub(crate) address_space: AddressSpace,
    pub(crate) secure: bool,
    pub(crate) csp: Csp,
    /// Its referrer policy (M33.3), which a local document takes.
    pub(crate) referrer_policy: ReferrerPolicy,
}

/// An element that may own a sheet.
struct Owner {
    node: NodeId,
    /// What the element's sheet was made from; a change to it makes the
    /// sheet again.
    key: Key,
    /// The element's `title`, empty when it has none.
    title: String,
    /// The element's `media`, which a change to does not fetch the
    /// sheet again: it says where the sheet's rules apply.
    media: Option<String>,
    /// The environment encoding of a fetched sheet: the `<link>`'s
    /// `charset`, or the document's encoding.
    environment: Encoding,
    /// Whether the document is not shown until the sheet has arrived or
    /// failed: the parser made the element before `<body>`, and its
    /// media held then.
    blocking: bool,
    /// Whether no script runs until the sheet has arrived or failed (HTML
    /// section 4.2.6, "a style sheet that is blocking scripts"): the
    /// parser made the element, and its media held then (M24.3).
    blocks_scripts: bool,
    sheet: Slot,
}

impl Owner {
    /// Whether it is a `<style>` its document's policies refused (M33.1):
    /// the only one that fails with no fetch.
    fn refused(&self) -> bool {
        matches!((&self.key, &self.sheet), (Key::Style { .. }, Slot::Failed))
    }
}

/// The attributes, and for `<style>` the text, that decide an element's
/// sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Key {
    Link {
        href: Option<String>,
        rel: Option<String>,
        kind: Option<String>,
        disabled: bool,
        /// Its CORS settings attribute (M27.3).
        crossorigin: Option<String>,
    },
    Style {
        text: String,
        kind: Option<String>,
    },
}

/// Where a sheet is.
enum Slot {
    /// The element makes no sheet.
    None,
    /// The sheet is being fetched.
    Waiting(FetchId),
    /// It could not be fetched, or was not a stylesheet, and is left out.
    Failed,
    Ready(Box<Sheet>),
}

struct Sheet {
    rules: css::Stylesheet,
    /// The URL the sheet was requested from, which its imports resolve
    /// against; `None` for a `<style>` element's.
    location: Option<Url>,
    /// Its character encoding, its imports' environment encoding.
    encoding: Encoding,
    /// Its `@import` rules, in order, each with the sheet it fetches,
    /// whose rules take the rule's place.
    imports: Vec<Imported>,
}

/// An `@import` rule and the sheet it fetches: `Slot::None` when it
/// fetches none, as for a cycle or an import too deep.
struct Imported {
    import: rules::Import,
    sheet: Slot,
}

/// An element found by a walk over the document.
struct Found {
    node: NodeId,
    key: Key,
    title: String,
    media: Option<String>,
    charset: Option<String>,
    /// The base URL as it was when the parser inserted the element: that
    /// of the first `<base href>` before it in tree order.
    base_here: Option<Url>,
    before_body: bool,
    /// Its `nonce`, which its request carries (M33.1).
    nonce: String,
    /// Its `referrerpolicy`, and whether its `rel` says `noreferrer`
    /// (M33.3).
    referrer_policy: Option<ReferrerPolicy>,
    noreferrer: bool,
}

/// A document's `style` attributes, as its policies allow them (M33.1):
/// one the CSSOM wrote always; another as `style-src-attr` allows it by
/// `'unsafe-inline'` or its hash with `'unsafe-hashes'`, a refusal
/// recorded once for each value.
impl style::InlineStyles for Page {
    fn allowed(&self, element: NodeId, value: &str) -> bool {
        if self.cssom_styles.contains(&element) || self.csp.is_empty() {
            return true;
        }
        let mut checks = self.style_checks.borrow_mut();
        if let Some((checked, allowed)) = checks.get(&element)
            && checked == value
        {
            return *allowed;
        }
        let violations = self
            .csp
            .inline_violations(Inline::StyleAttribute, "", value);
        self.csp.record(&violations);
        let allowed = net::csp::enforced(violations).is_ok();
        checks.insert(element, (value.to_owned(), allowed));
        allowed
    }
}

impl Page {
    /// The page of a document given to the engine: its `<style>`
    /// elements make sheets, and nothing is fetched.
    pub(crate) fn given(id: PageId) -> Page {
        let origin = Origin::new_opaque();
        Page {
            context: Context {
                id,
                url: None,
                encoding: encoding::UTF_8,
                address_space: AddressSpace::Public,
                origin: origin.clone(),
                fallback_base: None,
                creator_url: None,
                secure: None,
            },
            undecodable: None,
            language: None,
            content_type: None,
            owners: Vec::new(),
            csp: Csp::none(origin).with_sink(),
            metas: Vec::new(),
            sandbox: Sandbox::NONE,
            nonces: HashMap::new(),
            cssom_styles: HashSet::new(),
            style_checks: RefCell::default(),
            refused_styles: Vec::new(),
            refused_base: RefCell::default(),
            referrer_policy: None,
            referrer: String::new(),
        }
    }

    /// The page of a document fetched from `url`, decoded with
    /// `encoding`, from a server in `address_space`. `undecodable` is
    /// the encoding it is in when the engine cannot decode that one.
    pub(crate) fn fetched(
        id: PageId,
        url: Url,
        encoding: Encoding,
        undecodable: Option<Encoding>,
        address_space: AddressSpace,
    ) -> Page {
        let origin = url.origin();
        let csp = Csp::none(origin.clone()).with_sink();
        Page {
            context: Context {
                id,
                url: Some(url),
                encoding,
                address_space,
                origin,
                fallback_base: None,
                creator_url: None,
                secure: None,
            },
            undecodable,
            language: None,
            content_type: None,
            owners: Vec::new(),
            csp,
            metas: Vec::new(),
            sandbox: Sandbox::NONE,
            nonces: HashMap::new(),
            cssom_styles: HashSet::new(),
            style_checks: RefCell::default(),
            refused_styles: Vec::new(),
            refused_base: RefCell::default(),
            referrer_policy: None,
            referrer: String::new(),
        }
    }

    /// The page, its document under `csp`, its response's policies, the
    /// violations of its checks recorded for it.
    pub(crate) fn with_csp(mut self, csp: Csp) -> Page {
        self.csp = csp.with_sink();
        self
    }

    /// The document's policies as they stood when last looked at, its
    /// `<meta>` elements' since then aside.
    pub(crate) fn policies(&self) -> &Csp {
        &self.csp
    }

    /// What the document's policies refused, or would have, since the
    /// last time (M33.1), oldest first.
    pub(crate) fn take_violations(&self) -> Vec<net::csp::Violation> {
        self.csp.take_violations()
    }

    /// The page, its document sandboxed by `sandbox` (M33.2): of a new
    /// opaque origin unless `allow-same-origin` lifted that flag.
    pub(crate) fn with_sandbox(mut self, sandbox: Sandbox) -> Page {
        if sandbox.has(Sandbox::ORIGIN) {
            self.context.origin = Origin::new_opaque();
        }
        self.sandbox = sandbox;
        self
    }

    /// The document's sandboxing flags.
    pub(crate) fn sandbox(&self) -> Sandbox {
        self.sandbox
    }

    /// The page, its response's `Referrer-Policy` given (M33.3).
    pub(crate) fn with_referrer_policy(mut self, policy: Option<ReferrerPolicy>) -> Page {
        if policy.is_some() {
            self.referrer_policy = policy;
        }
        self
    }

    /// The page, `referrer` the referrer of the request that brought it.
    pub(crate) fn with_referrer(mut self, referrer: String) -> Page {
        self.referrer = referrer;
        self
    }

    /// `document.referrer` (M33.3).
    pub(crate) fn referrer(&self) -> &str {
        &self.referrer
    }

    /// The document's referrer policy as it stands (M33.3): that of the
    /// last `<meta name=referrer>` in tree order that names one (HTML
    /// section 4.2.5.1, applied as the parser meets it, legacy names
    /// mapped), else its response's or its creator's, else the default.
    pub(crate) fn referrer_policy(&self, document: &Document) -> ReferrerPolicy {
        document
            .subtree(document.root())
            .filter(|&node| {
                document.tag(node) == Some("meta")
                    && document
                        .attr(node, "name")
                        .is_some_and(|name| name.eq_ignore_ascii_case("referrer"))
            })
            .filter_map(|node| {
                document
                    .attr(node, "content")
                    .and_then(ReferrerPolicy::from_meta)
            })
            .last()
            .or(self.referrer_policy)
            .unwrap_or_default()
    }

    /// Element `node`'s nonce (M33.1): the one kept when its attribute
    /// was hidden or a script set it, else its attribute's.
    pub(crate) fn nonce(&self, document: &Document, node: NodeId) -> String {
        self.nonces
            .get(&node)
            .cloned()
            .or_else(|| document.attr(node, "nonce").map(str::to_owned))
            .unwrap_or_default()
    }

    /// Whether element `node`'s `style` attribute was last written by the
    /// CSSOM (M33.1), which the policies let through, as the reference
    /// browser does; markup's and `setAttribute`'s are theirs to allow.
    pub(crate) fn set_cssom_style(&mut self, node: NodeId, cssom: bool) {
        match cssom {
            true => self.cssom_styles.insert(node),
            false => self.cssom_styles.remove(&node),
        };
    }

    /// The `<style>` elements the policies refused since the last time
    /// (M33.1): each hears `error`.
    pub(crate) fn take_refused_styles(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.refused_styles)
    }

    /// Whether the policies allow `url` as the document's base URL
    /// (`base-uri`, M33.1): a refusal recorded once for each URL in turn.
    fn base_allowed(&self, url: &Url) -> bool {
        let violations = self.csp.base_uri_violations(url);
        if violations.is_empty() {
            return true;
        }
        let mut refused = self.refused_base.borrow_mut();
        if refused.as_ref() != Some(url) {
            self.csp.record(&violations);
            *refused = Some(url.clone());
        }
        net::csp::enforced(violations).is_ok()
    }

    /// Element `node`'s nonce is `nonce` from now on, its attribute
    /// unchanged (HTML's `nonce` IDL attribute's setter).
    pub(crate) fn set_nonce(&mut self, node: NodeId, nonce: String) {
        self.nonces.insert(node, nonce);
    }

    /// The nonce element `node` is checked by (CSP section 6.7.3.1, "is
    /// element nonceable"): none when the name or the value of one of its
    /// attributes holds `<script` or `<style`, as markup a dangling
    /// injection swallowed would.
    pub(crate) fn checked_nonce(&self, document: &Document, node: NodeId) -> String {
        checked_nonce(document, &self.nonces, node)
    }

    /// Hides the nonces of `document`'s elements (HTML's nonce attribute
    /// steps, M33.1) when one of its policies came from a header: each
    /// element's `nonce` attribute that is not empty is kept as its nonce
    /// and emptied, so that no selector reads it out.
    pub(crate) fn hide_nonces(&mut self, document: &mut Document) {
        if !self.csp.has_header_policy() {
            return;
        }
        let shown: Vec<(NodeId, String)> = document
            .subtree(document.root())
            .filter_map(|node| {
                let nonce = document
                    .attr(node, "nonce")
                    .filter(|nonce| !nonce.is_empty())?;
                Some((node, nonce.to_owned()))
            })
            .collect();
        for (node, nonce) in shown {
            self.nonces.insert(node, nonce);
            let _ = document.set_attribute(node, "nonce", "");
        }
    }

    /// What the document gives the documents of the frames it makes
    /// (M33.2).
    pub(crate) fn creator(&mut self, document: &Document) -> Creator {
        Creator {
            origin: self.context.origin.clone(),
            url: self
                .context
                .creator_url
                .clone()
                .or_else(|| self.context.url.clone()),
            base_url: self.base_url(document),
            address_space: self.context.address_space,
            secure: self.is_secure_context(),
            csp: self.csp(document).clone(),
            referrer_policy: self.referrer_policy(document),
        }
    }

    /// The page of a frame's document that `creator` made (M33.2): an
    /// `about:` document takes its origin, base URL, the URL its
    /// requests come from and its being a secure context; every one its
    /// policies.
    pub(crate) fn inheriting(mut self, creator: &Creator) -> Page {
        if self
            .context
            .url
            .as_ref()
            .is_some_and(|url| url.scheme() == "about")
        {
            self.context.origin = creator.origin.clone();
            self.context.address_space = creator.address_space;
            self.context.fallback_base = creator.base_url.clone();
            self.context.creator_url = creator.url.clone();
            self.context.secure = Some(creator.secure);
            // Its referrer its creator's URL (M33.3).
            self.referrer = creator.url.as_ref().map(Url::to_string).unwrap_or_default();
        }
        self.csp = creator.csp.inherited();
        self.referrer_policy = Some(creator.referrer_policy);
        self.metas.clear();
        self
    }

    /// The document's Content Security Policy as it stands (M24.6): its
    /// response's, and that of each `<meta http-equiv=
    /// "Content-Security-Policy">` with a `content` that is a child of
    /// the head (HTML section 4.2.5.3), taken in once, the first time it
    /// is seen, as the parser inserts it; it holds for what comes after.
    pub(crate) fn csp(&mut self, document: &Document) -> &Csp {
        let head = document.document_element().and_then(|html| {
            document
                .element_children(html)
                .find(|&child| document.tag(child) == Some("head"))
        });
        for meta in head
            .into_iter()
            .flat_map(|head| document.element_children(head))
        {
            if document.tag(meta) == Some("meta")
                && !self.metas.contains(&meta)
                && document
                    .attr(meta, "http-equiv")
                    .is_some_and(|value| value.eq_ignore_ascii_case("content-security-policy"))
                && let Some(content) = document.attr(meta, "content")
            {
                self.metas.push(meta);
                self.csp.add_meta(content);
            }
        }
        &self.csp
    }

    /// The page, its document in `language` by its response.
    pub(crate) fn with_language(mut self, language: Option<String>) -> Page {
        self.language = language;
        self
    }

    /// The language the response gave the document, if it named one.
    pub(crate) fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    /// The page, its document of type `content_type` (M32).
    pub(crate) fn with_content_type(mut self, content_type: Option<String>) -> Page {
        self.content_type = content_type;
        self
    }

    /// The document's type when it is no HTML document.
    pub(crate) fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    pub(crate) fn id(&self) -> PageId {
        self.context.id
    }

    /// The document's URL.
    pub(crate) fn url(&self) -> Option<&Url> {
        self.context.url.as_ref()
    }

    /// The document's origin (M27.2).
    pub(crate) fn origin(&self) -> &Origin {
        &self.context.origin
    }

    /// Whether the document is a secure context (HTML 8.1.3.5): its URL
    /// potentially trustworthy, as a document with none, an
    /// `about:blank`, is.
    pub(crate) fn is_secure_context(&self) -> bool {
        // A frame's `about:` document is as its creator (M33.2).
        self.context.secure.unwrap_or_else(|| {
            self.context
                .url
                .as_ref()
                .is_none_or(Url::is_potentially_trustworthy)
        })
    }

    /// The encoding of the page's document.
    pub(crate) fn encoding(&self) -> Encoding {
        self.context.encoding
    }

    /// The encoding the document is in, when the engine cannot decode
    /// it yet.
    pub(crate) fn undecodable(&self) -> Option<Encoding> {
        self.undecodable
    }

    /// Gives the document a new URL, as a navigation to one of its
    /// fragments does (M15). A document given to the engine keeps none.
    pub(crate) fn set_url(&mut self, url: Url) {
        if self.context.url.is_some() {
            self.context.url = Some(url);
        }
    }

    /// The document's base URL as it now stands (HTML's "document base
    /// URL"): the frozen base URL of the first `<base href>` in tree
    /// order, or else the document's URL. A document with no URL has
    /// none.
    pub(crate) fn base_url(&self, document: &Document) -> Option<Url> {
        let first_base = document.subtree(document.root()).find(|&node| {
            document.tag(node) == Some("base") && document.attr(node, "href").is_some()
        });
        match first_base.and_then(|node| document.attr(node, "href")) {
            // One the policies' `base-uri` refuses is not used (M33.1).
            Some(href) => frozen_base_url(href, &self.context)
                .filter(|url| self.base_allowed(url))
                .or_else(|| self.context.fallback()),
            None => self.context.fallback(),
        }
    }

    /// `href` parsed against the document's base URL with the
    /// document's encoding: HTML's "encoding-parse a URL". `None` when
    /// it does not parse.
    pub(crate) fn resolve(&self, document: &Document, href: &str) -> Option<Url> {
        let base = self.base_url(document);
        Url::parse_with(href, base.as_ref(), self.context.encoding).ok()
    }

    /// `href` parsed against the document's own URL, its fallback base
    /// URL, as a `<base>`'s `href` is read (M27).
    pub(crate) fn resolve_from_document(&self, href: &str) -> Option<Url> {
        Url::parse_with(
            href,
            self.context.fallback().as_ref(),
            self.context.encoding,
        )
        .ok()
    }

    /// What a request of the page's is initiated by; none for a page
    /// with no URL, which fetches nothing. A frame's `about:` document's
    /// requests are made from its creator's URL (M33.2).
    pub(crate) fn initiator(&self) -> Option<Initiator> {
        Some(Initiator {
            url: self
                .context
                .creator_url
                .clone()
                .or_else(|| self.context.url.clone())?,
            address_space: self.context.address_space,
            origin: self.context.origin.clone(),
        })
    }

    /// The request for the document's icon (HTML's `icon` link type,
    /// M15): the last `<link>` in tree order whose `rel` has the token
    /// `icon`, whose `type`, if it has one, is an image type, and whose
    /// `href` is not empty and parses; else, for an `http:` or `https:`
    /// document, `/favicon.ico` against its URL, as HTML allows. None for
    /// a document with no URL.
    pub(crate) fn icon_request(&self, document: &Document) -> Option<Request> {
        let url = self.context.url.clone()?;
        let linked = document
            .subtree(document.root())
            .filter(|&node| {
                document.tag(node) == Some("link")
                    && document.attr(node, "rel").is_some_and(|rel| {
                        rel.split(is_ascii_whitespace)
                            .any(|token| token.eq_ignore_ascii_case("icon"))
                    })
                    && document.attr(node, "type").is_none_or(|kind| {
                        kind.is_empty() || MimeType::parse(kind).is_some_and(|t| t.is_image())
                    })
            })
            .filter_map(|node| document.attr(node, "href"))
            .filter(|href| !href.is_empty())
            .filter_map(|href| self.resolve(document, href))
            .last();
        let icon = match linked {
            Some(icon) => icon,
            None if matches!(url.scheme(), "http" | "https") => url.join("/favicon.ico").ok()?,
            None => return None,
        };
        Some(
            Request::get(icon, Destination::Image).initiated_by(Initiator {
                url,
                address_space: self.context.address_space,
                origin: self.context.origin.clone(),
            }),
        )
    }

    /// Aborts the fetches of the page's sheets, as HTML's "stop loading"
    /// does, and leaves those sheets out as failed. Says whether any
    /// fetch was aborted.
    pub(crate) fn abort(&mut self, network: &mut Network) -> bool {
        let aborted = network.cancel(self.context.id, None);
        for owner in &mut self.owners {
            owner.sheet.fail_waiting();
        }
        aborted
    }

    /// Finds the sheets of `document`, freshly parsed, and starts their
    /// fetches: each element's URL resolved against the base URL as it
    /// was where the parser inserted it, and those before `<body>` whose
    /// media holds for `media` blocking (HTML: a sheet for another
    /// medium is applied when it comes, the page not held for it).
    pub(crate) fn begin(&mut self, document: &Document, network: &mut Network, media: &Media) {
        // Its requests are checked against its policies (M33.1).
        let csp = self.csp(document).clone();
        network.set_csp(self.context.id, Some(csp.clone()));
        network.set_referrer_policy(self.context.id, Some(self.referrer_policy(document)));
        let (found, _) = scan(document, &self.context, &self.nonces);
        let context = &self.context;
        self.owners = found
            .into_iter()
            .map(|found| {
                let base = found.base_here.clone();
                let blocking = found.before_body
                    && rules::media_attribute_holds(found.media.as_deref(), media);
                let mut owner = start(context, found, base.as_ref(), network, &csp);
                owner.blocking = blocking;
                owner
            })
            .collect();
        let refused = self
            .owners
            .iter()
            .filter(|owner| owner.refused())
            .map(|owner| owner.node);
        self.refused_styles.extend(refused);
    }

    /// Finds the sheets of `document` again after it changed, keeping
    /// each element's sheet whose key is unchanged and making the others
    /// afresh, against the base URL as it now stands. A sheet kept is
    /// not fetched again for a new base URL, as HTML's base URL change
    /// steps leave `<link>` elements alone. The fetches of the sheets
    /// dropped are aborted.
    ///
    /// For a document the parser is still building, `media` is given: a
    /// sheet found before `<body>` whose media holds blocks the document
    /// being shown, as [`Page::begin`]'s do (M24.8).
    pub(crate) fn rescan(
        &mut self,
        document: &Document,
        network: &mut Network,
        media: Option<&Media>,
    ) {
        let csp = self.csp(document).clone();
        network.set_csp(self.context.id, Some(csp.clone()));
        network.set_referrer_policy(self.context.id, Some(self.referrer_policy(document)));
        let (found, base) = scan(document, &self.context, &self.nonces);
        let mut old: HashMap<NodeId, Owner> = self
            .owners
            .drain(..)
            .map(|owner| (owner.node, owner))
            .collect();
        let mut owners = Vec::with_capacity(found.len());
        for found in found {
            match old.remove(&found.node) {
                Some(mut owner) if owner.key == found.key => {
                    owner.title = found.title;
                    owner.media = found.media;
                    owners.push(owner);
                }
                kept => {
                    if kept.is_some() {
                        network.cancel(self.context.id, Some(found.node));
                    }
                    let holds = media.is_some_and(|media| {
                        rules::media_attribute_holds(found.media.as_deref(), media)
                    });
                    let blocking = holds && found.before_body;
                    // An element the parser has just made resolves against
                    // the base URL where it put it, as `begin` has it.
                    let base = match media {
                        Some(_) => found.base_here.clone(),
                        None => base.clone(),
                    };
                    let mut owner = start(&self.context, found, base.as_ref(), network, &csp);
                    owner.blocking = blocking;
                    owner.blocks_scripts = holds;
                    if owner.refused() {
                        self.refused_styles.push(owner.node);
                    }
                    owners.push(owner);
                }
            }
        }
        for node in old.keys() {
            network.cancel(self.context.id, Some(*node));
        }
        self.owners = owners;
    }

    /// Whether the sheet of `owner` at `path` among its imports waits for
    /// fetch `id` (M26): the result is the one wanted.
    pub(crate) fn awaits(&self, owner: NodeId, path: &[usize], id: FetchId) -> bool {
        let Some(entry) = self.owners.iter().find(|o| o.node == owner) else {
            return false;
        };
        let mut slot = &entry.sheet;
        for &index in path {
            let Slot::Ready(sheet) = slot else {
                return false;
            };
            let Some(next) = sheet.imports.get(index) else {
                return false;
            };
            slot = &next.sheet;
        }
        matches!(slot, Slot::Waiting(waiting) if *waiting == id)
    }

    /// Takes the result of fetch `id` for the sheet of `owner` at
    /// `path` among its imports. A sheet that arrives with an ok status
    /// and the type of a stylesheet is decoded and parsed and its
    /// imports fetched; any other is left out. Says whether a sheet was
    /// added. A result for a sheet no longer wanted changes nothing.
    pub(crate) fn sheet_arrived(
        &mut self,
        owner: NodeId,
        path: &[usize],
        id: FetchId,
        result: Result<Response, LoadError>,
        network: &mut Network,
    ) -> bool {
        let Page {
            context, owners, ..
        } = self;
        let Some(owner_entry) = owners.iter_mut().find(|o| o.node == owner) else {
            return false;
        };
        // The way down to the slot: the locations above it, which an
        // import may not fetch again, and its environment encoding.
        let mut chain = Vec::new();
        let mut environment = owner_entry.environment;
        let mut slot = &mut owner_entry.sheet;
        for &index in path {
            let Slot::Ready(sheet) = slot else {
                return false;
            };
            if let Some(location) = &sheet.location {
                chain.push(location.without_fragment().to_owned());
            }
            environment = sheet.encoding;
            let Some(next) = sheet.imports.get_mut(index) else {
                return false;
            };
            slot = &mut next.sheet;
        }
        if !matches!(slot, Slot::Waiting(waiting) if *waiting == id) {
            return false;
        }
        let Some(response) = result.ok().filter(is_stylesheet) else {
            *slot = Slot::Failed;
            return false;
        };
        let transport = response
            .mime_type
            .as_ref()
            .and_then(|mime_type| mime_type.parameter("charset"))
            .and_then(Encoding::for_label);
        let (text, encoding) = css::decode_stylesheet(&response.body, transport, Some(environment));
        let location = response.url_list.first().cloned();
        if let Some(location) = &location {
            chain.push(location.without_fragment().to_owned());
        }
        let mut rules = css::parse_stylesheet(&text).unwrap_or_default();
        if let Some(location) = &location {
            rules::absolute_urls(&mut rules.rules, location);
        }
        let mut sheet = Sheet {
            rules,
            location,
            encoding,
            imports: Vec::new(),
        };
        fetch_imports(context, owner, path, &chain, &mut sheet, None, network);
        *slot = Slot::Ready(Box::new(sheet));
        true
    }

    /// Whether every blocking sheet, with its imports, has arrived or
    /// failed.
    pub(crate) fn blocking_settled(&self) -> bool {
        self.owners
            .iter()
            .filter(|owner| owner.blocking)
            .all(|owner| owner.sheet.settled())
    }

    /// Whether a sheet the parser made, whose media held, has yet to
    /// arrive or fail, with its imports: HTML's "has a style sheet that is
    /// blocking scripts" (M24.3).
    pub(crate) fn blocks_scripts(&self) -> bool {
        self.owners
            .iter()
            .any(|owner| owner.blocks_scripts && !owner.sheet.settled())
    }

    /// Whether every sheet, with its imports, has arrived or failed.
    pub(crate) fn settled(&self) -> bool {
        self.owners.iter().all(|owner| owner.sheet.settled())
    }

    /// The sheets that apply, in cascade order: each element's in tree
    /// order, the rules of the sheets it imports in place of its
    /// `@import` rules, and each in the blocks of its conditions: an
    /// import's layer, `supports()` and media, and the element's
    /// `media`. The first titled sheet's title names the preferred set,
    /// and a sheet with another title is left out (CSSOM §6.1's add
    /// steps).
    pub(crate) fn sheets(&self) -> Vec<css::Stylesheet> {
        let preferred = self
            .owners
            .iter()
            .find(|owner| !owner.title.is_empty() && matches!(owner.sheet, Slot::Ready(_)))
            .map(|owner| owner.title.as_str());
        let mut sheets = Vec::new();
        for owner in &self.owners {
            if let Slot::Ready(sheet) = &owner.sheet
                && (owner.title.is_empty() || Some(owner.title.as_str()) == preferred)
            {
                sheets.push(css::Stylesheet {
                    rules: rules::wrap_in_media(owner.media.as_deref(), sheet.rules()),
                });
            }
        }
        sheets
    }
}

impl Slot {
    /// Makes this sheet, and each import below it, failed where it is
    /// still on its way.
    fn fail_waiting(&mut self) {
        match self {
            Slot::Waiting(_) => *self = Slot::Failed,
            Slot::Ready(sheet) => {
                for imported in &mut sheet.imports {
                    imported.sheet.fail_waiting();
                }
            }
            Slot::None | Slot::Failed => {}
        }
    }

    fn settled(&self) -> bool {
        match self {
            Slot::Waiting(_) => false,
            Slot::Ready(sheet) => sheet.imports.iter().all(|i| i.sheet.settled()),
            Slot::None | Slot::Failed => true,
        }
    }
}

impl Sheet {
    /// The sheet's rules, each `@import` rule's place taken by the
    /// rules of the sheet it imports in the blocks of its conditions,
    /// empty when that sheet has not come.
    fn rules(&self) -> Vec<css::Rule> {
        let mut imports = self.imports.iter().peekable();
        let mut rules = Vec::with_capacity(self.rules.rules.len());
        for (index, rule) in self.rules.rules.iter().enumerate() {
            match imports.next_if(|imported| imported.import.rule == index) {
                Some(imported) => {
                    let inner = match &imported.sheet {
                        Slot::Ready(sheet) => sheet.rules(),
                        _ => Vec::new(),
                    };
                    rules.extend(imported.import.wrap(inner));
                }
                None => rules.push(rule.clone()),
            }
        }
        rules
    }
}

impl Context {
    /// A request for a stylesheet at `url`, made by the document; none
    /// for a document that fetches nothing.
    fn request(&self, url: Url) -> Option<Request> {
        let document = self.url.clone()?;
        Some(
            Request::get(url, Destination::Style).initiated_by(Initiator {
                url: document,
                address_space: self.address_space,
                origin: self.origin.clone(),
            }),
        )
    }
}

/// Whether a response is a stylesheet to apply: an ok status, and a
/// `Content-Type` of `text/css`, or none, which is taken as `text/css`
/// unless `X-Content-Type-Options: nosniff` says otherwise (Fetch's
/// should response to request be blocked due to nosniff).
pub(crate) fn is_stylesheet(response: &Response) -> bool {
    response.ok()
        && match &response.mime_type {
            Some(mime_type) => mime_type.essence() == "text/css",
            None => !net::mime::determine_nosniff(&response.headers),
        }
}

/// The owner `found` describes, its sheet begun: a `<link>`'s fetched,
/// its URL resolved against `base`, and a `<style>`'s parsed, with its
/// imports fetched.
fn start(
    context: &Context,
    found: Found,
    base: Option<&Url>,
    network: &mut Network,
    csp: &Csp,
) -> Owner {
    let environment = found
        .charset
        .as_deref()
        .and_then(Encoding::for_label)
        .unwrap_or(context.encoding);
    let sheet = match &found.key {
        key @ Key::Link {
            href, crossorigin, ..
        } if link_wants_sheet(key) => href
            .as_deref()
            .zip(base)
            .and_then(|(href, base)| Url::parse_with(href, Some(base), context.encoding).ok())
            .and_then(|url| context.request(url))
            .map(|request| super::with_cors_settings(request, crossorigin.as_deref()))
            .map(|mut request| {
                // Its element's nonce (M33.1), and referrer (M33.3).
                request.nonce.clone_from(&found.nonce);
                request.referrer_policy = found.referrer_policy;
                if found.noreferrer {
                    request.referrer = Referrer::None;
                }
                request
            })
            .map_or(Slot::None, |request| {
                let target = Target::Sheet {
                    owner: found.node,
                    path: Vec::new(),
                };
                Slot::Waiting(network.fetch(context.id, target, request))
            }),
        // By nonce, hash or 'unsafe-inline' (M33.1): refused, no sheet.
        key @ Key::Style { text, .. }
            if style_makes_sheet(key) && {
                let violations = csp.inline_violations(Inline::Style, &found.nonce, text);
                csp.record(&violations);
                net::csp::enforced(violations).is_err()
            } =>
        {
            Slot::Failed
        }
        key @ Key::Style { text, .. } if style_makes_sheet(key) => {
            let mut sheet = Sheet {
                rules: css::parse_stylesheet(text).unwrap_or_default(),
                location: None,
                encoding: context.encoding,
                imports: Vec::new(),
            };
            fetch_imports(context, found.node, &[], &[], &mut sheet, base, network);
            Slot::Ready(Box::new(sheet))
        }
        _ => Slot::None,
    };
    Owner {
        node: found.node,
        key: found.key,
        title: found.title,
        media: found.media,
        environment,
        blocking: false,
        blocks_scripts: false,
        sheet,
    }
}

/// Fetches the imports of `sheet`, at `path` in `owner`'s sheets, each
/// resolved against the sheet's location, or against `base` for a
/// `<style>` element's sheet. An import of a sheet in `chain`, the
/// locations from the owner's sheet down to this one, fetches nothing,
/// nor do imports deeper than [`MAX_IMPORT_DEPTH`]; their rules stand
/// in the sheet, their layers named all the same.
fn fetch_imports(
    context: &Context,
    owner: NodeId,
    path: &[usize],
    chain: &[String],
    sheet: &mut Sheet,
    base: Option<&Url>,
    network: &mut Network,
) {
    let against = sheet.location.clone().or_else(|| base.cloned());
    for import in rules::imports(&sheet.rules) {
        let index = sheet.imports.len();
        // URLs in CSS are parsed as UTF-8 (css-values-4 §4.5.4).
        let request = against
            .as_ref()
            .filter(|_| path.len() < MAX_IMPORT_DEPTH)
            .and_then(|against| against.join(&import.url).ok())
            .filter(|url| !chain.iter().any(|above| above == url.without_fragment()))
            .and_then(|url| context.request(url));
        let slot = match request {
            Some(request) => {
                let mut import_path = path.to_vec();
                import_path.push(index);
                let target = Target::Sheet {
                    owner,
                    path: import_path,
                };
                Slot::Waiting(network.fetch(context.id, target, request))
            }
            None => Slot::None,
        };
        sheet.imports.push(Imported {
            import,
            sheet: slot,
        });
    }
}

/// Whether a `<link>` is fetched as a stylesheet: its `rel` has the
/// token `stylesheet` and not `alternate`, it is not `disabled`, its
/// `type`, if given, is `text/css`, and its `href` is not empty. Its
/// `media` does not decide it: a sheet for print, or for a width the
/// window has not, is fetched for when it is wanted.
fn link_wants_sheet(key: &Key) -> bool {
    let Key::Link {
        href,
        rel,
        kind,
        disabled,
        ..
    } = key
    else {
        return false;
    };
    let has = |token: &str| {
        rel.as_deref().is_some_and(|rel| {
            rel.split(is_ascii_whitespace)
                .any(|t| t.eq_ignore_ascii_case(token))
        })
    };
    let type_ok = match kind.as_deref() {
        None | Some("") => true,
        Some(kind) => MimeType::parse(kind).is_some_and(|t| t.essence() == "text/css"),
    };
    has("stylesheet")
        && !has("alternate")
        && !disabled
        && type_ok
        && href.as_deref().is_some_and(|href| !href.is_empty())
}

/// Whether a `<style>` makes a sheet: its `type`, if given and not
/// empty, is `text/css` in any case (HTML's update a style block).
fn style_makes_sheet(key: &Key) -> bool {
    let Key::Style { kind, .. } = key else {
        return false;
    };
    kind.as_deref()
        .is_none_or(|kind| kind.is_empty() || kind.eq_ignore_ascii_case("text/css"))
}

fn is_ascii_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{c}' | '\r' | ' ')
}

/// The elements of `document` that may own a sheet, in tree order, and
/// its base URL.
fn scan(
    document: &Document,
    context: &Context,
    nonces: &HashMap<NodeId, String>,
) -> (Vec<Found>, Option<Url>) {
    let html = document.document_element();
    let mut found = Vec::new();
    let mut base = context.fallback();
    let mut base_seen = false;
    let mut before_body = true;
    for node in document.subtree(document.root()) {
        let tag = match document.tag(node) {
            Some(tag) => tag,
            // SVG's `<style>` owns a sheet as HTML's does (SVG 2 section
            // 6.3, M32).
            None if is_svg_style(document, node) => "style",
            None => continue,
        };
        let attr = |name: &str| document.attr(node, name).map(str::to_owned);
        let nonce = |node: NodeId| checked_nonce(document, nonces, node);
        let title = || attr("title").unwrap_or_default();
        match tag {
            // HTML's body element: the html element's child body.
            "body" if html.is_some() && document.parent(node) == html => before_body = false,
            "base" if !base_seen => {
                if let Some(href) = document.attr(node, "href") {
                    base_seen = true;
                    base = frozen_base_url(href, context);
                }
            }
            "link" => found.push(Found {
                node,
                key: Key::Link {
                    href: attr("href"),
                    rel: attr("rel"),
                    kind: attr("type"),
                    disabled: document.attr(node, "disabled").is_some(),
                    crossorigin: attr("crossorigin"),
                },
                title: title(),
                media: attr("media"),
                charset: attr("charset"),
                base_here: base.clone(),
                before_body,
                nonce: nonce(node),
                referrer_policy: document
                    .attr(node, "referrerpolicy")
                    .and_then(ReferrerPolicy::parse),
                noreferrer: has_token(document.attr(node, "rel"), "noreferrer"),
            }),
            "style" => found.push(Found {
                node,
                key: Key::Style {
                    text: document
                        .children(node)
                        .filter_map(|child| document.text(child))
                        .collect(),
                    kind: attr("type"),
                },
                title: title(),
                media: attr("media"),
                charset: None,
                base_here: base.clone(),
                before_body,
                nonce: nonce(node),
                referrer_policy: None,
                noreferrer: false,
            }),
            _ => {}
        }
    }
    (found, base)
}

/// Whether the token list `value` has `token`, in any case.
pub(crate) fn has_token(value: Option<&str>, token: &str) -> bool {
    value.is_some_and(|value| {
        value
            .split(is_ascii_whitespace)
            .any(|each| each.eq_ignore_ascii_case(token))
    })
}

/// The nonce element `node` of `document` is checked by (CSP section
/// 6.7.3.1, "is element nonceable", M33.1): its kept nonce of `nonces`,
/// else its attribute's; none when the name or the value of one of its
/// attributes holds `<script` or `<style`, as markup a dangling
/// injection swallowed would.
fn checked_nonce(document: &Document, nonces: &HashMap<NodeId, String>, node: NodeId) -> String {
    let injected = |text: &str| {
        let lower = text.to_ascii_lowercase();
        lower.contains("<script") || lower.contains("<style")
    };
    let swallowed = document
        .attrs(node)
        .unwrap_or_default()
        .iter()
        .any(|(name, value)| injected(name) || injected(value));
    if swallowed {
        return String::new();
    }
    // White space around it is not the nonce's, as the reference
    // browser takes it (p4).
    nonces
        .get(&node)
        .map(String::as_str)
        .or_else(|| document.attr(node, "nonce"))
        .unwrap_or_default()
        .trim_matches(is_ascii_whitespace)
        .to_owned()
}

/// A `<base>`'s frozen base URL (HTML §4.2.3): its `href` parsed against
/// the document's URL, or that URL when it fails to parse or is
/// `data:` or `javascript:`.
fn frozen_base_url(href: &str, context: &Context) -> Option<Url> {
    let fallback = context.fallback()?;
    match Url::parse_with(href, Some(&fallback), context.encoding) {
        Ok(url) if !matches!(url.scheme(), "data" | "javascript") => Some(url),
        _ => Some(fallback),
    }
}

/// Whether `node` is SVG's `<style>`.
fn is_svg_style(document: &Document, node: NodeId) -> bool {
    document.svg_tag(node) == Some("style")
}

/// Whether `change` can change a document's sheets: something inserted
/// or removed holds a `<link>` or `<style>`, or a `<style>`'s text
/// changed, or one of those elements' attributes did. A `<base>` alone
/// changes no sheet: those already made keep their URLs, and the next
/// sheet made reads the base URL as it then stands.
pub(crate) fn may_touch_sheets(document: &Document, change: &Change) -> bool {
    let is_style =
        |node: NodeId| document.tag(node) == Some("style") || is_svg_style(document, node);
    let owns = |node: NodeId| document.tag(node) == Some("link") || is_style(node);
    match change {
        Change::Inserted { parent, child } | Change::Removed { parent, child, .. } => {
            is_style(*parent)
                // A node freed since cannot be looked into.
                || !document.contains(*child)
                || document.subtree(*child).any(owns)
        }
        Change::AttributeChanged { element, .. } => owns(*element),
        Change::TextChanged { node } => document.parent(*node).is_some_and(is_style),
        Change::StateChanged { .. } | Change::ControlChanged { .. } => false,
        Change::Everything => true,
    }
}
