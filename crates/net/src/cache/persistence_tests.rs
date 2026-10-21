//! The cache in a profile (M38): entries written as changes, read back
//! by another cache, their bodies at first use; the bounds of memory
//! and of the profile; and what is damaged dropped. The profile here is
//! a map the changes are applied to, as the profile crate's tables are.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::*;

fn t0() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

fn url(input: &str) -> Url {
    Url::parse(input).expect(input)
}

fn site(input: &str) -> Partition {
    url(input).origin().site().into()
}

fn stored(pairs: &[(&str, &str)], body: &[u8]) -> Stored {
    let mut headers = HeaderList::new();
    for (name, value) in pairs {
        headers.append(name.as_bytes(), value.as_bytes());
    }
    Stored {
        status: 200,
        status_message: b"OK".to_vec(),
        headers,
        body: body.to_vec(),
        address_space: Some(AddressSpace::Public),
    }
}

const LASTING: &[(&str, &str)] = &[("Cache-Control", "max-age=600"), ("ETag", "\"e\"")];

/// The profile's two tables, and how many bodies were read from it.
#[derive(Clone, Default)]
struct Disk {
    records: Arc<Mutex<BTreeMap<u64, Vec<u8>>>>,
    bodies: Arc<Mutex<BTreeMap<u64, Vec<u8>>>>,
    reads: Arc<AtomicUsize>,
}

impl Disk {
    fn apply(&self, changes: Vec<Change>) {
        let mut records = self.records.lock().unwrap();
        let mut bodies = self.bodies.lock().unwrap();
        for change in changes {
            match change {
                Change::Put { id, record, body } => {
                    records.insert(id, record);
                    if let Some(body) = body {
                        bodies.insert(id, body);
                    }
                }
                Change::Delete { id } => {
                    records.remove(&id);
                    bodies.remove(&id);
                }
            }
        }
    }

    fn flush(&self, cache: &Cache) {
        self.apply(cache.take_changes());
    }

    fn reader(&self) -> BodyReader {
        let disk = self.clone();
        Arc::new(move |id| {
            disk.reads.fetch_add(1, Ordering::SeqCst);
            disk.bodies.lock().unwrap().get(&id).cloned()
        })
    }

    /// A cache given this profile, as a browser started again is.
    fn cache(&self) -> Cache {
        let cache = Cache::new();
        let records = self.records.lock().unwrap().clone().into_iter().collect();
        cache.load(records, self.reader());
        cache
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

fn put(cache: &Cache, at: &str, path: &str, pairs: &[(&str, &str)], body: &[u8]) {
    cache.store(
        &site(at),
        &url(&format!("{at}{path}")),
        &HeaderList::new(),
        stored(pairs, body),
        t0(),
        t0(),
    );
}

fn get(cache: &Cache, at: &str, path: &str) -> Option<Hit> {
    cache.lookup(
        &site(at),
        &url(&format!("{at}{path}")),
        &HeaderList::new(),
        t0() + Duration::from_secs(5),
    )
}

#[test]
fn a_cache_given_the_profile_again_has_its_entries_their_bodies_read_at_first_use() {
    let disk = Disk::default();
    let cache = disk.cache();
    put(&cache, "https://a.test/", "one", LASTING, b"first body");
    put(&cache, "https://a.test/", "two", LASTING, b"second body");
    assert!(cache.has_changes());
    disk.flush(&cache);
    assert!(!cache.has_changes());
    assert_eq!(disk.records.lock().unwrap().len(), 2);
    drop(cache);
    let again = disk.cache();
    assert_eq!(again.size(), (2, 0), "no body read yet");
    assert_eq!(again.profile_bytes(), 21);
    assert_eq!(disk.reads(), 0);
    let hit = get(&again, "https://a.test/", "one").expect("kept");
    assert!(hit.fresh);
    assert_eq!(hit.stored.body, b"first body");
    assert_eq!(hit.stored.status_message, b"OK");
    assert_eq!(hit.stored.address_space, Some(AddressSpace::Public));
    assert_eq!(hit.etag, Some(b"\"e\"".to_vec()));
    assert_eq!(hit.stored.headers.get(b"age"), Some(b"5".to_vec()));
    assert_eq!(disk.reads(), 1);
    get(&again, "https://a.test/", "one").expect("kept");
    assert_eq!(disk.reads(), 1, "read once");
    assert_eq!(again.size(), (2, 10));
    // Another site's partition is apart, there too.
    assert!(get(&again, "https://b.test/", "one").is_none());
}

#[test]
fn what_changes_is_written_and_what_goes_is_deleted() {
    let disk = Disk::default();
    let cache = disk.cache();
    put(&cache, "https://a.test/", "r", LASTING, b"old");
    disk.flush(&cache);
    // A use writes nothing.
    get(&cache, "https://a.test/", "r").unwrap();
    assert!(!cache.has_changes());
    // Stored again: the old one deleted, the new one written whole.
    put(&cache, "https://a.test/", "r", LASTING, b"new");
    let changes = cache.take_changes();
    assert!(matches!(changes[0], Change::Delete { .. }), "{changes:?}");
    assert!(matches!(&changes[1], Change::Put { body: Some(body), .. } if body == b"new"));
    disk.apply(changes);
    // A 304 freshens the record alone.
    let freshened = cache.freshen(
        &site("https://a.test/"),
        &url("https://a.test/r"),
        &HeaderList::new(),
        &HeaderList::new(),
        t0(),
        t0(),
    );
    assert!(freshened.is_some());
    assert!(matches!(
        cache.take_changes()[..],
        [Change::Put { body: None, .. }]
    ));
    // Invalidated: deleted.
    cache.invalidate(&site("https://a.test/"), &url("https://a.test/r"));
    disk.flush(&cache);
    assert!(disk.records.lock().unwrap().is_empty());
    assert!(disk.bodies.lock().unwrap().is_empty());
    assert!(disk.cache().size() == (0, 0));
}

#[test]
fn what_was_stored_before_the_profile_came_is_written_to_it() {
    let disk = Disk::default();
    put(&disk.cache(), "https://a.test/", "kept", LASTING, b"x");
    disk.flush(&{
        let cache = disk.cache();
        put(&cache, "https://a.test/", "kept", LASTING, b"x");
        cache
    });
    // A cache that stored without a profile, then given one.
    let cache = Cache::new();
    put(&cache, "https://a.test/", "early", LASTING, b"early");
    put(&cache, "https://a.test/", "kept", LASTING, b"newer");
    assert!(!cache.has_changes(), "no profile, nothing to write");
    let records = disk.records.lock().unwrap().clone().into_iter().collect();
    cache.load(records, disk.reader());
    assert_eq!(cache.size().0, 2, "the profile's same variant gives way");
    assert_eq!(
        get(&cache, "https://a.test/", "kept").unwrap().stored.body,
        b"newer"
    );
    disk.flush(&cache);
    let again = disk.cache();
    assert_eq!(
        get(&again, "https://a.test/", "early").unwrap().stored.body,
        b"early"
    );
    assert_eq!(
        get(&again, "https://a.test/", "kept").unwrap().stored.body,
        b"newer"
    );
    assert_eq!(disk.records.lock().unwrap().len(), 2);
}

#[test]
fn an_opaque_sites_entries_and_those_of_no_later_use_stay_in_memory() {
    let disk = Disk::default();
    let cache = disk.cache();
    let opaque = Partition::from(Site::Opaque(7));
    cache.store(
        &opaque,
        &url("https://a.test/r"),
        &HeaderList::new(),
        stored(LASTING, b"x"),
        t0(),
        t0(),
    );
    // Stale from the first, with no validator: a traversal's alone.
    put(
        &cache,
        "https://a.test/",
        "now",
        &[("Content-Type", "text/html")],
        b"y",
    );
    assert!(!cache.has_changes());
    assert_eq!(cache.size(), (2, 2));
    assert_eq!(cache.profile_bytes(), 0);
    // With a validator, or a lifetime, it is kept.
    put(
        &cache,
        "https://a.test/",
        "etag",
        &[("ETag", "\"v\"")],
        b"z",
    );
    put(
        &cache,
        "https://a.test/",
        "aged",
        &[("Expires", "Thu, 01 Jan 2099 00:00:00 GMT")],
        b"z",
    );
    assert_eq!(cache.take_changes().len(), 2);
}

#[test]
fn bodies_past_memorys_bound_are_read_again_and_the_profiles_bound_drops_entries() {
    let disk = Disk::default();
    let cache = disk.cache();
    let big = vec![b'x'; MAX_CACHE_ENTRY_BYTES];
    let in_memory = MAX_CACHE_BYTES / MAX_CACHE_ENTRY_BYTES;
    // A body the profile has not had yet stays in memory; once it has,
    // the least used leave.
    for i in 0..=in_memory {
        put(&cache, "https://a.test/", &i.to_string(), LASTING, &big);
    }
    assert_eq!(cache.size().1, MAX_CACHE_BYTES + MAX_CACHE_ENTRY_BYTES);
    disk.flush(&cache);
    assert_eq!(cache.size(), (in_memory + 1, MAX_CACHE_BYTES));
    assert_eq!(disk.reads(), 0);
    // The first left memory; read again from the profile.
    assert_eq!(
        get(&cache, "https://a.test/", "0").unwrap().stored.body,
        big
    );
    assert_eq!(disk.reads(), 1);
    assert!(cache.size().1 <= MAX_CACHE_BYTES);
    // The profile's bound: the least used entries deleted.
    let in_profile = MAX_PROFILE_CACHE_BYTES / MAX_CACHE_ENTRY_BYTES;
    for i in in_memory + 1..=in_profile {
        put(&cache, "https://a.test/", &i.to_string(), LASTING, &big);
        disk.flush(&cache);
    }
    assert_eq!(cache.size().0, in_profile);
    assert_eq!(cache.profile_bytes(), MAX_PROFILE_CACHE_BYTES);
    assert_eq!(disk.records.lock().unwrap().len(), in_profile);
    assert!(
        get(&cache, "https://a.test/", "1").is_none(),
        "least used went"
    );
    assert!(get(&cache, "https://a.test/", "0").is_some());
}

#[test]
fn a_damaged_record_or_body_is_dropped() {
    let disk = Disk::default();
    let cache = disk.cache();
    for path in ["a", "b", "c"] {
        put(&cache, "https://a.test/", path, LASTING, path.as_bytes());
    }
    disk.flush(&cache);
    let ids: Vec<u64> = disk.records.lock().unwrap().keys().copied().collect();
    disk.records
        .lock()
        .unwrap()
        .get_mut(&ids[0])
        .unwrap()
        .push(0);
    disk.bodies
        .lock()
        .unwrap()
        .insert(ids[1], b"too long".to_vec());
    let again = disk.cache();
    assert_eq!(again.size().0, 2, "the damaged record left out");
    assert!(get(&again, "https://a.test/", "a").is_none());
    assert!(
        get(&again, "https://a.test/", "b").is_none(),
        "its body not its length"
    );
    assert_eq!(
        get(&again, "https://a.test/", "c").unwrap().stored.body,
        b"c"
    );
    disk.flush(&again);
    assert_eq!(disk.records.lock().unwrap().len(), 1, "both deleted");
}

#[test]
fn a_record_reads_back_as_it_was_and_no_part_of_one_reads() {
    let cache = Cache::new();
    let mut request = HeaderList::new();
    request.append(b"Accept-Language", b"fr");
    let mut pairs = LASTING.to_vec();
    pairs.extend([
        ("Vary", "accept-language, x-none"),
        ("Cache-Control", "immutable"),
    ]);
    cache.store(
        &Partition {
            top: url("https://a.test/").origin().site(),
            frame: url("https://b.test/").origin().site(),
        },
        &url("https://a.test/r#f"),
        &request,
        stored(&pairs, b"body"),
        t0(),
        t0() + Duration::new(1, 5),
    );
    let inner = cache.lock();
    let (key, variants) = inner.entries.iter().next().unwrap();
    let entry = &variants[0];
    let bytes = record::encode(key, entry).unwrap();
    let back = record::decode(entry.id, &bytes).unwrap();
    assert_eq!(&back.key, key);
    assert_eq!(back.entry.vary, entry.vary);
    assert_eq!(back.entry.stored.headers, entry.stored.headers);
    assert_eq!(back.entry.response_time, entry.response_time);
    assert_eq!(
        (back.entry.lifetime, back.entry.initial_age, back.entry.used),
        (entry.lifetime, entry.initial_age, entry.used)
    );
    assert!(back.entry.immutable && !back.entry.no_cache);
    assert_eq!(back.entry.body_len, 4);
    for end in 0..bytes.len() {
        assert!(record::decode(1, &bytes[..end]).is_none(), "{end}");
    }
    let mut newer = bytes.clone();
    newer[0] = 2;
    assert!(record::decode(1, &newer).is_none());
}

#[test]
fn clearing_a_site_drops_its_partitions() {
    let disk = Disk::default();
    let cache = disk.cache();
    put(&cache, "https://a.test/", "r", LASTING, b"a");
    put(&cache, "https://b.test/", "r", LASTING, b"b");
    cache.store(
        &Partition {
            top: url("https://b.test/").origin().site(),
            frame: url("https://a.test/").origin().site(),
        },
        &url("https://a.test/f"),
        &HeaderList::new(),
        stored(LASTING, b"f"),
        t0(),
        t0(),
    );
    disk.flush(&cache);
    cache.clear_site(&url("https://a.test/").origin().site());
    assert_eq!(cache.size().0, 1);
    assert!(get(&cache, "https://b.test/", "r").is_some());
    disk.flush(&cache);
    assert_eq!(disk.records.lock().unwrap().len(), 1);
}
