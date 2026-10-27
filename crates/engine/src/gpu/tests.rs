//! The GPU process's protocols, service and clients (M41 step 1, D38).

use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use super::client::GpuClient;
use super::wire::{
    self, AdapterInfo, Command, CompilationMessage, ComputeCommand, FromGpu, GPU_VERSION, GpuError,
    RenderCommand, Reply, Request, ToGpu,
};
use crate::ipc::{self, Bytes, Wire};

const WAIT: Duration = Duration::from_secs(30);

fn round_trip<T: Wire + std::fmt::Debug + PartialEq>(value: T) {
    let bytes = ipc::encode(&value);
    assert_eq!(ipc::decode_all::<T>(&bytes).expect("decoded"), value);
}

#[test]
fn the_client_protocol_reads_back_what_it_wrote() {
    let requests = vec![
        Request::RequestAdapter {
            call: 7,
            id: 3,
            power: 2,
            fallback: true,
        },
        Request::Drop { id: 3 },
        // Step 3's: an encoder's commands, nested.
        Request::Finish {
            device: 4,
            id: 5,
            commands: vec![
                Command::RenderPass {
                    desc: "{}".to_owned(),
                    commands: vec![
                        RenderCommand::DrawIndexed {
                            indices: 3,
                            instances: 1,
                            first_index: 0,
                            base_vertex: -2,
                            first_instance: 0,
                        },
                        RenderCommand::SetViewport {
                            x: 0.5,
                            y: 0.0,
                            width: 4.0,
                            height: 4.0,
                            min_depth: 0.0,
                            max_depth: 1.0,
                        },
                        RenderCommand::EndOcclusionQuery {},
                    ],
                },
                Command::ComputePass {
                    commands: vec![ComputeCommand::Dispatch { x: 1, y: 2, z: 3 }],
                },
            ],
        },
        Request::CreateRenderPipeline {
            device: 4,
            id: 6,
            desc: "{}".to_owned(),
            call: Some(9),
        },
        Request::Invalid {
            device: 4,
            message: "why".to_owned(),
        },
    ];
    assert_eq!(
        wire::read_requests(&wire::requests(&requests)).unwrap(),
        requests
    );
    round_trip(Reply::Compilation {
        call: 1,
        messages: vec![CompilationMessage {
            kind: 0,
            line: 2,
            position: 3,
            offset: 4,
            length: 5,
            message: "bad".to_owned(),
        }],
    });
    round_trip(Reply::PipelineMade {
        call: 2,
        error: Some("bad".to_owned()),
    });
    let replies = vec![
        Reply::Adapter {
            call: 7,
            adapter: Some(AdapterInfo {
                vendor: "intel".to_owned(),
                fallback: false,
                features: vec!["core-features-and-limits".to_owned()],
                limits: vec![("maxBindGroups".to_owned(), 4)],
            }),
        },
        Reply::Adapter {
            call: 8,
            adapter: None,
        },
        Reply::Lost {
            why: "gone".to_owned(),
        },
    ];
    assert_eq!(
        wire::read_replies(&wire::replies(&replies)).unwrap(),
        replies
    );
    round_trip(AdapterInfo {
        vendor: String::new(),
        fallback: true,
        features: Vec::new(),
        limits: Vec::new(),
    });
    // A batch cut short, or with bytes after it, is no batch.
    let bytes = wire::requests(&requests);
    assert!(wire::read_requests(&bytes[..bytes.len() - 1]).is_err());
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(wire::read_requests(&longer).is_err());
}

#[test]
fn a_page_is_told_the_vendor_by_name_or_nothing() {
    assert_eq!(wire::vendor_name(0x8086), "intel");
    assert_eq!(wire::vendor_name(0x10de), "nvidia");
    assert_eq!(wire::vendor_name(0x1002), "amd");
    assert_eq!(wire::vendor_name(0x106b), "apple");
    // Microsoft's, which WARP is, and any other.
    assert_eq!(wire::vendor_name(0x1414), "");
    assert_eq!(wire::vendor_name(0), "");
}

#[test]
fn a_client_hears_its_reply_or_that_its_gpu_was_lost() {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let keep = sent.clone();
    let client = GpuClient::new(Box::new(move |bytes| keep.lock().unwrap().push(bytes)));
    assert_ne!(client.id(), client.id());
    let (heard, hear) = mpsc::channel();
    let first = heard.clone();
    client.call(
        |call| Request::RequestAdapter {
            call,
            id: 1,
            power: 0,
            fallback: false,
        },
        move |reply, _| first.send(reply).unwrap(),
    );
    // Queued until flushed.
    assert!(sent.lock().unwrap().is_empty());
    client.flush();
    let batch = wire::read_requests(&sent.lock().unwrap()[0]).unwrap();
    let Request::RequestAdapter { call, .. } = batch[0] else {
        panic!("{batch:?}");
    };
    let answer = Reply::Adapter {
        call,
        adapter: None,
    };
    client
        .deliver(&wire::replies(std::slice::from_ref(&answer)))
        .unwrap();
    assert_eq!(hear.recv_timeout(WAIT).unwrap(), answer);
    // A reply to no call, or bytes that are no batch, the service does not
    // send.
    assert!(client.deliver(&wire::replies(&[answer])).is_err());
    assert!(client.deliver(&[1, 2, 3]).is_err());
    // Lost: the call waiting hears it, and so does a later one, at once;
    // nothing more is sent.
    let second = heard.clone();
    client.call(
        |call| Request::RequestAdapter {
            call,
            id: 2,
            power: 0,
            fallback: false,
        },
        move |reply, _| second.send(reply).unwrap(),
    );
    client.flush();
    client
        .deliver(&wire::replies(&[Reply::Lost {
            why: "ended".to_owned(),
        }]))
        .unwrap();
    let lost = Reply::Lost {
        why: "ended".to_owned(),
    };
    assert_eq!(hear.recv_timeout(WAIT).unwrap(), lost);
    client.call(
        |call| Request::RequestAdapter {
            call,
            id: 3,
            power: 0,
            fallback: false,
        },
        move |reply, _| heard.send(reply).unwrap(),
    );
    assert_eq!(hear.recv_timeout(WAIT).unwrap(), lost);
    assert_eq!(sent.lock().unwrap().len(), 2);
    client.send(&[Request::Drop { id: 1 }]);
    assert_eq!(sent.lock().unwrap().len(), 2);
}

#[cfg(feature = "gpu")]
mod service {
    use super::*;
    use crate::gpu::service::GpuService;

    fn adapter(call: u32, id: u32) -> Request {
        Request::RequestAdapter {
            call,
            id,
            power: 0,
            fallback: false,
        }
    }

    #[test]
    fn an_adapter_is_the_machines_or_none_and_kept_by_the_clients_number() {
        let mut service = GpuService::new(false);
        let replies = service
            .handle(1, &wire::requests(&[adapter(1, 10)]))
            .unwrap();
        let [
            Reply::Adapter {
                call: 1,
                adapter: found,
            },
        ] = &replies[..]
        else {
            panic!("{replies:?}");
        };
        // Another client's numbers are its own.
        service
            .handle(2, &wire::requests(&[adapter(1, 10)]))
            .unwrap();
        if found.is_some() {
            assert_eq!(service.objects(1), 1);
            assert_eq!(service.objects(2), 1);
            // A number in use ends the client.
            let again = service.handle(1, &wire::requests(&[adapter(2, 10)]));
            assert_eq!(again.unwrap_err(), "object 10 made again");
            service
                .handle(1, &wire::requests(&[Request::Drop { id: 10 }]))
                .unwrap();
            assert_eq!(service.objects(1), 0);
        } else {
            eprintln!("no adapter here: the adapter's objects are not checked");
        }
        // A number not there ends it, as do bytes that are no batch.
        let dropped = service.handle(1, &wire::requests(&[Request::Drop { id: 99 }]));
        assert_eq!(
            dropped.unwrap_err(),
            "object 99 dropped, which it does not have"
        );
        assert!(
            service
                .handle(1, &[9, 9])
                .unwrap_err()
                .starts_with("a malformed batch")
        );
        service.gone(2);
        assert_eq!(service.objects(2), 0);
    }

    #[test]
    fn an_adapter_gives_one_device_and_objects_are_named_by_their_kind() {
        let mut service = GpuService::new(false);
        let replies = service
            .handle(1, &wire::requests(&[adapter(1, 10)]))
            .unwrap();
        if matches!(replies[..], [Reply::Adapter { adapter: None, .. }]) {
            eprintln!("no adapter here: devices are not checked");
            return;
        }
        let device = |call, id| Request::RequestDevice {
            call,
            adapter: 10,
            id,
            features: Vec::new(),
            limits: Vec::new(),
        };
        let replies = service
            .handle(1, &wire::requests(&[device(2, 11)]))
            .unwrap();
        assert!(
            matches!(replies[..], [Reply::Device { result: Ok(_), .. }]),
            "{replies:?}"
        );
        // A second device of the same adapter, past the page's own check,
        // is refused too.
        let replies = service
            .handle(1, &wire::requests(&[device(3, 12)]))
            .unwrap();
        assert_eq!(
            replies,
            [Reply::Device {
                call: 3,
                result: Err(
                    "adapter is \"consumed\": it has already been used to create a device"
                        .to_owned()
                ),
            }]
        );
        // An object named as of another kind ends the client.
        let wrong = service.handle(
            1,
            &wire::requests(&[Request::CreateBuffer {
                device: 10,
                id: 13,
                size: 4,
                usage: 8,
                mapped: false,
            }]),
        );
        assert_eq!(
            wrong.unwrap_err(),
            "object 10 named as a device, which is an adapter"
        );
    }

    #[test]
    fn the_gpu_process_says_hello_answers_its_clients_and_ends_a_bad_one() {
        let ((browser_in, browser_out), (gpu_in, gpu_out)) = ipc::duplex();
        let process = std::thread::spawn(move || crate::gpu::process::run(gpu_in, gpu_out));
        let to: ipc::Sender<ToGpu> = ipc::Sender::new(browser_out);
        // What the GPU process says, heard on a thread, so that a reply
        // that never comes fails the test rather than holding it.
        let (heard, hear) = mpsc::channel();
        std::thread::spawn(move || {
            let mut from = std::io::BufReader::new(browser_in);
            while let Ok(Some(message)) = ipc::read_message::<FromGpu>(&mut from) {
                let _ = heard.send(message);
            }
        });
        let read = || hear.recv_timeout(WAIT).expect("a message in time");
        to.send(&ToGpu::Hello {
            version: GPU_VERSION,
            fallback_only: false,
        })
        .unwrap();
        assert!(matches!(read(), FromGpu::Hello { version } if version == GPU_VERSION));
        to.send(&ToGpu::Client {
            client: 4,
            bytes: Bytes(wire::requests(&[adapter(5, 1)])),
        })
        .unwrap();
        let FromGpu::Client { client: 4, bytes } = read() else {
            panic!("no reply");
        };
        assert!(matches!(
            &wire::read_replies(&bytes.0).unwrap()[..],
            [Reply::Adapter { call: 5, .. }]
        ));
        // A batch that is none ends client 4 alone.
        to.send(&ToGpu::Client {
            client: 4,
            bytes: Bytes(vec![0xff]),
        })
        .unwrap();
        let FromGpu::Bad { client: 4, why } = read() else {
            panic!("not refused");
        };
        assert!(why.starts_with("a malformed batch"), "{why}");
        to.send(&ToGpu::Client {
            client: 5,
            bytes: Bytes(wire::requests(&[adapter(1, 1)])),
        })
        .unwrap();
        assert!(matches!(read(), FromGpu::Client { client: 5, .. }));
        to.send(&ToGpu::Gone { client: 5 }).unwrap();
        // The browser gone, the process ends cleanly.
        drop(to);
        assert_eq!(process.join().unwrap(), Ok(()));
    }

    #[test]
    fn a_gpu_process_greeted_otherwise_refuses() {
        let ((_browser_in, browser_out), (gpu_in, gpu_out)) = ipc::duplex();
        let to: ipc::Sender<ToGpu> = ipc::Sender::new(browser_out);
        to.send(&ToGpu::Hello {
            version: GPU_VERSION + 1,
            fallback_only: false,
        })
        .unwrap();
        // Nothing more: a hello taken would leave it waiting for the end.
        drop(to);
        let ran = crate::gpu::process::run(gpu_in, gpu_out);
        assert_eq!(
            ran,
            Err(format!(
                "a hello of version {}, not {GPU_VERSION}",
                GPU_VERSION + 1
            ))
        );
    }

    #[test]
    fn a_local_client_reaches_a_service_of_this_process() {
        let client = crate::gpu::local_client().expect("the gpu feature");
        let (heard, hear) = mpsc::channel();
        let id = client.id();
        client.call(
            |call| adapter(call, id),
            move |reply, _| heard.send(reply).unwrap(),
        );
        client.flush();
        assert!(matches!(
            hear.recv_timeout(WAIT).unwrap(),
            Reply::Adapter { .. }
        ));
        // A number not there loses its GPU.
        client.send(&[Request::Drop { id: id + 1000 }]);
        let deadline = std::time::Instant::now() + WAIT;
        while !client.is_lost() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            client.lost().as_deref(),
            Some(format!("object {} dropped, which it does not have", id + 1000).as_str())
        );
    }

    /// The names a list of the bootstrap's holds, from its source.
    fn bootstrap_list(name: &str) -> Vec<String> {
        let source = include_str!("../../../js/src/quickjs/bootstrap_gpu.js");
        let start = source.find(&format!("const {name} = [")).expect("the list");
        let list = &source[start..start + source[start..].find("];").expect("its end")];
        let mut names: Vec<String> = list
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect();
        names.sort();
        names
    }

    fn sorted<T>(list: &[(&str, T)]) -> Vec<String> {
        let mut names: Vec<String> = list.iter().map(|(name, _)| (*name).to_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn the_bootstraps_formats_are_those_the_service_reads() {
        // A name the page may give and the service does not know would
        // end the client.
        assert_eq!(
            bootstrap_list("GPU_FORMATS"),
            sorted(crate::gpu::convert::FORMATS)
        );
        assert_eq!(
            bootstrap_list("GPU_VERTEX_FORMATS"),
            sorted(crate::gpu::convert::VERTEX_FORMATS)
        );
    }

    /// A service with a device numbered 11, or `None` where there is no
    /// adapter.
    fn device() -> Option<GpuService> {
        let mut service = GpuService::new(false);
        let replies = service
            .handle(1, &wire::requests(&[adapter(1, 10)]))
            .unwrap();
        if matches!(replies[..], [Reply::Adapter { adapter: None, .. }]) {
            eprintln!("no adapter here: the work is not checked");
            return None;
        }
        let replies = service
            .handle(
                1,
                &wire::requests(&[Request::RequestDevice {
                    call: 2,
                    adapter: 10,
                    id: 11,
                    features: Vec::new(),
                    limits: Vec::new(),
                }]),
            )
            .unwrap();
        assert!(matches!(replies[..], [Reply::Device { result: Ok(_), .. }]));
        Some(service)
    }

    #[test]
    fn a_clients_validation_error_goes_to_a_validation_scope_or_is_uncaptured() {
        let Some(mut service) = device() else {
            return;
        };
        let invalid = Request::Invalid {
            device: 11,
            message: "the client's".to_owned(),
        };
        let replies = service
            .handle(
                1,
                &wire::requests(&[
                    Request::PushErrorScope {
                        device: 11,
                        filter: 0,
                    },
                    Request::PushErrorScope {
                        device: 11,
                        filter: 1,
                    },
                    invalid.clone(),
                    Request::PopErrorScope {
                        call: 3,
                        device: 11,
                    },
                    Request::PopErrorScope {
                        call: 4,
                        device: 11,
                    },
                ]),
            )
            .unwrap();
        let error = GpuError {
            kind: 0,
            message: "the client's".to_owned(),
        };
        assert_eq!(
            replies,
            [
                Reply::ErrorScope {
                    call: 3,
                    result: Ok(None),
                },
                Reply::ErrorScope {
                    call: 4,
                    result: Ok(Some(error.clone())),
                },
            ]
        );
        service.handle(1, &wire::requests(&[invalid])).unwrap();
        assert_eq!(
            service.events(),
            [(1, Reply::Uncaptured { device: 11, error })]
        );
    }

    #[test]
    fn a_vertex_attribute_past_any_stride_is_refused_without_a_panic() {
        // wgpu-core adds an attribute's offset and size unchecked; an
        // offset near 2^64 overflowed it (webgpu_ops, CI).
        let Some(mut service) = device() else {
            return;
        };
        let code =
            "@vertex fn main(@location(0) p: vec4f) -> @builtin(position) vec4f { return p; }";
        let desc = format!(
            r#"{{"layout":"auto","vertex":{{"module":12,"entryPoint":"main","buffers":[{{"arrayStride":16,"attributes":[{{"format":"float32x4","offset":{},"shaderLocation":0}}]}}]}}}}"#,
            u64::MAX - 4
        );
        service
            .handle(
                1,
                &wire::requests(&[
                    Request::CreateShaderModule {
                        device: 11,
                        id: 12,
                        code: code.to_owned(),
                    },
                    Request::CreateRenderPipeline {
                        device: 11,
                        id: 13,
                        desc,
                        call: None,
                    },
                ]),
            )
            .unwrap();
        assert!(matches!(
            service.events()[..],
            [(1, Reply::Uncaptured { device: 11, .. })]
        ));
    }

    #[test]
    fn a_device_holds_more_error_scopes_than_a_page_needs() {
        // The WebGPU CTS pushes 100,000; past MAX_ERROR_SCOPES the client
        // ends.
        let Some(mut service) = device() else {
            return;
        };
        let push = Request::PushErrorScope {
            device: 11,
            filter: 0,
        };
        service
            .handle(1, &wire::requests(&vec![push; 5000]))
            .unwrap();
    }

    #[test]
    fn an_invalid_buffer_mapped_at_creation_is_unmapped_without_an_error() {
        let Some(mut service) = device() else {
            return;
        };
        // Usage 0: its creation's error, and no other at its unmap.
        service
            .handle(
                1,
                &wire::requests(&[
                    Request::CreateBuffer {
                        device: 11,
                        id: 12,
                        size: 8,
                        usage: 0,
                        mapped: true,
                    },
                    Request::Unmap { buffer: 12 },
                ]),
            )
            .unwrap();
        assert!(matches!(
            service.events()[..],
            [(1, Reply::Uncaptured { device: 11, .. })]
        ));
    }

    #[test]
    fn ranges_past_a_buffer_are_refused_without_a_panic() {
        let Some(mut service) = device() else {
            return;
        };
        let buffer = |id, usage| Request::CreateBuffer {
            device: 11,
            id,
            size: 16,
            usage,
            mapped: false,
        };
        // MAP_READ | COPY_DST; VERTEX | INDEX.
        service
            .handle(1, &wire::requests(&[buffer(12, 9), buffer(13, 0x30)]))
            .unwrap();
        let replies = service
            .handle(
                1,
                &wire::requests(&[Request::MapBuffer {
                    call: 5,
                    buffer: 12,
                    mode: 1,
                    offset: 8,
                    size: 16,
                }]),
            )
            .unwrap();
        assert_eq!(
            replies,
            [Reply::Mapped {
                call: 5,
                error: Some("the range is past the buffer's end".to_owned()),
            }]
        );
        // And a validation error, as WebGPU's mapAsync.
        assert!(matches!(
            service.events()[..],
            [(1, Reply::Uncaptured { device: 11, ref error })] if error.kind == 0
        ));
        // Past 2^64 too (webgpu_ops found it ending the client).
        let replies = service
            .handle(
                1,
                &wire::requests(&[Request::MapBuffer {
                    call: 6,
                    buffer: 12,
                    mode: 1,
                    offset: u64::MAX,
                    size: 8,
                }]),
            )
            .unwrap();
        assert!(matches!(
            replies[..],
            [Reply::Mapped { error: Some(_), .. }]
        ));
        let texture = Request::CreateTexture {
            device: 11,
            id: 14,
            desc: r#"{"size":[4,4],"format":"rgba8unorm","usage":16}"#.to_owned(),
        };
        let view = Request::CreateView {
            texture: 14,
            id: 15,
            desc: "{}".to_owned(),
        };
        let past = vec![
            RenderCommand::SetVertexBuffer {
                slot: 0,
                buffer: Some(13),
                offset: 64,
                size: None,
            },
            RenderCommand::SetVertexBuffer {
                slot: 0,
                buffer: Some(13),
                offset: 8,
                size: Some(u64::MAX),
            },
            RenderCommand::SetIndexBuffer {
                buffer: 13,
                format: 0,
                offset: 0,
                size: Some(32),
            },
        ];
        let pass = Command::RenderPass {
            desc: r#"{"colorAttachments":[{"view":15,"loadOp":"clear","storeOp":"store"}]}"#
                .to_owned(),
            commands: past.clone(),
        };
        service
            .handle(
                1,
                &wire::requests(&[
                    texture,
                    view,
                    Request::Finish {
                        device: 11,
                        id: 16,
                        commands: vec![pass],
                    },
                    Request::CreateRenderBundle {
                        device: 11,
                        id: 17,
                        desc: r#"{"colorFormats":["rgba8unorm"]}"#.to_owned(),
                        commands: past,
                    },
                    Request::Submit {
                        device: 11,
                        buffers: vec![16],
                    },
                ]),
            )
            .unwrap();
    }

    /// A render pass's descriptor on a 4x4 texture numbered 12 (its view
    /// 13), made on the service's device 11.
    fn target(service: &mut GpuService) -> String {
        service
            .handle(
                1,
                &wire::requests(&[
                    Request::CreateTexture {
                        device: 11,
                        id: 12,
                        desc: r#"{"size":[4,4],"format":"rgba8unorm","usage":16}"#.to_owned(),
                    },
                    Request::CreateView {
                        texture: 12,
                        id: 13,
                        desc: "{}".to_owned(),
                    },
                ]),
            )
            .unwrap();
        r#"{"colorAttachments":[{"view":13,"loadOp":"clear","storeOp":"store"}]}"#.to_owned()
    }

    #[test]
    fn a_render_bundles_commands_are_replayed_and_their_errors_are_the_passes() {
        // wgpu's own bundles panic where they fail (webgpu_ops found it):
        // a bundle is its commands, replayed by the pass executing it.
        let Some(mut service) = device() else {
            return;
        };
        let desc = target(&mut service);
        let replies = service
            .handle(
                1,
                &wire::requests(&[
                    Request::PushErrorScope {
                        device: 11,
                        filter: 0,
                    },
                    Request::CreateRenderBundle {
                        device: 11,
                        id: 14,
                        desc: r#"{"colorFormats":["depth24plus"]}"#.to_owned(),
                        commands: vec![RenderCommand::Draw {
                            vertices: 3,
                            instances: 1,
                            first_vertex: 0,
                            first_instance: 0,
                        }],
                    },
                    Request::PopErrorScope {
                        call: 3,
                        device: 11,
                    },
                    Request::PushErrorScope {
                        device: 11,
                        filter: 0,
                    },
                    Request::Finish {
                        device: 11,
                        id: 15,
                        commands: vec![Command::RenderPass {
                            desc,
                            commands: vec![RenderCommand::ExecuteBundles {
                                bundles: vec![14, 14],
                            }],
                        }],
                    },
                    Request::PopErrorScope {
                        call: 4,
                        device: 11,
                    },
                ]),
            )
            .unwrap();
        let [
            Reply::ErrorScope {
                call: 3,
                result: Ok(None),
            },
            Reply::ErrorScope {
                call: 4,
                result: Ok(Some(executed)),
            },
        ] = &replies[..]
        else {
            panic!("{replies:?}");
        };
        assert!(!executed.message.is_empty(), "{executed:?}");
    }

    #[test]
    fn a_pass_replays_at_most_its_cap_of_commands() {
        let Some(mut service) = device() else {
            return;
        };
        let desc = target(&mut service);
        let bundle = vec![RenderCommand::SetStencilReference { reference: 1 }; 1000];
        let replies = service
            .handle(
                1,
                &wire::requests(&[
                    Request::CreateRenderBundle {
                        device: 11,
                        id: 14,
                        desc: "{}".to_owned(),
                        commands: bundle,
                    },
                    Request::PushErrorScope {
                        device: 11,
                        filter: 0,
                    },
                    Request::Finish {
                        device: 11,
                        id: 15,
                        commands: vec![Command::RenderPass {
                            desc,
                            commands: vec![RenderCommand::ExecuteBundles {
                                bundles: vec![14; 5000],
                            }],
                        }],
                    },
                    Request::PopErrorScope {
                        call: 3,
                        device: 11,
                    },
                ]),
            )
            .unwrap();
        assert_eq!(
            replies,
            [Reply::ErrorScope {
                call: 3,
                result: Ok(Some(GpuError {
                    kind: 0,
                    message: format!(
                        "A render pass replays more than {} commands.",
                        crate::gpu::service::MAX_PASS_COMMANDS
                    ),
                })),
            }]
        );
    }

    #[test]
    fn a_device_with_scopes_left_open_goes_without_a_panic() {
        // webgpu_ops found it: wgpu's scopes must go innermost first.
        let Some(mut service) = device() else {
            return;
        };
        let push = |filter| Request::PushErrorScope { device: 11, filter };
        service
            .handle(1, &wire::requests(&[push(0), push(1), push(0)]))
            .unwrap();
        service.gone(1);
        assert_eq!(service.objects(1), 0);
    }
}
