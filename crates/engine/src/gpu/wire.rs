//! The GPU process's protocols (M41 step 1, D38, docs/design/gpu.md):
//! the browser's with the GPU process, [`ToGpu`] and [`FromGpu`], which
//! carry each renderer's requests and replies as bytes the browser does
//! not read; and the client protocol inside them, [`Request`] and
//! [`Reply`], each message a batch of them. A variant's tag never
//! changes meaning within a [`GPU_VERSION`].

use crate::ipc::{Bytes, Malformed, decode_all, encode, wire_enum, wire_struct};

/// The protocols' version, which the GPU process's hello says.
pub const GPU_VERSION: u32 = 1;

/// The most bytes one request or reply carries: a buffer's longer
/// contents go in pieces of this.
pub const GPU_CHUNK: usize = 4 << 20;

/// The bytes a batch is made up to before it is sent, well within
/// `MAX_MESSAGE`.
pub const GPU_BATCH: usize = 8 << 20;

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
    /// numbers the client gives, unique among its live ones; a `call` is
    /// answered by a reply with the same number.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Request {
        /// An adapter, numbered `id`, as the page asks for one: `power`
        /// 0 for no preference, 1 for low power, 2 for high performance;
        /// the fallback adapter when `fallback`. Answered by
        /// [`Reply::Adapter`].
        0 => RequestAdapter { call: u32, id: u32, power: u8, fallback: bool },
        /// Object `id` goes.
        1 => Drop { id: u32 },
        /// A device of adapter `adapter`, numbered `id`, with the
        /// features and limits named (WebGPU's names). Answered by
        /// [`Reply::Device`].
        2 => RequestDevice {
            call: u32,
            adapter: u32,
            id: u32,
            features: Vec<String>,
            limits: Vec<(String, u64)>,
        },
        /// Device `id` is destroyed: its `lost` follows.
        3 => DestroyDevice { id: u32 },
        /// An error scope on device `device`'s stack: `filter` 0 for
        /// validation, 1 for out of memory, 2 for internal.
        4 => PushErrorScope { device: u32, filter: u8 },
        /// The top error scope of `device`'s stack, popped. Answered by
        /// [`Reply::ErrorScope`].
        5 => PopErrorScope { call: u32, device: u32 },
        /// A buffer of `device`, numbered `id`, `mapped` at creation.
        6 => CreateBuffer { device: u32, id: u32, size: u64, usage: u32, mapped: bool },
        /// Buffer `id` is destroyed.
        7 => DestroyBuffer { id: u32 },
        /// `bytes` written into `buffer` at `offset` by `device`'s queue.
        8 => WriteBuffer { device: u32, buffer: u32, offset: u64, bytes: Bytes },
        /// `buffer`'s range mapped, for reading (`mode` 1) or writing
        /// (2). Answered by [`Reply::MappedBytes`] pieces and then
        /// [`Reply::Mapped`].
        9 => MapBuffer { call: u32, buffer: u32, mode: u8, offset: u64, size: u64 },
        /// `bytes` written into `buffer`'s mapped range at `offset`.
        10 => MappedWrite { buffer: u32, offset: u64, bytes: Bytes },
        /// `buffer` unmapped, what was written into it kept.
        11 => Unmap { buffer: u32 },
        /// Answered by [`Reply::WorkDone`] once `device`'s queue has done
        /// the work submitted so far.
        12 => WorkDone { call: u32, device: u32 },
        /// A shader module of WGSL `code` (step 3). Descriptors cross as
        /// WebGPU's dictionaries in JSON, objects named by their numbers.
        13 => CreateShaderModule { device: u32, id: u32, code: String },
        /// Answered by [`Reply::Compilation`].
        14 => CompilationInfo { call: u32, module: u32 },
        15 => CreateTexture { device: u32, id: u32, desc: String },
        16 => CreateView { texture: u32, id: u32, desc: String },
        17 => DestroyTexture { id: u32 },
        18 => CreateSampler { device: u32, id: u32, desc: String },
        19 => CreateBindGroupLayout { device: u32, id: u32, desc: String },
        20 => CreatePipelineLayout { device: u32, id: u32, layouts: Vec<u32> },
        21 => CreateBindGroup { device: u32, id: u32, desc: String },
        /// A pipeline; with a `call`, the asynchronous kind, answered by
        /// [`Reply::PipelineMade`] with its error rather than reporting
        /// it to the device.
        22 => CreateComputePipeline { device: u32, id: u32, desc: String, call: Option<u32> },
        23 => CreateRenderPipeline { device: u32, id: u32, desc: String, call: Option<u32> },
        /// Pipeline `pipeline`'s bind group layout `index`, numbered `id`.
        24 => GetBindGroupLayout { pipeline: u32, index: u32, id: u32 },
        /// A command buffer, numbered `id`, of what an encoder recorded.
        25 => Finish { device: u32, id: u32, commands: Vec<Command> },
        /// The command buffers submitted to `device`'s queue, in order.
        26 => Submit { device: u32, buffers: Vec<u32> },
        /// `bytes` written into a texture by `device`'s queue: `desc` its
        /// destination, data layout and size.
        27 => WriteTexture { device: u32, desc: String, bytes: Bytes },
        /// A query set: `kind` 0 occlusion, 1 timestamp.
        28 => CreateQuerySet { device: u32, id: u32, kind: u8, count: u32 },
        29 => DestroyQuerySet { id: u32 },
        /// A render bundle of what its encoder recorded.
        30 => CreateRenderBundle { device: u32, id: u32, desc: String, commands: Vec<RenderCommand> },
        /// A validation error the client found on `device`: what the
        /// bootstrap checks before `wgpu` would (an encoder's state, a
        /// descriptor's shape), reported as `wgpu`'s are.
        31 => Invalid { device: u32, message: String },
    }
}

wire_enum! {
    /// What a command encoder recorded, replayed in the GPU process.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Command {
        0 => CopyBufferToBuffer { source: u32, source_offset: u64, destination: u32, destination_offset: u64, size: Option<u64> },
        /// The copies with a texture: `desc` the source, destination and
        /// size in WebGPU's dictionaries.
        1 => CopyBufferToTexture { desc: String },
        2 => CopyTextureToBuffer { desc: String },
        3 => CopyTextureToTexture { desc: String },
        4 => ClearBuffer { buffer: u32, offset: u64, size: Option<u64> },
        5 => ResolveQuerySet { set: u32, first: u32, count: u32, destination: u32, offset: u64 },
        /// A render pass: `desc` its attachments and occlusion query set.
        6 => RenderPass { desc: String, commands: Vec<RenderCommand> },
        7 => ComputePass { commands: Vec<ComputeCommand> },
    }
}

wire_enum! {
    /// What a render pass, or a render bundle, recorded.
    #[derive(Debug, Clone, PartialEq)]
    pub enum RenderCommand {
        0 => SetPipeline { pipeline: u32 },
        1 => SetBindGroup { index: u32, group: Option<u32>, offsets: Vec<u32> },
        2 => SetVertexBuffer { slot: u32, buffer: Option<u32>, offset: u64, size: Option<u64> },
        /// `format` 0 for uint16, 1 for uint32.
        3 => SetIndexBuffer { buffer: u32, format: u8, offset: u64, size: Option<u64> },
        4 => Draw { vertices: u32, instances: u32, first_vertex: u32, first_instance: u32 },
        5 => DrawIndexed { indices: u32, instances: u32, first_index: u32, base_vertex: i32, first_instance: u32 },
        6 => DrawIndirect { buffer: u32, offset: u64 },
        7 => DrawIndexedIndirect { buffer: u32, offset: u64 },
        8 => SetViewport { x: f32, y: f32, width: f32, height: f32, min_depth: f32, max_depth: f32 },
        9 => SetScissorRect { x: u32, y: u32, width: u32, height: u32 },
        10 => SetBlendConstant { r: f64, g: f64, b: f64, a: f64 },
        11 => SetStencilReference { reference: u32 },
        12 => BeginOcclusionQuery { index: u32 },
        13 => EndOcclusionQuery {},
        14 => ExecuteBundles { bundles: Vec<u32> },
    }
}

wire_enum! {
    /// What a compute pass recorded.
    #[derive(Debug, Clone, PartialEq)]
    pub enum ComputeCommand {
        0 => SetPipeline { pipeline: u32 },
        1 => SetBindGroup { index: u32, group: Option<u32>, offsets: Vec<u32> },
        2 => Dispatch { x: u32, y: u32, z: u32 },
        3 => DispatchIndirect { buffer: u32, offset: u64 },
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
        /// The device asked for by `call`, or why it was refused.
        2 => Device { call: u32, result: Result<DeviceInfo, String> },
        /// The error the popped scope caught, if any; Err when there was
        /// no scope to pop.
        3 => ErrorScope { call: u32, result: Result<Option<GpuError>, String> },
        /// An error no scope of `device` caught.
        4 => Uncaptured { device: u32, error: GpuError },
        /// Device `device` is lost: `destroyed` when by `destroy()`.
        5 => DeviceLost { device: u32, destroyed: bool, message: String },
        /// A piece of the range `call` mapped, at `offset` within it.
        6 => MappedBytes { call: u32, offset: u64, bytes: Bytes },
        /// The range `call` asked for is mapped, its pieces all sent; or,
        /// with `error`, why it could not be.
        7 => Mapped { call: u32, error: Option<String> },
        /// The queue's work submitted before `call` is done.
        8 => WorkDone { call: u32 },
        /// A shader module's compilation messages.
        9 => Compilation { call: u32, messages: Vec<CompilationMessage> },
        /// An asynchronous pipeline's making: its error, if it failed.
        10 => PipelineMade { call: u32, error: Option<String> },
    }
}

wire_struct! {
    /// A shader's compilation message: `kind` 0 error, 1 warning, 2
    /// info; where it is (lines and columns from 1, byte offsets).
    #[derive(Debug, Clone, PartialEq)]
    pub struct CompilationMessage {
        pub kind: u8,
        pub line: u64,
        pub position: u64,
        pub offset: u64,
        pub length: u64,
        pub message: String,
    }
}

wire_struct! {
    /// What a page is told of an adapter (D39): the vendor by name, from
    /// the adapter's PCI vendor, or empty; whether it is the fallback;
    /// its features and limits by WebGPU's names.
    #[derive(Debug, Clone, PartialEq)]
    pub struct AdapterInfo {
        pub vendor: String,
        pub fallback: bool,
        pub features: Vec<String>,
        pub limits: Vec<(String, u64)>,
    }
}

wire_struct! {
    /// A device's features and limits, by WebGPU's names.
    #[derive(Debug, Clone, PartialEq)]
    pub struct DeviceInfo {
        pub features: Vec<String>,
        pub limits: Vec<(String, u64)>,
    }
}

wire_struct! {
    /// An error of WebGPU's: `kind` 0 for validation, 1 for out of
    /// memory, 2 for internal; its message.
    #[derive(Debug, Clone, PartialEq)]
    pub struct GpuError {
        pub kind: u8,
        pub message: String,
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

/// `replies` in batches of at most about [`GPU_BATCH`] bytes each.
pub fn reply_batches(replies: Vec<Reply>) -> Vec<Vec<u8>> {
    let mut batches = Vec::new();
    let mut batch: Vec<Reply> = Vec::new();
    let mut size = 0;
    for reply in replies {
        let len = encode(&reply).len();
        if !batch.is_empty() && size + len > GPU_BATCH {
            batches.push(encode(&std::mem::take(&mut batch)));
            size = 0;
        }
        size += len;
        batch.push(reply);
    }
    if !batch.is_empty() {
        batches.push(encode(&batch));
    }
    batches
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
