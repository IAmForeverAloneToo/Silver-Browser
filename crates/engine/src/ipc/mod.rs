//! The messages between the browser process and its renderers (M34.2,
//! docs/design/processes.md, "IPC"): a codec of our own (no
//! serialization crate is in the tree), the messages each way, and the
//! channel they go over, two byte streams that a process's standard
//! input and output are, or the [`pipe`]s of a renderer run as a
//! thread. Nothing a renderer sends is carried out on trust: the
//! decoder answers any bytes with a message or [`Malformed`], and what
//! a message names is checked by the side that receives it.

mod channel;
mod codec;
mod messages;
mod net_wire;
mod pipe;
#[cfg(test)]
mod tests;

pub use channel::{Calls, Closed, ReadError, Sender, frame, read_message, spawn_reader};
pub use codec::{Bytes, Decoder, Encoder, Malformed, Wire, decode_all, encode};
#[allow(unused_imports)]
pub(crate) use codec::{wire_enum, wire_struct, wire_unit_enum};
pub use messages::{Answer, ToBrowser, ToRenderer};
pub use pipe::{PipeBreaker, PipeReader, PipeWriter, duplex, pipe};

/// The schema's version: a message's layout never changes without it.
/// The hello of each side carries it, and a renderer of another version
/// is refused.
pub const IPC_VERSION: u32 = 1;

/// The longest message, in bytes. A body or a picture longer goes in
/// pieces of at most [`MAX_CHUNK`].
pub const MAX_MESSAGE: usize = 16 << 20;

/// The longest piece of a body in one message.
pub const MAX_CHUNK: usize = 1 << 20;
