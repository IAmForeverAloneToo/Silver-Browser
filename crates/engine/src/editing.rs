//! Editing a page's text controls (M26.8, `docs/design/events.md`,
//! "Editing"): the focused text field's or textarea's keys, its text
//! typed, pasted or committed by an input method, cut and copied, each
//! edit a `beforeinput` and an `input`; the caret placed and the
//! selection dragged by the pointer; the caret and the selection drawn
//! over the frame; and a single-line field scrolled to keep the caret in
//! view. The model is `text::edit`'s, which the address field shares.

use dom::NodeId;
use dom::forms::{self, Direction, Selection};
use paint::{DisplayItem, Rect, Step, Viewport, padding_box, paint_order};
use text::edit::{Command, TextField};

use crate::keys::{Key, KeyOutcome};
use crate::{Engine, scripts};

/// The most bytes a text control holds, the document's own limit
/// standing behind it.
const MAX_CONTROL: usize = 1 << 24;

/// The selection's colour over a text control's text: the reference
/// browser's highlight, let through.
const SELECTION_COLOUR: paint::Rgba8 = paint::Rgba8 {
    r: 51,
    g: 144,
    b: 255,
    a: 110,
};

/// Where a text control's text stands in the viewport, and how it is
/// set: its content box, its font size, its line height, its colour.
struct Placed {
    content: Rect,
    size: f32,
    line_height: f32,
    color: paint::Rgba8,
}

impl Engine {
    /// The text control focused, if one is, for the keys and the
    /// clipboard: an `input` or a `textarea` that is not disabled.
    pub(crate) fn text_control(&self) -> Option<NodeId> {
        let document = &self.shown.document;
        document.focused().filter(|&node| {
            forms::is_text_control(document, node) && !forms::actually_disabled(document, node)
        })
    }

    /// Whether the control's value may be edited: not `readonly`.
    fn editable(&self, control: NodeId) -> bool {
        self.shown.document.attr(control, "readonly").is_none()
    }

    /// The control's value and selection as the edit model has them.
    fn field_of(&self, control: NodeId) -> TextField {
        let document = &self.shown.document;
        let value = forms::value(document, control);
        let selection = forms::selection(document, control);
        let (anchor, caret) = match selection.direction {
            Direction::Backward => (selection.end, selection.start),
            _ => (selection.start, selection.end),
        };
        let lines = document.tag(control) == Some("textarea");
        TextField::of(&value, lines, MAX_CONTROL, anchor, caret)
    }

    /// The selection the edit model has, as the document keeps it.
    fn selection_of(field: &TextField) -> Selection {
        let range = field.selection();
        let direction = if range.is_empty() {
            Direction::None
        } else if field.caret() == range.start {
            Direction::Backward
        } else {
            Direction::Forward
        };
        Selection {
            start: range.start,
            end: range.end,
            direction,
        }
    }

    /// Moves the caret or the selection as `command` says: no edit, no
    /// event.
    pub(crate) fn move_in(&mut self, control: NodeId, command: Command) {
        let mut field = self.field_of(control);
        field.apply(command);
        self.select_in(control, &field);
    }

    /// Takes the edit model's selection into the document, the field
    /// scrolled to its caret.
    fn select_in(&mut self, control: NodeId, field: &TextField) {
        let _ = forms::set_selection(&mut self.shown.document, control, Self::selection_of(field));
        self.keep_caret_in_view(control);
        self.state_changed();
    }

    /// An edit of the control as the user makes it (Input Events level 2,
    /// HTML section 4.10.5.5): `apply` run on its model; nothing when that
    /// changes no text, but the selection; else a cancelable
    /// `beforeinput` of `input_type` with `data`, then, unless canceled,
    /// the edit made on the control as it then stands and an `input`.
    /// Whether the text changed.
    fn edit(
        &mut self,
        control: NodeId,
        input_type: &str,
        data: Option<&str>,
        apply: &dyn Fn(&mut TextField),
    ) -> bool {
        let mut field = self.field_of(control);
        let before = field.text().to_owned();
        apply(&mut field);
        if field.text() == before {
            self.select_in(control, &field);
            return false;
        }
        let event = input_event("beforeinput", input_type, data, true);
        if self.fire(js::Target::Node(scripts::handle(control)), event) == Some(true)
            || !self.shown.document.contains(control)
            || !forms::is_text_control(&self.shown.document, control)
        {
            return false;
        }
        let mut field = self.field_of(control);
        apply(&mut field);
        let value = field.text().to_owned();
        if forms::user_edit(
            &mut self.shown.document,
            control,
            value,
            Self::selection_of(&field),
        )
        .is_err()
        {
            return false;
        }
        self.keep_caret_in_view(control);
        self.state_changed();
        let event = input_event("input", input_type, data, false);
        self.fire(js::Target::Node(scripts::handle(control)), event);
        true
    }

    /// `text` inserted at the control's caret in place of its selection,
    /// as typed (`insertText`) or pasted (`insertFromPaste`): cut to what
    /// its `maxlength` lets in, counted in UTF-16 code units.
    fn insert_in(&mut self, control: NodeId, text: &str, input_type: &str) -> bool {
        if !self.editable(control) {
            return false;
        }
        let max = self
            .shown
            .document
            .attr(control, "maxlength")
            .and_then(dom::parse_non_negative_integer);
        let fitted = match max {
            Some(max) => self
                .field_of(control)
                .fitting(text, usize::try_from(max).unwrap_or(usize::MAX)),
            None => text.to_owned(),
        };
        let insert = fitted.clone();
        self.edit(control, input_type, Some(&fitted), &move |field| {
            let _ = field.insert(&insert);
        })
    }

    /// Text typed into the page or committed by an input method (M26.8):
    /// inserted into the text control focused, with its events. Whether
    /// one took it.
    pub fn insert_text(&mut self, text: &str) -> bool {
        match self.text_control() {
            Some(control) => {
                self.insert_in(control, text, "insertText");
                true
            }
            None => false,
        }
    }

    /// Whether a text control has the focus, to paste into (M26.8).
    pub fn can_paste(&self) -> bool {
        self.text_control().is_some()
    }

    /// The text selected in the text control focused, for the clipboard
    /// (M26.8); none in a password field, nor with nothing selected.
    pub fn copy(&self) -> Option<String> {
        let control = self.text_control()?;
        if forms::input_type(&self.shown.document, control) == "password"
            && self.shown.document.tag(control) == Some("input")
        {
            return None;
        }
        let field = self.field_of(control);
        Some(field.selected().to_owned()).filter(|text| !text.is_empty())
    }

    /// [`Engine::copy`], the selection then cut out (`deleteByCut`).
    pub fn cut(&mut self) -> Option<String> {
        let control = self.text_control()?;
        let text = self.copy()?;
        if self.editable(control) {
            self.edit(control, "deleteByCut", None, &|field| {
                field.cut();
            });
        }
        Some(text)
    }

    /// `text` pasted into the text control focused (`insertFromPaste`).
    /// Whether one took it.
    pub fn paste(&mut self, text: &str) -> bool {
        match self.text_control() {
            Some(control) => {
                self.insert_in(control, text, "insertFromPaste");
                true
            }
            None => false,
        }
    }

    /// A key's default action in a text control (M26.8): the moves, by
    /// character, word, line or the whole text, Shift extending; the
    /// deletions, by character or word; Ctrl+A; Enter, a line feed in a
    /// textarea and in a field its change committed. On macOS Option
    /// moves by words and Cmd to the ends. `None` for a key it has no
    /// use for, which may type text.
    pub(crate) fn edit_key(&mut self, control: NodeId, key: &Key) -> Option<KeyOutcome> {
        let m = self.modifiers;
        let mac = cfg!(target_os = "macos");
        let word = if mac { m.alt } else { m.ctrl };
        let command = if mac { m.meta } else { m.ctrl };
        let ends = mac && m.meta;
        let extend = m.shift;
        let lines = self.shown.document.tag(control) == Some("textarea");
        let editable = self.editable(control);
        match key.key.as_str() {
            "a" | "A" if command && !m.alt => self.move_in(control, Command::SelectAll),
            "ArrowLeft" if ends => self.move_in(control, line_start(lines, extend)),
            "ArrowRight" if ends => self.move_in(control, line_end(lines, extend)),
            "ArrowLeft" if word => self.move_in(control, Command::WordLeft { extend }),
            "ArrowRight" if word => self.move_in(control, Command::WordRight { extend }),
            "ArrowLeft" => self.move_in(control, Command::Left { extend }),
            "ArrowRight" => self.move_in(control, Command::Right { extend }),
            "Home" if m.ctrl || !lines => self.move_in(control, Command::Home { extend }),
            "End" if m.ctrl || !lines => self.move_in(control, Command::End { extend }),
            "Home" => self.move_in(control, Command::LineStart { extend }),
            "End" => self.move_in(control, Command::LineEnd { extend }),
            "ArrowUp" | "ArrowDown" if ends => {
                let command = if key.key == "ArrowUp" {
                    Command::Home { extend }
                } else {
                    Command::End { extend }
                };
                self.move_in(control, command);
            }
            "ArrowUp" | "ArrowDown" if lines => {
                self.move_line(control, key.key == "ArrowUp", extend);
            }
            "ArrowUp" | "ArrowDown" => return None,
            "Backspace" if !editable => {}
            "Delete" if !editable => {}
            "Backspace" if word => {
                self.edit(control, "deleteWordBackward", None, &|field| {
                    field.apply(Command::WordBackspace);
                });
            }
            "Delete" if word => {
                self.edit(control, "deleteWordForward", None, &|field| {
                    field.apply(Command::WordDelete);
                });
            }
            "Backspace" => {
                self.edit(control, "deleteContentBackward", None, &|field| {
                    field.apply(Command::Backspace);
                });
            }
            "Delete" => {
                self.edit(control, "deleteContentForward", None, &|field| {
                    field.apply(Command::Delete);
                });
            }
            _ => return None,
        }
        Some(KeyOutcome::Taken)
    }

    /// The text a key types into a text control, after its `keypress`
    /// (M26.8): a line feed for Enter in a textarea, else the key's text
    /// but its control characters, with no shortcut modifier held but as
    /// AltGr. Whether it typed.
    pub(crate) fn type_key(&mut self, control: NodeId, key: &Key) -> bool {
        let m = self.modifiers;
        if (m.ctrl && !m.alt) || m.meta || !self.editable(control) {
            return false;
        }
        if key.key == "Enter" {
            if self.shown.document.tag(control) == Some("textarea") {
                self.edit(control, "insertLineBreak", None, &|field| {
                    let _ = field.insert("\n");
                });
                return true;
            }
            // Enter commits a field's edit (HTML section 4.10.5.5).
            self.commit_change(control);
            return false;
        }
        let Some(text) = key.text.as_deref() else {
            return false;
        };
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if text.is_empty() {
            return false;
        }
        self.insert_in(control, &text, "insertText");
        true
    }

    /// Up or Down in a textarea (M26.8): to the line before or after, as
    /// near the caret's x as its characters stand; at the first line Up
    /// goes to the start, at the last Down to the end. Lines are the
    /// text's line feeds: one wrapped is one line still.
    fn move_line(&mut self, control: NodeId, up: bool, extend: bool) {
        let mut field = self.field_of(control);
        let text = field.text().to_owned();
        let caret = field.caret();
        let line = field.line_around(caret);
        let size = self.placed(control).map_or(13.3333, |placed| placed.size);
        let x = self.layout_fonts.measure(&text[line.start..caret], size);
        let target = if up {
            match line.start.checked_sub(1) {
                Some(end) => field.line_around(end),
                None => {
                    field.place(0, extend);
                    self.select_in(control, &field);
                    return;
                }
            }
        } else {
            if line.end == text.len() {
                field.place(text.len(), extend);
                self.select_in(control, &field);
                return;
            }
            field.line_around(line.end + 1)
        };
        let at = target.start + self.index_at(&text[target.clone()], size, x);
        field.place(at, extend);
        self.select_in(control, &field);
    }

    /// The byte of `text` whose caret stop is nearest `x` at `size`.
    fn index_at(&self, text: &str, size: f32, x: f32) -> usize {
        let stops = self.layout_fonts.caret_stops(text, size);
        let boundaries: Vec<usize> = text
            .char_indices()
            .map(|(at, _)| at)
            .chain(std::iter::once(text.len()))
            .collect();
        let nearest = stops
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
            .map_or(0, |(i, _)| i);
        boundaries.get(nearest).copied().unwrap_or(text.len())
    }

    /// The text a control shows for its value: bullets for a password,
    /// one a character, which the caret stops of are measured on.
    fn shown_value(&self, control: NodeId, value: &str) -> String {
        let document = &self.shown.document;
        if document.tag(control) == Some("input")
            && forms::input_type(document, control) == "password"
        {
            value.chars().map(|_| '\u{2022}').collect()
        } else {
            value.to_owned()
        }
    }

    /// Where a text control's text is in the viewport, from the layout
    /// kept, and how it is set; `None` while it has no box.
    fn placed(&mut self, control: NodeId) -> Option<Placed> {
        let (width, height) = self.viewport;
        if width == 0 {
            return None;
        }
        let (_, offsets) = self.drawn_offsets();
        let scroll_y = self.shown.scroll_y;
        let layout = self.layout(width as f32);
        let whole = Viewport {
            width: width as f32,
            height: layout.height.max(0.0) + height as f32,
            scroll_y: 0.0,
        };
        for step in paint_order(&layout.roots, whole, &offsets) {
            if let Step::Box { fragment, rect, .. } = step
                && fragment.node == control
                && fragment.generated.is_none()
                && matches!(fragment.kind, layout::FragmentKind::Box)
            {
                let padding = padding_box(fragment, rect);
                let pad = |side: usize| match fragment.style.padding[side] {
                    style::computed::LengthPercentage::Px(px) => px,
                    _ => 0.0,
                };
                let content = Rect {
                    x: padding.x + pad(style::computed::LEFT),
                    y: padding.y + pad(style::computed::TOP) - scroll_y,
                    w: (padding.w - pad(style::computed::LEFT) - pad(style::computed::RIGHT))
                        .max(0.0),
                    h: (padding.h - pad(style::computed::TOP) - pad(style::computed::BOTTOM))
                        .max(0.0),
                };
                let size = fragment.style.font_size_px;
                let color = fragment.style.color;
                return Some(Placed {
                    content,
                    size,
                    line_height: self.layout_fonts.primary().metrics(size).line_height(),
                    color: paint::Rgba8 {
                        r: (color.r * 255.0).round() as u8,
                        g: (color.g * 255.0).round() as u8,
                        b: (color.b * 255.0).round() as u8,
                        a: (color.a * 255.0).round() as u8,
                    },
                });
            }
        }
        None
    }

    /// Where the caret's line is drawn in a text control: its byte's x
    /// from the content box's left, the scroll taken off, and its line's
    /// top from the content box's.
    fn caret_at(&self, control: NodeId, value: &str, at: usize, placed: &Placed) -> (f32, f32) {
        let shown = self.shown_value(control, value);
        let line_start = value[..at].rfind('\n').map_or(0, |i| i + 1);
        let line = value[..at].matches('\n').count();
        let chars = value[line_start..at].chars().count();
        let shown_start: usize = shown
            .char_indices()
            .nth(value[..line_start].chars().count())
            .map_or(shown.len(), |(i, _)| i);
        let shown_line_end = shown[shown_start..]
            .find('\n')
            .map_or(shown.len(), |i| shown_start + i);
        let stops = self
            .layout_fonts
            .caret_stops(&shown[shown_start..shown_line_end], placed.size);
        let x = stops.get(chars).copied().unwrap_or(0.0);
        let scroll = self
            .shown
            .document
            .control(control)
            .map_or(0.0, |c| c.scroll as f32);
        let element_scroll = self.element_scroll(control);
        (
            x - scroll,
            line as f32 * placed.line_height - element_scroll,
        )
    }

    /// Scrolls a single-line field's text so that its caret is in view
    /// (M26.8): as little as does it, and never past its text's end; a
    /// textarea, a scroll container, so that the caret's line is.
    fn keep_caret_in_view(&mut self, control: NodeId) {
        if self.shown.document.tag(control) == Some("textarea") {
            self.keep_line_in_view(control);
            return;
        }
        if self.shown.document.tag(control) != Some("input") {
            return;
        }
        let Some(placed) = self.placed(control) else {
            return;
        };
        let value = forms::value(&self.shown.document, control);
        let shown = self.shown_value(control, &value);
        let caret = forms::selection(&self.shown.document, control);
        let at = match caret.direction {
            Direction::Backward => caret.start,
            _ => caret.end,
        };
        let stops = self.layout_fonts.caret_stops(&shown, placed.size);
        let x = stops
            .get(value[..at].chars().count())
            .copied()
            .unwrap_or(0.0);
        let total = stops.last().copied().unwrap_or(0.0);
        let width = placed.content.w.max(1.0);
        let old = self
            .shown
            .document
            .control(control)
            .map_or(0.0, |c| c.scroll as f32);
        let mut scroll = old;
        if x - scroll > width - 1.0 {
            scroll = x - width + 1.0;
        }
        if x < scroll {
            scroll = x;
        }
        scroll = scroll.min((total - width + 1.0).max(0.0)).max(0.0);
        let scroll = scroll.ceil() as u32;
        if scroll != old as u32 {
            let _ = forms::set_scroll(&mut self.shown.document, control, scroll);
        }
    }

    /// Scrolls a textarea as little as brings its caret's line into its
    /// content box (M26.8): its lines being its line feeds.
    fn keep_line_in_view(&mut self, control: NodeId) {
        let Some(placed) = self.placed(control) else {
            return;
        };
        let value = forms::value(&self.shown.document, control);
        let at = forms::selection(&self.shown.document, control).end;
        let top = value[..at.min(value.len())].matches('\n').count() as f32 * placed.line_height;
        let bottom = top + placed.line_height;
        let scroll = self.element_scroll(control);
        let wanted = if top < scroll {
            top
        } else if bottom > scroll + placed.content.h {
            bottom - placed.content.h
        } else {
            return;
        };
        self.set_element_scroll(control, wanted);
    }

    /// The pointer pressed in a text control at viewport `(x, y)`, the
    /// `count`th press of a run (M26.8): the caret placed at the nearest
    /// character boundary, Shift extending the selection; a second press
    /// selects the word, a third the whole text.
    pub(crate) fn press_in(&mut self, control: NodeId, x: f32, y: f32, count: u32) {
        let at = self.index_at_point(control, x, y);
        let mut field = self.field_of(control);
        match count {
            2 => field.select_word_at(at),
            3.. => field.apply(Command::SelectAll),
            _ => field.place(at, self.modifiers.shift),
        }
        self.select_in(control, &field);
        self.shown.selecting = Some(control);
    }

    /// The pointer dragged with the primary button held after a press in
    /// a text control: the selection's caret end follows it.
    pub(crate) fn drag_in(&mut self, x: f32, y: f32) {
        let Some(control) = self.shown.selecting else {
            return;
        };
        if self.shown.document.focused() != Some(control) {
            self.shown.selecting = None;
            return;
        }
        let at = self.index_at_point(control, x, y);
        let mut field = self.field_of(control);
        field.place(at, true);
        self.select_in(control, &field);
    }

    /// The byte of a text control's value nearest the viewport point.
    fn index_at_point(&mut self, control: NodeId, x: f32, y: f32) -> usize {
        let Some(placed) = self.placed(control) else {
            return 0;
        };
        let value = forms::value(&self.shown.document, control);
        let shown = self.shown_value(control, &value);
        let scroll = self
            .shown
            .document
            .control(control)
            .map_or(0.0, |c| c.scroll as f32);
        let local_y = y - placed.content.y + self.element_scroll(control);
        let line = if placed.line_height > 0.0 {
            (local_y / placed.line_height).floor().max(0.0) as usize
        } else {
            0
        };
        let lines: Vec<&str> = value.split('\n').collect();
        let line = line.min(lines.len() - 1);
        let start: usize = lines[..line].iter().map(|l| l.len() + 1).sum();
        let shown_lines: Vec<&str> = shown.split('\n').collect();
        let shown_line = shown_lines.get(line).copied().unwrap_or("");
        let local_x = x - placed.content.x + scroll;
        let index = self.index_at(shown_line, placed.size, local_x);
        // From the shown line's bytes to the value's, by characters.
        let chars = shown_line[..index].chars().count();
        start
            + lines[line]
                .char_indices()
                .nth(chars)
                .map_or(lines[line].len(), |(i, _)| i)
    }

    /// The caret and the selection of the text control focused, drawn
    /// over the frame inside its content box (M26.8): a 1px bar the
    /// text's colour, which does not blink, and a band of the selection
    /// colour over each line's part of the selection. Nothing while the
    /// page does not have the system's focus.
    pub(crate) fn caret_items(&mut self) -> Vec<DisplayItem> {
        let Some(control) = self.text_control() else {
            return Vec::new();
        };
        if !self.window_focused {
            return Vec::new();
        }
        let Some(placed) = self.placed(control) else {
            return Vec::new();
        };
        let value = forms::value(&self.shown.document, control);
        let selection = forms::selection(&self.shown.document, control);
        let c = placed.content;
        let mut items = vec![DisplayItem::PushClip {
            x: c.x,
            y: c.y,
            w: c.w,
            h: c.h,
        }];
        if selection.start < selection.end {
            // One band a line, from the start's x to the end's, or to
            // the line's end where it goes on.
            let mut at = selection.start;
            while at < selection.end {
                let line_end = value[at..]
                    .find('\n')
                    .map_or(value.len(), |i| at + i)
                    .min(selection.end);
                let (x0, y0) = self.caret_at(control, &value, at, &placed);
                let (x1, _) = self.caret_at(control, &value, line_end, &placed);
                let wide = if line_end < selection.end {
                    placed.size * 0.3
                } else {
                    0.0
                };
                items.push(DisplayItem::Rect {
                    x: c.x + x0,
                    y: c.y + y0,
                    w: (x1 - x0 + wide).max(1.0),
                    h: placed.line_height,
                    color: SELECTION_COLOUR,
                });
                at = line_end + 1;
            }
        } else {
            let (x, y) = self.caret_at(control, &value, selection.end, &placed);
            items.push(DisplayItem::Rect {
                x: (c.x + x).floor(),
                y: c.y + y,
                w: 1.0,
                h: placed.line_height,
                color: placed.color,
            });
        }
        items.push(DisplayItem::PopClip);
        items
    }

    /// Where the caret of the text control focused is in the viewport,
    /// `(x, y, w, h)`, for an input method's window to stand by (M26.8).
    pub fn caret_area(&mut self) -> Option<(f32, f32, f32, f32)> {
        let control = self.text_control()?;
        let placed = self.placed(control)?;
        let value = forms::value(&self.shown.document, control);
        let at = forms::selection(&self.shown.document, control).end;
        let (x, y) = self.caret_at(control, &value, at, &placed);
        Some((
            placed.content.x + x,
            placed.content.y + y,
            1.0,
            placed.line_height,
        ))
    }
}

/// The command to a line's start: the text's in a field without lines.
fn line_start(lines: bool, extend: bool) -> Command {
    if lines {
        Command::LineStart { extend }
    } else {
        Command::Home { extend }
    }
}

/// The command to a line's end.
fn line_end(lines: bool, extend: bool) -> Command {
    if lines {
        Command::LineEnd { extend }
    } else {
        Command::End { extend }
    }
}

/// An `InputEvent` of `kind` (Input Events level 2): bubbling and
/// composed, a `beforeinput` cancelable, with its `inputType` and its
/// `data`, null when it has none.
fn input_event(
    kind: &str,
    input_type: &str,
    data: Option<&str>,
    cancelable: bool,
) -> js::EventInit {
    js::EventInit::new("InputEvent", kind)
        .with("bubbles", js::Field::Bool(true))
        .with("cancelable", js::Field::Bool(cancelable))
        .with("composed", js::Field::Bool(true))
        .with("view", js::Field::Window)
        .with("inputType", js::Field::String(input_type.to_owned()))
        .with(
            "data",
            match data {
                Some(data) => js::Field::String(data.to_owned()),
                None => js::Field::Node(None),
            },
        )
        .with("isComposing", js::Field::Bool(false))
}
