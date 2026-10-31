#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    InvalidDimensions,
    DimensionOverflow,
    AllocationFailed,
    BlockOutOfBounds,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "H.264 frame error: {self:?}")
    }
}

impl std::error::Error for FrameError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorPlane {
    Y,
    U,
    V,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Yuv420Frame {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

impl Yuv420Frame {
    pub fn new(width: u32, height: u32) -> Result<Self, FrameError> {
        if width == 0 || height == 0 {
            return Err(FrameError::InvalidDimensions);
        }

        let chroma_width = width.div_ceil(2);
        let chroma_height = height.div_ceil(2);
        let y_len = plane_len(width, height)?;
        let chroma_len = plane_len(chroma_width, chroma_height)?;

        let mut y = Vec::new();
        y.try_reserve_exact(y_len)
            .map_err(|_| FrameError::AllocationFailed)?;
        y.resize(y_len, 0);

        let mut u = Vec::new();
        u.try_reserve_exact(chroma_len)
            .map_err(|_| FrameError::AllocationFailed)?;
        u.resize(chroma_len, 128);

        let mut v = Vec::new();
        v.try_reserve_exact(chroma_len)
            .map_err(|_| FrameError::AllocationFailed)?;
        v.resize(chroma_len, 128);

        Ok(Self {
            width,
            height,
            y,
            u,
            v,
        })
    }

    pub fn plane_dimensions(&self, plane: ColorPlane) -> (u32, u32) {
        match plane {
            ColorPlane::Y => (self.width, self.height),
            ColorPlane::U | ColorPlane::V => (self.width.div_ceil(2), self.height.div_ceil(2)),
        }
    }

    /// Writes a predicted 4x4 block plus its inverse-transformed residual.
    /// The prediction and residual are both row-major.
    pub fn reconstruct_block4x4(
        &mut self,
        plane: ColorPlane,
        x: u32,
        y: u32,
        prediction: &[u8; 16],
        residual: &[i32; 16],
    ) -> Result<(), FrameError> {
        let (plane_width, plane_height) = self.plane_dimensions(plane);
        if x.checked_add(4).is_none_or(|right| right > plane_width)
            || y.checked_add(4).is_none_or(|bottom| bottom > plane_height)
        {
            return Err(FrameError::BlockOutOfBounds);
        }

        let target = match plane {
            ColorPlane::Y => &mut self.y,
            ColorPlane::U => &mut self.u,
            ColorPlane::V => &mut self.v,
        };
        let plane_width =
            usize::try_from(plane_width).map_err(|_| FrameError::DimensionOverflow)?;
        let x = usize::try_from(x).map_err(|_| FrameError::DimensionOverflow)?;
        let y = usize::try_from(y).map_err(|_| FrameError::DimensionOverflow)?;

        for row in 0..4 {
            let offset = (y + row) * plane_width + x;
            for column in 0..4 {
                let index = row * 4 + column;
                let sample = i32::from(prediction[index]) + residual[index];
                target[offset + column] = sample.clamp(0, 255) as u8;
            }
        }

        Ok(())
    }
}

fn plane_len(width: u32, height: u32) -> Result<usize, FrameError> {
    let length = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(FrameError::DimensionOverflow)?;
    usize::try_from(length).map_err(|_| FrameError::DimensionOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_yuv420_planes_with_ceil_chroma_dimensions() {
        let frame = Yuv420Frame::new(5, 3).unwrap();

        assert_eq!(frame.width, 5);
        assert_eq!(frame.height, 3);
        assert_eq!(frame.y.len(), 15);
        assert_eq!(frame.u.len(), 6);
        assert_eq!(frame.v.len(), 6);
        assert_eq!(frame.u, vec![128; 6]);
        assert_eq!(frame.plane_dimensions(ColorPlane::V), (3, 2));
    }

    #[test]
    fn reconstructs_blocks_and_clips_samples() {
        let mut frame = Yuv420Frame::new(8, 8).unwrap();
        let prediction = [128; 16];
        let mut residual = [0; 16];
        residual[0] = 200;
        residual[1] = -200;

        frame
            .reconstruct_block4x4(ColorPlane::Y, 4, 0, &prediction, &residual)
            .unwrap();

        assert_eq!(frame.y[4], 255);
        assert_eq!(frame.y[5], 0);
        assert_eq!(frame.y[6], 128);

        frame
            .reconstruct_block4x4(ColorPlane::U, 0, 0, &prediction, &[0; 16])
            .unwrap();
        assert_eq!(frame.u[0], 128);
    }

    #[test]
    fn rejects_invalid_dimensions_and_out_of_bounds_blocks() {
        assert_eq!(Yuv420Frame::new(0, 4), Err(FrameError::InvalidDimensions));

        let mut frame = Yuv420Frame::new(6, 6).unwrap();
        assert_eq!(
            frame.reconstruct_block4x4(ColorPlane::Y, 4, 0, &[0; 16], &[0; 16]),
            Err(FrameError::BlockOutOfBounds)
        );
    }
}
