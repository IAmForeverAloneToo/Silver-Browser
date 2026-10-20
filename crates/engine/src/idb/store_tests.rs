//! IndexedDB's store (M36.4): versions, transactions committed and
//! aborted, key generators, indices, cursors, the quota, and what the
//! profile keeps of it.

use super::key::Key;
use super::range::KeyRange;
use super::store::*;

const O: &str = "https://a.test";

fn n(n: f64) -> Key {
    Key::Number(n)
}

fn s(text: &str) -> Key {
    Key::string(text)
}

/// A store with database "db" at version 1, its object store "people"
/// keyed by an auto-incremented generator and indexed by "name", not
/// unique, and "email", unique.
fn people() -> IdbStore {
    let mut store = IdbStore::default();
    store.set_writing(true);
    let (before, upgrade) = store.open(O, "db", Some(1)).expect("opened");
    assert_eq!(before, 0);
    let txn = upgrade.expect("an upgrade");
    store
        .create_store(txn, "people", KeyPath::String("id".into()), true)
        .expect("made");
    store
        .create_index(
            txn,
            "people",
            "name",
            (KeyPath::String("name".into()), false, false),
            vec![],
        )
        .expect("indexed");
    store
        .create_index(
            txn,
            "people",
            "email",
            (KeyPath::String("email".into()), true, false),
            vec![],
        )
        .expect("indexed");
    store.commit(txn).expect("committed");
    store
}

fn add(store: &mut IdbStore, txn: u64, name: &str, email: &str) -> Key {
    let key = store.generate_key(txn, "people").expect("a key");
    store
        .put(
            txn,
            "people",
            Some(key.clone()),
            format!("{name}:{email}").into_bytes(),
            vec![
                ("name".into(), vec![s(name)]),
                ("email".into(), vec![s(email)]),
            ],
            true,
        )
        .expect("added")
}

fn values(found: &[Found]) -> Vec<String> {
    found
        .iter()
        .map(|f| String::from_utf8(f.value.clone().unwrap_or_default()).unwrap())
        .collect()
}

#[test]
fn a_database_is_opened_upgraded_and_its_version_kept() {
    let mut store = people();
    assert_eq!(store.databases(O), [("db".to_owned(), 1)]);
    assert_eq!(store.open(O, "db", None), Ok((1, None)));
    assert_eq!(store.open(O, "db", Some(1)), Ok((1, None)));
    assert_eq!(
        store.open(O, "db", Some(0)).map_err(|e| e.name),
        Err("TypeError")
    );
    // Another database of the origin, opened at a version asked for.
    let (before, upgrade) = store.open(O, "x", Some(2)).expect("opened");
    assert_eq!(before, 0);
    store
        .commit(upgrade.expect("an upgrade"))
        .expect("committed");
    assert_eq!(store.databases(O).len(), 2);
    let stores = store.stores(O, "db");
    assert_eq!(stores[0].name, "people");
    assert!(stores[0].auto_increment);
    assert_eq!(stores[0].indices.len(), 2);
    // Another origin's databases are its own.
    assert!(store.databases("https://b.test").is_empty());
}

#[test]
fn a_version_below_the_databases_is_refused() {
    let mut store = people();
    let (_, upgrade) = store.open(O, "db", Some(3)).expect("opened");
    store
        .commit(upgrade.expect("an upgrade"))
        .expect("committed");
    let error = store.open(O, "db", Some(2)).expect_err("lower");
    assert_eq!(error.name, "VersionError");
}

#[test]
fn records_are_put_got_counted_and_deleted_by_key_and_index() {
    let mut store = people();
    let txn = store
        .begin(O, "db", Mode::ReadWrite, vec!["people".into()])
        .expect("begun");
    assert_eq!(add(&mut store, txn, "bo", "bo@a"), n(1.0));
    assert_eq!(add(&mut store, txn, "al", "al@a"), n(2.0));
    assert_eq!(add(&mut store, txn, "bo", "bo2@a"), n(3.0));
    let all = store
        .get_all(txn, ("people", None), &KeyRange::all(), 0, false)
        .expect("all");
    assert_eq!(values(&all), ["bo:bo@a", "al:al@a", "bo:bo2@a"]);
    // By index: by name, then by primary key.
    let by_name = store
        .get_all(txn, ("people", Some("name")), &KeyRange::all(), 0, false)
        .expect("all");
    assert_eq!(values(&by_name), ["al:al@a", "bo:bo@a", "bo:bo2@a"]);
    assert_eq!(by_name[1].primary, n(1.0));
    assert_eq!(
        store.count(txn, ("people", Some("name")), &KeyRange::only(s("bo"))),
        Ok(2)
    );
    // A unique index refuses a second record of a key.
    let error = store
        .put(
            txn,
            "people",
            Some(n(9.0)),
            b"x".to_vec(),
            vec![("email".into(), vec![s("al@a")])],
            false,
        )
        .expect_err("not unique");
    assert_eq!(error.name, "ConstraintError");
    // add() refuses a key there; put() replaces the record.
    let error = store
        .put(txn, "people", Some(n(1.0)), b"y".to_vec(), vec![], true)
        .expect_err("there");
    assert_eq!(error.name, "ConstraintError");
    store
        .put(txn, "people", Some(n(1.0)), b"y".to_vec(), vec![], false)
        .expect("replaced");
    // Its old index keys go with it.
    assert_eq!(
        store.count(txn, ("people", Some("name")), &KeyRange::only(s("bo"))),
        Ok(1)
    );
    store
        .delete(
            txn,
            "people",
            &KeyRange {
                lower: Some((n(2.0), false)),
                upper: None,
            },
        )
        .expect("deleted");
    assert_eq!(store.count(txn, ("people", None), &KeyRange::all()), Ok(1));
    assert_eq!(
        store.count(txn, ("people", Some("email")), &KeyRange::all()),
        Ok(0)
    );
    store.commit(txn).expect("committed");
    // A finished transaction does nothing more.
    assert_eq!(
        store
            .count(txn, ("people", None), &KeyRange::all())
            .map_err(|e| e.name),
        Err("InvalidStateError")
    );
}

#[test]
fn an_abort_undoes_what_its_transaction_did() {
    let mut store = people();
    let txn = store
        .begin(O, "db", Mode::ReadWrite, vec!["people".into()])
        .expect("begun");
    add(&mut store, txn, "bo", "bo@a");
    store.commit(txn).expect("committed");
    let used = store.used(O);
    let txn = store
        .begin(O, "db", Mode::ReadWrite, vec!["people".into()])
        .expect("begun");
    add(&mut store, txn, "al", "al@a");
    store
        .put(
            txn,
            "people",
            Some(n(1.0)),
            b"changed".to_vec(),
            vec![],
            false,
        )
        .expect("put");
    store
        .delete(txn, "people", &KeyRange::all())
        .expect("deleted");
    store.abort(txn).expect("aborted");
    let check = store
        .begin(O, "db", Mode::ReadOnly, vec!["people".into()])
        .expect("begun");
    let all = store
        .get_all(check, ("people", None), &KeyRange::all(), 0, false)
        .expect("all");
    assert_eq!(values(&all), ["bo:bo@a"]);
    assert_eq!(
        store.count(check, ("people", Some("email")), &KeyRange::only(s("bo@a"))),
        Ok(1)
    );
    assert_eq!(store.used(O), used);
    store.commit(check).expect("done");
    // The generator went back too.
    let txn = store
        .begin(O, "db", Mode::ReadWrite, vec!["people".into()])
        .expect("begun");
    assert_eq!(store.generate_key(txn, "people"), Ok(n(2.0)));
    // A version change aborted leaves the database as it was; a new
    // database's, none.
    let (_, upgrade) = store.open(O, "db", Some(2)).expect("opened");
    let upgrade = upgrade.expect("an upgrade");
    store.delete_store(upgrade, "people").expect("deleted");
    store.abort(upgrade).expect("aborted");
    assert_eq!(store.databases(O), [("db".to_owned(), 1)]);
    assert_eq!(store.stores(O, "db").len(), 1);
    let (_, upgrade) = store.open(O, "new", None).expect("opened");
    store.abort(upgrade.expect("an upgrade")).expect("aborted");
    assert_eq!(store.databases(O).len(), 1);
}

#[test]
fn a_read_only_transaction_writes_nothing_and_a_scope_is_kept_to() {
    let mut store = people();
    let txn = store
        .begin(O, "db", Mode::ReadOnly, vec!["people".into()])
        .expect("begun");
    let error = store
        .put(txn, "people", Some(n(1.0)), vec![], vec![], false)
        .expect_err("read-only");
    assert_eq!(error.name, "ReadOnlyError");
    let error = store
        .begin(O, "db", Mode::ReadWrite, vec!["nothing".into()])
        .expect_err("no such store");
    assert_eq!(error.name, "NotFoundError");
    let error = store
        .create_store(txn, "more", KeyPath::None, false)
        .expect_err("no version change");
    assert_eq!(error.name, "ReadOnlyError");
}

#[test]
fn a_number_key_moves_the_generator_and_the_generator_ends() {
    let mut store = people();
    let txn = store
        .begin(O, "db", Mode::ReadWrite, vec!["people".into()])
        .expect("begun");
    store
        .put(txn, "people", Some(n(10.5)), vec![], vec![], false)
        .expect("put");
    assert_eq!(store.generate_key(txn, "people"), Ok(n(11.0)));
    store
        .put(
            txn,
            "people",
            Some(n(9_007_199_254_740_992.0)),
            vec![],
            vec![],
            false,
        )
        .expect("put");
    assert_eq!(
        store.generate_key(txn, "people").map_err(|e| e.name),
        Err("ConstraintError")
    );
    // A string key leaves it be.
    store.abort(txn).expect("aborted");
    let txn = store
        .begin(O, "db", Mode::ReadWrite, vec!["people".into()])
        .expect("begun");
    store
        .put(txn, "people", Some(s("x")), vec![], vec![], false)
        .expect("put");
    assert_eq!(store.generate_key(txn, "people"), Ok(n(1.0)));
}

#[test]
fn a_cursor_goes_each_way_and_skips_and_continues() {
    let mut store = people();
    let txn = store
        .begin(O, "db", Mode::ReadWrite, vec!["people".into()])
        .expect("begun");
    for (name, email) in [("bo", "1"), ("al", "2"), ("bo", "3"), ("cy", "4")] {
        add(&mut store, txn, name, email);
    }
    let walk = |direction: Direction| -> Vec<(String, f64)> {
        let mut out = Vec::new();
        let mut at: Option<(Key, Key)> = None;
        loop {
            let found = store
                .iterate(
                    txn,
                    ("people", Some("name")),
                    &KeyRange::all(),
                    direction,
                    at.as_ref().map(|(k, p)| (k, p)),
                    None,
                    1,
                    true,
                )
                .expect("iterated");
            let Some(found) = found else {
                return out;
            };
            let Key::String(units) = &found.key else {
                panic!("{found:?}")
            };
            let Key::Number(p) = found.primary else {
                panic!("{found:?}")
            };
            out.push((String::from_utf16(units).unwrap(), p));
            at = Some((found.key, found.primary));
        }
    };
    let names = |list: &[(&str, f64)]| -> Vec<(String, f64)> {
        list.iter().map(|(s, p)| ((*s).to_owned(), *p)).collect()
    };
    assert_eq!(
        walk(Direction::Next),
        names(&[("al", 2.0), ("bo", 1.0), ("bo", 3.0), ("cy", 4.0)])
    );
    assert_eq!(
        walk(Direction::Prev),
        names(&[("cy", 4.0), ("bo", 3.0), ("bo", 1.0), ("al", 2.0)])
    );
    assert_eq!(
        walk(Direction::NextUnique),
        names(&[("al", 2.0), ("bo", 1.0), ("cy", 4.0)])
    );
    // Backwards and unique, each key's record of the least primary key.
    assert_eq!(
        walk(Direction::PrevUnique),
        names(&[("cy", 4.0), ("bo", 1.0), ("al", 2.0)])
    );
    // advance(2) from the start, and continue() to a key.
    let second = store
        .iterate(
            txn,
            ("people", Some("name")),
            &KeyRange::all(),
            Direction::Next,
            None,
            None,
            2,
            true,
        )
        .expect("iterated")
        .expect("one");
    assert_eq!(second.primary, n(1.0));
    // An index's range open at its lower key leaves that key's records
    // out.
    let open = KeyRange {
        lower: Some((s("al"), true)),
        upper: Some((s("bo"), false)),
    };
    let after_al = store
        .get_all(txn, ("people", Some("name")), &open, 0, true)
        .expect("all");
    let primaries: Vec<Key> = after_al.into_iter().map(|f| f.primary).collect();
    assert_eq!(primaries, [n(1.0), n(3.0)]);
    // A unique cursor advanced counts each key once.
    let third = store
        .iterate(
            txn,
            ("people", Some("name")),
            &KeyRange::all(),
            Direction::NextUnique,
            None,
            None,
            3,
            true,
        )
        .expect("iterated")
        .expect("one");
    assert_eq!(third.key, s("cy"));
    let to = s("c");
    let reached = store
        .iterate(
            txn,
            ("people", Some("name")),
            &KeyRange::all(),
            Direction::Next,
            Some((&s("al"), &n(2.0))),
            Some((&to, None)),
            1,
            true,
        )
        .expect("iterated")
        .expect("one");
    assert_eq!(reached.key, s("cy"));
    // The object store's own cursor, keys and values.
    let first = store
        .iterate(
            txn,
            ("people", None),
            &KeyRange::all(),
            Direction::Prev,
            None,
            None,
            1,
            false,
        )
        .expect("iterated")
        .expect("one");
    assert_eq!(first.key, n(4.0));
    assert_eq!(first.value.as_deref(), Some(&b"cy:4"[..]));
}

#[test]
fn an_origin_is_held_to_its_quota_and_a_value_to_its_bound() {
    let mut store = people();
    let txn = store
        .begin(O, "db", Mode::ReadWrite, vec!["people".into()])
        .expect("begun");
    let big = vec![0; MAX_IDB_VALUE_BYTES + 1];
    let error = store
        .put(txn, "people", Some(n(1.0)), big, vec![], false)
        .expect_err("too big");
    assert_eq!(error.name, "DataError");
    let chunk = vec![0; MAX_IDB_VALUE_BYTES];
    let mut stored = 0;
    let error = loop {
        match store.put(
            txn,
            "people",
            Some(n(stored as f64)),
            chunk.clone(),
            vec![],
            false,
        ) {
            Ok(_) => stored += 1,
            Err(error) => break error,
        }
    };
    assert_eq!(error.name, "QuotaExceededError");
    assert!(store.used(O) <= IDB_ORIGIN_QUOTA);
    assert_eq!(stored, (IDB_ORIGIN_QUOTA / MAX_IDB_VALUE_BYTES as u64) - 1);
}

#[test]
fn what_is_committed_is_written_and_read_back_whole() {
    let mut store = people();
    let txn = store
        .begin(O, "db", Mode::ReadWrite, vec!["people".into()])
        .expect("begun");
    add(&mut store, txn, "bo", "bo@a");
    add(&mut store, txn, "al", "al@a");
    store
        .delete(txn, "people", &KeyRange::only(n(1.0)))
        .expect("deleted");
    // Nothing is written before the commit.
    let before: Vec<Written> = store.take_written();
    assert!(before.iter().all(|w| !matches!(w, Written::Record { .. })));
    store.commit(txn).expect("committed");
    let written = store.take_written();
    let mut databases = Vec::new();
    let mut records = std::collections::BTreeMap::new();
    for w in before.into_iter().chain(written) {
        match w {
            Written::Database { name, meta, .. } => databases.push((name, meta)),
            Written::Record {
                database,
                store,
                key,
                record,
                ..
            } => {
                records.insert((database, store, key), record);
            }
            other => panic!("{other:?}"),
        }
    }
    let records: Vec<(String, String, Vec<u8>, Vec<u8>)> = records
        .into_iter()
        .filter_map(|((d, s, k), r)| Some((d, s, k, r?)))
        .collect();
    assert_eq!(records.len(), 1);
    let mut again = IdbStore::default();
    again.load(O, databases.into_iter().rev().take(1).collect(), records);
    let txn = again
        .begin(O, "db", Mode::ReadOnly, vec!["people".into()])
        .expect("begun");
    let all = again
        .get_all(txn, ("people", Some("email")), &KeyRange::all(), 0, false)
        .expect("all");
    assert_eq!(values(&all), ["al:al@a"]);
    assert_eq!(again.used(O), store.used(O));
    // The generator came back where it was.
    again.commit(txn).expect("done");
    let txn = again
        .begin(O, "db", Mode::ReadWrite, vec!["people".into()])
        .expect("begun");
    assert_eq!(again.generate_key(txn, "people"), Ok(n(3.0)));
}

#[test]
fn a_database_deleted_is_gone_with_its_transactions() {
    let mut store = people();
    let txn = store
        .begin(O, "db", Mode::ReadWrite, vec!["people".into()])
        .expect("begun");
    add(&mut store, txn, "bo", "bo@a");
    assert_eq!(store.delete_database(O, "db"), 1);
    assert_eq!(store.used(O), 0);
    assert!(store.databases(O).is_empty());
    assert_eq!(
        store.commit(txn).map_err(|e| e.name),
        Err("InvalidStateError")
    );
    assert_eq!(store.delete_database(O, "db"), 0);
    assert!(store.take_written().contains(&Written::DatabaseDeleted {
        origin: O.into(),
        name: "db".into()
    }));
}

#[test]
fn a_unique_index_made_on_records_sharing_a_key_fails() {
    let mut store = people();
    let txn = store
        .begin(O, "db", Mode::ReadWrite, vec!["people".into()])
        .expect("begun");
    add(&mut store, txn, "bo", "1");
    add(&mut store, txn, "bo", "2");
    store.commit(txn).expect("committed");
    let (_, upgrade) = store.open(O, "db", Some(2)).expect("opened");
    let upgrade = upgrade.expect("an upgrade");
    let keys = vec![(n(1.0), vec![s("bo")]), (n(2.0), vec![s("bo")])];
    let error = store
        .create_index(
            upgrade,
            "people",
            "unique-name",
            (KeyPath::String("name".into()), true, false),
            keys.clone(),
        )
        .expect_err("not unique");
    assert_eq!(error.name, "ConstraintError");
    // Not unique, it is made of the records there.
    store
        .create_index(
            upgrade,
            "people",
            "by-name",
            (KeyPath::String("name".into()), false, true),
            keys,
        )
        .expect("made");
    assert_eq!(
        store.count(upgrade, ("people", Some("by-name")), &KeyRange::all()),
        Ok(2)
    );
}

#[test]
fn a_record_is_kept_with_its_index_keys() {
    assert!(roundtrip_record(Vec::new(), Vec::new()));
    assert!(roundtrip_record(
        b"value".to_vec(),
        vec![
            ("name".into(), vec![s("a"), n(1.0)]),
            ("tags".into(), vec![]),
            (
                "deep".into(),
                vec![Key::Array(vec![Key::Binary(vec![0, 1])])]
            ),
        ]
    ));
}
