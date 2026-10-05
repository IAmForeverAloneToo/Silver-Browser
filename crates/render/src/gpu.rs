//! The GPU backend (M19.3, roadmap D3 and D8): the display lists the
//! software backend draws, drawn by wgpu on the GPU the machine has,
//! and the layers it keeps, kept as textures under the same keys and
//! the same budget.
//!
//! Four pipelines draw everything. Rectangles are instanced quads whose
//! edges the fragment shader anti-aliases by the share of the pixel
//! inside them, the exact area the software backend's rasterizer
//! approximates by supersampling. The quads of a border's bands are
//! plain triangles without anti-aliasing, so that quads sharing an edge
//! tile it, as the GPU's pixel-centre rule tiles them. Glyphs and the
//! mitred polygons of borders are masks over an atlas texture: the
//! glyph masks from the shared rasterizer, and the polygons' coverage
//! from the same exact-area accumulation the software backend blends
//! by, so that text and mitres are the same coverage in both. A layer's
//! or a scratch's texture is composited by a four-by-four matrix with
//! its opacity, sampled bilinearly between texel centres as the
//! software resamples, the projective matrices of 3D by the GPU's
//! perspective-correct interpolation. The clip is the scissor
//! rectangle, rounded for a glyph and a composite as the software
//! rounds them; a rectangle, a quad and a polygon are cut to the clip's
//! geometry before they are drawn, as the software cuts them, so that
//! a fractional clip edge covers a pixel in part on both.
//!
//! A frame is drawn whole, every time, into a texture, and read back
//! into the frame the trait returns, so that the shell's present path
//! stays what it is (D9). Every group that composites, from a layer or
//! from a scratch of its own, is drawn into its texture in a render
//! pass of its own before the pass that draws it, inner groups first,
//! and a frame's passes go to the GPU in one submission. A layer is
//! drawn again whole where its items changed, a tile of a scrolled
//! layer where the damage of the change reaches it, by the same diff
//! the software backend finds its damage with; the rest of a frame's
//! drawing is the GPU's to repeat, which is what it is for.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use paint::{DisplayItem, DisplayList, GroupKey, LayerReason, Rgba8, TILE_HEIGHT};
use style::transform::Matrix;
use text::{FontCollection, FontError, Script};

use crate::{
    Backend, Clip, GlyphCache, Layer, LayerInfo, Layered, Layers, MAX_FRAME_PIXELS,
    MAX_PARTIAL_SIDE, PixelRect, Pixels, Redrawn, RenderBackend, State, Store, Tile, bounds,
    clip_polygon, damaged, fits_a_frame, group_end, outward_clip, polygon_coverage, reach,
    tile_rows, visible_in, whole_reach,
};

/// The side of an atlas page, in texels: four mebibytes of coverage,
/// which holds a page's worth of glyphs many times over.
const PAGE_SIDE: u32 = 2048;

/// How many bytes of atlas pages are held before the atlas starts
/// again, at the start of a frame: the pages are a speed-up, not a
/// store, and a page of a thousand sizes of text would fill them.
const ATLAS_BUDGET: usize = 64 << 20;

/// The floats of one rectangle instance: its quad in clip coordinates,
/// its edges in pixels, and its colour, premultiplied.
const RECT_FLOATS: usize = 12;
/// The floats of one vertex of a quad: its position in clip
/// coordinates and its colour, premultiplied.
const QUAD_FLOATS: usize = 6;
/// The floats of one mask instance: its quad in clip coordinates, its
/// texels in the atlas page, and its colour, premultiplied.
const MASK_FLOATS: usize = 12;
/// The floats of one composite instance: the texture's corners in its
/// own coordinates, the matrix from there to clip coordinates, the
/// texture coordinates of the corners, and the opacity with padding.
const COMPOSITE_FLOATS: usize = 28;

/// The shaders of the four pipelines.
const SHADERS: &str = r#"
struct RectIn {
    @location(0) quad: vec4<f32>,
    @location(1) edges: vec4<f32>,
    @location(2) color: vec4<f32>,
};
struct RectOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) edges: vec4<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
};
fn corner_x(i: u32) -> bool { return i == 1u || i == 3u || i == 4u; }
fn corner_y(i: u32) -> bool { return i == 2u || i == 4u || i == 5u; }
@vertex
fn rect_vs(@builtin(vertex_index) i: u32, r: RectIn) -> RectOut {
    var o: RectOut;
    o.pos = vec4<f32>(select(r.quad.x, r.quad.z, corner_x(i)), select(r.quad.y, r.quad.w, corner_y(i)), 0.0, 1.0);
    o.edges = r.edges;
    o.color = r.color;
    return o;
}
@fragment
fn rect_fs(i: RectOut) -> @location(0) vec4<f32> {
    let p = i.pos.xy;
    let cx = max(0.0, min(i.edges.z, p.x + 0.5) - max(i.edges.x, p.x - 0.5));
    let cy = max(0.0, min(i.edges.w, p.y + 0.5) - max(i.edges.y, p.y - 0.5));
    return i.color * (cx * cy);
}

struct QuadIn {
    @location(0) pos: vec2<f32>,
    @location(1) color: vec4<f32>,
};
struct QuadOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
};
@vertex
fn quad_vs(v: QuadIn) -> QuadOut {
    var o: QuadOut;
    o.pos = vec4<f32>(v.pos, 0.0, 1.0);
    o.color = v.color;
    return o;
}
@fragment
fn quad_fs(i: QuadOut) -> @location(0) vec4<f32> {
    return i.color;
}

@group(0) @binding(0) var sampled: texture_2d<f32>;
@group(0) @binding(1) var sampler_of: sampler;

struct MaskIn {
    @location(0) quad: vec4<f32>,
    @location(1) texels: vec4<f32>,
    @location(2) color: vec4<f32>,
};
struct MaskOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
};
@vertex
fn mask_vs(@builtin(vertex_index) i: u32, m: MaskIn) -> MaskOut {
    var o: MaskOut;
    o.pos = vec4<f32>(select(m.quad.x, m.quad.z, corner_x(i)), select(m.quad.y, m.quad.w, corner_y(i)), 0.0, 1.0);
    o.uv = vec2<f32>(select(m.texels.x, m.texels.z, corner_x(i)), select(m.texels.y, m.texels.w, corner_y(i)));
    o.color = m.color;
    return o;
}
@fragment
fn mask_fs(i: MaskOut) -> @location(0) vec4<f32> {
    return i.color * textureSample(sampled, sampler_of, i.uv).r;
}

struct CompositeIn {
    @location(0) corners: vec4<f32>,
    @location(1) m0: vec4<f32>,
    @location(2) m1: vec4<f32>,
    @location(3) m2: vec4<f32>,
    @location(4) m3: vec4<f32>,
    @location(5) texels: vec4<f32>,
    @location(6) opacity: vec4<f32>,
};
struct CompositeOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) opacity: f32,
};
@vertex
fn composite_vs(@builtin(vertex_index) i: u32, c: CompositeIn) -> CompositeOut {
    var o: CompositeOut;
    let m = mat4x4<f32>(c.m0, c.m1, c.m2, c.m3);
    let x = select(c.corners.x, c.corners.z, corner_x(i));
    let y = select(c.corners.y, c.corners.w, corner_y(i));
    o.pos = m * vec4<f32>(x, y, 0.0, 1.0);
    o.uv = vec2<f32>(select(c.texels.x, c.texels.z, corner_x(i)), select(c.texels.y, c.texels.w, corner_y(i)));
    o.opacity = c.opacity.x;
    return o;
}
@fragment
fn composite_fs(i: CompositeOut) -> @location(0) vec4<f32> {
    return textureSample(sampled, sampler_of, i.uv) * i.opacity;
}
"#;

/// Why the GPU backend could not be made.
#[derive(Debug)]
pub enum GpuError {
    /// No adapter at all: the machine has no GPU the driver exposes,
    /// and no software driver stands in.
    NoAdapter,
    /// The adapter would not give a device.
    Device(String),
    /// No font to draw with.
    Fonts(FontError),
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuError::NoAdapter => write!(f, "no GPU adapter"),
            GpuError::Device(why) => write!(f, "the GPU adapter gave no device: {why}"),
            GpuError::Fonts(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GpuError {}

/// A texture a layer, a scratch, the frame or an atlas page is kept
/// in, with the view a pass draws into and the group a pass samples it
/// by.
struct Texture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind: wgpu::BindGroup,
    width: u32,
    height: u32,
    /// Bytes per texel: four for colour, one for coverage.
    depth: usize,
}

impl Pixels for Texture {
    fn bytes(&self) -> usize {
        self.width as usize * self.height as usize * self.depth
    }
}

/// The pipelines, and what they bind.
struct Pipelines {
    rect: wgpu::RenderPipeline,
    quad: wgpu::RenderPipeline,
    mask: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    /// A texture and its sampler, what the mask and composite
    /// pipelines bind.
    sampled: wgpu::BindGroupLayout,
    /// The sampler of the atlas: the masks are drawn texel for pixel.
    nearest: wgpu::Sampler,
    /// The sampler of a composite: between texel centres, the edge
    /// texels standing in beyond the edges, as the software resamples.
    linear: wgpu::Sampler,
}

/// A render pass recorded, drawn when the frame is flushed: what it
/// draws into, and its draws in order.
struct Pass {
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    draws: Vec<Draw>,
}

/// One draw of a pass: by which pipeline, under which scissor, and
/// which vertices or instances of the pipeline's records.
struct Draw {
    kind: Kind,
    scissor: PixelRect,
    range: Range<u32>,
}

/// A draw's pipeline, with the texture it samples.
#[derive(Clone, PartialEq)]
enum Kind {
    Rect,
    Quad,
    Mask(wgpu::BindGroup),
    Composite(wgpu::BindGroup),
}

/// The vertex and instance records of a frame, one list per pipeline,
/// uploaded when the frame is flushed.
#[derive(Default)]
struct Records {
    rects: Vec<f32>,
    quads: Vec<f32>,
    masks: Vec<f32>,
    composites: Vec<f32>,
}

/// A scratch texture a group is drawn into for one frame, kept for the
/// next frame's group of the same size.
struct Scratch {
    texture: Texture,
    used: u64,
}

/// What names a mask in the atlas: a glyph by its script, id and size
/// bits, scaled unevenly by its two scales' bits too; a polygon by its
/// points' bits, the clip's and the raster's size, which its coverage
/// is cut to.
#[derive(Clone, PartialEq, Eq, Hash)]
enum MaskKey {
    Glyph(Script, u16, u32),
    Scaled(Script, u16, u32, u32, u32),
    Polygon(Vec<(u32, u32)>, [u32; 4], (u32, u32)),
}

/// Where a mask is in the atlas.
#[derive(Clone, Copy)]
struct Place {
    page: usize,
    x: u32,
    y: u32,
}

/// A page of the atlas, filled shelf by shelf: masks go on the shelf
/// from the left, and a mask the shelf has no room for opens the next
/// shelf below, as tall as it is.
struct Page {
    texture: Texture,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
}

impl Page {
    /// Room for a `w` by `h` mask, taken; `None` when the page has
    /// none.
    fn take(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w > self.texture.width || h > self.texture.height {
            return None;
        }
        if self.shelf_x + w <= self.texture.width && h <= self.shelf_h {
            let at = (self.shelf_x, self.shelf_y);
            self.shelf_x += w;
            return Some(at);
        }
        let y = self.shelf_y + self.shelf_h;
        if y + h > self.texture.height {
            return None;
        }
        self.shelf_y = y;
        self.shelf_h = h;
        self.shelf_x = w;
        Some((0, y))
    }
}

/// The atlas of masks: its pages, where each mask is, and the bytes
/// the pages hold.
#[derive(Default)]
struct Atlas {
    pages: Vec<Page>,
    places: HashMap<MaskKey, Place>,
    bytes: usize,
}

impl Atlas {
    fn clear(&mut self) {
        self.pages.clear();
        self.places.clear();
        self.bytes = 0;
    }
}

/// The texture a frame is drawn into, and the buffer it is read back
/// through.
struct Target {
    texture: Texture,
    readback: wgpu::Buffer,
    /// Bytes per row of the readback, the texture's rows padded to the
    /// copy's alignment.
    row: u32,
}

/// The GPU rasterizer: wgpu for geometry, the shared glyph masks for
/// text (M19.3). See the module's documentation for what it draws and
/// how.
pub struct GpuBackend {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// The adapter's name, for the record.
    adapter: String,
    /// The longest side a texture may have.
    max_side: u32,
    pipelines: Pipelines,
    /// The fonts, the shaped runs and the glyph masks.
    text: GlyphCache,
    atlas: Atlas,
    /// The layers (M19), by their groups' keys, under the budget.
    layers: Layers<Texture>,
    scratches: Vec<Scratch>,
    /// Which raster this is, counting them.
    frame: u64,
    layered: Layered,
    /// The groups whose key the last list gave to two groups: neither
    /// is composited from the layer under it, since a layer holds one.
    doubled: HashSet<GroupKey>,
    /// The tiles of the margin not yet drawn, as the layer's key and
    /// the row, with how far the row is from the viewport's middle.
    deferred: Vec<(GroupKey, i32, i32)>,
    /// The passes recorded and not yet drawn, in the order they draw.
    passes: Vec<Pass>,
    records: Records,
    /// The vertex buffers, one per pipeline, grown as frames need.
    buffers: [Option<wgpu::Buffer>; 4],
    target: Option<Target>,
    /// The last frame read back.
    pixels: Vec<u8>,
    /// What the device reported that no error scope caught; said once
    /// each on the standard error, since the trait has no way to say
    /// it, as the shell says what it cannot do.
    errors: Arc<Mutex<Vec<String>>>,
    reported: usize,
}

impl GpuBackend {
    /// Creates the backend on the machine's GPU, or its software
    /// Vulkan driver when it has one, loading a system font plus
    /// whatever per-script fallbacks are available (M8.1).
    pub fn new() -> Result<Self, GpuError> {
        Self::with_fonts(FontCollection::new().map_err(GpuError::Fonts)?)
    }

    /// Creates the backend over fonts already loaded.
    pub fn with_fonts(fonts: FontCollection) -> Result<Self, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::DX12,
            flags: wgpu::InstanceFlags::empty(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .map_err(|_| GpuError::NoAdapter)?;
        let limits = wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("silver"),
            required_features: wgpu::Features::empty(),
            required_limits: limits.clone(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
            ..Default::default()
        }))
        .map_err(|e| GpuError::Device(e.to_string()))?;
        let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = errors.clone();
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            sink.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(error.to_string());
        }));
        let pipelines = Pipelines::new(&device);
        Ok(GpuBackend {
            device,
            queue,
            adapter: adapter.get_info().name,
            max_side: limits.max_texture_dimension_2d.max(1),
            pipelines,
            text: GlyphCache::new(fonts),
            atlas: Atlas::default(),
            layers: Layers::new(),
            scratches: Vec::new(),
            frame: 0,
            layered: Layered::default(),
            doubled: HashSet::new(),
            deferred: Vec::new(),
            passes: Vec::new(),
            records: Records::default(),
            buffers: [None, None, None, None],
            target: None,
            pixels: Vec::new(),
            errors,
            reported: 0,
        })
    }

    /// The name of the adapter the backend draws on.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// The errors the device reported since the backend was made, in
    /// order; none is the expectation.
    pub fn errors(&self) -> Vec<String> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Says on the standard error what the device reported since the
    /// last time.
    fn report(&mut self) {
        let errors = self.errors();
        for error in &errors[self.reported.min(errors.len())..] {
            eprintln!("silver-browser: the GPU backend reported: {error}");
        }
        self.reported = errors.len();
    }

    /// Whether a texture with these corners can be made: finite, no
    /// more pixels than a frame may have, and no side past the
    /// device's longest.
    fn fits(&self, corners: (f32, f32, f32, f32)) -> bool {
        let (x0, y0, x1, y1) = corners;
        fits_a_frame(corners) && x1 - x0 <= self.max_side as f32 && y1 - y0 <= self.max_side as f32
    }

    /// A `width` by `height` texture of `format`, `depth` bytes a
    /// texel, with `usage`, bound with `sampler`; `None` when it cannot
    /// be made.
    fn texture(
        &self,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        depth: usize,
        usage: wgpu::TextureUsages,
        sampler: &wgpu::Sampler,
    ) -> Option<Texture> {
        if width == 0 || height == 0 || width > self.max_side || height > self.max_side {
            return None;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipelines.sampled,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        Some(Texture {
            texture,
            view,
            bind,
            width,
            height,
            depth,
        })
    }

    /// A colour texture a pass draws into and a composite samples.
    fn colour(&self, width: u32, height: u32) -> Option<Texture> {
        self.texture(
            width,
            height,
            wgpu::TextureFormat::Rgba8Unorm,
            4,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            &self.pipelines.linear,
        )
    }

    /// A scratch texture of `width` by `height` for this frame: one
    /// kept from the last frame when there is one of the size, else a
    /// new one. Its view and bind group.
    fn scratch(&mut self, width: u32, height: u32) -> Option<(wgpu::TextureView, wgpu::BindGroup)> {
        let frame = self.frame;
        if let Some(scratch) = self
            .scratches
            .iter_mut()
            .find(|s| s.used < frame && s.texture.width == width && s.texture.height == height)
        {
            scratch.used = frame;
            return Some((scratch.texture.view.clone(), scratch.texture.bind.clone()));
        }
        let texture = self.colour(width, height)?;
        let handles = (texture.view.clone(), texture.bind.clone());
        self.scratches.push(Scratch {
            texture,
            used: frame,
        });
        Some(handles)
    }

    /// Places a `w` by `h` mask of `coverage` in the atlas under `key`,
    /// or finds it there; `None` when there is no room for it.
    fn place(&mut self, key: MaskKey, w: u32, h: u32, coverage: &[u8]) -> Option<Place> {
        if let Some(place) = self.atlas.places.get(&key) {
            return Some(*place);
        }
        if w == 0 || h == 0 || w > self.max_side || h > self.max_side {
            return None;
        }
        let pages = self.atlas.pages.len();
        let mut found = self
            .atlas
            .pages
            .last_mut()
            .and_then(|page| page.take(w, h).map(|at| (pages - 1, at)));
        if found.is_none() {
            let (pw, ph) = (
                PAGE_SIDE.max(w).min(self.max_side),
                PAGE_SIDE.max(h).min(self.max_side),
            );
            let bytes = pw as usize * ph as usize;
            // A frame that asks for more than the budget four times
            // over is drawing text no one can read.
            if self.atlas.bytes + bytes > 4 * ATLAS_BUDGET {
                return None;
            }
            let texture = self.texture(
                pw,
                ph,
                wgpu::TextureFormat::R8Unorm,
                1,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                &self.pipelines.nearest,
            )?;
            let mut page = Page {
                texture,
                shelf_x: 0,
                shelf_y: 0,
                shelf_h: 0,
            };
            let at = page.take(w, h)?;
            self.atlas.pages.push(page);
            self.atlas.bytes += bytes;
            found = Some((self.atlas.pages.len() - 1, at));
        }
        let (index, (x, y)) = found?;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.atlas.pages[index].texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            coverage,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let place = Place { page: index, x, y };
        self.atlas.places.insert(key, place);
        Some(place)
    }

    /// Whether the group `key`, a layer when `layer` says so, is
    /// composited from the layer held under its key.
    fn is_held(&self, key: &GroupKey, layer: Option<LayerReason>) -> bool {
        layer.is_some() && self.layers.contains(key) && !self.doubled.contains(key)
    }

    /// Records the draws of `items` from `state` into a `size` target,
    /// as a whole raster draws them: each through the groups it is in,
    /// a group held as a layer composited from it, and a group that
    /// needs a scratch of its own composited from one.
    fn record_items(
        &mut self,
        draws: &mut Vec<Draw>,
        items: &[DisplayItem],
        mut state: State,
        size: (u32, u32),
    ) {
        let mut i = 0;
        while i < items.len() {
            let item = &items[i];
            if let DisplayItem::PushGroup {
                matrix,
                opacity,
                key,
                layer,
            } = item
                && (State::needs_scratch(matrix, *opacity) || self.is_held(key, *layer))
            {
                let end = group_end(items, i);
                let composed = state.matrix().map_or(*matrix, |m| m.then(matrix));
                let held = self.is_held(key, *layer).then_some(*key);
                self.record_composite(
                    draws,
                    &items[i + 1..end],
                    composed,
                    *opacity,
                    state.clip(),
                    held,
                    size,
                );
                i = end + 1;
                continue;
            }
            let clip = state.clip();
            let through = state.through();
            state.apply(item);
            self.record_item(draws, item, clip, through, size);
            i += 1;
        }
    }

    /// Records the composite of the group `items`, whose coordinates
    /// `matrix` maps to the target's, at `opacity` under `clip`: from
    /// the layer `held` names when one is, else from a scratch drawn
    /// first. When the matrix only translates and scales, the scratch
    /// is in the target's coordinates and the items are drawn through
    /// the matrix into it, and it is laid over the target pixel for
    /// pixel; otherwise the scratch is in the group's own coordinates
    /// and is sampled through the matrix, as the software backend does
    /// both.
    #[allow(clippy::too_many_arguments)]
    fn record_composite(
        &mut self,
        draws: &mut Vec<Draw>,
        items: &[DisplayItem],
        matrix: Matrix,
        opacity: f32,
        clip: Clip,
        held: Option<GroupKey>,
        size: (u32, u32),
    ) {
        let (width, height) = size;
        let Some(within) = clip.pixels(width, height) else {
            return;
        };
        if let Some(key) = held
            && self.record_layer(draws, key, matrix, opacity, within, size)
        {
            return;
        }
        if matrix.translates_and_scales_only() {
            let mut state = State::based(width, height, matrix);
            let mut reach: Option<PixelRect> = None;
            for item in items {
                if let Some(b) = bounds(&mut self.text, item, &state, width, height) {
                    reach = Some(reach.map_or(b, |r| r.union(b)));
                }
                state.apply(item);
            }
            let Some(region) = reach.and_then(|r| r.intersect(within)) else {
                return;
            };
            let (w, h) = (region.x1 - region.x0, region.y1 - region.y0);
            let base =
                Matrix::translate(-(region.x0 as f32), -(region.y0 as f32), 0.0).then(&matrix);
            let Some(bind) = self.render_scratch(items, w, h, base) else {
                return;
            };
            let at = Matrix::translate(region.x0 as f32, region.y0 as f32, 0.0);
            Self::push_composite(
                &mut self.records,
                draws,
                bind,
                (0.0, 0.0, w as f32, h as f32),
                &at,
                opacity,
                region,
                size,
            );
            return;
        }
        let Some(inverse) = matrix.inverse() else {
            return;
        };
        let (wx, wy) = (within.x0 as f32, within.y0 as f32);
        let (ww, wh) = (
            (within.x1 - within.x0) as f32,
            (within.y1 - within.y0) as f32,
        );
        let Some((lx0, ly0, lx1, ly1)) = inverse.bounds(wx, wy, ww, wh) else {
            return;
        };
        let landing = Clip {
            x0: lx0 - 2.0,
            y0: ly0 - 2.0,
            x1: lx1 + 2.0,
            y1: ly1 + 2.0,
        };
        let Some(reach) = reach(&mut self.text, items) else {
            return;
        };
        let mut corners = whole_reach(reach);
        if !self.fits(corners) {
            let cut = reach.intersect(landing);
            if cut.is_empty() {
                return;
            }
            corners = whole_reach(cut);
            if !self.fits(corners) {
                return;
            }
        }
        let (gx0, gy0, gx1, gy1) = corners;
        let (gw, gh) = ((gx1 - gx0) as f64, (gy1 - gy0) as f64);
        if gw < 1.0 || gh < 1.0 {
            return;
        }
        let (gw, gh) = (gw as u32, gh as u32);
        let base = Matrix::translate(-gx0, -gy0, 0.0);
        let Some(bind) = self.render_scratch(items, gw, gh, base) else {
            return;
        };
        Self::push_composite(
            &mut self.records,
            draws,
            bind,
            (gx0, gy0, gx1, gy1),
            &matrix,
            opacity,
            within,
            size,
        );
    }

    /// Draws `items` into a scratch of `width` by `height` whose
    /// coordinates are theirs mapped by `base`, in a pass of its own
    /// before the pass being recorded; the scratch's bind group.
    fn render_scratch(
        &mut self,
        items: &[DisplayItem],
        width: u32,
        height: u32,
        base: Matrix,
    ) -> Option<wgpu::BindGroup> {
        let (view, bind) = self.scratch(width, height)?;
        self.render_pass(view, width, height, items, base);
        Some(bind)
    }

    /// Records a pass that draws `items` into `view`, `width` by
    /// `height`, cleared first, their coordinates mapped by `base`.
    fn render_pass(
        &mut self,
        view: wgpu::TextureView,
        width: u32,
        height: u32,
        items: &[DisplayItem],
        base: Matrix,
    ) {
        let mut draws = Vec::new();
        self.record_items(
            &mut draws,
            items,
            State::based(width, height, base),
            (width, height),
        );
        self.passes.push(Pass {
            view,
            width,
            height,
            draws,
        });
    }

    /// Records the composite of the layer held under `key`, if one is,
    /// by `matrix` at `opacity`, within `within`: a whole layer as one
    /// texture, a layer of tiles each tile the region meets. Whether
    /// one was.
    fn record_layer(
        &mut self,
        draws: &mut Vec<Draw>,
        key: GroupKey,
        matrix: Matrix,
        opacity: f32,
        within: PixelRect,
        size: (u32, u32),
    ) -> bool {
        let frame = self.frame;
        let Some(layer) = self.layers.get_mut(&key) else {
            return false;
        };
        layer.used = frame;
        if matrix.inverse().is_some() {
            match &mut layer.store {
                Store::Whole { pixels, origin } => {
                    let corners = (
                        origin.0,
                        origin.1,
                        origin.0 + pixels.width as f32,
                        origin.1 + pixels.height as f32,
                    );
                    Self::push_composite(
                        &mut self.records,
                        draws,
                        pixels.bind.clone(),
                        corners,
                        &matrix,
                        opacity,
                        within,
                        size,
                    );
                }
                Store::Tiles { left, width, rows } => {
                    let tall = TILE_HEIGHT as f32;
                    for (row, tile) in rows.iter_mut() {
                        let corners = (
                            *left as f32,
                            *row as f32 * tall,
                            (*left + *width as i32) as f32,
                            (*row + 1) as f32 * tall,
                        );
                        let Some((fx0, fy0, fx1, fy1)) = matrix.bounds(
                            corners.0,
                            corners.1,
                            corners.2 - corners.0,
                            corners.3 - corners.1,
                        ) else {
                            continue;
                        };
                        if PixelRect::around(
                            fx0 - 1.0,
                            fy0 - 1.0,
                            fx1 + 1.0,
                            fy1 + 1.0,
                            size.0,
                            size.1,
                        )
                        .and_then(|r| r.intersect(within))
                        .is_none()
                        {
                            continue;
                        }
                        tile.used = frame;
                        Self::push_composite(
                            &mut self.records,
                            draws,
                            tile.pixels.bind.clone(),
                            corners,
                            &matrix,
                            opacity,
                            within,
                            size,
                        );
                    }
                }
            }
        }
        if !self.layered.composited.contains(&key) {
            self.layered.composited.push(key);
        }
        true
    }

    /// Records a composite instance: the texture `bind` samples, whose
    /// texels cover `corners` in its own coordinates, drawn by `matrix`
    /// at `opacity` under the scissor `scissor` into a `size` target.
    #[allow(clippy::too_many_arguments)]
    fn push_composite(
        records: &mut Records,
        draws: &mut Vec<Draw>,
        bind: wgpu::BindGroup,
        corners: (f32, f32, f32, f32),
        matrix: &Matrix,
        opacity: f32,
        scissor: PixelRect,
        size: (u32, u32),
    ) {
        let index = (records.composites.len() / COMPOSITE_FLOATS) as u32;
        records
            .composites
            .extend([corners.0, corners.1, corners.2, corners.3]);
        records.composites.extend(clip_matrix(matrix, size));
        records
            .composites
            .extend([0.0, 0.0, 1.0, 1.0, opacity, 0.0, 0.0, 0.0]);
        draws.push(Draw {
            kind: Kind::Composite(bind),
            scissor,
            range: index..index + 1,
        });
    }

    /// Records one draw's records, merged into the last draw when it
    /// is of the same kind under the same scissor.
    fn push_draw(draws: &mut Vec<Draw>, kind: Kind, scissor: PixelRect, range: Range<u32>) {
        if let Some(last) = draws.last_mut()
            && last.kind == kind
            && last.scissor == scissor
            && last.range.end == range.start
        {
            last.range.end = range.end;
            return;
        }
        draws.push(Draw {
            kind,
            scissor,
            range,
        });
    }

    /// Records one item under `clip`, through the scale and
    /// translation `(sx, sy, tx, ty)` of the groups it is in, into a
    /// `size` target, as the software backend draws it.
    fn record_item(
        &mut self,
        draws: &mut Vec<Draw>,
        item: &DisplayItem,
        clip: Clip,
        (sx, sy, tx, ty): (f32, f32, f32, f32),
        size: (u32, u32),
    ) {
        let point = |(x, y): (f32, f32)| (x * sx + tx, y * sy + ty);
        let (width, height) = size;
        // The pixels a rectangle, a quad or a polygon cut to the clip's
        // geometry can reach: the clip's bounds, outwards.
        let Some(outer) = PixelRect::around(clip.x0, clip.y0, clip.x1, clip.y1, width, height)
        else {
            return;
        };
        match item {
            DisplayItem::PushClip { .. }
            | DisplayItem::PopClip
            | DisplayItem::PushGroup { .. }
            | DisplayItem::PopGroup => {}
            DisplayItem::Rect { x, y, w, h, color } => {
                let r = Clip::new(x * sx + tx, y * sy + ty, w * sx, h * sy).intersect(clip);
                if r.is_empty() || ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) {
                    return;
                }
                let Some(quad) = PixelRect::around(r.x0, r.y0, r.x1, r.y1, width, height) else {
                    return;
                };
                let n = ndc(size);
                let (ax, ay) = n(quad.x0 as f32, quad.y0 as f32);
                let (bx, by) = n(quad.x1 as f32, quad.y1 as f32);
                let index = (self.records.rects.len() / RECT_FLOATS) as u32;
                self.records
                    .rects
                    .extend([ax, ay, bx, by, r.x0, r.y0, r.x1, r.y1]);
                self.records.rects.extend(premultiplied(color));
                Self::push_draw(draws, Kind::Rect, outer, index..index + 1);
            }
            DisplayItem::Text {
                text,
                x,
                y,
                size: font_size,
                color,
            } => {
                if clip.is_empty() {
                    return;
                }
                self.record_text(
                    draws,
                    text,
                    point((*x, *y)),
                    *font_size,
                    color,
                    clip,
                    (sx, sy),
                    size,
                );
            }
            DisplayItem::Quad { points, color } => {
                let polygon = clip_polygon(&points.map(point), clip);
                if polygon.len() < 3 || polygon.iter().any(|p| !p.0.is_finite() || !p.1.is_finite())
                {
                    return;
                }
                let n = ndc(size);
                let colour = premultiplied(color);
                let first = (self.records.quads.len() / QUAD_FLOATS) as u32;
                for i in 1..polygon.len() - 1 {
                    for p in [polygon[0], polygon[i], polygon[i + 1]] {
                        let (px, py) = n(p.0, p.1);
                        self.records.quads.extend([px, py]);
                        self.records.quads.extend(colour);
                    }
                }
                let last = (self.records.quads.len() / QUAD_FLOATS) as u32;
                Self::push_draw(draws, Kind::Quad, outer, first..last);
            }
            DisplayItem::Polygon { points, color } => {
                let points: Vec<(f32, f32)> = points.iter().map(|p| point(*p)).collect();
                let Some(coverage) = polygon_coverage(&points, clip, size) else {
                    return;
                };
                let key = MaskKey::Polygon(
                    points
                        .iter()
                        .map(|p| (p.0.to_bits(), p.1.to_bits()))
                        .collect(),
                    [
                        clip.x0.to_bits(),
                        clip.y0.to_bits(),
                        clip.x1.to_bits(),
                        clip.y1.to_bits(),
                    ],
                    size,
                );
                let (w, h) = (coverage.width as u32, coverage.height as u32);
                let bytes: Vec<u8> = coverage
                    .values
                    .iter()
                    .map(|v| (v * 255.0).round() as u8)
                    .collect();
                let Some(place) = self.place(key, w, h, &bytes) else {
                    return;
                };
                let at = (
                    coverage.x0,
                    coverage.y0,
                    coverage.x0 + w as i64,
                    coverage.y0 + h as i64,
                );
                self.push_mask(draws, place, at, premultiplied(color), outer, size);
            }
        }
    }

    /// Records the glyphs of `text` with the baseline's left end at
    /// `origin`, scaled by `scale`, as the software backend draws them:
    /// each glyph's mask placed from the scaled origin at a whole
    /// pixel, drawn where the clip's pixels hold it.
    #[allow(clippy::too_many_arguments)]
    fn record_text(
        &mut self,
        draws: &mut Vec<Draw>,
        text: &str,
        (x, y): (f32, f32),
        font_size: f32,
        color: &Rgba8,
        clip: Clip,
        (sx, sy): (f32, f32),
        size: (u32, u32),
    ) {
        let (width, height) = size;
        let Some(scissor) = clip.pixels(width, height) else {
            return;
        };
        let colour = premultiplied(color);
        let glyphs = self.text.shaped(text, font_size);
        for (script, g) in glyphs {
            let (gx, gy) = (x + g.x * sx, y + g.y * sy);
            let (key, mask) = if sx == sy {
                let at = font_size * sx;
                (
                    MaskKey::Glyph(script, g.glyph_id, at.to_bits()),
                    self.text.mask_for(script, g.glyph_id, at),
                )
            } else {
                (
                    MaskKey::Scaled(
                        script,
                        g.glyph_id,
                        font_size.to_bits(),
                        sx.to_bits(),
                        sy.to_bits(),
                    ),
                    self.text
                        .scaled_mask(script, g.glyph_id, font_size, (sx, sy)),
                )
            };
            if mask.width == 0 || mask.height == 0 {
                continue;
            }
            let (w, h) = (mask.width as i64, mask.height as i64);
            let ox = (gx + mask.left as f32).floor() as i64;
            let oy = (gy + mask.top as f32).floor() as i64;
            if ox >= scissor.x1 as i64
                || oy >= scissor.y1 as i64
                || ox + w <= scissor.x0 as i64
                || oy + h <= scissor.y0 as i64
            {
                continue;
            }
            let (mw, mh, coverage) = (mask.width, mask.height, mask.coverage.clone());
            let Some(place) = self.place(key, mw, mh, &coverage) else {
                continue;
            };
            self.push_mask(
                draws,
                place,
                (ox, oy, ox + w, oy + h),
                colour,
                scissor,
                size,
            );
        }
    }

    /// Records a mask instance: the mask at `place` in the atlas,
    /// drawn texel for pixel at the pixels `(x0, y0, x1, y1)` of a
    /// `size` target, which may begin before the target, with `colour`,
    /// under `scissor`.
    fn push_mask(
        &mut self,
        draws: &mut Vec<Draw>,
        place: Place,
        (x0, y0, x1, y1): (i64, i64, i64, i64),
        colour: [f32; 4],
        scissor: PixelRect,
        size: (u32, u32),
    ) {
        let page = &self.atlas.pages[place.page].texture;
        let (pw, ph) = (page.width as f32, page.height as f32);
        let n = ndc(size);
        let (ax, ay) = n(x0 as f32, y0 as f32);
        let (bx, by) = n(x1 as f32, y1 as f32);
        let (w, h) = ((x1 - x0) as f32, (y1 - y0) as f32);
        let (u0, v0) = (place.x as f32 / pw, place.y as f32 / ph);
        let (u1, v1) = ((place.x as f32 + w) / pw, (place.y as f32 + h) / ph);
        let index = (self.records.masks.len() / MASK_FLOATS) as u32;
        self.records.masks.extend([ax, ay, bx, by, u0, v0, u1, v1]);
        self.records.masks.extend(colour);
        Self::push_draw(
            draws,
            Kind::Mask(page.bind.clone()),
            scissor,
            index..index + 1,
        );
    }

    /// Brings the layers of the groups among `items` up to date, as
    /// the software backend does: a group that is a layer is drawn
    /// into its layer where its own items changed, inner groups first,
    /// and a layer the budget has no room for is let go. `extra`
    /// collects, in the coordinates of the layer being walked into,
    /// the rectangles whose pixels change for reasons the lists cannot
    /// tell: a group held as a layer that was not before, or the other
    /// way.
    fn update_layers(
        &mut self,
        items: &[DisplayItem],
        frame: &State,
        local: &State,
        seen: &mut HashSet<GroupKey>,
        extra: &mut Vec<Clip>,
    ) {
        let mut frame = frame.clone();
        let mut local = local.clone();
        let mut i = 0;
        while i < items.len() {
            let item = &items[i];
            if let DisplayItem::PushGroup {
                matrix,
                key,
                layer: Some(reason),
                ..
            } = item
            {
                let end = group_end(items, i);
                let inner = &items[i + 1..end];
                let composed = frame.matrix().map_or(*matrix, |m| m.then(matrix));
                let outward = local.matrix().map_or(*matrix, |m| m.then(matrix));
                let clip = frame.clip();
                let mut inner_frame = frame.clone();
                inner_frame.apply(item);
                let mut inner_extra = Vec::new();
                self.update_layers(
                    inner,
                    &inner_frame,
                    &State::unbounded(),
                    seen,
                    &mut inner_extra,
                );
                let was = self.layers.contains(key) && !self.doubled.contains(key);
                let held = if seen.insert(*key) {
                    self.update_layer(*key, *reason, inner, &composed, clip, &inner_extra)
                } else {
                    self.layers.drop(*key);
                    self.doubled.insert(*key);
                    false
                };
                if was != held
                    && let Some(reach) = reach(&mut self.text, inner)
                    && let Some(mapped) = outward_clip(&outward, reach)
                {
                    extra.push(mapped);
                }
                for r in inner_extra {
                    if let Some(mapped) = outward_clip(&outward, r) {
                        extra.push(mapped);
                    }
                }
                i = end + 1;
                continue;
            }
            frame.apply(item);
            local.apply(item);
            i += 1;
        }
    }

    /// Brings the layer of the group `key` up to date with `items`,
    /// which `composed` maps to the frame's coordinates, where `clip`
    /// is in force, `extra` being rectangles of it whose pixels changed
    /// for reasons its lists cannot tell. Whether the layer is held.
    fn update_layer(
        &mut self,
        key: GroupKey,
        reason: LayerReason,
        items: &[DisplayItem],
        composed: &Matrix,
        clip: Clip,
        extra: &[Clip],
    ) -> bool {
        let Some(visible) = visible_in(composed, clip) else {
            self.layers.drop(key);
            return false;
        };
        let mut old = self.layers.remove(&key);
        if old.as_ref().is_some_and(|layer| layer.reason != reason) {
            old = None;
        }
        let kept = match reason {
            LayerReason::Scrolled => self.update_tiles(key, reason, old, items, visible, extra),
            LayerReason::Transformed | LayerReason::WillChange => {
                self.update_whole(key, reason, old, items, extra)
            }
        };
        match kept {
            Some(layer) => {
                self.layers.insert(key, layer);
                true
            }
            None => false,
        }
    }

    /// The layer of the group `key` kept whole: one texture of the
    /// items' reach, a clear texel all round, drawn again whole when
    /// the items changed since `old`, whose texture is kept when the
    /// reach is where it was. `None` when the items draw nothing, or
    /// the budget has no room.
    fn update_whole(
        &mut self,
        key: GroupKey,
        reason: LayerReason,
        old: Option<Layer<Texture>>,
        items: &[DisplayItem],
        extra: &[Clip],
    ) -> Option<Layer<Texture>> {
        let reach = reach(&mut self.text, items)?;
        let corners = whole_reach(reach);
        if !self.fits(corners) {
            return None;
        }
        let (gx0, gy0, gx1, gy1) = corners;
        let (gw, gh) = ((gx1 - gx0) as f64, (gy1 - gy0) as f64);
        if gw < 1.0 || gh < 1.0 {
            return None;
        }
        let (gw, gh) = (gw as u32, gh as u32);
        let origin = (gx0, gy0);
        let base = Matrix::translate(-gx0, -gy0, 0.0);
        if let Some(Layer {
            store: Store::Whole {
                pixels,
                origin: was,
            },
            items: were,
            ..
        }) = old
            && was == origin
            && pixels.width == gw
            && pixels.height == gh
        {
            if were != items || !extra.is_empty() {
                self.render_pass(pixels.view.clone(), gw, gh, items, base);
                self.layered.rasterized.push((key, 1));
            }
            return Some(Layer {
                reason,
                items: items.to_vec(),
                store: Store::Whole { pixels, origin },
                used: self.frame,
            });
        }
        if !self
            .layers
            .reserve(gw as usize * gh as usize * 4, 0, self.frame)
        {
            return None;
        }
        let pixels = self.colour(gw, gh)?;
        self.render_pass(pixels.view.clone(), gw, gh, items, base);
        self.layered.rasterized.push((key, 1));
        Some(Layer {
            reason,
            items: items.to_vec(),
            store: Store::Whole { pixels, origin },
            used: self.frame,
        })
    }

    /// The layer of the scrolled group `key` kept as tiles, as the
    /// software backend keeps them: strips as wide as what of the group
    /// can show, `visible`, the tiles that meet it drawn now, those
    /// within a `visible`'s height above and below it left for the
    /// time after the frame, and those further off let go. The tiles
    /// of `old` are kept when the strips are where they were, each
    /// drawn again whole when the damage of the change reaches it.
    /// `None` when the budget has no room for a tile the frame shows.
    fn update_tiles(
        &mut self,
        key: GroupKey,
        reason: LayerReason,
        old: Option<Layer<Texture>>,
        items: &[DisplayItem],
        (vx0, vy0, vx1, vy1): (f32, f32, f32, f32),
        extra: &[Clip],
    ) -> Option<Layer<Texture>> {
        let (left, right) = (vx0.floor(), vx1.ceil());
        if !(left.is_finite() && right.is_finite())
            || right - left < 1.0
            || right - left > MAX_PARTIAL_SIDE as f32
            || right - left > self.max_side as f32
            || left.abs() > 1e8
        {
            return None;
        }
        let (left, width) = (left as i32, (right - left) as u32);
        let tall = TILE_HEIGHT;
        let now = tile_rows(vy0, vy1);
        let span = vy1 - vy0;
        let margin = tile_rows(vy0 - span, vy1 + span);
        let tile_bytes = width as usize * tall as usize * 4;
        let base = |row: i32| Matrix::translate(-(left as f32), -(row as f32 * tall as f32), 0.0);
        let mut rows: BTreeMap<i32, Tile<Texture>> = BTreeMap::new();
        let mut drawn = 0u32;
        if let Some(Layer {
            store:
                Store::Tiles {
                    left: had_left,
                    width: had_width,
                    rows: had,
                },
            items: were,
            ..
        }) = old
            && had_left == left
            && had_width == width
        {
            rows = had
                .into_iter()
                .filter(|(row, _)| margin.contains(row))
                .collect();
            if (were != items || !extra.is_empty()) && !rows.is_empty() {
                let first = *rows.keys().next().expect("a row");
                let last = *rows.keys().next_back().expect("a row");
                let height = ((last - first + 1) as u32).saturating_mul(tall);
                let top = first as f32 * tall as f32;
                let mut rects = damaged(
                    &mut self.text,
                    &were,
                    items,
                    width,
                    height,
                    Some(base(first)),
                );
                rects.extend(extra.iter().filter_map(|r| {
                    PixelRect::around(
                        r.x0 - left as f32,
                        r.y0 - top,
                        r.x1 - left as f32,
                        r.y1 - top,
                        width,
                        height,
                    )
                }));
                let mut touched: Vec<i32> = Vec::new();
                for rect in rects {
                    for offset in tile_rows(rect.y0 as f32, rect.y1 as f32) {
                        let row = first + offset;
                        if rows.contains_key(&row) && !touched.contains(&row) {
                            touched.push(row);
                        }
                    }
                }
                for row in touched {
                    let view = rows[&row].pixels.view.clone();
                    self.render_pass(view, width, tall, items, base(row));
                    drawn += 1;
                }
            }
        }
        let mut held: usize = rows.values().map(|t| t.pixels.bytes()).sum();
        for row in now.clone() {
            if let Some(tile) = rows.get_mut(&row) {
                tile.used = self.frame;
                continue;
            }
            if !self.layers.reserve(tile_bytes, held, self.frame) {
                return None;
            }
            let pixels = self.colour(width, tall)?;
            self.render_pass(pixels.view.clone(), width, tall, items, base(row));
            held += tile_bytes;
            drawn += 1;
            rows.insert(
                row,
                Tile {
                    pixels,
                    used: self.frame,
                },
            );
        }
        let middle = (*now.start() as i64 + *now.end() as i64) / 2;
        for row in margin {
            if !rows.contains_key(&row) {
                self.deferred.push((
                    key,
                    row,
                    (row as i64 - middle).unsigned_abs().min(i32::MAX as u64) as i32,
                ));
            }
        }
        if drawn > 0 {
            self.layered.rasterized.push((key, drawn));
        }
        Some(Layer {
            reason,
            items: items.to_vec(),
            store: Store::Tiles { left, width, rows },
            used: self.frame,
        })
    }

    /// Draws the tiles of the margin not yet drawn, nearest the
    /// viewport first, each submitted as it is recorded, while there
    /// is time before `deadline` and room under the budget without
    /// letting anything go; the rest wait for a frame that shows them.
    fn prepaint(&mut self, deadline: Option<Instant>) {
        let Some(deadline) = deadline else {
            self.deferred.clear();
            return;
        };
        let mut deferred = std::mem::take(&mut self.deferred);
        deferred.sort_by_key(|(_, _, distance)| *distance);
        for (key, row, _) in deferred {
            if Instant::now() >= deadline {
                break;
            }
            let Some(mut layer) = self.layers.remove(&key) else {
                continue;
            };
            if let Store::Tiles { left, width, rows } = &mut layer.store
                && !rows.contains_key(&row)
            {
                let (left, width) = (*left, *width);
                let tile_bytes = width as usize * TILE_HEIGHT as usize * 4;
                if self.layers.bytes() + layer.bytes() + tile_bytes > self.layers.budget() {
                    self.layers.insert(key, layer);
                    break;
                }
                if let Some(pixels) = self.colour(width, TILE_HEIGHT) {
                    let base =
                        Matrix::translate(-(left as f32), -(row as f32 * TILE_HEIGHT as f32), 0.0);
                    let items = std::mem::take(&mut layer.items);
                    self.render_pass(pixels.view.clone(), width, TILE_HEIGHT, &items, base);
                    layer.items = items;
                    self.flush(false);
                    if let Store::Tiles { rows, .. } = &mut layer.store {
                        rows.insert(
                            row,
                            Tile {
                                pixels,
                                used: self.frame,
                            },
                        );
                    }
                    self.layered.prepainted += 1;
                }
            }
            self.layers.insert(key, layer);
        }
    }

    /// The frame's target of `width` by `height`, made anew at a new
    /// size; its view.
    fn target(&mut self, width: u32, height: u32) -> Option<wgpu::TextureView> {
        if let Some(target) = &self.target
            && target.texture.width == width
            && target.texture.height == height
        {
            return Some(target.texture.view.clone());
        }
        self.target = None;
        let texture = self.colour(width, height)?;
        let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: row as u64 * height as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view = texture.view.clone();
        self.target = Some(Target {
            texture,
            readback,
            row,
        });
        Some(view)
    }

    /// Uploads the records, draws the passes recorded, in order, in
    /// one submission, and, when `readback` is asked, copies the
    /// frame's texture into `pixels`.
    fn flush(&mut self, readback: bool) {
        let records = std::mem::take(&mut self.records);
        let lists = [
            &records.rects,
            &records.quads,
            &records.masks,
            &records.composites,
        ];
        for (buffer, data) in self.buffers.iter_mut().zip(lists) {
            if data.is_empty() {
                continue;
            }
            let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_ne_bytes()).collect();
            let needed = bytes.len() as u64;
            if buffer.as_ref().is_none_or(|b| b.size() < needed) {
                let size = needed.next_power_of_two().max(1 << 16);
                *buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("records"),
                    size,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            }
            if let Some(buffer) = buffer {
                self.queue.write_buffer(buffer, 0, &bytes);
            }
        }
        let passes = std::mem::take(&mut self.passes);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        for pass in &passes {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &pass.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                ..Default::default()
            });
            for draw in &pass.draws {
                let s = draw.scissor;
                if s.x1 <= s.x0 || s.y1 <= s.y0 || s.x1 > pass.width || s.y1 > pass.height {
                    continue;
                }
                rp.set_scissor_rect(s.x0, s.y0, s.x1 - s.x0, s.y1 - s.y0);
                let (pipeline, buffer) = match &draw.kind {
                    Kind::Rect => (&self.pipelines.rect, &self.buffers[0]),
                    Kind::Quad => (&self.pipelines.quad, &self.buffers[1]),
                    Kind::Mask(bind) => {
                        rp.set_bind_group(0, bind, &[]);
                        (&self.pipelines.mask, &self.buffers[2])
                    }
                    Kind::Composite(bind) => {
                        rp.set_bind_group(0, bind, &[]);
                        (&self.pipelines.composite, &self.buffers[3])
                    }
                };
                let Some(buffer) = buffer else {
                    continue;
                };
                rp.set_pipeline(pipeline);
                rp.set_vertex_buffer(0, buffer.slice(..));
                match draw.kind {
                    Kind::Quad => rp.draw(draw.range.clone(), 0..1),
                    _ => rp.draw(0..6, draw.range.clone()),
                }
            }
        }
        if readback && let Some(target) = &self.target {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target.texture.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &target.readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(target.row),
                        rows_per_image: Some(target.texture.height),
                    },
                },
                wgpu::Extent3d {
                    width: target.texture.width,
                    height: target.texture.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.queue.submit([encoder.finish()]);
        if readback && let Some(target) = &self.target {
            let (width, height) = (
                target.texture.width as usize,
                target.texture.height as usize,
            );
            let slice = target.readback.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            let mapped = self
                .device
                .poll(wgpu::PollType::wait_indefinitely())
                .is_ok()
                && rx.recv().is_ok_and(|r| r.is_ok());
            if mapped && let Ok(data) = slice.get_mapped_range() {
                let row = target.row as usize;
                self.pixels.clear();
                self.pixels.reserve(width * height * 4);
                for y in 0..height {
                    self.pixels
                        .extend_from_slice(&data[y * row..y * row + width * 4]);
                }
            }
            target.readback.unmap();
        }
    }
}

impl Pipelines {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("silver"),
            source: wgpu::ShaderSource::Wgsl(SHADERS.into()),
        });
        let sampled = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sampled"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let plain = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let textured = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&sampled)],
            immediate_size: 0,
        });
        let float4 = |location: u32, offset: u64| wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset,
            shader_location: location,
        };
        let pipeline = |label: &str,
                        layout: &wgpu::PipelineLayout,
                        vs: &str,
                        fs: &str,
                        stride: u64,
                        step_mode: wgpu::VertexStepMode,
                        attributes: &[wgpu::VertexAttribute]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: stride,
                        step_mode,
                        attributes,
                    })],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let rect = pipeline(
            "rect",
            &plain,
            "rect_vs",
            "rect_fs",
            RECT_FLOATS as u64 * 4,
            wgpu::VertexStepMode::Instance,
            &[float4(0, 0), float4(1, 16), float4(2, 32)],
        );
        let quad = pipeline(
            "quad",
            &plain,
            "quad_vs",
            "quad_fs",
            QUAD_FLOATS as u64 * 4,
            wgpu::VertexStepMode::Vertex,
            &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                },
                float4(1, 8),
            ],
        );
        let mask = pipeline(
            "mask",
            &textured,
            "mask_vs",
            "mask_fs",
            MASK_FLOATS as u64 * 4,
            wgpu::VertexStepMode::Instance,
            &[float4(0, 0), float4(1, 16), float4(2, 32)],
        );
        let composite = pipeline(
            "composite",
            &textured,
            "composite_vs",
            "composite_fs",
            COMPOSITE_FLOATS as u64 * 4,
            wgpu::VertexStepMode::Instance,
            &[
                float4(0, 0),
                float4(1, 16),
                float4(2, 32),
                float4(3, 48),
                float4(4, 64),
                float4(5, 80),
                float4(6, 96),
            ],
        );
        let sampler = |filter: wgpu::FilterMode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: None,
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            })
        };
        Pipelines {
            rect,
            quad,
            mask,
            composite,
            sampled,
            nearest: sampler(wgpu::FilterMode::Nearest),
            linear: sampler(wgpu::FilterMode::Linear),
        }
    }
}

/// From a `size` target's pixels to clip coordinates: `x` across from
/// -1 to 1, `y` down from 1 to -1.
fn ndc((width, height): (u32, u32)) -> impl Fn(f32, f32) -> (f32, f32) {
    let (ax, ay) = (2.0 / width as f32, 2.0 / height as f32);
    move |x: f32, y: f32| (x * ax - 1.0, 1.0 - y * ay)
}

/// `matrix`, from a group's coordinates to a `size` target's pixels,
/// followed by the pixels' mapping to clip coordinates, column-major
/// as the shader's `mat4x4` takes it: `z` is dropped, and `w` kept, so
/// that a projective matrix is interpolated perspective-correctly.
fn clip_matrix(matrix: &Matrix, (width, height): (u32, u32)) -> [f32; 16] {
    let (ax, ay) = (2.0 / width as f32, 2.0 / height as f32);
    let mut out = [0.0; 16];
    for col in 0..4 {
        let (r0, r1, r3) = (matrix.at(0, col), matrix.at(1, col), matrix.at(3, col));
        out[col * 4] = ax * r0 - r3;
        out[col * 4 + 1] = r3 - ay * r1;
        out[col * 4 + 3] = r3;
    }
    out
}

/// A colour as the shaders take it: premultiplied, in `[0, 1]`.
fn premultiplied(color: &Rgba8) -> [f32; 4] {
    let a = color.a as f32 / 255.0;
    [
        color.r as f32 / 255.0 * a,
        color.g as f32 / 255.0 * a,
        color.b as f32 / 255.0 * a,
        a,
    ]
}

impl RenderBackend for GpuBackend {
    fn backend(&self) -> Backend {
        Backend::Gpu
    }

    fn rasterize(&mut self, list: &DisplayList, width: u32, height: u32) -> &[u8] {
        self.rasterize_by(list, width, height, None)
    }

    fn rasterize_by(
        &mut self,
        list: &DisplayList,
        width: u32,
        height: u32,
        deadline: Option<Instant>,
    ) -> &[u8] {
        self.frame += 1;
        self.layered = Layered::default();
        self.deferred.clear();
        self.doubled.clear();
        self.passes.clear();
        self.records = Records::default();
        let frame = self.frame;
        self.scratches.retain(|s| s.used + 1 >= frame);
        if self.atlas.bytes > ATLAS_BUDGET {
            self.atlas.clear();
        }
        if width as u64 * height as u64 > MAX_FRAME_PIXELS
            || width > self.max_side
            || height > self.max_side
        {
            self.pixels.clear();
            return &self.pixels;
        }
        let Some(view) = self.target(width, height) else {
            self.pixels.clear();
            return &self.pixels;
        };
        // The layers first: this frame's are marked, so that the budget
        // never lets one go for another, then each is drawn where its
        // items changed; the frame itself is drawn whole.
        let base = State::new(width, height);
        self.layers.mark(&list.items, &base, self.frame);
        let mut seen = HashSet::new();
        let mut extra = Vec::new();
        self.update_layers(&list.items, &base, &base, &mut seen, &mut extra);
        let mut draws = Vec::new();
        self.record_items(&mut draws, &list.items, base, (width, height));
        self.passes.push(Pass {
            view,
            width,
            height,
            draws,
        });
        self.flush(true);
        self.prepaint(deadline);
        self.layers.trim();
        self.report();
        &self.pixels
    }

    fn set_layer_budget(&mut self, bytes: usize) {
        self.layers.set_budget(bytes);
    }

    fn layer_bytes(&self) -> usize {
        self.layers.bytes()
    }

    fn layers(&self) -> Vec<LayerInfo> {
        self.layers.info()
    }

    fn layered(&self) -> Layered {
        self.layered.clone()
    }

    fn clear_glyph_cache(&mut self) {
        self.text.clear();
    }

    fn forget_frame(&mut self) {
        self.layers.clear();
        self.deferred.clear();
    }

    fn redrawn(&self) -> &Redrawn {
        static WHOLE: Redrawn = Redrawn::Whole;
        &WHOLE
    }
}

/// How far a channel may differ, between the two backends, at a pixel
/// inside a shape: the rounding of a blend.
pub const INSIDE_TOLERANCE: u8 = 2;

/// How far a channel may differ at a pixel on an edge: the difference
/// between the software rasterizer's supersampled coverage and the
/// shader's exact area, and between its own anti-aliasing and the
/// GPU's bilinear sampling at a layer's edge.
pub const EDGE_TOLERANCE: u8 = 64;

/// What comparing two frames found (see [`compare`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    /// How many pixels differ past their tolerance.
    pub differing: usize,
    /// The first of them, as `(x, y, reference, other)`.
    pub first: Option<(u32, u32, [u8; 4], [u8; 4])>,
    /// The rectangle around them all, as `(x0, y0, x1, y1)` inclusive.
    pub bounds: Option<(u32, u32, u32, u32)>,
    /// How many pixels differ within their tolerance.
    pub tolerated: usize,
    /// The reference, dimmed, with the differing pixels in red, RGB.
    pub diff: Vec<u8>,
}

/// Compares `other` with `reference`, two `width` by `height` RGBA8
/// frames, under the equivalence rule of M19.3: a pixel whose eight
/// neighbours in the reference are all its own colour, the inside of a
/// shape, must match within [`INSIDE_TOLERANCE`] a channel, and any
/// other pixel, on an edge, within [`EDGE_TOLERANCE`].
pub fn compare(reference: &[u8], other: &[u8], width: u32, height: u32) -> Comparison {
    let (w, h) = (width as usize, height as usize);
    let px = |frame: &[u8], x: usize, y: usize| -> [u8; 4] {
        let i = (y * w + x) * 4;
        [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
    };
    let mut out = Comparison {
        differing: 0,
        first: None,
        bounds: None,
        tolerated: 0,
        diff: Vec::with_capacity(w * h * 3),
    };
    if reference.len() != w * h * 4 || other.len() != w * h * 4 {
        out.differing = usize::MAX;
        return out;
    }
    for y in 0..h {
        for x in 0..w {
            let r = px(reference, x, y);
            let o = px(other, x, y);
            let inside = (y.saturating_sub(1)..(y + 2).min(h))
                .flat_map(|ny| (x.saturating_sub(1)..(x + 2).min(w)).map(move |nx| (nx, ny)))
                .all(|(nx, ny)| px(reference, nx, ny) == r);
            let tolerance = if inside {
                INSIDE_TOLERANCE
            } else {
                EDGE_TOLERANCE
            };
            let within = r.iter().zip(o).all(|(a, b)| a.abs_diff(b) <= tolerance);
            if r == o {
                out.diff.extend(r[..3].iter().map(|c| c / 2 + 0x40));
            } else if within {
                out.tolerated += 1;
                out.diff.extend(r[..3].iter().map(|c| c / 2 + 0x40));
            } else {
                out.differing += 1;
                let (x, y) = (x as u32, y as u32);
                out.first.get_or_insert((x, y, r, o));
                out.bounds = Some(match out.bounds {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
                out.diff.extend_from_slice(&[0xff, 0x00, 0x00]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SoftwareBackend;

    /// Both backends over the bundled test font, or `None` where there
    /// is no adapter, which the test then says.
    fn backends() -> Option<(SoftwareBackend, GpuBackend)> {
        let fonts = FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses");
        match GpuBackend::with_fonts(fonts.clone()) {
            Ok(gpu) => Some((SoftwareBackend::with_fonts(fonts), gpu)),
            Err(e) => {
                println!("no GPU adapter ({e}): the GPU tests are skipped here");
                None
            }
        }
    }

    fn rect(x: f32, y: f32, w: f32, h: f32, color: Rgba8) -> DisplayItem {
        DisplayItem::Rect { x, y, w, h, color }
    }

    fn text(text: &str, x: f32, y: f32, size: f32, color: Rgba8) -> DisplayItem {
        DisplayItem::Text {
            text: text.to_string(),
            x,
            y,
            size,
            color,
        }
    }

    fn group(matrix: Matrix, opacity: f32, key: u32, layer: Option<LayerReason>) -> DisplayItem {
        DisplayItem::PushGroup {
            matrix,
            opacity,
            key: GroupKey::Document(key),
            layer,
        }
    }

    fn rotation(degrees: f32) -> Matrix {
        Matrix::rotate_z(degrees)
    }

    fn rgba(r: u8, g: u8, b: u8, a: u8) -> Rgba8 {
        Rgba8 { r, g, b, a }
    }

    /// Draws `frames` on both backends in turn and holds each of the
    /// GPU's to the software's under the equivalence rule.
    fn equivalent(name: &str, frames: &[DisplayList], w: u32, h: u32) {
        let Some((mut software, mut gpu)) = backends() else {
            return;
        };
        for (i, list) in frames.iter().enumerate() {
            let reference = software.rasterize(list, w, h).to_vec();
            let other = gpu.rasterize(list, w, h).to_vec();
            assert_eq!(gpu.errors(), Vec::<String>::new(), "{name}, frame {i}");
            let found = compare(&reference, &other, w, h);
            assert_eq!(
                found.differing, 0,
                "{name}, frame {i}: {} pixels differ within {:?}; the first at {:?}",
                found.differing, found.bounds, found.first
            );
        }
    }

    #[test]
    fn rectangles_at_fractional_edges_and_translucent_colours_match() {
        let white = Rgba8::rgb(255, 255, 255);
        let list = DisplayList {
            items: vec![
                rect(0.0, 0.0, 200.0, 120.0, white),
                rect(10.25, 10.75, 50.5, 30.3, Rgba8::rgb(200, 30, 30)),
                rect(40.0, 20.0, 80.0, 60.0, rgba(30, 30, 200, 128)),
                rect(-20.0, 100.0, 300.0, 40.0, Rgba8::rgb(0, 128, 0)),
                DisplayItem::PushClip {
                    x: 100.5,
                    y: 5.5,
                    w: 60.25,
                    h: 70.0,
                },
                rect(90.0, 0.0, 100.0, 100.0, Rgba8::rgb(255, 200, 0)),
                DisplayItem::PopClip,
            ],
        };
        equivalent("rectangles", &[list], 200, 120);
    }

    #[test]
    fn text_quads_and_polygons_match() {
        let white = Rgba8::rgb(255, 255, 255);
        let black = Rgba8::rgb(0, 0, 0);
        let list = DisplayList {
            items: vec![
                rect(0.0, 0.0, 240.0, 160.0, white),
                text("Silver, quick & sharp", 8.0, 24.0, 16.0, black),
                text("Larger", 8.0, 64.0, 32.0, Rgba8::rgb(60, 60, 160)),
                DisplayItem::PushClip {
                    x: 0.0,
                    y: 70.0,
                    w: 60.0,
                    h: 30.0,
                },
                text("clipped away", 8.0, 92.0, 16.0, black),
                DisplayItem::PopClip,
                DisplayItem::Quad {
                    points: [(120.0, 80.0), (220.0, 80.0), (220.0, 90.0), (120.0, 90.0)],
                    color: Rgba8::rgb(200, 0, 0),
                },
                DisplayItem::Quad {
                    points: [(120.0, 90.0), (220.0, 90.0), (220.0, 100.0), (120.0, 100.0)],
                    color: Rgba8::rgb(200, 0, 0),
                },
                DisplayItem::Polygon {
                    points: vec![
                        (120.0, 110.0),
                        (220.0, 110.0),
                        (210.0, 120.0),
                        (130.0, 120.0),
                    ],
                    color: Rgba8::rgb(0, 0, 200),
                },
                DisplayItem::Polygon {
                    points: vec![
                        (120.0, 120.0),
                        (130.0, 120.0),
                        (130.0, 150.0),
                        (120.0, 150.0),
                    ],
                    color: Rgba8::rgb(0, 160, 0),
                },
            ],
        };
        equivalent("text, quads and polygons", &[list], 240, 160);
    }

    #[test]
    fn groups_drawn_through_and_from_scratches_match() {
        let white = Rgba8::rgb(255, 255, 255);
        let black = Rgba8::rgb(0, 0, 0);
        // A card's items, and the end of the group they are in.
        let card = || {
            vec![
                rect(0.0, 0.0, 80.0, 50.0, Rgba8::rgb(230, 230, 250)),
                rect(0.0, 0.0, 80.0, 2.0, Rgba8::rgb(100, 100, 200)),
                text("card", 6.0, 30.0, 14.0, black),
                DisplayItem::PopGroup,
            ]
        };
        let mut items = vec![rect(0.0, 0.0, 320.0, 200.0, white)];
        // Translated by a fraction, drawn through.
        items.push(group(Matrix::translate(10.5, 8.25, 0.0), 1.0, 1, None));
        items.extend(card());
        // Scaled, drawn through, the text at the scaled size.
        items.push(group(
            Matrix::translate(110.0, 8.0, 0.0).then(&Matrix::scale(1.5, 1.5, 1.0)),
            1.0,
            2,
            None,
        ));
        items.extend(card());
        // Faded, through a scratch in the frame's coordinates.
        items.push(group(Matrix::translate(10.0, 100.0, 0.0), 0.5, 3, None));
        items.extend(card());
        // Rotated, a layer resampled from its own plane.
        items.push(group(
            Matrix::translate(140.0, 110.0, 0.0).then(&rotation(12.0)),
            1.0,
            4,
            Some(LayerReason::Transformed),
        ));
        items.extend(card());
        // Skewed and faded, a scratch in the group's own coordinates.
        items.push(group(
            Matrix::translate(230.0, 100.0, 0.0)
                .then(&Matrix::from_2d([1.0, 0.0, 0.4, 1.0, 0.0, 0.0])),
            0.7,
            5,
            None,
        ));
        items.extend(card());
        let list = DisplayList { items };
        // Twice: the second frame composites the layer kept.
        equivalent("groups", &[list.clone(), list], 320, 200);
    }

    #[test]
    fn a_scrolled_layer_of_tiles_and_a_projective_group_match() {
        let white = Rgba8::rgb(255, 255, 255);
        let page = |scroll: f32| {
            let mut items = vec![rect(0.0, 0.0, 200.0, 150.0, white)];
            items.push(group(
                Matrix::translate(0.0, -scroll, 0.0),
                1.0,
                7,
                Some(LayerReason::Scrolled),
            ));
            for i in 0..12 {
                let y = i as f32 * 60.0;
                let shade = (i * 20) as u8;
                items.push(rect(
                    10.0,
                    y + 5.0,
                    150.0,
                    40.0,
                    Rgba8::rgb(255, shade, 120),
                ));
                items.push(text(
                    "row of text",
                    14.0,
                    y + 30.0,
                    15.0,
                    Rgba8::rgb(0, 0, 0),
                ));
            }
            items.push(DisplayItem::PopGroup);
            // Fixed over the content, turned in 3D under a perspective.
            let turned = Matrix::translate(120.0, 100.0, 0.0)
                .then(&Matrix::perspective(Some(300.0)))
                .then(&Matrix::rotate(0.0, 1.0, 0.0, 35.0))
                .flattened();
            items.push(group(turned, 1.0, 8, Some(LayerReason::Transformed)));
            items.push(rect(0.0, 0.0, 60.0, 40.0, Rgba8::rgb(20, 120, 20)));
            items.push(rect(5.0, 5.0, 50.0, 30.0, Rgba8::rgb(200, 220, 200)));
            items.push(DisplayItem::PopGroup);
            DisplayList { items }
        };
        equivalent(
            "scrolled tiles and a projective group",
            &[page(0.0), page(100.0), page(400.0)],
            200,
            150,
        );
    }

    #[test]
    fn the_frame_and_the_layers_report_as_the_software_backends_do() {
        let Some((mut software, mut gpu)) = backends() else {
            return;
        };
        let list = DisplayList {
            items: vec![
                rect(0.0, 0.0, 100.0, 100.0, Rgba8::rgb(255, 255, 255)),
                group(
                    Matrix::translate(20.0, 20.0, 0.0).then(&rotation(30.0)),
                    1.0,
                    1,
                    Some(LayerReason::Transformed),
                ),
                rect(0.0, 0.0, 30.0, 30.0, Rgba8::rgb(0, 0, 0)),
                DisplayItem::PopGroup,
            ],
        };
        for backend in [&mut software as &mut dyn RenderBackend, &mut gpu] {
            let on = backend.backend();
            backend.rasterize(&list, 100, 100);
            assert_eq!(backend.layers().len(), 1, "{on}");
            assert!(backend.layer_bytes() > 0, "{on}");
            assert_eq!(backend.layered().rasterized.len(), 1, "{on}");
            assert_eq!(backend.layered().composited.len(), 1, "{on}");
            // The same list again: the layer is kept, and composited
            // again by the GPU, which draws every frame whole, while
            // the software backend draws nothing anew.
            backend.rasterize(&list, 100, 100);
            assert!(backend.layered().rasterized.is_empty(), "{on}");
            assert_eq!(
                backend.layered().composited.len(),
                usize::from(on == Backend::Gpu),
                "{on}"
            );
            backend.set_layer_budget(0);
            backend.rasterize(&list, 100, 100);
            assert!(backend.layers().is_empty(), "{on}");
            assert_eq!(backend.layer_bytes(), 0, "{on}");
            assert!(backend.rasterize(&list, 0, 0).is_empty());
            assert!(backend.rasterize(&list, 100_000, 100_000).is_empty());
        }
        assert_eq!(gpu.backend(), Backend::Gpu);
        assert_eq!(software.backend(), Backend::Software);
        assert_eq!(gpu.errors(), Vec::<String>::new());
    }

    #[test]
    fn the_comparison_tells_inside_pixels_from_edge_pixels() {
        let (w, h) = (4u32, 3u32);
        let at = |x: usize, y: usize| (y * w as usize + x) * 4;
        let mut a = vec![0u8; (w * h * 4) as usize];
        for px in a.chunks_mut(4) {
            px.copy_from_slice(&[10, 20, 30, 255]);
        }
        // An edge at the right column.
        for y in 0..h as usize {
            a[at(3, y)..at(3, y) + 4].copy_from_slice(&[200, 200, 200, 255]);
        }
        let mut b = a.clone();
        b[at(1, 1)] += 2;
        assert_eq!(compare(&a, &b, w, h).differing, 0);
        b[at(1, 1)] += 1;
        let found = compare(&a, &b, w, h);
        assert_eq!(
            (found.differing, found.first),
            (1, Some((1, 1, [10, 20, 30, 255], [13, 20, 30, 255])))
        );
        let mut c = a.clone();
        c[at(2, 1)] += 60;
        assert_eq!(compare(&a, &c, w, h).differing, 0);
        c[at(2, 1)] += 10;
        assert_eq!(compare(&a, &c, w, h).differing, 1);
        assert_eq!(compare(&a, &a[..8], w, h).differing, usize::MAX);
    }
}
