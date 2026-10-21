//! HPACK, header compression for HTTP/2 (RFC 7541): integers and strings
//! (section 5), the static and dynamic tables (section 2.3 and 4), and
//! the representations a header block is made of (section 6). The
//! decoder takes any bytes a server sends: every length is checked
//! before it is used, the dynamic table is held to the size this end
//! announced, and a header list past its bound is decoded (so that the
//! table stays in step) and reported, not kept.

use std::collections::VecDeque;

use super::huffman_table::CODES;

/// The dynamic table's size HTTP/2 starts with on both ends (RFC 9113
/// 6.5.2, `SETTINGS_HEADER_TABLE_SIZE`'s initial value), which this
/// end keeps.
pub const HPACK_TABLE_SIZE: usize = 4096;

/// The most an integer may be: past it, a block is refused (no field
/// or table is that large).
const MAX_INTEGER: u64 = 1 << 30;

/// A header field: its name and its value, as bytes.
pub type Field = (Vec<u8>, Vec<u8>);

/// Why a header block could not be decoded: a connection error,
/// `COMPRESSION_ERROR` (RFC 9113 4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HpackError(pub &'static str);

impl std::fmt::Display for HpackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "header compression: {}", self.0)
    }
}

impl std::error::Error for HpackError {}

/// The static table (RFC 7541 Appendix A), index 1 first.
const STATIC: [(&str, &str); 61] = [
    (":authority", ""),
    (":method", "GET"),
    (":method", "POST"),
    (":path", "/"),
    (":path", "/index.html"),
    (":scheme", "http"),
    (":scheme", "https"),
    (":status", "200"),
    (":status", "204"),
    (":status", "206"),
    (":status", "304"),
    (":status", "400"),
    (":status", "404"),
    (":status", "500"),
    ("accept-charset", ""),
    ("accept-encoding", "gzip, deflate"),
    ("accept-language", ""),
    ("accept-ranges", ""),
    ("accept", ""),
    ("access-control-allow-origin", ""),
    ("age", ""),
    ("allow", ""),
    ("authorization", ""),
    ("cache-control", ""),
    ("content-disposition", ""),
    ("content-encoding", ""),
    ("content-language", ""),
    ("content-length", ""),
    ("content-location", ""),
    ("content-range", ""),
    ("content-type", ""),
    ("cookie", ""),
    ("date", ""),
    ("etag", ""),
    ("expect", ""),
    ("expires", ""),
    ("from", ""),
    ("host", ""),
    ("if-match", ""),
    ("if-modified-since", ""),
    ("if-none-match", ""),
    ("if-range", ""),
    ("if-unmodified-since", ""),
    ("last-modified", ""),
    ("link", ""),
    ("location", ""),
    ("max-forwards", ""),
    ("proxy-authenticate", ""),
    ("proxy-authorization", ""),
    ("range", ""),
    ("referer", ""),
    ("refresh", ""),
    ("retry-after", ""),
    ("server", ""),
    ("set-cookie", ""),
    ("strict-transport-security", ""),
    ("transfer-encoding", ""),
    ("user-agent", ""),
    ("vary", ""),
    ("via", ""),
    ("www-authenticate", ""),
];

/// A field's size in a table (RFC 7541 4.1): its name's and value's
/// lengths and 32.
pub fn field_size(name: &[u8], value: &[u8]) -> usize {
    name.len() + value.len() + 32
}

/// The dynamic table: the newest field first, held within `max` bytes
/// by field size.
#[derive(Debug, Clone, Default)]
struct Table {
    fields: VecDeque<Field>,
    size: usize,
    max: usize,
}

impl Table {
    fn new(max: usize) -> Table {
        Table {
            fields: VecDeque::new(),
            size: 0,
            max,
        }
    }

    /// Evicts the oldest fields until the table is within `max`.
    fn resize(&mut self, max: usize) {
        self.max = max;
        while self.size > self.max {
            self.evict();
        }
    }

    fn evict(&mut self) {
        if let Some((name, value)) = self.fields.pop_back() {
            self.size -= field_size(&name, &value);
        }
    }

    /// Adds a field, evicting the oldest to make room; one larger than
    /// the table empties it (section 4.4).
    fn insert(&mut self, name: Vec<u8>, value: Vec<u8>) {
        let size = field_size(&name, &value);
        while self.size + size > self.max && !self.fields.is_empty() {
            self.evict();
        }
        if size <= self.max {
            self.size += size;
            self.fields.push_front((name, value));
        }
    }

    /// The field at `index` of the address space both tables share
    /// (section 2.3.3), from 1.
    fn get(&self, index: u64) -> Option<(&[u8], &[u8])> {
        let index = usize::try_from(index).ok()?;
        if index == 0 {
            return None;
        }
        if index <= STATIC.len() {
            let (name, value) = STATIC[index - 1];
            return Some((name.as_bytes(), value.as_bytes()));
        }
        self.fields
            .get(index - STATIC.len() - 1)
            .map(|(name, value)| (name.as_slice(), value.as_slice()))
    }
}

/// A header block decoded: its fields, and whether its list was larger
/// than the bound it was decoded under (then its fields past the bound
/// are left out, and the stream is to be refused).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Decoded {
    pub fields: Vec<Field>,
    pub too_large: bool,
}

/// The decoder of a connection's header blocks.
#[derive(Debug, Clone)]
pub struct Decoder {
    table: Table,
    /// The most the peer may make the table, which this end announced.
    limit: usize,
}

impl Default for Decoder {
    fn default() -> Decoder {
        Decoder::new(HPACK_TABLE_SIZE)
    }
}

impl Decoder {
    /// A decoder whose table the peer may make at most `limit` bytes.
    pub fn new(limit: usize) -> Decoder {
        Decoder {
            table: Table::new(limit),
            limit,
        }
    }

    /// The dynamic table's size now, for the tests and fuzzing.
    pub fn table_size(&self) -> usize {
        self.table.size
    }

    /// Decodes one header block, its fields' list held to `max_list`
    /// bytes by field size (`SETTINGS_MAX_HEADER_LIST_SIZE`).
    pub fn decode(&mut self, mut block: &[u8], max_list: usize) -> Result<Decoded, HpackError> {
        let mut decoded = Decoded::default();
        let mut list = 0usize;
        let mut first = true;
        while let Some(&byte) = block.first() {
            let field = if byte & 0x80 != 0 {
                // An indexed field (6.1).
                let index = integer(&mut block, 7)?;
                let (name, value) = self.table.get(index).ok_or(HpackError("no such index"))?;
                Some((name.to_vec(), value.to_vec()))
            } else if byte & 0x40 != 0 {
                // A literal added to the table (6.2.1).
                let (name, value) = self.literal(&mut block, 6)?;
                self.table.insert(name.clone(), value.clone());
                Some((name, value))
            } else if byte & 0x20 != 0 {
                // A dynamic table size update (6.3), at a block's start
                // alone, and within what this end announced.
                if !first {
                    return Err(HpackError("a table size update after a field"));
                }
                let size = integer(&mut block, 5)?;
                if size > self.limit as u64 {
                    return Err(HpackError("a table larger than announced"));
                }
                self.table.resize(size as usize);
                None
            } else {
                // A literal not added, or never added (6.2.2, 6.2.3).
                Some(self.literal(&mut block, 4)?)
            };
            if let Some((name, value)) = field {
                first = false;
                list = list.saturating_add(field_size(&name, &value));
                if list > max_list {
                    decoded.too_large = true;
                } else {
                    decoded.fields.push((name, value));
                }
            }
        }
        Ok(decoded)
    }

    /// A literal field whose name is indexed by an integer of `prefix`
    /// bits, or, for 0, a string of its own; then its value.
    fn literal(&self, block: &mut &[u8], prefix: u8) -> Result<Field, HpackError> {
        let index = integer(block, prefix)?;
        let name = match index {
            0 => string(block)?,
            index => self
                .table
                .get(index)
                .ok_or(HpackError("no such index"))?
                .0
                .to_vec(),
        };
        let value = string(block)?;
        Ok((name, value))
    }
}

/// Reads an integer of a `prefix`-bit prefix (5.1), taking its bytes.
fn integer(block: &mut &[u8], prefix: u8) -> Result<u64, HpackError> {
    let (&first, mut rest) = block
        .split_first()
        .ok_or(HpackError("a truncated integer"))?;
    let mask = (1u16 << prefix) as u8 - 1;
    let mut value = u64::from(first & mask);
    if value == u64::from(mask) {
        let mut shift = 0u32;
        loop {
            let (&byte, more) = rest
                .split_first()
                .ok_or(HpackError("a truncated integer"))?;
            rest = more;
            value += u64::from(byte & 0x7f) << shift;
            if value > MAX_INTEGER {
                return Err(HpackError("an integer too large"));
            }
            if byte & 0x80 == 0 {
                break;
            }
            // Zeros may go on past any value's width (found by the
            // fuzzer): an integer of more bytes than its bound needs is
            // refused.
            shift += 7;
            if shift > 28 {
                return Err(HpackError("an integer too long"));
            }
        }
    }
    *block = rest;
    Ok(value)
}

/// Reads a string literal (5.2), Huffman-coded or not, taking its bytes.
fn string(block: &mut &[u8]) -> Result<Vec<u8>, HpackError> {
    let huffman = block.first().is_some_and(|byte| byte & 0x80 != 0);
    let length = integer(block, 7)? as usize;
    if length > block.len() {
        return Err(HpackError("a truncated string"));
    }
    let (bytes, rest) = block.split_at(length);
    *block = rest;
    match huffman {
        true => huffman_decode(bytes),
        false => Ok(bytes.to_vec()),
    }
}

/// Writes `value` as an integer of a `prefix`-bit prefix, the prefix's
/// byte's other bits `flags`.
fn put_integer(out: &mut Vec<u8>, value: usize, prefix: u8, flags: u8) {
    let mask = (1usize << prefix) - 1;
    if value < mask {
        out.push(flags | value as u8);
        return;
    }
    out.push(flags | mask as u8);
    let mut rest = value - mask;
    while rest >= 0x80 {
        out.push((rest & 0x7f) as u8 | 0x80);
        rest >>= 7;
    }
    out.push(rest as u8);
}

/// Writes a string literal, Huffman-coded when that is shorter.
fn put_string(out: &mut Vec<u8>, bytes: &[u8]) {
    let coded = huffman_encode(bytes);
    if coded.len() < bytes.len() {
        put_integer(out, coded.len(), 7, 0x80);
        out.extend_from_slice(&coded);
    } else {
        put_integer(out, bytes.len(), 7, 0);
        out.extend_from_slice(bytes);
    }
}

/// `bytes` in HPACK's Huffman code, padded with ones (5.2).
pub fn huffman_encode(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut bits: u64 = 0;
    let mut count: u32 = 0;
    for &byte in bytes {
        let (code, length) = CODES[usize::from(byte)];
        bits = (bits << length) | u64::from(code);
        count += u32::from(length);
        while count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    if count > 0 {
        let pad = 8 - count;
        out.push(((bits << pad) | ((1 << pad) - 1)) as u8);
    }
    out
}

/// The canonical code's first code and first symbol's place for each
/// length, and the symbols in code order.
struct Canonical {
    /// For each length 0..=30, the first code of that length and how
    /// many codes have it, and where its symbols start in `symbols`.
    first: [u32; 31],
    count: [u32; 31],
    start: [usize; 31],
    symbols: [u16; 257],
}

/// The canonical decoding tables, made from [`CODES`] at compile time.
const CANONICAL: Canonical = canonical();

const fn canonical() -> Canonical {
    let mut count = [0u32; 31];
    let mut symbol = 0;
    while symbol < 257 {
        count[CODES[symbol].1 as usize] += 1;
        symbol += 1;
    }
    let mut start = [0usize; 31];
    let mut first = [0u32; 31];
    let mut code = 0u32;
    let mut at = 0usize;
    let mut length = 1;
    while length < 31 {
        code = (code + count[length - 1]) << 1;
        first[length] = code;
        start[length] = at;
        at += count[length] as usize;
        length += 1;
    }
    // The symbols ordered by length, then by symbol, which is the
    // order of their codes in a canonical code.
    let mut symbols = [0u16; 257];
    let mut placed = [0usize; 31];
    let mut symbol = 0;
    while symbol < 257 {
        let length = CODES[symbol].1 as usize;
        symbols[start[length] + placed[length]] = symbol as u16;
        placed[length] += 1;
        symbol += 1;
    }
    Canonical {
        first,
        count,
        start,
        symbols,
    }
}

/// Decodes HPACK's Huffman code: the end-of-string symbol, or padding
/// longer than seven bits or not all ones, is an error (5.2).
pub fn huffman_decode(bytes: &[u8]) -> Result<Vec<u8>, HpackError> {
    let table = &CANONICAL;
    let mut out = Vec::with_capacity(bytes.len() * 8 / 5);
    let mut code: u32 = 0;
    let mut length: usize = 0;
    // Whether every bit since the last symbol was a one.
    let mut ones = true;
    for &byte in bytes {
        for shift in (0..8).rev() {
            let bit = u32::from(byte >> shift) & 1;
            code = (code << 1) | bit;
            length += 1;
            ones &= bit == 1;
            if length > 30 {
                return Err(HpackError("a Huffman code too long"));
            }
            let offset = code.wrapping_sub(table.first[length]);
            if code >= table.first[length] && offset < table.count[length] {
                let symbol = table.symbols[table.start[length] + offset as usize];
                if symbol == 256 {
                    return Err(HpackError("the end of string in a string"));
                }
                out.push(symbol as u8);
                code = 0;
                length = 0;
                ones = true;
            }
        }
    }
    if length > 7 || !ones {
        return Err(HpackError("bad Huffman padding"));
    }
    Ok(out)
}

/// The encoder of a connection's header blocks, with a dynamic table of
/// its own that the peer's decoder mirrors.
#[derive(Debug, Clone)]
pub struct Encoder {
    table: Table,
    /// A size the next block must announce first, after the peer's
    /// setting changed.
    pending: Option<usize>,
}

impl Default for Encoder {
    fn default() -> Encoder {
        Encoder {
            table: Table::new(HPACK_TABLE_SIZE),
            pending: None,
        }
    }
}

/// Whether a field is never added to a table (RFC 7541 7.1.3): one an
/// attacker could guess a secret of by compression.
fn sensitive(name: &[u8], value: &[u8]) -> bool {
    match name {
        b"authorization" | b"proxy-authorization" => true,
        b"cookie" | b"set-cookie" => value.len() < 20,
        _ => false,
    }
}

impl Encoder {
    /// The peer's decoder may keep `size` bytes from now on
    /// (`SETTINGS_HEADER_TABLE_SIZE`); the encoder keeps at most this
    /// end's own, and says so at its next block.
    pub fn set_max_size(&mut self, size: usize) {
        let size = size.min(HPACK_TABLE_SIZE);
        if size != self.table.max {
            self.table.resize(size);
            self.pending = Some(size);
        }
    }

    /// Encodes `fields` (names already lowercase) into one block.
    pub fn encode<'a>(
        &mut self,
        fields: impl IntoIterator<Item = (&'a [u8], &'a [u8])>,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(size) = self.pending.take() {
            put_integer(&mut out, size, 5, 0x20);
        }
        for (name, value) in fields {
            let mut name_index = 0;
            let mut whole = 0;
            for index in 1..=STATIC.len() + self.table.fields.len() {
                let Some((n, v)) = self.table.get(index as u64) else {
                    break;
                };
                if n == name {
                    if name_index == 0 {
                        name_index = index;
                    }
                    if v == value {
                        whole = index;
                        break;
                    }
                }
            }
            if whole != 0 && !sensitive(name, value) {
                put_integer(&mut out, whole, 7, 0x80);
                continue;
            }
            if sensitive(name, value) {
                put_integer(&mut out, name_index, 4, 0x10);
            } else {
                put_integer(&mut out, name_index, 6, 0x40);
                self.table.insert(name.to_vec(), value.to_vec());
            }
            if name_index == 0 {
                put_string(&mut out, name);
            }
            put_string(&mut out, value);
        }
        out
    }
}
