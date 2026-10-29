//! Module scripts (D61, HTML section 8.1.3): rquickjs's loader on a
//! realm's runtime. Its resolver asks the host for each specifier (HTML's
//! "resolve a module specifier", the import map's), so that no name but
//! a URL, the realm's own stubs among them, reaches QuickJS's module map,
//! and the bootstrap's modules, which are in that map, are refused; its
//! loader serves only the sources the engine fetched and gave the realm.
//!
//! The engine finds a module's static imports by having the realm
//! declare it apart, in a context of its own that is dropped after, each
//! import resolved there to one empty module ([`State::scan`]). Declaring
//! a module in the realm resolves its imports, which the loader declares
//! in turn: a graph is declared whole, or not at all.
//!
//! `import()` of a module the realm was not given yet resolves to a stub
//! whose one export is `then`: the `import()`'s promise, resolved with
//! the stub's namespace, adopts it and gives the stub its resolving
//! functions, which the engine's [`crate::Task::Imported`] settles once
//! it has fetched the graph, as the module at its URL evaluates.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{Coerced, Ctx, Exception, Function, Module, Persistent, Promise, Value};

use super::dom::{BOOTSTRAP_NAME, Budget, GPU_BINDINGS_NAME, throw};
use super::{Entries, Slot, with_host};
use crate::{Answer, Call, MAX_SPECIFIER_BYTES, ModuleSource, ScriptingHost};

/// The one module every import resolves to while a module is declared
/// apart: a name no URL is.
const SCAN_NAME: &str = "\u{1}scan";

/// What separates an inline module script's name from its base URL, and
/// starts a stub's: a character no serialized URL holds.
pub(super) const MARK: char = '\u{1}';

/// A stub's text: its `then`, which the `import()`'s promise calls with
/// its resolving functions, gives them to the realm.
const STUB: &str = "export function then(resolve, reject) { import.meta.settle(resolve, reject); }";

/// `url` as a page knows it: an inline module's name without the mark
/// and number the realm added.
pub(super) fn page_url(url: &str) -> &str {
    url.split(MARK).next().unwrap_or(url)
}

/// Where an `import()` waiting for its graph stands.
enum Settling {
    /// Its promise adopted its stub: the functions that resolve it.
    Waiting(Persistent<Function<'static>>, Persistent<Function<'static>>),
    /// Its graph came first: the promise it is to follow.
    Ready(Persistent<Promise<'static>>),
}

/// What the realm and its loader share.
#[derive(Default)]
pub(super) struct State {
    /// The modules the engine gave and the realm has not declared, by
    /// URL: their base URL and text.
    store: RefCell<HashMap<String, (String, String)>>,
    /// The URLs of the modules declared: each is in QuickJS's module map,
    /// and evaluated once.
    declared: RefCell<HashSet<String>>,
    /// While a module is declared apart, the specifiers its imports name.
    scan: RefCell<Option<Vec<String>>>,
    /// While a module script's graph is declared: a URL the realm was not
    /// given fails, rather than waiting for a fetch.
    fixed: Cell<bool>,
    /// A URL the realm's own `import` names, taken as it is once.
    direct: RefCell<Option<String>>,
    /// The inline module scripts named so far.
    inline: Cell<u32>,
    /// The `import()`s that waited for a fetch so far, and those waiting.
    imports: Cell<u32>,
    settling: RefCell<HashMap<u32, Settling>>,
}

impl State {
    /// Keeps `modules` for the loader, but those declared already.
    pub(super) fn give(&self, modules: &[ModuleSource]) {
        let declared = self.declared.borrow();
        let mut store = self.store.borrow_mut();
        for module in modules {
            if !declared.contains(&module.url) {
                store.insert(
                    module.url.clone(),
                    (module.base.clone(), module.text.clone()),
                );
            }
        }
    }

    /// The name of the next inline module script whose base URL is
    /// `base`.
    pub(super) fn inline_name(&self, base: &str) -> String {
        let n = self.inline.get().wrapping_add(1);
        self.inline.set(n);
        format!("{base}{MARK}{n}")
    }

    /// Whether the module at `url` was declared.
    pub(super) fn declared(&self, url: &str) -> bool {
        self.declared.borrow().contains(url)
    }

    /// Marks the module script at `url` declared.
    pub(super) fn mark_declared(&self, url: &str) {
        self.declared.borrow_mut().insert(url.to_owned());
    }

    /// Takes the source given for `url`.
    pub(super) fn take(&self, url: &str) -> Option<(String, String)> {
        self.store.borrow_mut().remove(url)
    }

    /// Gives `url`'s source back: its declaration failed, and the next
    /// that asks for it fails as it did.
    pub(super) fn put_back(&self, url: &str, source: (String, String)) {
        self.store.borrow_mut().insert(url.to_owned(), source);
    }

    /// Starts or ends declaring a module script's graph.
    pub(super) fn set_fixed(&self, fixed: bool) {
        self.fixed.set(fixed);
    }

    /// Starts declaring a module apart: what its imports name is kept.
    pub(super) fn start_scan(&self) {
        *self.scan.borrow_mut() = Some(Vec::new());
    }

    /// Ends it: the specifiers its imports named.
    pub(super) fn end_scan(&self) -> Vec<String> {
        self.scan.borrow_mut().take().unwrap_or_default()
    }

    /// Lets the realm's own `import` of `url` take it as it is.
    pub(super) fn set_direct(&self, url: Option<String>) {
        *self.direct.borrow_mut() = url;
    }

    /// Drops the saved values, before the runtime goes.
    pub(super) fn clear(&self) {
        self.settling.borrow_mut().clear();
    }
}

/// Declares `text` as the module `name`, whose base URL is `base`: its
/// imports resolved and declared, and its `import.meta` given `url` and
/// `resolve` (HTML's HostGetImportMetaProperties).
pub(super) fn declare<'js, H: ScriptingHost>(
    ctx: &Ctx<'js>,
    host: &Slot<H>,
    budget: &Budget,
    name: &str,
    base: &str,
    text: &str,
) -> rquickjs::Result<Module<'js>> {
    let module = Module::declare(ctx.clone(), name, text)?;
    let meta = module.meta()?;
    meta.set("url", base)?;
    let (host, budget, base) = (host.clone(), budget.clone(), base.to_owned());
    let resolve = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, specifier: Coerced<String>| -> rquickjs::Result<String> {
            resolve(&ctx, &host, &budget, &base, &specifier.0)
        },
    )?
    .with_name("resolve")?;
    meta.set("resolve", resolve)?;
    Ok(module)
}

/// Declares the modules given, in their order, which the engine makes
/// each module's imports' before its own: each finds its imports
/// declared, rather than declaring them nested, so that a deep graph
/// takes no deeper stack than a shallow one. One that fails is kept for
/// the module that imports it, which fails as it did.
pub(super) fn declare_given<'a, H: ScriptingHost>(
    ctx: &Ctx<'_>,
    state: &State,
    host: &Slot<H>,
    budget: &Budget,
    modules: impl IntoIterator<Item = &'a ModuleSource>,
) {
    state.set_fixed(true);
    for module in modules {
        if let Some((base, text)) = state.take(&module.url) {
            match declare(ctx, host, budget, &module.url, &base, &text) {
                Ok(_) => state.mark_declared(&module.url),
                Err(_) => {
                    let _ = ctx.catch();
                    state.put_back(&module.url, (base, text));
                }
            }
        }
    }
    state.set_fixed(false);
}

/// `specifier` of the script or module named `base`, resolved by the
/// host: a URL, or the `TypeError` it throws. A name too long is refused
/// before the host is asked.
fn resolve<H: ScriptingHost>(
    ctx: &Ctx<'_>,
    host: &Slot<H>,
    budget: &Budget,
    base: &str,
    specifier: &str,
) -> rquickjs::Result<String> {
    if budget.exhausted.get() {
        return Err(Exception::throw_internal(ctx, "interrupted"));
    }
    if specifier.len() > MAX_SPECIFIER_BYTES {
        return Err(Exception::throw_type(
            ctx,
            &format!("Module specifiers are limited to {MAX_SPECIFIER_BYTES} bytes."),
        ));
    }
    let call = Call::ResolveModule {
        base: page_url(base).to_owned(),
        specifier: specifier.to_owned(),
    };
    let (answer, work) = with_host(ctx, host, |host| (host.dom(call), host.take_work()))?;
    budget.charge(work);
    match answer {
        Ok(Answer::String(url)) if url != BOOTSTRAP_NAME && url != GPU_BINDINGS_NAME => Ok(url),
        Ok(_) => Err(Exception::throw_type(
            ctx,
            &format!("Failed to resolve module specifier \"{specifier}\"."),
        )),
        Err(error) => Err(throw(ctx, &error)),
    }
}

/// The realm's resolver.
pub(super) struct ModuleResolver<H: ScriptingHost> {
    pub(super) state: Rc<State>,
    pub(super) host: Slot<H>,
    pub(super) budget: Budget,
}

impl<H: ScriptingHost> Resolver for ModuleResolver<H> {
    fn resolve<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        if let Some(found) = self.state.scan.borrow_mut().as_mut() {
            found.push(name.to_owned());
            return Ok(SCAN_NAME.to_owned());
        }
        if self.state.direct.borrow().as_deref() == Some(name) {
            return Ok(self.state.direct.take().unwrap_or_default());
        }
        // Import attributes name JSON and CSS modules, which are not
        // here (D61, section 2).
        if let Some(kind) = attributes.map(|a| a.get_type()).transpose()?.flatten() {
            return Err(Exception::throw_type(
                ctx,
                &format!("\"{kind}\" is not a valid module type."),
            ));
        }
        resolve(ctx, &self.host, &self.budget, base, name)
    }
}

/// The realm's loader.
pub(super) struct ModuleLoader<H: ScriptingHost> {
    pub(super) state: Rc<State>,
    pub(super) host: Slot<H>,
    pub(super) budget: Budget,
    pub(super) entries: Entries,
}

impl<H: ScriptingHost> Loader for ModuleLoader<H> {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js>> {
        if name == SCAN_NAME {
            return Module::declare(ctx.clone(), SCAN_NAME, "");
        }
        let state = self.state.clone();
        if let Some((base, text)) = state.take(name) {
            return match declare(ctx, &self.host, &self.budget, name, &base, &text) {
                Ok(module) => {
                    state.mark_declared(name);
                    Ok(module)
                }
                Err(error) => {
                    state.put_back(name, (base, text));
                    Err(error)
                }
            };
        }
        if state.fixed.get() {
            return Err(Exception::throw_type(
                ctx,
                &format!("Failed to load module script: {name}"),
            ));
        }
        // An `import()` of a module not fetched yet: the engine fetches
        // its graph, and the stub's promise waits for it.
        let id = state.imports.get().wrapping_add(1);
        state.imports.set(id);
        let call = Call::ImportModule {
            id,
            url: name.to_owned(),
        };
        if let Err(error) = with_host(ctx, &self.host, |host| host.dom(call))? {
            return Err(throw(ctx, &error));
        }
        let stub = Module::declare(ctx.clone(), format!("{MARK}import:{id}"), STUB)?;
        let entries = self.entries.clone();
        let settle = Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, resolve: Function<'js>, reject: Function<'js>| {
                adopted(&ctx, &state, &entries, id, resolve, reject)
            },
        )?;
        stub.meta()?.set("settle", settle)?;
        Ok(stub)
    }
}

/// `import()` `id`'s promise adopted its stub, whose `then` gave these
/// functions: they settle it as its graph's promise does, at once if
/// that came first.
fn adopted<'js>(
    ctx: &Ctx<'js>,
    state: &State,
    entries: &Entries,
    id: u32,
    resolve: Function<'js>,
    reject: Function<'js>,
) -> rquickjs::Result<()> {
    let ready = state.settling.borrow_mut().remove(&id);
    match ready {
        Some(Settling::Ready(promise)) => {
            follow(ctx, entries, promise.restore(ctx)?, resolve, reject)
        }
        Some(Settling::Waiting(..)) | None => {
            let waiting = Settling::Waiting(
                Persistent::save(ctx, resolve),
                Persistent::save(ctx, reject),
            );
            state.settling.borrow_mut().insert(id, waiting);
            Ok(())
        }
    }
}

/// Settles `import()` `id` as `promise` does, now if its promise adopted
/// its stub, else once it does.
pub(super) fn settle<'js>(
    ctx: &Ctx<'js>,
    state: &State,
    entries: &Entries,
    id: u32,
    promise: Promise<'js>,
) -> rquickjs::Result<()> {
    let waiting = state.settling.borrow_mut().remove(&id);
    match waiting {
        Some(Settling::Waiting(resolve, reject)) => follow(
            ctx,
            entries,
            promise,
            resolve.restore(ctx)?,
            reject.restore(ctx)?,
        ),
        Some(Settling::Ready(_)) | None => {
            let ready = Settling::Ready(Persistent::save(ctx, promise));
            state.settling.borrow_mut().insert(id, ready);
            Ok(())
        }
    }
}

/// `resolve` and `reject` called as `promise` settles, by the bootstrap's
/// own `then`, whatever a page did to `Promise.prototype`.
fn follow<'js>(
    ctx: &Ctx<'js>,
    entries: &Entries,
    promise: Promise<'js>,
    resolve: Function<'js>,
    reject: Function<'js>,
) -> rquickjs::Result<()> {
    let Some(entries) = entries.borrow().clone() else {
        return Ok(());
    };
    entries
        .restore(ctx)?
        .get::<_, Function<'_>>("moduleSettle")?
        .call::<_, Value<'_>>((promise, resolve, reject))?;
    Ok(())
}

/// A promise rejected with a `TypeError` of `message`, made as QuickJS
/// makes its own errors.
pub(super) fn rejected<'js>(ctx: &Ctx<'js>, message: &str) -> rquickjs::Result<Promise<'js>> {
    let (promise, _, reject) = Promise::new(ctx)?;
    let _ = Exception::throw_type(ctx, message);
    let error = ctx.catch();
    reject.call::<_, Value<'_>>((error,))?;
    Ok(promise)
}
