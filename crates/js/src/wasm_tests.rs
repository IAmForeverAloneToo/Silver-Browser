//! WebAssembly in the realm (M29, `docs/design/wasm.md`): the
//! interface as scripts use it, its answers and words held to the
//! reference browser's (scratchpad m29 `p1-wasm.html`, Chrome 154)
//! where the interface leaves them open.

use crate::tests::{Recorder, realm, script};

/// A module's bytes, from its text, as a script's `Uint8Array`.
fn module(text: &str) -> String {
    let bytes = wat::parse_str(text).expect("the test's module parses");
    let list: Vec<String> = bytes.iter().map(u8::to_string).collect();
    format!("new Uint8Array([{}])", list.join(","))
}

/// The console's lines after running `source`, each step of it logged
/// by `step(name, f)` as `name=` and its value in JSON, or what it
/// threw.
fn steps(source: &str) -> Vec<String> {
    let mut realm = realm();
    let mut host = Recorder::default();
    let prelude = "const step = (name, f) => { let v; try { v = JSON.stringify(f()); } \
                   catch (e) { v = 'threw ' + (e && e.constructor && e.constructor.name) + \
                   ': ' + (e && e.message); } console.log(name + '=' + v); };\n";
    realm
        .run(&script(&format!("{prelude}{source}")), &mut host)
        .expect("it runs");
    assert!(realm.checkpoint(&mut host).is_empty());
    host.console.into_iter().map(|(_, text)| text).collect()
}

const ADD: &str = r#"(module (func (export "add") (param i32 i32) (result i32)
    local.get 0 local.get 1 i32.add))"#;

#[test]
fn the_namespace_is_the_interfaces() {
    let lines = steps(
        r#"
step("ns", () => [typeof WebAssembly, String(WebAssembly)]);
step("errors", () => ["CompileError", "LinkError", "RuntimeError"].map(n => {
  const E = WebAssembly[n], e = new E("x");
  return [e instanceof Error, e.name, e.message, String(e), Object.prototype.toString.call(E.prototype),
    E.prototype.message, Object.getOwnPropertyDescriptor(WebAssembly, n).enumerable];
}));
step("validateDesc", () => Object.getOwnPropertyDescriptor(WebAssembly, "validate").enumerable);
step("moduleTag", () => Object.prototype.toString.call(WebAssembly.Module.prototype));
"#,
    );
    assert_eq!(
        lines,
        [
            r#"ns=["object","[object WebAssembly]"]"#,
            r#"errors=[[true,"CompileError","x","CompileError: x","[object Object]","",false],[true,"LinkError","x","LinkError: x","[object Object]","",false],[true,"RuntimeError","x","RuntimeError: x","[object Object]","",false]]"#,
            "validateDesc=true",
            r#"moduleTag="[object WebAssembly.Module]""#,
        ]
    );
}

#[test]
fn bytes_are_validated_and_compiled_or_refused_in_the_reference_browsers_words() {
    let add = module(ADD);
    let lines = steps(&format!(
        r#"
const add = {add};
step("valid", () => [WebAssembly.validate(add), WebAssembly.validate(add.buffer),
  WebAssembly.validate(new DataView(add.buffer)), WebAssembly.validate(new Uint8Array([0, 1, 2]))]);
step("validateArg", () => WebAssembly.validate(42));
step("module", () => {{ const m = new WebAssembly.Module(add); return [m instanceof WebAssembly.Module,
  Object.prototype.toString.call(m)]; }});
step("compileError", () => {{ try {{ new WebAssembly.Module(new Uint8Array([0, 1, 2])); }} catch (e) {{
  return [e instanceof WebAssembly.CompileError, e.message.startsWith("WebAssembly.Module(): ")]; }} }});
step("moduleArg", () => new WebAssembly.Module(42));
step("moduleNoNew", () => WebAssembly.Module(add));
step("copied", () => {{ const b = new Uint8Array(add); const m = new WebAssembly.Module(b); b.fill(0);
  return WebAssembly.Module.exports(m).length; }});
"#
    ));
    assert_eq!(
        lines,
        [
            "valid=[true,true,true,false]",
            "validateArg=threw TypeError: WebAssembly.validate(): Argument 0 must be a buffer source",
            r#"module=[true,"[object WebAssembly.Module]"]"#,
            "compileError=[true,true]",
            "moduleArg=threw TypeError: WebAssembly.Module(): Argument 0 must be a buffer source",
            "moduleNoNew=threw TypeError: class constructors must be invoked with 'new'",
            "copied=1",
        ]
    );
}

#[test]
fn a_module_describes_its_imports_exports_and_custom_sections() {
    let reflected = module(
        r#"(module
  (import "env" "f" (func (param i32)))
  (import "env" "mem" (memory 1))
  (import "js" "table" (table 1 funcref))
  (import "js" "g" (global i32))
  (func (export "run"))
  (memory (export "memory") 1)
  (table (export "t") 2 funcref)
  (global (export "answer") i32 (i32.const 42))
  (@custom "hello" "\01\02\03")
  (@custom "hello" "\04")
  (@custom "other" ""))"#,
    );
    let lines = steps(&format!(
        r#"
const m = new WebAssembly.Module({reflected});
step("exports", () => WebAssembly.Module.exports(m));
step("imports", () => WebAssembly.Module.imports(m));
step("custom", () => WebAssembly.Module.customSections(m, "hello").map(b => [b instanceof ArrayBuffer,
  Array.from(new Uint8Array(b))]));
step("customNone", () => WebAssembly.Module.customSections(m, "none").length);
step("notModule", () => WebAssembly.Module.exports({{}}));
step("noName", () => WebAssembly.Module.customSections(m));
"#
    ));
    assert_eq!(
        lines,
        [
            r#"exports=[{"name":"run","kind":"function"},{"name":"memory","kind":"memory"},{"name":"t","kind":"table"},{"name":"answer","kind":"global"}]"#,
            r#"imports=[{"module":"env","name":"f","kind":"function"},{"module":"env","name":"mem","kind":"memory"},{"module":"js","name":"table","kind":"table"},{"module":"js","name":"g","kind":"global"}]"#,
            "custom=[[true,[1,2,3]],[true,[4]]]",
            "customNone=0",
            "notModule=threw TypeError: WebAssembly.Module.exports(): Argument 0 must be a WebAssembly.Module",
            "noName=threw TypeError: WebAssembly.Module.customSections(): Argument 1 is required",
        ]
    );
}

#[test]
fn base64_from_the_realm_is_read_as_it_writes_it() {
    use crate::quickjs::from_base64;
    assert_eq!(from_base64(""), Some(Vec::new()));
    assert_eq!(from_base64("AA=="), Some(vec![0]));
    assert_eq!(from_base64("AAE="), Some(vec![0, 1]));
    assert_eq!(from_base64("AAEC"), Some(vec![0, 1, 2]));
    assert_eq!(from_base64("/+8A"), Some(vec![0xff, 0xef, 0]));
    for bad in ["A", "AA=", "A===", "AA*=", "AAAA AAA"] {
        assert_eq!(from_base64(bad), None, "{bad}");
    }
}

#[test]
fn a_module_larger_than_the_limit_is_refused() {
    let lines = steps(
        r#"
step("big", () => WebAssembly.validate(new Uint8Array(64 * 1024 * 1024 + 1)));
step("module", () => new WebAssembly.Module(new Uint8Array(64 * 1024 * 1024 + 1)));
"#,
    );
    assert_eq!(
        lines,
        [
            "big=threw RangeError: WebAssembly.validate(): the module is larger than 64 MiB",
            "module=threw RangeError: WebAssembly.Module(): the module is larger than 64 MiB",
        ]
    );
}

#[test]
fn an_instances_functions_convert_as_the_interface_says() {
    let add = module(ADD);
    let echo = module(
        r#"(module
  (func (export "i64") (param i64) (result i64) local.get 0)
  (func (export "f32") (param f32) (result f32) local.get 0)
  (func (export "f64") (param f64) (result f64) local.get 0)
  (func (export "pair") (param i32) (result i32 i64) local.get 0 i64.const 7))"#,
    );
    let lines = steps(&format!(
        r#"
const instance = new WebAssembly.Instance(new WebAssembly.Module({add}));
const add = instance.exports.add;
step("calls", () => [add(2, 3), add(2.7, "4"), add(2 ** 31, 1), add(), add({{ valueOf() {{ return 9; }} }}, 1)]);
step("shape", () => [add.name, add.length, typeof add, Object.getPrototypeOf(add) === Function.prototype,
  add.hasOwnProperty("prototype"), Object.getOwnPropertyNames(add).sort().join()]);
step("string", () => String(add));
step("noNew", () => new add(1, 2));
step("same", () => [instance.exports.add === add, Object.isFrozen(instance.exports),
  Object.getPrototypeOf(instance.exports), Object.keys(instance.exports),
  Object.prototype.toString.call(instance)]);
step("frozen", () => {{ "use strict"; instance.exports.add = 1; }});
step("symbol", () => add(Symbol()));
step("bigint", () => add(1n));
const e = new WebAssembly.Instance(new WebAssembly.Module({echo})).exports;
step("i64", () => [String(e.i64(5n)), String(e.i64(2n ** 64n + 3n)), String(e.i64(-1n)), typeof e.i64(1n),
  String(e.i64("12")), String(e.i64(true))]);
step("i64Number", () => e.i64(5));
step("floats", () => [e.f32(0.1), e.f32(1e40), String(e.f32(NaN)), e.f64(0.1), e.f64("2.5")]);
step("pair", () => {{ const r = e.pair(3); return [Array.isArray(r), r[0], String(r[1])]; }});
step("receiver", () => Object.getOwnPropertyDescriptor(WebAssembly.Instance.prototype, "exports").get.call({{}}));
"#
    ));
    assert_eq!(
        lines,
        [
            "calls=[5,6,-2147483647,0,10]",
            r#"shape=["0",2,"function",true,false,"length,name"]"#,
            r#"string="function 0() {\n    [native code]\n}""#,
            "noNew=threw TypeError: not a constructor",
            r#"same=[true,true,null,["add"],"[object WebAssembly.Instance]"]"#,
            "frozen=threw TypeError: 'add' is read-only",
            "symbol=threw TypeError: cannot convert symbol to number",
            "bigint=threw TypeError: Cannot convert a BigInt value to a number",
            r#"i64=["5","3","-1","bigint","12","1"]"#,
            "i64Number=threw TypeError: Cannot convert 5 to a BigInt",
            r#"floats=[0.10000000149011612,null,"NaN",0.1,2.5]"#,
            r#"pair=[true,3,"7"]"#,
            "receiver=threw TypeError: Receiver is not a WebAssembly.Instance",
        ]
    );
}

#[test]
fn a_trap_is_a_runtime_error_in_the_reference_browsers_words() {
    let traps = module(
        r#"(module
  (memory 1 1)
  (table 2 funcref)
  (type $v (func))
  (func (export "unreachable") unreachable)
  (func (export "divZero") (result i32) i32.const 1 i32.const 0 i32.div_s)
  (func (export "divOverflow") (result i32) i32.const -2147483648 i32.const -1 i32.div_s)
  (func (export "memOOB") (result i32) i32.const 65536 i32.load)
  (func (export "truncNaN") (result i32) f32.const nan i32.trunc_f32_s)
  (func (export "callNull") i32.const 0 call_indirect (type $v))
  (func (export "tableOOB") i32.const 9 call_indirect (type $v))
  (func $deep (export "deep") call $deep)
  (func (export "fine") (result i32) i32.const 42))"#,
    );
    let lines = steps(&format!(
        r#"
const e = new WebAssembly.Instance(new WebAssembly.Module({traps})).exports;
for (const name of ["unreachable", "divZero", "divOverflow", "memOOB", "truncNaN", "callNull", "tableOOB"]) {{
  step(name, () => {{ try {{ e[name](); }} catch (error) {{
    return [error instanceof WebAssembly.RuntimeError, error.message]; }} }});
}}
step("deep", () => e.deep());
step("after", () => e.fine());
"#
    ));
    assert_eq!(
        lines,
        [
            r#"unreachable=[true,"unreachable"]"#,
            r#"divZero=[true,"divide by zero"]"#,
            r#"divOverflow=[true,"divide result unrepresentable"]"#,
            r#"memOOB=[true,"memory access out of bounds"]"#,
            r#"truncNaN=[true,"float unrepresentable in integer range"]"#,
            r#"callNull=[true,"null function"]"#,
            r#"tableOOB=[true,"table index is out of bounds"]"#,
            "deep=threw RangeError: Maximum call stack size exceeded",
            "after=42",
        ]
    );
}

const IMPORTS: &str = r#"(module
  (import "env" "f" (func $f (param i32) (result i32)))
  (func (export "g") (param i32) (result i32) local.get 0 call $f))"#;

#[test]
fn imports_are_read_in_the_modules_order_with_the_reference_browsers_words() {
    let imports = module(IMPORTS);
    let globals = module(
        r#"(module
  (import "env" "c" (global $c i32))
  (import "env" "big" (global $big i64))
  (import "env" "m" (global (mut i32)))
  (func (export "c") (result i32) global.get $c)
  (func (export "big") (result i64) global.get $big))"#,
    );
    let constants = module(
        r#"(module
  (import "env" "c" (global $c i32))
  (import "env" "big" (global $big i64))
  (import "env" "x" (global $x externref))
  (func (export "c") (result i32) global.get $c)
  (func (export "big") (result i64) global.get $big)
  (func (export "x") (result externref) global.get $x))"#,
    );
    let memory = module(r#"(module (import "env" "mem" (memory 1)))"#);
    let lines = steps(&format!(
        r#"
const m = new WebAssembly.Module({imports});
step("missingModule", () => new WebAssembly.Instance(m, {{}}));
step("missingImportsArg", () => new WebAssembly.Instance(m));
step("notObject", () => new WebAssembly.Instance(m, 5));
step("notCallable", () => {{ try {{ new WebAssembly.Instance(m, {{ env: {{ f: 1 }} }}); }}
  catch (e) {{ return [e instanceof WebAssembly.LinkError, e.message]; }} }});
step("call", () => new WebAssembly.Instance(m, {{ env: {{ f: (x) => x * 2 + 0.5 }} }}).exports.g(20));
step("throws", () => {{ const e = new Error("mine"); try {{
  new WebAssembly.Instance(m, {{ env: {{ f: () => {{ throw e; }} }} }}).exports.g(1); }} catch (c) {{ return c === e; }} }});
step("args", () => {{ let seen; new WebAssembly.Instance(m, {{ env: {{ f: function (...a) {{ seen = [this === globalThis, a]; return 0; }} }} }})
  .exports.g(7); return seen; }});
step("getter", () => {{ const order = []; const env = {{ get f() {{ order.push("f"); return () => 1; }} }};
  new WebAssembly.Instance(m, {{ get env() {{ order.push("env"); return env; }} }}); return order; }});
const g = new WebAssembly.Module({globals});
step("mutable", () => new WebAssembly.Instance(g, {{ env: {{ c: 1, big: 5n, m: 1 }} }}));
step("bigNumber", () => new WebAssembly.Instance(g, {{ env: {{ c: 1, big: 5 }} }}));
step("constants", () => {{ const x = {{}}; const e = new WebAssembly.Instance(new WebAssembly.Module({constants}),
  {{ env: {{ c: 41.9, big: 2n ** 63n, x }} }}).exports; return [e.c(), String(e.big()), e.x() === x]; }});
step("memory", () => new WebAssembly.Instance(new WebAssembly.Module({memory}), {{ env: {{ mem: {{}} }} }}));
"#
    ));
    assert_eq!(
        lines,
        [
            r#"missingModule=threw TypeError: WebAssembly.Instance(): Import #0 "env": module is not an object or function"#,
            "missingImportsArg=threw TypeError: WebAssembly.Instance(): Imports argument must be present and must be an object",
            "notObject=threw TypeError: WebAssembly.Instance(): Argument 1 must be an object",
            r#"notCallable=[true,"WebAssembly.Instance(): Import #0 \"env\" \"f\": function import requires a callable"]"#,
            "call=40",
            "throws=true",
            "args=[true,[7]]",
            r#"getter=["env","f"]"#,
            r#"mutable=threw LinkError: WebAssembly.Instance(): Import #2 "env" "m": imported mutable global must be a WebAssembly.Global object"#,
            r#"bigNumber=threw LinkError: WebAssembly.Instance(): Import #1 "env" "big": global import must be a number, valid Wasm reference, or WebAssembly.Global object"#,
            r#"constants=[41,"-9223372036854775808",true]"#,
            r#"memory=threw LinkError: WebAssembly.Instance(): Import #0 "env" "mem": memory import must be a WebAssembly.Memory object"#,
        ]
    );
}

#[test]
fn the_start_function_runs_at_instantiation_and_may_call_the_page() {
    let started = module(
        r#"(module
  (import "env" "log" (func $log (param i32)))
  (global $g (mut i32) (i32.const 0))
  (func $start i32.const 7 call $log i32.const 1 global.set $g)
  (start $start)
  (func (export "started") (result i32) global.get $g))"#,
    );
    let traps = module(r#"(module (func $s unreachable) (start $s))"#);
    let thrower = module(r#"(module (import "env" "f" (func $f)) (start $f))"#);
    let bad = module(r#"(module (func $s (param i32)) (start $s))"#);
    let named = module(
        r#"(module (func $s) (start $s)
  (func (export "silver.start") (result i32) i32.const 5)
  (func (export "silver.first") (result i32) i32.const 6))"#,
    );
    let lines = steps(&format!(
        r#"
const m = new WebAssembly.Module({started});
step("runs", () => {{ const seen = []; const e = new WebAssembly.Instance(m, {{ env: {{ log: (x) => seen.push(x) }} }}).exports;
  return [seen, e.started(), Object.keys(e), WebAssembly.Module.exports(m).map((x) => x.name)]; }});
step("traps", () => {{ try {{ new WebAssembly.Instance(new WebAssembly.Module({traps})); }} catch (e) {{
  return [e instanceof WebAssembly.RuntimeError, e.message]; }} }});
step("throws", () => {{ const mine = new Error("mine"); try {{ new WebAssembly.Instance(new WebAssembly.Module({thrower}),
  {{ env: {{ f() {{ throw mine; }} }} }}); }} catch (e) {{ return e === mine; }} }});
step("bad", () => {{ try {{ new WebAssembly.Module({bad}); }} catch (e) {{
  return [e instanceof WebAssembly.CompileError, e.message.startsWith("WebAssembly.Module(): ")]; }} }});
step("named", () => {{ const e = new WebAssembly.Instance(new WebAssembly.Module({named})).exports;
  return [Object.keys(e), e["silver.start"](), e["silver.first"]()]; }});
"#
    ));
    assert_eq!(
        lines,
        [
            r#"runs=[[7],1,["started"],["started"]]"#,
            r#"traps=[true,"unreachable"]"#,
            "throws=true",
            "bad=[true,true]",
            r#"named=[["silver.start","silver.first"],5,6]"#,
        ]
    );
}

#[test]
fn a_function_is_the_same_object_wherever_it_comes_from_and_named_by_its_index() {
    let tabled = module(
        r#"(module
  (import "env" "f" (func $f (param i32) (result i32)))
  (table $t 4 funcref)
  (elem (i32.const 0) func $a $b $c $f)
  (func $a (result i32) i32.const 1)
  (func $b (result i32) i32.const 2)
  (func $c (export "c") (result i32) i32.const 3)
  (func (export "get") (param i32) (result funcref) local.get 0 table.get $t)
  (func (export "id") (param funcref) (result funcref) local.get 0)
  (export "f" (func $f)))"#,
    );
    let add = module(ADD);
    let reimport = module(
        r#"(module
  (import "env" "f" (func $f (param i32 i32) (result i32)))
  (export "f" (func $f))
  (func (export "call") (param i32 i32) (result i32) local.get 0 local.get 1 call $f))"#,
    );
    let lines = steps(&format!(
        r#"
const e = new WebAssembly.Instance(new WebAssembly.Module({tabled}), {{ env: {{ f: (x) => x + 0.5 }} }}).exports;
step("names", () => [e.get(0).name, e.get(1).name, e.c.name, e.get.name, e.f.name]);
step("same", () => [e.get(2) === e.c, e.get(0) === e.get(0), e.get(3) === e.f, e.get(0)(), e.id(e.c) === e.c, e.id(null)]);
step("notExported", () => e.id(() => 1));
step("hostAgain", () => [e.f(5.9), e.f.length]);
const add = new WebAssembly.Instance(new WebAssembly.Module({add})).exports.add;
step("reimported", () => {{ const r = new WebAssembly.Instance(new WebAssembly.Module({reimport}), {{ env: {{ f: add }} }}).exports;
  return [r.f === add, r.call(2, 3)]; }});
step("mismatch", () => new WebAssembly.Instance(new WebAssembly.Module({imports}), {{ env: {{ f: add }} }}));
"#,
        imports = module(IMPORTS),
    ));
    assert_eq!(
        lines,
        [
            r#"names=["1","2","3","4","0"]"#,
            "same=[true,true,true,1,true,null]",
            "notExported=threw TypeError: type incompatibility when transforming from/to JS",
            "hostAgain=[5,1]",
            "reimported=[true,5]",
            r#"mismatch=threw LinkError: WebAssembly.Instance(): Import #0 "env" "f": imported function does not match the expected type"#,
        ]
    );
}

#[test]
fn calls_between_modules_and_scripts_nest() {
    let nested = module(
        r#"(module
  (import "env" "back" (func $back (param i32) (result i32)))
  (func (export "outer") (param i32) (result i32) local.get 0 call $back i32.const 1 i32.add)
  (func (export "inner") (param i32) (result i32) local.get 0 i32.const 10 i32.mul)
  (func (export "trap") (param i32) (result i32) unreachable))"#,
    );
    let lines = steps(&format!(
        r#"
const m = new WebAssembly.Module({nested});
let e;
let back = (x) => e.inner(x) + 100;
e = new WebAssembly.Instance(m, {{ env: {{ back: (x) => back(x) }} }}).exports;
step("once", () => e.outer(2));
step("deeper", () => {{ back = (x) => (x > 0 ? e.outer(x - 1) : 0); return e.outer(3); }});
step("trapInside", () => {{ back = (x) => e.trap(x); try {{ e.outer(1); }} catch (error) {{
  return [error instanceof WebAssembly.RuntimeError, error.message]; }} }});
step("after", () => {{ back = (x) => x; return e.outer(41); }});
"#
    ));
    assert_eq!(
        lines,
        [
            "once=121",
            "deeper=4",
            r#"trapInside=[true,"unreachable"]"#,
            "after=42",
        ]
    );
}

#[test]
fn external_references_and_several_results_cross_both_ways() {
    let refs = module(
        r#"(module
  (func (export "id") (param externref) (result externref) local.get 0)
  (func (export "isNull") (param externref) (result i32) local.get 0 ref.is_null))"#,
    );
    let two = module(
        r#"(module (import "env" "two" (func $two (result i32 i32)))
  (func (export "sum") (result i32) call $two i32.add))"#,
    );
    let lines = steps(&format!(
        r#"
const e = new WebAssembly.Instance(new WebAssembly.Module({refs})).exports;
const o = {{}};
step("refs", () => [e.id(o) === o, String(e.id(undefined)), e.id(null), e.id(5), e.id("s"), e.isNull(null),
  e.isNull(undefined), Number.isNaN(e.id(NaN)), e.id(o) === o]);
const m = new WebAssembly.Module({two});
const sum = (two) => new WebAssembly.Instance(m, {{ env: {{ two }} }}).exports.sum();
step("array", () => sum(() => [3, 4]));
step("iterable", () => sum(() => new Set([5, 6])));
step("short", () => sum(() => [1]));
step("notIterable", () => sum(() => 5));
"#
    ));
    assert_eq!(
        lines,
        [
            r#"refs=[true,"undefined",null,5,"s",1,0,true,true]"#,
            "array=7",
            "iterable=11",
            "short=threw TypeError: multi-return length mismatch",
            "notIterable=threw TypeError: multi-return value must be iterable",
        ]
    );
}

#[test]
fn a_run_stopped_inside_a_scripts_import_leaves_the_module_working() {
    let mut realm = crate::tests::realm_with(crate::Limits {
        steps: 200,
        ..crate::Limits::default()
    });
    let mut host = Recorder::default();
    let source = format!(
        "var spin = true; var m = new WebAssembly.Module({}); \
         var e = new WebAssembly.Instance(m, {{ env: {{ f: function (x) {{ while (spin) {{}} return x; }} }} }}).exports;",
        module(IMPORTS)
    );
    realm.run(&script(&source), &mut host).expect("it runs");
    let stopped = realm
        .run(&script("e.g(1)"), &mut host)
        .expect_err("it is stopped");
    assert_eq!(stopped.kind, crate::ErrorKind::Budget);
    let after = realm
        .run(&script("spin = false; String(e.g(5))"), &mut host)
        .expect("it runs");
    assert_eq!(after, crate::Completion::String("5".to_owned()));
}

const MEMORY: &str = r#"(module
  (memory (export "mem") 1 3)
  (func (export "load") (param i32) (result i32) local.get 0 i32.load)
  (func (export "store") (param i32 i32) local.get 0 local.get 1 i32.store)
  (func (export "grow") (param i32) (result i32) local.get 0 memory.grow)
  (func (export "size") (result i32) memory.size))"#;

#[test]
fn a_memory_is_shared_with_scripts_through_its_buffer() {
    let memory = module(MEMORY);
    let peek = module(
        r#"(module (import "env" "peek" (func $peek))
  (memory (export "mem") 1)
  (func (export "run") (result i32)
    i32.const 0 i32.const 11 i32.store
    call $peek
    i32.const 4 i32.load))"#,
    );
    let lines = steps(&format!(
        r#"
const e = new WebAssembly.Instance(new WebAssembly.Module({memory})).exports;
step("shared", () => {{ const view = new Int32Array(e.mem.buffer); view[1] = 77; const a = e.load(4); e.store(8, 99);
  return [a, view[2], e.mem instanceof WebAssembly.Memory, e.mem.buffer.byteLength, e.mem === e.mem]; }});
step("far", () => {{ e.store(65532, 123456); const view = new Int32Array(e.mem.buffer);
  view[100] = 5; return [view[16383], e.load(400)]; }});
step("grown", () => {{ const b = e.mem.buffer; const before = e.grow(1);
  return [before, b.byteLength, e.mem.buffer.byteLength, e.mem.buffer === b, e.size(), e.load(4)]; }});
step("after", () => {{ new Uint8Array(e.mem.buffer)[70000] = 5; return [e.load(70000) & 255, e.grow(5)]; }});
let p;
const v = () => new Int32Array(p.mem.buffer);
p = new WebAssembly.Instance(new WebAssembly.Module({peek}), {{ env: {{ peek() {{ seen = v()[0]; v()[1] = 22; }} }} }}).exports;
let seen;
step("midCall", () => {{ const r = p.run(); return [seen, r]; }});
"#
    ));
    assert_eq!(
        lines,
        [
            "shared=[77,99,true,65536,true]",
            "far=[123456,5]",
            "grown=[1,0,131072,false,2,77]",
            "after=[5,-1]",
            "midCall=[11,22]",
        ]
    );
}

#[test]
fn memory_objects_answer_as_the_reference_browsers() {
    let imported = module(
        r#"(module (import "env" "mem" (memory 1))
  (data (i32.const 0) "hi")
  (export "mem" (memory 0))
  (func (export "load8") (param i32) (result i32) local.get 0 i32.load8_u))"#,
    );
    let larger = module(r#"(module (import "env" "mem" (memory 2)))"#);
    let owner = module(
        r#"(module (memory (export "mem") 1) (table (export "t") 1 funcref)
  (global (export "g") (mut i32) (i32.const 3)))"#,
    );
    let again = module(
        r#"(module (import "env" "mem" (memory 1)) (import "env" "t" (table 1 funcref))
  (import "env" "g" (global (mut i32)))
  (export "mem" (memory 0)) (export "t" (table 0)) (export "g" (global 0)))"#,
    );
    let lines = steps(&format!(
        r#"
step("memory", () => {{
  const m = new WebAssembly.Memory({{ initial: 1, maximum: 3 }});
  const b1 = m.buffer;
  const r = [b1.byteLength, m.buffer === b1, m.grow(1), b1.byteLength, m.buffer.byteLength, m.buffer === b1];
  try {{ m.grow(5); }} catch (e) {{ r.push(e.constructor.name + ": " + e.message); }}
  return r;
}});
step("memoryTooBig", () => new WebAssembly.Memory({{ initial: 70000 }}));
step("none", () => new WebAssembly.Memory());
step("noInitial", () => new WebAssembly.Memory({{}}));
step("nan", () => new WebAssembly.Memory({{ initial: NaN }}));
step("below", () => new WebAssembly.Memory({{ initial: 2, maximum: 1 }}));
step("shared", () => new WebAssembly.Memory({{ initial: 1, maximum: 1, shared: true }}));
step("negative", () => new WebAssembly.Memory({{ initial: 1 }}).grow(-1));
step("growZero", () => {{ const m = new WebAssembly.Memory({{ initial: 1 }}); const b = m.buffer; const r = m.grow(0);
  return [r, b.byteLength, m.buffer.byteLength]; }});
step("imported", () => {{ const m = new WebAssembly.Memory({{ initial: 1 }}); const before = new Uint8Array(m.buffer);
  before[10] = 42; const e = new WebAssembly.Instance(new WebAssembly.Module({imported}), {{ env: {{ mem: m }} }}).exports;
  return [e.mem === m, before[0], before[1], e.load8(10)]; }});
step("reexported", () => {{ const a = new WebAssembly.Instance(new WebAssembly.Module({owner})).exports;
  const b = new WebAssembly.Instance(new WebAssembly.Module({again}), {{ env: a }}).exports;
  return [b.mem === a.mem, b.t === a.t, b.g === a.g, a.mem instanceof WebAssembly.Memory]; }});
step("tooSmall", () => {{ try {{ new WebAssembly.Instance(new WebAssembly.Module({larger}),
  {{ env: {{ mem: new WebAssembly.Memory({{ initial: 1 }}) }} }}); }} catch (e) {{ return e.constructor.name; }} }});
step("receiver", () => Object.getOwnPropertyDescriptor(WebAssembly.Memory.prototype, "buffer").get.call({{}}));
step("tags", () => ["Module", "Instance", "Memory", "Table", "Global", "CompileError", "LinkError", "RuntimeError"]
  .map((n) => Object.prototype.toString.call(WebAssembly[n].prototype)));
"#
    ));
    assert_eq!(
        lines,
        [
            r#"memory=[65536,true,1,0,131072,false,"RangeError: WebAssembly.Memory.grow(): Maximum memory size exceeded"]"#,
            "memoryTooBig=threw RangeError: WebAssembly.Memory(): Property 'initial': value 70000 is above the upper bound 65536",
            "none=threw TypeError: WebAssembly.Memory(): Argument 0 must be a memory descriptor",
            "noInitial=threw TypeError: WebAssembly.Memory(): Property 'initial' is required",
            "nan=threw TypeError: WebAssembly.Memory(): Property 'initial' must be convertible to a valid number",
            "below=threw RangeError: WebAssembly.Memory(): Property 'maximum': value 1 is below the lower bound 2",
            "shared=threw TypeError: WebAssembly.Memory(): shared memories are not supported",
            "negative=threw TypeError: WebAssembly.Memory.grow(): Argument 0 must be in the unsigned long range",
            "growZero=[1,0,65536]",
            "imported=[true,104,105,42]",
            "reexported=[true,true,true,true]",
            r#"tooSmall="LinkError""#,
            "receiver=threw TypeError: WebAssembly.Memory.buffer: Receiver is not a WebAssembly.Memory",
            r#"tags=["[object WebAssembly.Module]","[object WebAssembly.Instance]","[object WebAssembly.Memory]","[object WebAssembly.Table]","[object WebAssembly.Global]","[object Object]","[object Object]","[object Object]"]"#,
        ]
    );
}

#[test]
fn globals_hold_values_shared_with_modules() {
    let add = module(ADD);
    let shared = module(
        r#"(module (import "env" "g" (global $g (mut i32)))
  (global (export "own") (mut f64) (f64.const 1.5))
  (export "g" (global $g))
  (func (export "bump") global.get $g i32.const 1 i32.add global.set $g)
  (func (export "read") (result i32) global.get $g))"#,
    );
    let lines = steps(&format!(
        r#"
step("global", () => {{
  const g = new WebAssembly.Global({{ value: "i32", mutable: true }}, 42);
  const r = [g.value, g.valueOf()];
  g.value = 7.9;
  r.push(g.value);
  const c = new WebAssembly.Global({{ value: "i32" }}, 1);
  try {{ c.value = 2; }} catch (e) {{ r.push(e.constructor.name + ": " + e.message); }}
  const l = new WebAssembly.Global({{ value: "i64" }}, 5n);
  r.push(typeof l.value, String(new WebAssembly.Global({{ value: "f32" }}, 0.1).value));
  return r;
}});
const m = new WebAssembly.Module({shared});
step("shared", () => {{ const g = new WebAssembly.Global({{ value: "i32", mutable: true }}, 5);
  const e = new WebAssembly.Instance(m, {{ env: {{ g }} }}).exports; e.bump(); const a = g.value; g.value = 40; e.bump();
  return [a, e.read(), e.g === g, e.own instanceof WebAssembly.Global, e.own.value, String(e.own), e.own === e.own]; }});
step("constant", () => {{ try {{ new WebAssembly.Instance(m, {{ env: {{ g: new WebAssembly.Global({{ value: "i32" }}, 1) }} }}); }}
  catch (e) {{ return e.constructor.name; }} }});
step("wrongType", () => {{ try {{ new WebAssembly.Instance(m, {{ env: {{ g: new WebAssembly.Global({{ value: "i64", mutable: true }}) }} }}); }}
  catch (e) {{ return e.constructor.name; }} }});
const add = new WebAssembly.Instance(new WebAssembly.Module({add})).exports.add;
step("defaults", () => [String(new WebAssembly.Global({{ value: "i64" }}).value), new WebAssembly.Global({{ value: "f64" }}).value,
  String(new WebAssembly.Global({{ value: "externref" }}).value), new WebAssembly.Global({{ value: "anyfunc" }}).value,
  new WebAssembly.Global({{ value: "anyfunc" }}, add).value === add]);
step("v128", () => new WebAssembly.Global({{ value: "v128" }}));
step("i64Number", () => new WebAssembly.Global({{ value: "i64" }}, 5));
step("notFunction", () => new WebAssembly.Global({{ value: "anyfunc" }}, () => 1));
"#
    ));
    assert_eq!(
        lines,
        [
            r#"global=[42,42,7,"TypeError: set WebAssembly.Global.value): Can't set the value of an immutable global.","bigint","0.10000000149011612"]"#,
            r#"shared=[6,41,true,true,1.5,"[object WebAssembly.Global]",true]"#,
            r#"constant="LinkError""#,
            r#"wrongType="LinkError""#,
            r#"defaults=["0",0,"undefined",null,true]"#,
            "v128=threw TypeError: WebAssembly.Global(): Descriptor property 'value' must be a WebAssembly type",
            "i64Number=threw TypeError: Cannot convert 5 to a BigInt",
            "notFunction=threw TypeError: type incompatibility when transforming from/to JS",
        ]
    );
}

#[test]
fn tables_hold_references_shared_with_modules() {
    let add = module(ADD);
    let indirect = module(
        r#"(module (import "env" "t" (table 2 funcref))
  (type $r (func (result i32)))
  (table $own (export "tbl") 1 externref)
  (func (export "call") (param i32) (result i32) local.get 0 call_indirect (type $r)))"#,
    );
    let answer = module(r#"(module (func (export "f") (result i32) i32.const 42))"#);
    let lines = steps(&format!(
        r#"
const add = new WebAssembly.Instance(new WebAssembly.Module({add})).exports.add;
step("table", () => {{
  const t = new WebAssembly.Table({{ element: "anyfunc", initial: 2 }});
  const r = [t.length, t.get(0), t.set(0, add), t.get(0) === add, t.grow(2), t.length];
  try {{ t.get(9); }} catch (e) {{ r.push(e.constructor.name + ": " + e.message); }}
  try {{ t.set(0, () => 1); }} catch (e) {{ r.push(e.constructor.name + ": " + e.message); }}
  return r;
}});
const f = new WebAssembly.Instance(new WebAssembly.Module({answer})).exports.f;
step("indirect", () => {{ const t = new WebAssembly.Table({{ element: "anyfunc", initial: 2 }}); t.set(1, f);
  const e = new WebAssembly.Instance(new WebAssembly.Module({indirect}), {{ env: {{ t }} }}).exports;
  let trapped; try {{ e.call(0); }} catch (error) {{ trapped = error.message; }}
  return [e.call(1), trapped, e.tbl === e.tbl, e.tbl instanceof WebAssembly.Table, e.tbl.length]; }});
step("externs", () => {{ const o = {{}}; const t = new WebAssembly.Table({{ element: "externref", initial: 2 }}, "x");
  const r = [t.get(0), t.get(1)]; t.set(0, o); r.push(t.get(0) === o, t.grow(1), String(t.get(2))); return r; }});
step("full", () => new WebAssembly.Table({{ element: "anyfunc", initial: 1, maximum: 1 }}).grow(1));
step("element", () => new WebAssembly.Table({{ element: "i32", initial: 1 }}));
step("large", () => new WebAssembly.Table({{ element: "anyfunc", initial: 10000001 }}));
"#
    ));
    assert_eq!(
        lines,
        [
            r#"table=[2,null,null,true,2,4,"RangeError: WebAssembly.Table.get(): invalid address 9 in funcref table of size 4","TypeError: WebAssembly.Table.set(): Argument 1 is invalid for table: function-typed object must be null (if nullable) or a Wasm function object"]"#,
            r#"indirect=[42,"null function",true,true,1]"#,
            r#"externs=["x","x",true,2,"undefined"]"#,
            "full=threw RangeError: WebAssembly.Table.grow(): failed to grow table by 1",
            "element=threw TypeError: WebAssembly.Table(): Descriptor property 'element' must be a WebAssembly reference type",
            "large=threw RangeError: WebAssembly.Table(): Property 'initial': value 10000001 is above the upper bound 10000000",
        ]
    );
}

#[test]
fn a_module_that_loops_is_stopped_as_a_script_that_loops_is() {
    let mut realm = crate::tests::realm_with(crate::Limits {
        steps: 200,
        ..crate::Limits::default()
    });
    let mut host = Recorder::default();
    let looping = module(
        r#"(module
  (func (export "spin") (loop br 0))
  (func (export "count") (param i32) (result i32) (local i32)
    (loop
      local.get 1 i32.const 1 i32.add local.set 1
      local.get 1 local.get 0 i32.lt_u br_if 0)
    local.get 1))"#,
    );
    let starting = module(r#"(module (func $s (loop br 0)) (start $s))"#);
    let source = format!(
        "var e = new WebAssembly.Instance(new WebAssembly.Module({looping})).exports; \
         var starting = new WebAssembly.Module({starting});"
    );
    realm.run(&script(&source), &mut host).expect("it runs");
    let counted = realm
        .run(&script("String(e.count(100000))"), &mut host)
        .expect("it runs");
    assert_eq!(counted, crate::Completion::String("100000".to_owned()));
    for source in [
        "e.spin()",
        "try { e.spin(); } catch (error) {} 'caught'",
        "e.count(2000000000)",
        // Each call within the budget, together past it.
        "for (let i = 0; i < 1000; i++) { e.count(1000000); }",
        "new WebAssembly.Instance(starting)",
    ] {
        let stopped = realm
            .run(&script(source), &mut host)
            .expect_err("it is stopped");
        assert_eq!(stopped.kind, crate::ErrorKind::Budget, "{source}");
    }
    let after = realm
        .run(&script("String(e.count(3))"), &mut host)
        .expect("it runs");
    assert_eq!(after, crate::Completion::String("3".to_owned()));
}

#[test]
fn what_modules_allocate_is_bounded() {
    let big = module(r#"(module (memory 4096))"#);
    let grower = module(MEMORY);
    let empty = module(r#"(module)"#);
    let lines = steps(&format!(
        r#"
step("tooMuch", () => new WebAssembly.Memory({{ initial: 4097 }}));
const mem = new WebAssembly.Memory({{ initial: 1 }});
step("total", () => {{ try {{ new WebAssembly.Instance(new WebAssembly.Module({big})); }} catch (e) {{ return e.constructor.name; }} }});
const g = new WebAssembly.Instance(new WebAssembly.Module({grower})).exports;
step("grow", () => [g.grow(1), g.grow(4096), g.size()]);
step("jsGrow", () => mem.grow(4095));
step("table", () => new WebAssembly.Table({{ element: "anyfunc", initial: 1 }}).grow(10000000));
const e = new WebAssembly.Module({empty});
step("instances", () => {{ let n = 0; try {{ for (;;) {{ new WebAssembly.Instance(e); n++; }} }}
  catch (error) {{ return [n > 990, n < 1000, error.constructor.name]; }} }});
step("still", () => g.size());
"#
    ));
    assert_eq!(
        lines,
        [
            "tooMuch=threw RangeError: WebAssembly.Memory(): could not allocate memory",
            r#"total="RangeError""#,
            "grow=[1,-1,2]",
            "jsGrow=threw RangeError: WebAssembly.Memory.grow(): Maximum memory size exceeded",
            "table=threw RangeError: WebAssembly.Table.grow(): failed to grow table by 10000000",
            r#"instances=[true,true,"RangeError"]"#,
            "still=2",
        ]
    );
}

#[test]
fn calls_nest_only_so_deep() {
    let nested = module(
        r#"(module
  (import "env" "back" (func $back (param i32) (result i32)))
  (func (export "outer") (param i32) (result i32) local.get 0 call $back i32.const 1 i32.add))"#,
    );
    let deep = module(
        r#"(module (func $deep (export "deep") (param i32) (result i32)
  local.get 0 i32.eqz
  if (result i32) i32.const 0
  else local.get 0 i32.const 1 i32.sub call $deep i32.const 1 i32.add end))"#,
    );
    let lines = steps(&format!(
        r#"
let e;
let back;
e = new WebAssembly.Instance(new WebAssembly.Module({nested}), {{ env: {{ back: (x) => back(x) }} }}).exports;
step("nested", () => {{ back = (x) => e.outer(x); try {{ e.outer(1); }} catch (error) {{
  return [error.constructor.name, error.message]; }} }});
step("after", () => {{ back = (x) => x; return e.outer(41); }});
step("abandoned", () => {{ back = () => {{ throw new Error("x"); }};
  for (let i = 0; i < 100; i++) {{ try {{ e.outer(1); }} catch (error) {{}} }}
  back = (x) => (x > 0 ? e.outer(x - 1) : 0); return e.outer(4); }});
step("overflows", () => {{ back = (x) => e.outer(x);
  for (let i = 0; i < 100; i++) {{ try {{ e.outer(1); }} catch (error) {{}} }}
  back = (x) => (x > 0 ? e.outer(x - 1) : 0); return e.outer(4); }});
step("dives", () => {{ back = (x) => e.outer(x);
  const dive = (n) => {{ try {{ e.outer(1); }} catch (error) {{}} try {{ dive(n + 1); }} catch (error) {{}} }};
  for (let i = 0; i < 3; i++) {{ dive(0); }}
  back = (x) => (x > 0 ? e.outer(x - 1) : 0); return e.outer(4); }});
const d = new WebAssembly.Instance(new WebAssembly.Module({deep})).exports;
step("recursion", () => d.deep(5000));
step("tooDeep", () => d.deep(20000));
"#
    ));
    assert_eq!(
        lines,
        [
            r#"nested=["RangeError","Maximum call stack size exceeded"]"#,
            "after=42",
            "abandoned=5",
            "overflows=5",
            "dives=5",
            "recursion=5000",
            "tooDeep=threw RangeError: Maximum call stack size exceeded",
        ]
    );
}
