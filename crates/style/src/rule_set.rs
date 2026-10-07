//! A set of sheets' style rules with the conditions and the cascade
//! layers they are inside (M20). See `docs/design/media.md`,
//! "Conditional rules in the cascade" and "Cascade layers".
//!
//! [`RuleSet::new`] walks every sheet's rules to any depth once, when
//! the sheets are prepared: a style rule whose prelude parses is kept
//! with the innermost `@media` block around it and its layer; an
//! `@supports` block whose condition holds is walked into, and one
//! whose condition is false or invalid is passed over with all its
//! rules, the answer being the engine's and fixed; `@layer` statements
//! and blocks name the layers, a dotted name a layer inside a layer.
//! Every other at-rule is passed over. [`RuleSet::in_force`] then gives
//! the rules a medium applies: those every `@media` list around which
//! holds, each with its layer's place in the order, which depends on
//! the medium too, a layer named only inside a block whose list is
//! false having no place (Cascade 5 §6.4.3).

use std::collections::HashMap;

use css::{Block, Rule, Token};

use crate::cascade::ParsedRule;
use crate::conditions::supports_condition;
use crate::invalidation::SelectorFeatures;
use crate::media::{Media, MediaQueryList, parse_media_query_list, split_commas, trim};
use crate::selector_matching::{SelectorList, parse_selector_list};

/// The style rules of a set of sheets, in the order they appear.
#[derive(Debug, Clone, Default)]
pub(crate) struct RuleSet {
    rules: Vec<StyleRule>,
    /// The ways down to the rules, each a run of indices: the rule's
    /// among its sheet's, then among the block's it is in, and so on.
    paths: Vec<u32>,
    blocks: Vec<MediaBlock>,
    /// The layers, the one of the rules in no layer first.
    layers: Vec<Layer>,
    /// Each place a layer is named, in order: the layer, and the
    /// innermost `@media` block around the name.
    named: Vec<(u32, Option<u32>)>,
}

/// A style rule kept.
#[derive(Debug, Clone)]
struct StyleRule {
    /// Its sheet, by index.
    sheet: u32,
    /// Where its path starts in [`RuleSet::paths`], and its length.
    path: (u32, u32),
    list: SelectorList,
    /// The innermost `@media` block around it.
    block: Option<u32>,
    layer: u32,
}

/// An `@media` block: its list, and the block around it. A block comes
/// after the block around it.
#[derive(Debug, Clone)]
struct MediaBlock {
    list: MediaQueryList,
    parent: Option<u32>,
}

/// A cascade layer: the layer it is inside. The first, that of the
/// rules in no layer, is taken to be inside itself.
#[derive(Debug, Clone)]
struct Layer {
    parent: u32,
}

/// The layer of the rules in no layer.
const UNLAYERED: u32 = 0;

/// What a walk carries down: the sheet, and the innermost `@media`
/// block and the layer it is inside.
struct Place {
    sheet: u32,
    block: Option<u32>,
    layer: u32,
}

/// The state of the walk that makes a rule set.
struct Builder<'f> {
    set: RuleSet,
    /// The named layers by the layer they are inside and their name.
    names: HashMap<(u32, String), u32>,
    path: Vec<u32>,
    features: &'f mut SelectorFeatures,
}

impl RuleSet {
    /// The rules of `sheets`, in order, with what their selectors and
    /// declarations depend on added to `features`, from every style rule
    /// whatever the media around it.
    pub(crate) fn new(sheets: &[css::Stylesheet], features: &mut SelectorFeatures) -> RuleSet {
        let mut builder = Builder {
            set: RuleSet {
                layers: vec![Layer { parent: UNLAYERED }],
                ..RuleSet::default()
            },
            names: HashMap::new(),
            path: Vec::new(),
            features,
        };
        for (sheet, stylesheet) in sheets.iter().enumerate() {
            let place = Place {
                sheet: sheet as u32,
                block: None,
                layer: UNLAYERED,
            };
            builder.walk(&stylesheet.rules, &place);
        }
        builder.set
    }

    /// The answer of each `@media` block's own list under `media`, in
    /// the order of the blocks.
    pub(crate) fn answers(&self, media: &Media) -> Vec<bool> {
        self.blocks.iter().map(|b| b.list.evaluate(media)).collect()
    }

    /// The rules of `sheets`, those this set was made from, that
    /// `answers` leave in force, in order, each with its layer's place:
    /// the layers in the order they are first named where the media
    /// holds, a layer inside another before the other's own rules, and
    /// the rules in no layer after every layer.
    pub(crate) fn in_force<'s>(
        &'s self,
        sheets: &'s [css::Stylesheet],
        answers: &[bool],
    ) -> Vec<ParsedRule<'s>> {
        // A block holds when its list and every list around it do; the
        // block around one comes before it.
        let mut holds: Vec<bool> = Vec::with_capacity(self.blocks.len());
        for (block, answer) in self.blocks.iter().zip(answers) {
            let around = block.parent.is_none_or(|p| holds[p as usize]);
            holds.push(*answer && around);
        }
        let holds_at = |block: Option<u32>| block.is_none_or(|b| holds[b as usize]);
        let places = self.layer_places(&holds);
        let last = places[UNLAYERED as usize];
        self.rules
            .iter()
            .filter(|rule| holds_at(rule.block))
            .map(|rule| {
                let (start, len) = rule.path;
                let path = &self.paths[start as usize..(start + len) as usize];
                let place = places[rule.layer as usize];
                ParsedRule {
                    list: &rule.list,
                    declarations: declarations_at(&sheets[rule.sheet as usize], path),
                    layer: place,
                    // Among important declarations the order of the
                    // layers is reversed.
                    important_layer: last - place,
                }
            })
            .collect()
    }

    /// Each layer's place in the order, given which `@media` blocks
    /// hold: a walk of the layers in the order they are first named
    /// where the media holds, each after the layers inside it. A layer
    /// never so named has no place, and no rule in force is in it; it
    /// is given the last.
    fn layer_places(&self, holds: &[bool]) -> Vec<u32> {
        let count = self.layers.len();
        let mut children: Vec<Vec<u32>> = vec![Vec::new(); count];
        let mut named = vec![false; count];
        named[UNLAYERED as usize] = true;
        for &(layer, block) in &self.named {
            if named[layer as usize] || !block.is_none_or(|b| holds[b as usize]) {
                continue;
            }
            named[layer as usize] = true;
            children[self.layers[layer as usize].parent as usize].push(layer);
        }
        // After the layers inside it, with no recursion: a dotted name
        // can nest layers as deep as it is long.
        let mut places = vec![u32::MAX; count];
        let mut next = 0u32;
        let mut stack: Vec<(u32, usize)> = vec![(UNLAYERED, 0)];
        while let Some((layer, child)) = stack.pop() {
            if let Some(&inside) = children[layer as usize].get(child) {
                stack.push((layer, child + 1));
                stack.push((inside, 0));
            } else {
                places[layer as usize] = next;
                next += 1;
            }
        }
        let last = places[UNLAYERED as usize];
        for place in &mut places {
            if *place == u32::MAX {
                *place = last;
            }
        }
        places
    }
}

/// The declarations of the style rule at `path` in `sheet`.
fn declarations_at<'s>(sheet: &'s css::Stylesheet, path: &[u32]) -> &'s [css::Declaration] {
    let mut rules = sheet.rules.as_slice();
    let (&last, blocks) = path.split_last().expect("a rule's path is never empty");
    for &index in blocks {
        match &rules[index as usize] {
            Rule::At(at) => match &at.block {
                Some(Block::Rules(inner)) => rules = inner,
                _ => unreachable!("a path goes through blocks of rules"),
            },
            Rule::Qualified(_) => unreachable!("a path goes through at-rules"),
        }
    }
    match &rules[last as usize] {
        Rule::Qualified(rule) => &rule.declarations,
        Rule::At(_) => unreachable!("a path ends at a style rule"),
    }
}

impl Builder<'_> {
    fn walk(&mut self, rules: &[Rule], place: &Place) {
        for (index, rule) in rules.iter().enumerate() {
            self.path.push(index as u32);
            match rule {
                Rule::Qualified(rule) => {
                    // A rule whose selectors do not parse is dropped
                    // (css-syntax-3, M4.10).
                    if let Ok(list) = parse_selector_list(&rule.prelude) {
                        self.features.add(&list);
                        self.features.add_declarations(&rule.declarations);
                        let start = self.set.paths.len() as u32;
                        self.set.paths.extend_from_slice(&self.path);
                        self.set.rules.push(StyleRule {
                            sheet: place.sheet,
                            path: (start, self.path.len() as u32),
                            list,
                            block: place.block,
                            layer: place.layer,
                        });
                    }
                }
                Rule::At(at) => self.at_rule(at, place),
            }
            self.path.pop();
        }
    }

    fn at_rule(&mut self, at: &css::AtRule, place: &Place) {
        let name = at.name.to_ascii_lowercase();
        let rules = match &at.block {
            Some(Block::Rules(rules)) => Some(rules.as_slice()),
            _ => None,
        };
        match (name.as_str(), rules) {
            ("media", Some(rules)) => {
                let block = self.set.blocks.len() as u32;
                self.set.blocks.push(MediaBlock {
                    list: parse_media_query_list(&at.prelude),
                    parent: place.block,
                });
                let inside = Place {
                    block: Some(block),
                    ..*place
                };
                self.walk(rules, &inside);
            }
            ("supports", Some(rules)) => {
                if supports_condition(&at.prelude) == Some(true) {
                    self.walk(rules, place);
                }
            }
            // The statement: layers named in order, without rules.
            ("layer", None) if at.block.is_none() => {
                let names: Option<Vec<Vec<&str>>> = split_commas(&at.prelude)
                    .into_iter()
                    .map(|name| layer_name(trim(name)))
                    .collect();
                for name in names.into_iter().flatten() {
                    self.name_layer(&name, place);
                }
            }
            ("layer", Some(rules)) => {
                let prelude = trim(&at.prelude);
                let layer = if prelude.is_empty() {
                    // Anonymous: a layer of its own, which nothing else
                    // can name.
                    let layer = self.set.layers.len() as u32;
                    self.set.layers.push(Layer {
                        parent: place.layer,
                    });
                    self.set.named.push((layer, place.block));
                    layer
                } else {
                    match layer_name(prelude) {
                        Some(name) => self.name_layer(&name, place),
                        None => return,
                    }
                };
                let inside = Place { layer, ..*place };
                self.walk(rules, &inside);
            }
            _ => {}
        }
    }

    /// Names the layer `name`, its parts from the outermost, inside the
    /// layer `place` is in: each part a layer inside the one before,
    /// made the first time it is named. Gives the innermost.
    fn name_layer(&mut self, name: &[&str], place: &Place) -> u32 {
        let mut layer = place.layer;
        for part in name {
            let next = self.set.layers.len() as u32;
            layer = *self
                .names
                .entry((layer, (*part).to_owned()))
                .or_insert_with(|| {
                    self.set.layers.push(Layer { parent: layer });
                    next
                });
            self.set.named.push((layer, place.block));
        }
        layer
    }
}

/// Whether `tokens` are a `<layer-name>`, as `@import`'s `layer()`
/// takes one.
pub fn is_layer_name(tokens: &[Token]) -> bool {
    layer_name(tokens).is_some()
}

/// A `<layer-name>`: identifiers joined by `.` with nothing between
/// (Cascade 5 §6.4.2); `None` when the tokens are not one, or when a
/// part is a CSS-wide keyword, which the draft reserves.
fn layer_name(tokens: &[Token]) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    let mut tokens = tokens.iter();
    loop {
        let Some(Token::Ident(part)) = tokens.next() else {
            return None;
        };
        if ["initial", "inherit", "unset", "revert", "revert-layer"]
            .iter()
            .any(|keyword| part.eq_ignore_ascii_case(keyword))
        {
            return None;
        }
        parts.push(part.as_str());
        match tokens.next() {
            None => return Some(parts),
            Some(Token::Delim('.')) => {}
            Some(_) => return None,
        }
    }
}
