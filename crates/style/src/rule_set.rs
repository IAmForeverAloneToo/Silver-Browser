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
//! and blocks name the layers, a dotted name a layer inside a layer;
//! an `@page` rule whose selectors parse is kept as a style rule is,
//! for the pages (`crate::page`). Every other at-rule is passed over.
//! [`RuleSet::in_force`] then gives
//! the rules a medium applies: those every `@media` list around which
//! holds, each with its layer's place in the order, which depends on
//! the medium too, a layer named only inside a block whose list is
//! false having no place (Cascade 5 §6.4.3).
//!
//! The set keeps an index of its rules' selectors by what their
//! subjects require (M35.2): an element is tried against the selectors
//! of its id's, its classes' and its tag's buckets and the universal
//! one, in the rules' order, rather than every selector of every rule.

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;

use css::{Block, Rule, Token};

use crate::cascade::ParsedRule;
use crate::conditions::supports_condition;
use crate::invalidation::SelectorFeatures;
use crate::keyframes::{Keyframe, Keyframes, keyframes_name};
use crate::media::{Media, MediaQueryList, parse_media_query_list, split_commas, trim};
use crate::page::{PageSelector, parse_page_selectors};
use crate::selector_matching::{Namespaces, SelectorList, SimpleSelector, parse_selector_list_in};

/// The style rules of a set of sheets, in the order they appear.
#[derive(Debug, Clone, Default)]
pub(crate) struct RuleSet {
    rules: Vec<StyleRule>,
    /// The rules' selectors by what their subjects require (M35.2).
    index: RuleIndex,
    /// The `@page` rules kept, in the order they appear.
    pages: Vec<PageRule>,
    /// The ways down to the rules, each a run of indices: the rule's
    /// among its sheet's, then among the block's it is in, and so on.
    paths: Vec<u32>,
    blocks: Vec<MediaBlock>,
    /// The layers, the one of the rules in no layer first.
    layers: Vec<Layer>,
    /// Each place a layer is named, in order: the layer, and the
    /// innermost `@media` block around the name.
    named: Vec<(u32, Option<u32>)>,
    /// The `@keyframes` rules kept, in the order they appear (M22).
    keyframes: Vec<KeyframesRule>,
}

/// A `@keyframes` rule kept: its name, where it is, as a
/// [`StyleRule`]'s, and its keyframes processed.
#[derive(Debug, Clone)]
struct KeyframesRule {
    name: Arc<str>,
    sheet: u32,
    path: (u32, u32),
    block: Option<u32>,
    layer: u32,
    keyframes: Keyframes,
}

/// The `@keyframes` rule of one name in force: what an animation of
/// that name runs (M22).
#[derive(Debug, Clone, Copy)]
pub(crate) struct KeyframesInForce<'s> {
    /// Its keyframes, processed.
    pub(crate) keyframes: &'s Keyframes,
    /// The rule's keyframe blocks, which the keyframes' declarations
    /// are places in.
    blocks: &'s [Rule],
}

impl<'s> KeyframesInForce<'s> {
    /// The declarations of `keyframe`, one of [`Self::keyframes`]', in
    /// order.
    pub(crate) fn declarations(
        &self,
        keyframe: &'s Keyframe,
    ) -> impl Iterator<Item = &'s css::Declaration> + 's {
        let blocks = self.blocks;
        keyframe
            .declarations
            .iter()
            .map(move |&(b, i)| match &blocks[b as usize] {
                Rule::Qualified(block) => &block.declarations[i as usize],
                Rule::At(_) => unreachable!("a keyframe's declarations are a block's"),
            })
    }
}

/// A rule's selectors by what their subject compounds require (M35.2,
/// `docs/design/performance.md`, "Rules in buckets by their rightmost
/// compound"): a selector whose subject names an id is kept under the
/// id; else, one that names a class under its first class; else, one
/// that names a tag under the tag, lowercased; else under none, the
/// universal bucket. A selector can only match an element its bucket
/// is looked up for, so trying the buckets' alone changes no answer.
#[derive(Debug, Clone, Default)]
struct RuleIndex {
    ids: HashMap<Box<str>, Vec<Indexed>>,
    classes: HashMap<Box<str>, Vec<Indexed>>,
    tags: HashMap<Box<str>, Vec<Indexed>>,
    universal: Vec<Indexed>,
}

/// A selector, by its rule's index among the set's and its own in the
/// rule's list.
#[derive(Debug, Clone, Copy)]
struct Indexed {
    rule: u32,
    selector: u32,
}

/// What a subject compound requires of an element, if anything the
/// index keeps selectors by.
enum Requires<'s> {
    Id(&'s str),
    Class(&'s str),
    Tag(&'s str),
}

impl RuleIndex {
    /// The index of `rules`, each bucket in the rules' order.
    fn of(rules: &[StyleRule]) -> RuleIndex {
        let mut index = RuleIndex::default();
        for (rule, style_rule) in rules.iter().enumerate() {
            for (selector, parsed) in style_rule.list.0.iter().enumerate() {
                let entry = Indexed {
                    rule: rule as u32,
                    selector: selector as u32,
                };
                let requires = parsed.compounds.last().and_then(|subject| {
                    let simple = &subject.simple;
                    simple
                        .iter()
                        .find_map(|s| match s {
                            SimpleSelector::Id(id) => Some(Requires::Id(id)),
                            _ => None,
                        })
                        .or_else(|| {
                            simple.iter().find_map(|s| match s {
                                SimpleSelector::Class(class) => Some(Requires::Class(class)),
                                _ => None,
                            })
                        })
                        .or_else(|| {
                            simple.iter().find_map(|s| match s {
                                SimpleSelector::Type(tag) => Some(Requires::Tag(tag)),
                                _ => None,
                            })
                        })
                });
                let bucket = match requires {
                    Some(Requires::Id(id)) => index.ids.entry(id.into()).or_default(),
                    Some(Requires::Class(class)) => index.classes.entry(class.into()).or_default(),
                    Some(Requires::Tag(tag)) => index
                        .tags
                        .entry(tag.to_ascii_lowercase().into())
                        .or_default(),
                    None => &mut index.universal,
                };
                bucket.push(entry);
            }
        }
        index
    }
}

/// The rules a medium leaves in force (M20), with the index of the set
/// they are of (M35.2): a slice of them, in order.
pub(crate) struct InForce<'s> {
    rules: Vec<ParsedRule<'s>>,
    /// Each of the set's rules' place among `rules`, or [`NOT_IN_FORCE`].
    places: Vec<u32>,
    index: &'s RuleIndex,
}

/// The place of a rule the medium leaves out.
const NOT_IN_FORCE: u32 = u32::MAX;

impl<'s> Deref for InForce<'s> {
    type Target = [ParsedRule<'s>];

    fn deref(&self) -> &Self::Target {
        &self.rules
    }
}

impl InForce<'_> {
    /// The selectors that may match `element`, by their rules' places
    /// among these and their own in the rules' lists, in order and each
    /// once, into `out`: those of its id's bucket, each of its classes',
    /// its tag's, and the universal one (M35.2).
    pub(crate) fn candidates(
        &self,
        document: &dom::Document,
        element: dom::NodeId,
        out: &mut Vec<(u32, u32)>,
    ) {
        out.clear();
        let mut add = |entries: &[Indexed]| {
            for entry in entries {
                let place = self.places[entry.rule as usize];
                if place != NOT_IN_FORCE {
                    out.push((place, entry.selector));
                }
            }
        };
        let index = self.index;
        if let Some(id) = document.attr(element, "id")
            && let Some(entries) = index.ids.get(id)
        {
            add(entries);
        }
        // HTML splits `class` on ASCII white space only, as the matcher.
        if let Some(classes) = document.attr(element, "class") {
            for class in classes.split_ascii_whitespace() {
                if let Some(entries) = index.classes.get(class) {
                    add(entries);
                }
            }
        }
        if let Some(name) = document.local_name(element) {
            let entries = if name.bytes().any(|b| b.is_ascii_uppercase()) {
                index.tags.get(name.to_ascii_lowercase().as_str())
            } else {
                index.tags.get(name)
            };
            if let Some(entries) = entries {
                add(entries);
            }
        }
        add(&index.universal);
        out.sort_unstable();
        out.dedup();
    }
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

/// An `@page` rule kept: as a [`StyleRule`], with page selectors.
#[derive(Debug, Clone)]
struct PageRule {
    sheet: u32,
    path: (u32, u32),
    selectors: Vec<PageSelector>,
    block: Option<u32>,
    layer: u32,
}

/// An `@page` rule in force: its selectors and declarations, and its
/// layer's places as [`ParsedRule`]'s.
pub(crate) struct ParsedPage<'s> {
    pub(crate) selectors: &'s [PageSelector],
    pub(crate) declarations: &'s [css::Declaration],
    pub(crate) layer: u32,
    pub(crate) important_layer: u32,
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
    /// The sheet's namespace declarations, and whether a rule that ends
    /// them has come (CSS Namespaces 3: `@namespace` only after
    /// `@charset` and `@import`).
    namespaces: Namespaces,
    namespaces_closed: bool,
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
            namespaces: Namespaces::default(),
            namespaces_closed: false,
        };
        for (sheet, stylesheet) in sheets.iter().enumerate() {
            let place = Place {
                sheet: sheet as u32,
                block: None,
                layer: UNLAYERED,
            };
            builder.namespaces = Namespaces::default();
            builder.namespaces_closed = false;
            builder.walk(&stylesheet.rules, &place);
        }
        let mut set = builder.set;
        set.index = RuleIndex::of(&set.rules);
        set
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
    ) -> InForce<'s> {
        let (holds, places) = self.ordered(answers);
        let last = places[UNLAYERED as usize];
        let mut kept = 0u32;
        let rule_places = self
            .rules
            .iter()
            .map(|rule| {
                if rule.block.is_none_or(|b| holds[b as usize]) {
                    kept += 1;
                    kept - 1
                } else {
                    NOT_IN_FORCE
                }
            })
            .collect();
        let rules = self
            .rules
            .iter()
            .filter(|rule| rule.block.is_none_or(|b| holds[b as usize]))
            .map(|rule| {
                let place = places[rule.layer as usize];
                ParsedRule {
                    list: &rule.list,
                    declarations: declarations_at(
                        &sheets[rule.sheet as usize],
                        self.path(rule.path),
                    ),
                    layer: place,
                    // Among important declarations the order of the
                    // layers is reversed.
                    important_layer: last - place,
                    svg_only: !rule.list.0.is_empty()
                        && rule.list.0.iter().all(|selector| {
                            selector.compounds.last().is_some_and(|compound| {
                                compound
                                    .simple
                                    .contains(&SimpleSelector::Namespace(dom::Namespace::Svg))
                            })
                        }),
                }
            })
            .collect();
        InForce {
            rules,
            places: rule_places,
            index: &self.index,
        }
    }

    /// The `@page` rules of `sheets` that `answers` leave in force, in
    /// order, each with its layer's place, as [`RuleSet::in_force`] gives
    /// the style rules.
    pub(crate) fn pages_in_force<'s>(
        &'s self,
        sheets: &'s [css::Stylesheet],
        answers: &[bool],
    ) -> Vec<ParsedPage<'s>> {
        let (holds, places) = self.ordered(answers);
        let last = places[UNLAYERED as usize];
        self.pages
            .iter()
            .filter(|page| page.block.is_none_or(|b| holds[b as usize]))
            .map(|page| {
                let place = places[page.layer as usize];
                let rule = rule_at(&sheets[page.sheet as usize], self.path(page.path));
                let declarations = match rule {
                    Rule::At(at) => match &at.block {
                        Some(Block::Declarations(declarations)) => declarations.as_slice(),
                        _ => unreachable!("a page rule has declarations"),
                    },
                    Rule::Qualified(_) => unreachable!("a page rule is an at-rule"),
                };
                ParsedPage {
                    selectors: &page.selectors,
                    declarations,
                    layer: place,
                    important_layer: last - place,
                }
            })
            .collect()
    }

    /// The `@keyframes` rules of `sheets`, those this set was made from,
    /// that `answers` leave in force, by name: of the rules of a name,
    /// the one in the layer last in the order of normal declarations
    /// wins (Cascade 5 §6.4), and of those the last in the sheets' order
    /// (Animations 1 §3).
    pub(crate) fn keyframes_in_force<'s>(
        &'s self,
        sheets: &'s [css::Stylesheet],
        answers: &[bool],
    ) -> HashMap<&'s str, KeyframesInForce<'s>> {
        let (holds, places) = self.ordered(answers);
        let mut best: HashMap<&'s str, (u32, usize)> = HashMap::new();
        for (index, rule) in self.keyframes.iter().enumerate() {
            if !rule.block.is_none_or(|b| holds[b as usize]) {
                continue;
            }
            let rank = (places[rule.layer as usize], index);
            let entry = best.entry(&*rule.name).or_insert(rank);
            *entry = (*entry).max(rank);
        }
        best.into_iter()
            .map(|(name, (_, index))| {
                let rule = &self.keyframes[index];
                let blocks = match rule_at(&sheets[rule.sheet as usize], self.path(rule.path)) {
                    Rule::At(at) => match &at.block {
                        Some(Block::Rules(blocks)) => blocks.as_slice(),
                        _ => unreachable!("a keyframes rule has a block of rules"),
                    },
                    Rule::Qualified(_) => unreachable!("a keyframes rule is an at-rule"),
                };
                (
                    name,
                    KeyframesInForce {
                        keyframes: &rule.keyframes,
                        blocks,
                    },
                )
            })
            .collect()
    }

    /// Whether any `@keyframes` rule was kept: a document with none runs
    /// no animation, and its cascade asks nothing of them.
    pub(crate) fn has_keyframes(&self) -> bool {
        !self.keyframes.is_empty()
    }

    /// Which `@media` blocks hold under `answers`, a block holding when
    /// its list and every list around it do, and each layer's place.
    fn ordered(&self, answers: &[bool]) -> (Vec<bool>, Vec<u32>) {
        // The block around one comes before it.
        let mut holds: Vec<bool> = Vec::with_capacity(self.blocks.len());
        for (block, answer) in self.blocks.iter().zip(answers) {
            let around = block.parent.is_none_or(|p| holds[p as usize]);
            holds.push(*answer && around);
        }
        let places = self.layer_places(&holds);
        (holds, places)
    }

    /// The path a rule's `(start, len)` names.
    fn path(&self, (start, len): (u32, u32)) -> &[u32] {
        &self.paths[start as usize..(start + len) as usize]
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
    match rule_at(sheet, path) {
        Rule::Qualified(rule) => &rule.declarations,
        Rule::At(_) => unreachable!("a style rule's path ends at a style rule"),
    }
}

/// The rule at `path` in `sheet`.
fn rule_at<'s>(sheet: &'s css::Stylesheet, path: &[u32]) -> &'s Rule {
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
    &rules[last as usize]
}

impl Builder<'_> {
    /// Takes an `@namespace` rule's prelude: an optional prefix, then the
    /// namespace's URL as a `url()` or a string; one that does not parse
    /// is ignored.
    fn declare_namespace(&mut self, prelude: &[Token]) {
        let tokens: Vec<&Token> = prelude
            .iter()
            .filter(|t| !matches!(t, Token::Whitespace))
            .collect();
        let (prefix, url) = match tokens.as_slice() {
            [url] => (None, *url),
            [Token::Ident(prefix), url] => (Some(prefix.clone()), *url),
            _ => return,
        };
        let uri = match url {
            Token::Url { value, bad: false } | Token::String { value, bad: false } => value,
            _ => return,
        };
        let ns = dom::Namespace::from_uri(Some(uri));
        match prefix {
            Some(prefix) => self.namespaces.prefixes.push((prefix, ns)),
            None => self.namespaces.default = Some(ns),
        }
    }

    fn walk(&mut self, rules: &[Rule], place: &Place) {
        for (index, rule) in rules.iter().enumerate() {
            self.path.push(index as u32);
            match rule {
                Rule::Qualified(rule) => {
                    self.namespaces_closed = true;
                    // A rule whose selectors do not parse is dropped
                    // (css-syntax-3, M4.10).
                    if let Ok(list) = parse_selector_list_in(&rule.prelude, &self.namespaces) {
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
        match name.as_str() {
            // A sheet's namespaces, before its other rules and at its top.
            "namespace" => {
                if !self.namespaces_closed && self.path.len() == 1 && at.block.is_none() {
                    self.declare_namespace(&at.prelude);
                }
                return;
            }
            "charset" | "import" => {}
            _ => self.namespaces_closed = true,
        }
        let rules = match &at.block {
            Some(Block::Rules(rules)) => Some(rules.as_slice()),
            _ => None,
        };
        match (name.as_str(), rules) {
            ("page", None) => {
                if let (Some(Block::Declarations(_)), Some(selectors)) =
                    (&at.block, parse_page_selectors(&at.prelude))
                {
                    let start = self.set.paths.len() as u32;
                    self.set.paths.extend_from_slice(&self.path);
                    self.set.pages.push(PageRule {
                        sheet: place.sheet,
                        path: (start, self.path.len() as u32),
                        selectors,
                        block: place.block,
                        layer: place.layer,
                    });
                }
            }
            ("keyframes" | "-webkit-keyframes", Some(rules)) => {
                if let Some(name) = keyframes_name(&at.prelude) {
                    let start = self.set.paths.len() as u32;
                    self.set.paths.extend_from_slice(&self.path);
                    self.set.keyframes.push(KeyframesRule {
                        name,
                        sheet: place.sheet,
                        path: (start, self.path.len() as u32),
                        block: place.block,
                        layer: place.layer,
                        keyframes: crate::keyframes::process(rules),
                    });
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selector_matching::Matcher;

    fn sheets(src: &str) -> Vec<css::Stylesheet> {
        vec![css::parse_stylesheet(src).expect("a valid sheet")]
    }

    /// The bucket each selector of `src` is kept in, in order: `#` an
    /// ID's, `.` a class's, `<` a tag's, `*` the universal one.
    fn buckets(src: &str) -> Vec<String> {
        let sheets = sheets(src);
        let set = RuleSet::new(&sheets, &mut SelectorFeatures::default());
        let index = &set.index;
        let mut out: Vec<(u32, u32, String)> = Vec::new();
        for (kind, map) in [('#', &index.ids), ('.', &index.classes), ('<', &index.tags)] {
            for (name, entries) in map {
                for entry in entries {
                    out.push((entry.rule, entry.selector, format!("{kind}{name}")));
                }
            }
        }
        for entry in &index.universal {
            out.push((entry.rule, entry.selector, "*".to_owned()));
        }
        out.sort();
        out.into_iter().map(|(.., bucket)| bucket).collect()
    }

    /// M35.2: a selector is kept by its subject's ID, else its first
    /// class, else its tag, lowercased, else in the universal bucket.
    #[test]
    fn selectors_are_kept_by_what_their_subjects_require() {
        assert_eq!(
            buckets(
                "div#a.b {} .c.d {} p {} * {} [x] {} :hover {} .x span.y {} \
                 #a .z, div {} LinearGradient {} a::before {} :is(p) {}"
            ),
            [
                "#a",
                ".c",
                "<p",
                "*",
                "*",
                "*",
                ".y",
                ".z",
                "<div",
                "<lineargradient",
                "<a",
                "*"
            ]
        );
    }

    /// M35.2: whatever matches an element is among the selectors the
    /// index gives for it, in its rules' order and once each: the index
    /// leaves out only what cannot match.
    #[test]
    fn an_element_is_given_every_selector_that_matches_it() {
        let sheets = sheets(
            "@namespace svg url(http://www.w3.org/2000/svg);
             div#a.b, .c.d, p, * {} [x] {} .x span.y, div > span {} svg|rect {}
             linearGradient {} LINEARGRADIENT {} foreignobject {} #g, .r {}
             p::before {} div:hover {} :is(p, span) {} #a .y {} input[x] {}
             .c + p {} html div p.c {} span.y ~ p {} @media (max-width: 1px) { p {} }",
        );
        let set = RuleSet::new(&sheets, &mut SelectorFeatures::default());
        let in_force = set.in_force(&sheets, &[true]);
        let html = "<div id=a class='b c'><span class='y x'>s</span><p class='c d'>p</p>\
                    <svg><linearGradient id=g></linearGradient><rect class=r></rect>\
                    <foreignObject></foreignObject></svg><input type=text x=1>                    <b class='c c d'>twice</b></div>";
        let document = dom::Document::from_node(&html::parse_document(html).expect("parses"));
        let mut matcher = Matcher::new(&document);
        let mut candidates = Vec::new();
        let mut matched = 0;
        for element in document
            .subtree(document.root())
            .filter(|&node| document.is_element(node))
        {
            in_force.candidates(&document, element, &mut candidates);
            let mut sorted = candidates.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(candidates, sorted, "in order and once each");
            for (place, rule) in in_force.iter().enumerate() {
                for (index, selector) in rule.list.0.iter().enumerate() {
                    let matches = match &selector.pseudo_element {
                        None => matcher.matches_element(element, selector),
                        Some(ending) => {
                            matcher.matches_pseudo_element(element, selector, ending.element)
                        }
                    };
                    if matches {
                        matched += 1;
                        assert!(
                            candidates.contains(&(place as u32, index as u32)),
                            "{:?} matches {element:?} but is not given",
                            selector
                        );
                    }
                }
            }
        }
        assert!(matched > 20, "the selectors match: {matched}");
    }
}
