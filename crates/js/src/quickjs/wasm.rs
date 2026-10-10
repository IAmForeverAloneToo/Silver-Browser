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
//!
//! A memory's bytes are the store's, and a script's `buffer` of it a
//! copy, its mirror: what scripts wrote in it goes into the memory when
//! a call enters the store, and what modules wrote goes out into it,
//! in runs of [`MIRROR_CHUNK`] bytes that differ, when a call leaves.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use rquickjs::function::Rest;
use rquickjs::{Array, ArrayBuffer, Ctx, Exception, Function, Object, Value};
use wasmi::errors::{ErrorKind, HostError, InstantiationError};
use wasmi::{
    CompilationMode, Config, Engine, Error, Extern, ExternRef, ExternType, F32, F64, Func,
    FuncType, Global, Instance, Memory, MemoryType, Module, Mutability, Nullable, Ref, RefType,
    ResourceLimiter, ResumableCall, ResumableCallHostTrap, Store, Table, TableType, TrapCode, Val,
    ValType,
};
use wasmi_core::LimiterError;

use super::dom::Budget;
use crate::WORK_PER_STEP;

/// The largest module a realm compiles.
pub(crate) const MAX_WASM_MODULE_BYTES: usize = 64 << 20;

/// A module's work a step of the run's budget buys, in `wasmi`'s fuel.
pub(crate) const WASM_FUEL_PER_STEP: u64 = 100_000;

/// What a realm's memories and tables hold, in all.
pub(crate) const MAX_WASM_MEMORY: usize = 256 << 20;

/// A realm's instances; its memories and tables, ten times as many each.
pub(crate) const MAX_WASM_INSTANCES: usize = 1_000;

/// A table's length.
pub(crate) const MAX_WASM_TABLE_ELEMENTS: usize = 10_000_000;

/// Calls between modules and scripts, nested.
pub(crate) const MAX_WASM_NESTING: usize = 64;

/// A module's calls, nested; deeper is a stack overflow (`RangeError`),
/// as the reference browser's is.
const MAX_WASM_RECURSION: usize = 10_000;

const _: () = assert!(WASM_FUEL_PER_STEP.is_multiple_of(WORK_PER_STEP));

/// What a realm's store may hold ([`MAX_WASM_MEMORY`] in all, a table's
/// element counted as 8 bytes): `wasmi` asks before it allocates.
#[derive(Default)]
struct Limiter {
    /// The bytes taken, and those the last growth took, given back if
    /// it failed after all.
    taken: usize,
    last: usize,
}

impl Limiter {
    fn take(&mut self, bytes: usize) -> bool {
        match self.taken.checked_add(bytes) {
            Some(taken) if taken <= MAX_WASM_MEMORY => {
                self.taken = taken;
                self.last = bytes;
                true
            }
            _ => false,
        }
    }

    fn give_back(&mut self) {
        self.taken -= std::mem::take(&mut self.last);
    }
}

impl ResourceLimiter for Limiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, LimiterError> {
        Ok(self.take(desired.saturating_sub(current)))
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, LimiterError> {
        let bytes = desired.saturating_sub(current).saturating_mul(8);
        Ok(desired <= MAX_WASM_TABLE_ELEMENTS && self.take(bytes))
    }

    fn memory_grow_failed(
        &mut self,
        _error: &wasmi::errors::MemoryError,
    ) -> Result<(), LimiterError> {
        self.give_back();
        Ok(())
    }

    fn table_grow_failed(
        &mut self,
        _error: &wasmi::errors::TableError,
    ) -> Result<(), LimiterError> {
        self.give_back();
        Ok(())
    }

    fn instances(&self) -> usize {
        MAX_WASM_INSTANCES
    }

    fn tables(&self) -> usize {
        10 * MAX_WASM_INSTANCES
    }

    fn memories(&self) -> usize {
        10 * MAX_WASM_INSTANCES
    }
}

/// A memory's bytes and its buffer's are compared in runs of this many.
const MIRROR_CHUNK: usize = 4096;

/// What a realm's modules are made in.
pub(super) struct Wasm {
    engine: Engine,
    store: Store<Limiter>,
    /// The run's budget, which a module's work is charged to, and the
    /// fuel the call under way was given.
    budget: Budget,
    given: u64,
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
    /// The memories, tables and globals a script has been given, by
    /// their keys.
    memories: HashMap<u32, Memory>,
    tables: HashMap<u32, Table>,
    globals: HashMap<u32, Global>,
    /// What the buffer a script has of a memory holds, by the memory's
    /// key: the memory as the last call into the store left it.
    mirrors: HashMap<u32, Vec<u8>>,
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

/// `bytes` as base64, which `setFromBase64` reads.
fn to_base64(bytes: &[u8]) -> String {
    const LETTERS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let byte = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let word = byte(0) << 16 | byte(1) << 8 | byte(2);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(LETTERS[(word >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The value type the bootstrap names by `letter`.
fn value_type(letter: &str) -> Option<ValType> {
    Some(match letter {
        "i" => ValType::I32,
        "I" => ValType::I64,
        "f" => ValType::F32,
        "F" => ValType::F64,
        "r" => ValType::FuncRef,
        "e" => ValType::ExternRef,
        _ => return None,
    })
}

/// A reference value as a table holds it.
fn reference(val: Val) -> Ref {
    match val {
        Val::ExternRef(held) => Ref::Extern(held),
        Val::FuncRef(func) => Ref::Func(func),
        _ => Ref::Func(Nullable::Null),
    }
}

/// A table's element type: as a value's, and as the reference browser
/// names it.
fn element(ty: RefType) -> (ValType, &'static str) {
    match ty {
        RefType::Func => (ValType::FuncRef, "funcref"),
        RefType::Extern => (ValType::ExternRef, "externref"),
    }
}

/// `[0, values...]`: what the bootstrap reads of a native that may fail.
fn answer<'js>(ctx: &Ctx<'js>, values: &[Value<'js>]) -> rquickjs::Result<Array<'js>> {
    let out = Array::new(ctx.clone())?;
    out.set(0, 0)?;
    for (i, value) in values.iter().enumerate() {
        out.set(i + 1, value.clone())?;
    }
    Ok(out)
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
    fn new(budget: Budget) -> Wasm {
        let mut config = Config::default();
        // The core 2.0 features, and 3.0's tail calls, extended
        // constant expressions and multiple memories, as `wasmi` has
        // them; not SIMD or memory64, which this build leaves out.
        config
            .wasm_custom_page_sizes(false)
            .wasm_wide_arithmetic(false)
            .compilation_mode(CompilationMode::LazyTranslation)
            .consume_fuel(true)
            .set_max_recursion_depth(MAX_WASM_RECURSION);
        let engine = Engine::new(&config);
        let mut store = Store::new(&engine, Limiter::default());
        store.limiter(|limiter| limiter as &mut dyn ResourceLimiter);
        Wasm {
            engine,
            store,
            budget,
            given: 0,
            modules: Vec::new(),
            instances: Vec::new(),
            funcs: HashMap::new(),
            hosts: HashMap::new(),
            defined: Vec::new(),
            externs: Vec::new(),
            pending: Vec::new(),
            memories: HashMap::new(),
            tables: HashMap::new(),
            globals: HashMap::new(),
            mirrors: HashMap::new(),
        }
    }

    /// `item`'s key, the item kept in `map` for the scripts that have it.
    fn keep<T: Copy + fmt::Debug>(map: &mut HashMap<u32, T>, item: T) -> Option<u32> {
        let key = key(&item)?;
        map.entry(key).or_insert(item);
        Some(key)
    }

    /// What a script's handle names in `map`.
    fn kept<T: Copy>(
        map: &HashMap<u32, T>,
        ctx: &Ctx<'_>,
        handle: f64,
        what: &str,
    ) -> rquickjs::Result<T> {
        let key = index(handle).and_then(|key| u32::try_from(key).ok());
        match key.and_then(|key| map.get(&key)) {
            Some(item) => Ok(*item),
            None => unknown(ctx, what),
        }
    }

    fn module(&self, handle: f64) -> Option<&Compiled> {
        self.modules.get(index(handle)?)
    }

    /// `func`'s key, the function kept for the scripts that have it.
    fn func(&mut self, func: Func) -> Option<u32> {
        Wasm::keep(&mut self.funcs, func)
    }

    /// The function a script's handle names.
    fn func_of(&self, ctx: &Ctx<'_>, handle: f64) -> rquickjs::Result<Func> {
        Wasm::kept(&self.funcs, ctx, handle, "a function of the store")
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

    /// The fuel the run's budget has left given to a call into the store.
    fn fuel(&mut self) {
        let steps = self.budget.limit.saturating_sub(self.budget.steps.get());
        self.given = if self.budget.exhausted.get() {
            0
        } else {
            u64::from(steps) * WASM_FUEL_PER_STEP
        };
        let _ = self.store.set_fuel(self.given);
    }

    /// The fuel the call burned, charged to the run.
    fn burned(&mut self) {
        let left = self.store.get_fuel().unwrap_or(0);
        let units = self.given.saturating_sub(left) / (WASM_FUEL_PER_STEP / WORK_PER_STEP);
        self.budget.charge(units);
        self.given = 0;
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
            // The run's budget is spent: the realm's interrupt ends the
            // run at its next check, as a script that loops is ended.
            Ok(ResumableCall::OutOfFuel(_)) => {
                self.budget.exhausted.set(true);
                return failed(ctx, "RangeError", "the script ran too long");
            }
            Err(error) => {
                let (class, message) = failure(&error);
                return failed(ctx, class, &message);
            }
        }
        Ok(out)
    }

    /// A call of the function `handle` with the raw arguments `args`;
    /// `top` when no other call is under way, so that any call still
    /// stopped is one a script's error left behind.
    fn call<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        handle: f64,
        args: &Array<'js>,
        top: bool,
    ) -> rquickjs::Result<Array<'js>> {
        let func = self.func_of(ctx, handle)?;
        if top {
            self.pending.clear();
        }
        if self.pending.iter().flatten().count() >= MAX_WASM_NESTING {
            return failed(ctx, "RangeError", "Maximum call stack size exceeded");
        }
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
        self.fuel();
        let result = func.call_resumable(&mut self.store, &inputs, &mut outputs);
        self.burned();
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
        // The calls stopped above it are over: a script's error left
        // them, its bootstrap unable to say so (its stack spent).
        let at = index(id).unwrap_or(usize::MAX);
        self.pending.truncate(at.saturating_add(1));
        let Some(Pending { call, mut outputs }) = self.pending.get_mut(at).and_then(Option::take)
        else {
            return unknown(ctx, "a call stopped");
        };
        self.trim();
        let ty = call.host_func().ty(&self.store);
        let mut inputs = Vec::with_capacity(ty.results().len());
        for (i, &t) in ty.results().iter().enumerate() {
            inputs.push(self.val(ctx, &results.get(i)?, t)?);
        }
        self.fuel();
        let result = call.resume(&mut self.store, &inputs, &mut outputs);
        self.burned();
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
    /// callable, `"g"` and a global's value, `"m"`, `"t"` and `"G"` and
    /// a memory's, a table's and a global's key. `[0, instance, start]`,
    /// the key of the start function or -1; or why it failed.
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
                (ExternType::Memory(_), "m") => {
                    let handle = value.as_number().unwrap_or(-1.0);
                    Extern::Memory(Wasm::kept(&self.memories, ctx, handle, "a memory")?)
                }
                (ExternType::Table(_), "t") => {
                    let handle = value.as_number().unwrap_or(-1.0);
                    Extern::Table(Wasm::kept(&self.tables, ctx, handle, "a table")?)
                }
                (ExternType::Global(_), "G") => {
                    let handle = value.as_number().unwrap_or(-1.0);
                    Extern::Global(Wasm::kept(&self.globals, ctx, handle, "a global")?)
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
    /// name, kind and key.
    fn exports<'js>(&mut self, ctx: &Ctx<'js>, handle: f64) -> rquickjs::Result<Array<'js>> {
        let Some(&(instance, at)) = index(handle).and_then(|at| self.instances.get(at)) else {
            return unknown(ctx, "an instance");
        };
        let exports = self.modules[at].shape.exports.clone();
        let out = Array::new(ctx.clone())?;
        let mut n = 0;
        for (name, kind) in exports {
            let key = match instance.get_export(&self.store, &name) {
                Some(Extern::Func(func)) => self.func(func),
                Some(Extern::Memory(memory)) => Wasm::keep(&mut self.memories, memory),
                Some(Extern::Table(table)) => Wasm::keep(&mut self.tables, table),
                Some(Extern::Global(global)) => Wasm::keep(&mut self.globals, global),
                None => continue,
            };
            let Some(key) = key else {
                return unknown(ctx, "an export with a key");
            };
            out.set(n, name)?;
            out.set(n + 1, kind)?;
            out.set(n + 2, key)?;
            n += 3;
        }
        Ok(out)
    }

    /// A memory of `initial` pages, at most `maximum` (none when
    /// negative): `[0, key]`, or why there is none.
    fn memory_new<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        initial: f64,
        maximum: f64,
    ) -> rquickjs::Result<Array<'js>> {
        // The builder refuses limits that do not fit, where `new` panics.
        let mut ty = MemoryType::builder();
        ty.min(initial as u64)
            .max((maximum >= 0.0).then_some(maximum as u64));
        let made = ty
            .build()
            .ok()
            .and_then(|ty| Memory::new(&mut self.store, ty).ok())
            .and_then(|memory| Wasm::keep(&mut self.memories, memory));
        match made {
            Some(key) => answer(ctx, &[Value::new_number(ctx.clone(), f64::from(key))]),
            None => failed(ctx, "RangeError", "could not allocate memory"),
        }
    }

    /// The memory `handle` grown by `delta` pages: `[0, its pages
    /// before]`, or why it did not grow. Its buffer is the script's to
    /// detach.
    fn memory_grow<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        handle: f64,
        delta: f64,
    ) -> rquickjs::Result<Array<'js>> {
        let memory = Wasm::kept(&self.memories, ctx, handle, "a memory")?;
        match memory.grow(&mut self.store, delta as u64) {
            Ok(before) => {
                self.mirrors.remove(&(handle as u32));
                answer(ctx, &[Value::new_number(ctx.clone(), before as f64)])
            }
            Err(_) => failed(ctx, "RangeError", "Maximum memory size exceeded"),
        }
    }

    /// A new buffer of the memory `handle`, its mirror.
    fn memory_buffer<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        handle: f64,
    ) -> rquickjs::Result<ArrayBuffer<'js>> {
        let memory = Wasm::kept(&self.memories, ctx, handle, "a memory")?;
        let bytes = memory.data(&self.store).to_vec();
        self.mirrors.insert(handle as u32, bytes.clone());
        ArrayBuffer::new(ctx.clone(), bytes)
    }

    /// What scripts wrote in their buffers, into the memories: `list` a
    /// memory's key and its buffer's bytes as base64, in turn.
    fn sync_in(&mut self, ctx: &Ctx<'_>, list: &Array<'_>) -> rquickjs::Result<()> {
        for i in (0..list.len()).step_by(2) {
            let handle: f64 = list.get(i)?;
            let text: String = list.get(i + 1)?;
            let memory = Wasm::kept(&self.memories, ctx, handle, "a memory")?;
            let data = memory.data_mut(&mut self.store);
            if let Some(bytes) = from_base64(&text).filter(|bytes| bytes.len() == data.len()) {
                data.copy_from_slice(&bytes);
                self.mirrors.insert(handle as u32, bytes);
            }
        }
        Ok(())
    }

    /// What modules wrote, for the buffers: each memory whose buffer
    /// differs, its key and each run of bytes that differs, its offset
    /// and its bytes as base64; a memory grown, its key alone, its
    /// buffer to be detached.
    fn sync_out<'js>(&mut self, ctx: &Ctx<'js>) -> rquickjs::Result<Array<'js>> {
        let out = Array::new(ctx.clone())?;
        let mut grown = Vec::new();
        for (&key, mirror) in &mut self.mirrors {
            let Some(memory) = self.memories.get(&key) else {
                continue;
            };
            let data = memory.data(&self.store);
            let change = Array::new(ctx.clone())?;
            change.set(0, key)?;
            if data.len() != mirror.len() {
                grown.push(key);
            } else {
                let end = |at: usize| (at + MIRROR_CHUNK).min(data.len());
                let mut at = 0;
                while at < data.len() {
                    let start = at;
                    while at < data.len() && data[at..end(at)] != mirror[at..end(at)] {
                        at = end(at);
                    }
                    if at > start {
                        mirror[start..at].copy_from_slice(&data[start..at]);
                        change.set(change.len(), start)?;
                        change.set(change.len(), to_base64(&data[start..at]))?;
                    } else {
                        at = end(at);
                    }
                }
                if change.len() == 1 {
                    continue;
                }
            }
            out.set(out.len(), change)?;
        }
        for key in grown {
            self.mirrors.remove(&key);
        }
        Ok(out)
    }

    /// A table of `letter`'s references, `initial` long, at most
    /// `maximum` (none when negative), each element `init`: `[0, key]`,
    /// or why there is none.
    fn table_new<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        letter: &str,
        limits: (f64, f64),
        init: &Value<'js>,
    ) -> rquickjs::Result<Array<'js>> {
        let ty = match letter {
            "e" => RefType::Extern,
            _ => RefType::Func,
        };
        let init = reference(self.val(ctx, init, element(ty).0)?);
        let (initial, maximum) = (
            limits.0 as u32,
            (limits.1 >= 0.0).then_some(limits.1 as u32),
        );
        // `TableType::new` panics at limits that do not fit.
        if maximum.is_some_and(|maximum| maximum < initial) {
            return failed(ctx, "RangeError", "could not allocate table");
        }
        let ty = TableType::new(ty, initial, maximum);
        let made = Table::new(&mut self.store, ty, init)
            .ok()
            .and_then(|table| Wasm::keep(&mut self.tables, table));
        match made {
            Some(key) => answer(ctx, &[Value::new_number(ctx.clone(), f64::from(key))]),
            None => failed(ctx, "RangeError", "could not allocate table"),
        }
    }

    /// Why `index` is past the table's end, in the reference browser's
    /// words.
    fn past_end<'js>(
        &self,
        ctx: &Ctx<'js>,
        table: Table,
        index: f64,
    ) -> rquickjs::Result<Array<'js>> {
        let message = format!(
            "invalid address {index} in {} table of size {}",
            element(table.ty(&self.store).element()).1,
            table.size(&self.store)
        );
        failed(ctx, "RangeError", &message)
    }

    /// The element `index` of the table `handle`: `[0, value]`, or why
    /// there is none.
    fn table_get<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        handle: f64,
        index: f64,
    ) -> rquickjs::Result<Array<'js>> {
        let table = Wasm::kept(&self.tables, ctx, handle, "a table")?;
        match table.get(&self.store, index as u64) {
            Some(held) => {
                let value = self.raw(ctx, &Val::from(held))?;
                answer(ctx, &[value])
            }
            None => self.past_end(ctx, table, index),
        }
    }

    /// The element `index` of the table `handle` set to `value`: `[0]`,
    /// or why it is not.
    fn table_set<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        handle: f64,
        index: f64,
        value: &Value<'js>,
    ) -> rquickjs::Result<Array<'js>> {
        let table = Wasm::kept(&self.tables, ctx, handle, "a table")?;
        let ty = element(table.ty(&self.store).element()).0;
        let held = reference(self.val(ctx, value, ty)?);
        match table.set(&mut self.store, index as u64, held) {
            Ok(()) => answer(ctx, &[]),
            Err(_) => self.past_end(ctx, table, index),
        }
    }

    /// The table `handle` grown by `delta` elements of `value`: `[0, its
    /// length before]`, or why it did not grow.
    fn table_grow<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        handle: f64,
        delta: f64,
        value: &Value<'js>,
    ) -> rquickjs::Result<Array<'js>> {
        let table = Wasm::kept(&self.tables, ctx, handle, "a table")?;
        let ty = element(table.ty(&self.store).element()).0;
        let held = reference(self.val(ctx, value, ty)?);
        match table.grow(&mut self.store, delta as u64, held) {
            Ok(before) => answer(ctx, &[Value::new_number(ctx.clone(), before as f64)]),
            Err(_) => failed(
                ctx,
                "RangeError",
                &format!("failed to grow table by {delta}"),
            ),
        }
    }

    /// A global of `letter`'s values, mutable or not, holding `value`:
    /// its key.
    fn global_new(
        &mut self,
        ctx: &Ctx<'_>,
        letter: &str,
        mutable: bool,
        value: &Value<'_>,
    ) -> rquickjs::Result<f64> {
        let Some(ty) = value_type(letter) else {
            return unknown(ctx, "a value type");
        };
        let val = self.val(ctx, value, ty)?;
        let mutability = if mutable {
            Mutability::Var
        } else {
            Mutability::Const
        };
        let global = Global::new(&mut self.store, val, mutability);
        match Wasm::keep(&mut self.globals, global) {
            Some(key) => Ok(f64::from(key)),
            None => unknown(ctx, "a global with a key"),
        }
    }
}

/// A handle's index: a whole number, not negative.
fn index(handle: f64) -> Option<usize> {
    (handle.fract() == 0.0 && handle >= 0.0 && handle < u32::MAX as f64).then_some(handle as usize)
}

/// `f` of the realm's WebAssembly, made if this is its first use. A
/// panic of `wasmi`'s, which a valid module should never cause, is
/// caught here: the call throws, and the realm's WebAssembly is given
/// up, every later call throwing too.
fn with<R>(
    shared: &Shared,
    ctx: &Ctx<'_>,
    f: impl FnOnce(&mut Wasm) -> rquickjs::Result<R>,
) -> rquickjs::Result<R> {
    let mut state = shared.state.borrow_mut();
    if matches!(*state, State::Unmade) {
        *state = State::Live(Box::new(Wasm::new(shared.budget.clone())));
    }
    let State::Live(wasm) = &mut *state else {
        return Err(Exception::throw_internal(ctx, GIVEN_UP));
    };
    match catch_unwind(AssertUnwindSafe(|| f(wasm))) {
        Ok(result) => result,
        Err(_) => {
            *state = State::Dead;
            Err(Exception::throw_internal(ctx, GIVEN_UP))
        }
    }
}

const GIVEN_UP: &str = "WebAssembly failed in this page and is no longer available";

/// A realm's WebAssembly: made at its first use, given up after a panic.
enum State {
    Unmade,
    Live(Box<Wasm>),
    Dead,
}

/// What the natives share: the realm's WebAssembly and the run's budget.
struct Bridge {
    budget: Budget,
    state: RefCell<State>,
}

type Shared = Rc<Bridge>;

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
    let wasm: Shared = Rc::new(Bridge {
        budget: budget.clone(),
        state: RefCell::new(State::Unmade),
    });
    natives.set("wasmModuleLimit", MAX_WASM_MODULE_BYTES as f64)?;
    let (shared, charge) = (wasm.clone(), budget.clone());
    natives.set(
        "wasmValidate",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, bytes: String| -> rquickjs::Result<bool> {
                let bytes = module_bytes(&ctx, &charge, &bytes, "WebAssembly.validate()")?;
                with(&shared, &ctx, |wasm| {
                    Ok(Module::validate(&wasm.engine, &bytes).is_ok())
                })
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
                with(&shared, &ctx, |wasm| match compile(&wasm.engine, &bytes) {
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
                with(&shared, &ctx, |wasm| {
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
                with(&shared, &ctx, |wasm| {
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
                with(&shared, &ctx, |wasm| {
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
                with(&shared, &ctx, |wasm| wasm.instantiate(&ctx, handle, &given))
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmInstanceExports",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, handle: f64| -> rquickjs::Result<Array<'js>> {
                with(&shared, &ctx, |wasm| wasm.exports(&ctx, handle))
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmFuncInfo",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, handle: f64| -> rquickjs::Result<Array<'js>> {
                with(&shared, &ctx, |wasm| {
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
            move |ctx: Ctx<'js>,
                  handle: f64,
                  args: Array<'js>,
                  top: bool|
                  -> rquickjs::Result<Array<'js>> {
                with(&shared, &ctx, |wasm| wasm.call(&ctx, handle, &args, top))
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmResume",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, id: f64, results: Array<'js>| -> rquickjs::Result<Array<'js>> {
                with(&shared, &ctx, |wasm| wasm.resume(&ctx, id, &results))
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmAbandon",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, id: f64| {
            with(&shared, &ctx, |wasm| {
                wasm.pending.truncate(index(id).unwrap_or(usize::MAX));
                wasm.trim();
                Ok(())
            })
        })?,
    )?;
    install_objects(ctx, natives, &wasm)?;
    let shared = wasm;
    natives.set(
        "wasmReset",
        Function::new(ctx.clone(), move || {
            if let State::Live(wasm) = &mut *shared.state.borrow_mut() {
                wasm.pending.clear();
            }
        })?,
    )?;
    Ok(())
}

/// The natives of the `Memory`, `Table` and `Global` classes and of the
/// memories' mirrors: `wasmMemoryNew(initial, maximum)`,
/// `wasmMemoryGrow(key, delta)`, `wasmMemoryBuffer(key)`,
/// `wasmSyncIn(list)`, `wasmSyncOut()`, `wasmDetach(buffer)`;
/// `wasmTableNew(letter, initial, maximum, init)`, `wasmTableLength`,
/// `wasmTableGet`, `wasmTableSet`, `wasmTableGrow`, `wasmTableType`;
/// `wasmGlobalNew(letter, mutable, value)`, `wasmGlobalGet`,
/// `wasmGlobalSet`, `wasmGlobalType`.
fn install_objects<'js>(
    ctx: &Ctx<'js>,
    natives: &Object<'js>,
    wasm: &Shared,
) -> rquickjs::Result<()> {
    let shared = wasm.clone();
    natives.set(
        "wasmMemoryNew",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, initial: f64, maximum: f64| -> rquickjs::Result<Array<'js>> {
                with(&shared, &ctx, |wasm| {
                    wasm.memory_new(&ctx, initial, maximum)
                })
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmMemoryGrow",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, handle: f64, delta: f64| -> rquickjs::Result<Array<'js>> {
                with(&shared, &ctx, |wasm| wasm.memory_grow(&ctx, handle, delta))
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmMemoryBuffer",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, handle: f64| -> rquickjs::Result<ArrayBuffer<'js>> {
                with(&shared, &ctx, |wasm| wasm.memory_buffer(&ctx, handle))
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmSyncIn",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, list: Array<'js>| -> rquickjs::Result<()> {
                with(&shared, &ctx, |wasm| wasm.sync_in(&ctx, &list))
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmSyncOut",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>| -> rquickjs::Result<Array<'js>> {
                with(&shared, &ctx, |wasm| wasm.sync_out(&ctx))
            },
        )?,
    )?;
    natives.set(
        "wasmDetach",
        Function::new(ctx.clone(), |buffer: Value<'js>| {
            if let Some(mut buffer) = ArrayBuffer::from_value(buffer) {
                buffer.detach();
            }
        })?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmTableNew",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  letter: String,
                  initial: f64,
                  maximum: f64,
                  init: Value<'js>|
                  -> rquickjs::Result<Array<'js>> {
                with(&shared, &ctx, |wasm| {
                    wasm.table_new(&ctx, &letter, (initial, maximum), &init)
                })
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmTableLength",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, handle: f64| -> rquickjs::Result<f64> {
                with(&shared, &ctx, |wasm| {
                    let table = Wasm::kept(&wasm.tables, &ctx, handle, "a table")?;
                    Ok(table.size(&wasm.store) as f64)
                })
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmTableType",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, handle: f64| -> rquickjs::Result<String> {
                with(&shared, &ctx, |wasm| {
                    let table = Wasm::kept(&wasm.tables, &ctx, handle, "a table")?;
                    Ok(letter(element(table.ty(&wasm.store).element()).0).to_string())
                })
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmTableGet",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, handle: f64, index: f64| -> rquickjs::Result<Array<'js>> {
                with(&shared, &ctx, |wasm| wasm.table_get(&ctx, handle, index))
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmTableSet",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  handle: f64,
                  index: f64,
                  value: Value<'js>|
                  -> rquickjs::Result<Array<'js>> {
                with(&shared, &ctx, |wasm| {
                    wasm.table_set(&ctx, handle, index, &value)
                })
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmTableGrow",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  handle: f64,
                  delta: f64,
                  value: Value<'js>|
                  -> rquickjs::Result<Array<'js>> {
                with(&shared, &ctx, |wasm| {
                    wasm.table_grow(&ctx, handle, delta, &value)
                })
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmGlobalNew",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  letter: String,
                  mutable: bool,
                  value: Value<'js>|
                  -> rquickjs::Result<f64> {
                with(&shared, &ctx, |wasm| {
                    wasm.global_new(&ctx, &letter, mutable, &value)
                })
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmGlobalType",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, handle: f64| -> rquickjs::Result<String> {
                with(&shared, &ctx, |wasm| {
                    let global = Wasm::kept(&wasm.globals, &ctx, handle, "a global")?;
                    let ty = global.ty(&wasm.store);
                    let mutable = matches!(ty.mutability(), Mutability::Var);
                    Ok(format!(
                        "{}{}",
                        letter(ty.content()),
                        if mutable { 'm' } else { 'c' }
                    ))
                })
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmGlobalGet",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, handle: f64| -> rquickjs::Result<Value<'js>> {
                with(&shared, &ctx, |wasm| {
                    let global = Wasm::kept(&wasm.globals, &ctx, handle, "a global")?;
                    let val = global.get(&wasm.store);
                    wasm.raw(&ctx, &val)
                })
            },
        )?,
    )?;
    let shared = wasm.clone();
    natives.set(
        "wasmGlobalSet",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, handle: f64, value: Value<'js>| -> rquickjs::Result<()> {
                with(&shared, &ctx, |wasm| {
                    let global = Wasm::kept(&wasm.globals, &ctx, handle, "a global")?;
                    let val = wasm.val(&ctx, &value, global.ty(&wasm.store).content())?;
                    global
                        .set(&mut wasm.store, val)
                        .map_err(|_| Exception::throw_type(&ctx, "an immutable global"))
                })
            },
        )?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> Budget {
        Budget {
            steps: Rc::default(),
            work: Rc::default(),
            exhausted: Rc::default(),
            limit: 1_000,
        }
    }

    #[test]
    fn a_panic_in_the_engine_gives_up_the_realms_webassembly() {
        let runtime = rquickjs::Runtime::new().expect("a runtime");
        let context = rquickjs::Context::full(&runtime).expect("a context");
        context.with(|ctx| {
            let shared = Rc::new(Bridge {
                budget: budget(),
                state: RefCell::new(State::Unmade),
            });
            let made = with(&shared, &ctx, |wasm| Ok(wasm.modules.len()));
            assert_eq!(made.ok(), Some(0));
            let panicked = with(&shared, &ctx, |_| -> rquickjs::Result<()> {
                panic!("a fault of the engine's")
            });
            assert!(panicked.is_err());
            assert!(matches!(*shared.state.borrow(), State::Dead));
            assert!(with(&shared, &ctx, |_| Ok(())).is_err());
        });
    }

    #[test]
    fn the_limiter_counts_what_the_store_holds() {
        let mut limiter = Limiter::default();
        assert_eq!(
            limiter.memory_growing(0, MAX_WASM_MEMORY, None).ok(),
            Some(true)
        );
        assert_eq!(limiter.memory_growing(0, 1, None).ok(), Some(false));
        let failed = wasmi::errors::MemoryError::OutOfBoundsGrowth;
        limiter
            .memory_grow_failed(&failed)
            .expect("it is given back");
        assert_eq!(limiter.table_growing(0, 1000, None).ok(), Some(true));
        assert_eq!(limiter.taken, 8000);
        let past = MAX_WASM_TABLE_ELEMENTS + 1;
        assert_eq!(limiter.table_growing(0, past, None).ok(), Some(false));
    }

    /// The realm's stack ends a page's nesting long before the bound
    /// (M29 step 4), which holds all the same.
    #[test]
    fn calls_nest_no_deeper_than_the_bound() -> rquickjs::Result<()> {
        let runtime = rquickjs::Runtime::new().expect("a runtime");
        let context = rquickjs::Context::full(&runtime).expect("a context");
        context.with(|ctx| {
            let mut wasm = Wasm::new(budget());
            let text = r#"(module (import "env" "f" (func $f)) (func (export "g") call $f))"#;
            let bytes = wat::parse_str(text).expect("it parses");
            let compiled = compile(&wasm.engine, &bytes).expect("it compiles");
            wasm.modules.push(compiled);
            let given = Array::new(ctx.clone())?;
            given.set(0, "h")?;
            given.set(1, 0)?;
            let made = wasm.instantiate(&ctx, 0.0, &given)?;
            let exports = wasm.exports(&ctx, made.get(1)?)?;
            let g: f64 = exports.get(2)?;
            let none = Array::new(ctx.clone())?;
            for depth in 0..MAX_WASM_NESTING {
                let stopped = wasm.call(&ctx, g, &none, depth == 0)?;
                assert_eq!(stopped.get::<i32>(0)?, 1, "{depth}");
            }
            let refused = wasm.call(&ctx, g, &none, false)?;
            assert_eq!(
                refused.get::<String>(2)?,
                "Maximum call stack size exceeded"
            );
            // The outermost call resumed: those above it are dropped.
            let done = wasm.resume(&ctx, 0.0, &none)?;
            assert_eq!(done.get::<i32>(0)?, 0);
            assert!(wasm.pending.is_empty());
            // A call with none under way drops what an error left.
            wasm.call(&ctx, g, &none, true)?;
            wasm.call(&ctx, g, &none, false)?;
            wasm.call(&ctx, g, &none, true)?;
            assert_eq!(wasm.pending.len(), 1);
            Ok(())
        })
    }
}
