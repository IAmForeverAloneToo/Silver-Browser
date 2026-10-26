//! WebGPU's dictionaries, as the client sends them in JSON, to `wgpu`'s
//! types (M41 step 3, D39). The page's own values were checked by the
//! bootstrap as Web IDL says; what is left wrong is `wgpu`'s validation's
//! to say. A name that is none of WebGPU's, or a member of the wrong
//! shape, comes from a renderer that broke the protocol: Err, which ends
//! its client.

use devtools::Json;

/// Why a dictionary cannot be read.
pub type Bad = String;

pub fn member<'a>(dict: &'a Json, name: &str) -> Option<&'a Json> {
    dict.get(name).filter(|value| !matches!(value, Json::Null))
}

pub fn text<'a>(dict: &'a Json, name: &str) -> Result<&'a str, Bad> {
    member(dict, name)
        .and_then(Json::as_str)
        .ok_or_else(|| format!("no '{name}'"))
}

pub fn number(dict: &Json, name: &str, default: f64) -> f64 {
    member(dict, name).and_then(Json::as_f64).unwrap_or(default)
}

pub fn whole(dict: &Json, name: &str, default: u32) -> u32 {
    member(dict, name)
        .and_then(Json::as_f64)
        .map_or(default, |n| n.clamp(0.0, f64::from(u32::MAX)) as u32)
}

pub fn wide(dict: &Json, name: &str, default: u64) -> u64 {
    member(dict, name)
        .and_then(Json::as_f64)
        .map_or(default, |n| n.max(0.0) as u64)
}

pub fn flag(dict: &Json, name: &str) -> bool {
    member(dict, name).and_then(Json::as_bool).unwrap_or(false)
}

pub fn list<'a>(dict: &'a Json, name: &str) -> &'a [Json] {
    member(dict, name).and_then(Json::as_array).unwrap_or(&[])
}

/// A name of WebGPU's enum, read by `map`.
fn named<T>(value: &str, map: &[(&str, T)], what: &str) -> Result<T, Bad>
where
    T: Copy,
{
    map.iter()
        .find(|(name, _)| *name == value)
        .map(|(_, item)| *item)
        .ok_or_else(|| format!("a {what} of '{value}'"))
}

use wgpu::TextureFormat as F;

const fn astc(block: wgpu::AstcBlock, channel: wgpu::AstcChannel) -> F {
    F::Astc { block, channel }
}

use wgpu::AstcBlock as B;
use wgpu::AstcChannel as C;

/// WebGPU's texture formats: core, the 16-bit normalized ones of
/// texture-formats-tier1, and the compressed ones (BC, ETC2, ASTC); the
/// bootstrap's `GPU_FORMATS` names the same.
pub const FORMATS: &[(&str, F)] = &[
    ("r8unorm", F::R8Unorm),
    ("r8snorm", F::R8Snorm),
    ("r8uint", F::R8Uint),
    ("r8sint", F::R8Sint),
    ("r16unorm", F::R16Unorm),
    ("r16snorm", F::R16Snorm),
    ("rg16unorm", F::Rg16Unorm),
    ("rg16snorm", F::Rg16Snorm),
    ("rgba16unorm", F::Rgba16Unorm),
    ("rgba16snorm", F::Rgba16Snorm),
    ("r16uint", F::R16Uint),
    ("r16sint", F::R16Sint),
    ("r16float", F::R16Float),
    ("rg8unorm", F::Rg8Unorm),
    ("rg8snorm", F::Rg8Snorm),
    ("rg8uint", F::Rg8Uint),
    ("rg8sint", F::Rg8Sint),
    ("r32uint", F::R32Uint),
    ("r32sint", F::R32Sint),
    ("r32float", F::R32Float),
    ("rg16uint", F::Rg16Uint),
    ("rg16sint", F::Rg16Sint),
    ("rg16float", F::Rg16Float),
    ("rgba8unorm", F::Rgba8Unorm),
    ("rgba8unorm-srgb", F::Rgba8UnormSrgb),
    ("rgba8snorm", F::Rgba8Snorm),
    ("rgba8uint", F::Rgba8Uint),
    ("rgba8sint", F::Rgba8Sint),
    ("bgra8unorm", F::Bgra8Unorm),
    ("bgra8unorm-srgb", F::Bgra8UnormSrgb),
    ("rgb9e5ufloat", F::Rgb9e5Ufloat),
    ("rgb10a2uint", F::Rgb10a2Uint),
    ("rgb10a2unorm", F::Rgb10a2Unorm),
    ("rg11b10ufloat", F::Rg11b10Ufloat),
    ("rg32uint", F::Rg32Uint),
    ("rg32sint", F::Rg32Sint),
    ("rg32float", F::Rg32Float),
    ("rgba16uint", F::Rgba16Uint),
    ("rgba16sint", F::Rgba16Sint),
    ("rgba16float", F::Rgba16Float),
    ("rgba32uint", F::Rgba32Uint),
    ("rgba32sint", F::Rgba32Sint),
    ("rgba32float", F::Rgba32Float),
    ("stencil8", F::Stencil8),
    ("depth16unorm", F::Depth16Unorm),
    ("depth24plus", F::Depth24Plus),
    ("depth24plus-stencil8", F::Depth24PlusStencil8),
    ("depth32float", F::Depth32Float),
    ("depth32float-stencil8", F::Depth32FloatStencil8),
    ("bc1-rgba-unorm", F::Bc1RgbaUnorm),
    ("bc1-rgba-unorm-srgb", F::Bc1RgbaUnormSrgb),
    ("bc2-rgba-unorm", F::Bc2RgbaUnorm),
    ("bc2-rgba-unorm-srgb", F::Bc2RgbaUnormSrgb),
    ("bc3-rgba-unorm", F::Bc3RgbaUnorm),
    ("bc3-rgba-unorm-srgb", F::Bc3RgbaUnormSrgb),
    ("bc4-r-unorm", F::Bc4RUnorm),
    ("bc4-r-snorm", F::Bc4RSnorm),
    ("bc5-rg-unorm", F::Bc5RgUnorm),
    ("bc5-rg-snorm", F::Bc5RgSnorm),
    ("bc6h-rgb-ufloat", F::Bc6hRgbUfloat),
    ("bc6h-rgb-float", F::Bc6hRgbFloat),
    ("bc7-rgba-unorm", F::Bc7RgbaUnorm),
    ("bc7-rgba-unorm-srgb", F::Bc7RgbaUnormSrgb),
    ("etc2-rgb8unorm", F::Etc2Rgb8Unorm),
    ("etc2-rgb8unorm-srgb", F::Etc2Rgb8UnormSrgb),
    ("etc2-rgb8a1unorm", F::Etc2Rgb8A1Unorm),
    ("etc2-rgb8a1unorm-srgb", F::Etc2Rgb8A1UnormSrgb),
    ("etc2-rgba8unorm", F::Etc2Rgba8Unorm),
    ("etc2-rgba8unorm-srgb", F::Etc2Rgba8UnormSrgb),
    ("eac-r11unorm", F::EacR11Unorm),
    ("eac-r11snorm", F::EacR11Snorm),
    ("eac-rg11unorm", F::EacRg11Unorm),
    ("eac-rg11snorm", F::EacRg11Snorm),
    ("astc-4x4-unorm", astc(B::B4x4, C::Unorm)),
    ("astc-4x4-unorm-srgb", astc(B::B4x4, C::UnormSrgb)),
    ("astc-5x4-unorm", astc(B::B5x4, C::Unorm)),
    ("astc-5x4-unorm-srgb", astc(B::B5x4, C::UnormSrgb)),
    ("astc-5x5-unorm", astc(B::B5x5, C::Unorm)),
    ("astc-5x5-unorm-srgb", astc(B::B5x5, C::UnormSrgb)),
    ("astc-6x5-unorm", astc(B::B6x5, C::Unorm)),
    ("astc-6x5-unorm-srgb", astc(B::B6x5, C::UnormSrgb)),
    ("astc-6x6-unorm", astc(B::B6x6, C::Unorm)),
    ("astc-6x6-unorm-srgb", astc(B::B6x6, C::UnormSrgb)),
    ("astc-8x5-unorm", astc(B::B8x5, C::Unorm)),
    ("astc-8x5-unorm-srgb", astc(B::B8x5, C::UnormSrgb)),
    ("astc-8x6-unorm", astc(B::B8x6, C::Unorm)),
    ("astc-8x6-unorm-srgb", astc(B::B8x6, C::UnormSrgb)),
    ("astc-8x8-unorm", astc(B::B8x8, C::Unorm)),
    ("astc-8x8-unorm-srgb", astc(B::B8x8, C::UnormSrgb)),
    ("astc-10x5-unorm", astc(B::B10x5, C::Unorm)),
    ("astc-10x5-unorm-srgb", astc(B::B10x5, C::UnormSrgb)),
    ("astc-10x6-unorm", astc(B::B10x6, C::Unorm)),
    ("astc-10x6-unorm-srgb", astc(B::B10x6, C::UnormSrgb)),
    ("astc-10x8-unorm", astc(B::B10x8, C::Unorm)),
    ("astc-10x8-unorm-srgb", astc(B::B10x8, C::UnormSrgb)),
    ("astc-10x10-unorm", astc(B::B10x10, C::Unorm)),
    ("astc-10x10-unorm-srgb", astc(B::B10x10, C::UnormSrgb)),
    ("astc-12x10-unorm", astc(B::B12x10, C::Unorm)),
    ("astc-12x10-unorm-srgb", astc(B::B12x10, C::UnormSrgb)),
    ("astc-12x12-unorm", astc(B::B12x12, C::Unorm)),
    ("astc-12x12-unorm-srgb", astc(B::B12x12, C::UnormSrgb)),
];

pub fn format(name: &str) -> Result<F, Bad> {
    named(name, FORMATS, "texture format")
}

use wgpu::VertexFormat as V;

pub const VERTEX_FORMATS: &[(&str, V)] = &[
    ("uint8", V::Uint8),
    ("uint8x2", V::Uint8x2),
    ("uint8x4", V::Uint8x4),
    ("sint8", V::Sint8),
    ("sint8x2", V::Sint8x2),
    ("sint8x4", V::Sint8x4),
    ("unorm8", V::Unorm8),
    ("unorm8x2", V::Unorm8x2),
    ("unorm8x4", V::Unorm8x4),
    ("snorm8", V::Snorm8),
    ("snorm8x2", V::Snorm8x2),
    ("snorm8x4", V::Snorm8x4),
    ("uint16", V::Uint16),
    ("uint16x2", V::Uint16x2),
    ("uint16x4", V::Uint16x4),
    ("sint16", V::Sint16),
    ("sint16x2", V::Sint16x2),
    ("sint16x4", V::Sint16x4),
    ("unorm16", V::Unorm16),
    ("unorm16x2", V::Unorm16x2),
    ("unorm16x4", V::Unorm16x4),
    ("snorm16", V::Snorm16),
    ("snorm16x2", V::Snorm16x2),
    ("snorm16x4", V::Snorm16x4),
    ("float16", V::Float16),
    ("float16x2", V::Float16x2),
    ("float16x4", V::Float16x4),
    ("float32", V::Float32),
    ("float32x2", V::Float32x2),
    ("float32x3", V::Float32x3),
    ("float32x4", V::Float32x4),
    ("uint32", V::Uint32),
    ("uint32x2", V::Uint32x2),
    ("uint32x3", V::Uint32x3),
    ("uint32x4", V::Uint32x4),
    ("sint32", V::Sint32),
    ("sint32x2", V::Sint32x2),
    ("sint32x3", V::Sint32x3),
    ("sint32x4", V::Sint32x4),
    ("unorm10-10-10-2", V::Unorm10_10_10_2),
    ("unorm8x4-bgra", V::Unorm8x4Bgra),
];

pub fn vertex_format(name: &str) -> Result<V, Bad> {
    named(name, VERTEX_FORMATS, "vertex format")
}

pub fn compare(name: &str) -> Result<wgpu::CompareFunction, Bad> {
    use wgpu::CompareFunction as C;
    named(
        name,
        &[
            ("never", C::Never),
            ("less", C::Less),
            ("equal", C::Equal),
            ("less-equal", C::LessEqual),
            ("greater", C::Greater),
            ("not-equal", C::NotEqual),
            ("greater-equal", C::GreaterEqual),
            ("always", C::Always),
        ],
        "compare function",
    )
}

pub fn dimension(name: &str) -> Result<wgpu::TextureDimension, Bad> {
    use wgpu::TextureDimension as D;
    named(
        name,
        &[("1d", D::D1), ("2d", D::D2), ("3d", D::D3)],
        "texture dimension",
    )
}

pub fn view_dimension(name: &str) -> Result<wgpu::TextureViewDimension, Bad> {
    use wgpu::TextureViewDimension as D;
    named(
        name,
        &[
            ("1d", D::D1),
            ("2d", D::D2),
            ("2d-array", D::D2Array),
            ("cube", D::Cube),
            ("cube-array", D::CubeArray),
            ("3d", D::D3),
        ],
        "view dimension",
    )
}

pub fn aspect(name: &str) -> Result<wgpu::TextureAspect, Bad> {
    use wgpu::TextureAspect as A;
    named(
        name,
        &[
            ("all", A::All),
            ("stencil-only", A::StencilOnly),
            ("depth-only", A::DepthOnly),
        ],
        "texture aspect",
    )
}

/// A `GPUExtent3D` or `GPUOrigin3D`, as a dictionary or a list.
pub fn extent(value: Option<&Json>, default: u32) -> (u32, u32, u32) {
    let at = |i: usize, name: &str, fallback: u32| -> u32 {
        match value {
            Some(Json::Array(list)) => list
                .get(i)
                .and_then(Json::as_f64)
                .map_or(fallback, |n| n as u32),
            Some(dict) => whole(dict, name, fallback),
            None => fallback,
        }
    };
    match value {
        Some(Json::Array(_)) => (
            at(0, "", default),
            at(1, "", if default == 0 { 0 } else { 1 }),
            at(2, "", if default == 0 { 0 } else { 1 }),
        ),
        _ => (
            at(0, "width", default),
            at(1, "height", if default == 0 { 0 } else { 1 }),
            at(2, "depthOrArrayLayers", if default == 0 { 0 } else { 1 }),
        ),
    }
}

pub fn origin(value: Option<&Json>) -> wgpu::Origin3d {
    let (x, y, z) = match value {
        Some(Json::Array(list)) => {
            let at = |i: usize| list.get(i).and_then(Json::as_f64).unwrap_or(0.0) as u32;
            (at(0), at(1), at(2))
        }
        Some(dict) => (
            whole(dict, "x", 0),
            whole(dict, "y", 0),
            whole(dict, "z", 0),
        ),
        None => (0, 0, 0),
    };
    wgpu::Origin3d { x, y, z }
}

pub fn size(value: Option<&Json>) -> wgpu::Extent3d {
    let (width, height, depth_or_array_layers) = extent(value, 1);
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers,
    }
}

pub fn address(name: &str) -> Result<wgpu::AddressMode, Bad> {
    use wgpu::AddressMode as A;
    named(
        name,
        &[
            ("clamp-to-edge", A::ClampToEdge),
            ("repeat", A::Repeat),
            ("mirror-repeat", A::MirrorRepeat),
        ],
        "address mode",
    )
}

pub fn filter(name: &str) -> Result<wgpu::FilterMode, Bad> {
    use wgpu::FilterMode as M;
    named(
        name,
        &[("nearest", M::Nearest), ("linear", M::Linear)],
        "filter mode",
    )
}

pub fn mipmap_filter(name: &str) -> Result<wgpu::MipmapFilterMode, Bad> {
    use wgpu::MipmapFilterMode as M;
    named(
        name,
        &[("nearest", M::Nearest), ("linear", M::Linear)],
        "mipmap filter mode",
    )
}

pub fn topology(name: &str) -> Result<wgpu::PrimitiveTopology, Bad> {
    use wgpu::PrimitiveTopology as T;
    named(
        name,
        &[
            ("point-list", T::PointList),
            ("line-list", T::LineList),
            ("line-strip", T::LineStrip),
            ("triangle-list", T::TriangleList),
            ("triangle-strip", T::TriangleStrip),
        ],
        "primitive topology",
    )
}

pub fn blend_factor(name: &str) -> Result<wgpu::BlendFactor, Bad> {
    use wgpu::BlendFactor as B;
    named(
        name,
        &[
            ("zero", B::Zero),
            ("one", B::One),
            ("src", B::Src),
            ("one-minus-src", B::OneMinusSrc),
            ("src-alpha", B::SrcAlpha),
            ("one-minus-src-alpha", B::OneMinusSrcAlpha),
            ("dst", B::Dst),
            ("one-minus-dst", B::OneMinusDst),
            ("dst-alpha", B::DstAlpha),
            ("one-minus-dst-alpha", B::OneMinusDstAlpha),
            ("src-alpha-saturated", B::SrcAlphaSaturated),
            ("constant", B::Constant),
            ("one-minus-constant", B::OneMinusConstant),
            ("src1", B::Src1),
            ("one-minus-src1", B::OneMinusSrc1),
            ("src1-alpha", B::Src1Alpha),
            ("one-minus-src1-alpha", B::OneMinusSrc1Alpha),
        ],
        "blend factor",
    )
}

pub fn blend_operation(name: &str) -> Result<wgpu::BlendOperation, Bad> {
    use wgpu::BlendOperation as O;
    named(
        name,
        &[
            ("add", O::Add),
            ("subtract", O::Subtract),
            ("reverse-subtract", O::ReverseSubtract),
            ("min", O::Min),
            ("max", O::Max),
        ],
        "blend operation",
    )
}

pub fn stencil_operation(name: &str) -> Result<wgpu::StencilOperation, Bad> {
    use wgpu::StencilOperation as S;
    named(
        name,
        &[
            ("keep", S::Keep),
            ("zero", S::Zero),
            ("replace", S::Replace),
            ("invert", S::Invert),
            ("increment-clamp", S::IncrementClamp),
            ("decrement-clamp", S::DecrementClamp),
            ("increment-wrap", S::IncrementWrap),
            ("decrement-wrap", S::DecrementWrap),
        ],
        "stencil operation",
    )
}

/// A `GPUColor`, as a dictionary or a list.
pub fn color(value: Option<&Json>) -> wgpu::Color {
    let (r, g, b, a) = match value {
        Some(Json::Array(list)) => {
            let at = |i: usize| list.get(i).and_then(Json::as_f64).unwrap_or(0.0);
            (at(0), at(1), at(2), at(3))
        }
        Some(dict) => (
            number(dict, "r", 0.0),
            number(dict, "g", 0.0),
            number(dict, "b", 0.0),
            number(dict, "a", 0.0),
        ),
        None => (0.0, 0.0, 0.0, 0.0),
    };
    wgpu::Color { r, g, b, a }
}

/// A shader stage's pipeline-overridable constants.
pub fn constants(dict: &Json) -> Vec<(String, f64)> {
    match member(dict, "constants") {
        Some(Json::Object(members)) => members
            .iter()
            .filter_map(|(name, value)| Some((name.clone(), value.as_f64()?)))
            .collect(),
        _ => Vec::new(),
    }
}
