//! The one EXIF field read: the orientation, tag 0x0112 of IFD0.

use crate::Orientation;

/// The orientation `exif` records: a TIFF structure, after an optional
/// `Exif\0\0` prefix. Anything malformed reads as `Normal`.
pub fn orientation(exif: &[u8]) -> Orientation {
    let tiff = exif.strip_prefix(b"Exif\0\0").unwrap_or(exif);
    read(tiff).map_or(Orientation::Normal, Orientation::from_exif)
}

fn read(tiff: &[u8]) -> Option<u16> {
    let big = match tiff.get(..4)? {
        b"II*\0" => false,
        b"MM\0*" => true,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b: [u8; 2] = tiff.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(if big {
            u16::from_be_bytes(b)
        } else {
            u16::from_le_bytes(b)
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b: [u8; 4] = tiff.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if big {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        })
    };
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let entries = u16_at(ifd)?;
    for i in 0..usize::from(entries) {
        let entry = ifd.checked_add(2 + 12 * i)?;
        // Tag 0x0112, type SHORT (3), one value, held in the entry itself.
        if u16_at(entry)? == 0x0112 && u16_at(entry + 2)? == 3 && u32_at(entry + 4)? == 1 {
            return u16_at(entry + 8);
        }
    }
    None
}
