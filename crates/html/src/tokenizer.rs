//! The HTML tokenizer: HTML section 13.2.5's state machine (M24.8,
//! `docs/design/scripting.md`).
//!
//! Every state of the section is here, the script data states with their
//! escapes among them, RCDATA, RAWTEXT, PLAINTEXT, CDATA sections,
//! processing instructions and the full table of named character
//! references. The input is fed in pieces ([`Tokenizer::feed`], then
//! [`Tokenizer::end`]), and the tokenizer keeps its state between them:
//! the tokens are the same whatever the pieces, but that a run of text
//! may come as more than one [`Token::Text`] where a piece ends. When it
//! has used all it was given it returns `None` until it is given more.
//!
//! The input stream is preprocessed as section 13.2.3.5 says: a carriage
//! return and a carriage return followed by a line feed become a line
//! feed, across the end of a piece too. Lines are counted
//! ([`Tokenizer::line`]), so that an inline script's errors name the
//! document's line.
//!
//! What state an element's content is tokenized in is the tree
//! builder's to say ([`Tokenizer::set_state`]), as are whether a CDATA
//! section is foreign content's or a bogus comment
//! ([`Tokenizer::set_cdata_allowed`]). Parse errors are not reported.
//!
//! The tokenizer never panics on any input, and its work is linear in
//! the input: every step consumes a character, or leaves the state it is
//! in for one that will, and the longest look ahead, a named character
//! reference's, is 32 characters.

use std::collections::{HashSet, VecDeque};

use crate::named_references::{LONGEST_NAME, NAMED_REFERENCES};

/// A lexical token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// A doctype declaration, e.g. `<!DOCTYPE html>`.
    Doctype(Doctype),
    /// An opening tag.
    StartTag {
        /// Its name, ASCII lowercased.
        name: String,
        /// Its attributes, names ASCII lowercased, in order, the first of
        /// any name kept.
        attrs: Vec<(String, String)>,
        /// Whether it ended with `/>`.
        self_closing: bool,
    },
    /// A closing tag; its attributes, an error, are dropped.
    EndTag(String),
    /// Characters, with character references decoded.
    Text(String),
    /// A comment, a bogus one included.
    Comment(String),
    /// A processing instruction, `<?target data?>`.
    ProcessingInstruction { target: String, data: String },
}

/// A doctype: each part is `None` when it is missing, which is not the
/// empty string.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Doctype {
    pub name: Option<String>,
    pub public_id: Option<String>,
    pub system_id: Option<String>,
    pub force_quirks: bool,
}

/// The states the tree builder, or a test, may start an element's
/// content in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextState {
    Data,
    Rcdata,
    Rawtext,
    ScriptData,
    Plaintext,
    CdataSection,
}

/// Every state of section 13.2.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Data,
    Rcdata,
    Rawtext,
    ScriptData,
    Plaintext,
    TagOpen,
    EndTagOpen,
    TagName,
    RcdataLessThan,
    RcdataEndTagOpen,
    RcdataEndTagName,
    RawtextLessThan,
    RawtextEndTagOpen,
    RawtextEndTagName,
    ScriptDataLessThan,
    ScriptDataEndTagOpen,
    ScriptDataEndTagName,
    ScriptDataEscapeStart,
    ScriptDataEscapeStartDash,
    ScriptDataEscaped,
    ScriptDataEscapedDash,
    ScriptDataEscapedDashDash,
    ScriptDataEscapedLessThan,
    ScriptDataEscapedEndTagOpen,
    ScriptDataEscapedEndTagName,
    ScriptDataDoubleEscapeStart,
    ScriptDataDoubleEscaped,
    ScriptDataDoubleEscapedDash,
    ScriptDataDoubleEscapedDashDash,
    ScriptDataDoubleEscapedLessThan,
    ScriptDataDoubleEscapeEnd,
    BeforeAttributeName,
    AttributeName,
    AfterAttributeName,
    BeforeAttributeValue,
    AttributeValueDoubleQuoted,
    AttributeValueSingleQuoted,
    AttributeValueUnquoted,
    AfterAttributeValueQuoted,
    SelfClosingStartTag,
    BogusComment,
    MarkupDeclarationOpen,
    CommentStart,
    CommentStartDash,
    Comment,
    CommentLessThan,
    CommentLessThanBang,
    CommentLessThanBangDash,
    CommentLessThanBangDashDash,
    CommentEndDash,
    CommentEnd,
    CommentEndBang,
    Doctype,
    BeforeDoctypeName,
    DoctypeName,
    AfterDoctypeName,
    AfterDoctypePublicKeyword,
    BeforeDoctypePublicIdentifier,
    DoctypePublicIdentifierDoubleQuoted,
    DoctypePublicIdentifierSingleQuoted,
    AfterDoctypePublicIdentifier,
    BetweenDoctypePublicAndSystemIdentifiers,
    AfterDoctypeSystemKeyword,
    BeforeDoctypeSystemIdentifier,
    DoctypeSystemIdentifierDoubleQuoted,
    DoctypeSystemIdentifierSingleQuoted,
    AfterDoctypeSystemIdentifier,
    BogusDoctype,
    CdataSection,
    CdataSectionBracket,
    CdataSectionEnd,
    ProcessingInstructionOpen,
    ProcessingInstructionTarget,
    AfterProcessingInstructionTarget,
    ProcessingInstructionData,
    ProcessingInstructionQuestionable,
    CharacterReference,
    NamedCharacterReference,
    AmbiguousAmpersand,
    NumericCharacterReference,
    HexadecimalCharacterReferenceStart,
    HexadecimalCharacterReference,
    DecimalCharacterReference,
    NumericCharacterReferenceEnd,
}

/// What the next input character is.
enum Next {
    Char(char),
    /// The input has ended.
    Eof,
    /// The input so far is used up; more will come.
    Wait,
}

/// Whether a step may be followed by another, or must wait for input.
enum Flow {
    Go,
    Wait,
}

/// The tag being built.
#[derive(Default)]
struct Tag {
    end: bool,
    name: String,
    self_closing: bool,
    attrs: Vec<(String, String)>,
    /// The attribute being built, if any.
    attr: Option<(String, String)>,
    /// The names of `attrs`, once there are enough of them that looking
    /// through the list for a duplicate would cost more than a set.
    seen: Option<HashSet<String>>,
}

/// How many attributes a tag has before their names go in a set.
const MANY_ATTRIBUTES: usize = 16;

impl Tag {
    fn new(end: bool) -> Tag {
        Tag {
            end,
            ..Tag::default()
        }
    }

    /// Ends the attribute being built: kept unless the tag has one of
    /// its name already (a duplicate-attribute parse error).
    fn commit(&mut self) {
        let Some((name, value)) = self.attr.take() else {
            return;
        };
        let duplicate = match &mut self.seen {
            Some(seen) => !seen.insert(name.clone()),
            None => self.attrs.iter().any(|(n, _)| *n == name),
        };
        if duplicate {
            return;
        }
        self.attrs.push((name, value));
        if self.seen.is_none() && self.attrs.len() >= MANY_ATTRIBUTES {
            self.seen = Some(self.attrs.iter().map(|(n, _)| n.clone()).collect());
        }
    }

    fn start_attr(&mut self, name: String) {
        self.commit();
        self.attr = Some((name, String::new()));
    }

    fn attr_name(&mut self) -> &mut String {
        &mut self.attr.get_or_insert_with(Default::default).0
    }

    fn attr_value(&mut self) -> &mut String {
        &mut self.attr.get_or_insert_with(Default::default).1
    }
}

/// HTML section 13.2.5's tokenizer over input fed in pieces.
pub struct Tokenizer {
    input: Vec<char>,
    pos: usize,
    /// The input stream's insertion points (HTML section 13.2.3.5), the
    /// innermost last: positions in `input` where `document.write` puts
    /// its text, one a script nesting level (M25).
    points: Vec<usize>,
    /// Where the input ends for now while a write's text is parsed: the
    /// innermost insertion point, past which nothing is read.
    bound: Option<usize>,
    /// The last piece fed ended with a carriage return: a line feed
    /// opening the next is its pair.
    after_cr: bool,
    ended: bool,
    finished: bool,
    state: State,
    return_state: State,
    tag: Tag,
    comment: String,
    doctype: Doctype,
    pi_target: String,
    pi_data: String,
    temp: String,
    char_ref: u32,
    last_start_tag: Option<String>,
    cdata_allowed: bool,
    /// Characters not yet emitted as a text token.
    text: String,
    queue: VecDeque<Token>,
    line: u32,
}

/// HTML's ASCII white space, as the tokenizer's states list it.
fn is_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{c}' | ' ')
}

impl Default for Tokenizer {
    fn default() -> Tokenizer {
        Tokenizer {
            input: Vec::new(),
            pos: 0,
            points: Vec::new(),
            bound: None,
            after_cr: false,
            ended: false,
            finished: false,
            state: State::Data,
            return_state: State::Data,
            tag: Tag::default(),
            comment: String::new(),
            doctype: Doctype::default(),
            pi_target: String::new(),
            pi_data: String::new(),
            temp: String::new(),
            char_ref: 0,
            last_start_tag: None,
            cdata_allowed: false,
            text: String::new(),
            queue: VecDeque::new(),
            line: 1,
        }
    }
}

impl Tokenizer {
    /// A tokenizer over the whole of `source`.
    pub fn new(source: &str) -> Tokenizer {
        let mut tokenizer = Tokenizer::default();
        tokenizer.feed(source);
        tokenizer.end();
        tokenizer
    }

    /// A tokenizer waiting for its input.
    pub fn streaming() -> Tokenizer {
        Tokenizer::default()
    }

    /// Appends `text` to the input, preprocessed.
    pub fn feed(&mut self, text: &str) {
        if self.ended {
            return;
        }
        // What has been consumed is let go of now and then, so the
        // buffer holds what is left to read.
        if self.pos > 1 << 16 && self.pos * 2 > self.input.len() {
            self.input.drain(..self.pos);
            for point in self.points.iter_mut().chain(&mut self.bound) {
                *point = point.saturating_sub(self.pos);
            }
            self.pos = 0;
        }
        self.input.reserve(text.len());
        for c in text.chars() {
            match c {
                '\n' if self.after_cr => {}
                '\r' => self.input.push('\n'),
                c => self.input.push(c),
            }
            self.after_cr = c == '\r';
        }
    }

    /// Marks the end of the input.
    pub fn end(&mut self) {
        self.ended = true;
    }

    /// Defines an insertion point just before the next input character,
    /// the innermost (HTML: the parser stops after a script's end tag).
    pub fn push_insertion_point(&mut self) {
        self.points.push(self.pos);
    }

    /// Takes the innermost insertion point away, the one before it
    /// innermost again.
    pub fn pop_insertion_point(&mut self) {
        self.points.pop();
    }

    /// Whether an insertion point is defined.
    pub fn has_insertion_point(&self) -> bool {
        !self.points.is_empty()
    }

    /// Puts `text`, preprocessed, into the input just before the
    /// innermost insertion point, which then stands after it, as do the
    /// ones around it (`document.write`). Nothing without one.
    pub fn insert(&mut self, text: &str) {
        let Some(&at) = self.points.last() else {
            return;
        };
        let mut chars = Vec::with_capacity(text.len());
        let mut after_cr = false;
        for c in text.chars() {
            match c {
                '\n' if after_cr => {}
                '\r' => chars.push('\n'),
                c => chars.push(c),
            }
            after_cr = c == '\r';
        }
        let n = chars.len();
        self.input.splice(at..at, chars);
        for point in &mut self.points {
            if *point >= at {
                *point += n;
            }
        }
    }

    /// Reads no further than the innermost insertion point while `on`,
    /// as a write's text is parsed: the input ends there for now, and
    /// more is to come.
    pub fn bound_at_insertion_point(&mut self, on: bool) {
        self.bound = if on {
            self.points.last().copied()
        } else {
            None
        };
    }

    /// The input as far as it may be read now.
    fn available(&self) -> &[char] {
        match self.bound {
            Some(bound) => &self.input[..bound.min(self.input.len())],
            None => &self.input,
        }
    }

    /// Whether the end of what is read now is the end of the input.
    fn at_the_end(&self) -> bool {
        self.ended && self.bound.is_none()
    }

    /// Whether the end of the input has been tokenized.
    pub fn is_finished(&self) -> bool {
        self.finished && self.queue.is_empty()
    }

    /// The line the next input character is on, from 1.
    pub fn line(&self) -> u32 {
        self.line
    }

    /// Switches to `state`, as the tree builder does for an element's
    /// content (section 13.2.6).
    pub fn set_state(&mut self, state: TextState) {
        self.state = match state {
            TextState::Data => State::Data,
            TextState::Rcdata => State::Rcdata,
            TextState::Rawtext => State::Rawtext,
            TextState::ScriptData => State::ScriptData,
            TextState::Plaintext => State::Plaintext,
            TextState::CdataSection => State::CdataSection,
        };
    }

    /// Sets the name of the last start tag emitted, which an
    /// appropriate end tag matches; for tests.
    pub fn set_last_start_tag(&mut self, name: Option<&str>) {
        self.last_start_tag = name.map(str::to_owned);
    }

    /// Whether `<![CDATA[` opens a CDATA section: when the adjusted
    /// current node is not an HTML element.
    pub fn set_cdata_allowed(&mut self, allowed: bool) {
        self.cdata_allowed = allowed;
    }

    /// The next token, or `None` when none can be made from the input so
    /// far (more may come) or the input is finished.
    pub fn next_token(&mut self) -> Option<Token> {
        loop {
            if let Some(token) = self.queue.pop_front() {
                return Some(token);
            }
            if self.finished {
                return None;
            }
            if let Flow::Wait = self.step() {
                if self.text.is_empty() {
                    return None;
                }
                return Some(Token::Text(std::mem::take(&mut self.text)));
            }
        }
    }

    fn next(&mut self) -> Next {
        match self.available().get(self.pos) {
            Some(&c) => {
                self.pos += 1;
                if c == '\n' {
                    self.line += 1;
                }
                Next::Char(c)
            }
            None if self.at_the_end() => Next::Eof,
            None => Next::Wait,
        }
    }

    /// Gives the character just consumed back, to be consumed again in
    /// `state`.
    fn reconsume(&mut self, c: char, state: State) {
        self.pos -= 1;
        if c == '\n' {
            self.line -= 1;
        }
        self.state = state;
    }

    /// Consumes the characters from here that `ordinary` takes,
    /// returning them.
    fn run(&mut self, ordinary: impl Fn(char) -> bool) -> Option<std::ops::Range<usize>> {
        let start = self.pos;
        let end = self.available().len();
        while self.pos < end {
            let c = self.input[self.pos];
            if !ordinary(c) {
                break;
            }
            if c == '\n' {
                self.line += 1;
            }
            self.pos += 1;
        }
        (self.pos > start).then_some(start..self.pos)
    }

    /// Emits the ordinary characters from here as text.
    fn text_run(&mut self, ordinary: impl Fn(char) -> bool) -> bool {
        match self.run(ordinary) {
            Some(range) => {
                self.text.extend(&self.input[range]);
                true
            }
            None => false,
        }
    }

    fn emit(&mut self, token: Token) {
        if !self.text.is_empty() {
            self.queue
                .push_back(Token::Text(std::mem::take(&mut self.text)));
        }
        self.queue.push_back(token);
    }

    fn emit_eof(&mut self) {
        if !self.text.is_empty() {
            self.queue
                .push_back(Token::Text(std::mem::take(&mut self.text)));
        }
        self.finished = true;
    }

    fn emit_tag(&mut self) {
        let mut tag = std::mem::take(&mut self.tag);
        tag.commit();
        let token = if tag.end {
            Token::EndTag(tag.name)
        } else {
            self.last_start_tag = Some(tag.name.clone());
            Token::StartTag {
                name: tag.name,
                attrs: tag.attrs,
                self_closing: tag.self_closing,
            }
        };
        self.emit(token);
    }

    fn emit_comment(&mut self) {
        let comment = std::mem::take(&mut self.comment);
        self.emit(Token::Comment(comment));
    }

    fn emit_doctype(&mut self) {
        let doctype = std::mem::take(&mut self.doctype);
        self.emit(Token::Doctype(doctype));
    }

    fn emit_pi(&mut self) {
        let target = std::mem::take(&mut self.pi_target);
        let data = std::mem::take(&mut self.pi_data);
        self.emit(Token::ProcessingInstruction { target, data });
    }

    /// Whether the end tag being built is an appropriate one: named as
    /// the last start tag emitted.
    fn appropriate(&self) -> bool {
        self.last_start_tag.as_deref() == Some(self.tag.name.as_str())
    }

    fn in_attribute(&self) -> bool {
        matches!(
            self.return_state,
            State::AttributeValueDoubleQuoted
                | State::AttributeValueSingleQuoted
                | State::AttributeValueUnquoted
        )
    }

    /// Flushes the temporary buffer as a character reference's code
    /// points: into the attribute's value, or emitted.
    fn flush_reference(&mut self) {
        let buffer = std::mem::take(&mut self.temp);
        if self.in_attribute() {
            self.tag.attr_value().push_str(&buffer);
        } else {
            self.text.push_str(&buffer);
        }
    }

    /// Whether the input from here starts with `pattern`, ASCII case
    /// folded if `fold`: `Some` once it is known, `None` while the input
    /// so far is too short to tell.
    fn starts_with(&self, pattern: &str, fold: bool) -> Option<bool> {
        let rest = &self.available()[self.pos..];
        let same = |a: char, b: char| {
            if fold {
                a.eq_ignore_ascii_case(&b)
            } else {
                a == b
            }
        };
        let mut length = 0;
        for (i, p) in pattern.chars().enumerate() {
            match rest.get(i) {
                Some(&c) if same(c, p) => length += 1,
                Some(_) => return Some(false),
                None if self.at_the_end() => return Some(false),
                None => return None,
            }
        }
        Some(length == pattern.chars().count())
    }

    /// Consumes `n` characters, none a line feed.
    fn skip(&mut self, n: usize) {
        self.pos += n;
    }

    /// One step of the state machine.
    fn step(&mut self) -> Flow {
        use State as S;
        match self.state {
            S::Data => {
                if self.text_run(|c| !matches!(c, '&' | '<' | '\0')) {
                    return Flow::Go;
                }
                match self.next() {
                    Next::Char('&') => {
                        self.return_state = S::Data;
                        self.state = S::CharacterReference;
                    }
                    Next::Char('<') => self.state = S::TagOpen,
                    Next::Char(c) => self.text.push(c),
                    Next::Eof => self.emit_eof(),
                    Next::Wait => return Flow::Wait,
                }
            }
            S::Rcdata => {
                if self.text_run(|c| !matches!(c, '&' | '<' | '\0')) {
                    return Flow::Go;
                }
                match self.next() {
                    Next::Char('&') => {
                        self.return_state = S::Rcdata;
                        self.state = S::CharacterReference;
                    }
                    Next::Char('<') => self.state = S::RcdataLessThan,
                    Next::Char('\0') => self.text.push('\u{fffd}'),
                    Next::Char(c) => self.text.push(c),
                    Next::Eof => self.emit_eof(),
                    Next::Wait => return Flow::Wait,
                }
            }
            S::Rawtext | S::ScriptData | S::Plaintext => {
                let state = self.state;
                let stop = state != S::Plaintext;
                if self.text_run(|c| !(c == '\0' || (stop && c == '<'))) {
                    return Flow::Go;
                }
                match self.next() {
                    Next::Char('<') if state == S::Rawtext => self.state = S::RawtextLessThan,
                    Next::Char('<') if state == S::ScriptData => self.state = S::ScriptDataLessThan,
                    Next::Char('\0') => self.text.push('\u{fffd}'),
                    Next::Char(c) => self.text.push(c),
                    Next::Eof => self.emit_eof(),
                    Next::Wait => return Flow::Wait,
                }
            }
            S::TagOpen => match self.next() {
                Next::Char('!') => self.state = S::MarkupDeclarationOpen,
                Next::Char('/') => self.state = S::EndTagOpen,
                Next::Char(c) if c.is_ascii_alphabetic() => {
                    self.tag = Tag::new(false);
                    self.reconsume(c, S::TagName);
                }
                Next::Char('?') => {
                    self.temp.clear();
                    self.state = S::ProcessingInstructionOpen;
                }
                Next::Char(c) => {
                    self.text.push('<');
                    self.reconsume(c, S::Data);
                }
                Next::Eof => {
                    self.text.push('<');
                    self.emit_eof();
                }
                Next::Wait => return Flow::Wait,
            },
            S::EndTagOpen => match self.next() {
                Next::Char(c) if c.is_ascii_alphabetic() => {
                    self.tag = Tag::new(true);
                    self.reconsume(c, S::TagName);
                }
                Next::Char('>') => self.state = S::Data,
                Next::Char(c) => {
                    self.comment.clear();
                    self.reconsume(c, S::BogusComment);
                }
                Next::Eof => {
                    self.text.push_str("</");
                    self.emit_eof();
                }
                Next::Wait => return Flow::Wait,
            },
            S::TagName => {
                if let Some(range) = self.run(|c| {
                    !(is_space(c) || matches!(c, '/' | '>' | '\0') || c.is_ascii_uppercase())
                }) {
                    let run: String = self.input[range].iter().collect();
                    self.tag.name.push_str(&run);
                    return Flow::Go;
                }
                match self.next() {
                    Next::Char(c) if is_space(c) => self.state = S::BeforeAttributeName,
                    Next::Char('/') => self.state = S::SelfClosingStartTag,
                    Next::Char('>') => {
                        self.state = S::Data;
                        self.emit_tag();
                    }
                    Next::Char('\0') => self.tag.name.push('\u{fffd}'),
                    Next::Char(c) => self.tag.name.push(c.to_ascii_lowercase()),
                    Next::Eof => self.emit_eof(),
                    Next::Wait => return Flow::Wait,
                }
            }
            S::RcdataLessThan | S::RawtextLessThan => {
                let back = if self.state == S::RcdataLessThan {
                    (S::RcdataEndTagOpen, S::Rcdata)
                } else {
                    (S::RawtextEndTagOpen, S::Rawtext)
                };
                match self.next() {
                    Next::Char('/') => {
                        self.temp.clear();
                        self.state = back.0;
                    }
                    Next::Char(c) => {
                        self.text.push('<');
                        self.reconsume(c, back.1);
                    }
                    Next::Eof => {
                        self.text.push('<');
                        self.state = back.1;
                    }
                    Next::Wait => return Flow::Wait,
                }
            }
            S::RcdataEndTagOpen
            | S::RawtextEndTagOpen
            | S::ScriptDataEndTagOpen
            | S::ScriptDataEscapedEndTagOpen => {
                let (name_state, back) = match self.state {
                    S::RcdataEndTagOpen => (S::RcdataEndTagName, S::Rcdata),
                    S::RawtextEndTagOpen => (S::RawtextEndTagName, S::Rawtext),
                    S::ScriptDataEndTagOpen => (S::ScriptDataEndTagName, S::ScriptData),
                    _ => (S::ScriptDataEscapedEndTagName, S::ScriptDataEscaped),
                };
                match self.next() {
                    Next::Char(c) if c.is_ascii_alphabetic() => {
                        self.tag = Tag::new(true);
                        self.reconsume(c, name_state);
                    }
                    Next::Char(c) => {
                        self.text.push_str("</");
                        self.reconsume(c, back);
                    }
                    Next::Eof => {
                        self.text.push_str("</");
                        self.state = back;
                    }
                    Next::Wait => return Flow::Wait,
                }
            }
            S::RcdataEndTagName
            | S::RawtextEndTagName
            | S::ScriptDataEndTagName
            | S::ScriptDataEscapedEndTagName => {
                let back = match self.state {
                    S::RcdataEndTagName => S::Rcdata,
                    S::RawtextEndTagName => S::Rawtext,
                    S::ScriptDataEndTagName => S::ScriptData,
                    _ => S::ScriptDataEscaped,
                };
                let next = self.next();
                match next {
                    Next::Char(c) if is_space(c) && self.appropriate() => {
                        self.state = S::BeforeAttributeName;
                        return Flow::Go;
                    }
                    Next::Char('/') if self.appropriate() => {
                        self.state = S::SelfClosingStartTag;
                        return Flow::Go;
                    }
                    Next::Char('>') if self.appropriate() => {
                        self.state = S::Data;
                        self.emit_tag();
                        return Flow::Go;
                    }
                    Next::Char(c) if c.is_ascii_alphabetic() => {
                        self.tag.name.push(c.to_ascii_lowercase());
                        self.temp.push(c);
                        return Flow::Go;
                    }
                    Next::Wait => return Flow::Wait,
                    _ => {}
                }
                self.text.push_str("</");
                let buffer = std::mem::take(&mut self.temp);
                self.text.push_str(&buffer);
                match next {
                    Next::Char(c) => self.reconsume(c, back),
                    _ => self.state = back,
                }
            }
            S::ScriptDataLessThan => match self.next() {
                Next::Char('/') => {
                    self.temp.clear();
                    self.state = S::ScriptDataEndTagOpen;
                }
                Next::Char('!') => {
                    self.text.push_str("<!");
                    self.state = S::ScriptDataEscapeStart;
                }
                Next::Char(c) => {
                    self.text.push('<');
                    self.reconsume(c, S::ScriptData);
                }
                Next::Eof => {
                    self.text.push('<');
                    self.state = S::ScriptData;
                }
                Next::Wait => return Flow::Wait,
            },
            S::ScriptDataEscapeStart | S::ScriptDataEscapeStartDash => {
                let on = if self.state == S::ScriptDataEscapeStart {
                    S::ScriptDataEscapeStartDash
                } else {
                    S::ScriptDataEscapedDashDash
                };
                match self.next() {
                    Next::Char('-') => {
                        self.text.push('-');
                        self.state = on;
                    }
                    Next::Char(c) => self.reconsume(c, S::ScriptData),
                    Next::Eof => self.state = S::ScriptData,
                    Next::Wait => return Flow::Wait,
                }
            }
            S::ScriptDataEscaped => {
                if self.text_run(|c| !matches!(c, '-' | '<' | '\0')) {
                    return Flow::Go;
                }
                match self.next() {
                    Next::Char('-') => {
                        self.text.push('-');
                        self.state = S::ScriptDataEscapedDash;
                    }
                    Next::Char('<') => self.state = S::ScriptDataEscapedLessThan,
                    Next::Char('\0') => self.text.push('\u{fffd}'),
                    Next::Char(c) => self.text.push(c),
                    Next::Eof => self.emit_eof(),
                    Next::Wait => return Flow::Wait,
                }
            }
            S::ScriptDataEscapedDash | S::ScriptDataEscapedDashDash => {
                let dash_dash = self.state == S::ScriptDataEscapedDashDash;
                match self.next() {
                    Next::Char('-') => {
                        self.text.push('-');
                        self.state = S::ScriptDataEscapedDashDash;
                    }
                    Next::Char('<') => self.state = S::ScriptDataEscapedLessThan,
                    Next::Char('>') if dash_dash => {
                        self.text.push('>');
                        self.state = S::ScriptData;
                    }
                    Next::Char('\0') => {
                        self.text.push('\u{fffd}');
                        self.state = S::ScriptDataEscaped;
                    }
                    Next::Char(c) => {
                        self.text.push(c);
                        self.state = S::ScriptDataEscaped;
                    }
                    Next::Eof => self.emit_eof(),
                    Next::Wait => return Flow::Wait,
                }
            }
            S::ScriptDataEscapedLessThan => match self.next() {
                Next::Char('/') => {
                    self.temp.clear();
                    self.state = S::ScriptDataEscapedEndTagOpen;
                }
                Next::Char(c) if c.is_ascii_alphabetic() => {
                    self.temp.clear();
                    self.text.push('<');
                    self.reconsume(c, S::ScriptDataDoubleEscapeStart);
                }
                Next::Char(c) => {
                    self.text.push('<');
                    self.reconsume(c, S::ScriptDataEscaped);
                }
                Next::Eof => {
                    self.text.push('<');
                    self.state = S::ScriptDataEscaped;
                }
                Next::Wait => return Flow::Wait,
            },
            S::ScriptDataDoubleEscapeStart | S::ScriptDataDoubleEscapeEnd => {
                let starting = self.state == S::ScriptDataDoubleEscapeStart;
                let (on_script, otherwise) = if starting {
                    (S::ScriptDataDoubleEscaped, S::ScriptDataEscaped)
                } else {
                    (S::ScriptDataEscaped, S::ScriptDataDoubleEscaped)
                };
                match self.next() {
                    Next::Char(c) if is_space(c) || c == '/' || c == '>' => {
                        self.state = if self.temp == "script" {
                            on_script
                        } else {
                            otherwise
                        };
                        self.text.push(c);
                    }
                    Next::Char(c) if c.is_ascii_alphabetic() => {
                        self.temp.push(c.to_ascii_lowercase());
                        self.text.push(c);
                    }
                    Next::Char(c) => {
                        let back = if starting {
                            S::ScriptDataEscaped
                        } else {
                            S::ScriptDataDoubleEscaped
                        };
                        self.reconsume(c, back);
                    }
                    Next::Eof => {
                        self.state = if starting {
                            S::ScriptDataEscaped
                        } else {
                            S::ScriptDataDoubleEscaped
                        };
                    }
                    Next::Wait => return Flow::Wait,
                }
            }
            S::ScriptDataDoubleEscaped => {
                if self.text_run(|c| !matches!(c, '-' | '<' | '\0')) {
                    return Flow::Go;
                }
                match self.next() {
                    Next::Char('-') => {
                        self.text.push('-');
                        self.state = S::ScriptDataDoubleEscapedDash;
                    }
                    Next::Char('<') => {
                        self.text.push('<');
                        self.state = S::ScriptDataDoubleEscapedLessThan;
                    }
                    Next::Char('\0') => self.text.push('\u{fffd}'),
                    Next::Char(c) => self.text.push(c),
                    Next::Eof => self.emit_eof(),
                    Next::Wait => return Flow::Wait,
                }
            }
            S::ScriptDataDoubleEscapedDash | S::ScriptDataDoubleEscapedDashDash => {
                let dash_dash = self.state == S::ScriptDataDoubleEscapedDashDash;
                match self.next() {
                    Next::Char('-') => {
                        self.text.push('-');
                        self.state = S::ScriptDataDoubleEscapedDashDash;
                    }
                    Next::Char('<') => {
                        self.text.push('<');
                        self.state = S::ScriptDataDoubleEscapedLessThan;
                    }
                    Next::Char('>') if dash_dash => {
                        self.text.push('>');
                        self.state = S::ScriptData;
                    }
                    Next::Char('\0') => {
                        self.text.push('\u{fffd}');
                        self.state = S::ScriptDataDoubleEscaped;
                    }
                    Next::Char(c) => {
                        self.text.push(c);
                        self.state = S::ScriptDataDoubleEscaped;
                    }
                    Next::Eof => self.emit_eof(),
                    Next::Wait => return Flow::Wait,
                }
            }
            S::ScriptDataDoubleEscapedLessThan => match self.next() {
                Next::Char('/') => {
                    self.temp.clear();
                    self.text.push('/');
                    self.state = S::ScriptDataDoubleEscapeEnd;
                }
                Next::Char(c) => self.reconsume(c, S::ScriptDataDoubleEscaped),
                Next::Eof => self.state = S::ScriptDataDoubleEscaped,
                Next::Wait => return Flow::Wait,
            },
            S::BeforeAttributeName => match self.next() {
                Next::Char(c) if is_space(c) => {}
                Next::Char(c @ ('/' | '>')) => self.reconsume(c, S::AfterAttributeName),
                Next::Char('=') => {
                    self.tag.start_attr("=".to_owned());
                    self.state = S::AttributeName;
                }
                Next::Char(c) => {
                    self.tag.start_attr(String::new());
                    self.reconsume(c, S::AttributeName);
                }
                Next::Eof => self.state = S::AfterAttributeName,
                Next::Wait => return Flow::Wait,
            },
            S::AttributeName => {
                if let Some(range) = self.run(|c| {
                    !(is_space(c) || matches!(c, '/' | '>' | '=' | '\0') || c.is_ascii_uppercase())
                }) {
                    let run: String = self.input[range].iter().collect();
                    self.tag.attr_name().push_str(&run);
                    return Flow::Go;
                }
                match self.next() {
                    Next::Char(c) if is_space(c) || c == '/' || c == '>' => {
                        self.reconsume(c, S::AfterAttributeName)
                    }
                    Next::Char('=') => self.state = S::BeforeAttributeValue,
                    Next::Char('\0') => self.tag.attr_name().push('\u{fffd}'),
                    Next::Char(c) => self.tag.attr_name().push(c.to_ascii_lowercase()),
                    Next::Eof => self.state = S::AfterAttributeName,
                    Next::Wait => return Flow::Wait,
                }
            }
            S::AfterAttributeName => match self.next() {
                Next::Char(c) if is_space(c) => {}
                Next::Char('/') => self.state = S::SelfClosingStartTag,
                Next::Char('=') => self.state = S::BeforeAttributeValue,
                Next::Char('>') => {
                    self.state = S::Data;
                    self.emit_tag();
                }
                Next::Char(c) => {
                    self.tag.start_attr(String::new());
                    self.reconsume(c, S::AttributeName);
                }
                Next::Eof => self.emit_eof(),
                Next::Wait => return Flow::Wait,
            },
            S::BeforeAttributeValue => match self.next() {
                Next::Char(c) if is_space(c) => {}
                Next::Char('"') => self.state = S::AttributeValueDoubleQuoted,
                Next::Char('\'') => self.state = S::AttributeValueSingleQuoted,
                Next::Char('>') => {
                    self.state = S::Data;
                    self.emit_tag();
                }
                Next::Char(c) => self.reconsume(c, S::AttributeValueUnquoted),
                Next::Eof => self.state = S::AttributeValueUnquoted,
                Next::Wait => return Flow::Wait,
            },
            S::AttributeValueDoubleQuoted | S::AttributeValueSingleQuoted => {
                let quote = if self.state == S::AttributeValueDoubleQuoted {
                    '"'
                } else {
                    '\''
                };
                if let Some(range) = self.run(|c| !(c == quote || c == '&' || c == '\0')) {
                    let run: String = self.input[range].iter().collect();
                    self.tag.attr_value().push_str(&run);
                    return Flow::Go;
                }
                match self.next() {
                    Next::Char(c) if c == quote => self.state = S::AfterAttributeValueQuoted,
                    Next::Char('&') => {
                        self.return_state = self.state;
                        self.state = S::CharacterReference;
                    }
                    Next::Char('\0') => self.tag.attr_value().push('\u{fffd}'),
                    Next::Char(c) => self.tag.attr_value().push(c),
                    Next::Eof => self.emit_eof(),
                    Next::Wait => return Flow::Wait,
                }
            }
            S::AttributeValueUnquoted => {
                if let Some(range) = self.run(|c| !(is_space(c) || matches!(c, '&' | '>' | '\0'))) {
                    let run: String = self.input[range].iter().collect();
                    self.tag.attr_value().push_str(&run);
                    return Flow::Go;
                }
                match self.next() {
                    Next::Char(c) if is_space(c) => self.state = S::BeforeAttributeName,
                    Next::Char('&') => {
                        self.return_state = S::AttributeValueUnquoted;
                        self.state = S::CharacterReference;
                    }
                    Next::Char('>') => {
                        self.state = S::Data;
                        self.emit_tag();
                    }
                    Next::Char('\0') => self.tag.attr_value().push('\u{fffd}'),
                    Next::Char(c) => self.tag.attr_value().push(c),
                    Next::Eof => self.emit_eof(),
                    Next::Wait => return Flow::Wait,
                }
            }
            S::AfterAttributeValueQuoted => match self.next() {
                Next::Char(c) if is_space(c) => self.state = S::BeforeAttributeName,
                Next::Char('/') => self.state = S::SelfClosingStartTag,
                Next::Char('>') => {
                    self.state = S::Data;
                    self.emit_tag();
                }
                Next::Char(c) => self.reconsume(c, S::BeforeAttributeName),
                Next::Eof => self.emit_eof(),
                Next::Wait => return Flow::Wait,
            },
            S::SelfClosingStartTag => match self.next() {
                Next::Char('>') => {
                    self.tag.self_closing = true;
                    self.state = S::Data;
                    self.emit_tag();
                }
                Next::Char(c) => self.reconsume(c, S::BeforeAttributeName),
                Next::Eof => self.emit_eof(),
                Next::Wait => return Flow::Wait,
            },
            S::BogusComment => {
                if let Some(range) = self.run(|c| !matches!(c, '>' | '\0')) {
                    let run: String = self.input[range].iter().collect();
                    self.comment.push_str(&run);
                    return Flow::Go;
                }
                match self.next() {
                    Next::Char('>') => {
                        self.state = S::Data;
                        self.emit_comment();
                    }
                    Next::Char('\0') => self.comment.push('\u{fffd}'),
                    Next::Char(c) => self.comment.push(c),
                    Next::Eof => {
                        self.emit_comment();
                        self.emit_eof();
                    }
                    Next::Wait => return Flow::Wait,
                }
            }
            S::MarkupDeclarationOpen => {
                let checks = [
                    self.starts_with("--", false),
                    self.starts_with("doctype", true),
                    self.starts_with("[CDATA[", false),
                ];
                if checks.contains(&None) && !checks.contains(&Some(true)) {
                    return Flow::Wait;
                }
                if checks[0] == Some(true) {
                    self.skip(2);
                    self.comment.clear();
                    self.state = S::CommentStart;
                } else if checks[1] == Some(true) {
                    self.skip(7);
                    self.state = S::Doctype;
                } else if checks[2] == Some(true) {
                    self.skip(7);
                    if self.cdata_allowed {
                        self.state = S::CdataSection;
                    } else {
                        self.comment = "[CDATA[".to_owned();
                        self.state = S::BogusComment;
                    }
                } else {
                    self.comment.clear();
                    self.state = S::BogusComment;
                }
            }
            S::CommentStart => match self.next() {
                Next::Char('-') => self.state = S::CommentStartDash,
                Next::Char('>') => {
                    self.state = S::Data;
                    self.emit_comment();
                }
                Next::Char(c) => self.reconsume(c, S::Comment),
                Next::Eof => self.state = S::Comment,
                Next::Wait => return Flow::Wait,
            },
            S::CommentStartDash => match self.next() {
                Next::Char('-') => self.state = S::CommentEnd,
                Next::Char('>') => {
                    self.state = S::Data;
                    self.emit_comment();
                }
                Next::Char(c) => {
                    self.comment.push('-');
                    self.reconsume(c, S::Comment);
                }
                Next::Eof => {
                    self.emit_comment();
                    self.emit_eof();
                }
                Next::Wait => return Flow::Wait,
            },
            S::Comment => {
                if let Some(range) = self.run(|c| !matches!(c, '<' | '-' | '\0')) {
                    let run: String = self.input[range].iter().collect();
                    self.comment.push_str(&run);
                    return Flow::Go;
                }
                match self.next() {
                    Next::Char('<') => {
                        self.comment.push('<');
                        self.state = S::CommentLessThan;
                    }
                    Next::Char('-') => self.state = S::CommentEndDash,
                    Next::Char('\0') => self.comment.push('\u{fffd}'),
                    Next::Char(c) => self.comment.push(c),
                    Next::Eof => {
                        self.emit_comment();
                        self.emit_eof();
                    }
                    Next::Wait => return Flow::Wait,
                }
            }
            S::CommentLessThan => match self.next() {
                Next::Char('!') => {
                    self.comment.push('!');
                    self.state = S::CommentLessThanBang;
                }
                Next::Char('<') => self.comment.push('<'),
                Next::Char(c) => self.reconsume(c, S::Comment),
                Next::Eof => self.state = S::Comment,
                Next::Wait => return Flow::Wait,
            },
            S::CommentLessThanBang => match self.next() {
                Next::Char('-') => self.state = S::CommentLessThanBangDash,
                Next::Char(c) => self.reconsume(c, S::Comment),
                Next::Eof => self.state = S::Comment,
                Next::Wait => return Flow::Wait,
            },
            S::CommentLessThanBangDash => match self.next() {
                Next::Char('-') => self.state = S::CommentLessThanBangDashDash,
                Next::Char(c) => self.reconsume(c, S::CommentEndDash),
                Next::Eof => self.state = S::CommentEndDash,
                Next::Wait => return Flow::Wait,
            },
            S::CommentLessThanBangDashDash => match self.next() {
                Next::Char(c) => self.reconsume(c, S::CommentEnd),
                Next::Eof => self.state = S::CommentEnd,
                Next::Wait => return Flow::Wait,
            },
            S::CommentEndDash => match self.next() {
                Next::Char('-') => self.state = S::CommentEnd,
                Next::Char(c) => {
                    self.comment.push('-');
                    self.reconsume(c, S::Comment);
                }
                Next::Eof => {
                    self.emit_comment();
                    self.emit_eof();
                }
                Next::Wait => return Flow::Wait,
            },
            S::CommentEnd => match self.next() {
                Next::Char('>') => {
                    self.state = S::Data;
                    self.emit_comment();
                }
                Next::Char('!') => self.state = S::CommentEndBang,
                Next::Char('-') => self.comment.push('-'),
                Next::Char(c) => {
                    self.comment.push_str("--");
                    self.reconsume(c, S::Comment);
                }
                Next::Eof => {
                    self.emit_comment();
                    self.emit_eof();
                }
                Next::Wait => return Flow::Wait,
            },
            S::CommentEndBang => match self.next() {
                Next::Char('-') => {
                    self.comment.push_str("--!");
                    self.state = S::CommentEndDash;
                }
                Next::Char('>') => {
                    self.state = S::Data;
                    self.emit_comment();
                }
                Next::Char(c) => {
                    self.comment.push_str("--!");
                    self.reconsume(c, S::Comment);
                }
                Next::Eof => {
                    self.emit_comment();
                    self.emit_eof();
                }
                Next::Wait => return Flow::Wait,
            },
            S::Doctype => match self.next() {
                Next::Char(c) if is_space(c) => self.state = S::BeforeDoctypeName,
                Next::Char(c) => self.reconsume(c, S::BeforeDoctypeName),
                Next::Eof => {
                    self.doctype = Doctype {
                        force_quirks: true,
                        ..Doctype::default()
                    };
                    self.emit_doctype();
                    self.emit_eof();
                }
                Next::Wait => return Flow::Wait,
            },
            S::BeforeDoctypeName => match self.next() {
                Next::Char(c) if is_space(c) => {}
                Next::Char('>') => {
                    self.doctype = Doctype {
                        force_quirks: true,
                        ..Doctype::default()
                    };
                    self.state = S::Data;
                    self.emit_doctype();
                }
                Next::Char(c) => {
                    let first = if c == '\0' {
                        '\u{fffd}'
                    } else {
                        c.to_ascii_lowercase()
                    };
                    self.doctype = Doctype {
                        name: Some(first.to_string()),
                        ..Doctype::default()
                    };
                    self.state = S::DoctypeName;
                }
                Next::Eof => {
                    self.doctype = Doctype {
                        force_quirks: true,
                        ..Doctype::default()
                    };
                    self.emit_doctype();
                    self.emit_eof();
                }
                Next::Wait => return Flow::Wait,
            },
            S::DoctypeName => match self.next() {
                Next::Char(c) if is_space(c) => self.state = S::AfterDoctypeName,
                Next::Char('>') => {
                    self.state = S::Data;
                    self.emit_doctype();
                }
                Next::Char(c) => {
                    let c = if c == '\0' {
                        '\u{fffd}'
                    } else {
                        c.to_ascii_lowercase()
                    };
                    self.doctype.name.get_or_insert_with(String::new).push(c);
                }
                Next::Eof => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.emit_eof();
                }
                Next::Wait => return Flow::Wait,
            },
            S::AfterDoctypeName => {
                // The keywords are looked for from the current character,
                // so it is looked at before it is consumed.
                match self.available().get(self.pos) {
                    None if !self.at_the_end() => return Flow::Wait,
                    None => {
                        self.doctype.force_quirks = true;
                        self.emit_doctype();
                        self.emit_eof();
                        return Flow::Go;
                    }
                    Some(&c) if is_space(c) => {
                        self.next();
                        return Flow::Go;
                    }
                    Some('>') => {
                        self.next();
                        self.state = S::Data;
                        self.emit_doctype();
                        return Flow::Go;
                    }
                    Some(_) => {}
                }
                let public = self.starts_with("public", true);
                let system = self.starts_with("system", true);
                if public.is_none() || system.is_none() {
                    return Flow::Wait;
                }
                if public == Some(true) {
                    self.skip(6);
                    self.state = S::AfterDoctypePublicKeyword;
                } else if system == Some(true) {
                    self.skip(6);
                    self.state = S::AfterDoctypeSystemKeyword;
                } else {
                    self.doctype.force_quirks = true;
                    self.state = S::BogusDoctype;
                }
            }
            S::AfterDoctypePublicKeyword | S::AfterDoctypeSystemKeyword => {
                let public = self.state == S::AfterDoctypePublicKeyword;
                match self.next() {
                    Next::Char(c) if is_space(c) => {
                        self.state = if public {
                            S::BeforeDoctypePublicIdentifier
                        } else {
                            S::BeforeDoctypeSystemIdentifier
                        };
                    }
                    Next::Char(quote @ ('"' | '\'')) => self.open_identifier(public, quote),
                    Next::Char('>') => {
                        self.doctype.force_quirks = true;
                        self.state = S::Data;
                        self.emit_doctype();
                    }
                    Next::Char(c) => {
                        self.doctype.force_quirks = true;
                        self.reconsume(c, S::BogusDoctype);
                    }
                    Next::Eof => {
                        self.doctype.force_quirks = true;
                        self.emit_doctype();
                        self.emit_eof();
                    }
                    Next::Wait => return Flow::Wait,
                }
            }
            S::BeforeDoctypePublicIdentifier | S::BeforeDoctypeSystemIdentifier => {
                let public = self.state == S::BeforeDoctypePublicIdentifier;
                match self.next() {
                    Next::Char(c) if is_space(c) => {}
                    Next::Char(quote @ ('"' | '\'')) => self.open_identifier(public, quote),
                    Next::Char('>') => {
                        self.doctype.force_quirks = true;
                        self.state = S::Data;
                        self.emit_doctype();
                    }
                    Next::Char(c) => {
                        self.doctype.force_quirks = true;
                        self.reconsume(c, S::BogusDoctype);
                    }
                    Next::Eof => {
                        self.doctype.force_quirks = true;
                        self.emit_doctype();
                        self.emit_eof();
                    }
                    Next::Wait => return Flow::Wait,
                }
            }
            S::DoctypePublicIdentifierDoubleQuoted
            | S::DoctypePublicIdentifierSingleQuoted
            | S::DoctypeSystemIdentifierDoubleQuoted
            | S::DoctypeSystemIdentifierSingleQuoted => {
                let public = matches!(
                    self.state,
                    S::DoctypePublicIdentifierDoubleQuoted | S::DoctypePublicIdentifierSingleQuoted
                );
                let quote = if matches!(
                    self.state,
                    S::DoctypePublicIdentifierDoubleQuoted | S::DoctypeSystemIdentifierDoubleQuoted
                ) {
                    '"'
                } else {
                    '\''
                };
                match self.next() {
                    Next::Char(c) if c == quote => {
                        self.state = if public {
                            S::AfterDoctypePublicIdentifier
                        } else {
                            S::AfterDoctypeSystemIdentifier
                        };
                    }
                    Next::Char('>') => {
                        self.doctype.force_quirks = true;
                        self.state = S::Data;
                        self.emit_doctype();
                    }
                    Next::Char(c) => {
                        let c = if c == '\0' { '\u{fffd}' } else { c };
                        let id = if public {
                            &mut self.doctype.public_id
                        } else {
                            &mut self.doctype.system_id
                        };
                        id.get_or_insert_with(String::new).push(c);
                    }
                    Next::Eof => {
                        self.doctype.force_quirks = true;
                        self.emit_doctype();
                        self.emit_eof();
                    }
                    Next::Wait => return Flow::Wait,
                }
            }
            S::AfterDoctypePublicIdentifier | S::BetweenDoctypePublicAndSystemIdentifiers => {
                match self.next() {
                    Next::Char(c) if is_space(c) => {
                        self.state = S::BetweenDoctypePublicAndSystemIdentifiers;
                    }
                    Next::Char('>') => {
                        self.state = S::Data;
                        self.emit_doctype();
                    }
                    Next::Char(quote @ ('"' | '\'')) => self.open_identifier(false, quote),
                    Next::Char(c) => {
                        self.doctype.force_quirks = true;
                        self.reconsume(c, S::BogusDoctype);
                    }
                    Next::Eof => {
                        self.doctype.force_quirks = true;
                        self.emit_doctype();
                        self.emit_eof();
                    }
                    Next::Wait => return Flow::Wait,
                }
            }
            S::AfterDoctypeSystemIdentifier => match self.next() {
                Next::Char(c) if is_space(c) => {}
                Next::Char('>') => {
                    self.state = S::Data;
                    self.emit_doctype();
                }
                // Not force-quirks, unlike the other states' errors.
                Next::Char(c) => self.reconsume(c, S::BogusDoctype),
                Next::Eof => {
                    self.doctype.force_quirks = true;
                    self.emit_doctype();
                    self.emit_eof();
                }
                Next::Wait => return Flow::Wait,
            },
            S::BogusDoctype => match self.next() {
                Next::Char('>') => {
                    self.state = S::Data;
                    self.emit_doctype();
                }
                Next::Char(_) => {}
                Next::Eof => {
                    self.emit_doctype();
                    self.emit_eof();
                }
                Next::Wait => return Flow::Wait,
            },
            S::CdataSection => {
                if self.text_run(|c| c != ']') {
                    return Flow::Go;
                }
                match self.next() {
                    Next::Char(_) => self.state = S::CdataSectionBracket,
                    Next::Eof => self.emit_eof(),
                    Next::Wait => return Flow::Wait,
                }
            }
            S::CdataSectionBracket => match self.next() {
                Next::Char(']') => self.state = S::CdataSectionEnd,
                Next::Char(c) => {
                    self.text.push(']');
                    self.reconsume(c, S::CdataSection);
                }
                Next::Eof => {
                    self.text.push(']');
                    self.state = S::CdataSection;
                }
                Next::Wait => return Flow::Wait,
            },
            S::CdataSectionEnd => match self.next() {
                Next::Char(']') => self.text.push(']'),
                Next::Char('>') => self.state = S::Data,
                Next::Char(c) => {
                    self.text.push_str("]]");
                    self.reconsume(c, S::CdataSection);
                }
                Next::Eof => {
                    self.text.push_str("]]");
                    self.state = S::CdataSection;
                }
                Next::Wait => return Flow::Wait,
            },
            S::ProcessingInstructionOpen => match self.next() {
                Next::Char(c) if c.is_ascii_alphabetic() || c == '_' => {
                    self.reconsume(c, S::ProcessingInstructionTarget)
                }
                Next::Char(c) => {
                    self.comment = format!("?{}", self.temp);
                    self.reconsume(c, S::BogusComment);
                }
                Next::Eof => self.emit_eof(),
                Next::Wait => return Flow::Wait,
            },
            S::ProcessingInstructionTarget => match self.next() {
                Next::Char(c) if is_space(c) || c == '?' || c == '>' => {
                    if self.temp.eq_ignore_ascii_case("xml")
                        || self.temp.eq_ignore_ascii_case("xml-stylesheet")
                    {
                        self.comment = format!("?{}", self.temp);
                        self.reconsume(c, S::BogusComment);
                    } else {
                        self.pi_target = std::mem::take(&mut self.temp);
                        self.pi_data.clear();
                        self.reconsume(c, S::AfterProcessingInstructionTarget);
                    }
                }
                Next::Char(c) if c.is_ascii_alphanumeric() || c == '-' || c == '_' => {
                    self.temp.push(c)
                }
                Next::Char(c) => {
                    self.comment = format!("?{}", self.temp);
                    self.reconsume(c, S::BogusComment);
                }
                Next::Eof => self.emit_eof(),
                Next::Wait => return Flow::Wait,
            },
            S::AfterProcessingInstructionTarget => match self.next() {
                Next::Char(c) if is_space(c) => {}
                Next::Char(c) => self.reconsume(c, S::ProcessingInstructionData),
                Next::Eof => self.state = S::ProcessingInstructionData,
                Next::Wait => return Flow::Wait,
            },
            S::ProcessingInstructionData => {
                if let Some(range) = self.run(|c| !matches!(c, '?' | '>')) {
                    let run: String = self.input[range].iter().collect();
                    self.pi_data.push_str(&run);
                    return Flow::Go;
                }
                match self.next() {
                    Next::Char('?') => self.state = S::ProcessingInstructionQuestionable,
                    Next::Char('>') => {
                        self.state = S::Data;
                        self.emit_pi();
                    }
                    Next::Char(c) => self.pi_data.push(c),
                    Next::Eof => self.emit_eof(),
                    Next::Wait => return Flow::Wait,
                }
            }
            S::ProcessingInstructionQuestionable => match self.next() {
                Next::Char('>') => {
                    self.state = S::Data;
                    self.emit_pi();
                }
                Next::Char(c) => {
                    self.pi_data.push('?');
                    self.reconsume(c, S::ProcessingInstructionData);
                }
                Next::Eof => self.emit_eof(),
                Next::Wait => return Flow::Wait,
            },
            S::CharacterReference => {
                self.temp.clear();
                self.temp.push('&');
                match self.next() {
                    Next::Char(c) if c.is_ascii_alphanumeric() => {
                        self.reconsume(c, S::NamedCharacterReference)
                    }
                    Next::Char('#') => {
                        self.temp.push('#');
                        self.state = S::NumericCharacterReference;
                    }
                    Next::Char(c) => {
                        self.flush_reference();
                        self.reconsume(c, self.return_state);
                    }
                    Next::Eof => {
                        self.flush_reference();
                        self.state = self.return_state;
                    }
                    Next::Wait => return self.restart_reference(),
                }
            }
            S::NamedCharacterReference => return self.named_reference(),
            S::AmbiguousAmpersand => match self.next() {
                Next::Char(c) if c.is_ascii_alphanumeric() => {
                    if self.in_attribute() {
                        self.tag.attr_value().push(c);
                    } else {
                        self.text.push(c);
                    }
                }
                Next::Char(c) => self.reconsume(c, self.return_state),
                Next::Eof => self.state = self.return_state,
                Next::Wait => return Flow::Wait,
            },
            S::NumericCharacterReference => {
                self.char_ref = 0;
                match self.next() {
                    Next::Char(c @ ('x' | 'X')) => {
                        self.temp.push(c);
                        self.state = S::HexadecimalCharacterReferenceStart;
                    }
                    Next::Char(c) if c.is_ascii_digit() => {
                        self.reconsume(c, S::DecimalCharacterReference)
                    }
                    Next::Char(c) => {
                        self.flush_reference();
                        self.reconsume(c, self.return_state);
                    }
                    Next::Eof => {
                        self.flush_reference();
                        self.state = self.return_state;
                    }
                    Next::Wait => return self.restart_reference(),
                }
            }
            S::HexadecimalCharacterReferenceStart => match self.next() {
                Next::Char(c) if c.is_ascii_hexdigit() => {
                    self.reconsume(c, S::HexadecimalCharacterReference)
                }
                Next::Char(c) => {
                    self.flush_reference();
                    self.reconsume(c, self.return_state);
                }
                Next::Eof => {
                    self.flush_reference();
                    self.state = self.return_state;
                }
                Next::Wait => return self.restart_reference(),
            },
            S::HexadecimalCharacterReference | S::DecimalCharacterReference => {
                let radix = if self.state == S::HexadecimalCharacterReference {
                    16
                } else {
                    10
                };
                match self.next() {
                    Next::Char(c) if c.is_digit(radix) => {
                        let digit = c.to_digit(radix).unwrap_or(0);
                        // Past the last code point the value only needs
                        // to stay past it.
                        self.char_ref = self
                            .char_ref
                            .saturating_mul(radix)
                            .saturating_add(digit)
                            .min(0x11_0000);
                    }
                    Next::Char(';') => self.state = S::NumericCharacterReferenceEnd,
                    Next::Char(c) => self.reconsume(c, S::NumericCharacterReferenceEnd),
                    Next::Eof => self.state = S::NumericCharacterReferenceEnd,
                    Next::Wait => return Flow::Wait,
                }
            }
            S::NumericCharacterReferenceEnd => {
                let c = numeric_reference(self.char_ref);
                self.temp.clear();
                self.temp.push(c);
                self.flush_reference();
                self.state = self.return_state;
            }
        }
        Flow::Go
    }

    /// The input ran out before a character reference's start could be
    /// told: what it has read, all in the temporary buffer and none a
    /// line feed, is given back, and it is read again from its `&` once
    /// there is more.
    fn restart_reference(&mut self) -> Flow {
        self.pos -= self.temp.chars().count();
        self.temp.clear();
        self.state = self.return_state;
        Flow::Wait
    }

    /// Opens a doctype's public or system identifier quoted by `quote`.
    fn open_identifier(&mut self, public: bool, quote: char) {
        use State as S;
        let double = quote == '"';
        if public {
            self.doctype.public_id = Some(String::new());
            self.state = if double {
                S::DoctypePublicIdentifierDoubleQuoted
            } else {
                S::DoctypePublicIdentifierSingleQuoted
            };
        } else {
            self.doctype.system_id = Some(String::new());
            self.state = if double {
                S::DoctypeSystemIdentifierDoubleQuoted
            } else {
                S::DoctypeSystemIdentifierSingleQuoted
            };
        }
    }

    /// The named character reference state (section 13.2.5.78): the
    /// longest name in the table that the input starts with.
    fn named_reference(&mut self) -> Flow {
        let rest = &self.available()[self.pos..];
        // The candidates: a run of ASCII alphanumerics, and the
        // semicolon after it.
        let mut length = rest
            .iter()
            .take(LONGEST_NAME)
            .take_while(|c| c.is_ascii_alphanumeric())
            .count();
        let complete = length == LONGEST_NAME || length < rest.len() || self.at_the_end();
        if !complete {
            // A longer name may come.
            return self.restart_reference();
        }
        if rest.get(length) == Some(&';') {
            length += 1;
        }
        let candidate: String = rest[..length].iter().collect();
        let found = (1..=candidate.len()).rev().find_map(|n| {
            let name = &candidate[..n];
            NAMED_REFERENCES
                .binary_search_by(|(entry, _)| (*entry).cmp(name))
                .ok()
                .map(|i| (n, NAMED_REFERENCES[i].1))
        });
        let Some((matched, value)) = found else {
            self.flush_reference();
            self.state = State::AmbiguousAmpersand;
            return Flow::Go;
        };
        self.skip(matched);
        let ends_with_semicolon = candidate[..matched].ends_with(';');
        let following = self.available().get(self.pos).copied();
        if self.in_attribute()
            && !ends_with_semicolon
            && following.is_some_and(|c| c == '=' || c.is_ascii_alphanumeric())
        {
            self.temp.push_str(&candidate[..matched]);
        } else {
            self.temp.clear();
            self.temp.push_str(value);
        }
        self.flush_reference();
        self.state = self.return_state;
        Flow::Go
    }
}

/// The character a numeric reference's `code` stands for (section
/// 13.2.5.84): U+FFFD for zero, and for a surrogate and past the last
/// code point, which are no `char`; C1 controls read as Windows-1252, as
/// the table says.
fn numeric_reference(code: u32) -> char {
    match code {
        0 => '\u{fffd}',
        0x80 => '\u{20ac}',
        0x82 => '\u{201a}',
        0x83 => '\u{0192}',
        0x84 => '\u{201e}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02c6}',
        0x89 => '\u{2030}',
        0x8a => '\u{0160}',
        0x8b => '\u{2039}',
        0x8c => '\u{0152}',
        0x8e => '\u{017d}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201c}',
        0x94 => '\u{201d}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02dc}',
        0x99 => '\u{2122}',
        0x9a => '\u{0161}',
        0x9b => '\u{203a}',
        0x9c => '\u{0153}',
        0x9e => '\u{017e}',
        0x9f => '\u{0178}',
        code => char::from_u32(code).unwrap_or('\u{fffd}'),
    }
}

/// The state an element's content starts in when the tree builder has
/// no say: what section 13.2.6 gives each element whose content is
/// text, in HTML content, with scripting disabled.
pub fn content_state(name: &str) -> Option<TextState> {
    match name {
        "title" | "textarea" => Some(TextState::Rcdata),
        "style" | "xmp" | "iframe" | "noembed" | "noframes" => Some(TextState::Rawtext),
        "script" => Some(TextState::ScriptData),
        "plaintext" => Some(TextState::Plaintext),
        _ => None,
    }
}

/// Tokenizes the whole of `source`, each element's content in the state
/// [`content_state`] gives it.
pub fn tokenize(source: &str) -> Vec<Token> {
    let mut tokenizer = Tokenizer::new(source);
    let mut tokens = Vec::new();
    while let Some(token) = tokenizer.next_token() {
        if let Token::StartTag { name, .. } = &token
            && let Some(state) = content_state(name)
        {
            tokenizer.set_state(state);
        }
        tokens.push(token);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Token {
        Token::Text(s.to_string())
    }

    fn start(name: &str, attrs: &[(&str, &str)], self_closing: bool) -> Token {
        Token::StartTag {
            name: name.into(),
            attrs: attrs
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                .collect(),
            self_closing,
        }
    }

    /// `source`'s tokens fed in pieces of `size` characters, the text of
    /// adjacent tokens joined.
    fn in_pieces(source: &str, size: usize) -> Vec<Token> {
        let mut tokenizer = Tokenizer::streaming();
        let chars: Vec<char> = source.chars().collect();
        let mut tokens: Vec<Token> = Vec::new();
        let take = |tokenizer: &mut Tokenizer, tokens: &mut Vec<Token>| {
            while let Some(token) = tokenizer.next_token() {
                if let Token::StartTag { name, .. } = &token
                    && let Some(state) = content_state(name)
                {
                    tokenizer.set_state(state);
                }
                match (tokens.last_mut(), token) {
                    (Some(Token::Text(last)), Token::Text(more)) => last.push_str(&more),
                    (_, token) => tokens.push(token),
                }
            }
        };
        for piece in chars.chunks(size.min(chars.len().max(1))) {
            tokenizer.feed(&piece.iter().collect::<String>());
            take(&mut tokenizer, &mut tokens);
        }
        tokenizer.end();
        take(&mut tokenizer, &mut tokens);
        tokens
    }

    #[test]
    fn plain_text_and_a_small_document() {
        assert_eq!(tokenize("hello world"), vec![text("hello world")]);
        assert_eq!(
            tokenize("<h1>Hi</h1>"),
            vec![
                start("h1", &[], false),
                text("Hi"),
                Token::EndTag("h1".into())
            ]
        );
    }

    #[test]
    fn start_tags_with_attributes_and_self_closing_ones() {
        assert_eq!(
            tokenize(r#"<p class="intro" data-kind='x' hidden>"#),
            vec![start(
                "p",
                &[("class", "intro"), ("data-kind", "x"), ("hidden", "")],
                false
            )]
        );
        assert_eq!(tokenize("<br/>"), vec![start("br", &[], true)]);
        assert_eq!(tokenize("<br />"), vec![start("br", &[], true)]);
        assert_eq!(tokenize("</DIV a=b/>"), vec![Token::EndTag("div".into())]);
    }

    #[test]
    fn names_are_lowercased_in_ascii_only() {
        assert_eq!(
            tokenize("<P DATA-\u{212A}=1 \u{c9}T\u{c9}=2>"),
            vec![start(
                "p",
                &[("data-\u{212A}", "1"), ("\u{c9}t\u{c9}", "2")],
                false
            )]
        );
    }

    #[test]
    fn duplicate_attributes_keep_the_first_and_stay_linear() {
        assert_eq!(
            tokenize(r#"<p id="a" id="b">"#),
            vec![start("p", &[("id", "a")], false)]
        );
        // 60,000 distinct attributes, and a duplicate of the first after
        // them.
        let mut source = String::from("<a");
        for i in 0..60_000 {
            source.push_str(&format!(" a{i}"));
        }
        source.push_str(" a0=again a59999=again>");
        let tokens = tokenize(&source);
        let Token::StartTag { attrs, .. } = &tokens[0] else {
            panic!("a start tag");
        };
        assert_eq!(attrs.len(), 60_000);
        assert_eq!(attrs[0], ("a0".to_owned(), String::new()));
        assert_eq!(attrs[59_999], ("a59999".to_owned(), String::new()));
    }

    #[test]
    fn comments_doctypes_and_processing_instructions() {
        assert_eq!(tokenize("<!-- hi -->"), vec![Token::Comment(" hi ".into())]);
        assert_eq!(
            tokenize("<!DOCTYPE html>"),
            vec![Token::Doctype(Doctype {
                name: Some("html".into()),
                ..Doctype::default()
            })]
        );
        assert_eq!(
            tokenize(r#"<!doctype HTML PUBLIC "-//W3C//DTD HTML 4.01//EN" 'x'>"#),
            vec![Token::Doctype(Doctype {
                name: Some("html".into()),
                public_id: Some("-//W3C//DTD HTML 4.01//EN".into()),
                system_id: Some("x".into()),
                force_quirks: false,
            })]
        );
        for source in ["<!-->after", "<!--->after", "<!-- x --!>after"] {
            let tokens = tokenize(source);
            assert_eq!(tokens.len(), 2, "{source:?}: {tokens:?}");
            assert_eq!(tokens[1], text("after"), "{source:?}");
        }
        // `<?xml ...?>` is a bogus comment; any other target a processing
        // instruction (HTML section 13.2.5.72).
        assert_eq!(
            tokenize(r#"<?xml version="1.0"?><?php echo 1 ?>"#),
            vec![
                Token::Comment(r#"?xml version="1.0"?"#.into()),
                Token::ProcessingInstruction {
                    target: "php".into(),
                    data: "echo 1 ".into()
                },
            ]
        );
    }

    #[test]
    fn carriage_returns_become_line_feeds_across_pieces_too() {
        assert_eq!(tokenize("a\r\nb\rc\n"), vec![text("a\nb\nc\n")]);
        let mut tokenizer = Tokenizer::streaming();
        tokenizer.feed("a\r");
        tokenizer.feed("\nb\r");
        tokenizer.feed("\r");
        tokenizer.end();
        let mut got = String::new();
        while let Some(Token::Text(t)) = tokenizer.next_token() {
            got.push_str(&t);
        }
        assert_eq!(got, "a\nb\n\n");
    }

    #[test]
    fn stray_less_than_signs_are_text() {
        assert_eq!(tokenize("a < b and 5 < 6"), vec![text("a < b and 5 < 6")]);
        assert_eq!(tokenize("trailing <"), vec![text("trailing <")]);
        assert_eq!(tokenize("a</"), vec![text("a</")]);
        assert_eq!(tokenize("<!"), vec![Token::Comment(String::new())]);
        assert_eq!(tokenize("<?"), vec![]);
    }

    #[test]
    fn character_references_by_the_full_table() {
        assert_eq!(
            tokenize("Fish &amp; Chips &lt;3 &#65;&#x42;&nbsp;! &NotNestedGreaterGreater; &fjlig;"),
            vec![text("Fish & Chips <3 AB\u{a0}! \u{2aa2}\u{338} fj")]
        );
        assert_eq!(
            tokenize("&nope; &#; &#x; &"),
            vec![text("&nope; &#; &#x; &")]
        );
        // The legacy names without `;`, the longest prefix winning.
        assert_eq!(
            tokenize("&amp &copy2024 &notit; &notin;"),
            vec![text("& \u{a9}2024 \u{ac}it; \u{2209}")]
        );
        // Numeric: C1 as Windows-1252, the invalid as U+FFFD.
        assert_eq!(
            tokenize("&#65 it&#146;s &#0;&#xD800;&#x110000;&#99999999999999999999;"),
            vec![text("A it\u{2019}s \u{fffd}\u{fffd}\u{fffd}\u{fffd}")]
        );
    }

    #[test]
    fn an_attribute_keeps_a_query_string() {
        assert_eq!(
            tokenize(r#"<a href="?x=1&amp=2&copy=3&notit=4&lt5&amp;y">"#),
            vec![start(
                "a",
                &[("href", "?x=1&amp=2&copy=3&notit=4&lt5&y")],
                false
            )]
        );
        assert_eq!(
            tokenize("?x=1&amp=2&copy=3&notit=4"),
            vec![text("?x=1&=2\u{a9}=3\u{ac}it=4")]
        );
    }

    #[test]
    fn script_data_escapes_keep_a_script_end_tag_inside_a_comment() {
        // `</script>` inside `<!--<script>` ... `-->` is the script's
        // text: the double-escaped state.
        // After `-->` the escape is over: a `<script>` is text again.
        assert_eq!(
            tokenize("<script><!--a--><script></script>b"),
            vec![
                start("script", &[], false),
                text("<!--a--><script>"),
                Token::EndTag("script".into()),
                text("b"),
            ]
        );
        let source = "<script><!--<script>a</script>b--></script>after";
        assert_eq!(
            tokenize(source),
            vec![
                start("script", &[], false),
                text("<!--<script>a</script>b-->"),
                Token::EndTag("script".into()),
                text("after"),
            ]
        );
        assert_eq!(
            tokenize("<style>a</p></style>"),
            vec![
                start("style", &[], false),
                text("a</p>"),
                Token::EndTag("style".into())
            ]
        );
        assert_eq!(
            tokenize("<title>&amp;<b></title>"),
            vec![
                start("title", &[], false),
                text("&<b>"),
                Token::EndTag("title".into())
            ]
        );
    }

    #[test]
    fn the_tokens_are_the_same_whatever_the_pieces() {
        let source = concat!(
            "<!DOCTYPE html PUBLIC \"a\" \"b\"><html lang=en><head>\r\n",
            "<title>T &amp; &notin; &#x41</title><script><!--<script>x</script>--></script>",
            "<style>p{}</style></head><body><p class='a' id=b>caf&eacute; &copy2024 <!-- c --->",
            "<?php x ?><textarea>&lt;</textarea><![CDATA[x]]><a href=\"?a=1&amp=2\">l</a>",
            "</body></html>"
        );
        let whole = in_pieces(source, usize::MAX);
        for size in [1, 2, 3, 5, 7, 13] {
            assert_eq!(in_pieces(source, size), whole, "pieces of {size}");
        }
    }

    #[test]
    fn lines_are_counted() {
        let mut tokenizer = Tokenizer::new("a\n<p\nid=x>\r\n<!--\n-->b");
        assert_eq!(tokenizer.line(), 1);
        // The text comes out once the tag after it is read.
        assert_eq!(
            tokenizer.next_token(),
            Some(Token::Text(
                "a
"
                .into()
            ))
        );
        assert!(matches!(
            tokenizer.next_token(),
            Some(Token::StartTag { .. })
        ));
        assert_eq!(tokenizer.line(), 3, "after the start tag");
        while tokenizer.next_token().is_some() {}
        assert_eq!(tokenizer.line(), 5);
    }

    #[test]
    fn the_named_reference_table_is_sorted() {
        for pair in NAMED_REFERENCES.windows(2) {
            assert!(pair[0].0 < pair[1].0, "{} before {}", pair[0].0, pair[1].0);
        }
        assert_eq!(NAMED_REFERENCES.len(), 2_231);
    }
}
