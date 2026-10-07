//! M21's benchmark (the values note, "Performance and fuzzing"): what
//! custom properties, math functions, counters and justification cost a
//! page, at 1280 by 800, each stage the median of ten runs:
//!
//! - **A theme.** The mutation benchmark's page cascaded with its
//!   colours, sizes and spacing in custom properties on `:root`, read by
//!   `var()` in its rules: a theme of 8 properties, one of 200 of which
//!   the rules read the same 8, and that one with every paragraph
//!   declaring one of its own, which has each copy its parent's
//!   properties on write and compute its own; against the same page with
//!   the values written out.
//! - **Chains at the substitution bounds.** An element whose custom
//!   properties double the last one thirty times, which end at
//!   [`MAX_SUBSTITUTION`](style::custom::MAX_SUBSTITUTION); a chain of
//!   10,000 properties each naming the next, and the same closed into a
//!   cycle, which the resolver walks on a stack of its own; and a page
//!   built to exhaust
//!   [`MAX_CASCADE_SUBSTITUTION`](style::custom::MAX_CASCADE_SUBSTITUTION),
//!   1,000 elements each declaring the doubling chain, with the memory
//!   its cascade takes at its peak.
//! - **Math functions.** The box tree and layout of 1,000 boxes sized by
//!   `calc()`, `min()`, `max()` and `clamp()` of percentages and lengths,
//!   against the same boxes sized by the lengths they come to.
//! - **Counters.** The box tree and layout of an ordered list of 1,000
//!   items, each holding a list of two numbered by `counters()` in their
//!   markers' content, against the same elements as blocks, with neither
//!   counters nor markers.
//! - **Justification.** The layout of 1,000 paragraphs of six lines or
//!   so, justified, and justified by `inter-character`, against the same
//!   left-aligned.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench values
//! ```

use std::hint::black_box;
use std::time::{Duration, Instant};

use dom::Document;
use layout::DocumentLayout;
use style::computed::{BOTTOM, LEFT, LengthPercentage, LengthPercentageAuto};
use style::{CascadeContext, Media, Styles, Stylesheets};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const PARAGRAPHS: usize = 1000;
const RUNS: usize = 10;

/// The text of the mutation benchmark's paragraph `i`.
fn sentence(i: usize) -> String {
    format!(
        "Paragraph {i}. The quick brown fox jumps over the lazy dog; \
         pack my box with five dozen liquor jugs."
    )
}

/// A page of `body` under `sheet`.
fn page(sheet: &str, body: &str) -> String {
    format!("<style>{sheet}</style><body>{body}</body>")
}

/// The mutation benchmark's paragraphs, `n` of them.
fn paragraphs(n: usize) -> String {
    (0..n).map(|i| format!("<p>{}</p>", sentence(i))).collect()
}

fn median(mut values: Vec<Duration>) -> Duration {
    values.sort();
    values[values.len() / 2]
}

fn ms(d: Duration) -> String {
    format!("{:.2}ms", d.as_secs_f64() * 1000.0)
}

/// A figure of the process's memory from Linux's `/proc`, in kilobytes:
/// `VmRSS` resident now, `VmHWM` at the peak.
fn memory(field: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with(field))?;
    line[field.len()..]
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()
}

/// Has the peak resident memory start again from what is resident now
/// (Linux's `clear_refs`); whether it did.
fn reset_peak_memory() -> bool {
    std::fs::write("/proc/self/clear_refs", "5").is_ok()
}

fn context() -> CascadeContext<'static> {
    CascadeContext {
        viewport_width: WIDTH as f32,
        viewport_height: HEIGHT as f32,
        media: Media::screen(WIDTH as f32, HEIGHT as f32),
        ..CascadeContext::default()
    }
}

/// A page parsed, its sheets prepared.
struct Page {
    document: Document,
    stylesheets: Stylesheets,
}

impl Page {
    fn parse(source: &str) -> Self {
        let document = Document::from_node(&html::parse_document(source).expect("parses"));
        let sheets = document
            .subtree(document.root())
            .filter(|&n| document.tag(n) == Some("style"))
            .map(|n| css::parse_stylesheet(&document.text_content(n)).expect("the sheet parses"))
            .collect();
        Page {
            document,
            stylesheets: Stylesheets::new(sheets),
        }
    }

    fn cascade(&self) -> Styles {
        self.stylesheets.cascade(&self.document, &context())
    }

    /// The first element named `tag`.
    fn first(&self, tag: &str) -> dom::NodeId {
        let document = &self.document;
        document
            .subtree(document.root())
            .find(|&n| document.tag(n) == Some(tag))
            .expect("the page has one")
    }
}

/// The medians of a page's stages over [`RUNS`] runs, and its last
/// layout.
struct Stages {
    cascade: Duration,
    boxes: Duration,
    layout: Duration,
    laid_out: DocumentLayout,
}

impl Stages {
    fn of(source: &str, fonts: &text::FontCollection) -> Self {
        let page = Page::parse(source);
        let context = context();
        let (mut cascades, mut trees, mut layouts) = (Vec::new(), Vec::new(), Vec::new());
        let mut laid_out = None;
        for _ in 0..RUNS {
            let started = Instant::now();
            let styles = page.cascade();
            cascades.push(started.elapsed());
            let recascade = page
                .stylesheets
                .recascader(&page.document, &styles, &context);
            let started = Instant::now();
            let boxes = layout::build_box_tree(&page.document, &styles, &recascade);
            trees.push(started.elapsed());
            let started = Instant::now();
            laid_out = Some(layout::layout_document(&boxes, WIDTH as f32, fonts));
            layouts.push(started.elapsed());
        }
        Stages {
            cascade: median(cascades),
            boxes: median(trees),
            layout: median(layouts),
            laid_out: laid_out.expect("laid out"),
        }
    }

    fn line(&self, what: &str) {
        println!(
            "  {what}: cascade {}, box tree {}, layout {}",
            ms(self.cascade),
            ms(self.boxes),
            ms(self.layout)
        );
    }
}

/// A theme of `n` custom properties, the first 8 the page's own.
fn theme(n: usize) -> String {
    let mut sheet = String::from(
        ":root { --page: #fafafa; --text: #333333; --muted: #666666; --size: 16px; \
         --space: 16px; --gap: 12px; --rule: 1px; --accent: #3a7a4a;",
    );
    for i in 8..n {
        sheet.push_str(&format!(" --theme-{i}: {}px;", i % 40));
    }
    sheet.push_str(" }");
    sheet
}

const THEMED: &str = "body { margin: var(--space); background-color: var(--page); \
     color: var(--text); font-size: var(--size); } \
     p { margin: 0 0 var(--gap) 0; color: var(--text); \
     border-left: var(--rule) solid var(--accent); padding-left: calc(var(--gap) / 2); }";

const WRITTEN_OUT: &str = "body { margin: 16px; background-color: #fafafa; \
     color: #333333; font-size: 16px; } \
     p { margin: 0 0 12px 0; color: #333333; \
     border-left: 1px solid #3a7a4a; padding-left: 6px; }";

fn themes() {
    let body = paragraphs(PARAGRAPHS);
    let cascade = |sheet: &str| {
        let page = Page::parse(&page(sheet, &body));
        let mut times = Vec::new();
        let mut styles = None;
        for _ in 0..RUNS {
            let started = Instant::now();
            styles = Some(page.cascade());
            times.push(started.elapsed());
        }
        let styles = styles.expect("cascaded");
        let p = styles.get(page.first("p")).expect("styled");
        assert_eq!(
            (&p.margin[BOTTOM], &p.padding[LEFT]),
            (&LengthPercentageAuto::Px(12.0), &LengthPercentage::Px(6.0)),
            "the theme is read"
        );
        median(times)
    };
    println!("The cascade of {PARAGRAPHS} paragraphs, medians of {RUNS}:");
    println!("  the values written out: {}", ms(cascade(WRITTEN_OUT)));
    println!(
        "  a theme of 8 custom properties: {}",
        ms(cascade(&format!("{} {THEMED}", theme(8))))
    );
    println!(
        "  a theme of 200, the rules reading 8: {}",
        ms(cascade(&format!("{} {THEMED}", theme(200))))
    );
    println!(
        "  that, every paragraph declaring one of its own: {}",
        ms(cascade(&format!(
            "{} {THEMED} p {{ --muted: #777777; }}",
            theme(200)
        )))
    );
}

/// `n` custom properties, `--{name}0` a token and each after it the one
/// before twice.
fn doubling(name: &str, n: usize) -> String {
    let mut declarations = format!("--{name}0: x;");
    for i in 1..n {
        declarations.push_str(&format!(
            " --{name}{i}: var(--{name}{j}) var(--{name}{j});",
            j = i - 1
        ));
    }
    declarations
}

/// `n` custom properties each naming the next, the last naming the
/// first when `cycle`, else a length.
fn chain(n: usize, cycle: bool) -> String {
    let mut declarations = String::new();
    for i in 0..n - 1 {
        declarations.push_str(&format!("--c{i}: var(--c{});", i + 1));
    }
    if cycle {
        declarations.push_str(&format!("--c{}: var(--c0);", n - 1));
    } else {
        declarations.push_str(&format!("--c{}: 1px;", n - 1));
    }
    declarations
}

/// The median cascade of `source` over [`RUNS`] runs, and the custom
/// properties of its first `div` in the last.
fn custom_cascade(source: &str) -> (Duration, Styles, dom::NodeId) {
    let page = Page::parse(source);
    let mut times = Vec::new();
    let mut styles = None;
    for _ in 0..RUNS {
        let started = Instant::now();
        styles = Some(page.cascade());
        times.push(started.elapsed());
    }
    (median(times), styles.expect("cascaded"), page.first("div"))
}

fn chains() {
    println!("Custom properties at the bounds, medians of {RUNS}:");
    let (time, styles, div) = custom_cascade(&page(
        &format!("div {{ {} }}", doubling("v", 30)),
        "<div></div>",
    ));
    let style = styles.get(div).expect("styled");
    let longest = (0..30)
        .rev()
        .find_map(|i| Some((i, style.custom_property(&format!("--v{i}"))?.len())))
        .expect("the first ones have values");
    assert!(longest.0 < 29, "the chain ends at the bound");
    println!(
        "  thirty properties each doubling the last: {}, the last with a value \
         --v{} of {} tokens",
        ms(time),
        longest.0,
        longest.1
    );
    let (time, styles, div) = custom_cascade(&page(
        &format!("div {{ {} }}", chain(10_000, false)),
        "<div></div>",
    ));
    let style = styles.get(div).expect("styled");
    assert_eq!(style.custom_property("--c0").map(<[_]>::len), Some(1));
    println!("  a chain of 10,000 each naming the next: {}", ms(time));
    let (time, styles, div) = custom_cascade(&page(
        &format!("div {{ {} }}", chain(10_000, true)),
        "<div></div>",
    ));
    let style = styles.get(div).expect("styled");
    assert!(style.custom_property("--c0").is_none(), "a cycle");
    println!("  the same closed into a cycle: {}", ms(time));

    // The page built to exhaust the cascade's bound: every element computes
    // the chain of its own, a megabyte of tokens and more, until the
    // cascade's substitutions together weigh MAX_CASCADE_SUBSTITUTION.
    let source = page(
        &format!("div {{ {} }}", doubling("v", 16)),
        &"<div></div>".repeat(1000),
    );
    let page = Page::parse(&source);
    let resident = memory("VmRSS:");
    let reset = reset_peak_memory();
    let started = Instant::now();
    let styles = page.cascade();
    let time = started.elapsed();
    let divs: Vec<dom::NodeId> = page
        .document
        .subtree(page.document.root())
        .filter(|&n| page.document.tag(n) == Some("div"))
        .collect();
    let full = divs
        .iter()
        .filter(|&&div| {
            let style = styles.get(div).expect("styled");
            style
                .custom_property(&format!("--v{}", longest.0))
                .is_some()
        })
        .count();
    assert!(full < divs.len(), "the cascade's bound is met");
    println!(
        "  1,000 elements each declaring the doubling chain: {} once, the \
         chain whole in {full} of them",
        ms(time)
    );
    match (reset, resident, memory("VmHWM:")) {
        (true, Some(resident), Some(peak)) => println!(
            "  the memory its cascade takes at its peak: {:.0} MiB, over the {:.0} MiB \
             resident before",
            peak.saturating_sub(resident) as f64 / 1024.0,
            resident as f64 / 1024.0
        ),
        _ => println!("  the memory its cascade takes at its peak: unknown"),
    }
    drop(styles);
}

const CALC: &str = "body { margin: 16px; } \
     .b { width: calc(50% - 2 * 8px); height: calc(1em + 4px); \
     margin: calc(2px + 0.25em) 0; padding: max(4px, 1%) min(16px, 2%); \
     border-left: calc(1px + 1px) solid #888888; max-width: clamp(200px, 60% + 10px, 900px); }";

/// What [`CALC`]'s values come to in a body 1,248 pixels wide.
const PLAIN: &str = "body { margin: 16px; } \
     .b { width: 608px; height: 20px; margin: 6px 0; padding: 12.48px 16px; \
     border-left: 2px solid #888888; max-width: 758.8px; }";

fn math(fonts: &text::FontCollection) {
    let body = "<div class=b></div>".repeat(1000);
    println!("1,000 boxes, medians of {RUNS}:");
    let plain = Stages::of(&page(PLAIN, &body), fonts);
    plain.line("sized by lengths");
    let calc = Stages::of(&page(CALC, &body), fonts);
    calc.line("sized by math functions");
    assert!(
        (plain.laid_out.height - calc.laid_out.height).abs() < 1.0,
        "the math functions come to the lengths: {} and {}",
        plain.laid_out.height,
        calc.laid_out.height
    );
}

fn counters(fonts: &text::FontCollection) {
    let body = format!(
        "<ol>{}</ol>",
        (0..1000)
            .map(|i| format!("<li>Item {i}<ol><li>A point</li><li>Another</li></ol></li>"))
            .collect::<String>()
    );
    let numbered = "ol ol li::marker { content: counters(list-item, \".\") \" \"; }";
    println!("An ordered list of 1,000 items of two each, medians of {RUNS}:");
    Stages::of(&page("ol, li { display: block; }", &body), fonts).line("blocks");
    Stages::of(&page(numbered, &body), fonts).line("numbered by counters");
}

/// The pseudo-element benchmark's paragraphs, some six lines each at
/// the width given them.
fn long_paragraphs(n: usize) -> String {
    (0..n)
        .map(|i| {
            format!(
                "<p>Paragraph {i}. The quick brown fox jumps over the lazy dog; pack my \
                 box with five dozen liquor jugs. How vexingly quick daft zebras jump, \
                 and sphinx of black quartz, judge my vow. The five boxing wizards jump \
                 quickly; jackdaws love my big sphinx of quartz, while a wizard's job is \
                 to vex chumps quickly in fog.</p>"
            )
        })
        .collect()
}

fn justification(fonts: &text::FontCollection) {
    let body = long_paragraphs(PARAGRAPHS);
    let sheet = |align: &str| {
        format!("body {{ margin: 16px; }} p {{ width: 480px; margin: 0 0 12px 0; {align} }}")
    };
    println!("{PARAGRAPHS} paragraphs of six lines or so, medians of {RUNS}:");
    let left = Stages::of(&page(&sheet("text-align: left;"), &body), fonts);
    left.line("left-aligned");
    let justified = Stages::of(&page(&sheet("text-align: justify;"), &body), fonts);
    justified.line("justified");
    Stages::of(
        &page(
            &sheet("text-align: justify; text-justify: inter-character;"),
            &body,
        ),
        fonts,
    )
    .line("justified by inter-character");
    assert_eq!(
        left.laid_out.height, justified.laid_out.height,
        "justification moves no line"
    );
    black_box(justified.laid_out);
}

fn main() {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    // First, so that the memory the page built to exhaust the cascade's
    // bound takes is not memory the others freed.
    chains();
    themes();
    math(&fonts);
    counters(&fonts);
    justification(&fonts);
}
