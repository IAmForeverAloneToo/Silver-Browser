//! The selector parser on arbitrary text, and every selector it accepts
//! matched against every element of a small fixed document.

#![no_main]

use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;

const PAGE: &str = "<div id=a class=b><p class=c><span>x</span><em>y</em></p>\
                    <ul><li>1<li>2</ul></div>";

static FLAT: OnceLock<style::FlatDom> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    let tokens = css::tokenize(s);
    let Ok(list) = style::parse_selector_list(&tokens) else {
        return;
    };
    let flat = FLAT.get_or_init(|| {
        let doc = html::parse_document(PAGE).expect("the fixed page parses");
        style::FlatDom::build(&doc)
    });
    for i in 0..flat.nodes.len() {
        let _ = style::matches_selector_list(flat, i, &list);
    }
});
