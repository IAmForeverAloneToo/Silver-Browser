//! The GPU process's protocols, service and clients (M41 step 1, D38).

use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use super::client::GpuClient;
use super::wire::{self, AdapterInfo, FromGpu, GPU_VERSION, Reply, Request, ToGpu};
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
    ];
    assert_eq!(
        wire::read_requests(&wire::requests(&requests)).unwrap(),
        requests
    );
    let replies = vec![
        Reply::Adapter {
            call: 7,
            adapter: Some(AdapterInfo {
                vendor: "intel".to_owned(),
                fallback: false,
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
        move |reply| first.send(reply).unwrap(),
    );
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
        move |reply| second.send(reply).unwrap(),
    );
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
        move |reply| heard.send(reply).unwrap(),
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
            move |reply| heard.send(reply).unwrap(),
        );
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
}
