//! WebGL's textures, renderbuffers and framebuffers (M41 step 7, D40):
//! GL's images, each defined on its own, on WebGPU's textures of a fixed
//! shape. Each image (a face's level) is a texture of its own in the GPU
//! process, which a framebuffer draws into; a texture sampled past its
//! first level, or a cube, is put together from its images into one,
//! again when they change. Formats WebGPU lacks are widened on upload
//! (LUMINANCE, ALPHA and RGB as RGBA, packed shorts unpacked); copies
//! and mipmaps are drawn by a blit of this module's.

use std::collections::{BTreeMap, HashMap};

use js::{Answer, CanvasCall};

use super::gl::*;
use super::{Context, MAX_SIZE, i, n, u, wait};
use crate::gpu::wire::{Command, GPU_CHUNK, RenderCommand, Reply, Request};
use crate::ipc::Bytes;

/// The levels a texture may have: a side of [`MAX_SIZE`] halved to 1.
const MAX_LEVEL: i32 = 13;

/// A texture image's format: GL's format and type (a compressed or a
/// renderbuffer's internal format, its type 0), and WebGPU's storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Format {
    pub format: u32,
    pub kind: u32,
    pub storage: &'static str,
}

impl Format {
    fn depth(&self) -> bool {
        has_depth(self.storage) || has_stencil(self.storage)
    }

    fn compressed(&self) -> bool {
        self.storage.starts_with("bc")
    }

    /// Whether GL's format has no alpha (read as 1).
    fn opaque(&self) -> bool {
        matches!(
            self.format,
            RGB | SRGB_EXT | LUMINANCE | RGB565 | RGB16F_EXT | COMPRESSED_RGB_S3TC_DXT1_EXT
        )
    }
}

pub(crate) fn has_depth(storage: &str) -> bool {
    storage.starts_with("depth")
}

pub(crate) fn has_stencil(storage: &str) -> bool {
    storage.contains("stencil")
}

/// A storage's bytes a texel.
fn texel_bytes(storage: &str) -> u32 {
    match storage {
        "rgba16float" => 8,
        "rgba32float" => 16,
        "depth16unorm" => 2,
        "stencil8" => 1,
        _ => 4,
    }
}

/// A compressed format's bytes a block of 4 by 4.
fn block_bytes(format: u32) -> u32 {
    if matches!(
        format,
        COMPRESSED_RGB_S3TC_DXT1_EXT | COMPRESSED_RGBA_S3TC_DXT1_EXT
    ) {
        8
    } else {
        16
    }
}

fn round4(v: u32) -> u32 {
    v.div_ceil(4) * 4
}

/// A texture image: its size and format, and its texture and views in
/// the GPU process (none when empty): the one drawn into, and the one
/// sampled (a depth-and-stencil's depth).
#[derive(Clone, Debug)]
pub(crate) struct Image {
    pub width: u32,
    pub height: u32,
    pub format: Format,
    pub gpu: Option<(u32, u32, u32)>,
}

#[derive(Clone, Debug)]
pub(crate) struct Texture {
    pub target: Option<u32>,
    pub deleted: bool,
    pub min: u32,
    pub mag: u32,
    pub wrap_s: u32,
    pub wrap_t: u32,
    pub anisotropy: f32,
    /// Its images by face (0 for a 2D texture) and level.
    pub images: BTreeMap<(u8, u8), Image>,
    /// Counted up at every change to its images.
    pub generation: u64,
    /// Its images put together for sampling: texture, view, and what
    /// they were put together from.
    pub assembled: Option<(u32, u32, Assembly)>,
}

pub(crate) type Assembly = (u64, u8, u32, u32, &'static str);

impl Texture {
    fn new() -> Texture {
        Texture {
            target: None,
            deleted: false,
            min: NEAREST_MIPMAP_LINEAR,
            mag: LINEAR,
            wrap_s: REPEAT,
            wrap_t: REPEAT,
            anisotropy: 1.0,
            images: BTreeMap::new(),
            generation: 0,
            assembled: None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Renderbuffer {
    pub deleted: bool,
    pub bound: bool,
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub storage: Option<&'static str>,
    pub gpu: Option<(u32, u32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Attached {
    Texture { id: u32, face: u8, level: u8 },
    Renderbuffer(u32),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Framebuffer {
    pub deleted: bool,
    pub bound: bool,
    pub attachments: BTreeMap<u32, Attached>,
}

/// A context's textures, renderbuffers and framebuffers, and their
/// bindings.
#[derive(Debug)]
pub(crate) struct Surfaces {
    pub textures: HashMap<u32, Texture>,
    pub renderbuffers: HashMap<u32, Renderbuffer>,
    pub framebuffers: HashMap<u32, Framebuffer>,
    /// Each texture unit's 2D and cube texture.
    pub units: Vec<[Option<u32>; 2]>,
    pub renderbuffer: Option<u32>,
    pub framebuffer: Option<u32>,
    pub derivative_hint: u32,
}

impl Default for Surfaces {
    fn default() -> Surfaces {
        Surfaces {
            textures: HashMap::new(),
            renderbuffers: HashMap::new(),
            framebuffers: HashMap::new(),
            units: vec![[None; 2]; 32],
            renderbuffer: None,
            framebuffer: None,
            derivative_hint: DONT_CARE,
        }
    }
}

/// What a draw or a clear draws into: the drawing buffer or a
/// framebuffer's attachments.
#[derive(Clone, Debug)]
pub(crate) struct Target {
    /// Its views, which say whether a pass is still of it.
    pub key: (u32, u32),
    pub size: (u32, u32),
    /// The colour view, the one it resolves to, and the storage.
    pub color: Option<(u32, Option<u32>, &'static str)>,
    /// Whether the colour has no alpha: its writes masked, read as 1.
    pub rgb: bool,
    pub depth: Option<(u32, &'static str)>,
    pub depth_on: bool,
    pub stencil_on: bool,
    pub samples: u32,
    pub default: bool,
    /// The textures drawn into.
    pub textures: Vec<u32>,
}

/// An attachment's view, storage, size and whether it may be a colour
/// attachment; a texture's id when it is one.
struct Info {
    view: Option<u32>,
    texture: Option<u32>,
    gpu_texture: Option<u32>,
    sample_view: Option<u32>,
    storage: &'static str,
    size: (u32, u32),
    color_ok: bool,
    opaque: bool,
}

/// What `readPixels` and the copies read: a texture, its view for a
/// blit, size, storage, whether alpha is read as 1, and the texture
/// image it is when it is one.
struct Source {
    texture: u32,
    view: u32,
    size: (u32, u32),
    storage: &'static str,
    opaque: bool,
    image: Option<(u32, u8, u8)>,
}

/// The blit's module, layouts and pipelines (by the storage drawn into).
#[derive(Debug)]
pub(crate) struct Blit {
    module: u32,
    layout: u32,
    pipeline_layout: u32,
    pipelines: HashMap<&'static str, u32>,
}

const BLIT: &str = "struct B { src: vec2<i32>, dst: vec2<i32>, size: vec2<i32>, mode: i32, pad: i32 }
@group(0) @binding(0) var<uniform> b: B;
@group(0) @binding(1) var t: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var p = array(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(p[i], 0.0, 1.0);
}
@fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let d = vec2<i32>(floor(p.xy)) - b.dst;
    if (b.mode == 5) {
        let s = d * 2;
        let m = b.size - vec2<i32>(1, 1);
        let c = textureLoad(t, min(s, m), 0) + textureLoad(t, min(s + vec2<i32>(1, 0), m), 0)
            + textureLoad(t, min(s + vec2<i32>(0, 1), m), 0) + textureLoad(t, min(s + vec2<i32>(1, 1), m), 0);
        return c * 0.25;
    }
    let s = d + b.src;
    if (any(s < vec2<i32>(0, 0)) || any(s >= b.size)) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let c = textureLoad(t, s, 0);
    switch b.mode {
        case 1: { return vec4<f32>(c.rgb, 1.0); }
        case 2: { return vec4<f32>(c.rrr, 1.0); }
        case 3: { return vec4<f32>(0.0, 0.0, 0.0, c.a); }
        case 4: { return vec4<f32>(c.rrr, c.a); }
        default: { return c; }
    }
}
";

/// GL's target of a texture image: the binding (0 2D, 1 cube) and face.
fn image_target(target: u32) -> Option<(usize, u8)> {
    match target {
        TEXTURE_2D => Some((0, 0)),
        TEXTURE_CUBE_MAP_POSITIVE_X..=TEXTURE_CUBE_MAP_NEGATIVE_Z => {
            Some((1, (target - TEXTURE_CUBE_MAP_POSITIVE_X) as u8))
        }
        _ => None,
    }
}

fn pot(v: u32) -> bool {
    v.is_power_of_two()
}

/// A format's channels as the client gives them.
fn channels(format: u32) -> u32 {
    match format {
        ALPHA | LUMINANCE | DEPTH_COMPONENT | DEPTH_STENCIL => 1,
        LUMINANCE_ALPHA => 2,
        RGB | SRGB_EXT => 3,
        _ => 4,
    }
}

/// The client's bytes a pixel of `format` and `kind`.
fn client_pixel(format: u32, kind: u32) -> u32 {
    match kind {
        UNSIGNED_SHORT_5_6_5 | UNSIGNED_SHORT_4_4_4_4 | UNSIGNED_SHORT_5_5_5_1 => 2,
        UNSIGNED_BYTE => channels(format),
        HALF_FLOAT | HALF_FLOAT_OES | UNSIGNED_SHORT => 2 * channels(format),
        _ => 4 * channels(format),
    }
}

/// The JavaScript array a type's data comes in, by the bootstrap's code:
/// 1 Uint8Array, 2 Uint16Array, 3 Float32Array, 4 Uint32Array, 6
/// Uint8ClampedArray.
fn view_ok(kind: u32, view: u32, clamped_too: bool) -> bool {
    match kind {
        UNSIGNED_BYTE => view == 1 || (clamped_too && view == 6),
        FLOAT => view == 3,
        UNSIGNED_INT | UNSIGNED_INT_24_8_WEBGL => view == 4,
        _ => view == 2,
    }
}

/// The bytes rows of `w` pixels of `pixel` bytes take, `h` of them, at
/// an alignment: the row's stride and the whole.
fn rows(w: u32, h: u32, pixel: u32, align: u32) -> (u64, u64) {
    let row = u64::from(w) * u64::from(pixel);
    let stride = row.div_ceil(u64::from(align)) * u64::from(align);
    let whole = if h == 0 || w == 0 {
        0
    } else {
        stride * u64::from(h - 1) + row
    };
    (stride, whole)
}

pub(crate) fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((h >> 10) & 0x1F);
    let mantissa = f32::from(h & 0x3FF);
    match exponent {
        0 => sign * mantissa * 2f32.powi(-24),
        31 if mantissa == 0.0 => sign * f32::INFINITY,
        31 => f32::NAN,
        e => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(e - 15),
    }
}

pub(crate) fn f32_to_f16(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    if v.is_nan() {
        return 0x7E00;
    }
    let a = v.abs();
    if a >= 65520.0 {
        return sign | 0x7C00;
    }
    if a < 6.103_515_6e-5 {
        // Subnormal: a multiple of 2^-24.
        return sign | (a / 2f32.powi(-24)).round() as u16;
    }
    let exponent = ((bits >> 23) & 0xFF) as i32 - 127 + 15;
    let mantissa = bits & 0x7F_FFFF;
    let mut h = ((exponent as u32) << 10) | (mantissa >> 13);
    // Round to nearest, ties to even.
    let rest = mantissa & 0x1FFF;
    if rest > 0x1000 || (rest == 0x1000 && h & 1 == 1) {
        h += 1;
    }
    sign | h as u16
}

/// A format's channels put in RGBA's places: the missing colours 0, or
/// the luminance's; a missing alpha `one`.
fn expand<T: Copy>(format: u32, c: &[T], zero: T, one: T) -> [T; 4] {
    match format {
        ALPHA => [zero, zero, zero, c[0]],
        LUMINANCE => [c[0], c[0], c[0], one],
        LUMINANCE_ALPHA => [c[0], c[0], c[0], c[1]],
        RGB | SRGB_EXT => [c[0], c[1], c[2], one],
        _ => [c[0], c[1], c[2], c[3]],
    }
}

fn unpack565(v: u16) -> [u8; 4] {
    let r = ((v >> 11) & 31) as u8;
    let g = ((v >> 5) & 63) as u8;
    let b = (v & 31) as u8;
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
        255,
    ]
}

fn unpack4444(v: u16) -> [u8; 4] {
    let c = |s: u16| ((v >> s) & 15) as u8 * 17;
    [c(12), c(8), c(4), c(0)]
}

fn unpack5551(v: u16) -> [u8; 4] {
    let c = |s: u16| {
        let x = ((v >> s) & 31) as u8;
        (x << 3) | (x >> 2)
    };
    [c(11), c(6), c(1), if v & 1 == 1 { 255 } else { 0 }]
}

/// Client rows of `fmt` (at `align`) as the storage's texels, the rows
/// turned over and the colours multiplied by alpha as asked.
fn texels(
    fmt: Format,
    w: u32,
    h: u32,
    data: &[u8],
    align: u32,
    flip: bool,
    premultiply: bool,
) -> Vec<u8> {
    let pixel = client_pixel(fmt.format, fmt.kind) as usize;
    let (stride, _) = rows(w, h, pixel as u32, align);
    let texel = texel_bytes(fmt.storage) as usize;
    let mut out = vec![0u8; w as usize * h as usize * texel];
    let alpha = !fmt.opaque() && fmt.format != LUMINANCE;
    let n = channels(fmt.format) as usize;
    for y in 0..h as usize {
        let source_row = if flip { h as usize - 1 - y } else { y };
        let row = &data[source_row * stride as usize..];
        for x in 0..w as usize {
            let p = &row[x * pixel..x * pixel + pixel];
            let at = (y * w as usize + x) * texel;
            match fmt.kind {
                UNSIGNED_BYTE
                | UNSIGNED_SHORT_5_6_5
                | UNSIGNED_SHORT_4_4_4_4
                | UNSIGNED_SHORT_5_5_5_1 => {
                    let mut c = match fmt.kind {
                        UNSIGNED_BYTE => expand(fmt.format, &p[..n], 0, 255),
                        UNSIGNED_SHORT_5_6_5 => unpack565(u16::from_le_bytes([p[0], p[1]])),
                        UNSIGNED_SHORT_4_4_4_4 => unpack4444(u16::from_le_bytes([p[0], p[1]])),
                        _ => unpack5551(u16::from_le_bytes([p[0], p[1]])),
                    };
                    if premultiply && alpha {
                        let a = u32::from(c[3]);
                        for v in &mut c[..3] {
                            *v = ((u32::from(*v) * a + 127) / 255) as u8;
                        }
                    }
                    out[at..at + 4].copy_from_slice(&c);
                }
                FLOAT => {
                    let v: Vec<f32> = p
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|q| f32::from_le_bytes(*q))
                        .collect();
                    let mut c = expand(fmt.format, &v, 0.0, 1.0);
                    if premultiply && alpha {
                        for k in 0..3 {
                            c[k] *= c[3];
                        }
                    }
                    for (k, v) in c.iter().enumerate() {
                        out[at + 4 * k..at + 4 * k + 4].copy_from_slice(&v.to_le_bytes());
                    }
                }
                _ => {
                    // Half floats.
                    let v: Vec<u16> = p
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|q| u16::from_le_bytes(*q))
                        .collect();
                    let mut c = expand(fmt.format, &v, 0, 0x3C00);
                    if premultiply && alpha {
                        let a = f16_to_f32(c[3]);
                        for v in &mut c[..3] {
                            *v = f32_to_f16(f16_to_f32(*v) * a);
                        }
                    }
                    for (k, v) in c.iter().enumerate() {
                        out[at + 2 * k..at + 2 * k + 2].copy_from_slice(&v.to_le_bytes());
                    }
                }
            }
        }
    }
    out
}

/// An image source's pixels (RGBA, already turned over and multiplied by
/// alpha as asked) as client data of `fmt`, then its texels.
fn from_rgba8(fmt: Format, w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let quantize = |c: u8, bits: u32| u16::from(c) >> (8 - bits);
    let mut client = Vec::new();
    for p in rgba.as_chunks::<4>().0.iter().take(w as usize * h as usize) {
        let picked: Vec<u8> = match fmt.format {
            ALPHA => vec![p[3]],
            LUMINANCE => vec![p[0]],
            LUMINANCE_ALPHA => vec![p[0], p[3]],
            RGB | SRGB_EXT => vec![p[0], p[1], p[2]],
            _ => p.to_vec(),
        };
        match fmt.kind {
            UNSIGNED_BYTE => client.extend(picked),
            UNSIGNED_SHORT_5_6_5 => {
                let v = (quantize(p[0], 5) << 11) | (quantize(p[1], 6) << 5) | quantize(p[2], 5);
                client.extend(v.to_le_bytes());
            }
            UNSIGNED_SHORT_4_4_4_4 => {
                let v = (quantize(p[0], 4) << 12)
                    | (quantize(p[1], 4) << 8)
                    | (quantize(p[2], 4) << 4)
                    | quantize(p[3], 4);
                client.extend(v.to_le_bytes());
            }
            UNSIGNED_SHORT_5_5_5_1 => {
                let v = (quantize(p[0], 5) << 11)
                    | (quantize(p[1], 5) << 6)
                    | (quantize(p[2], 5) << 1)
                    | quantize(p[3], 1);
                client.extend(v.to_le_bytes());
            }
            FLOAT => {
                for c in picked {
                    client.extend((f32::from(c) / 255.0).to_le_bytes());
                }
            }
            _ => {
                for c in picked {
                    client.extend(f32_to_f16(f32::from(c) / 255.0).to_le_bytes());
                }
            }
        }
    }
    texels(fmt, w, h, &client, 1, false, false)
}

impl Context {
    /// The calls of textures, renderbuffers, framebuffers, `readPixels`
    /// and the extensions; none for another call.
    pub(super) fn texture_op(
        &mut self,
        op: &str,
        call: &CanvasCall,
        size: (u32, u32),
    ) -> Option<Answer> {
        let answer = match op {
            "createTexture" => {
                let id = self.id();
                self.surfaces.textures.insert(id, Texture::new());
                Answer::Number(f64::from(id))
            }
            "bindTexture" => {
                self.bind_texture(u(call, 1), u(call, 2));
                Answer::Nothing
            }
            "deleteTexture" => {
                self.delete_texture(u(call, 1));
                Answer::Nothing
            }
            "isTexture" => Answer::Bool(
                self.surfaces
                    .textures
                    .get(&u(call, 1))
                    .is_some_and(|t| !t.deleted && t.target.is_some()),
            ),
            "texParameteri" | "texParameterf" => {
                self.tex_parameter(u(call, 1), u(call, 2), n(call, 3));
                Answer::Nothing
            }
            "getTexParameter" => self.get_tex_parameter(u(call, 1), u(call, 2)),
            "texImage2D" => {
                self.tex_image(call);
                Answer::Nothing
            }
            "texSubImage2D" => {
                self.tex_sub_image(call);
                Answer::Nothing
            }
            "compressedTexImage2D" => {
                self.compressed_tex_image(call);
                Answer::Nothing
            }
            "compressedTexSubImage2D" => {
                self.compressed_tex_sub_image(call);
                Answer::Nothing
            }
            "copyTexImage2D" | "copyTexSubImage2D" => {
                self.copy_tex_image(call, op == "copyTexSubImage2D", size);
                Answer::Nothing
            }
            "generateMipmap" => {
                self.generate_mipmap(u(call, 1));
                Answer::Nothing
            }
            "createRenderbuffer" => {
                let id = self.id();
                self.surfaces.renderbuffers.insert(
                    id,
                    Renderbuffer {
                        deleted: false,
                        bound: false,
                        format: RGBA4,
                        width: 0,
                        height: 0,
                        storage: None,
                        gpu: None,
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
                self.renderbuffer_storage(u(call, 1), u(call, 2), i(call, 3), i(call, 4));
                Answer::Nothing
            }
            "getRenderbufferParameter" => self.renderbuffer_parameter(u(call, 1), u(call, 2)),
            "createFramebuffer" => {
                let id = self.id();
                self.surfaces
                    .framebuffers
                    .insert(id, Framebuffer::default());
                Answer::Number(f64::from(id))
            }
            "bindFramebuffer" => {
                let (target, id) = (u(call, 1), u(call, 2));
                if target != FRAMEBUFFER {
                    self.error(INVALID_ENUM);
                } else if id == 0 {
                    self.surfaces.framebuffer = None;
                } else {
                    match self.surfaces.framebuffers.get_mut(&id) {
                        Some(fb) if !fb.deleted => {
                            fb.bound = true;
                            self.surfaces.framebuffer = Some(id);
                        }
                        _ => self.error(INVALID_OPERATION),
                    }
                }
                Answer::Nothing
            }
            "deleteFramebuffer" => {
                let id = u(call, 1);
                if self.surfaces.framebuffers.contains_key(&id) {
                    if self.surfaces.framebuffer == Some(id) {
                        self.surfaces.framebuffer = None;
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
                self.framebuffer_texture(
                    u(call, 1),
                    u(call, 2),
                    u(call, 3),
                    u(call, 4),
                    i(call, 5),
                );
                Answer::Nothing
            }
            "framebufferRenderbuffer" => {
                self.framebuffer_renderbuffer(u(call, 1), u(call, 2), u(call, 3), u(call, 4));
                Answer::Nothing
            }
            "checkFramebufferStatus" => {
                if u(call, 1) != FRAMEBUFFER {
                    self.error(INVALID_ENUM);
                    Answer::Number(0.0)
                } else {
                    let status = match self.surfaces.framebuffer {
                        Some(fb) => self.status(fb),
                        None => FRAMEBUFFER_COMPLETE,
                    };
                    Answer::Number(f64::from(status))
                }
            }
            "getFramebufferAttachmentParameter" => {
                self.attachment_parameter(u(call, 1), u(call, 2), u(call, 3))
            }
            "readPixels" => self.read_pixels(call, size),
            "getSupportedExtensions" => Answer::Strings(
                super::ext::offered(self.version, &self.features)
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            ),
            "getExtension" => {
                let name = call.strings.first().cloned().unwrap_or_default();
                let offered = super::ext::offered(self.version, &self.features);
                let found = self.ext.enable(&offered, &name);
                if found == Some("EXT_blend_minmax") {
                    self.gpu.ext_blend_minmax = true;
                }
                Answer::MaybeString(found.map(str::to_owned))
            }
            "vertexAttribDivisor" => {
                let index = u(call, 1) as usize;
                match self.attribs.get_mut(index) {
                    Some(a) => a.divisor = u(call, 2),
                    None => self.error(INVALID_VALUE),
                }
                Answer::Nothing
            }
            _ => return None,
        };
        Some(answer)
    }

    // ---- Textures ----

    fn unit(&self) -> usize {
        (self.state.active_texture - TEXTURE0) as usize
    }

    /// The texture bound to `slot` (0 2D, 1 cube) of the active unit.
    fn bound_texture(&self, slot: usize) -> Option<u32> {
        self.surfaces.units[self.unit()][slot]
    }

    fn bind_texture(&mut self, target: u32, id: u32) {
        let slot = match target {
            TEXTURE_2D => 0,
            TEXTURE_CUBE_MAP => 1,
            _ => {
                self.error(INVALID_ENUM);
                return;
            }
        };
        if id != 0 {
            match self.surfaces.textures.get_mut(&id) {
                Some(t) if !t.deleted => {
                    if t.target.is_some_and(|first| first != target) {
                        self.error(INVALID_OPERATION);
                        return;
                    }
                    t.target = Some(target);
                }
                _ => {
                    self.error(INVALID_OPERATION);
                    return;
                }
            }
        }
        let unit = self.unit();
        self.surfaces.units[unit][slot] = (id != 0).then_some(id);
    }

    fn delete_texture(&mut self, id: u32) {
        let Some(t) = self.surfaces.textures.get_mut(&id) else {
            return;
        };
        if t.deleted {
            return;
        }
        t.deleted = true;
        for unit in &mut self.surfaces.units {
            for slot in unit.iter_mut() {
                if *slot == Some(id) {
                    *slot = None;
                }
            }
        }
        if let Some(fb) = self.surfaces.framebuffer
            && let Some(fb) = self.surfaces.framebuffers.get_mut(&fb)
        {
            fb.attachments
                .retain(|_, a| !matches!(a, Attached::Texture { id: t, .. } if *t == id));
        }
        self.sweep();
    }

    /// Detaches `what` from the bound framebuffer (GL deletes do).
    fn detach(&mut self, what: Attached) {
        if let Some(fb) = self.surfaces.framebuffer
            && let Some(fb) = self.surfaces.framebuffers.get_mut(&fb)
        {
            fb.attachments.retain(|_, a| *a != what);
        }
    }

    /// Deleted textures and renderbuffers no framebuffer holds: gone, in
    /// the GPU process too.
    fn sweep(&mut self) {
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
                    if let Some((a, b, c)) = image.gpu {
                        gone.extend([a, b, c]);
                    }
                }
                if let Some((a, b, _)) = t.assembled {
                    gone.extend([a, b]);
                }
            }
        }
        for id in renderbuffers {
            if let Some(r) = self.surfaces.renderbuffers.remove(&id)
                && let Some((a, b)) = r.gpu
            {
                gone.extend([a, b]);
            }
        }
        self.free(gone);
    }

    /// GPU objects gone: what was recorded with them sent first.
    fn free(&mut self, mut ids: Vec<u32>) {
        ids.sort_unstable();
        ids.dedup();
        if ids.is_empty() {
            return;
        }
        self.gpu_flush();
        self.gpu.forget(&self.client, &ids);
    }

    fn tex_parameter(&mut self, target: u32, name: u32, value: f64) {
        let slot = match target {
            TEXTURE_2D => 0,
            TEXTURE_CUBE_MAP => 1,
            _ => {
                self.error(INVALID_ENUM);
                return;
            }
        };
        let Some(id) = self.bound_texture(slot) else {
            self.error(INVALID_OPERATION);
            return;
        };
        let anisotropic = self.ext.on("EXT_texture_filter_anisotropic");
        let v = value as u32;
        let t = self.surfaces.textures.get_mut(&id).expect("bound");
        let ok = match name {
            TEXTURE_MIN_FILTER => {
                let ok = matches!(
                    v,
                    NEAREST
                        | LINEAR
                        | NEAREST_MIPMAP_NEAREST
                        | LINEAR_MIPMAP_NEAREST
                        | NEAREST_MIPMAP_LINEAR
                        | LINEAR_MIPMAP_LINEAR
                );
                if ok {
                    t.min = v;
                }
                ok
            }
            TEXTURE_MAG_FILTER => {
                let ok = matches!(v, NEAREST | LINEAR);
                if ok {
                    t.mag = v;
                }
                ok
            }
            TEXTURE_WRAP_S | TEXTURE_WRAP_T => {
                let ok = matches!(v, REPEAT | CLAMP_TO_EDGE | MIRRORED_REPEAT);
                if ok {
                    if name == TEXTURE_WRAP_S {
                        t.wrap_s = v;
                    } else {
                        t.wrap_t = v;
                    }
                }
                ok
            }
            TEXTURE_MAX_ANISOTROPY_EXT if anisotropic => {
                if value < 1.0 || value.is_nan() {
                    self.error(INVALID_VALUE);
                    return;
                }
                t.anisotropy = value as f32;
                true
            }
            _ => false,
        };
        if !ok {
            self.error(INVALID_ENUM);
        }
    }

    fn get_tex_parameter(&mut self, target: u32, name: u32) -> Answer {
        let slot = match target {
            TEXTURE_2D => 0,
            TEXTURE_CUBE_MAP => 1,
            _ => {
                self.error(INVALID_ENUM);
                return Answer::Nothing;
            }
        };
        let Some(id) = self.bound_texture(slot) else {
            self.error(INVALID_OPERATION);
            return Answer::Nothing;
        };
        let anisotropic = self.ext.on("EXT_texture_filter_anisotropic");
        let t = &self.surfaces.textures[&id];
        let v = match name {
            TEXTURE_MIN_FILTER => t.min,
            TEXTURE_MAG_FILTER => t.mag,
            TEXTURE_WRAP_S => t.wrap_s,
            TEXTURE_WRAP_T => t.wrap_t,
            TEXTURE_MAX_ANISOTROPY_EXT if anisotropic => {
                return Answer::Number(f64::from(t.anisotropy));
            }
            _ => {
                self.error(INVALID_ENUM);
                return Answer::Nothing;
            }
        };
        Answer::Number(f64::from(v))
    }

    /// GL's format and type as a texture's: the format or GL's error.
    fn texture_format(&self, internal: u32, format: u32, kind: u32) -> Result<Format, u32> {
        let v1 = self.version == 1;
        let ext = |name: &str| !v1 || self.ext.on(name);
        let format_ok = match format {
            ALPHA | RGB | RGBA | LUMINANCE | LUMINANCE_ALPHA => true,
            SRGB_EXT | SRGB_ALPHA_EXT => v1 && self.ext.on("EXT_sRGB"),
            DEPTH_COMPONENT | DEPTH_STENCIL => v1 && self.ext.on("WEBGL_depth_texture"),
            _ => false,
        };
        let kind_ok = match kind {
            UNSIGNED_BYTE
            | UNSIGNED_SHORT_5_6_5
            | UNSIGNED_SHORT_4_4_4_4
            | UNSIGNED_SHORT_5_5_5_1 => true,
            FLOAT => ext("OES_texture_float"),
            HALF_FLOAT_OES => v1 && self.ext.on("OES_texture_half_float"),
            HALF_FLOAT => !v1,
            UNSIGNED_SHORT | UNSIGNED_INT | UNSIGNED_INT_24_8_WEBGL => {
                v1 && self.ext.on("WEBGL_depth_texture")
            }
            _ => false,
        };
        if !format_ok || !kind_ok {
            return Err(INVALID_ENUM);
        }
        if internal != format {
            return Err(INVALID_OPERATION);
        }
        let storage = match (kind, format) {
            (UNSIGNED_BYTE, SRGB_EXT | SRGB_ALPHA_EXT) => "rgba8unorm-srgb",
            (UNSIGNED_BYTE, ALPHA | RGB | RGBA | LUMINANCE | LUMINANCE_ALPHA) => "rgba8unorm",
            (UNSIGNED_SHORT_5_6_5, RGB)
            | (UNSIGNED_SHORT_4_4_4_4 | UNSIGNED_SHORT_5_5_5_1, RGBA) => "rgba8unorm",
            (FLOAT, ALPHA | RGB | RGBA | LUMINANCE | LUMINANCE_ALPHA) => "rgba32float",
            (HALF_FLOAT | HALF_FLOAT_OES, ALPHA | RGB | RGBA | LUMINANCE | LUMINANCE_ALPHA) => {
                "rgba16float"
            }
            (UNSIGNED_SHORT, DEPTH_COMPONENT) => "depth16unorm",
            (UNSIGNED_INT, DEPTH_COMPONENT) => "depth32float",
            (UNSIGNED_INT_24_8_WEBGL, DEPTH_STENCIL) => "depth24plus-stencil8",
            _ => return Err(INVALID_OPERATION),
        };
        Ok(Format {
            format,
            kind,
            storage,
        })
    }

    /// A new texture in the GPU process: texture, view drawn into, view
    /// sampled.
    fn make_texture(&mut self, size: (u32, u32), storage: &str) -> (u32, u32, u32) {
        let (texture, view) = (self.gpu.id(&self.client), self.gpu.id(&self.client));
        // COPY_SRC | COPY_DST | TEXTURE_BINDING | RENDER_ATTACHMENT; a
        // depth one is drawn into and sampled alone, a compressed one
        // copied and sampled.
        let usage = if has_depth(storage) || has_stencil(storage) {
            20
        } else if storage.starts_with("bc") {
            7
        } else {
            23
        };
        self.client.queue([
            Request::CreateTexture {
                device: self.device,
                id: texture,
                desc: format!(
                    r#"{{"size":[{},{},1],"format":"{storage}","usage":{usage}}}"#,
                    size.0, size.1
                ),
            },
            Request::CreateView {
                texture,
                id: view,
                desc: "{}".to_owned(),
            },
        ]);
        let sample = if storage == "depth24plus-stencil8" {
            let id = self.gpu.id(&self.client);
            self.client.queue([Request::CreateView {
                texture,
                id,
                desc: r#"{"aspect":"depth-only"}"#.to_owned(),
            }]);
            id
        } else {
            view
        };
        (texture, view, sample)
    }

    /// `bytes` of rows `row` bytes long written into `texture` at
    /// `origin`, `size` texels (`block` rows of texels a row of bytes).
    fn upload(
        &mut self,
        texture: u32,
        origin: (u32, u32),
        size: (u32, u32),
        bytes: &[u8],
        row: u32,
        block: u32,
    ) {
        if bytes.is_empty() || row == 0 {
            return;
        }
        let total_rows = bytes.len() as u32 / row;
        let per = (GPU_CHUNK as u32 / row).max(1);
        let mut first = 0;
        while first < total_rows {
            let count = per.min(total_rows - first);
            let piece = bytes[(first * row) as usize..((first + count) * row) as usize].to_vec();
            let height = count * block;
            self.client.queue([Request::WriteTexture {
                device: self.device,
                desc: format!(
                    r#"{{"destination":{{"texture":{texture},"origin":[{},{},0]}},"dataLayout":{{"bytesPerRow":{row},"rowsPerImage":{count}}},"size":[{},{height},1]}}"#,
                    origin.0,
                    origin.1 + first * block,
                    size.0
                ),
                bytes: Bytes(piece),
            }]);
            first += count;
        }
    }

    /// The image of `(face, level)` of texture `id` set to `image` with
    /// its texels: the old one gone.
    fn set_image(
        &mut self,
        id: u32,
        key: (u8, u8),
        mut image: Image,
        texels: Option<(Vec<u8>, u32, u32)>,
    ) {
        // What was recorded reads the image as it was.
        self.gpu_flush();
        let old = self
            .surfaces
            .textures
            .get(&id)
            .and_then(|t| t.images.get(&key))
            .and_then(|i| i.gpu);
        if let Some((a, b, c)) = old {
            self.free(vec![a, b, c]);
        }
        if image.width > 0 && image.height > 0 {
            let physical = if image.format.compressed() {
                (round4(image.width), round4(image.height))
            } else {
                (image.width, image.height)
            };
            let made = self.make_texture(physical, image.format.storage);
            if let Some((bytes, row, block)) = texels {
                self.upload(made.0, (0, 0), physical, &bytes, row, block);
            }
            image.gpu = Some(made);
        }
        let t = self.surfaces.textures.get_mut(&id).expect("bound");
        t.images.insert(key, image);
        t.generation += 1;
    }

    fn check_level(&mut self, slot: usize, level: i32, w: i32, h: i32) -> bool {
        if !(0..=MAX_LEVEL).contains(&level) || w < 0 || h < 0 {
            self.error(INVALID_VALUE);
            return false;
        }
        let most = (MAX_SIZE >> level) as i32;
        if w > most || h > most || (slot == 1 && w != h) {
            self.error(INVALID_VALUE);
            return false;
        }
        true
    }

    /// `texImage2D`: [context, target, level, internal format, width,
    /// height, border, format, type, source, array kind]: source 0 an
    /// array's bytes, 1 none, 2 an image source's RGBA.
    fn tex_image(&mut self, call: &CanvasCall) {
        let (target, level, internal) = (u(call, 1), i(call, 2), u(call, 3));
        let (w, h, border) = (i(call, 4), i(call, 5), i(call, 6));
        let (format, kind, source, view) = (u(call, 7), u(call, 8), u(call, 9), u(call, 10));
        let Some((slot, face)) = image_target(target) else {
            self.error(INVALID_ENUM);
            return;
        };
        if !self.check_level(slot, level, w, h) {
            return;
        }
        if border != 0 {
            self.error(INVALID_VALUE);
            return;
        }
        let fmt = match self.texture_format(internal, format, kind) {
            Ok(fmt) => fmt,
            Err(e) => {
                self.error(e);
                return;
            }
        };
        let (w, h) = (w as u32, h as u32);
        if self.version == 1 && level > 0 && (!pot(w) || !pot(h)) {
            self.error(INVALID_VALUE);
            return;
        }
        let Some(id) = self.bound_texture(slot) else {
            self.error(INVALID_OPERATION);
            return;
        };
        if fmt.depth() && (target != TEXTURE_2D || level != 0 || source != 1) {
            self.error(INVALID_OPERATION);
            return;
        }
        let data = call.bytes.clone().unwrap_or_default();
        let bytes = match source {
            0 => {
                if !view_ok(kind, view, true) {
                    self.error(INVALID_OPERATION);
                    return;
                }
                let (_, whole) = rows(
                    w,
                    h,
                    client_pixel(format, kind),
                    self.state.unpack_alignment,
                );
                if (data.len() as u64) < whole {
                    self.error(INVALID_OPERATION);
                    return;
                }
                Some(texels(
                    fmt,
                    w,
                    h,
                    &data,
                    self.state.unpack_alignment,
                    self.state.unpack_flip_y,
                    self.state.unpack_premultiply_alpha,
                ))
            }
            2 => Some(from_rgba8(fmt, w, h, &data)),
            // No data: zeros, but a format without alpha keeps alpha 1.
            _ if fmt.opaque() && !fmt.depth() => {
                let zeros = vec![0; (w * h * client_pixel(format, kind)) as usize];
                Some(texels(fmt, w, h, &zeros, 1, false, false))
            }
            _ => None,
        };
        let row = w * texel_bytes(fmt.storage);
        let image = Image {
            width: w,
            height: h,
            format: fmt,
            gpu: None,
        };
        self.set_image(id, (face, level as u8), image, bytes.map(|b| (b, row, 1)));
    }

    /// `texSubImage2D`: [context, target, level, x, y, width, height,
    /// format, type, source, array kind].
    fn tex_sub_image(&mut self, call: &CanvasCall) {
        let (target, level, x, y) = (u(call, 1), i(call, 2), i(call, 3), i(call, 4));
        let (w, h, format, kind) = (i(call, 5), i(call, 6), u(call, 7), u(call, 8));
        let (source, view) = (u(call, 9), u(call, 10));
        let Some((slot, face)) = image_target(target) else {
            self.error(INVALID_ENUM);
            return;
        };
        if !(0..=MAX_LEVEL).contains(&level) {
            self.error(INVALID_VALUE);
            return;
        }
        if let Err(e) = self.texture_format(format, format, kind) {
            self.error(e);
            return;
        }
        let Some(id) = self.bound_texture(slot) else {
            self.error(INVALID_OPERATION);
            return;
        };
        let Some(image) = self.surfaces.textures[&id]
            .images
            .get(&(face, level as u8))
            .cloned()
        else {
            self.error(INVALID_OPERATION);
            return;
        };
        if x < 0
            || y < 0
            || w < 0
            || h < 0
            || (x + w) as u32 > image.width
            || (y + h) as u32 > image.height
        {
            self.error(INVALID_VALUE);
            return;
        }
        if image.format.format != format
            || image.format.kind != kind
            || image.format.depth()
            || image.format.compressed()
        {
            self.error(INVALID_OPERATION);
            return;
        }
        if source == 1 {
            self.error(INVALID_VALUE);
            return;
        }
        let (w, h) = (w as u32, h as u32);
        let data = call.bytes.clone().unwrap_or_default();
        let bytes = if source == 0 {
            if !view_ok(kind, view, true) {
                self.error(INVALID_OPERATION);
                return;
            }
            let (_, whole) = rows(
                w,
                h,
                client_pixel(format, kind),
                self.state.unpack_alignment,
            );
            if (data.len() as u64) < whole {
                self.error(INVALID_OPERATION);
                return;
            }
            texels(
                image.format,
                w,
                h,
                &data,
                self.state.unpack_alignment,
                self.state.unpack_flip_y,
                self.state.unpack_premultiply_alpha,
            )
        } else {
            from_rgba8(image.format, w, h, &data)
        };
        if w == 0 || h == 0 {
            return;
        }
        let Some((texture, _, _)) = image.gpu else {
            return;
        };
        self.gpu_flush();
        let row = w * texel_bytes(image.format.storage);
        self.upload(texture, (x as u32, y as u32), (w, h), &bytes, row, 1);
        self.surfaces
            .textures
            .get_mut(&id)
            .expect("bound")
            .generation += 1;
    }

    /// An S3TC format enabled: its storage.
    fn compressed_storage(&self, format: u32) -> Option<&'static str> {
        if !self.ext.on("WEBGL_compressed_texture_s3tc") {
            return None;
        }
        Some(match format {
            COMPRESSED_RGB_S3TC_DXT1_EXT | COMPRESSED_RGBA_S3TC_DXT1_EXT => "bc1-rgba-unorm",
            COMPRESSED_RGBA_S3TC_DXT3_EXT => "bc2-rgba-unorm",
            COMPRESSED_RGBA_S3TC_DXT5_EXT => "bc3-rgba-unorm",
            _ => return None,
        })
    }

    fn compressed_tex_image(&mut self, call: &CanvasCall) {
        let (target, level, internal) = (u(call, 1), i(call, 2), u(call, 3));
        let (w, h, border) = (i(call, 4), i(call, 5), i(call, 6));
        let Some((slot, face)) = image_target(target) else {
            self.error(INVALID_ENUM);
            return;
        };
        let Some(storage) = self.compressed_storage(internal) else {
            self.error(INVALID_ENUM);
            return;
        };
        if !self.check_level(slot, level, w, h) {
            return;
        }
        if border != 0 {
            self.error(INVALID_VALUE);
            return;
        }
        let (w, h) = (w as u32, h as u32);
        let data = call.bytes.clone().unwrap_or_default();
        let row = w.div_ceil(4) * block_bytes(internal);
        if data.len() as u64 != u64::from(row) * u64::from(h.div_ceil(4)) {
            self.error(INVALID_VALUE);
            return;
        }
        let side_ok = |v: u32| v.is_multiple_of(4) || (level > 0 && v <= 2);
        if !side_ok(w) || !side_ok(h) {
            self.error(INVALID_OPERATION);
            return;
        }
        let Some(id) = self.bound_texture(slot) else {
            self.error(INVALID_OPERATION);
            return;
        };
        let image = Image {
            width: w,
            height: h,
            format: Format {
                format: internal,
                kind: 0,
                storage,
            },
            gpu: None,
        };
        self.set_image(id, (face, level as u8), image, Some((data, row, 4)));
    }

    fn compressed_tex_sub_image(&mut self, call: &CanvasCall) {
        let (target, level, x, y) = (u(call, 1), i(call, 2), i(call, 3), i(call, 4));
        let (w, h, format) = (i(call, 5), i(call, 6), u(call, 7));
        let Some((slot, face)) = image_target(target) else {
            self.error(INVALID_ENUM);
            return;
        };
        if self.compressed_storage(format).is_none() {
            self.error(INVALID_ENUM);
            return;
        }
        if level < 0 || x < 0 || y < 0 || w < 0 || h < 0 {
            self.error(INVALID_VALUE);
            return;
        }
        let Some(id) = self.bound_texture(slot) else {
            self.error(INVALID_OPERATION);
            return;
        };
        let Some(image) = self.surfaces.textures[&id]
            .images
            .get(&(face, level as u8))
            .cloned()
        else {
            self.error(INVALID_OPERATION);
            return;
        };
        if image.format.format != format {
            self.error(INVALID_OPERATION);
            return;
        }
        let (x, y, w, h) = (x as u32, y as u32, w as u32, h as u32);
        if x + w > image.width || y + h > image.height {
            self.error(INVALID_VALUE);
            return;
        }
        let edge_ok = |at: u32, len: u32, side: u32| {
            at.is_multiple_of(4) && (len.is_multiple_of(4) || at + len == side)
        };
        if !edge_ok(x, w, image.width) || !edge_ok(y, h, image.height) {
            self.error(INVALID_OPERATION);
            return;
        }
        let data = call.bytes.clone().unwrap_or_default();
        let row = w.div_ceil(4) * block_bytes(format);
        if data.len() as u64 != u64::from(row) * u64::from(h.div_ceil(4)) {
            self.error(INVALID_VALUE);
            return;
        }
        let Some((texture, _, _)) = image.gpu else {
            return;
        };
        self.gpu_flush();
        self.upload(texture, (x, y), (round4(w), round4(h)), &data, row, 4);
        self.surfaces
            .textures
            .get_mut(&id)
            .expect("bound")
            .generation += 1;
    }

    /// `copyTexImage2D` [context, target, level, internal, x, y, width,
    /// height, border] and `copyTexSubImage2D` [context, target, level,
    /// x offset, y offset, x, y, width, height]: drawn by the blit from
    /// what the framebuffer reads.
    fn copy_tex_image(&mut self, call: &CanvasCall, sub: bool, size: (u32, u32)) {
        let (target, level) = (u(call, 1), i(call, 2));
        let Some((slot, face)) = image_target(target) else {
            self.error(INVALID_ENUM);
            return;
        };
        let (internal, at, x, y, w, h) = if sub {
            (
                0,
                (i(call, 3), i(call, 4)),
                i(call, 5),
                i(call, 6),
                i(call, 7),
                i(call, 8),
            )
        } else {
            (
                u(call, 3),
                (0, 0),
                i(call, 4),
                i(call, 5),
                i(call, 6),
                i(call, 7),
            )
        };
        if !sub {
            if !matches!(internal, ALPHA | LUMINANCE | LUMINANCE_ALPHA | RGB | RGBA) {
                self.error(INVALID_ENUM);
                return;
            }
            if !self.check_level(slot, level, w, h) {
                return;
            }
            if i(call, 8) != 0
                || (self.version == 1 && level > 0 && (!pot(w as u32) || !pot(h as u32)))
            {
                self.error(INVALID_VALUE);
                return;
            }
        } else if level < 0 || at.0 < 0 || at.1 < 0 || w < 0 || h < 0 {
            self.error(INVALID_VALUE);
            return;
        }
        let Some(id) = self.bound_texture(slot) else {
            self.error(INVALID_OPERATION);
            return;
        };
        let existing = self.surfaces.textures[&id]
            .images
            .get(&(face, level as u8))
            .cloned();
        let format = if sub {
            let Some(image) = &existing else {
                self.error(INVALID_OPERATION);
                return;
            };
            if (at.0 + w) as u32 > image.width || (at.1 + h) as u32 > image.height {
                self.error(INVALID_VALUE);
                return;
            }
            image.format
        } else {
            Format {
                format: internal,
                kind: UNSIGNED_BYTE,
                storage: "rgba8unorm",
            }
        };
        let Some(source) = self.read_source(size) else {
            return;
        };
        let wants_alpha = matches!(
            format.format,
            ALPHA | LUMINANCE_ALPHA | RGBA | SRGB_ALPHA_EXT
        );
        if !source.storage.starts_with("rgba8unorm")
            || !format.storage.starts_with("rgba8unorm")
            || (wants_alpha && source.opaque)
            || source.image == Some((id, face, level as u8))
        {
            self.error(INVALID_OPERATION);
            return;
        }
        if !sub {
            let image = Image {
                width: w as u32,
                height: h as u32,
                format,
                gpu: None,
            };
            self.set_image(id, (face, level as u8), image, None);
        }
        if w == 0 || h == 0 {
            return;
        }
        let dest = self.surfaces.textures[&id].images[&(face, level as u8)].clone();
        let Some((_, view, _)) = dest.gpu else {
            return;
        };
        let mode = match format.format {
            RGB | SRGB_EXT => 1,
            LUMINANCE => 2,
            ALPHA => 3,
            LUMINANCE_ALPHA => 4,
            _ => 0,
        };
        self.blit(
            source.view,
            source.size,
            (x, y),
            view,
            format.storage,
            (at.0 as u32, at.1 as u32),
            (w as u32, h as u32),
            mode,
        );
        self.surfaces
            .textures
            .get_mut(&id)
            .expect("bound")
            .generation += 1;
    }

    fn generate_mipmap(&mut self, target: u32) {
        let slot = match target {
            TEXTURE_2D => 0,
            TEXTURE_CUBE_MAP => 1,
            _ => {
                self.error(INVALID_ENUM);
                return;
            }
        };
        let Some(id) = self.bound_texture(slot) else {
            self.error(INVALID_OPERATION);
            return;
        };
        let faces: Vec<u8> = if slot == 1 { (0..6).collect() } else { vec![0] };
        let t = self.surfaces.textures[&id].clone();
        let Some(base) = t.images.get(&(0, 0)).cloned() else {
            self.error(INVALID_OPERATION);
            return;
        };
        let same = faces.iter().all(|&f| {
            t.images.get(&(f, 0)).is_some_and(|i| {
                i.width == base.width && i.height == base.height && i.format == base.format
            })
        });
        if !same
            || base.format.compressed()
            || base.format.depth()
            || (self.version == 1 && (!pot(base.width) || !pot(base.height)))
            || (base.format.storage == "rgba8unorm-srgb" && self.version == 1)
        {
            self.error(INVALID_OPERATION);
            return;
        }
        let levels = 32 - base.width.max(base.height).max(1).leading_zeros();
        for &face in &faces {
            for level in 1..levels as u8 {
                let above = self.surfaces.textures[&id].images[&(face, level - 1)].clone();
                let (w, h) = ((base.width >> level).max(1), (base.height >> level).max(1));
                let image = Image {
                    width: w,
                    height: h,
                    format: base.format,
                    gpu: None,
                };
                self.set_image(id, (face, level), image, None);
                let made = self.surfaces.textures[&id].images[&(face, level)].gpu;
                if let (Some((_, _, from)), Some((_, to, _))) = (above.gpu, made) {
                    self.blit(
                        from,
                        (above.width, above.height),
                        (0, 0),
                        to,
                        base.format.storage,
                        (0, 0),
                        (w, h),
                        5,
                    );
                }
            }
        }
    }

    // ---- Sampling ----

    /// The levels a texture is sampled with, or none when it is not
    /// complete (GL ES 2.0 3.8.2, WebGL 1's NPOT rule).
    fn complete(&self, t: &Texture, cube: bool) -> Option<u8> {
        let base = t.images.get(&(0, 0))?;
        if base.width == 0 || base.height == 0 || base.gpu.is_none() {
            return None;
        }
        let faces: &[u8] = if cube { &[0, 1, 2, 3, 4, 5] } else { &[0] };
        let mipmapped = !matches!(t.min, NEAREST | LINEAR);
        let npot = !pot(base.width) || !pot(base.height);
        if self.version == 1
            && npot
            && (mipmapped || t.wrap_s != CLAMP_TO_EDGE || t.wrap_t != CLAMP_TO_EDGE)
        {
            return None;
        }
        let levels = if mipmapped {
            (32 - base.width.max(base.height).leading_zeros()) as u8
        } else {
            1
        };
        for &face in faces {
            for level in 0..levels {
                let image = t.images.get(&(face, level))?;
                let want = ((base.width >> level).max(1), (base.height >> level).max(1));
                if (image.width, image.height) != want
                    || image.format != base.format
                    || image.gpu.is_none()
                {
                    return None;
                }
            }
        }
        // Filtering a format that cannot be filtered: incomplete.
        let nearest = t.mag == NEAREST && matches!(t.min, NEAREST | NEAREST_MIPMAP_NEAREST);
        let filterable = match base.format.storage {
            "rgba32float" => {
                self.ext.on("OES_texture_float_linear") && self.features.float32_filterable
            }
            "rgba16float" => self.version == 2 || self.ext.on("OES_texture_half_float_linear"),
            s => !has_depth(s) && !has_stencil(s),
        };
        if !filterable && !nearest {
            return None;
        }
        Some(levels)
    }

    /// A texture unit's texture of `cube` or not, as a draw samples it:
    /// view, sampler, whether WebGPU cannot filter it, and its id; none
    /// when there is none or it is not complete.
    pub(crate) fn sampled(&mut self, unit: u32, cube: bool) -> Option<(u32, u32, bool, u32)> {
        let id = self.surfaces.units.get(unit as usize)?[usize::from(cube)]?;
        let t = self.surfaces.textures.get(&id)?;
        let levels = self.complete(t, cube)?;
        let base = t.images[&(0, 0)].clone();
        let storage = base.format.storage;
        let unfilterable = has_depth(storage)
            || has_stencil(storage)
            || (storage == "rgba32float" && !self.features.float32_filterable);
        let (min, mag, wrap_s, wrap_t, anisotropy) =
            (t.min, t.mag, t.wrap_s, t.wrap_t, t.anisotropy);
        let view = if !cube && levels == 1 {
            base.gpu.expect("complete").2
        } else {
            self.assemble(id, cube, levels)
        };
        let address = |w: u32| match w {
            CLAMP_TO_EDGE => "clamp-to-edge",
            MIRRORED_REPEAT => "mirror-repeat",
            _ => "repeat",
        };
        let filter = |linear: bool| if linear { "linear" } else { "nearest" };
        let mag_linear = mag == LINEAR;
        let min_linear = matches!(min, LINEAR | LINEAR_MIPMAP_NEAREST | LINEAR_MIPMAP_LINEAR);
        let mip_linear = matches!(min, NEAREST_MIPMAP_LINEAR | LINEAR_MIPMAP_LINEAR) && levels > 1;
        let anisotropy = if mag_linear && min_linear && (mip_linear || levels == 1) && levels > 1 {
            anisotropy.clamp(1.0, 16.0) as u32
        } else {
            1
        };
        let desc = format!(
            r#"{{"addressModeU":"{}","addressModeV":"{}","magFilter":"{}","minFilter":"{}","mipmapFilter":"{}","lodMinClamp":0,"lodMaxClamp":{},"maxAnisotropy":{anisotropy}}}"#,
            address(wrap_s),
            address(wrap_t),
            filter(mag_linear),
            filter(min_linear),
            filter(mip_linear),
            if levels > 1 { 32 } else { 0 }
        );
        let sampler = match self.gpu.samplers.get(&desc) {
            Some(&s) => s,
            None => {
                let s = self.gpu.id(&self.client);
                self.client.queue([Request::CreateSampler {
                    device: self.device,
                    id: s,
                    desc: desc.clone(),
                }]);
                self.gpu.samplers.insert(desc, s);
                s
            }
        };
        Some((view, sampler, unfilterable, id))
    }

    /// A texture's images put together in one (a cube's faces, a
    /// texture's levels), again when they changed: its view.
    fn assemble(&mut self, id: u32, cube: bool, levels: u8) -> u32 {
        let t = &self.surfaces.textures[&id];
        let base = &t.images[&(0, 0)];
        let key: Assembly = (
            t.generation,
            levels,
            base.width,
            base.height,
            base.format.storage,
        );
        if let Some((_, view, had)) = t.assembled
            && had == key
        {
            return view;
        }
        let compressed = base.format.compressed();
        let (w, h, storage) = (base.width, base.height, base.format.storage);
        let old = t.assembled.map(|(a, b, _)| vec![a, b]);
        // What was recorded is sent before the copies go after it.
        self.gpu_flush();
        if let Some(old) = old {
            self.free(old);
        }
        let (texture, view) = (self.gpu.id(&self.client), self.gpu.id(&self.client));
        let layers = if cube { 6 } else { 1 };
        let (pw, ph) = if compressed {
            (round4(w), round4(h))
        } else {
            (w, h)
        };
        self.client.queue([
            Request::CreateTexture {
                device: self.device,
                id: texture,
                desc: format!(
                    r#"{{"size":[{pw},{ph},{layers}],"format":"{storage}","usage":6,"mipLevelCount":{levels}}}"#
                ),
            },
            Request::CreateView {
                texture,
                id: view,
                desc: if cube {
                    r#"{"dimension":"cube"}"#.to_owned()
                } else {
                    "{}".to_owned()
                },
            },
        ]);
        let t = &self.surfaces.textures[&id];
        for face in 0..layers as u8 {
            for level in 0..levels {
                let image = &t.images[&(face, level)];
                let Some((from, _, _)) = image.gpu else {
                    continue;
                };
                let (cw, ch) = if compressed {
                    (round4(image.width), round4(image.height))
                } else {
                    (image.width, image.height)
                };
                self.gpu.pending.push(Command::CopyTextureToTexture {
                    desc: format!(
                        r#"{{"source":{{"texture":{from}}},"destination":{{"texture":{texture},"mipLevel":{level},"origin":[0,0,{face}]}},"size":[{cw},{ch},1]}}"#
                    ),
                });
            }
        }
        self.surfaces
            .textures
            .get_mut(&id)
            .expect("there")
            .assembled = Some((texture, view, key));
        view
    }

    // ---- Renderbuffers ----

    fn renderbuffer_storage(&mut self, target: u32, internal: u32, w: i32, h: i32) {
        if target != RENDERBUFFER {
            self.error(INVALID_ENUM);
            return;
        }
        let storage = match internal {
            RGBA4 | RGB5_A1 | RGB565 => "rgba8unorm",
            DEPTH_COMPONENT16 => "depth16unorm",
            STENCIL_INDEX8 => "stencil8",
            DEPTH_STENCIL => "depth24plus-stencil8",
            SRGB8_ALPHA8_EXT if self.ext.on("EXT_sRGB") => "rgba8unorm-srgb",
            RGBA32F_EXT if self.ext.on("WEBGL_color_buffer_float") => "rgba32float",
            RGBA16F_EXT | RGB16F_EXT if self.ext.on("EXT_color_buffer_half_float") => "rgba16float",
            _ => {
                self.error(INVALID_ENUM);
                return;
            }
        };
        if w < 0 || h < 0 || w as u32 > MAX_SIZE || h as u32 > MAX_SIZE {
            self.error(INVALID_VALUE);
            return;
        }
        let Some(id) = self.surfaces.renderbuffer else {
            self.error(INVALID_OPERATION);
            return;
        };
        if let Some((a, b)) = self.surfaces.renderbuffers[&id].gpu {
            self.free(vec![a, b]);
        }
        let gpu = (w > 0 && h > 0).then(|| {
            let (texture, view) = (self.gpu.id(&self.client), self.gpu.id(&self.client));
            // RENDER_ATTACHMENT, and COPY_SRC and TEXTURE_BINDING for a
            // colour one's copies and reads.
            let usage = if storage.starts_with("rgba") { 21 } else { 16 };
            self.client.queue([
                Request::CreateTexture {
                    device: self.device,
                    id: texture,
                    desc: format!(r#"{{"size":[{w},{h},1],"format":"{storage}","usage":{usage}}}"#),
                },
                Request::CreateView {
                    texture,
                    id: view,
                    desc: "{}".to_owned(),
                },
            ]);
            (texture, view)
        });
        let r = self.surfaces.renderbuffers.get_mut(&id).expect("bound");
        r.format = internal;
        r.width = w as u32;
        r.height = h as u32;
        r.storage = Some(storage);
        r.gpu = gpu;
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
        let stored = r.storage.is_some();
        // Red, green, blue, alpha, depth and stencil bits.
        let bits: [u32; 6] = match (stored, r.format) {
            (false, _) => [0; 6],
            (_, RGBA4) => [4, 4, 4, 4, 0, 0],
            (_, RGB5_A1) => [5, 5, 5, 1, 0, 0],
            (_, RGB565) => [5, 6, 5, 0, 0, 0],
            (_, DEPTH_COMPONENT16) => [0, 0, 0, 0, 16, 0],
            (_, STENCIL_INDEX8) => [0, 0, 0, 0, 0, 8],
            (_, DEPTH_STENCIL) => [0, 0, 0, 0, 24, 8],
            (_, RGBA32F_EXT) => [32, 32, 32, 32, 0, 0],
            (_, RGBA16F_EXT) => [16, 16, 16, 16, 0, 0],
            (_, RGB16F_EXT) => [16, 16, 16, 0, 0, 0],
            _ => [8, 8, 8, 8, 0, 0],
        };
        let v = match name {
            RENDERBUFFER_WIDTH => r.width,
            RENDERBUFFER_HEIGHT => r.height,
            RENDERBUFFER_INTERNAL_FORMAT => r.format,
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

    fn attachment_point(&self, attachment: u32) -> bool {
        matches!(
            attachment,
            COLOR_ATTACHMENT0 | DEPTH_ATTACHMENT | STENCIL_ATTACHMENT | DEPTH_STENCIL_ATTACHMENT
        )
    }

    fn framebuffer_texture(
        &mut self,
        target: u32,
        attachment: u32,
        textarget: u32,
        texture: u32,
        level: i32,
    ) {
        if target != FRAMEBUFFER || !self.attachment_point(attachment) {
            self.error(INVALID_ENUM);
            return;
        }
        let Some(fb) = self.surfaces.framebuffer else {
            self.error(INVALID_OPERATION);
            return;
        };
        let attached = if texture == 0 {
            None
        } else {
            let Some((slot, face)) = image_target(textarget) else {
                self.error(INVALID_ENUM);
                return;
            };
            let Some(t) = self.surfaces.textures.get(&texture).filter(|t| !t.deleted) else {
                self.error(INVALID_OPERATION);
                return;
            };
            let wanted = if slot == 0 {
                TEXTURE_2D
            } else {
                TEXTURE_CUBE_MAP
            };
            if t.target != Some(wanted) {
                self.error(INVALID_OPERATION);
                return;
            }
            let mipmaps = self.version == 2 || self.ext.on("OES_fbo_render_mipmap");
            if !(0..=MAX_LEVEL).contains(&level) || (level != 0 && !mipmaps) {
                self.error(INVALID_VALUE);
                return;
            }
            Some(Attached::Texture {
                id: texture,
                face,
                level: level as u8,
            })
        };
        let attachments = &mut self
            .surfaces
            .framebuffers
            .get_mut(&fb)
            .expect("bound")
            .attachments;
        match attached {
            Some(a) => attachments.insert(attachment, a),
            None => attachments.remove(&attachment),
        };
        self.sweep();
    }

    fn framebuffer_renderbuffer(
        &mut self,
        target: u32,
        attachment: u32,
        rb_target: u32,
        renderbuffer: u32,
    ) {
        if target != FRAMEBUFFER || !self.attachment_point(attachment) || rb_target != RENDERBUFFER
        {
            self.error(INVALID_ENUM);
            return;
        }
        let Some(fb) = self.surfaces.framebuffer else {
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
        let attachments = &mut self
            .surfaces
            .framebuffers
            .get_mut(&fb)
            .expect("bound")
            .attachments;
        if renderbuffer == 0 {
            attachments.remove(&attachment);
        } else {
            attachments.insert(attachment, Attached::Renderbuffer(renderbuffer));
        }
        self.sweep();
    }

    /// What an attachment is: none when its object is gone.
    fn info(&self, a: &Attached) -> Option<Info> {
        match *a {
            Attached::Texture { id, face, level } => {
                let image = self
                    .surfaces
                    .textures
                    .get(&id)?
                    .images
                    .get(&(face, level))?;
                let f = image.format;
                let color_ok = match f.storage {
                    "rgba8unorm" => {
                        matches!(f.format, RGB | RGBA)
                    }
                    "rgba8unorm-srgb" => f.format == SRGB_ALPHA_EXT,
                    "rgba32float" => {
                        matches!(f.format, RGB | RGBA)
                            && (self.version == 2 || self.ext.on("WEBGL_color_buffer_float"))
                    }
                    "rgba16float" => {
                        matches!(f.format, RGB | RGBA)
                            && (self.version == 2 || self.ext.on("EXT_color_buffer_half_float"))
                    }
                    _ => false,
                };
                Some(Info {
                    view: image.gpu.map(|g| g.1),
                    texture: Some(id),
                    gpu_texture: image.gpu.map(|g| g.0),
                    sample_view: image.gpu.map(|g| g.2),
                    storage: f.storage,
                    size: (image.width, image.height),
                    color_ok,
                    opaque: f.opaque(),
                })
            }
            Attached::Renderbuffer(id) => {
                let r = self.surfaces.renderbuffers.get(&id)?;
                let storage = r.storage.unwrap_or("rgba8unorm");
                Some(Info {
                    view: r.gpu.map(|g| g.1),
                    texture: None,
                    gpu_texture: r.gpu.map(|g| g.0),
                    sample_view: r.gpu.map(|g| g.1),
                    storage,
                    size: (r.width, r.height),
                    color_ok: storage.starts_with("rgba"),
                    opaque: matches!(r.format, RGB565 | RGB16F_EXT),
                })
            }
        }
    }

    /// A framebuffer's completeness (GL ES 2.0 4.4.5, WebGL 1 6.6).
    pub(crate) fn status(&self, fb: u32) -> u32 {
        let Some(fb) = self.surfaces.framebuffers.get(&fb) else {
            return FRAMEBUFFER_COMPLETE;
        };
        let mut sizes = Vec::new();
        for (&point, a) in &fb.attachments {
            let Some(info) = self.info(a) else {
                return FRAMEBUFFER_INCOMPLETE_ATTACHMENT;
            };
            if info.size.0 == 0 || info.size.1 == 0 {
                return FRAMEBUFFER_INCOMPLETE_ATTACHMENT;
            }
            let ok = match point {
                COLOR_ATTACHMENT0 => info.color_ok,
                DEPTH_ATTACHMENT => matches!(info.storage, "depth16unorm" | "depth32float"),
                STENCIL_ATTACHMENT => info.storage == "stencil8",
                _ => info.storage == "depth24plus-stencil8",
            };
            if !ok {
                return FRAMEBUFFER_INCOMPLETE_ATTACHMENT;
            }
            sizes.push(info.size);
        }
        if sizes.is_empty() {
            return FRAMEBUFFER_INCOMPLETE_MISSING_ATTACHMENT;
        }
        if sizes.iter().any(|&s| s != sizes[0]) {
            return FRAMEBUFFER_INCOMPLETE_DIMENSIONS;
        }
        let depths = [
            DEPTH_ATTACHMENT,
            STENCIL_ATTACHMENT,
            DEPTH_STENCIL_ATTACHMENT,
        ]
        .iter()
        .filter(|p| fb.attachments.contains_key(p))
        .count();
        if depths > 1 {
            return FRAMEBUFFER_UNSUPPORTED;
        }
        FRAMEBUFFER_COMPLETE
    }

    fn attachment_parameter(&mut self, target: u32, attachment: u32, name: u32) -> Answer {
        if target != FRAMEBUFFER || !self.attachment_point(attachment) {
            self.error(INVALID_ENUM);
            return Answer::Nothing;
        }
        let Some(fb) = self.surfaces.framebuffer else {
            self.error(INVALID_OPERATION);
            return Answer::Nothing;
        };
        let a = self.surfaces.framebuffers[&fb]
            .attachments
            .get(&attachment)
            .copied();
        let srgb = self.ext.on("EXT_sRGB");
        let float =
            self.ext.on("WEBGL_color_buffer_float") || self.ext.on("EXT_color_buffer_half_float");
        match (a, name) {
            (None, FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE) => Answer::Number(f64::from(NONE)),
            (None, _) => {
                self.error(INVALID_ENUM);
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
            (Some(a), FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING_EXT) if srgb => {
                let storage = self.info(&a).map_or("", |i| i.storage);
                Answer::Number(f64::from(if storage.ends_with("srgb") {
                    SRGB_COLOR_ENCODING
                } else {
                    LINEAR_COLOR_ENCODING
                }))
            }
            (Some(a), FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE_EXT) if float => {
                let storage = self.info(&a).map_or("", |i| i.storage);
                Answer::Number(f64::from(if storage.ends_with("float") {
                    FLOAT
                } else {
                    UNSIGNED_NORMALIZED_EXT
                }))
            }
            _ => {
                self.error(INVALID_ENUM);
                Answer::Nothing
            }
        }
    }

    /// What draws and clears draw into: the bound framebuffer's
    /// attachments, or the drawing buffer; none, and GL's error, for an
    /// incomplete framebuffer.
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
            return Some(Target {
                key: (color, depth.map_or(0, |d| d.0)),
                size: d.size,
                color: Some((color, resolve, "rgba8unorm")),
                rgb: !self.attributes.alpha,
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
        let attachments = self.surfaces.framebuffers[&fb].attachments.clone();
        let mut target = Target {
            key: (0, 0),
            size: (0, 0),
            color: None,
            rgb: false,
            depth: None,
            depth_on: false,
            stencil_on: false,
            samples: 1,
            default: false,
            textures: Vec::new(),
        };
        for (point, a) in &attachments {
            let info = self.info(a).expect("complete");
            let view = info.view.expect("complete");
            target.size = info.size;
            if let Some(t) = info.texture {
                target.textures.push(t);
            }
            if *point == COLOR_ATTACHMENT0 {
                target.color = Some((view, None, info.storage));
                target.rgb = info.opaque;
                target.key.0 = view;
            } else {
                target.depth = Some((view, info.storage));
                target.depth_on = has_depth(info.storage);
                target.stencil_on = has_stencil(info.storage);
                target.key.1 = view;
            }
        }
        Some(target)
    }

    /// What `readPixels` and the copies read: the bound framebuffer's
    /// colour, or the drawing buffer's (its clear made first); none, and
    /// GL's error, when there is none to read.
    fn read_source(&mut self, size: (u32, u32)) -> Option<Source> {
        let Some(fb) = self.surfaces.framebuffer else {
            if self.needs_clear {
                // The drawing buffer cleared, as presented ones are.
                self.pass(size, None);
            }
            self.gpu_flush();
            self.drawing_buffer(size);
            let d = self.drawing.clone().expect("made");
            return Some(Source {
                texture: d.color.0,
                view: d.color.1,
                size: d.size,
                storage: "rgba8unorm",
                opaque: !self.attributes.alpha,
                image: None,
            });
        };
        if self.status(fb) != FRAMEBUFFER_COMPLETE {
            self.error(INVALID_FRAMEBUFFER_OPERATION);
            return None;
        }
        let Some(a) = self.surfaces.framebuffers[&fb]
            .attachments
            .get(&COLOR_ATTACHMENT0)
            .copied()
        else {
            self.error(INVALID_OPERATION);
            return None;
        };
        self.gpu_flush();
        let info = self.info(&a).expect("complete");
        let image = match a {
            Attached::Texture { id, face, level } => Some((id, face, level)),
            Attached::Renderbuffer(_) => None,
        };
        Some(Source {
            texture: info.gpu_texture.expect("complete"),
            view: info.sample_view.expect("complete"),
            size: info.size,
            storage: info.storage,
            opaque: info.opaque,
            image,
        })
    }

    /// The format and type `readPixels` takes besides RGBA and
    /// UNSIGNED_BYTE: the colour buffer's own.
    pub(crate) fn read_format(&self) -> (u32, u32) {
        let storage = self
            .surfaces
            .framebuffer
            .and_then(|fb| self.surfaces.framebuffers.get(&fb))
            .and_then(|fb| fb.attachments.get(&COLOR_ATTACHMENT0))
            .and_then(|a| self.info(a))
            .map_or("rgba8unorm", |i| i.storage);
        if storage.ends_with("float") {
            (RGBA, FLOAT)
        } else {
            (RGBA, UNSIGNED_BYTE)
        }
    }

    /// `readPixels` [context, x, y, width, height, format, type, array
    /// kind] with the array's bytes: those bytes with the rectangle's
    /// pixels in the framebuffer written, rows from the bottom; nothing
    /// on an error.
    fn read_pixels(&mut self, call: &CanvasCall, size: (u32, u32)) -> Answer {
        let (x, y, w, h) = (i(call, 1), i(call, 2), i(call, 3), i(call, 4));
        let (format, kind, view) = (u(call, 5), u(call, 6), u(call, 7));
        if !matches!(format, ALPHA | RGB | RGBA)
            || !matches!(
                kind,
                UNSIGNED_BYTE
                    | UNSIGNED_SHORT_5_6_5
                    | UNSIGNED_SHORT_4_4_4_4
                    | UNSIGNED_SHORT_5_5_5_1
                    | FLOAT
                    | HALF_FLOAT_OES
            )
        {
            self.error(INVALID_ENUM);
            return Answer::Nothing;
        }
        if w < 0 || h < 0 {
            self.error(INVALID_VALUE);
            return Answer::Nothing;
        }
        if let Some(fb) = self.surfaces.framebuffer
            && self.status(fb) != FRAMEBUFFER_COMPLETE
        {
            self.error(INVALID_FRAMEBUFFER_OPERATION);
            return Answer::Nothing;
        }
        let float = self.read_format().1 == FLOAT;
        let allowed = if float {
            (format, kind) == (RGBA, FLOAT)
        } else {
            (format, kind) == (RGBA, UNSIGNED_BYTE)
        };
        if !allowed {
            self.error(INVALID_OPERATION);
            return Answer::Nothing;
        }
        if (kind == UNSIGNED_BYTE && view != 1) || (kind == FLOAT && view != 3) {
            self.error(INVALID_OPERATION);
            return Answer::Nothing;
        }
        let pixel = if float { 16 } else { 4 };
        let (stride, whole) = rows(w as u32, h as u32, pixel, self.state.pack_alignment);
        let mut out = call.bytes.clone().unwrap_or_default();
        if (out.len() as u64) < whole {
            self.error(INVALID_OPERATION);
            return Answer::Nothing;
        }
        let Some(source) = self.read_source(size) else {
            return Answer::Nothing;
        };
        // The rectangle within the framebuffer; pixels outside it stay.
        let x0 = x.clamp(0, source.size.0 as i32);
        let y0 = y.clamp(0, source.size.1 as i32);
        let x1 = x.saturating_add(w).clamp(0, source.size.0 as i32);
        let y1 = y.saturating_add(h).clamp(0, source.size.1 as i32);
        if x1 <= x0 || y1 <= y0 {
            return Answer::Bytes(out);
        }
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
                        r#"{{"source":{{"texture":{},"origin":[{x0},{y0},0]}},"destination":{{"buffer":{buffer},"bytesPerRow":{row}}},"size":[{cw},{ch},1]}}"#,
                        source.texture
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
        let Some((Reply::Mapped { error: None, .. }, bytes)) = read else {
            return Answer::Bytes(out);
        };
        for r in 0..ch as usize {
            let from = &bytes[r * row as usize..];
            let at = (r + (y0 - y) as usize) * stride as usize + (x0 - x) as usize * pixel as usize;
            for c in 0..cw as usize {
                let p = &from[c * texel as usize..(c + 1) * texel as usize];
                let o = at + c * pixel as usize;
                match source.storage {
                    "rgba16float" => {
                        for k in 0..4 {
                            let v = f16_to_f32(u16::from_le_bytes([p[2 * k], p[2 * k + 1]]));
                            out[o + 4 * k..o + 4 * k + 4].copy_from_slice(&v.to_le_bytes());
                        }
                    }
                    _ => out[o..o + pixel as usize].copy_from_slice(&p[..pixel as usize]),
                }
                if source.opaque {
                    if float {
                        out[o + 12..o + 16].copy_from_slice(&1.0f32.to_le_bytes());
                    } else {
                        out[o + 3] = 255;
                    }
                }
            }
        }
        Answer::Bytes(out)
    }

    // ---- The blit ----

    /// Draws `extent` texels of `dest` at `at` from `source` (of `size`)
    /// at `origin` (texels outside it 0) by `mode`: 0 as they are, 1 RGB
    /// with alpha 1, 2 luminance, 3 alpha, 4 luminance and alpha, 5 the
    /// mean of a 2 by 2 for the next level.
    #[allow(clippy::too_many_arguments)]
    fn blit(
        &mut self,
        source: u32,
        size: (u32, u32),
        origin: (i32, i32),
        dest: u32,
        storage: &'static str,
        at: (u32, u32),
        extent: (u32, u32),
        mode: i32,
    ) {
        let device = self.device;
        if self.gpu.blit.is_none() {
            let (module, layout, pipeline_layout) = (
                self.gpu.id(&self.client),
                self.gpu.id(&self.client),
                self.gpu.id(&self.client),
            );
            self.client.queue([
                Request::CreateShaderModule {
                    device,
                    id: module,
                    code: BLIT.to_owned(),
                },
                Request::CreateBindGroupLayout {
                    device,
                    id: layout,
                    desc: r#"{"entries":[{"binding":0,"visibility":2,"buffer":{"type":"uniform","hasDynamicOffset":true,"minBindingSize":32}},{"binding":1,"visibility":2,"texture":{"sampleType":"unfilterable-float"}}]}"#.to_owned(),
                },
                Request::CreatePipelineLayout {
                    device,
                    id: pipeline_layout,
                    layouts: vec![layout],
                },
            ]);
            self.gpu.blit = Some(Blit {
                module,
                layout,
                pipeline_layout,
                pipelines: HashMap::new(),
            });
        }
        let blit = self.gpu.blit.as_ref().expect("made");
        let (module, layout, pipeline_layout) = (blit.module, blit.layout, blit.pipeline_layout);
        let pipeline = match blit.pipelines.get(storage) {
            Some(&p) => p,
            None => {
                let id = self.gpu.id(&self.client);
                self.client.queue([Request::CreateRenderPipeline {
                    device,
                    id,
                    desc: format!(
                        r#"{{"layout":{pipeline_layout},"vertex":{{"module":{module},"entryPoint":"vs","buffers":[]}},"fragment":{{"module":{module},"entryPoint":"fs","targets":[{{"format":"{storage}"}}]}}}}"#
                    ),
                    call: None,
                }]);
                self.gpu
                    .blit
                    .as_mut()
                    .expect("made")
                    .pipelines
                    .insert(storage, id);
                id
            }
        };
        let mut data = Vec::with_capacity(32);
        for v in [
            origin.0,
            origin.1,
            at.0 as i32,
            at.1 as i32,
            size.0 as i32,
            size.1 as i32,
            mode,
            0,
        ] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        // The pass of what was drawn ends first.
        self.end_pass();
        let offset = match self.gpu.arena(&self.client, &data, 256) {
            Some(o) => o,
            None => {
                self.gpu_flush();
                match self.gpu.arena(&self.client, &data, 256) {
                    Some(o) => o,
                    None => return,
                }
            }
        };
        let key = (layout, vec![source]);
        let group = match self.gpu.groups.get(&key) {
            Some(&g) => g,
            None => {
                let g = self.gpu.id(&self.client);
                self.client.queue([Request::CreateBindGroup {
                    device,
                    id: g,
                    desc: format!(
                        r#"{{"layout":{layout},"entries":[{{"binding":0,"buffer":{},"offset":0,"size":32}},{{"binding":1,"view":{source}}}]}}"#,
                        self.gpu.arena_id()
                    ),
                }]);
                self.gpu.groups.insert(key, g);
                g
            }
        };
        self.gpu.pending.push(Command::RenderPass {
            desc: format!(
                r#"{{"colorAttachments":[{{"view":{dest},"loadOp":"load","storeOp":"store"}}]}}"#
            ),
            commands: vec![
                RenderCommand::SetPipeline { pipeline },
                RenderCommand::SetBindGroup {
                    index: 0,
                    group: Some(group),
                    offsets: vec![offset as u32],
                },
                RenderCommand::SetViewport {
                    x: at.0 as f32,
                    y: at.1 as f32,
                    width: extent.0 as f32,
                    height: extent.1 as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                },
                RenderCommand::SetScissorRect {
                    x: at.0,
                    y: at.1,
                    width: extent.0,
                    height: extent.1,
                },
                RenderCommand::Draw {
                    vertices: 3,
                    instances: 1,
                    first_vertex: 0,
                    first_instance: 0,
                },
            ],
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_convert_both_ways() {
        for v in [0.0f32, 1.0, -2.5, 0.5, 65504.0, 6.0e-8, 0.333_333] {
            let back = f16_to_f32(f32_to_f16(v));
            assert!((back - v).abs() <= v.abs() / 1000.0 + 6.0e-8, "{v} {back}");
        }
        assert_eq!(f32_to_f16(1.0), 0x3C00);
        assert_eq!(f32_to_f16(1.0e9), 0x7C00);
    }

    #[test]
    fn client_rows_become_rgba_texels() {
        let fmt = |format, kind, storage| Format {
            format,
            kind,
            storage,
        };
        // Two rows of a 1-wide LUMINANCE_ALPHA at alignment 4: padded.
        let la = texels(
            fmt(LUMINANCE_ALPHA, UNSIGNED_BYTE, "rgba8unorm"),
            1,
            2,
            &[10, 20, 0, 0, 30, 40],
            4,
            false,
            false,
        );
        assert_eq!(la, [10, 10, 10, 20, 30, 30, 30, 40]);
        // Turned over and premultiplied.
        let rgba = texels(
            fmt(RGBA, UNSIGNED_BYTE, "rgba8unorm"),
            1,
            2,
            &[255, 0, 0, 128, 0, 255, 0, 255],
            1,
            true,
            true,
        );
        assert_eq!(rgba, [0, 255, 0, 255, 128, 0, 0, 128]);
        assert_eq!(unpack565(0xF800), [255, 0, 0, 255]);
        assert_eq!(unpack4444(0x000F), [0, 0, 0, 255]);
        assert_eq!(unpack5551(0x07C1), [0, 255, 0, 255]);
        // ALPHA floats: colours 0.
        let a = texels(
            fmt(ALPHA, FLOAT, "rgba32float"),
            1,
            1,
            &0.5f32.to_le_bytes(),
            4,
            false,
            false,
        );
        let back: Vec<f32> = a
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect();
        assert_eq!(back, [0.0, 0.0, 0.0, 0.5]);
        assert_eq!(rows(3, 2, 3, 4), (12, 21));
    }
}
