//! Animating the engine's properties (M22): a table of every longhand
//! the cascade has with its animation type (Web Animations Appendix A,
//! each property's own definition), and the code that copies, compares
//! and interpolates its computed value between two computed styles.
//!
//! A value is interpolated by its type (Values 4 §3) and the result
//! clamped to its property's range, so that an easing that overshoots
//! takes an opacity to 1 and a width to zero and no further:
//!
//! - numbers and lengths linearly, integers rounded to the nearest, a
//!   half up;
//! - a length and a percentage, or a math function, to the math
//!   function `calc((1 - p) * A + p * B)` M21 computes, which layout
//!   resolves with its basis — kept as a length plus a percentage
//!   whenever both are sums of such, so that a transition interrupted
//!   again and again does not grow the expression;
//! - colours in sRGB with premultiplied alpha (Color 4 §13.4);
//! - transforms by [`transform`](self::transform)'s rules;
//! - lists of the same shape, a grid's track list, part by part;
//! - every other value, and a pair a type cannot interpolate, discretely:
//!   the first value before progress one half, the second from it;
//!   `display` by Display 4 §2.9, the value other than `none` strictly
//!   between 0 and 1.
//!
//! Custom properties are not here: their names are the page's, and they
//! animate discretely, as token lists (Variables 1 §2), where they are
//! computed (`crate::custom`).

use std::sync::Arc;

use crate::calc::{Calc, Expression, MAX_DEPTH};
use crate::cascade::MAX_LENGTH_PX;
use crate::computed::{
    self, BOTTOM, ComputedStyle, DecorationThickness, LEFT, LengthPercentage, LengthPercentageAuto,
    MaxSize, RIGHT, Size, TOP, TrackBreadth, TrackList, TrackSection, TrackSize,
};

pub mod transform;

/// How a property animates (Web Animations Appendix A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationType {
    /// Not at all: a keyframe's declaration of it is ignored, and no
    /// transition starts for it.
    NotAnimatable,
    /// Discretely: the first value before progress one half, the
    /// second from it. A transition runs only with `allow-discrete`.
    Discrete,
    /// By computed value: interpolated where the two values can be,
    /// discretely where they cannot.
    ByComputedValue,
}

macro_rules! longhands {
    ($($variant:ident = $name:literal: $kind:ident,)*) => {
        /// A longhand the engine has, by the name the cascade knows it by.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Longhand {
            $($variant,)*
        }

        impl Longhand {
            /// Every longhand, in the table's order.
            pub const ALL: &'static [Longhand] = &[$(Longhand::$variant,)*];

            /// Its name.
            pub fn name(self) -> &'static str {
                match self {
                    $(Longhand::$variant => $name,)*
                }
            }

            /// The longhand of the name, as the cascade spells it after
            /// mapping its aliases; `None` for a shorthand or a name the
            /// engine does not know.
            pub fn from_name(name: &str) -> Option<Longhand> {
                match name {
                    $($name => Some(Longhand::$variant),)*
                    _ => None,
                }
            }

            /// How it animates.
            pub fn animation_type(self) -> AnimationType {
                match self {
                    $(Longhand::$variant => AnimationType::$kind,)*
                }
            }
        }
    };
}

longhands! {
    Color = "color": ByComputedValue,
    BackgroundColor = "background-color": ByComputedValue,
    FontSize = "font-size": ByComputedValue,
    FontFamily = "font-family": Discrete,
    Display = "display": Discrete,
    TextAlignAll = "text-align-all": Discrete,
    TextAlignLast = "text-align-last": Discrete,
    TextJustify = "text-justify": Discrete,
    TextTransform = "text-transform": Discrete,
    WhiteSpace = "white-space": Discrete,
    LetterSpacing = "letter-spacing": ByComputedValue,
    WordSpacing = "word-spacing": ByComputedValue,
    Width = "width": ByComputedValue,
    Height = "height": ByComputedValue,
    MinWidth = "min-width": ByComputedValue,
    MinHeight = "min-height": ByComputedValue,
    MaxWidth = "max-width": ByComputedValue,
    MaxHeight = "max-height": ByComputedValue,
    MarginTop = "margin-top": ByComputedValue,
    MarginRight = "margin-right": ByComputedValue,
    MarginBottom = "margin-bottom": ByComputedValue,
    MarginLeft = "margin-left": ByComputedValue,
    PaddingTop = "padding-top": ByComputedValue,
    PaddingRight = "padding-right": ByComputedValue,
    PaddingBottom = "padding-bottom": ByComputedValue,
    PaddingLeft = "padding-left": ByComputedValue,
    BorderTopWidth = "border-top-width": ByComputedValue,
    BorderRightWidth = "border-right-width": ByComputedValue,
    BorderBottomWidth = "border-bottom-width": ByComputedValue,
    BorderLeftWidth = "border-left-width": ByComputedValue,
    BorderTopColor = "border-top-color": ByComputedValue,
    BorderRightColor = "border-right-color": ByComputedValue,
    BorderBottomColor = "border-bottom-color": ByComputedValue,
    BorderLeftColor = "border-left-color": ByComputedValue,
    BorderTopStyle = "border-top-style": Discrete,
    BorderRightStyle = "border-right-style": Discrete,
    BorderBottomStyle = "border-bottom-style": Discrete,
    BorderLeftStyle = "border-left-style": Discrete,
    Overflow = "overflow": Discrete,
    BoxSizing = "box-sizing": Discrete,
    Float = "float": Discrete,
    Clear = "clear": Discrete,
    Position = "position": Discrete,
    Top = "top": ByComputedValue,
    Right = "right": ByComputedValue,
    Bottom = "bottom": ByComputedValue,
    Left = "left": ByComputedValue,
    ZIndex = "z-index": ByComputedValue,
    TableLayout = "table-layout": Discrete,
    BorderCollapse = "border-collapse": Discrete,
    BorderSpacing = "border-spacing": ByComputedValue,
    CaptionSide = "caption-side": Discrete,
    EmptyCells = "empty-cells": Discrete,
    VerticalAlign = "vertical-align": ByComputedValue,
    Content = "content": Discrete,
    Quotes = "quotes": Discrete,
    FlexDirection = "flex-direction": Discrete,
    FlexWrap = "flex-wrap": Discrete,
    Order = "order": ByComputedValue,
    FlexGrow = "flex-grow": ByComputedValue,
    FlexShrink = "flex-shrink": ByComputedValue,
    FlexBasis = "flex-basis": ByComputedValue,
    JustifyContent = "justify-content": Discrete,
    AlignContent = "align-content": Discrete,
    AlignItems = "align-items": Discrete,
    AlignSelf = "align-self": Discrete,
    JustifyItems = "justify-items": Discrete,
    JustifySelf = "justify-self": Discrete,
    RowGap = "row-gap": ByComputedValue,
    ColumnGap = "column-gap": ByComputedValue,
    GridTemplateColumns = "grid-template-columns": ByComputedValue,
    GridTemplateRows = "grid-template-rows": ByComputedValue,
    GridTemplateAreas = "grid-template-areas": Discrete,
    GridAutoColumns = "grid-auto-columns": ByComputedValue,
    GridAutoRows = "grid-auto-rows": ByComputedValue,
    GridAutoFlow = "grid-auto-flow": Discrete,
    GridRowStart = "grid-row-start": Discrete,
    GridRowEnd = "grid-row-end": Discrete,
    GridColumnStart = "grid-column-start": Discrete,
    GridColumnEnd = "grid-column-end": Discrete,
    Transform = "transform": ByComputedValue,
    TransformOrigin = "transform-origin": ByComputedValue,
    TransformBox = "transform-box": Discrete,
    Translate = "translate": ByComputedValue,
    Rotate = "rotate": ByComputedValue,
    Scale = "scale": ByComputedValue,
    TransformStyle = "transform-style": Discrete,
    Perspective = "perspective": ByComputedValue,
    PerspectiveOrigin = "perspective-origin": ByComputedValue,
    BackfaceVisibility = "backface-visibility": Discrete,
    Opacity = "opacity": ByComputedValue,
    WillChange = "will-change": NotAnimatable,
    TransitionProperty = "transition-property": NotAnimatable,
    TransitionDuration = "transition-duration": NotAnimatable,
    TransitionTimingFunction = "transition-timing-function": NotAnimatable,
    TransitionDelay = "transition-delay": NotAnimatable,
    TransitionBehavior = "transition-behavior": NotAnimatable,
    AnimationName = "animation-name": NotAnimatable,
    AnimationDuration = "animation-duration": NotAnimatable,
    AnimationTimingFunction = "animation-timing-function": NotAnimatable,
    AnimationIterationCount = "animation-iteration-count": NotAnimatable,
    AnimationDirection = "animation-direction": NotAnimatable,
    AnimationPlayState = "animation-play-state": NotAnimatable,
    AnimationDelay = "animation-delay": NotAnimatable,
    AnimationFillMode = "animation-fill-mode": NotAnimatable,
    BreakBefore = "break-before": Discrete,
    BreakAfter = "break-after": Discrete,
    BreakInside = "break-inside": Discrete,
    Orphans = "orphans": ByComputedValue,
    Widows = "widows": ByComputedValue,
    PrintColorAdjust = "print-color-adjust": Discrete,
    CounterReset = "counter-reset": Discrete,
    CounterIncrement = "counter-increment": Discrete,
    CounterSet = "counter-set": Discrete,
    ListStyleType = "list-style-type": Discrete,
    ListStylePosition = "list-style-position": Discrete,
    ListStyleImage = "list-style-image": Discrete,
    TextDecorationLine = "text-decoration-line": Discrete,
    TextDecorationStyle = "text-decoration-style": Discrete,
    TextDecorationColor = "text-decoration-color": ByComputedValue,
    TextDecorationThickness = "text-decoration-thickness": ByComputedValue,
    TextUnderlineOffset = "text-underline-offset": ByComputedValue,
    TextUnderlinePosition = "text-underline-position": Discrete,
}

impl Longhand {
    /// Whether the property animates at all.
    pub fn is_animatable(self) -> bool {
        self.animation_type() != AnimationType::NotAnimatable
    }
}

/// The longhands a property name stands for, as the cascade spells it
/// after mapping its aliases: a longhand itself, or a shorthand's
/// longhands, which is what a keyframe's declaration of a shorthand and
/// `transition-property` naming one mean (Web Animations §5.3.3,
/// Transitions 1 §2.1). Empty for a name the engine does not know.
pub fn longhands_of(name: &str) -> Vec<Longhand> {
    use Longhand::*;
    if let Some(longhand) = Longhand::from_name(name) {
        return vec![longhand];
    }
    let list: &[Longhand] = match name {
        "background" => &[BackgroundColor],
        "margin" => &[MarginTop, MarginRight, MarginBottom, MarginLeft],
        "padding" => &[PaddingTop, PaddingRight, PaddingBottom, PaddingLeft],
        "border-width" => &[
            BorderTopWidth,
            BorderRightWidth,
            BorderBottomWidth,
            BorderLeftWidth,
        ],
        "border-color" => &[
            BorderTopColor,
            BorderRightColor,
            BorderBottomColor,
            BorderLeftColor,
        ],
        "border-style" => &[
            BorderTopStyle,
            BorderRightStyle,
            BorderBottomStyle,
            BorderLeftStyle,
        ],
        "border" => &[
            BorderTopWidth,
            BorderRightWidth,
            BorderBottomWidth,
            BorderLeftWidth,
            BorderTopStyle,
            BorderRightStyle,
            BorderBottomStyle,
            BorderLeftStyle,
            BorderTopColor,
            BorderRightColor,
            BorderBottomColor,
            BorderLeftColor,
        ],
        "border-top" => &[BorderTopWidth, BorderTopStyle, BorderTopColor],
        "border-right" => &[BorderRightWidth, BorderRightStyle, BorderRightColor],
        "border-bottom" => &[BorderBottomWidth, BorderBottomStyle, BorderBottomColor],
        "border-left" => &[BorderLeftWidth, BorderLeftStyle, BorderLeftColor],
        "flex" => &[FlexGrow, FlexShrink, FlexBasis],
        "flex-flow" => &[FlexDirection, FlexWrap],
        "gap" => &[RowGap, ColumnGap],
        "place-content" => &[AlignContent, JustifyContent],
        "place-items" => &[AlignItems, JustifyItems],
        "place-self" => &[AlignSelf, JustifySelf],
        "grid-template" => &[GridTemplateColumns, GridTemplateRows, GridTemplateAreas],
        "grid" => &[
            GridTemplateColumns,
            GridTemplateRows,
            GridTemplateAreas,
            GridAutoColumns,
            GridAutoRows,
            GridAutoFlow,
        ],
        "grid-row" => &[GridRowStart, GridRowEnd],
        "grid-column" => &[GridColumnStart, GridColumnEnd],
        "grid-area" => &[GridRowStart, GridColumnStart, GridRowEnd, GridColumnEnd],
        "list-style" => &[ListStyleType, ListStylePosition, ListStyleImage],
        "text-decoration" => &[
            TextDecorationLine,
            TextDecorationStyle,
            TextDecorationColor,
            TextDecorationThickness,
        ],
        "text-align" => &[TextAlignAll, TextAlignLast],
        "page-break-before" => &[BreakBefore],
        "page-break-after" => &[BreakAfter],
        "page-break-inside" => &[BreakInside],
        "transition" => &[
            TransitionProperty,
            TransitionDuration,
            TransitionTimingFunction,
            TransitionDelay,
            TransitionBehavior,
        ],
        "animation" => &[
            AnimationName,
            AnimationDuration,
            AnimationTimingFunction,
            AnimationIterationCount,
            AnimationDirection,
            AnimationPlayState,
            AnimationDelay,
            AnimationFillMode,
        ],
        _ => &[],
    };
    list.to_vec()
}

/// Copies one part of a group behind a pointer from `from` to `to`,
/// sharing nothing new when neither has the group.
macro_rules! copy_group {
    ($to:expr, $from:expr, $field:ident, $read:ident, $write:ident, $($part:tt)+) => {{
        if $to.$field.is_some() || $from.$field.is_some() {
            let value = $from.$read().$($part)+.clone();
            $to.$write().$($part)+ = value;
        }
    }};
}

/// Copies `property`'s computed value from `from` into `to`.
pub fn copy(property: Longhand, from: &ComputedStyle, to: &mut ComputedStyle) {
    use Longhand::*;
    match property {
        Color => to.color = from.color,
        BackgroundColor => to.background_color = from.background_color,
        FontSize => to.font_size_px = from.font_size_px,
        FontFamily => to.font_family = from.font_family.clone(),
        Display => to.display = from.display,
        TextAlignAll => to.text_align = from.text_align,
        TextAlignLast => to.text_align_last = from.text_align_last,
        TextJustify => to.text_justify = from.text_justify,
        TextTransform => to.text_transform = from.text_transform,
        WhiteSpace => to.white_space = from.white_space,
        LetterSpacing => to.letter_spacing = from.letter_spacing,
        WordSpacing => to.word_spacing = from.word_spacing,
        Width => to.width = from.width.clone(),
        Height => to.height = from.height.clone(),
        MinWidth => to.min_width = from.min_width.clone(),
        MinHeight => to.min_height = from.min_height.clone(),
        MaxWidth => to.max_width = from.max_width.clone(),
        MaxHeight => to.max_height = from.max_height.clone(),
        MarginTop => to.margin[TOP] = from.margin[TOP].clone(),
        MarginRight => to.margin[RIGHT] = from.margin[RIGHT].clone(),
        MarginBottom => to.margin[BOTTOM] = from.margin[BOTTOM].clone(),
        MarginLeft => to.margin[LEFT] = from.margin[LEFT].clone(),
        PaddingTop => to.padding[TOP] = from.padding[TOP].clone(),
        PaddingRight => to.padding[RIGHT] = from.padding[RIGHT].clone(),
        PaddingBottom => to.padding[BOTTOM] = from.padding[BOTTOM].clone(),
        PaddingLeft => to.padding[LEFT] = from.padding[LEFT].clone(),
        BorderTopWidth => to.border_width[TOP] = from.border_width[TOP],
        BorderRightWidth => to.border_width[RIGHT] = from.border_width[RIGHT],
        BorderBottomWidth => to.border_width[BOTTOM] = from.border_width[BOTTOM],
        BorderLeftWidth => to.border_width[LEFT] = from.border_width[LEFT],
        BorderTopColor => to.border_color[TOP] = from.border_color[TOP],
        BorderRightColor => to.border_color[RIGHT] = from.border_color[RIGHT],
        BorderBottomColor => to.border_color[BOTTOM] = from.border_color[BOTTOM],
        BorderLeftColor => to.border_color[LEFT] = from.border_color[LEFT],
        BorderTopStyle => to.border_style[TOP] = from.border_style[TOP],
        BorderRightStyle => to.border_style[RIGHT] = from.border_style[RIGHT],
        BorderBottomStyle => to.border_style[BOTTOM] = from.border_style[BOTTOM],
        BorderLeftStyle => to.border_style[LEFT] = from.border_style[LEFT],
        Overflow => to.overflow = from.overflow,
        BoxSizing => to.box_sizing = from.box_sizing,
        Float => to.float = from.float,
        Clear => to.clear = from.clear,
        Position => to.position = from.position,
        Top => to.inset[TOP] = from.inset[TOP].clone(),
        Right => to.inset[RIGHT] = from.inset[RIGHT].clone(),
        Bottom => to.inset[BOTTOM] = from.inset[BOTTOM].clone(),
        Left => to.inset[LEFT] = from.inset[LEFT].clone(),
        ZIndex => to.z_index = from.z_index,
        TableLayout => to.table_layout = from.table_layout,
        BorderCollapse => to.border_collapse = from.border_collapse,
        BorderSpacing => to.border_spacing = from.border_spacing,
        CaptionSide => to.caption_side = from.caption_side,
        EmptyCells => to.empty_cells = from.empty_cells,
        VerticalAlign => to.vertical_align = from.vertical_align.clone(),
        Content => to.content = from.content.clone(),
        Quotes => to.quotes = from.quotes.clone(),
        FlexDirection => to.flex_direction = from.flex_direction,
        FlexWrap => to.flex_wrap = from.flex_wrap,
        Order => to.order = from.order,
        FlexGrow => to.flex_grow = from.flex_grow,
        FlexShrink => to.flex_shrink = from.flex_shrink,
        FlexBasis => to.flex_basis = from.flex_basis.clone(),
        JustifyContent => to.justify_content = from.justify_content,
        AlignContent => to.align_content = from.align_content,
        AlignItems => to.align_items = from.align_items,
        AlignSelf => to.align_self = from.align_self,
        JustifyItems => copy_group!(to, from, grid, grid, grid_mut, justify_items),
        JustifySelf => copy_group!(to, from, grid, grid, grid_mut, justify_self),
        RowGap => to.row_gap = from.row_gap.clone(),
        ColumnGap => to.column_gap = from.column_gap.clone(),
        GridTemplateColumns => copy_group!(to, from, grid, grid, grid_mut, template_columns),
        GridTemplateRows => copy_group!(to, from, grid, grid, grid_mut, template_rows),
        GridTemplateAreas => copy_group!(to, from, grid, grid, grid_mut, template_areas),
        GridAutoColumns => copy_group!(to, from, grid, grid, grid_mut, auto_columns),
        GridAutoRows => copy_group!(to, from, grid, grid, grid_mut, auto_rows),
        GridAutoFlow => copy_group!(to, from, grid, grid, grid_mut, auto_flow),
        GridRowStart => copy_group!(to, from, grid, grid, grid_mut, row_start),
        GridRowEnd => copy_group!(to, from, grid, grid, grid_mut, row_end),
        GridColumnStart => copy_group!(to, from, grid, grid, grid_mut, column_start),
        GridColumnEnd => copy_group!(to, from, grid, grid, grid_mut, column_end),
        Transform => copy_group!(to, from, effects, effects, effects_mut, transform),
        TransformOrigin => copy_group!(to, from, effects, effects, effects_mut, transform_origin),
        TransformBox => copy_group!(to, from, effects, effects, effects_mut, transform_box),
        Translate => copy_group!(to, from, effects, effects, effects_mut, translate),
        Rotate => copy_group!(to, from, effects, effects, effects_mut, rotate),
        Scale => copy_group!(to, from, effects, effects, effects_mut, scale),
        TransformStyle => copy_group!(to, from, effects, effects, effects_mut, transform_style),
        Perspective => copy_group!(to, from, effects, effects, effects_mut, perspective),
        PerspectiveOrigin => {
            copy_group!(to, from, effects, effects, effects_mut, perspective_origin)
        }
        BackfaceVisibility => {
            copy_group!(to, from, effects, effects, effects_mut, backface_visibility)
        }
        Opacity => copy_group!(to, from, effects, effects, effects_mut, opacity),
        WillChange => copy_group!(to, from, effects, effects, effects_mut, will_change),
        TransitionProperty => {
            copy_group!(to, from, motion, motion, motion_mut, transition_property)
        }
        TransitionDuration => {
            copy_group!(to, from, motion, motion, motion_mut, transition_duration)
        }
        TransitionTimingFunction => {
            copy_group!(
                to,
                from,
                motion,
                motion,
                motion_mut,
                transition_timing_function
            )
        }
        TransitionDelay => copy_group!(to, from, motion, motion, motion_mut, transition_delay),
        TransitionBehavior => {
            copy_group!(to, from, motion, motion, motion_mut, transition_behavior)
        }
        AnimationName => copy_group!(to, from, motion, motion, motion_mut, animation_name),
        AnimationDuration => {
            copy_group!(to, from, motion, motion, motion_mut, animation_duration)
        }
        AnimationTimingFunction => {
            copy_group!(
                to,
                from,
                motion,
                motion,
                motion_mut,
                animation_timing_function
            )
        }
        AnimationIterationCount => {
            copy_group!(
                to,
                from,
                motion,
                motion,
                motion_mut,
                animation_iteration_count
            )
        }
        AnimationDirection => {
            copy_group!(to, from, motion, motion, motion_mut, animation_direction)
        }
        AnimationPlayState => {
            copy_group!(to, from, motion, motion, motion_mut, animation_play_state)
        }
        AnimationDelay => copy_group!(to, from, motion, motion, motion_mut, animation_delay),
        AnimationFillMode => {
            copy_group!(to, from, motion, motion, motion_mut, animation_fill_mode)
        }
        BreakBefore => copy_group!(
            to,
            from,
            fragmentation,
            fragmentation,
            fragmentation_mut,
            break_before
        ),
        BreakAfter => copy_group!(
            to,
            from,
            fragmentation,
            fragmentation,
            fragmentation_mut,
            break_after
        ),
        BreakInside => copy_group!(
            to,
            from,
            fragmentation,
            fragmentation,
            fragmentation_mut,
            break_inside
        ),
        Orphans => copy_group!(
            to,
            from,
            fragmentation,
            fragmentation,
            fragmentation_mut,
            orphans
        ),
        Widows => copy_group!(
            to,
            from,
            fragmentation,
            fragmentation,
            fragmentation_mut,
            widows
        ),
        PrintColorAdjust => copy_group!(
            to,
            from,
            fragmentation,
            fragmentation,
            fragmentation_mut,
            print_color_adjust
        ),
        CounterReset => copy_group!(to, from, lists, lists, lists_mut, counter_reset),
        CounterIncrement => copy_group!(to, from, lists, lists, lists_mut, counter_increment),
        CounterSet => copy_group!(to, from, lists, lists, lists_mut, counter_set),
        ListStyleType => copy_group!(to, from, lists, lists, lists_mut, list_style_type),
        ListStylePosition => copy_group!(to, from, lists, lists, lists_mut, list_style_position),
        ListStyleImage => copy_group!(to, from, lists, lists, lists_mut, list_style_image),
        TextDecorationLine => copy_group!(to, from, decoration, decoration, decoration_mut, line),
        TextDecorationStyle => {
            copy_group!(to, from, decoration, decoration, decoration_mut, style)
        }
        TextDecorationColor => {
            copy_group!(to, from, decoration, decoration, decoration_mut, color)
        }
        TextDecorationThickness => {
            copy_group!(to, from, decoration, decoration, decoration_mut, thickness)
        }
        TextUnderlineOffset => {
            copy_group!(
                to,
                from,
                decoration,
                decoration,
                decoration_mut,
                underline_offset
            )
        }
        TextUnderlinePosition => {
            copy_group!(
                to,
                from,
                decoration,
                decoration,
                decoration_mut,
                underline_position
            )
        }
    }
}

/// Whether `a` and `b` have the same computed value of `property`.
pub fn same(property: Longhand, a: &ComputedStyle, b: &ComputedStyle) -> bool {
    use Longhand::*;
    match property {
        Color => a.color == b.color,
        BackgroundColor => a.background_color == b.background_color,
        FontSize => a.font_size_px == b.font_size_px,
        FontFamily => a.font_family == b.font_family,
        Display => a.display == b.display,
        TextAlignAll => a.text_align == b.text_align,
        TextAlignLast => a.text_align_last == b.text_align_last,
        TextJustify => a.text_justify == b.text_justify,
        TextTransform => a.text_transform == b.text_transform,
        WhiteSpace => a.white_space == b.white_space,
        LetterSpacing => a.letter_spacing == b.letter_spacing,
        WordSpacing => a.word_spacing == b.word_spacing,
        Width => a.width == b.width,
        Height => a.height == b.height,
        MinWidth => a.min_width == b.min_width,
        MinHeight => a.min_height == b.min_height,
        MaxWidth => a.max_width == b.max_width,
        MaxHeight => a.max_height == b.max_height,
        MarginTop => a.margin[TOP] == b.margin[TOP],
        MarginRight => a.margin[RIGHT] == b.margin[RIGHT],
        MarginBottom => a.margin[BOTTOM] == b.margin[BOTTOM],
        MarginLeft => a.margin[LEFT] == b.margin[LEFT],
        PaddingTop => a.padding[TOP] == b.padding[TOP],
        PaddingRight => a.padding[RIGHT] == b.padding[RIGHT],
        PaddingBottom => a.padding[BOTTOM] == b.padding[BOTTOM],
        PaddingLeft => a.padding[LEFT] == b.padding[LEFT],
        BorderTopWidth => a.border_width[TOP] == b.border_width[TOP],
        BorderRightWidth => a.border_width[RIGHT] == b.border_width[RIGHT],
        BorderBottomWidth => a.border_width[BOTTOM] == b.border_width[BOTTOM],
        BorderLeftWidth => a.border_width[LEFT] == b.border_width[LEFT],
        BorderTopColor => a.border_color[TOP] == b.border_color[TOP],
        BorderRightColor => a.border_color[RIGHT] == b.border_color[RIGHT],
        BorderBottomColor => a.border_color[BOTTOM] == b.border_color[BOTTOM],
        BorderLeftColor => a.border_color[LEFT] == b.border_color[LEFT],
        BorderTopStyle => a.border_style[TOP] == b.border_style[TOP],
        BorderRightStyle => a.border_style[RIGHT] == b.border_style[RIGHT],
        BorderBottomStyle => a.border_style[BOTTOM] == b.border_style[BOTTOM],
        BorderLeftStyle => a.border_style[LEFT] == b.border_style[LEFT],
        Overflow => a.overflow == b.overflow,
        BoxSizing => a.box_sizing == b.box_sizing,
        Float => a.float == b.float,
        Clear => a.clear == b.clear,
        Position => a.position == b.position,
        Top => a.inset[TOP] == b.inset[TOP],
        Right => a.inset[RIGHT] == b.inset[RIGHT],
        Bottom => a.inset[BOTTOM] == b.inset[BOTTOM],
        Left => a.inset[LEFT] == b.inset[LEFT],
        ZIndex => a.z_index == b.z_index,
        TableLayout => a.table_layout == b.table_layout,
        BorderCollapse => a.border_collapse == b.border_collapse,
        BorderSpacing => a.border_spacing == b.border_spacing,
        CaptionSide => a.caption_side == b.caption_side,
        EmptyCells => a.empty_cells == b.empty_cells,
        VerticalAlign => a.vertical_align == b.vertical_align,
        Content => a.content == b.content,
        Quotes => a.quotes == b.quotes,
        FlexDirection => a.flex_direction == b.flex_direction,
        FlexWrap => a.flex_wrap == b.flex_wrap,
        Order => a.order == b.order,
        FlexGrow => a.flex_grow == b.flex_grow,
        FlexShrink => a.flex_shrink == b.flex_shrink,
        FlexBasis => a.flex_basis == b.flex_basis,
        JustifyContent => a.justify_content == b.justify_content,
        AlignContent => a.align_content == b.align_content,
        AlignItems => a.align_items == b.align_items,
        AlignSelf => a.align_self == b.align_self,
        JustifyItems => a.grid().justify_items == b.grid().justify_items,
        JustifySelf => a.grid().justify_self == b.grid().justify_self,
        RowGap => a.row_gap == b.row_gap,
        ColumnGap => a.column_gap == b.column_gap,
        GridTemplateColumns => a.grid().template_columns == b.grid().template_columns,
        GridTemplateRows => a.grid().template_rows == b.grid().template_rows,
        GridTemplateAreas => a.grid().template_areas == b.grid().template_areas,
        GridAutoColumns => a.grid().auto_columns == b.grid().auto_columns,
        GridAutoRows => a.grid().auto_rows == b.grid().auto_rows,
        GridAutoFlow => a.grid().auto_flow == b.grid().auto_flow,
        GridRowStart => a.grid().row_start == b.grid().row_start,
        GridRowEnd => a.grid().row_end == b.grid().row_end,
        GridColumnStart => a.grid().column_start == b.grid().column_start,
        GridColumnEnd => a.grid().column_end == b.grid().column_end,
        Transform => a.effects().transform == b.effects().transform,
        TransformOrigin => a.effects().transform_origin == b.effects().transform_origin,
        TransformBox => a.effects().transform_box == b.effects().transform_box,
        Translate => a.effects().translate == b.effects().translate,
        Rotate => a.effects().rotate == b.effects().rotate,
        Scale => a.effects().scale == b.effects().scale,
        TransformStyle => a.effects().transform_style == b.effects().transform_style,
        Perspective => a.effects().perspective == b.effects().perspective,
        PerspectiveOrigin => a.effects().perspective_origin == b.effects().perspective_origin,
        BackfaceVisibility => a.effects().backface_visibility == b.effects().backface_visibility,
        Opacity => a.effects().opacity == b.effects().opacity,
        WillChange => a.effects().will_change == b.effects().will_change,
        TransitionProperty => a.motion().transition_property == b.motion().transition_property,
        TransitionDuration => a.motion().transition_duration == b.motion().transition_duration,
        TransitionTimingFunction => {
            a.motion().transition_timing_function == b.motion().transition_timing_function
        }
        TransitionDelay => a.motion().transition_delay == b.motion().transition_delay,
        TransitionBehavior => a.motion().transition_behavior == b.motion().transition_behavior,
        AnimationName => a.motion().animation_name == b.motion().animation_name,
        AnimationDuration => a.motion().animation_duration == b.motion().animation_duration,
        AnimationTimingFunction => {
            a.motion().animation_timing_function == b.motion().animation_timing_function
        }
        AnimationIterationCount => {
            a.motion().animation_iteration_count == b.motion().animation_iteration_count
        }
        AnimationDirection => a.motion().animation_direction == b.motion().animation_direction,
        AnimationPlayState => a.motion().animation_play_state == b.motion().animation_play_state,
        AnimationDelay => a.motion().animation_delay == b.motion().animation_delay,
        AnimationFillMode => a.motion().animation_fill_mode == b.motion().animation_fill_mode,
        BreakBefore => a.fragmentation().break_before == b.fragmentation().break_before,
        BreakAfter => a.fragmentation().break_after == b.fragmentation().break_after,
        BreakInside => a.fragmentation().break_inside == b.fragmentation().break_inside,
        Orphans => a.fragmentation().orphans == b.fragmentation().orphans,
        Widows => a.fragmentation().widows == b.fragmentation().widows,
        PrintColorAdjust => {
            a.fragmentation().print_color_adjust == b.fragmentation().print_color_adjust
        }
        CounterReset => a.lists().counter_reset == b.lists().counter_reset,
        CounterIncrement => a.lists().counter_increment == b.lists().counter_increment,
        CounterSet => a.lists().counter_set == b.lists().counter_set,
        ListStyleType => a.lists().list_style_type == b.lists().list_style_type,
        ListStylePosition => a.lists().list_style_position == b.lists().list_style_position,
        ListStyleImage => a.lists().list_style_image == b.lists().list_style_image,
        TextDecorationLine => a.decoration().line == b.decoration().line,
        TextDecorationStyle => a.decoration().style == b.decoration().style,
        TextDecorationColor => a.decoration().color == b.decoration().color,
        TextDecorationThickness => a.decoration().thickness == b.decoration().thickness,
        TextUnderlineOffset => a.decoration().underline_offset == b.decoration().underline_offset,
        TextUnderlinePosition => {
            a.decoration().underline_position == b.decoration().underline_position
        }
    }
}

/// `property`'s value `p` of the way from `a` to `b` into `to`, by its
/// animation type: interpolated where the two values can be, discretely
/// otherwise, `display` by Display 4 §2.9. Nothing for a property that
/// is not animatable. `p` outside [0, 1] goes on past the values where
/// they interpolate.
pub fn blend(
    property: Longhand,
    a: &ComputedStyle,
    b: &ComputedStyle,
    p: f64,
    to: &mut ComputedStyle,
) {
    match property.animation_type() {
        AnimationType::NotAnimatable => {}
        AnimationType::ByComputedValue if interpolate(property, a, b, p, to) => {}
        _ if property == Longhand::Display
            && (a.display == computed::Display::None) != (b.display == computed::Display::None) =>
        {
            // The value other than `none` strictly between the ends.
            let other = if a.display == computed::Display::None {
                b
            } else {
                a
            };
            to.display = if p <= 0.0 {
                a.display
            } else if p >= 1.0 {
                b.display
            } else {
                other.display
            };
        }
        _ => copy(property, if p < 0.5 { a } else { b }, to),
    }
}

/// Whether `property`'s values in `a` and `b` interpolate rather than
/// flip, as a transition asks of them (Transitions 1 §3, Transitions 2
/// §2.5: a discrete pair transitions only with `allow-discrete`).
pub fn interpolable(property: Longhand, a: &ComputedStyle, b: &ComputedStyle) -> bool {
    if property.animation_type() != AnimationType::ByComputedValue {
        return false;
    }
    let mut scratch = a.clone();
    interpolate(property, a, b, 0.5, &mut scratch)
}

fn lerp(a: f64, b: f64, p: f64) -> f64 {
    a + (b - a) * p
}

/// A number interpolated, within `min` and `max`.
fn number(a: f32, b: f32, p: f64, min: f32, max: f32) -> f32 {
    let v = lerp(f64::from(a), f64::from(b), p);
    if v.is_nan() {
        return a;
    }
    v.clamp(f64::from(min), f64::from(max)) as f32
}

/// A length in pixels interpolated, within the lengths the engine holds
/// and, for a property that takes none, not below zero.
fn length(a: f32, b: f32, p: f64, non_negative: bool) -> f32 {
    number(
        a,
        b,
        p,
        if non_negative { 0.0 } else { -MAX_LENGTH_PX },
        MAX_LENGTH_PX,
    )
}

/// An integer interpolated: the nearest, a half towards positive
/// infinity (Values 4 §3.1).
fn integer(a: i64, b: i64, p: f64, min: i64, max: i64) -> i64 {
    let v = (lerp(a as f64, b as f64, p) + 0.5).floor();
    if v.is_nan() {
        return a;
    }
    (v.clamp(min as f64, max as f64)) as i64
}

/// A colour interpolated in sRGB with premultiplied alpha (Color 4
/// §13.4): the alpha linearly, each channel premultiplied by its alpha
/// and divided by the result's, a transparent result black.
pub fn color(a: computed::Color, b: computed::Color, p: f64) -> computed::Color {
    let alpha = number(a.a, b.a, p, 0.0, 1.0);
    if alpha == 0.0 {
        return computed::Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        };
    }
    let channel = |x: f32, y: f32| {
        let v = lerp(f64::from(x * a.a), f64::from(y * b.a), p) / f64::from(alpha);
        if v.is_nan() {
            0.0
        } else {
            v.clamp(0.0, 1.0) as f32
        }
    };
    computed::Color {
        r: channel(a.r, b.r),
        g: channel(a.g, b.g),
        b: channel(a.b, b.b),
        a: alpha,
    }
}

/// A length-percentage as `px + pct%` when it is one: a sum of lengths
/// and percentages scaled by numbers, which is every value but one with
/// a `min()`, `max()` or `clamp()` of a percentage.
fn linear(value: &LengthPercentage) -> Option<(f64, f64)> {
    match value {
        LengthPercentage::Px(v) => Some((f64::from(*v), 0.0)),
        LengthPercentage::Percent(p) => Some((0.0, f64::from(*p))),
        LengthPercentage::Calc(calc) => linear_expression(calc.expression()),
    }
}

fn linear_expression(e: &Expression) -> Option<(f64, f64)> {
    match e {
        Expression::Value(v) => Some((f64::from(*v), 0.0)),
        Expression::Percent(p) => Some((0.0, f64::from(*p))),
        Expression::Sum(children) => children.iter().try_fold((0.0, 0.0), |(px, pct), child| {
            let (x, y) = linear_expression(child)?;
            Some((px + x, pct + y))
        }),
        Expression::Negate(child) => linear_expression(child).map(|(x, y)| (-x, -y)),
        Expression::Product(children) => {
            // A number times at most one sum with a percentage.
            let mut factor = 1.0;
            let mut form: Option<(f64, f64)> = None;
            for child in children.iter() {
                let (x, y) = linear_expression(child)?;
                if y == 0.0 {
                    factor *= x;
                } else if form.is_none() {
                    form = Some((x, y));
                } else {
                    return None;
                }
            }
            let (x, y) = form.unwrap_or((1.0, 0.0));
            Some((factor * x, factor * y))
        }
        Expression::Invert(child) => match linear_expression(child)? {
            (x, 0.0) => Some((1.0 / x, 0.0)),
            _ => None,
        },
        Expression::Min(_) | Expression::Max(_) | Expression::Clamp(_) => {
            // Constant only when no percentage is in it, which a
            // computed value folds: what is left has one.
            None
        }
    }
}

/// The value of `px + pct%`: a length, a percentage, or the sum of both.
fn from_linear(px: f64, pct: f64, non_negative: bool) -> LengthPercentage {
    let px = if px.is_nan() { 0.0 } else { px };
    let pct = if pct.is_nan() { 0.0 } else { pct };
    let max = f64::from(MAX_LENGTH_PX);
    let px_value = px.clamp(-max, max) as f32;
    let pct_value = pct.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
    if pct_value == 0.0 {
        LengthPercentage::Px(if non_negative {
            px_value.max(0.0)
        } else {
            px_value
        })
    } else if px_value == 0.0 {
        LengthPercentage::Percent(if non_negative {
            pct_value.max(0.0)
        } else {
            pct_value
        })
    } else {
        LengthPercentage::Calc(Arc::new(Calc::new(
            Expression::Sum(Box::new([
                Expression::Value(px_value),
                Expression::Percent(pct_value),
            ])),
            non_negative,
        )))
    }
}

fn expression_of(value: &LengthPercentage) -> Expression {
    match value {
        LengthPercentage::Px(v) => Expression::Value(*v),
        LengthPercentage::Percent(p) => Expression::Percent(*p),
        LengthPercentage::Calc(calc) => calc.expression().clone(),
    }
}

fn depth(e: &Expression) -> usize {
    1 + match e {
        Expression::Value(_) | Expression::Percent(_) => 0,
        Expression::Sum(children)
        | Expression::Product(children)
        | Expression::Min(children)
        | Expression::Max(children) => children.iter().map(depth).max().unwrap_or(0),
        Expression::Negate(child) | Expression::Invert(child) => depth(child),
        Expression::Clamp(parts) => parts.iter().map(depth).max().unwrap_or(0),
    }
}

/// A `<length-percentage>` interpolated (Values 4 §3.1): two lengths or
/// two percentages as numbers, any two sums of a length and a
/// percentage as such a sum, and anything else as the math function
/// `(1 - p) * A + p * B`, unless that would nest deeper than a math
/// function may, when it does not interpolate.
pub fn length_percentage(
    a: &LengthPercentage,
    b: &LengthPercentage,
    p: f64,
    non_negative: bool,
) -> Option<LengthPercentage> {
    match (a, b) {
        (LengthPercentage::Px(x), LengthPercentage::Px(y)) => {
            return Some(LengthPercentage::Px(length(*x, *y, p, non_negative)));
        }
        (LengthPercentage::Percent(x), LengthPercentage::Percent(y)) => {
            let min = if non_negative { 0.0 } else { -f32::MAX };
            return Some(LengthPercentage::Percent(number(*x, *y, p, min, f32::MAX)));
        }
        _ => {}
    }
    if let (Some((ax, ay)), Some((bx, by))) = (linear(a), linear(b)) {
        return Some(from_linear(lerp(ax, bx, p), lerp(ay, by, p), non_negative));
    }
    let expression = Expression::Sum(Box::new([
        Expression::Product(Box::new([
            Expression::Value((1.0 - p) as f32),
            expression_of(a),
        ])),
        Expression::Product(Box::new([Expression::Value(p as f32), expression_of(b)])),
    ]));
    (depth(&expression) <= MAX_DEPTH)
        .then(|| LengthPercentage::Calc(Arc::new(Calc::new(expression, non_negative))))
}

/// A size's length or percentage, when it has one.
fn size_value(size: &Size) -> Option<LengthPercentage> {
    match size {
        Size::Px(v) => Some(LengthPercentage::Px(*v)),
        Size::Percent(p) => Some(LengthPercentage::Percent(*p)),
        Size::Calc(c) => Some(LengthPercentage::Calc(Arc::clone(c))),
        _ => None,
    }
}

fn size_of(value: LengthPercentage) -> Size {
    match value {
        LengthPercentage::Px(v) => Size::Px(v),
        LengthPercentage::Percent(p) => Size::Percent(p),
        LengthPercentage::Calc(c) => Size::Calc(c),
    }
}

/// A size interpolated, both of them lengths or percentages; never
/// below zero.
fn size(a: &Size, b: &Size, p: f64) -> Option<Size> {
    let value = length_percentage(&size_value(a)?, &size_value(b)?, p, true)?;
    Some(size_of(value))
}

fn max_size(a: &MaxSize, b: &MaxSize, p: f64) -> Option<MaxSize> {
    let value = |m: &MaxSize| match m {
        MaxSize::Px(v) => Some(LengthPercentage::Px(*v)),
        MaxSize::Percent(p) => Some(LengthPercentage::Percent(*p)),
        MaxSize::Calc(c) => Some(LengthPercentage::Calc(Arc::clone(c))),
        _ => None,
    };
    Some(match length_percentage(&value(a)?, &value(b)?, p, true)? {
        LengthPercentage::Px(v) => MaxSize::Px(v),
        LengthPercentage::Percent(p) => MaxSize::Percent(p),
        LengthPercentage::Calc(c) => MaxSize::Calc(c),
    })
}

fn auto_value(value: &LengthPercentageAuto) -> Option<LengthPercentage> {
    match value {
        LengthPercentageAuto::Auto => None,
        LengthPercentageAuto::Px(v) => Some(LengthPercentage::Px(*v)),
        LengthPercentageAuto::Percent(p) => Some(LengthPercentage::Percent(*p)),
        LengthPercentageAuto::Calc(c) => Some(LengthPercentage::Calc(Arc::clone(c))),
    }
}

/// A margin or an inset interpolated, both of them other than `auto`.
fn length_percentage_auto(
    a: &LengthPercentageAuto,
    b: &LengthPercentageAuto,
    p: f64,
) -> Option<LengthPercentageAuto> {
    Some(
        match length_percentage(&auto_value(a)?, &auto_value(b)?, p, false)? {
            LengthPercentage::Px(v) => LengthPercentageAuto::Px(v),
            LengthPercentage::Percent(p) => LengthPercentageAuto::Percent(p),
            LengthPercentage::Calc(c) => LengthPercentageAuto::Calc(c),
        },
    )
}

fn vertical_align_value(value: &computed::VerticalAlign) -> Option<LengthPercentage> {
    match value {
        computed::VerticalAlign::Length(v) => Some(LengthPercentage::Px(*v)),
        computed::VerticalAlign::Percent(p) => Some(LengthPercentage::Percent(*p)),
        computed::VerticalAlign::Calc(c) => Some(LengthPercentage::Calc(Arc::clone(c))),
        _ => None,
    }
}

/// One track size interpolated, the two of one kind (Grid 2 §7.2.3.3).
fn track_breadth(a: &TrackBreadth, b: &TrackBreadth, p: f64) -> Option<TrackBreadth> {
    match (a, b) {
        (TrackBreadth::Fixed(x), TrackBreadth::Fixed(y)) => {
            Some(TrackBreadth::Fixed(length_percentage(x, y, p, true)?))
        }
        (TrackBreadth::Flex(x), TrackBreadth::Flex(y)) => {
            Some(TrackBreadth::Flex(number(*x, *y, p, 0.0, f32::MAX)))
        }
        (x, y) if x == y => Some(x.clone()),
        _ => None,
    }
}

fn track_size(a: &TrackSize, b: &TrackSize, p: f64) -> Option<TrackSize> {
    match (a, b) {
        (TrackSize::Breadth(x), TrackSize::Breadth(y)) => {
            Some(TrackSize::Breadth(track_breadth(x, y, p)?))
        }
        (TrackSize::MinMax(x0, x1), TrackSize::MinMax(y0, y1)) => Some(TrackSize::MinMax(
            track_breadth(x0, y0, p)?,
            track_breadth(x1, y1, p)?,
        )),
        (TrackSize::FitContent(x), TrackSize::FitContent(y)) => {
            Some(TrackSize::FitContent(length_percentage(x, y, p, true)?))
        }
        _ => None,
    }
}

fn track_sizes(a: &[TrackSize], b: &[TrackSize], p: f64) -> Option<Vec<TrackSize>> {
    if a.len() != b.len() {
        return None;
    }
    a.iter().zip(b).map(|(x, y)| track_size(x, y, p)).collect()
}

/// A track list interpolated (Grid 2 §7.2.3.3): the same tracks and
/// repetitions, size by size, the line names the nearer list's; `none`
/// only with `none`.
fn track_list(a: &Option<TrackList>, b: &Option<TrackList>, p: f64) -> Option<Option<TrackList>> {
    let (Some(a), Some(b)) = (a, b) else {
        return (a.is_none() && b.is_none()).then_some(None);
    };
    if a.sections.len() != b.sections.len() {
        return None;
    }
    let names = if p < 0.5 { a } else { b };
    let sections = a
        .sections
        .iter()
        .zip(&b.sections)
        .zip(&names.sections)
        .map(|((x, y), named)| match (x, y, named) {
            (TrackSection::Track(x), TrackSection::Track(y), _) => {
                Some(TrackSection::Track(track_size(x, y, p)?))
            }
            (TrackSection::Repeat(x), TrackSection::Repeat(y), TrackSection::Repeat(named))
                if x.count == y.count =>
            {
                let mut repeat = named.clone();
                repeat.tracks = track_sizes(&x.tracks, &y.tracks, p)?;
                Some(TrackSection::Repeat(repeat))
            }
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Some(TrackList {
        names: names.names.clone(),
        sections,
    }))
}

/// Interpolates `property` from `a` to `b` at `p` into `to`; says
/// whether it could, a pair it cannot being left for a discrete step.
pub fn interpolate(
    property: Longhand,
    a: &ComputedStyle,
    b: &ComputedStyle,
    p: f64,
    to: &mut ComputedStyle,
) -> bool {
    use Longhand::*;
    /// Sets `$target` to the interpolation when there is one.
    macro_rules! set {
        ($target:expr, $value:expr) => {
            match $value {
                Some(value) => {
                    $target = value;
                    true
                }
                None => false,
            }
        };
    }
    let side = |s: usize, to: &mut ComputedStyle| -> bool {
        set!(
            to.margin[s],
            length_percentage_auto(&a.margin[s], &b.margin[s], p)
        )
    };
    match property {
        Color => {
            to.color = color(a.color, b.color, p);
            true
        }
        BackgroundColor => {
            to.background_color = color(a.background_color, b.background_color, p);
            true
        }
        FontSize => {
            to.font_size_px = length(a.font_size_px, b.font_size_px, p, true);
            true
        }
        LetterSpacing => {
            to.letter_spacing = length(a.letter_spacing, b.letter_spacing, p, false);
            true
        }
        WordSpacing => {
            to.word_spacing = length(a.word_spacing, b.word_spacing, p, false);
            true
        }
        Width => set!(to.width, size(&a.width, &b.width, p)),
        Height => set!(to.height, size(&a.height, &b.height, p)),
        MinWidth => set!(to.min_width, size(&a.min_width, &b.min_width, p)),
        MinHeight => set!(to.min_height, size(&a.min_height, &b.min_height, p)),
        MaxWidth => set!(to.max_width, max_size(&a.max_width, &b.max_width, p)),
        MaxHeight => set!(to.max_height, max_size(&a.max_height, &b.max_height, p)),
        MarginTop => side(TOP, to),
        MarginRight => side(RIGHT, to),
        MarginBottom => side(BOTTOM, to),
        MarginLeft => side(LEFT, to),
        PaddingTop | PaddingRight | PaddingBottom | PaddingLeft => {
            let s = match property {
                PaddingTop => TOP,
                PaddingRight => RIGHT,
                PaddingBottom => BOTTOM,
                _ => LEFT,
            };
            set!(
                to.padding[s],
                length_percentage(&a.padding[s], &b.padding[s], p, true)
            )
        }
        BorderTopWidth | BorderRightWidth | BorderBottomWidth | BorderLeftWidth => {
            let s = match property {
                BorderTopWidth => TOP,
                BorderRightWidth => RIGHT,
                BorderBottomWidth => BOTTOM,
                _ => LEFT,
            };
            to.border_width[s] = length(a.border_width[s], b.border_width[s], p, true);
            true
        }
        BorderTopColor | BorderRightColor | BorderBottomColor | BorderLeftColor => {
            let s = match property {
                BorderTopColor => TOP,
                BorderRightColor => RIGHT,
                BorderBottomColor => BOTTOM,
                _ => LEFT,
            };
            to.border_color[s] = color(a.border_color[s], b.border_color[s], p);
            true
        }
        Top | Right | Bottom | Left => {
            let s = match property {
                Top => TOP,
                Right => RIGHT,
                Bottom => BOTTOM,
                _ => LEFT,
            };
            set!(
                to.inset[s],
                length_percentage_auto(&a.inset[s], &b.inset[s], p)
            )
        }
        ZIndex => match (a.z_index, b.z_index) {
            (Some(x), Some(y)) => {
                to.z_index =
                    Some(integer(x.into(), y.into(), p, i32::MIN.into(), i32::MAX.into()) as i32);
                true
            }
            _ => false,
        },
        BorderSpacing => {
            to.border_spacing.horizontal = length(
                a.border_spacing.horizontal,
                b.border_spacing.horizontal,
                p,
                true,
            );
            to.border_spacing.vertical = length(
                a.border_spacing.vertical,
                b.border_spacing.vertical,
                p,
                true,
            );
            true
        }
        VerticalAlign => {
            let (Some(x), Some(y)) = (
                vertical_align_value(&a.vertical_align),
                vertical_align_value(&b.vertical_align),
            ) else {
                return false;
            };
            set!(
                to.vertical_align,
                length_percentage(&x, &y, p, false).map(|value| match value {
                    LengthPercentage::Px(v) => computed::VerticalAlign::Length(v),
                    LengthPercentage::Percent(p) => computed::VerticalAlign::Percent(p),
                    LengthPercentage::Calc(c) => computed::VerticalAlign::Calc(c),
                })
            )
        }
        Order => {
            to.order = integer(
                a.order.into(),
                b.order.into(),
                p,
                i32::MIN.into(),
                i32::MAX.into(),
            ) as i32;
            true
        }
        FlexGrow => {
            to.flex_grow = number(a.flex_grow, b.flex_grow, p, 0.0, f32::MAX);
            true
        }
        FlexShrink => {
            to.flex_shrink = number(a.flex_shrink, b.flex_shrink, p, 0.0, f32::MAX);
            true
        }
        FlexBasis => match (&a.flex_basis, &b.flex_basis) {
            (computed::FlexBasis::Size(x), computed::FlexBasis::Size(y)) => {
                set!(to.flex_basis, size(x, y, p).map(computed::FlexBasis::Size))
            }
            _ => false,
        },
        RowGap | ColumnGap => {
            let (x, y) = if property == RowGap {
                (&a.row_gap, &b.row_gap)
            } else {
                (&a.column_gap, &b.column_gap)
            };
            let (Some(x), Some(y)) = (x, y) else {
                return false;
            };
            let Some(value) = length_percentage(x, y, p, true) else {
                return false;
            };
            if property == RowGap {
                to.row_gap = Some(value);
            } else {
                to.column_gap = Some(value);
            }
            true
        }
        GridTemplateColumns => set!(
            to.grid_mut().template_columns,
            track_list(&a.grid().template_columns, &b.grid().template_columns, p)
        ),
        GridTemplateRows => set!(
            to.grid_mut().template_rows,
            track_list(&a.grid().template_rows, &b.grid().template_rows, p)
        ),
        GridAutoColumns => set!(
            to.grid_mut().auto_columns,
            track_sizes(&a.grid().auto_columns, &b.grid().auto_columns, p)
        ),
        GridAutoRows => set!(
            to.grid_mut().auto_rows,
            track_sizes(&a.grid().auto_rows, &b.grid().auto_rows, p)
        ),
        Transform => set!(
            to.effects_mut().transform,
            transform::lists(&a.effects().transform, &b.effects().transform, p)
        ),
        TransformOrigin => {
            let (x, y) = (&a.effects().transform_origin, &b.effects().transform_origin);
            let (Some(ox), Some(oy)) = (
                length_percentage(&x.x, &y.x, p, false),
                length_percentage(&x.y, &y.y, p, false),
            ) else {
                return false;
            };
            let origin = &mut to.effects_mut().transform_origin;
            origin.x = ox;
            origin.y = oy;
            origin.z = length(x.z, y.z, p, false);
            true
        }
        PerspectiveOrigin => {
            let (x, y) = (
                &a.effects().perspective_origin,
                &b.effects().perspective_origin,
            );
            let (Some(ox), Some(oy)) = (
                length_percentage(&x.x, &y.x, p, false),
                length_percentage(&x.y, &y.y, p, false),
            ) else {
                return false;
            };
            let origin = &mut to.effects_mut().perspective_origin;
            origin.x = ox;
            origin.y = oy;
            true
        }
        Translate => set!(
            to.effects_mut().translate,
            transform::translate(&a.effects().translate, &b.effects().translate, p)
        ),
        Rotate => set!(
            to.effects_mut().rotate,
            transform::rotate(&a.effects().rotate, &b.effects().rotate, p)
        ),
        Scale => {
            to.effects_mut().scale = transform::scale(&a.effects().scale, &b.effects().scale, p);
            true
        }
        Perspective => match (a.effects().perspective, b.effects().perspective) {
            (Some(x), Some(y)) => {
                to.effects_mut().perspective = Some(length(x, y, p, true));
                true
            }
            _ => false,
        },
        Opacity => {
            to.effects_mut().opacity =
                number(a.effects().opacity, b.effects().opacity, p, 0.0, 1.0);
            true
        }
        Orphans | Widows => {
            let (x, y) = if property == Orphans {
                (a.fragmentation().orphans, b.fragmentation().orphans)
            } else {
                (a.fragmentation().widows, b.fragmentation().widows)
            };
            let value = integer(x.into(), y.into(), p, 1, u32::MAX.into()) as u32;
            if property == Orphans {
                to.fragmentation_mut().orphans = value;
            } else {
                to.fragmentation_mut().widows = value;
            }
            true
        }
        TextDecorationColor => {
            // `currentcolor`, the initial value, is the style's colour.
            let x = a.decoration().color.unwrap_or(a.color);
            let y = b.decoration().color.unwrap_or(b.color);
            to.decoration_mut().color = Some(color(x, y, p));
            true
        }
        TextDecorationThickness => match (&a.decoration().thickness, &b.decoration().thickness) {
            (DecorationThickness::Length(x), DecorationThickness::Length(y)) => set!(
                to.decoration_mut().thickness,
                length_percentage(x, y, p, false).map(DecorationThickness::Length)
            ),
            _ => false,
        },
        TextUnderlineOffset => {
            match (
                &a.decoration().underline_offset,
                &b.decoration().underline_offset,
            ) {
                (Some(x), Some(y)) => set!(
                    to.decoration_mut().underline_offset,
                    length_percentage(x, y, p, false).map(Some)
                ),
                _ => false,
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cascade::{CascadeContext, cascade_document};
    use crate::computed::{Rotate, TransformFunction};
    use crate::transform::{Matrix, list_matrix};
    use dom::Document;

    /// The cosine of an eighth turn, as the reference browser prints
    /// 0.707107.
    const R: f32 = std::f32::consts::FRAC_1_SQRT_2;

    /// The computed style of a `div` whose `style` attribute is `css`.
    fn style(css: &str) -> ComputedStyle {
        let html = format!("<div style=\"{css}\"></div>");
        let document = Document::from_node(&html::parse_document(&html).expect("never fails"));
        let styles = cascade_document(&document, &[], &CascadeContext::default());
        let div = document
            .subtree(document.root())
            .find(|&node| document.tag(node) == Some("div"))
            .expect("the div");
        styles.get(div).expect("styled").clone()
    }

    /// `property` `p` of the way from the style of `from` to that of `to`.
    fn blended(property: Longhand, from: &str, to: &str, p: f64) -> ComputedStyle {
        let (a, b) = (style(from), style(to));
        let mut out = a.clone();
        blend(property, &a, &b, p, &mut out);
        out
    }

    fn near(a: &Matrix, b: &Matrix) -> bool {
        a.0.iter()
            .zip(b.0.iter())
            .all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn the_table_names_every_longhand_once() {
        for &longhand in Longhand::ALL {
            assert_eq!(Longhand::from_name(longhand.name()), Some(longhand));
            assert_eq!(longhands_of(longhand.name()), vec![longhand]);
        }
        let mut names: Vec<_> = Longhand::ALL.iter().map(|l| l.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Longhand::ALL.len());
        // Every longhand is one the cascade takes: its initial value
        // written as `initial` is a declaration the cascade applies.
        for &longhand in Longhand::ALL {
            assert!(
                crate::cascade::supports(longhand.name(), &css::tokenize("initial")),
                "{}",
                longhand.name()
            );
        }
        assert_eq!(longhands_of("border").len(), 12);
        assert_eq!(
            longhands_of("margin"),
            vec![
                Longhand::MarginTop,
                Longhand::MarginRight,
                Longhand::MarginBottom,
                Longhand::MarginLeft
            ]
        );
        assert!(longhands_of("unknown").is_empty());
        assert_eq!(
            Longhand::WillChange.animation_type(),
            AnimationType::NotAnimatable
        );
        assert!(!Longhand::AnimationName.is_animatable());
        assert_eq!(Longhand::Position.animation_type(), AnimationType::Discrete);
    }

    #[test]
    fn copy_and_same_read_one_property_each() {
        // Two styles that differ in every property they can.
        let a = style(
            "color: red; background-color: blue; font-size: 20px; font-family: serif; \
             display: flex; text-align: right; text-transform: uppercase; white-space: pre; \
             letter-spacing: 1px; word-spacing: 2px; width: 10px; height: 20px; \
             min-width: 1px; min-height: 2px; max-width: 30px; max-height: 40px; \
             margin: 1px 2px 3px 4px; padding: 1px 2px 3px 4px; border-width: 1px 2px 3px 4px; \
             border-color: red green blue black; border-style: solid dashed dotted double; \
             overflow: hidden; box-sizing: border-box; float: left; clear: both; \
             position: relative; top: 1px; right: 2px; bottom: 3px; left: 4px; z-index: 3; \
             table-layout: fixed; border-collapse: collapse; border-spacing: 3px; \
             caption-side: bottom; empty-cells: hide; vertical-align: top; quotes: none; \
             flex-direction: column; flex-wrap: wrap; order: 2; flex-grow: 2; flex-shrink: 3; \
             flex-basis: 10px; justify-content: center; align-content: center; \
             align-items: center; align-self: center; justify-items: center; \
             justify-self: center; row-gap: 1px; column-gap: 2px; \
             grid-template-columns: 10px; grid-template-rows: 20px; \
             grid-template-areas: 'a'; grid-auto-columns: 5px; grid-auto-rows: 6px; \
             grid-auto-flow: column; grid-row: 1 / 2; grid-column: 2 / 3; \
             transform: rotate(10deg); transform-origin: 1px 2px; transform-box: border-box; \
             translate: 1px; rotate: 10deg; scale: 2; transform-style: preserve-3d; \
             perspective: 10px; perspective-origin: 1px 2px; backface-visibility: hidden; \
             opacity: 0.5; will-change: transform; transition: opacity 1s; \
             animation: a 2s; break-before: page; break-after: page; break-inside: avoid; \
             orphans: 3; widows: 4; print-color-adjust: exact; counter-reset: a; \
             counter-increment: a; counter-set: a; list-style: square inside; \
             text-decoration: underline wavy red 2px; text-underline-offset: 3px; \
             text-underline-position: under; content: 'x'; transition-behavior: allow-discrete",
        );
        let b = ComputedStyle::default();
        for &property in Longhand::ALL {
            assert!(same(property, &a, &a), "{}", property.name());
            let mut out = b.clone();
            copy(property, &a, &mut out);
            assert!(same(property, &out, &a), "{} not copied", property.name());
            for &other in Longhand::ALL {
                if other != property {
                    assert!(
                        same(other, &out, &b),
                        "copying {} changed {}",
                        property.name(),
                        other.name()
                    );
                }
            }
        }
    }

    #[test]
    fn values_interpolate_by_type_as_the_reference_browser_has_them() {
        // Colours with premultiplied alpha: red, not purple, half way to
        // transparent blue.
        let c = blended(Longhand::Color, "color: #ff0000", "color: #0000ff00", 0.5);
        assert_eq!(
            (c.color.r, c.color.g, c.color.b, c.color.a),
            (1.0, 0.0, 0.0, 0.5)
        );
        // A length and a percentage: 205px of an 800px basis half way.
        let w = blended(Longhand::Width, "width: 10px", "width: 50%", 0.5);
        let Size::Calc(calc) = &w.width else {
            panic!("a sum: {:?}", w.width);
        };
        assert_eq!(calc.resolve(800.0), 205.0);
        // At its ends, the values themselves.
        assert_eq!(
            blended(Longhand::Width, "width: 10px", "width: 50%", 0.0).width,
            Size::Px(10.0)
        );
        assert_eq!(
            blended(Longhand::Width, "width: 10px", "width: 50%", 1.0).width,
            Size::Percent(50.0)
        );
        // Integers round, a half up.
        let z = |p| blended(Longhand::ZIndex, "z-index: 1", "z-index: 2", p).z_index;
        assert_eq!((z(0.4), z(0.5), z(0.6)), (Some(1), Some(2), Some(2)));
        // `auto` and an integer flip, as the reference browser has them.
        let auto = |p| blended(Longhand::ZIndex, "z-index: auto", "z-index: 10", p).z_index;
        assert_eq!((auto(0.25), auto(0.5)), (None, Some(10)));
        // Clamped: an opacity no higher than 1, a width no lower than 0.
        let o = blended(Longhand::Opacity, "opacity: 0", "opacity: 1", 1.625);
        assert_eq!(o.effects().opacity, 1.0);
        let w = blended(Longhand::Width, "width: 100px", "width: 0px", 1.625);
        assert_eq!(w.width, Size::Px(0.0));
        // A margin may go below zero.
        let m = blended(
            Longhand::MarginLeft,
            "margin-left: 0px",
            "margin-left: 10px",
            -1.0,
        );
        assert_eq!(m.margin[LEFT], LengthPercentageAuto::Px(-10.0));
        // Discrete: `position` flips half way.
        let pos = |p| {
            blended(
                Longhand::Position,
                "position: static",
                "position: absolute",
                p,
            )
            .position
        };
        assert_ne!(pos(0.49), pos(0.5));
        assert_eq!(pos(0.5), style("position: absolute").position);
        // `display` keeps the value other than `none` strictly between.
        let d = |p| blended(Longhand::Display, "display: block", "display: none", p).display;
        assert_eq!(d(0.0), computed::Display::Block);
        assert_eq!(d(0.75), computed::Display::Block);
        assert_eq!(d(1.0), computed::Display::None);
        let shown = |p| blended(Longhand::Display, "display: none", "display: flex", p).display;
        assert_eq!(shown(0.25), computed::Display::Flex);
        assert_eq!(shown(0.0), computed::Display::None);
        // Not animatable: untouched.
        let n = blended(
            Longhand::WillChange,
            "will-change: auto",
            "will-change: transform",
            0.75,
        );
        assert_eq!(n.effects().will_change, style("").effects().will_change);
        // Grid tracks of one shape, size by size; of two, discretely.
        let g = blended(
            Longhand::GridTemplateColumns,
            "grid-template-columns: 10px 20px",
            "grid-template-columns: 30px 40px",
            0.5,
        );
        assert_eq!(
            g.grid().template_columns,
            style("grid-template-columns: 20px 30px")
                .grid()
                .template_columns
        );
        assert!(!interpolable(
            Longhand::GridTemplateColumns,
            &style("grid-template-columns: 10px 20px"),
            &style("grid-template-columns: 30px 40px 50px"),
        ));
        assert!(interpolable(
            Longhand::Width,
            &style("width: 1px"),
            &style("width: 50%")
        ));
        assert!(!interpolable(
            Longhand::Width,
            &style("width: auto"),
            &style("width: 5px")
        ));
        assert!(!interpolable(
            Longhand::Position,
            &style(""),
            &style("position: fixed")
        ));
        // `currentcolor` as the style's colour.
        let t = blended(
            Longhand::TextDecorationColor,
            "color: #000000",
            "text-decoration-color: #ff0000",
            0.5,
        );
        assert_eq!(t.decoration().color.map(|c| c.r), Some(0.5));
    }

    #[test]
    fn a_sum_of_a_length_and_a_percentage_does_not_grow() {
        // A transition interrupted over and over starts each time from
        // the last value: the sum stays a length and a percentage.
        let mut value = LengthPercentage::Px(10.0);
        for i in 0..1000 {
            let target = if i % 2 == 0 {
                LengthPercentage::Percent(50.0)
            } else {
                LengthPercentage::Px(-20.0)
            };
            value = length_percentage(&value, &target, 0.3, false).expect("interpolates");
        }
        let LengthPercentage::Calc(calc) = &value else {
            panic!("a sum: {value:?}");
        };
        assert!(depth(calc.expression()) <= 2);
        // `min()` of a percentage is not a sum: it nests, up to the
        // depth a math function may have, and no further.
        let tokens = css::tokenize("min(10px, 50%)");
        let parsed = crate::calc::parse(&tokens, crate::calc::Want::LengthPercentage).unwrap();
        struct Pixels;
        impl crate::calc::Lengths for Pixels {
            fn px(&self, value: f64, _: crate::calc::LengthUnit) -> f64 {
                value
            }
        }
        let crate::calc::Computed::Expression(e) = parsed.compute(&Pixels, None) else {
            panic!("an expression");
        };
        let mut value = LengthPercentage::Calc(Arc::new(Calc::new(e, false)));
        let mut steps = 0;
        while let Some(next) = length_percentage(&value, &LengthPercentage::Px(1.0), 0.5, false) {
            value = next;
            steps += 1;
            assert!(steps < 100);
        }
        assert!(steps > 5, "{steps}");
    }

    /// The matrix of the `transform` `p` of the way, over a box 100px
    /// square, as `getComputedStyle` reads it.
    fn transform(from: &str, to: &str, p: f64) -> Matrix {
        let out = blended(
            Longhand::Transform,
            &format!("transform: {from}"),
            &format!("transform: {to}"),
            p,
        );
        list_matrix(&out.effects().transform, 100.0, 100.0)
    }

    #[test]
    fn transforms_interpolate_as_the_reference_browser_has_them() {
        let m2 = |m: [f32; 6]| Matrix::from_2d(m);
        let cases: &[(&str, &str, f64, Matrix)] = &[
            // Matrices from the first pair that differ.
            (
                "scale(2) rotate(0deg)",
                "rotate(90deg) translate(10px)",
                0.5,
                m2([1.06066, 1.06066, -1.06066, 1.06066, 0.0, 5.0]),
            ),
            (
                "skewX(30deg)",
                "rotate(90deg)",
                0.25,
                m2([0.92388, 0.382683, 0.0173681, 1.08959, 0.0, 0.0]),
            ),
            (
                "rotate(45deg) translate(10px, 0)",
                "translate(10px, 0) rotate(45deg)",
                0.5,
                m2([R, R, -R, R, 8.53553, 3.53553]),
            ),
            // A percentage resolved with the box.
            (
                "translateX(50%) scale(1)",
                "rotate(90deg)",
                0.5,
                m2([R, R, -R, R, 25.0, 0.0]),
            ),
            // `none` as the identity of the other's functions.
            ("none", "scale(2)", 0.5, m2([1.5, 0.0, 0.0, 1.5, 0.0, 0.0])),
            // Pairs of a primitive: the turns kept, the percentage mixed.
            (
                "translateX(10px) rotate(0deg)",
                "translateX(50%) rotate(180deg)",
                0.25,
                m2([R, R, -R, R, 20.0, 0.0]),
            ),
            (
                "rotate(0deg)",
                "rotate(720deg)",
                0.25,
                m2([-1.0, 0.0, 0.0, -1.0, 0.0, 0.0]),
            ),
            // Skews of one name pairwise, of two as matrices, as the
            // reference browser has them.
            (
                "skewX(10deg)",
                "skewX(50deg)",
                0.5,
                m2([1.0, 0.0, 0.57735, 1.0, 0.0, 0.0]),
            ),
            (
                "skewY(10deg)",
                "skewY(50deg)",
                0.5,
                m2([1.0, 0.57735, 0.0, 1.0, 0.0, 0.0]),
            ),
            (
                "skew(10deg)",
                "skew(50deg, 20deg)",
                0.5,
                m2([1.0, 0.176327, 0.57735, 1.0, 0.0, 0.0]),
            ),
            (
                "skewX(10deg)",
                "skew(50deg)",
                0.5,
                m2([1.0, 0.0, 0.68404, 1.0, 0.0, 0.0]),
            ),
            (
                "skewX(10deg)",
                "skewY(40deg)",
                0.5,
                m2([1.08319, 0.394248, 0.119274, 0.983105, 0.0, 0.0]),
            ),
            (
                "scaleX(-1)",
                "scaleY(-1)",
                0.5,
                m2([0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
            ),
            // Perspective through its matrix.
            (
                "perspective(100px) translateZ(0)",
                "perspective(200px) translateZ(50px)",
                0.5,
                Matrix([
                    1.0, 0.0, 0.0, 0.0, //
                    0.0, 1.0, 0.0, 0.0, //
                    0.0, 0.0, 1.0, -0.0075, //
                    0.0, 0.0, 25.0, 0.8125,
                ]),
            ),
            // Two axes: as matrices.
            (
                "rotateX(60deg)",
                "rotateY(60deg)",
                0.5,
                Matrix([
                    0.857143, 0.142857, -0.494872, 0.0, //
                    0.142857, 0.857143, 0.494872, 0.0, //
                    0.494872, -0.494872, 0.714286, 0.0, //
                    0.0, 0.0, 0.0, 1.0,
                ]),
            ),
            // A matrix without an inverse: discrete, the first value
            // until half way.
            (
                "scale(0)",
                "rotate(90deg)",
                0.25,
                m2([0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
            ),
            (
                "matrix(1, 0, 0, 1, 0, 0)",
                "matrix(0, 0, 0, 0, 0, 0)",
                0.25,
                Matrix::IDENTITY,
            ),
        ];
        for (from, to, p, want) in cases {
            let got = transform(from, to, *p);
            assert!(
                near(&got, want),
                "{from} -> {to} @{p}: {got:?} not {want:?}"
            );
        }
        // A 2D pair stays a `matrix()`.
        let out = blended(
            Longhand::Transform,
            "transform: skewX(30deg)",
            "transform: rotate(90deg)",
            0.3,
        );
        assert!(matches!(
            out.effects().transform[..],
            [TransformFunction::Matrix(_)]
        ));
    }

    #[test]
    fn the_individual_transforms_interpolate_from_none() {
        let r = blended(Longhand::Rotate, "rotate: x 90deg", "rotate: y 90deg", 0.5);
        let Some(Rotate { x, y, z, angle }) = r.effects().rotate else {
            panic!("a rotation");
        };
        assert!((x - R).abs() < 1e-5 && (y - R).abs() < 1e-5 && z.abs() < 1e-6);
        assert!((angle - 70.5288).abs() < 1e-3, "{angle}");
        let r = blended(
            Longhand::Rotate,
            "rotate: none",
            "rotate: 0 0 1 720deg",
            0.25,
        );
        assert_eq!(r.effects().rotate.map(|r| r.angle), Some(180.0));
        let s = blended(Longhand::Scale, "scale: none", "scale: 2", 0.5);
        assert_eq!(s.effects().scale, Some([1.5, 1.5, 1.0]));
        let t = blended(
            Longhand::Translate,
            "translate: none",
            "translate: 100px 50%",
            0.5,
        );
        let translate = t.effects().translate.clone().expect("a translation");
        assert_eq!(translate.x, LengthPercentage::Px(50.0));
        assert_eq!(translate.y, LengthPercentage::Percent(25.0));
        let none = blended(Longhand::Scale, "scale: none", "scale: none", 0.5);
        assert_eq!(none.effects().scale, None);
    }
}
