use crate::video::h264::{
    frame::{ColorPlane, FrameError, Yuv420Frame},
    motion::{
        chroma::interpolate_chroma_sample, luma::interpolate_luma_sample, prediction::MotionVector,
    },
    weighted::WeightedPrediction,
};

pub type WeightParams<'a> = (
    &'a WeightedPrediction,
    Option<(i32, i32)>,
    Option<[(i32, i32); 2]>,
);

pub fn compensate_macroblock(
    output: &mut Yuv420Frame,
    reference: &Yuv420Frame,
    mb_x: u32,
    mb_y: u32,
    motion: MotionVector,
) -> Result<(), FrameError> {
    compensate_macroblock_weighted(output, reference, mb_x, mb_y, motion, None)
}

pub fn compensate_macroblock_weighted(
    output: &mut Yuv420Frame,
    reference: &Yuv420Frame,
    mb_x: u32,
    mb_y: u32,
    motion: MotionVector,
    weights: Option<WeightParams<'_>>,
) -> Result<(), FrameError> {
    if output.width != reference.width || output.height != reference.height {
        return Err(FrameError::InvalidDimensions);
    }

    compensate_luma_16x16_weighted(output, reference, mb_x, mb_y, motion, weights)?;

    compensate_chroma_8x8_weighted(
        output,
        reference,
        ColorPlane::U,
        mb_x,
        mb_y,
        motion,
        weights,
    )?;
    compensate_chroma_8x8_weighted(
        output,
        reference,
        ColorPlane::V,
        mb_x,
        mb_y,
        motion,
        weights,
    )?;

    Ok(())
}

fn compensate_luma_16x16_weighted(
    output: &mut Yuv420Frame,
    reference: &Yuv420Frame,
    mb_x: u32,
    mb_y: u32,
    motion: MotionVector,
    weights: Option<WeightParams<'_>>,
) -> Result<(), FrameError> {
    let x = mb_x * 16;
    let y = mb_y * 16;

    for dy in 0..16 {
        for dx in 0..16 {
            let src_x = i64::from(x + dx);
            let src_y = i64::from(y + dy);

            let mut sample = interpolate_luma_sample(
                reference,
                src_x * 4 + i64::from(motion.x),
                src_y * 4 + i64::from(motion.y),
            );

            if let Some((wp, Some((weight, offset)), _)) = weights {
                sample = wp.apply_luma(sample, weight, offset);
            }

            let out_x = usize::try_from(x + dx).map_err(|_| FrameError::DimensionOverflow)?;
            let out_y = usize::try_from(y + dy).map_err(|_| FrameError::DimensionOverflow)?;
            let width = usize::try_from(output.width).map_err(|_| FrameError::DimensionOverflow)?;

            if out_y * width + out_x >= output.y.len() {
                return Err(FrameError::BlockOutOfBounds);
            }

            output.y[out_y * width + out_x] = sample;
        }
    }

    Ok(())
}

fn compensate_chroma_8x8_weighted(
    output: &mut Yuv420Frame,
    reference: &Yuv420Frame,
    plane: ColorPlane,
    mb_x: u32,
    mb_y: u32,
    motion: MotionVector,
    weights: Option<WeightParams<'_>>,
) -> Result<(), FrameError> {
    let x = mb_x * 8;
    let y = mb_y * 8;

    let (chroma_width, _chroma_height) = reference.plane_dimensions(plane);
    let dst_plane = match plane {
        ColorPlane::U => &mut output.u,
        ColorPlane::V => &mut output.v,
        ColorPlane::Y => return Err(FrameError::InvalidDimensions),
    };

    let chroma_idx = match plane {
        ColorPlane::U => 0,
        ColorPlane::V => 1,
        ColorPlane::Y => return Err(FrameError::InvalidDimensions),
    };

    for dy in 0..8 {
        for dx in 0..8 {
            let mut sample =
                interpolate_chroma_sample(reference, plane, x + dx, y + dy, motion.x, motion.y);

            if let Some((wp, _, Some(chroma_array))) = weights {
                let (weight, offset) = chroma_array[chroma_idx];
                sample = wp.apply_chroma(sample, weight, offset);
            }

            let out_x = usize::try_from(x + dx).map_err(|_| FrameError::DimensionOverflow)?;
            let out_y = usize::try_from(y + dy).map_err(|_| FrameError::DimensionOverflow)?;
            let width = usize::try_from(chroma_width).map_err(|_| FrameError::DimensionOverflow)?;

            if out_y * width + out_x >= dst_plane.len() {
                return Err(FrameError::BlockOutOfBounds);
            }

            dst_plane[out_y * width + out_x] = sample;
        }
    }

    Ok(())
}

pub fn average_macroblock(
    output: &mut Yuv420Frame,
    reference0: &Yuv420Frame,
    reference1: &Yuv420Frame,
    mb_x: u32,
    mb_y: u32,
    motion0: MotionVector,
    motion1: MotionVector,
) -> Result<(), FrameError> {
    let mut temp = Yuv420Frame::new(output.width, output.height)?;

    compensate_macroblock(&mut temp, reference0, mb_x, mb_y, motion0)?;

    compensate_macroblock(output, reference1, mb_x, mb_y, motion1)?;

    let x = mb_x * 16;
    let y = mb_y * 16;
    let width = usize::try_from(output.width).map_err(|_| FrameError::DimensionOverflow)?;

    for dy in 0..16 {
        for dx in 0..16 {
            let idx = (y as usize + dy) * width + (x as usize + dx);
            if idx < output.y.len() {
                output.y[idx] =
                    (u16::from(output.y[idx]) + u16::from(temp.y[idx])).div_ceil(2) as u8;
            }
        }
    }

    let (chroma_width, _) = output.plane_dimensions(ColorPlane::U);
    let chroma_width = usize::try_from(chroma_width).map_err(|_| FrameError::DimensionOverflow)?;
    let chroma_x = x / 2;
    let chroma_y = y / 2;

    for dy in 0..8 {
        for dx in 0..8 {
            let idx = (chroma_y as usize + dy) * chroma_width + (chroma_x as usize + dx);
            if idx < output.u.len() {
                output.u[idx] =
                    (u16::from(output.u[idx]) + u16::from(temp.u[idx])).div_ceil(2) as u8;
            }
            if idx < output.v.len() {
                output.v[idx] =
                    (u16::from(output.v[idx]) + u16::from(temp.v[idx])).div_ceil(2) as u8;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compensates_macroblock_with_zero_motion() {
        let mut reference = Yuv420Frame::new(32, 32).unwrap();
        reference.y[0..16 * 32].fill(100);

        let mut output = Yuv420Frame::new(32, 32).unwrap();

        compensate_macroblock(&mut output, &reference, 0, 0, MotionVector::default()).unwrap();

        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(output.y[y * 32 + x], 100);
            }
        }
    }

    #[test]
    fn rejects_dimension_mismatch() {
        let reference = Yuv420Frame::new(32, 32).unwrap();
        let mut output = Yuv420Frame::new(16, 16).unwrap();

        let result = compensate_macroblock(&mut output, &reference, 0, 0, MotionVector::default());

        assert_eq!(result, Err(FrameError::InvalidDimensions));
    }
}
