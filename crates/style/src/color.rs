//! Colours past CSS Color 3 (gap batch E, D48,
//! `docs/design/boxes-colour.md`): the colour spaces of css-color-4
//! (section 10's `lab()`, `lch()`, `oklab()` and `oklch()`, section 10.8's
//! predefined spaces of `color()`), the conversions between them by its
//! sample code (section 18), interpolation by section 12, and css-color-5's
//! `color-mix()`.
//!
//! A [`Color`] keeps the space it was written in and its components
//! there, with which of them are `none`, beside the sRGB channels drawing
//! reads, worked out once when the colour is made. The legacy forms
//! (hexadecimal, named, `rgb()`) are [`Space::Legacy`], whose components
//! are those sRGB channels; `hsl()` and `hwb()` are [`Space::Hsl`] and
//! [`Space::Hwb`], legacy too, which keep theirs for a mix as the
//! reference browser does.

use css::Token;

use crate::calc::{self, LengthUnit, Want};
use crate::computed::Color;

/// The deepest `color-mix()` nests in another: a deeper one is invalid,
/// as calculations past `calc::MAX_DEPTH` are.
pub const MAX_DEPTH: usize = 32;

/// A colour space (css-color-4 sections 7 to 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Space {
    /// The legacy sRGB forms, serialized as `rgb()`.
    Legacy,
    Srgb,
    SrgbLinear,
    DisplayP3,
    DisplayP3Linear,
    A98Rgb,
    ProphotoRgb,
    Rec2020,
    XyzD50,
    XyzD65,
    Lab,
    Lch,
    Oklab,
    Oklch,
    /// `hsl()`, a legacy colour, and an interpolation space: a colour
    /// mixed in it is [`Space::Srgb`].
    Hsl,
    /// `hwb()`, likewise.
    Hwb,
}

impl Space {
    /// The name `color()` writes, for its predefined spaces.
    pub fn predefined_name(self) -> Option<&'static str> {
        Some(match self {
            Space::Srgb => "srgb",
            Space::SrgbLinear => "srgb-linear",
            Space::DisplayP3 => "display-p3",
            Space::DisplayP3Linear => "display-p3-linear",
            Space::A98Rgb => "a98-rgb",
            Space::ProphotoRgb => "prophoto-rgb",
            Space::Rec2020 => "rec2020",
            Space::XyzD50 => "xyz-d50",
            Space::XyzD65 => "xyz-d65",
            _ => return None,
        })
    }

    fn predefined(name: &str) -> Option<Space> {
        Some(match name.to_ascii_lowercase().as_str() {
            "srgb" => Space::Srgb,
            "srgb-linear" => Space::SrgbLinear,
            "display-p3" => Space::DisplayP3,
            "display-p3-linear" => Space::DisplayP3Linear,
            "a98-rgb" => Space::A98Rgb,
            "prophoto-rgb" => Space::ProphotoRgb,
            "rec2020" => Space::Rec2020,
            "xyz" | "xyz-d65" => Space::XyzD65,
            "xyz-d50" => Space::XyzD50,
            _ => return None,
        })
    }

    /// An interpolation method's space (css-color-4 section 12.1).
    fn interpolation(name: &str) -> Option<Space> {
        Some(match name.to_ascii_lowercase().as_str() {
            "lab" => Space::Lab,
            "lch" => Space::Lch,
            "oklab" => Space::Oklab,
            "oklch" => Space::Oklch,
            "hsl" => Space::Hsl,
            "hwb" => Space::Hwb,
            other => return Space::predefined(other),
        })
    }

    /// Whether the space has a hue, its third component.
    pub fn is_polar(self) -> bool {
        matches!(self, Space::Lch | Space::Oklch | Space::Hsl | Space::Hwb)
    }

    /// The analogous component category of each component (section
    /// 12.2), by which a missing one carries over to another space.
    fn categories(self) -> [Category; 3] {
        use Category::*;
        match self {
            Space::Lab | Space::Oklab => [Lightness, OpponentA, OpponentB],
            Space::Lch | Space::Oklch => [Lightness, Colorfulness, Hue],
            Space::Hsl => [Hue, Colorfulness, Lightness],
            Space::Hwb => [Hue, Other, Other],
            _ => [Reds, Greens, Blues],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Reds,
    Greens,
    Blues,
    Lightness,
    Colorfulness,
    Hue,
    OpponentA,
    OpponentB,
    Other,
}

/// How hues interpolate (section 12.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hue {
    #[default]
    Shorter,
    Longer,
    Increasing,
    Decreasing,
}

/// A `<color-interpolation-method>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Method {
    pub space: Space,
    pub hue: Hue,
}

impl Method {
    /// Oklab, css-color-4's default for colours that are not legacy.
    pub const OKLAB: Method = Method {
        space: Space::Oklab,
        hue: Hue::Shorter,
    };
}

/// What resolving a colour knows of the element whose value it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Context {
    /// What `currentcolor` is.
    pub current: Color,
}

impl Default for Context {
    fn default() -> Self {
        Context {
            current: Color::BLACK,
        }
    }
}

/// The largest a component's magnitude is kept at: past it, a colour's
/// conversions leave what an `f32` holds.
pub const MAX_COMPONENT: f64 = 1e6;

/// A colour written as CSS writes one, without `currentcolor` (which
/// `color-mix()` does not then take): `None` for any other tokens.
pub fn parse(tokens: &[Token]) -> Option<Color> {
    crate::cascade::parse_color(tokens)
}

/// A bit of [`Color::missing`] a component is `none` by; alpha's is 3.
const ALPHA: u8 = 1 << 3;

impl Color {
    /// A legacy sRGB colour from 0 to 1 channels.
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
        Color {
            r,
            g,
            b,
            a,
            space: Space::Legacy,
            c: [r, g, b],
            missing: 0,
        }
    }

    /// A colour of `space`, its components there, `missing` the bits of
    /// those that are `none` (alpha's the fourth), which count as zero.
    pub fn of(space: Space, c: [f64; 3], alpha: f64, missing: u8) -> Color {
        let c = c.map(|v| v.clamp(-MAX_COMPONENT, MAX_COMPONENT));
        let given = [0, 1, 2].map(|i| if missing & (1 << i) != 0 { 0.0 } else { c[i] });
        let alpha = if missing & ALPHA != 0 { 0.0 } else { alpha };
        let [r, g, b] = match space {
            Space::Legacy | Space::Srgb => given,
            _ => from_xyz65(Space::Srgb, to_xyz65(space, given)),
        }
        .map(|v| if v.is_finite() { v } else { 0.0 });
        let space = match space {
            Space::Hsl | Space::Hwb => Space::Srgb,
            other => other,
        };
        let c = if space == Space::Legacy { [r, g, b] } else { c };
        Color {
            r: r as f32,
            g: g as f32,
            b: b as f32,
            a: alpha as f32,
            space,
            c: c.map(|v| v as f32),
            missing,
        }
    }

    /// A legacy colour of `space` ([`Space::Legacy`]'s sRGB channels,
    /// [`Space::Hsl`] or [`Space::Hwb`]), its components there, `missing`
    /// the bits of those that are `none`, which count as zero.
    pub fn legacy(space: Space, c: [f64; 3], alpha: f64, missing: u8) -> Color {
        let given = [0, 1, 2].map(|i| if missing & (1 << i) != 0 { 0.0 } else { c[i] });
        let [r, g, b] = match space {
            Space::Hsl => hsl_to_srgb(given),
            Space::Hwb => hwb_to_srgb(given),
            _ => given,
        };
        Color {
            r: r as f32,
            g: g as f32,
            b: b as f32,
            a: if missing & ALPHA != 0 {
                0.0
            } else {
                alpha as f32
            },
            space,
            c: c.map(|v| v as f32),
            missing,
        }
    }

    /// The space the colour was written in.
    pub fn space(&self) -> Space {
        self.space
    }

    /// The components in [`Color::space`], `None` where `none`.
    pub fn components(&self) -> [Option<f32>; 3] {
        [0, 1, 2].map(|i| (self.missing & (1 << i) == 0).then_some(self.c[i]))
    }

    /// The alpha, `None` where `none`.
    pub fn alpha(&self) -> Option<f32> {
        (self.missing & ALPHA == 0).then_some(self.a)
    }

    /// Whether the colour is one of the legacy forms.
    pub fn is_legacy(&self) -> bool {
        matches!(self.space, Space::Legacy | Space::Hsl | Space::Hwb)
    }
}

impl PartialEq for Color {
    /// Two legacy colours are equal by their channels; others by their
    /// space, components and alpha.
    fn eq(&self, other: &Color) -> bool {
        let channels = self.r == other.r && self.g == other.g && self.b == other.b;
        if self.is_legacy() && other.is_legacy() {
            return channels && self.a == other.a;
        }
        self.space == other.space
            && self.c == other.c
            && self.missing == other.missing
            && self.a == other.a
    }
}

/// The system colours (css-color-4 section 6.2, and appendix A's
/// deprecated ones), by name in ASCII order for a binary search, light
/// then dark (gap batch E, D49): the reference browser's values.
const SYSTEM: [(&str, [u8; 3], [u8; 3]); 42] = [
    ("accentcolor", [0, 117, 255], [0, 117, 255]),
    ("accentcolortext", [255, 255, 255], [255, 255, 255]),
    ("activeborder", [0, 0, 0], [255, 255, 255]),
    ("activecaption", [255, 255, 255], [18, 18, 18]),
    ("activetext", [0, 102, 204], [255, 0, 0]),
    ("appworkspace", [255, 255, 255], [18, 18, 18]),
    ("background", [255, 255, 255], [18, 18, 18]),
    ("buttonborder", [0, 0, 0], [255, 255, 255]),
    ("buttonface", [240, 240, 240], [107, 107, 107]),
    ("buttonhighlight", [240, 240, 240], [107, 107, 107]),
    ("buttonshadow", [240, 240, 240], [107, 107, 107]),
    ("buttontext", [0, 0, 0], [255, 255, 255]),
    ("canvas", [255, 255, 255], [18, 18, 18]),
    ("canvastext", [0, 0, 0], [255, 255, 255]),
    ("captiontext", [0, 0, 0], [255, 255, 255]),
    ("field", [255, 255, 255], [59, 59, 59]),
    ("fieldtext", [0, 0, 0], [255, 255, 255]),
    ("graytext", [109, 109, 109], [128, 128, 128]),
    ("highlight", [0, 120, 215], [25, 103, 210]),
    ("highlighttext", [255, 255, 255], [255, 255, 255]),
    ("inactiveborder", [0, 0, 0], [255, 255, 255]),
    ("inactivecaption", [255, 255, 255], [18, 18, 18]),
    ("inactivecaptiontext", [128, 128, 128], [128, 128, 128]),
    ("infobackground", [255, 255, 255], [18, 18, 18]),
    ("infotext", [0, 0, 0], [255, 255, 255]),
    ("linktext", [0, 102, 204], [158, 158, 255]),
    ("mark", [255, 255, 0], [255, 255, 0]),
    ("marktext", [0, 0, 0], [0, 0, 0]),
    ("menu", [255, 255, 255], [18, 18, 18]),
    ("menutext", [0, 0, 0], [255, 255, 255]),
    ("scrollbar", [255, 255, 255], [18, 18, 18]),
    ("selecteditem", [25, 103, 210], [153, 200, 255]),
    ("selecteditemtext", [255, 255, 255], [59, 59, 59]),
    ("threeddarkshadow", [0, 0, 0], [255, 255, 255]),
    ("threedface", [240, 240, 240], [107, 107, 107]),
    ("threedhighlight", [0, 0, 0], [255, 255, 255]),
    ("threedlightshadow", [0, 0, 0], [255, 255, 255]),
    ("threedshadow", [0, 0, 0], [255, 255, 255]),
    ("visitedtext", [0, 102, 204], [208, 173, 240]),
    ("window", [255, 255, 255], [18, 18, 18]),
    ("windowframe", [0, 0, 0], [255, 255, 255]),
    ("windowtext", [0, 0, 0], [255, 255, 255]),
];

/// The system colour `name` is, by the light or the dark scheme.
pub fn system(name: &str, dark: bool) -> Option<Color> {
    let lower = name.to_ascii_lowercase();
    let at = SYSTEM
        .binary_search_by(|(n, ..)| n.cmp(&lower.as_str()))
        .ok()?;
    let (_, light, dusk) = SYSTEM[at];
    let [r, g, b] = if dark { dusk } else { light };
    Some(Color::rgb8(r, g, b))
}

type Matrix = [[f64; 3]; 3];

fn multiply(m: &Matrix, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

const LIN_SRGB_TO_XYZ: Matrix = [
    [506752.0 / 1228815.0, 87881.0 / 245763.0, 12673.0 / 70218.0],
    [87098.0 / 409605.0, 175762.0 / 245763.0, 12673.0 / 175545.0],
    [7918.0 / 409605.0, 87881.0 / 737289.0, 1001167.0 / 1053270.0],
];
const XYZ_TO_LIN_SRGB: Matrix = [
    [12831.0 / 3959.0, -329.0 / 214.0, -1974.0 / 3959.0],
    [
        -851781.0 / 878810.0,
        1648619.0 / 878810.0,
        36519.0 / 878810.0,
    ],
    [705.0 / 12673.0, -2585.0 / 12673.0, 705.0 / 667.0],
];
const LIN_P3_TO_XYZ: Matrix = [
    [
        608311.0 / 1250200.0,
        189793.0 / 714400.0,
        198249.0 / 1000160.0,
    ],
    [
        35783.0 / 156275.0,
        247089.0 / 357200.0,
        198249.0 / 2500400.0,
    ],
    [0.0, 32229.0 / 714400.0, 5220557.0 / 5000800.0],
];
const XYZ_TO_LIN_P3: Matrix = [
    [
        446124.0 / 178915.0,
        -333277.0 / 357830.0,
        -72051.0 / 178915.0,
    ],
    [-14852.0 / 17905.0, 63121.0 / 35810.0, 423.0 / 17905.0],
    [11844.0 / 330415.0, -50337.0 / 660830.0, 316169.0 / 330415.0],
];
const LIN_A98_TO_XYZ: Matrix = [
    [
        573536.0 / 994567.0,
        263643.0 / 1420810.0,
        187206.0 / 994567.0,
    ],
    [
        591459.0 / 1989134.0,
        6239551.0 / 9945670.0,
        374412.0 / 4972835.0,
    ],
    [
        53769.0 / 1989134.0,
        351524.0 / 4972835.0,
        4929758.0 / 4972835.0,
    ],
];
const XYZ_TO_LIN_A98: Matrix = [
    [
        1829569.0 / 896150.0,
        -506331.0 / 896150.0,
        -308931.0 / 896150.0,
    ],
    [
        -851781.0 / 878810.0,
        1648619.0 / 878810.0,
        36519.0 / 878810.0,
    ],
    [
        16779.0 / 1248040.0,
        -147721.0 / 1248040.0,
        1266979.0 / 1248040.0,
    ],
];
const LIN_REC2020_TO_XYZ: Matrix = [
    [
        63426534.0 / 99577255.0,
        20160776.0 / 139408157.0,
        47086771.0 / 278816314.0,
    ],
    [
        26158966.0 / 99577255.0,
        472592308.0 / 697040785.0,
        8267143.0 / 139408157.0,
    ],
    [0.0, 19567812.0 / 697040785.0, 295819943.0 / 278816314.0],
];
const XYZ_TO_LIN_REC2020: Matrix = [
    [
        30757411.0 / 17917100.0,
        -6372589.0 / 17917100.0,
        -4539589.0 / 17917100.0,
    ],
    [
        -19765991.0 / 29648200.0,
        47925759.0 / 29648200.0,
        467509.0 / 29648200.0,
    ],
    [
        792561.0 / 44930125.0,
        -1921689.0 / 44930125.0,
        42328811.0 / 44930125.0,
    ],
];
/// ProPhoto's primaries are relative to D50.
const LIN_PROPHOTO_TO_XYZ50: Matrix = [
    [
        0.797_766_644_900_642_3,
        0.135_181_297_400_533_08,
        0.031_347_734_128_392_2,
    ],
    [
        0.288_074_828_819_401_3,
        0.711_835_234_241_873,
        0.000_089_936_938_725_64,
    ],
    [0.0, 0.0, 0.825_104_602_510_460_2],
];
const XYZ50_TO_LIN_PROPHOTO: Matrix = [
    [
        1.345_786_881_647_158_3,
        -0.255_572_087_379_794_64,
        -0.051_101_864_975_545_26,
    ],
    [
        -0.544_630_705_124_901_9,
        1.508_247_742_845_146_8,
        0.020_527_447_436_421_39,
    ],
    [0.0, 0.0, 1.211_967_545_638_945_2],
];
/// Bradford's chromatic adaptation between the two white points.
const D65_TO_D50: Matrix = [
    [
        1.047_929_792_544_997,
        0.022_946_870_601_609_652,
        -0.050_192_266_289_205_24,
    ],
    [
        0.029_627_808_770_055_99,
        0.990_434_426_753_879_9,
        -0.017_073_799_063_418_826,
    ],
    [
        -0.009_243_040_646_204_504,
        0.015_055_191_490_298_152,
        0.751_874_281_428_137_1,
    ],
];
const D50_TO_D65: Matrix = [
    [
        0.955_473_421_488_075,
        -0.023_098_454_948_764_71,
        0.063_259_243_200_570_72,
    ],
    [
        -0.028_369_709_333_863_7,
        1.009_995_398_081_304_1,
        0.021_041_441_191_917_323,
    ],
    [
        0.012_314_014_864_481_998,
        -0.020_507_649_298_898_964,
        1.330_365_926_242_124,
    ],
];
const XYZ_TO_LMS: Matrix = [
    [
        0.819_022_437_996_703,
        0.361_906_260_052_890_4,
        -0.128_873_781_520_987_9,
    ],
    [
        0.032_983_653_932_388_5,
        0.929_286_861_586_343_4,
        0.036_144_666_350_642_4,
    ],
    [
        0.048_177_189_359_624_2,
        0.264_239_531_752_730_8,
        0.633_547_828_469_430_9,
    ],
];
const LMS_TO_OKLAB: Matrix = [
    [
        0.210_454_268_309_314,
        0.793_617_774_702_305_4,
        -0.004_072_043_011_619_3,
    ],
    [
        1.977_998_532_431_168_4,
        -2.428_592_242_048_58,
        0.450_593_709_617_411,
    ],
    [
        0.025_904_042_465_547_8,
        0.782_771_712_457_529_6,
        -0.808_675_754_923_077_4,
    ],
];
const OKLAB_TO_LMS: Matrix = [
    [1.0, 0.396_337_777_376_174_9, 0.215_803_757_309_913_6],
    [1.0, -0.105_561_345_815_658_6, -0.063_854_172_825_813_3],
    [1.0, -0.089_484_177_529_811_9, -1.291_485_548_019_409_2],
];
const LMS_TO_XYZ: Matrix = [
    [
        1.226_879_875_845_924_3,
        -0.557_814_994_460_217_1,
        0.281_391_045_665_964_7,
    ],
    [
        -0.040_575_745_214_800_8,
        1.112_286_803_280_317,
        -0.071_711_058_065_516_4,
    ],
    [
        -0.076_372_936_674_660_1,
        -0.421_493_332_402_243_2,
        1.586_924_019_836_781_6,
    ],
];

/// The D50 white of CIE Lab.
const D50: [f64; 3] = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];
const EPSILON: f64 = 216.0 / 24389.0;
const KAPPA: f64 = 24389.0 / 27.0;

fn srgb_linear(c: f64) -> f64 {
    let a = c.abs();
    if a <= 0.04045 {
        c / 12.92
    } else {
        c.signum() * ((a + 0.055) / 1.055).powf(2.4)
    }
}

fn srgb_gamma(c: f64) -> f64 {
    let a = c.abs();
    if a > 0.003_130_8 {
        c.signum() * (1.055 * a.powf(1.0 / 2.4) - 0.055)
    } else {
        12.92 * c
    }
}

fn a98_linear(c: f64) -> f64 {
    c.signum() * c.abs().powf(563.0 / 256.0)
}

fn a98_gamma(c: f64) -> f64 {
    c.signum() * c.abs().powf(256.0 / 563.0)
}

fn prophoto_linear(c: f64) -> f64 {
    let a = c.abs();
    if a <= 16.0 / 512.0 {
        c / 16.0
    } else {
        c.signum() * a.powf(1.8)
    }
}

fn prophoto_gamma(c: f64) -> f64 {
    let a = c.abs();
    if a >= 1.0 / 512.0 {
        c.signum() * a.powf(1.0 / 1.8)
    } else {
        16.0 * c
    }
}

const REC_ALPHA: f64 = 1.099_296_826_809_44;
const REC_BETA: f64 = 0.018_053_968_510_807;

fn rec2020_linear(c: f64) -> f64 {
    let a = c.abs();
    if a < REC_BETA * 4.5 {
        c / 4.5
    } else {
        c.signum() * ((a + REC_ALPHA - 1.0) / REC_ALPHA).powf(1.0 / 0.45)
    }
}

fn rec2020_gamma(c: f64) -> f64 {
    let a = c.abs();
    if a > REC_BETA {
        c.signum() * (REC_ALPHA * a.powf(0.45) - (REC_ALPHA - 1.0))
    } else {
        4.5 * c
    }
}

fn lab_to_xyz50([l, a, b]: [f64; 3]) -> [f64; 3] {
    let f1 = (l + 16.0) / 116.0;
    let f0 = a / 500.0 + f1;
    let f2 = f1 - b / 200.0;
    let x = if f0.powi(3) > EPSILON {
        f0.powi(3)
    } else {
        (116.0 * f0 - 16.0) / KAPPA
    };
    let y = if l > KAPPA * EPSILON {
        f1.powi(3)
    } else {
        l / KAPPA
    };
    let z = if f2.powi(3) > EPSILON {
        f2.powi(3)
    } else {
        (116.0 * f2 - 16.0) / KAPPA
    };
    [x * D50[0], y * D50[1], z * D50[2]]
}

fn xyz50_to_lab(xyz: [f64; 3]) -> [f64; 3] {
    let f = [0, 1, 2].map(|i| {
        let v = xyz[i] / D50[i];
        if v > EPSILON {
            v.cbrt()
        } else {
            (KAPPA * v + 16.0) / 116.0
        }
    });
    [
        116.0 * f[1] - 16.0,
        500.0 * (f[0] - f[1]),
        200.0 * (f[1] - f[2]),
    ]
}

fn oklab_to_xyz65(lab: [f64; 3]) -> [f64; 3] {
    let lms = multiply(&OKLAB_TO_LMS, lab).map(|v| v * v * v);
    multiply(&LMS_TO_XYZ, lms)
}

fn xyz65_to_oklab(xyz: [f64; 3]) -> [f64; 3] {
    let lms = multiply(&XYZ_TO_LMS, xyz).map(f64::cbrt);
    multiply(&LMS_TO_OKLAB, lms)
}

/// Lightness, chroma and hue from a rectangular `L a b`.
fn to_polar([l, a, b]: [f64; 3]) -> [f64; 3] {
    let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
    [l, a.hypot(b), hue]
}

fn from_polar([l, c, h]: [f64; 3]) -> [f64; 3] {
    let h = h.to_radians();
    [l, c * h.cos(), c * h.sin()]
}

/// HSL from gamma-encoded sRGB: hue in degrees, the others 0 to 100.
fn srgb_to_hsl(rgb: [f64; 3]) -> [f64; 3] {
    let [mut hue, mut sat, light] = srgb_hue(rgb);
    if sat < 0.0 {
        hue += 180.0;
        sat = sat.abs();
    }
    [hue.rem_euclid(360.0), sat, light]
}

/// HSL from gamma-encoded sRGB, a saturation left negative.
fn srgb_hue([r, g, b]: [f64; 3]) -> [f64; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let light = (min + max) / 2.0;
    let d = max - min;
    let mut hue = 0.0;
    let mut sat = 0.0;
    if d != 0.0 {
        sat = if light == 0.0 || light == 1.0 {
            0.0
        } else {
            (max - light) / light.min(1.0 - light)
        };
        hue = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        hue *= 60.0;
    }
    [hue.rem_euclid(360.0), sat * 100.0, light * 100.0]
}

fn hsl_to_srgb([h, s, l]: [f64; 3]) -> [f64; 3] {
    let (s, l) = (s / 100.0, l / 100.0);
    let hue = h.rem_euclid(360.0) / 30.0;
    let channel = |n: f64| {
        let k = (n + hue) % 12.0;
        let a = s * l.min(1.0 - l);
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [channel(0.0), channel(8.0), channel(4.0)]
}

fn srgb_to_hwb(rgb: [f64; 3]) -> [f64; 3] {
    // The hue unturned by a negative saturation, as the reference
    // browser has it for a colour out of the sRGB gamut.
    let [hue, ..] = srgb_hue(rgb);
    let white = rgb[0].min(rgb[1]).min(rgb[2]);
    let black = 1.0 - rgb[0].max(rgb[1]).max(rgb[2]);
    [hue, white * 100.0, black * 100.0]
}

fn hwb_to_srgb([h, w, b]: [f64; 3]) -> [f64; 3] {
    let (w, b) = (w / 100.0, b / 100.0);
    if w + b >= 1.0 {
        let gray = w / (w + b);
        return [gray; 3];
    }
    hsl_to_srgb([h, 100.0, 50.0]).map(|c| c * (1.0 - w - b) + w)
}

/// CIE XYZ relative to D65 from `c` of `space`.
fn to_xyz65(space: Space, c: [f64; 3]) -> [f64; 3] {
    match space {
        Space::Legacy | Space::Srgb => multiply(&LIN_SRGB_TO_XYZ, c.map(srgb_linear)),
        Space::SrgbLinear => multiply(&LIN_SRGB_TO_XYZ, c),
        Space::DisplayP3 => multiply(&LIN_P3_TO_XYZ, c.map(srgb_linear)),
        Space::DisplayP3Linear => multiply(&LIN_P3_TO_XYZ, c),
        Space::A98Rgb => multiply(&LIN_A98_TO_XYZ, c.map(a98_linear)),
        Space::ProphotoRgb => multiply(
            &D50_TO_D65,
            multiply(&LIN_PROPHOTO_TO_XYZ50, c.map(prophoto_linear)),
        ),
        Space::Rec2020 => multiply(&LIN_REC2020_TO_XYZ, c.map(rec2020_linear)),
        Space::XyzD65 => c,
        Space::XyzD50 => multiply(&D50_TO_D65, c),
        Space::Lab => multiply(&D50_TO_D65, lab_to_xyz50(c)),
        Space::Lch => multiply(&D50_TO_D65, lab_to_xyz50(from_polar(c))),
        Space::Oklab => oklab_to_xyz65(c),
        Space::Oklch => oklab_to_xyz65(from_polar(c)),
        Space::Hsl => to_xyz65(Space::Srgb, hsl_to_srgb(c)),
        Space::Hwb => to_xyz65(Space::Srgb, hwb_to_srgb(c)),
    }
}

/// The components in `space` of CIE XYZ relative to D65.
fn from_xyz65(space: Space, xyz: [f64; 3]) -> [f64; 3] {
    match space {
        Space::Legacy | Space::Srgb => multiply(&XYZ_TO_LIN_SRGB, xyz).map(srgb_gamma),
        Space::SrgbLinear => multiply(&XYZ_TO_LIN_SRGB, xyz),
        Space::DisplayP3 => multiply(&XYZ_TO_LIN_P3, xyz).map(srgb_gamma),
        Space::DisplayP3Linear => multiply(&XYZ_TO_LIN_P3, xyz),
        Space::A98Rgb => multiply(&XYZ_TO_LIN_A98, xyz).map(a98_gamma),
        Space::ProphotoRgb => {
            multiply(&XYZ50_TO_LIN_PROPHOTO, multiply(&D65_TO_D50, xyz)).map(prophoto_gamma)
        }
        Space::Rec2020 => multiply(&XYZ_TO_LIN_REC2020, xyz).map(rec2020_gamma),
        Space::XyzD65 => xyz,
        Space::XyzD50 => multiply(&D65_TO_D50, xyz),
        Space::Lab => xyz50_to_lab(multiply(&D65_TO_D50, xyz)),
        Space::Lch => to_polar(xyz50_to_lab(multiply(&D65_TO_D50, xyz))),
        Space::Oklab => xyz65_to_oklab(xyz),
        Space::Oklch => to_polar(xyz65_to_oklab(xyz)),
        Space::Hsl => srgb_to_hsl(from_xyz65(Space::Srgb, xyz)),
        Space::Hwb => srgb_to_hwb(from_xyz65(Space::Srgb, xyz)),
    }
}

/// `c`'s components in `space` and which are `none`, as the reference
/// browser has them: a colour of that space as it is; another converted,
/// its `none` carried over by section 12.2's analogous components, and a
/// hue made `none` where it is powerless there (section 4.4).
fn converted(color: &Color, space: Space) -> ([f64; 3], u8) {
    let own = color.c.map(f64::from);
    let given = [0, 1, 2].map(|i| {
        if color.missing & (1 << i) != 0 {
            0.0
        } else {
            own[i]
        }
    });
    if color.space == space || (color.space == Space::Legacy && space == Space::Srgb) {
        return (given, color.missing);
    }
    let rgb = match color.space {
        Space::Legacy | Space::Srgb => Some(given),
        Space::Hsl => Some(hsl_to_srgb(given)),
        Space::Hwb => Some(hwb_to_srgb(given)),
        _ => None,
    };
    let values = match (rgb, space) {
        (Some(rgb), Space::Hsl) => srgb_to_hsl(rgb),
        (Some(rgb), Space::Hwb) => srgb_to_hwb(rgb),
        (Some(rgb), _) => from_xyz65(space, to_xyz65(Space::Srgb, rgb)),
        (None, _) => from_xyz65(space, to_xyz65(color.space, given)),
    };
    let mut missing = color.missing & ALPHA;
    let none = color.missing & 7;
    if none == 7 {
        // All three `none` stay so in any space.
        missing |= 7;
    } else {
        // The categories `none`, a pair of the second and third taken as
        // the polar form's: Lab's a and b as LCH's chroma and hue, HWB's
        // whiteness and blackness as HSL's saturation and lightness.
        let from = match color.space {
            Space::Lab | Space::Oklab if none == 6 => Space::Lch.categories(),
            Space::Hwb if none == 6 => Space::Hsl.categories(),
            other => other.categories(),
        };
        let carried: Vec<Category> = (0..3)
            .filter(|i| none & (1 << i) != 0 && from[*i] != Category::Other)
            .map(|i| from[i])
            .collect();
        let to = space.categories();
        for (j, category) in to.iter().enumerate() {
            if carried.contains(category) {
                missing |= 1 << j;
            }
        }
        let pair = match space {
            Space::Lab | Space::Oklab => [Category::Colorfulness, Category::Hue],
            Space::Hwb => [Category::Colorfulness, Category::Lightness],
            _ => [Category::Other; 2],
        };
        if pair
            .iter()
            .all(|c| *c != Category::Other && carried.contains(c))
        {
            missing |= 6;
        }
    }
    // The reference browser's powerless hues: a chroma of 0.02 or less,
    // no saturation, or whiteness and blackness filling the colour.
    let powerless = match space {
        Space::Lch | Space::Oklch => values[1].abs() <= 0.02,
        Space::Hsl => values[1] == 0.0,
        Space::Hwb => values[1] + values[2] >= 100.0,
        _ => false,
    };
    if powerless {
        missing |= if space == Space::Lch || space == Space::Oklch {
            1 << 2
        } else {
            1
        };
    }
    (values, missing)
}

/// `a` and `b` interpolated by `method`, `p` of the way from `a` to `b`
/// (section 12): in the method's space, with premultiplied alpha, `none`
/// taking the other colour's component, hues by the method's arc. The
/// result's alpha is multiplied by `alpha_scale` (`color-mix()`'s
/// percentages summing under 100).
pub fn interpolate(a: &Color, b: &Color, p: f64, method: Method, alpha_scale: f64) -> Color {
    let space = method.space;
    let (mut x, xm) = converted(a, space);
    let (mut y, ym) = converted(b, space);
    let mut missing = 0u8;
    for i in 0..3 {
        let bit = 1 << i;
        match (xm & bit != 0, ym & bit != 0) {
            (true, true) => missing |= bit,
            (true, false) => x[i] = y[i],
            (false, true) => y[i] = x[i],
            (false, false) => {}
        }
    }
    let alpha_of = |c: &Color| f64::from(c.a);
    let (mut xa, mut ya) = (alpha_of(a), alpha_of(b));
    match (xm & ALPHA != 0, ym & ALPHA != 0) {
        (true, true) => {
            missing |= ALPHA;
            xa = 1.0;
            ya = 1.0;
        }
        (true, false) => xa = ya,
        (false, true) => ya = xa,
        (false, false) => {}
    }
    let hue = match space {
        Space::Lch | Space::Oklch => Some(2),
        Space::Hsl | Space::Hwb => Some(0),
        _ => None,
    };
    if let Some(h) = hue {
        let (from, to) = (x[h].rem_euclid(360.0), y[h].rem_euclid(360.0));
        let (from, to) = arc(from, to, method.hue);
        x[h] = from;
        y[h] = to;
    }
    // Past the ends the alpha is clamped before the colour is
    // unpremultiplied by it, as the web-platform tests ask.
    let alpha = (xa + (ya - xa) * p).clamp(0.0, 1.0);
    let mut out = [0.0; 3];
    for i in 0..3 {
        if Some(i) == hue {
            out[i] = (x[i] + (y[i] - x[i]) * p).rem_euclid(360.0);
            continue;
        }
        let premultiplied = x[i] * xa + (y[i] * ya - x[i] * xa) * p;
        out[i] = if alpha == 0.0 {
            premultiplied
        } else {
            premultiplied / alpha
        };
    }
    let alpha = if missing & ALPHA != 0 {
        0.0
    } else {
        (alpha * alpha_scale).clamp(0.0, 1.0)
    };
    // A lightness kept within its space, a chroma never negative, as an
    // extrapolation past the ends would leave them.
    match space {
        Space::Lab | Space::Lch => out[0] = out[0].clamp(0.0, 100.0),
        Space::Oklab | Space::Oklch => out[0] = out[0].clamp(0.0, 1.0),
        _ => {}
    }
    if matches!(space, Space::Lch | Space::Oklch) {
        out[1] = out[1].max(0.0);
    }
    match space {
        // A colour mixed in HSL or HWB is an sRGB one (the reference
        // browser's `color(srgb ...)`).
        Space::Hsl | Space::Hwb => {
            let rgb = if space == Space::Hsl {
                hsl_to_srgb(out)
            } else {
                hwb_to_srgb(out)
            };
            Color::of(Space::Srgb, rgb, alpha, missing & ALPHA)
        }
        _ => Color::of(space, out, alpha, missing),
    }
}

/// The two hues adjusted for `method`'s arc (section 12.4).
fn arc(from: f64, to: f64, method: Hue) -> (f64, f64) {
    let d = to - from;
    match method {
        Hue::Shorter if d > 180.0 => (from + 360.0, to),
        Hue::Shorter if d < -180.0 => (from, to + 360.0),
        // Equal hues go the whole way round (section 12.4.2).
        Hue::Longer if d > 0.0 && d < 180.0 => (from + 360.0, to),
        Hue::Longer if d > -180.0 && d <= 0.0 => (from, to + 360.0),
        Hue::Increasing if to < from => (from, to + 360.0),
        Hue::Decreasing if from < to => (from + 360.0, to),
        _ => (from, to),
    }
}

/// The colour `p` of the way from `a` to `b` in an animation or a
/// transition: in sRGB when both are legacy, the result legacy, else in
/// Oklab (section 12.1).
pub fn animate(a: &Color, b: &Color, p: f64) -> Color {
    if a.is_legacy() && b.is_legacy() {
        let c = interpolate(
            a,
            b,
            p,
            Method {
                space: Space::Srgb,
                hue: Hue::Shorter,
            },
            1.0,
        );
        let clamp = |v: f32| v.clamp(0.0, 1.0);
        if c.a == 0.0 {
            return Color::TRANSPARENT;
        }
        return Color::rgba(clamp(c.r), clamp(c.g), clamp(c.b), c.a);
    }
    interpolate(a, b, p, Method::OKLAB, 1.0)
}

/// A colour's component written as a number of six significant figures,
/// as the reference browser writes one: `0.504777`, `301.606`.
pub fn number(value: f32) -> String {
    let v = f64::from(value);
    if v == 0.0 || !v.is_finite() {
        return "0".to_owned();
    }
    let magnitude = v.abs().log10().floor() as i32;
    let decimals = (5 - magnitude).clamp(0, 12) as usize;
    let text = format!("{v:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    if text == "-0" { "0".to_owned() } else { text }
}

/// A colour that is not legacy, serialized in its own form (section
/// 15.2): `lab(50 20 30)`, `color(display-p3 1 0 0 / 0.5)`.
pub fn serialize(c: &Color) -> String {
    let parts: Vec<String> = c
        .components()
        .iter()
        .map(|v| v.map_or_else(|| "none".to_owned(), number))
        .collect();
    let mut text = match c.space {
        Space::Lab => format!("lab({}", parts.join(" ")),
        Space::Lch => format!("lch({}", parts.join(" ")),
        Space::Oklab => format!("oklab({}", parts.join(" ")),
        Space::Oklch => format!("oklch({}", parts.join(" ")),
        other => format!(
            "color({} {}",
            other.predefined_name().unwrap_or("srgb"),
            parts.join(" ")
        ),
    };
    match c.alpha() {
        None => text.push_str(" / none"),
        Some(a) if a < 1.0 => {
            text.push_str(" / ");
            text.push_str(&number(a));
        }
        Some(_) => {}
    }
    text.push(')');
    text
}

/// The component values of a function's arguments, white space aside: a
/// token, or a nested function with its arguments, or a `/`.
fn components(arguments: &[Token]) -> Option<Vec<&[Token]>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < arguments.len() {
        match &arguments[i] {
            Token::Whitespace => i += 1,
            Token::Function(_) | Token::OpenParen => {
                let start = i;
                let mut depth = 0usize;
                loop {
                    match arguments.get(i)? {
                        Token::Function(_) | Token::OpenParen => depth += 1,
                        Token::CloseParen => depth -= 1,
                        _ => {}
                    }
                    i += 1;
                    if depth == 0 {
                        break;
                    }
                }
                out.push(&arguments[start..i]);
            }
            _ => {
                out.push(&arguments[i..i + 1]);
                i += 1;
            }
        }
    }
    Some(out)
}

/// No element's lengths: a colour's component that needs them (a
/// relative length in a calculation) is not one here.
struct NoLengths(std::cell::Cell<bool>);

impl calc::Lengths for NoLengths {
    fn px(&self, _: f64, _: LengthUnit) -> f64 {
        self.0.set(true);
        0.0
    }
}

/// What a component is: a number, a percentage, an angle in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Component {
    Number(f64),
    Percent(f64),
    Degrees(f64),
    None,
}

/// `arguments` of a legacy colour function, `rgb()`, `hsl()` or `hwb()`,
/// with each math function in them computed to the number, percentage
/// or angle it is (gap batch E): `None` where one is invalid in a colour,
/// and `arguments` as they are where there is none. An infinite hue, the
/// first component where `hue_first`, is 0, as the reference browser has
/// it; another infinity is clamped to the component's range.
pub(crate) fn computed_math(
    arguments: &[Token],
    hue_first: bool,
) -> Option<std::borrow::Cow<'_, [Token]>> {
    let math = |t: &Token| calc::is_math_function(std::slice::from_ref(t));
    if !arguments.iter().any(math) {
        return Some(std::borrow::Cow::Borrowed(arguments));
    }
    let mut out = Vec::with_capacity(arguments.len());
    let mut i = 0;
    while i < arguments.len() {
        if !math(&arguments[i]) {
            out.push(arguments[i].clone());
            i += 1;
            continue;
        }
        let mut depth = 0usize;
        let mut end = i;
        for (j, token) in arguments.iter().enumerate().skip(i) {
            match token {
                Token::Function(_) | Token::OpenParen => depth += 1,
                Token::CloseParen => depth -= 1,
                _ => {}
            }
            end = j;
            if depth == 0 {
                break;
            }
        }
        let first = out
            .iter()
            .all(|t| matches!(t, Token::Whitespace | Token::Comma));
        let finite = |v: f64| {
            if v.is_finite() {
                v
            } else if hue_first && first {
                0.0
            } else {
                v.clamp(-MAX_COMPONENT, MAX_COMPONENT)
            }
        };
        out.push(match component(&arguments[i..=end])? {
            Component::Number(value) => Token::Number {
                value: finite(value),
                is_integer: false,
            },
            Component::Percent(value) => Token::Percentage {
                value: finite(value),
            },
            Component::Degrees(value) => Token::Dimension {
                value: if value.is_finite() { value } else { 0.0 },
                unit: "deg".into(),
            },
            Component::None => return None,
        });
        i = end + 1;
    }
    Some(std::borrow::Cow::Owned(out))
}

fn component(tokens: &[Token]) -> Option<Component> {
    if let [Token::Function(_), ..] = tokens {
        for want in [Want::Number, Want::Percentage, Want::Angle] {
            if let Some(parsed) = calc::parse(tokens, want) {
                let lengths = NoLengths(std::cell::Cell::new(false));
                let computed = parsed.compute(&lengths, None);
                if lengths.0.get() {
                    return None;
                }
                let value = match computed {
                    calc::Computed::Value(v) => v,
                    calc::Computed::Percent(p) => return Some(Component::Percent(p)),
                    calc::Computed::Expression(_) => return None,
                };
                // NaN is 0; an infinity is left for the component's
                // place, a hue's 0 and another's clamped.
                let value = if value.is_nan() { 0.0 } else { value };
                return Some(match want {
                    Want::Percentage => Component::Percent(value),
                    Want::Angle => Component::Degrees(value),
                    _ => Component::Number(value),
                });
            }
        }
        return None;
    }
    let [token] = tokens else {
        return None;
    };
    Some(match token {
        Token::Number { value, .. } if value.is_finite() => Component::Number(*value),
        Token::Percentage { value } if value.is_finite() => Component::Percent(*value),
        Token::Dimension { value, unit } if value.is_finite() => {
            Component::Degrees(match unit.to_ascii_lowercase().as_str() {
                "deg" => *value,
                "grad" => value * 0.9,
                "rad" => value.to_degrees(),
                "turn" => value * 360.0,
                _ => return None,
            })
        }
        Token::Ident(word) if word.eq_ignore_ascii_case("none") => Component::None,
        _ => return None,
    })
}

/// The three components and the optional alpha after a `/`.
fn channels(arguments: &[Token]) -> Option<([Component; 3], Component)> {
    let parts = components(arguments)?;
    let (main, alpha) = match parts.as_slice() {
        [a, b, c] => ([*a, *b, *c], None),
        [a, b, c, [Token::Delim('/')], alpha] => ([*a, *b, *c], Some(*alpha)),
        _ => return None,
    };
    let main = [
        component(main[0])?,
        component(main[1])?,
        component(main[2])?,
    ];
    let alpha = match alpha {
        None => Component::Number(1.0),
        Some(tokens) => match component(tokens)? {
            Component::Degrees(_) => return None,
            other => other,
        },
    };
    Some((main, alpha))
}

/// A component's value where `hundred` is what 100% is, `None` for
/// `none`; an angle is not one.
fn scaled(c: Component, hundred: f64) -> Option<Option<f64>> {
    match c {
        Component::Number(v) => Some(Some(v)),
        Component::Percent(p) => Some(Some(p / 100.0 * hundred)),
        Component::None => Some(None),
        Component::Degrees(_) => None,
    }
}

fn hue(c: Component) -> Option<Option<f64>> {
    match c {
        Component::Number(v) | Component::Degrees(v) if v.is_finite() => {
            Some(Some(v.rem_euclid(360.0)))
        }
        Component::Number(_) | Component::Degrees(_) => Some(Some(0.0)),
        Component::None => Some(None),
        Component::Percent(_) => None,
    }
}

/// `lab()`, `lch()`, `oklab()`, `oklch()` or `color()`'s arguments.
pub fn parse_function(name: &str, arguments: &[Token]) -> Option<Color> {
    let lower = name.to_ascii_lowercase();
    let (space, arguments) = match lower.as_str() {
        "lab" => (Space::Lab, arguments),
        "lch" => (Space::Lch, arguments),
        "oklab" => (Space::Oklab, arguments),
        "oklch" => (Space::Oklch, arguments),
        "color" => {
            let start = arguments
                .iter()
                .position(|t| !matches!(t, Token::Whitespace))?;
            let Token::Ident(space) = &arguments[start] else {
                return None;
            };
            (Space::predefined(space)?, &arguments[start + 1..])
        }
        _ => return None,
    };
    if arguments.iter().any(|t| matches!(t, Token::Comma)) {
        return None;
    }
    let (main, alpha) = channels(arguments)?;
    let values: [Option<f64>; 3] = match space {
        Space::Lab => [
            scaled(main[0], 100.0)?.map(|l| l.clamp(0.0, 100.0)),
            scaled(main[1], 125.0)?,
            scaled(main[2], 125.0)?,
        ],
        Space::Lch => [
            scaled(main[0], 100.0)?.map(|l| l.clamp(0.0, 100.0)),
            scaled(main[1], 150.0)?.map(|c| c.max(0.0)),
            hue(main[2])?,
        ],
        Space::Oklab => [
            scaled(main[0], 1.0)?.map(|l| l.clamp(0.0, 1.0)),
            scaled(main[1], 0.4)?,
            scaled(main[2], 0.4)?,
        ],
        Space::Oklch => [
            scaled(main[0], 1.0)?.map(|l| l.clamp(0.0, 1.0)),
            scaled(main[1], 0.4)?.map(|c| c.max(0.0)),
            hue(main[2])?,
        ],
        _ => [
            scaled(main[0], 1.0)?,
            scaled(main[1], 1.0)?,
            scaled(main[2], 1.0)?,
        ],
    };
    let alpha = scaled(alpha, 1.0)?.map(|a| a.clamp(0.0, 1.0));
    let mut missing = 0u8;
    for (i, v) in values.iter().enumerate() {
        if v.is_none() {
            missing |= 1 << i;
        }
    }
    if alpha.is_none() {
        missing |= ALPHA;
    }
    Some(Color::of(
        space,
        values.map(|v| v.unwrap_or(0.0)),
        alpha.unwrap_or(0.0),
        missing,
    ))
}

/// A `<color-interpolation-method>`'s words after `in`: a space, and a
/// hue method and `hue` after a polar one.
pub fn parse_method(words: &[&Token]) -> Option<Method> {
    let Some((Token::Ident(first), rest)) = words.split_first().map(|(f, r)| (*f, r)) else {
        return None;
    };
    if !first.eq_ignore_ascii_case("in") {
        return None;
    }
    let Some((Token::Ident(space), rest)) = rest.split_first().map(|(f, r)| (*f, r)) else {
        return None;
    };
    let space = Space::interpolation(space)?;
    let hue = match rest {
        [] => Hue::Shorter,
        [Token::Ident(how), Token::Ident(word)]
            if space.is_polar() && word.eq_ignore_ascii_case("hue") =>
        {
            match how.to_ascii_lowercase().as_str() {
                "shorter" => Hue::Shorter,
                "longer" => Hue::Longer,
                "increasing" => Hue::Increasing,
                "decreasing" => Hue::Decreasing,
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(Method { space, hue })
}

/// `color-mix()`'s arguments (css-color-5 section 2), each colour read
/// by `color`: an optional method, then two colours each with an
/// optional percentage.
pub fn parse_mix(
    arguments: &[Token],
    color: &mut dyn FnMut(&[Token]) -> Option<Color>,
) -> Option<Color> {
    let mut depth = 0usize;
    for token in arguments {
        match token {
            Token::Function(_) | Token::OpenParen => {
                depth += 1;
                if depth >= MAX_DEPTH {
                    return None;
                }
            }
            Token::CloseParen => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    let groups: Vec<&[Token]> = split_commas(arguments);
    let (method, colors) = match groups.as_slice() {
        [first, a, b] => {
            let words: Vec<&Token> = first
                .iter()
                .filter(|t| !matches!(t, Token::Whitespace))
                .collect();
            (parse_method(&words)?, [*a, *b])
        }
        [a, b] => (Method::OKLAB, [*a, *b]),
        _ => return None,
    };
    let mut mixed = Vec::with_capacity(2);
    for group in colors {
        let parts = components(group)?;
        // A percentage, or a calculation of one.
        let percentage = |part: &[Token]| match part {
            [Token::Percentage { value }] => Some(*value),
            _ if calc::is_math_function(part) => {
                match calc::parse(part, Want::Percentage)?
                    .compute(&NoLengths(Default::default()), None)
                {
                    calc::Computed::Percent(p) if !p.is_nan() => Some(p),
                    _ => None,
                }
            }
            _ => None,
        };
        let (value, percent) = match parts.as_slice() {
            [one] => (*one, None),
            [a, b] => match (percentage(a), percentage(b)) {
                (Some(p), None) => (*b, Some(p)),
                (None, Some(p)) => (*a, Some(p)),
                _ => return None,
            },
            _ => return None,
        };
        if percent.is_some_and(|p| !(0.0..=100.0).contains(&p)) {
            return None;
        }
        mixed.push((color(value)?, percent));
    }
    let (p1, p2) = match (mixed[0].1, mixed[1].1) {
        (None, None) => (50.0, 50.0),
        (Some(p), None) => (p, 100.0 - p),
        (None, Some(p)) => (100.0 - p, p),
        (Some(a), Some(b)) => (a, b),
    };
    // Percentages summing to 0 mix evenly to a transparent colour, as
    // the reference browser mixes them.
    let sum = p1 + p2;
    let (p, scale) = if sum <= 0.0 {
        (0.5, 0.0)
    } else {
        (p2 / sum, if sum < 100.0 { sum / 100.0 } else { 1.0 })
    };
    Some(interpolate(&mixed[0].0, &mixed[1].0, p, method, scale))
}

/// `arguments` split at their top-level commas.
fn split_commas(arguments: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, token) in arguments.iter().enumerate() {
        match token {
            Token::Function(_) | Token::OpenParen => depth += 1,
            Token::CloseParen => depth = depth.saturating_sub(1),
            Token::Comma if depth == 0 => {
                out.push(&arguments[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&arguments[start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Option<Color> {
        crate::cascade::parse_color_text(text)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.002
    }

    #[test]
    fn the_spaces_reach_srgb_as_the_reference_browser_mixes_them() {
        // Each against `color-mix(in srgb, X 100%, red 0%)` in the
        // reference browser.
        for (text, rgb) in [
            ("lab(50 20 30)", [0.630054, 0.412583, 0.268905]),
            ("oklch(0.5 0.1 120)", [0.361156, 0.420377, 0.128587]),
            ("color(display-p3 1 0 0)", [1.09302, -0.22669, -0.150073]),
            ("lch(50 30 120)", [0.411328, 0.492908, 0.286726]),
            ("color(xyz 0.2 0.3 0.4)", [-0.115147, 0.654269, 0.644341]),
            ("color(a98-rgb 1 0 0)", [1.15815, 0.000270443, 0.000181694]),
            (
                "color(prophoto-rgb 0.5 0.5 0.2)",
                [0.632855, 0.572954, 0.141254],
            ),
            (
                "color(rec2020 0.3 0.6 0.1)",
                [-0.226126, 0.666179, -0.120794],
            ),
            (
                "color(srgb-linear 0.5 0.5 0.5)",
                [0.735394, 0.735394, 0.735394],
            ),
            (
                "color(xyz-d50 0.2 0.3 0.4)",
                [-0.259227, 0.659534, 0.740507],
            ),
            ("oklab(0.7 -0.1 0.1)", [0.464767, 0.696124, 0.324405]),
            ("lab(10 -50 60)", [-0.11466, 0.162067, -0.204542]),
        ] {
            let c = parse(text).unwrap();
            assert!(
                close(c.r, rgb[0]) && close(c.g, rgb[1]) && close(c.b, rgb[2]),
                "{text}: {c:?}"
            );
        }
    }

    #[test]
    fn colours_serialize_in_their_own_form() {
        let s = |text: &str| serialize(&parse(text).unwrap());
        assert_eq!(s("lab(50 20 30)"), "lab(50 20 30)");
        assert_eq!(s("lab(50% 20% -30% / 0.5)"), "lab(50 25 -37.5 / 0.5)");
        assert_eq!(s("oklch(60% 0.2 400deg / 50%)"), "oklch(0.6 0.2 40 / 0.5)");
        assert_eq!(s("oklch(none 0.1 120)"), "oklch(none 0.1 120)");
        assert_eq!(s("color(xyz 0.2 0.3 0.4)"), "color(xyz-d65 0.2 0.3 0.4)");
        assert_eq!(s("color(display-p3 1.5 0 0)"), "color(display-p3 1.5 0 0)");
        assert_eq!(s("lab(150 0 0)"), "lab(100 0 0)");
        assert_eq!(s("oklab(1.5 0 0)"), "oklab(1 0 0)");
        assert_eq!(s("lch(50 -10 0)"), "lch(50 0 0)");
        assert_eq!(s("lab(calc(25 * 2) 20 30)"), "lab(50 20 30)");
        // A length relative to an element is not known to a colour.
        assert!(parse("color(srgb calc(1em / 1px) 0 0)").is_none());
        assert!(parse("lab(50, 20, 30)").is_none());
        assert!(parse("lab(50 20)").is_none());
        assert!(parse("lch(50 30 120%)").is_none());
        assert!(parse("color(nope 1 0 0)").is_none());
    }

    /// A serialization compared with the reference browser's to within
    /// the last figures, which its own matrices give differently.
    #[derive(Debug)]
    struct Mixed(String);

    impl PartialEq<&str> for Mixed {
        fn eq(&self, other: &&str) -> bool {
            let words = |t: &str| -> Vec<String> {
                t.split([' ', '(', ')'])
                    .filter(|w| !w.is_empty())
                    .map(str::to_owned)
                    .collect()
            };
            let (a, b) = (words(&self.0), words(other));
            a.len() == b.len()
                && a.iter()
                    .zip(&b)
                    .all(|(x, y)| match (x.parse::<f64>(), y.parse::<f64>()) {
                        (Ok(x), Ok(y)) => (x - y).abs() <= 1e-4_f64.max(y.abs() * 5e-4),
                        _ => x == y,
                    })
        }
    }

    #[test]
    fn color_mix_computes_in_its_space() {
        let s = |text: &str| Mixed(serialize(&parse(text).unwrap()));
        assert_eq!(s("color-mix(in srgb, red, blue)"), "color(srgb 0.5 0 0.5)");
        assert_eq!(
            s("color-mix(in srgb, red 20%, transparent 20%)"),
            "color(srgb 1 0 0 / 0.2)"
        );
        assert_eq!(
            s("color-mix(in hsl longer hue, red, blue)"),
            "color(srgb 0 1 0)"
        );
        assert_eq!(
            s("color-mix(in oklch, red 30%, blue)"),
            "oklch(0.504777 0.296558 301.606)"
        );
        assert_eq!(
            s("color-mix(in lab, red, blue)"),
            "lab(41.9277 74.5607 -21.0676)"
        );
        assert_eq!(
            s("color-mix(red, blue)"),
            "oklab(0.539974 0.0962086 -0.0928316)"
        );
        assert_eq!(
            s("color-mix(in oklch, white, blue)"),
            "oklch(0.725987 0.15663 264.051)"
        );
        assert_eq!(
            s("color-mix(in hsl, white, blue)"),
            "color(srgb 0.625 0.625 0.875)"
        );
        assert_eq!(
            s("color-mix(in lch, oklch(none 0.2 30), lab(50 20 30))"),
            "lch(50 21.6692 72.1425)"
        );
        assert_eq!(
            s("color-mix(in srgb, rgb(255 0 0 / 0.5), blue)"),
            "color(srgb 0.333333 0 0.666667 / 0.75)"
        );
        assert_eq!(
            s("color-mix(in oklch decreasing hue, red, blue)"),
            "oklch(0.539974 0.285457 326.643)"
        );
        assert_eq!(s("color-mix(in hwb, red 25%, blue)"), "color(srgb 0.5 0 1)");
        // White's hue is powerless: red's twice, the longer way round.
        assert_eq!(
            s("color-mix(in hsl longer hue, red, white)"),
            "color(srgb 0.625 0.875 0.875)"
        );
        // The longer arc of hues 60 degrees apart: 210 degrees.
        assert_eq!(
            s("color-mix(in hsl longer hue, red, yellow)"),
            "color(srgb 0 0.5 1)"
        );
        assert_eq!(
            s("color-mix(in xyz, red, blue)"),
            "color(xyz-d65 0.296439 0.142411 0.484861)"
        );
        assert_eq!(
            s("color-mix(in lch, red, blue 60%)"),
            "lch(39.4552 121.47 341.163)"
        );
        assert_eq!(
            s("color-mix(in srgb, color(srgb none 0.5 0.5), red)"),
            "color(srgb 1 0.25 0.25)"
        );
        assert_eq!(
            s("color(srgb 0.5 0.5 0.5 / none)"),
            "color(srgb 0.5 0.5 0.5 / none)"
        );
        assert_eq!(
            s("color-mix(in srgb, red 0%, blue 0%)"),
            "color(srgb 0.5 0 0.5 / 0)"
        );
        assert!(parse("color-mix(in srgb, red 120%, blue)").is_none());
        assert_eq!(
            s("color-mix(in srgb, red calc(10%), blue calc(80%))"),
            "color(srgb 0.111111 0 0.888889 / 0.9)"
        );
        assert!(parse("color-mix(in srgb, red calc(1px), blue)").is_none());
        assert!(parse("color-mix(in srgb longer hue, red, blue)").is_none());
        assert!(parse("color-mix(in srgb, red, blue, green)").is_none());
    }

    #[test]
    fn mixes_nest_to_a_bound() {
        let nested = |n: usize| {
            let mut text = "red".to_owned();
            for _ in 0..n {
                text = format!("color-mix(in srgb, {text}, blue)");
            }
            text
        };
        assert!(parse(&nested(MAX_DEPTH - 1)).is_some());
        assert!(parse(&nested(MAX_DEPTH + 1)).is_none());
        assert!(parse(&nested(10_000)).is_none());
        // A component past what the conversions hold is kept finite.
        let c = parse("color(srgb-linear 1e300 0 0)").unwrap();
        assert!(c.r.is_finite() && c.components()[0] == Some(MAX_COMPONENT as f32));
    }

    #[test]
    fn mixes_take_their_colours_as_the_reference_browser_takes_them() {
        // Gap batch E: the reference browser's results, to 0.002 as the
        // web-platform tests compare them.
        let near = |text: &str, want: &str| {
            let got = serialize(&parse(text).unwrap());
            let numbers = |s: &str| -> Vec<f64> {
                s.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                    .filter_map(|w| w.parse().ok())
                    .collect()
            };
            let (g, w) = (numbers(&got), numbers(want));
            assert!(
                got.split('(').next() == want.split('(').next()
                    && g.len() == w.len()
                    && g.iter()
                        .zip(&w)
                        .all(|(a, b)| (a - b).abs() <= 0.002 * b.abs().max(1.0)),
                "{text}: {got}, not {want}"
            );
        };
        // A colour of the mix's own space is taken as it is: a hue
        // with no saturation is no `none`.
        near(
            "color-mix(in hsl, hsl(0 0 50%), hsl(11 33 44))",
            "color(srgb 0.54755 0.406668 0.39245)",
        );
        near(
            "color-mix(in lch, lch(100 0 40deg), lch(100 0 60deg))",
            "lch(100 0 50)",
        );
        // A legacy colour keeps its nones.
        near(
            "color-mix(in hsl, hsl(none none none), hsl(30deg 40% 80%))",
            "color(srgb 0.88 0.8 0.72)",
        );
        near(
            "color-mix(in hsl, hsl(120deg 40% 40% / none), hsl(0deg 40% 40%))",
            "color(srgb 0.56 0.56 0.24)",
        );
        // All three none stay so; a pair of the last two is the polar
        // form's.
        near(
            "color-mix(in lab, rgb(none none none), lab(11 33 44))",
            "lab(11 33 44)",
        );
        near(
            "color-mix(in lch, lab(50 none none), lch(11 33 44))",
            "lch(30.5 33 44)",
        );
        near(
            "color-mix(in oklab, oklch(0.1 none none), oklab(0.3 0.2 0.4))",
            "oklab(0.2 0.2 0.4)",
        );
        // A hue is powerless at a chroma of 0.02 or less.
        near(
            "color-mix(in lch, lab(50 0.02 0), lch(11 33 44))",
            "lch(30.5 16.51 44)",
        );
        near(
            "color-mix(in lch, lab(50 0.0201 0), lch(11 33 44))",
            "lch(30.5 16.51 22)",
        );
        // HWB from a colour past sRGB keeps its hue unturned.
        near(
            "color-mix(in hwb, lab(none none 10), hwb(11 33 44))",
            "color(srgb 0.296792 0.199423 0.124028)",
        );
        near(
            "color(display-p3-linear 1 0.5 0.2)",
            "color(display-p3-linear 1 0.5 0.2)",
        );
        // Calculations in the legacy forms; an infinite hue is 0.
        let c = parse("hsl(calc(infinity) 50% 50%)").unwrap();
        assert!(
            (c.r - 0.75).abs() < 1e-3 && (c.g - 0.25).abs() < 1e-3,
            "{c:?}"
        );
        let c = parse("rgb(calc(infinity), calc(-infinity), calc(0 / 0))").unwrap();
        assert_eq!((c.r, c.g, c.b), (1.0, 0.0, 0.0));
        assert_eq!(parse("rgba(0, 0, 0, calc(0 / 0))").unwrap().a, 0.0);
        assert!(parse("rgb(calc(1px), 0, 0)").is_none());
    }

    #[test]
    fn a_powerless_hue_takes_the_other_colours() {
        // White has no hue in OKLCH: the mix keeps blue's.
        let c = parse("color-mix(in oklch, white, blue)").unwrap();
        let blue = converted(&parse("blue").unwrap(), Space::Oklch).0;
        assert!((f64::from(c.components()[2].unwrap()) - blue[2]).abs() < 0.01);
    }

    #[test]
    fn an_animation_past_its_ends_stays_in_the_space() {
        let (black, white) = (
            parse("rgb(0 0 0)").unwrap(),
            parse("color(srgb 1 1 1)").unwrap(),
        );
        assert_eq!(serialize(&animate(&black, &white, -0.3)), "oklab(0 0 0)");
        assert_eq!(serialize(&animate(&black, &white, 1.5)), "oklab(1 0 0)");
        // A legacy colour from none: the alpha clamped before the colour
        // is unpremultiplied by it.
        let (none, green) = (Color::TRANSPARENT, parse("green").unwrap());
        let c = animate(&none, &green, 1.5);
        assert_eq!((c.g * 255.0).round(), 192.0);
    }

    #[test]
    fn numbers_keep_six_figures() {
        assert_eq!(number(0.504_777_3), "0.504777");
        assert_eq!(number(301.606_2), "301.606");
        assert_eq!(number(-37.5), "-37.5");
        assert_eq!(number(-0.0), "0");
        assert_eq!(number(100.0), "100");
    }
}
