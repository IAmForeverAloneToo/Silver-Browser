//! WOFF 1 (W3C WOFF File Format 1.0): a font file whose tables are each
//! compressed by zlib, unpacked to the sfnt they came from (D54). Other
//! data is taken as an sfnt as it is; WOFF 2 is refused (a later gap:
//! its Brotli and transforms need a decoder).

use std::io::Read as _;

use super::FontError;
use super::web::MAX_FONT_BYTES;

/// `data` as an sfnt: unpacked when it is WOFF 1.
pub(crate) fn sfnt(data: Vec<u8>) -> Result<Vec<u8>, FontError> {
    if data.len() > MAX_FONT_BYTES {
        return Err(FontError::InvalidFont);
    }
    match data.get(..4) {
        Some(b"wOFF") => unpack(&data),
        Some(b"wOF2") => Err(FontError::InvalidFont),
        _ => Ok(data),
    }
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// The sfnt a WOFF 1 file holds.
fn unpack(data: &[u8]) -> Result<Vec<u8>, FontError> {
    unpacked(data).ok_or(FontError::InvalidFont)
}

fn unpacked(data: &[u8]) -> Option<Vec<u8>> {
    let flavor = u32_at(data, 4)?;
    let count = usize::from(u16_at(data, 12)?);
    let total = u32_at(data, 16)? as usize;
    if count == 0 || total > MAX_FONT_BYTES {
        return None;
    }
    // The sfnt's offset table, then its table records, then the tables,
    // each on four bytes.
    let mut out = Vec::with_capacity(total.min(MAX_FONT_BYTES));
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
    let records = out.len();
    out.resize(records + count * 16, 0);
    for n in 0..count {
        let entry = 44 + n * 20;
        let tag = data.get(entry..entry + 4)?;
        let offset = u32_at(data, entry + 4)? as usize;
        let packed = u32_at(data, entry + 8)? as usize;
        let length = u32_at(data, entry + 12)? as usize;
        let checksum = u32_at(data, entry + 16)?;
        let body = data.get(offset..offset.checked_add(packed)?)?;
        let at = out.len();
        if at + length > MAX_FONT_BYTES || packed > length {
            return None;
        }
        if packed < length {
            let mut table = Vec::with_capacity(length);
            flate2::read::ZlibDecoder::new(body)
                .take(length as u64 + 1)
                .read_to_end(&mut table)
                .ok()?;
            if table.len() != length {
                return None;
            }
            out.extend_from_slice(&table);
        } else {
            out.extend_from_slice(body);
        }
        while out.len() % 4 != 0 {
            out.push(0);
        }
        let record = records + n * 16;
        out[record..record + 4].copy_from_slice(tag);
        out[record + 4..record + 8].copy_from_slice(&checksum.to_be_bytes());
        out[record + 8..record + 12].copy_from_slice(&(at as u32).to_be_bytes());
        out[record + 12..record + 16].copy_from_slice(&(length as u32).to_be_bytes());
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// `sfnt` packed as WOFF 1, each table compressed.
    pub(crate) fn woff_of(sfnt: &[u8]) -> Vec<u8> {
        let count = usize::from(u16::from_be_bytes([sfnt[4], sfnt[5]]));
        let mut tables = Vec::new();
        for n in 0..count {
            let record = 12 + n * 16;
            let tag = &sfnt[record..record + 4];
            let checksum = &sfnt[record + 4..record + 8];
            let offset =
                u32::from_be_bytes(sfnt[record + 8..record + 12].try_into().unwrap()) as usize;
            let length =
                u32::from_be_bytes(sfnt[record + 12..record + 16].try_into().unwrap()) as usize;
            let body = &sfnt[offset..offset + length];
            let mut packed =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            packed.write_all(body).unwrap();
            let packed = packed.finish().unwrap();
            let stored = if packed.len() < body.len() {
                packed
            } else {
                body.to_vec()
            };
            tables.push((tag.to_vec(), checksum.to_vec(), stored, length));
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"wOFF");
        out.extend_from_slice(&sfnt[0..4]);
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(count as u16).to_be_bytes());
        out.extend_from_slice(&[0; 2]);
        out.extend_from_slice(&(sfnt.len() as u32).to_be_bytes());
        out.resize(44, 0);
        let mut at = 44 + count * 20;
        let mut bodies = Vec::new();
        for (tag, checksum, stored, length) in &tables {
            out.extend_from_slice(tag);
            out.extend_from_slice(&(at as u32).to_be_bytes());
            out.extend_from_slice(&(stored.len() as u32).to_be_bytes());
            out.extend_from_slice(&(*length as u32).to_be_bytes());
            out.extend_from_slice(checksum);
            bodies.extend_from_slice(stored);
            while bodies.len() % 4 != 0 {
                bodies.push(0);
            }
            at = 44 + count * 20 + bodies.len();
        }
        out.extend_from_slice(&bodies);
        out
    }

    #[test]
    fn a_woff_file_unpacks_to_a_font_that_measures_as_its_sfnt() {
        let woff = woff_of(crate::TEST_FONT);
        assert!(woff.len() < crate::TEST_FONT.len());
        let sfnt = sfnt(woff).expect("unpacks");
        // Each table on four bytes.
        let count = usize::from(u16_at(&sfnt, 4).unwrap());
        for n in 0..count {
            assert_eq!(u32_at(&sfnt, 12 + n * 16 + 8).unwrap() % 4, 0);
        }
        let unpacked = crate::FontStore::from_bytes(sfnt, 0).expect("a font");
        let original = crate::FontStore::from_bytes(crate::TEST_FONT.to_vec(), 0).expect("a font");
        assert_eq!(
            unpacked.measure("Silver", 16.0),
            original.measure("Silver", 16.0)
        );
    }

    #[test]
    fn what_is_broken_or_too_large_or_woff2_is_refused() {
        let woff = woff_of(crate::TEST_FONT);
        for cut in [8, 44, 60, woff.len() / 2] {
            assert!(sfnt(woff[..cut].to_vec()).is_err(), "cut at {cut}");
        }
        let mut huge = woff.clone();
        huge[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(sfnt(huge).is_err());
        let mut lying = woff.clone();
        // The first table says it unpacks to a byte more than it does.
        let length = u32_at(&lying, 44 + 12).unwrap() + 1;
        lying[56..60].copy_from_slice(&length.to_be_bytes());
        assert!(sfnt(lying).is_err());
        // A compressed table that unpacks to more than it says.
        let packed = (0..usize::from(u16_at(&woff, 12).unwrap()))
            .map(|n| 44 + n * 20)
            .find(|&e| u32_at(&woff, e + 8) < u32_at(&woff, e + 12))
            .expect("a compressed table");
        let mut longer = woff.clone();
        let length = u32_at(&longer, packed + 12).unwrap() - 1;
        longer[packed + 12..packed + 16].copy_from_slice(&length.to_be_bytes());
        assert!(sfnt(longer).is_err());
        assert!(sfnt(b"wOF2 and the rest".to_vec()).is_err());
        assert_eq!(sfnt(b"plain".to_vec()), Ok(b"plain".to_vec()));
    }
}
