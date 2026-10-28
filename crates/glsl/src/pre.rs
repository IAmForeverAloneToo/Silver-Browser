//! The preprocessor (GLSL ES 1.00 and 3.00, section 3.4): comments and
//! line continuations, `#version`, `#define` and `#undef` with object-
//! and function-like macros, the conditionals, `#extension`, `#pragma`,
//! `#line` and `#error`, as WebGL's shaders use them. Its output is the
//! shader's tokens, each on the line it came from.

use std::collections::HashMap;

use crate::lex::{self, Tok, Token};
use crate::{Error, MAX_GLSL_DEPTH, MAX_GLSL_TOKENS, Options, Stage};

/// Whether a `#define`'s text (after its `#`) makes a function-like
/// macro: its name followed at once by `(`.
fn function_like(directive: &str) -> bool {
    let rest = directive.trim_start();
    let Some(rest) = rest.strip_prefix("define") else {
        return false;
    };
    let rest = rest.trim_start();
    let name = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    rest[name..].starts_with('(')
}

/// What preprocessing found besides the tokens.
pub struct Preprocessed {
    pub tokens: Vec<Token>,
    /// 100 or 300.
    pub version: u32,
    /// The extensions the shader enabled, by their names.
    pub enabled: Vec<String>,
    /// Warnings, in the log's form.
    pub warnings: String,
}

#[derive(Clone, PartialEq)]
struct Macro {
    /// A function-like macro's parameters; `None` for an object-like one.
    params: Option<Vec<String>>,
    body: Vec<Tok>,
}

struct Cond {
    /// Whether the region it is in is taken.
    outer: bool,
    /// Whether its present branch is taken.
    taking: bool,
    /// Whether a branch of it was taken already.
    taken: bool,
    /// Whether its `#else` was met.
    else_seen: bool,
}

/// The extensions of WebGL 1's shaders this crate knows, with whether
/// `options` offers each.
fn known_extensions(options: &Options) -> [(&'static str, bool); 4] {
    let e = &options.extensions;
    [
        ("GL_OES_standard_derivatives", e.standard_derivatives),
        ("GL_EXT_frag_depth", e.frag_depth),
        ("GL_EXT_draw_buffers", e.draw_buffers),
        ("GL_EXT_shader_texture_lod", e.shader_texture_lod),
    ]
}

/// The source without its comments and line continuations, as logical
/// lines with the line each begins on; refused where a character outside
/// a comment is none of GLSL ES's.
fn lines(source: &str) -> Result<Vec<(u32, String)>, Error> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut start = 1;
    let mut line = 1;
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'/') => {
                while let Some(&next) = chars.peek() {
                    if next == '\n' {
                        break;
                    }
                    chars.next();
                }
                current.push(' ');
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut ended = false;
                while let Some(next) = chars.next() {
                    if next == '\n' {
                        // The comment's lines stay lines, so that what
                        // follows is numbered as it is.
                        out.push((start, std::mem::take(&mut current)));
                        line += 1;
                        start = line;
                    } else if next == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        ended = true;
                        break;
                    }
                }
                if !ended {
                    return Err(Error::new(line, "/*", "unterminated comment"));
                }
                current.push(' ');
            }
            '\\' if matches!(chars.peek(), Some('\n' | '\r')) => {
                if chars.next() == Some('\r') && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                line += 1;
            }
            '\n' => {
                out.push((start, std::mem::take(&mut current)));
                line += 1;
                start = line;
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    continue;
                }
                out.push((start, std::mem::take(&mut current)));
                line += 1;
                start = line;
            }
            c if lex::allowed(c) => current.push(c),
            c => return Err(Error::new(line, &c.to_string(), "invalid character")),
        }
    }
    out.push((start, current));
    Ok(out)
}

pub fn preprocess(source: &str, stage: Stage, options: &Options) -> Result<Preprocessed, Error> {
    let lines = lines(source)?;
    let mut pp = Pre {
        macros: HashMap::new(),
        version: 100,
        tokens: Vec::new(),
        enabled: Vec::new(),
        warnings: String::new(),
        options,
        line_offset: 0,
    };
    // `#version`, when there is one, before anything but space.
    let mut first = true;
    let mut conds: Vec<Cond> = Vec::new();
    let mut run: Vec<Token> = Vec::new();
    for (line, text) in &lines {
        let line = pp.at(*line);
        let trimmed = text.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let active = conds.last().is_none_or(|c| c.outer && c.taking);
        if let Some(directive) = trimmed.strip_prefix('#') {
            pp.flush(&mut run)?;
            let mut tokens = Vec::new();
            lex::lex(directive, line, &mut tokens)?;
            let name = tokens
                .first()
                .and_then(Token::ident)
                .unwrap_or("")
                .to_owned();
            let args = if tokens.is_empty() {
                &[][..]
            } else {
                &tokens[1..]
            };
            match name.as_str() {
                "version" => {
                    if !first {
                        return Err(Error::new(
                            line,
                            "version",
                            "#version directive must occur before anything else, except for comments and white space",
                        ));
                    }
                    first = false;
                    pp.version(args, line)?;
                    continue;
                }
                "ifdef" | "ifndef" => {
                    let asked = args.first().and_then(Token::ident);
                    if active && (asked.is_none() || args.len() != 1) {
                        return Err(Error::new(line, &name, "invalid macro name"));
                    }
                    let taking =
                        active && asked.is_some_and(|n| pp.defined(n)) == (name == "ifdef");
                    conds.push(Cond {
                        outer: active,
                        taking,
                        taken: taking,
                        else_seen: false,
                    });
                }
                "if" => {
                    let taking = active && pp.condition(args, line)?;
                    conds.push(Cond {
                        outer: active,
                        taking,
                        taken: taking,
                        else_seen: false,
                    });
                }
                "elif" => {
                    let cond = conds.last().ok_or_else(|| {
                        Error::new(line, "elif", "unexpected #elif found without #if")
                    })?;
                    if cond.else_seen {
                        return Err(Error::new(line, "elif", "#elif found after #else"));
                    }
                    // Its expression is read only where it could be taken.
                    let taking = cond.outer && !cond.taken && pp.condition(args, line)?;
                    let cond = conds.last_mut().expect("there");
                    cond.taking = taking;
                    cond.taken |= taking;
                }
                "else" => {
                    let cond = conds.last_mut().ok_or_else(|| {
                        Error::new(line, "else", "unexpected #else found without #if")
                    })?;
                    if cond.else_seen {
                        return Err(Error::new(line, "else", "#else after #else"));
                    }
                    cond.else_seen = true;
                    cond.taking = !cond.taken;
                    cond.taken = true;
                }
                "endif" => {
                    if conds.pop().is_none() {
                        return Err(Error::new(
                            line,
                            "endif",
                            "unexpected #endif found without #if",
                        ));
                    }
                }
                _ if !active => {}
                "" => {}
                "define" => pp.define(args, line, function_like(directive))?,
                "undef" => pp.undef(args, line)?,
                "extension" => pp.extension(args, line, first)?,
                "pragma" => {}
                "line" => pp.line(args, line)?,
                "error" => {
                    let message: Vec<String> = args.iter().map(Token::text).collect();
                    return Err(Error::new(line, "#error", &message.join(" ")));
                }
                other => {
                    return Err(Error::new(line, other, "invalid directive name"));
                }
            }
            if name != "extension" && name != "pragma" {
                first = false;
            }
            continue;
        }
        first = false;
        if !active {
            continue;
        }
        lex::lex(text, line, &mut run)?;
        if run.len() > MAX_GLSL_TOKENS {
            return Err(Error::new(line, "", "the shader has too many tokens"));
        }
    }
    pp.flush(&mut run)?;
    if !conds.is_empty() {
        let line = lines.last().map_or(1, |(line, _)| pp.at(*line));
        return Err(Error::new(
            line,
            "",
            "unexpected end of file found in conditional block",
        ));
    }
    let _ = stage;
    Ok(Preprocessed {
        tokens: pp.tokens,
        version: pp.version,
        enabled: pp.enabled,
        warnings: pp.warnings,
    })
}

struct Pre<'a> {
    macros: HashMap<String, Macro>,
    version: u32,
    tokens: Vec<Token>,
    enabled: Vec<String>,
    warnings: String,
    options: &'a Options,
    /// What `#line` moved the numbers by.
    line_offset: i64,
}

impl Pre<'_> {
    /// The line `physical` is numbered.
    fn at(&self, physical: u32) -> u32 {
        (i64::from(physical).saturating_add(self.line_offset)).clamp(0, i64::from(u32::MAX)) as u32
    }

    /// A predefined macro's being defined: `Some` for those there are.
    fn predefined(&self, name: &str) -> Option<bool> {
        match name {
            "__LINE__" | "__FILE__" | "__VERSION__" | "GL_ES" | "GL_FRAGMENT_PRECISION_HIGH" => {
                Some(true)
            }
            _ => known_extensions(self.options)
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, offered)| *offered && self.version == 100),
        }
    }

    fn defined(&self, name: &str) -> bool {
        self.macros.contains_key(name) || self.predefined(name) == Some(true)
    }

    fn version(&mut self, args: &[Token], line: u32) -> Result<(), Error> {
        let number = args.first().map(Token::text).unwrap_or_default();
        match (number.as_str(), args.get(1).and_then(Token::ident)) {
            ("100", None) if args.len() == 1 => self.version = 100,
            ("300", Some("es")) if args.len() == 2 && self.options.webgl2 => self.version = 300,
            ("300", Some("es")) => {
                return Err(Error::new(line, "300", "unsupported shader version"));
            }
            _ => {
                return Err(Error::new(line, &number, "version number not supported"));
            }
        }
        Ok(())
    }

    fn reserved(&mut self, name: &str, line: u32) -> Result<(), Error> {
        if self.predefined(name).is_some() {
            return Err(Error::new(line, name, "predefined macro redefined"));
        }
        if name.starts_with("GL_") {
            return Err(Error::new(
                line,
                name,
                "macro names beginning with \"GL_\" are reserved",
            ));
        }
        if name.contains("__") {
            if self.version == 300 {
                return Err(Error::new(
                    line,
                    name,
                    "macro names containing two consecutive underscores are reserved",
                ));
            }
            self.warnings.push_str(&format!(
                "WARNING: 0:{line}: '{name}' : macro name with a double underscore is reserved
"
            ));
        }
        Ok(())
    }

    fn define(&mut self, args: &[Token], line: u32, function_like: bool) -> Result<(), Error> {
        let name = args
            .first()
            .and_then(Token::ident)
            .ok_or_else(|| Error::new(line, "define", "invalid macro name"))?
            .to_owned();
        self.reserved(&name, line)?;
        let mut rest = &args[1..];
        let params = if function_like {
            let mut params = Vec::new();
            let mut i = 1;
            loop {
                match rest.get(i) {
                    Some(t) if t.is(")") => {
                        i += 1;
                        break;
                    }
                    Some(t) if t.ident().is_some() => {
                        let param = t.text();
                        if params.contains(&param) {
                            return Err(Error::new(line, &param, "duplicate macro parameter name"));
                        }
                        params.push(param);
                        i += 1;
                        match rest.get(i) {
                            Some(t) if t.is(",") => i += 1,
                            Some(t) if t.is(")") => {}
                            _ => {
                                return Err(Error::new(
                                    line,
                                    &name,
                                    "invalid macro parameter list",
                                ));
                            }
                        }
                    }
                    _ => return Err(Error::new(line, &name, "invalid macro parameter list")),
                }
            }
            rest = &rest[i..];
            Some(params)
        } else {
            None
        };
        let made = Macro {
            params,
            body: rest.iter().map(|t| t.tok.clone()).collect(),
        };
        if let Some(had) = self.macros.get(&name)
            && *had != made
        {
            return Err(Error::new(line, &name, "Macro redefined"));
        }
        self.macros.insert(name, made);
        Ok(())
    }

    fn undef(&mut self, args: &[Token], line: u32) -> Result<(), Error> {
        let name = args
            .first()
            .and_then(Token::ident)
            .ok_or_else(|| Error::new(line, "undef", "invalid macro name"))?;
        if self.predefined(name).is_some() {
            return Err(Error::new(line, name, "predefined macro undefined"));
        }
        if name.starts_with("GL_") {
            return Err(Error::new(
                line,
                name,
                "macro names beginning with \"GL_\" are reserved",
            ));
        }
        self.macros.remove(name);
        Ok(())
    }

    fn extension(&mut self, args: &[Token], line: u32, first: bool) -> Result<(), Error> {
        let (Some(name), Some(colon), Some(behavior)) = (
            args.first().and_then(Token::ident),
            args.get(1),
            args.get(2).and_then(Token::ident),
        ) else {
            return Err(Error::new(line, "extension", "invalid extension directive"));
        };
        if !colon.is(":") || args.len() != 3 {
            return Err(Error::new(line, "extension", "invalid extension directive"));
        }
        if self.version == 300 && !first && !self.tokens.is_empty() {
            return Err(Error::new(
                line,
                "extension",
                "extension directive must occur before any non-preprocessor tokens in ESSL3",
            ));
        }
        if !matches!(behavior, "require" | "enable" | "warn" | "disable") {
            return Err(Error::new(line, behavior, "invalid extension behavior"));
        }
        if name == "all" {
            if matches!(behavior, "require" | "enable") {
                return Err(Error::new(
                    line,
                    behavior,
                    "extension 'all' cannot have 'require' or 'enable' behavior",
                ));
            }
            self.enabled.clear();
            return Ok(());
        }
        let offered = self.version == 100
            && known_extensions(self.options)
                .iter()
                .any(|(n, offered)| *n == name && *offered);
        if !offered {
            if behavior == "require" {
                return Err(Error::new(line, name, "extension is not supported"));
            }
            self.warnings.push_str(&format!(
                "WARNING: 0:{line}: '{name}' : extension is not supported\n"
            ));
            return Ok(());
        }
        self.enabled.retain(|n| n != name);
        if behavior != "disable" {
            self.enabled.push(name.to_owned());
        }
        Ok(())
    }

    fn line(&mut self, args: &[Token], line: u32) -> Result<(), Error> {
        let mut expanded = Vec::new();
        self.expand(args, &mut Vec::new(), &mut expanded, 0)?;
        let number = expanded
            .first()
            .and_then(|t| match &t.tok {
                Tok::Int(text) => text.parse::<i64>().ok(),
                _ => None,
            })
            .ok_or_else(|| Error::new(line, "line", "invalid line directive"))?;
        // The next line is `number`; a line past u32's is clamped when
        // it is said, and the offset saturates on the way.
        let step = number.saturating_sub(i64::from(line)).saturating_sub(1);
        self.line_offset = self.line_offset.saturating_add(step);
        Ok(())
    }

    /// An `#if`'s or `#elif`'s condition.
    fn condition(&self, args: &[Token], line: u32) -> Result<bool, Error> {
        // `defined` first, then the macros.
        let mut resolved = Vec::new();
        let mut i = 0;
        while i < args.len() {
            if args[i].ident() == Some("defined") {
                let (name, next) = match (args.get(i + 1), args.get(i + 2), args.get(i + 3)) {
                    (Some(open), Some(name), Some(close)) if open.is("(") && close.is(")") => {
                        (name.ident(), i + 4)
                    }
                    (Some(name), _, _) => (name.ident(), i + 2),
                    _ => (None, i + 1),
                };
                let name = name
                    .ok_or_else(|| Error::new(line, "defined", "unexpected token after defined"))?;
                resolved.push(Token {
                    tok: Tok::Int(u8::from(self.defined(name)).to_string()),
                    line,
                });
                i = next;
                continue;
            }
            resolved.push(args[i].clone());
            i += 1;
        }
        let mut expanded = Vec::new();
        self.expand(&resolved, &mut Vec::new(), &mut expanded, 0)?;
        if expanded.is_empty() {
            return Err(Error::new(line, "if", "invalid expression in conditional"));
        }
        let mut reader = Cond2 {
            tokens: &expanded,
            at: 0,
            line,
        };
        let value = reader.expr(0)?;
        if reader.at != expanded.len() {
            return Err(Error::new(
                line,
                &expanded[reader.at].text(),
                "unexpected token after conditional expression",
            ));
        }
        Ok(value != 0)
    }

    /// The text run's tokens, its macros expanded, taken.
    fn flush(&mut self, run: &mut Vec<Token>) -> Result<(), Error> {
        let taken = std::mem::take(run);
        let mut out = std::mem::take(&mut self.tokens);
        self.expand(&taken, &mut Vec::new(), &mut out, 0)?;
        self.tokens = out;
        Ok(())
    }

    /// `tokens`, their macros expanded, pushed to `out`: `disabled` the
    /// macros being expanded, not expanded again.
    fn expand(
        &self,
        tokens: &[Token],
        disabled: &mut Vec<String>,
        out: &mut Vec<Token>,
        depth: usize,
    ) -> Result<(), Error> {
        if depth > MAX_GLSL_DEPTH {
            let line = tokens.first().map_or(0, |t| t.line);
            return Err(Error::new(line, "", "macro expansion too deep"));
        }
        let mut i = 0;
        while i < tokens.len() {
            let token = &tokens[i];
            if out.len() > MAX_GLSL_TOKENS {
                return Err(Error::new(token.line, "", "the shader has too many tokens"));
            }
            let Some(name) = token.ident() else {
                out.push(token.clone());
                i += 1;
                continue;
            };
            match name {
                "__LINE__" => {
                    out.push(Token {
                        tok: Tok::Int(token.line.to_string()),
                        line: token.line,
                    });
                    i += 1;
                    continue;
                }
                "__FILE__" => {
                    out.push(Token {
                        tok: Tok::Int("0".to_owned()),
                        line: token.line,
                    });
                    i += 1;
                    continue;
                }
                "__VERSION__" => {
                    out.push(Token {
                        tok: Tok::Int(self.version.to_string()),
                        line: token.line,
                    });
                    i += 1;
                    continue;
                }
                "GL_ES" | "GL_FRAGMENT_PRECISION_HIGH" => {
                    out.push(Token {
                        tok: Tok::Int("1".to_owned()),
                        line: token.line,
                    });
                    i += 1;
                    continue;
                }
                _ => {}
            }
            if self.predefined(name) == Some(true) {
                out.push(Token {
                    tok: Tok::Int("1".to_owned()),
                    line: token.line,
                });
                i += 1;
                continue;
            }
            let Some(made) = self
                .macros
                .get(name)
                .filter(|_| !disabled.iter().any(|d| d == name))
            else {
                out.push(token.clone());
                i += 1;
                continue;
            };
            let line = token.line;
            let body: Vec<Token> = made
                .body
                .iter()
                .map(|tok| Token {
                    tok: tok.clone(),
                    line,
                })
                .collect();
            match &made.params {
                None => {
                    disabled.push(name.to_owned());
                    self.expand(&body, disabled, out, depth + 1)?;
                    disabled.pop();
                    i += 1;
                }
                Some(params) => {
                    if !tokens.get(i + 1).is_some_and(|t| t.is("(")) {
                        out.push(token.clone());
                        i += 1;
                        continue;
                    }
                    // The arguments, split at the commas outside
                    // parentheses.
                    let mut args: Vec<Vec<Token>> = vec![Vec::new()];
                    let mut nesting = 0;
                    let mut j = i + 2;
                    loop {
                        let Some(t) = tokens.get(j) else {
                            return Err(Error::new(
                                line,
                                name,
                                "unexpected end of input in macro arguments",
                            ));
                        };
                        if t.is("(") {
                            nesting += 1;
                        } else if t.is(")") {
                            if nesting == 0 {
                                break;
                            }
                            nesting -= 1;
                        } else if t.is(",") && nesting == 0 {
                            args.push(Vec::new());
                            j += 1;
                            continue;
                        }
                        args.last_mut().expect("one").push(t.clone());
                        j += 1;
                    }
                    if params.is_empty() && args.len() == 1 && args[0].is_empty() {
                        args.clear();
                    }
                    if args.len() != params.len() {
                        let message = if args.len() < params.len() {
                            "macro has too few arguments"
                        } else {
                            "macro has too many arguments"
                        };
                        return Err(Error::new(line, name, message));
                    }
                    let mut expanded_args = Vec::new();
                    for arg in &args {
                        let mut e = Vec::new();
                        self.expand(arg, disabled, &mut e, depth + 1)?;
                        expanded_args.push(e);
                    }
                    let mut substituted = Vec::new();
                    for t in &body {
                        match t.ident().and_then(|n| params.iter().position(|p| p == n)) {
                            Some(k) => substituted.extend(expanded_args[k].iter().cloned()),
                            None => substituted.push(t.clone()),
                        }
                    }
                    disabled.push(name.to_owned());
                    self.expand(&substituted, disabled, out, depth + 1)?;
                    disabled.pop();
                    i = j + 1;
                }
            }
        }
        Ok(())
    }
}

/// An `#if`'s expression, C's integer operators.
struct Cond2<'a> {
    tokens: &'a [Token],
    at: usize,
    line: u32,
}

/// The binary operators, by precedence, the loosest first.
const BINARY: &[&[&str]] = &[
    &["||"],
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

impl Cond2<'_> {
    fn expr(&mut self, level: usize) -> Result<i64, Error> {
        if level == BINARY.len() {
            return self.unary(0);
        }
        let mut left = self.expr(level + 1)?;
        loop {
            let Some(op) = self.tokens.get(self.at).and_then(|t| match &t.tok {
                Tok::Punct(p) if BINARY[level].contains(p) => Some(*p),
                _ => None,
            }) else {
                return Ok(left);
            };
            self.at += 1;
            let right = self.expr(level + 1)?;
            left = match op {
                "||" => i64::from(left != 0 || right != 0),
                "&&" => i64::from(left != 0 && right != 0),
                "|" => left | right,
                "^" => left ^ right,
                "&" => left & right,
                "==" => i64::from(left == right),
                "!=" => i64::from(left != right),
                "<" => i64::from(left < right),
                ">" => i64::from(left > right),
                "<=" => i64::from(left <= right),
                ">=" => i64::from(left >= right),
                "<<" => left.wrapping_shl(right.clamp(0, 63) as u32),
                ">>" => left.wrapping_shr(right.clamp(0, 63) as u32),
                "+" => left.wrapping_add(right),
                "-" => left.wrapping_sub(right),
                "*" => left.wrapping_mul(right),
                "/" | "%" if right == 0 => {
                    return Err(Error::new(
                        self.line,
                        op,
                        "division by zero in preprocessor expression",
                    ));
                }
                "/" => left.wrapping_div(right),
                _ => left.wrapping_rem(right),
            };
        }
    }

    fn unary(&mut self, depth: usize) -> Result<i64, Error> {
        if depth > MAX_GLSL_DEPTH {
            return Err(Error::new(self.line, "", "expression too deep"));
        }
        let token = self
            .tokens
            .get(self.at)
            .ok_or_else(|| Error::new(self.line, "", "unexpected end of conditional expression"))?;
        self.at += 1;
        match &token.tok {
            Tok::Punct("+") => self.unary(depth + 1),
            Tok::Punct("-") => Ok(self.unary(depth + 1)?.wrapping_neg()),
            Tok::Punct("~") => Ok(!self.unary(depth + 1)?),
            Tok::Punct("!") => Ok(i64::from(self.unary(depth + 1)? == 0)),
            Tok::Punct("(") => {
                let value = self.expr_depth(depth + 1)?;
                if !self.tokens.get(self.at).is_some_and(|t| t.is(")")) {
                    return Err(Error::new(
                        self.line,
                        "(",
                        "missing ')' in conditional expression",
                    ));
                }
                self.at += 1;
                Ok(value)
            }
            Tok::Int(text) => crate::parse::int_literal(text)
                .map(|(value, _)| i64::from(value))
                .ok_or_else(|| Error::new(self.line, text, "integer overflow")),
            _ => Err(Error::new(
                self.line,
                &token.text(),
                "unexpected token in preprocessor conditional",
            )),
        }
    }

    fn expr_depth(&mut self, depth: usize) -> Result<i64, Error> {
        if depth > MAX_GLSL_DEPTH {
            return Err(Error::new(self.line, "", "expression too deep"));
        }
        self.expr(0)
    }
}
