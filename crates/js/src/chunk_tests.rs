//! The binding chunks read at their first use (D64): each chunk's names
//! are on the global and the prototypes that read it before its use, the
//! same names after, and a page that touches a chunk gets what it got
//! when the bindings were made at once.

use crate::tests::{Recorder, realm, script};
use crate::{Completion, Limits, QuickJs, Realm, RealmKind, ScriptEngine};

/// The string `source` ends with, run in a new realm of `kind`.
fn evaluated_in(kind: RealmKind, source: &str) -> String {
    let mut realm: Box<dyn Realm<Recorder>> = QuickJs
        .realm_of(kind, Limits::default(), None)
        .expect("a realm");
    let mut host = Recorder::default();
    match realm.run(&script(source), &mut host) {
        Ok(Completion::String(text)) => text,
        other => panic!("{source}: {other:?}"),
    }
}

fn evaluated(source: &str) -> String {
    evaluated_in(RealmKind::Window, source)
}

/// The own names of the global and of the prototypes the chunks put
/// their names on, a line each, then the accessors among them; with
/// `touch`, run first, every chunk this realm reaches made.
const NAMES: &str = "
const g = globalThis;
const targets = [['global', g]];
for (const name of ['Navigator', 'WorkerNavigator', 'HTMLCanvasElement', 'Element', 'Document']) {
  const d = Object.getOwnPropertyDescriptor(g, name);
  if (d !== undefined && 'value' in d) targets.push([name + '.prototype', d.value.prototype]);
}
targets.push(['Number.prototype', Number.prototype], ['BigInt.prototype', BigInt.prototype],
  ['Date.prototype', Date.prototype], ['String.prototype', String.prototype]);
TOUCH
for (const [, t] of targets) {
  for (const n of Object.getOwnPropertyNames(t)) {
    try { void t[n]; } catch (e) {}
  }
}
const lines = [];
const accessors = [];
for (const [label, t] of targets) {
  const names = Object.getOwnPropertyNames(t);
  lines.push(label + ': ' + names.join(' '));
  for (const n of names) {
    if ('get' in Object.getOwnPropertyDescriptor(t, n)) accessors.push(label + '.' + n);
  }
}
lines.join('\\n') + '\\n--\\n' + accessors.join(' ')
";

/// The names, and the accessors, of a realm of `kind` before any chunk
/// is made, and of one that made each it reaches.
fn names_untouched_and_touched(kind: &RealmKind, touch: &str) -> [(String, Vec<String>); 2] {
    let untouched = NAMES.replace("TOUCH", "").replace(
        "for (const [, t] of targets) {\n  for",
        "if (false) for (const [, t] of targets) {\n  for",
    );
    let touched = NAMES.replace("TOUCH", touch);
    [untouched, touched].map(|source| {
        let text = evaluated_in(kind.clone(), &source);
        let (names, accessors) = text.split_once("\n--\n").expect("two parts");
        (
            names.to_owned(),
            accessors.split(' ').map(str::to_owned).collect(),
        )
    })
}

/// Each chunk's names are complete (D64): the global and the prototypes
/// that read a chunk have the same own names, in the same order, before
/// any chunk is made and after every chunk the realm reaches is; what
/// differs is only that the chunks' accessors became what they stand
/// for. (`chunkInstall` holds the other half: an interface a chunk makes
/// and the realm exposes is one of its accessors.)
#[test]
fn each_chunks_names_are_the_same_before_and_after_it_is_made() {
    let kinds = [
        (
            RealmKind::Window,
            "void Intl, WebAssembly, AudioContext, IDBFactory, indexedDB, navigator.storage, \
             ImageData, GPUBuffer;",
            [
                "global.Intl",
                "global.WebAssembly",
                "global.AudioContext",
                "global.OfflineAudioCompletionEvent",
                "global.ScriptProcessorNode",
                "global.MediaStream",
                "global.CanvasCaptureMediaStreamTrack",
                "global.IDBFactory",
                "global.DOMStringList",
                "global.StorageManager",
                "global.CanvasRenderingContext2D",
                "global.ImageBitmap",
                "global.GPUBuffer",
                "global.GPUBufferUsage",
                "global.Animation",
                "global.KeyframeEffect",
                "Element.prototype.animate",
                "Document.prototype.getAnimations",
            ]
            .as_slice(),
            126,
        ),
        (
            RealmKind::DedicatedWorker {
                name: String::new(),
            },
            "void Intl, WebAssembly, ImageData;",
            [
                "global.Intl",
                "global.WebAssembly",
                "global.ImageData",
                "global.OffscreenCanvas",
            ]
            .as_slice(),
            10,
        ),
        (
            RealmKind::ServiceWorker,
            "void Intl, WebAssembly, ImageData;",
            ["global.Intl", "global.WebAssembly", "global.ImageBitmap"].as_slice(),
            10,
        ),
    ];
    for (kind, touch, some, count) in kinds {
        let [(before, lazy), (after, still)] = names_untouched_and_touched(&kind, touch);
        assert_eq!(before, after, "{kind:?}: the same names");
        assert!(
            still.iter().all(|name| lazy.contains(name)),
            "{kind:?}: no accessor made"
        );
        let made: Vec<&String> = lazy.iter().filter(|name| !still.contains(name)).collect();
        for name in some {
            assert!(made.iter().any(|m| m == name), "{kind:?}: {name} made");
        }
        assert_eq!(made.len(), count, "{kind:?}: {made:?}");
    }
}

/// `Intl`'s chunk is made by `Intl` or by a locale form of the
/// language's methods, which keep their functions, and answer as before.
#[test]
fn the_intl_chunk_is_made_at_its_first_use() {
    assert_eq!(
        evaluated(
            "const form = Number.prototype.toLocaleString;\
             const d = (o, n) => Object.getOwnPropertyDescriptor(o, n);\
             const before = typeof d(globalThis, 'Intl').get;\
             const text = (1234.5).toLocaleString('en-US');\
             const after = typeof d(globalThis, 'Intl').value;\
             const own = d(Number.prototype, 'toLocaleString');\
             [before, text, after, form === Number.prototype.toLocaleString,\
              own.writable && own.configurable && !own.enumerable, form.name, form.length,\
              String.prototype.localeCompare.length, 'a'.localeCompare('b'),\
              new Date(0).toLocaleDateString('en-US', { timeZone: 'UTC' }),\
              (12n).toLocaleString('de'), new Intl.NumberFormat('de').format(1234.5),\
              Intl.getCanonicalLocales('EN-us')[0]].join(' ')"
        ),
        "function 1,234.5 object true true toLocaleString 0 1 -1 1/1/1970 12 1.234,5 en-US"
    );
    // The chunk's exceptions have the page's frames, not the chunk's.
    let mut realm = realm();
    let mut host = Recorder::default();
    let thrown = realm
        .run(
            &script("\n\nString.prototype.localeCompare.call(null);"),
            &mut host,
        )
        .expect_err("a TypeError");
    let places: Vec<(&str, u32)> = thrown
        .stack
        .iter()
        .map(|frame| (frame.url.as_str(), frame.line))
        .collect();
    assert_eq!(places, [("https://example.com/page.html", 3)]);
    assert!(thrown.message.contains("localeCompare called on null"));
}

/// IndexedDB's and StorageManager's chunk is made by their interfaces,
/// `indexedDB` or `navigator.storage`, which stay the getters they were.
#[test]
fn the_indexeddb_chunk_is_made_at_its_first_use() {
    assert_eq!(
        evaluated(
            "const d = (o, n) => Object.getOwnPropertyDescriptor(o, n);\
             const before = [typeof d(globalThis, 'IDBFactory').get, typeof d(globalThis, 'indexedDB').get,\
               typeof d(Navigator.prototype, 'storage').get].join();\
             const db = indexedDB;\
             [before, db === indexedDB, db instanceof IDBFactory, String(db),\
              navigator.storage instanceof StorageManager, typeof d(globalThis, 'IDBFactory').value,\
              indexedDB.cmp(1, 2), IDBKeyRange.bound(1, 5, true).lowerOpen,\
              Object.keys(IDBObjectStore.prototype).includes('createIndex')].join(' ')"
        ),
        "function,function,function true true [object IDBFactory] true function -1 true true"
    );
}

/// Web Audio's chunk is made by its interfaces.
#[test]
fn the_web_audio_chunk_is_made_at_its_first_use() {
    assert_eq!(
        evaluated(
            "const before = typeof Object.getOwnPropertyDescriptor(globalThis, 'AudioBuffer').get;\
             const buffer = new AudioBuffer({ length: 4, sampleRate: 8000, numberOfChannels: 2 });\
             let error;\
             try { new AudioBuffer({ length: 4 }); } catch (e) { error = e.message; }\
             [before, buffer.length, buffer.numberOfChannels, buffer.getChannelData(1).length,\
              Object.getPrototypeOf(GainNode.prototype) === AudioNode.prototype,\
              typeof AudioContext.prototype.createGain, error].join(' ')"
        ),
        "function 4 2 4 true function Failed to construct 'AudioBuffer': Failed to read the \
         'sampleRate' property from 'AudioBufferOptions': Required member is undefined."
    );
}

/// WebAssembly's chunk is made by `WebAssembly`, in a window and in a
/// worker.
#[test]
fn the_webassembly_chunk_is_made_at_its_first_use() {
    let source = "const before = typeof Object.getOwnPropertyDescriptor(globalThis, 'WebAssembly').get;\
         const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);\
         const module = new WebAssembly.Module(bytes);\
         [before, WebAssembly.validate(bytes), String(WebAssembly),\
          new WebAssembly.Instance(module) instanceof WebAssembly.Instance,\
          new WebAssembly.Memory({ initial: 1 }).buffer.byteLength,\
          Object.keys(WebAssembly).join()].join(' ')";
    let expected = "function true [object WebAssembly] true 65536 \
                    validate,compile,instantiate,compileStreaming,instantiateStreaming";
    assert_eq!(evaluated(source), expected);
    let worker = RealmKind::DedicatedWorker {
        name: String::new(),
    };
    assert_eq!(evaluated_in(worker, source), expected);
}

/// The 2D canvas's chunk is made by its interfaces or by
/// `createImageBitmap`, which keeps its function, in a window and in a
/// worker.
#[test]
fn the_canvas_chunk_is_made_at_its_first_use() {
    let source = "const d = (o, n) => Object.getOwnPropertyDescriptor(o, n);\
         const create = createImageBitmap;\
         const before = typeof d(globalThis, 'ImageData').get;\
         create().catch(() => {});\
         const after = typeof d(globalThis, 'ImageData').value;\
         const data = new ImageData(2, 3);\
         [before, after, create === createImageBitmap, create.name, create.length,\
          d(globalThis, 'createImageBitmap').enumerable, data.data.length, data.colorSpace,\
          Object.getPrototypeOf(new ImageData(1, 1)) === ImageData.prototype,\
          typeof Path2D.prototype.arc].join(' ')";
    let expected = "function function true createImageBitmap 1 true 24 srgb true function";
    assert_eq!(evaluated(source), expected);
    let worker = RealmKind::DedicatedWorker {
        name: String::new(),
    };
    assert_eq!(evaluated_in(worker, source), expected);
    // `createImageBitmap` refuses as it did.
    let mut realm = realm();
    let mut host = Recorder::default();
    realm
        .run(
            &script("createImageBitmap().catch((e) => console.log(e.name + ': ' + e.message))"),
            &mut host,
        )
        .expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    assert_eq!(
        host.console
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>(),
        [
            "TypeError: Failed to execute 'createImageBitmap' on 'Window': 1 argument required, \
          but only 0 present."
        ]
    );
}

/// What a page put in place of a chunk's name before the chunk is made
/// stays: a value set, a property defined, a name deleted.
#[test]
fn what_a_page_puts_in_place_of_a_chunks_name_stays() {
    assert_eq!(
        evaluated(
            "Object.defineProperty(globalThis, 'DelayNode', { value: 6, configurable: true });\
             delete globalThis.PeriodicWave;\
             GainNode = 5;\
             [GainNode, DelayNode, 'PeriodicWave' in globalThis, typeof AnalyserNode,\
              Object.getOwnPropertyDescriptor(globalThis, 'GainNode').writable].join(' ')"
        ),
        "5 6 false function true"
    );
}

/// A worker has none of the window's chunk names: no accessor stands for
/// an interface its kind does not expose.
#[test]
fn a_worker_has_no_accessor_for_a_window_only_name() {
    let source = "['AudioContext', 'IDBFactory', 'CanvasRenderingContext2D']\
                  .map(function (n) { return n in self; }).join()";
    for kind in [
        RealmKind::DedicatedWorker {
            name: String::new(),
        },
        RealmKind::ServiceWorker,
    ] {
        assert_eq!(evaluated_in(kind, source), "false,false,false");
    }
}
