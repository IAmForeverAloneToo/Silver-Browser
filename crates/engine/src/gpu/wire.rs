//! The GPU process's protocols (M41 step 1, D38, docs/design/gpu.md):
//! the browser's with the GPU process, [`ToGpu`] and [`FromGpu`], which
//! carry each renderer's requests and replies as bytes the browser does
//! not read; and the client protocol inside them, [`Request`] and
//! [`Reply`], each message a batch of them. A variant's tag never
//! changes meaning within a [`GPU_VERSION`].

use crate::ipc::{Bytes, Malformed, decode_all, encode, wire_enum, wire_struct};

/// The protocols' version, which the GPU process's hello says.
pub const GPU_VERSION: u32 = 1;

wire_enum! {
    /// What the browser sends the GPU process.
    #[derive(Debug)]
    pub enum ToGpu {
        /// The first message: the protocols' version; whether the GPU
        /// process takes the platform's fallback adapter alone.
        0 => Hello { version: u32, fallback_only: bool },
        /// A batch of client `client`'s requests (a renderer's number).
        1 => Client { client: u32, bytes: Bytes },
        /// Client `client` is gone: its objects go.
        2 => Gone { client: u32 },
    }
}

wire_enum! {
    /// What the GPU process sends the browser.
    #[derive(Debug)]
    pub enum FromGpu {
        /// The answer to the hello.
        0 => Hello { version: u32 },
        /// A batch of replies for client `client`.
        1 => Client { client: u32, bytes: Bytes },
        /// Client `client` asked what the protocol does not allow: its
        /// objects are gone, and the browser ends it.
        2 => Bad { client: u32, why: String },
    }
}

wire_enum! {
    /// What a client asks of the GPU process. Objects are named by
    /// numbers the client gives, unique among its live ones.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Request {
        /// An adapter, numbered `id`, as the page asks for one: `power`
        /// 0 for no preference, 1 for low power, 2 for high performance;
        /// the fallback adapter when `fallback`. Answered by
        /// [`Reply::Adapter`] with `call`.
        0 => RequestAdapter { call: u32, id: u32, power: u8, fallback: bool },
        /// Object `id` goes.
        1 => Drop { id: u32 },
    }
}

wire_enum! {
    /// What the GPU process tells a client.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Reply {
        /// The adapter asked for by `call`, or `None` when there is none.
        0 => Adapter { call: u32, adapter: Option<AdapterInfo> },
        /// The client's GPU is gone (its process died, or it was ended):
        /// every object of its is lost.
        1 => Lost { why: String },
    }
}

wire_struct! {
    /// What a page is told of an adapter (D39): the vendor by name, from
    /// the adapter's PCI vendor, or empty; whether it is the fallback.
    #[derive(Debug, Clone, PartialEq)]
    pub struct AdapterInfo {
        pub vendor: String,
        pub fallback: bool,
    }
}

/// A batch of requests' bytes.
pub fn requests(batch: &[Request]) -> Vec<u8> {
    encode(&batch.to_vec())
}

/// A batch of requests, read from `bytes`.
pub fn read_requests(bytes: &[u8]) -> Result<Vec<Request>, Malformed> {
    decode_all(bytes)
}

/// A batch of replies' bytes.
pub fn replies(batch: &[Reply]) -> Vec<u8> {
    encode(&batch.to_vec())
}

/// A batch of replies, read from `bytes`.
pub fn read_replies(bytes: &[u8]) -> Result<Vec<Reply>, Malformed> {
    decode_all(bytes)
}

/// The vendor's name a page is told, by its PCI vendor number.
pub fn vendor_name(vendor: u32) -> &'static str {
    match vendor {
        0x8086 => "intel",
        0x10de => "nvidia",
        0x1002 | 0x1022 => "amd",
        0x106b => "apple",
        0x5143 => "qualcomm",
        0x13b5 => "arm",
        _ => "",
    }
}
