//! WebGL's shaders (M41 step 5, D40, docs/design/gpu.md): the OpenGL ES
//! Shading Language 1.00 and 3.00, as WebGL restricts them, read and
//! checked ([`compile`]), and a program's two shaders translated to WGSL
//! ([`link`]) that naga reads and validates here, with what the program
//! offers a page: its attributes, uniforms, samplers and blocks, where
//! each is. Errors are in the reference's form, `ERROR: 0:<line>:
//! '<token>' : <what>`.
//!
//! The translation is GL's model on WebGPU's: the default block's
//! uniforms in one uniform buffer (binding 0), each sampler a texture
//! and a sampler binding, attributes at the locations the link gives,
//! varyings matched by name; positions turned to WebGPU's depth range
//! and flipped vertically, the frame drawn upside down so that a
//! texture's first row is GL's bottom; and a variant of the vertex
//! shader that draws points of `gl_PointSize` as quads.

#![forbid(unsafe_code)]

mod builtins;
mod emit;
mod expr;
mod lex;
mod parse;
mod pre;
pub mod tir;

pub use emit::{
    Attribute, BlockInfo, BlockUniform, LinkOptions, Output, Program, SamplerUniform, Uniform, link,
};

/// The most bytes a shader's source may have.
pub const MAX_SHADER_SOURCE: usize = 1 << 20;

/// The most nesting of blocks, expressions and macros.
pub const MAX_GLSL_DEPTH: usize = 64;

/// The most tokens a shader may have after preprocessing.
pub const MAX_GLSL_TOKENS: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    Vertex,
    Fragment,
}

/// The WebGL 1 extensions a context enabled that its shaders may name.
#[derive(Clone, Debug, Default)]
pub struct Extensions {
    pub standard_derivatives: bool,
    pub frag_depth: bool,
    pub draw_buffers: bool,
    pub shader_texture_lod: bool,
}

/// What a context gives its shaders: WebGL 2's GLSL ES 3.00 or not, its
/// extensions, and its limits, as the built-in constants say them.
#[derive(Clone, Debug)]
pub struct Options {
    pub webgl2: bool,
    pub extensions: Extensions,
    pub max_vertex_attribs: u32,
    pub max_vertex_uniform_vectors: u32,
    pub max_fragment_uniform_vectors: u32,
    pub max_varying_vectors: u32,
    pub max_vertex_texture_units: u32,
    pub max_texture_units: u32,
    pub max_combined_texture_units: u32,
    pub max_draw_buffers: u32,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            webgl2: false,
            extensions: Extensions::default(),
            max_vertex_attribs: 16,
            max_vertex_uniform_vectors: 4096,
            max_fragment_uniform_vectors: 1024,
            // WebGPU's 16 inter-stage locations, one kept for points.
            max_varying_vectors: 15,
            max_vertex_texture_units: 16,
            max_texture_units: 16,
            max_combined_texture_units: 32,
            max_draw_buffers: 8,
        }
    }
}

/// A shader's error, in the reference's form.
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub line: u32,
    pub token: String,
    pub message: String,
}

impl Error {
    pub fn new(line: u32, token: &str, message: &str) -> Error {
        Error {
            line,
            token: token.to_owned(),
            message: message.to_owned(),
        }
    }

    /// The info log's line.
    pub fn log(&self) -> String {
        format!(
            "ERROR: 0:{}: '{}' : {}\n",
            self.line, self.token, self.message
        )
    }
}

/// A uniform block (GLSL ES 3.00): its name, its instance's, its array's
/// size, its members, and the global it is in the checked shader.
#[derive(Clone, Debug)]
pub struct Block {
    pub name: String,
    pub instance: Option<String>,
    pub array: Option<u32>,
    pub members: Vec<BlockMember>,
    pub global: usize,
    pub used: bool,
}

#[derive(Clone, Debug)]
pub struct BlockMember {
    pub name: String,
    pub ty: tir::Type,
    pub row_major: bool,
}

/// A shader read and checked.
#[derive(Clone, Debug)]
pub struct Shader {
    pub unit: tir::Unit,
    pub blocks: Vec<Block>,
    /// Its warnings, the info log of a shader that compiled.
    pub log: String,
}

/// Reads and checks a shader of `stage`: the shader, or the info log
/// that says why it does not compile.
pub fn compile(source: &str, stage: Stage, options: &Options) -> Result<Shader, String> {
    if source.len() > MAX_SHADER_SOURCE {
        return Err(Error::new(0, "", "the shader's source is too long").log());
    }
    let pre = pre::preprocess(source, stage, options).map_err(|e| e.log())?;
    let parser = parse::Parser::new(&pre.tokens, stage, pre.version, pre.enabled, options);
    let (mut unit, mut blocks) = parser.unit().map_err(|e| e.log())?;
    // The globals main's functions reach are the used ones.
    mark_reached(&mut unit);
    for block in &mut blocks {
        block.used = unit.globals[block.global].used;
    }
    Ok(Shader {
        unit,
        blocks,
        log: pre.warnings,
    })
}

/// Keeps `used` for the globals the functions `main` reaches read or
/// write: a function no one calls does not make its globals active.
fn mark_reached(unit: &mut tir::Unit) {
    let mut reached = vec![false; unit.functions.len()];
    let mut stack = vec![unit.main];
    while let Some(f) = stack.pop() {
        if reached[f] {
            continue;
        }
        reached[f] = true;
        stack.extend(unit.functions[f].calls.iter().copied());
    }
    let mut used = vec![false; unit.globals.len()];
    for (f, function) in unit.functions.iter().enumerate() {
        if !reached[f] {
            continue;
        }
        if let Some(body) = &function.body {
            for stmt in body {
                emit::globals_in_stmt(stmt, &mut used);
            }
        }
    }
    // Globals' initializers reach what they read.
    for g in 0..unit.globals.len() {
        if let Some(init) = unit.globals[g].init.clone() {
            emit::globals_in_expr(&init, &mut used);
        }
    }
    for (g, global) in unit.globals.iter_mut().enumerate() {
        global.used = used[g];
    }
}

#[cfg(test)]
mod tests;

/// A shader of the other stage that links with `shader`: for a fragment
/// shader, a vertex shader writing every varying it reads; for a vertex
/// shader, a fragment shader of one colour. For the tests and the
/// fuzzer.
pub fn partner(shader: &Shader) -> String {
    let unit = &shader.unit;
    let v3 = unit.version == 300;
    let mut out = String::new();
    if v3 {
        out.push_str("#version 300 es\n");
    }
    match unit.stage {
        Stage::Vertex => {
            if v3 {
                out.push_str("precision mediump float;\nout vec4 partner_color;\nvoid main() { partner_color = vec4(1.0); }\n");
            } else {
                out.push_str(
                    "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n",
                );
            }
        }
        Stage::Fragment => {
            for global in &unit.globals {
                let tir::GlobalKind::Varying {
                    output: false,
                    flat,
                } = global.kind
                else {
                    continue;
                };
                let (element, array) = match &global.ty {
                    tir::Type::Array(e, n) => ((**e).clone(), format!("[{n}]")),
                    other => (other.clone(), String::new()),
                };
                let qualifier = match (v3, flat) {
                    (true, true) => "flat out",
                    (true, false) => "out",
                    (false, _) => "varying",
                };
                out.push_str(&format!(
                    "{qualifier} highp {} {}{array};\n",
                    element.name(unit),
                    global.name
                ));
            }
            out.push_str("void main() { gl_Position = vec4(0.0); }\n");
        }
    }
    out
}
