pub const SCALE_FACTOR_BAND_LONG: [usize; 23] = [
    0, 4, 8, 12, 16, 20, 24, 30, 36, 44, 52, 62, 74, 90, 110, 134, 162, 196, 238, 288, 342, 418,
    576,
];

pub const SCALE_FACTOR_BAND_SHORT: [usize; 14] =
    [0, 4, 8, 12, 16, 22, 30, 40, 52, 66, 84, 106, 136, 192];

pub fn get_sfb_long() -> &'static [usize] {
    &SCALE_FACTOR_BAND_LONG
}

pub fn get_sfb_short() -> &'static [usize] {
    &SCALE_FACTOR_BAND_SHORT
}

pub fn pow43(x: f32) -> f32 {
    if x < 0.0 {
        -(-x).powf(4.0 / 3.0)
    } else {
        x.powf(4.0 / 3.0)
    }
}

pub fn dequantize_sample(sample: i16, global_gain: u8, scale_factor: f32) -> f32 {
    let sign = if sample < 0 { -1.0 } else { 1.0 };
    let abs_sample = sample.abs() as f32;

    let base = pow43(abs_sample);
    let gain = 2.0f32.powf(0.25 * (global_gain as f32 - 210.0));

    sign * base * gain * scale_factor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sfb_long_has_23_bands() {
        assert_eq!(SCALE_FACTOR_BAND_LONG.len(), 23);
    }

    #[test]
    fn sfb_short_has_14_bands() {
        assert_eq!(SCALE_FACTOR_BAND_SHORT.len(), 14);
    }

    #[test]
    fn sfb_long_ends_at_576() {
        assert_eq!(SCALE_FACTOR_BAND_LONG[22], 576);
    }

    #[test]
    fn sfb_short_ends_at_192() {
        assert_eq!(SCALE_FACTOR_BAND_SHORT[13], 192);
    }

    #[test]
    fn pow43_positive() {
        let result = pow43(8.0);
        assert!((result - 16.0).abs() < 0.001);
    }

    #[test]
    fn pow43_negative() {
        let result = pow43(-8.0);
        assert!((result + 16.0).abs() < 0.001);
    }

    #[test]
    fn dequantize_zero() {
        let result = dequantize_sample(0, 128, 1.0);
        assert_eq!(result, 0.0);
    }

    #[test]
    fn dequantize_preserves_sign() {
        let pos = dequantize_sample(4, 128, 1.0);
        let neg = dequantize_sample(-4, 128, 1.0);

        assert!(pos > 0.0);
        assert!(neg < 0.0);
        assert!((pos + neg).abs() < 0.001);
    }
}
