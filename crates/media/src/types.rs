//! `canPlayType()` (HTML 4.8.11.3): what a MIME type and its `codecs`
//! say this crate plays. The answers are the reference browser's for what
//! both play (probe p1), and `""` for H.264, AAC, VP8, VP9 and Theora,
//! which Chromium's open-source build answers so for the first two (D36).

/// `canPlayType()`'s answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanPlay {
    No,
    Maybe,
    Probably,
}

impl CanPlay {
    pub fn as_str(self) -> &'static str {
        match self {
            CanPlay::No => "",
            CanPlay::Maybe => "maybe",
            CanPlay::Probably => "probably",
        }
    }
}

/// The containers, and the codecs each carries here.
fn container(essence: &str) -> Option<(&'static [&'static str], bool)> {
    // (codecs, whether the type alone is certain)
    const WEBM_VIDEO: &[&str] = &["opus", "vorbis", "av01"];
    const WEBM_AUDIO: &[&str] = &["opus", "vorbis"];
    const MP4_VIDEO: &[&str] = &["opus", "flac", "mp3", "mp4a.69", "mp4a.6b", "av01"];
    const MP4_AUDIO: &[&str] = &["opus", "flac", "mp3", "mp4a.69", "mp4a.6b"];
    const OGG: &[&str] = &["opus", "vorbis", "flac"];
    const WAV: &[&str] = &["1"];
    const MP3: &[&str] = &["mp3"];
    const FLAC: &[&str] = &["flac"];
    Some(match essence {
        "video/webm" | "video/x-matroska" => (WEBM_VIDEO, false),
        "audio/webm" | "audio/x-matroska" => (WEBM_AUDIO, false),
        "video/mp4" => (MP4_VIDEO, false),
        "audio/mp4" => (MP4_AUDIO, false),
        "audio/ogg" | "application/ogg" | "video/ogg" => (OGG, false),
        "audio/wav" | "audio/x-wav" => (WAV, false),
        "audio/mpeg" | "audio/mp3" => (MP3, true),
        "audio/flac" => (FLAC, true),
        _ => return None,
    })
}

/// Whether `codec`, as a `codecs` parameter names it, is one of `known`:
/// AV1 by its `av01.P.LLT.DD` prefix (a profile of 0, 1 or 2, any level
/// and depth), the others whole, case aside.
fn codec_known(codec: &str, known: &[&str]) -> bool {
    let codec = codec.trim().to_ascii_lowercase();
    known.iter().any(|k| {
        if *k == "av01" {
            let mut parts = codec.split('.');
            parts.next() == Some("av01")
                && parts.next().is_some_and(|p| matches!(p, "0" | "1" | "2"))
                && parts.next().is_some_and(|l| l.len() == 3)
                && parts
                    .next()
                    .is_some_and(|d| matches!(d, "08" | "10" | "12"))
        } else {
            codec == *k
        }
    })
}

/// `canPlayType(type)`.
pub fn can_play_type(text: &str) -> CanPlay {
    let mut parts = text.split(';');
    let essence = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    let Some((known, certain)) = container(&essence) else {
        return CanPlay::No;
    };
    let mut codecs = None;
    for param in parts {
        let Some((name, value)) = param.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("codecs") {
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value);
            codecs = Some(value.to_owned());
        }
    }
    match codecs {
        None => {
            if certain {
                CanPlay::Probably
            } else {
                CanPlay::Maybe
            }
        }
        Some(list) => {
            let all = list.split(',').filter(|c| !c.trim().is_empty());
            let mut any = false;
            for codec in all {
                any = true;
                if !codec_known(codec, known) {
                    return CanPlay::No;
                }
            }
            if any {
                CanPlay::Probably
            } else {
                CanPlay::Maybe
            }
        }
    }
}
