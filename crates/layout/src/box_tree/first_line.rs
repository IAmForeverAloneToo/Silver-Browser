//! `::first-line` (M16, docs/design/selectors-pseudo.md): once the box
//! tree is built, and before `::first-letter` splits its text, the run
//! holding each block container's first formatted line is given what
//! its boxes are on that line. Which of the run's content the line
//! holds only the inline layout finds, filling the line in these
//! styles ([`crate::inline`]).
//!
//! The run is the container's own first that has a line, or that of
//! its first in-flow block-level child that has one, recursively — not
//! inside a table, whose lines are its cells', nor after one, which has
//! no first formatted line of its own. A run has a line when it holds
//! text other than collapsible white space, a line break kept by
//! `white-space`, an atomic inline, or an inline element with a
//! horizontal edge of some width.
//!
//! The run's anonymous block gets the `::first-line` pseudo-elements
//! of the containers whose first line it holds, outermost first, as
//! Pseudo-Elements 4's fictional tag sequence nests them: just inside
//! the block whose line it is, the outermost inheriting from that block
//! and each next from the one outside it. The line's root inline box
//! takes the innermost's style. Every inline-level box in the run then
//! gets its style on the line, where that differs from its own: text
//! the first-line style of what it is in; an inline element, or a
//! `::before` or `::after` inline box, its own rules cascaded again
//! with a parent whose inherited properties are its parent's on the
//! line and whose others are its parent's own (§2.1.3). An atomic
//! inline keeps its style, as do the boxes in it; a float or an
//! absolutely positioned box is not on the line.

use style::computed::{LEFT, LengthPercentage, LengthPercentageAuto, RIGHT, WhiteSpace};
use style::selector_matching::PseudoElement;
use style::{ComputedStyle, Recascade};

use super::{BoxKind, FirstLine, LayoutBox, Styled};

/// Gives the run of each block container's first formatted line in
/// `boxes` and their descendants that has a `::first-line` the styles
/// of its boxes on that line.
pub(super) fn apply(styled: &Styled<'_>, boxes: &mut [LayoutBox]) {
    for b in boxes.iter_mut() {
        visit(styled, b);
    }
    for b in boxes {
        restyle_runs(styled.recascade, b);
    }
}

/// Adds the `::first-line` of `b`, if it has one, to its first line's
/// run, then those of the boxes inside it: outermost first.
fn visit(styled: &Styled<'_>, b: &mut LayoutBox) {
    // A flex or a grid container has no first formatted line
    // (css-flexbox-1 §3, M17; css-grid-2 §5.1, M18); its items, block
    // containers, have theirs.
    if !b.anonymous
        && !b.has_items()
        && styled
            .styles
            .pseudo(b.node, PseudoElement::FirstLine)
            .is_some()
    {
        let mut path = Vec::new();
        if let Found::Line = in_blocks(&b.children, &mut path) {
            add(styled.recascade, b, &path);
        }
    }
    for child in &mut b.children {
        visit(styled, child);
    }
}

/// What a walk of a block container's children for its first line
/// found.
pub(super) enum Found {
    /// The run of the first line: the path leads to it.
    Line,
    /// A box with no first formatted line of its own came first: none.
    Blocked,
    /// Nothing with a line: the line is further on.
    Empty,
}

/// The run of the first line among the block-level `children` of a
/// block container: the first with a line, recursively. Any other
/// box's children are found to be no such line. `path` is the way down
/// so far, by index, and on [`Found::Line`] the way to the run.
pub(super) fn in_blocks(children: &[LayoutBox], path: &mut Vec<usize>) -> Found {
    for (i, child) in children.iter().enumerate() {
        path.push(i);
        let found = match child.kind {
            BoxKind::AnonymousBlock if has_line(&child.children) => Found::Line,
            BoxKind::AnonymousBlock => Found::Empty,
            BoxKind::Block => in_blocks(&child.children, path),
            // Out of flow, and a list item's marker beside its content
            // (M21.6).
            BoxKind::Float | BoxKind::Absolute | BoxKind::Marker { .. } => Found::Empty,
            // A table's lines are its cells', a flex container's its
            // items' (M17).
            _ => Found::Blocked,
        };
        match found {
            Found::Empty => {
                path.pop();
            }
            found => return found,
        }
    }
    Found::Empty
}

/// Whether the inline-level `items` make a line box (CSS2 §9.4.2): text
/// other than collapsible white space, a line break `white-space` keeps
/// (a line of its own, though empty), an atomic inline, or an inline
/// element with a horizontal margin, border or padding — a percentage
/// counted as some width, which a zero containing block would not give.
fn has_line(items: &[LayoutBox]) -> bool {
    items.iter().any(|b| match &b.kind {
        BoxKind::Text(text) => {
            let breaks_kept = matches!(
                b.style.white_space,
                WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::PreLine
            );
            text.chars()
                .any(|c| !crate::inline::is_css_space(c) || (c == '\n' && breaks_kept))
        }
        BoxKind::Inline => has_edges(&b.style) || has_line(&b.children),
        // An inside marker drawn as a shape is an item on the line; an
        // outside one is as wide as nothing there (M21.6).
        BoxKind::InlineBlock
        | BoxKind::Table
        | BoxKind::Flex
        | BoxKind::Grid
        | BoxKind::Marker { outside: false, .. } => true,
        _ => false,
    })
}

/// Whether an inline element's start or end edge has some width.
fn has_edges(style: &ComputedStyle) -> bool {
    let border = style.used_border_width();
    [LEFT, RIGHT].into_iter().any(|side| {
        border[side] != 0.0
            || !matches!(
                style.margin[side],
                LengthPercentageAuto::Auto | LengthPercentageAuto::Px(0.0)
            )
            || style.padding[side] != LengthPercentage::Px(0.0)
    })
}

/// Adds `container`'s `::first-line` to the run `path` leads to, inside
/// the ones already there. The blocks on the way down are marked as
/// styled by an ancestor: the run's layout reads the `::first-line`,
/// which is not theirs.
fn add(recascade: &dyn Recascade, container: &mut LayoutBox, path: &[usize]) {
    let element = container.node;
    let (&last, way) = path.split_last().expect("a path to a run");
    let mut holder = container;
    for &i in way {
        holder = &mut holder.children[i];
        if holder.kind == BoxKind::Block {
            holder.styled_by_ancestor = true;
        }
    }
    // The block whose line it is, and the run.
    let LayoutBox {
        style: block,
        children,
        ..
    } = holder;
    let run = &mut children[last];
    let parent = run.first_line.as_ref().map_or(&*block, |line| &line.style);
    let Some(style) = recascade.pseudo_element(element, PseudoElement::FirstLine, parent) else {
        return; // no rule styles it after all
    };
    let pseudo = LayoutBox {
        style: style.clone(),
        kind: BoxKind::Inline,
        children: Vec::new(),
        node: element,
        anonymous: true,
        generated: Some(PseudoElement::FirstLine),
        generated_text: 0,
        decorations: None,
        styled_by_ancestor: false,
        first_line: None,
        tree_order: 0,
        item: false,
        parent: None,
        in_3d: false,
        replaced: None,
    };
    match &mut run.first_line {
        Some(line) => {
            line.style = style;
            line.chain.push(pseudo);
        }
        None => {
            run.first_line = Some(Box::new(FirstLine {
                style,
                chain: vec![pseudo],
            }));
        }
    }
}

/// Gives the boxes in each first line's run under `b` their styles on
/// the line.
fn restyle_runs(recascade: &dyn Recascade, b: &mut LayoutBox) {
    let LayoutBox {
        style, children, ..
    } = b;
    for child in children {
        match &child.first_line {
            Some(line) if child.kind == BoxKind::AnonymousBlock => {
                let line = line.style.clone();
                restyle(recascade, &mut child.children, style, &line);
            }
            _ => restyle_runs(recascade, child),
        }
    }
}

/// Gives the inline-level `items`, whose parent's style is `parent` and
/// on the first line `line`, their styles on the line where they differ
/// from their own. What they inherit from there is `line`'s inherited
/// properties and `parent`'s others: where that is `parent`, nothing in
/// them changes, and nothing is cascaded again.
fn restyle(
    recascade: &dyn Recascade,
    items: &mut [LayoutBox],
    parent: &ComputedStyle,
    line: &ComputedStyle,
) {
    let inherits = parent.with_inherited_from(line);
    if inherits == *parent {
        return;
    }
    for b in items {
        let style = match (&b.kind, b.generated) {
            // Text has the style of what it is in, there.
            (BoxKind::Text(_), _) => line.clone(),
            (BoxKind::Inline, None) => recascade.element(b.node, &inherits),
            (BoxKind::Inline, Some(pseudo)) => {
                match recascade.pseudo_element(b.node, pseudo, &inherits) {
                    Some(style) => style,
                    None => continue,
                }
            }
            // An atomic inline keeps its style, and a float or an
            // absolutely positioned box is not on the line.
            _ => continue,
        };
        let LayoutBox {
            style: own,
            children,
            ..
        } = &mut *b;
        restyle(recascade, children, own, &style);
        if style != *own {
            b.first_line = Some(Box::new(FirstLine {
                style,
                chain: Vec::new(),
            }));
        }
    }
}
