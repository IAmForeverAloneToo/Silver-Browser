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
//!
//! An `@container` block's rules are kept with its queries and the
//! block around it (D72): every style rule in one is in force, matched
//! only where its queries hold for the element, which the cascade
//! evaluates; a block whose prelude does not parse is passed over with
//! its rules. The other rules in one, `@keyframes` and `@property`
//! among them, are taken whatever it answers, as the reference browser
//! takes them.
//!
//! Nesting and `@scope` are desugared as the set is made (D71): a style
//! rule nested in another is kept after it, its selectors read with `&`
//! the parent's list as `:is()` and, where they name no `&`, relative to
//! it; a run of declarations after a nested rule is kept as a rule of
//! the parent's very selectors, matched as its own declarations are; a
//! group rule nested in a style rule is walked as one at the top level
//! is, with the parent's selectors carried in. A rule in an `@scope`
//! block is kept
//! with the scope, its selectors relative to `:where(:scope)` where they
//! name neither `:scope` nor `&`; the scope keeps its root and limit
//! selectors and the scope around it, which matching reads (Cascade 6
//! section 2.5).

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;

use css::{Block, Rule, Token};

use crate::cascade::ParsedRule;
use crate::conditions::supports_condition;
use crate::container::ContainerRule;
use crate::invalidation::SelectorFeatures;
use crate::keyframes::{Keyframe, Keyframes, keyframes_name};
use crate::media::{Media, MediaQueryList, parse_media_query_list, split_commas, trim};
use crate::page::{PageSelector, parse_page_selectors};
use crate::selector_matching::{
    Namespaces, Nest, Relative, SelectorList, SimpleSelector, parse_nested_selector_list,
};

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
    /// The `@font-face` rules' faces, in the order they appear, each
    /// with the innermost `@media` block around it (D54).
    font_faces: Vec<(crate::font_face::FontFaceRule, Option<u32>)>,
    /// The `@scope` rules, in the order they appear (Cascade 6 section
    /// 2.5).
    scopes: Vec<ScopeRule>,
    /// The valid `@property` rules' registrations, in the order they
    /// appear, each with the innermost `@media` block around it and its
    /// layer (D73).
    properties: Vec<(crate::property::PropertyRegistration, Option<u32>, u32)>,
    /// Each sheet's implicit scoping root, by the sheet's index: what an
    /// `@scope` rule without roots of its own scopes to, the root
    /// element where there is none.
    roots: Vec<Option<dom::NodeId>>,
    /// The `@container` blocks, in the order they appear (D72).
    containers: Vec<ContainerBlock>,
}

/// An `@container` block kept: its queries, and the block around it,
/// whose queries must hold too. A block comes after the block around
/// it.
#[derive(Debug, Clone)]
pub(crate) struct ContainerBlock {
    pub(crate) rule: ContainerRule,
    pub(crate) parent: Option<u32>,
}

/// An `@scope` rule kept: what its scoping roots and limits are, and the
/// scope it is in, whose scope its roots must be in.
#[derive(Debug, Clone)]
pub(crate) struct ScopeRule {
    /// The selectors of its roots, `:scope` in them the outer scope's
    /// root; `None` for the implicit root, the parent element of the
    /// node that owns its sheet (Cascade 6 section 2.5.1).
    pub(crate) start: Option<Arc<SelectorList>>,
    /// The selectors of its limits, `:scope` and `&` in them the root.
    pub(crate) end: Option<SelectorList>,
    /// The `@scope` rule it is in.
    pub(crate) parent: Option<u32>,
    /// Its sheet, by index, which says what its implicit root is.
    pub(crate) sheet: u32,
}

/// How heavy a nested rule's selectors may be, each of its prelude's
/// tokens counted once and each `&` in it, written or implicit, as heavy
/// as its parent's: past it the rule and the rules in it are dropped, so
/// that `& &` nested a hundred deep cannot ask for 2^100 matches.
pub(crate) const MAX_NESTED_WEIGHT: usize = 1 << 16;

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
                _ => unreachable!("a keyframe's declarations are a block's"),
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
    /// The set's `@scope` rules, which its rules' scopes name.
    pub(crate) scopes: &'s [ScopeRule],
    /// Each sheet's implicit scoping root, if it has one.
    pub(crate) roots: &'s [Option<dom::NodeId>],
    /// The set's `@container` blocks, which its rules' name.
    pub(crate) containers: &'s [ContainerBlock],
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

/// A style rule kept, or a run of declarations nested in one.
#[derive(Debug, Clone)]
struct StyleRule {
    /// Its sheet, by index.
    sheet: u32,
    /// Where its path starts in [`RuleSet::paths`], and its length.
    path: (u32, u32),
    list: Arc<SelectorList>,
    /// The innermost `@media` block around it.
    block: Option<u32>,
    layer: u32,
    /// The innermost `@scope` rule around it.
    scope: Option<u32>,
    /// The innermost `@container` block around it.
    container: Option<u32>,
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

/// What a walk carries down: the sheet, the innermost `@media` block,
/// the layer and the `@scope` rule it is inside, and what a style rule's
/// selectors are read with there.
#[derive(Clone)]
struct Place {
    sheet: u32,
    block: Option<u32>,
    layer: u32,
    scope: Option<u32>,
    /// The innermost `@container` block.
    container: Option<u32>,
    /// What `&` is: the selectors of the style rule this is nested in,
    /// or of an `@scope` rule's root in one; `None` for `:where(:scope)`.
    parent: Option<Arc<SelectorList>>,
    /// The weight of `parent`'s selectors (see [`MAX_NESTED_WEIGHT`]).
    weight: usize,
    /// What a selector that names no anchor is relative to.
    relative: Relative,
}

impl Place {
    /// What a style rule's selectors are read with here.
    fn nest(&self) -> Nest<'_> {
        Nest {
            parent: self.parent.as_ref(),
            relative: self.relative,
        }
    }
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
                scope: None,
                container: None,
                parent: None,
                weight: 0,
                relative: Relative::No,
            };
            builder.namespaces = Namespaces::default();
            builder.namespaces_closed = false;
            builder.walk(&stylesheet.rules, &place);
        }
        let mut set = builder.set;
        set.index = RuleIndex::of(&set.rules);
        set
    }

    /// Gives each sheet, by its index, its implicit scoping root: what an
    /// `@scope` rule without roots of its own scopes to.
    pub(crate) fn set_scope_roots(&mut self, roots: Vec<Option<dom::NodeId>>) {
        self.roots = roots;
    }

    /// Each sheet's implicit scoping root, as
    /// [`RuleSet::set_scope_roots`] gave them.
    pub(crate) fn scope_roots(&self) -> &[Option<dom::NodeId>] {
        &self.roots
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
            .enumerate()
            .filter(|(_, rule)| rule.block.is_none_or(|b| holds[b as usize]))
            .map(|(index, rule)| {
                let place = places[rule.layer as usize];
                ParsedRule {
                    rule: index as u32,
                    list: &rule.list,
                    scope: rule.scope,
                    container: rule.container,
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
            scopes: &self.scopes,
            roots: &self.roots,
            containers: &self.containers,
        }
    }

    /// The `@font-face` rules' faces that `answers` leave in force, in
    /// order (D54).
    pub(crate) fn font_faces_in_force(
        &self,
        answers: &[bool],
    ) -> Vec<&crate::font_face::FontFaceRule> {
        let (holds, _) = self.ordered(answers);
        self.font_faces
            .iter()
            .filter(|(_, block)| block.is_none_or(|b| holds[b as usize]))
            .map(|(face, _)| face)
            .collect()
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
                    _ => unreachable!("a page rule is an at-rule"),
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
                    _ => unreachable!("a keyframes rule is an at-rule"),
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

    /// The registrations of the `@property` rules that `answers` leave in
    /// force, in their order of precedence, the least first: by their
    /// layers' places in the order of normal declarations, then in the
    /// sheets' order (D73, Cascade 5 section 6.4.3).
    pub(crate) fn properties_in_force(
        &self,
        answers: &[bool],
    ) -> Vec<&crate::property::PropertyRegistration> {
        if self.properties.is_empty() {
            return Vec::new();
        }
        let (holds, places) = self.ordered(answers);
        let mut kept: Vec<(u32, usize, &crate::property::PropertyRegistration)> = self
            .properties
            .iter()
            .enumerate()
            .filter(|(_, (_, block, _))| block.is_none_or(|b| holds[b as usize]))
            .map(|(index, (registration, _, layer))| (places[*layer as usize], index, registration))
            .collect();
        kept.sort_by_key(|&(place, index, _)| (place, index));
        kept.into_iter()
            .map(|(.., registration)| registration)
            .collect()
    }

    /// Whether any `@container` block was kept: the cascade of a document
    /// with none asks nothing of its containers (D72).
    pub(crate) fn has_containers(&self) -> bool {
        !self.containers.is_empty()
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

    /// Rule `rule` of these, of `sheets`, as the DevTools show it (M36):
    /// its sheet's index, its selector list as written, and the at-rules
    /// around it, outermost first, each with its prelude.
    pub(crate) fn described(
        &self,
        sheets: &[css::Stylesheet],
        rule: u32,
    ) -> (u32, String, Vec<String>) {
        let rule = &self.rules[rule as usize];
        let sheet = &sheets[rule.sheet as usize];
        let path = self.path(rule.path);
        let mut conditions = Vec::new();
        let mut rules = sheet.rules.as_slice();
        let (&last, blocks) = path.split_last().expect("a rule's path is never empty");
        // The selector of the style rule a run of declarations is in.
        let mut around = String::new();
        for &index in blocks {
            match &rules[index as usize] {
                Rule::At(at) => {
                    let prelude = crate::serialize::tokens(&at.prelude);
                    conditions.push(format!("@{} {prelude}", at.name).trim_end().to_owned());
                    if let Some(Block::Rules(inner)) = &at.block {
                        rules = inner;
                    }
                }
                Rule::Qualified(qualified) => {
                    around = crate::serialize::tokens(&qualified.prelude);
                    rules = &qualified.rules;
                }
                Rule::Declarations(_) => {}
            }
        }
        let selector = match &rules[last as usize] {
            Rule::Qualified(qualified) => crate::serialize::tokens(&qualified.prelude),
            Rule::Declarations(_) => around,
            Rule::At(_) => String::new(),
        };
        (rule.sheet, selector, conditions)
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

/// The declarations of the style rule, or of the run of declarations
/// nested in one, at `path` in `sheet`.
fn declarations_at<'s>(sheet: &'s css::Stylesheet, path: &[u32]) -> &'s [css::Declaration] {
    match rule_at(sheet, path) {
        Rule::Qualified(rule) => &rule.declarations,
        Rule::Declarations(declarations) => declarations,
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
            // A style rule's nested rules (CSS Nesting 1).
            Rule::Qualified(rule) => rules = &rule.rules,
            Rule::Declarations(_) => unreachable!("a path goes through rules with blocks"),
        }
    }
    &rules[last as usize]
}

/// The tokens of an `@scope` rule's root selectors and of its limit
/// selectors, either absent.
pub type ScopeTokens<'p> = (Option<&'p [Token]>, Option<&'p [Token]>);

/// An `@scope` rule's prelude read (Cascade 6 section 2.5):
/// `[(<scope-start>)]? [to (<scope-end>)]?`, each list's tokens; `None`
/// when it is not one, or a list is empty.
fn scope_prelude(prelude: &[Token]) -> Option<ScopeTokens<'_>> {
    let mut rest = trim(prelude);
    // A parenthesized list at the start of `tokens`: its inside and what
    // follows.
    fn parenthesized(tokens: &[Token]) -> Option<(&[Token], &[Token])> {
        if !matches!(tokens.first(), Some(Token::OpenParen)) {
            return None;
        }
        let mut depth = 0usize;
        for (at, token) in tokens.iter().enumerate() {
            match token {
                Token::OpenParen | Token::Function(_) | Token::OpenSquare => depth += 1,
                Token::CloseParen | Token::CloseSquare => {
                    depth -= 1;
                    if depth == 0 {
                        let inside = trim(&tokens[1..at]);
                        return (!inside.is_empty()).then(|| (inside, trim(&tokens[at + 1..])));
                    }
                }
                _ => {}
            }
        }
        None
    }
    let start = match parenthesized(rest) {
        Some((inside, after)) => {
            rest = after;
            Some(inside)
        }
        None => None,
    };
    let end = match rest {
        [] => None,
        [Token::Ident(to), after @ ..] if to.eq_ignore_ascii_case("to") => {
            let (inside, after) = parenthesized(trim(after))?;
            if !after.is_empty() {
                return None;
            }
            Some(inside)
        }
        _ => return None,
    };
    Some((start, end))
}

/// Whether a selector of `list` ends in a pseudo-element, which a scope's
/// root or limit may not.
fn has_pseudo_element(list: &SelectorList) -> bool {
    list.0.iter().any(|s| s.pseudo_element.is_some())
}

/// An `@scope` rule's root selectors and limit selectors, either absent.
type ScopeLists = (Option<SelectorList>, Option<SelectorList>);

/// An `@scope` rule's root and limit selectors, read from its prelude
/// with `namespaces` (Cascade 6 section 2.5): the roots where `start`
/// says, relative to the style rule the scope is nested in, or to the
/// root of the scope it is in, if any; the limits relative to the root,
/// which `&` and `:scope` in them are. `None` when the prelude does not
/// parse, or a selector ends in a pseudo-element.
fn scope_lists(prelude: &[Token], namespaces: &Namespaces, start: Nest<'_>) -> Option<ScopeLists> {
    let nest = start;
    let (start, end) = scope_prelude(prelude)?;
    let read = |tokens: &[Token], nest: Nest<'_>| {
        parse_nested_selector_list(tokens, namespaces, nest)
            .ok()
            .filter(|list| !has_pseudo_element(list))
    };
    let start = match start {
        Some(tokens) => Some(read(tokens, nest)?),
        None => None,
    };
    let end = match end {
        Some(tokens) => Some(read(
            tokens,
            Nest {
                parent: None,
                relative: Relative::Scope,
            },
        )?),
        None => None,
    };
    Some((start, end))
}

/// The tokens of the root and of the limit selectors of an `@scope`
/// rule whose prelude is `prelude`, read with `namespaces`, where
/// `relative` says the rule stands: nested in a style rule, or in
/// another `@scope` rule; `None` when it is not a valid one. What the
/// CSSOM's `CSSScopeRule` shows.
pub fn scope_selectors<'p>(
    prelude: &'p [Token],
    namespaces: &Namespaces,
    relative: Relative,
) -> Option<ScopeTokens<'p>> {
    let parent = Arc::new(SelectorList(Vec::new()));
    let nest = Nest {
        parent: (relative == Relative::Nesting).then_some(&parent),
        relative,
    };
    scope_lists(prelude, namespaces, nest)?;
    scope_prelude(prelude)
}

/// An `@namespace` rule's prelude read: its prefix, if any, and its
/// namespace's URL, given as a `url()` or a string; `None` for one that
/// does not parse.
pub fn namespace_declaration(prelude: &[Token]) -> Option<(Option<String>, String)> {
    let tokens: Vec<&Token> = prelude
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .collect();
    let (prefix, url) = match tokens.as_slice() {
        [url] => (None, *url),
        [Token::Ident(prefix), url] => (Some(prefix.clone()), *url),
        _ => return None,
    };
    match url {
        Token::Url { value, bad: false } | Token::String { value, bad: false } => {
            Some((prefix, value.clone()))
        }
        _ => None,
    }
}

/// The namespaces a sheet's `rules` declare (CSS Namespaces 3): its
/// `@namespace` rules before any rule but `@charset` and `@import`.
pub fn sheet_namespaces(rules: &[Rule]) -> Namespaces {
    let mut namespaces = Namespaces::default();
    for rule in rules {
        let Rule::At(at) = rule else {
            break;
        };
        match at.name.to_ascii_lowercase().as_str() {
            "namespace" if at.block.is_none() => {
                if let Some((prefix, uri)) = namespace_declaration(&at.prelude) {
                    let ns = dom::Namespace::from_uri(Some(&uri));
                    match prefix {
                        Some(prefix) => namespaces.prefixes.push((prefix, ns)),
                        None => namespaces.default = Some(ns),
                    }
                }
            }
            "charset" | "import" => {}
            _ => break,
        }
    }
    namespaces
}

impl Builder<'_> {
    /// Takes an `@namespace` rule's prelude: an optional prefix, then the
    /// namespace's URL as a `url()` or a string; one that does not parse
    /// is ignored.
    fn declare_namespace(&mut self, prelude: &[Token]) {
        let Some((prefix, uri)) = namespace_declaration(prelude) else {
            return;
        };
        let ns = dom::Namespace::from_uri(Some(&uri));
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
                    self.style_rule(rule, place);
                }
                Rule::Declarations(declarations) => self.nested_declarations(declarations, place),
                Rule::At(at) => self.at_rule(at, place),
            }
            self.path.pop();
        }
    }

    /// Keeps a style rule, its selectors read where `place` says, and
    /// walks the rules nested in it (CSS Nesting 1). A rule whose
    /// selectors do not parse is dropped with them (css-syntax-3, M4.10),
    /// and so is one heavier than [`MAX_NESTED_WEIGHT`].
    fn style_rule(&mut self, rule: &css::QualifiedRule, place: &Place) {
        let Ok(list) = parse_nested_selector_list(&rule.prelude, &self.namespaces, place.nest())
        else {
            return;
        };
        let anchors = rule
            .prelude
            .iter()
            .filter(|t| matches!(t, Token::Delim('&')))
            .count()
            .max(usize::from(place.relative != Relative::No));
        let weight = place
            .weight
            .saturating_mul(anchors)
            .saturating_add(rule.prelude.len());
        if weight > MAX_NESTED_WEIGHT {
            return;
        }
        let list = Arc::new(list);
        self.features.add(&list);
        self.features.add_declarations(&rule.declarations);
        self.keep(Arc::clone(&list), place);
        if !rule.rules.is_empty() {
            let inside = Place {
                parent: Some(list),
                weight,
                relative: Relative::Nesting,
                ..place.clone()
            };
            self.walk(&rule.rules, &inside);
        }
    }

    /// Keeps a run of declarations nested in a style rule, or in an
    /// `@scope` rule's block: a rule of the parent's very selectors, each
    /// matching with its own specificity as the parent's own declarations
    /// do; or, in `@scope`, of `:where(:scope)` (CSS Nesting 1 section
    /// 3.3, Cascade 6 section 2.5).
    fn nested_declarations(&mut self, declarations: &[css::Declaration], place: &Place) {
        let list = match (&place.parent, place.relative) {
            (Some(parent), Relative::Nesting) => Arc::clone(parent),
            _ => {
                match parse_nested_selector_list(&[Token::Delim('&')], &self.namespaces, Nest::TOP)
                {
                    Ok(list) => Arc::new(list),
                    Err(_) => return,
                }
            }
        };
        self.features.add_declarations(declarations);
        self.keep(list, place);
    }

    /// Keeps the rule the walk is at, its selectors `list`.
    fn keep(&mut self, list: Arc<SelectorList>, place: &Place) {
        let start = self.set.paths.len() as u32;
        self.set.paths.extend_from_slice(&self.path);
        self.set.rules.push(StyleRule {
            sheet: place.sheet,
            path: (start, self.path.len() as u32),
            list,
            block: place.block,
            layer: place.layer,
            scope: place.scope,
            container: place.container,
        });
    }

    /// Keeps an `@scope` rule and walks its block, the rules in it
    /// scoped (Cascade 6 section 2.5); one whose prelude does not parse
    /// is dropped with its rules. Nested in a style rule, its roots are
    /// relative to the parent's selectors; in another `@scope` rule, to
    /// that one's root. In its block `&` is `:where(:scope)`, as in every
    /// `@scope` rule's.
    fn scope_rule(&mut self, at: &css::AtRule, rules: &[Rule], place: &Place) {
        let nest = match place.relative {
            Relative::Nesting => place.nest(),
            Relative::Scope => Nest {
                parent: None,
                relative: Relative::Scope,
            },
            Relative::No => Nest::TOP,
        };
        let Some((start, end)) = scope_lists(&at.prelude, &self.namespaces, nest) else {
            return;
        };
        let start = start.map(Arc::new);
        for list in start.iter().map(|l| &**l).chain(end.iter()) {
            self.features.add_scope(list);
        }
        let scope = self.set.scopes.len() as u32;
        self.set.scopes.push(ScopeRule {
            start: start.clone(),
            end,
            parent: place.scope,
            sheet: place.sheet,
        });
        let inside = Place {
            scope: Some(scope),
            parent: None,
            relative: Relative::Scope,
            ..place.clone()
        };
        self.walk(rules, &inside);
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
            ("font-face", None) => {
                if let Some(Block::Declarations(declarations)) = &at.block
                    && let Some(face) = crate::font_face::read(declarations)
                {
                    self.set.font_faces.push((face, place.block));
                }
            }
            // A registration whatever the scope or rule it is in, as the
            // name-defining rules are (Cascade 6 section 2.5).
            ("property", None) => {
                if let Some(Block::Declarations(declarations)) = &at.block
                    && let Some(registration) =
                        crate::property::property_rule(&at.prelude, declarations)
                {
                    self.set
                        .properties
                        .push((registration, place.block, place.layer));
                }
            }
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
                    ..place.clone()
                };
                self.walk(rules, &inside);
            }
            ("supports", Some(rules)) => {
                if supports_condition(&at.prelude) == Some(true) {
                    self.walk(rules, place);
                }
            }
            ("scope", Some(rules)) => self.scope_rule(at, rules, place),
            ("container", Some(rules)) => {
                let Some(rule) = ContainerRule::parse(&at.prelude) else {
                    return;
                };
                let container = self.set.containers.len() as u32;
                self.set.containers.push(ContainerBlock {
                    rule,
                    parent: place.container,
                });
                let inside = Place {
                    container: Some(container),
                    ..place.clone()
                };
                self.walk(rules, &inside);
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
                let inside = Place {
                    layer,
                    ..place.clone()
                };
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
        if [
            "initial",
            "inherit",
            "unset",
            "revert",
            "revert-layer",
            "revert-rule",
        ]
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

    /// D71: a nested rule's selectors as heavy as `&` makes them past
    /// [`MAX_NESTED_WEIGHT`] are dropped, with the rules in it, so that
    /// `& &` nested deep asks for no exponential matching.
    #[test]
    fn nesting_past_the_weight_bound_is_dropped() {
        let depth = 24;
        let source =
            "a { ".to_owned() + &"& & { ".repeat(depth) + "color: red" + &"}".repeat(depth + 1);
        let sheets = sheets(&source);
        let set = RuleSet::new(&sheets, &mut SelectorFeatures::default());
        assert!(
            set.rules.len() > 8,
            "the shallow ones are kept: {}",
            set.rules.len()
        );
        assert!(
            set.rules.len() <= depth,
            "the deep ones are dropped: {}",
            set.rules.len()
        );
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
