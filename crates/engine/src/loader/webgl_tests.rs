//! WebGL's core (M41 step 6, D40): contexts, their state and errors,
//! buffers, shaders, programs, uniforms and draws, drawn through this
//! process's GPU service and presented as the canvas. Where there is no
//! adapter, a test says so.

use std::time::{Duration, Instant};

use super::webgpu_tests::server;
use crate::FrameTime;
use crate::testing::Tab;

/// The script's first lines: a 4 by 2 canvas's context `gl`, or "no gl"
/// and the end.
const CONTEXT: &str = r#"
const canvas = document.createElement('canvas');
canvas.width = 4;
canvas.height = 2;
document.documentElement.appendChild(canvas);
const gl = canvas.getContext('webgl', {antialias: false});
if (gl === null) { log('no gl'); return; }
const program = (vs, fs) => {
  const p = gl.createProgram();
  for (const [type, source] of [[gl.VERTEX_SHADER, vs], [gl.FRAGMENT_SHADER, fs]]) {
    const s = gl.createShader(type);
    gl.shaderSource(s, source);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) log('compile ' + gl.getShaderInfoLog(s));
    gl.attachShader(p, s);
  }
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) log('link ' + gl.getProgramInfoLog(p));
  return p;
};
// The canvas's pixels, read through a 2D canvas once presented.
// The canvas's pixels once a frame other than `last` is presented.
const pixels = async (last) => {
  await new Promise(r => requestAnimationFrame(r));
  const read = document.createElement('canvas').getContext('2d');
  read.canvas.width = 4;
  read.canvas.height = 2;
  let text = '';
  for (let i = 0; i < 300 && (text === '' || text === last || text.endsWith(' 0,0,0,0')); i++) {
    await new Promise(r => setTimeout(r, 10));
    read.clearRect(0, 0, 4, 2);
    read.drawImage(canvas, 0, 0);
    const data = Array.from(read.getImageData(0, 0, 4, 2).data);
    const out = [];
    for (let k = 0; k < data.length; k += 4) out.push(data.slice(k, k + 4).join(','));
    text = out.join(' ');
  }
  return text;
};
"#;

/// The page's lines, with the engine's frames run as the shell runs
/// them; `None` (said) where there is no adapter.
fn run(script: &str) -> Option<Vec<String>> {
    run_page(CONTEXT, script)
}

/// The page of `context`'s lines and `script`'s, run as [`run`] says.
fn run_page(context: &str, script: &str) -> Option<Vec<String>> {
    let page = server(&format!("{context}\n{script}"));
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(page.url("/page.html"));
    let began = Instant::now();
    let mut lines: Vec<String> = Vec::new();
    let mut at = Duration::from_secs(1);
    loop {
        tab.engine.poll_network();
        tab.engine.run_tasks();
        lines.extend(
            tab.engine
                .take_console()
                .into_iter()
                .map(|entry| entry.text),
        );
        if lines.iter().any(|l| l == "done" || l.starts_with("failed")) {
            break;
        }
        if tab.engine.needs_frame() {
            at += Duration::from_millis(16);
            tab.engine.frame_at(
                64,
                64,
                FrameTime {
                    now: at,
                    deadline: at + Duration::from_millis(16),
                },
            );
        }
        assert!(began.elapsed() < Duration::from_secs(30), "{lines:#?}");
        std::thread::sleep(Duration::from_millis(2));
    }
    lines.retain(|l| l != "done");
    if lines == ["no gl"] {
        eprintln!("no adapter here: WebGL is not checked");
        return None;
    }
    Some(lines)
}

#[test]
fn a_context_has_its_interfaces_attributes_state_and_errors() {
    let Some(lines) = run(r#"
log('context ' + gl.constructor.name + ' ' + (gl.canvas === canvas) + ' ' + (canvas.getContext('webgl') === gl) + ' ' + canvas.getContext('2d') + ' ' + canvas.getContext('webgl2'));
log('constants ' + gl.TRIANGLES + ' ' + WebGLRenderingContext.COLOR_BUFFER_BIT + ' ' + WebGL2RenderingContext.UNIFORM_BUFFER + ' ' + ('UNIFORM_BUFFER' in gl));
try { new WebGLRenderingContext(); } catch (e) { log('construct ' + e.name); }
const a = gl.getContextAttributes();
log('attributes ' + [a.alpha, a.depth, a.stencil, a.antialias, a.premultipliedAlpha, a.preserveDrawingBuffer].join(' '));
log('buffer ' + gl.drawingBufferWidth + 'x' + gl.drawingBufferHeight + ' ' + gl.isContextLost());
const viewport = gl.getParameter(gl.VIEWPORT);
log('viewport ' + viewport.constructor.name + ' ' + Array.from(viewport).join(','));
gl.clearColor(0.25, 0.5, 1, 2);
const clear = gl.getParameter(gl.COLOR_CLEAR_VALUE);
log('clear ' + clear.constructor.name + ' ' + Array.from(clear).join(','));
gl.colorMask(true, false, true, false);
log('mask ' + gl.getParameter(gl.COLOR_WRITEMASK).join(','));
gl.enable(gl.BLEND);
log('blend ' + gl.isEnabled(gl.BLEND) + ' ' + gl.getParameter(gl.BLEND) + ' ' + gl.getError());
gl.enable(0x1234);
log('bad enable ' + gl.getError() + ' ' + gl.getError());
gl.blendFunc(gl.SRC_ALPHA, gl.CONSTANT_COLOR);
gl.depthFunc(gl.GREATER);
log('state ' + gl.getParameter(gl.BLEND_SRC_RGB) + ' ' + gl.getParameter(gl.BLEND_DST_ALPHA) + ' ' + gl.getParameter(gl.DEPTH_FUNC));
log('strings ' + gl.getParameter(gl.VERSION) + ' / ' + gl.getParameter(gl.SHADING_LANGUAGE_VERSION));
const f = gl.getShaderPrecisionFormat(gl.FRAGMENT_SHADER, gl.HIGH_FLOAT);
log('precision ' + f.constructor.name + ' ' + f.rangeMin + ' ' + f.rangeMax + ' ' + f.precision);
try { gl.bindBuffer(gl.ARRAY_BUFFER, {}); } catch (e) { log('not a buffer ' + e.name); }
try { gl.clearColor(1); } catch (e) { log('too few ' + e.name); }
// Numbered alike in both contexts, but another's.
gl.createBuffer();
const other = document.createElement('canvas').getContext('webgl');
const theirs = other.createBuffer();
other.bindBuffer(other.ARRAY_BUFFER, theirs);
gl.bindBuffer(gl.ARRAY_BUFFER, theirs);
const bound = gl.getError();
// isBuffer of another's is false, with no error.
log('other context ' + bound + ' ' + gl.isBuffer(theirs) + ' ' + gl.getError());
log('extensions ' + (gl.getSupportedExtensions().length >= 15) + ' ' + gl.getExtension('OES_nothing'));
const two = document.createElement('canvas').getContext('webgl2');
log('webgl2 ' + two.constructor.name + ' ' + (two instanceof WebGLRenderingContext) + ' ' + two.getParameter(two.VERSION));
"#)
    else {
        return;
    };
    assert_eq!(
        lines,
        [
            "context WebGLRenderingContext true true null null",
            "constants 4 16384 35345 false",
            "construct TypeError",
            "attributes true true false false true false",
            "buffer 4x2 false",
            "viewport Int32Array 0,0,4,2",
            "clear Float32Array 0.25,0.5,1,1",
            "mask true,false,true,false",
            "blend true true 0",
            "bad enable 1280 0",
            "state 770 32769 516",
            "strings WebGL 1.0 (OpenGL ES 2.0 Chromium) / WebGL GLSL ES 1.0 (OpenGL ES GLSL ES 1.0 Chromium)",
            "precision WebGLShaderPrecisionFormat 127 127 23",
            "not a buffer TypeError",
            "too few TypeError",
            "other context 1282 false 0",
            "extensions true null",
            "webgl2 WebGL2RenderingContext false WebGL 2.0 (OpenGL ES 3.0 Chromium)",
        ]
    );
}

#[test]
fn a_cleared_canvas_and_drawn_triangles_are_presented_the_right_way_up() {
    let Some(lines) = run(r#"
gl.clearColor(0, 0, 1, 1);
gl.clear(gl.COLOR_BUFFER_BIT);
const p = program(
  'attribute vec2 position; void main() { gl_Position = vec4(position, 0.0, 1.0); }',
  'precision mediump float; uniform vec4 color; void main() { gl_FragColor = color; }');
gl.useProgram(p);
// The top half: a strip of two triangles.
const top = gl.createBuffer();
gl.bindBuffer(gl.ARRAY_BUFFER, top);
gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, 0, 1, 0, -1, 1, 1, 1]), gl.STATIC_DRAW);
const position = gl.getAttribLocation(p, 'position');
gl.enableVertexAttribArray(position);
gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
gl.uniform4f(gl.getUniformLocation(p, 'color'), 1, 0, 0, 1);
// GL's counter-clockwise front faces are kept.
gl.enable(gl.CULL_FACE);
gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
// The bottom right quarter: indexed, from bytes.
gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, -1, 1, -1, 0, 0, 1, 0]), gl.STATIC_DRAW);
const indices = gl.createBuffer();
gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, indices);
gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, new Uint8Array([0, 1, 2, 2, 1, 3]), gl.STATIC_DRAW);
gl.uniform4fv(gl.getUniformLocation(p, 'color'), [0, 1, 0, 1]);
gl.drawElements(gl.TRIANGLES, 6, gl.UNSIGNED_BYTE, 0);
log('errors ' + gl.getError());
const first = await pixels();
log('pixels ' + first);
// Not preserved: the next frame's buffer starts cleared to nothing.
gl.drawElements(gl.TRIANGLES, 6, gl.UNSIGNED_BYTE, 0);
log('next ' + await pixels(first));
// Past the buffer's end: refused.
gl.drawElements(gl.TRIANGLES, 6, gl.UNSIGNED_BYTE, 4);
log('past the end ' + gl.getError());
gl.drawArrays(gl.TRIANGLES, 0, 99);
log('too many ' + gl.getError());
gl.useProgram(null);
gl.drawArrays(gl.TRIANGLES, 0, 3);
log('no program ' + gl.getError());
"#)
    else {
        return;
    };
    assert_eq!(
        lines,
        [
            "errors 0",
            "pixels 255,0,0,255 255,0,0,255 255,0,0,255 255,0,0,255 0,0,255,255 0,0,255,255 0,255,0,255 0,255,0,255",
            "next 0,0,0,0 0,0,0,0 0,0,0,0 0,0,0,0 0,0,0,0 0,0,0,0 0,255,0,255 0,255,0,255",
            "past the end 1282",
            "too many 1282",
            "no program 1282",
        ]
    );
}

#[test]
fn programs_say_their_attributes_uniforms_and_errors() {
    let Some(lines) = run(r#"
const p = program(
  'attribute vec4 position; attribute float weight; uniform mat2 turn; varying float v; void main() { v = weight; gl_Position = position + vec4(turn[0], 0.0, 0.0); }',
  'precision mediump float; uniform vec3 colors[2]; uniform int pick; uniform bool on; varying float v; void main() { gl_FragColor = vec4(colors[pick] * v, on ? 1.0 : 0.0); }');
log('linked ' + gl.getProgramParameter(p, gl.LINK_STATUS) + ' ' + gl.getProgramParameter(p, gl.ACTIVE_ATTRIBUTES) + ' ' + gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS) + ' ' + gl.getAttachedShaders(p).length);
const names = [];
for (let i = 0; i < gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS); i++) {
  const info = gl.getActiveUniform(p, i);
  names.push(info.name + ':' + info.size + ':' + info.type);
}
log('uniforms ' + names.sort().join(' '));
const attribute = gl.getActiveAttrib(p, gl.getAttribLocation(p, 'weight') === 0 ? 0 : 1);
log('attribute ' + attribute.constructor.name + ' ' + (attribute.name === 'weight' || attribute.name === 'position'));
log('missing ' + gl.getUniformLocation(p, 'nothing') + ' ' + gl.getAttribLocation(p, 'nothing'));
gl.useProgram(p);
const colors = gl.getUniformLocation(p, 'colors');
gl.uniform3fv(colors, [1, 2, 3, 4, 5, 6]);
log('colors ' + Array.from(gl.getUniform(p, gl.getUniformLocation(p, 'colors[1]'))).join(','));
gl.uniformMatrix2fv(gl.getUniformLocation(p, 'turn'), false, [1, 2, 3, 4]);
log('turn ' + Array.from(gl.getUniform(p, gl.getUniformLocation(p, 'turn'))).join(','));
gl.uniform1i(gl.getUniformLocation(p, 'pick'), 1);
gl.uniform1i(gl.getUniformLocation(p, 'on'), 1);
log('scalars ' + gl.getUniform(p, gl.getUniformLocation(p, 'pick')) + ' ' + gl.getUniform(p, gl.getUniformLocation(p, 'on')));
gl.uniform1f(gl.getUniformLocation(p, 'pick'), 1);
log('wrong type ' + gl.getError());
gl.uniformMatrix2fv(gl.getUniformLocation(p, 'turn'), true, [1, 2, 3, 4]);
log('transposed ' + gl.getError());
gl.uniform3fv(colors, [1, 2]);
log('short ' + gl.getError());
const bad = gl.createShader(gl.FRAGMENT_SHADER);
gl.shaderSource(bad, 'void main() { gl_FragColor = nothing; }');
gl.compileShader(bad);
log('bad ' + gl.getShaderParameter(bad, gl.COMPILE_STATUS) + ' ' + (gl.getShaderInfoLog(bad).indexOf('ERROR') >= 0));
const lone = gl.createProgram();
gl.linkProgram(lone);
log('lone ' + gl.getProgramParameter(lone, gl.LINK_STATUS) + ' ' + (gl.getProgramInfoLog(lone).length > 0));
gl.deleteProgram(p);
log('deleted ' + gl.isProgram(p) + ' ' + gl.getParameter(gl.CURRENT_PROGRAM) + ' ' + gl.getProgramParameter(p, gl.DELETE_STATUS));
gl.useProgram(null);
log('gone ' + gl.isProgram(p) + ' ' + gl.getParameter(gl.CURRENT_PROGRAM));
const sh = gl.createShader(gl.VERTEX_SHADER);
const q = gl.createProgram();
gl.attachShader(q, sh);
gl.deleteShader(sh);
log('attached ' + gl.isShader(sh) + ' ' + gl.getShaderParameter(sh, gl.DELETE_STATUS));
gl.detachShader(q, sh);
log('detached ' + gl.isShader(sh));
"#)
    else {
        return;
    };
    assert_eq!(
        lines,
        [
            "linked true 2 4 2",
            "uniforms colors[0]:2:35665 on:1:35670 pick:1:5124 turn:1:35674",
            "attribute WebGLActiveInfo true",
            "missing null -1",
            "colors 4,5,6",
            "turn 1,2,3,4",
            "scalars 1 true",
            "wrong type 1282",
            "transposed 1281",
            "short 1281",
            "bad false true",
            "lone false true",
            "deleted true [object WebGLProgram] true",
            "gone false null",
            "attached true true",
            "detached false",
        ]
    );
}

/// A program drawing a texture over the whole canvas, its quad bound.
const TEXTURED: &str = r#"
const p = program(
  'attribute vec2 position; varying vec2 uv; void main() { uv = position * 0.5 + 0.5; gl_Position = vec4(position, 0.0, 1.0); }',
  'precision mediump float; uniform sampler2D t; varying vec2 uv; void main() { gl_FragColor = texture2D(t, uv); }');
gl.useProgram(p);
gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
gl.enableVertexAttribArray(0);
gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
const draw = () => gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
const read = (x, y, w, h) => {
  const out = new Uint8Array(w * h * 4);
  gl.readPixels(x, y, w, h, gl.RGBA, gl.UNSIGNED_BYTE, out);
  const px = [];
  for (let i = 0; i < out.length; i += 4) px.push(Array.from(out.slice(i, i + 4)).join(','));
  return px.join(' ');
};
const nearest = () => {
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
};
"#;

#[test]
fn textures_are_uploaded_sampled_the_right_way_up_and_read_back() {
    let Some(lines) = run(&format!(
        "{TEXTURED}{}",
        r#"
const t = gl.createTexture();
log('unbound ' + gl.getParameter(gl.TEXTURE_BINDING_2D));
gl.bindTexture(gl.TEXTURE_2D, t);
log('bound ' + (gl.getParameter(gl.TEXTURE_BINDING_2D) === t) + ' ' + gl.isTexture(t) + ' ' + gl.getTexParameter(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER));
// Its first row is GL's bottom.
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 2, 2, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255]));
// Mipmapped by default: incomplete, black.
draw();
log('incomplete ' + read(0, 0, 1, 1));
nearest();
draw();
log('rgba ' + read(0, 0, 4, 2));
// LUMINANCE rows at alignment 4, turned over.
gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
gl.texImage2D(gl.TEXTURE_2D, 0, gl.LUMINANCE, 2, 2, 0, gl.LUMINANCE, gl.UNSIGNED_BYTE, new Uint8Array([10, 20, 0, 0, 30, 40]));
gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
draw();
log('luminance ' + read(0, 0, 4, 1));
gl.texSubImage2D(gl.TEXTURE_2D, 0, 1, 0, 1, 1, gl.LUMINANCE, gl.UNSIGNED_BYTE, new Uint8Array([99]));
draw();
log('sub ' + read(2, 0, 1, 1));
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGB, 1, 1, 0, gl.RGB, gl.UNSIGNED_SHORT_5_6_5, new Uint16Array([0xF800]));
draw();
log('565 ' + read(0, 0, 1, 1));
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGB, 1, 1, 0, gl.RGB, gl.UNSIGNED_BYTE, null);
draw();
log('rgb empty ' + read(0, 0, 1, 1));
// Errors.
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 2, 2, 0, gl.RGB, gl.UNSIGNED_BYTE, null);
log('mismatch ' + gl.getError());
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 2, 2, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(3));
log('short ' + gl.getError());
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Float32Array(4));
log('wrong array ' + gl.getError());
gl.texImage2D(gl.TEXTURE_2D, 1, gl.RGBA, 3, 3, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
log('npot level ' + gl.getError());
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.FLOAT, null);
log('no float ' + gl.getError());
gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.REPEAT);
log('bad filter ' + gl.getError());
// readPixels: outside pixels kept; its array's type and size checked.
const kept = new Uint8Array(8).fill(7);
gl.readPixels(-1, 0, 2, 1, gl.RGBA, gl.UNSIGNED_BYTE, kept);
log('outside ' + Array.from(kept).join(','));
gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(3));
log('small ' + gl.getError());
gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.FLOAT, new Float32Array(4));
log('float read ' + gl.getError());
gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, new Uint16Array(4));
log('wrong read array ' + gl.getError());
gl.deleteTexture(t);
log('deleted ' + gl.isTexture(t) + ' ' + gl.getParameter(gl.TEXTURE_BINDING_2D));
"#
    )) else {
        return;
    };
    assert_eq!(
        lines,
        [
            "unbound null",
            "bound true true 9986",
            "incomplete 0,0,0,255",
            "rgba 255,0,0,255 255,0,0,255 0,255,0,255 0,255,0,255 0,0,255,255 0,0,255,255 255,255,0,255 255,255,0,255",
            "luminance 30,30,30,255 30,30,30,255 40,40,40,255 40,40,40,255",
            "sub 99,99,99,255",
            "565 255,0,0,255",
            "rgb empty 0,0,0,255",
            "mismatch 1282",
            "short 1282",
            "wrong array 1282",
            "npot level 1281",
            "no float 1280",
            "bad filter 1280",
            "outside 7,7,7,7,0,0,0,255",
            "small 1282",
            "float read 1282",
            "wrong read array 1282",
            "deleted false null",
        ]
    );
}

#[test]
fn framebuffers_draw_into_textures_and_renderbuffers() {
    let Some(lines) = run(&format!(
        "{TEXTURED}{}",
        r#"
const target = gl.createTexture();
gl.bindTexture(gl.TEXTURE_2D, target);
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 2, 2, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
nearest();
const fb = gl.createFramebuffer();
gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
log('empty ' + gl.checkFramebufferStatus(gl.FRAMEBUFFER).toString(16));
gl.clear(gl.COLOR_BUFFER_BIT);
log('incomplete draw ' + gl.getError());
gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, target, 0);
const depth = gl.createRenderbuffer();
gl.bindRenderbuffer(gl.RENDERBUFFER, depth);
gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT16, 2, 2);
gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth);
log('complete ' + gl.checkFramebufferStatus(gl.FRAMEBUFFER).toString(16) + ' ' + (gl.getFramebufferAttachmentParameter(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.FRAMEBUFFER_ATTACHMENT_OBJECT_NAME) === target) + ' ' + gl.getRenderbufferParameter(gl.RENDERBUFFER, gl.RENDERBUFFER_DEPTH_SIZE));
// Sampling the texture drawn into: a feedback loop.
draw();
log('feedback ' + gl.getError());
gl.bindTexture(gl.TEXTURE_2D, null);
gl.clearColor(0, 1, 0, 1);
gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
gl.clearColor(1, 0, 1, 1);
gl.enable(gl.SCISSOR_TEST);
gl.scissor(1, 1, 1, 1);
gl.clear(gl.COLOR_BUFFER_BIT);
gl.disable(gl.SCISSOR_TEST);
log('framebuffer ' + read(0, 0, 2, 2));
// The texture drawn into, sampled onto the canvas.
gl.bindFramebuffer(gl.FRAMEBUFFER, null);
gl.bindTexture(gl.TEXTURE_2D, target);
draw();
log('canvas ' + read(0, 0, 4, 2));
// A copy of the canvas's right half, as LUMINANCE.
const copy = gl.createTexture();
gl.bindTexture(gl.TEXTURE_2D, copy);
gl.copyTexImage2D(gl.TEXTURE_2D, 0, gl.LUMINANCE, 2, 0, 2, 2, 0);
nearest();
draw();
log('copied ' + read(0, 0, 4, 2) + ' ' + gl.getError());
gl.copyTexSubImage2D(gl.TEXTURE_2D, 0, 1, 1, 0, 0, 2, 2);
log('outside the image ' + gl.getError());
// Depth and stencil apart: unsupported.
gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
const stencil = gl.createRenderbuffer();
gl.bindRenderbuffer(gl.RENDERBUFFER, stencil);
gl.renderbufferStorage(gl.RENDERBUFFER, gl.STENCIL_INDEX8, 2, 2);
gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.STENCIL_ATTACHMENT, gl.RENDERBUFFER, stencil);
log('unsupported ' + gl.checkFramebufferStatus(gl.FRAMEBUFFER).toString(16));
gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.STENCIL_ATTACHMENT, gl.RENDERBUFFER, null);
gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, stencil);
log('wrong attachment ' + gl.checkFramebufferStatus(gl.FRAMEBUFFER).toString(16));
gl.bindRenderbuffer(gl.RENDERBUFFER, depth);
gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT16, 3, 3);
gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth);
log('dimensions ' + gl.checkFramebufferStatus(gl.FRAMEBUFFER).toString(16));
gl.deleteFramebuffer(fb);
log('deleted ' + gl.isFramebuffer(fb) + ' ' + gl.getParameter(gl.FRAMEBUFFER_BINDING));
"#
    )) else {
        return;
    };
    assert_eq!(
        lines,
        [
            "empty 8cd7",
            "incomplete draw 1286",
            "complete 8cd5 true 16",
            "feedback 1282",
            "framebuffer 0,255,0,255 0,255,0,255 0,255,0,255 255,0,255,255",
            "canvas 0,255,0,255 0,255,0,255 0,255,0,255 0,255,0,255 0,255,0,255 0,255,0,255 255,0,255,255 255,0,255,255",
            "copied 0,0,0,255 0,0,0,255 0,0,0,255 0,0,0,255 255,255,255,255 255,255,255,255 255,255,255,255 255,255,255,255 0",
            "outside the image 1281",
            "unsupported 8cdd",
            "wrong attachment 8cd6",
            "dimensions 8cd9",
            "deleted false null",
        ]
    );
}

#[test]
fn mipmaps_cubes_image_sources_and_extensions() {
    let Some(lines) = run(&format!(
        "{TEXTURED}{}",
        r#"
// A 4 by 4 half red, half blue: its last level their mean.
const t = gl.createTexture();
gl.bindTexture(gl.TEXTURE_2D, t);
const texels = new Uint8Array(64);
for (let i = 0; i < 16; i++) texels.set(i % 4 < 2 ? [255, 0, 0, 255] : [0, 0, 255, 255], i * 4);
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 4, 4, 0, gl.RGBA, gl.UNSIGNED_BYTE, texels);
gl.generateMipmap(gl.TEXTURE_2D);
gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST_MIPMAP_NEAREST);
gl.viewport(0, 0, 1, 1);
draw();
log('smallest ' + read(0, 0, 1, 1).split(',').map(v => Math.round(v / 8) * 8).join(','));
gl.viewport(0, 0, 4, 2);
// A cube's faces.
const cube = program(
  'attribute vec2 position; void main() { gl_Position = vec4(position, 0.0, 1.0); }',
  'precision mediump float; uniform samplerCube c; uniform vec3 d; void main() { gl_FragColor = textureCube(c, d); }');
gl.useProgram(cube);
const c = gl.createTexture();
gl.bindTexture(gl.TEXTURE_CUBE_MAP, c);
for (let f = 0; f < 6; f++) {
  gl.texImage2D(gl.TEXTURE_CUBE_MAP_POSITIVE_X + f, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([f * 40, 0, 0, 255]));
}
gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
const faces = [];
for (const d of [[1, 0, 0], [0, -1, 0], [0, 0, -1]]) {
  gl.uniform3fv(gl.getUniformLocation(cube, 'd'), d);
  draw();
  faces.push(read(0, 0, 1, 1).split(',')[0]);
}
log('cube ' + faces.join(' '));
gl.useProgram(p);
// An ImageData, its top row red: GL's bottom unless turned over.
gl.bindTexture(gl.TEXTURE_2D, t);
const image = new ImageData(new Uint8ClampedArray([255, 0, 0, 255, 0, 0, 255, 255]), 1, 2);
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, image);
nearest();
draw();
log('image ' + read(0, 0, 1, 1) + ' / ' + read(0, 1, 1, 1));
gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, image);
gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
draw();
log('turned over ' + read(0, 0, 1, 1));
try { gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, {}); } catch (e) { log('not a source ' + e.name); }
// Extensions.
const names = gl.getSupportedExtensions();
log('offered ' + names.includes('OES_texture_float') + ' ' + names.includes('WEBGL_depth_texture') + ' ' + gl.getExtension('nope'));
const half = gl.getExtension('oes_texture_half_float');
log('half ' + String(half) + ' ' + half.HALF_FLOAT_OES + ' ' + (gl.getExtension('OES_texture_half_float') === half));
gl.getExtension('OES_texture_float');
const f = gl.createTexture();
gl.bindTexture(gl.TEXTURE_2D, f);
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.FLOAT, new Float32Array([0.25, 2.5, -1, 1]));
nearest();
const ffb = gl.createFramebuffer();
gl.bindFramebuffer(gl.FRAMEBUFFER, ffb);
gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, f, 0);
const floats = new Float32Array(4);
gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.FLOAT, floats);
log('float ' + gl.checkFramebufferStatus(gl.FRAMEBUFFER).toString(16) + ' ' + Array.from(floats).join(',') + ' ' + gl.getParameter(gl.IMPLEMENTATION_COLOR_READ_TYPE));
gl.bindFramebuffer(gl.FRAMEBUFFER, null);
const instanced = gl.getExtension('ANGLE_instanced_arrays');
instanced.vertexAttribDivisorANGLE(1, 3);
log('divisor ' + gl.getVertexAttrib(1, instanced.VERTEX_ATTRIB_ARRAY_DIVISOR_ANGLE) + ' ' + gl.getError());
"#
    )) else {
        return;
    };
    assert_eq!(
        lines,
        [
            "smallest 128,0,128,256",
            "cube 0 120 200",
            "image 255,0,0,255 / 0,0,255,255",
            "turned over 0,0,255,255",
            "not a source TypeError",
            "offered true true null",
            "half [object OES_texture_half_float] 36193 true",
            "float 8cd5 0.25,2.5,-1,1 5126",
            "divisor 3 0",
        ]
    );
}

/// As [`run`], `gl` a context of `kind`.
fn run_with(kind: &str, script: &str) -> Option<Vec<String>> {
    let context = CONTEXT.replace("getContext('webgl', ", &format!("getContext('{kind}', "));
    run_page(&context, script)
}

#[test]
fn webgl2_buffers_vertex_arrays_and_uniform_blocks() {
    let Some(lines) = run_with(
        "webgl2",
        r#"
const p = program(
  '#version 300 es\nin vec2 position; in uint index; flat out uint v; uniform Colors { vec4 c[2]; float scale; }; void main() { v = index; gl_Position = vec4(position * scale, 0.0, 1.0); }',
  '#version 300 es\nprecision mediump float; flat in uint v; uniform Colors { vec4 c[2]; float scale; }; uniform uint pick; out vec4 o; void main() { o = c[(v + pick) % 2u]; }');
gl.useProgram(p);
const block = gl.getUniformBlockIndex(p, 'Colors');
log('block ' + block + ' ' + gl.getActiveUniformBlockName(p, block) + ' ' + gl.getActiveUniformBlockParameter(p, block, gl.UNIFORM_BLOCK_DATA_SIZE) + ' ' + gl.getProgramParameter(p, gl.ACTIVE_UNIFORM_BLOCKS) + ' ' + gl.getUniformBlockIndex(p, 'Nothing'));
const indices = gl.getUniformIndices(p, ['c', 'scale']);
log('members ' + gl.getActiveUniforms(p, indices, gl.UNIFORM_OFFSET).join(',') + ' ' + gl.getActiveUniforms(p, indices, gl.UNIFORM_ARRAY_STRIDE).join(',') + ' ' + gl.getActiveUniformBlockParameter(p, block, gl.UNIFORM_BLOCK_ACTIVE_UNIFORM_INDICES).constructor.name);
const ubo = gl.createBuffer();
gl.bindBuffer(gl.UNIFORM_BUFFER, ubo);
gl.bufferData(gl.UNIFORM_BUFFER, new Float32Array([1, 0, 0, 1, 0, 1, 0, 1, 1, 0, 0, 0]), gl.DYNAMIC_DRAW);
gl.bindBufferBase(gl.UNIFORM_BUFFER, 2, ubo);
gl.uniformBlockBinding(p, block, 2);
log('bound ' + (gl.getIndexedParameter(gl.UNIFORM_BUFFER_BINDING, 2) === ubo) + ' ' + gl.getActiveUniformBlockParameter(p, block, gl.UNIFORM_BLOCK_BINDING));
const vao = gl.createVertexArray();
gl.bindVertexArray(vao);
gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 0, -1, -1, 1, 0, 1, 0, -1, 1, -1, 0, 1, 1, 1]), gl.STATIC_DRAW);
const position = gl.getAttribLocation(p, 'position');
gl.enableVertexAttribArray(position);
gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
gl.bufferData(gl.ARRAY_BUFFER, new Uint32Array([0, 0, 0, 0, 1, 1, 1, 1]), gl.STATIC_DRAW);
const index = gl.getAttribLocation(p, 'index');
gl.enableVertexAttribArray(index);
gl.vertexAttribIPointer(index, 1, gl.UNSIGNED_INT, 0, 0);
gl.bindVertexArray(null);
log('default ' + gl.getVertexAttrib(position, gl.VERTEX_ATTRIB_ARRAY_ENABLED) + ' ' + gl.getParameter(gl.VERTEX_ARRAY_BINDING));
gl.bindVertexArray(vao);
log('vao ' + gl.getVertexAttrib(position, gl.VERTEX_ATTRIB_ARRAY_ENABLED) + ' ' + (gl.getParameter(gl.VERTEX_ARRAY_BINDING) === vao) + ' ' + gl.isVertexArray(vao));
gl.vertexAttribI4i(7, -2147483648, 2147483647, -1, 16777217);
const ints = gl.getVertexAttrib(7, gl.CURRENT_VERTEX_ATTRIB);
gl.vertexAttribI4ui(7, 4294967295, 0, 1, 16777217);
const uints = gl.getVertexAttrib(7, gl.CURRENT_VERTEX_ATTRIB);
gl.vertexAttrib4f(7, 0.5, 0, 0, 1);
const floats = gl.getVertexAttrib(7, gl.CURRENT_VERTEX_ATTRIB);
log('current ' + [ints, uints, floats].map(a => a.constructor.name + ' ' + Array.from(a).join(',')).join(' '));
gl.uniform1ui(gl.getUniformLocation(p, 'pick'), 1);
gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
gl.drawArrays(gl.TRIANGLE_STRIP, 4, 4);
const px = new Uint8Array(4 * 4);
gl.readPixels(0, 0, 4, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
log('drawn ' + gl.getError() + ' ' + Array.from(px).join(','));
// The block's buffer read back and copied; too small, and unbound.
const back = new Float32Array(3);
gl.getBufferSubData(gl.UNIFORM_BUFFER, 32, back);
log('read back ' + Array.from(back).join(','));
gl.bindBuffer(gl.COPY_WRITE_BUFFER, gl.createBuffer());
gl.bufferData(gl.COPY_WRITE_BUFFER, 8, gl.STATIC_COPY);
gl.copyBufferSubData(gl.UNIFORM_BUFFER, gl.COPY_WRITE_BUFFER, 16, 0, 8);
const copied = new Float32Array(2);
gl.getBufferSubData(gl.COPY_WRITE_BUFFER, 0, copied);
log('copied ' + Array.from(copied).join(',') + ' ' + gl.getError());
gl.bufferData(gl.UNIFORM_BUFFER, 16, gl.DYNAMIC_DRAW);
gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
log('too small ' + gl.getError());
gl.bindBufferBase(gl.UNIFORM_BUFFER, 2, null);
gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
log('unbound ' + gl.getError());
gl.bindBufferRange(gl.UNIFORM_BUFFER, 2, ubo, 100, 16);
log('unaligned ' + gl.getError());
gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, ubo);
log('element ' + gl.getError());
// An element array buffer may be bound to a copy target; one first bound
// there is still either.
const elementBuffer = gl.createBuffer();
gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, elementBuffer);
gl.bindBuffer(gl.COPY_READ_BUFFER, elementBuffer);
const copyFirst = gl.createBuffer();
gl.bindBuffer(gl.COPY_WRITE_BUFFER, copyFirst);
gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, copyFirst);
const copies = gl.getError();
gl.bindBuffer(gl.ARRAY_BUFFER, copyFirst);
log('copy targets ' + copies + ' ' + gl.isBuffer(copyFirst) + ' then an element array ' + gl.getError());
gl.deleteVertexArray(vao);
log('deleted ' + gl.isVertexArray(vao) + ' ' + gl.getParameter(gl.VERTEX_ARRAY_BINDING));
// WebGL 1's OES_vertex_array_object.
const g1 = document.createElement('canvas').getContext('webgl');
const oes = g1.getExtension('OES_vertex_array_object');
const v1 = oes.createVertexArrayOES();
oes.bindVertexArrayOES(v1);
log('oes ' + String(v1) + ' ' + (g1.getParameter(oes.VERTEX_ARRAY_BINDING_OES) === v1) + ' ' + oes.isVertexArrayOES(v1));
"#,
    ) else {
        return;
    };
    assert_eq!(
        lines,
        [
            "block 0 Colors 48 1 4294967295",
            "members 0,32 16,0 Uint32Array",
            "bound true 2",
            "default false null",
            "vao true true true",
            "current Int32Array -2147483648,2147483647,-1,16777217 Uint32Array 4294967295,0,1,16777217 Float32Array 0.5,0,0,1",
            "drawn 0 0,255,0,255,0,255,0,255,255,0,0,255,255,0,0,255",
            "read back 1,0,0",
            "copied 0,1 0",
            "too small 1282",
            "unbound 1282",
            "unaligned 1281",
            "element 1282",
            "copy targets 0 true then an element array 1282",
            "deleted false null",
            "oes [object WebGLVertexArrayObjectOES] true true",
        ]
    );
}

#[test]
fn webgl2_queries_fences_feedback_and_a_lost_context() {
    let Some(lines) = run_with(
        "webgl2",
        r#"
const tick = () => new Promise(r => setTimeout(r, 10));
const p = program(
  '#version 300 es\nin vec2 position; void main() { gl_Position = vec4(position, 0.0, 1.0); }',
  '#version 300 es\nprecision mediump float; out vec4 o; void main() { o = vec4(1.0); }');
gl.useProgram(p);
gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1]), gl.STATIC_DRAW);
gl.enableVertexAttribArray(0);
gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
const drawn = gl.createQuery();
gl.beginQuery(gl.ANY_SAMPLES_PASSED, drawn);
log('current ' + (gl.getQuery(gl.ANY_SAMPLES_PASSED, gl.CURRENT_QUERY) === drawn) + ' ' + gl.isQuery(drawn));
gl.beginQuery(gl.ANY_SAMPLES_PASSED_CONSERVATIVE, gl.createQuery());
log('two at once ' + gl.getError());
gl.drawArrays(gl.TRIANGLES, 0, 3);
gl.endQuery(gl.ANY_SAMPLES_PASSED);
log('at once ' + gl.getQueryParameter(drawn, gl.QUERY_RESULT_AVAILABLE));
const none = gl.createQuery();
gl.beginQuery(gl.ANY_SAMPLES_PASSED, none);
gl.endQuery(gl.ANY_SAMPLES_PASSED);
const sync = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0);
log('fence ' + String(sync) + ' ' + gl.getSyncParameter(sync, gl.SYNC_STATUS).toString(16) + ' ' + gl.clientWaitSync(sync, 0, 0).toString(16));
gl.clientWaitSync(sync, 0, 1);
log('waiting ' + gl.getError());
for (let i = 0; i < 300 && !(gl.getQueryParameter(drawn, gl.QUERY_RESULT_AVAILABLE) && gl.getSyncParameter(sync, gl.SYNC_STATUS) === gl.SIGNALED); i++) await tick();
log('results ' + gl.getQueryParameter(drawn, gl.QUERY_RESULT) + ' ' + gl.getQueryParameter(none, gl.QUERY_RESULT) + ' ' + gl.clientWaitSync(sync, 0, 0).toString(16));
const feedback = gl.createTransformFeedback();
gl.bindTransformFeedback(gl.TRANSFORM_FEEDBACK, feedback);
gl.beginTransformFeedback(gl.TRIANGLES);
log('feedback ' + gl.isTransformFeedback(feedback) + ' ' + gl.getError() + ' ' + (gl.getParameter(gl.TRANSFORM_FEEDBACK_BINDING) === feedback));
// Lost, its event prevented, restored.
const old = gl.createBuffer();
const lose = gl.getExtension('WEBGL_lose_context');
canvas.addEventListener('webglcontextlost', (e) => { log('lost event ' + e.constructor.name + ' ' + e.cancelable); e.preventDefault(); });
let restored = false;
canvas.addEventListener('webglcontextrestored', () => { restored = true; });
lose.loseContext();
log('lost ' + gl.isContextLost() + ' ' + gl.getError() + ' ' + gl.getError() + ' ' + gl.getContextAttributes() + ' ' + gl.getSupportedExtensions());
await tick();
lose.restoreContext();
for (let i = 0; i < 300 && !restored; i++) await tick();
gl.bindBuffer(gl.ARRAY_BUFFER, old);
log('restored ' + gl.isContextLost() + ' ' + gl.getError() + ' ' + (gl.getParameter(gl.ARRAY_BUFFER_BINDING) === null));
"#,
    ) else {
        return;
    };
    assert_eq!(
        lines,
        [
            "current true true",
            "two at once 1282",
            "at once false",
            "fence [object WebGLSync] 9118 911b",
            "waiting 1282",
            "results 1 0 911a",
            "feedback true 1282 true",
            "lost true 37442 0 null null",
            "lost event WebGLContextEvent true",
            "restored false 1282 true",
        ]
    );
}

/// WebGL 2's helpers: a full-canvas quad at attribute 0, a program of
/// GLSL ES 3.00 sources, and a read of RGBA bytes.
const GL2: &str = r#"
gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
gl.enableVertexAttribArray(0);
gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
const VS = '#version 300 es\nlayout(location = 0) in vec2 position; out vec2 uv; void main() { uv = position * 0.5 + 0.5; gl_Position = vec4(position, 0.0, 1.0); }';
const fs = (body) => program(VS, '#version 300 es\nprecision highp float; precision highp int; precision highp sampler3D; precision highp sampler2DArray; precision highp usampler2D; precision highp sampler2DShadow; in vec2 uv; ' + body);
const draw = () => gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
const read = (x, y, w, h) => {
  const out = new Uint8Array(w * h * 4);
  gl.readPixels(x, y, w, h, gl.RGBA, gl.UNSIGNED_BYTE, out);
  const px = [];
  for (let i = 0; i < out.length; i += 4) px.push(Array.from(out.slice(i, i + 4)).join(','));
  return px.join(' ');
};
const nearest = (target) => {
  gl.texParameteri(target, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(target, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
};
"#;

#[test]
fn webgl2_sized_integer_3d_and_array_textures_samplers_and_pixel_buffers() {
    let Some(lines) = run_with(
        "webgl2",
        &format!(
            "{GL2}{}",
            r#"
// An R8 of fixed storage, its rows given in part.
const plain = fs('uniform sampler2D t; out vec4 o; void main() { o = texture(t, uv); }');
gl.useProgram(plain);
const r8 = gl.createTexture();
gl.bindTexture(gl.TEXTURE_2D, r8);
gl.texStorage2D(gl.TEXTURE_2D, 1, gl.R8, 2, 1);
nearest(gl.TEXTURE_2D);
gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, 2, 1, gl.RED, gl.UNSIGNED_BYTE, new Uint8Array([255, 128]));
draw();
log('r8 ' + read(0, 0, 1, 1) + ' / ' + read(3, 0, 1, 1) + ' ' + gl.getTexParameter(gl.TEXTURE_2D, gl.TEXTURE_IMMUTABLE_FORMAT));
gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, 1, 1, 0, gl.RED, gl.UNSIGNED_BYTE, null);
log('immutable ' + gl.getError());
gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(4));
log('wrong format ' + gl.getError());
// An unsigned integer texture through a usampler2D.
const ints = fs('uniform usampler2D t; out vec4 o; void main() { o = vec4(texelFetch(t, ivec2(0, 0), 0)) / 255.0; }');
gl.useProgram(ints);
const ui = gl.createTexture();
gl.bindTexture(gl.TEXTURE_2D, ui);
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8UI, 1, 1, 0, gl.RGBA_INTEGER, gl.UNSIGNED_BYTE, new Uint8Array([10, 20, 30, 255]));
nearest(gl.TEXTURE_2D);
draw();
log('uint ' + read(0, 0, 1, 1) + ' ' + gl.getError());
// Linear filtering makes it incomplete: (0, 0, 0, 1).
gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
draw();
log('linear ints ' + read(0, 0, 1, 1));
// A float sampler of an integer texture: INVALID_OPERATION.
gl.useProgram(plain);
gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
draw();
log('wrong sampler ' + gl.getError());
// A 3D texture's second slice, and a 2D array's third layer.
const d3 = fs('uniform sampler3D t; out vec4 o; void main() { o = texture(t, vec3(uv, 0.75)); }');
gl.useProgram(d3);
const t3 = gl.createTexture();
gl.bindTexture(gl.TEXTURE_3D, t3);
gl.texImage3D(gl.TEXTURE_3D, 0, gl.RGBA8, 1, 1, 2, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([255, 0, 0, 255, 0, 0, 255, 255]));
nearest(gl.TEXTURE_3D);
draw();
log('3d ' + read(0, 0, 1, 1));
// A sampler object overrides the texture's own: mipmapped filtering of
// one level, incomplete.
const sampler = gl.createSampler();
gl.samplerParameteri(sampler, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
gl.bindSampler(0, sampler);
draw();
log('sampler ' + read(0, 0, 1, 1) + ' ' + gl.getSamplerParameter(sampler, gl.TEXTURE_MIN_FILTER) + ' ' + (gl.getParameter(gl.SAMPLER_BINDING) === sampler) + ' ' + gl.isSampler(sampler));
gl.bindSampler(0, null);
const arrays = fs('uniform sampler2DArray t; out vec4 o; void main() { o = texture(t, vec3(uv, 2.0)); }');
gl.useProgram(arrays);
const ta = gl.createTexture();
gl.bindTexture(gl.TEXTURE_2D_ARRAY, ta);
gl.texStorage3D(gl.TEXTURE_2D_ARRAY, 1, gl.RGBA8, 1, 1, 3);
gl.texSubImage3D(gl.TEXTURE_2D_ARRAY, 0, 0, 0, 2, 1, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([0, 255, 0, 255]));
nearest(gl.TEXTURE_2D_ARRAY);
draw();
log('array ' + read(0, 0, 1, 1));
// A depth texture compared: 0.4 <= 0.5 passes, 0.6 does not.
const shadow = fs('uniform sampler2DShadow t; uniform float ref; out vec4 o; void main() { o = vec4(texture(t, vec3(uv, ref))); }');
gl.useProgram(shadow);
const depth = gl.createTexture();
gl.bindTexture(gl.TEXTURE_2D, depth);
gl.texImage2D(gl.TEXTURE_2D, 0, gl.DEPTH_COMPONENT16, 1, 1, 0, gl.DEPTH_COMPONENT, gl.UNSIGNED_SHORT, new Uint16Array([32768]));
nearest(gl.TEXTURE_2D);
gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_COMPARE_MODE, gl.COMPARE_REF_TO_TEXTURE);
gl.uniform1f(gl.getUniformLocation(shadow, 'ref'), 0.4);
draw();
const pass = read(0, 0, 1, 1);
gl.uniform1f(gl.getUniformLocation(shadow, 'ref'), 0.6);
draw();
log('shadow ' + pass + ' / ' + read(0, 0, 1, 1) + ' ' + gl.getError());
// Uploads from a pixel unpack buffer; reads into a pixel pack buffer.
gl.useProgram(plain);
const unpack = gl.createBuffer();
gl.bindBuffer(gl.PIXEL_UNPACK_BUFFER, unpack);
gl.bufferData(gl.PIXEL_UNPACK_BUFFER, new Uint8Array([9, 9, 9, 9, 0, 0, 255, 255]), gl.STATIC_DRAW);
const fromBuffer = gl.createTexture();
gl.bindTexture(gl.TEXTURE_2D, fromBuffer);
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, 4);
nearest(gl.TEXTURE_2D);
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(4));
log('array with a buffer bound ' + gl.getError());
gl.bindBuffer(gl.PIXEL_UNPACK_BUFFER, null);
draw();
const packed = gl.createBuffer();
gl.bindBuffer(gl.PIXEL_PACK_BUFFER, packed);
gl.bufferData(gl.PIXEL_PACK_BUFFER, 8, gl.STREAM_READ);
gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, 4);
const back = new Uint8Array(8);
gl.getBufferSubData(gl.PIXEL_PACK_BUFFER, 0, back);
log('pixel buffers ' + Array.from(back).join(',') + ' ' + gl.getError());
gl.bindBuffer(gl.PIXEL_PACK_BUFFER, null);
// An integer cube: each face read by its direction (WebGPU samples no
// integer cube; its faces are a 2D array's layers, the face chosen in
// the shader).
const cubeInts = fs('uniform highp isamplerCube t; out vec4 o; void main() { o = vec4(texture(t, vec3(1.0, 0.0, 0.0)).r, texture(t, vec3(0.0, 0.0, -1.0)).r, texture(t, vec3(0.0, 1.0, 0.0)).r, 255) / 255.0; }');
gl.useProgram(cubeInts);
const ic = gl.createTexture();
gl.bindTexture(gl.TEXTURE_CUBE_MAP, ic);
for (let face = 0; face < 6; face++) {
  gl.texImage2D(gl.TEXTURE_CUBE_MAP_POSITIVE_X + face, 0, gl.RGBA8I, 1, 1, 0, gl.RGBA_INTEGER, gl.BYTE, new Int8Array([10 * (face + 1), 0, 0, 1]));
}
nearest(gl.TEXTURE_CUBE_MAP);
draw();
log('int cube ' + read(0, 0, 1, 1) + ' ' + gl.getError());
// An image source's sub-rectangle: UNPACK_SKIP_PIXELS of an ImageData.
gl.useProgram(plain);
const part = gl.createTexture();
gl.bindTexture(gl.TEXTURE_2D, part);
nearest(gl.TEXTURE_2D);
gl.pixelStorei(gl.UNPACK_SKIP_PIXELS, 1);
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new ImageData(new Uint8ClampedArray([255, 0, 0, 255, 0, 255, 0, 255]), 2, 1));
gl.pixelStorei(gl.UNPACK_SKIP_PIXELS, 0);
draw();
log('skipped ' + read(0, 0, 1, 1) + ' ' + gl.getError());
"#
        ),
    ) else {
        return;
    };
    assert_eq!(
        lines,
        [
            "r8 255,0,0,255 / 128,0,0,255 true",
            "immutable 1282",
            "wrong format 1282",
            "uint 10,20,30,255 0",
            "linear ints 0,0,0,1",
            "wrong sampler 1282",
            "3d 0,0,255,255",
            "sampler 0,0,0,255 9987 true true",
            "array 0,255,0,255",
            "shadow 255,255,255,255 / 0,0,0,0 0",
            "array with a buffer bound 1282",
            "pixel buffers 0,0,0,0,0,0,255,255 0",
            "int cube 10,60,30,255 0",
            "skipped 0,255,0,255 0",
        ]
    );
}

#[test]
fn webgl2_draw_buffers_clears_blits_and_multisampled_renderbuffers() {
    let Some(lines) = run_with(
        "webgl2",
        &format!(
            "{GL2}{}",
            r#"
// Two colour attachments, a float and an unsigned integer one.
const color = gl.createTexture();
gl.bindTexture(gl.TEXTURE_2D, color);
gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGBA8, 4, 2);
const counts = gl.createTexture();
gl.bindTexture(gl.TEXTURE_2D, counts);
gl.texStorage2D(gl.TEXTURE_2D, 1, gl.R32UI, 4, 2);
const fb = gl.createFramebuffer();
gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, color, 0);
gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT1, gl.TEXTURE_2D, counts, 0);
gl.drawBuffers([gl.COLOR_ATTACHMENT0, gl.COLOR_ATTACHMENT1]);
log('buffers ' + gl.checkFramebufferStatus(gl.FRAMEBUFFER).toString(16) + ' ' + (gl.getParameter(gl.DRAW_BUFFER1) === gl.COLOR_ATTACHMENT1));
gl.clear(gl.COLOR_BUFFER_BIT);
log('clear of ints ' + gl.getError());
const both = fs('layout(location = 0) out vec4 o; layout(location = 1) out uint n; void main() { o = vec4(1.0, 0.5, 0.0, 1.0); n = 7u; }');
gl.useProgram(both);
draw();
gl.clearBufferuiv(gl.COLOR, 1, new Uint32Array([3, 0, 0, 0]));
gl.clearBufferfv(gl.COLOR, 0, [0, 0, 1, 1]);
gl.enable(gl.SCISSOR_TEST);
gl.scissor(0, 0, 1, 1);
gl.clearBufferuiv(gl.COLOR, 1, new Uint32Array([9, 0, 0, 0]));
gl.disable(gl.SCISSOR_TEST);
gl.clearBufferiv(gl.COLOR, 1, new Int32Array(4));
log('wrong kind ' + gl.getError());
gl.readBuffer(gl.COLOR_ATTACHMENT1);
const n = new Uint32Array(4 * 2);
gl.readPixels(0, 0, 2, 1, gl.RGBA_INTEGER, gl.UNSIGNED_INT, n);
log('ints ' + n[0] + ' ' + n[4] + ' ' + gl.getParameter(gl.IMPLEMENTATION_COLOR_READ_FORMAT).toString(16));
gl.readBuffer(gl.COLOR_ATTACHMENT0);
log('floats ' + read(0, 0, 1, 1));
// An output of the wrong kind: INVALID_OPERATION.
const wrong = fs('layout(location = 0) out vec4 o; layout(location = 1) out vec4 m; void main() { o = vec4(1.0); m = vec4(1.0); }');
gl.useProgram(wrong);
draw();
log('wrong output ' + gl.getError());
// An active draw buffer the program writes nothing to: INVALID_OPERATION,
// unless its writes are masked.
gl.useProgram(fs('layout(location = 0) out vec4 o; void main() { o = vec4(1.0); }'));
gl.drawBuffers([gl.COLOR_ATTACHMENT0, gl.COLOR_ATTACHMENT1]);
draw();
const missing = gl.getError();
gl.colorMask(false, false, false, false);
draw();
log('missing output ' + missing + ' ' + gl.getError());
gl.colorMask(true, true, true, true);
// Blitted onto the canvas, turned over left to right.
gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, null);
gl.blitFramebuffer(0, 0, 4, 2, 4, 0, 0, 2, gl.COLOR_BUFFER_BIT, gl.NEAREST);
// Sides past an int32's range: INVALID_VALUE.
const blitted = gl.getError();
gl.blitFramebuffer(0, 0, 4, 2, -2147483648, 0, 2147483647, 2, gl.COLOR_BUFFER_BIT, gl.NEAREST);
log('too wide ' + gl.getError());
gl.bindFramebuffer(gl.READ_FRAMEBUFFER, null);
log('blit ' + blitted + ' ' + read(0, 0, 1, 1));
// A multisampled renderbuffer drawn into, resolved by a blit.
const ms = gl.createRenderbuffer();
gl.bindRenderbuffer(gl.RENDERBUFFER, ms);
gl.renderbufferStorageMultisample(gl.RENDERBUFFER, 4, gl.RGBA8, 4, 2);
log('samples ' + gl.getRenderbufferParameter(gl.RENDERBUFFER, gl.RENDERBUFFER_SAMPLES) + ' ' + Array.from(gl.getInternalformatParameter(gl.RENDERBUFFER, gl.RGBA8, gl.SAMPLES)).join(',') + ' ' + gl.getInternalformatParameter(gl.RENDERBUFFER, gl.R32UI, gl.SAMPLES).length);
const msfb = gl.createFramebuffer();
gl.bindFramebuffer(gl.FRAMEBUFFER, msfb);
gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.RENDERBUFFER, ms);
gl.clearColor(1, 0, 1, 1);
gl.clear(gl.COLOR_BUFFER_BIT);
gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(4));
log('read multisampled ' + gl.getError());
gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, fb);
gl.drawBuffers([gl.COLOR_ATTACHMENT0]);
gl.blitFramebuffer(0, 0, 4, 2, 0, 0, 4, 2, gl.COLOR_BUFFER_BIT, gl.NEAREST);
gl.bindFramebuffer(gl.READ_FRAMEBUFFER, fb);
log('resolved ' + gl.getError() + ' ' + read(3, 1, 1, 1));
gl.blitFramebuffer(0, 0, 4, 2, 0, 0, 2, 1, gl.COLOR_BUFFER_BIT, gl.LINEAR);
log('scaled from itself ' + gl.getError());
gl.invalidateFramebuffer(gl.FRAMEBUFFER, [gl.COLOR_ATTACHMENT0]);
gl.invalidateFramebuffer(gl.FRAMEBUFFER, [gl.BACK]);
log('invalidate ' + gl.getError());
// WebGL 1's WEBGL_draw_buffers.
const g1 = document.createElement('canvas').getContext('webgl');
const ext = g1.getExtension('WEBGL_draw_buffers');
const f1 = g1.createFramebuffer();
g1.bindFramebuffer(g1.FRAMEBUFFER, f1);
ext.drawBuffersWEBGL([ext.COLOR_ATTACHMENT0_WEBGL, ext.COLOR_ATTACHMENT1_WEBGL]);
log('webgl1 ' + String(ext) + ' ' + g1.getParameter(ext.MAX_DRAW_BUFFERS_WEBGL) + ' ' + (g1.getParameter(ext.DRAW_BUFFER1_WEBGL) === ext.COLOR_ATTACHMENT1_WEBGL) + ' ' + g1.getError());
"#
        ),
    ) else {
        return;
    };
    assert_eq!(
        lines,
        [
            "buffers 8cd5 true",
            "clear of ints 1282",
            "wrong kind 1282",
            "ints 9 3 8d94",
            "floats 0,0,255,255",
            "wrong output 1282",
            "missing output 1282 0",
            "too wide 1281",
            "blit 0 0,0,255,255",
            "samples 4 4 0",
            "read multisampled 1282",
            "resolved 0 255,0,255,255",
            "scaled from itself 1282",
            "invalidate 1280",
            "webgl1 [object WEBGL_draw_buffers] 8 true 0",
        ]
    );
}

#[test]
fn the_acceptance_pages_draw_what_the_reference_browser_draws() {
    use crate::webgl::acceptance;
    let script = format!("{}\nawait accept(log);", acceptance::SCRIPT);
    let Some(lines) = run_page("", &script) else {
        return;
    };
    let differences = acceptance::differences(&lines);
    assert!(differences.is_empty(), "{differences:#?}");
}
