//! WebAssembly in the realm (M29, decision D21; see
//! `docs/design/wasm.md`): the `wasmi` store a realm's modules live in,
//! made at its first module, and the natives the bootstrap's
//! `WebAssembly` classes call. A module's bytes cross from the realm as
//! base64, which QuickJS-ng's `Uint8Array.prototype.toBase64` makes at
//! the speed of C: `rquickjs` lends no buffer's bytes safely.

use std::cell::RefCell;
use std::rc::Rc;

use rquickjs::function::Rest;
use rquickjs::{Array, ArrayBuffer, Ctx, Exception, Function, Object, Value};
use wasmi::{CompilationMode, Config, Engine, Module, Store};

use super::dom::Budget;

/// The largest module a realm compiles.
pub(crate) const MAX_WASM_MODULE_BYTES: usize = 64 << 20;

/// What a realm's modules are made in.
pub(super) struct Wasm {
    engine: Engine,
    #[allow(dead_code)]
    store: Store<()>,
    /// The modules compiled, by their handles.
    modules: Vec<Compiled>,
}

/// A module compiled, and what the interface reads of it in the
/// module's own order, which `wasmi` keeps not (it has its exports by
/// name and its imports by kind).
struct Compiled {
    module: Module,
    /// Each import's module, name and kind.
    imports: Vec<(String, String, &'static str)>,
    /// Each export's name and kind.
    exports: Vec<(String, &'static str)>,
}

impl Compiled {
    /// `module`, compiled from `bytes`, which it validated: its sections
    /// read again for their order.
    fn new(module: Module, bytes: &[u8]) -> Compiled {
        use wasmparser::{ExternalKind, Parser, Payload, TypeRef};
        let mut compiled = Compiled {
            module,
            imports: Vec::new(),
            exports: Vec::new(),
        };
        for payload in Parser::new(0).parse_all(bytes) {
            match payload {
                Ok(Payload::ImportSection(reader)) => {
                    for import in reader.into_iter().flatten() {
                        let kind = match import.ty {
                            TypeRef::Func(_) => "function",
                            TypeRef::Table(_) => "table",
                            TypeRef::Memory(_) => "memory",
                            TypeRef::Global(_) => "global",
                            TypeRef::Tag(_) => "tag",
                        };
                        compiled.imports.push((
                            import.module.to_owned(),
                            import.name.to_owned(),
                            kind,
                        ));
                    }
                }
                Ok(Payload::ExportSection(reader)) => {
                    for export in reader.into_iter().flatten() {
                        let kind = match export.kind {
                            ExternalKind::Func => "function",
                            ExternalKind::Table => "table",
                            ExternalKind::Memory => "memory",
                            ExternalKind::Global => "global",
                            ExternalKind::Tag => "tag",
                        };
                        compiled.exports.push((export.name.to_owned(), kind));
                    }
                }
                _ => {}
            }
        }
        compiled
    }
}

/// A realm's WebAssembly, made at its first use.
type Shared = Rc<RefCell<Option<Wasm>>>;

impl Wasm {
    fn new() -> Wasm {
        let mut config = Config::default();
        // The core 2.0 features, and 3.0's tail calls, extended
        // constant expressions and multiple memories, as `wasmi` has
        // them; not SIMD or memory64, which this build leaves out.
        config
            .wasm_custom_page_sizes(false)
            .wasm_wide_arithmetic(false)
            .compilation_mode(CompilationMode::LazyTranslation);
        let engine = Engine::new(&config);
        let store = Store::new(&engine, ());
        Wasm {
            engine,
            store,
            modules: Vec::new(),
        }
    }

    fn module(&self, handle: f64) -> Option<&Compiled> {
        self.modules.get(index(handle)?)
    }
}

/// A handle's index: a whole number, not negative.
fn index(handle: f64) -> Option<usize> {
    (handle.fract() == 0.0 && handle >= 0.0 && handle < u32::MAX as f64).then_some(handle as usize)
}

/// `f` of the realm's WebAssembly, made if this is its first use.
fn with<R>(wasm: &Shared, f: impl FnOnce(&mut Wasm) -> R) -> R {
    let mut wasm = wasm.borrow_mut();
    f(wasm.get_or_insert_with(Wasm::new))
}

/// The bytes `text`, base64 as `toBase64` writes it, encode; `None` for
/// text that is not.
pub(crate) fn from_base64(text: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u32> {
        Some(u32::from(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        }))
    }
    let text = text.as_bytes();
    if !text.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    for chunk in text.chunks(4) {
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 {
            return None;
        }
        let mut word = 0u32;
        for &c in &chunk[..4 - pad] {
            word = word << 6 | value(c)?;
        }
        word <<= 6 * pad as u32;
        let bytes = [(word >> 16) as u8, (word >> 8) as u8, word as u8];
        out.extend_from_slice(&bytes[..3 - pad]);
    }
    Some(out)
}

/// A module's bytes from the realm, within [`MAX_WASM_MODULE_BYTES`],
/// charged to the run's budget as they are read.
fn module_bytes(
    ctx: &Ctx<'_>,
    budget: &Budget,
    text: &str,
    method: &str,
) -> rquickjs::Result<Vec<u8>> {
    let bytes =
        from_base64(text).ok_or_else(|| Exception::throw_type(ctx, "not a module's bytes"))?;
    if bytes.len() > MAX_WASM_MODULE_BYTES {
        return Err(Exception::throw_range(
            ctx,
            &format!(
                "{method}: the module is larger than {} MiB",
                MAX_WASM_MODULE_BYTES >> 20
            ),
        ));
    }
    budget.charge(bytes.len() as u64 / 16);
    Ok(bytes)
}

/// The natives of the `WebAssembly` classes: `wasmValidate(bytes)`,
/// whether bytes are a valid module; `wasmCompile(bytes, method)`, a
/// module's handle, or why it is none, a `CompileError`'s message;
/// `wasmModuleExports(module)`, its exports' names and kinds in turn;
/// `wasmModuleImports(module)`, its imports' modules, names and kinds;
/// `wasmModuleCustomSections(module, name)`, the custom sections of
/// that name, as buffers.
pub(super) fn install<'js>(
    ctx: &Ctx<'js>,
    natives: &Object<'js>,
    budget: &Budget,
) -> rquickjs::Result<()> {
    let wasm: Shared = Rc::default();
    natives.set("wasmModuleLimit", MAX_WASM_MODULE_BYTES as f64)?;
    let (shared, charge) = (wasm.clone(), budget.clone());
    natives.set(
        "wasmValidate",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, bytes: String| -> rquickjs::Result<bool> {
                let bytes = module_bytes(&ctx, &charge, &bytes, "WebAssembly.validate()")?;
                Ok(with(&shared, |wasm| {
                    Module::validate(&wasm.engine, &bytes).is_ok()
                }))
            },
        )?,
    )?;
    let (shared, charge) = (wasm.clone(), budget.clone());
    natives.set(
        "wasmCompile",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, bytes: String, method: String| -> rquickjs::Result<Value<'js>> {
                let bytes = module_bytes(&ctx, &charge, &bytes, &method)?;
                with(&shared, |wasm| match Module::new(&wasm.engine, &bytes) {
                    Ok(module) => {
                        wasm.modules.push(Compiled::new(module, &bytes));
                        Ok(Value::new_number(
                            ctx.clone(),
                            (wasm.modules.len() - 1) as f64,
                        ))
                    }
                    Err(error) => {
                        Ok(rquickjs::String::from_str(ctx.clone(), &error.to_string())?
                            .into_value())
                    }
                })
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmModuleExports",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, handle: f64| -> rquickjs::Result<Array<'js>> {
                let out = Array::new(ctx.clone())?;
                with(&shared, |wasm| {
                    let module = wasm
                        .module(handle)
                        .ok_or_else(|| Exception::throw_type(&ctx, "not a module"))?;
                    for (i, (name, kind)) in module.exports.iter().enumerate() {
                        out.set(2 * i, name.as_str())?;
                        out.set(2 * i + 1, *kind)?;
                    }
                    Ok::<_, rquickjs::Error>(())
                })?;
                Ok(out)
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmModuleImports",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, handle: f64| -> rquickjs::Result<Array<'js>> {
                let out = Array::new(ctx.clone())?;
                with(&shared, |wasm| {
                    let module = wasm
                        .module(handle)
                        .ok_or_else(|| Exception::throw_type(&ctx, "not a module"))?;
                    for (i, (from, name, kind)) in module.imports.iter().enumerate() {
                        out.set(3 * i, from.as_str())?;
                        out.set(3 * i + 1, name.as_str())?;
                        out.set(3 * i + 2, *kind)?;
                    }
                    Ok::<_, rquickjs::Error>(())
                })?;
                Ok(out)
            },
        )?,
    )?;
    let shared = wasm;
    natives.set(
        "wasmModuleCustomSections",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<Array<'js>> {
                let handle = args.0.first().and_then(Value::as_number).unwrap_or(-1.0);
                let name = match args.0.get(1).and_then(Value::as_string) {
                    Some(name) => name.to_string()?,
                    None => return Err(Exception::throw_type(&ctx, "not a name")),
                };
                let out = Array::new(ctx.clone())?;
                with(&shared, |wasm| {
                    let module = wasm
                        .module(handle)
                        .ok_or_else(|| Exception::throw_type(&ctx, "not a module"))?;
                    let mut n = 0;
                    for section in module.module.custom_sections() {
                        if section.name() == name {
                            out.set(n, ArrayBuffer::new(ctx.clone(), section.data().to_vec())?)?;
                            n += 1;
                        }
                    }
                    Ok::<_, rquickjs::Error>(())
                })?;
                Ok(out)
            },
        )?,
    )?;
    Ok(())
}
