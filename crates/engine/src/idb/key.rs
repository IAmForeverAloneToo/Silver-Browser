//! IndexedDB's keys (Indexed Database API 3.0, section 2.7), and their
//! encoding as bytes that sort as the keys do (section 2.7's "compare two
//! keys"): numbers, then dates, then strings, then binary, then arrays,
//! each array element by element and a shorter one first where one is a
//! prefix of the other. A range of keys is then a range of a table's
//! keys, and an index's order its encoded keys' order.

use std::cmp::Ordering;

/// The longest encoded key the store keeps (M36.4): a write of a longer
/// one fails with a `DataError`.
pub const MAX_IDB_KEY_BYTES: usize = 64 << 10;

/// How deep an array key may nest: a deeper one is no key the store
/// takes.
pub const MAX_KEY_DEPTH: usize = 64;

/// A key.
#[derive(Debug, Clone)]
pub enum Key {
    /// Never NaN; -0 is 0.
    Number(f64),
    /// A date's time value: never NaN.
    Date(f64),
    /// A string, as the UTF-16 code units it is, which are what keys
    /// are compared by.
    String(Vec<u16>),
    Binary(Vec<u8>),
    Array(Vec<Key>),
}

impl PartialEq for Key {
    fn eq(&self, other: &Key) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Key {}

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Key) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Key {
    /// IndexedDB's "compare two keys".
    fn cmp(&self, other: &Key) -> Ordering {
        let rank = |key: &Key| match key {
            Key::Number(_) => 0,
            Key::Date(_) => 1,
            Key::String(_) => 2,
            Key::Binary(_) => 3,
            Key::Array(_) => 4,
        };
        match (self, other) {
            (Key::Number(a), Key::Number(b)) | (Key::Date(a), Key::Date(b)) => {
                a.partial_cmp(b).unwrap_or(Ordering::Equal)
            }
            (Key::String(a), Key::String(b)) => a.cmp(b),
            (Key::Binary(a), Key::Binary(b)) => a.cmp(b),
            (Key::Array(a), Key::Array(b)) => a.cmp(b),
            (a, b) => rank(a).cmp(&rank(b)),
        }
    }
}

impl Key {
    /// A string key of `text`.
    pub fn string(text: &str) -> Key {
        Key::String(text.encode_utf16().collect())
    }

    /// A number key, or none for NaN.
    pub fn number(n: f64) -> Option<Key> {
        (!n.is_nan()).then_some(Key::Number(if n == 0.0 { 0.0 } else { n }))
    }

    /// The key's bytes, which sort as keys do.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Key::Number(n) => {
                out.push(TAG_NUMBER);
                out.extend_from_slice(&ordered(*n));
            }
            Key::Date(n) => {
                out.push(TAG_DATE);
                out.extend_from_slice(&ordered(*n));
            }
            Key::String(units) => {
                out.push(TAG_STRING);
                let mut bytes = Vec::with_capacity(units.len() * 2);
                for unit in units {
                    bytes.extend_from_slice(&unit.to_be_bytes());
                }
                escaped(&bytes, out);
            }
            Key::Binary(bytes) => {
                out.push(TAG_BINARY);
                escaped(bytes, out);
            }
            Key::Array(items) => {
                out.push(TAG_ARRAY);
                for item in items {
                    item.encode_into(out);
                }
                out.push(END);
            }
        }
    }

    /// The key `bytes` encode, all of them: `None` for bytes no key
    /// encodes to.
    pub fn decode(bytes: &[u8]) -> Option<Key> {
        let mut at = 0;
        let key = decode_at(bytes, &mut at, 0)?;
        (at == bytes.len()).then_some(key)
    }
}

const END: u8 = 0x00;
const TAG_NUMBER: u8 = 0x10;
const TAG_DATE: u8 = 0x20;
const TAG_STRING: u8 = 0x30;
const TAG_BINARY: u8 = 0x40;
const TAG_ARRAY: u8 = 0x50;

/// A double's bits, made to sort as the doubles do: a positive one's
/// with its sign bit set, a negative one's all flipped.
fn ordered(n: f64) -> [u8; 8] {
    let n = if n == 0.0 { 0.0 } else { n };
    let bits = n.to_bits();
    let bits = if bits >> 63 == 1 {
        !bits
    } else {
        bits | 1 << 63
    };
    bits.to_be_bytes()
}

fn unordered(bytes: [u8; 8]) -> f64 {
    let bits = u64::from_be_bytes(bytes);
    let bits = if bits >> 63 == 1 {
        bits & !(1 << 63)
    } else {
        !bits
    };
    f64::from_bits(bits)
}

/// `bytes` with each 0x00 written 0x00 0xFF, then 0x00 0x00: a prefix
/// sorts before what goes on from it.
fn escaped(bytes: &[u8], out: &mut Vec<u8>) {
    for &b in bytes {
        out.push(b);
        if b == 0 {
            out.push(0xFF);
        }
    }
    out.extend_from_slice(&[0, 0]);
}

fn unescaped(bytes: &[u8], at: &mut usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let b = *bytes.get(*at)?;
        *at += 1;
        if b != 0 {
            out.push(b);
            continue;
        }
        match bytes.get(*at)? {
            0xFF => out.push(0),
            0 => {
                *at += 1;
                return Some(out);
            }
            _ => return None,
        }
        *at += 1;
    }
}

fn decode_at(bytes: &[u8], at: &mut usize, depth: usize) -> Option<Key> {
    if depth > MAX_KEY_DEPTH {
        return None;
    }
    let tag = *bytes.get(*at)?;
    *at += 1;
    let double = |at: &mut usize| -> Option<f64> {
        let raw: [u8; 8] = bytes.get(*at..*at + 8)?.try_into().ok()?;
        *at += 8;
        let n = unordered(raw);
        // What `ordered` writes: no NaN, no -0.
        (!n.is_nan() && !(n == 0.0 && n.is_sign_negative())).then_some(n)
    };
    match tag {
        TAG_NUMBER => double(at).map(Key::Number),
        TAG_DATE => double(at).map(Key::Date),
        TAG_STRING => {
            let raw = unescaped(bytes, at)?;
            if raw.len() % 2 != 0 {
                return None;
            }
            Some(Key::String(
                raw.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_be_bytes(*pair))
                    .collect(),
            ))
        }
        TAG_BINARY => unescaped(bytes, at).map(Key::Binary),
        TAG_ARRAY => {
            let mut items = Vec::new();
            loop {
                if *bytes.get(*at)? == END {
                    *at += 1;
                    return Some(Key::Array(items));
                }
                items.push(decode_at(bytes, at, depth + 1)?);
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Vec<Key> {
        vec![
            Key::Number(f64::NEG_INFINITY),
            Key::Number(-1.5),
            Key::Number(0.0),
            Key::Number(1e-300),
            Key::Number(2.0),
            Key::Number(f64::INFINITY),
            Key::Date(-1.0),
            Key::Date(0.0),
            Key::string(""),
            Key::String(vec![0]),
            Key::String(vec![0, 0]),
            Key::String(vec![1]),
            Key::string("a"),
            Key::string("ab"),
            Key::String(vec![0xD800]),
            Key::string("\u{FFFF}"),
            Key::Binary(vec![]),
            Key::Binary(vec![0]),
            Key::Binary(vec![0, 255]),
            Key::Binary(vec![1]),
            Key::Array(vec![]),
            Key::Array(vec![Key::Number(1.0)]),
            Key::Array(vec![Key::Number(1.0), Key::Number(0.0)]),
            Key::Array(vec![Key::string("a")]),
            Key::Array(vec![Key::Array(vec![])]),
        ]
    }

    #[test]
    fn keys_sort_as_indexed_db_compares_them_and_their_bytes_do_too() {
        let keys = keys();
        for (i, a) in keys.iter().enumerate() {
            for (j, b) in keys.iter().enumerate() {
                assert_eq!(a.cmp(b), i.cmp(&j), "{a:?} {b:?}");
                assert_eq!(a.encode().cmp(&b.encode()), i.cmp(&j), "{a:?} {b:?}");
            }
            assert_eq!(Key::decode(&a.encode()).as_ref(), Some(a));
        }
    }

    #[test]
    fn minus_zero_is_zero_and_nan_is_no_key() {
        assert_eq!(Key::number(-0.0), Some(Key::Number(0.0)));
        assert_eq!(
            Key::number(-0.0).unwrap().encode(),
            Key::Number(0.0).encode()
        );
        assert_eq!(Key::number(f64::NAN), None);
    }

    #[test]
    fn what_no_key_encodes_to_is_refused() {
        for bytes in [
            &[][..],
            &[0x99],
            &[TAG_NUMBER, 1, 2],
            &[TAG_STRING, 0x41, 0, 0],
            &[TAG_STRING, 0, 7],
            &[TAG_ARRAY],
            // NaN, and -0, which no key is written as.
            &[TAG_NUMBER, 0xff, 0xf8, 0, 0, 0, 0, 0, 0],
            &[TAG_NUMBER, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        ] {
            assert_eq!(Key::decode(bytes), None, "{bytes:?}");
        }
        // Trailing bytes are no key's.
        let mut bytes = Key::Number(1.0).encode();
        bytes.push(0);
        assert_eq!(Key::decode(&bytes), None);
        // Nor is an array nested past the bound.
        let mut deep = Key::Array(vec![]);
        for _ in 0..=MAX_KEY_DEPTH {
            deep = Key::Array(vec![deep]);
        }
        assert_eq!(Key::decode(&deep.encode()), None);
    }
}
