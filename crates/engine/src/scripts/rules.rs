//! The CSSOM's rules (D66): which of a sheet's parsed rules a script
//! sees, as which kind of `CSSRule`, their `cssText` as CSSOM section
//! 6.7 serializes it, and the changes `insertRule`, `deleteRule` and the
//! rules' setters make to the parsed sheet the cascade reads. A style
//! rule is a grouping rule (CSS Nesting 1 section 3.3): its nested
//! rules, and the runs of declarations among them as
//! `CSSNestedDeclarations`, are a list as a group rule's are.

use std::sync::Arc;

use css::{Block, Declaration, Rule, Stylesheet, Token};
use js::DomError;
use style::selector_matching::{
    Namespaces, Nest, Relative, SelectorList, parse_nested_selector_list,
};

/// The most rules a list may hold after an insertion: what a hostile
/// script could otherwise grow a sheet by.
const MAX_RULES: usize = 1 << 16;

/// The kinds of rule a script sees, by `CSSRule.type` (the grouping and
/// layer rules have none of their own, 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Style,
    Import,
    Media,
    FontFace,
    Page,
    Keyframes,
    Keyframe,
    Supports,
    Layer,
    /// `@layer` without a block (`CSSLayerStatementRule`).
    LayerStatement,
    Namespace,
    /// `@container` (Conditional 5 section 7.1).
    Container,
    /// `@counter-style` (Counter Styles 3 section 9).
    CounterStyle,
    /// `@font-feature-values` (Fonts 4 section 6.10).
    FontFeatureValues,
    /// `@font-palette-values` (Fonts 4 section 9.1), a valid one.
    FontPaletteValues,
    /// A run of declarations among a style rule's nested rules, or in a
    /// group rule nested in one or in `@scope` (`CSSNestedDeclarations`,
    /// CSS Nesting 1 section 3.3).
    NestedDeclarations,
    /// `@scope` (Cascade 6 section 2.5.3).
    Scope,
    /// `@property` (Properties and Values API section 4.1), a valid one.
    Property,
}

impl Kind {
    /// The name the bindings make the rule's object by.
    fn name(self) -> &'static str {
        match self {
            Kind::Style => "style",
            Kind::Import => "import",
            Kind::Media => "media",
            Kind::FontFace => "font-face",
            Kind::Page => "page",
            Kind::Keyframes => "keyframes",
            Kind::Keyframe => "keyframe",
            Kind::Supports => "supports",
            Kind::Layer => "layer",
            Kind::LayerStatement => "layer-statement",
            Kind::Namespace => "namespace",
            Kind::Container => "container",
            Kind::CounterStyle => "counter-style",
            Kind::FontFeatureValues => "font-feature-values",
            Kind::FontPaletteValues => "font-palette-values",
            Kind::NestedDeclarations => "nested-declarations",
            Kind::Scope => "scope",
            Kind::Property => "property",
        }
    }
}

/// Where a rule list stands, which says what its rules may be.
#[derive(Debug, Clone, Copy)]
struct Place {
    /// It is a keyframes rule's: its rules are keyframes.
    keyframes: bool,
    /// It is a grouping rule's, not the sheet's own.
    nested: bool,
    /// What its style rules' selectors are relative to: the style rule
    /// it is nested in, or the `@scope` rule (CSS Nesting 1, Cascade 6).
    relative: Relative,
}

/// The sheet's own list.
const TOP: Place = Place {
    keyframes: false,
    nested: false,
    relative: Relative::No,
};

impl Place {
    /// The place of the list of a rule of `kind` in this list.
    fn inside(self, kind: Kind) -> Place {
        Place {
            keyframes: kind == Kind::Keyframes,
            nested: true,
            relative: match kind {
                Kind::Style => Relative::Nesting,
                Kind::Scope => Relative::Scope,
                _ => self.relative,
            },
        }
    }
}

/// Whether `tokens` are a style rule's selectors where `relative` says
/// the rule stands; `&` in them stands for any parent's.
fn valid_selector(tokens: &[Token], ns: &Namespaces, relative: Relative) -> bool {
    let parent = Arc::new(SelectorList(Vec::new()));
    let nest = Nest {
        parent: (relative == Relative::Nesting).then_some(&parent),
        relative,
    };
    parse_nested_selector_list(tokens, ns, nest).is_ok()
}

/// What `rule` is to a script in a list at `place`; `None` for a rule
/// the CSSOM leaves out: a style rule whose selector does not parse, a
/// run of declarations none of which the cascade takes, an import or a
/// namespace rule in a grouping rule, an at-rule it has no interface
/// for here.
fn kind(rule: &Rule, place: Place, ns: &Namespaces) -> Option<Kind> {
    let nested = place.nested;
    match rule {
        Rule::Qualified(_) if place.keyframes => Some(Kind::Keyframe),
        Rule::Qualified(rule) => {
            valid_selector(&rule.prelude, ns, place.relative).then_some(Kind::Style)
        }
        Rule::Declarations(list) => (place.relative != Relative::No
            && !declarations(list).is_empty())
        .then_some(Kind::NestedDeclarations),
        Rule::At(at) => {
            let rules = matches!(at.block, Some(Block::Rules(_)));
            let declarations = matches!(at.block, Some(Block::Declarations(_)));
            match at.name.to_ascii_lowercase().as_str() {
                // One without a URL is no rule.
                "import" if at.block.is_none() && !nested && starts_with_url(&at.prelude) => {
                    Some(Kind::Import)
                }
                "namespace"
                    if at.block.is_none()
                        && !nested
                        && style::namespace_declaration(&at.prelude).is_some() =>
                {
                    Some(Kind::Namespace)
                }
                "scope"
                    if rules
                        && style::scope_selectors(&at.prelude, ns, place.relative).is_some() =>
                {
                    Some(Kind::Scope)
                }
                // Names of identifiers joined by `.` (Cascade 5 section
                // 6.4.2): a statement's one or more, a block's one or none.
                "layer"
                    if at.block.is_none()
                        && split_top(&at.prelude, |t| matches!(t, Token::Comma))
                            .into_iter()
                            .all(|name| style::is_layer_name(trim_space(name))) =>
                {
                    Some(Kind::LayerStatement)
                }
                "media" if rules => Some(Kind::Media),
                "supports" if rules => Some(Kind::Supports),
                "layer"
                    if rules && {
                        let name = trim_space(&at.prelude);
                        name.is_empty() || style::is_layer_name(name)
                    } =>
                {
                    Some(Kind::Layer)
                }
                "keyframes" | "-webkit-keyframes" if rules => Some(Kind::Keyframes),
                "font-face" if declarations => Some(Kind::FontFace),
                "page" if declarations => Some(Kind::Page),
                // One whose prelude is a valid query list (D72).
                "container"
                    if rules && style::container::container_prelude(&at.prelude).is_some() =>
                {
                    Some(Kind::Container)
                }
                "counter-style" if declarations && counter_style_name(&at.prelude).is_some() => {
                    Some(Kind::CounterStyle)
                }
                "font-feature-values" if rules => Some(Kind::FontFeatureValues),
                // Its name is a `<dashed-ident>`, else the rule is none.
                "font-palette-values"
                    if declarations && style::palette_values_name(&at.prelude).is_some() =>
                {
                    Some(Kind::FontPaletteValues)
                }
                "property" => match &at.block {
                    Some(Block::Declarations(list))
                        if style::property_rule(&at.prelude, list).is_some() =>
                    {
                        Some(Kind::Property)
                    }
                    _ => None,
                },
                _ => None,
            }
        }
    }
}

/// Whether an `@import` prelude begins with its URL, a `url()` or a
/// string.
fn starts_with_url(prelude: &[Token]) -> bool {
    match prelude.iter().find(|t| !matches!(t, Token::Whitespace)) {
        Some(Token::Url { .. } | Token::String { .. }) => true,
        Some(Token::Function(name)) => name.eq_ignore_ascii_case("url"),
        _ => false,
    }
}

/// The rules of `list`, at `place`, a script sees: each one's index in
/// the list and its kind.
fn visible(list: &[Rule], place: Place, ns: &Namespaces) -> Vec<(usize, Kind)> {
    list.iter()
        .enumerate()
        .filter_map(|(at, rule)| kind(rule, place, ns).map(|kind| (at, kind)))
        .collect()
}

/// The rule list `path` names in `sheet`: the sheet's, or a grouping
/// rule's, a style rule's nested rules among them, by each index a
/// script sees, outermost first; where it stands; and the sheet's
/// namespaces, which its selectors are read with.
fn list<'a>(sheet: &'a Stylesheet, path: &[usize]) -> Option<(&'a Vec<Rule>, Place, Namespaces)> {
    let ns = style::sheet_namespaces(&sheet.rules);
    let mut rules = &sheet.rules;
    let mut place = TOP;
    for &index in path {
        let (at, kind) = *visible(rules, place, &ns).get(index)?;
        rules = match &rules[at] {
            Rule::At(css::AtRule {
                block: Some(Block::Rules(inner)),
                ..
            }) => inner,
            Rule::Qualified(rule) if kind == Kind::Style => &rule.rules,
            _ => return None,
        };
        place = place.inside(kind);
    }
    Some((rules, place, ns))
}

fn list_mut<'a>(
    sheet: &'a mut Stylesheet,
    path: &[usize],
) -> Option<(&'a mut Vec<Rule>, Place, Namespaces)> {
    let ns = style::sheet_namespaces(&sheet.rules);
    let mut rules = &mut sheet.rules;
    let mut place = TOP;
    for &index in path {
        let (at, kind) = *visible(rules, place, &ns).get(index)?;
        rules = match &mut rules[at] {
            Rule::At(css::AtRule {
                block: Some(Block::Rules(inner)),
                ..
            }) => inner,
            Rule::Qualified(rule) if kind == Kind::Style => &mut rule.rules,
            _ => return None,
        };
        place = place.inside(kind);
    }
    Some((rules, place, ns))
}

/// The rules of the list `path` names, four strings each: the kind's
/// name, its `cssText`, and two by kind (a style or page rule's
/// selector and declarations, an import's URL and media, a media or
/// supports rule's condition, a keyframes or layer rule's name, a
/// keyframe's key and declarations, a font-face rule's declarations).
pub(crate) fn rules(sheet: &Stylesheet, path: &[usize]) -> Vec<String> {
    let Some((list, place, ns)) = list(sheet, path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (at, kind) in visible(list, place, &ns) {
        let rule = &list[at];
        let (a, b) = parts(rule, kind, place, &ns);
        out.extend([
            kind.name().to_owned(),
            css_text(rule, kind, "", place, &ns),
            a,
            b,
        ]);
    }
    out
}

/// The two strings by kind [`rules`] gives, of `rule` in a list at
/// `place`: a nested declarations rule's declarations second; an
/// `@scope` rule's roots and limits, each after a `1` when it has them.
fn parts(rule: &Rule, kind: Kind, place: Place, ns: &Namespaces) -> (String, String) {
    match (rule, kind) {
        (Rule::Qualified(rule), Kind::Keyframe) => {
            (key_text(&rule.prelude), declarations(&rule.declarations))
        }
        (Rule::Qualified(rule), _) => (
            nested_selector_text(&rule.prelude, ns, place.relative),
            declarations(&rule.declarations),
        ),
        (Rule::Declarations(list), _) => (String::new(), declarations(list)),
        (Rule::At(at), Kind::Scope) => {
            let (start, end) = scope_texts(&at.prelude, place, ns);
            let flagged = |part: Option<String>| part.map_or_else(String::new, |p| format!("1{p}"));
            (flagged(start), flagged(end))
        }
        // The name; then the syntax, `1` when it inherits, and the initial
        // value after a `1` when it has one, parted by NULs.
        (Rule::At(at), Kind::Property) => match property(at) {
            Some(registration) => (
                registration.name.clone(),
                format!(
                    "{}\u{0}{}\u{0}{}",
                    registration.syntax_text,
                    u8::from(registration.inherits),
                    registration
                        .initial_text
                        .as_deref()
                        .map_or_else(String::new, |v| format!("1{}", initial_text(v)))
                ),
            ),
            None => (String::new(), String::new()),
        },
        (Rule::At(at), Kind::Namespace) => {
            let (prefix, uri) = style::namespace_declaration(&at.prelude).unwrap_or_default();
            (prefix.unwrap_or_default(), uri)
        }
        (Rule::At(at), Kind::LayerStatement) => (layer_names(&at.prelude), String::new()),
        // The media, then the layer and the `supports()` condition, each
        // after a NUL and a `1` when the rule has it.
        (Rule::At(at), Kind::Import) => {
            let (url, _) = import_url(&at.prelude);
            let (layer, supports, media) = import_conditions(&at.prelude);
            let flagged = |part: Option<String>| part.map_or_else(String::new, |p| format!("1{p}"));
            (
                url,
                format!("{media}\u{0}{}\u{0}{}", flagged(layer), flagged(supports)),
            )
        }
        (Rule::At(at), Kind::Media) => (media_text(&at.prelude), String::new()),
        // The condition, as the reference browser writes it (D72).
        // Then each query's name and condition, all parted by NULs.
        (Rule::At(at), Kind::Container) => {
            let (condition, queries) =
                style::container::container_prelude(&at.prelude).unwrap_or_default();
            let parts: Vec<String> = queries
                .into_iter()
                .flat_map(|(name, query)| [name, query])
                .collect();
            (condition, parts.join("\u{0}"))
        }
        (Rule::At(at), Kind::CounterStyle) => (
            counter_style_name(&at.prelude).unwrap_or_default(),
            match &at.block {
                Some(Block::Declarations(list)) => raw_declarations(list),
                _ => String::new(),
            },
        ),
        (Rule::At(at), Kind::FontFeatureValues) => {
            (selector(&at.prelude), feature_values(&at.block))
        }
        // The name; then the family, the base palette and the colours,
        // each after a `1` when the rule has it, parted by NULs.
        (Rule::At(at), Kind::FontPaletteValues) => {
            let values = palette_values(&at.block);
            let flagged =
                |part: &Option<String>| part.as_ref().map_or_else(String::new, |p| format!("1{p}"));
            (
                style::palette_values_name(&at.prelude).unwrap_or_default(),
                format!(
                    "{}\u{0}{}\u{0}{}",
                    flagged(&values.font_family),
                    flagged(&values.base_palette),
                    flagged(&values.override_colors)
                ),
            )
        }
        (Rule::At(at), Kind::FontFace | Kind::Page) => {
            let block = match &at.block {
                Some(Block::Declarations(list)) if kind == Kind::FontFace => descriptors(list),
                Some(Block::Declarations(list)) => raw_declarations(list),
                _ => String::new(),
            };
            (selector(&at.prelude), block)
        }
        (Rule::At(at), _) => (selector(&at.prelude), String::new()),
    }
}

/// Tokens as a selector or condition: white space as one space,
/// trimmed.
fn selector(tokens: &[css::Token]) -> String {
    style::serialize::tokens(tokens).trim().to_owned()
}

/// A layer statement's names, parted by `, `.
fn layer_names(tokens: &[Token]) -> String {
    selector(tokens)
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(", ")
}

/// A style rule's selector, where `relative` says it stands, as the
/// reference browser serializes it: one nested in a style rule that
/// begins with a combinator or names no `&` with its `&` written (CSS
/// Nesting 1 section 2.2); one in `@scope` as written.
fn nested_selector_text(tokens: &[Token], ns: &Namespaces, relative: Relative) -> String {
    if relative != Relative::Nesting {
        return selector_text(tokens, ns);
    }
    let mut written: Vec<Token> = Vec::new();
    for (n, member) in split_top(tokens, |t| matches!(t, Token::Comma))
        .into_iter()
        .enumerate()
    {
        if n > 0 {
            written.push(Token::Comma);
        }
        let member = trim_space(member);
        let leading = matches!(member.first(), Some(Token::Delim('>' | '+' | '~')));
        if leading || !member.contains(&Token::Delim('&')) {
            written.extend([Token::Delim('&'), Token::Whitespace]);
        }
        written.extend_from_slice(member);
    }
    selector_text(&written, ns)
}

/// The texts of an `@scope` rule's roots and limits, at `place`, as
/// the reference browser serializes them; `None` for those it has not.
fn scope_texts(
    prelude: &[Token],
    place: Place,
    ns: &Namespaces,
) -> (Option<String>, Option<String>) {
    let (start, end) = style::scope_selectors(prelude, ns, place.relative).unwrap_or_default();
    (
        start.map(|tokens| selector_text(tokens, ns)),
        end.map(|tokens| selector_text(tokens, ns)),
    )
}

/// A style rule's selector as CSSOM serializes it (section 6.7.1): each
/// simple selector in its canonical form, read with the sheet's
/// namespaces; as written, but for its spacing, where it is past what
/// [`canonical_selector`] reads.
fn selector_text(tokens: &[Token], ns: &Namespaces) -> String {
    let mut out = String::new();
    match canonical_selector(tokens, ns, &mut out) {
        Some(()) => out,
        None => spaced_selector(tokens),
    }
}

/// `tokens`, a selector list, written as CSSOM serializes it into
/// `out`; `None` for a part it does not read.
fn canonical_selector(tokens: &[Token], ns: &Namespaces, out: &mut String) -> Option<()> {
    let mut first = true;
    for member in split_top(tokens, |t| matches!(t, Token::Comma)) {
        if !first {
            out.push_str(", ");
        }
        first = false;
        complex(member, ns, out)?;
    }
    Some(())
}

/// The pieces of `tokens` between those `at` says part them, outside
/// brackets and functions.
fn split_top(tokens: &[Token], at: impl Fn(&Token) -> bool) -> Vec<&[Token]> {
    let mut pieces = Vec::new();
    let (mut depth, mut start) = (0usize, 0);
    for (i, t) in tokens.iter().enumerate() {
        match t {
            Token::Function(_) | Token::OpenParen | Token::OpenSquare => depth += 1,
            Token::CloseParen | Token::CloseSquare => depth = depth.saturating_sub(1),
            _ if depth == 0 && at(t) => {
                pieces.push(&tokens[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    pieces.push(&tokens[start..]);
    pieces
}

/// One complex selector: its compounds, a combinator between each.
fn complex(tokens: &[Token], ns: &Namespaces, out: &mut String) -> Option<()> {
    let tokens = trim_space(tokens);
    if tokens.is_empty() {
        return None;
    }
    let mut i = 0;
    let mut wrote = false;
    while i < tokens.len() {
        // A combinator, white space around it.
        let mut spaced = false;
        while matches!(tokens.get(i), Some(Token::Whitespace)) {
            spaced = true;
            i += 1;
        }
        if let Some(Token::Delim(c @ ('>' | '+' | '~'))) = tokens.get(i) {
            // A relative selector's leading one has nothing before it.
            if wrote {
                out.push(' ');
            }
            out.push_str(&format!("{c} "));
            i += 1;
            while matches!(tokens.get(i), Some(Token::Whitespace)) {
                i += 1;
            }
        } else if spaced && wrote {
            out.push(' ');
        }
        let end = (i..tokens.len())
            .find(|&j| {
                matches!(tokens[j], Token::Whitespace | Token::Delim('>' | '+' | '~'))
                    && depth_at(tokens, j) == 0
            })
            .unwrap_or(tokens.len());
        if end == i {
            return None;
        }
        compound(&tokens[i..end], ns, out)?;
        wrote = true;
        i = end;
    }
    Some(())
}

/// How deep in brackets and functions `tokens[at]` is.
fn depth_at(tokens: &[Token], at: usize) -> usize {
    let mut depth = 0usize;
    for t in &tokens[..at] {
        match t {
            Token::Function(_) | Token::OpenParen | Token::OpenSquare => depth += 1,
            Token::CloseParen | Token::CloseSquare => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    depth
}

fn trim_space(mut tokens: &[Token]) -> &[Token] {
    while let [Token::Whitespace, rest @ ..] = tokens {
        tokens = rest;
    }
    while let [rest @ .., Token::Whitespace] = tokens {
        tokens = rest;
    }
    tokens
}

/// How a type or universal selector's namespace was written.
enum Prefix {
    /// No prefix.
    Unwritten,
    /// `*|`.
    Any,
    /// `|`.
    NoNamespace,
    /// A declared prefix.
    Named(String),
}

/// One compound selector: its type or universal selector with the
/// prefix that says something, the universal one left out before other
/// simple selectors, and each of the others canonical.
fn compound(tokens: &[Token], ns: &Namespaces, out: &mut String) -> Option<()> {
    let name_at = |i: usize| match tokens.get(i) {
        Some(Token::Ident(name)) => Some(ident(name)),
        Some(Token::Delim('*')) => Some("*".to_owned()),
        _ => None,
    };
    let bar = |i: usize| matches!(tokens.get(i), Some(Token::Delim('|')));
    let (prefix, element, mut i) = match tokens.first() {
        Some(Token::Delim('|')) => (Prefix::NoNamespace, Some(name_at(1)?), 2),
        Some(Token::Delim('*')) if bar(1) => (Prefix::Any, Some(name_at(2)?), 3),
        Some(Token::Ident(p)) if bar(1) => (Prefix::Named(p.clone()), Some(name_at(2)?), 3),
        Some(Token::Ident(_) | Token::Delim('*')) => (Prefix::Unwritten, name_at(0), 1),
        _ => (Prefix::Unwritten, None, 0),
    };
    let mut rest = String::new();
    while i < tokens.len() {
        i = simple(tokens, i, ns, &mut rest)?;
    }
    let prefix = match prefix {
        Prefix::Unwritten => String::new(),
        Prefix::Any if ns.default.is_none() => String::new(),
        Prefix::Any => "*|".to_owned(),
        Prefix::NoNamespace => "|".to_owned(),
        Prefix::Named(p) => {
            let named = ns.prefix(&p)?;
            if ns.default.as_ref() == Some(named) {
                String::new()
            } else {
                format!("{}|", ident(&p))
            }
        }
    };
    if let Some(element) = element
        && !(element == "*" && prefix.is_empty() && !rest.is_empty())
    {
        out.push_str(&prefix);
        out.push_str(&element);
    }
    out.push_str(&rest);
    Some(())
}

/// The simple selector at `tokens[i]` written into `out`; where the next
/// starts.
fn simple(tokens: &[Token], i: usize, ns: &Namespaces, out: &mut String) -> Option<usize> {
    match (tokens.get(i)?, tokens.get(i + 1)) {
        // The nesting selector (CSS Nesting 1 section 4).
        (Token::Delim('&'), _) => {
            out.push('&');
            Some(i + 1)
        }
        (Token::Hash { value, .. }, _) => {
            out.push('#');
            out.push_str(&ident(value));
            Some(i + 1)
        }
        (Token::Delim('.'), Some(Token::Ident(name))) => {
            out.push('.');
            out.push_str(&ident(name));
            Some(i + 2)
        }
        (Token::OpenSquare, _) => {
            let close = (i..tokens.len()).find(|&j| matches!(tokens[j], Token::CloseSquare))?;
            attribute(trim_space(&tokens[i + 1..close]), out)?;
            Some(close + 1)
        }
        (Token::Colon, Some(Token::Colon)) => {
            out.push_str("::");
            pseudo(tokens, i + 2, ns, out)
        }
        (Token::Colon, Some(Token::Ident(name)))
            if matches!(
                name.to_ascii_lowercase().as_str(),
                "before" | "after" | "first-line" | "first-letter"
            ) =>
        {
            out.push_str("::");
            out.push_str(&name.to_ascii_lowercase());
            Some(i + 2)
        }
        (Token::Colon, _) => {
            out.push(':');
            pseudo(tokens, i + 1, ns, out)
        }
        _ => None,
    }
}

/// A pseudo-class's or pseudo-element's name at `tokens[i]` and its
/// arguments: selectors and `An+B` canonical, others as written.
fn pseudo(tokens: &[Token], i: usize, ns: &Namespaces, out: &mut String) -> Option<usize> {
    match tokens.get(i)? {
        Token::Ident(name) => {
            out.push_str(&name.to_ascii_lowercase());
            Some(i + 1)
        }
        Token::Function(name) => {
            let name = name.to_ascii_lowercase();
            let mut depth = 1usize;
            let mut close = i + 1;
            while close < tokens.len() {
                match tokens[close] {
                    Token::Function(_) | Token::OpenParen => depth += 1,
                    Token::CloseParen => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                close += 1;
            }
            let inner = trim_space(tokens.get(i + 1..close)?);
            out.push_str(&name);
            out.push('(');
            match name.as_str() {
                "not" | "is" | "where" | "has" | "matches" | "-webkit-any" => {
                    canonical_selector(inner, ns, out)?
                }
                "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type" => {
                    let parts = split_top(
                        inner,
                        |t| matches!(t, Token::Ident(w) if w.eq_ignore_ascii_case("of")),
                    );
                    out.push_str(&an_plus_b(parts.first()?)?);
                    if let Some(of) = parts.get(1) {
                        out.push_str(" of ");
                        canonical_selector(trim_space(of), ns, out)?;
                    }
                }
                _ => {
                    for t in inner {
                        match t {
                            Token::Ident(word) => out.push_str(&ident(word)),
                            Token::Whitespace => out.push(' '),
                            other => {
                                out.push_str(&style::serialize::tokens(std::slice::from_ref(other)))
                            }
                        }
                    }
                }
            }
            out.push(')');
            Some(close + 1)
        }
        _ => None,
    }
}

/// An attribute selector's inside: its prefix that says something, its
/// name, operator, value as a string, and flag.
fn attribute(tokens: &[Token], out: &mut String) -> Option<()> {
    let (prefix, at) = match tokens {
        [Token::Delim('|'), Token::Ident(_), ..] => (String::new(), 1),
        [Token::Delim('*'), Token::Delim('|'), Token::Ident(_), ..] => ("*|".to_owned(), 2),
        [Token::Ident(p), Token::Delim('|'), Token::Ident(_), ..] => (format!("{}|", ident(p)), 2),
        _ => (String::new(), 0),
    };
    let Some(Token::Ident(name)) = tokens.get(at) else {
        return None;
    };
    out.push('[');
    out.push_str(&prefix);
    out.push_str(&ident(name));
    let rest: Vec<&Token> = tokens[at + 1..]
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .collect();
    if !rest.is_empty() {
        let mut k = 0;
        let mut op = String::new();
        while let Some(Token::Delim(c)) = rest.get(k) {
            op.push(*c);
            k += 1;
        }
        if !op.ends_with('=') || op.len() > 2 {
            return None;
        }
        let value = match rest.get(k) {
            Some(Token::Ident(v) | Token::String { value: v, .. }) => v,
            _ => return None,
        };
        k += 1;
        let flag = match rest.get(k) {
            Some(Token::Ident(f)) => {
                k += 1;
                Some(f.to_ascii_lowercase())
            }
            _ => None,
        };
        if k != rest.len() {
            return None;
        }
        out.push_str(&op);
        out.push_str(&style::serialize::tokens(&[Token::String {
            value: value.clone(),
            bad: false,
        }]));
        if let Some(flag) = flag {
            out.push(' ');
            out.push_str(&flag);
        }
    }
    out.push(']');
    Some(())
}

/// An `An+B` argument at its shortest (CSS Syntax 3 section 9):
/// `even` as `2n`, `odd` as `2n+1`, a coefficient of one left out.
fn an_plus_b(tokens: &[Token]) -> Option<String> {
    let text: String = style::serialize::tokens(tokens)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    let (a, b) = match text.as_str() {
        "even" => (2, 0),
        "odd" => (2, 1),
        _ => match text.find('n') {
            None => (0, text.parse::<i64>().ok()?),
            Some(at) => {
                let a = match &text[..at] {
                    "" | "+" => 1,
                    "-" => -1,
                    a => a.parse::<i64>().ok()?,
                };
                let b = match &text[at + 1..] {
                    "" => 0,
                    b => b.strip_prefix('+').unwrap_or(b).parse::<i64>().ok()?,
                };
                (a, b)
            }
        },
    };
    let mut out = match a {
        0 => return Some(b.to_string()),
        1 => "n".to_owned(),
        -1 => "-n".to_owned(),
        a => format!("{a}n"),
    };
    if b > 0 {
        out.push_str(&format!("+{b}"));
    } else if b < 0 {
        out.push_str(&b.to_string());
    }
    Some(out)
}

/// CSSOM's "serialize an identifier" (section 2.1).
fn ident(name: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '\0' => out.push('\u{fffd}'),
            '\u{1}'..='\u{1f}' | '\u{7f}' => out.push_str(&format!("\\{:x} ", c as u32)),
            '0'..='9' if i == 0 || (i == 1 && chars[0] == '-') => {
                out.push_str(&format!("\\{:x} ", c as u32));
            }
            '-' if i == 0 && chars.len() == 1 => out.push_str("\\-"),
            c if c as u32 >= 0x80 || c.is_ascii_alphanumeric() || c == '-' || c == '_' => {
                out.push(c);
            }
            c => {
                out.push('\\');
                out.push(c);
            }
        }
    }
    out
}

/// A selector's tokens parted as CSSOM spaces them: its selectors by
/// `, ` and combinators by spaces, as written otherwise.
fn spaced_selector(tokens: &[Token]) -> String {
    let raw = selector(tokens);
    let mut out = String::new();
    let mut depth = 0usize;
    for c in raw.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth == 0 && matches!(c, ',' | '>' | '+' | '~') {
            while out.ends_with(' ') {
                out.pop();
            }
            if c == ',' {
                out.push_str(", ");
            } else {
                out.push(' ');
                out.push(c);
                out.push(' ');
            }
            continue;
        }
        if c == ' ' && out.ends_with(' ') {
            continue;
        }
        out.push(c);
    }
    out.trim().to_owned()
}

/// A keyframe's keys, `from` and `to` as percentages.
fn key_text(tokens: &[css::Token]) -> String {
    selector(tokens)
        .split(',')
        .map(|key| match key.trim().to_ascii_lowercase().as_str() {
            "from" => "0%".to_owned(),
            "to" => "100%".to_owned(),
            _ => key.trim().to_owned(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// An import's URL, and what follows it.
fn import_parts(prelude: &[css::Token]) -> (String, String) {
    let (url, rest) = import_url(prelude);
    (url, selector(rest))
}

/// An `@import` prelude's URL, given as a `url()` or a string, and the
/// tokens after it.
fn import_url(prelude: &[Token]) -> (String, &[Token]) {
    let start = prelude
        .iter()
        .position(|t| !matches!(t, Token::Whitespace))
        .unwrap_or(prelude.len());
    match prelude.get(start..).unwrap_or_default() {
        [
            Token::Url { value, .. } | Token::String { value, .. },
            rest @ ..,
        ] => (value.clone(), rest),
        [Token::Function(f), rest @ ..] if f.eq_ignore_ascii_case("url") => {
            let close = rest
                .iter()
                .position(|t| matches!(t, Token::CloseParen))
                .unwrap_or(rest.len());
            let url = rest[..close]
                .iter()
                .find_map(|t| match t {
                    Token::String { value, .. } => Some(value.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            (url, rest.get(close + 1..).unwrap_or_default())
        }
        _ => (String::new(), &[]),
    }
}

/// An `@import` rule's conditions after its URL: its layer (`Some("")`
/// for an anonymous one), its `supports()` condition, and its media.
fn import_conditions(prelude: &[Token]) -> (Option<String>, Option<String>, String) {
    let (_, mut rest) = import_url(prelude);
    rest = trim_space(rest);
    let mut layer = None;
    match rest.first() {
        Some(Token::Ident(word)) if word.eq_ignore_ascii_case("layer") => {
            layer = Some(String::new());
            rest = trim_space(&rest[1..]);
        }
        Some(Token::Function(f)) if f.eq_ignore_ascii_case("layer") => {
            let close = closing(rest);
            layer = Some(selector(&rest[1..close]).replace(' ', ""));
            rest = trim_space(rest.get(close + 1..).unwrap_or_default());
        }
        _ => {}
    }
    let mut supports = None;
    if let Some(Token::Function(f)) = rest.first()
        && f.eq_ignore_ascii_case("supports")
    {
        let close = closing(rest);
        let inner = trim_space(&rest[1..close]);
        // A declaration alone is written in parentheses.
        let text = selector(inner);
        let declaration = matches!(inner, [Token::Ident(_), ..])
            && inner.iter().any(|t| matches!(t, Token::Colon));
        supports = Some(if declaration {
            format!("({text})")
        } else {
            text
        });
        rest = trim_space(rest.get(close + 1..).unwrap_or_default());
    }
    (layer, supports, media_text(rest))
}

/// The index of the `)` that closes the function at `tokens[0]`.
fn closing(tokens: &[Token]) -> usize {
    let mut depth = 0usize;
    for (i, t) in tokens.iter().enumerate() {
        match t {
            Token::Function(_) | Token::OpenParen => depth += 1,
            Token::CloseParen => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    tokens.len()
}

/// A media query list as CSSOM serializes it (section 4.2): each query
/// normalized, `all and` before a condition left out, invalid ones as
/// `not all`.
pub(crate) fn media_text(tokens: &[Token]) -> String {
    media_items(tokens).join(", ")
}

/// [`media_text`]'s queries, each alone.
pub(crate) fn media_items(tokens: &[Token]) -> Vec<String> {
    if trim_space(tokens).is_empty() {
        return Vec::new();
    }
    split_top(tokens, |t| matches!(t, Token::Comma))
        .into_iter()
        .map(|query| {
            let text = style::serialize_media_query_list(trim_space(query));
            let text = if text.is_empty() {
                "not all".to_owned()
            } else {
                text
            };
            match text.strip_prefix("all and (") {
                Some(rest) => format!("({rest}"),
                None => text,
            }
        })
        .collect()
}

/// A style rule's or keyframe's declaration block's text as CSSOM
/// serializes it (G50): those the cascade takes, in their canonical
/// forms, as a `style` attribute's are.
pub(crate) fn declarations(list: &[Declaration]) -> String {
    super::cssom::block_text(&super::cssom::declarations(&raw_declarations(list)))
}

/// A declaration block's text: `name: value;` each, `!important` kept.
pub(crate) fn raw_declarations(list: &[Declaration]) -> String {
    let mut out = String::new();
    for declaration in list {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&declaration.name);
        out.push_str(": ");
        out.push_str(style::serialize::tokens(&declaration.value).trim());
        if declaration.important {
            out.push_str(" !important");
        }
        out.push(';');
    }
    out
}

/// What an `@font-palette-values` rule's block declares.
fn palette_values(block: &Option<Block>) -> style::PaletteValues {
    match block {
        Some(Block::Declarations(list)) => style::PaletteValues::of(list),
        _ => style::PaletteValues::default(),
    }
}

/// An `@font-face` rule's descriptors' text (CSS Fonts 4 section 4):
/// `name: value;` each, those the rule takes written back as the
/// reference browser does, a descriptor given again in its later place.
pub(crate) fn descriptors(list: &[Declaration]) -> String {
    descriptor_list(list)
        .iter()
        .map(|(name, value)| format!("{name}: {value};"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// An `@font-face` rule's descriptors as [`descriptors`] writes them,
/// each its name and value.
pub(crate) fn descriptor_list(list: &[Declaration]) -> Vec<(&'static str, String)> {
    let mut kept: Vec<(&str, String)> = Vec::new();
    for declaration in list.iter().filter(|d| !d.important) {
        if let Some((name, value)) =
            style::font_face::descriptor(&declaration.name, &declaration.value)
        {
            kept.retain(|(was, _)| *was != name);
            kept.push((name, value));
        }
    }
    kept
}

/// A rule's `cssText`, in a list at `place`, each nested line after the
/// first indented by `indent` more. A style rule with nested rules has
/// its declarations and each nested rule on a line of their own, those
/// rules' own lines not indented further, as the reference browser has
/// them.
fn css_text(rule: &Rule, kind: Kind, indent: &str, place: Place, ns: &Namespaces) -> String {
    let block = |head: String, list: &[Declaration]| {
        let body = match kind {
            Kind::FontFace => descriptors(list),
            Kind::Page => raw_declarations(list),
            _ => declarations(list),
        };
        if body.is_empty() {
            format!("{head} {{ }}")
        } else {
            format!("{head} {{ {body} }}")
        }
    };
    // The rules nested in a grouping rule of `kind`, each on its line.
    let children = |inner: &[Rule], out: &mut String| {
        let deeper = format!("{indent}  ");
        let inside = place.inside(kind);
        for (at, kind) in visible(inner, inside, ns) {
            out.push_str(&deeper);
            out.push_str(&css_text(&inner[at], kind, &deeper, inside, ns));
            out.push('\n');
        }
    };
    match (rule, kind) {
        (Rule::Qualified(rule), Kind::Keyframe) => {
            block(key_text(&rule.prelude), &rule.declarations)
        }
        (Rule::Qualified(rule), _) => {
            let selector = nested_selector_text(&rule.prelude, ns, place.relative);
            let inside = place.inside(kind);
            let nested = visible(&rule.rules, inside, ns);
            if nested.is_empty() {
                return block(selector, &rule.declarations);
            }
            let mut out = format!("{selector} {{");
            let body = declarations(&rule.declarations);
            if !body.is_empty() {
                out.push_str("\n  ");
                out.push_str(&body);
            }
            for (at, kind) in nested {
                out.push_str("\n  ");
                out.push_str(&css_text(&rule.rules[at], kind, "", inside, ns));
            }
            out.push_str("\n}");
            out
        }
        (Rule::Declarations(list), _) => declarations(list),
        // Its descriptors in their order, as the reference browser writes
        // them, whatever order they were given in.
        (Rule::At(at), Kind::Property) => match property(at) {
            Some(registration) => {
                let syntax = registration
                    .syntax_text
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"");
                let mut out = format!(
                    "@property {} {{ syntax: \"{syntax}\"; inherits: {};",
                    ident(&registration.name),
                    registration.inherits
                );
                if let Some(initial) = &registration.initial_text {
                    out.push_str(&format!(" initial-value: {};", initial_text(initial)));
                }
                out.push_str(" }");
                out
            }
            None => String::new(),
        },
        (Rule::At(at), Kind::Scope) => {
            let (start, end) = scope_texts(&at.prelude, place, ns);
            let mut out = "@scope".to_owned();
            if let Some(start) = start {
                out.push_str(&format!(" ({start})"));
            }
            if let Some(end) = end {
                out.push_str(&format!(" to ({end})"));
            }
            out.push_str(" {\n");
            if let Some(Block::Rules(inner)) = &at.block {
                children(inner, &mut out);
            }
            out.push_str(indent);
            out.push('}');
            out
        }
        (Rule::At(at), Kind::Import) => {
            let (url, rest) = import_parts(&at.prelude);
            let url = url.replace('\\', "\\\\").replace('"', "\\\"");
            if rest.is_empty() {
                format!("@import url(\"{url}\");")
            } else {
                format!("@import url(\"{url}\") {rest};")
            }
        }
        (Rule::At(at), Kind::Media) => {
            let mut out = format!("@media {} {{\n", media_text(&at.prelude));
            if let Some(Block::Rules(inner)) = &at.block {
                children(inner, &mut out);
            }
            out.push_str(indent);
            out.push('}');
            out
        }
        (Rule::At(at), Kind::Namespace) => {
            let (prefix, uri) = style::namespace_declaration(&at.prelude).unwrap_or_default();
            let uri = uri.replace('\\', "\\\\").replace('"', "\\\"");
            match prefix {
                Some(prefix) => format!("@namespace {} url(\"{uri}\");", ident(&prefix)),
                None => format!("@namespace url(\"{uri}\");"),
            }
        }
        (Rule::At(at), Kind::LayerStatement) => format!("@layer {};", layer_names(&at.prelude)),
        (Rule::At(at), Kind::FontFeatureValues) => {
            let body = feature_values_text(&feature_values(&at.block));
            format!("@font-feature-values {} {{ {body}}}", selector(&at.prelude))
        }
        // Its descriptors in their order, as the reference browser
        // writes them, whatever order they were given in.
        (Rule::At(at), Kind::FontPaletteValues) => {
            let name = style::palette_values_name(&at.prelude).unwrap_or_default();
            let body = palette_values(&at.block).text();
            if body.is_empty() {
                format!("@font-palette-values {name} {{ }}")
            } else {
                format!("@font-palette-values {name} {{ {body} }}")
            }
        }
        (Rule::At(at), Kind::FontFace | Kind::Page | Kind::CounterStyle) => {
            let prelude = selector(&at.prelude);
            let head = if prelude.is_empty() {
                format!("@{}", at.name)
            } else {
                format!("@{} {prelude}", at.name)
            };
            match &at.block {
                Some(Block::Declarations(list)) => block(head, list),
                _ => format!("{head} {{ }}"),
            }
        }
        (Rule::At(at), _) => {
            let prelude = match kind {
                Kind::Container => style::container::container_prelude(&at.prelude)
                    .map(|(condition, _)| condition)
                    .unwrap_or_default(),
                _ => selector(&at.prelude),
            };
            let mut out = if prelude.is_empty() {
                format!("@{} {{\n", at.name)
            } else {
                format!("@{} {prelude} {{\n", at.name)
            };
            if let Some(Block::Rules(inner)) = &at.block {
                children(inner, &mut out);
            }
            out.push_str(indent);
            out.push('}');
            out
        }
    }
}

/// The registration an `@property` rule makes, if it is valid.
fn property(at: &css::AtRule) -> Option<style::PropertyRegistration> {
    match &at.block {
        Some(Block::Declarations(list)) => style::property_rule(&at.prelude, list),
        _ => None,
    }
}

/// An `@property` rule's initial value as written back.
fn initial_text(tokens: &[Token]) -> String {
    // Each identifier as CSSOM serializes one, escaped.
    let escaped: Vec<Token> = tokens
        .iter()
        .map(|token| match token {
            Token::Ident(name) => Token::Ident(ident(name)),
            other => other.clone(),
        })
        .collect();
    style::serialize::tokens(&escaped).trim().to_owned()
}

/// `text` read as the one rule CSSOM's "insert a CSS rule" inserts into
/// a list at `place`: in a style rule, or a block read as one, a rule
/// nesting allows or a run of declarations, at least one of them one
/// the cascade takes (CSS Nesting 1 section 3.3); elsewhere a rule.
/// An at-rule nesting does not allow is a `HierarchyRequestError`.
fn parse_one_rule(text: &str, place: Place) -> Result<Rule, DomError> {
    let syntax = || DomError::new("SyntaxError", format!("Failed to parse the rule '{text}'."));
    if place.relative == Relative::No {
        let parsed = css::parse_stylesheet(text).map_err(|_| syntax())?;
        return <[Rule; 1]>::try_from(parsed.rules)
            .map(|[rule]| rule)
            .map_err(|_| syntax());
    }
    match <[Rule; 1]>::try_from(css::parse_block_contents(text)) {
        Ok([Rule::Declarations(list)]) if declarations(&list).is_empty() => Err(syntax()),
        Ok([rule]) => Ok(rule),
        Err(rules) if rules.is_empty() => {
            match css::parse_stylesheet(text).map(|sheet| sheet.rules) {
                Ok(rules) if matches!(rules.as_slice(), [Rule::At(_)]) => Err(DomError::new(
                    "HierarchyRequestError",
                    "Failed to insert the rule: it may not be nested.",
                )),
                _ => Err(syntax()),
            }
        }
        Err(_) => Err(syntax()),
    }
}

/// CSSOM's `insertRule` (section 6.1.2.1, "insert a CSS rule"): `text`
/// parsed as one rule put before the rule a script sees at `index` in
/// the list `path` names; its index, and each rule's old index in the
/// sheet's own list (for the imports to follow).
pub(crate) fn insert(
    sheet: &mut Stylesheet,
    path: &[usize],
    text: &str,
    index: usize,
) -> Result<(usize, Vec<Option<usize>>), DomError> {
    let syntax = || DomError::new("SyntaxError", format!("Failed to parse the rule '{text}'."));
    let top = sheet.rules.len();
    let (list, place, ns) = list_mut(sheet, path).ok_or_else(syntax)?;
    let rule = parse_one_rule(text, place)?;
    let nested = !path.is_empty();
    // An import or a namespace rule in a grouping rule is no rule there.
    if nested
        && let Rule::At(at) = &rule
        && matches!(
            at.name.to_ascii_lowercase().as_str(),
            "import" | "namespace"
        )
    {
        return Err(DomError::new(
            "HierarchyRequestError",
            "Failed to insert the rule.",
        ));
    }
    let new_kind = kind(&rule, place, &ns).ok_or_else(syntax)?;
    let seen = visible(list, place, &ns);
    if index > seen.len() {
        return Err(DomError::new(
            "IndexSizeError",
            format!(
                "The index provided ({index}) is larger than the maximum index ({}).",
                seen.len()
            ),
        ));
    }
    if list.len() >= MAX_RULES {
        return Err(DomError::new(
            "QuotaExceededError",
            "The sheet holds too many rules.",
        ));
    }
    let hierarchy = || DomError::new("HierarchyRequestError", "Failed to insert the rule.");
    // A namespace rule only into a list of imports and namespace rules
    // (section 6.1.2.1).
    let preamble = |k: &Kind| matches!(k, Kind::Import | Kind::Namespace);
    if new_kind == Kind::Namespace && path.is_empty() && !seen.iter().all(|(_, k)| preamble(k)) {
        return Err(DomError::new(
            "InvalidStateError",
            "Failed to insert the rule: a namespace rule after other rules.",
        ));
    }
    // An import only before every rule but other imports (section
    // 6.1.2.1's constraints), a namespace rule only after them and
    // before the rest; neither inside a grouping rule.
    if preamble(&new_kind) && !path.is_empty() {
        return Err(hierarchy());
    }
    if new_kind == Kind::Import && seen[..index].iter().any(|(_, k)| *k != Kind::Import) {
        return Err(hierarchy());
    }
    if new_kind != Kind::Import && seen[index..].iter().any(|(_, k)| *k == Kind::Import) {
        return Err(hierarchy());
    }
    if !preamble(&new_kind) && seen[index..].iter().any(|(_, k)| *k == Kind::Namespace) {
        return Err(hierarchy());
    }
    let at = seen.get(index).map_or(list.len(), |(at, _)| *at);
    list.insert(at, rule);
    let kept = if path.is_empty() {
        (0..top + 1)
            .map(|i| match i.cmp(&at) {
                std::cmp::Ordering::Less => Some(i),
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some(i - 1),
            })
            .collect()
    } else {
        (0..top).map(Some).collect()
    };
    Ok((index, kept))
}

/// CSSOM's `deleteRule` ("remove a CSS rule"): the rule a script sees at
/// `index` in the list `path` names taken out; each rule's old index in
/// the sheet's own list.
pub(crate) fn delete(
    sheet: &mut Stylesheet,
    path: &[usize],
    index: usize,
) -> Result<Vec<Option<usize>>, DomError> {
    let top = sheet.rules.len();
    let Some((list, place, ns)) = list_mut(sheet, path) else {
        return Err(DomError::new("NotFoundError", "The rule list is gone."));
    };
    let seen = visible(list, place, &ns);
    // A namespace rule stays while rules other than imports and
    // namespace rules follow it (section 6.1.2.2).
    if let Some((_, Kind::Namespace)) = seen.get(index)
        && seen
            .iter()
            .any(|(_, k)| !matches!(k, Kind::Import | Kind::Namespace))
    {
        return Err(DomError::new(
            "InvalidStateError",
            "Failed to delete the rule: other rules use the namespaces.",
        ));
    }
    let Some(&(at, _)) = seen.get(index) else {
        return Err(DomError::new(
            "IndexSizeError",
            format!(
                "The index provided ({index}) is larger than the maximum index ({}).",
                seen.len().saturating_sub(1)
            ),
        ));
    };
    list.remove(at);
    Ok(if path.is_empty() {
        (0..top).filter(|&i| i != at).map(Some).collect()
    } else {
        (0..top).map(Some).collect()
    })
}

/// The rule `path` names: its list's path and its index in it; with its
/// kind, where its list stands, and the sheet's namespaces.
fn rule_mut<'a>(
    sheet: &'a mut Stylesheet,
    path: &[usize],
) -> Option<(&'a mut Rule, Kind, Place, Namespaces)> {
    let (&last, outer) = path.split_last()?;
    let (list, place, ns) = list_mut(sheet, outer)?;
    let (at, kind) = *visible(list, place, &ns).get(last)?;
    Some((&mut list[at], kind, place, ns))
}

/// A rule's `style` set to the declarations `text` holds; or its
/// selector (or a keyframe's keys) set to `text`, which is ignored
/// unless it parses, when `selector`.
pub(crate) fn set(sheet: &mut Stylesheet, path: &[usize], text: &str, selector: bool) -> bool {
    let Some((rule, kind, place, ns)) = rule_mut(sheet, path) else {
        return false;
    };
    if selector {
        if kind == Kind::Page
            && let Rule::At(at) = rule
        {
            let Some(text) = page_selector(text) else {
                return false;
            };
            at.prelude = css::tokenize(&text);
            return true;
        }
        if kind == Kind::FontFeatureValues
            && let Rule::At(at) = rule
        {
            let tokens = css::tokenize(text);
            if tokens.iter().all(|t| matches!(t, Token::Whitespace)) {
                return false;
            }
            at.prelude = tokens;
            return true;
        }
        let Rule::Qualified(rule) = rule else {
            return false;
        };
        let tokens = css::tokenize(text);
        if kind == Kind::Style && !valid_selector(&tokens, &ns, place.relative) {
            return false;
        }
        rule.prelude = tokens;
        return true;
    }
    // A feature values rule's entries, one a line (see
    // [`feature_values`]).
    if kind == Kind::FontFeatureValues
        && let Rule::At(at) = rule
    {
        let body = feature_values_text(text);
        at.block = Some(Block::Rules(
            css::parse_stylesheet(&body).unwrap_or_default().rules,
        ));
        return true;
    }
    let parsed = css::parse_style_attribute(text).unwrap_or_default();
    match rule {
        Rule::Qualified(rule) => rule.declarations = parsed,
        Rule::Declarations(list) => *list = parsed,
        Rule::At(at) => match &mut at.block {
            Some(Block::Declarations(list)) => *list = parsed,
            _ => return false,
        },
    }
    true
}

/// An `@page` rule's selector list (CSS Paged Media 3 section 4.2) as
/// the reference browser writes it: each a page name and pseudo-classes
/// with nothing between them, their names lowercased; `None` for text
/// that is not one.
pub(crate) fn page_selector(text: &str) -> Option<String> {
    let mut out = Vec::new();
    for part in text.split(',') {
        let part = part.trim_matches(|c: char| c.is_ascii_whitespace());
        let (name, pseudo) = part.split_at(part.find(':').unwrap_or(part.len()));
        let ident =
            name.is_empty()
                || (name.chars().next().is_some_and(|c| {
                    c.is_ascii_alphabetic() || c == '_' || c == '-' || !c.is_ascii()
                }) && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || !c.is_ascii()));
        if !ident {
            return None;
        }
        let mut written = name.to_owned();
        for class in pseudo.split(':').skip(1) {
            let class = class.to_ascii_lowercase();
            if !matches!(class.as_str(), "first" | "left" | "right" | "blank") {
                return None;
            }
            written.push(':');
            written.push_str(&class);
        }
        if written.is_empty() && text.contains(',') {
            return None;
        }
        out.push(written);
    }
    Some(out.join(", "))
}

/// A `@counter-style` rule's name: one identifier, not `none` nor
/// `decimal` and the like a counter style may not take.
fn counter_style_name(prelude: &[Token]) -> Option<String> {
    let words: Vec<&Token> = prelude
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .collect();
    match words[..] {
        [Token::Ident(name)]
            if !matches!(
                name.to_ascii_lowercase().as_str(),
                "none"
                    | "decimal"
                    | "disc"
                    | "square"
                    | "circle"
                    | "disclosure-open"
                    | "disclosure-closed"
                    | "inherit"
                    | "initial"
                    | "unset"
                    | "default"
            ) =>
        {
            Some(name.clone())
        }
        _ => None,
    }
}

/// The feature types of `@font-feature-values` (Fonts 4 section
/// 6.10.1), in the order the reference browser writes them.
const FEATURE_TYPES: [&str; 7] = [
    "stylistic",
    "historical-forms",
    "styleset",
    "character-variant",
    "swash",
    "ornaments",
    "annotation",
];

/// A feature values rule's entries, one a line: its feature type, a
/// tab, the name, a tab and its numbers parted by spaces; those of a
/// type it declares again after the first, the later's value kept.
fn feature_values(block: &Option<Block>) -> String {
    let Some(Block::Rules(inner)) = block else {
        return String::new();
    };
    let mut entries: Vec<(usize, String, String)> = Vec::new();
    for rule in inner {
        let Rule::At(at) = rule else {
            continue;
        };
        let feature = at.name.to_ascii_lowercase();
        let Some(order) = FEATURE_TYPES.iter().position(|f| *f == feature) else {
            continue;
        };
        let Some(Block::Declarations(list)) = &at.block else {
            continue;
        };
        for declaration in list {
            let numbers: Option<Vec<String>> = declaration
                .value
                .iter()
                .filter(|t| !matches!(t, Token::Whitespace))
                .map(|t| match t {
                    Token::Number {
                        value,
                        is_integer: true,
                    } if *value >= 0.0 => Some(format!("{value}")),
                    _ => None,
                })
                .collect();
            // As many numbers as the type takes (section 6.10.1): any of
            // `styleset`, one or two of `character-variant`, one of the
            // rest.
            let most = match FEATURE_TYPES[order] {
                "styleset" => usize::MAX,
                "character-variant" => 2,
                _ => 1,
            };
            let Some(numbers) = numbers.filter(|n| !n.is_empty() && n.len() <= most) else {
                continue;
            };
            entries.retain(|(o, name, _)| !(*o == order && *name == declaration.name));
            entries.push((order, declaration.name.clone(), numbers.join(" ")));
        }
    }
    entries.sort_by_key(|(order, _, _)| *order);
    entries
        .iter()
        .map(|(order, name, numbers)| format!("{}\t{name}\t{numbers}\n", FEATURE_TYPES[*order]))
        .collect()
}

/// The entries [`feature_values`] writes as the blocks of a feature
/// values rule, each type's in its order.
fn feature_values_text(entries: &str) -> String {
    let mut out = String::new();
    let mut open: Option<&str> = None;
    for line in entries.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(feature), Some(name), Some(numbers)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if !FEATURE_TYPES.contains(&feature) || numbers.trim().is_empty() {
            continue;
        }
        if open != Some(feature) {
            if open.is_some() {
                out.push_str("} ");
            }
            out.push_str(&format!("@{feature} {{ "));
            open = Some(feature);
        }
        out.push_str(&format!("{}: {}; ", ident(name), numbers.trim()));
    }
    if open.is_some() {
        out.push_str("} ");
    }
    out
}

/// A media or import rule's media set to the list `text` holds (its
/// `MediaList`'s setters); an import's URL, layer and `supports()` are
/// kept.
pub(crate) fn set_media(sheet: &mut Stylesheet, path: &[usize], text: &str) -> bool {
    let Some((Rule::At(at), kind, ..)) = rule_mut(sheet, path) else {
        return false;
    };
    match kind {
        Kind::Media => at.prelude = css::tokenize(text),
        Kind::Import => {
            let (_, rest) = import_url(&at.prelude);
            let rest = trim_space(rest);
            // The layer and the condition before the media.
            let mut kept = 0;
            for _ in 0..2 {
                match rest.get(kept) {
                    Some(Token::Ident(word)) if word.eq_ignore_ascii_case("layer") => kept += 1,
                    Some(Token::Function(f))
                        if f.eq_ignore_ascii_case("layer")
                            || f.eq_ignore_ascii_case("supports") =>
                    {
                        kept += closing(&rest[kept..]) + 1;
                    }
                    _ => break,
                }
                while matches!(rest.get(kept), Some(Token::Whitespace)) {
                    kept += 1;
                }
            }
            let (url, _) = import_url(&at.prelude);
            let url = url.replace('\\', "\\\\").replace('"', "\\\"");
            let conditions = selector(rest.get(..kept).unwrap_or_default());
            at.prelude = css::tokenize(&format!(" url(\"{url}\") {conditions} {text}"));
        }
        _ => return false,
    }
    true
}

/// A sheet's rules a script sees at the top, each rule's kind name
/// alone: for the tests.
#[cfg(test)]
fn kinds(sheet: &Stylesheet) -> Vec<&'static str> {
    visible(&sheet.rules, TOP, &style::sheet_namespaces(&sheet.rules))
        .into_iter()
        .map(|(_, kind)| kind.name())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(text: &str) -> Stylesheet {
        css::parse_stylesheet(text).unwrap()
    }

    #[test]
    fn rules_serialize_as_the_reference_browser_writes_them() {
        let s = sheet(
            "@import 'a.css' screen; p , b { color : red ; margin: 0 !important } \
             ::bogus( { } @media (min-width: 1px) { i { top: 0 } } \
             @keyframes k { from { opacity: 0 } to { opacity: 1 } } @font-face { font-family: f }",
        );
        assert_eq!(
            kinds(&s),
            ["import", "style", "media", "keyframes", "font-face"]
        );
        let flat = rules(&s, &[]);
        let texts: Vec<&str> = flat.chunks(4).map(|r| r[1].as_str()).collect();
        assert_eq!(
            texts,
            [
                "@import url(\"a.css\") screen;",
                "p, b { color: red; margin: 0 !important; }",
                "@media (min-width: 1px) {\n  i { top: 0; }\n}",
                "@keyframes k {\n  0% { opacity: 0; }\n  100% { opacity: 1; }\n}",
                "@font-face { font-family: f; }",
            ]
        );
        assert_eq!(rules(&s, &[3])[2], "0%");
    }

    #[test]
    fn font_palette_values_rules_keep_their_descriptors_as_the_reference_does() {
        // Chrome 156, probed: the last valid of each descriptor, in their
        // order; a generic family, a negative or random() base palette
        // and `currentcolor` refused; a name that is no `<dashed-ident>`
        // no rule at all.
        let s = sheet(
            "@font-palette-values --A { override-colors: 0 red, 1 #00ff0080; \
             base-palette: 3; font-family: foo, \"bar\"; color: red } \
             @font-palette-values --B { base-palette: light; override-colors: 1 currentcolor; \
             font-family: serif } \
             @font-palette-values --C { base-palette: random(1, 2); base-palette: -1; \
             base-palette: calc(2); override-colors: 2 hsl(120 100% 50%) } \
             @font-palette-values E { } @font-palette-values --F --G { } \
             @font-palette-values --H { }",
        );
        assert_eq!(
            kinds(&s),
            [
                "font-palette-values",
                "font-palette-values",
                "font-palette-values",
                "font-palette-values"
            ]
        );
        let flat = rules(&s, &[]);
        let rule = |n: usize| &flat[n * 4..n * 4 + 4];
        assert_eq!(
            rule(0),
            [
                "font-palette-values",
                "@font-palette-values --A { font-family: foo, bar; base-palette: 3; \
                 override-colors: 0 red, 1 rgba(0, 255, 0, 0.5); }",
                "--A",
                "1foo, bar\u{0}13\u{0}10 red, 1 rgba(0, 255, 0, 0.5)",
            ]
        );
        assert_eq!(
            rule(1)[1],
            "@font-palette-values --B { base-palette: light; }"
        );
        assert_eq!(rule(1)[3], "\u{0}1light\u{0}");
        assert_eq!(
            rule(2)[1],
            "@font-palette-values --C { base-palette: calc(2); \
             override-colors: 2 rgb(0, 255, 0); }"
        );
        assert_eq!(rule(3)[1], "@font-palette-values --H { }");
    }

    #[test]
    fn font_feature_values_rules_keep_the_values_their_types_take() {
        // WPT's test_font_feature_values_parsing.html, as the reference
        // browser passes it: one number for most types, two at most for
        // `character-variant`, any for `styleset`; random() none.
        let s = sheet(
            "@font-feature-values bongo { @swash { a: 1 2; b: 3; c: random(1, 3) } \
             @character-variant { d: 1 2; e: 1 2 3 } @styleset { f: 1 2 3 4 } @blah { g: 1 } }",
        );
        assert_eq!(kinds(&s), ["font-feature-values"]);
        assert_eq!(
            rules(&s, &[])[1],
            "@font-feature-values bongo { @styleset { f: 1 2 3 4; } \
             @character-variant { d: 1 2; } @swash { b: 3; } }"
        );
    }

    #[test]
    fn selectors_serialize_in_their_canonical_forms() {
        let text = |css: &str| {
            let s = sheet(&format!("{css} {{ top: 0 }}"));
            let flat = rules(&s, &[]);
            flat[flat.len() - 4 + 2].clone()
        };
        assert_eq!(text(" *  >  div,p:before"), "* > div, p::before");
        assert_eq!(text("*.a *#b *:hover"), ".a #b :hover");
        assert_eq!(
            text(":nth-child( -1n + 5 ) :nth-of-type(odd) :nth-child(+10)"),
            ":nth-child(-n+5) :nth-of-type(2n+1) :nth-child(10)"
        );
        assert_eq!(text(":not(  *.x )"), ":not(.x)");
        assert_eq!(text("[\\30zonk][a = b i][|c]"), "[\\30 zonk][a=\"b\" i][c]");
        assert_eq!(text(":lang( j\\ a )"), ":lang(j\\ a)");
        let ns = "@namespace url(d); @namespace x url(d); @namespace y url(e);";
        assert_eq!(text(&format!("{ns} *|a")), "*|a");
        assert_eq!(text(&format!("{ns} x|b")), "b");
        assert_eq!(text(&format!("{ns} |d")), "|d");
        assert_eq!(text(&format!("{ns} *|a x|b y|c *|*")), "*|a b y|c *|*");
        assert_eq!(text("@namespace y url(e); *|a y|*.c"), "a y|*.c");
        // The namespace rules are the CSSOM's too.
        let s = sheet("@namespace y url(e); @layer a,b; y|p { top: 0 }");
        assert_eq!(kinds(&s), ["namespace", "layer-statement", "style"]);
        let flat = rules(&s, &[]);
        assert_eq!(flat[1], "@namespace y url(\"e\");");
        assert_eq!((flat[2].as_str(), flat[3].as_str()), ("y", "e"));
        assert_eq!(flat[5], "@layer a, b;");
    }

    #[test]
    fn namespace_rules_keep_their_place() {
        let mut s = sheet("@namespace y url(e); p { top: 0 }");
        assert_eq!(
            insert(&mut s, &[], "@namespace z url(f);", 0)
                .unwrap_err()
                .name,
            "InvalidStateError"
        );
        assert_eq!(
            delete(&mut s, &[], 0).unwrap_err().name,
            "InvalidStateError"
        );
        assert_eq!(
            insert(&mut s, &[], "i { top: 0 }", 0).unwrap_err().name,
            "HierarchyRequestError"
        );
        let mut s = sheet("@namespace y url(e);");
        assert!(insert(&mut s, &[], "@namespace z url(f);", 1).is_ok());
        assert!(delete(&mut s, &[], 0).is_ok());
    }

    #[test]
    fn a_style_rule_is_a_grouping_rule_of_its_nested_rules() {
        // CSS Nesting 1 section 3.3: its nested rules and runs of
        // declarations, the implicit `&` written, the lines as the
        // reference browser writes them.
        let mut s =
            sheet(".a { color: red; .b { top: 0 } left: 1px; @media screen { z-index: 1 } }");
        let flat = rules(&s, &[0]);
        let kinds: Vec<&str> = flat.chunks(4).map(|r| r[0].as_str()).collect();
        assert_eq!(kinds, ["style", "nested-declarations", "media"]);
        assert_eq!(flat[2], "& .b");
        assert_eq!(flat[5], "left: 1px;");
        assert_eq!(
            rules(&s, &[])[1],
            ".a {\n  color: red;\n  & .b { top: 0; }\n  left: 1px;\n  @media screen {\n  z-index: 1;\n}\n}"
        );
        // Inserted, a rule or declarations; an at-rule nesting does not
        // allow refused.
        assert_eq!(insert(&mut s, &[0], "> i { top: 0 }", 0).unwrap().0, 0);
        assert_eq!(rules(&s, &[0])[2], "& > i");
        assert_eq!(insert(&mut s, &[0], "width: 2px", 4).unwrap().0, 4);
        assert_eq!(
            insert(&mut s, &[0], "@font-face {}", 0).unwrap_err().name,
            "HierarchyRequestError"
        );
        assert_eq!(
            insert(&mut s, &[0], "nope: 1;", 0).unwrap_err().name,
            "SyntaxError"
        );
        assert!(delete(&mut s, &[0], 0).is_ok());
        assert!(set(&mut s, &[0, 0], "& .c", true));
        assert_eq!(rules(&s, &[0])[2], "& .c");
    }

    #[test]
    fn scope_and_property_rules_show_their_parts() {
        let s = sheet(
            "@scope (.a)to (> .b) { color: red; .c { top: 0 } } \
             @property --x { inherits: true; syntax: '<length>'; initial-value: 1px } \
             @property --bad { syntax: '<length>' }",
        );
        assert_eq!(kinds(&s), ["scope", "property"]);
        let flat = rules(&s, &[]);
        assert_eq!(
            flat[1],
            "@scope (.a) to (> .b) {\n  color: red;\n  .c { top: 0; }\n}"
        );
        assert_eq!((flat[2].as_str(), flat[3].as_str()), ("1.a", "1> .b"));
        assert_eq!(
            flat[5],
            "@property --x { syntax: \"<length>\"; inherits: true; initial-value: 1px; }"
        );
        assert_eq!(flat[7], "<length>\u{0}1\u{0}11px");
        let inside = rules(&s, &[0]);
        let kinds: Vec<&str> = inside.chunks(4).map(|r| r[0].as_str()).collect();
        assert_eq!(kinds, ["nested-declarations", "style"]);
    }

    #[test]
    fn insertions_and_deletions_keep_the_sheets_order() {
        let mut s = sheet("@import 'a.css'; p { color: red }");
        let (index, kept) = insert(&mut s, &[], "b { top: 0 }", 2).unwrap();
        assert_eq!(index, 2);
        assert_eq!(kept, [Some(0), Some(1), None]);
        assert_eq!(
            insert(&mut s, &[], "@import 'b.css';", 2).unwrap_err().name,
            "HierarchyRequestError"
        );
        assert_eq!(
            insert(&mut s, &[], "b {", 9).unwrap_err().name,
            "IndexSizeError"
        );
        assert_eq!(
            insert(&mut s, &[], "nonsense", 0).unwrap_err().name,
            "SyntaxError"
        );
        assert_eq!(
            insert(&mut s, &[], "p {} i {}", 0).unwrap_err().name,
            "SyntaxError"
        );
        let kept = delete(&mut s, &[], 0).unwrap();
        assert_eq!(kept, [Some(1), Some(2)]);
        assert_eq!(kinds(&s), ["style", "style"]);
        assert!(set(&mut s, &[0], "color: blue", false));
        assert!(!set(&mut s, &[0], "::bogus(", true));
        assert_eq!(rules(&s, &[])[1], "p { color: blue; }");
    }
}
