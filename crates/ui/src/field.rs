//! The address field's model (docs/design/navigation.md, "The address
//! field"): `text::edit`'s since M26.8, which a page's text controls
//! share; the toolbar draws it.

pub use text::edit::{Command, MAX_LEN, TextField, TooLong};
