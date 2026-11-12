//! G247: what gap batch H's features cost a page that uses them. A page
//! of 1,000 cards, each a heading and a paragraph with an emphasis, is
//! drawn at 1280 x 800 as it is, then with one feature at a time: each
//! card a declarative shadow tree slotting its content; each card a
//! size container its paragraph queries; the cards' rules in an
//! `@scope`; the same rules nested; a `:has()` rule; a modal dialog in
//! the top layer over the cards; a CSS animation on every card; and a
//! `:has()` rule whose subject holds all 1,000 cards, a card's class
//! toggled each frame. For each, the first frame, its style, layout and
//! paint, and the frames that each make its change: a card appended,
//! or, for the last, a card's class toggled. The plain page is the baseline: the features'
//! costs are their rows against it (`docs/perf-budgets.md`).
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench features
//! ```

#![forbid(unsafe_code)]

mod support;

use std::hint::black_box;
use std::time::Duration;

use dom::Document;
use engine::{Engine, FRAME_BUDGET, FrameStats, FrameTime};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const CARDS: usize = 1000;
const APPENDED: usize = 100;

/// The cards' own sheet, which every page has.
const BASE: &str = "body { margin: 8px; } \
    .cards { display: flex; flex-wrap: wrap; gap: 8px; } \
    .card { width: 180px; padding: 8px; border: 1px solid #ccc; } \
    .card h2 { font-size: 15px; margin: 0 0 4px; } \
    .card p { font-size: 13px; margin: 0; }";

/// A feature: its name, the sheet it adds, how each card is written,
/// what the page adds after the cards, and the change each frame after
/// the first makes.
struct Feature {
    name: &'static str,
    sheet: &'static str,
    card: fn(usize) -> String,
    tail: &'static str,
    change: fn(&mut Document, dom::NodeId, usize),
}

fn plain_card(i: usize) -> String {
    format!(
        "<div class=card><h2>Card {i}</h2><p>The quick brown fox <em>jumps</em> over \
         the lazy dog; pack my box with five dozen liquor jugs.</p></div>"
    )
}

fn shadow_card(i: usize) -> String {
    format!(
        "<div class=card><template shadowrootmode=open><style>:host {{ display: block }} \
         ::slotted(h2) {{ color: #234 }}</style><slot></slot></template><h2>Card {i}</h2>\
         <p>The quick brown fox <em>jumps</em> over the lazy dog; pack my box with five \
         dozen liquor jugs.</p></div>"
    )
}

const FEATURES: [Feature; 9] = [
    Feature {
        name: "no feature",
        sheet: "",
        card: plain_card,
        tail: "",
        change: append,
    },
    Feature {
        name: "a declarative shadow tree a card",
        sheet: "",
        card: shadow_card,
        tail: "",
        change: append,
    },
    Feature {
        name: "size containers",
        sheet: ".card { container-type: inline-size } \
                @container (min-width: 150px) { .card p { font-size: 14px } }",
        card: plain_card,
        tail: "",
        change: append,
    },
    Feature {
        name: "@scope",
        sheet: "@scope (.card) to (em) { h2 { color: #234 } p { line-height: 1.4 } }",
        card: plain_card,
        tail: "",
        change: append,
    },
    Feature {
        name: "nesting",
        sheet: ".card { & h2 { color: #234 } & p { line-height: 1.4; & em { color: #a00 } } }",
        card: plain_card,
        tail: "",
        change: append,
    },
    Feature {
        name: ":has()",
        sheet: ".card:has(em) { border-color: #88a } .card:has(> h2 + p) { background: #fafaff }",
        card: plain_card,
        tail: "",
        change: append,
    },
    Feature {
        name: "a modal dialog in the top layer",
        sheet: "dialog::backdrop { background: rgb(0 0 0 / 0.3) }",
        card: plain_card,
        tail: "<dialog id=d><p>A dialog over the cards.</p></dialog>\
               <script>document.getElementById('d').showModal()</script>",
        change: append,
    },
    Feature {
        name: "a CSS animation a card",
        sheet: "@keyframes glow { from { border-color: #ccc } to { border-color: #88a } } \
                .card { animation: glow 2s infinite alternate }",
        card: plain_card,
        tail: "",
        change: append,
    },
    Feature {
        name: ":has() over the 1,000 cards, a card's class toggled",
        sheet: ".cards:has(.card.hot) { background: #ffd }",
        card: plain_card,
        tail: "",
        change: toggle,
    },
];

/// The page of `n` cards with `feature`.
fn page(n: usize, feature: &Feature) -> String {
    let mut source = format!(
        "<style>{BASE} {}</style><body><div class=cards>",
        feature.sheet
    );
    for i in 0..n {
        source.push_str(&(feature.card)(i));
    }
    source.push_str("</div>");
    source.push_str(feature.tail);
    source.push_str("</body>");
    source
}

/// Toggles the class `hot` of a card of `cards`, the `i`th's.
fn toggle(document: &mut Document, cards: dom::NodeId, i: usize) {
    let card = document.children(cards).nth(i % CARDS).expect("the card");
    let class = if i.is_multiple_of(2) {
        "card hot"
    } else {
        "card"
    };
    document.set_attribute(card, "class", class).expect("set");
}

/// Appends a card to `cards`.
fn append(document: &mut Document, cards: dom::NodeId, i: usize) {
    let card = document.create_element("div").expect("an element");
    document.set_attribute(card, "class", "card").expect("set");
    let p = document.create_element("p").expect("an element");
    let text = document
        .create_text(&format!("Card {i}, appended."))
        .expect("a text node");
    document.append_child(p, text).expect("appended");
    document.append_child(card, p).expect("appended");
    document.append_child(cards, card).expect("appended");
}

/// The median of `of` over `stats`, in milliseconds.
fn median(stats: &[FrameStats], of: fn(&FrameStats) -> Duration) -> f64 {
    let mut times: Vec<f64> = stats.iter().map(|s| of(s).as_secs_f64() * 1000.0).collect();
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

fn main() {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    for feature in &FEATURES {
        let mut engine = Engine::with_fonts(fonts.clone());
        // The dialog's `showModal()` is a script's.
        engine.set_scripting(true);
        engine.load_document(&page(CARDS, feature));
        let mut frame = 0u32;
        let mut draw = |engine: &mut Engine| {
            frame += 1;
            black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(FRAME_BUDGET * frame)));
            engine.frame_stats()
        };
        let first = draw(&mut engine);
        let document = engine.document();
        let cards = document
            .subtree(document.root())
            .find(|&id| document.attr(id, "class") == Some("cards"))
            .expect("the page has its cards");
        let mut stats = Vec::with_capacity(support::rounds(APPENDED));
        for i in 0..support::rounds(APPENDED) {
            engine.mutate(|document| (feature.change)(document, cards, CARDS + i));
            stats.push(draw(&mut engine));
        }
        println!("{}:", feature.name);
        println!(
            "  the first frame: / style:  {:.3} ms",
            first.style.as_secs_f64() * 1000.0
        );
        println!(
            "  the first frame: / layout: {:.3} ms",
            first.layout.as_secs_f64() * 1000.0
        );
        println!(
            "  the first frame: / paint:  {:.3} ms",
            first.paint.as_secs_f64() * 1000.0
        );
        println!(
            "  the first frame: / frame:  {:.3} ms",
            first.total.as_secs_f64() * 1000.0
        );
        println!(
            "  its change, median of {}: / frame: {:.3} ms",
            stats.len(),
            median(&stats, |s| s.total)
        );
        support::hooks(feature.name);
    }
}
