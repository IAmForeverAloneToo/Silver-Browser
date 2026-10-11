//! Printing (M20.4): the document shown, as a PDF. See
//! `docs/design/media.md`, "Print: what prints" and "Print: the raster
//! and the PDF".
//!
//! [`Engine::print`] makes the PDF without touching what the window
//! shows: a cascade of its own under the print medium, a layout of its
//! own at the first page's area width, the pages `layout::paginate`
//! cuts, and a software backend of its own drawing each page's area at
//! the print's resolution through a group that scales the page's list,
//! so that text is drawn at the size the scale makes. The document's
//! styles, layout, scroll offsets and the window's backend are left as
//! they were. A page lists the document as the viewport onto its slice
//! of the paginated document would show it, its fixed boxes included,
//! clipped to its area; the root's background covers the area alone and
//! the margins are white, as the reference browser prints them.
//!
//! The page area is rounded as the reference browser rounds it: the
//! document is laid out at the first page's area width rounded up to a
//! whole pixel, and cut into pages each as tall as its area rounded up,
//! the viewport units measuring the area rounded down, and each page is
//! drawn clipped to its area as it is (see [`PageBox::layout_area`]).

use std::cell::RefCell;

use dom::Document;
use paint::{DisplayItem, DisplayList, GroupKey, Viewport, paint_fragments_with};
use render::RenderBackend;
use render::SoftwareBackend;
use render::pdf::{PdfPage, write_pdf};
use style::page::{DEFAULT_MARGIN, LETTER};
use style::transform::Matrix;
use style::{CascadeContext, Media, MediaType, PageBox, PagePosition};

use crate::{Engine, FontMeasurer};

/// What a print is asked for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrintOptions {
    /// The page box of a page no `@page` rule sizes, in CSS pixels: US
    /// Letter unless given, as the reference browser prints.
    pub page_size: (f32, f32),
    /// The margin of a side of a page no `@page` rule sets, in CSS
    /// pixels: 28 points unless given, the reference browser's.
    pub margin: f32,
    /// The raster's resolution, in dots per inch: 300 unless given. A
    /// page past the backend's largest raster is drawn at the resolution
    /// that fits it.
    pub dpi: f32,
}

impl Default for PrintOptions {
    fn default() -> Self {
        PrintOptions {
            page_size: LETTER,
            margin: DEFAULT_MARGIN,
            dpi: 300.0,
        }
    }
}

/// Why a print was not made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintError {
    /// The page size, margin or resolution is not a finite length, or
    /// the size or resolution not above zero.
    Options,
}

impl std::fmt::Display for PrintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrintError::Options => {
                write!(f, "the print's page size, margin or resolution is not one")
            }
        }
    }
}

impl std::error::Error for PrintError {}

/// A page of a print, laid out and painted but not drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct PrintedPage {
    /// The page's box: its size and its margins, which place its area.
    pub page_box: PageBox,
    /// Which page it is: the first, a left or a right one, a blank one.
    pub position: PagePosition,
    /// What the page's area shows, in the area's coordinates, its top
    /// left corner at the origin; drawn clipped to the area.
    pub list: DisplayList,
}

impl Engine {
    /// The document shown, printed to a PDF by `options`: its cascade
    /// under the print medium, its own layout and pagination, each page
    /// rasterized; nothing the window shows changes.
    pub fn print(&self, options: &PrintOptions) -> Result<Vec<u8>, PrintError> {
        let pages = self.print_pages(options)?;
        Ok(write_pdf(&pages, &self.title()))
    }

    /// The pages of [`Engine::print`], drawn, before they are written:
    /// each page's box and its area's raster.
    pub fn print_pages(&self, options: &PrintOptions) -> Result<Vec<PdfPage>, PrintError> {
        let pages = self.printed_pages(options)?;
        let mut backend = SoftwareBackend::with_fonts(self.layout_fonts.clone());
        Ok(pages
            .into_iter()
            .map(|page| rasterize_page(&mut backend, page, options.dpi))
            .collect())
    }

    /// The pages [`Engine::print`] prints by `options`, before they are
    /// drawn: each page's box and what its area shows. The print's tests
    /// read the text on each page from these.
    pub fn printed_pages(&self, options: &PrintOptions) -> Result<Vec<PrintedPage>, PrintError> {
        let (width, height) = options.page_size;
        let valid = |v: f32| v.is_finite() && v > 0.0;
        if !valid(width) || !valid(height) || !valid(options.dpi) || !options.margin.is_finite() {
            return Err(PrintError::Options);
        }
        let media = Media {
            media_type: MediaType::Print,
            width,
            height,
            device_width: width,
            device_height: height,
            resolution: options.dpi / 96.0,
            preferences: self.preferences,
            scripting: self.scripting,
        };
        let sheets = &self.shown.stylesheets;
        let document = &self.shown.document;
        let fonts = FontMeasurer(&self.layout_fonts);
        let first = PagePosition {
            first: true,
            ..PagePosition::default()
        };
        let page_box = |rem: f32, position: PagePosition| {
            sheets
                .page_style(&media, rem, position)
                .page_box(options.page_size, options.margin)
        };
        // The viewport is the first page's area, which `@page` rules may
        // size in the root's `em`: cascaded once against the area with
        // the initial font size, and again if the root's makes another
        // area and a viewport unit was used.
        let initial = style::ComputedStyle::default().font_size_px;
        let mut first_box = page_box(initial, first);
        // The screen's animations as they stand at the timeline's time
        // (M22): the print starts none of the screen's, and draws one
        // only its own styles name at its start.
        let animations = RefCell::new(self.shown.animations.borrow().clone());
        let context = |page_box: &PageBox| {
            let (width, height) = page_box.viewport();
            CascadeContext {
                viewport_width: width,
                viewport_height: height,
                fonts: Some(&fonts),
                language: self.shown.page.language(),
                media,
                animations: Some(&animations),
            }
        };
        let mut styles = sheets.cascade(document, &context(&first_box));
        let rem = root_font_size(document, &styles).unwrap_or(initial);
        let measured = page_box(rem, first);
        if measured != first_box {
            if measured.viewport() != first_box.viewport() && styles.uses_viewport_units {
                styles = sheets.cascade(document, &context(&measured));
            }
            first_box = measured;
        }
        let context = context(&first_box);
        let recascade = sheets.recascader(document, &styles, &context);
        let boxes =
            layout::build_box_tree_showing(document, &styles, &recascade, &self.shown.images);
        let (width, height) = first_box.layout_area();
        let laid = layout::layout_document_in(&boxes, width, Some(height), &self.layout_fonts);
        let paginated = layout::paginate(laid, &|position| page_box(rem, position).layout_area().1);
        let pages = paginated
            .pages
            .iter()
            .map(|page| {
                let viewport = Viewport {
                    width,
                    height: page.height,
                    scroll_y: page.top,
                };
                let mut list =
                    paint_fragments_with(&paginated.roots, viewport, &(), &self.shown.images);
                // A print waits for no decode: its images are decoded
                // now (M23.2).
                for item in &mut list.items {
                    if let DisplayItem::Image { image, .. } | DisplayItem::Pattern { image, .. } =
                        item
                        && image.pixels.is_none()
                    {
                        // A canvas prints its bitmap (M30.3).
                        image.pixels = match paint::canvas_node(image.id) {
                            Some(node) => self.shown.scripts.canvas_image(node),
                            None => self.shown.images.decoded_now(image.id),
                        };
                    }
                }
                PrintedPage {
                    page_box: page_box(rem, page.position),
                    position: page.position,
                    list,
                }
            })
            .collect();
        Ok(pages)
    }
}

/// The root element's font size, which `em` in `@page` rules is of.
fn root_font_size(document: &Document, styles: &style::Styles) -> Option<f32> {
    let root = document.document_element()?;
    styles.get(root).map(|style| style.font_size_px)
}

/// The scale an area `width` by `height` CSS pixels is drawn at, `scale`
/// or what brings it under the backend's largest raster, and the
/// raster's size in pixels, a pixel at least each way.
fn fitting(width: f32, height: f32, scale: f32) -> (f32, u32, u32) {
    let largest = render::MAX_FRAME_PIXELS as f32;
    let pixels = |scale: f32| {
        (
            (width * scale).ceil().max(1.0),
            (height * scale).ceil().max(1.0),
        )
    };
    let (w, h) = pixels(scale);
    let scale = if w * h > largest {
        // Down to the largest the backend draws, a little under for the
        // rounding up.
        scale * (largest / (w * h)).sqrt() * 0.999
    } else {
        scale
    };
    let (w, h) = pixels(scale);
    (scale, w as u32, h as u32)
}

/// `page`'s area, its list drawn at `dpi`, or at the resolution that fits
/// the backend's largest raster.
fn rasterize_page(backend: &mut SoftwareBackend, page: PrintedPage, dpi: f32) -> PdfPage {
    let PrintedPage { page_box, list, .. } = page;
    let (area_width, area_height) = page_box.area();
    let (scale, width, height) = fitting(area_width, area_height, dpi / 96.0);
    // The page's list, scaled to the raster and clipped to the area.
    let mut items = Vec::with_capacity(list.items.len() + 4);
    items.push(DisplayItem::PushGroup {
        matrix: Matrix::scale(scale, scale, 1.0),
        opacity: 1.0,
        key: GroupKey::Document(u32::MAX),
        layer: None,
    });
    items.push(DisplayItem::PushClip {
        x: 0.0,
        y: 0.0,
        w: area_width,
        h: area_height,
    });
    // A page is drawn once: its groups are drawn through, none kept as
    // a layer to composite, which is for frames to come (M19), so that
    // text and edges are drawn at the print's resolution rather than
    // resampled to it.
    items.extend(list.items.into_iter().map(|item| match item {
        DisplayItem::PushGroup {
            matrix,
            opacity,
            key,
            ..
        } => DisplayItem::PushGroup {
            matrix,
            opacity,
            key,
            layer: None,
        },
        other => other,
    }));
    items.push(DisplayItem::PopClip);
    items.push(DisplayItem::PopGroup);
    let rgba = backend.rasterize(&DisplayList { items }, width, height);
    // Premultiplied RGBA over white, which an opaque canvas leaves as it
    // is.
    let rgb: Vec<u8> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            let under = 255 - p[3];
            [
                p[0].saturating_add(under),
                p[1].saturating_add(under),
                p[2].saturating_add(under),
            ]
        })
        .collect();
    PdfPage::new(
        (page_box.width, page_box.height),
        page_box.origin(),
        scale * 96.0,
        (width, height),
        &rgb,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(html: &str) -> Engine {
        let mut engine = Engine::with_fonts(
            text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
                .expect("the bundled font parses"),
        );
        engine.load_document(html);
        engine
    }

    /// A small print, at 24 dots per inch, a quarter of a CSS pixel a
    /// dot: fast, and enough to tell boxes apart.
    fn small() -> PrintOptions {
        PrintOptions {
            dpi: 24.0,
            ..PrintOptions::default()
        }
    }

    /// The colour of the raster of `page` at the page area's CSS pixel
    /// `(x, y)`.
    fn at(page: &PdfPage, x: f32, y: f32) -> [u8; 3] {
        let scale = page.dpi / 96.0;
        let (px, py) = ((x * scale) as usize, (y * scale) as usize);
        let i = (py * page.width as usize + px) * 3;
        let rgb = page.rgb();
        [rgb[i], rgb[i + 1], rgb[i + 2]]
    }

    const RED: [u8; 3] = [255, 0, 0];
    const GREEN: [u8; 3] = [0, 255, 0];
    const BLUE: [u8; 3] = [0, 0, 255];
    const WHITE: [u8; 3] = [255, 255, 255];

    #[test]
    fn an_image_prints_though_no_frame_drew_it() {
        use crate::testing::{Tab, html, serve};
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, 100, 100);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("a header");
        writer
            .write_image_data(&[255, 0, 0].repeat(100 * 100))
            .expect("the pixels");
        writer.finish().expect("written");
        let server = serve(move |path| match path {
            "/page.html" => Some(html(
                "<body style='margin: 0'><img src=a.png>\
                 <div style='height: 50px; background: url(a.png) no-repeat'></div>\
                 <div style='height: 50px; background: linear-gradient(red, red)'></div></body>",
            )),
            "/a.png" => Some(net::test_server::Reply::ok("image/png", &png)),
            _ => None,
        });
        let mut tab = Tab::new();
        let url = server.url("/page.html");
        tab.engine.navigate(url.clone());
        tab.wait_until(|engine| {
            engine.url() == Some(&url) && engine.load_state() == &crate::LoadState::Loaded
        });
        let pages = tab.engine.print_pages(&small()).expect("printed");
        assert_eq!(at(&pages[0], 50.0, 50.0), RED);
        assert_eq!(at(&pages[0], 150.0, 50.0), WHITE);
        // The background below it, decoded now as the image is.
        let below = 104.0;
        assert_eq!(at(&pages[0], 50.0, below + 20.0), RED);
        assert_eq!(at(&pages[0], 150.0, below + 20.0), WHITE);
        // A gradient, rendered as paint rendered it.
        assert_eq!(at(&pages[0], 150.0, below + 70.0), RED);
    }

    #[test]
    fn a_short_page_prints_on_one_letter_page_with_the_default_margins() {
        let e = engine("<style>html { background: #0f0 }</style><p>x</p>");
        let pages = e.print_pages(&PrintOptions::default()).expect("printed");
        assert_eq!(pages.len(), 1);
        let page = &pages[0];
        assert_eq!(page.size, (816.0, 1056.0));
        assert_eq!(page.origin, (37.0, 37.0));
        // The page area, 741.33 by 981.33 pixels, at 300 dpi.
        assert_eq!((page.width, page.height), (2317, 3067));
        assert_eq!(page.dpi, 300.0);
        // The root's background covers the area; the margins are the
        // page's own white, outside the raster.
        assert_eq!(at(page, 700.0, 900.0), GREEN);
        let pdf = e.print(&small()).expect("printed");
        assert!(pdf.starts_with(b"%PDF-"));
        assert_eq!(pdf, e.print(&small()).expect("printed"), "the same bytes");
    }

    #[test]
    fn print_styles_and_page_rules_apply_and_the_screen_does_not_change() {
        let mut e = engine(
            "<style>.x { height: 50px; background: #00f }
             @media print { .x { background: #f00 } @page { size: landscape; margin: 0 } }
             </style><div class=x></div>",
        );
        let before = e.frame(200, 100);
        let runs = (e.style_runs(), e.layout_runs());
        let pages = e.print_pages(&small()).expect("printed");
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].size, (1056.0, 816.0));
        assert_eq!(pages[0].origin, (0.0, 0.0));
        assert_eq!(at(&pages[0], 20.0, 20.0), RED);
        assert_eq!(at(&pages[0], 20.0, 80.0), WHITE);
        // The window's styles, layout and frame are as they were.
        assert_eq!((e.style_runs(), e.layout_runs()), runs);
        assert!(!e.needs_frame());
        assert_eq!(e.frame(200, 100), before);
        let div = crate::testing::element(&e, "div");
        let style = e.styles().get(div).cloned();
        assert_eq!(
            style.expect("styled").background_color,
            style::computed::Color::rgb8(0, 0, 255)
        );
    }

    #[test]
    fn a_print_shows_the_screens_animations_at_the_timelines_time() {
        // The first box's animation has ended at the time of the print,
        // filled forwards; the second box's animation only the print
        // styles name, drawn at its start; the third box's is the
        // screen's, paused half way.
        let mut e = engine(
            "<style>body { margin: 0 } @page { margin: 0 }
             div { width: 40px; height: 40px; background: #f00 }
             @keyframes in { from { margin-left: 0px } to { margin-left: 200px } }
             @keyframes off { from { margin-left: 300px } to { margin-left: 0px } }
             .a { animation: in 1s linear forwards }
             .c { animation: in 1s linear -0.5s paused }
             @media print { .b { animation: off 1s linear } }</style>
             <div class=a></div><div class=b></div><div class=c></div>",
        );
        let at_time = |e: &mut Engine, ms: u64| {
            e.frame_at(
                200,
                100,
                crate::FrameTime::at(std::time::Duration::from_millis(ms)),
            );
        };
        at_time(&mut e, 0);
        at_time(&mut e, 2000);
        let animations = e.shown.animations.borrow().len();
        let pages = e.print_pages(&small()).expect("printed");
        assert_eq!(at(&pages[0], 220.0, 20.0), RED);
        assert_eq!(at(&pages[0], 20.0, 20.0), WHITE);
        assert_eq!(at(&pages[0], 320.0, 60.0), RED);
        assert_eq!(at(&pages[0], 120.0, 100.0), RED);
        // The screen's animations are as they were.
        assert_eq!(e.shown.animations.borrow().len(), animations);
        assert!(!e.needs_frame());
    }

    #[test]
    fn a_long_page_prints_across_pages_with_its_fixed_box_on_each() {
        let e = engine(
            "<style>body { margin: 0 } @page { margin: 0 }
             .f { position: fixed; top: 0; left: 0; width: 40px; height: 40px; background: #00f }
             .b { height: 600px; margin: 0; background: #0f0 }</style>
             <div class=f></div><div class=b></div><div class=b></div><div class=b></div>",
        );
        let pages = e.print_pages(&small()).expect("printed");
        // 1800px of green on pages 1056px tall, cut where they end.
        assert_eq!(pages.len(), 2);
        for page in &pages {
            assert_eq!(at(page, 20.0, 20.0), BLUE, "the fixed box");
            assert_eq!(at(page, 400.0, 400.0), GREEN);
        }
        assert_eq!(at(&pages[1], 400.0, 900.0), WHITE, "below the content");
    }

    #[test]
    fn forced_breaks_begin_pages_and_a_side_adds_a_blank_one() {
        let e = engine(
            "<style>@page { margin: 0 } div { height: 50px; background: #f00 }
             .r { break-before: right }</style><div></div><div class=r></div>",
        );
        let pages = e.print_pages(&small()).expect("printed");
        assert_eq!(pages.len(), 3);
        assert_eq!(at(&pages[1], 20.0, 20.0), WHITE, "the blank left page");
        assert_eq!(at(&pages[2], 20.0, 20.0), RED);
    }

    #[test]
    fn a_page_is_drawn_at_the_prints_resolution_not_resampled_to_it() {
        // The document's scroll group is a layer on the screen (M19); in
        // a print it is drawn through at four dots to the CSS pixel, so
        // that a box's edge falls between two dots, not across several.
        let e = engine(
            "<style>body { margin: 0 } @page { margin: 0 }
             div { width: 10px; height: 10px; background: #000 }</style><div></div>",
        );
        let options = PrintOptions {
            dpi: 384.0,
            ..PrintOptions::default()
        };
        let pages = e.print_pages(&options).expect("printed");
        assert_eq!(pages[0].dpi, 384.0);
        assert_eq!(at(&pages[0], 9.9, 5.0), [0, 0, 0]);
        assert_eq!(at(&pages[0], 10.1, 5.0), WHITE);
        assert_eq!(at(&pages[0], 5.0, 10.1), WHITE);
    }

    #[test]
    fn the_page_area_is_rounded_as_the_reference_browser_rounds_it() {
        // Letter at the default margins: an area of 741.33 by 981.33,
        // laid out 742 wide and cut every 982 pixels, the viewport units
        // measuring 741 by 981.
        let page = |spacer: &str| {
            engine(&format!(
                "<style>body {{ margin: 0 }} p {{ margin: 0 }} .s {{ height: {spacer} }}
                 .w {{ margin-left: 50vw }} .h {{ margin-left: 50% }}</style>
                 <p class=w>w</p><p class=h>h</p><div class=s></div><p>x</p>"
            ))
            .printed_pages(&PrintOptions::default())
            .expect("printed")
        };
        let lefts = |page: &PrintedPage| -> Vec<(String, f32)> {
            page.list
                .items
                .iter()
                .filter_map(|item| match item {
                    DisplayItem::Text { text, x, .. } => Some((text.clone(), *x)),
                    _ => None,
                })
                .collect()
        };
        // The last line ends at 982: it fits.
        let pages = page("925px");
        assert_eq!(pages.len(), 1);
        let (width, height) = pages[0].page_box.area();
        assert!((width - 741.333).abs() < 0.01 && (height - 981.333).abs() < 0.01);
        assert_eq!(
            lefts(&pages[0])[..2],
            [("w".to_string(), 370.5), ("h".to_string(), 371.0)]
        );
        // A little lower, it does not.
        assert_eq!(page("925.4px").len(), 2);
    }

    #[test]
    fn a_page_too_large_is_printed_at_a_resolution_that_fits_and_bad_options_are_refused() {
        assert_eq!(fitting(741.5, 981.25, 3.125), (3.125, 2318, 3067));
        let (scale, w, h) = fitting(20_000.0, 20_000.0, 3.125);
        assert!(w as u64 * h as u64 <= render::MAX_FRAME_PIXELS);
        assert!(scale < 3.125 && w > 8000);
        assert_eq!(fitting(0.1, 0.1, 1.0), (1.0, 1, 1));
        let e = engine("<p>x</p>");
        for bad in [
            PrintOptions {
                dpi: 0.0,
                ..PrintOptions::default()
            },
            PrintOptions {
                page_size: (f32::NAN, 10.0),
                ..PrintOptions::default()
            },
            PrintOptions {
                margin: f32::INFINITY,
                ..PrintOptions::default()
            },
        ] {
            assert_eq!(e.print(&bad), Err(PrintError::Options));
        }
    }
}
