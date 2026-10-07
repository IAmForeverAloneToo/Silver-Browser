//! The grid properties' values (css-grid-2 §7, §8, M18): track lists
//! with their line names and repetitions, `grid-template-areas`, the
//! placement properties, the `grid-template` and `grid` shorthands, and
//! `justify-items` and `justify-self` (css-align-3 §6.1, §6.2). Each
//! parser takes a declaration's value as the cascade has it, trimmed and
//! without `!important`, and says `None` when the value is not one the
//! property takes, so that the declaration is dropped.

use std::collections::HashMap;
use std::sync::Arc;

use css::Token;

use super::{
    LengthBases, Nesting, baseline_position, function_arguments, math,
    parse_non_negative_length_percentage, positional, single_ident, single_token,
};
use crate::calc::{Computed, Want};
use crate::computed::{
    AlignPosition, GridAutoFlow, GridLine, JustifyItems, LineNames, NamedArea, OverflowAlign,
    RepeatCount, SelfAlignment, TemplateAreas, TrackBreadth, TrackList, TrackRepeat, TrackSection,
    TrackSize,
};

/// A cursor over a value's tokens that steps over white space.
struct Cursor<'t> {
    tokens: &'t [Token],
    pos: usize,
}

impl<'t> Cursor<'t> {
    fn new(tokens: &'t [Token]) -> Self {
        Cursor { tokens, pos: 0 }
    }

    /// The next token that is not white space, left where it is.
    fn peek(&mut self) -> Option<&'t Token> {
        while matches!(self.tokens.get(self.pos), Some(Token::Whitespace)) {
            self.pos += 1;
        }
        self.tokens.get(self.pos)
    }

    /// The next token that is not white space, taken.
    fn next(&mut self) -> Option<&'t Token> {
        let token = self.peek();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    /// Takes the next token if it is `token`.
    fn eat(&mut self, token: &Token) -> bool {
        let found = self.peek() == Some(token);
        if found {
            self.pos += 1;
        }
        found
    }

    fn at_end(&mut self) -> bool {
        self.peek().is_none()
    }

    /// The next component value, taken: a token, or a function or a
    /// block with everything up to its closer (M21: a math function is
    /// one value).
    fn component(&mut self) -> Option<&'t [Token]> {
        self.peek()?;
        let start = self.pos;
        let mut nesting = Nesting::default();
        loop {
            nesting.step(&self.tokens[self.pos]);
            self.pos += 1;
            if nesting.is_top() || self.pos == self.tokens.len() {
                break;
            }
        }
        Some(&self.tokens[start..self.pos])
    }
}

/// A `<custom-ident>` (css-values-4 §4.2) as a grid line's name: any
/// identifier but the CSS-wide keywords, `default`, and the `span` and
/// `auto` that the grid excludes (css-grid-2 §7.2.2, §8.3), kept as
/// written.
fn custom_ident(name: &str) -> Option<Arc<str>> {
    let excluded = [
        "span",
        "auto",
        "inherit",
        "initial",
        "unset",
        "revert",
        "revert-layer",
        "default",
    ];
    (!excluded.iter().any(|x| name.eq_ignore_ascii_case(x))).then(|| Arc::from(name))
}

/// The words of a value that is nothing but identifiers, lowercased.
fn words(tokens: &[Token]) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut cursor = Cursor::new(tokens);
    while let Some(token) = cursor.next() {
        match token {
            Token::Ident(word) => words.push(word.to_ascii_lowercase()),
            _ => return None,
        }
    }
    Some(words)
}

/// The value's parts between `/`s, a `/` inside a function, a math
/// function's division, not dividing them. An empty part is one no
/// parser takes.
fn slash_parts(tokens: &[Token]) -> Vec<&[Token]> {
    let mut parts = Vec::new();
    let mut nesting = Nesting::default();
    let mut start = 0;
    for (i, token) in tokens.iter().enumerate() {
        if *token == Token::Delim('/') && nesting.is_top() {
            parts.push(&tokens[start..i]);
            start = i + 1;
        } else {
            nesting.step(token);
        }
    }
    parts.push(&tokens[start..]);
    parts
}

/// `[ <custom-ident>* ]`, the cursor at its `[`.
fn line_names(c: &mut Cursor<'_>) -> Option<LineNames> {
    if !c.eat(&Token::OpenSquare) {
        return None;
    }
    let mut names = Vec::new();
    loop {
        match c.next()? {
            Token::CloseSquare => return Some(names),
            Token::Ident(name) => names.push(custom_ident(name)?),
            _ => return None,
        }
    }
}

/// A `<track-breadth>`: a non-negative length or percentage, a
/// non-negative `<flex>`, `min-content`, `max-content` or `auto`.
fn breadth(component: &[Token], bases: &LengthBases<'_>) -> Option<TrackBreadth> {
    match single_token(component) {
        Some(Token::Dimension { value, unit }) if unit.eq_ignore_ascii_case("fr") => {
            let fr = *value as f32;
            (fr.is_finite() && fr >= 0.0).then_some(TrackBreadth::Flex(fr))
        }
        Some(Token::Ident(word)) => match word.to_ascii_lowercase().as_str() {
            "min-content" => Some(TrackBreadth::MinContent),
            "max-content" => Some(TrackBreadth::MaxContent),
            "auto" => Some(TrackBreadth::Auto),
            _ => None,
        },
        _ => parse_non_negative_length_percentage(component, bases).map(TrackBreadth::Fixed),
    }
}

/// A `<track-size>`: a breadth, `minmax(<inflexible-breadth>,
/// <track-breadth>)` or `fit-content(<length-percentage [0,∞]>)`.
fn track_size(c: &mut Cursor<'_>, bases: &LengthBases<'_>) -> Option<TrackSize> {
    let component = c.component()?;
    let arguments = |name: &str| match function_arguments(component) {
        Some((function, arguments)) if function.eq_ignore_ascii_case(name) => Some(arguments),
        _ => None,
    };
    match component.first()? {
        Token::Function(name) if name.eq_ignore_ascii_case("minmax") => {
            let [min, max] = arguments("minmax")?[..] else {
                return None;
            };
            let min = breadth(min, bases)?;
            if matches!(min, TrackBreadth::Flex(_)) {
                return None;
            }
            Some(TrackSize::MinMax(min, breadth(max, bases)?))
        }
        Token::Function(name) if name.eq_ignore_ascii_case("fit-content") => {
            let [limit] = arguments("fit-content")?[..] else {
                return None;
            };
            Some(TrackSize::FitContent(parse_non_negative_length_percentage(
                limit, bases,
            )?))
        }
        _ => breadth(component, bases).map(TrackSize::Breadth),
    }
}

/// Whether a breadth is fixed: a length or a percentage.
fn is_fixed_breadth(breadth: &TrackBreadth) -> bool {
    matches!(breadth, TrackBreadth::Fixed(_))
}

/// Whether a size is a `<fixed-size>` (css-grid-2 §7.2): a fixed
/// breadth, or a `minmax()` with a fixed minimum or a fixed maximum,
/// which is what an automatic repetition, and the list around one, may
/// hold.
fn is_fixed_size(size: &TrackSize) -> bool {
    match size {
        TrackSize::Breadth(breadth) => is_fixed_breadth(breadth),
        TrackSize::MinMax(min, max) => is_fixed_breadth(min) || is_fixed_breadth(max),
        TrackSize::FitContent(_) => false,
    }
}

/// The line names and track sizes of a list, or of a `repeat()`'s list
/// when `in_repeat` (which then ends at its `)`, taken, and may not hold
/// another `repeat()`): each size after the names of the line before it,
/// two name lists never next to each other. The names have one more
/// entry than the sections, at least one of which there must be.
fn sections(
    c: &mut Cursor<'_>,
    bases: &LengthBases<'_>,
    in_repeat: bool,
) -> Option<(Vec<LineNames>, Vec<TrackSection>)> {
    let mut names = Vec::new();
    let mut sections = Vec::new();
    let mut pending: Option<LineNames> = None;
    loop {
        match c.peek() {
            None if in_repeat => return None,
            None => break,
            Some(Token::CloseParen) if in_repeat => {
                c.next();
                break;
            }
            Some(Token::OpenSquare) => {
                if pending.is_some() {
                    return None;
                }
                pending = Some(line_names(c)?);
                continue;
            }
            Some(Token::Function(name)) if name.eq_ignore_ascii_case("repeat") => {
                if in_repeat {
                    return None;
                }
                c.next();
                names.push(pending.take().unwrap_or_default());
                sections.push(TrackSection::Repeat(repeat(c, bases)?));
            }
            Some(_) => {
                names.push(pending.take().unwrap_or_default());
                sections.push(TrackSection::Track(track_size(c, bases)?));
            }
        }
    }
    if sections.is_empty() {
        return None;
    }
    names.push(pending.unwrap_or_default());
    Some((names, sections))
}

/// A `repeat()`, the cursor after its `repeat(`: a count of at least
/// one, `auto-fill` or `auto-fit`, then its list. A count beyond what a
/// `u32` holds is kept as the largest one; layout limits it anyway.
fn repeat(c: &mut Cursor<'_>, bases: &LengthBases<'_>) -> Option<TrackRepeat> {
    let component = c.component()?;
    let count = match single_token(component) {
        Some(Token::Number {
            value,
            is_integer: true,
        }) if *value >= 1.0 => RepeatCount::Count(value.min(f64::from(u32::MAX)) as u32),
        Some(Token::Ident(word)) if word.eq_ignore_ascii_case("auto-fill") => RepeatCount::AutoFill,
        Some(Token::Ident(word)) if word.eq_ignore_ascii_case("auto-fit") => RepeatCount::AutoFit,
        // A math function's number, rounded, at least one (M21.2).
        _ => match math(component, Want::Number, bases, None)? {
            Computed::Value(v) => {
                RepeatCount::Count((v + 0.5).floor().clamp(1.0, f64::from(u32::MAX)) as u32)
            }
            _ => return None,
        },
    };
    if !c.eat(&Token::Comma) {
        return None;
    }
    let (names, sections) = sections(c, bases, true)?;
    let tracks = sections
        .into_iter()
        .map(|section| match section {
            TrackSection::Track(size) => size,
            TrackSection::Repeat(_) => unreachable!("a repeat() holds none"),
        })
        .collect();
    Some(TrackRepeat {
        count,
        names,
        tracks,
    })
}

/// A `<track-list>` or an `<auto-track-list>` (css-grid-2 §7.2): with an
/// automatic repetition, there is only one, it and every other size in
/// the list are fixed, and every other repetition has a count.
fn track_list(tokens: &[Token], bases: &LengthBases<'_>) -> Option<TrackList> {
    let mut c = Cursor::new(tokens);
    let (names, sections) = sections(&mut c, bases, false)?;
    let automatic = sections
        .iter()
        .filter(|s| matches!(s, TrackSection::Repeat(r) if r.count.is_automatic()))
        .count();
    if automatic > 1 {
        return None;
    }
    if automatic == 1 {
        let all_fixed = sections.iter().all(|section| match section {
            TrackSection::Track(size) => is_fixed_size(size),
            TrackSection::Repeat(repeat) => repeat.tracks.iter().all(is_fixed_size),
        });
        if !all_fixed {
            return None;
        }
    }
    Some(TrackList { names, sections })
}

/// `grid-template-columns` and `grid-template-rows`: `none` or a track
/// list. The outer `Option` is "parsed at all"; the inner is `None` for
/// `none`. `subgrid` is not taken (docs/design/grid.md, section 2).
pub(super) fn parse_template(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Option<TrackList>> {
    if single_ident(tokens).is_some_and(|w| w.eq_ignore_ascii_case("none")) {
        return Some(None);
    }
    track_list(tokens, bases).map(Some)
}

/// An `<explicit-track-list>`: names and sizes, no `repeat()`.
fn explicit_track_list(tokens: &[Token], bases: &LengthBases<'_>) -> Option<TrackList> {
    track_list(tokens, bases).filter(|list| {
        list.sections
            .iter()
            .all(|s| matches!(s, TrackSection::Track(_)))
    })
}

/// One cell of a template string: a named cell, or a null one.
type Cell = Option<String>;

/// A `grid-template-areas` string's cells (css-grid-2 §7.3): runs of
/// ident code points are named cells, runs of `.` null cells, white
/// space separates them, and anything else is trash, which makes the
/// declaration invalid.
fn cells(row: &str) -> Option<Vec<Cell>> {
    let is_name = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii();
    let is_space = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}');
    let mut cells = Vec::new();
    let mut chars = row.chars().peekable();
    while let Some(&c) = chars.peek() {
        if is_space(c) {
            chars.next();
        } else if c == '.' {
            while chars.next_if_eq(&'.').is_some() {}
            cells.push(None);
        } else if is_name(c) {
            let mut name = String::new();
            while let Some(c) = chars.next_if(|&c| is_name(c)) {
                name.push(c);
            }
            cells.push(Some(name));
        } else {
            return None;
        }
    }
    Some(cells)
}

/// The areas the template strings `rows` make (css-grid-2 §7.3): every
/// string has as many cells as the first, at least one, and each name's
/// cells fill a rectangle.
fn areas(rows: &[&str]) -> Option<TemplateAreas> {
    struct Found {
        rows: (u32, u32),
        columns: (u32, u32),
        cells: u64,
    }
    let mut columns = None;
    let mut order: Vec<String> = Vec::new();
    let mut found: HashMap<String, Found> = HashMap::new();
    for (r, row) in rows.iter().enumerate() {
        let cells = cells(row)?;
        if cells.is_empty() || columns.is_some_and(|n| n != cells.len()) {
            return None;
        }
        columns = Some(cells.len());
        let r = u32::try_from(r).ok()?;
        for (c, cell) in cells.into_iter().enumerate() {
            let Some(name) = cell else { continue };
            let c = u32::try_from(c).ok()?;
            match found.get_mut(&name) {
                Some(area) => {
                    area.rows = (area.rows.0.min(r), area.rows.1.max(r + 1));
                    area.columns = (area.columns.0.min(c), area.columns.1.max(c + 1));
                    area.cells += 1;
                }
                None => {
                    order.push(name.clone());
                    found.insert(
                        name,
                        Found {
                            rows: (r, r + 1),
                            columns: (c, c + 1),
                            cells: 1,
                        },
                    );
                }
            }
        }
    }
    let mut areas = Vec::with_capacity(order.len());
    for name in order {
        let area = &found[&name];
        let height = u64::from(area.rows.1 - area.rows.0);
        let width = u64::from(area.columns.1 - area.columns.0);
        if height * width != area.cells {
            return None;
        }
        areas.push(NamedArea {
            name: Arc::from(name),
            rows: area.rows,
            columns: area.columns,
        });
    }
    Some(TemplateAreas {
        rows: u32::try_from(rows.len()).ok()?,
        columns: u32::try_from(columns?).ok()?,
        areas,
    })
}

/// `grid-template-areas`: `none` or one or more strings. The outer
/// `Option` is "parsed at all"; the inner is `None` for `none`.
pub(super) fn parse_template_areas(tokens: &[Token]) -> Option<Option<TemplateAreas>> {
    if single_ident(tokens).is_some_and(|w| w.eq_ignore_ascii_case("none")) {
        return Some(None);
    }
    let mut rows = Vec::new();
    let mut c = Cursor::new(tokens);
    while let Some(token) = c.next() {
        match token {
            Token::String { value, bad: false } => rows.push(value.as_str()),
            _ => return None,
        }
    }
    areas(&rows).map(Some)
}

/// What `grid-template` sets (css-grid-2 §7.4): the rows, the columns
/// and the areas, `None` each for `none`.
pub(super) type Template = (Option<TrackList>, Option<TrackList>, Option<TemplateAreas>);

/// The `grid-template` shorthand: `none`; `<'grid-template-rows'> /
/// <'grid-template-columns'>`; or the strings, each with its row's size
/// and the names of the lines around it, then `/` and an explicit track
/// list for the columns.
pub(super) fn parse_grid_template(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Template> {
    if single_ident(tokens).is_some_and(|w| w.eq_ignore_ascii_case("none")) {
        return Some((None, None, None));
    }
    let parts = slash_parts(tokens);
    let has_strings = parts[0].iter().any(|t| matches!(t, Token::String { .. }));
    if !has_strings {
        let [rows, columns] = parts.as_slice() else {
            return None;
        };
        return Some((
            parse_template(rows, bases)?,
            parse_template(columns, bases)?,
            None,
        ));
    }
    let columns = match parts.as_slice() {
        [_] => None,
        [_, columns] => Some(explicit_track_list(columns, bases)?),
        _ => return None,
    };
    // The rows: `[ <line-names>? <string> <track-size>? <line-names>? ]+`.
    // The names after one row's string and those before the next's are
    // the same line's, merged.
    let mut c = Cursor::new(parts[0]);
    let mut names: Vec<LineNames> = vec![Vec::new()];
    let mut sizes = Vec::new();
    let mut strings = Vec::new();
    while !c.at_end() {
        if c.peek() == Some(&Token::OpenSquare) {
            let before = line_names(&mut c)?;
            names.last_mut().expect("never empty").extend(before);
        }
        match c.next()? {
            Token::String { value, bad: false } => strings.push(value.as_str()),
            _ => return None,
        }
        let size = match c.peek() {
            None | Some(Token::String { .. }) | Some(Token::OpenSquare) => TrackSize::AUTO,
            Some(_) => track_size(&mut c, bases)?,
        };
        sizes.push(TrackSection::Track(size));
        let after = if c.peek() == Some(&Token::OpenSquare) {
            line_names(&mut c)?
        } else {
            Vec::new()
        };
        names.push(after);
    }
    let areas = areas(&strings)?;
    let rows = TrackList {
        names,
        sections: sizes,
    };
    Some((Some(rows), columns, Some(areas)))
}

/// `grid-auto-columns` and `grid-auto-rows`: one or more track sizes.
pub(super) fn parse_auto_tracks(
    tokens: &[Token],
    bases: &LengthBases<'_>,
) -> Option<Vec<TrackSize>> {
    let mut c = Cursor::new(tokens);
    let mut sizes = Vec::new();
    while !c.at_end() {
        if matches!(c.peek(), Some(Token::Function(name)) if name.eq_ignore_ascii_case("repeat")) {
            return None;
        }
        sizes.push(track_size(&mut c, bases)?);
    }
    (!sizes.is_empty()).then_some(sizes)
}

/// `grid-auto-flow: [ row | column ] || dense`.
pub(super) fn parse_auto_flow(tokens: &[Token]) -> Option<GridAutoFlow> {
    let words = words(tokens)?;
    let mut flow = GridAutoFlow::default();
    let (mut direction, mut dense) = (false, false);
    for word in &words {
        match word.as_str() {
            "row" | "column" if !direction => {
                direction = true;
                flow.column = word == "column";
            }
            "dense" if !dense => {
                dense = true;
                flow.dense = true;
            }
            _ => return None,
        }
    }
    (!words.is_empty()).then_some(flow)
}

/// What the `grid` shorthand sets (css-grid-2 §7.8): the template, the
/// flow, and the implicit rows' and columns' sizes.
pub(super) struct Grid {
    pub(super) template: Template,
    pub(super) auto_flow: GridAutoFlow,
    pub(super) auto_rows: Vec<TrackSize>,
    pub(super) auto_columns: Vec<TrackSize>,
}

/// `[ auto-flow && dense? ] <track-size>*`: whether it is `dense`, and
/// the sizes, `auto` when none are given; `None` without `auto-flow`.
fn auto_flow_part(tokens: &[Token], bases: &LengthBases<'_>) -> Option<(bool, Vec<TrackSize>)> {
    let mut c = Cursor::new(tokens);
    let (mut auto_flow, mut dense) = (false, false);
    while let Some(Token::Ident(word)) = c.peek() {
        if word.eq_ignore_ascii_case("auto-flow") && !auto_flow {
            auto_flow = true;
        } else if word.eq_ignore_ascii_case("dense") && !dense {
            dense = true;
        } else {
            break;
        }
        c.next();
    }
    if !auto_flow {
        return None;
    }
    let rest = &c.tokens[c.pos..];
    let sizes = if rest.iter().all(|t| *t == Token::Whitespace) {
        vec![TrackSize::AUTO]
    } else {
        parse_auto_tracks(rest, bases)?
    };
    Some((dense, sizes))
}

/// The `grid` shorthand: a `grid-template`, every implicit property
/// initial; or `<'grid-template-rows'> / [ auto-flow && dense? ]
/// <'grid-auto-columns'>?`; or `[ auto-flow && dense? ]
/// <'grid-auto-rows'>? / <'grid-template-columns'>`.
pub(super) fn parse_grid(tokens: &[Token], bases: &LengthBases<'_>) -> Option<Grid> {
    if let Some(template) = parse_grid_template(tokens, bases) {
        return Some(Grid {
            template,
            auto_flow: GridAutoFlow::default(),
            auto_rows: vec![TrackSize::AUTO],
            auto_columns: vec![TrackSize::AUTO],
        });
    }
    let parts = slash_parts(tokens);
    let [rows, columns] = parts.as_slice() else {
        return None;
    };
    if let Some((dense, auto_columns)) = auto_flow_part(columns, bases) {
        return Some(Grid {
            template: (parse_template(rows, bases)?, None, None),
            auto_flow: GridAutoFlow {
                column: true,
                dense,
            },
            auto_rows: vec![TrackSize::AUTO],
            auto_columns,
        });
    }
    let (dense, auto_rows) = auto_flow_part(rows, bases)?;
    Some(Grid {
        template: (None, parse_template(columns, bases)?, None),
        auto_flow: GridAutoFlow {
            column: false,
            dense,
        },
        auto_rows,
        auto_columns: vec![TrackSize::AUTO],
    })
}

/// A `<grid-line>` (css-grid-2 §8.3): `auto`; a `<custom-ident>`; an
/// integer other than zero with a name or not, in either order; or
/// `span` first or last with a positive integer, a name or both. An
/// integer beyond what an `i32` holds is kept as the largest one; layout
/// limits it to the grid's lines anyway.
pub(super) fn parse_grid_line(tokens: &[Token]) -> Option<GridLine> {
    let mut c = Cursor::new(tokens);
    let mut parts = Vec::new();
    while let Some(token) = c.next() {
        parts.push(token);
    }
    if let [Token::Ident(word)] = parts.as_slice()
        && word.eq_ignore_ascii_case("auto")
    {
        return Some(GridLine::Auto);
    }
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let is_span = |t: &Token| matches!(t, Token::Ident(w) if w.eq_ignore_ascii_case("span"));
    let span_at = parts.iter().position(|t| is_span(t));
    if let Some(at) = span_at
        && at != 0
        && at != parts.len() - 1
    {
        return None;
    }
    let (mut integer, mut name) = (None, None);
    for (i, token) in parts.iter().enumerate() {
        match token {
            _ if Some(i) == span_at => {}
            Token::Number {
                value,
                is_integer: true,
            } if integer.is_none() => {
                integer = Some(value.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32);
            }
            Token::Ident(word) if name.is_none() => name = Some(custom_ident(word)?),
            _ => return None,
        }
    }
    match (span_at.is_some(), integer, name) {
        (true, Some(n), _) if n < 1 => None,
        (true, None, None) => None,
        (true, n, name) => Some(GridLine::Span(n.map_or(1, |n| n as u32), name)),
        (false, Some(0), _) => None,
        (false, Some(n), name) => Some(GridLine::Line(n, name)),
        (false, None, Some(name)) => Some(GridLine::Ident(name)),
        (false, None, None) => None,
    }
}

/// What an omitted end of `grid-row`, `grid-column` or `grid-area`
/// takes from the start it goes with (css-grid-2 §8.4): the same name
/// when that is a bare `<custom-ident>`, else `auto`.
fn end_from(start: &GridLine) -> GridLine {
    match start {
        GridLine::Ident(name) => GridLine::Ident(name.clone()),
        _ => GridLine::Auto,
    }
}

/// `grid-row` and `grid-column`: a start, then `/` and an end, which
/// when omitted is [`end_from`] the start.
pub(super) fn parse_grid_lines(tokens: &[Token]) -> Option<(GridLine, GridLine)> {
    let parts = slash_parts(tokens);
    let start = parse_grid_line(parts[0])?;
    let end = match parts.as_slice() {
        [_] => end_from(&start),
        [_, end] => parse_grid_line(end)?,
        _ => return None,
    };
    Some((start, end))
}

/// `grid-area`: up to four lines, row start, column start, row end and
/// column end, the missing ones from those before them (§8.4). Returned
/// in that order.
pub(super) fn parse_grid_area(tokens: &[Token]) -> Option<[GridLine; 4]> {
    let parts = slash_parts(tokens);
    if parts.len() > 4 {
        return None;
    }
    let lines: Vec<GridLine> = parts
        .iter()
        .map(|part| parse_grid_line(part))
        .collect::<Option<_>>()?;
    let row_start = lines[0].clone();
    let column_start = lines
        .get(1)
        .cloned()
        .unwrap_or_else(|| end_from(&row_start));
    let row_end = lines
        .get(2)
        .cloned()
        .unwrap_or_else(|| end_from(&row_start));
    let column_end = lines
        .get(3)
        .cloned()
        .unwrap_or_else(|| end_from(&column_start));
    Some([row_start, column_start, row_end, column_end])
}

/// `justify-self` (css-align-3 §6.1), or with `allow_auto` false the
/// value `justify-items` shares with it: `auto`, `normal`, `stretch`, a
/// baseline, or `[ safe | unsafe ]?` and a self-position, `left` or
/// `right`.
pub(super) fn justify_self(words: &[String], allow_auto: bool) -> Option<SelfAlignment> {
    use AlignPosition::*;
    if let [w] = words {
        match w.as_str() {
            "auto" if allow_auto => return Some(SelfAlignment::Auto),
            "normal" => return Some(SelfAlignment::Normal),
            "stretch" => return Some(SelfAlignment::Stretch),
            _ => {}
        }
    }
    if let Some(b) = baseline_position(words) {
        return Some(SelfAlignment::Baseline(b));
    }
    positional(
        words,
        &[
            Center, Start, End, SelfStart, SelfEnd, FlexStart, FlexEnd, Left, Right,
        ],
    )
    .map(|(p, o)| SelfAlignment::Position(p, o))
}

/// `justify-items` (css-align-3 §6.1): a `justify-self` value but
/// `auto`, `legacy` alone, which computes to the value of `parent`, the
/// parent's, when that carries `legacy` and to `normal` otherwise, or
/// `legacy` with `left`, `right` or `center` in either order.
pub(super) fn justify_items(words: &[String], parent: JustifyItems) -> Option<JustifyItems> {
    let side = |w: &str| match w {
        "left" => Some(AlignPosition::Left),
        "right" => Some(AlignPosition::Right),
        "center" => Some(AlignPosition::Center),
        _ => None,
    };
    let legacy_side = match words {
        [w] if w == "legacy" => return Some(JustifyItems::legacy_under(parent)),
        [a, b] if a == "legacy" => side(b),
        [a, b] if b == "legacy" => side(a),
        _ => None,
    };
    if let Some(position) = legacy_side {
        return Some(JustifyItems {
            value: SelfAlignment::Position(position, OverflowAlign::Default),
            legacy: true,
        });
    }
    if words.iter().any(|w| w == "legacy") {
        return None;
    }
    justify_self(words, false).map(|value| JustifyItems {
        value,
        legacy: false,
    })
}
