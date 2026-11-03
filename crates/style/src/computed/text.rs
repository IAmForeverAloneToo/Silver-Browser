//! The text properties past the common ones (CSS Text 3 and 4, gap batch
//! D's text gaps): what [`super::ComputedStyle::text`] points to when any
//! of them is not its initial value. All of them are inherited but
//! `text-group-align`.

use std::sync::Arc;

use super::{LengthPercentage, MathElement};

/// The text properties of [`super::ComputedStyle::text`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextProps {
    /// The element's content language, from the nearest `lang`
    /// attribute, ASCII-lowercased: what hyphenation, line breaking and
    /// case mapping are tailored by. Not a property; inherited as one.
    pub lang: Option<Arc<str>>,
    /// `text-indent`.
    pub text_indent: TextIndent,
    /// `tab-size`.
    pub tab_size: TabSize,
    /// `line-break`.
    pub line_break: LineBreak,
    /// `hyphens`.
    pub hyphens: Hyphens,
    /// `hyphenate-character`: `None` for `auto`.
    pub hyphenate_character: Option<Arc<str>>,
    /// `hyphenate-limit-chars`: the word, before and after limits,
    /// `None` for `auto`.
    pub hyphenate_limit_chars: [Option<i32>; 3],
    /// `hyphenate-limit-lines`: `None` for `no-limit`.
    pub hyphenate_limit_lines: Option<i32>,
    /// `text-autospace`.
    pub text_autospace: TextAutospace,
    /// `text-spacing-trim`.
    pub text_spacing_trim: TextSpacingTrim,
    /// `hanging-punctuation`.
    pub hanging_punctuation: HangingPunctuation,
    /// `text-wrap-style`.
    pub text_wrap_style: TextWrapStyle,
    /// `word-space-transform`.
    pub word_space_transform: WordSpaceTransform,
    /// `text-fit`.
    pub text_fit: TextFit,
    /// `text-group-align`, not inherited.
    pub text_group_align: TextGroupAlign,
    /// `text-transform`'s keywords past the case: `full-width`,
    /// `full-size-kana` and `math-auto`.
    pub transform_extra: TransformExtra,
    /// `letter-spacing` and `word-spacing` as computed when they have a
    /// percentage, of the font size: the style's own `letter_spacing`
    /// and `word_spacing` are them resolved.
    pub letter_spacing: Option<LengthPercentage>,
    pub word_spacing: Option<LengthPercentage>,
    /// `ruby-position` (CSS Ruby 1 section 3.1, D77).
    pub ruby_position: RubyPosition,
    /// `ruby-align` (CSS Ruby 1 section 4.1, D77).
    pub ruby_align: RubyAlign,
    /// `math-style` (MathML Core section 2.1.1, D77): whether a formula
    /// is set in display style, `normal`, or `compact`.
    pub math_style: MathStyle,
    /// `font-palette` (CSS Fonts 4 section 9.2): kept here, with the
    /// inherited properties the `font` shorthand leaves, as it leaves
    /// this one (the reference browser's, and WPT's
    /// font-palette-vs-shorthand.html).
    pub font_palette: FontPalette,
    /// Not a property, nor inherited: which of MathML Core's layouts the
    /// element's box takes when its `display` is `math` (D77), with what
    /// its attributes say for it; [`MathElement::None`] for an element
    /// that is not MathML's. Kept here, behind the pointer, as few
    /// elements are MathML's.
    pub math: MathElement,
}

/// `font-palette`'s computed value (CSS Fonts 4 section 9.2): a font's
/// palette by its keyword or by an `@font-palette-values` rule's name,
/// or a mix of two.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum FontPalette {
    #[default]
    Normal,
    Light,
    Dark,
    /// A `<dashed-ident>`, as written.
    Named(Arc<str>),
    /// `palette-mix()`.
    Mix(Arc<PaletteMix>),
}

/// `palette-mix()`: its interpolation method as it is written (`in`
/// and its space, the hue method after a polar one), and its two
/// palettes, each with its percentage when it has one, resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteMix {
    pub method: Arc<str>,
    pub parts: [(FontPalette, Option<f32>); 2],
}

/// `math-style`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MathStyle {
    #[default]
    Normal,
    Compact,
}

/// `ruby-position`: which side of its base an annotation goes, the
/// values the reference browser takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RubyPosition {
    #[default]
    Over,
    Under,
}

/// `ruby-align`: how a base or an annotation narrower than the other is
/// placed across the other's width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RubyAlign {
    /// Its space shared out around and between its characters.
    #[default]
    SpaceAround,
    /// Its space shared out between its characters alone.
    SpaceBetween,
    Start,
    Center,
}

impl TextProps {
    /// The values a child inherits: all but `text-group-align` and the
    /// MathML layout.
    pub(super) fn inherited(&self) -> TextProps {
        TextProps {
            text_group_align: TextGroupAlign::None,
            math: MathElement::None,
            ..self.clone()
        }
    }
}

/// `text-indent`: a length or percentage, and whether it is `hanging`
/// and `each-line`.
#[derive(Debug, Clone, PartialEq)]
pub struct TextIndent {
    pub length: LengthPercentage,
    pub hanging: bool,
    pub each_line: bool,
}

impl Default for TextIndent {
    fn default() -> Self {
        TextIndent {
            length: LengthPercentage::Px(0.0),
            hanging: false,
            each_line: false,
        }
    }
}

/// `tab-size`: a number of spaces, or a length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TabSize {
    Spaces(f32),
    Length(f32),
}

impl Default for TabSize {
    fn default() -> Self {
        TabSize::Spaces(8.0)
    }
}

/// `line-break` (CSS Text 3 section 5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineBreak {
    #[default]
    Auto,
    Loose,
    Normal,
    Strict,
    Anywhere,
}

/// `hyphens` (CSS Text 3 section 5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hyphens {
    None,
    /// Only at a soft hyphen: the initial value.
    #[default]
    Manual,
    Auto,
}

/// `text-autospace` (CSS Text 4 section 8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAutospace {
    #[default]
    Normal,
    NoAutospace,
    Auto,
    /// Where space goes, and how.
    Spaces {
        ideograph_alpha: bool,
        ideograph_numeric: bool,
        punctuation: bool,
        mode: AutospaceMode,
    },
}

/// Whether `text-autospace` inserts space or replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutospaceMode {
    #[default]
    Unset,
    Insert,
    Replace,
}

/// `text-spacing-trim` (CSS Text 4 section 8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextSpacingTrim {
    #[default]
    Normal,
    SpaceAll,
    SpaceFirst,
    TrimStart,
    TrimBoth,
    TrimAll,
    Auto,
}

/// `hanging-punctuation` (CSS Text 3 section 9): none of them for
/// `none`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HangingPunctuation {
    pub first: bool,
    pub end: HangingEnd,
    pub last: bool,
}

/// `hanging-punctuation`'s `force-end` or `allow-end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HangingEnd {
    #[default]
    None,
    ForceEnd,
    AllowEnd,
}

/// `text-wrap-style` (CSS Text 4 section 6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextWrapStyle {
    #[default]
    Auto,
    Balance,
    Stable,
    Pretty,
}

/// `word-space-transform` (CSS Text 4 section 4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WordSpaceTransform {
    #[default]
    None,
    /// `space` or `ideographic-space`, and whether `auto-phrase`.
    Space {
        ideographic: bool,
        auto_phrase: bool,
    },
}

/// `text-fit`: none, or how text grows or shrinks to its line.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextFit {
    pub kind: TextFitKind,
    pub method: TextFitMethod,
    /// The limit, a percentage.
    pub limit: Option<f32>,
}

/// `text-fit`'s `none`, `grow` or `shrink`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextFitKind {
    #[default]
    None,
    Grow,
    Shrink,
}

/// `text-fit`'s method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextFitMethod {
    #[default]
    Consistent,
    PerLine,
    PerLineAll,
}

/// `text-group-align`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextGroupAlign {
    #[default]
    None,
    Start,
    End,
    Left,
    Right,
    Center,
}

/// `text-transform`'s keywords past the case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransformExtra {
    pub full_width: bool,
    pub full_size_kana: bool,
    pub math_auto: bool,
}
