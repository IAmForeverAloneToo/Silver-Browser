//! The bindings loaded compiled (M35.12, `docs/design/performance.md`,
//! "A realm's bindings loaded compiled"): the bootstrap, compiled into
//! QuickJS's bytecode once a process by QuickJS's own writer, and read
//! by every realm after, rather than every realm parsing and compiling
//! its sixteen thousand lines again. The bootstrap is wrapped as a
//! module whose default export is its function, since rquickjs compiles
//! without running only a module; the function is the same.
//!
//! This is the one `unsafe` the crate writes, the exception the roadmap
//! asked for, reviewed (`docs/security.md`): QuickJS's reader trusts
//! what it reads, so rquickjs makes reading bytecode `unsafe`. What
//! [`bootstrap`] reads is only ever what [`compiled`] wrote: the
//! bootstrap compiled into this executable, written in this process by
//! the QuickJS that reads it, kept in a process-wide [`OnceLock`] that
//! is never written again nor freed, and never read from a file, a
//! pipe, another process or a page.

#![allow(unsafe_code)]

use std::sync::OnceLock;

use rquickjs::module::{WriteOptions, WriteOptionsEndianness};
use rquickjs::{Context, Ctx, Function, Module, Runtime};

/// The bootstrap's bytecode, once written; `None` if QuickJS would not
/// compile or write it, in which case each realm compiles the source as
/// before.
static BYTECODE: OnceLock<Option<Vec<u8>>> = OnceLock::new();

/// The bootstrap as a module whose default export is its function.
fn module_source() -> String {
    format!("export default {};\n", super::dom::BOOTSTRAP)
}

/// The bootstrap's bytecode: compiled the first time it is asked for,
/// in a runtime of its own, keeping its source and its lines as the
/// source run as a script keeps them (what a binding's `toString` and an
/// error's place say).
fn compiled() -> Option<&'static [u8]> {
    BYTECODE
        .get_or_init(|| {
            let runtime = Runtime::new().ok()?;
            let context = Context::full(&runtime).ok()?;
            context.with(|ctx| {
                let module =
                    Module::declare(ctx, super::dom::BOOTSTRAP_NAME, module_source()).ok()?;
                module
                    .write(WriteOptions {
                        endianness: WriteOptionsEndianness::Native,
                        allow_shared_array_buffer: false,
                        object_reference: false,
                        strip_source: false,
                        strip_debug: false,
                    })
                    .ok()
            })
        })
        .as_deref()
}

/// Compiles the bindings now, ahead of the first realm: a renderer does
/// it on a thread of its own as it starts, so that its first page's
/// first script does not wait for it.
pub fn warm() {
    let _ = compiled();
}

/// How many realms read the bindings' bytecode: for the tests.
#[cfg(test)]
static READ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The bootstrap's function in `ctx`: read from the bytecode, else, if
/// there is none, compiled from its source.
pub(super) fn bootstrap<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    let Some(bytes) = compiled() else {
        return ctx.eval(super::dom::BOOTSTRAP);
    };
    #[cfg(test)]
    READ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    // SAFETY: `bytes` is what `Module::write` wrote of the bootstrap, a
    // module this executable carries, in this process, by the QuickJS
    // that now reads it: valid bytecode, as `load` asks. It lives in a
    // static that is never written again or freed, so it outlives the
    // module, which QuickJS reads it as read-only data for.
    let module = unsafe { Module::load(ctx.clone(), bytes) }?;
    let (module, _) = module.eval()?;
    module.get("default")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{READ, compiled};

    /// The bindings are compiled and written once a process, and each
    /// realm made after reads them rather than compiling them (M35.12).
    #[test]
    fn the_bindings_are_written_once_and_read_by_each_realm() {
        let first = compiled().expect("the bootstrap compiles to bytecode");
        let again = compiled().expect("and keeps it");
        assert!(std::ptr::eq(first, again), "written once");
        let before = READ.load(Ordering::SeqCst);
        for _ in 0..2 {
            let mut realm = crate::tests::realm();
            let mut host = crate::tests::Recorder::default();
            let source = crate::tests::script("typeof document + ' ' + typeof Node");
            assert_eq!(
                realm.run(&source, &mut host).expect("it runs"),
                crate::Completion::String("object function".to_owned())
            );
        }
        assert!(READ.load(Ordering::SeqCst) >= before + 2);
    }
}
