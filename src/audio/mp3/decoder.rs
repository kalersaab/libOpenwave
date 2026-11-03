use crate::audio::mp3::bitstream::Mp3BitReader;
use crate::audio::mp3::tables::{LFS_INTENSITY_STEREO_TABLE, LFS_TABLE, SCALE_FACTOR_SIZES};
use crate::audio::mp3::types::*;
use crate::demux::mp3::{ChannelMode, Layer, Mp3FrameHeader, MpegVersion as DemuxVersion};
use std::io;

pub struct Mp3Decoder {
    state: DecoderState,
}

impl Mp3Decoder {
    pub fn new() -> Self {
        Self {
            state: DecoderState::new(),
        }
    }

    pub fn decode_frame(&mut self, header: &Mp3FrameHeader, data: &[u8]) -> io::Result<Vec<f32>> {
        let header = frame_header(header);
        let expected = header.num_granules() * 576 * header.channels.num_channels();

        let side_info = {
            let mut reader = Mp3BitReader::new(data);
            if header.crc {
                reader.skip_bits(16)?;
            }
            read_side_info(&mut reader, &header)?
        };

        let main_data_start = (if header.crc { 2 } else { 0 }) + header.side_data_len();
        let frame_main_data = data.get(main_data_start..).unwrap_or(&[]);

        let main_data_begin = side_info.main_data_begin as usize;
        let bytes_have = main_data_begin.min(self.state.frame_buffer_len);
        let prev_start = self.state.frame_buffer_len - bytes_have;
        self.state
            .frame_buffer
            .copy_within(prev_start..prev_start + bytes_have, 0);
        let end = bytes_have + frame_main_data.len();
        self.state.frame_buffer[bytes_have..end].copy_from_slice(frame_main_data);
        self.state.frame_buffer_len = end;

        if main_data_begin > bytes_have {
            return Ok(vec![0.0; expected]);
        }

        let mut main_data = {
            let mut reader = Mp3BitReader::new(&self.state.frame_buffer[..end]);
            read_main_data(&mut reader, &header, &side_info)?
        };

        let mut out_samples = [[0.0f32; 1152]; 2];
        decode_frame(
            &mut self.state,
            &header,
            &side_info,
            &mut main_data,
            &mut out_samples,
        )?;

        let num_samples = header.num_granules() * 576;
        let channels = header.channels.num_channels();
        let mut pcm = Vec::with_capacity(num_samples * channels);

        if channels == 1 {
            pcm.extend_from_slice(&out_samples[0][..num_samples]);
        } else {
            for i in 0..num_samples {
                pcm.push(out_samples[0][i]);
                pcm.push(out_samples[1][i]);
            }
        }

        Ok(pcm)
    }
}

impl Default for Mp3Decoder {
    fn default() -> Self {
        Self::new()
    }
}

fn frame_header(header: &Mp3FrameHeader) -> FrameHeader {
    let version = match header.version {
        DemuxVersion::Mpeg1 => MpegVersion::Mpeg1,
        DemuxVersion::Mpeg2 => MpegVersion::Mpeg2,
        DemuxVersion::Mpeg25 => MpegVersion::Mpeg2_5,
    };

    let sample_rate = match header.sample_rate {
        8000 => SampleRate::Hz8000,
        11025 => SampleRate::Hz11025,
        12000 => SampleRate::Hz12000,
        16000 => SampleRate::Hz16000,
        22050 => SampleRate::Hz22050,
        24000 => SampleRate::Hz24000,
        32000 => SampleRate::Hz32000,
        44100 => SampleRate::Hz44100,
        48000 => SampleRate::Hz48000,
        _ => SampleRate::Hz44100,
    };

    let sample_rate_table = match (version, header.sample_rate) {
        (MpegVersion::Mpeg1, 44100 | 48000 | 32000) => match header.sample_rate {
            44100 => 0,
            48000 => 1,
            _ => 2,
        },
        (MpegVersion::Mpeg2, 22050 | 24000 | 16000) => match header.sample_rate {
            22050 => 3,
            24000 => 4,
            _ => 5,
        },
        (MpegVersion::Mpeg2_5, 11025 | 12000 | 8000) => match header.sample_rate {
            11025 => 6,
            12000 => 7,
            _ => 8,
        },
        _ => 0,
    };

    let channels = match header.channel_mode {
        ChannelMode::Mono => Channels::Mono,
        ChannelMode::DualChannel => Channels::DualMono,
        ChannelMode::JointStereo => Channels::JointStereo {
            mid_side_stereo: header.mode_extension & 2 != 0,
            intensity_stereo: header.mode_extension & 1 != 0,
        },
        ChannelMode::Stereo => Channels::Stereo,
    };

    FrameHeader {
        version,
        layer: match header.layer {
            Layer::LayerIII => MpegLayer::Layer3,
            Layer::LayerII => MpegLayer::Layer2,
            Layer::LayerI => MpegLayer::Layer1,
        },
        crc: header.has_crc,
        bitrate: bitrate(header.bitrate),
        sample_rate,
        padding: header.padding,
        channels,
        copyright: false,
        original: true,
        emphasis: Emphasis::None,
        sample_rate_table,
    }
}

fn bitrate(bps: u32) -> BitRate {
    match bps {
        8_000 => BitRate::Kbps8,
        16_000 => BitRate::Kbps16,
        24_000 => BitRate::Kbps24,
        32_000 => BitRate::Kbps32,
        40_000 => BitRate::Kbps40,
        48_000 => BitRate::Kbps48,
        56_000 => BitRate::Kbps56,
        64_000 => BitRate::Kbps64,
        80_000 => BitRate::Kbps80,
        96_000 => BitRate::Kbps96,
        112_000 => BitRate::Kbps112,
        128_000 => BitRate::Kbps128,
        144_000 => BitRate::Kbps144,
        160_000 => BitRate::Kbps160,
        192_000 => BitRate::Kbps192,
        224_000 => BitRate::Kbps224,
        256_000 => BitRate::Kbps256,
        _ => BitRate::Kbps320,
    }
}

fn read_side_info(reader: &mut Mp3BitReader, header: &FrameHeader) -> io::Result<SideInfo> {
    let mut info = SideInfo::default();

    if header.version == MpegVersion::Mpeg1 {
        info.main_data_begin = reader.read_bits(9)? as u16;

        if header.channels == Channels::Mono {
            reader.skip_bits(5)?;
        } else {
            reader.skip_bits(3)?;
        }

        for scfsi in &mut info.scfsi[..header.channels.num_channels()] {
            for band in scfsi.iter_mut() {
                *band = reader.read_bit()?;
            }
        }
    } else {
        info.main_data_begin = reader.read_bits(8)? as u16;

        if header.channels == Channels::Mono {
            reader.skip_bits(1)?;
        } else {
            reader.skip_bits(2)?;
        }
    }

    for granule in &mut info.granules[..header.num_granules()] {
        *granule = read_granule_side_info(header, reader)?;
    }

    Ok(info)
}

fn read_granule_side_info(
    header: &FrameHeader,
    reader: &mut Mp3BitReader,
) -> io::Result<GranuleSideInfo> {
    let mut info = GranuleSideInfo::default();
    for channel_side_info in &mut info.channels[0..header.channels.num_channels()] {
        *channel_side_info = read_granule_channel_side_info(header, reader)?;
    }
    Ok(info)
}

fn read_granule_channel_side_info(
    header: &FrameHeader,
    reader: &mut Mp3BitReader,
) -> io::Result<GranuleChannelSideInfo> {
    let part2_3_length = reader.read_bits(12)? as u16;
    let big_values = reader.read_bits(9)? as u16;
    if big_values > 288 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "big_values > 288",
        ));
    }
    let global_gain = reader.read_bits(8)? as u8;
    let scalefac_compress_len = if header.version == MpegVersion::Mpeg1 {
        4
    } else {
        9
    };
    let scalefac_compress = reader.read_bits(scalefac_compress_len)? as u16;

    let window_switching = reader.read_bit()?;
    let (block_type, mixed_block, table_select, subblock_gain, region0_count, region1_count) =
        if window_switching {
            let block_type_id = reader.read_bits(2)? as u8;
            let mixed_block = reader.read_bit()?;
            let table_select = [reader.read_bits(5)? as u8, reader.read_bits(5)? as u8, 0];
            let subblock_gain = [
                reader.read_bits(3)? as f32,
                reader.read_bits(3)? as f32,
                reader.read_bits(3)? as f32,
            ];

            let block_type = match block_type_id {
                0b00 => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Forbidden block type",
                    ));
                }
                0b01 => BlockType::Start,
                0b10 => {
                    if mixed_block {
                        BlockType::Mixed
                    } else {
                        BlockType::Short
                    }
                }
                0b11 => BlockType::End,
                _ => unreachable!(),
            };

            let region0_count = if block_type == BlockType::Short { 8 } else { 7 };
            let region1_count = 20 - region0_count;

            (
                block_type,
                mixed_block,
                table_select,
                subblock_gain,
                region0_count,
                region1_count,
            )
        } else {
            let table_select = [
                reader.read_bits(5)? as u8,
                reader.read_bits(5)? as u8,
                reader.read_bits(5)? as u8,
            ];
            let region0_count = reader.read_bits(4)? as u8;
            let region1_count = reader.read_bits(3)? as u8;

            (
                BlockType::Long,
                false,
                table_select,
                [0.0; 3],
                region0_count,
                region1_count,
            )
        };

    let preflag = if header.version == MpegVersion::Mpeg1 {
        reader.read_bit()?
    } else {
        scalefac_compress >= 500
    };

    let scalefac_scale = reader.read_bit()?;
    let count1table_select = reader.read_bit()?;

    Ok(GranuleChannelSideInfo {
        part2_3_length,
        big_values,
        global_gain,
        scalefac_compress,
        block_type,
        mixed_block,
        subblock_gain,
        table_select,
        region0_count,
        region1_count,
        preflag,
        scalefac_scale,
        count1table_select,
    })
}

fn read_main_data(
    reader: &mut Mp3BitReader,
    header: &FrameHeader,
    side_info: &SideInfo,
) -> io::Result<MainData> {
    let mut data = MainData::default();

    for g in 0..header.num_granules() {
        for c in 0..header.channels.num_channels() {
            let bits_read = if header.version == MpegVersion::Mpeg1 {
                read_scale_factors(reader, g, c, side_info, &mut data)?
            } else {
                read_lfs_scale_factors(
                    reader,
                    c == 1 && header.is_intensity_stereo(),
                    &side_info.granules[g].channels[c],
                    &mut data.granules[g].channels[c],
                )?
            };

            let huffman_len = u32::from(side_info.granules[g].channels[c].part2_3_length)
                .saturating_sub(bits_read);
            data.granules[g].channels[c].count1 = crate::audio::mp3::huffman::read_huffman(
                reader,
                header,
                &side_info.granules[g].channels[c],
                huffman_len,
                &mut data.granules[g].channels[c].samples,
            )?;
        }
    }

    Ok(data)
}

fn read_scale_factors(
    reader: &mut Mp3BitReader,
    granule: usize,
    channel: usize,
    side_info: &SideInfo,
    main_data: &mut MainData,
) -> io::Result<u32> {
    let mut bits_read = 0;

    let block_type = side_info.granules[granule].channels[channel].block_type;
    let scalefac_compress =
        side_info.granules[granule].channels[channel].scalefac_compress as usize;
    let (scale_len1, scale_len2) = SCALE_FACTOR_SIZES[scalefac_compress];

    if block_type == BlockType::Short || block_type == BlockType::Mixed {
        let channel_info = &side_info.granules[granule].channels[channel];
        let channel_data = &mut main_data.granules[granule].channels[channel];
        if scale_len1 > 0 {
            if channel_info.block_type == BlockType::Mixed {
                for sfb in &mut channel_data.scalefac_l[..8] {
                    *sfb = reader.read_bits(scale_len1 as u8)? as u8;
                    bits_read += scale_len1;
                }
            }

            for sfb in &mut channel_data.scalefac_s[..6] {
                for window in sfb.iter_mut() {
                    *window = reader.read_bits(scale_len1 as u8)? as u8;
                    bits_read += scale_len1;
                }
            }
        }

        if scale_len2 > 0 {
            for sfb in &mut channel_data.scalefac_s[6..12] {
                for window in sfb.iter_mut() {
                    *window = reader.read_bits(scale_len2 as u8)? as u8;
                    bits_read += scale_len2;
                }
            }
        }
    } else {
        let slices = [(0usize, 6usize), (6, 11), (11, 16), (16, 21)];
        for (i, (start, end)) in slices.iter().enumerate() {
            let len = if i < 2 { scale_len1 } else { scale_len2 };
            if len > 0 {
                if granule == 0 || !side_info.scfsi[channel][i] {
                    for sfb in
                        &mut main_data.granules[granule].channels[channel].scalefac_l[*start..*end]
                    {
                        *sfb = reader.read_bits(len as u8)? as u8;
                        bits_read += len;
                    }
                } else if granule == 1 && side_info.scfsi[channel][i] {
                    let (granule0, granules) = main_data.granules.split_first_mut().unwrap();
                    granule0.channels[channel].scalefac_l[*start..*end]
                        .copy_from_slice(&granules[0].channels[channel].scalefac_l[*start..*end]);
                }
            }
        }
    }

    Ok(bits_read)
}

fn read_lfs_scale_factors(
    reader: &mut Mp3BitReader,
    intensity_stereo_channel: bool,
    channel_info: &GranuleChannelSideInfo,
    channel_data: &mut MainDataChannel,
) -> io::Result<u32> {
    let mut bits_read = 0;

    let lfs_table = if intensity_stereo_channel {
        &LFS_INTENSITY_STEREO_TABLE
    } else {
        &LFS_TABLE
    };
    let lfs_table = match channel_info.block_type {
        BlockType::Short => &lfs_table[1],
        BlockType::Mixed => &lfs_table[2],
        _ => &lfs_table[0],
    };

    let (scale_lens, lfs_table) = if intensity_stereo_channel {
        let sfc = u32::from(channel_info.scalefac_compress / 2);
        match sfc {
            0..=179 => ([sfc / 36, (sfc % 36) / 6, sfc % 6, 0], &lfs_table[0]),
            180..=243 => (
                [
                    ((sfc - 180) % 64) / 16,
                    ((sfc - 180) % 16) / 4,
                    (sfc - 180) % 4,
                    0,
                ],
                &lfs_table[1],
            ),
            244..=255 => ([(sfc - 244) / 3, (sfc - 244) % 3, 0, 0], &lfs_table[2]),
            _ => unreachable!(),
        }
    } else {
        let sfc = u32::from(channel_info.scalefac_compress);
        match sfc {
            0..=399 => (
                [sfc / 80, (sfc / 16) % 5, (sfc % 16) / 4, sfc & 3],
                &lfs_table[0],
            ),
            400..=499 => (
                [(sfc - 400) / 20, ((sfc - 400) / 4) % 5, (sfc - 400) % 4, 0],
                &lfs_table[1],
            ),
            500..=512 => ([(sfc - 500) / 3, (sfc - 500) % 3, 0, 0], &lfs_table[2]),
            _ => unreachable!(),
        }
    };

    let mut scalefacs = [0u8; 54];
    let mut i = 0;
    for (&len, &num_blocks) in scale_lens[..].iter().zip(lfs_table.iter()) {
        if len > 0 {
            for _ in 0..num_blocks {
                scalefacs[i] = reader.read_bits(len as u8)? as u8;
                bits_read += len;
                i += 1;
            }
        } else {
            i += num_blocks;
        }
    }

    i = 0;
    if channel_info.block_type == BlockType::Short || channel_info.block_type == BlockType::Mixed {
        let short_start = if channel_info.block_type == BlockType::Mixed {
            for sfb in 0..8 {
                channel_data.scalefac_l[sfb] = scalefacs[i];
                i += 1;
            }
            3
        } else {
            0
        };

        for sfb in short_start..12 {
            for window in 0..3 {
                channel_data.scalefac_s[sfb][window] = scalefacs[i];
                i += 1;
            }
        }
    } else {
        for sfb in 0..21 {
            channel_data.scalefac_l[sfb] = scalefacs[i];
            i += 1;
        }
    }

    Ok(bits_read)
}

fn decode_frame(
    decoder: &mut DecoderState,
    header: &FrameHeader,
    side_info: &SideInfo,
    main_data: &mut MainData,
    out_samples: &mut [[f32; 1152]; 2],
) -> io::Result<()> {
    use crate::audio::mp3::{requantize, stereo, synthesis};

    if header.channels == Channels::Mono {
        for gr in 0..header.num_granules() {
            let side_info = &side_info.granules[gr].channels[0];
            let main_data = &mut main_data.granules[gr].channels[0];

            requantize::requantize(header, side_info, main_data);
            requantize::reorder(header, side_info, main_data);
            synthesis::antialias(side_info, &mut main_data.samples);
            synthesis::hybrid_synthesis(
                side_info.block_type,
                &mut decoder.store[0],
                &mut main_data.samples,
            );
            synthesis::frequency_inversion(&mut main_data.samples);
            synthesis::subband_synthesis(
                &main_data.samples,
                &mut decoder.sbs_v_vec[0],
                &mut out_samples[0][gr * 576..(gr + 1) * 576],
            );
        }

        out_samples[1] = out_samples[0];
    } else {
        for gr in 0..header.num_granules() {
            for ch in 0..MAX_CHANNELS {
                let side_info = &side_info.granules[gr].channels[ch];
                let main_data = &mut main_data.granules[gr].channels[ch];

                requantize::requantize(header, side_info, main_data);
                requantize::reorder(header, side_info, main_data);
            }

            if let Channels::JointStereo {
                intensity_stereo,
                mid_side_stereo,
            } = header.channels
            {
                stereo::stereo(
                    header,
                    &side_info.granules[gr],
                    intensity_stereo,
                    mid_side_stereo,
                    &mut main_data.granules[gr],
                );
            }

            for (ch, out_channel) in out_samples.iter_mut().enumerate() {
                let side_info = &side_info.granules[gr].channels[ch];
                let main_data = &mut main_data.granules[gr].channels[ch];

                synthesis::antialias(side_info, &mut main_data.samples);
                synthesis::hybrid_synthesis(
                    side_info.block_type,
                    &mut decoder.store[ch],
                    &mut main_data.samples,
                );
                synthesis::frequency_inversion(&mut main_data.samples);
                synthesis::subband_synthesis(
                    &main_data.samples,
                    &mut decoder.sbs_v_vec[ch],
                    &mut out_channel[gr * 576..(gr + 1) * 576],
                );
            }
        }
    }
    Ok(())
}
