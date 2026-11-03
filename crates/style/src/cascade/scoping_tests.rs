//! The cascade over shadow trees (gap batch H, D69): each tree's rules
//! its own, `:host`, `::slotted()` and `::part()` across the boundaries,
//! the contexts' order, inheritance through the flat tree, and the
//! restyles that reach them.

use std::sync::Arc;

use dom::{Document, Namespace, NodeId, ShadowRootInit};

use super::{CascadeContext, Styles, Stylesheets};
use crate::computed::{Color, Display};

/// A page parsed from `page`, with a shadow tree of `markup` attached
/// to its element `#host`; its root.
fn page_with_shadow(page: &str, markup: &str) -> (Document, NodeId) {
    let mut document = Document::from_node(&html::parse_document(page).expect("parses"));
    let host = by_id(&document, document.root(), "host");
    let root = document
        .attach_shadow(host, ShadowRootInit::open(), false)
        .expect("a div hosts it");
    fill(&mut document, root, host, markup);
    (document, root)
}

/// Parses `markup` into `parent`, in the context of `context`.
fn fill(document: &mut Document, parent: NodeId, context: NodeId, markup: &str) {
    let container = document.create_element("html").unwrap();
    let name = document.local_name(context).unwrap_or("div").to_owned();
    html::parse_fragment(
        document,
        container,
        (&Namespace::Html, &name),
        false,
        dom::MAX_DEPTH,
        markup,
    );
    while let Some(child) = document.first_child(container) {
        document.append_child(parent, child).unwrap();
    }
    document.free(container).unwrap();
}

/// The element with ID `id` in the tree at `top`.
fn by_id(document: &Document, top: NodeId, id: &str) -> NodeId {
    document
        .subtree(top)
        .find(|&n| document.attr(n, "id") == Some(id))
        .unwrap_or_else(|| panic!("#{id}"))
}

fn sheet(src: &str) -> css::Stylesheet {
    css::parse_stylesheet(src).expect("never fails")
}

/// The styles of `document` with the page's sheet `css` and the shadow
/// trees' sheets `trees`.
fn cascade(document: &Document, css: &str, trees: &[(NodeId, &str)]) -> Styles {
    sheets(css, trees).cascade(document, &CascadeContext::default())
}

fn sheets(css: &str, trees: &[(NodeId, &str)]) -> Stylesheets {
    Stylesheets::new(vec![sheet(css)]).with_shadow_trees(
        trees
            .iter()
            .map(|&(root, src)| (root, vec![sheet(src)], vec![None]))
            .collect(),
    )
}

fn color(styles: &Styles, node: NodeId) -> Color {
    styles.get(node).expect("styled").color
}

const RED: Color = Color::rgb8(255, 0, 0);
const GREEN: Color = Color::rgb8(0, 128, 0);
const BLUE: Color = Color::rgb8(0, 0, 255);

#[test]
fn a_trees_rules_style_its_own_elements_alone() {
    let (document, root) =
        page_with_shadow("<p id=outer>o</p><div id=host></div>", "<p id=inner>i</p>");
    let styles = cascade(
        &document,
        "p { color: red }",
        &[(root, "p { color: green }")],
    );
    let outer = by_id(&document, document.root(), "outer");
    let inner = by_id(&document, root, "inner");
    assert_eq!(color(&styles, outer), RED);
    assert_eq!(color(&styles, inner), GREEN);
    // Without a sheet of its own, the tree's elements take none of the
    // page's rules, and inherit from the host.
    let styles = cascade(&document, "p { color: red } #host { color: blue }", &[]);
    assert_eq!(color(&styles, inner), BLUE);
}

#[test]
fn host_rules_match_the_featureless_host_and_lose_to_the_outer_trees() {
    let (document, root) = page_with_shadow(
        "<div id=host class=on><span id=light>l</span></div>",
        "<p id=inner>i</p>",
    );
    let host = by_id(&document, document.root(), "host");
    let inner = by_id(&document, root, "inner");
    let tree = ":host { color: green } :host(.off) { color: red } \
                :host(.on) p { color: blue } div { color: red }";
    let styles = cascade(&document, "", &[(root, tree)]);
    assert_eq!(color(&styles, host), GREEN, "`div` cannot match the host");
    assert_eq!(color(&styles, inner), BLUE);
    // The outer tree's normal declarations win over `:host`, whatever
    // the specificity; the inner tree's important ones win.
    let styles = cascade(
        &document,
        "div { color: red }",
        &[(root, ":host(#host) { color: green }")],
    );
    assert_eq!(color(&styles, host), RED);
    let styles = cascade(
        &document,
        "#host { color: red !important }",
        &[(root, ":host { color: green !important }")],
    );
    assert_eq!(color(&styles, host), GREEN);
    // The page's own `:host` matches nothing.
    let styles = cascade(&document, ":host { color: red }", &[]);
    assert_ne!(color(&styles, host), RED);
}

#[test]
fn host_context_matches_by_the_hosts_shadow_including_ancestors() {
    let (document, root) = page_with_shadow(
        "<section class=dark><div id=host></div></section>",
        "<p id=inner>i</p>",
    );
    let inner = by_id(&document, root, "inner");
    let styles = cascade(
        &document,
        "",
        &[(
            root,
            ":host-context(.dark) p { color: green } :host-context(.light) p { color: red }",
        )],
    );
    assert_eq!(color(&styles, inner), GREEN);
}

#[test]
fn slotted_rules_style_the_assigned_elements_and_lose_to_their_own_trees() {
    let (document, root) = page_with_shadow(
        "<div id=host><span id=a>a</span><b id=b slot=x>b</b></div>",
        "<div id=wrap><slot id=s></slot><slot name=x></slot></div>",
    );
    let a = by_id(&document, document.root(), "a");
    let b = by_id(&document, document.root(), "b");
    let tree = "::slotted(span) { color: green } slot[name=x]::slotted(*) { color: blue } \
                span { color: red }";
    let styles = cascade(&document, "", &[(root, tree)]);
    assert_eq!(color(&styles, a), GREEN);
    assert_eq!(color(&styles, b), BLUE);
    // The page's rule for its own element wins over `::slotted()`.
    let styles = cascade(
        &document,
        "#a { color: red }",
        &[(root, "::slotted(span) { color: green }")],
    );
    assert_eq!(color(&styles, a), RED);
    // A slot makes no box: `display: contents`, by the user agent.
    let slot = by_id(&document, root, "s");
    assert_eq!(styles.get(slot).unwrap().display, Display::Contents);
}

#[test]
fn slotted_elements_inherit_through_the_flat_tree() {
    let (document, root) = page_with_shadow(
        "<div id=host style='color: red'><span id=a>a</span><i id=unslotted slot=none>u</i></div>",
        "<div style='color: green'><slot></slot></div>",
    );
    let a = by_id(&document, document.root(), "a");
    let styles = cascade(&document, "", &[(root, "")]);
    assert_eq!(
        color(&styles, a),
        GREEN,
        "from the slot's parent, not the host"
    );
    // What no slot takes is in no flat tree: not styled.
    let unslotted = by_id(&document, document.root(), "unslotted");
    assert!(styles.get(unslotted).is_none());
}

#[test]
fn part_rules_reach_through_exportparts() {
    let (mut document, root) = page_with_shadow(
        "<div id=host></div>",
        "<p id=label part='label extra'>x</p><div id=inner-host exportparts='thing: inner, label'></div>",
    );
    let inner_host = by_id(&document, root, "inner-host");
    let inner_root = document
        .attach_shadow(inner_host, ShadowRootInit::open(), false)
        .unwrap();
    fill(
        &mut document,
        inner_root,
        inner_host,
        "<span id=thing part=thing>t</span>",
    );
    let label = by_id(&document, root, "label");
    let thing = by_id(&document, inner_root, "thing");
    let page = "#host::part(label) { color: green } #host::part(label missing) { color: red } \
                #host::part(inner) { color: blue } #host::part(thing) { color: red }";
    let styles = cascade(&document, page, &[(root, "p { color: red }")]);
    // The outer tree's `::part()` wins over the inner tree's own rule.
    assert_eq!(color(&styles, label), GREEN);
    assert_eq!(color(&styles, thing), BLUE, "exported as `inner`");
    // A tree's own `::part()` of its inner host, without the export.
    let styles = cascade(
        &document,
        "",
        &[(root, "#inner-host::part(thing) { color: green }")],
    );
    assert_eq!(color(&styles, thing), GREEN);
}

#[test]
fn nested_part_and_child_rules_reach_the_shadow_tree_and_the_slotted() {
    let (document, root) = page_with_shadow(
        "<x-box id=host><div id=content class=content>c</div></x-box>",
        "<div id=box part=box><slot></slot></div>",
    );
    let boxed = by_id(&document, root, "box");
    let content = by_id(&document, document.root(), "content");
    let page = "x-box { display: block; &::part(box) { color: green } > .content { color: blue } }";
    let styles = cascade(&document, page, &[(root, "")]);
    assert_eq!(color(&styles, boxed), GREEN);
    assert_eq!(color(&styles, content), BLUE);
    // The same when no shadow tree has a sheet of its own.
    let styles = cascade(&document, page, &[]);
    assert_eq!(color(&styles, boxed), GREEN);
}

#[test]
fn trees_of_the_same_sheets_share_their_rules() {
    let (mut document, first) =
        page_with_shadow("<div id=host></div><div id=other></div>", "<p>x</p>");
    let other = by_id(&document, document.root(), "other");
    let second = document
        .attach_shadow(other, ShadowRootInit::open(), false)
        .unwrap();
    let set = sheets(
        "",
        &[(first, "p { color: red }"), (second, "p { color: red }")],
    );
    assert!(Arc::ptr_eq(&set.shadow[&first], &set.shadow[&second]));
    assert!(set.same_shadow_trees(&[
        (first, vec![sheet("p { color: red }")], vec![None]),
        (second, vec![sheet("p { color: red }")], vec![None]),
    ]));
    assert!(!set.same_shadow_trees(&[(first, vec![sheet("p { color: red }")], vec![None])]));
}

#[test]
fn a_restyle_reaches_the_shadow_trees_and_the_slots() {
    let (mut document, root) = page_with_shadow(
        "<div id=host><span id=a>a</span></div>",
        "<p id=inner>i</p><slot name=x></slot>",
    );
    let host = by_id(&document, document.root(), "host");
    let inner = by_id(&document, root, "inner");
    let a = by_id(&document, document.root(), "a");
    let set = sheets(
        "",
        &[(
            root,
            ":host(.on) p { color: green } ::slotted(span) { color: blue }",
        )],
    );
    let context = CascadeContext::default();
    let mut styles = set.cascade(&document, &context);
    assert_ne!(color(&styles, inner), GREEN);
    assert!(styles.get(a).is_none(), "no slot takes it yet");
    document.take_changes();
    document.set_attribute(host, "class", "on").unwrap();
    document.set_attribute(a, "slot", "x").unwrap();
    let changes = document.take_changes();
    set.restyle(&document, &context, &mut styles, &changes);
    assert_eq!(color(&styles, inner), GREEN);
    assert_eq!(color(&styles, a), BLUE);
    assert_eq!(styles.get(a), set.cascade(&document, &context).get(a));
}

#[test]
fn keyframes_names_are_looked_up_in_the_tree_that_gave_them_then_outward() {
    // CSS Scoping 1 section 3.5: `animation-name` names the keyframes of
    // the tree its declaration is in, else of the trees around it; a
    // tree's keyframes are not the document's. `:host`'s declaration is
    // the shadow tree's, though the host is in the document.
    let (document, root) = page_with_shadow(
        "<p id=doc_inner class=anim>d</p><div id=host>h</div>",
        "<p id=inner_inner class=anim>i</p><p id=inner_doc class=anim>j</p>",
    );
    let page = "@keyframes doc { from, to { color: blue } } \
                .anim { color: red; animation-duration: 10s; animation-fill-mode: both } \
                #doc_inner { animation-name: inner }";
    let tree = "@keyframes inner { from, to { color: green } } \
                .anim { color: red; animation-duration: 10s; animation-fill-mode: both } \
                #inner_inner { animation-name: inner } #inner_doc { animation-name: doc } \
                :host { animation: inner 10s both }";
    let set = sheets(page, &[(root, tree)]);
    let animations = std::cell::RefCell::new(crate::animation::Animations::new());
    let context = CascadeContext {
        animations: Some(&animations),
        ..CascadeContext::default()
    };
    let mut styles = set.cascade(&document, &context);
    let moved = {
        let mut animations = animations.borrow_mut();
        animations.start_pending();
        animations.set_time(1000.0);
        animations.sample()
    };
    set.restyle_animated(&document, &context, &mut styles, &[], &moved);
    let at = |id: &str| {
        let top = match by_id_in(&document, document.root(), id) {
            Some(node) => node,
            None => by_id(&document, root, id),
        };
        color(&styles, top)
    };
    assert_eq!(at("inner_inner"), GREEN, "the tree's own keyframes");
    assert_eq!(at("inner_doc"), BLUE, "the document's, from inside");
    assert_eq!(at("doc_inner"), RED, "not a shadow tree's, from outside");
    assert_eq!(at("host"), GREEN, "`:host`'s, the shadow tree's");
}

/// The element with ID `id` in the tree at `top`, if there is one.
fn by_id_in(document: &Document, top: NodeId, id: &str) -> Option<NodeId> {
    document
        .subtree(top)
        .find(|&n| document.attr(n, "id") == Some(id))
}

#[test]
fn host_pseudo_elements_take_the_shadow_trees_rules() {
    let (document, root) = page_with_shadow("<div id=host></div>", "<p>i</p>");
    let host = by_id(&document, document.root(), "host");
    let tree = ":host::before { content: 'x'; color: green }";
    let styles = cascade(&document, "", &[(root, tree)]);
    let before = styles
        .pseudo(host, crate::selector_matching::PseudoElement::Before)
        .expect("generated");
    assert_eq!(before.color, GREEN);
}

#[test]
fn a_trees_scope_rules_reach_up_to_its_featureless_host() {
    // css-cascade-6 and CSS Scoping 1 (D69): `@scope (:host)` roots at
    // the host, which `:scope` then matches; an implicit `@scope` of a
    // sheet with no element around it roots there too.
    let (document, root) = page_with_shadow(
        "<div id=host></div><p class=a id=outside>o</p>",
        "<p class=a id=a>a</p>",
    );
    let host = by_id(&document, document.root(), "host");
    let a = by_id(&document, root, "a");
    let outside = by_id(&document, document.root(), "outside");
    let tree = "@scope (:host) { :scope { color: green } .a { color: blue } }";
    let styles = cascade(&document, "", &[(root, tree)]);
    assert_eq!(color(&styles, host), GREEN);
    assert_eq!(color(&styles, a), BLUE);
    assert_ne!(color(&styles, outside), BLUE);
    let tree = "@scope { :scope { color: blue } :scope > .a { color: green } }";
    let styles = cascade(&document, "", &[(root, tree)]);
    assert_eq!(color(&styles, host), BLUE);
    assert_eq!(color(&styles, a), GREEN);
}

#[test]
fn a_part_and_a_slotted_element_have_their_pseudo_elements_styled() {
    // CSS Shadow Parts 1 section 4.2, CSS Scoping 1 section 3.2.2:
    // `::part()` and `::slotted()` may be followed by `::before` and
    // `::after`, which style that element's pseudo-elements, not it.
    let (document, root) = page_with_shadow(
        "<div id=host><span id=light>l</span></div>",
        "<p id=label part=label>x</p><slot></slot>",
    );
    let label = by_id(&document, root, "label");
    let light = by_id(&document, document.root(), "light");
    let page = "#host::part(label)::before { content: 'p'; color: green } \
                #host::part(label) { color: blue }";
    let tree = "::slotted(span)::after { content: 's'; color: green }";
    let styles = cascade(&document, page, &[(root, tree)]);
    assert_eq!(
        color(&styles, label),
        BLUE,
        "the part itself by its own rule"
    );
    use crate::selector_matching::PseudoElement;
    let pseudo = |node, which| styles.pseudo(node, which).expect("styled").color;
    assert_eq!(pseudo(label, PseudoElement::Before), GREEN);
    assert_eq!(pseudo(light, PseudoElement::After), GREEN);
    assert_ne!(color(&styles, light), GREEN, "not the slotted element");
}

#[test]
fn a_hosts_pseudo_element_queries_the_host_as_its_style_container() {
    // `@container style()` of `:host::before` from the shadow tree's
    // rules: the host, the originating element, is the query container,
    // its style computed before its pseudo-elements' are (D72).
    let (document, root) = page_with_shadow("<div id=host></div>", "<p>i</p>");
    let host = by_id(&document, document.root(), "host");
    let tree = "@container style(--on: 1) { :host::before { content: 'x'; color: green } }";
    let styles = cascade(&document, "#host { --on: 1 }", &[(root, tree)]);
    let before = styles
        .pseudo(host, crate::selector_matching::PseudoElement::Before)
        .expect("styled");
    assert_eq!(before.color, GREEN);
    let styles = cascade(&document, "#host { --on: 2 }", &[(root, tree)]);
    assert!(
        styles
            .pseudo(host, crate::selector_matching::PseudoElement::Before)
            .is_none(),
        "the query does not hold"
    );
}

#[test]
fn host_part_matches_the_trees_own_parts() {
    // `:host::part(x)` in a shadow tree reaches that tree's own elements
    // with the part, through its host.
    let (document, root) = page_with_shadow(
        "<div id=host class=on></div>",
        "<p id=label part=label>x</p><p id=other>y</p>",
    );
    let label = by_id(&document, root, "label");
    let other = by_id(&document, root, "other");
    let tree = ":host(.on)::part(label) { color: green } :host(.off)::part(label) { color: red }";
    let styles = cascade(&document, "", &[(root, tree)]);
    assert_eq!(color(&styles, label), GREEN);
    assert_ne!(color(&styles, other), GREEN);
}

#[test]
fn defined_and_state_match_what_the_document_keeps() {
    // `:defined` and `:state()` (D70) read the custom element state and
    // the custom states the document keeps.
    let (mut document, root) = page_with_shadow(
        "<div id=host></div><x-a id=waiting></x-a><x-b id=custom></x-b>",
        "<p>i</p>",
    );
    let _ = root;
    let waiting = by_id(&document, document.root(), "waiting");
    let custom = by_id(&document, document.root(), "custom");
    document
        .set_custom_element_state(custom, dom::CustomElementState::Custom, None)
        .unwrap();
    document.set_custom_state(custom, "on", true).unwrap();
    let page = ":defined { color: green } :not(:defined) { color: red } \
                :state(on) { color: blue } :state(ON) { color: red }";
    let set = sheets(page, &[]);
    let context = CascadeContext::default();
    let mut styles = set.cascade(&document, &context);
    assert_eq!(color(&styles, waiting), RED);
    assert_eq!(color(&styles, custom), BLUE, "case-sensitive");
    // Two changes of its states between restyles, and its being defined,
    // restyle them.
    document.take_changes();
    document.set_custom_state(custom, "on", false).unwrap();
    document.set_custom_state(custom, "other", true).unwrap();
    document
        .set_custom_element_state(waiting, dom::CustomElementState::Custom, None)
        .unwrap();
    let changes = document.take_changes();
    set.restyle(&document, &context, &mut styles, &changes);
    assert_eq!(color(&styles, custom), GREEN);
    assert_eq!(color(&styles, waiting), GREEN);
}
