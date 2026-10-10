//! The presentational hints of form controls (M26.7, HTML section
//! 15.5.15): a text field's `size`, a textarea's `cols` and `rows`, a
//! list box's rows. The reference browser sizes them by its font's
//! average character width and line height; these take its figures at
//! its default 13.33px Arial as ems, so that a field is its size there
//! and scales with the font elsewhere.

/// `n` ten-thousandths of an em, written as ems, exactly.
fn ems(n: u64) -> String {
    let (whole, part) = (n / 10_000, n % 10_000);
    if part == 0 {
        return format!("{whole}em");
    }
    let part = format!("{part:04}");
    format!("{whole}.{}em", part.trim_end_matches('0'))
}

/// The width of a text field of `size` characters, in ten-thousandths of
/// an em: 0.525em a character and 2.175em more, 169px for the default
/// 20.
fn field_width(size: u64) -> u64 {
    size.saturating_mul(5_250).saturating_add(21_750)
}

/// Whether the attribute `name` of an element named `tag` is one of the
/// hints [`control_hints`] reads, whose change restyles the element.
pub(crate) fn is_control_hint(tag: &str, name: &str) -> bool {
    matches!(
        (tag, name),
        ("input", "size" | "type")
            | ("textarea", "cols" | "rows")
            | ("select", "size" | "multiple")
    )
}

/// The declarations a control's attributes are hints for, as the text of
/// a declaration block: a text field's `width` from its `size`; a
/// textarea's `width` from its `cols`, 0.5475em a column and 1.2em more,
/// and its `height` from its `rows`, 1.125em a row; a list box's
/// `height`, `multiple` or a `size` above 1 making one, 1.275em a row of
/// its display size and its border. Each a positive integer; empty for
/// any other element, and where the attribute is missing or invalid, the
/// UA sheet's defaults standing.
pub(super) fn control_hints(tag: &str, attr: impl Fn(&str) -> Option<String>) -> String {
    let positive = |name: &str| {
        attr(name)
            .and_then(|value| dom::parse_non_negative_integer(&value))
            .filter(|&n| n > 0)
    };
    match tag {
        "input" => {
            let kind = attr("type").unwrap_or_default().to_ascii_lowercase();
            let sized = matches!(
                kind.as_str(),
                "" | "text" | "search" | "tel" | "url" | "email" | "password"
            ) || !dom::forms::INPUT_TYPES.contains(&kind.as_str());
            match positive("size") {
                Some(size) if sized => format!("width: {};", ems(field_width(size))),
                _ => String::new(),
            }
        }
        "textarea" => {
            let mut out = String::new();
            if let Some(cols) = positive("cols") {
                let width = cols.saturating_mul(5_475).saturating_add(12_000);
                out.push_str(&format!("width: {};", ems(width)));
            }
            if let Some(rows) = positive("rows") {
                out.push_str(&format!("height: {};", ems(rows.saturating_mul(11_250))));
            }
            out
        }
        "select" => {
            let multiple = attr("multiple").is_some();
            let size = positive("size").unwrap_or(if multiple { 4 } else { 1 });
            if size > 1 {
                format!(
                    "height: {};",
                    ems(size.saturating_mul(12_750).saturating_add(1_500))
                )
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hints(tag: &str, attrs: &[(&str, &str)]) -> String {
        control_hints(tag, |name| {
            attrs
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| (*v).to_owned())
        })
    }

    #[test]
    fn sizes_are_the_reference_browsers_at_its_default_font() {
        let px = |n: u64| (n as f32 / 10_000.0 * 13.3333).round();
        assert_eq!(px(field_width(20)), 169.0);
        assert_eq!(px(field_width(10)), 99.0);
        assert_eq!(ems(12_000), "1.2em");
        assert_eq!(ems(20_000), "2em");
        assert_eq!(hints("input", &[("size", "10")]), "width: 7.425em;");
        assert_eq!(
            hints("input", &[("size", "10"), ("type", "Email")]),
            "width: 7.425em;"
        );
        assert_eq!(hints("input", &[("size", "10"), ("type", "number")]), "");
        assert_eq!(hints("input", &[("size", "0")]), "");
        assert_eq!(hints("input", &[("size", "x")]), "");
        assert_eq!(
            hints("textarea", &[("cols", "10"), ("rows", "4")]),
            "width: 6.675em;height: 4.5em;"
        );
        assert_eq!(hints("select", &[("size", "3")]), "height: 3.975em;");
        assert_eq!(hints("select", &[("multiple", "")]), "height: 5.25em;");
        assert_eq!(hints("select", &[("size", "1")]), "");
        assert_eq!(hints("select", &[]), "");
        assert_eq!(hints("div", &[("size", "3")]), "");
    }
}
