//! Web Storage and the profile under it (M28): `setItem`, `removeItem`
//! and `clear()` of a few origins' `localStorage` and `sessionStorage`
//! areas, keys and values from the input, some large enough to reach
//! the quota, through the engine's storage (`engine::StorageDriver`),
//! with the profile written, opened again, or copied as a crash would
//! leave it, and damaged, between them. Nothing panics; each area is
//! exactly what a model written again here says, in the order its items
//! were first set, within its quota of UTF-16 code units; a profile
//! opened again has every `localStorage` area as it was when last
//! written and no `sessionStorage`; a copy taken between writes has
//! what the last write wrote; and a damaged profile is refused or read
//! without a panic.

#![no_main]
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use engine::{STORAGE_QUOTA, StorageDriver};
use libfuzzer_sys::fuzz_target;
use profile::{Memory, Profile};

/// The origins areas are kept for: one host by scheme and port, and
/// another.
const ORIGINS: [&str; 4] = [
    "http://a.test",
    "http://a.test:8080",
    "https://a.test",
    "http://b.test",
];

/// The keys most calls use, so that they meet.
const KEYS: [&str; 6] = ["a", "b", "", "length", "key", "\u{fffd}"];

/// The input, read a byte at a time; past its end, zeros.
struct Input<'a>(&'a [u8]);

impl Input<'_> {
    fn byte(&mut self) -> u8 {
        let (&first, rest) = self.0.split_first().unwrap_or((&0, &[]));
        self.0 = rest;
        first
    }

    /// A key: one of the table's, or up to 15 bytes of the input.
    fn key(&mut self) -> String {
        let pick = self.byte();
        if pick < 192 {
            return KEYS[usize::from(pick) % KEYS.len()].to_owned();
        }
        self.text(usize::from(pick & 15))
    }

    /// A value: a few bytes of the input, or a run of one character
    /// long enough to fill much of an area.
    fn value(&mut self) -> String {
        let pick = self.byte();
        if pick < 224 {
            return self.text(usize::from(pick & 31));
        }
        let unit = ['x', '\u{e9}', '\u{1F600}'][usize::from(pick) % 3];
        unit.to_string()
            .repeat(usize::from(self.byte()) * 16_384 + usize::from(self.byte()))
    }

    fn text(&mut self, length: usize) -> String {
        let length = length.min(self.0.len());
        let (taken, rest) = self.0.split_at(length);
        self.0 = rest;
        String::from_utf8_lossy(taken).into_owned()
    }
}

/// An area as the model has it: its items in their order.
type Area = Vec<(String, String)>;

/// The areas by whether they are `localStorage`, and origin.
type Areas = BTreeMap<(bool, &'static str), Area>;

fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `setItem` as HTML 12.2.1 and the quota say, written again here:
/// whether the area held it.
fn set(area: &mut Area, key: &str, value: &str) -> bool {
    let used: usize = area.iter().map(|(k, v)| units(k) + units(v)).sum();
    match area.iter_mut().find(|(k, _)| k == key) {
        Some(item) => {
            if used - units(&item.1) + units(value) > STORAGE_QUOTA {
                return false;
            }
            item.1 = value.to_owned();
        }
        None => {
            if used + units(key) + units(value) > STORAGE_QUOTA {
                return false;
            }
            area.push((key.to_owned(), value.to_owned()));
        }
    }
    true
}

/// The `localStorage` areas a profile holds, as `local` has them.
fn kept(profile: &Profile, local: &Areas) {
    for origin in ORIGINS {
        let items: Area = profile
            .local_storage(origin)
            .expect("the profile reads")
            .into_iter()
            .map(|item| (item.key, item.value))
            .collect();
        let expected = local.get(&(true, origin)).cloned().unwrap_or_default();
        assert_eq!(items, expected, "{origin}'s localStorage as written");
    }
}

/// The model's `localStorage` areas alone.
fn local(areas: &Areas) -> Areas {
    areas
        .iter()
        .filter(|((is_local, _), _)| *is_local)
        .map(|(key, area)| (*key, area.clone()))
        .collect()
}

fuzz_target!(|data: &[u8]| {
    let mut input = Input(data);
    let memory = Memory::new();
    let open = |memory: &Memory| Profile::in_memory(memory).expect("a profile opens");
    let mut driver = StorageDriver::new(Some(open(&memory))).expect("a profile reads");
    let mut areas = Areas::new();
    let mut written = Areas::new();
    let mut steps = 0;
    while !input.0.is_empty() && steps < 48 {
        steps += 1;
        let op = input.byte();
        let is_local = op & 1 == 0;
        let origin = ORIGINS[usize::from(op >> 1) % ORIGINS.len()];
        let area = areas.entry((is_local, origin)).or_default();
        match op >> 3 {
            0..=11 => {
                let (key, value) = (input.key(), input.value());
                let held = driver.set(is_local, origin, &key, &value);
                assert_eq!(held, set(area, &key, &value), "setItem's answer");
            }
            12..=17 => {
                let key = input.key();
                let old = area.iter().position(|(k, _)| *k == key);
                let old = old.map(|at| area.remove(at).1);
                assert_eq!(driver.remove(is_local, origin, &key), old);
            }
            18..=20 => {
                let had = !area.is_empty();
                area.clear();
                assert_eq!(driver.clear(is_local, origin), had);
            }
            21..=23 => {
                let key = input.key();
                let value = area.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone());
                assert_eq!(driver.get(is_local, origin, &key), value);
            }
            24..=26 => {
                driver.flush().expect("the profile writes");
                written = local(&areas);
            }
            27..=28 => {
                // Closed and opened again: what was written is there,
                // and the tab's sessionStorage is gone.
                driver.flush().expect("the profile writes");
                drop(driver);
                written = local(&areas);
                areas = written.clone();
                kept(&open(&memory), &written);
                driver = StorageDriver::new(Some(open(&memory))).expect("a profile reads");
            }
            29..=30 => {
                // A crash now: a copy has what the last write wrote.
                kept(&open(&memory.snapshot()), &written);
            }
            _ => {
                // A copy damaged where the input says: refused, or read
                // without a panic.
                let damaged = memory.snapshot();
                let at = usize::from(input.byte()) << 8 | usize::from(input.byte());
                damaged.flip(at % damaged.len().max(1), input.byte());
                if let Ok(profile) = Profile::in_memory(&damaged)
                    && let Ok(mut opened) = StorageDriver::new(Some(profile))
                {
                    for origin in ORIGINS {
                        opened.items(true, origin);
                    }
                }
            }
        }
        for ((is_local, origin), area) in &areas {
            assert_eq!(
                &driver.items(*is_local, origin),
                area,
                "{origin}'s area as the model has it"
            );
        }
    }
    driver.flush().expect("the profile writes");
    drop(driver);
    kept(&open(&memory), &local(&areas));
});
