//! M35.1: the interaction benchmark (`docs/design/performance.md`, "What
//! is measured"), Speedometer's TodoMVC in plain DOM and scripts. Each
//! run loads the app in a new engine and times its three steps as
//! Speedometer does, each the events of the step (its synchronous part)
//! and the frame that follows (its asynchronous part):
//!
//! - a hundred items added through the input: the focus in the new
//!   item's field, each item's text typed and Enter pressed;
//! - every item marked complete, and then every item deleted: a key the
//!   app's runner listens for, which clicks each item's checkbox, and
//!   then each item's delete button, as Speedometer's runner clicks
//!   them from the page's own script.
//!
//! `harness = false` (see `Cargo.toml`): a plain binary, run by hand,
//! its numbers recorded in `docs/perf-budgets.md`:
//!
//! ```text
//! cargo bench -p engine --bench interaction
//! ```

#![forbid(unsafe_code)]

mod support;

use std::hint::black_box;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use engine::{Engine, FrameTime, Key, LoadState, Url};
use net::test_server::{Reply, TestServer};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const ITEMS: usize = 100;
const RUNS: usize = 10;

/// The app: TodoMVC's markup and behaviour in plain DOM, its count in
/// the document's title for the benchmark to read, and the runner's
/// keys: F8 clicks every checkbox, F9 every delete button.
const APP: &str = "<!DOCTYPE html><html><head><title>todos</title><style>\
    body { font: 14px sans-serif; background: #f5f5f5; margin: 0 }\
    .todoapp { background: #fff; margin: 40px auto; width: 550px; box-shadow: 0 2px 4px rgba(0,0,0,.2) }\
    h1 { text-align: center; color: #b83f45; font-size: 64px; margin: 0 }\
    .new-todo { box-sizing: border-box; width: 100%; padding: 16px 16px 16px 60px; font-size: 24px; border: 0 }\
    .todo-list { list-style: none; margin: 0; padding: 0 }\
    .todo-list li { position: relative; font-size: 24px; border-bottom: 1px solid #ededed }\
    .todo-list li .toggle { position: absolute; top: 0; bottom: 0; margin: auto 0; width: 40px }\
    .todo-list li label { display: block; padding: 15px 15px 15px 60px; line-height: 1.2 }\
    .todo-list li.completed label { color: #949494; text-decoration: line-through }\
    .todo-list li .destroy { position: absolute; top: 0; right: 10px; bottom: 0; width: 40px; height: 40px; margin: auto 0 }\
    .footer { padding: 10px 15px; color: #777; display: flex; justify-content: space-between }\
    </style></head><body>\
    <section class=todoapp><header><h1>todos</h1>\
    <input class=new-todo placeholder='What needs to be done?'></header>\
    <section class=main><ul class=todo-list></ul></section>\
    <footer class=footer><span class=todo-count></span><button class=clear-completed>Clear completed</button></footer>\
    </section><script>\
    const list = document.querySelector('.todo-list');\
    const input = document.querySelector('.new-todo');\
    const count = document.querySelector('.todo-count');\
    function update() {\
      const left = list.querySelectorAll('li:not(.completed)').length;\
      const text = left + (left === 1 ? ' item left' : ' items left');\
      count.textContent = text;\
      document.title = list.children.length + ' items, ' + text;\
    }\
    function add(title) {\
      const li = document.createElement('li');\
      const view = document.createElement('div');\
      view.className = 'view';\
      const toggle = document.createElement('input');\
      toggle.type = 'checkbox';\
      toggle.className = 'toggle';\
      const label = document.createElement('label');\
      label.textContent = title;\
      const destroy = document.createElement('button');\
      destroy.className = 'destroy';\
      view.appendChild(toggle); view.appendChild(label); view.appendChild(destroy);\
      li.appendChild(view);\
      list.appendChild(li);\
      update();\
    }\
    input.addEventListener('keydown', function (event) {\
      if (event.key === 'Enter' && input.value.trim() !== '') {\
        add(input.value.trim());\
        input.value = '';\
      }\
    });\
    list.addEventListener('change', function (event) {\
      if (event.target.classList.contains('toggle')) {\
        const li = event.target.parentNode.parentNode;\
        if (event.target.checked) { li.classList.add('completed'); } else { li.classList.remove('completed'); }\
        update();\
      }\
    });\
    list.addEventListener('click', function (event) {\
      if (event.target.classList.contains('destroy')) {\
        event.target.parentNode.parentNode.remove();\
        update();\
      }\
    });\
    document.addEventListener('keydown', function (event) {\
      if (event.key === 'F8') { for (const box of list.querySelectorAll('.toggle')) { box.click(); } }\
      if (event.key === 'F9') { for (const button of list.querySelectorAll('.destroy')) { button.click(); } }\
    });\
    update();\
    </script></body></html>";

/// The events of a step and the frame after it.
struct Step {
    events: Duration,
    frame: Duration,
}

fn frame(engine: &mut Engine, at: &mut u32) -> Duration {
    *at += 1;
    let start = Instant::now();
    black_box(engine.frame_at(WIDTH, HEIGHT, FrameTime::at(engine::FRAME_BUDGET * *at)));
    start.elapsed()
}

fn press(engine: &mut Engine, name: &str) {
    let key = Key::new(name, name);
    engine.key_down(&key);
    engine.key_up(&key);
}

/// One run: the app loaded in a new engine from `url`, then its three
/// steps.
fn run(url: &Url) -> [Step; 3] {
    let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
        .expect("the bundled font parses");
    let mut engine = Engine::with_fonts(fonts);
    engine.set_scripting(true);
    let (wake, woken) = channel();
    engine.set_waker(move || {
        let _ = wake.send(());
    });
    engine.navigate(url.clone());
    while *engine.load_state() == LoadState::Loading {
        woken
            .recv_timeout(Duration::from_secs(30))
            .expect("the app loads");
        engine.poll_network();
        engine.run_tasks();
    }
    let mut at = 0;
    frame(&mut engine, &mut at);
    assert!(
        engine.focus_next(false),
        "the new item's field takes the focus"
    );

    let start = Instant::now();
    for i in 0..ITEMS {
        engine.insert_text(&format!("Something to do, number {i}"));
        press(&mut engine, "Enter");
    }
    let adding = Step {
        events: start.elapsed(),
        frame: frame(&mut engine, &mut at),
    };
    assert_eq!(engine.title(), format!("{ITEMS} items, {ITEMS} items left"));

    let start = Instant::now();
    press(&mut engine, "F8");
    let completing = Step {
        events: start.elapsed(),
        frame: frame(&mut engine, &mut at),
    };
    assert_eq!(engine.title(), format!("{ITEMS} items, 0 items left"));

    let start = Instant::now();
    press(&mut engine, "F9");
    let deleting = Step {
        events: start.elapsed(),
        frame: frame(&mut engine, &mut at),
    };
    assert_eq!(engine.title(), "0 items, 0 items left");
    [adding, completing, deleting]
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn median(mut times: Vec<f64>) -> f64 {
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

fn main() {
    let server = TestServer::plain(|_| Reply::ok("text/html; charset=utf-8", APP.as_bytes()));
    let url = server.url("/");
    let runs: Vec<[Step; 3]> = (0..support::rounds(RUNS)).map(|_| run(&url)).collect();
    println!(
        "TodoMVC in plain DOM, {ITEMS} items, medians of {}:",
        runs.len()
    );
    let mut total = Vec::new();
    for (index, name) in ["adding 100 items", "completing them", "deleting them"]
        .iter()
        .enumerate()
    {
        let events = median(runs.iter().map(|run| ms(run[index].events)).collect());
        let frame = median(runs.iter().map(|run| ms(run[index].frame)).collect());
        total.push(events + frame);
        println!("  {name}: the events {events:.2} ms, the frame after {frame:.2} ms");
    }
    println!("  the three steps: {:.2} ms", total.iter().sum::<f64>());
}
