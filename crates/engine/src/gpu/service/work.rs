//! WebGPU's work in the GPU service (M41 step 3, D39): shader modules,
//! textures and their views, samplers, layouts, bind groups, pipelines,
//! command buffers replayed from what an encoder recorded, render
//! bundles, query sets, and the queue's submissions and texture writes.

use std::cell::Cell;
use std::sync::Arc;

use devtools::Json;

use super::super::convert::{self, Bad, member};
use super::super::wire::{Command, CompilationMessage, ComputeCommand, RenderCommand, Reply};
use super::{Client, GpuService, Object};

/// A buffer's range, if `buffer` holds it: `wgpu` panics at one it does
/// not, where WebGPU would report an error; the command is left out.
fn within(buffer: &wgpu::Buffer, offset: u64, size: Option<u64>) -> Option<(u64, u64)> {
    let whole = buffer.size();
    if offset > whole {
        return None;
    }
    let size = size.unwrap_or(whole - offset);
    let end = offset.checked_add(size)?;
    (end <= whole && size > 0).then_some((offset, end))
}

macro_rules! getters {
    ($($name:ident: $variant:ident => $ty:ty, $what:literal;)*) => {
        impl Client {
            $(
                pub(super) fn $name(&self, id: u32) -> Result<$ty, String> {
                    match self.objects.get(&id) {
                        Some(Object::$variant(object)) => Ok(object.clone()),
                        Some(other) => Err(format!(
                            "object {id} named as {}, which is {}",
                            $what,
                            other.kind()
                        )),
                        None => Err(format!("object {id} named, which it does not have")),
                    }
                }
            )*
        }
    };
}

getters! {
    module: ShaderModule => wgpu::ShaderModule, "a shader module";
    texture: Texture => wgpu::Texture, "a texture";
    view: View => wgpu::TextureView, "a texture view";
    sampler: Sampler => wgpu::Sampler, "a sampler";
    bind_group_layout: BindGroupLayout => wgpu::BindGroupLayout, "a bind group layout";
    pipeline_layout: PipelineLayout => wgpu::PipelineLayout, "a pipeline layout";
    bind_group: BindGroup => wgpu::BindGroup, "a bind group";
    compute_pipeline: ComputePipeline => wgpu::ComputePipeline, "a compute pipeline";
    render_pipeline: RenderPipeline => wgpu::RenderPipeline, "a render pipeline";
    query_set: QuerySet => wgpu::QuerySet, "a query set";
    render_bundle: RenderBundle => Arc<[RenderCommand]>, "a render bundle";
}

impl Client {
    fn wgpu_buffer(&self, id: u32) -> Result<wgpu::Buffer, String> {
        match self.objects.get(&id) {
            Some(Object::Buffer(buffer)) => Ok(buffer.buffer.clone()),
            Some(other) => Err(format!(
                "object {id} named as a buffer, which is {}",
                other.kind()
            )),
            None => Err(format!("object {id} named, which it does not have")),
        }
    }

    fn wgpu_device(&self, id: u32) -> Result<(wgpu::Device, wgpu::Queue), String> {
        match self.objects.get(&id) {
            Some(Object::Device(device)) => Ok((device.device.clone(), device.queue.clone())),
            Some(other) => Err(format!(
                "object {id} named as a device, which is {}",
                other.kind()
            )),
            None => Err(format!("object {id} named, which it does not have")),
        }
    }
}

fn parse(desc: &str) -> Result<Json, Bad> {
    Json::parse(desc).map_err(|_| "a malformed descriptor".to_owned())
}

/// A stage's constants, as `wgpu` borrows them.
fn constant_refs(constants: &[(String, f64)]) -> Vec<(&str, f64)> {
    constants
        .iter()
        .map(|(name, value)| (name.as_str(), *value))
        .collect()
}

impl GpuService {
    pub(super) fn create_shader_module(
        &mut self,
        client: u32,
        device: u32,
        id: u32,
        code: &str,
    ) -> Result<(), String> {
        let made = self.client(client);
        let (device, _) = made.wgpu_device(device)?;
        // `wgpu`'s checked module: its runtime checks stay on, as
        // untrusted code needs (D39).
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(code.into()),
        });
        made.insert(id, Object::ShaderModule(module))
    }

    pub(super) fn compilation_info(
        &mut self,
        client: u32,
        call: u32,
        module: u32,
        replies: &mut Vec<Reply>,
    ) -> Result<(), String> {
        let module = self.client(client).module(module)?;
        let info = pollster::block_on(module.get_compilation_info());
        let messages = info
            .messages
            .into_iter()
            .map(|message| {
                let (line, position, offset, length) =
                    message.location.map_or((0, 0, 0, 0), |at| {
                        (
                            u64::from(at.line_number),
                            u64::from(at.line_position),
                            u64::from(at.offset),
                            u64::from(at.length),
                        )
                    });
                CompilationMessage {
                    kind: match message.message_type {
                        wgpu::CompilationMessageType::Error => 0,
                        wgpu::CompilationMessageType::Warning => 1,
                        wgpu::CompilationMessageType::Info => 2,
                    },
                    line,
                    position,
                    offset,
                    length,
                    message: message.message,
                }
            })
            .collect();
        replies.push(Reply::Compilation { call, messages });
        Ok(())
    }

    pub(super) fn create_texture(
        &mut self,
        client: u32,
        device: u32,
        id: u32,
        desc: &str,
    ) -> Result<(), String> {
        let desc = parse(desc)?;
        let made = self.client(client);
        let (device, _) = made.wgpu_device(device)?;
        let format = convert::format(convert::text(&desc, "format")?)?;
        let view_formats = convert::list(&desc, "viewFormats")
            .iter()
            .filter_map(Json::as_str)
            .map(convert::format)
            .collect::<Result<Vec<_>, _>>()?;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: convert::size(member(&desc, "size")),
            mip_level_count: convert::whole(&desc, "mipLevelCount", 1),
            sample_count: convert::whole(&desc, "sampleCount", 1),
            dimension: convert::dimension(
                member(&desc, "dimension")
                    .and_then(Json::as_str)
                    .unwrap_or("2d"),
            )?,
            format,
            usage: wgpu::TextureUsages::from_bits_retain(convert::whole(&desc, "usage", 0)),
            view_formats: &view_formats,
        });
        made.insert(id, Object::Texture(texture))
    }

    pub(super) fn create_view(
        &mut self,
        client: u32,
        texture: u32,
        id: u32,
        desc: &str,
    ) -> Result<(), String> {
        let desc = parse(desc)?;
        let made = self.client(client);
        let texture = made.texture(texture)?;
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: None,
            format: member(&desc, "format")
                .and_then(Json::as_str)
                .map(convert::format)
                .transpose()?,
            dimension: member(&desc, "dimension")
                .and_then(Json::as_str)
                .map(convert::view_dimension)
                .transpose()?,
            usage: member(&desc, "usage")
                .and_then(Json::as_f64)
                .map(|usage| wgpu::TextureUsages::from_bits_retain(usage as u32)),
            aspect: convert::aspect(
                member(&desc, "aspect")
                    .and_then(Json::as_str)
                    .unwrap_or("all"),
            )?,
            base_mip_level: convert::whole(&desc, "baseMipLevel", 0),
            mip_level_count: member(&desc, "mipLevelCount")
                .and_then(Json::as_f64)
                .map(|n| n as u32),
            base_array_layer: convert::whole(&desc, "baseArrayLayer", 0),
            array_layer_count: member(&desc, "arrayLayerCount")
                .and_then(Json::as_f64)
                .map(|n| n as u32),
        });
        made.insert(id, Object::View(view))
    }

    pub(super) fn create_sampler(
        &mut self,
        client: u32,
        device: u32,
        id: u32,
        desc: &str,
    ) -> Result<(), String> {
        let desc = parse(desc)?;
        let made = self.client(client);
        let (device, _) = made.wgpu_device(device)?;
        let mode = |name: &str| -> Result<wgpu::AddressMode, Bad> {
            convert::address(
                member(&desc, name)
                    .and_then(Json::as_str)
                    .unwrap_or("clamp-to-edge"),
            )
        };
        let filter = |name: &str| -> Result<wgpu::FilterMode, Bad> {
            convert::filter(
                member(&desc, name)
                    .and_then(Json::as_str)
                    .unwrap_or("nearest"),
            )
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: None,
            address_mode_u: mode("addressModeU")?,
            address_mode_v: mode("addressModeV")?,
            address_mode_w: mode("addressModeW")?,
            mag_filter: filter("magFilter")?,
            min_filter: filter("minFilter")?,
            mipmap_filter: convert::mipmap_filter(
                member(&desc, "mipmapFilter")
                    .and_then(Json::as_str)
                    .unwrap_or("nearest"),
            )?,
            lod_min_clamp: convert::number(&desc, "lodMinClamp", 0.0) as f32,
            lod_max_clamp: convert::number(&desc, "lodMaxClamp", 32.0) as f32,
            compare: member(&desc, "compare")
                .and_then(Json::as_str)
                .map(convert::compare)
                .transpose()?,
            // wgpu refuses more than 16, which WebGPU clamps; 0 is
            // refused by both.
            anisotropy_clamp: convert::whole(&desc, "maxAnisotropy", 1).min(16) as u16,
            border_color: None,
        });
        made.insert(id, Object::Sampler(sampler))
    }

    pub(super) fn create_bind_group_layout(
        &mut self,
        client: u32,
        device: u32,
        id: u32,
        desc: &str,
    ) -> Result<(), String> {
        let desc = parse(desc)?;
        let made = self.client(client);
        let (device, _) = made.wgpu_device(device)?;
        let mut entries = Vec::new();
        for entry in convert::list(&desc, "entries") {
            let ty = if let Some(buffer) = member(entry, "buffer") {
                wgpu::BindingType::Buffer {
                    ty: match member(buffer, "type")
                        .and_then(Json::as_str)
                        .unwrap_or("uniform")
                    {
                        "uniform" => wgpu::BufferBindingType::Uniform,
                        "storage" => wgpu::BufferBindingType::Storage { read_only: false },
                        "read-only-storage" => wgpu::BufferBindingType::Storage { read_only: true },
                        other => return Err(format!("a buffer binding type of '{other}'")),
                    },
                    has_dynamic_offset: convert::flag(buffer, "hasDynamicOffset"),
                    min_binding_size: std::num::NonZeroU64::new(convert::wide(
                        buffer,
                        "minBindingSize",
                        0,
                    )),
                }
            } else if let Some(sampler) = member(entry, "sampler") {
                wgpu::BindingType::Sampler(
                    match member(sampler, "type")
                        .and_then(Json::as_str)
                        .unwrap_or("filtering")
                    {
                        "filtering" => wgpu::SamplerBindingType::Filtering,
                        "non-filtering" => wgpu::SamplerBindingType::NonFiltering,
                        "comparison" => wgpu::SamplerBindingType::Comparison,
                        other => return Err(format!("a sampler binding type of '{other}'")),
                    },
                )
            } else if let Some(texture) = member(entry, "texture") {
                wgpu::BindingType::Texture {
                    sample_type: match member(texture, "sampleType")
                        .and_then(Json::as_str)
                        .unwrap_or("float")
                    {
                        "float" => wgpu::TextureSampleType::Float { filterable: true },
                        "unfilterable-float" => {
                            wgpu::TextureSampleType::Float { filterable: false }
                        }
                        "depth" => wgpu::TextureSampleType::Depth,
                        "sint" => wgpu::TextureSampleType::Sint,
                        "uint" => wgpu::TextureSampleType::Uint,
                        other => return Err(format!("a sample type of '{other}'")),
                    },
                    view_dimension: convert::view_dimension(
                        member(texture, "viewDimension")
                            .and_then(Json::as_str)
                            .unwrap_or("2d"),
                    )?,
                    multisampled: convert::flag(texture, "multisampled"),
                }
            } else if let Some(storage) = member(entry, "storageTexture") {
                wgpu::BindingType::StorageTexture {
                    access: match member(storage, "access")
                        .and_then(Json::as_str)
                        .unwrap_or("write-only")
                    {
                        "write-only" => wgpu::StorageTextureAccess::WriteOnly,
                        "read-only" => wgpu::StorageTextureAccess::ReadOnly,
                        "read-write" => wgpu::StorageTextureAccess::ReadWrite,
                        other => return Err(format!("a storage access of '{other}'")),
                    },
                    format: convert::format(convert::text(storage, "format")?)?,
                    view_dimension: convert::view_dimension(
                        member(storage, "viewDimension")
                            .and_then(Json::as_str)
                            .unwrap_or("2d"),
                    )?,
                }
            } else {
                return Err("a layout entry of no kind".to_owned());
            };
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: convert::whole(entry, "binding", 0),
                visibility: wgpu::ShaderStages::from_bits_retain(convert::whole(
                    entry,
                    "visibility",
                    0,
                )),
                ty,
                count: None,
            });
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &entries,
        });
        made.insert(id, Object::BindGroupLayout(layout))
    }

    pub(super) fn create_pipeline_layout(
        &mut self,
        client: u32,
        device: u32,
        id: u32,
        layouts: &[u32],
    ) -> Result<(), String> {
        let made = self.client(client);
        let (device, _) = made.wgpu_device(device)?;
        // 0 for a null layout, an empty one.
        let layouts = layouts
            .iter()
            .map(|id| match id {
                0 => Ok(None),
                id => made.bind_group_layout(*id).map(Some),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let refs: Vec<Option<&wgpu::BindGroupLayout>> =
            layouts.iter().map(Option::as_ref).collect();
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &refs,
            immediate_size: 0,
        });
        made.insert(id, Object::PipelineLayout(layout))
    }

    pub(super) fn create_bind_group(
        &mut self,
        client: u32,
        device: u32,
        id: u32,
        desc: &str,
    ) -> Result<(), String> {
        let desc = parse(desc)?;
        let made = self.client(client);
        let (device, _) = made.wgpu_device(device)?;
        let layout = made.bind_group_layout(convert::whole(&desc, "layout", 0))?;
        // What the entries name, held while the descriptor borrows them.
        enum Held {
            Buffer(wgpu::Buffer, u64, Option<u64>),
            Sampler(wgpu::Sampler),
            View(wgpu::TextureView),
        }
        let mut held = Vec::new();
        for entry in convert::list(&desc, "entries") {
            let binding = convert::whole(entry, "binding", 0);
            let resource = if let Some(buffer) = member(entry, "buffer").and_then(Json::as_f64) {
                Held::Buffer(
                    made.wgpu_buffer(buffer as u32)?,
                    convert::wide(entry, "offset", 0),
                    member(entry, "size")
                        .and_then(Json::as_f64)
                        .map(|n| n as u64),
                )
            } else if let Some(sampler) = member(entry, "sampler").and_then(Json::as_f64) {
                Held::Sampler(made.sampler(sampler as u32)?)
            } else if let Some(view) = member(entry, "view").and_then(Json::as_f64) {
                Held::View(made.view(view as u32)?)
            } else {
                return Err("a bind group entry of no resource".to_owned());
            };
            held.push((binding, resource));
        }
        let entries: Vec<wgpu::BindGroupEntry> = held
            .iter()
            .map(|(binding, resource)| wgpu::BindGroupEntry {
                binding: *binding,
                resource: match resource {
                    Held::Buffer(buffer, offset, size) => {
                        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer,
                            offset: *offset,
                            size: size.and_then(std::num::NonZeroU64::new),
                        })
                    }
                    Held::Sampler(sampler) => wgpu::BindingResource::Sampler(sampler),
                    Held::View(view) => wgpu::BindingResource::TextureView(view),
                },
            })
            .collect();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &entries,
        });
        made.insert(id, Object::BindGroup(group))
    }

    /// The pipeline layout a descriptor names: `None` for `"auto"`.
    fn layout_of(made: &Client, desc: &Json) -> Result<Option<wgpu::PipelineLayout>, String> {
        match member(desc, "layout") {
            Some(Json::Number(id)) => Ok(Some(made.pipeline_layout(*id as u32)?)),
            _ => Ok(None),
        }
    }

    /// Makes a pipeline with `make`: with `call`, inside an error scope
    /// of its own, whose error is answered rather than reported.
    fn scoped<T>(
        device: &wgpu::Device,
        call: Option<u32>,
        replies: &mut Vec<Reply>,
        make: impl FnOnce() -> T,
    ) -> T {
        let Some(call) = call else {
            return make();
        };
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let made = make();
        let error = pollster::block_on(scope.pop()).map(|error| match error {
            wgpu::Error::Validation { description, .. }
            | wgpu::Error::Internal { description, .. } => description,
            wgpu::Error::OutOfMemory { .. } => "Out of memory".to_owned(),
        });
        replies.push(Reply::PipelineMade { call, error });
        made
    }

    pub(super) fn create_compute_pipeline(
        &mut self,
        client: u32,
        device: u32,
        id: u32,
        desc: &str,
        call: Option<u32>,
        replies: &mut Vec<Reply>,
    ) -> Result<(), String> {
        let desc = parse(desc)?;
        let made = self.client(client);
        let (device, _) = made.wgpu_device(device)?;
        let layout = Self::layout_of(made, &desc)?;
        let compute = member(&desc, "compute").ok_or("no 'compute'")?;
        let module = made.module(convert::whole(compute, "module", 0))?;
        let entry = member(compute, "entryPoint").and_then(Json::as_str);
        let constants = convert::constants(compute);
        let refs = constant_refs(&constants);
        let pipeline = Self::scoped(&device, call, replies, || {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: layout.as_ref(),
                module: &module,
                entry_point: entry,
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &refs,
                    zero_initialize_workgroup_memory: true,
                },
                cache: None,
            })
        });
        made.insert(id, Object::ComputePipeline(pipeline))
    }

    pub(super) fn create_render_pipeline(
        &mut self,
        client: u32,
        device: u32,
        id: u32,
        desc: &str,
        call: Option<u32>,
        replies: &mut Vec<Reply>,
    ) -> Result<(), String> {
        let desc = parse(desc)?;
        let made = self.client(client);
        let (device, _) = made.wgpu_device(device)?;
        let layout = Self::layout_of(made, &desc)?;
        let vertex = member(&desc, "vertex").ok_or("no 'vertex'")?;
        let vertex_module = made.module(convert::whole(vertex, "module", 0))?;
        let vertex_constants = convert::constants(vertex);
        let vertex_refs = constant_refs(&vertex_constants);
        // The vertex buffers' attributes, held while the layouts borrow
        // them.
        let mut attributes = Vec::new();
        let mut shapes = Vec::new();
        for buffer in convert::list(vertex, "buffers") {
            if matches!(buffer, Json::Null) {
                shapes.push(None);
                continue;
            }
            let mut list = Vec::new();
            for attribute in convert::list(buffer, "attributes") {
                list.push(wgpu::VertexAttribute {
                    format: convert::vertex_format(convert::text(attribute, "format")?)?,
                    // wgpu-core adds the offset and the format's size
                    // unchecked: one past any stride is held there, which
                    // it refuses as past the stride (the bootstrap sends
                    // none past 2^53; webgpu_ops did).
                    offset: convert::wide(attribute, "offset", 0).min(u64::from(u32::MAX)),
                    shader_location: convert::whole(attribute, "shaderLocation", 0),
                });
            }
            attributes.push(list);
            shapes.push(Some((
                convert::wide(buffer, "arrayStride", 0),
                match member(buffer, "stepMode")
                    .and_then(Json::as_str)
                    .unwrap_or("vertex")
                {
                    "vertex" => wgpu::VertexStepMode::Vertex,
                    "instance" => wgpu::VertexStepMode::Instance,
                    other => return Err(format!("a step mode of '{other}'")),
                },
            )));
        }
        let mut next = attributes.iter();
        let buffers: Vec<Option<wgpu::VertexBufferLayout>> = shapes
            .iter()
            .map(|shape| {
                shape.map(|(array_stride, step_mode)| wgpu::VertexBufferLayout {
                    array_stride,
                    step_mode,
                    attributes: next.next().map_or(&[], Vec::as_slice),
                })
            })
            .collect();
        let primitive = member(&desc, "primitive")
            .cloned()
            .unwrap_or(Json::Object(Vec::new()));
        let primitive = wgpu::PrimitiveState {
            topology: convert::topology(
                member(&primitive, "topology")
                    .and_then(Json::as_str)
                    .unwrap_or("triangle-list"),
            )?,
            strip_index_format: match member(&primitive, "stripIndexFormat").and_then(Json::as_str)
            {
                Some("uint16") => Some(wgpu::IndexFormat::Uint16),
                Some("uint32") => Some(wgpu::IndexFormat::Uint32),
                Some(other) => return Err(format!("an index format of '{other}'")),
                None => None,
            },
            front_face: match member(&primitive, "frontFace")
                .and_then(Json::as_str)
                .unwrap_or("ccw")
            {
                "ccw" => wgpu::FrontFace::Ccw,
                "cw" => wgpu::FrontFace::Cw,
                other => return Err(format!("a front face of '{other}'")),
            },
            cull_mode: match member(&primitive, "cullMode")
                .and_then(Json::as_str)
                .unwrap_or("none")
            {
                "none" => None,
                "front" => Some(wgpu::Face::Front),
                "back" => Some(wgpu::Face::Back),
                other => return Err(format!("a cull mode of '{other}'")),
            },
            unclipped_depth: convert::flag(&primitive, "unclippedDepth"),
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        };
        let depth_stencil = match member(&desc, "depthStencil") {
            None => None,
            Some(ds) => {
                let face = |name: &str| -> Result<wgpu::StencilFaceState, Bad> {
                    let face = member(ds, name)
                        .cloned()
                        .unwrap_or(Json::Object(Vec::new()));
                    let op = |member_name: &str| {
                        convert::stencil_operation(
                            member(&face, member_name)
                                .and_then(Json::as_str)
                                .unwrap_or("keep"),
                        )
                    };
                    Ok(wgpu::StencilFaceState {
                        compare: convert::compare(
                            member(&face, "compare")
                                .and_then(Json::as_str)
                                .unwrap_or("always"),
                        )?,
                        fail_op: op("failOp")?,
                        depth_fail_op: op("depthFailOp")?,
                        pass_op: op("passOp")?,
                    })
                };
                Some(wgpu::DepthStencilState {
                    format: convert::format(convert::text(ds, "format")?)?,
                    depth_write_enabled: member(ds, "depthWriteEnabled").and_then(Json::as_bool),
                    depth_compare: member(ds, "depthCompare")
                        .and_then(Json::as_str)
                        .map(convert::compare)
                        .transpose()?,
                    stencil: wgpu::StencilState {
                        front: face("stencilFront")?,
                        back: face("stencilBack")?,
                        read_mask: convert::whole(ds, "stencilReadMask", u32::MAX),
                        write_mask: convert::whole(ds, "stencilWriteMask", u32::MAX),
                    },
                    bias: wgpu::DepthBiasState {
                        constant: convert::number(ds, "depthBias", 0.0) as i32,
                        slope_scale: convert::number(ds, "depthBiasSlopeScale", 0.0) as f32,
                        clamp: convert::number(ds, "depthBiasClamp", 0.0) as f32,
                    },
                })
            }
        };
        let multisample = member(&desc, "multisample")
            .cloned()
            .unwrap_or(Json::Object(Vec::new()));
        let multisample = wgpu::MultisampleState {
            count: convert::whole(&multisample, "count", 1),
            mask: convert::wide(&multisample, "mask", u64::from(u32::MAX)),
            alpha_to_coverage_enabled: convert::flag(&multisample, "alphaToCoverageEnabled"),
        };
        let fragment = member(&desc, "fragment");
        let fragment_module = fragment
            .map(|fragment| made.module(convert::whole(fragment, "module", 0)))
            .transpose()?;
        let fragment_constants = fragment.map(convert::constants).unwrap_or_default();
        let fragment_refs = constant_refs(&fragment_constants);
        let mut targets = Vec::new();
        if let Some(fragment) = fragment {
            for target in convert::list(fragment, "targets") {
                if matches!(target, Json::Null) {
                    targets.push(None);
                    continue;
                }
                let component = |blend: &Json, name: &str| -> Result<wgpu::BlendComponent, Bad> {
                    let part = member(blend, name)
                        .cloned()
                        .unwrap_or(Json::Object(Vec::new()));
                    Ok(wgpu::BlendComponent {
                        src_factor: convert::blend_factor(
                            member(&part, "srcFactor")
                                .and_then(Json::as_str)
                                .unwrap_or("one"),
                        )?,
                        dst_factor: convert::blend_factor(
                            member(&part, "dstFactor")
                                .and_then(Json::as_str)
                                .unwrap_or("zero"),
                        )?,
                        operation: convert::blend_operation(
                            member(&part, "operation")
                                .and_then(Json::as_str)
                                .unwrap_or("add"),
                        )?,
                    })
                };
                let blend = match member(target, "blend") {
                    Some(blend) => Some(wgpu::BlendState {
                        color: component(blend, "color")?,
                        alpha: component(blend, "alpha")?,
                    }),
                    None => None,
                };
                targets.push(Some(wgpu::ColorTargetState {
                    format: convert::format(convert::text(target, "format")?)?,
                    blend,
                    write_mask: wgpu::ColorWrites::from_bits_retain(convert::whole(
                        target,
                        "writeMask",
                        0xF,
                    )),
                }));
            }
        }
        let fragment_state = match (&fragment_module, fragment) {
            (Some(module), Some(fragment)) => Some(wgpu::FragmentState {
                module,
                entry_point: member(fragment, "entryPoint").and_then(Json::as_str),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &fragment_refs,
                    zero_initialize_workgroup_memory: true,
                },
                targets: &targets,
            }),
            _ => None,
        };
        let pipeline = Self::scoped(&device, call, replies, || {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: layout.as_ref(),
                vertex: wgpu::VertexState {
                    module: &vertex_module,
                    entry_point: member(vertex, "entryPoint").and_then(Json::as_str),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &vertex_refs,
                        zero_initialize_workgroup_memory: true,
                    },
                    buffers: &buffers,
                },
                primitive,
                depth_stencil,
                multisample,
                fragment: fragment_state,
                multiview_mask: None,
                cache: None,
            })
        });
        made.insert(id, Object::RenderPipeline(pipeline))
    }

    pub(super) fn get_bind_group_layout(
        &mut self,
        client: u32,
        pipeline: u32,
        index: u32,
        id: u32,
    ) -> Result<(), String> {
        let made = self.client(client);
        let layout = match made.objects.get(&pipeline) {
            Some(Object::ComputePipeline(pipeline)) => pipeline.get_bind_group_layout(index),
            Some(Object::RenderPipeline(pipeline)) => pipeline.get_bind_group_layout(index),
            Some(other) => {
                return Err(format!(
                    "object {pipeline} named as a pipeline, which is {}",
                    other.kind()
                ));
            }
            None => return Err(format!("object {pipeline} named, which it does not have")),
        };
        made.insert(id, Object::BindGroupLayout(layout))
    }

    pub(super) fn create_query_set(
        &mut self,
        client: u32,
        device: u32,
        id: u32,
        kind: u8,
        count: u32,
    ) -> Result<(), String> {
        let made = self.client(client);
        let (device, _) = made.wgpu_device(device)?;
        let set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: None,
            ty: match kind {
                0 => wgpu::QueryType::Occlusion,
                _ => wgpu::QueryType::Timestamp,
            },
            count,
        });
        made.insert(id, Object::QuerySet(set))
    }

    pub(super) fn write_texture(
        &mut self,
        client: u32,
        device: u32,
        desc: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let desc = parse(desc)?;
        let made = self.client(client);
        let (_, queue) = made.wgpu_device(device)?;
        let destination = member(&desc, "destination").ok_or("no 'destination'")?;
        let texture = made.texture(convert::whole(destination, "texture", 0))?;
        let layout = member(&desc, "dataLayout")
            .cloned()
            .unwrap_or(Json::Object(Vec::new()));
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: convert::whole(destination, "mipLevel", 0),
                origin: convert::origin(member(destination, "origin")),
                aspect: convert::aspect(
                    member(destination, "aspect")
                        .and_then(Json::as_str)
                        .unwrap_or("all"),
                )?,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: member(&layout, "bytesPerRow")
                    .and_then(Json::as_f64)
                    .map(|n| n as u32),
                rows_per_image: member(&layout, "rowsPerImage")
                    .and_then(Json::as_f64)
                    .map(|n| n as u32),
            },
            convert::size(member(&desc, "size")),
        );
        Ok(())
    }

    /// A command buffer of what an encoder recorded, kept until it is
    /// submitted.
    pub(super) fn finish(
        &mut self,
        client: u32,
        device_id: u32,
        id: u32,
        commands: &[Command],
    ) -> Result<(), String> {
        let made = self.client(client);
        let (device, _) = made.wgpu_device(device_id)?;
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        let mut found = Vec::new();
        for command in commands {
            replay(made, &mut encoder, command, &mut found)?;
        }
        let buffer = encoder.finish();
        made.insert(id, Object::CommandBuffer(Some(buffer)))?;
        for message in found {
            self.invalid(client, device_id, message)?;
        }
        Ok(())
    }

    pub(super) fn submit(
        &mut self,
        client: u32,
        device: u32,
        buffers: &[u32],
    ) -> Result<(), String> {
        let made = self.client(client);
        let (_, queue) = made.wgpu_device(device)?;
        let mut taken = Vec::new();
        for id in buffers {
            match made.objects.get_mut(id) {
                // A command buffer submitted twice is submitted empty, so
                // that wgpu reports it.
                Some(Object::CommandBuffer(buffer)) => {
                    if let Some(buffer) = buffer.take() {
                        taken.push(buffer);
                    }
                }
                Some(other) => {
                    return Err(format!(
                        "object {id} named as a command buffer, which is {}",
                        other.kind()
                    ));
                }
                None => return Err(format!("object {id} named, which it does not have")),
            }
        }
        queue.submit(taken);
        Ok(())
    }

    /// A render bundle: its commands, which a pass executing it replays
    /// (D39). wgpu's own bundles panic where they fail their validation
    /// (RenderBundleEncoder::finish), which a page could make happen; the
    /// standard's checks of a bundle wgpu's pass does not make are the
    /// bootstrap's.
    pub(super) fn create_render_bundle(
        &mut self,
        client: u32,
        id: u32,
        commands: &[RenderCommand],
    ) -> Result<(), String> {
        let made = self.client(client);
        // What it names is checked now, as wgpu's bundle would.
        Held::of(made, commands)?;
        let kept: Vec<RenderCommand> = commands
            .iter()
            .filter(|command| !matches!(command, RenderCommand::ExecuteBundles { .. }))
            .cloned()
            .collect();
        made.insert(id, Object::RenderBundle(kept.into()))
    }
}

/// The objects a pass's commands name, cloned out of the table so that
/// the pass may borrow them.
#[derive(Default)]
struct Held {
    render_pipelines: std::collections::HashMap<u32, wgpu::RenderPipeline>,
    compute_pipelines: std::collections::HashMap<u32, wgpu::ComputePipeline>,
    groups: std::collections::HashMap<u32, wgpu::BindGroup>,
    buffers: std::collections::HashMap<u32, wgpu::Buffer>,
    bundles: std::collections::HashMap<u32, Arc<[RenderCommand]>>,
    /// The commands a pass may yet replay: past them, it is cut short.
    budget: Cell<usize>,
}

/// The most commands a pass replays, its bundles' counted each time they
/// are executed: a bundle executed many times over cannot hold the GPU
/// process for long.
pub const MAX_PASS_COMMANDS: usize = 1 << 22;

impl Held {
    /// What `commands` name, and the commands of the bundles they execute.
    fn of(made: &Client, commands: &[RenderCommand]) -> Result<Held, String> {
        let mut held = Held {
            budget: Cell::new(MAX_PASS_COMMANDS),
            ..Held::default()
        };
        held.take(made, commands)?;
        Ok(held)
    }

    fn take(&mut self, made: &Client, commands: &[RenderCommand]) -> Result<(), String> {
        for command in commands {
            match command {
                RenderCommand::SetPipeline { pipeline } => {
                    self.render_pipelines
                        .insert(*pipeline, made.render_pipeline(*pipeline)?);
                }
                RenderCommand::SetBindGroup {
                    group: Some(group), ..
                } => {
                    self.groups.insert(*group, made.bind_group(*group)?);
                }
                RenderCommand::SetVertexBuffer {
                    buffer: Some(buffer),
                    ..
                }
                | RenderCommand::SetIndexBuffer { buffer, .. }
                | RenderCommand::DrawIndirect { buffer, .. }
                | RenderCommand::DrawIndexedIndirect { buffer, .. } => {
                    self.buffers.insert(*buffer, made.wgpu_buffer(*buffer)?);
                }
                RenderCommand::ExecuteBundles { bundles } => {
                    for bundle in bundles {
                        if !self.bundles.contains_key(bundle) {
                            let commands = made.render_bundle(*bundle)?;
                            // A bundle holds no bundles: one level.
                            self.take(made, &commands)?;
                            self.bundles.insert(*bundle, commands);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Whether the pass was cut short at MAX_PASS_COMMANDS.
    fn cut_short(&self) -> bool {
        self.budget.get() == 0
    }

    fn compute(made: &Client, commands: &[ComputeCommand]) -> Result<Held, String> {
        let mut held = Held::default();
        for command in commands {
            match command {
                ComputeCommand::SetPipeline { pipeline } => {
                    held.compute_pipelines
                        .insert(*pipeline, made.compute_pipeline(*pipeline)?);
                }
                ComputeCommand::SetBindGroup {
                    group: Some(group), ..
                } => {
                    held.groups.insert(*group, made.bind_group(*group)?);
                }
                ComputeCommand::DispatchIndirect { buffer, .. } => {
                    held.buffers.insert(*buffer, made.wgpu_buffer(*buffer)?);
                }
                _ => {}
            }
        }
        Ok(held)
    }

    /// One command into a render pass.
    fn render(&self, pass: &mut wgpu::RenderPass<'_>, command: &RenderCommand) {
        let Some(left) = self.budget.get().checked_sub(1) else {
            return;
        };
        self.budget.set(left);
        match command {
            RenderCommand::SetPipeline { pipeline } => {
                pass.set_pipeline(&self.render_pipelines[pipeline])
            }
            RenderCommand::SetBindGroup {
                index,
                group,
                offsets,
            } => {
                pass.set_bind_group(*index, group.map(|group| &self.groups[&group]), offsets);
            }
            RenderCommand::SetVertexBuffer {
                slot,
                buffer,
                offset,
                size,
            } => match buffer {
                Some(buffer) => {
                    let buffer = &self.buffers[buffer];
                    if let Some((start, end)) = within(buffer, *offset, *size) {
                        pass.set_vertex_buffer(*slot, buffer.slice(start..end));
                    }
                }
                None => pass.set_vertex_buffer(*slot, None),
            },
            RenderCommand::SetIndexBuffer {
                buffer,
                format,
                offset,
                size,
            } => {
                let buffer = &self.buffers[buffer];
                if let Some((start, end)) = within(buffer, *offset, *size) {
                    let format = if *format == 0 {
                        wgpu::IndexFormat::Uint16
                    } else {
                        wgpu::IndexFormat::Uint32
                    };
                    pass.set_index_buffer(buffer.slice(start..end), format);
                }
            }
            RenderCommand::Draw {
                vertices,
                instances,
                first_vertex,
                first_instance,
            } => pass.draw(
                *first_vertex..first_vertex.saturating_add(*vertices),
                *first_instance..first_instance.saturating_add(*instances),
            ),
            RenderCommand::DrawIndexed {
                indices,
                instances,
                first_index,
                base_vertex,
                first_instance,
            } => pass.draw_indexed(
                *first_index..first_index.saturating_add(*indices),
                *base_vertex,
                *first_instance..first_instance.saturating_add(*instances),
            ),
            RenderCommand::DrawIndirect { buffer, offset } => {
                pass.draw_indirect(&self.buffers[buffer], *offset)
            }
            RenderCommand::DrawIndexedIndirect { buffer, offset } => {
                pass.draw_indexed_indirect(&self.buffers[buffer], *offset);
            }
            RenderCommand::SetViewport {
                x,
                y,
                width,
                height,
                min_depth,
                max_depth,
            } => pass.set_viewport(*x, *y, *width, *height, *min_depth, *max_depth),
            RenderCommand::SetScissorRect {
                x,
                y,
                width,
                height,
            } => pass.set_scissor_rect(*x, *y, *width, *height),
            RenderCommand::SetBlendConstant { r, g, b, a } => {
                pass.set_blend_constant(wgpu::Color {
                    r: *r,
                    g: *g,
                    b: *b,
                    a: *a,
                })
            }
            RenderCommand::SetStencilReference { reference } => {
                pass.set_stencil_reference(*reference)
            }
            RenderCommand::BeginOcclusionQuery { index } => pass.begin_occlusion_query(*index),
            RenderCommand::EndOcclusionQuery {} => pass.end_occlusion_query(),
            // The standard resets the pass's state before and after; wgpu's
            // pass keeps it (the bootstrap checks draws have a pipeline).
            RenderCommand::ExecuteBundles { bundles } => {
                for bundle in bundles {
                    for command in self.bundles[bundle].iter() {
                        self.render(pass, command);
                    }
                }
            }
        }
    }
}

/// An encoder's command, replayed.
fn replay(
    made: &Client,
    encoder: &mut wgpu::CommandEncoder,
    command: &Command,
    found: &mut Vec<String>,
) -> Result<(), String> {
    match command {
        Command::CopyBufferToBuffer {
            source,
            source_offset,
            destination,
            destination_offset,
            size,
        } => {
            let (source, destination) =
                (made.wgpu_buffer(*source)?, made.wgpu_buffer(*destination)?);
            encoder.copy_buffer_to_buffer(
                &source,
                *source_offset,
                &destination,
                *destination_offset,
                *size,
            );
        }
        Command::CopyBufferToTexture { desc }
        | Command::CopyTextureToBuffer { desc }
        | Command::CopyTextureToTexture { desc } => {
            let desc = parse(desc)?;
            let size = convert::size(member(&desc, "size"));
            let texel_texture = |copy: &Json| -> Result<
                (wgpu::Texture, u32, wgpu::Origin3d, wgpu::TextureAspect),
                String,
            > {
                Ok((
                    made.texture(convert::whole(copy, "texture", 0))?,
                    convert::whole(copy, "mipLevel", 0),
                    convert::origin(member(copy, "origin")),
                    convert::aspect(
                        member(copy, "aspect")
                            .and_then(Json::as_str)
                            .unwrap_or("all"),
                    )?,
                ))
            };
            let texel_buffer =
                |copy: &Json| -> Result<(wgpu::Buffer, wgpu::TexelCopyBufferLayout), String> {
                    Ok((
                        made.wgpu_buffer(convert::whole(copy, "buffer", 0))?,
                        wgpu::TexelCopyBufferLayout {
                            offset: convert::wide(copy, "offset", 0),
                            bytes_per_row: member(copy, "bytesPerRow")
                                .and_then(Json::as_f64)
                                .map(|n| n as u32),
                            rows_per_image: member(copy, "rowsPerImage")
                                .and_then(Json::as_f64)
                                .map(|n| n as u32),
                        },
                    ))
                };
            let source = member(&desc, "source").ok_or("no 'source'")?;
            let destination = member(&desc, "destination").ok_or("no 'destination'")?;
            match command {
                Command::CopyBufferToTexture { .. } => {
                    let (buffer, layout) = texel_buffer(source)?;
                    let (texture, mip_level, origin, aspect) = texel_texture(destination)?;
                    encoder.copy_buffer_to_texture(
                        wgpu::TexelCopyBufferInfo {
                            buffer: &buffer,
                            layout,
                        },
                        wgpu::TexelCopyTextureInfo {
                            texture: &texture,
                            mip_level,
                            origin,
                            aspect,
                        },
                        size,
                    );
                }
                Command::CopyTextureToBuffer { .. } => {
                    let (texture, mip_level, origin, aspect) = texel_texture(source)?;
                    let (buffer, layout) = texel_buffer(destination)?;
                    encoder.copy_texture_to_buffer(
                        wgpu::TexelCopyTextureInfo {
                            texture: &texture,
                            mip_level,
                            origin,
                            aspect,
                        },
                        wgpu::TexelCopyBufferInfo {
                            buffer: &buffer,
                            layout,
                        },
                        size,
                    );
                }
                _ => {
                    let (from, from_mip, from_origin, from_aspect) = texel_texture(source)?;
                    let (to, to_mip, to_origin, to_aspect) = texel_texture(destination)?;
                    encoder.copy_texture_to_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &from,
                            mip_level: from_mip,
                            origin: from_origin,
                            aspect: from_aspect,
                        },
                        wgpu::TexelCopyTextureInfo {
                            texture: &to,
                            mip_level: to_mip,
                            origin: to_origin,
                            aspect: to_aspect,
                        },
                        size,
                    );
                }
            }
        }
        Command::ClearBuffer {
            buffer,
            offset,
            size,
        } => {
            let buffer = made.wgpu_buffer(*buffer)?;
            encoder.clear_buffer(&buffer, *offset, *size);
        }
        Command::ResolveQuerySet {
            set,
            first,
            count,
            destination,
            offset,
        } => {
            let (set, destination) = (made.query_set(*set)?, made.wgpu_buffer(*destination)?);
            encoder.resolve_query_set(
                &set,
                *first..first.saturating_add(*count),
                &destination,
                *offset,
            );
        }
        Command::RenderPass { desc, commands } => {
            let desc = parse(desc)?;
            let held = Held::of(made, commands)?;
            // The attachments' views, held while the pass borrows them.
            let mut views = Vec::new();
            for attachment in convert::list(&desc, "colorAttachments") {
                if matches!(attachment, Json::Null) {
                    views.push(None);
                    continue;
                }
                let view = made.view(convert::whole(attachment, "view", 0))?;
                let resolve = member(attachment, "resolveTarget")
                    .and_then(Json::as_f64)
                    .map(|id| made.view(id as u32))
                    .transpose()?;
                views.push(Some((view, resolve, attachment.clone())));
            }
            let colors: Vec<Option<wgpu::RenderPassColorAttachment>> = views
                .iter()
                .map(|held| {
                    held.as_ref().map(|(view, resolve, attachment)| {
                        wgpu::RenderPassColorAttachment {
                            view,
                            depth_slice: member(attachment, "depthSlice")
                                .and_then(Json::as_f64)
                                .map(|n| n as u32),
                            resolve_target: resolve.as_ref(),
                            ops: wgpu::Operations {
                                load: match member(attachment, "loadOp").and_then(Json::as_str) {
                                    Some("clear") => wgpu::LoadOp::Clear(convert::color(member(
                                        attachment,
                                        "clearValue",
                                    ))),
                                    _ => wgpu::LoadOp::Load,
                                },
                                store: match member(attachment, "storeOp").and_then(Json::as_str) {
                                    Some("discard") => wgpu::StoreOp::Discard,
                                    _ => wgpu::StoreOp::Store,
                                },
                            },
                        }
                    })
                })
                .collect();
            let depth = member(&desc, "depthStencilAttachment");
            let depth_view = depth
                .map(|depth| made.view(convert::whole(depth, "view", 0)))
                .transpose()?;
            let ops = |depth: &Json, prefix: &str, clear: f64| {
                let load = member(depth, &format!("{prefix}LoadOp")).and_then(Json::as_str);
                let store = member(depth, &format!("{prefix}StoreOp")).and_then(Json::as_str);
                if convert::flag(depth, &format!("{prefix}ReadOnly")) || load.is_none() {
                    return None;
                }
                Some((
                    load == Some("clear"),
                    convert::number(depth, &format!("{prefix}ClearValue"), clear),
                    store != Some("discard"),
                ))
            };
            let depth_attachment = match (&depth_view, depth) {
                (Some(view), Some(depth)) => Some(wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: ops(depth, "depth", 0.0).map(|(clear, value, store)| {
                        wgpu::Operations {
                            load: if clear {
                                wgpu::LoadOp::Clear(value as f32)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: if store {
                                wgpu::StoreOp::Store
                            } else {
                                wgpu::StoreOp::Discard
                            },
                        }
                    }),
                    stencil_ops: ops(depth, "stencil", 0.0).map(|(clear, value, store)| {
                        wgpu::Operations {
                            load: if clear {
                                wgpu::LoadOp::Clear(value as u32)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: if store {
                                wgpu::StoreOp::Store
                            } else {
                                wgpu::StoreOp::Discard
                            },
                        }
                    }),
                }),
                _ => None,
            };
            let occlusion = member(&desc, "occlusionQuerySet")
                .and_then(Json::as_f64)
                .map(|id| made.query_set(id as u32))
                .transpose()?;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &colors,
                depth_stencil_attachment: depth_attachment,
                timestamp_writes: None,
                occlusion_query_set: occlusion.as_ref(),
                multiview_mask: None,
            });
            for command in commands {
                held.render(&mut pass, command);
            }
            if held.cut_short() {
                found.push(format!(
                    "A render pass replays more than {MAX_PASS_COMMANDS} commands."
                ));
            }
        }
        Command::ComputePass { commands } => {
            let held = Held::compute(made, commands)?;
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            for command in commands {
                match command {
                    ComputeCommand::SetPipeline { pipeline } => {
                        pass.set_pipeline(&held.compute_pipelines[pipeline])
                    }
                    ComputeCommand::SetBindGroup {
                        index,
                        group,
                        offsets,
                    } => {
                        pass.set_bind_group(
                            *index,
                            group.map(|group| &held.groups[&group]),
                            offsets,
                        );
                    }
                    ComputeCommand::Dispatch { x, y, z } => pass.dispatch_workgroups(*x, *y, *z),
                    ComputeCommand::DispatchIndirect { buffer, offset } => {
                        pass.dispatch_workgroups_indirect(&held.buffers[buffer], *offset);
                    }
                }
            }
        }
    }
    Ok(())
}
