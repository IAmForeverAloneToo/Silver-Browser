//! Shadows and blurs (CSS Backgrounds 3 section 7, Filter Effects 1
//! section 13; gap batch E, D52, `docs/design/boxes-colour.md`): the
//! coverage of a box's shadow, worked out once by code both backends
//! share, so that they draw the same pixels.
//!
//! A shadow is its shape's coverage blurred by a Gaussian of standard
//! deviation half the blur radius (section 7.1.1), made as three box
//! blurs along each axis, as the reference browser's raster makes it,
//! then cut to outside the box (an outer shadow) or inside its padding
//! box (an inset one).

use crate::{Clip, Coverage, PixelRect, contours_coverage};
use paint::rounded::RoundedRect;

/// The largest standard deviation a blur is worked out at, in device
/// pixels: past it, the shadow is as soft as it gets, and the work, a
/// region three of them beyond the shape on each side, stays bounded.
pub const MAX_SIGMA: f32 = 256.0;

/// The widths of the three box blurs whose succession approximates a
/// Gaussian of standard deviation `sigma` (Kovesi's "fast almost
/// Gaussian filtering"): odd, two sizes, the mean variance `sigma`'s.
pub(crate) fn boxes(sigma: f32) -> [usize; 3] {
    let s = f64::from(sigma);
    let ideal = (12.0 * s * s / 3.0 + 1.0).sqrt();
    let mut lower = ideal.floor() as i64;
    if lower % 2 == 0 {
        lower -= 1;
    }
    let lower = lower.max(1);
    let upper = lower + 2;
    let m = ((12.0 * s * s - 3.0 * (lower * lower) as f64 - 12.0 * lower as f64 - 9.0)
        / (-4.0 * lower as f64 - 4.0))
        .round() as i64;
    [0, 1, 2].map(|i| {
        if (i as i64) < m {
            lower as usize
        } else {
            upper as usize
        }
    })
}

/// One box blur of `width` (odd) along rows, values outside the buffer
/// `pad`.
fn blur_rows(src: &[f32], dst: &mut [f32], w: usize, h: usize, width: usize, pad: f32) {
    let r = width / 2;
    let scale = 1.0 / width as f32;
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        let at = |i: isize| -> f32 {
            if i < 0 || i as usize >= w {
                pad
            } else {
                row[i as usize]
            }
        };
        let mut sum: f32 = (-(r as isize)..=r as isize).map(at).sum();
        for x in 0..w {
            dst[y * w + x] = sum * scale;
            sum += at(x as isize + r as isize + 1) - at(x as isize - r as isize);
        }
    }
}

/// [`blur_rows`] along columns.
fn blur_columns(src: &[f32], dst: &mut [f32], w: usize, h: usize, width: usize, pad: f32) {
    let r = width / 2;
    let scale = 1.0 / width as f32;
    for x in 0..w {
        let at = |i: isize| -> f32 {
            if i < 0 || i as usize >= h {
                pad
            } else {
                src[i as usize * w + x]
            }
        };
        let mut sum: f32 = (-(r as isize)..=r as isize).map(at).sum();
        for y in 0..h {
            dst[y * w + x] = sum * scale;
            sum += at(y as isize + r as isize + 1) - at(y as isize - r as isize);
        }
    }
}

/// `values`, `w` by `h`, blurred in place by a Gaussian of standard
/// deviation `sigma`, what lies outside them taken as `pad`.
pub(crate) fn gaussian(values: &mut [f32], w: usize, h: usize, sigma: f32, pad: f32) {
    if sigma <= 0.0 || w == 0 || h == 0 {
        return;
    }
    let mut other = vec![0.0f32; values.len()];
    for width in boxes(sigma.min(MAX_SIGMA)) {
        if width <= 1 {
            continue;
        }
        blur_rows(values, &mut other, w, h, width, pad);
        blur_columns(&other, values, w, h, width, pad);
    }
}

/// How far a blur of `sigma` reaches: three standard deviations, whole.
pub(crate) fn reach(sigma: f32) -> f32 {
    (3.0 * sigma.clamp(0.0, MAX_SIGMA)).ceil()
}

/// `r` moved by `(dx, dy)`.
fn moved(r: &RoundedRect, dx: f32, dy: f32) -> RoundedRect {
    let mut out = *r;
    out.rect.x += dx;
    out.rect.y += dy;
    out
}

/// The coverage of `shape` in a `w` by `h` buffer whose origin is at
/// `(x0, y0)`.
fn shape_values(shape: &RoundedRect, x0: f32, y0: f32, w: usize, h: usize) -> Vec<f32> {
    let mut values = vec![0.0f32; w * h];
    let local = moved(shape, -x0, -y0);
    let clip = Clip::new(0.0, 0.0, w as f32, h as f32);
    if let Some(c) = contours_coverage(&[local.contour()], clip, (w as u32, h as u32)) {
        for row in 0..c.height {
            for col in 0..c.width {
                let (x, y) = (c.x0 as usize + col, c.y0 as usize + row);
                values[y * w + x] = c.values[row * c.width + col];
            }
        }
    }
    values
}

/// The coverage of a shadow in a `size` raster, under `clip`: `shape`,
/// in raster coordinates, blurred by `sigma` device pixels and cut to
/// outside `boxed` (an outer shadow) or, `inset`, its hole blurred and
/// cut to inside `boxed`. `None` where nothing shows.
pub(crate) fn shadow_coverage(
    shape: &RoundedRect,
    sigma: f32,
    inset: bool,
    boxed: &RoundedRect,
    clip: Clip,
    size: (u32, u32),
) -> Option<Coverage> {
    let sigma = sigma.clamp(0.0, MAX_SIGMA);
    let e = reach(sigma);
    let frame = PixelRect {
        x0: 0,
        y0: 0,
        x1: size.0,
        y1: size.1,
    };
    let visible = clip.pixels(size.0, size.1)?.intersect(frame)?;
    // What shows: around the shape as far as the blur reaches, or inside
    // the box for an inset shadow.
    let around = if inset { boxed.rect } else { shape.rect };
    let reach_x0 = around.x - if inset { 0.0 } else { e };
    let reach_y0 = around.y - if inset { 0.0 } else { e };
    let reach_x1 = around.x + around.w + if inset { 0.0 } else { e };
    let reach_y1 = around.y + around.h + if inset { 0.0 } else { e };
    let shows = PixelRect::around(reach_x0, reach_y0, reach_x1, reach_y1, size.0, size.1)?
        .intersect(visible)?;
    // The work: what shows, and the blur's reach beyond it.
    let (wx0, wy0) = (shows.x0 as f32 - e, shows.y0 as f32 - e);
    let (w, h) = (
        (shows.x1 - shows.x0) as usize + 2 * e as usize,
        (shows.y1 - shows.y0) as usize + 2 * e as usize,
    );
    let mut values = shape_values(shape, wx0, wy0, w, h);
    if inset {
        for v in &mut values {
            *v = 1.0 - *v;
        }
    }
    gaussian(&mut values, w, h, sigma, if inset { 1.0 } else { 0.0 });
    let cut = shape_values(boxed, wx0, wy0, w, h);
    let (sw, sh) = (
        (shows.x1 - shows.x0) as usize,
        (shows.y1 - shows.y0) as usize,
    );
    let mut out = vec![0.0f32; sw * sh];
    let mut any = false;
    for row in 0..sh {
        for col in 0..sw {
            let at = (row + e as usize) * w + col + e as usize;
            let box_cover = cut[at];
            let v = values[at].clamp(0.0, 1.0) * if inset { box_cover } else { 1.0 - box_cover };
            any |= v > 0.0;
            out[row * sw + col] = v;
        }
    }
    any.then_some(Coverage {
        x0: i64::from(shows.x0),
        y0: i64::from(shows.y0),
        width: sw,
        height: sh,
        values: out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_boxes_have_the_gaussians_variance() {
        for sigma in [0.5f32, 1.0, 2.0, 5.0, 12.5, 40.0] {
            let b = boxes(sigma);
            let variance: f64 = b.iter().map(|&w| ((w * w) as f64 - 1.0) / 12.0).sum();
            assert!(b.iter().all(|w| w % 2 == 1), "{b:?}");
            assert!(
                (variance.sqrt() - f64::from(sigma)).abs() < 0.6,
                "{sigma}: {b:?} {}",
                variance.sqrt()
            );
        }
    }

    #[test]
    fn a_blur_keeps_its_total_and_spreads_an_edge() {
        let (w, h) = (40usize, 1usize);
        let mut v: Vec<f32> = (0..w).map(|x| if x < 20 { 1.0 } else { 0.0 }).collect();
        let before: f32 = v.iter().sum();
        gaussian(&mut v, w, h, 3.0, 0.0);
        // Rows alone here, the columns of one row padded with zero: the
        // total shrinks by the columns' share, a third at each pass.
        assert!(v[19] > 0.0 && v[19] < 1.0 && v[20] > 0.0);
        assert!(v[0] < before);
        // A wide buffer padded with what it holds keeps its values.
        let mut flat = vec![1.0f32; 64];
        gaussian(&mut flat, 8, 8, 2.0, 1.0);
        assert!(flat.iter().all(|v| (v - 1.0).abs() < 1e-5));
    }

    #[test]
    fn an_outer_shadow_shows_outside_the_box_alone() {
        let r = |x: f32, y: f32, w: f32, h: f32| RoundedRect {
            rect: paint::Rect { x, y, w, h },
            radii: [(0.0, 0.0); 4],
        };
        let boxed = r(20.0, 20.0, 40.0, 40.0);
        let shape = r(25.0, 25.0, 40.0, 40.0);
        let c = shadow_coverage(
            &shape,
            2.0,
            false,
            &boxed,
            Clip::new(0.0, 0.0, 100.0, 100.0),
            (100, 100),
        )
        .expect("a shadow");
        let at = |x: i64, y: i64| c.values[((y - c.y0) as usize) * c.width + (x - c.x0) as usize];
        assert_eq!(at(40, 40), 0.0, "under the box");
        assert!(at(62, 40) > 0.9, "beside it, in the shadow");
        assert!(at(66, 40) > 0.0 && at(66, 40) < 0.6, "its blurred edge");
        assert!(c.x0 >= 0 && c.x0 + c.width as i64 <= 100);
        // An inset one inside the box alone.
        let shape = r(30.0, 30.0, 30.0, 30.0);
        let c = shadow_coverage(
            &shape,
            0.0,
            true,
            &boxed,
            Clip::new(0.0, 0.0, 100.0, 100.0),
            (100, 100),
        )
        .expect("a shadow");
        let at = |x: i64, y: i64| c.values[((y - c.y0) as usize) * c.width + (x - c.x0) as usize];
        assert_eq!(at(25, 40), 1.0);
        assert_eq!(at(45, 45), 0.0);
        assert!(c.x0 >= 20 && c.x0 + c.width as i64 <= 60);
    }
}
