//! WebGPU's work (M41 step 3, D39): shaders, pipelines, textures,
//! encoders, passes and bundles, run on this machine's adapter through a
//! GPU service in this process and read back. Where there is no adapter,
//! a test says so.

use std::time::{Duration, Instant};

use super::webgpu_tests::{ADAPTER, lines, server};
use crate::FrameTime;
use crate::testing::Tab;

/// The page's lines, or `None` (said) where there is no adapter.
fn run(script: &str) -> Option<Vec<String>> {
    let lines = lines(&server(&format!("{ADAPTER}\n{script}")));
    if lines == ["no adapter"] {
        eprintln!("no adapter here: the work is not checked");
        return None;
    }
    Some(lines)
}

/// As [`run`], with the engine's frames run as the shell runs them, a
/// frame's time 16 ms after the last's.
fn run_framed(script: &str) -> Option<Vec<String>> {
    let page = server(&format!(
        "{ADAPTER}
{script}"
    ));
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
    if lines == ["no adapter"] {
        eprintln!("no adapter here: the canvas is not checked");
        return None;
    }
    Some(lines)
}

#[test]
fn a_compute_pass_changes_a_storage_buffer_that_is_copied_and_read_back() {
    let Some(lines) = run(r#"
const device = await adapter.requestDevice();
const module = device.createShaderModule({code: `
@group(0) @binding(0) var<storage, read_write> data: array<u32>;
@compute @workgroup_size(4) fn main(@builtin(global_invocation_id) id: vec3u) {
  data[id.x] = data[id.x] * 2u + 1u;
}`});
const pipeline = device.createComputePipeline({layout: 'auto', compute: {module, entryPoint: 'main'}});
const storage = device.createBuffer({size: 16, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST});
device.queue.writeBuffer(storage, 0, new Uint32Array([1, 2, 3, 4]));
const read = device.createBuffer({size: 16, usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST});
const group = device.createBindGroup({layout: pipeline.getBindGroupLayout(0), entries: [{binding: 0, resource: {buffer: storage}}]});
const encoder = device.createCommandEncoder({label: 'mine'});
const pass = encoder.beginComputePass({label: 'pass'});
pass.setPipeline(pipeline);
pass.setBindGroup(0, group);
pass.dispatchWorkgroups(1);
pass.end();
encoder.copyBufferToBuffer(storage, 0, read, 0, 16);
const commands = encoder.finish();
device.queue.submit([commands]);
await read.mapAsync(GPUMapMode.READ);
log('changed ' + Array.from(new Uint32Array(read.getMappedRange())).join(','));
log('kinds ' + [module, pipeline, group, encoder, pass, commands].map(o => o.constructor.name).join(' '));
const labelled = await (await navigator.gpu.requestAdapter()).requestDevice({label: 'named'});
const render = labelled.createCommandEncoder().beginRenderPass({label: 'drawn', colorAttachments: []});
log('label ' + encoder.label + ' ' + pass.label + ' ' + labelled.label + ' ' + render.label);
"#)
    else {
        return;
    };
    assert_eq!(
        lines,
        [
            "changed 3,5,7,9",
            "kinds GPUShaderModule GPUComputePipeline GPUBindGroup GPUCommandEncoder GPUComputePassEncoder GPUCommandBuffer",
            "label mine pass named drawn",
        ]
    );
}

#[test]
fn a_render_pass_clears_and_draws_into_a_texture_directly_and_from_a_bundle() {
    let Some(lines) = run(r#"
const device = await adapter.requestDevice();
const texture = device.createTexture({size: [4, 4], format: 'rgba8unorm', usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC});
log('texture ' + [texture.width, texture.height, texture.depthOrArrayLayers, texture.mipLevelCount, texture.sampleCount, texture.dimension, texture.format, texture.usage].join(' '));
const module = device.createShaderModule({code: `
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
  var p = array(vec2f(-1, -1), vec2f(3, -1), vec2f(-1, 3));
  return vec4f(p[i], 0, 1);
}
@fragment fn fs() -> @location(0) vec4f { return vec4f(1, 0, 0, 1); }`});
const pipeline = await device.createRenderPipelineAsync({layout: 'auto', vertex: {module}, fragment: {module, targets: [{format: 'rgba8unorm'}]}});
log('pipeline ' + pipeline.constructor.name);
// The pixels of row 0 after `draw` records into a pass cleared blue.
async function pixels(draw) {
  const encoder = device.createCommandEncoder();
  const pass = encoder.beginRenderPass({colorAttachments: [{view: texture.createView(), loadOp: 'clear', clearValue: [0, 0, 1, 1], storeOp: 'store'}]});
  draw(pass);
  pass.end();
  const read = device.createBuffer({size: 256 * 4, usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST});
  encoder.copyTextureToBuffer({texture}, {buffer: read, bytesPerRow: 256}, [4, 4]);
  device.queue.submit([encoder.finish()]);
  await read.mapAsync(GPUMapMode.READ);
  const px = new Uint8Array(read.getMappedRange());
  return [0, 1, 2, 3].map(i => px[i * 4] ? 'r' : px[i * 4 + 2] ? 'b' : '?').join('');
}
log('cleared ' + await pixels(() => {}));
log('scissored ' + await pixels((pass) => { pass.setPipeline(pipeline); pass.setScissorRect(0, 0, 2, 4); pass.draw(3); }));
const bundles = device.createRenderBundleEncoder({colorFormats: ['rgba8unorm']});
bundles.setPipeline(pipeline);
bundles.draw(3);
const bundle = bundles.finish();
log('bundled ' + await pixels((pass) => pass.executeBundles([bundle])) + ' ' + bundle.constructor.name);
"#)
    else {
        return;
    };
    assert_eq!(
        lines,
        [
            "texture 4 4 1 1 1 2d rgba8unorm 17",
            "pipeline GPURenderPipeline",
            "cleared bbbb",
            "scissored rrbb",
            "bundled rrrr GPURenderBundle",
        ]
    );
}

#[test]
fn a_shader_modules_messages_say_where_and_a_bad_pipeline_is_refused() {
    let Some(lines) = run(r#"
const device = await adapter.requestDevice();
device.onuncapturederror = (e) => e.preventDefault();
const bad = device.createShaderModule({code: '// a module\n@compute @workgroup_size(1) fn main() {\n  let x: u32 = 1.5;\n}'});
const info = await bad.getCompilationInfo();
const m = info.messages[0];
log('message ' + info.constructor.name + ' ' + m.constructor.name + ' ' + m.type + ' ' + m.lineNum + ' ' + (m.linePos > 0) + ' ' + (m.offset > 0) + ' ' + (m.message.length > 0) + ' ' + Object.isFrozen(info.messages));
const good = device.createShaderModule({code: '@compute @workgroup_size(1) fn main() {}'});
log('good ' + (await good.getCompilationInfo()).messages.length);
log('async ' + await device.createComputePipelineAsync({layout: 'auto', compute: {module: bad}}).then(() => 'made', e => e.constructor.name + ' ' + e.name + ' ' + e.reason + ' ' + (e instanceof DOMException)));
const tried = (f) => { try { f(); return 'no error'; } catch (e) { return e.name + ': ' + e.message; } };
log(tried(() => device.createTexture({format: 'rgba8unorm', usage: 1})));
log(tried(() => device.createTexture({size: [1], format: 'rgba9unorm', usage: 1})));
log(tried(() => device.createTexture({size: [], format: 'rgba8unorm', usage: 1})));
log(tried(() => device.createComputePipeline({layout: 'manual', compute: {module: good}})));
log(tried(() => device.createComputePipeline({layout: 'auto', compute: {}})));
log(tried(() => device.createCommandEncoder().beginRenderPass({colorAttachments: [{view: good, loadOp: 'clear', storeOp: 'store'}]})));
"#)
    else {
        return;
    };
    assert_eq!(
        lines,
        [
            "message GPUCompilationInfo GPUCompilationMessage error 3 true true true true",
            "good 0",
            "async GPUPipelineError GPUPipelineError validation true",
            "TypeError: Failed to execute 'createTexture' on 'GPUDevice': Failed to read the 'size' property from 'GPUTextureDescriptor': Required member is undefined.",
            "TypeError: Failed to execute 'createTexture' on 'GPUDevice': Failed to read the 'format' property from 'GPUTextureDescriptor': The provided value 'rgba9unorm' is not a valid enum value of type GPUTextureFormat.",
            "TypeError: Failed to execute 'createTexture' on 'GPUDevice': Failed to read the 'size' property from 'GPUTextureDescriptor': A sequence of number used as a GPUExtent3D must have between 1 and 3 elements.",
            "TypeError: Failed to execute 'createComputePipeline' on 'GPUDevice': Failed to read the 'layout' property from 'GPUComputePipelineDescriptor': The provided value 'manual' is not a valid enum value of type GPUAutoLayoutMode.",
            "TypeError: Failed to execute 'createComputePipeline' on 'GPUDevice': Failed to read the 'compute' property from 'GPUComputePipelineDescriptor': Failed to read the 'module' property from 'GPUProgrammableStage': Required member is undefined.",
            "TypeError: Failed to execute 'beginRenderPass' on 'GPUCommandEncoder': Failed to read the 'colorAttachments' property from 'GPURenderPassDescriptor': Failed to read the 'view' property from 'GPURenderPassColorAttachment': Failed to convert value to 'GPUTextureView'.",
        ]
    );
}

#[test]
fn an_encoders_misuse_and_an_invalid_bundle_are_validation_errors_and_the_device_goes_on() {
    let Some(lines) = run(r#"
const device = await adapter.requestDevice();
const scoped = async (f) => {
  device.pushErrorScope('validation');
  f();
  const error = await device.popErrorScope();
  return error === null ? 'none' : error.constructor.name + ': ' + error.message.trim();
};
const buffer = device.createBuffer({size: 16, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ});
const encoder = device.createCommandEncoder();
const commands = encoder.finish();
log('finished twice ' + await scoped(() => encoder.finish()));
const locked = device.createCommandEncoder();
const pass = locked.beginComputePass();
log('locked ' + await scoped(() => { locked.clearBuffer(buffer); pass.end(); locked.finish(); }));
log('ended pass ' + await scoped(() => pass.end()));
log('used after its end ' + await scoped(() => pass.dispatchWorkgroups(1)));
log('submitted ' + await scoped(() => device.queue.submit([commands])));
log('submitted twice ' + await scoped(() => device.queue.submit([commands])));
log('popped ' + await scoped(() => { const e = device.createCommandEncoder(); e.popDebugGroup(); e.finish(); }));
log('no binding kind ' + await scoped(() => device.createBindGroupLayout({entries: [{binding: 0, visibility: 1}]})));
const bundles = device.createRenderBundleEncoder({colorFormats: ['rgba8unorm']});
bundles.draw(3);
let bundle;
log('bad bundle ' + await scoped(() => { bundle = bundles.finish(); }));
const other = device.createRenderBundleEncoder({colorFormats: ['bgra8unorm']}).finish();
const target = device.createTexture({size: [1, 1], format: 'rgba8unorm', usage: GPUTextureUsage.RENDER_ATTACHMENT});
log('other formats ' + await scoped(() => {
  const e = device.createCommandEncoder();
  const p = e.beginRenderPass({colorAttachments: [{view: target.createView(), loadOp: 'clear', storeOp: 'store'}]});
  p.executeBundles([other]);
  p.end();
  e.finish();
}));
log('bundle finished twice ' + await scoped(() => bundles.finish()));
device.queue.writeBuffer(buffer, 0, new Uint32Array([5, 6, 7, 8]));
await buffer.mapAsync(GPUMapMode.READ);
log('still working ' + Array.from(new Uint32Array(buffer.getMappedRange())).join(','));
"#)
    else {
        return;
    };
    assert_eq!(
        lines,
        [
            "finished twice GPUValidationError: The GPUCommandEncoder has already finished.",
            "locked GPUValidationError: The GPUCommandEncoder is locked while a pass of it is open.",
            "ended pass GPUValidationError: The GPUComputePassEncoder has already ended.",
            "used after its end GPUValidationError: The GPUComputePassEncoder has already ended.",
            "submitted none",
            "submitted twice GPUValidationError: A command buffer submitted is invalid, or was submitted before.",
            "popped GPUValidationError: A debug group was popped that was not pushed.",
            "no binding kind GPUValidationError: The entry of binding 0 has no binding layout.",
            "bad bundle GPUValidationError: A draw was made with no pipeline set.",
            "other formats GPUValidationError: A render bundle's formats or sample count are not the pass's.",
            "bundle finished twice GPUValidationError: The GPURenderBundleEncoder has already finished.",
            "still working 5,6,7,8",
        ]
    );
}

#[test]
fn a_texture_written_past_one_message_goes_in_pieces_of_rows() {
    let Some(lines) = run(r#"
const device = await adapter.requestDevice();
const width = 1024, height = 1100;
const data = new Uint8Array(width * 4 * height + 8);
for (let y = 0; y < height; y++) { data[8 + y * width * 4] = y & 255; data[8 + y * width * 4 + width * 4 - 1] = 255 - (y & 255); }
const texture = device.createTexture({size: {width, height}, format: 'rgba8unorm', usage: GPUTextureUsage.COPY_DST | GPUTextureUsage.COPY_SRC});
device.pushErrorScope('validation');
device.queue.writeTexture({texture}, data, {offset: 8, bytesPerRow: width * 4}, [width, height]);
const read = device.createBuffer({size: width * 4 * height, usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST});
const encoder = device.createCommandEncoder();
encoder.copyTextureToBuffer({texture}, {buffer: read, bytesPerRow: width * 4}, [width, height]);
device.queue.submit([encoder.finish()]);
log('errors ' + await device.popErrorScope());
await read.mapAsync(GPUMapMode.READ);
const back = new Uint8Array(read.getMappedRange());
let same = true;
for (let y = 0; y < height; y++) {
  if (back[y * width * 4] !== (y & 255) || back[y * width * 4 + width * 4 - 1] !== 255 - (y & 255)) same = false;
}
log('rows ' + same);
"#)
    else {
        return;
    };
    assert_eq!(lines, ["errors null", "rows true"]);
}

#[test]
fn a_canvas_context_presents_its_texture_as_the_canvas_after_the_frame() {
    let Some(lines) = run_framed(
        r#"
const canvas = document.createElement('canvas');
canvas.width = 4;
canvas.height = 2;
document.body.appendChild(canvas);
const context = canvas.getContext('webgpu');
log('context ' + context.constructor.name + ' ' + (context.canvas === canvas) + ' ' + (canvas.getContext('webgpu') === context) + ' ' + canvas.getContext('2d'));
try { context.getCurrentTexture(); } catch (e) { log('unconfigured ' + e.name); }
log('configuration ' + context.getConfiguration());
const device = await adapter.requestDevice();
const format = navigator.gpu.getPreferredCanvasFormat();
context.configure({device, format});
const c = context.getConfiguration();
log('configured ' + [c.format, c.alphaMode, c.colorSpace, c.toneMapping.mode, c.usage, c.device === device].join(' '));
const texture = context.getCurrentTexture();
log('texture ' + texture.width + 'x' + texture.height + ' ' + texture.format + ' ' + texture.usage + ' ' + (context.getCurrentTexture() === texture));
const encoder = device.createCommandEncoder();
encoder.beginRenderPass({colorAttachments: [{view: texture.createView(), loadOp: 'clear', clearValue: [1, 0, 0, 1], storeOp: 'store'}]}).end();
device.queue.submit([encoder.finish()]);
const frame = () => new Promise(r => requestAnimationFrame(r));
await frame();
// Within the frame's callbacks it is still the same: presented after them.
log('same in the frame ' + (context.getCurrentTexture() === texture));
// The rows come back a task or two later.
const read = document.createElement('canvas').getContext('2d');
let pixel = [0, 0, 0, 0];
for (let i = 0; i < 300 && pixel[3] === 0; i++) {
  await new Promise(r => setTimeout(r, 10));
  read.clearRect(0, 0, 1, 1);
  read.drawImage(canvas, 0, 0);
  pixel = Array.from(read.getImageData(3, 1, 1, 1).data);
}
log('pixel ' + pixel.join(','));
log('next frame ' + (context.getCurrentTexture() !== texture));
log('exported ' + (canvas.toDataURL().length > 50));
const other = new OffscreenCanvas(2, 2);
log('offscreen ' + other.getContext('webgpu').constructor.name + ' ' + other.getContext('2d'));
// A format not the canvas's throws; a usage, view format or size the
// texture cannot have is a validation error, the canvas configured.
const ctx = other.getContext('webgpu');
try { ctx.configure({device, format: 'rgba8uint'}); } catch (e) { log('rgba8uint ' + e.name + ' ' + ctx.getConfiguration()); }
const invalid = async (configuration) => {
  device.pushErrorScope('validation');
  ctx.configure(Object.assign({device, format: 'rgba8unorm'}, configuration));
  const error = await device.popErrorScope();
  return error === null ? 'none' : error.constructor.name;
};
log('usage 0 ' + await invalid({usage: 0}) + ', storage ' + await invalid({usage: GPUTextureUsage.STORAGE_BINDING}) +
  ', srgb view ' + await invalid({viewFormats: ['rgba8unorm-srgb']}) + ', r8 view ' + await invalid({viewFormats: ['r8unorm']}) +
  ' ' + ctx.getConfiguration().viewFormats);
other.width = 0;
log('zero ' + await invalid({}));
"#,
    ) else {
        return;
    };
    assert_eq!(
        lines,
        [
            "context GPUCanvasContext true true null",
            "unconfigured InvalidStateError",
            "configuration null",
            "configured bgra8unorm opaque srgb standard 16 true",
            "texture 4x2 bgra8unorm 16 true",
            "same in the frame true",
            "pixel 255,0,0,255",
            "next frame true",
            "exported true",
            "offscreen GPUCanvasContext null",
            "rgba8uint TypeError null",
            "usage 0 GPUValidationError, storage none, srgb view none, r8 view GPUValidationError r8unorm",
            "zero GPUValidationError",
        ]
    );
}

#[test]
fn an_image_is_copied_into_a_texture_flipped_as_asked() {
    let Some(lines) = run(r#"
const device = await adapter.requestDevice();
const source = document.createElement('canvas');
source.width = 2;
source.height = 2;
const g = source.getContext('2d');
g.fillStyle = 'blue';
g.fillRect(0, 0, 2, 2);
g.fillStyle = 'red';
g.fillRect(0, 0, 1, 1);
const texture = device.createTexture({size: [2, 2], format: 'rgba8unorm', usage: GPUTextureUsage.COPY_DST | GPUTextureUsage.COPY_SRC | GPUTextureUsage.RENDER_ATTACHMENT});
async function texels(flipY) {
  device.queue.copyExternalImageToTexture({source, flipY}, {texture}, [2, 2]);
  const read = device.createBuffer({size: 512, usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST});
  const encoder = device.createCommandEncoder();
  encoder.copyTextureToBuffer({texture}, {buffer: read, bytesPerRow: 256}, [2, 2]);
  device.queue.submit([encoder.finish()]);
  await read.mapAsync(GPUMapMode.READ);
  const px = new Uint8Array(read.getMappedRange());
  return [0, 4, 256, 260].map(i => px[i] ? 'r' : px[i + 2] ? 'b' : '?').join('');
}
log('copied ' + await texels(false));
log('flipped ' + await texels(true));
try { device.queue.copyExternalImageToTexture({source, origin: [1, 1]}, {texture}, [2, 2]); } catch (e) { log('outside ' + e.name); }
try { device.queue.copyExternalImageToTexture({source: 5}, {texture}, [1, 1]); } catch (e) { log('not an image ' + e.name); }
"#)
    else {
        return;
    };
    assert_eq!(
        lines,
        [
            "copied rbbb",
            "flipped bbrb",
            "outside OperationError",
            "not an image TypeError",
        ]
    );
}
