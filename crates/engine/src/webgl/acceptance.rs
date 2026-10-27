//! M41's acceptance pages (docs/design/gpu.md, M41.6): `accept.js`'s
//! `accept(log)` draws with WebGL 1 and 2 and WebGPU and says pixels and
//! values at points the edges do not touch, a line each; `REFERENCE` is
//! what the reference browser (Chrome) said, read on the probe machine.
//! The engine's tests run the pages in one process, the shell's
//! `--check-processes webgl` through a renderer and the GPU process.

/// The script: `accept(log)`, an async function.
pub const SCRIPT: &str = include_str!("accept.js");

/// The reference's lines, in order. Without WebGPU in the page, the
/// pages say `webgpu none` in place of the last two.
pub const REFERENCE: &[&str] = &[
    "webgl1 cube 0 32,32=51,13,13,255 24,40=237,59,59,255 40,24=31,31,123,255 20,28=51,13,13,255 44,38=31,31,123,255 4,4=26,26,26,255 60,58=26,26,26,255",
    "points 0 16,16=142,113,255,255 18,18=198,57,255,255 48,48=142,113,255,255 46,50=85,57,255,255 16,24=26,26,26,255 32,32=26,26,26,255",
    "framebuffer 8cd5 2,8=255,255,0,255 13,8=0,0,255,255",
    "sampled 0 10,32=255,255,0,255 54,32=0,0,255,255",
    "webgl2 cube 0 32,32=51,13,13,255 24,40=237,59,59,255 40,24=31,31,123,255 20,28=51,13,13,255 44,38=31,31,123,255 4,4=26,26,26,255 60,58=26,26,26,255",
    "webgpu triangle 16,16=131,131,128,255 10,24=84,195,128,255 22,24=179,195,128,255 2,2=0,0,0,255 30,4=0,0,0,255",
    "webgpu compute 0,31,62,93,124,155,186,217,248,279,310,341,372,403,434,465",
];

/// How far a number of a line may be from the reference's: the GPUs'
/// rounding of lighting and coordinates.
pub const TOLERANCE: i64 = 3;

/// Whether a line is the reference's: the same words, and numbers
/// within [`TOLERANCE`].
pub fn near(got: &str, want: &str) -> bool {
    let words = |s: &str| -> Vec<String> { s.split([' ', ',', '=']).map(str::to_owned).collect() };
    let (got, want) = (words(got), words(want));
    got.len() == want.len()
        && got
            .iter()
            .zip(&want)
            .all(|(g, w)| match (g.parse::<i64>(), w.parse::<i64>()) {
                (Ok(g), Ok(w)) => (g - w).abs() <= TOLERANCE,
                _ => g == w,
            })
}

/// Where the pages' `lines` differ from the reference: none when they
/// match it, else a line saying what was wanted for each.
pub fn differences(lines: &[String]) -> Vec<String> {
    let gpu = !lines.iter().any(|l| l == "webgpu none");
    let wanted = if gpu { REFERENCE } else { &REFERENCE[..5] };
    let got: Vec<&str> = lines
        .iter()
        .map(String::as_str)
        .filter(|l| *l != "webgpu none")
        .collect();
    if got.len() != wanted.len() {
        return vec![format!(
            "{} lines, wanted {}: {got:?}",
            got.len(),
            wanted.len()
        )];
    }
    got.iter()
        .zip(wanted)
        .filter(|(got, want)| !near(got, want))
        .map(|(got, want)| format!("got {got}, wanted {want}"))
        .collect()
}
