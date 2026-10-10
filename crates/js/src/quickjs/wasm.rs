//! WebAssembly in the realm (M29, decision D21; see
//! `docs/design/wasm.md`): the `wasmi` store a realm's modules live in,
//! made at its first module, and the natives the bootstrap's
//! `WebAssembly` classes call. A module's bytes cross from the realm as
//! base64, which QuickJS-ng's `Uint8Array.prototype.toBase64` makes at
//! the speed of C: `rquickjs` lends no buffer's bytes safely.
//!
//! A module calls a script's function by `wasmi`'s resumable call: the
//! host function made for the script's callable stops the module there
//! ([`CallJs`]), the native gives its arguments to the bootstrap, which
//! calls the callable with no borrow of the store held and resumes the
//! module with what it returned. So that function may call into modules
//! again, and nothing calls the realm from inside `wasmi`. A call's
//! answer to the bootstrap is an array: `[0, results...]` when it
//! returned, `[1, id, slot, arguments...]` when it stopped at a script's
//! function, `[2, class, message]` when it failed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use rquickjs::function::Rest;
use rquickjs::{Array, ArrayBuffer, Ctx, Exception, Function, Object, Value};
use wasmi::errors::{ErrorKind, HostError, InstantiationError};
use wasmi::{
    CompilationMode, Config, Engine, Error, Extern, ExternRef, ExternType, F32, F64, Func,
    FuncType, Global, Instance, Module, Mutability, Nullable, ResumableCall, ResumableCallHostTrap,
    Store, TrapCode, Val, ValType,
};

use super::dom::Budget;

/// The largest module a realm compiles.
pub(crate) const MAX_WASM_MODULE_BYTES: usize = 64 << 20;

/// What a realm's modules are made in.
pub(super) struct Wasm {
    engine: Engine,
    store: Store<()>,
    /// The modules compiled, by their handles.
    modules: Vec<Compiled>,
    /// The instances made, by their handles, each with its module's.
    instances: Vec<(Instance, usize)>,
    /// The functions a script has been given, by their keys ([`key`]).
    funcs: HashMap<u32, Func>,
    /// The host functions made for scripts' callables, by their keys:
    /// each one's slot in the bootstrap and its index in its module.
    hosts: HashMap<u32, (u32, u32)>,
    /// Each instance's defined functions: the key of the first, how many,
    /// and the first's index in its module; in the order of their keys.
    defined: Vec<(u32, u32, u32)>,
    /// The external references made, by the index each holds of the
    /// bootstrap's values.
    externs: Vec<ExternRef>,
    /// The calls stopped at a script's function, by their ids.
    pending: Vec<Option<Pending>>,
}

/// A call stopped at a script's function, and where its results go.
struct Pending {
    call: ResumableCallHostTrap,
    outputs: Vec<Val>,
}

/// What a host function made for a script's callable gives `wasmi` when
/// a module calls it: the call stops there, with the callable's slot and
/// the arguments.
#[derive(Debug)]
struct CallJs {
    slot: u32,
    params: Vec<Val>,
}

impl fmt::Display for CallJs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a call of the script's function {}", self.slot)
    }
}

impl HostError for CallJs {}

/// A module's section as its bytes have it: its id, where it begins, and
/// where its contents begin and end.
struct Section {
    id: u8,
    at: usize,
    start: usize,
    end: usize,
}

/// What the interface and the bridge read of a module's bytes, in the
/// module's own order, which `wasmi` keeps not (it has its exports by
/// name and its imports by kind).
#[derive(Default)]
struct Shape {
    /// Each import's module, name and kind.
    imports: Vec<(String, String, &'static str)>,
    /// Each export's name and kind.
    exports: Vec<(String, &'static str)>,
    /// How many functions it imports, and how many it defines.
    imported_funcs: u32,
    defined_funcs: u32,
    /// Its start function's index.
    start: Option<u32>,
    sections: Vec<Section>,
}

/// A WebAssembly integer in the variable-length form, and its length.
fn leb(bytes: &[u8]) -> Option<(u32, usize)> {
    let mut value = 0u32;
    for (i, &byte) in bytes.iter().enumerate().take(5) {
        value |= u32::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
    }
    None
}

fn put_leb(out: &mut Vec<u8>, mut value: u32) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

impl Shape {
    /// What `bytes` say; `None` for bytes that are no module.
    fn read(bytes: &[u8]) -> Option<Shape> {
        use wasmparser::{ExternalKind, Parser, Payload, TypeRef};
        let mut shape = Shape::default();
        let mut at = 8;
        while at < bytes.len() {
            let (size, n) = leb(bytes.get(at + 1..)?)?;
            let start = at + 1 + n;
            let end = start
                .checked_add(size as usize)
                .filter(|&end| end <= bytes.len())?;
            shape.sections.push(Section {
                id: bytes[at],
                at,
                start,
                end,
            });
            at = end;
        }
        for payload in Parser::new(0).parse_all(bytes) {
            match payload.ok()? {
                Payload::ImportSection(reader) => {
                    for import in reader {
                        let import = import.ok()?;
                        let kind = match import.ty {
                            TypeRef::Func(_) => {
                                shape.imported_funcs += 1;
                                "function"
                            }
                            TypeRef::Table(_) => "table",
                            TypeRef::Memory(_) => "memory",
                            TypeRef::Global(_) => "global",
                            TypeRef::Tag(_) => "tag",
                        };
                        shape.imports.push((
                            import.module.to_owned(),
                            import.name.to_owned(),
                            kind,
                        ));
                    }
                }
                Payload::FunctionSection(reader) => shape.defined_funcs = reader.count(),
                Payload::ExportSection(reader) => {
                    for export in reader {
                        let export = export.ok()?;
                        let kind = match export.kind {
                            ExternalKind::Func => "function",
                            ExternalKind::Table => "table",
                            ExternalKind::Memory => "memory",
                            ExternalKind::Global => "global",
                            ExternalKind::Tag => "tag",
                        };
                        shape.exports.push((export.name.to_owned(), kind));
                    }
                }
                Payload::StartSection { func, .. } => shape.start = Some(func),
                _ => {}
            }
        }
        Some(shape)
    }

    /// A name for an export of the bridge's, none of the module's.
    fn reserve(&self, base: &str, taken: Option<&str>) -> String {
        let mut name = base.to_owned();
        while taken == Some(name.as_str()) || self.exports.iter().any(|(n, _)| *n == name) {
            name.push('_');
        }
        name
    }

    /// `bytes` with the function exports `extra` added and the start
    /// section taken out.
    fn rewrite(&self, bytes: &[u8], extra: &[(&str, u32)]) -> Option<Vec<u8>> {
        let mut entries = Vec::new();
        for (name, index) in extra {
            put_leb(&mut entries, u32::try_from(name.len()).ok()?);
            entries.extend_from_slice(name.as_bytes());
            entries.push(0);
            put_leb(&mut entries, *index);
        }
        let section = |out: &mut Vec<u8>, count: u32, rest: &[u8]| {
            let mut body = Vec::with_capacity(rest.len() + entries.len() + 5);
            put_leb(&mut body, count);
            body.extend_from_slice(rest);
            body.extend_from_slice(&entries);
            out.push(7);
            put_leb(out, body.len() as u32);
            out.extend_from_slice(&body);
        };
        let added = u32::try_from(extra.len()).ok()?;
        let mut out = Vec::with_capacity(bytes.len() + entries.len() + 16);
        out.extend_from_slice(bytes.get(..8)?);
        let mut written = false;
        for s in &self.sections {
            match s.id {
                // The start section, which the bridge calls itself.
                8 => continue,
                7 => {
                    let (count, n) = leb(&bytes[s.start..s.end])?;
                    section(
                        &mut out,
                        count.checked_add(added)?,
                        &bytes[s.start + n..s.end],
                    );
                    written = true;
                    continue;
                }
                // The sections after the exports' place: start, element,
                // code, data and data count.
                8..=12 if !written => {
                    section(&mut out, added, &[]);
                    written = true;
                }
                _ => {}
            }
            out.extend_from_slice(&bytes[s.at..s.end]);
        }
        if !written {
            section(&mut out, added, &[]);
        }
        Some(out)
    }
}

/// A module compiled, and what the bridge reads of it.
struct Compiled {
    module: Module,
    shape: Shape,
    /// The exports of the bridge's own ([`compile`]): its first defined
    /// function's, by which its instances' functions are named, and its
    /// start function's.
    first: Option<String>,
    start: Option<String>,
}

/// `bytes` compiled. A module that defines functions or has a start
/// function is compiled with exports of the bridge's own added for them
/// and its start section taken out, so that the bridge calls its start
/// function itself, resumably: `wasmi` would call it at instantiation,
/// where it cannot stop at a script's function. A module that fails
/// fails with the error of its bytes as they came.
fn compile(engine: &Engine, bytes: &[u8]) -> Result<Compiled, String> {
    let Some(shape) = Shape::read(bytes) else {
        return Err(Module::new(engine, bytes).err().map_or_else(
            || "the module could not be read".to_owned(),
            |error| error.to_string(),
        ));
    };
    let first = (shape.defined_funcs > 0).then(|| shape.reserve("silver.first", None));
    let start = shape
        .start
        .map(|_| shape.reserve("silver.start", first.as_deref()));
    if first.is_some() || start.is_some() {
        let mut extra = Vec::new();
        if let Some(name) = &first {
            extra.push((name.as_str(), shape.imported_funcs));
        }
        if let (Some(name), Some(index)) = (&start, shape.start) {
            extra.push((name.as_str(), index));
        }
        let made = shape
            .rewrite(bytes, &extra)
            .and_then(|bytes| Module::new(engine, &bytes).ok());
        // The start function takes nothing and gives nothing, which the
        // module's own validation would have held it to.
        let fits = |module: &Module| {
            start.as_ref().is_none_or(|name| {
                module.exports().any(|export| {
                    export.name() == name
                        && matches!(export.ty(), ExternType::Func(ty)
                            if ty.params().is_empty() && ty.results().is_empty())
                })
            })
        };
        if let Some(module) = made.filter(fits) {
            return Ok(Compiled {
                module,
                shape,
                first,
                start,
            });
        }
    }
    match Module::new(engine, bytes) {
        Ok(module) => Ok(Compiled {
            module,
            shape,
            first: None,
            start: None,
        }),
        Err(error) => Err(error.to_string()),
    }
}

/// The key `wasmi` gave a function of the store, which tells two apart:
/// `wasmi` has no equality for them and names the key only in their
/// `Debug` form (its version held by the lockfile, and the form by the
/// tests of the functions' identity).
fn key(item: &impl fmt::Debug) -> Option<u32> {
    let text = format!("{item:?}");
    let digits = text.split_once("key: ")?.1;
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    digits[..end].parse().ok()
}

/// A value type as the bootstrap names it.
fn letter(ty: ValType) -> char {
    match ty {
        ValType::I32 => 'i',
        ValType::I64 => 'I',
        ValType::F32 => 'f',
        ValType::F64 => 'F',
        ValType::V128 => 'v',
        ValType::FuncRef => 'r',
        ValType::ExternRef => 'e',
    }
}

/// A function type as the bootstrap reads it: its parameters' letters,
/// `:`, its results'.
fn signature(ty: &FuncType) -> String {
    let mut out: String = ty.params().iter().map(|&t| letter(t)).collect();
    out.push(':');
    out.extend(ty.results().iter().map(|&t| letter(t)));
    out
}

fn kind_of(ty: &ExternType) -> &'static str {
    match ty {
        ExternType::Func(_) => "function",
        ExternType::Table(_) => "table",
        ExternType::Memory(_) => "memory",
        ExternType::Global(_) => "global",
    }
}

/// For each of `module`'s imports as `wasmi` has them, by kind, its
/// place in the module's own order, which `kinds` gives.
fn places(kinds: &[&'static str], module: &Module) -> Option<Vec<usize>> {
    let mut cursors: HashMap<&str, usize> = HashMap::new();
    module
        .imports()
        .map(|import| {
            let kind = kind_of(import.ty());
            let cursor = cursors.entry(kind).or_insert(0);
            let at = (*cursor..kinds.len()).find(|&i| kinds[i] == kind)?;
            *cursor = at + 1;
            Some(at)
        })
        .collect()
}

/// The class and words of what the bootstrap throws for `error`: a trap
/// a `RuntimeError` in the reference browser's words, a recursion too
/// deep its `RangeError`, a resource refused a `RangeError`, an import
/// that does not fit a `LinkError`.
fn failure(error: &Error) -> (&'static str, String) {
    if let Some(code) = error.as_trap_code() {
        let words = match code {
            TrapCode::StackOverflow => {
                return ("RangeError", "Maximum call stack size exceeded".into());
            }
            TrapCode::UnreachableCodeReached => "unreachable",
            TrapCode::MemoryOutOfBounds => "memory access out of bounds",
            TrapCode::TableOutOfBounds => "table index is out of bounds",
            TrapCode::IndirectCallToNull => "null function",
            TrapCode::IntegerDivisionByZero => "divide by zero",
            TrapCode::IntegerOverflow => "divide result unrepresentable",
            TrapCode::BadConversionToInteger => "float unrepresentable in integer range",
            TrapCode::BadSignature => "function signature mismatch",
            _ => return ("RuntimeError", code.to_string()),
        };
        return ("RuntimeError", words.into());
    }
    let class = match error.kind() {
        ErrorKind::Instantiation(
            InstantiationError::TooManyInstances
            | InstantiationError::TooManyTables
            | InstantiationError::TooManyMemories
            | InstantiationError::FailedToInstantiateMemory(_)
            | InstantiationError::FailedToInstantiateTable(_),
        )
        | ErrorKind::Memory(_)
        | ErrorKind::Table(_) => "RangeError",
        ErrorKind::Instantiation(InstantiationError::ElementSegmentDoesNotFit { .. }) => {
            "RuntimeError"
        }
        ErrorKind::Instantiation(_) => "LinkError",
        _ => "RuntimeError",
    };
    (class, error.to_string())
}

/// `[2, class, message]`: what the bootstrap throws.
fn failed<'js>(ctx: &Ctx<'js>, class: &str, message: &str) -> rquickjs::Result<Array<'js>> {
    let out = Array::new(ctx.clone())?;
    out.set(0, 2)?;
    out.set(1, class)?;
    out.set(2, message)?;
    Ok(out)
}

/// A handle the bootstrap gave that names nothing: a fault of its own.
fn unknown<T>(ctx: &Ctx<'_>, what: &str) -> rquickjs::Result<T> {
    Err(Exception::throw_type(ctx, &format!("not {what}")))
}

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
            instances: Vec::new(),
            funcs: HashMap::new(),
            hosts: HashMap::new(),
            defined: Vec::new(),
            externs: Vec::new(),
            pending: Vec::new(),
        }
    }

    fn module(&self, handle: f64) -> Option<&Compiled> {
        self.modules.get(index(handle)?)
    }

    /// `func`'s key, the function kept for the scripts that have it.
    fn func(&mut self, func: Func) -> Option<u32> {
        let key = key(&func)?;
        self.funcs.entry(key).or_insert(func);
        Some(key)
    }

    /// The function a script's handle names.
    fn func_of(&self, ctx: &Ctx<'_>, handle: f64) -> rquickjs::Result<Func> {
        let key = index(handle).and_then(|key| u32::try_from(key).ok());
        match key.and_then(|key| self.funcs.get(&key)) {
            Some(func) => Ok(*func),
            None => unknown(ctx, "a function of the store"),
        }
    }

    /// The name of the function `key`: its index in its module, as the
    /// interface names a function.
    fn name(&self, key: u32) -> u32 {
        if let Some(&(_, index)) = self.hosts.get(&key) {
            return index;
        }
        let at = self.defined.partition_point(|&(first, _, _)| first <= key);
        match at.checked_sub(1).map(|at| self.defined[at]) {
            Some((first, count, offset)) if key - first < count => offset + (key - first),
            _ => 0,
        }
    }

    /// A raw value of the bootstrap's, converted already by the
    /// interface's `ToWebAssemblyValue`, as one of `ty`.
    fn val(&mut self, ctx: &Ctx<'_>, value: &Value<'_>, ty: ValType) -> rquickjs::Result<Val> {
        let number = || value.as_number().ok_or("a number");
        let made = match ty {
            ValType::I32 => number().map(|n| Val::I32(n as i32)),
            ValType::I64 => value
                .as_big_int()
                .and_then(|n| n.clone().to_i64().ok())
                .map(Val::I64)
                .ok_or("a BigInt"),
            ValType::F32 => number().map(|n| Val::F32(F32::from_float(n as f32))),
            ValType::F64 => number().map(|n| Val::F64(F64::from_float(n))),
            ValType::FuncRef if value.is_null() => Ok(Val::FuncRef(Nullable::Null)),
            ValType::FuncRef => {
                let func = self.func_of(ctx, number().unwrap_or(-1.0))?;
                Ok(Val::FuncRef(Nullable::Val(func)))
            }
            ValType::ExternRef if value.is_null() => Ok(Val::ExternRef(Nullable::Null)),
            ValType::ExternRef => {
                let at = number().ok().and_then(index);
                match at {
                    Some(at) if at < self.externs.len() => Ok(self.externs[at]),
                    Some(at) if at == self.externs.len() => {
                        let made = ExternRef::new(&mut self.store, at as u32);
                        self.externs.push(made);
                        Ok(made)
                    }
                    _ => Err("an external value's index"),
                }
                .map(|made| Val::ExternRef(Nullable::Val(made)))
            }
            ValType::V128 => Err("a value of a known type"),
        };
        match made {
            Ok(made) => Ok(made),
            Err(what) => unknown(ctx, what),
        }
    }

    /// `val` as the bootstrap reads it: a number, a BigInt, a function's
    /// key, an external value's index, or null.
    fn raw<'js>(&mut self, ctx: &Ctx<'js>, val: &Val) -> rquickjs::Result<Value<'js>> {
        let null = Value::new_null(ctx.clone());
        Ok(match val {
            Val::I32(n) => Value::new_int(ctx.clone(), *n),
            Val::I64(n) => Value::new_big_int(ctx.clone(), *n)?,
            Val::F32(n) => Value::new_float(ctx.clone(), f64::from(n.to_float())),
            Val::F64(n) => Value::new_float(ctx.clone(), n.to_float()),
            Val::FuncRef(func) => match func.val() {
                Some(func) => match self.func(*func) {
                    Some(key) => Value::new_number(ctx.clone(), f64::from(key)),
                    None => return unknown(ctx, "a function with a key"),
                },
                None => null,
            },
            Val::ExternRef(held) => match held.val() {
                Some(held) => match held.data(&self.store).downcast_ref::<u32>() {
                    Some(&at) => Value::new_number(ctx.clone(), f64::from(at)),
                    None => null,
                },
                None => null,
            },
            Val::V128(_) => null,
        })
    }

    /// A call's answer to the bootstrap: its results, the script's
    /// function it stopped at, or its failure.
    fn outcome<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        result: Result<ResumableCall, Error>,
        outputs: Vec<Val>,
    ) -> rquickjs::Result<Array<'js>> {
        let out = Array::new(ctx.clone())?;
        match result {
            Ok(ResumableCall::Finished) => {
                out.set(0, 0)?;
                for (i, val) in outputs.iter().enumerate() {
                    out.set(i + 1, self.raw(ctx, val)?)?;
                }
            }
            Ok(ResumableCall::HostTrap(call)) => {
                let (slot, params) = match call.host_error().downcast_ref::<CallJs>() {
                    Some(stop) => (stop.slot, stop.params.clone()),
                    None => return failed(ctx, "RuntimeError", &call.host_error().to_string()),
                };
                let id = self.pending.len();
                self.pending.push(Some(Pending { call, outputs }));
                out.set(0, 1)?;
                out.set(1, id)?;
                out.set(2, slot)?;
                for (i, val) in params.iter().enumerate() {
                    out.set(i + 3, self.raw(ctx, val)?)?;
                }
            }
            Ok(ResumableCall::OutOfFuel(_)) => return failed(ctx, "RuntimeError", "out of fuel"),
            Err(error) => {
                let (class, message) = failure(&error);
                return failed(ctx, class, &message);
            }
        }
        Ok(out)
    }

    /// A call of the function `handle` with the raw arguments `args`.
    fn call<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        handle: f64,
        args: &Array<'js>,
    ) -> rquickjs::Result<Array<'js>> {
        let func = self.func_of(ctx, handle)?;
        let ty = func.ty(&self.store);
        let mut inputs = Vec::with_capacity(ty.params().len());
        for (i, &t) in ty.params().iter().enumerate() {
            inputs.push(self.val(ctx, &args.get(i)?, t)?);
        }
        let mut outputs: Vec<Val> = ty
            .results()
            .iter()
            .map(|&t| Val::default_for_ty(t))
            .collect();
        let result = func.call_resumable(&mut self.store, &inputs, &mut outputs);
        self.outcome(ctx, result, outputs)
    }

    /// The call `id`, stopped at a script's function, resumed with what
    /// the function returned, raw.
    fn resume<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        id: f64,
        results: &Array<'js>,
    ) -> rquickjs::Result<Array<'js>> {
        let Some(Pending { call, mut outputs }) = index(id)
            .and_then(|id| self.pending.get_mut(id))
            .and_then(Option::take)
        else {
            return unknown(ctx, "a call stopped");
        };
        self.trim();
        let ty = call.host_func().ty(&self.store);
        let mut inputs = Vec::with_capacity(ty.results().len());
        for (i, &t) in ty.results().iter().enumerate() {
            inputs.push(self.val(ctx, &results.get(i)?, t)?);
        }
        let result = call.resume(&mut self.store, &inputs, &mut outputs);
        self.outcome(ctx, result, outputs)
    }

    /// The calls stopped that are no longer, off the top.
    fn trim(&mut self) {
        while matches!(self.pending.last(), Some(None)) {
            self.pending.pop();
        }
    }

    /// An instance of the module `handle`, its imports `given` as the
    /// bootstrap resolved them in the module's order, a tag and a value
    /// each: `"f"` and a function's key, `"h"` and the slot of a script's
    /// callable, `"g"` and a global's value. `[0, instance, start]`, the
    /// key of the start function or -1; or why it failed.
    fn instantiate<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        handle: f64,
        given: &Array<'js>,
    ) -> rquickjs::Result<Array<'js>> {
        let Some(at) = index(handle).filter(|&at| at < self.modules.len()) else {
            return unknown(ctx, "a module");
        };
        let compiled = &self.modules[at];
        let module = compiled.module.clone();
        let kinds: Vec<&'static str> = compiled.shape.imports.iter().map(|i| i.2).collect();
        let (imported, defined) = (compiled.shape.imported_funcs, compiled.shape.defined_funcs);
        let (first, start) = (compiled.first.clone(), compiled.start.clone());
        let Some(places) = places(&kinds, &module) else {
            return unknown(ctx, "a module's imports");
        };
        let mut externs = Vec::with_capacity(places.len());
        for (import, place) in module.imports().zip(places) {
            let tag: String = given.get(2 * place)?;
            let value: Value<'js> = given.get(2 * place + 1)?;
            let label = format!(
                r#"Import #{place} "{}" "{}""#,
                import.module(),
                import.name()
            );
            externs.push(match (import.ty(), tag.as_str()) {
                (ExternType::Func(ty), "f") => {
                    let func = self.func_of(ctx, value.as_number().unwrap_or(-1.0))?;
                    if func.ty(&self.store) != *ty {
                        let message =
                            format!("{label}: imported function does not match the expected type");
                        return failed(ctx, "LinkError", &message);
                    }
                    Extern::Func(func)
                }
                (ExternType::Func(ty), "h") => {
                    let slot = value.as_number().unwrap_or(0.0) as u32;
                    let func = Func::new(&mut self.store, ty.clone(), move |_, params, _| {
                        Err(Error::host(CallJs {
                            slot,
                            params: params.to_vec(),
                        }))
                    });
                    let Some(key) = self.func(func) else {
                        return unknown(ctx, "a function with a key");
                    };
                    let index = kinds[..place].iter().filter(|k| **k == "function").count();
                    self.hosts.insert(key, (slot, index as u32));
                    Extern::Func(func)
                }
                (ExternType::Global(ty), "g") => {
                    let val = self.val(ctx, &value, ty.content())?;
                    Extern::Global(Global::new(&mut self.store, val, Mutability::Const))
                }
                _ => return unknown(ctx, "an import of its kind"),
            });
        }
        let instance = match Instance::new(&mut self.store, &module, &externs) {
            Ok(instance) => instance,
            Err(error) => {
                let (class, message) = failure(&error);
                return failed(ctx, class, &message);
            }
        };
        if let Some(func) = first.and_then(|name| instance.get_func(&self.store, &name))
            && let Some(key) = self.func(func)
        {
            let at = self.defined.partition_point(|&(first, _, _)| first <= key);
            self.defined.insert(at, (key, defined, imported));
        }
        let start = match start.and_then(|name| instance.get_func(&self.store, &name)) {
            Some(func) => self.func(func).map_or(-1.0, f64::from),
            None => -1.0,
        };
        self.instances.push((instance, at));
        let out = Array::new(ctx.clone())?;
        out.set(0, 0)?;
        out.set(1, self.instances.len() - 1)?;
        out.set(2, start)?;
        Ok(out)
    }

    /// The instance `handle`'s exports in its module's order: each one's
    /// name, kind and function's key.
    fn exports<'js>(&mut self, ctx: &Ctx<'js>, handle: f64) -> rquickjs::Result<Array<'js>> {
        let Some(&(instance, at)) = index(handle).and_then(|at| self.instances.get(at)) else {
            return unknown(ctx, "an instance");
        };
        let exports = self.modules[at].shape.exports.clone();
        let out = Array::new(ctx.clone())?;
        let mut n = 0;
        for (name, kind) in exports {
            let Some(func) = instance.get_func(&self.store, &name) else {
                continue;
            };
            let Some(key) = self.func(func) else {
                return unknown(ctx, "a function with a key");
            };
            out.set(n, name)?;
            out.set(n + 1, kind)?;
            out.set(n + 2, key)?;
            n += 3;
        }
        Ok(out)
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

/// A realm's WebAssembly, made at its first use.
type Shared = Rc<RefCell<Option<Wasm>>>;

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
/// `wasmModuleImports(module)`, its imports' modules, names, kinds and
/// types; `wasmModuleCustomSections(module, name)`, the custom sections
/// of that name, as buffers; `wasmInstantiate(module, imports)`,
/// `wasmInstanceExports(instance)`, `wasmFuncInfo(key)`,
/// `wasmCall(key, args)`, `wasmResume(id, results)`,
/// `wasmAbandon(id)` and `wasmReset()`, the instances and calls.
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
                with(&shared, |wasm| match compile(&wasm.engine, &bytes) {
                    Ok(compiled) => {
                        wasm.modules.push(compiled);
                        Ok(Value::new_number(
                            ctx.clone(),
                            (wasm.modules.len() - 1) as f64,
                        ))
                    }
                    Err(message) => {
                        Ok(rquickjs::String::from_str(ctx.clone(), &message)?.into_value())
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
                    for (i, (name, kind)) in module.shape.exports.iter().enumerate() {
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
                    // Each import's type, as the bootstrap reads it: a
                    // function's signature, a global's value type and
                    // `m` or `c` for mutable or constant.
                    let imports = &module.shape.imports;
                    let mut types = vec![String::new(); imports.len()];
                    let kinds: Vec<&'static str> = imports.iter().map(|i| i.2).collect();
                    let places = places(&kinds, &module.module).unwrap_or_default();
                    for (import, place) in module.module.imports().zip(places) {
                        types[place] = match import.ty() {
                            ExternType::Func(ty) => signature(ty),
                            ExternType::Global(ty) => {
                                let mutable = matches!(ty.mutability(), Mutability::Var);
                                format!(
                                    "{}{}",
                                    letter(ty.content()),
                                    if mutable { 'm' } else { 'c' }
                                )
                            }
                            _ => String::new(),
                        };
                    }
                    for (i, ((from, name, kind), ty)) in imports.iter().zip(types).enumerate() {
                        out.set(4 * i, from.as_str())?;
                        out.set(4 * i + 1, name.as_str())?;
                        out.set(4 * i + 2, *kind)?;
                        out.set(4 * i + 3, ty)?;
                    }
                    Ok::<_, rquickjs::Error>(())
                })?;
                Ok(out)
            },
        )?,
    )?;
    let shared = wasm.clone();
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
    let shared = wasm.clone();
    natives.set(
        "wasmInstantiate",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, handle: f64, given: Array<'js>| -> rquickjs::Result<Array<'js>> {
                with(&shared, |wasm| wasm.instantiate(&ctx, handle, &given))
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmInstanceExports",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, handle: f64| -> rquickjs::Result<Array<'js>> {
                with(&shared, |wasm| wasm.exports(&ctx, handle))
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmFuncInfo",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, handle: f64| -> rquickjs::Result<Array<'js>> {
                with(&shared, |wasm| {
                    let func = wasm.func_of(&ctx, handle)?;
                    let key = handle as u32;
                    let out = Array::new(ctx.clone())?;
                    out.set(0, signature(&func.ty(&wasm.store)))?;
                    out.set(1, wasm.name(key).to_string())?;
                    out.set(
                        2,
                        wasm.hosts
                            .get(&key)
                            .map_or(-1.0, |&(slot, _)| f64::from(slot)),
                    )?;
                    Ok(out)
                })
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmCall",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, handle: f64, args: Array<'js>| -> rquickjs::Result<Array<'js>> {
                with(&shared, |wasm| wasm.call(&ctx, handle, &args))
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmResume",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, id: f64, results: Array<'js>| -> rquickjs::Result<Array<'js>> {
                with(&shared, |wasm| wasm.resume(&ctx, id, &results))
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmAbandon",
        Function::new(ctx.clone(), move |id: f64| {
            with(&shared, |wasm| {
                if let Some(call) = index(id).and_then(|id| wasm.pending.get_mut(id)) {
                    *call = None;
                }
                wasm.trim();
            });
        })?,
    )?;
    let shared = wasm;
    natives.set(
        "wasmReset",
        Function::new(ctx.clone(), move || {
            if let Some(wasm) = shared.borrow_mut().as_mut() {
                wasm.pending.clear();
            }
        })?,
    )?;
    Ok(())
}
