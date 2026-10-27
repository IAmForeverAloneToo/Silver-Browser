//! M41's benchmark: WebGL's frames and draws, and WebGPU's round trip.
//!
//! - **A frame of a WebGL page.** A 512 by 512 canvas drawing a lit,
//!   textured cube turning each animation frame, on a page of 800 by 600:
//!   the time a frame, its script's draw and the canvas presented and
//!   composited, for two seconds of frames.
//! - **A WebGL draw call.** A uniform set and a triangle drawn, a
//!   thousand times, then a pixel read (which waits for the GPU): the time
//!   a call, as the page's clock says.
//! - **A WebGPU compute round trip.** Sixteen numbers written to a
//!   buffer, a compute pass over them, the buffer copied and mapped and
//!   read: the time a round trip, as the page's clock says.
//!
//! Where there is no adapter, it says so. `harness = false` (see
//! `Cargo.toml`): a plain binary, run by hand, its numbers recorded in
//! `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench gpu
//! ```

#![forbid(unsafe_code)]

mod support;

use std::time::{Duration, Instant};

use engine::{Engine, LoadState};
use net::test_server::{Reply, TestServer};

const RUNS: usize = 20;

/// The cube page: a canvas drawing the cube each animation frame.
const CUBE: &str = r#"<body style=margin:0><canvas id=c width=512 height=512></canvas><script>
const gl = c.getContext('webgl', {antialias: false});
const program = (vs, fs) => {
  const p = gl.createProgram();
  for (const [type, src] of [[gl.VERTEX_SHADER, vs], [gl.FRAGMENT_SHADER, fs]]) {
    const s = gl.createShader(type);
    gl.shaderSource(s, src);
    gl.compileShader(s);
    gl.attachShader(p, s);
  }
  gl.bindAttribLocation(p, 0, 'position');
  gl.bindAttribLocation(p, 1, 'normal');
  gl.bindAttribLocation(p, 2, 'uv');
  gl.linkProgram(p);
  gl.useProgram(p);
  return p;
};
const p = program(
  'attribute vec3 position; attribute vec3 normal; attribute vec2 uv; uniform float a; varying vec3 n; varying vec2 t;\n' +
  'void main() { float c = cos(a), s = sin(a); mat3 r = mat3(c, 0.0, -s, 0.0, 1.0, 0.0, s, 0.0, c);\n' +
  'n = r * normal; t = uv; vec3 q = r * position; gl_Position = vec4(q.xy * 0.5, q.z * 0.1, 1.0); }',
  'precision mediump float; uniform sampler2D tex; varying vec3 n; varying vec2 t;\n' +
  'void main() { float l = max(dot(normalize(n), vec3(0.48, 0.7, 0.53)), 0.0) * 0.8 + 0.2; gl_FragColor = vec4(texture2D(tex, t).rgb * l, 1.0); }');
const data = [];
for (const [n, u, v] of [[[0, 0, 1], [1, 0, 0], [0, 1, 0]], [[0, 0, -1], [-1, 0, 0], [0, 1, 0]],
  [[1, 0, 0], [0, 0, -1], [0, 1, 0]], [[-1, 0, 0], [0, 0, 1], [0, 1, 0]],
  [[0, 1, 0], [1, 0, 0], [0, 0, -1]], [[0, -1, 0], [1, 0, 0], [0, 0, 1]]]) {
  for (const [a, b] of [[-1, -1], [1, -1], [1, 1], [-1, -1], [1, 1], [-1, 1]]) {
    data.push(n[0] + a * u[0] + b * v[0], n[1] + a * u[1] + b * v[1], n[2] + a * u[2] + b * v[2], ...n, (a + 1) / 2, (b + 1) / 2);
  }
}
gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(data), gl.STATIC_DRAW);
for (const [i, size, offset] of [[0, 3, 0], [1, 3, 12], [2, 2, 24]]) {
  gl.enableVertexAttribArray(i);
  gl.vertexAttribPointer(i, size, gl.FLOAT, false, 32, offset);
}
gl.bindTexture(gl.TEXTURE_2D, gl.createTexture());
gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 2, 2, 0, gl.RGBA, gl.UNSIGNED_BYTE,
  new Uint8Array([255, 64, 64, 255, 64, 64, 255, 255, 64, 64, 255, 255, 255, 64, 64, 255]));
gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
gl.enable(gl.DEPTH_TEST);
const angle = gl.getUniformLocation(p, 'a');
let frame = 0;
const draw = () => {
  gl.clearColor(0.1, 0.1, 0.1, 1);
  gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
  gl.uniform1f(angle, frame++ * 0.02);
  gl.drawArrays(gl.TRIANGLES, 0, 36);
  requestAnimationFrame(draw);
};
requestAnimationFrame(draw);
</script>"#;

/// The page timing draw calls and compute round trips, saying each
/// run's time a call or a round trip, then "end".
const TIMED: &str = r#"<script>
(async () => {
  const c = document.createElement('canvas');
  c.width = 64;
  c.height = 64;
  const gl = c.getContext('webgl', {antialias: false});
  if (gl === null) { console.log('no gl'); console.log('end'); return; }
  const p = gl.createProgram();
  for (const [type, src] of [[gl.VERTEX_SHADER, 'attribute vec2 position; uniform vec2 at; void main() { gl_Position = vec4(position * 0.1 + at, 0.0, 1.0); }'],
    [gl.FRAGMENT_SHADER, 'precision mediump float; void main() { gl_FragColor = vec4(1.0, 0.5, 0.0, 1.0); }']]) {
    const s = gl.createShader(type);
    gl.shaderSource(s, src);
    gl.compileShader(s);
    gl.attachShader(p, s);
  }
  gl.linkProgram(p);
  gl.useProgram(p);
  gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, 0, 1]), gl.STATIC_DRAW);
  gl.enableVertexAttribArray(0);
  gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
  const at = gl.getUniformLocation(p, 'at');
  const pixel = new Uint8Array(4);
  for (let run = 0; run < RUNS; run++) {
    const began = performance.now();
    for (let i = 0; i < 1000; i++) {
      gl.uniform2f(at, (i % 20) * 0.1 - 1, ((i / 20) | 0) * 0.04 - 1);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
    }
    gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
    console.log('draw ' + (performance.now() - began) / 1000);
  }
  const adapter = navigator.gpu && await navigator.gpu.requestAdapter();
  if (!adapter) { console.log('end'); return; }
  const device = await adapter.requestDevice();
  const module = device.createShaderModule({code: `@group(0) @binding(0) var<storage, read_write> v: array<u32>;
    @compute @workgroup_size(16) fn main(@builtin(global_invocation_id) id: vec3u) { v[id.x] = v[id.x] * 3u + id.x; }`});
  const pipeline = device.createComputePipeline({layout: 'auto', compute: {module, entryPoint: 'main'}});
  const storage = device.createBuffer({size: 64, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST});
  const out = device.createBuffer({size: 64, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ});
  const group = device.createBindGroup({layout: pipeline.getBindGroupLayout(0), entries: [{binding: 0, resource: {buffer: storage}}]});
  const values = new Uint32Array(16);
  for (let run = 0; run < RUNS; run++) {
    const began = performance.now();
    device.queue.writeBuffer(storage, 0, values);
    const encoder = device.createCommandEncoder();
    const pass = encoder.beginComputePass();
    pass.setPipeline(pipeline);
    pass.setBindGroup(0, group);
    pass.dispatchWorkgroups(1);
    pass.end();
    encoder.copyBufferToBuffer(storage, 0, out, 0, 64);
    device.queue.submit([encoder.finish()]);
    await out.mapAsync(GPUMapMode.READ);
    values.set(new Uint32Array(out.getMappedRange()));
    out.unmap();
    console.log('compute ' + (performance.now() - began));
  }
  console.log('end');
})();
</script>"#;

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// The median and range of `times`, in milliseconds.
fn spread(times: &mut [f64]) -> String {
    times.sort_by(f64::total_cmp);
    format!(
        "median {:.3} ms, {:.3}-{:.3} ms",
        times[times.len() / 2],
        times[0],
        times[times.len() - 1]
    )
}

fn engine(server: &TestServer) -> Engine {
    let mut engine = Engine::new().expect("an engine");
    engine.set_scripting(true);
    engine.navigate(server.url("/page.html"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while *engine.load_state() != LoadState::Loaded && Instant::now() < deadline {
        engine.poll_network();
        engine.run_tasks();
        std::thread::sleep(Duration::from_millis(5));
    }
    engine
}

/// Two seconds of the cube page's frames (a fifth of a second in the quick
/// mode): the times a frame.
fn frames() -> Vec<f64> {
    let server = TestServer::plain(|request| match request.target.as_str() {
        "/page.html" => Reply::ok("text/html", CUBE.as_bytes()),
        _ => Reply::with_fields(404, &[], b""),
    });
    let mut engine = engine(&server);
    // Its first frames (the pipeline made), before the rest are timed.
    for _ in 0..10 {
        engine.run_tasks();
        engine.frame(800, 600);
    }
    let mut times = Vec::new();
    let span = if support::quick() {
        Duration::from_millis(200)
    } else {
        Duration::from_secs(2)
    };
    let end = Instant::now() + span;
    while times.is_empty() || Instant::now() < end {
        engine.poll_network();
        engine.run_tasks();
        let started = Instant::now();
        engine.frame(800, 600);
        times.push(ms(started.elapsed()));
    }
    times
}

/// The timed page's lines: its times a draw call and a round trip.
fn timed() -> (Vec<f64>, Vec<f64>) {
    let page = TIMED.replace("RUNS", &support::rounds(RUNS).to_string());
    let server = TestServer::plain(move |request| match request.target.as_str() {
        "/page.html" => Reply::ok("text/html", page.as_bytes()),
        _ => Reply::with_fields(404, &[], b""),
    });
    let mut engine = engine(&server);
    let (mut draws, mut trips) = (Vec::new(), Vec::new());
    let deadline = Instant::now() + Duration::from_secs(60);
    'run: while Instant::now() < deadline {
        engine.poll_network();
        engine.run_tasks();
        for entry in engine.take_console() {
            let text = entry.text;
            if let Some(v) = text.strip_prefix("draw ") {
                draws.push(v.parse().expect("a number"));
            } else if let Some(v) = text.strip_prefix("compute ") {
                trips.push(v.parse().expect("a number"));
            } else if text == "end" {
                break 'run;
            }
        }
        if engine.needs_frame() {
            engine.frame(800, 600);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    (draws, trips)
}

fn main() {
    if engine::gpu::local_client().is_none() {
        println!("no adapter here: nothing to time");
        return;
    }
    let mut times = frames();
    let count = times.len();
    println!(
        "a frame of a page drawing a WebGL cube at 512 by 512: {} ({count} frames)",
        spread(&mut times)
    );
    let (mut draws, mut trips) = timed();
    if !draws.is_empty() {
        println!(
            "a WebGL draw call (a uniform and a triangle): {}",
            spread(&mut draws)
        );
    }
    if !trips.is_empty() {
        println!(
            "a WebGPU compute round trip (write, dispatch, copy, map): {}",
            spread(&mut trips)
        );
    }
}
