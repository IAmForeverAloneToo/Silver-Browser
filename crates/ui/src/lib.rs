//! The browser's own interface (M15, docs/design/navigation.md, "Where
//! the code goes"): the toolbar with its address field and buttons, the
//! status bubble, and the window's title. It draws with the engine's own
//! parts — `paint`'s display list, `render`'s rasterizer and `text`'s
//! shaping — and touches no OS: the shell gives it input and shows its
//! pixels, and gives it what it shows of the page.
//!
//! The permission prompt (M33.4) is drawn and given input the same way.

#![forbid(unsafe_code)]

pub mod field;
pub mod prompt;
pub mod status;
mod title;
pub mod toolbar;

pub use field::{Command, TextField, TooLong};
pub use prompt::{Choice, PromptBar};
pub use status::StatusBubble;
pub use title::window_title;
pub use toolbar::{Action, Cursor, Escape, HEIGHT, PageState, Part, Toolbar};

/// The device pixels `css` CSS pixels take at the display's `scale`
/// (M32.4), rounded up: the rows a bar of that height is drawn in.
pub fn rows(css: u32, scale: f32) -> u32 {
    (css as f32 * scale).ceil() as u32
}

#[cfg(test)]
mod tests;
