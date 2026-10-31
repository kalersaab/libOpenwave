use crate::video::h264::slice::PredWeightTable;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeightedSample {
    pub weight: i32,
    pub offset: i32,
}

impl WeightedSample {
    pub fn new(weight: i32, offset: i32) -> Self {
        Self { weight, offset }
    }

    pub fn apply(&self, sample: u8, log2_weight_denom: u32) -> u8 {
        let scaled = i32::from(sample) * self.weight;
        let shifted = (scaled + (1 << (log2_weight_denom - 1))) >> log2_weight_denom;
        let result = shifted + self.offset;
        result.clamp(0, 255) as u8
    }

    pub fn apply_bi(
        weight0: i32,
        offset0: i32,
        sample0: u8,
        weight1: i32,
        offset1: i32,
        sample1: u8,
        log2_weight_denom: u32,
    ) -> u8 {
        let scaled0 = i32::from(sample0) * weight0;
        let scaled1 = i32::from(sample1) * weight1;

        let sum = scaled0 + scaled1;
        let shifted = (sum + (1 << log2_weight_denom)) >> (log2_weight_denom + 1);
        let result = shifted + ((offset0 + offset1 + 1) >> 1);

        result.clamp(0, 255) as u8
    }
}

pub struct WeightedPrediction {
    pub luma_log2_weight_denom: u32,
    pub chroma_log2_weight_denom: u32,
}

impl WeightedPrediction {
    pub fn from_weight_table(table: &PredWeightTable) -> Self {
        Self {
            luma_log2_weight_denom: table.luma_log2_weight_denom,
            chroma_log2_weight_denom: table.chroma_log2_weight_denom.unwrap_or(0),
        }
    }

    pub fn implicit(log2_weight_denom: u32) -> Self {
        Self {
            luma_log2_weight_denom: log2_weight_denom,
            chroma_log2_weight_denom: log2_weight_denom,
        }
    }

    pub fn apply_luma(&self, sample: u8, weight: i32, offset: i32) -> u8 {
        WeightedSample::new(weight, offset).apply(sample, self.luma_log2_weight_denom)
    }

    pub fn apply_chroma(&self, sample: u8, weight: i32, offset: i32) -> u8 {
        WeightedSample::new(weight, offset).apply(sample, self.chroma_log2_weight_denom)
    }

    pub fn apply_luma_bi(
        &self,
        weight0: i32,
        offset0: i32,
        sample0: u8,
        weight1: i32,
        offset1: i32,
        sample1: u8,
    ) -> u8 {
        WeightedSample::apply_bi(
            weight0,
            offset0,
            sample0,
            weight1,
            offset1,
            sample1,
            self.luma_log2_weight_denom,
        )
    }

    pub fn apply_chroma_bi(
        &self,
        weight0: i32,
        offset0: i32,
        sample0: u8,
        weight1: i32,
        offset1: i32,
        sample1: u8,
    ) -> u8 {
        WeightedSample::apply_bi(
            weight0,
            offset0,
            sample0,
            weight1,
            offset1,
            sample1,
            self.chroma_log2_weight_denom,
        )
    }

    pub fn implicit_weight(
        dist_scale_factor: i32,
        reference_index0: usize,
        reference_index1: usize,
    ) -> (i32, i32) {
        if reference_index0 == reference_index1 {
            (32, 32)
        } else {
            let w0 = (64 - dist_scale_factor).clamp(0, 128);
            let w1 = dist_scale_factor.clamp(0, 128);
            (w0, w1)
        }
    }

    pub fn calculate_dist_scale_factor(current_poc: i64, ref0_poc: i64, ref1_poc: i64) -> i32 {
        let tb = (current_poc - ref0_poc).abs();
        let td = (ref1_poc - ref0_poc).abs();

        if td == 0 || tb >= td {
            return 32;
        }

        let numerator = tb * 256 + 128;
        let denominator = td * 2;
        let scale = (numerator / denominator).clamp(0, 128);

        scale as i32
    }
}

pub type WeightResult = (Option<(i32, i32)>, Option<[(i32, i32); 2]>);

pub fn get_explicit_weights(
    table: &PredWeightTable,
    list_idx: usize,
    ref_idx: usize,
) -> Option<WeightResult> {
    let weights = match list_idx {
        0 => table.list0.get(ref_idx)?,
        1 => table.list1.get(ref_idx)?,
        _ => return None,
    };

    Some((weights.luma, weights.chroma))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_explicit_luma_weighting() {
        let wp = WeightedPrediction {
            luma_log2_weight_denom: 5,
            chroma_log2_weight_denom: 5,
        };

        let result = wp.apply_luma(128, 32, 0);
        assert_eq!(result, 128);

        let result = wp.apply_luma(100, 64, -32);
        assert_eq!(result, 168);
    }

    #[test]
    fn applies_explicit_chroma_weighting() {
        let wp = WeightedPrediction {
            luma_log2_weight_denom: 5,
            chroma_log2_weight_denom: 6,
        };

        let result = wp.apply_chroma(64, 64, 16);
        assert_eq!(result, 80);
    }

    #[test]
    fn clips_weighted_samples_to_valid_range() {
        let wp = WeightedPrediction {
            luma_log2_weight_denom: 5,
            chroma_log2_weight_denom: 5,
        };

        let result = wp.apply_luma(255, 128, 100);
        assert_eq!(result, 255);

        let result = wp.apply_luma(10, 8, -50);
        assert_eq!(result, 0);
    }

    #[test]
    fn applies_bidirectional_weighted_prediction() {
        let wp = WeightedPrediction {
            luma_log2_weight_denom: 5,
            chroma_log2_weight_denom: 5,
        };

        let result = wp.apply_luma_bi(32, 0, 100, 32, 0, 140);

        assert_eq!(result, 120);
    }

    #[test]
    fn calculates_implicit_weights_from_temporal_distance() {
        let scale = WeightedPrediction::calculate_dist_scale_factor(2, 0, 4);

        assert!((75..=85).contains(&scale));

        let scale = WeightedPrediction::calculate_dist_scale_factor(1, 0, 4);

        assert!(scale <= 55);
    }

    #[test]
    fn implicit_weights_are_symmetric() {
        let (w0, w1) = WeightedPrediction::implicit_weight(32, 0, 1);
        assert_eq!(w0 + w1, 64);
    }

    #[test]
    fn applies_rounding_before_shift() {
        let ws = WeightedSample::new(32, 0);

        let result = ws.apply(128, 5);
        assert_eq!(result, 128);

        let result = ws.apply(129, 5);
        assert_eq!(result, 129);
    }

    #[test]
    fn bidirectional_applies_proper_rounding() {
        let result = WeightedSample::apply_bi(32, 0, 100, 32, 0, 101, 5);

        assert!(result == 100 || result == 101);
    }
}
