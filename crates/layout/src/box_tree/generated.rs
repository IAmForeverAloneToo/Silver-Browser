//! The text of the boxes `::before`, `::after` and a list item's marker
//! generate (M16, M21.4, M21.6): their `content`, its quote marks chosen
//! and its counters written, found by a walk of the document in tree
//! order ahead of the build, which is the order both depend on.
//!
//! The walk visits what the builder makes a box for: an element with a
//! style whose `display` is not `none`, and the pseudo-elements
//! [`Styled::children`] gives it, a list item's marker as its first
//! child, then `::before`, and `::after` as its last; not what is inside
//! a column, nor the children of a column group other than its columns
//! (CSS2 §17.2.1). What it does not visit generates no box and touches
//! no counter (Lists 3 §4.5).
//!
//! **Counters.** Each element and pseudo-element visited has a set of
//! counters, each a name, what instantiated it and a value, the last of
//! a name its innermost (Lists 3 §4.4). A first child starts with its
//! parent's set and any other with its preceding sibling's (§4.4.1),
//! except that a counter the sibling instantiated inside a counter of
//! the same name one of their ancestors instantiated ends with the
//! sibling. That is the reference browser's scoping: the draft of 28
//! April 2026 passes such a counter on, where the reference browser and
//! three of the web-platform-tests' four `counters-scope` tests do not
//! (the fourth asks for a rule neither has). A counter's value is
//! the one the walk last gave it, which is §4.4.1's value from the
//! element just before in tree order. Then the element's `counter-reset`
//! instantiates its counters, each replacing the innermost counter of
//! its name when the element or a preceding sibling instantiated it and
//! nesting inside it otherwise (§4.4.2); its `counter-increment` adds to
//! them and its `counter-set` sets them, instantiating one at 0 first
//! where the set has none of the name, as `counter()` and `counters()`
//! do. A value is clamped to 32 bits.
//!
//! A list item increments `list-item` as though `list-item 1`, or `-1`
//! when the counter it increments is reversed, ended its
//! `counter-increment`, unless that names `list-item` (§4.6, M21.6), and
//! its marker, which comes before its `::before` in tree order, is the
//! `content` it has or, for `normal`, `list-item` written in its
//! `list-style-type` with the counter style's prefix and suffix, or the
//! type's string (§3.2).
//!
//! A reversed counter written without a value starts at what §4.4.2
//! calculates from the increments and sets of it that come after it in
//! its scope, which the walk knows only once it has passed them: when a
//! document has one, it is walked twice, the first time to find where
//! each such counter starts.
//!
//! **Bounds.** A set holds at most [`MAX_SET`] counters and a walk
//! instantiates at most [`MAX_COUNTERS`]: an instantiation past either
//! does not happen, as though it were not written, and a `counter()` of
//! a name the set then lacks writes 0. A text is at most
//! [`MAX_GENERATED_TEXT`] bytes, and a walk's texts together at most
//! [`MAX_TEXTS`]; past either, a text is cut after the last character
//! that fits.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use dom::NodeId;
use style::ComputedStyle;
use style::computed::{Content, ContentItem, Display, ListStyleType, Lists};
use style::counter_style;

use super::{Item, OBJECT, Side, Styled};

/// The counter list items increment (Lists 3 §4.6).
const LIST_ITEM: &str = "list-item";

/// The most counters one element's set holds.
const MAX_SET: usize = 1024;

/// The most counters one walk instantiates.
const MAX_COUNTERS: usize = 1 << 20;

/// The longest one pseudo-element's text, in bytes: a `::before`'s, an
/// `::after`'s or a marker's.
pub const MAX_GENERATED_TEXT: usize = 1 << 20;

/// The most text, in bytes, a walk writes in all.
const MAX_TEXTS: usize = 1 << 28;

/// The text of each `::before`, `::after` and marker of a document that
/// generates a box, by originating element and side.
#[derive(Debug)]
pub(super) struct GeneratedText {
    texts: Texts,
}

impl GeneratedText {
    /// Walks the document `styled` reads, twice when a reversed counter
    /// needs its starting value.
    pub(super) fn of(styled: &Styled<'_>) -> GeneratedText {
        let mut first = Walk::new(styled, None);
        first.run();
        if first.pending.is_empty() {
            return GeneratedText { texts: first.texts };
        }
        let starts = first.starting_values();
        let mut second = Walk::new(styled, Some(starts));
        second.run();
        GeneratedText {
            texts: second.texts,
        }
    }

    /// The text of `node`'s `side` pseudo-element; `None` for one the
    /// walk did not visit.
    pub(super) fn get(&self, node: NodeId, side: Side) -> Option<&str> {
        self.texts.get(node.index())?.as_ref()?[side as usize].as_deref()
    }
}

/// The texts of each element's pseudo-elements, by side, by the
/// element's slot in the document: none for one that generates none.
type Texts = Vec<Option<Box<[Option<String>; 3]>>>;

/// One counter (Lists 3 §4.4).
#[derive(Debug, Clone, Copy)]
struct Counter {
    /// Its name, interned.
    name: u32,
    /// The walk's number for the element or pseudo-element that
    /// instantiated it, and for that one's parent.
    creator: u32,
    parent: u32,
    value: i32,
    /// Whether it is `reversed()`, which a list item counts down.
    reversed: bool,
}

/// One element's or pseudo-element's set of counters: their numbers in
/// [`Walk::counters`], in the order instantiated, a counter that
/// replaced another in that one's place; the counters of a name are so
/// outermost first.
#[derive(Debug, Clone)]
struct Set {
    counters: Rc<[u32]>,
    /// The walk's number for the element whose set it is.
    owner: u32,
    /// Whether the element itself instantiated any of them.
    instantiated: bool,
}

impl Set {
    /// The set of the element numbered `owner` that starts as `counters`.
    fn of(owner: u32, counters: Rc<[u32]>) -> Set {
        Set {
            counters,
            owner,
            instantiated: false,
        }
    }
}

/// What §4.4.2's calculation of a reversed counter's starting value has
/// found so far, the increments and sets of it met in tree order.
#[derive(Debug, Clone, Copy, Default)]
struct Start {
    num: i64,
    last_nonzero_increment_negated: i64,
    done: bool,
}

/// An element or pseudo-element visited, while the walk is inside it.
struct Frame {
    /// The walk's number for it; 0 for the document.
    number: u32,
    display: Display,
    set: Set,
    /// Its children, and the next one to visit.
    children: Vec<Item>,
    next: usize,
    /// The set of the last child visited, which the next one starts
    /// from.
    last: Option<Set>,
}

/// One walk of the document.
struct Walk<'w, 'a> {
    styled: &'w Styled<'a>,
    names: HashMap<Arc<str>, u32>,
    counters: Vec<Counter>,
    /// The starting values of the reversed counters written without
    /// one, by their number, which a first walk found; `None` in the
    /// first walk.
    starts: Option<HashMap<u32, i32>>,
    /// Those counters, while a first walk finds their starting values.
    pending: HashMap<u32, Start>,
    /// The quote depth reached (CSS 2.2 §12.3.2): the `open-quote`s and
    /// `no-open-quote`s so far less the `close-quote`s and
    /// `no-close-quote`s, never below zero.
    quote_depth: u32,
    texts: Texts,
    /// The bytes of text written so far.
    written: usize,
    /// The number the next element or pseudo-element visited gets.
    next_number: u32,
}

impl<'w, 'a> Walk<'w, 'a> {
    fn new(styled: &'w Styled<'a>, starts: Option<HashMap<u32, i32>>) -> Self {
        Walk {
            styled,
            names: HashMap::new(),
            counters: Vec::new(),
            starts,
            pending: HashMap::new(),
            quote_depth: 0,
            texts: vec![None; styled.document.slot_count()],
            written: 0,
            next_number: 1,
        }
    }

    /// Visits the document in tree order, without recursion: each
    /// element before its children, the children in order.
    fn run(&mut self) {
        let root = Item::Node(self.styled.document.root());
        let mut stack = vec![Frame {
            number: 0,
            display: Display::Block,
            set: Set::of(0, Rc::from([])),
            children: self.styled.children(root),
            next: 0,
            last: None,
        }];
        while let Some(top) = stack.last_mut() {
            let Some(&child) = top.children.get(top.next) else {
                let done = stack.pop().expect("the frame just looked at");
                if let Some(parent) = stack.last_mut() {
                    parent.last = Some(done.set);
                }
                continue;
            };
            top.next += 1;
            let Some(style) = self.visited(child, top.display) else {
                continue;
            };
            let number = self.next_number;
            self.next_number = self.next_number.saturating_add(1);
            let parent = top.number;
            let mut set = match top.last.take() {
                Some(sibling) => self.inherit(number, sibling, &stack),
                None => Set::of(number, Rc::clone(&top.set.counters)),
            };
            let mut working = None;
            let list_item = matches!(child, Item::Node(_)) && style.display == Display::ListItem;
            if style.lists.is_some() || list_item {
                let working = working.get_or_insert_with(|| Working::of(self, &set));
                self.change(working, style.lists(), list_item, number, parent);
            }
            if let Item::Generated(node, side) = child
                && holds_content(style.display)
            {
                let text = self.text(style, side, &mut working, &set, number, parent);
                if let Some(slot) = self.texts.get_mut(node.index()) {
                    slot.get_or_insert_default()[side as usize] = Some(text);
                }
            }
            if let Some(working) = working {
                set = Set {
                    counters: Rc::from(working.counters),
                    owner: number,
                    instantiated: working.instantiated,
                };
            }
            let top = stack.len() - 1;
            if matches!(child, Item::Node(_)) && style.display != Display::TableColumn {
                stack.push(Frame {
                    number,
                    display: style.display,
                    set,
                    children: self.styled.children(child),
                    next: 0,
                    last: None,
                });
            } else {
                stack[top].last = Some(set);
            }
        }
    }

    /// The style of `child`, a child of an element whose `display` is
    /// `parent`, when the builder makes a box for it.
    fn visited(&self, child: Item, parent: Display) -> Option<&'a ComputedStyle> {
        if !self.styled.is_element(child) {
            return None;
        }
        let style = self.styled.style(child)?;
        match style.display {
            Display::None => None,
            display if parent == Display::TableColumnGroup && display != Display::TableColumn => {
                None
            }
            _ => Some(style),
        }
    }

    /// The set the element numbered `number` starts with, its preceding
    /// sibling's being `sibling` and the elements open around it
    /// `stack`: the sibling's, less a counter the sibling instantiated
    /// inside one of the same name that one of their ancestors did. The
    /// counters a sibling before that instantiated have been passed on
    /// or dropped by the same rule already.
    fn inherit(&self, number: u32, sibling: Set, stack: &[Frame]) -> Set {
        if !sibling.instantiated {
            return Set::of(number, sibling.counters);
        }
        let is_ancestor = |creator: u32| {
            stack
                .binary_search_by_key(&creator, |frame| frame.number)
                .is_ok()
        };
        // The creator of the innermost counter of each name so far.
        let mut innermost: HashMap<u32, u32> = HashMap::new();
        let mut kept = Vec::with_capacity(sibling.counters.len());
        for &id in sibling.counters.iter() {
            let counter = self.counters[id as usize];
            if counter.creator == sibling.owner
                && innermost
                    .get(&counter.name)
                    .is_some_and(|&outer| is_ancestor(outer))
            {
                continue;
            }
            innermost.insert(counter.name, counter.creator);
            kept.push(id);
        }
        Set::of(number, Rc::from(kept))
    }

    /// Applies `lists`, the counter properties of the element numbered
    /// `number`, whose parent is numbered `parent`, to its set: resets,
    /// then increments, a `list_item`'s own last, then sets (§4.4).
    fn change(
        &mut self,
        working: &mut Working,
        lists: &Lists,
        list_item: bool,
        number: u32,
        parent: u32,
    ) {
        for reset in &lists.counter_reset {
            let name = self.intern(&reset.name);
            let replaced = working.innermost(name).filter(|&at| {
                let counter = self.counters[working.counters[at] as usize];
                counter.creator == number || counter.parent == parent
            });
            if replaced.is_none() && working.is_full() {
                continue;
            }
            // A reversed counter written without a value starts at 0
            // while the first walk finds where it starts.
            let value = reset.value.unwrap_or_else(|| {
                let id = self.counters.len() as u32;
                self.starts
                    .as_ref()
                    .and_then(|starts| starts.get(&id).copied())
                    .unwrap_or(0)
            });
            let Some(id) = self.new_counter(name, value, reset.reversed, number, parent) else {
                continue;
            };
            if reset.value.is_none() && self.starts.is_none() {
                self.pending.insert(id, Start::default());
            }
            match replaced {
                Some(at) => working.replace(at, id),
                None => working.push(name, id),
            }
        }
        let mut seen: HashMap<u32, (i64, Option<i32>)> = HashMap::new();
        let mut increment = |walk: &mut Self, working: &mut Working, name: u32, by: Option<i32>| {
            let id = walk.counter_named(working, name, number, parent)?;
            let counter = &mut walk.counters[id as usize];
            // A list item's own increment counts down a reversed counter.
            let by = by.unwrap_or(if counter.reversed { -1 } else { 1 });
            // An increment that would take the value past what a 32-bit
            // integer holds is not made, the value staying, as in the
            // reference browser and the web-platform-tests
            // (counter-reset-increment-overflow-underflow), where the
            // draft clamps it.
            counter.value = counter.value.checked_add(by).unwrap_or(counter.value);
            if walk.pending.contains_key(&id) {
                let (sum, _) = seen.entry(id).or_default();
                *sum = sum.saturating_add(i64::from(by));
            }
            Some(())
        };
        for change in &lists.counter_increment {
            let name = self.intern(&change.name);
            increment(self, working, name, Some(change.value));
        }
        if list_item
            && !lists
                .counter_increment
                .iter()
                .any(|change| &*change.name == LIST_ITEM)
        {
            let name = self.intern(LIST_ITEM);
            increment(self, working, name, None);
        }
        for set in &lists.counter_set {
            let name = self.intern(&set.name);
            let Some(id) = self.counter_named(working, name, number, parent) else {
                continue;
            };
            self.counters[id as usize].value = set.value;
            if self.pending.contains_key(&id) {
                seen.entry(id).or_default().1 = Some(set.value);
            }
        }
        // §4.4.2's loop, for this element, over the reversed counters
        // whose starting values are being found.
        for (id, (increment, set)) in seen {
            let start = self.pending.get_mut(&id).expect("a pending counter");
            if start.done {
                continue;
            }
            let negated = increment.saturating_neg();
            if negated != 0 {
                start.last_nonzero_increment_negated = negated;
            }
            match set {
                Some(value) => {
                    start.num = start.num.saturating_add(i64::from(value));
                    start.done = true;
                }
                None => start.num = start.num.saturating_add(negated),
            }
        }
    }

    /// The innermost counter named `name` in `working`, instantiated at 0
    /// by the element numbered `number`, whose parent is numbered
    /// `parent`, when there is none; `None` when none can be.
    fn counter_named(
        &mut self,
        working: &mut Working,
        name: u32,
        number: u32,
        parent: u32,
    ) -> Option<u32> {
        if let Some(at) = working.innermost(name) {
            return Some(working.counters[at]);
        }
        if working.is_full() {
            return None;
        }
        let id = self.new_counter(name, 0, false, number, parent)?;
        working.push(name, id);
        Some(id)
    }

    /// A new counter, unless the walk has instantiated as many as it
    /// may.
    fn new_counter(
        &mut self,
        name: u32,
        value: i32,
        reversed: bool,
        creator: u32,
        parent: u32,
    ) -> Option<u32> {
        if self.counters.len() >= MAX_COUNTERS {
            return None;
        }
        self.counters.push(Counter {
            name,
            creator,
            parent,
            value,
            reversed,
        });
        Some((self.counters.len() - 1) as u32)
    }

    fn intern(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.names.get(name) {
            return id;
        }
        let id = self.names.len() as u32;
        self.names.insert(Arc::from(name), id);
        id
    }

    /// The text of the `side` pseudo-element with the computed `style`,
    /// numbered `number`, whose set is `working` if its counter
    /// properties have changed it and `set` otherwise: its `content` with
    /// the quote marks at the depth the walk has reached, which they then
    /// move, and its counters written in their styles; or a marker's
    /// from its `list-style-type`.
    fn text(
        &mut self,
        style: &ComputedStyle,
        side: Side,
        working: &mut Option<Working>,
        set: &Set,
        number: u32,
        parent: u32,
    ) -> String {
        let room = MAX_GENERATED_TEXT.min(MAX_TEXTS - self.written);
        let mut text = Bounded::new(room);
        let items = match &style.content {
            Content::Items(items) => items,
            Content::Normal if side == Side::Marker => {
                match &style.lists().list_style_type {
                    ListStyleType::Style(counter_style) => {
                        let name = self.intern(LIST_ITEM);
                        let working = working.get_or_insert_with(|| Working::of(self, set));
                        let value = self
                            .counter_named(working, name, number, parent)
                            .map_or(0, |id| self.counters[id as usize].value);
                        let marker = counter_style::marker(counter_style, value);
                        text.push(&marker.unwrap_or_default());
                    }
                    ListStyleType::String(string) => text.push(string),
                    ListStyleType::None => {}
                }
                self.written += text.text.len();
                return text.text;
            }
            Content::Normal | Content::None => return String::new(),
        };
        for part in items.iter() {
            match part {
                // The character that stands for an image (M23.3) is
                // another in a string.
                ContentItem::Text(s) if s.contains(OBJECT) => {
                    text.push(&s.replace(OBJECT, "\u{fffd}"))
                }
                ContentItem::Text(s) => text.push(s),
                ContentItem::Image(_) => text.push(&OBJECT.to_string()),
                ContentItem::OpenQuote => {
                    if let Some((open, _)) = style.quotes.marks(self.quote_depth as usize) {
                        text.push(open);
                    }
                    self.quote_depth = self.quote_depth.saturating_add(1);
                }
                // One that would take the depth below zero gives no
                // mark, and the depth stays (§12.3.2).
                ContentItem::CloseQuote => {
                    if self.quote_depth > 0 {
                        self.quote_depth -= 1;
                        if let Some((_, close)) = style.quotes.marks(self.quote_depth as usize) {
                            text.push(close);
                        }
                    }
                }
                ContentItem::NoOpenQuote => self.quote_depth = self.quote_depth.saturating_add(1),
                ContentItem::NoCloseQuote => self.quote_depth = self.quote_depth.saturating_sub(1),
                ContentItem::Counter {
                    name,
                    style: counter_style,
                } => {
                    let name = self.intern(name);
                    let working = working.get_or_insert_with(|| Working::of(self, set));
                    let value = self
                        .counter_named(working, name, number, parent)
                        .map_or(0, |id| self.counters[id as usize].value);
                    if !text.is_full() {
                        text.push(&counter_style::represent(counter_style, value));
                    }
                }
                ContentItem::Counters {
                    name,
                    separator,
                    style: counter_style,
                } => {
                    let name = self.intern(name);
                    let working = working.get_or_insert_with(|| Working::of(self, set));
                    // One is instantiated at 0 when the set has none of
                    // the name, unless none can be.
                    if self.counter_named(working, name, number, parent).is_none() {
                        text.push(&counter_style::represent(counter_style, 0));
                    }
                    let all = working.all(name);
                    for (i, &at) in all.iter().enumerate() {
                        if text.is_full() {
                            break;
                        }
                        if i > 0 {
                            text.push(separator);
                        }
                        let value = self.counters[working.counters[at] as usize].value;
                        text.push(&counter_style::represent(counter_style, value));
                    }
                }
            }
        }
        self.written += text.text.len();
        text.text
    }

    /// Where each reversed counter written without a value starts
    /// (§4.4.2), by its number, once a first walk has passed them all.
    fn starting_values(&self) -> HashMap<u32, i32> {
        self.pending
            .iter()
            .map(|(&id, start)| {
                let num = start
                    .num
                    .saturating_add(start.last_nonzero_increment_negated);
                (
                    id,
                    num.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
                )
            })
            .collect()
    }
}

/// Whether a box of this `display` shows its `content`: not a column or
/// a column group, which hold none (§17.2.1).
fn holds_content(display: Display) -> bool {
    !matches!(display, Display::TableColumn | Display::TableColumnGroup)
}

/// A set while one element's counter properties and `content` change
/// it, and where its counters of each name are. A reset replacing a
/// counter takes its place, so that the places stay right.
struct Working {
    counters: Vec<u32>,
    by_name: HashMap<u32, Vec<usize>>,
    instantiated: bool,
}

impl Working {
    fn of(walk: &Walk<'_, '_>, set: &Set) -> Working {
        let mut by_name: HashMap<u32, Vec<usize>> = HashMap::new();
        for (at, &id) in set.counters.iter().enumerate() {
            by_name
                .entry(walk.counters[id as usize].name)
                .or_default()
                .push(at);
        }
        Working {
            counters: set.counters.to_vec(),
            by_name,
            instantiated: set.instantiated,
        }
    }

    /// The place of the innermost counter named `name`.
    fn innermost(&self, name: u32) -> Option<usize> {
        self.by_name
            .get(&name)
            .and_then(|places| places.last().copied())
    }

    /// The places of every counter named `name`, outermost first.
    fn all(&self, name: u32) -> &[usize] {
        self.by_name.get(&name).map_or(&[], Vec::as_slice)
    }

    /// Puts the new counter `id` in the place `at` of the one it
    /// replaces, which has its name.
    fn replace(&mut self, at: usize, id: u32) {
        self.counters[at] = id;
        self.instantiated = true;
    }

    /// Whether the set holds as many counters as one may.
    fn is_full(&self) -> bool {
        self.counters.len() >= MAX_SET
    }

    /// Adds the new counter `id`, named `name`, as the innermost of its
    /// name; the set is not full.
    fn push(&mut self, name: u32, id: u32) {
        debug_assert!(!self.is_full());
        self.by_name
            .entry(name)
            .or_default()
            .push(self.counters.len());
        self.counters.push(id);
        self.instantiated = true;
    }
}

/// A text being written, cut after the last character that fits in its
/// room.
struct Bounded {
    text: String,
    room: usize,
}

impl Bounded {
    fn new(room: usize) -> Self {
        Bounded {
            text: String::new(),
            room,
        }
    }

    fn is_full(&self) -> bool {
        self.room == 0
    }

    fn push(&mut self, s: &str) {
        let mut end = s.len().min(self.room);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        self.text.push_str(&s[..end]);
        self.room = if end < s.len() { 0 } else { self.room - end };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_is_cut_after_the_last_character_that_fits() {
        let mut text = Bounded::new(5);
        text.push("ab");
        assert!(!text.is_full());
        // "é" fits in the three bytes left, "€" not in the one after.
        text.push("é€");
        assert!(text.is_full());
        text.push("x");
        assert_eq!(text.text, "abé");
        let mut text = Bounded::new(3);
        text.push("abc");
        assert_eq!((text.text.as_str(), text.is_full()), ("abc", true));
    }
}
