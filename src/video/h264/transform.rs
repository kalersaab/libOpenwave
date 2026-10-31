#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformError {
    InvalidQp,
    InvalidScalingFactor,
    Overflow,
}

impl std::fmt::Display for TransformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "H.264 transform error: {self:?}")
    }
}

impl std::error::Error for TransformError {}

const LEVEL_SCALE: [[i64; 3]; 6] = [
    [10, 13, 16],
    [11, 14, 18],
    [13, 16, 20],
    [14, 18, 23],
    [16, 20, 25],
    [18, 23, 29],
];

const SCALE_CLASS_4X4: [usize; 16] = [0, 1, 0, 1, 1, 2, 1, 2, 0, 1, 0, 1, 1, 2, 1, 2];

pub fn inverse_scale_4x4(
    levels: &[i32; 16],
    qp: u8,
    scaling_list: Option<&[u8; 16]>,
) -> Result<[i32; 16], TransformError> {
    if qp > 51 {
        return Err(TransformError::InvalidQp);
    }
    let scaling_list = scaling_list.copied().unwrap_or([16; 16]);
    if scaling_list.contains(&0) {
        return Err(TransformError::InvalidScalingFactor);
    }

    let qp_div6 = u32::from(qp / 6);
    let qp_mod6 = usize::from(qp % 6);
    let mut scaled = [0i32; 16];

    for index in 0..16 {
        let level_scale =
            LEVEL_SCALE[qp_mod6][SCALE_CLASS_4X4[index]] * i64::from(scaling_list[index]);
        let product = i64::from(levels[index])
            .checked_mul(level_scale)
            .ok_or(TransformError::Overflow)?;
        let value = if qp_div6 >= 4 {
            product
                .checked_shl(qp_div6 - 4)
                .ok_or(TransformError::Overflow)?
        } else {
            let shift = 4 - qp_div6;
            let rounding = 1i64 << (shift - 1);
            product
                .checked_add(rounding)
                .ok_or(TransformError::Overflow)?
                >> shift
        };
        scaled[index] = i32::try_from(value).map_err(|_| TransformError::Overflow)?;
    }

    Ok(scaled)
}

fn inverse_transform_1d(input: [i64; 4]) -> [i64; 4] {
    let e0 = input[0] + input[2];
    let e1 = input[0] - input[2];
    let e2 = (input[1] >> 1) - input[3];
    let e3 = input[1] + (input[3] >> 1);

    [e0 + e3, e1 + e2, e1 - e2, e0 - e3]
}

pub fn inverse_transform_4x4(coefficients: &[i32; 16]) -> Result<[i32; 16], TransformError> {
    let mut temporary = [[0i64; 4]; 4];

    for row in 0..4 {
        let transformed = inverse_transform_1d([
            i64::from(coefficients[row * 4]),
            i64::from(coefficients[row * 4 + 1]),
            i64::from(coefficients[row * 4 + 2]),
            i64::from(coefficients[row * 4 + 3]),
        ]);
        temporary[row] = transformed;
    }

    let mut residual = [0i32; 16];
    for column in 0..4 {
        let transformed = inverse_transform_1d([
            temporary[0][column],
            temporary[1][column],
            temporary[2][column],
            temporary[3][column],
        ]);
        for row in 0..4 {
            let value = (transformed[row] + 32) >> 6;
            residual[row * 4 + column] =
                i32::try_from(value).map_err(|_| TransformError::Overflow)?;
        }
    }

    Ok(residual)
}

pub fn reconstruct_residual_4x4(
    levels: &[i32; 16],
    qp: u8,
    scaling_list: Option<&[u8; 16]>,
) -> Result<[i32; 16], TransformError> {
    let coefficients = inverse_scale_4x4(levels, qp, scaling_list)?;
    inverse_transform_4x4(&coefficients)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_scales_by_position_class_and_qp() {
        let mut levels = [0; 16];
        levels[0] = 1;
        levels[1] = 1;
        levels[5] = 1;

        assert_eq!(
            inverse_scale_4x4(&levels, 0, None),
            Ok({
                let mut expected = [0; 16];
                expected[0] = 10;
                expected[1] = 13;
                expected[5] = 16;
                expected
            })
        );
        assert_eq!(
            inverse_scale_4x4(&levels, 24, None),
            Ok({
                let mut expected = [0; 16];
                expected[0] = 160;
                expected[1] = 208;
                expected[5] = 256;
                expected
            })
        );
    }

    #[test]
    fn inverse_scale_uses_custom_scaling_list() {
        let levels = [1; 16];
        let scaling_list = [6; 16];
        let scaled = inverse_scale_4x4(&levels, 0, Some(&scaling_list)).unwrap();

        assert_eq!(scaled[0], 4);
        assert_eq!(scaled[1], 5);
        assert_eq!(scaled[5], 6);
    }

    #[test]
    fn inverse_transform_handles_dc_and_zero_blocks() {
        assert_eq!(inverse_transform_4x4(&[0; 16]), Ok([0; 16]));

        let mut coefficients = [0; 16];
        coefficients[0] = 64;
        assert_eq!(inverse_transform_4x4(&coefficients), Ok([1; 16]));

        coefficients = [0; 16];
        coefficients[1] = 64;
        let expected = [1, 1, 0, -1, 1, 1, 0, -1, 1, 1, 0, -1, 1, 1, 0, -1];
        assert_eq!(inverse_transform_4x4(&coefficients), Ok(expected));
    }

    #[test]
    fn combined_inverse_scale_and_transform_produces_residual() {
        let mut levels = [0; 16];
        levels[0] = 1;
        assert_eq!(reconstruct_residual_4x4(&levels, 0, None), Ok([0; 16]));

        assert_eq!(reconstruct_residual_4x4(&levels, 24, None), Ok([3; 16]));
    }

    #[test]
    fn rejects_invalid_qp_scaling_and_overflow() {
        assert_eq!(
            inverse_scale_4x4(&[0; 16], 52, None),
            Err(TransformError::InvalidQp)
        );

        assert_eq!(
            inverse_scale_4x4(&[0; 16], 0, Some(&[0; 16])),
            Err(TransformError::InvalidScalingFactor)
        );

        let mut levels = [0; 16];
        levels[0] = i32::MAX;
        assert_eq!(
            inverse_scale_4x4(&levels, 51, None),
            Err(TransformError::Overflow)
        );

        let mut coefficients = [0; 16];
        coefficients[0] = i32::MAX;
        assert!(inverse_transform_4x4(&coefficients).is_ok());
    }
}
