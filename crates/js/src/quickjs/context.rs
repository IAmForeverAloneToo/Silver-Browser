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

#![allow(unsafe_code)]

use rquickjs::context::intrinsic::All;
use rquickjs::{Context, Runtime, qjs};

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
}
