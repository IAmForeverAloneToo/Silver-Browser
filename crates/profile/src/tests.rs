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
