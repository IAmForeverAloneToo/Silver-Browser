//! IndexedDB as a page's scripts use it (M36.7): a database opened and
//! upgraded, records put and read by key, by range, by index and by
//! cursor, a transaction aborted, its events in their order, and what a
//! page kept still there when it is loaded again with the profile.

use net::test_server::{Reply, TestServer};

use crate::testing::*;

/// A server of `script`, run in a page of its own, at `/`.
fn page(script: &'static str) -> TestServer {
    TestServer::plain(move |_| {
        let body = format!("<!doctype html><script>{script}</script>");
        Reply::ok("text/html", body.as_bytes())
    })
}

/// The console's lines of a tab with scripts on that loads `server`'s
/// page, until one is `last`.
fn lines(tab: &mut Tab, server: &TestServer, last: &str) -> Vec<String> {
    tab.engine.set_scripting(true);
    tab.engine.navigate(server.url("/"));
    tab.run_until(|lines| {
        lines
            .iter()
            .any(|line| line == last || line.starts_with("failed"))
    })
}

const BASICS: &str = r#"
const log = (...a) => console.log(a.join(' '));
const open = indexedDB.open('people', 1);
open.onupgradeneeded = (e) => {
  log('upgrade', e.oldVersion, e.newVersion, e.target.transaction.mode);
  const db = open.result;
  const store = db.createObjectStore('people', { keyPath: 'id', autoIncrement: true });
  store.createIndex('name', 'name');
  store.createIndex('tags', 'tags', { multiEntry: true });
  store.put({ name: 'bo', tags: ['a', 'b'] });
  store.put({ name: 'al', tags: ['b'] });
  store.add({ id: 10, name: 'cy', tags: [] });
};
open.onsuccess = () => {
  const db = open.result;
  log('open', db.name, db.version, Array.from(db.objectStoreNames).join(','));
  const t = db.transaction('people', 'readwrite');
  const store = t.objectStore('people');
  store.get(1).onsuccess = (e) => log('get', JSON.stringify(e.target.result));
  store.getAll(IDBKeyRange.bound(1, 2)).onsuccess = (e) =>
    log('range', e.target.result.map((p) => p.name).join(','));
  store.index('name').getAll().onsuccess = (e) =>
    log('by name', e.target.result.map((p) => p.name).join(','));
  store.index('tags').getAllKeys('b').onsuccess = (e) => log('tag b', e.target.result.join(','));
  store.count().onsuccess = (e) => log('count', e.target.result);
  const names = [];
  store.openCursor(null, 'prev').onsuccess = (e) => {
    const cursor = e.target.result;
    if (cursor) {
      names.push(cursor.value.name + '@' + cursor.key);
      cursor.continue();
    } else {
      log('cursor', names.join(','));
    }
  };
  store.add({ id: 1, name: 'twice' }).onerror = (e) => {
    log('add error', e.target.error.name);
    e.preventDefault();
  };
  t.oncomplete = () => {
    log('complete');
    const t2 = db.transaction('people', 'readwrite');
    t2.objectStore('people').put({ name: 'lost' });
    t2.abort();
    t2.onabort = () => {
      db.transaction('people').objectStore('people').count().onsuccess = (e) => {
        log('after abort', e.target.result);
        log('done');
      };
    };
  };
};
open.onerror = () => log('failed', open.error && open.error.name);
"#;

#[test]
fn a_page_opens_upgrades_and_reads_its_database() {
    let server = page(BASICS);
    let mut tab = Tab::new();
    let lines = lines(&mut tab, &server, "done");
    assert_eq!(
        lines,
        [
            "upgrade 0 1 versionchange",
            "open people 1 people",
            r#"get {"name":"bo","tags":["a","b"],"id":1}"#,
            "range bo,al",
            "by name al,bo,cy",
            "tag b 1,2",
            "count 3",
            // The cursor's continue() asks after the add(), whose error
            // comes first.
            "add error ConstraintError",
            "cursor cy@10,al@2,bo@1",
            "complete",
            "after abort 3",
            "done",
        ]
    );
}

const ERRORS: &str = r#"
const log = (...a) => console.log(a.join(' '));
const tryIt = (what, f) => { try { f(); log(what, 'ok'); } catch (e) { log(what, e.name); } };
tryIt('no key', () => indexedDB.cmp(1, {}));
log('cmp', indexedDB.cmp(1, 'a'), indexedDB.cmp([1, 2], [1]), indexedDB.cmp('b', 'a'));
tryIt('bad range', () => IDBKeyRange.bound(2, 1));
log('includes', IDBKeyRange.lowerBound(3, true).includes(3), IDBKeyRange.only('x').includes('x'));
tryIt('version 0', () => indexedDB.open('x', 0));
const open = indexedDB.open('e', 2);
open.onupgradeneeded = () => {
  const db = open.result;
  tryIt('bad path', () => db.createObjectStore('s', { keyPath: 'a b' }));
  db.createObjectStore('s');
  tryIt('again', () => db.createObjectStore('s'));
  tryIt('auto array', () => db.createObjectStore('t', { keyPath: ['a'], autoIncrement: true }));
};
open.onsuccess = () => {
  const db = open.result;
  const t = db.transaction(['s'], 'readonly');
  const kept = t.objectStore('s');
  tryIt('read-only put', () => kept.put(1, 1));
  tryIt('no store', () => db.transaction('nope'));
  tryIt('no key given', () => db.transaction('s', 'readwrite').objectStore('s').put({ a: 1 }));
  setTimeout(() => {
    tryIt('inactive', () => kept.get(1));
    tryIt('finished', () => t.objectStore('s'));
    const lower = indexedDB.open('e', 1);
    lower.onerror = (e) => { log('lower', lower.error.name); e.preventDefault(); log('done'); };
  }, 0);
};
"#;

#[test]
fn what_indexed_db_refuses_it_refuses_in_the_reference_browsers_words() {
    let server = page(ERRORS);
    let mut tab = Tab::new();
    let lines = lines(&mut tab, &server, "done");
    assert_eq!(
        lines,
        [
            "no key DataError",
            "cmp -1 1 1",
            "bad range DataError",
            "includes false true",
            "version 0 TypeError",
            "bad path SyntaxError",
            "again ConstraintError",
            "auto array InvalidAccessError",
            "read-only put ReadOnlyError",
            "no store NotFoundError",
            "no key given DataError",
            "inactive TransactionInactiveError",
            "finished InvalidStateError",
            "lower VersionError",
            "done",
        ]
    );
}

const KEPT_WRITE: &str = r#"
const open = indexedDB.open('kept', 1);
open.onupgradeneeded = () => open.result.createObjectStore('s').put({ when: new Date(0), bytes: new Uint8Array([1, 2]) }, [1, 'a']);
open.onsuccess = () => console.log('written');
"#;

const KEPT_READ: &str = r#"
indexedDB.databases().then((list) => console.log('databases', JSON.stringify(list)));
const open = indexedDB.open('kept');
open.onsuccess = () => {
  const store = open.result.transaction('s').objectStore('s');
  store.get([1, 'a']).onsuccess = (e) => {
    const v = e.target.result;
    console.log('read', v.when.getTime(), v.bytes.join(','));
  };
};
"#;

#[test]
fn what_a_page_kept_is_there_when_the_profile_is_opened_again() {
    let memory = profile::Memory::new();
    // Two pages of one origin: one writes, the other reads.
    let server = TestServer::plain(|request| {
        let script = if request.target == "/read" {
            KEPT_READ
        } else {
            KEPT_WRITE
        };
        let body = format!("<!doctype html><script>{script}</script>");
        Reply::ok("text/html", body.as_bytes())
    });
    let run = |path: &str, last: &str| {
        let mut tab = Tab::new();
        tab.engine
            .set_profile(profile::Profile::in_memory(&memory).unwrap())
            .unwrap();
        tab.engine.set_scripting(true);
        tab.engine.navigate(server.url(path));
        let lines = tab.run_until(|lines| {
            lines.iter().filter(|l| l.starts_with(last)).count() > 0 && lines.len() >= 2
                || lines.iter().any(|l| l == "written")
        });
        tab.engine.flush_profile().unwrap();
        lines
    };
    assert_eq!(run("/write", "written"), ["written"]);
    let lines = run("/read", "read");
    assert!(
        lines.contains(&r#"databases [{"name":"kept","version":1}]"#.to_owned()),
        "{lines:?}"
    );
    assert!(lines.contains(&"read 0 1,2".to_owned()), "{lines:?}");
}

const CHECKPOINT: &str = r#"
const open = indexedDB.open('c', 1);
open.addEventListener('success', () => {
  Promise.resolve().then(() => console.log('microtask'));
  console.log('first');
});
open.addEventListener('success', () => { console.log('second'); console.log('done'); });
"#;

#[test]
fn a_success_event_checkpoints_microtasks_after_each_listener() {
    let server = page(CHECKPOINT);
    let mut tab = Tab::new();
    assert_eq!(
        lines(&mut tab, &server, "done"),
        ["first", "microtask", "second", "done"]
    );
}

const ABORTS: &str = r#"
const log = (...a) => console.log(a.join(' '));
const open = indexedDB.open('a', 1);
open.onupgradeneeded = () => open.result.createObjectStore('s').put(1, 'k');
open.onsuccess = () => {
  const db = open.result;
  db.onerror = (e) => log('bubbled', e.target.error.name);
  const t1 = db.transaction('s', 'readwrite');
  t1.objectStore('s').put(2, 'k');
  t1.oncomplete = () => log('t1 complete');
  // A writer of the same store waits for the first.
  const t2 = db.transaction('s', 'readwrite');
  t2.objectStore('s').get('k').onsuccess = (e) => log('t2 read', e.target.result);
  // An error not prevented aborts its transaction, after bubbling.
  t2.objectStore('s').add(3, 'k');
  t2.onabort = () => {
    log('t2 aborted', t2.error.name);
    db.transaction('s').objectStore('s').get('k').onsuccess = (e) => {
      log('kept', e.target.result);
      log('done');
    };
  };
};
"#;

#[test]
fn writers_wait_their_turn_and_an_error_not_prevented_aborts() {
    let server = page(ABORTS);
    let mut tab = Tab::new();
    assert_eq!(
        lines(&mut tab, &server, "done"),
        [
            "t1 complete",
            "t2 read 2",
            "bubbled ConstraintError",
            "t2 aborted ConstraintError",
            "kept 2",
            "done",
        ]
    );
}

#[test]
fn an_opaque_origins_page_has_no_databases() {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    let url = "data:text/html,<script>try { indexedDB.open('x'); console.log('opened') } catch (e) { console.log(e.name) }</script>";
    tab.engine.navigate(net::Url::parse(url).expect("a URL"));
    let lines = tab.run_until(|lines| !lines.is_empty());
    assert_eq!(lines, ["SecurityError"]);
}
