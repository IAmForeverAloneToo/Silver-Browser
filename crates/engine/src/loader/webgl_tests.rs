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
    let page = server(&format!("{CONTEXT}\n{script}"));
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
gl.bindBuffer(gl.ARRAY_BUFFER, other.createBuffer());
log('other context ' + gl.getError());
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
            "other context 1282",
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
