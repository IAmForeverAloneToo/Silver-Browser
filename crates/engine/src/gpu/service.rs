//! The GPU service (M41 step 1, D38): what the GPU process runs, and a
//! thread of the browser in one process. It holds `wgpu`'s instance
//! and, for each client, a table of the objects the client made, by the
//! numbers it gave them. A batch it cannot read, a number in use or not
//! there, or more objects than [`MAX_GPU_OBJECTS`] ends the client.

use std::collections::HashMap;

use super::wire::{self, AdapterInfo, Reply, Request};

/// The most objects a client may have live at once.
pub const MAX_GPU_OBJECTS: usize = 1_000_000;

/// One of a client's objects.
enum Object {
    // Its devices come with WebGPU's objects (step 2).
    #[allow(dead_code)]
    Adapter(wgpu::Adapter),
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
}

/// The GPU service.
pub struct GpuService {
    instance: wgpu::Instance,
    /// Whether the fallback adapter alone is taken (a setting, and the
    /// tests').
    fallback_only: bool,
    clients: HashMap<u32, Client>,
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
        }
    }

    /// Takes a batch of `client`'s requests: the replies to send it now,
    /// or why the client is ended.
    pub fn handle(&mut self, client: u32, bytes: &[u8]) -> Result<Vec<Reply>, String> {
        let requests = wire::read_requests(bytes)
            .map_err(|malformed| format!("a malformed batch: {malformed}"))?;
        let mut replies = Vec::new();
        for request in requests {
            self.request(client, request, &mut replies)?;
        }
        Ok(replies)
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
                    self.clients
                        .entry(client)
                        .or_default()
                        .insert(id, Object::Adapter(adapter))?;
                }
                replies.push(Reply::Adapter {
                    call,
                    adapter: info,
                });
            }
            Request::Drop { id } => {
                let objects = &mut self.clients.entry(client).or_default().objects;
                if objects.remove(&id).is_none() {
                    return Err(format!("object {id} dropped, which it does not have"));
                }
            }
        }
        Ok(())
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
    }
}
