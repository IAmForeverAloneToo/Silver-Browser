//! The containment properties (CSS Containment 2, gap batch H): what
//! [`super::ComputedStyle::containment`] points to when any of them is
//! not its initial value, none of them inherited; and `pointer-events`
//! and `reading-flow`, which the engine's hit testing and focus order
//! read.

/// `contain`'s keywords (CSS Containment 2 section 2), a bit each:
/// `strict` and `content` are their keywords' sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Contain(pub u8);

impl Contain {
    /// `none`, the initial value.
    pub const NONE: Contain = Contain(0);
    /// `size`.
    pub const SIZE: Contain = Contain(1);
    /// `inline-size`.
    pub const INLINE_SIZE: Contain = Contain(2);
    /// `layout`.
    pub const LAYOUT: Contain = Contain(4);
    /// `style`.
    pub const STYLE: Contain = Contain(8);
    /// `paint`.
    pub const PAINT: Contain = Contain(16);
    /// `strict`: `size layout style paint`.
    pub const STRICT: Contain = Contain(1 | 4 | 8 | 16);
    /// `content`: `layout style paint`.
    pub const CONTENT: Contain = Contain(4 | 8 | 16);

    /// Whether every keyword of `other` is set.
    pub fn contains(self, other: Contain) -> bool {
        self.0 & other.0 == other.0
    }

    /// These keywords and `other`'s.
    pub fn with(self, other: Contain) -> Contain {
        Contain(self.0 | other.0)
    }
}

/// `content-visibility` (CSS Containment 2 section 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum ContentVisibility {
    /// No effect: the initial value.
    #[default]
    Visible,
    /// Layout, style and paint containment, and the contents skipped
    /// while they are not relevant to the user (section 4.3), with size
    /// containment then.
    Auto,
    /// Layout, style and paint containment, and the contents always
    /// skipped, with size containment.
    Hidden,
}

/// One axis of `contain-intrinsic-size` (CSS Sizing 4 section 5.1): the
/// size a box with size containment takes in that axis, as if its
/// content were that big.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ContainIntrinsicSize {
    /// `none`, the initial value: as if empty.
    #[default]
    None,
    /// A length, never negative, in CSS pixels.
    Length(f32),
    /// `auto <length>` or `auto none`: the box's last remembered size
    /// in that axis when it has one (section 5.2), else the length, or
    /// nothing.
    Auto(Option<f32>),
}

impl ContainIntrinsicSize {
    /// The size it gives, `remembered` being the box's last remembered
    /// size in its axis, if any: `None` for `none` and for `auto none`
    /// with nothing remembered.
    pub fn used(self, remembered: Option<f32>) -> Option<f32> {
        match self {
            ContainIntrinsicSize::None => None,
            ContainIntrinsicSize::Length(px) => Some(px),
            ContainIntrinsicSize::Auto(px) => remembered.or(px),
        }
    }

    /// Whether it is `auto`, which asks the box's size to be remembered.
    pub fn is_auto(self) -> bool {
        matches!(self, ContainIntrinsicSize::Auto(_))
    }
}

/// The containment properties of [`super::ComputedStyle::containment`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Containment {
    /// `contain`.
    pub contain: Contain,
    /// `content-visibility`.
    pub content_visibility: ContentVisibility,
    /// `contain-intrinsic-width` and `contain-intrinsic-height`, in
    /// that order; the logical longhands map onto them, the engine
    /// laying out horizontal text alone.
    pub intrinsic: [ContainIntrinsicSize; 2],
    /// Not a property: whether the element skips its contents (section
    /// 4.1), which [`ComputedStyle::skips_contents`] reads. Kept here,
    /// where only an element with a `content-visibility` other than
    /// `visible` has it, rather than in the style itself.
    ///
    /// [`ComputedStyle::skips_contents`]: crate::computed::ComputedStyle::skips_contents
    pub skips_contents: bool,
}

/// `reading-flow` (CSS Display 4 section 5.1): the order sequential
/// focus navigation takes a flex or grid container's items, or a block
/// container's children, in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum ReadingFlow {
    /// The initial value: tree order.
    #[default]
    Normal,
    /// A flex container's items in their visual order along the main
    /// axis.
    FlexVisual,
    /// A flex container's items in the order the flex layout put them,
    /// `order` and the direction's reversal taken.
    FlexFlow,
    /// A grid container's items row by row.
    GridRows,
    /// A grid container's items column by column.
    GridColumns,
    /// A grid container's items in order-modified document order.
    GridOrder,
    /// Any block, flex or grid container's children by `reading-order`.
    SourceOrder,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_intrinsic_size_takes_the_remembered_one_only_when_auto() {
        assert_eq!(ContainIntrinsicSize::None.used(Some(5.0)), None);
        assert_eq!(ContainIntrinsicSize::Length(3.0).used(Some(5.0)), Some(3.0));
        assert_eq!(ContainIntrinsicSize::Auto(Some(3.0)).used(None), Some(3.0));
        assert_eq!(
            ContainIntrinsicSize::Auto(Some(3.0)).used(Some(5.0)),
            Some(5.0)
        );
        assert_eq!(ContainIntrinsicSize::Auto(None).used(None), None);
        assert!(ContainIntrinsicSize::Auto(None).is_auto());
        assert!(!ContainIntrinsicSize::Length(1.0).is_auto());
    }

    #[test]
    fn strict_and_content_are_their_keywords() {
        assert!(Contain::STRICT.contains(Contain::SIZE.with(Contain::PAINT)));
        assert!(!Contain::CONTENT.contains(Contain::SIZE));
        assert!(Contain::CONTENT.contains(Contain::LAYOUT.with(Contain::STYLE)));
    }
}
