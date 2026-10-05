//! M19.3's acceptance: the GPU backend against the software backend.
//!
//! Every page under `tests/reference/` and `tests/layouts/`, and the
//! pages below of text, transforms, opacity, 3D and `will-change`, is
//! rendered by an engine on each backend at 800 by 600, as loaded and
//! again scrolled down where the page scrolls, and the GPU's frame is
//! held to the software's under the equivalence rule of
//! `docs/design/compositing.md`: a pixel whose neighbours in the
//! software frame are all its own colour, the inside of a shape, must
//! match within two levels, the rounding of a blend; a pixel on an
//! edge within sixty-four, the difference between tiny-skia's
//! supersampled coverage and the shader's exact area, and between its
//! anti-aliasing and the GPU's bilinear sampling at a layer's edge.
//! When a frame fails, both frames and a difference image are written
//! under `target/equivalence/` for a look, and the failure names the
//! pixels.
//!
//! The suite runs where an adapter exists, on Mesa's software Vulkan
//! driver in CI's Linux job; where none does it says so and passes.

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use engine::{Backend, Engine};
use render::gpu::{self, GpuBackend};

const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

/// How far the scrolled frame scrolls, in px: past the first tile of
/// the document's layer, so that the second frame composites tiles the
/// first did not draw.
const SCROLL: f32 = 300.0;

/// The pages of this milestone's own: text, which the reference pages
/// leave out, under the transforms, the opacity and the 3D of M19.
const PAGES: &[(&str, &str)] = &[
    (
        "text",
        "<h1>A heading</h1><p>A paragraph of text in the engine's font, long enough to wrap \
         onto a second line and a third one too, so that the lines are many.</p>\
         <p style=\"font-size: 11px; color: #345\">Smaller text in a colour.</p>\
         <div style=\"overflow: hidden; width: 120px; height: 24px; background: #eee\">\
         <p style=\"margin: 0\">Clipped text that overflows its box on the right and below</p></div>",
    ),
    (
        "translated",
        "<div style=\"transform: translate(10.5px, 3.25px); border: 2px solid #c33; \
         background: #eef; width: 200px; padding: 8px\">Translated by a fraction of a pixel, \
         drawn through with its border and text.</div>",
    ),
    (
        "scaled",
        "<div style=\"transform: scale(1.5); transform-origin: 0 0; border: 1px solid #000; \
         width: 200px; padding: 4px\">Scaled text, rasterized at the scaled size.</div>\
         <div style=\"transform: scale(1.25, 0.8); transform-origin: 0 0; margin-top: 120px; \
         background: #dfd; width: 200px\">Unevenly scaled text.</div>",
    ),
    (
        "opacity",
        "<div style=\"opacity: 0.5; background: #08f; width: 220px; padding: 6px\">\
         <p style=\"margin: 0\">Faded text over a faded background.</p>\
         <div style=\"opacity: 0.6; background: #f80; width: 100px; height: 40px\"></div></div>",
    ),
    (
        "rotated",
        "<div style=\"transform: rotate(7deg); border: 3px solid #393; width: 240px; \
         padding: 10px; margin: 30px; background: #efe\">A rotated card with text, a layer \
         resampled from its own plane.</div>",
    ),
    (
        "skewed",
        "<div style=\"transform: skewX(20deg); background: #fc0; width: 200px; padding: 6px; \
         margin-left: 60px; border-left: 4px solid #000\">Skewed text</div>",
    ),
    (
        "perspective",
        "<div style=\"perspective: 400px; width: 300px; height: 200px\">\
         <div style=\"transform: rotateY(30deg); background: #4a6; width: 200px; height: 120px; \
         border: 2px solid #000; padding: 6px\">Turned in 3D</div></div>",
    ),
    (
        "preserve-3d",
        "<div style=\"transform-style: preserve-3d; perspective: 500px; width: 300px; \
         height: 200px; position: relative\">\
         <div style=\"transform: translateZ(40px); background: #c44; width: 150px; height: 100px; \
         position: absolute\"></div>\
         <div style=\"transform: rotateX(45deg); background: #44c; width: 150px; height: 100px; \
         position: absolute; left: 60px; top: 40px\"></div></div>",
    ),
    (
        "will-change",
        "<div style=\"will-change: transform; background: #ddd; width: 200px; padding: 5px; \
         margin-left: 3.5px\">Promoted text in a layer at a fractional offset</div>",
    ),
    (
        "mitres",
        "<div style=\"transform: rotate(-5deg); border-width: 6px; border-style: solid; \
         border-color: red green blue orange; width: 200px; height: 80px; margin: 20px\"></div>\
         <div style=\"border-width: 5px 10px; border-style: solid; border-color: #333 #999; \
         width: 150px; height: 40px; opacity: 0.7\"></div>",
    ),
    (
        "clipped-faded-scaled",
        "<div style=\"transform: translate(20px, 10px) scale(1.2); transform-origin: 0 0; \
         overflow: hidden; width: 150px; height: 60px; opacity: 0.8; background: #fdd\">\
         <p style=\"margin: 0\">Overflowing text inside a scaled, clipped and faded box that \
         goes on and on</p></div>",
    ),
];

/// A long page with a fixed header, which scrolls.
fn fixed_page() -> String {
    let mut page = String::from(
        "<div style=\"position: fixed; top: 0; left: 0; right: 0; height: 40px; \
         background: #333; color: #fff; padding: 8px\">A fixed header</div>\
         <div style=\"margin-top: 60px\">",
    );
    for i in 0..60 {
        page.push_str(&format!(
            "<p style=\"margin: 4px 0; background: #f{:x}f\">Paragraph {i} of the long page, \
             scrolled under the header.</p>",
            i % 16
        ));
    }
    page.push_str("</div>");
    page
}

/// An engine with the bundled font: the same glyphs on every machine.
fn fonts() -> text::FontCollection {
    text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).expect("the bundled font parses")
}

/// The pages under `dir`, by name, in name order.
fn pages_in(dir: &str) -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
    let mut pages: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("the directory exists")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "html"))
        .collect();
    pages.sort();
    pages
        .into_iter()
        .map(|path| {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .expect("a page name")
                .to_string();
            (name, fs::read_to_string(&path).expect("the page is read"))
        })
        .collect()
}

fn write_png(path: &Path, rgb: &[u8]) {
    let file = File::create(path).expect("the output directory is writable");
    let mut encoder = png::Encoder::new(file, WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(rgb))
        .expect("the image is written");
}

fn rgb(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect()
}

/// Compares the GPU's frame with the software's: `None` when they are
/// equivalent, else a description, with the frames written for a look.
fn compare(name: &str, software: &[u8], on_gpu: &[u8], out_dir: &Path) -> Option<String> {
    let found = gpu::compare(software, on_gpu, WIDTH, HEIGHT);
    if found.differing == 0 {
        if found.tolerated > 0 {
            println!("{name}: {} pixels within the tolerances", found.tolerated);
        }
        return None;
    }
    fs::create_dir_all(out_dir).expect("the output directory is created");
    write_png(
        &out_dir.join(format!("{name}.software.png")),
        &rgb(software),
    );
    write_png(&out_dir.join(format!("{name}.gpu.png")), &rgb(on_gpu));
    write_png(&out_dir.join(format!("{name}.diff.png")), &found.diff);
    let (x, y, s, g) = found.first.expect("some pixel differs");
    Some(format!(
        "{name}: {} pixels differ past the tolerances within {:?}, {} more within them; \
         the first at ({x}, {y}) is {s:02x?} in software and {g:02x?} on the GPU; see {}",
        found.differing,
        found.bounds,
        found.tolerated,
        out_dir.display()
    ))
}

#[test]
fn the_gpu_backend_draws_every_page_as_the_software_backend_does() {
    let fonts = fonts();
    let backend = match GpuBackend::with_fonts(fonts.clone()) {
        Ok(backend) => backend,
        Err(e) => {
            println!("no GPU adapter here ({e}): the equivalence suite is skipped");
            return;
        }
    };
    println!("on {}", backend.adapter());
    let mut on_gpu = Engine::with_backend(fonts.clone(), Box::new(backend));
    let mut software = Engine::with_fonts(fonts);
    assert_eq!(on_gpu.backend(), Backend::Gpu);
    assert_eq!(software.backend(), Backend::Software);
    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/equivalence");
    let fixed = fixed_page();
    let mut pages: Vec<(String, String)> = PAGES
        .iter()
        .map(|(name, body)| (name.to_string(), body.to_string()))
        .collect();
    pages.push(("fixed-header".to_string(), fixed));
    pages.extend(pages_in("tests/reference"));
    pages.extend(pages_in("tests/layouts"));
    let mut failures = Vec::new();
    for (name, source) in &pages {
        software.load_document(source);
        on_gpu.load_document(source);
        let a = software.frame(WIDTH, HEIGHT);
        let b = on_gpu.frame(WIDTH, HEIGHT);
        failures.extend(compare(name, &a, &b, &out_dir));
        if software.max_scroll() > 0.0 {
            software.set_scroll(SCROLL);
            on_gpu.set_scroll(SCROLL);
            let a = software.frame(WIDTH, HEIGHT);
            let b = on_gpu.frame(WIDTH, HEIGHT);
            failures.extend(compare(&format!("{name}.scrolled"), &a, &b, &out_dir));
            // Back up, which composites the tiles kept.
            software.set_scroll(0.0);
            on_gpu.set_scroll(0.0);
            let a = software.frame(WIDTH, HEIGHT);
            let b = on_gpu.frame(WIDTH, HEIGHT);
            failures.extend(compare(&format!("{name}.back"), &a, &b, &out_dir));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
