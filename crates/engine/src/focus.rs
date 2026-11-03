//! Focus (M26.6, `docs/design/events.md`, HTML section 6.6): which
//! elements can take the focus, and in what order Tab moves it, for the
//! engine and its scripts' host alike. Shadow trees (gap batch H, D68)
//! are focus navigation scopes of their own, as their slots are, and a
//! host that delegates focus gives it to the first area in its tree. A
//! scroll container that can scroll takes the focus too, and Tab reaches
//! one with no focusable area in it, as the reference browser's keyboard
//! focusable scrollers do; and a container's `reading-flow` (CSS Display
//! 4 section 5) makes it and its items scopes too, its own ordered as
//! its layout places them.

use std::collections::{HashMap, HashSet};

use dom::{Document, NodeId, forms};
use layout::{DocumentLayout, FragmentKind};
use style::Styles;
use style::computed::{ContentVisibility, Display, FlexDirection, Overflow, ReadingFlow};

/// What the focus rules read of a layout: each element's first box, its
/// border box on the page, and the scroll containers that can scroll.
#[derive(Debug, Default)]
pub(crate) struct Laid {
    boxes: HashMap<NodeId, [f32; 4]>,
    scrollers: HashSet<NodeId>,
}

impl Laid {
    /// What `layout` says.
    pub(crate) fn of(layout: &DocumentLayout) -> Laid {
        let mut laid = Laid::default();
        for placed in layout.placed() {
            let f = placed.fragment;
            if !matches!(f.kind, FragmentKind::Box) || f.generated.is_some() {
                continue;
            }
            laid.boxes
                .entry(f.node)
                .or_insert([placed.x, placed.top(), f.w, f.h]);
            // A user can scroll it: its `overflow` in an axis it overflows
            // is `auto` or `scroll`.
            let user = |o: Overflow| matches!(o, Overflow::Auto | Overflow::Scroll);
            let scrolls = (user(f.style.overflow_y) && f.max_scroll() > 0.0)
                || (user(f.style.overflow_x) && crate::geometry::max_scroll_x(f) > 0.0);
            if f.clips() && scrolls && !f.style.root {
                laid.scrollers.insert(f.node);
            }
        }
        laid
    }
}

/// The `tabindex` an element's attribute gives it, if it parses (HTML's
/// rules for parsing integers).
pub(crate) fn tab_index(document: &Document, element: NodeId) -> Option<i32> {
    document
        .attr(element, "tabindex")
        .and_then(dom::parse_integer)
        .and_then(|n| i32::try_from(n).ok())
}

/// Whether the element can take the focus as it is, with no `tabindex`
/// (HTML section 6.6.3): a link, a control that is not disabled, an
/// `input` that is not hidden.
pub(crate) fn natively_focusable(document: &Document, element: NodeId) -> bool {
    match document.tag(element) {
        Some("a" | "area") => document.attr(element, "href").is_some(),
        Some("input") => {
            !document
                .attr(element, "type")
                .is_some_and(|kind| kind.eq_ignore_ascii_case("hidden"))
                && !forms::actually_disabled(document, element)
        }
        Some("button" | "select" | "textarea") => !forms::actually_disabled(document, element),
        // A frame's element, the focus passing into its document (M33.2).
        Some("iframe") => true,
        // A media element with controls, whose keys play, pause and skip
        // (M40), as the reference's.
        Some("video" | "audio") => document.attr(element, "controls").is_some(),
        // The summary of its `details`, which it opens and closes (gap
        // batch H).
        Some("summary") => document.summary_details(element).is_some(),
        // An editing host (HTML section 6.8.1).
        _ => editing_host(document, element),
    }
}

/// Whether `element` is an editing host: `contenteditable` true or
/// plaintext-only, its parent not editable.
fn editing_host(document: &Document, element: NodeId) -> bool {
    let editable = |e: NodeId| {
        document.attr(e, "contenteditable").is_some_and(|v| {
            v.is_empty()
                || v.eq_ignore_ascii_case("true")
                || v.eq_ignore_ascii_case("plaintext-only")
        })
    };
    editable(element)
        && !document
            .parent(element)
            .filter(|&p| document.is_element(p))
            .is_some_and(editable)
}

/// Whether `element` is in the content of a `details` element that is
/// not open, its summary aside, which is not rendered (HTML section
/// 15.5.1).
fn in_closed_details(document: &Document, element: NodeId) -> bool {
    let mut at = element;
    while let Some(parent) = document.parent(at) {
        if document.tag(parent) == Some("details")
            && document.attr(parent, "open").is_none()
            && document.summary_details(at) != Some(parent)
        {
            return true;
        }
        at = parent;
    }
    false
}

/// Whether the element is a focusable area (HTML section 6.6.2): one
/// that can take the focus as it is, or has a `tabindex`, and is being
/// rendered, by `styles`, which say it is when they are not given; a
/// disabled control does not take it whatever its `tabindex`, nor does
/// an inert element (gap batch H). An open `dialog` takes it too, as the
/// reference browser's does, though neither Tab nor a click gives it
/// ([`open_dialog_alone`]).
pub(crate) fn focusable(document: &Document, styles: Option<&Styles>, element: NodeId) -> bool {
    if !document.is_element(element) || !rendered(document, styles, element) {
        return false;
    }
    if styles
        .and_then(|styles| styles.get(element))
        .is_some_and(|style| style.inert)
    {
        return false;
    }
    if forms::can_be_disabled(document, element) && forms::actually_disabled(document, element) {
        return false;
    }
    natively_focusable(document, element)
        || tab_index(document, element).is_some()
        || is_open_dialog(document, element)
}

/// Whether the element is a `dialog` with an `open` attribute.
fn is_open_dialog(document: &Document, element: NodeId) -> bool {
    document.tag(element) == Some("dialog") && document.attr(element, "open").is_some()
}

/// Whether the element is focusable only as an open dialog, with no
/// `tabindex`: what the dialog focusing steps and `focus()` focus, and
/// Tab and a click pass over.
fn open_dialog_alone(document: &Document, element: NodeId) -> bool {
    is_open_dialog(document, element) && tab_index(document, element).is_none()
}

/// Whether the element is a focusable area that takes the focus itself:
/// [`focusable`], and not a shadow host that delegates focus, whose
/// focus goes to its focus delegate (D68).
pub(crate) fn focus_area(document: &Document, styles: Option<&Styles>, element: NodeId) -> bool {
    !delegates_focus(document, element) && focusable(document, styles, element)
}

/// Whether `element` hosts a shadow root that delegates focus.
pub(crate) fn delegates_focus(document: &Document, element: NodeId) -> bool {
    document
        .shadow_root(element)
        .and_then(|root| document.shadow_init(root))
        .is_some_and(|init| init.delegates_focus)
}

/// Whether the element can take the focus: [`focusable`], or a scroll
/// container `laid` says can scroll, which the reference browser focuses
/// (gap batch H) though its `tabIndex` stays -1.
pub(crate) fn focusable_in(
    document: &Document,
    styles: Option<&Styles>,
    laid: Option<&Laid>,
    element: NodeId,
) -> bool {
    focusable(document, styles, element)
        || (laid.is_some_and(|laid| laid.scrollers.contains(&element))
            && rendered(document, styles, element))
}

/// Whether Tab stops at the element: a [`focus_area`], or a scroll
/// container `laid` says can scroll that holds no focusable area (the
/// reference browser's keyboard focusable scrollers, gap batch H).
fn sequential_area(
    document: &Document,
    styles: Option<&Styles>,
    laid: Option<&Laid>,
    element: NodeId,
) -> bool {
    if focus_area(document, styles, element) {
        return true;
    }
    laid.is_some_and(|laid| laid.scrollers.contains(&element))
        && rendered(document, styles, element)
        && !document
            .subtree(element)
            .skip(1)
            .any(|node| focusable(document, styles, node))
}

/// Whether the element is being rendered: in the document's flat tree
/// (D68), it and every ancestor there with a style other than
/// `display: none`, and no ancestor skipping its contents by
/// `content-visibility: hidden` (CSS Containment 2 section 4.1; gap
/// batch H), as the reference browser has it, where the contents an
/// `auto` one skips still take the focus.
fn rendered(document: &Document, styles: Option<&Styles>, element: NodeId) -> bool {
    if !document.in_flat_tree(element) || in_closed_details(document, element) {
        return false;
    }
    let Some(styles) = styles else {
        return true;
    };
    // An element of `display: contents` has no box, but takes the focus
    // as the reference browser's does; a slot so, as by default (D69),
    // does not, its focus scope standing for it.
    if document.is_slot(element)
        && styles
            .get(element)
            .is_some_and(|style| style.display == Display::Contents)
    {
        return false;
    }
    std::iter::once(element)
        .chain(document.flat_ancestors(element))
        .filter(|&node| document.is_element(node))
        .all(|node| {
            styles.get(node).is_some_and(|style| {
                style.display != Display::None
                    && !(node != element
                        && style.skips_contents()
                        && style.containment().content_visibility == ContentVisibility::Hidden)
            })
        })
}

/// HTML's "focus delegate" of `target` (section 6.6.3), for a shadow host
/// that delegates focus (D68) and for a dialog (section 4.11.4, gap batch
/// H); none for another element, or a host that does not delegate. Where
/// it looks is the host's shadow tree, or the dialog's own subtree: the
/// first element there with `autofocus` that is a focusable area or
/// delegates to one, failing which the first that is or does, in tree
/// order, as the reference browser finds them (a dialog's focusable
/// areas whatever their `tabindex`).
pub(crate) fn focus_delegate(
    document: &Document,
    styles: Option<&Styles>,
    target: NodeId,
) -> Option<NodeId> {
    let root = match document.shadow_root(target) {
        Some(root) if delegates_focus(document, target) => root,
        Some(_) => return None,
        None if document.tag(target) == Some("dialog") => target,
        None => return None,
    };
    let elements = elements_under(document, root);
    autofocus_among(document, styles, &elements).or_else(|| {
        elements
            .iter()
            .find_map(|&element| candidate(document, styles, element))
    })
}

/// HTML's "autofocus delegate" of `target`: the first element in its
/// subtree (its shadow tree, for a host) with `autofocus` that is a
/// focusable area or delegates focus to one: what a popover shown
/// focuses (section 6.12, gap batch H).
pub(crate) fn autofocus_delegate(
    document: &Document,
    styles: Option<&Styles>,
    target: NodeId,
) -> Option<NodeId> {
    let root = document.shadow_root(target).unwrap_or(target);
    autofocus_among(document, styles, &elements_under(document, root))
}

/// The elements under `root`, `root` aside, in tree order.
fn elements_under(document: &Document, root: NodeId) -> Vec<NodeId> {
    document
        .subtree(root)
        .skip(1)
        .filter(|&node| document.is_element(node))
        .collect()
}

/// The first of `elements` with `autofocus` that is a [`candidate`].
fn autofocus_among(
    document: &Document,
    styles: Option<&Styles>,
    elements: &[NodeId],
) -> Option<NodeId> {
    elements
        .iter()
        .filter(|&&element| document.attr(element, "autofocus").is_some())
        .find_map(|&element| candidate(document, styles, element))
}

/// `element` when it is a focusable area that takes the focus itself,
/// else its own focus delegate: a nested host's is searched for in its
/// own tree, which the DOM's depth cap bounds.
fn candidate(document: &Document, styles: Option<&Styles>, element: NodeId) -> Option<NodeId> {
    if focus_area(document, styles, element) && !open_dialog_alone(document, element) {
        return Some(element);
    }
    if document.shadow_root(element).is_some() {
        return focus_delegate(document, styles, element);
    }
    None
}

/// The reading flow `container` orders its items by (CSS Display 4
/// section 5.1), if it is one whose `reading-flow` applies: a flex
/// container's flex values, a grid container's grid ones, `source-order`
/// on either or on a block container.
fn reading_flow(styles: Option<&Styles>, container: NodeId) -> Option<ReadingFlow> {
    let style = styles?.get(container)?;
    let flow = style.grid().reading_flow;
    let display = style.display;
    let applies = match flow {
        ReadingFlow::Normal => false,
        ReadingFlow::FlexVisual | ReadingFlow::FlexFlow => display.is_flex(),
        ReadingFlow::GridRows | ReadingFlow::GridColumns | ReadingFlow::GridOrder => {
            display.is_grid()
        }
        ReadingFlow::SourceOrder => {
            display.has_items()
                || matches!(
                    display,
                    Display::Block
                        | Display::InlineBlock
                        | Display::ListItem
                        | Display::TableCell
                        | Display::TableCaption
                )
        }
    };
    applies.then_some(flow)
}

/// The display of `element`'s style, `None` with no styles or none for it.
fn display_of(styles: Option<&Styles>, element: NodeId) -> Option<Display> {
    styles?.get(element).map(|style| style.display)
}

/// The reading flow container `element` is in, if any (CSS Display 4
/// section 5.1): its nearest ancestor in the flat tree that has a box,
/// those of `display: contents` passed through, when that orders its
/// items by a reading flow.
fn reading_container(
    document: &Document,
    styles: Option<&Styles>,
    element: NodeId,
) -> Option<NodeId> {
    // The DOM's depth cap bounds the walk.
    let mut at = document.flat_parent_element(element)?;
    while display_of(styles, at)? == Display::Contents {
        at = document.flat_parent_element(at)?;
    }
    reading_flow(styles, at).map(|_| at)
}

/// Whether `element` is a reading flow item: one with a box of its own
/// in a reading flow container.
fn reading_item(document: &Document, styles: Option<&Styles>, element: NodeId) -> bool {
    display_of(styles, element).is_some_and(|d| !matches!(d, Display::None | Display::Contents))
        && reading_container(document, styles, element).is_some()
}

/// Whether `element`, of `display: contents` in a reading flow container
/// and no slot, groups the items in it: the reference browser visits
/// them together, where the first of them in the reading flow is.
fn reading_group(document: &Document, styles: Option<&Styles>, element: NodeId) -> bool {
    display_of(styles, element) == Some(Display::Contents)
        && !document.is_slot(element)
        && reading_container(document, styles, element).is_some()
}

/// Whether `slot`, of `display: contents` in a reading flow container,
/// gives its assigned nodes to that container's reading flow as items of
/// their own, mixed with the others, as the reference browser does.
fn reading_slot(document: &Document, styles: Option<&Styles>, slot: NodeId) -> bool {
    document.is_slot(slot)
        && display_of(styles, slot) == Some(Display::Contents)
        && reading_container(document, styles, slot).is_some()
}

/// The focus navigation scope owner an element belongs to (HTML section
/// 6.6.3, "associated focus navigation owner"): a slotted element's
/// slot, and a slot's fallback content's, as the reference browser
/// scopes it; a shadow tree's host; a reading flow container's for its
/// items, an item's for what is in it, and an element grouping items
/// for them (HTML's reading flow scopes), a slot in a reading flow
/// passing its own on to the scope it is in; the document; `None` for a
/// host's child no slot has, which is in no scope.
fn scope_owner(document: &Document, styles: Option<&Styles>, element: NodeId) -> Option<NodeId> {
    let mut at = element;
    // Each turn moves up the tree, or to a slot further up the flat
    // tree, which the DOM's depth cap bounds.
    for _ in 0..dom::MAX_DEPTH * 4 {
        let parent = document.parent(at)?;
        if document.shadow_root(parent).is_some() {
            let slot = document.assigned_slot(at)?;
            if reading_slot(document, styles, slot) {
                at = slot;
                continue;
            }
            return Some(slot);
        }
        if document.is_slot(parent) && document.is_shadow_root(document.root_of(parent)) {
            if reading_slot(document, styles, parent) {
                at = parent;
                continue;
            }
            return Some(parent);
        }
        if let Some(host) = document.shadow_host(parent) {
            return Some(host);
        }
        if reading_flow(styles, parent).is_some()
            || reading_item(document, styles, parent)
            || reading_group(document, styles, parent)
        {
            return Some(parent);
        }
        if document.is_document(parent) || document.document_element() == Some(parent) {
            return Some(document.root());
        }
        at = parent;
    }
    None
}

/// The sequential focus navigation order (HTML section 6.6.3): the
/// flattened tabindex-ordered focus navigation scope of the document.
/// Each scope's focusable areas and the scope owners in it (hosts,
/// slots, reading flow containers and their items), but those whose
/// `tabindex` is negative, those with a positive one first, by it and
/// then tree order, then the rest in tree order; each owner followed by
/// its own scope's, or replaced by it when it is no focusable area
/// itself. A reading flow container's scope is in its reading flow
/// instead, whatever its items' `tabindex`. A scroll container `laid`
/// says can scroll that holds no focusable area is one of the rest (the
/// reference browser's keyboard focusable scrollers, gap batch H).
pub(crate) fn sequential_order(
    document: &Document,
    styles: Option<&Styles>,
    laid: Option<&Laid>,
) -> Vec<NodeId> {
    let order = Order {
        document,
        styles,
        laid,
        scopes: scopes(document, styles, laid),
    };
    order.flattened(document.root())
}

/// The area Tab moves the focus to from `current`, an element focused
/// that is not in the [`sequential_order`] (its `tabindex` negative, or
/// in a scope whose owner's is, D68), as the reference browser finds it:
/// the next area of `current`'s own scope, or of the scope around that,
/// and so on out to the document's, an owner standing for the areas of
/// its scope; the previous, `backward`. `None` when there is none, or
/// `current` is in no scope.
pub(crate) fn navigate_from(
    document: &Document,
    styles: Option<&Styles>,
    laid: Option<&Laid>,
    current: NodeId,
    backward: bool,
) -> Option<NodeId> {
    let order = Order {
        document,
        styles,
        laid,
        scopes: scopes(document, styles, laid),
    };
    let root = document.root();
    // Forward from a scope owner, its own scope's first area, as the
    // reference browser goes into a focused host's tree or reading flow
    // (gap batch H).
    if !backward
        && order.scopes.contains_key(&current)
        && let Some(&first) = order.flattened(current).first()
    {
        return Some(first);
    }
    let mut from = current;
    // Each turn moves out a scope, which the DOM's depth cap bounds.
    while from != root {
        let owner = scope_owner(document, styles, from)?;
        let in_tree_order = order.scopes.get(&owner).map_or(&[][..], Vec::as_slice);
        let items = order.items_of(owner);
        // After `from` in its scope's order when it is there, else after
        // it in tree order, whatever the items' `tabindex`.
        let candidates: Vec<NodeId> = match items.iter().position(|&item| item == from) {
            Some(at) if backward => items[..at].iter().rev().copied().collect(),
            Some(at) => items[at + 1..].to_vec(),
            None if backward => in_tree_order
                .iter()
                .copied()
                .filter(|&item| document.shadow_including_precedes(item, from))
                .rev()
                .collect(),
            None => in_tree_order
                .iter()
                .copied()
                .filter(|&item| document.shadow_including_precedes(from, item))
                .collect(),
        };
        for item in candidates {
            let inner = match order.scopes.contains_key(&item) {
                true => order.flattened(item),
                false => Vec::new(),
            };
            let area = order.area(item).then_some(item);
            let found = match backward {
                true => inner.last().copied().or(area),
                false => area.or_else(|| inner.first().copied()),
            };
            if found.is_some() {
                return found;
            }
        }
        from = owner;
    }
    None
}

/// Each focus navigation scope's items, by its owner (the document, a
/// host, a slot, a reading flow container or item): its focusable areas,
/// its keyboard focusable scrollers and the owners of the scopes in it,
/// but those whose `tabindex` is negative, in tree order. A reading flow
/// item, or an element grouping some, is its scope's whatever its
/// `tabindex`, standing for its own scope, as the reference browser has
/// it.
fn scopes(
    document: &Document,
    styles: Option<&Styles>,
    laid: Option<&Laid>,
) -> HashMap<NodeId, Vec<NodeId>> {
    let root = document.root();
    let nodes = match document.has_shadow_roots() {
        true => document.shadow_including_subtree(root),
        false => document.subtree(root).collect(),
    };
    let mut scopes: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for node in nodes {
        if !document.is_element(node) {
            continue;
        }
        let item = reading_item(document, styles, node) || reading_group(document, styles, node);
        let owner_kind = document.shadow_root(node).is_some()
            || (document.is_slot(node) && document.is_shadow_root(document.root_of(node)))
            || reading_flow(styles, node).is_some()
            || item;
        // An open dialog is focusable, but not by Tab (gap batch H).
        if !owner_kind
            && (!sequential_area(document, styles, laid, node) || open_dialog_alone(document, node))
        {
            continue;
        }
        if !item && tab_index(document, node).is_some_and(|n| n < 0) {
            continue;
        }
        if let Some(owner) = scope_owner(document, styles, node) {
            scopes.entry(owner).or_default().push(node);
        }
    }
    scopes
}

/// What the sequential order is made from: the document, its styles and
/// layout, and its focus navigation scopes.
struct Order<'a> {
    document: &'a Document,
    styles: Option<&'a Styles>,
    laid: Option<&'a Laid>,
    scopes: HashMap<NodeId, Vec<NodeId>>,
}

impl Order<'_> {
    /// Whether Tab stops at `item` of a scope: a [`sequential_area`]
    /// whose `tabindex` is not negative.
    fn area(&self, item: NodeId) -> bool {
        !tab_index(self.document, item).is_some_and(|n| n < 0)
            && sequential_area(self.document, self.styles, self.laid, item)
    }

    /// The items of `owner`'s scope in its order: a reading flow
    /// container's, and those of an element grouping items in one, in the
    /// container's reading flow, an element grouping items where the first
    /// of them is; any other's in tabindex order.
    fn items_of(&self, owner: NodeId) -> Vec<NodeId> {
        let items = self.scopes.get(&owner).map_or(&[][..], Vec::as_slice);
        let (document, styles) = (self.document, self.styles);
        let container = match reading_flow(styles, owner) {
            Some(_) => Some(owner),
            None if reading_group(document, styles, owner) => {
                reading_container(document, styles, owner)
            }
            None => None,
        };
        let Some((container, flow)) =
            container.and_then(|c| reading_flow(styles, c).map(|flow| (c, flow)))
        else {
            return ordered(document, items);
        };
        let placed: HashMap<NodeId, usize> = self
            .in_flow_order(container, self.flow_items(container), flow)
            .into_iter()
            .enumerate()
            .map(|(at, node)| (node, at))
            .collect();
        let mut keyed: Vec<(usize, NodeId)> = items
            .iter()
            .map(|&item| (self.reading_place(&placed, item, 0), item))
            .collect();
        // A stable sort keeps tree order among equals.
        keyed.sort_by_key(|&(at, _)| at);
        keyed.into_iter().map(|(_, item)| item).collect()
    }

    /// Where `node` is in a reading flow whose items are `placed`: its own
    /// place, or that of the first item in it; last when there is none.
    fn reading_place(&self, placed: &HashMap<NodeId, usize>, node: NodeId, depth: usize) -> usize {
        if let Some(&at) = placed.get(&node) {
            return at;
        }
        if depth > dom::MAX_DEPTH {
            return usize::MAX;
        }
        self.document
            .flat_children(node)
            .map(|child| self.reading_place(placed, child, depth + 1))
            .min()
            .unwrap_or(usize::MAX)
    }

    /// The reading flow items of `container` (CSS Display 4 section
    /// 5.1): its element children in the flat tree, those of an element
    /// of `display: contents` in its place, none of `display: none`.
    fn flow_items(&self, container: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = self.document.flat_children(container).collect();
        stack.reverse();
        while let Some(node) = stack.pop() {
            if !self.document.is_element(node) {
                continue;
            }
            match display_of(self.styles, node) {
                Some(Display::Contents) if out.len() + stack.len() < dom::MAX_DEPTH * 64 => {
                    let children: Vec<NodeId> = self.document.flat_children(node).collect();
                    stack.extend(children.into_iter().rev());
                }
                Some(Display::None) => {}
                _ => out.push(node),
            }
        }
        out
    }

    /// The flattened order of `owner`'s scope: its items in its order,
    /// each owner among them followed by its own scope's, flattened so.
    fn flattened(&self, owner: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        // Each entry: a scope's items still to write, in order.
        let mut stack: Vec<std::vec::IntoIter<NodeId>> = vec![self.items_of(owner).into_iter()];
        while let Some(items) = stack.last_mut() {
            let Some(item) = items.next() else {
                stack.pop();
                continue;
            };
            if self.area(item) {
                out.push(item);
            }
            if self.scopes.contains_key(&item) && stack.len() <= dom::MAX_DEPTH {
                stack.push(self.items_of(item).into_iter());
            }
        }
        out
    }

    /// `items` in the order `flow` gives them in `container`.
    fn in_flow_order(
        &self,
        container: NodeId,
        items: Vec<NodeId>,
        flow: ReadingFlow,
    ) -> Vec<NodeId> {
        let styles = self.styles;
        let reading_order = |node: NodeId| {
            styles
                .and_then(|styles| styles.get(node))
                .map_or(0, |style| style.grid().reading_order)
        };
        let order = |node: NodeId| {
            styles
                .and_then(|styles| styles.get(node))
                .map_or(0, |style| style.order)
        };
        let boxes = self.laid.map(|laid| &laid.boxes);
        let place = |node: NodeId| boxes.and_then(|boxes| boxes.get(&node)).copied();
        let rtl = styles
            .and_then(|styles| styles.get(container))
            .is_some_and(|style| style.direction == style::computed::Direction::Rtl);
        let column = styles
            .and_then(|styles| styles.get(container))
            .is_some_and(|style| {
                matches!(
                    style.flex_direction,
                    FlexDirection::Column | FlexDirection::ColumnReverse
                )
            });
        // The items taken out of flow come after the rest, in tree order
        // (the reference browser's).
        let (mut items, mut out_of_flow): (Vec<NodeId>, Vec<NodeId>) =
            items.into_iter().partition(|&n| {
                !styles
                    .and_then(|styles| styles.get(n))
                    .is_some_and(|style| style.position.is_out_of_flow())
            });
        match flow {
            // Order-modified document order: a stable sort by `order`.
            ReadingFlow::FlexFlow | ReadingFlow::GridOrder => items.sort_by_key(|&n| order(n)),
            ReadingFlow::SourceOrder | ReadingFlow::Normal => {}
            ReadingFlow::FlexVisual => {
                // By the lines they are on, in the block direction, and
                // along each in the inline direction; a column flex
                // container's lines are its columns.
                items = in_lines(items, &place, column, rtl);
            }
            ReadingFlow::GridRows | ReadingFlow::GridColumns => {
                // By the row, then the column, each item's first; by the
                // column first for `grid-columns`.
                items = in_tracks(items, &place, flow == ReadingFlow::GridColumns, rtl);
            }
        }
        // `reading-order` first, the flow's order among equals.
        items.sort_by_key(|&n| reading_order(n));
        out_of_flow.sort_by_key(|&n| reading_order(n));
        items.extend(out_of_flow);
        items
    }
}

/// `items`, a scope's, in tabindex order: those with a positive
/// `tabindex` first, by it, then the rest, each group in tree order.
fn ordered(document: &Document, items: &[NodeId]) -> Vec<NodeId> {
    let (mut positive, rest): (Vec<NodeId>, Vec<NodeId>) = items
        .iter()
        .partition(|&&node| tab_index(document, node).is_some_and(|n| n > 0));
    // A stable sort keeps tree order among equals.
    positive.sort_by_key(|&node| tab_index(document, node));
    positive.extend(rest);
    positive
}

/// An item and its border box.
type Boxed = (NodeId, [f32; 4]);

/// `items` by the grid tracks their boxes start in: by the row, then
/// the column from the left (the right for `rtl`), or with `across` by
/// the column, then the row; an item with no box last.
fn in_tracks(
    items: Vec<NodeId>,
    place: &impl Fn(NodeId) -> Option<[f32; 4]>,
    across: bool,
    rtl: bool,
) -> Vec<NodeId> {
    let x = |b: &[f32; 4]| if rtl { -(b[0] + b[2]) } else { b[0] };
    let mut placed: Vec<(NodeId, [f32; 2])> = Vec::new();
    let mut unplaced = Vec::new();
    for node in items {
        match place(node) {
            Some(b) if across => placed.push((node, [x(&b), b[1]])),
            Some(b) => placed.push((node, [b[1], x(&b)])),
            None => unplaced.push(node),
        }
    }
    // A stable sort keeps tree order among items in the same cell.
    placed.sort_by(|a, b| {
        a.1[0]
            .total_cmp(&b.1[0])
            .then_with(|| a.1[1].total_cmp(&b.1[1]))
    });
    let mut out: Vec<NodeId> = placed.into_iter().map(|(node, _)| node).collect();
    out.extend(unplaced);
    out
}

/// `items` by the lines their boxes are on: rows from the top, each from
/// the left (the right for `rtl`), or with `across` columns from the left
/// (the right), each from the top; an item with no box last.
fn in_lines(
    items: Vec<NodeId>,
    place: &impl Fn(NodeId) -> Option<[f32; 4]>,
    across: bool,
    rtl: bool,
) -> Vec<NodeId> {
    let (mut placed, unplaced): (Vec<Boxed>, Vec<NodeId>) = {
        let mut placed = Vec::new();
        let mut unplaced = Vec::new();
        for node in items {
            match place(node) {
                Some(b) => placed.push((node, b)),
                None => unplaced.push(node),
            }
        }
        (placed, unplaced)
    };
    // The axis lines run across, and the one along them.
    let (line, along) = if across { (0, 1) } else { (1, 0) };
    let reading = |b: &[f32; 4], axis: usize| {
        let start = b[axis];
        if axis == 0 && rtl {
            -(start + b[2])
        } else {
            start
        }
    };
    placed.sort_by(|a, b| reading(&a.1, line).total_cmp(&reading(&b.1, line)));
    // Lines: boxes whose extents across overlap the line's, each with
    // its extent.
    let mut lines: Vec<(f32, f32, Vec<Boxed>)> = Vec::new();
    for (node, b) in placed {
        let (from, to) = (b[line], b[line] + b[line + 2]);
        match lines.last_mut() {
            Some((start, end, members)) if from < *end && to > *start => {
                *end = end.max(to);
                members.push((node, b));
            }
            _ => lines.push((from, to, vec![(node, b)])),
        }
    }
    let mut out = Vec::new();
    for (_, _, mut members) in lines {
        members.sort_by(|a, b| reading(&a.1, along).total_cmp(&reading(&b.1, along)));
        out.extend(members.into_iter().map(|(node, _)| node));
    }
    out.extend(unplaced);
    out
}

/// The element a click focuses (HTML section 6.6.4's "focusable area"
/// of the event's target): the nearest inclusive ancestor in the flat
/// tree that is a focusable area, or a host that delegates focus, for
/// its focus delegate.
pub(crate) fn click_focus_target(
    document: &Document,
    styles: Option<&Styles>,
    node: NodeId,
) -> Option<NodeId> {
    let found = std::iter::once(node)
        .chain(document.flat_ancestors(node))
        .find(|&node| {
            delegates_focus(document, node)
                || (focusable(document, styles, node) && !open_dialog_alone(document, node))
        })?;
    if !delegates_focus(document, found) {
        return Some(found);
    }
    // The focus already in the host's tree stays where it is.
    match document.focused() {
        Some(focused) if document.is_shadow_including_inclusive_ancestor(found, focused) => {
            Some(focused)
        }
        _ => focus_delegate(document, styles, found),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_read_in_the_inline_direction() {
        let node = |n: u64| NodeId::from_bits((1 << 32) | n).expect("an id");
        let (a, b, c) = (node(1), node(2), node(3));
        // a and b on one row, a left of b; c on the row below.
        let boxes: HashMap<NodeId, [f32; 4]> = [
            (a, [0.0, 0.0, 10.0, 10.0]),
            (b, [50.0, 2.0, 10.0, 10.0]),
            (c, [0.0, 30.0, 10.0, 10.0]),
        ]
        .into_iter()
        .collect();
        let place = |n: NodeId| boxes.get(&n).copied();
        assert_eq!(in_lines(vec![c, b, a], &place, false, false), [a, b, c]);
        // Right to left along each row.
        assert_eq!(in_lines(vec![c, b, a], &place, false, true), [b, a, c]);
        // By columns: a and c in the left one, b in the right.
        assert_eq!(in_lines(vec![c, b, a], &place, true, false), [a, c, b]);
        // An item with no box last.
        let d = node(4);
        assert_eq!(in_lines(vec![d, a], &place, false, false), [a, d]);
    }
}
