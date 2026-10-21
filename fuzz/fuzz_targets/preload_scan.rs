//! The preload scanner (M38): any text, given whole and given in pieces
//! of a size the input's first byte chooses, with scripting on or off.
//! Nothing panics; both give the same hints; and every hint's URL is,
//! trimmed, the value of the attribute its element names it by (`href`
//! or `src`), which the element has.

#![no_main]
#![forbid(unsafe_code)]

use html::preload::{Hint, HintKind, Scanner};
use libfuzzer_sys::fuzz_target;

fn scan(text: &str, scripting: bool, piece: usize) -> Vec<Hint> {
    let mut scanner = Scanner::new(scripting);
    let chars: Vec<char> = text.chars().collect();
    let mut hints = Vec::new();
    for chunk in chars.chunks(piece) {
        scanner.feed(&chunk.iter().collect::<String>());
        hints.extend(scanner.take_hints());
    }
    scanner.end();
    hints.extend(scanner.take_hints());
    hints
}

fuzz_target!(|data: &[u8]| {
    let Some((&first, rest)) = data.split_first() else {
        return;
    };
    let text = String::from_utf8_lossy(rest);
    let scripting = first & 0x80 != 0;
    let piece = usize::from(first & 0x7f) + 1;
    let whole = scan(&text, scripting, text.chars().count().max(1));
    assert_eq!(whole, scan(&text, scripting, piece), "pieces of {piece}");
    for hint in &whole {
        let named = match hint.kind {
            HintKind::Base | HintKind::Stylesheet | HintKind::Preload => "href",
            HintKind::Script | HintKind::Image => "src",
            HintKind::Policy => {
                assert!(hint.url.is_empty());
                continue;
            }
        };
        let value = hint.attr(named).expect("the attribute it names");
        assert_eq!(
            value.trim_matches(|c: char| c.is_ascii_whitespace()),
            hint.url
        );
        assert!(!hint.url.is_empty());
    }
});
