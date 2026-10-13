//! The permissions the user gives a site (M33.4,
//! docs/design/hardening.md): Local Network Access's `local-network` and
//! `loopback-network`, asked for in a prompt the shell shows under its
//! address field, and kept in the profile once given.
//!
//! A request of a page that may ask, on its way into a more private
//! address space than its page's, waits on its fetch's thread for the
//! answer: the one kept for its site, or the user's. The requests that
//! come while a prompt for the same site and permission is open wait on
//! the same answer.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use net::{AbortHandle, AddressSpace, LocalNetworkGate, Origin};

use crate::loader::pool::Waker;

/// How long a prompt waits for its answer: then the requests waiting on
/// it are refused, and nothing is kept.
pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(60);

/// How often a waiting request looks at its abort.
const POLL: Duration = Duration::from_millis(50);

/// The most answers kept: past it, a new answer is not kept.
const MAX_KEPT: usize = 1000;

/// A permission a site may be given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    /// To reach the local network.
    LocalNetwork,
    /// To reach this machine.
    LoopbackNetwork,
}

impl Permission {
    /// Its name, as Permissions Policy and the profile's records have it.
    pub fn name(self) -> &'static str {
        match self {
            Permission::LocalNetwork => "local-network",
            Permission::LoopbackNetwork => "loopback-network",
        }
    }

    fn from_name(name: &str) -> Option<Permission> {
        match name {
            "local-network" => Some(Permission::LocalNetwork),
            "loopback-network" => Some(Permission::LoopbackNetwork),
            _ => None,
        }
    }

    /// The permission a request into `space` needs; none for the public
    /// one, which needs none.
    fn for_space(space: AddressSpace) -> Option<Permission> {
        match space {
            AddressSpace::Loopback => Some(Permission::LoopbackNetwork),
            AddressSpace::Local => Some(Permission::LocalNetwork),
            AddressSpace::Public => None,
        }
    }
}

/// A prompt the shell is to show: which, the site asking, and for what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionPrompt {
    pub id: u64,
    /// The origin asking, serialized.
    pub origin: String,
    pub permission: Permission,
}

impl PermissionPrompt {
    /// What the prompt says.
    pub fn text(&self) -> String {
        match self.permission {
            Permission::LocalNetwork => {
                format!(
                    "{} wants to connect to devices on your local network",
                    self.origin
                )
            }
            Permission::LoopbackNetwork => {
                format!("{} wants to connect to apps on this device", self.origin)
            }
        }
    }
}

/// A prompt open, and the requests waiting on it.
struct Open {
    id: u64,
    origin: String,
    permission: Permission,
    opened: Instant,
    waiting: usize,
    /// Its answer, once it has one: `false` for a prompt dismissed or
    /// not answered in time.
    outcome: Option<bool>,
}

#[derive(Default)]
struct State {
    /// The answers kept, by origin and permission.
    kept: HashMap<(String, Permission), bool>,
    /// How many times they changed: what a profile writes when it moved.
    changes: u64,
    /// The prompts open, oldest first.
    open: Vec<Open>,
    next: u64,
}

/// The engine's answers to the requests that would reach a more private
/// address space: kept, or asked of the user.
pub(crate) struct Permissions {
    state: Mutex<State>,
    answered: Condvar,
    /// What wakes the engine's owner when a prompt opens or closes.
    waker: Mutex<Option<Waker>>,
    timeout: Duration,
}

impl fmt::Debug for Permissions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Permissions")
    }
}

impl Default for Permissions {
    fn default() -> Permissions {
        Permissions::with_timeout(PROMPT_TIMEOUT)
    }
}

impl Permissions {
    /// Permissions whose prompts wait `timeout` for an answer.
    pub(crate) fn with_timeout(timeout: Duration) -> Permissions {
        Permissions {
            state: Mutex::new(State::default()),
            answered: Condvar::new(),
            waker: Mutex::new(None),
            timeout,
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn set_waker(&self, waker: Waker) {
        *self.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }

    fn wake(&self) {
        let waker = self
            .waker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(waker) = waker {
            waker();
        }
    }

    /// The prompt to show: the oldest open without an answer.
    pub(crate) fn prompt(&self) -> Option<PermissionPrompt> {
        let state = self.state();
        let open = state.open.iter().find(|open| open.outcome.is_none())?;
        Some(PermissionPrompt {
            id: open.id,
            origin: open.origin.clone(),
            permission: open.permission,
        })
    }

    /// The user's answer to the prompt `id`: `Some(allow)` kept for its
    /// site, `None` (dismissed) refusing the requests waiting without
    /// keeping anything.
    pub(crate) fn answer(&self, id: u64, answer: Option<bool>) {
        let mut state = self.state();
        let Some(index) = state
            .open
            .iter()
            .position(|open| open.id == id && open.outcome.is_none())
        else {
            return;
        };
        state.open[index].outcome = Some(answer == Some(true));
        if let Some(allow) = answer {
            let key = (
                state.open[index].origin.clone(),
                state.open[index].permission,
            );
            if state.kept.len() < MAX_KEPT || state.kept.contains_key(&key) {
                state.kept.insert(key, allow);
                state.changes += 1;
            }
        }
        if state.open[index].waiting == 0 {
            state.open.remove(index);
        }
        drop(state);
        self.answered.notify_all();
    }

    /// How many times the answers kept changed.
    pub(crate) fn changes(&self) -> u64 {
        self.state().changes
    }

    /// The answers kept, as records for a profile: the origin, the
    /// permission's name, and `1` or `0`, tab-separated.
    pub(crate) fn export(&self) -> Vec<Vec<u8>> {
        let state = self.state();
        let mut records: Vec<Vec<u8>> = state
            .kept
            .iter()
            .map(|((origin, permission), allow)| {
                format!("{origin}\t{}\t{}", permission.name(), u8::from(*allow)).into_bytes()
            })
            .collect();
        records.sort();
        records
    }

    /// Keeps the answers of `records`, a profile's; a record that does
    /// not read is left out.
    pub(crate) fn restore(&self, records: &[Vec<u8>]) {
        let mut state = self.state();
        for record in records {
            let Ok(text) = std::str::from_utf8(record) else {
                continue;
            };
            let mut parts = text.split('\t');
            let (Some(origin), Some(name), Some(allow), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let (Some(permission), Some(allow)) = (
                Permission::from_name(name),
                match allow {
                    "1" => Some(true),
                    "0" => Some(false),
                    _ => None,
                },
            ) else {
                continue;
            };
            if origin.is_empty() || state.kept.len() >= MAX_KEPT {
                continue;
            }
            state.kept.insert((origin.to_owned(), permission), allow);
        }
    }
}

impl LocalNetworkGate for Permissions {
    fn allow(&self, origin: &Origin, target: AddressSpace, abort: &AbortHandle) -> bool {
        let Some(permission) = Permission::for_space(target) else {
            return false;
        };
        if origin.is_opaque() {
            return false;
        }
        let origin = origin.serialization();
        let mut state = self.state();
        if let Some(&kept) = state.kept.get(&(origin.clone(), permission)) {
            return kept;
        }
        let waiting = state.open.iter_mut().find(|open| {
            open.origin == origin && open.permission == permission && open.outcome.is_none()
        });
        let id = match waiting {
            Some(open) => {
                open.waiting += 1;
                open.id
            }
            None => {
                let id = state.next;
                state.next += 1;
                state.open.push(Open {
                    id,
                    origin,
                    permission,
                    opened: Instant::now(),
                    waiting: 1,
                    outcome: None,
                });
                drop(state);
                self.wake();
                state = self.state();
                id
            }
        };
        loop {
            let index = state
                .open
                .iter()
                .position(|open| open.id == id)
                .expect("a prompt stays while a request waits on it");
            let open = &mut state.open[index];
            let outcome = match open.outcome {
                Some(outcome) => Some(outcome),
                // Not answered in time: refused, the others with it.
                None if open.opened.elapsed() >= self.timeout => {
                    open.outcome = Some(false);
                    self.answered.notify_all();
                    Some(false)
                }
                None if abort.is_aborted() => Some(false),
                None => None,
            };
            if let Some(outcome) = outcome {
                open.waiting -= 1;
                if open.waiting == 0 {
                    // Nobody waits: the prompt goes, answered or not.
                    state.open.remove(index);
                    drop(state);
                    self.wake();
                }
                return outcome;
            }
            state = self
                .answered
                .wait_timeout(state, POLL)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread;

    use super::*;

    /// Long enough for any answer: a request that waits longer hangs.
    const WAIT: Duration = Duration::from_secs(10);

    fn origin(s: &str) -> Origin {
        net::Url::parse(s).unwrap().origin()
    }

    /// Asks on a thread of its own, as a fetch does, `abort` its abort.
    fn ask_with(
        permissions: &Arc<Permissions>,
        url: &str,
        space: AddressSpace,
        abort: AbortHandle,
    ) -> mpsc::Receiver<bool> {
        let (permissions, origin) = (permissions.clone(), origin(url));
        let (send, answered) = mpsc::channel();
        thread::spawn(move || {
            let _ = send.send(permissions.allow(&origin, space, &abort));
        });
        answered
    }

    fn ask(permissions: &Arc<Permissions>, url: &str, space: AddressSpace) -> mpsc::Receiver<bool> {
        ask_with(permissions, url, space, AbortHandle::new())
    }

    /// Waits for a prompt to be open.
    fn prompted(permissions: &Permissions) -> PermissionPrompt {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(prompt) = permissions.prompt() {
                return prompt;
            }
            assert!(Instant::now() < deadline, "no prompt");
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_prompt_is_answered_once_for_the_requests_waiting_and_kept() {
        let permissions = Arc::new(Permissions::default());
        let woken = Arc::new(AtomicUsize::new(0));
        let count = woken.clone();
        permissions.set_waker(Arc::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
        }));
        let first = ask(&permissions, "https://a.test/", AddressSpace::Local);
        let prompt = prompted(&permissions);
        // The shell is woken to show it.
        let deadline = Instant::now() + WAIT;
        while woken.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "not woken");
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(prompt.origin, "https://a.test");
        assert_eq!(prompt.permission, Permission::LocalNetwork);
        assert_eq!(
            prompt.text(),
            "https://a.test wants to connect to devices on your local network"
        );
        let second = ask(&permissions, "https://a.test/x", AddressSpace::Local);
        // Another site's, or another permission's, is a prompt of its own.
        let other = ask(&permissions, "https://a.test/", AddressSpace::Loopback);
        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            permissions.prompt(),
            Some(prompt.clone()),
            "the oldest first"
        );
        permissions.answer(prompt.id, Some(true));
        assert!(first.recv_timeout(WAIT).unwrap());
        assert!(second.recv_timeout(WAIT).unwrap());
        let next = prompted(&permissions);
        assert_eq!(next.permission, Permission::LoopbackNetwork);
        permissions.answer(next.id, Some(false));
        assert!(!other.recv_timeout(WAIT).unwrap());
        assert!(woken.load(Ordering::SeqCst) >= 2);
        // Kept: asked no more.
        assert!(
            ask(&permissions, "https://a.test/", AddressSpace::Local)
                .recv_timeout(WAIT)
                .unwrap()
        );
        assert!(
            !ask(&permissions, "https://a.test/", AddressSpace::Loopback)
                .recv_timeout(WAIT)
                .unwrap()
        );
        assert_eq!(permissions.prompt(), None);
        assert_eq!(permissions.changes(), 2);
        // The public space needs none; an opaque origin is never asked.
        assert!(!permissions.allow(
            &origin("https://b.test/"),
            AddressSpace::Public,
            &AbortHandle::new()
        ));
        let (send, opaque) = mpsc::channel();
        let waiting = permissions.clone();
        thread::spawn(move || {
            let answer = waiting.allow(
                &Origin::new_opaque(),
                AddressSpace::Local,
                &AbortHandle::new(),
            );
            let _ = send.send(answer);
        });
        let answer = opaque.recv_timeout(Duration::from_secs(1));
        assert_eq!(answer, Ok(false), "refused without asking");
        assert_eq!(permissions.prompt(), None);
    }

    #[test]
    fn a_prompt_dismissed_or_not_answered_refuses_without_keeping() {
        let permissions = Arc::new(Permissions::with_timeout(Duration::from_millis(300)));
        let first = ask(&permissions, "https://a.test/", AddressSpace::Local);
        let prompt = prompted(&permissions);
        permissions.answer(prompt.id, None);
        assert!(!first.recv_timeout(WAIT).unwrap());
        assert_eq!(permissions.changes(), 0);
        // Asked again, then not answered in time.
        let again = ask(&permissions, "https://a.test/", AddressSpace::Local);
        let prompt = prompted(&permissions);
        assert!(!again.recv_timeout(WAIT).unwrap());
        assert_eq!(permissions.prompt(), None);
        // An answer too late does nothing.
        permissions.answer(prompt.id, Some(true));
        assert_eq!(permissions.changes(), 0);
        // A request aborted stops waiting, before any timeout; with
        // nobody waiting, the prompt goes.
        let permissions = Arc::new(Permissions::default());
        let abort = AbortHandle::new();
        let aborted = ask_with(
            &permissions,
            "https://c.test/",
            AddressSpace::Local,
            abort.clone(),
        );
        prompted(&permissions);
        abort.abort();
        assert!(!aborted.recv_timeout(WAIT).unwrap());
        assert_eq!(permissions.prompt(), None);
    }

    #[test]
    fn the_answers_kept_are_records_for_the_profile() {
        let permissions = Arc::new(Permissions::default());
        let asked = ask(&permissions, "https://a.test/", AddressSpace::Loopback);
        permissions.answer(prompted(&permissions).id, Some(true));
        assert!(asked.recv_timeout(WAIT).unwrap());
        let blocked = ask(&permissions, "https://a.test/", AddressSpace::Local);
        permissions.answer(prompted(&permissions).id, Some(false));
        assert!(!blocked.recv_timeout(WAIT).unwrap());
        let records = permissions.export();
        assert_eq!(
            records,
            [
                b"https://a.test\tlocal-network\t0".to_vec(),
                b"https://a.test\tloopback-network\t1".to_vec()
            ]
        );
        let back = Permissions::default();
        back.restore(&records);
        back.restore(&[
            b"https://b.test\tcamera\t1".to_vec(),
            b"https://b.test\tlocal-network\tyes".to_vec(),
            b"\tlocal-network\t1".to_vec(),
            vec![0xff],
        ]);
        assert_eq!(back.export(), records);
        let kept = |space| back.allow(&origin("https://a.test/"), space, &AbortHandle::new());
        assert!(kept(AddressSpace::Loopback));
        assert!(!kept(AddressSpace::Local));
    }
}
