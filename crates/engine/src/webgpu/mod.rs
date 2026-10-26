//! A document's WebGPU (M41 step 2, D39, docs/design/gpu.md): the
//! bootstrap's objects call one native, `native.gpu(op, bytes)`, whose
//! operations become requests of the engine's GPU client; what the GPU
//! process answers comes back through the document's inbox as
//! `WorkerMessage::Gpu`, then `js::Task::Gpu`. The objects a document
//! made are dropped in the GPU process when it goes.

use std::collections::BTreeSet;
use std::sync::Arc;

use devtools::Json;

use crate::gpu::GpuClient;
use crate::gpu::wire::{AdapterInfo, Reply, Request};
use crate::scripts::workers::{Inbox, WorkerMessage};

mod work;

/// A document's WebGPU: the objects it made, and the client they live
/// in.
#[derive(Default)]
pub(crate) struct WebGpu {
    client: Option<Arc<GpuClient>>,
    /// Its objects, dropped in the GPU process when it goes.
    made: BTreeSet<u32>,
    /// Its devices, whose news it listens to.
    devices: BTreeSet<u32>,
    /// Each canvas's buffer its frames are read back through (step 4),
    /// by the canvas's handle, with its size.
    readback: std::collections::BTreeMap<i64, (u32, u64)>,
}

/// An answer the bootstrap throws: a `DOMException` of `name`.
fn fail(name: &str, message: &str) -> (String, Vec<u8>) {
    let error = Json::object([("name", Json::from(name)), ("message", Json::from(message))]);
    (Json::object([("error", error)]).write(), Vec::new())
}

fn done(result: Json) -> (String, Vec<u8>) {
    (Json::object([("result", result)]).write(), Vec::new())
}

fn number(op: &Json, name: &str) -> u64 {
    op.get(name).and_then(Json::as_f64).unwrap_or(0.0) as u64
}

fn whole(op: &Json, name: &str) -> u32 {
    u32::try_from(number(op, name)).unwrap_or(u32::MAX)
}

/// Tells the document, as a task, `told` with `bytes`.
fn tell(inbox: &Inbox, told: Json, bytes: Vec<u8>) {
    let _ = inbox.send(WorkerMessage::Gpu(told.write(), bytes));
}

/// A promise's settling, told: `members` beside its number.
fn settle(inbox: &Inbox, promise: u64, members: Vec<(&str, Json)>, bytes: Vec<u8>) {
    let mut all = vec![("promise", Json::from(promise))];
    all.extend(members);
    tell(inbox, Json::object(all), bytes);
}

fn rejection(name: &str, message: &str) -> Vec<(&'static str, Json)> {
    vec![(
        "error",
        Json::object([("name", Json::from(name)), ("message", Json::from(message))]),
    )]
}

fn pairs(list: &[(String, u64)]) -> Json {
    Json::Array(
        list.iter()
            .map(|(name, value)| Json::Array(vec![Json::from(name.as_str()), Json::from(*value)]))
            .collect(),
    )
}

fn strings(list: &[String]) -> Json {
    Json::Array(list.iter().map(|name| Json::from(name.as_str())).collect())
}

fn adapter_json(id: u32, info: &AdapterInfo) -> Json {
    Json::object([
        ("id", Json::from(id)),
        ("vendor", Json::from(info.vendor.as_str())),
        ("fallback", Json::Bool(info.fallback)),
        ("features", strings(&info.features)),
        ("limits", pairs(&info.limits)),
    ])
}

impl WebGpu {
    /// One call of `native.gpu`: its answer, `{"result"}` or
    /// `{"error": {name, message}}`, and bytes.
    pub(crate) fn call(
        &mut self,
        request: &str,
        bytes: Vec<u8>,
        client: Option<&Arc<GpuClient>>,
        inbox: &Inbox,
    ) -> (String, Vec<u8>) {
        let Ok(op) = Json::parse(request) else {
            return fail("TypeError", "a malformed WebGPU call");
        };
        let what = op.get("op").and_then(Json::as_str).unwrap_or("");
        let promise = number(&op, "promise");
        // Without a GPU (no `gpu` feature): no adapter.
        let Some(client) = client else {
            if what == "requestAdapter" {
                settle(inbox, promise, vec![("adapter", Json::Null)], Vec::new());
                return done(Json::Null);
            }
            return fail("OperationError", "there is no GPU");
        };
        self.client = Some(client.clone());
        match what {
            "requestAdapter" => {
                let id = client.id();
                let inbox = inbox.clone();
                let (power, fallback) = (
                    number(&op, "power") as u8,
                    op.get("fallback").and_then(Json::as_bool).unwrap_or(false),
                );
                self.made.insert(id);
                client.call(
                    |call| Request::RequestAdapter {
                        call,
                        id,
                        power,
                        fallback,
                    },
                    move |reply, _| {
                        let adapter = match reply {
                            Reply::Adapter {
                                adapter: Some(info),
                                ..
                            } => adapter_json(id, &info),
                            _ => Json::Null,
                        };
                        settle(&inbox, promise, vec![("adapter", adapter)], Vec::new());
                    },
                );
                done(Json::Null)
            }
            "requestDevice" => {
                let id = client.id();
                let adapter = whole(&op, "adapter");
                let features = op
                    .get("features")
                    .and_then(Json::as_array)
                    .map(|list| {
                        list.iter()
                            .filter_map(Json::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                let limits = op
                    .get("limits")
                    .and_then(Json::as_array)
                    .map(|list| {
                        list.iter()
                            .filter_map(|pair| {
                                let pair = pair.as_array()?;
                                Some((
                                    pair.first()?.as_str()?.to_owned(),
                                    pair.get(1)?.as_f64()? as u64,
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.made.insert(id);
                self.devices.insert(id);
                let news = inbox.clone();
                client.listen(id, move |reply| match reply {
                    Reply::Uncaptured { error, .. } => tell(
                        &news,
                        Json::object([
                            ("device", Json::from(id)),
                            (
                                "uncaptured",
                                Json::object([
                                    ("kind", Json::from(u64::from(error.kind))),
                                    ("message", Json::from(error.message.as_str())),
                                ]),
                            ),
                        ]),
                        Vec::new(),
                    ),
                    Reply::DeviceLost {
                        destroyed, message, ..
                    } => tell(
                        &news,
                        Json::object([
                            ("device", Json::from(id)),
                            (
                                "lost",
                                Json::object([
                                    (
                                        "reason",
                                        Json::from(if destroyed { "destroyed" } else { "unknown" }),
                                    ),
                                    ("message", Json::from(message.as_str())),
                                ]),
                            ),
                        ]),
                        Vec::new(),
                    ),
                    _ => {}
                });
                let inbox = inbox.clone();
                client.call(
                    |call| Request::RequestDevice {
                        call,
                        adapter,
                        id,
                        features,
                        limits,
                    },
                    move |reply, _| {
                        let members = match reply {
                            Reply::Device {
                                result: Ok(info), ..
                            } => vec![(
                                "device",
                                Json::object([
                                    ("id", Json::from(id)),
                                    ("features", strings(&info.features)),
                                    ("limits", pairs(&info.limits)),
                                ]),
                            )],
                            Reply::Device {
                                result: Err(why), ..
                            } => rejection("OperationError", &why),
                            _ => rejection("OperationError", "the GPU is lost"),
                        };
                        settle(&inbox, promise, members, Vec::new());
                    },
                );
                done(Json::Null)
            }
            "destroyDevice" => {
                client.queue([Request::DestroyDevice {
                    id: whole(&op, "device"),
                }]);
                done(Json::Null)
            }
            "pushErrorScope" => {
                client.queue([Request::PushErrorScope {
                    device: whole(&op, "device"),
                    filter: number(&op, "filter") as u8,
                }]);
                done(Json::Null)
            }
            "popErrorScope" => {
                let device = whole(&op, "device");
                let inbox = inbox.clone();
                client.call(
                    |call| Request::PopErrorScope { call, device },
                    move |reply, _| {
                        let members = match reply {
                            Reply::ErrorScope {
                                result: Ok(error), ..
                            } => vec![(
                                "scope",
                                error.map_or(Json::Null, |error| {
                                    Json::object([
                                        ("kind", Json::from(u64::from(error.kind))),
                                        ("message", Json::from(error.message.as_str())),
                                    ])
                                }),
                            )],
                            Reply::ErrorScope {
                                result: Err(why), ..
                            } => rejection("OperationError", &why),
                            _ => vec![("scope", Json::Null)],
                        };
                        settle(&inbox, promise, members, Vec::new());
                    },
                );
                done(Json::Null)
            }
            "createBuffer" => {
                let id = client.id();
                self.made.insert(id);
                client.queue([Request::CreateBuffer {
                    device: whole(&op, "device"),
                    id,
                    size: number(&op, "size"),
                    usage: whole(&op, "usage"),
                    mapped: op.get("mapped").and_then(Json::as_bool).unwrap_or(false),
                }]);
                done(Json::object([("id", Json::from(id))]))
            }
            "destroyBuffer" => {
                client.queue([Request::DestroyBuffer {
                    id: whole(&op, "buffer"),
                }]);
                done(Json::Null)
            }
            "writeBuffer" => {
                let (device, buffer) = (whole(&op, "device"), whole(&op, "buffer"));
                client.queue_bytes(number(&op, "offset"), &bytes, |offset, bytes| {
                    Request::WriteBuffer {
                        device,
                        buffer,
                        offset,
                        bytes,
                    }
                });
                done(Json::Null)
            }
            "mapAsync" => {
                let buffer = whole(&op, "buffer");
                let (mode, offset, size) = (
                    number(&op, "mode") as u8,
                    number(&op, "offset"),
                    number(&op, "size"),
                );
                let inbox = inbox.clone();
                client.call(
                    |call| Request::MapBuffer {
                        call,
                        buffer,
                        mode,
                        offset,
                        size,
                    },
                    move |reply, bytes| {
                        let members = match reply {
                            Reply::Mapped { error: None, .. } => vec![("mapped", Json::Bool(true))],
                            Reply::Mapped {
                                error: Some(why), ..
                            } => rejection("OperationError", &why),
                            _ => rejection("AbortError", "the GPU is lost"),
                        };
                        settle(&inbox, promise, members, bytes);
                    },
                );
                done(Json::Null)
            }
            "unmap" => {
                let buffer = whole(&op, "buffer");
                if op.get("write").and_then(Json::as_bool).unwrap_or(false) {
                    client.queue_bytes(number(&op, "offset"), &bytes, |offset, bytes| {
                        Request::MappedWrite {
                            buffer,
                            offset,
                            bytes,
                        }
                    });
                }
                client.queue([Request::Unmap { buffer }]);
                done(Json::Null)
            }
            "workDone" => {
                let device = whole(&op, "device");
                let inbox = inbox.clone();
                client.call(
                    |call| Request::WorkDone { call, device },
                    move |_, _| settle(&inbox, promise, Vec::new(), Vec::new()),
                );
                done(Json::Null)
            }
            _ => self
                .work(what, &op, &bytes, client, inbox)
                .unwrap_or_else(|| fail("TypeError", "an unknown WebGPU call")),
        }
    }
}

impl Drop for WebGpu {
    /// The document goes: its objects in the GPU process go too.
    fn drop(&mut self) {
        let Some(client) = &self.client else {
            return;
        };
        for device in &self.devices {
            client.unlisten(*device);
        }
        client.queue(self.made.iter().map(|id| Request::Drop { id: *id }));
        client.flush();
    }
}
