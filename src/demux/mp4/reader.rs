use super::parser::*;
use super::types::*;
use super::{Mp4Error, Mp4Result};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};

pub struct Mp4Reader<R: Read + Seek> {
    reader: R,
    tracks: Vec<Mp4Track>,
    sample_tables: HashMap<u32, SampleTableBox>,
    _mdat_offset: Option<u64>,
}

impl<R: Read + Seek> Mp4Reader<R> {
    pub fn new(mut reader: R) -> Mp4Result<Self> {
        let mut tracks = Vec::new();
        let mut sample_tables = HashMap::new();
        let mut mdat_offset = None;

        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;
        eprintln!("[MP4Reader] File size: {} bytes", file_size);

        loop {
            let pos = reader.stream_position()?;

            let header = match parse_box_header_allow_eof(&mut reader) {
                Ok(h) => h,
                Err(Mp4Error::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    eprintln!("[MP4Reader] Reached EOF at position {}", pos);
                    break;
                }
                Err(e) => {
                    eprintln!(
                        "[MP4Reader] Error parsing box header at position {}: {}",
                        pos, e
                    );
                    return Err(e);
                }
            };

            eprintln!(
                "[MP4Reader] Found box type {:?} at pos {}, size: {}, content_size: {}",
                header.box_type,
                pos,
                header.size,
                header.size - header.header_size
            );

            let content_size = header.size - header.header_size;

            
            
            if pos + header.size > file_size && header.box_type != BoxType::MDAT {
                eprintln!(
                    "[MP4Reader] Warning: Box at {} with size {} extends beyond file (file size: {})",
                    pos, header.size, file_size
                );
                return Err(Mp4Error::InvalidFormat("Box size extends beyond file"));
            }

            
            let actual_content_size =
                if header.box_type == BoxType::MDAT && pos + header.size > file_size {
                    file_size.saturating_sub(pos + header.header_size)
                } else {
                    content_size
                };

            match header.box_type {
                BoxType::FTYP => {
                    parse_ftyp(&mut reader, actual_content_size)?;
                }
                BoxType::MOOV => {
                    let moov_start = pos;
                    Self::parse_moov(
                        &mut reader,
                        actual_content_size,
                        &mut tracks,
                        &mut sample_tables,
                    )?;
                    let after_moov_pos = reader.stream_position()?;
                    let expected_pos = moov_start + header.size;
                    eprintln!(
                        "[MP4Reader] After moov: current position = {}, expected = {}",
                        after_moov_pos, expected_pos
                    );

                    
                    if after_moov_pos != expected_pos {
                        eprintln!(
                            "[MP4Reader] Position mismatch after moov! Seeking to {}",
                            expected_pos
                        );
                        reader.seek(SeekFrom::Start(expected_pos))?;
                    }
                }
                BoxType::MDAT => {
                    mdat_offset = Some(pos + header.header_size);
                    skip_bytes(&mut reader, actual_content_size)?;
                }
                _ => {
                    skip_bytes(&mut reader, actual_content_size)?;
                }
            }
        }

        if tracks.is_empty() {
            return Err(Mp4Error::MissingBox("moov/trak"));
        }

        eprintln!("[MP4Reader] Successfully parsed {} track(s)", tracks.len());

        Ok(Self {
            reader,
            tracks,
            sample_tables,
            _mdat_offset: mdat_offset,
        })
    }

    fn parse_moov(
        reader: &mut R,
        size: u64,
        tracks: &mut Vec<Mp4Track>,
        sample_tables: &mut HashMap<u32, SampleTableBox>,
    ) -> Mp4Result<()> {
        let start_pos = reader.stream_position()?;
        let end_pos = start_pos + size;
        eprintln!(
            "[MP4Reader] Parsing moov: start={}, end={}, size={}",
            start_pos, end_pos, size
        );

        while reader.stream_position()? < end_pos {
            let pos = reader.stream_position()?;
            let header = match parse_box_header(reader) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!(
                        "[MP4Reader] Error parsing box header in moov at position {}: {}",
                        pos, e
                    );
                    return Err(e);
                }
            };
            let content_size = header.size - header.header_size;
            eprintln!(
                "[MP4Reader] moov child: {:?} at pos {}, size={}, content_size={}",
                header.box_type, pos, header.size, content_size
            );

            match header.box_type {
                BoxType::MVHD => {
                    eprintln!("[MP4Reader] Parsing mvhd...");
                    let (_timescale, _duration) = parse_mvhd(reader)?;
                    eprintln!("[MP4Reader] mvhd parsed successfully");
                }
                BoxType::TRAK => {
                    eprintln!("[MP4Reader] Parsing trak...");
                    let trak_start = reader.stream_position()?;
                    Self::parse_trak(reader, content_size, tracks, sample_tables)?;
                    let trak_end = reader.stream_position()?;
                    let bytes_read = trak_end - trak_start;
                    eprintln!(
                        "[MP4Reader] trak parsed: expected {} bytes, read {} bytes",
                        content_size, bytes_read
                    );

                    
                    if bytes_read != content_size {
                        let expected_pos = trak_start + content_size;
                        eprintln!(
                            "[MP4Reader] Position mismatch! Seeking from {} to {}",
                            trak_end, expected_pos
                        );
                        reader.seek(SeekFrom::Start(expected_pos))?;
                    }
                }
                _ => {
                    eprintln!(
                        "[MP4Reader] Skipping unknown box: {:?}, {} bytes",
                        header.box_type, content_size
                    );
                    skip_bytes(reader, content_size)?;
                }
            }
        }

        eprintln!("[MP4Reader] moov parsing complete");
        Ok(())
    }

    fn parse_trak(
        reader: &mut R,
        size: u64,
        tracks: &mut Vec<Mp4Track>,
        sample_tables: &mut HashMap<u32, SampleTableBox>,
    ) -> Mp4Result<()> {
        let start_pos = reader.stream_position()?;
        let end_pos = start_pos + size;

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

        let mut sample_table = SampleTableBox {
            chunk_offsets: Vec::new(),
            sample_sizes: Vec::new(),
            sample_to_chunk: Vec::new(),
            time_to_sample: Vec::new(),
            sync_samples: None,
        };

        while reader.stream_position()? < end_pos {
            let header = parse_box_header(reader)?;
            let content_size = header.size - header.header_size;
            let box_start = reader.stream_position()?;
            let box_end = box_start + content_size;

            match header.box_type {
                BoxType::TKHD => {
                    let (track_id, duration, width, height) = parse_tkhd(reader)?;
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

            
            let current_pos = reader.stream_position()?;
            if current_pos > box_end {
                eprintln!(
                    "[MP4Reader] WARNING: Box {:?} over-read in trak! Expected end at {}, currently at {} (over by {} bytes)",
                    header.box_type,
                    box_end,
                    current_pos,
                    current_pos - box_end
                );
                reader.seek(std::io::SeekFrom::Start(box_end))?;
            } else if current_pos < box_end {
                skip_bytes(reader, box_end - current_pos)?;
            }
        }

        if track.track_id != 0 {
            sample_tables.insert(track.track_id, sample_table);
            tracks.push(track);
        }

        Ok(())
    }

    fn parse_mdia(
        reader: &mut R,
        size: u64,
        track: &mut Mp4Track,
        sample_table: &mut SampleTableBox,
    ) -> Mp4Result<()> {
        let start_pos = reader.stream_position()?;
        let end_pos = start_pos + size;

        while reader.stream_position()? < end_pos {
            let header = parse_box_header(reader)?;
            let content_size = header.size - header.header_size;
            let box_start = reader.stream_position()?;
            let box_end = box_start + content_size;

            match header.box_type {
                BoxType::MDHD => {
                    let (timescale, duration) = parse_mdhd(reader)?;
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

            
            let current_pos = reader.stream_position()?;
            if current_pos > box_end {
                eprintln!(
                    "[MP4Reader] WARNING: Box {:?} over-read in mdia! Expected end at {}, currently at {} (over by {} bytes)",
                    header.box_type,
                    box_end,
                    current_pos,
                    current_pos - box_end
                );
                reader.seek(std::io::SeekFrom::Start(box_end))?;
            } else if current_pos < box_end {
                skip_bytes(reader, box_end - current_pos)?;
            }
        }

        Ok(())
    }

    fn parse_minf(
        reader: &mut R,
        size: u64,
        track: &mut Mp4Track,
        sample_table: &mut SampleTableBox,
    ) -> Mp4Result<()> {
        let start_pos = reader.stream_position()?;
        let end_pos = start_pos + size;

        while reader.stream_position()? < end_pos {
            let header = parse_box_header(reader)?;
            let content_size = header.size - header.header_size;
            let box_start = reader.stream_position()?;
            let box_end = box_start + content_size;

            if header.box_type == BoxType::STBL {
                Self::parse_stbl(reader, content_size, track, sample_table)?;
            } else {
                skip_bytes(reader, content_size)?;
            }

            
            let current_pos = reader.stream_position()?;
            if current_pos > box_end {
                eprintln!(
                    "[MP4Reader] WARNING: Box {:?} over-read in minf! Expected end at {}, currently at {} (over by {} bytes)",
                    header.box_type,
                    box_end,
                    current_pos,
                    current_pos - box_end
                );
                reader.seek(std::io::SeekFrom::Start(box_end))?;
            } else if current_pos < box_end {
                skip_bytes(reader, box_end - current_pos)?;
            }
        }

        Ok(())
    }

    fn parse_stbl(
        reader: &mut R,
        size: u64,
        track: &mut Mp4Track,
        sample_table: &mut SampleTableBox,
    ) -> Mp4Result<()> {
        let start_pos = reader.stream_position()?;
        let end_pos = start_pos + size;

        while reader.stream_position()? < end_pos {
            let header = parse_box_header(reader)?;
            let content_size = header.size - header.header_size;
            let box_start = reader.stream_position()?;
            let box_end = box_start + content_size;

            match header.box_type {
                BoxType::STSD => {
                    Self::parse_stsd(reader, content_size, track)?;
                }
                BoxType::STTS => {
                    sample_table.time_to_sample = parse_stts(reader)?;
                }
                BoxType::STSC => {
                    sample_table.sample_to_chunk = parse_stsc(reader)?;
                }
                BoxType::STSZ => {
                    sample_table.sample_sizes = parse_stsz(reader)?;
                }
                BoxType::STCO => {
                    sample_table.chunk_offsets = parse_stco(reader)?;
                }
                BoxType::CO64 => {
                    sample_table.chunk_offsets = parse_co64(reader)?;
                }
                BoxType::STSS => {
                    sample_table.sync_samples = Some(parse_stss(reader)?);
                }
                _ => {
                    skip_bytes(reader, content_size)?;
                }
            }

            
            let current_pos = reader.stream_position()?;
            if current_pos > box_end {
                eprintln!(
                    "[MP4Reader] WARNING: Box {:?} over-read! Expected end at {}, currently at {} (over by {} bytes)",
                    header.box_type,
                    box_end,
                    current_pos,
                    current_pos - box_end
                );
                
                reader.seek(std::io::SeekFrom::Start(box_end))?;
            } else if current_pos < box_end {
                
                skip_bytes(reader, box_end - current_pos)?;
            }
        }

        Ok(())
    }

    fn parse_stsd(reader: &mut R, _size: u64, track: &mut Mp4Track) -> Mp4Result<()> {
        skip_bytes(reader, 4)?;
        let entry_count = read_u32(reader)?;

        if entry_count == 0 {
            return Ok(());
        }

        let header = parse_box_header(reader)?;
        let content_size = header.size - header.header_size;

        match header.box_type {
            BoxType::AVC1 => {
                skip_bytes(reader, 6)?;
                skip_bytes(reader, 2)?;
                skip_bytes(reader, 16)?;

                let width = read_u16(reader)?;
                let height = read_u16(reader)?;
                track.width = Some(width as u32);
                track.height = Some(height as u32);

                skip_bytes(reader, 50)?;

                let remaining = content_size.saturating_sub(78);
                Self::parse_avc1_extensions(reader, remaining, track)?;
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

        Ok(())
    }

    fn parse_avc1_extensions(reader: &mut R, size: u64, track: &mut Mp4Track) -> Mp4Result<()> {
        let start_pos = reader.stream_position()?;
        let end_pos = start_pos + size;

        while reader.stream_position()? < end_pos {
            let header = parse_box_header(reader)?;
            let content_size = header.size - header.header_size;

            if header.box_type == BoxType::AVCC {
                let mut config = vec![0u8; content_size as usize];
                reader.read_exact(&mut config)?;
                track.codec_private_data = config;
                eprintln!("[MP4Reader] Found avcC box: {} bytes", content_size);
            } else {
                skip_bytes(reader, content_size)?;
            }
        }

        Ok(())
    }

    pub fn tracks(&self) -> &[Mp4Track] {
        &self.tracks
    }

    pub fn sample_count(&self, track_id: u32) -> Option<usize> {
        self.sample_tables
            .get(&track_id)
            .map(|st| st.sample_sizes.len())
    }

    pub fn read_sample(&mut self, track_id: u32, sample_index: u32) -> Mp4Result<Mp4Sample> {
        let sample_table = self
            .sample_tables
            .get(&track_id)
            .ok_or(Mp4Error::InvalidFormat("Invalid track ID"))?;

        let sample_index = sample_index as usize;
        if sample_index >= sample_table.sample_sizes.len() {
            return Err(Mp4Error::InvalidFormat("Sample index out of bounds"));
        }

        let size = sample_table.sample_sizes[sample_index];
        let offset = self.calculate_sample_offset(sample_table, sample_index)?;

        let is_keyframe = sample_table
            .sync_samples
            .as_ref()
            .map(|ss| ss.contains(&((sample_index + 1) as u32)))
            .unwrap_or(true);

        let timestamp = self.calculate_sample_timestamp(sample_table, sample_index)?;

        self.reader.seek(SeekFrom::Start(offset))?;
        let mut data = vec![0u8; size as usize];
        self.reader.read_exact(&mut data)?;

        Ok(Mp4Sample {
            data,
            timestamp,
            duration: 0,
            is_keyframe,
            track_id,
        })
    }

    fn calculate_sample_offset(
        &self,
        table: &SampleTableBox,
        sample_index: usize,
    ) -> Mp4Result<u64> {
        let mut current_sample = 0usize;

        for (i, stsc) in table.sample_to_chunk.iter().enumerate() {
            let next_chunk = if i + 1 < table.sample_to_chunk.len() {
                table.sample_to_chunk[i + 1].first_chunk as usize
            } else {
                table.chunk_offsets.len() + 1
            };

            let chunk_count = next_chunk - stsc.first_chunk as usize;
            let samples_in_range = chunk_count * stsc.samples_per_chunk as usize;

            if current_sample + samples_in_range > sample_index {
                let samples_before_target = sample_index - current_sample;
                let chunks_before_target = samples_before_target / stsc.samples_per_chunk as usize;
                let sample_in_chunk = samples_before_target % stsc.samples_per_chunk as usize;

                let chunk_index = (stsc.first_chunk as usize - 1) + chunks_before_target;

                let mut offset = table
                    .chunk_offsets
                    .get(chunk_index)
                    .copied()
                    .ok_or(Mp4Error::InvalidFormat("Chunk offset not found"))?;

                let first_sample_in_chunk =
                    current_sample + chunks_before_target * stsc.samples_per_chunk as usize;
                for i in first_sample_in_chunk..(first_sample_in_chunk + sample_in_chunk) {
                    offset += table.sample_sizes[i] as u64;
                }

                return Ok(offset);
            }

            current_sample += samples_in_range;
        }

        Err(Mp4Error::InvalidFormat("Could not calculate sample offset"))
    }

    fn calculate_sample_timestamp(
        &self,
        table: &SampleTableBox,
        sample_index: usize,
    ) -> Mp4Result<u64> {
        let mut timestamp = 0u64;
        let mut current_sample = 0usize;

        for entry in &table.time_to_sample {
            let samples_in_entry = entry.sample_count as usize;

            if current_sample + samples_in_entry > sample_index {
                let samples_before = sample_index - current_sample;
                timestamp += samples_before as u64 * entry.sample_delta as u64;
                return Ok(timestamp);
            }

            timestamp += samples_in_entry as u64 * entry.sample_delta as u64;
            current_sample += samples_in_entry;
        }

        Ok(timestamp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_reader_from_empty_fails() {
        use std::io::Cursor;
        let data = vec![];
        let cursor = Cursor::new(data);
        assert!(Mp4Reader::new(cursor).is_err());
    }
}
