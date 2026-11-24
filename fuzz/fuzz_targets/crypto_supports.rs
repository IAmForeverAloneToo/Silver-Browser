//! `SubtleCrypto.supports`'s contract (G255, the lead's ruling) on any
//! bytes: each input an operation, an algorithm whose members are cut
//! from the bytes (any name, any member, any kind of value, a key from
//! the oracle's among them), and a length or another algorithm.
//! `supports` and the operation itself, run on the oracle's throwaway
//! keys (`crypto::oracle`), must agree: never a yes the operation
//! refuses, never a no it takes.

#![no_main]
#![forbid(unsafe_code)]

use std::cell::RefCell;

use crypto::Value;
use crypto::oracle::{NAMES, OPERATIONS, Oracle};
use libfuzzer_sys::fuzz_target;

thread_local! {
    /// The throwaway keys, made once: RSA's take a while.
    static ORACLE: RefCell<Option<Oracle>> = const { RefCell::new(None) };
}

/// The members an algorithm dictionary has, of every algorithm.
const MEMBERS: &[&str] = &[
    "length",
    "hash",
    "namedCurve",
    "iv",
    "counter",
    "tagLength",
    "additionalData",
    "label",
    "saltLength",
    "salt",
    "info",
    "iterations",
    "public",
    "modulusLength",
    "publicExponent",
    "context",
];

/// Numbers around every bound the operations check.
const NUMBERS: &[f64] = &[
    0.0,
    1.0,
    3.0,
    7.0,
    8.0,
    12.0,
    16.0,
    32.0,
    64.0,
    96.0,
    104.0,
    128.0,
    129.0,
    192.0,
    222.0,
    255.0,
    256.0,
    257.0,
    384.0,
    512.0,
    528.0,
    529.0,
    1024.0,
    2048.0,
    40800.0,
    65280.0,
    65288.0,
    4294967288.0,
    4294967295.0,
];

const WORDS: &[&str] = &[
    "SHA-1", "SHA-256", "SHA-384", "SHA-512", "P-256", "P-384", "P-521", "P-51", "MD5", "",
];

/// Reads the input a byte at a time.
struct Input<'a>(&'a [u8]);

impl Input<'_> {
    fn byte(&mut self) -> Option<u8> {
        let (&b, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(b)
    }

    fn pick<T: Copy>(&mut self, from: &[T]) -> Option<T> {
        Some(from[usize::from(self.byte()?) % from.len()])
    }
}

fn value(input: &mut Input, publics: &[u32]) -> Option<Value> {
    Some(match input.byte()? % 7 {
        0 | 1 => Value::Num(input.pick(NUMBERS)?),
        2 => Value::Str(input.pick(WORDS)?.to_owned()),
        3 => Value::Str(input.pick(NAMES)?.to_owned()),
        4 => Value::Bytes(match input.byte()? % 4 {
            0 => vec![3],
            1 => vec![1, 0, 1],
            _ => vec![0x5a; usize::from(input.byte()?) % 40],
        }),
        5 => Value::Key(
            publics
                .get(usize::from(input.byte()?))
                .copied()
                .unwrap_or(u32::MAX),
        ),
        _ => Value::Bool(true),
    })
}

fn algorithm(input: &mut Input, publics: &[u32]) -> Option<Value> {
    let first = input.byte()?;
    let name = NAMES[usize::from(first & 0x7f) % NAMES.len()];
    if first & 0x80 == 0 {
        return Some(Value::Str(name.to_owned()));
    }
    let mut members = vec![("name".to_owned(), Value::Str(name.to_owned()))];
    for _ in 0..input.byte()? % 6 {
        let member = input.pick(MEMBERS)?;
        members.push((member.to_owned(), value(input, publics)?));
    }
    Some(Value::Dict(members))
}

/// Whether running it would cost minutes: PBKDF2's work grows with its
/// iterations and its length, which may be any unsigned long.
fn too_costly(values: &[&Value]) -> bool {
    let pbkdf2 = values.iter().any(|v| match v {
        Value::Str(name) => name.eq_ignore_ascii_case("PBKDF2"),
        _ => v.get("name") == &Value::Str("PBKDF2".to_owned()),
    });
    let big = |v: &Value| matches!(v, Value::Num(n) if *n > 65536.0);
    pbkdf2
        && values.iter().any(|v| {
            big(v)
                || match v {
                    Value::Dict(members) => members.iter().any(|(_, m)| big(m)),
                    _ => false,
                }
        })
}

fuzz_target!(|data: &[u8]| {
    ORACLE.with(|cell| {
        let mut cell = cell.borrow_mut();
        let oracle = cell.get_or_insert_with(Oracle::new);
        let publics: Vec<u32> = oracle.public_keys().iter().map(|(_, k)| *k).collect();
        let mut input = Input(data);
        let Some(op) = input.pick(OPERATIONS) else {
            return;
        };
        let Some(alg) = algorithm(&mut input, &publics) else {
            return;
        };
        let extra = match input.byte().map(|b| b % 3) {
            Some(0) => input.pick(NUMBERS).map_or(Value::Absent, Value::Num),
            Some(1) => algorithm(&mut input, &publics).unwrap_or(Value::Absent),
            _ => Value::Absent,
        };
        if too_costly(&[&alg, &extra]) {
            return;
        }
        if let Err(why) = oracle.check(op, &alg, &extra) {
            panic!("{why}");
        }
    });
});
