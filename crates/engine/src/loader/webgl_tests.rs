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
log('extensions ' + gl.getSupportedExtensions().length + ' ' + gl.getExtension('OES_nothing'));
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
            "extensions 0 null",
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
