//! Channel messaging across realms (M31.5, HTML section 9.5): every
//! `MessagePort` of a document and of its workers is known to one
//! router, which keeps each port's entangled port, the realm holding it
//! (none while it is in a message on its way), and the messages that
//! wait for it to be started or to arrive.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use js::{Answer, Call, DomError};

use super::workers::{Inbox, MAX_MESSAGE_BYTES, MAX_QUEUED_BYTES, WorkerMessage};

/// A document's ports, its workers' included: past it, a new channel
/// is refused.
pub(crate) const MAX_PORTS: usize = 65_536;

/// A port the router knows.
struct Port {
    entangled: Option<u32>,
    /// The realm holding it; none while it is on its way.
    holder: Option<u32>,
    /// Whether its realm started it: until then, and while it is on its
    /// way, its messages wait here.
    started: bool,
    waiting: VecDeque<String>,
}

/// The ports of a document and its workers, and the realms' inboxes.
#[derive(Default)]
pub(crate) struct Router {
    ports: HashMap<u32, Port>,
    next_port: u32,
    realms: HashMap<u32, Inbox>,
    next_realm: u32,
    /// The bytes of the messages waiting here.
    waiting: usize,
}

/// The router a document and its workers share.
pub(crate) type SharedRouter = Arc<Mutex<Router>>;

/// `router`, locked.
pub(crate) fn lock(router: &SharedRouter) -> MutexGuard<'_, Router> {
    router.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Router {
    /// Sends realm `realm`, a worker's document's, `message` (M31.4: a
    /// worker's frame); the realms are the tab's since M33.2.
    pub(crate) fn send_to(&self, realm: u32, message: WorkerMessage) {
        if let Some(inbox) = self.realms.get(&realm) {
            inbox.send(message);
        }
    }

    /// A new realm, whose port messages go to `inbox`: its number.
    pub(crate) fn add_realm(&mut self, inbox: Inbox) -> u32 {
        let realm = self.next_realm;
        self.next_realm += 1;
        self.realms.insert(realm, inbox);
        realm
    }

    /// Realm `realm` ended: its ports are closed.
    pub(crate) fn remove_realm(&mut self, realm: u32) {
        self.realms.remove(&realm);
        let held: Vec<u32> = self
            .ports
            .iter()
            .filter(|(_, port)| port.holder == Some(realm))
            .map(|(id, _)| *id)
            .collect();
        for id in held {
            self.close(realm, id);
        }
    }

    /// `new MessageChannel()` in `realm`: two entangled ports.
    pub(crate) fn channel(&mut self, realm: u32) -> Result<(u32, u32), DomError> {
        if self.ports.len() + 2 > MAX_PORTS {
            return Err(DomError::new(
                "RangeError",
                format!("No more than {MAX_PORTS} message ports may be open."),
            ));
        }
        let (a, b) = (self.next_port, self.next_port + 1);
        self.next_port += 2;
        for (id, other) in [(a, b), (b, a)] {
            self.ports.insert(
                id,
                Port {
                    entangled: Some(other),
                    holder: Some(realm),
                    started: false,
                    waiting: VecDeque::new(),
                },
            );
        }
        Ok((a, b))
    }

    /// Posts `data` from port `from`, which `realm` holds, to its
    /// entangled port: to its realm's inbox when it is started and
    /// there, else kept for it. Nothing for a port not entangled.
    pub(crate) fn post(&mut self, realm: u32, from: u32, data: String) -> Result<(), DomError> {
        if data.len() > MAX_MESSAGE_BYTES {
            return Err(too_large());
        }
        let Some(to) = self
            .ports
            .get(&from)
            .filter(|port| port.holder == Some(realm))
            .and_then(|port| port.entangled)
        else {
            return Ok(());
        };
        let Some(port) = self.ports.get_mut(&to) else {
            return Ok(());
        };
        if port.started
            && let Some(holder) = port.holder
            && let Some(inbox) = self.realms.get(&holder)
        {
            if !inbox.send(WorkerMessage::Port(to, data)) {
                return Err(too_large());
            }
            return Ok(());
        }
        if self.waiting + data.len() > MAX_QUEUED_BYTES {
            return Err(too_large());
        }
        self.waiting += data.len();
        port.waiting.push_back(data);
        Ok(())
    }

    /// Starts port `id` of `realm`: what waits for it goes to the realm.
    pub(crate) fn start(&mut self, realm: u32, id: u32) {
        let Some(port) = self
            .ports
            .get_mut(&id)
            .filter(|port| port.holder == Some(realm))
        else {
            return;
        };
        port.started = true;
        let waiting = std::mem::take(&mut port.waiting);
        if let Some(inbox) = self.realms.get(&realm) {
            for data in waiting {
                self.waiting -= data.len();
                inbox.send(WorkerMessage::Port(id, data));
            }
        }
    }

    /// Port `id` of `realm` is in a message on its way: it is held by no
    /// realm, and not started, until one takes it.
    pub(crate) fn send_away(&mut self, realm: u32, id: u32) {
        if let Some(port) = self
            .ports
            .get_mut(&id)
            .filter(|port| port.holder == Some(realm))
        {
            port.holder = None;
            port.started = false;
        }
    }

    /// `realm` took port `id` from a message: whether it may hold it, a
    /// port on its way.
    pub(crate) fn adopt(&mut self, realm: u32, id: u32) -> bool {
        match self.ports.get_mut(&id) {
            Some(port) if port.holder.is_none() => {
                port.holder = Some(realm);
                true
            }
            _ => false,
        }
    }

    /// `port.close()` in `realm`: both ports disentangled, what waited
    /// for it dropped.
    pub(crate) fn close(&mut self, realm: u32, id: u32) {
        if self
            .ports
            .get(&id)
            .is_none_or(|port| port.holder != Some(realm))
        {
            return;
        }
        if let Some(port) = self.ports.remove(&id) {
            self.waiting -= port.waiting.iter().map(String::len).sum::<usize>();
            if let Some(other) = port.entangled.and_then(|other| self.ports.get_mut(&other)) {
                other.entangled = None;
            }
        }
    }
}

/// A port's call from `realm` (M31.5); `None` for another call.
pub(crate) fn port_call(
    router: &SharedRouter,
    realm: u32,
    call: &Call,
) -> Option<Result<Answer, DomError>> {
    let mut router = lock(router);
    Some(match call {
        Call::PortChannel => router
            .channel(realm)
            .map(|(a, b)| Answer::Numbers(vec![f64::from(a), f64::from(b)])),
        Call::PortPost { port, data } => router
            .post(realm, *port, data.clone())
            .map(|()| Answer::Nothing),
        Call::PortStart(port) => {
            router.start(realm, *port);
            Ok(Answer::Nothing)
        }
        Call::PortClose(port) => {
            router.close(realm, *port);
            Ok(Answer::Nothing)
        }
        Call::PortAway(port) => {
            router.send_away(realm, *port);
            Ok(Answer::Nothing)
        }
        Call::PortAdopt(port) => Ok(Answer::Bool(router.adopt(realm, *port))),
        _ => return None,
    })
}

/// The `DataCloneError` of a message past the bounds.
fn too_large() -> DomError {
    DomError::new("DataCloneError", "Data cannot be cloned, out of memory.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripts::workers::inbox;

    #[test]
    fn a_documents_ports_are_bounded() {
        let mut router = Router::default();
        let (to, _mailbox) = inbox(None);
        let realm = router.add_realm(to);
        let mut made = 0;
        while router.channel(realm).is_ok() {
            made += 2;
        }
        assert_eq!(made, MAX_PORTS);
        let refused = router.channel(realm).expect_err("past the bound");
        assert_eq!(refused.name, "RangeError");
    }

    #[test]
    fn a_ports_messages_wait_while_it_travels_and_go_where_it_lands() {
        let mut router = Router::default();
        let (here, here_box) = inbox(None);
        let (there, there_box) = inbox(None);
        let (a, b) = (router.add_realm(here), router.add_realm(there));
        let (one, two) = router.channel(a).unwrap();
        router.send_away(a, two);
        // Posting from a port the realm does not hold sends nothing.
        router.post(b, two, "forged".to_owned()).unwrap();
        router.post(a, one, "first".to_owned()).unwrap();
        assert!(!router.adopt(a, one), "held already");
        assert!(router.adopt(b, two));
        router.post(a, one, "second".to_owned()).unwrap();
        assert!(there_box.try_take().is_none(), "not started");
        router.start(b, two);
        let took: Vec<String> = std::iter::from_fn(|| there_box.try_take())
            .filter_map(|message| match message {
                WorkerMessage::Port(port, data) if port == two => Some(data),
                _ => None,
            })
            .collect();
        assert_eq!(took, ["first", "second"]);
        assert_eq!(router.waiting, 0);
        // Closed, neither side hears the other.
        router.close(b, two);
        router.post(a, one, "after".to_owned()).unwrap();
        assert!(there_box.try_take().is_none());
        assert!(here_box.try_take().is_none());
        assert!(
            router
                .ports
                .get(&one)
                .is_some_and(|port| port.entangled.is_none())
        );
    }
}
