//! A document's stylesheets (docs/design/net.md, "The engine's
//! loader"): the `<link>` and `<style>` elements that own them, in tree
//! order, each with its sheet and the sheets that sheet imports, fetched
//! or failed or on their way.

use std::collections::HashMap;

use dom::{Change, Document, NodeId};
use encoding::Encoding;
use net::{AddressSpace, Destination, Initiator, MimeType, Request, Response, Url};

use super::{FetchId, LoadError, Network, PageId, Target, rules};

/// How deep imports go: a sheet's imports are one deeper than it, and
/// one deeper than this is not fetched.
pub(crate) const MAX_IMPORT_DEPTH: usize = 16;

/// One document's stylesheets.
pub(crate) struct Page {
    context: Context,
    /// The encoding the document is in when the engine cannot decode
    /// it yet, and decoded it as UTF-8 instead.
    undecodable: Option<Encoding>,
    /// The elements that may own a sheet, `<link>` and `<style>`, in
    /// tree order.
    owners: Vec<Owner>,
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
}

/// An element that may own a sheet.
struct Owner {
    node: NodeId,
    /// What the element's sheet was made from; a change to it makes the
    /// sheet again.
    key: Key,
    /// The element's `title`, empty when it has none.
    title: String,
    /// The environment encoding of a fetched sheet: the `<link>`'s
    /// `charset`, or the document's encoding.
    environment: Encoding,
    /// Whether the document is not shown until the sheet has arrived or
    /// failed: the parser made the element before `<body>`.
    blocking: bool,
    sheet: Slot,
}

/// The attributes, and for `<style>` the text, that decide an element's
/// sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Key {
    Link {
        href: Option<String>,
        rel: Option<String>,
        kind: Option<String>,
        media: Option<String>,
        disabled: bool,
    },
    Style {
        text: String,
        kind: Option<String>,
        media: Option<String>,
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
    /// The sheets its `@import` rules fetch, in order: each one's rules
    /// come before the sheet's own.
    imports: Vec<Slot>,
}

/// An element found by a walk over the document.
struct Found {
    node: NodeId,
    key: Key,
    title: String,
    charset: Option<String>,
    /// The base URL as it was when the parser inserted the element: that
    /// of the first `<base href>` before it in tree order.
    base_here: Option<Url>,
    before_body: bool,
}

impl Page {
    /// The page of a document given to the engine: its `<style>`
    /// elements make sheets, and nothing is fetched.
    pub(crate) fn given(id: PageId) -> Page {
        Page {
            context: Context {
                id,
                url: None,
                encoding: encoding::UTF_8,
                address_space: AddressSpace::Public,
            },
            undecodable: None,
            owners: Vec::new(),
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
        Page {
            context: Context {
                id,
                url: Some(url),
                encoding,
                address_space,
            },
            undecodable,
            owners: Vec::new(),
        }
    }

    pub(crate) fn id(&self) -> PageId {
        self.context.id
    }

    /// The document's URL.
    pub(crate) fn url(&self) -> Option<&Url> {
        self.context.url.as_ref()
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
            Some(href) => frozen_base_url(href, &self.context),
            None => self.context.url.clone(),
        }
    }

    /// `href` parsed against the document's base URL with the
    /// document's encoding: HTML's "encoding-parse a URL". `None` when
    /// it does not parse.
    pub(crate) fn resolve(&self, document: &Document, href: &str) -> Option<Url> {
        let base = self.base_url(document);
        Url::parse_with(href, base.as_ref(), self.context.encoding).ok()
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
    /// was where the parser inserted it, and those before `<body>`
    /// blocking.
    pub(crate) fn begin(&mut self, document: &Document, network: &mut Network) {
        let (found, _) = scan(document, &self.context);
        let context = &self.context;
        self.owners = found
            .into_iter()
            .map(|found| {
                let base = found.base_here.clone();
                let blocking = found.before_body;
                let mut owner = start(context, found, base.as_ref(), network);
                owner.blocking = blocking;
                owner
            })
            .collect();
    }

    /// Finds the sheets of `document` again after it changed, keeping
    /// each element's sheet whose key is unchanged and making the others
    /// afresh, against the base URL as it now stands. A sheet kept is
    /// not fetched again for a new base URL, as HTML's base URL change
    /// steps leave `<link>` elements alone. The fetches of the sheets
    /// dropped are aborted.
    pub(crate) fn rescan(&mut self, document: &Document, network: &mut Network) {
        let (found, base) = scan(document, &self.context);
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
                    owners.push(owner);
                }
                kept => {
                    if kept.is_some() {
                        network.cancel(self.context.id, Some(found.node));
                    }
                    owners.push(start(&self.context, found, base.as_ref(), network));
                }
            }
        }
        for node in old.keys() {
            network.cancel(self.context.id, Some(*node));
        }
        self.owners = owners;
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
            slot = next;
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
        let mut sheet = Sheet {
            rules: css::parse_stylesheet(&text).unwrap_or_default(),
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

    /// Whether every sheet, with its imports, has arrived or failed.
    pub(crate) fn settled(&self) -> bool {
        self.owners.iter().all(|owner| owner.sheet.settled())
    }

    /// The sheets that apply, in cascade order: each element's in tree
    /// order, its imports before it. The first titled sheet's title
    /// names the preferred set, and a sheet with another title is left
    /// out (CSSOM §6.1's add steps).
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
                sheet.push_to(&mut sheets);
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
            Slot::Ready(sheet) => sheet.imports.iter_mut().for_each(Slot::fail_waiting),
            Slot::None | Slot::Failed => {}
        }
    }

    fn settled(&self) -> bool {
        match self {
            Slot::Waiting(_) => false,
            Slot::Ready(sheet) => sheet.imports.iter().all(Slot::settled),
            Slot::None | Slot::Failed => true,
        }
    }
}

impl Sheet {
    /// Pushes the sheet's imports that arrived, then the sheet.
    fn push_to(&self, sheets: &mut Vec<css::Stylesheet>) {
        for import in &self.imports {
            if let Slot::Ready(sheet) = import {
                sheet.push_to(sheets);
            }
        }
        sheets.push(self.rules.clone());
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
            }),
        )
    }
}

/// Whether a response is a stylesheet to apply: an ok status, and a
/// `Content-Type` of `text/css`, or none, which is taken as `text/css`
/// unless `X-Content-Type-Options: nosniff` says otherwise (Fetch's
/// should response to request be blocked due to nosniff).
fn is_stylesheet(response: &Response) -> bool {
    response.ok()
        && match &response.mime_type {
            Some(mime_type) => mime_type.essence() == "text/css",
            None => !net::mime::determine_nosniff(&response.headers),
        }
}

/// The owner `found` describes, its sheet begun: a `<link>`'s fetched,
/// its URL resolved against `base`, and a `<style>`'s parsed, with its
/// imports fetched.
fn start(context: &Context, found: Found, base: Option<&Url>, network: &mut Network) -> Owner {
    let environment = found
        .charset
        .as_deref()
        .and_then(Encoding::for_label)
        .unwrap_or(context.encoding);
    let sheet = match &found.key {
        key @ Key::Link { href, .. } if link_wants_sheet(key) => href
            .as_deref()
            .zip(base)
            .and_then(|(href, base)| Url::parse_with(href, Some(base), context.encoding).ok())
            .and_then(|url| context.request(url))
            .map_or(Slot::None, |request| {
                let target = Target::Sheet {
                    owner: found.node,
                    path: Vec::new(),
                };
                Slot::Waiting(network.fetch(context.id, target, request))
            }),
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
        environment,
        blocking: false,
        sheet,
    }
}

/// Fetches the imports of `sheet`, at `path` in `owner`'s sheets, each
/// resolved against the sheet's location, or against `base` for a
/// `<style>` element's sheet. An import of a sheet in `chain`, the
/// locations from the owner's sheet down to this one, is skipped, and
/// so are imports deeper than [`MAX_IMPORT_DEPTH`].
fn fetch_imports(
    context: &Context,
    owner: NodeId,
    path: &[usize],
    chain: &[String],
    sheet: &mut Sheet,
    base: Option<&Url>,
    network: &mut Network,
) {
    if path.len() >= MAX_IMPORT_DEPTH {
        return;
    }
    let Some(against) = sheet.location.clone().or_else(|| base.cloned()) else {
        return;
    };
    for written in rules::imports(&sheet.rules) {
        // URLs in CSS are parsed as UTF-8 (css-values-4 §4.5.4).
        let Ok(url) = against.join(&written) else {
            continue;
        };
        if chain.iter().any(|above| above == url.without_fragment()) {
            continue;
        }
        let Some(request) = context.request(url) else {
            continue;
        };
        let mut import_path = path.to_vec();
        import_path.push(sheet.imports.len());
        let target = Target::Sheet {
            owner,
            path: import_path,
        };
        sheet
            .imports
            .push(Slot::Waiting(network.fetch(context.id, target, request)));
    }
}

/// Whether a `<link>` is fetched as a stylesheet: its `rel` has the
/// token `stylesheet` and not `alternate`, it is not `disabled`, its
/// `type`, if given, is `text/css`, its `media` holds, and its `href`
/// is not empty.
fn link_wants_sheet(key: &Key) -> bool {
    let Key::Link {
        href,
        rel,
        kind,
        media,
        disabled,
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
        && rules::media_attribute_matches(media.as_deref())
        && href.as_deref().is_some_and(|href| !href.is_empty())
}

/// Whether a `<style>` makes a sheet: its `type`, if given and not
/// empty, is `text/css` in any case, and its `media` holds (HTML's
/// update a style block).
fn style_makes_sheet(key: &Key) -> bool {
    let Key::Style { kind, media, .. } = key else {
        return false;
    };
    kind.as_deref()
        .is_none_or(|kind| kind.is_empty() || kind.eq_ignore_ascii_case("text/css"))
        && rules::media_attribute_matches(media.as_deref())
}

fn is_ascii_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{c}' | '\r' | ' ')
}

/// The elements of `document` that may own a sheet, in tree order, and
/// its base URL.
fn scan(document: &Document, context: &Context) -> (Vec<Found>, Option<Url>) {
    let html = document.document_element();
    let mut found = Vec::new();
    let mut base = context.url.clone();
    let mut base_seen = false;
    let mut before_body = true;
    for node in document.subtree(document.root()) {
        let Some(tag) = document.tag(node) else {
            continue;
        };
        let attr = |name: &str| document.attr(node, name).map(str::to_owned);
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
                    media: attr("media"),
                    disabled: document.attr(node, "disabled").is_some(),
                },
                title: title(),
                charset: attr("charset"),
                base_here: base.clone(),
                before_body,
            }),
            "style" => found.push(Found {
                node,
                key: Key::Style {
                    text: document
                        .children(node)
                        .filter_map(|child| document.text(child))
                        .collect(),
                    kind: attr("type"),
                    media: attr("media"),
                },
                title: title(),
                charset: None,
                base_here: base.clone(),
                before_body,
            }),
            _ => {}
        }
    }
    (found, base)
}

/// A `<base>`'s frozen base URL (HTML §4.2.3): its `href` parsed against
/// the document's URL, or that URL when it fails to parse or is
/// `data:` or `javascript:`.
fn frozen_base_url(href: &str, context: &Context) -> Option<Url> {
    let fallback = context.url.clone()?;
    match Url::parse_with(href, Some(&fallback), context.encoding) {
        Ok(url) if !matches!(url.scheme(), "data" | "javascript") => Some(url),
        _ => Some(fallback),
    }
}

/// Whether `change` can change a document's sheets: something inserted
/// or removed holds a `<link>` or `<style>`, or a `<style>`'s text
/// changed, or one of those elements' attributes did. A `<base>` alone
/// changes no sheet: those already made keep their URLs, and the next
/// sheet made reads the base URL as it then stands.
pub(crate) fn may_touch_sheets(document: &Document, change: &Change) -> bool {
    let owns = |node: NodeId| matches!(document.tag(node), Some("link" | "style"));
    let is_style = |node: NodeId| document.tag(node) == Some("style");
    match change {
        Change::Inserted { parent, child } | Change::Removed { parent, child, .. } => {
            is_style(*parent)
                // A node freed since cannot be looked into.
                || !document.contains(*child)
                || document.subtree(*child).any(owns)
        }
        Change::AttributeChanged { element, .. } => owns(*element),
        Change::TextChanged { node } => document.parent(*node).is_some_and(is_style),
        Change::Everything => true,
    }
}
