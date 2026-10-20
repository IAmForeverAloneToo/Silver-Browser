//! The page's values on the wire (M34.5): the input the browser sends a
//! renderer's view, the settings it is shown under, and the state the
//! renderer reports back for the session to read.

use crate::{
    Button, ColorScheme, ConsoleEntry, ConsoleKind, ConsoleLevel, ConsoleSource, Cursor, Key,
    LoadError, LoadState, Modifiers, Preferences, Scrollbar, StackFrame,
};

use super::codec::{Decoder, Encoder, Malformed, Wire, wire_enum, wire_struct, wire_unit_enum};

wire_unit_enum!(Button {
    0 => Button::Primary,
    1 => Button::Auxiliary,
    2 => Button::Secondary,
    3 => Button::Back,
    4 => Button::Forward,
});

wire_unit_enum!(ColorScheme {
    0 => ColorScheme::Light,
    1 => ColorScheme::Dark,
});

wire_unit_enum!(ConsoleLevel {
    0 => ConsoleLevel::Log,
    1 => ConsoleLevel::Info,
    2 => ConsoleLevel::Warn,
    3 => ConsoleLevel::Error,
    4 => ConsoleLevel::Debug,
});

wire_unit_enum!(Cursor {
    0 => Cursor::Auto,
    1 => Cursor::Default,
    2 => Cursor::None,
    3 => Cursor::ContextMenu,
    4 => Cursor::Help,
    5 => Cursor::Pointer,
    6 => Cursor::Progress,
    7 => Cursor::Wait,
    8 => Cursor::Cell,
    9 => Cursor::Crosshair,
    10 => Cursor::Text,
    11 => Cursor::VerticalText,
    12 => Cursor::Alias,
    13 => Cursor::Copy,
    14 => Cursor::Move,
    15 => Cursor::NoDrop,
    16 => Cursor::NotAllowed,
    17 => Cursor::Grab,
    18 => Cursor::Grabbing,
    19 => Cursor::EResize,
    20 => Cursor::NResize,
    21 => Cursor::NeResize,
    22 => Cursor::NwResize,
    23 => Cursor::SResize,
    24 => Cursor::SeResize,
    25 => Cursor::SwResize,
    26 => Cursor::WResize,
    27 => Cursor::EwResize,
    28 => Cursor::NsResize,
    29 => Cursor::NeswResize,
    30 => Cursor::NwseResize,
    31 => Cursor::ColResize,
    32 => Cursor::RowResize,
    33 => Cursor::AllScroll,
    34 => Cursor::ZoomIn,
    35 => Cursor::ZoomOut,
});

impl Wire for Key {
    fn encode(&self, out: &mut Encoder) {
        out.str(&self.key);
        out.str(&self.code);
        out.u32(self.location);
        out.bool(self.repeat);
        self.text.encode(out);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(Key {
            key: String::decode(input)?,
            code: String::decode(input)?,
            location: input.u32()?,
            repeat: input.bool()?,
            text: Option::decode(input)?,
        })
    }
}

impl Wire for Modifiers {
    fn encode(&self, out: &mut Encoder) {
        out.bool(self.shift);
        out.bool(self.ctrl);
        out.bool(self.alt);
        out.bool(self.meta);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(Modifiers {
            shift: input.bool()?,
            ctrl: input.bool()?,
            alt: input.bool()?,
            meta: input.bool()?,
        })
    }
}

impl Wire for Preferences {
    fn encode(&self, out: &mut Encoder) {
        self.color_scheme.encode(out);
        out.bool(self.reduced_motion);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(Preferences {
            color_scheme: ColorScheme::decode(input)?,
            reduced_motion: input.bool()?,
        })
    }
}

impl Wire for Scrollbar {
    fn encode(&self, out: &mut Encoder) {
        for value in [self.x, self.w, self.h, self.thumb_y, self.thumb_h] {
            value.encode(out);
        }
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(Scrollbar {
            x: f32::decode(input)?,
            w: f32::decode(input)?,
            h: f32::decode(input)?,
            thumb_y: f32::decode(input)?,
            thumb_h: f32::decode(input)?,
        })
    }
}

impl Wire for LoadState {
    fn encode(&self, out: &mut Encoder) {
        match self {
            LoadState::Loading => out.u8(0),
            LoadState::Loaded => out.u8(1),
            LoadState::Failed(LoadError::Network(error)) => {
                out.u8(2);
                error.encode(out);
            }
            LoadState::Failed(LoadError::Lost) => out.u8(3),
            LoadState::Failed(LoadError::Framed) => out.u8(4),
        }
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(match input.u8()? {
            0 => LoadState::Loading,
            1 => LoadState::Loaded,
            2 => LoadState::Failed(LoadError::Network(net::NetworkError::decode(input)?)),
            3 => LoadState::Failed(LoadError::Lost),
            4 => LoadState::Failed(LoadError::Framed),
            tag => return Err(Malformed::Tag("LoadState", u32::from(tag))),
        })
    }
}

impl Wire for ConsoleEntry {
    fn encode(&self, out: &mut Encoder) {
        self.level.encode(out);
        out.str(&self.text);
        self.source.encode(out);
        self.kind.encode(out);
        self.time.encode(out);
        out.str(&self.url);
        self.line.encode(out);
        self.column.encode(out);
        self.stack.encode(out);
        out.str(&self.previews);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(ConsoleEntry {
            level: ConsoleLevel::decode(input)?,
            text: String::decode(input)?,
            source: ConsoleSource::decode(input)?,
            kind: ConsoleKind::decode(input)?,
            time: u64::decode(input)?,
            url: String::decode(input)?,
            line: u32::decode(input)?,
            column: u32::decode(input)?,
            stack: Vec::decode(input)?,
            previews: String::decode(input)?,
        })
    }
}

wire_unit_enum!(ConsoleSource {
    0 => ConsoleSource::Console,
    1 => ConsoleSource::Exception,
    2 => ConsoleSource::Network,
    3 => ConsoleSource::Policy,
    4 => ConsoleSource::Worker,
    5 => ConsoleSource::Engine,
});

wire_unit_enum!(ConsoleKind {
    0 => ConsoleKind::Log,
    1 => ConsoleKind::Dir,
    2 => ConsoleKind::Table,
    3 => ConsoleKind::Trace,
    4 => ConsoleKind::Group,
    5 => ConsoleKind::GroupCollapsed,
    6 => ConsoleKind::GroupEnd,
    7 => ConsoleKind::Clear,
});

impl Wire for StackFrame {
    fn encode(&self, out: &mut Encoder) {
        out.str(&self.function);
        out.str(&self.url);
        self.line.encode(out);
        self.column.encode(out);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(StackFrame {
            function: String::decode(input)?,
            url: String::decode(input)?,
            line: u32::decode(input)?,
            column: u32::decode(input)?,
        })
    }
}

wire_struct! {
    /// What a view is shown under: the session's settings, sent whole
    /// when one changes.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Settings {
        pub preferences: Preferences,
        pub scripting: bool,
        pub compat_user_agent: bool,
        pub hidden: bool,
        pub focused: bool,
        pub scale: f32,
        pub modifiers: Modifiers,
        pub layer_budget: Option<u64>,
    }
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            preferences: Preferences::default(),
            scripting: false,
            compat_user_agent: false,
            hidden: false,
            focused: false,
            scale: 1.0,
            modifiers: Modifiers::default(),
            layer_budget: None,
        }
    }
}

wire_enum! {
    /// Input given to a view, in its CSS pixels.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Input {
        0 => PointerMove { x: f32, y: f32 },
        1 => PointerLeave {},
        2 => PointerDown { x: f32, y: f32, button: Button, count: u32 },
        3 => PointerUp { x: f32, y: f32, button: Button, count: u32 },
        4 => Wheel { x: f32, y: f32, dy: f32 },
        5 => KeyDown { key: Key },
        6 => KeyUp { key: Key },
        7 => FocusNext { backward: bool },
        8 => InsertText { text: String },
        9 => Paste { text: String },
        10 => ScrollBy { dy: f32 },
        11 => ScrollToFraction { fraction: f32 },
    }
}

wire_struct! {
    /// What the session reads of a view, as its renderer last reported
    /// it.
    #[derive(Debug, Clone, PartialEq)]
    pub struct ViewState {
        pub url: Option<net::Url>,
        pub title: String,
        pub load: LoadState,
        /// The name of the encoding its document is in that cannot be
        /// decoded, if so.
        pub undecodable: Option<String>,
        pub can_go_back: bool,
        pub can_go_forward: bool,
        pub popup: bool,
        pub can_paste: bool,
        pub caret: Option<(f32, f32, f32, f32)>,
        pub scrollbar: Option<Scrollbar>,
        /// The cursor and the link at the pointer's last place.
        pub cursor: Cursor,
        pub hover: Option<net::Url>,
    }
}

impl Default for ViewState {
    fn default() -> ViewState {
        ViewState {
            url: None,
            title: String::new(),
            load: LoadState::Loading,
            undecodable: None,
            can_go_back: false,
            can_go_forward: false,
            popup: false,
            can_paste: false,
            caret: None,
            scrollbar: None,
            cursor: Cursor::Default,
            hover: None,
        }
    }
}
