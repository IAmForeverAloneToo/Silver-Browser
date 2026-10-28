//! A page's `alert()`, `confirm()` or `prompt()` (gap batch A, D43,
//! docs/design/scripting-essentials.md): a sheet under the toolbar, over
//! the top of the page, saying which origin asks and its message, with
//! `prompt()`'s text field and the OK and Cancel buttons at its foot. The
//! shell puts its pixels over the page's, and gives it the keys and the
//! pointer's presses while it is open.

use paint::{DisplayItem, DisplayList, Rgba8};
use render::{RenderBackend as _, SoftwareBackend};
use text::FontCollection;

use crate::field::{Command, TextField};

const PADDING: f32 = 12.0;
const FONT_SIZE: f32 = 13.0;
const LINE: f32 = 18.0;
/// The most lines of the message shown; the last cut short says so.
pub const MAX_LINES: usize = 8;
const BUTTON_W: f32 = 72.0;
const BUTTON_H: f32 = 26.0;
const FIELD_H: f32 = 26.0;
const GAP: f32 = 8.0;
const SHEET: Rgba8 = Rgba8::rgb(0xFF, 0xFF, 0xFF);
const EDGE: Rgba8 = Rgba8::rgb(0xC9, 0xC9, 0xC9);
const BUTTON: Rgba8 = Rgba8::rgb(0xF2, 0xF2, 0xF2);
const DEFAULT_BUTTON: Rgba8 = Rgba8::rgb(0x1A, 0x73, 0xE8);
const BUTTON_EDGE: Rgba8 = Rgba8::rgb(0xB8, 0xB8, 0xB8);
const TEXT: Rgba8 = Rgba8::rgb(0x1F, 0x1F, 0x1F);
const DEFAULT_TEXT: Rgba8 = Rgba8::rgb(0xFF, 0xFF, 0xFF);
const SELECTION: Rgba8 = Rgba8::rgb(0xB4, 0xD2, 0xFA);
/// What stands for the end of a text cut short.
const ELLIPSIS: &str = "...";

/// Which of the three a page called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt,
}

/// What a press in the sheet is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogPart {
    Ok,
    Cancel,
    Field,
}

/// The dialog's sheet: the one open, if any, and what was typed in it.
pub struct DialogSheet {
    fonts: FontCollection,
    backend: SoftwareBackend,
    open: Option<Open>,
}

struct Open {
    id: u64,
    kind: DialogKind,
    title: String,
    lines: Vec<String>,
    field: TextField,
}

impl DialogSheet {
    pub fn new(fonts: FontCollection) -> DialogSheet {
        DialogSheet {
            backend: SoftwareBackend::with_fonts(fonts.clone()),
            fonts,
            open: None,
        }
    }

    /// Opens dialog `id`, unless it is the one open: `origin` asks,
    /// saying `message`, `prompt()`'s field holding `default`, all of it
    /// selected.
    pub fn open(&mut self, id: u64, kind: DialogKind, origin: &str, message: &str, default: &str) {
        if self.id() == Some(id) {
            return;
        }
        let mut field = TextField::new();
        field.set_text(default);
        field.select_all();
        let mut lines: Vec<String> = message.split('\n').map(str::to_owned).collect();
        if lines.len() > MAX_LINES {
            lines.truncate(MAX_LINES);
            lines[MAX_LINES - 1].push_str(ELLIPSIS);
        }
        if message.is_empty() {
            lines.clear();
        }
        self.open = Some(Open {
            id,
            kind,
            title: format!("{origin} says"),
            lines,
            field,
        });
    }

    /// Closes the dialog open.
    pub fn close(&mut self) {
        self.open = None;
    }

    /// The dialog open's number.
    pub fn id(&self) -> Option<u64> {
        self.open.as_ref().map(|open| open.id)
    }

    /// What `prompt()`'s field holds.
    pub fn value(&self) -> Option<&str> {
        self.open
            .as_ref()
            .filter(|open| open.kind == DialogKind::Prompt)
            .map(|open| open.field.text())
    }

    /// Edits `prompt()`'s field, or moves in it.
    pub fn command(&mut self, command: Command) {
        if let Some(open) = self
            .open
            .as_mut()
            .filter(|open| open.kind == DialogKind::Prompt)
        {
            open.field.apply(command);
        }
    }

    /// Text typed or pasted into `prompt()`'s field; what does not fit
    /// is dropped.
    pub fn insert(&mut self, text: &str) {
        if let Some(open) = self
            .open
            .as_mut()
            .filter(|open| open.kind == DialogKind::Prompt)
        {
            let fitting = open.field.fitting(text, crate::field::MAX_LEN);
            let _ = open.field.insert(&fitting);
        }
    }

    /// The sheet's height in CSS pixels; none while no dialog is open.
    pub fn height(&self) -> u32 {
        let Some(open) = &self.open else {
            return 0;
        };
        let field = match open.kind {
            DialogKind::Prompt => FIELD_H + GAP,
            _ => 0.0,
        };
        let text = LINE * (1 + open.lines.len()) as f32;
        (PADDING + text + GAP + field + BUTTON_H + PADDING) as u32
    }

    /// The buttons there are, from the right.
    fn buttons(&self) -> &'static [DialogPart] {
        match self.open.as_ref().map(|open| open.kind) {
            Some(DialogKind::Alert) => &[DialogPart::Ok],
            Some(_) => &[DialogPart::Cancel, DialogPart::Ok],
            None => &[],
        }
    }

    /// The left edge of the `index`th button from the right, in a sheet
    /// `width` wide, and their top.
    fn button_at(&self, index: usize, width: f32) -> (f32, f32) {
        let x = width - PADDING - BUTTON_W - index as f32 * (BUTTON_W + GAP);
        (x, self.height() as f32 - PADDING - BUTTON_H)
    }

    /// The field's top.
    fn field_top(&self) -> f32 {
        self.height() as f32 - PADDING - BUTTON_H - GAP - FIELD_H
    }

    /// What is at `(x, y)`, in the sheet's own CSS pixels, in a sheet
    /// `width` CSS pixels wide.
    pub fn part_at(&self, width: f32, x: f32, y: f32) -> Option<DialogPart> {
        let open = self.open.as_ref()?;
        for (index, &part) in self.buttons().iter().enumerate() {
            let (left, top) = self.button_at(index, width);
            if (left..left + BUTTON_W).contains(&x) && (top..top + BUTTON_H).contains(&y) {
                return Some(part);
            }
        }
        let top = self.field_top();
        (open.kind == DialogKind::Prompt
            && (PADDING..width - PADDING).contains(&x)
            && (top..top + FIELD_H).contains(&y))
        .then_some(DialogPart::Field)
    }

    /// `text` as it fits in `room` pixels: all of it, or its start and an
    /// ellipsis.
    fn fit(&self, text: &str, room: f32) -> String {
        if self.fonts.measure(text, FONT_SIZE) <= room {
            return text.to_owned();
        }
        let room = room - self.fonts.measure(ELLIPSIS, FONT_SIZE);
        let stops = self.fonts.caret_stops(text, FONT_SIZE);
        let fits = stops.iter().take_while(|&&x| x <= room).count().max(1) - 1;
        let end = text.char_indices().nth(fits).map_or(text.len(), |(i, _)| i);
        format!("{}{ELLIPSIS}", &text[..end])
    }

    /// Draws the sheet `width` device pixels wide at the display's
    /// `scale`: its RGBA8 pixels, [`DialogSheet::height`] CSS pixels
    /// high, lent until the next frame; nothing while no dialog is open.
    pub fn frame(&mut self, width: u32, scale: f32) -> &[u8] {
        let Some(open) = &self.open else {
            return &[];
        };
        let w = width as f32 / scale;
        let h = self.height() as f32;
        let metrics = self.fonts.primary().metrics(FONT_SIZE);
        let baseline =
            |top: f32, height: f32| top + (height + metrics.ascent - metrics.descent) / 2.0;
        let room = (w - 2.0 * PADDING).max(0.0);
        let mut items = vec![
            DisplayItem::Rect {
                x: 0.0,
                y: 0.0,
                w,
                h,
                color: EDGE,
            },
            DisplayItem::Rect {
                x: 0.0,
                y: 0.0,
                w,
                h: h - 1.0,
                color: SHEET,
            },
        ];
        for (index, line) in std::iter::once(&open.title).chain(&open.lines).enumerate() {
            items.push(DisplayItem::Text {
                text: self.fit(line, room),
                x: PADDING,
                y: baseline(PADDING + index as f32 * LINE, LINE),
                size: FONT_SIZE,
                color: TEXT,
            });
        }
        if open.kind == DialogKind::Prompt {
            let top = self.field_top();
            items.extend([
                DisplayItem::Rect {
                    x: PADDING,
                    y: top,
                    w: room,
                    h: FIELD_H,
                    color: DEFAULT_BUTTON,
                },
                DisplayItem::Rect {
                    x: PADDING + 1.0,
                    y: top + 1.0,
                    w: (room - 2.0).max(0.0),
                    h: FIELD_H - 2.0,
                    color: SHEET,
                },
            ]);
            // The field's text from its start, scrolled to keep the
            // caret in view.
            let text = open.field.text();
            let stops = self.fonts.caret_stops(text, FONT_SIZE);
            let x_of = |index: usize| {
                let chars = text[..index].chars().count();
                stops.get(chars).copied().unwrap_or(0.0)
            };
            let inner = (room - 2.0 * GAP).max(0.0);
            let scroll = (x_of(open.field.caret()) - inner).max(0.0);
            let origin = PADDING + GAP - scroll;
            let selection = open.field.selection();
            if !selection.is_empty() {
                let (start, end) = (x_of(selection.start), x_of(selection.end));
                items.push(DisplayItem::Rect {
                    x: origin + start,
                    y: top + 4.0,
                    w: end - start,
                    h: FIELD_H - 8.0,
                    color: SELECTION,
                });
            }
            items.push(DisplayItem::Text {
                text: text.to_owned(),
                x: origin,
                y: baseline(top, FIELD_H),
                size: FONT_SIZE,
                color: TEXT,
            });
            if selection.is_empty() {
                items.push(DisplayItem::Rect {
                    x: (origin + x_of(open.field.caret())).round(),
                    y: top + 4.0,
                    w: 1.0,
                    h: FIELD_H - 8.0,
                    color: TEXT,
                });
            }
        }
        for (index, &part) in self.buttons().iter().enumerate() {
            let (x, top) = self.button_at(index, w);
            let label = match part {
                DialogPart::Cancel => "Cancel",
                _ => "OK",
            };
            let (fill, ink) = match part {
                DialogPart::Ok => (DEFAULT_BUTTON, DEFAULT_TEXT),
                _ => (BUTTON, TEXT),
            };
            let inset = (BUTTON_W - self.fonts.measure(label, FONT_SIZE)) / 2.0;
            items.extend([
                DisplayItem::Rect {
                    x,
                    y: top,
                    w: BUTTON_W,
                    h: BUTTON_H,
                    color: BUTTON_EDGE,
                },
                DisplayItem::Rect {
                    x: x + 1.0,
                    y: top + 1.0,
                    w: BUTTON_W - 2.0,
                    h: BUTTON_H - 2.0,
                    color: fill,
                },
                DisplayItem::Text {
                    text: label.to_owned(),
                    x: x + inset,
                    y: baseline(top, BUTTON_H),
                    size: FONT_SIZE,
                    color: ink,
                },
            ]);
        }
        let height = self.height();
        let mut list = DisplayList { items };
        paint::scaled(&mut list.items, scale, paint::GroupKey::Document(0));
        self.backend
            .rasterize(&list, width.max(1), crate::rows(height, scale))
    }
}
