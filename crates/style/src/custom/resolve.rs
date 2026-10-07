//! The substitution itself (Values 5 Appendix A): tokens walked and their
//! substitution functions replaced, an element's own custom properties
//! computed as they are needed, with the substitution contexts guarded,
//! on a stack of its own.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use css::Token;

use super::attr::{self, Kind, Spec, Syntax};
use super::{
    CustomProperties, Function, MAX_SUBSTITUTION, Substitutions, check, closer,
    custom_property_name, environment, has_substitution, is_custom_ident, is_revert, opens,
    substitution_function, weight,
};
use crate::cascade::{CssWide, css_wide_keyword};

/// Tokens being substituted — a declaration's, borrowed, or an
/// attribute's value, read for the substitution — and where each block in
/// them closes.
pub(super) struct Source<'t> {
    tokens: Cow<'t, [Token]>,
    /// For each token that opens a block, a function or a `(`, `[` or
    /// `{`, the place of its closer, or the length of the tokens when it
    /// has none; nothing that means anything for the other tokens.
    closes: Vec<usize>,
}

impl<'t> Source<'t> {
    pub(super) fn new(tokens: Cow<'t, [Token]>) -> Self {
        let mut closes = vec![tokens.len(); tokens.len()];
        let mut open: Vec<(usize, u8)> = Vec::new();
        for (at, token) in tokens.iter().enumerate() {
            if let Some(closer) = opens(token) {
                open.push((at, closer));
            } else if let Some(closer) = closer(token)
                && open.last().is_some_and(|&(_, c)| c == closer)
            {
                let (opener, _) = open.pop().expect("a block is open");
                closes[opener] = at;
            }
        }
        Source { tokens, closes }
    }

    /// Where the block opened at `at` closes.
    fn close(&self, at: usize) -> usize {
        self.closes[at]
    }

    /// The first comma between `start` and `end` outside every block.
    fn top_level_comma(&self, start: usize, end: usize) -> Option<usize> {
        let mut at = start;
        while at < end {
            match &self.tokens[at] {
                Token::Comma => return Some(at),
                token if opens(token).is_some() => at = self.close(at) + 1,
                _ => at += 1,
            }
        }
        None
    }

    /// `start..end` without the white space at its ends.
    fn trim(&self, mut start: usize, mut end: usize) -> Span {
        while start < end && matches!(self.tokens[start], Token::Whitespace) {
            start += 1;
        }
        while end > start && matches!(self.tokens[end - 1], Token::Whitespace) {
            end -= 1;
        }
        (start, end)
    }

    /// The argument between `start` and `end`: trimmed, and, when it is
    /// wrapped in `{}`, what is inside them (Values 5 §2.4.1); `None`
    /// when anything follows its wrapper.
    fn argument(&self, start: usize, end: usize) -> Option<Span> {
        let (start, end) = self.trim(start, end);
        if start < end && matches!(self.tokens[start], Token::OpenCurly) {
            let close = self.close(start);
            if close + 1 < end {
                return None;
            }
            return Some(self.trim(start + 1, close.min(end)));
        }
        Some((start, end))
    }

    /// The arguments of the `var()` or `attr()` whose tokens run from
    /// `start` to `end`: the first, not empty, and the fallback, when a
    /// comma is there.
    fn arguments(&self, start: usize, end: usize) -> Option<Arguments> {
        let comma = self.top_level_comma(start, end);
        let first = self.argument(start, comma.unwrap_or(end))?;
        if first.0 == first.1 {
            return None;
        }
        let fallback = match comma {
            Some(comma) => Some(self.argument(comma + 1, end)?),
            None => None,
        };
        Some((first, fallback))
    }

    /// The arguments of the `env()` whose tokens run from `start` to
    /// `end`: the variable's name, how many indices follow it, and the
    /// fallback, when a comma is there.
    fn env_arguments(&self, start: usize, end: usize) -> Option<Environment<'_>> {
        let comma = self.top_level_comma(start, end);
        let mut name = None;
        let mut indices = 0;
        for token in &self.tokens[start..comma.unwrap_or(end)] {
            match token {
                Token::Whitespace => {}
                Token::Ident(n) if name.is_none() && is_custom_ident(n) => name = Some(n.as_str()),
                Token::Number {
                    value,
                    is_integer: true,
                } if name.is_some() && *value >= 0.0 => indices += 1,
                _ => return None,
            }
        }
        let fallback = match comma {
            Some(comma) => Some(self.argument(comma + 1, end)?),
            None => None,
        };
        Some(Environment {
            name: name?,
            indices,
            fallback,
        })
    }
}

/// A run of a source's tokens: where it begins and where it ends.
type Span = (usize, usize);

/// A function's first argument and its fallback.
type Arguments = (Span, Option<Span>);

/// What an `env()`'s arguments say: the variable's name, how many
/// indices follow it, and the fallback.
struct Environment<'a> {
    name: &'a str,
    indices: usize,
    fallback: Option<Span>,
}

/// Where an element's own custom property stands while its custom
/// properties are computed.
pub(super) enum State<'t> {
    /// Declared with substitution functions; not computed yet.
    Pending(&'t [Token]),
    /// Being computed, under the guard at this place among the
    /// resolver's.
    Active(usize),
    /// Computed: its value, or `None` for the guaranteed-invalid value.
    Done(Option<Arc<[Token]>>),
}

/// A substitution context (Values 5 Appendix A).
#[derive(Debug, PartialEq)]
enum Context {
    /// «"property", name»: the element's own custom property at this
    /// place among the states.
    Property(usize),
    /// «"attribute", name».
    Attribute(String),
}

/// A substitution context guarded while what it substitutes is.
struct Guard {
    context: Context,
    /// Where its frame is on the stack.
    frame: usize,
    /// Where the lowest cycle found through it begins on the stack, when
    /// one is: see [`Resolver::cycle_to`].
    cycle: Option<usize>,
}

/// A step of the substitution.
enum Frame {
    /// Computing the element's own custom property at this place among
    /// the states.
    Property(usize),
    /// Substituting an attribute's value and matching it against a
    /// syntax, for an `attr()` with a `type()`.
    Attribute(Box<Typed>),
    /// Substituting tokens.
    Value(Run),
}

/// An attribute whose value is read as a type.
struct Typed {
    /// Its name, ASCII-lowercased.
    name: String,
    syntax: Syntax,
}

/// What a frame waits for, the function it is replacing ending at `end`.
#[derive(Clone, Copy)]
enum Waiting {
    /// `var()`'s first argument substituted, the name it gives.
    Name { end: usize, fallback: Option<Span> },
    /// The element's custom property at `index`, which `var()` names,
    /// computed.
    Property {
        end: usize,
        index: usize,
        fallback: Option<Span>,
    },
    /// `attr()`'s first argument substituted, the attribute and type it
    /// gives.
    AttrName { end: usize, fallback: Option<Span> },
    /// The attribute's value substituted and matched.
    Attribute { end: usize, fallback: Option<Span> },
    /// The fallback substituted.
    Fallback { end: usize },
}

/// What a frame gave the one under it.
enum Returned {
    /// A run's tokens and their weight, or `None` for the
    /// guaranteed-invalid value.
    Value(Option<(Vec<Token>, usize)>),
    /// A custom property computed, which the frame under reads.
    Property,
    /// An attribute's value, substituted and matching its syntax, and its
    /// weight; `None` when it failed.
    Attribute(Option<(Vec<Token>, usize)>),
}

/// Tokens being substituted: a run of a source's from `pos` to `end`, and
/// what they have given so far.
struct Run {
    source: usize,
    pos: usize,
    end: usize,
    out: Vec<Token>,
    weight: usize,
    /// Set once the run has met the guaranteed-invalid value; the rest of
    /// it is still read, for the cycles in it, but not kept.
    invalid: bool,
    waiting: Option<Waiting>,
}

impl Run {
    fn new(source: usize, start: usize, end: usize) -> Self {
        Run {
            source,
            pos: start,
            end,
            out: Vec::new(),
            weight: 0,
            invalid: false,
            waiting: None,
        }
    }

    /// Adds `weight` to the run's, within the bounds; the run becomes the
    /// guaranteed-invalid value when it would go past one.
    fn grow(&mut self, weight: usize, substitutions: &Substitutions) -> bool {
        self.weight += weight;
        if self.weight > MAX_SUBSTITUTION || !substitutions.charge(weight) {
            self.fail();
            return false;
        }
        true
    }

    fn fail(&mut self) {
        self.invalid = true;
        self.out = Vec::new();
    }

    /// Copies `token`, one of the run's own.
    fn push(&mut self, token: &Token, substitutions: &Substitutions) {
        if !self.invalid && self.grow(weight(token), substitutions) {
            self.out.push(token.clone());
        }
    }

    /// Copies a function's result, `tokens`.
    fn splice(&mut self, tokens: &[Token], substitutions: &Substitutions) {
        if self.invalid {
            return;
        }
        let weight = tokens.iter().map(weight).sum();
        if weight > MAX_SUBSTITUTION {
            self.fail();
        } else if self.grow(weight, substitutions) {
            self.out.extend_from_slice(tokens);
        }
    }

    /// Takes a function's result, `tokens`, which weigh `weight` and were
    /// charged when they were copied.
    fn append(&mut self, tokens: Vec<Token>, weight: usize) {
        if self.invalid {
            return;
        }
        self.weight += weight;
        if self.weight > MAX_SUBSTITUTION {
            self.fail();
        } else {
            self.out.extend(tokens);
        }
    }

    /// What the run gave, the white space at its ends left out, and its
    /// weight; `None` for the guaranteed-invalid value.
    fn finish(self) -> Option<(Vec<Token>, usize)> {
        if self.invalid {
            return None;
        }
        let mut out = self.out;
        while matches!(out.last(), Some(Token::Whitespace)) {
            out.pop();
        }
        let leading = out
            .iter()
            .take_while(|t| matches!(t, Token::Whitespace))
            .count();
        out.drain(..leading);
        Some((out, self.weight))
    }
}

/// The substitution of one element's values: its own custom properties,
/// declared, being computed or computed; the custom properties it has
/// otherwise, its parent's or, once its own are computed, its own; and
/// its attributes, or its originating element's.
pub(super) struct Resolver<'t, 's> {
    /// The places of the element's own custom properties among `states`,
    /// by name.
    own: HashMap<&'t str, usize>,
    /// Their names, by their places.
    names: Vec<&'t str>,
    pub(super) states: Vec<State<'t>>,
    /// The contexts guarded, the newest last.
    guards: Vec<Guard>,
    inherited: Option<&'s CustomProperties>,
    attributes: &'s [(String, String)],
    substitutions: &'s Substitutions,
}

impl<'t, 's> Resolver<'t, 's> {
    /// A resolver of an element's values: `names` are its own custom
    /// properties', in `states`' order, `inherited` the custom properties
    /// it has otherwise, and `attributes` what `attr()` reads.
    pub(super) fn new(
        names: Vec<&'t str>,
        states: Vec<State<'t>>,
        inherited: Option<&'s CustomProperties>,
        attributes: &'s [(String, String)],
        substitutions: &'s Substitutions,
    ) -> Self {
        let own = names.iter().enumerate().map(|(i, &n)| (n, i)).collect();
        Resolver {
            own,
            names,
            states,
            guards: Vec::new(),
            inherited,
            attributes,
            substitutions,
        }
    }

    /// Computes the element's own custom property at `index`, and those
    /// it needs, leaving their values in their states.
    pub(super) fn compute(&mut self, index: usize) {
        self.run(Vec::new(), Frame::Property(index));
    }

    /// `value` with its substitution functions replaced; `None` for the
    /// guaranteed-invalid value.
    pub(super) fn substitute(&mut self, value: &'t [Token]) -> Option<Vec<Token>> {
        let root = Frame::Value(Run::new(0, 0, value.len()));
        self.run(vec![Source::new(Cow::Borrowed(value))], root)
    }

    /// The value of the attribute `name`, ASCII-lowercased.
    fn attribute(&self, name: &str) -> Option<&'s str> {
        self.attributes
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, value)| value.as_str())
    }

    /// Guards `context`, whose frame is at `frame` on the stack; its
    /// place among the guards.
    fn guard(&mut self, context: Context, frame: usize) -> usize {
        self.guards.push(Guard {
            context,
            frame,
            cycle: None,
        });
        self.guards.len() - 1
    }

    /// A cycle back to the guard at `place`: every context from it to
    /// the newest is in it. It is marked on the newest, which hands it to
    /// the one under it as it is left ([`Resolver::unguard`]), and that
    /// on, down to this one: a mark costs the same however deep the
    /// stack.
    fn cycle_to(&mut self, place: usize) {
        let begin = self.guards[place].frame;
        let newest = self.guards.last_mut().expect("the context is guarded");
        newest.cycle = Some(newest.cycle.map_or(begin, |c| c.min(begin)));
    }

    /// Leaves the newest guard: whether a cycle went through it. The
    /// cycle reaches the guard under it when it begins at or below that
    /// one's frame.
    fn unguard(&mut self) -> bool {
        let guard = self.guards.pop().expect("a context is guarded");
        if let (Some(begin), Some(under)) = (guard.cycle, self.guards.last_mut())
            && begin <= under.frame
        {
            under.cycle = Some(under.cycle.map_or(begin, |c| c.min(begin)));
        }
        guard.cycle.is_some()
    }

    /// Runs the substitution from `root` over `sources` to its end: the
    /// root run's tokens, or `None` for the guaranteed-invalid value; a
    /// root custom property leaves its value in its state.
    ///
    /// The frames are kept on a stack of their own: a run that meets a
    /// function whose result needs another run, a custom property
    /// computed or an attribute's value read pushes itself and that, and
    /// is resumed with what it gave.
    fn run(&mut self, mut sources: Vec<Source<'t>>, root: Frame) -> Option<Vec<Token>> {
        let mut stack = vec![root];
        let mut returned: Option<Returned> = None;
        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Property(index) => match returned.take() {
                    None => {
                        let State::Pending(tokens) = self.states[index] else {
                            unreachable!("a custom property is computed once")
                        };
                        let place = self.guard(Context::Property(index), stack.len());
                        self.states[index] = State::Active(place);
                        sources.push(Source::new(Cow::Borrowed(tokens)));
                        stack.push(Frame::Property(index));
                        stack.push(Frame::Value(Run::new(sources.len() - 1, 0, tokens.len())));
                    }
                    Some(Returned::Value(value)) => {
                        let cyclic = self.unguard();
                        let value = value
                            .filter(|_| !cyclic)
                            .and_then(|(tokens, _)| self.settle(index, tokens));
                        self.states[index] = State::Done(value);
                        returned = Some(Returned::Property);
                    }
                    Some(_) => unreachable!("a property waits for its value"),
                },
                Frame::Attribute(typed) => match returned.take() {
                    None => {
                        // «"attribute", name» met again while its value is
                        // substituted: a cycle, and this substitution the
                        // guaranteed-invalid value.
                        let again = self.guards.iter().position(
                            |g| matches!(&g.context, Context::Attribute(n) if *n == typed.name),
                        );
                        if let Some(place) = again {
                            self.cycle_to(place);
                            returned = Some(Returned::Attribute(None));
                            continue;
                        }
                        let value = self.attribute(&typed.name).unwrap_or_default();
                        let tokens = css::tokenize(value);
                        // A value that is no `<declaration-value>`, or
                        // whose own functions are invalid, parses as no
                        // syntax.
                        if !check(&tokens, true) {
                            returned = Some(Returned::Attribute(None));
                            continue;
                        }
                        let len = tokens.len();
                        self.guard(Context::Attribute(typed.name.clone()), stack.len());
                        sources.push(Source::new(Cow::Owned(tokens)));
                        stack.push(Frame::Attribute(typed));
                        stack.push(Frame::Value(Run::new(sources.len() - 1, 0, len)));
                    }
                    Some(Returned::Value(value)) => {
                        let cyclic = self.unguard();
                        let value =
                            value.filter(|(tokens, _)| !cyclic && typed.syntax.matches(tokens));
                        returned = Some(Returned::Attribute(value));
                    }
                    Some(_) => unreachable!("an attribute waits for its value"),
                },
                Frame::Value(mut run) => match self.step(&mut run, returned.take(), &sources) {
                    Some(child) => {
                        stack.push(Frame::Value(run));
                        stack.push(child);
                    }
                    None => returned = Some(Returned::Value(run.finish())),
                },
            }
        }
        match returned {
            Some(Returned::Value(value)) => value.map(|(tokens, _)| tokens),
            _ => None,
        }
    }

    /// The computed value of the element's custom property at `index`,
    /// whose declared value substituted to `tokens`. A lone CSS-wide
    /// keyword is that keyword, as if it were declared (Values 5 Appendix
    /// A): `initial` the guaranteed-invalid value, `inherit` and `unset`
    /// the inherited value. `revert` and `revert-layer`, which the
    /// cascade does not know (G7), are `unset`, which is what they give a
    /// custom property no stylesheet of an earlier origin declares.
    fn settle(&self, index: usize, tokens: Vec<Token>) -> Option<Arc<[Token]>> {
        let inherited = || {
            self.inherited
                .and_then(|m| m.get(self.names[index]))
                .cloned()
        };
        match css_wide_keyword(&tokens) {
            Some(CssWide::Initial) => None,
            Some(CssWide::Inherit | CssWide::Unset) => inherited(),
            None if is_revert(&tokens) => inherited(),
            None => Some(tokens.into()),
        }
    }

    /// Takes the run on, with what the frame it waited for `returned`:
    /// to its end, or to a function whose result needs another frame,
    /// which it returns.
    fn step(
        &mut self,
        run: &mut Run,
        returned: Option<Returned>,
        sources: &[Source<'t>],
    ) -> Option<Frame> {
        let source = &sources[run.source];
        if let Some(returned) = returned
            && let Some(child) = self.resume(run, returned)
        {
            return Some(child);
        }
        while run.pos < run.end {
            let at = run.pos;
            let token = &source.tokens[at];
            let Some(function) = substitution_function(token) else {
                run.push(token, self.substitutions);
                run.pos += 1;
                continue;
            };
            let end = source.close(at).min(run.end);
            let child = match function {
                Function::Var => match source.arguments(at + 1, end) {
                    None => self.replace(run, None, end, None),
                    Some(((start, stop), fallback)) => {
                        let first = &source.tokens[start..stop];
                        if let Some(name) = custom_property_name(first) {
                            self.var(run, name, end, fallback)
                        } else if has_substitution(first) {
                            run.waiting = Some(Waiting::Name { end, fallback });
                            Some(Frame::Value(Run::new(run.source, start, stop)))
                        } else {
                            // No substitution can make it a name.
                            self.replace(run, None, end, fallback)
                        }
                    }
                },
                Function::Attr => match source.arguments(at + 1, end) {
                    None => self.replace(run, None, end, None),
                    Some(((start, stop), fallback)) => {
                        let first = &source.tokens[start..stop];
                        if has_substitution(first) {
                            run.waiting = Some(Waiting::AttrName { end, fallback });
                            Some(Frame::Value(Run::new(run.source, start, stop)))
                        } else {
                            self.attr(run, first, end, fallback)
                        }
                    }
                },
                Function::Env => match source.env_arguments(at + 1, end) {
                    None => self.replace(run, None, end, None),
                    Some(env) => {
                        self.replace(run, environment(env.name, env.indices), end, env.fallback)
                    }
                },
            };
            if child.is_some() {
                return child;
            }
        }
        None
    }

    /// Resumes the run with what the frame it waited for `returned`.
    fn resume(&mut self, run: &mut Run, returned: Returned) -> Option<Frame> {
        match (run.waiting.take(), returned) {
            (Some(Waiting::Name { end, fallback }), Returned::Value(value)) => {
                match value
                    .as_ref()
                    .and_then(|(tokens, _)| custom_property_name(tokens))
                {
                    Some(name) => self.var(run, name, end, fallback),
                    None => self.replace(run, None, end, fallback),
                }
            }
            (
                Some(Waiting::Property {
                    end,
                    index,
                    fallback,
                }),
                Returned::Property,
            ) => {
                let State::Done(value) = &self.states[index] else {
                    unreachable!("the property waited for is computed")
                };
                let value = value.clone();
                self.replace(run, value, end, fallback)
            }
            (Some(Waiting::AttrName { end, fallback }), Returned::Value(value)) => match value {
                Some((tokens, _)) => self.attr(run, &tokens, end, fallback),
                None => self.attr_failed(run, end, fallback, true),
            },
            (Some(Waiting::Attribute { end, fallback }), Returned::Attribute(value)) => match value
            {
                Some((tokens, weight)) => {
                    run.append(tokens, weight);
                    run.pos = end + 1;
                    None
                }
                None => self.attr_failed(run, end, fallback, true),
            },
            (Some(Waiting::Fallback { end }), Returned::Value(value)) => {
                match value {
                    Some((tokens, weight)) => run.append(tokens, weight),
                    None => run.fail(),
                }
                run.pos = end + 1;
                None
            }
            _ => unreachable!("a run resumes with what it waits for"),
        }
    }

    /// Replaces the `var()` ending at `end` that names `name`: by the
    /// custom property's value, once it is computed.
    fn var(
        &mut self,
        run: &mut Run,
        name: &str,
        end: usize,
        fallback: Option<Span>,
    ) -> Option<Frame> {
        let value = match self.own.get(name).copied() {
            Some(index) => match &self.states[index] {
                State::Pending(_) => {
                    run.waiting = Some(Waiting::Property {
                        end,
                        index,
                        fallback,
                    });
                    return Some(Frame::Property(index));
                }
                State::Active(place) => {
                    let place = *place;
                    self.cycle_to(place);
                    None
                }
                State::Done(value) => value.clone(),
            },
            None => self.inherited.and_then(|m| m.get(name)).cloned(),
        };
        self.replace(run, value, end, fallback)
    }

    /// Replaces the `attr()` ending at `end` whose first argument,
    /// substituted, is `first` (Values 5 §8.7.1): by the attribute's
    /// value as a string, as a number or as the syntax its `type()` gives
    /// matches, which a frame of its own reads.
    fn attr(
        &mut self,
        run: &mut Run,
        first: &[Token],
        end: usize,
        fallback: Option<Span>,
    ) -> Option<Frame> {
        // A first argument that names no attribute and type has none:
        // with no fallback, the guaranteed-invalid value.
        let Some(spec) = Spec::parse(first) else {
            return self.attr_failed(run, end, fallback, true);
        };
        let typed = spec.kind != Kind::String;
        let value = spec.name.as_deref().and_then(|name| self.attribute(name));
        let (Some(name), Some(value)) = (spec.name, value) else {
            return self.attr_failed(run, end, fallback, typed);
        };
        match spec.kind {
            Kind::String => {
                let string = Token::String {
                    value: value.to_owned(),
                    bad: false,
                };
                run.splice(&[string], self.substitutions);
            }
            Kind::Number(unit) => match attr::number(value, unit.as_deref()) {
                Some(number) => run.splice(&[number], self.substitutions),
                None => return self.attr_failed(run, end, fallback, true),
            },
            Kind::Typed(syntax) => {
                run.waiting = Some(Waiting::Attribute { end, fallback });
                return Some(Frame::Attribute(Box::new(Typed { name, syntax })));
            }
        }
        run.pos = end + 1;
        None
    }

    /// FAILURE, Values 5 §8.7.1's last step, for the `attr()` ending at
    /// `end`: its fallback substituted, or, with none, an empty string
    /// for an `attr()` without a type and the guaranteed-invalid value
    /// for one with (`typed`).
    fn attr_failed(
        &mut self,
        run: &mut Run,
        end: usize,
        fallback: Option<Span>,
        typed: bool,
    ) -> Option<Frame> {
        if fallback.is_none() && !typed {
            let empty = Token::String {
                value: String::new(),
                bad: false,
            };
            run.splice(&[empty], self.substitutions);
            run.pos = end + 1;
            return None;
        }
        self.replace(run, None, end, fallback)
    }

    /// Replaces the function ending at `end` by `value`, or, when that is
    /// the guaranteed-invalid value, by its fallback substituted, or by
    /// the guaranteed-invalid value when it has none.
    fn replace(
        &mut self,
        run: &mut Run,
        value: Option<Arc<[Token]>>,
        end: usize,
        fallback: Option<Span>,
    ) -> Option<Frame> {
        match (value, fallback) {
            (Some(value), _) => run.splice(&value, self.substitutions),
            (None, Some((start, stop))) => {
                run.waiting = Some(Waiting::Fallback { end });
                return Some(Frame::Value(Run::new(run.source, start, stop)));
            }
            (None, None) => run.fail(),
        }
        run.pos = end + 1;
        None
    }
}
