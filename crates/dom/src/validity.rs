//! Constraint validation (M26.9b, `docs/design/events.md`, HTML section
//! 4.10.20): which controls are candidates, the validity states each
//! fails, the reference browser's messages for them, a custom validity,
//! and the user validity `:user-valid` and `:user-invalid` read. A
//! `pattern` is an ECMAScript regular expression and a URL is the URL
//! Standard's, which crates above this one know: the engine installs
//! them as [`Checks`]; without them, every pattern and URL passes.

use std::sync::OnceLock;

use crate::forms::{self, ControlState};
use crate::{Document, MutationError, NodeId};

/// What the checks need from the crates above dom.
#[derive(Debug, Clone, Copy)]
pub struct Checks {
    /// Whether a value matches a pattern, as `js::pattern_matches`
    /// answers: `None` for no constraint.
    pub pattern: fn(&str, &str) -> Option<bool>,
    /// Whether a string is a valid absolute URL.
    pub url: fn(&str) -> bool,
}

static CHECKS: OnceLock<Checks> = OnceLock::new();

/// Installs the checks, once for the process.
pub fn install(checks: Checks) {
    let _ = CHECKS.set(checks);
}

/// A control's validity states (HTML section 4.10.20.3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Validity {
    pub value_missing: bool,
    pub type_mismatch: bool,
    pub pattern_mismatch: bool,
    pub too_long: bool,
    pub too_short: bool,
    pub range_underflow: bool,
    pub range_overflow: bool,
    pub step_mismatch: bool,
    pub bad_input: bool,
    pub custom_error: bool,
}

impl Validity {
    /// Whether it fails none of them.
    pub fn valid(&self) -> bool {
        *self == Validity::default()
    }

    /// The states as bits, `valueMissing` the lowest, in the order the
    /// fields have, for the bindings.
    pub fn bits(&self) -> u32 {
        [
            self.value_missing,
            self.type_mismatch,
            self.pattern_mismatch,
            self.too_long,
            self.too_short,
            self.range_underflow,
            self.range_overflow,
            self.step_mismatch,
            self.bad_input,
            self.custom_error,
        ]
        .iter()
        .enumerate()
        .fold(0, |bits, (i, &set)| bits | (u32::from(set) << i))
    }
}

/// The input types text is typed into whose `readonly`, `maxlength`,
/// `minlength` and, but number and the dates, `pattern` apply.
fn texty(kind: &str) -> bool {
    matches!(
        kind,
        "text" | "search" | "url" | "tel" | "email" | "password"
    )
}

/// Whether `readonly` applies to an input of `kind`.
fn readonly_applies(kind: &str) -> bool {
    texty(kind)
        || matches!(
            kind,
            "date" | "month" | "week" | "time" | "datetime-local" | "number"
        )
}

/// Whether `required` applies to the control.
fn required_applies(document: &Document, element: NodeId) -> bool {
    match document.tag(element) {
        Some("input") => !matches!(
            forms::input_type(document, element),
            "hidden" | "range" | "color" | "submit" | "image" | "reset" | "button"
        ),
        Some("select" | "textarea") => true,
        _ => false,
    }
}

/// Whether the control is a candidate for constraint validation (HTML
/// section 4.10.20.2): a submittable one but a hidden, reset or button
/// input, a reset or button `button`, an `object`, one disabled, one in
/// a `datalist`, and one `readonly` where that applies.
pub fn will_validate(document: &Document, element: NodeId) -> bool {
    let candidate = match document.tag(element) {
        Some("input") => {
            let kind = forms::input_type(document, element);
            !matches!(kind, "hidden" | "reset" | "button")
                && !(readonly_applies(kind) && document.attr(element, "readonly").is_some())
        }
        Some("button") => forms::button_type(document, element) == "submit",
        Some("select") => true,
        Some("textarea") => document.attr(element, "readonly").is_none(),
        _ => false,
    };
    candidate
        && !forms::actually_disabled(document, element)
        && !document
            .ancestors(element)
            .any(|ancestor| document.tag(ancestor) == Some("datalist"))
}

/// A control's own state, if it has one.
fn state(document: &Document, element: NodeId) -> ControlState {
    document.control(element).cloned().unwrap_or_default()
}

/// A length in UTF-16 code units, as HTML counts `maxlength`.
fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

/// A non-negative integer attribute.
fn length_attribute(document: &Document, element: NodeId, name: &str) -> Option<usize> {
    document
        .attr(element, name)
        .and_then(crate::parse_non_negative_integer)
        .and_then(|n| usize::try_from(n).ok())
}

/// Whether `text` is a valid email address (HTML section 4.10.5.1.5):
/// the local part's characters, `@`, and labels of letters, digits and
/// inner hyphens, each at most 63 long.
pub fn valid_email(text: &str) -> bool {
    let Some((local, domain)) = text.split_once('@') else {
        return false;
    };
    let local_ok = !local.is_empty()
        && local
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&b));
    let label_ok = |label: &str| {
        let bytes = label.as_bytes();
        (1..=63).contains(&bytes.len())
            && bytes
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
            && bytes[0] != b'-'
            && bytes[bytes.len() - 1] != b'-'
    };
    local_ok && domain.split('.').all(label_ok)
}

/// An email input's values: each of a `multiple` one's comma-separated
/// list, trimmed, or the one.
fn email_values(document: &Document, element: NodeId, value: &str) -> Vec<String> {
    if document.attr(element, "multiple").is_some() {
        value
            .split(',')
            .map(|part| {
                part.trim_matches(|c: char| c.is_ascii_whitespace())
                    .to_owned()
            })
            .collect()
    } else {
        vec![value.to_owned()]
    }
}

/// A number's step (HTML section 4.10.5.3.8): its `step`, a positive
/// number, else 1; `None` for `any`.
fn step(document: &Document, element: NodeId) -> Option<f64> {
    match document.attr(element, "step") {
        Some(step) if step.eq_ignore_ascii_case("any") => None,
        Some(step) => Some(forms::parse_float(step).filter(|&n| n > 0.0).unwrap_or(1.0)),
        None => Some(1.0),
    }
}

/// A number's step base: its `min`, else its `value` attribute, else 0.
fn step_base(document: &Document, element: NodeId) -> f64 {
    ["min", "value"]
        .iter()
        .find_map(|name| document.attr(element, name).and_then(forms::parse_float))
        .unwrap_or(0.0)
}

/// The validity states the control fails (HTML section 4.10.20.3): a
/// required one empty, unchecked, its radio group's none checked or its
/// select's choice none or the placeholder; an email or URL that is not
/// one; a pattern not matched; past `maxlength` or short of `minlength`
/// after the user's edit; a number below `min`, above `max` or off its
/// step; a custom validity set.
pub fn validity(document: &Document, element: NodeId) -> Validity {
    let checks = CHECKS.get();
    let own = state(document, element);
    let mut v = Validity {
        custom_error: own.custom_error.as_deref().is_some_and(|m| !m.is_empty()),
        ..Validity::default()
    };
    let required =
        required_applies(document, element) && document.attr(element, "required").is_some();
    match document.tag(element) {
        Some("input") => {
            let kind = forms::input_type(document, element);
            let value = forms::value(document, element);
            match kind {
                "checkbox" => v.value_missing = required && !forms::checkedness(document, element),
                "radio" => {
                    let group = forms::radio_group(document, element);
                    v.value_missing = group
                        .iter()
                        .any(|&radio| document.attr(radio, "required").is_some())
                        && !group
                            .iter()
                            .any(|&radio| forms::checkedness(document, radio));
                }
                // No file is ever chosen yet (G61).
                "file" => v.value_missing = required,
                _ => v.value_missing = required && value.is_empty(),
            }
            if value.is_empty() {
                return v;
            }
            let values = if kind == "email" {
                email_values(document, element, &value)
            } else {
                vec![value.clone()]
            };
            match kind {
                "email" => v.type_mismatch = values.iter().any(|one| !valid_email(one)),
                "url" => v.type_mismatch = checks.is_some_and(|checks| !(checks.url)(&value)),
                _ => {}
            }
            if texty(kind)
                && let (Some(pattern), Some(checks)) = (document.attr(element, "pattern"), checks)
            {
                v.pattern_mismatch = values
                    .iter()
                    .any(|one| (checks.pattern)(pattern, one) == Some(false));
            }
            if texty(kind) && own.user_edited {
                lengths(document, element, &value, &mut v);
            }
            if kind == "number"
                && let Some(n) = forms::parse_float(&value)
            {
                let bound = |name| document.attr(element, name).and_then(forms::parse_float);
                v.range_underflow = bound("min").is_some_and(|min| n < min);
                v.range_overflow = bound("max").is_some_and(|max| n > max);
                if let Some(step) = step(document, element) {
                    let q = (n - step_base(document, element)) / step;
                    v.step_mismatch = (q - q.round()).abs() > 1e-9 * q.abs().max(1.0);
                }
            }
        }
        Some("textarea") => {
            let value = forms::value(document, element);
            v.value_missing = required && value.is_empty();
            if own.user_edited && !value.is_empty() {
                lengths(document, element, &value, &mut v);
            }
        }
        Some("select") => {
            v.value_missing = required && select_missing(document, element);
        }
        _ => {}
    }
    v
}

/// `tooLong` and `tooShort` of a value the user edited.
fn lengths(document: &Document, element: NodeId, value: &str, v: &mut Validity) {
    let n = units(value);
    v.too_long = length_attribute(document, element, "maxlength").is_some_and(|max| n > max);
    v.too_short = length_attribute(document, element, "minlength").is_some_and(|min| n < min);
}

/// Whether a required select has no value (HTML section 4.10.7): none
/// of its options chosen, or only its placeholder label option, the
/// first option, a child of a single drop-down, with an empty value.
fn select_missing(document: &Document, select: NodeId) -> bool {
    let options = forms::list_of_options(document, select);
    let chosen: Vec<NodeId> = options
        .iter()
        .copied()
        .filter(|&option| forms::selectedness(document, option))
        .collect();
    let placeholder = options.first().copied().filter(|&first| {
        document.attr(select, "multiple").is_none()
            && forms::display_size(document, select) == 1
            && document.parent(first) == Some(select)
            && forms::option_value(document, first).is_empty()
    });
    chosen.is_empty() || (chosen.len() == 1 && Some(chosen[0]) == placeholder)
}

/// A number as the reference browser writes it in a message.
fn number(n: f64) -> String {
    if n == n.trunc() && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// The message the reference browser gives for a candidate's first
/// failure (HTML's `validationMessage`): its custom validity first, then
/// each state's, in its words; empty for one valid or no candidate.
pub fn validation_message(document: &Document, element: NodeId) -> String {
    if !will_validate(document, element) {
        return String::new();
    }
    let v = validity(document, element);
    if v.custom_error {
        return state(document, element).custom_error.unwrap_or_default();
    }
    let kind =
        (document.tag(element) == Some("input")).then(|| forms::input_type(document, element));
    let value = forms::value(document, element);
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    if v.value_missing {
        return match (document.tag(element), kind) {
            (_, Some("checkbox")) => "Please check this box if you want to proceed.",
            (_, Some("radio")) => "Please select one of these options.",
            (_, Some("file")) => "Please select a file.",
            (Some("select"), _) => "Please select an item in the list.",
            _ => "Please fill out this field.",
        }
        .to_owned();
    }
    if v.type_mismatch {
        return match kind {
            Some("email") if !value.contains('@') => {
                format!("Please include an '@' in the email address. '{value}' is missing an '@'.")
            }
            Some("email") => "Please enter an email address.".to_owned(),
            _ => "Please enter a URL.".to_owned(),
        };
    }
    if v.pattern_mismatch {
        return "Please match the requested format.".to_owned();
    }
    let n = units(&value);
    if v.too_long {
        let max = length_attribute(document, element, "maxlength").unwrap_or(0);
        return format!(
            "Please shorten this text to {max} character{} or less (you are currently using {n} character{}).",
            plural(max),
            plural(n)
        );
    }
    if v.too_short {
        let min = length_attribute(document, element, "minlength").unwrap_or(0);
        return format!(
            "Please lengthen this text to {min} character{} or more (you are currently using {n} character{}).",
            plural(min),
            plural(n)
        );
    }
    let bound = |name| document.attr(element, name).and_then(forms::parse_float);
    if v.range_underflow {
        return format!(
            "Value must be greater than or equal to {}.",
            number(bound("min").unwrap_or(0.0))
        );
    }
    if v.range_overflow {
        return format!(
            "Value must be less than or equal to {}.",
            number(bound("max").unwrap_or(0.0))
        );
    }
    if v.step_mismatch
        && let (Some(n), Some(step)) = (forms::parse_float(&value), step(document, element))
    {
        let base = step_base(document, element);
        let low = base + ((n - base) / step).floor() * step;
        let high = low + step;
        if bound("max").is_some_and(|max| high > max) {
            return format!(
                "Please enter a valid value. The nearest valid value is {}.",
                number(low)
            );
        }
        return format!(
            "Please enter a valid value. The two nearest valid values are {} and {}.",
            number(low),
            number(high)
        );
    }
    String::new()
}

/// Sets a control's custom validity message, an empty one clearing it.
pub fn set_custom_validity(
    document: &mut Document,
    element: NodeId,
    message: &str,
) -> Result<(), MutationError> {
    let mut own = state(document, element);
    own.custom_error = (!message.is_empty()).then(|| message.to_owned());
    document.set_control(element, own)
}

/// Sets a control's user validity (HTML section 4.10.20.1), which
/// `:user-valid` and `:user-invalid` need: the user committed a change
/// to it, or tried to submit its form.
pub fn set_user_validity(
    document: &mut Document,
    element: NodeId,
    on: bool,
) -> Result<(), MutationError> {
    let mut own = state(document, element);
    own.user_validity = on;
    document.set_control(element, own)
}

/// Whether the control's user validity is set.
pub fn user_validity(document: &Document, element: NodeId) -> bool {
    document.control(element).is_some_and(|c| c.user_validity)
}

/// The candidates `form` owns, in tree order.
fn candidates(document: &Document, form: NodeId) -> Vec<NodeId> {
    let root = document.ancestors(form).last().unwrap_or(form);
    document
        .subtree(root)
        .filter(|&node| {
            forms::is_submittable(document, node)
                && forms::form_owner(document, node) == Some(form)
                && will_validate(document, node)
        })
        .collect()
}

/// The controls of `form` that fail their constraints, in tree order
/// (HTML's "statically validate the constraints").
pub fn invalid_controls(document: &Document, form: NodeId) -> Vec<NodeId> {
    candidates(document, form)
        .into_iter()
        .filter(|&node| !validity(document, node).valid())
        .collect()
}

/// A submission's attempt (HTML section 4.10.21.3): every control of
/// the form has its user validity set.
pub fn mark_submitted(document: &mut Document, form: NodeId) {
    for node in candidates(document, form) {
        let _ = set_user_validity(document, node, true);
    }
}

/// Whether an element matches `:invalid` (selectors-4, HTML section
/// 4.16.3): a candidate failing a constraint, or a form or fieldset with
/// one inside it.
pub fn invalid(document: &Document, element: NodeId) -> bool {
    match document.tag(element) {
        Some("form") => !invalid_controls(document, element).is_empty(),
        Some("fieldset") => document.subtree(element).skip(1).any(|node| {
            forms::is_submittable(document, node)
                && will_validate(document, node)
                && !validity(document, node).valid()
        }),
        _ => will_validate(document, element) && !validity(document, element).valid(),
    }
}

/// Whether an element matches `:valid`: a candidate failing none, or a
/// form or fieldset with none failing inside it.
pub fn valid(document: &Document, element: NodeId) -> bool {
    match document.tag(element) {
        Some("form" | "fieldset") => !invalid(document, element),
        _ => will_validate(document, element) && validity(document, element).valid(),
    }
}

/// Whether a number or range input has range limitations, for
/// `:in-range` and `:out-of-range`: a range always, a number with a
/// `min` or a `max`.
pub fn has_range(document: &Document, element: NodeId) -> bool {
    document.tag(element) == Some("input")
        && match forms::input_type(document, element) {
            "range" => true,
            "number" => {
                document.attr(element, "min").is_some() || document.attr(element, "max").is_some()
            }
            _ => false,
        }
}

/// Whether an element matches `:read-write` (HTML section 4.16.3): a
/// field whose `readonly` applies and that is neither read-only nor
/// disabled, a textarea likewise.
pub fn read_write(document: &Document, element: NodeId) -> bool {
    let editable = match document.tag(element) {
        Some("input") => readonly_applies(forms::input_type(document, element)),
        Some("textarea") => true,
        _ => false,
    };
    editable
        && document.attr(element, "readonly").is_none()
        && !forms::actually_disabled(document, element)
}

/// Whether an element matches `:default` (HTML section 4.16.3): a
/// checkbox or radio button checked by its attribute, an option selected
/// by its, its form's default button.
pub fn is_default(document: &Document, element: NodeId) -> bool {
    match document.tag(element) {
        Some("input") if matches!(forms::input_type(document, element), "checkbox" | "radio") => {
            document.attr(element, "checked").is_some()
        }
        Some("option") => document.attr(element, "selected").is_some(),
        _ => {
            forms::is_submit_button(document, element)
                && forms::form_owner(document, element)
                    .and_then(|form| forms::default_button(document, form))
                    == Some(element)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_addresses_are_htmls() {
        for (text, ok) in [
            ("a@b", true),
            ("first.last+tag@example.co.uk", true),
            ("a@b-c.d", true),
            ("@b", false),
            ("a@", false),
            ("a b@c", false),
            ("a@-b", false),
            ("a@b..c", false),
            ("a@b@c", false),
        ] {
            assert_eq!(valid_email(text), ok, "{text}");
        }
    }
}
