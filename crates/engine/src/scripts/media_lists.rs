//! `matchMedia()`'s lists (gap batch A): each parsed once and kept by a
//! number of the document's while its `MediaQueryList` lives, answered
//! against the window's medium when asked and evaluated again in each
//! frame's update, which reports those whose answer changed.

use std::collections::BTreeMap;

use style::{Media, MediaQueryList};

/// The most lists a document keeps; past it the oldest is dropped, and
/// reports no change again.
pub(crate) const MAX_MEDIA_LISTS: usize = 1_000;

/// A document's lists, by their numbers, each with its last answer.
#[derive(Default)]
pub(crate) struct MediaLists {
    lists: BTreeMap<u32, (MediaQueryList, bool)>,
    next: u32,
}

impl MediaLists {
    /// `matchMedia(query)`: the list's serialization, whether it holds
    /// for `media` now (its last answer), and its number.
    pub(crate) fn make(&mut self, query: &str, media: &Media) -> (String, bool, u32) {
        let tokens = css::tokenize(query);
        let list = style::parse_media_query_list(&tokens);
        let matches = list.evaluate(media);
        if self.lists.len() >= MAX_MEDIA_LISTS {
            self.lists.pop_first();
        }
        self.next = self.next.wrapping_add(1);
        self.lists.insert(self.next, (list, matches));
        (
            style::serialize_media_query_list(&tokens),
            matches,
            self.next,
        )
    }

    /// Whether list `id` holds for `media` now; false for one dropped.
    pub(crate) fn matches(&self, id: u32, media: &Media) -> bool {
        self.lists
            .get(&id)
            .is_some_and(|(list, _)| list.evaluate(media))
    }

    /// Drops list `id`, its object gone.
    pub(crate) fn forget(&mut self, id: u32) {
        self.lists.remove(&id);
    }

    /// Whether there is any list to evaluate.
    pub(crate) fn is_empty(&self) -> bool {
        self.lists.is_empty()
    }

    /// HTML's "evaluate media queries and report changes": the lists
    /// whose answer for `media` is not their last, in the order they
    /// were made, each with its new answer kept.
    pub(crate) fn changed(&mut self, media: &Media) -> Vec<u32> {
        let mut changed = Vec::new();
        for (&id, (list, last)) in &mut self.lists {
            let now = list.evaluate(media);
            if now != *last {
                *last = now;
                changed.push(id);
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_reports_each_change_of_its_answer_once() {
        let mut lists = MediaLists::default();
        let narrow = Media::screen(400.0, 300.0);
        let wide = Media::screen(900.0, 300.0);
        let (media, matches, id) = lists.make("(MIN-WIDTH:500px)", &narrow);
        assert_eq!((media.as_str(), matches), ("(min-width: 500px)", false));
        let (_, _, other) = lists.make("print", &narrow);
        assert!(lists.matches(id, &wide));
        assert_eq!(lists.changed(&wide), [id]);
        assert_eq!(lists.changed(&wide), Vec::<u32>::new());
        assert_eq!(lists.changed(&narrow), [id]);
        lists.forget(id);
        assert!(!lists.matches(id, &wide));
        assert!(lists.changed(&wide).is_empty());
        lists.forget(other);
        assert!(lists.is_empty());
        for _ in 0..=MAX_MEDIA_LISTS {
            lists.make("all", &narrow);
        }
        assert_eq!(lists.lists.len(), MAX_MEDIA_LISTS);
    }
}
