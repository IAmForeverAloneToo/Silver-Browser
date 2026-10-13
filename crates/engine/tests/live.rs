//! M15.6's acceptance (docs/design/navigation.md, "Tests"): the engine
//! navigates to https://example.com/ and finds the page's title and its
//! text, and draws it. The site changes what else it shows from time to
//! time, so nothing else is looked for. It needs the network, so it is
//! ignored unless asked for, and run by hand:
//!
//! ```text
//! cargo test -p engine --test live -- --ignored
//! ```
//!
//! It trusts Mozilla's roots, as the browser does, and beside them the
//! certificates of the PEM file `SILVER_EXTRA_ROOTS` names, if it names
//! one: the root of a network that intercepts TLS. The browser itself
//! has no such option.

#![forbid(unsafe_code)]

use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use engine::{Engine, LoadState, Url};
use net::Client;
use net::transport::Config;

#[test]
#[ignore = "needs the network; run by hand"]
fn example_com_is_shown_with_its_title_and_text() {
    let mut engine = Engine::new().expect("the system's fonts");
    let config = match std::env::var_os("SILVER_EXTRA_ROOTS") {
        Some(path) => {
            let pem = std::fs::read(&path).expect("the roots' file reads");
            Config::default()
                .trust_pem(&pem)
                .expect("the file holds roots")
        }
        None => Config::default(),
    };
    engine.set_network(Client::new(config));
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });

    let url = Url::parse("https://example.com/").expect("the URL parses");
    engine.navigate(url.clone());
    assert_eq!(*engine.load_state(), LoadState::Loading);
    let deadline = Instant::now() + Duration::from_secs(60);
    while *engine.load_state() == LoadState::Loading {
        let left = deadline
            .checked_duration_since(Instant::now())
            .expect("the page arrives within a minute");
        woken.recv_timeout(left).expect("the page arrives in time");
        engine.poll_network();
    }

    assert_eq!(*engine.load_state(), LoadState::Loaded);
    assert_eq!(engine.url(), Some(&url));
    assert_eq!(engine.title(), "Example Domain");
    let document = engine.document();
    let text = document.text_content(document.root());
    assert!(text.contains("for use in documentation examples"), "{text}");
    // And it is drawn: the frame is not blank.
    let frame = engine.frame(800, 600);
    let (pixels, _) = frame.as_chunks::<4>();
    assert!(
        pixels.iter().any(|pixel| *pixel != pixels[0]),
        "something drawn"
    );
}
