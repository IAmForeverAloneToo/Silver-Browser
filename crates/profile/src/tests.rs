use std::path::PathBuf;

use super::*;

fn item(key: &str, value: &str, order: u64) -> Item {
    Item {
        key: key.to_owned(),
        value: value.to_owned(),
        order,
    }
}

/// A new directory under the system's temporary one, removed first if
/// an earlier run left it.
fn directory(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("silver-profile-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn items_come_back_in_their_order() {
    let profile = Profile::in_memory(&Memory::new()).unwrap();
    let mut changes = Changes::new();
    changes.set_item("http://a.test", "b", "1", 0);
    changes.set_item("http://a.test", "a", "2", 1);
    changes.set_item("http://a.test", "c", "3", 2);
    profile.commit(&changes).unwrap();
    assert_eq!(
        profile.local_storage("http://a.test").unwrap(),
        [item("b", "1", 0), item("a", "2", 1), item("c", "3", 2)]
    );
}

#[test]
fn an_origin_reads_only_its_own() {
    let profile = Profile::in_memory(&Memory::new()).unwrap();
    let mut changes = Changes::new();
    // Origins one a prefix of another, and the empty one.
    for (index, origin) in ["http://a.test", "http://a.test:8080", "http://a.tes", ""]
        .into_iter()
        .enumerate()
    {
        changes.set_item(origin, "k", origin, index as u64);
    }
    profile.commit(&changes).unwrap();
    for origin in ["http://a.test", "http://a.test:8080", "http://a.tes", ""] {
        let items = profile.local_storage(origin).unwrap();
        assert_eq!(items.len(), 1, "{origin}");
        assert_eq!(items[0].value, origin);
    }
    assert!(profile.local_storage("http://b.test").unwrap().is_empty());
}

#[test]
fn changes_apply_in_their_order() {
    let profile = Profile::in_memory(&Memory::new()).unwrap();
    let mut changes = Changes::new();
    changes.set_item("o", "a", "1", 0);
    changes.set_item("o", "b", "2", 1);
    changes.set_item("p", "a", "kept", 0);
    profile.commit(&changes).unwrap();

    let mut changes = Changes::new();
    changes.set_item("o", "a", "replaced", 0);
    changes.remove_item("o", "b");
    changes.remove_item("o", "missing");
    changes.set_item("o", "c", "3", 2);
    profile.commit(&changes).unwrap();
    assert_eq!(
        profile.local_storage("o").unwrap(),
        [item("a", "replaced", 0), item("c", "3", 2)]
    );

    let mut changes = Changes::new();
    changes.clear("o");
    changes.set_item("o", "after", "x", 7);
    profile.commit(&changes).unwrap();
    assert_eq!(profile.local_storage("o").unwrap(), [item("after", "x", 7)]);
    assert_eq!(profile.local_storage("p").unwrap(), [item("a", "kept", 0)]);
}

#[test]
fn keys_and_values_keep_every_character() {
    let profile = Profile::in_memory(&Memory::new()).unwrap();
    let odd = ["", "\0", "a\0b", "\u{fffd}", "\u{10ffff}", " "];
    let mut changes = Changes::new();
    for (index, text) in (0u64..).zip(odd) {
        changes.set_item("o", &format!("{text}{index}"), text, index);
    }
    profile.commit(&changes).unwrap();
    let items = profile.local_storage("o").unwrap();
    assert_eq!(items.len(), odd.len());
    for (index, (item, text)) in (0u64..).zip(items.iter().zip(odd)) {
        assert_eq!(item.key, format!("{text}{index}"));
        assert_eq!(item.value, text);
    }
}

#[test]
fn hsts_hosts_are_replaced_whole_apart_from_the_cookies() {
    let profile = Profile::in_memory(&Memory::new()).unwrap();
    assert!(profile.hsts().unwrap().is_empty());
    let mut changes = Changes::new();
    changes.hsts(vec![b"b.test".to_vec(), b"a.test".to_vec()]);
    changes.cookies(vec![b"c".to_vec()]);
    profile.commit(&changes).unwrap();
    assert_eq!(
        profile.hsts().unwrap(),
        [b"b.test".to_vec(), b"a.test".to_vec()]
    );
    let mut changes = Changes::new();
    changes.hsts(vec![b"only.test".to_vec()]);
    profile.commit(&changes).unwrap();
    assert_eq!(profile.hsts().unwrap(), [b"only.test".to_vec()]);
    assert_eq!(profile.cookies().unwrap(), [b"c".to_vec()]);
    // Cookies alone leave the hosts.
    let mut changes = Changes::new();
    changes.cookies(Vec::new());
    profile.commit(&changes).unwrap();
    assert_eq!(profile.hsts().unwrap(), [b"only.test".to_vec()]);
}

#[test]
fn permissions_are_replaced_whole_apart_from_the_hosts() {
    let profile = Profile::in_memory(&Memory::new()).unwrap();
    assert!(profile.permissions().unwrap().is_empty());
    let mut changes = Changes::new();
    changes.permissions(vec![b"p".to_vec(), b"o".to_vec()]);
    changes.hsts(vec![b"h".to_vec()]);
    profile.commit(&changes).unwrap();
    let mut changes = Changes::new();
    changes.permissions(vec![b"q".to_vec()]);
    profile.commit(&changes).unwrap();
    assert_eq!(profile.permissions().unwrap(), [b"q".to_vec()]);
    assert_eq!(profile.hsts().unwrap(), [b"h".to_vec()]);
}

#[test]
fn cookies_are_replaced_whole_and_keep_their_order() {
    let profile = Profile::in_memory(&Memory::new()).unwrap();
    assert!(profile.cookies().unwrap().is_empty());
    let mut changes = Changes::new();
    changes.cookies(vec![b"z".to_vec(), b"a".to_vec(), Vec::new(), vec![0, 255]]);
    profile.commit(&changes).unwrap();
    assert_eq!(
        profile.cookies().unwrap(),
        [b"z".to_vec(), b"a".to_vec(), Vec::new(), vec![0, 255]]
    );
    let mut changes = Changes::new();
    changes.cookies(vec![b"only".to_vec()]);
    profile.commit(&changes).unwrap();
    assert_eq!(profile.cookies().unwrap(), [b"only".to_vec()]);
    // Changes without cookies leave them.
    let mut changes = Changes::new();
    changes.set_item("o", "k", "v", 0);
    profile.commit(&changes).unwrap();
    assert_eq!(profile.cookies().unwrap(), [b"only".to_vec()]);
    let mut changes = Changes::new();
    changes.cookies(Vec::new());
    profile.commit(&changes).unwrap();
    assert!(profile.cookies().unwrap().is_empty());
}

#[test]
fn nothing_to_commit_is_empty() {
    let mut changes = Changes::new();
    assert!(changes.is_empty());
    changes.clear("o");
    assert!(!changes.is_empty());
    let mut changes = Changes::new();
    changes.cookies(Vec::new());
    assert!(!changes.is_empty());
}

#[test]
fn memory_opened_again_has_what_was_committed() {
    let memory = Memory::new();
    {
        let profile = Profile::in_memory(&memory).unwrap();
        let mut changes = Changes::new();
        changes.set_item("o", "k", "v", 3);
        changes.cookies(vec![b"c".to_vec()]);
        profile.commit(&changes).unwrap();
    }
    assert!(!memory.is_empty());
    let profile = Profile::in_memory(&memory).unwrap();
    assert_eq!(profile.local_storage("o").unwrap(), [item("k", "v", 3)]);
    assert_eq!(profile.cookies().unwrap(), [b"c".to_vec()]);
}

#[test]
fn a_snapshot_is_what_a_crash_leaves() {
    // What was committed before a snapshot is in it, and what was
    // committed after is not: a browser stopped between two commits
    // finds the first.
    let memory = Memory::new();
    let profile = Profile::in_memory(&memory).unwrap();
    let mut changes = Changes::new();
    changes.set_item("o", "first", "1", 0);
    profile.commit(&changes).unwrap();
    let crashed = memory.snapshot();
    let mut changes = Changes::new();
    changes.set_item("o", "second", "2", 1);
    profile.commit(&changes).unwrap();
    drop(profile);
    let reopened = Profile::in_memory(&crashed).unwrap();
    assert_eq!(
        reopened.local_storage("o").unwrap(),
        [item("first", "1", 0)]
    );
    let reopened = Profile::in_memory(&memory).unwrap();
    assert_eq!(reopened.local_storage("o").unwrap().len(), 2);
}

#[test]
fn a_directory_opened_again_has_what_was_committed() {
    let dir = directory("again");
    {
        let profile = Profile::open(&dir).unwrap();
        let mut changes = Changes::new();
        changes.set_item("http://a.test", "k", "v", 0);
        changes.cookies(vec![b"c".to_vec()]);
        profile.commit(&changes).unwrap();
    }
    assert!(dir.join(FILE).is_file());
    let profile = Profile::open(&dir).unwrap();
    assert_eq!(
        profile.local_storage("http://a.test").unwrap(),
        [item("k", "v", 0)]
    );
    assert_eq!(profile.cookies().unwrap(), [b"c".to_vec()]);
    drop(profile);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_profile_open_is_refused_to_a_second() {
    let dir = directory("in-use");
    let profile = Profile::open(&dir).unwrap();
    assert!(matches!(Profile::open(&dir), Err(OpenError::InUse)));
    drop(profile);
    let profile = Profile::open(&dir).unwrap();
    drop(profile);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_that_is_not_a_profile_is_refused() {
    let dir = directory("garbage");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(FILE), vec![0x5a; 9000]).unwrap();
    match Profile::open(&dir) {
        Err(OpenError::Unreadable(why)) => assert!(!why.is_empty()),
        other => panic!("{other:?}"),
    }
    // And left as it was, for the user to look at.
    assert_eq!(std::fs::read(dir.join(FILE)).unwrap(), vec![0x5a; 9000]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_new_profile_records_its_schema() {
    let memory = Memory::new();
    drop(Profile::in_memory(&memory).unwrap());
    let db = Database::builder()
        .create_with_backend(memory.clone())
        .unwrap();
    let txn = db.begin_read().unwrap();
    let meta = txn.open_table(META).unwrap();
    assert_eq!(meta.get("schema").unwrap().unwrap().value(), SCHEMA);
}

#[test]
fn a_newer_schema_is_refused() {
    let memory = Memory::new();
    drop(Profile::in_memory(&memory).unwrap());
    {
        let db = Database::builder()
            .create_with_backend(memory.clone())
            .unwrap();
        let txn = db.begin_write().unwrap();
        txn.open_table(META)
            .unwrap()
            .insert("schema", SCHEMA + 1)
            .unwrap();
        txn.commit().unwrap();
    }
    match Profile::in_memory(&memory) {
        Err(OpenError::Newer(schema)) => assert_eq!(schema, SCHEMA + 1),
        other => panic!("{other:?}"),
    }
}

#[test]
fn reasons_read_as_sentences() {
    assert_eq!(OpenError::InUse.to_string(), "another browser has it open");
    assert_eq!(
        OpenError::Newer(2).to_string(),
        "a newer browser wrote it (schema 2)"
    );
    assert_eq!(
        OpenError::Unreadable("why".into()).to_string(),
        "it cannot be read: why"
    );
}

#[test]
fn a_damaged_profile_is_refused_or_read_without_a_panic() {
    let memory = Memory::new();
    {
        let profile = Profile::in_memory(&memory).unwrap();
        let mut changes = Changes::new();
        changes.set_item("o", "k", "v", 0);
        changes.cookies(vec![b"c".to_vec()]);
        profile.commit(&changes).unwrap();
    }
    let length = memory.len();
    for at in (0..length).step_by(length / 64 + 1) {
        let damaged = memory.snapshot();
        damaged.flip(at, 0xff);
        if let Ok(profile) = Profile::in_memory(&damaged) {
            let _ = profile.local_storage("o");
            let _ = profile.cookies();
        }
    }
    // Damaged under a profile open: its reads and writes fail, or
    // succeed, and do not panic.
    for at in (0..length).step_by(length / 256 + 1) {
        let copy = memory.snapshot();
        let Ok(profile) = Profile::in_memory(&copy) else {
            continue;
        };
        copy.flip(at, 0xff);
        let mut changes = Changes::new();
        changes.set_item("o", "k2", "v2", 1);
        let _ = (
            profile.local_storage("o"),
            profile.cookies(),
            profile.commit(&changes),
        );
    }
    // Past the end, nothing changes.
    let copy = memory.snapshot();
    copy.flip(length, 0xff);
    assert_eq!(copy.len(), length);
}

#[test]
fn a_store_that_panics_is_the_profiles_error_and_used_no_more() {
    let profile = Profile::in_memory(&Memory::new()).unwrap();
    let panicked: Result<(), Error> = profile.guarded(|_| panic!("a damaged page"));
    assert_eq!(panicked, Err(damaged()));
    assert_eq!(profile.cookies(), Err(damaged()));
    assert_eq!(profile.local_storage("o"), Err(damaged()));
    assert_eq!(profile.commit(&Changes::new()), Ok(()));
    let mut changes = Changes::new();
    changes.clear("o");
    assert_eq!(profile.commit(&changes), Err(damaged()));
}

/// A wrap of the tests' own: the key turned over bit by bit, under the
/// name `test`; and one of the same name that unwraps nothing, as a
/// profile copied to another user meets.
fn turned(key: &[u8]) -> Option<Vec<u8>> {
    Some(key.iter().map(|byte| !byte).collect())
}

fn nothing(_: &[u8]) -> Option<Vec<u8>> {
    None
}

const TEST_WRAP: KeyWrap = KeyWrap {
    name: "test",
    wrap: turned,
    unwrap: turned,
};

const OTHER_USER: KeyWrap = KeyWrap {
    name: "test",
    wrap: turned,
    unwrap: nothing,
};

fn short(_: &[u8]) -> Option<Vec<u8>> {
    Some(vec![1; 5])
}

/// One that unwraps to what is not a key.
const NOT_A_KEY: KeyWrap = KeyWrap {
    name: "test",
    wrap: turned,
    unwrap: short,
};

/// Whether `needle` is anywhere in `memory`'s bytes.
fn in_the_clear(memory: &Memory, needle: &[u8]) -> bool {
    let bytes = memory.0.read().unwrap();
    bytes.windows(needle.len()).any(|window| window == needle)
}

/// A profile in `memory` with an item and a cookie, its key kept by
/// `wrap`.
fn written(memory: &Memory, wrap: KeyWrap) {
    let profile = Profile::in_memory_with(memory, wrap).unwrap();
    let mut changes = Changes::new();
    changes.set_item("https://a.test", "token", "secret-item-value", 0);
    changes.cookies(vec![b"secret-cookie-record".to_vec()]);
    changes.hsts(vec![b"secret-hsts-record".to_vec()]);
    profile.commit(&changes).unwrap();
}

fn read(memory: &Memory, wrap: KeyWrap) -> (Vec<Item>, Vec<Vec<u8>>) {
    let profile = Profile::in_memory_with(memory, wrap).unwrap();
    (
        profile.local_storage("https://a.test").unwrap(),
        profile.cookies().unwrap(),
    )
}

#[test]
fn the_values_are_sealed_on_disk_and_open_again() {
    let memory = Memory::new();
    written(&memory, TEST_WRAP);
    for value in [
        &b"secret-item-value"[..],
        b"secret-cookie-record",
        b"secret-hsts-record",
    ] {
        assert!(
            !in_the_clear(&memory, value),
            "{:?}",
            String::from_utf8_lossy(value)
        );
    }
    // The keys of the tables stay readable.
    assert!(in_the_clear(&memory, b"https://a.test"));
    // Each value its own nonce: the same value twice is two sealings.
    let profile = Profile::in_memory_with(&memory, TEST_WRAP).unwrap();
    let mut changes = Changes::new();
    changes.cookies(vec![b"same".to_vec(), b"same".to_vec()]);
    profile.commit(&changes).unwrap();
    drop(profile);
    {
        let db = Database::builder()
            .create_with_backend(memory.clone())
            .unwrap();
        let txn = db.begin_read().unwrap();
        let cookies = txn.open_table(COOKIES).unwrap();
        let sealed = |at: u64| {
            cookies
                .get(at)
                .unwrap()
                .unwrap()
                .value()
                .get(..12)
                .map(<[u8]>::to_vec)
        };
        assert_ne!(sealed(0), sealed(1));
    }
    let profile = Profile::in_memory_with(&memory, TEST_WRAP).unwrap();
    let mut changes = Changes::new();
    changes.cookies(vec![b"secret-cookie-record".to_vec()]);
    profile.commit(&changes).unwrap();
    drop(profile);
    let (items, cookies) = read(&memory, TEST_WRAP);
    assert_eq!(items, [item("token", "secret-item-value", 0)]);
    assert_eq!(cookies, [b"secret-cookie-record".to_vec()]);
    // The key itself is kept wrapped.
    let db = Database::builder()
        .create_with_backend(memory.clone())
        .unwrap();
    let txn = db.begin_read().unwrap();
    let table = txn.open_table(KEY).unwrap();
    assert_eq!(table.get("wrap").unwrap().unwrap().value(), b"test");
    let wrapped = table.get("data").unwrap().unwrap().value().to_vec();
    assert_eq!(wrapped.len(), 32);
    let key = turned(&wrapped).unwrap();
    assert!(!in_the_clear(&memory, &key), "never unwrapped on disk");
}

#[test]
fn a_key_that_does_not_unwrap_leaves_the_values_out() {
    let memory = Memory::new();
    written(&memory, TEST_WRAP);
    let (items, cookies) = read(&memory, OTHER_USER);
    assert!(items.is_empty() && cookies.is_empty());
    written(&memory, TEST_WRAP);
    let (items, cookies) = read(&memory, NOT_A_KEY);
    assert!(items.is_empty() && cookies.is_empty());
    // A new key: what is written now opens again.
    let profile = Profile::in_memory_with(&memory, OTHER_USER).unwrap();
    let mut changes = Changes::new();
    changes.set_item("https://a.test", "again", "v", 1);
    profile.commit(&changes).unwrap();
    drop(profile);
    let profile = Profile::in_memory_with(&memory, TEST_WRAP).unwrap();
    assert_eq!(
        profile.local_storage("https://a.test").unwrap(),
        [item("again", "v", 1)]
    );
}

#[test]
fn a_key_kept_as_it_is_is_wrapped_when_a_wrap_comes() {
    let memory = Memory::new();
    written(&memory, KeyWrap::NONE);
    assert_eq!(read(&memory, TEST_WRAP).0.len(), 1);
    assert_eq!(read(&memory, TEST_WRAP).0.len(), 1);
    // Wrapped now: a profile that cannot unwrap it reads nothing.
    assert!(read(&memory, KeyWrap::NONE).0.is_empty());
}

#[test]
fn a_value_moved_to_another_key_does_not_open() {
    let memory = Memory::new();
    let profile = Profile::in_memory(&memory).unwrap();
    let mut changes = Changes::new();
    changes.set_item("https://a.test", "one", "1", 0);
    changes.set_item("https://a.test", "two", "2", 1);
    changes.cookies(vec![b"first".to_vec(), b"second".to_vec()]);
    profile.commit(&changes).unwrap();
    drop(profile);
    {
        let db = Database::builder()
            .create_with_backend(memory.clone())
            .unwrap();
        let txn = db.begin_write().unwrap();
        {
            let mut table = txn.open_table(LOCAL_STORAGE).unwrap();
            let one = table
                .get(("https://a.test", "one"))
                .unwrap()
                .unwrap()
                .value()
                .1
                .to_vec();
            table
                .insert(("https://a.test", "two"), (1, one.as_slice()))
                .unwrap();
            let mut cookies = txn.open_table(COOKIES).unwrap();
            let first = cookies.get(0).unwrap().unwrap().value().to_vec();
            cookies.insert(1, first.as_slice()).unwrap();
        }
        txn.commit().unwrap();
    }
    let profile = Profile::in_memory(&memory).unwrap();
    assert_eq!(
        profile.local_storage("https://a.test").unwrap(),
        [item("one", "1", 0)]
    );
    assert_eq!(profile.cookies().unwrap(), [b"first".to_vec()]);
}

#[test]
fn a_schema_1_profile_is_sealed_in_place() {
    let memory = Memory::new();
    {
        let db = Database::builder()
            .create_with_backend(memory.clone())
            .unwrap();
        let txn = db.begin_write().unwrap();
        {
            txn.open_table(META).unwrap().insert("schema", 1).unwrap();
            let mut items = txn.open_table(LOCAL_STORAGE_1).unwrap();
            items
                .insert(("https://a.test", "k"), (3, "kept-in-the-clear"))
                .unwrap();
            let mut cookies = txn.open_table(COOKIES).unwrap();
            cookies.insert(0, &b"cookie-in-the-clear"[..]).unwrap();
        }
        txn.commit().unwrap();
    }
    let (items, cookies) = read(&memory, TEST_WRAP);
    assert_eq!(items, [item("k", "kept-in-the-clear", 3)]);
    assert_eq!(cookies, [b"cookie-in-the-clear".to_vec()]);
    let db = Database::builder()
        .create_with_backend(memory.clone())
        .unwrap();
    let txn = db.begin_read().unwrap();
    let schema = txn
        .open_table(META)
        .unwrap()
        .get("schema")
        .unwrap()
        .unwrap()
        .value();
    assert_eq!(schema, SCHEMA);
    let sealed = txn
        .open_table(COOKIES)
        .unwrap()
        .get(0)
        .unwrap()
        .unwrap()
        .value()
        .to_vec();
    assert_ne!(sealed, b"cookie-in-the-clear");
    assert!(
        txn.open_table(LOCAL_STORAGE_1).is_err(),
        "the old table gone"
    );
}

#[test]
fn indexed_dbs_databases_and_records_are_kept_sealed_and_dropped() {
    let memory = Memory::new();
    let a = "https://a.test";
    {
        let profile = Profile::in_memory_with(&memory, TEST_WRAP).unwrap();
        let mut changes = Changes::new();
        changes.idb_database(a, "db", Some(b"secret-database-meta".to_vec()));
        changes.idb_record(
            (a, "db", "s1"),
            &[1, 0, 2],
            Some(b"secret-record-one".to_vec()),
        );
        changes.idb_record((a, "db", "s1"), &[1, 0, 3], Some(b"two".to_vec()));
        changes.idb_record((a, "db", "s2"), &[9], Some(b"three".to_vec()));
        changes.idb_record((a, "other", "s1"), &[1], Some(b"four".to_vec()));
        changes.idb_database("https://b.test", "db", Some(b"b".to_vec()));
        changes.idb_record(("https://b.test", "db", "s1"), &[1], Some(b"b's".to_vec()));
        profile.commit(&changes).unwrap();
    }
    for value in [&b"secret-database-meta"[..], b"secret-record-one"] {
        assert!(
            !in_the_clear(&memory, value),
            "{}",
            String::from_utf8_lossy(value)
        );
    }
    let profile = Profile::in_memory_with(&memory, TEST_WRAP).unwrap();
    let (databases, records) = profile.idb(a).unwrap();
    assert_eq!(
        databases,
        [("db".to_owned(), b"secret-database-meta".to_vec())]
    );
    assert_eq!(records.len(), 4);
    assert_eq!(
        records[0],
        (
            "db".to_owned(),
            "s1".to_owned(),
            vec![1, 0, 2],
            b"secret-record-one".to_vec()
        )
    );
    // A record deleted, a store's records dropped, a database dropped.
    let mut changes = Changes::new();
    changes.idb_record((a, "db", "s1"), &[1, 0, 3], None);
    changes.idb_store_deleted(a, "db", "s2");
    changes.idb_database(a, "other", None);
    profile.commit(&changes).unwrap();
    let (_, records) = profile.idb(a).unwrap();
    let left: Vec<(&str, &str, &[u8])> = records
        .iter()
        .map(|(d, s, k, _)| (d.as_str(), s.as_str(), k.as_slice()))
        .collect();
    assert_eq!(left, [("db", "s1", &[1u8, 0, 2][..])]);
    // Another storage key's are its own, and stay.
    let (databases, records) = profile.idb("https://b.test").unwrap();
    assert_eq!((databases.len(), records.len()), (1, 1));
    // A record sealed for one place does not open at another.
    drop(profile);
    {
        let db = Database::builder()
            .create_with_backend(memory.clone())
            .unwrap();
        let txn = db.begin_write().unwrap();
        {
            let mut table = txn.open_table(IDB_RECORDS).unwrap();
            let sealed = table
                .get((a, "db", "s1", &[1u8, 0, 2][..]))
                .unwrap()
                .unwrap()
                .value()
                .to_vec();
            table
                .insert((a, "db", "s1", &[7u8][..]), sealed.as_slice())
                .unwrap();
        }
        txn.commit().unwrap();
    }
    let profile = Profile::in_memory_with(&memory, TEST_WRAP).unwrap();
    let (_, records) = profile.idb(a).unwrap();
    assert_eq!(records.len(), 1, "the moved record does not open");
}

#[test]
fn the_settings_are_kept_sealed_and_replaced_whole() {
    // M37: schema 4's table.
    let memory = Memory::new();
    let profile = Profile::in_memory_with(&memory, TEST_WRAP).unwrap();
    assert!(profile.settings().unwrap().is_empty());
    let mut changes = Changes::new();
    changes.settings(vec![b"secret-settings-record".to_vec()]);
    profile.commit(&changes).unwrap();
    drop(profile);
    assert!(!in_the_clear(&memory, b"secret-settings-record"));
    let profile = Profile::in_memory_with(&memory, TEST_WRAP).unwrap();
    assert_eq!(
        profile.settings().unwrap(),
        [b"secret-settings-record".to_vec()]
    );
    let mut changes = Changes::new();
    changes.settings(vec![b"next".to_vec()]);
    profile.commit(&changes).unwrap();
    assert_eq!(profile.settings().unwrap(), [b"next".to_vec()]);
}

#[test]
fn the_http_caches_entries_and_bodies_are_kept_sealed_and_deleted() {
    // M38: schema 5's tables.
    let memory = Memory::new();
    let profile = Profile::in_memory_with(&memory, TEST_WRAP).unwrap();
    assert!(profile.cache_entries().unwrap().is_empty());
    assert_eq!(profile.cache_body(1).unwrap(), None);
    let mut changes = Changes::new();
    changes.cache_put(
        1,
        b"secret-entry-one".to_vec(),
        Some(b"secret-body-one".to_vec()),
    );
    changes.cache_put(2, b"secret-entry-two".to_vec(), Some(b"body-two".to_vec()));
    assert!(!changes.is_empty());
    profile.commit(&changes).unwrap();
    drop(profile);
    assert!(!in_the_clear(&memory, b"secret-entry-one"));
    assert!(!in_the_clear(&memory, b"secret-body-one"));
    let profile = Profile::in_memory_with(&memory, TEST_WRAP).unwrap();
    assert_eq!(
        profile.cache_entries().unwrap(),
        [
            (1, b"secret-entry-one".to_vec()),
            (2, b"secret-entry-two".to_vec())
        ]
    );
    assert_eq!(
        profile.cache_body(1).unwrap(),
        Some(b"secret-body-one".to_vec())
    );
    // A record changed with its body kept, then one deleted.
    let mut changes = Changes::new();
    changes.cache_put(1, b"entry-one-again".to_vec(), None);
    changes.cache_delete(2);
    profile.commit(&changes).unwrap();
    assert_eq!(
        profile.cache_entries().unwrap(),
        [(1, b"entry-one-again".to_vec())]
    );
    assert_eq!(
        profile.cache_body(1).unwrap(),
        Some(b"secret-body-one".to_vec())
    );
    assert_eq!(profile.cache_body(2).unwrap(), None);
    drop(profile);
    // A body moved to another number does not open.
    {
        let db = Database::builder()
            .create_with_backend(memory.clone())
            .unwrap();
        let txn = db.begin_write().unwrap();
        {
            let mut bodies = txn.open_table(HTTP_CACHE_BODIES).unwrap();
            let sealed = bodies.get(1).unwrap().unwrap().value().to_vec();
            bodies.insert(3, sealed.as_slice()).unwrap();
        }
        txn.commit().unwrap();
    }
    let profile = Profile::in_memory_with(&memory, TEST_WRAP).unwrap();
    assert_eq!(profile.cache_body(3).unwrap(), None);
    assert!(profile.cache_body(1).unwrap().is_some());
}

#[test]
fn the_cache_apis_caches_entries_and_bodies_are_kept_sealed_by_storage_key() {
    // M39: schema 6's tables.
    let memory = Memory::new();
    let profile = Profile::in_memory_with(&memory, TEST_WRAP).unwrap();
    let key = "https://a.test";
    assert_eq!(profile.cache_api(key).unwrap(), (Vec::new(), Vec::new()));
    let mut changes = Changes::new();
    changes.cache_api_cache(key, 1, Some(b"secret-cache-one".to_vec()));
    changes.cache_api_cache(key, 2, Some(b"cache-two".to_vec()));
    changes.cache_api_cache("https://b.test", 1, Some(b"other key".to_vec()));
    changes.cache_api_entry(
        key,
        7,
        Some(b"secret-entry".to_vec()),
        Some(b"secret-body".to_vec()),
    );
    changes.cache_api_entry(key, 8, Some(b"entry-eight".to_vec()), Some(Vec::new()));
    assert!(!changes.is_empty());
    profile.commit(&changes).unwrap();
    drop(profile);
    for secret in [&b"secret-cache-one"[..], b"secret-entry", b"secret-body"] {
        assert!(!in_the_clear(&memory, secret));
    }
    let profile = Profile::in_memory_with(&memory, TEST_WRAP).unwrap();
    let (caches, entries) = profile.cache_api(key).unwrap();
    assert_eq!(
        caches,
        [
            (1, b"secret-cache-one".to_vec()),
            (2, b"cache-two".to_vec())
        ]
    );
    assert_eq!(
        entries,
        [(7, b"secret-entry".to_vec()), (8, b"entry-eight".to_vec())]
    );
    assert_eq!(
        profile.cache_api_body(key, 7).unwrap(),
        Some(b"secret-body".to_vec())
    );
    assert_eq!(profile.cache_api_body(key, 8).unwrap(), Some(Vec::new()));
    assert_eq!(profile.cache_api_body("https://b.test", 7).unwrap(), None);
    assert!(
        profile
            .storage_keys()
            .unwrap()
            .contains(&"https://b.test".to_owned())
    );
    // A record changed with its body kept; an entry and a cache gone.
    let mut changes = Changes::new();
    changes.cache_api_entry(key, 7, Some(b"entry-again".to_vec()), None);
    changes.cache_api_entry(key, 8, None, None);
    changes.cache_api_cache(key, 2, None);
    profile.commit(&changes).unwrap();
    let (caches, entries) = profile.cache_api(key).unwrap();
    assert_eq!(caches.len(), 1);
    assert_eq!(entries, [(7, b"entry-again".to_vec())]);
    assert_eq!(
        profile.cache_api_body(key, 7).unwrap(),
        Some(b"secret-body".to_vec())
    );
    assert_eq!(profile.cache_api_body(key, 8).unwrap(), None);
    drop(profile);
    // A body moved to another key does not open.
    {
        let db = Database::builder()
            .create_with_backend(memory.clone())
            .unwrap();
        let txn = db.begin_write().unwrap();
        {
            let mut bodies = txn.open_table(CACHE_API_BODIES).unwrap();
            let sealed = bodies.get((key, 7)).unwrap().unwrap().value().to_vec();
            bodies
                .insert(("https://b.test", 7), sealed.as_slice())
                .unwrap();
        }
        txn.commit().unwrap();
    }
    let profile = Profile::in_memory_with(&memory, TEST_WRAP).unwrap();
    assert_eq!(profile.cache_api_body("https://b.test", 7).unwrap(), None);
}
