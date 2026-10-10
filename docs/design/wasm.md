# Design note: WebAssembly

Roadmap M29. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ends: where they
disagree, the code wins, and the places are marked *(corrected)*. What
ships is described in `docs/architecture.md` "Current state".

A page's scripts run in QuickJS-ng (D1, M24), which has no
WebAssembly: `typeof WebAssembly` is `"undefined"`, and a page that
loads a module, a Rust program built with `wasm-bindgen`, a C program
built with Emscripten, a library that ships its hot loop as a module,
fails at its first line about it. The roadmap saw this when it chose
QuickJS-ng: M29.1 names the contingency, a WebAssembly engine bridged
into the realm, or the change of JavaScript engine D1 left open.

This milestone takes the contingency. It gives the realm the
`WebAssembly` namespace of the JavaScript interface, with `Module`,
`Instance`, `Memory`, `Table`, `Global` and their errors, the
promise-returning `compile` and `instantiate` and their streaming
forms, over an interpreter that runs inside the realm's limits: a
module that traps throws, one that loops is stopped as a script that
loops is, one that asks for more memory than the page may have is
refused, and nothing a module does ends the engine. Its acceptance is
the roadmap's: a small module, a Rust function compiled to
WebAssembly, takes numbers from a page's script and gives them back.

The standards are read at the versions current when this note was
written (11 October 2026):

- the WebAssembly Core Specification, Release 2.0, with the 3.0
  features named below: the binary format, validation and execution
  (traps, the instructions' semantics, the limits an embedder may
  set);
- the WebAssembly JavaScript Interface (Editor's Draft): the namespace
  and its methods, the classes and their constructors, the conversions
  between JavaScript values and WebAssembly's (`ToWebAssemblyValue`,
  `ToJSValue`), the exported function objects, the errors;
- the WebAssembly Web API (Editor's Draft): `compileStreaming` and
  `instantiateStreaming` over a `Response`, its MIME type and status;
- Web IDL for the classes' shape, and the HTML Standard for the
  promise jobs the asynchronous methods settle in.

What the standards leave to the implementation, or state but are worth
checking, was put to the reference browser (Chrome 154.0.8037.99,
headless) before this note was written, with the web-platform tests'
own `wasm-module-builder.js` building the modules (scratchpad
`m29/pages/p1-wasm.html`):

- `WebAssembly` is `[object WebAssembly]`, its members `CompileError`,
  `Global`, `Instance`, `LinkError`, `Memory`, `Module`,
  `RuntimeError`, `Table`, `compile`, `compileStreaming`,
  `instantiate`, `instantiateStreaming`, `validate`, and those of
  proposals past the core (`Exception`, `Tag`, `JSTag`, `Suspending`,
  `SuspendError`, `promising`);
- an exported function is a function named by its index (`"0"`), its
  `length` its parameters' count, `function 0() { [native code] }` as
  a string, with no `prototype`; the same function object each time an
  export is read; arguments convert as `ToInt32` and `ToNumber` say
  (`add(2.7, "4")` is 6, `add(2 ** 31, 1)` is -2147483647, `add()` is
  0); an `i64` takes a `BigInt` only ("Cannot convert 5 to a BigInt"),
  wrapping as `BigInt.asIntN(64)`; an `f32` comes back rounded to
  single precision;
- traps throw `WebAssembly.RuntimeError`, in these words: `unreachable`,
  `divide by zero`, `divide result unrepresentable`, `memory access out
  of bounds`, `float unrepresentable in integer range`, `null
  function`, `table index is out of bounds`; a recursion too deep
  throws `RangeError: Maximum call stack size exceeded`;
- a module that does not decode or validate throws `CompileError`
  ("WebAssembly.Module(): expected 4 bytes, fell off end @+0"); an
  argument that is no buffer source a `TypeError`
  ("WebAssembly.Module(): Argument 0 must be a buffer source"); an
  import object missing, or a module of it, a `TypeError`
  ('WebAssembly.Instance(): Import #0 "env": module is not an object
  or function'); an import of the wrong kind a `LinkError`
  ('WebAssembly.Instance(): Import #0 "env" "f": function import
  requires a callable');
- an imported JavaScript function's result converts as the import's
  type says (`x * 2 + 0.5` returned to an `i32` is truncated); what it
  throws reaches the script that called into the module, the same
  object;
- a `Memory`'s `buffer` is the same object until the memory grows,
  then detached (`byteLength` 0) and a new one made; growing past the
  maximum is a `RangeError` ("WebAssembly.Memory.grow(): Maximum memory
  size exceeded"); an `initial` past 65,536 pages a `RangeError`; what a
  script writes in the buffer a module reads, and what the module
  stores the script's view sees;
- a `Global` reads and writes its `value`, an immutable one refusing
  ("Can't set the value of an immutable global."), an `i64` one a
  `BigInt`; a `Table` of functions has `length`, `get`, `set`, `grow`,
  `get` past its end a `RangeError`;
- `Module.exports`, `Module.imports` and `Module.customSections`
  describe a module; `instantiate` of bytes gives `{module, instance}`,
  of a module an `Instance`; `compile` of bad bytes rejects with a
  `CompileError`; `instantiateStreaming` of a response whose type is
  not `application/wasm` rejects with a `TypeError` ("Incorrect
  response MIME type. Expected 'application/wasm'.").

## 1. Decisions

**The engine (D21, M29.1).** WebAssembly runs in `wasmi` 2.0 (MIT or
Apache-2.0), an interpreter in Rust: it validates every module before
it runs a byte of it, translates each function to its own register
machine code, and executes on stacks of its own, so that a module's
recursion is a trap and never the native stack. It meters execution in
fuel, takes limits on what a store may allocate, and can suspend a call
at a host function and resume it, which is how a module calls a page's
JavaScript here (below). Rather than `wasmtime` (Cranelift's JIT:
executable memory in the process that runs pages, which is M34's
sandbox to decide, a dependency tree several times this workspace's,
and `unsafe` throughout) or the change of JavaScript engine D1 left
open (SpiderMonkey or V8 bring their own WebAssembly, and everything
else of theirs). An interpreter is slower than a JIT, by about ten
times on compute; that is the cost of this decision, measured below,
and a JIT is a question for when the sandbox exists. `wasmi` has
`unsafe` of its own, in its executor, which M33.5's audit takes; the
`js` crate over it still forbids any. Its features: the core 2.0 set
(mutable globals, sign extension, saturating float-to-int conversions,
multi-value, bulk memory, reference types) and 3.0's tail calls and
extended constant expressions, what compilers emit today; not SIMD,
memory64, multiple memories, threads, exceptions or GC (section 2).

- *Where it lives.* In `js`, beside QuickJS-ng: a module's functions
  and a page's are called from each other in one realm, on one thread,
  inside one budget, so the bridge is the realm's. Each realm keeps one
  `wasmi` store, made at its first module: its modules, instances,
  memories, tables, globals and functions, each known to the
  bootstrap's objects by a handle, as nodes are. Nothing of it reaches
  the engine: a module is bytes a script already holds.
- *The JavaScript interface* is the bootstrap's, over natives of the
  `js` crate's own (`wasmCompile`, `wasmInstantiate`, `wasmCall`, ...),
  not the host's: the classes, their constructors' and methods'
  conversions and checks, in the reference browser's words, the error
  classes `CompileError`, `LinkError` and `RuntimeError` deriving from
  `Error` as the interface says.
- *Exported functions* are made by the bootstrap, one per function the
  store has, kept in a map so that the same function is the same
  object: `name` the function's index, `length` its parameters,
  calling it converting each argument by `ToWebAssemblyValue` (`i32`
  by `ToInt32`, `i64` by `ToBigInt64`, `f32` and `f64` by `ToNumber`,
  `externref` any value, `funcref` an exported function or `null`) and
  each result by `ToJSValue` (several results an array).
- *Imports.* Instantiation reads the import object in the module's
  order, as the interface says, with the reference browser's messages:
  a function import takes any callable, an exported function by its
  own function; a global takes a `Global` or a number (`BigInt` for
  `i64`); a memory a `Memory`, a table a `Table`, their limits checked.
  A JavaScript function becomes a host function of the store which,
  called, stops the module: `wasmi`'s resumable call returns to the
  native with the arguments, the native calls the function with them
  in the realm, converts its result by the import's type, and resumes
  the module with it. What the function throws ends the call and
  reaches the script, the same value. A module's call into the page's
  code and the page's call back into a module nest as deep as the
  realm's stack lets them.

**The memory a script sees (M29.2).** A module's linear memory is the
store's. A script sees it as `memory.buffer`, an `ArrayBuffer`, and
here the two cannot be one: `rquickjs` gives no safe way to lend a
buffer's bytes to Rust or Rust's to a buffer, and the `js` crate
writes no `unsafe`. So the buffer is a mirror, kept the same as the
memory where it can be seen to differ:

- a memory no script has asked the buffer of has no mirror, and costs
  nothing;
- when a call leaves the module (it returns, traps, or calls a
  JavaScript import), the memory's pages that differ from what the
  mirror last held are written into the mirror in place, by
  `Uint8Array.prototype.setFromBase64` (QuickJS-ng's own, at the speed
  of C), so that a view a script holds sees them;
- when a call enters the module (a script calls an export, or an
  import returns), the mirror's bytes are read by `toBase64` and the
  pages that differ from the memory are written into it;
- when the memory grows, by the module or by `grow()`, the mirror is
  detached, as the interface says, and the next `buffer` is new.

A call into a module whose memory a script has seen therefore costs
the reading of the whole mirror, about a millisecond for each
megabyte here. A module of a few megabytes called a few times a frame
is served; one of hundreds of megabytes called in a loop is not. The
copy is what this milestone can do without `unsafe`, and it is
measured; sharing the bytes is G79.

**Traps and limits (M29.3).** Nothing a module does ends the engine:

- *Traps* throw `WebAssembly.RuntimeError` with the reference
  browser's messages; `wasmi`'s stack overflow throws `RangeError`
  ("Maximum call stack size exceeded"), as the reference browser's
  does; a call is unwound and the store stays usable.
- *Time.* A module runs on the run's budget: each entry is given the
  fuel the steps left buy, `WASM_FUEL_PER_STEP` a step, and what it
  burns is charged to the run as steps. Out of fuel, the run is spent,
  and the realm's interrupt ends it at its next check, as a script that
  loops is ended: the page's scripts are stopped (M24).
- *Memory.* What a realm's modules allocate is bounded by
  `MAX_WASM_MEMORY` in all, its memories and tables counted, a
  `ResourceLimiter` of the store's refusing past it: a memory too
  large to make is a `RangeError`, a `memory.grow` past it returns -1
  to the module, as the core specification lets an embedder decide.
  `MAX_WASM_INSTANCES`, `MAX_WASM_TABLE_ELEMENTS` and
  `MAX_WASM_MODULE_BYTES` bound the rest.
- *A panic* of `wasmi`'s, which a valid module should never cause, is
  caught at the bridge: the call throws, and the realm's store is not
  used again, its modules throwing for the rest of the page's life.
- *Compilation* of a large module is work the run is charged for, by
  its bytes; functions are translated lazily, at their first call, so
  that a large module starts as fast as it validates.

**The asynchronous interface.** `compile` and `instantiate` resolve in
a promise job, compiling there, on the realm's thread: there is no
background compilation (a worker's thread is M31's). `compileStreaming`
and `instantiateStreaming` take a `Response` or a promise of one, check
its MIME type (`application/wasm`) and status as the Web API says, in
the reference browser's words, and compile its body once it has all
come; compiling while it arrives is M38's streaming bodies (G67).

**Limits (the bounds a hostile page cannot pass).**

| Constant | Value | Bounds |
|---|---|---|
| `WASM_FUEL_PER_STEP` | 100,000 | a module's work a step buys |
| `MAX_WASM_MEMORY` | 256 MiB | a realm's memories and tables |
| `MAX_WASM_MODULE_BYTES` | 64 MiB | a module compiled |
| `MAX_WASM_INSTANCES` | 1,000 | a realm's instances |
| `MAX_WASM_TABLE_ELEMENTS` | 10,000,000 | a table's length |
| `MAX_WASM_NESTING` | 64 | calls between a module and scripts, nested |

The store keeps what it made for the realm's life: `wasmi` frees
nothing before its store goes, and a page's modules go with its realm.
The bounds above are what holds a hostile page.

**Tests.** The `js` crate's tests build modules from bytes written in
the tests (`wat`, the text format's crate, a dev-dependency) and drive
the interface through scripts, as the realm's other tests do: each
class and method, each conversion, each trap and error and its words,
imports calling the page's code and back, the memory's mirror both
ways and after a grow, the budget stopping a loop, the limits refusing
a memory, a table and a module too large.

**Fuzzing.** A new target, `wasm`: modules the input makes, valid ones
by `wasm-smith` and any bytes besides, compiled, instantiated with
imports of every kind, their exports called with arguments the input
says, under small limits: nothing panics, every failure is one of the
interface's errors, and the realm runs a script after. `dom_api` gains
the interface's calls on modules of the target's making.

**The web-platform tests.** The interface's tests (`wasm/jsapi`), which
build their modules with `wasm-module-builder.js`, and the core
specification's own tests run through the interface (`wasm/core`),
those under 256 KiB: the float tables beside them, megabytes each, are
left out by size, a new clause of the pull's rule.

**Golden cases** continue from 129: a page whose boxes are sized by
what a module computed, with its memory read and written by the page's
script, captured from the reference browser.

**Acceptance.** The roadmap's: a Rust function compiled to WebAssembly
(`wasm32-unknown-unknown`, its module in the tests) given numbers by a
page's script and giving them back, in the engine's tests and in the
real shell; a module that traps, and one that loops, leaving the page
running and its scripts stopped as M24 stops them. A benchmark,
`benches/wasm.rs`: a module's compute against the same in a script, a
call's cost across the bridge, with and without a memory a script has
seen, and a module compiled.

## 2. Not in this milestone, said rather than hidden

- *SIMD* (`v128`, relaxed SIMD), *memory64* and *multiple memories*.
  G80.
- *Threads*: shared memories and atomics, with `SharedArrayBuffer`,
  which the realm does not have. M31, with workers.
- *Exception handling* (`Tag`, `Exception`, `JSTag`) and *GC*. G80.
- *JavaScript promise integration* (`Suspending`, `promising`) and the
  *JS string builtins*. G80.
- *ES module integration* (`import` of a `.wasm`). With module
  scripts, G41.
- *A JIT.* M34, with the sandbox; an interpreter until then.
- *Sharing the memory's bytes* with the script's `ArrayBuffer`. G79.
- *Compilation off the realm's thread, while the bytes arrive, or
  cached* (in IndexedDB, M36). M31 and M38.
- *Debugging a module*: names, source maps, stepping. M36's tools.

## 3. Order of work

Each step a vertical slice, with its tests, a sabotage pass and the
gate.

1. *Modules*: `wasmi` in `js`, the realm's store, `validate`,
   `Module` and its reflection, `CompileError`, the limits on a module.
2. *Instances*: imports of every kind, the resumable calls into the
   page's code, exports, exported functions and their conversions,
   traps, `LinkError`, `RuntimeError`, the start function.
3. *Memory, Table and Global*, and the memory's mirror.
4. *The budget and the limits*: fuel, nesting, the resource limiter,
   the panic guard, hostile modules.
5. *The asynchronous interface*: `compile`, `instantiate`, and their
   streaming forms.
6. *Fuzzing*: the `wasm` target, `dom_api`'s calls.
7. *The web-platform tests*: `wasm/jsapi` and `wasm/core`.
8. *Golden cases* from 129.
9. *Acceptance*: the Rust function in the engine and the real shell,
   the benchmark, T4.3, fuzz runs.
10. *The documents*.
