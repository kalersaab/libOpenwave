use crate::video::h264::{frame::Yuv420Frame, motion::interpolation::half_sample_luma};

/// Interpolates one luma sample at a quarter-luma-pixel coordinate.
pub fn interpolate_luma_sample(frame: &Yuv420Frame, x_quarter: i64, y_quarter: i64) -> u8 {
    let x_quarter = x_quarter.clamp(-32, (i64::from(frame.width) + 8) * 4);
    let y_quarter = y_quarter.clamp(-32, (i64::from(frame.height) + 8) * 4);
    let x_phase = x_quarter.rem_euclid(4);
    let y_phase = y_quarter.rem_euclid(4);

    match (x_phase % 2, y_phase % 2) {
        (0, 0) => anchor(frame, x_quarter, y_quarter),
        (1, 0) => average(
            anchor(frame, x_quarter - 1, y_quarter),
            anchor(frame, x_quarter + 1, y_quarter),
        ),
        (0, 1) => average(
            anchor(frame, x_quarter, y_quarter - 1),
            anchor(frame, x_quarter, y_quarter + 1),
        ),
        (1, 1) => {
            let x = x_quarter.div_euclid(4);
            let y = y_quarter.div_euclid(4);
            let integer_x = x + i64::from(x_phase == 3);
            let integer_y = y + i64::from(y_phase == 3);
            let integer = half_sample_luma(frame, integer_x, integer_y, false, false);
            let diagonal = half_sample_luma(frame, x, y, true, true);
            average(integer, diagonal)
        }
        _ => unreachable!(),
    }
}

fn anchor(frame: &Yuv420Frame, x_quarter: i64, y_quarter: i64) -> u8 {
    let x_phase = x_quarter.rem_euclid(4);
    let y_phase = y_quarter.rem_euclid(4);
    debug_assert_eq!(x_phase % 2, 0);
    debug_assert_eq!(y_phase % 2, 0);
    half_sample_luma(
        frame,
        x_quarter.div_euclid(4),
        y_quarter.div_euclid(4),
        x_phase == 2,
        y_phase == 2,
    )
}

fn average(a: u8, b: u8) -> u8 {
    ((u16::from(a) + u16::from(b) + 1) >> 1) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_from_rows(rows: &[&[u8]]) -> Yuv420Frame {
        let width = rows[0].len();
        let height = rows.len();
        let mut frame = Yuv420Frame::new(width as u32, height as u32).unwrap();
        for (row, samples) in rows.iter().enumerate() {
            frame.y[row * width..(row + 1) * width].copy_from_slice(samples);
        }
        frame
    }

    #[test]
    fn six_tap_half_samples_match_known_filter_values() {
        let zero: &[u8] = &[0; 8];
        let impulse: &[u8] = &[0, 0, 0, 255, 0, 0, 0, 0];
        let frame = frame_from_rows(&[zero, zero, zero, impulse, zero, zero, zero, zero]);

        assert_eq!(interpolate_luma_sample(&frame, 4 * 3 + 2, 4 * 3), 159);
        assert_eq!(interpolate_luma_sample(&frame, 4 * 3, 4 * 3 + 2), 159);
        assert_eq!(interpolate_luma_sample(&frame, 4 * 3 + 2, 4 * 3 + 2), 100);
    }

    #[test]
    fn quarter_samples_round_between_integer_and_half_positions() {
        let row: &[u8] = &[0, 40, 80, 120, 160, 200, 240, 255];
        let frame = frame_from_rows(&[row; 8]);

        assert_eq!(interpolate_luma_sample(&frame, 12, 4), 120);
        assert_eq!(interpolate_luma_sample(&frame, 13, 4), 130);
        assert_eq!(interpolate_luma_sample(&frame, 14, 4), 140);
        assert_eq!(interpolate_luma_sample(&frame, 15, 4), 150);

        let mut diagonal = Yuv420Frame::new(8, 8).unwrap();
        for y in 0..8 {
            for x in 0..8 {
                diagonal.y[y * 8 + x] = (20 * x + 10 * y) as u8;
            }
        }
        assert_eq!(interpolate_luma_sample(&diagonal, 12, 13), 93);
        assert_eq!(interpolate_luma_sample(&diagonal, 13, 13), 98);
    }

    #[test]
    fn luma_interpolation_extends_edges_for_out_of_frame_motion() {
        let frame = frame_from_rows(&[&[7, 31], &[83, 127]]);

        assert_eq!(interpolate_luma_sample(&frame, -40, -40), 7);
        assert_eq!(interpolate_luma_sample(&frame, 100, 100), 127);
    }
}
