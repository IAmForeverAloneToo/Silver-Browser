//! The checked shader: GLSL ES's types and a typed tree of its
//! declarations, statements and expressions, every name resolved and
//! every overload chosen, which the WGSL writer reads.

use crate::Stage;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scalar {
    Float,
    Int,
    Uint,
    Bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dim {
    D2,
    D3,
    Cube,
    D2Array,
}

/// A sampler type: its dimension, whether it compares (a shadow
/// sampler), and what it gives (float, int or uint).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Sampler {
    pub dim: Dim,
    pub shadow: bool,
    pub sample: Scalar,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Void,
    Scalar(Scalar),
    Vector(Scalar, u8),
    /// A matrix of `cols` columns of `rows` floats each.
    Matrix(u8, u8),
    Sampler(Sampler),
    /// A struct, by its index in [`Unit::structs`].
    Struct(usize),
    Array(Box<Type>, u32),
}

pub const FLOAT: Type = Type::Scalar(Scalar::Float);
pub const INT: Type = Type::Scalar(Scalar::Int);
pub const UINT: Type = Type::Scalar(Scalar::Uint);
pub const BOOL: Type = Type::Scalar(Scalar::Bool);

impl Type {
    pub fn vector(scalar: Scalar, size: u8) -> Type {
        if size == 1 {
            Type::Scalar(scalar)
        } else {
            Type::Vector(scalar, size)
        }
    }

    /// The scalar of a scalar, vector or matrix.
    pub fn scalar(&self) -> Option<Scalar> {
        match self {
            Type::Scalar(s) | Type::Vector(s, _) => Some(*s),
            Type::Matrix(..) => Some(Scalar::Float),
            _ => None,
        }
    }

    /// The components of a scalar, a vector or a matrix.
    pub fn components(&self) -> u32 {
        match self {
            Type::Scalar(_) => 1,
            Type::Vector(_, n) => u32::from(*n),
            Type::Matrix(c, r) => u32::from(*c) * u32::from(*r),
            _ => 0,
        }
    }

    /// A vector's size, 1 for a scalar.
    pub fn size(&self) -> u8 {
        match self {
            Type::Scalar(_) => 1,
            Type::Vector(_, n) => *n,
            _ => 0,
        }
    }

    pub fn is_scalar(&self) -> bool {
        matches!(self, Type::Scalar(_))
    }

    pub fn is_vector(&self) -> bool {
        matches!(self, Type::Vector(..))
    }

    /// A scalar or a vector.
    pub fn is_scalar_or_vector(&self) -> bool {
        matches!(self, Type::Scalar(_) | Type::Vector(..))
    }

    pub fn is_matrix(&self) -> bool {
        matches!(self, Type::Matrix(..))
    }

    /// A float scalar or vector (GLSL's genType).
    pub fn is_gen_float(&self) -> bool {
        matches!(
            self,
            Type::Scalar(Scalar::Float) | Type::Vector(Scalar::Float, _)
        )
    }

    /// Whether a sampler is in it: it cannot be assigned or be a
    /// variable's but a uniform's or a parameter's.
    pub fn has_sampler(&self, unit: &Unit) -> bool {
        match self {
            Type::Sampler(_) => true,
            Type::Array(element, _) => element.has_sampler(unit),
            Type::Struct(id) => unit.structs[*id]
                .fields
                .iter()
                .any(|(_, ty)| ty.has_sampler(unit)),
            _ => false,
        }
    }

    pub fn has_array(&self, unit: &Unit) -> bool {
        match self {
            Type::Array(..) => true,
            Type::Struct(id) => unit.structs[*id]
                .fields
                .iter()
                .any(|(_, ty)| ty.has_array(unit)),
            _ => false,
        }
    }

    /// GLSL's name of the type, as errors name it.
    pub fn name(&self, unit: &Unit) -> String {
        match self {
            Type::Void => "void".to_owned(),
            Type::Scalar(s) => scalar_name(*s).to_owned(),
            Type::Vector(s, n) => format!("{}vec{n}", vector_prefix(*s)),
            Type::Matrix(c, r) if c == r => format!("mat{c}"),
            Type::Matrix(c, r) => format!("mat{c}x{r}"),
            Type::Sampler(s) => sampler_name(s).to_owned(),
            Type::Struct(id) => unit.structs[*id].name.clone(),
            Type::Array(element, n) => format!("{}[{n}]", element.name(unit)),
        }
    }
}

fn scalar_name(s: Scalar) -> &'static str {
    match s {
        Scalar::Float => "float",
        Scalar::Int => "int",
        Scalar::Uint => "uint",
        Scalar::Bool => "bool",
    }
}

pub fn vector_prefix(s: Scalar) -> &'static str {
    match s {
        Scalar::Float => "",
        Scalar::Int => "i",
        Scalar::Uint => "u",
        Scalar::Bool => "b",
    }
}

pub fn sampler_name(s: &Sampler) -> &'static str {
    let prefixed = |f: &'static str, i: &'static str, u: &'static str| match s.sample {
        Scalar::Int => i,
        Scalar::Uint => u,
        _ => f,
    };
    match (s.dim, s.shadow) {
        (Dim::D2, false) => prefixed("sampler2D", "isampler2D", "usampler2D"),
        (Dim::D3, _) => prefixed("sampler3D", "isampler3D", "usampler3D"),
        (Dim::Cube, false) => prefixed("samplerCube", "isamplerCube", "usamplerCube"),
        (Dim::D2Array, false) => prefixed("sampler2DArray", "isampler2DArray", "usampler2DArray"),
        (Dim::D2, true) => "sampler2DShadow",
        (Dim::Cube, true) => "samplerCubeShadow",
        (Dim::D2Array, true) => "sampler2DArrayShadow",
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<(String, Type)>,
}

/// A constant's components, flattened: a matrix's columns in turn, a
/// struct's fields, an array's elements.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Comp {
    F(f32),
    I(i32),
    U(u32),
    B(bool),
}

impl Comp {
    pub fn as_f32(self) -> f32 {
        match self {
            Comp::F(v) => v,
            Comp::I(v) => v as f32,
            Comp::U(v) => v as f32,
            Comp::B(v) => f32::from(u8::from(v)),
        }
    }

    pub fn as_i32(self) -> i32 {
        match self {
            Comp::F(v) => v as i32,
            Comp::I(v) => v,
            Comp::U(v) => v as i32,
            Comp::B(v) => i32::from(v),
        }
    }

    pub fn as_u32(self) -> u32 {
        match self {
            Comp::F(v) => v as u32,
            Comp::I(v) => v as u32,
            Comp::U(v) => v,
            Comp::B(v) => u32::from(v),
        }
    }

    pub fn as_bool(self) -> bool {
        match self {
            Comp::F(v) => v != 0.0,
            Comp::I(v) => v != 0,
            Comp::U(v) => v != 0,
            Comp::B(v) => v,
        }
    }

    /// The component as one of `scalar`.
    pub fn to(self, scalar: Scalar) -> Comp {
        match scalar {
            Scalar::Float => Comp::F(self.as_f32()),
            Scalar::Int => Comp::I(self.as_i32()),
            Scalar::Uint => Comp::U(self.as_u32()),
            Scalar::Bool => Comp::B(self.as_bool()),
        }
    }
}

/// How a parameter passes: copied in, out, or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamMode {
    In,
    Out,
    InOut,
}

/// The variables the language gives each stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinVar {
    Position,
    PointSize,
    VertexId,
    InstanceId,
    FragCoord,
    FrontFacing,
    PointCoord,
    FragColor,
    FragData,
    FragDepth,
    DepthRange,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GlobalKind {
    /// A variable of the shader's own.
    Private,
    /// A `const`, its value known.
    Const,
    Uniform,
    /// A vertex shader's input, at the location its layout gives.
    Attribute(Option<u32>),
    /// An output of the vertex shader or an input of the fragment
    /// shader: `flat` when it is not interpolated.
    Varying {
        output: bool,
        flat: bool,
    },
    /// A fragment shader's output (GLSL ES 3.00), at its location.
    FragOut(Option<u32>),
    Builtin(BuiltinVar),
}

#[derive(Clone, Debug)]
pub struct Global {
    pub name: String,
    pub ty: Type,
    pub kind: GlobalKind,
    pub init: Option<Expr>,
    /// A constant's value.
    pub value: Option<Vec<Comp>>,
    /// Whether a function or an initializer reads or writes it.
    pub used: bool,
    pub invariant: bool,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub mode: ParamMode,
}

#[derive(Clone, Debug)]
pub struct Local {
    pub name: String,
    pub ty: Type,
    /// A `const` local's value.
    pub value: Option<Vec<Comp>>,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Type,
    pub locals: Vec<Local>,
    pub body: Option<Vec<Stmt>>,
    /// The user functions it calls, by index.
    pub calls: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Shl,
    Shr,
    BitAnd,
    BitOr,
    BitXor,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Lit(Comp),
    Global(usize),
    Local(usize),
    Param(usize),
    Index(Box<Expr>, Box<Expr>),
    Field(Box<Expr>, usize),
    Swizzle(Box<Expr>, Vec<u8>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Xor(Box<Expr>, Box<Expr>),
    Select(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `a = b`, or `a op= b`.
    Assign(Option<BinOp>, Box<Expr>, Box<Expr>),
    /// `++a`, `a--` and their kin.
    IncDec {
        pre: bool,
        inc: bool,
        target: Box<Expr>,
    },
    Call(usize, Vec<Expr>),
    Builtin(&'static str, Vec<Expr>),
    /// A constructor of the expression's type.
    Construct(Vec<Expr>),
    Comma(Box<Expr>, Box<Expr>),
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
    /// Its value, when it is a constant expression.
    pub value: Option<Vec<Comp>>,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub enum SwitchItem {
    Case(i64),
    Default,
    Stmt(Box<Stmt>),
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Expr(Expr),
    /// A local's declaration, with its initializer.
    Decl(usize, Option<Expr>),
    Block(Vec<Stmt>),
    If(Expr, Vec<Stmt>, Option<Vec<Stmt>>),
    For {
        init: Vec<Stmt>,
        cond: Option<Expr>,
        update: Option<Expr>,
        body: Vec<Stmt>,
    },
    While(Expr, Vec<Stmt>),
    DoWhile(Vec<Stmt>, Expr),
    Switch(Expr, Vec<SwitchItem>),
    Break,
    Continue,
    Return(Option<Expr>),
    Discard,
}

/// A checked shader.
#[derive(Clone, Debug)]
pub struct Unit {
    pub stage: Stage,
    pub version: u32,
    pub structs: Vec<StructDef>,
    pub globals: Vec<Global>,
    pub functions: Vec<Function>,
    /// `main`'s index.
    pub main: usize,
    /// The extensions it enabled.
    pub enabled: Vec<String>,
}

impl Unit {
    pub fn global(&self, name: &str) -> Option<(usize, &Global)> {
        self.globals
            .iter()
            .enumerate()
            .find(|(_, g)| g.name == name)
    }
}
