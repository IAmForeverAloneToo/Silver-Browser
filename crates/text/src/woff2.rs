//! WOFF 2 (W3C WOFF File Format 2.0): a font file whose tables are
//! compressed together by Brotli, `glyf` and `loca` (and perhaps `hmtx`)
//! transformed first, unpacked to an sfnt (D54). Every size read is
//! checked against the data and [`MAX_FONT_BYTES`] before anything is
//! allocated for it; a font collection (`ttcf`) is refused.

use std::io::Read as _;

use super::web::MAX_FONT_BYTES;

/// The tags a table directory entry may name by number (section 5.1).
const KNOWN_TAGS: [&[u8; 4]; 63] = [
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"cvt ", b"fpgm",
    b"glyf", b"loca", b"prep", b"CFF ", b"VORG", b"EBDT", b"EBLC", b"gasp", b"hdmx", b"kern",
    b"LTSH", b"PCLT", b"VDMX", b"vhea", b"vmtx", b"BASE", b"GDEF", b"GPOS", b"GSUB", b"EBSC",
    b"JSTF", b"MATH", b"CBDT", b"CBLC", b"COLR", b"CPAL", b"SVG ", b"sbix", b"acnt", b"avar",
    b"bdat", b"bloc", b"bsln", b"cvar", b"fdsc", b"feat", b"fmtx", b"fvar", b"gvar", b"hsty",
    b"just", b"lcar", b"mort", b"morx", b"opbd", b"prop", b"trak", b"Zapf", b"Silf", b"Glat",
    b"Gloc", b"Feat", b"Sill",
];

/// The most tables a file may have.
const MAX_TABLES: usize = 512;

/// A reader over bytes that says `None` past their end.
struct Bytes<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Bytes<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bytes { data, at: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let out = self.data.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(out)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes(self.take(2)?.try_into().ok()?))
    }

    fn i16(&mut self) -> Option<i16> {
        Some(self.u16()? as i16)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }

    /// A `UIntBase128` (section 4.1): at most five bytes, no leading
    /// zeros, below 2^32.
    fn base128(&mut self) -> Option<u32> {
        let mut value: u32 = 0;
        for n in 0..5 {
            let byte = self.u8()?;
            if n == 0 && byte == 0x80 || value & 0xFE00_0000 != 0 {
                return None;
            }
            value = value << 7 | u32::from(byte & 0x7F);
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    /// A `255UInt16` (section 4.2).
    fn u255(&mut self) -> Option<u16> {
        Some(match self.u8()? {
            253 => self.u16()?,
            255 => u16::from(self.u8()?) + 253,
            254 => u16::from(self.u8()?) + 506,
            code => u16::from(code),
        })
    }
}

/// One table of the directory.
struct Entry {
    tag: [u8; 4],
    length: usize,
    /// Its bytes in the unpacked stream: its transformed length when it
    /// is transformed.
    stored: usize,
    transformed: bool,
}

/// The sfnt a WOFF 2 file holds; `None` for one that is broken, too
/// large or a collection.
pub(crate) fn unpacked(data: &[u8]) -> Option<Vec<u8>> {
    let mut r = Bytes::new(data);
    if r.take(4)? != b"wOF2" {
        return None;
    }
    let flavor = r.u32()?;
    if flavor == u32::from_be_bytes(*b"ttcf") {
        return None;
    }
    let _length = r.u32()?;
    let count = usize::from(r.u16()?);
    let _reserved = r.u16()?;
    let total = r.u32()? as usize;
    let compressed = r.u32()? as usize;
    r.take(24)?;
    if count == 0 || count > MAX_TABLES || total > MAX_FONT_BYTES {
        return None;
    }
    let mut entries = Vec::with_capacity(count);
    let mut stream = 0usize;
    for _ in 0..count {
        let flags = r.u8()?;
        let tag = match flags & 0x3F {
            63 => r.take(4)?.try_into().ok()?,
            n => *KNOWN_TAGS[usize::from(n)],
        };
        let version = flags >> 6;
        let length = r.base128()? as usize;
        // `glyf` and `loca` are transformed by version 0, the rest by any
        // other.
        let transformed = if &tag == b"glyf" || &tag == b"loca" {
            version == 0
        } else {
            version != 0
        };
        let stored = if transformed {
            r.base128()? as usize
        } else {
            length
        };
        if transformed && !matches!(&tag, b"glyf" | b"loca" | b"hmtx") {
            return None;
        }
        if length > MAX_FONT_BYTES {
            return None;
        }
        stream = stream.checked_add(stored)?;
        entries.push(Entry {
            tag,
            length,
            stored,
            transformed,
        });
    }
    if stream > MAX_FONT_BYTES {
        return None;
    }
    let packed = r.take(compressed)?;
    let mut unpacked = Vec::with_capacity(stream);
    brotli_decompressor::Decompressor::new(packed, 4096)
        .take(stream as u64 + 1)
        .read_to_end(&mut unpacked)
        .ok()?;
    if unpacked.len() != stream {
        return None;
    }
    // Each table's bytes in the stream, in the directory's order.
    let mut at = 0;
    let mut stored: Vec<&[u8]> = Vec::with_capacity(count);
    for entry in &entries {
        stored.push(&unpacked[at..at + entry.stored]);
        at += entry.stored;
    }
    let find = |tag: &[u8; 4]| entries.iter().position(|e| &e.tag == tag);
    // `glyf` and `loca` are transformed together, and `hmtx` only with
    // them.
    let transformed = |tag: &[u8; 4]| find(tag).is_some_and(|n| entries[n].transformed);
    if transformed(b"glyf") != transformed(b"loca") || transformed(b"hmtx") && !transformed(b"glyf")
    {
        return None;
    }
    let mut tables: Vec<([u8; 4], Vec<u8>)> = Vec::with_capacity(count);
    // The glyphs' boxes' left edges, for a transformed `hmtx`.
    let mut x_mins = Vec::new();
    for (n, entry) in entries.iter().enumerate() {
        let body = match &entry.tag {
            b"glyf" if entry.transformed => {
                let loca = find(b"loca")?;
                let (glyf, loca_table, mins) = glyf(stored[n])?;
                if loca_table.len() != entries[loca].length {
                    return None;
                }
                x_mins = mins;
                tables.push((*b"loca", loca_table));
                glyf
            }
            // Made with `glyf`.
            b"loca" if entry.transformed => continue,
            b"hmtx" if entry.transformed => continue,
            _ => stored[n].to_vec(),
        };
        if !entry.transformed && body.len() != entry.length {
            return None;
        }
        tables.push((entry.tag, body));
    }
    if let Some(n) = find(b"hmtx")
        && entries[n].transformed
    {
        let hhea = &tables.iter().find(|(tag, _)| tag == b"hhea")?.1;
        let metrics = usize::from(u16::from_be_bytes(hhea.get(34..36)?.try_into().ok()?));
        let body = hmtx(stored[n], metrics, &x_mins)?;
        if body.len() != entries[n].length {
            return None;
        }
        tables.push((*b"hmtx", body));
    }
    sfnt(flavor, tables)
}

/// The sfnt of `tables`: the offset table, the records by tag, then each
/// table on four bytes.
fn sfnt(flavor: u32, mut tables: Vec<([u8; 4], Vec<u8>)>) -> Option<Vec<u8>> {
    tables.sort_by_key(|table| table.0);
    let count = tables.len();
    let size = 12
        + count * 16
        + tables
            .iter()
            .map(|(_, body)| body.len().div_ceil(4) * 4)
            .sum::<usize>();
    if size > MAX_FONT_BYTES {
        return None;
    }
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(&flavor.to_be_bytes());
    out.extend_from_slice(&(count as u16).to_be_bytes());
    let mut power = 1usize;
    let mut log = 0u16;
    while power * 2 <= count {
        power *= 2;
        log += 1;
    }
    let range = (power * 16) as u16;
    out.extend_from_slice(&range.to_be_bytes());
    out.extend_from_slice(&log.to_be_bytes());
    out.extend_from_slice(&((count * 16) as u16).wrapping_sub(range).to_be_bytes());
    let mut offset = 12 + count * 16;
    for (tag, body) in &tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum(body).to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        offset += body.len().div_ceil(4) * 4;
    }
    for (_, body) in &tables {
        out.extend_from_slice(body);
        out.resize(out.len().div_ceil(4) * 4, 0);
    }
    Some(out)
}

/// A table's checksum: its words summed.
fn checksum(body: &[u8]) -> u32 {
    body.chunks(4).fold(0u32, |sum, word| {
        let mut w = [0u8; 4];
        w[..word.len()].copy_from_slice(word);
        sum.wrapping_add(u32::from_be_bytes(w))
    })
}

/// A point's move from a flag and its bytes (section 5.2's triplets).
fn triplet(flag: u8, bytes: &[u8]) -> (i32, i32) {
    let sign = |flag: u8, base: i32| if flag & 1 != 0 { base } else { -base };
    let b = |n: usize| i32::from(bytes[n]);
    let f = i32::from(flag);
    if flag < 10 {
        (0, sign(flag, ((f & 14) << 7) + b(0)))
    } else if flag < 20 {
        (sign(flag, (((f - 10) & 14) << 7) + b(0)), 0)
    } else if flag < 84 {
        let b0 = f - 20;
        (
            sign(flag, 1 + (b0 & 0x30) + (b(0) >> 4)),
            sign(flag >> 1, 1 + ((b0 & 0x0C) << 2) + (b(0) & 0x0F)),
        )
    } else if flag < 120 {
        let b0 = f - 84;
        (
            sign(flag, 1 + ((b0 / 12) << 8) + b(0)),
            sign(flag >> 1, 1 + (((b0 % 12) >> 2) << 8) + b(1)),
        )
    } else if flag < 124 {
        (
            sign(flag, (b(0) << 4) + (b(1) >> 4)),
            sign(flag >> 1, ((b(1) & 0x0F) << 8) + b(2)),
        )
    } else {
        (
            sign(flag, (b(0) << 8) + b(1)),
            sign(flag >> 1, (b(2) << 8) + b(3)),
        )
    }
}

/// How many bytes a triplet's flag takes.
fn triplet_bytes(flag: u8) -> usize {
    match flag {
        0..84 => 1,
        84..120 => 2,
        120..124 => 3,
        _ => 4,
    }
}

/// The `glyf` and `loca` a transformed `glyf` (section 5.1) makes, and
/// each glyph's least x.
fn glyf(data: &[u8]) -> Option<(Vec<u8>, Vec<u8>, Vec<i16>)> {
    let mut r = Bytes::new(data);
    let _reserved = r.u16()?;
    let options = r.u16()?;
    let glyphs = usize::from(r.u16()?);
    let long = r.u16()? != 0;
    let mut sizes = [0usize; 7];
    for size in &mut sizes {
        *size = r.u32()? as usize;
    }
    let mut streams = Vec::with_capacity(7);
    for size in sizes {
        streams.push(Bytes::new(r.take(size)?));
    }
    let [
        mut contours,
        mut points,
        mut flags,
        mut coords,
        mut composites,
        mut bboxes,
        mut instructions,
    ]: [Bytes<'_>; 7] = streams.try_into().ok()?;
    let overlap = if options & 1 != 0 {
        Some(r.take(glyphs.div_ceil(8))?)
    } else {
        None
    };
    let bitmap_len = glyphs.div_ceil(32) * 4;
    let bitmap = bboxes.take(bitmap_len)?;
    let has_box = |n: usize| bitmap[n / 8] & (0x80 >> (n % 8)) != 0;
    let mut out = Vec::new();
    let mut offsets = Vec::with_capacity(glyphs + 1);
    let mut x_mins = Vec::with_capacity(glyphs);
    for n in 0..glyphs {
        offsets.push(out.len());
        let count = contours.i16()?;
        if count == 0 {
            if has_box(n) {
                return None;
            }
            x_mins.push(0);
            continue;
        }
        if count < 0 {
            if !has_box(n) {
                return None;
            }
            let bbox = bboxes.take(8)?;
            let start = composites.at;
            let mut instructed = false;
            loop {
                let flag = composites.u16()?;
                composites.take(2)?;
                let mut more = if flag & 0x0001 != 0 { 4 } else { 2 };
                if flag & 0x0008 != 0 {
                    more += 2;
                } else if flag & 0x0040 != 0 {
                    more += 4;
                } else if flag & 0x0080 != 0 {
                    more += 8;
                }
                composites.take(more)?;
                instructed |= flag & 0x0100 != 0;
                if flag & 0x0020 == 0 {
                    break;
                }
            }
            let parts = &composites.data[start..composites.at];
            out.extend_from_slice(&(-1i16).to_be_bytes());
            out.extend_from_slice(bbox);
            out.extend_from_slice(parts);
            if instructed {
                let length = coords.u255()?;
                out.extend_from_slice(&length.to_be_bytes());
                out.extend_from_slice(instructions.take(usize::from(length))?);
            }
            x_mins.push(i16::from_be_bytes([bbox[0], bbox[1]]));
        } else {
            let count = count as usize;
            let mut ends = Vec::with_capacity(count.min(points.data.len()));
            let mut total = 0usize;
            for _ in 0..count {
                total += usize::from(points.u255()?);
                if total == 0 || total > 0xFFFF {
                    return None;
                }
                ends.push((total - 1) as u16);
            }
            // One flag a point: no more points than there are flags.
            let point_flags = flags.take(total)?;
            let mut xs = Vec::with_capacity(total);
            let mut ys = Vec::with_capacity(total);
            let (mut x, mut y) = (0i32, 0i32);
            for &flag in point_flags {
                let bytes = coords.take(triplet_bytes(flag & 0x7F))?;
                let (dx, dy) = triplet(flag & 0x7F, bytes);
                x += dx;
                y += dy;
                xs.push(x);
                ys.push(y);
            }
            let length = coords.u255()?;
            let code = instructions.take(usize::from(length))?;
            let bbox: [i16; 4] = if has_box(n) {
                let mut b = [0i16; 4];
                for v in &mut b {
                    *v = bboxes.i16()?;
                }
                b
            } else {
                let clamp = |v: i32| v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
                [
                    clamp(*xs.iter().min()?),
                    clamp(*ys.iter().min()?),
                    clamp(*xs.iter().max()?),
                    clamp(*ys.iter().max()?),
                ]
            };
            out.extend_from_slice(&(count as i16).to_be_bytes());
            for v in bbox {
                out.extend_from_slice(&v.to_be_bytes());
            }
            for end in &ends {
                out.extend_from_slice(&end.to_be_bytes());
            }
            out.extend_from_slice(&length.to_be_bytes());
            out.extend_from_slice(code);
            let overlapping = overlap.is_some_and(|bits| bits[n / 8] & (0x80 >> (n % 8)) != 0);
            // Each point's flag (on the curve or not, the first perhaps
            // overlapping), then the moves as words.
            for (i, &flag) in point_flags.iter().enumerate() {
                let mut f = if flag & 0x80 == 0 { 1u8 } else { 0 };
                if i == 0 && overlapping {
                    f |= 0x40;
                }
                out.push(f);
            }
            let mut last = 0i32;
            for &v in &xs {
                out.extend_from_slice(&((v - last) as i16).to_be_bytes());
                last = v;
            }
            last = 0;
            for &v in &ys {
                out.extend_from_slice(&((v - last) as i16).to_be_bytes());
                last = v;
            }
            x_mins.push(bbox[0]);
        }
        out.resize(out.len().div_ceil(4) * 4, 0);
        if out.len() > MAX_FONT_BYTES {
            return None;
        }
    }
    offsets.push(out.len());
    let mut loca = Vec::with_capacity((glyphs + 1) * if long { 4 } else { 2 });
    for offset in offsets {
        if long {
            loca.extend_from_slice(&(offset as u32).to_be_bytes());
        } else {
            let half = u16::try_from(offset / 2).ok()?;
            loca.extend_from_slice(&half.to_be_bytes());
        }
    }
    Some((out, loca, x_mins))
}

/// The `hmtx` a transformed one (section 5.4) makes, its left side
/// bearings left out taken from the glyphs' least x.
fn hmtx(data: &[u8], metrics: usize, x_mins: &[i16]) -> Option<Vec<u8>> {
    let glyphs = x_mins.len();
    if metrics == 0 || metrics > glyphs {
        return None;
    }
    let mut r = Bytes::new(data);
    let flags = r.u8()?;
    let advances = r.take(metrics * 2)?;
    let proportional = if flags & 1 == 0 {
        Some(r.take(metrics * 2)?)
    } else {
        None
    };
    let mono = if flags & 2 == 0 {
        Some(r.take((glyphs - metrics) * 2)?)
    } else {
        None
    };
    let mut out = Vec::with_capacity(metrics * 4 + (glyphs - metrics) * 2);
    for n in 0..metrics {
        out.extend_from_slice(&advances[n * 2..n * 2 + 2]);
        match proportional {
            Some(lsb) => out.extend_from_slice(&lsb[n * 2..n * 2 + 2]),
            None => out.extend_from_slice(&x_mins[n].to_be_bytes()),
        }
    }
    for (k, x_min) in x_mins[metrics..].iter().enumerate() {
        match mono {
            Some(lsb) => out.extend_from_slice(&lsb[k * 2..k * 2 + 2]),
            None => out.extend_from_slice(&x_min.to_be_bytes()),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(data: &[u8], at: usize) -> u16 {
        u16::from_be_bytes([data[at], data[at + 1]])
    }

    fn long(data: &[u8], at: usize) -> u32 {
        u32::from_be_bytes(data[at..at + 4].try_into().unwrap())
    }

    /// Bits written least first, as Brotli reads them.
    #[derive(Default)]
    struct Bits {
        out: Vec<u8>,
        acc: u64,
        count: u32,
    }

    impl Bits {
        fn put(&mut self, value: u64, n: u32) {
            self.acc |= value << self.count;
            self.count += n;
            while self.count >= 8 {
                self.out.push(self.acc as u8);
                self.acc >>= 8;
                self.count -= 8;
            }
        }

        fn align(&mut self) {
            if self.count > 0 {
                self.put(0, 8 - self.count);
            }
        }
    }

    /// `bytes` as a Brotli stream of stored meta-blocks.
    fn brotli_stored(bytes: &[u8]) -> Vec<u8> {
        let mut bits = Bits::default();
        // A window of 2^16.
        bits.put(0, 1);
        for chunk in bytes.chunks(1 << 16) {
            // Not last, four nibbles of length, stored.
            bits.put(0, 1);
            bits.put(0, 2);
            bits.put(chunk.len() as u64 - 1, 16);
            bits.put(1, 1);
            bits.align();
            bits.out.extend_from_slice(chunk);
        }
        // The last meta-block, empty.
        bits.put(3, 2);
        bits.align();
        bits.out
    }

    fn base128(mut v: u32) -> Vec<u8> {
        let mut out = vec![(v & 0x7F) as u8];
        v >>= 7;
        while v > 0 {
            out.insert(0, (v & 0x7F) as u8 | 0x80);
            v >>= 7;
        }
        out
    }

    /// The tables of `sfnt`, by tag, in its order.
    fn tables_of(sfnt: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
        (0..usize::from(word(sfnt, 4)))
            .map(|n| {
                let r = 12 + n * 16;
                let (offset, length) = (long(sfnt, r + 8) as usize, long(sfnt, r + 12) as usize);
                (
                    sfnt[r..r + 4].try_into().unwrap(),
                    sfnt[offset..offset + length].to_vec(),
                )
            })
            .collect()
    }

    /// A simple glyph's flags and moves: its points' flags, unrepeated,
    /// then the x and the y moves.
    fn simple_points(g: &[u8], points: usize, mut at: usize) -> (Vec<u8>, Vec<i32>, Vec<i32>) {
        let mut flags = Vec::new();
        while flags.len() < points {
            let f = g[at];
            at += 1;
            flags.push(f);
            if f & 0x08 != 0 {
                for _ in 0..g[at] {
                    flags.push(f);
                }
                at += 1;
            }
        }
        let mut read = |short: u8, same: u8| -> Vec<i32> {
            flags
                .iter()
                .map(|&f| {
                    if f & short != 0 {
                        let v = i32::from(g[at]);
                        at += 1;
                        if f & same != 0 { v } else { -v }
                    } else if f & same != 0 {
                        0
                    } else {
                        let v = i32::from(word(g, at) as i16);
                        at += 2;
                        v
                    }
                })
                .collect()
        };
        let xs = read(0x02, 0x10);
        let ys = read(0x04, 0x20);
        (flags, xs, ys)
    }

    /// `glyf` transformed (section 5.1), every point by the four-byte
    /// triplet, the boxes of composite glyphs alone written; its `loca`
    /// long.
    fn transformed_glyf(glyf: &[u8], loca: &[u8], glyphs: usize) -> Vec<u8> {
        let offset = |n: usize| long(loca, n * 4) as usize;
        let mut s: [Vec<u8>; 7] = Default::default();
        let mut bitmap = vec![0u8; glyphs.div_ceil(32) * 4];
        for n in 0..glyphs {
            let g = &glyf[offset(n)..offset(n + 1)];
            let contours = if g.is_empty() { 0 } else { word(g, 0) as i16 };
            s[0].extend_from_slice(&contours.to_be_bytes());
            if contours == 0 {
                continue;
            }
            if contours < 0 {
                bitmap[n / 8] |= 0x80 >> (n % 8);
                s[5].extend_from_slice(&g[2..10]);
                let mut at = 10;
                let mut instructed = false;
                loop {
                    let flag = word(g, at);
                    let mut size = 4 + if flag & 1 != 0 { 4 } else { 2 };
                    size += if flag & 0x08 != 0 {
                        2
                    } else if flag & 0x40 != 0 {
                        4
                    } else if flag & 0x80 != 0 {
                        8
                    } else {
                        0
                    };
                    s[4].extend_from_slice(&g[at..at + size]);
                    at += size;
                    instructed |= flag & 0x100 != 0;
                    if flag & 0x20 == 0 {
                        break;
                    }
                }
                if instructed {
                    let length = usize::from(word(g, at));
                    s[3].push(253);
                    s[3].extend_from_slice(&(length as u16).to_be_bytes());
                    s[6].extend_from_slice(&g[at + 2..at + 2 + length]);
                }
                continue;
            }
            let contours = contours as usize;
            let mut previous = 0usize;
            for c in 0..contours {
                let end = usize::from(word(g, 10 + c * 2)) + 1;
                s[1].push(253);
                s[1].extend_from_slice(&((end - previous) as u16).to_be_bytes());
                previous = end;
            }
            let at = 10 + contours * 2;
            let length = usize::from(word(g, at));
            let (flags, xs, ys) = simple_points(g, previous, at + 2 + length);
            for ((&f, &dx), &dy) in flags.iter().zip(&xs).zip(&ys) {
                let signs = u8::from(dx >= 0) | (u8::from(dy >= 0) << 1);
                s[2].push((124 + signs) | if f & 1 == 0 { 0x80 } else { 0 });
                s[3].extend_from_slice(&(dx.unsigned_abs() as u16).to_be_bytes());
                s[3].extend_from_slice(&(dy.unsigned_abs() as u16).to_be_bytes());
            }
            s[3].push(253);
            s[3].extend_from_slice(&(length as u16).to_be_bytes());
            s[6].extend_from_slice(&g[at + 2..at + 2 + length]);
        }
        bitmap.extend_from_slice(&s[5]);
        s[5] = bitmap;
        let mut out = vec![0, 0, 0, 0];
        out.extend_from_slice(&(glyphs as u16).to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        for stream in &s {
            out.extend_from_slice(&(stream.len() as u32).to_be_bytes());
        }
        for stream in &s {
            out.extend_from_slice(stream);
        }
        out
    }

    /// `sfnt` packed as WOFF 2, its `glyf` and `loca` transformed when
    /// `transform`.
    fn woff2_of(sfnt: &[u8], transform: bool) -> Vec<u8> {
        let tables = tables_of(sfnt);
        let table = |tag: &[u8; 4]| &tables.iter().find(|(t, _)| t == tag).unwrap().1;
        let glyphs = usize::from(word(table(b"maxp"), 4));
        // The test font's `loca` is long, as the transform writes it.
        assert_eq!(word(table(b"head"), 50), 1);
        let mut directory = Vec::new();
        let mut stream = Vec::new();
        for (tag, body) in &tables {
            let known = KNOWN_TAGS.iter().position(|k| *k == tag);
            let glyph_table = tag == b"glyf" || tag == b"loca";
            let version = if glyph_table && !transform { 3 } else { 0 };
            directory.push(known.map_or(63, |n| n as u8) | (version << 6));
            if known.is_none() {
                directory.extend_from_slice(tag);
            }
            directory.extend(base128(body.len() as u32));
            if glyph_table && transform {
                let data = if tag == b"glyf" {
                    transformed_glyf(body, table(b"loca"), glyphs)
                } else {
                    Vec::new()
                };
                directory.extend(base128(data.len() as u32));
                stream.extend_from_slice(&data);
            } else {
                stream.extend_from_slice(body);
            }
        }
        let packed = brotli_stored(&stream);
        let mut out = Vec::new();
        out.extend_from_slice(b"wOF2");
        out.extend_from_slice(&sfnt[0..4]);
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&(sfnt.len() as u32).to_be_bytes());
        out.extend_from_slice(&(packed.len() as u32).to_be_bytes());
        out.resize(48, 0);
        out.extend_from_slice(&directory);
        out.extend_from_slice(&packed);
        let length = out.len() as u32;
        out[8..12].copy_from_slice(&length.to_be_bytes());
        out
    }

    /// Each glyph's outline's box and advance in `data`, a font.
    fn glyphs_of(data: &[u8]) -> Vec<(Option<ttf_parser::Rect>, Option<u16>)> {
        struct Nothing;
        impl ttf_parser::OutlineBuilder for Nothing {
            fn move_to(&mut self, _: f32, _: f32) {}
            fn line_to(&mut self, _: f32, _: f32) {}
            fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
            fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
            fn close(&mut self) {}
        }
        let face = ttf_parser::Face::parse(data, 0).expect("a font");
        (0..face.number_of_glyphs())
            .map(|n| {
                let id = ttf_parser::GlyphId(n);
                (
                    face.outline_glyph(id, &mut Nothing),
                    face.glyph_hor_advance(id),
                )
            })
            .collect()
    }

    #[test]
    fn a_woff2_file_unpacks_to_its_fonts_glyphs() {
        let original = glyphs_of(crate::TEST_FONT);
        let was = crate::FontStore::from_bytes(crate::TEST_FONT.to_vec(), 0).expect("a font");
        for transform in [false, true] {
            let sfnt = crate::woff::sfnt(woff2_of(crate::TEST_FONT, transform)).expect("unpacks");
            assert!(glyphs_of(&sfnt) == original, "transformed: {transform}");
            let font = crate::FontStore::from_bytes(sfnt, 0).expect("a font");
            assert_eq!(font.measure("Silver", 16.0), was.measure("Silver", 16.0));
        }
    }

    #[test]
    fn the_web_platform_tests_woff2_fonts_unpack() {
        // Made by Google's encoder: Brotli compressed, `glyf` transformed.
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../engine/tests/wpt/");
        for (file, text) in [
            (
                "fonts/noto/NotoNaskhArabic-regular.woff2",
                "\u{628}\u{64a}\u{62a}",
            ),
            (
                "css/css-fonts/resources/NotoSansMongolian-18xx.woff2",
                "\u{1820}\u{1821}",
            ),
            (
                "css/css-fonts/variations/resources/FontStyleTest-slnt-VF.woff2",
                "A",
            ),
        ] {
            let data = std::fs::read(format!("{dir}{file}")).expect("in the checkout");
            let sfnt = unpacked(&data).expect(file);
            let glyphs = glyphs_of(&sfnt);
            assert!(
                glyphs
                    .iter()
                    .filter(|(outline, _)| outline.is_some())
                    .count()
                    > 1
            );
            let font = crate::FontStore::from_bytes(sfnt, 0).expect(file);
            let shaped = font.shape(text, 20.0);
            assert!(shaped.iter().all(|g| g.glyph_id != 0), "{file}");
            assert!(font.measure(text, 20.0) > 0.0);
        }
    }

    #[test]
    fn a_truncated_or_oversized_woff2_file_is_refused() {
        let woff2 = woff2_of(crate::TEST_FONT, true);
        for cut in [4, 30, 48, 60, woff2.len() / 2, woff2.len() - 1] {
            assert!(unpacked(&woff2[..cut]).is_none(), "cut at {cut}");
        }
        // An sfnt said to be past the bound.
        let mut huge = woff2.clone();
        huge[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(unpacked(&huge).is_none());
        // A table said to be past the bound: the first's length, after
        // its flags and perhaps its tag.
        let at = if woff2[48] & 0x3F == 63 { 53 } else { 49 };
        let first = Bytes::new(&woff2[at..]).base128().unwrap();
        let mut table = woff2[..at].to_vec();
        table.extend(base128(MAX_FONT_BYTES as u32 + 1));
        table.extend_from_slice(&woff2[at + base128(first).len()..]);
        assert!(unpacked(&table).is_none());
        // The tables of a stream that unpacks to more than they say.
        let mut short = woff2_of(crate::TEST_FONT, false);
        let first = Bytes::new(&short[at..]).base128().unwrap();
        let fewer = base128(first - 1);
        assert_eq!(fewer.len(), base128(first).len());
        short.splice(at..at + fewer.len(), fewer);
        assert!(unpacked(&short).is_none());
        assert!(unpacked(b"wOF2").is_none());
        // A number with a leading zero, or past 32 bits, is none.
        assert_eq!(Bytes::new(&[0x80, 0x01]).base128(), None);
        assert_eq!(Bytes::new(&[0x90, 0x80, 0x80, 0x80, 0x00]).base128(), None);
        assert_eq!(Bytes::new(&[0x81, 0x00]).base128(), Some(128));
        assert_eq!(Bytes::new(&[254, 3]).u255(), Some(509));
    }
}
