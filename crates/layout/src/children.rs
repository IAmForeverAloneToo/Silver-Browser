//! A fragment's children (M35.9): shared between layouts, so that the
//! layout cache hands a subtree back, and keeps one, without copying
//! it, and copied on write a level at a time when a fragment under them
//! is changed. No children are no allocation.
//!
//! The boxes' places in tree order (M17) move when a box is inserted
//! or removed before them, which leaves their layout as it was: a
//! subtree handed back is moved along by [`Children::shift`], which
//! applies to every fragment under the list, rather than by a copy of
//! the subtree with every place moved.

use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use crate::block::Fragment;

/// The fragments of a box's content, in tree order: read as a `Vec`,
/// and written as one after the list is made the fragment's own
/// (`Arc::make_mut`), the fragments in it copied but not theirs.
#[derive(Clone, Default)]
pub struct Children {
    list: Option<Arc<Vec<Fragment>>>,
    shift: u32,
}

/// The list of no children.
static NONE: Vec<Fragment> = Vec::new();

impl Children {
    /// None.
    pub fn new() -> Children {
        Children::default()
    }

    /// How far the places in tree order of every fragment under the
    /// list moved since they were laid out: what their own say is that
    /// much behind, wrapping.
    pub fn shift(&self) -> u32 {
        self.shift
    }

    /// Moves the places in tree order of every fragment under the list
    /// `by` along, wrapping.
    pub fn move_along(&mut self, by: u32) {
        self.shift = self.shift.wrapping_add(by);
    }

    /// The children to write, as a slice: none is not allocated for.
    pub fn as_mut_slice(&mut self) -> &mut [Fragment] {
        match &mut self.list {
            None => &mut [],
            Some(list) => Arc::make_mut(list).as_mut_slice(),
        }
    }

    /// The children to write, one by one.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Fragment> {
        self.as_mut_slice().iter_mut()
    }

    /// Whether `a` and `b` are the same list, shared: what a subtree
    /// handed back whole is.
    pub fn ptr_eq(a: &Children, b: &Children) -> bool {
        match (&a.list, &b.list) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }

    /// The shared list, when there is one: what tells one list from
    /// another while each is alive.
    pub(crate) fn shared(&self) -> Option<&Arc<Vec<Fragment>>> {
        self.list.as_ref()
    }

    /// Puts `list` in place of the children, their shift kept: a copy
    /// with a change written.
    pub(crate) fn share(&mut self, list: Arc<Vec<Fragment>>) {
        self.list = Some(list);
    }
}

impl Deref for Children {
    type Target = Vec<Fragment>;

    fn deref(&self) -> &Vec<Fragment> {
        self.list.as_deref().unwrap_or(&NONE)
    }
}

impl DerefMut for Children {
    fn deref_mut(&mut self) -> &mut Vec<Fragment> {
        Arc::make_mut(self.list.get_or_insert_with(Arc::default))
    }
}

impl From<Vec<Fragment>> for Children {
    fn from(list: Vec<Fragment>) -> Children {
        Children {
            list: (!list.is_empty()).then(|| Arc::new(list)),
            shift: 0,
        }
    }
}

impl FromIterator<Fragment> for Children {
    fn from_iter<I: IntoIterator<Item = Fragment>>(iter: I) -> Children {
        Children::from(iter.into_iter().collect::<Vec<_>>())
    }
}

impl IntoIterator for Children {
    type Item = Fragment;
    type IntoIter = std::vec::IntoIter<Fragment>;

    /// The children by value: the list itself when it is no one
    /// else's, else a copy; the shift is left to the caller.
    fn into_iter(self) -> Self::IntoIter {
        self.list
            .map(Arc::unwrap_or_clone)
            .unwrap_or_default()
            .into_iter()
    }
}

impl<'a> IntoIterator for &'a Children {
    type Item = &'a Fragment;
    type IntoIter = std::slice::Iter<'a, Fragment>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a mut Children {
    type Item = &'a mut Fragment;
    type IntoIter = std::slice::IterMut<'a, Fragment>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl fmt::Debug for Children {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(x: f32) -> Fragment {
        crate::testing::fragment(x, 0.0, Vec::new())
    }

    #[test]
    fn none_is_no_allocation_and_a_shared_list_is_copied_on_write() {
        let mut none = Children::new();
        assert!(none.is_empty());
        assert!(none.iter_mut().next().is_none());
        assert!(none.shared().is_none(), "reading nor iterating allocates");
        assert!(Children::from(Vec::new()).shared().is_none());
        let kept: Children = vec![leaf(1.0), leaf(2.0)].into();
        let mut taken = kept.clone();
        assert!(Children::ptr_eq(&kept, &taken), "a clone shares");
        taken[0].x = 5.0;
        assert!(!Children::ptr_eq(&kept, &taken), "written, it is its own");
        assert_eq!((kept[0].x, taken[0].x), (1.0, 5.0));
        // Its own list is written in place.
        let before = Arc::as_ptr(taken.shared().unwrap());
        taken.push(leaf(3.0));
        taken.iter_mut().for_each(|f| f.y = 1.0);
        assert_eq!(Arc::as_ptr(taken.shared().unwrap()), before);
        let xs: Vec<f32> = taken.into_iter().map(|f| f.x).collect();
        assert_eq!(xs, [5.0, 2.0, 3.0]);
    }
}
