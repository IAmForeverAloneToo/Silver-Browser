//! M22.6's acceptance, as far as it holds whatever the machine: what
//! the frames of the milestone's two demos (`tests/demos/`) do. The
//! spinner's frames lay nothing out and composite its layer without
//! drawing it again; the colour fade's lay out the card alone, the rest
//! of the page taken from the layout cache. How long the frames take,
//! sixty a second for ten seconds on each backend, is
//! `benches/animation.rs`'s to measure.

use std::path::Path;
use std::time::Duration;

use engine::{ColorScheme, Engine, FRAME_BUDGET, FrameTime, Preferences};
use paint::GroupKey;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;

/// An engine over the bundled font.
fn engine() -> Engine {
    Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    )
}

/// The demo page `name`.
fn demo(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/demos/{name}.html"));
    std::fs::read_to_string(path).expect("the demo is read")
}

/// The frame of a sixty-a-second clock's `i`th tick.
fn draw(engine: &mut Engine, i: u32) -> Vec<u8> {
    engine
        .frame_at(WIDTH, HEIGHT, FrameTime::at(FRAME_BUDGET * i))
        .to_vec()
}

/// The pixel at `(x, y)` of an RGBA frame.
fn px(frame: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * WIDTH + x) * 4) as usize;
    [frame[i], frame[i + 1], frame[i + 2]]
}

const GREEN: [u8; 3] = [0x3a, 0x7a, 0x4a];

#[test]
fn the_spinners_frames_lay_nothing_out_and_composite_its_layer() {
    let mut engine = engine();
    engine.load_document(&demo("spinner"));
    // The ring, 96px square with its borders, at (592, 145): its top
    // side the turning colour.
    let first = draw(&mut engine, 0);
    assert_eq!(px(&first, 640, 151), GREEN);
    assert_ne!(px(&first, 682, 193), GREEN);
    let runs = engine.layout_runs();
    let mut layer = None;
    for i in 1..=120 {
        let frame = draw(&mut engine, i);
        let layered = engine.layered();
        // The ring's layer, composited at every frame and drawn at none
        // after its first; the document's tiles it is in are drawn again
        // where it reaches, until a layer tree composites it over them
        // (M35.10).
        let boxes: Vec<GroupKey> = layered
            .composited
            .iter()
            .copied()
            .filter(|key| matches!(key, GroupKey::Box(_)))
            .collect();
        assert_eq!(boxes.len(), 1, "frame {i}: {layered:?}");
        assert_eq!(*layer.get_or_insert(boxes[0]), boxes[0]);
        if i > 1 {
            assert!(
                layered.rasterized.iter().all(|(key, _)| *key != boxes[0]),
                "frame {i}: {layered:?}"
            );
        }
        // A quarter of a second in, a quarter turn: the side on the
        // right.
        if i == 15 {
            assert_eq!(px(&frame, 682, 193), GREEN);
            assert_ne!(px(&frame, 640, 151), GREEN);
        }
    }
    assert_eq!(engine.layout_runs(), runs, "no frame laid anything out");
    assert!(engine.needs_frame(), "it turns for as long as it is shown");
}

#[test]
fn under_reduced_motion_the_pages_own_query_stops_the_spinner() {
    let mut engine = engine();
    engine.set_preferences(Preferences {
        color_scheme: ColorScheme::Light,
        reduced_motion: true,
    });
    engine.load_document(&demo("spinner"));
    let first = draw(&mut engine, 0);
    assert!(!engine.needs_frame());
    assert_eq!(engine.next_frame_at(), None);
    let later = engine
        .frame_at(WIDTH, HEIGHT, FrameTime::at(Duration::from_millis(250)))
        .to_vec();
    assert_eq!(first, later);
}

#[test]
fn the_colour_fades_frames_lay_out_the_card_alone() {
    let mut engine = engine();
    engine.load_document(&demo("colour-fade"));
    draw(&mut engine, 0);
    let (mut laid_out, mut reused) = (engine.boxes_laid_out(), engine.boxes_reused());
    for i in 1..=60 {
        draw(&mut engine, i);
        let (now_laid_out, now_reused) = (engine.boxes_laid_out(), engine.boxes_reused());
        // The card, its heading, paragraph and badge, which inherit its
        // colour, and the main, body and html around it; the header and
        // the other two cards taken whole.
        assert_eq!(now_laid_out - laid_out, 7, "frame {i}");
        assert_eq!(now_reused - reused, 3, "frame {i}");
        (laid_out, reused) = (now_laid_out, now_reused);
    }
    assert!(
        engine.needs_frame(),
        "it alternates for as long as it is shown"
    );
}
