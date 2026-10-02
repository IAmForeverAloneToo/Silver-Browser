//! The few Infra Standard primitives the rest of `net` shares: isomorphic
//! decoding and encoding, which map bytes to the code points U+0000 to
//! U+00FF one for one and back.

/// *Isomorphic decode*: each byte becomes the code point of the same
/// value.
pub(crate) fn isomorphic_decode(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// *Isomorphic encode*: each code point becomes the byte of the same
/// value. The caller guarantees there is none past U+00FF.
pub(crate) fn isomorphic_encode(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| u8::try_from(c).expect("no code point past U+00FF"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isomorphic_round_trip_covers_every_byte() {
        let bytes: Vec<u8> = (0..=255).collect();
        let text = isomorphic_decode(&bytes);
        assert_eq!(text.chars().count(), 256);
        assert_eq!(text.chars().nth(0xE9), Some('é'));
        assert_eq!(isomorphic_encode(&text), bytes);
    }
}
