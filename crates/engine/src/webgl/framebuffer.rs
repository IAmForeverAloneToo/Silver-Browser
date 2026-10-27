//! WebGL's framebuffers and renderbuffers (M41 steps 7 and 8, D40): what
//! draws go into (the drawing buffer or a framebuffer's attachments, a
//! colour each draw buffer), their completeness, clears (a pass's load,
//! or a quad when scissored or masked), clearBuffer, readPixels (copied
//! to a buffer and waited for), blitFramebuffer (a draw sampling the
//! read framebuffer) and the rest of their calls.

use std::collections::{BTreeMap, HashMap};

use js::{Answer, CanvasCall};

use super::formats::{self, Layout, Sample, has_depth, has_stencil, texel_bytes};
use super::gl::*;
use super::texture::{SamplerObject, Texture};
use super::{Context, MAX_SIZE, i, n, u, wait};
use crate::gpu::wire::{Command, RenderCommand, Reply, Request};

/// The most colour attachments and draw buffers.
pub(crate) const MAX_COLORS: u32 = 8;

/// A renderbuffer: its format, size, samples (1 or 4), and its textures
/// and views in the GPU process (a multisampled one's resolved texture
/// beside it).
#[derive(Clone, Debug)]
pub(crate) struct Renderbuffer {
    pub deleted: bool,
    pub bound: bool,
    pub internal: u32,
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    pub storage: Option<&'static str>,
    pub gpu: Option<(u32, u32)>,
    pub resolve: Option<(u32, u32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Attached {
    Texture {
        id: u32,
        face: u8,
        level: u8,
        layer: u32,
    },
    Renderbuffer(u32),
}

#[derive(Clone, Debug)]
pub(crate) struct Framebuffer {
    pub deleted: bool,
    pub bound: bool,
    pub attachments: BTreeMap<u32, Attached>,
    pub draw_buffers: Vec<u32>,
    pub read_buffer: u32,
}

impl Default for Framebuffer {
    fn default() -> Framebuffer {
        Framebuffer {
            deleted: false,
            bound: false,
            attachments: BTreeMap::new(),
            draw_buffers: vec![COLOR_ATTACHMENT0],
            read_buffer: COLOR_ATTACHMENT0,
        }
    }
}

/// A context's textures, samplers, renderbuffers and framebuffers, and
/// their bindings.
#[derive(Debug)]
pub(crate) struct Surfaces {
    pub textures: HashMap<u32, Texture>,
    pub renderbuffers: HashMap<u32, Renderbuffer>,
    pub framebuffers: HashMap<u32, Framebuffer>,
    pub samplers: HashMap<u32, SamplerObject>,
    /// Each texture unit's 2D, cube, 3D and array texture.
    pub units: Vec<[Option<u32>; 4]>,
    pub sampler_units: Vec<Option<u32>>,
    pub renderbuffer: Option<u32>,
    /// The draw framebuffer (FRAMEBUFFER's) and the read one.
    pub framebuffer: Option<u32>,
    pub read_framebuffer: Option<u32>,
    pub derivative_hint: u32,
    /// The drawing buffer's draw and read buffers (BACK or NONE).
    pub default_draw: u32,
    pub default_read: u32,
}

impl Default for Surfaces {
    fn default() -> Surfaces {
        Surfaces {
            textures: HashMap::new(),
            renderbuffers: HashMap::new(),
            framebuffers: HashMap::new(),
            samplers: HashMap::new(),
            units: vec![[None; 4]; 32],
            sampler_units: vec![None; 32],
            renderbuffer: None,
            framebuffer: None,
            read_framebuffer: None,
            derivative_hint: DONT_CARE,
            default_draw: BACK,
            default_read: BACK,
        }
    }
}

/// A colour draws go into: its view, the view it resolves to, a 3D
/// texture's slice, the storage, and whether it has no alpha (its writes
/// masked, read as 1).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ColorTarget {
    pub view: u32,
    pub resolve: Option<u32>,
    pub slice: Option<u32>,
    pub storage: &'static str,
    pub rgb: bool,
}

/// What draws and clears go into: the drawing buffer, or the draw
/// framebuffer's attachments, a colour each draw buffer.
#[derive(Clone, Debug)]
pub(crate) struct Target {
    /// Its views (and slices), which say whether a pass is still of it.
    pub key: Vec<u32>,
    pub size: (u32, u32),
    pub colors: Vec<Option<ColorTarget>>,
    pub depth: Option<(u32, &'static str)>,
    pub depth_on: bool,
    pub stencil_on: bool,
    pub samples: u32,
    pub default: bool,
    /// The textures drawn into.
    pub textures: Vec<u32>,
}

/// What readPixels, the copies and blits read: a texture, its view,
/// size, storage, a 3D or array layer, whether alpha reads as 1, the
/// texture image it is (when one), and its samples.
pub(crate) struct Source {
    pub texture: u32,
    pub view: u32,
    pub size: (u32, u32),
    pub storage: &'static str,
    pub layer: u32,
    pub opaque: bool,
    pub image: Option<(u32, u8, u8)>,
    pub samples: u32,
}

/// An attachment as completeness sees it.
struct Info {
    storage: &'static str,
    internal: u32,
    size: (u32, u32),
    samples: u32,
    color_ok: bool,
    opaque: bool,
    made: bool,
}

/// The pipelines and modules of the clears drawn as quads and of
/// blitFramebuffer.
#[derive(Debug, Default)]
pub(crate) struct Kit {
    layout: Option<(u32, u32, u32)>,
    modules: HashMap<(u8, u8), u32>,
    pipelines: HashMap<String, u32>,
    blit_layouts: HashMap<u8, (u32, u32)>,
    blit_modules: HashMap<u8, u32>,
}

/// A clear quad's module: its outputs, all of one kind (0 float, 1
/// int, 2 uint), at the locations of `mask`'s bits; or the depth alone.
fn clear_code(kind: u8, mask: u8) -> String {
    let ty = ["vec4<f32>", "vec4<i32>", "vec4<u32>"][usize::from(kind)];
    let value = [
        "bitcast<vec4<f32>>(c.color)",
        "bitcast<vec4<i32>>(c.color)",
        "c.color",
    ][usize::from(kind)];
    let mut fields = String::new();
    let mut sets = String::new();
    for l in 0..8 {
        if mask & (1 << l) != 0 {
            fields.push_str(&format!("@location({l}) o{l}: {ty}, "));
            sets.push_str(&format!("o.o{l} = {value}; "));
        }
    }
    let fragment = if mask == 0 {
        "@fragment fn fs() {}\n".to_owned()
    } else {
        format!("struct O {{ {fields} }}\n@fragment fn fs() -> O {{ var o: O; {sets}return o; }}\n")
    };
    format!(
        "struct C {{ color: vec4<u32>, depth: vec4<f32> }}\n@group(0) @binding(0) var<uniform> c: C;\n@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{\n    var p = array(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));\n    return vec4<f32>(p[i], c.depth.x, 1.0);\n}}\n{fragment}"
    )
}

/// blitFramebuffer's module: kind 0 a float colour sampled (nearest by
/// load, linear by the sampler), 1 int, 2 uint, 3 depth (single
/// sampled).
fn blit_code(kind: u8) -> String {
    let (tex, out, body) = match kind {
        1 => (
            "texture_2d<i32>",
            "@location(0) vec4<i32>",
            "return textureLoad(t, vec2<i32>(floor(s)), 0);",
        ),
        2 => (
            "texture_2d<u32>",
            "@location(0) vec4<u32>",
            "return textureLoad(t, vec2<i32>(floor(s)), 0);",
        ),
        3 => (
            "texture_depth_2d",
            "@builtin(frag_depth) f32",
            "return textureLoad(t, vec2<i32>(floor(s)), 0);",
        ),
        _ => (
            "texture_2d<f32>",
            "@location(0) vec4<f32>",
            "if (b.linear != 0u) { return textureSampleLevel(t, smp, s / b.size, 0.0); }\n    return textureLoad(t, vec2<i32>(floor(s)), 0);",
        ),
    };
    format!(
        "struct B {{ src0: vec2<f32>, src1: vec2<f32>, dst0: vec2<f32>, dst1: vec2<f32>, size: vec2<f32>, linear: u32, pad: u32 }}
@group(0) @binding(0) var<uniform> b: B;
@group(0) @binding(1) var t: {tex};
@group(0) @binding(2) var smp: sampler;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{
    var p = array(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(p[i], 0.0, 1.0);
}}
@fragment fn fs(@builtin(position) p: vec4<f32>) -> {out} {{
    let s = b.src0 + (p.xy - b.dst0) * (b.src1 - b.src0) / (b.dst1 - b.dst0);
    if (any(s < vec2<f32>(0.0, 0.0)) || any(s >= b.size)) {{ discard; }}
    {body}
}}
"
    )
}

/// What a storage's colour is, by clearBuffer's and the outputs' kinds:
/// 0 float, 1 int, 2 uint.
pub(crate) fn kind_of(storage: &str) -> u8 {
    match formats::info(storage).2 {
        Sample::Sint => 1,
        Sample::Uint => 2,
        _ => 0,
    }
}

impl Context {
    /// The calls of renderbuffers and framebuffers, readPixels and the
    /// clears of buffers; none for another.
    pub(super) fn framebuffer_op(
        &mut self,
        op: &str,
        call: &CanvasCall,
        size: (u32, u32),
    ) -> Option<Answer> {
        let answer = match op {
            "createRenderbuffer" => {
                let id = self.id();
                self.surfaces.renderbuffers.insert(
                    id,
                    Renderbuffer {
                        deleted: false,
                        bound: false,
                        internal: RGBA4,
                        width: 0,
                        height: 0,
                        samples: 0,
                        storage: None,
                        gpu: None,
                        resolve: None,
                    },
                );
                Answer::Number(f64::from(id))
            }
            "bindRenderbuffer" => {
                let (target, id) = (u(call, 1), u(call, 2));
                if target != RENDERBUFFER {
                    self.error(INVALID_ENUM);
                } else if id == 0 {
                    self.surfaces.renderbuffer = None;
                } else {
                    match self.surfaces.renderbuffers.get_mut(&id) {
                        Some(r) if !r.deleted => {
                            r.bound = true;
                            self.surfaces.renderbuffer = Some(id);
                        }
                        _ => self.error(INVALID_OPERATION),
                    }
                }
                Answer::Nothing
            }
            "deleteRenderbuffer" => {
                let id = u(call, 1);
                if let Some(r) = self.surfaces.renderbuffers.get_mut(&id) {
                    r.deleted = true;
                    if self.surfaces.renderbuffer == Some(id) {
                        self.surfaces.renderbuffer = None;
                    }
                    self.detach(Attached::Renderbuffer(id));
                    self.sweep();
                }
                Answer::Nothing
            }
            "isRenderbuffer" => Answer::Bool(
                self.surfaces
                    .renderbuffers
                    .get(&u(call, 1))
                    .is_some_and(|r| !r.deleted && r.bound),
            ),
            "renderbufferStorage" => {
                self.renderbuffer_storage(u(call, 1), 0, u(call, 2), i(call, 3), i(call, 4));
                Answer::Nothing
            }
            "renderbufferStorageMultisample" => {
                self.renderbuffer_storage(
                    u(call, 1),
                    i(call, 2),
                    u(call, 3),
                    i(call, 4),
                    i(call, 5),
                );
                Answer::Nothing
            }
            "getRenderbufferParameter" => self.renderbuffer_parameter(u(call, 1), u(call, 2)),
            "getInternalformatParameter" => {
                let (target, internal, name) = (u(call, 1), u(call, 2), u(call, 3));
                let ext = |n: &str| self.ext.on(n);
                let format = formats::renderbuffer_format(
                    self.version,
                    &ext,
                    self.features.rg11b10_renderable,
                    self.features.depth32_stencil,
                    internal,
                );
                match format {
                    _ if target != RENDERBUFFER || name != SAMPLES => {
                        self.error(INVALID_ENUM);
                        Answer::Nothing
                    }
                    None => {
                        self.error(INVALID_ENUM);
                        Answer::Nothing
                    }
                    Some(f) if kind_of(f.storage) != 0 => Answer::Numbers(vec![1.0]),
                    Some(_) => Answer::Numbers(vec![1.0, 4.0]),
                }
            }
            "createFramebuffer" => {
                let id = self.id();
                self.surfaces
                    .framebuffers
                    .insert(id, Framebuffer::default());
                Answer::Number(f64::from(id))
            }
            "bindFramebuffer" => {
                let (target, id) = (u(call, 1), u(call, 2));
                let v2 = self.version == 2;
                let (draw, read) = match target {
                    FRAMEBUFFER => (true, true),
                    DRAW_FRAMEBUFFER if v2 => (true, false),
                    READ_FRAMEBUFFER if v2 => (false, true),
                    _ => {
                        self.error(INVALID_ENUM);
                        return Some(Answer::Nothing);
                    }
                };
                if id != 0 {
                    match self.surfaces.framebuffers.get_mut(&id) {
                        Some(fb) if !fb.deleted => fb.bound = true,
                        _ => {
                            self.error(INVALID_OPERATION);
                            return Some(Answer::Nothing);
                        }
                    }
                }
                let value = (id != 0).then_some(id);
                if draw {
                    self.surfaces.framebuffer = value;
                }
                if read {
                    self.surfaces.read_framebuffer = value;
                }
                Answer::Nothing
            }
            "deleteFramebuffer" => {
                let id = u(call, 1);
                if self.surfaces.framebuffers.contains_key(&id) {
                    if self.surfaces.framebuffer == Some(id) {
                        self.surfaces.framebuffer = None;
                    }
                    if self.surfaces.read_framebuffer == Some(id) {
                        self.surfaces.read_framebuffer = None;
                    }
                    self.surfaces.framebuffers.remove(&id);
                    self.sweep();
                }
                Answer::Nothing
            }
            "isFramebuffer" => Answer::Bool(
                self.surfaces
                    .framebuffers
                    .get(&u(call, 1))
                    .is_some_and(|fb| !fb.deleted && fb.bound),
            ),
            "framebufferTexture2D" => {
                let level = i(call, 5);
                self.framebuffer_texture(
                    u(call, 1),
                    u(call, 2),
                    Some(u(call, 3)),
                    u(call, 4),
                    level,
                    0,
                );
                Answer::Nothing
            }
            "framebufferTextureLayer" => {
                self.framebuffer_texture(
                    u(call, 1),
                    u(call, 2),
                    None,
                    u(call, 3),
                    i(call, 4),
                    i(call, 5),
                );
                Answer::Nothing
            }
            "framebufferRenderbuffer" => {
                self.framebuffer_renderbuffer(u(call, 1), u(call, 2), u(call, 3), u(call, 4));
                Answer::Nothing
            }
            "checkFramebufferStatus" => match self.framebuffer_of(u(call, 1)) {
                Err(e) => {
                    self.error(e);
                    Answer::Number(0.0)
                }
                Ok(Some(fb)) => Answer::Number(f64::from(self.status(fb))),
                Ok(None) => Answer::Number(f64::from(FRAMEBUFFER_COMPLETE)),
            },
            "getFramebufferAttachmentParameter" => {
                self.attachment_parameter(u(call, 1), u(call, 2), u(call, 3))
            }
            "drawBuffers" => {
                let list: Vec<u32> = call.numbers.iter().skip(1).map(|&v| v as u32).collect();
                self.draw_buffers(&list);
                Answer::Nothing
            }
            "readBuffer" => {
                let source = u(call, 1);
                match self.surfaces.read_framebuffer {
                    None if matches!(source, BACK | NONE) => self.surfaces.default_read = source,
                    None => self.error(INVALID_OPERATION),
                    Some(fb) => {
                        let ok = source == NONE
                            || (COLOR_ATTACHMENT0..COLOR_ATTACHMENT0 + MAX_COLORS)
                                .contains(&source);
                        if !ok {
                            self.error(if source == BACK {
                                INVALID_OPERATION
                            } else {
                                INVALID_ENUM
                            });
                        } else if let Some(fb) = self.surfaces.framebuffers.get_mut(&fb) {
                            fb.read_buffer = source;
                        }
                    }
                }
                Answer::Nothing
            }
            "clearBuffer" => {
                self.clear_buffer(call, size);
                Answer::Nothing
            }
            "blitFramebuffer" => {
                self.blit_framebuffer(call, size);
                Answer::Nothing
            }
            "invalidateFramebuffer" | "invalidateSubFramebuffer" => {
                // Nothing is thrown away; the calls are checked.
                if let Err(e) = self.framebuffer_of(u(call, 1)) {
                    self.error(e);
                } else {
                    let count = if op == "invalidateFramebuffer" {
                        call.numbers.len() - 2
                    } else {
                        call.numbers.len() - 6
                    };
                    let bound = self.framebuffer_of(u(call, 1)).ok().flatten();
                    for &a in call.numbers.iter().skip(2).take(count) {
                        let a = a as u32;
                        let ok = match bound {
                            None => matches!(a, COLOR | DEPTH | STENCIL),
                            Some(_) => self.attachment_point(a),
                        };
                        if !ok {
                            self.error(INVALID_ENUM);
                            break;
                        }
                    }
                }
                Answer::Nothing
            }
            "readPixels" => self.read_pixels(call, size),
            _ => return None,
        };
        Some(answer)
    }

    /// The framebuffer a target names: the draw one for FRAMEBUFFER and
    /// DRAW_FRAMEBUFFER, the read one for READ_FRAMEBUFFER.
    fn framebuffer_of(&self, target: u32) -> Result<Option<u32>, u32> {
        match target {
            FRAMEBUFFER => Ok(self.surfaces.framebuffer),
            DRAW_FRAMEBUFFER if self.version == 2 => Ok(self.surfaces.framebuffer),
            READ_FRAMEBUFFER if self.version == 2 => Ok(self.surfaces.read_framebuffer),
            _ => Err(INVALID_ENUM),
        }
    }

    /// How many colour attachments a framebuffer may have.
    fn colors(&self) -> u32 {
        if self.version == 2 || self.ext.on("WEBGL_draw_buffers") {
            MAX_COLORS
        } else {
            1
        }
    }

    pub(crate) fn attachment_point(&self, attachment: u32) -> bool {
        matches!(
            attachment,
            DEPTH_ATTACHMENT | STENCIL_ATTACHMENT | DEPTH_STENCIL_ATTACHMENT
        ) || (COLOR_ATTACHMENT0..COLOR_ATTACHMENT0 + self.colors()).contains(&attachment)
    }

    /// A texture's attachments gone from the bound framebuffers (GL's
    /// deletes do).
    pub(crate) fn detach_texture(&mut self, id: u32) {
        for fb in [self.surfaces.framebuffer, self.surfaces.read_framebuffer]
            .into_iter()
            .flatten()
        {
            if let Some(fb) = self.surfaces.framebuffers.get_mut(&fb) {
                fb.attachments
                    .retain(|_, a| !matches!(a, Attached::Texture { id: t, .. } if *t == id));
            }
        }
    }

    fn detach(&mut self, what: Attached) {
        for fb in [self.surfaces.framebuffer, self.surfaces.read_framebuffer]
            .into_iter()
            .flatten()
        {
            if let Some(fb) = self.surfaces.framebuffers.get_mut(&fb) {
                fb.attachments.retain(|_, a| *a != what);
            }
        }
    }

    /// Deleted textures and renderbuffers no framebuffer holds: gone, in
    /// the GPU process too.
    pub(crate) fn sweep(&mut self) {
        let held: Vec<Attached> = self
            .surfaces
            .framebuffers
            .values()
            .flat_map(|fb| fb.attachments.values().copied())
            .collect();
        let textures: Vec<u32> = self
            .surfaces
            .textures
            .iter()
            .filter(|(id, t)| {
                t.deleted
                    && !held
                        .iter()
                        .any(|a| matches!(a, Attached::Texture { id: h, .. } if h == *id))
            })
            .map(|(id, _)| *id)
            .collect();
        let renderbuffers: Vec<u32> = self
            .surfaces
            .renderbuffers
            .iter()
            .filter(|(id, r)| r.deleted && !held.contains(&Attached::Renderbuffer(**id)))
            .map(|(id, _)| *id)
            .collect();
        if textures.is_empty() && renderbuffers.is_empty() {
            return;
        }
        let mut gone = Vec::new();
        for id in textures {
            if let Some(t) = self.surfaces.textures.remove(&id) {
                for image in t.images.values() {
                    gone.extend(Self::image_objects(image));
                }
                if let Some((a, b, _)) = t.assembled {
                    gone.extend([a, b]);
                }
            }
        }
        for id in renderbuffers {
            if let Some(r) = self.surfaces.renderbuffers.remove(&id) {
                for (a, b) in [r.gpu, r.resolve].into_iter().flatten() {
                    gone.extend([a, b]);
                }
            }
        }
        self.free(gone);
    }

    // ---- Renderbuffers ----

    fn renderbuffer_storage(&mut self, target: u32, samples: i32, internal: u32, w: i32, h: i32) {
        if target != RENDERBUFFER {
            self.error(INVALID_ENUM);
            return;
        }
        let ext = |n: &str| self.ext.on(n);
        let Some(format) = formats::renderbuffer_format(
            self.version,
            &ext,
            self.features.rg11b10_renderable,
            self.features.depth32_stencil,
            internal,
        ) else {
            self.error(INVALID_ENUM);
            return;
        };
        if samples < 0 || w < 0 || h < 0 || w as u32 > MAX_SIZE || h as u32 > MAX_SIZE {
            self.error(INVALID_VALUE);
            return;
        }
        if samples > 4 || (samples > 0 && kind_of(format.storage) != 0) {
            self.error(INVALID_OPERATION);
            return;
        }
        let Some(id) = self.surfaces.renderbuffer else {
            self.error(INVALID_OPERATION);
            return;
        };
        let old: Vec<u32> = {
            let r = &self.surfaces.renderbuffers[&id];
            [r.gpu, r.resolve]
                .into_iter()
                .flatten()
                .flat_map(|(a, b)| [a, b])
                .collect()
        };
        self.free(old);
        let count = if samples > 0 { 4 } else { 1 };
        let storage = format.storage;
        let color = !format.depth();
        let make = |this: &mut Context, usage: u32, count: u32| {
            let (texture, view) = (this.gpu.id(&this.client), this.gpu.id(&this.client));
            this.client.queue([
                Request::CreateTexture {
                    device: this.device,
                    id: texture,
                    desc: format!(
                        r#"{{"size":[{w},{h},1],"format":"{storage}","usage":{usage},"sampleCount":{count}}}"#
                    ),
                },
                Request::CreateView {
                    texture,
                    id: view,
                    desc: "{}".to_owned(),
                },
            ]);
            (texture, view)
        };
        let (gpu, resolve) = if w > 0 && h > 0 {
            // RENDER_ATTACHMENT, TEXTURE_BINDING (blits read it), and a
            // single-sampled colour's COPY_SRC (reads copy it).
            let usage = if color && count == 1 { 21 } else { 20 };
            let gpu = make(self, usage, count);
            let resolve = (color && count > 1).then(|| make(self, 21, 1));
            (Some(gpu), resolve)
        } else {
            (None, None)
        };
        let r = self.surfaces.renderbuffers.get_mut(&id).expect("bound");
        r.internal = internal;
        r.width = w as u32;
        r.height = h as u32;
        r.samples = if samples > 0 { 4 } else { 0 };
        r.storage = Some(storage);
        r.gpu = gpu;
        r.resolve = resolve;
    }

    fn renderbuffer_parameter(&mut self, target: u32, name: u32) -> Answer {
        if target != RENDERBUFFER {
            self.error(INVALID_ENUM);
            return Answer::Nothing;
        }
        let Some(id) = self.surfaces.renderbuffer else {
            self.error(INVALID_OPERATION);
            return Answer::Nothing;
        };
        let r = &self.surfaces.renderbuffers[&id];
        let bits = r.storage.map_or([0; 6], |s| bits_of(r.internal, s));
        let v = match name {
            RENDERBUFFER_WIDTH => r.width,
            RENDERBUFFER_HEIGHT => r.height,
            RENDERBUFFER_INTERNAL_FORMAT => r.internal,
            RENDERBUFFER_SAMPLES if self.version == 2 => r.samples,
            RENDERBUFFER_RED_SIZE..=RENDERBUFFER_STENCIL_SIZE => {
                bits[(name - RENDERBUFFER_RED_SIZE) as usize]
            }
            _ => {
                self.error(INVALID_ENUM);
                return Answer::Nothing;
            }
        };
        Answer::Number(f64::from(v))
    }

    // ---- Framebuffers ----

    fn framebuffer_texture(
        &mut self,
        target: u32,
        attachment: u32,
        textarget: Option<u32>,
        texture: u32,
        level: i32,
        layer: i32,
    ) {
        let fb = match self.framebuffer_of(target) {
            Ok(fb) => fb,
            Err(e) => {
                self.error(e);
                return;
            }
        };
        if !self.attachment_point(attachment) {
            self.error(INVALID_ENUM);
            return;
        }
        let Some(fb) = fb else {
            self.error(INVALID_OPERATION);
            return;
        };
        let attached = if texture == 0 {
            None
        } else {
            let Some(t) = self.surfaces.textures.get(&texture).filter(|t| !t.deleted) else {
                self.error(INVALID_OPERATION);
                return;
            };
            let face = match textarget {
                Some(textarget) => {
                    let Some((slot, face)) = super::texture::image_target(textarget, 1) else {
                        self.error(INVALID_ENUM);
                        return;
                    };
                    let wanted = if slot == super::texture::SLOT_2D {
                        TEXTURE_2D
                    } else {
                        TEXTURE_CUBE_MAP
                    };
                    if t.target != Some(wanted) {
                        self.error(INVALID_OPERATION);
                        return;
                    }
                    face
                }
                None => {
                    // framebufferTextureLayer: a 3D or array texture's layer.
                    if !matches!(t.target, Some(TEXTURE_3D | TEXTURE_2D_ARRAY)) {
                        self.error(INVALID_OPERATION);
                        return;
                    }
                    if layer < 0 || layer as u32 >= super::texture::MAX_3D {
                        self.error(INVALID_VALUE);
                        return;
                    }
                    0
                }
            };
            let mipmaps = self.version == 2 || self.ext.on("OES_fbo_render_mipmap");
            if !(0..=super::texture::MAX_LEVEL).contains(&level) || (level != 0 && !mipmaps) {
                self.error(INVALID_VALUE);
                return;
            }
            Some(Attached::Texture {
                id: texture,
                face,
                level: level as u8,
                layer: layer.max(0) as u32,
            })
        };
        self.attach(fb, attachment, attached);
    }

    /// An attachment set or cleared; WebGL 2's DEPTH_STENCIL_ATTACHMENT
    /// is both depth and stencil.
    fn attach(&mut self, fb: u32, attachment: u32, attached: Option<Attached>) {
        let v2 = self.version == 2;
        let attachments = &mut self
            .surfaces
            .framebuffers
            .get_mut(&fb)
            .expect("bound")
            .attachments;
        let points: &[u32] = if v2 && attachment == DEPTH_STENCIL_ATTACHMENT {
            &[DEPTH_ATTACHMENT, STENCIL_ATTACHMENT]
        } else {
            std::slice::from_ref(&attachment)
        };
        for &point in points {
            match attached {
                Some(a) => attachments.insert(point, a),
                None => attachments.remove(&point),
            };
        }
        self.sweep();
    }

    fn framebuffer_renderbuffer(
        &mut self,
        target: u32,
        attachment: u32,
        rb_target: u32,
        renderbuffer: u32,
    ) {
        let fb = match self.framebuffer_of(target) {
            Ok(fb) => fb,
            Err(e) => {
                self.error(e);
                return;
            }
        };
        if !self.attachment_point(attachment) || rb_target != RENDERBUFFER {
            self.error(INVALID_ENUM);
            return;
        }
        let Some(fb) = fb else {
            self.error(INVALID_OPERATION);
            return;
        };
        if renderbuffer != 0
            && !self
                .surfaces
                .renderbuffers
                .get(&renderbuffer)
                .is_some_and(|r| !r.deleted)
        {
            self.error(INVALID_OPERATION);
            return;
        }
        self.attach(
            fb,
            attachment,
            (renderbuffer != 0).then_some(Attached::Renderbuffer(renderbuffer)),
        );
    }

    /// What an attachment is: none when its object is gone.
    fn info(&self, a: &Attached) -> Option<Info> {
        let ext = |n: &str| self.ext.on(n);
        match *a {
            Attached::Texture {
                id,
                face,
                level,
                layer,
            } => {
                let image = self
                    .surfaces
                    .textures
                    .get(&id)?
                    .images
                    .get(&(face, level))?;
                let f = image.format;
                let color_ok = !f.depth()
                    && super::texture::renderable_storage(
                        f.storage,
                        self.features.rg11b10_renderable,
                    )
                    && formats::color_renderable(
                        self.version,
                        &ext,
                        self.features.rg11b10_renderable,
                        &f,
                    );
                Some(Info {
                    storage: f.storage,
                    internal: f.internal,
                    size: (image.width, image.height),
                    samples: 0,
                    color_ok,
                    opaque: f.opaque(),
                    made: image.gpu.is_some() && layer < image.depth,
                })
            }
            Attached::Renderbuffer(id) => {
                let r = self.surfaces.renderbuffers.get(&id)?;
                let storage = r.storage.unwrap_or("rgba8unorm");
                Some(Info {
                    storage,
                    internal: r.internal,
                    size: (r.width, r.height),
                    samples: r.samples,
                    color_ok: !has_depth(storage) && !has_stencil(storage),
                    opaque: matches!(r.internal, RGB565 | RGB8 | RGB16F_EXT | R11F_G11F_B10F),
                    made: r.gpu.is_some(),
                })
            }
        }
    }

    /// A framebuffer's completeness (GL ES 3.0 4.4.4, WebGL 1 6.6 and
    /// WebGL 2's equal sizes).
    pub(crate) fn status(&self, fb: u32) -> u32 {
        let Some(fb) = self.surfaces.framebuffers.get(&fb) else {
            return FRAMEBUFFER_COMPLETE;
        };
        let mut sizes = Vec::new();
        let mut samples = Vec::new();
        for (&point, a) in &fb.attachments {
            let Some(info) = self.info(a) else {
                return FRAMEBUFFER_INCOMPLETE_ATTACHMENT;
            };
            if info.size.0 == 0 || info.size.1 == 0 || !info.made {
                return FRAMEBUFFER_INCOMPLETE_ATTACHMENT;
            }
            let ok = match point {
                DEPTH_ATTACHMENT => has_depth(info.storage),
                STENCIL_ATTACHMENT => has_stencil(info.storage),
                DEPTH_STENCIL_ATTACHMENT => has_depth(info.storage) && has_stencil(info.storage),
                _ => info.color_ok,
            };
            if !ok {
                return FRAMEBUFFER_INCOMPLETE_ATTACHMENT;
            }
            sizes.push(info.size);
            samples.push(info.samples);
        }
        if sizes.is_empty() {
            return FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT;
        }
        if sizes.iter().any(|&s| s != sizes[0]) {
            return FRAMEBUFFER_INCOMPLETE_DIMENSIONS;
        }
        if samples.iter().any(|&s| s != samples[0]) {
            return FRAMEBUFFER_INCOMPLETE_MULTISAMPLE;
        }
        // One depth-and-stencil texture: WebGL 1's points apart are
        // unsupported; WebGL 2's must be the same image.
        let depth = fb.attachments.get(&DEPTH_ATTACHMENT);
        let stencil = fb.attachments.get(&STENCIL_ATTACHMENT);
        let both = fb.attachments.get(&DEPTH_STENCIL_ATTACHMENT);
        let unsupported = if self.version == 2 {
            matches!((depth, stencil), (Some(d), Some(s)) if d != s)
        } else {
            [depth, stencil, both]
                .iter()
                .filter(|a| a.is_some())
                .count()
                > 1
        };
        if unsupported {
            return FRAMEBUFFER_UNSUPPORTED;
        }
        FRAMEBUFFER_COMPLETE
    }

    fn attachment_parameter(&mut self, target: u32, attachment: u32, name: u32) -> Answer {
        let fb = match self.framebuffer_of(target) {
            Ok(fb) => fb,
            Err(e) => {
                self.error(e);
                return Answer::Nothing;
            }
        };
        let Some(fb) = fb else {
            return self.default_attachment_parameter(attachment, name);
        };
        if !self.attachment_point(attachment) {
            self.error(INVALID_ENUM);
            return Answer::Nothing;
        }
        let attachments = &self.surfaces.framebuffers[&fb].attachments;
        let a = if self.version == 2 && attachment == DEPTH_STENCIL_ATTACHMENT {
            let (d, s) = (
                attachments.get(&DEPTH_ATTACHMENT),
                attachments.get(&STENCIL_ATTACHMENT),
            );
            if d != s {
                self.error(INVALID_OPERATION);
                return Answer::Nothing;
            }
            d.copied()
        } else {
            attachments.get(&attachment).copied()
        };
        let srgb = self.version == 2 || self.ext.on("EXT_sRGB");
        let float = self.version == 2
            || self.ext.on("WEBGL_color_buffer_float")
            || self.ext.on("EXT_color_buffer_half_float");
        let info = a.and_then(|a| self.info(&a));
        match (a, name) {
            (None, FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE) => Answer::Number(f64::from(NONE)),
            (None, FRAMEBUFFER_ATTACHMENT_OBJECT_NAME) if self.version == 2 => Answer::Number(0.0),
            (None, _) => {
                self.error(if self.version == 2 {
                    INVALID_OPERATION
                } else {
                    INVALID_ENUM
                });
                Answer::Nothing
            }
            (Some(a), FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE) => Answer::Number(f64::from(match a {
                Attached::Texture { .. } => TEXTURE,
                Attached::Renderbuffer(_) => RENDERBUFFER,
            })),
            (Some(a), FRAMEBUFFER_ATTACHMENT_OBJECT_NAME) => Answer::Number(f64::from(match a {
                Attached::Texture { id, .. } | Attached::Renderbuffer(id) => id,
            })),
            (Some(Attached::Texture { level, .. }), FRAMEBUFFER_ATTACHMENT_TEXTURE_LEVEL) => {
                Answer::Number(f64::from(level))
            }
            (
                Some(Attached::Texture { id, face, .. }),
                FRAMEBUFFER_ATTACHMENT_TEXTURE_CUBE_MAP_FACE,
            ) => {
                let cube = self.surfaces.textures.get(&id).and_then(|t| t.target)
                    == Some(TEXTURE_CUBE_MAP);
                Answer::Number(if cube {
                    f64::from(TEXTURE_CUBE_MAP_POSITIVE_X + u32::from(face))
                } else {
                    0.0
                })
            }
            (Some(Attached::Texture { layer, .. }), FRAMEBUFFER_ATTACHMENT_TEXTURE_LAYER)
                if self.version == 2 =>
            {
                Answer::Number(f64::from(layer))
            }
            (Some(_), FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING_EXT) if srgb => {
                let storage = info.map_or("", |i| i.storage);
                Answer::Number(f64::from(if storage.ends_with("srgb") {
                    SRGB_COLOR_ENCODING
                } else {
                    LINEAR_COLOR_ENCODING
                }))
            }
            (Some(_), FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE_EXT) if float => {
                if self.version == 2 && attachment == DEPTH_STENCIL_ATTACHMENT {
                    self.error(INVALID_OPERATION);
                    return Answer::Nothing;
                }
                let storage = info.map_or("", |i| i.storage);
                Answer::Number(f64::from(component_type(storage)))
            }
            (Some(_), FRAMEBUFFER_ATTACHMENT_RED_SIZE..=FRAMEBUFFER_ATTACHMENT_STENCIL_SIZE)
                if self.version == 2 =>
            {
                let bits = info.map_or([0; 6], |i| bits_of(i.internal, i.storage));
                let k = match name {
                    FRAMEBUFFER_ATTACHMENT_RED_SIZE => 0,
                    FRAMEBUFFER_ATTACHMENT_GREEN_SIZE => 1,
                    FRAMEBUFFER_ATTACHMENT_BLUE_SIZE => 2,
                    FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE => 3,
                    FRAMEBUFFER_ATTACHMENT_DEPTH_SIZE => 4,
                    _ => 5,
                };
                Answer::Number(f64::from(bits[k]))
            }
            _ => {
                self.error(INVALID_ENUM);
                Answer::Nothing
            }
        }
    }

    /// WebGL 2's queries of the drawing buffer's attachments.
    fn default_attachment_parameter(&mut self, attachment: u32, name: u32) -> Answer {
        if self.version == 1 {
            self.error(INVALID_OPERATION);
            return Answer::Nothing;
        }
        let a = &self.attributes;
        let present = match attachment {
            BACK => true,
            DEPTH => a.depth,
            STENCIL => a.stencil,
            _ => {
                self.error(INVALID_ENUM);
                return Answer::Nothing;
            }
        };
        let bits: [u32; 6] = match attachment {
            BACK => [8, 8, 8, if a.alpha { 8 } else { 0 }, 0, 0],
            DEPTH => [0, 0, 0, 0, if a.depth { 24 } else { 0 }, 0],
            _ => [0, 0, 0, 0, 0, if a.stencil { 8 } else { 0 }],
        };
        match name {
            FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE => {
                Answer::Number(f64::from(if present { FRAMEBUFFER_DEFAULT } else { NONE }))
            }
            FRAMEBUFFER_ATTACHMENT_RED_SIZE..=FRAMEBUFFER_ATTACHMENT_STENCIL_SIZE if present => {
                Answer::Number(f64::from(
                    bits[(name - FRAMEBUFFER_ATTACHMENT_RED_SIZE) as usize],
                ))
            }
            FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE_EXT if present && attachment != STENCIL => {
                // The colour's bytes and the depth's 24 bits alike.
                Answer::Number(f64::from(UNSIGNED_NORMALIZED_EXT))
            }
            FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING_EXT if present => {
                Answer::Number(f64::from(LINEAR_COLOR_ENCODING))
            }
            _ => {
                self.error(if present {
                    INVALID_ENUM
                } else {
                    INVALID_OPERATION
                });
                Answer::Nothing
            }
        }
    }

    fn draw_buffers(&mut self, list: &[u32]) {
        match self.surfaces.framebuffer {
            None => {
                if list.len() != 1 || !matches!(list[0], BACK | NONE) {
                    self.error(INVALID_OPERATION);
                } else {
                    self.surfaces.default_draw = list[0];
                }
            }
            Some(fb) => {
                if list.len() as u32 > MAX_COLORS {
                    self.error(INVALID_VALUE);
                    return;
                }
                for (k, &b) in list.iter().enumerate() {
                    if b != NONE && b != COLOR_ATTACHMENT0 + k as u32 {
                        self.error(
                            if b == BACK
                                || (COLOR_ATTACHMENT0..COLOR_ATTACHMENT0 + MAX_COLORS).contains(&b)
                            {
                                INVALID_OPERATION
                            } else {
                                INVALID_ENUM
                            },
                        );
                        return;
                    }
                }
                if let Some(fb) = self.surfaces.framebuffers.get_mut(&fb) {
                    fb.draw_buffers = list.to_vec();
                }
            }
        }
    }

    /// The draw buffer `k`'s value (DRAW_BUFFERi).
    pub(crate) fn draw_buffer(&self, k: u32) -> u32 {
        match self
            .surfaces
            .framebuffer
            .and_then(|fb| self.surfaces.framebuffers.get(&fb))
        {
            None => {
                if k == 0 {
                    self.surfaces.default_draw
                } else {
                    NONE
                }
            }
            Some(fb) => fb.draw_buffers.get(k as usize).copied().unwrap_or(NONE),
        }
    }

    /// READ_BUFFER's value.
    pub(crate) fn read_buffer(&self) -> u32 {
        match self
            .surfaces
            .read_framebuffer
            .and_then(|fb| self.surfaces.framebuffers.get(&fb))
        {
            None => self.surfaces.default_read,
            Some(fb) => fb.read_buffer,
        }
    }

    /// A drawn-into attachment's view (a layer's, a 3D slice's) and the
    /// view it resolves to.
    fn attachment_view(&mut self, a: &Attached) -> Option<(u32, Option<u32>, Option<u32>)> {
        match *a {
            Attached::Texture {
                id,
                face,
                level,
                layer,
            } => {
                let (view, slice) = self.layer_view(id, (face, level), layer)?;
                Some((view, None, slice))
            }
            Attached::Renderbuffer(id) => {
                let r = self.surfaces.renderbuffers.get(&id)?;
                Some((r.gpu?.1, r.resolve.map(|v| v.1), None))
            }
        }
    }

    /// What draws and clears go into: the draw framebuffer's attachments,
    /// or the drawing buffer; none, and GL's error, for an incomplete
    /// framebuffer.
    pub(crate) fn target(&mut self, size: (u32, u32)) -> Option<Target> {
        let Some(fb) = self.surfaces.framebuffer else {
            self.drawing_buffer(size);
            let d = self.drawing.clone().expect("made");
            let (color, resolve) = match d.msaa {
                Some((_, v)) => (v, Some(d.color.1)),
                None => (d.color.1, None),
            };
            let stencil = self.attributes.stencil;
            let depth = d.depth.map(|(_, v)| {
                (
                    v,
                    if stencil {
                        "depth24plus-stencil8"
                    } else {
                        "depth24plus"
                    },
                )
            });
            let drawn = self.surfaces.default_draw == BACK;
            return Some(Target {
                key: vec![if drawn { color } else { 0 }, depth.map_or(0, |d| d.0)],
                size: d.size,
                colors: vec![drawn.then_some(ColorTarget {
                    view: color,
                    resolve,
                    slice: None,
                    storage: "rgba8unorm",
                    rgb: !self.attributes.alpha,
                })],
                depth,
                depth_on: self.attributes.depth,
                stencil_on: self.attributes.stencil,
                samples: if self.attributes.antialias { 4 } else { 1 },
                default: true,
                textures: Vec::new(),
            });
        };
        if self.status(fb) != FRAMEBUFFER_COMPLETE {
            self.error(INVALID_FRAMEBUFFER_OPERATION);
            return None;
        }
        let framebuffer = self.surfaces.framebuffers[&fb].clone();
        let mut target = Target {
            key: Vec::new(),
            size: (0, 0),
            colors: Vec::new(),
            depth: None,
            depth_on: false,
            stencil_on: false,
            samples: 1,
            default: false,
            textures: Vec::new(),
        };
        for (point, a) in &framebuffer.attachments {
            let info = self.info(a).expect("complete");
            target.size = info.size;
            target.samples = info.samples.max(1);
            if let Attached::Texture { id, .. } = a {
                target.textures.push(*id);
            }
            if matches!(
                *point,
                DEPTH_ATTACHMENT | STENCIL_ATTACHMENT | DEPTH_STENCIL_ATTACHMENT
            ) {
                let (view, _, _) = self.attachment_view(a).expect("complete");
                target.depth = Some((view, info.storage));
                target.depth_on = has_depth(info.storage);
                target.stencil_on = has_stencil(info.storage);
            }
        }
        for (k, &buffer) in framebuffer.draw_buffers.iter().enumerate() {
            let point = COLOR_ATTACHMENT0 + k as u32;
            let color = match framebuffer.attachments.get(&point) {
                Some(a) if buffer == point => {
                    let info = self.info(a).expect("complete");
                    let (view, resolve, slice) = self.attachment_view(a).expect("complete");
                    Some(ColorTarget {
                        view,
                        resolve,
                        slice,
                        storage: info.storage,
                        rgb: info.opaque,
                    })
                }
                _ => None,
            };
            target.colors.push(color);
        }
        while target.colors.last().is_some_and(Option::is_none) {
            target.colors.pop();
        }
        for c in &target.colors {
            target.key.push(c.as_ref().map_or(0, |c| c.view));
            target
                .key
                .push(c.as_ref().and_then(|c| c.slice).unwrap_or(0));
        }
        target.key.push(target.depth.map_or(0, |d| d.0));
        Some(target)
    }

    /// What readPixels, the copies and blits read: the read framebuffer's
    /// read buffer, or the drawing buffer's (its clear made first); none,
    /// and GL's error, when there is none to read.
    pub(crate) fn read_source(&mut self, size: (u32, u32)) -> Option<Source> {
        let Some(fb) = self.surfaces.read_framebuffer else {
            if self.surfaces.default_read == NONE {
                self.error(INVALID_OPERATION);
                return None;
            }
            if self.needs_clear {
                // The drawing buffer cleared, as presented ones are.
                let draw = self.surfaces.framebuffer.take();
                self.pass(size, None);
                self.surfaces.framebuffer = draw;
            }
            self.gpu_flush();
            self.drawing_buffer(size);
            let d = self.drawing.clone().expect("made");
            return Some(Source {
                texture: d.color.0,
                view: d.color.1,
                size: d.size,
                storage: "rgba8unorm",
                layer: 0,
                opaque: !self.attributes.alpha,
                image: None,
                samples: 0,
            });
        };
        if self.status(fb) != FRAMEBUFFER_COMPLETE {
            self.error(INVALID_FRAMEBUFFER_OPERATION);
            return None;
        }
        let framebuffer = &self.surfaces.framebuffers[&fb];
        let read = framebuffer.read_buffer;
        let Some(a) = (read != NONE)
            .then(|| framebuffer.attachments.get(&read).copied())
            .flatten()
        else {
            self.error(INVALID_OPERATION);
            return None;
        };
        self.gpu_flush();
        let info = self.info(&a).expect("complete");
        let (texture, view, layer, image) = match a {
            Attached::Texture {
                id,
                face,
                level,
                layer,
            } => {
                let slot = self.surfaces.textures[&id]
                    .target
                    .and_then(|t| super::texture::slot_of(t, self.version))
                    .unwrap_or(0);
                let image_gpu = self.surfaces.textures[&id].images[&(face, level)]
                    .gpu
                    .expect("complete");
                // Blits and copies read a 2D view of the layer.
                let view = if slot == super::texture::SLOT_ARRAY {
                    self.layer_view(id, (face, level), layer)
                        .map_or(image_gpu.2, |v| v.0)
                } else {
                    image_gpu.2
                };
                (image_gpu.0, view, layer, Some((id, face, level)))
            }
            Attached::Renderbuffer(id) => {
                let r = &self.surfaces.renderbuffers[&id];
                let (texture, view) = r.resolve.or(r.gpu).expect("complete");
                (texture, view, 0, None)
            }
        };
        Some(Source {
            texture,
            view,
            size: info.size,
            storage: info.storage,
            layer,
            opaque: info.opaque,
            image,
            samples: info.samples,
        })
    }

    /// The format and type readPixels takes besides RGBA and
    /// UNSIGNED_BYTE (or RGBA_INTEGER and INT or UNSIGNED_INT): the read
    /// buffer's own.
    pub(crate) fn read_format(&self) -> (u32, u32) {
        let storage = match self
            .surfaces
            .read_framebuffer
            .and_then(|fb| self.surfaces.framebuffers.get(&fb))
        {
            None => "rgba8unorm",
            Some(fb) => fb
                .attachments
                .get(&fb.read_buffer)
                .and_then(|a| self.info(a))
                .map_or("rgba8unorm", |i| i.storage),
        };
        own_format(self.version, storage)
    }

    /// readPixels [context, x, y, width, height, format, type, array
    /// kind, offset] with the array's bytes (none: the pixel pack
    /// buffer's at the offset): the bytes with the rectangle's pixels in
    /// the framebuffer written, rows from the bottom; nothing on an
    /// error.
    fn read_pixels(&mut self, call: &CanvasCall, size: (u32, u32)) -> Answer {
        let (x, y, w, h) = (i(call, 1), i(call, 2), i(call, 3), i(call, 4));
        let (format, kind, view, offset) = (u(call, 5), u(call, 6), u(call, 7), n(call, 8));
        let formats_known = matches!(
            format,
            ALPHA
                | RGB
                | RGBA
                | RED
                | RG
                | RED_INTEGER
                | RG_INTEGER
                | RGB_INTEGER
                | RGBA_INTEGER
                | LUMINANCE
                | LUMINANCE_ALPHA
        );
        let kinds_known = matches!(
            kind,
            UNSIGNED_BYTE
                | BYTE
                | UNSIGNED_SHORT
                | SHORT
                | UNSIGNED_INT
                | INT
                | HALF_FLOAT
                | HALF_FLOAT_OES
                | FLOAT
                | UNSIGNED_SHORT_5_6_5
                | UNSIGNED_SHORT_4_4_4_4
                | UNSIGNED_SHORT_5_5_5_1
                | UNSIGNED_INT_2_10_10_10_REV
                | UNSIGNED_INT_10F_11F_11F_REV
                | UNSIGNED_INT_5_9_9_9_REV
        );
        if !formats_known || !kinds_known {
            self.error(INVALID_ENUM);
            return Answer::Nothing;
        }
        if w < 0 || h < 0 {
            self.error(INVALID_VALUE);
            return Answer::Nothing;
        }
        if let Some(fb) = self.surfaces.read_framebuffer
            && self.status(fb) != FRAMEBUFFER_COMPLETE
        {
            self.error(INVALID_FRAMEBUFFER_OPERATION);
            return Answer::Nothing;
        }
        // What the read buffer takes: RGBA by its kind, or its own.
        let storage = match self.surfaces.read_framebuffer {
            None => "rgba8unorm",
            Some(fb) => {
                let fb = &self.surfaces.framebuffers[&fb];
                fb.attachments
                    .get(&fb.read_buffer)
                    .and_then(|a| self.info(a))
                    .map_or("rgba8unorm", |i| i.storage)
            }
        };
        let mandatory = match kind_of(storage) {
            1 => (RGBA_INTEGER, INT),
            2 => (RGBA_INTEGER, UNSIGNED_INT),
            _ if storage.ends_with("float") && !storage.ends_with("ufloat") => (RGBA, FLOAT),
            _ if storage.ends_with("ufloat") => (RGBA, FLOAT),
            _ => (RGBA, UNSIGNED_BYTE),
        };
        let own = own_format(self.version, storage);
        let mandatory = if self.version == 1 && storage.ends_with("float") {
            (RGBA, FLOAT)
        } else {
            mandatory
        };
        if (format, kind) != mandatory && (format, kind) != own {
            self.error(INVALID_OPERATION);
            return Answer::Nothing;
        }
        let pack = self.v2.pixel_pack;
        let from_array = call.bytes.is_some();
        if (pack.is_some() && from_array) || (pack.is_none() && !from_array) {
            self.error(INVALID_OPERATION);
            return Answer::Nothing;
        }
        if from_array && !formats::view_ok(kind, view) {
            self.error(INVALID_OPERATION);
            return Answer::Nothing;
        }
        let pixel = formats::client_pixel(format, kind);
        let layout = Layout {
            alignment: self.state.pack_alignment,
            row_length: self.state.pack_row_length,
            image_height: 0,
            skip_pixels: self.state.pack_skip_pixels,
            skip_rows: self.state.pack_skip_rows,
            skip_images: 0,
        };
        let (stride, _, first, whole) = layout.measure(w as u32, h as u32, 1, pixel);
        let mut out = match (pack, &call.bytes) {
            (Some(buffer), _) => {
                let data = &self.buffers[&buffer].data;
                if offset < 0.0 || offset as u64 + whole > data.len() as u64 {
                    self.error(INVALID_OPERATION);
                    return Answer::Nothing;
                }
                data[offset as usize..].to_vec()
            }
            (None, Some(bytes)) => bytes.clone(),
            _ => Vec::new(),
        };
        if (out.len() as u64) < whole {
            self.error(INVALID_OPERATION);
            return Answer::Nothing;
        }
        let Some(source) = self.read_source(size) else {
            return Answer::Nothing;
        };
        if source.samples > 1 {
            self.error(INVALID_OPERATION);
            return Answer::Nothing;
        }
        // The rectangle within the framebuffer; pixels outside it stay.
        let x0 = x.clamp(0, source.size.0 as i32);
        let y0 = y.clamp(0, source.size.1 as i32);
        let x1 = x.saturating_add(w).clamp(0, source.size.0 as i32);
        let y1 = y.saturating_add(h).clamp(0, source.size.1 as i32);
        if x1 > x0 && y1 > y0 {
            let (cw, ch) = ((x1 - x0) as u32, (y1 - y0) as u32);
            let texel = texel_bytes(source.storage);
            let row = (cw * texel).div_ceil(256) * 256;
            let buffer_size = u64::from(row) * u64::from(ch);
            let buffer = self.client.id();
            let commands = self.client.id();
            self.client.queue([
                Request::CreateBuffer {
                    device: self.device,
                    id: buffer,
                    size: buffer_size,
                    usage: 9,
                    mapped: false,
                },
                Request::Finish {
                    device: self.device,
                    id: commands,
                    commands: vec![Command::CopyTextureToBuffer {
                        desc: format!(
                            r#"{{"source":{{"texture":{},"origin":[{x0},{y0},{}]}},"destination":{{"buffer":{buffer},"bytesPerRow":{row}}},"size":[{cw},{ch},1]}}"#,
                            source.texture, source.layer
                        ),
                    }],
                },
                Request::Submit {
                    device: self.device,
                    buffers: vec![commands],
                },
                Request::Drop { id: commands },
            ]);
            let read = wait(&self.client, |call| Request::MapBuffer {
                call,
                buffer,
                mode: 1,
                offset: 0,
                size: buffer_size,
            });
            self.client
                .queue([Request::Unmap { buffer }, Request::Drop { id: buffer }]);
            if let Some((Reply::Mapped { error: None, .. }, bytes)) = read {
                for r in 0..ch as usize {
                    let from = &bytes[r * row as usize..];
                    let at = first as usize
                        + (r + (y0 - y) as usize) * stride as usize
                        + (x0 - x) as usize * pixel as usize;
                    for c in 0..cw as usize {
                        let t = &from[c * texel as usize..(c + 1) * texel as usize];
                        let (mut channels, _) = formats::decode(source.storage, t);
                        if source.opaque {
                            channels[3] = 1.0;
                        }
                        let o = at + c * pixel as usize;
                        formats::encode(format, kind, channels, &mut out[o..o + pixel as usize]);
                    }
                }
            }
        }
        match pack {
            Some(buffer) => {
                let length = self.buffers[&buffer].data.len() - offset as usize;
                out.truncate(length);
                self.buffer_sub_data(buffer, offset as usize, &out);
                Answer::Nothing
            }
            None => Answer::Bytes(out),
        }
    }

    // ---- Clears ----

    /// The clear module, layout and group, made once.
    fn clear_kit(&mut self) -> (u32, u32, u32) {
        if let Some(l) = self.gpu.kit.layout {
            return l;
        }
        let device = self.device;
        let (layout, pipeline_layout, group) = (
            self.gpu.id(&self.client),
            self.gpu.id(&self.client),
            self.gpu.id(&self.client),
        );
        self.client.queue([
            Request::CreateBindGroupLayout {
                device,
                id: layout,
                desc: r#"{"entries":[{"binding":0,"visibility":3,"buffer":{"type":"uniform","hasDynamicOffset":true,"minBindingSize":32}}]}"#.to_owned(),
            },
            Request::CreatePipelineLayout {
                device,
                id: pipeline_layout,
                layouts: vec![layout],
            },
            Request::CreateBindGroup {
                device,
                id: group,
                desc: format!(
                    r#"{{"layout":{layout},"entries":[{{"binding":0,"buffer":{},"offset":0,"size":32}}]}}"#,
                    self.gpu.arena_id()
                ),
            },
        ]);
        self.gpu.kit.layout = Some((layout, pipeline_layout, group));
        (layout, pipeline_layout, group)
    }

    fn clear_module(&mut self, kind: u8, mask: u8) -> u32 {
        if let Some(&m) = self.gpu.kit.modules.get(&(kind, mask)) {
            return m;
        }
        let id = self.gpu.id(&self.client);
        self.client.queue([Request::CreateShaderModule {
            device: self.device,
            id,
            code: clear_code(kind, mask),
        }]);
        self.gpu.kit.modules.insert((kind, mask), id);
        id
    }

    /// A quad drawn into the pass begun: colour `bits` (of `kind`) into
    /// the targets of `colors` (a storage and write mask each, none
    /// skipped), a depth and stencil when asked; scissored as GL's state
    /// says.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn clear_quad(
        &mut self,
        kind: u8,
        bits: [u32; 4],
        colors: &[Option<(&'static str, u32)>],
        depth: Option<(&'static str, bool, u32)>,
        clear_depth: f32,
        reference: u32,
        samples: u32,
        size: (u32, u32),
    ) {
        let (_, pipeline_layout, group) = self.clear_kit();
        let mask = colors
            .iter()
            .enumerate()
            .fold(0u8, |m, (k, c)| if c.is_some() { m | (1 << k) } else { m });
        let module = self.clear_module(kind, mask);
        let targets: Vec<String> = colors
            .iter()
            .map(|c| {
                c.map_or("null".to_owned(), |(f, w)| {
                    format!(r#"{{"format":"{f}","writeMask":{w}}}"#)
                })
            })
            .collect();
        let depth_stencil = depth.map_or(String::new(), |(format, write, stencil_write)| {
            let face = r#"{"compare":"always","passOp":"replace","failOp":"keep","depthFailOp":"keep"}"#;
            let stencil = if has_stencil(format) {
                format!(r#","stencilFront":{face},"stencilBack":{face},"stencilWriteMask":{stencil_write}"#)
            } else {
                String::new()
            };
            let write = write && has_depth(format);
            format!(r#","depthStencil":{{"format":"{format}","depthWriteEnabled":{write},"depthCompare":"always"{stencil}}}"#)
        });
        let desc = format!(
            r#"{{"layout":{pipeline_layout},"vertex":{{"module":{module},"entryPoint":"vs","buffers":[]}},"fragment":{{"module":{module},"entryPoint":"fs","targets":[{}]}},"multisample":{{"count":{samples}}}{depth_stencil}}}"#,
            targets.join(",")
        );
        let pipeline = match self.gpu.kit.pipelines.get(&desc) {
            Some(&p) => p,
            None => {
                let id = self.gpu.id(&self.client);
                self.client.queue([Request::CreateRenderPipeline {
                    device: self.device,
                    id,
                    desc: desc.clone(),
                    call: None,
                }]);
                self.gpu.kit.pipelines.insert(desc, id);
                id
            }
        };
        let mut data = Vec::with_capacity(32);
        for v in bits {
            data.extend_from_slice(&v.to_le_bytes());
        }
        for v in [clear_depth, 0.0, 0.0, 0.0] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let Some(offset) = self.arena_or_flush(&data, 256, size) else {
            return;
        };
        let scissor = self.scissor_rect(size);
        let Some(pass) = self.gpu_pass() else {
            return;
        };
        pass.extend([
            RenderCommand::SetPipeline { pipeline },
            RenderCommand::SetBindGroup {
                index: 0,
                group: Some(group),
                offsets: vec![offset as u32],
            },
            RenderCommand::SetViewport {
                x: 0.0,
                y: 0.0,
                width: size.0 as f32,
                height: size.1 as f32,
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

    /// clear(): every draw buffer's colour, the depth and stencil; a
    /// pass's loads when it may be, else a quad. Integer colour buffers
    /// are INVALID_OPERATION (WebGL 2 5.14).
    pub(crate) fn clear(&mut self, mask: u32, size: (u32, u32)) {
        if mask & !(COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT | STENCIL_BUFFER_BIT) != 0 {
            self.error(INVALID_VALUE);
            return;
        }
        let Some(target) = self.target(size) else {
            return;
        };
        if mask & COLOR_BUFFER_BIT != 0
            && target
                .colors
                .iter()
                .flatten()
                .any(|c| kind_of(c.storage) != 0)
        {
            self.error(INVALID_OPERATION);
            return;
        }
        if mask == 0 {
            return;
        }
        self.drew(&target);
        let s = self.state.clone();
        let whole_color = s.color_mask.iter().all(|&m| m);
        let whole_stencil = s.stencil[0].write_mask & 0xFF == 0xFF;
        let simple = !s.scissor_test
            && (mask & COLOR_BUFFER_BIT == 0 || whole_color)
            && (mask & DEPTH_BUFFER_BIT == 0 || s.depth_mask)
            && (mask & STENCIL_BUFFER_BIT == 0 || whole_stencil);
        if simple {
            self.pass(
                size,
                Some((mask, s.clear_color, s.clear_depth, s.clear_stencil)),
            );
            return;
        }
        // Else a quad, scissored and masked.
        if self.pass(size, None).is_none() {
            return;
        }
        let write = s
            .color_mask
            .iter()
            .enumerate()
            .fold(0, |m, (k, &on)| if on { m | (1 << k) } else { m });
        let colors: Vec<Option<(&'static str, u32)>> = target
            .colors
            .iter()
            .map(|c| {
                c.as_ref().map(|c| {
                    let w = if mask & COLOR_BUFFER_BIT == 0 {
                        0
                    } else if c.rgb {
                        write & 7
                    } else {
                        write
                    };
                    (c.storage, w)
                })
            })
            .collect();
        let mut color = s.clear_color;
        if target.colors.iter().flatten().any(|c| c.rgb) && target.colors.len() == 1 {
            color[3] = 1.0;
        }
        let depth = target.depth.map(|(_, format)| {
            let stencil_write = if mask & STENCIL_BUFFER_BIT != 0 && target.stencil_on {
                s.stencil[0].write_mask & 0xFF
            } else {
                0
            };
            (
                format,
                mask & DEPTH_BUFFER_BIT != 0 && s.depth_mask && target.depth_on,
                stencil_write,
            )
        });
        self.clear_quad(
            0,
            color.map(f32::to_bits),
            &colors,
            depth,
            s.clear_depth,
            s.clear_stencil as u32 & 0xFF,
            target.samples,
            target.size,
        );
    }

    /// clearBuffer{fv,iv,uiv,fi}: [context, buffer, draw buffer, kind (0
    /// float, 1 int, 2 uint, 3 the depth and stencil), values...].
    fn clear_buffer(&mut self, call: &CanvasCall, size: (u32, u32)) {
        let (buffer, drawbuffer, kind) = (u(call, 1), i(call, 2), u(call, 3));
        let values: Vec<f64> = call.numbers.iter().skip(4).copied().collect();
        let ok = match buffer {
            COLOR => kind <= 2,
            DEPTH => kind == 0,
            STENCIL => kind == 1,
            DEPTH_STENCIL => kind == 3,
            _ => false,
        };
        if !ok {
            self.error(INVALID_ENUM);
            return;
        }
        let needed = match buffer {
            COLOR => 4,
            DEPTH_STENCIL => 2,
            _ => 1,
        };
        if values.len() < needed {
            self.error(INVALID_VALUE);
            return;
        }
        if drawbuffer < 0
            || (buffer == COLOR && drawbuffer as u32 >= MAX_COLORS)
            || (buffer != COLOR && drawbuffer != 0)
        {
            self.error(INVALID_VALUE);
            return;
        }
        let Some(target) = self.target(size) else {
            return;
        };
        self.cleared_first(&target, size);
        let s = self.state.clone();
        if buffer == COLOR {
            let Some(Some(color)) = target.colors.get(drawbuffer as usize).cloned() else {
                return;
            };
            if u32::from(kind_of(color.storage)) != kind {
                self.error(INVALID_OPERATION);
                return;
            }
            self.drew(&target);
            let bits: [u32; 4] = match kind {
                0 => [0, 1, 2, 3].map(|k| (values[k] as f32).to_bits()),
                1 => [0, 1, 2, 3].map(|k| values[k] as i32 as u32),
                _ => [0, 1, 2, 3].map(|k| values[k] as u32),
            };
            let write = s
                .color_mask
                .iter()
                .enumerate()
                .fold(0, |m, (k, &on)| if on { m | (1 << k) } else { m });
            let mut colors: Vec<Option<(&'static str, u32)>> = vec![None; drawbuffer as usize];
            colors.push(Some((
                color.storage,
                if color.rgb { write & 7 } else { write },
            )));
            let resolve = color
                .resolve
                .map_or(String::new(), |r| format!(r#","resolveTarget":{r}"#));
            let slice = color
                .slice
                .map_or(String::new(), |s| format!(r#","depthSlice":{s}"#));
            let attachments: Vec<String> = (0..drawbuffer)
                .map(|_| "null".to_owned())
                .chain([format!(
                    r#"{{"view":{}{resolve}{slice},"loadOp":"load","storeOp":"store"}}"#,
                    color.view
                )])
                .collect();
            self.own_pass(format!(
                r#"{{"colorAttachments":[{}]}}"#,
                attachments.join(",")
            ));
            self.clear_quad(
                kind as u8,
                bits,
                &colors,
                None,
                1.0,
                0,
                target.samples,
                target.size,
            );
            return;
        }
        let Some((view, format)) = target.depth else {
            return;
        };
        self.drew(&target);
        let (clear_depth, stencil) = match buffer {
            DEPTH => (values[0] as f32, None),
            STENCIL => (1.0, Some(values[0] as i32)),
            _ => (values[0] as f32, Some(values[1] as i32)),
        };
        let depth_write = buffer != STENCIL && s.depth_mask && target.depth_on;
        let stencil_write = if stencil.is_some() && target.stencil_on {
            s.stencil[0].write_mask & 0xFF
        } else {
            0
        };
        let depth_ops = if has_depth(format) {
            r#","depthLoadOp":"load","depthStoreOp":"store""#
        } else {
            ""
        };
        let stencil_ops = if has_stencil(format) {
            r#","stencilLoadOp":"load","stencilStoreOp":"store""#
        } else {
            ""
        };
        self.own_pass(format!(
            r#"{{"colorAttachments":[],"depthStencilAttachment":{{"view":{view}{depth_ops}{stencil_ops}}}}}"#
        ));
        self.clear_quad(
            0,
            [0; 4],
            &[],
            Some((format, depth_write, stencil_write)),
            clear_depth.clamp(0.0, 1.0),
            stencil.unwrap_or(0) as u32 & 0xFF,
            target.samples,
            target.size,
        );
    }

    /// A presented drawing buffer's clear made before a pass of its own
    /// draws into it.
    fn cleared_first(&mut self, target: &Target, size: (u32, u32)) {
        if target.default && self.needs_clear {
            self.pass(size, None);
        }
    }

    // ---- blitFramebuffer ----

    /// blitFramebuffer [context, source x0, y0, x1, y1, destination x0,
    /// y0, x1, y1, mask, filter]: drawn into each draw buffer from the
    /// read buffer, and the depth from the read depth (stencil: G-list).
    fn blit_framebuffer(&mut self, call: &CanvasCall, size: (u32, u32)) {
        let src = [i(call, 1), i(call, 2), i(call, 3), i(call, 4)];
        let dst = [i(call, 5), i(call, 6), i(call, 7), i(call, 8)];
        let (mask, filter) = (u(call, 9), u(call, 10));
        if mask & !(COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT | STENCIL_BUFFER_BIT) != 0 {
            self.error(INVALID_VALUE);
            return;
        }
        if !matches!(filter, NEAREST | LINEAR) {
            self.error(INVALID_ENUM);
            return;
        }
        if mask & (DEPTH_BUFFER_BIT | STENCIL_BUFFER_BIT) != 0 && filter == LINEAR {
            self.error(INVALID_OPERATION);
            return;
        }
        for fb in [self.surfaces.read_framebuffer, self.surfaces.framebuffer]
            .into_iter()
            .flatten()
        {
            if self.status(fb) != FRAMEBUFFER_COMPLETE {
                self.error(INVALID_FRAMEBUFFER_OPERATION);
                return;
            }
        }
        let Some(target) = self.target(size) else {
            return;
        };
        // The read buffer: none is no colour to copy.
        let source = if mask & COLOR_BUFFER_BIT != 0 && self.read_buffer() != NONE {
            let had = self.errors.clone();
            let s = self.read_source(size);
            self.errors = had;
            s
        } else {
            None
        };
        let read_samples = source.as_ref().map_or(0, |s| s.samples);
        let same_size = (src[2] - src[0], src[3] - src[1]) == (dst[2] - dst[0], dst[3] - dst[1]);
        if read_samples > 1 && (!same_size || target.samples > 1) {
            self.error(INVALID_OPERATION);
            return;
        }
        if let Some(source) = &source {
            let source_kind = kind_of(source.storage);
            for c in target.colors.iter().flatten() {
                if kind_of(c.storage) != source_kind
                    || (source_kind != 0 && filter == LINEAR)
                    || (read_samples > 1 && c.storage != source.storage)
                {
                    self.error(INVALID_OPERATION);
                    return;
                }
            }
            // The same image read and drawn: INVALID_OPERATION.
            if let Some((id, _, _)) = source.image
                && target.textures.contains(&id)
                && target
                    .colors
                    .iter()
                    .flatten()
                    .any(|c| c.view == source.view)
            {
                self.error(INVALID_OPERATION);
                return;
            }
        }
        let depth_source = if mask & DEPTH_BUFFER_BIT != 0 {
            self.read_depth()
        } else {
            None
        };
        if let (Some((_, read_format)), Some((_, draw_format))) = (depth_source, target.depth)
            && read_format != draw_format
        {
            self.error(INVALID_OPERATION);
            return;
        }
        self.cleared_first(&target, size);
        self.drew(&target);
        self.end_pass();
        let uniform = |src_size: (u32, u32)| {
            let mut data = Vec::with_capacity(48);
            for v in [
                src[0], src[1], src[2], src[3], dst[0], dst[1], dst[2], dst[3],
            ] {
                data.extend_from_slice(&(v as f32).to_le_bytes());
            }
            data.extend_from_slice(&(src_size.0 as f32).to_le_bytes());
            data.extend_from_slice(&(src_size.1 as f32).to_le_bytes());
            data.extend_from_slice(&u32::from(filter == LINEAR).to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
            data
        };
        if let Some(source) = source {
            let kind = kind_of(source.storage);
            for (k, c) in target.colors.iter().enumerate() {
                let Some(c) = c else {
                    continue;
                };
                let resolve = c
                    .resolve
                    .map_or(String::new(), |r| format!(r#","resolveTarget":{r}"#));
                let slice = c
                    .slice
                    .map_or(String::new(), |s| format!(r#","depthSlice":{s}"#));
                let attachments: Vec<String> = (0..k)
                    .map(|_| "null".to_owned())
                    .chain([format!(
                        r#"{{"view":{}{resolve}{slice},"loadOp":"load","storeOp":"store"}}"#,
                        c.view
                    )])
                    .collect();
                let target_json: Vec<String> = (0..k)
                    .map(|_| "null".to_owned())
                    .chain([format!(r#"{{"format":"{}"}}"#, c.storage)])
                    .collect();
                self.own_pass(format!(
                    r#"{{"colorAttachments":[{}]}}"#,
                    attachments.join(",")
                ));
                self.blit_draw(
                    kind,
                    k as u8,
                    &target_json.join(","),
                    None,
                    source.view,
                    uniform(source.size),
                    target.samples,
                    &target,
                    dst,
                );
            }
        }
        if let (Some((view, _)), Some((draw_view, format))) = (depth_source, target.depth) {
            let depth_ops = r#","depthLoadOp":"load","depthStoreOp":"store""#;
            let stencil_ops = if has_stencil(format) {
                r#","stencilLoadOp":"load","stencilStoreOp":"store""#
            } else {
                ""
            };
            self.own_pass(format!(
                r#"{{"colorAttachments":[],"depthStencilAttachment":{{"view":{draw_view}{depth_ops}{stencil_ops}}}}}"#
            ));
            let read_size = self.read_size(size);
            self.blit_draw(
                3,
                0,
                "",
                Some(format),
                view,
                uniform(read_size),
                target.samples,
                &target,
                dst,
            );
        }
    }

    /// The read framebuffer's depth: a view of its depth aspect and the
    /// storage (a single-sampled one's: G-list for multisampled).
    fn read_depth(&mut self) -> Option<(u32, &'static str)> {
        match self.surfaces.read_framebuffer {
            None => {
                if !self.attributes.depth || self.attributes.antialias {
                    return None;
                }
                let d = self.drawing.clone()?;
                let (texture, _) = d.depth?;
                let storage = if self.attributes.stencil {
                    "depth24plus-stencil8"
                } else {
                    "depth24plus"
                };
                let view = self.gpu.id(&self.client);
                self.client.queue([Request::CreateView {
                    texture,
                    id: view,
                    desc: r#"{"aspect":"depth-only"}"#.to_owned(),
                }]);
                Some((view, storage))
            }
            Some(fb) => {
                let fb = self.surfaces.framebuffers.get(&fb)?;
                let a = fb
                    .attachments
                    .get(&DEPTH_ATTACHMENT)
                    .or(fb.attachments.get(&DEPTH_STENCIL_ATTACHMENT))?;
                match *a {
                    Attached::Texture {
                        id, face, level, ..
                    } => {
                        let image = self
                            .surfaces
                            .textures
                            .get(&id)?
                            .images
                            .get(&(face, level))?;
                        Some((image.gpu?.2, image.format.storage))
                    }
                    Attached::Renderbuffer(id) => {
                        let r = self.surfaces.renderbuffers.get(&id)?;
                        if r.samples > 1 {
                            return None;
                        }
                        let (texture, _) = r.gpu?;
                        let storage = r.storage?;
                        let view = self.gpu.id(&self.client);
                        self.client.queue([Request::CreateView {
                            texture,
                            id: view,
                            desc: r#"{"aspect":"depth-only"}"#.to_owned(),
                        }]);
                        Some((view, storage))
                    }
                }
            }
        }
    }

    /// The read framebuffer's size.
    fn read_size(&self, size: (u32, u32)) -> (u32, u32) {
        match self
            .surfaces
            .read_framebuffer
            .and_then(|fb| self.surfaces.framebuffers.get(&fb))
        {
            None => self.drawing.as_ref().map_or(size, |d| d.size),
            Some(fb) => fb
                .attachments
                .values()
                .next()
                .and_then(|a| self.info(a))
                .map_or(size, |i| i.size),
        }
    }

    /// A blit drawn into the pass begun: from `view` (of `kind`) into the
    /// colour at `index` (`targets`' JSON) or the depth of `depth`.
    #[allow(clippy::too_many_arguments)]
    fn blit_draw(
        &mut self,
        kind: u8,
        index: u8,
        targets: &str,
        depth: Option<&'static str>,
        view: u32,
        data: Vec<u8>,
        samples: u32,
        target: &Target,
        dst: [i32; 4],
    ) {
        let device = self.device;
        let (layout, pipeline_layout) = match self.gpu.kit.blit_layouts.get(&kind) {
            Some(&l) => l,
            None => {
                let sample = ["float", "sint", "uint", "depth"][usize::from(kind)];
                let sampler = if kind == 0 {
                    "filtering"
                } else {
                    "non-filtering"
                };
                let (layout, pipeline_layout) =
                    (self.gpu.id(&self.client), self.gpu.id(&self.client));
                self.client.queue([
                    Request::CreateBindGroupLayout {
                        device,
                        id: layout,
                        desc: format!(
                            r#"{{"entries":[{{"binding":0,"visibility":2,"buffer":{{"type":"uniform","hasDynamicOffset":true,"minBindingSize":48}}}},{{"binding":1,"visibility":2,"texture":{{"sampleType":"{sample}"}}}},{{"binding":2,"visibility":2,"sampler":{{"type":"{sampler}"}}}}]}}"#
                        ),
                    },
                    Request::CreatePipelineLayout {
                        device,
                        id: pipeline_layout,
                        layouts: vec![layout],
                    },
                ]);
                self.gpu
                    .kit
                    .blit_layouts
                    .insert(kind, (layout, pipeline_layout));
                (layout, pipeline_layout)
            }
        };
        let module = match self.gpu.kit.blit_modules.get(&kind) {
            Some(&m) => m,
            None => {
                let m = self.gpu.id(&self.client);
                self.client.queue([Request::CreateShaderModule {
                    device,
                    id: m,
                    code: blit_code(kind),
                }]);
                self.gpu.kit.blit_modules.insert(kind, m);
                m
            }
        };
        let _ = index;
        let depth_stencil = depth.map_or(String::new(), |format| {
            format!(r#","depthStencil":{{"format":"{format}","depthWriteEnabled":true,"depthCompare":"always"}}"#)
        });
        let fragment_targets = if depth.is_some() {
            String::new()
        } else {
            targets.to_owned()
        };
        let desc = format!(
            r#"{{"layout":{pipeline_layout},"vertex":{{"module":{module},"entryPoint":"vs","buffers":[]}},"fragment":{{"module":{module},"entryPoint":"fs","targets":[{fragment_targets}]}},"multisample":{{"count":{samples}}}{depth_stencil}}}"#
        );
        let pipeline = match self.gpu.kit.pipelines.get(&desc) {
            Some(&p) => p,
            None => {
                let id = self.gpu.id(&self.client);
                self.client.queue([Request::CreateRenderPipeline {
                    device,
                    id,
                    desc: desc.clone(),
                    call: None,
                }]);
                self.gpu.kit.pipelines.insert(desc, id);
                id
            }
        };
        let linear = data[40..44] != [0, 0, 0, 0];
        let sampler = self.sampler_of(format!(
            r#"{{"magFilter":"{0}","minFilter":"{0}"}}"#,
            if linear && kind == 0 {
                "linear"
            } else {
                "nearest"
            }
        ));
        let Some(offset) = self.arena_or_flush(&data, 256, target.size) else {
            return;
        };
        let key = (layout, vec![view, sampler]);
        let group = match self.gpu.groups.get(&key) {
            Some(&g) => g,
            None => {
                let g = self.gpu.id(&self.client);
                self.client.queue([Request::CreateBindGroup {
                    device,
                    id: g,
                    desc: format!(
                        r#"{{"layout":{layout},"entries":[{{"binding":0,"buffer":{},"offset":0,"size":48}},{{"binding":1,"view":{view}}},{{"binding":2,"sampler":{sampler}}}]}}"#,
                        self.gpu.arena_id()
                    ),
                }]);
                self.gpu.groups.insert(key, g);
                g
            }
        };
        // The destination rectangle, scissored as GL's state says.
        let (x0, x1) = (dst[0].min(dst[2]), dst[0].max(dst[2]));
        let (y0, y1) = (dst[1].min(dst[3]), dst[1].max(dst[3]));
        let s = self.scissor_rect(target.size);
        let clip = |lo: i32, hi: i32, at: u32, len: u32| {
            let a = lo.max(at as i32).max(0);
            let b = hi.min((at + len) as i32);
            (a as u32, (b - a).max(0) as u32)
        };
        let (sx, sw) = clip(x0, x1, s.0, s.2);
        let (sy, sh) = clip(y0, y1, s.1, s.3);
        let Some(pass) = self.gpu_pass() else {
            return;
        };
        if sw == 0 || sh == 0 {
            return;
        }
        pass.extend([
            RenderCommand::SetPipeline { pipeline },
            RenderCommand::SetBindGroup {
                index: 0,
                group: Some(group),
                offsets: vec![offset as u32],
            },
            RenderCommand::SetViewport {
                x: 0.0,
                y: 0.0,
                width: target.size.0 as f32,
                height: target.size.1 as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            },
            RenderCommand::SetScissorRect {
                x: sx,
                y: sy,
                width: sw,
                height: sh,
            },
            RenderCommand::Draw {
                vertices: 3,
                instances: 1,
                first_vertex: 0,
                first_instance: 0,
            },
        ]);
    }
}

/// A storage's component type (FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE).
fn component_type(storage: &str) -> u32 {
    if has_depth(storage) || has_stencil(storage) {
        return if storage.starts_with("depth32") {
            FLOAT
        } else {
            UNSIGNED_NORMALIZED_EXT
        };
    }
    match formats::info(storage).2 {
        Sample::Sint => INT,
        Sample::Uint => UNSIGNED_INT,
        _ if storage.ends_with("float") => FLOAT,
        _ if storage.contains("snorm") => SIGNED_NORMALIZED,
        _ => UNSIGNED_NORMALIZED_EXT,
    }
}

/// The red, green, blue, alpha, depth and stencil bits of an internal
/// format kept in `storage`.
fn bits_of(internal: u32, storage: &str) -> [u32; 6] {
    match internal {
        RGBA4 => return [4, 4, 4, 4, 0, 0],
        RGB5_A1 => return [5, 5, 5, 1, 0, 0],
        RGB565 => return [5, 6, 5, 0, 0, 0],
        RGB8 | SRGB8 => return [8, 8, 8, 0, 0, 0],
        RGB16F => return [16, 16, 16, 0, 0, 0],
        RGB32F => return [32, 32, 32, 0, 0, 0],
        DEPTH_COMPONENT16 => return [0, 0, 0, 0, 16, 0],
        DEPTH_COMPONENT24 => return [0, 0, 0, 0, 24, 0],
        DEPTH_COMPONENT32F => return [0, 0, 0, 0, 32, 0],
        DEPTH24_STENCIL8 | DEPTH_STENCIL => return [0, 0, 0, 0, 24, 8],
        DEPTH32F_STENCIL8 => return [0, 0, 0, 0, 32, 8],
        STENCIL_INDEX8 => return [0, 0, 0, 0, 0, 8],
        _ => {}
    }
    match storage {
        "rgb10a2unorm" | "rgb10a2uint" => [10, 10, 10, 2, 0, 0],
        "rg11b10ufloat" => [11, 11, 10, 0, 0, 0],
        "rgb9e5ufloat" => [9, 9, 9, 0, 0, 0],
        s => {
            let (channels, comp, _) = formats::info(s);
            let bits = match comp {
                formats::Comp::U8 | formats::Comp::S8 => 8,
                formats::Comp::U16 | formats::Comp::S16 | formats::Comp::F16 => 16,
                _ => 32,
            };
            let mut out = [0; 6];
            for slot in out.iter_mut().take(channels) {
                *slot = bits;
            }
            out
        }
    }
}

/// readPixels' own format and type for a read buffer's storage (its
/// IMPLEMENTATION_COLOR_READ_FORMAT and _TYPE).
fn own_format(version: u8, storage: &str) -> (u32, u32) {
    if version == 1 {
        return if storage.ends_with("float") {
            (RGBA, FLOAT)
        } else {
            (RGBA, UNSIGNED_BYTE)
        };
    }
    let (channels, comp, sample) = formats::info(storage);
    let integer = matches!(sample, Sample::Sint | Sample::Uint);
    let format = match (channels, integer) {
        (1, false) => RED,
        (2, false) => RG,
        (3, false) => RGB,
        (_, false) => RGBA,
        (1, true) => RED_INTEGER,
        (2, true) => RG_INTEGER,
        (3, true) => RGB_INTEGER,
        _ => RGBA_INTEGER,
    };
    let kind = match (storage, comp) {
        ("rgb10a2unorm" | "rgb10a2uint", _) => UNSIGNED_INT_2_10_10_10_REV,
        ("rg11b10ufloat", _) => UNSIGNED_INT_10F_11F_11F_REV,
        (_, formats::Comp::U8) => UNSIGNED_BYTE,
        (_, formats::Comp::S8) => BYTE,
        (_, formats::Comp::U16) => UNSIGNED_SHORT,
        (_, formats::Comp::S16) => SHORT,
        (_, formats::Comp::U32) => UNSIGNED_INT,
        (_, formats::Comp::S32) => INT,
        (_, formats::Comp::F16) => HALF_FLOAT,
        _ => FLOAT,
    };
    (format, kind)
}
