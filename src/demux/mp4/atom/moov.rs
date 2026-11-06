use super::{
    bounded_atom_end, finish_atom, parse_co64, parse_ctts, parse_stco, parse_stsc, parse_stss,
    parse_stsz, parse_stts,
};
use crate::demux::mp4::parser::*;
use crate::demux::mp4::types::*;
use crate::demux::mp4::{Mp4Error, Mp4Result};
use std::collections::HashMap;
use std::io::{Read, Seek};

#[derive(Debug, Default)]
pub struct MoovBox {
    pub tracks: Vec<Mp4Track>,
    pub sample_tables: HashMap<u32, SampleTableBox>,
    pub trex_defaults: HashMap<u32, TrexDefaults>,
}

impl MoovBox {
    pub fn parse<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<Self> {
        let start_pos = reader.stream_position()?;
        let end_pos = start_pos
            .checked_add(size)
            .ok_or(Mp4Error::InvalidFormat("moov box size overflow"))?;

        let mut moov = Self::default();

        while reader.stream_position()? < end_pos {
            let atom_start = reader.stream_position()?;
            let header = parse_box_header(reader)?;
            let atom_end = bounded_atom_end(&header, atom_start, end_pos)?;
            let content_size = header.content_size();

            match header.box_type {
                BoxType::MVHD => {
                    let (_timescale, _duration) = parse_mvhd(reader, content_size)?;
                }
                BoxType::TRAK => {
                    let (track, sample_table) = Self::parse_trak(reader, content_size)?;
                    if track.track_id != 0 {
                        moov.sample_tables.insert(track.track_id, sample_table);
                        moov.tracks.push(track);
                    }
                }
                BoxType::MVEX => {
                    Self::parse_mvex(reader, content_size, &mut moov.trex_defaults)?;
                }
                _ => {
                    skip_bytes(reader, content_size)?;
                }
            }

            finish_atom(reader, atom_end)?;
        }

        Ok(moov)
    }

    fn parse_mvex<R: Read + Seek>(
        reader: &mut R,
        size: u64,
        trex_defaults: &mut HashMap<u32, TrexDefaults>,
    ) -> Mp4Result<()> {
        let start_pos = reader.stream_position()?;
        let end_pos = start_pos
            .checked_add(size)
            .ok_or(Mp4Error::InvalidFormat("mvex box size overflow"))?;

        while reader.stream_position()? < end_pos {
            let atom_start = reader.stream_position()?;
            let header = parse_box_header(reader)?;
            let atom_end = bounded_atom_end(&header, atom_start, end_pos)?;
            let content_size = header.content_size();

            if header.box_type == BoxType::TREX {
                let (track_id, defaults) = Self::parse_trex(reader, content_size)?;
                if track_id != 0 {
                    trex_defaults.insert(track_id, defaults);
                }
            } else {
                skip_bytes(reader, content_size)?;
            }

            finish_atom(reader, atom_end)?;
        }

        Ok(())
    }

    fn parse_trex<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<(u32, TrexDefaults)> {
        ensure_box_size(size, 24, "trex box too small")?;
        skip_bytes(reader, 4)?;

        let track_id = read_u32(reader)?;
        let defaults = TrexDefaults {
            sample_description_index: read_u32(reader)?,
            sample_duration: read_u32(reader)?,
            sample_size: read_u32(reader)?,
            sample_flags: read_u32(reader)?,
        };

        let consumed = 24;
        if size > consumed {
            skip_bytes(reader, size - consumed)?;
        }

        Ok((track_id, defaults))
    }

    fn parse_trak<R: Read + Seek>(
        reader: &mut R,
        size: u64,
    ) -> Mp4Result<(Mp4Track, SampleTableBox)> {
        let start_pos = reader.stream_position()?;
        let end_pos = start_pos
            .checked_add(size)
            .ok_or(Mp4Error::InvalidFormat("trak box size overflow"))?;

        let mut track = Mp4Track {
            track_id: 0,
            track_type: TrackType::Unknown,
            timescale: 0,
            duration: 0,
            codec_private_data: Vec::new(),
            width: None,
            height: None,
            sample_rate: None,
            channels: None,
        };

        let mut sample_table = SampleTableBox::default();

        while reader.stream_position()? < end_pos {
            let atom_start = reader.stream_position()?;
            let header = parse_box_header(reader)?;
            let atom_end = bounded_atom_end(&header, atom_start, end_pos)?;
            let content_size = header.content_size();

            match header.box_type {
                BoxType::TKHD => {
                    let (track_id, duration, width, height) = parse_tkhd(reader, content_size)?;
                    track.track_id = track_id;
                    track.duration = duration;
                    track.width = width;
                    track.height = height;
                }
                BoxType::MDIA => {
                    Self::parse_mdia(reader, content_size, &mut track, &mut sample_table)?;
                }
                _ => {
                    skip_bytes(reader, content_size)?;
                }
            }

            finish_atom(reader, atom_end)?;
        }

        Ok((track, sample_table))
    }

    fn parse_mdia<R: Read + Seek>(
        reader: &mut R,
        size: u64,
        track: &mut Mp4Track,
        sample_table: &mut SampleTableBox,
    ) -> Mp4Result<()> {
        let start_pos = reader.stream_position()?;
        let end_pos = start_pos
            .checked_add(size)
            .ok_or(Mp4Error::InvalidFormat("mdia box size overflow"))?;

        while reader.stream_position()? < end_pos {
            let atom_start = reader.stream_position()?;
            let header = parse_box_header(reader)?;
            let atom_end = bounded_atom_end(&header, atom_start, end_pos)?;
            let content_size = header.content_size();

            match header.box_type {
                BoxType::MDHD => {
                    let (timescale, duration) = parse_mdhd(reader, content_size)?;
                    track.timescale = timescale;
                    track.duration = duration;
                }
                BoxType::HDLR => {
                    track.track_type = parse_hdlr(reader, content_size)?;
                }
                BoxType::MINF => {
                    Self::parse_minf(reader, content_size, track, sample_table)?;
                }
                _ => {
                    skip_bytes(reader, content_size)?;
                }
            }

            finish_atom(reader, atom_end)?;
        }

        Ok(())
    }

    fn parse_minf<R: Read + Seek>(
        reader: &mut R,
        size: u64,
        track: &mut Mp4Track,
        sample_table: &mut SampleTableBox,
    ) -> Mp4Result<()> {
        let start_pos = reader.stream_position()?;
        let end_pos = start_pos
            .checked_add(size)
            .ok_or(Mp4Error::InvalidFormat("minf box size overflow"))?;

        while reader.stream_position()? < end_pos {
            let atom_start = reader.stream_position()?;
            let header = parse_box_header(reader)?;
            let atom_end = bounded_atom_end(&header, atom_start, end_pos)?;
            let content_size = header.content_size();

            if header.box_type == BoxType::STBL {
                Self::parse_stbl(reader, content_size, track, sample_table)?;
            } else {
                skip_bytes(reader, content_size)?;
            }

            finish_atom(reader, atom_end)?;
        }

        Ok(())
    }

    fn parse_stbl<R: Read + Seek>(
        reader: &mut R,
        size: u64,
        track: &mut Mp4Track,
        sample_table: &mut SampleTableBox,
    ) -> Mp4Result<()> {
        let start_pos = reader.stream_position()?;
        let end_pos = start_pos
            .checked_add(size)
            .ok_or(Mp4Error::InvalidFormat("stbl box size overflow"))?;

        while reader.stream_position()? < end_pos {
            let atom_start = reader.stream_position()?;
            let header = parse_box_header(reader)?;
            let atom_end = bounded_atom_end(&header, atom_start, end_pos)?;
            let content_size = header.content_size();

            match header.box_type {
                BoxType::STSD => {
                    Self::parse_stsd(reader, content_size, track)?;
                }
                BoxType::STTS => {
                    sample_table.time_to_sample = parse_stts(reader, content_size)?;
                }
                BoxType::CTTS => {
                    sample_table.composition_offsets = Some(parse_ctts(reader, content_size)?);
                }
                BoxType::STSC => {
                    sample_table.sample_to_chunk = parse_stsc(reader, content_size)?;
                }
                BoxType::STSZ => {
                    sample_table.sample_sizes = parse_stsz(reader, content_size)?;
                }
                BoxType::STCO => {
                    sample_table.chunk_offsets = parse_stco(reader, content_size)?;
                }
                BoxType::CO64 => {
                    sample_table.chunk_offsets = parse_co64(reader, content_size)?;
                }
                BoxType::STSS => {
                    sample_table.sync_samples = Some(parse_stss(reader, content_size)?);
                }
                _ => {
                    skip_bytes(reader, content_size)?;
                }
            }

            finish_atom(reader, atom_end)?;
        }

        Ok(())
    }

    fn parse_stsd<R: Read + Seek>(
        reader: &mut R,
        size: u64,
        track: &mut Mp4Track,
    ) -> Mp4Result<()> {
        if size < 8 {
            return Err(Mp4Error::InvalidFormat("stsd box too small"));
        }

        let start_pos = reader.stream_position()?;
        let end_pos = start_pos
            .checked_add(size)
            .ok_or(Mp4Error::InvalidFormat("stsd box size overflow"))?;

        skip_bytes(reader, 4)?;
        let mut entry_count = read_u32(reader)?;

        while entry_count > 0 && reader.stream_position()? < end_pos {
            let entry_start = reader.stream_position()?;
            let header = parse_box_header(reader)?;
            let entry_end = bounded_atom_end(&header, entry_start, end_pos)?;
            let content_size = header.content_size();

            match header.box_type {
                BoxType::AVC1 => {
                    if content_size < 78 {
                        return Err(Mp4Error::InvalidFormat("avc1 box too small"));
                    }

                    skip_bytes(reader, 6)?;
                    skip_bytes(reader, 2)?;
                    skip_bytes(reader, 16)?;

                    let width = read_u16(reader)?;
                    let height = read_u16(reader)?;
                    track.width = Some(width as u32);
                    track.height = Some(height as u32);

                    skip_bytes(reader, 50)?;

                    Self::parse_avc1_extensions(reader, content_size - 78, track)?;
                }
                BoxType::MP4A => {
                    let (sample_rate, channels, audio_config) = parse_mp4a(reader, content_size)?;
                    track.sample_rate = Some(sample_rate);
                    track.channels = Some(channels);
                    track.codec_private_data = audio_config;
                }
                _ => {
                    skip_bytes(reader, content_size)?;
                }
            }

            finish_atom(reader, entry_end)?;
            entry_count -= 1;
        }

        if entry_count > 0 {
            return Err(Mp4Error::InvalidFormat("stsd entry count exceeds box size"));
        }

        Ok(())
    }

    fn parse_avc1_extensions<R: Read + Seek>(
        reader: &mut R,
        size: u64,
        track: &mut Mp4Track,
    ) -> Mp4Result<()> {
        let start_pos = reader.stream_position()?;
        let end_pos = start_pos
            .checked_add(size)
            .ok_or(Mp4Error::InvalidFormat("avc1 extensions size overflow"))?;

        while reader.stream_position()? < end_pos {
            let atom_start = reader.stream_position()?;
            let header = parse_box_header(reader)?;
            let atom_end = bounded_atom_end(&header, atom_start, end_pos)?;
            let content_size = header.content_size();

            if header.box_type == BoxType::AVCC {
                let mut config = vec![0u8; content_size as usize];
                reader.read_exact(&mut config)?;
                track.codec_private_data = config;
            } else {
                skip_bytes(reader, content_size)?;
            }

            finish_atom(reader, atom_end)?;
        }

        Ok(())
    }
}
