use crate::video::h264::macroblock::MacroblockType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeType {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundaryStrength(pub u8);

impl BoundaryStrength {
    pub const NONE: Self = Self(0);
    pub const WEAK: Self = Self(1);
    pub const MEDIUM: Self = Self(2);
    pub const STRONG_INTRA: Self = Self(3);
    pub const STRONG: Self = Self(4);

    pub fn value(&self) -> u8 {
        self.0
    }

    pub fn is_filtering_enabled(&self) -> bool {
        self.0 > 0
    }
}

pub struct BsContext {
    pub mb_type: MacroblockType,
    pub neighbor_mb_type: Option<MacroblockType>,
    pub has_coded_residual: bool,
    pub neighbor_has_coded_residual: bool,
    pub ref_idx: [i32; 2],
    pub neighbor_ref_idx: [i32; 2],
    pub mv: [[i32; 2]; 2],
    pub neighbor_mv: [[i32; 2]; 2],
}

pub fn calculate_bs_4x4(
    edge_type: EdgeType,
    edge_idx: usize,
    ctx: &BsContext,
    is_mb_edge: bool,
) -> BoundaryStrength {
    if is_mb_edge {
        calculate_bs_mb_edge(ctx)
    } else {
        calculate_bs_internal_edge(edge_type, edge_idx, ctx)
    }
}

fn calculate_bs_mb_edge(ctx: &BsContext) -> BoundaryStrength {
    let is_intra = matches!(
        ctx.mb_type,
        MacroblockType::Intra4x4 | MacroblockType::Intra16x16 { .. } | MacroblockType::Ipcm
    );

    let neighbor_is_intra = ctx
        .neighbor_mb_type
        .as_ref()
        .map(|t| {
            matches!(
                t,
                MacroblockType::Intra4x4 | MacroblockType::Intra16x16 { .. } | MacroblockType::Ipcm
            )
        })
        .unwrap_or(false);

    if is_intra || neighbor_is_intra {
        return BoundaryStrength::STRONG;
    }

    if ctx.has_coded_residual || ctx.neighbor_has_coded_residual {
        return BoundaryStrength::MEDIUM;
    }

    if ctx.ref_idx[0] != ctx.neighbor_ref_idx[0] || ctx.ref_idx[1] != ctx.neighbor_ref_idx[1] {
        return BoundaryStrength::WEAK;
    }

    let mv_diff_l0 = [
        (ctx.mv[0][0] - ctx.neighbor_mv[0][0]).abs(),
        (ctx.mv[0][1] - ctx.neighbor_mv[0][1]).abs(),
    ];

    let mv_diff_l1 = [
        (ctx.mv[1][0] - ctx.neighbor_mv[1][0]).abs(),
        (ctx.mv[1][1] - ctx.neighbor_mv[1][1]).abs(),
    ];

    if mv_diff_l0[0] >= 4 || mv_diff_l0[1] >= 4 || mv_diff_l1[0] >= 4 || mv_diff_l1[1] >= 4 {
        return BoundaryStrength::WEAK;
    }

    BoundaryStrength::NONE
}

fn calculate_bs_internal_edge(
    _edge_type: EdgeType,
    _edge_idx: usize,
    ctx: &BsContext,
) -> BoundaryStrength {
    if ctx.has_coded_residual {
        BoundaryStrength::MEDIUM
    } else {
        BoundaryStrength::NONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_intra_ctx() -> BsContext {
        BsContext {
            mb_type: MacroblockType::Intra16x16 {
                prediction_mode: crate::video::h264::macroblock::Intra16x16Mode::Vertical,
                coded_chroma_pattern: 0,
                coded_luma_pattern: 0,
            },
            neighbor_mb_type: Some(MacroblockType::Intra16x16 {
                prediction_mode: crate::video::h264::macroblock::Intra16x16Mode::Vertical,
                coded_chroma_pattern: 0,
                coded_luma_pattern: 0,
            }),
            has_coded_residual: false,
            neighbor_has_coded_residual: false,
            ref_idx: [0, -1],
            neighbor_ref_idx: [0, -1],
            mv: [[0, 0], [0, 0]],
            neighbor_mv: [[0, 0], [0, 0]],
        }
    }

    fn make_inter_ctx() -> BsContext {
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
    fn bs_intra_edge_is_strong() {
        let ctx = make_intra_ctx();
        let bs = calculate_bs_mb_edge(&ctx);
        assert_eq!(bs, BoundaryStrength::STRONG);
    }

    #[test]
    #[ignore]
    fn bs_with_residual_is_medium() {
        let mut ctx = make_inter_ctx();
        ctx.has_coded_residual = true;
        let bs = calculate_bs_mb_edge(&ctx);
        assert_eq!(bs, BoundaryStrength::MEDIUM);
    }

    #[test]
    fn bs_neighbor_with_residual_is_medium() {
        let mut ctx = make_inter_ctx();
        ctx.neighbor_has_coded_residual = true;
        let bs = calculate_bs_mb_edge(&ctx);
        assert_eq!(bs, BoundaryStrength::MEDIUM);
    }

    #[test]
    fn bs_different_ref_is_weak() {
        let mut ctx = make_inter_ctx();
        ctx.ref_idx = [0, -1];
        ctx.neighbor_ref_idx = [1, -1];
        let bs = calculate_bs_mb_edge(&ctx);
        assert_eq!(bs, BoundaryStrength::WEAK);
    }

    #[test]
    fn bs_large_mv_diff_is_weak() {
        let mut ctx = make_inter_ctx();
        ctx.mv = [[8, 0], [0, 0]];
        ctx.neighbor_mv = [[0, 0], [0, 0]];
        let bs = calculate_bs_mb_edge(&ctx);
        assert_eq!(bs, BoundaryStrength::WEAK);
    }

    #[test]
    fn bs_same_inter_is_none() {
        let ctx = make_inter_ctx();
        let bs = calculate_bs_mb_edge(&ctx);
        assert_eq!(bs, BoundaryStrength::NONE);
    }

    #[test]
    fn bs_internal_with_residual_is_medium() {
        let mut ctx = make_inter_ctx();
        ctx.has_coded_residual = true;
        let bs = calculate_bs_internal_edge(EdgeType::Vertical, 0, &ctx);
        assert_eq!(bs, BoundaryStrength::MEDIUM);
    }

    #[test]
    fn bs_internal_without_residual_is_none() {
        let ctx = make_inter_ctx();
        let bs = calculate_bs_internal_edge(EdgeType::Vertical, 0, &ctx);
        assert_eq!(bs, BoundaryStrength::NONE);
    }
}
