//! The context against the reference browser: each expected pixel was
//! read back from Chrome 154 drawing the same thing (the probe page of
//! the M30 notes), equal or within the stated levels where two
//! rasterizers may round an edge differently. `tiny-skia` samples an
//! edge four by four, so a pixel a curve crosses can be a sixteenth
//! (16 levels) from Chrome's exact coverage.

use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::sync::{Arc, Mutex};

use tiny_skia::FillRule;

use crate::path::Path;
use crate::state::{Composite, Gradient, GradientKind, Matrix, Paint, Pattern, Repetition};
use crate::{
    Canvas, MAX_CANVAS_SIDE, MAX_CANVAS_STATES, MAX_PATH_POINTS, Rgba, parse_color, serialize_color,
};

fn px(canvas: &Canvas, x: i64, y: i64) -> [u8; 4] {
    let data = canvas.get_image_data(x, y, 1, 1).expect("readable");
    [data[0], data[1], data[2], data[3]]
}

fn near(got: [u8; 4], want: [u8; 4], levels: u8) -> bool {
    got.iter().zip(want).all(|(&g, w)| g.abs_diff(w) <= levels)
}

#[track_caller]
fn assert_near(got: [u8; 4], want: [u8; 4], levels: u8) {
    assert!(
        near(got, want, levels),
        "got {got:?}, wanted {want:?} within {levels}"
    );
}

fn color(text: &str) -> Rgba {
    parse_color(text, Rgba::BLACK).expect("a colour")
}

fn fill(canvas: &mut Canvas, text: &str) {
    canvas.state_mut().fill = Paint::Color(color(text));
}

#[test]
fn colours_read_back_as_the_standard_serializes_them() {
    let cases = [
        ("red", "#ff0000"),
        ("rgba(255,0,0,0.5)", "rgba(255, 0, 0, 0.5)"),
        ("#abc", "#aabbcc"),
        ("hsl(120, 100%, 50%)", "#00ff00"),
        ("transparent", "rgba(0, 0, 0, 0)"),
        ("rgb(1 2 3 / 40%)", "rgba(1, 2, 3, 0.4)"),
        ("currentcolor", "#000000"),
        ("  #000  ", "#000000"),
    ];
    for (text, want) in cases {
        assert_eq!(serialize_color(color(text)), want, "{text}");
    }
    assert_eq!(parse_color("bogus", Rgba::BLACK), None);
    assert_eq!(parse_color("", Rgba::BLACK), None);
    assert_eq!(
        parse_color("CurrentColor", Rgba::new(1, 2, 3, 4)),
        Some(Rgba::new(1, 2, 3, 4))
    );
}

#[test]
fn fill_rect_covers_whole_pixels_exactly() {
    let mut canvas = Canvas::new(10, 10, false);
    fill(&mut canvas, "rgb(10, 20, 30)");
    canvas.fill_rect(2.0, 2.0, 4.0, 4.0);
    assert_eq!(px(&canvas, 1, 1), [0, 0, 0, 0]);
    assert_eq!(px(&canvas, 2, 2), [10, 20, 30, 255]);
    assert_eq!(px(&canvas, 5, 5), [10, 20, 30, 255]);
    assert_eq!(px(&canvas, 6, 6), [0, 0, 0, 0]);
}

#[test]
fn a_rectangle_on_half_pixels_covers_its_edges_by_half() {
    let mut canvas = Canvas::new(10, 10, false);
    canvas.fill_rect(2.5, 2.5, 3.0, 3.0);
    assert_eq!(px(&canvas, 3, 3), [0, 0, 0, 255]);
    assert_near(px(&canvas, 2, 4), [0, 0, 0, 128], 1);
    assert_near(px(&canvas, 4, 5), [0, 0, 0, 128], 1);
}

#[test]
fn colour_alpha_and_global_alpha_compose_as_chrome_does() {
    let mut canvas = Canvas::new(10, 10, false);
    fill(&mut canvas, "rgba(255, 128, 0, 0.5)");
    canvas.fill_rect(0.0, 0.0, 10.0, 10.0);
    canvas.state_mut().set_global_alpha(0.5);
    fill(&mut canvas, "#0000ff");
    canvas.fill_rect(5.0, 0.0, 5.0, 10.0);
    assert_near(px(&canvas, 1, 1), [255, 128, 0, 128], 1);
    assert_near(px(&canvas, 6, 1), [85, 42, 170, 192], 1);
}

#[test]
fn image_data_round_trips_through_the_premultiplied_bitmap() {
    let mut canvas = Canvas::new(10, 10, false);
    for (x, pixel) in [[255, 128, 7, 1], [200, 100, 50, 128], [255, 255, 255, 0]]
        .iter()
        .enumerate()
    {
        canvas.put_image_data(pixel, 1, 1, x as i64, 0, None);
    }
    assert_eq!(px(&canvas, 0, 0), [255, 255, 0, 1]);
    assert_eq!(px(&canvas, 1, 0), [199, 100, 50, 128]);
    assert_eq!(px(&canvas, 2, 0), [0, 0, 0, 0]);
}

#[test]
fn put_image_data_writes_only_the_dirty_rectangle_within_the_bitmap() {
    let mut canvas = Canvas::new(4, 4, false);
    let data: Vec<u8> = [9, 8, 7, 255].repeat(9);
    canvas.state_mut().set_global_alpha(0.0);
    canvas.translate(1.0, 1.0);
    canvas.put_image_data(&data, 3, 3, 2, 2, Some((-1, 0, 2, 2)));
    assert_eq!(px(&canvas, 2, 2), [9, 8, 7, 255]);
    assert_eq!(px(&canvas, 2, 3), [9, 8, 7, 255]);
    assert_eq!(px(&canvas, 3, 2), [0, 0, 0, 0]);
    canvas.put_image_data(&data, 3, 3, -2, -2, None);
    assert_eq!(px(&canvas, 0, 0), [9, 8, 7, 255]);
    assert_eq!(px(&canvas, 1, 0), [0, 0, 0, 0]);
    // A length that does not match is refused, not read past.
    canvas.put_image_data(&data[..8], 3, 3, 0, 0, None);
}

#[test]
fn get_image_data_reads_outside_as_transparent_and_flips_negative_sizes() {
    let mut canvas = Canvas::new(4, 4, false);
    fill(&mut canvas, "#f00");
    canvas.fill_rect(0.0, 0.0, 1.0, 1.0);
    assert_eq!(canvas.get_image_data(-5, -5, 2, 2).unwrap(), vec![0; 16]);
    // From (1, 1) back two each way: (0, 0) is the last pixel read.
    let data = canvas.get_image_data(1, 1, -2, -2).unwrap();
    assert_eq!(&data[..12], &[0; 12]);
    assert_eq!(&data[12..], &[255, 0, 0, 255]);
    let error = canvas.get_image_data(0, 0, 0, 1).unwrap_err();
    assert_eq!(
        (error.name, error.message.as_str()),
        ("IndexSizeError", "The source width is 0.")
    );
    let error = canvas.get_image_data(0, 0, 1 << 20, 1 << 20).unwrap_err();
    assert_eq!(error.name, "RangeError");
}

#[test]
fn composite_operators_match_the_reference() {
    let cases = [
        ("source-over", [109, 0, 146, 223], [255, 0, 0, 191]),
        ("copy", [0, 0, 255, 128], [0, 0, 0, 0]),
        ("destination-out", [255, 0, 0, 95], [255, 0, 0, 191]),
        ("lighter", [191, 0, 128, 255], [255, 0, 0, 191]),
        ("multiply", [109, 0, 37, 223], [255, 0, 0, 191]),
        ("xor", [191, 0, 64, 127], [255, 0, 0, 191]),
        ("source-in", [0, 0, 255, 96], [0, 0, 0, 0]),
        ("difference", [218, 0, 146, 223], [255, 0, 0, 191]),
    ];
    for (name, inside, outside) in cases {
        let mut canvas = Canvas::new(10, 10, false);
        fill(&mut canvas, "rgba(255, 0, 0, 0.75)");
        canvas.fill_rect(0.0, 0.0, 10.0, 10.0);
        canvas.state_mut().composite = Composite::from_name(name).unwrap();
        fill(&mut canvas, "rgba(0, 0, 255, 0.5)");
        canvas.fill_rect(0.0, 0.0, 5.0, 10.0);
        assert!(
            near(px(&canvas, 1, 1), inside, 2),
            "{name} inside {:?}",
            px(&canvas, 1, 1)
        );
        assert!(
            near(px(&canvas, 7, 1), outside, 2),
            "{name} outside {:?}",
            px(&canvas, 7, 1)
        );
    }
    assert_eq!(Composite::from_name("bogus"), None);
    assert_eq!(Composite::from_name("lighter").unwrap().name(), "lighter");
}

#[test]
fn a_filled_circle_matches_the_reference_at_its_edges() {
    let mut canvas = Canvas::new(20, 20, false);
    canvas
        .with_path(|path, m| path.arc(10.0, 10.0, 6.0, 0.0, TAU, false, m))
        .unwrap();
    canvas.fill(None, FillRule::Winding);
    assert_eq!(px(&canvas, 10, 10), [0, 0, 0, 255]);
    assert_near(px(&canvas, 10, 4), [0, 0, 0, 249], 8);
    assert_near(px(&canvas, 4, 10), [0, 0, 0, 249], 8);
    assert_near(px(&canvas, 14, 14), [0, 0, 0, 35], 20);
    assert_eq!(px(&canvas, 15, 15), [0, 0, 0, 0]);
    assert_eq!(px(&canvas, 16, 10), [0, 0, 0, 0]);
}

#[test]
fn strokes_cover_the_reference_rows() {
    let mut canvas = Canvas::new(20, 20, false);
    canvas.state_mut().set_line_width(2.0);
    canvas.with_path(|path, m| {
        path.move_to(2.0, 10.0, m);
        path.line_to(18.0, 10.0, m);
    });
    canvas.stroke(None);
    assert_eq!(px(&canvas, 10, 8), [0, 0, 0, 0]);
    assert_eq!(px(&canvas, 10, 9), [0, 0, 0, 255]);
    assert_eq!(px(&canvas, 10, 10), [0, 0, 0, 255]);
    assert_eq!(px(&canvas, 10, 11), [0, 0, 0, 0]);
    assert_eq!(px(&canvas, 1, 10), [0, 0, 0, 0]);
    assert_eq!(px(&canvas, 18, 10), [0, 0, 0, 0]);

    let mut canvas = Canvas::new(20, 20, false);
    canvas.with_path(|path, m| {
        path.move_to(2.0, 10.0, m);
        path.line_to(18.0, 10.0, m);
    });
    canvas.stroke(None);
    assert_near(px(&canvas, 10, 9), [0, 0, 0, 128], 1);
    assert_near(px(&canvas, 10, 10), [0, 0, 0, 128], 1);
}

#[test]
fn a_stroke_is_traced_in_the_transform_it_was_drawn_in() {
    // The path is built at scale 1, then stroked at scale 4: its width
    // is four pixels, the points stay where they were put.
    let mut canvas = Canvas::new(20, 20, false);
    canvas.with_path(|path, m| {
        path.move_to(0.0, 10.0, m);
        path.line_to(20.0, 10.0, m);
    });
    canvas.scale(4.0, 4.0);
    canvas.stroke(None);
    assert_eq!(px(&canvas, 5, 8), [0, 0, 0, 255]);
    assert_eq!(px(&canvas, 5, 11), [0, 0, 0, 255]);
    assert_eq!(px(&canvas, 5, 12), [0, 0, 0, 0]);
    assert_eq!(px(&canvas, 5, 7), [0, 0, 0, 0]);
}

#[test]
fn dashes_leave_gaps() {
    let mut canvas = Canvas::new(20, 4, false);
    canvas.state_mut().set_line_width(2.0);
    canvas.state_mut().dash.set(&[2.0, 2.0]);
    canvas.with_path(|path, m| {
        path.move_to(0.0, 2.0, m);
        path.line_to(20.0, 2.0, m);
    });
    canvas.stroke(None);
    assert_eq!(px(&canvas, 0, 2)[3], 255);
    assert_eq!(px(&canvas, 2, 2)[3], 0);
    assert_eq!(px(&canvas, 4, 2)[3], 255);

    let dash = &mut canvas.state_mut().dash;
    dash.set(&[1.0, 2.0, 3.0]);
    assert_eq!(dash.list, vec![1.0, 2.0, 3.0, 1.0, 2.0, 3.0]);
    dash.set(&[1.0, -1.0]);
    dash.set(&[1.0, f64::NAN]);
    assert_eq!(dash.list, vec![1.0, 2.0, 3.0, 1.0, 2.0, 3.0]);
}

#[test]
fn a_linear_gradient_matches_the_reference() {
    let mut canvas = Canvas::new(10, 1, false);
    let mut gradient = Gradient::new(GradientKind::Linear {
        x0: 0.0,
        y0: 0.0,
        x1: 10.0,
        y1: 0.0,
    });
    gradient.add_stop(0.0, color("#000")).unwrap();
    gradient.add_stop(1.0, color("#fff")).unwrap();
    canvas.state_mut().fill = Paint::Gradient(Arc::new(Mutex::new(gradient)));
    canvas.fill_rect(0.0, 0.0, 10.0, 1.0);
    assert_near(px(&canvas, 0, 0), [12, 12, 12, 255], 1);
    assert_near(px(&canvas, 5, 0), [140, 140, 140, 255], 1);
    assert_near(px(&canvas, 9, 0), [242, 242, 242, 255], 1);
}

#[test]
fn a_radial_gradient_matches_the_reference() {
    let mut canvas = Canvas::new(20, 20, false);
    let mut gradient = Gradient::new(GradientKind::Radial {
        x0: 10.0,
        y0: 10.0,
        r0: 2.0,
        x1: 10.0,
        y1: 10.0,
        r1: 8.0,
    });
    gradient.add_stop(0.0, color("#f00")).unwrap();
    gradient.add_stop(1.0, color("#00f")).unwrap();
    canvas.state_mut().fill = Paint::Gradient(Arc::new(Mutex::new(gradient)));
    canvas.fill_rect(0.0, 0.0, 20.0, 20.0);
    assert_near(px(&canvas, 10, 10), [255, 0, 0, 255], 1);
    assert_near(px(&canvas, 15, 10), [105, 0, 150, 255], 2);
    assert_near(px(&canvas, 19, 19), [0, 0, 255, 255], 1);
}

#[test]
fn a_conic_gradient_turns_clockwise_from_its_angle() {
    let mut canvas = Canvas::new(20, 20, false);
    let mut gradient = Gradient::new(GradientKind::Conic {
        angle: 0.0,
        x: 10.0,
        y: 10.0,
    });
    gradient.add_stop(0.0, color("#f00")).unwrap();
    gradient.add_stop(1.0, color("#00f")).unwrap();
    canvas.state_mut().fill = Paint::Gradient(Arc::new(Mutex::new(gradient)));
    canvas.fill_rect(0.0, 0.0, 20.0, 20.0);
    // A quarter turn (below the centre) is three quarters red; half a
    // turn (left of it) half and half.
    let below = px(&canvas, 10, 18);
    let left = px(&canvas, 2, 10);
    assert!(below[0] > 170 && below[2] < 85, "{below:?}");
    assert!(left[0].abs_diff(left[2]) < 30, "{left:?}");
}

#[test]
fn gradients_of_no_stops_or_no_extent_paint_nothing() {
    for (kind, stops) in [
        (
            GradientKind::Linear {
                x0: 1.0,
                y0: 1.0,
                x1: 1.0,
                y1: 1.0,
            },
            2,
        ),
        (
            GradientKind::Radial {
                x0: 1.0,
                y0: 1.0,
                r0: 3.0,
                x1: 1.0,
                y1: 1.0,
                r1: 3.0,
            },
            2,
        ),
        (
            GradientKind::Linear {
                x0: 0.0,
                y0: 0.0,
                x1: 4.0,
                y1: 0.0,
            },
            0,
        ),
    ] {
        let mut canvas = Canvas::new(4, 4, false);
        let mut gradient = Gradient::new(kind);
        for i in 0..stops {
            gradient.add_stop(f64::from(i), color("#f00")).unwrap();
        }
        canvas.state_mut().fill = Paint::Gradient(Arc::new(Mutex::new(gradient)));
        canvas.fill_rect(0.0, 0.0, 4.0, 4.0);
        assert_eq!(px(&canvas, 1, 1), [0, 0, 0, 0], "{kind:?}");
    }
    let mut gradient = Gradient::new(GradientKind::Conic {
        angle: 0.0,
        x: 0.0,
        y: 0.0,
    });
    let error = gradient.add_stop(2.0, Rgba::BLACK).unwrap_err();
    assert_eq!(error.name, "IndexSizeError");
    assert_eq!(
        error.message,
        "The provided value (2) is outside the range (0.0, 1.0)."
    );
    // Stops of one offset keep the order they were added in.
    gradient.add_stop(0.5, color("#f00")).unwrap();
    gradient.add_stop(0.5, color("#0f0")).unwrap();
    gradient.add_stop(0.25, color("#00f")).unwrap();
    let order: Vec<String> = gradient
        .stops
        .iter()
        .map(|&(_, c)| serialize_color(c))
        .collect();
    assert_eq!(order, ["#0000ff", "#ff0000", "#00ff00"]);
}

#[test]
fn rect_and_round_rect_fill_as_the_reference() {
    let mut canvas = Canvas::new(20, 20, false);
    canvas
        .with_path(|path, m| {
            path.rect(2.0, 2.0, 6.0, 6.0, m);
            path.round_rect(10.0, 10.0, 8.0, 8.0, &[(3.0, 3.0)], m)
        })
        .unwrap();
    canvas.fill(None, FillRule::Winding);
    assert_eq!(px(&canvas, 4, 4), [0, 0, 0, 255]);
    assert_near(px(&canvas, 10, 10), [0, 0, 0, 0], 16);
    assert_eq!(px(&canvas, 14, 14), [0, 0, 0, 255]);

    // Radii larger than the sides are scaled down together: a square
    // with radii of twice its side is a circle.
    let mut canvas = Canvas::new(10, 10, false);
    canvas
        .with_path(|path, m| path.round_rect(0.0, 0.0, 10.0, 10.0, &[(20.0, 20.0)], m))
        .unwrap();
    canvas.fill(None, FillRule::Winding);
    assert_eq!(px(&canvas, 0, 0), [0, 0, 0, 0]);
    assert_eq!(px(&canvas, 5, 0)[3], 255);
    assert_eq!(px(&canvas, 5, 5)[3], 255);

    let mut path = Path::new();
    let m = Matrix::IDENTITY;
    let error = path.round_rect(0.0, 0.0, 1.0, 1.0, &[], &m).unwrap_err();
    assert_eq!(
        (error.name, error.message.as_str()),
        (
            "RangeError",
            "0 radii provided. Between one and four radii are necessary."
        )
    );
    let error = path
        .round_rect(0.0, 0.0, 1.0, 1.0, &[(1.0, -2.0)], &m)
        .unwrap_err();
    assert_eq!(error.message, "Radius value -2 is negative.");
}

#[test]
fn arcs_refuse_a_negative_radius_and_ignore_values_not_finite() {
    let mut path = Path::new();
    let m = Matrix::IDENTITY;
    let error = path.arc(0.0, 0.0, -1.0, 0.0, 1.0, false, &m).unwrap_err();
    assert_eq!(
        (error.name, error.message.as_str()),
        ("IndexSizeError", "The radius provided (-1) is negative.")
    );
    let error = path.arc_to(0.0, 0.0, 1.0, 1.0, -1.0, &m).unwrap_err();
    assert_eq!(error.message, "The radius provided (-1) is negative.");
    let error = path
        .ellipse(0.0, 0.0, 1.0, -0.5, 0.0, 0.0, 1.0, false, &m)
        .unwrap_err();
    assert_eq!(
        error.message,
        "The minor-axis radius provided (-0.5) is negative."
    );
    path.arc(f64::NAN, 0.0, 1.0, 0.0, 1.0, false, &m).unwrap();
    assert!(path.is_empty());
}

#[test]
fn arc_sweeps_follow_the_direction_and_never_pass_a_turn() {
    // Clockwise from 0 to -pi/2 goes three quarters of the way round;
    // anticlockwise, a quarter.
    let quarter = |anticlockwise: bool| {
        let mut canvas = Canvas::new(20, 20, false);
        canvas
            .with_path(|path, m| {
                path.move_to(10.0, 10.0, m);
                path.arc(10.0, 10.0, 8.0, 0.0, -FRAC_PI_2, anticlockwise, m)
            })
            .unwrap();
        canvas.fill(None, FillRule::Winding);
        (
            px(&canvas, 14, 6)[3],
            px(&canvas, 6, 14)[3],
            px(&canvas, 14, 14)[3],
        )
    };
    assert_eq!(quarter(false), (0, 255, 255));
    assert_eq!(quarter(true), (255, 0, 0));
    // Ten turns are one.
    let mut canvas = Canvas::new(20, 20, false);
    canvas
        .with_path(|path, m| path.arc(10.0, 10.0, 8.0, 0.0, 10.0 * TAU, false, m))
        .unwrap();
    // A move and four quarter turns of three points each.
    assert_eq!(canvas.path().len(), 13);
}

#[test]
fn arc_to_rounds_the_corner_and_ends_at_the_second_tangent() {
    let mut canvas = Canvas::new(20, 20, false);
    canvas
        .with_path(|path, m| {
            path.move_to(0.0, 2.0, m);
            path.arc_to(12.0, 2.0, 12.0, 20.0, 5.0, m)
        })
        .unwrap();
    let (x, y) = canvas.path().last_point().unwrap();
    assert!((x - 12.0).abs() < 1e-9 && (y - 7.0).abs() < 1e-9, "{x} {y}");
    // Collinear points make a line to the first.
    let mut path = Path::new();
    let m = Matrix::IDENTITY;
    path.move_to(0.0, 0.0, &m);
    path.arc_to(5.0, 0.0, 10.0, 0.0, 3.0, &m).unwrap();
    assert_eq!(path.last_point(), Some((5.0, 0.0)));
}

#[test]
fn clip_limits_drawing_and_clear_rect() {
    let mut canvas = Canvas::new(20, 20, false);
    canvas.fill_rect(0.0, 0.0, 20.0, 20.0);
    canvas.with_path(|path, m| path.rect(5.0, 5.0, 10.0, 10.0, m));
    canvas.save();
    canvas.clip(None, FillRule::Winding);
    canvas.clear_rect(0.0, 0.0, 20.0, 20.0);
    assert_eq!(px(&canvas, 2, 2), [0, 0, 0, 255]);
    assert_eq!(px(&canvas, 7, 7), [0, 0, 0, 0]);
    fill(&mut canvas, "#f00");
    canvas.fill_rect(0.0, 0.0, 20.0, 20.0);
    assert_eq!(px(&canvas, 7, 7), [255, 0, 0, 255]);
    assert_eq!(px(&canvas, 2, 2), [0, 0, 0, 255]);
    canvas.restore();
    canvas.clear_rect(0.0, 0.0, 20.0, 20.0);
    assert_eq!(px(&canvas, 2, 2), [0, 0, 0, 0]);
}

#[test]
fn points_in_paths_and_strokes_match_the_reference() {
    let mut canvas = Canvas::new(20, 20, false);
    canvas.with_path(|path, m| path.rect(5.0, 5.0, 10.0, 10.0, m));
    assert!(canvas.is_point_in_path(None, 6.0, 6.0, FillRule::Winding));
    assert!(!canvas.is_point_in_path(None, 1.0, 1.0, FillRule::Winding));
    assert!(canvas.is_point_in_path(None, 15.0, 15.0, FillRule::Winding));
    assert!(canvas.is_point_in_stroke(None, 5.0, 10.0));
    assert!(!canvas.is_point_in_stroke(None, 7.0, 10.0));
    assert!(!canvas.is_point_in_path(None, f64::NAN, 1.0, FillRule::Winding));

    let mut triangle = Path::new();
    let m = Matrix::IDENTITY;
    triangle.move_to(0.0, 0.0, &m);
    triangle.line_to(10.0, 0.0, &m);
    triangle.line_to(10.0, 10.0, &m);
    triangle.close();
    assert!(canvas.is_point_in_path(Some(&triangle), 8.0, 2.0, FillRule::Winding));
    // A Path2D goes through the current transform; the point does not.
    canvas.translate(100.0, 0.0);
    assert!(!canvas.is_point_in_path(Some(&triangle), 8.0, 2.0, FillRule::Winding));
    assert!(canvas.is_point_in_path(Some(&triangle), 108.0, 2.0, FillRule::Winding));

    // Two nested squares the same way round: inside both by nonzero,
    // a hole by even-odd.
    let mut nested = Path::new();
    nested.rect(0.0, 0.0, 10.0, 10.0, &m);
    nested.rect(2.0, 2.0, 6.0, 6.0, &m);
    canvas.set_transform(&Matrix::IDENTITY);
    assert!(canvas.is_point_in_path(Some(&nested), 5.0, 5.0, FillRule::Winding));
    assert!(!canvas.is_point_in_path(Some(&nested), 5.0, 5.0, FillRule::EvenOdd));
}

#[test]
fn shadows_match_the_reference() {
    let mut canvas = Canvas::new(20, 20, false);
    let state = canvas.state_mut();
    state.shadow_color = color("rgba(0,0,0,1)");
    state.set_shadow_offset(Some(5.0), None);
    fill(&mut canvas, "#f00");
    canvas.fill_rect(2.0, 2.0, 5.0, 5.0);
    assert_eq!(px(&canvas, 3, 3), [255, 0, 0, 255]);
    assert_eq!(px(&canvas, 9, 3), [0, 0, 0, 255]);
    assert_eq!(px(&canvas, 13, 3), [0, 0, 0, 0]);

    let mut canvas = Canvas::new(30, 10, false);
    let state = canvas.state_mut();
    state.shadow_color = color("#000");
    state.set_shadow_blur(4.0);
    state.set_shadow_offset(Some(15.0), None);
    fill(&mut canvas, "#f00");
    canvas.fill_rect(2.0, 2.0, 6.0, 6.0);
    for (x, alpha) in [(20, 186), (17, 130), (15, 50), (25, 23), (27, 3)] {
        let got = px(&canvas, x, 5);
        assert!(
            got[3].abs_diff(alpha) <= 12,
            "x {x}: {got:?}, wanted alpha {alpha}"
        );
    }
}

#[test]
fn a_shape_outside_the_bitmap_casts_its_shadow_in() {
    let mut canvas = Canvas::new(10, 10, false);
    let state = canvas.state_mut();
    state.shadow_color = color("#00f");
    state.set_shadow_offset(Some(50.0), Some(0.0));
    canvas.fill_rect(-48.0, 0.0, 4.0, 10.0);
    assert_eq!(px(&canvas, 3, 5), [0, 0, 255, 255]);
    assert_eq!(px(&canvas, 7, 5), [0, 0, 0, 0]);
    // An offset far past the bitmap draws nothing and makes no layer.
    canvas.state_mut().set_shadow_offset(Some(1e30), None);
    canvas.state_mut().set_shadow_blur(1e9);
    canvas.fill_rect(0.0, 0.0, 10.0, 10.0);
    assert_eq!(px(&canvas, 7, 5), [0, 0, 0, 255]);
}

fn checker() -> Pattern {
    // Two by two, red at the top left, the rest transparent.
    let mut pixels = vec![0u8; 16];
    pixels[..4].copy_from_slice(&[255, 0, 0, 255]);
    Pattern {
        width: 2,
        height: 2,
        pixels: Arc::from(pixels),
        repetition: Repetition::Repeat,
        transform: Matrix::IDENTITY,
        origin_clean: true,
    }
}

#[test]
fn patterns_repeat_as_asked() {
    let mut canvas = Canvas::new(10, 10, false);
    canvas.state_mut().fill = Paint::Pattern(Arc::new(Mutex::new(checker())));
    canvas.fill_rect(0.0, 0.0, 10.0, 10.0);
    assert_eq!(px(&canvas, 0, 0), [255, 0, 0, 255]);
    assert_eq!(px(&canvas, 1, 0), [0, 0, 0, 0]);
    assert_eq!(px(&canvas, 2, 2), [255, 0, 0, 255]);

    for (repetition, at_2_0, at_0_2) in [
        (Repetition::NoRepeat, 0, 0),
        (Repetition::RepeatX, 255, 0),
        (Repetition::RepeatY, 0, 255),
    ] {
        let mut canvas = Canvas::new(10, 10, false);
        let pattern = Pattern {
            repetition,
            ..checker()
        };
        canvas.state_mut().fill = Paint::Pattern(Arc::new(Mutex::new(pattern)));
        canvas.fill_rect(0.0, 0.0, 10.0, 10.0);
        assert_eq!(px(&canvas, 0, 0)[3], 255, "{repetition:?}");
        assert_eq!(px(&canvas, 2, 0)[3], at_2_0, "{repetition:?}");
        assert_eq!(px(&canvas, 0, 2)[3], at_0_2, "{repetition:?}");
    }
    assert_eq!(Repetition::from_name("bogus"), None);
    assert_eq!(Repetition::from_name(""), Some(Repetition::Repeat));
}

#[test]
fn a_pattern_follows_the_transform_and_its_own() {
    let mut canvas = Canvas::new(10, 10, false);
    let pattern = Pattern {
        repetition: Repetition::NoRepeat,
        transform: Matrix::new(1.0, 0.0, 0.0, 1.0, 2.0, 0.0),
        ..checker()
    };
    canvas.state_mut().fill = Paint::Pattern(Arc::new(Mutex::new(pattern)));
    canvas.translate(0.0, 3.0);
    canvas.fill_rect(-10.0, -10.0, 30.0, 30.0);
    assert_eq!(px(&canvas, 2, 3), [255, 0, 0, 255]);
    assert_eq!(px(&canvas, 0, 0), [0, 0, 0, 0]);
    assert_eq!(px(&canvas, 4, 3), [0, 0, 0, 0]);
}

#[test]
fn transforms_multiply_and_restore_takes_them_back() {
    let mut canvas = Canvas::new(10, 10, false);
    canvas.translate(2.0, 3.0);
    canvas.scale(2.0, 2.0);
    canvas.rotate(0.0);
    assert_eq!(
        canvas.state().transform,
        Matrix::new(2.0, 0.0, 0.0, 2.0, 2.0, 3.0)
    );
    canvas.scale(f64::NAN, 1.0);
    canvas.rotate(f64::INFINITY);
    assert_eq!(
        canvas.state().transform,
        Matrix::new(2.0, 0.0, 0.0, 2.0, 2.0, 3.0)
    );
    canvas.rotate(PI);
    let (x, y) = canvas.state().transform.apply(1.0, 0.0);
    assert!((x - 0.0).abs() < 1e-12 && (y - 3.0).abs() < 1e-12);

    fill(&mut canvas, "red");
    let before = canvas.state().transform;
    canvas.save();
    fill(&mut canvas, "blue");
    canvas.translate(5.0, 5.0);
    canvas.restore();
    canvas.restore();
    assert!(matches!(canvas.state().fill, Paint::Color(c) if c == color("red")));
    assert_eq!(canvas.state().transform, before);
}

#[test]
fn a_transform_that_flattens_draws_nothing() {
    let mut canvas = Canvas::new(10, 10, false);
    canvas.scale(0.0, 1.0);
    canvas.fill_rect(0.0, 0.0, 10.0, 10.0);
    canvas.with_path(|path, m| {
        path.move_to(0.0, 0.0, m);
        path.line_to(5.0, 5.0, m);
    });
    canvas.stroke(None);
    canvas.set_transform(&Matrix::IDENTITY);
    canvas.state_mut().set_line_width(4.0);
    canvas.stroke_rect(1.0, 1.0, 0.0, 0.0);
    assert_eq!(canvas.get_image_data(0, 0, 10, 10).unwrap(), vec![0; 400]);
    // A rectangle of no width is a line.
    canvas.stroke_rect(5.0, 0.0, 0.0, 10.0);
    assert_eq!(px(&canvas, 5, 5)[3], 255);
}

#[test]
fn resizing_clears_the_bitmap_and_the_state() {
    let mut canvas = Canvas::new(10, 10, false);
    fill(&mut canvas, "red");
    canvas.fill_rect(0.0, 0.0, 10.0, 10.0);
    canvas.translate(3.0, 3.0);
    canvas.save();
    canvas.taint();
    let generation = canvas.generation();
    canvas.resize(10, 10);
    assert!(canvas.generation() > generation);
    assert!(canvas.origin_clean());
    assert_eq!(px(&canvas, 1, 1), [0, 0, 0, 0]);
    assert_eq!(canvas.state().transform, Matrix::IDENTITY);
    assert!(matches!(canvas.state().fill, Paint::Color(Rgba::BLACK)));
}

#[test]
fn a_tainted_canvas_refuses_to_be_read() {
    let mut canvas = Canvas::new(2, 2, false);
    canvas.taint();
    let error = canvas.get_image_data(0, 0, 1, 1).unwrap_err();
    assert_eq!(
        (error.name, error.message.as_str()),
        (
            "SecurityError",
            "The canvas has been tainted by cross-origin data."
        )
    );
}

#[test]
fn an_opaque_canvas_stays_opaque() {
    let mut canvas = Canvas::new(4, 4, true);
    assert_eq!(px(&canvas, 0, 0), [0, 0, 0, 255]);
    canvas.clear_rect(0.0, 0.0, 4.0, 4.0);
    assert_eq!(px(&canvas, 0, 0), [0, 0, 0, 255]);
    fill(&mut canvas, "rgba(255, 255, 255, 0.5)");
    canvas.fill_rect(0.0, 0.0, 4.0, 4.0);
    assert_near(px(&canvas, 1, 1), [128, 128, 128, 255], 1);
    canvas.state_mut().composite = Composite::Copy;
    canvas.fill_rect(0.0, 0.0, 2.0, 2.0);
    assert_eq!(px(&canvas, 3, 3), [0, 0, 0, 255]);
    assert_eq!(px(&canvas, 0, 0)[3], 255);
    // Exclusive-or of opaque over opaque leaves nothing: black.
    fill(&mut canvas, "#fff");
    canvas.state_mut().composite = Composite::SourceOver;
    canvas.fill_rect(0.0, 0.0, 4.0, 4.0);
    canvas.state_mut().composite = Composite::Xor;
    canvas.fill_rect(0.0, 0.0, 4.0, 4.0);
    assert_eq!(px(&canvas, 1, 1), [0, 0, 0, 255]);
}

#[test]
fn a_canvas_past_the_bounds_has_no_bitmap_and_draws_nothing() {
    let mut canvas = Canvas::new(MAX_CANVAS_SIDE + 1, 1, false);
    assert!(!canvas.has_bitmap());
    canvas.fill_rect(0.0, 0.0, 10.0, 10.0);
    canvas.clear_rect(0.0, 0.0, 10.0, 10.0);
    canvas.clip(None, FillRule::Winding);
    canvas.put_image_data(&[1, 2, 3, 4], 1, 1, 0, 0, None);
    assert_eq!(px(&canvas, 0, 0), [0, 0, 0, 0]);
    assert!(canvas.snapshot().is_none());
    let empty = Canvas::new(0, 5, false);
    assert!(!empty.has_bitmap());
}

#[test]
fn the_state_stack_and_paths_are_bounded() {
    // With the stack full, a further save keeps nothing: the restore
    // after it gives back the state saved before.
    let mut canvas = Canvas::new(1, 1, false);
    for _ in 0..MAX_CANVAS_STATES {
        canvas.save();
    }
    canvas.translate(1.0, 0.0);
    canvas.save();
    canvas.translate(1.0, 0.0);
    canvas.restore();
    assert_eq!(canvas.state().transform, Matrix::IDENTITY);

    let mut path = Path::new();
    let m = Matrix::IDENTITY;
    path.move_to(0.0, 0.0, &m);
    for i in 0..MAX_PATH_POINTS + 10 {
        path.line_to(i as f64, 1.0, &m);
    }
    assert_eq!(path.len(), MAX_PATH_POINTS);
}

#[test]
fn work_is_charged_by_area_and_points() {
    let mut canvas = Canvas::new(100, 100, false);
    canvas.take_work();
    canvas.fill_rect(0.0, 0.0, 100.0, 100.0);
    let big = canvas.take_work();
    canvas.fill_rect(0.0, 0.0, 4.0, 4.0);
    let small = canvas.take_work();
    assert!(big >= 625 && small < 20, "{big} {small}");
    canvas.fill_rect(-1e9, -1e9, 2e9, 2e9);
    assert!(canvas.take_work() <= 625 + 10);
}

#[test]
fn every_change_moves_the_generation_on() {
    let mut canvas = Canvas::new(4, 4, false);
    let mut last = canvas.generation();
    let mut moved = |canvas: &Canvas| {
        let now = canvas.generation();
        assert!(now > last);
        last = now;
    };
    canvas.fill_rect(0.0, 0.0, 1.0, 1.0);
    moved(&canvas);
    canvas.clear_rect(0.0, 0.0, 1.0, 1.0);
    moved(&canvas);
    canvas.put_image_data(&[0; 4], 1, 1, 0, 0, None);
    moved(&canvas);
    canvas.reset();
    moved(&canvas);
    let snapshot = canvas.snapshot().unwrap();
    assert_eq!((snapshot.width, snapshot.height), (4, 4));
}
