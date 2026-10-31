use std::collections::HashMap;

use crate::video::{
    bitstream::{BitReader, BitstreamError},
    h264::{
        cavlc::{CavlcError, Residual4x4Kind, decode_residual_4x4},
        frame::{ColorPlane, FrameError, Yuv420Frame},
        intra::{Intra16x16Mode, Intra16x16References, IntraPredictionError, predict_intra16x16},
        macroblock::{MacroblockParseError, MacroblockType, parse_p_macroblock},
        motion::{chroma::interpolate_chroma_sample, luma::interpolate_luma_sample},
        nal::{AnnexBError, NalUnitType, parse_annex_b},
        pps::{Pps, PpsError, parse_pps},
        slice::{SliceError, SliceType, parse_slice_header},
        sps::{Sps, SpsError, parse_sps},
        transform::{TransformError, inverse_scale_4x4, inverse_transform_4x4},
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum H264DecodeError {
    AnnexB(AnnexBError),
    Bitstream(BitstreamError),
    Sps(SpsError),
    Pps(PpsError),
    Slice(SliceError),
    Cavlc(CavlcError),
    Transform(TransformError),
    Prediction(IntraPredictionError),
    Macroblock(MacroblockParseError),
    Frame(FrameError),
    MissingParameterSet,
    MissingIdrSlice,
    MultipleSlicesUnsupported,
    InvalidReferenceIndex { index: u32, list_length: usize },
    UnsupportedFeature,
    InvalidStream,
}

impl std::fmt::Display for H264DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "H.264 decode error: {self:?}")
    }
}

impl std::error::Error for H264DecodeError {}

macro_rules! from_error {
    ($source:ty, $variant:ident) => {
        impl From<$source> for H264DecodeError {
            fn from(value: $source) -> Self {
                Self::$variant(value)
            }
        }
    };
}

from_error!(AnnexBError, AnnexB);
from_error!(BitstreamError, Bitstream);
from_error!(SpsError, Sps);
from_error!(PpsError, Pps);
from_error!(SliceError, Slice);
from_error!(CavlcError, Cavlc);
from_error!(TransformError, Transform);
from_error!(IntraPredictionError, Prediction);
from_error!(MacroblockParseError, Macroblock);
from_error!(FrameError, Frame);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencePicture {
    pub frame_num: u32,
    pub pic_order_cnt: i64,
    pub long_term_frame_idx: Option<u32>,
    pub frame: Yuv420Frame,
}

#[derive(Default)]
struct ReferencePictureBuffer {
    pictures: Vec<ReferencePicture>,
}

impl ReferencePictureBuffer {
    fn clear(&mut self) {
        self.pictures.clear();
    }

    fn pictures(&self) -> &[ReferencePicture] {
        &self.pictures
    }

    fn insert(
        &mut self,
        picture: ReferencePicture,
        max_num_ref_frames: u32,
        current_frame_num: u32,
        max_frame_num: u32,
    ) -> Result<(), H264DecodeError> {
        let capacity =
            usize::try_from(max_num_ref_frames).map_err(|_| H264DecodeError::InvalidStream)?;
        if capacity == 0 {
            return Ok(());
        }

        while self.pictures.len() >= capacity {
            let oldest_short_term = self
                .pictures
                .iter()
                .enumerate()
                .filter(|(_, picture)| picture.long_term_frame_idx.is_none())
                .min_by_key(|(_, picture)| {
                    frame_num_wrap(picture.frame_num, current_frame_num, max_frame_num)
                })
                .map(|(index, _)| index);
            let Some(index) = oldest_short_term else {
                return Err(H264DecodeError::InvalidStream);
            };
            self.pictures.remove(index);
        }
        self.pictures.push(picture);
        Ok(())
    }
}

struct ReferencePictureLists<'a> {
    list0: Vec<&'a ReferencePicture>,
    list1: Vec<&'a ReferencePicture>,
}

struct ReferencePictureListConfig {
    slice_type: SliceType,
    current_frame_num: u32,
    max_frame_num: u32,
    current_pic_order_cnt: i64,
    max_pic_order_cnt: Option<i64>,
    list0_count: u32,
    list1_count: u32,
}

impl<'a> ReferencePictureLists<'a> {
    fn build(
        pictures: &'a [ReferencePicture],
        config: ReferencePictureListConfig,
    ) -> Result<Self, H264DecodeError> {
        let mut short_term = pictures
            .iter()
            .filter(|picture| picture.long_term_frame_idx.is_none())
            .collect::<Vec<_>>();
        let mut long_term = pictures
            .iter()
            .filter(|picture| picture.long_term_frame_idx.is_some())
            .collect::<Vec<_>>();
        long_term.sort_by_key(|picture| picture.long_term_frame_idx);

        let (mut list0, mut list1) = match config.slice_type {
            SliceType::P => {
                short_term.sort_by_key(|picture| {
                    std::cmp::Reverse(frame_num_wrap(
                        picture.frame_num,
                        config.current_frame_num,
                        config.max_frame_num,
                    ))
                });
                let mut list0 = short_term;
                list0.extend(long_term);
                (list0, Vec::new())
            }
            SliceType::B => {
                let mut before = Vec::new();
                let mut after = Vec::new();
                for picture in short_term {
                    let order = unwrap_pic_order_cnt(
                        picture.pic_order_cnt,
                        config.current_pic_order_cnt,
                        config.max_pic_order_cnt,
                    );
                    if order < config.current_pic_order_cnt {
                        before.push((order, picture));
                    } else {
                        after.push((order, picture));
                    }
                }
                before.sort_by_key(|(order, _)| std::cmp::Reverse(*order));
                after.sort_by_key(|(order, _)| *order);
                let mut list0 = before
                    .iter()
                    .chain(after.iter())
                    .map(|(_, picture)| *picture)
                    .collect::<Vec<_>>();
                let mut list1 = after
                    .iter()
                    .chain(before.iter())
                    .map(|(_, picture)| *picture)
                    .collect::<Vec<_>>();
                list0.extend(long_term.iter().copied());
                list1.extend(long_term);
                if list0.len() > 1
                    && list0.len() == list1.len()
                    && list0
                        .iter()
                        .zip(&list1)
                        .all(|(left, right)| std::ptr::eq(*left, *right))
                {
                    list1.swap(0, 1);
                }
                (list0, list1)
            }
            _ => (Vec::new(), Vec::new()),
        };

        list0.truncate(
            usize::try_from(config.list0_count).map_err(|_| H264DecodeError::InvalidStream)?,
        );
        list1.truncate(
            usize::try_from(config.list1_count).map_err(|_| H264DecodeError::InvalidStream)?,
        );
        validate_reference_list_length(config.list0_count, list0.len())?;
        if config.slice_type == SliceType::B {
            validate_reference_list_length(config.list1_count, list1.len())?;
        }

        Ok(Self { list0, list1 })
    }
}

fn validate_reference_list_length(
    active_count: u32,
    list_length: usize,
) -> Result<(), H264DecodeError> {
    let active_count = usize::try_from(active_count).map_err(|_| H264DecodeError::InvalidStream)?;
    if active_count > list_length {
        return Err(H264DecodeError::InvalidReferenceIndex {
            index: u32::try_from(list_length).unwrap_or(u32::MAX),
            list_length,
        });
    }
    Ok(())
}

fn frame_num_wrap(frame_num: u32, current_frame_num: u32, max_frame_num: u32) -> i64 {
    if frame_num > current_frame_num {
        i64::from(frame_num) - i64::from(max_frame_num)
    } else {
        i64::from(frame_num)
    }
}

fn unwrap_pic_order_cnt(
    reference_pic_order_cnt: i64,
    current_pic_order_cnt: i64,
    max_pic_order_cnt: Option<i64>,
) -> i64 {
    let Some(max_pic_order_cnt) = max_pic_order_cnt else {
        return reference_pic_order_cnt;
    };
    let mut difference =
        (reference_pic_order_cnt - current_pic_order_cnt).rem_euclid(max_pic_order_cnt);
    if difference >= max_pic_order_cnt / 2 {
        difference -= max_pic_order_cnt;
    }
    current_pic_order_cnt + difference
}

fn reference_picture<'a>(
    list: &'a [&ReferencePicture],
    reference_index: u32,
) -> Result<&'a ReferencePicture, H264DecodeError> {
    let index = usize::try_from(reference_index).map_err(|_| H264DecodeError::InvalidStream)?;
    list.get(index)
        .copied()
        .ok_or(H264DecodeError::InvalidReferenceIndex {
            index: reference_index,
            list_length: list.len(),
        })
}

pub fn decode_annex_b(data: &[u8]) -> Result<Yuv420Frame, H264DecodeError> {
    decode_annex_b_frames(data)?
        .into_iter()
        .next()
        .ok_or(H264DecodeError::MissingIdrSlice)
}

pub fn decode_annex_b_frames(data: &[u8]) -> Result<Vec<Yuv420Frame>, H264DecodeError> {
    let units = parse_annex_b(data)?;
    let mut sequence_parameter_sets = HashMap::<u32, Sps>::new();
    let mut picture_parameter_sets = HashMap::<u32, Pps>::new();
    let mut reference_pictures = ReferencePictureBuffer::default();
    let mut decoded_frames = Vec::<Yuv420Frame>::new();

    for unit in &units {
        match unit.header.unit_type {
            NalUnitType::Sps => {
                let sps = parse_sps(unit.data)?;
                sequence_parameter_sets.insert(sps.id, sps);
            }
            NalUnitType::Pps => {
                let pps = parse_pps(unit.data)?;
                picture_parameter_sets.insert(pps.id, pps);
            }
            NalUnitType::Idr | NalUnitType::CodedSliceNonIdr => {
                let prefix = crate::video::h264::slice::parse_slice_prefix(unit.data)?;
                let pps = picture_parameter_sets
                    .get(&prefix.pps_id)
                    .ok_or(H264DecodeError::MissingParameterSet)?;
                let sps = sequence_parameter_sets
                    .get(&pps.sps_id)
                    .ok_or(H264DecodeError::MissingParameterSet)?;
                let header = parse_slice_header(unit.data, sps, pps)?;
                validate_decoder_configuration(sps, pps, &header)?;

                let rbsp = ebsp_to_rbsp(&unit.data[1..]);
                let mut reader = BitReader::new(&rbsp);
                reader.skip_bits(header.slice_data_bit_offset)?;

                if prefix.is_idr {
                    reference_pictures.clear();
                }
                let max_frame_num = 1u32
                    .checked_shl(sps.log2_max_frame_num)
                    .ok_or(H264DecodeError::InvalidStream)?;
                let current_pic_order_cnt =
                    i64::from(header.pic_order_cnt_lsb.unwrap_or(header.frame_num));
                let max_pic_order_cnt = if sps.pic_order_cnt_type == 0 {
                    Some(
                        1i64.checked_shl(sps.log2_max_pic_order_cnt_lsb)
                            .ok_or(H264DecodeError::InvalidStream)?,
                    )
                } else {
                    None
                };
                let (list0_count, list1_count) =
                    if matches!(header.prefix.slice_type, SliceType::P | SliceType::B) {
                        (header.num_ref_idx_l0_active, header.num_ref_idx_l1_active)
                    } else {
                        (0, 0)
                    };
                let reference_lists = ReferencePictureLists::build(
                    reference_pictures.pictures(),
                    ReferencePictureListConfig {
                        slice_type: header.prefix.slice_type,
                        current_frame_num: header.frame_num,
                        max_frame_num,
                        current_pic_order_cnt,
                        max_pic_order_cnt,
                        list0_count,
                        list1_count,
                    },
                )?;
                let frame = match header.prefix.slice_type {
                    SliceType::I if prefix.is_idr => {
                        reconstruct_slice(&mut reader, sps, pps, &header)?
                    }
                    SliceType::P | SliceType::B if !prefix.is_idr => {
                        reconstruct_inter_slice(&mut reader, sps, &header, &reference_lists)?
                    }
                    _ => return Err(H264DecodeError::UnsupportedFeature),
                };

                if unit.header.nal_ref_idc != 0 {
                    let long_term_frame_idx = header
                        .dec_ref_pic_marking
                        .as_ref()
                        .and_then(|marking| marking.long_term_reference_flag)
                        .filter(|&is_long_term| prefix.is_idr && is_long_term)
                        .map(|_| 0);
                    reference_pictures.insert(
                        ReferencePicture {
                            frame_num: header.frame_num,
                            pic_order_cnt: current_pic_order_cnt,
                            long_term_frame_idx,
                            frame: copy_frame(&frame)?,
                        },
                        sps.max_num_ref_frames,
                        header.frame_num,
                        max_frame_num,
                    )?;
                }
                decoded_frames.push(frame);
            }
            NalUnitType::CodedSliceDataPartitionA
            | NalUnitType::CodedSliceDataPartitionB
            | NalUnitType::CodedSliceDataPartitionC => {
                return Err(H264DecodeError::UnsupportedFeature);
            }
            NalUnitType::Unspecified | NalUnitType::Other(_) => {
                return Err(H264DecodeError::UnsupportedFeature);
            }
            NalUnitType::Sei
            | NalUnitType::Aud
            | NalUnitType::EndOfSequence
            | NalUnitType::EndOfStream
            | NalUnitType::Filler => {}
        }
    }

    if decoded_frames.is_empty() {
        return Err(H264DecodeError::MissingIdrSlice);
    }
    Ok(decoded_frames)
}

fn validate_decoder_configuration(
    sps: &Sps,
    pps: &Pps,
    header: &crate::video::h264::slice::SliceHeader,
) -> Result<(), H264DecodeError> {
    let coded_width = sps
        .pic_width_in_mbs
        .checked_mul(16)
        .ok_or(H264DecodeError::InvalidStream)?;
    let coded_height = sps
        .pic_height_in_map_units
        .checked_mul(16)
        .ok_or(H264DecodeError::InvalidStream)?;

    let slice_type_supported = match header.prefix.slice_type {
        SliceType::I => header.prefix.is_idr,
        SliceType::P | SliceType::B => !header.prefix.is_idr,
        SliceType::Sp | SliceType::Si => false,
    };
    let inter_header_supported = match header.prefix.slice_type {
        SliceType::P => {
            header.ref_pic_list_modification_l0.is_empty() && header.pred_weight_table.is_none()
        }
        SliceType::B => {
            header.num_ref_idx_l0_active == 1
                && header.num_ref_idx_l1_active == 1
                && header.ref_pic_list_modification_l0.is_empty()
                && header.ref_pic_list_modification_l1.is_empty()
                && header.pred_weight_table.is_none()
        }
        _ => true,
    };

    if !matches!(sps.profile_idc, 66 | 77)
        || sps.chroma_format_idc != 1
        || sps.separate_colour_plane
        || sps.bit_depth_luma != 8
        || sps.bit_depth_chroma != 8
        || !sps.frame_mbs_only
        || sps.width != coded_width
        || sps.height != coded_height
        || pps.entropy_coding_mode
        || pps.num_slice_groups != 1
        || sps.max_num_ref_frames > 2
        || !slice_type_supported
        || header.prefix.first_mb_in_slice != 0
        || header.field_pic_flag
        || header.redundant_pic_cnt.unwrap_or(0) != 0
        || !inter_header_supported
        || header.dec_ref_pic_marking.as_ref().is_some_and(|marking| {
            !header.prefix.is_idr
                && (marking.adaptive_ref_pic_marking_mode_flag || !marking.operations.is_empty())
        })
        || (header.prefix.slice_type == SliceType::B && pps.weighted_bipred_idc != 0)
        || header
            .deblocking_filter
            .is_none_or(|parameters| parameters.disable_deblocking_filter_idc != 1)
    {
        return Err(H264DecodeError::UnsupportedFeature);
    }

    Ok(())
}

fn reconstruct_slice(
    reader: &mut BitReader<'_>,
    sps: &Sps,
    pps: &Pps,
    header: &crate::video::h264::slice::SliceHeader,
) -> Result<Yuv420Frame, H264DecodeError> {
    let mut frame = Yuv420Frame::new(sps.width, sps.height)?;
    let macroblock_count = sps
        .pic_width_in_mbs
        .checked_mul(sps.pic_height_in_map_units)
        .ok_or(H264DecodeError::InvalidStream)?;
    let grid_width = sps
        .pic_width_in_mbs
        .checked_mul(4)
        .ok_or(H264DecodeError::InvalidStream)?;
    let grid_height = sps
        .pic_height_in_map_units
        .checked_mul(4)
        .ok_or(H264DecodeError::InvalidStream)?;
    let grid_length = usize::try_from(grid_width)
        .ok()
        .and_then(|width| {
            usize::try_from(grid_height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or(H264DecodeError::InvalidStream)?;
    let mut luma_nonzero_counts = Vec::new();
    luma_nonzero_counts
        .try_reserve_exact(grid_length)
        .map_err(|_| H264DecodeError::Frame(FrameError::AllocationFailed))?;
    luma_nonzero_counts.resize(grid_length, 0u8);
    let initial_qp = 26i32 + pps.pic_init_qp_minus26 + header.slice_qp_delta;
    if !(0..=51).contains(&initial_qp) {
        return Err(H264DecodeError::InvalidStream);
    }
    let mut qp_y = initial_qp;

    for macroblock_address in 0..macroblock_count {
        let mb_type = reader.read_ue()?;
        if !(1..=24).contains(&mb_type) {
            return Err(H264DecodeError::UnsupportedFeature);
        }

        let encoded_type = mb_type - 1;
        let prediction_mode = match encoded_type % 4 {
            0 => Intra16x16Mode::Vertical,
            1 => Intra16x16Mode::Horizontal,
            2 => Intra16x16Mode::Dc,
            3 => Intra16x16Mode::Plane,
            _ => unreachable!(),
        };
        let chroma_pattern = (encoded_type / 4) % 3;
        let luma_pattern = if encoded_type / 12 == 0 { 0 } else { 15 };
        let chroma_prediction_mode = reader.read_ue()?;
        if chroma_prediction_mode != 0 || chroma_pattern != 0 {
            return Err(H264DecodeError::UnsupportedFeature);
        }

        let qp_delta = reader.read_se()?;
        if !(-26..=25).contains(&qp_delta) {
            return Err(H264DecodeError::InvalidStream);
        }
        qp_y = (qp_y + qp_delta).rem_euclid(52);
        let qp_y_u8 = u8::try_from(qp_y).map_err(|_| H264DecodeError::InvalidStream)?;

        let mb_x = macroblock_address % sps.pic_width_in_mbs;
        let mb_y = macroblock_address / sps.pic_width_in_mbs;
        let dc_block = decode_residual_4x4(reader, Some(0), Residual4x4Kind::Full)?;
        let dc_coefficients = inverse_scale_4x4(&dc_block.coeff_levels, qp_y_u8, None)?;
        let dc_transform = inverse_hadamard_4x4(&dc_coefficients)?;
        let mut ac_blocks = [[0i32; 16]; 16];

        if luma_pattern != 0 {
            for (block_index, ac_block) in ac_blocks.iter_mut().enumerate() {
                let block_x = block_index % 4;
                let block_y = block_index / 4;
                let grid_x = mb_x as usize * 4 + block_x;
                let grid_y = mb_y as usize * 4 + block_y;
                let nc = luma_neighbor_context(
                    &luma_nonzero_counts,
                    grid_width as usize,
                    grid_x,
                    grid_y,
                );
                let block = decode_residual_4x4(reader, nc, Residual4x4Kind::AcOnly)?;
                luma_nonzero_counts[grid_y * grid_width as usize + grid_x] = block
                    .coeff_levels
                    .iter()
                    .filter(|&&level| level != 0)
                    .count()
                    as u8;
                *ac_block = block.coeff_levels;
            }
        }

        let x = mb_x * 16;
        let y = mb_y * 16;
        let top = if mb_y == 0 {
            None
        } else {
            Some(std::array::from_fn(|index| {
                frame.y[((y - 1) * frame.width + x + index as u32) as usize]
            }))
        };
        let left = if mb_x == 0 {
            None
        } else {
            Some(std::array::from_fn(|index| {
                frame.y[((y + index as u32) * frame.width + x - 1) as usize]
            }))
        };
        let top_left = if mb_x == 0 || mb_y == 0 {
            None
        } else {
            Some(frame.y[((y - 1) * frame.width + x - 1) as usize])
        };
        let prediction = predict_intra16x16(
            prediction_mode,
            Intra16x16References {
                top,
                left,
                top_left,
            },
        )?;

        for block_index in 0..16usize {
            let block_x = block_index % 4;
            let block_y = block_index / 4;
            let mut scaled = inverse_scale_4x4(&ac_blocks[block_index], qp_y_u8, None)?;
            scaled[0] = dc_transform[block_index];
            let residual = inverse_transform_4x4(&scaled)?;
            let prediction_block = std::array::from_fn(|index| {
                prediction[block_y * 4 + index / 4][block_x * 4 + index % 4]
            });
            frame.reconstruct_block4x4(
                ColorPlane::Y,
                x + (block_x * 4) as u32,
                y + (block_y * 4) as u32,
                &prediction_block,
                &residual,
            )?;
        }

        reconstruct_chroma_dc(&mut frame, ColorPlane::U, mb_x, mb_y)?;
        reconstruct_chroma_dc(&mut frame, ColorPlane::V, mb_x, mb_y)?;
    }

    if !reader.read_bit()? {
        return Err(H264DecodeError::InvalidStream);
    }
    while reader.bits_remaining() > 0 {
        if reader.read_bit()? {
            return Err(H264DecodeError::InvalidStream);
        }
    }

    Ok(frame)
}

fn reconstruct_inter_slice(
    reader: &mut BitReader<'_>,
    sps: &Sps,
    header: &crate::video::h264::slice::SliceHeader,
    reference_lists: &ReferencePictureLists<'_>,
) -> Result<Yuv420Frame, H264DecodeError> {
    let macroblock_count = sps
        .pic_width_in_mbs
        .checked_mul(sps.pic_height_in_map_units)
        .ok_or(H264DecodeError::InvalidStream)?;
    match header.prefix.slice_type {
        SliceType::P => {
            let mut frame = Yuv420Frame::new(sps.width, sps.height)?;
            let mut motion_grid = MotionGrid::new(
                sps.pic_width_in_mbs as usize * 4,
                sps.pic_height_in_map_units as usize * 4,
            )?;
            let mut address = 0u32;

            while address < macroblock_count {
                let skip_run = reader.read_ue()?;
                let remaining = macroblock_count - address;
                if skip_run > remaining {
                    return Err(H264DecodeError::InvalidStream);
                }

                for _ in 0..skip_run {
                    let mb_x = address % sps.pic_width_in_mbs;
                    let mb_y = address / sps.pic_width_in_mbs;
                    let motion = motion_grid.predict(mb_x, mb_y, 0);
                    let skip_motion = if motion_grid.p_skip_requires_zero(mb_x, mb_y) {
                        MotionVector::default()
                    } else {
                        motion
                    };
                    let reference = reference_picture(&reference_lists.list0, 0)?;
                    motion_compensate_macroblock(
                        &mut frame,
                        &reference.frame,
                        mb_x,
                        mb_y,
                        skip_motion,
                    )?;
                    motion_grid.set_macroblock(mb_x, mb_y, 0, skip_motion);
                    address += 1;
                }

                if address == macroblock_count {
                    break;
                }

                let mb_x = address % sps.pic_width_in_mbs;
                let mb_y = address / sps.pic_width_in_mbs;
                match parse_p_macroblock(reader, header.num_ref_idx_l0_active)? {
                    MacroblockType::P16x16 {
                        reference_index,
                        motion_vector_difference,
                        coded_block_pattern: 0,
                    } => {
                        let motion_prediction = motion_grid.predict(mb_x, mb_y, reference_index);
                        let motion = MotionVector {
                            x: motion_prediction
                                .x
                                .checked_add(motion_vector_difference[0])
                                .ok_or(H264DecodeError::InvalidStream)?,
                            y: motion_prediction
                                .y
                                .checked_add(motion_vector_difference[1])
                                .ok_or(H264DecodeError::InvalidStream)?,
                        };
                        let reference = reference_picture(&reference_lists.list0, reference_index)?;
                        motion_compensate_macroblock(
                            &mut frame,
                            &reference.frame,
                            mb_x,
                            mb_y,
                            motion,
                        )?;
                        motion_grid.set_macroblock(mb_x, mb_y, reference_index, motion);
                    }
                    _ => return Err(H264DecodeError::UnsupportedFeature),
                }
                address += 1;
            }

            consume_rbsp_trailing_bits(reader)?;
            Ok(frame)
        }
        SliceType::B => {
            let skip_run = reader.read_ue()?;
            if skip_run != macroblock_count {
                return Err(H264DecodeError::UnsupportedFeature);
            }
            consume_rbsp_trailing_bits(reader)?;
            let first = reference_picture(&reference_lists.list0, 0)?;
            let second = reference_picture(&reference_lists.list1, 0)?;
            average_frames(&first.frame, &second.frame)
        }
        _ => Err(H264DecodeError::UnsupportedFeature),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MotionVector {
    x: i32,
    y: i32,
}

struct MotionGrid {
    width: usize,
    height: usize,
    cells: Vec<Option<(u32, MotionVector)>>,
}

impl MotionGrid {
    fn new(width: usize, height: usize) -> Result<Self, H264DecodeError> {
        let length = width
            .checked_mul(height)
            .ok_or(H264DecodeError::InvalidStream)?;
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(length)
            .map_err(|_| H264DecodeError::Frame(FrameError::AllocationFailed))?;
        cells.resize(length, None);
        Ok(Self {
            width,
            height,
            cells,
        })
    }

    fn cell(&self, x: usize, y: usize) -> Option<(u32, MotionVector)> {
        (x < self.width && y < self.height)
            .then(|| self.cells[y * self.width + x])
            .flatten()
    }

    fn neighbors(&self, mb_x: u32, mb_y: u32) -> [Option<(u32, MotionVector)>; 3] {
        let x = mb_x as usize * 4;
        let y = mb_y as usize * 4;
        let left = x.checked_sub(1).and_then(|left_x| self.cell(left_x, y + 3));
        let top = y.checked_sub(1).and_then(|top_y| self.cell(x, top_y));
        let top_right = y
            .checked_sub(1)
            .filter(|_| x + 4 < self.width)
            .and_then(|top_y| self.cell(x + 4, top_y));
        let top_left = y
            .checked_sub(1)
            .and_then(|top_y| x.checked_sub(1).and_then(|left_x| self.cell(left_x, top_y)));
        [left, top, top_right.or(top_left)]
    }

    fn predict(&self, mb_x: u32, mb_y: u32, reference_index: u32) -> MotionVector {
        let matching = self
            .neighbors(mb_x, mb_y)
            .into_iter()
            .flatten()
            .filter(|(index, _)| *index == reference_index)
            .map(|(_, motion)| motion)
            .collect::<Vec<_>>();
        if matching.len() == 1 {
            return matching[0];
        }

        let mut vectors = self.neighbors(mb_x, mb_y).map(|candidate| {
            candidate
                .filter(|(index, _)| *index == reference_index)
                .map_or(MotionVector::default(), |(_, motion)| motion)
        });
        vectors.sort_by_key(|motion| motion.x);
        let x = vectors[1].x;
        vectors.sort_by_key(|motion| motion.y);
        MotionVector { x, y: vectors[1].y }
    }

    fn p_skip_requires_zero(&self, mb_x: u32, mb_y: u32) -> bool {
        let [left, top, _] = self.neighbors(mb_x, mb_y);
        [left, top]
            .into_iter()
            .any(|candidate| candidate.is_none_or(|(_, motion)| motion == MotionVector::default()))
    }

    fn set_macroblock(&mut self, mb_x: u32, mb_y: u32, reference_index: u32, motion: MotionVector) {
        let x = mb_x as usize * 4;
        let y = mb_y as usize * 4;
        for row in 0..4 {
            for column in 0..4 {
                self.cells[(y + row) * self.width + x + column] = Some((reference_index, motion));
            }
        }
    }
}

fn motion_compensate_macroblock(
    output: &mut Yuv420Frame,
    reference: &Yuv420Frame,
    mb_x: u32,
    mb_y: u32,
    motion: MotionVector,
) -> Result<(), H264DecodeError> {
    if output.width != reference.width || output.height != reference.height {
        return Err(H264DecodeError::InvalidStream);
    }
    copy_luma_macroblock(output, reference, mb_x, mb_y, motion)?;
    copy_chroma_macroblock(output, reference, ColorPlane::U, mb_x, mb_y, motion)?;
    copy_chroma_macroblock(output, reference, ColorPlane::V, mb_x, mb_y, motion)?;
    Ok(())
}

fn copy_luma_macroblock(
    output: &mut Yuv420Frame,
    reference: &Yuv420Frame,
    mb_x: u32,
    mb_y: u32,
    motion: MotionVector,
) -> Result<(), H264DecodeError> {
    let width = output.width as usize;
    let height = output.height as usize;
    let x_start = usize::try_from(mb_x)
        .ok()
        .and_then(|x| x.checked_mul(16))
        .ok_or(H264DecodeError::InvalidStream)?;
    let y_start = usize::try_from(mb_y)
        .ok()
        .and_then(|y| y.checked_mul(16))
        .ok_or(H264DecodeError::InvalidStream)?;
    if x_start.checked_add(16).is_none_or(|right| right > width)
        || y_start.checked_add(16).is_none_or(|bottom| bottom > height)
    {
        return Err(H264DecodeError::InvalidStream);
    }
    for row in 0..16usize {
        for column in 0..16usize {
            let x = x_start + column;
            let y = y_start + row;
            let x_quarter = (x as i64) * 4 + i64::from(motion.x);
            let y_quarter = (y as i64) * 4 + i64::from(motion.y);
            output.y[y * width + x] = interpolate_luma_sample(reference, x_quarter, y_quarter);
        }
    }
    Ok(())
}

fn copy_chroma_macroblock(
    output: &mut Yuv420Frame,
    reference: &Yuv420Frame,
    plane: ColorPlane,
    mb_x: u32,
    mb_y: u32,
    motion: MotionVector,
) -> Result<(), H264DecodeError> {
    let (width, height) = output.plane_dimensions(plane);
    if reference.plane_dimensions(plane) != (width, height) {
        return Err(H264DecodeError::InvalidStream);
    }
    if plane == ColorPlane::Y {
        return Err(H264DecodeError::InvalidStream);
    }
    let x_start = usize::try_from(mb_x)
        .ok()
        .and_then(|x| x.checked_mul(8))
        .ok_or(H264DecodeError::InvalidStream)?;
    let y_start = usize::try_from(mb_y)
        .ok()
        .and_then(|y| y.checked_mul(8))
        .ok_or(H264DecodeError::InvalidStream)?;
    let width = width as usize;
    let height = height as usize;
    if x_start.checked_add(8).is_none_or(|right| right > width)
        || y_start.checked_add(8).is_none_or(|bottom| bottom > height)
    {
        return Err(H264DecodeError::InvalidStream);
    }
    let target = match plane {
        ColorPlane::U => &mut output.u,
        ColorPlane::V => &mut output.v,
        ColorPlane::Y => unreachable!(),
    };
    for row in 0..8usize {
        for column in 0..8usize {
            let x = x_start + column;
            let y = y_start + row;
            target[y * width + x] =
                interpolate_chroma_sample(reference, plane, x as u32, y as u32, motion.x, motion.y);
        }
    }
    Ok(())
}

fn copy_frame(source: &Yuv420Frame) -> Result<Yuv420Frame, H264DecodeError> {
    let mut frame = Yuv420Frame::new(source.width, source.height)?;
    frame.y.copy_from_slice(&source.y);
    frame.u.copy_from_slice(&source.u);
    frame.v.copy_from_slice(&source.v);
    Ok(frame)
}

fn average_frames(
    first: &Yuv420Frame,
    second: &Yuv420Frame,
) -> Result<Yuv420Frame, H264DecodeError> {
    if first.width != second.width || first.height != second.height {
        return Err(H264DecodeError::InvalidStream);
    }

    let mut frame = Yuv420Frame::new(first.width, first.height)?;
    average_plane(&mut frame.y, &first.y, &second.y)?;
    average_plane(&mut frame.u, &first.u, &second.u)?;
    average_plane(&mut frame.v, &first.v, &second.v)?;
    Ok(frame)
}

fn average_plane(output: &mut [u8], first: &[u8], second: &[u8]) -> Result<(), H264DecodeError> {
    if output.len() != first.len() || first.len() != second.len() {
        return Err(H264DecodeError::InvalidStream);
    }
    for ((target, &a), &b) in output.iter_mut().zip(first).zip(second) {
        *target = ((u16::from(a) + u16::from(b) + 1) >> 1) as u8;
    }
    Ok(())
}

fn consume_rbsp_trailing_bits(reader: &mut BitReader<'_>) -> Result<(), H264DecodeError> {
    if !reader.read_bit()? {
        return Err(H264DecodeError::InvalidStream);
    }
    while reader.bits_remaining() > 0 {
        if reader.read_bit()? {
            return Err(H264DecodeError::InvalidStream);
        }
    }
    Ok(())
}

fn luma_neighbor_context(counts: &[u8], grid_width: usize, x: usize, y: usize) -> Option<u32> {
    let left = (x > 0).then(|| u32::from(counts[y * grid_width + x - 1]));
    let top = (y > 0).then(|| u32::from(counts[(y - 1) * grid_width + x]));
    match (left, top) {
        (Some(left), Some(top)) => Some((left + top + 1) >> 1),
        (Some(count), None) | (None, Some(count)) => Some(count),
        (None, None) => None,
    }
}

fn reconstruct_chroma_dc(
    frame: &mut Yuv420Frame,
    plane: ColorPlane,
    mb_x: u32,
    mb_y: u32,
) -> Result<(), H264DecodeError> {
    let (width, _) = frame.plane_dimensions(plane);
    let x = mb_x * 8;
    let y = mb_y * 8;
    let top: Option<[u8; 8]> = (mb_y > 0).then(|| {
        std::array::from_fn(|index| {
            let offset = ((y - 1) * width + x + index as u32) as usize;
            match plane {
                ColorPlane::U => frame.u[offset],
                ColorPlane::V => frame.v[offset],
                ColorPlane::Y => unreachable!(),
            }
        })
    });
    let left: Option<[u8; 8]> = (mb_x > 0).then(|| {
        std::array::from_fn(|index| {
            let offset = ((y + index as u32) * width + x - 1) as usize;
            match plane {
                ColorPlane::U => frame.u[offset],
                ColorPlane::V => frame.v[offset],
                ColorPlane::Y => unreachable!(),
            }
        })
    });
    let average = |values: &[u8]| {
        ((values.iter().map(|&value| u16::from(value)).sum::<u16>() + 2) >> 2) as u8
    };
    let dc = match (top, left) {
        (Some(top), Some(left)) => [
            ((top[..4]
                .iter()
                .chain(&left[..4])
                .map(|&v| u16::from(v))
                .sum::<u16>()
                + 4)
                >> 3) as u8,
            average(&top[4..8]),
            average(&left[4..8]),
            ((top[4..8]
                .iter()
                .chain(&left[4..8])
                .map(|&v| u16::from(v))
                .sum::<u16>()
                + 4)
                >> 3) as u8,
        ],
        (Some(top), None) => [
            average(&top[..4]),
            average(&top[4..8]),
            average(&top[..4]),
            average(&top[4..8]),
        ],
        (None, Some(left)) => [
            average(&left[..4]),
            average(&left[..4]),
            average(&left[4..8]),
            average(&left[4..8]),
        ],
        (None, None) => [128; 4],
    };
    let prediction: [[u8; 8]; 8] =
        std::array::from_fn(|row| std::array::from_fn(|column| dc[(row / 4) * 2 + column / 4]));

    for block_y in 0..2 {
        for block_x in 0..2 {
            let prediction_block = std::array::from_fn(|index| {
                prediction[block_y * 4 + index / 4][block_x * 4 + index % 4]
            });
            frame.reconstruct_block4x4(
                plane,
                x + (block_x * 4) as u32,
                y + (block_y * 4) as u32,
                &prediction_block,
                &[0; 16],
            )?;
        }
    }

    Ok(())
}

fn inverse_hadamard_4x4(input: &[i32; 16]) -> Result<[i32; 16], H264DecodeError> {
    let mut temporary = [[0i64; 4]; 4];
    for row in 0..4 {
        let values = [
            i64::from(input[row * 4]),
            i64::from(input[row * 4 + 1]),
            i64::from(input[row * 4 + 2]),
            i64::from(input[row * 4 + 3]),
        ];
        let a0 = values[0] + values[1];
        let a1 = values[0] - values[1];
        let a2 = values[2] + values[3];
        let a3 = values[2] - values[3];
        temporary[row] = [a0 + a2, a1 + a3, a0 - a2, a1 - a3];
    }

    let mut output = [0i32; 16];
    for column in 0..4 {
        let a0 = temporary[0][column] + temporary[1][column];
        let a1 = temporary[0][column] - temporary[1][column];
        let a2 = temporary[2][column] + temporary[3][column];
        let a3 = temporary[2][column] - temporary[3][column];
        for (row, value) in [a0 + a2, a1 + a3, a0 - a2, a1 - a3].into_iter().enumerate() {
            output[row * 4 + column] =
                i32::try_from((value + 2) >> 2).map_err(|_| H264DecodeError::InvalidStream)?;
        }
    }

    Ok(output)
}

fn ebsp_to_rbsp(ebsp: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(ebsp.len());
    let mut zero_count = 0usize;
    for &byte in ebsp {
        if zero_count >= 2 && byte == 3 {
            zero_count = 0;
            continue;
        }
        rbsp.push(byte);
        zero_count = if byte == 0 { zero_count + 1 } else { 0 };
    }
    rbsp
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDR_STREAM: &[u8] = include_bytes!("../../../tests/fixtures/h264/gray_idr_16x16.h264");

    const REFERENCE_YUV: &[u8] =
        include_bytes!("../../../tests/fixtures/h264/gray_idr_16x16.reference.yuv");
    const P_B_STREAM: &[u8] =
        include_bytes!("../../../tests/fixtures/h264/gray_p_b_skip_16x16.h264");
    const P_B_REFERENCE_YUV: &[u8] =
        include_bytes!("../../../tests/fixtures/h264/gray_p_b_skip_16x16.reference.yuv");
    const P16_STREAM: &[u8] =
        include_bytes!("../../../tests/fixtures/h264/gray_p16x16_base_32x16.h264");
    const P16_REFERENCE_YUV: &[u8] =
        include_bytes!("../../../tests/fixtures/h264/gray_p16x16_base_32x16.reference.yuv");

    fn make_reference_picture(frame_num: u32, pic_order_cnt: i64) -> ReferencePicture {
        ReferencePicture {
            frame_num,
            pic_order_cnt,
            long_term_frame_idx: None,
            frame: Yuv420Frame::new(16, 16).unwrap(),
        }
    }

    #[test]
    fn builds_p_and_b_reference_lists_in_picture_order() {
        let pictures = [
            make_reference_picture(15, 2),
            make_reference_picture(14, 6),
            make_reference_picture(13, 9),
        ];

        let p_lists = ReferencePictureLists::build(
            &pictures,
            ReferencePictureListConfig {
                slice_type: SliceType::P,
                current_frame_num: 0,
                max_frame_num: 16,
                current_pic_order_cnt: 0,
                max_pic_order_cnt: Some(16),
                list0_count: 3,
                list1_count: 0,
            },
        )
        .unwrap();
        assert_eq!(
            p_lists
                .list0
                .iter()
                .map(|picture| picture.frame_num)
                .collect::<Vec<_>>(),
            [15, 14, 13]
        );

        let b_lists = ReferencePictureLists::build(
            &pictures,
            ReferencePictureListConfig {
                slice_type: SliceType::B,
                current_frame_num: 0,
                max_frame_num: 16,
                current_pic_order_cnt: 4,
                max_pic_order_cnt: Some(16),
                list0_count: 3,
                list1_count: 3,
            },
        )
        .unwrap();
        assert_eq!(
            b_lists
                .list0
                .iter()
                .map(|picture| picture.pic_order_cnt)
                .collect::<Vec<_>>(),
            [2, 6, 9]
        );
        assert_eq!(
            b_lists
                .list1
                .iter()
                .map(|picture| picture.pic_order_cnt)
                .collect::<Vec<_>>(),
            [6, 9, 2]
        );
    }

    #[test]
    fn validates_reference_indices_and_evicts_oldest_short_term_picture() {
        let mut buffer = ReferencePictureBuffer::default();
        buffer
            .insert(make_reference_picture(14, 0), 2, 14, 16)
            .unwrap();
        buffer
            .insert(make_reference_picture(15, 1), 2, 15, 16)
            .unwrap();
        buffer
            .insert(make_reference_picture(0, 2), 2, 0, 16)
            .unwrap();
        assert_eq!(
            buffer
                .pictures()
                .iter()
                .map(|picture| picture.frame_num)
                .collect::<Vec<_>>(),
            [15, 0]
        );

        let lists = ReferencePictureLists::build(
            buffer.pictures(),
            ReferencePictureListConfig {
                slice_type: SliceType::P,
                current_frame_num: 1,
                max_frame_num: 16,
                current_pic_order_cnt: 3,
                max_pic_order_cnt: Some(16),
                list0_count: 2,
                list1_count: 0,
            },
        )
        .unwrap();
        assert_eq!(
            reference_picture(&lists.list0, 2),
            Err(H264DecodeError::InvalidReferenceIndex {
                index: 2,
                list_length: 2,
            })
        );
    }

    #[test]
    fn decodes_real_baseline_idr_against_reference_frame() {
        let units = parse_annex_b(IDR_STREAM).unwrap();
        let sps = units
            .iter()
            .find(|unit| unit.header.unit_type == NalUnitType::Sps)
            .map(|unit| parse_sps(unit.data).unwrap())
            .unwrap();
        let pps = units
            .iter()
            .find(|unit| unit.header.unit_type == NalUnitType::Pps)
            .map(|unit| parse_pps(unit.data).unwrap())
            .unwrap();
        assert_eq!((sps.profile_idc, sps.width, sps.height), (66, 16, 16));
        assert_eq!(pps.sps_id, sps.id);
        assert!(!pps.entropy_coding_mode);

        let frame = decode_annex_b(IDR_STREAM).unwrap();
        assert_eq!((frame.width, frame.height), (16, 16));
        assert_eq!(&frame.y, &REFERENCE_YUV[..256]);
        assert_eq!(&frame.u, &REFERENCE_YUV[256..320]);
        assert_eq!(&frame.v, &REFERENCE_YUV[320..384]);
    }

    #[test]
    fn rejects_truncated_and_malformed_streams() {
        assert!(decode_annex_b(&IDR_STREAM[..IDR_STREAM.len() - 2]).is_err());
        assert!(decode_annex_b(&[0, 0, 1, 0x65]).is_err());
        assert!(decode_annex_b_frames(&P_B_STREAM[..P_B_STREAM.len() - 1]).is_err());
    }

    #[test]
    fn decodes_p_skip_and_b_direct_skip_pictures_against_reference() {
        let slice_types = parse_annex_b(P_B_STREAM)
            .unwrap()
            .into_iter()
            .filter(|unit| {
                matches!(
                    unit.header.unit_type,
                    NalUnitType::Idr | NalUnitType::CodedSliceNonIdr
                )
            })
            .map(|unit| {
                crate::video::h264::slice::parse_slice_prefix(unit.data)
                    .unwrap()
                    .slice_type
            })
            .collect::<Vec<_>>();
        assert_eq!(
            slice_types,
            [SliceType::I, SliceType::P, SliceType::B, SliceType::P]
        );

        let frames = decode_annex_b_frames(P_B_STREAM).unwrap();
        assert_eq!(frames.len(), 4);
        for (index, frame) in frames.iter().enumerate() {
            let start = index * 384;
            assert_eq!(frame.width, 16);
            assert_eq!(frame.height, 16);
            assert_eq!(&frame.y, &P_B_REFERENCE_YUV[start..start + 256]);
            assert_eq!(&frame.u, &P_B_REFERENCE_YUV[start + 256..start + 320]);
            assert_eq!(&frame.v, &P_B_REFERENCE_YUV[start + 320..start + 384]);
        }
    }

    #[test]
    fn decodes_p16x16_motion_against_reference() {
        let units = parse_annex_b(P16_STREAM).unwrap();
        let sps = units
            .iter()
            .find(|unit| unit.header.unit_type == NalUnitType::Sps)
            .map(|unit| parse_sps(unit.data).unwrap())
            .unwrap();
        let pps = units
            .iter()
            .find(|unit| unit.header.unit_type == NalUnitType::Pps)
            .map(|unit| parse_pps(unit.data).unwrap())
            .unwrap();
        let mut stream = Vec::new();
        let mut decoded_pictures = 0;
        for unit in units {
            stream.extend_from_slice(&[0, 0, 0, 1]);
            if unit.header.unit_type != NalUnitType::CodedSliceNonIdr {
                stream.extend_from_slice(unit.data);
                continue;
            }

            let header = parse_slice_header(unit.data, &sps, &pps).unwrap();
            let rbsp = ebsp_to_rbsp(&unit.data[1..]);
            let mut reader = BitReader::new(&rbsp);
            let mut bits = Vec::new();
            for _ in 0..header.slice_data_bit_offset {
                bits.push(reader.read_bit().unwrap());
            }

            for mb in 0..2 {
                push_ue(&mut bits, 0);
                push_ue(&mut bits, 0);
                push_se(&mut bits, if mb == 0 { 0 } else { -64 });
                push_se(&mut bits, 0);
                push_ue(&mut bits, 0);
            }
            bits.push(true);
            while bits.len() % 8 != 0 {
                bits.push(false);
            }
            let rbsp = bits_to_bytes(&bits);
            stream.push(unit.data[0]);
            stream.extend_from_slice(&rbsp_to_ebsp(&rbsp));
            decoded_pictures += 1;
        }
        assert_eq!(decoded_pictures, 2);

        for unit in parse_annex_b(&stream).unwrap() {
            if unit.header.unit_type == NalUnitType::CodedSliceNonIdr {
                let header = parse_slice_header(unit.data, &sps, &pps).unwrap();
                let rbsp = ebsp_to_rbsp(&unit.data[1..]);
                let mut rbsp_reader = BitReader::new(&rbsp);
                rbsp_reader.skip_bits(header.slice_data_bit_offset).unwrap();
                for mb in 0..2 {
                    assert_eq!(rbsp_reader.read_ue().unwrap(), 0);
                    assert_eq!(
                        crate::video::h264::macroblock::inter::parse_p_macroblock(
                            &mut rbsp_reader,
                            1
                        ),
                        Ok(MacroblockType::P16x16 {
                            reference_index: 0,
                            motion_vector_difference: if mb == 0 { [0, 0] } else { [-64, 0] },
                            coded_block_pattern: 0,
                        })
                    );
                }
            }
        }

        let frames = decode_annex_b_frames(&stream).unwrap();
        assert_eq!(frames.len(), 3);
        let expected_idr = &P16_REFERENCE_YUV[..768];
        for frame in &frames {
            assert_eq!((frame.width, frame.height), (32, 16));
            assert_eq!(&frame.y, &expected_idr[..512]);
            assert_eq!(&frame.u, &expected_idr[512..640]);
            assert_eq!(&frame.v, &expected_idr[640..768]);
        }
    }

    #[test]
    fn motion_compensation_applies_integer_luma_and_chroma_displacement() {
        let mut reference = Yuv420Frame::new(32, 16).unwrap();
        for row in 0..16 {
            for column in 0..32 {
                reference.y[row * 32 + column] = column as u8;
            }
        }
        for row in 0..8 {
            for column in 0..16 {
                reference.u[row * 16 + column] = (100 + column) as u8;
                reference.v[row * 16 + column] = (160 + column) as u8;
            }
        }

        let mut output = Yuv420Frame::new(32, 16).unwrap();
        motion_compensate_macroblock(&mut output, &reference, 1, 0, MotionVector { x: -64, y: 0 })
            .unwrap();

        for row in 0..16 {
            assert_eq!(
                &output.y[row * 32 + 16..row * 32 + 32],
                &reference.y[row * 32..row * 32 + 16]
            );
        }
        for row in 0..8 {
            assert_eq!(
                &output.u[row * 16 + 8..row * 16 + 16],
                &reference.u[row * 16..row * 16 + 8]
            );
            assert_eq!(
                &output.v[row * 16 + 8..row * 16 + 16],
                &reference.v[row * 16..row * 16 + 8]
            );
        }
    }

    #[test]
    fn motion_compensation_uses_fractional_luma_and_chroma_interpolation() {
        let mut reference = Yuv420Frame::new(32, 16).unwrap();
        for row in 0..16 {
            for column in 0..32 {
                reference.y[row * 32 + column] = (column * 4 + row * 2) as u8;
            }
        }
        for row in 0..8 {
            for column in 0..16 {
                reference.u[row * 16 + column] = (column * 80) as u8;
                reference.v[row * 16 + column] = (10 + column * 40) as u8;
            }
        }

        let mut output = Yuv420Frame::new(32, 16).unwrap();
        motion_compensate_macroblock(&mut output, &reference, 0, 0, MotionVector { x: 1, y: 1 })
            .unwrap();

        assert_eq!(output.y[3 * 32 + 3], 20);
        assert_eq!(output.u[0], 10);
        assert_eq!(output.v[0], 15);

        motion_compensate_macroblock(&mut output, &reference, 0, 0, MotionVector { x: 2, y: 0 })
            .unwrap();
        assert_eq!(output.u[0], 20);
        assert_eq!(output.v[0], 20);
    }

    fn push_ue(bits: &mut Vec<bool>, value: u32) {
        let code = u64::from(value) + 1;
        let width = 64 - code.leading_zeros();
        bits.extend(std::iter::repeat_n(false, (width - 1) as usize));
        for shift in (0..width).rev() {
            bits.push((code >> shift) & 1 != 0);
        }
    }

    fn push_se(bits: &mut Vec<bool>, value: i32) {
        let code = if value <= 0 {
            value.unsigned_abs() * 2
        } else {
            value as u32 * 2 - 1
        };
        push_ue(bits, code);
    }

    fn bits_to_bytes(bits: &[bool]) -> Vec<u8> {
        let mut bytes = vec![0; bits.len().div_ceil(8)];
        for (index, &bit) in bits.iter().enumerate() {
            if bit {
                bytes[index / 8] |= 1 << (7 - index % 8);
            }
        }
        bytes
    }

    fn rbsp_to_ebsp(rbsp: &[u8]) -> Vec<u8> {
        let mut ebsp = Vec::with_capacity(rbsp.len());
        let mut zero_count = 0;
        for &byte in rbsp {
            if zero_count >= 2 && byte <= 3 {
                ebsp.push(3);
                zero_count = 0;
            }
            ebsp.push(byte);
            zero_count = if byte == 0 { zero_count + 1 } else { 0 };
        }
        ebsp
    }
}
