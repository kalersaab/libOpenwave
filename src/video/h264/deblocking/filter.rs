use super::tables::{ALPHA_TABLE, BETA_TABLE, TC0_TABLE};

fn clip3(min: i32, max: i32, value: i32) -> i32 {
    value.max(min).min(max)
}

pub fn filter_luma_edge_bs4(
    pixels: &mut [u8],
    stride: usize,
    alpha: u8,
    beta: u8,
    is_vertical: bool,
) {
    for i in 0..4 {
        if is_vertical {
            filter_luma_sample_bs4_vertical(pixels, stride, i, alpha, beta);
        } else {
            filter_luma_sample_bs4_horizontal(pixels, stride, i, alpha, beta);
        }
    }
}

fn filter_luma_sample_bs4_vertical(
    pixels: &mut [u8],
    stride: usize,
    offset: usize,
    alpha: u8,
    beta: u8,
) {
    let row = offset * stride;
    let p3 = i32::from(pixels[row]);
    let p2 = i32::from(pixels[row + 1]);
    let p1 = i32::from(pixels[row + 2]);
    let p0 = i32::from(pixels[row + 3]);
    let q0 = i32::from(pixels[row + 4]);
    let q1 = i32::from(pixels[row + 5]);
    let q2 = i32::from(pixels[row + 6]);
    let q3 = i32::from(pixels[row + 7]);

    let alpha = i32::from(alpha);
    let beta = i32::from(beta);

    if (p0 - q0).abs() >= alpha || (p1 - p0).abs() >= beta || (q1 - q0).abs() >= beta {
        return;
    }

    let ap = (p2 - p0).abs();
    let aq = (q2 - q0).abs();

    if ap < beta {
        let p_new = (p2 + 2 * p1 + 2 * p0 + 2 * q0 + q1 + 4) >> 3;
        pixels[row + 2] = p_new as u8;

        if ap < (beta >> 2) {
            let p2_new = (2 * p3 + 3 * p2 + p1 + p0 + q0 + 4) >> 3;
            pixels[row + 1] = p2_new as u8;
        }
    } else {
        let p_new = (2 * p1 + p0 + q1 + 2) >> 2;
        pixels[row + 2] = p_new as u8;
    }

    if aq < beta {
        let q_new = (p1 + 2 * p0 + 2 * q0 + 2 * q1 + q2 + 4) >> 3;
        pixels[row + 5] = q_new as u8;

        if aq < (beta >> 2) {
            let q2_new = (p0 + q0 + q1 + 3 * q2 + 2 * q3 + 4) >> 3;
            pixels[row + 6] = q2_new as u8;
        }
    } else {
        let q_new = (2 * q1 + q0 + p1 + 2) >> 2;
        pixels[row + 5] = q_new as u8;
    }

    let p0_new = (p2 + 2 * p1 + 2 * p0 + 2 * q0 + q1 + 4) >> 3;
    let q0_new = (p1 + 2 * p0 + 2 * q0 + 2 * q1 + q2 + 4) >> 3;
    pixels[row + 3] = p0_new as u8;
    pixels[row + 4] = q0_new as u8;
}

fn filter_luma_sample_bs4_horizontal(
    pixels: &mut [u8],
    stride: usize,
    offset: usize,
    alpha: u8,
    beta: u8,
) {
    let p3 = i32::from(pixels[offset]);
    let p2 = i32::from(pixels[offset + stride]);
    let p1 = i32::from(pixels[offset + 2 * stride]);
    let p0 = i32::from(pixels[offset + 3 * stride]);
    let q0 = i32::from(pixels[offset + 4 * stride]);
    let q1 = i32::from(pixels[offset + 5 * stride]);
    let q2 = i32::from(pixels[offset + 6 * stride]);
    let q3 = i32::from(pixels[offset + 7 * stride]);

    let alpha = i32::from(alpha);
    let beta = i32::from(beta);

    if (p0 - q0).abs() >= alpha || (p1 - p0).abs() >= beta || (q1 - q0).abs() >= beta {
        return;
    }

    let ap = (p2 - p0).abs();
    let aq = (q2 - q0).abs();

    if ap < beta {
        let p_new = (p2 + 2 * p1 + 2 * p0 + 2 * q0 + q1 + 4) >> 3;
        pixels[offset + 2 * stride] = p_new as u8;

        if ap < (beta >> 2) {
            let p2_new = (2 * p3 + 3 * p2 + p1 + p0 + q0 + 4) >> 3;
            pixels[offset + stride] = p2_new as u8;
        }
    } else {
        let p_new = (2 * p1 + p0 + q1 + 2) >> 2;
        pixels[offset + 2 * stride] = p_new as u8;
    }

    if aq < beta {
        let q_new = (p1 + 2 * p0 + 2 * q0 + 2 * q1 + q2 + 4) >> 3;
        pixels[offset + 5 * stride] = q_new as u8;

        if aq < (beta >> 2) {
            let q2_new = (p0 + q0 + q1 + 3 * q2 + 2 * q3 + 4) >> 3;
            pixels[offset + 6 * stride] = q2_new as u8;
        }
    } else {
        let q_new = (2 * q1 + q0 + p1 + 2) >> 2;
        pixels[offset + 5 * stride] = q_new as u8;
    }

    let p0_new = (p2 + 2 * p1 + 2 * p0 + 2 * q0 + q1 + 4) >> 3;
    let q0_new = (p1 + 2 * p0 + 2 * q0 + 2 * q1 + q2 + 4) >> 3;
    pixels[offset + 3 * stride] = p0_new as u8;
    pixels[offset + 4 * stride] = q0_new as u8;
}

pub fn filter_luma_edge_bs_lt4(
    pixels: &mut [u8],
    stride: usize,
    bs: u8,
    qp: u8,
    is_vertical: bool,
) {
    let index_a = clip3(0, 51, i32::from(qp)) as usize;
    let alpha = ALPHA_TABLE[index_a];
    let beta = BETA_TABLE[index_a];
    let tc0 = TC0_TABLE[index_a][(bs - 1) as usize];
    let tc = i32::from(tc0) + 1;

    for i in 0..4 {
        if is_vertical {
            filter_luma_sample_bs_lt4_vertical(pixels, stride, i, alpha, beta, tc);
        } else {
            filter_luma_sample_bs_lt4_horizontal(pixels, stride, i, alpha, beta, tc);
        }
    }
}

fn filter_luma_sample_bs_lt4_vertical(
    pixels: &mut [u8],
    stride: usize,
    offset: usize,
    alpha: u8,
    beta: u8,
    tc: i32,
) {
    let row = offset * stride;
    let p2 = i32::from(pixels[row + 1]);
    let p1 = i32::from(pixels[row + 2]);
    let p0 = i32::from(pixels[row + 3]);
    let q0 = i32::from(pixels[row + 4]);
    let q1 = i32::from(pixels[row + 5]);
    let q2 = i32::from(pixels[row + 6]);

    let alpha = i32::from(alpha);
    let beta = i32::from(beta);

    if (p0 - q0).abs() >= alpha || (p1 - p0).abs() >= beta || (q1 - q0).abs() >= beta {
        return;
    }

    let ap = (p2 - p0).abs();
    let aq = (q2 - q0).abs();

    let tc_p = if ap < beta { tc + 1 } else { tc };
    let _tc_q = if aq < beta { tc + 1 } else { tc };

    let delta = clip3(-tc_p, tc_p, ((q0 - p0) * 4 + (p1 - q1) + 4) >> 3);

    let p0_new = clip3(0, 255, p0 + delta);
    let q0_new = clip3(0, 255, q0 - delta);
    pixels[row + 3] = p0_new as u8;
    pixels[row + 4] = q0_new as u8;

    if ap < beta {
        let delta_p = clip3(-tc, tc, (p2 + ((p0 + q0 + 1) >> 1) - (p1 << 1)) >> 1);
        let p1_new = clip3(0, 255, p1 + delta_p);
        pixels[row + 2] = p1_new as u8;
    }

    if aq < beta {
        let delta_q = clip3(-tc, tc, (q2 + ((p0 + q0 + 1) >> 1) - (q1 << 1)) >> 1);
        let q1_new = clip3(0, 255, q1 + delta_q);
        pixels[row + 5] = q1_new as u8;
    }
}

fn filter_luma_sample_bs_lt4_horizontal(
    pixels: &mut [u8],
    stride: usize,
    offset: usize,
    alpha: u8,
    beta: u8,
    tc: i32,
) {
    let p2 = i32::from(pixels[offset + stride]);
    let p1 = i32::from(pixels[offset + 2 * stride]);
    let p0 = i32::from(pixels[offset + 3 * stride]);
    let q0 = i32::from(pixels[offset + 4 * stride]);
    let q1 = i32::from(pixels[offset + 5 * stride]);
    let q2 = i32::from(pixels[offset + 6 * stride]);

    let alpha = i32::from(alpha);
    let beta = i32::from(beta);

    if (p0 - q0).abs() >= alpha || (p1 - p0).abs() >= beta || (q1 - q0).abs() >= beta {
        return;
    }

    let ap = (p2 - p0).abs();
    let aq = (q2 - q0).abs();

    let tc_p = if ap < beta { tc + 1 } else { tc };
    let _tc_q = if aq < beta { tc + 1 } else { tc };

    let delta = clip3(-tc_p, tc_p, ((q0 - p0) * 4 + (p1 - q1) + 4) >> 3);

    let p0_new = clip3(0, 255, p0 + delta);
    let q0_new = clip3(0, 255, q0 - delta);
    pixels[offset + 3 * stride] = p0_new as u8;
    pixels[offset + 4 * stride] = q0_new as u8;

    if ap < beta {
        let delta_p = clip3(-tc, tc, (p2 + ((p0 + q0 + 1) >> 1) - (p1 << 1)) >> 1);
        let p1_new = clip3(0, 255, p1 + delta_p);
        pixels[offset + 2 * stride] = p1_new as u8;
    }

    if aq < beta {
        let delta_q = clip3(-tc, tc, (q2 + ((p0 + q0 + 1) >> 1) - (q1 << 1)) >> 1);
        let q1_new = clip3(0, 255, q1 + delta_q);
        pixels[offset + 5 * stride] = q1_new as u8;
    }
}

pub fn filter_chroma_edge(pixels: &mut [u8], stride: usize, bs: u8, qp: u8, is_vertical: bool) {
    if bs == 0 {
        return;
    }

    let index_a = clip3(0, 51, i32::from(qp)) as usize;
    let alpha = ALPHA_TABLE[index_a];
    let beta = BETA_TABLE[index_a];
    let tc0 = TC0_TABLE[index_a][(bs.min(3) - 1) as usize];
    let tc = i32::from(tc0) + 1;

    for i in 0..2 {
        if is_vertical {
            filter_chroma_sample_vertical(pixels, stride, i * 2, alpha, beta, tc);
        } else {
            filter_chroma_sample_horizontal(pixels, stride, i * 2, alpha, beta, tc);
        }
    }
}

fn filter_chroma_sample_vertical(
    pixels: &mut [u8],
    stride: usize,
    offset: usize,
    alpha: u8,
    beta: u8,
    tc: i32,
) {
    let row = offset * stride;
    let p1 = i32::from(pixels[row + 2]);
    let p0 = i32::from(pixels[row + 3]);
    let q0 = i32::from(pixels[row + 4]);
    let q1 = i32::from(pixels[row + 5]);

    let alpha = i32::from(alpha);
    let beta = i32::from(beta);

    if (p0 - q0).abs() >= alpha || (p1 - p0).abs() >= beta || (q1 - q0).abs() >= beta {
        return;
    }

    let delta = clip3(-tc, tc, ((q0 - p0) * 4 + (p1 - q1) + 4) >> 3);

    let p0_new = clip3(0, 255, p0 + delta);
    let q0_new = clip3(0, 255, q0 - delta);
    pixels[row + 3] = p0_new as u8;
    pixels[row + 4] = q0_new as u8;
}

fn filter_chroma_sample_horizontal(
    pixels: &mut [u8],
    stride: usize,
    offset: usize,
    alpha: u8,
    beta: u8,
    tc: i32,
) {
    let p1 = i32::from(pixels[offset + 2 * stride]);
    let p0 = i32::from(pixels[offset + 3 * stride]);
    let q0 = i32::from(pixels[offset + 4 * stride]);
    let q1 = i32::from(pixels[offset + 5 * stride]);

    let alpha = i32::from(alpha);
    let beta = i32::from(beta);

    if (p0 - q0).abs() >= alpha || (p1 - p0).abs() >= beta || (q1 - q0).abs() >= beta {
        return;
    }

    let delta = clip3(-tc, tc, ((q0 - p0) * 4 + (p1 - q1) + 4) >> 3);

    let p0_new = clip3(0, 255, p0 + delta);
    let q0_new = clip3(0, 255, q0 - delta);
    pixels[offset + 3 * stride] = p0_new as u8;
    pixels[offset + 4 * stride] = q0_new as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip3_works_correctly() {
        assert_eq!(clip3(0, 255, -10), 0);
        assert_eq!(clip3(0, 255, 128), 128);
        assert_eq!(clip3(0, 255, 300), 255);
    }

    #[test]
    fn filter_reduces_discontinuity() {
        let mut pixels = vec![0u8; 16];
        pixels.iter_mut().take(8).enumerate().for_each(|(i, p)| {
            *p = if i < 4 { 100 } else { 150 };
        });

        let diff_before = (i32::from(pixels[3]) - i32::from(pixels[4])).abs();
        filter_luma_sample_bs_lt4_vertical(&mut pixels, 1, 0, 60, 10, 2);
        let diff_after = (i32::from(pixels[3]) - i32::from(pixels[4])).abs();

        assert!(
            diff_after < diff_before,
            "Filter should reduce difference: before={}, after={}",
            diff_before,
            diff_after
        );
    }

    #[test]
    fn filter_respects_alpha_threshold() {
        let mut pixels = vec![0u8; 16];
        pixels[3] = 0;
        pixels[4] = 255;

        let original = pixels.clone();
        filter_luma_sample_bs_lt4_vertical(&mut pixels, 1, 0, 10, 5, 2);

        assert_eq!(pixels, original);
    }

    #[test]
    fn chroma_filter_reduces_discontinuity() {
        let mut pixels = vec![0u8; 16];
        pixels.iter_mut().take(8).enumerate().for_each(|(i, p)| {
            *p = if i < 4 { 100 } else { 150 };
        });

        let diff_before = (i32::from(pixels[3]) - i32::from(pixels[4])).abs();
        filter_chroma_sample_vertical(&mut pixels, 1, 0, 60, 10, 2);
        let diff_after = (i32::from(pixels[3]) - i32::from(pixels[4])).abs();

        assert!(
            diff_after < diff_before,
            "Filter should reduce difference: before={}, after={}",
            diff_before,
            diff_after
        );
    }
}
