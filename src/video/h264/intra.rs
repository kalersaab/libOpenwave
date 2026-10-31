use crate::video::h264::{
    frame::{ColorPlane, FrameError, Yuv420Frame},
    transform::{TransformError, reconstruct_residual_4x4},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntraPredictionError {
    MissingReference,
}

impl std::fmt::Display for IntraPredictionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "H.264 intra prediction error: {self:?}")
    }
}

impl std::error::Error for IntraPredictionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntraReconstructionError {
    Prediction(IntraPredictionError),
    Transform(TransformError),
    Frame(FrameError),
}

impl std::fmt::Display for IntraReconstructionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "H.264 intra reconstruction error: {self:?}")
    }
}

impl std::error::Error for IntraReconstructionError {}

impl From<IntraPredictionError> for IntraReconstructionError {
    fn from(value: IntraPredictionError) -> Self {
        Self::Prediction(value)
    }
}

impl From<TransformError> for IntraReconstructionError {
    fn from(value: TransformError) -> Self {
        Self::Transform(value)
    }
}

impl From<FrameError> for IntraReconstructionError {
    fn from(value: FrameError) -> Self {
        Self::Frame(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Intra4x4References {
    pub top: Option<[u8; 4]>,
    pub top_right: Option<[u8; 4]>,
    pub left: Option<[u8; 4]>,
    pub top_left: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intra4x4Mode {
    Vertical,
    Horizontal,
    Dc,
    DiagonalDownLeft,
    DiagonalDownRight,
    VerticalRight,
    HorizontalDown,
    VerticalLeft,
    HorizontalUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Intra16x16References {
    pub top: Option<[u8; 16]>,
    pub left: Option<[u8; 16]>,
    pub top_left: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intra16x16Mode {
    Vertical,
    Horizontal,
    Dc,
    Plane,
}

fn avg2(a: u8, b: u8) -> u8 {
    ((u16::from(a) + u16::from(b) + 1) >> 1) as u8
}

fn avg3(a: u8, b: u8, c: u8) -> u8 {
    ((u16::from(a) + 2 * u16::from(b) + u16::from(c) + 2) >> 2) as u8
}

fn require<T: Copy>(value: Option<T>) -> Result<T, IntraPredictionError> {
    value.ok_or(IntraPredictionError::MissingReference)
}

fn dc4(top: Option<[u8; 4]>, left: Option<[u8; 4]>) -> [[u8; 4]; 4] {
    let value = match (top, left) {
        (Some(top), Some(left)) => {
            ((top
                .iter()
                .chain(left.iter())
                .map(|&v| u16::from(v))
                .sum::<u16>()
                + 4)
                >> 3) as u8
        }
        (Some(top), None) => ((top.iter().map(|&v| u16::from(v)).sum::<u16>() + 2) >> 2) as u8,
        (None, Some(left)) => ((left.iter().map(|&v| u16::from(v)).sum::<u16>() + 2) >> 2) as u8,
        (None, None) => 128,
    };
    [[value; 4]; 4]
}

pub fn predict_intra4x4(
    mode: Intra4x4Mode,
    references: Intra4x4References,
) -> Result<[[u8; 4]; 4], IntraPredictionError> {
    use Intra4x4Mode as Mode;

    let top = references.top;
    let left = references.left;
    if mode == Mode::Dc {
        return Ok(dc4(top, left));
    }

    let mut output = [[0; 4]; 4];
    match mode {
        Mode::Vertical => {
            let top = require(top)?;
            output.fill(top);
        }
        Mode::Horizontal => {
            let left = require(left)?;
            for (row, value) in output.iter_mut().zip(left) {
                *row = [value; 4];
            }
        }
        Mode::Dc => unreachable!(),
        Mode::DiagonalDownLeft => {
            let top = require(top)?;
            let top_right = references.top_right.unwrap_or([top[3]; 4]);
            let mut reference = [0; 8];
            reference[..4].copy_from_slice(&top);
            reference[4..].copy_from_slice(&top_right);
            for (y, row) in output.iter_mut().enumerate() {
                for (x, pixel) in row.iter_mut().enumerate() {
                    let index = x + y;
                    *pixel = if index == 6 {
                        ((u16::from(reference[6]) + 3 * u16::from(reference[7]) + 2) >> 2) as u8
                    } else {
                        avg3(reference[index], reference[index + 1], reference[index + 2])
                    };
                }
            }
        }
        Mode::DiagonalDownRight => {
            let top = require(top)?;
            let left = require(left)?;
            let top_left = require(references.top_left)?;
            let mut reference = [0; 9];
            reference[..4].copy_from_slice(&left.iter().rev().copied().collect::<Vec<_>>());
            reference[4] = top_left;
            reference[5..].copy_from_slice(&top);

            for (y, row) in output.iter_mut().enumerate() {
                for (x, pixel) in row.iter_mut().enumerate() {
                    let index = 4isize + x as isize - y as isize;
                    *pixel = avg3(
                        reference[(index - 1) as usize],
                        reference[index as usize],
                        reference[(index + 1) as usize],
                    );
                }
            }
        }
        Mode::VerticalRight => {
            let top = require(top)?;
            let left = require(left)?;
            let top_left = require(references.top_left)?;
            let q = |a, b, c| avg3(a, b, c);
            output[0] = [
                avg2(top_left, top[0]),
                avg2(top[0], top[1]),
                avg2(top[1], top[2]),
                avg2(top[2], top[3]),
            ];
            output[1] = [
                q(left[0], top_left, top[0]),
                q(top_left, top[0], top[1]),
                q(top[0], top[1], top[2]),
                q(top[1], top[2], top[3]),
            ];
            output[2] = [
                q(top_left, left[0], left[1]),
                q(left[0], top_left, top[0]),
                q(top_left, top[0], top[1]),
                q(top[0], top[1], top[2]),
            ];
            output[3] = [
                q(left[0], left[1], left[2]),
                q(left[0], top_left, top[0]),
                q(top_left, top[0], top[1]),
                q(top[0], top[1], top[2]),
            ];
        }
        Mode::HorizontalDown => {
            let top = require(top)?;
            let left = require(left)?;
            let top_left = require(references.top_left)?;
            let q = |a, b, c| avg3(a, b, c);
            output[0] = [
                avg2(top_left, left[0]),
                q(left[0], top_left, top[0]),
                q(top_left, top[0], top[1]),
                q(top[0], top[1], top[2]),
            ];
            output[1] = [
                avg2(left[0], left[1]),
                q(top_left, left[0], left[1]),
                avg2(top_left, left[0]),
                q(left[0], top_left, top[0]),
            ];
            output[2] = [
                avg2(left[1], left[2]),
                q(left[0], left[1], left[2]),
                avg2(left[0], left[1]),
                q(top_left, left[0], left[1]),
            ];
            output[3] = [
                avg2(left[2], left[3]),
                q(left[1], left[2], left[3]),
                avg2(left[1], left[2]),
                q(left[0], left[1], left[2]),
            ];
        }
        Mode::VerticalLeft => {
            let top = require(top)?;
            let top_right = references.top_right.unwrap_or([top[3]; 4]);
            let mut reference = [0; 8];
            reference[..4].copy_from_slice(&top);
            reference[4..].copy_from_slice(&top_right);
            for (y, row) in output.iter_mut().enumerate() {
                for (x, pixel) in row.iter_mut().enumerate() {
                    let index = x + y / 2;
                    *pixel = if y % 2 == 0 {
                        avg2(reference[index], reference[index + 1])
                    } else {
                        avg3(reference[index], reference[index + 1], reference[index + 2])
                    };
                }
            }
        }
        Mode::HorizontalUp => {
            let left = require(left)?;
            for (y, row) in output.iter_mut().enumerate() {
                for (x, pixel) in row.iter_mut().enumerate() {
                    let index = x + y;
                    *pixel = if index >= 6 {
                        left[3]
                    } else if index % 2 == 0 {
                        avg2(left[index / 2], left[index / 2 + 1])
                    } else {
                        avg3(
                            left[index / 2],
                            left[index / 2 + 1],
                            left[(index / 2 + 2).min(3)],
                        )
                    };
                }
            }
        }
    }

    Ok(output)
}

fn dc16(top: Option<[u8; 16]>, left: Option<[u8; 16]>) -> [[u8; 16]; 16] {
    let value = match (top, left) {
        (Some(top), Some(left)) => {
            ((top
                .iter()
                .chain(left.iter())
                .map(|&v| u32::from(v))
                .sum::<u32>()
                + 16)
                >> 5) as u8
        }
        (Some(top), None) => ((top.iter().map(|&v| u32::from(v)).sum::<u32>() + 8) >> 4) as u8,
        (None, Some(left)) => ((left.iter().map(|&v| u32::from(v)).sum::<u32>() + 8) >> 4) as u8,
        (None, None) => 128,
    };
    [[value; 16]; 16]
}

pub fn predict_intra16x16(
    mode: Intra16x16Mode,
    references: Intra16x16References,
) -> Result<[[u8; 16]; 16], IntraPredictionError> {
    use Intra16x16Mode as Mode;

    if mode == Mode::Dc {
        return Ok(dc16(references.top, references.left));
    }

    let mut output = [[0; 16]; 16];
    match mode {
        Mode::Vertical => {
            let top = require(references.top)?;
            output.fill(top);
        }
        Mode::Horizontal => {
            let left = require(references.left)?;
            for (row, value) in output.iter_mut().zip(left) {
                *row = [value; 16];
            }
        }
        Mode::Dc => unreachable!(),
        Mode::Plane => {
            let top = require(references.top)?;
            let left = require(references.left)?;
            let top_left = require(references.top_left)?;

            let mut horizontal = i32::from(top[8]) - i32::from(top[6]);
            let mut vertical = i32::from(left[8]) - i32::from(left[6]);
            for index in 2..=8 {
                horizontal += index
                    * (i32::from(top[7 + index as usize])
                        - i32::from(if index == 8 {
                            top_left
                        } else {
                            top[7 - index as usize]
                        }));
                vertical += index
                    * (i32::from(left[7 + index as usize])
                        - i32::from(if index == 8 {
                            top_left
                        } else {
                            left[7 - index as usize]
                        }));
            }

            let a = 16 * (i32::from(top[15]) + i32::from(left[15])) - 7 * (horizontal + vertical);
            let b = (5 * horizontal + 32) >> 6;
            let c = (5 * vertical + 32) >> 6;
            for (y, row) in output.iter_mut().enumerate() {
                for (x, pixel) in row.iter_mut().enumerate() {
                    *pixel = ((a + b * (x as i32 - 7) + c * (y as i32 - 7) + 16) >> 5).clamp(0, 255)
                        as u8;
                }
            }
        }
    }

    Ok(output)
}

pub struct Intra4x4Reconstruction<'a> {
    pub plane: ColorPlane,
    pub x: u32,
    pub y: u32,
    pub mode: Intra4x4Mode,
    pub references: Intra4x4References,
    pub quantized_levels: &'a [i32; 16],
    pub qp: u8,
    pub scaling_list: Option<&'a [u8; 16]>,
}

pub fn reconstruct_intra4x4_block(
    frame: &mut Yuv420Frame,
    block: Intra4x4Reconstruction<'_>,
) -> Result<(), IntraReconstructionError> {
    let prediction = predict_intra4x4(block.mode, block.references)?;
    let prediction = std::array::from_fn(|index| prediction[index / 4][index % 4]);
    let residual = reconstruct_residual_4x4(block.quantized_levels, block.qp, block.scaling_list)?;
    frame.reconstruct_block4x4(block.plane, block.x, block.y, &prediction, &residual)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn references4() -> Intra4x4References {
        Intra4x4References {
            top: Some([10, 20, 30, 40]),
            top_right: Some([50, 60, 70, 80]),
            left: Some([90, 100, 110, 120]),
            top_left: Some(80),
        }
    }

    #[test]
    fn predicts_intra4x4_basic_modes() {
        let references = references4();
        assert_eq!(
            predict_intra4x4(Intra4x4Mode::Vertical, references).unwrap(),
            [[10, 20, 30, 40]; 4]
        );
        assert_eq!(
            predict_intra4x4(Intra4x4Mode::Horizontal, references).unwrap(),
            [
                [90, 90, 90, 90],
                [100, 100, 100, 100],
                [110, 110, 110, 110],
                [120, 120, 120, 120]
            ]
        );
        assert_eq!(
            predict_intra4x4(Intra4x4Mode::Dc, references).unwrap(),
            [[65; 4]; 4]
        );
    }

    #[test]
    fn predicts_intra4x4_directional_modes() {
        let references = references4();
        let down_left = predict_intra4x4(Intra4x4Mode::DiagonalDownLeft, references).unwrap();
        assert_eq!(down_left[0][0], 20);
        assert_eq!(down_left[0][3], 50);
        assert_eq!(down_left[3][3], 78);

        let down_right = predict_intra4x4(Intra4x4Mode::DiagonalDownRight, references).unwrap();
        assert_eq!(down_right[0][0], 65);
        assert_eq!(down_right[0][3], 30);
        assert_eq!(down_right[3][0], 110);

        let vertical_right = predict_intra4x4(Intra4x4Mode::VerticalRight, references).unwrap();
        assert_eq!(vertical_right[0][0], 45);
        assert_eq!(vertical_right[1][0], 65);

        let horizontal_down = predict_intra4x4(Intra4x4Mode::HorizontalDown, references).unwrap();
        assert_eq!(horizontal_down[0][0], 85);
        assert_eq!(horizontal_down[0][1], 65);

        let vertical_left = predict_intra4x4(Intra4x4Mode::VerticalLeft, references).unwrap();
        assert_eq!(vertical_left[0][0], 15);
        assert_eq!(vertical_left[1][0], 20);

        let horizontal_up = predict_intra4x4(Intra4x4Mode::HorizontalUp, references).unwrap();
        assert_eq!(horizontal_up[0][0], 95);
        assert_eq!(horizontal_up[0][1], 100);
        assert_eq!(horizontal_up[3][3], 120);
    }

    #[test]
    fn substitutes_unavailable_top_right_and_dc_boundaries() {
        let mut references = references4();
        references.top_right = None;
        let predicted = predict_intra4x4(Intra4x4Mode::DiagonalDownLeft, references).unwrap();
        assert_eq!(predicted[0][3], 40);
        assert_eq!(predicted[3][3], 40);

        references.top = None;
        references.left = None;
        assert_eq!(
            predict_intra4x4(Intra4x4Mode::Dc, references).unwrap(),
            [[128; 4]; 4]
        );
        assert_eq!(
            predict_intra4x4(Intra4x4Mode::Vertical, references),
            Err(IntraPredictionError::MissingReference)
        );
    }

    fn references16(value: u8) -> Intra16x16References {
        Intra16x16References {
            top: Some([value; 16]),
            left: Some([value; 16]),
            top_left: Some(value),
        }
    }

    #[test]
    fn predicts_intra16x16_vertical_horizontal_and_dc() {
        let references = references16(80);
        assert_eq!(
            predict_intra16x16(Intra16x16Mode::Vertical, references).unwrap(),
            [[80; 16]; 16]
        );
        assert_eq!(
            predict_intra16x16(Intra16x16Mode::Horizontal, references).unwrap(),
            [[80; 16]; 16]
        );
        assert_eq!(
            predict_intra16x16(Intra16x16Mode::Dc, references).unwrap(),
            [[80; 16]; 16]
        );
    }

    #[test]
    fn predicts_intra16x16_plane_and_rejects_missing_edges() {
        assert_eq!(
            predict_intra16x16(Intra16x16Mode::Plane, references16(128)).unwrap(),
            [[128; 16]; 16]
        );
        assert_eq!(
            predict_intra16x16(
                Intra16x16Mode::Plane,
                Intra16x16References {
                    top: None,
                    left: Some([128; 16]),
                    top_left: Some(128),
                }
            ),
            Err(IntraPredictionError::MissingReference)
        );
    }

    #[test]
    fn reconstructs_intra4x4_prediction_and_residual_into_yuv_frame() {
        let mut frame = Yuv420Frame::new(8, 8).unwrap();
        let references = Intra4x4References {
            top: None,
            top_right: None,
            left: None,
            top_left: None,
        };
        let mut levels = [0; 16];
        levels[0] = 1;

        reconstruct_intra4x4_block(
            &mut frame,
            Intra4x4Reconstruction {
                plane: ColorPlane::Y,
                x: 0,
                y: 0,
                mode: Intra4x4Mode::Dc,
                references,
                quantized_levels: &levels,
                qp: 24,
                scaling_list: None,
            },
        )
        .unwrap();

        assert_eq!(&frame.y[..4], &[131; 4]);
        assert_eq!(&frame.y[8..12], &[131; 4]);
    }
}
