//! The codec of the messages (M34.2, docs/design/processes.md, "IPC"):
//! integers little-endian, strings and byte strings by a 32-bit length,
//! every enum by a tag the decoder checks, every length bounded by the
//! bytes left before anything is allocated. The decoder takes any bytes
//! and answers a value or [`Malformed`], never a panic.

use net::Url;

/// Why bytes are no message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Malformed {
    /// The bytes end inside a field.
    Truncated,
    /// A tag that no variant of the type named has in this version.
    Tag(&'static str, u32),
    /// A field's value that its type cannot have: a string that is not
    /// UTF-8, a URL that does not parse, a number past its range.
    Invalid(&'static str),
    /// Bytes left over after the message.
    Trailing,
}

impl std::fmt::Display for Malformed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Malformed::Truncated => write!(f, "truncated"),
            Malformed::Tag(what, tag) => write!(f, "unknown tag {tag} of {what}"),
            Malformed::Invalid(what) => write!(f, "invalid {what}"),
            Malformed::Trailing => write!(f, "trailing bytes"),
        }
    }
}

/// Writes values into bytes.
#[derive(Debug, Default)]
pub struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    pub fn new() -> Encoder {
        Encoder::default()
    }

    /// An encoder whose bytes start with `prefix`, for a frame's length.
    pub(crate) fn after(prefix: &[u8]) -> Encoder {
        Encoder {
            bytes: prefix.to_vec(),
        }
    }

    pub fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    /// A length, which must fit in 32 bits: a message is far shorter.
    pub fn length(&mut self, len: usize) {
        self.u32(u32::try_from(len).unwrap_or(u32::MAX));
    }

    /// A byte string, by its length.
    pub fn bytes(&mut self, bytes: &[u8]) {
        self.length(bytes.len());
        self.bytes.extend_from_slice(bytes);
    }

    /// A string, by its length in bytes.
    pub fn str(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    /// The bytes written.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// The bytes written so far.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Reads values out of bytes, each read checked against what is left.
#[derive(Debug)]
pub struct Decoder<'a> {
    rest: &'a [u8],
}

impl<'a> Decoder<'a> {
    pub fn new(bytes: &'a [u8]) -> Decoder<'a> {
        Decoder { rest: bytes }
    }

    /// How many bytes are left.
    pub fn left(&self) -> usize {
        self.rest.len()
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Malformed> {
        if n > self.rest.len() {
            return Err(Malformed::Truncated);
        }
        let (taken, rest) = self.rest.split_at(n);
        self.rest = rest;
        Ok(taken)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], Malformed> {
        let mut array = [0; N];
        array.copy_from_slice(self.take(N)?);
        Ok(array)
    }

    pub fn u8(&mut self) -> Result<u8, Malformed> {
        Ok(self.array::<1>()?[0])
    }

    pub fn u16(&mut self) -> Result<u16, Malformed> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> Result<u32, Malformed> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn u64(&mut self) -> Result<u64, Malformed> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    /// A boolean: 0 or 1, nothing else.
    pub fn bool(&mut self) -> Result<bool, Malformed> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Malformed::Invalid("bool")),
        }
    }

    /// A length of what follows, which cannot be more than the bytes
    /// left.
    pub fn length(&mut self) -> Result<usize, Malformed> {
        let len = usize::try_from(self.u32()?).map_err(|_| Malformed::Truncated)?;
        if len > self.rest.len() {
            return Err(Malformed::Truncated);
        }
        Ok(len)
    }

    /// A count of values that follow, each at least a byte long: no
    /// more than the bytes left.
    pub fn count(&mut self) -> Result<usize, Malformed> {
        self.length()
    }

    /// A byte string.
    pub fn bytes(&mut self) -> Result<&'a [u8], Malformed> {
        let len = self.length()?;
        self.take(len)
    }

    /// A string, which must be UTF-8.
    pub fn str(&mut self) -> Result<&'a str, Malformed> {
        std::str::from_utf8(self.bytes()?).map_err(|_| Malformed::Invalid("string"))
    }

    /// Nothing may be left.
    pub fn finish(self) -> Result<(), Malformed> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(Malformed::Trailing)
        }
    }
}

/// A value that goes over the wire.
pub trait Wire: Sized {
    fn encode(&self, out: &mut Encoder);
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed>;
}

/// `bytes` as one value of `T`, all of them.
pub fn decode_all<T: Wire>(bytes: &[u8]) -> Result<T, Malformed> {
    let mut input = Decoder::new(bytes);
    let value = T::decode(&mut input)?;
    input.finish()?;
    Ok(value)
}

/// `value`'s bytes.
pub fn encode<T: Wire>(value: &T) -> Vec<u8> {
    let mut out = Encoder::new();
    value.encode(&mut out);
    out.into_bytes()
}

macro_rules! wire_int {
    ($($ty:ty => $method:ident),*) => {$(
        impl Wire for $ty {
            fn encode(&self, out: &mut Encoder) {
                out.$method(*self);
            }
            fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
                input.$method()
            }
        }
    )*};
}

wire_int!(u8 => u8, u16 => u16, u32 => u32, u64 => u64, bool => bool);

impl Wire for i32 {
    fn encode(&self, out: &mut Encoder) {
        out.u32(u32::from_le_bytes(self.to_le_bytes()));
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(i32::from_le_bytes(input.u32()?.to_le_bytes()))
    }
}

impl Wire for i64 {
    fn encode(&self, out: &mut Encoder) {
        out.u64(u64::from_le_bytes(self.to_le_bytes()));
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(i64::from_le_bytes(input.u64()?.to_le_bytes()))
    }
}

impl Wire for usize {
    fn encode(&self, out: &mut Encoder) {
        out.u64(*self as u64);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        usize::try_from(input.u64()?).map_err(|_| Malformed::Invalid("usize"))
    }
}

impl Wire for f32 {
    fn encode(&self, out: &mut Encoder) {
        out.u32(self.to_bits());
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(f32::from_bits(input.u32()?))
    }
}

impl Wire for f64 {
    fn encode(&self, out: &mut Encoder) {
        out.u64(self.to_bits());
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok(f64::from_bits(input.u64()?))
    }
}

impl Wire for String {
    fn encode(&self, out: &mut Encoder) {
        out.str(self);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        input.str().map(str::to_owned)
    }
}

impl<T: Wire> Wire for Option<T> {
    fn encode(&self, out: &mut Encoder) {
        match self {
            None => out.u8(0),
            Some(value) => {
                out.u8(1);
                value.encode(out);
            }
        }
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        match input.u8()? {
            0 => Ok(None),
            1 => T::decode(input).map(Some),
            tag => Err(Malformed::Tag("Option", u32::from(tag))),
        }
    }
}

impl<T: Wire> Wire for Vec<T> {
    fn encode(&self, out: &mut Encoder) {
        out.length(self.len());
        for value in self {
            value.encode(out);
        }
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        let count = input.count()?;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(T::decode(input)?);
        }
        Ok(values)
    }
}

impl<A: Wire, B: Wire> Wire for (A, B) {
    fn encode(&self, out: &mut Encoder) {
        self.0.encode(out);
        self.1.encode(out);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok((A::decode(input)?, B::decode(input)?))
    }
}

impl<A: Wire, B: Wire, C: Wire> Wire for (A, B, C) {
    fn encode(&self, out: &mut Encoder) {
        self.0.encode(out);
        self.1.encode(out);
        self.2.encode(out);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Ok((A::decode(input)?, B::decode(input)?, C::decode(input)?))
    }
}

impl<T: Wire> Wire for Box<T> {
    fn encode(&self, out: &mut Encoder) {
        (**self).encode(out);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        T::decode(input).map(Box::new)
    }
}

impl Wire for Url {
    fn encode(&self, out: &mut Encoder) {
        out.str(self.as_str());
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        Url::parse(input.str()?).map_err(|_| Malformed::Invalid("URL"))
    }
}

/// A byte string, a body's or pixels': written whole, not byte by byte.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);

impl std::fmt::Debug for Bytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bytes({})", self.0.len())
    }
}

impl Wire for Bytes {
    fn encode(&self, out: &mut Encoder) {
        out.bytes(&self.0);
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, Malformed> {
        input.bytes().map(|bytes| Bytes(bytes.to_vec()))
    }
}

/// An enum whose variants have named fields, each a [`Wire`], with the
/// tag of each variant given: its encoding is the tag (16 bits) and the
/// fields in order.
macro_rules! wire_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$vmeta:meta])*
                $tag:literal => $variant:ident { $( $(#[$fmeta:meta])* $field:ident : $ty:ty ),* $(,)? }
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis enum $name {
            $(
                $(#[$vmeta])*
                $variant { $( $(#[$fmeta])* $field: $ty ),* },
            )*
        }

        impl $crate::ipc::Wire for $name {
            #[allow(unused_variables)]
            fn encode(&self, out: &mut $crate::ipc::Encoder) {
                match self {
                    $(
                        $name::$variant { $($field),* } => {
                            out.u16($tag);
                            $( $crate::ipc::Wire::encode($field, out); )*
                        }
                    )*
                }
            }

            #[allow(unused_variables)]
            fn decode(
                input: &mut $crate::ipc::Decoder<'_>,
            ) -> Result<Self, $crate::ipc::Malformed> {
                match input.u16()? {
                    $(
                        $tag => Ok($name::$variant {
                            $( $field: <$ty as $crate::ipc::Wire>::decode(input)?, )*
                        }),
                    )*
                    tag => Err($crate::ipc::Malformed::Tag(stringify!($name), u32::from(tag))),
                }
            }
        }
    };
}

/// A struct of named fields, each a [`Wire`], encoded in order.
#[allow(unused_macros)]
macro_rules! wire_struct {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $( $(#[$fmeta:meta])* $fvis:vis $field:ident : $ty:ty ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis struct $name {
            $( $(#[$fmeta])* $fvis $field: $ty ),*
        }

        impl $crate::ipc::Wire for $name {
            fn encode(&self, out: &mut $crate::ipc::Encoder) {
                $( $crate::ipc::Wire::encode(&self.$field, out); )*
            }

            fn decode(
                input: &mut $crate::ipc::Decoder<'_>,
            ) -> Result<Self, $crate::ipc::Malformed> {
                Ok($name {
                    $( $field: <$ty as $crate::ipc::Wire>::decode(input)?, )*
                })
            }
        }
    };
}

/// An enum of unit variants, each with its tag (8 bits).
#[allow(unused_macros)]
macro_rules! wire_unit_enum {
    ($name:ty { $( $tag:literal => $variant:path ),* $(,)? }) => {
        impl $crate::ipc::Wire for $name {
            fn encode(&self, out: &mut $crate::ipc::Encoder) {
                out.u8(match self {
                    $( $variant => $tag, )*
                });
            }

            fn decode(
                input: &mut $crate::ipc::Decoder<'_>,
            ) -> Result<Self, $crate::ipc::Malformed> {
                match input.u8()? {
                    $( $tag => Ok($variant), )*
                    tag => Err($crate::ipc::Malformed::Tag(stringify!($name), u32::from(tag))),
                }
            }
        }
    };
}

pub(crate) use {wire_enum, wire_struct, wire_unit_enum};
