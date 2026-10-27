//! WebGL's textures and samplers (M41 steps 7 and 8, D40): GL's images,
//! each defined on its own, on WebGPU's textures of a fixed shape. Each
//! image (a face's or a level's, a 3D or array level's layers
//! together) is a texture of its own in the GPU process, which a
//! framebuffer draws into; a texture sampled past one level, or a cube,
//! is put together from its images into one, again when they change.
//! Copies and mipmaps are drawn by a blit of this module's.

use std::collections::{BTreeMap, HashMap};

use js::{Answer, CanvasCall};

use super::formats::{self, Format, Layout, Sample, has_depth, has_stencil, texel_bytes};
use super::gl::*;
use super::{Context, MAX_SIZE, i, n, u};
use crate::gpu::wire::{Command, GPU_CHUNK, RenderCommand, Request};
use crate::ipc::Bytes;

/// The levels a texture may have: a side of [`MAX_SIZE`] halved to 1.
pub(crate) const MAX_LEVEL: i32 = 13;

/// The largest 3D texture's side and array texture's layers.
pub(crate) const MAX_3D: u32 = 2048;

/// The binding slots of a texture unit.
pub(crate) const SLOT_2D: usize = 0;
pub(crate) const SLOT_CUBE: usize = 1;
pub(crate) const SLOT_3D: usize = 2;
pub(crate) const SLOT_ARRAY: usize = 3;

/// A texture image: its size (layers or depth its third side), format,
/// its texture and views in the GPU process (none when empty: the
/// texture, the view drawn into, the one sampled), and views of single
/// layers made for framebuffers and copies.
#[derive(Clone, Debug)]
pub(crate) struct Image {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub format: Format,
    pub gpu: Option<(u32, u32, u32)>,
    pub layers: BTreeMap<u32, u32>,
}

/// What a sampler (a texture's own, or a sampler object) says.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SamplerState {
    pub min: u32,
    pub mag: u32,
    pub wrap_s: u32,
    pub wrap_t: u32,
    pub wrap_r: u32,
    pub min_lod: f32,
    pub max_lod: f32,
    pub compare_mode: u32,
    pub compare_func: u32,
}

impl Default for SamplerState {
    fn default() -> SamplerState {
        SamplerState {
            min: NEAREST_MIPMAP_LINEAR,
            mag: LINEAR,
            wrap_s: REPEAT,
            wrap_t: REPEAT,
            wrap_r: REPEAT,
            min_lod: -1000.0,
            max_lod: 1000.0,
            compare_mode: NONE,
            compare_func: LEQUAL,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Texture {
    pub target: Option<u32>,
    pub deleted: bool,
    pub sampler: SamplerState,
    pub anisotropy: f32,
    pub base_level: u32,
    pub max_level: u32,
    /// texStorage's levels: its images fixed.
    pub immutable: Option<u32>,
    /// Its images by face (0 but for a cube's) and level.
    pub images: BTreeMap<(u8, u8), Image>,
    /// Counted up at every change to its images.
    pub generation: u64,
    /// Its images put together for sampling: texture, view, and what
    /// they were put together from.
    pub assembled: Option<(u32, u32, Assembly)>,
}

pub(crate) type Assembly = (u64, u32, u8, (u32, u32, u32), &'static str);

impl Texture {
    fn new() -> Texture {
        Texture {
            target: None,
            deleted: false,
            sampler: SamplerState::default(),
            anisotropy: 1.0,
            base_level: 0,
            max_level: 1000,
            immutable: None,
            images: BTreeMap::new(),
            generation: 0,
            assembled: None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SamplerObject {
    pub deleted: bool,
    pub state: SamplerState,
}

/// What a program's sampler reads: its binding slot (by dimension), what
/// it samples, and whether it compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Want {
    pub slot: usize,
    pub sample: Sample,
    pub shadow: bool,
}

/// A texture as a draw samples it: its view and sampler, whether WebGPU
/// cannot filter it, and its id.
pub(crate) struct Sampled {
    pub view: u32,
    pub sampler: u32,
    pub unfilterable: bool,
    pub id: u32,
}

/// The blit's modules, layouts and pipelines (by the storage drawn into
/// and whether the source is 3D).
#[derive(Debug)]
pub(crate) struct Blit {
    modules: [u32; 2],
    layouts: [u32; 2],
    pipeline_layouts: [u32; 2],
    pipelines: HashMap<(&'static str, bool), u32>,
}

const BLIT: &str = "struct B { src: vec2<i32>, dst: vec2<i32>, size: vec2<i32>, mode: i32, z: i32 }
@group(0) @binding(0) var<uniform> b: B;
@group(0) @binding(1) var t: TEX;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var p = array(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(p[i], 0.0, 1.0);
}
fn load(s: vec2<i32>) -> vec4<f32> { return LOAD; }
@fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let d = vec2<i32>(floor(p.xy)) - b.dst;
    if (b.mode == 5) {
        let s = d * 2;
        let m = b.size - vec2<i32>(1, 1);
        let c = load(min(s, m)) + load(min(s + vec2<i32>(1, 0), m))
            + load(min(s + vec2<i32>(0, 1), m)) + load(min(s + vec2<i32>(1, 1), m));
        return c * 0.25;
    }
    let s = d + b.src;
    if (any(s < vec2<i32>(0, 0)) || any(s >= b.size)) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let c = load(s);
    switch b.mode {
        case 1: { return vec4<f32>(c.rgb, 1.0); }
        case 2: { return vec4<f32>(c.rrr, 1.0); }
        case 3: { return vec4<f32>(0.0, 0.0, 0.0, c.a); }
        case 4: { return vec4<f32>(c.rrr, c.a); }
        case 6: { return vec4<f32>(c.r, 0.0, 0.0, 1.0); }
        case 7: { return vec4<f32>(c.rg, 0.0, 1.0); }
        default: { return c; }
    }
}
";

/// GL's target of a texture image: the binding slot and face.
pub(crate) fn image_target(target: u32, version: u8) -> Option<(usize, u8)> {
    match target {
        TEXTURE_2D => Some((SLOT_2D, 0)),
        TEXTURE_CUBE_MAP_POSITIVE_X..=TEXTURE_CUBE_MAP_NEGATIVE_Z => {
            Some((SLOT_CUBE, (target - TEXTURE_CUBE_MAP_POSITIVE_X) as u8))
        }
        TEXTURE_3D if version == 2 => Some((SLOT_3D, 0)),
        TEXTURE_2D_ARRAY if version == 2 => Some((SLOT_ARRAY, 0)),
        _ => None,
    }
}

/// A texture binding target's slot.
pub(crate) fn slot_of(target: u32, version: u8) -> Option<usize> {
    match target {
        TEXTURE_2D => Some(SLOT_2D),
        TEXTURE_CUBE_MAP => Some(SLOT_CUBE),
        TEXTURE_3D if version == 2 => Some(SLOT_3D),
        TEXTURE_2D_ARRAY if version == 2 => Some(SLOT_ARRAY),
        _ => None,
    }
}

fn pot(v: u32) -> bool {
    v.is_power_of_two()
}

fn round4(v: u32) -> u32 {
    v.div_ceil(4) * 4
}

/// Whether WebGPU draws into a storage.
pub(crate) fn renderable_storage(storage: &str, rg11b10: bool) -> bool {
    !(storage.contains("snorm")
        || storage == "rgb9e5ufloat"
        || storage.starts_with("bc")
        || (storage == "rg11b10ufloat" && !rg11b10))
}

/// A level's side from the base's.
fn mip(side: u32, level: u32) -> u32 {
    (side >> level).max(1)
}

/// A level's third side: a 3D texture's halves, an array's layers stay.
fn mip_depth(slot: usize, depth: u32, level: u32) -> u32 {
    if slot == SLOT_3D {
        mip(depth, level)
    } else {
        depth
    }
}

/// What an upload says, from its call.
#[derive(Clone, Copy, Debug)]
struct Upload {
    target: u32,
    level: i32,
    internal: u32,
    at: (i32, i32, i32),
    size: (i32, i32, i32),
    border: i32,
    format: u32,
    kind: u32,
    /// 0 an array's bytes, 1 none, 2 an image source's RGBA, 3 the pixel
    /// unpack buffer's at `offset`.
    source: u32,
    view: u32,
    offset: f64,
    sub: bool,
    three: bool,
}

impl Upload {
    /// texImage2D: [context, target, level, internal, width, height,
    /// border, format, type, source, array kind, offset]; texImage3D a
    /// depth after the height; the sub images an x, y (and z) for the
    /// internal format and no border.
    fn of(call: &CanvasCall, sub: bool, three: bool) -> Upload {
        let mut k = 0;
        let mut next = || {
            k += 1;
            k
        };
        let target = u(call, next());
        let level = i(call, next());
        let internal = if sub { 0 } else { u(call, next()) };
        let at = if sub {
            (
                i(call, next()),
                i(call, next()),
                if three { i(call, next()) } else { 0 },
            )
        } else {
            (0, 0, 0)
        };
        let size = (
            i(call, next()),
            i(call, next()),
            if three { i(call, next()) } else { 1 },
        );
        let border = if sub { 0 } else { i(call, next()) };
        let format = u(call, next());
        let kind = u(call, next());
        let source = u(call, next());
        let view = u(call, next());
        let offset = n(call, next());
        Upload {
            target,
            level,
            internal,
            at,
            size,
            border,
            format,
            kind,
            source,
            view,
            offset,
            sub,
            three,
        }
    }
}

impl Context {
    /// The calls of textures and samplers, and the extensions; none for
    /// another.
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
            "texImage2D" | "texImage3D" | "texSubImage2D" | "texSubImage3D" => {
                let upload = Upload::of(call, op.starts_with("texSub"), op.ends_with("3D"));
                self.tex_image(&upload, call.bytes.as_deref().unwrap_or_default());
                Answer::Nothing
            }
            "texStorage2D" | "texStorage3D" => {
                let three = op == "texStorage3D";
                let depth = if three { i(call, 6) } else { 1 };
                self.tex_storage(
                    u(call, 1),
                    i(call, 2),
                    u(call, 3),
                    (i(call, 4), i(call, 5), depth),
                    three,
                );
                Answer::Nothing
            }
            "compressedTexImage2D"
            | "compressedTexSubImage2D"
            | "compressedTexImage3D"
            | "compressedTexSubImage3D" => {
                self.compressed_tex_image(call, op.contains("Sub"), op.ends_with("3D"));
                Answer::Nothing
            }
            "copyTexImage2D" | "copyTexSubImage2D" | "copyTexSubImage3D" => {
                self.copy_tex_image(call, op, size);
                Answer::Nothing
            }
            "generateMipmap" => {
                self.generate_mipmap(u(call, 1));
                Answer::Nothing
            }
            // ---- Sampler objects ----
            "createSampler" => {
                let id = self.id();
                self.surfaces.samplers.insert(
                    id,
                    SamplerObject {
                        deleted: false,
                        state: SamplerState::default(),
                    },
                );
                Answer::Number(f64::from(id))
            }
            "deleteSampler" => {
                let id = u(call, 1);
                if let Some(s) = self.surfaces.samplers.get_mut(&id) {
                    s.deleted = true;
                    for unit in &mut self.surfaces.sampler_units {
                        if *unit == Some(id) {
                            *unit = None;
                        }
                    }
                }
                Answer::Nothing
            }
            "isSampler" => Answer::Bool(
                self.surfaces
                    .samplers
                    .get(&u(call, 1))
                    .is_some_and(|s| !s.deleted),
            ),
            "bindSampler" => {
                let (unit, id) = (u(call, 1) as usize, u(call, 2));
                if unit >= self.surfaces.sampler_units.len() {
                    self.error(INVALID_VALUE);
                } else if id != 0 && !self.surfaces.samplers.get(&id).is_some_and(|s| !s.deleted) {
                    self.error(INVALID_OPERATION);
                } else {
                    self.surfaces.sampler_units[unit] = (id != 0).then_some(id);
                }
                Answer::Nothing
            }
            "samplerParameteri" | "samplerParameterf" => {
                let (id, name, value) = (u(call, 1), u(call, 2), n(call, 3));
                match self
                    .surfaces
                    .samplers
                    .get(&id)
                    .filter(|s| !s.deleted)
                    .map(|s| s.state)
                {
                    None => self.error(INVALID_OPERATION),
                    Some(mut state) => match set_sampler(&mut state, name, value) {
                        Err(e) => self.error(e),
                        Ok(()) => self.surfaces.samplers.get_mut(&id).expect("there").state = state,
                    },
                }
                Answer::Nothing
            }
            "getSamplerParameter" => {
                let (id, name) = (u(call, 1), u(call, 2));
                match self
                    .surfaces
                    .samplers
                    .get(&id)
                    .filter(|s| !s.deleted)
                    .map(|s| s.state)
                {
                    None => {
                        self.error(INVALID_OPERATION);
                        Answer::Nothing
                    }
                    Some(state) => match get_sampler(&state, name) {
                        Some(v) => Answer::Number(v),
                        None => {
                            self.error(INVALID_ENUM);
                            Answer::Nothing
                        }
                    },
                }
            }
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

    pub(crate) fn unit(&self) -> usize {
        (self.state.active_texture - TEXTURE0) as usize
    }

    /// The texture bound to `slot` of the active unit.
    pub(crate) fn bound_texture(&self, slot: usize) -> Option<u32> {
        self.surfaces.units[self.unit()][slot]
    }

    fn bind_texture(&mut self, target: u32, id: u32) {
        let Some(slot) = slot_of(target, self.version) else {
            self.error(INVALID_ENUM);
            return;
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
        self.detach_texture(id);
        self.sweep();
    }

    /// GPU objects gone: what was recorded with them sent first.
    pub(crate) fn free(&mut self, mut ids: Vec<u32>) {
        ids.sort_unstable();
        ids.dedup();
        if ids.is_empty() {
            return;
        }
        self.gpu_flush();
        self.gpu.forget(&self.client, &ids);
    }

    /// An image's GPU objects, its layers' views among them.
    pub(crate) fn image_objects(image: &Image) -> Vec<u32> {
        let mut out: Vec<u32> = image.layers.values().copied().collect();
        if let Some((a, b, c)) = image.gpu {
            out.extend([a, b, c]);
        }
        out
    }

    /// The texture bound for a parameter call's target.
    fn parameter_texture(&mut self, target: u32) -> Option<u32> {
        let Some(slot) = slot_of(target, self.version) else {
            self.error(INVALID_ENUM);
            return None;
        };
        let id = self.bound_texture(slot);
        if id.is_none() {
            self.error(INVALID_OPERATION);
        }
        id
    }

    fn tex_parameter(&mut self, target: u32, name: u32, value: f64) {
        let Some(id) = self.parameter_texture(target) else {
            return;
        };
        let v2 = self.version == 2;
        let anisotropic = self.ext.on("EXT_texture_filter_anisotropic");
        let t = self.surfaces.textures.get_mut(&id).expect("bound");
        let result = match name {
            TEXTURE_MAX_ANISOTROPY_EXT if anisotropic => {
                if value < 1.0 || value.is_nan() {
                    Err(INVALID_VALUE)
                } else {
                    t.anisotropy = value as f32;
                    Ok(())
                }
            }
            TEXTURE_BASE_LEVEL | TEXTURE_MAX_LEVEL if v2 => {
                if value < 0.0 {
                    Err(INVALID_VALUE)
                } else {
                    if name == TEXTURE_BASE_LEVEL {
                        t.base_level = value as u32;
                    } else {
                        t.max_level = value as u32;
                    }
                    Ok(())
                }
            }
            TEXTURE_WRAP_R | TEXTURE_MIN_LOD | TEXTURE_MAX_LOD | TEXTURE_COMPARE_MODE
            | TEXTURE_COMPARE_FUNC
                if !v2 =>
            {
                Err(INVALID_ENUM)
            }
            _ => set_sampler(&mut t.sampler, name, value),
        };
        if let Err(e) = result {
            self.error(e);
        }
    }

    fn get_tex_parameter(&mut self, target: u32, name: u32) -> Answer {
        let Some(id) = self.parameter_texture(target) else {
            return Answer::Nothing;
        };
        let v2 = self.version == 2;
        let anisotropic = self.ext.on("EXT_texture_filter_anisotropic");
        let t = &self.surfaces.textures[&id];
        let v = match name {
            TEXTURE_MAX_ANISOTROPY_EXT if anisotropic => Some(f64::from(t.anisotropy)),
            TEXTURE_BASE_LEVEL if v2 => Some(f64::from(t.base_level)),
            TEXTURE_MAX_LEVEL if v2 => Some(f64::from(t.max_level)),
            TEXTURE_IMMUTABLE_FORMAT if v2 => return Answer::Bool(t.immutable.is_some()),
            TEXTURE_IMMUTABLE_LEVELS if v2 => Some(f64::from(t.immutable.unwrap_or(0))),
            TEXTURE_WRAP_R | TEXTURE_MIN_LOD | TEXTURE_MAX_LOD | TEXTURE_COMPARE_MODE
            | TEXTURE_COMPARE_FUNC
                if !v2 =>
            {
                None
            }
            _ => get_sampler(&t.sampler, name),
        };
        match v {
            Some(v) => Answer::Number(v),
            None => {
                self.error(INVALID_ENUM);
                Answer::Nothing
            }
        }
    }

    /// GL's format and type as a texture's: the format or GL's error.
    pub(crate) fn texture_format(
        &self,
        internal: u32,
        format: u32,
        kind: u32,
    ) -> Result<Format, u32> {
        formats::texture_format(
            self.version,
            &|name| self.ext.on(name),
            self.features.depth32_stencil,
            internal,
            format,
            kind,
        )
    }

    /// A new texture in the GPU process of `size` (a 3D one's third side
    /// its depth, else layers): texture, view drawn into, view sampled.
    pub(crate) fn make_texture(
        &mut self,
        size: (u32, u32, u32),
        storage: &str,
        slot: usize,
    ) -> (u32, u32, u32) {
        let (texture, view) = (self.gpu.id(&self.client), self.gpu.id(&self.client));
        // COPY_SRC | COPY_DST | TEXTURE_BINDING | RENDER_ATTACHMENT, but
        // what WebGPU will not do with the storage.
        let usage = if has_depth(storage) || has_stencil(storage) {
            if storage == "depth16unorm" { 22 } else { 20 }
        } else if !renderable_storage(storage, self.features.rg11b10_renderable) {
            7
        } else {
            23
        };
        let dimension = if slot == SLOT_3D { "3d" } else { "2d" };
        self.client.queue([
            Request::CreateTexture {
                device: self.device,
                id: texture,
                desc: format!(
                    r#"{{"size":[{},{},{}],"format":"{storage}","usage":{usage},"dimension":"{dimension}"}}"#,
                    size.0, size.1, size.2
                ),
            },
            Request::CreateView {
                texture,
                id: view,
                desc: if slot == SLOT_ARRAY {
                    r#"{"dimension":"2d-array"}"#.to_owned()
                } else {
                    "{}".to_owned()
                },
            },
        ]);
        // Sampled: a depth-and-stencil's depth.
        let sample = if has_stencil(storage) && has_depth(storage) {
            let id = self.gpu.id(&self.client);
            let dimension = if slot == SLOT_ARRAY {
                r#","dimension":"2d-array""#
            } else {
                ""
            };
            self.client.queue([Request::CreateView {
                texture,
                id,
                desc: format!(r#"{{"aspect":"depth-only"{dimension}}}"#),
            }]);
            id
        } else {
            view
        };
        (texture, view, sample)
    }

    /// A view of one layer of an image to draw into (an array's layer as
    /// a 2D view; a 3D image's view and the slice), made once.
    pub(crate) fn layer_view(
        &mut self,
        id: u32,
        key: (u8, u8),
        layer: u32,
    ) -> Option<(u32, Option<u32>)> {
        let t = self.surfaces.textures.get(&id)?;
        let slot = t.target.and_then(|target| slot_of(target, self.version))?;
        let image = t.images.get(&key)?;
        let (texture, view, _) = image.gpu?;
        if slot == SLOT_3D {
            return Some((view, Some(layer)));
        }
        if slot != SLOT_ARRAY {
            return Some((view, None));
        }
        if let Some(&v) = image.layers.get(&layer) {
            return Some((v, None));
        }
        let v = self.gpu.id(&self.client);
        self.client.queue([Request::CreateView {
            texture,
            id: v,
            desc: format!(r#"{{"dimension":"2d","baseArrayLayer":{layer},"arrayLayerCount":1}}"#),
        }]);
        self.surfaces
            .textures
            .get_mut(&id)
            .and_then(|t| t.images.get_mut(&key))
            .expect("there")
            .layers
            .insert(layer, v);
        Some((v, None))
    }

    /// `bytes`, rows `row` bytes long, written into `texture` at
    /// `origin`, `size` texels (`block` rows of texels a row of bytes).
    fn upload(
        &mut self,
        texture: u32,
        origin: (u32, u32, u32),
        size: (u32, u32, u32),
        bytes: &[u8],
        row: u32,
        block: u32,
    ) {
        if bytes.is_empty() || row == 0 {
            return;
        }
        let rows = size.1.div_ceil(block);
        let per = (GPU_CHUNK as u32 / row).max(1);
        for z in 0..size.2 {
            let start = (z * rows * row) as usize;
            let Some(image) = bytes.get(start..start + (rows * row) as usize) else {
                return;
            };
            let mut first = 0;
            while first < rows {
                let count = per.min(rows - first);
                let piece =
                    image[(first * row) as usize..((first + count) * row) as usize].to_vec();
                self.client.queue([Request::WriteTexture {
                    device: self.device,
                    desc: format!(
                        r#"{{"destination":{{"texture":{texture},"origin":[{},{},{}]}},"dataLayout":{{"bytesPerRow":{row},"rowsPerImage":{count}}},"size":[{},{},1]}}"#,
                        origin.0,
                        origin.1 + first * block,
                        origin.2 + z,
                        size.0,
                        count * block
                    ),
                    bytes: Bytes(piece),
                }]);
                first += count;
            }
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
            .map(Self::image_objects);
        if let Some(old) = old {
            self.free(old);
        }
        let slot = self.surfaces.textures[&id]
            .target
            .and_then(|t| slot_of(t, self.version))
            .unwrap_or(SLOT_2D);
        if image.width > 0 && image.height > 0 && image.depth > 0 {
            let physical = if image.format.compressed() {
                (round4(image.width), round4(image.height), image.depth)
            } else {
                (image.width, image.height, image.depth)
            };
            let made = self.make_texture(physical, image.format.storage, slot);
            if let Some((bytes, row, block)) = texels {
                self.upload(made.0, (0, 0, 0), physical, &bytes, row, block);
            }
            image.gpu = Some(made);
        }
        let t = self.surfaces.textures.get_mut(&id).expect("bound");
        t.images.insert(key, image);
        t.generation += 1;
    }

    /// GL's checks of a level and size: false, and GL's error, when wrong.
    fn check_level(&mut self, slot: usize, level: i32, w: i32, h: i32, d: i32) -> bool {
        if !(0..=MAX_LEVEL).contains(&level) || w < 0 || h < 0 || d < 0 {
            self.error(INVALID_VALUE);
            return false;
        }
        let most = (MAX_SIZE >> level) as i32;
        let third = match slot {
            SLOT_3D => (MAX_3D >> level) as i32,
            SLOT_ARRAY => MAX_3D as i32,
            _ => 1,
        };
        let side = if slot == SLOT_3D {
            (MAX_3D >> level) as i32
        } else {
            most
        };
        if w > side || h > side || d > third || (slot == SLOT_CUBE && w != h) {
            self.error(INVALID_VALUE);
            return false;
        }
        true
    }

    /// The unpack state as a layout of client rows.
    pub(crate) fn unpack_layout(&self, three: bool) -> Layout {
        let s = &self.state;
        Layout {
            alignment: s.unpack_alignment,
            row_length: s.unpack_row_length,
            image_height: if three { s.unpack_image_height } else { 0 },
            skip_pixels: s.unpack_skip_pixels,
            skip_rows: s.unpack_skip_rows,
            skip_images: if three { s.unpack_skip_images } else { 0 },
        }
    }

    /// The data of an upload: an array's (`data`), the pixel unpack
    /// buffer's, or none; GL's error when it cannot be.
    fn upload_data(&self, up: &Upload, data: &[u8]) -> Result<Option<Vec<u8>>, u32> {
        match (up.source, self.v2.pixel_unpack) {
            (3, None) | (0 | 2, Some(_)) => Err(INVALID_OPERATION),
            (3, Some(buffer)) => {
                let bytes = &self.buffers[&buffer].data;
                if up.offset < 0.0 || up.offset > bytes.len() as f64 {
                    return Err(INVALID_OPERATION);
                }
                Ok(Some(bytes[up.offset as usize..].to_vec()))
            }
            (1, _) => Ok(None),
            _ => Ok(Some(data.to_vec())),
        }
    }

    /// texImage2D, texImage3D, texSubImage2D and texSubImage3D.
    fn tex_image(&mut self, up: &Upload, data: &[u8]) {
        let Some((slot, face)) = image_target(up.target, self.version) else {
            self.error(INVALID_ENUM);
            return;
        };
        // The 2D calls take 2D and cube targets; the 3D ones 3D and array.
        if up.three != (slot == SLOT_3D || slot == SLOT_ARRAY) {
            self.error(INVALID_ENUM);
            return;
        }
        let (w, h, d) = up.size;
        if !self.check_level(slot, up.level, w, h, d) {
            return;
        }
        if up.border != 0 {
            self.error(INVALID_VALUE);
            return;
        }
        let Some(id) = self.bound_texture(slot) else {
            self.error(INVALID_OPERATION);
            return;
        };
        let key = (face, up.level as u8);
        let existing = self.surfaces.textures[&id].images.get(&key).cloned();
        // The format: the call's, or for a sub image the image's with the
        // call's format and type, which must be ones it takes.
        let fmt = if up.sub {
            let Some(image) = &existing else {
                self.error(INVALID_OPERATION);
                return;
            };
            if let Err(e) = self.texture_format(image.format.internal, up.format, up.kind) {
                self.error(if e == INVALID_ENUM {
                    e
                } else {
                    INVALID_OPERATION
                });
                return;
            }
            let (x, y, z) = up.at;
            if x < 0
                || y < 0
                || z < 0
                || (x + w) as u32 > image.width
                || (y + h) as u32 > image.height
                || (z + d) as u32 > image.depth
            {
                self.error(INVALID_VALUE);
                return;
            }
            if image.format.compressed()
                || (self.version == 1
                    && (image.format.format != up.format || image.format.kind != up.kind))
            {
                self.error(INVALID_OPERATION);
                return;
            }
            Format {
                format: up.format,
                kind: up.kind,
                ..image.format
            }
        } else {
            if self.surfaces.textures[&id].immutable.is_some() {
                self.error(INVALID_OPERATION);
                return;
            }
            match self.texture_format(up.internal, up.format, up.kind) {
                Ok(fmt) => fmt,
                Err(e) => {
                    self.error(e);
                    return;
                }
            }
        };
        let (w, h, d) = (w as u32, h as u32, d as u32);
        if !up.sub && self.version == 1 && up.level > 0 && (!pot(w) || !pot(h)) {
            self.error(INVALID_VALUE);
            return;
        }
        // Depth textures: 2D (cubes and arrays too in WebGL 2), WebGL 1's
        // of level 0 and given no data.
        if fmt.depth() && (slot == SLOT_3D || (slot == SLOT_CUBE && self.version == 1)) {
            self.error(INVALID_OPERATION);
            return;
        }
        if fmt.depth() && self.version == 1 && (up.level != 0 || up.source != 1) {
            self.error(INVALID_OPERATION);
            return;
        }
        if up.sub && up.source == 1 {
            self.error(INVALID_VALUE);
            return;
        }
        if up.source == 2 && fmt.depth() {
            self.error(INVALID_OPERATION);
            return;
        }
        let bytes = match self.upload_data(up, data) {
            Ok(b) => b,
            Err(e) => {
                self.error(e);
                return;
            }
        };
        let layout = self.unpack_layout(up.three);
        let pixel = formats::client_pixel(fmt.format, fmt.kind);
        let texels = match (up.source, bytes) {
            (2, Some(rgba)) => {
                // The source whole (its size given, or the upload's), its
                // rectangle at the skips, a 3D slice an image's rows on.
                let (sw, sh) = if up.view > 0 {
                    (up.view, up.offset as u32)
                } else {
                    (w, h * d)
                };
                let image_rows = if up.three && layout.image_height > 0 {
                    layout.image_height
                } else {
                    h
                };
                let (x0, y0) = (layout.skip_pixels, layout.skip_rows);
                let last_row = u64::from(y0)
                    + u64::from(d.saturating_sub(1)) * u64::from(image_rows)
                    + u64::from(h);
                if u64::from(x0) + u64::from(w) > u64::from(sw)
                    || (d > 0 && last_row > u64::from(sh))
                    || (rgba.len() as u64) < u64::from(sw) * u64::from(sh) * 4
                {
                    self.error(INVALID_OPERATION);
                    return;
                }
                let mut part = Vec::with_capacity((w * h * d * 4) as usize);
                for z in 0..d {
                    for y in 0..h {
                        let row = (y0 + z * image_rows + y) as usize;
                        let start = (row * sw as usize + x0 as usize) * 4;
                        part.extend_from_slice(&rgba[start..start + w as usize * 4]);
                    }
                }
                Some(formats::from_rgba8(&fmt, w, h * d, &part))
            }
            (_, Some(bytes)) => {
                if up.source == 0 && !formats::view_ok(fmt.kind, up.view) {
                    self.error(INVALID_OPERATION);
                    return;
                }
                let (_, _, _, whole) = layout.measure(w, h, d, pixel);
                if (bytes.len() as u64) < whole {
                    self.error(INVALID_OPERATION);
                    return;
                }
                let flip = self.state.unpack_flip_y;
                let premultiply = self.state.unpack_premultiply_alpha;
                // Depth data WebGPU takes for 16-bit depth alone (G-list).
                (!fmt.depth() || fmt.storage == "depth16unorm")
                    .then(|| formats::texels(&fmt, w, h, d, &bytes, &layout, flip, premultiply))
            }
            // No data: zeros, but a format without alpha keeps alpha 1.
            (_, None) if fmt.opaque() && !fmt.depth() && !up.sub => {
                let zeros = vec![0; (w * h * d * pixel) as usize];
                Some(formats::texels(
                    &fmt,
                    w,
                    h,
                    d,
                    &zeros,
                    &Layout::packed(1),
                    false,
                    false,
                ))
            }
            (_, None) => None,
        };
        let row = w * texel_bytes(fmt.storage);
        if !up.sub {
            let image = Image {
                width: w,
                height: h,
                depth: d,
                format: fmt,
                gpu: None,
                layers: BTreeMap::new(),
            };
            self.set_image(id, key, image, texels.map(|t| (t, row, 1)));
            return;
        }
        let Some(texels) = texels else {
            return;
        };
        if w == 0 || h == 0 || d == 0 {
            return;
        }
        let Some((texture, _, _)) = existing.and_then(|e| e.gpu) else {
            return;
        };
        self.gpu_flush();
        let (x, y, z) = up.at;
        self.upload(
            texture,
            (x as u32, y as u32, z as u32),
            (w, h, d),
            &texels,
            row,
            1,
        );
        self.surfaces
            .textures
            .get_mut(&id)
            .expect("bound")
            .generation += 1;
    }

    /// texStorage2D and texStorage3D: every level made at once, fixed.
    fn tex_storage(
        &mut self,
        target: u32,
        levels: i32,
        internal: u32,
        size: (i32, i32, i32),
        three: bool,
    ) {
        let slot = match (target, three) {
            (TEXTURE_2D, false) => SLOT_2D,
            (TEXTURE_CUBE_MAP, false) => SLOT_CUBE,
            (TEXTURE_3D, true) => SLOT_3D,
            (TEXTURE_2D_ARRAY, true) => SLOT_ARRAY,
            _ => {
                self.error(INVALID_ENUM);
                return;
            }
        };
        let (w, h, d) = size;
        if levels < 1 || w < 1 || h < 1 || d < 1 {
            self.error(INVALID_VALUE);
            return;
        }
        let compressed = self.compressed_storage(internal).map(|storage| Format {
            internal,
            format: internal,
            kind: 0,
            storage,
        });
        let Some(fmt) = formats::sized(internal, self.features.depth32_stencil).or(compressed)
        else {
            self.error(INVALID_ENUM);
            return;
        };
        if !self.check_level(slot, 0, w, h, d) {
            return;
        }
        let (w, h, d) = (w as u32, h as u32, d as u32);
        let most = if slot == SLOT_3D {
            w.max(h).max(d)
        } else {
            w.max(h)
        };
        if levels as u32 > 32 - most.leading_zeros() || (fmt.depth() && slot == SLOT_3D) {
            self.error(INVALID_OPERATION);
            return;
        }
        let Some(id) = self.bound_texture(slot) else {
            self.error(INVALID_OPERATION);
            return;
        };
        if self.surfaces.textures[&id].immutable.is_some() {
            self.error(INVALID_OPERATION);
            return;
        }
        let faces: u8 = if slot == SLOT_CUBE { 6 } else { 1 };
        for level in 0..levels as u32 {
            let size = (mip(w, level), mip(h, level), mip_depth(slot, d, level));
            for face in 0..faces {
                let pixel = formats::client_pixel(fmt.format, fmt.kind);
                let texels = (fmt.opaque() && !fmt.depth() && !fmt.compressed()).then(|| {
                    let zeros = vec![0; (size.0 * size.1 * size.2 * pixel) as usize];
                    formats::texels(
                        &fmt,
                        size.0,
                        size.1,
                        size.2,
                        &zeros,
                        &Layout::packed(1),
                        false,
                        false,
                    )
                });
                let row = size.0 * texel_bytes(fmt.storage);
                let image = Image {
                    width: size.0,
                    height: size.1,
                    depth: size.2,
                    format: fmt,
                    gpu: None,
                    layers: BTreeMap::new(),
                };
                self.set_image(id, (face, level as u8), image, texels.map(|t| (t, row, 1)));
            }
        }
        self.surfaces
            .textures
            .get_mut(&id)
            .expect("bound")
            .immutable = Some(levels as u32);
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

    /// compressedTexImage2D/3D [context, target, level, internal, width,
    /// height, (depth,) border] and the sub images [context, target,
    /// level, x, y, (z,) width, height, (depth,) format], the data's
    /// bytes after.
    fn compressed_tex_image(&mut self, call: &CanvasCall, sub: bool, three: bool) {
        let mut k = 0;
        let mut next = || {
            k += 1;
            k
        };
        let target = u(call, next());
        let level = i(call, next());
        let internal = if sub { 0 } else { u(call, next()) };
        let at = if sub {
            (
                i(call, next()),
                i(call, next()),
                if three { i(call, next()) } else { 0 },
            )
        } else {
            (0, 0, 0)
        };
        let (w, h, d) = (
            i(call, next()),
            i(call, next()),
            if three { i(call, next()) } else { 1 },
        );
        let last = i(call, next());
        let format = if sub { last as u32 } else { internal };
        let border = if sub { 0 } else { last };
        let Some((slot, face)) = image_target(target, self.version) else {
            self.error(INVALID_ENUM);
            return;
        };
        if three != (slot == SLOT_3D || slot == SLOT_ARRAY) {
            self.error(INVALID_ENUM);
            return;
        }
        let Some(storage) = self.compressed_storage(format) else {
            self.error(INVALID_ENUM);
            return;
        };
        if slot == SLOT_3D {
            self.error(INVALID_OPERATION);
            return;
        }
        if !self.check_level(slot, level, w, h, d) {
            return;
        }
        if border != 0 || at.0 < 0 || at.1 < 0 || at.2 < 0 {
            self.error(INVALID_VALUE);
            return;
        }
        let Some(id) = self.bound_texture(slot) else {
            self.error(INVALID_OPERATION);
            return;
        };
        let (w, h, d) = (w as u32, h as u32, d as u32);
        let data = call.bytes.clone().unwrap_or_default();
        let block = if matches!(
            format,
            COMPRESSED_RGB_S3TC_DXT1_EXT | COMPRESSED_RGBA_S3TC_DXT1_EXT
        ) {
            8
        } else {
            16
        };
        let row = w.div_ceil(4) * block;
        if data.len() as u64 != u64::from(row) * u64::from(h.div_ceil(4)) * u64::from(d) {
            self.error(INVALID_VALUE);
            return;
        }
        let key = (face, level as u8);
        if !sub {
            let side_ok = |v: u32| v.is_multiple_of(4) || (level > 0 && v <= 2);
            if !side_ok(w) || !side_ok(h) || self.surfaces.textures[&id].immutable.is_some() {
                self.error(INVALID_OPERATION);
                return;
            }
            let image = Image {
                width: w,
                height: h,
                depth: d,
                format: Format {
                    internal: format,
                    format,
                    kind: 0,
                    storage,
                },
                gpu: None,
                layers: BTreeMap::new(),
            };
            self.set_image(id, key, image, Some((data, row, 4)));
            return;
        }
        let Some(image) = self.surfaces.textures[&id].images.get(&key).cloned() else {
            self.error(INVALID_OPERATION);
            return;
        };
        if image.format.internal != format {
            self.error(INVALID_OPERATION);
            return;
        }
        let (x, y, z) = (at.0 as u32, at.1 as u32, at.2 as u32);
        if x + w > image.width || y + h > image.height || z + d > image.depth {
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
        let Some((texture, _, _)) = image.gpu else {
            return;
        };
        self.gpu_flush();
        self.upload(texture, (x, y, z), (round4(w), round4(h), d), &data, row, 4);
        self.surfaces
            .textures
            .get_mut(&id)
            .expect("bound")
            .generation += 1;
    }

    /// copyTexImage2D [context, target, level, internal, x, y, width,
    /// height, border], copyTexSubImage2D [context, target, level, x
    /// offset, y offset, x, y, width, height] and copyTexSubImage3D (a z
    /// offset after the y offset): drawn by the blit from what the read
    /// framebuffer reads.
    fn copy_tex_image(&mut self, call: &CanvasCall, op: &str, size: (u32, u32)) {
        let sub = op != "copyTexImage2D";
        let three = op == "copyTexSubImage3D";
        let (target, level) = (u(call, 1), i(call, 2));
        let Some((slot, face)) = image_target(target, self.version) else {
            self.error(INVALID_ENUM);
            return;
        };
        if three != (slot == SLOT_3D || slot == SLOT_ARRAY) {
            self.error(INVALID_ENUM);
            return;
        }
        let (internal, at, x, y, w, h) = if three {
            (
                0,
                (i(call, 3), i(call, 4), i(call, 5)),
                i(call, 6),
                i(call, 7),
                i(call, 8),
                i(call, 9),
            )
        } else if sub {
            (
                0,
                (i(call, 3), i(call, 4), 0),
                i(call, 5),
                i(call, 6),
                i(call, 7),
                i(call, 8),
            )
        } else {
            (
                u(call, 3),
                (0, 0, 0),
                i(call, 4),
                i(call, 5),
                i(call, 6),
                i(call, 7),
            )
        };
        if !sub {
            if !self.check_level(slot, level, w, h, 1) {
                return;
            }
            if i(call, 8) != 0
                || (self.version == 1 && level > 0 && (!pot(w as u32) || !pot(h as u32)))
            {
                self.error(INVALID_VALUE);
                return;
            }
        } else if !(0..=MAX_LEVEL).contains(&level)
            || at.0 < 0
            || at.1 < 0
            || at.2 < 0
            || w < 0
            || h < 0
        {
            self.error(INVALID_VALUE);
            return;
        }
        let Some(id) = self.bound_texture(slot) else {
            self.error(INVALID_OPERATION);
            return;
        };
        let key = (face, level as u8);
        let existing = self.surfaces.textures[&id].images.get(&key).cloned();
        let format = if sub {
            let Some(image) = &existing else {
                self.error(INVALID_OPERATION);
                return;
            };
            if (at.0 + w) as u32 > image.width
                || (at.1 + h) as u32 > image.height
                || at.2 as u32 >= image.depth
            {
                self.error(INVALID_VALUE);
                return;
            }
            image.format
        } else {
            if self.surfaces.textures[&id].immutable.is_some() {
                self.error(INVALID_OPERATION);
                return;
            }
            match internal {
                ALPHA | LUMINANCE | LUMINANCE_ALPHA | RGB | RGBA => Format {
                    internal,
                    format: internal,
                    kind: UNSIGNED_BYTE,
                    storage: "rgba8unorm",
                },
                _ if self.version == 2 => match formats::sized(internal, false) {
                    Some(f) if f.sample() == Sample::Float && !f.depth() => f,
                    Some(_) => {
                        self.error(INVALID_OPERATION);
                        return;
                    }
                    None => {
                        self.error(INVALID_ENUM);
                        return;
                    }
                },
                _ => {
                    self.error(INVALID_ENUM);
                    return;
                }
            }
        };
        let Some(source) = self.read_source(size) else {
            return;
        };
        let wants_alpha = matches!(
            format.format,
            ALPHA | LUMINANCE_ALPHA | RGBA | SRGB_ALPHA_EXT
        );
        let float_storage = |s: &str| formats::info(s).2 == Sample::Float;
        if !float_storage(source.storage)
            || !float_storage(format.storage)
            || !renderable_storage(format.storage, self.features.rg11b10_renderable)
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
                depth: 1,
                format,
                gpu: None,
                layers: BTreeMap::new(),
            };
            self.set_image(id, key, image, None);
        }
        if w == 0 || h == 0 {
            return;
        }
        let Some(dest) = self.layer_view(id, key, at.2 as u32) else {
            return;
        };
        let mode = match format.format {
            RGB | SRGB_EXT => 1,
            LUMINANCE => 2,
            ALPHA => 3,
            LUMINANCE_ALPHA => 4,
            RED => 6,
            RG => 7,
            _ => 0,
        };
        self.blit(
            (source.view, false, 0),
            source.size,
            (x, y),
            dest,
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
        let Some(slot) = slot_of(target, self.version) else {
            self.error(INVALID_ENUM);
            return;
        };
        let Some(id) = self.bound_texture(slot) else {
            self.error(INVALID_OPERATION);
            return;
        };
        let faces: Vec<u8> = if slot == SLOT_CUBE {
            (0..6).collect()
        } else {
            vec![0]
        };
        let t = self.surfaces.textures[&id].clone();
        let base_level = t.base_level.min(MAX_LEVEL as u32) as u8;
        let Some(base) = t.images.get(&(0, base_level)).cloned() else {
            self.error(INVALID_OPERATION);
            return;
        };
        let same = faces.iter().all(|&f| {
            t.images.get(&(f, base_level)).is_some_and(|i| {
                i.width == base.width && i.height == base.height && i.format == base.format
            })
        });
        let storage = base.format.storage;
        let filterable = formats::info(storage).2 == Sample::Float
            && !storage.contains("snorm")
            && storage != "rgb9e5ufloat"
            && (!storage.ends_with("32float")
                || self.version == 1
                || self.ext.on("OES_texture_float_linear"));
        let ext = |name: &str| self.ext.on(name);
        let renderable = renderable_storage(storage, self.features.rg11b10_renderable)
            && (self.version == 1
                || base.format.is_unsized()
                || formats::color_renderable(
                    2,
                    &ext,
                    self.features.rg11b10_renderable,
                    &base.format,
                ));
        if !same
            || base.format.compressed()
            || base.format.depth()
            || !filterable
            || !renderable
            || (self.version == 1 && (!pot(base.width) || !pot(base.height)))
            || (storage == "rgba8unorm-srgb" && self.version == 1)
        {
            self.error(INVALID_OPERATION);
            return;
        }
        let most = if slot == SLOT_3D {
            base.width.max(base.height).max(base.depth)
        } else {
            base.width.max(base.height)
        };
        let count = 32 - most.max(1).leading_zeros();
        let last = match t.immutable {
            Some(levels) => levels,
            None => (u32::from(base_level) + count).min(t.max_level.saturating_add(1)),
        };
        for &face in &faces {
            for level in u32::from(base_level) + 1..last.min(MAX_LEVEL as u32 + 1) {
                let level = level as u8;
                let step = u32::from(level - base_level);
                let above = self.surfaces.textures[&id].images[&(face, level - 1)].clone();
                let (w, h, d) = (
                    mip(base.width, step),
                    mip(base.height, step),
                    mip_depth(slot, base.depth, step),
                );
                if t.immutable.is_none() {
                    let image = Image {
                        width: w,
                        height: h,
                        depth: d,
                        format: base.format,
                        gpu: None,
                        layers: BTreeMap::new(),
                    };
                    self.set_image(id, (face, level), image, None);
                }
                // Each layer from the level above (a 3D level's slices from
                // every other slice above: G-list).
                for layer in 0..d {
                    let from_layer = if slot == SLOT_3D {
                        (layer * 2).min(above.depth - 1)
                    } else {
                        layer
                    };
                    let source = match slot {
                        SLOT_ARRAY => self
                            .layer_view(id, (face, level - 1), from_layer)
                            .map(|v| (v.0, false, 0)),
                        SLOT_3D => above.gpu.map(|g| (g.2, true, from_layer as i32)),
                        _ => above.gpu.map(|g| (g.2, false, 0)),
                    };
                    let dest = self.layer_view(id, (face, level), layer);
                    if let (Some(source), Some(dest)) = (source, dest) {
                        self.blit(
                            source,
                            (above.width, above.height),
                            (0, 0),
                            dest,
                            storage,
                            (0, 0),
                            (w, h),
                            5,
                        );
                    }
                }
            }
        }
        self.surfaces
            .textures
            .get_mut(&id)
            .expect("bound")
            .generation += 1;
    }

    // ---- Sampling ----

    /// The levels a texture is sampled with, from its base level, or none
    /// when it is not complete (GL ES 3.0 3.8.14, WebGL 1's NPOT rule).
    fn complete(&self, t: &Texture, slot: usize, state: &SamplerState) -> Option<u8> {
        let base_level = t.base_level.min(MAX_LEVEL as u32) as u8;
        let base = t.images.get(&(0, base_level))?;
        if base.width == 0 || base.height == 0 || base.depth == 0 || base.gpu.is_none() {
            return None;
        }
        let faces: &[u8] = if slot == SLOT_CUBE {
            &[0, 1, 2, 3, 4, 5]
        } else {
            &[0]
        };
        let mipmapped = !matches!(state.min, NEAREST | LINEAR);
        let npot = !pot(base.width) || !pot(base.height);
        if self.version == 1
            && npot
            && (mipmapped || state.wrap_s != CLAMP_TO_EDGE || state.wrap_t != CLAMP_TO_EDGE)
        {
            return None;
        }
        let most = if slot == SLOT_3D {
            base.width.max(base.height).max(base.depth)
        } else {
            base.width.max(base.height)
        };
        let mut levels = if mipmapped {
            32 - most.leading_zeros()
        } else {
            1
        };
        if let Some(immutable) = t.immutable {
            levels = levels.min(immutable.saturating_sub(u32::from(base_level)));
        }
        if mipmapped {
            levels = levels.min(t.max_level.saturating_sub(u32::from(base_level)) + 1);
        }
        if levels == 0 {
            return None;
        }
        for &face in faces {
            for step in 0..levels {
                let image = t.images.get(&(face, base_level + step as u8))?;
                let want = (
                    mip(base.width, step),
                    mip(base.height, step),
                    mip_depth(slot, base.depth, step),
                );
                if (image.width, image.height, image.depth) != want
                    || image.format != base.format
                    || image.gpu.is_none()
                {
                    return None;
                }
            }
        }
        // Filtering a format that cannot be filtered: incomplete.
        let nearest = state.mag == NEAREST && matches!(state.min, NEAREST | NEAREST_MIPMAP_NEAREST);
        let storage = base.format.storage;
        let filterable = match formats::info(storage).2 {
            Sample::Sint | Sample::Uint => false,
            Sample::Depth => state.compare_mode == COMPARE_REF_TO_TEXTURE,
            Sample::Float => match storage {
                "rgba32float" | "rg32float" | "r32float" => {
                    self.ext.on("OES_texture_float_linear") && self.features.float32_filterable
                }
                "rgba16float" => self.version == 2 || self.ext.on("OES_texture_half_float_linear"),
                _ => true,
            },
        };
        if !filterable && !nearest {
            return None;
        }
        Some(levels as u8)
    }

    /// A texture unit's texture as a sampler of `want` reads it; none
    /// when there is none or it is not complete (the default texture is
    /// read), Err when it is of the wrong kind (INVALID_OPERATION).
    pub(crate) fn sampled(&mut self, unit: u32, want: Want) -> Result<Option<Sampled>, ()> {
        let Some(id) = self
            .surfaces
            .units
            .get(unit as usize)
            .and_then(|u| u[want.slot])
        else {
            return Ok(None);
        };
        let Some(t) = self.surfaces.textures.get(&id) else {
            return Ok(None);
        };
        let state = match self
            .surfaces
            .sampler_units
            .get(unit as usize)
            .copied()
            .flatten()
        {
            Some(s) => self
                .surfaces
                .samplers
                .get(&s)
                .map_or(t.sampler, |s| s.state),
            None => t.sampler,
        };
        let Some(levels) = self.complete(t, want.slot, &state) else {
            return Ok(None);
        };
        let base_level = t.base_level.min(MAX_LEVEL as u32) as u8;
        let base = t.images[&(0, base_level)].clone();
        let anisotropy = t.anisotropy;
        let storage = base.format.storage;
        let sample = formats::info(storage).2;
        let fits = match (want.shadow, want.sample) {
            (true, _) => sample == Sample::Depth && state.compare_mode == COMPARE_REF_TO_TEXTURE,
            (false, Sample::Float) => matches!(sample, Sample::Float | Sample::Depth),
            (false, s) => s == sample,
        };
        if !fits {
            return Err(());
        }
        let unfilterable = !want.shadow
            && (sample == Sample::Depth
                || (storage.ends_with("32float") && !self.features.float32_filterable));
        let view = if want.slot != SLOT_CUBE && levels == 1 {
            base.gpu.expect("complete").2
        } else {
            self.assemble(id, want.slot, levels)
        };
        let address = |w: u32| match w {
            CLAMP_TO_EDGE => "clamp-to-edge",
            MIRRORED_REPEAT => "mirror-repeat",
            _ => "repeat",
        };
        let filter = |linear: bool| if linear { "linear" } else { "nearest" };
        let flat = unfilterable || matches!(sample, Sample::Sint | Sample::Uint);
        let mag_linear = state.mag == LINEAR && !flat;
        let min_linear = matches!(
            state.min,
            LINEAR | LINEAR_MIPMAP_NEAREST | LINEAR_MIPMAP_LINEAR
        ) && !flat;
        let mip_linear = matches!(state.min, NEAREST_MIPMAP_LINEAR | LINEAR_MIPMAP_LINEAR)
            && levels > 1
            && !flat;
        let anisotropy = if mag_linear && min_linear && (mip_linear || levels == 1) {
            anisotropy.clamp(1.0, 16.0) as u32
        } else {
            1
        };
        let top = f32::from(levels - 1);
        let (lod_min, lod_max) = if levels > 1 {
            let low = state.min_lod.clamp(0.0, top);
            (low, state.max_lod.clamp(0.0, top).max(low))
        } else {
            (0.0, 0.0)
        };
        let compare = if want.shadow {
            format!(
                r#","compare":"{}""#,
                super::draw::compare(state.compare_func)
            )
        } else {
            String::new()
        };
        let desc = format!(
            r#"{{"addressModeU":"{}","addressModeV":"{}","addressModeW":"{}","magFilter":"{}","minFilter":"{}","mipmapFilter":"{}","lodMinClamp":{lod_min},"lodMaxClamp":{lod_max},"maxAnisotropy":{anisotropy}{compare}}}"#,
            address(state.wrap_s),
            address(state.wrap_t),
            address(state.wrap_r),
            filter(mag_linear),
            filter(min_linear),
            filter(mip_linear),
        );
        let sampler = self.sampler_of(desc);
        Ok(Some(Sampled {
            view,
            sampler,
            unfilterable,
            id,
        }))
    }

    /// A sampler of `desc`, made once.
    pub(crate) fn sampler_of(&mut self, desc: String) -> u32 {
        match self.gpu.samplers.get(&desc) {
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
        }
    }

    /// A texture's images put together in one (a cube's faces, a
    /// texture's levels from its base), again when they changed: its
    /// view.
    fn assemble(&mut self, id: u32, slot: usize, levels: u8) -> u32 {
        let t = &self.surfaces.textures[&id];
        let base_level = t.base_level.min(MAX_LEVEL as u32) as u8;
        let base = &t.images[&(0, base_level)];
        let key: Assembly = (
            t.generation,
            u32::from(base_level),
            levels,
            (base.width, base.height, base.depth),
            base.format.storage,
        );
        if let Some((_, view, had)) = t.assembled
            && had == key
        {
            return view;
        }
        let compressed = base.format.compressed();
        let (w, h, d, storage) = (base.width, base.height, base.depth, base.format.storage);
        let old = t.assembled.map(|(a, b, _)| vec![a, b]);
        // What was recorded is sent before the copies go after it.
        self.gpu_flush();
        if let Some(old) = old {
            self.free(old);
        }
        let (texture, view) = (self.gpu.id(&self.client), self.gpu.id(&self.client));
        let third = if slot == SLOT_CUBE { 6 } else { d };
        let (pw, ph) = if compressed {
            (round4(w), round4(h))
        } else {
            (w, h)
        };
        let dimension = if slot == SLOT_3D { "3d" } else { "2d" };
        let integer = matches!(formats::info(storage).2, Sample::Sint | Sample::Uint);
        let view_dimension = match slot {
            // An integer cube is read as its faces' array (glsl).
            SLOT_CUBE if integer => "2d-array",
            SLOT_CUBE => "cube",
            SLOT_3D => "3d",
            SLOT_ARRAY => "2d-array",
            _ => "2d",
        };
        let aspect = if has_stencil(storage) {
            r#","aspect":"depth-only""#
        } else {
            ""
        };
        self.client.queue([
            Request::CreateTexture {
                device: self.device,
                id: texture,
                desc: format!(
                    r#"{{"size":[{pw},{ph},{third}],"format":"{storage}","usage":6,"mipLevelCount":{levels},"dimension":"{dimension}"}}"#
                ),
            },
            Request::CreateView {
                texture,
                id: view,
                desc: format!(r#"{{"dimension":"{view_dimension}"{aspect}}}"#),
            },
        ]);
        let t = &self.surfaces.textures[&id];
        let faces = if slot == SLOT_CUBE { 6 } else { 1 };
        for face in 0..faces as u8 {
            for step in 0..levels {
                let image = &t.images[&(face, base_level + step)];
                let Some((from, _, _)) = image.gpu else {
                    continue;
                };
                let (cw, ch) = if compressed {
                    (round4(image.width), round4(image.height))
                } else {
                    (image.width, image.height)
                };
                let layers = if slot == SLOT_CUBE { 1 } else { image.depth };
                self.gpu.pending.push(Command::CopyTextureToTexture {
                    desc: format!(
                        r#"{{"source":{{"texture":{from}}},"destination":{{"texture":{texture},"mipLevel":{step},"origin":[0,0,{face}]}},"size":[{cw},{ch},{layers}]}}"#
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

    // ---- The blit ----

    /// Draws `extent` texels of `dest` (a view, and a 3D one's slice) at
    /// `at` from `source` (a view, whether 3D, and its slice; of `size`)
    /// at `origin` (texels outside it 0) by `mode`: 0 as they are, 1 RGB
    /// with alpha 1, 2 luminance, 3 alpha, 4 luminance and alpha, 5 the
    /// mean of a 2 by 2 for the next level, 6 red, 7 red and green.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn blit(
        &mut self,
        source: (u32, bool, i32),
        size: (u32, u32),
        origin: (i32, i32),
        dest: (u32, Option<u32>),
        storage: &'static str,
        at: (u32, u32),
        extent: (u32, u32),
        mode: i32,
    ) {
        let device = self.device;
        if self.gpu.blit.is_none() {
            let mut blit = Blit {
                modules: [0; 2],
                layouts: [0; 2],
                pipeline_layouts: [0; 2],
                pipelines: HashMap::new(),
            };
            for (k, (tex, load, dimension)) in [
                ("texture_2d<f32>", "textureLoad(t, s, 0)", "2d"),
                (
                    "texture_3d<f32>",
                    "textureLoad(t, vec3<i32>(s, b.z), 0)",
                    "3d",
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let (module, layout, pipeline_layout) = (
                    self.gpu.id(&self.client),
                    self.gpu.id(&self.client),
                    self.gpu.id(&self.client),
                );
                self.client.queue([
                    Request::CreateShaderModule {
                        device,
                        id: module,
                        code: BLIT.replace("TEX", tex).replace("LOAD", load),
                    },
                    Request::CreateBindGroupLayout {
                        device,
                        id: layout,
                        desc: format!(
                            r#"{{"entries":[{{"binding":0,"visibility":2,"buffer":{{"type":"uniform","hasDynamicOffset":true,"minBindingSize":32}}}},{{"binding":1,"visibility":2,"texture":{{"sampleType":"unfilterable-float","viewDimension":"{dimension}"}}}}]}}"#
                        ),
                    },
                    Request::CreatePipelineLayout {
                        device,
                        id: pipeline_layout,
                        layouts: vec![layout],
                    },
                ]);
                blit.modules[k] = module;
                blit.layouts[k] = layout;
                blit.pipeline_layouts[k] = pipeline_layout;
            }
            self.gpu.blit = Some(blit);
        }
        let k = usize::from(source.1);
        let blit = self.gpu.blit.as_ref().expect("made");
        let (module, layout, pipeline_layout) =
            (blit.modules[k], blit.layouts[k], blit.pipeline_layouts[k]);
        let pipeline = match blit.pipelines.get(&(storage, source.1)) {
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
                    .insert((storage, source.1), id);
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
            source.2,
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
        let key = (layout, vec![source.0]);
        let group = match self.gpu.groups.get(&key) {
            Some(&g) => g,
            None => {
                let g = self.gpu.id(&self.client);
                self.client.queue([Request::CreateBindGroup {
                    device,
                    id: g,
                    desc: format!(
                        r#"{{"layout":{layout},"entries":[{{"binding":0,"buffer":{},"offset":0,"size":32}},{{"binding":1,"view":{}}}]}}"#,
                        self.gpu.arena_id(),
                        source.0
                    ),
                }]);
                self.gpu.groups.insert(key, g);
                g
            }
        };
        let slice = dest
            .1
            .map_or(String::new(), |s| format!(r#","depthSlice":{s}"#));
        self.gpu.pending.push(Command::RenderPass {
            desc: format!(
                r#"{{"colorAttachments":[{{"view":{}{slice},"loadOp":"load","storeOp":"store"}}]}}"#,
                dest.0
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

/// A sampler parameter set: GL's error for a name or value it does not
/// take.
fn set_sampler(s: &mut SamplerState, name: u32, value: f64) -> Result<(), u32> {
    let v = value as u32;
    let filter = |v: u32| {
        matches!(
            v,
            NEAREST
                | LINEAR
                | NEAREST_MIPMAP_NEAREST
                | LINEAR_MIPMAP_NEAREST
                | NEAREST_MIPMAP_LINEAR
                | LINEAR_MIPMAP_LINEAR
        )
    };
    let wrap = |v: u32| matches!(v, REPEAT | CLAMP_TO_EDGE | MIRRORED_REPEAT);
    match name {
        TEXTURE_MIN_FILTER if filter(v) => s.min = v,
        TEXTURE_MAG_FILTER if matches!(v, NEAREST | LINEAR) => s.mag = v,
        TEXTURE_WRAP_S if wrap(v) => s.wrap_s = v,
        TEXTURE_WRAP_T if wrap(v) => s.wrap_t = v,
        TEXTURE_WRAP_R if wrap(v) => s.wrap_r = v,
        TEXTURE_MIN_LOD => s.min_lod = value as f32,
        TEXTURE_MAX_LOD => s.max_lod = value as f32,
        TEXTURE_COMPARE_MODE if matches!(v, NONE | COMPARE_REF_TO_TEXTURE) => s.compare_mode = v,
        TEXTURE_COMPARE_FUNC if (NEVER..=ALWAYS).contains(&v) => s.compare_func = v,
        _ => return Err(INVALID_ENUM),
    }
    Ok(())
}

fn get_sampler(s: &SamplerState, name: u32) -> Option<f64> {
    Some(match name {
        TEXTURE_MIN_FILTER => f64::from(s.min),
        TEXTURE_MAG_FILTER => f64::from(s.mag),
        TEXTURE_WRAP_S => f64::from(s.wrap_s),
        TEXTURE_WRAP_T => f64::from(s.wrap_t),
        TEXTURE_WRAP_R => f64::from(s.wrap_r),
        TEXTURE_MIN_LOD => f64::from(s.min_lod),
        TEXTURE_MAX_LOD => f64::from(s.max_lod),
        TEXTURE_COMPARE_MODE => f64::from(s.compare_mode),
        TEXTURE_COMPARE_FUNC => f64::from(s.compare_func),
        _ => return None,
    })
}

/// The slot getParameter's binding names.
pub(crate) fn binding_slot(name: u32) -> Option<usize> {
    match name {
        TEXTURE_BINDING_2D => Some(SLOT_2D),
        TEXTURE_BINDING_CUBE_MAP => Some(SLOT_CUBE),
        TEXTURE_BINDING_3D => Some(SLOT_3D),
        TEXTURE_BINDING_2D_ARRAY => Some(SLOT_ARRAY),
        _ => None,
    }
}
