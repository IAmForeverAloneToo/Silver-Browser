//! Expressions (GLSL ES 1.00 and 3.00, section 5): their grammar, their
//! types by the language's rules (no implicit conversions), the l-values
//! assignments need, and the values of constant expressions, folded as
//! they are read.

use crate::Error;
use crate::builtins;
use crate::lex::Tok;
use crate::parse::{Parser, Var, int_literal};
use crate::tir::*;

type R<T> = Result<T, Error>;

fn expr(kind: ExprKind, ty: Type, line: u32) -> Expr {
    Expr {
        kind,
        ty,
        value: None,
        line,
    }
}

fn with_value(mut e: Expr, value: Option<Vec<Comp>>) -> Expr {
    e.value = value;
    e
}

/// The binary operators by precedence, the loosest first.
const LEVELS: &[&[&str]] = &[
    &["||"],
    &["^^"],
    &["&&"],
    &["|"],
    &["^"],
    &["&"],
    &["==", "!="],
    &["<", ">", "<=", ">="],
    &["<<", ">>"],
    &["+", "-"],
    &["*", "/", "%"],
];

fn bin_op(punct: &str) -> Option<BinOp> {
    Some(match punct {
        "+" => BinOp::Add,
        "-" => BinOp::Sub,
        "*" => BinOp::Mul,
        "/" => BinOp::Div,
        "%" => BinOp::Rem,
        "<<" => BinOp::Shl,
        ">>" => BinOp::Shr,
        "&" => BinOp::BitAnd,
        "|" => BinOp::BitOr,
        "^" => BinOp::BitXor,
        "<" => BinOp::Lt,
        ">" => BinOp::Gt,
        "<=" => BinOp::Le,
        ">=" => BinOp::Ge,
        "==" => BinOp::Eq,
        "!=" => BinOp::Ne,
        _ => return None,
    })
}

pub fn op_text(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Lt => "<",
        BinOp::Gt => ">",
        BinOp::Le => "<=",
        BinOp::Ge => ">=",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
    }
}

/// Whether an expression could change something: an assignment, an
/// increment, or a call of the shader's own functions.
pub fn has_effects(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Assign(..) | ExprKind::IncDec { .. } | ExprKind::Call(..) => true,
        ExprKind::Lit(_) | ExprKind::Global(_) | ExprKind::Local(_) | ExprKind::Param(_) => false,
        ExprKind::Index(a, b)
        | ExprKind::Binary(_, a, b)
        | ExprKind::And(a, b)
        | ExprKind::Or(a, b)
        | ExprKind::Xor(a, b)
        | ExprKind::Comma(a, b) => has_effects(a) || has_effects(b),
        ExprKind::Field(a, _) | ExprKind::Swizzle(a, _) | ExprKind::Unary(_, a) => has_effects(a),
        ExprKind::Select(a, b, c) => has_effects(a) || has_effects(b) || has_effects(c),
        ExprKind::Builtin(name, args) => *name == "modf" || args.iter().any(has_effects),
        ExprKind::Construct(args) => args.iter().any(has_effects),
    }
}

impl Parser<'_> {
    pub fn expression(&mut self) -> R<Expr> {
        let mut e = self.assignment()?;
        while self.is(",") {
            let line = self.line();
            self.next()?;
            let right = self.assignment()?;
            let ty = right.ty.clone();
            let value = right.value.clone().filter(|_| e.value.is_some());
            e = with_value(
                expr(ExprKind::Comma(Box::new(e), Box::new(right)), ty, line),
                value,
            );
            // A comma's value is no constant expression in GLSL ES.
            e.value = None;
        }
        Ok(e)
    }

    pub fn assignment(&mut self) -> R<Expr> {
        self.deeper()?;
        let result = self.assignment_inner();
        self.shallower();
        result
    }

    fn assignment_inner(&mut self) -> R<Expr> {
        let target = self.conditional()?;
        let Some(token) = self.peek().cloned() else {
            return Ok(target);
        };
        let op = match &token.tok {
            Tok::Punct("=") => None,
            Tok::Punct(p)
                if p.len() >= 2 && p.ends_with('=') && !matches!(*p, "==" | "!=" | "<=" | ">=") =>
            {
                Some(bin_op(&p[..p.len() - 1]).expect("an operator"))
            }
            _ => return Ok(target),
        };
        let line = token.line;
        self.next()?;
        let value = self.assignment()?;
        let what = if op.is_some() {
            token.text()
        } else {
            "assign".to_owned()
        };
        self.check_lvalue(&target, &what, line)?;
        if let Some(op) = op {
            if self.ctx.version == 100
                && matches!(
                    op,
                    BinOp::Rem
                        | BinOp::Shl
                        | BinOp::Shr
                        | BinOp::BitAnd
                        | BinOp::BitOr
                        | BinOp::BitXor
                )
            {
                return Err(Error::new(line, &token.text(), "reserved operator"));
            }
            let result = self.binary_type(op, &target.ty, &value.ty).map_err(|_| {
                Error::new(
                    line,
                    &token.text(),
                    &format!(
                        "wrong operand types - no operation '{}' exists that takes a left-hand operand of type '{}' and a right operand of type '{}' (or there is no acceptable conversion)",
                        token.text(),
                        target.ty.name(&self.unit),
                        value.ty.name(&self.unit)
                    ),
                )
            })?;
            if result != target.ty {
                return Err(Error::new(
                    line,
                    &token.text(),
                    &format!(
                        "cannot convert from '{}' to '{}'",
                        result.name(&self.unit),
                        target.ty.name(&self.unit)
                    ),
                ));
            }
        } else if value.ty != target.ty {
            return Err(Error::new(
                line,
                "=",
                &format!(
                    "cannot convert from '{}' to '{}'",
                    value.ty.name(&self.unit),
                    target.ty.name(&self.unit)
                ),
            ));
        }
        if target.ty.has_sampler(&self.unit) {
            return Err(Error::new(line, "=", "samplers cannot be assigned"));
        }
        if self.ctx.version == 100 && target.ty.has_array(&self.unit) {
            return Err(Error::new(
                line,
                "=",
                "arrays cannot be assigned in GLSL ES 1.00",
            ));
        }
        let ty = target.ty.clone();
        Ok(expr(
            ExprKind::Assign(op, Box::new(target), Box::new(value)),
            ty,
            line,
        ))
    }

    /// Refuses what may not be assigned: a constant, a uniform, an input,
    /// a swizzle naming a component twice, a 1.00 loop's index.
    pub fn check_lvalue(&self, e: &Expr, what: &str, line: u32) -> R<()> {
        let refuse = |why: &str| Err(Error::new(line, what, &format!("l-value required ({why})")));
        match &e.kind {
            ExprKind::Global(g) => {
                let global = &self.unit.globals[*g];
                match &global.kind {
                    GlobalKind::Const => {
                        refuse(&format!("can't modify a const variable '{}'", global.name))
                    }
                    GlobalKind::Uniform => {
                        refuse(&format!("can't modify a uniform '{}'", global.name))
                    }
                    GlobalKind::Attribute(_) => {
                        refuse(&format!("can't modify an attribute '{}'", global.name))
                    }
                    GlobalKind::Varying { output: false, .. } => {
                        refuse(&format!("can't modify a varying '{}'", global.name))
                    }
                    GlobalKind::Builtin(
                        BuiltinVar::FragCoord
                        | BuiltinVar::FrontFacing
                        | BuiltinVar::PointCoord
                        | BuiltinVar::VertexId
                        | BuiltinVar::InstanceId
                        | BuiltinVar::DepthRange,
                    ) => refuse(&format!("can't modify '{}'", global.name)),
                    _ => Ok(()),
                }
            }
            ExprKind::Local(l) => {
                if self.in_function() && self.current().locals[*l].value.is_some() {
                    return refuse(&format!(
                        "can't modify a const variable '{}'",
                        self.current().locals[*l].name
                    ));
                }
                if self.is_loop_index(Var::Local(*l)) {
                    return Err(Error::new(
                        line,
                        what,
                        "Loop index cannot be statically assigned to within the body of the loop",
                    ));
                }
                Ok(())
            }
            ExprKind::Param(_) => Ok(()),
            ExprKind::Index(base, _) | ExprKind::Field(base, _) => {
                self.check_lvalue(base, what, line)
            }
            ExprKind::Swizzle(base, comps) => {
                let mut seen = [false; 4];
                for &c in comps {
                    if seen[c as usize] {
                        return refuse("vector components may not be repeated");
                    }
                    seen[c as usize] = true;
                }
                self.check_lvalue(base, what, line)
            }
            _ => refuse("not an l-value"),
        }
    }

    fn conditional(&mut self) -> R<Expr> {
        let cond = self.binary(0)?;
        if !self.is("?") {
            return Ok(cond);
        }
        let line = self.line();
        self.next()?;
        if cond.ty != BOOL {
            return Err(Error::new(line, "?", "boolean expression expected"));
        }
        let a = self.assignment()?;
        self.expect(":")?;
        let b = self.assignment()?;
        if a.ty != b.ty {
            return Err(Error::new(
                line,
                ":",
                "wrong operand types - no operation ':' exists that takes a left-hand operand of the true expression's type and a right operand of the false expression's (or there is no acceptable conversion)",
            ));
        }
        if a.ty.has_sampler(&self.unit) || (self.ctx.version == 100 && a.ty.has_array(&self.unit)) {
            return Err(Error::new(
                line,
                "?",
                "ternary operator is not allowed for this type",
            ));
        }
        let value = match (&cond.value, &a.value, &b.value) {
            (Some(c), Some(x), Some(y)) => Some(if c[0].as_bool() { x.clone() } else { y.clone() }),
            _ => None,
        };
        let ty = a.ty.clone();
        Ok(with_value(
            expr(
                ExprKind::Select(Box::new(cond), Box::new(a), Box::new(b)),
                ty,
                line,
            ),
            value,
        ))
    }

    fn binary(&mut self, level: usize) -> R<Expr> {
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        loop {
            let Some(token) = self.peek().cloned() else {
                return Ok(left);
            };
            let Tok::Punct(p) = token.tok else {
                return Ok(left);
            };
            if !LEVELS[level].contains(&p) {
                return Ok(left);
            }
            let line = token.line;
            self.next()?;
            let right = self.binary(level + 1)?;
            left = self.binary_expr(p, left, right, line)?;
        }
    }

    fn binary_expr(&self, p: &'static str, left: Expr, right: Expr, line: u32) -> R<Expr> {
        let wrong = || {
            Error::new(
                line,
                p,
                &format!(
                    "wrong operand types - no operation '{p}' exists that takes a left-hand operand of type '{}' and a right operand of type '{}' (or there is no acceptable conversion)",
                    left.ty.name(&self.unit),
                    right.ty.name(&self.unit)
                ),
            )
        };
        match p {
            "&&" | "||" | "^^" => {
                if left.ty != BOOL || right.ty != BOOL {
                    return Err(wrong());
                }
                let value = match (&left.value, &right.value) {
                    (Some(a), Some(b)) => {
                        let (a, b) = (a[0].as_bool(), b[0].as_bool());
                        Some(vec![Comp::B(match p {
                            "&&" => a && b,
                            "||" => a || b,
                            _ => a != b,
                        })])
                    }
                    _ => None,
                };
                let kind = match p {
                    "&&" => ExprKind::And(Box::new(left), Box::new(right)),
                    "||" => ExprKind::Or(Box::new(left), Box::new(right)),
                    _ => ExprKind::Xor(Box::new(left), Box::new(right)),
                };
                return Ok(with_value(expr(kind, BOOL, line), value));
            }
            _ => {}
        }
        let op = bin_op(p).expect("an operator");
        if self.ctx.version == 100
            && matches!(
                op,
                BinOp::Rem | BinOp::Shl | BinOp::Shr | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor
            )
        {
            return Err(Error::new(line, p, "reserved operator"));
        }
        let ty = self
            .binary_type(op, &left.ty, &right.ty)
            .map_err(|_| wrong())?;
        if matches!(op, BinOp::Eq | BinOp::Ne)
            && (left.ty.has_sampler(&self.unit)
                || (self.ctx.version == 100 && left.ty.has_array(&self.unit)))
        {
            return Err(wrong());
        }
        let value = match (&left.value, &right.value) {
            (Some(a), Some(b)) => self.fold_binary(op, &left.ty, a, &right.ty, b, &ty, line)?,
            _ => None,
        };
        Ok(with_value(
            expr(
                ExprKind::Binary(op, Box::new(left), Box::new(right)),
                ty,
                line,
            ),
            value,
        ))
    }

    /// The type `a op b` has, by GLSL ES's rules.
    pub fn binary_type(&self, op: BinOp, a: &Type, b: &Type) -> Result<Type, ()> {
        use Type::*;
        match op {
            BinOp::Eq | BinOp::Ne => {
                if a == b && *a != Type::Void {
                    Ok(BOOL)
                } else {
                    Err(())
                }
            }
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => match (a, b) {
                (Scalar(x), Scalar(y)) if x == y && *x != crate::tir::Scalar::Bool => Ok(BOOL),
                _ => Err(()),
            },
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                let numeric = |s: &crate::tir::Scalar| *s != crate::tir::Scalar::Bool;
                match (a, b) {
                    (Scalar(x), Scalar(y)) if x == y && numeric(x) => Ok(a.clone()),
                    (Vector(x, n), Scalar(y)) | (Scalar(y), Vector(x, n))
                        if x == y && numeric(x) =>
                    {
                        Ok(Vector(*x, *n))
                    }
                    (Vector(x, n), Vector(y, m)) if x == y && n == m && numeric(x) => Ok(a.clone()),
                    (Matrix(..), Scalar(crate::tir::Scalar::Float))
                    | (Scalar(crate::tir::Scalar::Float), Matrix(..)) => {
                        Ok(if a.is_matrix() { a.clone() } else { b.clone() })
                    }
                    (Matrix(c1, r1), Matrix(c2, r2)) if op == BinOp::Mul => {
                        if c1 == r2 {
                            Ok(Matrix(*c2, *r1))
                        } else {
                            Err(())
                        }
                    }
                    (Matrix(c1, r1), Matrix(c2, r2)) => {
                        if c1 == c2 && r1 == r2 {
                            Ok(a.clone())
                        } else {
                            Err(())
                        }
                    }
                    (Matrix(c, r), Vector(crate::tir::Scalar::Float, n))
                        if op == BinOp::Mul && c == n =>
                    {
                        Ok(Vector(crate::tir::Scalar::Float, *r))
                    }
                    (Vector(crate::tir::Scalar::Float, n), Matrix(c, r))
                        if op == BinOp::Mul && r == n =>
                    {
                        Ok(Vector(crate::tir::Scalar::Float, *c))
                    }
                    _ => Err(()),
                }
            }
            BinOp::Rem | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                let integral = |s: &crate::tir::Scalar| {
                    matches!(s, crate::tir::Scalar::Int | crate::tir::Scalar::Uint)
                };
                match (a, b) {
                    (Scalar(x), Scalar(y)) if x == y && integral(x) => Ok(a.clone()),
                    (Vector(x, n), Scalar(y)) | (Scalar(y), Vector(x, n))
                        if x == y && integral(x) =>
                    {
                        Ok(Vector(*x, *n))
                    }
                    (Vector(x, n), Vector(y, m)) if x == y && n == m && integral(x) => {
                        Ok(a.clone())
                    }
                    _ => Err(()),
                }
            }
            BinOp::Shl | BinOp::Shr => {
                let integral = |t: &Type| {
                    matches!(
                        t.scalar(),
                        Some(crate::tir::Scalar::Int | crate::tir::Scalar::Uint)
                    ) && t.is_scalar_or_vector()
                };
                if !integral(a) || !integral(b) {
                    return Err(());
                }
                match (a, b) {
                    (Scalar(_), Scalar(_)) | (Vector(..), Scalar(_)) => Ok(a.clone()),
                    (Vector(_, n), Vector(_, m)) if n == m => Ok(a.clone()),
                    _ => Err(()),
                }
            }
        }
    }

    fn unary(&mut self) -> R<Expr> {
        self.deeper()?;
        let result = self.unary_inner();
        self.shallower();
        result
    }

    fn unary_inner(&mut self) -> R<Expr> {
        let Some(token) = self.peek().cloned() else {
            return Err(self.syntax());
        };
        let line = token.line;
        match &token.tok {
            Tok::Punct("++" | "--") => {
                self.next()?;
                let target = self.unary()?;
                return self.inc_dec(target, true, token.is("++"), line);
            }
            Tok::Punct("+") => {
                self.next()?;
                let e = self.unary()?;
                if !e.ty.scalar().is_some_and(|s| s != Scalar::Bool) {
                    return Err(Error::new(
                        line,
                        "+",
                        "wrong operand type - no operation '+' exists that takes an operand of this type",
                    ));
                }
                return Ok(e);
            }
            Tok::Punct("-") => {
                self.next()?;
                let e = self.unary()?;
                if !e.ty.scalar().is_some_and(|s| s != Scalar::Bool) {
                    return Err(Error::new(
                        line,
                        "-",
                        &format!(
                            "wrong operand type - no operation '-' exists that takes an operand of type {}",
                            e.ty.name(&self.unit)
                        ),
                    ));
                }
                let value = e.value.as_ref().map(|v| {
                    v.iter()
                        .map(|c| match *c {
                            Comp::F(x) => Comp::F(-x),
                            Comp::I(x) => Comp::I(x.wrapping_neg()),
                            Comp::U(x) => Comp::U(x.wrapping_neg()),
                            other => other,
                        })
                        .collect()
                });
                let ty = e.ty.clone();
                return Ok(with_value(
                    expr(ExprKind::Unary(UnOp::Neg, Box::new(e)), ty, line),
                    value,
                ));
            }
            Tok::Punct("!") => {
                self.next()?;
                let e = self.unary()?;
                if e.ty != BOOL {
                    return Err(Error::new(
                        line,
                        "!",
                        "wrong operand type - no operation '!' exists that takes an operand of this type",
                    ));
                }
                let value = e.value.as_ref().map(|v| vec![Comp::B(!v[0].as_bool())]);
                return Ok(with_value(
                    expr(ExprKind::Unary(UnOp::Not, Box::new(e)), BOOL, line),
                    value,
                ));
            }
            Tok::Punct("~") => {
                self.next()?;
                if self.ctx.version == 100 {
                    return Err(Error::new(line, "~", "reserved operator"));
                }
                let e = self.unary()?;
                if !matches!(e.ty.scalar(), Some(Scalar::Int | Scalar::Uint))
                    || !e.ty.is_scalar_or_vector()
                {
                    return Err(Error::new(
                        line,
                        "~",
                        "wrong operand type - no operation '~' exists that takes an operand of this type",
                    ));
                }
                let value = e.value.as_ref().map(|v| {
                    v.iter()
                        .map(|c| match *c {
                            Comp::I(x) => Comp::I(!x),
                            Comp::U(x) => Comp::U(!x),
                            other => other,
                        })
                        .collect()
                });
                let ty = e.ty.clone();
                return Ok(with_value(
                    expr(ExprKind::Unary(UnOp::BitNot, Box::new(e)), ty, line),
                    value,
                ));
            }
            _ => {}
        }
        self.postfix()
    }

    fn inc_dec(&self, target: Expr, pre: bool, inc: bool, line: u32) -> R<Expr> {
        let op = if inc { "++" } else { "--" };
        if !target.ty.scalar().is_some_and(|s| s != Scalar::Bool) {
            return Err(Error::new(
                line,
                op,
                "wrong operand type - no operation exists that takes an operand of this type",
            ));
        }
        self.check_lvalue(&target, op, line)?;
        let ty = target.ty.clone();
        Ok(expr(
            ExprKind::IncDec {
                pre,
                inc,
                target: Box::new(target),
            },
            ty,
            line,
        ))
    }

    fn postfix(&mut self) -> R<Expr> {
        let mut e = self.primary()?;
        loop {
            let line = self.line();
            if self.eat("[") {
                let index = self.expression()?;
                self.expect("]")?;
                e = self.index(e, index, line)?;
            } else if self.eat(".") {
                let token = self.next()?;
                let Some(name) = token.ident().map(str::to_owned) else {
                    return Err(Error::new(token.line, &token.text(), "syntax error"));
                };
                // `.length()` of an array (GLSL ES 3.00).
                if name == "length" && self.is("(") {
                    let Type::Array(_, n) = e.ty else {
                        return Err(Error::new(
                            line,
                            "length",
                            "length can only be called on arrays",
                        ));
                    };
                    if self.ctx.version == 100 {
                        return Err(Error::new(
                            line,
                            "length",
                            "methods are not supported in GLSL ES 1.00",
                        ));
                    }
                    self.expect("(")?;
                    self.expect(")")?;
                    // Its value, and its side effects (`a[i++].length()`)
                    // kept by a comma.
                    let length = with_value(
                        expr(ExprKind::Lit(Comp::I(n as i32)), INT, line),
                        Some(vec![Comp::I(n as i32)]),
                    );
                    e = if crate::expr::has_effects(&e) {
                        expr(ExprKind::Comma(Box::new(e), Box::new(length)), INT, line)
                    } else {
                        length
                    };
                    continue;
                }
                e = self.field(e, &name, line)?;
            } else if self.is("++") || self.is("--") {
                let inc = self.is("++");
                self.next()?;
                e = self.inc_dec(e, false, inc, line)?;
            } else {
                return Ok(e);
            }
        }
    }

    fn index(&self, base: Expr, index: Expr, line: u32) -> R<Expr> {
        let int = matches!(index.ty, Type::Scalar(Scalar::Int))
            || (self.ctx.version == 300 && index.ty == UINT);
        if !int {
            return Err(Error::new(line, "[", "integer expression required"));
        }
        let (ty, bound) = match &base.ty {
            Type::Array(element, n) => ((**element).clone(), *n),
            Type::Vector(s, n) => (Type::Scalar(*s), u32::from(*n)),
            Type::Matrix(c, r) => (Type::Vector(Scalar::Float, *r), u32::from(*c)),
            _ => {
                return Err(Error::new(
                    line,
                    "[",
                    " left of '[' is not of type array, matrix, or vector  ",
                ));
            }
        };
        if let Some(v) = &index.value {
            let i = v[0].as_i32();
            if i < 0 || i as u32 >= bound {
                return Err(Error::new(
                    line,
                    "[",
                    &format!("array index out of range '{i}'"),
                ));
            }
        } else if base.ty.has_sampler(&self.unit) && !self.constant_index(&index) {
            return Err(Error::new(
                line,
                "[",
                "An array of samplers can only be indexed with a constant index expression",
            ));
        }
        let value = match (&base.value, &index.value) {
            (Some(b), Some(i)) => {
                let per = self.flat_size(&ty) as usize;
                let at = i[0].as_i32() as usize * per;
                b.get(at..at + per).map(<[Comp]>::to_vec)
            }
            _ => None,
        };
        Ok(with_value(
            expr(ExprKind::Index(Box::new(base), Box::new(index)), ty, line),
            value,
        ))
    }

    /// Whether an index is a constant-index-expression (GLSL ES 1.00,
    /// Appendix A): constants and the loops' indices.
    fn constant_index(&self, e: &Expr) -> bool {
        if e.value.is_some() {
            return true;
        }
        if self.ctx.version == 300 {
            return false;
        }
        match &e.kind {
            ExprKind::Local(l) => self.is_loop_index(Var::Local(*l)),
            ExprKind::Binary(_, a, b) => self.constant_index(a) && self.constant_index(b),
            ExprKind::Unary(_, a) => self.constant_index(a),
            ExprKind::Construct(args) | ExprKind::Builtin(_, args) => {
                args.iter().all(|a| self.constant_index(a))
            }
            _ => false,
        }
    }

    /// The components a type flattens to in a constant.
    pub fn flat_size(&self, ty: &Type) -> u32 {
        match ty {
            Type::Array(element, n) => self.flat_size(element) * n,
            Type::Struct(s) => self.unit.structs[*s]
                .fields
                .iter()
                .map(|(_, t)| self.flat_size(t))
                .sum(),
            other => other.components(),
        }
    }

    fn field(&self, base: Expr, name: &str, line: u32) -> R<Expr> {
        match &base.ty {
            Type::Struct(s) => {
                let def = &self.unit.structs[*s];
                let Some(i) = def.fields.iter().position(|(n, _)| n == name) else {
                    return Err(Error::new(line, name, " no such field in structure"));
                };
                let ty = def.fields[i].1.clone();
                let value = base.value.as_ref().and_then(|v| {
                    let before: u32 = def.fields[..i].iter().map(|(_, t)| self.flat_size(t)).sum();
                    let size = self.flat_size(&ty);
                    v.get(before as usize..(before + size) as usize)
                        .map(<[Comp]>::to_vec)
                });
                Ok(with_value(
                    expr(ExprKind::Field(Box::new(base), i), ty, line),
                    value,
                ))
            }
            Type::Vector(s, _) => {
                let size = base.ty.size();
                let sets = ["xyzw", "rgba", "stpq"];
                let set = sets
                    .iter()
                    .find(|set| name.chars().all(|c| set.contains(c)))
                    .ok_or_else(|| Error::new(line, name, "illegal vector field selection"))?;
                let comps: Vec<u8> = name
                    .chars()
                    .map(|c| set.find(c).expect("in the set") as u8)
                    .collect();
                if comps.len() > 4 {
                    return Err(Error::new(line, name, "illegal vector field selection"));
                }
                if comps.iter().any(|&c| c >= size) {
                    return Err(Error::new(
                        line,
                        name,
                        "vector field selection out of range",
                    ));
                }
                let ty = Type::vector(*s, comps.len() as u8);
                let value = base
                    .value
                    .as_ref()
                    .map(|v| comps.iter().map(|&c| v[c as usize]).collect());
                Ok(with_value(
                    expr(ExprKind::Swizzle(Box::new(base), comps), ty, line),
                    value,
                ))
            }
            _ => Err(Error::new(
                line,
                ".",
                " field selection requires structure or vector on left hand side",
            )),
        }
    }

    fn primary(&mut self) -> R<Expr> {
        let token = self.next()?;
        let line = token.line;
        match &token.tok {
            Tok::Punct("(") => {
                let e = self.expression()?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Int(text) => {
                let (value, unsigned) =
                    int_literal(text).ok_or_else(|| Error::new(line, text, "Integer overflow"))?;
                if unsigned {
                    if self.ctx.version == 100 {
                        return Err(Error::new(line, text, "invalid number"));
                    }
                    return Ok(with_value(
                        expr(ExprKind::Lit(Comp::U(value)), UINT, line),
                        Some(vec![Comp::U(value)]),
                    ));
                }
                // A literal of 32 bits past int's range is its bits (GLSL
                // ES 3.00, 4.1.3).
                let v = value as i32;
                Ok(with_value(
                    expr(ExprKind::Lit(Comp::I(v)), INT, line),
                    Some(vec![Comp::I(v)]),
                ))
            }
            Tok::Float(text) => {
                if self.ctx.version == 100 && text.ends_with(['f', 'F']) {
                    return Err(Error::new(line, text, "invalid number"));
                }
                let digits = text.trim_end_matches(['f', 'F']);
                let v: f32 = digits
                    .parse()
                    .map_err(|_| Error::new(line, text, "invalid number"))?;
                if !v.is_finite() {
                    return Err(Error::new(line, text, "Float overflow"));
                }
                Ok(with_value(
                    expr(ExprKind::Lit(Comp::F(v)), FLOAT, line),
                    Some(vec![Comp::F(v)]),
                ))
            }
            Tok::Ident(word) => {
                let word = word.clone();
                match word.as_str() {
                    "true" | "false" => {
                        let b = word == "true";
                        return Ok(with_value(
                            expr(ExprKind::Lit(Comp::B(b)), BOOL, line),
                            Some(vec![Comp::B(b)]),
                        ));
                    }
                    _ => {}
                }
                if self.is("(") || (self.is("[") && self.basic_type(&word).is_some()) {
                    return self.call(&word, line);
                }
                if let Some(var) = self.lookup_var(&word) {
                    return Ok(self.var_expr(var, line));
                }
                if let Some(&(global, field)) = self.block_members.get(&word) {
                    self.unit_mark_used(global);
                    let base = self.var_expr(Var::Global(global), line);
                    let ty = match &base.ty {
                        Type::Struct(s) => self.unit.structs[*s].fields[field].1.clone(),
                        _ => Type::Void,
                    };
                    return Ok(expr(ExprKind::Field(Box::new(base), field), ty, line));
                }
                Err(Error::new(line, &word, "undeclared identifier"))
            }
            _ => Err(Error::new(line, &token.text(), "syntax error")),
        }
    }

    fn unit_mark_used(&mut self, global: usize) {
        self.unit.globals[global].used = true;
    }

    fn var_expr(&mut self, var: Var, line: u32) -> Expr {
        let ty = self.var_type(var).clone();
        let (kind, value) = match var {
            Var::Global(g) => {
                self.unit.globals[g].used = true;
                (ExprKind::Global(g), self.unit.globals[g].value.clone())
            }
            Var::Local(l) => (ExprKind::Local(l), self.current().locals[l].value.clone()),
            Var::Param(p) => (ExprKind::Param(p), None),
        };
        with_value(expr(kind, ty, line), value)
    }

    /// A call: a constructor's, a function's of the shader's, or a
    /// built-in's.
    fn call(&mut self, name: &str, line: u32) -> R<Expr> {
        // An array's constructor (GLSL ES 3.00): `T[n](...)` or `T[](...)`.
        let mut array: Option<Option<u32>> = None;
        if self.eat("[") {
            if self.ctx.version == 100 {
                return Err(Error::new(
                    line,
                    "[",
                    "array constructors are not supported in GLSL ES 1.00",
                ));
            }
            if self.eat("]") {
                array = Some(None);
            } else {
                let size = self.expression()?;
                self.expect("]")?;
                let n = size.value.as_ref().map(|v| v[0].as_i32()).filter(|&n| {
                    n > 0
                        && size
                            .ty
                            .scalar()
                            .is_some_and(|s| matches!(s, Scalar::Int | Scalar::Uint))
                });
                let n = n.ok_or_else(|| {
                    Error::new(
                        line,
                        "[",
                        "array size must be a constant integer expression greater than zero",
                    )
                })?;
                array = Some(Some(n as u32));
            }
        }
        self.expect("(")?;
        let mut args = Vec::new();
        if self.is_word("void") && self.peek_at(1).is_some_and(|t| t.is(")")) {
            self.next()?;
        }
        if !self.eat(")") {
            loop {
                args.push(self.assignment()?);
                if self.eat(")") {
                    break;
                }
                self.expect(",")?;
            }
        }
        // A variable's name hides the functions of that name.
        if self.lookup_var(name).is_some() && self.basic_type(name).is_none() {
            return Err(Error::new(line, name, "function name expected"));
        }
        if let Some(ty) = self.basic_type(name) {
            let ty = match array {
                Some(Some(n)) => Type::Array(Box::new(ty), n),
                Some(None) => Type::Array(Box::new(ty), args.len() as u32),
                None => ty,
            };
            return self.construct(ty, args, line);
        }
        if let Some(overloads) = self.functions_named(name).cloned() {
            let found = overloads.iter().copied().find(|&i| {
                let f = &self.unit.functions[i];
                f.params.len() == args.len()
                    && f.params.iter().zip(&args).all(|(p, a)| p.ty == a.ty)
            });
            let Some(index) = found else {
                return Err(Error::new(
                    line,
                    name,
                    "no matching overloaded function found",
                ));
            };
            let function = self.unit.functions[index].clone();
            for (param, arg) in function.params.iter().zip(&args) {
                if param.mode != ParamMode::In {
                    self.check_lvalue(arg, name, line).map_err(|_| {
                        Error::new(
                            line,
                            name,
                            "Constant value cannot be passed for 'out' or 'inout' parameters.",
                        )
                    })?;
                }
            }
            self.note_call(index);
            return Ok(expr(ExprKind::Call(index, args), function.ret, line));
        }
        let types: Vec<Type> = args.iter().map(|a| a.ty.clone()).collect();
        match builtins::resolve(name, &types, &self.ctx, &self.unit) {
            Some(Ok((canonical, ty))) => {
                if canonical == "modf" {
                    self.check_lvalue(&args[1], name, line)?;
                }
                if builtins::needs_constant_offset(canonical)
                    && let Some(offset) = args.last()
                    && offset.value.is_none()
                {
                    return Err(Error::new(
                        line,
                        name,
                        "Texture offset must be a constant expression",
                    ));
                }
                let value = if args.iter().all(|a| a.value.is_some()) {
                    builtins::fold(canonical, &args, &ty)
                } else {
                    None
                };
                Ok(with_value(
                    expr(ExprKind::Builtin(canonical, args), ty, line),
                    value,
                ))
            }
            Some(Err(())) => Err(Error::new(
                line,
                name,
                "no matching overloaded function found",
            )),
            None => Err(Error::new(
                line,
                name,
                "no matching overloaded function found",
            )),
        }
    }

    /// A constructor of `ty` (GLSL ES 5.4).
    fn construct(&self, ty: Type, args: Vec<Expr>, line: u32) -> R<Expr> {
        let name = ty.name(&self.unit);
        let fail = |why: &str| Err(Error::new(line, &name, why));
        if args.is_empty() {
            return fail("constructor does not have any arguments");
        }
        if args
            .iter()
            .any(|a| a.ty.has_sampler(&self.unit) || a.ty == Type::Void)
        {
            return fail("cannot convert a sampler");
        }
        match &ty {
            Type::Void | Type::Sampler(_) => return fail("cannot construct this type"),
            Type::Struct(s) => {
                let fields = &self.unit.structs[*s].fields;
                if fields.len() != args.len()
                    || fields.iter().zip(&args).any(|((_, t), a)| *t != a.ty)
                {
                    return fail(
                        "Number of constructor parameters does not match the number of structure fields",
                    );
                }
            }
            Type::Array(element, n) => {
                if args.len() as u32 != *n || args.iter().any(|a| a.ty != **element) {
                    return fail(
                        "array constructor argument count or types do not match the array",
                    );
                }
            }
            Type::Scalar(_) | Type::Vector(..) | Type::Matrix(..) => {
                if args
                    .iter()
                    .any(|a| !(a.ty.is_scalar_or_vector() || a.ty.is_matrix()))
                {
                    return fail("cannot convert parameter");
                }
                let needed = ty.components();
                if ty.is_matrix() && args.iter().any(|a| a.ty.is_matrix()) {
                    if args.len() > 1 {
                        return fail("constructing matrix from matrix can only take one argument");
                    }
                } else if !(args.len() == 1 && args[0].ty.is_scalar()) {
                    // Every argument is needed; the last may give more.
                    let mut have = 0;
                    for (i, a) in args.iter().enumerate() {
                        if have >= needed {
                            return fail("too many arguments");
                        }
                        have += a.ty.components();
                        let _ = i;
                    }
                    if have < needed {
                        return fail("not enough data provided for construction");
                    }
                    if self.ctx.version == 100
                        && ty.is_matrix()
                        && args.iter().any(|a| a.ty.is_matrix())
                    {
                        return fail("cannot construct a matrix from a matrix in GLSL ES 1.00");
                    }
                }
            }
        }
        let value = if args.iter().all(|a| a.value.is_some()) {
            self.fold_construct(&ty, &args)
        } else {
            None
        };
        Ok(with_value(expr(ExprKind::Construct(args), ty, line), value))
    }

    // ---- Constants ----

    fn fold_construct(&self, ty: &Type, args: &[Expr]) -> Option<Vec<Comp>> {
        let values: Vec<&Vec<Comp>> = args
            .iter()
            .map(|a| a.value.as_ref())
            .collect::<Option<_>>()?;
        match ty {
            Type::Struct(_) | Type::Array(..) => {
                Some(values.into_iter().flatten().copied().collect())
            }
            Type::Scalar(s) => Some(vec![values[0][0].to(*s)]),
            Type::Vector(s, n) => {
                if args.len() == 1 && args[0].ty.is_scalar() {
                    return Some(vec![values[0][0].to(*s); *n as usize]);
                }
                Some(
                    values
                        .into_iter()
                        .flatten()
                        .take(*n as usize)
                        .map(|c| c.to(*s))
                        .collect(),
                )
            }
            Type::Matrix(c, r) => {
                let (c, r) = (*c as usize, *r as usize);
                if args.len() == 1 && args[0].ty.is_scalar() {
                    let d = values[0][0].as_f32();
                    return Some(
                        (0..c * r)
                            .map(|i| Comp::F(if i / r == i % r { d } else { 0.0 }))
                            .collect(),
                    );
                }
                if let Type::Matrix(c2, r2) = args[0].ty {
                    let (c2, r2) = (c2 as usize, r2 as usize);
                    let m = values[0];
                    return Some(
                        (0..c * r)
                            .map(|i| {
                                let (col, row) = (i / r, i % r);
                                if col < c2 && row < r2 {
                                    m[col * r2 + row]
                                } else {
                                    Comp::F(if col == row { 1.0 } else { 0.0 })
                                }
                            })
                            .collect(),
                    );
                }
                Some(
                    values
                        .into_iter()
                        .flatten()
                        .take(c * r)
                        .map(|v| v.to(Scalar::Float))
                        .collect(),
                )
            }
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fold_binary(
        &self,
        op: BinOp,
        at: &Type,
        a: &[Comp],
        bt: &Type,
        b: &[Comp],
        ty: &Type,
        line: u32,
    ) -> R<Option<Vec<Comp>>> {
        match op {
            BinOp::Eq | BinOp::Ne => {
                let same = a == b;
                return Ok(Some(vec![Comp::B(if op == BinOp::Eq {
                    same
                } else {
                    !same
                })]));
            }
            BinOp::Mul if at.is_matrix() || bt.is_matrix() => {
                if at.is_scalar() || bt.is_scalar() {
                    // Component-wise.
                } else {
                    return Ok(Some(matrix_product(at, a, bt, b)));
                }
            }
            _ => {}
        }
        let n = ty.components().max(1) as usize;
        let pick = |v: &[Comp], i: usize| if v.len() == 1 { v[0] } else { v[i] };
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let (x, y) = (pick(a, i), pick(b, i));
            out.push(match (x, y) {
                (Comp::F(x), Comp::F(y)) => match op {
                    BinOp::Add => Comp::F(x + y),
                    BinOp::Sub => Comp::F(x - y),
                    BinOp::Mul => Comp::F(x * y),
                    BinOp::Div => Comp::F(x / y),
                    BinOp::Lt => Comp::B(x < y),
                    BinOp::Gt => Comp::B(x > y),
                    BinOp::Le => Comp::B(x <= y),
                    BinOp::Ge => Comp::B(x >= y),
                    _ => return Ok(None),
                },
                (Comp::I(x), Comp::I(y)) => match op {
                    BinOp::Add => Comp::I(x.wrapping_add(y)),
                    BinOp::Sub => Comp::I(x.wrapping_sub(y)),
                    BinOp::Mul => Comp::I(x.wrapping_mul(y)),
                    BinOp::Div | BinOp::Rem if y == 0 => {
                        return Err(Error::new(
                            line,
                            op_text(op),
                            "Divide by zero error during constant folding",
                        ));
                    }
                    BinOp::Div => Comp::I(x.wrapping_div(y)),
                    BinOp::Rem => Comp::I(x.wrapping_rem(y)),
                    BinOp::BitAnd => Comp::I(x & y),
                    BinOp::BitOr => Comp::I(x | y),
                    BinOp::BitXor => Comp::I(x ^ y),
                    BinOp::Shl => Comp::I(x.wrapping_shl(y as u32)),
                    BinOp::Shr => Comp::I(x.wrapping_shr(y as u32)),
                    BinOp::Lt => Comp::B(x < y),
                    BinOp::Gt => Comp::B(x > y),
                    BinOp::Le => Comp::B(x <= y),
                    BinOp::Ge => Comp::B(x >= y),
                    _ => return Ok(None),
                },
                (Comp::U(x), Comp::U(y)) => match op {
                    BinOp::Add => Comp::U(x.wrapping_add(y)),
                    BinOp::Sub => Comp::U(x.wrapping_sub(y)),
                    BinOp::Mul => Comp::U(x.wrapping_mul(y)),
                    BinOp::Div | BinOp::Rem if y == 0 => {
                        return Err(Error::new(
                            line,
                            op_text(op),
                            "Divide by zero error during constant folding",
                        ));
                    }
                    BinOp::Div => Comp::U(x / y),
                    BinOp::Rem => Comp::U(x % y),
                    BinOp::BitAnd => Comp::U(x & y),
                    BinOp::BitOr => Comp::U(x | y),
                    BinOp::BitXor => Comp::U(x ^ y),
                    BinOp::Shl => Comp::U(x.wrapping_shl(y)),
                    BinOp::Shr => Comp::U(x.wrapping_shr(y)),
                    BinOp::Lt => Comp::B(x < y),
                    BinOp::Gt => Comp::B(x > y),
                    BinOp::Le => Comp::B(x <= y),
                    BinOp::Ge => Comp::B(x >= y),
                    _ => return Ok(None),
                },
                // A shift by the other signedness.
                (Comp::I(x), Comp::U(y)) => match op {
                    BinOp::Shl => Comp::I(x.wrapping_shl(y)),
                    BinOp::Shr => Comp::I(x.wrapping_shr(y)),
                    _ => return Ok(None),
                },
                (Comp::U(x), Comp::I(y)) => match op {
                    BinOp::Shl => Comp::U(x.wrapping_shl(y as u32)),
                    BinOp::Shr => Comp::U(x.wrapping_shr(y as u32)),
                    _ => return Ok(None),
                },
                _ => return Ok(None),
            });
        }
        Ok(Some(out))
    }
}

/// A constant matrix's product with a matrix or a vector.
fn matrix_product(at: &Type, a: &[Comp], bt: &Type, b: &[Comp]) -> Vec<Comp> {
    // Each as columns by rows; a vector is a column on the right, a row
    // on the left.
    let (ac, ar) = match at {
        Type::Matrix(c, r) => (*c as usize, *r as usize),
        Type::Vector(_, n) => (*n as usize, 1),
        _ => (1, 1),
    };
    let (bc, br) = match bt {
        Type::Matrix(c, r) => (*c as usize, *r as usize),
        Type::Vector(_, n) => (1, *n as usize),
        _ => (1, 1),
    };
    let _ = br;
    let mut out = Vec::with_capacity(bc * ar);
    for col in 0..bc {
        for row in 0..ar {
            let mut sum = 0.0f32;
            for k in 0..ac {
                sum += a[k * ar + row].as_f32() * b[col * ac + k].as_f32();
            }
            out.push(Comp::F(sum));
        }
    }
    out
}
