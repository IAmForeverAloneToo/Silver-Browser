//! Table-driven selector-matching tests (roadmap M4.11): each row is a
//! `(selector, HTML, expected matched tags)` triple, exercised end to end
//! through real markup (`html::parse_document`) rather than hand-built
//! `dom::Node` trees. This is a black-box integration test against the
//! crate's public API only — it complements, rather than replaces, the
//! inline unit tests in `selector_matching::{parse, matcher}`.

struct Case {
    name: &'static str,
    html: &'static str,
    selector: &'static str,
    expected: &'static [&'static str],
}

/// Runs every case in `cases`, reporting which one failed on mismatch.
fn run(cases: &[Case]) {
    for case in cases {
        let doc = html::parse_document(case.html).expect("html parsing never fails");
        let document = dom::Document::from_node(&doc);
        let list = style::selector_matching::parse_selector_list(&css::tokenize(case.selector))
            .unwrap_or_else(|e| {
                panic!(
                    "case {:?}: selector {:?} must parse: {e}",
                    case.name, case.selector
                )
            });
        // The parser implies `html`, `head` and `body` around every
        // case's markup; they are reported only where the case wrote
        // them, so each row reads as the markup it shows.
        let wrote = |tag: &str| case.html.contains(&format!("<{tag}"));
        let matched: Vec<&str> = document
            .subtree(document.root())
            .filter(|&i| style::selector_matching::matches_selector_list(&document, i, &list))
            .filter_map(|i| document.tag(i))
            .filter(|tag| !matches!(*tag, "html" | "head" | "body") || wrote(tag))
            .collect();
        assert_eq!(
            matched, case.expected,
            "case {:?}: selector {:?} against {:?}",
            case.name, case.selector, case.html
        );
    }
}

#[test]
fn simple_selectors_table() {
    run(&[
        Case {
            name: "type",
            html: "<div><p>x</p><span>y</span></div>",
            selector: "p",
            expected: &["p"],
        },
        Case {
            name: "universal",
            html: "<div><p>x</p></div>",
            selector: "*",
            expected: &["div", "p"],
        },
        Case {
            name: "class",
            html: "<p class='a'>x</p><p class='b'>y</p>",
            selector: ".a",
            expected: &["p"],
        },
        Case {
            name: "id",
            html: "<p id='x'>a</p><p id='y'>b</p>",
            selector: "#y",
            expected: &["p"],
        },
        Case {
            name: "compound",
            html: "<p class='a'>x</p><div class='a'>y</div>",
            selector: "div.a",
            expected: &["div"],
        },
        Case {
            name: "type_case_insensitive",
            html: "<DIV></DIV>",
            selector: "div",
            expected: &["div"],
        },
        Case {
            name: "no_match",
            html: "<p>x</p>",
            selector: "span",
            expected: &[],
        },
    ]);
}

#[test]
fn combinator_table() {
    run(&[
        Case {
            name: "descendant",
            html: "<div><section><p>x</p></section></div>",
            selector: "div p",
            expected: &["p"],
        },
        Case {
            name: "child",
            html: "<div><p>x</p></div>",
            selector: "div > p",
            expected: &["p"],
        },
        Case {
            name: "child_excludes_grandchild",
            html: "<div><section><p>x</p></section></div>",
            selector: "div > p",
            expected: &[],
        },
        Case {
            name: "next_sibling",
            html: "<ul><li>a</li><li>b</li><li>c</li></ul>",
            selector: "li + li",
            expected: &["li", "li"],
        },
        Case {
            name: "later_sibling",
            html: "<ul><li class='m'>a</li><li>b</li><li>c</li></ul>",
            selector: ".m ~ li",
            expected: &["li", "li"],
        },
        Case {
            name: "chained",
            html: "<div><section><article><p>x</p></article></section></div>",
            selector: "div section > article p",
            expected: &["p"],
        },
    ]);
}

#[test]
fn attribute_selector_table() {
    run(&[
        Case {
            name: "exists",
            html: "<input disabled><input>",
            selector: "[disabled]",
            expected: &["input"],
        },
        Case {
            name: "equals",
            html: "<a href='x'>a</a><a href='y'>b</a>",
            selector: "[href=x]",
            expected: &["a"],
        },
        Case {
            name: "prefix",
            html: "<a href='https://a'>a</a><a href='mailto:b'>b</a>",
            selector: "[href^=https]",
            expected: &["a"],
        },
        Case {
            name: "suffix",
            html: "<img src='a.png'><img src='b.gif'>",
            selector: "[src$='.png']",
            expected: &["img"],
        },
        Case {
            name: "substring",
            html: "<a href='x-mid-y'>a</a><a href='z'>b</a>",
            selector: "[href*=mid]",
            expected: &["a"],
        },
        Case {
            name: "case_insensitive_flag",
            html: "<p data-x='ABC'>a</p>",
            selector: "[data-x=abc i]",
            expected: &["p"],
        },
    ]);
}

#[test]
fn structural_pseudo_class_table() {
    run(&[
        Case {
            name: "first_child",
            html: "<ul><li>a</li><li>b</li></ul>",
            selector: "li:first-child",
            expected: &["li"],
        },
        Case {
            name: "last_child",
            html: "<ul><li>a</li><li>b</li></ul>",
            selector: "li:last-child",
            expected: &["li"],
        },
        Case {
            name: "nth_child_odd",
            html: "<ul><li>1</li><li>2</li><li>3</li><li>4</li></ul>",
            selector: "li:nth-child(odd)",
            expected: &["li", "li"],
        },
        Case {
            name: "only_child",
            html: "<div><p>solo</p></div><div><p>a</p><p>b</p></div>",
            selector: "p:only-child",
            expected: &["p"],
        },
        Case {
            name: "empty",
            html: "<div></div><div>text</div>",
            selector: "div:empty",
            expected: &["div"],
        },
        Case {
            name: "root",
            html: "<html><body><p>x</p></body></html>",
            selector: ":root",
            expected: &["html"],
        },
    ]);
}

#[test]
fn logical_pseudo_class_table() {
    run(&[
        Case {
            name: "not",
            html: "<p class='a'>x</p><p>y</p>",
            selector: "p:not(.a)",
            expected: &["p"],
        },
        Case {
            name: "is",
            html: "<div>x</div><span>y</span><p>z</p>",
            selector: ":is(div, span)",
            expected: &["div", "span"],
        },
        Case {
            name: "where_zero_specificity_still_matches",
            html: "<p id='x'>a</p>",
            selector: ":where(#x)",
            expected: &["p"],
        },
        Case {
            name: "not_with_descendant",
            html: "<div><p>a</p></div><p>b</p>",
            selector: "p:not(div p)",
            expected: &["p"],
        },
    ]);
}

#[test]
fn realistic_markup_table() {
    run(&[
        Case {
            name: "nav_active_link",
            html: "<nav><ul><li><a href='/'>Home</a></li><li><a href='/about' class='active'>About</a></li></ul></nav>",
            selector: "nav a.active",
            expected: &["a"],
        },
        Case {
            name: "form_required_inputs",
            html: "<form><input required><input><input required></form>",
            selector: "input[required]",
            expected: &["input", "input"],
        },
        Case {
            name: "card_grid_odd_cards",
            html: "<div class='grid'><div class='card'>1</div><div class='card'>2</div><div class='card'>3</div></div>",
            selector: ".card:nth-child(odd)",
            expected: &["div", "div"],
        },
        Case {
            name: "table_row_striping",
            html: "<table><tr><td>1</td></tr><tr><td>2</td></tr><tr><td>3</td></tr></table>",
            selector: "tr:nth-child(2n-1)",
            expected: &["tr", "tr"],
        },
    ]);
}
