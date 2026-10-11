//! The Gaussian blur of a shadow (HTML 4.12.5.1.18, deviation half the
//! `shadowBlur`), approximated as Filter Effects says for
//! `feGaussianBlur`: three box blurs along each axis.

/// The box sizes, as (reach left, reach right) of each of the three
/// passes, for a deviation of `sigma`; none when the blur is too small
/// to show.
fn boxes(sigma: f64) -> Option<[(usize, usize); 3]> {
    let d = (sigma * 3.0 * (2.0 * std::f64::consts::PI).sqrt() / 4.0 + 0.5).floor() as usize;
    if d == 0 {
        return None;
    }
    let half = d / 2;
    Some(if d % 2 == 1 {
        [(half, half); 3]
    } else {
        // Two boxes of an even size, the first a pixel left of centre
        // and the second a pixel right, then one a pixel larger.
        [(half, half - 1), (half - 1, half), (half, half)]
    })
}

/// One pass of a box over `line`, outside which is nothing.
fn pass(line: &mut [f32], sums: &mut Vec<f64>, (left, right): (usize, usize)) {
    sums.clear();
    sums.push(0.0);
    let mut total = 0.0;
    for &v in line.iter() {
        total += f64::from(v);
        sums.push(total);
    }
    let n = line.len();
    let size = (left + right + 1) as f64;
    for (i, out) in line.iter_mut().enumerate() {
        let from = i.saturating_sub(left);
        let to = (i + right + 1).min(n);
        *out = ((sums[to] - sums[from]) / size) as f32;
    }
}

/// `alpha`, `width` by `height`, blurred with a deviation of `sigma`.
pub(crate) fn blur(alpha: &[u8], width: usize, height: usize, sigma: f64) -> Vec<u8> {
    let Some(boxes) = boxes(sigma).filter(|_| width > 0 && height > 0) else {
        return alpha.to_vec();
    };
    let mut values: Vec<f32> = alpha.iter().map(|&a| f32::from(a)).collect();
    let mut sums = Vec::with_capacity(width.max(height) + 1);
    for row in values.chunks_exact_mut(width) {
        for &reach in &boxes {
            pass(row, &mut sums, reach);
        }
    }
    let mut column = vec![0.0f32; height];
    for x in 0..width {
        for (y, v) in column.iter_mut().enumerate() {
            *v = values[y * width + x];
        }
        for &reach in &boxes {
            pass(&mut column, &mut sums, reach);
        }
        for (y, &v) in column.iter().enumerate() {
            values[y * width + x] = v;
        }
    }
    values
        .into_iter()
        .map(|v| v.round().clamp(0.0, 255.0) as u8)
        .collect()
}
