//! SVG images (M32, `docs/design/svg.md`, "SVG images"): a response of
//! `image/svg+xml`, or of no type whose bytes begin like SVG, read by the
//! XML parser, its root an SVG `svg` or the image broken; its natural
//! size from its `width`, `height` and `viewBox`; drawn at the size it
//! lands on in raster pixels, styled by its own sheets alone. It runs no
//! script, loads nothing outside itself and plays no animation.

use std::sync::Arc;

use dom::{Document, Namespace, NodeId};
use images::{Format, Header, Image, Orientation, Vector};
use style::{CascadeContext, Stylesheets};

/// The natural size an SVG image reports when it has none (CSS Images 3
/// section 4.3's default object size).
const DEFAULT: (f32, f32) = (300.0, 150.0);

/// Whether a response typed `mime` with `body` is an SVG image: typed
/// `image/svg+xml`, or typed nothing an image is known by and beginning,
/// past white space, a declaration, comments and a doctype, with `<svg`.
pub(crate) fn is_svg(mime: Option<&str>, body: &[u8]) -> bool {
    match mime {
        Some("image/svg+xml") => true,
        Some(m) if m.starts_with("image/") => false,
        _ => begins_like_svg(body),
    }
}

fn begins_like_svg(body: &[u8]) -> bool {
    let head = &body[..body.len().min(4096)];
    let Ok(text) =
        std::str::from_utf8(head).or_else(|e| std::str::from_utf8(&head[..e.valid_up_to()]))
    else {
        return false;
    };
    let mut rest = text.trim_start_matches('\u{feff}');
    loop {
        rest = rest.trim_start();
        if rest.starts_with("<svg") {
            return true;
        }
        let close = if rest.starts_with("<?") {
            "?>"
        } else if rest.starts_with("<!--") {
            "-->"
        } else if rest.starts_with("<!") {
            ">"
        } else {
            return false;
        };
        match rest.find(close) {
            Some(at) => rest = &rest[at + close.len()..],
            None => return false,
        }
    }
}

/// An SVG image's document and its root `<svg>`: `None` for bytes that
/// are not well formed, or whose root is anything else.
fn parsed(bytes: &[u8]) -> Option<(Document, NodeId)> {
    let document = html::xml::parse_xml(bytes).ok()?;
    let root = document.element_children(document.root()).next()?;
    (document.namespace(root) == Some(&Namespace::Svg) && document.svg_tag(root) == Some("svg"))
        .then_some((document, root))
}

/// The natural size of the SVG image whose root is `svg`: its `width`
/// and `height` where they are absolute lengths (`em` of the initial
/// 16 pixels), its `viewBox`'s ratio or else theirs, a side it lacks
/// from the other through the ratio.
fn natural(document: &Document, svg: NodeId) -> Vector {
    let side = |name: &str| {
        let length = svg::parse_length(document.attr(svg, name)?)?;
        if length.unit == svg::Unit::Percent || length.value < 0.0 {
            return None;
        }
        let em = svg::Viewport {
            width: 0.0,
            height: 0.0,
            font_size: 16.0,
            root_font_size: 16.0,
        };
        let px = length.resolve(svg::Axis::X, &em) as f32;
        px.is_finite().then_some(px)
    };
    let (mut width, mut height) = (side("width"), side("height"));
    let ratio = document
        .attr(svg, "viewBox")
        .and_then(svg::parse_view_box)
        .filter(svg::ViewBox::has_area)
        .map(|v| (v.width / v.height) as f32)
        .or_else(|| match (width, height) {
            (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some(w / h),
            _ => None,
        })
        .filter(|r| r.is_finite() && *r > 0.0);
    if let Some(r) = ratio {
        match (width, height) {
            (Some(w), None) => height = Some(w / r),
            (None, Some(h)) => width = Some(h * r),
            _ => {}
        }
    }
    Vector {
        width,
        height,
        ratio,
    }
}

/// An SVG image's header: its natural size as `images::Vector` has it,
/// and as the size it reports, 300 by 150 where it has none.
pub(crate) fn header(bytes: &[u8]) -> Result<Header, images::Error> {
    let (document, svg) = parsed(bytes).ok_or(images::Error::Malformed)?;
    let vector = natural(&document, svg);
    let reported = |v: Option<f32>, default: f32| v.unwrap_or(default).round().max(0.0) as u32;
    Ok(Header {
        format: Format::Svg,
        width: reported(vector.width, DEFAULT.0),
        height: reported(vector.height, DEFAULT.1),
        orientation: Orientation::Normal,
        vector: Some(vector),
    })
}

/// An SVG image drawn `size` raster pixels big, its text in `fonts`: its
/// render tree built for a viewport of its natural size (its viewBox
/// mapped onto that), or of `size` where it has none, and scaled to fill
/// `size` (the reference browser draws an image without a `viewBox` as
/// if its natural size were one).
pub(crate) fn draw(
    bytes: &[u8],
    size: (u32, u32),
    fonts: Option<&text::FontCollection>,
) -> Result<Image, images::Error> {
    let (document, svg) = parsed(bytes).ok_or(images::Error::Malformed)?;
    let vector = natural(&document, svg);
    let (w, h) = (size.0.max(1), size.1.max(1));
    let viewport = match (vector.width, vector.height) {
        (Some(vw), Some(vh)) if vw > 0.0 && vh > 0.0 => (f64::from(vw), f64::from(vh)),
        _ => (f64::from(w), f64::from(h)),
    };
    let sheets: Vec<css::Stylesheet> = document
        .subtree(document.root())
        .filter(|&n| document.svg_tag(n) == Some("style"))
        .filter_map(|n| css::parse_stylesheet(&document.text_content(n)).ok())
        .collect();
    let sheets = Stylesheets::new(sheets);
    let context = CascadeContext {
        viewport_width: viewport.0 as f32,
        viewport_height: viewport.1 as f32,
        ..CascadeContext::default()
    };
    let styles = sheets.cascade(&document, &context);
    let recascade = sheets.recascader(&document, &styles, &context);
    let tree = svg::build(
        svg::Context {
            document: &document,
            styles: &styles,
            recascade: Some(&recascade),
            fonts,
        },
        svg,
        viewport.0,
        viewport.1,
    );
    let mut canvas = canvas::Canvas::new(w, h, false);
    let base = canvas::Matrix::new(
        f64::from(w) / viewport.0,
        0.0,
        0.0,
        f64::from(h) / viewport.1,
        0.0,
        0.0,
    );
    svg::draw(&tree, &mut canvas, &base);
    let image = canvas.snapshot().ok_or(images::Error::TooLarge)?;
    Ok(Arc::unwrap_or_clone(image))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_svg_image_is_known_by_its_type_or_its_beginning() {
        let svg = b"<svg xmlns='http://www.w3.org/2000/svg'/>";
        assert!(is_svg(Some("image/svg+xml"), b"anything"));
        assert!(
            !is_svg(Some("image/png"), svg),
            "typed an image of another kind"
        );
        assert!(is_svg(None, svg));
        assert!(is_svg(Some("text/plain"), svg));
        let prologue = "\u{feff}  <?xml version='1.0'?>\n<!-- made -->\n\
                        <!DOCTYPE svg PUBLIC '-//W3C//DTD SVG 1.1//EN' 'x'>\n<svg>";
        assert!(is_svg(
            Some("application/octet-stream"),
            prologue.as_bytes()
        ));
        assert!(!is_svg(None, b"<html><svg>"));
        assert!(!is_svg(None, b"<!-- unclosed <svg>"));
        assert!(is_svg(None, b"<!-- a > b --><svg>"), "a comment to its end");
        assert!(!is_svg(None, b"\x89PNG\r\n"));
    }

    #[test]
    fn a_header_is_the_natural_size_and_a_drawing_any_size() {
        let header = |text: &str| header(text.as_bytes()).ok().map(|h| (h.width, h.height));
        let svg = |attrs: &str| format!("<svg xmlns='http://www.w3.org/2000/svg' {attrs}/>");
        assert_eq!(header(&svg("width='40' height='20'")), Some((40, 20)));
        assert_eq!(header(&svg("width='40' viewBox='0 0 4 1'")), Some((40, 10)));
        assert_eq!(header(&svg("height='-3'")), Some((300, 150)));
        assert_eq!(header("<svg/>"), None, "no namespace");
        assert_eq!(
            header("<svg xmlns='http://www.w3.org/2000/svg'>"),
            None,
            "not well formed"
        );
        let vector = super::header(svg("viewBox='0 0 2 1'").as_bytes())
            .expect("a header")
            .vector
            .expect("a vector");
        assert_eq!(
            (vector.width, vector.height, vector.ratio),
            (None, None, Some(2.0))
        );
        let sized = super::header(svg("width='40' height='20'").as_bytes())
            .expect("a header")
            .vector
            .expect("a vector");
        assert_eq!(sized.ratio, Some(2.0), "its size's ratio, with no viewBox");
        // Drawn at the size asked, its own sheet styling it, a natural
        // size scaled to fill it.
        let bytes = b"<svg xmlns='http://www.w3.org/2000/svg' width='2' height='1'>\
                      <style>rect { fill: lime }</style><rect width='1' height='1'/></svg>";
        let image = draw(bytes, (20, 10), None).expect("drawn");
        assert_eq!((image.width, image.height), (20, 10));
        let at = |x: usize, y: usize| {
            let i = (y * 20 + x) * 4;
            image.frames[0].pixels[i..i + 4].to_vec()
        };
        assert_eq!(at(5, 5), [0, 255, 0, 255]);
        assert_eq!(at(15, 5), [0, 0, 0, 0]);
    }
}
