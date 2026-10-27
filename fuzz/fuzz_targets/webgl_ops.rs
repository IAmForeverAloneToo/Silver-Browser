//! WebGL's calls (M41 step 8, D40) driven by arbitrary bytes: any
//! sequence of the calls the bootstrap makes of a WebGL 1 or 2 context,
//! with GL's enums, small numbers, objects by their numbers, shaders
//! that compile and data, into a GPU service on this machine's adapter
//! (none: nothing is checked). No call panics: what is wrong is GL's
//! error to record, and what the translation makes, `wgpu`'s
//! validation's to say.

#![no_main]
#![forbid(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::mem::ManuallyDrop;

use engine::webgl::Harness;
use libfuzzer_sys::fuzz_target;

thread_local! {
    /// A harness of each version for every input, its context made again
    /// every [`AGAIN`] inputs (a device is slow to make); never dropped,
    /// torn down as the process exits.
    static HARNESSES: RefCell<Option<ManuallyDrop<[Harness; 2]>>> = const { RefCell::new(None) };
    static RUNS: Cell<u32> = const { Cell::new(0) };
}

/// The inputs a context is kept for.
const AGAIN: u32 = 32;

/// Reads the input a byte or a few at a time.
struct Input<'a>(&'a [u8]);

impl Input<'_> {
    fn byte(&mut self) -> Option<u8> {
        let (&b, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(b)
    }

    fn small(&mut self, below: usize) -> Option<usize> {
        Some(usize::from(self.byte()?) % below.max(1))
    }

    /// A number around what matters: GL's enums, small sizes and
    /// counts, objects' numbers, and some past any limit.
    fn number(&mut self) -> Option<f64> {
        Some(match self.byte()? {
            b @ 0..=63 => f64::from(b),
            b @ 64..=191 => f64::from(ENUMS[usize::from(b - 64) % ENUMS.len()]),
            b @ 192..=239 => f64::from(1u32 << (b - 192) % 13),
            240 => -1.0,
            241 => 0.5,
            242 => f64::from(u32::MAX),
            243 => 1.0e12,
            244 => f64::NAN,
            _ => f64::from(self.byte()?) / 255.0,
        })
    }
}

/// GL's enums the calls take: targets, formats, types, parameters,
/// attachments, buffers and modes.
const ENUMS: &[u32] = &[
    0x0DE1, 0x8513, 0x8515, 0x8516, 0x806F, 0x8C1A, // textures
    0x8892, 0x8893, 0x8F36, 0x8F37, 0x88EB, 0x88EC, 0x8A11, 0x8C8E, // buffers
    0x8D40, 0x8CA8, 0x8CA9, 0x8D41, // framebuffer, renderbuffer
    0x1906, 0x1907, 0x1908, 0x1909, 0x190A, 0x1902, 0x84F9, 0x1903, 0x8227, 0x8D94, 0x8228,
    0x8D99, // formats
    0x1401, 0x1400, 0x1403, 0x1402, 0x1405, 0x1404, 0x1406, 0x140B, 0x8D61, 0x8363, 0x8033, 0x8034,
    0x8368, 0x84FA, // types
    0x8229, 0x822B, 0x8058, 0x8814, 0x881A, 0x8D7C, 0x8232, 0x822E, 0x8C3A, 0x8C3D, 0x81A5, 0x81A6,
    0x8CAC, 0x88F0, 0x8D48, 0x8056, 0x8D62, 0x8C43, // sized
    0x2800, 0x2801, 0x2802, 0x2803, 0x8072, 0x884C, 0x884D, 0x813C, 0x813D, 0x2600, 0x2601, 0x2703,
    0x812F, 0x8370, 0x2901, 0x884E, // parameters
    0x8CE0, 0x8CE1, 0x8CE2, 0x8D00, 0x8D20, 0x821A, 0x0405, 0x1800, 0x1801,
    0x1802, // attachments
    0x4000, 0x0100, 0x0400, 0x4500, // clear bits
    0x0000, 0x0001, 0x0002, 0x0003, 0x0004, 0x0005, 0x0006, // modes
    0x0B71, 0x0BE2, 0x0B44, 0x0C11, 0x0B90, 0x8037, 0x809E, 0x80A0, // capabilities
    0x8B31, 0x8B30, 0x8B81, 0x8B82, 0x88E4, 0x88E8, 0x8C2F, 0x8D6A, 0x9117,
];

/// Shaders that compile, for programs that link and draw.
const SHADERS: &[&str] = &[
    "attribute vec2 p; void main() { gl_Position = vec4(p, 0.0, 1.0); gl_PointSize = 4.0; }",
    "precision mediump float; uniform vec4 c; uniform sampler2D t; void main() { gl_FragColor = c + texture2D(t, vec2(0.5)); }",
    "#version 300 es\nlayout(location = 0) in vec2 p; void main() { gl_Position = vec4(p, 0.0, 1.0); }",
    "#version 300 es\nprecision mediump float; uniform B { vec4 c; }; uniform mediump usampler2D u; layout(location = 0) out vec4 o; layout(location = 1) out uvec4 n; void main() { o = c; n = texelFetch(u, ivec2(0), 0); }",
    "#version 300 es\nprecision mediump float; uniform mediump sampler3D t; uniform mediump sampler2DArray a; uniform mediump sampler2DShadow s; out vec4 o; void main() { o = texture(t, vec3(0.5)) + texture(a, vec3(0.5)) + vec4(texture(s, vec3(0.5))); }",
];

/// The calls, by the bootstrap's names: their numbers after the
/// context's, a string, and bytes.
const CALLS: &[(&str, usize, bool, bool)] = &[
    ("createBuffer", 0, false, false),
    ("bindBuffer", 2, false, false),
    ("bufferData", 3, false, true),
    ("bufferSubData", 2, false, true),
    ("createTexture", 0, false, false),
    ("bindTexture", 2, false, false),
    ("activeTexture", 1, false, false),
    ("texImage2D", 11, false, true),
    ("texImage3D", 12, false, true),
    ("texSubImage2D", 11, false, true),
    ("texSubImage3D", 13, false, true),
    ("texStorage2D", 5, false, false),
    ("texStorage3D", 6, false, false),
    ("texParameteri", 3, false, false),
    ("generateMipmap", 1, false, false),
    ("copyTexImage2D", 8, false, false),
    ("copyTexSubImage2D", 8, false, false),
    ("createFramebuffer", 0, false, false),
    ("bindFramebuffer", 2, false, false),
    ("framebufferTexture2D", 5, false, false),
    ("framebufferTextureLayer", 5, false, false),
    ("createRenderbuffer", 0, false, false),
    ("bindRenderbuffer", 2, false, false),
    ("renderbufferStorage", 4, false, false),
    ("renderbufferStorageMultisample", 5, false, false),
    ("framebufferRenderbuffer", 4, false, false),
    ("drawBuffers", 2, false, false),
    ("readBuffer", 1, false, false),
    ("clear", 1, false, false),
    ("clearColor", 4, false, false),
    ("clearBuffer", 7, false, false),
    ("blitFramebuffer", 10, false, false),
    ("readPixels", 8, false, true),
    ("createShader", 1, false, false),
    ("shaderSource", 1, true, false),
    ("compileShader", 1, false, false),
    ("createProgram", 0, false, false),
    ("attachShader", 2, false, false),
    ("linkProgram", 1, false, false),
    ("useProgram", 1, false, false),
    ("getUniformLocation", 1, true, false),
    ("uniform", 10, false, false),
    ("uniformBlockBinding", 3, false, false),
    ("bindBufferBase", 3, false, false),
    ("enableVertexAttribArray", 1, false, false),
    ("vertexAttribPointer", 6, false, false),
    ("vertexAttribIPointer", 5, false, false),
    ("vertexAttribDivisor", 2, false, false),
    ("drawArrays", 3, false, false),
    ("drawArraysInstanced", 4, false, false),
    ("drawElements", 4, false, false),
    ("drawElementsInstanced", 5, false, false),
    ("enable", 1, false, false),
    ("disable", 1, false, false),
    ("viewport", 4, false, false),
    ("scissor", 4, false, false),
    ("colorMask", 4, false, false),
    ("blendFunc", 2, false, false),
    ("depthFunc", 1, false, false),
    ("stencilFunc", 3, false, false),
    ("pixelStorei", 2, false, false),
    ("createVertexArray", 0, false, false),
    ("bindVertexArray", 1, false, false),
    ("createSampler", 0, false, false),
    ("bindSampler", 2, false, false),
    ("samplerParameteri", 3, false, false),
    ("createQuery", 0, false, false),
    ("beginQuery", 2, false, false),
    ("endQuery", 1, false, false),
    ("fenceSync", 2, false, false),
    ("getExtension", 0, true, false),
    ("deleteTexture", 1, false, false),
    ("deleteFramebuffer", 1, false, false),
    ("deleteBuffer", 1, false, false),
    ("deleteProgram", 1, false, false),
    ("getParameter", 1, false, false),
    ("getError", 0, false, false),
    ("present", 0, false, false),
    ("loseContext", 0, false, false),
    ("restoreContext", 0, false, false),
];

const EXTENSIONS: &[&str] = &[
    "OES_texture_float",
    "OES_texture_half_float",
    "WEBGL_depth_texture",
    "WEBGL_draw_buffers",
    "EXT_color_buffer_float",
    "OES_element_index_uint",
    "WEBGL_compressed_texture_s3tc",
    "EXT_texture_filter_anisotropic",
];

/// A program linked from two of the shaders, used, and a buffer at
/// attribute 0: what most inputs could not make by chance.
fn prelude(h: &mut Harness, two: bool) {
    let (vertex, fragment) = if two { (2, 3) } else { (0, 1) };
    h.call("createShader", &[f64::from(0x8B31u32)], Vec::new(), None);
    h.call("createShader", &[f64::from(0x8B30u32)], Vec::new(), None);
    // The shaders are 1 and 2, the program 3, the buffer 4.
    h.call(
        "shaderSource",
        &[1.0],
        vec![SHADERS[vertex].to_owned()],
        None,
    );
    h.call(
        "shaderSource",
        &[2.0],
        vec![SHADERS[fragment].to_owned()],
        None,
    );
    h.call("compileShader", &[1.0], Vec::new(), None);
    h.call("compileShader", &[2.0], Vec::new(), None);
    h.call("createProgram", &[], Vec::new(), None);
    h.call("attachShader", &[3.0, 1.0], Vec::new(), None);
    h.call("attachShader", &[3.0, 2.0], Vec::new(), None);
    h.call("linkProgram", &[3.0], Vec::new(), None);
    h.call("useProgram", &[3.0], Vec::new(), None);
    h.call("createBuffer", &[], Vec::new(), None);
    h.call("bindBuffer", &[f64::from(0x8892u32), 4.0], Vec::new(), None);
    let quad: Vec<u8> = [-1.0f32, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    h.call(
        "bufferData",
        &[f64::from(0x8892u32), 0.0, f64::from(0x88E4u32)],
        Vec::new(),
        Some(quad),
    );
    h.call("enableVertexAttribArray", &[0.0], Vec::new(), None);
    h.call(
        "vertexAttribPointer",
        &[0.0, 2.0, f64::from(0x1406u32), 0.0, 0.0, 0.0],
        Vec::new(),
        None,
    );
}

fuzz_target!(|data: &[u8]| {
    let mut input = Input(data);
    let Some(first) = input.byte() else {
        return;
    };
    HARNESSES.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.is_none() {
            let (Some(one), Some(two)) = (engine::gpu::local_client(), engine::gpu::local_client())
            else {
                return;
            };
            *cell = Some(ManuallyDrop::new([Harness::new(one), Harness::new(two)]));
        }
        let runs = RUNS.get();
        RUNS.set(runs + 1);
        let two = first & 1 == 1;
        let h = &mut cell.as_mut().expect("made")[usize::from(two)];
        if runs % AGAIN < 2 || !h.made() {
            h.call("loseContext", &[], Vec::new(), None);
            if !h.create(if two { 2 } else { 1 }) {
                return;
            }
        }
        if first & 2 == 2 {
            prelude(h, two);
        }
        for _ in 0..256 {
            let Some(k) = input.small(CALLS.len()) else {
                break;
            };
            let (name, count, string, bytes) = CALLS[k];
            let mut numbers = Vec::with_capacity(count);
            for _ in 0..count {
                let Some(v) = input.number() else {
                    break;
                };
                numbers.push(v);
            }
            let strings = if string {
                let pick = input
                    .small(SHADERS.len() + EXTENSIONS.len() + 1)
                    .unwrap_or(0);
                vec![match pick {
                    p if p < SHADERS.len() => SHADERS[p].to_owned(),
                    p if p < SHADERS.len() + EXTENSIONS.len() => {
                        EXTENSIONS[p - SHADERS.len()].to_owned()
                    }
                    _ => "c".to_owned(),
                }]
            } else {
                Vec::new()
            };
            let data = if bytes {
                let length = input.small(64).unwrap_or(0) * 4;
                let fill = input.byte().unwrap_or(0);
                Some(vec![fill; length])
            } else {
                None
            };
            h.call(name, &numbers, strings, data);
        }
        // What was drawn sent, as a task's end would.
        h.call("flush", &[], Vec::new(), None);
    });
});
