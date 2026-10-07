//! Math functions (css-values-4 §10, M21.2): `calc()`, `min()`, `max()`
//! and `clamp()`, parsed from a declaration's tokens, typed, and
//! simplified where the value is computed.
//!
//! [`parse`](crate::calc::parse) reads one math function by §10.8's grammar: sums whose `+`
//! and `-` have white space on both sides, products with `*` and `/`,
//! parenthesized sums, nested math functions, numbers, dimensions,
//! percentages and the constants `e`, `pi`, `infinity`, `-infinity` and
//! `NaN`, the names and constants ASCII case-insensitive. Nesting, by
//! parentheses and functions together, is bounded at [`MAX_DEPTH`](crate::calc::MAX_DEPTH); a
//! deeper calculation is invalid, as §10.8 lets a user agent's limit
//! make it. Every calculation is typed by §10.9's algebra, the types of
//! CSS Typed OM §4.3.2 — a power of each base type and a percent hint —
//! so that `calc(100px / 10px)` is a number and `calc(1px * 1px)` a
//! length squared, which no property takes; and the type is checked
//! against what the value's place takes, a [`Want`](crate::calc::Want). A calculation whose
//! type fails, or does not match, is invalid, as any bad value is.
//!
//! [`Parsed::compute`](crate::calc::Parsed::compute) simplifies by §10.10 with what computing a value
//! knows: every dimension is made canonical — a length in CSS pixels by
//! the cascade's bases, an angle in degrees, a time in seconds, a
//! frequency in hertz, a resolution in dots per pixel — and every
//! subtree with no percentage in it folds to its value. What is left is
//! a value, a percentage, or, where a percentage stands for a length
//! not yet known, an [`Expression`](crate::calc::Expression) that layout resolves against the
//! percentage's basis ([`Calc`](crate::calc::Calc)). §10.9.2 censors a top-level result:
//! `NaN` is 0, and an infinity is the largest value the place holds.

use css::Token;

use crate::cascade::MAX_LENGTH_PX;

/// The deepest a calculation may nest, parentheses and math functions
/// together: §10.8's least that a user agent must support, as
/// selectors' and media queries' parentheses are bounded.
pub const MAX_DEPTH: usize = 32;

/// What a math function's place takes (§10.9's matching of a type to a
/// production, with the place's calculation context).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    /// A `<number>`, or an `<integer>`, which the caller rounds.
    Number,
    /// A `<number>` or a `<percentage>` that stands for a number, as in
    /// `opacity` and `scale()`: the two are never added together.
    NumberOrPercentage,
    /// A `<length>`; no percentage.
    Length,
    /// A `<length-percentage>`: a percentage stands for a length.
    LengthPercentage,
    /// An `<angle>`.
    Angle,
    /// A `<time>`.
    Time,
    /// A `<resolution>`.
    Resolution,
    /// A `<percentage>` alone.
    Percentage,
}

/// A unit of length (css-values-4 §6.1 and §6.2), as the cascade takes
/// lengths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthUnit {
    Px,
    In,
    Cm,
    Mm,
    Q,
    Pt,
    Pc,
    Em,
    Rem,
    Ex,
    Ch,
    Vw,
    Vh,
    Vmin,
    Vmax,
}

impl LengthUnit {
    /// The unit of `name`, ASCII case-insensitively.
    pub fn of(name: &str) -> Option<LengthUnit> {
        Some(match name.to_ascii_lowercase().as_str() {
            "px" => LengthUnit::Px,
            "in" => LengthUnit::In,
            "cm" => LengthUnit::Cm,
            "mm" => LengthUnit::Mm,
            "q" => LengthUnit::Q,
            "pt" => LengthUnit::Pt,
            "pc" => LengthUnit::Pc,
            "em" => LengthUnit::Em,
            "rem" => LengthUnit::Rem,
            "ex" => LengthUnit::Ex,
            "ch" => LengthUnit::Ch,
            "vw" => LengthUnit::Vw,
            "vh" => LengthUnit::Vh,
            "vmin" => LengthUnit::Vmin,
            "vmax" => LengthUnit::Vmax,
            _ => return None,
        })
    }

    /// CSS pixels per unit, for an absolute unit; `None` for a relative
    /// one, which the cascade's bases resolve.
    pub fn absolute(self) -> Option<f64> {
        Some(match self {
            LengthUnit::Px => 1.0,
            LengthUnit::In => 96.0,
            LengthUnit::Cm => 96.0 / 2.54,
            LengthUnit::Mm => 96.0 / 25.4,
            LengthUnit::Q => 96.0 / 25.4 / 4.0,
            LengthUnit::Pt => 96.0 / 72.0,
            LengthUnit::Pc => 16.0,
            _ => return None,
        })
    }
}

/// What a dimension's unit measures: what a `<syntax>`'s data types ask
/// of a value without computing it (M21.5's `attr()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measure {
    Length,
    Angle,
    Time,
    Frequency,
    Resolution,
}

/// What the unit `name` measures, ASCII case-insensitively; `None` for a
/// unit this engine does not know.
pub fn measure(name: &str) -> Option<Measure> {
    Some(match Unit::of(name)? {
        Unit::Length(_) => Measure::Length,
        Unit::Angle(_) => Measure::Angle,
        Unit::Time(_) => Measure::Time,
        Unit::Frequency(_) => Measure::Frequency,
        Unit::Resolution(_) => Measure::Resolution,
    })
}

/// A dimension's unit.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Unit {
    Length(LengthUnit),
    /// An angle, by degrees per unit.
    Angle(f64),
    /// A time, by seconds per unit.
    Time(f64),
    /// A frequency, by hertz per unit.
    Frequency(f64),
    /// A resolution, by dots per CSS pixel per unit.
    Resolution(f64),
}

impl Unit {
    fn of(name: &str) -> Option<Unit> {
        if let Some(length) = LengthUnit::of(name) {
            return Some(Unit::Length(length));
        }
        Some(match name.to_ascii_lowercase().as_str() {
            "deg" => Unit::Angle(1.0),
            "grad" => Unit::Angle(0.9),
            "rad" => Unit::Angle(180.0 / std::f64::consts::PI),
            "turn" => Unit::Angle(360.0),
            "s" => Unit::Time(1.0),
            "ms" => Unit::Time(0.001),
            "hz" => Unit::Frequency(1.0),
            "khz" => Unit::Frequency(1000.0),
            "dppx" | "x" => Unit::Resolution(1.0),
            "dpi" => Unit::Resolution(1.0 / 96.0),
            "dpcm" => Unit::Resolution(2.54 / 96.0),
            _ => return None,
        })
    }

    fn base(self) -> Base {
        match self {
            Unit::Length(_) => Base::Length,
            Unit::Angle(_) => Base::Angle,
            Unit::Time(_) => Base::Time,
            Unit::Frequency(_) => Base::Frequency,
            Unit::Resolution(_) => Base::Resolution,
        }
    }
}

/// A base type of §10.9 (Typed OM's "length", "angle", …). `flex` is
/// left out: no math function here takes `fr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Base {
    Length = 0,
    Angle = 1,
    Time = 2,
    Frequency = 3,
    Resolution = 4,
    Percent = 5,
}

const BASES: usize = 6;

/// A calculation's type (Typed OM §4.3.2): the power of each base type,
/// and the percent hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Type {
    powers: [i32; BASES],
    hint: Option<Base>,
}

impl Type {
    const NUMBER: Type = Type {
        powers: [0; BASES],
        hint: None,
    };

    fn of(base: Base) -> Type {
        let mut t = Type::NUMBER;
        t.powers[base as usize] = 1;
        t
    }

    /// "Apply the percent hint": the percent power moved onto `hint`.
    fn apply_hint(mut self, hint: Base) -> Type {
        self.hint = Some(hint);
        if hint != Base::Percent {
            self.powers[hint as usize] += self.powers[Base::Percent as usize];
            self.powers[Base::Percent as usize] = 0;
        }
        self
    }

    /// "Add two types".
    fn add(self, other: Type) -> Option<Type> {
        let (mut a, mut b) = (self, other);
        match (a.hint, b.hint) {
            (Some(x), Some(y)) if x != y => return None,
            (Some(x), None) => b = b.apply_hint(x),
            (None, Some(y)) => a = a.apply_hint(y),
            _ => {}
        }
        if a.powers == b.powers {
            return Some(Type {
                powers: a.powers,
                hint: a.hint,
            });
        }
        let percent = Base::Percent as usize;
        let has_percent = a.powers[percent] != 0 || b.powers[percent] != 0;
        let has_other = (0..BASES).any(|i| i != percent && (a.powers[i] != 0 || b.powers[i] != 0));
        if has_percent && has_other {
            for hint in [
                Base::Length,
                Base::Angle,
                Base::Time,
                Base::Frequency,
                Base::Resolution,
            ] {
                let (x, y) = (a.apply_hint(hint), b.apply_hint(hint));
                if x.powers == y.powers {
                    return Some(Type {
                        powers: x.powers,
                        hint: Some(hint),
                    });
                }
            }
        }
        None
    }

    /// "Multiply two types".
    fn multiply(self, other: Type) -> Option<Type> {
        let (mut a, mut b) = (self, other);
        match (a.hint, b.hint) {
            (Some(x), Some(y)) if x != y => return None,
            (Some(x), None) => b = b.apply_hint(x),
            (None, Some(y)) => a = a.apply_hint(y),
            _ => {}
        }
        let mut powers = a.powers;
        for (power, add) in powers.iter_mut().zip(b.powers) {
            *power += add;
        }
        Some(Type {
            powers,
            hint: a.hint,
        })
    }

    /// "Invert a type".
    fn invert(self) -> Type {
        Type {
            powers: self.powers.map(|p| -p),
            hint: self.hint,
        }
    }

    /// Whether the type is `base` to the first power and nothing else.
    fn is(self, base: Base) -> bool {
        (0..BASES).all(|i| self.powers[i] == i32::from(i == base as usize))
    }

    fn is_number(self) -> bool {
        self.powers == [0; BASES]
    }

    /// Whether the type matches what `want` takes (Typed OM's "match a
    /// CSS production" with the context's percentages).
    fn matches(self, want: Want) -> bool {
        let no_hint = self.hint.is_none();
        let percent_hint = matches!(self.hint, None | Some(Base::Percent));
        match want {
            Want::Number => self.is_number() && no_hint,
            Want::NumberOrPercentage => {
                (self.is_number() || self.is(Base::Percent)) && percent_hint
            }
            Want::Length => self.is(Base::Length) && no_hint,
            Want::LengthPercentage => {
                self.is(Base::Length) && matches!(self.hint, None | Some(Base::Length))
            }
            Want::Angle => self.is(Base::Angle) && no_hint,
            Want::Time => self.is(Base::Time) && no_hint,
            Want::Resolution => self.is(Base::Resolution) && no_hint,
            Want::Percentage => self.is(Base::Percent) && percent_hint,
        }
    }
}

/// A calculation as parsed: §10.10's calculation tree before it is
/// simplified, its leaves as written.
#[derive(Debug, Clone, PartialEq)]
enum Expr {
    Number(f64),
    Percent(f64),
    Dimension(f64, Unit),
    Sum(Vec<Expr>),
    Negate(Box<Expr>),
    Product(Vec<Expr>),
    Invert(Box<Expr>),
    Min(Vec<Expr>),
    Max(Vec<Expr>),
    /// `clamp()`, its `none` bounds absent.
    Clamp(Option<Box<Expr>>, Box<Expr>, Option<Box<Expr>>),
}

/// A math function parsed and typed, to be computed for each element.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    expr: Expr,
    /// Whether the calculation's type is a percentage, in a place that
    /// takes a number or a percentage.
    percentage: bool,
}

/// Whether `tokens`, white space aside, are one math function this
/// module parses — `calc()`, `min()`, `max()` or `clamp()`.
pub fn is_math_function(tokens: &[Token]) -> bool {
    let mut it = tokens.iter().filter(|t| !matches!(t, Token::Whitespace));
    matches!(it.next(), Some(Token::Function(name)) if math_name(name))
}

fn math_name(name: &str) -> bool {
    ["calc", "min", "max", "clamp"]
        .iter()
        .any(|known| name.eq_ignore_ascii_case(known))
}

/// Parses `tokens`, white space aside one math function, for a place
/// that takes `want`; `None` when they are not one, it is malformed, or
/// its type fails or does not match.
pub fn parse(tokens: &[Token], want: Want) -> Option<Parsed> {
    let tokens = trim(tokens);
    let mut p = Parser { tokens, pos: 0 };
    let expr = p.function(0)?;
    if p.pos != tokens.len() {
        return None;
    }
    let ty = type_of(&expr, want)?;
    if !ty.matches(want) {
        return None;
    }
    let percentage = matches!(want, Want::NumberOrPercentage | Want::Percentage)
        && ty.powers[Base::Percent as usize] == 1;
    Some(Parsed { expr, percentage })
}

fn trim(tokens: &[Token]) -> &[Token] {
    let start = tokens
        .iter()
        .position(|t| !matches!(t, Token::Whitespace))
        .unwrap_or(tokens.len());
    let end = tokens
        .iter()
        .rposition(|t| !matches!(t, Token::Whitespace))
        .map_or(start, |i| i + 1);
    &tokens[start..end]
}

/// The type of `expr` where a percentage is typed by `want`'s context
/// (§10.9: a percentage resolved against a length is a length with a
/// percent hint).
fn type_of(expr: &Expr, want: Want) -> Option<Type> {
    Some(match expr {
        Expr::Number(_) => Type::NUMBER,
        Expr::Percent(_) => {
            if want == Want::LengthPercentage {
                Type::of(Base::Length).apply_hint(Base::Length)
            } else {
                Type::of(Base::Percent).apply_hint(Base::Percent)
            }
        }
        Expr::Dimension(_, unit) => Type::of(unit.base()),
        Expr::Negate(child) => type_of(child, want)?,
        Expr::Invert(child) => type_of(child, want)?.invert(),
        Expr::Product(children) => {
            let mut ty = Type::NUMBER;
            for child in children {
                ty = ty.multiply(type_of(child, want)?)?;
            }
            ty
        }
        Expr::Sum(children) | Expr::Min(children) | Expr::Max(children) => {
            let mut types = children.iter().map(|c| type_of(c, want));
            let mut ty = types.next()??;
            for next in types {
                ty = ty.add(next?)?;
            }
            ty
        }
        Expr::Clamp(min, value, max) => {
            let mut ty = type_of(value, want)?;
            for bound in [min, max].into_iter().flatten() {
                ty = ty.add(type_of(bound, want)?)?;
            }
            ty
        }
    })
}

struct Parser<'t> {
    tokens: &'t [Token],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn skip_whitespace(&mut self) -> bool {
        let start = self.pos;
        while matches!(self.peek(), Some(Token::Whitespace)) {
            self.pos += 1;
        }
        self.pos != start
    }

    /// A math function, at its name; `depth` the levels of nesting
    /// around it, its own sums being one deeper.
    fn function(&mut self, depth: usize) -> Option<Expr> {
        if depth >= MAX_DEPTH {
            return None;
        }
        let Some(Token::Function(name)) = self.peek() else {
            return None;
        };
        let name = name.to_ascii_lowercase();
        self.pos += 1;
        let expr = match name.as_str() {
            "calc" => {
                let sum = self.sum(depth + 1)?;
                self.close()?;
                sum
            }
            "min" | "max" => {
                let mut arguments = vec![self.sum(depth + 1)?];
                while self.comma() {
                    arguments.push(self.sum(depth + 1)?);
                }
                self.close()?;
                if name == "min" {
                    Expr::Min(arguments)
                } else {
                    Expr::Max(arguments)
                }
            }
            "clamp" => {
                let min = self.bound(depth + 1)?;
                if !self.comma() {
                    return None;
                }
                let value = self.sum(depth + 1)?;
                if !self.comma() {
                    return None;
                }
                let max = self.bound(depth + 1)?;
                self.close()?;
                Expr::Clamp(min.map(Box::new), Box::new(value), max.map(Box::new))
            }
            _ => return None,
        };
        Some(expr)
    }

    /// A `clamp()` bound: `none`, or a sum.
    fn bound(&mut self, depth: usize) -> Option<Option<Expr>> {
        self.skip_whitespace();
        if let Some(Token::Ident(word)) = self.peek()
            && word.eq_ignore_ascii_case("none")
        {
            self.pos += 1;
            self.skip_whitespace();
            return Some(None);
        }
        self.sum(depth).map(Some)
    }

    /// Takes a `,` after optional white space.
    fn comma(&mut self) -> bool {
        self.skip_whitespace();
        if matches!(self.peek(), Some(Token::Comma)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Takes the `)` that ends a function or a parenthesized sum.
    fn close(&mut self) -> Option<()> {
        self.skip_whitespace();
        if matches!(self.peek(), Some(Token::CloseParen)) {
            self.pos += 1;
            Some(())
        } else {
            None
        }
    }

    /// A `<calc-sum>`: products joined by `+` and `-`, each with white
    /// space on both sides.
    fn sum(&mut self, depth: usize) -> Option<Expr> {
        self.skip_whitespace();
        let mut terms = vec![self.product(depth)?];
        loop {
            let before = self.pos;
            let spaced = self.skip_whitespace();
            let negate = match self.peek() {
                Some(Token::Delim('+')) => false,
                Some(Token::Delim('-')) => true,
                _ => {
                    self.pos = before;
                    break;
                }
            };
            self.pos += 1;
            if !spaced || !self.skip_whitespace() {
                return None;
            }
            let term = self.product(depth)?;
            terms.push(if negate {
                Expr::Negate(Box::new(term))
            } else {
                term
            });
        }
        Some(if terms.len() == 1 {
            terms.pop()?
        } else {
            Expr::Sum(terms)
        })
    }

    /// A `<calc-product>`: values joined by `*` and `/`.
    fn product(&mut self, depth: usize) -> Option<Expr> {
        let mut factors = vec![self.value(depth)?];
        loop {
            let before = self.pos;
            self.skip_whitespace();
            let invert = match self.peek() {
                Some(Token::Delim('*')) => false,
                Some(Token::Delim('/')) => true,
                _ => {
                    self.pos = before;
                    break;
                }
            };
            self.pos += 1;
            self.skip_whitespace();
            let factor = self.value(depth)?;
            factors.push(if invert {
                Expr::Invert(Box::new(factor))
            } else {
                factor
            });
        }
        Some(if factors.len() == 1 {
            factors.pop()?
        } else {
            Expr::Product(factors)
        })
    }

    /// A `<calc-value>`, `depth` levels deep: a parenthesized sum or a
    /// math function in it is a level deeper.
    fn value(&mut self, depth: usize) -> Option<Expr> {
        let token = self.peek()?.clone();
        Some(match token {
            Token::Number { value, .. } => {
                self.pos += 1;
                Expr::Number(value)
            }
            Token::Percentage { value } => {
                self.pos += 1;
                Expr::Percent(value)
            }
            Token::Dimension { value, unit } => {
                self.pos += 1;
                Expr::Dimension(value, Unit::of(&unit)?)
            }
            Token::Ident(word) => {
                self.pos += 1;
                Expr::Number(match word.to_ascii_lowercase().as_str() {
                    "e" => std::f64::consts::E,
                    "pi" => std::f64::consts::PI,
                    "infinity" => f64::INFINITY,
                    "-infinity" => f64::NEG_INFINITY,
                    "nan" => f64::NAN,
                    _ => return None,
                })
            }
            Token::OpenParen => {
                if depth >= MAX_DEPTH {
                    return None;
                }
                self.pos += 1;
                let sum = self.sum(depth + 1)?;
                self.close()?;
                sum
            }
            Token::Function(_) => self.function(depth)?,
            _ => return None,
        })
    }
}

/// The result of computing a math function for an element.
#[derive(Debug, Clone, PartialEq)]
pub enum Computed {
    /// A value in its canonical unit — CSS pixels, degrees, seconds,
    /// hertz, dots per pixel — or a number. An infinity is left for the
    /// place to clamp to its range; `NaN` is already 0.
    Value(f64),
    /// A percentage, where the place takes one: in a number's place
    /// (`opacity`), or a length's whose calculation was a percentage
    /// alone. An infinity is left for the place to clamp to its range,
    /// a length's once the percentage is resolved; `NaN` is already 0.
    Percent(f64),
    /// A length with a percentage in it, for layout to resolve against
    /// the percentage's basis.
    Expression(Expression),
}

/// A length calculation simplified with everything known when a value
/// is computed (§10.10): its leaves are values in CSS pixels or numbers
/// and percentages of a basis not yet known.
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    /// A length in CSS pixels, or a number, folded.
    Value(f32),
    /// A percentage of the basis, as written (`50%` is `50.0`).
    Percent(f32),
    Sum(Box<[Expression]>),
    Negate(Box<Expression>),
    Product(Box<[Expression]>),
    Invert(Box<Expression>),
    Min(Box<[Expression]>),
    Max(Box<[Expression]>),
    /// `clamp()`, a `none` bound an infinity.
    Clamp(Box<[Expression; 3]>),
}

impl Expression {
    /// The expression's value with `basis` for its percentages, before
    /// §10.9.2's censoring of a top-level result.
    pub fn evaluate(&self, basis: f32) -> f64 {
        let basis = f64::from(basis);
        self.at(basis)
    }

    fn at(&self, basis: f64) -> f64 {
        match self {
            Expression::Value(v) => f64::from(*v),
            Expression::Percent(p) => basis * f64::from(*p) / 100.0,
            Expression::Sum(children) => children.iter().map(|c| c.at(basis)).sum(),
            Expression::Negate(child) => -child.at(basis),
            Expression::Product(children) => children.iter().map(|c| c.at(basis)).product(),
            Expression::Invert(child) => 1.0 / child.at(basis),
            Expression::Min(children) => children
                .iter()
                .map(|c| c.at(basis))
                .reduce(min)
                .unwrap_or(f64::NAN),
            Expression::Max(children) => children
                .iter()
                .map(|c| c.at(basis))
                .reduce(max)
                .unwrap_or(f64::NAN),
            Expression::Clamp(bounds) => {
                let [low, value, high] = &**bounds;
                clamp(low.at(basis), value.at(basis), high.at(basis))
            }
        }
    }

    /// Whether the expression has a percentage in it.
    fn has_percent(&self) -> bool {
        match self {
            Expression::Value(_) => false,
            Expression::Percent(_) => true,
            Expression::Negate(child) | Expression::Invert(child) => child.has_percent(),
            Expression::Sum(children)
            | Expression::Product(children)
            | Expression::Min(children)
            | Expression::Max(children) => children.iter().any(Expression::has_percent),
            Expression::Clamp(bounds) => bounds.iter().any(Expression::has_percent),
        }
    }
}

/// `min()` of two values by §10.9.2: `NaN` wins, and `-0` is below `0`.
fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a < b || (a == b && a.is_sign_negative()) {
        a
    } else {
        b
    }
}

/// `max()` of two values by §10.9.2.
fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a > b || (a == b && b.is_sign_negative()) {
        a
    } else {
        b
    }
}

/// `clamp(low, value, high)` (§10.4): `max(low, min(value, high))`,
/// the lower bound winning when the bounds cross.
fn clamp(low: f64, value: f64, high: f64) -> f64 {
    max(low, min(value, high))
}

/// How a length calculation's units become CSS pixels, given by the
/// cascade with the element's bases.
pub trait Lengths {
    /// `value` of `unit`, a relative unit, in CSS pixels.
    fn px(&self, value: f64, unit: LengthUnit) -> f64;
}

impl Parsed {
    /// Whether the calculation's type is a percentage, in a place that
    /// takes a number or a percentage: `opacity: calc(50% / 2)`.
    pub fn is_percentage(&self) -> bool {
        self.percentage
    }

    /// Computes the calculation for an element: its lengths resolved by
    /// `lengths`, and its percentages by `basis` where it is known.
    pub fn compute(&self, lengths: &dyn Lengths, basis: Option<f64>) -> Computed {
        let simplified = simplify(&self.expr, lengths, basis);
        match simplified {
            Folded::Value(v) => {
                let v = if v.is_nan() { 0.0 } else { v };
                if self.percentage {
                    Computed::Percent(v)
                } else {
                    Computed::Value(v)
                }
            }
            Folded::Tree(Expression::Percent(p)) => {
                Computed::Percent(if p.is_nan() { 0.0 } else { f64::from(p) })
            }
            Folded::Tree(tree) => Computed::Expression(tree),
        }
    }
}

/// A subtree simplified: its value, or what is left with a percentage
/// in it.
enum Folded {
    Value(f64),
    Tree(Expression),
}

impl Folded {
    fn into_expression(self) -> Expression {
        match self {
            Folded::Value(v) => Expression::Value(v as f32),
            Folded::Tree(tree) => tree,
        }
    }
}

/// §10.10.1's simplification, with what is known when a value is
/// computed. A percentage with no basis is the only leaf left unknown;
/// every subtree without one folds to its value, and a percentage's
/// products with numbers fold into it.
fn simplify(expr: &Expr, lengths: &dyn Lengths, basis: Option<f64>) -> Folded {
    match expr {
        Expr::Number(v) => Folded::Value(*v),
        Expr::Percent(p) => match basis {
            Some(basis) => Folded::Value(basis * p / 100.0),
            None => Folded::Tree(Expression::Percent(*p as f32)),
        },
        Expr::Dimension(v, unit) => Folded::Value(match unit {
            Unit::Length(unit) => match unit.absolute() {
                Some(px) => v * px,
                None => lengths.px(*v, *unit),
            },
            Unit::Angle(f) | Unit::Time(f) | Unit::Frequency(f) | Unit::Resolution(f) => v * f,
        }),
        Expr::Negate(child) => match simplify(child, lengths, basis) {
            Folded::Value(v) => Folded::Value(-v),
            Folded::Tree(Expression::Percent(p)) => Folded::Tree(Expression::Percent(-p)),
            Folded::Tree(Expression::Negate(inner)) => Folded::Tree(*inner),
            Folded::Tree(tree) => Folded::Tree(Expression::Negate(Box::new(tree))),
        },
        Expr::Invert(child) => match simplify(child, lengths, basis) {
            Folded::Value(v) => Folded::Value(1.0 / v),
            Folded::Tree(Expression::Invert(inner)) => Folded::Tree(*inner),
            Folded::Tree(tree) => Folded::Tree(Expression::Invert(Box::new(tree))),
        },
        Expr::Sum(children) => {
            let mut value = None::<f64>;
            let mut percent = None::<f64>;
            let mut rest = Vec::new();
            for child in children {
                match simplify(child, lengths, basis) {
                    Folded::Value(v) => *value.get_or_insert(0.0) += v,
                    Folded::Tree(Expression::Percent(p)) => {
                        *percent.get_or_insert(0.0) += f64::from(p)
                    }
                    Folded::Tree(Expression::Sum(inner)) => rest.extend(inner.into_vec()),
                    Folded::Tree(tree) => rest.push(tree),
                }
            }
            if rest.is_empty() && percent.is_none() {
                return Folded::Value(value.unwrap_or(0.0));
            }
            let mut terms = Vec::with_capacity(rest.len() + 2);
            terms.extend(percent.map(|p| Expression::Percent(p as f32)));
            terms.extend(value.map(|v| Expression::Value(v as f32)));
            terms.extend(rest);
            if terms.len() == 1 {
                Folded::Tree(terms.pop().expect("one term"))
            } else {
                Folded::Tree(Expression::Sum(terms.into_boxed_slice()))
            }
        }
        Expr::Product(children) => {
            let mut factor = 1.0;
            let mut rest = Vec::new();
            for child in children {
                match simplify(child, lengths, basis) {
                    Folded::Value(v) => factor *= v,
                    Folded::Tree(Expression::Product(inner)) => rest.extend(inner.into_vec()),
                    Folded::Tree(tree) => rest.push(tree),
                }
            }
            match rest.as_slice() {
                [] => Folded::Value(factor),
                // A percentage times numbers is a percentage, `50% * 2`
                // being `100%`; a sum of a percentage and a value
                // distributes the factor.
                [Expression::Percent(p)] => {
                    Folded::Tree(Expression::Percent((f64::from(*p) * factor) as f32))
                }
                [Expression::Sum(terms)] if terms.iter().all(is_leaf) => Folded::Tree(
                    Expression::Sum(terms.iter().map(|t| scale(t, factor)).collect()),
                ),
                _ => {
                    if factor != 1.0 {
                        rest.insert(0, Expression::Value(factor as f32));
                    }
                    if rest.len() == 1 {
                        Folded::Tree(rest.pop().expect("one factor"))
                    } else {
                        Folded::Tree(Expression::Product(rest.into_boxed_slice()))
                    }
                }
            }
        }
        Expr::Min(children) | Expr::Max(children) => {
            let is_min = matches!(expr, Expr::Min(_));
            let pick = if is_min { min } else { max };
            let mut value = None::<f64>;
            let mut rest = Vec::new();
            for child in children {
                match simplify(child, lengths, basis) {
                    Folded::Value(v) => value = Some(value.map_or(v, |w| pick(w, v))),
                    Folded::Tree(tree) => rest.push(tree),
                }
            }
            if rest.is_empty() {
                return Folded::Value(value.unwrap_or(f64::NAN));
            }
            rest.extend(value.map(|v| Expression::Value(v as f32)));
            if rest.len() == 1 {
                return Folded::Tree(rest.pop().expect("one argument"));
            }
            let rest = rest.into_boxed_slice();
            Folded::Tree(if is_min {
                Expression::Min(rest)
            } else {
                Expression::Max(rest)
            })
        }
        Expr::Clamp(low, value, high) => {
            let bound = |b: &Option<Box<Expr>>, none: f64| match b {
                Some(b) => simplify(b, lengths, basis),
                None => Folded::Value(none),
            };
            let low = bound(low, f64::NEG_INFINITY);
            let value = simplify(value, lengths, basis);
            let high = bound(high, f64::INFINITY);
            match (low, value, high) {
                (Folded::Value(l), Folded::Value(v), Folded::Value(h)) => {
                    Folded::Value(clamp(l, v, h))
                }
                (low, value, high) => Folded::Tree(Expression::Clamp(Box::new([
                    low.into_expression(),
                    value.into_expression(),
                    high.into_expression(),
                ]))),
            }
        }
    }
}

fn is_leaf(e: &Expression) -> bool {
    matches!(e, Expression::Value(_) | Expression::Percent(_))
}

fn scale(leaf: &Expression, factor: f64) -> Expression {
    match leaf {
        Expression::Value(v) => Expression::Value((f64::from(*v) * factor) as f32),
        Expression::Percent(p) => Expression::Percent((f64::from(*p) * factor) as f32),
        other => other.clone(),
    }
}

/// A length with a percentage in it, as a computed value holds it until
/// layout knows the percentage's basis: the simplified expression and
/// the range its place takes.
#[derive(Debug, Clone, PartialEq)]
pub struct Calc {
    expression: Expression,
    /// Whether the place takes no negative length (`padding`, `width`).
    non_negative: bool,
}

impl Calc {
    /// The computed expression for a place that takes negative lengths
    /// or not; an expression with no percentage in it is not one.
    pub fn new(expression: Expression, non_negative: bool) -> Calc {
        debug_assert!(expression.has_percent(), "{expression:?}");
        Calc {
            expression,
            non_negative,
        }
    }

    /// The length with `basis` for its percentages, in CSS pixels:
    /// `NaN` is 0, and the value is clamped to its place's range, which
    /// is at most [`MAX_LENGTH_PX`] either way (§10.9.2, §10.12).
    pub fn resolve(&self, basis: f32) -> f32 {
        let v = self.expression.evaluate(basis);
        let v = if v.is_nan() { 0.0 } else { v };
        let low = if self.non_negative {
            0.0
        } else {
            -f64::from(MAX_LENGTH_PX)
        };
        v.clamp(low, f64::from(MAX_LENGTH_PX)) as f32
    }

    /// The expression.
    pub fn expression(&self) -> &Expression {
        &self.expression
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lengths with `em` 16 pixels and `vw` 8 (an 800-pixel viewport);
    /// the other relative units are not used here.
    struct Bases;

    impl Lengths for Bases {
        fn px(&self, value: f64, unit: LengthUnit) -> f64 {
            match unit {
                LengthUnit::Em | LengthUnit::Rem => value * 16.0,
                LengthUnit::Vw => value * 8.0,
                _ => f64::NAN,
            }
        }
    }

    fn compute(source: &str, want: Want) -> Option<Computed> {
        parse(&css::tokenize(source), want).map(|p| p.compute(&Bases, None))
    }

    fn value(source: &str, want: Want) -> f64 {
        match compute(source, want) {
            Some(Computed::Value(v)) => v,
            other => panic!("{source}: {other:?}"),
        }
    }

    #[test]
    fn arithmetic_follows_the_grammar_and_precedence() {
        assert_eq!(value("calc(1px + 2px * 3)", Want::Length), 7.0);
        assert_eq!(value("calc((1px + 2px) * 3)", Want::Length), 9.0);
        assert_eq!(value("calc(10px - 2px - 3px)", Want::Length), 5.0);
        assert_eq!(value("calc(12px / 4 / 3)", Want::Length), 1.0);
        assert_eq!(value("calc(1px*3)", Want::Length), 3.0);
        assert_eq!(value("calc(1em + 2vw)", Want::Length), 32.0);
        assert_eq!(value("calc(1in - 90px)", Want::Length), 6.0);
        assert_eq!(value("CALC(2PX)", Want::Length), 2.0);
        assert_eq!(
            value("calc( calc(1px) + min(2px, 3px) )", Want::Length),
            3.0
        );
        assert_eq!(value("calc(1 / 3)", Want::Number), 1.0 / 3.0);
        assert_eq!(value("calc(e)", Want::Number), std::f64::consts::E);
        assert_eq!(value("calc(PI * 2)", Want::Number), std::f64::consts::TAU);
        assert_eq!(value("calc(0.5turn)", Want::Angle), 180.0);
        assert_eq!(value("calc(100ms + 1s)", Want::Time), 1.1);
        assert_eq!(value("calc(96dpi + 1x)", Want::Resolution), 2.0);
    }

    #[test]
    fn plus_and_minus_need_white_space_on_both_sides() {
        // §10.8: `+` and `-` must have white space on both sides; `*` and
        // `/` need none. `1px -2px` is two values with no operator.
        for bad in [
            "calc(1px -2px)",
            "calc(1px+2px)",
            "calc(1px - (2px)-3px)",
            "calc(1px +2px)",
            "calc(1px- 2px)",
        ] {
            assert_eq!(compute(bad, Want::Length), None, "{bad}");
        }
        assert_eq!(value("calc(1px - -2px)", Want::Length), 3.0);
    }

    #[test]
    fn types_are_checked_by_the_algebra() {
        // §10.9: a length and a number do not add, nor a length and a
        // time, whatever simplification would make of them; a product
        // and a quotient multiply their types, so a length over a length
        // is a number and a length times a length is no production.
        for (bad, want) in [
            ("calc(1px + 1)", Want::Length),
            ("calc(0 + 5px)", Want::Length),
            ("calc(5px - 5px + 10s)", Want::Length),
            ("calc(0 * 5px + 10s)", Want::Time),
            ("calc(1px * 1px)", Want::Length),
            ("calc(1px)", Want::Number),
            ("calc(50%)", Want::Length),
            ("calc(.25 + 25%)", Want::NumberOrPercentage),
            ("calc(1deg)", Want::Length),
            ("calc(1fr)", Want::Length),
            ("calc(1foo)", Want::Length),
            ("calc(bar)", Want::Number),
        ] {
            assert_eq!(compute(bad, want), None, "{bad}");
        }
        assert_eq!(value("calc(100px / 10px)", Want::Number), 10.0);
        assert_eq!(value("calc(10px * 2px / 4px)", Want::Length), 5.0);
        assert_eq!(value("calc(1em / 1px * 1deg)", Want::Angle), 16.0);
    }

    #[test]
    fn a_percentage_is_typed_by_its_place() {
        // In `width` a percentage is a length; in `opacity` it is a
        // percentage that stands for a number, never added to one.
        assert_eq!(
            compute("calc(50%)", Want::LengthPercentage),
            Some(Computed::Percent(50.0))
        );
        let Some(Computed::Percent(third)) = compute("calc(100% / 3)", Want::NumberOrPercentage)
        else {
            panic!("a percentage");
        };
        assert!((third - 100.0 / 3.0).abs() < 1e-5);
        assert_eq!(
            compute("calc(1 / 4)", Want::NumberOrPercentage),
            Some(Computed::Value(0.25))
        );
        assert_eq!(
            compute("calc(50% * 2)", Want::LengthPercentage),
            Some(Computed::Percent(100.0))
        );
        assert_eq!(
            compute("calc(20% + 30%)", Want::Percentage),
            Some(Computed::Percent(50.0))
        );
        // A known basis resolves the percentage with the rest: a font
        // size's percentage of its parent's.
        let parsed = parse(&css::tokenize("calc(50% + 2px)"), Want::LengthPercentage).unwrap();
        assert_eq!(parsed.compute(&Bases, Some(20.0)), Computed::Value(12.0));
    }

    #[test]
    fn a_length_with_a_percentage_keeps_its_expression() {
        let Some(Computed::Expression(e)) = compute("calc(100% - 2em)", Want::LengthPercentage)
        else {
            panic!("an expression");
        };
        assert_eq!(e.evaluate(200.0), 168.0);
        assert_eq!(
            e,
            Expression::Sum(Box::new([
                Expression::Percent(100.0),
                Expression::Value(-32.0)
            ]))
        );
        // min() of a length and a percentage stays, as the reference
        // browser keeps `min(10px, 5%)`; the lengths in it fold.
        let Some(Computed::Expression(e)) =
            compute("min(10px, 5%, 2px + 3px)", Want::LengthPercentage)
        else {
            panic!("an expression");
        };
        assert_eq!(
            e,
            Expression::Min(Box::new([Expression::Percent(5.0), Expression::Value(5.0)]))
        );
        assert_eq!(e.evaluate(784.0), 5.0);
        assert_eq!(e.evaluate(40.0), 2.0);
        // A product of a sum of leaves and a number distributes.
        let Some(Computed::Expression(e)) =
            compute("calc((10% + 1px) * 2)", Want::LengthPercentage)
        else {
            panic!("an expression");
        };
        assert_eq!(
            e,
            Expression::Sum(Box::new([
                Expression::Percent(20.0),
                Expression::Value(2.0)
            ]))
        );
        // The reference browser's own: `max(10px, min(20%, 300px))` at
        // 784 is 156.8; `calc(3px + 3%)` at 784 is 26.52.
        let width = |source: &str| match compute(source, Want::LengthPercentage) {
            Some(Computed::Expression(e)) => Calc::new(e, true).resolve(784.0),
            other => panic!("{other:?}"),
        };
        assert!((width("max(10px, min(20%, 300px))") - 156.8).abs() < 1e-3);
        assert!((width("calc(3px + 3%)") - 26.52).abs() < 1e-3);
        assert!((width("calc(min(100px, 50%) + 1px)") - 101.0).abs() < 1e-3);
        assert_eq!(width("calc(10% - 1000px)"), 0.0);
    }

    #[test]
    fn min_max_and_clamp() {
        assert_eq!(value("min(3px, 1px, 2px)", Want::Length), 1.0);
        assert_eq!(value("max(3px, 1px, 2px)", Want::Length), 3.0);
        assert_eq!(value("clamp(10px, 1000px, 50px)", Want::Length), 50.0);
        assert_eq!(value("clamp(10px, 1px, 50px)", Want::Length), 10.0);
        // Crossed bounds: the lower wins (§10.4).
        assert_eq!(value("clamp(50px, 30px, 10px)", Want::Length), 50.0);
        assert_eq!(value("clamp(none, 30px, 10px)", Want::Length), 10.0);
        assert_eq!(value("clamp(20px, 30px, none)", Want::Length), 30.0);
        assert_eq!(compute("clamp(1px, 2px)", Want::Length), None);
        assert_eq!(compute("min()", Want::Length), None);
        assert_eq!(compute("min(1px,)", Want::Length), None);
    }

    #[test]
    fn infinities_and_nan_are_censored_at_the_top() {
        // §10.9.2: an infinity is left for its place to clamp, `NaN` is
        // 0; inside the tree they behave as IEEE-754 says.
        assert_eq!(value("calc(1px / 0)", Want::Length), f64::INFINITY);
        assert_eq!(value("calc(-1px / 0)", Want::Length), f64::NEG_INFINITY);
        assert_eq!(value("calc(infinity * 1px)", Want::Length), f64::INFINITY);
        assert_eq!(
            value("calc(InFiNiTy * -1px)", Want::Length),
            f64::NEG_INFINITY
        );
        assert_eq!(value("calc(NaN * 1px)", Want::Length), 0.0);
        assert_eq!(value("calc(0px / 0)", Want::Length), 0.0);
        assert_eq!(value("min(NaN * 1px, 5px)", Want::Length), 0.0);
        assert_eq!(value("calc(1px / (1 / 0))", Want::Length), 0.0);
        let Some(Computed::Expression(e)) =
            compute("calc(infinity * 1px + 10%)", Want::LengthPercentage)
        else {
            panic!("an expression");
        };
        assert_eq!(Calc::new(e, false).resolve(100.0), MAX_LENGTH_PX);
        // A percentage alone is censored the same way: `NaN` is 0, and an
        // infinity is left for its place, where it is resolved.
        for (source, expected) in [
            ("calc(1% * NaN)", 0.0),
            ("calc(10% / 0 * 0)", 0.0),
            ("calc(1% * infinity)", f64::INFINITY),
            ("calc(-1% / 0)", f64::NEG_INFINITY),
        ] {
            assert_eq!(
                compute(source, Want::LengthPercentage),
                Some(Computed::Percent(expected)),
                "{source}"
            );
        }
    }

    #[test]
    fn nesting_is_bounded() {
        // §10.8: at least 32 levels; deeper is invalid rather than
        // recursed into.
        let nested = |depth: usize| format!("{}1px{}", "calc(".repeat(depth), ")".repeat(depth));
        assert_eq!(value(&nested(MAX_DEPTH), Want::Length), 1.0);
        assert_eq!(compute(&nested(MAX_DEPTH + 1), Want::Length), None);
        let parens = format!(
            "calc({}1px{})",
            "(".repeat(MAX_DEPTH),
            ")".repeat(MAX_DEPTH)
        );
        assert_eq!(compute(&parens, Want::Length), None);
        let deep = format!("calc({}1px{})", "(".repeat(100_000), ")".repeat(100_000));
        assert_eq!(compute(&deep, Want::Length), None);
    }

    #[test]
    fn only_one_math_function_is_a_value() {
        assert!(is_math_function(&css::tokenize(" calc(1px) ")));
        assert!(is_math_function(&css::tokenize("Min(1px, 2px)")));
        assert!(!is_math_function(&css::tokenize("rgb(1, 2, 3)")));
        assert_eq!(compute("calc(1px) calc(2px)", Want::Length), None);
        assert_eq!(compute("calc(1px", Want::Length), None);
        assert_eq!(compute("calc()", Want::Length), None);
        assert_eq!(compute("calc(1px 2px)", Want::Length), None);
        assert_eq!(compute("calc(var(--x))", Want::Length), None);
    }
}
