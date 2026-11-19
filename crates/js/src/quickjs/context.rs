//! A realm's context (G90): a context of its agent's runtime with the
//! intrinsics QuickJS-ng's `JS_NewContext` gives, `DOMException` its own
//! whichever of the runtime's contexts it is.
//!
//! `JS_NewContext` adds `DOMException` only through its base64 intrinsic
//! (`JS_AddIntrinsicAToB`), which skips it once the class is registered
//! in the runtime: every context after a runtime's first had none, and
//! the bindings, which take it, could not start in it. rquickjs's
//! `Context::custom` with all its intrinsics adds `DOMException` to each
//! context, but not the base64 functions (`atob`, `btoa`), and its
//! intrinsics are sealed: so the one call that adds them is made here,
//! the js crate's fourth module that allows `unsafe`, reviewed.
//!
//! And a realm made as a run of another realm of its agent first reaches
//! its document (G90 phase 3 (b), `docs/design/same-origin-frames.md`):
//! that run holds the runtime's lock, which `Context::custom` takes, so
//! its context is made with QuickJS's own calls under the lock held, the
//! same intrinsics in the same order, and entered without taking it.
//! The module's second reviewed block.

#![allow(unsafe_code)]

use std::ptr::NonNull;

use rquickjs::context::intrinsic::All;
use rquickjs::{Context, Ctx, Runtime, qjs};

/// A new context of `runtime` with the intrinsics `JS_NewContext` gives,
/// `DOMException` its own.
pub(super) fn new_context(runtime: &Runtime) -> rquickjs::Result<Context> {
    let context = Context::custom::<All>(runtime)?;
    let added = context.with(|ctx| {
        // SAFETY: the context was made just now, by this runtime, on this
        // thread, and nothing has run in it; this is the call
        // `JS_NewContext` makes of each new context, after the intrinsics
        // `Context::custom` added (`DOMException` among them, so that this
        // call leaves it be). It returns nonzero only when it could not
        // allocate, which is an error here.
        unsafe { qjs::JS_AddIntrinsicAToB(ctx.as_raw().as_ptr()) }
    });
    if added != 0 {
        return Err(rquickjs::Error::Allocation);
    }
    Ok(context)
}

/// A new context of the runtime of `anchor`, a context of the agent,
/// made while a run of `held`, a context of that runtime, is under way
/// on this thread and holds the runtime's lock (G90 phase 3 (b)): the
/// intrinsics [`new_context`] gives, in rquickjs's `All` order and then
/// the base64 ones, each call's failure an allocation error. Made, the
/// runtime's heap limit becomes `limit`, its agent's with the new realm,
/// at once: the run goes on reaching more. `None` when `held` and
/// `anchor` are not of one runtime.
pub(super) fn new_context_locked(
    held: &Ctx<'_>,
    anchor: &Context,
    limit: usize,
) -> Option<rquickjs::Result<Context>> {
    let rt = anchor.get_runtime_ptr();
    // SAFETY: `held` exists only inside a `Context::with` of its runtime
    // on this thread, which holds that runtime's lock for as long as
    // `held` lives: the runtime is not in use by any other caller, and
    // this one does not reenter rquickjs while the calls below run. That
    // runtime is `anchor`'s (checked first), whose `Runtime` owns the new
    // context. The new context is fresh, nothing has run in it; the calls
    // are those `Context::custom` with `All` and `JS_NewContext` make,
    // each checked. `Context::from_raw` takes the one reference
    // `JS_NewContextRaw` gave: the `Context` frees it as it drops, which
    // rquickjs does at once when the lock is held on this thread (its
    // `drop_context`), so a failure below frees what was made. The limit
    // is a field each allocation reads (`malloc_state.malloc_limit`),
    // which `Runtime::set_memory_limit` sets as this does, under the lock.
    unsafe {
        if qjs::JS_GetRuntime(held.as_raw().as_ptr()) != rt {
            return None;
        }
        let raw = NonNull::new(qjs::JS_NewContextRaw(rt))?;
        let context = Context::from_raw(raw, anchor.runtime().clone());
        let ctx = raw.as_ptr();
        // `Context::custom`'s base objects and `All`, in their order, then
        // the base64 functions; each nonzero is an allocation failed.
        let mut failed = qjs::JS_AddIntrinsicBaseObjects(ctx) != 0;
        failed |= qjs::JS_AddIntrinsicDate(ctx) != 0;
        failed |= qjs::JS_AddIntrinsicEval(ctx) != 0;
        qjs::JS_AddIntrinsicRegExpCompiler(ctx);
        failed |= qjs::JS_AddIntrinsicRegExp(ctx) != 0;
        failed |= qjs::JS_AddIntrinsicJSON(ctx) != 0;
        failed |= qjs::JS_AddIntrinsicProxy(ctx) != 0;
        failed |= qjs::JS_AddIntrinsicMapSet(ctx) != 0;
        failed |= qjs::JS_AddIntrinsicTypedArrays(ctx) != 0;
        failed |= qjs::JS_AddIntrinsicPromise(ctx) != 0;
        failed |= qjs::JS_AddPerformance(ctx) != 0;
        failed |= qjs::JS_AddIntrinsicWeakRef(ctx) != 0;
        failed |= qjs::JS_AddIntrinsicDOMException(ctx) != 0;
        failed |= qjs::JS_AddIntrinsicAToB(ctx) != 0;
        if failed {
            return Some(Err(rquickjs::Error::Allocation));
        }
        qjs::JS_SetMemoryLimit(rt, u64::try_from(limit).unwrap_or(u64::MAX));
        Some(Ok(context))
    }
}

/// Calls `f` in `context` while a run of `held`, a context of the same
/// runtime, is under way on this thread and holds the runtime's lock
/// (G90 phase 3 (b)), where `Context::with`, which takes the lock, would
/// panic: a realm made as it is reached installs its bindings so. `None`
/// when the two are not of one runtime.
pub(super) fn with_locked<R>(
    held: &Ctx<'_>,
    context: &Context,
    f: impl for<'js> FnOnce(Ctx<'js>) -> R,
) -> Option<R> {
    let raw = context.as_raw();
    // SAFETY: `held` exists only inside a `Context::with` of its runtime
    // on this thread, which holds that runtime's lock while `held` lives;
    // `context` is of that runtime (checked first), so the lock rquickjs
    // asks of `Ctx::from_raw` is held. The `Ctx` lives in `f` alone, whose
    // lifetime `'js` is its own and cannot leave it (`R` does not name
    // it), so it does not outlive the lock; it takes a reference to the
    // context, which it gives back as it drops.
    unsafe {
        if qjs::JS_GetRuntime(raw.as_ptr()) != qjs::JS_GetRuntime(held.as_raw().as_ptr()) {
            return None;
        }
        Some(f(Ctx::from_raw(raw)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names a context's global holds, enumerable or not, and those
    /// of each of their values and of each value's prototype: one per
    /// line, sorted.
    const NAMES: &str = "(() => { const out = []; \
        const name = (k) => typeof k === 'symbol' ? '@' + k.description : k; \
        for (const k of Reflect.ownKeys(globalThis)) { \
          out.push(name(k)); \
          let v; try { v = globalThis[k]; } catch (e) { continue; } \
          if (v === null || (typeof v !== 'object' && typeof v !== 'function')) continue; \
          for (const j of Reflect.ownKeys(v)) out.push(name(k) + '.' + name(j)); \
          let p; try { p = v.prototype; } catch (e) { continue; } \
          if (p !== null && typeof p === 'object') \
            for (const j of Reflect.ownKeys(p)) out.push(name(k) + '.prototype.' + name(j)); \
        } \
        return out.sort().join('\\n'); })()";

    fn names(context: &Context) -> String {
        context.with(|ctx| ctx.eval::<String, _>(NAMES).expect("it runs"))
    }

    /// A runtime's second and third contexts made here hold what a first
    /// context `JS_NewContext` made holds, name for name: the base64
    /// functions and `DOMException` both.
    #[test]
    fn a_context_holds_what_js_new_context_gives_whichever_it_is() {
        let old_runtime = Runtime::new().expect("a runtime");
        let old = Context::full(&old_runtime).expect("a context");
        let expected = names(&old);
        assert!(expected.lines().any(|line| line == "atob"));
        assert!(expected.lines().any(|line| line == "DOMException"));
        let runtime = Runtime::new().expect("a runtime");
        let first = new_context(&runtime).expect("a context");
        let second = new_context(&runtime).expect("another");
        let third = new_context(&runtime).expect("a third");
        for (n, context) in [first, second, third].iter().enumerate() {
            let got = names(context);
            if got != expected {
                let missing: Vec<&str> = expected
                    .lines()
                    .filter(|l| !got.lines().any(|g| g == *l))
                    .collect();
                let extra: Vec<&str> = got
                    .lines()
                    .filter(|l| !expected.lines().any(|e| e == *l))
                    .collect();
                panic!("context {n}: missing {missing:?}, extra {extra:?}");
            }
            let thrown: String = context.with(|ctx| {
                ctx.eval(
                    "try { atob('*'); 'no' } catch (e) { String(e instanceof DOMException) + ' ' + e.name }",
                )
                .expect("it runs")
            });
            assert_eq!(thrown, "true InvalidCharacterError", "context {n}");
        }
    }

    /// A realm's context made under a run's held lock (G90 phase 3 (b))
    /// holds what one made eagerly does: its global's names in their
    /// order, those of `Object.prototype`, `Function.prototype` and
    /// `Array.prototype`, and the type of each; should rquickjs's `All`
    /// change, the order copied by hand fails here.
    #[test]
    fn a_context_made_under_a_held_lock_holds_what_an_eager_one_does() {
        const SHAPE: &str = "(() => { const out = []; \
            const name = (k) => typeof k === 'symbol' ? '@' + k.description : k; \
            const of = (o, at) => { for (const k of Reflect.ownKeys(o)) { \
              let t; try { t = typeof o[k]; } catch (e) { t = 'throws'; } \
              out.push(at + name(k) + ':' + t); } }; \
            of(globalThis, ''); of(Object.prototype, 'Object.prototype.'); \
            of(Function.prototype, 'Function.prototype.'); \
            of(Array.prototype, 'Array.prototype.'); \
            return out.join('\\n'); })()";
        let shape = |context: &Context| -> String {
            context.with(|ctx| ctx.eval::<String, _>(SHAPE).expect("it runs"))
        };
        let runtime = Runtime::new().expect("a runtime");
        let eager = new_context(&runtime).expect("a context");
        let second = new_context(&runtime).expect("another");
        let reached = eager
            .with(|held| new_context_locked(&held, &eager, 0))
            .expect("of one runtime")
            .expect("a context");
        let expected = shape(&second);
        assert!(expected.lines().any(|line| line == "atob:function"));
        assert!(expected.lines().any(|line| line == "DOMException:function"));
        assert_eq!(shape(&reached), expected);
        // Entered under the held lock, it runs as when entered itself.
        let answer = eager.with(|held| {
            with_locked(&held, &reached, |ctx| {
                ctx.eval::<String, _>("typeof atob + ' ' + [1, 2].map(x => x * 2)")
                    .expect("it runs")
            })
        });
        assert_eq!(answer.as_deref(), Some("function 2,4"));
        // Another runtime's context is refused.
        let other = Runtime::new().expect("another runtime");
        let elsewhere = new_context(&other).expect("a context");
        assert!(
            eager
                .with(|held| new_context_locked(&held, &elsewhere, 0))
                .is_none()
        );
        assert!(
            eager
                .with(|held| with_locked(&held, &elsewhere, |_| ()))
                .is_none()
        );
    }
}
