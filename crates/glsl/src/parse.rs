//! The grammar and the checks (GLSL ES 1.00 and 3.00, sections 4 to 9,
//! and WebGL's own rules): a recursive descent over the preprocessed
//! tokens that resolves every name and type as it reads, GLSL declaring
//! everything before its use, into the typed tree of [`crate::tir`].
//! Expressions are `expr`'s.

use std::collections::HashMap;

use crate::builtins::Ctx;
use crate::lex::{Tok, Token};
use crate::tir::*;
use crate::{Error, MAX_GLSL_DEPTH, Options, Stage};

/// An integer literal's value and whether it is unsigned (`u`): `None`
/// past 32 bits.
pub fn int_literal(text: &str) -> Option<(u32, bool)> {
    let (digits, unsigned) = match text.strip_suffix(['u', 'U']) {
        Some(d) => (d, true),
        None => (text, false),
    };
    let value = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).ok()?
    } else if digits.len() > 1 && digits.starts_with('0') {
        u64::from_str_radix(&digits[1..], 8).ok()?
    } else {
        digits.parse::<u64>().ok()?
    };
    u32::try_from(value).ok().map(|v| (v, unsigned))
}

/// The words GLSL ES keeps for itself, which no name may be (section
/// 3.7), by version: 1.00's and 3.00's.
const RESERVED_100: &[&str] = &[
    "asm",
    "class",
    "union",
    "enum",
    "typedef",
    "template",
    "this",
    "packed",
    "goto",
    "switch",
    "default",
    "inline",
    "noinline",
    "volatile",
    "public",
    "static",
    "extern",
    "external",
    "interface",
    "flat",
    "long",
    "short",
    "double",
    "half",
    "fixed",
    "unsigned",
    "superp",
    "input",
    "output",
    "hvec2",
    "hvec3",
    "hvec4",
    "dvec2",
    "dvec3",
    "dvec4",
    "fvec2",
    "fvec3",
    "fvec4",
    "sampler1D",
    "sampler3D",
    "sampler1DShadow",
    "sampler2DShadow",
    "sampler2DRect",
    "sampler3DRect",
    "sampler2DRectShadow",
    "sizeof",
    "cast",
    "namespace",
    "using",
];

const RESERVED_300: &[&str] = &[
    "attribute",
    "varying",
    "coherent",
    "restrict",
    "readonly",
    "writeonly",
    "resource",
    "atomic_uint",
    "noperspective",
    "patch",
    "sample",
    "subroutine",
    "common",
    "partition",
    "active",
    "asm",
    "class",
    "union",
    "enum",
    "typedef",
    "template",
    "this",
    "goto",
    "inline",
    "noinline",
    "volatile",
    "public",
    "static",
    "extern",
    "external",
    "interface",
    "long",
    "short",
    "double",
    "half",
    "fixed",
    "unsigned",
    "superp",
    "input",
    "output",
    "hvec2",
    "hvec3",
    "hvec4",
    "dvec2",
    "dvec3",
    "dvec4",
    "fvec2",
    "fvec3",
    "fvec4",
    "sampler3DRect",
    "filter",
    "image1D",
    "image2D",
    "image3D",
    "imageCube",
    "iimage1D",
    "iimage2D",
    "iimage3D",
    "iimageCube",
    "uimage1D",
    "uimage2D",
    "uimage3D",
    "uimageCube",
    "image1DArray",
    "image2DArray",
    "iimage1DArray",
    "iimage2DArray",
    "uimage1DArray",
    "uimage2DArray",
    "imageBuffer",
    "iimageBuffer",
    "uimageBuffer",
    "sampler1D",
    "sampler1DShadow",
    "sampler1DArray",
    "sampler1DArrayShadow",
    "isampler1D",
    "isampler1DArray",
    "usampler1D",
    "usampler1DArray",
    "sampler2DRect",
    "sampler2DRectShadow",
    "isampler2DRect",
    "usampler2DRect",
    "samplerBuffer",
    "isamplerBuffer",
    "usamplerBuffer",
    "sampler2DMS",
    "isampler2DMS",
    "usampler2DMS",
    "sampler2DMSArray",
    "isampler2DMSArray",
    "usampler2DMSArray",
    "sizeof",
    "cast",
    "namespace",
    "using",
];

/// The keywords of both versions, which are no names either.
const KEYWORDS: &[&str] = &[
    "attribute",
    "const",
    "uniform",
    "varying",
    "break",
    "continue",
    "do",
    "for",
    "while",
    "if",
    "else",
    "in",
    "out",
    "inout",
    "float",
    "int",
    "void",
    "bool",
    "true",
    "false",
    "lowp",
    "mediump",
    "highp",
    "precision",
    "invariant",
    "discard",
    "return",
    "mat2",
    "mat3",
    "mat4",
    "vec2",
    "vec3",
    "vec4",
    "ivec2",
    "ivec3",
    "ivec4",
    "bvec2",
    "bvec3",
    "bvec4",
    "sampler2D",
    "samplerCube",
    "struct",
];

const KEYWORDS_300: &[&str] = &[
    "layout",
    "centroid",
    "flat",
    "smooth",
    "switch",
    "case",
    "default",
    "uint",
    "uvec2",
    "uvec3",
    "uvec4",
    "mat2x2",
    "mat2x3",
    "mat2x4",
    "mat3x2",
    "mat3x3",
    "mat3x4",
    "mat4x2",
    "mat4x3",
    "mat4x4",
    "sampler3D",
    "sampler2DShadow",
    "samplerCubeShadow",
    "sampler2DArray",
    "sampler2DArrayShadow",
    "isampler2D",
    "isampler3D",
    "isamplerCube",
    "isampler2DArray",
    "usampler2D",
    "usampler3D",
    "usamplerCube",
    "usampler2DArray",
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Var {
    Global(usize),
    Local(usize),
    Param(usize),
}

#[derive(Clone, Debug)]
pub(crate) enum Symbol {
    Var(Var),
    Struct(usize),
}

/// What a declaration's qualifiers said.
#[derive(Clone, Debug, Default)]
pub(crate) struct Qualifiers {
    storage: Option<&'static str>,
    precision: Option<&'static str>,
    invariant: bool,
    flat: bool,
    centroid: bool,
    smooth: bool,
    location: Option<u32>,
    /// A block's layout: `std140`, `shared`, `packed`, and the matrices'
    /// `row_major` or `column_major`.
    block_layout: Vec<String>,
    line: u32,
}

/// A function being read: its locals and where its loops and switches
/// are.
struct FnCtx {
    index: usize,
    ret: Type,
    loops: usize,
    switches: usize,
    /// The loop indices of 1.00's loops (Appendix A), which the body
    /// may not change.
    loop_indices: Vec<usize>,
}

pub struct Parser<'a> {
    toks: &'a [Token],
    at: usize,
    pub unit: Unit,
    scopes: Vec<HashMap<String, Symbol>>,
    /// The user functions, by name: their overloads' indices.
    functions: HashMap<String, Vec<usize>>,
    func: Option<FnCtx>,
    pub ctx: Ctx,
    depth: usize,
    /// The default precisions in force, a scope each: float's, and
    /// samplers' by type name.
    precisions: Vec<HashMap<String, &'static str>>,
    max_identifier: usize,
    /// Whether a declaration other than a preprocessor's came yet.
    pub declared: bool,
    /// The uniform blocks (GLSL ES 3.00).
    pub blocks: Vec<crate::Block>,
    /// The members of blocks without an instance's name, names of the
    /// global scope: their block's global and their field.
    pub block_members: HashMap<String, (usize, usize)>,
    /// A local just declared, for the statement that declares it.
    pending_decl: Option<(usize, Option<Expr>)>,
}

type R<T> = Result<T, Error>;

impl<'a> Parser<'a> {
    pub fn new(
        toks: &'a [Token],
        stage: Stage,
        version: u32,
        enabled: Vec<String>,
        options: &Options,
    ) -> Self {
        let ctx = Ctx {
            stage,
            version,
            derivatives: version == 300
                || enabled.iter().any(|e| e == "GL_OES_standard_derivatives"),
            texture_lod: version == 100 && enabled.iter().any(|e| e == "GL_EXT_shader_texture_lod"),
            frag_depth: version == 300 || enabled.iter().any(|e| e == "GL_EXT_frag_depth"),
            draw_buffers: version == 100 && enabled.iter().any(|e| e == "GL_EXT_draw_buffers"),
        };
        let mut parser = Parser {
            toks,
            at: 0,
            unit: Unit {
                stage,
                version,
                structs: Vec::new(),
                globals: Vec::new(),
                functions: Vec::new(),
                main: usize::MAX,
                enabled,
            },
            scopes: vec![HashMap::new()],
            functions: HashMap::new(),
            func: None,
            ctx,
            depth: 0,
            precisions: vec![HashMap::new()],
            max_identifier: if options.webgl2 { 1024 } else { 256 },
            declared: false,
            blocks: Vec::new(),
            block_members: HashMap::new(),
            pending_decl: None,
        };
        parser.builtin_variables(options);
        parser
    }

    // ---- The tokens ----

    pub fn peek(&self) -> Option<&Token> {
        self.toks.get(self.at)
    }

    pub fn peek_at(&self, ahead: usize) -> Option<&Token> {
        self.toks.get(self.at + ahead)
    }

    pub fn line(&self) -> u32 {
        self.peek()
            .or_else(|| self.toks.last())
            .map_or(1, |t| t.line)
    }

    pub fn next(&mut self) -> R<Token> {
        let token =
            self.toks.get(self.at).cloned().ok_or_else(|| {
                Error::new(self.line(), "", "syntax error: unexpected end of file")
            })?;
        self.at += 1;
        Ok(token)
    }

    pub fn is(&self, punct: &str) -> bool {
        self.peek().is_some_and(|t| t.is(punct))
    }

    pub fn is_word(&self, word: &str) -> bool {
        self.peek().and_then(Token::ident) == Some(word)
    }

    pub fn eat(&mut self, punct: &str) -> bool {
        if self.is(punct) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    pub fn eat_word(&mut self, word: &str) -> bool {
        if self.is_word(word) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    pub fn expect(&mut self, punct: &str) -> R<()> {
        if self.eat(punct) {
            Ok(())
        } else {
            Err(self.syntax())
        }
    }

    /// A syntax error at the token here.
    pub fn syntax(&self) -> Error {
        match self.peek() {
            Some(t) => Error::new(t.line, &t.text(), "syntax error"),
            None => Error::new(self.line(), "", "syntax error: unexpected end of file"),
        }
    }

    pub fn deeper(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_GLSL_DEPTH {
            return Err(Error::new(self.line(), "", "the shader nests too deeply"));
        }
        Ok(())
    }

    pub fn shallower(&mut self) {
        self.depth -= 1;
    }

    // ---- Names ----

    fn is_keyword(&self, name: &str) -> bool {
        KEYWORDS.contains(&name) || (self.ctx.version == 300 && KEYWORDS_300.contains(&name))
    }

    /// A name the shader declares, checked as WebGL wants.
    pub fn name(&mut self) -> R<(String, u32)> {
        let token = self.next()?;
        let Some(name) = token.ident() else {
            return Err(Error::new(token.line, &token.text(), "syntax error"));
        };
        let reserved = if self.ctx.version == 300 {
            RESERVED_300
        } else {
            RESERVED_100
        };
        if reserved.contains(&name) {
            return Err(Error::new(token.line, name, "Illegal use of reserved word"));
        }
        if self.is_keyword(name) {
            return Err(Error::new(token.line, name, "syntax error"));
        }
        if name.starts_with("gl_") {
            return Err(Error::new(token.line, name, "reserved built-in name"));
        }
        if name.starts_with("webgl_") || name.starts_with("_webgl_") {
            return Err(Error::new(token.line, name, "reserved built-in name"));
        }
        if name.contains("__") {
            return Err(Error::new(
                token.line,
                name,
                "identifiers containing two consecutive underscores (__) are reserved as possible future keywords",
            ));
        }
        if name.len() > self.max_identifier {
            return Err(Error::new(token.line, name, "Identifier name is too long"));
        }
        Ok((name.to_owned(), token.line))
    }

    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    pub fn lookup_var(&self, name: &str) -> Option<Var> {
        match self.lookup(name) {
            Some(Symbol::Var(v)) => Some(*v),
            _ => None,
        }
    }

    pub fn lookup_struct(&self, name: &str) -> Option<usize> {
        match self.lookup(name) {
            Some(Symbol::Struct(s)) => Some(*s),
            _ => None,
        }
    }

    pub fn functions_named(&self, name: &str) -> Option<&Vec<usize>> {
        self.functions.get(name)
    }

    /// Declares `name` in the innermost scope.
    fn declare(&mut self, name: &str, symbol: Symbol, line: u32) -> R<()> {
        // A function's parameters are in the scope its body opens.
        if self.scopes.len() == 1 && self.functions.contains_key(name) {
            return Err(Error::new(line, name, "redefinition"));
        }
        let scope = self.scopes.last_mut().expect("the global scope");
        if scope.contains_key(name) {
            return Err(Error::new(line, name, "redefinition"));
        }
        scope.insert(name.to_owned(), symbol);
        Ok(())
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.precisions.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
        self.precisions.pop();
    }

    pub fn var_type(&self, var: Var) -> &Type {
        match var {
            Var::Global(i) => &self.unit.globals[i].ty,
            Var::Local(i) => &self.current().locals[i].ty,
            Var::Param(i) => &self.current().params[i].ty,
        }
    }

    pub fn current(&self) -> &Function {
        &self.unit.functions[self.func.as_ref().expect("in a function").index]
    }

    fn current_mut(&mut self) -> &mut Function {
        let index = self.func.as_ref().expect("in a function").index;
        &mut self.unit.functions[index]
    }

    pub fn in_function(&self) -> bool {
        self.func.is_some()
    }

    /// Whether `var` is a 1.00 loop's index, which its body may not
    /// change.
    pub fn is_loop_index(&self, var: Var) -> bool {
        match (var, &self.func) {
            (Var::Local(i), Some(f)) => f.loop_indices.contains(&i),
            _ => false,
        }
    }

    /// Notes that the function being read calls user function `index`.
    pub fn note_call(&mut self, index: usize) {
        if self.func.is_some() {
            let calls = &mut self.current_mut().calls;
            if !calls.contains(&index) {
                calls.push(index);
            }
        }
    }

    // ---- The built-in variables ----

    fn builtin(&mut self, name: &str, ty: Type, kind: GlobalKind, value: Option<Vec<Comp>>) {
        let index = self.unit.globals.len();
        self.unit.globals.push(Global {
            name: name.to_owned(),
            ty,
            kind,
            init: None,
            value,
            used: false,
            invariant: false,
        });
        self.scopes[0].insert(name.to_owned(), Symbol::Var(Var::Global(index)));
    }

    fn builtin_variables(&mut self, options: &Options) {
        let v3 = self.ctx.version == 300;
        let int = |v: i32| Some(vec![Comp::I(v)]);
        let constants: Vec<(&str, i32)> = vec![
            ("gl_MaxVertexAttribs", options.max_vertex_attribs as i32),
            (
                "gl_MaxVertexUniformVectors",
                options.max_vertex_uniform_vectors as i32,
            ),
            (
                "gl_MaxVertexTextureImageUnits",
                options.max_vertex_texture_units as i32,
            ),
            (
                "gl_MaxCombinedTextureImageUnits",
                options.max_combined_texture_units as i32,
            ),
            ("gl_MaxTextureImageUnits", options.max_texture_units as i32),
            (
                "gl_MaxFragmentUniformVectors",
                options.max_fragment_uniform_vectors as i32,
            ),
            (
                "gl_MaxDrawBuffers",
                if v3 || self.ctx.draw_buffers {
                    options.max_draw_buffers as i32
                } else {
                    1
                },
            ),
        ];
        for (name, value) in constants {
            self.builtin(name, INT, GlobalKind::Const, int(value));
        }
        if v3 {
            self.builtin(
                "gl_MaxVertexOutputVectors",
                INT,
                GlobalKind::Const,
                int(options.max_varying_vectors as i32),
            );
            self.builtin(
                "gl_MaxFragmentInputVectors",
                INT,
                GlobalKind::Const,
                int(options.max_varying_vectors as i32),
            );
            self.builtin("gl_MinProgramTexelOffset", INT, GlobalKind::Const, int(-8));
            self.builtin("gl_MaxProgramTexelOffset", INT, GlobalKind::Const, int(7));
        } else {
            self.builtin(
                "gl_MaxVaryingVectors",
                INT,
                GlobalKind::Const,
                int(options.max_varying_vectors as i32),
            );
        }
        // gl_DepthRange, a struct of three floats.
        let depth_range = self.unit.structs.len();
        self.unit.structs.push(StructDef {
            name: "gl_DepthRangeParameters".to_owned(),
            fields: vec![
                ("near".to_owned(), FLOAT),
                ("far".to_owned(), FLOAT),
                ("diff".to_owned(), FLOAT),
            ],
        });
        self.builtin(
            "gl_DepthRange",
            Type::Struct(depth_range),
            GlobalKind::Builtin(BuiltinVar::DepthRange),
            None,
        );
        let vec4 = Type::Vector(Scalar::Float, 4);
        match self.ctx.stage {
            Stage::Vertex => {
                self.builtin(
                    "gl_Position",
                    vec4,
                    GlobalKind::Builtin(BuiltinVar::Position),
                    None,
                );
                self.builtin(
                    "gl_PointSize",
                    FLOAT,
                    GlobalKind::Builtin(BuiltinVar::PointSize),
                    None,
                );
                if v3 {
                    self.builtin(
                        "gl_VertexID",
                        INT,
                        GlobalKind::Builtin(BuiltinVar::VertexId),
                        None,
                    );
                    self.builtin(
                        "gl_InstanceID",
                        INT,
                        GlobalKind::Builtin(BuiltinVar::InstanceId),
                        None,
                    );
                }
            }
            Stage::Fragment => {
                self.builtin(
                    "gl_FragCoord",
                    vec4.clone(),
                    GlobalKind::Builtin(BuiltinVar::FragCoord),
                    None,
                );
                self.builtin(
                    "gl_FrontFacing",
                    BOOL,
                    GlobalKind::Builtin(BuiltinVar::FrontFacing),
                    None,
                );
                self.builtin(
                    "gl_PointCoord",
                    Type::Vector(Scalar::Float, 2),
                    GlobalKind::Builtin(BuiltinVar::PointCoord),
                    None,
                );
                if !v3 {
                    self.builtin(
                        "gl_FragColor",
                        vec4.clone(),
                        GlobalKind::Builtin(BuiltinVar::FragColor),
                        None,
                    );
                    let buffers = if self.ctx.draw_buffers {
                        options.max_draw_buffers
                    } else {
                        1
                    };
                    self.builtin(
                        "gl_FragData",
                        Type::Array(Box::new(vec4), buffers),
                        GlobalKind::Builtin(BuiltinVar::FragData),
                        None,
                    );
                    if self.ctx.frag_depth {
                        self.builtin(
                            "gl_FragDepthEXT",
                            FLOAT,
                            GlobalKind::Builtin(BuiltinVar::FragDepth),
                            None,
                        );
                    }
                } else {
                    self.builtin(
                        "gl_FragDepth",
                        FLOAT,
                        GlobalKind::Builtin(BuiltinVar::FragDepth),
                        None,
                    );
                }
            }
        }
    }

    // ---- The shader ----

    pub fn unit(mut self) -> R<(Unit, Vec<crate::Block>)> {
        while self.peek().is_some() {
            self.external()?;
        }
        let line = self.line();
        let main = self
            .functions
            .get("main")
            .and_then(|overloads| overloads.first().copied())
            .filter(|&i| self.unit.functions[i].body.is_some())
            .ok_or_else(|| Error::new(line, "", "Missing main()"))?;
        self.unit.main = main;
        // Every function called is defined; none calls itself, even
        // through others.
        for (index, function) in self.unit.functions.iter().enumerate() {
            if function.body.is_none() && self.called(index) {
                return Err(Error::new(
                    line,
                    &function.name,
                    "Function declared but not defined",
                ));
            }
        }
        if let Some(name) = self.recursion() {
            return Err(Error::new(line, &name, "Function recursion detected"));
        }
        Ok((self.unit, self.blocks))
    }

    fn called(&self, index: usize) -> bool {
        self.unit.functions.iter().any(|f| f.calls.contains(&index))
    }

    /// A function that calls itself, through others or not.
    fn recursion(&self) -> Option<String> {
        // Depth-first, from each function: 0 unseen, 1 on the path,
        // 2 done.
        let n = self.unit.functions.len();
        let mut state = vec![0u8; n];
        fn visit(f: usize, functions: &[Function], state: &mut [u8]) -> Option<usize> {
            if state[f] == 1 {
                return Some(f);
            }
            if state[f] == 2 {
                return None;
            }
            state[f] = 1;
            for &callee in &functions[f].calls {
                if let Some(found) = visit(callee, functions, state) {
                    return Some(found);
                }
            }
            state[f] = 2;
            None
        }
        (0..n).find_map(|f| {
            visit(f, &self.unit.functions, &mut state).map(|i| self.unit.functions[i].name.clone())
        })
    }

    fn external(&mut self) -> R<()> {
        self.declared = true;
        if self.eat_word("precision") {
            return self.precision_statement();
        }
        // `invariant gl_Position;` and its kin.
        if self.is_word("invariant")
            && self.peek_at(1).and_then(Token::ident).is_some()
            && self.peek_at(2).is_some_and(|t| t.is(";") || t.is(","))
        {
            self.at += 1;
            loop {
                let token = self.next()?;
                let name = token.ident().unwrap_or("").to_owned();
                match self.lookup_var(&name) {
                    Some(Var::Global(g)) => {
                        let kind = self.unit.globals[g].kind.clone();
                        let allowed = matches!(kind, GlobalKind::Varying { output: true, .. })
                            || (self.ctx.stage == Stage::Vertex
                                && matches!(
                                    kind,
                                    GlobalKind::Builtin(
                                        BuiltinVar::Position | BuiltinVar::PointSize
                                    )
                                ))
                            || (self.ctx.version == 100
                                && matches!(kind, GlobalKind::Varying { .. }))
                            || (self.ctx.stage == Stage::Fragment
                                && matches!(
                                    kind,
                                    GlobalKind::Builtin(
                                        BuiltinVar::FragCoord
                                            | BuiltinVar::PointCoord
                                            | BuiltinVar::FrontFacing
                                    )
                                ));
                        if !allowed {
                            return Err(Error::new(
                                token.line,
                                &name,
                                "Cannot be qualified as invariant.",
                            ));
                        }
                        self.unit.globals[g].invariant = true;
                    }
                    _ => return Err(Error::new(token.line, &name, "undeclared identifier")),
                }
                if self.eat(";") {
                    return Ok(());
                }
                self.expect(",")?;
            }
        }
        let quals = self.qualifiers()?;
        // A uniform block (GLSL ES 3.00).
        if quals.storage == Some("uniform")
            && self.ctx.version == 300
            && self.peek().and_then(Token::ident).is_some()
            && self.peek_at(1).is_some_and(|t| t.is("{"))
        {
            return self.uniform_block(quals);
        }
        // `layout(...) uniform;` alone, the blocks' default layout.
        if quals.storage.is_some() && self.is(";") {
            self.at += 1;
            return Ok(());
        }
        let line = self.line();
        let ty = self.type_specifier(&quals)?;
        if self.eat(";") {
            // A struct's declaration alone.
            return Ok(());
        }
        let (name, name_line) = self.name()?;
        if self.is("(") {
            if quals.storage.is_some() && quals.storage != Some("const") || quals.invariant {
                return Err(Error::new(
                    line,
                    &name,
                    "no qualifiers allowed for function return",
                ));
            }
            return self.function(ty, name, name_line);
        }
        self.declarators(ty, quals, name, name_line, true)
    }

    fn precision_statement(&mut self) -> R<()> {
        let token = self.next()?;
        let precision = match token.ident() {
            Some("lowp") => "lowp",
            Some("mediump") => "mediump",
            Some("highp") => "highp",
            _ => return Err(Error::new(token.line, &token.text(), "syntax error")),
        };
        let ty_token = self.next()?;
        let name = ty_token.ident().unwrap_or("").to_owned();
        let allowed = name == "float"
            || name == "int"
            || matches!(
                name.as_str(),
                "sampler2D"
                    | "samplerCube"
                    | "sampler3D"
                    | "sampler2DShadow"
                    | "samplerCubeShadow"
                    | "sampler2DArray"
                    | "sampler2DArrayShadow"
                    | "isampler2D"
                    | "isampler3D"
                    | "isamplerCube"
                    | "isampler2DArray"
                    | "usampler2D"
                    | "usampler3D"
                    | "usamplerCube"
                    | "usampler2DArray"
            ) && (self.ctx.version == 300
                || matches!(name.as_str(), "sampler2D" | "samplerCube"));
        if !allowed {
            return Err(Error::new(
                ty_token.line,
                &name,
                "illegal type argument for default precision qualifier",
            ));
        }
        self.expect(";")?;
        self.precisions
            .last_mut()
            .expect("a scope")
            .insert(name, precision);
        Ok(())
    }

    /// Whether a default precision is in force for type `name`.
    fn has_default_precision(&self, name: &str) -> bool {
        if self.precisions.iter().any(|p| p.contains_key(name)) {
            return true;
        }
        // Both stages' int is given a default, and sampler2D's and
        // samplerCube's; float only the vertex shader's (GLSL ES
        // 4.5.3). The other samplers have none.
        match name {
            "int" | "uint" | "bool" | "sampler2D" | "samplerCube" => true,
            "float" => self.ctx.stage == Stage::Vertex,
            _ => false,
        }
    }

    /// Refuses a type without a precision where its stage has no default
    /// for it (a fragment shader's floats, GLSL ES 4.5.3).
    fn check_precision(&self, ty: &Type, quals: &Qualifiers, line: u32) -> R<()> {
        if quals.precision.is_some() {
            return Ok(());
        }
        let name = match self.basic(ty) {
            Some(Type::Scalar(Scalar::Float))
            | Some(Type::Vector(Scalar::Float, _))
            | Some(Type::Matrix(..)) => "float".to_owned(),
            Some(Type::Sampler(s)) => sampler_name(&s).to_owned(),
            _ => return Ok(()),
        };
        if self.has_default_precision(&name) {
            return Ok(());
        }
        Err(Error::new(
            line,
            "",
            &format!("No precision specified for ({name})"),
        ))
    }

    /// The scalar, vector, matrix or sampler under arrays, for precision.
    fn basic(&self, ty: &Type) -> Option<Type> {
        match ty {
            Type::Array(element, _) => self.basic(element),
            Type::Struct(_) | Type::Void => None,
            other => Some(other.clone()),
        }
    }

    fn qualifiers(&mut self) -> R<Qualifiers> {
        let mut q = Qualifiers {
            line: self.line(),
            ..Qualifiers::default()
        };
        let v3 = self.ctx.version == 300;
        loop {
            let Some(word) = self.peek().and_then(Token::ident) else {
                return Ok(q);
            };
            match word {
                "const" | "uniform" => {
                    if q.storage.is_some() {
                        return Err(self.syntax());
                    }
                    q.storage = Some(if word == "const" { "const" } else { "uniform" });
                }
                "attribute" | "varying" if !v3 => {
                    if q.storage.is_some() {
                        return Err(self.syntax());
                    }
                    q.storage = Some(if word == "attribute" {
                        "attribute"
                    } else {
                        "varying"
                    });
                }
                "in" | "out" if v3 && self.func.is_none() => {
                    if q.storage.is_some() {
                        return Err(self.syntax());
                    }
                    q.storage = Some(if word == "in" { "in" } else { "out" });
                }
                "invariant" => q.invariant = true,
                "flat" if v3 => q.flat = true,
                "smooth" if v3 => q.smooth = true,
                "centroid" if v3 => q.centroid = true,
                "lowp" | "mediump" | "highp" => {
                    q.precision = Some(match word {
                        "lowp" => "lowp",
                        "mediump" => "mediump",
                        _ => "highp",
                    });
                }
                "layout" if v3 => {
                    self.at += 1;
                    self.expect("(")?;
                    loop {
                        let token = self.next()?;
                        let name = token.ident().unwrap_or("").to_owned();
                        if self.eat("=") {
                            let value = self.next()?;
                            let Tok::Int(text) = &value.tok else {
                                return Err(Error::new(
                                    value.line,
                                    &value.text(),
                                    "invalid layout qualifier",
                                ));
                            };
                            let number = int_literal(text).map(|(v, _)| v).ok_or_else(|| {
                                Error::new(value.line, text, "invalid layout qualifier")
                            })?;
                            if name != "location" {
                                return Err(Error::new(
                                    token.line,
                                    &name,
                                    "invalid layout qualifier",
                                ));
                            }
                            q.location = Some(number);
                        } else {
                            match name.as_str() {
                                "std140" | "shared" | "packed" | "row_major" | "column_major" => {
                                    q.block_layout.push(name);
                                }
                                _ => {
                                    return Err(Error::new(
                                        token.line,
                                        &name,
                                        "invalid layout qualifier",
                                    ));
                                }
                            }
                        }
                        if self.eat(")") {
                            break;
                        }
                        self.expect(",")?;
                    }
                    continue;
                }
                _ => return Ok(q),
            }
            self.at += 1;
        }
    }

    /// A type, a struct's definition with it.
    pub fn type_specifier(&mut self, quals: &Qualifiers) -> R<Type> {
        let token = self.next()?;
        let Some(word) = token.ident() else {
            return Err(Error::new(token.line, &token.text(), "syntax error"));
        };
        if word == "struct" {
            return self.struct_specifier(quals);
        }
        let base = self
            .basic_type(word)
            .ok_or_else(|| Error::new(token.line, word, "syntax error"))?;
        // `float[3] x;` (GLSL ES 3.00).
        if self.is("[") && self.ctx.version == 300 {
            self.at += 1;
            let size = self.array_size()?;
            return Ok(Type::Array(Box::new(base), size));
        }
        Ok(base)
    }

    /// A type's name: a basic type or a struct's.
    pub fn basic_type(&self, word: &str) -> Option<Type> {
        let v3 = self.ctx.version == 300;
        let f = Scalar::Float;
        let sampler = |dim, shadow, sample| {
            Some(Type::Sampler(Sampler {
                dim,
                shadow,
                sample,
            }))
        };
        let ty = match word {
            "void" => Type::Void,
            "float" => FLOAT,
            "int" => INT,
            "bool" => BOOL,
            "uint" if v3 => UINT,
            "vec2" => Type::Vector(f, 2),
            "vec3" => Type::Vector(f, 3),
            "vec4" => Type::Vector(f, 4),
            "ivec2" => Type::Vector(Scalar::Int, 2),
            "ivec3" => Type::Vector(Scalar::Int, 3),
            "ivec4" => Type::Vector(Scalar::Int, 4),
            "bvec2" => Type::Vector(Scalar::Bool, 2),
            "bvec3" => Type::Vector(Scalar::Bool, 3),
            "bvec4" => Type::Vector(Scalar::Bool, 4),
            "uvec2" if v3 => Type::Vector(Scalar::Uint, 2),
            "uvec3" if v3 => Type::Vector(Scalar::Uint, 3),
            "uvec4" if v3 => Type::Vector(Scalar::Uint, 4),
            "mat2" => Type::Matrix(2, 2),
            "mat3" => Type::Matrix(3, 3),
            "mat4" => Type::Matrix(4, 4),
            "mat2x2" if v3 => Type::Matrix(2, 2),
            "mat2x3" if v3 => Type::Matrix(2, 3),
            "mat2x4" if v3 => Type::Matrix(2, 4),
            "mat3x2" if v3 => Type::Matrix(3, 2),
            "mat3x3" if v3 => Type::Matrix(3, 3),
            "mat3x4" if v3 => Type::Matrix(3, 4),
            "mat4x2" if v3 => Type::Matrix(4, 2),
            "mat4x3" if v3 => Type::Matrix(4, 3),
            "mat4x4" if v3 => Type::Matrix(4, 4),
            "sampler2D" => return sampler(Dim::D2, false, f),
            "samplerCube" => return sampler(Dim::Cube, false, f),
            "sampler3D" if v3 => return sampler(Dim::D3, false, f),
            "sampler2DArray" if v3 => return sampler(Dim::D2Array, false, f),
            "sampler2DShadow" if v3 => return sampler(Dim::D2, true, f),
            "samplerCubeShadow" if v3 => return sampler(Dim::Cube, true, f),
            "sampler2DArrayShadow" if v3 => return sampler(Dim::D2Array, true, f),
            "isampler2D" if v3 => return sampler(Dim::D2, false, Scalar::Int),
            "isampler3D" if v3 => return sampler(Dim::D3, false, Scalar::Int),
            "isamplerCube" if v3 => return sampler(Dim::Cube, false, Scalar::Int),
            "isampler2DArray" if v3 => return sampler(Dim::D2Array, false, Scalar::Int),
            "usampler2D" if v3 => return sampler(Dim::D2, false, Scalar::Uint),
            "usampler3D" if v3 => return sampler(Dim::D3, false, Scalar::Uint),
            "usamplerCube" if v3 => return sampler(Dim::Cube, false, Scalar::Uint),
            "usampler2DArray" if v3 => return sampler(Dim::D2Array, false, Scalar::Uint),
            other => return self.lookup_struct(other).map(Type::Struct),
        };
        Some(ty)
    }

    fn struct_specifier(&mut self, quals: &Qualifiers) -> R<Type> {
        let line = self.line();
        let name = if self.is("{") {
            // An anonymous struct: named for the WGSL alone.
            format!("webgl_anonymous{}", self.unit.structs.len())
        } else {
            self.name()?.0
        };
        self.expect("{")?;
        self.deeper()?;
        let mut fields: Vec<(String, Type)> = Vec::new();
        while !self.eat("}") {
            let field_quals = self.qualifiers()?;
            if field_quals.storage.is_some()
                || field_quals.invariant
                || field_quals.location.is_some()
            {
                return Err(Error::new(
                    field_quals.line,
                    "",
                    "invalid qualifier on a struct member",
                ));
            }
            let base_line = self.line();
            let base = self.type_specifier(&field_quals)?;
            if base == Type::Void {
                return Err(Error::new(base_line, "void", "illegal use of type 'void'"));
            }
            loop {
                let (field, field_line) = self.name()?;
                let ty = self.array_suffix(base.clone())?;
                if self.ctx.version == 100
                    && matches!(ty, Type::Array(ref e, _) if matches!(**e, Type::Array(..)))
                {
                    return Err(Error::new(
                        field_line,
                        &field,
                        "arrays of arrays are not allowed",
                    ));
                }
                if ty.has_sampler(&self.unit) {
                    // Samplers in structs are uniforms' alone; this
                    // translation does not take them (G-list).
                    return Err(Error::new(
                        field_line,
                        &field,
                        "a sampler in a struct is not supported",
                    ));
                }
                self.check_precision(&ty, &field_quals, field_line)?;
                if fields.iter().any(|(n, _)| *n == field) {
                    return Err(Error::new(
                        field_line,
                        &field,
                        "duplicate field name in structure",
                    ));
                }
                fields.push((field, ty));
                if self.eat(";") {
                    break;
                }
                self.expect(",")?;
            }
        }
        self.shallower();
        if fields.is_empty() {
            return Err(Error::new(line, &name, "structure has no fields"));
        }
        let _ = quals;
        let index = self.unit.structs.len();
        self.unit.structs.push(StructDef {
            name: name.clone(),
            fields,
        });
        if !name.starts_with("webgl_anonymous") {
            self.declare(&name, Symbol::Struct(index), line)?;
        }
        Ok(Type::Struct(index))
    }

    /// `[n]` after a name: the array's type.
    pub fn array_suffix(&mut self, base: Type) -> R<Type> {
        if !self.eat("[") {
            return Ok(base);
        }
        let size = self.array_size()?;
        let ty = Type::Array(Box::new(base), size);
        if self.is("[") {
            let line = self.line();
            return Err(Error::new(line, "[", "arrays of arrays are not supported"));
        }
        Ok(ty)
    }

    /// An array's size, after its `[`, through its `]`.
    fn array_size(&mut self) -> R<u32> {
        let line = self.line();
        if self.is("]") {
            return Err(Error::new(
                line,
                "]",
                "implicitly sized arrays are not supported",
            ));
        }
        let size = self.expression()?;
        self.expect("]")?;
        let value = match (&size.ty, &size.value) {
            (Type::Scalar(Scalar::Int | Scalar::Uint), Some(value)) => value[0].as_i32(),
            _ => {
                return Err(Error::new(
                    line,
                    "",
                    "array size must be a constant integer expression",
                ));
            }
        };
        if value <= 0 {
            return Err(Error::new(line, "", "array size must be greater than zero"));
        }
        Ok(value as u32)
    }

    // ---- Declarations ----

    /// A declaration's names after its first's: `name`, then `, name`
    /// and their initializers, through its `;`.
    fn declarators(
        &mut self,
        base: Type,
        quals: Qualifiers,
        first: String,
        first_line: u32,
        global: bool,
    ) -> R<()> {
        let mut name = first;
        let mut line = first_line;
        loop {
            let ty = self.array_suffix(base.clone())?;
            let init = if self.eat("=") {
                Some(self.assignment()?)
            } else {
                None
            };
            self.variable(&name, ty, &quals, init, line, global)?;
            if self.eat(";") {
                return Ok(());
            }
            self.expect(",")?;
            let (next, next_line) = self.name()?;
            name = next;
            line = next_line;
        }
    }

    /// Declares one variable.
    fn variable(
        &mut self,
        name: &str,
        ty: Type,
        quals: &Qualifiers,
        init: Option<Expr>,
        line: u32,
        global: bool,
    ) -> R<()> {
        let v3 = self.ctx.version == 300;
        if ty == Type::Void {
            return Err(Error::new(line, name, "illegal use of type 'void'"));
        }
        self.check_precision(&ty, quals, line)?;
        let storage = quals.storage;
        if ty.has_sampler(&self.unit) && storage != Some("uniform") {
            return Err(Error::new(line, name, "samplers must be uniform"));
        }
        if !global
            && matches!(
                storage,
                Some("uniform" | "attribute" | "varying" | "in" | "out")
            )
        {
            return Err(Error::new(
                line,
                name,
                "only const and no storage qualifier are allowed in a function",
            ));
        }
        let element = match &ty {
            Type::Array(e, _) => (**e).clone(),
            other => other.clone(),
        };
        let is_array = matches!(ty, Type::Array(..));
        let kind = match storage {
            Some("const") => GlobalKind::Const,
            Some("uniform") => {
                if init.is_some() {
                    return Err(Error::new(line, name, "cannot initialize a uniform"));
                }
                GlobalKind::Uniform
            }
            Some("attribute") | Some("in") if self.ctx.stage == Stage::Vertex => {
                if self.ctx.stage != Stage::Vertex {
                    return Err(Error::new(
                        line,
                        "attribute",
                        "supported in vertex shaders only",
                    ));
                }
                let numeric = match &element {
                    Type::Scalar(Scalar::Float)
                    | Type::Vector(Scalar::Float, _)
                    | Type::Matrix(..) => true,
                    Type::Scalar(Scalar::Int | Scalar::Uint)
                    | Type::Vector(Scalar::Int | Scalar::Uint, _) => v3,
                    _ => false,
                };
                if !numeric || is_array {
                    return Err(Error::new(
                        line,
                        name,
                        "cannot be used with a structure, an array, a boolean or a sampler",
                    ));
                }
                if init.is_some() {
                    return Err(Error::new(line, name, "cannot initialize an attribute"));
                }
                GlobalKind::Attribute(quals.location)
            }
            Some("attribute") => {
                return Err(Error::new(
                    line,
                    "attribute",
                    "supported in vertex shaders only",
                ));
            }
            Some("varying") | Some("out") if self.ctx.stage == Stage::Vertex => {
                self.check_varying(&ty, &element, quals, line, name)?;
                GlobalKind::Varying {
                    output: true,
                    flat: quals.flat || element.scalar().is_some_and(|s| s != Scalar::Float),
                }
            }
            Some("varying") | Some("in") => {
                self.check_varying(&ty, &element, quals, line, name)?;
                GlobalKind::Varying {
                    output: false,
                    flat: quals.flat || element.scalar().is_some_and(|s| s != Scalar::Float),
                }
            }
            Some("out") => {
                // A fragment shader's output.
                let ok = match &element {
                    Type::Scalar(s) | Type::Vector(s, _) => *s != Scalar::Bool,
                    _ => false,
                };
                if !ok {
                    return Err(Error::new(
                        line,
                        name,
                        "cannot be a matrix, a struct, a boolean or a sampler",
                    ));
                }
                if init.is_some() {
                    return Err(Error::new(line, name, "cannot initialize an output"));
                }
                GlobalKind::FragOut(quals.location)
            }
            _ => GlobalKind::Private,
        };
        if kind == GlobalKind::Const {
            match &init {
                None => {
                    return Err(Error::new(
                        line,
                        name,
                        "variables with qualifier 'const' must be initialized",
                    ));
                }
                Some(e) if e.value.is_none() => {
                    return Err(Error::new(line, "=", "assigning non-constant to 'const'"));
                }
                _ => {}
            }
        }
        if let Some(e) = &init {
            if is_array && self.ctx.version == 100 {
                return Err(Error::new(
                    line,
                    name,
                    "arrays may not be initialized in GLSL ES 1.00",
                ));
            }
            if e.ty != ty {
                return Err(Error::new(
                    line,
                    "=",
                    &format!(
                        "cannot convert from '{}' to '{}'",
                        e.ty.name(&self.unit),
                        ty.name(&self.unit)
                    ),
                ));
            }
            // A global's initializer is constant (GLSL ES 4.3).
            if global && e.value.is_none() && kind == GlobalKind::Private && !self.global_init_ok(e)
            {
                return Err(Error::new(
                    line,
                    "=",
                    "global variable initializers must be constant expressions",
                ));
            }
        }
        let value = if kind == GlobalKind::Const {
            init.as_ref().and_then(|e| e.value.clone())
        } else {
            None
        };
        if global {
            let index = self.unit.globals.len();
            self.unit.globals.push(Global {
                name: name.to_owned(),
                ty,
                kind,
                init,
                value,
                used: false,
                invariant: quals.invariant,
            });
            self.declare(name, Symbol::Var(Var::Global(index)), line)?;
        } else {
            let function = self.current_mut();
            let index = function.locals.len();
            function.locals.push(Local {
                name: name.to_owned(),
                ty,
                value,
            });
            self.declare(name, Symbol::Var(Var::Local(index)), line)?;
            self.pending_decl = Some((index, init));
        }
        Ok(())
    }

    /// Whether a global's initializer that is no constant expression is
    /// allowed: in GLSL ES 1.00 the reference takes one that reads
    /// uniforms and other globals, for older pages; the translation sets
    /// it as `main` begins.
    fn global_init_ok(&self, _e: &Expr) -> bool {
        self.ctx.version == 100
    }

    fn check_varying(
        &self,
        ty: &Type,
        element: &Type,
        quals: &Qualifiers,
        line: u32,
        name: &str,
    ) -> R<()> {
        let v3 = self.ctx.version == 300;
        let ok = match element {
            Type::Scalar(Scalar::Float) | Type::Vector(Scalar::Float, _) | Type::Matrix(..) => true,
            Type::Scalar(Scalar::Int | Scalar::Uint)
            | Type::Vector(Scalar::Int | Scalar::Uint, _) => v3,
            Type::Struct(_) => false,
            _ => false,
        };
        if !ok {
            return Err(Error::new(
                line,
                name,
                "cannot be a boolean, a struct or a sampler",
            ));
        }
        if v3 && element.scalar().is_some_and(|s| s != Scalar::Float) && !quals.flat {
            return Err(Error::new(line, name, "must use 'flat' interpolation here"));
        }
        let _ = ty;
        Ok(())
    }

    fn uniform_block(&mut self, quals: Qualifiers) -> R<()> {
        let (block_name, line) = self.name()?;
        self.expect("{")?;
        let mut members: Vec<(String, Type, bool)> = Vec::new();
        let block_major = quals
            .block_layout
            .iter()
            .rev()
            .find_map(|l| match l.as_str() {
                "row_major" => Some(true),
                "column_major" => Some(false),
                _ => None,
            });
        while !self.eat("}") {
            let mq = self.qualifiers()?;
            let base = self.type_specifier(&mq)?;
            let row_major = mq
                .block_layout
                .iter()
                .rev()
                .find_map(|l| match l.as_str() {
                    "row_major" => Some(true),
                    "column_major" => Some(false),
                    _ => None,
                })
                .or(block_major)
                .unwrap_or(false);
            loop {
                let (member, member_line) = self.name()?;
                let ty = self.array_suffix(base.clone())?;
                if ty.has_sampler(&self.unit) {
                    return Err(Error::new(
                        member_line,
                        &member,
                        "samplers are not allowed in uniform blocks",
                    ));
                }
                if members.iter().any(|(n, _, _)| *n == member) {
                    return Err(Error::new(member_line, &member, "redefinition"));
                }
                members.push((member, ty, row_major));
                if self.eat(";") {
                    break;
                }
                self.expect(",")?;
            }
        }
        let instance = if self.is(";") {
            None
        } else {
            let (instance, instance_line) = self.name()?;
            let array = if self.eat("[") {
                let size = self.array_size()?;
                Some(size)
            } else {
                None
            };
            Some((instance, instance_line, array))
        };
        self.expect(";")?;
        if self.blocks.iter().any(|b| b.name == block_name) {
            return Err(Error::new(line, &block_name, "redefinition"));
        }
        let block_index = self.blocks.len();
        // The members, as a struct the block's global is.
        let struct_index = self.unit.structs.len();
        self.unit.structs.push(StructDef {
            name: format!("webgl_block_{block_name}"),
            fields: members
                .iter()
                .map(|(n, t, _)| (n.clone(), t.clone()))
                .collect(),
        });
        let block_ty = Type::Struct(struct_index);
        let (global_ty, instance_name, array) = match &instance {
            Some((n, _, Some(size))) => (
                Type::Array(Box::new(block_ty), *size),
                Some(n.clone()),
                Some(*size),
            ),
            Some((n, _, None)) => (block_ty, Some(n.clone()), None),
            None => (block_ty, None, None),
        };
        let global = self.unit.globals.len();
        self.unit.globals.push(Global {
            name: format!("webgl_block{block_index}"),
            ty: global_ty,
            kind: GlobalKind::Uniform,
            init: None,
            value: None,
            used: false,
            invariant: false,
        });
        match &instance {
            Some((n, l, _)) => self.declare(n, Symbol::Var(Var::Global(global)), *l)?,
            None => {
                // Its members are names of the global scope, each a
                // field of the block's global.
                for (i, (member, _, _)) in members.iter().enumerate() {
                    if self.lookup(member).is_some() {
                        return Err(Error::new(line, member, "redefinition"));
                    }
                    self.block_members.insert(member.clone(), (global, i));
                }
            }
        }
        self.blocks.push(crate::Block {
            name: block_name,
            instance: instance_name,
            array,
            members: members
                .into_iter()
                .map(|(name, ty, row_major)| crate::BlockMember {
                    name,
                    ty,
                    row_major,
                })
                .collect(),
            global,
            used: false,
        });
        Ok(())
    }

    // ---- Functions ----

    fn function(&mut self, ret: Type, name: String, line: u32) -> R<()> {
        if crate::builtins::is_builtin_name(&name, &self.ctx) {
            return Err(Error::new(
                line,
                &name,
                "Name of a built-in function cannot be redeclared as function",
            ));
        }
        if self.scopes[0].contains_key(&name) {
            return Err(Error::new(line, &name, "redefinition"));
        }
        if ret.has_sampler(&self.unit) {
            return Err(Error::new(
                line,
                &name,
                "Function return type cannot be a sampler",
            ));
        }
        if self.ctx.version == 100 && matches!(ret, Type::Array(..)) {
            return Err(Error::new(
                line,
                &name,
                "functions cannot return arrays in GLSL ES 1.00",
            ));
        }
        self.expect("(")?;
        let mut params: Vec<Param> = Vec::new();
        if self.is_word("void") && self.peek_at(1).is_some_and(|t| t.is(")")) {
            self.at += 1;
        }
        if !self.eat(")") {
            loop {
                let mut mode = ParamMode::In;
                let mut constant = false;
                let mut precision = None;
                loop {
                    match self.peek().and_then(Token::ident) {
                        Some("const") => constant = true,
                        Some("in") => mode = ParamMode::In,
                        Some("out") => mode = ParamMode::Out,
                        Some("inout") => mode = ParamMode::InOut,
                        Some("lowp" | "mediump" | "highp") => precision = Some("highp"),
                        _ => break,
                    }
                    self.at += 1;
                }
                if constant && mode != ParamMode::In {
                    return Err(Error::new(
                        self.line(),
                        "const",
                        "qualifier not allowed with out or inout",
                    ));
                }
                let quals = Qualifiers {
                    precision,
                    ..Qualifiers::default()
                };
                let param_line = self.line();
                let base = self.type_specifier(&quals)?;
                if base == Type::Void {
                    return Err(Error::new(param_line, "void", "illegal use of type 'void'"));
                }
                let (param_name, ty) = if self.peek().and_then(Token::ident).is_some() {
                    let (n, _) = self.name()?;
                    (n, self.array_suffix(base)?)
                } else {
                    (
                        format!("webgl_unnamed{}", params.len()),
                        self.array_suffix(base)?,
                    )
                };
                self.check_precision(&ty, &quals, param_line)?;
                if ty.has_sampler(&self.unit) && mode != ParamMode::In {
                    return Err(Error::new(
                        param_line,
                        &param_name,
                        "samplers cannot be output parameters",
                    ));
                }
                if matches!(ty, Type::Array(ref e, _) if e.has_sampler(&self.unit)) {
                    return Err(Error::new(
                        param_line,
                        &param_name,
                        "arrays of samplers as parameters are not supported",
                    ));
                }
                params.push(Param {
                    name: param_name,
                    ty,
                    mode,
                });
                if self.eat(")") {
                    break;
                }
                self.expect(",")?;
            }
        }
        if name == "main" && (ret != Type::Void || !params.is_empty()) {
            return Err(Error::new(
                line,
                "main",
                "function cannot take any parameter(s) and must return void",
            ));
        }
        // Its earlier declaration, if any: the same parameters' types.
        let overloads = self.functions.get(&name).cloned().unwrap_or_default();
        let earlier = overloads.iter().copied().find(|&i| {
            let f = &self.unit.functions[i];
            f.params.len() == params.len()
                && f.params.iter().zip(&params).all(|(a, b)| a.ty == b.ty)
        });
        let index = match earlier {
            Some(i) => {
                let f = &self.unit.functions[i];
                if f.ret != ret {
                    return Err(Error::new(
                        line,
                        &name,
                        "overloaded functions must have the same return type",
                    ));
                }
                if f.params.iter().zip(&params).any(|(a, b)| a.mode != b.mode) {
                    return Err(Error::new(
                        line,
                        &name,
                        "function must have the same parameter qualifiers in all of its declarations",
                    ));
                }
                if f.body.is_some() && self.is("{") {
                    return Err(Error::new(line, &name, "function already has a body"));
                }
                // The definition's names are the ones its body uses.
                self.unit.functions[i].params = params;
                i
            }
            None => {
                if name == "main" && !overloads.is_empty() {
                    return Err(Error::new(line, "main", "function cannot be overloaded"));
                }
                let i = self.unit.functions.len();
                self.unit.functions.push(Function {
                    name: name.clone(),
                    params,
                    ret: ret.clone(),
                    locals: Vec::new(),
                    body: None,
                    calls: Vec::new(),
                });
                self.functions.entry(name.clone()).or_default().push(i);
                i
            }
        };
        if self.eat(";") {
            return Ok(());
        }
        if !self.is("{") {
            return Err(self.syntax());
        }
        // Its body, its parameters a scope around it.
        self.func = Some(FnCtx {
            index,
            ret: ret.clone(),
            loops: 0,
            switches: 0,
            loop_indices: Vec::new(),
        });
        self.push_scope();
        let params = self.unit.functions[index].params.clone();
        for (i, param) in params.iter().enumerate() {
            if param.name.starts_with("webgl_unnamed") {
                continue;
            }
            self.declare(&param.name, Symbol::Var(Var::Param(i)), line)?;
        }
        self.at += 1;
        let body = self.statements_until_brace()?;
        self.pop_scope();
        if ret != Type::Void && !returns(&body) && name != "main" {
            // GLSL leaves a missing return undefined; WGSL refuses it:
            // the translation returns a zero value then.
        }
        self.unit.functions[index].body = Some(body);
        self.func = None;
        Ok(())
    }

    // ---- Statements ----

    /// The statements through a `}`, in the scope a caller opened.
    fn statements_until_brace(&mut self) -> R<Vec<Stmt>> {
        let mut out = Vec::new();
        while !self.eat("}") {
            if self.peek().is_none() {
                return Err(self.syntax());
            }
            self.statement(&mut out)?;
        }
        Ok(out)
    }

    fn block(&mut self) -> R<Vec<Stmt>> {
        self.expect("{")?;
        self.deeper()?;
        self.push_scope();
        let body = self.statements_until_brace()?;
        self.pop_scope();
        self.shallower();
        Ok(body)
    }

    /// A statement that is a scope of its own (an `if`'s, a loop's
    /// body): a block, or one statement.
    fn scoped(&mut self) -> R<Vec<Stmt>> {
        if self.is("{") {
            return self.block();
        }
        self.push_scope();
        let mut out = Vec::new();
        self.statement(&mut out)?;
        self.pop_scope();
        Ok(out)
    }

    pub fn statement(&mut self, out: &mut Vec<Stmt>) -> R<()> {
        self.deeper()?;
        let result = self.statement_inner(out);
        self.shallower();
        result
    }

    fn statement_inner(&mut self, out: &mut Vec<Stmt>) -> R<()> {
        let line = self.line();
        let Some(token) = self.peek().cloned() else {
            return Err(self.syntax());
        };
        if token.is("{") {
            let body = self.block()?;
            out.push(Stmt::Block(body));
            return Ok(());
        }
        if token.is(";") {
            self.at += 1;
            return Ok(());
        }
        match token.ident() {
            Some("if") => {
                self.at += 1;
                self.expect("(")?;
                let cond = self.expression()?;
                if cond.ty != BOOL {
                    return Err(Error::new(line, "if", "boolean expression expected"));
                }
                self.expect(")")?;
                let then = self.scoped()?;
                let otherwise = if self.eat_word("else") {
                    Some(self.scoped()?)
                } else {
                    None
                };
                out.push(Stmt::If(cond, then, otherwise));
            }
            Some("for") => {
                self.at += 1;
                self.for_loop(out, line)?;
            }
            Some("while") => {
                self.at += 1;
                if self.ctx.version == 100 {
                    return Err(Error::new(
                        line,
                        "while",
                        "This type of loop is not allowed",
                    ));
                }
                self.expect("(")?;
                let cond = self.expression()?;
                if cond.ty != BOOL {
                    return Err(Error::new(line, "while", "boolean expression expected"));
                }
                self.expect(")")?;
                self.loop_mut(1);
                let body = self.scoped();
                self.loop_mut(-1);
                out.push(Stmt::While(cond, body?));
            }
            Some("do") => {
                self.at += 1;
                if self.ctx.version == 100 {
                    return Err(Error::new(line, "do", "This type of loop is not allowed"));
                }
                self.loop_mut(1);
                let body = self.scoped();
                self.loop_mut(-1);
                let body = body?;
                if !self.eat_word("while") {
                    return Err(self.syntax());
                }
                self.expect("(")?;
                let cond = self.expression()?;
                if cond.ty != BOOL {
                    return Err(Error::new(line, "while", "boolean expression expected"));
                }
                self.expect(")")?;
                self.expect(";")?;
                out.push(Stmt::DoWhile(body, cond));
            }
            Some("switch") if self.ctx.version == 300 => {
                self.at += 1;
                self.switch(out, line)?;
            }
            Some("break") => {
                self.at += 1;
                self.expect(";")?;
                let f = self.func.as_ref().expect("in a function");
                if f.loops == 0 && f.switches == 0 {
                    return Err(Error::new(
                        line,
                        "break",
                        "break statement only allowed in loops and switch statements",
                    ));
                }
                out.push(Stmt::Break);
            }
            Some("continue") => {
                self.at += 1;
                self.expect(";")?;
                if self.func.as_ref().expect("in a function").loops == 0 {
                    return Err(Error::new(
                        line,
                        "continue",
                        "continue statement only allowed in loops",
                    ));
                }
                out.push(Stmt::Continue);
            }
            Some("return") => {
                self.at += 1;
                let ret = self.func.as_ref().expect("in a function").ret.clone();
                if self.eat(";") {
                    if ret != Type::Void {
                        return Err(Error::new(
                            line,
                            "return",
                            "non-void function must return a value",
                        ));
                    }
                    out.push(Stmt::Return(None));
                } else {
                    let value = self.expression()?;
                    self.expect(";")?;
                    if ret == Type::Void {
                        return Err(Error::new(
                            line,
                            "return",
                            "void function cannot return a value",
                        ));
                    }
                    if value.ty != ret {
                        return Err(Error::new(
                            line,
                            "return",
                            "function return is not matching type:",
                        ));
                    }
                    out.push(Stmt::Return(Some(value)));
                }
            }
            Some("discard") => {
                self.at += 1;
                self.expect(";")?;
                if self.ctx.stage != Stage::Fragment {
                    return Err(Error::new(
                        line,
                        "discard",
                        "discard supported in fragment shaders only",
                    ));
                }
                out.push(Stmt::Discard);
            }
            Some("precision") => {
                self.at += 1;
                self.precision_statement()?;
            }
            Some("case" | "default") if self.ctx.version == 300 => {
                return Err(Error::new(
                    line,
                    &token.text(),
                    "case labels need to be inside switch statements",
                ));
            }
            _ if self.at_declaration() => {
                self.local_declaration(out)?;
            }
            _ => {
                let e = self.expression()?;
                self.expect(";")?;
                out.push(Stmt::Expr(e));
            }
        }
        Ok(())
    }

    fn loop_mut(&mut self, by: i32) {
        let f = self.func.as_mut().expect("in a function");
        f.loops = (f.loops as i32 + by) as usize;
    }

    /// Whether a declaration begins here: a type's name or a qualifier,
    /// not a constructor's call.
    fn at_declaration(&self) -> bool {
        match self.peek().and_then(Token::ident) {
            Some("const" | "lowp" | "mediump" | "highp" | "struct" | "invariant") => true,
            Some(word) if self.basic_type(word).is_some() && self.lookup_var(word).is_none() => {
                // `vec4(...)` is a constructor; `vec4 x` and `vec4[2] x`
                // a declaration.
                !self.peek_at(1).is_some_and(|t| t.is("(") || t.is("."))
                    && !(self.peek_at(1).is_some_and(|t| t.is("["))
                        && self.array_constructor_ahead())
            }
            _ => false,
        }
    }

    /// Whether `T[n](...)` is ahead, an array's constructor.
    fn array_constructor_ahead(&self) -> bool {
        let mut i = 1;
        let mut depth = 0;
        while let Some(t) = self.peek_at(i) {
            if t.is("[") {
                depth += 1;
            } else if t.is("]") {
                depth -= 1;
                if depth == 0 {
                    return self.peek_at(i + 1).is_some_and(|t| t.is("("));
                }
            }
            i += 1;
        }
        false
    }

    fn local_declaration(&mut self, out: &mut Vec<Stmt>) -> R<()> {
        let quals = self.qualifiers()?;
        if quals.storage.is_some() && quals.storage != Some("const") {
            let line = quals.line;
            return Err(Error::new(
                line,
                quals.storage.unwrap_or(""),
                "only const and no storage qualifier are allowed in a function",
            ));
        }
        let base = self.type_specifier(&quals)?;
        if self.eat(";") {
            return Ok(());
        }
        let (mut name, mut line) = self.name()?;
        loop {
            let ty = self.array_suffix(base.clone())?;
            let init = if self.eat("=") {
                Some(self.assignment()?)
            } else {
                None
            };
            self.variable(&name, ty, &quals, init, line, false)?;
            if let Some((index, init)) = self.pending_decl.take() {
                out.push(Stmt::Decl(index, init));
            }
            if self.eat(";") {
                return Ok(());
            }
            self.expect(",")?;
            let next = self.name()?;
            name = next.0;
            line = next.1;
        }
    }

    fn for_loop(&mut self, out: &mut Vec<Stmt>, line: u32) -> R<()> {
        self.expect("(")?;
        self.push_scope();
        let mut init = Vec::new();
        if !self.eat(";") {
            if self.at_declaration() {
                self.local_declaration(&mut init)?;
            } else {
                let e = self.expression()?;
                self.expect(";")?;
                init.push(Stmt::Expr(e));
            }
        }
        let cond = if self.is(";") {
            None
        } else {
            let c = self.expression()?;
            if c.ty != BOOL {
                return Err(Error::new(line, "for", "boolean expression expected"));
            }
            Some(c)
        };
        self.expect(";")?;
        let update = if self.is(")") {
            None
        } else {
            Some(self.expression()?)
        };
        self.expect(")")?;
        // GLSL ES 1.00's loops (Appendix A): one index, of int or float,
        // set to a constant, tested against a constant, stepped by a
        // constant, and not changed in the body.
        let index = if self.ctx.version == 100 {
            Some(self.appendix_a_loop(&init, cond.as_ref(), update.as_ref(), line)?)
        } else {
            None
        };
        if let Some(i) = index {
            self.func
                .as_mut()
                .expect("in a function")
                .loop_indices
                .push(i);
        }
        self.loop_mut(1);
        let body = if self.is("{") {
            // The body's block shares the loop's scope's names in GLSL
            // ES 3.00 (its init's are not redeclared).
            self.block()
        } else {
            self.scoped()
        };
        self.loop_mut(-1);
        if index.is_some() {
            self.func
                .as_mut()
                .expect("in a function")
                .loop_indices
                .pop();
        }
        let body = body?;
        self.pop_scope();
        out.push(Stmt::For {
            init,
            cond,
            update,
            body,
        });
        Ok(())
    }

    /// Appendix A's loop, checked: its index's local.
    fn appendix_a_loop(
        &self,
        init: &[Stmt],
        cond: Option<&Expr>,
        update: Option<&Expr>,
        line: u32,
    ) -> R<usize> {
        let bad = |what: &str| Error::new(line, "for", what);
        let index = match init {
            [Stmt::Decl(i, Some(e))] if e.value.is_some() => *i,
            [Stmt::Decl(_, Some(_))] => {
                return Err(bad(
                    "Loop index cannot be initialized with non-constant expression",
                ));
            }
            _ => return Err(bad("Invalid init declaration")),
        };
        let ty = &self.current().locals[index].ty;
        if *ty != INT && *ty != FLOAT {
            return Err(bad("Invalid type for loop index"));
        }
        let is_index = |e: &Expr| matches!(e.kind, ExprKind::Local(i) if i == index);
        match cond.map(|c| &c.kind) {
            Some(ExprKind::Binary(op, a, b))
                if matches!(
                    op,
                    BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::Eq | BinOp::Ne
                ) && is_index(a)
                    && b.value.is_some() => {}
            _ => return Err(bad("Invalid condition")),
        }
        let ok = match update.map(|u| &u.kind) {
            Some(ExprKind::IncDec { target, .. }) => is_index(target),
            Some(ExprKind::Assign(Some(BinOp::Add | BinOp::Sub), target, by)) => {
                is_index(target) && by.value.is_some()
            }
            _ => false,
        };
        if !ok {
            return Err(bad("Invalid expression"));
        }
        Ok(index)
    }

    fn switch(&mut self, out: &mut Vec<Stmt>, line: u32) -> R<()> {
        self.expect("(")?;
        let selector = self.expression()?;
        if selector.ty != INT && selector.ty != UINT {
            return Err(Error::new(
                line,
                "switch",
                "init-expression in a switch statement must be a scalar integer",
            ));
        }
        self.expect(")")?;
        self.expect("{")?;
        self.push_scope();
        self.func.as_mut().expect("in a function").switches += 1;
        let mut items = Vec::new();
        let mut labels: Vec<i64> = Vec::new();
        let mut defaulted = false;
        let result = (|| -> R<()> {
            while !self.eat("}") {
                let label_line = self.line();
                if self.eat_word("case") {
                    let value = self.expression()?;
                    self.expect(":")?;
                    let Some(v) = value.value.as_ref().filter(|_| value.ty == selector.ty) else {
                        return Err(Error::new(
                            label_line,
                            "case",
                            "case label must be a constant integer expression of the selector's type",
                        ));
                    };
                    let v = match v[0] {
                        Comp::I(i) => i64::from(i),
                        Comp::U(u) => i64::from(u),
                        _ => 0,
                    };
                    if labels.contains(&v) {
                        return Err(Error::new(label_line, "case", "duplicate case label"));
                    }
                    labels.push(v);
                    items.push(SwitchItem::Case(v));
                } else if self.eat_word("default") {
                    self.expect(":")?;
                    if defaulted {
                        return Err(Error::new(label_line, "default", "duplicate default label"));
                    }
                    defaulted = true;
                    items.push(SwitchItem::Default);
                } else {
                    if items.is_empty() {
                        return Err(Error::new(
                            label_line,
                            "switch",
                            "statement before the first label",
                        ));
                    }
                    let mut stmts = Vec::new();
                    self.statement(&mut stmts)?;
                    items.extend(stmts.into_iter().map(|s| SwitchItem::Stmt(Box::new(s))));
                }
            }
            Ok(())
        })();
        self.func.as_mut().expect("in a function").switches -= 1;
        self.pop_scope();
        result?;
        if matches!(
            items.last(),
            Some(SwitchItem::Case(_) | SwitchItem::Default)
        ) {
            return Err(Error::new(
                line,
                "switch",
                "label at the end of a switch statement",
            ));
        }
        out.push(Stmt::Switch(selector, items));
        Ok(())
    }
}

/// Whether statements end in a return on every path.
fn returns(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Return(_) | Stmt::Discard => true,
        Stmt::Block(b) => returns(b),
        Stmt::If(_, a, Some(b)) => returns(a) && returns(b),
        _ => false,
    })
}
