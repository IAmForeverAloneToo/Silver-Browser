//! A reader of the PDFs the print tests compare (M20.4; see
//! `docs/design/media.md`, "Print: what is checked against the reference
//! browser"): the reference browser's print to PDF, which
//! `examples/print.rs capture` reads, and the engine's own, which
//! `tests/print.rs` reads back. It is shared by the two through `#[path]`.
//!
//! It reads what these two write, not PDFs in general: objects standing
//! in the file, not in object streams, found by scanning for them rather
//! than through the cross-reference table; streams unfiltered or
//! Flate-compressed; the page tree, with the media box and resources a
//! page inherits; and of each page's content, its forms included, the
//! current matrix, the first clip, the images drawn and the text shown.
//! Text is read through its font's ToUnicode map, a code being two bytes
//! in a composite font and one in a simple one, and the `ActualText` of
//! marked content stands for the glyphs it covers, as for a ligature.
//! It is read as words: the text between white space, at the origin of
//! its first glyph, the left end of its baseline; white space is what
//! CSS collapses (css-text-3 §4.1), a no-break space being part of the
//! word it is in, as in the engine's words. A text object, and a move
//! to another baseline within one, ends a word. The reference browser
//! places each glyph with an operator of its own, so that each word's
//! place is its own; where a string shows several glyphs, a word that
//! starts inside it is placed at the string's start.
//!
//! Positions are in CSS pixels from the page box's top left corner.
//! What a capture keeps of a print, and how two are compared, is
//! [`Capture`].

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Read as _;

/// CSS pixels in a point.
const PX_PER_POINT: f64 = 96.0 / 72.0;

/// How deeply arrays and dictionaries, the page tree, forms and saved
/// graphics states may nest before the file is refused.
const MAX_DEPTH: usize = 256;

/// The most a stream may inflate to.
const MAX_STREAM: u64 = 1 << 28;

/// A rectangle, in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A word: its text, and the left end of its baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub x: f32,
    pub y: f32,
}

/// A page of a PDF, as this reader reads it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Page {
    /// The media box's width and height.
    pub size: (f32, f32),
    /// The first rectangle the page's content is clipped to: the
    /// reference browser's page area.
    pub clip: Option<Rect>,
    /// The rectangle each image is drawn over, in order.
    pub images: Vec<Rect>,
    /// The words, in the order they are shown.
    pub words: Vec<Word>,
    /// The fonts the page's text is shown in, by their names less any
    /// subset's tag, each once.
    pub fonts: Vec<String>,
}

/// The pages of `pdf`, in order.
pub fn read(pdf: &[u8]) -> Result<Vec<Page>, String> {
    let file = File::scan(pdf)?;
    let root = match file.trailer.as_ref().and_then(|t| t.get("Root")) {
        Some(root) => file.resolve(root).clone(),
        None => file
            .objects
            .values()
            .find(|o| o.dict().and_then(|d| d.get("Type")) == Some(&Object::Name("Catalog".into())))
            .cloned()
            .ok_or("the file has no catalog")?,
    };
    let tree = root
        .dict()
        .and_then(|d| d.get("Pages"))
        .ok_or("the catalog has no page tree")?;
    let mut pages = Vec::new();
    // The tree's nodes, last first, each with what its pages inherit.
    let mut stack = vec![(tree.clone(), None::<Object>, None::<Object>, 0)];
    while let Some((node, media_box, resources, depth)) = stack.pop() {
        if depth > MAX_DEPTH || pages.len() + stack.len() > 1 << 20 {
            return Err("the page tree is too deep".into());
        }
        let dict = file
            .resolve(&node)
            .dict()
            .ok_or("a page is not a dictionary")?;
        let media_box = dict.get("MediaBox").cloned().or(media_box);
        let resources = dict.get("Resources").cloned().or(resources);
        if let Some(kids) = dict.get("Kids") {
            let kids = file
                .resolve(kids)
                .array()
                .ok_or("a page tree's kids are not an array")?;
            for kid in kids.iter().rev() {
                stack.push((kid.clone(), media_box.clone(), resources.clone(), depth + 1));
            }
        } else {
            pages.push(file.page(dict, media_box.as_ref(), resources.as_ref())?);
        }
    }
    Ok(pages)
}

/// What a capture keeps of a print: each page's box and area, and the
/// words on it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Capture {
    pub pages: Vec<CapturedPage>,
}

/// A page of a [`Capture`].
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedPage {
    /// The page box's width and height.
    pub size: (f32, f32),
    /// The page area, in the page box; unknown for a page of the
    /// reference browser's that it draws nothing on, which it does not
    /// clip.
    pub area: Option<Rect>,
    /// The words whose baselines start in the area, their positions the
    /// area's, from its top left corner.
    pub words: Vec<Word>,
}

impl CapturedPage {
    /// The page of `size` whose area is `area`, showing `words` placed in
    /// the area's coordinates; those outside it are left out, and none
    /// is in an area unknown. White space around a word's text is taken
    /// off, and a word of none is left out.
    pub fn new(
        size: (f32, f32),
        area: Option<Rect>,
        words: impl IntoIterator<Item = Word>,
    ) -> Self {
        let words = words
            .into_iter()
            .filter(|word| {
                area.is_some_and(|area| {
                    (0.0..=area.height).contains(&word.y) && word.x < area.width
                }) && word.x.is_finite()
            })
            .map(|word| Word {
                text: normalize(&word.text),
                ..word
            })
            .filter(|word| !word.text.is_empty())
            .collect();
        CapturedPage { size, area, words }
    }
}

impl Capture {
    /// What a capture keeps of `pages`, the reference browser's print:
    /// it clips each page it draws on to its area, and its words are
    /// placed in it. A page with words and no clip is refused.
    pub fn of(pages: &[Page]) -> Result<Capture, String> {
        let pages = pages
            .iter()
            .enumerate()
            .map(|(index, page)| {
                let area = page.clip;
                let (x, y) = area.map_or((0.0, 0.0), |a| (a.x, a.y));
                if area.is_none() && !page.words.is_empty() {
                    return Err(format!("page {} is not clipped to an area", index + 1));
                }
                let words = page.words.iter().map(|word| Word {
                    text: word.text.clone(),
                    x: word.x - x,
                    y: word.y - y,
                });
                Ok(CapturedPage::new(page.size, area, words))
            })
            .collect::<Result<_, String>>()?;
        Ok(Capture { pages })
    }

    /// The capture as the text of its file: `header` as comment lines,
    /// then a `page` line for each page, its box's width and height and,
    /// when known, its area's left, top, width and height, and a `word`
    /// line for each word on it, its left end and baseline and its text.
    pub fn to_text(&self, header: &str) -> String {
        let mut out = String::new();
        for line in header.lines() {
            let _ = writeln!(out, "# {line}");
        }
        for page in &self.pages {
            let _ = write!(out, "page {} {}", number(page.size.0), number(page.size.1));
            if let Some(a) = page.area {
                let _ = write!(
                    out,
                    " {} {} {} {}",
                    number(a.x),
                    number(a.y),
                    number(a.width),
                    number(a.height)
                );
            }
            out.push('\n');
            for word in &page.words {
                let _ = writeln!(
                    out,
                    "word {} {} {}",
                    number(word.x),
                    number(word.y),
                    word.text
                );
            }
        }
        out
    }

    /// The capture whose file's text `text` is.
    pub fn parse(text: &str) -> Result<Capture, String> {
        let mut pages: Vec<CapturedPage> = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let bad = || format!("line {}: {line}", index + 1);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (kind, rest) = line.split_once(' ').ok_or_else(bad)?;
            match kind {
                "page" => {
                    let numbers = rest
                        .split(' ')
                        .map(|n| n.parse::<f32>().map_err(|_| bad()))
                        .collect::<Result<Vec<f32>, String>>()?;
                    let (size, area) = match numbers[..] {
                        [w, h] => ((w, h), None),
                        [w, h, x, y, width, height] => (
                            (w, h),
                            Some(Rect {
                                x,
                                y,
                                width,
                                height,
                            }),
                        ),
                        _ => return Err(bad()),
                    };
                    pages.push(CapturedPage {
                        size,
                        area,
                        words: Vec::new(),
                    });
                }
                "word" => {
                    let mut fields = rest.splitn(3, ' ');
                    let mut next = || fields.next().ok_or_else(bad);
                    let x = next()?.parse().map_err(|_| bad())?;
                    let y = next()?.parse().map_err(|_| bad())?;
                    let text = next()?.to_string();
                    let page = pages.last_mut().ok_or_else(bad)?;
                    page.words.push(Word { text, x, y });
                }
                _ => return Err(bad()),
            }
        }
        Ok(Capture { pages })
    }
}

/// Whether `c` is white space as CSS collapses it (css-text-3 §4.1): a
/// no-break space is not.
fn is_white_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

/// `text` with its white space collapsed to single spaces and none at
/// either end.
pub fn normalize(text: &str) -> String {
    text.split(is_white_space)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// How `actual` differs from `expected`, a line for each difference:
/// the number of pages, a page's size or area by more than `box_slack`
/// (an area `expected` does not know being any),
/// a word of `expected` that `actual` has no word of the same text
/// within `slack` of, on the same page, and a word of `actual` left over.
/// Words are paired by their text, each with the nearest.
pub fn differences(
    expected: &Capture,
    actual: &Capture,
    slack: f32,
    box_slack: f32,
) -> Vec<String> {
    let mut out = Vec::new();
    if expected.pages.len() != actual.pages.len() {
        out.push(format!(
            "{} pages, where {} were expected",
            actual.pages.len(),
            expected.pages.len()
        ));
    }
    for (index, (e, a)) in expected.pages.iter().zip(&actual.pages).enumerate() {
        let page = index + 1;
        let near = |x: f32, y: f32| (x - y).abs() <= box_slack;
        if !near(e.size.0, a.size.0) || !near(e.size.1, a.size.1) {
            out.push(format!(
                "page {page}: the box is {:?}, not {:?}",
                a.size, e.size
            ));
        }
        if let Some(ea) = e.area {
            let same = a.area.is_some_and(|aa| {
                near(ea.x, aa.x)
                    && near(ea.y, aa.y)
                    && near(ea.width, aa.width)
                    && near(ea.height, aa.height)
            });
            if !same {
                out.push(format!("page {page}: the area is {:?}, not {ea:?}", a.area));
            }
        }
        let mut left: Vec<Option<&Word>> = a.words.iter().map(Some).collect();
        for word in &e.words {
            let distance = |w: &Word| (w.x - word.x).abs().max((w.y - word.y).abs());
            let nearest = left
                .iter()
                .enumerate()
                .filter_map(|(i, w)| w.filter(|w| w.text == word.text).map(|w| (i, distance(w))))
                .min_by(|x, y| x.1.total_cmp(&y.1));
            match nearest {
                Some((i, d)) if d <= slack => left[i] = None,
                Some((i, _)) => {
                    let w = left[i].take().expect("a word left");
                    out.push(format!(
                        "page {page}: {:?} at ({}, {}), not ({}, {})",
                        word.text, w.x, w.y, word.x, word.y
                    ));
                }
                None => out.push(format!(
                    "page {page}: no {:?} at ({}, {})",
                    word.text, word.x, word.y
                )),
            }
        }
        for w in left.into_iter().flatten() {
            out.push(format!(
                "page {page}: {:?} at ({}, {}) was not expected",
                w.text, w.x, w.y
            ));
        }
    }
    out
}

/// `value` with two decimals at most, and no trailing zeros.
fn number(value: f32) -> String {
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

/// A PDF object.
#[derive(Debug, Clone, PartialEq)]
enum Object {
    Null,
    Bool(bool),
    Number(f64),
    Name(String),
    String(Vec<u8>),
    Array(Vec<Object>),
    Dict(Dict),
    /// A reference to an object by its number.
    Ref(u32),
    /// A stream: its dictionary and its bytes as the file holds them.
    Stream(Dict, Vec<u8>),
    /// A bare word: a content stream's operator, or a keyword.
    Keyword(String),
}

type Dict = HashMap<String, Object>;

impl Object {
    fn dict(&self) -> Option<&Dict> {
        match self {
            Object::Dict(d) | Object::Stream(d, _) => Some(d),
            _ => None,
        }
    }

    fn array(&self) -> Option<&[Object]> {
        match self {
            Object::Array(a) => Some(a),
            _ => None,
        }
    }

    fn number(&self) -> Option<f64> {
        match self {
            Object::Number(n) => Some(*n),
            _ => None,
        }
    }

    fn name(&self) -> Option<&str> {
        match self {
            Object::Name(n) => Some(n),
            _ => None,
        }
    }

    fn string(&self) -> Option<&[u8]> {
        match self {
            Object::String(s) => Some(s),
            _ => None,
        }
    }
}

/// A reader of the objects in bytes, one after another.
struct Lexer<'a> {
    bytes: &'a [u8],
    at: usize,
}

fn is_space(b: u8) -> bool {
    matches!(b, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

impl<'a> Lexer<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Lexer { bytes, at: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    /// Past white space and comments.
    fn skip_space(&mut self) {
        while let Some(b) = self.peek() {
            if is_space(b) {
                self.at += 1;
            } else if b == b'%' {
                while self.peek().is_some_and(|b| b != b'\n' && b != b'\r') {
                    self.at += 1;
                }
            } else {
                break;
            }
        }
    }

    /// The bytes of a word: a number, a name's or a keyword.
    fn word(&mut self) -> &'a [u8] {
        let start = self.at;
        while self
            .peek()
            .is_some_and(|b| !is_space(b) && !is_delimiter(b))
        {
            self.at += 1;
        }
        &self.bytes[start..self.at]
    }

    /// The next object, `None` at the end; a closing bracket or `>>` is
    /// a keyword of its own, which an array or dictionary ends at.
    fn next(&mut self, depth: usize) -> Option<Result<Object, String>> {
        self.skip_space();
        let b = self.peek()?;
        if depth > MAX_DEPTH {
            return Some(Err("objects nest too deeply".into()));
        }
        Some(match b {
            b'/' => {
                self.at += 1;
                Ok(Object::Name(name(self.word())))
            }
            b'(' => self.literal().map(Object::String),
            b'<' if self.bytes.get(self.at + 1) == Some(&b'<') => {
                self.at += 2;
                self.dictionary(depth)
            }
            b'<' => self.hex().map(Object::String),
            b'>' if self.bytes.get(self.at + 1) == Some(&b'>') => {
                self.at += 2;
                Ok(Object::Keyword(">>".into()))
            }
            b'[' => {
                self.at += 1;
                self.array(depth)
            }
            b']' | b'{' | b'}' | b')' | b'>' => {
                self.at += 1;
                Ok(Object::Keyword((b as char).to_string()))
            }
            _ => {
                let word = self.word();
                if word.is_empty() {
                    self.at += 1;
                    return Some(Err("an unexpected byte".into()));
                }
                Ok(self.word_object(word))
            }
        })
    }

    /// The object `word` is: a number, a reference if two integers and
    /// `R` follow, a boolean, null, or a keyword.
    fn word_object(&mut self, word: &[u8]) -> Object {
        let text = String::from_utf8_lossy(word);
        match &*text {
            "true" => return Object::Bool(true),
            "false" => return Object::Bool(false),
            "null" => return Object::Null,
            _ => {}
        }
        let Ok(number) = text.parse::<f64>() else {
            return Object::Keyword(text.into_owned());
        };
        if word.iter().all(u8::is_ascii_digit) {
            // `n g R` is a reference.
            let back = self.at;
            self.skip_space();
            let generation = self.word();
            if !generation.is_empty() && generation.iter().all(u8::is_ascii_digit) {
                self.skip_space();
                if self.word() == b"R"
                    && let Ok(id) = text.parse()
                {
                    return Object::Ref(id);
                }
            }
            self.at = back;
        }
        Object::Number(number)
    }

    fn array(&mut self, depth: usize) -> Result<Object, String> {
        let mut items = Vec::new();
        loop {
            match self.next(depth + 1) {
                None => return Err("an array is not closed".into()),
                Some(Ok(Object::Keyword(k))) if k == "]" => return Ok(Object::Array(items)),
                Some(item) => items.push(item?),
            }
        }
    }

    fn dictionary(&mut self, depth: usize) -> Result<Object, String> {
        let mut dict = Dict::new();
        loop {
            match self.next(depth + 1) {
                None => return Err("a dictionary is not closed".into()),
                Some(Ok(Object::Keyword(k))) if k == ">>" => return Ok(Object::Dict(dict)),
                Some(Ok(Object::Name(key))) => {
                    let value = self
                        .next(depth + 1)
                        .ok_or("a dictionary's value is missing")??;
                    dict.insert(key, value);
                }
                Some(other) => return Err(format!("a dictionary's key is {:?}", other?)),
            }
        }
    }

    /// A literal string, from its `(`.
    fn literal(&mut self) -> Result<Vec<u8>, String> {
        self.at += 1;
        let mut out = Vec::new();
        let mut open = 1;
        loop {
            let b = self.peek().ok_or("a string is not closed")?;
            self.at += 1;
            match b {
                b'(' => {
                    open += 1;
                    out.push(b);
                }
                b')' => {
                    open -= 1;
                    if open == 0 {
                        return Ok(out);
                    }
                    out.push(b);
                }
                b'\\' => {
                    let e = self.peek().ok_or("a string is not closed")?;
                    self.at += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut value = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d @ b'0'..=b'7') => {
                                        value = value * 8 + u32::from(d - b'0');
                                        self.at += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(value as u8);
                        }
                        // A line's end after a backslash continues the
                        // string on the next line.
                        b'\r' => {
                            if self.peek() == Some(b'\n') {
                                self.at += 1;
                            }
                        }
                        b'\n' => {}
                        other => out.push(other),
                    }
                }
                _ => out.push(b),
            }
        }
    }

    /// A hexadecimal string, from its `<`.
    fn hex(&mut self) -> Result<Vec<u8>, String> {
        self.at += 1;
        let mut digits = Vec::new();
        loop {
            let b = self.peek().ok_or("a hexadecimal string is not closed")?;
            self.at += 1;
            match b {
                b'>' => break,
                b if is_space(b) => {}
                b => digits.push(
                    (b as char)
                        .to_digit(16)
                        .ok_or("a hexadecimal string has a byte that is no digit")?
                        as u8,
                ),
            }
        }
        if digits.len() % 2 == 1 {
            digits.push(0);
        }
        Ok(digits.chunks(2).map(|p| p[0] * 16 + p[1]).collect())
    }
}

/// A name's text, its `#xx` escapes read.
fn name(word: &[u8]) -> String {
    let mut out = Vec::with_capacity(word.len());
    let mut i = 0;
    while i < word.len() {
        let escaped = word
            .get(i + 1..i + 3)
            .and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok());
        match (word[i], escaped) {
            (b'#', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The objects of a file, by their numbers, and its trailer.
struct File {
    objects: HashMap<u32, Object>,
    trailer: Option<Dict>,
}

impl File {
    /// The objects `pdf` holds, scanned for one after another.
    fn scan(pdf: &[u8]) -> Result<File, String> {
        if !pdf.starts_with(b"%PDF-") {
            return Err("the file is not a PDF".into());
        }
        let mut lexer = Lexer::new(pdf);
        let mut objects = HashMap::new();
        let mut trailer = None;
        // The two objects before the one read: `n g obj`.
        let mut last: [Option<Object>; 2] = [None, None];
        while let Some(item) = lexer.next(0) {
            let Ok(item) = item else {
                // Between objects, what this reader does not take is
                // passed over.
                last = [None, None];
                continue;
            };
            match &item {
                Object::Keyword(k) if k == "obj" => {
                    if let [Some(Object::Number(id)), Some(Object::Number(_))] = &last {
                        let id = *id as u32;
                        let object = lexer.next(0).ok_or("an object is missing")??;
                        let object = stream(&mut lexer, object)?;
                        objects.insert(id, object);
                    }
                    last = [None, None];
                    continue;
                }
                Object::Keyword(k) if k == "trailer" => {
                    if let Some(Ok(Object::Dict(d))) = lexer.next(0) {
                        trailer = Some(d);
                    }
                }
                _ => {}
            }
            last = [last[1].take(), Some(item)];
        }
        Ok(File { objects, trailer })
    }

    /// The object `object` is, through a reference; a reference to no
    /// object is null.
    fn resolve<'a>(&'a self, object: &'a Object) -> &'a Object {
        match object {
            Object::Ref(id) => self.objects.get(id).unwrap_or(&Object::Null),
            other => other,
        }
    }

    /// The dictionary `object` is, through a reference.
    fn dict<'a>(&'a self, object: Option<&'a Object>) -> Option<&'a Dict> {
        object.map(|o| self.resolve(o)).and_then(Object::dict)
    }

    /// The bytes of the stream `object` is, through a reference, its
    /// filter undone.
    fn decoded(&self, object: &Object) -> Result<Vec<u8>, String> {
        let Object::Stream(dict, raw) = self.resolve(object) else {
            return Err("a stream was expected".into());
        };
        let filters: Vec<&str> = match dict.get("Filter").map(|f| self.resolve(f)) {
            None => Vec::new(),
            Some(Object::Name(n)) => vec![n],
            Some(Object::Array(a)) => a.iter().filter_map(Object::name).collect(),
            Some(other) => return Err(format!("a stream's filter is {other:?}")),
        };
        match filters[..] {
            [] => Ok(raw.clone()),
            ["FlateDecode"] => {
                let mut out = Vec::new();
                flate2::read::ZlibDecoder::new(&raw[..])
                    .take(MAX_STREAM)
                    .read_to_end(&mut out)
                    .map_err(|e| format!("a stream does not inflate: {e}"))?;
                Ok(out)
            }
            _ => Err(format!("a stream's filters are {filters:?}")),
        }
    }

    /// The page `dict` is, its media box and resources the ones it
    /// inherits unless its own.
    fn page(
        &self,
        dict: &Dict,
        media_box: Option<&Object>,
        resources: Option<&Object>,
    ) -> Result<Page, String> {
        let media_box = media_box
            .map(|m| self.resolve(m))
            .and_then(Object::array)
            .and_then(|a| {
                a.iter()
                    .map(|n| self.resolve(n).number())
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or("a page has no media box")?;
        let [left, bottom, right, top] = media_box[..] else {
            return Err("a page's media box is not four numbers".into());
        };
        let mut page = Page {
            size: (
                ((right - left) * PX_PER_POINT) as f32,
                ((top - bottom) * PX_PER_POINT) as f32,
            ),
            ..Page::default()
        };
        let mut content = Vec::new();
        match dict.get("Contents").map(|c| self.resolve(c)) {
            None => {}
            Some(Object::Array(parts)) => {
                for part in parts {
                    content.extend(self.decoded(part)?);
                    content.push(b'\n');
                }
            }
            Some(stream) => content = self.decoded(stream)?,
        }
        let resources = self.dict(resources).cloned().unwrap_or_default();
        let mut painter = Painter {
            file: self,
            origin: (left, top),
            page: &mut page,
            fonts: HashMap::new(),
        };
        painter.paint(&content, &resources, IDENTITY, 0)?;
        Ok(page)
    }
}

/// `object`, read after `n g obj`, with the stream that follows it if
/// one does.
fn stream(lexer: &mut Lexer, object: Object) -> Result<Object, String> {
    let Object::Dict(dict) = object else {
        return Ok(object);
    };
    let back = lexer.at;
    lexer.skip_space();
    if lexer.word() != b"stream" {
        lexer.at = back;
        return Ok(Object::Dict(dict));
    }
    let bytes = lexer.bytes;
    // The data begins after the line's end.
    let mut start = lexer.at;
    if bytes.get(start) == Some(&b'\r') {
        start += 1;
    }
    if bytes.get(start) == Some(&b'\n') {
        start += 1;
    }
    let ends_at = |end: usize| {
        let mut at = end;
        while bytes.get(at).is_some_and(|b| is_space(*b)) {
            at += 1;
        }
        bytes[at.min(bytes.len())..].starts_with(b"endstream")
    };
    let end = match dict.get("Length").and_then(Object::number) {
        Some(length)
            if length >= 0.0
                && start + (length as usize) <= bytes.len()
                && ends_at(start + length as usize) =>
        {
            start + length as usize
        }
        _ => {
            let found = bytes[start..]
                .windows(9)
                .position(|w| w == b"endstream")
                .ok_or("a stream does not end")?;
            let mut end = start + found;
            if end > start && bytes[end - 1] == b'\n' {
                end -= 1;
            }
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            end
        }
    };
    let data = bytes[start..end].to_vec();
    let after = bytes[end..]
        .windows(9)
        .position(|w| w == b"endstream")
        .ok_or("a stream does not end")?;
    lexer.at = end + after + 9;
    Ok(Object::Stream(dict, data))
}

/// An affine matrix, `[a b c d e f]` as PDF writes it.
type Matrix = [f64; 6];

const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m` then `n`: a point mapped by `m`, then by `n`.
fn concat(m: &Matrix, n: &Matrix) -> Matrix {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

fn apply(m: &Matrix, x: f64, y: f64) -> (f64, f64) {
    (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5])
}

fn translation(x: f64, y: f64) -> Matrix {
    [1.0, 0.0, 0.0, 1.0, x, y]
}

/// A font as the text shown in it is read.
struct Font {
    /// Whether a code is two bytes, as in a composite font, or one.
    wide: bool,
    /// The text each code stands for, from the font's ToUnicode map.
    text: HashMap<u32, String>,
    /// The font's name less its subset's tag.
    name: String,
}

/// What the text state holds while a text object is open.
#[derive(Default)]
struct Text {
    matrix: Matrix,
    line: Matrix,
    leading: f64,
    font: Option<String>,
    /// The baseline of the text being read, while a text object shows
    /// it.
    baseline: Option<f32>,
    /// The word being read.
    word: Option<Word>,
}

/// The reader of a page's content.
struct Painter<'a> {
    file: &'a File,
    /// The media box's left and top, in points.
    origin: (f64, f64),
    page: &'a mut Page,
    /// The fonts read so far, by their objects' numbers, or their names
    /// in their resources when given directly.
    fonts: HashMap<String, Font>,
}

impl Painter<'_> {
    /// The point `(x, y)` of the user space `m` maps, in CSS pixels in
    /// the page box.
    fn place(&self, m: &Matrix, x: f64, y: f64) -> (f32, f32) {
        let (x, y) = apply(m, x, y);
        (
            ((x - self.origin.0) * PX_PER_POINT) as f32,
            ((self.origin.1 - y) * PX_PER_POINT) as f32,
        )
    }

    /// The rectangle around the corners of `(x, y, w, h)` mapped by `m`.
    fn rect(&self, m: &Matrix, x: f64, y: f64, w: f64, h: f64) -> Rect {
        let corners = [
            self.place(m, x, y),
            self.place(m, x + w, y),
            self.place(m, x, y + h),
            self.place(m, x + w, y + h),
        ];
        let min_x = corners.iter().map(|c| c.0).fold(f32::INFINITY, f32::min);
        let max_x = corners
            .iter()
            .map(|c| c.0)
            .fold(f32::NEG_INFINITY, f32::max);
        let min_y = corners.iter().map(|c| c.1).fold(f32::INFINITY, f32::min);
        let max_y = corners
            .iter()
            .map(|c| c.1)
            .fold(f32::NEG_INFINITY, f32::max);
        Rect {
            x: min_x,
            y: min_y,
            width: max_x - min_x,
            height: max_y - min_y,
        }
    }

    /// The key of the font `name` names in `resources`, read if it has
    /// not been.
    fn font(&mut self, resources: &Dict, name: &str) -> Result<String, String> {
        let fonts = self.file.dict(resources.get("Font"));
        let object = fonts
            .and_then(|f| f.get(name))
            .ok_or_else(|| format!("no font {name}"))?;
        let key = match object {
            Object::Ref(id) => format!("{id}"),
            _ => format!("/{name}"),
        };
        if !self.fonts.contains_key(&key) {
            let dict = self
                .file
                .resolve(object)
                .dict()
                .ok_or("a font is not a dictionary")?;
            let font = self.read_font(dict)?;
            if !self.page.fonts.contains(&font.name) {
                self.page.fonts.push(font.name.clone());
            }
            self.fonts.insert(key.clone(), font);
        }
        Ok(key)
    }

    fn read_font(&self, dict: &Dict) -> Result<Font, String> {
        let file = self.file;
        let wide = dict.get("Subtype").and_then(Object::name) == Some("Type0");
        let base = dict
            .get("BaseFont")
            .and_then(|b| file.resolve(b).name())
            .or_else(|| {
                file.dict(dict.get("FontDescriptor"))
                    .and_then(|d| d.get("FontName"))
                    .and_then(|n| file.resolve(n).name())
            })
            .unwrap_or("");
        // A subset's tag is six capital letters and a plus.
        let name = match base.split_once('+') {
            Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => {
                rest
            }
            _ => base,
        };
        let text = match dict.get("ToUnicode") {
            Some(map) => to_unicode(&file.decoded(map)?)?,
            None => HashMap::new(),
        };
        Ok(Font {
            wide,
            text,
            name: name.to_string(),
        })
    }

    /// Reads `content`, under `matrix`, with `resources`; a form it draws
    /// is read in turn, `depth` deep.
    fn paint(
        &mut self,
        content: &[u8],
        resources: &Dict,
        matrix: Matrix,
        depth: usize,
    ) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err("forms nest too deeply".into());
        }
        let mut lexer = Lexer::new(content);
        let mut operands: Vec<Object> = Vec::new();
        let mut ctm = matrix;
        let mut saved: Vec<Matrix> = Vec::new();
        let mut text = Text::default();
        // The rectangle a path is, if it is one, and whether it clips.
        let mut path: Option<Rect> = None;
        let mut clips = false;
        // Marked content open: each's `ActualText`, and whether it was
        // shown.
        let mut marked: Vec<Option<(String, bool)>> = Vec::new();
        while let Some(item) = lexer.next(0) {
            let operator = match item? {
                Object::Keyword(k) => k,
                operand => {
                    operands.push(operand);
                    continue;
                }
            };
            let n = |i: usize| operands.get(i).and_then(Object::number).unwrap_or(0.0);
            let six = || [n(0), n(1), n(2), n(3), n(4), n(5)];
            match operator.as_str() {
                "q" => {
                    if saved.len() > MAX_DEPTH {
                        return Err("graphics states nest too deeply".into());
                    }
                    saved.push(ctm);
                }
                "Q" => ctm = saved.pop().unwrap_or(matrix),
                "cm" => ctm = concat(&six(), &ctm),
                "re" => path = Some(self.rect(&ctm, n(0), n(1), n(2), n(3))),
                "m" | "l" | "c" | "v" | "y" | "h" => path = None,
                "W" | "W*" => clips = true,
                "n" | "f" | "F" | "f*" | "S" | "s" | "B" | "B*" | "b" | "b*" => {
                    if clips && self.page.clip.is_none() {
                        self.page.clip = path;
                    }
                    path = None;
                    clips = false;
                }
                "BT" => {
                    text.matrix = IDENTITY;
                    text.line = IDENTITY;
                }
                "ET" => self.end_word(&mut text, true),
                "Tf" => {
                    if let Some(name) = operands.first().and_then(Object::name) {
                        text.font = Some(self.font(resources, name)?);
                    }
                }
                "TL" => text.leading = n(0),
                "Tm" => {
                    text.matrix = six();
                    text.line = text.matrix;
                }
                "Td" | "TD" => {
                    if operator == "TD" {
                        text.leading = -n(1);
                    }
                    text.line = concat(&translation(n(0), n(1)), &text.line);
                    text.matrix = text.line;
                }
                "T*" | "'" | "\"" => {
                    text.line = concat(&translation(0.0, -text.leading), &text.line);
                    text.matrix = text.line;
                    if operator != "T*"
                        && let Some(s) = operands.last().and_then(Object::string)
                    {
                        self.show(&mut text, &mut marked, s, &ctm);
                    }
                }
                "Tj" => {
                    if let Some(s) = operands.first().and_then(Object::string) {
                        self.show(&mut text, &mut marked, s, &ctm);
                    }
                }
                "TJ" => {
                    if let Some(parts) = operands.first().and_then(Object::array) {
                        for s in parts.iter().filter_map(Object::string) {
                            self.show(&mut text, &mut marked, s, &ctm);
                        }
                    }
                }
                "BMC" => marked.push(None),
                "BDC" => {
                    let properties = match operands.get(1) {
                        Some(Object::Name(name)) => self
                            .file
                            .dict(resources.get("Properties"))
                            .and_then(|p| self.file.dict(p.get(name.as_str()))),
                        Some(other) => other.dict(),
                        None => None,
                    };
                    let actual = properties
                        .and_then(|p| p.get("ActualText"))
                        .and_then(|t| self.file.resolve(t).string())
                        .map(|t| (text_string(t), false));
                    marked.push(actual);
                }
                "EMC" => {
                    marked.pop();
                }
                "Do" => {
                    if let Some(name) = operands.first().and_then(Object::name) {
                        self.draw(resources, name, &ctm, depth)?;
                    }
                }
                "BI" => {
                    // An inline image's data, up to its `EI`, is passed
                    // over.
                    let rest = &content[lexer.at..];
                    let end = rest
                        .windows(3)
                        .position(|w| is_space(w[0]) && &w[1..] == b"EI")
                        .ok_or("an inline image does not end")?;
                    lexer.at += end + 3;
                }
                _ => {}
            }
            operands.clear();
        }
        self.end_word(&mut text, true);
        Ok(())
    }

    /// Shows the string `s` in the text state's font at its matrix.
    fn show(
        &mut self,
        text: &mut Text,
        marked: &mut [Option<(String, bool)>],
        s: &[u8],
        ctm: &Matrix,
    ) {
        let shown = match marked.iter_mut().rev().find_map(Option::as_mut) {
            // Marked content's actual text stands for its glyphs, once.
            Some((actual, used)) => {
                if *used {
                    String::new()
                } else {
                    *used = true;
                    actual.clone()
                }
            }
            None => {
                let font = text.font.as_ref().and_then(|key| self.fonts.get(key));
                decode(font, s)
            }
        };
        let (x, y) = self.place(&concat(&text.matrix, ctm), 0.0, 0.0);
        if text
            .baseline
            .is_some_and(|baseline| (baseline - y).abs() > 0.5)
        {
            self.end_word(text, true);
        }
        text.baseline = Some(y);
        for c in shown.chars() {
            if is_white_space(c) {
                self.end_word(text, false);
            } else {
                text.word
                    .get_or_insert(Word {
                        text: String::new(),
                        x,
                        y,
                    })
                    .text
                    .push(c);
            }
        }
    }

    /// Ends the word being read, and with `line` the text object's
    /// baseline too.
    fn end_word(&mut self, text: &mut Text, line: bool) {
        if let Some(word) = text.word.take() {
            self.page.words.push(word);
        }
        if line {
            text.baseline = None;
        }
    }

    /// Draws the external object `name` names in `resources`: an image
    /// over the unit square, a form by reading its content.
    fn draw(
        &mut self,
        resources: &Dict,
        name: &str,
        ctm: &Matrix,
        depth: usize,
    ) -> Result<(), String> {
        let file = self.file;
        let Some(object) = file
            .dict(resources.get("XObject"))
            .and_then(|x| x.get(name))
        else {
            return Err(format!("no external object {name}"));
        };
        let dict = file
            .resolve(object)
            .dict()
            .ok_or("an external object is no stream")?;
        match dict.get("Subtype").and_then(Object::name) {
            Some("Image") => {
                let rect = self.rect(ctm, 0.0, 0.0, 1.0, 1.0);
                self.page.images.push(rect);
            }
            Some("Form") => {
                let matrix = dict
                    .get("Matrix")
                    .and_then(|m| file.resolve(m).array())
                    .and_then(|m| m.iter().map(Object::number).collect::<Option<Vec<_>>>())
                    .and_then(|m| <[f64; 6]>::try_from(m).ok())
                    .unwrap_or(IDENTITY);
                let content = file.decoded(object)?;
                let inner = file
                    .dict(dict.get("Resources"))
                    .unwrap_or(resources)
                    .clone();
                self.paint(&content, &inner, concat(&matrix, ctm), depth + 1)?;
            }
            _ => {}
        }
        Ok(())
    }
}

/// The text the codes of `s` stand for in `font`: each code's from the
/// font's map, a byte's own character when a simple font has none, and
/// U+FFFD when a composite one has none.
fn decode(font: Option<&Font>, s: &[u8]) -> String {
    let wide = font.is_some_and(|f| f.wide);
    let codes: Vec<u32> = if wide {
        s.chunks(2)
            .map(|c| c.iter().fold(0, |code, b| (code << 8) | u32::from(*b)))
            .collect()
    } else {
        s.iter().map(|b| u32::from(*b)).collect()
    };
    codes
        .into_iter()
        .map(|code| match font.and_then(|f| f.text.get(&code)) {
            Some(text) => text.clone(),
            None if wide => '\u{fffd}'.to_string(),
            None => char::from(code as u8).to_string(),
        })
        .collect()
}

/// A text string: UTF-16 after its byte order mark, else a byte a
/// character (PDFDocEncoding, which is Latin-1 where this reader meets
/// it).
fn text_string(s: &[u8]) -> String {
    match s.strip_prefix(&[0xfe, 0xff]) {
        Some(utf16) => utf16_be(utf16),
        None => s.iter().map(|b| char::from(*b)).collect(),
    }
}

fn utf16_be(bytes: &[u8]) -> String {
    let units = bytes
        .chunks(2)
        .map(|c| (u16::from(c[0]) << 8) | u16::from(*c.get(1).unwrap_or(&0)));
    char::decode_utf16(units)
        .map(|c| c.unwrap_or('\u{fffd}'))
        .collect()
}

/// The text each code stands for in a ToUnicode CMap: its `bfchar`
/// pairs and its `bfrange`s, a range's last byte counting up from its
/// first code's text.
fn to_unicode(cmap: &[u8]) -> Result<HashMap<u32, String>, String> {
    let mut lexer = Lexer::new(cmap);
    let mut out = HashMap::new();
    let mut objects = Vec::new();
    while let Some(item) = lexer.next(0) {
        objects.push(item?);
    }
    let code = |s: &[u8]| s.iter().fold(0u32, |code, b| (code << 8) | u32::from(*b));
    let mut i = 0;
    while i < objects.len() {
        match &objects[i] {
            Object::Keyword(k) if k == "beginbfchar" => {
                i += 1;
                while let [Object::String(src), Object::String(dst), ..] = &objects[i..] {
                    out.insert(code(src), utf16_be(dst));
                    i += 2;
                }
            }
            Object::Keyword(k) if k == "beginbfrange" => {
                i += 1;
                while let [Object::String(lo), Object::String(hi), dst, ..] = &objects[i..] {
                    let (lo, hi) = (code(lo), code(hi));
                    if hi < lo || hi - lo > 0xffff {
                        return Err("a ToUnicode range is too long".into());
                    }
                    for (offset, c) in (lo..=hi).enumerate() {
                        let text = match dst {
                            Object::String(first) => {
                                let mut units: Vec<u8> = first.clone();
                                // The last unit counts up.
                                let n = units.len();
                                if n >= 2 {
                                    let last =
                                        (u16::from(units[n - 2]) << 8) | u16::from(units[n - 1]);
                                    let last = last.wrapping_add(offset as u16);
                                    units[n - 2] = (last >> 8) as u8;
                                    units[n - 1] = last as u8;
                                }
                                utf16_be(&units)
                            }
                            Object::Array(each) => {
                                match each.get(offset).and_then(Object::string) {
                                    Some(s) => utf16_be(s),
                                    None => continue,
                                }
                            }
                            _ => return Err("a ToUnicode range's text is neither".into()),
                        };
                        out.insert(c, text);
                    }
                    i += 3;
                }
            }
            _ => i += 1,
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PDF of `objects`, numbered from one, the first the catalog.
    fn pdf<T: AsRef<[u8]>>(objects: &[T]) -> Vec<u8> {
        let mut out = b"%PDF-1.4\n%\xd3\xeb\xe9\xe1\n".to_vec();
        for (index, object) in objects.iter().enumerate() {
            out.extend(format!("{} 0 obj\n", index + 1).bytes());
            out.extend(object.as_ref());
            out.extend(b"\nendobj\n");
        }
        out.extend(b"xref\n0 1\n0000000000 65535 f \ntrailer\n<</Size 9 /Root 1 0 R>>\n");
        out.extend(b"startxref\n0\n%%EOF\n");
        out
    }

    /// A stream object of `data`, its dictionary holding `dict`.
    fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
        let mut out = format!("<<{dict} /Length {}>> stream\n", data.len()).into_bytes();
        out.extend(data);
        out.extend(b"\nendstream");
        out
    }

    fn deflated(data: &[u8]) -> Vec<u8> {
        use std::io::Write as _;
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).expect("a vector takes every byte");
        encoder.finish().expect("a vector takes every byte")
    }

    const CMAP: &str = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
        /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def \
        1 begincodespacerange <0000> <FFFF> endcodespacerange \
        2 beginbfchar <0003> <0020> <13B2> <FB01> endbfchar \
        1 beginbfrange <0044> <0046> <0061> endbfrange \
        1 beginbfrange <0050> <0051> [<0078> <D83DDE00>] endbfrange \
        endcmap CMapName currentdict /CMap defineresource pop end end";

    #[test]
    fn the_reference_browsers_pages_read_as_their_boxes_clips_and_text() {
        let content = b".23999999 0 0 -.23999999 0 792 cm\nq\n115.625 115.625 2316.6992 3066.6992 re\nW* n\n\
            q\n3.125 0 0 3.125 115.625 115.625 cm\n/P <</MCID 0 >>BDC\nBT\n/F4 16 Tf\n1 0 0 -1 8 31 Tm\n\
            <0044> Tj\n10 0 Td <0045> Tj\n10 0 Td <0003> Tj\n\
            /Span<</ActualText (fi) >> BDC\n5 0 Td <13B2> Tj\nEMC\n10 0 Td <0046> Tj\nET\n\
            BT\n/F4 16 Tf\n1 0 0 -1 8 50 Tm\n<00500051> Tj\n0 19 Td (\\000\\077) Tj\nET\nEMC\nQ\nQ\n";
        let objects = [
            b"<</Type /Catalog /Pages 2 0 R>>".to_vec(),
            b"<</Type /Pages /Count 1 /Kids [3 0 R] /MediaBox [0 0 612 792]>>".to_vec(),
            b"<</Type /Page /Parent 2 0 R /Resources <</Font <</F4 5 0 R>>>> /Contents 4 0 R>>"
                .to_vec(),
            stream(" /Filter /FlateDecode", &deflated(content)),
            b"<</Type /Font /Subtype /Type0 /BaseFont /AAAAAA+DejaVuSans /Encoding /Identity-H \
             /ToUnicode 6 0 R>>"
                .to_vec(),
            stream("", CMAP.as_bytes()),
        ];
        let pages = read(&pdf(&objects)).expect("the file reads");
        assert_eq!(pages.len(), 1);
        let page = &pages[0];
        assert_eq!(page.size, (816.0, 1056.0));
        let clip = page.clip.expect("clipped");
        assert!((clip.x - 37.0).abs() < 0.01 && (clip.y - 37.0).abs() < 0.01);
        // The reference browser's clip is a little past the area, at
        // 741.34 by 981.34.
        assert!((clip.width - 741.34).abs() < 0.01 && (clip.height - 981.34).abs() < 0.01);
        assert_eq!(page.fonts, ["DejaVuSans"]);
        let words: Vec<(&str, f32, f32)> = page
            .words
            .iter()
            .map(|w| {
                (
                    w.text.as_str(),
                    (w.x * 100.0).round() / 100.0,
                    (w.y * 100.0).round() / 100.0,
                )
            })
            .collect();
        // The actual text stands for the ligature; the second text
        // object moves to another baseline, which ends its first word.
        assert_eq!(
            words,
            [
                ("ab", 45.0, 68.0),
                ("fic", 70.0, 68.0),
                ("x\u{1f600}", 45.0, 87.0),
                ("\u{fffd}", 45.0, 68.0)
            ]
        );
        // A capture places them in the area the page is clipped to.
        let capture = Capture::of(&pages).expect("the page is clipped");
        let first = &capture.pages[0].words[1];
        assert_eq!(first.text, "fic");
        assert!((first.x - 33.0).abs() < 0.01 && (first.y - 31.0).abs() < 0.01);
        // A page with text and no clip is refused; one with neither, as
        // the reference browser writes a page it draws nothing on, has
        // no area known.
        let unclipped = Page {
            clip: None,
            ..page.clone()
        };
        assert!(Capture::of(&[unclipped]).is_err());
        let blank = Page {
            size: page.size,
            ..Page::default()
        };
        let capture = Capture::of(&[blank]).expect("a blank page captures");
        assert_eq!(capture.pages[0].area, None);
    }

    #[test]
    fn inherited_resources_images_forms_and_simple_fonts_are_read() {
        let content = b"q 100 0 0 50 10 20 cm /Im0 Do Q\nq 1 0 0 1 72 0 cm /Fm0 Do Q";
        let form = b"BT /F1 12 Tf 10 700 Td [(A) -250 (b)] TJ T* (c) ' ET";
        let objects = [
            b"<</Type /Catalog /Pages 2 0 R>>".to_vec(),
            b"<</Type /Pages /Kids [3 0 R 7 0 R] /MediaBox [0 0 200 100] \
              /Resources <</XObject <</Im0 5 0 R /Fm0 6 0 R>> /Font <</F1 8 0 R>>>>>>"
                .to_vec(),
            b"<</Type /Page /Parent 2 0 R /Contents [4 0 R]>>".to_vec(),
            stream("", content),
            stream(" /Subtype /Image /Width 1 /Height 1", b"\xff\xff\xff"),
            stream(" /Subtype /Form /Matrix [1 0 0 1 0 -600]", form),
            b"<</Type /Page /Parent 2 0 R /MediaBox [0 0 72 72]>>".to_vec(),
            b"<</Type /Font /Subtype /Type1 /BaseFont /Helvetica>>".to_vec(),
        ];
        let pages = read(&pdf(&objects)).expect("the file reads");
        assert_eq!(pages.len(), 2);
        let page = &pages[0];
        assert_eq!(page.size.0.round(), 267.0);
        assert_eq!(page.clip, None);
        // The image fills the unit square under the matrix: 100 by 50
        // points with its bottom left corner at (10, 20).
        let image = page.images[0];
        let near = |a: f32, b: f64| (f64::from(a) - b).abs() < 0.01;
        assert!(near(image.x, 10.0 * PX_PER_POINT) && near(image.width, 100.0 * PX_PER_POINT));
        assert!(near(image.y, 30.0 * PX_PER_POINT) && near(image.height, 50.0 * PX_PER_POINT));
        // The form's text, with no leading: one baseline, one word, at
        // the start of the string it starts in.
        assert_eq!(page.words.len(), 1);
        assert_eq!(page.words[0].text, "Abc");
        assert!(near(page.words[0].x, 82.0 * PX_PER_POINT) && near(page.words[0].y, 0.0));
        assert_eq!(page.fonts, ["Helvetica"]);
        assert_eq!(pages[1].size, (96.0, 96.0));
        assert!(pages[1].words.is_empty());
    }

    #[test]
    fn what_is_no_pdf_is_refused() {
        assert!(read(b"not a pdf").is_err());
        assert!(read(&pdf(&["<</Type /Catalog>>"])).is_err());
        // Arrays nested past the limit are refused, not overflowed.
        let deep = format!("<</Type /Catalog /Pages {}>>", "[".repeat(100_000));
        assert!(read(&pdf(&[deep])).is_err());
        let looped = [
            "<</Type /Catalog /Pages 2 0 R>>",
            "<</Type /Pages /Kids [2 0 R]>>",
        ];
        assert!(read(&pdf(&looped)).is_err());
    }

    #[test]
    fn literal_strings_hex_strings_and_names_read_their_escapes() {
        let mut lexer = Lexer::new(b"(a\\(b\\) (c) \\101\\\nd) <41 4> /A#20B 12 0 R -.5 +3");
        let mut next = || lexer.next(0).expect("an object").expect("it reads");
        assert_eq!(next(), Object::String(b"a(b) (c) Ad".to_vec()));
        assert_eq!(next(), Object::String(vec![0x41, 0x40]));
        assert_eq!(next(), Object::Name("A B".into()));
        assert_eq!(next(), Object::Ref(12));
        assert_eq!(next(), Object::Number(-0.5));
        assert_eq!(next(), Object::Number(3.0));
    }

    #[test]
    fn a_capture_writes_reads_and_compares() {
        let area = Rect {
            x: 37.0,
            y: 37.0,
            width: 741.333,
            height: 981.333,
        };
        let word = |text: &str, x: f32, y: f32| Word {
            text: text.into(),
            x,
            y,
        };
        let page = CapturedPage::new(
            (816.0, 1056.0),
            Some(area),
            [
                word(" A\n", 8.0, 31.0),
                word("   ", 8.0, 40.0),
                word("below", 8.0, 990.0),
                word("right", 750.0, 31.0),
                word("next", 8.5, 50.25),
            ],
        );
        assert_eq!(page.words, [word("A", 8.0, 31.0), word("next", 8.5, 50.25)]);
        let capture = Capture { pages: vec![page] };
        let text = capture.to_text("A capture\nof a page");
        assert!(text.starts_with("# A capture\n# of a page\npage 816 1056 37 37 741.33 981.33\n"));
        assert!(
            text.ends_with("word 8 31 A\nword 8.5 50.25 next\n"),
            "{text}"
        );
        let read = Capture::parse(&text).expect("it parses");
        assert!(differences(&capture, &read, 0.01, 0.01).is_empty());
        assert!(Capture::parse("word 1 2 x").is_err());
        assert!(Capture::parse("page 1 2 3").is_err());

        // A page whose area is unknown has no words, and its area is not
        // compared.
        let blank = Capture::parse("page 816 1056\n").expect("it parses");
        assert_eq!(blank.pages[0].area, None);
        assert!(
            differences(
                &blank,
                &Capture {
                    pages: vec![CapturedPage {
                        words: Vec::new(),
                        ..capture.pages[0].clone()
                    }]
                },
                0.0,
                0.0
            )
            .is_empty()
        );
        assert!(!differences(&capture, &blank, 2.0, 0.5).is_empty());

        let mut moved = read.clone();
        moved.pages[0].words[1].y += 3.0;
        moved.pages[0].words[0].text = "B".into();
        moved.pages.push(moved.pages[0].clone());
        assert_eq!(
            differences(&capture, &moved, 2.0, 0.5),
            [
                "2 pages, where 1 were expected",
                "page 1: no \"A\" at (8, 31)",
                "page 1: \"next\" at (8.5, 53.25), not (8.5, 50.25)",
                "page 1: \"B\" at (8, 31) was not expected",
            ]
        );
    }
}
