//! The CSSOM's rules (D66): which of a sheet's parsed rules a script
//! sees, as which kind of `CSSRule`, their `cssText` as CSSOM section
//! 6.7 serializes it, and the changes `insertRule`, `deleteRule` and the
//! rules' setters make to the parsed sheet the cascade reads.

use css::{Block, Declaration, Rule, Stylesheet};
use js::DomError;

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
        }
    }
}

/// What `rule` is to a script, in a keyframes rule's list or not; `None`
/// for a rule the CSSOM leaves out: a style rule whose selector does not
/// parse, an at-rule it has no interface for here.
fn kind(rule: &Rule, keyframes: bool) -> Option<Kind> {
    match rule {
        Rule::Qualified(_) if keyframes => Some(Kind::Keyframe),
        Rule::Qualified(rule) => style::parse_selector_list(&rule.prelude)
            .is_ok()
            .then_some(Kind::Style),
        Rule::At(at) => {
            let rules = matches!(at.block, Some(Block::Rules(_)));
            let declarations = matches!(at.block, Some(Block::Declarations(_)));
            match at.name.to_ascii_lowercase().as_str() {
                "import" if at.block.is_none() => Some(Kind::Import),
                "media" if rules => Some(Kind::Media),
                "supports" if rules => Some(Kind::Supports),
                "layer" if rules => Some(Kind::Layer),
                "keyframes" | "-webkit-keyframes" if rules => Some(Kind::Keyframes),
                "font-face" if declarations => Some(Kind::FontFace),
                "page" if declarations => Some(Kind::Page),
                _ => None,
            }
        }
    }
}

/// The rules of `list` a script sees: each one's index in the list and
/// its kind.
fn visible(list: &[Rule], keyframes: bool) -> Vec<(usize, Kind)> {
    list.iter()
        .enumerate()
        .filter_map(|(at, rule)| kind(rule, keyframes).map(|kind| (at, kind)))
        .collect()
}

/// The rule list `path` names in `sheet`: the sheet's, or a grouping
/// rule's by each index a script sees, outermost first; and whether it
/// is a keyframes rule's.
fn list<'a>(sheet: &'a Stylesheet, path: &[usize]) -> Option<(&'a Vec<Rule>, bool)> {
    let mut rules = &sheet.rules;
    let mut keyframes = false;
    for &index in path {
        let (at, kind) = *visible(rules, keyframes).get(index)?;
        let Rule::At(rule) = &rules[at] else {
            return None;
        };
        let Some(Block::Rules(inner)) = &rule.block else {
            return None;
        };
        keyframes = kind == Kind::Keyframes;
        rules = inner;
    }
    Some((rules, keyframes))
}

fn list_mut<'a>(sheet: &'a mut Stylesheet, path: &[usize]) -> Option<(&'a mut Vec<Rule>, bool)> {
    let mut rules = &mut sheet.rules;
    let mut keyframes = false;
    for &index in path {
        let (at, kind) = *visible(rules, keyframes).get(index)?;
        let Rule::At(rule) = &mut rules[at] else {
            return None;
        };
        let Some(Block::Rules(inner)) = &mut rule.block else {
            return None;
        };
        keyframes = kind == Kind::Keyframes;
        rules = inner;
    }
    Some((rules, keyframes))
}

/// The rules of the list `path` names, four strings each: the kind's
/// name, its `cssText`, and two by kind (a style or page rule's
/// selector and declarations, an import's URL and media, a media or
/// supports rule's condition, a keyframes or layer rule's name, a
/// keyframe's key and declarations, a font-face rule's declarations).
pub(crate) fn rules(sheet: &Stylesheet, path: &[usize]) -> Vec<String> {
    let Some((list, keyframes)) = list(sheet, path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (at, kind) in visible(list, keyframes) {
        let rule = &list[at];
        let (a, b) = parts(rule, kind);
        out.extend([kind.name().to_owned(), css_text(rule, kind, ""), a, b]);
    }
    out
}

/// The two strings by kind [`rules`] gives.
fn parts(rule: &Rule, kind: Kind) -> (String, String) {
    match (rule, kind) {
        (Rule::Qualified(rule), Kind::Keyframe) => {
            (key_text(&rule.prelude), declarations(&rule.declarations))
        }
        (Rule::Qualified(rule), _) => (
            selector_text(&rule.prelude),
            declarations(&rule.declarations),
        ),
        (Rule::At(at), Kind::Import) => {
            let (url, rest) = import_parts(&at.prelude);
            (url, rest)
        }
        (Rule::At(at), Kind::FontFace | Kind::Page) => {
            let block = match &at.block {
                Some(Block::Declarations(list)) => declarations(list),
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

/// A style rule's selector as CSSOM serializes it: its selectors
/// parted by `, ` and combinators by spaces, as written otherwise.
fn selector_text(tokens: &[css::Token]) -> String {
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
    let mut tokens = prelude
        .iter()
        .skip_while(|t| matches!(t, css::Token::Whitespace));
    let url = match tokens.next() {
        Some(css::Token::Url { value, .. } | css::Token::String { value, .. }) => value.clone(),
        _ => String::new(),
    };
    let rest: Vec<css::Token> = tokens.cloned().collect();
    (url, selector(&rest))
}

/// A declaration block's text: `name: value;` each, `!important` kept.
pub(crate) fn declarations(list: &[Declaration]) -> String {
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

/// A rule's `cssText`, each nested line after the first indented by
/// `indent` more.
fn css_text(rule: &Rule, kind: Kind, indent: &str) -> String {
    let block = |head: String, list: &[Declaration]| {
        let body = declarations(list);
        if body.is_empty() {
            format!("{head} {{ }}")
        } else {
            format!("{head} {{ {body} }}")
        }
    };
    match (rule, kind) {
        (Rule::Qualified(rule), Kind::Keyframe) => {
            block(key_text(&rule.prelude), &rule.declarations)
        }
        (Rule::Qualified(rule), _) => block(selector_text(&rule.prelude), &rule.declarations),
        (Rule::At(at), Kind::Import) => {
            let (url, rest) = import_parts(&at.prelude);
            let url = url.replace('\\', "\\\\").replace('"', "\\\"");
            if rest.is_empty() {
                format!("@import url(\"{url}\");")
            } else {
                format!("@import url(\"{url}\") {rest};")
            }
        }
        (Rule::At(at), Kind::FontFace | Kind::Page) => {
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
            let prelude = selector(&at.prelude);
            let mut out = if prelude.is_empty() {
                format!("@{} {{\n", at.name)
            } else {
                format!("@{} {prelude} {{\n", at.name)
            };
            if let Some(Block::Rules(inner)) = &at.block {
                let deeper = format!("{indent}  ");
                for (at, kind) in visible(inner, kind == Kind::Keyframes) {
                    out.push_str(&deeper);
                    out.push_str(&css_text(&inner[at], kind, &deeper));
                    out.push('\n');
                }
            }
            out.push_str(indent);
            out.push('}');
            out
        }
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
    let parsed = css::parse_stylesheet(text).map_err(|_| syntax())?;
    let [rule] = <[Rule; 1]>::try_from(parsed.rules).map_err(|_| syntax())?;
    let top = sheet.rules.len();
    let (list, keyframes) = list_mut(sheet, path).ok_or_else(syntax)?;
    let new_kind = kind(&rule, keyframes).ok_or_else(syntax)?;
    let seen = visible(list, keyframes);
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
    // An import only before every rule but other imports (section
    // 6.1.2.1's constraints); none inside a grouping rule.
    if new_kind == Kind::Import
        && (!path.is_empty() || seen[..index].iter().any(|(_, k)| *k != Kind::Import))
    {
        return Err(DomError::new(
            "HierarchyRequestError",
            "Failed to insert the rule.",
        ));
    }
    if new_kind != Kind::Import && seen[index..].iter().any(|(_, k)| *k == Kind::Import) {
        return Err(DomError::new(
            "HierarchyRequestError",
            "Failed to insert the rule.",
        ));
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
    let Some((list, keyframes)) = list_mut(sheet, path) else {
        return Err(DomError::new("NotFoundError", "The rule list is gone."));
    };
    let seen = visible(list, keyframes);
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

/// The rule `path` names: its list's path and its index in it.
fn rule_mut<'a>(sheet: &'a mut Stylesheet, path: &[usize]) -> Option<(&'a mut Rule, Kind)> {
    let (&last, outer) = path.split_last()?;
    let (list, keyframes) = list_mut(sheet, outer)?;
    let (at, kind) = *visible(list, keyframes).get(last)?;
    Some((&mut list[at], kind))
}

/// A rule's `style` set to the declarations `text` holds; or its
/// selector (or a keyframe's keys) set to `text`, which is ignored
/// unless it parses, when `selector`.
pub(crate) fn set(sheet: &mut Stylesheet, path: &[usize], text: &str, selector: bool) -> bool {
    let Some((rule, kind)) = rule_mut(sheet, path) else {
        return false;
    };
    if selector {
        let Rule::Qualified(rule) = rule else {
            return false;
        };
        let tokens = css::tokenize(text);
        if kind == Kind::Style && style::parse_selector_list(&tokens).is_err() {
            return false;
        }
        rule.prelude = tokens;
        return true;
    }
    let parsed = css::parse_style_attribute(text).unwrap_or_default();
    match rule {
        Rule::Qualified(rule) => rule.declarations = parsed,
        Rule::At(at) => match &mut at.block {
            Some(Block::Declarations(list)) => *list = parsed,
            _ => return false,
        },
    }
    true
}

/// A sheet's rules a script sees at the top, each rule's kind name
/// alone: for the tests.
#[cfg(test)]
fn kinds(sheet: &Stylesheet) -> Vec<&'static str> {
    visible(&sheet.rules, false)
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
