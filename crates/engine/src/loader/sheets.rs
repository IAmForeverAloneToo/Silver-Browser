//! A document's stylesheets (docs/design/net.md, "The engine's
//! loader"): the `<link>` and `<style>` elements that own them, in tree
//! order, each with its sheet and the sheets that sheet imports, fetched
//! or failed or on their way. Every sheet is fetched whatever its media
//! or its imports' conditions, which wrap its rules for the cascade to
//! evaluate (docs/design/media.md); only one whose media holds when the
//! parser meets it holds the page back.

use std::cell::{OnceCell, RefCell};
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

/// A parsed `<style>`'s key: the document's base URL and the text.
type InlineKey = (Option<String>, String);

/// How deep imports go: a sheet's imports are one deeper than it, and
/// one deeper than this is not fetched.
pub(crate) const MAX_IMPORT_DEPTH: usize = 16;

/// The most `<style>` texts a page keeps parsed ([`Page`]), and the most
/// bytes of them all: past either, what it kept is let go of.
const MAX_PARSED_INLINE: usize = 128;
const MAX_PARSED_INLINE_BYTES: usize = 8 << 20;

/// One document's stylesheets.
pub(crate) struct Page {
    context: Context,
    /// The `<style>` sheets no element owns (a shadow tree's) as parsed,
    /// by the document's base URL and their text. The text is all the
    /// parser reads: it takes no quirks mode (the cascade applies that)
    /// and keeps a URL as written, resolved against the base when used,
    /// as an owned `<style>`'s is; the base is in the key so that a sheet
    /// made with it is never one of another base's. With the bytes of
    /// the texts and bases kept, [`MAX_PARSED_INLINE`] and [`MAX_PARSED_INLINE_BYTES`]
    /// at most: a flush for a script reads them at every first ask of a
    /// run, and parsed them each time (the site sweep's reddit, 100 ms or
    /// more a flush).
    parsed_inline: RefCell<(HashMap<InlineKey, css::Stylesheet>, usize)>,
    /// How many times a script's CSSOM changed the sheets (rules edited
    /// or replaced, sheets adopted, disabled or given media, properties
    /// registered): none of it is in the document's journal, so a flush
    /// built on the frame's styles checks the frame was made at this
    /// count.
    sheets_generation: u64,
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
    /// What lets its requests ask their way into a more private address
    /// space (M33.4), which its engine works out.
    local_network: Option<net::LocalNetworkAccess>,
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
    /// Whether it is the DevTools panel's page (M36), which the browser
    /// alone shows: its window has the bridge to the browser.
    panel: bool,
    /// Whether a script changed a sheet through the CSSOM (D66), or
    /// registered a custom property, since the engine last took the
    /// sheets.
    cssom_edited: bool,
    /// The custom properties the document's scripts registered
    /// (`CSS.registerProperty`, D73), in order.
    registered: Vec<style::PropertyRegistration>,
    /// The sheets scripts constructed (`new CSSStyleSheet()`, CSS module
    /// scripts), and the serial numbers of those the document adopted,
    /// in order.
    constructed: Vec<Constructed>,
    adopted: Vec<u64>,
    /// The serial numbers of the sheets each shadow root adopted (D69).
    adopted_shadow: HashMap<NodeId, Vec<u64>>,

    /// The serial numbers of the sheets a script disabled (CSSOM's
    /// disabled flag): a sheet made again is a new one, enabled.
    disabled: HashSet<u64>,
    /// The sheets of the elements taken out of the document (or whose
    /// sheet was made again), the latest last, kept for the CSSOM objects
    /// scripts still hold (CSSOM section 6.1: a sheet outlives its owner);
    /// at most [`MAX_DETACHED_SHEETS`].
    detached: Vec<Owner>,
}

/// The most sheets of elements taken out a page keeps for the CSSOM.
const MAX_DETACHED_SHEETS: usize = 64;

/// The most sheets a document's scripts may construct.
pub(crate) const MAX_CONSTRUCTED_SHEETS: usize = 4096;

/// The most custom properties a document's scripts may register.
pub(crate) const MAX_REGISTERED: usize = 1 << 14;

/// A constructed sheet (CSSOM's constructed flag): its serial number,
/// its rules, and the URL they resolve against.
struct Constructed {
    serial: u64,
    rules: css::Stylesheet,
    base: Option<Url>,
    /// Its `media`, where its rules apply; none for every medium.
    media: Option<String>,
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
    /// The shadow root of the tree the element is in, whose rules its
    /// sheet's are (D69); `None` for the document's.
    scope: Option<NodeId>,
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
    /// Whether the sheet was made in a script's run and its imports are
    /// yet to be fetched, at the rescan after it.
    imports_waiting: bool,
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
        /// Its integrity metadata (D62).
        integrity: Option<String>,
    },
    Style {
        text: String,
        kind: Option<String>,
        /// The text nodes the text is of: children put in or taken out
        /// make the sheet again, the same text or not, as HTML's
        /// "children changed steps" update a style block.
        nodes: Vec<NodeId>,
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
    /// Which sheet it is, for the CSSOM's objects (D66): a new number
    /// for each sheet made.
    serial: u64,
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
    /// The shadow root of its tree, if it is in one (D69).
    scope: Option<NodeId>,
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
            sheets_generation: 0,
            parsed_inline: RefCell::default(),
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
            local_network: None,
            nonces: HashMap::new(),
            cssom_styles: HashSet::new(),
            style_checks: RefCell::default(),
            refused_styles: Vec::new(),
            refused_base: RefCell::default(),
            referrer_policy: None,
            referrer: String::new(),
            panel: false,
            cssom_edited: false,
            registered: Vec::new(),
            constructed: Vec::new(),
            adopted: Vec::new(),
            adopted_shadow: HashMap::new(),
            disabled: HashSet::new(),
            detached: Vec::new(),
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
            sheets_generation: 0,
            parsed_inline: RefCell::default(),
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
            local_network: None,
            nonces: HashMap::new(),
            cssom_styles: HashSet::new(),
            style_checks: RefCell::default(),
            refused_styles: Vec::new(),
            refused_base: RefCell::default(),
            referrer_policy: None,
            referrer: String::new(),
            panel: false,
            cssom_edited: false,
            registered: Vec::new(),
            constructed: Vec::new(),
            adopted: Vec::new(),
            adopted_shadow: HashMap::new(),
            disabled: HashSet::new(),
            detached: Vec::new(),
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

    /// The page, the DevTools panel's (M36).
    pub(crate) fn into_panel(mut self) -> Page {
        self.panel = true;
        self
    }

    /// Whether it is the DevTools panel's page.
    pub(crate) fn is_panel(&self) -> bool {
        self.panel
    }

    /// The document's sandboxing flags.
    pub(crate) fn sandbox(&self) -> Sandbox {
        self.sandbox
    }

    /// What lets its requests ask their way into a more private address
    /// space (M33.4).
    pub(crate) fn local_network(&self) -> Option<&net::LocalNetworkAccess> {
        self.local_network.as_ref()
    }

    pub(crate) fn set_local_network(&mut self, access: Option<net::LocalNetworkAccess>) {
        self.local_network = access;
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
    /// Gives `network` the policies `document` has now, the `<meta>`
    /// ones parsed so far among them, for a fetch started before the
    /// page looks at the document again (M40).
    pub(crate) fn update_policies(&mut self, document: &Document, network: &mut Network) {
        let csp = self.csp(document).clone();
        network.set_csp(self.context.id, Some(csp));
        network.set_referrer_policy(self.context.id, Some(self.referrer_policy(document)));
    }

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

    /// A copy `to` of element `from` has its nonce (HTML's nonce
    /// attribute's cloning steps), hidden or not.
    pub(crate) fn copy_nonce(&mut self, from: NodeId, to: NodeId) {
        if let Some(nonce) = self.nonces.get(&from).cloned() {
            self.nonces.insert(to, nonce);
        }
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
        let root = document.root();
        self.hide_nonces_under(document, root);
    }

    /// [`Page::hide_nonces`] of the elements of `root`'s subtree, which a
    /// script connected or a write's parse inserted (HTML's nonce
    /// attribute's steps for an element that becomes connected).
    pub(crate) fn hide_nonces_under(&mut self, document: &mut Document, root: NodeId) {
        if !self.csp.has_header_policy() || !document.is_connected(root) {
            return;
        }
        let shown: Vec<(NodeId, String)> = document
            .subtree(root)
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
                self.keep_style_checks(document, meta);
                self.csp.add_meta(content);
            }
        }
        &self.csp
    }

    /// Decides now, by the policies so far, each `style` attribute before
    /// `meta` not yet decided: one the parser set before the `<meta>`
    /// brought a policy keeps what was said of it then, as the reference
    /// browser checks each when it is set.
    fn keep_style_checks(&self, document: &Document, meta: NodeId) {
        for node in document.subtree(document.root()) {
            if node == meta {
                break;
            }
            let Some(value) = document.attr(node, "style") else {
                continue;
            };
            if self.csp.is_empty() {
                self.style_checks
                    .borrow_mut()
                    .entry(node)
                    .or_insert_with(|| (value.to_owned(), true));
            } else {
                style::InlineStyles::allowed(self, node, value);
            }
        }
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

    /// `value`, a value a script gives as CSS text, with its URLs made
    /// absolute against the document's base URL, as a sheet's are against
    /// the sheet's: what a registered custom property's initial value
    /// resolves against (Properties and Values API section 3.1, D73).
    pub(crate) fn absolute_value(&self, document: &Document, value: &str) -> String {
        let tokens = css::tokenize(value);
        let names_url = tokens.iter().any(|t| match t {
            css::Token::Url { bad, .. } => !bad,
            css::Token::Function(name) => {
                name.eq_ignore_ascii_case("url") || name.eq_ignore_ascii_case("image-set")
            }
            _ => false,
        });
        let base = self.base_url(document);
        let Some(base) = base.filter(|_| names_url) else {
            return value.to_owned();
        };
        let mut rules = vec![css::Rule::Declarations(vec![css::Declaration {
            name: "--value".to_owned(),
            value: tokens,
            important: false,
        }])];
        rules::absolute_urls(&mut rules, &base);
        match &rules[0] {
            css::Rule::Declarations(list) => style::serialize::tokens(&list[0].value),
            _ => value.to_owned(),
        }
    }

    /// `href` parsed against the document's base URL by the UTF-8 URL
    /// parser, whatever the document's encoding (Fetch's `Request`).
    pub(crate) fn resolve_utf8(&self, document: &Document, href: &str) -> Option<Url> {
        let base = self.base_url(document);
        Url::parse_with(href, base.as_ref(), encoding::UTF_8).ok()
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
        let implicit = linked.is_none();
        let icon = match linked {
            Some(icon) => icon,
            None if matches!(url.scheme(), "http" | "https") => url.join("/favicon.ico").ok()?,
            None => return None,
        };
        let mut request = Request::get(icon, Destination::Image).initiated_by(Initiator {
            url,
            address_space: self.context.address_space,
            origin: self.context.origin.clone(),
        });
        // The reference browser fetches the `/favicon.ico` no link names
        // outside the page: the page's policies neither hold it nor
        // report it (M33).
        if implicit {
            request.csp = Some(Csp::none(self.context.origin.clone()));
        }
        Some(request)
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
                    // A sheet a run made, whose imports wait for the
                    // network (`rescan_inline`).
                    if std::mem::take(&mut owner.imports_waiting)
                        && let Slot::Ready(sheet) = &mut owner.sheet
                    {
                        let base = match media {
                            Some(_) => found.base_here.clone(),
                            None => base.clone(),
                        };
                        fetch_imports(
                            &self.context,
                            owner.node,
                            &[],
                            &[],
                            sheet,
                            base.as_ref(),
                            network,
                        );
                    }
                    owner.title = found.title;
                    owner.media = found.media;
                    owners.push(owner);
                }
                kept => {
                    if let Some(kept) = kept {
                        network.cancel(self.context.id, Some(found.node));
                        self.detach(kept);
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
        for (node, owner) in old {
            network.cancel(self.context.id, Some(node));
            self.detach(owner);
        }
        self.owners = owners;
        self.forget_shadow_sheets(document);
    }

    /// [`Page::rescan`] in a script's run, which has no network (D65,
    /// G50): the `<style>` elements put in or whose text changed have
    /// their sheets made now, as the CSSOM and the cascade read them at
    /// once, and the elements taken out lose theirs; a `<link>` waits
    /// for the rescan after the run, as do a `<style>`'s imports, which
    /// that rescan fetches into the sheet made here.
    pub(crate) fn rescan_inline(&mut self, document: &Document) {
        let csp = self.csp(document).clone();
        let (found, _) = scan(document, &self.context, &self.nonces);
        let mut old: HashMap<NodeId, Owner> = self
            .owners
            .drain(..)
            .map(|owner| (owner.node, owner))
            .collect();
        let mut owners = Vec::with_capacity(found.len());
        // Whether a sheet came, went or was made again: a change of the
        // sheets no document journal says ([`Page::sheets_generation`]).
        let mut changed = false;
        for found in found {
            let kept = old.remove(&found.node);
            let makes_now = matches!(&found.key, Key::Style { .. });
            match kept {
                Some(mut owner) if owner.key == found.key || !makes_now => {
                    if owner.key == found.key {
                        changed |= owner.title != found.title || owner.media != found.media;
                        owner.title = found.title;
                        owner.media = found.media;
                    }
                    owners.push(owner);
                }
                kept if makes_now => {
                    changed = true;
                    if let Some(kept) = kept {
                        self.detach(kept);
                    }
                    let owner = start_inline(&self.context, found, &csp);
                    if owner.refused() {
                        self.refused_styles.push(owner.node);
                    }
                    owners.push(owner);
                }
                _ => {}
            }
        }
        for (_, owner) in old {
            changed = true;
            self.detach(owner);
        }
        self.owners = owners;
        self.sheets_generation += u64::from(changed);
        self.forget_shadow_sheets(document);
    }

    /// Keeps the sheet of an owner that has it no more, for the CSSOM.
    fn detach(&mut self, owner: Owner) {
        if !matches!(owner.sheet, Slot::Ready(_)) {
            return;
        }
        self.detached.retain(|o| o.node != owner.node);
        if self.detached.len() >= MAX_DETACHED_SHEETS {
            self.detached.remove(0);
        }
        self.detached.push(owner);
    }

    /// The owner of `node`'s sheet: the element's in the document, or
    /// the last it had, with whether it is still the element's.
    fn owner_of(&self, node: NodeId) -> Option<(&Owner, bool)> {
        match self.owners.iter().find(|o| o.node == node) {
            Some(owner) => Some((owner, true)),
            None => self
                .detached
                .iter()
                .rev()
                .find(|o| o.node == node)
                .map(|owner| (owner, false)),
        }
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
            serial: next_serial(),
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

    /// The preferred style sheet set's name (CSSOM section 6.1): the
    /// title of the titled sheet added first, the element made first
    /// standing for it, as a run's rescan finds several at once.
    fn preferred(&self) -> Option<&str> {
        self.owners
            .iter()
            .filter(|owner| !owner.title.is_empty() && matches!(owner.sheet, Slot::Ready(_)))
            .min_by_key(|owner| owner.node)
            .map(|owner| owner.title.as_str())
    }

    /// What names each of [`Page::sheets`], in the same order, for the
    /// DevTools (M36): a fetched sheet's URL, or `<style>`.
    pub(crate) fn sheet_names(&self) -> Vec<String> {
        let preferred = self.preferred();
        self.owners
            .iter()
            .filter_map(|owner| match &owner.sheet {
                Slot::Ready(sheet)
                    if owner.title.is_empty() || Some(owner.title.as_str()) == preferred =>
                {
                    Some(
                        sheet
                            .location
                            .as_ref()
                            .map_or_else(|| "<style>".to_owned(), ToString::to_string),
                    )
                }
                _ => None,
            })
            .collect()
    }

    /// The sheets that apply, in cascade order: each element's in tree
    /// order, the rules of the sheets it imports in place of its
    /// `@import` rules, and each in the blocks of its conditions: an
    /// import's layer, `supports()` and media, and the element's
    /// `media`. The first titled sheet's title names the preferred set,
    /// and a sheet with another title is left out (CSSOM section 6.1's
    /// add steps).
    pub(crate) fn sheets(&self) -> Vec<css::Stylesheet> {
        self.sheets_of(None)
    }

    /// The sheets of each shadow tree (D69), by its root, in tree order:
    /// [`Page::sheets`] of its `<style>` and `<link>` elements, then its
    /// adopted sheets, with their implicit scoping roots in `document`.
    pub(crate) fn shadow_sheets(&self, document: &Document) -> Vec<style::ShadowTreeSheets> {
        let mut roots: Vec<NodeId> = Vec::new();
        for owner in &self.owners {
            if let Some(scope) = owner.scope
                && !roots.contains(&scope)
            {
                roots.push(scope);
            }
        }
        for &root in self.adopted_shadow.keys() {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        roots
            .into_iter()
            .map(|root| {
                let roots = self.scope_roots_of(document, Some(root));
                (root, self.sheets_of(Some(root)), roots)
            })
            .collect()
    }

    /// The sheets of the tree whose shadow root is `scope`, the
    /// document's for `None`, then its adopted ones.
    fn sheets_of(&self, scope: Option<NodeId>) -> Vec<css::Stylesheet> {
        let preferred = self.preferred();
        let mut sheets = Vec::new();
        for owner in self.owners.iter().filter(|owner| owner.scope == scope) {
            if let Slot::Ready(sheet) = &owner.sheet
                && (owner.title.is_empty() || Some(owner.title.as_str()) == preferred)
                && !self.disabled.contains(&sheet.serial)
            {
                sheets.push(css::Stylesheet {
                    rules: rules::wrap_in_media(owner.media.as_deref(), sheet.rules()),
                });
            }
        }
        // The adopted sheets after the document's (CSSOM's document or
        // shadow root CSS style sheets).
        self.adopted_sheets(scope, &mut sheets);
        sheets
    }

    /// The implicit scoping root of each of [`Page::sheets`], in the same
    /// order, as `document` now stands: the parent element of the element
    /// whose sheet it is, or none for an adopted sheet. What an `@scope`
    /// rule without roots of its own scopes to (Cascade 6 section 2.5.1).
    pub(crate) fn scope_roots(&self, document: &Document) -> Vec<Option<NodeId>> {
        self.scope_roots_of(document, None)
    }

    /// [`Page::scope_roots`] of the sheets of the tree whose shadow root
    /// is `scope` (D69), the document's for `None`, as
    /// [`Page::shadow_sheets`] gives them.
    fn scope_roots_of(&self, document: &Document, scope: Option<NodeId>) -> Vec<Option<NodeId>> {
        let preferred = self.preferred();
        let mut roots: Vec<Option<NodeId>> = self
            .owners
            .iter()
            .filter(|owner| {
                owner.scope == scope
                    && matches!(&owner.sheet, Slot::Ready(sheet)
                    if (owner.title.is_empty() || Some(owner.title.as_str()) == preferred)
                        && !self.disabled.contains(&sheet.serial))
            })
            .map(|owner| document.parent_element(owner.node))
            .collect();
        let mut adopted = Vec::new();
        self.adopted_sheets(scope, &mut adopted);
        roots.extend(adopted.iter().map(|_| None));
        roots
    }

    /// [`Page::sheets`] prepared for the cascade, each with its scoping
    /// root in `document`, with the custom properties its scripts
    /// registered, and the shadow trees' sheets (D69).
    pub(crate) fn stylesheets(&self, document: &Document) -> style::Stylesheets {
        style::Stylesheets::with_scope_roots(self.sheets(), self.scope_roots(document))
            .with_registered(self.registered.clone())
            .with_shadow_trees(self.shadow_sheets(document))
    }

    /// The adopted sheets of the tree whose shadow root is `scope`, the
    /// document's for `None`, that apply, each in its `media`, after
    /// `sheets`.
    fn adopted_sheets(&self, scope: Option<NodeId>, sheets: &mut Vec<css::Stylesheet>) {
        let adopted = match scope {
            Some(root) => self
                .adopted_shadow
                .get(&root)
                .map_or(&[][..], Vec::as_slice),
            None => &self.adopted,
        };
        for serial in adopted {
            if let Some(sheet) = self.constructed.iter().find(|c| c.serial == *serial)
                && !self.disabled.contains(serial)
            {
                sheets.push(css::Stylesheet {
                    rules: rules::wrap_in_media(sheet.media.as_deref(), sheet.rules.rules.clone()),
                });
            }
        }
    }
}

/// A new sheet's serial number.
fn next_serial() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// What the CSSOM reads of a sheet (D66): its rules, the URL it came
/// from, and the sheets its `@import` rules fetched, by their rules'
/// indices.
pub(crate) struct SheetView<'a> {
    pub(crate) rules: &'a css::Stylesheet,
    pub(crate) location: Option<&'a Url>,
    pub(crate) serial: u64,
}

impl Page {
    /// [`Page::sheets`] as they stand in `document` now, in tree order:
    /// with the sheets of the `<style>` elements the page has not scanned
    /// yet, those a script put in the run going on, and without those of
    /// the elements a script took out. What `document.fonts` reads of the
    /// rules before the run ends, as the reference browser's style update
    /// before `load()` has them; a style its media leaves out, or the
    /// page's policies refuse, gives none.
    pub(crate) fn sheets_now(&self, document: &Document, media: &Media) -> Vec<css::Stylesheet> {
        self.scoped_sheets_now(document, media).0
    }

    /// [`Page::sheets_now`], each with its implicit scoping root, as
    /// [`Page::scope_roots`] gives them.
    pub(crate) fn scoped_sheets_now(
        &self,
        document: &Document,
        media: &Media,
    ) -> (Vec<css::Stylesheet>, Vec<Option<NodeId>>) {
        self.scoped_sheets_now_of(document, media, document.root(), &OnceCell::new())
    }

    /// [`Page::sheets_now`] of each shadow tree in `document` (D69), by
    /// its root, in tree order, with its adopted sheets.
    pub(crate) fn shadow_sheets_now(
        &self,
        document: &Document,
        media: &Media,
    ) -> Vec<style::ShadowTreeSheets> {
        if !document.has_shadow_roots() {
            return Vec::new();
        }
        // The base URL found once for every tree, not walked for each.
        let base = OnceCell::new();
        document
            .shadow_including_subtree(document.root())
            .into_iter()
            .filter(|&node| document.is_shadow_root(node))
            .map(|root| {
                let (sheets, roots) = self.scoped_sheets_now_of(document, media, root, &base);
                (root, sheets, roots)
            })
            .collect()
    }

    /// [`Page::sheets_generation`]'s count.
    pub(crate) fn sheets_generation(&self) -> u64 {
        self.sheets_generation
    }

    /// `text` parsed as a sheet, from what was parsed of the same text
    /// at the same `base`, the document's base URL, before when it was;
    /// `None` for one that does not parse.
    fn parse_inline(&self, base: Option<&Url>, text: String) -> Option<css::Stylesheet> {
        let key = (base.map(|base| base.as_str().to_owned()), text);
        if let Some(sheet) = self.parsed_inline.borrow().0.get(&key) {
            return Some(sheet.clone());
        }
        let sheet = css::parse_stylesheet(&key.1).ok()?;
        let size = key.1.len() + key.0.as_ref().map_or(0, String::len);
        let mut parsed = self.parsed_inline.borrow_mut();
        let (kept, bytes) = &mut *parsed;
        if kept.len() >= MAX_PARSED_INLINE || *bytes + size > MAX_PARSED_INLINE_BYTES {
            kept.clear();
            *bytes = 0;
        }
        if size <= MAX_PARSED_INLINE_BYTES {
            *bytes += size;
            kept.insert(key, sheet.clone());
        }
        Some(sheet)
    }

    /// How many `<style>` texts are kept parsed, and their bytes: for the
    /// tests.
    #[cfg(test)]
    pub(crate) fn parsed_inline(&self) -> (usize, usize) {
        let parsed = self.parsed_inline.borrow();
        (parsed.0.len(), parsed.1)
    }

    /// [`Page::scoped_sheets_now`] of the tree at `top`, the document or
    /// a shadow root.
    fn scoped_sheets_now_of(
        &self,
        document: &Document,
        media: &Media,
        top: NodeId,
        base: &OnceCell<Option<Url>>,
    ) -> (Vec<css::Stylesheet>, Vec<Option<NodeId>>) {
        let preferred = self.preferred();
        let csp = self.policies();
        let mut sheets = Vec::new();
        let mut roots = Vec::new();
        // The owners by element, found once, not searched for each node.
        let owners: HashMap<NodeId, &Owner> = self
            .owners
            .iter()
            .map(|owner| (owner.node, owner))
            .collect();
        for node in document.subtree(top) {
            match owners.get(&node).copied() {
                Some(owner) => {
                    if let Slot::Ready(sheet) = &owner.sheet
                        && (owner.title.is_empty() || Some(owner.title.as_str()) == preferred)
                        && !self.disabled.contains(&sheet.serial)
                    {
                        sheets.push(css::Stylesheet {
                            rules: rules::wrap_in_media(owner.media.as_deref(), sheet.rules()),
                        });
                        roots.push(document.parent_element(node));
                    }
                }
                None if document.tag(node) == Some("style")
                    && document.attr(node, "type").is_none_or(|kind| {
                        kind.is_empty() || kind.eq_ignore_ascii_case("text/css")
                    })
                    && rules::media_attribute_holds(document.attr(node, "media"), media) =>
                {
                    let text = document.text_content(node);
                    let nonce = self.nonce(document, node);
                    let violations = csp.inline_violations(Inline::Style, &nonce, &text);
                    if net::csp::enforced(violations).is_ok()
                        && let Some(sheet) = self.parse_inline(
                            base.get_or_init(|| self.base_url(document)).as_ref(),
                            text,
                        )
                    {
                        sheets.push(sheet);
                        roots.push(document.parent_element(node));
                    }
                }
                None => {}
            }
        }
        let scope = Some(top).filter(|&top| document.is_shadow_root(top));
        let first_adopted = sheets.len();
        self.adopted_sheets(scope, &mut sheets);
        roots.resize(roots.len() + sheets.len() - first_adopted, None);
        (sheets, roots)
    }

    /// Sets the disabled flag of the sheet `serial` (CSSOM's `disabled`):
    /// the cascade leaves it out while it is set.
    pub(crate) fn set_sheet_disabled(&mut self, serial: u64, disabled: bool) {
        self.sheets_generation += 1;
        let changed = if disabled {
            self.disabled.len() < MAX_CONSTRUCTED_SHEETS && self.disabled.insert(serial)
        } else {
            self.disabled.remove(&serial)
        };
        self.cssom_edited |= changed;
    }

    /// Sets the `media` of the constructed sheet `serial`: its rules apply
    /// where the list holds, none for every medium.
    pub(crate) fn set_constructed_media(&mut self, serial: u64, media: &str) {
        self.sheets_generation += 1;
        let Some(sheet) = self.constructed.iter_mut().find(|c| c.serial == serial) else {
            return;
        };
        let media = (!media.trim().is_empty()).then(|| media.to_owned());
        if sheet.media != media {
            sheet.media = media;
            self.cssom_edited |= self.adopted.contains(&serial);
        }
    }

    /// The CSSOM's `document.styleSheets` (D66): each element whose
    /// sheet is there, in tree order, with its sheet's serial number.
    pub(crate) fn cssom_sheets(&self, scope: Option<NodeId>) -> Vec<(NodeId, u64)> {
        self.owners
            .iter()
            .filter(|owner| owner.scope == scope)
            .filter_map(|owner| match &owner.sheet {
                Slot::Ready(sheet) => Some((owner.node, sheet.serial)),
                _ => None,
            })
            .collect()
    }

    /// The sheet of `owner`, or of an import of its at `imports` (each
    /// an import's place among its sheet's `@import` rules).
    pub(crate) fn cssom_sheet(&self, owner: NodeId, imports: &[usize]) -> Option<SheetView<'_>> {
        let (owner, _) = self.owner_of(owner)?;
        let mut sheet = match &owner.sheet {
            Slot::Ready(sheet) => sheet,
            _ => return None,
        };
        for &i in imports {
            sheet = match &sheet.imports.get(i)?.sheet {
                Slot::Ready(inner) => inner,
                _ => return None,
            };
        }
        Some(SheetView {
            rules: &sheet.rules,
            location: sheet.location.as_ref(),
            serial: sheet.serial,
        })
    }

    /// The `title` and `media` of `owner`'s element, and whether the
    /// sheet is still the element's.
    pub(crate) fn cssom_owner(&self, owner: NodeId) -> Option<(&str, Option<&str>, bool)> {
        let (owner, attached) = self.owner_of(owner)?;
        Some((owner.title.as_str(), owner.media.as_deref(), attached))
    }

    /// Changes the rules of `owner`'s sheet, or an import's, by `edit`:
    /// what a script does through the CSSOM. The `@import` rules keep
    /// their fetched sheets by `kept`, which the edit gives as the old
    /// index of each rule now there (`None` for one it made).
    pub(crate) fn cssom_edit<R>(
        &mut self,
        owner: NodeId,
        imports: &[usize],
        edit: impl FnOnce(&mut css::Stylesheet) -> Option<(R, Vec<Option<usize>>)>,
    ) -> Option<R> {
        self.sheets_generation += 1;
        let owner = match self.owners.iter_mut().find(|o| o.node == owner) {
            Some(found) => found,
            None => self.detached.iter_mut().rev().find(|o| o.node == owner)?,
        };
        let mut sheet = match &mut owner.sheet {
            Slot::Ready(sheet) => sheet,
            _ => return None,
        };
        for &i in imports {
            sheet = match &mut sheet.imports.get_mut(i)?.sheet {
                Slot::Ready(inner) => inner,
                _ => return None,
            };
        }
        let (result, kept) = edit(&mut sheet.rules)?;
        // The imports follow their rules; one whose rule went goes.
        let imports = std::mem::take(&mut sheet.imports);
        sheet.imports = imports
            .into_iter()
            .filter_map(|mut imported| {
                let now = kept
                    .iter()
                    .position(|&old| old == Some(imported.import.rule))?;
                imported.import.rule = now;
                Some(imported)
            })
            .collect();
        self.cssom_edited = true;
        Some(result)
    }

    /// Whether a script changed a sheet since this was last asked.
    pub(crate) fn take_cssom_edited(&mut self) -> bool {
        std::mem::take(&mut self.cssom_edited)
    }

    /// Registers a custom property for the document's scripts
    /// (`CSS.registerProperty`, Properties and Values API section 3.1):
    /// one whose name is registered already is refused with an
    /// `InvalidModificationError`, and past [`MAX_REGISTERED`] more are
    /// refused too.
    pub(crate) fn register_property(
        &mut self,
        registration: style::PropertyRegistration,
    ) -> Result<(), js::DomError> {
        self.sheets_generation += 1;
        if self.registered.iter().any(|r| r.name == registration.name) {
            return Err(js::DomError::new(
                "InvalidModificationError",
                format!("The name '{}' is already registered.", registration.name),
            ));
        }
        if self.registered.len() >= MAX_REGISTERED {
            return Err(js::DomError::new(
                "QuotaExceededError",
                "The document registered too many custom properties.",
            ));
        }
        self.registered.push(registration);
        self.cssom_edited = true;
        Ok(())
    }

    /// The custom properties the document's scripts registered, in order.
    pub(crate) fn registered_properties(&self) -> &[style::PropertyRegistration] {
        &self.registered
    }

    /// A new constructed sheet, empty, whose rules' URLs resolve against
    /// `base`: its serial number; none past [`MAX_CONSTRUCTED_SHEETS`].
    pub(crate) fn construct_sheet(&mut self, base: Option<Url>) -> Option<u64> {
        if self.constructed.len() >= MAX_CONSTRUCTED_SHEETS {
            return None;
        }
        let serial = next_serial();
        self.constructed.push(Constructed {
            serial,
            rules: css::Stylesheet::default(),
            base,
            media: None,
        });
        Some(serial)
    }

    /// `replaceSync` of the constructed sheet `serial` (CSSOM 6.1.2): its
    /// rules those of `text`, its `@import` rules left out; whether it is
    /// there.
    pub(crate) fn replace_sheet(&mut self, serial: u64, text: &str) -> bool {
        self.sheets_generation += 1;
        let Some(sheet) = self.constructed.iter_mut().find(|c| c.serial == serial) else {
            return false;
        };
        let mut rules = css::parse_stylesheet(text).unwrap_or_default();
        rules.rules.retain(
            |rule| !matches!(rule, css::Rule::At(at) if at.name.eq_ignore_ascii_case("import")),
        );
        if let Some(base) = &sheet.base {
            rules::absolute_urls(&mut rules.rules, base);
        }
        sheet.rules = rules;
        self.cssom_edited |= self.adopted.contains(&serial);
        true
    }

    /// The constructed sheet `serial`, as the CSSOM reads it.
    pub(crate) fn constructed_sheet(&self, serial: u64) -> Option<SheetView<'_>> {
        let sheet = self.constructed.iter().find(|c| c.serial == serial)?;
        Some(SheetView {
            rules: &sheet.rules,
            location: None,
            serial,
        })
    }

    /// Changes the rules of the constructed sheet `serial` by `edit`, as
    /// [`Page::cssom_edit`] does an element's.
    pub(crate) fn constructed_edit<R>(
        &mut self,
        serial: u64,
        edit: impl FnOnce(&mut css::Stylesheet) -> Option<(R, Vec<Option<usize>>)>,
    ) -> Option<R> {
        self.sheets_generation += 1;
        let sheet = self.constructed.iter_mut().find(|c| c.serial == serial)?;
        let (result, _) = edit(&mut sheet.rules)?;
        self.cssom_edited |= self.adopted.contains(&serial);
        Some(result)
    }

    /// `document.adoptedStyleSheets` set to the constructed sheets
    /// `serials`, in order; those not constructed left out.
    pub(crate) fn adopt_sheets(&mut self, serials: Vec<u64>) {
        self.sheets_generation += 1;
        let serials: Vec<u64> = serials
            .into_iter()
            .filter(|serial| self.constructed.iter().any(|c| c.serial == *serial))
            .collect();
        if serials != self.adopted {
            self.adopted = serials;
            self.cssom_edited = true;
        }
    }

    /// A shadow root's `adoptedStyleSheets` set to the sheets `serials`
    /// numbers (D69).
    pub(crate) fn adopt_shadow_sheets(&mut self, root: NodeId, serials: Vec<u64>) {
        self.sheets_generation += 1;
        let serials: Vec<u64> = serials
            .into_iter()
            .filter(|serial| self.constructed.iter().any(|c| c.serial == *serial))
            .collect();
        let old = self
            .adopted_shadow
            .get(&root)
            .map_or(&[][..], Vec::as_slice);
        if serials != old {
            if serials.is_empty() {
                self.adopted_shadow.remove(&root);
            } else if self.adopted_shadow.len() < MAX_CONSTRUCTED_SHEETS {
                self.adopted_shadow.insert(root, serials);
            }
            self.cssom_edited = true;
        }
    }

    /// Forgets the adopted sheets of the shadow roots not in `document`
    /// any more.
    pub(crate) fn forget_shadow_sheets(&mut self, document: &Document) {
        self.adopted_shadow
            .retain(|&root, _| document.contains(root));
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
            href,
            crossorigin,
            integrity,
            ..
        } if link_wants_sheet(key) => href
            .as_deref()
            .zip(base)
            .and_then(|(href, base)| Url::parse_with(href, Some(base), context.encoding).ok())
            .and_then(|url| context.request(url))
            .map(|request| super::with_cors_settings(request, crossorigin.as_deref()))
            .map(|mut request| {
                // Its element's nonce (M33.1), referrer (M33.3) and
                // integrity (D62).
                request.nonce.clone_from(&found.nonce);
                request.integrity = integrity.clone().unwrap_or_default();
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
                serial: next_serial(),
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
        scope: found.scope,
        key: found.key,
        title: found.title,
        media: found.media,
        environment,
        blocking: false,
        blocks_scripts: false,
        sheet,
        imports_waiting: false,
    }
}

/// [`start`] for a `<style>` element in a script's run, which has no
/// network: its imports wait for the rescan after the run.
fn start_inline(context: &Context, found: Found, csp: &Csp) -> Owner {
    let sheet = match &found.key {
        key @ Key::Style { text, .. } if style_makes_sheet(key) => {
            let violations = csp.inline_violations(Inline::Style, &found.nonce, text);
            csp.record(&violations);
            if net::csp::enforced(violations).is_err() {
                Slot::Failed
            } else {
                Slot::Ready(Box::new(Sheet {
                    serial: next_serial(),
                    rules: css::parse_stylesheet(text).unwrap_or_default(),
                    location: None,
                    encoding: context.encoding,
                    imports: Vec::new(),
                }))
            }
        }
        _ => Slot::None,
    };
    Owner {
        node: found.node,
        scope: found.scope,
        key: found.key,
        title: found.title,
        media: found.media,
        environment: context.encoding,
        blocking: false,
        blocks_scripts: false,
        imports_waiting: matches!(&sheet, Slot::Ready(made) if made.rules.rules.iter().any(
            |rule| matches!(rule, css::Rule::At(at) if at.name.eq_ignore_ascii_case("import"))
        )),
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
    // The shadow trees' elements too, each tree's sheets its own (D69).
    let nodes = match document.has_shadow_roots() {
        true => document.shadow_including_subtree(document.root()),
        false => document.subtree(document.root()).collect(),
    };
    for node in nodes {
        let tag = match document.tag(node) {
            Some(tag) => tag,
            // SVG's `<style>` owns a sheet as HTML's does (SVG 2 section
            // 6.3, M32).
            None if is_svg_style(document, node) => "style",
            None => continue,
        };
        let scope = Some(document.root_of(node)).filter(|&top| document.is_shadow_root(top));
        // A shadow tree's `base` and `body` are no document's.
        if scope.is_some() && matches!(tag, "base" | "body") {
            continue;
        }
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
                scope,
                key: Key::Link {
                    href: attr("href"),
                    rel: attr("rel"),
                    kind: attr("type"),
                    disabled: document.attr(node, "disabled").is_some(),
                    crossorigin: attr("crossorigin"),
                    integrity: attr("integrity"),
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
                scope,
                key: Key::Style {
                    text: document
                        .children(node)
                        .filter_map(|child| document.text(child))
                        .collect(),
                    kind: attr("type"),
                    nodes: document
                        .children(node)
                        .filter(|&child| document.text(child).is_some())
                        .collect(),
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
                // A node freed since cannot be looked into; the shadow
                // trees in what came or went hold sheets too (D69).
                || !document.contains(*child)
                || document
                    .shadow_including_subtree(*child)
                    .into_iter()
                    .any(owns)
        }
        Change::AttributeChanged { element, .. } => owns(*element),
        Change::TextChanged { node } => document.parent(*node).is_some_and(is_style),
        Change::StateChanged { .. }
        | Change::ControlChanged { .. }
        | Change::Moved { .. }
        | Change::FlatTree { .. } => false,
        Change::Everything => true,
    }
}

#[cfg(test)]
mod parsed_inline_tests {
    use super::*;

    #[test]
    fn style_texts_are_kept_parsed_by_their_text_and_bounded_by_their_bytes() {
        // The text is the parser's whole input: one text, one entry, the
        // same sheet; another text, another entry.
        let page = Page::given(0);
        let a = page.parse_inline(None, "p { color: red }".to_owned());
        let again = page.parse_inline(None, "p { color: red }".to_owned());
        assert!(a.is_some() && a == again);
        assert_eq!(page.parsed_inline().0, 1);
        page.parse_inline(None, "p { color: blue }".to_owned());
        assert_eq!(page.parsed_inline().0, 2);
        // The same text at another base is another entry, at the same
        // base the same one.
        let one = Url::parse("http://a.test/one/").unwrap();
        let two = Url::parse("http://a.test/two/").unwrap();
        page.parse_inline(Some(&one), "p { color: red }".to_owned());
        assert_eq!(page.parsed_inline().0, 3);
        page.parse_inline(Some(&two), "p { color: red }".to_owned());
        page.parse_inline(Some(&two), "p { color: red }".to_owned());
        assert_eq!(page.parsed_inline().0, 4);
        // Twelve sheets of a MiB each, kept 8 MiB at most.
        let rule = "p { color: red }\n".repeat(61_700);
        for n in 0..12 {
            page.parse_inline(None, format!("/* {n} */{rule}"));
            assert!(page.parsed_inline().1 <= MAX_PARSED_INLINE_BYTES, "{n}");
        }
        assert!(page.parsed_inline().0 > 0);
    }
}
