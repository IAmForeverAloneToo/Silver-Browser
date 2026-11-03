//! A math function as a specified value is written (css-values-4
//! sections 10.10 and 10.13, gap batch H, D78): simplified with what is
//! known without an element, then serialized. Numbers fold; a dimension
//! is made canonical where its unit is absolute (a length in `px`, an
//! angle in `deg`, a time in `s`, a frequency in `hz`, a resolution in
//! `dppx`) and kept in its own unit where it is relative to an element
//! (`em`, `vw`); the terms of a sum of one unit add up, a product's
//! numbers multiply into its one value, and a function of values it can
//! work out is its value. An infinite or NaN value is kept, written as
//! `infinity`, `-infinity` or `NaN`, a dimension's as that times one of
//! its unit (`calc(infinity * 1px)`), so that what a script wrote reads
//! back as Values 4 says. A relative colour's channel keywords (`r`,
//! `alpha`) are numbers kept by name.

use css::Token;

use super::{Expr, LengthUnit, Op, Parser, Rounding, Unit, Want, trim, type_of};

/// A leaf's unit as a specified value keeps it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Number,
    Percent,
    /// A dimension in its canonical unit, by its name.
    Canonical(&'static str),
    /// A length relative to an element, in its own unit.
    Relative(LengthUnit),
}

impl Kind {
    /// The unit's text after a value.
    fn unit(self) -> String {
        match self {
            Kind::Number => String::new(),
            Kind::Percent => "%".to_owned(),
            Kind::Canonical(unit) => unit.to_owned(),
            Kind::Relative(unit) => format!("{unit:?}").to_ascii_lowercase(),
        }
    }

    /// Where a sum's terms of this unit go (section 10.12's sort): a
    /// number, a percentage, then the dimensions by their units.
    fn rank(self) -> (u8, String) {
        match self {
            Kind::Number => (0, String::new()),
            Kind::Percent => (1, String::new()),
            other => (2, other.unit()),
        }
    }
}

/// A function a simplified calculation keeps.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Function {
    Min,
    Max,
    Clamp,
    Call(Op),
}

/// A calculation simplified as a specified value.
#[derive(Debug, Clone, PartialEq)]
enum Node {
    Value(f64, Kind),
    Keyword(usize),
    Sum(Vec<Node>),
    Product(Vec<Node>),
    Negate(Box<Node>),
    Invert(Box<Node>),
    /// A function of its arguments, a `clamp()` bound of `none` absent.
    Function(Function, Vec<Option<Node>>),
}

/// `tokens`, white space aside one math function, written as a specified
/// value is, `keywords` the channel keywords it may hold (none outside a
/// relative colour), `percentages` whether its percentages are compared
/// in `min()`, `max()` and `clamp()`: in a place where they stand for
/// numbers, as `opacity`'s do, not where their basis may be negative.
/// `None` when they are not one, it is malformed, or its type fails.
pub fn specified(tokens: &[Token], keywords: &[&str], percentages: bool) -> Option<String> {
    let tokens = trim(tokens);
    let mut p = Parser {
        tokens,
        pos: 0,
        keywords,
    };
    let expr = p.function(0)?;
    if p.pos != tokens.len() {
        return None;
    }
    type_of(&expr, Want::Percentage)?;
    let node = fold(&expr, percentages);
    // A `min()`, `max()` or `clamp()` left is written as itself (section
    // 10.13); another function left in a `calc()` stays in it, as the
    // reference browser keeps it.
    let in_calc =
        matches!(tokens.first(), Some(Token::Function(f)) if f.eq_ignore_ascii_case("calc"));
    Some(match &node {
        Node::Function(function @ Function::Call(_), args) if !in_calc => {
            function_text(*function, args, keywords)
        }
        Node::Function(function @ (Function::Min | Function::Max | Function::Clamp), args) => {
            function_text(*function, args, keywords)
        }
        other => format!("calc({})", text(other, keywords)),
    })
}

/// `expr` simplified.
fn fold(expr: &Expr, percentages: bool) -> Node {
    match expr {
        Expr::Number(v) => Node::Value(*v, Kind::Number),
        Expr::Percent(p) => Node::Value(*p, Kind::Percent),
        Expr::Dimension(v, unit) => match unit {
            Unit::Length(length) => match length.absolute() {
                Some(px) => Node::Value(v * px, Kind::Canonical("px")),
                None => Node::Value(*v, Kind::Relative(*length)),
            },
            Unit::Angle(f) => Node::Value(v * f, Kind::Canonical("deg")),
            Unit::Time(f) => Node::Value(v * f, Kind::Canonical("s")),
            Unit::Frequency(f) => Node::Value(v * f, Kind::Canonical("hz")),
            Unit::Resolution(f) => Node::Value(v * f, Kind::Canonical("dppx")),
        },
        Expr::Keyword(at) => Node::Keyword(*at),
        Expr::Negate(child) => match fold(child, percentages) {
            Node::Value(v, kind) => Node::Value(-v, kind),
            Node::Negate(inner) => *inner,
            other => Node::Negate(Box::new(other)),
        },
        Expr::Invert(child) => match fold(child, percentages) {
            Node::Value(v, Kind::Number) => Node::Value(1.0 / v, Kind::Number),
            Node::Invert(inner) => *inner,
            other => Node::Invert(Box::new(other)),
        },
        Expr::Sum(children) => sum(children.iter().map(|e| fold(e, percentages)).collect()),
        Expr::Product(children) => product(children.iter().map(|e| fold(e, percentages)).collect()),
        Expr::Min(args) => pick(
            Function::Min,
            args.iter().map(|e| fold(e, percentages)).collect(),
            percentages,
        ),
        Expr::Max(args) => pick(
            Function::Max,
            args.iter().map(|e| fold(e, percentages)).collect(),
            percentages,
        ),
        Expr::Clamp(low, value, high) => {
            let bound = |b: &Option<Box<Expr>>| b.as_deref().map(|b| fold(b, percentages));
            clamp(
                bound(low),
                fold(value, percentages),
                bound(high),
                percentages,
            )
        }
        Expr::Call(op, args) => call(*op, args.iter().map(|e| fold(e, percentages)).collect()),
    }
}

/// A sum of `terms`: a sum within its terms, and the values of one unit
/// added up.
fn sum(terms: Vec<Node>) -> Node {
    let mut out: Vec<Node> = Vec::new();
    let flat = terms.into_iter().flat_map(|t| match t {
        Node::Sum(inner) => inner,
        other => vec![other],
    });
    for term in flat {
        if let Node::Value(v, kind) = term
            && let Some(Node::Value(w, _)) = out
                .iter_mut()
                .find(|t| matches!(t, Node::Value(_, k) if *k == kind))
        {
            *w += v;
            continue;
        }
        out.push(term);
    }
    if out.len() == 1 {
        out.pop().expect("one term")
    } else {
        Node::Sum(out)
    }
}

/// A product of `factors`: a product within them, its numbers multiplied
/// into its one value, or into each value of its one sum of values.
fn product(factors: Vec<Node>) -> Node {
    let mut factor = 1.0;
    let mut numbers = false;
    let mut rest: Vec<Node> = Vec::new();
    let flat = factors.into_iter().flat_map(|f| match f {
        Node::Product(inner) => inner,
        other => vec![other],
    });
    for f in flat {
        match f {
            Node::Value(v, Kind::Number) => {
                factor *= v;
                numbers = true;
            }
            other => rest.push(other),
        }
    }
    match rest.as_slice() {
        [] => return Node::Value(factor, Kind::Number),
        [Node::Value(v, kind)] => return Node::Value(v * factor, *kind),
        [Node::Sum(terms)] if terms.iter().all(|t| matches!(t, Node::Value(..))) => {
            return sum(terms
                .iter()
                .map(|t| match t {
                    Node::Value(v, kind) => Node::Value(v * factor, *kind),
                    other => other.clone(),
                })
                .collect());
        }
        _ => {}
    }
    // The values first, the number among them, then the rest in order.
    let mut out = Vec::with_capacity(rest.len() + 1);
    if numbers {
        out.push(Node::Value(factor, Kind::Number));
    }
    let (values, others): (Vec<Node>, Vec<Node>) =
        rest.into_iter().partition(|f| matches!(f, Node::Value(..)));
    out.extend(values);
    out.extend(others);
    if out.len() == 1 {
        out.pop().expect("one factor")
    } else {
        Node::Product(out)
    }
}

/// The values of `args` when all are values of one unit.
fn values(args: &[Node]) -> Option<(Vec<f64>, Kind)> {
    let mut kind = None;
    let mut out = Vec::with_capacity(args.len());
    for arg in args {
        let Node::Value(v, k) = arg else {
            return None;
        };
        if kind.is_some_and(|kind| kind != *k) {
            return None;
        }
        kind = Some(*k);
        out.push(*v);
    }
    Some((out, kind?))
}

/// `min()` or `max()` of `args`: their value when they are values of one
/// unit, but percentages, whose basis is not known to be positive.
fn pick(function: Function, args: Vec<Node>, percentages: bool) -> Node {
    if let Some((vals, kind)) =
        values(&args).filter(|(_, kind)| percentages || *kind != Kind::Percent)
    {
        let pick = if function == Function::Min {
            super::min
        } else {
            super::max
        };
        if let Some(v) = vals.into_iter().reduce(pick) {
            return Node::Value(v, kind);
        }
    }
    if args.len() == 1 {
        return args.into_iter().next().expect("one argument");
    }
    Node::Function(function, args.into_iter().map(Some).collect())
}

/// `clamp()`: its value when its bounds and its value are values of one
/// unit, but percentages, a `none` bound an infinity of it.
fn clamp(low: Option<Node>, value: Node, high: Option<Node>, percentages: bool) -> Node {
    if let Node::Value(v, kind) = value
        && (percentages || kind != Kind::Percent)
    {
        let bound = |b: &Option<Node>, none: f64| match b {
            None => Some(none),
            Some(Node::Value(b, k)) if *k == kind => Some(*b),
            Some(_) => None,
        };
        if let (Some(l), Some(h)) = (bound(&low, f64::NEG_INFINITY), bound(&high, f64::INFINITY)) {
            return Node::Value(super::clamp(l, v, h), kind);
        }
    }
    Node::Function(Function::Clamp, vec![low, Some(value), high])
}

/// One of the other functions of `args`: its value where they are values
/// it takes.
fn call(op: Op, args: Vec<Node>) -> Node {
    let same = values(&args);
    let result = match (op, &same) {
        (Op::Sign, Some((v, kind))) if *kind != Kind::Percent => Some((op.apply(v), Kind::Number)),
        (Op::Abs | Op::Round(_) | Op::Mod | Op::Rem | Op::Hypot, Some((v, kind)))
            if *kind != Kind::Percent =>
        {
            Some((op.apply(v), *kind))
        }
        (
            Op::Sin(_) | Op::Cos(_) | Op::Tan(_),
            Some((v, Kind::Number | Kind::Canonical("deg"))),
        ) => Some((op.apply(v), Kind::Number)),
        (Op::Asin | Op::Acos | Op::Atan, Some((v, Kind::Number))) => {
            Some((op.apply(v), Kind::Canonical("deg")))
        }
        (Op::Atan2, Some((v, kind))) if *kind != Kind::Percent => {
            Some((op.apply(v), Kind::Canonical("deg")))
        }
        (Op::Pow | Op::Sqrt | Op::Log | Op::Exp, Some((v, Kind::Number))) => {
            Some((op.apply(v), Kind::Number))
        }
        _ => None,
    };
    match result {
        Some((v, kind)) => Node::Value(v, kind),
        None => Node::Function(Function::Call(op), args.into_iter().map(Some).collect()),
    }
}

/// A value written: a number at its shortest, with its unit; an infinite
/// or NaN one by its keyword, times one of its unit.
fn value_text(v: f64, kind: Kind) -> String {
    if v.is_finite() {
        return format!("{}{}", crate::color::number(v as f32), kind.unit());
    }
    let word = if v.is_nan() {
        "NaN"
    } else if v > 0.0 {
        "infinity"
    } else {
        "-infinity"
    };
    match kind {
        Kind::Number => word.to_owned(),
        other => format!("{word} * 1{}", other.unit()),
    }
}

/// `node` written inside a calculation.
fn text(node: &Node, keywords: &[&str]) -> String {
    match node {
        Node::Value(v, kind) => value_text(*v, *kind),
        Node::Keyword(at) => keywords.get(*at).copied().unwrap_or_default().to_owned(),
        Node::Sum(terms) => {
            let mut sorted: Vec<&Node> = terms.iter().collect();
            sorted.sort_by_key(|t| match t {
                Node::Value(_, kind) => kind.rank(),
                _ => (3, String::new()),
            });
            let mut out = String::new();
            for (i, term) in sorted.into_iter().enumerate() {
                let (minus, written) = match term {
                    Node::Negate(inner) => (true, grouped(inner, keywords)),
                    Node::Value(v, kind) if *v < 0.0 => (true, value_text(-v, *kind)),
                    other => (false, grouped(other, keywords)),
                };
                match (i, minus) {
                    (0, true) => out.push('-'),
                    (0, false) => {}
                    (_, true) => out.push_str(" - "),
                    (_, false) => out.push_str(" + "),
                }
                out.push_str(&written);
            }
            out
        }
        Node::Product(factors) => {
            let mut out = String::new();
            for (i, factor) in factors.iter().enumerate() {
                match factor {
                    Node::Invert(inner) => {
                        out.push_str(if i == 0 { "1 / " } else { " / " });
                        out.push_str(&grouped(inner, keywords));
                    }
                    other => {
                        if i > 0 {
                            out.push_str(" * ");
                        }
                        out.push_str(&grouped(other, keywords));
                    }
                }
            }
            out
        }
        Node::Negate(inner) => format!("-1 * {}", grouped(inner, keywords)),
        Node::Invert(inner) => format!("1 / {}", grouped(inner, keywords)),
        Node::Function(function, args) => function_text(*function, args, keywords),
    }
}

/// `node` written as an operand: a sum or a product in parentheses.
fn grouped(node: &Node, keywords: &[&str]) -> String {
    match node {
        Node::Sum(_) | Node::Product(_) | Node::Negate(_) | Node::Invert(_) => {
            format!("({})", text(node, keywords))
        }
        Node::Value(v, kind) if !v.is_finite() && *kind != Kind::Number => {
            format!("({})", text(node, keywords))
        }
        other => text(other, keywords),
    }
}

/// A function and its arguments written.
fn function_text(function: Function, args: &[Option<Node>], keywords: &[&str]) -> String {
    let name = match function {
        Function::Min => "min",
        Function::Max => "max",
        Function::Clamp => "clamp",
        Function::Call(op) => match op {
            Op::Round(_) => "round",
            Op::Mod => "mod",
            Op::Rem => "rem",
            Op::Abs => "abs",
            Op::Sign => "sign",
            Op::Hypot => "hypot",
            Op::Sin(_) => "sin",
            Op::Cos(_) => "cos",
            Op::Tan(_) => "tan",
            Op::Asin => "asin",
            Op::Acos => "acos",
            Op::Atan => "atan",
            Op::Atan2 => "atan2",
            Op::Pow => "pow",
            Op::Sqrt => "sqrt",
            Op::Log => "log",
            Op::Exp => "exp",
        },
    };
    let strategy = match function {
        Function::Call(Op::Round(Rounding::Up)) => "up, ",
        Function::Call(Op::Round(Rounding::Down)) => "down, ",
        Function::Call(Op::Round(Rounding::ToZero)) => "to-zero, ",
        _ => "",
    };
    let args: Vec<String> = args
        .iter()
        .map(|arg| match arg {
            Some(node) => text(node, keywords),
            None => "none".to_owned(),
        })
        .collect();
    format!("{name}({strategy}{})", args.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(source: &str) -> String {
        specified(&css::tokenize(source), &[], false).unwrap_or_else(|| panic!("{source}"))
    }

    #[test]
    fn a_calculation_is_simplified_and_written_as_values_4_writes_it() {
        assert_eq!(written("calc(1px + 2px)"), "calc(3px)");
        assert_eq!(written("calc(1in + 2em)"), "calc(2em + 96px)");
        assert_eq!(written("calc(2em - 10px + 1em)"), "calc(3em - 10px)");
        assert_eq!(written("calc(2 * (1px + 2em))"), "calc(4em + 2px)");
        assert_eq!(
            written("calc(50% + (sign(1em - 10px) * 10%))"),
            "calc(50% + (10% * sign(1em - 10px)))"
        );
        assert_eq!(written("min(1px, 2px)"), "calc(1px)");
        assert_eq!(written("min(1px, 2em)"), "min(1px, 2em)");
        assert_eq!(
            written("calc(pow(2, sign(1em - 18px)))"),
            "calc(pow(2, sign(1em - 18px)))"
        );
        assert_eq!(written("sin(0)"), "calc(0)");
        assert_eq!(written("calc(1ms)"), "calc(0.001s)");
        assert_eq!(written("round(up, 1.5em, 1em)"), "calc(2em)");
        assert_eq!(written("round(up, 1.5em, 1px)"), "round(up, 1.5em, 1px)");
        assert_eq!(written("clamp(none, 5px, 2px)"), "calc(2px)");
        // Percentages are compared where they stand for numbers alone.
        assert_eq!(written("min(1%, 2%)"), "min(1%, 2%)");
        let numbers = specified(&css::tokenize("min(1%, 2%)"), &[], true);
        assert_eq!(numbers.as_deref(), Some("calc(1%)"));
    }

    #[test]
    fn infinity_and_nan_are_kept_to_the_text() {
        assert_eq!(written("calc(1px * NaN)"), "calc(NaN * 1px)");
        assert_eq!(
            written("calc(1px * infinity / infinity)"),
            "calc(NaN * 1px)"
        );
        assert_eq!(written("calc(1% * -infinity)"), "calc(-infinity * 1%)");
        assert_eq!(written("calc(infinity)"), "calc(infinity)");
        assert_eq!(written("calc(0 / 0)"), "calc(NaN)");
        assert_eq!(written("calc(1px * 1/infinity)"), "calc(0px)");
        assert_eq!(
            written("calc(1 * min(NaN * 2px, NaN * 4em))"),
            "calc(1 * min(NaN * 1px, NaN * 1em))"
        );
        assert_eq!(
            written("calc(1px * clamp(-inFinity, infinity, 10))"),
            "calc(10px)"
        );
    }

    #[test]
    fn a_relative_colours_channel_keywords_are_kept_by_name() {
        let keywords = ["l", "a", "b", "alpha"];
        let written =
            |source: &str| specified(&css::tokenize(source), &keywords, true).expect("parses");
        assert_eq!(written("calc(l - 20)"), "calc(-20 + l)");
        assert_eq!(written("calc(a / 3)"), "calc(0.333333 * a)");
        assert_eq!(
            written("calc(b * .5 - a * .5)"),
            "calc((0.5 * b) - (0.5 * a))"
        );
        assert_eq!(written("calc(1 / l)"), "calc(1 / l)");
        assert_eq!(written("calc(ALPHA)"), "calc(alpha)");
        assert!(specified(&css::tokenize("calc(l)"), &[], true).is_none());
    }
}
