//! WebGL's drawing (M41 step 6, D40): GL's state turned into WebGPU's
//! pipelines, bind groups and render passes on the context's device in
//! the GPU process. Each draw's uniforms and disabled attributes'
//! values go into an arena buffer, at an offset of their own; draws are
//! recorded into the pass of the drawing buffer, and sent when the task
//! ends or something must be read or written in order.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use devtools::Json;

use super::gl::*;
use super::{Context, MAX_PIPELINES, StencilFace, clamp_size, type_size, wait};
use crate::gpu::GpuClient;
use crate::gpu::wire::{Command, RenderCommand, Reply, Request};
use crate::ipc::Bytes;
use crate::scripts::canvas::GpuFrame;
use crate::scripts::workers::{Inbox, WorkerMessage};

/// The arena's bytes: a draw's data past them sends what was recorded
/// first.
const ARENA: u64 = 4 << 20;

/// WebGPU's alignment of a uniform buffer's dynamic offset.
const ALIGN: u64 = 256;

/// What a pipeline is made for: the program, its vertex buffers'
/// layouts, the topology, and GL's state that WebGPU keeps in one.
/// A vertex buffer's layout: its stride, whether per instance, and its
/// attributes (format, offset, location).
type BufferLayout = (u64, bool, Vec<(String, u64, u32)>);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PipelineKey {
    program: u32,
    generation: u32,
    points: bool,
    topology: &'static str,
    strip_index: Option<&'static str>,
    buffers: Vec<BufferLayout>,
    blend: Option<[u32; 6]>,
    write_mask: u32,
    cull: u32,
    front_ccw: bool,
    depth: Option<(u32, bool, i32, u32)>,
    stencil: Option<[StencilFace; 2]>,
    samples: u32,
    alpha_to_coverage: bool,
    mask: u32,
}

struct Pass {
    desc: String,
    commands: Vec<RenderCommand>,
}

/// A context's objects in the GPU process.
pub(crate) struct Gpu {
    pub device: u32,
    made: Vec<u32>,
    arena: u32,
    arena_used: u64,
    pass: Option<Pass>,
    pending: Vec<Command>,
    pipelines: HashMap<PipelineKey, u32>,
    order: VecDeque<PipelineKey>,
    groups: HashMap<(u32, Vec<u32>), u32>,
    /// The texture a sampler with no texture reads: 2D and cube views,
    /// and a sampler.
    defaults: (u32, u32, u32),
    readback: Option<(u32, u64)>,
    clear: Option<ClearGpu>,
    pub ext_blend_minmax: bool,
}

/// What a clear drawn as a quad needs: its module, layouts and
/// pipelines.
struct ClearGpu {
    module: u32,
    pipeline_layout: u32,
    group: u32,
    pipelines: HashMap<(u32, bool, u32, bool, u32), u32>,
}

/// A device for a context: its number, or none without an adapter.
pub(crate) fn device(client: &Arc<GpuClient>) -> Option<u32> {
    let adapter = client.id();
    let (reply, _) = wait(client, |call| Request::RequestAdapter {
        call,
        id: adapter,
        power: 0,
        fallback: false,
    })?;
    let Reply::Adapter {
        adapter: Some(info),
        ..
    } = reply
    else {
        return None;
    };
    // As many vertex buffers as attributes, where the adapter has them.
    let buffers = info
        .limits
        .iter()
        .find(|(n, _)| n == "maxVertexBuffers")
        .map_or(8, |(_, v)| *v)
        .min(16);
    let device = client.id();
    let (reply, _) = wait(client, |call| Request::RequestDevice {
        call,
        adapter,
        id: device,
        features: Vec::new(),
        limits: vec![("maxVertexBuffers".to_owned(), buffers)],
    })?;
    match reply {
        Reply::Device { result: Ok(_), .. } => Some(device),
        _ => None,
    }
}

impl Gpu {
    pub(crate) fn new(client: &Arc<GpuClient>, device: u32) -> Gpu {
        let mut made = Vec::new();
        let mut id = || {
            let i = client.id();
            made.push(i);
            i
        };
        let arena = id();
        // UNIFORM | VERTEX | INDEX | COPY_DST.
        client.queue([Request::CreateBuffer {
            device,
            id: arena,
            size: ARENA,
            usage: 0x78,
            mapped: false,
        }]);
        // The texture of a sampler with none: GL's incomplete texture,
        // black and opaque.
        let (texture, view, cube, cube_view, sampler) = (id(), id(), id(), id(), id());
        client.queue([
            Request::CreateTexture {
                device,
                id: texture,
                desc: r#"{"size":[1,1,1],"format":"rgba8unorm","usage":6}"#.to_owned(),
            },
            Request::CreateView {
                texture,
                id: view,
                desc: "{}".to_owned(),
            },
            Request::CreateTexture {
                device,
                id: cube,
                desc: r#"{"size":[1,1,6],"format":"rgba8unorm","usage":6}"#.to_owned(),
            },
            Request::CreateView {
                texture: cube,
                id: cube_view,
                desc: r#"{"dimension":"cube"}"#.to_owned(),
            },
            Request::CreateSampler {
                device,
                id: sampler,
                desc: "{}".to_owned(),
            },
            Request::WriteTexture {
                device,
                desc: format!(r#"{{"destination":{{"texture":{texture}}},"dataLayout":{{"bytesPerRow":4}},"size":[1,1,1]}}"#),
                bytes: Bytes(vec![0, 0, 0, 255]),
            },
            Request::WriteTexture {
                device,
                desc: format!(r#"{{"destination":{{"texture":{cube}}},"dataLayout":{{"bytesPerRow":4,"rowsPerImage":1}},"size":[1,1,6]}}"#),
                bytes: Bytes([0u8, 0, 0, 255].repeat(6)),
            },
        ]);
        Gpu {
            device,
            made,
            arena,
            arena_used: 0,
            pass: None,
            pending: Vec::new(),
            pipelines: HashMap::new(),
            order: VecDeque::new(),
            groups: HashMap::new(),
            defaults: (view, cube_view, sampler),
            readback: None,
            clear: None,
            ext_blend_minmax: false,
        }
    }

    fn id(&mut self, client: &GpuClient) -> u32 {
        let i = client.id();
        self.made.push(i);
        i
    }

    /// Every object it made gone in the GPU process, its device last.
    pub(crate) fn release(&mut self, client: &Arc<GpuClient>) {
        self.pass = None;
        self.pending.clear();
        let made = std::mem::take(&mut self.made);
        client.queue(made.into_iter().rev().map(|id| Request::Drop { id }));
        client.queue([Request::Drop { id: self.device }]);
    }

    /// A linked program's modules (vertex, fragment, points), bind group
    /// layout and pipeline layout.
    pub(crate) fn program(
        &mut self,
        client: &Arc<GpuClient>,
        program: &glsl::Program,
    ) -> (u32, u32, Option<u32>, u32, u32) {
        let device = self.device;
        let (vertex, fragment) = (self.id(client), self.id(client));
        client.queue([
            Request::CreateShaderModule {
                device,
                id: vertex,
                code: program.vertex.clone(),
            },
            Request::CreateShaderModule {
                device,
                id: fragment,
                code: program.fragment.clone(),
            },
        ]);
        let points = program.points.as_ref().map(|code| {
            let id = self.id(client);
            client.queue([Request::CreateShaderModule {
                device,
                id,
                code: code.clone(),
            }]);
            id
        });
        // Binding 0, the default block at a dynamic offset in the
        // arena; each sampler's texture and sampler.
        let mut entries = vec![format!(
            r#"{{"binding":0,"visibility":3,"buffer":{{"type":"uniform","hasDynamicOffset":true,"minBindingSize":{}}}}}"#,
            program.uniform_size.max(16)
        )];
        for s in &program.samplers {
            let (sample, sampler) = match (s.sampler.shadow, s.sampler.sample) {
                (true, _) => ("depth", "comparison"),
                (false, glsl::tir::Scalar::Int) => ("sint", "non-filtering"),
                (false, glsl::tir::Scalar::Uint) => ("uint", "non-filtering"),
                _ => ("float", "filtering"),
            };
            let dimension = match s.sampler.dim {
                glsl::tir::Dim::D2 => "2d",
                glsl::tir::Dim::D3 => "3d",
                glsl::tir::Dim::Cube => "cube",
                glsl::tir::Dim::D2Array => "2d-array",
            };
            for &binding in &s.bindings {
                entries.push(format!(r#"{{"binding":{binding},"visibility":3,"texture":{{"sampleType":"{sample}","viewDimension":"{dimension}"}}}}"#));
                entries.push(format!(
                    r#"{{"binding":{},"visibility":3,"sampler":{{"type":"{sampler}"}}}}"#,
                    binding + 1
                ));
            }
        }
        let (layout, pipeline_layout) = (self.id(client), self.id(client));
        client.queue([
            Request::CreateBindGroupLayout {
                device,
                id: layout,
                desc: format!(r#"{{"entries":[{}]}}"#, entries.join(",")),
            },
            Request::CreatePipelineLayout {
                device,
                id: pipeline_layout,
                layouts: vec![layout],
            },
        ]);
        (vertex, fragment, points, layout, pipeline_layout)
    }

    /// `bytes` in the arena, at an offset aligned to `align`: the offset,
    /// or none when the arena is full (the caller sends what was
    /// recorded and asks again).
    fn arena(&mut self, client: &GpuClient, bytes: &[u8], align: u64) -> Option<u64> {
        let offset = self.arena_used.div_ceil(align) * align;
        let size = (bytes.len() as u64).max(4).div_ceil(4) * 4;
        if offset + size > ARENA {
            return None;
        }
        let mut data = bytes.to_vec();
        data.resize(size as usize, 0);
        client.queue([Request::WriteBuffer {
            device: self.device,
            buffer: self.arena,
            offset,
            bytes: Bytes(data),
        }]);
        self.arena_used = offset + size;
        Some(offset)
    }
}

/// A WebGPU vertex format for a GL attribute's pointer: none where
/// WebGPU has none (three bytes or shorts; integers read as floats
/// unnormalized: G-list).
fn vertex_format(
    kind: u32,
    size: u32,
    normalized: bool,
    integer: bool,
    shader: glsl::tir::Scalar,
) -> Option<String> {
    use glsl::tir::Scalar;
    let float_shader = shader == Scalar::Float;
    let name = match (kind, normalized || integer) {
        (FLOAT, _) => match size {
            1 => "float32".to_owned(),
            n => format!("float32x{n}"),
        },
        (HALF_FLOAT, _) => match size {
            1 => "float16".to_owned(),
            2 | 4 => format!("float16x{size}"),
            _ => return None,
        },
        (BYTE | UNSIGNED_BYTE | SHORT | UNSIGNED_SHORT, _) => {
            let signed = matches!(kind, BYTE | SHORT);
            let bits = if matches!(kind, BYTE | UNSIGNED_BYTE) {
                8
            } else {
                16
            };
            let base = match (normalized, integer || !float_shader, signed) {
                (true, _, true) => "snorm",
                (true, _, false) => "unorm",
                (false, true, true) => "sint",
                (false, true, false) => "uint",
                (false, false, _) => return None,
            };
            match size {
                1 => format!("{base}{bits}"),
                2 | 4 => format!("{base}{bits}x{size}"),
                _ => return None,
            }
        }
        (INT | UNSIGNED_INT, _) if integer || !float_shader => {
            let base = if kind == INT { "sint32" } else { "uint32" };
            if size == 1 {
                base.to_owned()
            } else {
                format!("{base}x{size}")
            }
        }
        _ => return None,
    };
    // A float attribute reads float formats; an integer one integers.
    let integral = name.starts_with("sint") || name.starts_with("uint");
    if integral == float_shader {
        return None;
    }
    Some(name)
}

fn blend_factor(f: u32, alpha_constant: bool) -> &'static str {
    match f {
        ZERO => "zero",
        ONE => "one",
        SRC_COLOR => "src",
        ONE_MINUS_SRC_COLOR => "one-minus-src",
        SRC_ALPHA => "src-alpha",
        ONE_MINUS_SRC_ALPHA => "one-minus-src-alpha",
        DST_ALPHA => "dst-alpha",
        ONE_MINUS_DST_ALPHA => "one-minus-dst-alpha",
        DST_COLOR => "dst",
        ONE_MINUS_DST_COLOR => "one-minus-dst",
        SRC_ALPHA_SATURATE => "src-alpha-saturated",
        CONSTANT_COLOR | CONSTANT_ALPHA => {
            let _ = alpha_constant;
            "constant"
        }
        _ => "one-minus-constant",
    }
}

fn blend_operation(e: u32) -> &'static str {
    match e {
        FUNC_SUBTRACT => "subtract",
        FUNC_REVERSE_SUBTRACT => "reverse-subtract",
        MIN => "min",
        MAX => "max",
        _ => "add",
    }
}

fn compare(f: u32) -> &'static str {
    match f {
        NEVER => "never",
        LESS => "less",
        EQUAL => "equal",
        LEQUAL => "less-equal",
        GREATER => "greater",
        NOTEQUAL => "not-equal",
        GEQUAL => "greater-equal",
        _ => "always",
    }
}

fn stencil_operation(op: u32) -> &'static str {
    match op {
        ZERO => "zero",
        REPLACE => "replace",
        INCR => "increment-clamp",
        DECR => "decrement-clamp",
        INVERT => "invert",
        INCR_WRAP => "increment-wrap",
        DECR_WRAP => "decrement-wrap",
        _ => "keep",
    }
}

/// A draw's indices, read from an element array buffer's data.
fn indices(data: &[u8], kind: u32, offset: usize, count: usize) -> Option<Vec<u32>> {
    let size = type_size(kind) as usize;
    let bytes = data.get(offset..offset.checked_add(count.checked_mul(size)?)?)?;
    Some(
        bytes
            .chunks_exact(size)
            .map(|c| match size {
                1 => u32::from(c[0]),
                2 => u32::from(u16::from_le_bytes([c[0], c[1]])),
                _ => u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
            })
            .collect(),
    )
}

/// The indices of a fan as a list of triangles, of a loop as a strip
/// that closes.
fn convert_indices(mode: u32, list: &[u32]) -> Vec<u32> {
    match mode {
        TRIANGLE_FAN => {
            let mut out = Vec::with_capacity(list.len().saturating_sub(2) * 3);
            for k in 1..list.len().saturating_sub(1) {
                out.extend_from_slice(&[list[0], list[k], list[k + 1]]);
            }
            out
        }
        LINE_LOOP if !list.is_empty() => {
            let mut out = list.to_vec();
            out.push(list[0]);
            out
        }
        _ => list.to_vec(),
    }
}

impl Context {
    // ---- Buffers ----

    pub(crate) fn buffer_data(&mut self, id: u32, data: Vec<u8>, usage: u32) {
        // What was recorded reads the buffer as it was.
        self.gpu_flush();
        let size = (data.len() as u64).max(4).div_ceil(4) * 4;
        let old = self.buffers.get(&id).and_then(|b| b.gpu);
        let gpu = match old {
            Some((gpu, had)) if had == size => gpu,
            _ => {
                let gpu = self.gpu.id(&self.client);
                // VERTEX | INDEX | COPY_DST | COPY_SRC.
                self.client.queue([Request::CreateBuffer {
                    device: self.device,
                    id: gpu,
                    size,
                    usage: 0x3C,
                    mapped: false,
                }]);
                gpu
            }
        };
        if !data.is_empty() {
            let mut padded = data.clone();
            padded.resize(size as usize, 0);
            self.client
                .queue_bytes(0, &padded, |offset, bytes| Request::WriteBuffer {
                    device: self.device,
                    buffer: gpu,
                    offset,
                    bytes,
                });
        }
        let buffer = self.buffers.get_mut(&id).expect("bound");
        buffer.data = data;
        buffer.usage = usage;
        buffer.gpu = Some((gpu, size));
    }

    pub(crate) fn buffer_sub_data(&mut self, id: u32, offset: usize, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.gpu_flush();
        let buffer = self.buffers.get_mut(&id).expect("bound");
        buffer.data[offset..offset + bytes.len()].copy_from_slice(bytes);
        let Some((gpu, size)) = buffer.gpu else {
            return;
        };
        // WebGPU writes four bytes at a time: the words around the
        // range, from the data kept here.
        let start = offset / 4 * 4;
        let end = ((offset + bytes.len()).div_ceil(4) * 4).min(size as usize);
        let mut words = vec![0u8; end - start];
        let data_end = buffer.data.len().min(end);
        words[..data_end - start].copy_from_slice(&buffer.data[start..data_end]);
        let device = self.device;
        self.client
            .queue_bytes(start as u64, &words, |offset, bytes| Request::WriteBuffer {
                device,
                buffer: gpu,
                offset,
                bytes,
            });
    }

    // ---- The drawing buffer ----

    /// The drawing buffer, made or made again at the canvas's size.
    fn drawing_buffer(&mut self, size: (u32, u32)) {
        let size = clamp_size(size);
        if self.drawing.as_ref().is_some_and(|d| d.size == size) {
            return;
        }
        self.gpu_flush();
        let device = self.device;
        let (w, h) = size;
        let color = (self.gpu.id(&self.client), self.gpu.id(&self.client));
        // RENDER_ATTACHMENT | COPY_SRC | TEXTURE_BINDING.
        self.client.queue([
            Request::CreateTexture {
                device,
                id: color.0,
                desc: format!(r#"{{"size":[{w},{h},1],"format":"rgba8unorm","usage":21}}"#),
            },
            Request::CreateView {
                texture: color.0,
                id: color.1,
                desc: "{}".to_owned(),
            },
        ]);
        let msaa = self.attributes.antialias.then(|| {
            let ids = (self.gpu.id(&self.client), self.gpu.id(&self.client));
            self.client.queue([
                Request::CreateTexture {
                    device,
                    id: ids.0,
                    desc: format!(
                        r#"{{"size":[{w},{h},1],"format":"rgba8unorm","usage":16,"sampleCount":4}}"#
                    ),
                },
                Request::CreateView {
                    texture: ids.0,
                    id: ids.1,
                    desc: "{}".to_owned(),
                },
            ]);
            ids
        });
        let depth = (self.attributes.depth || self.attributes.stencil).then(|| {
            let ids = (self.gpu.id(&self.client), self.gpu.id(&self.client));
            let format = if self.attributes.stencil { "depth24plus-stencil8" } else { "depth24plus" };
            let samples = if self.attributes.antialias { 4 } else { 1 };
            self.client.queue([
                Request::CreateTexture {
                    device,
                    id: ids.0,
                    desc: format!(r#"{{"size":[{w},{h},1],"format":"{format}","usage":16,"sampleCount":{samples}}}"#),
                },
                Request::CreateView {
                    texture: ids.0,
                    id: ids.1,
                    desc: "{}".to_owned(),
                },
            ]);
            ids
        });
        self.drawing = Some(super::DrawingBuffer {
            size,
            color,
            msaa,
            depth,
        });
        // A new drawing buffer is cleared, and the viewport and scissor
        // stay as the page set them (WebGL 2.2).
        self.needs_clear = true;
        self.gpu.readback = None;
    }

    /// The pass of the drawing buffer, begun if none is: with its loads,
    /// or the clears a drawing buffer starts with.
    fn pass(&mut self, size: (u32, u32), clear: Option<(u32, [f32; 4], f32, i32)>) {
        self.drawing_buffer(size);
        if self.gpu.pass.is_some() && clear.is_none() {
            return;
        }
        if let Some(pass) = self.gpu.pass.take() {
            self.gpu.pending.push(Command::RenderPass {
                desc: pass.desc,
                commands: pass.commands,
            });
        }
        let d = self.drawing.clone().expect("made");
        let fresh = std::mem::take(&mut self.needs_clear);
        let (mask, color, depth, stencil) = match (clear, fresh) {
            (Some((mask, c, z, s)), true) => (
                mask | COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT | STENCIL_BUFFER_BIT,
                c,
                z,
                s,
            ),
            (Some(c), false) => c,
            (None, true) => (
                COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT | STENCIL_BUFFER_BIT,
                [0.0; 4],
                1.0,
                0,
            ),
            (None, false) => (0, [0.0; 4], 1.0, 0),
        };
        // A fresh buffer's parts not asked for are cleared to GL's
        // defaults, not the page's values.
        let (color, depth, stencil) = match clear {
            Some(c) if fresh => (
                if c.0 & COLOR_BUFFER_BIT != 0 {
                    color
                } else {
                    [0.0; 4]
                },
                if c.0 & DEPTH_BUFFER_BIT != 0 {
                    depth
                } else {
                    1.0
                },
                if c.0 & STENCIL_BUFFER_BIT != 0 {
                    stencil
                } else {
                    0
                },
            ),
            _ => (color, depth, stencil),
        };
        let load = |bit: u32| if mask & bit != 0 { "clear" } else { "load" };
        let (view, resolve) = match d.msaa {
            Some((_, v)) => (v, format!(r#","resolveTarget":{}"#, d.color.1)),
            None => (d.color.1, String::new()),
        };
        let depth_desc = match d.depth {
            Some((_, v)) => {
                let stencil_ops = if self.attributes.stencil {
                    format!(
                        r#","stencilLoadOp":"{}","stencilStoreOp":"store","stencilClearValue":{}"#,
                        load(STENCIL_BUFFER_BIT),
                        stencil as u32 & 0xFF
                    )
                } else {
                    String::new()
                };
                format!(
                    r#","depthStencilAttachment":{{"view":{v},"depthLoadOp":"{}","depthStoreOp":"store","depthClearValue":{depth}{stencil_ops}}}"#,
                    load(DEPTH_BUFFER_BIT)
                )
            }
            None => String::new(),
        };
        let desc = format!(
            r#"{{"colorAttachments":[{{"view":{view}{resolve},"loadOp":"{}","storeOp":"store","clearValue":[{},{},{},{}]}}]{depth_desc}}}"#,
            load(COLOR_BUFFER_BIT),
            color[0],
            color[1],
            color[2],
            color[3]
        );
        self.gpu.pass = Some(Pass {
            desc,
            commands: Vec::new(),
        });
    }

    /// What was recorded sent to the GPU process: the passes, one
    /// command buffer.
    pub(crate) fn gpu_flush(&mut self) {
        if let Some(pass) = self.gpu.pass.take() {
            self.gpu.pending.push(Command::RenderPass {
                desc: pass.desc,
                commands: pass.commands,
            });
        }
        if self.gpu.pending.is_empty() {
            return;
        }
        let commands = std::mem::take(&mut self.gpu.pending);
        let id = self.client.id();
        self.client.queue([
            Request::Finish {
                device: self.device,
                id,
                commands,
            },
            Request::Submit {
                device: self.device,
                buffers: vec![id],
            },
            Request::Drop { id },
        ]);
        self.gpu.arena_used = 0;
        self.gpu.groups.retain(|_, _| true);
    }

    // ---- Clears ----

    pub(crate) fn clear(&mut self, mask: u32, size: (u32, u32)) {
        if mask & !(COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT | STENCIL_BUFFER_BIT) != 0 {
            self.error(INVALID_VALUE);
            return;
        }
        if mask == 0 {
            return;
        }
        self.drawn = true;
        let s = self.state.clone();
        let whole_color = s.color_mask.iter().all(|&m| m);
        let whole_stencil = s.stencil[0].write_mask & 0xFF == 0xFF;
        let simple = !s.scissor_test
            && (mask & COLOR_BUFFER_BIT == 0 || whole_color)
            && (mask & DEPTH_BUFFER_BIT == 0 || s.depth_mask)
            && (mask & STENCIL_BUFFER_BIT == 0 || whole_stencil);
        // A clear of all a pass would: the pass's load (and any recorded
        // before it stays).
        let color = if self.attributes.alpha {
            s.clear_color
        } else {
            [s.clear_color[0], s.clear_color[1], s.clear_color[2], 1.0]
        };
        if simple {
            self.pass(size, Some((mask, color, s.clear_depth, s.clear_stencil)));
            return;
        }
        // Else a quad, scissored and masked.
        self.pass(size, None);
        self.clear_quad(mask, color, size);
    }

    fn clear_quad(&mut self, mask: u32, color: [f32; 4], size: (u32, u32)) {
        let device = self.device;
        let samples = if self.attributes.antialias { 4 } else { 1 };
        let has_depth = self.attributes.depth || self.attributes.stencil;
        if self.gpu.clear.is_none() {
            let (module, layout, pipeline_layout, group) = (
                self.gpu.id(&self.client),
                self.gpu.id(&self.client),
                self.gpu.id(&self.client),
                self.gpu.id(&self.client),
            );
            let code = "struct C { color: vec4<f32>, depth: vec4<f32> }\n@group(0) @binding(0) var<uniform> c: C;\n@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {\n    var p = array(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));\n    return vec4<f32>(p[i], c.depth.x, 1.0);\n}\n@fragment fn fs() -> @location(0) vec4<f32> { return c.color; }\n";
            self.client.queue([
                Request::CreateShaderModule { device, id: module, code: code.to_owned() },
                Request::CreateBindGroupLayout {
                    device,
                    id: layout,
                    desc: r#"{"entries":[{"binding":0,"visibility":3,"buffer":{"type":"uniform","hasDynamicOffset":true,"minBindingSize":32}}]}"#.to_owned(),
                },
                Request::CreatePipelineLayout { device, id: pipeline_layout, layouts: vec![layout] },
                Request::CreateBindGroup {
                    device,
                    id: group,
                    desc: format!(r#"{{"layout":{layout},"entries":[{{"binding":0,"buffer":{},"offset":0,"size":32}}]}}"#, self.gpu.arena),
                },
            ]);
            self.gpu.clear = Some(ClearGpu {
                module,
                pipeline_layout,
                group,
                pipelines: HashMap::new(),
            });
        }
        let s = self.state.clone();
        let write = if mask & COLOR_BUFFER_BIT != 0 {
            s.color_mask
                .iter()
                .enumerate()
                .fold(0, |m, (k, &on)| if on { m | (1 << k) } else { m })
        } else {
            0
        };
        let depth_write = mask & DEPTH_BUFFER_BIT != 0 && s.depth_mask;
        let stencil_write = if mask & STENCIL_BUFFER_BIT != 0 && self.attributes.stencil {
            s.stencil[0].write_mask & 0xFF
        } else {
            0
        };
        let key = (write, depth_write, stencil_write, has_depth, samples);
        let clear = self.gpu.clear.as_ref().expect("made");
        let pipeline = match clear.pipelines.get(&key) {
            Some(&p) => p,
            None => {
                let (module, pipeline_layout) = (clear.module, clear.pipeline_layout);
                let id = self.gpu.id(&self.client);
                let depth_stencil = if has_depth {
                    let format = if self.attributes.stencil {
                        "depth24plus-stencil8"
                    } else {
                        "depth24plus"
                    };
                    let face = r#"{"compare":"always","passOp":"replace","failOp":"keep","depthFailOp":"keep"}"#;
                    format!(
                        r#","depthStencil":{{"format":"{format}","depthWriteEnabled":{depth_write},"depthCompare":"always","stencilFront":{face},"stencilBack":{face},"stencilWriteMask":{stencil_write}}}"#
                    )
                } else {
                    String::new()
                };
                let desc = format!(
                    r#"{{"layout":{pipeline_layout},"vertex":{{"module":{module},"entryPoint":"vs","buffers":[]}},"fragment":{{"module":{module},"entryPoint":"fs","targets":[{{"format":"rgba8unorm","writeMask":{write}}}]}},"multisample":{{"count":{samples}}}{depth_stencil}}}"#
                );
                self.client.queue([Request::CreateRenderPipeline {
                    device,
                    id,
                    desc,
                    call: None,
                }]);
                self.gpu
                    .clear
                    .as_mut()
                    .expect("made")
                    .pipelines
                    .insert(key, id);
                id
            }
        };
        let mut data = Vec::with_capacity(32);
        for v in color {
            data.extend_from_slice(&v.to_le_bytes());
        }
        // The depth, from GL's window value to the quad's NDC (the
        // translation's range), and the rest.
        for v in [s.clear_depth, 0.0, 0.0, 0.0] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let offset = match self.gpu.arena(&self.client, &data, ALIGN) {
            Some(o) => o,
            None => {
                self.gpu_flush();
                self.pass(size, None);
                match self.gpu.arena(&self.client, &data, ALIGN) {
                    Some(o) => o,
                    None => return,
                }
            }
        };
        let group = self.gpu.clear.as_ref().expect("made").group;
        let d = self.drawing.clone().expect("made");
        let scissor = self.scissor_rect(d.size);
        let reference = s.clear_stencil as u32 & 0xFF;
        let pass = self.gpu.pass.as_mut().expect("begun");
        pass.commands.extend([
            RenderCommand::SetPipeline { pipeline },
            RenderCommand::SetBindGroup {
                index: 0,
                group: Some(group),
                offsets: vec![offset as u32],
            },
            RenderCommand::SetViewport {
                x: 0.0,
                y: 0.0,
                width: d.size.0 as f32,
                height: d.size.1 as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            },
            RenderCommand::SetScissorRect {
                x: scissor.0,
                y: scissor.1,
                width: scissor.2,
                height: scissor.3,
            },
            RenderCommand::SetStencilReference { reference },
            RenderCommand::Draw {
                vertices: 3,
                instances: 1,
                first_vertex: 0,
                first_instance: 0,
            },
        ]);
    }

    /// The scissor rectangle within the drawing buffer: GL's box when the
    /// test is on (its y from the bottom is the row from the top here:
    /// the frame is drawn upside down), else the whole buffer.
    fn scissor_rect(&self, size: (u32, u32)) -> (u32, u32, u32, u32) {
        if !self.state.scissor_test {
            return (0, 0, size.0, size.1);
        }
        let [x, y, w, h] = self.state.scissor;
        let x0 = x.clamp(0, size.0 as i32);
        let y0 = y.clamp(0, size.1 as i32);
        let x1 = (x.saturating_add(w)).clamp(0, size.0 as i32);
        let y1 = (y.saturating_add(h)).clamp(0, size.1 as i32);
        (
            x0 as u32,
            y0 as u32,
            (x1 - x0).max(0) as u32,
            (y1 - y0).max(0) as u32,
        )
    }

    // ---- Draws ----

    pub(crate) fn draw_arrays(
        &mut self,
        mode: u32,
        first: i32,
        count: i32,
        instances: i32,
        size: (u32, u32),
    ) {
        if mode > TRIANGLE_FAN {
            self.error(INVALID_ENUM);
            return;
        }
        if first < 0 || count < 0 || instances < 0 {
            self.error(INVALID_VALUE);
            return;
        }
        if self
            .check_attributes(
                Some((first as u64 + count as u64).saturating_sub(1)),
                instances as u64,
            )
            .is_none()
        {
            return;
        }
        if count == 0 || instances == 0 {
            return;
        }
        let list: Option<Vec<u32>> = matches!(mode, TRIANGLE_FAN | LINE_LOOP).then(|| {
            convert_indices(
                mode,
                &(first as u32..(first + count) as u32).collect::<Vec<_>>(),
            )
        });
        self.draw(
            mode,
            list,
            None,
            first as u32,
            count as u32,
            instances as u32,
            size,
        );
    }

    pub(crate) fn draw_elements(
        &mut self,
        mode: u32,
        count: i32,
        kind: u32,
        offset: f64,
        instances: i32,
        size: (u32, u32),
    ) {
        if mode > TRIANGLE_FAN {
            self.error(INVALID_ENUM);
            return;
        }
        if !matches!(kind, UNSIGNED_BYTE | UNSIGNED_SHORT)
            && !(kind == UNSIGNED_INT && self.version == 2)
        {
            self.error(INVALID_ENUM);
            return;
        }
        if count < 0 || offset < 0.0 || instances < 0 {
            self.error(INVALID_VALUE);
            return;
        }
        let offset = offset as u64;
        if !offset.is_multiple_of(u64::from(type_size(kind))) {
            self.error(INVALID_OPERATION);
            return;
        }
        let Some(element) = self.element_buffer else {
            self.error(INVALID_OPERATION);
            return;
        };
        let data = self.buffers[&element].data.clone();
        let Some(list) = indices(&data, kind, offset as usize, count as usize) else {
            self.error(INVALID_OPERATION);
            return;
        };
        // WebGL checks the largest index against the attributes' data
        // (WebGL 6.6).
        let restart = if self.version == 2 {
            Some(if kind == UNSIGNED_INT {
                u32::MAX
            } else if kind == UNSIGNED_SHORT {
                0xFFFF
            } else {
                0xFF
            })
        } else {
            None
        };
        let max = list.iter().copied().filter(|&i| Some(i) != restart).max();
        if self
            .check_attributes(max.map(u64::from), instances as u64)
            .is_none()
        {
            return;
        }
        if count == 0 || instances == 0 {
            return;
        }
        let converted =
            matches!(mode, TRIANGLE_FAN | LINE_LOOP).then(|| convert_indices(mode, &list));
        let indexed = if converted.is_some() {
            None
        } else {
            Some((element, kind, offset))
        };
        self.draw(
            mode,
            converted,
            indexed,
            0,
            count as u32,
            instances as u32,
            size,
        );
    }

    /// Checks the program and the enabled attributes' data for vertices
    /// to `last` (none: no vertex is read) and `instances`: Some when the
    /// draw may go on, else the error is recorded.
    fn check_attributes(&mut self, last: Option<u64>, instances: u64) -> Option<()> {
        let Some(program) = self.program else {
            self.error(INVALID_OPERATION);
            return None;
        };
        let Some(linked) = self.programs.get(&program).and_then(|p| p.linked.clone()) else {
            self.error(INVALID_OPERATION);
            return None;
        };
        for attribute in &linked.program.attributes {
            for k in 0..attribute.locations {
                let a = &self.attribs[(attribute.location + k) as usize];
                if !a.enabled {
                    continue;
                }
                let Some(buffer) = a.buffer.and_then(|b| self.buffers.get(&b)) else {
                    self.error(INVALID_OPERATION);
                    return None;
                };
                let element = u64::from(a.size * type_size(a.kind));
                let stride = if a.stride == 0 {
                    element
                } else {
                    u64::from(a.stride)
                };
                let reads = if a.divisor > 0 {
                    instances.saturating_sub(1) / u64::from(a.divisor)
                } else {
                    match last {
                        Some(l) => l,
                        None => continue,
                    }
                };
                let end = a.offset + reads * stride + element;
                if end > buffer.data.len() as u64 {
                    self.error(INVALID_OPERATION);
                    return None;
                }
            }
        }
        Some(())
    }

    /// Records a draw: its pipeline, bind group, vertex buffers and
    /// indices. `list` its indices made here (a fan's, a loop's),
    /// `indexed` an element array buffer's.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        mode: u32,
        list: Option<Vec<u32>>,
        indexed: Option<(u32, u32, u64)>,
        first: u32,
        count: u32,
        instances: u32,
        size: (u32, u32),
    ) {
        let s = self.state.clone();
        if s.cull && s.cull_mode == FRONT_AND_BACK && mode >= TRIANGLES {
            return;
        }
        self.drawn = true;
        self.pass(size, None);
        let program_id = self.program.expect("checked");
        let linked = self.programs[&program_id].linked.clone().expect("checked");
        let shader_attributes = linked.program.attributes.clone();
        // Points of a size: the points variant, an instance a point.
        let points = mode == POINTS
            && linked.points.is_some()
            && list.is_none()
            && indexed.is_none()
            && instances == 1;
        let topology: &'static str = match mode {
            POINTS if points => "triangle-strip",
            POINTS => "point-list",
            LINES => "line-list",
            LINE_STRIP | LINE_LOOP => "line-strip",
            TRIANGLE_STRIP => "triangle-strip",
            _ => "triangle-list",
        };
        // The vertex buffers: an enabled attribute's own; the disabled
        // ones' values together in the arena.
        let mut buffers: Vec<BufferLayout> = Vec::new();
        let mut slots: Vec<(Option<u32>, u64)> = Vec::new();
        let mut constants: Vec<u8> = Vec::new();
        let mut constant_attributes: Vec<(String, u64, u32)> = Vec::new();
        for attribute in &shader_attributes {
            for k in 0..attribute.locations {
                let location = attribute.location + k;
                let a = self.attribs[location as usize].clone();
                if a.enabled {
                    let Some((gpu, _)) = a
                        .buffer
                        .and_then(|b| self.buffers.get(&b))
                        .and_then(|b| b.gpu)
                    else {
                        return;
                    };
                    let Some(format) =
                        vertex_format(a.kind, a.size, a.normalized, a.integer, attribute.scalar)
                    else {
                        // A pointer WebGPU has no format for (G-list).
                        return;
                    };
                    let element = u64::from(a.size * type_size(a.kind));
                    let stride = if a.stride == 0 {
                        element
                    } else {
                        u64::from(a.stride)
                    };
                    let stride = stride.div_ceil(4) * 4;
                    let instance = a.divisor > 0 || points;
                    buffers.push((stride, instance, vec![(format, 0, location)]));
                    slots.push((Some(gpu), a.offset));
                } else {
                    let (format, bytes): (String, Vec<u8>) = match (attribute.scalar, a.value_kind)
                    {
                        (glsl::tir::Scalar::Int, _) => (
                            "sint32x4".to_owned(),
                            a.value.iter().flat_map(|v| v.to_le_bytes()).collect(),
                        ),
                        (glsl::tir::Scalar::Uint, _) => (
                            "uint32x4".to_owned(),
                            a.value.iter().flat_map(|v| v.to_le_bytes()).collect(),
                        ),
                        _ => (
                            "float32x4".to_owned(),
                            a.value.iter().flat_map(|v| v.to_le_bytes()).collect(),
                        ),
                    };
                    let shown_format = if attribute.components < 4
                        && attribute.scalar == glsl::tir::Scalar::Float
                    {
                        // A float attribute of fewer components reads the
                        // first of the four.
                        format.clone()
                    } else {
                        format.clone()
                    };
                    constant_attributes.push((shown_format, constants.len() as u64, location));
                    constants.extend(bytes);
                }
            }
        }
        if !constant_attributes.is_empty() {
            let offset = self.arena_or_flush(&constants, 4, size);
            let Some(offset) = offset else { return };
            buffers.push((0, false, constant_attributes));
            slots.push((Some(self.gpu.arena), offset));
        }
        // The pipeline.
        let blend = s.blend.then(|| {
            let minmax = |e: u32| matches!(e, MIN | MAX);
            let (mut src, mut dst) = (s.blend_src, s.blend_dst);
            for k in 0..2 {
                if minmax(s.blend_equation[k]) {
                    src[k] = ONE;
                    dst[k] = ONE;
                }
            }
            [
                src[0],
                dst[0],
                src[1],
                dst[1],
                s.blend_equation[0],
                s.blend_equation[1],
            ]
        });
        let write_mask = s
            .color_mask
            .iter()
            .enumerate()
            .fold(0, |m, (k, &on)| if on { m | (1 << k) } else { m });
        let triangles = mode >= TRIANGLES || points;
        let has_depth = self.attributes.depth || self.attributes.stencil;
        let depth = has_depth.then(|| {
            let bias = if s.polygon_offset && triangles && !points {
                s.polygon_offset_units as i32
            } else {
                0
            };
            let slope = if s.polygon_offset && triangles && !points {
                s.polygon_offset_factor
            } else {
                0.0
            };
            if s.depth_test && self.attributes.depth {
                (s.depth_func, s.depth_mask, bias, slope.to_bits())
            } else {
                (ALWAYS, false, bias, slope.to_bits())
            }
        });
        let stencil = (s.stencil_test && self.attributes.stencil).then_some(s.stencil);
        let strip_index = match (topology, indexed) {
            ("triangle-strip" | "line-strip", Some((_, kind, _))) => {
                Some(if kind == UNSIGNED_INT {
                    "uint32"
                } else {
                    "uint16"
                })
            }
            _ => None,
        };
        let key = PipelineKey {
            program: program_id,
            generation: linked.generation,
            points,
            topology,
            strip_index,
            buffers: buffers.clone(),
            blend,
            write_mask,
            cull: if s.cull && triangles {
                if s.cull_mode == FRONT { 1 } else { 2 }
            } else {
                0
            },
            front_ccw: s.front_face == CCW,
            depth,
            stencil,
            samples: if self.attributes.antialias { 4 } else { 1 },
            alpha_to_coverage: s.sample_alpha_to_coverage && self.attributes.antialias,
            mask: if s.sample_coverage && self.attributes.antialias {
                let covered = (s.sample_coverage_value * 4.0).round() as u32;
                let bits = (1u32 << covered.min(4)) - 1;
                if s.sample_coverage_invert {
                    !bits & 0xF
                } else {
                    bits
                }
            } else {
                u32::MAX
            },
        };
        let pipeline = self.pipeline(&key, &linked);
        // The uniforms, their first 32 bytes the translation's own: the
        // viewport (for points) and the depth range.
        let viewport = s.viewport;
        let mut uniforms = linked.uniforms.clone();
        let own = [
            viewport[0] as f32,
            viewport[1] as f32,
            viewport[2] as f32,
            viewport[3] as f32,
            s.depth_range[0],
            s.depth_range[1],
            s.depth_range[1] - s.depth_range[0],
            0.0,
        ];
        for (k, v) in own.iter().enumerate() {
            if let Some(slot) = uniforms.get_mut(k * 4..k * 4 + 4) {
                slot.copy_from_slice(&v.to_le_bytes());
            }
        }
        let Some(uniform_offset) = self.arena_or_flush(&uniforms, 256, size) else {
            return;
        };
        let group = self.bind_group(&linked);
        // Indices made here go in the arena too.
        let index = match &list {
            Some(list) => {
                let bytes: Vec<u8> = list.iter().flat_map(|i| i.to_le_bytes()).collect();
                let Some(offset) = self.arena_or_flush(&bytes, 4, size) else {
                    return;
                };
                Some((self.gpu.arena, 1u8, offset, list.len() as u32))
            }
            None => indexed.map(|(element, kind, offset)| {
                let gpu = self.buffers[&element].gpu.map_or(0, |(g, _)| g);
                (
                    gpu,
                    if kind == UNSIGNED_INT {
                        1
                    } else if kind == UNSIGNED_SHORT {
                        0
                    } else {
                        2
                    },
                    offset,
                    count,
                )
            }),
        };
        // Bytes as indices WebGPU does not take: made u16 in the arena.
        let index = match index {
            Some((_, 2, offset, n)) => {
                let element = indexed.expect("indexed").0;
                let data = self.buffers[&element].data.clone();
                let list =
                    indices(&data, UNSIGNED_BYTE, offset as usize, n as usize).unwrap_or_default();
                let bytes: Vec<u8> = list
                    .iter()
                    .flat_map(|&i| (i as u16).to_le_bytes())
                    .collect();
                let Some(at) = self.arena_or_flush(&bytes, 4, size) else {
                    return;
                };
                Some((self.gpu.arena, 0u8, at, n))
            }
            other => other,
        };
        let d = self.drawing.clone().expect("made");
        let (vx, vy) = (viewport[0].max(0) as f32, viewport[1].max(0) as f32);
        let vw = (viewport[2] as f32).min(d.size.0 as f32 - vx).max(0.0);
        let vh = (viewport[3] as f32).min(d.size.1 as f32 - vy).max(0.0);
        let scissor = self.scissor_rect(d.size);
        let blend_constant = s.blend_color;
        let alpha_constant = blend.is_some_and(|b| {
            b[..4]
                .iter()
                .any(|&f| matches!(f, CONSTANT_ALPHA | ONE_MINUS_CONSTANT_ALPHA))
        });
        let constant = if alpha_constant {
            [blend_constant[3]; 4]
        } else {
            blend_constant
        };
        let reference = s.stencil[0].reference as u32 & 0xFF;
        let pass = self.gpu.pass.as_mut().expect("begun");
        pass.commands.push(RenderCommand::SetPipeline { pipeline });
        pass.commands.push(RenderCommand::SetBindGroup {
            index: 0,
            group: Some(group),
            offsets: vec![uniform_offset as u32],
        });
        for (slot, (buffer, offset)) in slots.iter().enumerate() {
            pass.commands.push(RenderCommand::SetVertexBuffer {
                slot: slot as u32,
                buffer: *buffer,
                offset: *offset,
                size: None,
            });
        }
        pass.commands.push(RenderCommand::SetViewport {
            x: vx,
            y: vy,
            width: vw,
            height: vh,
            min_depth: s.depth_range[0],
            max_depth: s.depth_range[1],
        });
        pass.commands.push(RenderCommand::SetScissorRect {
            x: scissor.0,
            y: scissor.1,
            width: scissor.2,
            height: scissor.3,
        });
        pass.commands.push(RenderCommand::SetBlendConstant {
            r: f64::from(constant[0]),
            g: f64::from(constant[1]),
            b: f64::from(constant[2]),
            a: f64::from(constant[3]),
        });
        pass.commands
            .push(RenderCommand::SetStencilReference { reference });
        if vw <= 0.0 || vh <= 0.0 || scissor.2 == 0 || scissor.3 == 0 {
            return;
        }
        match index {
            Some((buffer, format, offset, n)) => {
                pass.commands.push(RenderCommand::SetIndexBuffer {
                    buffer,
                    format,
                    offset,
                    size: None,
                });
                pass.commands.push(RenderCommand::DrawIndexed {
                    indices: n,
                    instances,
                    first_index: 0,
                    base_vertex: 0,
                    first_instance: 0,
                });
            }
            None if points => {
                pass.commands.push(RenderCommand::Draw {
                    vertices: 4,
                    instances: count,
                    first_vertex: 0,
                    first_instance: first,
                });
            }
            None => {
                pass.commands.push(RenderCommand::Draw {
                    vertices: count,
                    instances,
                    first_vertex: first,
                    first_instance: 0,
                });
            }
        }
    }

    /// `bytes` in the arena; when it is full, what was recorded is sent
    /// and the pass begun again.
    fn arena_or_flush(&mut self, bytes: &[u8], align: u64, size: (u32, u32)) -> Option<u64> {
        if let Some(o) = self.gpu.arena(&self.client, bytes, align) {
            return Some(o);
        }
        self.gpu_flush();
        self.pass(size, None);
        self.gpu.arena(&self.client, bytes, align)
    }

    fn pipeline(&mut self, key: &PipelineKey, linked: &super::Linked) -> u32 {
        if let Some(&p) = self.gpu.pipelines.get(key) {
            return p;
        }
        if self.gpu.pipelines.len() >= MAX_PIPELINES
            && let Some(old) = self.gpu.order.pop_front()
            && let Some(id) = self.gpu.pipelines.remove(&old)
        {
            self.gpu.made.retain(|&m| m != id);
            self.client.queue([Request::Drop { id }]);
        }
        let id = self.gpu.id(&self.client);
        let buffers: Vec<String> = key
            .buffers
            .iter()
            .map(|(stride, instance, attributes)| {
                let list: Vec<String> = attributes
                    .iter()
                    .map(|(format, offset, location)| format!(r#"{{"format":"{format}","offset":{offset},"shaderLocation":{location}}}"#))
                    .collect();
                format!(
                    r#"{{"arrayStride":{stride},"stepMode":"{}","attributes":[{}]}}"#,
                    if *instance { "instance" } else { "vertex" },
                    list.join(",")
                )
            })
            .collect();
        let blend = key.blend.map_or(String::new(), |b| {
            format!(
                r#","blend":{{"color":{{"srcFactor":"{}","dstFactor":"{}","operation":"{}"}},"alpha":{{"srcFactor":"{}","dstFactor":"{}","operation":"{}"}}}}"#,
                blend_factor(b[0], false),
                blend_factor(b[1], false),
                blend_operation(b[4]),
                blend_factor(b[2], true),
                blend_factor(b[3], true),
                blend_operation(b[5])
            )
        });
        // The frame drawn upside down turns windings round: GL's
        // counter-clockwise front is WebGPU's clockwise.
        let front = if key.front_ccw { "cw" } else { "ccw" };
        let cull = match key.cull {
            1 => "front",
            2 => "back",
            _ => "none",
        };
        let strip = key
            .strip_index
            .map_or(String::new(), |f| format!(r#","stripIndexFormat":"{f}""#));
        let depth = key.depth.map_or(String::new(), |(func, write, bias, slope)| {
            let format = if self.attributes.stencil { "depth24plus-stencil8" } else { "depth24plus" };
            let stencil = match &key.stencil {
                Some(faces) => {
                    let face = |f: &StencilFace| format!(r#"{{"compare":"{}","failOp":"{}","depthFailOp":"{}","passOp":"{}"}}"#, compare(f.func), stencil_operation(f.fail), stencil_operation(f.depth_fail), stencil_operation(f.pass));
                    format!(
                        r#","stencilFront":{},"stencilBack":{},"stencilReadMask":{},"stencilWriteMask":{}"#,
                        face(&faces[0]),
                        face(&faces[1]),
                        faces[0].value_mask & 0xFF,
                        faces[0].write_mask & 0xFF
                    )
                }
                None => String::new(),
            };
            format!(
                r#","depthStencil":{{"format":"{format}","depthWriteEnabled":{write},"depthCompare":"{}","depthBias":{bias},"depthBiasSlopeScale":{}{stencil}}}"#,
                compare(func),
                f32::from_bits(slope)
            )
        });
        let vertex_module = if key.points {
            linked.points.unwrap_or(linked.vertex)
        } else {
            linked.vertex
        };
        let targets = format!(
            r#"[{{"format":"rgba8unorm","writeMask":{}{blend}}}]"#,
            if linked.program.outputs.is_empty() {
                0
            } else {
                key.write_mask
            }
        );
        let desc = format!(
            r#"{{"layout":{},"vertex":{{"module":{vertex_module},"entryPoint":"main","buffers":[{}]}},"primitive":{{"topology":"{}","frontFace":"{front}","cullMode":"{cull}"{strip}}},"multisample":{{"count":{},"mask":{},"alphaToCoverageEnabled":{}}},"fragment":{{"module":{},"entryPoint":"main","targets":{targets}}}{depth}}}"#,
            linked.pipeline_layout,
            buffers.join(","),
            key.topology,
            key.samples,
            key.mask,
            key.alpha_to_coverage,
            linked.fragment
        );
        self.client.queue([Request::CreateRenderPipeline {
            device: self.device,
            id,
            desc,
            call: None,
        }]);
        self.gpu.pipelines.insert(key.clone(), id);
        self.gpu.order.push_back(key.clone());
        id
    }

    /// The bind group of a program's uniforms (the arena, at a dynamic
    /// offset) and its samplers' textures.
    fn bind_group(&mut self, linked: &super::Linked) -> u32 {
        // Step 6: every sampler reads the default texture (textures are
        // step 7's).
        let mut resources = Vec::new();
        for s in &linked.program.samplers {
            let view = if s.sampler.dim == glsl::tir::Dim::Cube {
                self.gpu.defaults.1
            } else {
                self.gpu.defaults.0
            };
            for _ in &s.bindings {
                resources.push(view);
                resources.push(self.gpu.defaults.2);
            }
        }
        let key = (linked.layout, resources.clone());
        if let Some(&g) = self.gpu.groups.get(&key) {
            return g;
        }
        let id = self.gpu.id(&self.client);
        let mut entries = vec![format!(
            r#"{{"binding":0,"buffer":{},"offset":0,"size":{}}}"#,
            self.gpu.arena,
            linked.program.uniform_size.max(16)
        )];
        let mut k = 0;
        for s in &linked.program.samplers {
            for &binding in &s.bindings {
                entries.push(format!(
                    r#"{{"binding":{binding},"view":{}}}"#,
                    resources[k]
                ));
                entries.push(format!(
                    r#"{{"binding":{},"sampler":{}}}"#,
                    binding + 1,
                    resources[k + 1]
                ));
                k += 2;
            }
        }
        self.client.queue([Request::CreateBindGroup {
            device: self.device,
            id,
            desc: format!(
                r#"{{"layout":{},"entries":[{}]}}"#,
                linked.layout,
                entries.join(",")
            ),
        }]);
        self.gpu.groups.insert(key, id);
        id
    }

    // ---- Presentation ----

    /// The drawing buffer sent to the canvas: copied to the readback
    /// buffer, mapped, its rows (bottom first, as drawn) given to the
    /// document; then cleared before it is drawn again, unless
    /// preserved.
    pub(crate) fn present(&mut self, inbox: &Inbox) {
        self.gpu_flush();
        self.drawn = false;
        let Some(d) = self.drawing.clone() else {
            return;
        };
        let (width, height) = d.size;
        let stride = (u64::from(width) * 4).div_ceil(256) * 256;
        let size = stride * u64::from(height);
        let buffer = match self.gpu.readback {
            Some((id, had)) if had == size => id,
            _ => {
                let id = self.gpu.id(&self.client);
                self.client.queue([Request::CreateBuffer {
                    device: self.device,
                    id,
                    size,
                    usage: 9,
                    mapped: false,
                }]);
                self.gpu.readback = Some((id, size));
                id
            }
        };
        let copy = Json::object([
            ("source", Json::object([("texture", Json::from(d.color.0))])),
            (
                "destination",
                Json::object([
                    ("buffer", Json::from(buffer)),
                    ("bytesPerRow", Json::from(stride)),
                ]),
            ),
            (
                "size",
                Json::Array(vec![
                    Json::from(width),
                    Json::from(height),
                    Json::from(1u32),
                ]),
            ),
        ]);
        let commands = self.client.id();
        self.client.queue([
            Request::Finish {
                device: self.device,
                id: commands,
                commands: vec![Command::CopyTextureToBuffer { desc: copy.write() }],
            },
            Request::Submit {
                device: self.device,
                buffers: vec![commands],
            },
            Request::Drop { id: commands },
        ]);
        let inbox = inbox.clone();
        let handle = self.canvas;
        let premultiplied = self.attributes.premultiplied_alpha && self.attributes.alpha;
        let opaque = !self.attributes.alpha;
        self.client.call(
            |call| Request::MapBuffer {
                call,
                buffer,
                mode: 1,
                offset: 0,
                size,
            },
            move |reply, bytes| {
                if let Reply::Mapped { error: None, .. } = reply {
                    let frame = GpuFrame {
                        width,
                        height,
                        stride: stride as u32,
                        format: 0,
                        premultiplied,
                        opaque,
                        flip: true,
                        bytes,
                    };
                    let _ = inbox.send(WorkerMessage::GpuFrame(handle, frame));
                }
            },
        );
        self.client.queue([Request::Unmap { buffer }]);
        if !self.attributes.preserve_drawing_buffer {
            self.needs_clear = true;
        }
    }
}
