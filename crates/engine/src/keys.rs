//! Keys as the page hears of them (M26.6, `docs/design/events.md`, UI
//! Events section 5.6): a key's `key` value, its `code`, its location
//! and whether it repeats; and the legacy `keyCode` the reference
//! browser gives the keys every browser agrees on.

/// A key pressed or released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    /// Its `key` value: the character it types, or its name (`Enter`,
    /// `ArrowLeft`, `Shift`, ...); `" "` for the space bar.
    pub key: String,
    /// The physical key, its `code` (`KeyA`, `Digit1`, `ShiftLeft`, ...).
    pub code: String,
    /// Its `location`: 0 standard, 1 left, 2 right, 3 the numeric pad.
    pub location: u32,
    /// Whether it is held down and repeating.
    pub repeat: bool,
    /// The text it types, if any, for its `keypress`.
    pub text: Option<String>,
}

impl Key {
    /// A key of the standard location, not repeating.
    pub fn new(key: &str, code: &str) -> Key {
        Key {
            key: key.to_owned(),
            code: code.to_owned(),
            location: 0,
            repeat: false,
            text: None,
        }
    }

    /// The key typing `text`.
    pub fn with_text(mut self, text: &str) -> Key {
        self.text = Some(text.to_owned());
        self
    }
}

/// What a key press did on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    /// The page took it: a listener canceled it, or its default action
    /// was done.
    Taken,
    /// Tab went past the last area that takes the focus, or Shift+Tab
    /// before the first: the browser's own interface is to take it.
    FocusLeft,
    /// The page did nothing with it, for the browser to use.
    Ignored,
}

/// The legacy `keyCode` of `key` (UI Events appendix B): the code of a
/// letter's capital or a digit, the numeric pad's own, the punctuation's
/// of a US layout, and the named keys'; 0 for the rest.
pub(crate) fn key_code(key: &Key) -> u32 {
    let code = key.code.as_str();
    if let Some(letter) = code.strip_prefix("Key")
        && let [c] = letter.as_bytes()
        && c.is_ascii_uppercase()
    {
        return u32::from(*c);
    }
    if let Some(digit) = code.strip_prefix("Digit")
        && let [c] = digit.as_bytes()
        && c.is_ascii_digit()
    {
        return u32::from(*c);
    }
    if let Some(digit) = code.strip_prefix("Numpad")
        && let [c] = digit.as_bytes()
        && c.is_ascii_digit()
    {
        return 96 + u32::from(c - b'0');
    }
    let by_code = match code {
        "NumpadMultiply" => 106,
        "NumpadAdd" => 107,
        "NumpadSubtract" => 109,
        "NumpadDecimal" => 110,
        "NumpadDivide" => 111,
        "NumpadEnter" => 13,
        "Semicolon" => 186,
        "Equal" => 187,
        "Comma" => 188,
        "Minus" => 189,
        "Period" => 190,
        "Slash" => 191,
        "Backquote" => 192,
        "BracketLeft" => 219,
        "Backslash" => 220,
        "BracketRight" => 221,
        "Quote" => 222,
        _ => 0,
    };
    if by_code != 0 {
        return by_code;
    }
    match key.key.as_str() {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" => 13,
        "Shift" => 16,
        "Control" => 17,
        "Alt" | "AltGraph" => 18,
        "Pause" => 19,
        "CapsLock" => 20,
        "Escape" => 27,
        " " => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "Insert" => 45,
        "Delete" => 46,
        "Meta" => 91,
        "ContextMenu" => 93,
        "NumLock" => 144,
        "ScrollLock" => 145,
        name => name
            .strip_prefix('F')
            .and_then(|n| n.parse::<u32>().ok())
            .filter(|n| (1..=24).contains(n))
            .map_or(0, |n| 111 + n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_codes_are_the_reference_browsers() {
        for (key, code, expected) in [
            ("a", "KeyA", 65),
            ("A", "KeyA", 65),
            ("1", "Digit1", 49),
            ("!", "Digit1", 49),
            ("1", "Numpad1", 97),
            ("Enter", "NumpadEnter", 13),
            (";", "Semicolon", 186),
            ("Tab", "Tab", 9),
            ("Enter", "Enter", 13),
            (" ", "Space", 32),
            ("ArrowLeft", "ArrowLeft", 37),
            ("Shift", "ShiftLeft", 16),
            ("F5", "F5", 116),
            ("F25", "F25", 0),
            ("Unidentified", "", 0),
        ] {
            assert_eq!(key_code(&Key::new(key, code)), expected, "{key} {code}");
        }
    }
}
