//! A select's choices (M26.8, `docs/design/events.md`, "The select's
//! list"): a drop-down's list of options, drawn by the engine over the
//! page below the control (above, with no room), opened by a press or by
//! Alt+Down, F4 or Space, its rows chosen by the pointer or the keys; a
//! closed drop-down's choice changed by the arrows; a list box's options
//! chosen by presses and the arrows. A choice changed is an `input` and a
//! `change` at the select.

use std::collections::HashMap;

use dom::NodeId;
use dom::forms;
use paint::{DisplayItem, Rect, Rgba8, Step, Viewport, paint_order};

use crate::keys::{Key, KeyOutcome};
use crate::{Engine, scripts};

/// The most rows a drop-down's list shows at once.
const MAX_ROWS: usize = 20;
/// The space either side of a row's label, in px.
const ROW_PAD: f32 = 4.0;

const LIST_BACKGROUND: Rgba8 = Rgba8 {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};
const LIST_EDGE: Rgba8 = Rgba8 {
    r: 118,
    g: 118,
    b: 118,
    a: 255,
};
/// The row the pointer or the keys are on: the reference browser's
/// highlight, its text white.
const HIGHLIGHT: Rgba8 = Rgba8 {
    r: 25,
    g: 103,
    b: 210,
    a: 255,
};
const ROW_TEXT: Rgba8 = Rgba8 {
    r: 0,
    g: 0,
    b: 0,
    a: 255,
};
const DISABLED_TEXT: Rgba8 = Rgba8 {
    r: 109,
    g: 109,
    b: 109,
    a: 255,
};

/// A drop-down's list while it is open.
#[derive(Debug, Clone)]
pub(crate) struct Popup {
    select: NodeId,
    options: Vec<NodeId>,
    /// The row the pointer or the keys are on.
    highlighted: Option<usize>,
    /// The first row shown, when there are more than [`MAX_ROWS`].
    top: usize,
    /// The list's box in the viewport, and a row's height.
    rect: Rect,
    row: f32,
    size: f32,
    /// Whether the press that opened it is still held.
    held: bool,
}

impl Popup {
    /// The list over a copy of the document, its nodes found in
    /// `places` (the differential check's snapshot, M26.10): none when
    /// one is not there.
    pub(crate) fn mapped(&self, places: &HashMap<NodeId, NodeId>) -> Option<Popup> {
        Some(Popup {
            select: *places.get(&self.select)?,
            options: self
                .options
                .iter()
                .map(|option| places.get(option).copied())
                .collect::<Option<_>>()?,
            ..self.clone()
        })
    }

    /// The row at the viewport point, if it is in the list.
    fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        let r = self.rect;
        if x < r.x || x >= r.x + r.w || y < r.y || y >= r.y + r.h {
            return None;
        }
        let row = self.top + ((y - r.y - 1.0) / self.row).floor().max(0.0) as usize;
        (row < self.options.len()).then_some(row)
    }
}

impl Engine {
    /// The select whose list is open, if any.
    pub fn popup_open(&self) -> Option<NodeId> {
        // A frame's list, the frame focused (M33.2).
        let frame = || {
            let node = self.focused_frame()?;
            self.shown.frames.get(&node)?.engine.popup_open()
        };
        self.shown
            .popup
            .as_ref()
            .map(|popup| popup.select)
            .or_else(frame)
    }

    /// The border box of an element in the viewport, from the layout
    /// kept, with its font size.
    fn border_box(&mut self, node: NodeId) -> Option<(Rect, f32)> {
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
        paint_order(&layout.roots, whole, &offsets)
            .into_iter()
            .find_map(|step| match step {
                Step::Box { fragment, rect, .. }
                    if fragment.node == node
                        && fragment.generated.is_none()
                        && matches!(fragment.kind, layout::FragmentKind::Box) =>
                {
                    Some((
                        Rect {
                            y: rect.y - scroll_y,
                            ..rect
                        },
                        fragment.style.font_size_px,
                    ))
                }
                _ => None,
            })
    }

    /// Opens a drop-down's list below it, or above when the viewport has
    /// no room below, as wide as the select or its widest label, the
    /// chosen option highlighted (M26.8).
    pub(crate) fn open_popup(&mut self, select: NodeId, held: bool) {
        let Some((select_box, size)) = self.border_box(select) else {
            return;
        };
        let options = forms::list_of_options(&self.shown.document, select);
        if options.is_empty() {
            return;
        }
        let metrics = self.layout_fonts.primary().metrics(size);
        let row = (metrics.line_height() + 2.0).ceil();
        let widest = options
            .iter()
            .map(|&option| {
                let label = forms::option_label(&self.shown.document, option);
                self.layout_fonts.measure(&label, size)
            })
            .fold(0.0f32, f32::max);
        let rows = options.len().min(MAX_ROWS);
        let w = select_box.w.max(widest + 2.0 * ROW_PAD + 2.0);
        let h = rows as f32 * row + 2.0;
        let below = select_box.y + select_box.h;
        let y = if below + h > self.viewport.1 as f32 && select_box.y - h >= 0.0 {
            select_box.y - h
        } else {
            below
        };
        let highlighted = forms::selected_index(&self.shown.document, select);
        let top = highlighted.map_or(0, |at| at.saturating_sub(rows - 1));
        self.shown.popup = Some(Popup {
            select,
            options,
            highlighted,
            top,
            rect: Rect {
                x: select_box.x,
                y,
                w,
                h,
            },
            row,
            size,
            held,
        });
        self.changed = true;
    }

    /// Closes the list, choosing nothing.
    pub(crate) fn close_popup(&mut self) {
        if self.shown.popup.take().is_some() {
            self.changed = true;
        }
    }

    /// The option at `index` of a select's list chosen, as the user
    /// chooses (M26.8): selected, the rest not in a select without
    /// `multiple`, and when that changed the choice, `input` and then
    /// `change` at the select. A disabled option is not chosen.
    fn choose(&mut self, select: NodeId, index: usize) {
        let options = forms::list_of_options(&self.shown.document, select);
        let Some(&option) = options.get(index) else {
            return;
        };
        if forms::option_disabled(&self.shown.document, option) {
            return;
        }
        let document = &self.shown.document;
        let multiple = document.attr(select, "multiple").is_some();
        let changed = if multiple {
            options
                .iter()
                .enumerate()
                .any(|(i, &other)| forms::selectedness(document, other) != (i == index))
        } else {
            forms::selected_index(document, select) != Some(index)
        };
        if !changed {
            return;
        }
        let _ = forms::set_selectedness(&mut self.shown.document, option, true, true);
        if multiple {
            // A press without Ctrl chooses this one alone.
            for (i, &other) in options.iter().enumerate() {
                if i != index && forms::selectedness(&self.shown.document, other) {
                    let _ = forms::set_selectedness(&mut self.shown.document, other, false, true);
                }
            }
        }
        self.state_changed();
        self.choice_changed(select);
    }

    /// `input` and `change` at a select whose choice the user changed,
    /// its user validity set (M26.9b).
    fn choice_changed(&mut self, select: NodeId) {
        let _ = dom::validity::set_user_validity(&mut self.shown.document, select, true);
        for (kind, composed) in [("input", true), ("change", false)] {
            if !self.shown.document.contains(select) {
                return;
            }
            let event = js::EventInit::new("Event", kind)
                .with("bubbles", js::Field::Bool(true))
                .with("composed", js::Field::Bool(composed));
            self.fire(js::Target::Node(scripts::handle(select)), event);
        }
    }

    /// The row after or before `from` that is not disabled, `step` rows
    /// on, or the first or last such for none.
    fn next_row(&self, options: &[NodeId], from: Option<usize>, forward: bool) -> Option<usize> {
        let document = &self.shown.document;
        let usable = |i: &usize| !forms::option_disabled(document, options[*i]);
        match (from, forward) {
            (Some(at), true) => (at + 1..options.len()).find(usable).or(Some(at)),
            (Some(at), false) => (0..at).rev().find(usable).or(Some(at)),
            (None, true) => (0..options.len()).find(usable),
            (None, false) => (0..options.len()).rev().find(usable),
        }
    }

    /// A key while the list is open (M26.8): the arrows, Home and End
    /// move the highlight, Enter or Space choose it, Escape and Tab close
    /// the list. The page hears none of them, as with the reference
    /// browser's list, which is a window of its own.
    pub(crate) fn popup_key(&mut self, key: &Key) -> KeyOutcome {
        let Some(mut popup) = self.shown.popup.clone() else {
            return KeyOutcome::Ignored;
        };
        let options = popup.options.clone();
        match key.key.as_str() {
            "ArrowDown" => popup.highlighted = self.next_row(&options, popup.highlighted, true),
            "ArrowUp" => popup.highlighted = self.next_row(&options, popup.highlighted, false),
            "Home" => popup.highlighted = self.next_row(&options, None, true),
            "End" => popup.highlighted = self.next_row(&options, None, false),
            "Enter" | " " => {
                self.close_popup();
                if let Some(at) = popup.highlighted {
                    self.choose(popup.select, at);
                }
                return KeyOutcome::Taken;
            }
            "Escape" | "Tab" => {
                self.close_popup();
                return KeyOutcome::Taken;
            }
            _ => return KeyOutcome::Taken,
        }
        if let Some(at) = popup.highlighted {
            let rows = options.len().min(MAX_ROWS);
            if at < popup.top {
                popup.top = at;
            } else if at >= popup.top + rows {
                popup.top = at + 1 - rows;
            }
        }
        self.shown.popup = Some(popup);
        self.changed = true;
        KeyOutcome::Taken
    }

    /// A key at a focused select whose list is closed (M26.8): Alt+Down,
    /// Alt+Up, F4 or Space opens a drop-down's list; the arrows, Home and
    /// End change a drop-down's or a single list box's choice at once.
    /// `None` for a key it has no use for.
    pub(crate) fn select_key(&mut self, select: NodeId, key: &Key) -> Option<KeyOutcome> {
        let document = &self.shown.document;
        let list_box = forms::is_list_box(document, select);
        let m = self.modifiers;
        let opens = match key.key.as_str() {
            "ArrowDown" | "ArrowUp" => m.alt,
            "F4" => true,
            " " => !m.ctrl && !m.meta,
            _ => false,
        };
        if opens && !list_box {
            self.open_popup(select, false);
            return Some(KeyOutcome::Taken);
        }
        if m.ctrl || m.alt || m.meta {
            return None;
        }
        let options = forms::list_of_options(document, select);
        let at = forms::selected_index(document, select);
        let to = match key.key.as_str() {
            "ArrowDown" | "ArrowRight" if !list_box || key.key == "ArrowDown" => {
                self.next_row(&options, at, true)
            }
            "ArrowUp" | "ArrowLeft" if !list_box || key.key == "ArrowUp" => {
                self.next_row(&options, at, false)
            }
            "Home" => self.next_row(&options, None, true),
            "End" => self.next_row(&options, None, false),
            _ => return None,
        };
        if let Some(to) = to
            && Some(to) != at
        {
            self.choose(select, to);
        }
        Some(KeyOutcome::Taken)
    }

    /// The pointer pressed while the list is open (M26.8): in the list,
    /// nothing until the release; elsewhere, the list closes, and the
    /// press goes no further. Whether the list took the press.
    pub(crate) fn popup_press(&mut self, x: f32, y: f32) -> bool {
        let Some(popup) = &self.shown.popup else {
            return false;
        };
        if popup.row_at(x, y).is_none() {
            self.close_popup();
        }
        true
    }

    /// The pointer released while the list is open: on a row, it is
    /// chosen and the list closes, and the page hears nothing of it; the
    /// release of the press that opened the list, elsewhere, leaves it
    /// open and goes on to the page. Whether the list took it.
    pub(crate) fn popup_release(&mut self, x: f32, y: f32) -> bool {
        let Some(popup) = self.shown.popup.as_mut() else {
            return false;
        };
        let row = popup.row_at(x, y);
        popup.held = false;
        let select = popup.select;
        match row {
            Some(at) => {
                self.close_popup();
                self.choose(select, at);
                true
            }
            None => false,
        }
    }

    /// The pointer moved while the list is open: the row under it is
    /// highlighted. Whether the list took the move.
    pub(crate) fn popup_move(&mut self, x: f32, y: f32) -> bool {
        let Some(popup) = self.shown.popup.as_mut() else {
            return false;
        };
        if let Some(at) = popup.row_at(x, y)
            && popup.highlighted != Some(at)
            && !forms::option_disabled(&self.shown.document, popup.options[at])
        {
            popup.highlighted = Some(at);
            self.changed = true;
        }
        true
    }

    /// A press on an option of a list box (M26.8): chosen, the rest not
    /// unless the select has `multiple` and Ctrl is held, which toggles
    /// the option instead.
    pub(crate) fn press_option(&mut self, select: NodeId, option: NodeId) {
        let options = forms::list_of_options(&self.shown.document, select);
        let Some(index) = options.iter().position(|&o| o == option) else {
            return;
        };
        let multiple = self.shown.document.attr(select, "multiple").is_some();
        if multiple && (self.modifiers.ctrl || self.modifiers.meta) {
            if forms::option_disabled(&self.shown.document, option) {
                return;
            }
            let on = forms::selectedness(&self.shown.document, option);
            let _ = forms::set_selectedness(&mut self.shown.document, option, !on, true);
            self.state_changed();
            self.choice_changed(select);
            return;
        }
        self.choose(select, index);
    }

    /// The open list, drawn over the frame (M26.8): a white box with a
    /// grey edge, a row a label, the highlighted row in the highlight's
    /// colour, a disabled option's label grey.
    pub(crate) fn popup_items(&mut self) -> Vec<DisplayItem> {
        let Some(popup) = self.shown.popup.clone() else {
            return Vec::new();
        };
        if !self.shown.document.contains(popup.select) {
            self.shown.popup = None;
            return Vec::new();
        }
        let r = popup.rect;
        let mut items = vec![
            DisplayItem::Rect {
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
                color: LIST_BACKGROUND,
            },
            DisplayItem::Border {
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
                widths: [1.0; 4],
                color: LIST_EDGE,
            },
            DisplayItem::PushClip {
                x: r.x + 1.0,
                y: r.y + 1.0,
                w: r.w - 2.0,
                h: r.h - 2.0,
            },
        ];
        let metrics = self.layout_fonts.primary().metrics(popup.size);
        let rows = popup.options.len().min(MAX_ROWS);
        for (shown, at) in (popup.top..popup.top + rows).enumerate() {
            let Some(&option) = popup.options.get(at) else {
                break;
            };
            let y = r.y + 1.0 + shown as f32 * popup.row;
            let lit = popup.highlighted == Some(at);
            if lit {
                items.push(DisplayItem::Rect {
                    x: r.x + 1.0,
                    y,
                    w: r.w - 2.0,
                    h: popup.row,
                    color: HIGHLIGHT,
                });
            }
            let disabled = forms::option_disabled(&self.shown.document, option);
            let color = match (lit, disabled) {
                (_, true) => DISABLED_TEXT,
                (true, false) => LIST_BACKGROUND,
                (false, false) => ROW_TEXT,
            };
            let baseline = (y + (popup.row - metrics.line_height()) / 2.0 + metrics.ascent).round();
            items.push(DisplayItem::Text {
                text: forms::option_label(&self.shown.document, option),
                x: r.x + 1.0 + ROW_PAD,
                y: baseline,
                size: popup.size,
                color,
            });
        }
        items.push(DisplayItem::PopClip);
        items
    }
}
