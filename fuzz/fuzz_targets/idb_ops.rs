//! IndexedDB's store (M36.4) under a script of operations, held to a
//! model of it in memory after each: transactions that commit or abort,
//! puts, adds, deletes by range and clears of two object stores, one
//! with a unique index; and what was committed, read back from what the
//! store wrote for the profile, the same.

#![no_main]
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use arbitrary::Arbitrary;
use engine::idb::{IdbStore, Key, KeyPath, KeyRange, Mode, Written};
use libfuzzer_sys::fuzz_target;

const O: &str = "https://fuzz.test";
const STORES: [&str; 2] = ["a", "b"];

#[derive(Arbitrary, Debug)]
enum Op {
    Begin(bool),
    Put {
        store: bool,
        key: u8,
        value: u8,
        add: bool,
    },
    Delete {
        store: bool,
        low: u8,
        high: u8,
    },
    Commit,
    Abort,
}

type Model = BTreeMap<(usize, u8), u8>;

fn key(k: u8) -> Key {
    Key::Number(f64::from(k))
}

/// The store's records, as the model has them.
fn records(store: &IdbStore, txn: u64) -> Model {
    let mut out = Model::new();
    for (i, name) in STORES.iter().enumerate() {
        let all = store
            .get_all(txn, (name, None), &KeyRange::all(), 0, false)
            .expect("readable");
        for found in all {
            let Key::Number(k) = found.key else {
                panic!("{found:?}");
            };
            out.insert((i, k as u8), found.value.expect("a value")[0]);
        }
    }
    out
}

fuzz_target!(|ops: Vec<Op>| {
    let mut store = IdbStore::default();
    store.set_writing(true);
    let (_, upgrade) = store.open(O, "db", Some(1)).expect("opened");
    let upgrade = upgrade.expect("an upgrade");
    for name in STORES {
        store
            .create_store(upgrade, name, KeyPath::None, false)
            .expect("made");
    }
    store
        .create_index(
            upgrade,
            "b",
            "u",
            (KeyPath::String("v".into()), true, false),
            vec![],
        )
        .expect("indexed");
    store.commit(upgrade).expect("committed");
    let mut committed = Model::new();
    let mut open: Option<(u64, Model)> = None;
    let mut written = store.take_written();
    for op in ops.into_iter().take(64) {
        match op {
            Op::Begin(write) => {
                if open.is_none() {
                    let mode = if write {
                        Mode::ReadWrite
                    } else {
                        Mode::ReadOnly
                    };
                    let scope = STORES.iter().map(|s| (*s).to_owned()).collect();
                    let txn = store.begin(O, "db", mode, scope).expect("begun");
                    open = Some((txn, committed.clone()));
                }
            }
            Op::Put {
                store: b,
                key: k,
                value,
                add,
            } => {
                let Some((txn, model)) = &mut open else {
                    continue;
                };
                let i = usize::from(b);
                // Store "b"'s index is unique over the value.
                let index = if b {
                    vec![("u".to_owned(), vec![key(value)])]
                } else {
                    vec![]
                };
                let result = store.put(*txn, STORES[i], Some(key(k)), vec![value], index, add);
                let taken = b
                    && model
                        .iter()
                        .any(|(&(s, kk), &v)| s == 1 && v == value && kk != k);
                let expected_ok = !(add && model.contains_key(&(i, k))) && !taken;
                match result {
                    Ok(_) => {
                        assert!(expected_ok, "{result:?}");
                        model.insert((i, k), value);
                    }
                    Err(error) if error.name == "ReadOnlyError" => {}
                    Err(error) => {
                        assert!(!expected_ok, "{error:?}");
                        assert_eq!(error.name, "ConstraintError");
                    }
                }
            }
            Op::Delete {
                store: b,
                low,
                high,
            } => {
                let Some((txn, model)) = &mut open else {
                    continue;
                };
                let (low, high) = (low.min(high), low.max(high));
                let range = KeyRange {
                    lower: Some((key(low), false)),
                    upper: Some((key(high), false)),
                };
                let i = usize::from(b);
                if store.delete(*txn, STORES[i], &range).is_ok() {
                    model.retain(|&(s, k), _| s != i || k < low || k > high);
                }
            }
            Op::Commit => {
                if let Some((txn, model)) = open.take() {
                    store.commit(txn).expect("committed");
                    committed = model;
                }
            }
            Op::Abort => {
                if let Some((txn, _)) = open.take() {
                    store.abort(txn).expect("aborted");
                }
            }
        }
        // The store as the model has it, within the transaction or not.
        let (txn, model) = match &open {
            Some((txn, model)) => (*txn, model.clone()),
            None => {
                let scope = STORES.iter().map(|s| (*s).to_owned()).collect();
                let txn = store.begin(O, "db", Mode::ReadOnly, scope).expect("begun");
                (txn, committed.clone())
            }
        };
        assert_eq!(records(&store, txn), model);
        if open.is_none() {
            store.commit(txn).expect("done");
        }
    }
    if let Some((txn, _)) = open.take() {
        store.abort(txn).expect("aborted");
    }
    // What was written for the profile reads back as what was committed.
    written.extend(store.take_written());
    let mut databases = Vec::new();
    let mut kept: BTreeMap<(String, Vec<u8>), Option<Vec<u8>>> = BTreeMap::new();
    for w in written {
        match w {
            Written::Database { name, meta, .. } => databases = vec![(name, meta)],
            Written::Record {
                store, key, record, ..
            } => {
                kept.insert((store, key), record);
            }
            _ => {}
        }
    }
    let records_kept = kept
        .into_iter()
        .filter_map(|((s, k), r)| Some(("db".to_owned(), s, k, r?)))
        .collect();
    let mut again = IdbStore::default();
    again.load(O, databases, records_kept);
    let scope = STORES.iter().map(|s| (*s).to_owned()).collect();
    let txn = again.begin(O, "db", Mode::ReadOnly, scope).expect("begun");
    assert_eq!(records(&again, txn), committed);
});
