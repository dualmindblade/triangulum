//! Tensor quintic B-spline of (a-p)/(p*(1-p*p)), in coordinates (p*p, q*q).
//! Factoring the residual enforces mirrors and exact edge locations, while
//! using one scalar spline and its transpose enforces coordinate exchange.

const DATA: &[u8] = include_bytes!("../data/equal_area.bin");
const COBE_DATA: &[u8] = include_bytes!("../data/equal_area_cobe.bin");

const fn checked_intervals(data: &[u8]) -> usize {
    let intervals = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
    assert!(intervals >= 8);
    assert!(
        u64::from_le_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]
        ]) == u64::from_le_bytes(*b"CSEQAR01")
    );
    assert!(data.len() == 16 + 8 * (intervals + 1) * (intervals + 1));
    assert!(data[12] == 5 && data[13] == 0 && data[14] == 0 && data[15] == 0);
    intervals
}

const INTERVALS: usize = checked_intervals(DATA);
const COBE_INTERVALS: usize = checked_intervals(COBE_DATA);

pub(super) const fn intervals<const COBE: bool>() -> usize {
    if COBE { COBE_INTERVALS } else { INTERVALS }
}

#[inline]
fn coefficient<const COBE: bool>(i: usize, j: usize) -> f64 {
    let data = if COBE { COBE_DATA } else { DATA };
    let offset = 16 + 8 * (i * (intervals::<COBE>() + 1) + j);
    f64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

#[inline]
fn knot<const COBE: bool>(i: usize) -> f64 {
    if i <= 5 {
        0.0
    } else if i > intervals::<COBE>() {
        1.0
    } else {
        (i - 3) as f64 / intervals::<COBE>() as f64
    }
}

#[inline]
fn basis<const DERIV: bool, const COBE: bool>(x: f64) -> (usize, [f64; 6], [f64; 6]) {
    let span = ((x * intervals::<COBE>() as f64) as usize + 3).clamp(5, intervals::<COBE>());
    let mut n = [0.0; 6];
    let mut fourth = [0.0; 6];
    let mut left = [0.0; 6];
    let mut right = [0.0; 6];
    n[0] = 1.0;
    for j in 1..=5 {
        left[j] = x - knot::<COBE>(span + 1 - j);
        right[j] = knot::<COBE>(span + j) - x;
        let mut saved = 0.0;
        for r in 0..j {
            let temp = n[r] / (right[r + 1] + left[j - r]);
            n[r] = saved + right[r + 1] * temp;
            saved = left[j - r] * temp;
        }
        n[j] = saved;
        if DERIV && j == 4 {
            fourth = n;
        }
    }
    let mut d = [0.0; 6];
    if DERIV {
        for r in 0..6 {
            let i = span - 5 + r;
            if r > 0 {
                d[r] += 5.0 * fourth[r - 1] / (knot::<COBE>(i + 5) - knot::<COBE>(i));
            }
            if r < 5 {
                d[r] -= 5.0 * fourth[r] / (knot::<COBE>(i + 6) - knot::<COBE>(i + 1));
            }
        }
    }
    (span - 5, n, d)
}

/// (g, g_x, g_y), (h, h_y, h_x), with h(x,y) = g(y,x).
#[inline]
pub(super) fn residual<const DERIV: bool, const COBE: bool>(
    x: f64,
    y: f64,
) -> ([f64; 3], [f64; 3]) {
    let (ix, nx, dx) = basis::<DERIV, COBE>(x);
    let (iy, ny, dy) = basis::<DERIV, COBE>(y);
    let mut g = [0.0; 3];
    let mut h = [0.0; 3];
    for i in 0..6 {
        let mut row_g = 0.0;
        let mut row_h = 0.0;
        let mut row_gy = 0.0;
        let mut row_hx = 0.0;
        for j in 0..6 {
            let c = coefficient::<COBE>(ix + i, iy + j);
            let ct = coefficient::<COBE>(iy + i, ix + j);
            row_g += ny[j] * c;
            row_h += nx[j] * ct;
            if DERIV {
                row_gy += dy[j] * c;
                row_hx += dx[j] * ct;
            }
        }
        g[0] += nx[i] * row_g;
        h[0] += ny[i] * row_h;
        if DERIV {
            g[1] += dx[i] * row_g;
            g[2] += nx[i] * row_gy;
            h[1] += dy[i] * row_h;
            h[2] += ny[i] * row_hx;
        }
    }
    (g, h)
}
