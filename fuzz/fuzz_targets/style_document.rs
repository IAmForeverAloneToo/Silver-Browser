//! The cascade over a parsed page. The whole input is also tried as a
//! stylesheet, so author rules reach the cascade without the fuzzer
//! having to produce a well-formed `<style>` element.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(parsed) = html::parse_document(s) else {
        return;
    };
    let doc = dom::Document::from_node(&parsed);
    let mut sheets = Vec::new();
    if let Ok(sheet) = css::parse_stylesheet(s) {
        sheets.push(sheet);
    }
    let _ = style::cascade_document(&doc, &sheets, &style::CascadeContext::default());
});
