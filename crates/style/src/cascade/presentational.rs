//! HTML's presentational hints (HTML section 15, gap batch D, D60): the
//! attributes the rendering section maps to CSS, as the text of a
//! declaration block that goes in the cascade with the other hints, under
//! every author rule. The list's, an image's and a frame's own sizes are
//! in `lists` and `images`.

use dom::{Document, NodeId};

/// Whether `name` on an element named `tag` is a presentational hint
/// here, whose change restyles the element: and the element's subtree
/// where its descendants read it (a table's `border` and `cellpadding`,
/// which its cells take, and `body`'s link colours).
pub(crate) fn is_presentational_hint(tag: &str, name: &str) -> bool {
    match name {
        "align" => matches!(
            tag,
            "div"
                | "p"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "caption"
                | "table"
                | "tr"
                | "td"
                | "th"
                | "thead"
                | "tbody"
                | "tfoot"
                | "col"
                | "colgroup"
                | "hr"
                | "img"
                | "iframe"
                | "embed"
                | "object"
                | "input"
                | "legend"
        ),
        "bgcolor" | "background" => matches!(
            tag,
            "body" | "table" | "tr" | "td" | "th" | "thead" | "tbody" | "tfoot"
        ),
        "valign" => matches!(
            tag,
            "tr" | "td" | "th" | "thead" | "tbody" | "tfoot" | "col" | "colgroup"
        ),
        "width" => matches!(
            tag,
            "table" | "td" | "th" | "col" | "colgroup" | "hr" | "pre"
        ),
        "height" => matches!(tag, "table" | "tr" | "td" | "th"),
        "border" => matches!(tag, "table" | "img" | "object"),
        "cellpadding" | "cellspacing" => tag == "table",
        "nowrap" => matches!(tag, "td" | "th"),
        "hspace" | "vspace" => matches!(tag, "img" | "iframe" | "embed" | "object" | "video"),
        "color" => matches!(tag, "font" | "hr"),
        "face" => tag == "font",
        "size" => matches!(tag, "font" | "hr"),
        "noshade" => tag == "hr",
        "text" | "link" => tag == "body",
        _ => false,
    }
}

/// Whether a change of `name` on `tag` reaches the element's descendants.
pub(crate) fn reaches_descendants(tag: &str, name: &str) -> bool {
    matches!(
        (tag, name),
        ("table", "border" | "cellpadding") | ("body", "link")
    )
}

/// The declarations `element`'s presentational attributes give, as the
/// text of a declaration block; empty for none.
pub(super) fn hints(document: &Document, element: NodeId, tag: &str) -> String {
    let attr = |name: &str| document.attr(element, name);
    let mut out = String::new();
    let mut push = |text: String| out.push_str(&text);
    let dimension = |name: &str, nonzero: bool| {
        attr(name)
            .and_then(dom::parse_dimension)
            .and_then(|d| match d {
                dom::Dimension::Length(px) if nonzero && px == 0.0 => None,
                dom::Dimension::Length(px) => Some(format!("{px}px")),
                dom::Dimension::Percentage(p) => Some(format!("{p}%")),
            })
    };
    let pixels = |name: &str| {
        attr(name)
            .and_then(dom::parse_integer)
            .filter(|n| *n >= 0)
            .map(|n| n.min(1 << 20))
    };
    let colour = |name: &str| attr(name).and_then(legacy_colour);
    let image = |name: &str| {
        attr(name)
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(|url| format!("background-image: url({});", css_string(url)))
    };
    match tag {
        "body" => {
            if let Some(c) = colour("bgcolor") {
                push(format!("background-color: {c};"));
            }
            push(image("background").unwrap_or_default());
            if let Some(c) = colour("text") {
                push(format!("color: {c};"));
            }
        }
        // A link takes `body`'s `link` colour (every link is unvisited).
        "a" | "area" if attr("href").is_some() => {
            let body = document
                .subtree(document.root())
                .find(|&n| document.tag(n) == Some("body"));
            if let Some(c) = body
                .and_then(|b| document.attr(b, "link"))
                .and_then(legacy_colour)
            {
                push(format!("color: {c};"));
            }
        }
        "div" | "caption" | "legend" => {
            if tag == "caption"
                && let Some(side) =
                    attr("align").and_then(|a| match a.trim().to_ascii_lowercase().as_str() {
                        "top" => Some("top"),
                        "bottom" => Some("bottom"),
                        _ => None,
                    })
            {
                push(format!("caption-side: {side};"));
            } else if let Some(align) = attr("align").and_then(legacy_text_align) {
                push(format!("text-align: {align};"));
            }
        }
        "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            if let Some(align) = attr("align").and_then(plain_text_align) {
                push(format!("text-align: {align};"));
            }
        }
        "table" => {
            match attr("align")
                .map(|a| a.trim().to_ascii_lowercase())
                .as_deref()
            {
                Some("left") => push("float: left;".into()),
                Some("right") => push("float: right;".into()),
                Some("center") => push("margin-left: auto; margin-right: auto;".into()),
                _ => {}
            }
            if let Some(w) = dimension("width", true) {
                push(format!("width: {w};"));
            }
            if let Some(h) = dimension("height", true) {
                push(format!("height: {h};"));
            }
            if let Some(c) = colour("bgcolor") {
                push(format!("background-color: {c};"));
            }
            push(image("background").unwrap_or_default());
            if let Some(s) = pixels("cellspacing") {
                push(format!("border-spacing: {s}px;"));
            }
            if let Some(b) = table_border(document, element) {
                push(format!("border-width: {b}px;"));
                if b > 0 {
                    push("border-style: outset;".into());
                }
            }
        }
        "tr" | "thead" | "tbody" | "tfoot" | "td" | "th" => {
            if let Some(align) = attr("align").and_then(legacy_text_align) {
                push(format!("text-align: {align};"));
            }
            if let Some(v) = attr("valign").and_then(vertical_align) {
                push(format!("vertical-align: {v};"));
            }
            if let Some(c) = colour("bgcolor") {
                push(format!("background-color: {c};"));
            }
            push(image("background").unwrap_or_default());
            if tag == "tr"
                && let Some(h) = dimension("height", false)
            {
                push(format!("height: {h};"));
            }
            if matches!(tag, "td" | "th") {
                if let Some(w) = dimension("width", true) {
                    push(format!("width: {w};"));
                }
                if let Some(h) = dimension("height", true) {
                    push(format!("height: {h};"));
                }
                if attr("nowrap").is_some() {
                    push("white-space: nowrap;".into());
                }
                let table = ancestor(document, element, "table");
                if let Some(table) = table {
                    if table_border(document, table).is_some_and(|b| b > 0) {
                        push("border-width: 1px; border-style: inset;".into());
                    }
                    let padding = document
                        .attr(table, "cellpadding")
                        .and_then(dom::parse_integer)
                        .filter(|n| *n >= 0);
                    if let Some(p) = padding {
                        push(format!("padding: {}px;", p.min(1 << 20)));
                    }
                }
            }
        }
        "col" | "colgroup" => {
            if let Some(w) = dimension("width", true) {
                push(format!("width: {w};"));
            }
            if let Some(v) = attr("valign").and_then(vertical_align) {
                push(format!("vertical-align: {v};"));
            }
        }
        "hr" => {
            match attr("align")
                .map(|a| a.trim().to_ascii_lowercase())
                .as_deref()
            {
                Some("left") => push("margin-left: 0; margin-right: auto;".into()),
                Some("right") => push("margin-left: auto; margin-right: 0;".into()),
                Some("center") => push("margin-left: auto; margin-right: auto;".into()),
                _ => {}
            }
            let colour = colour("color");
            if attr("noshade").is_some() || colour.is_some() {
                push("border-style: solid;".into());
                let c = colour.unwrap_or_else(|| "gray".into());
                push(format!("border-color: {c}; background-color: {c};"));
            }
            if let Some(size) = pixels("size") {
                if size == 1 {
                    push("border-bottom-width: 0;".into());
                } else if size > 1 {
                    push(format!("height: {}px;", size - 2));
                }
            }
            if let Some(w) = dimension("width", false) {
                push(format!("width: {w};"));
            }
        }
        "pre" => {}
        "img" | "iframe" | "embed" | "object" | "video" | "input" => {
            if tag != "input" || attr("type").is_some_and(|t| t.eq_ignore_ascii_case("image")) {
                if let Some(h) = pixels("hspace").filter(|_| tag != "input") {
                    push(format!("margin-left: {h}px; margin-right: {h}px;"));
                }
                if let Some(v) = pixels("vspace").filter(|_| tag != "input") {
                    push(format!("margin-top: {v}px; margin-bottom: {v}px;"));
                }
                if let Some(a) = attr("align").and_then(image_align) {
                    push(a.into());
                }
            }
            if matches!(tag, "img" | "object")
                && let Some(b) = pixels("border")
            {
                push(format!("border-width: {b}px; border-style: solid;"));
            }
        }
        "font" => {
            if let Some(c) = colour("color") {
                push(format!("color: {c};"));
            }
            if let Some(face) = attr("face").map(str::trim).filter(|f| !f.is_empty()) {
                let families: Vec<String> = face
                    .split(',')
                    .map(str::trim)
                    .filter(|f| !f.is_empty())
                    .map(css_string)
                    .collect();
                if !families.is_empty() {
                    push(format!("font-family: {};", families.join(", ")));
                }
            }
            if let Some(size) = attr("size").and_then(legacy_font_size) {
                push(format!("font-size: {size};"));
            }
        }
        _ => {}
    }
    out
}

/// The nearest ancestor of `element` named `tag`.
fn ancestor(document: &Document, element: NodeId, tag: &str) -> Option<NodeId> {
    let mut at = document.parent(element);
    while let Some(node) = at {
        if document.tag(node) == Some(tag) {
            return Some(node);
        }
        at = document.parent(node);
    }
    None
}

/// A table's `border`: its integer, an empty or unparsable one 1.
fn table_border(document: &Document, table: NodeId) -> Option<i64> {
    let value = document.attr(table, "border")?;
    Some(
        dom::parse_integer(value)
            .map_or(1, |n| n.max(0))
            .min(1 << 20),
    )
}

/// `align` as the legacy `text-align` blocks and table parts take.
fn legacy_text_align(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "left" => Some("-webkit-left"),
        "right" => Some("-webkit-right"),
        "center" | "middle" => Some("-webkit-center"),
        "justify" => Some("justify"),
        _ => None,
    }
}

/// `align` as the plain `text-align` paragraphs and headings take.
fn plain_text_align(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "left" => Some("left"),
        "right" => Some("right"),
        "center" => Some("center"),
        "justify" => Some("justify"),
        _ => None,
    }
}

/// `valign` as `vertical-align`.
fn vertical_align(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "top" => Some("top"),
        "middle" | "center" => Some("middle"),
        "bottom" => Some("bottom"),
        "baseline" => Some("baseline"),
        _ => None,
    }
}

/// An image's (or a frame's) `align` (HTML section 15.4.3).
fn image_align(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "left" => Some("float: left;"),
        "right" => Some("float: right;"),
        "top" => Some("vertical-align: top;"),
        "middle" | "absmiddle" | "abscenter" => Some("vertical-align: middle;"),
        "bottom" | "baseline" => Some("vertical-align: baseline;"),
        "texttop" => Some("vertical-align: text-top;"),
        "absbottom" => Some("vertical-align: bottom;"),
        _ => None,
    }
}

/// `value` as a CSS string.
fn css_string(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' | '\r' | '\u{c}' => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// HTML's rules for parsing a legacy font size (section 15.3.6): a
/// number, after `+` or `-` relative to 3, clamped to 1 to 7, as the
/// absolute size keyword.
pub(crate) fn legacy_font_size(value: &str) -> Option<&'static str> {
    let value = value.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let (sign, digits) = match value.as_bytes().first()? {
        b'+' => (1, &value[1..]),
        b'-' => (-1, &value[1..]),
        _ => (0, value),
    };
    let end = digits.bytes().take_while(u8::is_ascii_digit).count();
    let n: i64 = digits[..end].parse().ok()?;
    let size = match sign {
        1 => 3 + n,
        -1 => 3 - n,
        _ => n,
    }
    .clamp(1, 7);
    Some(
        [
            "x-small",
            "small",
            "medium",
            "large",
            "x-large",
            "xx-large",
            "xxx-large",
        ][size as usize - 1],
    )
}

/// HTML's rules for parsing a legacy colour value (section 2.3.6): a
/// colour keyword, `#rgb`, or the rest read as hexadecimal by thirds,
/// as `rgb()` text; `None` for an empty value or `transparent`.
pub(crate) fn legacy_colour(value: &str) -> Option<String> {
    let value = value.trim_matches(|c: char| c.is_ascii_whitespace());
    if value.is_empty() || value.eq_ignore_ascii_case("transparent") {
        return None;
    }
    if let Some(c) = super::named_color(&value.to_ascii_lowercase()) {
        let to = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        return Some(format!("rgb({}, {}, {})", to(c.r), to(c.g), to(c.b)));
    }
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let bytes = value.as_bytes();
    if bytes.len() == 4 && bytes[0] == b'#' && bytes[1..].iter().all(|b| hex(*b).is_some()) {
        let [r, g, b] = [1, 2, 3].map(|i| hex(bytes[i]).unwrap_or(0) * 17);
        return Some(format!("rgb({r}, {g}, {b})"));
    }
    // Past the BMP a character is "00"; the input cut at 128; a leading
    // `#` dropped; what is no hexadecimal digit `0`.
    let mut digits: String = value
        .chars()
        .flat_map(|c| {
            if (c as u32) > 0xFFFF {
                vec!['0', '0']
            } else {
                vec![c]
            }
        })
        .take(128)
        .collect();
    if digits.starts_with('#') {
        digits.remove(0);
    }
    // Each character that is no hexadecimal digit one `0`.
    let mut digits: Vec<u8> = digits
        .chars()
        .map(|c| if c.is_ascii_hexdigit() { c as u8 } else { b'0' })
        .collect();
    while digits.is_empty() || !digits.len().is_multiple_of(3) {
        digits.push(b'0');
    }
    let third = digits.len() / 3;
    let mut parts: Vec<&[u8]> = digits.chunks(third).collect();
    let mut length = third;
    if length > 8 {
        parts = parts.iter().map(|p| &p[length - 8..]).collect();
        length = 8;
    }
    while length > 2 && parts.iter().all(|p| p[0] == b'0') {
        parts = parts.iter().map(|p| &p[1..]).collect();
        length -= 1;
    }
    let channel = |p: &[u8]| -> u8 {
        let p = &p[..p.len().min(2)];
        p.iter().fold(0u8, |acc, b| {
            acc.wrapping_mul(16).wrapping_add(hex(*b).unwrap_or(0))
        })
    };
    Some(format!(
        "rgb({}, {}, {})",
        channel(parts[0]),
        channel(parts[1]),
        channel(parts[2])
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computed::LengthPercentageAuto;

    use crate::cascade::{CascadeContext, cascade_document};
    use crate::computed::{
        BorderStyle, Color, ComputedStyle, Display, Float, LegacyAlign, LengthPercentage,
        TextAlign, VerticalAlign, WhiteSpace,
    };

    /// The style of the element with id `id` in `html`.
    fn style(html: &str, id: &str) -> ComputedStyle {
        let document = Document::from_node(&html::parse_document(html).expect("never fails"));
        let styles = cascade_document(&document, &[], &CascadeContext::default());
        let node = document
            .subtree(document.root())
            .find(|&n| document.attr(n, "id") == Some(id))
            .expect("the element");
        styles.get(node).expect("styled").clone()
    }

    #[test]
    fn html_attributes_are_hints_under_every_author_rule() {
        let html = "<body bgcolor=yellow text=red link=lime><a id=a href=x>a</a>\
            <table id=t border=3 cellpadding=5 cellspacing=7 width=50% bgcolor=#00f align=center>\
            <tr id=r valign=top align=right><td id=d nowrap height=20>d</td></tr></table>\
            <div id=v align=center><p id=p align=justify>p</p></div>\
            <font id=f color=green face='A, B' size=+2>f</font>\
            <hr id=h noshade size=4 width=30 align=left>\
            <img id=i hspace=4 vspace=6 border=2 align=right>\
            <p id=x hidden>hidden</p></body>";
        let s = |id| style(html, id);
        let rgb = Color::rgb8;
        assert_eq!(s("a").color, rgb(0, 255, 0));
        let table = s("t");
        assert_eq!(table.background_color, rgb(0, 0, 255));
        assert_eq!(table.border_spacing.horizontal, 7.0);
        assert_eq!(table.used_border_width(), [3.0; 4]);
        assert_eq!(table.border_style[0], BorderStyle::Outset);
        assert_eq!(table.width, crate::computed::Size::Percent(50.0));
        assert!(table.margin[1].is_auto() && table.margin[3].is_auto());
        let row = s("r");
        assert_eq!(
            (row.vertical_align, row.text_align),
            (VerticalAlign::Top, TextAlign::Right)
        );
        assert_eq!(row.legacy_align, LegacyAlign::Right);
        let cell = s("d");
        assert_eq!(cell.used_border_width(), [1.0; 4]);
        assert_eq!(cell.border_style[0], BorderStyle::Inset);
        assert_eq!(cell.padding, [const { LengthPercentage::Px(5.0) }; 4]);
        assert_eq!(cell.white_space, WhiteSpace::NOWRAP);
        assert_eq!(
            cell.vertical_align,
            VerticalAlign::Top,
            "inherited from the row"
        );
        let div = s("v");
        assert_eq!(
            (div.text_align, div.legacy_align),
            (TextAlign::Center, LegacyAlign::Center)
        );
        let p = s("p");
        assert_eq!(
            (p.text_align, p.legacy_align, p.placed_by),
            (TextAlign::Justify, LegacyAlign::None, LegacyAlign::Center)
        );
        let font = s("f");
        assert_eq!(font.color, rgb(0, 128, 0));
        assert_eq!(&*font.font().family, "A, B");
        assert_eq!(font.font_size_px, 24.0);
        let hr = s("h");
        assert_eq!(hr.border_style[0], BorderStyle::Solid);
        assert_eq!(hr.height, crate::computed::Size::Px(2.0));
        assert!(hr.margin[1].is_auto());
        let img = s("i");
        assert_eq!(img.float, Float::Right);
        assert_eq!(img.used_border_width(), [2.0; 4]);
        assert_eq!(img.margin[1], LengthPercentageAuto::Px(4.0));
        assert_eq!(img.margin[0], LengthPercentageAuto::Px(6.0));
        assert_eq!(s("x").display, Display::None);
        // A `border` with no number is 1.
        let bare = "<table id=t border><tr><td id=d>d</td></tr></table>";
        assert_eq!(style(bare, "t").used_border_width(), [1.0; 4]);
        // `match-parent` takes the parent's legacy value; `larger` is the
        // parent's size by 1.2.
        let matched = "<div align=center><p id=p style='text-align: match-parent; font-size: larger'>p</p></div>";
        let p = style(matched, "p");
        assert_eq!(p.legacy_align, LegacyAlign::Center);
        assert_eq!(p.font_size_px, 16.0 * 1.2);
        // An author's declaration wins over a hint.
        let author = "<table cellpadding=9><tr><td id=d style='padding: 0'>d</td></tr></table>";
        assert_eq!(
            style(author, "d").padding,
            [const { LengthPercentage::Px(0.0) }; 4]
        );
    }

    #[test]
    fn legacy_colours_are_read_as_html_reads_them() {
        let c = |v: &str| legacy_colour(v);
        assert_eq!(c("red").as_deref(), Some("rgb(255, 0, 0)"));
        assert_eq!(c(" #f80 ").as_deref(), Some("rgb(255, 136, 0)"));
        assert_eq!(c("#ff8800").as_deref(), Some("rgb(255, 136, 0)"));
        assert_eq!(c("chucknorris").as_deref(), Some("rgb(192, 0, 0)"));
        assert_eq!(c("ninja").as_deref(), Some("rgb(0, 0, 160)"));
        assert_eq!(c("crap").as_deref(), Some("rgb(192, 160, 0)"));
        assert_eq!(c("#1234567890").as_deref(), Some("rgb(18, 86, 144)"));
        // Past eight digits a third keeps its last eight; leading zeros
        // all three share go.
        assert_eq!(
            c("#123456789abcdef0123456789").as_deref(),
            Some("rgb(35, 188, 69)")
        );
        assert_eq!(c("#0f0f0f0f0f0f").as_deref(), Some("rgb(240, 240, 240)"));
        // A character past ASCII is one `0`, past the BMP two.
        assert_eq!(c("\u{e9}ff").as_deref(), Some("rgb(0, 15, 15)"));
        assert_eq!(c("\u{1f600}f").as_deref(), Some("rgb(0, 0, 15)"));
        assert_eq!(c(""), None);
        assert_eq!(c("transparent"), None);
    }

    #[test]
    fn legacy_font_sizes_are_relative_to_three_and_clamped() {
        let s = |v: &str| legacy_font_size(v);
        assert_eq!(s("1"), Some("x-small"));
        assert_eq!(s("+1"), Some("large"));
        assert_eq!(s("-2"), Some("x-small"));
        assert_eq!(s("9"), Some("xxx-large"));
        assert_eq!(s("0"), Some("x-small"));
        assert_eq!(s("4px"), Some("large"));
        assert_eq!(s("x"), None);
        assert_eq!(s(""), None);
    }
}
