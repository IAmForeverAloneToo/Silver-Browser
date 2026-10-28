//! The bindings loaded compiled (M35.12, `docs/design/performance.md`,
//! "A realm's bindings loaded compiled"): the bootstrap, compiled into
//! QuickJS's bytecode once a process by QuickJS's own writer, and read
//! by every realm after, rather than every realm parsing and compiling
//! its sixteen thousand lines again. The bootstrap is wrapped as a
//! module whose default export is its function, since rquickjs compiles
//! without running only a module; the function is the same. WebGPU and
//! WebGL's bindings (M41) are compiled so too, apart, and read into a
//! realm only when its page first uses them.
//!
//! This is the one `unsafe` the crate writes, the exception the roadmap
//! asked for, reviewed (`docs/security.md`): QuickJS's reader trusts
//! what it reads, so rquickjs makes reading bytecode `unsafe`. What
//! [`read`] reads is only ever what [`compiled`] wrote: bindings
//! compiled into this executable, written in this process by the
//! QuickJS that reads them, kept in a process-wide [`OnceLock`] that is
//! never written again nor freed, and never read from a file, a pipe,
//! another process or a page.

#![allow(unsafe_code)]

use std::sync::OnceLock;

use rquickjs::module::{WriteOptions, WriteOptionsEndianness};
use rquickjs::{Context, Ctx, Function, Module, Runtime};

/// Bindings compiled into this executable: their module's name, their
/// source (a function), and their bytecode once written (`None` if
/// QuickJS would not compile or write it, in which case each realm
/// compiles the source as before).
struct Bindings {
    name: &'static str,
    source: &'static str,
    bytecode: OnceLock<Option<Vec<u8>>>,
}

static BOOTSTRAP: Bindings = Bindings {
    name: super::dom::BOOTSTRAP_NAME,
    source: super::dom::BOOTSTRAP,
    bytecode: OnceLock::new(),
};

static GPU: Bindings = Bindings {
    name: super::dom::GPU_BINDINGS_NAME,
    source: super::dom::GPU_BINDINGS,
    bytecode: OnceLock::new(),
};

/// `bindings`' bytecode: compiled the first time it is asked for, in a
/// runtime of its own, as a module whose default export is their
/// function, keeping its lines as the source run as a script keeps them
/// (what an error's place says) but not its source, so that a binding's
/// `toString` says it is native code (D47, gap batch A).
fn compiled(bindings: &'static Bindings) -> Option<&'static [u8]> {
    bindings
        .bytecode
        .get_or_init(|| {
            let runtime = Runtime::new().ok()?;
            let context = Context::full(&runtime).ok()?;
            context.with(|ctx| {
                let source = format!("export default {};\n", bindings.source);
                let module = Module::declare(ctx, bindings.name, source).ok()?;
                module
                    .write(WriteOptions {
                        endianness: WriteOptionsEndianness::Native,
                        allow_shared_array_buffer: false,
                        object_reference: false,
                        strip_source: true,
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
    let _ = compiled(&BOOTSTRAP);
    let _ = compiled(&GPU);
}

/// How many realms read the bindings' bytecode: for the tests.
#[cfg(test)]
static READ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// `bindings`' function in `ctx`: read from their bytecode, else, if
/// there is none, compiled from their source.
fn read<'js>(ctx: &Ctx<'js>, bindings: &'static Bindings) -> rquickjs::Result<Function<'js>> {
    let Some(bytes) = compiled(bindings) else {
        return ctx.eval(bindings.source);
    };
    #[cfg(test)]
    READ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    // SAFETY: `bytes` is what `Module::write` wrote of bindings this
    // executable carries, in this process, by the QuickJS that now reads
    // it: valid bytecode, as `load` asks. It lives in a static that is
    // never written again or freed, so it outlives the module, which
    // QuickJS reads it as read-only data for.
    let module = unsafe { Module::load(ctx.clone(), bytes) }?;
    let (module, _) = module.eval()?;
    module.get("default")
}

/// The bootstrap's function in `ctx`.
pub(super) fn bootstrap<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    read(ctx, &BOOTSTRAP)
}

/// WebGPU and WebGL's bindings' function in `ctx` (M41).
pub(super) fn gpu_bindings<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    read(ctx, &GPU)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{BOOTSTRAP, GPU, READ, compiled};

    /// The bindings are compiled and written once a process, and each
    /// realm made after reads them rather than compiling them (M35.12).
    #[test]
    fn the_bindings_are_written_once_and_read_by_each_realm() {
        let first = compiled(&BOOTSTRAP).expect("the bootstrap compiles to bytecode");
        let again = compiled(&BOOTSTRAP).expect("and keeps it");
        assert!(std::ptr::eq(first, again), "written once");
        compiled(&GPU).expect("WebGPU and WebGL's bindings compile to bytecode");
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

    /// WebGPU and WebGL's names are on a new window before their use,
    /// and their bindings are read at it (M41, T4.3).
    #[test]
    fn the_gpu_bindings_are_read_at_their_first_use() {
        let mut realm = crate::tests::realm();
        let mut host = crate::tests::Recorder::default();
        let source = crate::tests::script(
            "const before = typeof Object.getOwnPropertyDescriptor(window, 'GPUBuffer').get;\
             [before, typeof GPUBuffer, typeof Object.getOwnPropertyDescriptor(window, 'GPUBuffer').get,\
             typeof WebGL2RenderingContext.prototype.drawArrays, typeof GPUBufferUsage.MAP_READ].join(' ')",
        );
        assert_eq!(
            realm.run(&source, &mut host).expect("it runs"),
            crate::Completion::String("function function undefined function number".to_owned())
        );
        // An exception from their bindings has the page's frames in its
        // stack, not theirs, as the bootstrap's has.
        let thrown = realm
            .run(&crate::tests::script("\n\nnew GPUBuffer();"), &mut host)
            .expect_err("an illegal constructor");
        let places: Vec<(&str, u32)> = thrown
            .stack
            .iter()
            .map(|frame| (frame.url.as_str(), frame.line))
            .collect();
        assert_eq!(places, [("https://example.com/page.html", 3)]);
    }
}
