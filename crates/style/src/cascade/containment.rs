//! The containment properties of [`crate::computed::Containment`] (CSS
//! Containment 2, CSS Sizing 4 section 5, gap batch H), `pointer-events`
//! and the reading flow (CSS Display 4 section 5). Each parser takes a
//! declaration's value as the cascade has it and says `None` when the
//! value is not one the property takes, so that the declaration is
//! dropped.

use css::Token;

use super::{LengthBases, Range, components, integer, length, single_ident};
use crate::computed::{Contain, ContainIntrinsicSize, ContentVisibility, ReadingFlow};

/// The keyword `tokens` are, lowercased.
fn keyword(tokens: &[Token]) -> Option<String> {
    single_ident(tokens).map(str::to_ascii_lowercase)
}

/// `contain`: `none`, `strict`, `content`, or any of `size` (or
/// `inline-size`), `layout`, `style` and `paint`, each once, in any
/// order (CSS Containment 2 section 2).
pub(super) fn parse_contain(tokens: &[Token]) -> Option<Contain> {
    let parts = components(tokens);
    if let [only] = parts.as_slice() {
        match keyword(only)?.as_str() {
            "none" => return Some(Contain::NONE),
            "strict" => return Some(Contain::STRICT),
            "content" => return Some(Contain::CONTENT),
            _ => {}
        }
    }
    let mut contain = Contain::NONE;
    for part in parts {
        let flag = match keyword(part)?.as_str() {
            "size" => Contain::SIZE,
            "inline-size" => Contain::INLINE_SIZE,
            "layout" => Contain::LAYOUT,
            "style" => Contain::STYLE,
            "paint" => Contain::PAINT,
            _ => return None,
        };
        let sizes = Contain::SIZE.with(Contain::INLINE_SIZE);
        if contain.0 & flag.0 != 0 || (flag.0 & sizes.0 != 0 && contain.0 & sizes.0 != 0) {
            return None;
        }
        contain = contain.with(flag);
    }
    (contain != Contain::NONE).then_some(contain)
}

/// `content-visibility: visible | auto | hidden` (section 4).
pub(super) fn parse_content_visibility(tokens: &[Token]) -> Option<ContentVisibility> {
    Some(match keyword(tokens)?.as_str() {
        "visible" => ContentVisibility::Visible,
        "auto" => ContentVisibility::Auto,
        "hidden" => ContentVisibility::Hidden,
        _ => return None,
    })
}

/// One axis of `contain-intrinsic-size` from `parts`, its components
/// from `*at` on, `*at` moved past it: `none`, a length not below zero,
/// `auto` and either of those.
fn intrinsic_axis(
    parts: &[&[Token]],
    at: &mut usize,
    bases: &LengthBases<'_>,
) -> Option<ContainIntrinsicSize> {
    let auto = parts
        .get(*at)
        .and_then(|part| keyword(part))
        .is_some_and(|word| word == "auto");
    if auto {
        *at += 1;
    }
    let part = parts.get(*at)?;
    *at += 1;
    let size = match keyword(part).as_deref() {
        Some("none") => None,
        Some(_) => return None,
        None => Some(length(part, bases, Range::NonNegative)?),
    };
    Some(match (auto, size) {
        (true, size) => ContainIntrinsicSize::Auto(size),
        (false, None) => ContainIntrinsicSize::None,
        (false, Some(px)) => ContainIntrinsicSize::Length(px),
    })
}

/// A longhand of `contain-intrinsic-size` (`contain-intrinsic-width`,
/// `-height`, `-block-size`, `-inline-size`).
pub(super) fn parse_contain_intrinsic_axis(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<ContainIntrinsicSize> {
    let parts = components(tokens);
    let mut at = 0;
    let axis = intrinsic_axis(&parts, &mut at, bases)?;
    (at == parts.len()).then_some(axis)
}

/// `contain-intrinsic-size`: the width's value, then the height's, the
/// one value for both when only one is written.
pub(super) fn parse_contain_intrinsic_size(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<[ContainIntrinsicSize; 2]> {
    let parts = components(tokens);
    let mut at = 0;
    let width = intrinsic_axis(&parts, &mut at, bases)?;
    if at == parts.len() {
        return Some([width, width]);
    }
    let height = intrinsic_axis(&parts, &mut at, bases)?;
    (at == parts.len()).then_some([width, height])
}

/// `reading-flow` (CSS Display 4 section 5.1), the values the reference
/// browser takes.
pub(super) fn parse_reading_flow(tokens: &[Token]) -> Option<ReadingFlow> {
    Some(match keyword(tokens)?.as_str() {
        "normal" => ReadingFlow::Normal,
        "flex-visual" => ReadingFlow::FlexVisual,
        "flex-flow" => ReadingFlow::FlexFlow,
        "grid-rows" => ReadingFlow::GridRows,
        "grid-columns" => ReadingFlow::GridColumns,
        "grid-order" => ReadingFlow::GridOrder,
        "source-order" => ReadingFlow::SourceOrder,
        _ => return None,
    })
}

/// `reading-order`: an integer (section 5.2).
pub(super) fn parse_reading_order(tokens: &[Token], bases: &LengthBases<'_>) -> Option<i32> {
    integer(tokens, bases)
}

#[cfg(test)]
mod tests {
    use super::super::with_default_bases;
    use super::*;

    fn tokens(value: &str) -> Vec<Token> {
        css::tokenize(value)
    }

    #[test]
    fn contain_is_written_in_its_canonical_order() {
        let written = |v: &str| crate::serialize::specified_of("contain", &tokens(v));
        assert_eq!(written("layout size"), "size layout");
        assert_eq!(written("paint style"), "style paint");
        assert_eq!(
            written("layout paint style size"),
            "size layout style paint"
        );
        assert_eq!(written("strict"), "strict");
        assert_eq!(
            crate::serialize::specified_of("float", &tokens("left")),
            "left"
        );
    }

    #[test]
    fn contain_takes_its_keywords_once_each() {
        let contain = |v: &str| parse_contain(&tokens(v));
        assert_eq!(contain("none"), Some(Contain::NONE));
        assert_eq!(contain("strict"), Some(Contain::STRICT));
        assert_eq!(contain("content"), Some(Contain::CONTENT));
        assert_eq!(
            contain("paint layout"),
            Some(Contain::PAINT.with(Contain::LAYOUT))
        );
        assert_eq!(
            contain("inline-size style"),
            Some(Contain::INLINE_SIZE.with(Contain::STYLE))
        );
        for bad in [
            "",
            "size size",
            "size inline-size",
            "strict paint",
            "none size",
            "auto",
            "10px",
        ] {
            assert_eq!(contain(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn content_visibility_and_reading_flow_take_their_keywords() {
        assert_eq!(
            parse_content_visibility(&tokens("AUTO")),
            Some(ContentVisibility::Auto)
        );
        assert_eq!(parse_content_visibility(&tokens("collapse")), None);
        assert_eq!(
            parse_reading_flow(&tokens("grid-order")),
            Some(ReadingFlow::GridOrder)
        );
        assert_eq!(parse_reading_flow(&tokens("flex-visual grid-rows")), None);
        assert_eq!(
            with_default_bases(|b| parse_reading_order(&tokens("-3"), b)),
            Some(-3)
        );
        assert_eq!(
            with_default_bases(|b| parse_reading_order(&tokens("1.5"), b)),
            None
        );
    }

    #[test]
    fn contain_intrinsic_size_takes_one_value_or_two() {
        use ContainIntrinsicSize::{Auto, Length, None as Nothing};
        let size = |v: &str| with_default_bases(|b| parse_contain_intrinsic_size(&tokens(v), b));
        assert_eq!(size("none"), Some([Nothing, Nothing]));
        assert_eq!(size("10px"), Some([Length(10.0), Length(10.0)]));
        assert_eq!(size("10px 20px"), Some([Length(10.0), Length(20.0)]));
        assert_eq!(
            size("auto 10px"),
            Some([Auto(Some(10.0)), Auto(Some(10.0))])
        );
        assert_eq!(size("auto none"), Some([Auto(None), Auto(None)]));
        assert_eq!(
            size("auto 10px auto 20px"),
            Some([Auto(Some(10.0)), Auto(Some(20.0))])
        );
        assert_eq!(
            size("10px auto 20px"),
            Some([Length(10.0), Auto(Some(20.0))])
        );
        assert_eq!(size("auto 1px 2px"), Some([Auto(Some(1.0)), Length(2.0)]));
        for bad in [
            "",
            "auto",
            "-1px",
            "10%",
            "1px 2px 3px",
            "auto auto 1px",
            "none auto",
        ] {
            assert_eq!(size(bad), None, "{bad:?}");
        }
        let axis = |v: &str| with_default_bases(|b| parse_contain_intrinsic_axis(&tokens(v), b));
        assert_eq!(axis("auto 3px"), Some(Auto(Some(3.0))));
        assert_eq!(axis("3px 4px"), None);
    }

    /// The document `html` cascaded with `sheet`, and its element `id`.
    fn styled(html: &str, sheet: &str) -> (dom::Document, crate::Styles) {
        let document = dom::Document::from_node(&html::parse_document(html).expect("never fails"));
        let sheets = [css::parse_stylesheet(sheet).expect("a valid sheet")];
        let styles = crate::cascade_document(&document, &sheets, &Default::default());
        (document, styles)
    }

    fn by_id(document: &dom::Document, id: &str) -> dom::NodeId {
        document
            .subtree(document.root())
            .find(|&n| document.attr(n, "id") == Some(id))
            .unwrap_or_else(|| panic!("#{id}"))
    }

    #[test]
    fn content_visibility_skips_contents() {
        let (document, styles) = styled(
            "<div id=h style='content-visibility: hidden'><p id=in>x</p></div>\
             <div id=a style='content-visibility: auto'><p id=ain>x</p></div>\
             <span id=s style='content-visibility: hidden'><b id=sb>x</b></span>\
             <div id=v><p id=vin>x</p></div>",
            "",
        );
        let style = |id: &str| styles.get(by_id(&document, id)).expect("styled");
        // `hidden` skips its contents, and the box has size, layout and
        // paint containment.
        assert!(style("h").skips_contents() && !style("in").skips_contents());
        assert!(style("h").contains_size() && style("h").contains_layout());
        assert!(style("h").contains_paint() && style("h").contains_style());
        // `auto` not yet found relevant skips too, with containment.
        assert!(style("a").skips_contents() && !style("ain").skips_contents());
        // An inline box takes no containment, so skips nothing.
        assert!(!style("s").skips_contents());
        assert!(!style("s").contains_layout());
        assert!(!style("v").skips_contents());
        assert!(!style("v").contains_paint());
    }

    #[test]
    fn a_relevant_auto_element_does_not_skip_but_keeps_its_containment() {
        // By a rule, so that the two would share a style.
        let html = "<div id=a class=c><p id=in>x</p></div>\
                    <div id=b class=c><p id=bin>x</p></div>";
        let mut document =
            dom::Document::from_node(&html::parse_document(html).expect("never fails"));
        let a = by_id(&document, "a");
        document
            .set_state(a, dom::ElementState::RELEVANT)
            .expect("an element");
        let sheets = crate::Stylesheets::new(vec![
            css::parse_stylesheet(".c { content-visibility: auto }").expect("a valid sheet"),
        ]);
        let context = crate::CascadeContext::default();
        let mut styles = sheets.cascade(&document, &context);
        let (in_a, b) = (by_id(&document, "in"), by_id(&document, "b"));
        let style = |styles: &crate::Styles, node: dom::NodeId| styles.get(node).cloned();
        let a_style = style(&styles, a).expect("styled");
        assert!(!a_style.skips_contents() && !a_style.contains_size());
        assert!(a_style.contains_layout() && a_style.contains_paint());
        // The two would share a style but for their relevance.
        assert!(style(&styles, b).expect("styled").skips_contents());
        // A change of relevance restyles the element.
        let _ = document.take_changes();
        document
            .set_state(a, dom::ElementState::NONE)
            .expect("an element");
        let changes = document.take_changes();
        sheets.restyle(&document, &context, &mut styles, &changes);
        assert!(style(&styles, a).expect("styled").skips_contents());
        assert!(!style(&styles, in_a).expect("styled").skips_contents());
    }

    #[test]
    fn the_new_properties_cascade_and_serialize() {
        let (document, styles) = styled(
            "<div id=d><p id=p>x</p></div>",
            "#d { contain: paint layout; contain-intrinsic-size: auto 10px 20px; \
             reading-flow: grid-rows; pointer-events: none } \
             #p { reading-order: 3; content-visibility: hidden }",
        );
        let value = |id: &str, name: &str| {
            crate::serialize::computed_value(styles.get(by_id(&document, id)).unwrap(), name)
                .unwrap_or_default()
        };
        assert_eq!(value("d", "contain"), "layout paint");
        assert_eq!(value("d", "contain-intrinsic-width"), "auto 10px");
        assert_eq!(value("d", "contain-intrinsic-inline-size"), "auto 10px");
        assert_eq!(value("d", "contain-intrinsic-height"), "20px");
        assert_eq!(value("d", "reading-flow"), "grid-rows");
        assert_eq!(value("d", "pointer-events"), "none");
        // Inherited, unlike the others.
        assert_eq!(value("p", "pointer-events"), "none");
        assert_eq!(value("p", "reading-flow"), "normal");
        assert_eq!(value("p", "contain"), "none");
        assert_eq!(value("p", "reading-order"), "3");
        assert_eq!(value("p", "content-visibility"), "hidden");
        assert_eq!(value("p", "contain-intrinsic-height"), "none");
        let style = styles.get(by_id(&document, "d")).unwrap();
        let resolved = crate::serialize::resolved_value(
            style,
            "contain-intrinsic-size",
            &crate::serialize::Resolver::COMPUTED,
        );
        assert_eq!(resolved.as_deref(), Some("auto 10px 20px"));
    }

    #[test]
    fn a_highlight_is_styled_by_its_own_rules_with_its_own_properties() {
        use crate::selector_matching::HighlightName;
        let html = "<div id=d><p id=p>x</p></div>";
        let document = dom::Document::from_node(&html::parse_document(html).expect("never fails"));
        let sheets = crate::Stylesheets::new(vec![
            css::parse_stylesheet(
                "#d::highlight(mark) { background-color: yellow; color: red; width: 5px; \
                 text-decoration: underline } \
                 #p::highlight(mark) { color: blue; text-decoration-line: overline } \
                 #p::highlight(other) { color: green } \
                 #p::highlight(mark):hover { color: black }",
            )
            .expect("a valid sheet"),
        ]);
        let context = crate::CascadeContext::default();
        let styles = sheets.cascade(&document, &context);
        let recascade = sheets.recascader(&document, &styles, &context);
        let mark = HighlightName::new("mark").expect("a name");
        let (d, p) = (by_id(&document, "d"), by_id(&document, "p"));
        let root = crate::cascade::highlight_root(styles.get(d).unwrap());
        let on_d = recascade.highlight(d, mark, &root).expect("styled");
        assert_eq!(on_d.color, crate::computed::Color::rgb8(255, 0, 0));
        assert_eq!(
            on_d.background_color,
            crate::computed::Color::rgb8(255, 255, 0)
        );
        assert!(on_d.decoration().line.underline);
        // `width` does not apply to a highlight.
        assert_eq!(on_d.width, crate::computed::Size::Auto);
        // The paragraph's inherits the div's, and overrides its colour.
        let on_p = recascade.highlight(p, mark, &on_d).expect("styled");
        assert_eq!(on_p.color, crate::computed::Color::rgb8(0, 0, 255));
        assert!(on_p.decoration().line.overline);
        // A highlight no rule styles has none of its own.
        let unused = HighlightName::new("unused").expect("a name");
        assert!(recascade.highlight(p, unused, &on_d).is_none());
        // With no rule anywhere, the colour stays `currentcolor`.
        let other = HighlightName::new("other").expect("a name");
        assert!(recascade.highlight(d, other, &root).is_none());
        assert!(root.color.is_current());
    }
}
