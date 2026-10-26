//! A document's WebGPU work (M41 step 3, D39): the calls that make
//! shader modules, textures, samplers, layouts, bind groups, pipelines,
//! query sets and render bundles, and an encoder's commands, which the
//! bootstrap records and the GPU process replays at `finish`.

use std::sync::Arc;

use devtools::Json;

use super::{WebGpu, done, number, rejection, settle, whole};
use crate::gpu::GpuClient;
use crate::gpu::wire::{Command, ComputeCommand, RenderCommand, Reply, Request};
use crate::ipc::Bytes;
use crate::scripts::canvas::GpuFrame;
use crate::scripts::workers::{Inbox, WorkerMessage};

fn optional(op: &Json, name: &str) -> Option<u64> {
    op.get(name).and_then(Json::as_f64).map(|n| n as u64)
}

fn float(op: &Json, name: &str) -> f64 {
    op.get(name).and_then(Json::as_f64).unwrap_or(0.0)
}

fn list(op: &Json, name: &str) -> Vec<u32> {
    op.get(name)
        .and_then(Json::as_array)
        .map(|list| {
            list.iter()
                .map(|n| n.as_f64().map_or(0, |n| n as u32))
                .collect()
        })
        .unwrap_or_default()
}

/// A descriptor member, as the GPU process reads it.
fn desc(op: &Json) -> String {
    op.get("desc").map_or_else(|| "{}".to_owned(), Json::write)
}

fn commands(op: &Json) -> &[Json] {
    op.get("commands").and_then(Json::as_array).unwrap_or(&[])
}

/// An encoder's command, as the bootstrap recorded it.
fn command(c: &Json) -> Option<Command> {
    Some(match c.get("c")?.as_str()? {
        "copyBufferToBuffer" => Command::CopyBufferToBuffer {
            source: whole(c, "source"),
            source_offset: number(c, "sourceOffset"),
            destination: whole(c, "destination"),
            destination_offset: number(c, "destinationOffset"),
            size: optional(c, "size"),
        },
        "copyBufferToTexture" => Command::CopyBufferToTexture { desc: desc(c) },
        "copyTextureToBuffer" => Command::CopyTextureToBuffer { desc: desc(c) },
        "copyTextureToTexture" => Command::CopyTextureToTexture { desc: desc(c) },
        "clearBuffer" => Command::ClearBuffer {
            buffer: whole(c, "buffer"),
            offset: number(c, "offset"),
            size: optional(c, "size"),
        },
        "resolveQuerySet" => Command::ResolveQuerySet {
            set: whole(c, "set"),
            first: whole(c, "first"),
            count: whole(c, "count"),
            destination: whole(c, "destination"),
            offset: number(c, "offset"),
        },
        "renderPass" => Command::RenderPass {
            desc: desc(c),
            commands: commands(c).iter().filter_map(render_command).collect(),
        },
        "computePass" => Command::ComputePass {
            commands: commands(c).iter().filter_map(compute_command).collect(),
        },
        _ => return None,
    })
}

fn group(c: &Json) -> Option<u32> {
    c.get("group").and_then(Json::as_f64).map(|n| n as u32)
}

/// A render pass's or a render bundle's command.
fn render_command(c: &Json) -> Option<RenderCommand> {
    Some(match c.get("c")?.as_str()? {
        "setPipeline" => RenderCommand::SetPipeline {
            pipeline: whole(c, "pipeline"),
        },
        "setBindGroup" => RenderCommand::SetBindGroup {
            index: whole(c, "index"),
            group: group(c),
            offsets: list(c, "offsets"),
        },
        "setVertexBuffer" => RenderCommand::SetVertexBuffer {
            slot: whole(c, "slot"),
            buffer: c.get("buffer").and_then(Json::as_f64).map(|n| n as u32),
            offset: number(c, "offset"),
            size: optional(c, "size"),
        },
        "setIndexBuffer" => RenderCommand::SetIndexBuffer {
            buffer: whole(c, "buffer"),
            format: number(c, "format") as u8,
            offset: number(c, "offset"),
            size: optional(c, "size"),
        },
        "draw" => RenderCommand::Draw {
            vertices: whole(c, "vertices"),
            instances: whole(c, "instances"),
            first_vertex: whole(c, "firstVertex"),
            first_instance: whole(c, "firstInstance"),
        },
        "drawIndexed" => RenderCommand::DrawIndexed {
            indices: whole(c, "indices"),
            instances: whole(c, "instances"),
            first_index: whole(c, "firstIndex"),
            base_vertex: float(c, "baseVertex") as i32,
            first_instance: whole(c, "firstInstance"),
        },
        "drawIndirect" => RenderCommand::DrawIndirect {
            buffer: whole(c, "buffer"),
            offset: number(c, "offset"),
        },
        "drawIndexedIndirect" => RenderCommand::DrawIndexedIndirect {
            buffer: whole(c, "buffer"),
            offset: number(c, "offset"),
        },
        "setViewport" => RenderCommand::SetViewport {
            x: float(c, "x") as f32,
            y: float(c, "y") as f32,
            width: float(c, "width") as f32,
            height: float(c, "height") as f32,
            min_depth: float(c, "minDepth") as f32,
            max_depth: float(c, "maxDepth") as f32,
        },
        "setScissorRect" => RenderCommand::SetScissorRect {
            x: whole(c, "x"),
            y: whole(c, "y"),
            width: whole(c, "width"),
            height: whole(c, "height"),
        },
        "setBlendConstant" => RenderCommand::SetBlendConstant {
            r: float(c, "r"),
            g: float(c, "g"),
            b: float(c, "b"),
            a: float(c, "a"),
        },
        "setStencilReference" => RenderCommand::SetStencilReference {
            reference: whole(c, "reference"),
        },
        "beginOcclusionQuery" => RenderCommand::BeginOcclusionQuery {
            index: whole(c, "index"),
        },
        "endOcclusionQuery" => RenderCommand::EndOcclusionQuery {},
        "executeBundles" => RenderCommand::ExecuteBundles {
            bundles: list(c, "bundles"),
        },
        _ => return None,
    })
}

fn compute_command(c: &Json) -> Option<ComputeCommand> {
    Some(match c.get("c")?.as_str()? {
        "setPipeline" => ComputeCommand::SetPipeline {
            pipeline: whole(c, "pipeline"),
        },
        "setBindGroup" => ComputeCommand::SetBindGroup {
            index: whole(c, "index"),
            group: group(c),
            offsets: list(c, "offsets"),
        },
        "dispatch" => ComputeCommand::Dispatch {
            x: whole(c, "x"),
            y: whole(c, "y"),
            z: whole(c, "z"),
        },
        "dispatchIndirect" => ComputeCommand::DispatchIndirect {
            buffer: whole(c, "buffer"),
            offset: number(c, "offset"),
        },
        _ => return None,
    })
}

impl WebGpu {
    /// An object made by `make` of a new number, which the answer gives.
    fn made(&mut self, client: &GpuClient, make: impl FnOnce(u32) -> Request) -> (String, Vec<u8>) {
        let id = client.id();
        self.made.insert(id);
        client.queue([make(id)]);
        done(Json::object([("id", Json::from(id))]))
    }

    /// The calls of step 3, or `None` for a call that is none of them.
    pub(super) fn work(
        &mut self,
        what: &str,
        op: &Json,
        bytes: &[u8],
        client: &Arc<GpuClient>,
        inbox: &Inbox,
    ) -> Option<(String, Vec<u8>)> {
        let device = whole(op, "device");
        let promise = number(op, "promise");
        Some(match what {
            "createShaderModule" => {
                let code = op
                    .get("code")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_owned();
                self.made(client, |id| Request::CreateShaderModule {
                    device,
                    id,
                    code,
                })
            }
            "compilationInfo" => {
                let module = whole(op, "module");
                let inbox = inbox.clone();
                client.call(
                    |call| Request::CompilationInfo { call, module },
                    move |reply, _| {
                        let messages = match reply {
                            Reply::Compilation { messages, .. } => messages
                                .iter()
                                .map(|message| {
                                    Json::object([
                                        ("kind", Json::from(u64::from(message.kind))),
                                        ("line", Json::from(message.line)),
                                        ("offset", Json::from(message.offset)),
                                        ("length", Json::from(message.length)),
                                        ("message", Json::from(message.message.as_str())),
                                    ])
                                })
                                .collect(),
                            _ => Vec::new(),
                        };
                        settle(
                            &inbox,
                            promise,
                            vec![("compilation", Json::Array(messages))],
                            Vec::new(),
                        );
                    },
                );
                done(Json::Null)
            }
            "createTexture" => {
                let desc = desc(op);
                self.made(client, |id| Request::CreateTexture { device, id, desc })
            }
            "createView" => {
                let (texture, desc) = (whole(op, "texture"), desc(op));
                self.made(client, |id| Request::CreateView { texture, id, desc })
            }
            "destroyTexture" => {
                client.queue([Request::DestroyTexture {
                    id: whole(op, "texture"),
                }]);
                done(Json::Null)
            }
            "createSampler" => {
                let desc = desc(op);
                self.made(client, |id| Request::CreateSampler { device, id, desc })
            }
            "createBindGroupLayout" => {
                let desc = desc(op);
                self.made(client, |id| Request::CreateBindGroupLayout {
                    device,
                    id,
                    desc,
                })
            }
            "createPipelineLayout" => {
                let layouts = list(op, "layouts");
                self.made(client, |id| Request::CreatePipelineLayout {
                    device,
                    id,
                    layouts,
                })
            }
            "createBindGroup" => {
                let desc = desc(op);
                self.made(client, |id| Request::CreateBindGroup { device, id, desc })
            }
            "createComputePipeline" | "createRenderPipeline" => {
                let desc = desc(op);
                let compute = what == "createComputePipeline";
                let id = client.id();
                self.made.insert(id);
                let make = move |call| {
                    if compute {
                        Request::CreateComputePipeline {
                            device,
                            id,
                            desc,
                            call,
                        }
                    } else {
                        Request::CreateRenderPipeline {
                            device,
                            id,
                            desc,
                            call,
                        }
                    }
                };
                if promise == 0 {
                    client.queue([make(None)]);
                } else {
                    let inbox = inbox.clone();
                    client.call(
                        |call| make(Some(call)),
                        move |reply, _| {
                            let members = match reply {
                                Reply::PipelineMade { error: None, .. } => {
                                    vec![("pipeline", Json::Bool(true))]
                                }
                                Reply::PipelineMade {
                                    error: Some(why), ..
                                } => rejection("GPUPipelineError", &why),
                                _ => rejection("OperationError", "the GPU is lost"),
                            };
                            settle(&inbox, promise, members, Vec::new());
                        },
                    );
                }
                done(Json::object([("id", Json::from(id))]))
            }
            "getBindGroupLayout" => {
                let (pipeline, index) = (whole(op, "pipeline"), whole(op, "index"));
                self.made(client, |id| Request::GetBindGroupLayout {
                    pipeline,
                    index,
                    id,
                })
            }
            "finish" => {
                let commands = commands(op).iter().filter_map(command).collect();
                self.made(client, |id| Request::Finish {
                    device,
                    id,
                    commands,
                })
            }
            "submit" => {
                client.queue([Request::Submit {
                    device,
                    buffers: list(op, "buffers"),
                }]);
                done(Json::Null)
            }
            "writeTexture" => {
                client.queue([Request::WriteTexture {
                    device,
                    desc: desc(op),
                    bytes: Bytes(bytes.to_vec()),
                }]);
                done(Json::Null)
            }
            "createQuerySet" => {
                let (kind, count) = (number(op, "kind") as u8, whole(op, "count"));
                self.made(client, |id| Request::CreateQuerySet {
                    device,
                    id,
                    kind,
                    count,
                })
            }
            "destroyQuerySet" => {
                client.queue([Request::DestroyQuerySet {
                    id: whole(op, "set"),
                }]);
                done(Json::Null)
            }
            "createRenderBundle" => {
                let (desc, commands) = (
                    desc(op),
                    commands(op).iter().filter_map(render_command).collect(),
                );
                self.made(client, |id| Request::CreateRenderBundle {
                    device,
                    id,
                    desc,
                    commands,
                })
            }
            // A canvas's frame (step 4): its texture copied to the
            // canvas's readback buffer, mapped, and the rows sent to the
            // document's canvas as its bitmap.
            "present" => {
                let handle = op.get("canvas").and_then(Json::as_f64).unwrap_or(0.0);
                let (texture, width, height) = (
                    whole(op, "texture"),
                    whole(op, "width"),
                    whole(op, "height"),
                );
                let format = number(op, "format") as u8;
                let texel = if format == 2 { 8 } else { 4 };
                let stride = (u64::from(width) * texel).div_ceil(256) * 256;
                let size = stride * u64::from(height);
                if size == 0 {
                    return Some(done(Json::Null));
                }
                let key = handle as i64;
                let buffer = match self.readback.get(&key).copied() {
                    Some((id, had)) if had == size => id,
                    old => {
                        if let Some((id, _)) = old {
                            self.made.remove(&id);
                            client.queue([Request::Drop { id }]);
                        }
                        let id = client.id();
                        self.made.insert(id);
                        // MAP_READ | COPY_DST.
                        client.queue([Request::CreateBuffer {
                            device,
                            id,
                            size,
                            usage: 9,
                            mapped: false,
                        }]);
                        self.readback.insert(key, (id, size));
                        id
                    }
                };
                let copy = Json::object([
                    ("source", Json::object([("texture", Json::from(texture))])),
                    (
                        "destination",
                        Json::object([
                            ("buffer", Json::from(buffer)),
                            ("bytesPerRow", Json::from(stride)),
                        ]),
                    ),
                    (
                        "size",
                        Json::Array(vec![
                            Json::from(width),
                            Json::from(height),
                            Json::from(1u32),
                        ]),
                    ),
                ]);
                let commands = client.id();
                client.queue([
                    Request::Finish {
                        device,
                        id: commands,
                        commands: vec![Command::CopyTextureToBuffer { desc: copy.write() }],
                    },
                    Request::Submit {
                        device,
                        buffers: vec![commands],
                    },
                    Request::Drop { id: commands },
                ]);
                let inbox = inbox.clone();
                let premultiplied = op.get("premultiplied").and_then(Json::as_bool) == Some(true);
                let opaque = op.get("opaque").and_then(Json::as_bool) == Some(true);
                client.call(
                    |call| Request::MapBuffer {
                        call,
                        buffer,
                        mode: 1,
                        offset: 0,
                        size,
                    },
                    move |reply, bytes| {
                        if let Reply::Mapped { error: None, .. } = reply {
                            let frame = GpuFrame {
                                width,
                                height,
                                stride: stride as u32,
                                format,
                                premultiplied,
                                opaque,
                                bytes,
                            };
                            let _ = inbox.send(WorkerMessage::GpuFrame(handle, frame));
                        }
                    },
                );
                client.queue([Request::Unmap { buffer }]);
                done(Json::Null)
            }
            "invalid" => {
                client.queue([Request::Invalid {
                    device,
                    message: op
                        .get("message")
                        .and_then(Json::as_str)
                        .unwrap_or("")
                        .to_owned(),
                }]);
                done(Json::Null)
            }
            _ => return None,
        })
    }
}
