use crate::video::h264::frame::Yuv420Frame;

pub struct RgbFrame {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl RgbFrame {
    pub fn new(width: usize, height: usize) -> Self {
        RgbFrame {
            width,
            height,
            data: vec![0; width * height * 3],
        }
    }

    pub fn from_yuv(yuv: &Yuv420Frame) -> Self {
        let width = yuv.width as usize;
        let height = yuv.height as usize;
        let mut rgb = RgbFrame::new(width, height);

        yuv_to_rgb_420(&yuv.y, &yuv.u, &yuv.v, width, height, &mut rgb.data);

        rgb
    }

    pub fn save_ppm(&self, path: &str) -> std::io::Result<()> {
        use std::fs::File;
        use std::io::Write;

        let mut file = File::create(path)?;

        writeln!(file, "P6")?;
        writeln!(file, "{} {}", self.width, self.height)?;
        writeln!(file, "255")?;
        file.write_all(&self.data)?;

        Ok(())
    }
}

pub fn yuv_to_rgb_420(
    y_plane: &[u8],
    u_plane: &[u8],
    v_plane: &[u8],
    width: usize,
    height: usize,
    rgb_output: &mut [u8],
) {
    let stride_y = width;
    let stride_uv = width.div_ceil(2);

    for y in 0..height {
        for x in 0..width {
            let y_idx = y * stride_y + x;
            let uv_y = y / 2;
            let uv_x = x / 2;
            let uv_idx = uv_y * stride_uv + uv_x;

            let y_val = y_plane[y_idx] as i32;
            let u_val = u_plane[uv_idx] as i32 - 128;
            let v_val = v_plane[uv_idx] as i32 - 128;

            let r = (y_val + (1.402 * v_val as f32) as i32).clamp(0, 255) as u8;
            let g = (y_val - (0.344 * u_val as f32) as i32 - (0.714 * v_val as f32) as i32)
                .clamp(0, 255) as u8;
            let b = (y_val + (1.772 * u_val as f32) as i32).clamp(0, 255) as u8;

            let rgb_idx = (y * width + x) * 3;
            rgb_output[rgb_idx] = r;
            rgb_output[rgb_idx + 1] = g;
            rgb_output[rgb_idx + 2] = b;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::h264::frame::Yuv420Frame;

    #[test]
    fn converts_black_yuv_to_rgb() {
        let y = vec![16; 64];
        let u = vec![128; 16];
        let v = vec![128; 16];
        let mut rgb = vec![0u8; 64 * 3];

        yuv_to_rgb_420(&y, &u, &v, 8, 8, &mut rgb);

        for pixel in rgb.chunks(3) {
            assert!(pixel[0] < 20);
            assert!(pixel[1] < 20);
            assert!(pixel[2] < 20);
        }
    }

    #[test]
    fn converts_white_yuv_to_rgb() {
        let y = vec![235; 64];
        let u = vec![128; 16];
        let v = vec![128; 16];
        let mut rgb = vec![0u8; 64 * 3];

        yuv_to_rgb_420(&y, &u, &v, 8, 8, &mut rgb);

        for pixel in rgb.chunks(3) {
            assert!(pixel[0] > 230);
            assert!(pixel[1] > 230);
            assert!(pixel[2] > 230);
        }
    }

    #[test]
    fn converts_red_yuv_to_rgb() {
        let y = vec![81; 64];
        let u = vec![90; 16];
        let v = vec![240; 16];
        let mut rgb = vec![0u8; 64 * 3];

        yuv_to_rgb_420(&y, &u, &v, 8, 8, &mut rgb);

        let first_pixel = &rgb[0..3];
        assert!(first_pixel[0] > 200);
        assert!(first_pixel[1] < 50);
        assert!(first_pixel[2] < 50);
    }

    #[test]
    fn creates_rgb_frame_from_yuv() {
        let yuv = Yuv420Frame::new(16, 16).unwrap();

        let rgb = RgbFrame::from_yuv(&yuv);

        assert_eq!(rgb.width, 16);
        assert_eq!(rgb.height, 16);
        assert_eq!(rgb.data.len(), 16 * 16 * 3);
    }
}
