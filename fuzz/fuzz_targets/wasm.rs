//! WebAssembly in a realm (M29): modules the input makes, valid ones by
//! `wasm-smith`, of the features the interpreter has, and any bytes
//! besides, compiled, instantiated with imports of every kind, each
//! export called with arguments the input gives, or used as its kind
//! is, under small limits. Nothing panics; every failure a script sees
//! is one of the interface's errors (anything else is logged, and
//! fails the input); a run ends, by completing or being stopped by its
//! budget; and the realm runs a script after.

#![no_main]

use arbitrary::Unstructured;
use js::{ConsoleLevel, ErrorKind, Limits, QuickJs, Script, ScriptEngine, ScriptingHost};
use libfuzzer_sys::fuzz_target;

#[derive(Default)]
struct Recorder {
    lines: Vec<String>,
}

impl ScriptingHost for Recorder {
    fn console(&mut self, _: ConsoleLevel, message: String) {
        self.lines.push(message);
    }

    fn title(&mut self) -> String {
        String::new()
    }

    fn set_title(&mut self, _: String) {}
}

/// Small limits, so the time goes into the modules; the stack a page's.
const LIMITS: Limits = Limits {
    heap: 16 << 20,
    steps: 200,
    stack: js::MAX_SCRIPT_STACK,
};

/// What a script does with `bytes` and `args`: the module compiled,
/// instantiated with what its imports ask for, each export used; what
/// throws, if not one of the interface's errors, logged as unexpected.
const DRIVE: &str = r#"
const allowed = (e) => e instanceof WebAssembly.CompileError || e instanceof WebAssembly.LinkError ||
  e instanceof WebAssembly.RuntimeError || e instanceof RangeError || e instanceof TypeError;
const check = (e) => { if (!allowed(e)) { console.log("unexpected " + e); } };
const imported = (...a) => a[0];
let module;
try { module = new WebAssembly.Module(bytes); } catch (e) { check(e); }
if (module) {
  WebAssembly.Module.exports(module);
  WebAssembly.Module.customSections(module, "name");
  const imports = {};
  for (const i of WebAssembly.Module.imports(module)) {
    const from = imports[i.module] || (imports[i.module] = {});
    if (i.kind === "function") { from[i.name] = imported; }
    if (i.kind === "memory") { from[i.name] = new WebAssembly.Memory({ initial: 1, maximum: 2 }); }
    if (i.kind === "table") { from[i.name] = new WebAssembly.Table({ element: "anyfunc", initial: 1 }); }
    if (i.kind === "global") { from[i.name] = 0; }
  }
  let instance;
  try { instance = new WebAssembly.Instance(module, imports); } catch (e) { check(e); }
  if (instance) {
    for (const name of Object.keys(instance.exports)) {
      const value = instance.exports[name];
      try {
        if (typeof value === "function") {
          try { value(...args); } catch (e) { check(e); }
          value(...args.map(BigInt));
        } else if (value instanceof WebAssembly.Memory) {
          new Uint8Array(value.buffer)[0] = 1;
          value.grow(1);
        } else if (value instanceof WebAssembly.Table) {
          value.get(0);
          value.grow(1);
        } else {
          value.value = value.value;
        }
      } catch (e) { check(e); }
    }
  }
}
"#;

/// A module of the input's making: `wasm-smith`'s, of the core 2.0
/// features, tail calls, extended constants and several memories, kept
/// small; or the input's bytes as they come.
fn module(u: &mut Unstructured<'_>) -> Option<Vec<u8>> {
    if u.arbitrary::<bool>().ok()? {
        let rest = u.len();
        return u.bytes(rest).ok().map(<[u8]>::to_vec);
    }
    let mut config = wasm_smith::Config::default();
    config.simd_enabled = false;
    config.relaxed_simd_enabled = false;
    config.threads_enabled = false;
    config.exceptions_enabled = false;
    config.gc_enabled = false;
    config.memory64_enabled = false;
    config.export_everything = true;
    config.max_memories = 2;
    config.max_memory32_bytes = 1 << 20;
    config.max_tables = 2;
    config.max_table_elements = 1_000;
    config.max_funcs = 20;
    config.max_instructions = 200;
    wasm_smith::Module::new(config, u)
        .ok()
        .map(|module| module.to_bytes())
}

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let count = u.int_in_range(0..=4).unwrap_or(0);
    let args: Vec<String> = (0..count)
        .map(|_| u.arbitrary::<i32>().unwrap_or(0).to_string())
        .collect();
    let Some(bytes) = module(&mut u) else {
        return;
    };
    let list: Vec<String> = bytes.iter().map(u8::to_string).collect();
    let source = format!(
        "const bytes = new Uint8Array([{}]); const args = [{}];{DRIVE}",
        list.join(","),
        args.join(",")
    );
    let mut realm = QuickJs.realm::<Recorder>(LIMITS).expect("a realm");
    let mut host = Recorder::default();
    let script = Script {
        source: &source,
        url: "https://example.com/fuzz.html",
        line: 1,
        muted: false,
    };
    if let Err(error) = realm.run(&script, &mut host) {
        assert!(
            matches!(error.kind, ErrorKind::Budget | ErrorKind::Heap),
            "{error:?}"
        );
    }
    realm.checkpoint(&mut host);
    assert!(
        host.lines
            .iter()
            .all(|line| !line.starts_with("unexpected")),
        "{:?}",
        host.lines
    );
    let after = Script {
        source: "String(typeof WebAssembly)",
        ..script
    };
    if realm.dead().is_none() {
        realm
            .run(&after, &mut host)
            .expect("the realm runs a script after");
    }
});
