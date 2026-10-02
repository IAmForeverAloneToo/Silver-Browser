//! M4.12 baseline: match ~1,000 selectors against a ~10,000-element
//! synthetic DOM, through one `Matcher` for the whole pass as the
//! cascade matches (M13: a one-off match recounts the element's siblings
//! for the structural pseudo-classes, which a pass would repeat for
//! every element).
//!
//! `harness = false` (see `Cargo.toml`): this is a plain binary, not a
//! `#[bench]`-harness target, so it needs no bench-runner dependency.
//! Criterion-tracked, CI-enforced regression budgets are a T4 (performance
//! track) commitment for later (M35+); this is deliberately just a
//! baseline number, run by hand with `cargo bench -p style` and eyeballed
//! against the milestone's <100ms (debug) budget.

use std::hint::black_box;
use std::time::Instant;

use dom::{Element, Node};
use style::selector_matching::{Matcher, parse_selector_list};

const TARGET_ELEMENTS: usize = 10_000;
const TARGET_SELECTORS: usize = 1_000;

/// Builds a synthetic DOM with roughly `target_count` elements: a wide,
/// shallow forest of `<div class="row [featured]"><p id=item-N class=item>
/// <span class=tag-K></span></div>` groups. Wide-and-shallow mirrors real
/// page markup's branching factor better than a deep chain would, and
/// gives the descendant/child/sibling combinators below something
/// realistic to walk.
fn synthetic_dom(target_count: usize) -> Node {
    let groups = target_count / 3 + 1;
    let mut rows = Vec::with_capacity(groups);
    for i in 0..groups {
        let class = if i % 5 == 0 { "row featured" } else { "row" };
        let row = Element::new("div")
            .attr("class", class)
            .push(
                Element::new("p")
                    .attr("id", format!("item-{i}"))
                    .attr("class", "item")
                    .text("content"),
            )
            .push(Element::new("span").attr("class", format!("tag-{}", i % 20)));
        rows.push(row.into());
    }
    Node::document(vec![Element::new("body").push_children(rows).into()])
}

/// A representative mix of selector shapes (types, classes, ids,
/// attributes, all four combinators, structural and logical
/// pseudo-classes), cycled to reach `target_count` sources.
fn synthetic_selector_sources(target_count: usize) -> Vec<String> {
    let templates: [&str; 16] = [
        "div",
        ".row",
        ".featured",
        "#item-42",
        "div.row",
        "p.item",
        "div > p",
        "div p",
        "p ~ span",
        "div + div",
        "[class]",
        "[class~='row']",
        "p:first-child",
        "div:nth-child(2n+1)",
        "p:not(.item)",
        ":is(div, span, p)",
    ];
    (0..target_count)
        .map(|i| templates[i % templates.len()].to_string())
        .collect()
}

fn main() {
    let dom = synthetic_dom(TARGET_ELEMENTS);
    let document = dom::Document::from_node(&dom);
    let elements: Vec<dom::NodeId> = document
        .subtree(document.root())
        .filter(|&id| document.is_element(id))
        .collect();
    let element_count = elements.len();

    let selectors: Vec<_> = synthetic_selector_sources(TARGET_SELECTORS)
        .iter()
        .map(|src| {
            parse_selector_list(&css::tokenize(src)).expect("all templates are valid selectors")
        })
        .collect();

    let start = Instant::now();
    let mut match_count = 0usize;
    let mut matcher = Matcher::new(&document);
    for list in &selectors {
        for &id in &elements {
            if matcher.matches_list(id, list) {
                match_count += 1;
            }
        }
    }
    let elapsed = start.elapsed();
    black_box(match_count);

    println!(
        "match: {element_count} elements x {} selectors -> {elapsed:?} ({match_count} matches) [budget: <100ms debug, per M4.12]",
        selectors.len(),
    );
}
