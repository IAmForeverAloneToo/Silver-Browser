//! Replaced boxes (M23.2, `docs/design/images.md`): an `img`'s box,
//! sized by CSS Images 3 section 4.3's default sizing algorithm with a
//! default object size of 300 by 150, CSS2 sections 10.3.2 and 10.6.2,
//! and section 10.4's table where `min-width`, `max-width`, `min-height`
//! and `max-height` meet a ratio; the ratio being the image's natural
//! one or the one `aspect-ratio` gives (css-sizing-4 section 5.1).
//!
//! Every formatting context sizes a replaced box through here: a block,
//! an inline-block, a float and an absolutely positioned box take
//! [`used_size`]'s width, flex and grid items their content widths,
//! [`content_widths`], and every one its height for the width it was
//! given, [`used_height`], unless a flex or grid container stretches it.

use style::ComputedStyle;
use style::computed::{AspectRatio, BOTTOM, BoxSizing, ImageOrientation, LEFT, MaxSize, Size, TOP};

use crate::block::{
    AdjoiningMargins, BlockLayout, Edges, FragmentKind, Imposed, box_fragment, clamp_height,
    definite_height, height_bounds,
};
use crate::box_tree::{LayoutBox, Replaced};

/// The default object size (CSS Images 3 section 4.3): what a replaced
/// box with no natural size and no ratio is.
pub(crate) const DEFAULT_WIDTH: f32 = 300.0;
pub(crate) const DEFAULT_HEIGHT: f32 = 150.0;

/// The side of the icon an image that cannot be shown is drawn as.
pub const ICON_SIZE: f32 = 16.0;

/// The side of a checkbox or a radio button by nature (M26.7).
pub const MARK_SIZE: f32 = 13.0;

/// A replaced box's natural size and ratio, in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Natural {
    width: Option<f32>,
    height: Option<f32>,
    ratio: Option<f32>,
}

/// What `replaced` is by nature, oriented as `style`'s
/// `image-orientation` says: an image its header's size, turned back
/// when its orientation is not to be applied; the icon 16 by 16; nothing
/// 0 by 0 with no ratio.
fn natural(style: &ComputedStyle, replaced: &Replaced) -> Natural {
    match replaced {
        Replaced::Image { header, .. } => {
            let (w, h) = if style.image().image_orientation == ImageOrientation::None
                && header.orientation.swaps_sides()
            {
                (header.height, header.width)
            } else {
                (header.width, header.height)
            };
            let (w, h) = (w as f32, h as f32);
            Natural {
                width: Some(w),
                height: Some(h),
                ratio: (w > 0.0 && h > 0.0).then(|| w / h),
            }
        }
        Replaced::Icon => Natural {
            width: Some(ICON_SIZE),
            height: Some(ICON_SIZE),
            ratio: Some(1.0),
        },
        Replaced::Empty => Natural {
            width: Some(0.0),
            height: Some(0.0),
            ratio: None,
        },
        // A checkbox or a radio button is 13 by 13 (M26.7), as the
        // reference browser's are; an arrow, sized by its style, is too.
        // None has a ratio: a checkbox given a width alone keeps its
        // height, as there (M26.11).
        Replaced::Mark(_) => Natural {
            width: Some(MARK_SIZE),
            height: Some(MARK_SIZE),
            ratio: None,
        },
        // A canvas is its bitmap's size (M30).
        Replaced::Canvas { width, height, .. } => {
            let (w, h) = (*width as f32, *height as f32);
            Natural {
                width: Some(w),
                height: Some(h),
                ratio: (w > 0.0 && h > 0.0).then(|| w / h),
            }
        }
        // A gradient has no natural size nor ratio: the default object
        // size stands in (CSS Images 3 section 4.3).
        Replaced::Gradient(_) => Natural {
            width: None,
            height: None,
            ratio: None,
        },
    }
}

/// The box's preferred ratio, width over height, and whether it is of
/// the content box: a natural ratio is; one `aspect-ratio` gives is of
/// the box `box-sizing` names (css-sizing-4 section 5.1). A degenerate
/// `<ratio>` is `auto`.
fn ratio(style: &ComputedStyle, natural: &Natural) -> Option<(f32, bool)> {
    let natural_ratio = natural.ratio.map(|r| (r, true));
    let given = |r: style::computed::Ratio| r.value().map(|r| (r, false));
    match style.image().aspect_ratio {
        AspectRatio::Auto => natural_ratio,
        AspectRatio::AutoOr(r) => natural_ratio.or_else(|| given(r)),
        AspectRatio::Ratio(r) => given(r).or(natural_ratio),
    }
}

/// The sizes a replaced box is fitted between, as content-box sizes.
struct Sizing<'a> {
    style: &'a ComputedStyle,
    natural: Natural,
    ratio: Option<(f32, bool)>,
    /// Its border and padding across and down.
    horizontal: f32,
    vertical: f32,
    /// The icon with not both of `width` and `height` given: 16 by 16,
    /// whatever its style says.
    fixed: bool,
}

impl<'a> Sizing<'a> {
    fn of(b: &'a LayoutBox, replaced: &Replaced, cb_width: Option<f32>) -> Self {
        let style = &b.style;
        let natural = natural(style, replaced);
        let edges = Edges::of(style, cb_width.unwrap_or(0.0));
        let fixed = *replaced == Replaced::Icon
            && !(style.width.length_percentage().is_some()
                && style.height.length_percentage().is_some());
        Sizing {
            style,
            natural,
            ratio: ratio(style, &natural),
            horizontal: edges.horizontal(),
            vertical: edges.vertical(),
            fixed,
        }
    }

    fn inner(&self, v: f32, non_content: f32) -> f32 {
        match self.style.box_sizing {
            BoxSizing::ContentBox => v.max(0.0),
            BoxSizing::BorderBox => (v - non_content).max(0.0),
        }
    }

    /// `width` as a content-box width: `None` for `auto`, a keyword,
    /// and a percentage of a width not known.
    fn width(&self, cb_width: Option<f32>) -> Option<f32> {
        let v = self.style.width.length_percentage()?;
        match v {
            style::computed::LengthPercentage::Px(px) => Some(self.inner(px, self.horizontal)),
            v => cb_width.map(|w| self.inner(v.resolve(w), self.horizontal)),
        }
    }

    /// `min-width` and `max-width` as content-box widths: a percentage
    /// of a width not known is zero and `none`, a keyword likewise.
    fn width_bounds(&self, cb_width: Option<f32>) -> (f32, Option<f32>) {
        let min = match &self.style.min_width {
            Size::Px(px) => self.inner(*px, self.horizontal),
            size => size.length_percentage().map_or(0.0, |v| {
                self.inner(v.resolve(cb_width.unwrap_or(0.0)), self.horizontal)
            }),
        };
        let max = match &self.style.max_width {
            MaxSize::Px(px) => Some(self.inner(*px, self.horizontal)),
            size => size
                .length_percentage()
                .and_then(|v| cb_width.map(|w| self.inner(v.resolve(w), self.horizontal))),
        };
        (min, max)
    }

    /// The content-box height a content-box width gives through the
    /// ratio, applied to the box `box-sizing` names where the ratio is
    /// not the image's.
    fn height_for(&self, width: f32) -> Option<f32> {
        let (ratio, natural) = self.ratio?;
        Some(
            if natural || self.style.box_sizing == BoxSizing::ContentBox {
                width / ratio
            } else {
                ((width + self.horizontal) / ratio - self.vertical).max(0.0)
            },
        )
    }

    /// The content-box width a content-box height gives, the same way.
    fn width_for(&self, height: f32) -> Option<f32> {
        let (ratio, natural) = self.ratio?;
        Some(
            if natural || self.style.box_sizing == BoxSizing::ContentBox {
                height * ratio
            } else {
                ((height + self.vertical) * ratio - self.horizontal).max(0.0)
            },
        )
    }
}

/// `width` between `min` and `max`, the minimum winning.
fn clamp(width: f32, (min, max): (f32, Option<f32>)) -> f32 {
    max.map_or(width, |max| width.min(max)).max(min)
}

/// CSS2 section 10.4's table: a width and height both from the box's
/// nature, fitted between the limits with their ratio kept where the
/// limits allow it.
fn fit(
    w: f32,
    h: f32,
    (min_w, max_w): (f32, Option<f32>),
    (min_h, max_h): (f32, Option<f32>),
) -> (f32, f32) {
    if w <= 0.0 || h <= 0.0 {
        return (clamp(w, (min_w, max_w)), clamp(h, (min_h, max_h)));
    }
    // A maximum below its minimum is the minimum (section 10.4).
    let max_w = max_w.unwrap_or(f32::INFINITY).max(min_w);
    let max_h = max_h.unwrap_or(f32::INFINITY).max(min_h);
    let (wide, narrow) = (w > max_w, w < min_w);
    let (tall, short) = (h > max_h, h < min_h);
    match (wide, narrow, tall, short) {
        (true, _, true, _) if max_w / w <= max_h / h => (max_w, min_h.max(max_w * h / w)),
        (true, _, true, _) => (min_w.max(max_h * w / h), max_h),
        (_, true, _, true) if min_w / w <= min_h / h => (max_w.min(min_h * w / h), min_h),
        (_, true, _, true) => (min_w, max_h.min(min_w * h / w)),
        (_, true, true, _) => (min_w, max_h),
        (true, _, _, true) => (max_w, min_h),
        (true, ..) => (max_w, (max_w * h / w).max(min_h)),
        (_, true, ..) => (min_w, (min_w * h / w).min(max_h)),
        (_, _, true, _) => ((max_h * w / h).max(min_w), max_h),
        (_, _, _, true) => ((min_h * w / h).min(max_w), min_h),
        _ => (w, h),
    }
}

/// The content-box width and height of the replaced box `b` against a
/// containing block `cb_width` wide and `cb_height` tall, either `None`
/// where not known, which makes a percentage of it `auto`.
pub(crate) fn used_size(
    b: &LayoutBox,
    replaced: &Replaced,
    cb_width: Option<f32>,
    cb_height: Option<f32>,
) -> (f32, f32) {
    let s = Sizing::of(b, replaced, cb_width);
    if s.fixed {
        return (ICON_SIZE, ICON_SIZE);
    }
    let style = s.style;
    let width = s.width(cb_width);
    let height = definite_height(style, cb_height, s.vertical);
    let width_bounds = s.width_bounds(cb_width);
    let height_bounds = height_bounds(style, cb_height, s.vertical, None);
    let natural = s.natural;
    match (width, height) {
        (Some(w), Some(h)) => (clamp(w, width_bounds), clamp_height(h, height_bounds)),
        (None, None) => natural_fit(&s, width_bounds, height_bounds),
        (Some(w), None) => {
            let w = clamp(w, width_bounds);
            let h = s.height_for(w).or(natural.height).unwrap_or(DEFAULT_HEIGHT);
            (w, clamp_height(h, height_bounds))
        }
        (None, Some(h)) => {
            let h = clamp_height(h, height_bounds);
            let w = s.width_for(h).or(natural.width).unwrap_or(DEFAULT_WIDTH);
            (clamp(w, width_bounds), h)
        }
    }
}

/// The size of a replaced box with neither `width` nor `height` given,
/// fitted between `width_bounds` and `height_bounds`: its natural size,
/// a side it lacks from the other through its ratio, else the default
/// object size, kept in its ratio by section 10.4's table.
fn natural_fit(
    s: &Sizing<'_>,
    width_bounds: (f32, Option<f32>),
    height_bounds: (f32, Option<f32>),
) -> (f32, f32) {
    let natural = s.natural;
    let (w, h) = match (natural.width, natural.height) {
        (Some(w), h) => (w, s.height_for(w).or(h).unwrap_or(DEFAULT_HEIGHT)),
        (None, Some(h)) => (s.width_for(h).unwrap_or(DEFAULT_WIDTH), h),
        // A ratio and no size: contained in the default object size.
        (None, None) => match s.ratio {
            Some((r, _)) if r >= DEFAULT_WIDTH / DEFAULT_HEIGHT => {
                (DEFAULT_WIDTH, DEFAULT_WIDTH / r)
            }
            Some((r, _)) => (DEFAULT_HEIGHT * r, DEFAULT_HEIGHT),
            None => (DEFAULT_WIDTH, DEFAULT_HEIGHT),
        },
    };
    if s.ratio.is_some() {
        fit(w, h, width_bounds, height_bounds)
    } else {
        (clamp(w, width_bounds), clamp_height(h, height_bounds))
    }
}

/// The content-box width of the replaced box `b` as if its `width`,
/// `min-width` and `max-width` were their initial values (css-sizing-3
/// section 5.1, M23): what its ratio makes of a definite height, its
/// `height` against `cb_height` or else `stretched`, one its formatting
/// context makes definite, between its height limits; else its natural
/// size fitted under its height limits alone.
fn auto_width(
    b: &LayoutBox,
    replaced: &Replaced,
    cb_height: Option<f32>,
    stretched: Option<f32>,
) -> f32 {
    let s = Sizing::of(b, replaced, None);
    if s.fixed {
        return ICON_SIZE;
    }
    let bounds = height_bounds(s.style, cb_height, s.vertical, None);
    match definite_height(s.style, cb_height, s.vertical).or(stretched) {
        Some(h) => {
            let h = clamp_height(h, bounds);
            s.width_for(h).or(s.natural.width).unwrap_or(DEFAULT_WIDTH)
        }
        None => natural_fit(&s, (0.0, None), bounds).0,
    }
}

/// The content-box width of the replaced box `b` as if none of its
/// sizes and limits were given: its natural size, or the default object
/// size, kept in its ratio; what a flex container measures a column item
/// at before its own main size, a height, counts (css-flexbox-1 section
/// 9.2 step 3).
pub(crate) fn natural_width(b: &LayoutBox, replaced: &Replaced) -> f32 {
    let s = Sizing::of(b, replaced, None);
    if s.fixed {
        return ICON_SIZE;
    }
    natural_fit(&s, (0.0, None), (0.0, None)).0
}

/// The content-box width the replaced box `b`'s ratio makes of the
/// content-box `height` (css-sizing-4 section 5.1): `None` for a box with
/// no ratio, and for the icon, which keeps its size.
pub(crate) fn transferred_width(b: &LayoutBox, replaced: &Replaced, height: f32) -> Option<f32> {
    let s = Sizing::of(b, replaced, None);
    if s.fixed {
        return None;
    }
    s.width_for(height)
}

/// The content-box height of the replaced box `b` at the content-box
/// `width` as if its `height`, `min-height` and `max-height` were their
/// initial values: what its ratio makes of the width, else its natural
/// height, else 150; a block's automatic content height, unclamped.
fn auto_height(b: &LayoutBox, replaced: &Replaced, width: f32, cb_width: f32) -> f32 {
    let s = Sizing::of(b, replaced, Some(cb_width));
    if s.fixed {
        return ICON_SIZE;
    }
    s.height_for(width)
        .or(s.natural.height)
        .unwrap_or(DEFAULT_HEIGHT)
}

/// The content-box height of the replaced box `b` given the content-box
/// `width` its formatting context gave it: its `height`, else what the
/// ratio makes of the width, else its natural height, else 150; between
/// `min-height` and `max-height`. Where the width came from
/// [`used_size`] this is the height that gave, section 10.4's table
/// included.
pub(crate) fn used_height(
    b: &LayoutBox,
    replaced: &Replaced,
    width: f32,
    cb_width: f32,
    cb_height: Option<f32>,
) -> f32 {
    let s = Sizing::of(b, replaced, Some(cb_width));
    if s.fixed {
        return ICON_SIZE;
    }
    let bounds = height_bounds(s.style, cb_height, s.vertical, None);
    let height = definite_height(s.style, cb_height, s.vertical)
        .or_else(|| s.height_for(width))
        .or(s.natural.height)
        .unwrap_or(DEFAULT_HEIGHT);
    clamp_height(height, bounds)
}

/// The min-content and max-content content widths of the replaced box
/// `b`: [`widths_in`] with the block axis unknown.
pub(crate) fn content_widths(b: &LayoutBox, replaced: &Replaced) -> (f32, f32) {
    widths_in(b, replaced, None, None)
}

/// The min-content and max-content content widths of the replaced box
/// `b` (css-sizing-3 section 5.1): its width as if its own `width`,
/// `min-width` and `max-width` were their initial values, which whoever
/// asks applies, its height known from `cb_height`, against which a
/// percentage of its own resolves, or `stretched` (see [`auto_width`]);
/// for the minimum zero where its `width` or `max-width` is a percentage
/// (section 5.2.2: such a box is compressible).
pub(crate) fn widths_in(
    b: &LayoutBox,
    replaced: &Replaced,
    cb_height: Option<f32>,
    stretched: Option<f32>,
) -> (f32, f32) {
    let width = auto_width(b, replaced, cb_height, stretched);
    let percentage = |v: Option<style::computed::LengthPercentage>| {
        v.is_some_and(|v| v.resolve(0.0) != v.resolve(100.0))
    };
    let style = &b.style;
    let compressible = !Sizing::of(b, replaced, None).fixed
        && (percentage(style.width.length_percentage())
            || percentage(style.max_width.length_percentage()));
    (if compressible { 0.0 } else { width }, width)
}

/// Lays out the replaced box `b` at `content_width`: its box, and in its
/// content box a fragment of what it shows. Its height is the one
/// `imposed` on it, when its formatting context imposes one, else
/// [`used_height`]. It has no baseline: its line puts its bottom margin
/// edge there (CSS2 section 10.8.1), a flex or grid container its
/// bottom border edge.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_replaced<'a>(
    b: &'a LayoutBox,
    replaced: &Replaced,
    edges: &Edges,
    cb_width: f32,
    cb_height: Option<f32>,
    content_width: f32,
    margin_left: f32,
    imposed: Option<Imposed>,
) -> BlockLayout<'a> {
    let content_height = match imposed {
        Some(imposed) => (imposed.height - edges.vertical()).max(0.0),
        None => used_height(b, replaced, content_width, cb_width, cb_height),
    };
    let height = content_height + edges.vertical();
    let mut root = box_fragment(content_width + edges.horizontal(), height, b);
    let mut content = box_fragment(content_width, content_height, b);
    content.kind = FragmentKind::Replaced(replaced.clone());
    content.x = edges.border[LEFT] + edges.padding[LEFT];
    content.y = edges.border[TOP] + edges.padding[TOP];
    root.children.push(content);
    let style = &b.style;
    let own_top = AdjoiningMargins::of(style.margin[TOP].resolve(cb_width, 0.0));
    let own_bottom = AdjoiningMargins::of(style.margin[BOTTOM].resolve(cb_width, 0.0));
    BlockLayout {
        root,
        height,
        margin_left,
        margin_top: own_top,
        margin_bottom: own_bottom,
        top_edge: own_top,
        bottom_edge: own_bottom,
        self_collapsing: false,
        first_baseline: None,
        first_line: None,
        last_baseline: None,
        inline_block_baseline: None,
        absolutes: Vec::new(),
        // As a block's, the height its `height: auto` would be, unclamped:
        // what a flex container's minimum and keywords read.
        auto_content_height: auto_height(b, replaced, content_width, cb_width),
        held: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Fragment, layout_document};
    use crate::box_tree::ImageState;
    use crate::testing::{Laid, boxes_with};
    use images::Orientation;
    use text::FontCollection;

    /// The image a test's `src` names: `200x100.png` is 200 by 100,
    /// `turned.jpg` stored 200 by 100 and turned a quarter by its EXIF
    /// data, `broken.png` cannot be shown, and any other is coming.
    fn state(src: Option<&str>) -> ImageState {
        let header = |width, height, orientation| images::Header {
            format: images::Format::Png,
            width,
            height,
            orientation,
        };
        match src {
            Some("200x100.png") => ImageState::Ready {
                id: 1,
                header: header(200, 100, Orientation::Normal),
            },
            Some("turned.jpg") => ImageState::Ready {
                id: 2,
                header: header(100, 200, Orientation::Rotate90),
            },
            Some("10x10.png") => ImageState::Ready {
                id: 3,
                header: header(10, 10, Orientation::Normal),
            },
            Some("broken.png") => ImageState::Broken,
            _ => ImageState::Coming,
        }
    }

    /// `html` laid out under `css` against an 800px viewport, the body's
    /// margin taken away.
    fn laid(html: &str, css: &str) -> Laid {
        let fonts = FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the test font");
        let doc = html::parse_document(html).expect("html parsing never fails");
        let sheet = css::parse_stylesheet(&format!("body {{ margin: 0 }} {css}"))
            .expect("test css is valid");
        let (document, boxes) = boxes_with(&doc, &[sheet], state);
        let layout = layout_document(&boxes, 800.0, &fonts);
        Laid { document, layout }
    }

    /// The fragments of the element whose `id` is `id`.
    fn fragments<'l>(laid: &'l Laid, id: &str) -> Vec<&'l Fragment> {
        let document = &laid.document;
        let node = document
            .subtree(document.root())
            .find(|&n| document.attr(n, "id") == Some(id))
            .unwrap_or_else(|| panic!("no element #{id}"));
        laid.iter().filter(|f| f.node == node).collect()
    }

    /// A rectangle: `(x, y, w, h)`.
    type Rect = (f32, f32, f32, f32);

    /// The border box of `#i` and the content box what it shows is in.
    fn img(laid: &Laid) -> (Rect, Rect) {
        let found = fragments(laid, "i");
        let own = found
            .iter()
            .find(|f| matches!(f.kind, FragmentKind::Box) && !f.children.is_empty())
            .expect("the img's box");
        let content = &own.children[0];
        assert!(
            matches!(content.kind, FragmentKind::Replaced(_)),
            "{content:?}"
        );
        (
            (own.x, own.y, own.w, own.h),
            (content.x, content.y, content.w, content.h),
        )
    }

    fn size(html: &str, css: &str) -> (f32, f32) {
        let ((_, _, w, h), _) = img(&laid(html, css));
        (w, h)
    }

    const IMG: &str = "<img id=i src=200x100.png>";

    #[test]
    fn an_image_is_its_natural_size_its_ratio_kept_by_one_given_side() {
        assert_eq!(size(IMG, ""), (200.0, 100.0));
        assert_eq!(
            size("<img id=i src=200x100.png width=100>", ""),
            (100.0, 50.0)
        );
        assert_eq!(
            size("<img id=i src=200x100.png height=80>", ""),
            (160.0, 80.0)
        );
        assert_eq!(
            size("<img id=i src=200x100.png width=50 height=50>", ""),
            (50.0, 50.0)
        );
        assert_eq!(size(IMG, "#i { width: 50% }"), (400.0, 200.0));
        assert_eq!(size(IMG, "#i { aspect-ratio: 1 / 1 }"), (200.0, 200.0));
        // Section 10.4's table: a limit on either side keeps the ratio.
        assert_eq!(size(IMG, "#i { max-width: 100px }"), (100.0, 50.0));
        assert_eq!(size(IMG, "#i { max-height: 50px }"), (100.0, 50.0));
        assert_eq!(size(IMG, "#i { min-width: 300px }"), (300.0, 150.0));
        // A given width is clamped alone.
        assert_eq!(
            size(IMG, "#i { width: 300px; max-width: 100px }"),
            (100.0, 50.0)
        );
        // The content box holds what it shows.
        let (own, content) = img(&laid(IMG, "#i { padding: 10px; border: 5px solid }"));
        assert_eq!((own.2, own.3), (230.0, 130.0));
        assert_eq!(content, (own.0 + 15.0, own.1 + 15.0, 200.0, 100.0));
    }

    #[test]
    fn the_icon_and_nothing_are_sized_as_the_reference_browser_sizes_them() {
        let broken = "<img id=i src=broken.png";
        assert_eq!(size(&format!("{broken}>"), ""), (16.0, 16.0));
        // One side given is not enough to size the icon's box.
        assert_eq!(size(&format!("{broken} width=100>"), ""), (16.0, 16.0));
        assert_eq!(
            size(&format!("{broken}>"), "#i { width: 100px }"),
            (16.0, 16.0)
        );
        assert_eq!(
            size(&format!("{broken} width=100 height=80>"), ""),
            (100.0, 80.0)
        );
        // No `src` and no `alt`, or an empty `alt`: nothing, which its
        // attributes still size.
        assert_eq!(size("<img id=i>", ""), (0.0, 0.0));
        assert_eq!(size("<img id=i src=''>", ""), (0.0, 0.0));
        assert_eq!(size(&format!("{broken} alt=''>"), ""), (0.0, 0.0));
        assert_eq!(size("<img id=i width=100 height=80>", ""), (100.0, 80.0));
        // An image still coming: what its attributes give, their ratio
        // sizing a side its style leaves `auto`.
        assert_eq!(size("<img id=i src=later.png>", ""), (0.0, 0.0));
        assert_eq!(
            size(
                "<img id=i src=later.png width=100 height=50>",
                "#i { width: 300px; height: auto }"
            ),
            (300.0, 150.0)
        );
        // A ratio `aspect-ratio` gives is of the box `box-sizing` names.
        assert_eq!(
            size(
                "<img id=i src=later.png>",
                "#i { box-sizing: border-box; width: 100px; padding: 10px 20px; aspect-ratio: 1 }"
            ),
            (100.0, 100.0)
        );
    }

    #[test]
    fn an_image_that_cannot_be_shown_with_alt_text_is_that_text_after_the_icon() {
        let l = laid(
            "<p><img id=i src=broken.png alt=Hello width=100 height=80></p>",
            "",
        );
        let found = fragments(&l, "i");
        let inline = found
            .iter()
            .find(|f| matches!(f.kind, FragmentKind::InlineBox))
            .expect("an inline box, not a replaced one");
        let icon = found
            .iter()
            .find(|f| {
                f.children
                    .first()
                    .is_some_and(|c| matches!(c.kind, FragmentKind::Replaced(Replaced::Icon)))
            })
            .expect("the icon");
        let text = found
            .iter()
            .find(|f| matches!(&f.kind, FragmentKind::Text(t) if t == "Hello"))
            .expect("the text");
        // The width and height do not apply: the icon is 16 by 16, the
        // text after it.
        assert_eq!((icon.w, icon.h), (16.0, 16.0));
        assert_eq!(text.x, icon.x + 16.0);
        assert!(inline.w < 100.0 && inline.x == icon.x, "{inline:?}");
        // No `src` with alt text is the same.
        let l = laid("<p><img id=i alt=Hello></p>", "");
        assert!(
            fragments(&l, "i")
                .iter()
                .any(|f| matches!(f.kind, FragmentKind::InlineBox))
        );
        // A block shows it on a line of its own.
        let l = laid(
            "<img id=i src=broken.png alt=Hello>",
            "#i { display: block }",
        );
        let own = fragments(&l, "i")
            .into_iter()
            .find(|f| matches!(f.kind, FragmentKind::Box) && f.w == 800.0);
        assert!(own.is_some());
    }

    #[test]
    fn a_replaced_box_is_placed_as_its_display_says() {
        let rect = |css: &str| img(&laid(IMG, css)).0;
        assert_eq!(
            rect("#i { display: block; margin: 0 auto }"),
            (300.0, 0.0, 200.0, 100.0)
        );
        assert_eq!(rect("#i { float: right }"), (600.0, 0.0, 200.0, 100.0));
        // Insets do not stretch it.
        assert_eq!(
            rect("#i { position: absolute; left: 0; right: 0; top: 0; bottom: 0 }"),
            (0.0, 0.0, 200.0, 100.0)
        );
        assert_eq!(
            rect("#i { position: absolute; left: 0; right: 0; margin: 0 auto }"),
            (300.0, 0.0, 200.0, 100.0)
        );
        // Nor does a containing block of a definite height.
        let l = laid(
            "<div id=d><img id=i src=200x100.png></div>",
            "#d { position: relative; height: 300px }
             #i { position: absolute; top: 0; bottom: 0 }",
        );
        let (own, content) = img(&l);
        assert_eq!((own.3, content.3), (100.0, 100.0));
        // A percentage of a width being found: the max-content width is
        // the natural one, the min-content width nothing.
        let html = "<div id=d><img id=i src=200x100.png></div>";
        let l = laid(html, "#d { display: inline-block } #i { width: 100% }");
        assert_eq!((img(&l).0.2, img(&l).0.3), (200.0, 100.0));
        let l = laid(
            html,
            "#d { float: left; width: min-content } #i { width: 100% }",
        );
        assert_eq!((img(&l).0.2, img(&l).0.3), (0.0, 0.0));
        let l = laid(html, "#d { float: left; width: min-content }");
        assert_eq!(img(&l).0.2, 200.0);
        // A percentage height of a definite height gives the width
        // through the ratio, inline as in a block.
        let l = laid(html, "#d { height: 300px } #i { height: 50% }");
        assert_eq!((img(&l).0.2, img(&l).0.3), (300.0, 150.0));
        let l = laid(
            html,
            "#d { height: 300px } #i { height: 50%; display: block }",
        );
        assert_eq!((img(&l).0.2, img(&l).0.3), (300.0, 150.0));
    }

    #[test]
    fn an_imgs_content_image_replaces_its_own_unless_broken() {
        let img = "<img id=i src=10x10.png>";
        assert_eq!(
            size(img, "#i { content: url(200x100.png) }"),
            (200.0, 100.0)
        );
        assert_eq!(size(img, "#i { content: url(broken.png) }"), (10.0, 10.0));
    }

    #[test]
    fn a_replaced_flex_items_sizes_come_through_its_ratio() {
        let item = |html: &str, css: &str| {
            let ((_, _, w, h), _) = img(&laid(html, css));
            (w, h)
        };
        let one = "<div id=c><img id=i src=200x100.png></div>";
        // Its content size suggestion, the automatic minimum, from its
        // stretched height: it shrinks no further in a container of none.
        assert_eq!(
            item(one, "#c { display: flex; width: 0; height: 50px }"),
            (100.0, 50.0)
        );
        // The minimum is the smaller of its `width` and its content size,
        // which its own `width` has no part in.
        assert_eq!(
            item(
                one,
                "#c { display: flex; width: 0; align-items: flex-start } \
                 #i { width: 400px; height: 30px }"
            ),
            (60.0, 30.0)
        );
        // An inline-flex container's width from its height, which a
        // single line stretches its item to; a multi-line one does not.
        let l = laid(one, "#c { display: inline-flex; height: 50px }");
        assert_eq!((img(&l).0.2, fragments(&l, "c")[0].w), (100.0, 100.0));
        let l = laid(
            one,
            "#c { display: inline-flex; height: 50px; flex-wrap: wrap }",
        );
        assert_eq!((img(&l).0.2, fragments(&l, "c")[0].w), (200.0, 200.0));
        // A column item's base size from its natural size, its
        // `min-height` applied after: 100 at its natural width, then the
        // 300px left over shared with its sibling.
        let column = "<div id=c><img id=i src=200x100.png><div style='flex: 1 0 0px'></div></div>";
        let (_, h) = item(
            column,
            "#c { display: flex; flex-direction: column; height: 400px; align-items: flex-start } \
             #i { min-height: 200px; flex: 1 0 auto }",
        );
        assert_eq!(h, 250.0);
    }

    #[test]
    fn a_flex_container_stretches_a_replaced_item_and_a_grid_does_not() {
        let html = "<div id=c><img id=i src=200x100.png></div>";
        let item = |css: &str| {
            let ((_, _, w, h), _) = img(&laid(html, css));
            (w, h)
        };
        // Its width what its ratio makes of the stretched height
        // (css-flexbox-1 section 9.2 step 3, case B).
        assert_eq!(item("#c { display: flex; height: 300px }"), (600.0, 300.0));
        assert_eq!(
            item("#c { display: flex; height: 300px; align-items: flex-start }"),
            (200.0, 100.0)
        );
        assert_eq!(
            item("#c { display: flex; flex-direction: column; width: 500px }"),
            (500.0, 250.0)
        );
        assert_eq!(item("#c { display: flex } #i { flex: 1 }"), (800.0, 400.0));
        let grid = "#c { display: grid; grid-template-columns: 400px; height: 300px }";
        assert_eq!(item(grid), (200.0, 100.0));
        assert_eq!(
            item(&format!("{grid} #i {{ justify-self: stretch }}")),
            (400.0, 200.0)
        );
        // Stretched, its width what its ratio makes of its height, past
        // its column, as the reference browser has it.
        assert_eq!(
            item(&format!("{grid} #i {{ align-self: stretch }}")),
            (600.0, 300.0)
        );
    }

    #[test]
    fn a_replaced_grid_items_width_comes_from_its_rows() {
        let html = "<div id=c><img id=i src=200x100.png></div>";
        let item = |css: &str| {
            let l = laid(html, css);
            let ((_, _, w, h), _) = img(&l);
            (w, h, fragments(&l, "c")[0].w)
        };
        // Its height a percentage of its row's: its column sized to the
        // width its ratio makes of it (the reference browser's sizes).
        assert_eq!(
            item("#c { display: inline-grid; grid-template-rows: 50px } #i { height: 100% }"),
            (100.0, 50.0, 100.0)
        );
        assert_eq!(
            item("#c { display: grid; grid-template-rows: 80px } #i { max-height: 50% }"),
            (80.0, 40.0, 800.0)
        );
        // Stretched in a row with a fixed maximum.
        assert_eq!(
            item(
                "#c { display: inline-grid; grid-template-rows: minmax(0, 60px) } #i { align-self: stretch }"
            ),
            (120.0, 60.0, 120.0)
        );
        assert_eq!(
            item(
                "#c { display: inline-grid; grid-template-rows: 80px } \
                 #i { align-self: stretch; margin: 10px; padding: 5px }"
            ),
            (110.0, 60.0, 130.0)
        );
        // A row with no fixed maximum gives it nothing.
        assert_eq!(
            item("#c { display: inline-grid } #i { height: 100% }"),
            (200.0, 100.0, 200.0)
        );
    }

    #[test]
    fn an_inline_image_sits_on_the_baseline_and_makes_its_line_tall() {
        let l = laid(
            "<p id=p>x<img id=i src=200x100.png>y</p>",
            "p { margin: 0 }",
        );
        let ((x, y, _, h), _) = img(&l);
        let text = l
            .iter()
            .find(|f| matches!(&f.kind, FragmentKind::Text(t) if t == "x"))
            .expect("the text");
        assert_eq!(y + h, text.y, "its bottom margin edge on the baseline");
        assert!(x > text.x);
        let p = fragments(&l, "p")[0];
        assert!(p.h > 100.0, "{}", p.h);
        // The used display of an inline image is an inline-block's, of
        // any other a block's.
        let display = |css: &str| {
            let l = laid(IMG, css);
            let own = fragments(&l, "i")[0];
            own.style.display
        };
        assert_eq!(display(""), style::computed::Display::InlineBlock);
        assert_eq!(
            display("#i { display: inline-flex }"),
            style::computed::Display::InlineBlock
        );
        assert_eq!(
            display("#i { display: table }"),
            style::computed::Display::Block
        );
        assert_eq!(
            display("#i { display: table-cell }"),
            style::computed::Display::Block
        );
    }

    /// The marker's image fragment, and the text `x`'s fragment.
    fn marker_and_text(l: &Laid) -> (Option<&Fragment>, &Fragment) {
        use style::selector_matching::PseudoElement;
        let marker = l.iter().find(|f| {
            f.generated == Some(PseudoElement::Marker)
                && matches!(f.kind, FragmentKind::Replaced(_))
        });
        let text = l
            .iter()
            .find(|f| matches!(&f.kind, FragmentKind::Text(t) if t == "x"))
            .expect("the text");
        (marker, text)
    }

    #[test]
    fn an_image_marker_sits_on_the_baseline_seven_pixels_from_the_content() {
        let list = "ul { margin: 0; padding: 0 0 0 100px; list-style-image: url(10x10.png) }";
        let l = laid("<ul><li>x</li></ul>", list);
        let (marker, text) = marker_and_text(&l);
        let marker = marker.expect("an image marker");
        assert_eq!((marker.x, marker.w, marker.h), (83.0, 10.0, 10.0));
        assert_eq!(marker.y + marker.h, text.y, "its bottom on the baseline");
        let rtl = "ul { margin: 0; padding: 0 100px 0 0; list-style-image: url(10x10.png) }";
        let l = laid("<ul dir=rtl><li>x</li></ul>", rtl);
        assert_eq!(marker_and_text(&l).0.expect("an image marker").x, 707.0);
        // Inside: the content after it and the gap.
        let l = laid(
            "<ul><li>x</li></ul>",
            &format!("{list} li {{ list-style-position: inside }}"),
        );
        let (marker, text) = marker_and_text(&l);
        assert_eq!(marker.expect("an image marker").x, 100.0);
        assert_eq!(text.x, 117.0);
        // A gradient: a square half the font's ascent.
        let l = laid(
            "<ul><li>x</li></ul>",
            "ul { list-style-image: linear-gradient(red, blue) }",
        );
        let (marker, _) = marker_and_text(&l);
        let marker = marker.expect("a gradient marker");
        let fonts = FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the test font");
        let side = (fonts.metrics_for_text(" ", 16.0).ascent / 2.0).floor();
        assert_eq!((marker.w, marker.h), (side, side));
    }

    #[test]
    fn the_list_style_type_stands_in_for_an_image_coming_or_broken() {
        let shape = |css: &str| {
            let l = laid("<ul><li>x</li></ul>", css);
            let shape = l.iter().find_map(|f| match f.kind {
                FragmentKind::Marker(shape) => Some(shape),
                _ => None,
            });
            (shape, marker_and_text(&l).0.is_some())
        };
        let disc = Some(crate::MarkerShape::Disc);
        assert_eq!(
            shape("ul { list-style-image: url(later.png) }"),
            (disc, false)
        );
        assert_eq!(
            shape("ul { list-style-image: url(broken.png) }"),
            (disc, false)
        );
        assert_eq!(
            shape("ul { list-style-image: url(10x10.png) }"),
            (None, true)
        );
        assert_eq!(
            shape("ul { list-style: none url(later.png) }"),
            (None, false),
            "no type to stand in"
        );
        assert_eq!(
            shape("ul { list-style: none url(10x10.png) }"),
            (None, true)
        );
        // A marker's own content wins over the image.
        assert_eq!(
            shape("ul { list-style-image: url(10x10.png) } li::marker { content: '-' }"),
            (None, false)
        );
    }

    #[test]
    fn content_with_an_image_replaces_an_element_and_joins_a_pseudo_elements_text() {
        assert_eq!(
            size("<div id=i>text</div>", "#i { content: url(200x100.png) }"),
            (200.0, 100.0)
        );
        assert_eq!(
            size(
                "<span id=i>text</span>",
                "#i { content: url(200x100.png); width: 50px }"
            ),
            (50.0, 25.0)
        );
        assert_eq!(
            size("<div id=i>text</div>", "#i { content: url(later.png) }"),
            (0.0, 0.0)
        );
        assert_eq!(
            size(
                "<div id=i>text</div>",
                "#i { content: url(200x100.png); display: table-cell }"
            ),
            (200.0, 100.0),
            "a table part's display is a block's"
        );
        // A `::before` of text and an image: the image between, on the
        // line; one that cannot be shown is left out.
        let pieces = |css: &str| {
            use style::selector_matching::PseudoElement;
            let l = laid("<p id=p>a</p>", css);
            let mut found: Vec<(f32, String)> = l
                .iter()
                .filter(|f| f.generated == Some(PseudoElement::Before))
                .filter_map(|f| match &f.kind {
                    FragmentKind::Text(t) => Some((f.x, t.clone())),
                    FragmentKind::Replaced(_) => Some((f.x, format!("{}x{}", f.w, f.h))),
                    _ => None,
                })
                .collect();
            found.sort_by(|a, b| a.0.total_cmp(&b.0));
            found.into_iter().map(|(_, s)| s).collect::<Vec<_>>()
        };
        assert_eq!(
            pieces("#p::before { content: '[' url(200x100.png) ']' }"),
            ["[", "200x100", "]"]
        );
        assert_eq!(
            pieces("#p::before { content: '[' url(broken.png) ']' }"),
            ["[", "]"]
        );
        assert_eq!(
            pieces("#p::before { content: '\\fffc' url(10x10.png) }"),
            ["\u{fffd}", "10x10"],
            "a string's own object character is not an image's"
        );
    }

    #[test]
    fn exif_orientation_is_applied_unless_image_orientation_is_none() {
        let html = "<img id=i src=turned.jpg>";
        assert_eq!(size(html, ""), (100.0, 200.0));
        assert_eq!(size(html, "#i { image-orientation: none }"), (200.0, 100.0));
    }

    #[test]
    fn the_table_keeps_the_ratio_where_the_limits_allow() {
        let none = (0.0, None);
        assert_eq!(fit(200.0, 100.0, none, none), (200.0, 100.0));
        assert_eq!(fit(200.0, 100.0, (0.0, Some(100.0)), none), (100.0, 50.0));
        assert_eq!(fit(200.0, 100.0, none, (0.0, Some(50.0))), (100.0, 50.0));
        assert_eq!(fit(200.0, 100.0, (300.0, None), none), (300.0, 150.0));
        assert_eq!(fit(200.0, 100.0, none, (120.0, None)), (240.0, 120.0));
        // Both too large: the tighter limit wins.
        assert_eq!(
            fit(200.0, 100.0, (0.0, Some(100.0)), (0.0, Some(80.0))),
            (100.0, 50.0)
        );
        assert_eq!(
            fit(200.0, 100.0, (0.0, Some(180.0)), (0.0, Some(40.0))),
            (80.0, 40.0)
        );
        // Both too small: the larger minimum wins.
        assert_eq!(fit(20.0, 10.0, (40.0, None), (30.0, None)), (60.0, 30.0));
        assert_eq!(fit(20.0, 10.0, (80.0, None), (30.0, None)), (80.0, 40.0));
        // The limits fight: each side takes its own.
        assert_eq!(
            fit(20.0, 100.0, (40.0, None), (0.0, Some(50.0))),
            (40.0, 50.0)
        );
        assert_eq!(
            fit(200.0, 10.0, (0.0, Some(50.0)), (40.0, None)),
            (50.0, 40.0)
        );
        // A maximum below the minimum is the minimum.
        assert_eq!(fit(200.0, 100.0, (150.0, Some(100.0)), none), (150.0, 75.0));
        // Nothing to keep a ratio of.
        assert_eq!(fit(0.0, 0.0, (10.0, None), (5.0, None)), (10.0, 5.0));
    }
}
