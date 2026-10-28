//! Mutation observers (gap batch A, D42; `docs/design/scripting-essentials.md`):
//! a document's registrations, matched against the changes the `dom`
//! crate keeps while any is registered, and each observer's queue of
//! records, which the realm's mutation observer microtask takes.
//!
//! One binding's call is one operation: its changes to one parent's
//! children, one after another, are one record, as the DOM groups those
//! of `innerHTML`, `replaceChildren` or a fragment's insertion; a node
//! taken out and put back under the same parent (a move) is two. The
//! parser's insertions, outside any call, are a record each.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use dom::{Mutated, Mutation, NodeId};

/// The most records an observer's queue holds; past it the oldest go.
pub(crate) const MAX_MUTATION_RECORDS: usize = 100_000;

/// What `observe()` asked for, its defaults filled in as the DOM fills
/// them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Options {
    pub(crate) child_list: bool,
    pub(crate) attributes: bool,
    pub(crate) character_data: bool,
    pub(crate) subtree: bool,
    pub(crate) attribute_old_value: bool,
    pub(crate) character_data_old_value: bool,
    pub(crate) attribute_filter: Option<Vec<String>>,
}

/// An observer's registration on a node; a transient one, on a node
/// taken out of an observed subtree, lasts until the observer is next
/// notified.
struct Registration {
    observer: u32,
    node: NodeId,
    options: Options,
    transient: bool,
}

/// A record's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    ChildList,
    Attributes,
    CharacterData,
}

impl Kind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::ChildList => "childList",
            Kind::Attributes => "attributes",
            Kind::CharacterData => "characterData",
        }
    }
}

/// A `MutationRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    pub(crate) kind: Kind,
    pub(crate) target: NodeId,
    pub(crate) added: Vec<NodeId>,
    pub(crate) removed: Vec<NodeId>,
    pub(crate) previous: Option<NodeId>,
    pub(crate) next: Option<NodeId>,
    pub(crate) attribute: Option<String>,
    pub(crate) old: Option<String>,
}

impl Record {
    fn new(kind: Kind, target: NodeId) -> Record {
        Record {
            kind,
            target,
            added: Vec::new(),
            removed: Vec::new(),
            previous: None,
            next: None,
            attribute: None,
            old: None,
        }
    }

    /// The nodes the record names, which the document keeps while it is
    /// queued.
    fn nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        [self.target]
            .into_iter()
            .chain(self.added.iter().copied())
            .chain(self.removed.iter().copied())
            .chain(self.previous)
            .chain(self.next)
    }
}

/// A document's mutation observers, by the numbers the realm gave them.
#[derive(Default)]
pub(crate) struct MutationObservers {
    registrations: Vec<Registration>,
    queues: BTreeMap<u32, VecDeque<Record>>,
    /// A record was queued since the realm was last woken for one.
    wake: bool,
}

impl MutationObservers {
    /// Whether any observer is registered, so that changes are kept.
    pub(crate) fn observing(&self) -> bool {
        !self.registrations.is_empty()
    }

    /// `observe(node, options)`: a registration, or the one the observer
    /// has on `node` given the new options, its transient ones dropped.
    pub(crate) fn observe(&mut self, observer: u32, node: NodeId, options: Options) {
        let existing = self
            .registrations
            .iter()
            .position(|r| !r.transient && r.observer == observer && r.node == node);
        match existing {
            Some(at) => {
                self.registrations[at].options = options;
                self.registrations
                    .retain(|r| !(r.transient && r.observer == observer));
            }
            None => self.registrations.push(Registration {
                observer,
                node,
                options,
                transient: false,
            }),
        }
    }

    /// `disconnect()`: the observer's registrations and records gone.
    pub(crate) fn disconnect(&mut self, observer: u32) {
        self.registrations.retain(|r| r.observer != observer);
        self.queues.remove(&observer);
    }

    /// `takeRecords()`: the observer's records, its queue emptied.
    pub(crate) fn take(&mut self, observer: u32) -> Vec<Record> {
        self.queues
            .remove(&observer)
            .map(Vec::from)
            .unwrap_or_default()
    }

    /// The observers to notify, those with records, in the order they
    /// were made; their transient registrations go, as notifying them
    /// ends those.
    pub(crate) fn notifying(&mut self) -> Vec<u32> {
        let observers: Vec<u32> = self
            .queues
            .iter()
            .filter(|(_, queue)| !queue.is_empty())
            .map(|(&observer, _)| observer)
            .collect();
        self.registrations
            .retain(|r| !(r.transient && observers.contains(&r.observer)));
        observers
    }

    /// Whether a record was queued since this was last asked: the realm
    /// is to queue its mutation observer microtask.
    pub(crate) fn take_wake(&mut self) -> bool {
        std::mem::take(&mut self.wake)
    }

    /// The nodes queued records name, which the document must keep.
    pub(crate) fn held(&self) -> HashSet<NodeId> {
        self.queues
            .values()
            .flatten()
            .flat_map(Record::nodes)
            .collect()
    }

    /// The nodes registrations are on, each with whether one watches
    /// its subtree, for the document to tell changes of.
    pub(crate) fn watched(&self) -> HashMap<NodeId, bool> {
        let mut watched = HashMap::new();
        for r in &self.registrations {
            *watched.entry(r.node).or_default() |= r.options.subtree;
        }
        watched
    }

    /// Queues the records of `changes`, the document's since it was last
    /// asked: one operation's, grouped, when `operation`; else each its
    /// own record.
    pub(crate) fn record(&mut self, changes: Vec<Mutation>, operation: bool) {
        let mut group: Option<(Record, HashSet<NodeId>, Vec<NodeId>)> = None;
        for Mutation { change, watched } in changes {
            // A change no observer watches (a fragment's emptying as its
            // children are inserted) neither makes a record nor ends a
            // group.
            if watched.is_empty() {
                continue;
            }
            match change {
                Mutated::ChildList {
                    parent,
                    added,
                    removed,
                    prev,
                    next,
                } => {
                    if let Some(node) = removed {
                        self.transients(&watched, node);
                    }
                    // Joined to the group of the same parent's changes,
                    // unless it puts back a node the group took out.
                    let joins = operation
                        && group.as_ref().is_some_and(|(record, taken, _)| {
                            record.target == parent
                                && added.is_none_or(|node| !taken.contains(&node))
                        });
                    if !joins && let Some((record, _, watched)) = group.take() {
                        self.queue(record, &watched);
                    }
                    let (record, taken, _) = group.get_or_insert_with(|| {
                        let mut record = Record::new(Kind::ChildList, parent);
                        record.previous = prev;
                        (record, HashSet::new(), watched)
                    });
                    record.next = next;
                    record.added.extend(added);
                    record.removed.extend(removed);
                    taken.extend(removed);
                }
                Mutated::Attribute { element, name, old } => {
                    if let Some((record, _, watched)) = group.take() {
                        self.queue(record, &watched);
                    }
                    let mut record = Record::new(Kind::Attributes, element);
                    record.attribute = Some(name);
                    record.old = old;
                    self.queue(record, &watched);
                }
                Mutated::Text { node, old } => {
                    if let Some((record, _, watched)) = group.take() {
                        self.queue(record, &watched);
                    }
                    let mut record = Record::new(Kind::CharacterData, node);
                    record.old = Some(old);
                    self.queue(record, &watched);
                }
            }
        }
        if let Some((record, _, watched)) = group {
            self.queue(record, &watched);
        }
    }

    /// DOM's removing steps for observers: each registration with
    /// `subtree` on a watched inclusive ancestor of the parent, as
    /// `watched` gives them, gains a transient one on `node`, taken out
    /// from under it.
    fn transients(&mut self, watched: &[NodeId], node: NodeId) {
        let made: Vec<Registration> = self
            .registrations
            .iter()
            .filter(|r| r.options.subtree && watched.contains(&r.node))
            .map(|r| Registration {
                observer: r.observer,
                node,
                options: r.options.clone(),
                transient: true,
            })
            .collect();
        self.registrations.extend(made);
    }

    /// DOM's "queue a mutation record": `record` to each observer
    /// interested in a change to its target, through a registration on
    /// the target or, with `subtree`, on an ancestor of it (`watched`,
    /// those that were when it changed); with the old value only for
    /// those that asked for it.
    fn queue(&mut self, record: Record, watched: &[NodeId]) {
        let target = record.target;
        let mut interested: BTreeMap<u32, bool> = BTreeMap::new();
        for r in &self.registrations {
            if !watched.contains(&r.node) {
                continue;
            }
            let o = &r.options;
            let wanted = (r.node == target || o.subtree)
                && match record.kind {
                    Kind::ChildList => o.child_list,
                    Kind::Attributes => {
                        o.attributes
                            && o.attribute_filter.as_ref().is_none_or(|filter| {
                                record
                                    .attribute
                                    .as_ref()
                                    .is_some_and(|name| filter.contains(name))
                            })
                    }
                    Kind::CharacterData => o.character_data,
                };
            if !wanted {
                continue;
            }
            let old = match record.kind {
                Kind::Attributes => o.attribute_old_value,
                Kind::CharacterData => o.character_data_old_value,
                Kind::ChildList => false,
            };
            *interested.entry(r.observer).or_default() |= old;
        }
        for (observer, old) in interested {
            let mut given = record.clone();
            if !old {
                given.old = None;
            }
            let queue = self.queues.entry(observer).or_default();
            if queue.len() >= MAX_MUTATION_RECORDS {
                queue.pop_front();
            }
            queue.push_back(given);
            self.wake = true;
        }
    }
}

/// Records as JSON, for the realm to make objects of: each its type,
/// its nodes' handles and its strings, `null` where it has none.
pub(crate) fn json(records: &[Record]) -> String {
    use devtools::Json;
    let node = |node: Option<NodeId>| node.map_or(Json::Null, |n| Json::from(super::handle(n).0));
    let nodes = |list: &[NodeId]| Json::Array(list.iter().map(|&n| node(Some(n))).collect());
    let text = |text: &Option<String>| text.as_deref().map_or(Json::Null, Json::from);
    Json::Array(
        records
            .iter()
            .map(|r| {
                Json::object([
                    ("type", Json::from(r.kind.name())),
                    ("target", node(Some(r.target))),
                    ("added", nodes(&r.added)),
                    ("removed", nodes(&r.removed)),
                    ("previous", node(r.previous)),
                    ("next", node(r.next)),
                    ("attribute", text(&r.attribute)),
                    ("old", text(&r.old)),
                ])
            })
            .collect(),
    )
    .write()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dom::Document;

    fn all(subtree: bool) -> Options {
        Options {
            child_list: true,
            attributes: true,
            character_data: true,
            subtree,
            attribute_old_value: true,
            character_data_old_value: true,
            attribute_filter: None,
        }
    }

    #[test]
    fn an_operations_changes_to_one_parent_are_one_record_and_a_move_two() {
        let mut doc = Document::new();
        let root = doc.root();
        let body = doc.create_element("body").unwrap();
        doc.append_child(root, body).unwrap();
        let (a, b, c) = (
            doc.create_element("a").unwrap(),
            doc.create_element("b").unwrap(),
            doc.create_element("c").unwrap(),
        );
        doc.append_child(body, a).unwrap();
        let mut observers = MutationObservers::default();
        observers.observe(1, body, all(false));
        doc.observe(true);
        doc.set_watched(observers.watched());
        // `replaceChildren(b, c)`: one record.
        doc.remove_child(body, a).unwrap();
        doc.append_child(body, b).unwrap();
        doc.append_child(body, c).unwrap();
        observers.record(doc.take_observed(), true);
        // `appendChild(b)`, b already there: a move, two records.
        doc.append_child(body, b).unwrap();
        observers.record(doc.take_observed(), true);
        assert!(observers.take_wake());
        assert_eq!(observers.notifying(), [1]);
        let records = observers.take(1);
        assert_eq!(records.len(), 3);
        assert_eq!(
            (
                &records[0].added,
                &records[0].removed,
                records[0].previous,
                records[0].next
            ),
            (&vec![b, c], &vec![a], None, None)
        );
        assert_eq!(
            (&records[1].removed, records[1].previous, records[1].next),
            (&vec![b], None, Some(c))
        );
        assert_eq!(
            (&records[2].added, records[2].previous, records[2].next),
            (&vec![b], Some(c), None)
        );
        assert!(observers.take(1).is_empty());
    }

    #[test]
    fn a_change_reaches_the_observers_that_asked_for_it() {
        let mut doc = Document::new();
        let root = doc.root();
        let outer = doc.create_element("div").unwrap();
        let inner = doc.create_element("p").unwrap();
        let text = doc.create_text("t").unwrap();
        doc.append_child(root, outer).unwrap();
        doc.append_child(outer, inner).unwrap();
        doc.append_child(inner, text).unwrap();
        let mut observers = MutationObservers::default();
        // 1 sees the subtree with old values; 2 the outer node alone; 3
        // one attribute, without old values.
        observers.observe(1, outer, all(true));
        observers.observe(2, outer, all(false));
        observers.observe(
            3,
            outer,
            Options {
                attributes: true,
                subtree: true,
                attribute_filter: Some(vec!["title".to_owned()]),
                ..Options::default()
            },
        );
        doc.observe(true);
        doc.set_watched(observers.watched());
        doc.set_attribute(inner, "title", "x").unwrap();
        doc.set_attribute(inner, "id", "y").unwrap();
        doc.set_text(text, "u").unwrap();
        // Taken out, the paragraph is still seen by 1, through a
        // transient registration, until 1 is notified.
        doc.remove_child(outer, inner).unwrap();
        doc.set_attribute(inner, "title", "z").unwrap();
        observers.record(doc.take_observed(), false);
        let kinds = |records: Vec<Record>| {
            records
                .into_iter()
                .map(|r| format!("{} {:?} {:?}", r.kind.name(), r.attribute, r.old))
                .collect::<Vec<_>>()
        };
        assert_eq!(observers.notifying(), [1, 2, 3]);
        doc.set_watched(observers.watched());
        assert_eq!(
            kinds(observers.take(1)),
            [
                "attributes Some(\"title\") None",
                "attributes Some(\"id\") None",
                "characterData None Some(\"t\")",
                "childList None None",
                "attributes Some(\"title\") Some(\"x\")",
            ]
        );
        assert_eq!(kinds(observers.take(2)), ["childList None None"]);
        assert_eq!(
            kinds(observers.take(3)),
            [
                "attributes Some(\"title\") None",
                "attributes Some(\"title\") None"
            ]
        );
        // Observed again, an observer's transient registrations go.
        doc.append_child(outer, inner).unwrap();
        doc.remove_child(outer, inner).unwrap();
        observers.record(doc.take_observed(), false);
        observers.observe(1, outer, all(true));
        doc.set_watched(observers.watched());
        doc.set_attribute(inner, "title", "v").unwrap();
        observers.record(doc.take_observed(), false);
        assert!(observers.take(1).iter().all(|r| r.kind == Kind::ChildList));
        observers.notifying();
        doc.set_watched(observers.watched());
        // Notified, the transient registration is gone.
        doc.set_attribute(inner, "title", "w").unwrap();
        observers.record(doc.take_observed(), false);
        assert!(observers.take(1).is_empty());
        observers.disconnect(1);
        observers.disconnect(2);
        observers.disconnect(3);
        assert!(!observers.observing());
    }
}
