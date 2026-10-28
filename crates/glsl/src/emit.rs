//! A program's two shaders to WGSL (D40), and the link that matches
//! them: varyings by name, attributes at their locations, the uniforms
//! of both in one buffer. naga reads and validates what is written, and
//! gives the uniforms' places.
//!
//! The uniforms are laid out as GLSL ES 3.00's std140 lays a block out,
//! in WGSL's uniform rules: an array's scalars and vectors each in a
//! vec4, a matrix of two rows as an array of vec4 columns, a row-major
//! matrix as its rows, booleans as u32; each read turned back to its
//! GLSL type where it is read. Expressions with side effects are taken
//! apart into statements in GLSL's order: WGSL's assignments and
//! increments are statements, its `&&`, `||` and `select` evaluate
//! differently.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::builtins::texture_kind;
use crate::expr::has_effects;
use crate::tir::*;
use crate::{Block, Shader, Stage};

// ---- What a link gives ----

/// What the link is told: the locations `bindAttribLocation` gave, and
/// the context's limits.
#[derive(Clone, Debug)]
pub struct LinkOptions {
    pub attribute_locations: Vec<(String, u32)>,
    pub max_vertex_attribs: u32,
    pub max_varying_vectors: u32,
    pub max_draw_buffers: u32,
}

impl Default for LinkOptions {
    fn default() -> LinkOptions {
        LinkOptions {
            attribute_locations: Vec::new(),
            max_vertex_attribs: 16,
            max_varying_vectors: 15,
            max_draw_buffers: 8,
        }
    }
}

/// An active attribute: its GL type, its location and the locations it
/// takes (a matrix's columns), its scalar and components.
#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub gl_type: u32,
    pub location: u32,
    pub locations: u32,
    pub scalar: Scalar,
    pub components: u32,
}

/// An active uniform: its name as `getActiveUniform` gives it (`a[0]`
/// for an array), its GL type and array size, and where its first
/// element is in its buffer: its offset, the bytes between elements and
/// between a matrix's columns (or rows, when row-major).
#[derive(Clone, Debug, PartialEq)]
pub struct Uniform {
    pub name: String,
    pub gl_type: u32,
    pub size: u32,
    pub offset: u32,
    pub array_stride: u32,
    pub matrix_stride: u32,
    pub row_major: bool,
    /// The block it is a member of, by its index in [`Program::blocks`].
    pub block: Option<usize>,
}

pub type BlockUniform = Uniform;

/// An active sampler uniform: each element's texture binding (its
/// sampler's is the next).
#[derive(Clone, Debug, PartialEq)]
pub struct SamplerUniform {
    pub name: String,
    pub gl_type: u32,
    pub size: u32,
    pub bindings: Vec<u32>,
    pub sampler: Sampler,
}

/// An active uniform block: its buffer's size, a binding for each of its
/// array's elements, the stages that read it.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockInfo {
    pub name: String,
    pub size: u32,
    pub bindings: Vec<u32>,
    pub vertex: bool,
    pub fragment: bool,
}

/// A fragment output: its location, scalar and components.
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    pub location: u32,
    pub scalar: Scalar,
    pub components: u32,
}

/// A linked program.
#[derive(Clone, Debug)]
pub struct Program {
    pub vertex: String,
    pub fragment: String,
    /// The vertex shader that draws points of `gl_PointSize`, when the
    /// vertex shader writes it: an instance a point, four vertices of a
    /// triangle strip its quad.
    pub points: Option<String>,
    pub attributes: Vec<Attribute>,
    pub uniforms: Vec<Uniform>,
    pub samplers: Vec<SamplerUniform>,
    pub blocks: Vec<BlockInfo>,
    /// The default uniform buffer's size; its first 32 bytes are the
    /// translation's own (the viewport and the depth range).
    pub uniform_size: u32,
    pub outputs: Vec<Output>,
    pub writes_depth: bool,
}

/// The binding of the default uniforms.
pub const UNIFORM_BINDING: u32 = 0;

// ---- GL's names of types ----

pub fn gl_type(ty: &Type) -> u32 {
    match ty {
        Type::Scalar(Scalar::Float) => 0x1406,
        Type::Scalar(Scalar::Int) => 0x1404,
        Type::Scalar(Scalar::Uint) => 0x1405,
        Type::Scalar(Scalar::Bool) => 0x8B56,
        Type::Vector(Scalar::Float, n) => 0x8B50 + u32::from(*n) - 2,
        Type::Vector(Scalar::Int, n) => 0x8B53 + u32::from(*n) - 2,
        Type::Vector(Scalar::Bool, n) => 0x8B57 + u32::from(*n) - 2,
        Type::Vector(Scalar::Uint, n) => 0x8DC6 + u32::from(*n) - 2,
        Type::Matrix(2, 2) => 0x8B5A,
        Type::Matrix(3, 3) => 0x8B5B,
        Type::Matrix(4, 4) => 0x8B5C,
        Type::Matrix(2, 3) => 0x8B65,
        Type::Matrix(2, 4) => 0x8B66,
        Type::Matrix(3, 2) => 0x8B67,
        Type::Matrix(3, 4) => 0x8B68,
        Type::Matrix(4, 2) => 0x8B69,
        Type::Matrix(4, 3) => 0x8B6A,
        Type::Sampler(s) => match (s.dim, s.shadow, s.sample) {
            (Dim::D2, false, Scalar::Float) => 0x8B5E,
            (Dim::D3, false, Scalar::Float) => 0x8B5F,
            (Dim::Cube, false, Scalar::Float) => 0x8B60,
            (Dim::D2, true, _) => 0x8B62,
            (Dim::D2Array, false, Scalar::Float) => 0x8DC1,
            (Dim::D2Array, true, _) => 0x8DC4,
            (Dim::Cube, true, _) => 0x8DC5,
            (Dim::D2, false, Scalar::Int) => 0x8DCA,
            (Dim::D3, false, Scalar::Int) => 0x8DCB,
            (Dim::Cube, false, Scalar::Int) => 0x8DCC,
            (Dim::D2Array, false, Scalar::Int) => 0x8DCF,
            (Dim::D2, false, _) => 0x8DD2,
            (Dim::D3, false, _) => 0x8DD3,
            (Dim::Cube, false, _) => 0x8DD4,
            (Dim::D2Array, false, _) => 0x8DD7,
            (Dim::D3, true, _) => 0,
        },
        _ => 0,
    }
}

// ---- WGSL's types ----

fn scalar_wgsl(s: Scalar) -> &'static str {
    match s {
        Scalar::Float => "f32",
        Scalar::Int => "i32",
        Scalar::Uint => "u32",
        Scalar::Bool => "bool",
    }
}

pub fn wgsl_type(ty: &Type) -> String {
    match ty {
        Type::Void => String::new(),
        Type::Scalar(s) => scalar_wgsl(*s).to_owned(),
        Type::Vector(s, n) => format!("vec{n}<{}>", scalar_wgsl(*s)),
        Type::Matrix(c, r) => format!("mat{c}x{r}<f32>"),
        Type::Struct(i) => format!("S{i}"),
        Type::Array(element, n) => format!("array<{}, {n}>", wgsl_type(element)),
        Type::Sampler(_) => "webgl_sampler".to_owned(),
    }
}

fn texture_type(s: &Sampler) -> String {
    if s.shadow {
        return match s.dim {
            Dim::D2 => "texture_depth_2d",
            Dim::Cube => "texture_depth_cube",
            Dim::D2Array => "texture_depth_2d_array",
            Dim::D3 => "texture_3d<f32>",
        }
        .to_owned();
    }
    let dim = match s.dim {
        Dim::D2 => "2d",
        Dim::D3 => "3d",
        // An integer cube is read by texel from its six faces as layers:
        // WGSL loads no texel of a cube.
        Dim::Cube if s.sample != Scalar::Float => "2d_array",
        Dim::Cube => "cube",
        Dim::D2Array => "2d_array",
    };
    format!("texture_{dim}<{}>", scalar_wgsl(s.sample))
}

/// A constant's WGSL, from its components.
fn literal(ty: &Type, comps: &[Comp]) -> String {
    match ty {
        Type::Scalar(s) => scalar_literal(comps[0].to(*s)),
        Type::Vector(s, n) => {
            let parts: Vec<String> = comps[..*n as usize]
                .iter()
                .map(|c| scalar_literal(c.to(*s)))
                .collect();
            format!("{}({})", wgsl_type(ty), parts.join(", "))
        }
        Type::Matrix(c, r) => {
            let r = *r as usize;
            let cols: Vec<String> = (0..*c as usize)
                .map(|i| {
                    literal(
                        &Type::Vector(Scalar::Float, r as u8),
                        &comps[i * r..(i + 1) * r],
                    )
                })
                .collect();
            format!("{}({})", wgsl_type(ty), cols.join(", "))
        }
        _ => format!("{}()", wgsl_type(ty)),
    }
}

fn scalar_literal(c: Comp) -> String {
    match c {
        Comp::F(v) if v.is_finite() => {
            let text = format!("{v:?}");
            format!("{text}f")
        }
        Comp::F(v) if v.is_nan() => "webgl_nan()".to_owned(),
        Comp::F(v) if v > 0.0 => "webgl_inf()".to_owned(),
        Comp::F(_) => "-webgl_inf()".to_owned(),
        Comp::I(i32::MIN) => "i32(-2147483647 - 1)".to_owned(),
        Comp::I(v) => format!("{v}i"),
        Comp::U(v) => format!("{v}u"),
        Comp::B(v) => v.to_string(),
    }
}

/// A full constant of any type, by its flattened components.
fn constant(unit: &Unit, ty: &Type, comps: &[Comp]) -> String {
    match ty {
        Type::Array(element, n) => {
            let size = flat_size(unit, element) as usize;
            let parts: Vec<String> = (0..*n as usize)
                .map(|i| constant(unit, element, &comps[i * size..(i + 1) * size]))
                .collect();
            format!("{}({})", wgsl_type(ty), parts.join(", "))
        }
        Type::Struct(s) => {
            let mut at = 0;
            let mut parts = Vec::new();
            for (_, field) in &unit.structs[*s].fields {
                let size = flat_size(unit, field) as usize;
                parts.push(constant(unit, field, &comps[at..at + size]));
                at += size;
            }
            format!("{}({})", wgsl_type(ty), parts.join(", "))
        }
        _ => literal(ty, comps),
    }
}

fn flat_size(unit: &Unit, ty: &Type) -> u32 {
    match ty {
        Type::Array(element, n) => flat_size(unit, element) * n,
        Type::Struct(s) => unit.structs[*s]
            .fields
            .iter()
            .map(|(_, t)| flat_size(unit, t))
            .sum(),
        other => other.components(),
    }
}

const XYZW: [char; 4] = ['x', 'y', 'z', 'w'];

fn swizzle_text(comps: &[u8]) -> String {
    comps.iter().map(|&c| XYZW[c as usize]).collect()
}

fn first(n: u8) -> String {
    match n {
        1 => ".x".to_owned(),
        2 => ".xy".to_owned(),
        3 => ".xyz".to_owned(),
        _ => String::new(),
    }
}

// ---- The uniforms' layouts ----

/// How a uniform's value is kept in its buffer.
#[derive(Clone, Debug, PartialEq)]
enum Lay {
    /// As its WGSL type.
    Same(Type),
    /// A boolean or boolean vector, as u32s.
    Bool(Type),
    /// An array's scalar or vector, in a vec4.
    Wrapped(Box<Lay>),
    /// A matrix of two rows, column-major: an array of its columns.
    Cols(u8, u8),
    /// A row-major matrix: an array of its rows.
    Rows(u8, u8),
    Array(Box<Lay>, u32),
    /// A struct of the vertex (`'V'`) or the fragment (`'F'`) shader.
    Struct(char, usize),
}

fn lay(ty: &Type, row_major: bool, tag: char) -> Lay {
    match ty {
        Type::Scalar(Scalar::Bool) | Type::Vector(Scalar::Bool, _) => Lay::Bool(ty.clone()),
        Type::Matrix(c, r) if row_major => Lay::Rows(*c, *r),
        Type::Matrix(c, 2) => Lay::Cols(*c, 2),
        Type::Array(element, n) => {
            let inner = lay(element, row_major, tag);
            let inner = match &inner {
                Lay::Same(t) if t.is_scalar_or_vector() && t.size() != 4 => {
                    Lay::Wrapped(Box::new(inner))
                }
                Lay::Bool(t) if t.size() != 4 => Lay::Wrapped(Box::new(inner)),
                _ => inner,
            };
            Lay::Array(Box::new(inner), *n)
        }
        Type::Struct(s) => Lay::Struct(tag, *s),
        other => Lay::Same(other.clone()),
    }
}

/// Whether a layout keeps its value as the value's WGSL type.
fn lay_is_same(l: &Lay) -> bool {
    match l {
        Lay::Same(_) => true,
        Lay::Array(e, _) => lay_is_same(e),
        _ => false,
    }
}

// ---- The interface the link settles ----

#[derive(Clone, Debug, Default)]
struct Interface {
    /// The default block's uniforms (not samplers), by name: the vertex
    /// shader's type and the fragment shader's, where each declares it.
    uniforms: Vec<(String, Option<Type>, Option<Type>)>,
    /// The samplers: their names, types and first element's number.
    samplers: Vec<(String, Type, u32)>,
    /// The blocks, by name: their first binding.
    blocks: Vec<(String, u32)>,
    /// The varyings the fragment shader reads: name, type, first
    /// location, flat.
    varyings: Vec<(String, Type, u32, bool)>,
    /// `gl_PointCoord`'s location, when the fragment shader reads it.
    point_coord: Option<u32>,
    /// The attributes' locations, by name.
    attributes: BTreeMap<String, u32>,
    /// The fragment shader's outputs: the global, its location.
    outputs: Vec<(usize, u32)>,
}

/// The locations a varying of `ty` takes.
fn locations(ty: &Type) -> u32 {
    match ty {
        Type::Matrix(c, _) => u32::from(*c),
        Type::Array(element, n) => locations(element) * n,
        _ => 1,
    }
}

pub fn link(vertex: &Shader, fragment: &Shader, options: &LinkOptions) -> Result<Program, String> {
    let fail = |message: &str| Err(format!("{message}\n"));
    if vertex.unit.stage != Stage::Vertex || fragment.unit.stage != Stage::Fragment {
        return fail("Linking needs a vertex and a fragment shader.");
    }
    if vertex.unit.version != fragment.unit.version {
        return fail("Version of vertex shader and fragment shader differ.");
    }
    let mut interface = Interface::default();
    // Varyings: each the fragment shader reads, the vertex shader's of
    // the same name and type.
    let mut next_location = 0;
    for (g, input) in fragment.unit.globals.iter().enumerate() {
        let GlobalKind::Varying {
            output: false,
            flat,
        } = input.kind
        else {
            continue;
        };
        let _ = g;
        if !input.used {
            continue;
        }
        let Some((_, out)) = vertex
            .unit
            .global(&input.name)
            .filter(|(_, o)| matches!(o.kind, GlobalKind::Varying { output: true, .. }))
        else {
            return fail(&format!(
                "Varying '{}' not found in vertex shader.",
                input.name
            ));
        };
        if out.ty != input.ty {
            return fail(&format!(
                "Types of varying '{}' differ between vertex and fragment shaders.",
                input.name
            ));
        }
        if let GlobalKind::Varying { flat: out_flat, .. } = out.kind
            && out_flat != flat
            && vertex.unit.version == 300
        {
            return fail(&format!(
                "Interpolation types for '{}' differ between vertex and fragment shaders.",
                input.name
            ));
        }
        let n = locations(&input.ty);
        interface
            .varyings
            .push((input.name.clone(), input.ty.clone(), next_location, flat));
        next_location += n;
    }
    if fragment
        .unit
        .globals
        .iter()
        .any(|g| g.used && g.kind == GlobalKind::Builtin(BuiltinVar::PointCoord))
    {
        interface.point_coord = Some(next_location);
        next_location += 1;
    }
    if next_location > options.max_varying_vectors + u32::from(interface.point_coord.is_some()) {
        return fail("Could not pack varying: too many varyings.");
    }
    // Uniforms: the default block's of both, the same type where both
    // declare one.
    let mut binding = 1;
    for shader in [vertex, fragment] {
        for global in &shader.unit.globals {
            if global.kind != GlobalKind::Uniform || global.name.starts_with("webgl_block") {
                continue;
            }
            let other = if std::ptr::eq(shader, vertex) {
                fragment
            } else {
                vertex
            };
            if let Some((_, o)) = other.unit.global(&global.name)
                && o.kind == GlobalKind::Uniform
                && !same_type(&shader.unit, &global.ty, &other.unit, &o.ty)
            {
                return fail(&format!(
                    "Types of uniform '{}' differ between vertex and fragment shaders.",
                    global.name
                ));
            }
            let used = global.used
                || other
                    .unit
                    .global(&global.name)
                    .is_some_and(|(_, o)| o.used && o.kind == GlobalKind::Uniform);
            if !used {
                continue;
            }
            if global.ty.has_sampler(&shader.unit) {
                if interface.samplers.iter().any(|(n, _, _)| *n == global.name) {
                    continue;
                }
                let count = match &global.ty {
                    Type::Array(_, n) => *n,
                    _ => 1,
                };
                interface
                    .samplers
                    .push((global.name.clone(), global.ty.clone(), binding));
                binding += 2 * count;
            } else if !interface.uniforms.iter().any(|(n, _, _)| *n == global.name) {
                let of = |s: &Shader| {
                    s.unit
                        .global(&global.name)
                        .filter(|(_, g)| g.kind == GlobalKind::Uniform)
                        .map(|(_, g)| g.ty.clone())
                };
                interface
                    .uniforms
                    .push((global.name.clone(), of(vertex), of(fragment)));
            }
        }
    }
    // Uniform blocks: by name, the same members.
    for shader in [vertex, fragment] {
        for block in &shader.blocks {
            let other = if std::ptr::eq(shader, vertex) {
                fragment
            } else {
                vertex
            };
            if let Some(o) = other.blocks.iter().find(|b| b.name == block.name) {
                let same = o.members.len() == block.members.len()
                    && o.members.iter().zip(&block.members).all(|(a, b)| {
                        a.name == b.name
                            && a.row_major == b.row_major
                            && same_type(&other.unit, &a.ty, &shader.unit, &b.ty)
                    })
                    && o.array == block.array;
                if !same {
                    return fail(&format!(
                        "Interface block '{}' differs between vertex and fragment shaders.",
                        block.name
                    ));
                }
            }
            if interface.blocks.iter().any(|(n, _)| *n == block.name) {
                continue;
            }
            let used = block.used || other.blocks.iter().any(|b| b.name == block.name && b.used);
            if !used {
                continue;
            }
            interface.blocks.push((block.name.clone(), binding));
            binding += block.array.unwrap_or(1);
        }
    }
    // Attributes: at the locations bound or laid out, else the lowest
    // free.
    let mut attributes = Vec::new();
    let mut taken = vec![false; options.max_vertex_attribs as usize];
    let mut pending = Vec::new();
    for global in &vertex.unit.globals {
        let GlobalKind::Attribute(layout) = global.kind else {
            continue;
        };
        if !global.used {
            continue;
        }
        let bound = options
            .attribute_locations
            .iter()
            .find(|(n, _)| *n == global.name)
            .map(|(_, l)| *l)
            .or(layout);
        pending.push((global, bound));
    }
    for pass in [true, false] {
        for (global, bound) in &pending {
            if bound.is_some() != pass {
                continue;
            }
            let n = locations(&global.ty);
            let location = match bound {
                Some(l) => *l,
                None => {
                    let found = (0..options.max_vertex_attribs)
                        .find(|&l| (l..l + n).all(|k| taken.get(k as usize) == Some(&false)));
                    match found {
                        Some(l) => l,
                        None => return fail("Too many attributes."),
                    }
                }
            };
            for k in location..location + n {
                match taken.get_mut(k as usize) {
                    Some(slot) if !*slot => *slot = true,
                    Some(_) if pass => {
                        // Bound locations may alias in WebGL 1 (an error
                        // of the draw, not the link); they share then.
                    }
                    Some(_) => return fail("Attribute location aliasing."),
                    None => {
                        return fail(&format!(
                            "Attribute '{}' is bound past the attributes there are.",
                            global.name
                        ));
                    }
                }
            }
            interface.attributes.insert(global.name.clone(), location);
            let element = match &global.ty {
                Type::Matrix(_, r) => Type::Vector(Scalar::Float, *r),
                other => other.clone(),
            };
            attributes.push(Attribute {
                name: global.name.clone(),
                gl_type: gl_type(&global.ty),
                location,
                locations: n,
                scalar: element.scalar().unwrap_or(Scalar::Float),
                components: u32::from(element.size().max(1)),
            });
        }
    }
    attributes.sort_by_key(|a| a.location);
    // Fragment outputs.
    let mut outputs = Vec::new();
    let unit = &fragment.unit;
    let color = unit
        .globals
        .iter()
        .position(|g| g.kind == GlobalKind::Builtin(BuiltinVar::FragColor) && g.used);
    let data = unit
        .globals
        .iter()
        .position(|g| g.kind == GlobalKind::Builtin(BuiltinVar::FragData) && g.used);
    if color.is_some() && data.is_some() {
        return fail("A fragment shader cannot write both gl_FragColor and gl_FragData.");
    }
    if let Some(g) = color {
        interface.outputs.push((g, 0));
        outputs.push(Output {
            location: 0,
            scalar: Scalar::Float,
            components: 4,
        });
    }
    if let Some(g) = data {
        interface.outputs.push((g, 0));
        let Type::Array(_, n) = &unit.globals[g].ty else {
            unreachable!()
        };
        for location in 0..*n {
            outputs.push(Output {
                location,
                scalar: Scalar::Float,
                components: 4,
            });
        }
    }
    let declared: Vec<usize> = unit
        .globals
        .iter()
        .enumerate()
        .filter(|(_, g)| matches!(g.kind, GlobalKind::FragOut(_)))
        .map(|(i, _)| i)
        .collect();
    for &g in &declared {
        let GlobalKind::FragOut(layout) = unit.globals[g].kind else {
            unreachable!()
        };
        let location = match layout {
            Some(l) => l,
            None if declared.len() == 1 => 0,
            None => {
                return fail(
                    "must explicitly specify all locations when using multiple fragment outputs",
                );
            }
        };
        let ty = &unit.globals[g].ty;
        let (element, count) = match ty {
            Type::Array(e, n) => ((**e).clone(), *n),
            other => (other.clone(), 1),
        };
        for k in location..location + count {
            if k >= options.max_draw_buffers || outputs.iter().any(|o: &Output| o.location == k) {
                return fail("Fragment output location is out of range or used twice.");
            }
            outputs.push(Output {
                location: k,
                scalar: element.scalar().unwrap_or(Scalar::Float),
                components: u32::from(element.size().max(1)),
            });
        }
        interface.outputs.push((g, location));
    }
    outputs.sort_by_key(|o| o.location);
    let writes_depth = unit
        .globals
        .iter()
        .any(|g| g.kind == GlobalKind::Builtin(BuiltinVar::FragDepth) && g.used);
    let vertex_wgsl = Emitter::stage(vertex, fragment, &interface, false)?;
    let fragment_wgsl = Emitter::stage(fragment, vertex, &interface, false)?;
    let writes_size = vertex
        .unit
        .globals
        .iter()
        .any(|g| g.kind == GlobalKind::Builtin(BuiltinVar::PointSize) && g.used);
    let points = if writes_size {
        Some(Emitter::stage(vertex, fragment, &interface, true)?)
    } else {
        None
    };
    let module = validate(&vertex_wgsl)?;
    let fragment_module = validate(&fragment_wgsl)?;
    if let Some(points) = &points {
        validate(points)?;
    }
    // The uniforms' places, from naga's layout of the vertex shader's
    // (both stages declare the same).
    let mut program = Program {
        vertex: vertex_wgsl,
        fragment: fragment_wgsl,
        points,
        attributes,
        uniforms: Vec::new(),
        samplers: Vec::new(),
        blocks: Vec::new(),
        uniform_size: 0,
        outputs,
        writes_depth,
    };
    reflect(
        &mut program,
        &module,
        &fragment_module,
        &interface,
        vertex,
        fragment,
    );
    Ok(program)
}

/// Whether two shaders' types are the same (their structs by their
/// names and fields).
fn same_type(a: &Unit, x: &Type, b: &Unit, y: &Type) -> bool {
    match (x, y) {
        (Type::Struct(i), Type::Struct(j)) => {
            let (s, t) = (&a.structs[*i], &b.structs[*j]);
            s.name == t.name
                && s.fields.len() == t.fields.len()
                && s.fields
                    .iter()
                    .zip(&t.fields)
                    .all(|((n, p), (m, q))| n == m && same_type(a, p, b, q))
        }
        (Type::Array(p, n), Type::Array(q, m)) => n == m && same_type(a, p, b, q),
        _ => x == y,
    }
}

/// naga's module of `wgsl`, validated; an error is the translation's, no
/// page's.
fn validate(wgsl: &str) -> Result<naga::Module, String> {
    let module = naga::front::wgsl::parse_str(wgsl).map_err(|e| {
        format!(
            "Internal error: the translation could not be read: {}\n",
            e.emit_to_string(wgsl)
        )
    })?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| {
        format!(
            "Internal error: the translation is not valid: {}\n",
            e.emit_to_string(wgsl)
        )
    })?;
    Ok(module)
}

// ---- The uniforms' reflection ----

fn reflect(
    program: &mut Program,
    module: &naga::Module,
    fragment_module: &naga::Module,
    interface: &Interface,
    vertex: &Shader,
    fragment: &Shader,
) {
    let mut layouter = naga::proc::Layouter::default();
    let _ = layouter.update(module.to_ctx());
    let mut fragment_layouter = naga::proc::Layouter::default();
    let _ = fragment_layouter.update(fragment_module.to_ctx());
    // The default block.
    if let Some(var) = module
        .global_variables
        .iter()
        .find(|(_, v)| v.name.as_deref() == Some("webgl_u"))
        .map(|(_, v)| v)
    {
        program.uniform_size = layouter[var.ty].size;
        if let naga::TypeInner::Struct { members, .. } = &module.types[var.ty].inner {
            for (name, vertex_ty, fragment_ty) in &interface.uniforms {
                let (unit, ty) = match (vertex_ty, fragment_ty) {
                    (Some(t), _) => (&vertex.unit, t),
                    (None, Some(t)) => (&fragment.unit, t),
                    (None, None) => continue,
                };
                let member = format!("u_{name}");
                if let Some(m) = members.iter().find(|m| m.name.as_deref() == Some(&member)) {
                    leaves(program, module, unit, name, ty, m.ty, m.offset, false, None);
                }
            }
        }
    }
    // The samplers.
    for (name, ty, first) in &interface.samplers {
        let (sampler, count) = match ty {
            Type::Sampler(s) => (*s, 1),
            Type::Array(e, n) => match **e {
                Type::Sampler(s) => (s, *n),
                _ => continue,
            },
            _ => continue,
        };
        let element = Type::Sampler(sampler);
        program.samplers.push(SamplerUniform {
            name: if count > 1 || matches!(ty, Type::Array(..)) {
                format!("{name}[0]")
            } else {
                name.clone()
            },
            gl_type: gl_type(&element),
            size: count,
            bindings: (0..count).map(|i| first + 2 * i).collect(),
            sampler,
        });
    }
    // The blocks, each laid out by the module of a stage that declares
    // it.
    for (index, (name, first)) in interface.blocks.iter().enumerate() {
        let (shader, block, module, layouter) = match vertex.blocks.iter().find(|b| b.name == *name)
        {
            Some(b) => (vertex, b, module, &layouter),
            None => (
                fragment,
                fragment
                    .blocks
                    .iter()
                    .find(|b| b.name == *name)
                    .expect("one of them"),
                fragment_module,
                &fragment_layouter,
            ),
        };
        let var_name = format!("webgl_b{index}");
        let Some(var) = module
            .global_variables
            .iter()
            .find(|(_, v)| {
                v.name.as_deref() == Some(&var_name)
                    || v.name.as_deref() == Some(&format!("{var_name}_0"))
            })
            .map(|(_, v)| v)
        else {
            continue;
        };
        let count = block.array.unwrap_or(1);
        program.blocks.push(BlockInfo {
            name: if block.array.is_some() {
                format!("{name}[0]")
            } else {
                name.clone()
            },
            size: layouter[var.ty].size,
            bindings: (0..count).map(|i| first + i).collect(),
            vertex: vertex.blocks.iter().any(|b| b.name == *name && b.used),
            fragment: fragment.blocks.iter().any(|b| b.name == *name && b.used),
        });
        let block_index = program.blocks.len() - 1;
        if let naga::TypeInner::Struct { members, .. } = &module.types[var.ty].inner {
            for member in &block.members {
                let m_name = format!("m_{}", member.name);
                if let Some(m) = members.iter().find(|m| m.name.as_deref() == Some(&m_name)) {
                    let prefix = match &block.instance {
                        Some(_) => format!("{}.{}", block.name, member.name),
                        None => member.name.clone(),
                    };
                    leaves(
                        program,
                        module,
                        &shader.unit,
                        &prefix,
                        &member.ty,
                        m.ty,
                        m.offset,
                        member.row_major,
                        Some(block_index),
                    );
                }
            }
        }
        let _ = shader;
    }
}

/// The leaves of a uniform of `ty` at `offset`, laid out as naga's `nty`
/// says: each basic type or array of them, named as GL names them.
#[allow(clippy::too_many_arguments)]
fn leaves(
    program: &mut Program,
    module: &naga::Module,
    unit: &Unit,
    name: &str,
    ty: &Type,
    nty: naga::Handle<naga::Type>,
    offset: u32,
    row_major: bool,
    block: Option<usize>,
) {
    match ty {
        Type::Struct(s) => {
            let naga::TypeInner::Struct { members, .. } = &module.types[nty].inner else {
                return;
            };
            for ((field, fty), m) in unit.structs[*s].fields.iter().zip(members) {
                leaves(
                    program,
                    module,
                    unit,
                    &format!("{name}.{field}"),
                    fty,
                    m.ty,
                    offset + m.offset,
                    row_major,
                    block,
                );
            }
        }
        Type::Array(element, n) => {
            let naga::TypeInner::Array { base, stride, .. } = module.types[nty].inner else {
                return;
            };
            if matches!(**element, Type::Struct(_) | Type::Array(..)) {
                for i in 0..*n {
                    leaves(
                        program,
                        module,
                        unit,
                        &format!("{name}[{i}]"),
                        element,
                        base,
                        offset + i * stride,
                        row_major,
                        block,
                    );
                }
            } else {
                program.uniforms.push(Uniform {
                    name: format!("{name}[0]"),
                    gl_type: gl_type(element),
                    size: *n,
                    offset,
                    array_stride: stride,
                    matrix_stride: matrix_stride(element, row_major),
                    row_major: row_major && element.is_matrix(),
                    block,
                });
            }
        }
        _ => program.uniforms.push(Uniform {
            name: name.to_owned(),
            gl_type: gl_type(ty),
            size: 1,
            offset,
            array_stride: 0,
            matrix_stride: matrix_stride(ty, row_major),
            row_major: row_major && ty.is_matrix(),
            block,
        }),
    }
}

/// The bytes between a matrix's columns (rows, when row-major): 16, as
/// std140 has them.
fn matrix_stride(ty: &Type, _row_major: bool) -> u32 {
    if ty.is_matrix() { 16 } else { 0 }
}

// ---- The globals functions read and write ----

pub fn globals_in_expr(e: &Expr, used: &mut [bool]) {
    match &e.kind {
        ExprKind::Global(g) => used[*g] = true,
        ExprKind::Lit(_) | ExprKind::Local(_) | ExprKind::Param(_) => {}
        ExprKind::Index(a, b)
        | ExprKind::Binary(_, a, b)
        | ExprKind::And(a, b)
        | ExprKind::Or(a, b)
        | ExprKind::Xor(a, b)
        | ExprKind::Assign(_, a, b)
        | ExprKind::Comma(a, b) => {
            globals_in_expr(a, used);
            globals_in_expr(b, used);
        }
        ExprKind::Field(a, _) | ExprKind::Swizzle(a, _) | ExprKind::Unary(_, a) => {
            globals_in_expr(a, used)
        }
        ExprKind::IncDec { target, .. } => globals_in_expr(target, used),
        ExprKind::Select(a, b, c) => {
            globals_in_expr(a, used);
            globals_in_expr(b, used);
            globals_in_expr(c, used);
        }
        ExprKind::Call(_, args) | ExprKind::Builtin(_, args) | ExprKind::Construct(args) => {
            for a in args {
                globals_in_expr(a, used);
            }
        }
    }
}

pub fn globals_in_stmt(s: &Stmt, used: &mut [bool]) {
    match s {
        Stmt::Expr(e) => globals_in_expr(e, used),
        Stmt::Decl(_, Some(e)) => globals_in_expr(e, used),
        Stmt::Block(b) => b.iter().for_each(|s| globals_in_stmt(s, used)),
        Stmt::If(c, a, b) => {
            globals_in_expr(c, used);
            a.iter().for_each(|s| globals_in_stmt(s, used));
            if let Some(b) = b {
                b.iter().for_each(|s| globals_in_stmt(s, used));
            }
        }
        Stmt::For {
            init,
            cond,
            update,
            body,
        } => {
            init.iter().for_each(|s| globals_in_stmt(s, used));
            if let Some(c) = cond {
                globals_in_expr(c, used);
            }
            if let Some(u) = update {
                globals_in_expr(u, used);
            }
            body.iter().for_each(|s| globals_in_stmt(s, used));
        }
        Stmt::While(c, b) | Stmt::DoWhile(b, c) => {
            globals_in_expr(c, used);
            b.iter().for_each(|s| globals_in_stmt(s, used));
        }
        Stmt::Switch(c, items) => {
            globals_in_expr(c, used);
            for item in items {
                if let SwitchItem::Stmt(s) = item {
                    globals_in_stmt(s, used);
                }
            }
        }
        Stmt::Return(Some(e)) => globals_in_expr(e, used),
        _ => {}
    }
}

// ---- The writer ----

/// A sampler expression's texture and sampler: one binding's, or one of
/// an array's chosen by an index at run time.
enum Pair {
    Static(String, String),
    Dynamic(Vec<(String, String)>, String),
}

/// An assignment's target: a path, and the components of the vector it
/// names when it names some.
struct Lvalue {
    path: String,
    comps: Option<Vec<u8>>,
}

/// What a `continue` inside a switch taken apart into a loop does: sets
/// its flag and breaks out.
struct SwitchFlag {
    continue_flag: String,
    /// The loops opened inside the switch since.
    inner_loops: usize,
}

struct Emitter<'a> {
    shader: &'a Shader,
    unit: &'a Unit,
    /// The other stage's shader, whose structs a uniform only it declares
    /// is laid out by.
    other: &'a Unit,
    /// This stage's tag in its layouts' names, and the other's.
    tag: char,
    /// The uniform layouts' structs, written once each.
    layout_defs: BTreeMap<(char, usize), String>,
    interface: &'a Interface,
    points: bool,
    out: String,
    indent: usize,
    temps: usize,
    helpers: BTreeMap<String, String>,
    function: Option<usize>,
    switches: Vec<SwitchFlag>,
    /// The loops of the function's that enclose what is written.
    loops: usize,
}

type R<T> = Result<T, String>;

fn internal(what: &str) -> String {
    format!("Internal error: {what}\n")
}

impl<'a> Emitter<'a> {
    fn stage(
        shader: &'a Shader,
        other: &'a Shader,
        interface: &'a Interface,
        points: bool,
    ) -> R<String> {
        let tag = if shader.unit.stage == Stage::Vertex {
            'V'
        } else {
            'F'
        };
        let mut e = Emitter {
            shader,
            unit: &shader.unit,
            other: &other.unit,
            tag,
            layout_defs: BTreeMap::new(),
            interface,
            points,
            out: String::new(),
            indent: 0,
            temps: 0,
            helpers: BTreeMap::new(),
            function: None,
            switches: Vec::new(),
            loops: 0,
        };
        e.module()
    }

    fn line(&mut self, text: &str) {
        for _ in 0..self.indent {
            self.out.push_str("    ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn temp(&mut self) -> String {
        self.temps += 1;
        format!("webgl_t{}", self.temps)
    }

    fn helper(&mut self, name: &str, code: impl FnOnce() -> String) -> String {
        if !self.helpers.contains_key(name) {
            let code = code();
            self.helpers.insert(name.to_owned(), code);
        }
        name.to_owned()
    }

    fn global_name(&self, g: usize) -> String {
        let global = &self.unit.globals[g];
        match &global.kind {
            GlobalKind::Private => format!("g{g}_{}", global.name),
            GlobalKind::Const => format!("c{g}_{}", global.name),
            GlobalKind::Attribute(_) => format!("a_{}", global.name),
            GlobalKind::Varying { .. } => format!("v_{}", global.name),
            GlobalKind::FragOut(_) => format!("o_{}", global.name),
            GlobalKind::Uniform => format!("webgl_u.u_{}", global.name),
            GlobalKind::Builtin(b) => format!("webgl_{b:?}"),
        }
    }

    fn local_name(&self, l: usize) -> String {
        let f = &self.unit.functions[self.function.expect("in a function")];
        format!("l{l}_{}", f.locals[l].name)
    }

    fn param_name(&self, p: usize) -> String {
        let f = &self.unit.functions[self.function.expect("in a function")];
        format!("p{p}_{}", f.params[p].name)
    }

    fn function_name(&self, f: usize) -> String {
        format!("f{f}_{}", self.unit.functions[f].name)
    }

    // ---- The module ----

    fn module(&mut self) -> R<String> {
        let unit = self.unit;
        let vertex = unit.stage == Stage::Vertex;
        // The functions first, so that the helpers they need are known.
        let mut functions = String::new();
        for (f, function) in unit.functions.iter().enumerate() {
            if function.body.is_none() {
                continue;
            }
            self.function = Some(f);
            self.out.clear();
            self.temps = 0;
            self.function_text(f)?;
            functions.push_str(&self.out);
            functions.push('\n');
        }
        self.function = None;
        self.out.clear();
        let entry = self.entry()?;
        let mut m = String::new();
        m.push_str("diagnostic(off, derivative_uniformity);\n\n");
        // The structs, and their uniform layouts.
        for (i, s) in unit.structs.iter().enumerate() {
            let fields: Vec<String> = s
                .fields
                .iter()
                .map(|(n, t)| format!("    f_{n}: {},", wgsl_type(t)))
                .collect();
            let _ = writeln!(m, "struct S{i} {{\n{}\n}}", fields.join("\n"));
        }
        // The default block's uniforms.
        let mut members = vec![
            "    webgl_viewport: vec4<f32>,".to_owned(),
            "    webgl_depth: vec4<f32>,".to_owned(),
        ];
        let other_tag = if self.tag == 'V' { 'F' } else { 'V' };
        for (name, vertex_ty, fragment_ty) in &self.interface.uniforms {
            // This stage's declaration, else the other's.
            let (own, theirs) = if self.tag == 'V' {
                (vertex_ty, fragment_ty)
            } else {
                (fragment_ty, vertex_ty)
            };
            let l = match (own, theirs) {
                (Some(t), _) => lay(t, false, self.tag),
                (None, Some(t)) => lay(t, false, other_tag),
                (None, None) => continue,
            };
            let align = if matches!(
                l,
                Lay::Array(..) | Lay::Struct(..) | Lay::Cols(..) | Lay::Rows(..)
            ) {
                "@align(16) "
            } else {
                ""
            };
            members.push(format!("    {align}u_{name}: {},", self.lay_type(&l)));
        }
        let _ = writeln!(m, "struct webgl_Uniforms {{\n{}\n}}", members.join("\n"));
        let _ = writeln!(
            m,
            "@group(0) @binding({UNIFORM_BINDING}) var<uniform> webgl_u: webgl_Uniforms;"
        );
        // The blocks.
        for (index, (name, first)) in self.interface.blocks.iter().enumerate() {
            let block = self.shader.blocks.iter().find(|b| b.name == *name);
            let block = match block {
                Some(b) => b.clone(),
                None => continue,
            };
            let fields: Vec<String> = block
                .members
                .iter()
                .enumerate()
                .map(|(i, member)| {
                    let l = lay(&member.ty, member.row_major, self.tag);
                    let align = if i == 0
                        || matches!(
                            l,
                            Lay::Array(..) | Lay::Struct(..) | Lay::Cols(..) | Lay::Rows(..)
                        ) {
                        "@align(16) "
                    } else {
                        ""
                    };
                    format!("    {align}m_{}: {},", member.name, self.lay_type(&l))
                })
                .collect();
            let _ = writeln!(m, "struct B{index} {{\n{}\n}}", fields.join("\n"));
            match block.array {
                Some(n) => {
                    for k in 0..n {
                        let _ = writeln!(
                            m,
                            "@group(0) @binding({}) var<uniform> webgl_b{index}_{k}: B{index};",
                            first + k
                        );
                    }
                }
                None => {
                    let _ = writeln!(
                        m,
                        "@group(0) @binding({first}) var<uniform> webgl_b{index}: B{index};"
                    );
                }
            }
        }
        // The samplers this shader declares.
        for (name, ty, first) in &self.interface.samplers {
            if !self
                .unit
                .globals
                .iter()
                .any(|g| g.name == *name && g.kind == GlobalKind::Uniform)
            {
                continue;
            }
            let (s, count) = match ty {
                Type::Sampler(s) => (*s, 1),
                Type::Array(e, n) => match **e {
                    Type::Sampler(s) => (s, *n),
                    _ => continue,
                },
                _ => continue,
            };
            for k in 0..count {
                let b = first + 2 * k;
                let sampler = if s.shadow {
                    "sampler_comparison"
                } else {
                    "sampler"
                };
                let _ = writeln!(
                    m,
                    "@group(0) @binding({b}) var webgl_tex{b}: {};",
                    texture_type(&s)
                );
                let _ = writeln!(
                    m,
                    "@group(0) @binding({}) var webgl_smp{b}: {sampler};",
                    b + 1
                );
            }
        }
        // The globals.
        for (g, global) in unit.globals.iter().enumerate() {
            match &global.kind {
                GlobalKind::Const => {
                    if let Some(value) = &global.value
                        && global.used
                        && !global.name.starts_with("gl_")
                    {
                        let _ = writeln!(
                            m,
                            "const {}: {} = {};",
                            self.global_name(g),
                            wgsl_type(&global.ty),
                            constant(unit, &global.ty, value)
                        );
                    }
                }
                GlobalKind::Private
                | GlobalKind::Attribute(_)
                | GlobalKind::Varying { .. }
                | GlobalKind::FragOut(_) => {
                    let _ = writeln!(
                        m,
                        "var<private> {}: {};",
                        self.global_name(g),
                        wgsl_type(&global.ty)
                    );
                }
                GlobalKind::Builtin(b) => {
                    let ty = match b {
                        BuiltinVar::VertexId | BuiltinVar::InstanceId => INT,
                        BuiltinVar::DepthRange => continue,
                        _ => global.ty.clone(),
                    };
                    let _ = writeln!(
                        m,
                        "var<private> {}: {};",
                        self.global_name(g),
                        wgsl_type(&ty)
                    );
                }
                GlobalKind::Uniform => {}
            }
        }
        m.push('\n');
        let defs: Vec<String> = self.layout_defs.values().cloned().collect();
        for def in defs {
            m.push_str(&def);
        }
        // Non-finite constants, which WGSL's constant expressions do not
        // have, made at run time.
        let all = format!(
            "{functions}{entry}{}",
            self.helpers.values().cloned().collect::<String>()
        );
        if all.contains("webgl_nan()") || all.contains("webgl_inf()") {
            self.helper("webgl_bits", || {
                "fn webgl_bits(x: u32) -> u32 {\n    return x;\n}\n".to_owned()
            });
            self.helper("webgl_nan", || {
                "fn webgl_nan() -> f32 {\n    return bitcast<f32>(webgl_bits(0x7fc00000u));\n}\n"
                    .to_owned()
            });
            self.helper("webgl_inf", || {
                "fn webgl_inf() -> f32 {\n    return bitcast<f32>(webgl_bits(0x7f800000u));\n}\n"
                    .to_owned()
            });
        }
        // The helpers, after the declarations they read.
        let helpers: Vec<String> = self.helpers.values().cloned().collect();
        for helper in helpers {
            m.push_str(&helper);
            m.push('\n');
        }
        m.push_str(&functions);
        m.push_str(&entry);
        let _ = vertex;
        Ok(m)
    }

    /// A layout's WGSL type, the struct layouts it needs kept to be
    /// written once.
    fn lay_type(&mut self, l: &Lay) -> String {
        match l {
            Lay::Same(t) => wgsl_type(t),
            Lay::Bool(t) => match t {
                Type::Vector(_, n) => format!("vec{n}<u32>"),
                _ => "u32".to_owned(),
            },
            Lay::Wrapped(inner) => {
                let scalar = match &**inner {
                    Lay::Same(t) => scalar_wgsl(t.scalar().unwrap_or(Scalar::Float)),
                    _ => "u32",
                };
                format!("vec4<{scalar}>")
            }
            Lay::Cols(c, _) => format!("array<vec4<f32>, {c}>"),
            Lay::Rows(_, r) => format!("array<vec4<f32>, {r}>"),
            Lay::Array(e, n) => format!("array<{}, {n}>", self.lay_type(e)),
            Lay::Struct(tag, s) => {
                let name = format!("L{tag}{s}");
                if !self.layout_defs.contains_key(&(*tag, *s)) {
                    // Kept before its fields are, so that it is written
                    // once.
                    self.layout_defs.insert((*tag, *s), String::new());
                    let unit = if *tag == self.tag {
                        self.unit
                    } else {
                        self.other
                    };
                    let fields = unit.structs[*s].fields.clone();
                    let lines: Vec<String> = fields
                        .iter()
                        .enumerate()
                        .map(|(i, (n, t))| {
                            let fl = lay(t, false, *tag);
                            let align = if i == 0
                                || matches!(
                                    fl,
                                    Lay::Array(..)
                                        | Lay::Struct(..)
                                        | Lay::Cols(..)
                                        | Lay::Rows(..)
                                ) {
                                "@align(16) "
                            } else {
                                ""
                            };
                            format!("    {align}f_{n}: {},", self.lay_type(&fl))
                        })
                        .collect();
                    self.layout_defs.insert(
                        (*tag, *s),
                        format!("struct {name} {{\n{}\n}}\n", lines.join("\n")),
                    );
                }
                name
            }
        }
    }

    /// The value at `code`, kept as `l` lays it out, as its GLSL type.
    fn lay_value(&mut self, code: &str, l: &Lay) -> String {
        match l {
            Lay::Same(_) => code.to_owned(),
            Lay::Bool(t) => format!("{}({code})", wgsl_type(t)),
            Lay::Wrapped(inner) => {
                let n = match &**inner {
                    Lay::Same(t) | Lay::Bool(t) => t.size(),
                    _ => 4,
                };
                let part = format!("{code}{}", first(n));
                self.lay_value(&part, inner)
            }
            Lay::Cols(c, r) => {
                let cols: Vec<String> = (0..*c)
                    .map(|i| format!("{code}[{i}]{}", first(*r)))
                    .collect();
                format!("mat{c}x{r}<f32>({})", cols.join(", "))
            }
            Lay::Rows(c, r) => {
                let rows: Vec<String> = (0..*r)
                    .map(|i| format!("{code}[{i}]{}", first(*c)))
                    .collect();
                format!("transpose(mat{r}x{c}<f32>({}))", rows.join(", "))
            }
            Lay::Array(e, n) => {
                if lay_is_same(e) {
                    return code.to_owned();
                }
                let from = format!("array<{}, {n}>", self.lay_type(e));
                let element_ty = self.lay_value_type(e);
                let to = format!("array<{element_ty}, {n}>");
                let name = format!("webgl_ua_{}", sanitize(&from));
                let read = self.lay_value("a[i]", e);
                self.helper(&name, || {
                    format!("fn {name}(a: {from}) -> {to} {{\n    var r: {to};\n    for (var i = 0u; i < {n}u; i++) {{\n        r[i] = {read};\n    }}\n    return r;\n}}\n")
                });
                format!("{name}({code})")
            }
            Lay::Struct(tag, s) => {
                let name = format!("webgl_us{s}");
                let fields = self.unit.structs[*s].fields.clone();
                let parts: Vec<String> = fields
                    .iter()
                    .map(|(n, t)| self.lay_value(&format!("s.f_{n}"), &lay(t, false, *tag)))
                    .collect();
                self.helper(&name, || {
                    format!(
                        "fn {name}(s: L{tag}{s}) -> S{s} {{\n    return S{s}({});\n}}\n",
                        parts.join(", ")
                    )
                });
                format!("{name}({code})")
            }
        }
    }

    /// The GLSL type a layout keeps, in WGSL.
    fn lay_value_type(&self, l: &Lay) -> String {
        match l {
            Lay::Same(t) | Lay::Bool(t) => wgsl_type(t),
            Lay::Wrapped(inner) => self.lay_value_type(inner),
            Lay::Cols(c, r) | Lay::Rows(c, r) => format!("mat{c}x{r}<f32>"),
            Lay::Array(e, n) => format!("array<{}, {n}>", self.lay_value_type(e)),
            Lay::Struct(_, s) => format!("S{s}"),
        }
    }

    // ---- Functions ----

    fn function_text(&mut self, f: usize) -> R<()> {
        let function = &self.unit.functions[f];
        let mut params = Vec::new();
        for (i, p) in function.params.iter().enumerate() {
            let name = self.param_name(i);
            match (&p.ty, p.mode) {
                (Type::Sampler(s), _) => {
                    params.push(format!("{name}_tex: {}", texture_type(s)));
                    params.push(format!(
                        "{name}_smp: {}",
                        if s.shadow {
                            "sampler_comparison"
                        } else {
                            "sampler"
                        }
                    ));
                }
                (ty, ParamMode::In) => params.push(format!("{name}_in: {}", wgsl_type(ty))),
                (ty, _) => params.push(format!("{name}_ptr: ptr<function, {}>", wgsl_type(ty))),
            }
        }
        let ret = if function.ret == Type::Void {
            String::new()
        } else {
            format!(" -> {}", wgsl_type(&function.ret))
        };
        self.line(&format!(
            "fn {}({}){ret} {{",
            self.function_name(f),
            params.join(", ")
        ));
        self.indent += 1;
        // The parameters as variables: GLSL's are copies a body may
        // change; out and inout copied back at each return.
        for (i, p) in function.params.iter().enumerate() {
            let name = self.param_name(i);
            match (&p.ty, p.mode) {
                (Type::Sampler(_), _) => {}
                (ty, ParamMode::In) => {
                    self.line(&format!("var {name}: {} = {name}_in;", wgsl_type(ty)))
                }
                (ty, ParamMode::InOut) => {
                    self.line(&format!("var {name}: {} = *{name}_ptr;", wgsl_type(ty)))
                }
                (ty, ParamMode::Out) => self.line(&format!("var {name}: {};", wgsl_type(ty))),
            }
        }
        let body = function.body.clone().unwrap_or_default();
        for stmt in &body {
            self.stmt(stmt)?;
        }
        self.copy_back();
        if function.ret != Type::Void {
            self.line(&format!("return {}();", wgsl_type(&function.ret)));
        }
        self.indent -= 1;
        self.line("}");
        Ok(())
    }

    /// The out and inout parameters copied back, before a return.
    fn copy_back(&mut self) {
        let f = self.function.expect("in a function");
        let params = self.unit.functions[f].params.clone();
        for (i, p) in params.iter().enumerate() {
            if p.mode != ParamMode::In && !matches!(p.ty, Type::Sampler(_)) {
                let name = self.param_name(i);
                self.line(&format!("*{name}_ptr = {name};"));
            }
        }
    }

    // ---- Statements ----

    fn stmts(&mut self, stmts: &[Stmt]) -> R<()> {
        for s in stmts {
            self.stmt(s)?;
        }
        Ok(())
    }

    fn stmt(&mut self, s: &Stmt) -> R<()> {
        match s {
            Stmt::Expr(e) => self.effect(e)?,
            Stmt::Decl(l, init) => {
                let f = self.function.expect("in a function");
                let local = &self.unit.functions[f].locals[*l];
                let name = self.local_name(*l);
                let ty = wgsl_type(&local.ty);
                if let Some(value) = &local.value {
                    let text = constant(self.unit, &local.ty, value);
                    self.line(&format!("let {name}: {ty} = {text};"));
                    // GLSL's const locals are values; a `let` is enough.
                } else if let Some(init) = init {
                    let value = self.expr(init)?;
                    self.line(&format!("var {name}: {ty} = {value};"));
                } else {
                    self.line(&format!("var {name}: {ty};"));
                }
            }
            Stmt::Block(b) => {
                self.line("{");
                self.indent += 1;
                self.stmts(b)?;
                self.indent -= 1;
                self.line("}");
            }
            Stmt::If(c, a, b) => {
                let cond = self.expr(c)?;
                self.line(&format!("if ({cond}) {{"));
                self.indent += 1;
                self.stmts(a)?;
                self.indent -= 1;
                match b {
                    Some(b) => {
                        self.line("} else {");
                        self.indent += 1;
                        self.stmts(b)?;
                        self.indent -= 1;
                        self.line("}");
                    }
                    None => self.line("}"),
                }
            }
            Stmt::For {
                init,
                cond,
                update,
                body,
            } => {
                self.line("{");
                self.indent += 1;
                self.stmts(init)?;
                self.line("loop {");
                self.indent += 1;
                self.enter_loop();
                if let Some(c) = cond {
                    let cond = self.expr(c)?;
                    self.line(&format!("if (!({cond})) {{ break; }}"));
                }
                self.stmts(body)?;
                if let Some(u) = update {
                    self.line("continuing {");
                    self.indent += 1;
                    self.effect(u)?;
                    self.indent -= 1;
                    self.line("}");
                }
                self.leave_loop();
                self.indent -= 1;
                self.line("}");
                self.indent -= 1;
                self.line("}");
            }
            Stmt::While(c, body) => {
                self.line("loop {");
                self.indent += 1;
                self.enter_loop();
                let cond = self.expr(c)?;
                self.line(&format!("if (!({cond})) {{ break; }}"));
                self.stmts(body)?;
                self.leave_loop();
                self.indent -= 1;
                self.line("}");
            }
            Stmt::DoWhile(body, c) => {
                self.line("loop {");
                self.indent += 1;
                self.enter_loop();
                self.stmts(body)?;
                self.line("continuing {");
                self.indent += 1;
                let cond = self.expr(c)?;
                self.line(&format!("break if !({cond});"));
                self.indent -= 1;
                self.line("}");
                self.leave_loop();
                self.indent -= 1;
                self.line("}");
            }
            Stmt::Switch(selector, items) => self.switch(selector, items)?,
            Stmt::Break => self.line("break;"),
            Stmt::Continue => {
                if let Some(flag) = self.switches.last()
                    && flag.inner_loops == 0
                {
                    let name = flag.continue_flag.clone();
                    self.line(&format!("{name} = true;"));
                    self.line("break;");
                } else {
                    self.line("continue;");
                }
            }
            Stmt::Return(value) => {
                let f = self.function.expect("in a function");
                match value {
                    Some(v) => {
                        let text = self.expr(v)?;
                        let has_out = self.unit.functions[f]
                            .params
                            .iter()
                            .any(|p| p.mode != ParamMode::In);
                        if has_out {
                            let t = self.temp();
                            self.line(&format!("let {t} = {text};"));
                            self.copy_back();
                            self.line(&format!("return {t};"));
                        } else {
                            self.line(&format!("return {text};"));
                        }
                    }
                    None => {
                        self.copy_back();
                        self.line("return;");
                    }
                }
            }
            Stmt::Discard => self.line("discard;"),
        }
        Ok(())
    }

    fn enter_loop(&mut self) {
        self.loops += 1;
        if let Some(flag) = self.switches.last_mut() {
            flag.inner_loops += 1;
        }
    }

    fn leave_loop(&mut self) {
        self.loops -= 1;
        if let Some(flag) = self.switches.last_mut() {
            flag.inner_loops -= 1;
        }
    }

    /// A switch: WGSL's own when no case falls through to the next,
    /// else a loop of conditions that keep falling.
    fn switch(&mut self, selector: &Expr, items: &[SwitchItem]) -> R<()> {
        // The groups: their labels and statements.
        let mut groups: Vec<(Vec<Option<i64>>, Vec<Stmt>)> = Vec::new();
        for item in items {
            match item {
                SwitchItem::Case(v) => {
                    if groups.last().is_none_or(|(_, s)| !s.is_empty()) {
                        groups.push((Vec::new(), Vec::new()));
                    }
                    groups.last_mut().expect("one").0.push(Some(*v));
                }
                SwitchItem::Default => {
                    if groups.last().is_none_or(|(_, s)| !s.is_empty()) {
                        groups.push((Vec::new(), Vec::new()));
                    }
                    groups.last_mut().expect("one").0.push(None);
                }
                SwitchItem::Stmt(s) => groups
                    .last_mut()
                    .expect("a label first")
                    .1
                    .push((**s).clone()),
            }
        }
        let ends = |stmts: &[Stmt]| {
            matches!(
                stmts.last(),
                Some(Stmt::Break | Stmt::Return(_) | Stmt::Continue | Stmt::Discard)
            )
        };
        let falls = groups
            .iter()
            .take(groups.len().saturating_sub(1))
            .any(|(_, s)| !ends(s));
        let sel = self.expr(selector)?;
        let int = selector.ty == INT;
        let lit = |v: i64| {
            if int {
                format!("{}i", v as i32)
            } else {
                format!("{}u", v as u32)
            }
        };
        if !falls {
            self.line(&format!("switch ({sel}) {{"));
            self.indent += 1;
            let mut defaulted = false;
            for (labels, stmts) in &groups {
                let mut parts: Vec<String> = labels
                    .iter()
                    .map(|l| l.map_or("default".to_owned(), lit))
                    .collect();
                if labels.contains(&None) {
                    defaulted = true;
                }
                parts.dedup();
                self.line(&format!("case {}: {{", parts.join(", ")));
                self.indent += 1;
                // WGSL's cases end on their own: a final break is not
                // needed, and inside a loop it would break the switch
                // alone either way.
                let body: &[Stmt] = match stmts.last() {
                    Some(Stmt::Break) => &stmts[..stmts.len() - 1],
                    _ => stmts,
                };
                self.stmts(body)?;
                self.indent -= 1;
                self.line("}");
            }
            if !defaulted {
                self.line("default: {}");
            }
            self.indent -= 1;
            self.line("}");
            return Ok(());
        }
        let selector_name = self.temp();
        let fall = self.temp();
        let hit = self.temp();
        let cont = self.temp();
        self.line(&format!("let {selector_name} = {sel};"));
        self.line(&format!("var {fall} = false;"));
        self.line(&format!("var {cont} = false;"));
        let all: Vec<String> = groups
            .iter()
            .flat_map(|(labels, _)| {
                labels
                    .iter()
                    .flatten()
                    .map(|v| format!("{selector_name} == {}", lit(*v)))
            })
            .collect();
        self.line(&format!(
            "let {hit} = {};",
            if all.is_empty() {
                "false".to_owned()
            } else {
                all.join(" || ")
            }
        ));
        self.line("loop {");
        self.indent += 1;
        self.switches.push(SwitchFlag {
            continue_flag: cont.clone(),
            inner_loops: 0,
        });
        for (labels, stmts) in &groups {
            let mut conds = vec![fall.clone()];
            for l in labels {
                conds.push(match l {
                    Some(v) => format!("{selector_name} == {}", lit(*v)),
                    None => format!("!{hit}"),
                });
            }
            self.line(&format!("if ({}) {{", conds.join(" || ")));
            self.indent += 1;
            self.line(&format!("{fall} = true;"));
            self.stmts(stmts)?;
            self.indent -= 1;
            self.line("}");
        }
        self.switches.pop();
        self.line("break;");
        self.indent -= 1;
        self.line("}");
        // A `continue` it took on to the loop around it, if any.
        if self.switches.last().is_some_and(|f| f.inner_loops == 0) {
            let outer = self.switches.last().expect("one").continue_flag.clone();
            self.line(&format!("if ({cont}) {{ {outer} = true; break; }}"));
        } else if self.loops > 0 {
            self.line(&format!("if ({cont}) {{ continue; }}"));
        }
        Ok(())
    }

    /// An expression evaluated for its effects alone.
    fn effect(&mut self, e: &Expr) -> R<()> {
        match &e.kind {
            ExprKind::Assign(op, target, value) => self.assign(*op, target, value).map(|_| ()),
            ExprKind::IncDec { target, inc, .. } => {
                let lv = self.lvalue(target)?;
                let one = self.one(&target.ty);
                let op = if *inc { "+" } else { "-" };
                let read = self.lvalue_read(&lv);
                let updated = self.arith(
                    op,
                    &read,
                    &target.ty,
                    &one,
                    &scalar_type(&target.ty),
                    &target.ty,
                )?;
                self.store(&lv, &updated);
                Ok(())
            }
            ExprKind::Comma(a, b) => {
                self.effect(a)?;
                self.effect(b)
            }
            ExprKind::Call(f, _) if self.unit.functions[*f].ret == Type::Void => {
                let text = self.expr(e)?;
                self.line(&format!("{text};"));
                Ok(())
            }
            _ if has_effects(e) => {
                let text = self.expr(e)?;
                if e.ty == Type::Void {
                    self.line(&format!("{text};"));
                } else {
                    self.line(&format!("_ = {text};"));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    // ---- Expressions ----

    /// Whether WGSL may read the text of `e` more than once at no cost.
    fn simple(&self, e: &Expr) -> bool {
        matches!(
            e.kind,
            ExprKind::Lit(_) | ExprKind::Global(_) | ExprKind::Local(_) | ExprKind::Param(_)
        )
    }

    /// The operands' texts in GLSL's order: one before an operand with
    /// effects is bound to a `let` first.
    fn operands(&mut self, args: &[&Expr]) -> R<Vec<String>> {
        let mut out = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            let later = args[i + 1..].iter().any(|b| has_effects(b));
            let text = self.expr(a)?;
            if later && !matches!(a.kind, ExprKind::Lit(_)) {
                let t = self.temp();
                self.line(&format!("let {t} = {text};"));
                out.push(t);
            } else {
                out.push(text);
            }
        }
        Ok(out)
    }

    fn expr(&mut self, e: &Expr) -> R<String> {
        // A constant expression is its value (but samplers').
        if let Some(value) = &e.value
            && !matches!(e.ty, Type::Sampler(_) | Type::Void)
            && !has_effects(e)
            && value
                .iter()
                .all(|c| !matches!(c, Comp::F(v) if !v.is_finite()))
        {
            return Ok(constant(self.unit, &e.ty, value));
        }
        if let Some((path, l)) = self.uniform_path(e)? {
            return Ok(self.lay_value(&path, &l));
        }
        Ok(match &e.kind {
            ExprKind::Lit(c) => scalar_literal(*c),
            ExprKind::Global(g) => self.global_value(*g),
            ExprKind::Local(l) => self.local_name(*l),
            ExprKind::Param(p) => self.param_name(*p),
            ExprKind::Index(base, index) => {
                let parts = self.operands(&[base, index])?;
                let mut b = parts[0].clone();
                // An array that is a value (not a variable) is copied to
                // one before a dynamic index.
                if matches!(base.ty, Type::Array(..))
                    && !self.is_reference(base)
                    && index.value.is_none()
                {
                    let t = self.temp();
                    self.line(&format!("var {t} = {b};"));
                    b = t;
                }
                format!("{b}[{}]", parts[1])
            }
            ExprKind::Field(base, i) => {
                let b = self.expr(base)?;
                let Type::Struct(s) = base.ty else {
                    return Err(internal("a field of no struct"));
                };
                format!("{b}.f_{}", self.unit.structs[s].fields[*i].0)
            }
            ExprKind::Swizzle(base, comps) => {
                let b = self.expr(base)?;
                if base.ty.is_scalar() {
                    // A scalar's swizzle: a splat of it.
                    let b = if self.simple(base) {
                        b
                    } else {
                        let t = self.temp();
                        self.line(&format!("let {t} = {b};"));
                        t
                    };
                    if comps.len() == 1 {
                        b
                    } else {
                        format!("{}({b})", wgsl_type(&e.ty))
                    }
                } else {
                    format!("({b}).{}", swizzle_text(comps))
                }
            }
            ExprKind::Unary(op, a) => {
                let t = self.expr(a)?;
                match op {
                    UnOp::Neg if a.ty.is_matrix() => format!("(({t}) * -1.0f)"),
                    // WGSL negates no unsigned integer: GLSL's wraps, as
                    // a subtraction from zero does.
                    UnOp::Neg if a.ty.scalar() == Some(Scalar::Uint) => format!("(0u - ({t}))"),
                    UnOp::Neg => format!("(-({t}))"),
                    UnOp::Not => format!("(!({t}))"),
                    UnOp::BitNot => format!("(~({t}))"),
                }
            }
            ExprKind::Binary(op, a, b) => {
                let parts = self.operands(&[a, b])?;
                self.binary(*op, &parts[0], &a.ty, &parts[1], &b.ty, &e.ty)?
            }
            ExprKind::And(a, b) | ExprKind::Or(a, b) => {
                let and = matches!(e.kind, ExprKind::And(..));
                if has_effects(b) {
                    let left = self.expr(a)?;
                    let t = self.temp();
                    self.line(&format!("var {t} = {left};"));
                    self.line(&format!("if ({}{t}) {{", if and { "" } else { "!" }));
                    self.indent += 1;
                    let right = self.expr(b)?;
                    self.line(&format!("{t} = {right};"));
                    self.indent -= 1;
                    self.line("}");
                    t
                } else {
                    let parts = self.operands(&[a, b])?;
                    format!(
                        "({} {} {})",
                        parts[0],
                        if and { "&&" } else { "||" },
                        parts[1]
                    )
                }
            }
            ExprKind::Xor(a, b) => {
                let parts = self.operands(&[a, b])?;
                format!("({} != {})", parts[0], parts[1])
            }
            ExprKind::Select(c, a, b) => {
                let selectable = e.ty.is_scalar_or_vector();
                if !selectable || has_effects(a) || has_effects(b) {
                    let cond = self.expr(c)?;
                    let t = self.temp();
                    self.line(&format!("var {t}: {};", wgsl_type(&e.ty)));
                    self.line(&format!("if ({cond}) {{"));
                    self.indent += 1;
                    let x = self.expr(a)?;
                    self.line(&format!("{t} = {x};"));
                    self.indent -= 1;
                    self.line("} else {");
                    self.indent += 1;
                    let y = self.expr(b)?;
                    self.line(&format!("{t} = {y};"));
                    self.indent -= 1;
                    self.line("}");
                    t
                } else {
                    let parts = self.operands(&[c, a, b])?;
                    format!("select({}, {}, {})", parts[2], parts[1], parts[0])
                }
            }
            ExprKind::Assign(op, target, value) => self.assign(*op, target, value)?,
            ExprKind::IncDec { pre, inc, target } => {
                let lv = self.lvalue(target)?;
                let read = self.lvalue_read(&lv);
                let one = self.one(&target.ty);
                let op = if *inc { "+" } else { "-" };
                let old = self.temp();
                self.line(&format!("let {old} = {read};"));
                let updated = self.arith(
                    op,
                    &old,
                    &target.ty,
                    &one,
                    &scalar_type(&target.ty),
                    &target.ty,
                )?;
                self.store(&lv, &updated);
                if *pre { self.lvalue_read(&lv) } else { old }
            }
            ExprKind::Call(f, args) => self.call(*f, args)?,
            ExprKind::Builtin(name, args) => self.builtin(name, args, &e.ty)?,
            ExprKind::Construct(args) => self.construct(&e.ty, args)?,
            ExprKind::Comma(a, b) => {
                self.effect(a)?;
                self.expr(b)?
            }
        })
    }

    /// Whether an expression names memory (a variable, or a part of one),
    /// which WGSL indexes by any value.
    fn is_reference(&self, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Global(g) => !matches!(
                self.unit.globals[*g].kind,
                GlobalKind::Const | GlobalKind::Builtin(BuiltinVar::DepthRange)
            ),
            ExprKind::Local(l) => {
                let f = self.function.expect("in a function");
                self.unit.functions[f].locals[*l].value.is_none()
            }
            ExprKind::Param(_) => true,
            ExprKind::Index(b, _) | ExprKind::Field(b, _) => self.is_reference(b),
            _ => false,
        }
    }

    fn global_value(&mut self, g: usize) -> String {
        let global = &self.unit.globals[g];
        match global.kind {
            GlobalKind::Builtin(BuiltinVar::DepthRange) => {
                let Type::Struct(s) = global.ty else {
                    return String::new();
                };
                format!("S{s}(webgl_u.webgl_depth.x, webgl_u.webgl_depth.y, webgl_u.webgl_depth.z)")
            }
            _ => self.global_name(g),
        }
    }

    /// A uniform's path in its buffer, and its layout there: for the
    /// default block's uniforms and the blocks' members, and their
    /// elements and fields.
    fn uniform_path(&mut self, e: &Expr) -> R<Option<(String, Lay)>> {
        match &e.kind {
            ExprKind::Global(g) => {
                let global = &self.unit.globals[*g];
                if global.kind != GlobalKind::Uniform
                    || global.ty.has_sampler(self.unit)
                    || global.name.starts_with("webgl_block")
                {
                    return Ok(None);
                }
                Ok(Some((
                    format!("webgl_u.u_{}", global.name),
                    lay(&global.ty, false, self.tag),
                )))
            }
            ExprKind::Field(base, i) => {
                // A block's member.
                if let Some((block, path)) = self.block_path(base)? {
                    let member = &block.members[*i];
                    return Ok(Some((
                        format!("{path}.m_{}", member.name),
                        lay(&member.ty, member.row_major, self.tag),
                    )));
                }
                let Some((path, l)) = self.uniform_path(base)? else {
                    return Ok(None);
                };
                let Lay::Struct(_, s) = l else {
                    return Ok(None);
                };
                let (name, ty) = self.unit.structs[s].fields[*i].clone();
                Ok(Some((
                    format!("{path}.f_{name}"),
                    lay(&ty, false, self.tag),
                )))
            }
            ExprKind::Index(base, index) => {
                let Some((path, l)) = self.uniform_path(base)? else {
                    return Ok(None);
                };
                let Lay::Array(element, _) = l else {
                    return Ok(None);
                };
                let i = self.expr(index)?;
                let i = if has_effects(index) {
                    let t = self.temp();
                    self.line(&format!("let {t} = {i};"));
                    t
                } else {
                    i
                };
                Ok(Some((format!("{path}[{i}]"), *element)))
            }
            _ => Ok(None),
        }
    }

    /// A block's global, or one element of a block's array: the block and
    /// its variable's name.
    fn block_path(&mut self, e: &Expr) -> R<Option<(Block, String)>> {
        let (g, index) = match &e.kind {
            ExprKind::Global(g) => (*g, None),
            ExprKind::Index(base, index) => match base.kind {
                ExprKind::Global(g) => (g, Some(index)),
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        let Some(block) = self.shader.blocks.iter().find(|b| b.global == g) else {
            return Ok(None);
        };
        let element = match index {
            None => None,
            Some(index) => match &index.value {
                Some(v) => Some(v[0].as_i32()),
                None => return Err(internal("a block array indexed by a value not constant")),
            },
        };
        let Some(index) = self
            .interface
            .blocks
            .iter()
            .position(|(n, _)| *n == block.name)
        else {
            return Err(internal("a block the link did not keep"));
        };
        let path = match element {
            Some(k) => format!("webgl_b{index}_{k}"),
            None => format!("webgl_b{index}"),
        };
        Ok(Some((block.clone(), path)))
    }

    /// `a op b`, by GLSL's rules, in WGSL.
    fn binary(
        &mut self,
        op: BinOp,
        a: &str,
        at: &Type,
        b: &str,
        bt: &Type,
        ty: &Type,
    ) -> R<String> {
        Ok(match op {
            BinOp::Eq | BinOp::Ne => {
                let eq = self.equal(a, b, at)?;
                if op == BinOp::Eq {
                    eq
                } else {
                    format!("(!{eq})")
                }
            }
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                format!("({a} {} {b})", crate::expr::op_text(op))
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                self.arith(crate::expr::op_text(op), a, at, b, bt, ty)?
            }
            BinOp::Rem => {
                let (a, b) = self.splat(a, at, b, bt);
                format!("({a} % {b})")
            }
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                let (a, b) = self.splat(a, at, b, bt);
                format!("({a} {} {b})", crate::expr::op_text(op))
            }
            BinOp::Shl | BinOp::Shr => {
                let amount = match (at, bt) {
                    (Type::Vector(_, n), Type::Scalar(_)) => format!("vec{n}<u32>(u32({b}))"),
                    (Type::Vector(_, n), Type::Vector(..)) => format!("vec{n}<u32>({b})"),
                    _ => format!("u32({b})"),
                };
                format!("({a} {} {amount})", crate::expr::op_text(op))
            }
        })
    }

    /// A scalar operand beside a vector, made the vector's type.
    fn splat(&self, a: &str, at: &Type, b: &str, bt: &Type) -> (String, String) {
        match (at, bt) {
            (Type::Vector(..), Type::Scalar(_)) => {
                (a.to_owned(), format!("{}({b})", wgsl_type(at)))
            }
            (Type::Scalar(_), Type::Vector(..)) => {
                (format!("{}({a})", wgsl_type(bt)), b.to_owned())
            }
            _ => (a.to_owned(), b.to_owned()),
        }
    }

    /// Arithmetic: WGSL's where it has it, else a matrix's columns in
    /// turn.
    fn arith(&mut self, op: &str, a: &str, at: &Type, b: &str, bt: &Type, ty: &Type) -> R<String> {
        let linear = op == "*"
            && (at.is_matrix() && (bt.is_matrix() || bt.is_vector())
                || at.is_vector() && bt.is_matrix());
        let native = linear
            || (!at.is_matrix() && !bt.is_matrix())
            || (op == "*" && (at.is_scalar() || bt.is_scalar()))
            || ((op == "+" || op == "-") && at.is_matrix() && bt.is_matrix());
        if native {
            return Ok(format!("({a} {op} {b})"));
        }
        // Component-wise on a matrix: column by column.
        let Type::Matrix(c, r) = ty else {
            return Err(internal("matrix arithmetic of no matrix"));
        };
        let a = self.let_of(a);
        let b = self.let_of(b);
        let col = |x: &str, t: &Type, i: u8| {
            if t.is_matrix() {
                format!("{x}[{i}]")
            } else {
                x.to_owned()
            }
        };
        let cols: Vec<String> = (0..*c)
            .map(|i| format!("({} {op} {})", col(&a, at, i), col(&b, bt, i)))
            .collect();
        Ok(format!("mat{c}x{r}<f32>({})", cols.join(", ")))
    }

    /// `text` bound to a `let` when it is not a name already.
    fn let_of(&mut self, text: &str) -> String {
        if text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        {
            return text.to_owned();
        }
        let t = self.temp();
        self.line(&format!("let {t} = {text};"));
        t
    }

    /// `a == b` for a type of any kind.
    fn equal(&mut self, a: &str, b: &str, ty: &Type) -> R<String> {
        Ok(match ty {
            Type::Scalar(_) => format!("({a} == {b})"),
            Type::Vector(..) => format!("all({a} == {b})"),
            Type::Matrix(c, _) => {
                let a = self.let_of(a);
                let b = self.let_of(b);
                let parts: Vec<String> = (0..*c)
                    .map(|i| format!("all({a}[{i}] == {b}[{i}])"))
                    .collect();
                format!("({})", parts.join(" && "))
            }
            Type::Array(element, n) => {
                let name = format!("webgl_eq_{}", sanitize(&wgsl_type(ty)));
                let element_eq = self.equal("x[i]", "y[i]", element)?;
                let t = wgsl_type(ty);
                self.helper(&name, || {
                    format!("fn {name}(x_in: {t}, y_in: {t}) -> bool {{\n    var x = x_in;\n    var y = y_in;\n    for (var i = 0u; i < {n}u; i++) {{\n        if (!{element_eq}) {{ return false; }}\n    }}\n    return true;\n}}\n")
                });
                format!("{name}({a}, {b})")
            }
            Type::Struct(s) => {
                let name = format!("webgl_eq_S{s}");
                let fields = self.unit.structs[*s].fields.clone();
                let mut parts = Vec::new();
                for (n, t) in &fields {
                    parts.push(self.equal(&format!("x.f_{n}"), &format!("y.f_{n}"), t)?);
                }
                let t = wgsl_type(ty);
                self.helper(&name, || {
                    format!(
                        "fn {name}(x: {t}, y: {t}) -> bool {{\n    return {};\n}}\n",
                        parts.join(" && ")
                    )
                });
                format!("{name}({a}, {b})")
            }
            _ => return Err(internal("equality of this type")),
        })
    }

    /// A one of a type's scalar, for `++` and `--`.
    fn one(&self, ty: &Type) -> String {
        match ty.scalar() {
            Some(Scalar::Int) => "1i".to_owned(),
            Some(Scalar::Uint) => "1u".to_owned(),
            _ => "1.0f".to_owned(),
        }
    }

    // ---- Assignment ----

    fn lvalue(&mut self, e: &Expr) -> R<Lvalue> {
        Ok(match &e.kind {
            ExprKind::Global(g) => Lvalue {
                path: self.global_name(*g),
                comps: None,
            },
            ExprKind::Local(l) => Lvalue {
                path: self.local_name(*l),
                comps: None,
            },
            ExprKind::Param(p) => Lvalue {
                path: self.param_name(*p),
                comps: None,
            },
            ExprKind::Index(base, index) => {
                let lv = self.lvalue(base)?;
                let i = if index.value.is_some() || self.simple(index) {
                    self.expr(index)?
                } else {
                    let text = self.expr(index)?;
                    let t = self.temp();
                    self.line(&format!("let {t} = {text};"));
                    t
                };
                match lv.comps {
                    Some(comps) => {
                        let k = index
                            .value
                            .as_ref()
                            .map(|v| v[0].as_i32() as usize)
                            .ok_or_else(|| internal("a swizzle indexed by a value not constant"))?;
                        Lvalue {
                            path: lv.path,
                            comps: Some(vec![comps[k]]),
                        }
                    }
                    None => Lvalue {
                        path: format!("{}[{i}]", lv.path),
                        comps: None,
                    },
                }
            }
            ExprKind::Field(base, i) => {
                let lv = self.lvalue(base)?;
                let Type::Struct(s) = base.ty else {
                    return Err(internal("a field of no struct"));
                };
                Lvalue {
                    path: format!("{}.f_{}", lv.path, self.unit.structs[s].fields[*i].0),
                    comps: None,
                }
            }
            ExprKind::Swizzle(base, comps) => {
                let lv = self.lvalue(base)?;
                let comps = match lv.comps {
                    Some(outer) => comps.iter().map(|&c| outer[c as usize]).collect(),
                    None => comps.clone(),
                };
                Lvalue {
                    path: lv.path,
                    comps: Some(comps),
                }
            }
            _ => return Err(internal("an assignment to no l-value")),
        })
    }

    fn lvalue_read(&self, lv: &Lvalue) -> String {
        match &lv.comps {
            Some(c) => format!("{}.{}", lv.path, swizzle_text(c)),
            None => lv.path.clone(),
        }
    }

    /// `value` stored at `lv`: component by component for a swizzle of
    /// more than one.
    fn store(&mut self, lv: &Lvalue, value: &str) {
        match &lv.comps {
            None => self.line(&format!("{} = {value};", lv.path)),
            Some(c) if c.len() == 1 => {
                self.line(&format!("{}.{} = {value};", lv.path, XYZW[c[0] as usize]))
            }
            Some(c) => {
                let t = self.temp();
                self.line(&format!("let {t} = {value};"));
                for (i, &k) in c.iter().enumerate() {
                    self.line(&format!(
                        "{}.{} = {t}.{};",
                        lv.path, XYZW[k as usize], XYZW[i]
                    ));
                }
            }
        }
    }

    fn assign(&mut self, op: Option<BinOp>, target: &Expr, value: &Expr) -> R<String> {
        // GLSL evaluates the right side, then the left's place.
        let rhs = self.expr(value)?;
        let rhs = if has_effects(target) || !self.simple(value) {
            let t = self.temp();
            self.line(&format!("let {t} = {rhs};"));
            t
        } else {
            rhs
        };
        let lv = self.lvalue(target)?;
        let stored = match op {
            None => rhs,
            Some(op) => {
                let read = self.lvalue_read(&lv);
                self.binary(op, &read, &target.ty, &rhs, &value.ty, &target.ty)?
            }
        };
        self.store(&lv, &stored);
        Ok(self.lvalue_read(&lv))
    }

    // ---- Calls ----

    fn call(&mut self, f: usize, args: &[Expr]) -> R<String> {
        let function = &self.unit.functions[f];
        let params = function.params.clone();
        let mut texts = Vec::new();
        let mut copies: Vec<(Lvalue, String)> = Vec::new();
        for (i, (param, arg)) in params.iter().zip(args).enumerate() {
            let later = args[i + 1..].iter().any(has_effects);
            match (&param.ty, param.mode) {
                (Type::Sampler(_), _) => match self.sampler_pair(arg)? {
                    Pair::Static(t, s) => {
                        texts.push(t);
                        texts.push(s);
                    }
                    Pair::Dynamic(..) => {
                        return Err(internal(
                            "a sampler array's element chosen at run time passed to a function",
                        ));
                    }
                },
                (ty, ParamMode::In) => {
                    let text = self.expr(arg)?;
                    if later && !self.simple(arg) {
                        let t = self.temp();
                        self.line(&format!("let {t} = {text};"));
                        texts.push(t);
                    } else {
                        texts.push(text);
                    }
                    let _ = ty;
                }
                (ty, mode) => {
                    let lv = self.lvalue(arg)?;
                    let t = self.temp();
                    if mode == ParamMode::InOut {
                        let read = self.lvalue_read(&lv);
                        self.line(&format!("var {t}: {} = {read};", wgsl_type(ty)));
                    } else {
                        self.line(&format!("var {t}: {};", wgsl_type(ty)));
                    }
                    texts.push(format!("&{t}"));
                    copies.push((lv, t));
                }
            }
        }
        let call = format!("{}({})", self.function_name(f), texts.join(", "));
        if copies.is_empty() {
            return Ok(call);
        }
        let ret = self.unit.functions[f].ret.clone();
        let result = if ret == Type::Void {
            self.line(&format!("{call};"));
            String::new()
        } else {
            let t = self.temp();
            self.line(&format!("let {t} = {call};"));
            t
        };
        for (lv, t) in copies {
            self.store(&lv, &t);
        }
        Ok(result)
    }

    // ---- Constructors ----

    /// The components of `args`, each a scalar's text, as many as
    /// `needed` (all when `None`).
    fn components(&mut self, args: &[Expr], needed: Option<u32>) -> R<Vec<String>> {
        let texts = self.operands(&args.iter().collect::<Vec<_>>())?;
        let mut out = Vec::new();
        for (a, text) in args.iter().zip(texts) {
            let n = a.ty.components();
            let text = if n > 1 { self.let_of(&text) } else { text };
            match &a.ty {
                Type::Scalar(_) => out.push(text),
                Type::Vector(_, size) => {
                    for i in 0..*size {
                        out.push(format!("{text}.{}", XYZW[i as usize]));
                    }
                }
                Type::Matrix(c, r) => {
                    for i in 0..*c {
                        for j in 0..*r {
                            out.push(format!("{text}[{i}][{j}]"));
                        }
                    }
                }
                _ => return Err(internal("components of no scalar, vector or matrix")),
            }
            if needed.is_some_and(|n| out.len() as u32 >= n) {
                break;
            }
        }
        if let Some(n) = needed {
            out.truncate(n as usize);
        }
        Ok(out)
    }

    fn construct(&mut self, ty: &Type, args: &[Expr]) -> R<String> {
        let wty = wgsl_type(ty);
        match ty {
            Type::Struct(_) | Type::Array(..) => {
                let texts = self.operands(&args.iter().collect::<Vec<_>>())?;
                Ok(format!("{wty}({})", texts.join(", ")))
            }
            Type::Scalar(s) => {
                let comps = self.components(&args[..1], Some(1))?;
                let source = args[0].ty.scalar().unwrap_or(*s);
                Ok(if source == *s {
                    comps[0].clone()
                } else {
                    format!("{}({})", scalar_wgsl(*s), comps[0])
                })
            }
            Type::Vector(s, n) => {
                if args.len() == 1 && args[0].ty.is_scalar() {
                    let c = self.components(args, Some(1))?;
                    return Ok(format!("{wty}({}({}))", scalar_wgsl(*s), c[0]));
                }
                // A vector of the same size, converted whole.
                if args.len() == 1 && args[0].ty.size() == *n && args[0].ty.is_vector() {
                    let t = self.expr(&args[0])?;
                    return Ok(if args[0].ty == *ty {
                        t
                    } else {
                        format!("{wty}({t})")
                    });
                }
                let comps = self.components(args, Some(u32::from(*n)))?;
                let conv: Vec<String> = comps
                    .into_iter()
                    .map(|c| format!("{}({c})", scalar_wgsl(*s)))
                    .collect();
                Ok(format!("{wty}({})", conv.join(", ")))
            }
            Type::Matrix(c, r) => {
                let (c, r) = (*c, *r);
                if args.len() == 1 && args[0].ty.is_scalar() {
                    let d = self.components(args, Some(1))?;
                    let d = self.let_of(&format!("f32({})", d[0]));
                    let cols: Vec<String> = (0..c)
                        .map(|i| {
                            let parts: Vec<String> = (0..r)
                                .map(|j| if i == j { d.clone() } else { "0.0f".to_owned() })
                                .collect();
                            format!("vec{r}<f32>({})", parts.join(", "))
                        })
                        .collect();
                    return Ok(format!("{wty}({})", cols.join(", ")));
                }
                if let [Type::Matrix(c2, r2)] = [args[0].ty.clone()]
                    && args.len() == 1
                {
                    let m = self.expr(&args[0])?;
                    let m = self.let_of(&m);
                    let cols: Vec<String> = (0..c)
                        .map(|i| {
                            let parts: Vec<String> = (0..r)
                                .map(|j| {
                                    if i < c2 && j < r2 {
                                        format!("{m}[{i}][{j}]")
                                    } else if i == j {
                                        "1.0f".to_owned()
                                    } else {
                                        "0.0f".to_owned()
                                    }
                                })
                                .collect();
                            format!("vec{r}<f32>({})", parts.join(", "))
                        })
                        .collect();
                    return Ok(format!("{wty}({})", cols.join(", ")));
                }
                let comps = self.components(args, Some(u32::from(c) * u32::from(r)))?;
                let cols: Vec<String> = (0..c as usize)
                    .map(|i| {
                        let parts: Vec<String> = comps[i * r as usize..(i + 1) * r as usize]
                            .iter()
                            .map(|x| format!("f32({x})"))
                            .collect();
                        format!("vec{r}<f32>({})", parts.join(", "))
                    })
                    .collect();
                Ok(format!("{wty}({})", cols.join(", ")))
            }
            _ => Err(internal("a constructor of this type")),
        }
    }

    // ---- Built-ins ----

    fn builtin(&mut self, name: &str, args: &[Expr], ty: &Type) -> R<String> {
        if texture_kind(name).is_some() {
            return self.texture(name, args, ty);
        }
        if name == "modf" {
            let x = self.expr(&args[0])?;
            let r = self.temp();
            self.line(&format!("let {r} = modf({x});"));
            let lv = self.lvalue(&args[1])?;
            self.store(&lv, &format!("{r}.whole"));
            return Ok(format!("{r}.fract"));
        }
        let texts = self.operands(&args.iter().collect::<Vec<_>>())?;
        let a = |i: usize| texts[i].clone();
        let splat_to = |t: &str, from: &Type, to: &Type| -> String {
            if from.is_scalar() && to.is_vector() {
                format!("{}({t})", wgsl_type(to))
            } else {
                t.to_owned()
            }
        };
        // GLSL's geometric functions take floats too; WGSL's vectors
        // alone: their scalar forms written out.
        if args.first().is_some_and(|e| e.ty.is_scalar()) {
            match name {
                "normalize" => return Ok(format!("sign({})", a(0))),
                "dot" => return Ok(format!("({} * {})", a(0), a(1))),
                "reflect" => {
                    let (i, n) = (self.let_of(&a(0)), self.let_of(&a(1)));
                    return Ok(format!("({i} - 2.0f * {n} * {i} * {n})"));
                }
                "faceforward" => {
                    let n = self.let_of(&a(0));
                    return Ok(format!("select(-{n}, {n}, ({} * {}) < 0.0f)", a(2), a(1)));
                }
                "refract" => {
                    let (i, n, eta) = (self.let_of(&a(0)), self.let_of(&a(1)), self.let_of(&a(2)));
                    let k = self.let_of(&format!(
                        "(1.0f - {eta} * {eta} * (1.0f - {n} * {i} * {n} * {i}))"
                    ));
                    return Ok(format!(
                        "select({eta} * {i} - ({eta} * {n} * {i} + sqrt({k})) * {n}, 0.0f, {k} < 0.0f)"
                    ));
                }
                _ => {}
            }
        }
        Ok(match name {
            "radians" | "degrees" | "sin" | "cos" | "tan" | "asin" | "acos" | "exp" | "log"
            | "exp2" | "log2" | "sqrt" | "abs" | "sign" | "floor" | "ceil" | "fract"
            | "normalize" | "length" | "sinh" | "cosh" | "tanh" | "asinh" | "acosh" | "atanh"
            | "trunc" | "determinant" | "transpose" => {
                format!("{name}({})", a(0))
            }
            "round" | "roundEven" => format!("round({})", a(0)),
            "inversesqrt" => format!("inverseSqrt({})", a(0)),
            "atan" if args.len() == 2 => format!("atan2({}, {})", a(0), a(1)),
            "atan" => format!("atan({})", a(0)),
            "pow" | "distance" | "dot" | "cross" | "reflect" => {
                format!("{name}({}, {})", a(0), a(1))
            }
            "refract" => format!("refract({}, {}, {})", a(0), a(1), a(2)),
            "faceforward" => format!("faceForward({}, {}, {})", a(0), a(1), a(2)),
            "mod" => {
                let x = self.let_of(&a(0));
                let y = self.let_of(&splat_to(&a(1), &args[1].ty, &args[0].ty));
                format!("({x} - {y} * floor({x} / {y}))")
            }
            "min" | "max" => format!(
                "{name}({}, {})",
                a(0),
                splat_to(&a(1), &args[1].ty, &args[0].ty)
            ),
            "clamp" => format!(
                "clamp({}, {}, {})",
                a(0),
                splat_to(&a(1), &args[1].ty, &args[0].ty),
                splat_to(&a(2), &args[2].ty, &args[0].ty)
            ),
            "mix" if args[2].ty.scalar() == Some(Scalar::Bool) => {
                format!("select({}, {}, {})", a(0), a(1), a(2))
            }
            "mix" => format!(
                "mix({}, {}, {})",
                a(0),
                a(1),
                splat_to(&a(2), &args[2].ty, &args[0].ty)
            ),
            "step" => format!(
                "step({}, {})",
                splat_to(&a(0), &args[0].ty, &args[1].ty),
                a(1)
            ),
            "smoothstep" => format!(
                "smoothstep({}, {}, {})",
                splat_to(&a(0), &args[0].ty, &args[2].ty),
                splat_to(&a(1), &args[1].ty, &args[2].ty),
                a(2)
            ),
            "matrixCompMult" => {
                let Type::Matrix(c, r) = ty else {
                    return Err(internal("matrixCompMult"));
                };
                let x = self.let_of(&a(0));
                let y = self.let_of(&a(1));
                let cols: Vec<String> = (0..*c).map(|i| format!("{x}[{i}] * {y}[{i}]")).collect();
                format!("mat{c}x{r}<f32>({})", cols.join(", "))
            }
            "outerProduct" => {
                let Type::Matrix(c, r) = ty else {
                    return Err(internal("outerProduct"));
                };
                let x = self.let_of(&a(0));
                let y = self.let_of(&a(1));
                let cols: Vec<String> = (0..*c)
                    .map(|i| format!("{x} * {y}.{}", XYZW[i as usize]))
                    .collect();
                format!("mat{c}x{r}<f32>({})", cols.join(", "))
            }
            "inverse" => {
                let Type::Matrix(n, _) = ty else {
                    return Err(internal("inverse"));
                };
                let name = self.inverse_helper(*n);
                format!("{name}({})", a(0))
            }
            "lessThan" => format!("({} < {})", a(0), a(1)),
            "lessThanEqual" => format!("({} <= {})", a(0), a(1)),
            "greaterThan" => format!("({} > {})", a(0), a(1)),
            "greaterThanEqual" => format!("({} >= {})", a(0), a(1)),
            "equal" => format!("({} == {})", a(0), a(1)),
            "notEqual" => format!("({} != {})", a(0), a(1)),
            "any" | "all" => format!("{name}({})", a(0)),
            "not" => format!("(!{})", a(0)),
            "dFdx" => format!("dpdx({})", a(0)),
            "dFdy" => format!("dpdy({})", a(0)),
            "fwidth" => format!("fwidth({})", a(0)),
            "isnan" => {
                let x = self.let_of(&a(0));
                format!("({x} != {x})")
            }
            "isinf" => format!(
                "(abs({}) > {})",
                a(0),
                splat_to("3.40282347e38f", &FLOAT, ty)
            ),
            "floatBitsToInt" | "floatBitsToUint" | "intBitsToFloat" | "uintBitsToFloat" => {
                format!("bitcast<{}>({})", wgsl_type(ty), a(0))
            }
            "packSnorm2x16" => format!("pack2x16snorm({})", a(0)),
            "unpackSnorm2x16" => format!("unpack2x16snorm({})", a(0)),
            "packUnorm2x16" => format!("pack2x16unorm({})", a(0)),
            "unpackUnorm2x16" => format!("unpack2x16unorm({})", a(0)),
            "packHalf2x16" => format!("pack2x16float({})", a(0)),
            "unpackHalf2x16" => format!("unpack2x16float({})", a(0)),
            other => return Err(internal(&format!("the built-in {other}"))),
        })
    }

    fn inverse_helper(&mut self, n: u8) -> String {
        let name = format!("webgl_inverse{n}");
        self.helper(&name, || match n {
            2 => "fn webgl_inverse2(m: mat2x2<f32>) -> mat2x2<f32> {\n    let d = m[0][0] * m[1][1] - m[1][0] * m[0][1];\n    return mat2x2<f32>(vec2<f32>(m[1][1], -m[0][1]), vec2<f32>(-m[1][0], m[0][0])) * (1.0f / d);\n}\n".to_owned(),
            3 => "fn webgl_inverse3(m: mat3x3<f32>) -> mat3x3<f32> {\n    let a = m[0];\n    let b = m[1];\n    let c = m[2];\n    let r0 = cross(b, c);\n    let r1 = cross(c, a);\n    let r2 = cross(a, b);\n    let d = dot(a, r0);\n    return transpose(mat3x3<f32>(r0, r1, r2)) * (1.0f / d);\n}\n".to_owned(),
            _ => "fn webgl_inverse4(m: mat4x4<f32>) -> mat4x4<f32> {\n    let a00 = m[0][0]; let a01 = m[0][1]; let a02 = m[0][2]; let a03 = m[0][3];\n    let a10 = m[1][0]; let a11 = m[1][1]; let a12 = m[1][2]; let a13 = m[1][3];\n    let a20 = m[2][0]; let a21 = m[2][1]; let a22 = m[2][2]; let a23 = m[2][3];\n    let a30 = m[3][0]; let a31 = m[3][1]; let a32 = m[3][2]; let a33 = m[3][3];\n    let b00 = a00 * a11 - a01 * a10; let b01 = a00 * a12 - a02 * a10;\n    let b02 = a00 * a13 - a03 * a10; let b03 = a01 * a12 - a02 * a11;\n    let b04 = a01 * a13 - a03 * a11; let b05 = a02 * a13 - a03 * a12;\n    let b06 = a20 * a31 - a21 * a30; let b07 = a20 * a32 - a22 * a30;\n    let b08 = a20 * a33 - a23 * a30; let b09 = a21 * a32 - a22 * a31;\n    let b10 = a21 * a33 - a23 * a31; let b11 = a22 * a33 - a23 * a32;\n    let d = 1.0f / (b00 * b11 - b01 * b10 + b02 * b09 + b03 * b08 - b04 * b07 + b05 * b06);\n    return mat4x4<f32>(\n        vec4<f32>(a11 * b11 - a12 * b10 + a13 * b09, a02 * b10 - a01 * b11 - a03 * b09, a31 * b05 - a32 * b04 + a33 * b03, a22 * b04 - a21 * b05 - a23 * b03) * d,\n        vec4<f32>(a12 * b08 - a10 * b11 - a13 * b07, a00 * b11 - a02 * b08 + a03 * b07, a32 * b02 - a30 * b05 - a33 * b01, a20 * b05 - a22 * b02 + a23 * b01) * d,\n        vec4<f32>(a10 * b10 - a11 * b08 + a13 * b06, a01 * b08 - a00 * b10 - a03 * b06, a30 * b04 - a31 * b02 + a33 * b00, a21 * b02 - a20 * b04 - a23 * b00) * d,\n        vec4<f32>(a11 * b07 - a10 * b09 - a12 * b06, a00 * b09 - a01 * b07 + a02 * b06, a31 * b01 - a30 * b03 - a32 * b00, a20 * b03 - a21 * b01 + a22 * b00) * d);\n}\n".to_owned(),
        })
    }

    /// A sampler expression's texture and sampler.
    fn sampler_pair(&mut self, e: &Expr) -> R<Pair> {
        match &e.kind {
            ExprKind::Global(g) => {
                let name = &self.unit.globals[*g].name;
                let first = self.sampler_binding(name)?;
                Ok(Pair::Static(
                    format!("webgl_tex{first}"),
                    format!("webgl_smp{first}"),
                ))
            }
            ExprKind::Param(p) => {
                let name = self.param_name(*p);
                Ok(Pair::Static(format!("{name}_tex"), format!("{name}_smp")))
            }
            ExprKind::Index(base, index) => {
                let ExprKind::Global(g) = base.kind else {
                    return Err(internal("a sampler of no uniform"));
                };
                let Type::Array(_, n) = base.ty else {
                    return Err(internal("a sampler array of no array"));
                };
                let name = self.unit.globals[g].name.clone();
                let first = self.sampler_binding(&name)?;
                if let Some(v) = &index.value {
                    let b = first + 2 * v[0].as_u32();
                    return Ok(Pair::Static(
                        format!("webgl_tex{b}"),
                        format!("webgl_smp{b}"),
                    ));
                }
                let i = self.expr(index)?;
                let i = self.let_of(&i);
                Ok(Pair::Dynamic(
                    (0..n)
                        .map(|k| {
                            (
                                format!("webgl_tex{}", first + 2 * k),
                                format!("webgl_smp{}", first + 2 * k),
                            )
                        })
                        .collect(),
                    i,
                ))
            }
            _ => Err(internal("a sampler expression of this kind")),
        }
    }

    fn sampler_binding(&self, name: &str) -> R<u32> {
        self.interface
            .samplers
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, _, b)| *b)
            .ok_or_else(|| internal("a sampler the link did not keep"))
    }

    /// A texture function's call: WGSL's sampling functions, with GL's
    /// projective divides, levels and biases.
    fn texture(&mut self, name: &str, args: &[Expr], ty: &Type) -> R<String> {
        let kind = texture_kind(name).expect("a texture function");
        let Type::Sampler(s) = args[0].ty else {
            return Err(internal("a texture function of no sampler"));
        };
        let pair = self.sampler_pair(&args[0])?;
        let rest = self.operands(&args[1..].iter().collect::<Vec<_>>())?;
        let rest: Vec<String> = rest.into_iter().map(|t| self.let_of(&t)).collect();
        let pairs = match &pair {
            Pair::Static(t, s) => vec![(t.clone(), s.clone())],
            Pair::Dynamic(list, _) => list.clone(),
        };
        let mut calls = Vec::new();
        for (tex, smp) in &pairs {
            calls.push(self.texture_call(kind, &s, tex, smp, args, &rest)?);
        }
        match pair {
            Pair::Static(..) => Ok(calls.remove(0)),
            Pair::Dynamic(_, index) => {
                // One binding a branch: textures are no values WGSL picks.
                let t = self.temp();
                self.line(&format!("var {t}: {};", wgsl_type(ty)));
                self.line(&format!("switch ({index}) {{"));
                self.indent += 1;
                for (k, call) in calls.iter().enumerate() {
                    let label = if k == 0 {
                        "case 0, default".to_owned()
                    } else {
                        format!("case {k}")
                    };
                    self.line(&format!("{label}: {{ {t} = {call}; }}"));
                }
                self.indent -= 1;
                self.line("}");
                Ok(t)
            }
        }
    }

    fn texture_call(
        &mut self,
        kind: crate::builtins::TexKind,
        s: &Sampler,
        tex: &str,
        smp: &str,
        args: &[Expr],
        rest: &[String],
    ) -> R<String> {
        let fragment = self.unit.stage == Stage::Fragment;
        if kind.size {
            let level = &rest[0];
            return Ok(match s.dim {
                Dim::D2 | Dim::Cube => format!("vec2<i32>(textureDimensions({tex}, {level}))"),
                Dim::D3 => format!("vec3<i32>(textureDimensions({tex}, {level}))"),
                Dim::D2Array => format!(
                    "vec3<i32>(vec3<u32>(textureDimensions({tex}, {level}), textureNumLayers({tex})))"
                ),
            });
        }
        if kind.fetch {
            let coord = &rest[0];
            let level = &rest[1];
            let coord = if kind.offset {
                format!("({coord} + {})", rest[2])
            } else {
                coord.clone()
            };
            return Ok(match s.dim {
                Dim::D2Array => format!("textureLoad({tex}, {coord}.xy, {coord}.z, {level})"),
                _ => format!("textureLoad({tex}, {coord}, {level})"),
            });
        }
        // The coordinates, projected.
        let coord = &rest[0];
        let coord_ty = &args[1].ty;
        let mut i = 1;
        let (uv, depth_ref): (String, Option<String>) = if kind.proj {
            let n = coord_ty.size();
            let w = XYZW[n as usize - 1];
            match (s.dim, s.shadow) {
                (Dim::D2, false) => (format!("({coord}.xy / {coord}.{w})"), None),
                (Dim::D2, true) => (
                    format!("({coord}.xy / {coord}.w)"),
                    Some(format!("({coord}.z / {coord}.w)")),
                ),
                (Dim::D3, _) => (format!("({coord}.xyz / {coord}.w)"), None),
                _ => return Err(internal("a projection of this sampler")),
            }
        } else if s.shadow {
            match s.dim {
                Dim::D2 => (format!("{coord}.xy"), Some(format!("{coord}.z"))),
                Dim::Cube => (format!("{coord}.xyz"), Some(format!("{coord}.w"))),
                Dim::D2Array => (format!("{coord}.xy"), Some(format!("{coord}.w"))),
                Dim::D3 => return Err(internal("a 3D shadow sampler")),
            }
        } else {
            match s.dim {
                Dim::D2Array => (format!("{coord}.xy"), None),
                _ => (coord.clone(), None),
            }
        };
        let layer = if s.dim == Dim::D2Array {
            let z = format!("{coord}.z");
            Some(format!("i32(floor({z} + 0.5f))"))
        } else {
            None
        };
        let mut level = None;
        let mut grad = None;
        let mut offset = None;
        let mut bias = None;
        if kind.lod {
            level = Some(rest[i].clone());
            i += 1;
        }
        if kind.grad {
            grad = Some((rest[i].clone(), rest[i + 1].clone()));
            i += 2;
        }
        if kind.offset {
            // A constant expression, as WGSL's offsets are.
            let value = args[i + 1]
                .value
                .as_ref()
                .ok_or_else(|| internal("an offset not constant"))?;
            offset = Some(constant(self.unit, &args[i + 1].ty, value));
            i += 1;
        }
        if i < rest.len() {
            bias = Some(rest[i].clone());
        }
        let layer_arg = layer.map(|l| format!(", {l}")).unwrap_or_default();
        let offset_arg = offset.map(|o| format!(", {o}")).unwrap_or_default();
        // Integer textures are read by texelLoad: their texel nearest
        // the coordinates, clamped (WGSL samples floats alone).
        if s.sample != Scalar::Float {
            let lod = level
                .clone()
                .map_or("0".to_owned(), |l| format!("i32({l})"));
            let dims = format!("textureDimensions({tex}, {lod})");
            let texel = match s.dim {
                Dim::D2 | Dim::D2Array => format!(
                    "clamp(vec2<i32>(floor({uv} * vec2<f32>({dims}))), vec2<i32>(0), vec2<i32>({dims}) - vec2<i32>(1))"
                ),
                Dim::D3 => format!(
                    "clamp(vec3<i32>(floor({uv} * vec3<f32>({dims}))), vec3<i32>(0), vec3<i32>({dims}) - vec3<i32>(1))"
                ),
                Dim::Cube => {
                    // The face and its coordinates of a direction (GL ES
                    // 3.0 table 3.21), then the face's texel.
                    let face = self.helper("webgl_cube_face", || {
                        "fn webgl_cube_face(d: vec3<f32>) -> vec3<f32> {
    let a = abs(d);
    if (a.x >= a.y && a.x >= a.z) {
        if (d.x >= 0.0f) { return vec3<f32>((-d.z / a.x + 1.0f) * 0.5f, (-d.y / a.x + 1.0f) * 0.5f, 0.0f); }
        return vec3<f32>((d.z / a.x + 1.0f) * 0.5f, (-d.y / a.x + 1.0f) * 0.5f, 1.0f);
    }
    if (a.y >= a.z) {
        if (d.y >= 0.0f) { return vec3<f32>((d.x / a.y + 1.0f) * 0.5f, (d.z / a.y + 1.0f) * 0.5f, 2.0f); }
        return vec3<f32>((d.x / a.y + 1.0f) * 0.5f, (-d.z / a.y + 1.0f) * 0.5f, 3.0f);
    }
    if (d.z >= 0.0f) { return vec3<f32>((d.x / a.z + 1.0f) * 0.5f, (-d.y / a.z + 1.0f) * 0.5f, 4.0f); }
    return vec3<f32>((-d.x / a.z + 1.0f) * 0.5f, (-d.y / a.z + 1.0f) * 0.5f, 5.0f);
}
".to_owned()
                    });
                    let f = self.let_of(&format!("{face}({uv})"));
                    let texel = format!(
                        "clamp(vec2<i32>(floor({f}.xy * vec2<f32>({dims}))), vec2<i32>(0), vec2<i32>({dims}) - vec2<i32>(1))"
                    );
                    return Ok(format!("textureLoad({tex}, {texel}, i32({f}.z), {lod})"));
                }
            };
            return Ok(match s.dim {
                Dim::D2Array => format!(
                    "textureLoad({tex}, {texel}, {}, {lod})",
                    layer_arg.trim_start_matches(", ")
                ),
                _ => format!("textureLoad({tex}, {texel}, {lod})"),
            });
        }
        if let Some(r) = depth_ref {
            // Comparisons: WGSL's level-0 form outside fragment shaders and
            // for explicit levels and gradients.
            return Ok(if fragment && level.is_none() && grad.is_none() {
                format!("textureSampleCompare({tex}, {smp}, {uv}{layer_arg}, {r}{offset_arg})")
            } else {
                format!("textureSampleCompareLevel({tex}, {smp}, {uv}{layer_arg}, {r}{offset_arg})")
            });
        }
        Ok(if let Some((dx, dy)) = grad {
            format!("textureSampleGrad({tex}, {smp}, {uv}{layer_arg}, {dx}, {dy}{offset_arg})")
        } else if let Some(l) = level {
            format!("textureSampleLevel({tex}, {smp}, {uv}{layer_arg}, {l}{offset_arg})")
        } else if !fragment {
            format!("textureSampleLevel({tex}, {smp}, {uv}{layer_arg}, 0.0f{offset_arg})")
        } else if let Some(b) = bias {
            format!("textureSampleBias({tex}, {smp}, {uv}{layer_arg}, {b}{offset_arg})")
        } else {
            format!("textureSample({tex}, {smp}, {uv}{layer_arg}{offset_arg})")
        })
    }

    // ---- The entry point ----

    fn entry(&mut self) -> R<String> {
        self.out.clear();
        self.indent = 0;
        let unit = self.unit;
        let vertex = unit.stage == Stage::Vertex;
        let mut text = String::new();
        let mut inputs: Vec<String> = Vec::new();
        let mut outputs: Vec<String> = Vec::new();
        let mut prologue: Vec<String> = Vec::new();
        let mut epilogue: Vec<String> = Vec::new();
        let builtin = |b: BuiltinVar| {
            unit.globals
                .iter()
                .position(|g| g.kind == GlobalKind::Builtin(b))
        };
        if vertex {
            for (g, global) in unit.globals.iter().enumerate() {
                if !matches!(global.kind, GlobalKind::Attribute(_)) || !global.used {
                    continue;
                }
                let location = self
                    .interface
                    .attributes
                    .get(&global.name)
                    .copied()
                    .unwrap_or(0);
                let name = self.global_name(g);
                match &global.ty {
                    Type::Matrix(c, r) => {
                        let cols: Vec<String> = (0..*c)
                            .map(|i| {
                                inputs.push(format!(
                                    "    @location({}) {name}_{i}: vec{r}<f32>,",
                                    location + u32::from(i)
                                ));
                                format!("webgl_in.{name}_{i}")
                            })
                            .collect();
                        prologue.push(format!("{name} = mat{c}x{r}<f32>({});", cols.join(", ")));
                    }
                    ty => {
                        inputs.push(format!(
                            "    @location({location}) {name}: {},",
                            wgsl_type(ty)
                        ));
                        prologue.push(format!("{name} = webgl_in.{name};"));
                    }
                }
            }
            let index = if self.points {
                "webgl_instance"
            } else {
                "webgl_vertex"
            };
            if let Some(g) = builtin(BuiltinVar::VertexId) {
                prologue.push(format!("{} = i32({index});", self.global_name(g)));
            }
            if let Some(g) = builtin(BuiltinVar::InstanceId) {
                // Each point is an instance of its own in the points
                // variant; GL's instances are not then.
                let instance = if self.points { "0u" } else { "webgl_instance" };
                prologue.push(format!("{} = i32({instance});", self.global_name(g)));
            }
            let position = builtin(BuiltinVar::Position)
                .map(|g| self.global_name(g))
                .unwrap_or_default();
            let invariant =
                builtin(BuiltinVar::Position).is_some_and(|g| unit.globals[g].invariant);
            outputs.push(format!(
                "    @builtin(position){} webgl_position: vec4<f32>,",
                if invariant { " @invariant" } else { "" }
            ));
            for (name, ty, location, flat) in &self.interface.varyings {
                let Some((g, _)) = unit.global(name) else {
                    continue;
                };
                let var = self.global_name(g);
                let interp = if *flat { " @interpolate(flat)" } else { "" };
                for (k, (part, part_ty)) in varying_parts(&var, ty).into_iter().enumerate() {
                    let field = format!("{}_{k}", var);
                    outputs.push(format!(
                        "    @location({}){interp} {field}: {},",
                        location + k as u32,
                        wgsl_type(&part_ty)
                    ));
                    epilogue.push(format!("webgl_out.{field} = {part};"));
                }
            }
            if let Some(location) = self.interface.point_coord {
                outputs.push(format!(
                    "    @location({location}) webgl_point_coord: vec2<f32>,"
                ));
                if !self.points {
                    epilogue.push("webgl_out.webgl_point_coord = vec2<f32>(0.0f);".to_owned());
                }
            }
            // The position, as WebGPU's: depth from [-1, 1] to [0, 1],
            // y flipped (the frame is drawn upside down).
            epilogue.push(format!("var webgl_p = {position};"));
            if self.points {
                let size = builtin(BuiltinVar::PointSize)
                    .map(|g| self.global_name(g))
                    .unwrap_or_else(|| "1.0f".to_owned());
                epilogue.push("var webgl_corners = array<vec2<f32>, 4>(vec2<f32>(-1.0f, -1.0f), vec2<f32>(1.0f, -1.0f), vec2<f32>(-1.0f, 1.0f), vec2<f32>(1.0f, 1.0f));".to_owned());
                epilogue.push("let webgl_corner = webgl_corners[webgl_vertex % 4u];".to_owned());
                epilogue.push(format!("let webgl_size = clamp({size}, 1.0f, 1024.0f);"));
                epilogue.push("webgl_p = vec4<f32>(webgl_p.xy + webgl_corner * webgl_size / max(webgl_u.webgl_viewport.zw, vec2<f32>(1.0f)) * webgl_p.w, webgl_p.zw);".to_owned());
                if self.interface.point_coord.is_some() {
                    epilogue.push("webgl_out.webgl_point_coord = vec2<f32>((webgl_corner.x + 1.0f) * 0.5f, (1.0f - webgl_corner.y) * 0.5f);".to_owned());
                }
            }
            epilogue.push("webgl_out.webgl_position = vec4<f32>(webgl_p.x, -webgl_p.y, (webgl_p.z + webgl_p.w) * 0.5f, webgl_p.w);".to_owned());
        } else {
            inputs.push("    @builtin(position) webgl_frag_coord: vec4<f32>,".to_owned());
            if let Some(g) = builtin(BuiltinVar::FragCoord) {
                prologue.push(format!(
                    "{} = webgl_in.webgl_frag_coord;",
                    self.global_name(g)
                ));
            }
            if let Some(g) = builtin(BuiltinVar::FrontFacing).filter(|&g| unit.globals[g].used) {
                inputs.push("    @builtin(front_facing) webgl_front_facing: bool,".to_owned());
                prologue.push(format!(
                    "{} = webgl_in.webgl_front_facing;",
                    self.global_name(g)
                ));
            }
            for (name, ty, location, flat) in &self.interface.varyings {
                let Some((g, _)) = unit.global(name) else {
                    continue;
                };
                let var = self.global_name(g);
                let interp = if *flat { " @interpolate(flat)" } else { "" };
                let parts = varying_parts(&var, ty);
                let mut fields = Vec::new();
                for (k, (_, part_ty)) in parts.iter().enumerate() {
                    let field = format!("{var}_{k}");
                    inputs.push(format!(
                        "    @location({}){interp} {field}: {},",
                        location + k as u32,
                        wgsl_type(part_ty)
                    ));
                    fields.push(format!("webgl_in.{field}"));
                }
                prologue.push(format!("{var} = {};", rebuild(ty, &fields)));
            }
            if let (Some(location), Some(g)) =
                (self.interface.point_coord, builtin(BuiltinVar::PointCoord))
            {
                inputs.push(format!(
                    "    @location({location}) webgl_point_coord: vec2<f32>,"
                ));
                prologue.push(format!(
                    "{} = webgl_in.webgl_point_coord;",
                    self.global_name(g)
                ));
            }
            if let Some(g) = builtin(BuiltinVar::FragDepth).filter(|&g| unit.globals[g].used) {
                prologue.push(format!(
                    "{} = webgl_in.webgl_frag_coord.z;",
                    self.global_name(g)
                ));
                outputs.push("    @builtin(frag_depth) webgl_depth: f32,".to_owned());
                epilogue.push(format!("webgl_out.webgl_depth = {};", self.global_name(g)));
            }
            for (g, location) in &self.interface.outputs {
                let global = &unit.globals[*g];
                let name = self.global_name(*g);
                match &global.ty {
                    Type::Array(e, n) => {
                        for k in 0..*n {
                            outputs.push(format!(
                                "    @location({}) {name}_{k}: {},",
                                location + k,
                                wgsl_type(e)
                            ));
                            epilogue.push(format!("webgl_out.{name}_{k} = {name}[{k}];"));
                        }
                    }
                    ty => {
                        outputs.push(format!(
                            "    @location({location}) {name}: {},",
                            wgsl_type(ty)
                        ));
                        epilogue.push(format!("webgl_out.{name} = {name};"));
                    }
                }
            }
        }
        // The globals' initializers, as main begins.
        for (g, global) in unit.globals.iter().enumerate() {
            if global.kind == GlobalKind::Private
                && let Some(init) = &global.init
            {
                self.out.clear();
                self.indent = 1;
                let value = self.expr(init)?;
                let pre = std::mem::take(&mut self.out);
                prologue.push(format!(
                    "{}{} = {value};",
                    pre.trim_start(),
                    self.global_name(g)
                ));
            }
        }
        let stage = if vertex { "vertex" } else { "fragment" };
        let in_struct = if vertex {
            "webgl_VertexInput"
        } else {
            "webgl_FragmentInput"
        };
        let out_struct = if vertex {
            "webgl_VertexOutput"
        } else {
            "webgl_FragmentOutput"
        };
        let mut params = Vec::new();
        if !inputs.is_empty() {
            let _ = writeln!(text, "struct {in_struct} {{\n{}\n}}", inputs.join("\n"));
            params.push(format!("webgl_in: {in_struct}"));
        }
        if vertex {
            params.push("@builtin(vertex_index) webgl_vertex: u32".to_owned());
            params.push("@builtin(instance_index) webgl_instance: u32".to_owned());
        }
        let ret = if outputs.is_empty() {
            String::new()
        } else {
            let _ = writeln!(text, "struct {out_struct} {{\n{}\n}}", outputs.join("\n"));
            format!(" -> {out_struct}")
        };
        let _ = writeln!(text, "@{stage} fn main({}){ret} {{", params.join(", "));
        for line in &prologue {
            let _ = writeln!(text, "    {line}");
        }
        let _ = writeln!(text, "    {}();", self.function_name(unit.main));
        if !outputs.is_empty() {
            let _ = writeln!(text, "    var webgl_out: {out_struct};");
            for line in &epilogue {
                let _ = writeln!(text, "    {line}");
            }
            let _ = writeln!(text, "    return webgl_out;");
        }
        text.push_str("}\n");
        Ok(text)
    }
}

/// A varying's parts that cross between the stages: a matrix's columns,
/// an array's elements, each with its type.
fn varying_parts(var: &str, ty: &Type) -> Vec<(String, Type)> {
    match ty {
        Type::Matrix(c, r) => (0..*c)
            .map(|i| (format!("{var}[{i}]"), Type::Vector(Scalar::Float, *r)))
            .collect(),
        Type::Array(element, n) => (0..*n)
            .flat_map(|i| varying_parts(&format!("{var}[{i}]"), element))
            .collect(),
        other => vec![(var.to_owned(), other.clone())],
    }
}

/// A varying of `ty` made again from its parts.
fn rebuild(ty: &Type, parts: &[String]) -> String {
    match ty {
        Type::Matrix(..) => format!("{}({})", wgsl_type(ty), parts.join(", ")),
        Type::Array(element, n) => {
            let per = parts.len() / *n as usize;
            let elements: Vec<String> = (0..*n as usize)
                .map(|i| rebuild(element, &parts[i * per..(i + 1) * per]))
                .collect();
            format!("{}({})", wgsl_type(ty), elements.join(", "))
        }
        _ => parts[0].clone(),
    }
}

fn scalar_type(ty: &Type) -> Type {
    Type::Scalar(ty.scalar().unwrap_or(Scalar::Float))
}

/// A type's text as a part of a helper's name.
fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
