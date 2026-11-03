//! The keywords that roll the cascade back (Cascade 5 sections 7.3.3 to
//! 7.3.5, D73): `revert`, which takes a property's value from the
//! origins below its declaration's, `revert-layer`, from the layers
//! below its declaration's, the unlayered declarations being their
//! origin's last layer, and `revert-rule`, from the declarations below
//! its rule's in the cascade's order, as if its rule were not there.
//!
//! [`super::compute_style`] applies an element's declarations sorted into
//! cascade order, each pass of it a later declaration overwriting an
//! earlier one. What the cascade gives with a declaration's origin, its
//! layer or its rule, and every one above, taken away is then what a
//! pass had made of the declarations before the first of them: a revert
//! keyword takes its property's value from the pass's state there, kept
//! as the pass goes. Where that is, a [`Mark`], depends on the
//! declaration's place in the cascade's order and its rule's alone; the
//! marks of an element's revert keywords are found before the passes,
//! and each pass keeps its state at them, nothing at all where the
//! element has none. The user agent's origin has none below it: a
//! `revert` there rolls back to no declaration, which is `unset`. The
//! animations' values, between the normal declarations and the
//! important ones, are kept by the mark there that is after them: a
//! layer or a rule of important declarations rolls back to them, an
//! origin past them. A keyframe's revert keyword takes the value of the
//! user agent's declarations, the animations' origin rolling back as the
//! author's does.

use std::cmp::Reverse;

use css::Token;

use super::{Applied, TierBase, single_ident, tier};

/// A keyword that rolls the cascade back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Revert {
    /// `revert`: to the origin below.
    Origin,
    /// `revert-layer`: to the layer below.
    Layer,
    /// `revert-rule`: to the rule below.
    Rule,
}

/// The revert keyword `tokens` are, alone but for white space.
pub(crate) fn revert_keyword(tokens: &[Token]) -> Option<Revert> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "revert" => Some(Revert::Origin),
        "revert-layer" => Some(Revert::Layer),
        "revert-rule" => Some(Revert::Rule),
        _ => None,
    }
}

/// A place in a pass over the sorted declarations: before the one at
/// `at`, and, where the animations' values apply, before or after them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Mark {
    at: usize,
    animated: bool,
}

/// The marks an element's revert keywords roll back to, sorted.
pub(super) struct Rollbacks {
    marks: Vec<Mark>,
    /// Where the important declarations begin, before which the
    /// animations' values apply.
    important: usize,
}

impl Rollbacks {
    /// The marks of the revert keywords among `sorted`, an element's
    /// declarations in cascade order whose important ones begin at
    /// `important`: of each declaration that is one, or that may become
    /// one once its substitution functions are replaced; and the user
    /// agent's when `keyframes` says a keyframe the element's animations
    /// read is one.
    pub(super) fn of(sorted: &[&Applied<'_>], important: usize, keyframes: bool) -> Rollbacks {
        let mut rollbacks = Rollbacks {
            marks: Vec::new(),
            important,
        };
        let mut marks = Vec::new();
        for (n, a) in sorted.iter().enumerate() {
            if revert_keyword(&a.decl.value).is_some()
                || crate::custom::has_substitution(&a.decl.value)
            {
                for keyword in [Revert::Origin, Revert::Layer, Revert::Rule] {
                    marks.push(rollbacks.mark(sorted, n, keyword));
                }
            }
        }
        if keyframes {
            marks.push(rollbacks.user_agent(sorted));
        }
        marks.sort_unstable();
        marks.dedup();
        rollbacks.marks = marks;
        rollbacks
    }

    /// Whether no revert keyword is there to roll back: no pass keeps
    /// anything.
    pub(super) fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// Where the cascade rolls back to for `keyword` in `sorted[n]`.
    pub(super) fn mark(&self, sorted: &[&Applied<'_>], n: usize, keyword: Revert) -> Mark {
        let a = sorted[n];
        let user_agent = [
            tier(TierBase::UserAgent, false),
            tier(TierBase::UserAgent, true),
        ];
        let (at, animated) = match keyword {
            // Nothing is below the user agent's origin, nor the layers of
            // its important declarations, whose next lower are another
            // origin's.
            _ if a.tier == user_agent[1] => (0, false),
            Revert::Origin if a.tier == user_agent[0] => (0, false),
            Revert::Origin => return self.user_agent(sorted),
            Revert::Layer => (
                sorted.partition_point(|b| (b.tier, b.layer) < (a.tier, a.layer)),
                a.tier >= tier(TierBase::AuthorSheet, true),
            ),
            // The declarations of a rule are together in the cascade's
            // order, its first at the rule's own place.
            Revert::Rule => (
                sorted.partition_point(|b| {
                    (
                        b.tier,
                        b.layer,
                        b.specificity,
                        Reverse(b.proximity),
                        b.order,
                    ) < (a.tier, a.layer, a.specificity, Reverse(a.proximity), a.rule)
                }),
                a.tier >= tier(TierBase::AuthorSheet, true),
            ),
        };
        Mark {
            at,
            animated: animated && at == self.important,
        }
    }

    /// The mark of the user agent's normal declarations alone: what an
    /// author's or an animation's `revert` rolls back to.
    pub(super) fn user_agent(&self, sorted: &[&Applied<'_>]) -> Mark {
        Mark {
            at: sorted.partition_point(|b| b.tier < tier(TierBase::AuthorSheet, false)),
            animated: false,
        }
    }

    /// Keeps `state`, a pass's at `at`, after the animations' values
    /// there when `animated`, in `kept`, if a revert keyword rolls back
    /// there. A pass that applies no animation keeps it as both.
    pub(super) fn keep<T: Clone>(
        &self,
        kept: &mut Vec<(Mark, T)>,
        at: usize,
        animated: Option<bool>,
        state: &T,
    ) {
        if self.marks.is_empty() {
            return;
        }
        let sides: &[bool] = match animated {
            Some(false) => &[false],
            Some(true) => &[true],
            None => &[false, true],
        };
        for &animated in sides {
            let mark = Mark {
                at,
                animated: animated && at == self.important,
            };
            if self.marks.binary_search(&mark).is_ok() && !kept.iter().any(|(m, _)| *m == mark) {
                kept.push((mark, state.clone()));
            }
        }
    }

    /// Where a declaration's mark is in a pass's `kept`, if there.
    pub(super) fn kept<T>(kept: &[(Mark, T)], mark: Mark) -> Option<&T> {
        kept.iter()
            .find(|(m, _)| *m == mark)
            .map(|(_, state)| state)
    }

    /// The place in the order of declarations and animations of the
    /// declaration at `n`, for the custom properties' pass, which puts
    /// the animations' declarations before the important ones: those
    /// before `mark` count and those from it do not.
    pub(super) fn counts_before(&self, mark: Mark, place: (usize, bool)) -> bool {
        // An animation's declaration stands at the important ones' start,
        // before the first of them.
        let (at, animation) = place;
        match animation {
            true => at < mark.at || (at == mark.at && mark.animated),
            false => at < mark.at,
        }
    }
}
