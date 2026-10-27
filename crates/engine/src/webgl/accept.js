// M41's acceptance pages (docs/design/gpu.md, M41.6): WebGL 1 and 2
// drawing a lit, textured cube, points of a size, a framebuffer drawn
// into and read, and WebGPU drawing a triangle and running a compute
// pass, each saying pixels or values at points the edges do not touch.
// The reference browser's lines are acceptance.rs's REFERENCE.
async function accept(log) {
  const multiply = (a, b) => {
    const out = new Float32Array(16);
    for (let c = 0; c < 4; c++) {
      for (let r = 0; r < 4; r++) {
        let v = 0;
        for (let k = 0; k < 4; k++) v += a[k * 4 + r] * b[c * 4 + k];
        out[c * 4 + r] = v;
      }
    }
    return out;
  };
  const perspective = (fov, aspect, near, far) => {
    const f = 1 / Math.tan(fov / 2);
    return new Float32Array([f / aspect, 0, 0, 0, 0, f, 0, 0, 0, 0, (far + near) / (near - far), -1, 0, 0, 2 * far * near / (near - far), 0]);
  };
  const rotate = (ax, ay) => {
    const [cx, sx, cy, sy] = [Math.cos(ax), Math.sin(ax), Math.cos(ay), Math.sin(ay)];
    const x = new Float32Array([1, 0, 0, 0, 0, cx, sx, 0, 0, -sx, cx, 0, 0, 0, 0, 1]);
    const y = new Float32Array([cy, 0, -sy, 0, 0, 1, 0, 0, sy, 0, cy, 0, 0, 0, 0, 1]);
    return multiply(x, y);
  };
  const translate = (z) => new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, z, 1]);
  const model = rotate(0.6, 0.8);
  const mvp = multiply(perspective(0.9, 1, 0.1, 10), multiply(translate(-3), model));
  // A cube's faces: positions, normals and texture coordinates.
  const faces = [
    [[0, 0, 1], [1, 0, 0], [0, 1, 0]], [[0, 0, -1], [-1, 0, 0], [0, 1, 0]],
    [[1, 0, 0], [0, 0, -1], [0, 1, 0]], [[-1, 0, 0], [0, 0, 1], [0, 1, 0]],
    [[0, 1, 0], [1, 0, 0], [0, 0, -1]], [[0, -1, 0], [1, 0, 0], [0, 0, 1]],
  ];
  const data = [];
  for (const [n, u, v] of faces) {
    const corner = (a, b) => [n[0] + a * u[0] + b * v[0], n[1] + a * u[1] + b * v[1], n[2] + a * u[2] + b * v[2]];
    for (const [a, b] of [[-1, -1], [1, -1], [1, 1], [-1, -1], [1, 1], [-1, 1]]) {
      data.push(...corner(a, b), ...n, (a + 1) / 2, (b + 1) / 2);
    }
  }
  const vertices = new Float32Array(data);
  const checker = new Uint8Array([255, 64, 64, 255, 64, 64, 255, 255, 64, 64, 255, 255, 255, 64, 64, 255]);
  const points = [[32, 32], [24, 40], [40, 24], [20, 28], [44, 38], [4, 4], [60, 58]];
  const read = (gl, at) => at.map(([x, y]) => {
    const px = new Uint8Array(4);
    gl.readPixels(x, y, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
    return x + ',' + y + '=' + Array.from(px).join(',');
  }).join(' ');
  const compile = (gl, vs, fs) => {
    const p = gl.createProgram();
    for (const [type, src] of [[gl.VERTEX_SHADER, vs], [gl.FRAGMENT_SHADER, fs]]) {
      const s = gl.createShader(type);
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) log('compile ' + gl.getShaderInfoLog(s));
      gl.attachShader(p, s);
    }
    gl.bindAttribLocation(p, 0, 'position');
    gl.bindAttribLocation(p, 1, 'normal');
    gl.bindAttribLocation(p, 2, 'uv');
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) log('link ' + gl.getProgramInfoLog(p));
    gl.useProgram(p);
    return p;
  };
  const cube = (gl, two) => {
    const buffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, vertices, gl.STATIC_DRAW);
    for (const [i, size, offset] of [[0, 3, 0], [1, 3, 12], [2, 2, 24]]) {
      gl.enableVertexAttribArray(i);
      gl.vertexAttribPointer(i, size, gl.FLOAT, false, 32, offset);
    }
    const t = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 2, 2, 0, gl.RGBA, gl.UNSIGNED_BYTE, checker);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.enable(gl.DEPTH_TEST);
    gl.enable(gl.CULL_FACE);
    gl.clearColor(0.1, 0.1, 0.1, 1);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    gl.drawArrays(gl.TRIANGLES, 0, 36);
    log((two ? 'webgl2' : 'webgl1') + ' cube ' + gl.getError() + ' ' + read(gl, points));
  };
  const canvas = (w, h) => {
    const c = document.createElement('canvas');
    c.width = w;
    c.height = h;
    return c;
  };
  const light = 'float l = max(dot(normalize(n), normalize(vec3(0.4, 0.7, 0.6))), 0.0) * 0.8 + 0.2;';

  // WebGL 1: the cube.
  {
    const gl = canvas(64, 64).getContext('webgl', {antialias: false});
    if (gl === null) {
      log('no gl');
      return;
    }
    const p = compile(gl,
      'attribute vec3 position; attribute vec3 normal; attribute vec2 uv; uniform mat4 mvp; uniform mat4 model; varying vec3 n; varying vec2 t;\n' +
      'void main() { n = (model * vec4(normal, 0.0)).xyz; t = uv; gl_Position = mvp * vec4(position, 1.0); }',
      'precision mediump float; uniform sampler2D tex; varying vec3 n; varying vec2 t;\n' +
      'void main() { ' + light + ' gl_FragColor = vec4(texture2D(tex, t).rgb * l, 1.0); }');
    gl.uniformMatrix4fv(gl.getUniformLocation(p, 'mvp'), false, mvp);
    gl.uniformMatrix4fv(gl.getUniformLocation(p, 'model'), false, model);
    cube(gl, false);
    // Points of a size: 9 pixels across.
    gl.disable(gl.DEPTH_TEST);
    compile(gl,
      'attribute vec3 position; void main() { gl_Position = vec4(position, 1.0); gl_PointSize = 9.0; }',
      'precision mediump float; void main() { gl_FragColor = vec4(gl_PointCoord.x, gl_PointCoord.y, 1.0, 1.0); }');
    gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-0.5, -0.5, 0, 0.5, 0.5, 0]), gl.STATIC_DRAW);
    gl.disableVertexAttribArray(1);
    gl.disableVertexAttribArray(2);
    gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.drawArrays(gl.POINTS, 0, 2);
    log('points ' + gl.getError() + ' ' + read(gl, [[16, 16], [18, 18], [48, 48], [46, 50], [16, 24], [32, 32]]));
    // A framebuffer drawn into, then read and sampled.
    const target = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, target);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 16, 16, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    const fb = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, target, 0);
    gl.viewport(0, 0, 16, 16);
    gl.clearColor(0, 0, 1, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.enable(gl.SCISSOR_TEST);
    gl.scissor(0, 0, 8, 16);
    gl.clearColor(1, 1, 0, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.disable(gl.SCISSOR_TEST);
    log('framebuffer ' + gl.checkFramebufferStatus(gl.FRAMEBUFFER).toString(16) + ' ' + read(gl, [[2, 8], [13, 8]]));
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, 64, 64);
    compile(gl,
      'attribute vec3 position; varying vec2 t; void main() { t = position.xy * 0.5 + 0.5; gl_Position = vec4(position, 1.0); }',
      'precision mediump float; uniform sampler2D tex; varying vec2 t; void main() { gl_FragColor = texture2D(tex, t); }');
    gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 0, 1, -1, 0, -1, 1, 0, 1, 1, 0]), gl.STATIC_DRAW);
    gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    log('sampled ' + gl.getError() + ' ' + read(gl, [[10, 32], [54, 32]]));
  }

  // WebGL 2: the cube from a vertex array and a uniform block.
  {
    const gl = canvas(64, 64).getContext('webgl2', {antialias: false});
    const p = compile(gl,
      '#version 300 es\nin vec3 position; in vec3 normal; in vec2 uv; uniform Matrices { mat4 mvp; mat4 model; }; out vec3 n; out vec2 t;\n' +
      'void main() { n = (model * vec4(normal, 0.0)).xyz; t = uv; gl_Position = mvp * vec4(position, 1.0); }',
      '#version 300 es\nprecision mediump float; uniform sampler2D tex; in vec3 n; in vec2 t; out vec4 o;\n' +
      'void main() { ' + light + ' o = vec4(texture(tex, t).rgb * l, 1.0); }');
    const ubo = gl.createBuffer();
    gl.bindBuffer(gl.UNIFORM_BUFFER, ubo);
    const matrices = new Float32Array(32);
    matrices.set(mvp, 0);
    matrices.set(model, 16);
    gl.bufferData(gl.UNIFORM_BUFFER, matrices, gl.STATIC_DRAW);
    gl.uniformBlockBinding(p, gl.getUniformBlockIndex(p, 'Matrices'), 0);
    gl.bindBufferBase(gl.UNIFORM_BUFFER, 0, ubo);
    gl.bindVertexArray(gl.createVertexArray());
    cube(gl, true);
  }

  // WebGPU: a triangle drawn into a texture and read; a compute pass.
  if (!navigator.gpu) {
    log('webgpu none');
    return;
  }
  const adapter = await navigator.gpu.requestAdapter();
  const device = await adapter.requestDevice();
  const module = device.createShaderModule({code: `
    @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
      var p = array(vec2f(-0.8, -0.8), vec2f(0.8, -0.8), vec2f(0.0, 0.8));
      return vec4f(p[i], 0.0, 1.0);
    }
    @fragment fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f { return vec4f(p.x / 32.0, p.y / 32.0, 0.5, 1.0); }
    @group(0) @binding(0) var<storage, read_write> values: array<u32>;
    @compute @workgroup_size(8) fn main(@builtin(global_invocation_id) id: vec3u) { values[id.x] = values[id.x] * 3u + id.x; }`});
  const texture = device.createTexture({size: [32, 32], format: 'rgba8unorm', usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC});
  const pipeline = device.createRenderPipeline({layout: 'auto', vertex: {module, entryPoint: 'vs'},
    fragment: {module, entryPoint: 'fs', targets: [{format: 'rgba8unorm'}]}});
  const readback = device.createBuffer({size: 256 * 32, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ});
  let encoder = device.createCommandEncoder();
  const pass = encoder.beginRenderPass({colorAttachments: [{view: texture.createView(), loadOp: 'clear', clearValue: [0, 0, 0, 1], storeOp: 'store'}]});
  pass.setPipeline(pipeline);
  pass.draw(3);
  pass.end();
  encoder.copyTextureToBuffer({texture}, {buffer: readback, bytesPerRow: 256}, [32, 32]);
  device.queue.submit([encoder.finish()]);
  await readback.mapAsync(GPUMapMode.READ);
  const pixels = new Uint8Array(readback.getMappedRange());
  const at = (x, y) => x + ',' + y + '=' + Array.from(pixels.slice(y * 256 + x * 4, y * 256 + x * 4 + 4)).join(',');
  log('webgpu triangle ' + [at(16, 16), at(10, 24), at(22, 24), at(2, 2), at(30, 4)].join(' '));
  readback.unmap();
  const storage = device.createBuffer({size: 64, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST});
  device.queue.writeBuffer(storage, 0, new Uint32Array([...Array(16).keys()].map((v) => v * 10)));
  const compute = device.createComputePipeline({layout: 'auto', compute: {module, entryPoint: 'main'}});
  const group = device.createBindGroup({layout: compute.getBindGroupLayout(0), entries: [{binding: 0, resource: {buffer: storage}}]});
  const out = device.createBuffer({size: 64, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ});
  encoder = device.createCommandEncoder();
  const cpass = encoder.beginComputePass();
  cpass.setPipeline(compute);
  cpass.setBindGroup(0, group);
  cpass.dispatchWorkgroups(2);
  cpass.end();
  encoder.copyBufferToBuffer(storage, 0, out, 0, 64);
  device.queue.submit([encoder.finish()]);
  await out.mapAsync(GPUMapMode.READ);
  log('webgpu compute ' + Array.from(new Uint32Array(out.getMappedRange())).join(','));
}
