//! The pages the browser makes (docs/design/navigation.md, "Pages the
//! browser makes"): `about:silver`, and the pages shown in place of a
//! response there is nothing to show for — a network error, an error
//! status with an empty body, a type this version cannot show — and the
//! text documents made from text responses.
//!
//! Each is built as a tree, not as markup: text that comes from outside
//! — a URL, an error's reason, a MIME type, a server's text — goes into
//! a text node, so nothing in it can become an element.

use dom::{Document, Element, Node};
use net::{NetworkError, TlsFailure, Url};

use crate::LoadError;

/// `about:silver`, the built-in page: the page shown with nothing to
/// load, and the engine's demonstration of what it draws. It is a file
/// of its own, which the print tests also print through the reference
/// browser.
pub(crate) const ABOUT_SILVER: &str = include_str!("about-silver.html");

/// The style of the pages the browser makes in place of a response.
const STYLE: &str = "\
html { background-color: #f6f6f4; }\
body { margin: 48px; color: #202124; }\
h1 { font-size: 24px; color: #202124; }\
p { color: #3c4043; }\
.detail { color: #6b6e72; font-size: 14px; }";

/// The style HTML lets the user agent add to a text document: lines
/// wrap, so a long one can be read.
const TEXT_STYLE: &str = "pre { white-space: pre-wrap }";

/// The page shown, under `url`, when its navigation failed with `error`.
pub(crate) fn error(url: &Url, error: &LoadError) -> Document {
    let place = place(url);
    let (heading, explanation) = match error {
        LoadError::Network(error) => network_error(url, &place, error),
        LoadError::Lost => (
            format!("Can't load {place}"),
            "Something went wrong in the browser while loading this page.".to_owned(),
        ),
    };
    page(
        &heading,
        &heading,
        vec![
            paragraph("", explanation),
            paragraph("detail", url.as_str().to_owned()),
            paragraph("detail", format!("The error: {error}.")),
        ],
    )
}

/// The heading and the explanation for a network error.
fn network_error(url: &Url, place: &str, error: &NetworkError) -> (String, String) {
    let reach = format!("Can't reach {place}");
    match error {
        NetworkError::BadScheme(_) if url.scheme() == "about" => (
            format!("No page at {}", url.as_str()),
            "Silver Browser has no page by that name.".to_owned(),
        ),
        NetworkError::BadScheme(scheme) => (
            format!("Can't open {scheme}: addresses"),
            format!("This version of Silver Browser cannot open {scheme}: addresses."),
        ),
        NetworkError::BadPort(port) => (
            reach,
            format!(
                "Port {port} is one browsers never connect to, so that a web page cannot \
                 use them to reach the other services that listen there."
            ),
        ),
        NetworkError::AddressSpace { .. } | NetworkError::MixedContent => (
            reach,
            "The page that asked for this one may not reach it.".to_owned(),
        ),
        NetworkError::Resolve(_) => (
            format!("Can't find {place}"),
            format!(
                "The name {place} did not resolve to an address. Check that it is spelled right."
            ),
        ),
        NetworkError::Connect(_) => (reach, format!("No connection could be made to {place}.")),
        NetworkError::Tls(failure) => (
            format!("Can't connect securely to {place}"),
            format!(
                "{} The connection would not be private, and there is no way past this.",
                tls_failure(failure)
            ),
        ),
        NetworkError::Malformed(_) | NetworkError::UnaskedUpgrade => (
            reach,
            format!("{place} sent a response that breaks the rules of HTTP."),
        ),
        NetworkError::Truncated | NetworkError::Io(_) => (
            reach,
            "The connection failed before the page had arrived.".to_owned(),
        ),
        NetworkError::HeadersTooLarge | NetworkError::BodyTooLarge => (
            reach,
            "The response is larger than this browser takes.".to_owned(),
        ),
        NetworkError::ContentCoding => (
            reach,
            "The page arrived compressed, and would not decompress.".to_owned(),
        ),
        NetworkError::DataUrl => (
            "Can't open this data: address".to_owned(),
            "It is not a data: address that can be read.".to_owned(),
        ),
        NetworkError::File(_) => (
            "Can't open this file".to_owned(),
            "There is no file there that can be read.".to_owned(),
        ),
        NetworkError::Timeout => (reach, format!("{place} took too long to answer.")),
        NetworkError::RedirectLimit => (reach, "The page redirected too many times.".to_owned()),
        NetworkError::BadRedirect => (
            reach,
            "The page redirected to somewhere a redirect may not go.".to_owned(),
        ),
        NetworkError::Aborted => (reach, "The load was stopped.".to_owned()),
        NetworkError::Csp(_) => (
            reach,
            "The page's Content-Security-Policy does not allow it.".to_owned(),
        ),
    }
}

/// What is wrong with a certificate, in a sentence.
fn tls_failure(failure: &TlsFailure) -> String {
    match failure {
        TlsFailure::Expired => "The site's certificate has expired.".to_owned(),
        TlsFailure::NotYetValid => {
            "The site's certificate is not valid yet; this computer's clock may be wrong."
                .to_owned()
        }
        TlsFailure::UnknownIssuer => {
            "The site's certificate was not issued by an authority this browser trusts.".to_owned()
        }
        TlsFailure::WrongName => "The site's certificate is for another name.".to_owned(),
        TlsFailure::Other(_) => "The secure connection could not be made.".to_owned(),
    }
}

/// The page shown, under `url`, for a response with an error status
/// and nothing in its body: the status, as Chromium shows it.
pub(crate) fn status(url: &Url, status: u16, message: &[u8]) -> Document {
    let reason = String::from_utf8_lossy(message);
    let line = match reason.trim() {
        "" => format!("HTTP {status}"),
        reason => format!("HTTP {status} {reason}"),
    };
    page(
        &format!("{} — {line}", place(url)),
        &line,
        vec![
            paragraph(
                "",
                format!(
                    "{} answered with this status, and sent nothing to show.",
                    place(url)
                ),
            ),
            paragraph("detail", url.as_str().to_owned()),
        ],
    )
}

/// The page shown, under `url`, for a response of a type this version
/// cannot show yet: an image, audio or video.
pub(crate) fn unshowable(url: &Url, essence: &str) -> Document {
    page(
        &format!("Can't show {}", place(url)),
        "Can't show this yet",
        vec![
            paragraph(
                "",
                format!(
                    "This is a {essence} file, which this version of Silver Browser \
                     cannot show yet."
                ),
            ),
            paragraph("detail", url.as_str().to_owned()),
        ],
    )
}

/// The page shown, under `url`, in place of a document a form's POST
/// made, on a reload or a return to it (M26.9, G57): the reference
/// browser asks before it sends the form again; this says why it was not.
pub(crate) fn resubmission(url: &Url) -> Document {
    page(
        "Confirm Form Resubmission",
        "Confirm Form Resubmission",
        vec![
            paragraph(
                "",
                "This page was made from information sent in a form. Sending it again \
                 could repeat what it did, such as a purchase, so it was not sent again."
                    .to_owned(),
            ),
            paragraph("detail", url.as_str().to_owned()),
        ],
    )
}

/// A text document, as HTML's "load a text document" makes one: one
/// `<pre>` holding `text`, with the style HTML lets the user agent add.
/// The text is what an HTML parser in its PLAINTEXT state would put in
/// the `<pre>`: each line break as a line feed, each NUL replaced.
pub(crate) fn text(text: &str) -> Document {
    let text = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\0', "\u{FFFD}");
    let html = Element::new("html")
        .push(Element::new("head").push(Element::new("style").text(TEXT_STYLE)))
        .push(Element::new("body").push(Element::new("pre").text(text)));
    Document::from_node(&Node::document(vec![html.into()]))
}

/// Where a URL leads, for a sentence: its host, or else its scheme.
fn place(url: &Url) -> String {
    match url.host() {
        Some(host) if !host.to_string().is_empty() => host.to_string(),
        _ => match url.scheme() {
            "file" => "this file".to_owned(),
            scheme => format!("this {scheme}: address"),
        },
    }
}

/// A paragraph of `class` (none when empty) holding `text`.
fn paragraph(class: &str, text: String) -> Element {
    let p = Element::new("p");
    let p = if class.is_empty() {
        p
    } else {
        p.attr("class", class)
    };
    p.text(text)
}

/// A page of the browser's own: `title`, a heading, and paragraphs.
fn page(title: &str, heading: &str, paragraphs: Vec<Element>) -> Document {
    let head = Element::new("head")
        .push(Element::new("title").text(title))
        .push(Element::new("style").text(STYLE));
    let body = Element::new("body")
        .push(Element::new("h1").text(heading))
        .push_children(paragraphs.into_iter().map(Node::from));
    let html = Element::new("html").push(head).push(body);
    Document::from_node(&Node::document(vec![html.into()]))
}
