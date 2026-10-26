//! Any text as a WebGL shader (M41 step 5, D40): GLSL ES 1.00 or 3.00,
//! of either stage, with or without WebGL 1's extensions. Nothing
//! panics; a shader that compiles links with a partner of the other
//! stage, its translation one naga validates (the link's errors are the
//! program's, never the translation's).

#![no_main]
#![forbid(unsafe_code)]

use glsl::{Extensions, LinkOptions, Options, Stage, compile, link, partner};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&flags, rest)) = data.split_first() else {
        return;
    };
    let stage = if flags & 1 == 0 {
        Stage::Vertex
    } else {
        Stage::Fragment
    };
    let options = Options {
        webgl2: flags & 2 != 0,
        extensions: Extensions {
            standard_derivatives: flags & 4 != 0,
            frag_depth: flags & 8 != 0,
            draw_buffers: flags & 16 != 0,
            shader_texture_lod: flags & 32 != 0,
        },
        ..Options::default()
    };
    let source = String::from_utf8_lossy(rest);
    let Ok(shader) = compile(&source, stage, &options) else {
        return;
    };
    let other_stage = if stage == Stage::Vertex {
        Stage::Fragment
    } else {
        Stage::Vertex
    };
    let other = compile(&partner(&shader), other_stage, &options).expect("the partner compiles");
    let (vertex, fragment) = if stage == Stage::Vertex {
        (&shader, &other)
    } else {
        (&other, &shader)
    };
    if let Err(log) = link(vertex, fragment, &LinkOptions::default()) {
        assert!(!log.starts_with("Internal error"), "{log}\n{source}");
    }
});
