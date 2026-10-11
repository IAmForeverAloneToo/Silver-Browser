//! Structured clone in a realm (M31, HTML section 2.7): each value made
//! again as the reference browser makes it when it goes to a worker and
//! back, told by the probe page's own `describe`, and each refusal in
//! its words.

use crate::tests::logged;

/// The probe page's description of a value.
const DESCRIBE: &str = r##"
const describe = (x, seen) => {
  if (x === null || typeof x !== "object") {
    if (typeof x === "number" && Object.is(x, -0)) return "-0";
    if (typeof x === "bigint") return x + "n";
    return typeof x + ":" + String(x);
  }
  if (seen.has(x)) return "#cycle";
  seen.add(x);
  const tag = Object.prototype.toString.call(x);
  if (x instanceof Date) return "Date:" + x.getTime();
  if (x instanceof RegExp) return "RegExp:" + x.source + "/" + x.flags + " last=" + x.lastIndex;
  if (x instanceof ArrayBuffer) return "ArrayBuffer:" + x.byteLength;
  if (ArrayBuffer.isView(x)) return tag + ":" + x.byteOffset + "," + x.byteLength + " [" +
    Array.from(new Uint8Array(x.buffer, x.byteOffset, Math.min(x.byteLength, 8))) + "] sameBuf=" + (seen.lastBuf === x.buffer) + (seen.lastBuf = x.buffer, "");
  if (x instanceof Map) return "Map{" + Array.from(x, ([k, val]) => describe(k, seen) + "=>" + describe(val, seen)).join(",") + "}";
  if (x instanceof Set) return "Set{" + Array.from(x, (val) => describe(val, seen)).join(",") + "}";
  if (x instanceof Error) return "Error:" + x.constructor.name + "/" + x.name + "/" + x.message + " stack=" + (typeof x.stack) + " cause=" + ("cause" in x ? describe(x.cause, seen) : "none") + " own=" + Object.getOwnPropertyNames(x).sort();
  if (x instanceof Blob) return "Blob:" + x.size + "," + x.type + (x instanceof File ? ",File:" + x.name : "");
  if (x instanceof ImageData) return "ImageData:" + x.width + "x" + x.height + "," + x.data.length + "," + x.colorSpace;
  if (x instanceof Boolean || x instanceof Number || x instanceof String || x instanceof BigInt) return tag + ":" + x.valueOf();
  if (Array.isArray(x)) return "Array(" + x.length + ")[" + Object.keys(x).map((k) => k + ":" + describe(x[k], seen)).join(",") + "] proto=" + (Object.getPrototypeOf(x) === Array.prototype);
  return tag + "{" + Object.keys(x).map((k) => k + ":" + describe(x[k], seen)).join(",") + "} proto=" + (Object.getPrototypeOf(x) === Object.prototype);
};
"##;

#[track_caller]
fn check(lines: &[String], want: &[&str]) {
    for line in want {
        assert!(lines.iter().any(|l| l == line), "no {line:?} in {lines:#?}");
    }
}

#[test]
fn values_are_made_again_as_the_reference_browser_makes_them() {
    let source = format!(
        "{DESCRIBE}{}",
        r#"
var cyc = { a: 1 }; cyc.self = cyc;
var buf = new ArrayBuffer(16); new Uint8Array(buf).set([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
var sparse = [1, , 3]; sparse.length = 5; sparse.extra = "x";
var re = /ab+c/gi; re.lastIndex = 3;
var e1 = new TypeError("bad thing", { cause: 7 }); e1.extra = 1;
var custom = new Error("plain"); custom.name = "MyError";
var sym = { a: 1 }; sym[Symbol("s")] = 2;
var holes = [0]; holes[3] = 3;
var values = [
  ["undefined", undefined], ["null", null], ["negzero", -0], ["nan", NaN],
  ["bigint", 12345678901234567890n], ["bool", true], ["Number obj", new Number(3)],
  ["String obj", new String("s")], ["Boolean obj", new Boolean(false)],
  ["BigInt obj", Object(5n)], ["date", new Date(1700000000000)], ["regexp", re],
  ["cycle", cyc], ["sparse", sparse], ["map", new Map([[1, "a"], [{ k: 1 }, [2]]])],
  ["set", new Set([1, "1", {}])], ["buffer", buf],
  ["views", [new Uint8Array(buf, 2, 4), new Float64Array(buf, 8, 1), new DataView(buf, 1, 3)]],
  ["error", e1], ["custom error", custom], ["range", new RangeError("r")],
  ["dom exception", new DOMException("dm", "NotFoundError")],
  ["blob", new Blob(["abc"], { type: "text/plain" })],
  ["file", new File(["xy"], "f.txt", { type: "text/x", lastModified: 5 })],
  ["imagedata", new ImageData(2, 3)],
  ["class", new (class Foo { constructor() { this.x = 1; } })()],
  ["symbol key", sym], ["getter", { get g() { return 42; } }],
  ["nonenumerable", Object.defineProperty({ a: 1 }, "h", { value: 2, enumerable: false })],
  ["holes", holes],
];
for (var i = 0; i < values.length; i++) {
  console.log(values[i][0] + "=" + describe(structuredClone(values[i][1]), new Set()));
}
"#
    );
    check(
        &logged(&source),
        &[
            "undefined=undefined:undefined",
            "null=object:null",
            "negzero=-0",
            "nan=number:NaN",
            "bigint=12345678901234567890n",
            "bool=boolean:true",
            "Number obj=[object Number]:3",
            "String obj=[object String]:s",
            "Boolean obj=[object Boolean]:false",
            "BigInt obj=[object BigInt]:5",
            "date=Date:1700000000000",
            "regexp=RegExp:ab+c/gi last=0",
            "cycle=[object Object]{a:number:1,self:#cycle} proto=true",
            "sparse=Array(5)[0:number:1,2:number:3,extra:string:x] proto=true",
            "map=Map{number:1=>string:a,[object Object]{k:number:1} proto=true=>Array(1)[0:number:2] proto=true}",
            "set=Set{number:1,string:1,[object Object]{} proto=true}",
            "buffer=ArrayBuffer:16",
            "views=Array(3)[0:[object Uint8Array]:2,4 [3,4,5,6] sameBuf=false,\
             1:[object Float64Array]:8,8 [9,10,0,0,0,0,0,0] sameBuf=true,\
             2:[object DataView]:1,3 [2,3,4] sameBuf=true] proto=true",
            "error=Error:TypeError/TypeError/bad thing stack=string cause=number:7 own=cause,message,stack",
            "custom error=Error:Error/Error/plain stack=string cause=none own=message,stack",
            "range=Error:RangeError/RangeError/r stack=string cause=none own=message,stack",
            "dom exception=Error:DOMException/NotFoundError/dm stack=undefined cause=none own=",
            "blob=Blob:3,text/plain",
            "file=Blob:2,text/x,File:f.txt",
            "imagedata=ImageData:2x3,24,srgb",
            "class=[object Object]{x:number:1} proto=true",
            "symbol key=[object Object]{a:number:1} proto=true",
            "getter=[object Object]{g:number:42} proto=true",
            "nonenumerable=[object Object]{a:number:1} proto=true",
            "holes=Array(4)[0:number:0,3:number:3] proto=true",
        ],
    );
}

#[test]
fn what_cannot_be_cloned_is_refused_in_the_reference_browsers_words() {
    let lines = logged(
        r#"
var err = function (e) {
  return e.name + ": " + e.message + " [" + e.constructor.name + " " + e.code + "]";
};
var bad = [["function", function f() {}], ["symbol", Symbol("q")],
  ["inner function", { f: function () {} }], ["promise", Promise.resolve(1)],
  ["weakmap", new WeakMap()], ["proxy", new Proxy({}, {})], ["window", window],
  ["event", new Event("x")]];
for (var j = 0; j < bad.length; j++) {
  try {
    structuredClone(bad[j][1]);
    console.log(bad[j][0] + "=no throw");
  } catch (e) {
    console.log(bad[j][0] + "=" + err(e));
  }
}
var t = new ArrayBuffer(8);
var c = structuredClone(t, { transfer: [t] });
console.log("transfer=" + t.byteLength + " " + c.byteLength);
try { structuredClone(t, { transfer: [t] }); } catch (e) { console.log("detached=" + err(e)); }
var t2 = new ArrayBuffer(4);
try { structuredClone(1, { transfer: [t2, t2] }); } catch (e) { console.log("duplicate=" + err(e)); }
try { structuredClone(1, { transfer: [{}] }); } catch (e) { console.log("object=" + err(e)); }
try { structuredClone(); } catch (e) { console.log("none=" + err(e)); }
console.log("bare undefined=" + String(structuredClone(undefined)));
var order = [];
var o = {};
["b", "a", "c"].forEach(function (k) {
  Object.defineProperty(o, k, { enumerable: true, get: function () { order.push(k); return k; } });
});
structuredClone(o);
console.log("getter order=" + order.join(""));
try {
  structuredClone({ get x() { throw new Error("from getter"); } });
} catch (e) {
  console.log("getter throws=" + e.message);
}
"#,
    );
    let w = "Failed to execute 'structuredClone' on 'Window': ";
    let refused =
        |what: &str, why: &str| format!("{what}=DataCloneError: {w}{why} [DOMException 25]");
    let want = [
        refused("function", "function f() {} could not be cloned."),
        refused("symbol", "Symbol(q) could not be cloned."),
        refused("inner function", "function () {} could not be cloned."),
        refused("promise", "#<Promise> could not be cloned."),
        refused("weakmap", "#<WeakMap> could not be cloned."),
        refused("proxy", "#<Object> could not be cloned."),
        refused("window", "#<Window> could not be cloned."),
        refused("event", "Event object could not be cloned."),
        "transfer=0 8".to_owned(),
        refused("detached", "ArrayBuffer at index 0 is already detached."),
        refused(
            "duplicate",
            "ArrayBuffer at index 1 is a duplicate of an earlier ArrayBuffer.",
        ),
        refused(
            "object",
            "Value at index 0 does not have a transferable type.",
        ),
        format!(
            "none=TypeError: {w}1 argument required, but only 0 present. [TypeError undefined]"
        ),
        "bare undefined=undefined".to_owned(),
        "getter order=bac".to_owned(),
        "getter throws=from getter".to_owned(),
    ];
    let want: Vec<&str> = want.iter().map(String::as_str).collect();
    check(&lines, &want);
}

#[test]
fn a_message_to_the_window_is_made_again_in_a_message_event() {
    let lines = logged(
        r#"
var e = new MessageEvent("message", { data: { a: 1 }, origin: "https://a.test", source: window });
console.log("event=" + e.data.a + " " + e.origin + " " + (e.source === window) + " " +
  e.ports.length + " " + Object.isFrozen(e.ports) + " " + e.lastEventId.length);
try { new MessageEvent("message", { source: {} }); } catch (x) { console.log("source=" + x.message); }
try { new MessageEvent("message", { ports: [1] }); } catch (x) { console.log("ports=" + x.message); }
try { window.postMessage(); } catch (x) { console.log("none=" + x.message); }
try { window.postMessage(1, "*", [{}]); } catch (x) { console.log("transfer=" + x.message); }
try { window.postMessage(function () {}, "*"); } catch (x) { console.log("function=" + x.name); }
"#,
    );
    check(
        &lines,
        &[
            "event=1 https://a.test true 0 true 0",
            "source=Failed to construct 'MessageEvent': Failed to read the 'source' property \
             from 'MessageEventInit': The provided value is not of type '(MessagePort or \
             ServiceWorker or Window)'.",
            "ports=Failed to construct 'MessageEvent': Failed to read the 'ports' property from \
             'MessageEventInit': Failed to convert value to 'MessagePort'.",
            "none=Failed to execute 'postMessage' on 'Window': 1 argument required, but only 0 present.",
            "transfer=Failed to execute 'postMessage' on 'Window': Value at index 0 does not have a \
             transferable type.",
            "function=DataCloneError",
        ],
    );
}
