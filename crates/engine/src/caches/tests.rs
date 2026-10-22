//! The Cache API's store (M39): the reference browser's answers on the
//! store's own terms (scratchpad `m39/pages/p3`), its quota, and what it
//! keeps in the profile.

use super::*;

const KEY: &str = "https://a.test";

fn get(url: &str) -> StoredRequest {
    StoredRequest {
        method: "GET".to_owned(),
        url: url.to_owned(),
        headers: Vec::new(),
    }
}

fn with(url: &str, name: &str, value: &str) -> StoredRequest {
    StoredRequest {
        headers: vec![(name.to_owned(), value.to_owned())],
        ..get(url)
    }
}

fn made(n: u32, headers: &[(&str, &str)]) -> StoredResponse {
    let mut fields = vec![("x-n".to_owned(), n.to_string())];
    fields.extend(
        headers
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned())),
    );
    StoredResponse {
        kind: "default".to_owned(),
        urls: Vec::new(),
        status: 200,
        status_text: String::new(),
        headers: fields,
    }
}

fn put(request: StoredRequest, response: StoredResponse, body: &[u8]) -> Operation {
    Operation::Put {
        request,
        response,
        body: body.to_vec(),
    }
}

fn none(_: u64) -> Option<Vec<u8>> {
    None
}

/// The `x-n` of the responses matching `query` in `cache`.
fn ns(
    store: &Store,
    cache: u64,
    query: Option<&StoredRequest>,
    options: QueryOptions,
) -> Vec<String> {
    store
        .match_all(KEY, cache, query, options, &none)
        .unwrap()
        .iter()
        .map(|m| header(&m.response.headers, "x-n").unwrap())
        .collect()
}

#[test]
fn caches_keep_the_order_they_were_made_and_a_name_its_cache() {
    let mut store = Store::default();
    let a = store.open(KEY, "a").unwrap();
    store.open(KEY, "z").unwrap();
    store.open(KEY, "b").unwrap();
    assert_eq!(store.open(KEY, "a").unwrap(), a);
    assert_eq!(store.names(KEY), ["a", "z", "b"]);
    assert!(store.number(KEY, "z").is_some() && store.number(KEY, "nope").is_none());
    assert!(store.delete(KEY, "z"));
    assert!(!store.delete(KEY, "z"));
    assert_eq!(store.names(KEY), ["a", "b"]);
    assert!(store.names("https://b.test").is_empty(), "keys apart");
}

#[test]
fn a_query_matches_as_the_standard_says() {
    let mut store = Store::default();
    let c = store.open(KEY, "c").unwrap();
    store
        .batch(
            KEY,
            c,
            vec![
                put(get("https://a.test/one"), made(1, &[]), b"body 1"),
                put(get("https://a.test/two?x=1"), made(2, &[]), b"b2"),
                put(
                    with("https://a.test/three", "x-v", "a"),
                    made(3, &[("vary", "X-V")]),
                    b"",
                ),
                put(get("https://a.test/frag#x"), made(4, &[]), b""),
            ],
        )
        .unwrap();
    let plain = QueryOptions::default();
    assert_eq!(
        ns(&store, c, Some(&get("https://a.test/one")), plain),
        ["1"]
    );
    assert!(ns(&store, c, Some(&get("https://a.test/two")), plain).is_empty());
    let search = QueryOptions {
        ignore_search: true,
        ..plain
    };
    assert_eq!(
        ns(&store, c, Some(&get("https://a.test/two")), search),
        ["2"]
    );
    let post = StoredRequest {
        method: "POST".to_owned(),
        ..get("https://a.test/one")
    };
    assert!(ns(&store, c, Some(&post), plain).is_empty());
    let method = QueryOptions {
        ignore_method: true,
        ..plain
    };
    assert_eq!(ns(&store, c, Some(&post), method), ["1"]);
    // Vary: the field it names must be the same, its name's case aside.
    assert!(
        ns(
            &store,
            c,
            Some(&with("https://a.test/three", "x-v", "b")),
            plain
        )
        .is_empty()
    );
    assert_eq!(
        ns(
            &store,
            c,
            Some(&with("https://a.test/three", "X-V", "a")),
            plain
        ),
        ["3"]
    );
    let vary = QueryOptions {
        ignore_vary: true,
        ..plain
    };
    assert_eq!(
        ns(&store, c, Some(&get("https://a.test/three")), vary),
        ["3"]
    );
    // A fragment is kept in the key and ignored by a query.
    assert_eq!(
        ns(&store, c, Some(&get("https://a.test/frag#y")), plain),
        ["4"]
    );
    assert_eq!(
        store.requests(KEY, c, None, plain).unwrap()[3].url,
        "https://a.test/frag#x"
    );
    // The body comes back whole.
    let found = store
        .match_all(KEY, c, Some(&get("https://a.test/one")), plain, &none)
        .unwrap();
    assert_eq!(&*found[0].body, b"body 1");
}

#[test]
fn a_put_replaces_its_match_at_the_end_and_a_delete_says_what_it_did() {
    let mut store = Store::default();
    let c = store.open(KEY, "c").unwrap();
    for (n, url) in [(1, "one"), (2, "two"), (3, "three")] {
        let url = format!("https://a.test/{url}");
        store
            .batch(KEY, c, vec![put(get(&url), made(n, &[]), b"")])
            .unwrap();
    }
    store
        .batch(
            KEY,
            c,
            vec![put(get("https://a.test/one"), made(11, &[]), b"")],
        )
        .unwrap();
    assert_eq!(
        ns(&store, c, None, QueryOptions::default()),
        ["2", "3", "11"]
    );
    let deleted = store
        .batch(
            KEY,
            c,
            vec![
                Operation::Delete {
                    request: get("https://a.test/two"),
                    options: QueryOptions::default(),
                },
                Operation::Delete {
                    request: get("https://a.test/two"),
                    options: QueryOptions::default(),
                },
            ],
        )
        .unwrap();
    assert_eq!(deleted, [true, false]);
    // caches.match: the caches in order, or the one named.
    let d = store.open(KEY, "d").unwrap();
    store
        .batch(
            KEY,
            d,
            vec![put(get("https://a.test/one"), made(99, &[]), b"")],
        )
        .unwrap();
    let any = |name| {
        store
            .match_any(
                KEY,
                name,
                &get("https://a.test/one"),
                QueryOptions::default(),
                &none,
            )
            .map(|m| header(&m.response.headers, "x-n").unwrap())
    };
    assert_eq!(any(None).as_deref(), Some("11"));
    assert_eq!(any(Some("d")).as_deref(), Some("99"));
    assert_eq!(any(Some("nope")), None);
}

#[test]
fn a_deleted_caches_object_still_reads_what_it_held() {
    let mut store = Store::default();
    let c = store.open(KEY, "c").unwrap();
    store
        .batch(
            KEY,
            c,
            vec![put(get("https://a.test/one"), made(4, &[]), b"")],
        )
        .unwrap();
    store.delete(KEY, "c");
    assert_eq!(ns(&store, c, None, QueryOptions::default()), ["4"]);
    assert_eq!(store.usage(KEY), 0, "not counted");
    store.clear(KEY);
    assert!(
        store
            .match_all(KEY, c, None, QueryOptions::default(), &none)
            .is_err()
    );
}

#[test]
fn a_batch_past_the_quota_changes_nothing_and_opaque_responses_are_padded() {
    let mut store = Store::default();
    let c = store.open(KEY, "c").unwrap();
    let opaque = StoredResponse {
        kind: "opaque".to_owned(),
        status: 0,
        ..made(1, &[])
    };
    store
        .batch(KEY, c, vec![put(get("https://b.test/x"), opaque, b"tiny")])
        .unwrap();
    assert!(store.usage(KEY) >= OPAQUE_PADDING);
    let big = vec![0u8; (CACHE_QUOTA - store.usage(KEY)) as usize];
    let error = store
        .batch(
            KEY,
            c,
            vec![
                put(get("https://a.test/small"), made(2, &[]), b"s"),
                put(get("https://a.test/big"), made(3, &[]), &big),
            ],
        )
        .unwrap_err();
    assert_eq!(error.name, "QuotaExceededError");
    assert_eq!(
        ns(&store, c, None, QueryOptions::default()),
        ["1"],
        "all or nothing"
    );
}

#[test]
fn what_changes_is_written_and_read_back_by_another_store() {
    let mut store = Store::default();
    store.set_writing(true);
    let c = store.open(KEY, "c").unwrap();
    store
        .batch(
            KEY,
            c,
            vec![
                put(get("https://a.test/one"), made(1, &[]), b"first"),
                put(
                    with("https://a.test/two", "x-v", "a"),
                    made(2, &[("vary", "x-v")]),
                    b"second",
                ),
            ],
        )
        .unwrap();
    let written = store.take_written();
    assert!(!store.has_written());
    // A profile, as the profile crate keeps it: records and bodies.
    let mut caches = Vec::new();
    let mut entries = Vec::new();
    let mut bodies = HashMap::new();
    for change in written {
        match change {
            Written::Cache {
                id,
                record: Some(record),
                ..
            } => caches.push((id, record)),
            Written::Entry {
                id,
                record: Some(record),
                body,
                ..
            } => {
                entries.push((id, record));
                if let Some(body) = body {
                    bodies.insert(id, body);
                }
            }
            _ => {}
        }
    }
    // Written bodies leave memory: read from the profile.
    let body_of = |id: u64| bodies.get(&id).cloned();
    let found = store
        .match_all(
            KEY,
            c,
            Some(&get("https://a.test/one")),
            QueryOptions::default(),
            &body_of,
        )
        .unwrap();
    assert_eq!(&*found[0].body, b"first");
    let mut again = Store::default();
    again.load(KEY, caches, entries);
    assert_eq!(again.names(KEY), ["c"]);
    let c = again.number(KEY, "c").unwrap();
    let found = again
        .match_all(
            KEY,
            c,
            Some(&with("https://a.test/two", "x-v", "a")),
            QueryOptions::default(),
            &body_of,
        )
        .unwrap();
    assert_eq!(&*found[0].body, b"second");
    assert_eq!(again.usage(KEY), store.usage(KEY));
    // New numbers follow the profile's.
    let d = again.open(KEY, "d").unwrap();
    assert!(d > c);
}

#[test]
fn a_record_that_does_not_read_is_left_out_and_deleted() {
    let mut store = Store::default();
    store.set_writing(true);
    store.load(
        KEY,
        vec![(1, record::encode_cache("ok")), (2, vec![9, 9])],
        vec![
            (
                5,
                record::encode_entry(1, &get("https://a.test/x"), &made(1, &[]), 0),
            ),
            (
                6,
                record::encode_entry(7, &get("https://a.test/y"), &made(2, &[]), 0),
            ),
            (7, vec![1, 2, 3]),
        ],
    );
    assert_eq!(store.names(KEY), ["ok"]);
    let deleted: Vec<u64> = store
        .take_written()
        .iter()
        .map(|w| match w {
            Written::Cache {
                id, record: None, ..
            }
            | Written::Entry {
                id, record: None, ..
            } => *id,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        deleted,
        [2, 6, 7],
        "a bad cache, an orphan entry, a bad entry"
    );
}

#[test]
fn every_part_of_a_record_is_needed() {
    let entry = record::encode_entry(
        3,
        &with("https://a.test/x", "accept", "*/*"),
        &made(1, &[("vary", "accept")]),
        12,
    );
    record::fuzz_entry_record(&entry);
    for end in 0..entry.len() {
        assert!(record::decode_entry(&entry[..end]).is_none(), "{end}");
    }
    let mut longer = entry.clone();
    longer.push(0);
    assert!(record::decode_entry(&longer).is_none());
}

#[test]
fn the_bindings_requests_are_answered_with_their_bodies() {
    let mut store = Store::default();
    let (answer, _) = serve(&mut store, KEY, r#"{"op":"open","name":"c"}"#, &[], &none);
    assert_eq!(answer, r#"{"result":0}"#);
    let put = r#"{"op":"batch","cache":0,"ops":[{"put":{"request":{"method":"GET","url":"https://a.test/r","headers":[]},"response":{"type":"basic","urls":["https://a.test/r"],"status":200,"statusText":"OK","headers":[["x-n","1"]],"body":[3,4]}}}]}"#;
    let (answer, _) = serve(&mut store, KEY, put, b"...body", &none);
    assert_eq!(answer, r#"{"result":[]}"#);
    let (answer, bytes) = serve(
        &mut store,
        KEY,
        r#"{"op":"match","cache":0,"request":{"method":"GET","url":"https://a.test/r","headers":[]},"options":{},"all":false}"#,
        &[],
        &none,
    );
    assert!(answer.contains(r#""body":[0,4]"#), "{answer}");
    assert_eq!(bytes, b"body");
    let (answer, _) = serve(&mut store, KEY, r#"{"op":"what"}"#, &[], &none);
    assert!(answer.contains(r#""name":"TypeError""#), "{answer}");
    let (answer, _) = serve(
        &mut store,
        KEY,
        r#"{"op":"batch","cache":0,"ops":[{"put":{"request":{"method":"GET","url":"u","headers":[]},"response":{"type":"basic","urls":[],"status":200,"statusText":"","headers":[],"body":[0,99]}}}]}"#,
        b"short",
        &none,
    );
    assert!(
        answer.contains("TypeError"),
        "a body past the bytes: {answer}"
    );
}

#[test]
fn the_fuzz_targets_driver_holds_on_many_sequences() {
    // M39: the `cache_api` target's model, on inputs made here.
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    for _ in 0..3000 {
        let mut data = Vec::new();
        for _ in 0..(state % 300) as usize {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            data.push((state >> 24) as u8);
        }
        state = state.wrapping_add(1);
        fuzz_store(&data);
    }
}
