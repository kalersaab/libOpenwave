use crate::video::h264::{
    frame::{ColorPlane, FrameError, Yuv420Frame},
    motion::{
        chroma::interpolate_chroma_sample, luma::interpolate_luma_sample, prediction::MotionVector,
    },
    weighted::WeightedPrediction,
};

pub type BiWeightParams<'a> = (
    &'a WeightedPrediction,
    [(i32, i32); 2],
    [[(i32, i32); 2]; 2],
);

pub struct BiPredParams<'a> {
    pub reference_l0: &'a Yuv420Frame,
    pub reference_l1: &'a Yuv420Frame,
    pub mb_x: u32,
    pub mb_y: u32,
    pub motion_l0: MotionVector,
    pub motion_l1: MotionVector,
    pub weights: Option<BiWeightParams<'a>>,
}

struct LumaBiPredParams<'a> {
    reference_l0: &'a Yuv420Frame,
    reference_l1: &'a Yuv420Frame,
    mb_x: u32,
    mb_y: u32,
    motion_l0: MotionVector,
    motion_l1: MotionVector,
    weights: Option<(&'a WeightedPrediction, [(i32, i32); 2])>,
}

struct ChromaBiPredParams<'a> {
    reference_l0: &'a Yuv420Frame,
    reference_l1: &'a Yuv420Frame,
    plane: ColorPlane,
    mb_x: u32,
    mb_y: u32,
    motion_l0: MotionVector,
    motion_l1: MotionVector,
    weights: Option<(&'a WeightedPrediction, [(i32, i32); 2])>,
}

pub fn bipred_macroblock_weighted(
    output: &mut Yuv420Frame,
    params: BiPredParams<'_>,
) -> Result<(), FrameError> {
    if output.width != params.reference_l0.width
        || output.height != params.reference_l0.height
        || output.width != params.reference_l1.width
        || output.height != params.reference_l1.height
    {
        return Err(FrameError::InvalidDimensions);
    }

    bipred_luma_16x16(
        output,
        LumaBiPredParams {
            reference_l0: params.reference_l0,
            reference_l1: params.reference_l1,
            mb_x: params.mb_x,
            mb_y: params.mb_y,
            motion_l0: params.motion_l0,
            motion_l1: params.motion_l1,
            weights: params
                .weights
                .as_ref()
                .map(|(wp, luma_weights, _)| (*wp, *luma_weights)),
        },
    )?;

    bipred_chroma_8x8(
        output,
        ChromaBiPredParams {
            reference_l0: params.reference_l0,
            reference_l1: params.reference_l1,
            plane: ColorPlane::U,
            mb_x: params.mb_x,
            mb_y: params.mb_y,
            motion_l0: params.motion_l0,
            motion_l1: params.motion_l1,
            weights: params
                .weights
                .as_ref()
                .map(|(wp, _, chroma_weights)| (*wp, [chroma_weights[0][0], chroma_weights[1][0]])),
        },
    )?;

    bipred_chroma_8x8(
        output,
        ChromaBiPredParams {
            reference_l0: params.reference_l0,
            reference_l1: params.reference_l1,
            plane: ColorPlane::V,
            mb_x: params.mb_x,
            mb_y: params.mb_y,
            motion_l0: params.motion_l0,
            motion_l1: params.motion_l1,
            weights: params
                .weights
                .as_ref()
                .map(|(wp, _, chroma_weights)| (*wp, [chroma_weights[0][1], chroma_weights[1][1]])),
        },
    )?;

    Ok(())
}

fn bipred_luma_16x16(
    output: &mut Yuv420Frame,
    params: LumaBiPredParams<'_>,
) -> Result<(), FrameError> {
    let x = params.mb_x * 16;
    let y = params.mb_y * 16;

    for dy in 0..16 {
        for dx in 0..16 {
            let src_x = i64::from(x + dx);
            let src_y = i64::from(y + dy);

            let sample_l0 = interpolate_luma_sample(
                params.reference_l0,
                src_x * 4 + i64::from(params.motion_l0.x),
                src_y * 4 + i64::from(params.motion_l0.y),
            );

            let sample_l1 = interpolate_luma_sample(
                params.reference_l1,
                src_x * 4 + i64::from(params.motion_l1.x),
                src_y * 4 + i64::from(params.motion_l1.y),
            );

            let final_sample = if let Some((wp, [(w0, o0), (w1, o1)])) = params.weights {
                wp.apply_luma_bi(w0, o0, sample_l0, w1, o1, sample_l1)
            } else {
                ((u16::from(sample_l0) + u16::from(sample_l1) + 1) >> 1) as u8
            };

            let out_x = usize::try_from(x + dx).map_err(|_| FrameError::DimensionOverflow)?;
            let out_y = usize::try_from(y + dy).map_err(|_| FrameError::DimensionOverflow)?;
            let width = usize::try_from(output.width).map_err(|_| FrameError::DimensionOverflow)?;

            if out_y * width + out_x >= output.y.len() {
                return Err(FrameError::BlockOutOfBounds);
            }

            output.y[out_y * width + out_x] = final_sample;
        }
    }

    Ok(())
}

fn bipred_chroma_8x8(
    output: &mut Yuv420Frame,
    params: ChromaBiPredParams<'_>,
) -> Result<(), FrameError> {
    let x = params.mb_x * 8;
    let y = params.mb_y * 8;

    let (chroma_width, _) = params.reference_l0.plane_dimensions(params.plane);
    let dst_plane = match params.plane {
        ColorPlane::U => &mut output.u,
        ColorPlane::V => &mut output.v,
        ColorPlane::Y => return Err(FrameError::InvalidDimensions),
    };

    for dy in 0..8 {
        for dx in 0..8 {
            let sample_l0 = interpolate_chroma_sample(
                params.reference_l0,
                params.plane,
                x + dx,
                y + dy,
                params.motion_l0.x,
                params.motion_l0.y,
            );

            let sample_l1 = interpolate_chroma_sample(
                params.reference_l1,
                params.plane,
                x + dx,
                y + dy,
                params.motion_l1.x,
                params.motion_l1.y,
            );

            let final_sample = if let Some((wp, [(w0, o0), (w1, o1)])) = params.weights {
                wp.apply_chroma_bi(w0, o0, sample_l0, w1, o1, sample_l1)
            } else {
                ((u16::from(sample_l0) + u16::from(sample_l1) + 1) >> 1) as u8
            };

            let out_x = usize::try_from(x + dx).map_err(|_| FrameError::DimensionOverflow)?;
            let out_y = usize::try_from(y + dy).map_err(|_| FrameError::DimensionOverflow)?;
            let width = usize::try_from(chroma_width).map_err(|_| FrameError::DimensionOverflow)?;

            if out_y * width + out_x >= dst_plane.len() {
                return Err(FrameError::BlockOutOfBounds);
            }

            dst_plane[out_y * width + out_x] = final_sample;
        }
    }

    Ok(())
}

pub fn bipred_macroblock(
    output: &mut Yuv420Frame,
    reference_l0: &Yuv420Frame,
    reference_l1: &Yuv420Frame,
    mb_x: u32,
    mb_y: u32,
    motion_l0: MotionVector,
    motion_l1: MotionVector,
) -> Result<(), FrameError> {
    bipred_macroblock_weighted(
        output,
        BiPredParams {
            reference_l0,
            reference_l1,
            mb_x,
            mb_y,
            motion_l0,
            motion_l1,
            weights: None,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bipred_averages_two_references() {
        let mut ref_l0 = Yuv420Frame::new(32, 32).unwrap();
        let mut ref_l1 = Yuv420Frame::new(32, 32).unwrap();

        for i in 0..16 * 32 {
            ref_l0.y[i] = 100;
            ref_l1.y[i] = 140;
        }

        let mut output = Yuv420Frame::new(32, 32).unwrap();

        bipred_macroblock(
            &mut output,
            &ref_l0,
            &ref_l1,
            0,
            0,
            MotionVector::default(),
            MotionVector::default(),
        )
        .unwrap();

        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(output.y[y * 32 + x], 120);
            }
        }
    }

    #[test]
    fn bipred_applies_weighted_prediction() {
        let mut ref_l0 = Yuv420Frame::new(32, 32).unwrap();
        let mut ref_l1 = Yuv420Frame::new(32, 32).unwrap();

        ref_l0.y[0..16 * 32].fill(100);
        ref_l1.y[0..16 * 32].fill(140);

        let mut output = Yuv420Frame::new(32, 32).unwrap();

        let wp = WeightedPrediction::implicit(5);

        let weights = Some((&wp, [(48, 0), (16, 0)], [[(32, 0), (32, 0)]; 2]));

        bipred_macroblock_weighted(
            &mut output,
            BiPredParams {
                reference_l0: &ref_l0,
                reference_l1: &ref_l1,
                mb_x: 0,
                mb_y: 0,
                motion_l0: MotionVector::default(),
                motion_l1: MotionVector::default(),
                weights,
            },
        )
        .unwrap();

        let expected = 110;
        for y in 0..16 {
            for x in 0..16 {
                let actual = output.y[y * 32 + x];
                assert!((expected - i32::from(actual)).abs() <= 1);
            }
        }
    }

    #[test]
    fn bipred_rejects_dimension_mismatch() {
        let ref_l0 = Yuv420Frame::new(32, 32).unwrap();
        let ref_l1 = Yuv420Frame::new(16, 16).unwrap();
        let mut output = Yuv420Frame::new(32, 32).unwrap();

        let result = bipred_macroblock(
            &mut output,
            &ref_l0,
            &ref_l1,
            0,
            0,
            MotionVector::default(),
            MotionVector::default(),
        );

        assert_eq!(result, Err(FrameError::InvalidDimensions));
    }
}
