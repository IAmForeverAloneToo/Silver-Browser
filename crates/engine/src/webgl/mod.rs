//! A document's WebGL (M41 step 6, D40, docs/design/gpu.md): GL's state
//! machine in the renderer, over WebGPU. Each context has a device of
//! the GPU process, its drawing buffer there, and every GL object's
//! WebGPU objects; the bootstrap's WebGL objects call one native,
//! `native.webgl(op, ...)`, answered here at once, `getError` and every
//! query from this state, with no round trip. Draws are recorded into a
//! render pass, sent at the end of the task, or when something must be
//! read or written in order.

pub mod acceptance;
mod draw;
mod ext;
mod formats;
mod framebuffer;
pub(crate) mod gl;
mod texture;
mod webgl2;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use js::{Answer, CanvasCall, DomError};

use crate::gpu::GpuClient;
use crate::gpu::wire::{Reply, Request};
use crate::scripts::workers::Inbox;

use gl::*;

/// The most WebGL contexts a document keeps live; a new one past them
/// loses the oldest, as the reference does.
pub const MAX_WEBGL_CONTEXTS: usize = 16;

/// The most pipelines a context keeps; past them the oldest go.
pub const MAX_PIPELINES: usize = 1024;

/// The largest texture side, renderbuffer and viewport the contexts give.
pub const MAX_SIZE: u32 = 8192;

/// How long the renderer waits on the GPU process for what GL makes it
/// wait for (a context's device, `readPixels`).
const WAIT: Duration = Duration::from_secs(30);

/// What a call of the bootstrap needs of the document.
pub(crate) struct Env<'a> {
    pub client: Option<&'a Arc<GpuClient>>,
    pub inbox: &'a Inbox,
    /// The size of the canvas a handle names (an `OffscreenCanvas`'s by
    /// the negative of its number).
    pub canvas_size: &'a dyn Fn(f64) -> (u32, u32),
}

/// A context's attributes, as `getContextAttributes()` gives them.
#[derive(Clone, Debug)]
pub(crate) struct Attributes {
    pub alpha: bool,
    pub depth: bool,
    pub stencil: bool,
    pub antialias: bool,
    pub premultiplied_alpha: bool,
    pub preserve_drawing_buffer: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Buffer {
    /// The data, kept here too: WebGL checks draws against it, and
    /// lines' and fans' indices are made from it.
    pub data: Vec<u8>,
    pub usage: u32,
    /// What it was first bound to: an element array buffer stays one.
    pub target: Option<u32>,
    pub gpu: Option<(u32, u64)>,
    pub deleted: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Shader {
    pub kind: u32,
    pub source: String,
    pub compiled: Option<glsl::Shader>,
    pub compile_status: bool,
    pub log: String,
    pub deleted: bool,
    /// The programs it is attached to: deleted, it lives on while
    /// attached.
    pub attached: usize,
}

/// A linked program: its translation, its modules in the GPU process,
/// and its uniforms' values.
#[derive(Clone, Debug)]
pub(crate) struct Linked {
    pub program: glsl::Program,
    pub vertex: u32,
    pub fragment: u32,
    pub points: Option<u32>,
    /// The default block's bytes, as the uniform calls set them.
    pub uniforms: Vec<u8>,
    /// Each sampler element's texture unit.
    pub units: Vec<u32>,
    pub layout: u32,
    pub pipeline_layout: u32,
    /// Which link this was, its uniform locations' generation.
    pub generation: u32,
    /// Each active uniform block's binding point.
    pub block_bindings: Vec<u32>,
}

#[derive(Clone, Debug)]
pub(crate) struct Program {
    pub vertex: Option<u32>,
    pub fragment: Option<u32>,
    pub linked: Option<Linked>,
    pub link_status: bool,
    pub validate_status: bool,
    pub log: String,
    pub deleted: bool,
    pub bindings: Vec<(String, u32)>,
    pub links: u32,
}

/// What a uniform location names: a program's uniform or sampler,
/// one element of it.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Target {
    Uniform(usize, u32),
    Sampler(usize, u32),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Location {
    pub program: u32,
    pub generation: u32,
    pub target: Target,
}

/// A vertex attribute's array, and its value when disabled.
#[derive(Clone, Debug)]
pub(crate) struct Attrib {
    pub enabled: bool,
    pub buffer: Option<u32>,
    pub size: u32,
    pub kind: u32,
    pub normalized: bool,
    pub stride: u32,
    pub offset: u64,
    pub integer: bool,
    pub divisor: u32,
    /// The value of `vertexAttrib*`: four floats' bits, or ints'.
    pub value: [u32; 4],
    pub value_kind: u32,
}

impl Default for Attrib {
    fn default() -> Attrib {
        Attrib {
            enabled: false,
            buffer: None,
            size: 4,
            kind: FLOAT,
            normalized: false,
            stride: 0,
            offset: 0,
            integer: false,
            divisor: 0,
            value: [0, 0, 0, 1.0f32.to_bits()],
            value_kind: FLOAT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct StencilFace {
    pub func: u32,
    pub reference: i32,
    pub value_mask: u32,
    pub write_mask: u32,
    pub fail: u32,
    pub depth_fail: u32,
    pub pass: u32,
}

/// GL's state a context keeps (the parts of it step 6 draws by).
#[derive(Clone, Debug)]
pub(crate) struct State {
    pub viewport: [i32; 4],
    pub scissor: [i32; 4],
    pub scissor_test: bool,
    pub clear_color: [f32; 4],
    pub clear_depth: f32,
    pub clear_stencil: i32,
    pub color_mask: [bool; 4],
    pub depth_test: bool,
    pub depth_func: u32,
    pub depth_mask: bool,
    pub depth_range: [f32; 2],
    pub blend: bool,
    pub blend_src: [u32; 2],
    pub blend_dst: [u32; 2],
    pub blend_equation: [u32; 2],
    pub blend_color: [f32; 4],
    pub cull: bool,
    pub cull_mode: u32,
    pub front_face: u32,
    pub polygon_offset: bool,
    pub polygon_offset_factor: f32,
    pub polygon_offset_units: f32,
    pub stencil_test: bool,
    pub stencil: [StencilFace; 2],
    pub dither: bool,
    pub sample_alpha_to_coverage: bool,
    pub sample_coverage: bool,
    pub sample_coverage_value: f32,
    pub sample_coverage_invert: bool,
    pub line_width: f32,
    pub pack_alignment: u32,
    pub unpack_alignment: u32,
    pub unpack_flip_y: bool,
    pub unpack_premultiply_alpha: bool,
    pub unpack_colorspace_conversion: u32,
    pub generate_mipmap_hint: u32,
    pub active_texture: u32,
    /// WebGL 2's pixel store state.
    pub unpack_row_length: u32,
    pub unpack_image_height: u32,
    pub unpack_skip_pixels: u32,
    pub unpack_skip_rows: u32,
    pub unpack_skip_images: u32,
    pub pack_row_length: u32,
    pub pack_skip_pixels: u32,
    pub pack_skip_rows: u32,
}

impl State {
    fn new(size: (u32, u32)) -> State {
        let face = StencilFace {
            func: ALWAYS,
            reference: 0,
            value_mask: u32::MAX,
            write_mask: u32::MAX,
            fail: KEEP,
            depth_fail: KEEP,
            pass: KEEP,
        };
        let (w, h) = (size.0 as i32, size.1 as i32);
        State {
            viewport: [0, 0, w, h],
            scissor: [0, 0, w, h],
            scissor_test: false,
            clear_color: [0.0; 4],
            clear_depth: 1.0,
            clear_stencil: 0,
            color_mask: [true; 4],
            depth_test: false,
            depth_func: LESS,
            depth_mask: true,
            depth_range: [0.0, 1.0],
            blend: false,
            blend_src: [ONE, ONE],
            blend_dst: [ZERO, ZERO],
            blend_equation: [FUNC_ADD, FUNC_ADD],
            blend_color: [0.0; 4],
            cull: false,
            cull_mode: BACK,
            front_face: CCW,
            polygon_offset: false,
            polygon_offset_factor: 0.0,
            polygon_offset_units: 0.0,
            stencil_test: false,
            stencil: [face, face],
            dither: true,
            sample_alpha_to_coverage: false,
            sample_coverage: false,
            sample_coverage_value: 1.0,
            sample_coverage_invert: false,
            line_width: 1.0,
            pack_alignment: 4,
            unpack_alignment: 4,
            unpack_flip_y: false,
            unpack_premultiply_alpha: false,
            unpack_colorspace_conversion: BROWSER_DEFAULT_WEBGL,
            generate_mipmap_hint: DONT_CARE,
            active_texture: TEXTURE0,
            unpack_row_length: 0,
            unpack_image_height: 0,
            unpack_skip_pixels: 0,
            unpack_skip_rows: 0,
            unpack_skip_images: 0,
            pack_row_length: 0,
            pack_skip_pixels: 0,
            pack_skip_rows: 0,
        }
    }
}

/// A context's drawing buffer in the GPU process: its color texture (the
/// resolved one when antialiased), its multisampled texture, its depth
/// and stencil texture, each with a view.
#[derive(Clone, Debug)]
pub(crate) struct DrawingBuffer {
    pub size: (u32, u32),
    pub color: (u32, u32),
    pub msaa: Option<(u32, u32)>,
    pub depth: Option<(u32, u32)>,
}

pub(crate) struct Context {
    pub version: u8,
    pub canvas: f64,
    pub attributes: Attributes,
    pub lost: bool,
    pub errors: Vec<u32>,
    pub device: u32,
    pub client: Arc<GpuClient>,
    pub drawing: Option<DrawingBuffer>,
    /// Whether the drawing buffer is to be cleared before it is drawn
    /// into again: after it was presented, unless preserved.
    pub needs_clear: bool,
    /// Whether it was drawn into since it was last presented.
    pub drawn: bool,
    pub buffers: HashMap<u32, Buffer>,
    pub shaders: HashMap<u32, Shader>,
    pub programs: HashMap<u32, Program>,
    pub locations: HashMap<u32, Location>,
    next: u32,
    pub array_buffer: Option<u32>,
    pub element_buffer: Option<u32>,
    pub attribs: Vec<Attrib>,
    pub program: Option<u32>,
    pub state: State,
    pub gpu: draw::Gpu,
    pub surfaces: framebuffer::Surfaces,
    pub features: ext::Features,
    pub ext: ext::Enabled,
    pub v2: webgl2::State2,
}

impl Context {
    fn error(&mut self, code: u32) {
        if !self.errors.contains(&code) {
            self.errors.push(code);
        }
    }

    /// A shader detached: gone if deleted and attached nowhere.
    fn release_shader(&mut self, id: u32) {
        if let Some(s) = self.shaders.get_mut(&id) {
            s.attached = s.attached.saturating_sub(1);
            if s.deleted && s.attached == 0 {
                self.shaders.remove(&id);
            }
        }
    }

    /// A deleted program gone once not in use, its shaders detached.
    fn free_program(&mut self, id: u32) {
        if self.program == Some(id) || !self.programs.get(&id).is_some_and(|p| p.deleted) {
            return;
        }
        if let Some(p) = self.programs.remove(&id) {
            for shader in [p.vertex, p.fragment].into_iter().flatten() {
                self.release_shader(shader);
            }
        }
    }

    fn id(&mut self) -> u32 {
        self.next += 1;
        self.next
    }

    /// A buffer object's target binding, by `target`.
    fn bound_buffer(&self, target: u32) -> Result<Option<u32>, u32> {
        let v2 = self.version == 2;
        match target {
            ARRAY_BUFFER => Ok(self.array_buffer),
            ELEMENT_ARRAY_BUFFER => Ok(self.element_buffer),
            COPY_READ_BUFFER if v2 => Ok(self.v2.copy_read),
            COPY_WRITE_BUFFER if v2 => Ok(self.v2.copy_write),
            PIXEL_PACK_BUFFER if v2 => Ok(self.v2.pixel_pack),
            PIXEL_UNPACK_BUFFER if v2 => Ok(self.v2.pixel_unpack),
            UNIFORM_BUFFER if v2 => Ok(self.v2.uniform),
            TRANSFORM_FEEDBACK_BUFFER if v2 => Ok(self.v2.feedback_buffer),
            _ => Err(INVALID_ENUM),
        }
    }
}

/// One WebGL context as the fuzz targets drive it (M41, `webgl_ops`):
/// its calls made as the bootstrap makes them, of a canvas of 64 by 32.
pub struct Harness {
    webgl: WebGl,
    client: Arc<GpuClient>,
    inbox: Inbox,
    _mailbox: crate::scripts::workers::Mailbox,
    context: Option<u32>,
}

impl Harness {
    /// A harness of the GPU service `client` reaches.
    pub fn new(client: Arc<GpuClient>) -> Harness {
        let (inbox, mailbox) = crate::scripts::workers::inbox(None);
        Harness {
            webgl: WebGl::default(),
            client,
            inbox,
            _mailbox: mailbox,
            context: None,
        }
    }

    /// A new context of `version` (1 or 2), the one called from now on:
    /// false when none could be made.
    pub fn create(&mut self, version: u8) -> bool {
        let call = CanvasCall {
            op: "create".to_owned(),
            numbers: vec![1.0, f64::from(version), 1.0, 1.0, 1.0, 0.0, 1.0, 0.0],
            strings: Vec::new(),
            bytes: None,
        };
        let client = Arc::clone(&self.client);
        let env = Env {
            client: Some(&client),
            inbox: &self.inbox,
            canvas_size: &|_| (64, 32),
        };
        self.context = match self.webgl.call(call, &env) {
            Ok(Answer::Number(id)) if id > 0.0 => Some(id as u32),
            _ => None,
        };
        self.context.is_some()
    }

    /// Whether a context was made and is not lost.
    pub fn made(&self) -> bool {
        self.context
            .and_then(|id| self.webgl.contexts.get(&id))
            .is_some_and(|c| !c.lost)
    }

    /// A call of the context: its operation, the numbers after the
    /// context's, its strings and bytes.
    pub fn call(
        &mut self,
        op: &str,
        numbers: &[f64],
        strings: Vec<String>,
        bytes: Option<Vec<u8>>,
    ) {
        let Some(id) = self.context else {
            return;
        };
        let mut all = vec![f64::from(id)];
        all.extend_from_slice(numbers);
        let call = CanvasCall {
            op: op.to_owned(),
            numbers: all,
            strings,
            bytes,
        };
        let client = Arc::clone(&self.client);
        let env = Env {
            client: Some(&client),
            inbox: &self.inbox,
            canvas_size: &|_| (64, 32),
        };
        let _ = self.webgl.call(call, &env);
    }
}

/// A document's WebGL contexts.
#[derive(Default)]
pub(crate) struct WebGl {
    contexts: BTreeMap<u32, Context>,
    next: u32,
    /// Their order of making, the oldest first, for MAX_WEBGL_CONTEXTS.
    made: Vec<u32>,
}

fn n(call: &CanvasCall, i: usize) -> f64 {
    call.numbers.get(i).copied().unwrap_or(0.0)
}

fn u(call: &CanvasCall, i: usize) -> u32 {
    let v = n(call, i);
    if v.is_finite() && v >= 0.0 {
        v.min(f64::from(u32::MAX)) as u32
    } else {
        0
    }
}

fn i(call: &CanvasCall, i: usize) -> i32 {
    let v = n(call, i);
    if v.is_finite() {
        v.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
    } else {
        0
    }
}

fn b(call: &CanvasCall, i: usize) -> bool {
    n(call, i) != 0.0
}

fn f(call: &CanvasCall, i: usize) -> f32 {
    n(call, i) as f32
}

/// The GPU's answer to `ask`, waited for.
pub(crate) fn wait(
    client: &Arc<GpuClient>,
    ask: impl FnOnce(u32) -> Request,
) -> Option<(Reply, Vec<u8>)> {
    let (heard, hear) = mpsc::channel();
    client.call(ask, move |reply, bytes| {
        let _ = heard.send((reply, bytes));
    });
    client.flush();
    hear.recv_timeout(WAIT).ok()
}

impl WebGl {
    /// One call of `native.webgl`.
    pub(crate) fn call(&mut self, call: CanvasCall, env: &Env<'_>) -> Result<Answer, DomError> {
        if call.op == "create" {
            return Ok(self.create(&call, env));
        }
        let id = u(&call, 0);
        let Some(context) = self.contexts.get_mut(&id) else {
            return Err(DomError::new("TypeError", "Illegal invocation"));
        };
        // A lost context answers its queries as lost ones do, and does
        // nothing (WebGL 5.15.2).
        if context.lost {
            // Restored (WEBGL_lose_context, or after its lost event's
            // default was prevented): a new context of the same kind.
            if call.op == "restoreContext" {
                let Some(client) = env.client else {
                    return Ok(Answer::Bool(false));
                };
                let (canvas, version, attributes) =
                    (context.canvas, context.version, context.attributes.clone());
                let size = (env.canvas_size)(canvas);
                let Some(fresh) = Context::fresh(client, canvas, version, attributes, size) else {
                    return Ok(Answer::Bool(false));
                };
                self.contexts.insert(id, fresh);
                if !self.made.contains(&id) {
                    self.made.push(id);
                }
                return Ok(Answer::Bool(true));
            }
            return Ok(lost_answer(&call.op, context));
        }
        if call.op == "restoreContext" {
            return Ok(Answer::Bool(false));
        }
        // Its GPU process gone: lost.
        if context.client.is_lost() {
            context.lose();
            return Ok(lost_answer(&call.op, context));
        }
        let size = (env.canvas_size)(context.canvas);
        context.op(&call, size, env)
    }

    /// A new context of `getContext`: its number, or null when there is
    /// no GPU to draw with.
    fn create(&mut self, call: &CanvasCall, env: &Env<'_>) -> Answer {
        let Some(client) = env.client else {
            return Answer::Number(-1.0);
        };
        let canvas = n(call, 0);
        let version = if u(call, 1) == 2 { 2 } else { 1 };
        let attributes = Attributes {
            alpha: b(call, 2),
            depth: b(call, 3),
            stencil: b(call, 4),
            antialias: b(call, 5),
            premultiplied_alpha: b(call, 6),
            preserve_drawing_buffer: b(call, 7),
        };
        let size = (env.canvas_size)(canvas);
        let Some(context) = Context::fresh(client, canvas, version, attributes, size) else {
            return Answer::Number(-1.0);
        };
        // Past the most contexts, the oldest is lost.
        if self.made.len() >= MAX_WEBGL_CONTEXTS {
            let oldest = self.made.remove(0);
            if let Some(old) = self.contexts.get_mut(&oldest) {
                old.lose();
            }
        }
        self.next += 1;
        let id = self.next;
        self.contexts.insert(id, context);
        self.made.push(id);
        Answer::Number(f64::from(id))
    }
}

impl Context {
    /// A context of a new device, in GL's first state; none without a
    /// GPU.
    fn fresh(
        client: &Arc<GpuClient>,
        canvas: f64,
        version: u8,
        attributes: Attributes,
        size: (u32, u32),
    ) -> Option<Context> {
        let (device, features) = draw::device(client)?;
        let gpu = draw::Gpu::new(client, device);
        Some(Context {
            version,
            canvas,
            attributes,
            lost: false,
            errors: Vec::new(),
            device,
            client: client.clone(),
            drawing: None,
            needs_clear: true,
            drawn: false,
            buffers: HashMap::new(),
            shaders: HashMap::new(),
            programs: HashMap::new(),
            locations: HashMap::new(),
            next: 0,
            array_buffer: None,
            element_buffer: None,
            attribs: vec![Attrib::default(); 16],
            program: None,
            state: State::new(size),
            gpu,
            surfaces: framebuffer::Surfaces::default(),
            features,
            ext: ext::Enabled::default(),
            v2: webgl2::State2::default(),
        })
    }
}

impl Drop for WebGl {
    fn drop(&mut self) {
        for context in self.contexts.values_mut() {
            context.lose();
        }
    }
}

/// What a lost context answers.
fn lost_answer(op: &str, context: &mut Context) -> Answer {
    match op {
        "getError" => {
            let lost = !context.errors.contains(&CONTEXT_LOST_WEBGL);
            if lost {
                context.errors.push(CONTEXT_LOST_WEBGL);
                Answer::Number(f64::from(CONTEXT_LOST_WEBGL))
            } else {
                Answer::Number(0.0)
            }
        }
        "isContextLost" => Answer::Bool(true),
        "checkFramebufferStatus" => Answer::Number(0.0),
        _ => Answer::Nothing,
    }
}

/// GL's name for a usage, a buffer's.
fn usage_ok(usage: u32, version: u8) -> bool {
    matches!(usage, STREAM_DRAW | STATIC_DRAW | DYNAMIC_DRAW)
        || (version == 2
            && matches!(
                usage,
                STATIC_READ | STATIC_COPY | STREAM_READ | STREAM_COPY | DYNAMIC_READ | DYNAMIC_COPY
            ))
}

impl Context {
    fn op(
        &mut self,
        call: &CanvasCall,
        size: (u32, u32),
        env: &Env<'_>,
    ) -> Result<Answer, DomError> {
        let op = call.op.as_str();
        let answer = match op {
            "getError" => {
                let code = if self.errors.is_empty() {
                    NO_ERROR
                } else {
                    self.errors.remove(0)
                };
                Answer::Number(f64::from(code))
            }
            "isContextLost" => Answer::Bool(false),
            // An error the bindings found: an object of another context,
            // or a null buffer's data.
            "error" => {
                let code = u(call, 1);
                if matches!(code, INVALID_ENUM | INVALID_VALUE | INVALID_OPERATION) {
                    self.error(code);
                }
                Answer::Nothing
            }
            "loseContext" => {
                self.lose();
                Answer::Nothing
            }
            "drawingBufferSize" => {
                let (w, h) = clamp_size(size);
                Answer::Numbers(vec![f64::from(w), f64::from(h)])
            }
            // ---- State ----
            "enable" | "disable" => {
                let on = op == "enable";
                let cap = u(call, 1);
                match cap {
                    BLEND => self.state.blend = on,
                    CULL_FACE => self.state.cull = on,
                    DEPTH_TEST => self.state.depth_test = on,
                    DITHER => self.state.dither = on,
                    POLYGON_OFFSET_FILL => self.state.polygon_offset = on,
                    SAMPLE_ALPHA_TO_COVERAGE => self.state.sample_alpha_to_coverage = on,
                    SAMPLE_COVERAGE => self.state.sample_coverage = on,
                    SCISSOR_TEST => self.state.scissor_test = on,
                    STENCIL_TEST => self.state.stencil_test = on,
                    RASTERIZER_DISCARD | PRIMITIVE_RESTART_FIXED_INDEX if self.version == 2 => {}
                    _ => self.error(INVALID_ENUM),
                }
                Answer::Nothing
            }
            "isEnabled" => Answer::Bool(match u(call, 1) {
                BLEND => self.state.blend,
                CULL_FACE => self.state.cull,
                DEPTH_TEST => self.state.depth_test,
                DITHER => self.state.dither,
                POLYGON_OFFSET_FILL => self.state.polygon_offset,
                SAMPLE_ALPHA_TO_COVERAGE => self.state.sample_alpha_to_coverage,
                SAMPLE_COVERAGE => self.state.sample_coverage,
                SCISSOR_TEST => self.state.scissor_test,
                STENCIL_TEST => self.state.stencil_test,
                RASTERIZER_DISCARD | PRIMITIVE_RESTART_FIXED_INDEX if self.version == 2 => false,
                _ => {
                    self.error(INVALID_ENUM);
                    false
                }
            }),
            "viewport" | "scissor" => {
                let (w, h) = (i(call, 3), i(call, 4));
                if w < 0 || h < 0 {
                    self.error(INVALID_VALUE);
                } else {
                    let rect = [
                        i(call, 1),
                        i(call, 2),
                        w.min(MAX_SIZE as i32),
                        h.min(MAX_SIZE as i32),
                    ];
                    if op == "viewport" {
                        self.state.viewport = rect;
                    } else {
                        self.state.scissor = rect;
                    }
                }
                Answer::Nothing
            }
            "clearColor" => {
                // GL ES clamps them (2.0 and 3.0 alike).
                self.state.clear_color = [1, 2, 3, 4].map(|k| f(call, k).clamp(0.0, 1.0));
                Answer::Nothing
            }
            "clearDepth" => {
                self.state.clear_depth = f(call, 1).clamp(0.0, 1.0);
                Answer::Nothing
            }
            "clearStencil" => {
                self.state.clear_stencil = i(call, 1);
                Answer::Nothing
            }
            "colorMask" => {
                self.state.color_mask = [b(call, 1), b(call, 2), b(call, 3), b(call, 4)];
                Answer::Nothing
            }
            "depthMask" => {
                self.state.depth_mask = b(call, 1);
                Answer::Nothing
            }
            "depthFunc" => {
                let func = u(call, 1);
                if (NEVER..=ALWAYS).contains(&func) {
                    self.state.depth_func = func;
                } else {
                    self.error(INVALID_ENUM);
                }
                Answer::Nothing
            }
            "depthRange" => {
                let (near, far) = (f(call, 1).clamp(0.0, 1.0), f(call, 2).clamp(0.0, 1.0));
                // WebGL refuses a range turned around (WebGL 6.12).
                if near > far {
                    self.error(INVALID_OPERATION);
                } else {
                    self.state.depth_range = [near, far];
                }
                Answer::Nothing
            }
            "blendColor" => {
                self.state.blend_color = [1, 2, 3, 4].map(|k| f(call, k).clamp(0.0, 1.0));
                Answer::Nothing
            }
            "blendEquation" | "blendEquationSeparate" => {
                let (rgb, alpha) = if op == "blendEquation" {
                    (u(call, 1), u(call, 1))
                } else {
                    (u(call, 1), u(call, 2))
                };
                let ok = |e: u32| {
                    matches!(e, FUNC_ADD | FUNC_SUBTRACT | FUNC_REVERSE_SUBTRACT)
                        || (matches!(e, MIN | MAX)
                            && (self.version == 2 || self.gpu.ext_blend_minmax))
                };
                if ok(rgb) && ok(alpha) {
                    self.state.blend_equation = [rgb, alpha];
                } else {
                    self.error(INVALID_ENUM);
                }
                Answer::Nothing
            }
            "blendFunc" | "blendFuncSeparate" => {
                let (src_rgb, dst_rgb, src_a, dst_a) = if op == "blendFunc" {
                    (u(call, 1), u(call, 2), u(call, 1), u(call, 2))
                } else {
                    (u(call, 1), u(call, 2), u(call, 3), u(call, 4))
                };
                let factor = |x: u32| {
                    matches!(
                        x,
                        ZERO | ONE
                            | SRC_COLOR
                            | ONE_MINUS_SRC_COLOR
                            | SRC_ALPHA
                            | ONE_MINUS_SRC_ALPHA
                            | DST_ALPHA
                            | ONE_MINUS_DST_ALPHA
                            | DST_COLOR
                            | ONE_MINUS_DST_COLOR
                            | SRC_ALPHA_SATURATE
                            | CONSTANT_COLOR
                            | ONE_MINUS_CONSTANT_COLOR
                            | CONSTANT_ALPHA
                            | ONE_MINUS_CONSTANT_ALPHA
                    )
                };
                let constant_color =
                    |x: u32| matches!(x, CONSTANT_COLOR | ONE_MINUS_CONSTANT_COLOR);
                let constant_alpha =
                    |x: u32| matches!(x, CONSTANT_ALPHA | ONE_MINUS_CONSTANT_ALPHA);
                if ![src_rgb, dst_rgb, src_a, dst_a].iter().all(|&x| factor(x))
                    || (self.version == 1 && [dst_rgb, dst_a].contains(&SRC_ALPHA_SATURATE))
                {
                    self.error(INVALID_ENUM);
                } else if (constant_color(src_rgb) && constant_alpha(dst_rgb))
                    || (constant_alpha(src_rgb) && constant_color(dst_rgb))
                {
                    // WebGL refuses a constant color with a constant
                    // alpha (WebGL 6.13).
                    self.error(INVALID_OPERATION);
                } else {
                    self.state.blend_src = [src_rgb, src_a];
                    self.state.blend_dst = [dst_rgb, dst_a];
                }
                Answer::Nothing
            }
            "cullFace" => {
                let mode = u(call, 1);
                if matches!(mode, FRONT | BACK | FRONT_AND_BACK) {
                    self.state.cull_mode = mode;
                } else {
                    self.error(INVALID_ENUM);
                }
                Answer::Nothing
            }
            "frontFace" => {
                let mode = u(call, 1);
                if matches!(mode, CW | CCW) {
                    self.state.front_face = mode;
                } else {
                    self.error(INVALID_ENUM);
                }
                Answer::Nothing
            }
            "lineWidth" => {
                let w = f(call, 1);
                if w <= 0.0 || !w.is_finite() {
                    self.error(INVALID_VALUE);
                } else {
                    self.state.line_width = w;
                }
                Answer::Nothing
            }
            "polygonOffset" => {
                self.state.polygon_offset_factor = f(call, 1);
                self.state.polygon_offset_units = f(call, 2);
                Answer::Nothing
            }
            "sampleCoverage" => {
                self.state.sample_coverage_value = f(call, 1).clamp(0.0, 1.0);
                self.state.sample_coverage_invert = b(call, 2);
                Answer::Nothing
            }
            "stencilFunc" | "stencilFuncSeparate" => {
                let (faces, at) = if op == "stencilFunc" {
                    (FRONT_AND_BACK, 1)
                } else {
                    (u(call, 1), 2)
                };
                let func = u(call, at);
                if !(NEVER..=ALWAYS).contains(&func)
                    || !matches!(faces, FRONT | BACK | FRONT_AND_BACK)
                {
                    self.error(INVALID_ENUM);
                } else {
                    for k in face_indices(faces) {
                        let face = &mut self.state.stencil[k];
                        face.func = func;
                        face.reference = i(call, at + 1);
                        face.value_mask = u(call, at + 2);
                    }
                }
                Answer::Nothing
            }
            "stencilMask" | "stencilMaskSeparate" => {
                let (faces, at) = if op == "stencilMask" {
                    (FRONT_AND_BACK, 1)
                } else {
                    (u(call, 1), 2)
                };
                if !matches!(faces, FRONT | BACK | FRONT_AND_BACK) {
                    self.error(INVALID_ENUM);
                } else {
                    for k in face_indices(faces) {
                        self.state.stencil[k].write_mask = u(call, at);
                    }
                }
                Answer::Nothing
            }
            "stencilOp" | "stencilOpSeparate" => {
                let (faces, at) = if op == "stencilOp" {
                    (FRONT_AND_BACK, 1)
                } else {
                    (u(call, 1), 2)
                };
                let ops = [u(call, at), u(call, at + 1), u(call, at + 2)];
                let ok = |x: u32| {
                    matches!(
                        x,
                        ZERO | KEEP | REPLACE | INCR | DECR | INVERT | INCR_WRAP | DECR_WRAP
                    )
                };
                if !ops.iter().all(|&x| ok(x)) || !matches!(faces, FRONT | BACK | FRONT_AND_BACK) {
                    self.error(INVALID_ENUM);
                } else {
                    for k in face_indices(faces) {
                        let face = &mut self.state.stencil[k];
                        face.fail = ops[0];
                        face.depth_fail = ops[1];
                        face.pass = ops[2];
                    }
                }
                Answer::Nothing
            }
            "hint" => {
                let derivatives = self.version == 2 || self.ext.on("OES_standard_derivatives");
                if u(call, 1) == GENERATE_MIPMAP_HINT
                    && matches!(u(call, 2), DONT_CARE | FASTEST | NICEST)
                {
                    self.state.generate_mipmap_hint = u(call, 2);
                } else if u(call, 1) == FRAGMENT_SHADER_DERIVATIVE_HINT_OES
                    && derivatives
                    && matches!(u(call, 2), DONT_CARE | FASTEST | NICEST)
                {
                    self.surfaces.derivative_hint = u(call, 2);
                } else {
                    self.error(INVALID_ENUM);
                }
                Answer::Nothing
            }
            "pixelStorei" => {
                let (name, value) = (u(call, 1), i(call, 2));
                match name {
                    PACK_ALIGNMENT | UNPACK_ALIGNMENT if [1, 2, 4, 8].contains(&value) => {
                        if name == PACK_ALIGNMENT {
                            self.state.pack_alignment = value as u32;
                        } else {
                            self.state.unpack_alignment = value as u32;
                        }
                    }
                    PACK_ALIGNMENT | UNPACK_ALIGNMENT => self.error(INVALID_VALUE),
                    UNPACK_ROW_LENGTH | UNPACK_IMAGE_HEIGHT | UNPACK_SKIP_PIXELS
                    | UNPACK_SKIP_ROWS | UNPACK_SKIP_IMAGES | PACK_ROW_LENGTH
                    | PACK_SKIP_PIXELS | PACK_SKIP_ROWS
                        if self.version == 2 =>
                    {
                        if value < 0 {
                            self.error(INVALID_VALUE);
                        } else {
                            let s = &mut self.state;
                            *match name {
                                UNPACK_ROW_LENGTH => &mut s.unpack_row_length,
                                UNPACK_IMAGE_HEIGHT => &mut s.unpack_image_height,
                                UNPACK_SKIP_PIXELS => &mut s.unpack_skip_pixels,
                                UNPACK_SKIP_ROWS => &mut s.unpack_skip_rows,
                                UNPACK_SKIP_IMAGES => &mut s.unpack_skip_images,
                                PACK_ROW_LENGTH => &mut s.pack_row_length,
                                PACK_SKIP_PIXELS => &mut s.pack_skip_pixels,
                                _ => &mut s.pack_skip_rows,
                            } = value as u32;
                        }
                    }
                    UNPACK_FLIP_Y_WEBGL => self.state.unpack_flip_y = value != 0,
                    UNPACK_PREMULTIPLY_ALPHA_WEBGL => {
                        self.state.unpack_premultiply_alpha = value != 0
                    }
                    UNPACK_COLORSPACE_CONVERSION_WEBGL => {
                        if value as u32 == BROWSER_DEFAULT_WEBGL || value == 0 {
                            self.state.unpack_colorspace_conversion = value as u32;
                        } else {
                            self.error(INVALID_VALUE);
                        }
                    }
                    _ => self.error(INVALID_ENUM),
                }
                Answer::Nothing
            }
            "activeTexture" => {
                let unit = u(call, 1);
                if !(TEXTURE0..TEXTURE0 + 32).contains(&unit) {
                    self.error(INVALID_ENUM);
                } else {
                    self.state.active_texture = unit;
                }
                Answer::Nothing
            }
            "flush" | "finish" => {
                self.gpu_flush();
                Answer::Nothing
            }
            "getParameter" => self.parameter(u(call, 1)),
            // ---- Buffers ----
            "createBuffer" => {
                let id = self.id();
                self.buffers.insert(
                    id,
                    Buffer {
                        data: Vec::new(),
                        usage: STATIC_DRAW,
                        target: None,
                        gpu: None,
                        deleted: false,
                    },
                );
                Answer::Number(f64::from(id))
            }
            "bindBuffer" => {
                let (target, id) = (u(call, 1), u(call, 2));
                if let Err(code) = self.bound_buffer(target) {
                    self.error(code);
                    return Ok(Answer::Nothing);
                }
                if id != 0 {
                    match self.buffers.get_mut(&id) {
                        None => {
                            self.error(INVALID_OPERATION);
                            return Ok(Answer::Nothing);
                        }
                        Some(buffer) if buffer.deleted => {
                            self.error(INVALID_OPERATION);
                            return Ok(Answer::Nothing);
                        }
                        Some(buffer) => {
                            // An element array buffer is one for good, and
                            // no other is one (WebGL 6.1); the copy targets
                            // take either and make neither (WebGL 2 5.1).
                            let element = |t: u32| t == ELEMENT_ARRAY_BUFFER;
                            let neutral = |t: u32| t == COPY_READ_BUFFER || t == COPY_WRITE_BUFFER;
                            if !neutral(target) {
                                if let Some(first) = buffer.target
                                    && !neutral(first)
                                    && element(first) != element(target)
                                {
                                    self.error(INVALID_OPERATION);
                                    return Ok(Answer::Nothing);
                                }
                                if buffer.target.is_none_or(neutral) {
                                    buffer.target = Some(target);
                                }
                            }
                            buffer.target.get_or_insert(target);
                        }
                    }
                }
                let value = if id == 0 { None } else { Some(id) };
                match target {
                    ARRAY_BUFFER => self.array_buffer = value,
                    ELEMENT_ARRAY_BUFFER => self.element_buffer = value,
                    COPY_READ_BUFFER => self.v2.copy_read = value,
                    COPY_WRITE_BUFFER => self.v2.copy_write = value,
                    PIXEL_PACK_BUFFER => self.v2.pixel_pack = value,
                    PIXEL_UNPACK_BUFFER => self.v2.pixel_unpack = value,
                    UNIFORM_BUFFER => self.v2.uniform = value,
                    _ => self.v2.feedback_buffer = value,
                }
                Answer::Nothing
            }
            "bufferData" => {
                let (target, usage) = (u(call, 1), u(call, 3));
                let bound = match self.bound_buffer(target) {
                    Ok(bound) => bound,
                    Err(code) => {
                        self.error(code);
                        return Ok(Answer::Nothing);
                    }
                };
                if !usage_ok(usage, self.version) {
                    self.error(INVALID_ENUM);
                    return Ok(Answer::Nothing);
                }
                let Some(id) = bound else {
                    self.error(INVALID_OPERATION);
                    return Ok(Answer::Nothing);
                };
                // Data, or a size of zeros.
                let data = match &call.bytes {
                    Some(bytes) => bytes.clone(),
                    None => {
                        let size = n(call, 2);
                        if size < 0.0 {
                            self.error(INVALID_VALUE);
                            return Ok(Answer::Nothing);
                        }
                        if size > f64::from(crate::gpu::wire::GPU_BATCH as u32) * 64.0 {
                            self.error(OUT_OF_MEMORY);
                            return Ok(Answer::Nothing);
                        }
                        vec![0; size as usize]
                    }
                };
                self.buffer_data(id, data, usage);
                Answer::Nothing
            }
            "bufferSubData" => {
                let target = u(call, 1);
                let offset = n(call, 2);
                let bound = match self.bound_buffer(target) {
                    Ok(bound) => bound,
                    Err(code) => {
                        self.error(code);
                        return Ok(Answer::Nothing);
                    }
                };
                let Some(id) = bound else {
                    self.error(INVALID_OPERATION);
                    return Ok(Answer::Nothing);
                };
                let bytes = call.bytes.clone().unwrap_or_default();
                if offset < 0.0 {
                    self.error(INVALID_VALUE);
                    return Ok(Answer::Nothing);
                }
                let offset = offset as usize;
                let len = self.buffers[&id].data.len();
                if offset.checked_add(bytes.len()).is_none_or(|end| end > len) {
                    self.error(INVALID_VALUE);
                    return Ok(Answer::Nothing);
                }
                self.buffer_sub_data(id, offset, &bytes);
                Answer::Nothing
            }
            "deleteBuffer" => {
                let id = u(call, 1);
                if let Some(buffer) = self.buffers.get_mut(&id) {
                    buffer.deleted = true;
                    if self.array_buffer == Some(id) {
                        self.array_buffer = None;
                    }
                    if self.element_buffer == Some(id) {
                        self.element_buffer = None;
                    }
                    for attrib in &mut self.attribs {
                        if attrib.buffer == Some(id) {
                            attrib.buffer = None;
                        }
                    }
                    let v2 = &mut self.v2;
                    for slot in [
                        &mut v2.copy_read,
                        &mut v2.copy_write,
                        &mut v2.pixel_pack,
                        &mut v2.pixel_unpack,
                        &mut v2.uniform,
                        &mut v2.feedback_buffer,
                    ] {
                        if *slot == Some(id) {
                            *slot = None;
                        }
                    }
                    for slot in v2
                        .uniform_bindings
                        .iter_mut()
                        .chain(v2.feedback_bindings.iter_mut())
                    {
                        if slot.is_some_and(|(b, _, _)| b == id) {
                            *slot = None;
                        }
                    }
                }
                Answer::Nothing
            }
            "isBuffer" => {
                let id = u(call, 1);
                Answer::Bool(
                    self.buffers
                        .get(&id)
                        .is_some_and(|b| !b.deleted && b.target.is_some()),
                )
            }
            "getBufferParameter" => {
                let (target, name) = (u(call, 1), u(call, 2));
                let bound = match self.bound_buffer(target) {
                    Ok(bound) => bound,
                    Err(code) => {
                        self.error(code);
                        return Ok(Answer::Nothing);
                    }
                };
                match (bound.and_then(|id| self.buffers.get(&id)), name) {
                    (None, _) => {
                        self.error(INVALID_OPERATION);
                        Answer::Nothing
                    }
                    (Some(buffer), BUFFER_SIZE) => Answer::Number(buffer.data.len() as f64),
                    (Some(buffer), BUFFER_USAGE) => Answer::Number(f64::from(buffer.usage)),
                    _ => {
                        self.error(INVALID_ENUM);
                        Answer::Nothing
                    }
                }
            }
            // ---- Vertex attributes ----
            "enableVertexAttribArray" | "disableVertexAttribArray" => {
                let index = u(call, 1) as usize;
                match self.attribs.get_mut(index) {
                    Some(attrib) => attrib.enabled = op == "enableVertexAttribArray",
                    None => self.error(INVALID_VALUE),
                }
                Answer::Nothing
            }
            "vertexAttribPointer" | "vertexAttribIPointer" => {
                let integer = op == "vertexAttribIPointer";
                let index = u(call, 1) as usize;
                let (size, kind) = (u(call, 2), u(call, 3));
                let (normalized, stride, offset) = if integer {
                    (false, i(call, 4), n(call, 5))
                } else {
                    (b(call, 4), i(call, 5), n(call, 6))
                };
                let bytes = type_size(kind);
                let float_kinds =
                    matches!(kind, BYTE | UNSIGNED_BYTE | SHORT | UNSIGNED_SHORT | FLOAT)
                        || (self.version == 2 && matches!(kind, INT | UNSIGNED_INT | HALF_FLOAT));
                let int_kinds = matches!(
                    kind,
                    BYTE | UNSIGNED_BYTE | SHORT | UNSIGNED_SHORT | INT | UNSIGNED_INT
                );
                if index >= self.attribs.len()
                    || !(1..=4).contains(&size)
                    || !(0..=255).contains(&stride)
                    || offset < 0.0
                {
                    self.error(INVALID_VALUE);
                } else if (integer && !int_kinds) || (!integer && !float_kinds) {
                    self.error(INVALID_ENUM);
                } else if !(stride as u32).is_multiple_of(bytes)
                    || !(offset as u64).is_multiple_of(u64::from(bytes))
                {
                    // WebGL's alignment of strides and offsets (WebGL 6.4).
                    self.error(INVALID_OPERATION);
                } else if self.array_buffer.is_none() && offset != 0.0 {
                    self.error(INVALID_OPERATION);
                } else {
                    let buffer = self.array_buffer;
                    let attrib = &mut self.attribs[index];
                    attrib.buffer = buffer;
                    attrib.size = size;
                    attrib.kind = kind;
                    attrib.normalized = normalized;
                    attrib.stride = stride as u32;
                    attrib.offset = offset as u64;
                    attrib.integer = integer;
                }
                Answer::Nothing
            }
            "vertexAttrib" => {
                // index, kind (FLOAT, INT, UNSIGNED_INT), and four values.
                let index = u(call, 1) as usize;
                let kind = u(call, 2);
                match self.attribs.get_mut(index) {
                    None => self.error(INVALID_VALUE),
                    Some(attrib) => {
                        attrib.value_kind = kind;
                        for k in 0..4 {
                            let v = n(call, 3 + k);
                            attrib.value[k] = match kind {
                                INT => (v as i32) as u32,
                                UNSIGNED_INT => v as u32,
                                _ => (v as f32).to_bits(),
                            };
                        }
                    }
                }
                Answer::Nothing
            }
            "getVertexAttrib" => self.vertex_attrib(u(call, 1) as usize, u(call, 2)),
            "getVertexAttribOffset" => {
                let index = u(call, 1) as usize;
                if u(call, 2) != VERTEX_ATTRIB_ARRAY_POINTER {
                    self.error(INVALID_ENUM);
                    Answer::Number(0.0)
                } else {
                    match self.attribs.get(index) {
                        Some(a) => Answer::Number(a.offset as f64),
                        None => {
                            self.error(INVALID_VALUE);
                            Answer::Number(0.0)
                        }
                    }
                }
            }
            // ---- Shaders and programs ----
            "createShader" => {
                let kind = u(call, 1);
                if !matches!(kind, VERTEX_SHADER | FRAGMENT_SHADER) {
                    self.error(INVALID_ENUM);
                    return Ok(Answer::Number(0.0));
                }
                let id = self.id();
                self.shaders.insert(
                    id,
                    Shader {
                        kind,
                        source: String::new(),
                        compiled: None,
                        compile_status: false,
                        log: String::new(),
                        deleted: false,
                        attached: 0,
                    },
                );
                Answer::Number(f64::from(id))
            }
            "shaderSource" => {
                let id = u(call, 1);
                let source = call.strings.first().cloned().unwrap_or_default();
                match self.shaders.get_mut(&id) {
                    Some(shader) if !shader.deleted => shader.source = source,
                    _ => self.error(INVALID_OPERATION),
                }
                Answer::Nothing
            }
            "getShaderSource" => match self.shaders.get(&u(call, 1)) {
                Some(shader) if !shader.deleted => Answer::MaybeString(Some(shader.source.clone())),
                _ => {
                    self.error(INVALID_OPERATION);
                    Answer::MaybeString(None)
                }
            },
            "compileShader" => {
                let id = u(call, 1);
                let options = self.glsl_options();
                match self.shaders.get_mut(&id) {
                    Some(shader) if !shader.deleted => {
                        let stage = if shader.kind == VERTEX_SHADER {
                            glsl::Stage::Vertex
                        } else {
                            glsl::Stage::Fragment
                        };
                        match glsl::compile(&shader.source, stage, &options) {
                            Ok(compiled) => {
                                shader.log = compiled.log.clone();
                                shader.compiled = Some(compiled);
                                shader.compile_status = true;
                            }
                            Err(log) => {
                                shader.log = log;
                                shader.compiled = None;
                                shader.compile_status = false;
                            }
                        }
                    }
                    _ => self.error(INVALID_OPERATION),
                }
                Answer::Nothing
            }
            "getShaderParameter" => {
                let (id, name) = (u(call, 1), u(call, 2));
                match self.shaders.get(&id) {
                    None => {
                        self.error(INVALID_VALUE);
                        Answer::Nothing
                    }
                    Some(shader) => match name {
                        SHADER_TYPE => Answer::Number(f64::from(shader.kind)),
                        DELETE_STATUS => Answer::Bool(shader.deleted),
                        COMPILE_STATUS => Answer::Bool(shader.compile_status),
                        _ => {
                            self.error(INVALID_ENUM);
                            Answer::Nothing
                        }
                    },
                }
            }
            "getShaderInfoLog" => match self.shaders.get(&u(call, 1)) {
                Some(shader) => Answer::MaybeString(Some(shader.log.clone())),
                None => {
                    self.error(INVALID_VALUE);
                    Answer::MaybeString(None)
                }
            },
            "deleteShader" => {
                let id = u(call, 1);
                if let Some(shader) = self.shaders.get_mut(&id) {
                    shader.deleted = true;
                    if shader.attached == 0 {
                        self.shaders.remove(&id);
                    }
                }
                Answer::Nothing
            }
            // A deleted shader lives while attached, a deleted program
            // while in use (GL ES 2.0 2.10.3).
            "isShader" => Answer::Bool(self.shaders.contains_key(&u(call, 1))),
            "createProgram" => {
                let id = self.id();
                self.programs.insert(
                    id,
                    Program {
                        vertex: None,
                        fragment: None,
                        linked: None,
                        link_status: false,
                        validate_status: false,
                        log: String::new(),
                        deleted: false,
                        bindings: Vec::new(),
                        links: 0,
                    },
                );
                Answer::Number(f64::from(id))
            }
            "attachShader" | "detachShader" => {
                let (program, shader) = (u(call, 1), u(call, 2));
                let kind = match self.shaders.get(&shader) {
                    Some(s) if !s.deleted || op == "detachShader" => s.kind,
                    _ => {
                        self.error(INVALID_OPERATION);
                        return Ok(Answer::Nothing);
                    }
                };
                let Some(p) = self.programs.get_mut(&program).filter(|p| !p.deleted) else {
                    self.error(INVALID_OPERATION);
                    return Ok(Answer::Nothing);
                };
                let slot = if kind == VERTEX_SHADER {
                    &mut p.vertex
                } else {
                    &mut p.fragment
                };
                if op == "attachShader" {
                    if slot.is_some() {
                        self.error(INVALID_OPERATION);
                    } else {
                        *slot = Some(shader);
                        if let Some(s) = self.shaders.get_mut(&shader) {
                            s.attached += 1;
                        }
                    }
                } else if *slot == Some(shader) {
                    *slot = None;
                    self.release_shader(shader);
                } else {
                    self.error(INVALID_OPERATION);
                }
                Answer::Nothing
            }
            "bindAttribLocation" => {
                let (program, index) = (u(call, 1), u(call, 2));
                let name = call.strings.first().cloned().unwrap_or_default();
                if index >= 16 {
                    self.error(INVALID_VALUE);
                } else if name.starts_with("gl_")
                    || name.starts_with("webgl_")
                    || name.starts_with("_webgl_")
                {
                    self.error(INVALID_OPERATION);
                } else {
                    match self.programs.get_mut(&program).filter(|p| !p.deleted) {
                        Some(p) => {
                            p.bindings.retain(|(n, _)| *n != name);
                            p.bindings.push((name, index));
                        }
                        None => self.error(INVALID_OPERATION),
                    }
                }
                Answer::Nothing
            }
            "linkProgram" => {
                self.link(u(call, 1));
                Answer::Nothing
            }
            "useProgram" => {
                let id = u(call, 1);
                let old = self.program;
                if id == 0 {
                    self.program = None;
                } else {
                    match self.programs.get(&id) {
                        Some(p) if p.deleted => self.error(INVALID_OPERATION),
                        Some(p) if !p.link_status => self.error(INVALID_OPERATION),
                        Some(_) => self.program = Some(id),
                        None => self.error(INVALID_OPERATION),
                    }
                }
                if let Some(old) = old.filter(|&old| self.program != Some(old)) {
                    self.free_program(old);
                }
                Answer::Nothing
            }
            "validateProgram" => {
                let id = u(call, 1);
                match self.programs.get_mut(&id) {
                    Some(p) => p.validate_status = p.link_status,
                    None => self.error(INVALID_VALUE),
                }
                Answer::Nothing
            }
            "deleteProgram" => {
                let id = u(call, 1);
                if let Some(p) = self.programs.get_mut(&id) {
                    p.deleted = true;
                    self.free_program(id);
                }
                Answer::Nothing
            }
            "isProgram" => Answer::Bool(self.programs.contains_key(&u(call, 1))),
            "getProgramParameter" => self.program_parameter(u(call, 1), u(call, 2)),
            "getProgramInfoLog" => match self.programs.get(&u(call, 1)) {
                Some(p) => Answer::MaybeString(Some(p.log.clone())),
                None => {
                    self.error(INVALID_VALUE);
                    Answer::MaybeString(None)
                }
            },
            "getAttachedShaders" => match self.programs.get(&u(call, 1)) {
                Some(p) => Answer::Numbers(
                    [p.vertex, p.fragment]
                        .into_iter()
                        .flatten()
                        .map(f64::from)
                        .collect(),
                ),
                None => {
                    self.error(INVALID_VALUE);
                    Answer::Numbers(Vec::new())
                }
            },
            "getAttribLocation" => {
                let name = call.strings.first().cloned().unwrap_or_default();
                match self.programs.get(&u(call, 1)) {
                    Some(Program {
                        linked: Some(l), ..
                    }) => Answer::Number(
                        l.program
                            .attributes
                            .iter()
                            .find(|a| a.name == name)
                            .map_or(-1.0, |a| f64::from(a.location)),
                    ),
                    Some(_) => {
                        self.error(INVALID_OPERATION);
                        Answer::Number(-1.0)
                    }
                    None => {
                        self.error(INVALID_OPERATION);
                        Answer::Number(-1.0)
                    }
                }
            }
            "getActiveAttrib" | "getActiveUniform" => {
                let (program, index) = (u(call, 1), u(call, 2) as usize);
                let Some(p) = self.programs.get(&program) else {
                    self.error(INVALID_VALUE);
                    return Ok(Answer::Nothing);
                };
                let Some(linked) = &p.linked else {
                    self.error(INVALID_VALUE);
                    return Ok(Answer::Nothing);
                };
                let info = if op == "getActiveAttrib" {
                    linked
                        .program
                        .attributes
                        .get(index)
                        .map(|a| (a.name.clone(), 1, a.gl_type))
                } else {
                    active_uniforms(&linked.program).get(index).cloned()
                };
                match info {
                    Some((name, size, ty)) => {
                        Answer::Pair(format!("[{},{}]", size, ty), name.into_bytes())
                    }
                    None => {
                        self.error(INVALID_VALUE);
                        Answer::Nothing
                    }
                }
            }
            "getUniformLocation" => {
                let program = u(call, 1);
                let name = call.strings.first().cloned().unwrap_or_default();
                self.uniform_location(program, &name)
            }
            "uniform" => {
                self.uniform(call);
                Answer::Nothing
            }
            "getUniform" => self.get_uniform(u(call, 1), u(call, 2)),
            "getShaderPrecisionFormat" => {
                let (shader, precision) = (u(call, 1), u(call, 2));
                if !matches!(shader, VERTEX_SHADER | FRAGMENT_SHADER)
                    || !(LOW_FLOAT..=HIGH_INT).contains(&precision)
                {
                    self.error(INVALID_ENUM);
                    Answer::Nothing
                } else if precision <= HIGH_FLOAT {
                    Answer::Numbers(vec![127.0, 127.0, 23.0])
                } else {
                    Answer::Numbers(vec![31.0, 30.0, 0.0])
                }
            }
            // ---- Drawing ----
            "clear" => {
                self.clear(u(call, 1), size);
                Answer::Nothing
            }
            "drawArrays" | "drawArraysInstanced" => {
                let instances = if op == "drawArrays" { 1 } else { i(call, 4) };
                self.draw_arrays(u(call, 1), i(call, 2), i(call, 3), instances, size);
                Answer::Nothing
            }
            "drawElements" | "drawElementsInstanced" => {
                let instances = if op == "drawElements" { 1 } else { i(call, 5) };
                self.draw_elements(
                    u(call, 1),
                    i(call, 2),
                    u(call, 3),
                    n(call, 4),
                    instances,
                    size,
                );
                Answer::Nothing
            }
            "present" => {
                if self.drawn {
                    self.present(env.inbox);
                }
                Answer::Nothing
            }
            other => match self.texture_op(other, call, size) {
                Some(answer) => answer,
                None => match self.framebuffer_op(other, call, size) {
                    Some(answer) => answer,
                    None => match self.webgl2_op(other, call, size) {
                        Some(answer) => answer,
                        None => {
                            return Err(DomError::new("TypeError", "an unknown WebGL call"));
                        }
                    },
                },
            },
        };
        Ok(answer)
    }

    fn glsl_options(&self) -> glsl::Options {
        let on = |name: &str| self.version == 1 && self.ext.on(name);
        glsl::Options {
            webgl2: self.version == 2,
            max_draw_buffers: if self.version == 2 || on("WEBGL_draw_buffers") {
                8
            } else {
                1
            },
            extensions: glsl::Extensions {
                standard_derivatives: on("OES_standard_derivatives"),
                frag_depth: on("EXT_frag_depth"),
                draw_buffers: on("WEBGL_draw_buffers"),
                shader_texture_lod: on("EXT_shader_texture_lod"),
            },
            ..glsl::Options::default()
        }
    }

    /// The context lost: its objects in the GPU process gone.
    pub(crate) fn lose(&mut self) {
        if self.lost {
            return;
        }
        self.lost = true;
        self.gpu.release(&self.client);
    }

    fn vertex_attrib(&mut self, index: usize, name: u32) -> Answer {
        let Some(a) = self.attribs.get(index).cloned() else {
            self.error(INVALID_VALUE);
            return Answer::Nothing;
        };
        match name {
            VERTEX_ATTRIB_ARRAY_BUFFER_BINDING => Answer::Number(a.buffer.map_or(0.0, f64::from)),
            VERTEX_ATTRIB_ARRAY_ENABLED => Answer::Bool(a.enabled),
            VERTEX_ATTRIB_ARRAY_SIZE => Answer::Number(f64::from(a.size)),
            VERTEX_ATTRIB_ARRAY_STRIDE => Answer::Number(f64::from(a.stride)),
            VERTEX_ATTRIB_ARRAY_TYPE => Answer::Number(f64::from(a.kind)),
            VERTEX_ATTRIB_ARRAY_NORMALIZED => Answer::Bool(a.normalized),
            VERTEX_ATTRIB_ARRAY_INTEGER if self.version == 2 => Answer::Bool(a.integer),
            VERTEX_ATTRIB_ARRAY_DIVISOR
                if self.version == 2 || self.ext.on("ANGLE_instanced_arrays") =>
            {
                Answer::Number(f64::from(a.divisor))
            }
            CURRENT_VERTEX_ATTRIB => Answer::Numbers(
                a.value
                    .iter()
                    .map(|&bits| match a.value_kind {
                        INT => f64::from(bits as i32),
                        UNSIGNED_INT => f64::from(bits),
                        _ => f64::from(f32::from_bits(bits)),
                    })
                    .collect(),
            ),
            _ => {
                self.error(INVALID_ENUM);
                Answer::Nothing
            }
        }
    }

    fn program_parameter(&mut self, id: u32, name: u32) -> Answer {
        let Some(p) = self.programs.get(&id) else {
            self.error(INVALID_VALUE);
            return Answer::Nothing;
        };
        match name {
            DELETE_STATUS => Answer::Bool(p.deleted),
            LINK_STATUS => Answer::Bool(p.link_status),
            VALIDATE_STATUS => Answer::Bool(p.validate_status),
            ATTACHED_SHADERS => Answer::Number(f64::from(
                u8::from(p.vertex.is_some()) + u8::from(p.fragment.is_some()),
            )),
            ACTIVE_ATTRIBUTES => {
                Answer::Number(p.linked.as_ref().map_or(0, |l| l.program.attributes.len()) as f64)
            }
            ACTIVE_UNIFORMS => Answer::Number(
                p.linked
                    .as_ref()
                    .map_or(0, |l| active_uniforms(&l.program).len()) as f64,
            ),
            ACTIVE_UNIFORM_BLOCKS if self.version == 2 => Answer::Number(
                p.linked
                    .as_ref()
                    .map_or(0, |l| webgl2::active_blocks(&l.program).len()) as f64,
            ),
            TRANSFORM_FEEDBACK_BUFFER_MODE if self.version == 2 => {
                Answer::Number(f64::from(INTERLEAVED_ATTRIBS))
            }
            TRANSFORM_FEEDBACK_VARYINGS if self.version == 2 => Answer::Number(0.0),
            _ => {
                self.error(INVALID_ENUM);
                Answer::Nothing
            }
        }
    }

    /// Links `id`: its shaders' translation, its modules made.
    fn link(&mut self, id: u32) {
        let Some(p) = self.programs.get(&id).filter(|p| !p.deleted).cloned() else {
            self.error(INVALID_OPERATION);
            return;
        };
        let shader = |s: Option<u32>| {
            s.and_then(|s| self.shaders.get(&s))
                .and_then(|s| s.compiled.clone())
        };
        let result = match (p.vertex, p.fragment) {
            (Some(_), Some(_)) => match (shader(p.vertex), shader(p.fragment)) {
                (Some(v), Some(f)) => glsl::link(
                    &v,
                    &f,
                    &glsl::LinkOptions {
                        attribute_locations: p.bindings.clone(),
                        ..glsl::LinkOptions::default()
                    },
                ),
                (None, _) => Err("Vertex shader is not compiled.\n".to_owned()),
                (_, None) => Err("Fragment shader is not compiled.\n".to_owned()),
            },
            _ => Err("Attached vertex and fragment shaders are needed.\n".to_owned()),
        };
        let links = p.links + 1;
        let linked = match result {
            Ok(program) => {
                let made = self.gpu.program(&self.client, &program);
                Some(Linked {
                    block_bindings: vec![0; webgl2::active_blocks(&program).len()],
                    uniforms: vec![0; program.uniform_size as usize],
                    units: vec![0; program.samplers.iter().map(|s| s.size as usize).sum()],
                    vertex: made.0,
                    fragment: made.1,
                    points: made.2,
                    layout: made.3,
                    pipeline_layout: made.4,
                    program,
                    generation: links,
                })
            }
            Err(log) => {
                let entry = self.programs.get_mut(&id).expect("there");
                entry.log = log;
                entry.link_status = false;
                entry.links = links;
                // The program in use keeps its last good link (GL 2.0).
                if self.program != Some(id) {
                    entry.linked = None;
                }
                return;
            }
        };
        let entry = self.programs.get_mut(&id).expect("there");
        entry.log = String::new();
        entry.link_status = true;
        entry.links = links;
        entry.linked = linked;
    }

    /// A uniform location of `program` named `name`: GL's names (`a`,
    /// `a[2]`, `s.f`, `s[1].f`).
    fn uniform_location(&mut self, program: u32, name: &str) -> Answer {
        let Some(p) = self.programs.get(&program) else {
            self.error(INVALID_VALUE);
            return Answer::Number(0.0);
        };
        let Some(linked) = p.linked.as_ref().filter(|_| p.link_status) else {
            self.error(INVALID_OPERATION);
            return Answer::Number(0.0);
        };
        if name.starts_with("gl_") || name.starts_with("webgl_") || name.starts_with("_webgl_") {
            return Answer::Number(0.0);
        }
        // `name[k]`, or `name` for its element 0.
        let (base, element) = match name.strip_suffix(']').and_then(|s| s.rsplit_once('[')) {
            Some((base, k)) => match k.parse::<u32>() {
                Ok(k) => (base.to_owned(), k),
                Err(_) => return Answer::Number(0.0),
            },
            None => (name.to_owned(), 0),
        };
        let generation = linked.generation;
        let found = linked
            .program
            .uniforms
            .iter()
            .enumerate()
            .filter(|(_, u)| u.block.is_none())
            .find_map(|(i, u)| {
                let plain = u.name.strip_suffix("[0]").unwrap_or(&u.name);
                let is_array = u.name.ends_with("[0]");
                (plain == base
                    && (element < u.size)
                    && (is_array || element == 0 && !name.ends_with(']'))
                    || (u.name == name))
                    .then_some(Target::Uniform(i, if u.name == name { 0 } else { element }))
            })
            .or_else(|| {
                linked
                    .program
                    .samplers
                    .iter()
                    .enumerate()
                    .find_map(|(i, s)| {
                        let plain = s.name.strip_suffix("[0]").unwrap_or(&s.name);
                        (plain == base && element < s.size).then_some(Target::Sampler(i, element))
                    })
            });
        match found {
            Some(target) => {
                let id = self.id();
                self.locations.insert(
                    id,
                    Location {
                        program,
                        generation,
                        target,
                    },
                );
                Answer::Number(f64::from(id))
            }
            None => Answer::Number(0.0),
        }
    }

    /// `uniform*`: [context, location, kind, components, columns,
    /// transpose] then the values. `kind` 0 float, 1 int, 2 uint;
    /// `columns` 0 for a vector, else a matrix's columns.
    fn uniform(&mut self, call: &CanvasCall) {
        let location = u(call, 1);
        if location == 0 {
            return;
        }
        let kind = u(call, 2);
        let components = u(call, 3);
        let columns = u(call, 4);
        let transpose = b(call, 5);
        let values: Vec<f64> = call
            .numbers
            .get(6..)
            .map(<[f64]>::to_vec)
            .unwrap_or_default();
        let Some(loc) = self.locations.get(&location).copied() else {
            self.error(INVALID_OPERATION);
            return;
        };
        let Some(current) = self.program else {
            self.error(INVALID_OPERATION);
            return;
        };
        let generation_ok = self
            .programs
            .get(&current)
            .and_then(|p| p.linked.as_ref())
            .is_some_and(|l| l.generation == loc.generation);
        if loc.program != current || !generation_ok {
            self.error(INVALID_OPERATION);
            return;
        }
        if transpose && self.version == 1 {
            self.error(INVALID_VALUE);
            return;
        }
        let per = if columns > 0 {
            columns * components
        } else {
            components
        } as usize;
        if per == 0 || values.is_empty() || !values.len().is_multiple_of(per) {
            self.error(INVALID_VALUE);
            return;
        }
        let linked = self
            .programs
            .get_mut(&current)
            .and_then(|p| p.linked.as_mut())
            .expect("linked");
        match loc.target {
            Target::Sampler(s, element) => {
                // Samplers take uniform1i and uniform1iv alone.
                if kind != 1 || components != 1 || columns != 0 {
                    self.error(INVALID_OPERATION);
                    return;
                }
                let first: usize = linked.program.samplers[..s]
                    .iter()
                    .map(|x| x.size as usize)
                    .sum();
                let size = linked.program.samplers[s].size;
                for (k, v) in values.iter().enumerate() {
                    if element + k as u32 >= size {
                        break;
                    }
                    let unit = *v as i64;
                    if !(0..32).contains(&unit) {
                        self.errors.push(INVALID_VALUE);
                        return;
                    }
                    linked.units[first + element as usize + k] = unit as u32;
                }
            }
            Target::Uniform(index, element) => {
                let uniform = linked.program.uniforms[index].clone();
                let Some((ty_kind, ty_components, ty_columns)) = uniform_shape(uniform.gl_type)
                else {
                    self.errors.push(INVALID_OPERATION);
                    return;
                };
                let bool_uniform = matches!(uniform.gl_type, 0x8B56..=0x8B59);
                // The call's kind matches the uniform's: a boolean takes
                // floats or ints; the sizes are the same.
                let kind_ok = ty_kind == kind || (bool_uniform && kind != 2);
                if !kind_ok || ty_components != components || ty_columns != columns {
                    self.errors.push(INVALID_OPERATION);
                    return;
                }
                let count = values.len() / per;
                if count > 1 && uniform.size == 1 && !uniform.name.ends_with("[0]") {
                    self.errors.push(INVALID_OPERATION);
                    return;
                }
                for k in 0..count {
                    let e = element + k as u32;
                    if e >= uniform.size {
                        break;
                    }
                    let base = uniform.offset as usize + e as usize * uniform.array_stride as usize;
                    let chunk = &values[k * per..(k + 1) * per];
                    write_uniform(
                        &mut linked.uniforms,
                        base,
                        chunk,
                        ty_kind,
                        bool_uniform,
                        components,
                        columns,
                        transpose,
                        uniform.matrix_stride,
                    );
                }
            }
        }
    }

    fn get_uniform(&mut self, program: u32, location: u32) -> Answer {
        let Some(loc) = self.locations.get(&location).copied() else {
            self.error(INVALID_OPERATION);
            return Answer::Nothing;
        };
        let Some(linked) = self.programs.get(&program).and_then(|p| p.linked.as_ref()) else {
            self.error(INVALID_OPERATION);
            return Answer::Nothing;
        };
        if loc.program != program || linked.generation != loc.generation {
            self.error(INVALID_OPERATION);
            return Answer::Nothing;
        }
        match loc.target {
            Target::Sampler(s, element) => {
                let first: usize = linked.program.samplers[..s]
                    .iter()
                    .map(|x| x.size as usize)
                    .sum();
                Answer::Pair(
                    format!("[0,1,{}]", linked.units[first + element as usize]),
                    Vec::new(),
                )
            }
            Target::Uniform(index, element) => {
                let uniform = &linked.program.uniforms[index];
                let Some((kind, components, columns)) = uniform_shape(uniform.gl_type) else {
                    return Answer::Nothing;
                };
                let base =
                    uniform.offset as usize + element as usize * uniform.array_stride as usize;
                let mut values = Vec::new();
                let cols = columns.max(1);
                for c in 0..cols {
                    for r in 0..components {
                        let at = base + (c * uniform.matrix_stride + r * 4) as usize;
                        let bits = u32::from_le_bytes(
                            linked.uniforms[at..at + 4].try_into().expect("four bytes"),
                        );
                        values.push(match kind {
                            0 => f64::from(f32::from_bits(bits)),
                            1 => f64::from(bits as i32),
                            _ => f64::from(bits),
                        });
                    }
                }
                let bool_uniform = matches!(uniform.gl_type, 0x8B56..=0x8B59);
                // [kind (0 float, 1 int, 2 uint, 3 bool), count, values...]
                let kind = if bool_uniform { 3 } else { kind };
                let list: Vec<String> = values.iter().map(|v| format!("{v}")).collect();
                Answer::Pair(
                    format!("[{kind},{},{}]", values.len(), list.join(",")),
                    Vec::new(),
                )
            }
        }
    }

    fn parameter(&mut self, name: u32) -> Answer {
        let num = |v: u32| Answer::Number(f64::from(v));
        let nums = |v: &[f64]| Answer::Numbers(v.to_vec());
        // Strings, and typed arrays by a code the bootstrap reads:
        // [kind, values...]: 1 Int32Array, 2 Float32Array, 3 bools.
        let typed = |kind: u32, v: Vec<f64>| {
            let mut all = vec![f64::from(kind)];
            all.extend(v);
            Answer::Numbers(all)
        };
        let s = &self.state;
        match name {
            VENDOR => Answer::String("WebKit".to_owned()),
            RENDERER => Answer::String("WebKit WebGL".to_owned()),
            VERSION => Answer::String(
                if self.version == 2 {
                    "WebGL 2.0 (OpenGL ES 3.0 Chromium)"
                } else {
                    "WebGL 1.0 (OpenGL ES 2.0 Chromium)"
                }
                .to_owned(),
            ),
            SHADING_LANGUAGE_VERSION => Answer::String(
                if self.version == 2 {
                    "WebGL GLSL ES 3.00 (OpenGL ES GLSL ES 3.0 Chromium)"
                } else {
                    "WebGL GLSL ES 1.0 (OpenGL ES GLSL ES 1.0 Chromium)"
                }
                .to_owned(),
            ),
            VIEWPORT => typed(1, s.viewport.iter().map(|&v| f64::from(v)).collect()),
            SCISSOR_BOX => typed(1, s.scissor.iter().map(|&v| f64::from(v)).collect()),
            COLOR_CLEAR_VALUE => typed(2, s.clear_color.iter().map(|&v| f64::from(v)).collect()),
            BLEND_COLOR => typed(2, s.blend_color.iter().map(|&v| f64::from(v)).collect()),
            DEPTH_RANGE => typed(2, s.depth_range.iter().map(|&v| f64::from(v)).collect()),
            COLOR_WRITEMASK => typed(
                3,
                s.color_mask
                    .iter()
                    .map(|&v| f64::from(u8::from(v)))
                    .collect(),
            ),
            ALIASED_POINT_SIZE_RANGE => typed(2, vec![1.0, 1024.0]),
            ALIASED_LINE_WIDTH_RANGE => typed(2, vec![1.0, 1.0]),
            MAX_VIEWPORT_DIMS => typed(1, vec![f64::from(MAX_SIZE), f64::from(MAX_SIZE)]),
            BLEND
            | CULL_FACE
            | DEPTH_TEST
            | DITHER
            | POLYGON_OFFSET_FILL
            | SAMPLE_ALPHA_TO_COVERAGE
            | SAMPLE_COVERAGE
            | SCISSOR_TEST
            | STENCIL_TEST => Answer::Bool(match name {
                BLEND => s.blend,
                CULL_FACE => s.cull,
                DEPTH_TEST => s.depth_test,
                DITHER => s.dither,
                POLYGON_OFFSET_FILL => s.polygon_offset,
                SAMPLE_ALPHA_TO_COVERAGE => s.sample_alpha_to_coverage,
                SAMPLE_COVERAGE => s.sample_coverage,
                SCISSOR_TEST => s.scissor_test,
                _ => s.stencil_test,
            }),
            DEPTH_WRITEMASK => Answer::Bool(s.depth_mask),
            SAMPLE_COVERAGE_INVERT => Answer::Bool(s.sample_coverage_invert),
            UNPACK_FLIP_Y_WEBGL => Answer::Bool(s.unpack_flip_y),
            UNPACK_PREMULTIPLY_ALPHA_WEBGL => Answer::Bool(s.unpack_premultiply_alpha),
            DEPTH_CLEAR_VALUE => Answer::Number(f64::from(s.clear_depth)),
            LINE_WIDTH => Answer::Number(f64::from(s.line_width)),
            POLYGON_OFFSET_FACTOR => Answer::Number(f64::from(s.polygon_offset_factor)),
            POLYGON_OFFSET_UNITS => Answer::Number(f64::from(s.polygon_offset_units)),
            SAMPLE_COVERAGE_VALUE => Answer::Number(f64::from(s.sample_coverage_value)),
            STENCIL_CLEAR_VALUE => Answer::Number(f64::from(s.clear_stencil)),
            DEPTH_FUNC => num(s.depth_func),
            CULL_FACE_MODE => num(s.cull_mode),
            FRONT_FACE => num(s.front_face),
            BLEND_SRC_RGB => num(s.blend_src[0]),
            BLEND_SRC_ALPHA => num(s.blend_src[1]),
            BLEND_DST_RGB => num(s.blend_dst[0]),
            BLEND_DST_ALPHA => num(s.blend_dst[1]),
            BLEND_EQUATION_RGB => num(s.blend_equation[0]),
            BLEND_EQUATION_ALPHA => num(s.blend_equation[1]),
            STENCIL_FUNC => num(s.stencil[0].func),
            STENCIL_FAIL => num(s.stencil[0].fail),
            STENCIL_PASS_DEPTH_FAIL => num(s.stencil[0].depth_fail),
            STENCIL_PASS_DEPTH_PASS => num(s.stencil[0].pass),
            STENCIL_REF => Answer::Number(f64::from(s.stencil[0].reference)),
            STENCIL_VALUE_MASK => num(s.stencil[0].value_mask),
            STENCIL_WRITEMASK => num(s.stencil[0].write_mask),
            STENCIL_BACK_FUNC => num(s.stencil[1].func),
            STENCIL_BACK_FAIL => num(s.stencil[1].fail),
            STENCIL_BACK_PASS_DEPTH_FAIL => num(s.stencil[1].depth_fail),
            STENCIL_BACK_PASS_DEPTH_PASS => num(s.stencil[1].pass),
            STENCIL_BACK_REF => Answer::Number(f64::from(s.stencil[1].reference)),
            STENCIL_BACK_VALUE_MASK => num(s.stencil[1].value_mask),
            STENCIL_BACK_WRITEMASK => num(s.stencil[1].write_mask),
            PACK_ALIGNMENT => num(s.pack_alignment),
            UNPACK_ALIGNMENT => num(s.unpack_alignment),
            UNPACK_COLORSPACE_CONVERSION_WEBGL => num(s.unpack_colorspace_conversion),
            GENERATE_MIPMAP_HINT => num(s.generate_mipmap_hint),
            ACTIVE_TEXTURE => num(s.active_texture),
            MAX_TEXTURE_SIZE | MAX_CUBE_MAP_TEXTURE_SIZE | MAX_RENDERBUFFER_SIZE => num(MAX_SIZE),
            MAX_VERTEX_ATTRIBS => num(16),
            MAX_VERTEX_UNIFORM_VECTORS => num(4096),
            MAX_FRAGMENT_UNIFORM_VECTORS => num(1024),
            MAX_VARYING_VECTORS => num(15),
            MAX_COMBINED_TEXTURE_IMAGE_UNITS => num(32),
            MAX_VERTEX_TEXTURE_IMAGE_UNITS | MAX_TEXTURE_IMAGE_UNITS => num(16),
            SUBPIXEL_BITS => num(4),
            RED_BITS | GREEN_BITS | BLUE_BITS => num(8),
            ALPHA_BITS => num(if self.attributes.alpha { 8 } else { 0 }),
            DEPTH_BITS => num(if self.attributes.depth { 24 } else { 0 }),
            STENCIL_BITS => num(if self.attributes.stencil { 8 } else { 0 }),
            SAMPLE_BUFFERS => num(u32::from(self.attributes.antialias)),
            SAMPLES => num(if self.attributes.antialias { 4 } else { 0 }),
            IMPLEMENTATION_COLOR_READ_FORMAT => num(self.read_format().0),
            IMPLEMENTATION_COLOR_READ_TYPE => num(self.read_format().1),
            ARRAY_BUFFER_BINDING => Answer::Number(self.array_buffer.map_or(0.0, f64::from)),
            ELEMENT_ARRAY_BUFFER_BINDING => {
                Answer::Number(self.element_buffer.map_or(0.0, f64::from))
            }
            CURRENT_PROGRAM => Answer::Number(self.program.map_or(0.0, f64::from)),
            COMPRESSED_TEXTURE_FORMATS => typed(
                1,
                if self.ext.on("WEBGL_compressed_texture_s3tc") {
                    (COMPRESSED_RGB_S3TC_DXT1_EXT..=COMPRESSED_RGBA_S3TC_DXT5_EXT)
                        .map(f64::from)
                        .collect()
                } else {
                    Vec::new()
                },
            ),
            FRAMEBUFFER_BINDING => Answer::Number(self.surfaces.framebuffer.map_or(0.0, f64::from)),
            RENDERBUFFER_BINDING => {
                Answer::Number(self.surfaces.renderbuffer.map_or(0.0, f64::from))
            }
            TEXTURE_BINDING_2D | TEXTURE_BINDING_CUBE_MAP => {
                let slot = texture::binding_slot(name).expect("a binding");
                Answer::Number(self.bound_texture(slot).map_or(0.0, f64::from))
            }
            TEXTURE_BINDING_3D | TEXTURE_BINDING_2D_ARRAY if self.version == 2 => {
                let slot = texture::binding_slot(name).expect("a binding");
                Answer::Number(self.bound_texture(slot).map_or(0.0, f64::from))
            }
            SAMPLER_BINDING if self.version == 2 => {
                let unit = self.unit();
                Answer::Number(self.surfaces.sampler_units[unit].map_or(0.0, f64::from))
            }
            READ_FRAMEBUFFER_BINDING if self.version == 2 => {
                Answer::Number(self.surfaces.read_framebuffer.map_or(0.0, f64::from))
            }
            UNPACK_ROW_LENGTH if self.version == 2 => num(self.state.unpack_row_length),
            UNPACK_IMAGE_HEIGHT if self.version == 2 => num(self.state.unpack_image_height),
            UNPACK_SKIP_PIXELS if self.version == 2 => num(self.state.unpack_skip_pixels),
            UNPACK_SKIP_ROWS if self.version == 2 => num(self.state.unpack_skip_rows),
            UNPACK_SKIP_IMAGES if self.version == 2 => num(self.state.unpack_skip_images),
            PACK_ROW_LENGTH if self.version == 2 => num(self.state.pack_row_length),
            PACK_SKIP_PIXELS if self.version == 2 => num(self.state.pack_skip_pixels),
            PACK_SKIP_ROWS if self.version == 2 => num(self.state.pack_skip_rows),
            MAX_COLOR_ATTACHMENTS | MAX_DRAW_BUFFERS if self.ext.on("WEBGL_draw_buffers") => num(8),
            MAX_TEXTURE_MAX_ANISOTROPY_EXT if self.ext.on("EXT_texture_filter_anisotropic") => {
                Answer::Number(16.0)
            }
            FRAGMENT_SHADER_DERIVATIVE_HINT_OES
                if self.version == 2 || self.ext.on("OES_standard_derivatives") =>
            {
                num(self.surfaces.derivative_hint)
            }
            MAX_ELEMENT_INDEX if self.version == 2 => Answer::Number(4294967294.0),
            MAX_3D_TEXTURE_SIZE | MAX_ARRAY_TEXTURE_LAYERS if self.version == 2 => num(2048),
            MAX_DRAW_BUFFERS | MAX_COLOR_ATTACHMENTS if self.version == 2 => num(8),
            MAX_SAMPLES if self.version == 2 => num(4),
            MAX_UNIFORM_BUFFER_BINDINGS if self.version == 2 => num(24),
            MAX_UNIFORM_BLOCK_SIZE if self.version == 2 => num(65536),
            UNIFORM_BUFFER_OFFSET_ALIGNMENT if self.version == 2 => num(256),
            MAX_VERTEX_UNIFORM_BLOCKS | MAX_FRAGMENT_UNIFORM_BLOCKS if self.version == 2 => num(12),
            MAX_COMBINED_UNIFORM_BLOCKS if self.version == 2 => num(24),
            MAX_VERTEX_OUTPUT_COMPONENTS
            | MAX_FRAGMENT_INPUT_COMPONENTS
            | MAX_VARYING_COMPONENTS
                if self.version == 2 =>
            {
                num(60)
            }
            MAX_VERTEX_UNIFORM_COMPONENTS if self.version == 2 => num(16384),
            MAX_FRAGMENT_UNIFORM_COMPONENTS if self.version == 2 => num(4096),
            MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS | MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS
                if self.version == 2 =>
            {
                num(212992)
            }
            MAX_PROGRAM_TEXEL_OFFSET if self.version == 2 => num(7),
            MIN_PROGRAM_TEXEL_OFFSET if self.version == 2 => Answer::Number(-8.0),
            MAX_TEXTURE_LOD_BIAS if self.version == 2 => num(15),
            MAX_ELEMENTS_VERTICES | MAX_ELEMENTS_INDICES if self.version == 2 => num(1 << 20),
            MAX_SERVER_WAIT_TIMEOUT if self.version == 2 => num(0),
            MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS
            | MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS
                if self.version == 2 =>
            {
                num(64)
            }
            MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS if self.version == 2 => num(4),
            VERTEX_ARRAY_BINDING if self.version == 2 || self.ext.on("OES_vertex_array_object") => {
                Answer::Number(self.v2.vao.map_or(0.0, f64::from))
            }
            COPY_READ_BUFFER_BINDING
            | COPY_WRITE_BUFFER_BINDING
            | PIXEL_PACK_BUFFER_BINDING
            | PIXEL_UNPACK_BUFFER_BINDING
            | UNIFORM_BUFFER_BINDING
            | TRANSFORM_FEEDBACK_BUFFER_BINDING
                if self.version == 2 =>
            {
                let v2 = &self.v2;
                let bound = match name {
                    COPY_READ_BUFFER_BINDING => v2.copy_read,
                    COPY_WRITE_BUFFER_BINDING => v2.copy_write,
                    PIXEL_PACK_BUFFER_BINDING => v2.pixel_pack,
                    PIXEL_UNPACK_BUFFER_BINDING => v2.pixel_unpack,
                    UNIFORM_BUFFER_BINDING => v2.uniform,
                    _ => v2.feedback_buffer,
                };
                Answer::Number(bound.map_or(0.0, f64::from))
            }
            TRANSFORM_FEEDBACK_BINDING if self.version == 2 => {
                Answer::Number(self.v2.feedback.map_or(0.0, f64::from))
            }
            TRANSFORM_FEEDBACK_ACTIVE | TRANSFORM_FEEDBACK_PAUSED if self.version == 2 => {
                Answer::Bool(false)
            }
            READ_BUFFER if self.version == 2 => num(self.read_buffer()),
            n if (self.version == 2 || self.ext.on("WEBGL_draw_buffers"))
                && (DRAW_BUFFER0..DRAW_BUFFER0 + 8).contains(&n) =>
            {
                num(self.draw_buffer(n - DRAW_BUFFER0))
            }
            _ => {
                self.error(INVALID_ENUM);
                let _ = nums;
                Answer::Nothing
            }
        }
    }
}

/// The drawing buffer's size: the canvas's, at least 1 by 1, at most
/// MAX_SIZE a side.
pub(crate) fn clamp_size(size: (u32, u32)) -> (u32, u32) {
    (size.0.clamp(1, MAX_SIZE), size.1.clamp(1, MAX_SIZE))
}

fn face_indices(faces: u32) -> Vec<usize> {
    match faces {
        FRONT => vec![0],
        BACK => vec![1],
        _ => vec![0, 1],
    }
}

pub(crate) fn type_size(kind: u32) -> u32 {
    match kind {
        BYTE | UNSIGNED_BYTE => 1,
        SHORT | UNSIGNED_SHORT | HALF_FLOAT => 2,
        _ => 4,
    }
}

/// The active uniforms, as `getActiveUniform` lists them: the default
/// block's and the samplers, then the blocks'.
fn active_uniforms(program: &glsl::Program) -> Vec<(String, u32, u32)> {
    let mut out: Vec<(String, u32, u32)> = program
        .uniforms
        .iter()
        .filter(|u| u.block.is_none())
        .map(|u| (u.name.clone(), u.size, u.gl_type))
        .collect();
    out.extend(
        program
            .samplers
            .iter()
            .map(|s| (s.name.clone(), s.size, s.gl_type)),
    );
    out.extend(
        program
            .uniforms
            .iter()
            .filter(|u| u.block.is_some())
            .map(|u| (u.name.clone(), u.size, u.gl_type)),
    );
    out
}

/// A uniform's type as the calls see it: kind (0 float or bool, 1 int,
/// 2 uint), components (a matrix's rows), columns (0 for no matrix).
fn uniform_shape(gl_type: u32) -> Option<(u32, u32, u32)> {
    Some(match gl_type {
        0x1406 => (0, 1, 0),
        0x8B50 => (0, 2, 0),
        0x8B51 => (0, 3, 0),
        0x8B52 => (0, 4, 0),
        0x1404 => (1, 1, 0),
        0x8B53 => (1, 2, 0),
        0x8B54 => (1, 3, 0),
        0x8B55 => (1, 4, 0),
        0x1405 => (2, 1, 0),
        0x8DC6 => (2, 2, 0),
        0x8DC7 => (2, 3, 0),
        0x8DC8 => (2, 4, 0),
        // Booleans take floats or ints: kind 0 here, either allowed.
        0x8B56 => (0, 1, 0),
        0x8B57 => (0, 2, 0),
        0x8B58 => (0, 3, 0),
        0x8B59 => (0, 4, 0),
        0x8B5A => (0, 2, 2),
        0x8B5B => (0, 3, 3),
        0x8B5C => (0, 4, 4),
        0x8B65 => (0, 3, 2),
        0x8B66 => (0, 4, 2),
        0x8B67 => (0, 2, 3),
        0x8B68 => (0, 4, 3),
        0x8B69 => (0, 2, 4),
        0x8B6A => (0, 3, 4),
        _ => return None,
    })
}

/// One element's values written at `base`, in the layout the
/// translation gives.
#[allow(clippy::too_many_arguments)]
fn write_uniform(
    bytes: &mut [u8],
    base: usize,
    values: &[f64],
    kind: u32,
    boolean: bool,
    components: u32,
    columns: u32,
    transpose: bool,
    matrix_stride: u32,
) {
    let put = |bytes: &mut [u8], at: usize, bits: u32| {
        if let Some(slot) = bytes.get_mut(at..at + 4) {
            slot.copy_from_slice(&bits.to_le_bytes());
        }
    };
    let bits = |v: f64| -> u32 {
        if boolean {
            u32::from(v != 0.0)
        } else {
            match kind {
                0 => (v as f32).to_bits(),
                1 => (v as i32) as u32,
                _ => v as u32,
            }
        }
    };
    if columns == 0 {
        for (k, &v) in values.iter().enumerate() {
            put(bytes, base + k * 4, bits(v));
        }
        return;
    }
    let (cols, rows) = (columns as usize, components as usize);
    for c in 0..cols {
        for r in 0..rows {
            // Transposed, the values are row after row.
            let v = if transpose {
                values[r * cols + c]
            } else {
                values[c * rows + r]
            };
            put(bytes, base + c * matrix_stride as usize + r * 4, bits(v));
        }
    }
}
