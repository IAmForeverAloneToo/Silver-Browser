//! M20's benchmark: media queries and printing, at 1280 by 800.
//!
//! - **Evaluating a page's media query lists.** A sheet of 1,000
//!   `@media` rules, their lists in every form the milestone takes —
//!   min- and max- features, ranges, `not`, `only`, lists, `or`,
//!   preferences, nested rules — over the mutation benchmark's page:
//!   what a resize or a change of the user's preferences asks before it
//!   reuses the page's styles, every list answered again and the
//!   answers compared (`Stylesheets::media_changed`).
//! - **A resize.** The mutation benchmark's page with a breakpoint at
//!   1,200 pixels, its paragraphs' margins narrower below it: the window
//!   resized across the breakpoint and back, which cascades the page
//!   again, and within it, which does not; each frame stage by stage.
//! - **A print.** The mutation benchmark's page printed to PDF on US
//!   Letter at 300 dots per inch: its layout, pagination and paint
//!   (`Engine::printed_pages`), its pagination alone
//!   (`layout::paginate`), its rasters, each compressed as it is drawn
//!   (`Engine::print_pages`, less the former), the PDF written from them
//!   (`render::pdf::write_pdf`), the whole (`Engine::print`), the pages,
//!   the file's size, and the process's peak memory, which a print holds
//!   one raw raster of at a time.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench media
//! ```

#![forbid(unsafe_code)]

use std::hint::black_box;
use std::time::{Duration, Instant};

use engine::{Engine, PrintOptions};
use style::{CascadeContext, Media, Stylesheets};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const PARAGRAPHS: usize = 1000;

/// The mutation benchmark's page: `n` paragraphs, and `extra` in its
/// style.
fn page(n: usize, extra: &str) -> String {
    let mut source = format!(
        "<style>body {{ margin: 16px; }} p {{ margin: 0 0 12px 0; }} {extra}</style><body>"
    );
    for i in 0..n {
        source.push_str(&format!(
            "<p>Paragraph {i}. The quick brown fox jumps over the lazy dog; \
             pack my box with five dozen liquor jugs.</p>"
        ));
    }
    source.push_str("</body>");
    source
}

/// A sheet of `n` `@media` rules, a list of each form in turn.
fn media_rules(n: usize) -> String {
    let mut sheet = String::new();
    for i in 0..n {
        let rule = format!(".m{i} {{ color: #3a5a7a; }}");
        let list = match i % 6 {
            0 => format!("(min-width: {}px)", 300 + i),
            1 => format!(
                "screen and (max-width: {}px) and (orientation: landscape)",
                600 + i
            ),
            2 => format!("({}px <= width < {}px)", 200 + i, 900 + i),
            3 => format!(
                "not print and (prefers-color-scheme: dark), (min-aspect-ratio: {}/1000)",
                500 + i
            ),
            4 => format!(
                "(prefers-reduced-motion: reduce) or (min-resolution: {}dppx)",
                1 + i % 3
            ),
            _ => {
                sheet.push_str(&format!(
                    "@media only screen {{ @media (height > {}px) {{ {rule} }} }}\n",
                    100 + i
                ));
                continue;
            }
        };
        sheet.push_str(&format!("@media {list} {{ {rule} }}\n"));
    }
    sheet
}

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort();
    values[values.len() / 2]
}

fn ms(d: Duration) -> String {
    format!("{:.2}ms", d.as_secs_f64() * 1000.0)
}

/// The process's peak resident memory, from Linux's `/proc`.
fn peak_memory() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|l| l.starts_with("VmHWM:"))
                .map(|l| l["VmHWM:".len()..].trim().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn evaluation() {
    let source = page(PARAGRAPHS, &media_rules(1000));
    let document = dom::Document::from_node(&html::parse_document(&source).expect("parses"));
    let sheets: Vec<css::Stylesheet> = document
        .subtree(document.root())
        .filter(|&n| document.tag(n) == Some("style"))
        .map(|n| css::parse_stylesheet(&document.text_content(n)).expect("the sheet parses"))
        .collect();
    let started = Instant::now();
    let stylesheets = Stylesheets::new(sheets);
    let prepared = started.elapsed();
    let media = Media::screen(WIDTH as f32, HEIGHT as f32);
    let context = CascadeContext {
        viewport_width: WIDTH as f32,
        viewport_height: HEIGHT as f32,
        media,
        ..CascadeContext::default()
    };
    let started = Instant::now();
    let styles = stylesheets.cascade(&document, &context);
    let cascaded = started.elapsed();
    let plain = page(PARAGRAPHS, "");
    let plain_document = dom::Document::from_node(&html::parse_document(&plain).expect("parses"));
    let plain_sheets = Stylesheets::new(vec![
        css::parse_stylesheet("body { margin: 16px; } p { margin: 0 0 12px 0; }")
            .expect("the sheet parses"),
    ]);
    let started = Instant::now();
    let plain_styles = plain_sheets.cascade(&plain_document, &context);
    let plain_cascaded = started.elapsed();
    black_box(&plain_styles);
    // Every list answered again at another width, as a resize asks.
    let resized = Media::screen(WIDTH as f32 - 1.0, HEIGHT as f32);
    let mut times = Vec::new();
    for _ in 0..1000 {
        let started = Instant::now();
        black_box(stylesheets.media_changed(&styles, black_box(&resized)));
        times.push(started.elapsed());
    }
    println!("Evaluating 1,000 @media lists over {PARAGRAPHS} paragraphs:");
    println!("  the sheets prepared: {}", ms(prepared));
    println!(
        "  the page's cascade: {}, against {} without the rules",
        ms(cascaded),
        ms(plain_cascaded)
    );
    println!(
        "  every list answered again and compared, median of 1,000: {:.1}µs",
        median(times).as_secs_f64() * 1e6
    );
}

fn resize() {
    let mut engine = Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    );
    engine.load_document(&page(
        PARAGRAPHS,
        "@media (width < 1200px) { p { margin: 0 0 8px 0; } }",
    ));
    engine.frame(WIDTH, HEIGHT);
    let mut across = Vec::new();
    let mut within = Vec::new();
    for _ in 0..10 {
        for width in [1100, WIDTH] {
            let runs = engine.style_runs();
            engine.frame(width, HEIGHT);
            assert_eq!(engine.style_runs(), runs + 1, "a breakpoint crossed");
            across.push(engine.frame_stats());
        }
        for width in [1270, WIDTH] {
            let runs = engine.style_runs();
            engine.frame(width, HEIGHT);
            assert_eq!(engine.style_runs(), runs, "no breakpoint crossed");
            within.push(engine.frame_stats());
        }
    }
    println!("Resizing the page of {PARAGRAPHS} paragraphs, medians of 20 frames:");
    for (what, stats) in [("across the breakpoint", across), ("within it", within)] {
        let stage =
            |f: fn(&engine::FrameStats) -> Duration| ms(median(stats.iter().map(f).collect()));
        println!(
            "  {what}: {} a frame; style {}, layout {}, paint {}, raster {}",
            stage(|s| s.total),
            stage(|s| s.style),
            stage(|s| s.layout),
            stage(|s| s.paint),
            stage(|s| s.raster)
        );
    }
}

fn print() {
    let mut engine = Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    );
    engine.load_document(&page(PARAGRAPHS, ""));
    let options = PrintOptions::default();
    let mut laid = Vec::new();
    let mut drawn = Vec::new();
    let mut written = Vec::new();
    let mut whole = Vec::new();
    let mut pagination = Vec::new();
    let mut pages = 0;
    let mut size = 0;
    for _ in 0..3 {
        let started = Instant::now();
        let printed = engine.printed_pages(&options).expect("prints");
        laid.push(started.elapsed());
        pages = printed.len();
        drop(printed);
        // Its pagination alone, on the layout at the first page's width.
        let layout = engine.layout(742.0).clone();
        let started = Instant::now();
        black_box(layout::paginate(layout, &|_| 982.0));
        pagination.push(started.elapsed());
        let started = Instant::now();
        let rasters = engine.print_pages(&options).expect("prints");
        drawn.push(started.elapsed());
        let started = Instant::now();
        let pdf = render::pdf::write_pdf(&rasters, "A thousand paragraphs");
        written.push(started.elapsed());
        size = pdf.len();
        drop((rasters, pdf));
        let started = Instant::now();
        black_box(engine.print(&options).expect("prints"));
        whole.push(started.elapsed());
    }
    let laid = median(laid);
    let drawn = median(drawn);
    println!("Printing the page of {PARAGRAPHS} paragraphs, medians of 3:");
    println!("  {pages} pages of US Letter at 300 dots per inch");
    println!("  layout, pagination and paint: {}", ms(laid));
    println!("  its pagination alone: {}", ms(median(pagination)));
    println!(
        "  the rasters, compressed as they are drawn: {}",
        ms(drawn.saturating_sub(laid))
    );
    println!("  the PDF written from them: {}", ms(median(written)));
    println!("  the whole print: {}", ms(median(whole)));
    println!(
        "  the file: {:.1} MiB, {:.0} KiB a page",
        size as f64 / (1024.0 * 1024.0),
        size as f64 / 1024.0 / pages as f64
    );
    println!("  the process's peak memory: {}", peak_memory());
}

fn main() {
    evaluation();
    resize();
    print();
}
