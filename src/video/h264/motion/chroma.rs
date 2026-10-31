use crate::video::h264::{
    frame::{ColorPlane, Yuv420Frame},
    motion::interpolation::{interpolate_bilinear, plane_samples},
};

pub fn interpolate_chroma_sample(
    frame: &Yuv420Frame,
    plane: ColorPlane,
    x: u32,
    y: u32,
    motion_x_quarter: i32,
    motion_y_quarter: i32,
) -> u8 {
    let (samples, width, height) = plane_samples(frame, plane);
    let x_eighth = i64::from(x) * 8 + i64::from(motion_x_quarter);
    let y_eighth = i64::from(y) * 8 + i64::from(motion_y_quarter);
    interpolate_bilinear(samples, width, height, x_eighth, y_eighth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chroma_bilinear_values_match_known_fractional_positions() {
        let mut frame = Yuv420Frame::new(4, 4).unwrap();
        frame.u.copy_from_slice(&[0, 80, 160, 240]);
        frame.v.copy_from_slice(&[20, 100, 180, 255]);

        assert_eq!(
            interpolate_chroma_sample(&frame, ColorPlane::U, 0, 0, 2, 0),
            20
        );
        assert_eq!(
            interpolate_chroma_sample(&frame, ColorPlane::U, 0, 0, 4, 4),
            120
        );
        assert_eq!(
            interpolate_chroma_sample(&frame, ColorPlane::V, 0, 0, 2, 2),
            80
        );
    }

    #[test]
    fn chroma_bilinear_interpolation_extends_edges() {
        let mut frame = Yuv420Frame::new(4, 4).unwrap();
        frame.u.copy_from_slice(&[11, 22, 33, 44]);

        assert_eq!(
            interpolate_chroma_sample(&frame, ColorPlane::U, 0, 0, -16, -16),
            11
        );
        assert_eq!(
            interpolate_chroma_sample(&frame, ColorPlane::U, 1, 1, 16, 16),
            44
        );
    }
}
