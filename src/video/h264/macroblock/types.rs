#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intra16x16Mode {
    Vertical,
    Horizontal,
    Dc,
    Plane,
}

impl TryFrom<u8> for Intra16x16Mode {
    type Error = &'static str;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Vertical),
            1 => Ok(Self::Horizontal),
            2 => Ok(Self::Dc),
            3 => Ok(Self::Plane),
            _ => Err("Invalid Intra16x16 prediction mode"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroblockType {
    Intra4x4,
    Intra16x16 {
        prediction_mode: Intra16x16Mode,
        coded_chroma_pattern: u8,
        coded_luma_pattern: u8,
    },
    P16x16 {
        reference_index: u32,
        motion_vector_difference: [i32; 2],
        coded_block_pattern: u8,
    },
    PSkip,
    Ipcm,
}

impl MacroblockType {
    /// Maps mb_type values for an H.264 I-slice.
    pub fn from_i_slice_mb_type(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Intra4x4),

            1..=24 => {
                let code = value - 1;

                let prediction_mode = Intra16x16Mode::try_from((code % 4) as u8).ok()?;

                let coded_chroma_pattern = ((code / 4) % 3) as u8;

                let coded_luma_pattern = if code / 12 == 0 { 0 } else { 15 };

                Some(Self::Intra16x16 {
                    prediction_mode,
                    coded_chroma_pattern,
                    coded_luma_pattern,
                })
            }

            25 => Some(Self::Ipcm),

            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResidualBlock {
    pub coefficients: [i32; 16],
    pub total_coeff: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Macroblock {
    pub address: u32,
    pub mb_type: MacroblockType,
    pub coded_block_pattern: u8,
    pub qp_delta: Option<i32>,
    pub luma_residual: Vec<ResidualBlock>,
    pub chroma_cb_residual: Vec<ResidualBlock>,
    pub chroma_cr_residual: Vec<ResidualBlock>,
}

impl Macroblock {
    pub fn new(address: u32, mb_type: MacroblockType) -> Self {
        Self {
            address,
            mb_type,
            coded_block_pattern: 0,
            qp_delta: None,
            luma_residual: Vec::new(),
            chroma_cb_residual: Vec::new(),
            chroma_cr_residual: Vec::new(),
        }
    }
}
