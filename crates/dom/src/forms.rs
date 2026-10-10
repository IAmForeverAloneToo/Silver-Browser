//! What HTML says of forms and their controls that more than one stage
//! reads (M26): whether a control is disabled, a `select`'s options, an
//! element's form owner, and the controls' state (M26.7): values,
//! checkedness, selectedness and selections, which the document keeps
//! in a [`ControlState`] once they change. Style's matcher reads them
//! for `:disabled`, `:checked` and the radio groups; layout for what a
//! control shows; the engine and its scripts for the rest.

use crate::{Document, MutationError, NodeId};

/// The elements `:enabled` and `:disabled` apply to (HTML section
/// 4.16.3).
pub fn can_be_disabled(document: &Document, element: NodeId) -> bool {
    matches!(
        document.tag(element),
        Some("button" | "input" | "select" | "textarea" | "optgroup" | "option" | "fieldset")
    )
}

/// Whether the element is actually disabled (HTML section 4.16.3): a
/// `button`, `input`, `select`, `textarea` or `fieldset` with a
/// `disabled` attribute or in a `fieldset` that disables it (sections
/// 4.10.19.5 and 4.10.15); an `optgroup` with one; an option that is
/// disabled.
pub fn actually_disabled(document: &Document, element: NodeId) -> bool {
    match document.tag(element) {
        Some("button" | "input" | "select" | "textarea" | "fieldset") => {
            document.attr(element, "disabled").is_some() || disabled_by_fieldset(document, element)
        }
        Some("optgroup") => document.attr(element, "disabled").is_some(),
        Some("option") => option_disabled(document, element),
        _ => false,
    }
}

/// Whether a `fieldset` with a `disabled` attribute around the element
/// disables it: one it is not inside the first `legend` child of.
fn disabled_by_fieldset(document: &Document, element: NodeId) -> bool {
    let mut child = element;
    for ancestor in document.ancestors(element) {
        if document.tag(ancestor) == Some("fieldset")
            && document.attr(ancestor, "disabled").is_some()
            && !(document.tag(child) == Some("legend")
                && document
                    .element_children(ancestor)
                    .find(|&c| document.tag(c) == Some("legend"))
                    == Some(child))
        {
            return true;
        }
        child = ancestor;
    }
    false
}

/// Whether an option is disabled (HTML section 4.10.10): it has a
/// `disabled` attribute, or the nearest of its ancestors that is an
/// `optgroup`, `select`, `hr`, `datalist` or `option` is an `optgroup`
/// with one.
pub fn option_disabled(document: &Document, option: NodeId) -> bool {
    if document.attr(option, "disabled").is_some() {
        return true;
    }
    for ancestor in document.ancestors(option) {
        match document.tag(ancestor) {
            Some("select" | "hr" | "datalist" | "option") => return false,
            Some("optgroup") => return document.attr(ancestor, "disabled").is_some(),
            _ => {}
        }
    }
    false
}

/// The `select` whose list of options holds `option` (HTML section
/// 4.10.7): its nearest `select` ancestor, unless an `hr`, a `datalist`
/// or an `option` comes before it, or two `optgroup`s do.
pub fn owner_select(document: &Document, option: NodeId) -> Option<NodeId> {
    let mut optgroups = 0;
    for ancestor in document.ancestors(option) {
        match document.tag(ancestor) {
            Some("select") => return Some(ancestor),
            Some("hr" | "datalist" | "option") => return None,
            Some("optgroup") => {
                optgroups += 1;
                if optgroups == 2 {
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

/// A `select`'s list of options (HTML section 4.10.7), in tree order:
/// its `option` descendants, not looking inside a `select`, `hr`,
/// `option` or `datalist`, nor an `optgroup` inside another.
pub fn list_of_options(document: &Document, select: NodeId) -> Vec<NodeId> {
    let mut options = Vec::new();
    let mut node = document.first_child(select);
    while let Some(at) = node {
        let tag = document.tag(at);
        if tag == Some("option") {
            options.push(at);
        }
        let closed = matches!(tag, Some("select" | "hr" | "option" | "datalist"))
            || (tag == Some("optgroup")
                && document
                    .ancestors(at)
                    .take_while(|&ancestor| ancestor != select)
                    .any(|ancestor| document.tag(ancestor) == Some("optgroup")));
        node = match document.first_child(at) {
            Some(child) if !closed => Some(child),
            _ => next_within(document, at, select),
        };
    }
    options
}

/// The node after `node` and its subtree in tree order, within `root`.
fn next_within(document: &Document, mut node: NodeId, root: NodeId) -> Option<NodeId> {
    while node != root {
        if let Some(next) = document.next_sibling(node) {
            return Some(next);
        }
        node = document.parent(node)?;
    }
    None
}

/// The option a `select` without `multiple` leaves selected, by the
/// selectedness setting algorithm (HTML section 4.10.7) over its
/// options' selectedness: the last selected, since one selected
/// unselects the rest and a later one inserted selected does too; with
/// none, and a display size of 1, the first that is not disabled, unless
/// a script or the user unselected them since, which leaves none until
/// the algorithm runs again.
pub fn selected_option(document: &Document, select: NodeId) -> Option<NodeId> {
    let options = list_of_options(document, select);
    if let Some(&last) = options
        .iter()
        .rev()
        .find(|&&option| own_selectedness(document, option))
    {
        return Some(last);
    }
    let unselected = options
        .iter()
        .any(|&option| document.control(option).is_some_and(|c| c.dirty_selected));
    if display_size(document, select) != 1 || unselected {
        return None;
    }
    options
        .into_iter()
        .find(|&option| !option_disabled(document, option))
}

/// Whether the element is a text control (M26): a `textarea`, or an
/// `input` that is none of the types with no text to edit, the types
/// that need a picker being text fields until they have one (G61).
pub fn is_text_control(document: &Document, element: NodeId) -> bool {
    match document.tag(element) {
        Some("textarea") => true,
        Some("input") => {
            let kind = document.attr(element, "type").unwrap_or("text");
            ![
                "checkbox", "radio", "submit", "reset", "button", "image", "hidden",
            ]
            .iter()
            .any(|other| kind.eq_ignore_ascii_case(other))
        }
        _ => false,
    }
}

// ---- Control state (M26.7) --------------------------------------------------

/// A form control's state that HTML keeps apart from its attributes
/// (M26.7, sections 4.10.5, 4.10.10 and 4.10.11): made at its first
/// change, kept by the document and journaled as
/// [`crate::Change::ControlChanged`]. What is `None` follows the
/// element's attributes or children.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ControlState {
    /// The value once the dirty value flag is set: an `input`'s, or a
    /// `textarea`'s raw value. Before, the default value stands.
    pub value: Option<String>,
    /// A checkbox's or radio button's checkedness once something set
    /// it; before, its `checked` attribute's.
    pub checked: Option<bool>,
    /// The dirty checkedness flag: the user or a script set it.
    pub dirty_checked: bool,
    /// An option's selectedness once something set it; before, its
    /// `selected` attribute's.
    pub selected: Option<bool>,
    /// The dirty selectedness flag.
    pub dirty_selected: bool,
    /// A checkbox's indeterminate flag.
    pub indeterminate: bool,
    /// A text control's selection, in bytes of its value.
    pub selection: Selection,
}

impl ControlState {
    /// The bytes of data it holds, for the document's limit.
    pub(crate) fn bytes(&self) -> usize {
        self.value.as_ref().map_or(0, String::len)
    }
}

/// A text control's selection (HTML section 4.10.20): `start` to `end`,
/// in bytes of its value, the caret at `end` unless it is backward.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Selection {
    pub start: usize,
    pub end: usize,
    pub direction: Direction,
}

/// A selection's direction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Direction {
    #[default]
    None,
    Forward,
    Backward,
}

/// The types an `input` can be (HTML section 4.10.5), in the table's
/// order.
pub const INPUT_TYPES: [&str; 22] = [
    "hidden",
    "text",
    "search",
    "tel",
    "url",
    "email",
    "password",
    "date",
    "month",
    "week",
    "time",
    "datetime-local",
    "number",
    "range",
    "color",
    "checkbox",
    "radio",
    "file",
    "submit",
    "image",
    "reset",
    "button",
];

/// An `input`'s type: its `type` attribute's keyword, whatever its case,
/// or `text` for none or one HTML does not know.
pub fn input_type(document: &Document, input: NodeId) -> &'static str {
    document
        .attr(input, "type")
        .and_then(|value| {
            INPUT_TYPES
                .into_iter()
                .find(|t| t.eq_ignore_ascii_case(value))
        })
        .unwrap_or("text")
}

/// How an `input`'s `value` IDL attribute behaves (HTML section
/// 4.10.5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueMode {
    /// The value is the element's own: the text fields and the rest.
    Value,
    /// The `value` attribute: hidden inputs and the buttons.
    Default,
    /// The `value` attribute, or `on`: checkboxes and radio buttons.
    DefaultOn,
    /// The files chosen: a file input.
    Filename,
}

/// An `input`'s value mode.
pub fn value_mode(document: &Document, input: NodeId) -> ValueMode {
    match input_type(document, input) {
        "hidden" | "submit" | "image" | "reset" | "button" => ValueMode::Default,
        "checkbox" | "radio" => ValueMode::DefaultOn,
        "file" => ValueMode::Filename,
        _ => ValueMode::Value,
    }
}

/// The control's state, or the default one.
fn state(document: &Document, element: NodeId) -> ControlState {
    document.control(element).cloned().unwrap_or_default()
}

/// What the control's `value` IDL attribute returns (HTML sections
/// 4.10.5.4, 4.10.11, 4.10.7, 4.10.10 and 4.10.6): an `input`'s by its
/// value mode, a `textarea`'s API value, a `select`'s first selected
/// option's value, an option's, a button's attribute.
pub fn value(document: &Document, element: NodeId) -> String {
    match document.tag(element) {
        Some("input") => match value_mode(document, element) {
            ValueMode::Value => match document.control(element).and_then(|c| c.value.clone()) {
                Some(value) => value,
                None => sanitize(
                    document,
                    element,
                    document.attr(element, "value").unwrap_or(""),
                ),
            },
            ValueMode::Default => document.attr(element, "value").unwrap_or("").to_owned(),
            ValueMode::DefaultOn => document.attr(element, "value").unwrap_or("on").to_owned(),
            ValueMode::Filename => String::new(),
        },
        Some("textarea") => normalize_newlines(&raw_value(document, element)),
        Some("select") => list_of_options(document, element)
            .into_iter()
            .find(|&option| selectedness(document, option))
            .map(|option| option_value(document, option))
            .unwrap_or_default(),
        Some("option") => option_value(document, element),
        Some("button") => document.attr(element, "value").unwrap_or("").to_owned(),
        _ => String::new(),
    }
}

/// A `textarea`'s default value: its child text content.
pub fn default_value(document: &Document, textarea: NodeId) -> String {
    document
        .children(textarea)
        .filter_map(|child| document.text(child))
        .collect()
}

/// A `textarea`'s raw value: what was typed or set, else its default
/// value.
pub fn raw_value(document: &Document, textarea: NodeId) -> String {
    document
        .control(textarea)
        .and_then(|c| c.value.clone())
        .unwrap_or_else(|| default_value(document, textarea))
}

/// `value` with each CRLF and lone CR made LF.
pub fn normalize_newlines(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

/// Sets a control's value as the `value` IDL attribute does in the
/// value mode (HTML section 4.10.5.4): an `input`'s, sanitized, or a
/// `textarea`'s raw value, its newlines normalized; the dirty value
/// flag set and, when the value changed, the selection collapsed at its
/// end. An element of another kind is refused.
pub fn set_value(
    document: &mut Document,
    element: NodeId,
    value: &str,
) -> Result<(), MutationError> {
    let value = match document.tag(element) {
        Some("input") => sanitize(document, element, value),
        Some("textarea") => normalize_newlines(value),
        _ => return Err(MutationError::WrongKind),
    };
    let before = self::value(document, element);
    let mut state = state(document, element);
    if before != value {
        state.selection = Selection {
            start: value.len(),
            end: value.len(),
            direction: Direction::None,
        };
    }
    state.value = Some(value);
    document.set_control(element, state)
}

/// The value sanitization algorithm of the `input`'s type (HTML section
/// 4.10.5.1): line breaks out of the text fields, the URL and email
/// ones trimmed too; a number, a date or a time that does not parse is
/// empty; a colour is a valid lowercase one or black; a range's number
/// is clamped to its range, or its default.
pub fn sanitize(document: &Document, input: NodeId, value: &str) -> String {
    let kind = input_type(document, input);
    match kind {
        "text" | "search" | "tel" | "password" => strip_newlines(value),
        "url" | "email" => strip_newlines(value)
            .trim_matches(|c: char| c.is_ascii_whitespace())
            .to_owned(),
        "number" => {
            if parse_float(value).is_some() {
                value.to_owned()
            } else {
                String::new()
            }
        }
        "range" => {
            let attr = |name, default: f64| {
                document
                    .attr(input, name)
                    .and_then(parse_float)
                    .unwrap_or(default)
            };
            let min = attr("min", 0.0);
            let max = attr("max", 100.0).max(min);
            let number = parse_float(value).unwrap_or(min + (max - min) / 2.0);
            format_float(number.clamp(min, max))
        }
        "color" => {
            let bytes = value.as_bytes();
            if bytes.len() == 7 && bytes[0] == b'#' && bytes[1..].iter().all(u8::is_ascii_hexdigit)
            {
                value.to_ascii_lowercase()
            } else {
                "#000000".to_owned()
            }
        }
        "date" | "month" | "week" | "time" | "datetime-local" => {
            if valid_date_or_time(kind, value) {
                value.to_owned()
            } else {
                String::new()
            }
        }
        _ => value.to_owned(),
    }
}

/// `value` with its line feeds and carriage returns taken out.
fn strip_newlines(value: &str) -> String {
    value.chars().filter(|&c| c != '\n' && c != '\r').collect()
}

/// A valid floating-point number's value (HTML section 2.3.4.3):
/// `-`, digits with an optional fraction, or a fraction alone, and an
/// optional exponent; nothing else, and finite.
pub fn parse_float(value: &str) -> Option<f64> {
    let bytes = value.as_bytes();
    let mut at = usize::from(bytes.first() == Some(&b'-'));
    let digits = |at: &mut usize| {
        let start = *at;
        while bytes.get(*at).is_some_and(u8::is_ascii_digit) {
            *at += 1;
        }
        *at - start
    };
    let whole = digits(&mut at);
    let mut fraction = 0;
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        fraction = digits(&mut at);
        if fraction == 0 {
            return None;
        }
    }
    if whole == 0 && fraction == 0 {
        return None;
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        if digits(&mut at) == 0 {
            return None;
        }
    }
    if at != bytes.len() {
        return None;
    }
    value.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// The best representation of `n` as a floating-point number, as a
/// script would print it, for the numbers a range takes.
fn format_float(n: f64) -> String {
    if n == n.trunc() && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// Whether `value` is a valid date, month, week, time or local date and
/// time string (HTML sections 2.3.5.2 to 2.3.5.8), as `kind` asks.
fn valid_date_or_time(kind: &str, value: &str) -> bool {
    match kind {
        "date" => date(value).is_some(),
        "month" => month(value).is_some(),
        "week" => week(value),
        "time" => time(value),
        _ => value
            .split_once(['T', ' '])
            .is_some_and(|(day, at)| date(day).is_some() && time(at)),
    }
}

/// `n` ASCII digits at the start of `s` as a number, and the rest.
fn digits(s: &str, n: usize) -> Option<(u32, &str)> {
    let head = s.get(..n)?;
    if !head.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((head.parse().ok()?, &s[n..]))
}

/// A year of four or more digits, above 0, and the rest.
fn year(s: &str) -> Option<(u32, &str)> {
    let n = s.bytes().take_while(u8::is_ascii_digit).count();
    if n < 4 {
        return None;
    }
    let (year, rest) = digits(s, n)?;
    (year > 0).then_some((year, rest))
}

fn month(s: &str) -> Option<(u32, u32)> {
    let (year, rest) = year(s)?;
    let (month, rest) = digits(rest.strip_prefix('-')?, 2)?;
    (rest.is_empty() && (1..=12).contains(&month)).then_some((year, month))
}

fn date(s: &str) -> Option<()> {
    let (y, rest) = year(s)?;
    let (m, rest) = digits(rest.strip_prefix('-')?, 2)?;
    let (d, rest) = digits(rest.strip_prefix('-')?, 2)?;
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    (rest.is_empty() && (1..=days).contains(&d)).then_some(())
}

fn week(s: &str) -> bool {
    let Some((y, rest)) = year(s) else {
        return false;
    };
    let Some((w, rest)) = rest.strip_prefix("-W").and_then(|rest| digits(rest, 2)) else {
        return false;
    };
    // A year has 53 weeks when it starts on a Thursday, or on a
    // Wednesday in a leap year (ISO 8601); 0 is a Sunday.
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let p = y - 1;
    let first = (1 + 5 * (p % 4) + 4 * (p % 100) + 6 * (p % 400)) % 7;
    let weeks = if first == 4 || (leap && first == 3) {
        53
    } else {
        52
    };
    rest.is_empty() && (1..=weeks).contains(&w)
}

fn time(s: &str) -> bool {
    let Some((h, rest)) = digits(s, 2) else {
        return false;
    };
    let Some((m, rest)) = rest.strip_prefix(':').and_then(|rest| digits(rest, 2)) else {
        return false;
    };
    if h > 23 || m > 59 {
        return false;
    }
    let Some(rest) = rest.strip_prefix(':') else {
        return rest.is_empty();
    };
    let Some((sec, rest)) = digits(rest, 2) else {
        return false;
    };
    if sec > 59 {
        return false;
    }
    match rest.strip_prefix('.') {
        None => rest.is_empty(),
        Some(fraction) => {
            (1..=3).contains(&fraction.len()) && fraction.bytes().all(|b| b.is_ascii_digit())
        }
    }
}

/// An option's text (HTML section 4.10.10): its descendant text, but a
/// script's, its ASCII whitespace stripped and collapsed.
pub fn option_text(document: &Document, option: NodeId) -> String {
    let text: String = document
        .subtree(option)
        .filter(|&node| {
            !document
                .ancestors(node)
                .take_while(|&ancestor| ancestor != option)
                .any(|ancestor| document.tag(ancestor) == Some("script"))
        })
        .filter_map(|node| document.text(node))
        .collect();
    text.split(|c: char| c.is_ascii_whitespace())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// An option's value: its `value` attribute, else its text.
pub fn option_value(document: &Document, option: NodeId) -> String {
    match document.attr(option, "value") {
        Some(value) => value.to_owned(),
        None => option_text(document, option),
    }
}

/// An option's label: its `label` attribute when not empty, else its
/// text.
pub fn option_label(document: &Document, option: NodeId) -> String {
    match document.attr(option, "label") {
        Some(label) if !label.is_empty() => label.to_owned(),
        _ => option_text(document, option),
    }
}

/// A checkbox's or radio button's own checkedness: what something set,
/// else its `checked` attribute's.
pub fn own_checkedness(document: &Document, input: NodeId) -> bool {
    document
        .control(input)
        .and_then(|c| c.checked)
        .unwrap_or_else(|| document.attr(input, "checked").is_some())
}

/// Whether an `input` is checked (HTML section 4.10.5): a checkbox by
/// its checkedness; a radio button when it is the last of its group,
/// in tree order, whose checkedness is set, since one set unchecks the
/// rest and a later one inserted checked does too. Any other is not.
pub fn checkedness(document: &Document, input: NodeId) -> bool {
    match input_type(document, input) {
        "checkbox" => own_checkedness(document, input),
        "radio" => {
            own_checkedness(document, input)
                && radio_group(document, input)
                    .into_iter()
                    .rev()
                    .find(|&radio| own_checkedness(document, radio))
                    == Some(input)
        }
        _ => false,
    }
}

/// A radio button's group (HTML section 4.10.5.1.16), in tree order: the
/// radio buttons in its tree with its form owner and its name; one with
/// no name is alone.
pub fn radio_group(document: &Document, radio: NodeId) -> Vec<NodeId> {
    let Some(name) = document.attr(radio, "name").filter(|name| !name.is_empty()) else {
        return vec![radio];
    };
    let root = document.ancestors(radio).last().unwrap_or(radio);
    let owner = form_owner(document, radio);
    document
        .subtree(root)
        .filter(|&node| {
            node == radio
                || (document.tag(node) == Some("input")
                    && input_type(document, node) == "radio"
                    && document.attr(node, "name") == Some(name)
                    && form_owner(document, node) == owner)
        })
        .collect()
}

/// Sets an `input`'s checkedness, the dirty flag too when `dirty`: a
/// radio button checked unchecks the rest of its group.
pub fn set_checkedness(
    document: &mut Document,
    input: NodeId,
    checked: bool,
    dirty: bool,
) -> Result<(), MutationError> {
    if document.tag(input) != Some("input") {
        return Err(MutationError::WrongKind);
    }
    let mut state = state(document, input);
    state.checked = Some(checked);
    state.dirty_checked |= dirty;
    document.set_control(input, state)?;
    if checked && input_type(document, input) == "radio" {
        for other in radio_group(document, input) {
            if other != input && own_checkedness(document, other) {
                let mut state = self::state(document, other);
                state.checked = Some(false);
                document.set_control(other, state)?;
            }
        }
    }
    Ok(())
}

/// A checkbox's indeterminate flag.
pub fn indeterminate(document: &Document, input: NodeId) -> bool {
    document.control(input).is_some_and(|c| c.indeterminate)
}

/// Sets an `input`'s indeterminate flag.
pub fn set_indeterminate(
    document: &mut Document,
    input: NodeId,
    indeterminate: bool,
) -> Result<(), MutationError> {
    if document.tag(input) != Some("input") {
        return Err(MutationError::WrongKind);
    }
    let mut state = state(document, input);
    state.indeterminate = indeterminate;
    document.set_control(input, state)
}

/// An option's own selectedness: what something set, else its
/// `selected` attribute's.
pub fn own_selectedness(document: &Document, option: NodeId) -> bool {
    document
        .control(option)
        .and_then(|c| c.selected)
        .unwrap_or_else(|| document.attr(option, "selected").is_some())
}

/// Whether an option is selected: in a `select` without `multiple`,
/// when it is the one its select leaves selected; otherwise by its own
/// selectedness.
pub fn selectedness(document: &Document, option: NodeId) -> bool {
    match owner_select(document, option) {
        Some(select) if document.attr(select, "multiple").is_none() => {
            selected_option(document, select) == Some(option)
        }
        _ => own_selectedness(document, option),
    }
}

/// Sets an option's selectedness, the dirty flag too when `dirty`: one
/// selected in a `select` without `multiple` leaves the rest
/// unselected.
pub fn set_selectedness(
    document: &mut Document,
    option: NodeId,
    selected: bool,
    dirty: bool,
) -> Result<(), MutationError> {
    if document.tag(option) != Some("option") {
        return Err(MutationError::WrongKind);
    }
    let mut state = state(document, option);
    state.selected = Some(selected);
    state.dirty_selected |= dirty;
    document.set_control(option, state)?;
    if let Some(select) = owner_select(document, option)
        && selected
        && document.attr(select, "multiple").is_none()
    {
        for other in list_of_options(document, select) {
            if other != option && own_selectedness(document, other) {
                let mut state = self::state(document, other);
                state.selected = Some(false);
                document.set_control(other, state)?;
            }
        }
    }
    Ok(())
}

/// A `select`'s display size (HTML section 4.10.7): its `size`, a
/// non-negative integer; else 4 with `multiple` and 1 without.
pub fn display_size(document: &Document, select: NodeId) -> u64 {
    document
        .attr(select, "size")
        .and_then(crate::parse_non_negative_integer)
        .unwrap_or(if document.attr(select, "multiple").is_some() {
            4
        } else {
            1
        })
}

/// Whether a `select` is shown as a list box rather than a drop-down
/// box: with `multiple`, or a display size above 1. A size of 0 is a
/// drop-down, as the reference browser draws it, though it leaves no
/// option selected as HTML has it.
pub fn is_list_box(document: &Document, select: NodeId) -> bool {
    document.attr(select, "multiple").is_some() || display_size(document, select) > 1
}

/// The index of a `select`'s first selected option in its list, if any.
pub fn selected_index(document: &Document, select: NodeId) -> Option<usize> {
    list_of_options(document, select)
        .into_iter()
        .position(|option| selectedness(document, option))
}

/// A text control's selection: what was set, else the caret at the
/// start.
pub fn selection(document: &Document, control: NodeId) -> Selection {
    document
        .control(control)
        .map(|c| c.selection)
        .unwrap_or_default()
}

/// Sets a text control's selection, clamped to its value and to the
/// characters' boundaries, the start no later than the end.
pub fn set_selection(
    document: &mut Document,
    control: NodeId,
    selection: Selection,
) -> Result<(), MutationError> {
    if !is_text_control(document, control) {
        return Err(MutationError::WrongKind);
    }
    let value = value(document, control);
    let clamp = |at: usize| {
        let mut at = at.min(value.len());
        while !value.is_char_boundary(at) {
            at -= 1;
        }
        at
    };
    let end = clamp(selection.end);
    let start = clamp(selection.start).min(end);
    let mut state = state(document, control);
    state.selection = Selection {
        start,
        end,
        direction: selection.direction,
    };
    document.set_control(control, state)
}

/// What a control shows inside its box (M26.7): a text field's value, as
/// bullets for a password, or its placeholder when that is empty; a
/// textarea's value or placeholder; a button input's value or its
/// default label; a drop-down select's chosen option's label. With
/// whether it is the placeholder. `None` for what shows no text of its
/// own: a checkbox, a radio button, a list box, a hidden or image input,
/// anything else.
pub fn shown_text(document: &Document, element: NodeId) -> Option<(String, bool)> {
    let placeholder = || {
        document
            .attr(element, "placeholder")
            .map(|text| (strip_newlines(text), true))
    };
    match document.tag(element) {
        Some("input") => match input_type(document, element) {
            "checkbox" | "radio" | "hidden" | "image" => None,
            "submit" | "reset" | "button" => {
                let label = match (
                    document.attr(element, "value"),
                    input_type(document, element),
                ) {
                    (Some(value), _) => value.to_owned(),
                    (None, "submit") => "Submit".to_owned(),
                    (None, "reset") => "Reset".to_owned(),
                    (None, _) => String::new(),
                };
                Some((label, false))
            }
            "password" => {
                let value = value(document, element);
                if value.is_empty() {
                    placeholder().or(Some((String::new(), false)))
                } else {
                    Some((value.chars().map(|_| '\u{2022}').collect(), false))
                }
            }
            _ => {
                let value = value(document, element);
                if value.is_empty() {
                    placeholder().or(Some((value, false)))
                } else {
                    Some((value, false))
                }
            }
        },
        Some("textarea") => {
            let value = value(document, element);
            if value.is_empty() {
                document
                    .attr(element, "placeholder")
                    .map(|text| (normalize_newlines(text), true))
                    .or(Some((value, false)))
            } else {
                Some((value, false))
            }
        }
        Some("select") if !is_list_box(document, element) => Some((
            selected_option(document, element)
                .map(|option| option_label(document, option))
                .unwrap_or_default(),
            false,
        )),
        _ => None,
    }
}

/// Whether a control shows its placeholder (`:placeholder-shown`, M26.7).
pub fn placeholder_shown(document: &Document, element: NodeId) -> bool {
    let applies = match document.tag(element) {
        Some("textarea") => true,
        Some("input") => matches!(
            input_type(document, element),
            "text" | "search" | "tel" | "url" | "email" | "password" | "number"
        ),
        _ => false,
    };
    applies
        && document.attr(element, "placeholder").is_some()
        && value(document, element).is_empty()
}

/// The steps HTML takes when a control's `checked` or `selected`
/// attribute is added or removed (sections 4.10.5 and 4.10.10): unless
/// its state is dirty, its checkedness or selectedness follows; added,
/// it unchecks the rest of a radio group or unselects the rest of a
/// select, and removed, the state follows the attribute again.
pub(crate) fn default_changed(document: &mut Document, element: NodeId, name: &str) {
    let present = document.attr(element, name).is_some();
    let Some(mut state) = document.control(element).cloned() else {
        if present {
            set_default(document, element, name);
        }
        return;
    };
    match (document.tag(element), name) {
        (Some("input"), "checked") if !state.dirty_checked => {
            if present {
                set_default(document, element, name);
            } else {
                state.checked = None;
                let _ = document.set_control(element, state);
            }
        }
        (Some("option"), "selected") if !state.dirty_selected => {
            if present {
                set_default(document, element, name);
            } else {
                state.selected = None;
                let _ = document.set_control(element, state);
            }
        }
        _ => {}
    }
}

/// An `input` checked or an option selected by its attribute's addition.
fn set_default(document: &mut Document, element: NodeId, name: &str) {
    match (document.tag(element), name) {
        (Some("input"), "checked") => {
            let _ = set_checkedness(document, element, true, false);
        }
        (Some("option"), "selected") => {
            let _ = set_selectedness(document, element, true, false);
        }
        _ => {}
    }
}

/// Whether the element is listed (HTML section 4.10.2): one of a form's
/// `elements`, an image input not.
pub fn is_listed(document: &Document, element: NodeId) -> bool {
    match document.tag(element) {
        Some("input") => input_type(document, element) != "image",
        Some("button" | "fieldset" | "object" | "output" | "select" | "textarea") => true,
        _ => false,
    }
}

/// Whether the element is labelable (HTML section 4.10.2).
pub fn is_labelable(document: &Document, element: NodeId) -> bool {
    match document.tag(element) {
        Some("input") => input_type(document, element) != "hidden",
        Some("button" | "meter" | "output" | "progress" | "select" | "textarea") => true,
        _ => false,
    }
}

/// The control a `label` labels (HTML section 4.10.4): with a `for`
/// attribute, the first element in its tree with that ID if it is
/// labelable; without, its first labelable descendant.
pub fn labeled_control(document: &Document, label: NodeId) -> Option<NodeId> {
    if document.tag(label) != Some("label") {
        return None;
    }
    match document.attr(label, "for") {
        Some(id) => {
            let root = document.ancestors(label).last().unwrap_or(label);
            let target = document
                .subtree(root)
                .find(|&node| !id.is_empty() && document.attr(node, "id") == Some(id))?;
            is_labelable(document, target).then_some(target)
        }
        // The label itself is not labelable.
        None => document
            .subtree(label)
            .find(|&node| is_labelable(document, node)),
    }
}

/// The labels of a labelable element, in tree order: the `label`s in its
/// tree that label it.
pub fn labels(document: &Document, element: NodeId) -> Vec<NodeId> {
    if !is_labelable(document, element) {
        return Vec::new();
    }
    let root = document.ancestors(element).last().unwrap_or(element);
    document
        .subtree(root)
        .filter(|&node| {
            document.tag(node) == Some("label") && labeled_control(document, node) == Some(element)
        })
        .collect()
}

/// Whether the element is form-associated (HTML section 4.10.2): it can
/// have a form owner.
pub fn form_associated(document: &Document, element: NodeId) -> bool {
    matches!(
        document.tag(element),
        Some("button" | "fieldset" | "input" | "object" | "output" | "select" | "textarea" | "img")
    )
}

/// The element's form owner (HTML section 4.10.18.3): for one with a
/// `form` attribute, in the document, the first element in tree order
/// with that ID if it is a `form`, else none; for any other, its nearest
/// `form` ancestor. Style's matcher works it out the same way with the
/// IDs it keeps.
pub fn form_owner(document: &Document, element: NodeId) -> Option<NodeId> {
    let root = document.root();
    let connected = element == root || document.ancestors(element).last() == Some(root);
    if let Some(id) = document.attr(element, "form")
        && connected
    {
        let owner = document
            .subtree(root)
            .find(|&node| !id.is_empty() && document.attr(node, "id") == Some(id))?;
        return (document.tag(owner) == Some("form")).then_some(owner);
    }
    document
        .ancestors(element)
        .find(|&ancestor| document.tag(ancestor) == Some("form"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Element, Node};

    fn e(tag: &str) -> Element {
        Element::new(tag)
    }

    /// The element with ID `id`.
    fn by_id(document: &Document, id: &str) -> NodeId {
        document
            .subtree(document.root())
            .find(|&node| document.attr(node, "id") == Some(id))
            .expect("the element")
    }

    #[test]
    fn a_controls_form_owner_is_its_form_attributes_form_or_its_ancestor() {
        let page: Node = e("body")
            .push(
                e("form")
                    .attr("id", "outer")
                    .push(e("input").attr("id", "inside"))
                    .push(e("input").attr("id", "elsewhere").attr("form", "other"))
                    .push(e("input").attr("id", "nowhere").attr("form", "missing"))
                    .push(e("input").attr("id", "not-a-form").attr("form", "p")),
            )
            .push(e("form").attr("id", "other"))
            .push(e("p").attr("id", "p"))
            .push(e("input").attr("id", "loose"))
            .into();
        let document = Document::from_node(&page);
        let owner = |id| form_owner(&document, by_id(&document, id));
        assert_eq!(owner("inside"), Some(by_id(&document, "outer")));
        assert_eq!(owner("elsewhere"), Some(by_id(&document, "other")));
        assert_eq!(owner("nowhere"), None);
        assert_eq!(owner("not-a-form"), None);
        assert_eq!(owner("loose"), None);
    }

    #[test]
    fn a_control_is_disabled_by_its_attribute_or_its_fieldset_but_the_first_legend() {
        let page: Node = e("fieldset")
            .attr("disabled", "")
            .push(e("legend").push(e("button").attr("id", "in-legend")))
            .push(e("legend").push(e("button").attr("id", "in-second")))
            .push(e("input").attr("id", "inside"))
            .push(e("div").push(e("a").attr("id", "link")))
            .into();
        let document = Document::from_node(&page);
        let disabled = |id| actually_disabled(&document, by_id(&document, id));
        assert!(!disabled("in-legend"));
        assert!(disabled("in-second"));
        assert!(disabled("inside"));
        assert!(!disabled("link"));
    }

    #[test]
    fn values_are_sanitized_by_type() {
        let page: Node = e("body")
            .push(e("input").attr("id", "i"))
            .push(
                e("input")
                    .attr("id", "r")
                    .attr("type", "range")
                    .attr("min", "10")
                    .attr("max", "20"),
            )
            .into();
        let mut document = Document::from_node(&page);
        let (i, r) = (by_id(&document, "i"), by_id(&document, "r"));
        let as_type = |document: &mut Document, kind: &str, value: &str| {
            document.set_attribute(i, "type", kind).unwrap();
            sanitize(document, i, value)
        };
        for (kind, value, expected) in [
            ("text", "a\r\nb", "ab"),
            ("url", " \n http://x \t", "http://x"),
            ("number", "1.5e3", "1.5e3"),
            ("number", "1.", ""),
            ("number", ".5", ".5"),
            ("number", "+1", ""),
            ("number", "1e", ""),
            ("color", "#00FF00", "#00ff00"),
            ("color", "#0f0", "#000000"),
            ("date", "2024-02-29", "2024-02-29"),
            ("date", "2023-02-29", ""),
            ("date", "0000-01-01", ""),
            ("month", "2024-13", ""),
            ("month", "12024-01", "12024-01"),
            ("week", "2015-W53", "2015-W53"),
            ("week", "2014-W53", ""),
            ("week", "2020-W53", "2020-W53"),
            ("time", "23:59:59.999", "23:59:59.999"),
            ("time", "24:00", ""),
            ("time", "12:00:00.", ""),
            ("datetime-local", "2024-01-01T10:00", "2024-01-01T10:00"),
            ("datetime-local", "2024-01-01 10:00", "2024-01-01 10:00"),
            ("datetime-local", "2024-01-01", ""),
        ] {
            assert_eq!(
                as_type(&mut document, kind, value),
                expected,
                "{kind} {value:?}"
            );
        }
        assert_eq!(sanitize(&document, r, "x"), "15");
        assert_eq!(sanitize(&document, r, "25"), "20");
        assert_eq!(sanitize(&document, r, "12.5"), "12.5");
    }

    #[test]
    fn a_radio_group_keeps_one_checked_and_a_select_one_selected() {
        let page: Node = e("body")
            .push(
                e("form")
                    .push(
                        e("input")
                            .attr("id", "a")
                            .attr("type", "radio")
                            .attr("name", "r")
                            .attr("checked", ""),
                    )
                    .push(
                        e("input")
                            .attr("id", "b")
                            .attr("type", "radio")
                            .attr("name", "r")
                            .attr("checked", ""),
                    )
                    .push(
                        e("input")
                            .attr("id", "c")
                            .attr("type", "radio")
                            .attr("name", "other"),
                    ),
            )
            .push(
                e("select")
                    .attr("id", "s")
                    .push(e("option").attr("id", "o1").text("1"))
                    .push(e("option").attr("id", "o2").text("2")),
            )
            .push(
                e("form").push(
                    e("input")
                        .attr("id", "f")
                        .attr("type", "radio")
                        .attr("name", "r")
                        .attr("checked", ""),
                ),
            )
            .push(
                e("input")
                    .attr("id", "x")
                    .attr("type", "radio")
                    .attr("name", "q"),
            )
            .push(
                e("input")
                    .attr("id", "y")
                    .attr("type", "radio")
                    .attr("name", "q")
                    .attr("checked", ""),
            )
            .into();
        let mut document = Document::from_node(&page);
        let id = |document: &Document, id: &str| by_id(document, id);
        let (a, b, c) = (id(&document, "a"), id(&document, "b"), id(&document, "c"));
        // Parsed with both checked: the last wins.
        assert!(!checkedness(&document, a) && checkedness(&document, b));
        set_checkedness(&mut document, a, true, true).unwrap();
        assert!(checkedness(&document, a) && !checkedness(&document, b));
        assert!(!checkedness(&document, c));
        // The attribute added to a clean one checks it; to a dirty one,
        // nothing.
        document.remove_attribute(b, "checked").unwrap();
        document.set_attribute(b, "checked", "").unwrap();
        assert!(checkedness(&document, b) && !checkedness(&document, a));
        document.remove_attribute(a, "checked").unwrap();
        document.set_attribute(a, "checked", "").unwrap();
        assert!(checkedness(&document, b), "a is dirty");
        // Another form's group of the same name is another group.
        assert!(checkedness(&document, id(&document, "f")));
        // A clean earlier one given the attribute takes the check, a
        // state of its own or not.
        let (x, y) = (id(&document, "x"), id(&document, "y"));
        set_indeterminate(&mut document, x, false).unwrap();
        assert!(document.control(x).is_some_and(|c| !c.dirty_checked));
        document.set_attribute(x, "checked", "").unwrap();
        assert!(checkedness(&document, x) && !checkedness(&document, y));
        let (s, o1, o2) = (id(&document, "s"), id(&document, "o1"), id(&document, "o2"));
        assert_eq!(
            selected_option(&document, s),
            Some(o1),
            "the first, with none selected"
        );
        set_selectedness(&mut document, o2, true, true).unwrap();
        assert_eq!(selected_index(&document, s), Some(1));
        // An earlier one selected unselects the later.
        set_selectedness(&mut document, o1, true, true).unwrap();
        assert_eq!(selected_option(&document, s), Some(o1));
        set_selectedness(&mut document, o2, true, true).unwrap();
        set_selectedness(&mut document, o2, false, true).unwrap();
        assert_eq!(
            selected_option(&document, s),
            None,
            "unselected by a script"
        );
        assert_eq!(value(&document, s), "");
        assert_eq!(value(&document, o1), "1");
    }

    #[test]
    fn a_control_counts_its_value_against_the_data_limit() {
        let page: Node = e("input").attr("id", "i").into();
        let mut document = Document::from_node(&page);
        let i = by_id(&document, "i");
        set_value(&mut document, i, "abc").unwrap();
        assert_eq!(value(&document, i), "abc");
        let caret = selection(&document, i);
        assert_eq!((caret.start, caret.end), (3, 3), "the caret at the end");
        document.validate().unwrap();
        set_value(&mut document, i, "").unwrap();
        document.validate().unwrap();
        assert!(document.control(i).is_some());
    }
}
