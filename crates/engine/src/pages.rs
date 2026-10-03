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
/// load, and the engine's demonstration of what it draws.
pub(crate) const ABOUT_SILVER: &str = "\
<html>\
  <head>\
    <title>Silver Browser</title>\
    <style>\
      html { background-color: #1e1e2e; }\
      h1 { color: #cdd6f4; }\
      p { color: #a6adc8; }\
      .highlight { color: #f9e2af; font-size: 20px; }\
      .cards {\
        display: flex;\
        flex-wrap: wrap;\
        align-items: flex-start;\
        gap: 16px;\
        margin: 16px 0;\
      }\
      .card {\
        display: block;\
        flex: 1 1 220px;\
        padding: 12px;\
        border-width: 2px;\
        border-style: solid;\
        border-color: #89b4fa;\
        background-color: #313244;\
      }\
      .card::before {\
        content: attr(data-label);\
        display: block;\
        margin-bottom: 4px;\
        color: #89b4fa;\
        font-size: 12px;\
        text-transform: uppercase;\
      }\
      .card::after { content: \"\u{a7}\"; display: block; text-align: right; color: #6c7086; }\
      .lead::first-letter {\
        float: left;\
        font-size: 52px;\
        padding-right: 6px;\
        color: #f9e2af;\
      }\
      .lead::first-line { text-transform: uppercase; color: #cdd6f4; }\
      q:lang(fr) { quotes: \"\u{ab}\u{a0}\" \"\u{a0}\u{bb}\" \"\u{2039}\u{a0}\" \"\u{a0}\u{203a}\"; }\
      .clipped { height: 40px; overflow: hidden; }\
      .scrolls { height: 60px; overflow: auto; }\
      a { color: #89b4fa; }\
    </style>\
  </head>\
  <body>\
    <h1 id=top>Hello from Silver Browser</h1>\
    <p>A hobby browser engine in Rust. This is about:silver, the browser's own page.</p>\
    <p class=\"highlight\">This page is laid out by our own block formatting context — real width/margin resolution and margin collapsing, not a flat vertical list.</p>\
    <p class=\"lead\">Pseudo-elements (M16): this paragraph opens with a drop cap, its first letter floated left and set large by ::first-letter, and its first line is in capitals by ::first-line, however wide the window makes it. The cards below are labelled by ::before, from their data-label attributes, and closed by ::after. Quotation marks come from the quotes property: <q>a quotation, with <q>one inside it</q></q>, and in French, chosen by :lang(fr), <q lang=\"fr\">une citation</q>.</p>\
    <div class=\"cards\">\
    <div class=\"card\" data-label=\"Flexible boxes\">\
      <p>These cards are flex items (M17): they share a row, grow into the room it leaves, and wrap onto another when the window is too narrow for them, with a gap between them either way.</p>\
    </div>\
    <div class=\"card\" data-label=\"The box model\">\
      <p>This card's background and border are painted from real fragment geometry (M6.8) — the box model actually exists now.</p>\
    </div>\
    <div class=\"card clipped\" data-label=\"overflow: hidden\">\
      <p>This card is 40px tall with overflow: hidden, so this second sentence is clipped away at its padding edge rather than painted over whatever follows. The clip is a rectangle in the display list (M9.3).</p>\
    </div>\
    <div class=\"card scrolls\" data-label=\"overflow: auto\">\
      <p>This card is 60px tall with overflow: auto, so it scrolls on its own: turn the wheel over it and its content moves inside the clip, with the overlay scrollbar at its right edge showing where it is (M10.5). At the end of its content the wheel goes on to the page, which is scroll chaining.</p>\
      <p>A second paragraph, to give the card something more to scroll through.</p>\
    </div>\
    </div>\
    <p>Scroll with the wheel, the arrow keys, Page Up and Page Down, Home and End, the space bar, or by dragging the scrollbar at the right (M9.2). <a href=\"#end\">This link</a> goes to the end of the page without leaving it, a navigation to a fragment (M15); Back comes back here.</p>\
    <p>The paragraphs below only exist to make the page taller than the window.</p>\
    <p>One. The quick brown fox jumps over the lazy dog, and the layout is cached across scrolls (M9.5): only paint and raster run again.</p>\
    <p>Two. The quick brown fox jumps over the lazy dog.</p>\
    <p>Three. The quick brown fox jumps over the lazy dog.</p>\
    <p>Four. The quick brown fox jumps over the lazy dog.</p>\
    <p>Five. The quick brown fox jumps over the lazy dog.</p>\
    <p>Six. The quick brown fox jumps over the lazy dog.</p>\
    <p>Seven. The quick brown fox jumps over the lazy dog.</p>\
    <p>Eight. The quick brown fox jumps over the lazy dog.</p>\
    <p>Nine. The quick brown fox jumps over the lazy dog.</p>\
    <p id=end>Ten. The end of the page. <a href=\"#top\">Back to the top</a>.</p>\
  </body>\
</html>";

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
