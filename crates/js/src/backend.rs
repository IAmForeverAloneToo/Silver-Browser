//! The script engine a process runs (SilverJS's design note, phase J0):
//! QuickJS-ng, the only backend until SilverJS joins it (J5).
//!
//! The choice is `SILVER_JS_ENGINE`, read once per process: unset, empty
//! or `quickjs` is QuickJS-ng; any other name is refused, and then every
//! agent and realm the process asks for fails with a [`ScriptError`]
//! that says why. `engine` makes its agents and realms through
//! [`engine`] alone, so that a second backend changes no `engine` code:
//! [`Agent`] and [`WeakAgent`] are enums over the backends' own, with
//! one variant for now. One process runs one backend: the realms of an
//! agent share its heap.

#[cfg(not(feature = "quickjs"))]
compile_error!("the js crate needs a script engine: its `quickjs` feature");

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};

use crate::quickjs;
use crate::{
    ErrorKind, Limits, Member, Realm, RealmKind, ScriptEngine, ScriptError, ScriptingHost,
};

/// The engine a process runs when `SILVER_JS_ENGINE` is unset.
pub const DEFAULT_ENGINE: &str = "quickjs";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    QuickJs,
    /// `SILVER_JS_ENGINE` named no backend of this build.
    Refused,
}

/// A script engine: one of the backends this build has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Engine(Backend);

impl Engine {
    /// The engine `name` names, or why it is refused.
    pub fn named(name: &str) -> Result<Engine, String> {
        match name {
            "quickjs" => Ok(Engine(Backend::QuickJs)),
            "silverjs" => Err("SILVER_JS_ENGINE=silverjs: this build has no SilverJS yet".into()),
            other => Err(format!(
                "SILVER_JS_ENGINE names no script engine: {other:?} (this build has quickjs)"
            )),
        }
    }

    /// A new agent (G90): one heap under `limits`, one job queue, one
    /// budget and one interrupt check, which `stop` stops.
    pub fn agent<H: ScriptingHost>(
        &self,
        limits: Limits,
        stop: Option<Arc<AtomicBool>>,
    ) -> Result<Agent<H>, ScriptError> {
        match self.0 {
            Backend::QuickJs => quickjs::QuickJs.agent(limits, stop).map(Agent::QuickJs),
            Backend::Refused => Err(refused()),
        }
    }

    /// A new realm of `kind` in `agent`, sharing its heap.
    pub fn realm_in<H: ScriptingHost>(
        &self,
        agent: &Agent<H>,
        kind: RealmKind,
        secure: bool,
    ) -> Result<Box<dyn Realm<H>>, ScriptError> {
        match (self.0, agent) {
            (Backend::QuickJs, Agent::QuickJs(agent)) => {
                quickjs::QuickJs.realm_in(agent, kind, secure)
            }
            (Backend::Refused, _) => Err(refused()),
        }
    }
}

impl ScriptEngine for Engine {
    fn realm_of<H: ScriptingHost>(
        &self,
        kind: RealmKind,
        limits: Limits,
        stop: Option<Arc<AtomicBool>>,
    ) -> Result<Box<dyn Realm<H>>, ScriptError> {
        match self.0 {
            Backend::QuickJs => quickjs::QuickJs.realm_of(kind, limits, stop),
            Backend::Refused => Err(refused()),
        }
    }
}

/// The choice, made at the first call: the engine, or why none.
fn chosen() -> &'static Result<Engine, String> {
    static CHOSEN: OnceLock<Result<Engine, String>> = OnceLock::new();
    CHOSEN.get_or_init(|| {
        let name = std::env::var("SILVER_JS_ENGINE").unwrap_or_default();
        Engine::named(if name.is_empty() {
            DEFAULT_ENGINE
        } else {
            &name
        })
    })
}

/// The script engine this process runs.
pub fn engine() -> Engine {
    match chosen() {
        Ok(engine) => *engine,
        Err(_) => Engine(Backend::Refused),
    }
}

fn refused() -> ScriptError {
    let message = match chosen() {
        Err(why) => why.clone(),
        Ok(_) => "no script engine".into(),
    };
    ScriptError {
        kind: ErrorKind::Exception,
        message,
        url: String::new(),
        line: 0,
        column: 0,
        stack: Vec::new(),
    }
}

/// An agent (HTML section 8.1.2.1, G90) of the process's engine: the
/// heap, job queue, budget and interrupt check its realms share.
pub enum Agent<H: ScriptingHost> {
    QuickJs(quickjs::Agent<H>),
}

impl<H: ScriptingHost> Clone for Agent<H> {
    fn clone(&self) -> Self {
        match self {
            Agent::QuickJs(a) => Agent::QuickJs(a.clone()),
        }
    }
}

impl<H: ScriptingHost> Agent<H> {
    /// A handle that does not keep the agent.
    pub fn downgrade(&self) -> WeakAgent<H> {
        match self {
            Agent::QuickJs(a) => WeakAgent::QuickJs(a.downgrade()),
        }
    }

    /// Whether `other` is this very agent.
    pub fn same(&self, other: &Agent<H>) -> bool {
        match (self, other) {
            (Agent::QuickJs(a), Agent::QuickJs(b)) => a.same(b),
        }
    }

    /// Whether any realm is made in it.
    pub fn has_realms(&self) -> bool {
        match self {
            Agent::QuickJs(a) => a.has_realms(),
        }
    }

    /// How many realms are made in it.
    pub fn realm_count(&self) -> usize {
        match self {
            Agent::QuickJs(a) => a.realm_count(),
        }
    }

    /// Lends the document of frame `frame`, which has no realm, for a run
    /// of another of its realms (G90 phase 3 (b)).
    pub fn lend_unborn(&self, frame: u32, host: H) {
        match self {
            Agent::QuickJs(a) => a.lend_unborn(frame, host),
        }
    }

    /// The document lent for frame `frame`, if no realm was made for it.
    pub fn take_unborn(&self, frame: u32) -> Option<H> {
        match self {
            Agent::QuickJs(a) => a.take_unborn(frame),
        }
    }

    /// The realm made for frame `frame`'s document as a run reached it.
    pub fn take_made(&self, frame: u32) -> Option<Box<dyn Realm<H>>> {
        match self {
            Agent::QuickJs(a) => a.take_made(frame),
        }
    }

    /// Whether a realm made by a reach waits for frame `frame`'s engine.
    pub fn has_made(&self, frame: u32) -> bool {
        match self {
            Agent::QuickJs(a) => a.has_made(frame),
        }
    }

    /// Drops the realms made by a reach that no engine took, and gives
    /// back the documents still lent as unborn.
    pub fn drop_unclaimed(&self) -> (usize, Vec<(u32, H)>) {
        match self {
            Agent::QuickJs(a) => a.drop_unclaimed(),
        }
    }

    /// The heap it may hold now.
    pub fn heap_limit(&self) -> usize {
        match self {
            Agent::QuickJs(a) => a.heap_limit(),
        }
    }

    /// Whether more than one document is of it, realm made or not, or
    /// more than one realm made.
    pub fn shared(&self) -> bool {
        match self {
            Agent::QuickJs(a) => a.shared(),
        }
    }

    /// A document's place in the agent, for as long as it is kept.
    pub fn join(&self) -> Member {
        match self {
            Agent::QuickJs(a) => a.join(),
        }
    }
}

/// An agent not kept ([`Agent::downgrade`]).
pub enum WeakAgent<H: ScriptingHost> {
    QuickJs(quickjs::WeakAgent<H>),
}

impl<H: ScriptingHost> WeakAgent<H> {
    /// The agent, while a realm of it lives.
    pub fn upgrade(&self) -> Option<Agent<H>> {
        match self {
            WeakAgent::QuickJs(w) => w.upgrade().map(Agent::QuickJs),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_ENGINE, Engine, engine};
    use crate::{ErrorKind, Limits, RealmKind, ScriptEngine};

    #[test]
    fn only_quickjs_is_accepted() {
        assert_eq!(Engine::named(DEFAULT_ENGINE), Ok(engine()));
        assert!(Engine::named("quickjs").is_ok());
        for refused in ["silverjs", "v8", "QuickJS", " quickjs", ""] {
            let why = Engine::named(refused).unwrap_err();
            assert!(why.starts_with("SILVER_JS_ENGINE"), "{why}");
        }
    }

    #[test]
    fn a_refused_engine_makes_nothing() {
        let refused = Engine(super::Backend::Refused);
        let err = refused
            .realm_of::<crate::tests::Recorder>(RealmKind::Window, Limits::default(), None)
            .err()
            .expect("refused");
        assert_eq!(err.kind, ErrorKind::Exception);
        assert!(
            refused
                .agent::<crate::tests::Recorder>(Limits::default(), None)
                .is_err()
        );
        // The process's own engine makes both, and a realm in its agent.
        let agent = engine()
            .agent::<crate::tests::Recorder>(Limits::default(), None)
            .expect("an agent");
        let realm = engine().realm_in(&agent, RealmKind::Window, true);
        assert!(realm.is_ok());
        assert_eq!(agent.realm_count(), 1);
        assert!(agent.same(&agent.clone()));
        assert!(agent.downgrade().upgrade().is_some());
    }
}
