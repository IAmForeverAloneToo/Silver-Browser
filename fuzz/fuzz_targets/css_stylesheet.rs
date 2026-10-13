//! The CSS tokenizer, the stylesheet parser and the `style` attribute
//! parser on arbitrary text; and any bytes decoded as a stylesheet's
//! are, `@charset` and all, then parsed.

#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let (text, encoding) = css::decode_stylesheet(data, None, None);
    assert!(encoding.can_decode(), "{encoding:?}");
    let _ = css::parse_stylesheet(&text);
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    let _ = css::tokenize(s);
    let _ = css::parse_stylesheet(s);
    let _ = css::parse_style_attribute(s);
});
