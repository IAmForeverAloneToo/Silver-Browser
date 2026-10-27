//! The GPU service (M41 step 1, D38): what the GPU process runs, and a
//! thread of the browser in one process. It holds `wgpu`'s instance
//! and, for each client, a table of the objects the client made, by the
//! numbers it gave them. A batch it cannot read, a number in use or not
//! there or of another kind, or more objects than [`MAX_GPU_OBJECTS`]
//! ends the client. WebGPU's objects (step 2, D39) are `wgpu`'s safe
//! API's: what a page does wrong with them is `wgpu`'s validation's to
//! say, through the device's error scopes or as an uncaptured error.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

mod work;

pub use work::MAX_PASS_COMMANDS;

use super::names;
use super::wire::{
    self, AdapterInfo, DeviceInfo, GPU_CHUNK, GpuError, RenderCommand, Reply, Request,
};
use crate::ipc::Bytes;

/// The most objects a client may have live at once.
pub const MAX_GPU_OBJECTS: usize = 1_000_000;

/// Whether `wgpu` makes a buffer of `usage` and `size`, as WebGPU's
/// createBuffer validates it: a usage of WebGPU's bits, MAP_READ only
/// with COPY_DST, MAP_WRITE only with COPY_SRC, a size in the device's
/// limit and, mapped at creation, a multiple of 4.
fn buffer_valid(usage: u32, size: u64, mapped: bool, max: u64) -> bool {
    const MAP_READ: u32 = 0x1;
    const MAP_WRITE: u32 = 0x2;
    const COPY_SRC: u32 = 0x4;
    const COPY_DST: u32 = 0x8;
    usage != 0
        && usage & !0x3ff == 0
        && (usage & MAP_READ == 0 || usage & !(MAP_READ | COPY_DST) == 0)
        && (usage & MAP_WRITE == 0 || usage & !(MAP_WRITE | COPY_SRC) == 0)
        && size <= max
        && (!mapped || size.is_multiple_of(4))
}

/// The most error scopes a device's stack holds: past it, a push ends
/// the client. The standard has no bound and the WebGPU CTS pushes
/// 100,000; this one bounds a page's memory (tens of bytes a scope).
pub const MAX_ERROR_SCOPES: usize = 1 << 20;

struct Device {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// The page's error scopes, the innermost last.
    scopes: Vec<Scope>,
}

impl Drop for Device {
    /// The page's scopes left open go innermost first: `wgpu` panics at
    /// any other order (webgpu_ops found it).
    fn drop(&mut self) {
        while let Some(scope) = self.scopes.pop() {
            drop(scope);
        }
    }
}

/// An error scope: `wgpu`'s, and the first error the client found while
/// it was the innermost of its filter's.
struct Scope {
    filter: u8,
    guard: wgpu::ErrorScopeGuard,
    found: Option<GpuError>,
}

struct Buffer {
    buffer: wgpu::Buffer,
    /// Its device, which a map waits on, and the device's number.
    device: wgpu::Device,
    device_id: u32,
    /// The range mapped, while it is.
    mapped: Option<(u64, u64)>,
    /// Whether `wgpu` made it: an invalid buffer's mapping at creation is
    /// the page's alone, its writes and unmap no error (WebGPU 5.1.6).
    valid: bool,
}

/// One of a client's objects.
enum Object {
    /// An adapter, and whether a device was asked of it (it is then
    /// "consumed", as WebGPU says).
    Adapter(wgpu::Adapter, bool),
    Device(Device),
    Buffer(Buffer),
    ShaderModule(wgpu::ShaderModule),
    Texture(wgpu::Texture),
    View(wgpu::TextureView),
    Sampler(wgpu::Sampler),
    BindGroupLayout(wgpu::BindGroupLayout),
    PipelineLayout(wgpu::PipelineLayout),
    BindGroup(wgpu::BindGroup),
    ComputePipeline(wgpu::ComputePipeline),
    RenderPipeline(wgpu::RenderPipeline),
    /// A command buffer, until it is submitted.
    CommandBuffer(Option<wgpu::CommandBuffer>),
    QuerySet(wgpu::QuerySet),
    /// A render bundle: its commands (step 3, `work`).
    RenderBundle(std::sync::Arc<[RenderCommand]>),
}

impl Object {
    fn kind(&self) -> &'static str {
        match self {
            Object::Adapter(..) => "an adapter",
            Object::Device(_) => "a device",
            Object::Buffer(_) => "a buffer",
            Object::ShaderModule(_) => "a shader module",
            Object::Texture(_) => "a texture",
            Object::View(_) => "a texture view",
            Object::Sampler(_) => "a sampler",
            Object::BindGroupLayout(_) => "a bind group layout",
            Object::PipelineLayout(_) => "a pipeline layout",
            Object::BindGroup(_) => "a bind group",
            Object::ComputePipeline(_) => "a compute pipeline",
            Object::RenderPipeline(_) => "a render pipeline",
            Object::CommandBuffer(_) => "a command buffer",
            Object::QuerySet(_) => "a query set",
            Object::RenderBundle(_) => "a render bundle",
        }
    }
}

#[derive(Default)]
struct Client {
    objects: HashMap<u32, Object>,
}

impl Client {
    /// Keeps `object` as number `id`, a new number.
    fn insert(&mut self, id: u32, object: Object) -> Result<(), String> {
        if self.objects.len() >= MAX_GPU_OBJECTS {
            return Err(format!("more than {MAX_GPU_OBJECTS} objects"));
        }
        if self.objects.contains_key(&id) {
            return Err(format!("object {id} made again"));
        }
        self.objects.insert(id, object);
        Ok(())
    }

    fn object(&mut self, id: u32) -> Result<&mut Object, String> {
        self.objects
            .get_mut(&id)
            .ok_or_else(|| format!("object {id} named, which it does not have"))
    }

    fn device(&mut self, id: u32) -> Result<&mut Device, String> {
        match self.object(id)? {
            Object::Device(device) => Ok(device),
            other => Err(format!(
                "object {id} named as a device, which is {}",
                other.kind()
            )),
        }
    }

    fn buffer(&mut self, id: u32) -> Result<&mut Buffer, String> {
        match self.object(id)? {
            Object::Buffer(buffer) => Ok(buffer),
            other => Err(format!(
                "object {id} named as a buffer, which is {}",
                other.kind()
            )),
        }
    }
}

/// What the devices' callbacks tell, for the client of each: their
/// uncaptured errors and losses, sent after the batch they came in.
type Events = Arc<Mutex<Vec<(u32, Reply)>>>;

/// The GPU service.
pub struct GpuService {
    instance: wgpu::Instance,
    /// Whether the fallback adapter alone is taken (a setting, and the
    /// tests').
    fallback_only: bool,
    clients: HashMap<u32, Client>,
    events: Events,
}

/// The backends the service tries: Vulkan, Metal and Direct3D 12, or
/// those of them `WGPU_BACKEND` names, as the browser's GPU backend does
/// (CI's Windows job names Vulkan, its runners' software rasterizer
/// crashing a device's process).
fn backends() -> wgpu::Backends {
    let ours = wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::DX12;
    wgpu::Backends::from_env()
        .map(|named| named & ours)
        .filter(|named| !named.is_empty())
        .unwrap_or(ours)
}

/// A `wgpu` error as WebGPU's.
fn error_of(error: wgpu::Error) -> GpuError {
    match error {
        wgpu::Error::Validation { description, .. } => GpuError {
            kind: 0,
            message: description,
        },
        wgpu::Error::OutOfMemory { .. } => GpuError {
            kind: 1,
            message: "Out of memory".to_owned(),
        },
        wgpu::Error::Internal { description, .. } => GpuError {
            kind: 2,
            message: description,
        },
    }
}

impl GpuService {
    pub fn new(fallback_only: bool) -> GpuService {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: backends(),
            flags: wgpu::InstanceFlags::empty(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        GpuService {
            instance,
            fallback_only,
            clients: HashMap::new(),
            events: Arc::default(),
        }
    }

    /// Takes a batch of `client`'s requests: the replies to send it now,
    /// or why the client is ended. What devices told meanwhile is in
    /// [`GpuService::events`].
    pub fn handle(&mut self, client: u32, bytes: &[u8]) -> Result<Vec<Reply>, String> {
        let requests = wire::read_requests(bytes)
            .map_err(|malformed| format!("a malformed batch: {malformed}"))?;
        let mut replies = Vec::new();
        for request in requests {
            self.request(client, request, &mut replies)?;
        }
        Ok(replies)
    }

    /// What the devices told since last asked, by client.
    pub fn events(&self) -> Vec<(u32, Reply)> {
        std::mem::take(&mut *self.events.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn request(
        &mut self,
        client: u32,
        request: Request,
        replies: &mut Vec<Reply>,
    ) -> Result<(), String> {
        match request {
            Request::RequestAdapter {
                call,
                id,
                power,
                fallback,
            } => {
                let adapter = self.adapter(power, fallback);
                let info = adapter.as_ref().map(info_of);
                if let Some(adapter) = adapter {
                    self.client(client)
                        .insert(id, Object::Adapter(adapter, false))?;
                }
                replies.push(Reply::Adapter {
                    call,
                    adapter: info,
                });
            }
            Request::Drop { id } => {
                if self.client(client).objects.remove(&id).is_none() {
                    return Err(format!("object {id} dropped, which it does not have"));
                }
            }
            Request::RequestDevice {
                call,
                adapter,
                id,
                features,
                limits,
            } => {
                let events = self.events.clone();
                let result = self.device(client, adapter, id, &features, &limits, events)?;
                replies.push(Reply::Device { call, result });
            }
            Request::DestroyDevice { id } => {
                let device = self.client(client).device(id)?;
                device.device.destroy();
                let _ = device.device.poll(wgpu::PollType::Poll);
            }
            Request::PushErrorScope { device, filter } => {
                let device = self.client(client).device(device)?;
                if device.scopes.len() >= MAX_ERROR_SCOPES {
                    return Err(format!("more than {MAX_ERROR_SCOPES} error scopes"));
                }
                let filter = match filter {
                    0 => wgpu::ErrorFilter::Validation,
                    1 => wgpu::ErrorFilter::OutOfMemory,
                    _ => wgpu::ErrorFilter::Internal,
                };
                let guard = device.device.push_error_scope(filter);
                device.scopes.push(Scope {
                    filter: u8::from(!matches!(filter, wgpu::ErrorFilter::Validation)),
                    guard,
                    found: None,
                });
            }
            Request::PopErrorScope { call, device } => {
                let device = self.client(client).device(device)?;
                // `wgpu`'s error before the client's: which came first is
                // not known.
                let result = match device.scopes.pop() {
                    Some(scope) => Ok(pollster::block_on(scope.guard.pop())
                        .map(error_of)
                        .or(scope.found)),
                    None => Err("No error scopes to pop".to_owned()),
                };
                replies.push(Reply::ErrorScope { call, result });
            }
            Request::CreateBuffer {
                device,
                id,
                size,
                usage,
                mapped,
            } => {
                let made = self.client(client).device(device)?;
                let buffer = made.device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size,
                    usage: wgpu::BufferUsages::from_bits_retain(usage),
                    mapped_at_creation: mapped,
                });
                let buffer = Buffer {
                    buffer,
                    device: made.device.clone(),
                    device_id: device,
                    mapped: mapped.then_some((0, size)),
                    valid: buffer_valid(usage, size, mapped, made.device.limits().max_buffer_size),
                };
                self.client(client).insert(id, Object::Buffer(buffer))?;
            }
            Request::DestroyBuffer { id } => {
                let buffer = self.client(client).buffer(id)?;
                buffer.buffer.destroy();
                buffer.mapped = None;
            }
            Request::WriteBuffer {
                device,
                buffer,
                offset,
                bytes,
            } => {
                let client = self.client(client);
                let target = client.buffer(buffer)?.buffer.clone();
                client
                    .device(device)?
                    .queue
                    .write_buffer(&target, offset, &bytes.0);
            }
            Request::MapBuffer {
                call,
                buffer,
                mode,
                offset,
                size,
            } => self.map(client, call, buffer, mode, offset, size, replies)?,
            Request::MappedWrite {
                buffer,
                offset,
                bytes,
            } => {
                let buffer = self.client(client).buffer(buffer)?;
                let Some((start, len)) = buffer.mapped else {
                    return Err("a write into a buffer not mapped".to_owned());
                };
                let end = offset.checked_add(bytes.0.len() as u64);
                if offset < start || end.is_none_or(|end| end > start + len) {
                    return Err("a write outside the mapped range".to_owned());
                }
                // A range the buffer cannot give (it was destroyed) takes
                // nothing.
                if !buffer.valid {
                    return Ok(());
                }
                if let Ok(mut view) = buffer
                    .buffer
                    .get_mapped_range_mut(offset..offset + bytes.0.len() as u64)
                {
                    view.copy_from_slice(&bytes.0);
                }
            }
            Request::Unmap { buffer } => {
                let buffer = self.client(client).buffer(buffer)?;
                if buffer.mapped.take().is_some() && buffer.valid {
                    buffer.buffer.unmap();
                }
            }
            Request::WorkDone { call, device } => {
                let device = self.client(client).device(device)?;
                device.queue.on_submitted_work_done(|| {});
                let _ = device.device.poll(wgpu::PollType::wait_indefinitely());
                replies.push(Reply::WorkDone { call });
            }
            Request::CreateShaderModule { device, id, code } => {
                self.create_shader_module(client, device, id, &code)?;
            }
            Request::CompilationInfo { call, module } => {
                self.compilation_info(client, call, module, replies)?;
            }
            Request::CreateTexture { device, id, desc } => {
                self.create_texture(client, device, id, &desc)?;
            }
            Request::CreateView { texture, id, desc } => {
                self.create_view(client, texture, id, &desc)?;
            }
            Request::DestroyTexture { id } => self.client(client).texture(id)?.destroy(),
            Request::CreateSampler { device, id, desc } => {
                self.create_sampler(client, device, id, &desc)?;
            }
            Request::CreateBindGroupLayout { device, id, desc } => {
                self.create_bind_group_layout(client, device, id, &desc)?;
            }
            Request::CreatePipelineLayout {
                device,
                id,
                layouts,
            } => self.create_pipeline_layout(client, device, id, &layouts)?,
            Request::CreateBindGroup { device, id, desc } => {
                self.create_bind_group(client, device, id, &desc)?;
            }
            Request::CreateComputePipeline {
                device,
                id,
                desc,
                call,
            } => self.create_compute_pipeline(client, device, id, &desc, call, replies)?,
            Request::CreateRenderPipeline {
                device,
                id,
                desc,
                call,
            } => self.create_render_pipeline(client, device, id, &desc, call, replies)?,
            Request::GetBindGroupLayout {
                pipeline,
                index,
                id,
            } => self.get_bind_group_layout(client, pipeline, index, id)?,
            Request::Finish {
                device,
                id,
                commands,
            } => self.finish(client, device, id, &commands)?,
            Request::Submit { device, buffers } => self.submit(client, device, &buffers)?,
            Request::WriteTexture {
                device,
                desc,
                bytes,
            } => self.write_texture(client, device, &desc, &bytes.0)?,
            Request::CreateQuerySet {
                device,
                id,
                kind,
                count,
            } => self.create_query_set(client, device, id, kind, count)?,
            Request::DestroyQuerySet { id } => self.client(client).query_set(id)?.destroy(),
            Request::CreateRenderBundle {
                device,
                id,
                commands,
                ..
            } => {
                self.client(client).device(device)?;
                self.create_render_bundle(client, id, &commands)?;
            }
            Request::Invalid { device, message } => self.invalid(client, device, message)?,
        }
        Ok(())
    }

    fn client(&mut self, client: u32) -> &mut Client {
        self.clients.entry(client).or_default()
    }

    /// A validation error on `device`: caught by its innermost validation
    /// scope, else told as uncaptured.
    pub(super) fn invalid(
        &mut self,
        client: u32,
        device: u32,
        message: String,
    ) -> Result<(), String> {
        let error = GpuError { kind: 0, message };
        let made = self.client(client).device(device)?;
        match made.scopes.iter_mut().rev().find(|scope| scope.filter == 0) {
            Some(scope) => {
                scope.found.get_or_insert(error);
            }
            None => self
                .events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((client, Reply::Uncaptured { device, error })),
        }
        Ok(())
    }

    /// Whether it has an adapter for a page that asks for any.
    pub fn has_adapter(&self) -> bool {
        self.adapter(0, false).is_some()
    }

    /// An adapter as a page asks: the machine's, of the power asked for,
    /// else the platform's fallback; the fallback alone when it asks for
    /// it or the service takes no other.
    fn adapter(&self, power: u8, fallback: bool) -> Option<wgpu::Adapter> {
        let power_preference = match power {
            1 => wgpu::PowerPreference::LowPower,
            2 => wgpu::PowerPreference::HighPerformance,
            _ => wgpu::PowerPreference::None,
        };
        let ask = |force_fallback_adapter| {
            pollster::block_on(self.instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference,
                force_fallback_adapter,
                compatible_surface: None,
                // The adapter's limits in wgpu's buckets, so that a page
                // learns less of the machine (D39); never the page's
                // choice.
                apply_limit_buckets: true,
            }))
            .ok()
        };
        if fallback || self.fallback_only {
            return ask(true);
        }
        ask(false).or_else(|| ask(true))
    }

    /// A device of `adapter` with the features and limits asked for, or
    /// why it is refused. The standard's defaults stand for limits not
    /// asked for, and for those asked for worse than the defaults.
    fn device(
        &mut self,
        client: u32,
        adapter: u32,
        id: u32,
        features: &[String],
        limits: &[(String, u64)],
        events: Events,
    ) -> Result<Result<DeviceInfo, String>, String> {
        let made = self.client(client);
        let (adapter, consumed) = match made.object(adapter)? {
            Object::Adapter(adapter, consumed) => (adapter.clone(), consumed),
            other => {
                return Err(format!(
                    "object {adapter} named as an adapter, which is {}",
                    other.kind()
                ));
            }
        };
        if *consumed {
            return Ok(Err(
                "adapter is \"consumed\": it has already been used to create a device".to_owned(),
            ));
        }
        *consumed = true;
        let required_features = match names::features_named(features) {
            Ok(features) => features,
            Err(name) => return Ok(Err(format!("the feature '{name}' is not supported"))),
        };
        let defaults = wgpu::Limits::default();
        let mut required_limits = defaults.clone();
        for (name, value) in limits {
            let mut asked = defaults.clone();
            if !names::set_limit(&mut asked, name, *value) {
                return Ok(Err(format!("the limit '{name}' cannot be {value}")));
            }
            // Asked for worse than the default, the default stands.
            if asked.check_limits(&defaults) {
                continue;
            }
            names::set_limit(&mut required_limits, name, *value);
        }
        let descriptor = wgpu::DeviceDescriptor {
            label: None,
            required_features,
            required_limits,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        };
        let (device, queue) = match pollster::block_on(adapter.request_device(&descriptor)) {
            Ok(made) => made,
            Err(error) => return Ok(Err(error.to_string())),
        };
        let uncaptured = events.clone();
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            uncaptured
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((
                    client,
                    Reply::Uncaptured {
                        device: id,
                        error: error_of(error),
                    },
                ));
        }));
        device.set_device_lost_callback(move |reason, message| {
            events.lock().unwrap_or_else(PoisonError::into_inner).push((
                client,
                Reply::DeviceLost {
                    device: id,
                    destroyed: reason == wgpu::DeviceLostReason::Destroyed,
                    message,
                },
            ));
        });
        let info = DeviceInfo {
            features: names::feature_names(device.features()),
            limits: names::limit_values(&device.limits()),
        };
        self.client(client).insert(
            id,
            Object::Device(Device {
                device,
                queue,
                scopes: Vec::new(),
            }),
        )?;
        Ok(Ok(info))
    }

    /// Maps `buffer`'s range, waiting for the GPU, and sends its bytes.
    #[allow(clippy::too_many_arguments)]
    fn map(
        &mut self,
        client: u32,
        call: u32,
        buffer: u32,
        mode: u8,
        offset: u64,
        size: u64,
        replies: &mut Vec<Reply>,
    ) -> Result<(), String> {
        let made = self.client(client).buffer(buffer)?;
        // A buffer mapped (`wgpu` rejects it with no error), a range past
        // the buffer (`wgpu` panics at it) and a mode not one of read and
        // write: a validation error and a rejection, as WebGPU's
        // mapAsync; `wgpu` says the rest.
        let end = offset
            .checked_add(size)
            .filter(|end| *end <= made.buffer.size());
        let mode = match mode {
            1 => Some(wgpu::MapMode::Read),
            2 => Some(wgpu::MapMode::Write),
            _ => None,
        };
        let (Some(end), Some(mode), None) = (end, mode, made.mapped) else {
            let why = if made.mapped.is_some() {
                "the buffer is mapped"
            } else if end.is_none() {
                "the range is past the buffer's end"
            } else {
                "the mode is not one of read and write"
            };
            let device = made.device_id;
            self.invalid(client, device, format!("mapAsync: {why}"))?;
            replies.push(Reply::Mapped {
                call,
                error: Some(why.to_owned()),
            });
            return Ok(());
        };
        let done = Arc::new(Mutex::new(None));
        let told = done.clone();
        made.buffer.map_async(mode, offset..end, move |result| {
            *told.lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
        });
        let _ = made.device.poll(wgpu::PollType::wait_indefinitely());
        let result = done.lock().unwrap_or_else(PoisonError::into_inner).take();
        match result {
            Some(Ok(())) => {
                made.mapped = Some((offset, size));
                if let Ok(view) = made.buffer.get_mapped_range(offset..end) {
                    for (i, piece) in view.chunks(GPU_CHUNK).enumerate() {
                        replies.push(Reply::MappedBytes {
                            call,
                            offset: (i * GPU_CHUNK) as u64,
                            bytes: Bytes(piece.to_vec()),
                        });
                    }
                }
                replies.push(Reply::Mapped { call, error: None });
            }
            Some(Err(error)) => replies.push(Reply::Mapped {
                call,
                error: Some(error.to_string()),
            }),
            None => replies.push(Reply::Mapped {
                call,
                error: Some("the buffer could not be mapped".to_owned()),
            }),
        }
        Ok(())
    }

    /// Client `client` is gone: its objects go.
    pub fn gone(&mut self, client: u32) {
        self.clients.remove(&client);
    }

    /// How many objects `client` holds.
    pub fn objects(&self, client: u32) -> usize {
        self.clients.get(&client).map_or(0, |c| c.objects.len())
    }
}

/// What a page is told of `adapter`.
fn info_of(adapter: &wgpu::Adapter) -> AdapterInfo {
    let info = adapter.get_info();
    AdapterInfo {
        vendor: wire::vendor_name(info.vendor).to_owned(),
        fallback: info.device_type == wgpu::DeviceType::Cpu,
        features: names::feature_names(adapter.features()),
        limits: names::limit_values(&adapter.limits()),
    }
}
