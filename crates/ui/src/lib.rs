//! The browser's own interface (M15, docs/design/navigation.md, "Where
//! the code goes"): the toolbar with its address field and buttons, the
//! status bubble, and the window's title. It draws with the engine's own
//! parts — `paint`'s display list, `render`'s rasterizer and `text`'s
//! shaping — and touches no OS: the shell gives it input and shows its
//! pixels, and gives it what it shows of the page.

pub mod field;
pub mod status;
mod title;
pub mod toolbar;

pub use field::{Command, TextField, TooLong};
pub use status::StatusBubble;
pub use title::window_title;
pub use toolbar::{Action, Cursor, Escape, HEIGHT, PageState, Part, Toolbar};

#[cfg(test)]
mod tests;
