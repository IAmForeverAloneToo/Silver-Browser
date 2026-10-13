//! The selector parser on arbitrary text, and every selector it accepts
//! matched against every node of a small fixed document — its text
//! nodes and the document node included, which match nothing — as the
//! element, and as the originating element of the pseudo-element the
//! selector ends in, if it ends in one.

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;

const PAGE: &str = "<div id=a class=b><p class=c><span>x</span><em>y</em></p>\
                    <ul><li>1<li>2</ul></div>";

static DOC: OnceLock<dom::Document> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    let tokens = css::tokenize(s);
    let Ok(list) = style::parse_selector_list(&tokens) else {
        return;
    };
    let doc = DOC.get_or_init(|| {
        let page = html::parse_document(PAGE).expect("the fixed page parses");
        dom::Document::from_node(&page)
    });
    let mut matcher = style::selector_matching::Matcher::new(doc);
    for id in doc.subtree(doc.root()) {
        let _ = style::matches_selector_list(doc, id, &list);
        for selector in &list.0 {
            if let Some(ending) = &selector.pseudo_element {
                let _ = matcher.matches_pseudo_element(id, selector, ending.element);
            }
        }
    }
});
