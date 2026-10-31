use crate::video::h264::frame::{ColorPlane, Yuv420Frame};

const SIX_TAP: [i32; 6] = [1, -5, 20, 20, -5, 1];

pub(super) fn edge_sample(samples: &[u8], width: usize, height: usize, x: i64, y: i64) -> i32 {
    let x = x.clamp(0, width as i64 - 1) as usize;
    let y = y.clamp(0, height as i64 - 1) as usize;
    i32::from(samples[y * width + x])
}

pub(super) fn half_sample_luma(
    frame: &Yuv420Frame,
    x: i64,
    y: i64,
    half_x: bool,
    half_y: bool,
) -> u8 {
    let width = frame.width as usize;
    let height = frame.height as usize;
    let samples = &frame.y;

    match (half_x, half_y) {
        (false, false) => edge_sample(samples, width, height, x, y) as u8,
        (true, false) => {
            let sum = SIX_TAP
                .iter()
                .enumerate()
                .map(|(index, &coefficient)| {
                    coefficient * edge_sample(samples, width, height, x + index as i64 - 2, y)
                })
                .sum::<i32>();
            ((sum + 16) >> 5).clamp(0, 255) as u8
        }
        (false, true) => {
            let sum = SIX_TAP
                .iter()
                .enumerate()
                .map(|(index, &coefficient)| {
                    coefficient * edge_sample(samples, width, height, x, y + index as i64 - 2)
                })
                .sum::<i32>();
            ((sum + 16) >> 5).clamp(0, 255) as u8
        }
        (true, true) => {
            let mut sum = 0i32;
            for (row, &vertical) in SIX_TAP.iter().enumerate() {
                for (column, &horizontal) in SIX_TAP.iter().enumerate() {
                    sum += horizontal
                        * vertical
                        * edge_sample(
                            samples,
                            width,
                            height,
                            x + column as i64 - 2,
                            y + row as i64 - 2,
                        );
                }
            }
            ((sum + 512) >> 10).clamp(0, 255) as u8
        }
    }
}

pub(super) fn interpolate_bilinear(
    samples: &[u8],
    width: usize,
    height: usize,
    x_eighth: i64,
    y_eighth: i64,
) -> u8 {
    let x0 = x_eighth.div_euclid(8);
    let y0 = y_eighth.div_euclid(8);
    let fx = x_eighth.rem_euclid(8);
    let fy = y_eighth.rem_euclid(8);
    let a = edge_sample(samples, width, height, x0, y0);
    let b = edge_sample(samples, width, height, x0 + 1, y0);
    let c = edge_sample(samples, width, height, x0, y0 + 1);
    let d = edge_sample(samples, width, height, x0 + 1, y0 + 1);
    let weighted = a * (8 - fx) as i32 * (8 - fy) as i32
        + b * fx as i32 * (8 - fy) as i32
        + c * (8 - fx) as i32 * fy as i32
        + d * fx as i32 * fy as i32;
    ((weighted + 32) >> 6).clamp(0, 255) as u8
}

pub(super) fn plane_samples(frame: &Yuv420Frame, plane: ColorPlane) -> (&[u8], usize, usize) {
    let (width, height) = frame.plane_dimensions(plane);
    let samples = match plane {
        ColorPlane::Y => &frame.y,
        ColorPlane::U => &frame.u,
        ColorPlane::V => &frame.v,
    };
    (samples, width as usize, height as usize)
}
