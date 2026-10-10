//! A text field's model (docs/design/navigation.md, "The address
//! field", and, since M26.8, docs/design/events.md, "Editing"): its
//! text, the caret, and the other end of the selection, edited by
//! commands. The browser's address field and a page's text controls
//! share it; a textarea's has lines. It knows nothing of drawing.
//!
//! Positions are byte offsets into the text that always stand on a
//! character boundary: the caret moves by Unicode scalar values, and
//! never stands inside one. Grapheme clusters come with M42.

use std::ops::Range;

/// The most bytes the address field holds.
pub const MAX_LEN: usize = 65_536;

/// An edit or a move, as the keys ask for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// One character left, or to the selection's start; with `extend`,
    /// the selection follows.
    Left { extend: bool },
    /// One character right, or to the selection's end.
    Right { extend: bool },
    /// To the start of the word at or before the caret.
    WordLeft { extend: bool },
    /// To the end of the word at or after the caret.
    WordRight { extend: bool },
    /// To the start of the text.
    Home { extend: bool },
    /// To the end of the text.
    End { extend: bool },
    /// To the start of the caret's line (M26.8): the text's in a field
    /// without lines.
    LineStart { extend: bool },
    /// To the end of the caret's line.
    LineEnd { extend: bool },
    /// Deletes the selection, or the character before the caret.
    Backspace,
    /// Deletes the selection, or the character after the caret.
    Delete,
    /// Deletes the selection, or back to the start of the word.
    WordBackspace,
    /// Deletes the selection, or on to the end of the word.
    WordDelete,
    /// Selects the whole text.
    SelectAll,
}

/// An insertion the field refused, since it would pass [`MAX_LEN`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLong;

/// A text field: one line, or lines for a textarea.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextField {
    text: String,
    /// Where the caret stands.
    caret: usize,
    /// The selection's other end; the caret itself when nothing is
    /// selected.
    anchor: usize,
    /// Whether it keeps line feeds: a textarea's.
    lines: bool,
    /// The most bytes it holds.
    max_len: usize,
}

impl Default for TextField {
    fn default() -> TextField {
        TextField {
            text: String::new(),
            caret: 0,
            anchor: 0,
            lines: false,
            max_len: MAX_LEN,
        }
    }
}

impl TextField {
    /// An empty single-line field of at most [`MAX_LEN`] bytes.
    pub fn new() -> TextField {
        TextField::default()
    }

    /// A field holding `text`, with lines or not and at most `max_len`
    /// bytes, its selection from `anchor` to `caret`, each moved to the
    /// character boundary at or before it (M26.8): a page's text
    /// control as its state stands.
    pub fn of(text: &str, lines: bool, max_len: usize, anchor: usize, caret: usize) -> TextField {
        let mut field = TextField {
            text: String::new(),
            caret: 0,
            anchor: 0,
            lines,
            max_len,
        };
        field.text = field.clean(text);
        field.anchor = floor_boundary(&field.text, anchor);
        field.caret = floor_boundary(&field.text, caret);
        field
    }

    /// The caret's line's start and end in a field with lines: the line
    /// feeds before and after it, or the text's ends.
    pub fn line_around(&self, index: usize) -> Range<usize> {
        let index = floor_boundary(&self.text, index);
        let start = self.text[..index].rfind('\n').map_or(0, |at| at + 1);
        let end = self.text[index..]
            .find('\n')
            .map_or(self.text.len(), |at| index + at);
        start..end
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Where the caret stands, as a byte offset.
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selection, as a byte range; empty, at the caret, when
    /// nothing is selected.
    pub fn selection(&self) -> Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    /// The text selected.
    pub fn selected(&self) -> &str {
        &self.text[self.selection()]
    }

    /// Replaces the text, the caret at its end and nothing selected. The
    /// text is cleaned as an insertion's is, and cut at [`MAX_LEN`] on a
    /// character boundary: what the field is given to show, not what
    /// the user types.
    pub fn set_text(&mut self, text: &str) {
        let mut text = self.clean(text);
        if text.len() > self.max_len {
            let mut end = self.max_len;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        self.text = text;
        self.caret = self.text.len();
        self.anchor = self.caret;
    }

    /// Selects the whole text.
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// Puts the caret at `index`, the nearest character boundary at or
    /// before it; with `extend`, the selection's other end stays.
    pub fn place(&mut self, index: usize, extend: bool) {
        self.caret = floor_boundary(&self.text, index);
        if !extend {
            self.anchor = self.caret;
        }
    }

    /// Selects the word at `index`, or the run of other characters
    /// there: what a double click selects.
    pub fn select_word_at(&mut self, index: usize) {
        let index = floor_boundary(&self.text, index);
        let at = self.text[index..]
            .chars()
            .next()
            .or_else(|| self.text[..index].chars().next_back());
        let Some(at) = at else {
            return;
        };
        let wordy = is_word(at);
        let start = self.text[..index]
            .char_indices()
            .rev()
            .take_while(|&(_, c)| is_word(c) == wordy)
            .last()
            .map_or(index, |(i, _)| i);
        let end = self.text[index..]
            .char_indices()
            .take_while(|&(_, c)| is_word(c) == wordy)
            .last()
            .map_or(index, |(i, c)| index + i + c.len_utf8());
        self.anchor = start;
        self.caret = end;
    }

    /// Inserts `text` at the caret in place of the selection: typed,
    /// pasted, or committed by an input method. Its control characters
    /// are dropped first, and its line breaks but in a field with lines,
    /// where each is a line feed. Refused whole when the field would hold
    /// more than its most bytes.
    pub fn insert(&mut self, text: &str) -> Result<(), TooLong> {
        let text = self.clean(text);
        let selection = self.selection();
        if self.text.len() - selection.len() + text.len() > self.max_len {
            return Err(TooLong);
        }
        self.text.replace_range(selection.clone(), &text);
        self.caret = selection.start + text.len();
        self.anchor = self.caret;
        Ok(())
    }

    /// `text` as [`TextField::insert`] would insert it, cut to the most
    /// UTF-16 code units that leave the field at `units` or fewer, on a
    /// character boundary (M26.8): what a control's `maxlength` lets in,
    /// counted as HTML counts it.
    pub fn fitting(&self, text: &str, units: usize) -> String {
        let text = self.clean(text);
        let kept =
            self.text.encode_utf16().count() - self.text[self.selection()].encode_utf16().count();
        let mut room = units.saturating_sub(kept);
        let mut end = 0;
        for (at, c) in text.char_indices() {
            if c.len_utf16() > room {
                break;
            }
            room -= c.len_utf16();
            end = at + c.len_utf8();
        }
        text[..end].to_owned()
    }

    /// `text` without its control characters, and without its line
    /// breaks but in a field with lines, where each is made a line feed.
    fn clean(&self, text: &str) -> String {
        if self.lines {
            text.replace("\r\n", "\n")
                .replace('\r', "\n")
                .chars()
                .filter(|&c| c == '\n' || !c.is_control())
                .collect()
        } else {
            clean(text)
        }
    }

    /// Takes the selection out of the text and gives it back: a cut.
    pub fn cut(&mut self) -> String {
        let selection = self.selection();
        let cut = self.text[selection.clone()].to_owned();
        self.text.replace_range(selection.clone(), "");
        self.caret = selection.start;
        self.anchor = self.caret;
        cut
    }

    /// Carries out `command`.
    pub fn apply(&mut self, command: Command) {
        let selection = self.selection();
        match command {
            Command::Left { extend } => {
                let to = if !extend && !selection.is_empty() {
                    selection.start
                } else {
                    prev_boundary(&self.text, self.caret)
                };
                self.place(to, extend);
            }
            Command::Right { extend } => {
                let to = if !extend && !selection.is_empty() {
                    selection.end
                } else {
                    next_boundary(&self.text, self.caret)
                };
                self.place(to, extend);
            }
            Command::WordLeft { extend } => self.place(word_left(&self.text, self.caret), extend),
            Command::WordRight { extend } => {
                self.place(word_right(&self.text, self.caret), extend);
            }
            Command::Home { extend } => self.place(0, extend),
            Command::End { extend } => self.place(self.text.len(), extend),
            Command::LineStart { extend } => {
                let start = self.line_around(self.caret).start;
                self.place(start, extend);
            }
            Command::LineEnd { extend } => {
                let end = self.line_around(self.caret).end;
                self.place(end, extend);
            }
            Command::Backspace => {
                self.delete_or(|text, caret| prev_boundary(text, caret)..caret);
            }
            Command::Delete => self.delete_or(|text, caret| caret..next_boundary(text, caret)),
            Command::WordBackspace => self.delete_or(|text, caret| word_left(text, caret)..caret),
            Command::WordDelete => self.delete_or(|text, caret| caret..word_right(text, caret)),
            Command::SelectAll => self.select_all(),
        }
    }

    /// Deletes the selection, or else the range `around` gives for the
    /// caret.
    fn delete_or(&mut self, around: impl Fn(&str, usize) -> Range<usize>) {
        let mut range = self.selection();
        if range.is_empty() {
            range = around(&self.text, self.caret);
        }
        self.text.replace_range(range.clone(), "");
        self.caret = range.start;
        self.anchor = self.caret;
    }
}

/// `text` without its line breaks and other control characters.
fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// Whether `c` belongs to a word: a letter or a digit, so `/`, `.`, `:`
/// and `-` part words, as in the browsers' address bars.
fn is_word(c: char) -> bool {
    c.is_alphanumeric()
}

/// The character boundary at or before `index`.
fn floor_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// The character boundary before `index`, or the start.
fn prev_boundary(text: &str, index: usize) -> usize {
    text[..index]
        .char_indices()
        .next_back()
        .map_or(0, |(i, _)| i)
}

/// The character boundary after `index`, or the end.
fn next_boundary(text: &str, index: usize) -> usize {
    text[index..]
        .chars()
        .next()
        .map_or(text.len(), |c| index + c.len_utf8())
}

/// The start of the word at or before `index`: back over what is not a
/// word, then over the word.
fn word_left(text: &str, index: usize) -> usize {
    let mut chars = text[..index].char_indices().rev().peekable();
    while chars.next_if(|&(_, c)| !is_word(c)).is_some() {}
    let mut start = chars.peek().map_or(0, |&(i, c)| i + c.len_utf8());
    while let Some((i, _)) = chars.next_if(|&(_, c)| is_word(c)) {
        start = i;
    }
    start
}

/// The end of the word at or after `index`: on over what is not a word,
/// then over the word.
fn word_right(text: &str, index: usize) -> usize {
    let mut chars = text[index..].char_indices().peekable();
    while chars.next_if(|&(_, c)| !is_word(c)).is_some() {}
    let mut end = chars.peek().map_or(text.len() - index, |&(i, _)| i);
    while let Some((i, c)) = chars.next_if(|&(_, c)| is_word(c)) {
        end = i + c.len_utf8();
    }
    index + end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(text: &str) -> TextField {
        let mut field = TextField::new();
        field.set_text(text);
        field
    }

    #[test]
    fn typing_inserts_at_the_caret_in_place_of_the_selection() {
        let mut f = field("https://example.com/");
        f.apply(Command::SelectAll);
        assert_eq!(f.selected(), "https://example.com/");
        f.insert("exa").unwrap();
        f.insert("mple.org").unwrap();
        assert_eq!((f.text(), f.caret()), ("example.org", 11));
        f.apply(Command::Home { extend: false });
        f.insert("www.").unwrap();
        assert_eq!(f.text(), "www.example.org");
        // Line breaks and control characters go; the rest is kept.
        f.apply(Command::End { extend: false });
        f.insert("/a\r\nb\tc\u{7}\u{e9}").unwrap();
        assert_eq!(f.text(), "www.example.org/abc\u{e9}");
    }

    #[test]
    fn left_and_right_move_a_character_or_collapse_the_selection() {
        let mut f = field("a\u{f1}b");
        f.apply(Command::Left { extend: false });
        f.apply(Command::Left { extend: false });
        assert_eq!(f.caret(), 1, "over the two-byte \u{f1}");
        f.apply(Command::Right { extend: true });
        assert_eq!(f.selected(), "\u{f1}");
        f.apply(Command::Left { extend: false });
        assert_eq!((f.caret(), f.selection()), (1, 1..1), "to the start");
        f.apply(Command::Right { extend: true });
        f.apply(Command::Right { extend: true });
        f.apply(Command::Right { extend: false });
        assert_eq!(f.caret(), 4, "to the end");
        f.apply(Command::Right { extend: false });
        assert_eq!(f.caret(), 4, "no further");
        f.apply(Command::Home { extend: true });
        assert_eq!(f.selected(), "a\u{f1}b");
        // A selection of more than one character collapses to its end.
        f.place(4, false);
        f.place(0, true);
        f.apply(Command::Right { extend: false });
        assert_eq!((f.caret(), f.selection()), (4, 4..4));
        f.apply(Command::SelectAll);
        f.apply(Command::Left { extend: false });
        assert_eq!((f.caret(), f.selection()), (0, 0..0));
    }

    #[test]
    fn words_end_at_the_punctuation_of_a_url() {
        let text = "https://www.example.com/a-b?q=1";
        let mut f = field(text);
        let mut stops = Vec::new();
        f.apply(Command::Home { extend: false });
        loop {
            f.apply(Command::WordRight { extend: false });
            stops.push(&text[..f.caret()]);
            if f.caret() == text.len() {
                break;
            }
        }
        assert_eq!(
            stops,
            [
                "https",
                "https://www",
                "https://www.example",
                "https://www.example.com",
                "https://www.example.com/a",
                "https://www.example.com/a-b",
                "https://www.example.com/a-b?q",
                "https://www.example.com/a-b?q=1",
            ]
        );
        f.apply(Command::WordLeft { extend: false });
        assert_eq!(&text[f.caret()..], "1");
        f.apply(Command::WordLeft { extend: false });
        assert_eq!(&text[f.caret()..], "q=1");
        f.apply(Command::WordBackspace);
        assert_eq!(f.text(), "https://www.example.com/a-q=1");
        f.apply(Command::WordDelete);
        assert_eq!(f.text(), "https://www.example.com/a-=1");
    }

    #[test]
    fn backspace_and_delete_take_the_selection_or_one_character() {
        let mut f = field("\u{65e5}\u{672c}\u{8a9e}");
        f.apply(Command::Backspace);
        assert_eq!(f.text(), "\u{65e5}\u{672c}");
        f.apply(Command::Home { extend: false });
        f.apply(Command::Delete);
        assert_eq!(f.text(), "\u{672c}");
        f.apply(Command::Delete);
        f.apply(Command::Delete);
        f.apply(Command::Backspace);
        assert_eq!((f.text(), f.caret()), ("", 0));
        let mut f = field("abcdef");
        f.place(1, false);
        f.place(4, true);
        f.apply(Command::Backspace);
        assert_eq!(f.text(), "aef");
    }

    #[test]
    fn a_double_click_selects_the_word_or_the_run_between_words() {
        let mut f = field("https://example.com/path");
        f.select_word_at(10);
        assert_eq!(f.selected(), "example");
        f.select_word_at(6);
        assert_eq!(f.selected(), "://");
        f.select_word_at(f.text().len());
        assert_eq!(f.selected(), "path");
        let mut empty = TextField::new();
        empty.select_word_at(0);
        assert_eq!(empty.selection(), 0..0);
    }

    #[test]
    fn the_field_holds_at_most_max_len_bytes() {
        let mut f = TextField::new();
        f.insert(&"a".repeat(MAX_LEN - 1)).unwrap();
        assert_eq!(f.insert("bb"), Err(TooLong), "refused whole");
        assert_eq!(f.text().len(), MAX_LEN - 1);
        assert_eq!(f.insert("\u{e9}"), Err(TooLong), "two bytes, one free");
        f.insert("b").unwrap();
        assert_eq!(f.text().len(), MAX_LEN);
        // Replacing a selection frees what it held.
        f.apply(Command::SelectAll);
        f.insert(&"c".repeat(MAX_LEN)).unwrap();
        assert_eq!(f.text(), "c".repeat(MAX_LEN));
        // Text given to show is cut on a boundary instead.
        let mut f = TextField::new();
        f.set_text(&"\u{e9}".repeat(MAX_LEN));
        assert_eq!(f.text().len(), MAX_LEN);
        assert!(f.text().chars().all(|c| c == '\u{e9}'));
    }

    #[test]
    fn a_cut_takes_the_selection() {
        let mut f = field("example.com");
        f.place(0, false);
        f.place(8, true);
        assert_eq!(f.cut(), "example.");
        assert_eq!((f.text(), f.caret()), ("com", 0));
        assert_eq!(f.cut(), "");
        let mut f = field("example.com");
        f.place(7, false);
        f.place(11, true);
        assert_eq!(f.cut(), ".com");
        assert_eq!((f.text(), f.caret()), ("example", 7));
    }

    #[test]
    fn a_field_with_lines_keeps_its_line_feeds() {
        let mut f = TextField::of("one\r\ntwo", true, MAX_LEN, 0, 0);
        assert_eq!(f.text(), "one\ntwo");
        f.place(5, false);
        f.apply(Command::LineStart { extend: true });
        assert_eq!(f.selected(), "t");
        f.apply(Command::LineEnd { extend: false });
        assert_eq!(f.caret(), 7);
        f.insert("\r\nthree\u{7}").unwrap();
        assert_eq!(f.text(), "one\ntwo\nthree");
        assert_eq!(f.line_around(1), 0..3);
        let mut flat = TextField::of("a\nb", false, MAX_LEN, 9, 9);
        assert_eq!((flat.text(), flat.caret()), ("ab", 2));
        flat.apply(Command::LineStart { extend: false });
        assert_eq!(flat.caret(), 0, "the text's start");
    }

    #[test]
    fn what_fits_is_counted_in_utf16_units() {
        let f = TextField::of("ab", false, MAX_LEN, 2, 2);
        assert_eq!(f.fitting("cde", 4), "cd");
        assert_eq!(f.fitting("\u{1f600}x", 4), "\u{1f600}");
        assert_eq!(f.fitting("\u{1f600}", 3), "", "a pair does not split");
        let all = TextField::of("ab", false, MAX_LEN, 0, 2);
        assert_eq!(all.fitting("wxyz", 3), "wxy", "the selection is replaced");
        assert_eq!(f.fitting("x", 1), "", "already past it");
    }

    /// A seeded run of thousands of commands over text with characters
    /// of every width: after each, the caret and the selection stand on
    /// character boundaries inside the text.
    #[test]
    fn any_run_of_commands_keeps_the_caret_on_boundaries() {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let pieces = [
            "a",
            "\u{e9}",
            "\u{65e5}\u{672c}",
            "\u{1f600}",
            "-",
            "/",
            " ",
            "ab.cd",
            "\u{f1}\u{301}",
            "",
        ];
        let mut f = TextField::new();
        for _ in 0..20_000 {
            let extend = next() % 2 == 0;
            match next() % 16 {
                0 => {
                    let _ = f.insert(pieces[(next() % pieces.len() as u64) as usize]);
                }
                1 => f.apply(Command::Left { extend }),
                2 => f.apply(Command::Right { extend }),
                3 => f.apply(Command::WordLeft { extend }),
                4 => f.apply(Command::WordRight { extend }),
                5 => f.apply(Command::Home { extend }),
                6 => f.apply(Command::End { extend }),
                7 => f.apply(Command::Backspace),
                8 => f.apply(Command::Delete),
                9 => f.apply(Command::WordBackspace),
                10 => f.apply(Command::WordDelete),
                11 => f.apply(if extend {
                    Command::LineStart { extend }
                } else {
                    Command::SelectAll
                }),
                12 => f.place((next() % (f.text().len() as u64 + 3)) as usize, extend),
                13 => f.select_word_at((next() % (f.text().len() as u64 + 1)) as usize),
                14 => {
                    let _ = f.cut();
                }
                _ => {
                    let _ = f.insert("xyz");
                }
            }
            let text = f.text();
            for at in [f.caret, f.anchor] {
                assert!(
                    at <= text.len() && text.is_char_boundary(at),
                    "{at} in {text:?}"
                );
            }
            let _ = f.selected();
        }
    }
}
