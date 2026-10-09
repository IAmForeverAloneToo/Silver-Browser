//! The Console Standard's Logger, Formatter and Printer (section 2):
//! a console method's arguments made into the one line of text the
//! host is given.

use rquickjs::{Coerced, Ctx, FromJs, Value};

/// The text of a console call with `args`, or `None` when there are
/// none, which logs nothing (Logger, step 1). A first argument that is
/// a string with further arguments is a format: each `%s`, `%d`, `%i`,
/// `%f`, `%o`, `%O` or `%c` takes the next argument (`%c`'s styles are
/// dropped) and `%%` is a percent sign, as the reference browser has
/// it; the arguments left over follow, a space apart.
pub(super) fn format<'js>(ctx: &Ctx<'js>, args: &[Value<'js>]) -> rquickjs::Result<Option<String>> {
    let Some((first, rest)) = args.split_first() else {
        return Ok(None);
    };
    let Some(target) = first.as_string().filter(|_| !rest.is_empty()) else {
        let printed: Vec<String> = args.iter().map(|arg| describe(ctx, arg)).collect();
        return Ok(Some(printed.join(" ")));
    };
    let target = target.to_string()?;
    let mut out = String::with_capacity(target.len());
    let mut next = rest.iter();
    let mut chars = target.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(&spec) = chars.peek() else {
            out.push('%');
            break;
        };
        if spec == '%' {
            chars.next();
            out.push('%');
            continue;
        }
        if !matches!(spec, 's' | 'd' | 'i' | 'f' | 'o' | 'O' | 'c') {
            out.push('%');
            continue;
        }
        let Some(arg) = next.next() else {
            out.push('%');
            continue;
        };
        chars.next();
        match spec {
            's' | 'o' | 'O' => out.push_str(&describe(ctx, arg)),
            'd' | 'i' => out.push_str(&number(ctx, arg, parse_int)),
            'f' => out.push_str(&number(ctx, arg, parse_float)),
            _ => {}
        }
    }
    for arg in next {
        out.push(' ');
        out.push_str(&describe(ctx, arg));
    }
    Ok(Some(out))
}

/// `value` as text: a string as it is, a symbol as `String()` writes
/// it, anything else by ECMAScript's `ToString`, and a value whose
/// conversion throws as `[object]`, the exception dropped.
pub(super) fn describe<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> String {
    if let Some(text) = value.as_string() {
        return text.to_string().unwrap_or_default();
    }
    // `ToString` throws for a symbol; `String()` writes its description.
    if let Some(symbol) = value.as_symbol() {
        let description = symbol
            .description()
            .ok()
            .and_then(|d| d.as_string().and_then(|d| d.to_string().ok()))
            .unwrap_or_default();
        return format!("Symbol({description})");
    }
    match Coerced::<String>::from_js(ctx, value.clone()) {
        Ok(text) => text.0,
        Err(_) => {
            let _ = ctx.catch();
            "[object]".to_owned()
        }
    }
}

/// `%d`, `%i` and `%f`: `NaN` for a symbol, else `parse` of the value as
/// text, written as ECMAScript writes numbers.
fn number<'js>(ctx: &Ctx<'js>, value: &Value<'js>, parse: fn(&str) -> f64) -> String {
    let n = if value.is_symbol() {
        f64::NAN
    } else {
        parse(&describe(ctx, value))
    };
    describe(ctx, &Value::new_number(ctx.clone(), n))
}

/// ECMAScript's `StrWhiteSpaceChar`: white space and line terminators.
fn is_str_white_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `parseInt(text, 10)` (ECMAScript section 19.2.5): the leading
/// decimal digits after white space and a sign, else `NaN`.
pub(crate) fn parse_int(text: &str) -> f64 {
    let text = text.trim_start_matches(is_str_white_space);
    let (negative, text) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return f64::NAN;
    }
    let n: f64 = text[..digits].parse().unwrap_or(f64::NAN);
    if negative { -n } else { n }
}

/// `parseFloat(text)` (ECMAScript section 19.2.4): the longest prefix
/// after white space that is a decimal literal or `Infinity`, signed,
/// else `NaN`.
pub(crate) fn parse_float(text: &str) -> f64 {
    let text = text.trim_start_matches(is_str_white_space);
    let bytes = text.as_bytes();
    let mut i = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        i = 1;
    }
    if text[i..].starts_with("Infinity") {
        let infinite = f64::INFINITY;
        return if bytes[0] == b'-' {
            -infinite
        } else {
            infinite
        };
    }
    let digits = |from: usize| {
        bytes[from..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    let whole = digits(i);
    let mut end = i + whole;
    if bytes.get(end) == Some(&b'.') {
        let fraction = digits(end + 1);
        if whole > 0 || fraction > 0 {
            end += 1 + fraction;
        }
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut at = end + 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let exponent = digits(at);
        if exponent > 0 {
            end = at + exponent;
        }
    }
    text[..end].parse().unwrap_or(f64::NAN)
}
