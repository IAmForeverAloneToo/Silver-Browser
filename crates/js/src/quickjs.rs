//! The QuickJS-ng backend (roadmap D1), through `rquickjs`'s safe
//! interface: a runtime and a context a realm, the host moved into a
//! slot the native functions read for the length of a run, and the
//! limits set on the runtime.

mod bytecode;
pub(crate) mod console;
mod context;
mod dom;
mod intl;
mod intl_dates;
mod intl_names;
mod intl_text;
mod intl_zone;
mod modules;
#[cfg(feature = "js-sampler")]
mod sampler;
// The sampler is a tool, never in a release: a build with it has debug
// assertions (a profiling run of an optimised build sets them, with
// `--config profile.release.debug-assertions=true`).
#[cfg(all(feature = "js-sampler", not(debug_assertions)))]
compile_error!("js-sampler is a tool: build it with debug assertions");
/// The sampler, to the crate's tests.
#[cfg(all(test, feature = "js-sampler"))]
pub(crate) use sampler::Sampler as sampler_for_tests;
mod wasm;

pub use bytecode::warm as warm_bindings;
pub use intl_zone::set_windows_time_zone;

#[cfg(test)]
pub(crate) use wasm::from_base64;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rquickjs::context::EvalOptions;
use rquickjs::function::Rest;
use rquickjs::promise::PromiseHookType;
use rquickjs::{
    CatchResultExt, CaughtError, Coerced, Context, Ctx, Exception, Function, Module, Object,
    Persistent, Promise, Runtime, Value,
};

use crate::{
    Completion, ConsoleKind, ConsoleLevel, ConsoleRecord, Death, ErrorKind, FetchAnswer, Field,
    Limits, MAX_PREVIEW_BYTES, MAX_STACK_FRAMES, ModuleRequest, ModuleScript, Realm, RealmKind,
    Script, ScriptEngine, ScriptError, ScriptingHost, StackFrame, Target, Task,
};

/// QuickJS-ng 0.16, which `rquickjs` builds from its source.
#[derive(Debug, Clone, Copy, Default)]
pub struct QuickJs;

impl ScriptEngine for QuickJs {
    fn realm_of<H: ScriptingHost>(
        &self,
        kind: RealmKind,
        limits: Limits,
        stop: Option<Arc<AtomicBool>>,
    ) -> Result<Box<dyn Realm<H>>, ScriptError> {
        let agent = Agent::new(limits, stop)?;
        Ok(Box::new(QuickJsRealm::new(&kind, &agent, None)?))
    }
}

impl QuickJs {
    /// A new agent (HTML section 8.1.2.1, G90): a runtime its realms
    /// share, with one heap under `limits`, one job queue, one budget
    /// and one interrupt check, which `stop` stops (as `realm_of`'s).
    pub fn agent<H: ScriptingHost>(
        &self,
        limits: Limits,
        stop: Option<Arc<AtomicBool>>,
    ) -> Result<Agent<H>, ScriptError> {
        Agent::new(limits, stop)
    }

    /// A new realm of `kind` in `agent`: a context of its runtime, its
    /// own global, bindings and natives, sharing the agent's heap.
    pub fn realm_in<H: ScriptingHost>(
        &self,
        agent: &Agent<H>,
        kind: RealmKind,
        secure: bool,
    ) -> Result<Box<dyn Realm<H>>, ScriptError> {
        Ok(Box::new(QuickJsRealm::new(&kind, agent, Some(secure))?))
    }
}

/// An agent (HTML section 8.1.2.1; `docs/design/same-origin-frames.md`,
/// G90): the runtime that the realms of one agent cluster share, its
/// heap limit, its budget, its interrupt check and its job queue. A
/// realm made by [`ScriptEngine::realm_of`] is an agent's only one.
pub struct Agent<H: ScriptingHost> {
    core: Rc<AgentCore<H>>,
}

impl<H: ScriptingHost> Clone for Agent<H> {
    fn clone(&self) -> Self {
        Agent {
            core: Rc::clone(&self.core),
        }
    }
}

impl<H: ScriptingHost> Agent<H> {
    /// A handle that does not keep the agent: a table of the agents by
    /// origin holds them so, each going with its last realm.
    pub fn downgrade(&self) -> WeakAgent<H> {
        WeakAgent(Rc::downgrade(&self.core))
    }

    /// Whether `other` is this very agent.
    pub fn same(&self, other: &Agent<H>) -> bool {
        Rc::ptr_eq(&self.core, &other.core)
    }

    /// Whether any realm is made in it.
    pub fn has_realms(&self) -> bool {
        !self.core.realms.borrow().is_empty()
    }

    /// How many realms are made in it.
    pub fn realm_count(&self) -> usize {
        self.core.realms.borrow().len()
    }

    /// Lends the document of frame `frame`, which has no realm, to the
    /// agent for a run of another of its realms (G90 phase 3 (b)): a
    /// realm is made for it, and the document lent to it, if the run
    /// reaches it ([`Agent::take_made`]); else it is taken back as lent
    /// ([`Agent::take_unborn`]).
    pub fn lend_unborn(&self, frame: u32, host: H) {
        self.core.reach.unborn.borrow_mut().insert(frame, host);
    }

    /// The document [`Agent::lend_unborn`] lent for frame `frame`, if no
    /// realm was made for it.
    pub fn take_unborn(&self, frame: u32) -> Option<H> {
        self.core.reach.unborn.borrow_mut().remove(&frame)
    }

    /// The realm made for frame `frame`'s document as a run reached it
    /// (G90 phase 3 (b)), its document lent to it: the frame's engine's
    /// from now on.
    pub fn take_made(&self, frame: u32) -> Option<Box<dyn Realm<H>>> {
        let made = self.core.reach.made.borrow_mut().remove(&frame)?;
        if self.core.heap_unfit.replace(false) {
            self.core.fit_heap(false);
        }
        Some(Box::new(made))
    }

    /// Whether a realm made by a reach waits for frame `frame`'s engine.
    pub fn has_made(&self, frame: u32) -> bool {
        self.core.reach.made.borrow().contains_key(&frame)
    }

    /// Drops the realms made by a reach that no engine took (their frames
    /// went in the run), and gives back the documents still lent as
    /// unborn, which their engines did not take: none should be. A made
    /// realm holds its agent, so one left would keep it.
    pub fn drop_unclaimed(&self) -> (usize, Vec<(u32, H)>) {
        let made = std::mem::take(&mut *self.core.reach.made.borrow_mut());
        let unborn = std::mem::take(&mut *self.core.reach.unborn.borrow_mut());
        (made.len(), unborn.into_iter().collect())
    }

    /// The heap its runtime may hold now ([`AgentCore::fit_heap`]).
    pub fn heap_limit(&self) -> usize {
        self.core.heap_limit.get()
    }

    /// The limit its runtime holds to, as QuickJS has it: what
    /// [`Agent::heap_limit`] says it set, for the tests.
    #[cfg(test)]
    pub(crate) fn runtime_limit(&self) -> i64 {
        self.core.runtime.memory_usage().malloc_limit
    }

    /// Whether more than one document is of it, realm made or not (G90
    /// phase 3 (b)), or more than one realm made: only then may a run of
    /// one reach another's document.
    pub fn shared(&self) -> bool {
        self.core.members.get() > 1 || self.core.realms.borrow().len() > 1
    }

    /// A document's place in the agent, for as long as it is kept: what
    /// [`Agent::shared`] counts, its realm made or not.
    pub fn join(&self) -> Member {
        self.core
            .members
            .set(self.core.members.get().saturating_add(1));
        Member(Rc::clone(&self.core.members))
    }
}

/// A document's place in its agent ([`Agent::join`]): it leaves as this
/// goes.
pub struct Member(Rc<Cell<usize>>);

impl Drop for Member {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

/// An agent not kept ([`Agent::downgrade`]).
pub struct WeakAgent<H: ScriptingHost>(std::rc::Weak<AgentCore<H>>);

impl<H: ScriptingHost> WeakAgent<H> {
    /// The agent, while a realm of it lives.
    pub fn upgrade(&self) -> Option<Agent<H>> {
        self.0.upgrade().map(|core| Agent { core })
    }
}

/// An agent's state, which its realms hold: dropped, the runtime with
/// it, when the last of them goes.
struct AgentCore<H: ScriptingHost> {
    limits: Limits,
    /// Interrupt checks taken in the run under way, whether they ran
    /// out, and the host's work not yet charged: one run at a time in
    /// an agent, whichever realm it is in.
    budget: dom::Budget,
    /// The agent's death: its heap is all its realms'.
    dead: Rc<Cell<Option<Death>>>,
    /// What the runtime's hooks find of each realm, by its context.
    realms: Realms<H>,
    /// The heap its runtime may hold now ([`AgentCore::fit_heap`]).
    heap_limit: Cell<usize>,
    /// A realm that could not be made under a run's held lock (G90 phase
    /// 3 (b)) grew the limit for itself: fitted again before the next run.
    heap_unfit: Cell<bool>,
    /// The documents of the agent, realm made or not ([`Agent::join`]).
    members: Rc<Cell<usize>>,
    /// The frame of the realm whose run is under way (G90): HTML's entry
    /// settings object, which a window's `postMessage` called from
    /// another realm of the agent posts as.
    entry: Entry,
    /// The scripts' sampler, when the tool is built and asked for.
    #[cfg(feature = "js-sampler")]
    sampler: Option<Rc<sampler::Sampler>>,
    /// The realms made as a run reaches their documents, and the documents
    /// lent for it (G90 phase 3 (b)).
    reach: Rc<Reach<H>>,
    runtime: Runtime,
}

/// What a realm made as a run of its agent first reaches its document
/// needs (G90 phase 3 (b), `docs/design/same-origin-frames.md`): the
/// documents of the agent with no realm, lent by their frames; the
/// realms so made, until their engines take them; the frame whose realm
/// is being made; and the agent, to make it in.
pub(crate) struct Reach<H: ScriptingHost> {
    unborn: RefCell<std::collections::HashMap<u32, H>>,
    made: RefCell<std::collections::HashMap<u32, QuickJsRealm<H>>>,
    making: Cell<Option<u32>>,
    core: std::rc::Weak<AgentCore<H>>,
}

thread_local! {
    /// The reaches of a frame whose realm was being made (G90 phase 3
    /// (b)): each answered as another agent's window. Taken by the
    /// engine for the site sweep.
    static REENTERED: Cell<u32> = const { Cell::new(0) };
}

/// The reaches of a frame whose realm was being made since this was last
/// asked, on this thread ([`REENTERED`]).
pub fn take_reentered_reaches() -> u32 {
    REENTERED.with(|count| count.replace(0))
}

/// The global of frame `frame`'s realm, made now as the run of `ctx`'s
/// realm first reaches the frame's document (G90 phase 3 (b)): the
/// document lent to the agent with no realm, or the blank the run's
/// host makes for an `iframe` it inserted. `None` when there is neither
/// (another agent's frame: the cross-origin window answers), or the
/// frame's realm is being made (a reentry, counted). A realm that could
/// not be made leaves nothing: its document goes back, and the reach
/// throws the error.
pub(crate) fn reach_frame<'js, H: ScriptingHost>(
    reach: &Reach<H>,
    ctx: &Ctx<'js>,
    frame: u32,
) -> rquickjs::Result<Option<Object<'js>>> {
    let Some(core) = reach.core.upgrade() else {
        return Ok(None);
    };
    if reach.making.get().is_some() {
        REENTERED.with(|count| count.set(count.get().saturating_add(1)));
        return Ok(None);
    }
    let Some(links) = links_of(&core.realms, ctx) else {
        return Ok(None);
    };
    let unborn = reach.unborn.borrow_mut().remove(&frame);
    let from_unborn = unborn.is_some();
    let mut host = match unborn {
        Some(host) => host,
        None => match with_host(ctx, &links.host, |host| host.blank_frame(frame))? {
            Some(host) => host,
            None => return Ok(None),
        },
    };
    let (secure, guarded) = host.reached_as();
    reach.making.set(Some(frame));
    // Its document lent to it before its bootstrap installs.
    let slot: Slot<H> = Rc::new(RefCell::new(Some(host)));
    let made = QuickJsRealm::make(
        &RealmKind::Window,
        Rc::clone(&core),
        Some(secure),
        Some((ctx, &links.context)),
        Rc::clone(&slot),
    );
    let mut realm = match made {
        Ok(realm) => realm,
        Err(error) => {
            reach.making.set(None);
            let host = slot.borrow_mut().take();
            if from_unborn && let Some(host) = host {
                reach.unborn.borrow_mut().insert(frame, host);
            }
            return Err(Exception::throw_internal(ctx, &error.message));
        }
    };
    realm.set_frame(frame);
    realm.name_frame(Some(frame));
    // Code from strings asks the document's policies from the first call
    // (M33.1), as an eager realm's does once its document is lent.
    let entries = realm.entries.borrow().clone();
    if guarded && let Some(entries) = entries {
        let _ = context::with_locked(ctx, &realm.context, |new| {
            let guard = entries.restore(&new).and_then(|entries| {
                entries
                    .get::<_, Function<'_>>("guardCode")?
                    .call::<_, Value<'_>>(())
            });
            drop(guard.catch(&new));
        });
    }
    reach.making.set(None);
    reach.made.borrow_mut().insert(frame, realm);
    frame_global(&core.realms, ctx, frame)
}

impl<H: ScriptingHost> AgentCore<H> {
    /// Sets the heap its realms may hold together as they are made and go
    /// (G90): its first realm's limit, and [`crate::REALM_HEAP`] more for
    /// each after it, up to [`crate::MAX_AGENT_HEAP`] (or the first's, if
    /// more). As one goes (`gone`), never under what the runtime holds
    /// now, with room for its next collection: a realm gone frees its
    /// objects only then.
    fn fit_heap(&self, gone: bool) {
        let cap = crate::MAX_AGENT_HEAP.max(self.limits.heap);
        let wanted = self.wanted_heap(0);
        let limit = if gone {
            let held = usize::try_from(self.runtime.memory_usage().malloc_size).unwrap_or(0);
            wanted.max(held.saturating_add(crate::REALM_HEAP / 4).min(cap))
        } else {
            wanted
        };
        self.heap_limit.set(limit);
        self.runtime.set_memory_limit(limit);
    }

    /// The heap its realms may hold together, `more` realms to come: its
    /// first realm's limit, and [`crate::REALM_HEAP`] for each after it,
    /// up to [`crate::MAX_AGENT_HEAP`] (or the first's, if more).
    fn wanted_heap(&self, more: usize) -> usize {
        let realms = self.realms.borrow().len().saturating_add(more);
        let cap = crate::MAX_AGENT_HEAP.max(self.limits.heap);
        self.limits
            .heap
            .saturating_add(crate::REALM_HEAP.saturating_mul(realms.saturating_sub(1)))
            .min(cap)
    }
}

impl<H: ScriptingHost> Drop for AgentCore<H> {
    fn drop(&mut self) {
        self.runtime.set_host_promise_rejection_tracker(None);
        self.runtime.set_promise_hook(None);
    }
}

/// What the runtime's hooks (its rejection tracker, its promise hook,
/// its module resolver and loader) find of a realm of the agent, by its
/// context's address.
pub(crate) struct Links<H: ScriptingHost> {
    rejections: Rc<RefCell<Vec<Rejection>>>,
    muted: Rc<Cell<bool>>,
    modules: Rc<modules::State>,
    host: Slot<H>,
    entries: Entries,
    /// Its global object, for the agent's other realms (G90 phase 2).
    global: Global,
    /// The frame its document is of, once the engine names it.
    frame: Rc<Cell<Option<u32>>>,
    /// Its nodes whose objects were collected while its document was not
    /// lent (G90): released to its host when it next is.
    released: Released,
    /// Its context: an anchor of the agent's runtime for a realm made as
    /// its run reaches a document (G90 phase 3 (b)).
    context: Context,
}

/// A realm's nodes let go of while no host of its was there, released
/// to the next ([`Links::released`]).
pub(crate) type Released = Rc<RefCell<Vec<crate::dom::Handle>>>;

/// The frame of the realm of an agent whose run is under way.
pub(crate) type Entry = Rc<Cell<Option<u32>>>;

/// A realm's global object, kept for the other realms of its agent;
/// emptied as the realm goes, before its context.
type Global = Rc<RefCell<Option<Persistent<Object<'static>>>>>;

impl<H: ScriptingHost> Clone for Links<H> {
    fn clone(&self) -> Self {
        Links {
            rejections: Rc::clone(&self.rejections),
            muted: Rc::clone(&self.muted),
            modules: Rc::clone(&self.modules),
            host: Rc::clone(&self.host),
            entries: Rc::clone(&self.entries),
            global: Rc::clone(&self.global),
            frame: Rc::clone(&self.frame),
            released: Rc::clone(&self.released),
            context: self.context.clone(),
        }
    }
}

/// The global object of the realm of `frame` in the agent of `ctx`'s
/// realm, if it has one there (G90 phase 2): a script reaches a frame of
/// its own agent's origin as itself.
pub(crate) fn frame_global<'js, H: ScriptingHost>(
    realms: &Realms<H>,
    ctx: &Ctx<'js>,
    frame: u32,
) -> rquickjs::Result<Option<Object<'js>>> {
    let found = realms
        .borrow()
        .values()
        .find(|links| links.frame.get() == Some(frame))
        .and_then(|links| links.global.borrow().clone());
    found.map(|global| global.restore(ctx)).transpose()
}

/// The bootstrap's entry points of the realm of `frame` in the agent of
/// `ctx`'s realm, if it has one there (G90).
pub(crate) fn frame_entries<'js, H: ScriptingHost>(
    realms: &Realms<H>,
    ctx: &Ctx<'js>,
    frame: u32,
) -> rquickjs::Result<Option<Object<'js>>> {
    let found = realms
        .borrow()
        .values()
        .find(|links| links.frame.get() == Some(frame))
        .and_then(|links| links.entries.borrow().clone());
    found.map(|entries| entries.restore(ctx)).transpose()
}

/// Whether `value` is a node of another realm of the agent of `ctx`'s
/// realm (G90): one this realm's methods refuse until nodes move between
/// documents (phase 5), rather than take as no node.
pub(crate) fn node_of_other_realm<'js, H: ScriptingHost>(
    realms: &Realms<H>,
    ctx: &Ctx<'js>,
    value: &Value<'js>,
) -> rquickjs::Result<bool> {
    let Some(object) = value.as_object() else {
        return Ok(false);
    };
    let own = context_key(ctx);
    let globals: Vec<_> = realms
        .borrow()
        .iter()
        .filter(|(key, _)| **key != own)
        .filter_map(|(_, links)| links.global.borrow().clone())
        .collect();
    for global in globals {
        let global = global.restore(ctx)?;
        let Ok(node) = global.get::<_, Object<'js>>("Node") else {
            continue;
        };
        let Ok(prototype) = node.get::<_, Object<'js>>("prototype") else {
            continue;
        };
        let mut at = object.get_prototype();
        for _ in 0..64 {
            let Some(above) = at else {
                break;
            };
            if above == prototype {
                return Ok(true);
            }
            at = above.get_prototype();
        }
    }
    Ok(false)
}

/// An agent's realms' links, by their contexts' addresses.
type Realms<H> = Rc<RefCell<std::collections::HashMap<usize, Links<H>>>>;

/// The address a context is known by in its agent.
fn context_key(ctx: &Ctx<'_>) -> usize {
    ctx.as_raw().as_ptr() as usize
}

/// The links of the realm whose context `ctx` is: cloned, so that the
/// registry is not borrowed while they are used (a hook may run
/// scripts, which may come back to a hook).
fn links_of<H: ScriptingHost>(realms: &Realms<H>, ctx: &Ctx<'_>) -> Option<Links<H>> {
    realms.borrow().get(&context_key(ctx)).cloned()
}

/// The share of the heap limit that a realm still holds, after a failed
/// run and a collection, that makes it dead: what is held stays held,
/// and the next allocations would fail too.
const DEAD_HEAP: (usize, usize) = (3, 4);

/// A promise rejected with no handler yet, and the error it reports if
/// it still has none at the end of the checkpoint.
struct Rejection {
    promise: Persistent<Value<'static>>,
    error: ScriptError,
}

/// The slot a run moves the host into.
type Slot<H> = Rc<RefCell<Option<H>>>;

/// The bootstrap's entry points (M26), which the realm's tasks and the
/// error reports of scripts run nested call; emptied before the
/// context goes.
type Entries = Rc<RefCell<Option<Persistent<Object<'static>>>>>;

struct QuickJsRealm<H: ScriptingHost> {
    /// Saved values must go before the context and runtime they belong
    /// to; `Drop` empties this first.
    rejections: Rc<RefCell<Vec<Rejection>>>,
    /// The bootstrap's entry points, which [`Realm::enter`] calls (M26).
    entries: Entries,
    /// The natives as made, which a debug build's guard of each calls
    /// (`dom::guard_natives`); emptied with the entries.
    guards: dom::Guards,
    /// The errors of the jobs run between an engine dispatch's
    /// listeners, which the next checkpoint reports.
    deferred: Vec<ScriptError>,
    /// The last run spent its budget, perhaps inside the bootstrap: the
    /// next one first puts the bootstrap's own state back.
    unwind: bool,
    host: Slot<H>,
    /// The agent's interrupt checks taken in this run, and whether they
    /// ran out.
    steps: Rc<Cell<u32>>,
    exhausted: Rc<Cell<bool>>,
    /// The host's work in this run not yet charged as a whole step.
    work: Rc<Cell<u64>>,
    /// Whether the run under way reports muted errors.
    muted: Rc<Cell<bool>>,
    /// What its module loader shares with it (D61), and the budget the
    /// loader's calls of the host charge.
    modules: Rc<modules::State>,
    budget: dom::Budget,
    /// The agent's death, which is its every realm's.
    dead: Rc<Cell<Option<Death>>>,
    /// Its context's address, by which the agent's hooks know it.
    key: usize,
    /// Its global object and its frame, as the agent's other realms
    /// find them.
    global: Global,
    frame: Rc<Cell<Option<u32>>>,
    /// The frame its document is of, shown or not: what its runs are
    /// entered as.
    own_frame: Option<u32>,
    /// Its nodes let go of while its document was not lent.
    released: Released,
    /// Before the agent, which holds the runtime: a context goes first.
    context: Context,
    agent: Rc<AgentCore<H>>,
}

impl<H: ScriptingHost> Agent<H> {
    fn new(limits: Limits, stop: Option<Arc<AtomicBool>>) -> Result<Agent<H>, ScriptError> {
        let runtime = Runtime::new().map_err(made_none)?;
        runtime.set_memory_limit(limits.heap);
        runtime.set_max_stack_size(limits.stack);
        let steps = Rc::new(Cell::new(0u32));
        let exhausted = Rc::new(Cell::new(false));
        let (taken, out, budget) = (steps.clone(), exhausted.clone(), limits.steps);
        #[cfg(feature = "js-sampler")]
        let sampler = sampler::Sampler::from_env();
        #[cfg(feature = "js-sampler")]
        let sampling = sampler.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            // A sample, which says nothing of whether the run goes on.
            #[cfg(feature = "js-sampler")]
            if let Some(sampler) = &sampling {
                sampler.tick();
            }
            let n = taken.get().saturating_add(1);
            taken.set(n);
            if n > budget
                || stop
                    .as_ref()
                    .is_some_and(|stop| stop.load(Ordering::Relaxed))
            {
                out.set(true);
            }
            out.get()
        })));
        let realms: Realms<H> = Rc::default();
        let tracked = realms.clone();
        runtime.set_host_promise_rejection_tracker(Some(Box::new(
            move |ctx: Ctx<'_>, promise: Value<'_>, reason: Value<'_>, handled: bool| {
                let Some(links) = links_of(&tracked, &ctx) else {
                    return;
                };
                if handled {
                    links.rejections.borrow_mut().retain(|r| {
                        r.promise
                            .clone()
                            .restore(&ctx)
                            .map_or(true, |saved| saved != promise)
                    });
                } else {
                    let mut error = thrown(&ctx, reason, "", links.muted.get());
                    error.kind = ErrorKind::Rejection;
                    let promise = Persistent::save(&ctx, promise);
                    links
                        .rejections
                        .borrow_mut()
                        .push(Rejection { promise, error });
                }
            },
        )));
        let budget = dom::Budget {
            steps,
            work: Rc::default(),
            exhausted,
            limit: limits.steps,
        };
        // The promises code a string compiled makes find their active
        // script, each in its own realm's module state.
        let hooked = realms.clone();
        runtime.set_promise_hook(Some(Box::new(
            move |ctx: Ctx<'_>, hook: PromiseHookType, _: Value<'_>, _: Value<'_>| {
                if matches!(hook, PromiseHookType::Init)
                    && modules::made_by_input(&ctx)
                    && let Some(links) = links_of(&hooked, &ctx)
                {
                    modules::promise_made(&ctx, &links.modules);
                }
            },
        )));
        // Module scripts (D61): the resolver and loader, each realm's
        // state, host and entries found by its context.
        runtime.set_loader(
            modules::ModuleResolver {
                realms: realms.clone(),
                budget: budget.clone(),
            },
            modules::ModuleLoader {
                realms: realms.clone(),
                budget: budget.clone(),
            },
        );
        Ok(Agent {
            core: Rc::new_cyclic(|core| AgentCore {
                limits,
                budget,
                dead: Rc::default(),
                realms,
                heap_limit: Cell::new(limits.heap),
                heap_unfit: Cell::new(false),
                members: Rc::default(),
                entry: Entry::default(),
                #[cfg(feature = "js-sampler")]
                sampler,
                reach: Rc::new(Reach {
                    unborn: RefCell::default(),
                    made: RefCell::default(),
                    making: Cell::new(None),
                    core: core.clone(),
                }),
                runtime,
            }),
        })
    }
}

/// The error of a runtime or context QuickJS could not make.
fn made_none(e: rquickjs::Error) -> ScriptError {
    ScriptError {
        kind: ErrorKind::Heap,
        message: format!("InternalError: {e}"),
        url: String::new(),
        line: 0,
        column: 0,
        stack: Vec::new(),
    }
}

impl<H: ScriptingHost> QuickJsRealm<H> {
    fn new(
        kind: &RealmKind,
        agent: &Agent<H>,
        secure: Option<bool>,
    ) -> Result<QuickJsRealm<H>, ScriptError> {
        QuickJsRealm::make(kind, Rc::clone(&agent.core), secure, None, Rc::default())
    }

    /// A realm of `kind` in the agent `core`: made eagerly, or, given
    /// `held` (a context of the agent whose run is under way, and holds
    /// the runtime's lock) and an anchor context of the agent, made as
    /// that run first reaches its document (G90 phase 3 (b)), its
    /// document lent in `host` before its bootstrap installs, as an eager
    /// realm's is before its first run: what the bootstrap asks of it as
    /// it installs, the document answers.
    fn make(
        kind: &RealmKind,
        core: Rc<AgentCore<H>>,
        secure: Option<bool>,
        held: Option<(&Ctx<'_>, &Context)>,
        host: Slot<H>,
    ) -> Result<QuickJsRealm<H>, ScriptError> {
        let limits = Limits {
            secure: secure.unwrap_or(core.limits.secure),
            ..core.limits
        };
        // No realm where a deep script would end the process: the thread
        // must have the scripts' stack and the native work's under it,
        // and say so (G253).
        let needed = limits.stack.saturating_add(crate::NATIVE_STACK);
        let left = crate::remaining_stack();
        if left.is_none_or(|left| left < needed) {
            let thread = std::thread::current();
            return Err(ScriptError {
                kind: ErrorKind::Stack,
                message: format!(
                    "No scripts run on the thread {:?}: it has {} bytes of stack left, and a realm needs {needed}",
                    thread.name().unwrap_or("(unnamed)"),
                    left.map_or_else(
                        || "an unknown number of".to_owned(),
                        |left| left.to_string()
                    ),
                ),
                url: String::new(),
                line: 0,
                column: 0,
                stack: Vec::new(),
            });
        }
        let context = match held {
            None => context::new_context(&core.runtime),
            Some((held, anchor)) => {
                // The heap grows with the realm at once, under the lock the
                // reaching run holds: it may reach a hundred more.
                let limit = core.wanted_heap(1);
                let made = context::new_context_locked(held, anchor, limit)
                    .unwrap_or(Err(rquickjs::Error::Unknown));
                if made.is_ok() {
                    core.heap_limit.set(limit);
                }
                made
            }
        }
        .map_err(made_none)?;
        let held = held.map(|(held, _)| held);
        let key = enter(held, &context, |ctx| context_key(&ctx));
        let rejections: Rc<RefCell<Vec<Rejection>>> = Rc::default();
        let muted = Rc::new(Cell::new(false));
        let budget = core.budget.clone();
        let entries: Entries = Rc::default();
        let guards: dom::Guards = Rc::default();
        // Module scripts (D61): CSS modules a window's only.
        let module_state: Rc<modules::State> = Rc::default();
        module_state
            .css
            .set(matches!(kind, RealmKind::Window | RealmKind::DevTools));
        let global: Global = Rc::default();
        let frame: Rc<Cell<Option<u32>>> = Rc::default();
        let released: Released = Rc::default();
        core.realms.borrow_mut().insert(
            key,
            Links {
                rejections: rejections.clone(),
                muted: muted.clone(),
                modules: module_state.clone(),
                host: host.clone(),
                entries: entries.clone(),
                global: global.clone(),
                frame: frame.clone(),
                released: released.clone(),
                context: context.clone(),
            },
        );
        // Under a run's held lock the limit was fitted as the context was
        // made.
        if held.is_none() {
            core.fit_heap(false);
        }
        let saved = enter(held, &context, |ctx| {
            install(
                &ctx,
                kind,
                limits.secure,
                &host,
                &budget,
                (&entries, &guards),
                (&module_state, &core.realms, &core.entry, &core.reach),
            )
            .map(|entries| Persistent::save(&ctx, entries))
            .catch(&ctx)
            .map_err(|e| e.to_string())
        })
        .map_err(|message| {
            // A realm not made goes without its `Drop`: the natives a
            // debug build guards go before its context, as there, and
            // the agent forgets it.
            guards.borrow_mut().clear();
            core.realms.borrow_mut().remove(&key);
            // Grown for it under a run's lock: fitted again before the
            // next run, the lock not held then.
            if held.is_some() {
                core.heap_unfit.set(true);
            }
            ScriptError {
                kind: ErrorKind::Exception,
                message,
                url: String::new(),
                line: 0,
                column: 0,
                stack: Vec::new(),
            }
        })?;
        *entries.borrow_mut() = Some(saved);
        enter(held, &context, |ctx| {
            *global.borrow_mut() = Some(Persistent::save(&ctx, ctx.globals()));
        });
        Ok(QuickJsRealm {
            rejections,
            entries,
            guards,
            deferred: Vec::new(),
            unwind: false,
            host,
            steps: budget.steps.clone(),
            exhausted: budget.exhausted.clone(),
            work: budget.work.clone(),
            muted,
            modules: module_state,
            budget,
            dead: Rc::clone(&core.dead),
            key,
            global,
            frame,
            own_frame: None,
            released,
            context,
            agent: core,
        })
    }

    /// Moves `host` in and renews the budget; after a run its budget
    /// stopped, the bootstrap's own state is put back first (M26).
    fn begin(&mut self, host: &mut H, muted: bool) {
        // The sampler reads the stack through the context that runs.
        #[cfg(feature = "js-sampler")]
        if let Some(sampler) = &self.agent.sampler {
            sampler.attach(&self.context);
        }
        // A realm the last run's reach could not make shrinks it again.
        if self.agent.heap_unfit.replace(false) {
            self.agent.fit_heap(false);
        }
        // What compiled code of an earlier run left pending ([`clear_stale`]).
        self.context.with(|ctx| clear_stale(&ctx, Stale::Begun));
        release_to(&self.released, host);
        *self.host.borrow_mut() = Some(std::mem::take(host));
        self.agent.entry.set(self.own_frame.or(self.frame.get()));
        self.steps.set(0);
        self.work.set(0);
        self.exhausted.set(false);
        self.muted.set(muted);
        if std::mem::take(&mut self.unwind)
            && let Some(entries) = self.entries.borrow().clone()
        {
            self.context.with(|ctx| {
                let reset = entries.restore(&ctx).and_then(|entries| {
                    entries
                        .get::<_, Function<'_>>("reset")?
                        .call::<_, Value<'_>>(())
                });
                let _ = reset.catch(&ctx);
            });
        }
        // The window's names as the document now is (gap batch A).
        let entries = self.entries.clone();
        let host = self.host.clone();
        self.context.with(|ctx| {
            let _ = dom::sync_named(&ctx, &host, &entries);
        });
    }

    /// Moves the host back out.
    fn end(&mut self, host: &mut H) {
        if let Some(back) = self.host.borrow_mut().take() {
            *host = back;
        }
        self.agent.entry.set(None);
        #[cfg(feature = "js-sampler")]
        if let Some(sampler) = &self.agent.sampler {
            sampler.detach();
        }
        self.muted.set(false);
        if self.exhausted.get() {
            self.unwind = true;
        }
    }

    /// Runs the promise jobs queued while the budget lasts; the errors
    /// of those that threw.
    fn jobs(&mut self) -> Vec<ScriptError> {
        let mut errors = Vec::new();
        while self.agent.runtime.is_job_pending() && !self.exhausted.get() {
            if let Err(job) = self.agent.runtime.execute_pending_job() {
                let error = job.0.with(|ctx| {
                    let thrown = ctx.catch();
                    thrown_or_error(&ctx, thrown)
                });
                errors.push(error);
            }
        }
        errors
    }

    /// Runs one of the bootstrap's stepped tasks (M26): `start` calls the
    /// entry point that makes it, a generator, which is resumed until it
    /// ends, the microtask checkpoint run between its steps, the
    /// listeners or callbacks it calls. One budget covers it all. What it
    /// ends with: for a dispatch, whether the event was canceled.
    fn stepped<F>(
        &mut self,
        entries: &Persistent<Object<'static>>,
        start: F,
    ) -> Result<bool, ScriptError>
    where
        F: for<'js> FnOnce(&Ctx<'js>, Object<'js>) -> rquickjs::Result<Value<'js>>,
    {
        let run = self.context.with(|ctx| {
            let started = entries
                .clone()
                .restore(&ctx)
                .and_then(|entries| start(&ctx, entries));
            started
                .catch(&ctx)
                .map(|run| Persistent::save(&ctx, run))
                .map_err(|caught| caught_error(&ctx, caught, "", false))
        })?;
        loop {
            let step = self.context.with(|ctx| {
                let resumed = entries.clone().restore(&ctx).and_then(|entries| {
                    let run = run.clone().restore(&ctx)?;
                    entries
                        .get::<_, Function<'_>>("resume")?
                        .call::<_, Value<'_>>((run,))
                });
                resumed
                    .catch(&ctx)
                    .map(|value| value.as_bool())
                    .map_err(|caught| caught_error(&ctx, caught, "", false))
            })?;
            if let Some(canceled) = step {
                return Ok(canceled);
            }
            let errors = self.jobs();
            self.deferred.extend(errors);
        }
    }

    /// Whether the heap, collected, is still nearly full: then the realm
    /// is dead. Full of the agent's limit now, which grows with its realms
    /// (G90): of the first realm's, a page of many frames of one origin
    /// would die of any error thrown.
    fn check_heap(&mut self) {
        self.agent.runtime.run_gc();
        let (num, den) = DEAD_HEAP;
        if self.heap() > self.agent.heap_limit.get() / den * num {
            self.dead.set(Some(Death::Heap));
        }
    }

    /// `error` as a run's failure: the budget's when it ran out, the
    /// heap's when the realm is now dead.
    fn failure(&mut self, mut error: ScriptError) -> ScriptError {
        if self.exhausted.get() {
            error.kind = ErrorKind::Budget;
            error.message = "Script ran too long and was stopped".to_owned();
            return error;
        }
        self.check_heap();
        if self.dead.get().is_some() {
            error.kind = ErrorKind::Heap;
        }
        error
    }
}

impl<H: ScriptingHost> Drop for QuickJsRealm<H> {
    fn drop(&mut self) {
        // The agent forgets it; its hooks are the agent's, which goes
        // with its last realm.
        self.agent.realms.borrow_mut().remove(&self.key);
        self.agent.fit_heap(true);
        *self.global.borrow_mut() = None;
        self.rejections.borrow_mut().clear();
        self.modules.clear();
        *self.entries.borrow_mut() = None;
        self.guards.borrow_mut().clear();
    }
}

impl<H: ScriptingHost> Realm<H> for QuickJsRealm<H> {
    fn run(&mut self, script: &Script<'_>, host: &mut H) -> Result<Completion, ScriptError> {
        if let Some(death) = self.dead.get() {
            return Err(dead(death));
        }
        self.begin(host, script.muted);
        // Its `import()`s' referrer (HTML's script record).
        let base = if script.base.is_empty() {
            script.url
        } else {
            script.base
        };
        let referrer = modules::Referrer {
            base: base.to_owned(),
            options: script.options,
        };
        self.modules.note(script.url, referrer);
        self.modules.ran(script.url);
        let (entries, exhausted) = (self.entries.clone(), self.exhausted.clone());
        let result =
            self.context.with(
                |ctx| match evaluate(&ctx, script.source, script.url, script.line) {
                    Ok(value) => Ok(completion(&value)),
                    Err(caught) => Err(report(
                        &ctx,
                        &entries,
                        &exhausted,
                        caught,
                        script.url,
                        script.muted,
                    )),
                },
            );
        self.end(host);
        // A run that spent its budget is stopped, though the script caught
        // a native's refusal and ended before the next interrupt check.
        let result = match result {
            Ok(_) if self.exhausted.get() => Err(ScriptError {
                kind: ErrorKind::Budget,
                message: String::new(),
                url: script.url.to_owned(),
                line: 0,
                column: 0,
                stack: Vec::new(),
            }),
            other => other,
        };
        result.map_err(|error| self.failure(error))
    }

    fn module_requests(
        &mut self,
        url: &str,
        source: &str,
    ) -> Result<Vec<ModuleRequest>, ScriptError> {
        if let Some(death) = self.dead.get() {
            return Err(dead(death));
        }
        // A context of the realm's runtime with only the base objects and
        // the compiler, which runs nothing, and goes with what it declared;
        // the regular expressions' compiler with it, without which a module
        // that writes one does not compile, and is found to import nothing.
        let scratch = Context::custom::<(
            rquickjs::context::intrinsic::Eval,
            rquickjs::context::intrinsic::RegExpCompiler,
        )>(&self.agent.runtime);
        let scratch = scratch.map_err(|e| ScriptError {
            kind: ErrorKind::Heap,
            message: format!("InternalError: {e}"),
            url: String::new(),
            line: 0,
            column: 0,
            stack: Vec::new(),
        })?;
        // The scratch context is this realm's for the agent's resolver
        // while it declares, which collects the scan's requests here.
        let scratch_key = scratch.with(|ctx| context_key(&ctx));
        let links = self.agent.realms.borrow().get(&self.key).cloned();
        if let Some(links) = links {
            self.agent.realms.borrow_mut().insert(scratch_key, links);
        }
        self.modules.start_scan();
        let declared = scratch.with(|ctx| {
            Module::declare(ctx.clone(), url, source)
                .map(drop)
                .catch(&ctx)
                .map_err(|caught| caught_error(&ctx, caught, url, false))
        });
        let found = self.modules.end_scan();
        self.agent.realms.borrow_mut().remove(&scratch_key);
        drop(scratch);
        declared.map(|()| found)
    }

    fn run_module(&mut self, script: &ModuleScript<'_>, host: &mut H) -> Result<(), ScriptError> {
        if let Some(death) = self.dead.get() {
            return Err(dead(death));
        }
        self.begin(host, false);
        self.modules.give(script.modules);
        self.modules.set_giving(script.options);
        let (entries, exhausted) = (self.entries.clone(), self.exhausted.clone());
        let (state, slot, budget) = (self.modules.clone(), self.host.clone(), self.budget.clone());
        let url = script.url;
        let result = self.context.with(|ctx| {
            let given = script.modules.iter().filter(|module| module.url != url);
            modules::declare_given(&ctx, &state, &slot, &budget, &entries, given);
            let evaluation = match script.inline {
                Some((text, line)) => {
                    let name = state.inline_name(url);
                    let referrer = modules::Referrer {
                        base: url.to_owned(),
                        options: script.options,
                    };
                    state.note(&name, referrer);
                    state.ran(&name);
                    let skip = line.saturating_sub(1) as usize;
                    let mut padded = String::with_capacity(skip + text.len());
                    padded.extend(std::iter::repeat_n('\n', skip));
                    padded.push_str(text);
                    state.set_fixed(true);
                    let declared =
                        modules::declare(&ctx, &slot, &budget, &entries, &name, url, &padded);
                    state.set_fixed(false);
                    declared.and_then(|module| module.eval()).map(|(_, p)| p)
                }
                None if !state.declared(url) => match state.take(url) {
                    Some((base, text)) => {
                        state.ran(url);
                        state.set_fixed(true);
                        let declared = modules::declare_kept(
                            &ctx,
                            &state,
                            (&slot, &budget, &entries),
                            url,
                            &base,
                            &text,
                        );
                        state.set_fixed(false);
                        match declared {
                            Ok(module) => {
                                state.mark_declared(url);
                                let referrer = modules::Referrer {
                                    base,
                                    options: script.options,
                                };
                                state.note(url, referrer);
                                module.eval().map(|(_, p)| p)
                            }
                            Err(error) => {
                                state.put_back(url, (base, text));
                                Err(error)
                            }
                        }
                    }
                    None => Err(Exception::throw_type(
                        &ctx,
                        &format!("Failed to load module script: {url}"),
                    )),
                },
                // Declared already, by another's graph: its evaluation,
                // which it has had or has now, once.
                None => {
                    state.ran(url);
                    state.set_direct(Some(url.to_owned()));
                    let promise = Module::import(&ctx, url);
                    state.set_direct(None);
                    promise
                }
            };
            match evaluation.catch(&ctx) {
                Ok(promise) => evaluated(&ctx, &entries, promise)
                    .catch(&ctx)
                    .map_err(|caught| caught_error(&ctx, caught, url, false)),
                Err(caught) => Err(report(&ctx, &entries, &exhausted, caught, url, false)),
            }
        });
        self.end(host);
        let result = match result {
            Ok(()) if self.exhausted.get() => Err(ScriptError {
                kind: ErrorKind::Budget,
                message: String::new(),
                url: url.to_owned(),
                line: 0,
                column: 0,
                stack: Vec::new(),
            }),
            other => other,
        };
        result.map_err(|error| self.failure(error))
    }

    fn enter(&mut self, task: &Task, host: &mut H) -> Result<bool, ScriptError> {
        if let Some(death) = self.dead.get() {
            return Err(dead(death));
        }
        let Some(entries) = self.entries.borrow().clone() else {
            return Ok(false);
        };
        self.begin(host, false);
        let result = match task {
            Task::Timer(id) => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("timer")?
                    .call::<_, Value<'_>>((*id,))
            }),
            Task::Dispatch { target, event } => self.stepped(&entries, |ctx, entries| {
                let at = match target {
                    Target::Node(handle) => Value::new_float(ctx.clone(), handle.0 as f64),
                    Target::Window => Value::new_null(ctx.clone()),
                };
                let init = init_object(ctx, &event.fields)?;
                entries
                    .get::<_, Function<'_>>("dispatch")?
                    .call::<_, Value<'_>>((at, event.interface, event.kind.as_str(), init))
            }),
            Task::AnimationFrames(now) => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("frames")?
                    .call::<_, Value<'_>>((*now,))
            }),
            Task::Message {
                target,
                data,
                origin,
            } => self.stepped(&entries, |_, entries| {
                let (kind, n) = match target {
                    crate::MessageTarget::Global => (0, 0),
                    crate::MessageTarget::Worker(n) => (1, *n),
                    crate::MessageTarget::Port(n) => (2, *n),
                    crate::MessageTarget::Frame(n) => (3, *n),
                };
                entries
                    .get::<_, Function<'_>>("message")?
                    .call::<_, Value<'_>>((kind, n, data.as_str(), origin.as_str()))
            }),
            Task::WorkerError { worker, error } => self.stepped(&entries, |ctx, entries| {
                let (message, url, line, column) = match error {
                    Some(error) => (
                        rquickjs::String::from_str(ctx.clone(), &error.message)?.into_value(),
                        error.url.as_str(),
                        error.line,
                        error.column,
                    ),
                    None => (Value::new_null(ctx.clone()), "", 0, 0),
                };
                entries
                    .get::<_, Function<'_>>("workerError")?
                    .call::<_, Value<'_>>((*worker, message, url, line, column))
            }),
            Task::WorkerEnded(worker) => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("workerEnded")?
                    .call::<_, Value<'_>>((*worker,))
            }),
            Task::FrameList(names) => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("frameList")?
                    .call::<_, Value<'_>>((names.clone(),))
            }),
            Task::Detach => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("detach")?
                    .call::<_, Value<'_>>(())
            }),
            Task::GuardCode => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("guardCode")?
                    .call::<_, Value<'_>>(())
            }),
            Task::Evaluate {
                expression,
                selected,
            } => self.stepped(&entries, |ctx, entries| {
                let selected = match selected {
                    Some(handle) => Value::new_float(ctx.clone(), handle.0 as f64),
                    None => Value::new_null(ctx.clone()),
                };
                entries
                    .get::<_, Function<'_>>("evaluate")?
                    .call::<_, Value<'_>>((expression.as_str(), selected))
            }),
            Task::Idb => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("idb")?
                    .call::<_, Value<'_>>(())
            }),
            Task::DevTools(message) => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("devtools")?
                    .call::<_, Value<'_>>((message.as_str(),))
            }),
            Task::Media(message) => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("media")?
                    .call::<_, Value<'_>>((message.as_str(),))
            }),
            Task::Animations(message) => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("animations")?
                    .call::<_, Value<'_>>((message.as_str(),))
            }),
            Task::Broadcast {
                name,
                data,
                sender,
                origin,
            } => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("broadcast")?
                    .call::<_, Value<'_>>((
                        name.as_str(),
                        data.as_str(),
                        *sender as f64,
                        origin.as_str(),
                    ))
            }),
            Task::Observations(text) => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("observations")?
                    .call::<_, Value<'_>>((text.as_str(),))
            }),
            Task::Mutations => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("mutations")?
                    .call::<_, Value<'_>>(())
            }),
            Task::Construct { name, is, prefix } => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("construct")?
                    .call::<_, Value<'_>>((name.as_str(), is.as_deref(), prefix.as_deref()))
            }),
            Task::Reactions => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("reactions")?
                    .call::<_, Value<'_>>(())
            }),
            Task::PopState(state) => self.stepped(&entries, |ctx, entries| {
                let state = match state {
                    Some(text) => rquickjs::String::from_str(ctx.clone(), text)?.into_value(),
                    None => Value::new_null(ctx.clone()),
                };
                entries
                    .get::<_, Function<'_>>("popState")?
                    .call::<_, Value<'_>>((state,))
            }),
            Task::MediaChanged(ids) => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("mediaChanged")?
                    .call::<_, Value<'_>>((ids.clone(),))
            }),
            Task::CloseRequest { can_prevent } => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("closeRequest")?
                    .call::<_, Value<'_>>((*can_prevent,))
            }),
            Task::LightDismiss { target, up, x, y } => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("lightDismiss")?
                    .call::<_, Value<'_>>((target.0 as f64, *up, *x, *y))
            }),
            Task::Interest {
                source,
                target,
                gained,
            } => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("interest")?
                    .call::<_, Value<'_>>((source.0 as f64, target.0 as f64, *gained))
            }),
            Task::Gpu(message, bytes) => self.stepped(&entries, |ctx, entries| {
                let bytes = if bytes.is_empty() {
                    Value::new_null(ctx.clone())
                } else {
                    rquickjs::ArrayBuffer::new(ctx.clone(), bytes.clone())?.into_value()
                };
                entries
                    .get::<_, Function<'_>>("gpu")?
                    .call::<_, Value<'_>>((message.as_str(), bytes))
            }),
            Task::Audio(message, bytes) => self.stepped(&entries, |ctx, entries| {
                let samples = if bytes.is_empty() {
                    Value::new_null(ctx.clone())
                } else {
                    rquickjs::ArrayBuffer::new(ctx.clone(), bytes.clone())?.into_value()
                };
                entries
                    .get::<_, Function<'_>>("audio")?
                    .call::<_, Value<'_>>((message.as_str(), samples))
            }),
            Task::ServiceWorker(message) => self.stepped(&entries, |_, entries| {
                entries
                    .get::<_, Function<'_>>("serviceWorker")?
                    .call::<_, Value<'_>>((message.as_str(),))
            }),
            Task::Imported {
                id,
                url,
                modules,
                error,
            } => {
                self.modules.give(modules);
                self.modules.set_giving(self.modules.imported(*id));
                let (state, slots) = (self.modules.clone(), self.entries.clone());
                let (slot, budget) = (self.host.clone(), self.budget.clone());
                self.context.with(|ctx| {
                    modules::declare_given(&ctx, &state, &slot, &budget, &slots, modules);
                    let kind = crate::module_of_name(url).1;
                    let promise = match error {
                        Some(message) => modules::rejected(&ctx, message),
                        // A JSON, CSS or text module is imported with its
                        // type, as QuickJS-ng keys it.
                        None if kind != crate::ModuleKind::Js => {
                            modules::import_with(&ctx, &slots, url, kind)
                        }
                        None => {
                            state.set_direct(Some(url.clone()));
                            let promise = Module::import(&ctx, url.as_str());
                            state.set_direct(None);
                            promise
                        }
                    };
                    promise
                        .and_then(|promise| modules::settle(&ctx, &state, &slots, *id, promise))
                        .catch(&ctx)
                        .map(|()| false)
                        .map_err(|caught| caught_error(&ctx, caught, "", false))
                })
            }
            Task::Fetched { id, answer } => self.context.with(|ctx| {
                let called = entries.clone().restore(&ctx).and_then(|entries| {
                    let answer = match answer {
                        None => Value::new_null(ctx.clone()),
                        Some(answer) => answer_object(&ctx, answer)?.into_value(),
                    };
                    entries
                        .get::<_, Function<'_>>("fetched")?
                        .call::<_, Value<'_>>((*id, answer))
                });
                called
                    .catch(&ctx)
                    .map(|_| false)
                    .map_err(|caught| caught_error(&ctx, caught, "", false))
            }),
        };
        self.end(host);
        let result = match result {
            Ok(_) if self.exhausted.get() => Err(ScriptError {
                kind: ErrorKind::Budget,
                message: String::new(),
                url: String::new(),
                line: 0,
                column: 0,
                stack: Vec::new(),
            }),
            other => other,
        };
        result.map_err(|error| self.failure(error))
    }

    #[cfg(feature = "js-sampler")]
    fn samples(&self) -> Vec<(String, u64)> {
        self.agent
            .sampler
            .as_ref()
            .map_or_else(Vec::new, |sampler| sampler.top())
    }

    fn checkpoint(&mut self, host: &mut H) -> Vec<ScriptError> {
        if let Some(death) = self.dead.get() {
            return vec![dead(death)];
        }
        self.begin(host, false);
        let mut errors = std::mem::take(&mut self.deferred);
        errors.extend(self.jobs());
        if !self.agent.runtime.is_job_pending() {
            self.modules.jobs_ran();
        }
        self.end(host);
        let errors: Vec<ScriptError> = errors.into_iter().map(|e| self.failure(e)).collect();
        let mut reported: Vec<ScriptError> = errors;
        reported.extend(
            std::mem::take(&mut *self.rejections.borrow_mut())
                .into_iter()
                .map(|r| r.error),
        );
        reported
    }

    fn heap(&self) -> usize {
        usize::try_from(self.agent.runtime.memory_usage().malloc_size).unwrap_or(usize::MAX)
    }

    fn collect(&mut self) {
        self.agent.runtime.run_gc();
    }

    fn sweep(&mut self, host: &mut H, collect: bool) {
        let entries = match self.dead.get() {
            None => self.entries.borrow().clone(),
            Some(_) => None,
        };
        let Some(entries) = entries else {
            if collect {
                self.agent.runtime.run_gc();
            }
            return;
        };
        self.begin(host, false);
        // The bootstrap's half of the sweep: whether the garbage is to be
        // collected now, with the holds it let go of.
        let half = |realm: &Self, name: &str| {
            realm.context.with(|ctx| {
                let called = entries.clone().restore(&ctx).and_then(|entries| {
                    entries
                        .get::<_, Function<'_>>(name)?
                        .call::<_, bool>((collect,))
                });
                called.catch(&ctx).unwrap_or(false)
            })
        };
        if half(self, "sweepPrepare") || collect {
            // Out of the bootstrap's frames, what only the holds it let go
            // of kept is collected, and the trees nothing reaches.
            self.agent.runtime.run_gc();
            half(self, "sweepFinish");
            // The finalization callbacks release what was collected.
            let errors = self.jobs();
            self.deferred.extend(errors);
        }
        self.end(host);
    }

    fn dead(&self) -> Option<Death> {
        self.dead.get()
    }

    fn lend(&mut self, mut host: H) {
        release_to(&self.released, &mut host);
        *self.host.borrow_mut() = Some(host);
    }

    fn reclaim(&mut self) -> Option<H> {
        self.host.borrow_mut().take()
    }

    fn is_lent(&self) -> bool {
        self.host.borrow().is_some()
    }

    fn set_frame(&mut self, frame: u32) {
        self.own_frame = Some(frame);
    }

    fn name_frame(&mut self, frame: Option<u32>) {
        // One realm of the agent is a frame's at a time: the realm of the
        // document it showed before (its initial `about:blank`, while the
        // next one parses) is no longer reached as the frame's window.
        if let Some(frame) = frame {
            for links in self.agent.realms.borrow().values() {
                if links.frame.get() == Some(frame) {
                    links.frame.set(None);
                }
            }
        }
        self.frame.set(frame);
    }
}

/// Runs `source` as a classic script: global, not strict, its errors
/// naming `url`, its first line counted as `line` of its document (line
/// feeds go before the text, so that QuickJS-ng counts them).
fn evaluate<'js>(
    ctx: &Ctx<'js>,
    source: &str,
    url: &str,
    line: u32,
) -> Result<Value<'js>, CaughtError<'js>> {
    let skip = line.saturating_sub(1) as usize;
    let mut text = String::with_capacity(skip + source.len());
    text.extend(std::iter::repeat_n('\n', skip));
    text.push_str(source);
    let mut options = EvalOptions::default();
    options.global = true;
    options.strict = false;
    options.filename = Some(url.replace('\0', "%00"));
    let result = ctx
        .eval_with_options::<Value<'_>, _>(text, options)
        .catch(ctx);
    clear_stale(ctx, Stale::Compiled);
    result
}

/// Clears the exception QuickJS-ng's parser can leave pending though the
/// code compiled: its look past a parenthesized expression, to tell an
/// arrow function's parameters, reads a regular expression after `)` (as
/// in `for (...) /9a/.test(x)`) as a division, fails on what follows,
/// and gives up without clearing the error (its own "XXX: should clear
/// the exception generated by next_token()", unchanged upstream
/// 2026-11-16). Left, it is thrown by whatever runs next, and fails the
/// next module a graph links: Google Maps' scripts, 2026-11-18. None is
/// pending where this is called (after code compiled, as an operation
/// begins) but such a one.
///
/// A debug build says each it clears, and where, and panics for one that
/// is not a SyntaxError: what else is pending there is a bug of ours (the
/// lead's condition, 2026-11-18). A SyntaxError is let go wherever it is
/// found, as code compiled within a script (its own text, `eval`, `new
/// Function`) leaves one with no place after it to clear it but these.
pub(crate) fn clear_stale(ctx: &Ctx<'_>, site: Stale) {
    if !ctx.has_exception() {
        return;
    }
    let pending = ctx.catch();
    #[cfg(debug_assertions)]
    {
        // A true SyntaxError, of the realm's own class, which the page
        // cannot replace: one the realm makes now has its prototype. A
        // DOMException named so (an invalid selector's) is not one.
        let intrinsic = {
            let _ = Exception::throw_syntax(ctx, "");
            ctx.catch().as_object().and_then(Object::get_prototype)
        };
        let syntax = pending
            .as_object()
            .and_then(Object::get_prototype)
            .is_some_and(|prototype| Some(prototype) == intrinsic);
        let message = pending
            .as_object()
            .and_then(|error| error.get::<_, String>("message").ok())
            .unwrap_or_default();
        eprintln!("clear_stale ({site:?}): {message}");
        assert!(
            syntax,
            "an exception not QuickJS-ng's parser's was pending ({site:?}): {pending:?}"
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = (pending, site);
}

/// Where [`clear_stale`] found what it cleared.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Stale {
    /// After code compiled: a classic script, a module.
    Compiled,
    /// As a run of the realm began.
    Begun,
    /// Before a native's call (a debug build's guard).
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    Native,
}

/// The error a dead realm reports for what it was asked to run.
fn dead(death: Death) -> ScriptError {
    let message = match death {
        Death::Heap => "Scripts stopped: out of memory",
    };
    ScriptError {
        kind: ErrorKind::Dead,
        message: message.to_owned(),
        url: String::new(),
        line: 0,
        column: 0,
        stack: Vec::new(),
    }
}

/// What a script's completion value is kept as.
fn completion(value: &Value<'_>) -> Completion {
    if value.is_undefined() {
        Completion::Undefined
    } else if let Some(text) = value.as_string() {
        text.to_string()
            .map(Completion::String)
            .unwrap_or(Completion::Other)
    } else {
        Completion::Other
    }
}

/// The error a caught failure reports.
fn caught_error<'js>(
    ctx: &Ctx<'js>,
    caught: CaughtError<'js>,
    url: &str,
    muted: bool,
) -> ScriptError {
    match caught {
        CaughtError::Exception(exception) => {
            let error = exception_error(&exception, url);
            mute(error, muted)
        }
        CaughtError::Value(value) => thrown(ctx, value, url, muted),
        CaughtError::Error(error) => mute(
            ScriptError {
                kind: ErrorKind::Exception,
                message: format!("InternalError: {error}"),
                url: url.to_owned(),
                line: 0,
                column: 0,
                stack: Vec::new(),
            },
            muted,
        ),
    }
}

/// Reports a script's uncaught exception as HTML section 8.1.4.6 does
/// (M26): an `error` event at the window, which the bootstrap fires,
/// and, unless a listener canceled it, the error the caller gives the
/// console, which is [`ErrorKind::Handled`] otherwise. A run its budget
/// stopped fires nothing.
fn report<'js>(
    ctx: &Ctx<'js>,
    entries: &Entries,
    exhausted: &Cell<bool>,
    caught: CaughtError<'js>,
    url: &str,
    muted: bool,
) -> ScriptError {
    let value = match &caught {
        CaughtError::Exception(exception) => exception.clone().into_value(),
        CaughtError::Value(value) => value.clone(),
        CaughtError::Error(_) => Value::new_undefined(ctx.clone()),
    };
    let mut error = caught_error(ctx, caught, url, muted);
    if exhausted.get() || error.kind != ErrorKind::Exception {
        return error;
    }
    let Some(entries) = entries.borrow().clone() else {
        return error;
    };
    let canceled = entries.restore(ctx).and_then(|entries| {
        entries.get::<_, Function<'_>>("report")?.call::<_, bool>((
            value,
            error.message.as_str(),
            error.url.as_str(),
            error.line,
            error.column,
            muted,
        ))
    });
    if let Ok(true) = canceled.catch(ctx) {
        error.kind = ErrorKind::Handled;
    }
    error
}

/// A module script's evaluation (D61, HTML's "run a module script"):
/// its rejection, at once or after a top-level await, is reported as an
/// uncaught exception, by the bootstrap's own `then`.
fn evaluated<'js>(
    ctx: &Ctx<'js>,
    entries: &Entries,
    promise: Promise<'js>,
) -> rquickjs::Result<()> {
    let Some(entries) = entries.borrow().clone() else {
        return Ok(());
    };
    entries
        .restore(ctx)?
        .get::<_, Function<'_>>("moduleEvaluated")?
        .call::<_, Value<'_>>((promise,))?;
    Ok(())
}

/// An event's dictionary as the bootstrap's constructors read it: an
/// object with no prototype, so that no getter a page put on
/// `Object.prototype` answers for a member it leaves out.
fn init_object<'js>(
    ctx: &Ctx<'js>,
    fields: &[(&'static str, Field)],
) -> rquickjs::Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set_prototype(None)?;
    for (name, field) in fields {
        let value = match field {
            Field::Bool(value) => Value::new_bool(ctx.clone(), *value),
            Field::Number(value) => Value::new_number(ctx.clone(), *value),
            Field::String(text) => rquickjs::String::from_str(ctx.clone(), text)?.into_value(),
            Field::Node(Some(handle)) => Value::new_float(ctx.clone(), handle.0 as f64),
            Field::Node(None) | Field::Null => Value::new_null(ctx.clone()),
            Field::Window => ctx.globals().into_value(),
            // The bootstrap's `dispatch` makes it the window's area.
            Field::Storage(local) => Value::new_bool(ctx.clone(), *local),
        };
        object.set(*name, value)?;
    }
    Ok(object)
}

/// A fetch's answer as the bootstrap's `fetched` takes it (M27.1): its
/// type, URL list, status, status text, fields as names and values one
/// after the other, and body, a buffer or `null`.
pub(crate) fn answer_object<'js>(
    ctx: &Ctx<'js>,
    answer: &FetchAnswer,
) -> rquickjs::Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set("type", answer.response_type)?;
    let urls = rquickjs::Array::new(ctx.clone())?;
    for (i, url) in answer.url_list.iter().enumerate() {
        urls.set(i, url.as_str())?;
    }
    object.set("urlList", urls)?;
    object.set("status", answer.status)?;
    object.set("statusText", answer.status_text.as_str())?;
    let headers = rquickjs::Array::new(ctx.clone())?;
    for (i, (name, value)) in answer.headers.iter().enumerate() {
        headers.set(2 * i, name.as_str())?;
        headers.set(2 * i + 1, value.as_str())?;
    }
    object.set("headers", headers)?;
    match &answer.body {
        Some(body) => object.set("body", rquickjs::ArrayBuffer::new_copy(ctx.clone(), body)?)?,
        None => object.set("body", Value::new_null(ctx.clone()))?,
    }
    Ok(object)
}

/// The error a thrown value, or a failure with none, reports.
fn thrown_or_error<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> ScriptError {
    match as_exception(&value) {
        Some(exception) => exception_error(&exception, ""),
        None => thrown(ctx, value, "", false),
    }
}

/// The error a thrown value reports: an `Error`'s name, message and
/// position, else the value as a string.
fn thrown<'js>(ctx: &Ctx<'js>, value: Value<'js>, url: &str, muted: bool) -> ScriptError {
    if let Some(exception) = as_exception(&value) {
        return mute(exception_error(&exception, url), muted);
    }
    let message = console::describe(ctx, &value);
    mute(
        ScriptError {
            kind: ErrorKind::Exception,
            message,
            url: url.to_owned(),
            line: 0,
            column: 0,
            stack: Vec::new(),
        },
        muted,
    )
}

/// `value` as an `Error` object, if it is one.
fn as_exception<'js>(value: &Value<'js>) -> Option<Exception<'js>> {
    value.as_object().cloned().and_then(Exception::from_object)
}

/// An `Error` object's report: `name: message`, at the first frame of
/// its stack.
fn exception_error(exception: &Exception<'_>, url: &str) -> ScriptError {
    let name = exception
        .as_object()
        .get::<_, Coerced<String>>("name")
        .map(|name| name.0)
        .unwrap_or_else(|_| "Error".to_owned());
    let message = match exception.message() {
        Some(text) if !text.is_empty() => format!("{name}: {text}"),
        _ => name,
    };
    let stack = exception.stack().unwrap_or_default();
    let (at, line, column) = first_frame(&stack).unwrap_or_else(|| (url.to_owned(), 0, 0));
    ScriptError {
        kind: ErrorKind::Exception,
        message,
        url: at,
        line,
        column,
        stack: parse_stack(&stack),
    }
}

/// The URL, line and column of a stack's first frame:
/// `    at f (url:line:column)` or `    at url:line:column`. The URL may
/// hold spaces and parentheses, as a `data:` URL's may: the place is
/// what follows the name's ` (`.
fn first_frame(stack: &str) -> Option<(String, u32, u32)> {
    let frame = stack.lines().next()?.trim().strip_prefix("at ")?;
    let place = match frame.strip_suffix(')') {
        Some(called) => &called[called.find(" (")? + 2..],
        None => frame,
    };
    let mut parts = place.rsplitn(3, ':');
    let column = parts.next()?.parse().ok()?;
    let line = parts.next()?.parse().ok()?;
    let url = parts.next()?;
    Some((modules::page_url(url).to_owned(), line, column))
}

/// The frames of a stack as QuickJS writes it, innermost first, at most
/// [`MAX_STACK_FRAMES`] (M36): each `at f (url:line:column)` or `at
/// url:line:column`; the bindings' own frames, and those of native code,
/// left out.
pub(crate) fn parse_stack(stack: &str) -> Vec<StackFrame> {
    let mut frames = Vec::new();
    for line in stack.lines() {
        if frames.len() == MAX_STACK_FRAMES {
            break;
        }
        let Some(frame) = line.trim().strip_prefix("at ") else {
            continue;
        };
        let (function, place) = match frame.strip_suffix(')') {
            Some(called) => match called.find(" (") {
                Some(at) => (&called[..at], &called[at + 2..]),
                None => continue,
            },
            None => ("", frame),
        };
        let mut parts = place.rsplitn(3, ':');
        let (Some(column), Some(line), Some(url)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let (Ok(line), Ok(column)) = (line.parse(), column.parse()) else {
            continue;
        };
        // An `import()`'s stub's (D61) too.
        if dom::is_bindings(url) || url.starts_with(modules::MARK) {
            continue;
        }
        frames.push(StackFrame {
            function: function.to_owned(),
            url: modules::page_url(url).to_owned(),
            line,
            column,
        });
    }
    frames
}

/// `error`, muted if `muted` (HTML section 8.1.4.6): its details
/// hidden.
fn mute(error: ScriptError, muted: bool) -> ScriptError {
    if !muted {
        return error;
    }
    ScriptError {
        kind: error.kind,
        message: "Script error.".to_owned(),
        url: String::new(),
        line: 0,
        column: 0,
        stack: Vec::new(),
    }
}

/// Fills in a new realm's global object: `window` and `self`,
/// `console`, and the DOM's interfaces and `document` (M25); and takes
/// away what QuickJS-ng adds that a page of a browser that is not
/// cross-origin isolated has not: an unrounded `performance` clock and
/// `SharedArrayBuffer`.
fn install<'js, H: ScriptingHost>(
    ctx: &Ctx<'js>,
    kind: &RealmKind,
    secure: bool,
    host: &Slot<H>,
    budget: &dom::Budget,
    (entries, guards): (&Entries, &dom::Guards),
    (modules, realms, entry, reach): (&Rc<modules::State>, &Realms<H>, &Entry, &Rc<Reach<H>>),
) -> rquickjs::Result<Object<'js>> {
    let global = ctx.globals();
    global.remove("performance")?;
    global.remove("SharedArrayBuffer")?;
    // A worker's `self` is its scope's (M31), and it has no `window`.
    if matches!(kind, RealmKind::Window | RealmKind::DevTools) {
        global.set("window", global.clone())?;
        global.set("self", global.clone())?;
    }
    // The console's one native (M36): the bootstrap's console calls it
    // with a call's level, kind, previews and stack, then the array of
    // the call's arguments and the first of them to log, which it formats
    // as the Console Standard's formatter does. The array is the call's
    // own, so that a heap spent needs nothing more of it. Formatting may
    // run the page's code, so the host is borrowed only once the text is
    // made.
    let console_host = host.clone();
    let console = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<()> {
            let head = args.0;
            if head.len() < 6 {
                return Ok(());
            }
            let first = head[5].as_int().unwrap_or(0).max(0) as usize;
            let rest: Vec<Value<'js>> = match head[4].as_array() {
                Some(array) => array
                    .iter::<Value<'js>>()
                    .skip(first)
                    .collect::<rquickjs::Result<_>>()?,
                None => Vec::new(),
            };
            let string = |value: &Value<'js>| {
                value
                    .as_string()
                    .and_then(|text| text.to_string().ok())
                    .unwrap_or_default()
            };
            let level = match head[0].as_int() {
                Some(1) => ConsoleLevel::Info,
                Some(2) => ConsoleLevel::Warn,
                Some(3) => ConsoleLevel::Error,
                Some(4) => ConsoleLevel::Debug,
                _ => ConsoleLevel::Log,
            };
            let kind = match string(&head[1]).as_str() {
                "dir" => ConsoleKind::Dir,
                "table" => ConsoleKind::Table,
                "trace" => ConsoleKind::Trace,
                "group" => ConsoleKind::Group,
                "groupCollapsed" => ConsoleKind::GroupCollapsed,
                "groupEnd" => ConsoleKind::GroupEnd,
                "clear" => ConsoleKind::Clear,
                _ => ConsoleKind::Log,
            };
            let mut previews = string(&head[2]);
            if previews.len() > MAX_PREVIEW_BYTES {
                previews = String::new();
            }
            let stack = parse_stack(&string(&head[3]));
            let text = match console::format(&ctx, &rest)? {
                Some(text) => text,
                // A log of nothing logs nothing (Logger, step 1).
                None if matches!(
                    kind,
                    ConsoleKind::Log | ConsoleKind::Dir | ConsoleKind::Table | ConsoleKind::Trace
                ) =>
                {
                    return Ok(());
                }
                None => String::new(),
            };
            let record = ConsoleRecord {
                level,
                kind,
                text,
                previews,
                stack,
            };
            with_host(&ctx, &console_host, |host| host.console_record(record))
        },
    )?
    .with_name("consoleMessage")?;
    dom::install(
        ctx,
        kind,
        secure,
        (host, budget),
        (entries, guards),
        (modules, realms, entry, reach),
        console,
    )
}

thread_local! {
    /// Calls of a realm's natives with no host lent (G90): a document of
    /// the agent its engine did not lend for the run that reached it (a
    /// frame's animation frame callbacks calling into their parent, phase
    /// 4). Taken by the engine for the site sweep.
    static UNLENT: Cell<u32> = const { Cell::new(0) };
}

/// The calls of natives with no host lent since this was last asked, on
/// this thread ([`UNLENT`]).
pub fn take_unlent_calls() -> u32 {
    UNLENT.with(|count| count.replace(0))
}

/// Runs `f` in `context`: through `Context::with`, or, given `held`, a
/// context of the same runtime whose run holds its lock, under that
/// lock (G90 phase 3 (b)), where `Context::with` would take it again.
fn enter<R>(
    held: Option<&Ctx<'_>>,
    context: &Context,
    f: impl for<'js> FnOnce(Ctx<'js>) -> R,
) -> R {
    match held {
        None => context.with(f),
        Some(held) => {
            context::with_locked(held, context, f).expect("a context of the held runtime")
        }
    }
}

/// Releases to `host` the nodes `released` holds, whose objects were
/// collected while no host of their realm's was there (G90).
fn release_to<H: ScriptingHost>(released: &Released, host: &mut H) {
    let handles = std::mem::take(&mut *released.borrow_mut());
    for handle in handles {
        // Releasing answers nothing, and refuses nothing a caller sees.
        let _ = host.dom(crate::dom::Call::Released(handle));
    }
}

/// Calls `f` with the host, which is there only while a run the engine
/// started is under way; otherwise throws.
fn with_host<H: ScriptingHost, R>(
    ctx: &Ctx<'_>,
    host: &Slot<H>,
    f: impl FnOnce(&mut H) -> R,
) -> rquickjs::Result<R> {
    let Ok(mut slot) = host.try_borrow_mut() else {
        return Err(Exception::throw_internal(ctx, "the host is busy"));
    };
    match slot.as_mut() {
        Some(host) => Ok(f(host)),
        None => {
            UNLENT.with(|count| count.set(count.get().saturating_add(1)));
            Err(Exception::throw_internal(ctx, "no script is running"))
        }
    }
}

#[cfg(test)]
mod stale_tests {
    use rquickjs::{Context, Exception, Runtime};

    use super::{Stale, clear_stale};

    /// A SyntaxError pending, QuickJS-ng's parser's, is cleared, said and
    /// let go.
    #[test]
    fn a_pending_syntax_error_is_cleared() {
        let runtime = Runtime::new().expect("a runtime");
        let context = Context::full(&runtime).expect("a context");
        context.with(|ctx| {
            let _ = Exception::throw_syntax(&ctx, "invalid number literal");
            clear_stale(&ctx, Stale::Begun);
            assert!(!ctx.has_exception());
        });
    }

    /// Not a SyntaxError though named one: a DOMException's (an invalid
    /// selector's), one of ours left pending, panics in a debug build.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "an exception not QuickJS-ng's parser's was pending (Native)")]
    fn a_dom_exception_named_syntax_error_panics_in_a_debug_build() {
        let runtime = Runtime::new().expect("a runtime");
        let context = Context::full(&runtime).expect("a context");
        context.with(|ctx| {
            let named = rquickjs::Object::new(ctx.clone()).expect("an object");
            named.set("name", "SyntaxError").expect("its name");
            named
                .set("message", "'##' is not a valid selector")
                .expect("its message");
            let _ = ctx.throw(named.into_value());
            clear_stale(&ctx, Stale::Native);
        });
    }

    /// Nor one of a class the page made SyntaxError's name.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "an exception not QuickJS-ng's parser's was pending (Compiled)")]
    fn a_page_class_named_syntax_error_panics_in_a_debug_build() {
        let runtime = Runtime::new().expect("a runtime");
        let context = Context::full(&runtime).expect("a context");
        context.with(|ctx| {
            let made: rquickjs::Value<'_> = ctx
                .eval("globalThis.SyntaxError = class SyntaxError extends Error {}; new SyntaxError('x')")
                .expect("it runs");
            let _ = ctx.throw(made);
            clear_stale(&ctx, Stale::Compiled);
        });
    }

    /// Anything else pending as a run begins is a bug of ours: a debug
    /// build panics.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "an exception not QuickJS-ng's parser's was pending (Begun)")]
    fn another_pending_error_panics_in_a_debug_build() {
        let runtime = Runtime::new().expect("a runtime");
        let context = Context::full(&runtime).expect("a context");
        context.with(|ctx| {
            let _ = Exception::throw_type(&ctx, "not a string");
            clear_stale(&ctx, Stale::Begun);
        });
    }
}
