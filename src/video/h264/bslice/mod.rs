use crate::video::h264::{
    frame::{FrameError, Yuv420Frame},
    macroblock::MacroblockType,
    motion::{
        BiPredParams, BiWeightParams, MotionVector, MotionVectorGrid, bipred_macroblock,
        bipred_macroblock_weighted, compensate_macroblock, compensate_macroblock_weighted,
    },
    reference::ReferencePictureLists,
    slice::PredWeightTable,
    weighted::{WeightedPrediction, get_explicit_weights},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BPredictionMode {
    L0Only,

    L1Only,

    Bidirectional,

    Direct,
}

struct BiMbParams {
    mb_x: u32,
    mb_y: u32,
    ref_idx_l0: u32,
    mvd_l0: [i32; 2],
    ref_idx_l1: u32,
    mvd_l1: [i32; 2],
}

pub struct BSliceDecoder<'a> {
    reference_lists: &'a ReferencePictureLists<'a>,
    motion_grid: &'a mut MotionVectorGrid,
    weighted_pred: Option<WeightedPrediction>,
    pred_weight_table: Option<&'a PredWeightTable>,
}

impl<'a> BSliceDecoder<'a> {
    pub fn new(
        reference_lists: &'a ReferencePictureLists<'a>,
        motion_grid: &'a mut MotionVectorGrid,
        pred_weight_table: Option<&'a PredWeightTable>,
    ) -> Self {
        let weighted_pred = pred_weight_table.map(WeightedPrediction::from_weight_table);
        Self {
            reference_lists,
            motion_grid,
            weighted_pred,
            pred_weight_table,
        }
    }

    pub fn decode_macroblock(
        &mut self,
        output: &mut Yuv420Frame,
        mb_type: &MacroblockType,
        mb_x: u32,
        mb_y: u32,
    ) -> Result<(), BSliceError> {
        match mb_type {
            MacroblockType::BDirect16x16 => {
                self.decode_direct_16x16(output, mb_x, mb_y)?;
            }
            MacroblockType::BL0_16x16 {
                reference_index,
                motion_vector_difference,
                coded_block_pattern: _,
            } => {
                self.decode_l0_16x16(
                    output,
                    mb_x,
                    mb_y,
                    *reference_index,
                    *motion_vector_difference,
                )?;
            }
            MacroblockType::BL1_16x16 {
                reference_index,
                motion_vector_difference,
                coded_block_pattern: _,
            } => {
                self.decode_l1_16x16(
                    output,
                    mb_x,
                    mb_y,
                    *reference_index,
                    *motion_vector_difference,
                )?;
            }
            MacroblockType::BBi16x16 {
                reference_index_l0,
                motion_vector_difference_l0,
                reference_index_l1,
                motion_vector_difference_l1,
                coded_block_pattern: _,
            } => {
                self.decode_bi_16x16(
                    output,
                    BiMbParams {
                        mb_x,
                        mb_y,
                        ref_idx_l0: *reference_index_l0,
                        mvd_l0: *motion_vector_difference_l0,
                        ref_idx_l1: *reference_index_l1,
                        mvd_l1: *motion_vector_difference_l1,
                    },
                )?;
            }
            _ => return Err(BSliceError::UnsupportedMacroblockType),
        }

        Ok(())
    }

    fn decode_l0_16x16(
        &mut self,
        output: &mut Yuv420Frame,
        mb_x: u32,
        mb_y: u32,
        reference_index: u32,
        mvd: [i32; 2],
    ) -> Result<(), BSliceError> {
        let motion_pred = self.motion_grid.predict(mb_x, mb_y, reference_index);
        let motion = MotionVector {
            x: motion_pred
                .x
                .checked_add(mvd[0])
                .ok_or(BSliceError::InvalidMotionVector)?,
            y: motion_pred
                .y
                .checked_add(mvd[1])
                .ok_or(BSliceError::InvalidMotionVector)?,
        };

        let reference = self.reference_lists.get_list0(reference_index)?;

        if let Some(weights) = self.get_l0_weights(reference_index) {
            compensate_macroblock_weighted(
                output,
                &reference.frame,
                mb_x,
                mb_y,
                motion,
                Some(weights),
            )?;
        } else {
            compensate_macroblock(output, &reference.frame, mb_x, mb_y, motion)?;
        }

        self.motion_grid
            .set_macroblock(mb_x, mb_y, reference_index, motion);
        Ok(())
    }

    fn decode_l1_16x16(
        &mut self,
        output: &mut Yuv420Frame,
        mb_x: u32,
        mb_y: u32,
        reference_index: u32,
        mvd: [i32; 2],
    ) -> Result<(), BSliceError> {
        let motion_pred = self.motion_grid.predict(mb_x, mb_y, reference_index);
        let motion = MotionVector {
            x: motion_pred
                .x
                .checked_add(mvd[0])
                .ok_or(BSliceError::InvalidMotionVector)?,
            y: motion_pred
                .y
                .checked_add(mvd[1])
                .ok_or(BSliceError::InvalidMotionVector)?,
        };

        let reference = self.reference_lists.get_list1(reference_index)?;

        if let Some(weights) = self.get_l1_weights(reference_index) {
            compensate_macroblock_weighted(
                output,
                &reference.frame,
                mb_x,
                mb_y,
                motion,
                Some(weights),
            )?;
        } else {
            compensate_macroblock(output, &reference.frame, mb_x, mb_y, motion)?;
        }

        self.motion_grid
            .set_macroblock(mb_x, mb_y, reference_index, motion);
        Ok(())
    }

    fn decode_bi_16x16(
        &mut self,
        output: &mut Yuv420Frame,
        params: BiMbParams,
    ) -> Result<(), BSliceError> {
        let motion_pred_l0 = self
            .motion_grid
            .predict(params.mb_x, params.mb_y, params.ref_idx_l0);
        let motion_l0 = MotionVector {
            x: motion_pred_l0
                .x
                .checked_add(params.mvd_l0[0])
                .ok_or(BSliceError::InvalidMotionVector)?,
            y: motion_pred_l0
                .y
                .checked_add(params.mvd_l0[1])
                .ok_or(BSliceError::InvalidMotionVector)?,
        };

        let motion_pred_l1 = self
            .motion_grid
            .predict(params.mb_x, params.mb_y, params.ref_idx_l1);
        let motion_l1 = MotionVector {
            x: motion_pred_l1
                .x
                .checked_add(params.mvd_l1[0])
                .ok_or(BSliceError::InvalidMotionVector)?,
            y: motion_pred_l1
                .y
                .checked_add(params.mvd_l1[1])
                .ok_or(BSliceError::InvalidMotionVector)?,
        };

        let reference_l0 = self.reference_lists.get_list0(params.ref_idx_l0)?;
        let reference_l1 = self.reference_lists.get_list1(params.ref_idx_l1)?;

        if let Some(weights) = self.get_bi_weights(params.ref_idx_l0, params.ref_idx_l1) {
            bipred_macroblock_weighted(
                output,
                BiPredParams {
                    reference_l0: &reference_l0.frame,
                    reference_l1: &reference_l1.frame,
                    mb_x: params.mb_x,
                    mb_y: params.mb_y,
                    motion_l0,
                    motion_l1,
                    weights: Some(weights),
                },
            )?;
        } else {
            bipred_macroblock(
                output,
                &reference_l0.frame,
                &reference_l1.frame,
                params.mb_x,
                params.mb_y,
                motion_l0,
                motion_l1,
            )?;
        }

        self.motion_grid
            .set_macroblock(params.mb_x, params.mb_y, params.ref_idx_l0, motion_l0);
        Ok(())
    }

    fn decode_direct_16x16(
        &mut self,
        output: &mut Yuv420Frame,
        mb_x: u32,
        mb_y: u32,
    ) -> Result<(), BSliceError> {
        let reference_l0 = self.reference_lists.get_list0(0)?;
        let reference_l1 = self.reference_lists.get_list1(0)?;

        bipred_macroblock(
            output,
            &reference_l0.frame,
            &reference_l1.frame,
            mb_x,
            mb_y,
            MotionVector::default(),
            MotionVector::default(),
        )?;

        self.motion_grid
            .set_macroblock(mb_x, mb_y, 0, MotionVector::default());
        Ok(())
    }

    fn get_l0_weights(
        &self,
        ref_idx: u32,
    ) -> Option<super::motion::compensation::WeightParams<'_>> {
        let table = self.pred_weight_table?;
        let wp = self.weighted_pred.as_ref()?;
        let (luma, chroma) = get_explicit_weights(table, 0, ref_idx as usize)?;
        Some((wp, luma, chroma))
    }

    fn get_l1_weights(
        &self,
        ref_idx: u32,
    ) -> Option<super::motion::compensation::WeightParams<'_>> {
        let table = self.pred_weight_table?;
        let wp = self.weighted_pred.as_ref()?;
        let (luma, chroma) = get_explicit_weights(table, 1, ref_idx as usize)?;
        Some((wp, luma, chroma))
    }

    fn get_bi_weights(&self, ref_idx_l0: u32, ref_idx_l1: u32) -> Option<BiWeightParams<'_>> {
        let table = self.pred_weight_table?;
        let wp = self.weighted_pred.as_ref()?;

        let (luma_l0, chroma_l0) = get_explicit_weights(table, 0, ref_idx_l0 as usize)?;
        let (luma_l1, chroma_l1) = get_explicit_weights(table, 1, ref_idx_l1 as usize)?;

        let luma_weights = [luma_l0?, luma_l1?];
        let chroma_weights = [chroma_l0?, chroma_l1?];

        Some((wp, luma_weights, chroma_weights))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BSliceError {
    Frame(FrameError),
    InvalidMotionVector,
    InvalidReferenceIndex,
    UnsupportedMacroblockType,
}

impl From<FrameError> for BSliceError {
    fn from(err: FrameError) -> Self {
        Self::Frame(err)
    }
}

impl From<crate::video::h264::decoder::H264DecodeError> for BSliceError {
    fn from(_: crate::video::h264::decoder::H264DecodeError) -> Self {
        Self::InvalidReferenceIndex
    }
}

impl std::fmt::Display for BSliceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "B-slice error: {:?}", self)
    }
}

impl std::error::Error for BSliceError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::h264::reference::ReferencePicture;

    fn make_test_reference(poc: i64) -> ReferencePicture {
        let mut frame = Yuv420Frame::new(32, 32).unwrap();
        frame.y.fill((poc * 10) as u8);
        ReferencePicture::new(poc as u32, poc, frame)
    }

    #[test]
    fn decodes_l0_forward_prediction() {
        let ref0 = make_test_reference(0);
        let ref1 = make_test_reference(1);

        let refs = vec![&ref0, &ref1];
        let lists = ReferencePictureLists {
            list0: refs.clone(),
            list1: refs,
        };

        let mut motion_grid = MotionVectorGrid::new(8, 8).unwrap();
        let mut decoder = BSliceDecoder::new(&lists, &mut motion_grid, None);

        let mut output = Yuv420Frame::new(32, 32).unwrap();
        let mb_type = MacroblockType::BL0_16x16 {
            reference_index: 0,
            motion_vector_difference: [0, 0],
            coded_block_pattern: 0,
        };

        decoder
            .decode_macroblock(&mut output, &mb_type, 0, 0)
            .unwrap();

        assert_eq!(output.y[0], 0);
    }

    #[test]
    fn decodes_l1_backward_prediction() {
        let ref0 = make_test_reference(0);
        let ref1 = make_test_reference(2);

        let lists = ReferencePictureLists {
            list0: vec![&ref0],
            list1: vec![&ref1],
        };

        let mut motion_grid = MotionVectorGrid::new(8, 8).unwrap();
        let mut decoder = BSliceDecoder::new(&lists, &mut motion_grid, None);

        let mut output = Yuv420Frame::new(32, 32).unwrap();
        let mb_type = MacroblockType::BL1_16x16 {
            reference_index: 0,
            motion_vector_difference: [0, 0],
            coded_block_pattern: 0,
        };

        decoder
            .decode_macroblock(&mut output, &mb_type, 0, 0)
            .unwrap();

        assert_eq!(output.y[0], 20);
    }

    #[test]
    fn decodes_bidirectional_prediction() {
        let ref0 = make_test_reference(0);
        let ref1 = make_test_reference(4);

        let lists = ReferencePictureLists {
            list0: vec![&ref0],
            list1: vec![&ref1],
        };

        let mut motion_grid = MotionVectorGrid::new(8, 8).unwrap();
        let mut decoder = BSliceDecoder::new(&lists, &mut motion_grid, None);

        let mut output = Yuv420Frame::new(32, 32).unwrap();
        let mb_type = MacroblockType::BBi16x16 {
            reference_index_l0: 0,
            motion_vector_difference_l0: [0, 0],
            reference_index_l1: 0,
            motion_vector_difference_l1: [0, 0],
            coded_block_pattern: 0,
        };

        decoder
            .decode_macroblock(&mut output, &mb_type, 0, 0)
            .unwrap();

        assert_eq!(output.y[0], 20);
    }

    #[test]
    fn decodes_direct_mode() {
        let ref0 = make_test_reference(1);
        let ref1 = make_test_reference(3);

        let lists = ReferencePictureLists {
            list0: vec![&ref0],
            list1: vec![&ref1],
        };

        let mut motion_grid = MotionVectorGrid::new(8, 8).unwrap();
        let mut decoder = BSliceDecoder::new(&lists, &mut motion_grid, None);

        let mut output = Yuv420Frame::new(32, 32).unwrap();
        let mb_type = MacroblockType::BDirect16x16;

        decoder
            .decode_macroblock(&mut output, &mb_type, 0, 0)
            .unwrap();

        assert_eq!(output.y[0], 20);
    }
}
