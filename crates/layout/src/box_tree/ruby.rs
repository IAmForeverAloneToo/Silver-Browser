//! Ruby's boxes (CSS Ruby 1 section 2, gap batch H, D77): a ruby
//! container is an inline box of columns, each column a base and the
//! annotation over or under it, which `crate::ruby` lays out as one
//! atomic piece of its line.
//!
//! The container's content is paired as CSS Ruby 1 section 2.2's
//! anonymous box fixup pairs it, for the one level of annotations HTML's
//! markup makes: the bases met before a run of annotations take them in
//! order, the first base the first annotation; a run of content that is
//! not a ruby box is a base of its own (an anonymous one), and white
//! space alone between them is none. A base left without an annotation
//! is a column alone, and an annotation without a base sits over an
//! empty one. An annotation container (`rtc`) is one annotation, and a
//! base container (`rbc`) one base. A column is an anonymous
//! [`BoxKind::InlineBlock`] whose style, inherited from the container,
//! has `display: ruby`, which is how layout tells it from any other
//! inline-block ([`crate::ruby::is_column`]); its children are the base
//! and then the annotation, each an inline-block of its own content.

use style::ComputedStyle;
use style::computed::Display;

use super::{BoxKind, Item, LayoutBox, Styled, build_block_container, build_block_container_of};

/// One base of a column, before it is built: a ruby base element, or a
/// run of the container's other content.
enum Base {
    Element(Item),
    Run(Vec<Item>),
}

/// The inline box of the ruby container `item`, of the computed `style`:
/// its columns.
pub(super) fn container(styled: &Styled<'_>, item: Item, style: &ComputedStyle) -> LayoutBox {
    styled.own_box(item, style, BoxKind::Inline, || {
        columns(styled, item, &styled.flat_children(item))
    })
}

/// What the block of the `block ruby` element `item` holds: one line of
/// one anonymous inline ruby container of its content.
pub(super) fn block_contents(styled: &Styled<'_>, item: Item) -> Vec<LayoutBox> {
    let inline = styled.anonymous_box(
        item,
        Display::Ruby,
        BoxKind::Inline,
        columns(styled, item, &styled.flat_children(item)),
    );
    vec![styled.anonymous_box(item, Display::Block, BoxKind::AnonymousBlock, vec![inline])]
}

/// The anonymous ruby container around `item`, a base or an annotation
/// met outside any (CSS Ruby 1 section 2.2).
pub(super) fn wrapped(styled: &Styled<'_>, item: Item) -> LayoutBox {
    let columns = columns(styled, item, &[item]);
    styled.anonymous_box(item, Display::Ruby, BoxKind::Inline, columns)
}

/// The columns of the ruby container `parent` whose content is
/// `children`, paired as the module says.
fn columns(styled: &Styled<'_>, parent: Item, children: &[Item]) -> Vec<LayoutBox> {
    let mut out = Vec::new();
    let mut bases: Vec<Base> = Vec::new();
    let mut annotations: Vec<Item> = Vec::new();
    let mut run: Vec<Item> = Vec::new();
    for &child in children {
        let display = if styled.is_element(child) {
            match styled.style(child) {
                Some(style) if style.display != Display::None => Some(style.display),
                _ => continue,
            }
        } else {
            None
        };
        match display {
            Some(Display::RubyText | Display::RubyTextContainer) => {
                close_run(styled, &mut run, &mut bases);
                annotations.push(child);
            }
            Some(Display::RubyBase | Display::RubyBaseContainer) => {
                if !annotations.is_empty() {
                    flush(styled, parent, &mut bases, &mut annotations, &mut out);
                }
                close_run(styled, &mut run, &mut bases);
                bases.push(Base::Element(child));
            }
            _ => {
                if !annotations.is_empty() && !is_white_space(styled, child) {
                    flush(styled, parent, &mut bases, &mut annotations, &mut out);
                }
                if annotations.is_empty() {
                    run.push(child);
                }
            }
        }
    }
    close_run(styled, &mut run, &mut bases);
    flush(styled, parent, &mut bases, &mut annotations, &mut out);
    out
}

/// Whether `item` is a text of white space alone.
fn is_white_space(styled: &Styled<'_>, item: Item) -> bool {
    !styled.is_element(item)
        && styled
            .text(item)
            .is_none_or(|text| text.chars().all(crate::inline::is_css_space))
}

/// Ends the run of content being collected into a base: a base of its
/// own when there is anything but white space in it.
fn close_run(styled: &Styled<'_>, run: &mut Vec<Item>, bases: &mut Vec<Base>) {
    let items = std::mem::take(run);
    if items.iter().any(|&item| !is_white_space(styled, item)) {
        bases.push(Base::Run(items));
    }
}

/// The columns of the bases and annotations met so far, paired in
/// order, added to `out`.
fn flush(
    styled: &Styled<'_>,
    parent: Item,
    bases: &mut Vec<Base>,
    annotations: &mut Vec<Item>,
    out: &mut Vec<LayoutBox>,
) {
    let count = bases.len().max(annotations.len());
    let mut bases = std::mem::take(bases).into_iter();
    let mut annotations = std::mem::take(annotations).into_iter();
    for _ in 0..count {
        out.push(column(styled, parent, bases.next(), annotations.next()));
    }
}

/// The column of `base` and `annotation` in the ruby container `parent`.
fn column(
    styled: &Styled<'_>,
    parent: Item,
    base: Option<Base>,
    annotation: Option<Item>,
) -> LayoutBox {
    let base = match base {
        Some(Base::Element(item)) => match styled.style(item) {
            Some(style) => styled.own_box(item, style, BoxKind::InlineBlock, || {
                build_block_container(styled, item)
            }),
            None => empty(styled, parent),
        },
        Some(Base::Run(items)) => styled.anonymous_box(
            parent,
            Display::InlineBlock,
            BoxKind::InlineBlock,
            build_block_container_of(styled, parent, &items),
        ),
        None => empty(styled, parent),
    };
    let mut children = vec![base];
    if let Some(item) = annotation
        && let Some(style) = styled.style(item)
    {
        children.push(styled.own_box(item, style, BoxKind::InlineBlock, || {
            build_block_container(styled, item)
        }));
    }
    styled.anonymous_box(parent, Display::Ruby, BoxKind::InlineBlock, children)
}

/// An empty anonymous base, under an annotation that has none.
fn empty(styled: &Styled<'_>, parent: Item) -> LayoutBox {
    styled.anonymous_box(
        parent,
        Display::InlineBlock,
        BoxKind::InlineBlock,
        Vec::new(),
    )
}
