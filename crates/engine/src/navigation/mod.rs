//! The session history (docs/design/navigation.md, "The session
//! history"): the entries of the engine's one navigable, the current
//! one, and the documents the back-forward cache keeps for them.
//!
//! An entry names its document by its document state, which entries a
//! fragment navigation made from one another share, as HTML's document
//! state is shared. The document of the current entry's state is the
//! one shown, which the engine holds; the cache holds others, whole, for
//! states some entry still names. Which document a state shows is the
//! engine's to say: this module keeps the list, its limits and the
//! cache's.

use net::Url;

/// The most entries a history holds: past it, the oldest goes, as in
/// Chromium and Firefox.
pub(crate) const MAX_ENTRIES: usize = 50;

/// The most documents the back-forward cache keeps besides the one
/// shown, as Chromium's does by default: past it, the one shown least
/// recently goes.
pub(crate) const MAX_KEPT: usize = 6;

/// Which document state an entry has.
pub(crate) type StateId = u64;

/// One entry of the session history.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entry {
    /// Its URL, fragment included; none for a document given to the
    /// engine as source or as a tree.
    pub(crate) url: Option<Url>,
    pub(crate) state: StateId,
    /// The viewport's scroll offset, saved when the entry stopped being
    /// the current one (HTML's "save persisted state").
    pub(crate) scroll: f32,
    /// Whether a form's POST made its document (M26.9): a reload or a
    /// traversal does not send it again.
    pub(crate) posted: bool,
}

/// The session history, and the back-forward cache of `D`s, the
/// documents with what was made from them.
pub(crate) struct History<D> {
    entries: Vec<Entry>,
    current: usize,
    next_state: StateId,
    /// The documents kept, by state, the one shown least recently first.
    kept: Vec<(StateId, D)>,
}

impl<D> History<D> {
    /// A history of one entry, for `url`, with a new state.
    pub(crate) fn new(url: Option<Url>) -> History<D> {
        History {
            entries: vec![Entry {
                url,
                state: 0,
                scroll: 0.0,
                posted: false,
            }],
            current: 0,
            next_state: 1,
            kept: Vec::new(),
        }
    }

    /// The current entry.
    pub(crate) fn current(&self) -> &Entry {
        &self.entries[self.current]
    }

    pub(crate) fn current_mut(&mut self) -> &mut Entry {
        &mut self.entries[self.current]
    }

    #[cfg(test)]
    pub(crate) fn current_index(&self) -> usize {
        self.current
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// The entry `delta` away from the current one, and its index, if
    /// the history reaches that far.
    pub(crate) fn by_delta(&self, delta: isize) -> Option<(usize, &Entry)> {
        let index = self.current.checked_add_signed(delta)?;
        self.entries.get(index).map(|entry| (index, entry))
    }

    pub(crate) fn entry_mut(&mut self, index: usize) -> &mut Entry {
        &mut self.entries[index]
    }

    /// A document state no entry has had.
    pub(crate) fn new_state(&mut self) -> StateId {
        let state = self.next_state;
        self.next_state += 1;
        state
    }

    /// Adds an entry after the current one, which it becomes: the
    /// entries after the current one go first, and the oldest when the
    /// history is full. Documents kept for states no entry has any more
    /// go with them.
    pub(crate) fn push(&mut self, url: Option<Url>, state: StateId) {
        self.entries.truncate(self.current + 1);
        self.entries.push(Entry {
            url,
            state,
            scroll: 0.0,
            posted: false,
        });
        if self.entries.len() > MAX_ENTRIES {
            let over = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(..over);
        }
        self.current = self.entries.len() - 1;
        self.prune();
    }

    /// Puts a new entry in the current one's place.
    pub(crate) fn replace(&mut self, url: Option<Url>, state: StateId) {
        self.entries[self.current] = Entry {
            url,
            state,
            scroll: 0.0,
            posted: false,
        };
        self.prune();
    }

    /// Makes the entry at `index` the current one.
    pub(crate) fn go_to(&mut self, index: usize) {
        assert!(index < self.entries.len(), "an entry of the history");
        self.current = index;
    }

    pub(crate) fn can_go_back(&self) -> bool {
        self.current > 0
    }

    pub(crate) fn can_go_forward(&self) -> bool {
        self.current + 1 < self.entries.len()
    }

    /// Whether an entry has `state`.
    pub(crate) fn names(&self, state: StateId) -> bool {
        self.entries.iter().any(|entry| entry.state == state)
    }

    /// Keeps `document` for `state`, as the one shown most recently,
    /// when an entry has that state; the one shown least recently goes
    /// when the cache is full. Returns the documents that went, for the
    /// engine to drop.
    pub(crate) fn keep(&mut self, state: StateId, document: D) -> Vec<D> {
        if !self.names(state) {
            return vec![document];
        }
        let mut gone: Vec<D> = Vec::new();
        if let Some(at) = self.kept.iter().position(|(kept, _)| *kept == state) {
            gone.push(self.kept.remove(at).1);
        }
        self.kept.push((state, document));
        gone.extend(self.prune());
        gone
    }

    /// Takes the document kept for `state`, if there is one.
    pub(crate) fn take_kept(&mut self, state: StateId) -> Option<D> {
        let at = self.kept.iter().position(|(kept, _)| *kept == state)?;
        Some(self.kept.remove(at).1)
    }

    /// How many documents the cache keeps.
    #[cfg(test)]
    pub(crate) fn kept_len(&self) -> usize {
        self.kept.len()
    }

    /// Whether a document is kept for `state`.
    #[cfg(test)]
    pub(crate) fn is_kept(&self, state: StateId) -> bool {
        self.kept.iter().any(|(kept, _)| *kept == state)
    }

    /// Drops the documents kept for states no entry has, and the ones
    /// shown least recently past [`MAX_KEPT`]; returns them.
    fn prune(&mut self) -> Vec<D> {
        let mut gone = Vec::new();
        let mut kept = Vec::with_capacity(self.kept.len());
        for (state, document) in self.kept.drain(..) {
            if self.entries.iter().any(|entry| entry.state == state) {
                kept.push((state, document));
            } else {
                gone.push(document);
            }
        }
        if kept.len() > MAX_KEPT {
            let over = kept.len() - MAX_KEPT;
            gone.extend(kept.drain(..over).map(|(_, document)| document));
        }
        self.kept = kept;
        gone
    }
}

#[cfg(test)]
mod tests;
