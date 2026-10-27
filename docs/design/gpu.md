# Design note: WebGL and WebGPU

Roadmap M41. Written before the code, as the record of the decisions,
and corrected to the code when the milestone ends: where they
disagree, the code wins, and section 4 lists the places, marked
*(corrected)*. What ships is described in `docs/architecture.md`
"Current state", and the limits it adds live in `docs/security.md`.

A page here draws on a canvas in two dimensions and nothing else.
`getContext("webgl")`, `"webgl2"` and `"webgpu"` return `null`;
`navigator.gpu` does not exist; no `WebGLRenderingContext`, no `GPU*`
interface. The engine has a GPU backend (M19.3, D3): in one process the
shell draws pages through `wgpu` over Direct3D 12, Vulkan or Metal,
the frame read back for the window. With renderers (M34), pages are
drawn in software: a renderer is sandboxed, and the GPU's driver is a
large surface to give it (G110). A page whose content is a map, a
model, a chart drawn by WebGL, a game, or a computation on the GPU
does not work.

This milestone gives the engine WebGL 1 and 2 and WebGPU. The GPU is
driven by a *GPU process* of its own (closing G110 for these APIs),
which runs `wgpu` and answers the renderers' requests through the
browser. WebGPU is `wgpu`'s own model, which already validates what
the standard asks. WebGL is built on the same WebGPU, in the renderer:
its GLSL ES shaders are read by a front end of ours, translated to
WGSL and validated before they leave the renderer, and its state
machine is kept there and drawn through WebGPU pipelines, so that the
GPU process meets WebGPU alone. The canvas's pixels come back to the
renderer for its page, as a 2D canvas's do. Its acceptance is a set of
pages drawing with WebGL and WebGPU as the reference browser does, a
subset of the Khronos WebGL conformance suite and of the WebGPU
conformance test suite, and WPT's `webgl` directory.

The documents this note follows, at the versions current when it was
written (26 October 2026):

- WebGL 1.0 (Khronos, the specification and its editor's draft), its
  differences from OpenGL ES 2.0 (section 6), the context creation
  attributes, the drawing buffer, context loss; WebGL 2.0 and its
  differences from OpenGL ES 3.0; the registered extensions this note
  names;
- OpenGL ES 2.0.25 and OpenGL ES 3.0.6 (Khronos), for the behaviour
  WebGL defers to;
- The OpenGL ES Shading Language 1.00 (revision 17, with Appendix A's
  limits as WebGL requires them) and 3.00 (revision 6);
- WebGPU (W3C Candidate Recommendation Draft of 2026) and WGSL;
- the HTML Standard's canvas element: `getContext()`, the context
  modes, `OffscreenCanvas`, `toDataURL()`, `toBlob()`, `ImageBitmap`
  and the canvas image source rules (origin-clean);
- the Khronos WebGL conformance suite (`sdk/tests`, main at 3a65272e4,
  21 October 2026) and the WebGPU conformance test suite (gpuweb/cts,
  its published WPT build of 24 October 2026).

What the standards leave open, or say but are worth checking, was put
to the reference browser (Chrome 156.0.8078.47, headless, on this
machine's Intel UHD Graphics) before this note was written, its pages
served on `127.0.0.1` (scratchpad `m41/pages`, `q1` and `q3`).

- *WebGL's identity.* `getContext("webgl")` is a
  `WebGLRenderingContext`, `"webgl2"` a `WebGL2RenderingContext`; a
  second `getContext` of the same kind returns the same object, of
  another kind `null`. `getContextAttributes()`: `alpha`, `antialias`,
  `depth`, `premultipliedAlpha` true; `stencil`,
  `preserveDrawingBuffer`, `desynchronized`,
  `failIfMajorPerformanceCaveat`, `xrCompatible` false;
  `powerPreference` `"default"`. `VERSION` is "WebGL 1.0 (OpenGL ES 2.0
  Chromium)" and "WebGL 2.0 (OpenGL ES 3.0 Chromium)",
  `SHADING_LANGUAGE_VERSION` "WebGL GLSL ES 1.0 (OpenGL ES GLSL ES 1.0
  Chromium)" and "WebGL GLSL ES 3.00 (OpenGL ES GLSL ES 3.0
  Chromium)", `VENDOR` "WebKit", `RENDERER` "WebKit WebGL";
  `WEBGL_debug_renderer_info` gives "Google Inc. (Intel)" and "ANGLE
  (Intel, Intel(R) UHD Graphics (0x00009A60) Direct3D11 vs_5_0 ps_5_0,
  D3D11)".
- *Its numbers.* `MAX_TEXTURE_SIZE`, `MAX_CUBE_MAP_TEXTURE_SIZE` and
  `MAX_RENDERBUFFER_SIZE` 16384; `MAX_VERTEX_ATTRIBS` 16;
  `MAX_VERTEX_UNIFORM_VECTORS` 4096, `MAX_FRAGMENT_UNIFORM_VECTORS`
  1024, `MAX_VARYING_VECTORS` 30; texture units 16 a stage, 32 in all;
  `MAX_VIEWPORT_DIMS` 32767; points 1 to 1024, lines 1 to 1; 8 bits a
  channel, a 24-bit depth buffer, no stencil, 4 samples; WebGL 2's
  `MAX_3D_TEXTURE_SIZE` and `MAX_ARRAY_TEXTURE_LAYERS` 2048, 8 draw
  buffers, 24 uniform buffer bindings of 65536 bytes each, 12 blocks a
  stage, an offset alignment of 256, `MAX_ELEMENT_INDEX` 4294967294.
  Its extensions: 35 for WebGL 1 (from `ANGLE_instanced_arrays` to
  `WEBGL_polygon_mode`), 32 for WebGL 2.
- *Its errors.* `getError()` is 0, then `INVALID_ENUM` (1280) after
  `enable(0x1234)`, `INVALID_VALUE` (1281) after a negative viewport;
  reading it clears it. A shader that does not compile says
  `"ERROR: 0:1: 'undefinedThing' : undeclared identifier\n"`; a
  fragment shader of WebGL 1 with no default precision compiles when
  it declares no float of its own (`gl_FragColor = vec4(1.0)`); a
  program with a shader that did not compile does not link, saying `"Fragment shader is not compiled.\n"`. A draw that reads past
  its buffer draws, with no error, as robust access lets it.
- *Its pixels.* A canvas of 64 by 32 cleared blue with a red triangle
  over its lower left: `readPixels` from the bottom left gives red
  (255, 0, 0, 255), from the top right blue; pixels read outside the
  drawing buffer are left as they were. Drawn into a 2D canvas after
  the frame, its bottom left is red and its top left, on the
  triangle's antialiased edge, (128, 0, 128); `toDataURL()` then gives
  an empty image, the drawing buffer having been presented and cleared.
  `loseContext()` makes `isContextLost()` true, `getError()`
  `CONTEXT_LOST_WEBGL` (37442) once, `getParameter` `null`, and fires
  `webglcontextlost`; `restoreContext()` fires `webglcontextrestored`.
- *WebGPU.* `navigator.gpu` is a `GPU` on a secure context (here
  `127.0.0.1`); its preferred canvas format is `bgra8unorm`;
  `requestAdapter()` gives an adapter whose `info` is vendor "intel",
  architecture "gen-12lp", an empty device and description, not a
  fallback adapter; its features 19 (`texture-compression-bc`,
  `timestamp-query`, `shader-f16`, ...); its limits the device's own
  (a 2D texture of 16384), but a device asked for nothing has the
  standard's defaults (8192, 4 bind groups, 256 MiB buffers) and only
  `core-features-and-limits`. A second `requestDevice()` of an adapter
  rejects `OperationError` (the adapter is "consumed"); a limit past
  the adapter's rejects "Required limit (1000) is greater than the
  supported limit (4)."; an unknown feature is a `TypeError`.
- *WebGPU's errors.* A shader module with an unknown name compiles to a
  `getCompilationInfo()` of one error at line 1, column 27, offset 26,
  length 14, "unresolved value 'undefinedThing'"; a buffer of usage 0
  is a `GPUValidationError` in its error scope, "Buffer usages must not
  be 0." with Dawn's context lines; outside a scope, an
  `uncapturederror` event; `popErrorScope()` with none rejects
  `OperationError` "No error scopes to pop". A compute pass doubling
  four numbers reads back 2, 4, 6, 9 through `mapAsync`;
  `getMappedRange()` after `unmap()` throws `OperationError`.
- *WebGPU's canvas.* `getContext("webgpu")` is a `GPUCanvasContext`;
  configured, `getConfiguration()` gives the format, `alphaMode`, the
  `srgb` colour space, standard tone mapping, usage 16 (render
  attachment); `getCurrentTexture()` is the canvas's size and the same
  object within a task, a new one the next frame; its drawing shows in
  the canvas after the frame (green, a red triangle at the bottom
  left). `device.destroy()` resolves `lost` with reason `"destroyed"`
  and "Device was destroyed.".

## 1. Decisions

### The GPU process (D38)

**One process drives the GPU.** The browser starts a *GPU process*, the
same executable with `--gpu`, at the first request for a WebGL or
WebGPU context and keeps it while one lives. It alone opens the
graphics device: it runs `wgpu` (Direct3D 12 on Windows, Vulkan,
Metal), as the browser's own backend does, and nothing else of the
engine; it reads no page, no URL, no profile. On Windows it runs under
the renderers' restricted token if the device opens under it, else one
step weaker (a restricted token without low integrity), as the
reference's GPU process runs with more than its renderers; its job
allows no second process and caps its memory at `MAX_GPU_MEMORY`. Step
1 finds which token the device accepts and records it in section 4.
Elsewhere it is a plain process (G113, as the renderers). In one
process, the GPU service is a thread of the browser, on the same
protocol, as the renderers' are in tests.

**Renderers reach it through the browser.** A renderer sends its GPU
requests to the browser (`ToBrowser::Gpu`), which forwards each to the
GPU process with the renderer's number (`ToGpu::Client`); answers come
back the same way (`FromGpu::Client`, `ToRenderer::Gpu`). The browser
reads none of it but the number, which it, not the renderer, stamps;
each renderer is a *client* of the GPU process with its own objects,
which no other client can name. A direct channel, and shared memory
for large uploads, are later (G-list): a relayed message costs a copy
and is bounded by `MAX_MESSAGE`.

**What it answers to.** A client's message the GPU process cannot read,
or one naming an object the client does not have, ends that client:
its objects are dropped and the browser is told, which kills the
renderer, as it does for a malformed message of its own. A panic in
`wgpu` is caught at the message and ends the client too; a device the
driver loses ends its contexts (WebGL's `webglcontextlost`, WebGPU's
`lost`), and a GPU process that dies loses every client's, the
browser starting another at the next request, up to three times in
five minutes, after which WebGL and WebGPU are off for the session
(`getContext` `null`, `requestAdapter()` `null`), as the reference
gives up on its GPU process.

**The adapter.** The GPU process asks for a high-performance or
low-power adapter as a page asks, and when there is none, the
platform's fallback adapter (WARP on Windows), which `info` then
names; with neither, WebGL and WebGPU are off. A setting (and the
tests) can ask for the fallback adapter alone, so that pixels are the
same on every machine that has it.

### The client protocol

**Objects by numbers the renderer gives.** A client names each object
it makes (an adapter, a device, a buffer, a texture, a view, a
sampler, a shader module, a layout, a bind group, a pipeline, a
command buffer, a query set, a render bundle, a canvas's swap chain)
by a number it chooses, unique among its live objects, as Firefox's
content process assigns `wgpu` ids. The GPU process keeps, for each
client, a table from those numbers to `wgpu`'s objects; a number in
use, or one not there, ends the client. Dropping an object is a
message; so is a garbage-collected one (the realm's finalization).

**Calls batched, answers awaited only where the API waits.** The
renderer queues its calls and sends them as one message at the end of
the task, or before any call that needs an answer: WebGPU's promises
(`requestAdapter`, `requestDevice`, `mapAsync`, `popErrorScope`,
`getCompilationInfo`, `onSubmittedWorkDone`) take their answer as a
later task; WebGL's synchronous reads (`readPixels`,
`getBufferSubData`, `finish`, a query's result) wait for it, blocking
the script as the reference's do. A render or compute pass is recorded
in the renderer and sent whole at `end()`, replayed into `wgpu`'s pass
in the GPU process.

**Errors where WebGPU puts them.** Validation is `wgpu`'s, under an
error scope the GPU process pushes around each call whose error the
page may catch: the page's own scopes are kept by the GPU process per
device, as a stack, and an error outside them is an `uncapturederror`
event, its message `wgpu`'s words. Device loss is told as it happens.

### WebGPU (D39)

**The standard's API over `wgpu`'s safe interface.** The bootstrap gives
`navigator.gpu` in secure contexts (windows; workers later, G-list),
`GPU`, `GPUAdapter` with `GPUAdapterInfo`, `GPUSupportedFeatures` and
`GPUSupportedLimits`, `GPUDevice` and its queue, buffers with their
mapping, textures and views, samplers, shader modules and their
compilation info, bind group layouts, pipeline layouts, bind groups,
render and compute pipelines (with `layout: "auto"` and
`getBindGroupLayout`), command encoders, render and compute passes,
render bundles, query sets (occlusion; timestamps later), the error
classes and scopes, `GPUCanvasContext`, `GPUColorWrite`,
`GPUBufferUsage`, `GPUTextureUsage`, `GPUShaderStage`, `GPUMapMode`;
each argument checked by WebIDL as the reference words it. The GPU
process calls `wgpu`'s safe API only, never `wgpu-core` directly,
never a passthrough shader or a native handle; its shader modules
keep `wgpu`'s runtime checks (bounds checks, loop bounding), which the
standard's untrusted code needs.

**What a page is told of the machine.** The adapter's features are
those `wgpu` gives that WebGPU names; its limits the adapter's, no more
than the reference's tier (a 2D texture of 16384, 2 GiB buffers); a
device asked for nothing gets the standard's defaults, as the
reference's. `info` gives the vendor (from the adapter's PCI vendor:
`intel`, `nvidia`, `amd`, `apple`, `qualcomm`, `arm`, else empty) and
nothing more: architecture, device and description empty, as the
reference may, so that a page learns little of the machine.
`isFallbackAdapter` is true on the fallback adapter.

**Mapping across processes.** `mapAsync` maps the buffer in the GPU
process, which sends its bytes for a read; for a write, the renderer
keeps the bytes the page writes into, sent back at `unmap()`; a buffer
made `mappedAtCreation` starts with zeroed bytes in the renderer.
`getMappedRange` hands out `ArrayBuffer`s that `unmap()` detaches.
`queue.writeBuffer` and `writeTexture` carry their bytes in the
message.

**WGSL.** Shader modules are `wgpu`'s, through naga's WGSL front end.
`getCompilationInfo()` gives naga's messages with their lines,
columns, offsets and lengths; the words are naga's, not the
reference's (Dawn's): WGSL's errors are not the standard's to word.

### The canvas, presented

**A canvas's pixels come back to the renderer.** A `GPUCanvasContext`,
configured, has a swap chain in the GPU process: a texture of the
canvas's size and format, `getCurrentTexture()` the same within a task
and a new one after the frame. At the end of a task that drew into it,
the renderer asks for its pixels: the GPU process copies the texture
to a buffer, maps it, and sends the rows; the renderer gives them to
its canvas as a 2D context's bitmap is given (`Canvases`, the
`Replaced::Canvas` box, paint's canvas image), converted to RGBA,
premultiplied as `alphaMode` says. WebGL's drawing buffer is the same:
read back at the end of the task that drew, then cleared unless
`preserveDrawingBuffer`. `drawImage()`, `toDataURL()`, `toBlob()` and
`createImageBitmap()` of the canvas read those pixels; a frame of a
page that does not draw sends nothing. The renderer's page and the
browser's compositor stay in software (G91); a page's own frames on
the GPU are later.

**Images into textures.** `copyExternalImageToTexture` (WebGPU) and
`texImage2D` and its kin from a source (WebGL) take an `ImageBitmap`,
an image, a canvas, a video's current picture or `ImageData` by its
pixels in the renderer, held to the canvas image source rules: a
source that is not origin-clean is refused (WebGL's `SecurityError`),
as a canvas would be tainted.

### WebGL over WebGPU (D40)

**In the renderer, on the client protocol.** `WebGLRenderingContext`
and `WebGL2RenderingContext` are the bootstrap's objects over an
engine-side context (`engine::webgl`) in the renderer that keeps
WebGL's state and turns its draws into WebGPU calls on the same client
protocol: buffers as buffers, textures as textures (with their sampler
state), framebuffers and renderbuffers as textures and views, the
drawing buffer as a texture with its depth and a multisampled texture
when `antialias` (4 samples), programs as shader modules, and every
draw as a render pass segment with a pipeline for the state it was
drawn in, cached by that state. The GPU process sees WebGPU alone and
validates it as it validates a page's own: what WebGL's translation
gets wrong cannot reach the driver past `wgpu`'s checks.

**GLSL ES read by a front end of ours.** A new crate, `glsl`
(`#![forbid(unsafe_code)]`, a leaf below `engine`), reads the OpenGL
ES Shading Language 1.00 and 3.00 as WebGL restricts them: the
preprocessor, the grammar, the types and qualifiers, the built-in
variables and functions, Appendix A's limits for 1.00 (loops of
constant bounds, indexing by constant-index expressions), WebGL's own
rules (no `#extension` it does not offer, identifiers not starting
`webgl_` or `_webgl_`, no more than 256 characters, a shader of at most
`MAX_SHADER_SOURCE` bytes, nesting at most `MAX_GLSL_DEPTH`), its
errors in the reference's form (`ERROR: 0:<line>: '<token>' :
<what>`). It lowers a program to naga's IR, which naga validates, and
writes WGSL with naga's writer; the GPU process reads that WGSL as it
reads a page's. naga's own GLSL front end reads Vulkan's GLSL 4.40 and
later only, not WebGL's (no `attribute`, no `varying`, no
`gl_FragColor`, every uniform bound by a layout), and its preprocessor
is a crate this build does not have.

**GL's model on WebGPU's.** What the translation does, each the
standard's behaviour:

- *Uniforms.* A program's default-block uniforms are packed into one
  uniform buffer a stage (std140's rules, as WebGL 2's blocks are),
  written when a draw finds them changed; samplers become a texture
  and a sampler binding each, from the texture unit's texture and its
  sampler state; WebGL 2's uniform blocks are uniform buffer bindings.
- *Attributes and varyings.* Attributes are vertex inputs at the
  locations the program was given (`bindAttribLocation`, else in
  order), from the bound buffers' layouts, a disabled attribute's
  constant value from a small buffer of its own; varyings are
  inter-stage locations matched by name at link, as GLSL ES requires.
- *Coordinates.* The vertex stage's position is turned to WebGPU's
  (depth from [-1, 1] to [0, 1], the y axis flipped), and rendering
  happens upside down, so that a texture's first row is GL's bottom,
  as GL keeps it: `readPixels`, `texImage2D`, framebuffers and
  `gl_FragCoord` need no flip; the drawing buffer is flipped once, when
  it is read back for the page. `frontFace` is swapped with it.
- *Points.* WebGPU draws points a pixel wide; a draw of `POINTS` whose
  vertex shader writes `gl_PointSize` is drawn as instanced quads of
  that size, the program translated a second time for it, with
  `gl_PointCoord`. Lines are a pixel wide, as the reference's range
  [1, 1] allows.
- *Formats.* `LUMINANCE`, `ALPHA`, `LUMINANCE_ALPHA` and `RGB` are kept
  as RGBA textures, expanded on upload; packed types
  (`UNSIGNED_SHORT_5_6_5` and its kin) unpacked; `UNPACK_FLIP_Y_WEBGL`,
  `UNPACK_PREMULTIPLY_ALPHA_WEBGL` and `UNPACK_COLORSPACE_CONVERSION`
  applied in the renderer; WebGL 2's sized formats mapped to WebGPU's,
  those it lacks (`RGB9_E5` renderable, `SRGB8` without alpha) widened.
- *State.* Blending, depth and stencil tests and writes, culling,
  polygon offset, colour masks, scissor and viewport map to a
  pipeline's state or a pass's dynamic state; a clear is a pass's
  load, or a draw of a quad when scissored or masked.
- *Robustness.* Draws are checked against their buffers' sizes in the
  renderer as WebGL 1 requires (`INVALID_OPERATION` for an index
  past a buffer), and WebGPU's robust access stands behind them;
  every buffer and texture starts zeroed, as both standards say.
- *Reading back.* `readPixels` copies the rectangle to a buffer in the
  GPU process and waits for it; pixels outside the framebuffer are left
  as they were.

**Errors and state in the renderer.** `getError`, `getParameter`, the
`is*` and `get*` queries, `getShaderInfoLog`, `getProgramInfoLog` and
the compile and link status are answered from the renderer's own
state, with no round trip. `getParameter`'s identity strings are the
reference's (`"WebKit"`, `"WebKit WebGL"`, `"WebGL 1.0 (OpenGL ES 2.0
Chromium)"`); its numbers what the device's limits allow, against the
reference's above. `WEBGL_debug_renderer_info` is not offered: it
would name the machine's GPU.

**Extensions.** WebGL 1: `ANGLE_instanced_arrays`, `EXT_blend_minmax`,
`EXT_color_buffer_half_float`, `EXT_float_blend`, `EXT_frag_depth`,
`EXT_shader_texture_lod`, `EXT_sRGB`, `EXT_texture_filter_anisotropic`,
`OES_element_index_uint`, `OES_fbo_render_mipmap`,
`OES_standard_derivatives`, `OES_texture_float`,
`OES_texture_float_linear`, `OES_texture_half_float`,
`OES_texture_half_float_linear`, `OES_vertex_array_object`,
`WEBGL_color_buffer_float`, `WEBGL_compressed_texture_s3tc` (where the
adapter has BC), `WEBGL_depth_texture`, `WEBGL_draw_buffers`,
`WEBGL_lose_context`. WebGL 2: `EXT_color_buffer_float`,
`EXT_color_buffer_half_float`, `EXT_float_blend`,
`EXT_texture_filter_anisotropic`, `OES_texture_float_linear`,
`WEBGL_compressed_texture_s3tc`, `WEBGL_lose_context`. The others the
reference offers are later (G-list).

**WebGL 2.** Vertex array objects, uniform buffers, 3D and array
textures, integer and sized formats, multiple render targets,
instancing, sampler objects, occlusion queries, sync objects (fences
the GPU process signals), `blitFramebuffer`, `texStorage`,
`invalidateFramebuffer`, `getBufferSubData`, `copyBufferSubData`,
GLSL ES 3.00. Transform feedback is not here: WebGPU's vertex stage
writes no buffer, and it wants a compute pass of the vertex shader
(G-list); `beginTransformFeedback` raises `INVALID_OPERATION`.

**Context loss and limits.** A renderer has at most
`MAX_WEBGL_CONTEXTS` live WebGL contexts (16, the reference's); a new
one past them loses the oldest, as the reference does. Lost by
`loseContext()`, the device, or the GPU process: the context's objects
are gone, `webglcontextlost` fires, and, when the page prevents its
default, `webglcontextrestored` follows once the GPU process can make
another (or after `restoreContext()`).

### Workers and `OffscreenCanvas`

`OffscreenCanvas` in a window takes `"webgl"`, `"webgl2"` and
`"webgpu"` as its element's canvas does; `transferToImageBitmap()`
takes its pixels. In workers, `navigator.gpu` and an offscreen canvas's
WebGL and WebGPU contexts are later (G-list): a worker's realm reaches
the GPU process through its document's renderer, a second client
there.

### DevTools

The console says what the reference's says for WebGL's errors ("WebGL:
INVALID_ENUM: enable: invalid capability"), at most 32 a context, as
the reference stops at 32; a shader's info log on a failed compile.
WebGPU's uncaptured errors go to the console as the reference's do. A
GPU panel is not there (G-list).

### Bounds

`MAX_GPU_MEMORY` (the GPU process's job, 4 GiB), `MAX_GPU_CLIENT_BYTES`
(the bytes of buffers and textures a client holds, 2 GiB, past which
creation fails as out of memory), `MAX_GPU_OBJECTS` (a client's live
objects, 1,000,000), `MAX_GPU_PENDING` (a client's messages queued in
the browser, past which the renderer waits), `MAX_WEBGL_CONTEXTS`
(16), `MAX_SHADER_SOURCE` (a shader's bytes, 1 MiB), `MAX_GLSL_DEPTH`
(nesting of blocks and expressions, 64), `MAX_GLSL_TOKENS` (a shader's
tokens after preprocessing), `MAX_PIPELINES` (a WebGL context's cached
pipelines, past which the oldest are dropped), the device limits a
WebGPU device is given, `MAX_MESSAGE` for any one call's bytes, and
`MAX_GPU_RESTARTS` (3 in five minutes).

### Fuzzing

`glsl_parse` (any text as a GLSL ES 1.00 or 3.00 shader of either
stage: never a panic, within the bounds, and every shader it accepts
translated to WGSL that naga validates), `gpu_wire` (any bytes as the
client protocol's messages and the GPU process's answers, each decoded
one encoded again the same), `webgl_ops` (any sequence of WebGL calls
against a context on a GPU service with the fallback or a no-op
adapter, never a panic, `getError` as a model of its rules says), and
`webgpu_ops` (any sequence of WebGPU calls through the client: no panic
in the GPU service, every error a WebGPU error). `ipc_decode`'s seeds
gain the new messages.

### The acceptance (M41.6)

Pages drawing with WebGL 1 and 2 (a lit, textured cube; points of a
size; framebuffers rendered to and read) and with WebGPU (a rendered
triangle, a compute pass read back) give the reference's pixels to its
tolerance, through a renderer and in one process; the probes' lines
above met. A subset of the Khronos WebGL conformance suite
(`conformance` and `conformance2`, the tests the engine can run) and
of the WebGPU conformance test suite (its `api`, `shader` and
`web_platform` queries the engine can run), with expectations, and
WPT's `webgl` directory. Benches: a WebGL scene's frame, a WebGPU
compute dispatch's round trip, a WebGL draw call's cost.

## 2. Not in this milestone, said rather than hidden

- *A direct channel and shared memory* between renderers and the GPU
  process: requests are relayed by the browser, their bytes copied.
- *The page's own frames on the GPU* through the GPU process, and the
  compositor there (G91): renderers draw in software.
- *Workers*: `navigator.gpu` in workers, WebGL and WebGPU on an
  `OffscreenCanvas` in a worker.
- *Transform feedback* (WebGL 2), and WebGPU's timestamp queries.
- *Extensions* past those named: compressed formats other than S3TC
  (ETC, ASTC, PVRTC, RGTC, BPTC), `EXT_disjoint_timer_query`,
  `WEBGL_multi_draw`, `OVR_multiview2`, `KHR_parallel_shader_compile`,
  `WEBGL_debug_renderer_info` and `WEBGL_debug_shaders`.
- *WebXR*, `xrCompatible` (always false).
- *A GPU panel* in DevTools.
- *Wide lines*, which the reference does not draw either.
- *WebGPU's optional features* past those `wgpu` gives on the adapter,
  and its HDR canvas configurations (`toneMapping` `"extended"`).

## 3. Order of work

1. *The GPU process*: its launch, sandbox and restarts, the browser's
   relay, the thread in one process, the adapter and its fallback, the
   client tables, the protocol's encoding; `gpu_wire`.
2. *WebGPU's objects*: `navigator.gpu`, adapters, devices and queues,
   buffers and their mapping, error scopes, uncaptured errors, device
   loss.
3. *WebGPU's work*: shader modules and compilation info, textures,
   views, samplers, layouts, bind groups, pipelines, command encoders,
   passes recorded and replayed, copies, render bundles, queries;
   `webgpu_ops`.
4. *The canvas*: `GPUCanvasContext`, swap chains, the pixels read back
   to the page, `copyExternalImageToTexture`, `OffscreenCanvas`.
5. *The `glsl` crate*: the preprocessor, the grammar, the types and
   the rules of 1.00 and 3.00, WebGL's limits, the errors; lowering to
   naga's IR and WGSL, the points variant; `glsl_parse`.
6. *WebGL's core*: contexts and attributes, buffers, attributes,
   programs, uniforms, draws, clears, the state, the pipeline cache,
   the drawing buffer presented, `getParameter` and `getError`.
7. *WebGL's textures and framebuffers*: uploads from arrays and
   sources with the unpack flags, the formats, mipmaps, cube maps,
   framebuffers and renderbuffers, `readPixels`, `copyTexImage2D`,
   WebGL 1's extensions.
8. *WebGL 2*: vertex array objects, uniform buffers, 3D and array
   textures, integer formats, draw buffers, instancing, samplers,
   queries, syncs, `blitFramebuffer`, GLSL ES 3.00's features; context
   loss and its limits; `webgl_ops`.
9. *The acceptance*: the pages, the conformance suites' subsets, WPT's
   `webgl`, the benches.
10. *T4.3 and fuzzing.*
11. *Documents.*

## 4. Corrected to the code

What the work found that this note did not say, or said otherwise, by
step:

- **Before the work.** naga 30's GLSL front end reads Vulkan GLSL 4.40
  and later only, and needs pp-rs: the glsl crate is the engine's own
  (step 5).
- **The GPU process (step 1).** There is no `MAX_GPU_PENDING`: the relay
  writes to the GPU process's pipe on the renderer's reader thread, so a
  full pipe holds that renderer, as wanted. The GPU process runs under
  the renderers' own restricted, low-integrity token on Windows, and a
  device opens under it (step 2): no weaker token is needed. wgpu's
  `apply_limit_buckets` is on for every adapter, never the page's
  choice.
- **WebGPU's objects (step 2).** The adapter's limits are wgpu's
  buckets, not Dawn's (`maxBindGroups` 8 and
  `minUniformBufferOffsetAlignment` 64 here where the reference says 4
  and 256): tests hold them to the standard's bounds. A
  `GPUValidationError`'s message is wgpu's words. Objects a page lets go
  are dropped when their document goes, not when collected.
- **WebGPU's work (step 3).** Validation wgpu cannot see is the
  bootstrap's, sent as `Request::Invalid` and reported as wgpu's are; a
  scope's error from wgpu is answered before the client's when both
  came. Where the standard makes an object invalid that the bootstrap
  found wrong, the object is made of what is valid and the error
  reported. Render bundles are not wgpu's (its bundle encoder panics at
  a bundle's invalid commands, which a page can cause): a bundle is its
  commands, kept by the service and replayed into the pass that executes
  it, at most `MAX_PASS_COMMANDS` (4M) a pass; a pass keeps a bundle's
  pipeline and bindings after it, where the standard resets them.
  wgpu panics at a buffer range past the buffer: the service checks
  every slice first. A device dropped with scopes open pops them
  innermost first (wgpu panics at the other order; `webgpu_ops` found
  it). A texture write past 4 MiB goes in pieces of whole rows.
- **The canvas (step 4).** The frame shows a task or so after the frame
  that presented it (its rows come back from the GPU process).
  `copyExternalImageToTexture` writes rgba8unorm and bgra8unorm (and
  their srgb) only.
- **The glsl crate (step 5).** The crate writes WGSL itself and naga
  reads and validates it at the link, rather than building naga's IR.
  Uniforms of both stages are laid out as std140 lays a block out, so
  one scheme serves the default block and the page's blocks.
  `gl_MaxVaryingVectors` is 15 (WebGPU's 16 inter-stage locations less
  one for `gl_PointCoord`; the reference: 30). An expression of more
  than `MAX_GLSL_OPERATORS` (512) binary operators is refused
  "Expression too complex", as the reference's translator (step 9:
  longer ones overflowed the stack). GLSL's geometric functions of
  scalars are written out (WGSL has them of vectors); an integer cube
  sampler reads a 2D array of its faces, the face and texel chosen in
  the shader (GL ES 3.0 table 3.21), as WebGPU loads no texel of a cube
  (step 9).
- **WebGL's core (step 6).** The clear and blend colours are clamped to
  [0, 1] in both versions. `VENDOR` and `RENDERER` are the masked
  "WebKit" and "WebKit WebGL"; past `MAX_WEBGL_CONTEXTS` (16) the
  oldest context is lost. Another context's object is INVALID_OPERATION
  and the call is not made, but an `is*()` of it is false with no error
  (step 9).
- **Textures and framebuffers (steps 7, 8).** Each GL image is a WebGPU
  texture of its own, a mipmapped texture or a cube put together into
  one texture by copies when sampled. Textures WebGPU cannot filter are
  bound as "unfilterable-float", a layout variant a pipeline's key.
  `copyTexImage2D`, `copyTexSubImage2D`, `generateMipmap` and
  `blitFramebuffer` are draws, as WebGPU has no converting copy, mipmap
  generation or blit; a multisampled renderbuffer always resolves into a
  texture beside it, which a blit reads. `renderbufferStorageMultisample`
  gives 4 samples for any of 1 to 4. A blit's rectangle past an int32 is
  INVALID_VALUE (step 9). An image source's sub-rectangle is taken at
  `UNPACK_SKIP_PIXELS`, `UNPACK_SKIP_ROWS` and `UNPACK_IMAGE_HEIGHT`
  (step 9; step 8 took the whole source).
- **WebGL 2's objects (step 8).** Step 8 was two commits. A buffer bound
  first as an element array buffer is one for good, any other never one;
  the copy targets take either and make neither (step 9). Occlusion
  queries count each draw in a slot of a 4096-slot set; results and
  fences are seen once control returns to the event loop.
  `clientWaitSync` never waits. Transform feedback's objects exist and
  beginning it is INVALID_OPERATION. A draw with an active draw buffer
  the program writes nothing to, and a write mask not all false, is
  INVALID_OPERATION (step 9).
- **The acceptance (step 9).** The pages' lines are Chrome's to 3 a
  number (its clear of 0.1 is 26, the engine's 25), in one process and
  through a renderer and the GPU process (`--check-processes webgl`). A
  context's release sends its drops at once: a context lost as its
  document went left its device in the GPU process, and the Khronos
  suite ran out of devices. The Khronos subset is 241 tests of 41
  directories (`tests/khronos`, 143 expected failures); the WebGPU CTS
  subset is 36 queries, its modules one classic script with a `URL` of
  its own, as the engine has neither module scripts (G41) nor `URL`
  (G68); both subsets' expectations hold on Windows, where they were
  made, and elsewhere every test must end. The CTS found: `configure()`
  throws TypeError for a format not the canvas's, and a zero size, an
  invalid usage, bgra8unorm storage without its feature and incompatible
  view formats are validation errors, the canvas configured, its texture
  invalid and never presented; `createBuffer` mapped at creation of a
  size not of 4 throws RangeError; an invalid buffer's unmap is no
  error; `mapAsync` is validated on the device's timeline (a mapped,
  destroyed or invalid buffer, a mode, offset or size not its own: a
  validation error and a rejection), a pending map alone rejecting at
  once; `MAX_ERROR_SCOPES` is 2^20 (1024 ended the client; the CTS
  pushes 100,000); a device's and a pass's labels come from their
  descriptors. The standard's per-stage storage limits
  (`maxStorageBuffersInVertexStage` and the like) are not given: wgpu
  has none. The gate's test step limit is 900 s, the two subsets adding
  about three minutes.
- **T4.3 and fuzzing (step 10).** A realm made cost a third more than
  before M41: WebGPU's and WebGL's bindings, 4,200 lines of the
  bootstrap, made and read by every realm. They are a script of their
  own (`js/src/quickjs/bootstrap_gpu.js`), compiled to bytecode apart
  and read into a realm when its page first reaches them (a name of
  theirs, `navigator.gpu`, a canvas's context, a GPU task), a function
  of the bootstrap's names they use; their interface objects are
  accessors until then. A realm is back within 3% of M40's. CI's
  Windows runners have no adapter, so WPT's `webgl` is left out there;
  off Windows the Khronos subset fails only on a panic or a page that
  does not load. The fuzzers found an empty uniform block (refused, as
  GLSL's grammar has it) and `wgpu-core` adding a vertex attribute's
  offset unchecked (the service holds the offset where it cannot
  overflow).
