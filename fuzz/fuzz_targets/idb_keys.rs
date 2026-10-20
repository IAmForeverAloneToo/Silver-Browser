//! IndexedDB's keys (M36.4) on arbitrary bytes: what decodes encodes
//! back to the same bytes; and keys made of the input encode to bytes
//! that sort as the keys compare, and decode to the same keys.

#![no_main]
#![forbid(unsafe_code)]

use arbitrary::{Arbitrary, Unstructured};
use engine::idb::{Key, MAX_KEY_DEPTH};
use libfuzzer_sys::fuzz_target;

/// A key of the fuzzer's making, `depth` arrays deep at most.
fn key(u: &mut Unstructured<'_>, depth: usize) -> arbitrary::Result<Key> {
    Ok(match u.int_in_range(0..=4)? {
        0 => {
            let n = f64::arbitrary(u)?;
            Key::number(n).unwrap_or(Key::Number(0.0))
        }
        1 => {
            let n = f64::arbitrary(u)?;
            Key::Date(if n.is_nan() {
                0.0
            } else if n == 0.0 {
                0.0
            } else {
                n
            })
        }
        2 => Key::String(Vec::<u16>::arbitrary(u)?),
        3 => Key::Binary(Vec::<u8>::arbitrary(u)?),
        _ if depth == 0 => Key::Binary(Vec::new()),
        _ => {
            let len = u.int_in_range(0..=4)?;
            let mut items = Vec::new();
            for _ in 0..len {
                items.push(key(u, depth - 1)?);
            }
            Key::Array(items)
        }
    })
}

fuzz_target!(|data: &[u8]| {
    if let Some(decoded) = Key::decode(data) {
        assert_eq!(decoded.encode(), data, "{decoded:?}");
    }
    let mut u = Unstructured::new(data);
    let (Ok(a), Ok(b)) = (key(&mut u, MAX_KEY_DEPTH.min(4)), key(&mut u, 4)) else {
        return;
    };
    assert_eq!(a.cmp(&b), a.encode().cmp(&b.encode()), "{a:?} {b:?}");
    assert_eq!(Key::decode(&a.encode()), Some(a.clone()));
});
