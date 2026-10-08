//! `@keyframes` rules (CSS Animations 1 §3, Animations 2 §3.2; M22):
//! their names, their keyframe selectors, and their keyframes processed
//! apart from any element.
//!
//! A rule's keyframe blocks are grouped by offset and by the
//! `animation-timing-function` each declares last, a block whose
//! selector lists several offsets joining a group for each, the groups
//! in the order they first appear and then sorted by offset, stably. A
//! group's declarations are its blocks' in order, so that the last of
//! a property's wins, as the cascade takes them; an `!important` one is
//! ignored (Animations 1 §3), as is one of a property that is not
//! animatable, `animation-timing-function` being the keyframe's easing.
//! The keyframes are kept as places of declarations in their sheet, as
//! the rule set keeps its style rules; an element's animation computes
//! their values when it is sampled, against the element (Web Animations
//! §5.3.3). The implicit keyframes at 0% and 100% are its too: a
//! property no keyframe at an end names takes the element's underlying
//! value there.
//!
//! A keyframe's `animation-timing-function` is read apart from any
//! element, so one that substitutes `var()` is not taken and the
//! keyframe takes the animation's easing.

use std::sync::Arc;

use css::{Rule, Token};

use crate::animate::{Longhand, longhands_of};
use crate::cascade::{canonical_name, keyframe_easing, supports};
use crate::custom::{is_custom_ident, is_custom_property_name, is_valid_custom_declaration};
use crate::easing::Easing;
use crate::media::{split_commas, trim};

/// A keyframes rule's keyframes, processed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Keyframes {
    /// The keyframes in the order an animation uses them: by offset,
    /// those of one offset in the order their groups first appear.
    pub(crate) keyframes: Vec<Keyframe>,
    /// The longhands any keyframe sets: the properties the animation
    /// animates, sorted.
    pub(crate) longhands: Vec<Longhand>,
    /// The custom properties any keyframe sets, sorted.
    pub(crate) custom: Vec<Arc<str>>,
}

/// One keyframe.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Keyframe {
    /// Its offset, in [0, 1].
    pub(crate) offset: f64,
    /// Its `animation-timing-function`; `None` for the animation's own.
    pub(crate) easing: Option<Easing>,
    /// Its declarations, in order, as places in the rule: the keyframe
    /// block's index among the rule's rules, and the declaration's
    /// among the block's.
    pub(crate) declarations: Vec<(u32, u32)>,
    /// The longhands it sets, sorted.
    pub(crate) longhands: Vec<Longhand>,
    /// The custom properties it sets, sorted.
    pub(crate) custom: Vec<Arc<str>>,
}

impl Keyframe {
    /// Whether it sets `longhand`.
    pub(crate) fn sets(&self, longhand: Longhand) -> bool {
        self.longhands.binary_search(&longhand).is_ok()
    }

    /// Whether it sets the custom property `name`.
    pub(crate) fn sets_custom(&self, name: &str) -> bool {
        self.custom.binary_search_by(|n| (**n).cmp(name)).is_ok()
    }
}

/// A `<keyframes-name>`: a `<custom-ident>` other than `none`, or a
/// string, compared as written.
pub(crate) fn keyframes_name(prelude: &[Token]) -> Option<Arc<str>> {
    let [token] = trim(prelude) else {
        return None;
    };
    match token {
        Token::Ident(name) if is_custom_ident(name) && !name.eq_ignore_ascii_case("none") => {
            Some(Arc::from(name.as_str()))
        }
        Token::String { value, bad: false } => Some(Arc::from(value.as_str())),
        _ => None,
    }
}

/// A keyframe selector list's offsets: each `from`, `to` or a
/// percentage from 0% to 100%; `None` when any is not, which drops the
/// block (Animations 1 §3).
pub(crate) fn offsets(prelude: &[Token]) -> Option<Vec<f64>> {
    split_commas(prelude)
        .into_iter()
        .map(|selector| match trim(selector) {
            [Token::Ident(word)] if word.eq_ignore_ascii_case("from") => Some(0.0),
            [Token::Ident(word)] if word.eq_ignore_ascii_case("to") => Some(1.0),
            [Token::Percentage { value }] if (0.0..=100.0).contains(value) => Some(value / 100.0),
            _ => None,
        })
        .collect()
}

/// The animatable longhands a declaration of `name` sets, the name as
/// written in the keyframe; `None` for a custom property.
fn animated(name: &str) -> Vec<Longhand> {
    longhands_of(canonical_name(&name.to_ascii_lowercase()))
        .into_iter()
        .filter(|longhand| longhand.is_animatable())
        .collect()
}

/// Keyframe blocks of one offset and easing, taken together: the offset,
/// the easing, and their declarations, as places in the rule.
type Group = (f64, Option<Easing>, Vec<(u32, u32)>);

/// Processes a keyframes rule's blocks (Animations 2 §3.2).
pub(crate) fn process(blocks: &[Rule]) -> Keyframes {
    let mut groups: Vec<Group> = Vec::new();
    for (b, rule) in blocks.iter().enumerate() {
        let Rule::Qualified(block) = rule else {
            continue;
        };
        let Some(offsets) = offsets(&block.prelude) else {
            continue;
        };
        // The last declaration valid where it is parsed wins, as in a
        // style rule; one that does not parse is dropped.
        let easing = block
            .declarations
            .iter()
            .rev()
            .find(|d| {
                !d.important
                    && d.name.eq_ignore_ascii_case("animation-timing-function")
                    && supports("animation-timing-function", &d.value)
            })
            .and_then(|d| keyframe_easing(&d.value));
        let declarations: Vec<(u32, u32)> = block
            .declarations
            .iter()
            .enumerate()
            .filter(|(_, d)| {
                !d.important
                    && if is_custom_property_name(&d.name) {
                        is_valid_custom_declaration(&d.value)
                    } else {
                        !animated(&d.name).is_empty() && supports(&d.name, &d.value)
                    }
            })
            .map(|(i, _)| (b as u32, i as u32))
            .collect();
        for offset in offsets {
            match groups
                .iter_mut()
                .find(|(at, eased, _)| *at == offset && *eased == easing)
            {
                Some((_, _, list)) => list.extend_from_slice(&declarations),
                None => groups.push((offset, easing.clone(), declarations.clone())),
            }
        }
    }
    // By offset, stably: the groups of one offset keep their order.
    groups.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut all_longhands = Vec::new();
    let mut all_custom: Vec<Arc<str>> = Vec::new();
    let keyframes = groups
        .into_iter()
        .map(|(offset, easing, declarations)| {
            let mut longhands = Vec::new();
            let mut custom: Vec<Arc<str>> = Vec::new();
            for &(b, i) in &declarations {
                let Rule::Qualified(block) = &blocks[b as usize] else {
                    unreachable!("a keyframe's declarations are a block's");
                };
                let name = &block.declarations[i as usize].name;
                if is_custom_property_name(name) {
                    custom.push(Arc::from(name.as_str()));
                } else {
                    longhands.extend(animated(name));
                }
            }
            longhands.sort_unstable();
            longhands.dedup();
            custom.sort_unstable();
            custom.dedup();
            all_longhands.extend_from_slice(&longhands);
            all_custom.extend_from_slice(&custom);
            Keyframe {
                offset,
                easing,
                declarations,
                longhands,
                custom,
            }
        })
        .collect();
    all_longhands.sort_unstable();
    all_longhands.dedup();
    all_custom.sort_unstable();
    all_custom.dedup();
    Keyframes {
        keyframes,
        longhands: all_longhands,
        custom: all_custom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(source: &str) -> Vec<Rule> {
        let sheet = css::parse_stylesheet(&format!("@keyframes k {{ {source} }}")).unwrap();
        match sheet.rules.into_iter().next() {
            Some(Rule::At(at)) => match at.block {
                Some(css::Block::Rules(rules)) => rules,
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn names_and_selectors_parse_as_the_draft_has_them() {
        let name = |source: &str| keyframes_name(&css::tokenize(source)).map(|n| n.to_string());
        assert_eq!(name("spin"), Some("spin".into()));
        assert_eq!(name(" Spin "), Some("Spin".into()));
        assert_eq!(name("\"a b\""), Some("a b".into()));
        assert_eq!(name("\"none\""), Some("none".into()));
        for bad in ["none", "NONE", "initial", "default", "a b", "1", ""] {
            assert_eq!(name(bad), None, "{bad}");
        }
        let offsets = |source: &str| offsets(&css::tokenize(source));
        assert_eq!(offsets("from"), Some(vec![0.0]));
        assert_eq!(offsets("TO, 50%, 0%"), Some(vec![1.0, 0.5, 0.0]));
        assert_eq!(offsets("12.5%"), Some(vec![0.125]));
        for bad in ["0", "101%", "-1%", "50% 60%", "50%,", "middle", ""] {
            assert_eq!(offsets(bad), None, "{bad}");
        }
    }

    #[test]
    fn keyframes_group_by_offset_and_easing_and_sort_stably() {
        let k = process(&blocks(
            "to { opacity: 1 } \
             50% { width: 110px } \
             from { opacity: 0; animation-timing-function: linear } \
             50% { height: 7px } \
             25%, 75% { color: red !important; transform: none } \
             50% { animation-timing-function: steps(2); margin: 1px } \
             bad { opacity: 0.3 } \
             60% { animation-name: x; transition: none; will-change: auto; --v: 1 }",
        ));
        let summary: Vec<_> = k
            .keyframes
            .iter()
            .map(|f| (f.offset, f.easing.clone(), f.declarations.clone()))
            .collect();
        assert_eq!(
            summary,
            vec![
                (0.0, Some(Easing::Linear), vec![(2, 0)]),
                (0.25, None, vec![(4, 1)]),
                // The two 50% blocks without an easing are one keyframe;
                // the one with `steps(2)` another, after them.
                (0.5, None, vec![(1, 0), (3, 0)]),
                (
                    0.5,
                    Some(Easing::Steps {
                        count: 2,
                        position: crate::easing::StepPosition::JumpEnd
                    }),
                    vec![(5, 1)]
                ),
                // Only the custom property: the rest are not animatable.
                (0.6, None, vec![(7, 3)]),
                (0.75, None, vec![(4, 1)]),
                (1.0, None, vec![(0, 0)]),
            ]
        );
        assert_eq!(
            k.keyframes[2].longhands,
            vec![Longhand::Width, Longhand::Height]
        );
        assert!(k.keyframes[3].sets(Longhand::MarginLeft));
        assert!(k.keyframes[4].sets_custom("--v"));
        assert!(!k.keyframes[4].sets_custom("--w"));
        // The animated properties: `color` was important and is not.
        assert!(!k.longhands.contains(&Longhand::Color));
        assert!(k.longhands.contains(&Longhand::Opacity));
        assert!(k.longhands.contains(&Longhand::Transform));
        assert_eq!(k.custom, vec![Arc::<str>::from("--v")]);
    }

    #[test]
    fn declarations_that_do_not_parse_set_nothing() {
        let k = process(&blocks(
            "from { width: bogus; opacity: 0.5; --x: ) } \
             50% { animation-timing-function: steps(2); animation-timing-function: bogus } \
             to { animation-timing-function: linear; animation-timing-function: var(--e) }",
        ));
        assert_eq!(k.keyframes[0].longhands, vec![Longhand::Opacity]);
        assert!(k.keyframes[0].custom.is_empty());
        // The last easing that parses wins; one that substitutes cannot
        // be read apart from an element and leaves the animation's.
        assert_eq!(
            k.keyframes[1].easing,
            Some(Easing::Steps {
                count: 2,
                position: crate::easing::StepPosition::JumpEnd
            })
        );
        assert_eq!(k.keyframes[2].easing, None);
    }

    #[test]
    fn aliases_and_shorthands_set_their_longhands() {
        let k = process(&blocks(
            "from { -webkit-transform: none; border: 1px solid red; Opacity: 0 }",
        ));
        let longhands = &k.keyframes[0].longhands;
        assert!(longhands.contains(&Longhand::Transform));
        assert!(longhands.contains(&Longhand::Opacity));
        assert!(longhands.contains(&Longhand::BorderLeftColor));
        assert!(longhands.contains(&Longhand::BorderTopStyle));
        assert_eq!(longhands.len(), 14);
    }
}
