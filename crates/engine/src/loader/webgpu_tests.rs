//! WebGPU's objects (M41 step 2, D39): adapters, devices, buffers and
//! their mapping, errors and loss, as the reference showed them (probe
//! q3, scratchpad `m41/pages`), on this machine's adapter through a GPU
//! service in this process. Where there is no adapter (CI's Windows
//! runners), a test says so and checks what it can without one.

use net::test_server::TestServer;

use crate::testing::*;

pub(super) fn server(script: &str) -> TestServer {
    let body = format!(
        "<script>const log = (s) => console.log(s);\n\
         (async () => {{\n{script}\n}})().then(() => log('done'), (e) => log('failed ' + e + ' ' + e.stack));</script>"
    );
    serve(move |path| match path {
        p if p.ends_with(".html") => Some(html(&body)),
        _ => None,
    })
}

/// The page's lines until "done".
pub(super) fn lines(page: &TestServer) -> Vec<String> {
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    tab.engine.navigate(page.url("/page.html"));
    tab.run_until(|lines| lines.iter().any(|l| l == "done" || l.starts_with("failed")))
        .into_iter()
        .filter(|l| l != "done")
        .collect()
}

/// The script's first lines: an adapter, or "no adapter" and the end.
pub(super) const ADAPTER: &str = r#"
const adapter = await navigator.gpu.requestAdapter();
if (adapter === null) { log('no adapter'); return; }
"#;

#[test]
fn navigator_gpu_gives_an_adapter_with_its_features_limits_and_info() {
    let page = server(&format!(
        r#"
log('gpu ' + navigator.gpu.constructor.name + ' ' + isSecureContext);
log('preferred ' + navigator.gpu.getPreferredCanvasFormat());
log('wgsl ' + [...navigator.gpu.wgslLanguageFeatures].sort().join(','));
log('bad power ' + await navigator.gpu.requestAdapter({{powerPreference: 'fast'}}).catch(e => e.name + ': ' + e.message));
{ADAPTER}
log('adapter ' + adapter.constructor.name + ' ' + adapter.features.constructor.name + ' ' + adapter.limits.constructor.name);
log('core ' + adapter.features.has('core-features-and-limits'));
// The adapter's own, in wgpu's buckets: at least the standard's.
const align = adapter.limits.minUniformBufferOffsetAlignment;
log('limits ' + (adapter.limits.maxTextureDimension2D >= 8192) + ' ' + (adapter.limits.maxBindGroups >= 4) + ' ' + (align <= 256 && (align & (align - 1)) === 0));
const info = adapter.info;
log('info ' + typeof info.vendor + ' [' + info.architecture + '][' + info.device + '][' + info.description + '] ' + (info.isFallbackAdapter === adapter.isFallbackAdapter));
"#
    ));
    let lines = lines(&page);
    assert_eq!(
        lines[..4],
        [
            "gpu GPU true",
            "preferred bgra8unorm",
            "wgsl packed_4x8_integer_dot_product,pointer_composite_access,readonly_and_readwrite_storage_textures",
            "bad power TypeError: Failed to execute 'requestAdapter' on 'GPU': Failed to read the 'powerPreference' property from 'GPURequestAdapterOptions': The provided value 'fast' is not a valid enum value of type GPUPowerPreference.",
        ]
    );
    if lines[4] == "no adapter" {
        eprintln!("no adapter here: its objects are not checked");
        return;
    }
    assert_eq!(
        lines[4..],
        [
            "adapter GPUAdapter GPUSupportedFeatures GPUSupportedLimits",
            "core true",
            "limits true true true",
            "info string [][][] true",
        ]
    );
}

#[test]
fn a_device_has_the_standards_defaults_and_its_adapter_is_consumed() {
    let page = server(&format!(
        r#"{ADAPTER}
log('bad feature ' + await adapter.requestDevice({{requiredFeatures: ['bogus-feature']}}).catch(e => e.name + ': ' + e.message));
log('too many ' + await adapter.requestDevice({{requiredLimits: {{maxBindGroups: 1000}}}}).catch(e => e.name + ': ' + e.message.replace('(' + adapter.limits.maxBindGroups + ')', '(N)')));
const device = await adapter.requestDevice();
log('device ' + device.constructor.name + ' ' + device.queue.constructor.name);
log('defaults ' + device.limits.maxTextureDimension2D + ' ' + device.limits.maxBindGroups + ' ' + device.limits.maxBufferSize);
log('features ' + [...device.features].join(','));
log('info ' + (device.adapterInfo === adapter.info));
log('second ' + await adapter.requestDevice().catch(e => e.name + ': ' + e.message));
"#
    ));
    let lines = lines(&page);
    if lines == ["no adapter"] {
        eprintln!("no adapter here: devices are not checked");
        return;
    }
    assert_eq!(
        lines,
        [
            "bad feature TypeError: Failed to execute 'requestDevice' on 'GPUAdapter': Failed to read the 'requiredFeatures' property from 'GPUDeviceDescriptor': The provided value 'bogus-feature' is not a valid enum value of type GPUFeatureName.",
            "too many OperationError: Failed to execute 'requestDevice' on 'GPUAdapter': Required limit (1000) is greater than the supported limit (N).\n - While validating maxBindGroups\n - While validating required limits",
            "device GPUDevice GPUQueue",
            "defaults 8192 4 268435456",
            "features core-features-and-limits",
            "info true",
            "second OperationError: Failed to execute 'requestDevice' on 'GPUAdapter': adapter is \"consumed\": it has already been used to create a device",
        ]
    );
}

#[test]
fn errors_go_to_the_innermost_scope_or_the_uncaptured_event() {
    let page = server(&format!(
        r#"{ADAPTER}
const device = await adapter.requestDevice();
device.pushErrorScope('validation');
device.createBuffer({{size: 4, usage: 0}});
const error = await device.popErrorScope();
log('scoped ' + error.constructor.name + ' ' + (error instanceof GPUError) + ' ' + (error.message.length > 0));
device.pushErrorScope('out-of-memory');
device.createBuffer({{size: 4, usage: 0}});
const none = await device.popErrorScope().catch(e => 'rejected');
log('other filter ' + none);
log('empty ' + await device.popErrorScope().catch(e => e.name + ': ' + e.message));
try {{ device.pushErrorScope('bogus'); }} catch (e) {{ log('bad filter ' + e.name); }}
const heard = new Promise(r => device.onuncapturederror = (e) => r(e));
device.createBuffer({{size: 4, usage: 0}});
const event = await heard;
log('uncaptured ' + event.constructor.name + ' ' + event.error.constructor.name + ' ' + event.isTrusted);
"#
    ));
    let lines = lines(&page);
    if lines == ["no adapter"] {
        eprintln!("no adapter here: errors are not checked");
        return;
    }
    assert_eq!(
        lines,
        [
            "scoped GPUValidationError true true",
            "other filter null",
            "empty OperationError: No error scopes to pop",
            "bad filter TypeError",
            "uncaptured GPUUncapturedErrorEvent GPUValidationError true",
        ]
    );
}

#[test]
fn a_buffer_written_by_the_queue_is_read_back_by_mapping_it() {
    let page = server(&format!(
        r#"{ADAPTER}
const device = await adapter.requestDevice();
const buffer = device.createBuffer({{size: 16, usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST}});
log('made ' + buffer.size + ' ' + buffer.usage + ' ' + buffer.mapState);
device.queue.writeBuffer(buffer, 0, new Float32Array([1, 2, 3, 4.5]));
const mapping = buffer.mapAsync(GPUMapMode.READ);
log('pending ' + buffer.mapState);
await mapping;
log('mapped ' + buffer.mapState);
const range = buffer.getMappedRange();
log('read ' + Array.from(new Float32Array(range)).join(','));
try {{ buffer.getMappedRange(); }} catch (e) {{ log('overlap ' + e.name); }}
buffer.unmap();
log('unmapped ' + buffer.mapState + ' ' + range.byteLength);
try {{ buffer.getMappedRange(); }} catch (e) {{ log('after unmap ' + e.name + ': ' + e.message); }}
log('twice ' + await buffer.mapAsync(GPUMapMode.WRITE).catch(e => e.name));
// Mapped at creation: what the page writes is the buffer's.
const made = device.createBuffer({{size: 8, usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST, mappedAtCreation: true}});
new Uint32Array(made.getMappedRange()).set([7, 9]);
made.unmap();
await made.mapAsync(GPUMapMode.READ);
log('created ' + Array.from(new Uint32Array(made.getMappedRange())).join(','));
await device.queue.onSubmittedWorkDone();
log('work done');
"#
    ));
    let lines = lines(&page);
    if lines == ["no adapter"] {
        eprintln!("no adapter here: buffers are not checked");
        return;
    }
    assert_eq!(
        lines,
        [
            "made 16 9 unmapped",
            "pending pending",
            "mapped mapped",
            "read 1,2,3,4.5",
            "overlap OperationError",
            "unmapped unmapped 0",
            "after unmap OperationError: Failed to execute 'getMappedRange' on 'GPUBuffer': getMappedRange failed",
            "twice OperationError",
            "created 7,9",
            "work done",
        ]
    );
}

#[test]
fn a_destroyed_device_is_lost_as_destroyed() {
    let page = server(&format!(
        r#"{ADAPTER}
const device = await adapter.requestDevice();
device.destroy();
const info = await device.lost;
log('lost ' + info.constructor.name + ' ' + info.reason + ' ' + (typeof info.message));
"#
    ));
    let lines = lines(&page);
    if lines == ["no adapter"] {
        eprintln!("no adapter here: loss is not checked");
        return;
    }
    assert_eq!(lines, ["lost GPUDeviceLostInfo destroyed string"]);
}
