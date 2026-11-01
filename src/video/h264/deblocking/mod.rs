pub mod boundary_strength;
pub mod filter;
pub mod tables;

use crate::video::h264::frame::Yuv420Frame;
use boundary_strength::{BsContext, EdgeType};
use filter::{filter_chroma_edge, filter_luma_edge_bs_lt4, filter_luma_edge_bs4};

pub struct DeblockingFilter {
    disable_deblocking_filter_idc: u32,
    alpha_c0_offset: i32,
    _beta_offset: i32,
}

impl DeblockingFilter {
    pub fn new(disable_deblocking_filter_idc: u32, alpha_c0_offset: i32, beta_offset: i32) -> Self {
        Self {
            disable_deblocking_filter_idc,
            alpha_c0_offset,
            _beta_offset: beta_offset,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.disable_deblocking_filter_idc != 1
    }

    pub fn filter_macroblock(
        &self,
        frame: &mut Yuv420Frame,
        mb_x: u32,
        mb_y: u32,
        qp_y: u8,
        qp_c: u8,
        bs_ctx: &BsContext,
    ) {
        if !self.is_enabled() {
            return;
        }

        let _mb_width = frame.width / 16;
        let skip_left = mb_x == 0 || self.disable_deblocking_filter_idc == 2;
        let skip_top = mb_y == 0 || self.disable_deblocking_filter_idc == 2;

        if !skip_left {
            self.filter_vertical_mb_edge(frame, mb_x, mb_y, qp_y, qp_c, bs_ctx);
        }

        if !skip_top {
            self.filter_horizontal_mb_edge(frame, mb_x, mb_y, qp_y, qp_c, bs_ctx);
        }

        for edge_idx in 1..4 {
            self.filter_vertical_internal_edge(frame, mb_x, mb_y, edge_idx, qp_y, qp_c, bs_ctx);
            self.filter_horizontal_internal_edge(frame, mb_x, mb_y, edge_idx, qp_y, qp_c, bs_ctx);
        }
    }

    fn filter_vertical_mb_edge(
        &self,
        frame: &mut Yuv420Frame,
        mb_x: u32,
        mb_y: u32,
        qp_y: u8,
        qp_c: u8,
        bs_ctx: &BsContext,
    ) {
        let bs = boundary_strength::calculate_bs_4x4(EdgeType::Vertical, 0, bs_ctx, true);

        if !bs.is_filtering_enabled() {
            return;
        }

        let x = (mb_x * 16) as usize;
        let y = (mb_y * 16) as usize;
        let stride = frame.width as usize;

        if x < 4 {
            return;
        }

        let start = y * stride + x - 4;
        let pixels = &mut frame.y[start..start + stride * 16];

        if bs.value() == 4 {
            let index_a = (qp_y as i32 + self.alpha_c0_offset).clamp(0, 51) as usize;
            let alpha = tables::ALPHA_TABLE[index_a];
            let beta = tables::BETA_TABLE[index_a];
            filter_luma_edge_bs4(pixels, stride, alpha, beta, true);
        } else {
            filter_luma_edge_bs_lt4(pixels, stride, bs.value(), qp_y, true);
        }

        let chroma_x = (mb_x * 8) as usize;
        let chroma_y = (mb_y * 8) as usize;
        let chroma_stride = (frame.width / 2) as usize;

        if chroma_x >= 4 {
            let start_u = chroma_y * chroma_stride + chroma_x - 4;
            let pixels_u = &mut frame.u[start_u..start_u + chroma_stride * 8];
            filter_chroma_edge(pixels_u, chroma_stride, bs.value(), qp_c, true);

            let start_v = chroma_y * chroma_stride + chroma_x - 4;
            let pixels_v = &mut frame.v[start_v..start_v + chroma_stride * 8];
            filter_chroma_edge(pixels_v, chroma_stride, bs.value(), qp_c, true);
        }
    }

    fn filter_horizontal_mb_edge(
        &self,
        frame: &mut Yuv420Frame,
        mb_x: u32,
        mb_y: u32,
        qp_y: u8,
        qp_c: u8,
        bs_ctx: &BsContext,
    ) {
        let bs = boundary_strength::calculate_bs_4x4(EdgeType::Horizontal, 0, bs_ctx, true);

        if !bs.is_filtering_enabled() {
            return;
        }

        let x = (mb_x * 16) as usize;
        let y = (mb_y * 16) as usize;
        let stride = frame.width as usize;

        if y < 4 {
            return;
        }

        let start = (y - 4) * stride + x;
        let pixels = &mut frame.y[start..];

        if bs.value() == 4 {
            let index_a = (qp_y as i32 + self.alpha_c0_offset).clamp(0, 51) as usize;
            let alpha = tables::ALPHA_TABLE[index_a];
            let beta = tables::BETA_TABLE[index_a];
            filter_luma_edge_bs4(pixels, stride, alpha, beta, false);
        } else {
            filter_luma_edge_bs_lt4(pixels, stride, bs.value(), qp_y, false);
        }

        let chroma_x = (mb_x * 8) as usize;
        let chroma_y = (mb_y * 8) as usize;
        let chroma_stride = (frame.width / 2) as usize;

        if chroma_y >= 4 {
            let start_u = (chroma_y - 4) * chroma_stride + chroma_x;
            let pixels_u = &mut frame.u[start_u..];
            filter_chroma_edge(pixels_u, chroma_stride, bs.value(), qp_c, false);

            let start_v = (chroma_y - 4) * chroma_stride + chroma_x;
            let pixels_v = &mut frame.v[start_v..];
            filter_chroma_edge(pixels_v, chroma_stride, bs.value(), qp_c, false);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn filter_vertical_internal_edge(
        &self,
        frame: &mut Yuv420Frame,
        mb_x: u32,
        mb_y: u32,
        edge_idx: usize,
        qp_y: u8,
        _qp_c: u8,
        bs_ctx: &BsContext,
    ) {
        let bs = boundary_strength::calculate_bs_4x4(EdgeType::Vertical, edge_idx, bs_ctx, false);

        if !bs.is_filtering_enabled() {
            return;
        }

        let x = (mb_x * 16 + edge_idx as u32 * 4) as usize;
        let y = (mb_y * 16) as usize;
        let stride = frame.width as usize;

        if x < 4 {
            return;
        }

        let start = y * stride + x - 4;
        let pixels = &mut frame.y[start..start + stride * 16];

        if bs.value() == 4 {
            let index_a = (qp_y as i32 + self.alpha_c0_offset).clamp(0, 51) as usize;
            let alpha = tables::ALPHA_TABLE[index_a];
            let beta = tables::BETA_TABLE[index_a];
            filter_luma_edge_bs4(pixels, stride, alpha, beta, true);
        } else {
            filter_luma_edge_bs_lt4(pixels, stride, bs.value(), qp_y, true);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn filter_horizontal_internal_edge(
        &self,
        frame: &mut Yuv420Frame,
        mb_x: u32,
        mb_y: u32,
        edge_idx: usize,
        qp_y: u8,
        _qp_c: u8,
        bs_ctx: &BsContext,
    ) {
        let bs = boundary_strength::calculate_bs_4x4(EdgeType::Horizontal, edge_idx, bs_ctx, false);

        if !bs.is_filtering_enabled() {
            return;
        }

        let x = (mb_x * 16) as usize;
        let y = (mb_y * 16 + edge_idx as u32 * 4) as usize;
        let stride = frame.width as usize;

        if y < 4 {
            return;
        }

        let start = (y - 4) * stride + x;
        let pixels = &mut frame.y[start..];

        if bs.value() == 4 {
            let index_a = (qp_y as i32 + self.alpha_c0_offset).clamp(0, 51) as usize;
            let alpha = tables::ALPHA_TABLE[index_a];
            let beta = tables::BETA_TABLE[index_a];
            filter_luma_edge_bs4(pixels, stride, alpha, beta, false);
        } else {
            filter_luma_edge_bs_lt4(pixels, stride, bs.value(), qp_y, false);
        }
    }
}

impl Default for DeblockingFilter {
    fn default() -> Self {
        Self::new(0, 0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::h264::macroblock::MacroblockType;

    fn make_test_frame() -> Yuv420Frame {
        Yuv420Frame::new(32, 32).unwrap()
    }

    fn make_test_ctx() -> BsContext {
        BsContext {
            mb_type: MacroblockType::P16x16 {
                reference_index: 0,
                motion_vector_difference: [0, 0],
                coded_block_pattern: 0,
            },
            neighbor_mb_type: Some(MacroblockType::P16x16 {
                reference_index: 0,
                motion_vector_difference: [0, 0],
                coded_block_pattern: 0,
            }),
            has_coded_residual: false,
            neighbor_has_coded_residual: false,
            ref_idx: [0, -1],
            neighbor_ref_idx: [0, -1],
            mv: [[0, 0], [0, 0]],
            neighbor_mv: [[0, 0], [0, 0]],
        }
    }

    #[test]
    fn deblocking_filter_can_be_created() {
        let filter = DeblockingFilter::new(0, 0, 0);
        assert!(filter.is_enabled());
    }

    #[test]
    fn deblocking_filter_can_be_disabled() {
        let filter = DeblockingFilter::new(1, 0, 0);
        assert!(!filter.is_enabled());
    }

    #[test]
    fn filter_macroblock_runs_without_panic() {
        let filter = DeblockingFilter::default();
        let mut frame = make_test_frame();
        let ctx = make_test_ctx();

        filter.filter_macroblock(&mut frame, 1, 1, 26, 26, &ctx);
    }

    #[test]
    fn disabled_filter_does_not_modify_frame() {
        let filter = DeblockingFilter::new(1, 0, 0);
        let mut frame = make_test_frame();
        let original = frame.clone();
        let ctx = make_test_ctx();

        filter.filter_macroblock(&mut frame, 1, 1, 26, 26, &ctx);

        assert_eq!(frame.y, original.y);
        assert_eq!(frame.u, original.u);
        assert_eq!(frame.v, original.v);
    }
}
