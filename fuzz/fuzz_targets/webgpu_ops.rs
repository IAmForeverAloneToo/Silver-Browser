//! WebGPU's calls (M41 step 3, D39) driven by arbitrary bytes: any
//! sequence of the requests the bootstrap sends, naming objects of the
//! kinds it names, with any values, into one GPU service on this
//! machine's adapter (none: nothing is checked). No request panics, and
//! none ends the client: what is wrong is `wgpu`'s validation's to say.

#![no_main]
#![forbid(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::mem::ManuallyDrop;

use engine::gpu::convert::{FORMATS, VERTEX_FORMATS};
use engine::gpu::service::GpuService;
use engine::gpu::wire::{self, Command, ComputeCommand, RenderCommand, Reply, Request};
use engine::ipc::Bytes as Data;
use libfuzzer_sys::fuzz_target;

thread_local! {
    /// One service for every input (its error scopes are its thread's),
    /// a client of its own for each. Never dropped: torn down as the
    /// process exits, its devices made the exit fail on Windows.
    static SERVICE: RefCell<Option<ManuallyDrop<GpuService>>> = const { RefCell::new(None) };
    static CLIENT: Cell<u32> = const { Cell::new(1) };
}

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

    /// A number around the sizes and offsets that matter: small ones,
    /// alignments, and some past any buffer.
    fn number(&mut self) -> Option<u64> {
        Some(match self.byte()? {
            b @ 0..=191 => u64::from(b),
            b @ 192..=247 => 4u64 << (b - 192) % 12,
            248 => u64::from(u32::MAX),
            _ => u64::MAX,
        })
    }

    fn pick(&mut self, list: &[u32]) -> Option<u32> {
        if list.is_empty() {
            return None;
        }
        Some(list[self.small(list.len())?])
    }
}

/// The objects made, by kind.
#[derive(Default)]
struct Made {
    next: u32,
    buffers: Vec<u32>,
    textures: Vec<u32>,
    views: Vec<u32>,
    samplers: Vec<u32>,
    modules: Vec<u32>,
    layouts: Vec<u32>,
    pipeline_layouts: Vec<u32>,
    groups: Vec<u32>,
    computes: Vec<u32>,
    renders: Vec<u32>,
    commands: Vec<u32>,
    sets: Vec<u32>,
    bundles: Vec<u32>,
}

impl Made {
    fn id(&mut self) -> u32 {
        self.next += 1;
        self.next
    }
}

const SHADERS: &[&str] = &[
    "@group(0) @binding(0) var<storage, read_write> d: array<u32>;\n\
     @compute @workgroup_size(4) fn main(@builtin(global_invocation_id) i: vec3u) { d[i.x] = d[i.x] + 1u; }",
    "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f { return vec4f(f32(i), 0, 0, 1); }\n\
     @fragment fn fs() -> @location(0) vec4f { return vec4f(1); }",
    "@group(0) @binding(0) var t: texture_2d<f32>; @group(0) @binding(1) var s: sampler;\n\
     @vertex fn vs(@location(0) p: vec2f) -> @builtin(position) vec4f { return vec4f(p, 0, 1); }\n\
     @fragment fn fs() -> @location(0) vec4f { return textureSample(t, s, vec2f(0.5)); }",
    "fn main( {",
    "@compute @workgroup_size(1024, 1024) fn main() {}",
];

const BINDINGS: &[&str] = &[
    r#""buffer":{"type":"uniform"}"#,
    r#""buffer":{"type":"storage","hasDynamicOffset":true}"#,
    r#""buffer":{"type":"read-only-storage","minBindingSize":64}"#,
    r#""sampler":{"type":"filtering"}"#,
    r#""sampler":{"type":"comparison"}"#,
    r#""texture":{"sampleType":"float","viewDimension":"2d"}"#,
    r#""texture":{"sampleType":"depth","viewDimension":"cube","multisampled":true}"#,
    r#""storageTexture":{"access":"read-write","format":"r32float"}"#,
];

const VIEW_DIMENSIONS: &[&str] = &["1d", "2d", "2d-array", "cube", "cube-array", "3d"];
const ASPECTS: &[&str] = &["all", "depth-only", "stencil-only"];
const TOPOLOGIES: &[&str] = &[
    "point-list",
    "line-list",
    "line-strip",
    "triangle-list",
    "triangle-strip",
];

fn render_command(input: &mut Input, made: &Made, bundle: bool) -> Option<RenderCommand> {
    Some(match input.small(if bundle { 8 } else { 15 })? {
        0 => RenderCommand::SetPipeline {
            pipeline: input.pick(&made.renders)?,
        },
        1 => RenderCommand::SetBindGroup {
            index: input.small(6)? as u32,
            group: if input.small(4)? == 0 {
                None
            } else {
                Some(input.pick(&made.groups)?)
            },
            offsets: vec![input.number()? as u32; input.small(3)?],
        },
        2 => RenderCommand::SetVertexBuffer {
            slot: input.small(10)? as u32,
            buffer: Some(input.pick(&made.buffers)?),
            offset: input.number()?,
            size: if input.small(2)? == 0 {
                None
            } else {
                Some(input.number()?)
            },
        },
        3 => RenderCommand::SetIndexBuffer {
            buffer: input.pick(&made.buffers)?,
            format: input.small(2)? as u8,
            offset: input.number()?,
            size: if input.small(2)? == 0 {
                None
            } else {
                Some(input.number()?)
            },
        },
        4 => RenderCommand::Draw {
            vertices: input.number()? as u32,
            instances: input.small(4)? as u32,
            first_vertex: input.number()? as u32,
            first_instance: input.small(4)? as u32,
        },
        5 => RenderCommand::DrawIndexed {
            indices: input.number()? as u32,
            instances: input.small(4)? as u32,
            first_index: input.number()? as u32,
            base_vertex: i32::from(input.byte()? as i8),
            first_instance: input.small(4)? as u32,
        },
        6 => RenderCommand::DrawIndirect {
            buffer: input.pick(&made.buffers)?,
            offset: input.number()?,
        },
        7 => RenderCommand::DrawIndexedIndirect {
            buffer: input.pick(&made.buffers)?,
            offset: input.number()?,
        },
        8 => RenderCommand::SetViewport {
            x: f32::from(input.byte()? as i8),
            y: f32::from(input.byte()? as i8),
            width: f32::from(input.byte()?),
            height: f32::from(input.byte()?),
            min_depth: f32::from(input.byte()?) / 128.0,
            max_depth: f32::from(input.byte()?) / 128.0,
        },
        9 => RenderCommand::SetScissorRect {
            x: input.number()? as u32,
            y: input.number()? as u32,
            width: input.number()? as u32,
            height: input.number()? as u32,
        },
        10 => RenderCommand::SetBlendConstant {
            r: f64::from(input.byte()?) / 64.0,
            g: 0.5,
            b: 0.0,
            a: 1.0,
        },
        11 => RenderCommand::SetStencilReference {
            reference: input.number()? as u32,
        },
        12 => RenderCommand::BeginOcclusionQuery {
            index: input.small(8)? as u32,
        },
        13 => RenderCommand::EndOcclusionQuery {},
        _ => RenderCommand::ExecuteBundles {
            bundles: vec![input.pick(&made.bundles)?; input.small(3)? + 1],
        },
    })
}

fn command(input: &mut Input, made: &Made) -> Option<Command> {
    let copy = |input: &mut Input, made: &Made, buffer_side: bool| -> Option<String> {
        Some(if buffer_side {
            format!(
                r#"{{"buffer":{},"offset":{},"bytesPerRow":{},"rowsPerImage":{}}}"#,
                input.pick(&made.buffers)?,
                input.number()?,
                input.number()?,
                input.number()?
            )
        } else {
            format!(
                r#"{{"texture":{},"mipLevel":{},"origin":[{},{},{}],"aspect":"{}"}}"#,
                input.pick(&made.textures)?,
                input.small(3)?,
                input.number()?,
                input.small(4)?,
                input.small(2)?,
                ASPECTS[input.small(3)?]
            )
        })
    };
    let size = |input: &mut Input| -> Option<String> {
        Some(format!(
            "[{},{},{}]",
            input.small(70)?,
            input.small(70)?,
            input.small(3)?
        ))
    };
    Some(match input.small(8)? {
        0 => Command::CopyBufferToBuffer {
            source: input.pick(&made.buffers)?,
            source_offset: input.number()?,
            destination: input.pick(&made.buffers)?,
            destination_offset: input.number()?,
            size: if input.small(2)? == 0 {
                None
            } else {
                Some(input.number()?)
            },
        },
        1 => Command::CopyBufferToTexture {
            desc: format!(
                r#"{{"source":{},"destination":{},"size":{}}}"#,
                copy(input, made, true)?,
                copy(input, made, false)?,
                size(input)?
            ),
        },
        2 => Command::CopyTextureToBuffer {
            desc: format!(
                r#"{{"source":{},"destination":{},"size":{}}}"#,
                copy(input, made, false)?,
                copy(input, made, true)?,
                size(input)?
            ),
        },
        3 => Command::CopyTextureToTexture {
            desc: format!(
                r#"{{"source":{},"destination":{},"size":{}}}"#,
                copy(input, made, false)?,
                copy(input, made, false)?,
                size(input)?
            ),
        },
        4 => Command::ClearBuffer {
            buffer: input.pick(&made.buffers)?,
            offset: input.number()?,
            size: if input.small(2)? == 0 {
                None
            } else {
                Some(input.number()?)
            },
        },
        5 => Command::ResolveQuerySet {
            set: input.pick(&made.sets)?,
            first: input.small(10)? as u32,
            count: input.small(10)? as u32,
            destination: input.pick(&made.buffers)?,
            offset: input.number()?,
        },
        6 => {
            let mut colors = Vec::new();
            for _ in 0..input.small(3)? {
                colors.push(format!(
                    r#"{{"view":{},"loadOp":"{}","storeOp":"{}","clearValue":[0,1,0,1]}}"#,
                    input.pick(&made.views)?,
                    ["load", "clear"][input.small(2)?],
                    ["store", "discard"][input.small(2)?]
                ));
            }
            let depth = if input.small(2)? == 0 {
                String::new()
            } else {
                format!(
                    r#","depthStencilAttachment":{{"view":{},"depthLoadOp":"clear","depthStoreOp":"store","depthClearValue":1,"stencilReadOnly":{}}}"#,
                    input.pick(&made.views)?,
                    input.small(2)? == 0
                )
            };
            let occlusion = match input.pick(&made.sets) {
                Some(set) if input.small(2)? == 0 => format!(r#","occlusionQuerySet":{set}"#),
                _ => String::new(),
            };
            let mut commands = Vec::new();
            for _ in 0..input.small(8)? {
                commands.push(render_command(input, made, false)?);
            }
            Command::RenderPass {
                desc: format!(
                    r#"{{"colorAttachments":[{}]{depth}{occlusion}}}"#,
                    colors.join(",")
                ),
                commands,
            }
        }
        _ => {
            let mut commands = Vec::new();
            for _ in 0..input.small(6)? {
                commands.push(match input.small(4)? {
                    0 => ComputeCommand::SetPipeline {
                        pipeline: input.pick(&made.computes)?,
                    },
                    1 => ComputeCommand::SetBindGroup {
                        index: input.small(5)? as u32,
                        group: Some(input.pick(&made.groups)?),
                        offsets: Vec::new(),
                    },
                    2 => ComputeCommand::Dispatch {
                        x: input.number()? as u32,
                        y: input.small(3)? as u32,
                        z: 1,
                    },
                    _ => ComputeCommand::DispatchIndirect {
                        buffer: input.pick(&made.buffers)?,
                        offset: input.number()?,
                    },
                });
            }
            Command::ComputePass { commands }
        }
    })
}

/// The next request, and the object it makes; `None` at the input's end
/// or where it names a kind of object not yet made (that request is
/// left out).
fn request(input: &mut Input, made: &mut Made, device: u32, scopes: &mut u32) -> Option<Request> {
    let id = made.next + 1;
    let request = match input.small(22)? {
        0 => {
            made.buffers.push(id);
            Request::CreateBuffer {
                device,
                id,
                size: input.number()? % 4096,
                usage: u32::from(input.byte()?) | (input.small(4)? as u32) << 8,
                mapped: input.small(4)? == 0,
            }
        }
        1 => {
            let (format, _) = FORMATS[input.small(FORMATS.len())?];
            made.textures.push(id);
            Request::CreateTexture {
                device,
                id,
                desc: format!(
                    r#"{{"size":[{},{},{}],"format":"{format}","usage":{},"mipLevelCount":{},"sampleCount":{},"dimension":"{}"}}"#,
                    input.small(70)?,
                    input.small(70)?,
                    input.small(7)?,
                    input.small(32)?,
                    input.small(4)?,
                    [1, 4][input.small(2)?],
                    ["1d", "2d", "3d"][input.small(3)?]
                ),
            }
        }
        2 => {
            let texture = input.pick(&made.textures)?;
            let format = if input.small(3)? == 0 {
                format!(r#","format":"{}""#, FORMATS[input.small(FORMATS.len())?].0)
            } else {
                String::new()
            };
            made.views.push(id);
            Request::CreateView {
                texture,
                id,
                desc: format!(
                    r#"{{"dimension":"{}","aspect":"{}","baseMipLevel":{},"baseArrayLayer":{}{format}}}"#,
                    VIEW_DIMENSIONS[input.small(6)?],
                    ASPECTS[input.small(3)?],
                    input.small(3)?,
                    input.small(3)?
                ),
            }
        }
        3 => {
            made.samplers.push(id);
            Request::CreateSampler {
                device,
                id,
                desc: format!(
                    r#"{{"magFilter":"{}","minFilter":"linear","mipmapFilter":"linear","maxAnisotropy":{},"lodMinClamp":{},"lodMaxClamp":{}{}}}"#,
                    ["nearest", "linear"][input.small(2)?],
                    input.small(40)?,
                    input.small(40)?,
                    input.small(40)?,
                    if input.small(2)? == 0 {
                        r#","compare":"less""#
                    } else {
                        ""
                    }
                ),
            }
        }
        4 => {
            made.modules.push(id);
            Request::CreateShaderModule {
                device,
                id,
                code: SHADERS[input.small(SHADERS.len())?].to_owned(),
            }
        }
        5 => {
            let mut entries = Vec::new();
            for _ in 0..input.small(4)? {
                entries.push(format!(
                    r#"{{"binding":{},"visibility":{},{}}}"#,
                    input.small(4)?,
                    input.small(8)?,
                    BINDINGS[input.small(BINDINGS.len())?]
                ));
            }
            made.layouts.push(id);
            Request::CreateBindGroupLayout {
                device,
                id,
                desc: format!(r#"{{"entries":[{}]}}"#, entries.join(",")),
            }
        }
        6 => {
            let mut layouts = Vec::new();
            for _ in 0..input.small(5)? {
                layouts.push(if input.small(5)? == 0 {
                    0
                } else {
                    input.pick(&made.layouts)?
                });
            }
            made.pipeline_layouts.push(id);
            Request::CreatePipelineLayout {
                device,
                id,
                layouts,
            }
        }
        7 => {
            let layout = input.pick(&made.layouts)?;
            let mut entries = Vec::new();
            for _ in 0..input.small(4)? {
                let binding = input.small(4)?;
                entries.push(match input.small(3)? {
                    0 => format!(
                        r#"{{"binding":{binding},"buffer":{},"offset":{},"size":{}}}"#,
                        input.pick(&made.buffers)?,
                        input.number()?,
                        input.number()?
                    ),
                    1 => format!(
                        r#"{{"binding":{binding},"sampler":{}}}"#,
                        input.pick(&made.samplers)?
                    ),
                    _ => format!(
                        r#"{{"binding":{binding},"view":{}}}"#,
                        input.pick(&made.views)?
                    ),
                });
            }
            made.groups.push(id);
            Request::CreateBindGroup {
                device,
                id,
                desc: format!(r#"{{"layout":{layout},"entries":[{}]}}"#, entries.join(",")),
            }
        }
        8 | 9 => {
            let module = input.pick(&made.modules)?;
            let layout = match input.pick(&made.pipeline_layouts) {
                Some(layout) if input.small(2)? == 0 => layout.to_string(),
                _ => r#""auto""#.to_owned(),
            };
            let call = if input.small(2)? == 0 { Some(id) } else { None };
            if input.small(2)? == 0 {
                made.computes.push(id);
                Request::CreateComputePipeline {
                    device,
                    id,
                    desc: format!(
                        r#"{{"layout":{layout},"compute":{{"module":{module},"constants":{{"x":{}}}}}}}"#,
                        input.byte()?
                    ),
                    call,
                }
            } else {
                let (vertex_format, _) = VERTEX_FORMATS[input.small(VERTEX_FORMATS.len())?];
                let (format, _) = FORMATS[input.small(FORMATS.len())?];
                made.renders.push(id);
                Request::CreateRenderPipeline {
                    device,
                    id,
                    desc: format!(
                        r#"{{"layout":{layout},"vertex":{{"module":{module},"buffers":[{{"arrayStride":{},"attributes":[{{"format":"{vertex_format}","offset":{},"shaderLocation":{}}}]}},null]}},"primitive":{{"topology":"{}","stripIndexFormat":"uint16"}},"multisample":{{"count":{}}},"fragment":{{"module":{module},"targets":[{{"format":"{format}","blend":{{"color":{{"srcFactor":"src1"}},"alpha":{{}}}}}}]}}{}}}"#,
                        input.number()?,
                        input.number()?,
                        input.small(20)?,
                        TOPOLOGIES[input.small(5)?],
                        [1, 4][input.small(2)?],
                        if input.small(2)? == 0 {
                            r#","depthStencil":{"format":"depth24plus","depthWriteEnabled":true,"depthCompare":"less"}"#
                        } else {
                            ""
                        }
                    ),
                    call,
                }
            }
        }
        10 => {
            let pipeline = match input.small(2)? {
                0 => input.pick(&made.computes)?,
                _ => input.pick(&made.renders)?,
            };
            made.layouts.push(id);
            Request::GetBindGroupLayout {
                pipeline,
                index: input.small(6)? as u32,
                id,
            }
        }
        11 | 12 => {
            let mut commands = Vec::new();
            for _ in 0..input.small(5)? {
                commands.push(command(input, made)?);
            }
            made.commands.push(id);
            Request::Finish {
                device,
                id,
                commands,
            }
        }
        13 => {
            let mut buffers = Vec::new();
            for _ in 0..input.small(3)? + 1 {
                buffers.push(input.pick(&made.commands)?);
            }
            Request::Submit { device, buffers }
        }
        14 => Request::WriteBuffer {
            device,
            buffer: input.pick(&made.buffers)?,
            offset: input.number()?,
            bytes: Data(vec![7; input.small(64)?]),
        },
        15 => Request::WriteTexture {
            device,
            desc: format!(
                r#"{{"destination":{{"texture":{},"mipLevel":{},"origin":[{},0,0]}},"dataLayout":{{"bytesPerRow":{},"rowsPerImage":{}}},"size":[{},{},1]}}"#,
                input.pick(&made.textures)?,
                input.small(2)?,
                input.small(8)?,
                input.number()?,
                input.small(8)?,
                input.small(70)?,
                input.small(70)?
            ),
            bytes: Data(vec![1; input.small(256)? * 16]),
        },
        16 => {
            let buffer = input.pick(&made.buffers)?;
            let map = Request::MapBuffer {
                call: id,
                buffer,
                mode: input.small(2)? as u8 + 1,
                offset: input.number()?,
                size: input.number()?,
            };
            made.next = id;
            return Some(if input.small(2)? == 0 {
                map
            } else {
                Request::Unmap { buffer }
            });
        }
        17 => {
            made.sets.push(id);
            Request::CreateQuerySet {
                device,
                id,
                kind: input.small(2)? as u8,
                count: input.number()? as u32 % 5000,
            }
        }
        18 => {
            let mut commands = Vec::new();
            for _ in 0..input.small(6)? {
                commands.push(render_command(input, made, true)?);
            }
            made.bundles.push(id);
            Request::CreateRenderBundle {
                device,
                id,
                desc: format!(
                    r#"{{"colorFormats":["{}"],"sampleCount":{}}}"#,
                    FORMATS[input.small(FORMATS.len())?].0,
                    [1, 4][input.small(2)?]
                ),
                commands,
            }
        }
        19 => {
            // Within the service's MAX_ERROR_SCOPES.
            if *scopes >= 1000 || (*scopes > 0 && input.small(2)? == 0) {
                *scopes -= 1;
                made.next = id;
                return Some(Request::PopErrorScope { call: id, device });
            }
            *scopes += 1;
            Request::PushErrorScope {
                device,
                filter: input.small(3)? as u8,
            }
        }
        20 => match input.small(3)? {
            0 => Request::DestroyBuffer {
                id: input.pick(&made.buffers)?,
            },
            1 => Request::DestroyTexture {
                id: input.pick(&made.textures)?,
            },
            _ => Request::DestroyQuerySet {
                id: input.pick(&made.sets)?,
            },
        },
        _ => Request::Invalid {
            device,
            message: "the fuzzer's".to_owned(),
        },
    };
    made.next = id;
    Some(request)
}

fuzz_target!(|data: &[u8]| {
    SERVICE.with_borrow_mut(|service| {
        let service = service.get_or_insert_with(|| ManuallyDrop::new(GpuService::new(false)));
        let client = CLIENT.replace(CLIENT.get() + 1);
        run(service, client, data);
    });
});

fn run(service: &mut GpuService, client: u32, data: &[u8]) {
    let mut made = Made::default();
    let (adapter, device) = (made.id(), made.id());
    let replies = service
        .handle(
            client,
            &wire::requests(&[Request::RequestAdapter {
                call: 1,
                id: adapter,
                power: 0,
                fallback: false,
            }]),
        )
        .expect("an adapter asked for");
    if matches!(replies[..], [Reply::Adapter { adapter: None, .. }]) {
        return;
    }
    let replies = service
        .handle(
            client,
            &wire::requests(&[Request::RequestDevice {
                call: 2,
                adapter,
                id: device,
                features: Vec::new(),
                limits: Vec::new(),
            }]),
        )
        .expect("a device asked for");
    if !matches!(replies[..], [Reply::Device { result: Ok(_), .. }]) {
        return;
    }
    let mut input = Input(data);
    let mut scopes = 0;
    while !input.0.is_empty() {
        let Some(request) = request(&mut input, &mut made, device, &mut scopes) else {
            continue;
        };
        if let Err(why) = service.handle(client, &wire::requests(&[request.clone()])) {
            panic!("the client was ended: {why} ({request:?})");
        }
    }
    service.gone(client);
}
