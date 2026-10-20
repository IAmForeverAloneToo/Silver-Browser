//! IndexedDB's key ranges (section 2.9): a lower and an upper bound,
//! each open or closed, or none.

use std::ops::Bound;

use super::key::Key;

/// A key range.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyRange {
    pub lower: Option<(Key, bool)>,
    pub upper: Option<(Key, bool)>,
}

impl KeyRange {
    /// The range of `key` alone.
    pub fn only(key: Key) -> KeyRange {
        KeyRange {
            lower: Some((key.clone(), false)),
            upper: Some((key, false)),
        }
    }

    /// Every key.
    pub fn all() -> KeyRange {
        KeyRange::default()
    }

    /// Whether `key` is in the range (section 2.9's "in").
    pub fn contains(&self, key: &Key) -> bool {
        let above = match &self.lower {
            None => true,
            Some((lower, open)) => {
                if *open {
                    key > lower
                } else {
                    key >= lower
                }
            }
        };
        let below = match &self.upper {
            None => true,
            Some((upper, open)) => {
                if *open {
                    key < upper
                } else {
                    key <= upper
                }
            }
        };
        above && below
    }

    /// The range as bounds of encoded keys: a table's range.
    pub fn bounds(&self) -> (Bound<Vec<u8>>, Bound<Vec<u8>>) {
        let bound = |end: &Option<(Key, bool)>| match end {
            None => Bound::Unbounded,
            Some((key, true)) => Bound::Excluded(key.encode()),
            Some((key, false)) => Bound::Included(key.encode()),
        };
        (bound(&self.lower), bound(&self.upper))
    }

    /// Whether the range is one a script may make: its lower bound not
    /// above its upper, nor equal to it with either open.
    pub fn is_valid(&self) -> bool {
        match (&self.lower, &self.upper) {
            (Some((lower, lower_open)), Some((upper, upper_open))) => {
                lower < upper || (lower == upper && !lower_open && !upper_open)
            }
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_holds_what_its_bounds_say() {
        let n = |n: f64| Key::Number(n);
        let closed = KeyRange {
            lower: Some((n(1.0), false)),
            upper: Some((n(3.0), false)),
        };
        assert!(closed.contains(&n(1.0)) && closed.contains(&n(3.0)));
        assert!(!closed.contains(&n(0.5)) && !closed.contains(&Key::string("a")));
        let open = KeyRange {
            lower: Some((n(1.0), true)),
            upper: Some((n(3.0), true)),
        };
        assert!(!open.contains(&n(1.0)) && !open.contains(&n(3.0)) && open.contains(&n(2.0)));
        assert!(KeyRange::all().contains(&Key::Array(vec![])));
        assert!(KeyRange::only(n(2.0)).contains(&n(2.0)));
        assert!(!KeyRange::only(n(2.0)).contains(&n(2.5)));
        assert!(closed.is_valid() && KeyRange::only(n(1.0)).is_valid());
        let empty = KeyRange {
            lower: Some((n(1.0), true)),
            upper: Some((n(1.0), false)),
        };
        assert!(!empty.is_valid());
        let backwards = KeyRange {
            lower: Some((n(2.0), false)),
            upper: Some((n(1.0), false)),
        };
        assert!(!backwards.is_valid());
    }
}
