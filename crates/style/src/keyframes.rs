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
//! keyframe takes the animation's easing; so is its
//! `animation-composition` (Animations 2), its composite operation, and
//! blocks of one offset and easing but different operations are
//! different keyframes.
//!
//! A script's keyframes (Web Animations 1 section 6.6, D75) are kept as
//! [`ScriptKeyframes`]: the keyframes as the script gave them, which
//! `getKeyframes` reads back, each a block of its valid declarations;
//! and the same processed as an `@keyframes` rule's are, in the order
//! given (they are not sorted), with their computed offsets.

use std::sync::Arc;

use css::{Rule, Token};

use crate::animate::{Longhand, longhands_of};
use crate::cascade::{canonical_name, easings_apart, keyframe_easing, supports};
use crate::computed::Composite;
use crate::custom::{
    has_substitution, is_custom_ident, is_custom_property_name, is_valid_custom_declaration,
};
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
    /// Its composite operation; `None` for the effect's own.
    pub(crate) composite: Option<Composite>,
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

/// Keyframe blocks of one offset, easing and composite operation, taken
/// together: the offset, the easing, the operation, and their
/// declarations, as places in the rule.
type Group = (f64, Option<Easing>, Option<Composite>, Vec<(u32, u32)>);

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
        let composite = block
            .declarations
            .iter()
            .rev()
            .filter(|d| !d.important && d.name.eq_ignore_ascii_case("animation-composition"))
            .find_map(|d| match trim(&d.value) {
                [Token::Ident(word)] => Composite::from_name(word),
                _ => None,
            });
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
            match groups.iter_mut().find(|(at, eased, composited, _)| {
                *at == offset && *eased == easing && *composited == composite
            }) {
                Some((_, _, _, list)) => list.extend_from_slice(&declarations),
                None => groups.push((offset, easing.clone(), composite, declarations.clone())),
            }
        }
    }
    // By offset, stably: the groups of one offset keep their order.
    groups.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut all_longhands = Vec::new();
    let mut all_custom: Vec<Arc<str>> = Vec::new();
    let keyframes = groups
        .into_iter()
        .map(|(offset, easing, composite, declarations)| {
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
                composite,
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

/// A CSS animation's keyframe as its effect reads it (Animations 2
/// section 4.1): its offset, its easing and its composite operation if
/// it declares them, and its declarations, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct CssKeyframe {
    pub offset: f64,
    pub easing: Option<Easing>,
    pub composite: Option<Composite>,
    pub declarations: Vec<css::Declaration>,
}

/// A keyframe as a script gave it (Web Animations 1 section 6.6): its
/// offset, `None` where it gave none; its easing; its composite
/// operation, `None` for `auto`; and its declarations that are valid,
/// each a property as the cascade spells it with its value.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptKeyframe {
    pub offset: Option<f64>,
    pub easing: Easing,
    pub composite: Option<Composite>,
    pub declarations: Vec<css::Declaration>,
}

/// A script's keyframes: as given, and processed.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptKeyframes {
    /// The keyframes as given, in order.
    pub given: Vec<ScriptKeyframe>,
    /// Their computed offsets, in order.
    pub computed_offsets: Vec<f64>,
    /// The keyframes processed, one for each given, in order.
    pub(crate) keyframes: Keyframes,
    /// Each keyframe's declarations as a block, which the processed
    /// keyframes' places are in.
    pub(crate) blocks: Vec<Rule>,
}

/// Why a script's keyframes were refused: a `TypeError`'s message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyframesError(pub &'static str);

impl ScriptKeyframes {
    /// None: an effect that animates nothing.
    pub fn empty() -> ScriptKeyframes {
        ScriptKeyframes::new(Vec::new()).expect("no keyframes is valid")
    }

    /// The keyframes `given`, processed (section 6.6): a `TypeError`
    /// when an offset is outside [0, 1] or the offsets given are not in
    /// order. The missing offsets are computed (section 5.3.3): the
    /// first 0 and the last 1 when there are more than one, a lone one
    /// 1, and those between spread evenly between their neighbours.
    pub fn new(given: Vec<ScriptKeyframe>) -> Result<ScriptKeyframes, KeyframesError> {
        let mut last = f64::NEG_INFINITY;
        for offset in given.iter().filter_map(|k| k.offset) {
            if !(0.0..=1.0).contains(&offset) {
                return Err(KeyframesError(
                    "Offsets must be null or in the range [0,1].",
                ));
            }
            if offset < last {
                return Err(KeyframesError(
                    "Offsets must be monotonically non-decreasing.",
                ));
            }
            last = offset;
        }
        let computed_offsets = computed_offsets(&given);
        let mut blocks = Vec::with_capacity(given.len());
        let mut all_longhands = Vec::new();
        let mut all_custom: Vec<Arc<str>> = Vec::new();
        let mut keyframes = Vec::with_capacity(given.len());
        for (b, (keyframe, &offset)) in given.iter().zip(&computed_offsets).enumerate() {
            let mut longhands = Vec::new();
            let mut custom: Vec<Arc<str>> = Vec::new();
            for declaration in &keyframe.declarations {
                if is_custom_property_name(&declaration.name) {
                    custom.push(Arc::from(declaration.name.as_str()));
                } else {
                    longhands.extend(animated(&declaration.name));
                }
            }
            longhands.sort_unstable();
            longhands.dedup();
            custom.sort_unstable();
            custom.dedup();
            all_longhands.extend_from_slice(&longhands);
            all_custom.extend_from_slice(&custom);
            keyframes.push(Keyframe {
                offset,
                easing: Some(keyframe.easing.clone()),
                composite: keyframe.composite,
                declarations: (0..keyframe.declarations.len())
                    .map(|i| (b as u32, i as u32))
                    .collect(),
                longhands,
                custom,
            });
            blocks.push(Rule::Qualified(css::QualifiedRule {
                prelude: Vec::new(),
                declarations: keyframe.declarations.clone(),
                rules: Vec::new(),
            }));
        }
        all_longhands.sort_unstable();
        all_longhands.dedup();
        all_custom.sort_unstable();
        all_custom.dedup();
        Ok(ScriptKeyframes {
            given,
            computed_offsets,
            keyframes: Keyframes {
                keyframes,
                longhands: all_longhands,
                custom: all_custom,
            },
            blocks,
        })
    }

    /// The longhands any keyframe sets, sorted.
    pub fn longhands(&self) -> &[Longhand] {
        &self.keyframes.longhands
    }

    /// The custom properties any keyframe sets, sorted.
    pub fn custom(&self) -> &[Arc<str>] {
        &self.keyframes.custom
    }

    /// The processed keyframes.
    pub(crate) fn processed(&self) -> &Keyframes {
        &self.keyframes
    }

    /// The declarations of processed keyframe `keyframe`, in order.
    pub(crate) fn declarations<'s>(
        &'s self,
        keyframe: &'s Keyframe,
    ) -> impl Iterator<Item = &'s css::Declaration> + 's {
        keyframe
            .declarations
            .iter()
            .map(move |&(b, i)| match &self.blocks[b as usize] {
                Rule::Qualified(block) => &block.declarations[i as usize],
                Rule::At(_) | Rule::Declarations(_) => {
                    unreachable!("a keyframe's declarations are a block's")
                }
            })
    }
}

/// The computed offsets of `given` (Web Animations 1 section 5.3.3).
fn computed_offsets(given: &[ScriptKeyframe]) -> Vec<f64> {
    let mut offsets: Vec<Option<f64>> = given.iter().map(|k| k.offset).collect();
    if let Some(first) = offsets.first_mut()
        && first.is_none()
    {
        *first = Some(if given.len() == 1 { 1.0 } else { 0.0 });
    }
    if given.len() > 1
        && let Some(last) = offsets.last_mut()
        && last.is_none()
    {
        *last = Some(1.0);
    }
    let mut i = 0;
    while i < offsets.len() {
        if offsets[i].is_some() {
            i += 1;
            continue;
        }
        let before = i - 1;
        let after = (i..offsets.len())
            .find(|&j| offsets[j].is_some())
            .unwrap_or(offsets.len() - 1);
        let (from, to) = (
            offsets[before].unwrap_or(0.0),
            offsets[after].unwrap_or(1.0),
        );
        let span = (after - before) as f64;
        for (k, slot) in offsets.iter_mut().enumerate().take(after).skip(i) {
            *slot = Some(from + (to - from) * (k - before) as f64 / span);
        }
        i = after;
    }
    offsets.into_iter().map(|o| o.unwrap_or(1.0)).collect()
}

/// A keyframe's offset a script wrote as a string (Web Animations 2
/// section 6.6): a `<number>`, a math function's among them, read apart
/// from any element; `None` for anything else, which is a `TypeError`.
pub fn parse_offset(text: &str) -> Option<f64> {
    let tokens = css::tokenize(text);
    let value = trim(&tokens);
    if value.is_empty() || has_substitution(value) {
        return None;
    }
    crate::cascade::number_apart(value).filter(|n| n.is_finite())
}

/// The easing a script wrote (Web Animations 1 section 6.7.1): one
/// `<easing-function>`, apart from any element; `None` for anything
/// else, which is a `TypeError`.
pub fn parse_easing(text: &str) -> Option<Easing> {
    let tokens = css::tokenize(text);
    let value = trim(&tokens);
    if value.is_empty() || has_substitution(value) {
        return None;
    }
    let mut list = easings_apart(value)?;
    if list.len() != 1 {
        return None;
    }
    list.pop()
}

/// A keyframe's declaration of `name` as a script wrote its `value`:
/// `None` when the value is not one the property takes, or the property
/// does not animate. `name` is a property's as CSS spells it, or a
/// custom property's.
pub fn script_declaration(name: &str, value: &str) -> Option<css::Declaration> {
    let custom = is_custom_property_name(name);
    if !custom && animated(name).is_empty() {
        return None;
    }
    let parsed = css::parse_style_attribute(&format!("{name}:{value}")).ok()?;
    let [declaration] = parsed.as_slice() else {
        return None;
    };
    if declaration.important || declaration.name != name {
        return None;
    }
    let valid = if custom {
        is_valid_custom_declaration(&declaration.value)
    } else {
        supports(name, &declaration.value)
    };
    valid.then(|| declaration.clone())
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
        assert_eq!(longhands.len(), 19);
    }
}
