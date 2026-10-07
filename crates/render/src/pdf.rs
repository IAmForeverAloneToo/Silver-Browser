//! The PDF a document prints to (M20.4): see `docs/design/media.md`,
//! "Print: the raster and the PDF".
//!
//! Each page is a page of the PDF at its page box's size, in points,
//! with its page area's raster on it as an image: RGB, eight bits a
//! channel, Flate-compressed, placed at the page area's origin at the
//! size its resolution says. The file names its title and the program
//! that made it, and carries no date, so that a page prints to the same
//! bytes each time. Its text is pictures of text, which cannot be
//! selected or searched.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref, TextStr};

/// Points in a CSS pixel: 72 to the inch against 96.
const POINTS_PER_PX: f32 = 72.0 / 96.0;

/// One page to write.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfPage {
    /// The page box's width and height, in CSS pixels.
    pub size: (f32, f32),
    /// Where the page area's top left corner is in the page box, in CSS
    /// pixels.
    pub origin: (f32, f32),
    /// The raster's resolution, in dots per inch.
    pub dpi: f32,
    /// The raster's width and height, in pixels.
    pub width: u32,
    pub height: u32,
    /// The raster, RGB, a row at a time from the top, three bytes a
    /// pixel.
    pub rgb: Vec<u8>,
}

/// The PDF of `pages`, in order, titled `title`.
pub fn write_pdf(pages: &[PdfPage], title: &str) -> Vec<u8> {
    let mut pdf = Pdf::new();
    let catalog = Ref::new(1);
    let tree = Ref::new(2);
    let info = Ref::new(3);
    // Each page takes three references: the page, its content and its
    // image.
    let page_ref = |index: usize, which: i32| Ref::new(4 + 3 * index as i32 + which);
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree)
        .kids((0..pages.len()).map(|index| page_ref(index, 0)))
        .count(pages.len() as i32);
    pdf.document_info(info)
        .title(TextStr(title))
        .producer(TextStr("Silver Browser"));
    let image_name = Name(b"Im0");
    for (index, page) in pages.iter().enumerate() {
        let (id, content_id, image_id) =
            (page_ref(index, 0), page_ref(index, 1), page_ref(index, 2));
        let width = page.size.0 * POINTS_PER_PX;
        let height = page.size.1 * POINTS_PER_PX;
        let mut writer = pdf.page(id);
        writer
            .media_box(Rect::new(0.0, 0.0, width, height))
            .parent(tree)
            .contents(content_id);
        writer.resources().x_objects().pair(image_name, image_id);
        writer.finish();

        let compressed = deflate(&page.rgb);
        let mut image = pdf.image_xobject(image_id, &compressed);
        image.filter(Filter::FlateDecode);
        image.width(page.width as i32);
        image.height(page.height as i32);
        image.color_space().device_rgb();
        image.bits_per_component(8);
        image.finish();

        // The raster at its resolution, its top left at the origin; PDF
        // measures up from the page's bottom.
        let shown_width = page.width as f32 * 72.0 / page.dpi;
        let shown_height = page.height as f32 * 72.0 / page.dpi;
        let left = page.origin.0 * POINTS_PER_PX;
        let bottom = height - page.origin.1 * POINTS_PER_PX - shown_height;
        let mut content = Content::new();
        content.save_state();
        content.transform([shown_width, 0.0, 0.0, shown_height, left, bottom]);
        content.x_object(image_name);
        content.restore_state();
        pdf.stream(content_id, &content.finish());
    }
    pdf.finish()
}

/// `data` compressed by zlib's deflate, which `FlateDecode` reads.
fn deflate(data: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    // Writing to a vector does not fail.
    encoder.write_all(data).expect("a vector takes every byte");
    encoder.finish().expect("a vector takes every byte")
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    fn page(size: (f32, f32), width: u32, height: u32, seed: u8) -> PdfPage {
        let rgb = (0..width * height * 3)
            .map(|i| (i as u8).wrapping_mul(seed))
            .collect();
        PdfPage {
            size,
            origin: (37.0, 37.0),
            dpi: 300.0,
            width,
            height,
            rgb,
        }
    }

    /// The streams of `pdf` that inflate, in order.
    fn streams(pdf: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut rest = pdf;
        // `endstream` holds `stream` too, but not after a line's end.
        while let Some(at) = find(rest, b"\nstream\n") {
            let body = &rest[at + 8..];
            let end = find(body, b"\nendstream").expect("a stream ends");
            let mut inflated = Vec::new();
            if flate2::read::ZlibDecoder::new(&body[..end])
                .read_to_end(&mut inflated)
                .is_ok()
            {
                out.push(inflated);
            }
            rest = &body[end..];
        }
        out
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    fn count(haystack: &[u8], needle: &[u8]) -> usize {
        haystack
            .windows(needle.len())
            .filter(|w| *w == needle)
            .count()
    }

    #[test]
    fn each_page_is_its_box_with_its_raster_at_the_origin() {
        let pages = [
            page((816.0, 1056.0), 20, 10, 3),
            page((1056.0, 816.0), 7, 5, 5),
        ];
        let pdf = write_pdf(&pages, "A (title)");
        assert!(pdf.starts_with(b"%PDF-"));
        assert_eq!(count(&pdf, b"/Type /Page\n"), 2);
        assert!(find(&pdf, b"/MediaBox [0 0 612 792]").is_some());
        assert!(find(&pdf, b"/MediaBox [0 0 792 612]").is_some());
        // Balanced parentheses stand in a PDF string; one alone is escaped.
        assert!(find(&pdf, b"/Title (A (title))").is_some());
        let unbalanced = write_pdf(&[], "a ) b");
        assert!(find(&unbalanced, b"/Title (a \\) b)").is_some());
        assert!(find(&pdf, b"/Producer (Silver Browser)").is_some());
        assert!(find(&pdf, b"/CreationDate").is_none());
        // The images inflate to the rasters given.
        let streams = streams(&pdf);
        assert!(streams.contains(&pages[0].rgb));
        assert!(streams.contains(&pages[1].rgb));
        // The first page's raster: 20 pixels at 300 dpi are 4.8 points
        // wide, its top 37px, 27.75pt, below the page's top.
        assert!(find(&pdf, b"/Width 20").is_some() && find(&pdf, b"/Height 10").is_some());
        let shown = format!("{} 0 0 {} {} {} cm", 4.8, 2.4, 27.75, 792.0 - 27.75 - 2.4);
        assert!(
            find(&pdf, shown.as_bytes()).is_some(),
            "{}",
            String::from_utf8_lossy(&pdf)
        );
    }

    #[test]
    fn the_same_pages_write_the_same_bytes() {
        let pages = [page((816.0, 1056.0), 9, 9, 7)];
        assert_eq!(write_pdf(&pages, "t"), write_pdf(&pages, "t"));
        let none = write_pdf(&[], "empty");
        assert_eq!(count(&none, b"/Type /Page\n"), 0);
        assert!(find(&none, b"/Count 0").is_some());
    }
}
