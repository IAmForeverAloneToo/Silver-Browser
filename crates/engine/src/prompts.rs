//! A page's user prompts (gap batch A, D43,
//! docs/design/scripting-essentials.md): `alert()`, `confirm()` and
//! `prompt()` ask the user, the page's script waiting for the answer.
//! A renderer's view asks the browser ([`crate::ipc::ToBrowser::Prompt`]),
//! whose tab shows the dialog ([`crate::View::dialog`]) and answers when
//! the user does. An engine with no one to ask answers at once, as a
//! browser that shows no dialogs.

/// The most characters of a dialog's message, and of `prompt()`'s
/// default, shown; the rest is dropped.
pub const MAX_PROMPT_CHARS: usize = 10_000;

/// Which of the three a page called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt,
}

impl DialogKind {
    pub(crate) fn from_name(name: &str) -> Option<DialogKind> {
        match name {
            "alert" => Some(DialogKind::Alert),
            "confirm" => Some(DialogKind::Confirm),
            "prompt" => Some(DialogKind::Prompt),
            _ => None,
        }
    }

    /// Its number on the wire.
    pub(crate) fn number(self) -> u32 {
        self as u32
    }

    pub(crate) fn from_number(number: u32) -> Option<DialogKind> {
        [DialogKind::Alert, DialogKind::Confirm, DialogKind::Prompt]
            .into_iter()
            .find(|kind| kind.number() == number)
    }
}

/// A dialog a tab has open: the shell shows it over the page, and gives
/// its answer to [`crate::View::answer_dialog`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserDialog {
    pub id: u64,
    pub kind: DialogKind,
    /// The origin of the page asking, serialized; empty for an opaque
    /// one.
    pub origin: String,
    pub message: String,
    /// What `prompt()`'s field holds at first.
    pub default: String,
}

/// Who a document's dialogs are asked of, a document of `origin` asking:
/// the answer is `Some` for OK, with what `prompt()`'s field held, and
/// `None` for Cancel.
pub(crate) trait Asker: Send + Sync {
    fn ask(
        &self,
        origin: &net::Origin,
        kind: DialogKind,
        message: &str,
        default: &str,
    ) -> Option<String>;
}

/// `text` as a dialog shows it: its line breaks `\n`, its tabs spaces,
/// without control characters or the ones that reorder text, and at most
/// [`MAX_PROMPT_CHARS`] characters.
pub(crate) fn clean(text: &str) -> String {
    let reorders = |c: char| matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
    text.replace("\r\n", "\n")
        .chars()
        .map(|c| match c {
            '\r' => '\n',
            '\t' => ' ',
            c => c,
        })
        .filter(|&c| c == '\n' || !(c.is_control() || reorders(c)))
        .take(MAX_PROMPT_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dialogs_text_keeps_its_lines_and_drops_what_could_mislead() {
        assert_eq!(clean("a\r\nb\rc\td"), "a\nb\nc d");
        assert_eq!(clean("x\u{0}\u{1b}[2Jy\u{202e}z\u{2067}"), "x[2Jyz");
        assert_eq!(
            clean(&"e".repeat(MAX_PROMPT_CHARS + 5)).len(),
            MAX_PROMPT_CHARS
        );
        for kind in [DialogKind::Alert, DialogKind::Confirm, DialogKind::Prompt] {
            assert_eq!(DialogKind::from_number(kind.number()), Some(kind));
        }
        assert_eq!(DialogKind::from_number(3), None);
        assert_eq!(DialogKind::from_name("print"), None);
    }
}
