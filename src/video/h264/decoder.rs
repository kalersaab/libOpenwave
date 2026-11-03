use std::collections::HashMap;

use crate::video::{
    bitstream::{BitReader, BitstreamError},
    h264::{
        cabac::{
            CabacContexts, CabacEngine, ResidualCategory, decode_cabac_cbp,
            decode_cabac_chroma_pred_mode, decode_cabac_mb_qp_delta, decode_cabac_mvd,
            decode_cabac_ref_idx, decode_cabac_residual_block, decode_i_slice_mb_type,
            decode_intra4x4_pred_mode, decode_mb_skip_flag, decode_p_slice_mb_type,
        },
        cavlc::{CavlcError, Residual4x4Kind, decode_residual_4x4},
        frame::{ColorPlane, FrameError, Yuv420Frame},
        intra::{
            Intra4x4Mode, Intra4x4Reconstruction, Intra4x4References, Intra16x16Mode,
            Intra16x16References, IntraPredictionError, IntraReconstructionError,
            predict_intra16x16, reconstruct_intra4x4_block,
        },
        macroblock::{MacroblockParseError, MacroblockType, parse_p_macroblock},
        motion::{chroma::interpolate_chroma_sample, luma::interpolate_luma_sample},
        nal::{AnnexBError, NalUnitType, parse_annex_b},
        pps::{Pps, PpsError, parse_pps},
        slice::{SliceError, SliceType, parse_slice_header},
        sps::{Sps, SpsError, parse_sps},
        transform::{
            TransformError, inverse_scale_4x4, inverse_scale_luma_dc, inverse_transform_4x4,
            reconstruct_residual_4x4,
        },
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

#[derive(Default)]
pub struct H264Decoder {
    sequence_parameter_sets: HashMap<u32, Sps>,
    picture_parameter_sets: HashMap<u32, Pps>,
    reference_pictures: ReferencePictureBuffer,
}

impl H264Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn decode_frame(&mut self, data: &[u8]) -> Result<Yuv420Frame, H264DecodeError> {
        let units = parse_annex_b(data)?;
        let mut decoded_frame = None;

        eprintln!("[H264Decoder] decode_frame: {} NAL units", units.len());

        for (i, unit) in units.iter().enumerate() {
            eprintln!(
                "[H264Decoder] NAL unit {}: type={:?}",
                i, unit.header.unit_type
            );

            match unit.header.unit_type {
                NalUnitType::Sps => {
                    let sps = parse_sps(unit.data)?;
                    eprintln!("[H264Decoder] SPS: id={}", sps.id);
                    self.sequence_parameter_sets.insert(sps.id, sps);
                }
                NalUnitType::Pps => {
                    let pps = parse_pps(unit.data)?;
                    eprintln!("[H264Decoder] PPS: id={}, sps_id={}", pps.id, pps.sps_id);
                    self.picture_parameter_sets.insert(pps.id, pps);
                }
                NalUnitType::Idr | NalUnitType::CodedSliceNonIdr => {
                    eprintln!(
                        "[H264Decoder] Decoding slice (NAL type={:?})",
                        unit.header.unit_type
                    );
                    let frame = self.decode_slice(unit)?;
                    decoded_frame = Some(frame);
                }
                NalUnitType::CodedSliceDataPartitionA
                | NalUnitType::CodedSliceDataPartitionB
                | NalUnitType::CodedSliceDataPartitionC
                | NalUnitType::Unspecified
                | NalUnitType::Other(_) => {
                    eprintln!(
                        "[H264Decoder] Unsupported NAL type: {:?}",
                        unit.header.unit_type
                    );
                    return Err(H264DecodeError::UnsupportedFeature);
                }
                NalUnitType::Sei
                | NalUnitType::Aud
                | NalUnitType::EndOfSequence
                | NalUnitType::EndOfStream
                | NalUnitType::Filler => {
                    eprintln!(
                        "[H264Decoder] Skipping NAL type: {:?}",
                        unit.header.unit_type
                    );
                }
            }
        }

        decoded_frame.ok_or(H264DecodeError::MissingIdrSlice)
    }

    fn decode_slice(
        &mut self,
        unit: &crate::video::h264::nal::NalUnit,
    ) -> Result<Yuv420Frame, H264DecodeError> {
        let prefix = crate::video::h264::slice::parse_slice_prefix(unit.data)?;
        let pps = self
            .picture_parameter_sets
            .get(&prefix.pps_id)
            .ok_or(H264DecodeError::MissingParameterSet)?;
        let sps = self
            .sequence_parameter_sets
            .get(&pps.sps_id)
            .ok_or(H264DecodeError::MissingParameterSet)?;
        let header = parse_slice_header(unit.data, sps, pps)?;
        validate_decoder_configuration(sps, pps, &header)?;

        let rbsp = ebsp_to_rbsp(&unit.data[1..]);
        let mut reader = BitReader::new(&rbsp);
        reader.skip_bits(header.slice_data_bit_offset)?;

        if prefix.is_idr {
            self.reference_pictures.clear();
        }
        let max_frame_num = 1u32
            .checked_shl(sps.log2_max_frame_num)
            .ok_or(H264DecodeError::InvalidStream)?;
        let current_pic_order_cnt = i64::from(header.pic_order_cnt_lsb.unwrap_or(header.frame_num));
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
            self.reference_pictures.pictures(),
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

        eprintln!(
            "[H264Decoder] Decoding slice: type={:?}, is_idr={}, frame_num={}",
            header.prefix.slice_type, prefix.is_idr, header.frame_num
        );

        let frame = if pps.entropy_coding_mode {
            match (header.prefix.slice_type, prefix.is_idr) {
                (SliceType::I, _) => reconstruct_slice_cabac(&rbsp, sps, pps, &header)?,
                (SliceType::P, false) | (SliceType::B, false) => {
                    reconstruct_inter_slice_cabac(&rbsp, sps, pps, &header, &reference_lists)?
                }
                _ => {
                    eprintln!(
                        "[H264Decoder] Unsupported CABAC: slice_type={:?}, is_idr={}",
                        header.prefix.slice_type, prefix.is_idr
                    );
                    return Err(H264DecodeError::UnsupportedFeature);
                }
            }
        } else {
            match (header.prefix.slice_type, prefix.is_idr) {
                (SliceType::I, true) => reconstruct_slice(&mut reader, sps, pps, &header)?,
                (SliceType::I, false) => reconstruct_slice(&mut reader, sps, pps, &header)?,
                (SliceType::P, false) | (SliceType::B, false) => {
                    reconstruct_inter_slice(&mut reader, sps, &header, &reference_lists)?
                }
                _ => {
                    eprintln!(
                        "[H264Decoder] Unsupported: slice_type={:?}, is_idr={}",
                        header.prefix.slice_type, prefix.is_idr
                    );
                    return Err(H264DecodeError::UnsupportedFeature);
                }
            }
        };

        if unit.header.nal_ref_idc != 0 {
            let long_term_frame_idx = header
                .dec_ref_pic_marking
                .as_ref()
                .and_then(|marking| marking.long_term_reference_flag)
                .filter(|&is_long_term| prefix.is_idr && is_long_term)
                .map(|_| 0);
            self.reference_pictures.insert(
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

        Ok(frame)
    }

    pub fn reset(&mut self) {
        self.sequence_parameter_sets.clear();
        self.picture_parameter_sets.clear();
        self.reference_pictures.clear();
    }
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
                let frame = if pps.entropy_coding_mode {
                    match (header.prefix.slice_type, prefix.is_idr) {
                        (SliceType::I, _) => reconstruct_slice_cabac(&rbsp, sps, pps, &header)?,
                        (SliceType::P, false) | (SliceType::B, false) => {
                            reconstruct_inter_slice_cabac(
                                &rbsp,
                                sps,
                                pps,
                                &header,
                                &reference_lists,
                            )?
                        }
                        _ => return Err(H264DecodeError::UnsupportedFeature),
                    }
                } else {
                    match (header.prefix.slice_type, prefix.is_idr) {
                        (SliceType::I, true) => reconstruct_slice(&mut reader, sps, pps, &header)?,
                        (SliceType::I, false) => reconstruct_slice(&mut reader, sps, pps, &header)?,
                        (SliceType::P, false) | (SliceType::B, false) => {
                            reconstruct_inter_slice(&mut reader, sps, &header, &reference_lists)?
                        }
                        _ => return Err(H264DecodeError::UnsupportedFeature),
                    }
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
        SliceType::I => true,
        SliceType::P | SliceType::B => !header.prefix.is_idr,
        SliceType::Sp | SliceType::Si => false,
    };
    let inter_header_supported = match header.prefix.slice_type {
        SliceType::P => {
            let weights_default = header.pred_weight_table.as_ref().is_none_or(|t| {
                t.list0
                    .iter()
                    .all(|w| w.luma.is_none() && w.chroma.is_none())
            });
            header.ref_pic_list_modification_l0.is_empty() && weights_default
        }
        SliceType::B => {
            let weights_default = header.pred_weight_table.as_ref().is_none_or(|t| {
                t.list0
                    .iter()
                    .all(|w| w.luma.is_none() && w.chroma.is_none())
                    && t.list1
                        .iter()
                        .all(|w| w.luma.is_none() && w.chroma.is_none())
            });
            header.num_ref_idx_l0_active == 1
                && header.num_ref_idx_l1_active == 1
                && header.ref_pic_list_modification_l0.is_empty()
                && header.ref_pic_list_modification_l1.is_empty()
                && weights_default
        }
        _ => true,
    };

    if !matches!(sps.profile_idc, 66 | 77 | 100) {
        eprintln!("[validate] Unsupported profile: {}", sps.profile_idc);
        return Err(H264DecodeError::UnsupportedFeature);
    }

    if sps.chroma_format_idc != 1 {
        eprintln!(
            "[validate] Unsupported chroma_format_idc: {}",
            sps.chroma_format_idc
        );
        return Err(H264DecodeError::UnsupportedFeature);
    }
    if sps.separate_colour_plane {
        eprintln!("[validate] separate_colour_plane not supported");
        return Err(H264DecodeError::UnsupportedFeature);
    }
    if sps.bit_depth_luma != 8 {
        eprintln!(
            "[validate] Unsupported bit_depth_luma: {}",
            sps.bit_depth_luma
        );
        return Err(H264DecodeError::UnsupportedFeature);
    }
    if sps.bit_depth_chroma != 8 {
        eprintln!(
            "[validate] Unsupported bit_depth_chroma: {}",
            sps.bit_depth_chroma
        );
        return Err(H264DecodeError::UnsupportedFeature);
    }
    if !sps.frame_mbs_only {
        eprintln!("[validate] frame_mbs_only=false not supported (interlaced)");
        return Err(H264DecodeError::UnsupportedFeature);
    }
    if sps.width != coded_width {
        eprintln!(
            "[validate] width mismatch: {} != {}",
            sps.width, coded_width
        );
        return Err(H264DecodeError::UnsupportedFeature);
    }
    if sps.height != coded_height {
        eprintln!(
            "[validate] height mismatch: {} != {}",
            sps.height, coded_height
        );
        return Err(H264DecodeError::UnsupportedFeature);
    }

    if pps.num_slice_groups != 1 {
        eprintln!("[validate] num_slice_groups: {}", pps.num_slice_groups);
        return Err(H264DecodeError::UnsupportedFeature);
    }
    if sps.max_num_ref_frames > 16 {
        eprintln!("[validate] max_num_ref_frames: {}", sps.max_num_ref_frames);
        return Err(H264DecodeError::UnsupportedFeature);
    }

    if !slice_type_supported {
        eprintln!(
            "[validate] Unsupported slice_type: {:?}, is_idr={}",
            header.prefix.slice_type, header.prefix.is_idr
        );
        return Err(H264DecodeError::UnsupportedFeature);
    }

    if header.prefix.first_mb_in_slice != 0 {
        eprintln!(
            "[validate] first_mb_in_slice: {}",
            header.prefix.first_mb_in_slice
        );
        return Err(H264DecodeError::UnsupportedFeature);
    }
    if header.field_pic_flag {
        eprintln!("[validate] field_pic_flag=true (interlaced)");
        return Err(H264DecodeError::UnsupportedFeature);
    }
    if header.redundant_pic_cnt.unwrap_or(0) != 0 {
        eprintln!(
            "[validate] redundant_pic_cnt: {:?}",
            header.redundant_pic_cnt
        );
        return Err(H264DecodeError::UnsupportedFeature);
    }
    if !inter_header_supported {
        eprintln!(
            "[validate] inter_header not supported for {:?}: ref_pic_list_modification_l0={:?}, pred_weight_table={:?}",
            header.prefix.slice_type, header.ref_pic_list_modification_l0, header.pred_weight_table,
        );
        return Err(H264DecodeError::UnsupportedFeature);
    }
    if header.dec_ref_pic_marking.as_ref().is_some_and(|marking| {
        !header.prefix.is_idr
            && (marking.adaptive_ref_pic_marking_mode_flag || !marking.operations.is_empty())
    }) {
        eprintln!("[validate] dec_ref_pic_marking not supported");
        return Err(H264DecodeError::UnsupportedFeature);
    }
    if header.prefix.slice_type == SliceType::B && pps.weighted_bipred_idc != 0 {
        eprintln!(
            "[validate] weighted_bipred_idc: {}",
            pps.weighted_bipred_idc
        );
        return Err(H264DecodeError::UnsupportedFeature);
    }
    if header
        .deblocking_filter
        .as_ref()
        .is_some_and(|parameters| parameters.disable_deblocking_filter_idc > 2)
    {
        eprintln!("[validate] invalid disable_deblocking_filter_idc");
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
        let dc_coefficients = inverse_scale_luma_dc(&dc_block.coeff_levels, qp_y_u8, None)?;
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
        let refs16 = Intra16x16References {
            top,
            left,
            top_left,
        };
        let prediction = predict_intra16x16(prediction_mode, refs16).or_else(|e| {
            if matches!(e, IntraPredictionError::MissingReference) {
                predict_intra16x16(Intra16x16Mode::Dc, refs16)
            } else {
                Err(e)
            }
        })?;

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

#[allow(clippy::too_many_arguments)]
fn decode_and_reconstruct_intra16x16_cabac(
    engine: &mut CabacEngine<'_>,
    contexts: &mut CabacContexts,
    frame: &mut Yuv420Frame,
    sps: &Sps,
    pps: &Pps,
    mb_x: u32,
    mb_y: u32,
    macroblock_address: u32,
    prediction_mode: crate::video::h264::macroblock::types::Intra16x16Mode,
    coded_chroma_pattern: u8,
    coded_luma_pattern: u8,
    prev_qp_delta: &mut i32,
    qp_y: &mut i32,
    grid_width: usize,
    chroma_grid_w: usize,
    cbp_cache: &mut [u32],
    chroma_pred_modes: &mut [u8],
    luma_nonzero_counts: &mut [u8],
    cb_nonzero_counts: &mut [u8],
    cr_nonzero_counts: &mut [u8],
) -> Result<(), H264DecodeError> {
    let cbp_luma = if coded_luma_pattern != 0 { 0x0F } else { 0 };
    let cbp_chroma = (coded_chroma_pattern as u32) << 4;
    cbp_cache[macroblock_address as usize] = cbp_luma | cbp_chroma;
    let left_chroma = if mb_x > 0 {
        chroma_pred_modes[macroblock_address as usize - 1]
    } else {
        0
    };
    let top_chroma = if mb_y > 0 {
        chroma_pred_modes[macroblock_address as usize - sps.pic_width_in_mbs as usize]
    } else {
        0
    };
    let chroma_mode = decode_cabac_chroma_pred_mode(engine, contexts, left_chroma, top_chroma)?;
    chroma_pred_modes[macroblock_address as usize] = chroma_mode;

    let qp_delta = decode_cabac_mb_qp_delta(engine, contexts, *prev_qp_delta)?;
    *prev_qp_delta = qp_delta;
    *qp_y = (*qp_y + qp_delta).rem_euclid(52);
    let qp_y_u8 = u8::try_from(*qp_y).map_err(|_| H264DecodeError::InvalidStream)?;
    let qp_c = chroma_qp_from_luma(*qp_y, pps.chroma_qp_index_offset);

    let left_has_dc = if mb_x > 0 {
        (cbp_cache[macroblock_address as usize - 1] & 0x100) != 0
    } else {
        false
    };
    let top_has_dc = if mb_y > 0 {
        (cbp_cache[macroblock_address as usize - sps.pic_width_in_mbs as usize] & 0x100) != 0
    } else {
        false
    };

    let (dc_block, dc_nnz) = decode_cabac_residual_block(
        engine,
        contexts,
        ResidualCategory::LumaDc,
        left_has_dc,
        top_has_dc,
    )?;
    if dc_nnz > 0 {
        cbp_cache[macroblock_address as usize] |= 0x100;
    }

    let dc_coefficients = inverse_scale_luma_dc(&dc_block, qp_y_u8, None)?;
    let dc_transform = inverse_hadamard_4x4(&dc_coefficients)?;
    let mut ac_blocks = [[0i32; 16]; 16];

    if coded_luma_pattern != 0 {
        for (block_index, ac_block) in ac_blocks.iter_mut().enumerate() {
            let block_x = block_index % 4;
            let block_y = block_index / 4;
            let grid_x = mb_x as usize * 4 + block_x;
            let grid_y = mb_y as usize * 4 + block_y;

            let cond_a = if grid_x > 0 {
                luma_nonzero_counts[grid_y * grid_width + grid_x - 1] > 0
            } else {
                false
            };
            let cond_b = if grid_y > 0 {
                luma_nonzero_counts[(grid_y - 1) * grid_width + grid_x] > 0
            } else {
                false
            };

            let (block, nnz) = decode_cabac_residual_block(
                engine,
                contexts,
                ResidualCategory::LumaAc,
                cond_a,
                cond_b,
            )?;
            luma_nonzero_counts[grid_y * grid_width + grid_x] = nnz as u8;
            *ac_block = block;
        }
    }

    let (chroma_dc_cb, chroma_dc_cr) = decode_chroma_dc_cabac(
        engine,
        contexts,
        u32::from(coded_chroma_pattern),
        mb_x,
        mb_y,
        sps.pic_width_in_mbs,
        cbp_cache,
    )?;
    let (chroma_ac_cb, chroma_ac_cr) = if coded_chroma_pattern >= 2 {
        decode_chroma_ac_cabac(
            engine,
            contexts,
            mb_x,
            mb_y,
            chroma_grid_w,
            cb_nonzero_counts,
            cr_nonzero_counts,
        )?
    } else {
        ([[0i32; 16]; 4], [[0i32; 16]; 4])
    };

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
    let intra_mode = match prediction_mode {
        crate::video::h264::macroblock::types::Intra16x16Mode::Vertical => Intra16x16Mode::Vertical,
        crate::video::h264::macroblock::types::Intra16x16Mode::Horizontal => {
            Intra16x16Mode::Horizontal
        }
        crate::video::h264::macroblock::types::Intra16x16Mode::Dc => Intra16x16Mode::Dc,
        crate::video::h264::macroblock::types::Intra16x16Mode::Plane => Intra16x16Mode::Plane,
    };
    let refs16 = Intra16x16References {
        top,
        left,
        top_left,
    };
    let prediction = predict_intra16x16(intra_mode, refs16).or_else(|e| {
        if matches!(e, IntraPredictionError::MissingReference) {
            predict_intra16x16(Intra16x16Mode::Dc, refs16)
        } else {
            Err(e)
        }
    })?;

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

    reconstruct_chroma_blocks(
        frame,
        mb_x,
        mb_y,
        chroma_mode,
        &chroma_dc_cb,
        &chroma_dc_cr,
        &chroma_ac_cb,
        &chroma_ac_cr,
        qp_c,
        chroma_grid_w,
        cb_nonzero_counts,
        cr_nonzero_counts,
    )?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn decode_and_reconstruct_intra4x4_cabac(
    engine: &mut CabacEngine<'_>,
    contexts: &mut CabacContexts,
    frame: &mut Yuv420Frame,
    sps: &Sps,
    pps: &Pps,
    mb_x: u32,
    mb_y: u32,
    macroblock_address: u32,
    prev_qp_delta: &mut i32,
    qp_y: &mut i32,
    grid_width: usize,
    chroma_grid_w: usize,
    cbp_cache: &mut [u32],
    chroma_pred_modes: &mut [u8],
    intra4x4_modes: &mut [Intra4x4Mode],
    mb_types: &[Option<MacroblockType>],
    luma_nonzero_counts: &mut [u8],
    cb_nonzero_counts: &mut [u8],
    cr_nonzero_counts: &mut [u8],
) -> Result<(), H264DecodeError> {
    let mut modes = [Intra4x4Mode::Dc; 16];
    for b_idx in 0..16 {
        let b_8x8 = b_idx / 4;
        let b_4x4 = b_idx % 4;
        let bx = (b_8x8 % 2) * 2 + (b_4x4 % 2);
        let by = (b_8x8 / 2) * 2 + (b_4x4 / 2);
        let gx = mb_x as usize * 4 + bx;
        let gy = mb_y as usize * 4 + by;

        let pred_a = if bx > 0 {
            Some(intra4x4_modes[gy * grid_width + (gx - 1)])
        } else if mb_x > 0 {
            match mb_types[macroblock_address as usize - 1] {
                Some(MacroblockType::Intra4x4) => Some(intra4x4_modes[gy * grid_width + (gx - 1)]),
                Some(MacroblockType::Intra16x16 { .. }) => Some(Intra4x4Mode::Dc),
                _ => None,
            }
        } else {
            None
        };

        let pred_b = if by > 0 {
            Some(intra4x4_modes[(gy - 1) * grid_width + gx])
        } else if mb_y > 0 {
            match mb_types[macroblock_address as usize - sps.pic_width_in_mbs as usize] {
                Some(MacroblockType::Intra4x4) => Some(intra4x4_modes[(gy - 1) * grid_width + gx]),
                Some(MacroblockType::Intra16x16 { .. }) => Some(Intra4x4Mode::Dc),
                _ => None,
            }
        } else {
            None
        };

        let predicted_mode = if let (Some(mode_a), Some(mode_b)) = (pred_a, pred_b) {
            (mode_a as u8).min(mode_b as u8)
        } else {
            2
        };

        let mode_val = decode_intra4x4_pred_mode(engine, contexts, predicted_mode)?;
        let mode = match mode_val {
            0 => Intra4x4Mode::Vertical,
            1 => Intra4x4Mode::Horizontal,
            2 => Intra4x4Mode::Dc,
            3 => Intra4x4Mode::DiagonalDownLeft,
            4 => Intra4x4Mode::DiagonalDownRight,
            5 => Intra4x4Mode::VerticalRight,
            6 => Intra4x4Mode::HorizontalDown,
            7 => Intra4x4Mode::VerticalLeft,
            8 => Intra4x4Mode::HorizontalUp,
            _ => Intra4x4Mode::Dc,
        };
        modes[b_idx] = mode;
        intra4x4_modes[gy * grid_width + gx] = mode;
    }

    let left_chroma = if mb_x > 0 {
        chroma_pred_modes[macroblock_address as usize - 1]
    } else {
        0
    };
    let top_chroma = if mb_y > 0 {
        chroma_pred_modes[macroblock_address as usize - sps.pic_width_in_mbs as usize]
    } else {
        0
    };
    let chroma_mode = decode_cabac_chroma_pred_mode(engine, contexts, left_chroma, top_chroma)?;
    chroma_pred_modes[macroblock_address as usize] = chroma_mode;

    let left_cbp = if mb_x > 0 {
        cbp_cache[macroblock_address as usize - 1]
    } else {
        0x7CF
    };
    let top_cbp = if mb_y > 0 {
        cbp_cache[macroblock_address as usize - sps.pic_width_in_mbs as usize]
    } else {
        0x7CF
    };
    let cbp = decode_cabac_cbp(engine, contexts, left_cbp, top_cbp)?;
    cbp_cache[macroblock_address as usize] = cbp;

    if cbp != 0 {
        let qp_delta = decode_cabac_mb_qp_delta(engine, contexts, *prev_qp_delta)?;
        *prev_qp_delta = qp_delta;
        *qp_y = (*qp_y + qp_delta).rem_euclid(52);
    } else {
        *prev_qp_delta = 0;
    }
    let qp_y_u8 = u8::try_from(*qp_y).map_err(|_| H264DecodeError::InvalidStream)?;
    let qp_c = chroma_qp_from_luma(*qp_y, pps.chroma_qp_index_offset);
    let coded_chroma_pattern = (cbp >> 4) & 0x3;

    for b_idx in 0..16usize {
        let b_8x8 = b_idx / 4;
        let b_4x4 = b_idx % 4;
        let bx = ((b_8x8 % 2) * 2 + (b_4x4 % 2)) as u32;
        let by = ((b_8x8 / 2) * 2 + (b_4x4 / 2)) as u32;
        let blk_x = mb_x * 16 + bx * 4;
        let blk_y = mb_y * 16 + by * 4;

        let gx = mb_x as usize * 4 + bx as usize;
        let gy = mb_y as usize * 4 + by as usize;
        let cond_a = if gx > 0 {
            luma_nonzero_counts[gy * grid_width + gx - 1] > 0
        } else {
            false
        };
        let cond_b = if gy > 0 {
            luma_nonzero_counts[(gy - 1) * grid_width + gx] > 0
        } else {
            false
        };

        let (residual_levels, nnz) = if (cbp & (1 << (b_idx / 4))) != 0 {
            decode_cabac_residual_block(
                engine,
                contexts,
                ResidualCategory::Luma4x4,
                cond_a,
                cond_b,
            )?
        } else {
            ([0i32; 16], 0)
        };
        luma_nonzero_counts[gy * grid_width + gx] = nnz as u8;

        let has_top = blk_y > 0;
        let has_left = blk_x > 0;
        let top = has_top.then(|| {
            std::array::from_fn(|i| {
                frame.y[((blk_y - 1) * frame.width + blk_x + i as u32) as usize]
            })
        });
        let left = has_left.then(|| {
            std::array::from_fn(|i| {
                frame.y[((blk_y + i as u32) * frame.width + blk_x - 1) as usize]
            })
        });
        let top_left = (has_top && has_left)
            .then(|| frame.y[((blk_y - 1) * frame.width + blk_x - 1) as usize]);
        let top_right = if has_top {
            let raster_in_mb = by as usize * 4 + bx as usize;
            let avail_tr = match raster_in_mb {
                3 | 7 | 11 | 15 => false,
                _ => blk_x + 4 <= frame.width,
            };
            if avail_tr {
                Some(std::array::from_fn(|i| {
                    frame.y[((blk_y - 1) * frame.width + blk_x + 4 + i as u32) as usize]
                }))
            } else {
                top.map(|t| [t[3], t[3], t[3], t[3]])
            }
        } else {
            None
        };

        let refs = Intra4x4References {
            top,
            left,
            top_left,
            top_right,
        };
        let result = reconstruct_intra4x4_block(
            frame,
            Intra4x4Reconstruction {
                plane: ColorPlane::Y,
                x: blk_x,
                y: blk_y,
                mode: modes[b_idx],
                references: refs,
                quantized_levels: &residual_levels,
                qp: qp_y_u8,
                scaling_list: None,
            },
        );
        if let Err(IntraReconstructionError::Prediction(IntraPredictionError::MissingReference)) =
            result
        {
            reconstruct_intra4x4_block(
                frame,
                Intra4x4Reconstruction {
                    plane: ColorPlane::Y,
                    x: blk_x,
                    y: blk_y,
                    mode: Intra4x4Mode::Dc,
                    references: refs,
                    quantized_levels: &residual_levels,
                    qp: qp_y_u8,
                    scaling_list: None,
                },
            )
            .map_err(|_| H264DecodeError::InvalidStream)?;
        } else {
            result.map_err(|_| H264DecodeError::InvalidStream)?;
        }
    }

    let (chroma_dc_cb, chroma_dc_cr) = decode_chroma_dc_cabac(
        engine,
        contexts,
        coded_chroma_pattern,
        mb_x,
        mb_y,
        sps.pic_width_in_mbs,
        cbp_cache,
    )?;
    let (chroma_ac_cb, chroma_ac_cr) = if coded_chroma_pattern >= 2 {
        decode_chroma_ac_cabac(
            engine,
            contexts,
            mb_x,
            mb_y,
            chroma_grid_w,
            cb_nonzero_counts,
            cr_nonzero_counts,
        )?
    } else {
        ([[0i32; 16]; 4], [[0i32; 16]; 4])
    };

    reconstruct_chroma_blocks(
        frame,
        mb_x,
        mb_y,
        chroma_mode,
        &chroma_dc_cb,
        &chroma_dc_cr,
        &chroma_ac_cb,
        &chroma_ac_cr,
        qp_c,
        chroma_grid_w,
        cb_nonzero_counts,
        cr_nonzero_counts,
    )?;

    Ok(())
}

fn reconstruct_slice_cabac(
    rbsp: &[u8],
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

    let chroma_grid_w = (sps.pic_width_in_mbs * 2) as usize;
    let chroma_grid_h = (sps.pic_height_in_map_units * 2) as usize;
    let chroma_grid_len = chroma_grid_w
        .checked_mul(chroma_grid_h)
        .ok_or(H264DecodeError::InvalidStream)?;
    let mut luma_nonzero_counts = vec![0u8; grid_length];
    let mut cb_nonzero_counts = vec![0u8; chroma_grid_len];
    let mut cr_nonzero_counts = vec![0u8; chroma_grid_len];

    let byte_offset = header.slice_data_bit_offset / 8;
    eprintln!(
        "[CABAC INIT] slice_data_bit_offset={} byte_offset={} rbsp_len={} initial_qp={} cabac_init_idc={:?}",
        header.slice_data_bit_offset,
        byte_offset,
        rbsp.len(),
        26i32 + pps.pic_init_qp_minus26 + header.slice_qp_delta,
        header.cabac_init_idc
    );
    eprintln!(
        "[CABAC INIT] First 16 CABAC bytes: {:02x?}",
        &rbsp[byte_offset..byte_offset.min(rbsp.len()) + (16.min(rbsp.len() - byte_offset))]
    );
    if byte_offset >= rbsp.len() {
        return Err(H264DecodeError::InvalidStream);
    }
    let mut engine = CabacEngine::new(&rbsp[byte_offset..])?;
    let initial_qp = 26i32 + pps.pic_init_qp_minus26 + header.slice_qp_delta;
    if !(0..=51).contains(&initial_qp) {
        return Err(H264DecodeError::InvalidStream);
    }
    let mut contexts =
        CabacContexts::new(SliceType::I, header.cabac_init_idc.unwrap_or(0), initial_qp);
    let mut qp_y = initial_qp;
    let mut prev_qp_delta = 0i32;

    let mut mb_types = vec![None; macroblock_count as usize];
    let mut cbp_cache = vec![0u32; macroblock_count as usize];
    let mut chroma_pred_modes = vec![0u8; macroblock_count as usize];
    let mut intra4x4_modes = vec![Intra4x4Mode::Dc; (macroblock_count * 16) as usize];

    for macroblock_address in 0..macroblock_count {
        let mb_x = macroblock_address % sps.pic_width_in_mbs;
        let mb_y = macroblock_address / sps.pic_width_in_mbs;


        eprintln!(
            "[CABAC STATE] mb_addr={} bits={} range={} offset={}",
            macroblock_address, engine.bits_consumed, engine.range, engine.offset
        );

        let left_is_intra16 = mb_x > 0
            && matches!(
                mb_types[macroblock_address as usize - 1],
                Some(MacroblockType::Intra16x16 { .. })
            );
        let top_is_intra16 = mb_y > 0
            && matches!(
                mb_types[macroblock_address as usize - sps.pic_width_in_mbs as usize],
                Some(MacroblockType::Intra16x16 { .. })
            );

        let mb_type =
            decode_i_slice_mb_type(&mut engine, &mut contexts, left_is_intra16, top_is_intra16)?;
        mb_types[macroblock_address as usize] = Some(mb_type);
        eprintln!(
            "[reconstruct_slice_cabac] mb_addr: {}/{}, mb_type: {:?}",
            macroblock_address, macroblock_count, mb_type
        );

        match mb_type {
            MacroblockType::Intra16x16 {
                prediction_mode,
                coded_chroma_pattern,
                coded_luma_pattern,
            } => {
                decode_and_reconstruct_intra16x16_cabac(
                    &mut engine,
                    &mut contexts,
                    &mut frame,
                    sps,
                    pps,
                    mb_x,
                    mb_y,
                    macroblock_address,
                    prediction_mode,
                    coded_chroma_pattern,
                    coded_luma_pattern,
                    &mut prev_qp_delta,
                    &mut qp_y,
                    grid_width as usize,
                    chroma_grid_w,
                    &mut cbp_cache,
                    &mut chroma_pred_modes,
                    &mut luma_nonzero_counts,
                    &mut cb_nonzero_counts,
                    &mut cr_nonzero_counts,
                )?;
            }
            MacroblockType::Intra4x4 => {
                decode_and_reconstruct_intra4x4_cabac(
                    &mut engine,
                    &mut contexts,
                    &mut frame,
                    sps,
                    pps,
                    mb_x,
                    mb_y,
                    macroblock_address,
                    &mut prev_qp_delta,
                    &mut qp_y,
                    grid_width as usize,
                    chroma_grid_w,
                    &mut cbp_cache,
                    &mut chroma_pred_modes,
                    &mut intra4x4_modes,
                    &mb_types,
                    &mut luma_nonzero_counts,
                    &mut cb_nonzero_counts,
                    &mut cr_nonzero_counts,
                )?;
            }
            _ => return Err(H264DecodeError::UnsupportedFeature),
        }

        if mb_y == 0 {
            let offset = (mb_x * 16) as usize;
            eprintln!(
                "Our MB {} Y[0..7]: {:?}",
                macroblock_address,
                &frame.y[offset..offset + 8]
            );
        }

        if engine.decode_terminate()? == 1 {
            break;
        }
    }

    Ok(frame)
}

fn reconstruct_inter_slice_cabac(
    rbsp: &[u8],
    sps: &Sps,
    pps: &Pps,
    header: &crate::video::h264::slice::SliceHeader,
    reference_lists: &ReferencePictureLists<'_>,
) -> Result<Yuv420Frame, H264DecodeError> {
    let macroblock_count = sps
        .pic_width_in_mbs
        .checked_mul(sps.pic_height_in_map_units)
        .ok_or(H264DecodeError::InvalidStream)?;
    let grid_width = (sps.pic_width_in_mbs * 4) as usize;
    let grid_height = (sps.pic_height_in_map_units * 4) as usize;

    let byte_offset = header.slice_data_bit_offset / 8;
    if byte_offset >= rbsp.len() {
        return Err(H264DecodeError::InvalidStream);
    }
    let mut engine = CabacEngine::new(&rbsp[byte_offset..])?;
    let initial_qp = 26i32 + pps.pic_init_qp_minus26 + header.slice_qp_delta;
    if !(0..=51).contains(&initial_qp) {
        return Err(H264DecodeError::InvalidStream);
    }

    match header.prefix.slice_type {
        SliceType::P => {
            let mut contexts =
                CabacContexts::new(SliceType::P, header.cabac_init_idc.unwrap_or(0), initial_qp);
            let mut frame = Yuv420Frame::new(sps.width, sps.height)?;
            let mut motion_grid = MotionGrid::new(grid_width, grid_height)?;
            let mut skipped = vec![false; macroblock_count as usize];
            let mut cbp_cache = vec![0u32; macroblock_count as usize];
            let mut mvd_cache = vec![[0i32; 2]; macroblock_count as usize];
            let mut prev_qp_delta = 0i32;
            let mut qp_y = initial_qp;

            let mut intra4x4_modes = vec![Intra4x4Mode::Dc; (macroblock_count * 16) as usize];
            let mut chroma_pred_modes = vec![0u8; macroblock_count as usize];
            let mut mb_types: Vec<Option<MacroblockType>> = vec![None; macroblock_count as usize];
            let mut luma_nonzero_counts = vec![0u8; grid_width * grid_height];
            let mut cb_nonzero_counts =
                vec![0u8; (sps.pic_width_in_mbs * 2 * sps.pic_height_in_map_units * 2) as usize];
            let mut cr_nonzero_counts =
                vec![0u8; (sps.pic_width_in_mbs * 2 * sps.pic_height_in_map_units * 2) as usize];
            let chroma_grid_w = (sps.pic_width_in_mbs * 2) as usize;

            for address in 0..macroblock_count {
                let mb_x = address % sps.pic_width_in_mbs;
                let mb_y = address / sps.pic_width_in_mbs;

                eprintln!(
                    "[P CABAC STATE] mb_addr={} bits={} range={} offset={}",
                    address, engine.bits_consumed, engine.range, engine.offset
                );

                let left_avail = mb_x > 0;
                let top_avail = mb_y > 0;
                let left_skip = left_avail && skipped[address as usize - 1];
                let top_skip =
                    top_avail && skipped[address as usize - sps.pic_width_in_mbs as usize];

                let is_skip = decode_mb_skip_flag(
                    &mut engine,
                    &mut contexts,
                    SliceType::P,
                    left_avail,
                    left_skip,
                    top_avail,
                    top_skip,
                )?;

                if is_skip {
                    skipped[address as usize] = true;
                    prev_qp_delta = 0;
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
                } else {
                    eprintln!(
                        "[P-slice] mb_addr={}/{} (x={}, y={}) is_skip=false",
                        address, macroblock_count, mb_x, mb_y
                    );
                    let mb_type_opt =
                        match decode_p_slice_mb_type(&mut engine, &mut contexts, false, false) {
                            Ok(t) => t,
                            Err(e) => {
                                eprintln!("[P-slice] decode_p_slice_mb_type error: {:?}", e);
                                return Err(e);
                            }
                        };
                    eprintln!(
                        "[P-slice] mb_addr={} mb_type_opt={:?}",
                        address, mb_type_opt
                    );

                    match mb_type_opt {
                        None => {
                            let ref_a = if mb_x > 0 {
                                motion_grid
                                    .cell((mb_x as usize - 1) * 4, mb_y as usize * 4)
                                    .map(|(r, _)| r)
                            } else {
                                None
                            };
                            let ref_b = if mb_y > 0 {
                                motion_grid
                                    .cell(mb_x as usize * 4, (mb_y as usize - 1) * 4)
                                    .map(|(r, _)| r)
                            } else {
                                None
                            };
                            let ref_idx = decode_cabac_ref_idx(
                                &mut engine,
                                &mut contexts,
                                ref_a,
                                ref_b,
                                header.num_ref_idx_l0_active,
                            )?;

                            let mvd_a = if mb_x > 0 {
                                mvd_cache[address as usize - 1]
                            } else {
                                [0, 0]
                            };
                            let mvd_b = if mb_y > 0 {
                                mvd_cache[address as usize - sps.pic_width_in_mbs as usize]
                            } else {
                                [0, 0]
                            };
                            let amvd_x = mvd_a[0].abs() + mvd_b[0].abs();
                            let amvd_y = mvd_a[1].abs() + mvd_b[1].abs();
                            let mvd_x = decode_cabac_mvd(&mut engine, &mut contexts, amvd_x, 40)?;
                            let mvd_y = decode_cabac_mvd(&mut engine, &mut contexts, amvd_y, 47)?;
                            mvd_cache[address as usize] = [mvd_x, mvd_y];

                            let left_cbp = if mb_x > 0 {
                                cbp_cache[address as usize - 1]
                            } else {
                                0x00F
                            };
                            let top_cbp = if mb_y > 0 {
                                cbp_cache[address as usize - sps.pic_width_in_mbs as usize]
                            } else {
                                0x00F
                            };
                            let cbp =
                                decode_cabac_cbp(&mut engine, &mut contexts, left_cbp, top_cbp)?;
                            eprintln!(
                                "[P-slice mb 81] ref_idx={} mvd=({},{}) cbp={:#x}",
                                ref_idx, mvd_x, mvd_y, cbp
                            );
                            cbp_cache[address as usize] = cbp;

                            if cbp != 0 {
                                let qp_delta = decode_cabac_mb_qp_delta(
                                    &mut engine,
                                    &mut contexts,
                                    prev_qp_delta,
                                )?;
                                prev_qp_delta = qp_delta;
                                qp_y = (qp_y + qp_delta).rem_euclid(52);
                            } else {
                                prev_qp_delta = 0;
                            }

                            let motion_prediction = motion_grid.predict(mb_x, mb_y, ref_idx);
                            let motion = MotionVector {
                                x: motion_prediction
                                    .x
                                    .checked_add(mvd_x)
                                    .ok_or(H264DecodeError::InvalidStream)?,
                                y: motion_prediction
                                    .y
                                    .checked_add(mvd_y)
                                    .ok_or(H264DecodeError::InvalidStream)?,
                            };
                            let reference = reference_picture(&reference_lists.list0, ref_idx)?;
                            motion_compensate_macroblock(
                                &mut frame,
                                &reference.frame,
                                mb_x,
                                mb_y,
                                motion,
                            )?;
                            motion_grid.set_macroblock(mb_x, mb_y, ref_idx, motion);

                            if cbp != 0 {
                                decode_and_apply_inter_residual_cabac(
                                    &mut engine,
                                    &mut contexts,
                                    &mut frame,
                                    sps,
                                    pps,
                                    mb_x,
                                    mb_y,
                                    cbp,
                                    qp_y,
                                    grid_width,
                                    chroma_grid_w,
                                    &cbp_cache,
                                    &mut luma_nonzero_counts,
                                    &mut cb_nonzero_counts,
                                    &mut cr_nonzero_counts,
                                )?;
                            }
                        }
                        Some(MacroblockType::Intra16x16 {
                            prediction_mode,
                            coded_chroma_pattern,
                            coded_luma_pattern,
                        }) => {
                            mb_types[address as usize] = Some(MacroblockType::Intra16x16 {
                                prediction_mode,
                                coded_chroma_pattern,
                                coded_luma_pattern,
                            });
                            decode_and_reconstruct_intra16x16_cabac(
                                &mut engine,
                                &mut contexts,
                                &mut frame,
                                sps,
                                pps,
                                mb_x,
                                mb_y,
                                address,
                                prediction_mode,
                                coded_chroma_pattern,
                                coded_luma_pattern,
                                &mut prev_qp_delta,
                                &mut qp_y,
                                grid_width,
                                chroma_grid_w,
                                &mut cbp_cache,
                                &mut chroma_pred_modes,
                                &mut luma_nonzero_counts,
                                &mut cb_nonzero_counts,
                                &mut cr_nonzero_counts,
                            )?;
                        }
                        Some(MacroblockType::Intra4x4) => {
                            mb_types[address as usize] = Some(MacroblockType::Intra4x4);
                            decode_and_reconstruct_intra4x4_cabac(
                                &mut engine,
                                &mut contexts,
                                &mut frame,
                                sps,
                                pps,
                                mb_x,
                                mb_y,
                                address,
                                &mut prev_qp_delta,
                                &mut qp_y,
                                grid_width,
                                chroma_grid_w,
                                &mut cbp_cache,
                                &mut chroma_pred_modes,
                                &mut intra4x4_modes,
                                &mb_types,
                                &mut luma_nonzero_counts,
                                &mut cb_nonzero_counts,
                                &mut cr_nonzero_counts,
                            )?;
                        }
                        _ => return Err(H264DecodeError::UnsupportedFeature),
                    }
                }

                if engine.decode_terminate()? == 1 {
                    break;
                }
            }

            Ok(frame)
        }
        SliceType::B => {
            let mut contexts =
                CabacContexts::new(SliceType::B, header.cabac_init_idc.unwrap_or(0), initial_qp);
            for address in 0..macroblock_count {
                let mb_x = address % sps.pic_width_in_mbs;
                let mb_y = address / sps.pic_width_in_mbs;

                let is_skip = decode_mb_skip_flag(
                    &mut engine,
                    &mut contexts,
                    SliceType::B,
                    mb_x > 0,
                    false,
                    mb_y > 0,
                    false,
                )?;
                if !is_skip {
                    return Err(H264DecodeError::UnsupportedFeature);
                }
                if engine.decode_terminate()? == 1 {
                    break;
                }
            }
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

#[allow(clippy::too_many_arguments)]
fn decode_and_apply_inter_residual_cabac(
    engine: &mut CabacEngine<'_>,
    contexts: &mut CabacContexts,
    frame: &mut Yuv420Frame,
    sps: &Sps,
    pps: &Pps,
    mb_x: u32,
    mb_y: u32,
    cbp: u32,
    qp_y: i32,
    grid_width: usize,
    chroma_grid_w: usize,
    cbp_cache: &[u32],
    luma_nonzero_counts: &mut [u8],
    cb_nonzero_counts: &mut [u8],
    cr_nonzero_counts: &mut [u8],
) -> Result<(), H264DecodeError> {
    let qp_y_u8 = u8::try_from(qp_y).map_err(|_| H264DecodeError::InvalidStream)?;
    let qp_c = chroma_qp_from_luma(qp_y, pps.chroma_qp_index_offset);


    for b_idx in 0..16usize {
        let b_8x8 = b_idx / 4;
        let b_4x4 = b_idx % 4;
        let bx = (b_8x8 % 2) * 2 + (b_4x4 % 2);
        let by = (b_8x8 / 2) * 2 + (b_4x4 / 2);
        let gx = mb_x as usize * 4 + bx;
        let gy = mb_y as usize * 4 + by;
        let cond_a = if gx > 0 {
            luma_nonzero_counts[gy * grid_width + gx - 1] > 0
        } else {
            false
        };
        let cond_b = if gy > 0 {
            luma_nonzero_counts[(gy - 1) * grid_width + gx] > 0
        } else {
            false
        };

        let (residual_levels, nnz) = if (cbp & (1 << (b_idx / 4))) != 0 {
            decode_cabac_residual_block(
                engine,
                contexts,
                ResidualCategory::Luma4x4,
                cond_a,
                cond_b,
            )?
        } else {
            ([0i32; 16], 0)
        };
        luma_nonzero_counts[gy * grid_width + gx] = nnz as u8;

        if nnz > 0 {
            eprintln!(
                "[inter residual] mb({},{}) b_idx={} cond_a={} cond_b={} nnz={}: {:?}",
                mb_x,
                mb_y,
                b_idx,
                cond_a,
                cond_b,
                nnz,
                &residual_levels[..8]
            );
            let residual = reconstruct_residual_4x4(&residual_levels, qp_y_u8, None)?;
            let blk_x = mb_x * 16 + bx as u32 * 4;
            let blk_y = mb_y * 16 + by as u32 * 4;
            for row in 0..4u32 {
                let row_offset = ((blk_y + row) * frame.width + blk_x) as usize;
                for col in 0..4u32 {
                    let idx = (row * 4 + col) as usize;
                    let sample = i32::from(frame.y[row_offset + col as usize]) + residual[idx];
                    frame.y[row_offset + col as usize] = sample.clamp(0, 255) as u8;
                }
            }
        }
    }


    let coded_chroma_pattern = (cbp >> 4) & 0x3;
    let (chroma_dc_cb, chroma_dc_cr) = decode_chroma_dc_cabac(
        engine,
        contexts,
        coded_chroma_pattern,
        mb_x,
        mb_y,
        sps.pic_width_in_mbs,
        cbp_cache,
    )?;
    let (chroma_ac_cb, chroma_ac_cr) = if coded_chroma_pattern >= 2 {
        decode_chroma_ac_cabac(
            engine,
            contexts,
            mb_x,
            mb_y,
            chroma_grid_w,
            cb_nonzero_counts,
            cr_nonzero_counts,
        )?
    } else {
        ([[0i32; 16]; 4], [[0i32; 16]; 4])
    };

    if coded_chroma_pattern > 0 {
        let dc_trans_cb = inverse_hadamard_2x2(&chroma_dc_cb);
        let dc_scaled_cb: [i32; 4] = std::array::from_fn(|i| scale_chroma_dc(dc_trans_cb[i], qp_c));
        let (cw, _) = frame.plane_dimensions(ColorPlane::U);
        for b_idx in 0..4usize {
            let bx = (b_idx % 2) as u32;
            let by = (b_idx / 2) as u32;
            let blk_x = mb_x * 8 + bx * 4;
            let blk_y = mb_y * 8 + by * 4;
            let mut scaled_ac = inverse_scale_4x4(&chroma_ac_cb[b_idx], qp_c, None)?;
            scaled_ac[0] = dc_scaled_cb[b_idx];
            let residual = inverse_transform_4x4(&scaled_ac)?;
            for row in 0..4u32 {
                let row_offset = ((blk_y + row) * cw + blk_x) as usize;
                for col in 0..4u32 {
                    let idx = (row * 4 + col) as usize;
                    let sample = i32::from(frame.u[row_offset + col as usize]) + residual[idx];
                    frame.u[row_offset + col as usize] = sample.clamp(0, 255) as u8;
                }
            }
        }

        let dc_trans_cr = inverse_hadamard_2x2(&chroma_dc_cr);
        let dc_scaled_cr: [i32; 4] = std::array::from_fn(|i| scale_chroma_dc(dc_trans_cr[i], qp_c));
        let (cw, _) = frame.plane_dimensions(ColorPlane::V);
        for b_idx in 0..4usize {
            let bx = (b_idx % 2) as u32;
            let by = (b_idx / 2) as u32;
            let blk_x = mb_x * 8 + bx * 4;
            let blk_y = mb_y * 8 + by * 4;
            let mut scaled_ac = inverse_scale_4x4(&chroma_ac_cr[b_idx], qp_c, None)?;
            scaled_ac[0] = dc_scaled_cr[b_idx];
            let residual = inverse_transform_4x4(&scaled_ac)?;
            for row in 0..4u32 {
                let row_offset = ((blk_y + row) * cw + blk_x) as usize;
                for col in 0..4u32 {
                    let idx = (row * 4 + col) as usize;
                    let sample = i32::from(frame.v[row_offset + col as usize]) + residual[idx];
                    frame.v[row_offset + col as usize] = sample.clamp(0, 255) as u8;
                }
            }
        }
    }

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

#[allow(clippy::too_many_arguments)]
fn decode_chroma_dc_cabac(
    engine: &mut CabacEngine<'_>,
    contexts: &mut CabacContexts,
    coded_chroma_pattern: u32,
    mb_x: u32,
    mb_y: u32,
    pic_width_in_mbs: u32,
    cbp_cache: &[u32],
) -> Result<([i32; 4], [i32; 4]), H264DecodeError> {
    if coded_chroma_pattern == 0 {
        return Ok(([0i32; 4], [0i32; 4]));
    }


    let cond_a = if mb_x > 0 {
        ((cbp_cache[mb_y as usize * pic_width_in_mbs as usize + mb_x as usize - 1] >> 4) & 0x3) > 0
    } else {
        false
    };
    let cond_b = if mb_y > 0 {
        ((cbp_cache[(mb_y as usize - 1) * pic_width_in_mbs as usize + mb_x as usize] >> 4) & 0x3)
            > 0
    } else {
        false
    };


    let (block_cb, nnz_cb) =
        decode_cabac_residual_block(engine, contexts, ResidualCategory::ChromaDc, cond_a, cond_b)?;
    let dc_cb = [block_cb[0], block_cb[1], block_cb[2], block_cb[3]];


    let (block_cr, nnz_cr) =
        decode_cabac_residual_block(engine, contexts, ResidualCategory::ChromaDc, cond_a, cond_b)?;
    let dc_cr = [block_cr[0], block_cr[1], block_cr[2], block_cr[3]];
    eprintln!(
        "[ChromaDC mb({},{})] Cb nnz={}: {:?}, Cr nnz={}: {:?}",
        mb_x, mb_y, nnz_cb, dc_cb, nnz_cr, dc_cr
    );

    Ok((dc_cb, dc_cr))
}



fn decode_chroma_ac_cabac(
    engine: &mut CabacEngine<'_>,
    contexts: &mut CabacContexts,
    mb_x: u32,
    mb_y: u32,
    chroma_grid_w: usize,
    cb_nonzero_counts: &mut [u8],
    cr_nonzero_counts: &mut [u8],
) -> Result<([[i32; 16]; 4], [[i32; 16]; 4]), H264DecodeError> {
    let mut ac_cb = [[0i32; 16]; 4];
    let mut ac_cr = [[0i32; 16]; 4];

    for b_idx in 0..4usize {
        let bx = b_idx % 2;
        let by = b_idx / 2;
        let gx = mb_x as usize * 2 + bx;
        let gy = mb_y as usize * 2 + by;
        let cond_a = if gx > 0 {
            cb_nonzero_counts[gy * chroma_grid_w + gx - 1] > 0
        } else {
            false
        };
        let cond_b = if gy > 0 {
            cb_nonzero_counts[(gy - 1) * chroma_grid_w + gx] > 0
        } else {
            false
        };
        let (block, nnz) = decode_cabac_residual_block(
            engine,
            contexts,
            ResidualCategory::ChromaAc,
            cond_a,
            cond_b,
        )?;
        cb_nonzero_counts[gy * chroma_grid_w + gx] = nnz as u8;
        if nnz > 0 {
            eprintln!(
                "[ChromaAC Cb mb({},{}) b={}] nnz={}: {:?}",
                mb_x,
                mb_y,
                b_idx,
                nnz,
                &block[..8]
            );
        }
        ac_cb[b_idx] = block;
    }
    for b_idx in 0..4usize {
        let bx = b_idx % 2;
        let by = b_idx / 2;
        let gx = mb_x as usize * 2 + bx;
        let gy = mb_y as usize * 2 + by;
        let cond_a = if gx > 0 {
            cr_nonzero_counts[gy * chroma_grid_w + gx - 1] > 0
        } else {
            false
        };
        let cond_b = if gy > 0 {
            cr_nonzero_counts[(gy - 1) * chroma_grid_w + gx] > 0
        } else {
            false
        };
        let (block, nnz) = decode_cabac_residual_block(
            engine,
            contexts,
            ResidualCategory::ChromaAc,
            cond_a,
            cond_b,
        )?;
        cr_nonzero_counts[gy * chroma_grid_w + gx] = nnz as u8;
        if nnz > 0 {
            eprintln!(
                "[ChromaAC Cr mb({},{}) b={}] nnz={}: {:?}",
                mb_x,
                mb_y,
                b_idx,
                nnz,
                &block[..8]
            );
        }
        ac_cr[b_idx] = block;
    }
    eprintln!(
        "[ChromaAC mb({},{})] decoded 4 Cb and 4 Cr blocks",
        mb_x, mb_y
    );
    Ok((ac_cb, ac_cr))
}

fn scale_chroma_dc(dc_level: i32, qp_c: u8) -> i32 {
    let qp = qp_c as i32;
    let shift = qp / 6;

    let v = [10i32, 11, 13, 14, 16, 18][(qp % 6) as usize];
    let scaled = dc_level * v;
    if shift > 0 {
        (scaled + (1 << (shift - 1))) >> shift
    } else {
        scaled
    }
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_chroma_plane(
    frame: &mut Yuv420Frame,
    plane: ColorPlane,
    mb_x: u32,
    mb_y: u32,
    chroma_mode: u8,
    dc_coeffs: &[i32; 4],
    ac_blocks: &[[i32; 16]; 4],
    qp_c: u8,
    chroma_grid_w: usize,
    nonzero_counts: &mut [u8],
) -> Result<(), H264DecodeError> {
    let (frame_width, _) = frame.plane_dimensions(plane);
    let cx = mb_x * 8;
    let cy = mb_y * 8;

    let top_row: Option<[u8; 8]> = (mb_y > 0).then(|| {
        std::array::from_fn(|i| {
            let offset = ((cy - 1) * frame_width + cx + i as u32) as usize;
            match plane {
                ColorPlane::U => frame.u[offset],
                ColorPlane::V => frame.v[offset],
                ColorPlane::Y => unreachable!(),
            }
        })
    });
    let left_col: Option<[u8; 8]> = (mb_x > 0).then(|| {
        std::array::from_fn(|i| {
            let offset = ((cy + i as u32) * frame_width + cx - 1) as usize;
            match plane {
                ColorPlane::U => frame.u[offset],
                ColorPlane::V => frame.v[offset],
                ColorPlane::Y => unreachable!(),
            }
        })
    });

    let pred_dc = |tr: Option<&[u8]>, lc: Option<&[u8]>| -> u8 {
        let sum: u32 = tr
            .iter()
            .flat_map(|s| s.iter())
            .chain(lc.iter().flat_map(|s| s.iter()))
            .map(|&b| b as u32)
            .sum();
        let count = tr.map_or(0, |s| s.len()) + lc.map_or(0, |s| s.len());
        if count == 0 {
            128
        } else {
            ((sum + count as u32 / 2) / count as u32) as u8
        }
    };

    let pred: [u8; 4] = match (top_row, left_col) {
        (Some(ref t), Some(ref l)) => [
            pred_dc(Some(&t[..4]), Some(&l[..4])),
            pred_dc(Some(&t[4..]), None),
            pred_dc(None, Some(&l[4..])),
            pred_dc(Some(&t[4..]), Some(&l[4..])),
        ],
        (Some(ref t), None) => [
            pred_dc(Some(&t[..4]), None),
            pred_dc(Some(&t[4..]), None),
            pred_dc(Some(&t[..4]), None),
            pred_dc(Some(&t[4..]), None),
        ],
        (None, Some(ref l)) => [
            pred_dc(None, Some(&l[..4])),
            pred_dc(None, Some(&l[..4])),
            pred_dc(None, Some(&l[4..])),
            pred_dc(None, Some(&l[4..])),
        ],
        (None, None) => [128u8; 4],
    };

    let dc_transformed = inverse_hadamard_2x2(dc_coeffs);
    let dc_scaled: [i32; 4] = std::array::from_fn(|i| scale_chroma_dc(dc_transformed[i], qp_c));

    for b_idx in 0..4usize {
        let bx = (b_idx % 2) as u32;
        let by = (b_idx / 2) as u32;
        let blk_x = cx + bx * 4;
        let blk_y = cy + by * 4;

        let gx = mb_x as usize * 2 + bx as usize;
        let gy = mb_y as usize * 2 + by as usize;

        let mut scaled_ac = inverse_scale_4x4(&ac_blocks[b_idx], qp_c, None)?;
        scaled_ac[0] = dc_scaled[b_idx];
        let residual = inverse_transform_4x4(&scaled_ac)?;

        let dc_pred = pred[b_idx];
        let nnz = ac_blocks[b_idx].iter().filter(|&&v| v != 0).count()
            + if dc_coeffs[b_idx] != 0 { 1 } else { 0 };
        nonzero_counts[gy * chroma_grid_w + gx] = nnz as u8;

        let target = match plane {
            ColorPlane::U => &mut frame.u,
            ColorPlane::V => &mut frame.v,
            ColorPlane::Y => unreachable!(),
        };
        for row in 0..4u32 {
            for col in 0..4u32 {
                let pixel_idx = ((blk_y + row) * frame_width + blk_x + col) as usize;
                let r = residual[(row * 4 + col) as usize];
                let pred_val = if chroma_mode == 0 {
                    dc_pred as i32
                } else {
                    dc_pred as i32
                };
                target[pixel_idx] = (pred_val + r).clamp(0, 255) as u8;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_chroma_blocks(
    frame: &mut Yuv420Frame,
    mb_x: u32,
    mb_y: u32,
    chroma_mode: u8,
    dc_cb: &[i32; 4],
    dc_cr: &[i32; 4],
    ac_cb: &[[i32; 16]; 4],
    ac_cr: &[[i32; 16]; 4],
    qp_c: u8,
    chroma_grid_w: usize,
    cb_nonzero_counts: &mut [u8],
    cr_nonzero_counts: &mut [u8],
) -> Result<(), H264DecodeError> {
    reconstruct_chroma_plane(
        frame,
        ColorPlane::U,
        mb_x,
        mb_y,
        chroma_mode,
        dc_cb,
        ac_cb,
        qp_c,
        chroma_grid_w,
        cb_nonzero_counts,
    )?;
    reconstruct_chroma_plane(
        frame,
        ColorPlane::V,
        mb_x,
        mb_y,
        chroma_mode,
        dc_cr,
        ac_cr,
        qp_c,
        chroma_grid_w,
        cr_nonzero_counts,
    )
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

fn inverse_hadamard_2x2(input: &[i32; 4]) -> [i32; 4] {
    let a = input[0] + input[1] + input[2] + input[3];
    let b = input[0] - input[1] + input[2] - input[3];
    let c = input[0] + input[1] - input[2] - input[3];
    let d = input[0] - input[1] - input[2] + input[3];

    [a, b, c, d]
}

fn chroma_qp_from_luma(qp_y: i32, chroma_qp_index_offset: i32) -> u8 {
    const CHROMA_QP_TABLE: [u8; 52] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 29, 30, 31, 32, 32, 33, 34, 34, 35, 35, 36, 36, 37, 37, 37, 38, 38, 38,
        39, 39, 39, 39,
    ];
    let qp_c_prime = (qp_y + chroma_qp_index_offset).clamp(0, 51) as usize;
    CHROMA_QP_TABLE[qp_c_prime]
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
