//! The top layer in the cascade (CSS Position 4 section 4, HTML
//! sections 4.11.4 and 6.12, gap batch H): what being in it does to an
//! element's computed style and its `::backdrop`'s, the inertness a modal
//! dialog and `interactivity` give (CSS UI 4 section 7.1), and the
//! properties `interactivity`, `interest-delay` with its longhands, and
//! `pointer-events`, by which a popover's `::backdrop` lets the pointer
//! through.

use css::Token;
use dom::{Document, NodeId};

use super::motion::duration;
use super::{LengthBases, single_ident, split_components};
use crate::computed::{ComputedStyle, Float, PointerEvents, Position};

/// `interactivity`: `auto` (false) or `inert` (true).
pub(super) fn parse_interactivity(value: &[Token]) -> Option<bool> {
    match single_ident(value)?.to_ascii_lowercase().as_str() {
        "auto" => Some(false),
        "inert" => Some(true),
        _ => None,
    }
}

/// `pointer-events`: its keywords, the SVG ones among them, ASCII
/// case-insensitively.
pub(super) fn parse_pointer_events(value: &[Token]) -> Option<PointerEvents> {
    use PointerEvents as P;
    Some(match single_ident(value)?.to_ascii_lowercase().as_str() {
        "auto" => P::Auto,
        "none" => P::None,
        "visiblepainted" => P::VisiblePainted,
        "visiblefill" => P::VisibleFill,
        "visiblestroke" => P::VisibleStroke,
        "visible" => P::Visible,
        "painted" => P::Painted,
        "fill" => P::Fill,
        "stroke" => P::Stroke,
        "all" => P::All,
        "bounding-box" => P::BoundingBox,
        _ => return None,
    })
}

/// `interest-delay-start` or `interest-delay-end`: `normal` (`None`) or
/// a time not below zero.
pub(super) fn parse_interest_delay(
    value: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Option<f64>> {
    if single_ident(value).is_some_and(|word| word.eq_ignore_ascii_case("normal")) {
        return Some(None);
    }
    if value.iter().any(|t| matches!(t, Token::Comma)) {
        return None;
    }
    duration(value, bases).map(Some)
}

/// `interest-delay`: its start's value, and its end's, the start's when
/// one value is given.
pub(super) fn parse_interest_delay_shorthand(
    value: &[Token],
    bases: &LengthBases<'_>,
) -> Option<[Option<f64>; 2]> {
    match split_components(value).as_slice() {
        [both] => {
            let both = parse_interest_delay(both, bases)?;
            Some([both, both])
        }
        [start, end] => Some([
            parse_interest_delay(start, bases)?,
            parse_interest_delay(end, bases)?,
        ]),
        _ => None,
    }
}

/// What the top layer and inertness make of `element`'s computed
/// style, its other values computed:
///
/// - An element in the top layer is absolutely positioned when its
///   `position` is neither `absolute` nor `fixed`, does not float, and
///   its `display` is blockified (CSS Position 4 section 4.1).
/// - The document's element is inert while a modal dialog blocks the
///   document, and with it every node but that dialog and what is in
///   it, which inherit; the topmost modal dialog is not inert by its
///   ancestors (HTML's "blocked by a modal dialog").
/// - An `inert` attribute makes `interactivity` `inert` (HTML section
///   6.3), and an element whose `interactivity` is `inert` is inert with
///   all it holds, whatever their own `interactivity` (CSS UI 4 section
///   7.1), the modal dialog's own and those in it too.
pub(super) fn adjust(document: &Document, element: NodeId, style: &mut ComputedStyle) {
    dom::hook!("style::top_layer::adjust");
    if document.in_top_layer(element) {
        style.top_layer = true;
        if !matches!(style.position, Position::Absolute | Position::Fixed) {
            style.position = Position::Absolute;
        }
        style.float = Float::None;
        style.display = style.display.blockified();
    }
    if let Some(modal) = document.topmost_modal() {
        if modal == element {
            style.inert = false;
        } else if document.document_element() == Some(element) {
            style.inert = true;
        }
    }
    if document.is_html(element) && document.attr(element, "inert").is_some() {
        style.interactivity_inert = true;
    }
    if style.interactivity_inert {
        style.inert = true;
    }
}

/// A `::backdrop`'s style, from what its rules gave it: in the top layer
/// with its element, its `position` `fixed` unless `absolute`, and
/// blockified, as an element in it is.
pub(super) fn adjust_backdrop(style: &mut ComputedStyle) {
    style.top_layer = true;
    if !matches!(style.position, Position::Absolute | Position::Fixed) {
        style.position = Position::Fixed;
    }
    style.float = Float::None;
    style.display = style.display.blockified();
}

#[cfg(test)]
mod tests {
    use dom::{Dialog, Document, ElementState, NodeId, Popover, PopoverList, PopoverMode};

    use super::super::{CascadeContext, Stylesheets, cascade_document};
    use crate::computed::{Display, PointerEvents, Position};
    use crate::selector_matching::PseudoElement;
    use crate::serialize::computed_value;
    use crate::{ComputedStyle, Styles};

    /// The document `html` makes, `id` its element of that id.
    fn page(html: &str) -> Document {
        Document::from_node(&html::parse_document(html).expect("never fails"))
    }

    fn by_id(document: &Document, id: &str) -> NodeId {
        document
            .subtree(document.root())
            .find(|&n| document.attr(n, "id") == Some(id))
            .unwrap_or_else(|| panic!("no #{id}"))
    }

    fn styles(document: &Document, css: &str) -> Styles {
        let sheets = if css.is_empty() {
            Vec::new()
        } else {
            vec![css::parse_stylesheet(css).expect("valid")]
        };
        cascade_document(document, &sheets, &CascadeContext::default())
    }

    fn of<'s>(styles: &'s Styles, document: &Document, id: &str) -> &'s ComputedStyle {
        styles.get(by_id(document, id)).expect("styled")
    }

    fn modal(document: &mut Document, id: &str) {
        let dialog = by_id(document, id);
        document.set_attribute(dialog, "open", "").unwrap();
        document.open_dialog_state(
            dialog,
            Dialog {
                modal: true,
                previously_focused: None,
            },
        );
    }

    fn show(document: &mut Document, id: &str) {
        let element = by_id(document, id);
        document.show_popover_state(
            element,
            Popover {
                mode: PopoverMode::Auto,
                list: PopoverList::Auto,
                invoker: None,
                previously_focused: None,
                anchor: None,
            },
        );
    }

    const PAGE: &str = "<body><p id=p>text</p>\
        <dialog id=d><button id=b>b</button></dialog>\
        <div id=pop popover>pop</div>\
        <span id=sp popover style='position: relative'>span</span>\
        <div id=inert inert><i id=i style='interactivity: auto'>i</i></div>\
        <div id=plain><i>i</i></div>";

    #[test]
    fn a_dialog_is_shown_by_its_open_attribute_and_modal_one_is_fixed() {
        let mut d = page(PAGE);
        let s = styles(&d, "");
        assert_eq!(of(&s, &d, "d").display, Display::None);
        assert_eq!(of(&s, &d, "d").position, Position::Absolute);
        let dialog = by_id(&d, "d");
        d.set_attribute(dialog, "open", "").unwrap();
        let s = styles(&d, "");
        assert_eq!(of(&s, &d, "d").display, Display::Block);
        assert_eq!(of(&s, &d, "d").position, Position::Absolute);
        assert!(!of(&s, &d, "d").top_layer);
        // Shown, it has no backdrop: it is not in the top layer.
        assert!(s.pseudo(dialog, PseudoElement::Backdrop).is_none());
        // One a page styles is computed all the same, for scripts.
        let styled = styles(&d, "#d::backdrop { color: rgb(5, 5, 5) }");
        assert_eq!(
            styled
                .pseudo(dialog, PseudoElement::Backdrop)
                .and_then(|b| computed_value(b, "color"))
                .as_deref(),
            Some("rgb(5, 5, 5)")
        );
        d.remove_attribute(dialog, "open").unwrap();
        modal(&mut d, "d");
        let s = styles(&d, "");
        let style = of(&s, &d, "d");
        assert_eq!(style.position, Position::Fixed);
        assert!(style.top_layer);
        assert_eq!(
            computed_value(style, "max-width").as_deref(),
            Some("calc(100% - 38px)")
        );
        let backdrop = s
            .pseudo(dialog, PseudoElement::Backdrop)
            .expect("a backdrop");
        assert_eq!(backdrop.position, Position::Fixed);
        assert!(backdrop.top_layer);
        assert_eq!(
            computed_value(backdrop, "background-color").as_deref(),
            Some("rgba(0, 0, 0, 0.1)")
        );
        assert_eq!(computed_value(backdrop, "top").as_deref(), Some("0px"));
    }

    #[test]
    fn a_modal_dialog_makes_all_but_itself_inert() {
        let mut d = page(PAGE);
        let s = styles(&d, "");
        assert!(!of(&s, &d, "p").inert && !of(&s, &d, "b").inert);
        modal(&mut d, "d");
        show(&mut d, "pop");
        let s = styles(&d, "");
        assert!(of(&s, &d, "p").inert);
        // A popover above the dialog is blocked by it too.
        assert!(of(&s, &d, "pop").inert);
        assert!(!of(&s, &d, "d").inert && !of(&s, &d, "b").inert);
        // Its `interactivity` makes it inert whatever.
        let s = styles(&d, "#d { interactivity: inert }");
        assert!(of(&s, &d, "d").inert && of(&s, &d, "b").inert);
    }

    #[test]
    fn interactivity_inert_and_the_inert_attribute_make_a_subtree_inert() {
        let d = page(PAGE);
        let s = styles(
            &d,
            "#p { interactivity: inert } #inert { interactivity: auto }",
        );
        assert!(of(&s, &d, "p").inert);
        assert_eq!(
            computed_value(of(&s, &d, "p"), "interactivity").as_deref(),
            Some("inert")
        );
        // The attribute wins over the page's `auto`.
        assert!(of(&s, &d, "inert").interactivity_inert);
        // What is inside is inert, its own `auto` notwithstanding.
        let i = of(&s, &d, "i");
        assert!(i.inert && !i.interactivity_inert);
        assert_eq!(computed_value(i, "interactivity").as_deref(), Some("auto"));
        assert!(!of(&s, &d, "d").inert);
        // A sibling of the same declarations is not, whatever the styles
        // the cascade shares.
        let s = styles(&d, "");
        assert!(of(&s, &d, "inert").inert);
        assert!(!of(&s, &d, "plain").inert && !of(&s, &d, "plain").interactivity_inert);
    }

    #[test]
    fn a_popover_shown_is_blockified_and_out_of_flow_in_the_top_layer() {
        let mut d = page(PAGE);
        let s = styles(&d, "");
        assert_eq!(of(&s, &d, "pop").display, Display::None);
        assert_eq!(of(&s, &d, "pop").position, Position::Fixed);
        show(&mut d, "pop");
        show(&mut d, "sp");
        let s = styles(&d, "");
        assert_eq!(of(&s, &d, "pop").display, Display::Block);
        assert!(of(&s, &d, "pop").top_layer);
        assert!(of(&s, &d, "pop").is_stacking_context(false));
        // `inset: 0`, the four sides as one.
        for side in ["top", "right", "bottom", "left"] {
            assert_eq!(
                computed_value(of(&s, &d, "pop"), side).as_deref(),
                Some("0px")
            );
        }
        let s2 = styles(&d, "#p { position: absolute; inset: 1px 2px 3% }");
        assert_eq!(
            computed_value(of(&s2, &d, "p"), "inset").as_deref(),
            Some("1px 2px 3%")
        );
        // A relative position is absolute in the top layer, and an inline
        // box blockified.
        let sp = of(&s, &d, "sp");
        assert_eq!(sp.position, Position::Absolute);
        assert_eq!(sp.display, Display::Block);
        let backdrop = s
            .pseudo(by_id(&d, "pop"), PseudoElement::Backdrop)
            .expect("a backdrop");
        assert_eq!(
            computed_value(backdrop, "background-color").as_deref(),
            Some("rgba(0, 0, 0, 0)")
        );
        // It inherits from its element, and lets the pointer through
        // whatever a page says.
        let s = styles(
            &d,
            "#pop { color: rgb(1, 2, 3) } ::backdrop { pointer-events: auto }",
        );
        let backdrop = s.pseudo(by_id(&d, "pop"), PseudoElement::Backdrop).unwrap();
        assert_eq!(
            computed_value(backdrop, "color").as_deref(),
            Some("rgb(1, 2, 3)")
        );
        assert_eq!(backdrop.pointer_events, PointerEvents::None);
        assert_eq!(
            computed_value(backdrop, "pointer-events").as_deref(),
            Some("none")
        );
        let s = styles(&d, "#p { pointer-events: VisiblePainted }");
        assert_eq!(
            computed_value(of(&s, &d, "p"), "pointer-events").as_deref(),
            Some("visiblePainted")
        );
        assert_eq!(of(&s, &d, "pop").pointer_events, PointerEvents::Auto);
    }

    #[test]
    fn the_pseudo_classes_read_the_states() {
        let mut d = page(PAGE);
        let css = "dialog:modal { color: rgb(1, 0, 0) } \
            :popover-open { color: rgb(2, 0, 0) } #pop:modal { color: rgb(9, 9, 9) } \
            dialog:open { outline-style: solid } \
            #b:interest-source { color: rgb(3, 0, 0) } \
            #p:interest-target { color: rgb(4, 0, 0) }";
        let dialog = by_id(&d, "d");
        d.set_attribute(dialog, "open", "").unwrap();
        let s = styles(&d, css);
        assert_eq!(
            computed_value(of(&s, &d, "d"), "outline-style").as_deref(),
            Some("solid")
        );
        assert_ne!(
            computed_value(of(&s, &d, "d"), "color").as_deref(),
            Some("rgb(1, 0, 0)")
        );
        d.remove_attribute(dialog, "open").unwrap();
        modal(&mut d, "d");
        show(&mut d, "pop");
        let b = by_id(&d, "b");
        let p = by_id(&d, "p");
        d.set_state(b, d.state(b).with(ElementState::INTEREST_SOURCE, true))
            .unwrap();
        d.set_state(p, d.state(p).with(ElementState::INTEREST_TARGET, true))
            .unwrap();
        let s = styles(&d, css);
        let color = |id| computed_value(of(&s, &d, id), "color");
        assert_eq!(color("d").as_deref(), Some("rgb(1, 0, 0)"));
        assert_eq!(color("pop").as_deref(), Some("rgb(2, 0, 0)"));
        assert_eq!(color("b").as_deref(), Some("rgb(3, 0, 0)"));
        assert_eq!(color("p").as_deref(), Some("rgb(4, 0, 0)"));
    }

    #[test]
    fn a_restyle_follows_the_top_layer_and_the_modal_flag() {
        let mut d = page(PAGE);
        let sheets = Stylesheets::new(Vec::new());
        let context = CascadeContext::default();
        let mut s = sheets.cascade(&d, &context);
        d.take_changes();
        modal(&mut d, "d");
        let changes = d.take_changes();
        sheets.restyle(&d, &context, &mut s, &changes);
        assert!(of(&s, &d, "p").inert, "every node's inertness");
        assert_eq!(of(&s, &d, "d").position, Position::Fixed);
        show(&mut d, "sp");
        let changes = d.take_changes();
        sheets.restyle(&d, &context, &mut s, &changes);
        assert_eq!(of(&s, &d, "sp").position, Position::Absolute);
        assert!(s.pseudo(by_id(&d, "sp"), PseudoElement::Backdrop).is_some());
        let sp = by_id(&d, "sp");
        d.hide_popover_state(sp);
        let dialog = by_id(&d, "d");
        d.close_dialog_state(dialog);
        let changes = d.take_changes();
        sheets.restyle(&d, &context, &mut s, &changes);
        assert_eq!(of(&s, &d, "sp").display, Display::None);
        assert!(!of(&s, &d, "p").inert);
        assert_eq!(of(&s, &d, "d").position, Position::Absolute);
        assert!(s.pseudo(dialog, PseudoElement::Backdrop).is_none());
    }

    #[test]
    fn the_interest_delays_parse_and_serialize() {
        let d = page("<div id=a></div><div id=b></div><div id=c><i id=i></i></div>");
        let s = styles(
            &d,
            "#a { interest-delay: 1s 200ms } #b { interest-delay: 3s; interest-delay-end: -1s } \
             #c { interest-delay-start: 0.2s; interest-delay-end: normal; interest-delay: 1s, 2s }",
        );
        let value = |id, name| computed_value(of(&s, &d, id), name);
        assert_eq!(value("a", "interest-delay-start").as_deref(), Some("1s"));
        assert_eq!(value("a", "interest-delay-end").as_deref(), Some("0.2s"));
        assert_eq!(value("a", "interest-delay").as_deref(), Some("1s 0.2s"));
        // One value is both; a negative time is no value.
        assert_eq!(value("b", "interest-delay").as_deref(), Some("3s"));
        assert_eq!(value("c", "interest-delay-start").as_deref(), Some("0.2s"));
        assert_eq!(value("c", "interest-delay-end").as_deref(), Some("normal"));
        // Not inherited; `normal` is the reference browser's delays.
        let i = of(&s, &d, "i");
        assert_eq!(
            computed_value(i, "interest-delay").as_deref(),
            Some("normal")
        );
        assert_eq!(i.interest_delay(false), 0.5);
        assert_eq!(i.interest_delay(true), 0.25);
        assert_eq!(of(&s, &d, "a").interest_delay(true), 0.2);
        assert!(crate::supports("interest-delay", &css::tokenize("1s 2s")));
        assert!(!crate::supports(
            "interest-delay-start",
            &css::tokenize("-1s")
        ));
        assert!(crate::supports("interactivity", &css::tokenize("inert")));
        assert!(!crate::supports("interactivity", &css::tokenize("none")));
    }
}
