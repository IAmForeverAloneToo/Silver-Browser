//! The built-in functions (GLSL ES 1.00 and 3.00, section 8, and the
//! extensions WebGL offers): which overload a call names, by its
//! arguments' types, the version, the stage and the extensions enabled;
//! and the values of those of constant arguments.

use crate::Stage;
use crate::tir::*;

/// What decides which built-ins a shader has.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub stage: Stage,
    pub version: u32,
    /// `dFdx` and its kin: GLSL ES 3.00, or OES_standard_derivatives.
    pub derivatives: bool,
    /// The `...Lod` and `...Grad` functions in fragment shaders:
    /// EXT_shader_texture_lod.
    pub texture_lod: bool,
    pub frag_depth: bool,
    pub draw_buffers: bool,
}

/// Every built-in function's name.
const NAMES: &[&str] = &[
    "radians",
    "degrees",
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "pow",
    "exp",
    "log",
    "exp2",
    "log2",
    "sqrt",
    "inversesqrt",
    "abs",
    "sign",
    "floor",
    "ceil",
    "fract",
    "mod",
    "min",
    "max",
    "clamp",
    "mix",
    "step",
    "smoothstep",
    "length",
    "distance",
    "dot",
    "cross",
    "normalize",
    "faceforward",
    "reflect",
    "refract",
    "matrixCompMult",
    "lessThan",
    "lessThanEqual",
    "greaterThan",
    "greaterThanEqual",
    "equal",
    "notEqual",
    "any",
    "all",
    "not",
    "texture2D",
    "texture2DProj",
    "texture2DLod",
    "texture2DProjLod",
    "textureCube",
    "textureCubeLod",
    "texture2DLodEXT",
    "texture2DProjLodEXT",
    "textureCubeLodEXT",
    "texture2DGradEXT",
    "texture2DProjGradEXT",
    "textureCubeGradEXT",
    "dFdx",
    "dFdy",
    "fwidth",
    "sinh",
    "cosh",
    "tanh",
    "asinh",
    "acosh",
    "atanh",
    "trunc",
    "round",
    "roundEven",
    "modf",
    "isnan",
    "isinf",
    "floatBitsToInt",
    "floatBitsToUint",
    "intBitsToFloat",
    "uintBitsToFloat",
    "packSnorm2x16",
    "unpackSnorm2x16",
    "packUnorm2x16",
    "unpackUnorm2x16",
    "packHalf2x16",
    "unpackHalf2x16",
    "outerProduct",
    "transpose",
    "determinant",
    "inverse",
    "texture",
    "textureProj",
    "textureLod",
    "textureOffset",
    "texelFetch",
    "texelFetchOffset",
    "textureProjOffset",
    "textureLodOffset",
    "textureProjLod",
    "textureProjLodOffset",
    "textureGrad",
    "textureGradOffset",
    "textureProjGrad",
    "textureProjGradOffset",
    "textureSize",
];

/// Those GLSL ES 3.00 adds.
const NAMES_300: &[&str] = &[
    "sinh",
    "cosh",
    "tanh",
    "asinh",
    "acosh",
    "atanh",
    "trunc",
    "round",
    "roundEven",
    "modf",
    "isnan",
    "isinf",
    "floatBitsToInt",
    "floatBitsToUint",
    "intBitsToFloat",
    "uintBitsToFloat",
    "packSnorm2x16",
    "unpackSnorm2x16",
    "packUnorm2x16",
    "unpackUnorm2x16",
    "packHalf2x16",
    "unpackHalf2x16",
    "outerProduct",
    "transpose",
    "determinant",
    "inverse",
    "texture",
    "textureProj",
    "textureLod",
    "textureOffset",
    "texelFetch",
    "texelFetchOffset",
    "textureProjOffset",
    "textureLodOffset",
    "textureProjLod",
    "textureProjLodOffset",
    "textureGrad",
    "textureGradOffset",
    "textureProjGrad",
    "textureProjGradOffset",
    "textureSize",
];

/// Those of GLSL ES 1.00 alone.
const NAMES_100: &[&str] = &[
    "texture2D",
    "texture2DProj",
    "texture2DLod",
    "texture2DProjLod",
    "textureCube",
    "textureCubeLod",
    "texture2DLodEXT",
    "texture2DProjLodEXT",
    "textureCubeLodEXT",
    "texture2DGradEXT",
    "texture2DProjGradEXT",
    "textureCubeGradEXT",
];

/// Whether `name` is a built-in function's, which the shader may not
/// define (of either version: the reference refuses both).
pub fn is_builtin_name(name: &str, ctx: &Ctx) -> bool {
    let _ = ctx;
    NAMES.contains(&name)
}

/// What a texture function does besides sampling: a projective divide,
/// an explicit level, gradients, an offset; a fetch of a texel, or the
/// texture's size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TexKind {
    pub proj: bool,
    pub lod: bool,
    pub grad: bool,
    pub offset: bool,
    pub fetch: bool,
    pub size: bool,
}

pub fn texture_kind(name: &str) -> Option<TexKind> {
    let k = |proj, lod, grad, offset| {
        Some(TexKind {
            proj,
            lod,
            grad,
            offset,
            ..TexKind::default()
        })
    };
    match name {
        "texture2D" | "textureCube" | "texture" => k(false, false, false, false),
        "texture2DProj" | "textureProj" => k(true, false, false, false),
        "texture2DLod" | "textureCubeLod" | "texture2DLodEXT" | "textureCubeLodEXT"
        | "textureLod" => k(false, true, false, false),
        "texture2DProjLod" | "texture2DProjLodEXT" | "textureProjLod" => {
            k(true, true, false, false)
        }
        "texture2DGradEXT" | "textureCubeGradEXT" | "textureGrad" => k(false, false, true, false),
        "texture2DProjGradEXT" | "textureProjGrad" => k(true, false, true, false),
        "textureOffset" => k(false, false, false, true),
        "textureProjOffset" => k(true, false, false, true),
        "textureLodOffset" => k(false, true, false, true),
        "textureProjLodOffset" => k(true, true, false, true),
        "textureGradOffset" => k(false, false, true, true),
        "textureProjGradOffset" => k(true, false, true, true),
        "texelFetch" => Some(TexKind {
            fetch: true,
            ..TexKind::default()
        }),
        "texelFetchOffset" => Some(TexKind {
            fetch: true,
            offset: true,
            ..TexKind::default()
        }),
        "textureSize" => Some(TexKind {
            size: true,
            ..TexKind::default()
        }),
        _ => None,
    }
}

/// Whether the function's last argument is an offset, which must be a
/// constant expression.
pub fn needs_constant_offset(name: &str) -> bool {
    texture_kind(name).is_some_and(|k| k.offset)
}

/// The overload a call of `name` with `args` names: its canonical name
/// and its type; `None` when there is no built-in of that name here,
/// `Some(Err)` when none of its overloads takes the arguments.
pub fn resolve(
    name: &str,
    args: &[Type],
    ctx: &Ctx,
    unit: &Unit,
) -> Option<Result<(&'static str, Type), ()>> {
    let canonical: &'static str = NAMES.iter().copied().find(|n| *n == name)?;
    let v3 = ctx.version == 300;
    if (NAMES_300.contains(&canonical) && !v3) || (NAMES_100.contains(&canonical) && v3) {
        return None;
    }
    let _ = unit;
    let ty = signature(canonical, args, ctx);
    Some(ty.map(|t| (canonical, t)))
}

fn gen_f(t: &Type) -> bool {
    t.is_gen_float()
}

fn gen_of(t: &Type, s: Scalar) -> bool {
    matches!(t, Type::Scalar(x) | Type::Vector(x, _) if *x == s)
}

/// A genType of float, int or uint.
fn gen_num(t: &Type, v3: bool) -> bool {
    gen_f(t) || (v3 && (gen_of(t, Scalar::Int) || gen_of(t, Scalar::Uint)))
}

fn signature(name: &str, a: &[Type], ctx: &Ctx) -> Result<Type, ()> {
    let v3 = ctx.version == 300;
    let fragment = ctx.stage == Stage::Fragment;
    let n = a.len();
    let ok = |b: bool, t: Type| if b { Ok(t) } else { Err(()) };
    let scalar_of = |t: &Type| Type::Scalar(t.scalar().unwrap_or(Scalar::Float));
    match name {
        "radians" | "degrees" | "sin" | "cos" | "tan" | "asin" | "acos" | "exp" | "log"
        | "exp2" | "log2" | "sqrt" | "inversesqrt" | "floor" | "ceil" | "fract" | "normalize"
        | "sinh" | "cosh" | "tanh" | "asinh" | "acosh" | "atanh" | "trunc" | "round"
        | "roundEven" => ok(
            n == 1 && gen_f(&a[0]),
            a.first().cloned().unwrap_or(Type::Void),
        ),
        "abs" | "sign" => ok(
            n == 1 && (gen_f(&a[0]) || (v3 && gen_of(&a[0], Scalar::Int))),
            a.first().cloned().unwrap_or(Type::Void),
        ),
        "atan" => match n {
            1 => ok(gen_f(&a[0]), a[0].clone()),
            2 => ok(gen_f(&a[0]) && a[0] == a[1], a[0].clone()),
            _ => Err(()),
        },
        "pow" => ok(
            n == 2 && gen_f(&a[0]) && a[0] == a[1],
            a.first().cloned().unwrap_or(Type::Void),
        ),
        "mod" => ok(
            n == 2 && gen_f(&a[0]) && (a[0] == a[1] || a[1] == FLOAT),
            a.first().cloned().unwrap_or(Type::Void),
        ),
        "min" | "max" => ok(
            n == 2 && gen_num(&a[0], v3) && (a[0] == a[1] || a[1] == scalar_of(&a[0])),
            a.first().cloned().unwrap_or(Type::Void),
        ),
        "clamp" => ok(
            n == 3
                && gen_num(&a[0], v3)
                && ((a[1] == a[0] && a[2] == a[0])
                    || (a[1] == scalar_of(&a[0]) && a[2] == scalar_of(&a[0]))),
            a.first().cloned().unwrap_or(Type::Void),
        ),
        "mix" => ok(
            n == 3
                && gen_f(&a[0])
                && a[1] == a[0]
                && (a[2] == a[0]
                    || a[2] == FLOAT
                    || (v3 && gen_of(&a[2], Scalar::Bool) && a[2].size() == a[0].size())),
            a.first().cloned().unwrap_or(Type::Void),
        ),
        "step" => ok(
            n == 2 && gen_f(&a[1]) && (a[0] == a[1] || a[0] == FLOAT),
            a.get(1).cloned().unwrap_or(Type::Void),
        ),
        "smoothstep" => ok(
            n == 3
                && gen_f(&a[2])
                && ((a[0] == a[2] && a[1] == a[2]) || (a[0] == FLOAT && a[1] == FLOAT)),
            a.get(2).cloned().unwrap_or(Type::Void),
        ),
        "length" => ok(n == 1 && gen_f(&a[0]), FLOAT),
        "distance" | "dot" => ok(n == 2 && gen_f(&a[0]) && a[0] == a[1], FLOAT),
        "cross" => {
            let v3t = Type::Vector(Scalar::Float, 3);
            ok(n == 2 && a[0] == v3t && a[1] == v3t, v3t)
        }
        "faceforward" => ok(
            n == 3 && gen_f(&a[0]) && a[1] == a[0] && a[2] == a[0],
            a.first().cloned().unwrap_or(Type::Void),
        ),
        "reflect" => ok(
            n == 2 && gen_f(&a[0]) && a[1] == a[0],
            a.first().cloned().unwrap_or(Type::Void),
        ),
        "refract" => ok(
            n == 3 && gen_f(&a[0]) && a[1] == a[0] && a[2] == FLOAT,
            a.first().cloned().unwrap_or(Type::Void),
        ),
        "matrixCompMult" => match (a.first(), a.get(1)) {
            (Some(Type::Matrix(c, r)), Some(b)) if n == 2 && a[0] == *b && (v3 || c == r) => {
                Ok(a[0].clone())
            }
            _ => Err(()),
        },
        "outerProduct" => match (a.first(), a.get(1)) {
            (Some(Type::Vector(Scalar::Float, r)), Some(Type::Vector(Scalar::Float, c)))
                if n == 2 =>
            {
                Ok(Type::Matrix(*c, *r))
            }
            _ => Err(()),
        },
        "transpose" => match a.first() {
            Some(Type::Matrix(c, r)) if n == 1 => Ok(Type::Matrix(*r, *c)),
            _ => Err(()),
        },
        "determinant" => match a.first() {
            Some(Type::Matrix(c, r)) if n == 1 && c == r => Ok(FLOAT),
            _ => Err(()),
        },
        "inverse" => match a.first() {
            Some(Type::Matrix(c, r)) if n == 1 && c == r => Ok(a[0].clone()),
            _ => Err(()),
        },
        "lessThan" | "lessThanEqual" | "greaterThan" | "greaterThanEqual" => match a.first() {
            Some(Type::Vector(s, size))
                if n == 2
                    && a[1] == a[0]
                    && (*s == Scalar::Float || *s == Scalar::Int || (v3 && *s == Scalar::Uint)) =>
            {
                Ok(Type::Vector(Scalar::Bool, *size))
            }
            _ => Err(()),
        },
        "equal" | "notEqual" => match a.first() {
            Some(Type::Vector(_, size)) if n == 2 && a[1] == a[0] => {
                Ok(Type::Vector(Scalar::Bool, *size))
            }
            _ => Err(()),
        },
        "any" | "all" => ok(
            n == 1 && matches!(a[0], Type::Vector(Scalar::Bool, _)),
            BOOL,
        ),
        "not" => ok(
            n == 1 && matches!(a[0], Type::Vector(Scalar::Bool, _)),
            a.first().cloned().unwrap_or(Type::Void),
        ),
        "dFdx" | "dFdy" | "fwidth" => ok(
            fragment && ctx.derivatives && n == 1 && gen_f(&a[0]),
            a.first().cloned().unwrap_or(Type::Void),
        ),
        "modf" => ok(
            n == 2 && gen_f(&a[0]) && a[1] == a[0],
            a.first().cloned().unwrap_or(Type::Void),
        ),
        "isnan" | "isinf" => ok(
            n == 1 && gen_f(&a[0]),
            Type::vector(Scalar::Bool, a.first().map_or(1, Type::size)),
        ),
        "floatBitsToInt" => ok(
            n == 1 && gen_f(&a[0]),
            Type::vector(Scalar::Int, a.first().map_or(1, Type::size)),
        ),
        "floatBitsToUint" => ok(
            n == 1 && gen_f(&a[0]),
            Type::vector(Scalar::Uint, a.first().map_or(1, Type::size)),
        ),
        "intBitsToFloat" => ok(
            n == 1 && gen_of(&a[0], Scalar::Int),
            Type::vector(Scalar::Float, a.first().map_or(1, Type::size)),
        ),
        "uintBitsToFloat" => ok(
            n == 1 && gen_of(&a[0], Scalar::Uint),
            Type::vector(Scalar::Float, a.first().map_or(1, Type::size)),
        ),
        "packSnorm2x16" | "packUnorm2x16" | "packHalf2x16" => {
            ok(n == 1 && a[0] == Type::Vector(Scalar::Float, 2), UINT)
        }
        "unpackSnorm2x16" | "unpackUnorm2x16" | "unpackHalf2x16" => {
            ok(n == 1 && a[0] == UINT, Type::Vector(Scalar::Float, 2))
        }
        _ => texture_signature(name, a, ctx),
    }
}

fn texture_signature(name: &str, a: &[Type], ctx: &Ctx) -> Result<Type, ()> {
    let kind = texture_kind(name).ok_or(())?;
    let fragment = ctx.stage == Stage::Fragment;
    let Some(Type::Sampler(s)) = a.first() else {
        return Err(());
    };
    // GLSL ES 1.00's: sampler2D's and samplerCube's by name; the
    // explicit levels in vertex shaders, or EXT_shader_texture_lod's in
    // fragment shaders.
    if ctx.version == 100 {
        let cube = name.starts_with("textureCube");
        let want = if cube { Dim::Cube } else { Dim::D2 };
        if s.dim != want || s.shadow {
            return Err(());
        }
        let ext = name.ends_with("EXT");
        if ext && !(fragment && ctx.texture_lod) {
            return Err(());
        }
        if kind.lod && !ext && fragment {
            return Err(());
        }
    }
    let float = |n: u8| Type::vector(Scalar::Float, n);
    let int = |n: u8| Type::vector(Scalar::Int, n);
    if kind.size {
        let dims = match s.dim {
            Dim::D2 | Dim::Cube => 2,
            Dim::D3 | Dim::D2Array => 3,
        };
        return if a.len() == 2 && a[1] == INT {
            Ok(int(dims))
        } else {
            Err(())
        };
    }
    let result = if s.shadow {
        FLOAT
    } else {
        Type::Vector(s.sample, 4)
    };
    if kind.fetch {
        if s.dim == Dim::Cube || s.shadow {
            return Err(());
        }
        let dims = if s.dim == Dim::D2 { 2 } else { 3 };
        let offset = if s.dim == Dim::D3 { 3 } else { 2 };
        let want: Vec<Type> = if kind.offset {
            vec![int(dims), INT, int(offset)]
        } else {
            vec![int(dims), INT]
        };
        return if a[1..] == want[..] {
            Ok(result)
        } else {
            Err(())
        };
    }
    // Integer textures are not sampled with gradients or projected
    // shadows: as the standard has it, else refused.
    let coords: u8 = match s.dim {
        Dim::D2 => 2,
        Dim::D3 | Dim::Cube | Dim::D2Array => 3,
    } + u8::from(s.shadow);
    let mut i = 1;
    let coord = a.get(i).ok_or(())?;
    if kind.proj {
        let allowed = match (s.dim, s.shadow) {
            (Dim::D2, false) => *coord == float(3) || *coord == float(4),
            (Dim::D2, true) | (Dim::D3, false) => *coord == float(4),
            _ => false,
        };
        if !allowed {
            return Err(());
        }
    } else if *coord != float(coords) {
        return Err(());
    }
    i += 1;
    if kind.lod {
        if s.shadow && s.dim != Dim::D2 {
            return Err(());
        }
        if a.get(i) != Some(&FLOAT) {
            return Err(());
        }
        i += 1;
    }
    if kind.grad {
        let g = match s.dim {
            Dim::D2 | Dim::D2Array => 2,
            Dim::D3 | Dim::Cube => 3,
        };
        if a.get(i) != Some(&float(g)) || a.get(i + 1) != Some(&float(g)) {
            return Err(());
        }
        i += 2;
    }
    if kind.offset {
        if s.dim == Dim::Cube {
            return Err(());
        }
        let o = if s.dim == Dim::D3 { 3 } else { 2 };
        if a.get(i) != Some(&int(o)) {
            return Err(());
        }
        i += 1;
    }
    // A bias, in fragment shaders.
    if i + 1 == a.len() && !kind.lod && !kind.grad && a[i] == FLOAT && fragment {
        if s.shadow && s.dim == Dim::D2Array {
            return Err(());
        }
        i += 1;
    }
    if i != a.len() {
        return Err(());
    }
    Ok(result)
}

/// The value of a built-in of constant arguments, where it is one GLSL
/// ES lets constant expressions call (the trigonometric, exponential and
/// common functions, the geometric ones, the vectors' relations).
pub fn fold(name: &str, args: &[Expr], ty: &Type) -> Option<Vec<Comp>> {
    let values: Vec<&Vec<Comp>> = args
        .iter()
        .map(|a| a.value.as_ref())
        .collect::<Option<_>>()?;
    let n = ty.components().max(1) as usize;
    let at = |v: &[Comp], i: usize| if v.len() == 1 { v[0] } else { v[i] };
    let f1 = |op: fn(f32) -> f32| -> Option<Vec<Comp>> {
        Some(
            (0..n)
                .map(|i| Comp::F(op(at(values[0], i).as_f32())))
                .collect(),
        )
    };
    let f2 = |op: fn(f32, f32) -> f32| -> Option<Vec<Comp>> {
        Some(
            (0..n)
                .map(|i| Comp::F(op(at(values[0], i).as_f32(), at(values[1], i).as_f32())))
                .collect(),
        )
    };
    let out = match name {
        "radians" => f1(f32::to_radians)?,
        "degrees" => f1(f32::to_degrees)?,
        "sin" => f1(f32::sin)?,
        "cos" => f1(f32::cos)?,
        "tan" => f1(f32::tan)?,
        "asin" => f1(f32::asin)?,
        "acos" => f1(f32::acos)?,
        "atan" if args.len() == 1 => f1(f32::atan)?,
        "atan" => f2(f32::atan2)?,
        "pow" => f2(f32::powf)?,
        "exp" => f1(f32::exp)?,
        "log" => f1(f32::ln)?,
        "exp2" => f1(f32::exp2)?,
        "log2" => f1(f32::log2)?,
        "sqrt" => f1(f32::sqrt)?,
        "inversesqrt" => f1(|x| 1.0 / x.sqrt())?,
        "floor" => f1(f32::floor)?,
        "ceil" => f1(f32::ceil)?,
        "fract" => f1(|x| x - x.floor())?,
        "trunc" => f1(f32::trunc)?,
        "round" | "roundEven" => f1(f32::round_ties_even)?,
        "mod" => f2(|x, y| x - y * (x / y).floor())?,
        "abs" | "sign" | "min" | "max" | "clamp" => {
            let s = ty.scalar()?;
            (0..n)
                .map(|i| {
                    let v: Vec<Comp> = values.iter().map(|v| at(v, i)).collect();
                    Some(match (s, name) {
                        (Scalar::Float, "abs") => Comp::F(v[0].as_f32().abs()),
                        (Scalar::Float, "sign") => Comp::F(sign(v[0].as_f32())),
                        (Scalar::Float, "min") => Comp::F(v[0].as_f32().min(v[1].as_f32())),
                        (Scalar::Float, "max") => Comp::F(v[0].as_f32().max(v[1].as_f32())),
                        (Scalar::Float, _) => {
                            Comp::F(v[0].as_f32().max(v[1].as_f32()).min(v[2].as_f32()))
                        }
                        (Scalar::Int, "abs") => Comp::I(v[0].as_i32().wrapping_abs()),
                        (Scalar::Int, "sign") => Comp::I(v[0].as_i32().signum()),
                        (Scalar::Int, "min") => Comp::I(v[0].as_i32().min(v[1].as_i32())),
                        (Scalar::Int, "max") => Comp::I(v[0].as_i32().max(v[1].as_i32())),
                        (Scalar::Int, _) => {
                            Comp::I(v[0].as_i32().max(v[1].as_i32()).min(v[2].as_i32()))
                        }
                        (Scalar::Uint, "min") => Comp::U(v[0].as_u32().min(v[1].as_u32())),
                        (Scalar::Uint, "max") => Comp::U(v[0].as_u32().max(v[1].as_u32())),
                        (Scalar::Uint, "clamp") => {
                            Comp::U(v[0].as_u32().max(v[1].as_u32()).min(v[2].as_u32()))
                        }
                        _ => return None,
                    })
                })
                .collect::<Option<Vec<_>>>()?
        }
        "mix" if args[2].ty.scalar() == Some(Scalar::Float) => (0..n)
            .map(|i| {
                let (x, y, t) = (
                    at(values[0], i).as_f32(),
                    at(values[1], i).as_f32(),
                    at(values[2], i).as_f32(),
                );
                Comp::F(x * (1.0 - t) + y * t)
            })
            .collect(),
        "step" => (0..n)
            .map(|i| {
                Comp::F(if at(values[1], i).as_f32() < at(values[0], i).as_f32() {
                    0.0
                } else {
                    1.0
                })
            })
            .collect(),
        "smoothstep" => (0..n)
            .map(|i| {
                let (e0, e1, x) = (
                    at(values[0], i).as_f32(),
                    at(values[1], i).as_f32(),
                    at(values[2], i).as_f32(),
                );
                let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
                Comp::F(t * t * (3.0 - 2.0 * t))
            })
            .collect(),
        "length" => vec![Comp::F(
            values[0]
                .iter()
                .map(|c| c.as_f32() * c.as_f32())
                .sum::<f32>()
                .sqrt(),
        )],
        "dot" => vec![Comp::F(
            values[0]
                .iter()
                .zip(values[1].iter())
                .map(|(a, b)| a.as_f32() * b.as_f32())
                .sum(),
        )],
        "distance" => vec![Comp::F(
            values[0]
                .iter()
                .zip(values[1].iter())
                .map(|(a, b)| (a.as_f32() - b.as_f32()).powi(2))
                .sum::<f32>()
                .sqrt(),
        )],
        "normalize" => {
            let length = values[0]
                .iter()
                .map(|c| c.as_f32() * c.as_f32())
                .sum::<f32>()
                .sqrt();
            values[0]
                .iter()
                .map(|c| Comp::F(c.as_f32() / length))
                .collect()
        }
        "cross" => {
            let (a, b) = (values[0], values[1]);
            let g = |i: usize| a[i].as_f32();
            let h = |i: usize| b[i].as_f32();
            vec![
                Comp::F(g(1) * h(2) - h(1) * g(2)),
                Comp::F(g(2) * h(0) - h(2) * g(0)),
                Comp::F(g(0) * h(1) - h(0) * g(1)),
            ]
        }
        "lessThan" | "lessThanEqual" | "greaterThan" | "greaterThanEqual" | "equal"
        | "notEqual" => (0..n)
            .map(|i| {
                let (x, y) = (values[0][i], values[1][i]);
                let ord = match (x, y) {
                    (Comp::F(a), Comp::F(b)) => a.partial_cmp(&b),
                    (Comp::I(a), Comp::I(b)) => Some(a.cmp(&b)),
                    (Comp::U(a), Comp::U(b)) => Some(a.cmp(&b)),
                    (Comp::B(a), Comp::B(b)) => Some(a.cmp(&b)),
                    _ => None,
                };
                use std::cmp::Ordering::*;
                Comp::B(match (name, ord) {
                    ("lessThan", Some(Less)) => true,
                    ("lessThanEqual", Some(Less | Equal)) => true,
                    ("greaterThan", Some(Greater)) => true,
                    ("greaterThanEqual", Some(Greater | Equal)) => true,
                    ("equal", Some(Equal)) => true,
                    ("notEqual", o) => o != Some(Equal),
                    _ => false,
                })
            })
            .collect(),
        "any" => vec![Comp::B(values[0].iter().any(|c| c.as_bool()))],
        "all" => vec![Comp::B(values[0].iter().all(|c| c.as_bool()))],
        "not" => values[0].iter().map(|c| Comp::B(!c.as_bool())).collect(),
        "matrixCompMult" => values[0]
            .iter()
            .zip(values[1].iter())
            .map(|(a, b)| Comp::F(a.as_f32() * b.as_f32()))
            .collect(),
        _ => return None,
    };
    // Constant expressions keep to finite values: one that is not is
    // left to the GPU.
    if out
        .iter()
        .any(|c| matches!(c, Comp::F(v) if !v.is_finite()))
    {
        return None;
    }
    Some(out)
}

fn sign(x: f32) -> f32 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}
