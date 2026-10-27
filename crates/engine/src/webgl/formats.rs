//! WebGL's texture and renderbuffer formats (M41 steps 7 and 8, D40):
//! GL's internal formats, formats and types as WebGPU's storage, the
//! client's pixels made texels (widened where WebGPU has no format of
//! three channels or of luminance), and texels read back as the
//! client's.

use super::gl::*;

/// A component's encoding: the client's or the storage's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Comp {
    U8,
    S8,
    U16,
    S16,
    U32,
    S32,
    F16,
    F32,
    /// Packed together: 565, 4444 and 5551 in 16 bits; rgb10a2,
    /// rg11b10 floats and rgb9e5 in 32; a depth and stencil in 32 or 64.
    Packed,
}

impl Comp {
    fn size(self) -> usize {
        match self {
            Comp::U8 | Comp::S8 => 1,
            Comp::U16 | Comp::S16 | Comp::F16 => 2,
            _ => 4,
        }
    }
}

/// What a sampler reads from a storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Sample {
    Float,
    Sint,
    Uint,
    Depth,
}

/// A texture image's or renderbuffer's format: GL's internal format,
/// the client's format and type (0 for a compressed one or a
/// renderbuffer), and WebGPU's storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Format {
    pub internal: u32,
    pub format: u32,
    pub kind: u32,
    pub storage: &'static str,
}

impl Format {
    pub(crate) fn depth(&self) -> bool {
        has_depth(self.storage) || has_stencil(self.storage)
    }

    pub(crate) fn compressed(&self) -> bool {
        self.storage.starts_with("bc")
    }

    pub(crate) fn sample(&self) -> Sample {
        info(self.storage).2
    }

    /// Whether GL's format has no alpha (read as 1).
    pub(crate) fn opaque(&self) -> bool {
        matches!(
            self.internal,
            RGB | SRGB_EXT
                | LUMINANCE
                | RGB565
                | COMPRESSED_RGB_S3TC_DXT1_EXT
                | RGB8
                | SRGB8
                | RGB8_SNORM
                | R11F_G11F_B10F
                | RGB9_E5
                | RGB16F
                | RGB32F
                | RGB8UI
                | RGB8I
                | RGB16UI
                | RGB16I
                | RGB32UI
                | RGB32I
        )
    }

    /// Whether GL's format is unsized (WebGL 1's), whose component sizes
    /// come from its type.
    pub(crate) fn is_unsized(&self) -> bool {
        matches!(
            self.internal,
            ALPHA | RGB | RGBA | LUMINANCE | LUMINANCE_ALPHA | SRGB_EXT | SRGB_ALPHA_EXT
        ) || (self.kind != 0 && matches!(self.internal, DEPTH_COMPONENT | DEPTH_STENCIL))
    }
}

pub(crate) fn has_depth(storage: &str) -> bool {
    storage.starts_with("depth")
}

pub(crate) fn has_stencil(storage: &str) -> bool {
    storage.contains("stencil")
}

/// A storage's channels, their encoding, and what a sampler reads.
pub(crate) fn info(storage: &str) -> (usize, Comp, Sample) {
    let channels = if storage.starts_with("rgba") || storage.starts_with("rgb10a2") {
        4
    } else if storage.starts_with("rg11b10") || storage.starts_with("rgb9e5") {
        3
    } else if storage.starts_with("rg") {
        2
    } else if storage.starts_with('r') {
        1
    } else if storage.starts_with("bc") {
        4
    } else {
        1
    };
    let sample = if has_depth(storage) {
        Sample::Depth
    } else if storage.ends_with("sint") {
        Sample::Sint
    } else if storage.ends_with("uint") || storage == "stencil8" {
        Sample::Uint
    } else {
        Sample::Float
    };
    let comp = match storage {
        "rgb10a2unorm" | "rgb10a2uint" | "rg11b10ufloat" | "rgb9e5ufloat" => Comp::Packed,
        "depth16unorm" => Comp::U16,
        "depth24plus" | "depth24plus-stencil8" | "depth32float-stencil8" => Comp::Packed,
        "depth32float" => Comp::F32,
        "stencil8" => Comp::U8,
        s if s.starts_with("bc") => Comp::Packed,
        s => {
            let bits = if s.contains("32") {
                32
            } else if s.contains("16") {
                16
            } else {
                8
            };
            match (
                bits,
                s.ends_with("float"),
                s.ends_with("sint") || s.ends_with("snorm"),
            ) {
                (_, true, _) if bits == 16 => Comp::F16,
                (_, true, _) => Comp::F32,
                (8, _, false) => Comp::U8,
                (8, _, true) => Comp::S8,
                (16, _, false) => Comp::U16,
                (16, _, true) => Comp::S16,
                (_, _, false) => Comp::U32,
                (_, _, true) => Comp::S32,
            }
        }
    };
    (channels, comp, sample)
}

/// A storage's bytes a texel (a compressed one's a block of 4 by 4).
pub(crate) fn texel_bytes(storage: &str) -> u32 {
    match storage {
        "depth32float-stencil8" => 8,
        "rgb10a2unorm"
        | "rgb10a2uint"
        | "rg11b10ufloat"
        | "rgb9e5ufloat"
        | "depth24plus"
        | "depth24plus-stencil8" => 4,
        "bc1-rgba-unorm" | "bc1-rgba-unorm-srgb" => 8,
        s if s.starts_with("bc") => 16,
        s => {
            let (channels, comp, _) = info(s);
            (channels * comp.size()) as u32
        }
    }
}

/// A client format's channels.
pub(crate) fn channels(format: u32) -> u32 {
    match format {
        ALPHA | LUMINANCE | DEPTH_COMPONENT | DEPTH_STENCIL | RED | RED_INTEGER => 1,
        LUMINANCE_ALPHA | RG | RG_INTEGER => 2,
        RGB | SRGB_EXT | RGB_INTEGER => 3,
        _ => 4,
    }
}

fn integer_format(format: u32) -> bool {
    matches!(
        format,
        RED_INTEGER | RG_INTEGER | RGB_INTEGER | RGBA_INTEGER
    )
}

/// A client type's components.
fn client_comp(kind: u32) -> Comp {
    match kind {
        UNSIGNED_BYTE => Comp::U8,
        BYTE => Comp::S8,
        UNSIGNED_SHORT => Comp::U16,
        SHORT => Comp::S16,
        UNSIGNED_INT => Comp::U32,
        INT => Comp::S32,
        HALF_FLOAT | HALF_FLOAT_OES => Comp::F16,
        FLOAT => Comp::F32,
        _ => Comp::Packed,
    }
}

/// The client's bytes a pixel of `format` and `kind`.
pub(crate) fn client_pixel(format: u32, kind: u32) -> u32 {
    match kind {
        UNSIGNED_SHORT_5_6_5 | UNSIGNED_SHORT_4_4_4_4 | UNSIGNED_SHORT_5_5_5_1 => 2,
        UNSIGNED_INT_2_10_10_10_REV
        | UNSIGNED_INT_10F_11F_11F_REV
        | UNSIGNED_INT_5_9_9_9_REV
        | UNSIGNED_INT_24_8 => 4,
        FLOAT_32_UNSIGNED_INT_24_8_REV => 8,
        k => channels(format) * client_comp(k).size() as u32,
    }
}

/// The JavaScript array a type's data comes in, by the bootstrap's code:
/// 1 Uint8Array, 2 Uint16Array, 3 Float32Array, 4 Uint32Array, 5
/// Int8Array, 6 Uint8ClampedArray, 7 Int16Array, 8 Int32Array.
pub(crate) fn view_ok(kind: u32, view: u32) -> bool {
    match kind {
        UNSIGNED_BYTE => view == 1 || view == 6,
        BYTE => view == 5,
        SHORT => view == 7,
        INT => view == 8,
        FLOAT => view == 3,
        UNSIGNED_INT
        | UNSIGNED_INT_24_8
        | UNSIGNED_INT_2_10_10_10_REV
        | UNSIGNED_INT_10F_11F_11F_REV
        | UNSIGNED_INT_5_9_9_9_REV
        | FLOAT_32_UNSIGNED_INT_24_8_REV => view == 4,
        _ => view == 2,
    }
}

/// WebGL 2's sized internal formats (GL ES 3.0 table 3.2): the format,
/// the types taken, and the storage.
const SIZED: &[(u32, u32, &[u32], &str)] = &[
    (R8, RED, &[UNSIGNED_BYTE], "r8unorm"),
    (R8_SNORM, RED, &[BYTE], "r8snorm"),
    (R16F, RED, &[HALF_FLOAT, FLOAT], "r16float"),
    (R32F, RED, &[FLOAT], "r32float"),
    (R8UI, RED_INTEGER, &[UNSIGNED_BYTE], "r8uint"),
    (R8I, RED_INTEGER, &[BYTE], "r8sint"),
    (R16UI, RED_INTEGER, &[UNSIGNED_SHORT], "r16uint"),
    (R16I, RED_INTEGER, &[SHORT], "r16sint"),
    (R32UI, RED_INTEGER, &[UNSIGNED_INT], "r32uint"),
    (R32I, RED_INTEGER, &[INT], "r32sint"),
    (RG8, RG, &[UNSIGNED_BYTE], "rg8unorm"),
    (RG8_SNORM, RG, &[BYTE], "rg8snorm"),
    (RG16F, RG, &[HALF_FLOAT, FLOAT], "rg16float"),
    (RG32F, RG, &[FLOAT], "rg32float"),
    (RG8UI, RG_INTEGER, &[UNSIGNED_BYTE], "rg8uint"),
    (RG8I, RG_INTEGER, &[BYTE], "rg8sint"),
    (RG16UI, RG_INTEGER, &[UNSIGNED_SHORT], "rg16uint"),
    (RG16I, RG_INTEGER, &[SHORT], "rg16sint"),
    (RG32UI, RG_INTEGER, &[UNSIGNED_INT], "rg32uint"),
    (RG32I, RG_INTEGER, &[INT], "rg32sint"),
    (RGB8, RGB, &[UNSIGNED_BYTE], "rgba8unorm"),
    (SRGB8, RGB, &[UNSIGNED_BYTE], "rgba8unorm-srgb"),
    (
        RGB565,
        RGB,
        &[UNSIGNED_BYTE, UNSIGNED_SHORT_5_6_5],
        "rgba8unorm",
    ),
    (RGB8_SNORM, RGB, &[BYTE], "rgba8snorm"),
    (
        R11F_G11F_B10F,
        RGB,
        &[UNSIGNED_INT_10F_11F_11F_REV, HALF_FLOAT, FLOAT],
        "rg11b10ufloat",
    ),
    (
        RGB9_E5,
        RGB,
        &[UNSIGNED_INT_5_9_9_9_REV, HALF_FLOAT, FLOAT],
        "rgb9e5ufloat",
    ),
    (RGB16F, RGB, &[HALF_FLOAT, FLOAT], "rgba16float"),
    (RGB32F, RGB, &[FLOAT], "rgba32float"),
    (RGB8UI, RGB_INTEGER, &[UNSIGNED_BYTE], "rgba8uint"),
    (RGB8I, RGB_INTEGER, &[BYTE], "rgba8sint"),
    (RGB16UI, RGB_INTEGER, &[UNSIGNED_SHORT], "rgba16uint"),
    (RGB16I, RGB_INTEGER, &[SHORT], "rgba16sint"),
    (RGB32UI, RGB_INTEGER, &[UNSIGNED_INT], "rgba32uint"),
    (RGB32I, RGB_INTEGER, &[INT], "rgba32sint"),
    (RGBA8, RGBA, &[UNSIGNED_BYTE], "rgba8unorm"),
    (SRGB8_ALPHA8, RGBA, &[UNSIGNED_BYTE], "rgba8unorm-srgb"),
    (RGBA8_SNORM, RGBA, &[BYTE], "rgba8snorm"),
    (
        RGB5_A1,
        RGBA,
        &[
            UNSIGNED_BYTE,
            UNSIGNED_SHORT_5_5_5_1,
            UNSIGNED_INT_2_10_10_10_REV,
        ],
        "rgba8unorm",
    ),
    (
        RGBA4,
        RGBA,
        &[UNSIGNED_BYTE, UNSIGNED_SHORT_4_4_4_4],
        "rgba8unorm",
    ),
    (
        RGB10_A2,
        RGBA,
        &[UNSIGNED_INT_2_10_10_10_REV],
        "rgb10a2unorm",
    ),
    (RGBA16F, RGBA, &[HALF_FLOAT, FLOAT], "rgba16float"),
    (RGBA32F, RGBA, &[FLOAT], "rgba32float"),
    (RGBA8UI, RGBA_INTEGER, &[UNSIGNED_BYTE], "rgba8uint"),
    (RGBA8I, RGBA_INTEGER, &[BYTE], "rgba8sint"),
    (
        RGB10_A2UI,
        RGBA_INTEGER,
        &[UNSIGNED_INT_2_10_10_10_REV],
        "rgb10a2uint",
    ),
    (RGBA16UI, RGBA_INTEGER, &[UNSIGNED_SHORT], "rgba16uint"),
    (RGBA16I, RGBA_INTEGER, &[SHORT], "rgba16sint"),
    (RGBA32I, RGBA_INTEGER, &[INT], "rgba32sint"),
    (RGBA32UI, RGBA_INTEGER, &[UNSIGNED_INT], "rgba32uint"),
    (
        DEPTH_COMPONENT16,
        DEPTH_COMPONENT,
        &[UNSIGNED_SHORT, UNSIGNED_INT],
        "depth16unorm",
    ),
    (
        DEPTH_COMPONENT24,
        DEPTH_COMPONENT,
        &[UNSIGNED_INT],
        "depth24plus",
    ),
    (
        DEPTH_COMPONENT32F,
        DEPTH_COMPONENT,
        &[FLOAT],
        "depth32float",
    ),
    (
        DEPTH24_STENCIL8,
        DEPTH_STENCIL,
        &[UNSIGNED_INT_24_8],
        "depth24plus-stencil8",
    ),
    (
        DEPTH32F_STENCIL8,
        DEPTH_STENCIL,
        &[FLOAT_32_UNSIGNED_INT_24_8_REV],
        "depth32float-stencil8",
    ),
];

/// WebGL 2's unsized internal formats (GL ES 3.0 table 3.3).
const UNSIZED: &[(u32, &[u32])] = &[
    (
        RGBA,
        &[
            UNSIGNED_BYTE,
            UNSIGNED_SHORT_4_4_4_4,
            UNSIGNED_SHORT_5_5_5_1,
        ],
    ),
    (RGB, &[UNSIGNED_BYTE, UNSIGNED_SHORT_5_6_5]),
    (LUMINANCE_ALPHA, &[UNSIGNED_BYTE]),
    (LUMINANCE, &[UNSIGNED_BYTE]),
    (ALPHA, &[UNSIGNED_BYTE]),
];

fn known_format(format: u32) -> bool {
    matches!(
        format,
        ALPHA
            | RGB
            | RGBA
            | LUMINANCE
            | LUMINANCE_ALPHA
            | RED
            | RG
            | RED_INTEGER
            | RG_INTEGER
            | RGB_INTEGER
            | RGBA_INTEGER
            | DEPTH_COMPONENT
            | DEPTH_STENCIL
            | SRGB_EXT
            | SRGB_ALPHA_EXT
    )
}

fn known_kind(kind: u32) -> bool {
    matches!(
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
            | UNSIGNED_INT_24_8
            | FLOAT_32_UNSIGNED_INT_24_8_REV
    )
}

/// GL's internal format, format and type as a texture's: the format, or
/// GL's error. `ext` says whether an extension is on (WebGL 1's).
pub(crate) fn texture_format(
    version: u8,
    ext: &dyn Fn(&str) -> bool,
    depth32_stencil: bool,
    internal: u32,
    format: u32,
    kind: u32,
) -> Result<Format, u32> {
    if version == 1 {
        return texture_format1(ext, internal, format, kind);
    }
    let sized = SIZED.iter().find(|s| s.0 == internal);
    let plain = UNSIZED.iter().find(|s| s.0 == internal);
    if sized.is_none() && plain.is_none() {
        return Err(INVALID_VALUE);
    }
    if !known_format(format) || matches!(format, SRGB_EXT | SRGB_ALPHA_EXT) {
        return Err(INVALID_ENUM);
    }
    if !known_kind(kind) || kind == HALF_FLOAT_OES {
        return Err(INVALID_ENUM);
    }
    if let Some(&(_, wanted, kinds, storage)) = sized {
        if wanted != format || !kinds.contains(&kind) {
            return Err(INVALID_OPERATION);
        }
        let storage = if storage == "depth32float-stencil8" && !depth32_stencil {
            "depth24plus-stencil8"
        } else {
            storage
        };
        return Ok(Format {
            internal,
            format,
            kind,
            storage,
        });
    }
    let (_, kinds) = plain.expect("found");
    if internal != format || !kinds.contains(&kind) {
        return Err(INVALID_OPERATION);
    }
    Ok(Format {
        internal,
        format,
        kind,
        storage: "rgba8unorm",
    })
}

/// WebGL 1's formats: unsized, some by extensions.
fn texture_format1(
    ext: &dyn Fn(&str) -> bool,
    internal: u32,
    format: u32,
    kind: u32,
) -> Result<Format, u32> {
    let format_ok = match format {
        ALPHA | RGB | RGBA | LUMINANCE | LUMINANCE_ALPHA => true,
        SRGB_EXT | SRGB_ALPHA_EXT => ext("EXT_sRGB"),
        DEPTH_COMPONENT | DEPTH_STENCIL => ext("WEBGL_depth_texture"),
        _ => false,
    };
    let kind_ok = match kind {
        UNSIGNED_BYTE | UNSIGNED_SHORT_5_6_5 | UNSIGNED_SHORT_4_4_4_4 | UNSIGNED_SHORT_5_5_5_1 => {
            true
        }
        FLOAT => ext("OES_texture_float"),
        HALF_FLOAT_OES => ext("OES_texture_half_float"),
        UNSIGNED_SHORT | UNSIGNED_INT | UNSIGNED_INT_24_8_WEBGL => ext("WEBGL_depth_texture"),
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
        (UNSIGNED_SHORT_5_6_5, RGB) | (UNSIGNED_SHORT_4_4_4_4 | UNSIGNED_SHORT_5_5_5_1, RGBA) => {
            "rgba8unorm"
        }
        (FLOAT, ALPHA | RGB | RGBA | LUMINANCE | LUMINANCE_ALPHA) => "rgba32float",
        (HALF_FLOAT_OES, ALPHA | RGB | RGBA | LUMINANCE | LUMINANCE_ALPHA) => "rgba16float",
        (UNSIGNED_SHORT, DEPTH_COMPONENT) => "depth16unorm",
        (UNSIGNED_INT, DEPTH_COMPONENT) => "depth32float",
        (UNSIGNED_INT_24_8_WEBGL, DEPTH_STENCIL) => "depth24plus-stencil8",
        _ => return Err(INVALID_OPERATION),
    };
    Ok(Format {
        internal,
        format,
        kind,
        storage,
    })
}

/// A sized internal format's own format and storage (for texStorage,
/// copies and renderbuffers); none for one not sized.
pub(crate) fn sized(internal: u32, depth32_stencil: bool) -> Option<Format> {
    let &(internal, format, kinds, storage) = SIZED.iter().find(|s| s.0 == internal)?;
    let storage = if storage == "depth32float-stencil8" && !depth32_stencil {
        "depth24plus-stencil8"
    } else {
        storage
    };
    Some(Format {
        internal,
        format,
        kind: kinds[0],
        storage,
    })
}

/// Whether a format can be drawn into as a colour attachment (GL ES 3.0
/// table 3.13, EXT_color_buffer_float's, and WebGL 1's extensions).
pub(crate) fn color_renderable(
    version: u8,
    ext: &dyn Fn(&str) -> bool,
    rg11b10: bool,
    f: &Format,
) -> bool {
    if version == 1 {
        return match f.storage {
            "rgba8unorm" => matches!(f.internal, RGB | RGBA | RGBA4 | RGB5_A1 | RGB565),
            "rgba8unorm-srgb" => matches!(f.internal, SRGB_ALPHA_EXT | SRGB8_ALPHA8_EXT),
            "rgba32float" => {
                matches!(f.internal, RGB | RGBA | RGBA32F_EXT) && ext("WEBGL_color_buffer_float")
            }
            "rgba16float" => {
                matches!(f.internal, RGB | RGBA | RGBA16F_EXT | RGB16F_EXT)
                    && ext("EXT_color_buffer_half_float")
            }
            _ => false,
        };
    }
    let float = ext("EXT_color_buffer_float");
    match f.internal {
        R8 | RG8 | RGB8 | RGB565 | RGBA4 | RGB5_A1 | RGBA8 | RGB10_A2 | RGB10_A2UI
        | SRGB8_ALPHA8 | R8I | R8UI | R16I | R16UI | R32I | R32UI | RG8I | RG8UI | RG16I
        | RG16UI | RG32I | RG32UI | RGBA8I | RGBA8UI | RGBA16I | RGBA16UI | RGBA32I | RGBA32UI
        | RGB | RGBA => true,
        R16F | RG16F | RGBA16F | R32F | RG32F | RGBA32F => float,
        R11F_G11F_B10F => float && rg11b10,
        _ => false,
    }
}

/// A renderbuffer's internal format's format; none for one it cannot
/// have.
pub(crate) fn renderbuffer_format(
    version: u8,
    ext: &dyn Fn(&str) -> bool,
    rg11b10: bool,
    depth32_stencil: bool,
    internal: u32,
) -> Option<Format> {
    let format = |storage: &'static str| Format {
        internal,
        format: 0,
        kind: 0,
        storage,
    };
    match internal {
        RGBA4 | RGB5_A1 | RGB565 => return Some(format("rgba8unorm")),
        DEPTH_COMPONENT16 => return Some(format("depth16unorm")),
        STENCIL_INDEX8 => return Some(format("stencil8")),
        DEPTH_STENCIL => return Some(format("depth24plus-stencil8")),
        _ => {}
    }
    if version == 1 {
        return match internal {
            SRGB8_ALPHA8_EXT if ext("EXT_sRGB") => Some(format("rgba8unorm-srgb")),
            RGBA32F_EXT if ext("WEBGL_color_buffer_float") => Some(format("rgba32float")),
            RGBA16F_EXT | RGB16F_EXT if ext("EXT_color_buffer_half_float") => {
                Some(format("rgba16float"))
            }
            _ => None,
        };
    }
    let mut f = sized(internal, depth32_stencil)?;
    f.format = 0;
    f.kind = 0;
    (f.depth() || color_renderable(version, ext, rg11b10, &f)).then_some(f)
}

/// How rows of client data are laid out (the unpack or pack state).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    pub alignment: u32,
    pub row_length: u32,
    pub image_height: u32,
    pub skip_pixels: u32,
    pub skip_rows: u32,
    pub skip_images: u32,
}

impl Layout {
    pub(crate) fn packed(alignment: u32) -> Layout {
        Layout {
            alignment,
            row_length: 0,
            image_height: 0,
            skip_pixels: 0,
            skip_rows: 0,
            skip_images: 0,
        }
    }

    /// A row's stride, an image's, where the first pixel is, and the
    /// bytes `w` by `h` by `d` pixels of `pixel` bytes take.
    pub(crate) fn measure(&self, w: u32, h: u32, d: u32, pixel: u32) -> (u64, u64, u64, u64) {
        let row_pixels = if self.row_length > 0 {
            self.row_length
        } else {
            w
        };
        let row = u64::from(row_pixels) * u64::from(pixel);
        let align = u64::from(self.alignment.max(1));
        let stride = row.div_ceil(align) * align;
        let image_rows = if self.image_height > 0 {
            self.image_height
        } else {
            h
        };
        let image = stride * u64::from(image_rows);
        let first = u64::from(self.skip_images) * image
            + u64::from(self.skip_rows) * stride
            + u64::from(self.skip_pixels) * u64::from(pixel);
        let whole = if w == 0 || h == 0 || d == 0 {
            0
        } else {
            first
                + u64::from(d - 1) * image
                + u64::from(h - 1) * stride
                + u64::from(w) * u64::from(pixel)
        };
        (stride, image, first, whole)
    }
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

/// A non-negative float as an unsigned float of `mantissa` bits and 5
/// of exponent (the 11- and 10-bit floats of R11F_G11F_B10F).
fn small_float(v: f32, mantissa: u32) -> u32 {
    if v.is_nan() {
        return (31 << mantissa) | 1;
    }
    let v = v.max(0.0);
    let half = u32::from(f32_to_f16(v));
    // A half's exponent and the top of its mantissa.
    let exponent = (half >> 10) & 0x1F;
    let m = (half & 0x3FF) >> (10 - mantissa);
    (exponent << mantissa) | m
}

fn small_float_back(bits: u32, mantissa: u32) -> f32 {
    let exponent = (bits >> mantissa) & 0x1F;
    let m = bits & ((1 << mantissa) - 1);
    f16_to_f32(((exponent << 10) | (m << (10 - mantissa))) as u16)
}

fn rgb9e5(c: [f32; 3]) -> u32 {
    const MAX: f32 = 65408.0;
    let c = c.map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, MAX) });
    let most = c[0].max(c[1]).max(c[2]);
    let mut exponent = (most.max(f32::MIN_POSITIVE).log2().floor() as i32).max(-16) + 1 + 15;
    let mut scale = 2f32.powi(exponent - 15 - 9);
    if (most / scale + 0.5).floor() as u32 == 512 {
        exponent += 1;
        scale *= 2.0;
    }
    let m = c.map(|v| ((v / scale + 0.5).floor() as u32).min(511));
    m[0] | (m[1] << 9) | (m[2] << 18) | ((exponent.clamp(0, 31) as u32) << 27)
}

fn rgb9e5_back(v: u32) -> [f32; 3] {
    let scale = 2f32.powi(((v >> 27) as i32) - 15 - 9);
    [v & 511, (v >> 9) & 511, (v >> 18) & 511].map(|m| m as f32 * scale)
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

/// A format's channels put in RGBA's places: the missing colours 0 or
/// the luminance's, a missing alpha `one`.
fn expand<T: Copy>(format: u32, c: &[T], zero: T, one: T) -> [T; 4] {
    match format {
        ALPHA => [zero, zero, zero, c[0]],
        LUMINANCE => [c[0], c[0], c[0], one],
        LUMINANCE_ALPHA => [c[0], c[0], c[0], c[1]],
        RED | RED_INTEGER | DEPTH_COMPONENT => [c[0], zero, zero, one],
        RG | RG_INTEGER => [c[0], c[1], zero, one],
        RGB | SRGB_EXT | RGB_INTEGER => [c[0], c[1], c[2], one],
        _ => [c[0], c[1], c[2], c[3]],
    }
}

/// A component's "one" in an encoding: the largest normalized value, or
/// 1 for integers and floats.
fn one(comp: Comp, integer: bool) -> u32 {
    match (comp, integer) {
        (Comp::F16, _) => 0x3C00,
        (Comp::F32, _) => 1.0f32.to_bits(),
        (_, true) => 1,
        (Comp::U8, _) => 0xFF,
        (Comp::S8, _) => 0x7F,
        (Comp::U16, _) => 0xFFFF,
        (Comp::S16, _) => 0x7FFF,
        _ => u32::MAX,
    }
}

fn read_comp(p: &[u8], comp: Comp) -> u32 {
    match comp.size() {
        1 => u32::from(p[0]),
        2 => u32::from(u16::from_le_bytes([p[0], p[1]])),
        _ => u32::from_le_bytes([p[0], p[1], p[2], p[3]]),
    }
}

fn write_comp(out: &mut [u8], comp: Comp, v: u32) {
    match comp.size() {
        1 => out[0] = v as u8,
        2 => out[..2].copy_from_slice(&(v as u16).to_le_bytes()),
        _ => out[..4].copy_from_slice(&v.to_le_bytes()),
    }
}

/// A component as a float of its value (normalized ones in [0, 1] or
/// [-1, 1]).
fn comp_float(v: u32, comp: Comp, integer: bool) -> f32 {
    match (comp, integer) {
        (Comp::F16, _) => f16_to_f32(v as u16),
        (Comp::F32, _) => f32::from_bits(v),
        (Comp::U8, false) => v as f32 / 255.0,
        (Comp::S8, false) => (f32::from(v as u8 as i8) / 127.0).max(-1.0),
        (Comp::U16, false) => v as f32 / 65535.0,
        (Comp::S16, false) => (f32::from(v as u16 as i16) / 32767.0).max(-1.0),
        (Comp::S8, true) => f32::from(v as u8 as i8),
        (Comp::S16, true) => f32::from(v as u16 as i16),
        (Comp::S32, _) => v as i32 as f32,
        _ => v as f32,
    }
}

/// One client pixel as a texel of `fmt`'s storage, its colour
/// multiplied by alpha when asked.
fn texel(fmt: &Format, p: &[u8], out: &mut [u8], premultiply: bool) {
    let (s_channels, s_comp, _) = info(fmt.storage);
    match fmt.kind {
        UNSIGNED_SHORT_5_6_5 | UNSIGNED_SHORT_4_4_4_4 | UNSIGNED_SHORT_5_5_5_1 => {
            let v = u16::from_le_bytes([p[0], p[1]]);
            let mut c = match fmt.kind {
                UNSIGNED_SHORT_5_6_5 => unpack565(v),
                UNSIGNED_SHORT_4_4_4_4 => unpack4444(v),
                _ => unpack5551(v),
            };
            if premultiply {
                let a = u32::from(c[3]);
                for v in &mut c[..3] {
                    *v = ((u32::from(*v) * a + 127) / 255) as u8;
                }
            }
            out[..4].copy_from_slice(&c);
        }
        UNSIGNED_INT_2_10_10_10_REV => {
            let v = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
            if s_comp == Comp::Packed {
                out[..4].copy_from_slice(&v.to_le_bytes());
            } else {
                // RGB5_A1's: to 8 bits each.
                let c10 = |s: u32| (((v >> s) & 1023) * 255 / 1023) as u8;
                out[..4].copy_from_slice(&[c10(0), c10(10), c10(20), ((v >> 30) * 85) as u8]);
            }
        }
        UNSIGNED_INT_10F_11F_11F_REV | UNSIGNED_INT_5_9_9_9_REV => {
            out[..4].copy_from_slice(&p[..4])
        }
        UNSIGNED_INT_24_8 | FLOAT_32_UNSIGNED_INT_24_8_REV => {}
        kind => {
            let c_comp = client_comp(kind);
            let integer = integer_format(fmt.format);
            let n = channels(fmt.format) as usize;
            let values: Vec<u32> = (0..n)
                .map(|k| read_comp(&p[k * c_comp.size()..], c_comp))
                .collect();
            let rgba = expand(fmt.format, &values, 0, one(c_comp, integer));
            if s_comp == Comp::Packed {
                // Floats into R11F_G11F_B10F's or RGB9_E5's bits.
                let f = rgba.map(|v| comp_float(v, c_comp, false));
                let bits = if fmt.storage == "rg11b10ufloat" {
                    small_float(f[0], 6)
                        | (small_float(f[1], 6) << 11)
                        | (small_float(f[2], 5) << 22)
                } else {
                    rgb9e5([f[0], f[1], f[2]])
                };
                out[..4].copy_from_slice(&bits.to_le_bytes());
                return;
            }
            let premultiply = premultiply && !integer && channels(fmt.format) == 4 || {
                premultiply && fmt.format == LUMINANCE_ALPHA
            };
            let alpha = comp_float(rgba[3], c_comp, false);
            for k in 0..s_channels {
                let mut v = rgba[k];
                if premultiply && k < 3 {
                    let f = comp_float(v, c_comp, false) * alpha;
                    v = match c_comp {
                        Comp::F32 => f.to_bits(),
                        Comp::F16 => u32::from(f32_to_f16(f)),
                        Comp::U8 => (f * 255.0).round() as u32,
                        Comp::U16 => (f * 65535.0).round() as u32,
                        _ => v,
                    };
                }
                if c_comp == Comp::F32 && s_comp == Comp::F16 {
                    v = u32::from(f32_to_f16(f32::from_bits(v)));
                }
                write_comp(&mut out[k * s_comp.size()..], s_comp, v);
            }
        }
    }
}

/// Client rows of `fmt` (laid out by `layout`, `d` images of `h` rows
/// of `w`) as the storage's texels: rows turned over and colours
/// multiplied by alpha as asked.
#[allow(clippy::too_many_arguments)]
pub(crate) fn texels(
    fmt: &Format,
    w: u32,
    h: u32,
    d: u32,
    data: &[u8],
    layout: &Layout,
    flip: bool,
    premultiply: bool,
) -> Vec<u8> {
    let pixel = client_pixel(fmt.format, fmt.kind) as usize;
    let (stride, image, first, _) = layout.measure(w, h, d, pixel as u32);
    let size = texel_bytes(fmt.storage) as usize;
    let (w, h, d) = (w as usize, h as usize, d as usize);
    let mut out = vec![0u8; w * h * d * size];
    for z in 0..d {
        for y in 0..h {
            let source_row = if flip { h - 1 - y } else { y };
            let at = first as usize + z * image as usize + source_row * stride as usize;
            for x in 0..w {
                let p = &data[at + x * pixel..at + x * pixel + pixel];
                let o = ((z * h + y) * w + x) * size;
                texel(fmt, p, &mut out[o..o + size], premultiply);
            }
        }
    }
    out
}

/// An image source's pixels (RGBA, already turned over and multiplied by
/// alpha as asked) as client data of `fmt`, then its texels.
pub(crate) fn from_rgba8(fmt: &Format, w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let quantize = |c: u8, bits: u32| u16::from(c) >> (8 - bits);
    let comp = client_comp(fmt.kind);
    let integer = integer_format(fmt.format);
    let mut client = Vec::new();
    for p in rgba.as_chunks::<4>().0.iter().take(w as usize * h as usize) {
        let picked: Vec<u8> = match fmt.format {
            ALPHA => vec![p[3]],
            LUMINANCE | RED | RED_INTEGER => vec![p[0]],
            LUMINANCE_ALPHA => vec![p[0], p[3]],
            RG | RG_INTEGER => vec![p[0], p[1]],
            RGB | SRGB_EXT | RGB_INTEGER => vec![p[0], p[1], p[2]],
            _ => p.to_vec(),
        };
        match fmt.kind {
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
            UNSIGNED_INT_2_10_10_10_REV => {
                let c = |v: u8| u32::from(v) * 1023 / 255;
                let v = c(p[0])
                    | (c(p[1]) << 10)
                    | (c(p[2]) << 20)
                    | ((u32::from(p[3]) * 3 / 255) << 30);
                client.extend(v.to_le_bytes());
            }
            UNSIGNED_INT_10F_11F_11F_REV => {
                let f = |v: u8| f32::from(v) / 255.0;
                let v = small_float(f(p[0]), 6)
                    | (small_float(f(p[1]), 6) << 11)
                    | (small_float(f(p[2]), 5) << 22);
                client.extend(v.to_le_bytes());
            }
            UNSIGNED_INT_5_9_9_9_REV => {
                let f = |v: u8| f32::from(v) / 255.0;
                client.extend(rgb9e5([f(p[0]), f(p[1]), f(p[2])]).to_le_bytes());
            }
            _ => {
                for c in picked {
                    let v = match (comp, integer) {
                        (_, true) => u32::from(c),
                        (Comp::F32, _) => (f32::from(c) / 255.0).to_bits(),
                        (Comp::F16, _) => u32::from(f32_to_f16(f32::from(c) / 255.0)),
                        (Comp::S8, _) => u32::from(c >> 1),
                        (Comp::U16, _) => u32::from(c) * 257,
                        (Comp::S16, _) => u32::from(c) * 257 / 2,
                        _ => u32::from(c),
                    };
                    let mut bytes = [0u8; 4];
                    write_comp(&mut bytes, comp, v);
                    client.extend_from_slice(&bytes[..comp.size()]);
                }
            }
        }
    }
    texels(fmt, w, h, 1, &client, &Layout::packed(1), false, false)
}

/// A texel of `storage` as four channels (a missing colour 0, a missing
/// alpha 1), and whether they are integers.
pub(crate) fn decode(storage: &str, t: &[u8]) -> ([f64; 4], bool) {
    let (n, comp, sample) = info(storage);
    let integer = matches!(sample, Sample::Sint | Sample::Uint);
    let mut out = [0.0, 0.0, 0.0, 1.0];
    match storage {
        "rgb10a2unorm" | "rgb10a2uint" => {
            let v = u32::from_le_bytes([t[0], t[1], t[2], t[3]]);
            let scale = |bits: u32, max: f64| {
                if integer {
                    f64::from(bits)
                } else {
                    f64::from(bits) / max
                }
            };
            out = [
                scale(v & 1023, 1023.0),
                scale((v >> 10) & 1023, 1023.0),
                scale((v >> 20) & 1023, 1023.0),
                scale(v >> 30, 3.0),
            ];
        }
        "rg11b10ufloat" => {
            let v = u32::from_le_bytes([t[0], t[1], t[2], t[3]]);
            out[0] = f64::from(small_float_back(v & 0x7FF, 6));
            out[1] = f64::from(small_float_back((v >> 11) & 0x7FF, 6));
            out[2] = f64::from(small_float_back(v >> 22, 5));
        }
        "rgb9e5ufloat" => {
            let c = rgb9e5_back(u32::from_le_bytes([t[0], t[1], t[2], t[3]]));
            out[..3].copy_from_slice(&c.map(f64::from));
        }
        _ => {
            for (k, slot) in out.iter_mut().enumerate().take(n) {
                let v = read_comp(&t[k * comp.size()..], comp);
                *slot = f64::from(comp_float(v, comp, integer));
            }
        }
    }
    (out, integer)
}

/// Four channels as a client pixel of `format` and `kind` (readPixels'
/// formats: RGBA, RGBA_INTEGER and a colour buffer's own).
pub(crate) fn encode(format: u32, kind: u32, c: [f64; 4], out: &mut [u8]) {
    let picked: Vec<f64> = match format {
        ALPHA => vec![c[3]],
        RED | RED_INTEGER | LUMINANCE => vec![c[0]],
        RG | RG_INTEGER => vec![c[0], c[1]],
        LUMINANCE_ALPHA => vec![c[0], c[3]],
        RGB | RGB_INTEGER => vec![c[0], c[1], c[2]],
        _ => c.to_vec(),
    };
    match kind {
        UNSIGNED_SHORT_5_6_5 | UNSIGNED_SHORT_4_4_4_4 | UNSIGNED_SHORT_5_5_5_1 => {
            let q = |v: f64, bits: u32| {
                ((v.clamp(0.0, 1.0) * f64::from((1 << bits) - 1)).round()) as u16
            };
            let v = match kind {
                UNSIGNED_SHORT_5_6_5 => (q(c[0], 5) << 11) | (q(c[1], 6) << 5) | q(c[2], 5),
                UNSIGNED_SHORT_4_4_4_4 => {
                    (q(c[0], 4) << 12) | (q(c[1], 4) << 8) | (q(c[2], 4) << 4) | q(c[3], 4)
                }
                _ => (q(c[0], 5) << 11) | (q(c[1], 5) << 6) | (q(c[2], 5) << 1) | q(c[3], 1),
            };
            out[..2].copy_from_slice(&v.to_le_bytes());
        }
        UNSIGNED_INT_2_10_10_10_REV => {
            let integer = format == RGBA_INTEGER;
            let q = |v: f64, max: f64| {
                if integer {
                    v.clamp(0.0, max) as u32
                } else {
                    (v.clamp(0.0, 1.0) * max).round() as u32
                }
            };
            let v = q(c[0], 1023.0)
                | (q(c[1], 1023.0) << 10)
                | (q(c[2], 1023.0) << 20)
                | (q(c[3], 3.0) << 30);
            out[..4].copy_from_slice(&v.to_le_bytes());
        }
        UNSIGNED_INT_10F_11F_11F_REV => {
            let v = small_float(c[0] as f32, 6)
                | (small_float(c[1] as f32, 6) << 11)
                | (small_float(c[2] as f32, 5) << 22);
            out[..4].copy_from_slice(&v.to_le_bytes());
        }
        _ => {
            let comp = client_comp(kind);
            let integer = integer_format(format);
            for (k, &v) in picked.iter().enumerate() {
                let bits = match (comp, integer) {
                    (Comp::F32, _) => (v as f32).to_bits(),
                    (Comp::F16, _) => u32::from(f32_to_f16(v as f32)),
                    (Comp::S8 | Comp::S16 | Comp::S32, true) => v as i64 as i32 as u32,
                    (_, true) => v.max(0.0) as u64 as u32,
                    (Comp::U8, false) => (v.clamp(0.0, 1.0) * 255.0).round() as u32,
                    (Comp::S8, false) => (v.clamp(-1.0, 1.0) * 127.0).round() as i32 as u32,
                    (Comp::U16, false) => (v.clamp(0.0, 1.0) * 65535.0).round() as u32,
                    (Comp::S16, false) => (v.clamp(-1.0, 1.0) * 32767.0).round() as i32 as u32,
                    _ => (v.clamp(0.0, 1.0) * f64::from(u32::MAX)).round() as u32,
                };
                write_comp(&mut out[k * comp.size()..], comp, bits);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_and_small_floats_convert_both_ways() {
        for v in [0.0f32, 1.0, -2.5, 0.5, 65504.0, 6.0e-8, 0.333_333] {
            let back = f16_to_f32(f32_to_f16(v));
            assert!((back - v).abs() <= v.abs() / 1000.0 + 6.0e-8, "{v} {back}");
        }
        assert_eq!(f32_to_f16(1.0), 0x3C00);
        assert_eq!(f32_to_f16(1.0e9), 0x7C00);
        assert_eq!(small_float_back(small_float(1.5, 6), 6), 1.5);
        assert_eq!(small_float_back(small_float(-3.0, 5), 5), 0.0);
        assert_eq!(rgb9e5_back(rgb9e5([1.0, 0.5, 0.25])), [1.0, 0.5, 0.25]);
    }

    #[test]
    fn client_rows_become_texels() {
        let fmt = |internal, format, kind, storage| Format {
            internal,
            format,
            kind,
            storage,
        };
        let packed = |a| Layout::packed(a);
        // Two rows of a 1-wide LUMINANCE_ALPHA at alignment 4: padded.
        let la = texels(
            &fmt(
                LUMINANCE_ALPHA,
                LUMINANCE_ALPHA,
                UNSIGNED_BYTE,
                "rgba8unorm",
            ),
            1,
            2,
            1,
            &[10, 20, 0, 0, 30, 40],
            &packed(4),
            false,
            false,
        );
        assert_eq!(la, [10, 10, 10, 20, 30, 30, 30, 40]);
        // Turned over and premultiplied.
        let rgba = texels(
            &fmt(RGBA, RGBA, UNSIGNED_BYTE, "rgba8unorm"),
            1,
            2,
            1,
            &[255, 0, 0, 128, 0, 255, 0, 255],
            &packed(1),
            true,
            true,
        );
        assert_eq!(rgba, [0, 255, 0, 255, 128, 0, 0, 128]);
        assert_eq!(unpack565(0xF800), [255, 0, 0, 255]);
        assert_eq!(unpack4444(0x000F), [0, 0, 0, 255]);
        assert_eq!(unpack5551(0x07C1), [0, 255, 0, 255]);
        // RGB integers into RGBA: alpha 1, not the largest.
        let ints = texels(
            &fmt(RGB8UI, RGB_INTEGER, UNSIGNED_BYTE, "rgba8uint"),
            1,
            1,
            1,
            &[7, 8, 9],
            &packed(1),
            false,
            false,
        );
        assert_eq!(ints, [7, 8, 9, 1]);
        // Floats into halves; R and RG kept to their channels.
        let half = texels(
            &fmt(RG16F, RG, FLOAT, "rg16float"),
            1,
            1,
            1,
            &[0.5f32.to_le_bytes(), 2.0f32.to_le_bytes()].concat(),
            &packed(4),
            false,
            false,
        );
        assert_eq!(half, [0x00, 0x38, 0x00, 0x40]);
        // A row length and skipped pixels, rows and images.
        let layout = Layout {
            alignment: 1,
            row_length: 3,
            image_height: 2,
            skip_pixels: 1,
            skip_rows: 1,
            skip_images: 1,
        };
        let data: Vec<u8> = (0..12).collect();
        let r8 = texels(
            &fmt(R8, RED, UNSIGNED_BYTE, "r8unorm"),
            2,
            1,
            1,
            &data,
            &layout,
            false,
            false,
        );
        assert_eq!(r8, [10, 11]);
        assert_eq!(layout.measure(2, 1, 1, 1), (3, 6, 10, 12));
        // Read back: an R8's red, green 0 and alpha 1, as RGBA bytes.
        let (c, integer) = decode("r8unorm", &[255]);
        assert!(!integer);
        let mut out = [0u8; 4];
        encode(RGBA, UNSIGNED_BYTE, c, &mut out);
        assert_eq!(out, [255, 0, 0, 255]);
        let (c, integer) = decode("rgba16sint", &[0xFE, 0xFF, 2, 0, 0, 0, 1, 0]);
        assert!(integer);
        let mut out = [0u8; 16];
        encode(RGBA_INTEGER, INT, c, &mut out);
        assert_eq!(&out[..8], &[0xFE, 0xFF, 0xFF, 0xFF, 2, 0, 0, 0]);
    }
}
