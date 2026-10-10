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
