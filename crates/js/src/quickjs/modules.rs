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
//!
//! A module is named by its URL and its kind ([`crate::module_name`]),
//! so that a JSON, CSS or text module is not the JavaScript one at its
//! URL; those three are made here, their one export their value.
//!
//! An `import()` resolves against its referencing script (HTML's
//! HostLoadImportedModule with the active script) and fetches with that
//! script's options: each script and module run is kept by the name
//! QuickJS-ng knows its code by. Code a string compiled (`eval`,
//! `Function`) is named alike by QuickJS-ng, so a promise made by such
//! code, an `import()`'s among them, keeps the script on the stack
//! below it; with none there, the script that ran last stands in.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use rquickjs::function::Opt;
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{Atom, Coerced, Ctx, Exception, Function, Module, Persistent, Promise, Value};

use super::dom::{Budget, is_bindings, throw};
use super::{Entries, Slot, with_host};
use crate::{
    Answer, Call, MAX_SPECIFIER_BYTES, MODULE_MARK, ModuleKind, ModuleRequest, ModuleSource,
    ScriptingHost, module_name, module_of_name,
};

/// The one module every import resolves to while a module is declared
/// apart: a name no URL is.
const SCAN_NAME: &str = "\u{1}scan";

/// What separates an inline module script's name from its base URL, and
/// starts a stub's: a character no serialized URL holds.
pub(super) const MARK: char = MODULE_MARK;

/// The name QuickJS-ng gives the code a string compiled: `eval`'s and
/// the `Function` constructor's.
pub(super) const INPUT: &str = "<input>";

/// The most scripts and modules a realm keeps as referrers: past them a
/// new one's `import()`s resolve against its own URL with the default
/// options.
const MAX_REFERRERS: usize = 8192;

/// How many frames up the stack the active script is looked for.
const MAX_ACTIVE_DEPTH: isize = 64;

/// The most active scripts kept for promises made by code a string
/// compiled, until the jobs run.
const MAX_INPUT: usize = 256;

/// A stub's text: its `then`, which the `import()`'s promise calls with
/// its resolving functions, gives them to the realm.
const STUB: &str = "export function then(resolve, reject) { import.meta.settle(resolve, reject); }";

/// `url` as a page knows it: an inline module's name without the mark
/// and number the realm added.
pub(super) fn page_url(url: &str) -> &str {
    url.split(MARK).next().unwrap_or(url)
}

/// A script or module as its `import()`s and `import.meta.resolve` see
/// it (HTML's referencing script): the base URL they resolve against,
/// empty for the document's, and the host's number of its fetch options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Referrer {
    pub(super) base: String,
    pub(super) options: u32,
}

/// Why an import's attributes are refused: a key other than `type`, or
/// a type unknown.
enum Refused {
    Key(String),
    Type(String),
}

/// The kind an import's `attributes` ask for, or why they are refused.
fn requested(
    attributes: Option<&ImportAttributes<'_>>,
) -> rquickjs::Result<Result<ModuleKind, Refused>> {
    let Some(attributes) = attributes else {
        return Ok(Ok(ModuleKind::Js));
    };
    for key in attributes.keys() {
        let key = key?;
        if key != "type" {
            return Ok(Err(Refused::Key(key)));
        }
    }
    Ok(match attributes.get_type()? {
        None => Ok(ModuleKind::Js),
        Some(kind) => ModuleKind::of_type(&kind).ok_or(Refused::Type(kind)),
    })
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
    /// While a module is declared apart, the imports it names.
    scan: RefCell<Option<Vec<ModuleRequest>>>,
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
    /// The scripts and modules run or declared, by their names: what
    /// their `import()`s resolve against and fetch with.
    referrers: RefCell<HashMap<String, Referrer>>,
    /// The name of the script that ran last: the active script of code
    /// with none on the stack (a promise's job).
    current: RefCell<String>,
    /// The active scripts of the code strings compiled, as the promises
    /// made in it found them, in order: an `import()`'s among them,
    /// which its resolution takes.
    input: RefCell<VecDeque<Option<String>>>,
    /// The fetch options of the `import()` the resolver last resolved,
    /// which the loader asks the engine with.
    asking: Cell<u32>,
    /// The fetch options the modules being declared take: their
    /// graph's root's.
    giving: Cell<u32>,
    /// The fetch options of the `import()`s waiting for their graphs,
    /// which the modules they are given take.
    import_options: RefCell<HashMap<u32, u32>>,
    /// Whether CSS modules may be imported: a window's realm only
    /// (HTML's "module type allowed").
    pub(super) css: Cell<bool>,
    /// The error each module that failed to parse or resolve threw (its
    /// module script's parse error), thrown again, the same object, by
    /// each graph that holds it.
    failed: RefCell<HashMap<String, Persistent<Value<'static>>>>,
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

    /// Ends it: the imports it named.
    pub(super) fn end_scan(&self) -> Vec<ModuleRequest> {
        self.scan.borrow_mut().take().unwrap_or_default()
    }

    /// Keeps the script or module `name` as a referrer: the one it
    /// replaces.
    pub(super) fn note(&self, name: &str, referrer: Referrer) -> Option<Referrer> {
        let mut referrers = self.referrers.borrow_mut();
        if referrers.len() < MAX_REFERRERS || referrers.contains_key(name) {
            return referrers.insert(name.to_owned(), referrer);
        }
        None
    }

    /// The script `name` runs: the active script while no other is.
    pub(super) fn ran(&self, name: &str) {
        name.clone_into(&mut self.current.borrow_mut());
    }

    /// The name of the script that ran last.
    pub(super) fn current(&self) -> String {
        self.current.borrow().clone()
    }

    /// The modules declared from now on take the fetch options `options`.
    pub(super) fn set_giving(&self, options: u32) {
        self.giving.set(options);
    }

    /// The fetch options `import()` `id` asked with, its graph come.
    pub(super) fn imported(&self, id: u32) -> u32 {
        self.import_options.borrow_mut().remove(&id).unwrap_or(0)
    }

    /// The referrer of code named `name`: code a string compiled stands
    /// for the active script its next promise found, the bindings' for
    /// the script that ran last; a handler's (a name with the mark the
    /// realm did not keep) for the document, with the default options.
    pub(super) fn referrer(&self, name: &str) -> Referrer {
        let name = if name == INPUT {
            let input = self.input.borrow_mut().pop_front().flatten();
            input.unwrap_or_else(|| self.current())
        } else if is_bindings(name) {
            self.current()
        } else {
            name.to_owned()
        };
        if let Some(found) = self.referrers.borrow().get(&name) {
            return found.clone();
        }
        if name.contains(MARK) {
            return Referrer::default();
        }
        Referrer {
            base: page_url(&name).to_owned(),
            options: 0,
        }
    }

    /// Lets the realm's own `import` of `url` take it as it is.
    pub(super) fn set_direct(&self, url: Option<String>) {
        *self.direct.borrow_mut() = url;
    }

    /// Forgets the active scripts kept for promises: the jobs ran.
    pub(super) fn jobs_ran(&self) {
        self.input.borrow_mut().clear();
    }

    /// Drops the saved values, before the runtime goes.
    pub(super) fn clear(&self) {
        self.settling.borrow_mut().clear();
        self.failed.borrow_mut().clear();
    }
}

/// A promise was made (the runtime's hook): one made by code a string
/// compiled, an `import()`'s among them, keeps the active script that
/// code runs for, which such an `import()` resolves against.
pub(super) fn promise_made(ctx: &Ctx<'_>, state: &State) {
    let Some(name) = ctx.script_or_module_name(0) else {
        return;
    };
    if Atom::from_str(ctx.clone(), INPUT).is_ok_and(|input| input == name) {
        let found = active(ctx, 1);
        let mut input = state.input.borrow_mut();
        if input.len() == MAX_INPUT {
            input.pop_front();
        }
        input.push_back(found);
    }
}

/// The active script from `from` frames up the stack (HTML's
/// GetActiveScriptOrModule): the name of the first script's or module's
/// code, past natives', the bindings', stubs' and code a string compiled.
pub(super) fn active(ctx: &Ctx<'_>, from: isize) -> Option<String> {
    let input = Atom::from_str(ctx.clone(), INPUT).ok()?;
    for level in from..MAX_ACTIVE_DEPTH {
        let Some(name) = ctx.script_or_module_name(level) else {
            continue;
        };
        if name == input {
            continue;
        }
        let Ok(name) = name.to_string() else {
            continue;
        };
        if !is_bindings(&name) && !name.starts_with(MARK) {
            return Some(name);
        }
    }
    None
}

/// Declares `text` as the module `name`, whose base URL is `base`: its
/// imports resolved and declared, and its `import.meta` given `url` and
/// `resolve` (HTML's HostGetImportMetaProperties). A JSON, CSS or text
/// module is made as [`synthetic`] makes it.
pub(super) fn declare<'js, H: ScriptingHost>(
    ctx: &Ctx<'js>,
    host: &Slot<H>,
    budget: &Budget,
    entries: &Entries,
    name: &str,
    base: &str,
    text: &str,
) -> rquickjs::Result<Module<'js>> {
    let kind = module_of_name(name).1;
    if kind != ModuleKind::Js {
        return synthetic(ctx, entries, name, base, text, kind);
    }
    let module = Module::declare(ctx.clone(), name, text)?;
    let meta = module.meta()?;
    meta.set("url", base)?;
    let (host, budget, base) = (host.clone(), budget.clone(), base.to_owned());
    let resolve = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, specifier: Opt<Coerced<String>>| -> rquickjs::Result<String> {
            let specifier = specifier.0.map_or_else(|| "undefined".to_owned(), |s| s.0);
            resolve(&ctx, &host, &budget, &base, &specifier)
        },
    )?
    .with_name("resolve")?;
    meta.set("resolve", resolve)?;
    Ok(module)
}

/// [`declare`], a module's failure kept: one that failed before throws
/// what it threw then (HTML: a module script's parse error is its own,
/// rethrown by each graph that holds it).
pub(super) fn declare_kept<'js, H: ScriptingHost>(
    ctx: &Ctx<'js>,
    state: &State,
    (host, budget, entries): (&Slot<H>, &Budget, &Entries),
    name: &str,
    base: &str,
    text: &str,
) -> rquickjs::Result<Module<'js>> {
    let kept = state.failed.borrow().get(name).cloned();
    if let Some(error) = kept {
        let error = error.restore(ctx)?;
        return Err(ctx.throw(error));
    }
    match declare(ctx, host, budget, entries, name, base, text) {
        Err(rquickjs::Error::Exception) if !budget.exhausted.get() => {
            let error = ctx.catch();
            // Only its own: one an import of it threw is that module's.
            let failed: Vec<Persistent<Value<'static>>> =
                state.failed.borrow().values().cloned().collect();
            let theirs = failed
                .into_iter()
                .any(|kept| kept.restore(ctx).is_ok_and(|kept| kept == error));
            if !theirs {
                let saved = Persistent::save(ctx, error.clone());
                state.failed.borrow_mut().insert(name.to_owned(), saved);
            }
            Err(ctx.throw(error))
        }
        declared => declared,
    }
}

/// A JSON, CSS or text module (HTML's "create a JSON module script" and
/// its kin): one whose default export is its value, made now: the JSON
/// parsed, its `SyntaxError` thrown if it does not; a style sheet
/// constructed of the CSS, whose URLs resolve against `base`; or the
/// text as it is.
fn synthetic<'js>(
    ctx: &Ctx<'js>,
    entries: &Entries,
    name: &str,
    base: &str,
    text: &str,
    kind: ModuleKind,
) -> rquickjs::Result<Module<'js>> {
    let value: Value<'js> = match kind {
        // A raw NUL is in no JSON text (QuickJS-ng reads C strings).
        ModuleKind::Json if text.contains('\0') => {
            return Err(Exception::throw_syntax(
                ctx,
                "JSON Parse error: Unexpected character",
            ));
        }
        ModuleKind::Json => ctx.json_parse(text)?,
        ModuleKind::Css => {
            let Some(entries) = entries.borrow().clone() else {
                return Err(Exception::throw_type(ctx, "No style sheet can be made."));
            };
            entries
                .restore(ctx)?
                .get::<_, Function<'_>>("cssModule")?
                .call((text, base))?
        }
        ModuleKind::Js | ModuleKind::Text => {
            rquickjs::String::from_str(ctx.clone(), text)?.into_value()
        }
    };
    let module = Module::declare(ctx.clone(), name, "export default import.meta.value;")?;
    module.meta()?.set("value", value)?;
    Ok(module)
}

/// Declares the modules given, in their order, which the engine makes
/// each module's imports' before its own: each finds its imports
/// declared, rather than declaring them nested, so that a deep graph
/// takes no deeper stack than a shallow one. One that fails is kept for
/// the module that imports it, which fails as it did. A JSON, CSS or
/// text module is kept for the loader too: QuickJS-ng knows a module by
/// its name and the attributes it was imported with, which only an
/// import gives it.
pub(super) fn declare_given<'a, H: ScriptingHost>(
    ctx: &Ctx<'_>,
    state: &State,
    host: &Slot<H>,
    budget: &Budget,
    entries: &Entries,
    modules: impl IntoIterator<Item = &'a ModuleSource>,
) {
    state.set_fixed(true);
    for module in modules {
        if module_of_name(&module.url).1 != ModuleKind::Js {
            continue;
        }
        if let Some((base, text)) = state.take(&module.url) {
            let declared = declare_kept(
                ctx,
                state,
                (host, budget, entries),
                &module.url,
                &base,
                &text,
            );
            match declared {
                Ok(_) => {
                    state.mark_declared(&module.url);
                    let options = state.giving.get();
                    state.note(&module.url, Referrer { base, options });
                }
                Err(_) => {
                    let _ = ctx.catch();
                    state.put_back(&module.url, (base, text));
                }
            }
        }
    }
    state.set_fixed(false);
}

/// `specifier` of the script or module whose base URL is `base` (empty
/// for the document's), resolved by the host: a URL, or the `TypeError`
/// it throws. A name too long is refused before the host is asked.
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
        base: base.to_owned(),
        specifier: specifier.to_owned(),
    };
    let (answer, work) = with_host(ctx, host, |host| (host.dom(call), host.take_work()))?;
    budget.charge(work);
    match answer {
        Ok(Answer::String(url)) if !is_bindings(&url) => Ok(url),
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
        let kind = requested(attributes.as_ref())?;
        if let Some(found) = self.state.scan.borrow_mut().as_mut() {
            found.push(ModuleRequest {
                specifier: name.to_owned(),
                kind: kind.ok(),
            });
            return Ok(SCAN_NAME.to_owned());
        }
        if self.state.direct.borrow().as_deref() == Some(name) {
            return Ok(self.state.direct.take().unwrap_or_default());
        }
        // A JSON, CSS or text module the engine gave, imported by its
        // name (the realm's own `importWith`): no page's specifier holds
        // the mark, which a URL escapes.
        if name.contains(MARK)
            && (self.state.store.borrow().contains_key(name) || self.state.declared(name))
        {
            return Ok(name.to_owned());
        }
        // A key other than `type` is a static import's syntax error and
        // an `import()`'s type error, found before its specifier; then a
        // type unknown, or CSS in a worker (HTML's "module type allowed").
        let kind = match kind {
            Ok(kind) if kind != ModuleKind::Css || self.state.css.get() => kind,
            Ok(kind) => return Err(not_a_type(ctx, kind.as_str())),
            Err(Refused::Type(kind)) => return Err(not_a_type(ctx, &kind)),
            Err(Refused::Key(key)) => {
                let message = format!("Invalid attribute key \"{key}\".");
                return Err(match self.state.fixed.get() {
                    true => Exception::throw_syntax(ctx, &message),
                    false => Exception::throw_type(ctx, &message),
                });
            }
        };
        let referrer = self.state.referrer(base);
        self.state.asking.set(referrer.options);
        let url = resolve(ctx, &self.host, &self.budget, &referrer.base, name)?;
        Ok(module_name(&url, kind))
    }
}

/// The `TypeError` of an import of a type the realm does not take.
fn not_a_type(ctx: &Ctx<'_>, kind: &str) -> rquickjs::Error {
    Exception::throw_type(ctx, &format!("\"{kind}\" is not a valid module type."))
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
            let declared = declare_kept(
                ctx,
                &state,
                (&self.host, &self.budget, &self.entries),
                name,
                &base,
                &text,
            );
            return match declared {
                Ok(module) => {
                    state.mark_declared(name);
                    let options = state.giving.get();
                    state.note(name, Referrer { base, options });
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
        let options = state.asking.get();
        let call = Call::ImportModule {
            id,
            url: name.to_owned(),
            options,
        };
        if let Err(error) = with_host(ctx, &self.host, |host| host.dom(call))? {
            return Err(throw(ctx, &error));
        }
        state.import_options.borrow_mut().insert(id, options);
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

/// The promise of `import(name, { with: { type } })` of a JSON, CSS or
/// text module the engine gave, by the bootstrap's `importWith`.
pub(super) fn import_with<'js>(
    ctx: &Ctx<'js>,
    entries: &Entries,
    name: &str,
    kind: ModuleKind,
) -> rquickjs::Result<Promise<'js>> {
    let Some(entries) = entries.borrow().clone() else {
        return rejected(ctx, &format!("Failed to load module script: {name}"));
    };
    entries
        .restore(ctx)?
        .get::<_, Function<'_>>("importWith")?
        .call((name, kind.as_str()))
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
