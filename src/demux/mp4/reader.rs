use super::atom::{MdatBox, MoofBox, MoovBox, finish_atom, parse_ftyp};
use super::parser::*;
use super::sample_table::{self, SampleTiming};
use super::types::*;
use super::{Mp4Error, Mp4Result};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};

pub struct Mp4Reader<R: Read + Seek> {
    reader: R,
    file_size: u64,
    tracks: Vec<Mp4Track>,
    sample_tables: HashMap<u32, SampleTableBox>,
    file_type: Option<FileType>,
    mdat_offset: Option<u64>,
    mdat_size: Option<u64>,
}

impl<R: Read + Seek> Mp4Reader<R> {
    pub fn new(mut reader: R) -> Mp4Result<Self> {
        let mut tracks = Vec::new();
        let mut sample_tables = HashMap::new();
        let mut file_type = None;
        let mut mdat_offset = None;
        let mut mdat_size = None;
        let mut trex_defaults = HashMap::new();
        let mut fragment_positions = Vec::new();

        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        loop {
            let pos = reader.stream_position()?;
            if pos == file_size {
                break;
            }
            if pos > file_size {
                return Err(Mp4Error::InvalidFormat("Box starts beyond end of file"));
            }

            let header = match parse_box_header_allow_eof(&mut reader) {
                Ok(h) => h,
                Err(Mp4Error::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    break;
                }
                Err(e) => {
                    return Err(e);
                }
            };

            let content_start = pos
                .checked_add(header.header_size)
                .ok_or(Mp4Error::InvalidFormat("Box header size overflow"))?;
            let box_end = pos
                .checked_add(header.size)
                .ok_or(Mp4Error::InvalidFormat("Box size overflow"))?;

            if content_start > file_size {
                return Err(Mp4Error::InvalidFormat("Box starts beyond end of file"));
            }

            if box_end > file_size && header.box_type != BoxType::MDAT {
                return Err(Mp4Error::InvalidFormat("Box size extends beyond file"));
            }

            let actual_content_size = if box_end > file_size {
                file_size - content_start
            } else {
                header.content_size()
            };
            let actual_box_end = content_start + actual_content_size;

            match header.box_type {
                BoxType::FTYP => {
                    file_type = Some(parse_ftyp(&mut reader, actual_content_size)?);
                }
                BoxType::MOOV => {
                    let moov = MoovBox::parse(&mut reader, actual_content_size)?;
                    tracks.extend(moov.tracks);
                    sample_tables.extend(moov.sample_tables);
                    trex_defaults.extend(moov.trex_defaults);
                    finish_atom(&mut reader, actual_box_end)?;
                }
                BoxType::MDAT => {
                    if mdat_offset.is_none() {
                        let mdat = MdatBox::new(content_start, actual_content_size);
                        mdat_offset = Some(mdat.offset);
                        mdat_size = Some(mdat.size);
                    }
                    reader.seek(SeekFrom::Start(actual_box_end))?;
                }
                BoxType::MOOF => {
                    fragment_positions.push((pos, content_start, actual_content_size));
                    reader.seek(SeekFrom::Start(actual_box_end))?;
                }
                _ => {
                    reader.seek(SeekFrom::Start(actual_box_end))?;
                }
            }
        }

        if tracks.is_empty() {
            return Err(Mp4Error::MissingBox("moov/trak"));
        }

        for (moof_start, content_start, content_size) in fragment_positions {
            reader.seek(SeekFrom::Start(content_start))?;
            let moof = MoofBox::parse(&mut reader, content_size)?;
            moof.append_samples(moof_start, &trex_defaults, &mut sample_tables)?;
        }

        Ok(Self {
            reader,
            file_size,
            tracks,
            sample_tables,
            file_type,
            mdat_offset,
            mdat_size,
        })
    }

    /// File type metadata from the `ftyp` box, if present.
    pub fn file_type(&self) -> Option<&FileType> {
        self.file_type.as_ref()
    }

    /// Byte range `(offset, size)` of the first `mdat` payload.
    pub fn mdat_range(&self) -> Option<(u64, u64)> {
        self.mdat_offset.zip(self.mdat_size)
    }

    pub fn tracks(&self) -> &[Mp4Track] {
        &self.tracks
    }

    pub fn sample_count(&self, track_id: u32) -> Option<usize> {
        self.sample_tables
            .get(&track_id)
            .map(|st| st.sample_sizes.len())
    }

    /// Timing of a sample in track timescale units.
    pub fn sample_timing(&self, track_id: u32, sample_index: u32) -> Mp4Result<SampleTiming> {
        let sample_table = self
            .sample_tables
            .get(&track_id)
            .ok_or(Mp4Error::InvalidFormat("Invalid track ID"))?;

        let sample_index = sample_index as usize;
        if sample_index >= sample_table.sample_sizes.len() {
            return Err(Mp4Error::InvalidFormat("Sample index out of bounds"));
        }

        sample_table::sample_timing(sample_table, sample_index)
    }

    pub fn read_sample(&mut self, track_id: u32, sample_index: u32) -> Mp4Result<Mp4Sample> {
        let sample_index = sample_index as usize;

        let (offset, size, timing, is_keyframe) = {
            let sample_table = self
                .sample_tables
                .get(&track_id)
                .ok_or(Mp4Error::InvalidFormat("Invalid track ID"))?;

            if sample_index >= sample_table.sample_sizes.len() {
                return Err(Mp4Error::InvalidFormat("Sample index out of bounds"));
            }

            (
                sample_table::sample_offset(sample_table, sample_index)?,
                sample_table.sample_sizes[sample_index],
                sample_table::sample_timing(sample_table, sample_index)?,
                sample_table::is_sync_sample(sample_table, sample_index),
            )
        };

        let data = self.read_bytes(offset, size)?;

        Ok(Mp4Sample {
            data,
            timestamp: timing.dts,
            dts: timing.dts,
            pts: timing.pts,
            duration: timing.duration,
            is_keyframe,
            track_id,
        })
    }

    /// Iterate over every sample of every track, interleaved in decode
    /// timestamp (DTS) order.
    ///
    /// Tracks use independent timescales, so ordering compares
    /// `dts / timescale` exactly with cross multiplication. Ties are broken by
    /// track ID to keep the output deterministic.
    pub fn packets(&mut self) -> Mp4PacketIter<'_, R> {
        let cursors = self
            .tracks
            .iter()
            .filter_map(|track| {
                let sample_table = self.sample_tables.get(&track.track_id)?;
                if sample_table.sample_sizes.is_empty() {
                    return None;
                }
                Some(TrackCursor::new(
                    track.track_id,
                    track.timescale,
                    sample_table,
                ))
            })
            .collect();

        Mp4PacketIter {
            reader: self,
            cursors,
        }
    }

    fn read_bytes(&mut self, offset: u64, size: u32) -> Mp4Result<Vec<u8>> {
        let sample_end = offset
            .checked_add(size as u64)
            .ok_or(Mp4Error::InvalidFormat("Sample byte range overflow"))?;
        if sample_end > self.file_size {
            return Err(Mp4Error::InvalidFormat("Sample extends beyond file"));
        }

        self.reader.seek(SeekFrom::Start(offset))?;
        let mut data = vec![0u8; size as usize];
        self.reader.read_exact(&mut data)?;
        Ok(data)
    }
}

struct TrackCursor {
    track_id: u32,
    timescale: u32,
    sample_count: usize,
    next_index: usize,
    dts: u64,
    stts_entries: Vec<TimeToSample>,
    stts_entry: usize,
    stts_remaining: u32,
    stts_delta: u64,
    ctts_entries: Vec<CompositionOffset>,
    ctts_entry: usize,
    ctts_remaining: u32,
    ctts_offset: i64,
}

impl TrackCursor {
    fn new(track_id: u32, timescale: u32, table: &SampleTableBox) -> Self {
        let mut cursor = Self {
            track_id,
            timescale,
            sample_count: table.sample_sizes.len(),
            next_index: 0,
            dts: 0,
            stts_entries: table.time_to_sample.clone(),
            stts_entry: 0,
            stts_remaining: 0,
            stts_delta: 0,
            ctts_entries: table.composition_offsets.clone().unwrap_or_default(),
            ctts_entry: 0,
            ctts_remaining: 0,
            ctts_offset: 0,
        };

        cursor.load_stts_entry();
        cursor.load_ctts_entry();
        cursor
    }

    fn load_stts_entry(&mut self) {
        while let Some(entry) = self.stts_entries.get(self.stts_entry) {
            if entry.sample_count > 0 {
                self.stts_remaining = entry.sample_count;
                self.stts_delta = entry.sample_delta as u64;
                return;
            }
            self.stts_entry += 1;
        }

        self.stts_remaining = 0;
        self.stts_delta = 0;
    }

    fn load_ctts_entry(&mut self) {
        while let Some(entry) = self.ctts_entries.get(self.ctts_entry) {
            if entry.sample_count > 0 {
                self.ctts_remaining = entry.sample_count;
                self.ctts_offset = entry.sample_offset;
                return;
            }
            self.ctts_entry += 1;
        }

        self.ctts_remaining = 0;
        self.ctts_offset = 0;
    }

    fn is_finished(&self) -> bool {
        self.next_index >= self.sample_count
    }

    fn has_timing(&self) -> bool {
        self.stts_remaining > 0
    }

    fn pts(&self) -> Mp4Result<i64> {
        i64::try_from(self.dts as i128 + self.ctts_offset as i128)
            .map_err(|_| Mp4Error::InvalidFormat("presentation timestamp overflow"))
    }

    fn advance(&mut self) -> Mp4Result<()> {
        self.dts = self
            .dts
            .checked_add(self.stts_delta)
            .ok_or(Mp4Error::InvalidFormat("stts timestamp overflow"))?;
        self.next_index += 1;

        self.stts_remaining -= 1;
        if self.stts_remaining == 0 {
            if self.stts_entry < self.stts_entries.len() {
                self.stts_entry += 1;
            }
            self.load_stts_entry();
        }

        if self.ctts_remaining > 0 {
            self.ctts_remaining -= 1;
        }
        if self.ctts_remaining == 0 {
            if self.ctts_entry < self.ctts_entries.len() {
                self.ctts_entry += 1;
            }
            self.load_ctts_entry();
        }

        Ok(())
    }
}

/// Iterator returned by [`Mp4Reader::packets`].
///
/// Samples from all tracks are yielded in decode order (DTS), not sample-table
/// order. Each item carries its own track ID so callers can route it to the
/// appropriate decoder.
pub struct Mp4PacketIter<'a, R: Read + Seek> {
    reader: &'a mut Mp4Reader<R>,
    cursors: Vec<TrackCursor>,
}

impl<R: Read + Seek> Iterator for Mp4PacketIter<'_, R> {
    type Item = Mp4Result<Mp4Sample>;

    fn next(&mut self) -> Option<Self::Item> {
        let cursor_index = self.next_cursor_index()?;

        let cursor = &self.cursors[cursor_index];
        let sample_count = cursor.sample_count;
        let has_timing = cursor.has_timing();
        let pts = cursor.pts();
        let track_id = cursor.track_id;
        let sample_index = cursor.next_index;
        let dts = cursor.dts;
        let duration = cursor.stts_delta;

        if !has_timing {
            self.cursors[cursor_index].next_index = sample_count;
            return Some(Err(Mp4Error::InvalidFormat("stts does not cover sample")));
        }

        let pts = match pts {
            Ok(pts) => pts,
            Err(e) => {
                self.cursors[cursor_index].next_index = sample_count;
                return Some(Err(e));
            }
        };

        let (offset, size, is_keyframe) = {
            let sample_table = match self.reader.sample_tables.get(&track_id) {
                Some(table) => table,
                None => {
                    self.cursors[cursor_index].next_index = sample_count;
                    return Some(Err(Mp4Error::InvalidFormat("Invalid track ID")));
                }
            };

            let offset = match sample_table::sample_offset(sample_table, sample_index) {
                Ok(offset) => offset,
                Err(e) => {
                    self.cursors[cursor_index].next_index = sample_count;
                    return Some(Err(e));
                }
            };

            let size = match sample_table.sample_sizes.get(sample_index) {
                Some(&size) => size,
                None => {
                    self.cursors[cursor_index].next_index = sample_count;
                    return Some(Err(Mp4Error::InvalidFormat("Sample index out of bounds")));
                }
            };

            (
                offset,
                size,
                sample_table::is_sync_sample(sample_table, sample_index),
            )
        };

        let data = match self.reader.read_bytes(offset, size) {
            Ok(data) => data,
            Err(e) => {
                self.cursors[cursor_index].next_index = sample_count;
                return Some(Err(e));
            }
        };

        if let Err(e) = self.cursors[cursor_index].advance() {
            self.cursors[cursor_index].next_index = sample_count;
            return Some(Err(e));
        }

        Some(Ok(Mp4Sample {
            data,
            timestamp: dts,
            dts,
            pts,
            duration,
            is_keyframe,
            track_id,
        }))
    }
}

impl<R: Read + Seek> Mp4PacketIter<'_, R> {
    fn next_cursor_index(&self) -> Option<usize> {
        let mut best: Option<usize> = None;

        for (i, cursor) in self.cursors.iter().enumerate() {
            if cursor.is_finished() {
                continue;
            }

            match best {
                None => best = Some(i),
                Some(best_index) => {
                    if cursor_is_before(cursor, &self.cursors[best_index]) {
                        best = Some(i);
                    }
                }
            }
        }

        best
    }
}

/// Compare two tracks by decode time in seconds, using exact integer math.
fn cursor_is_before(a: &TrackCursor, b: &TrackCursor) -> bool {
    let a_timescale = u128::from(a.timescale.max(1));
    let b_timescale = u128::from(b.timescale.max(1));
    let left = u128::from(a.dts) * b_timescale;
    let right = u128::from(b.dts) * a_timescale;

    left < right || (left == right && a.track_id < b.track_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn make_box(box_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = 8 + payload.len() as u32;
        let mut out = Vec::with_capacity(size as usize);
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(box_type);
        out.extend_from_slice(payload);
        out
    }

    fn make_minimal_moov(chunk_offset: u32) -> Vec<u8> {
        let mut mvhd_payload = vec![0u8; 100];
        mvhd_payload[12..16].copy_from_slice(&1000u32.to_be_bytes());
        mvhd_payload[16..20].copy_from_slice(&100u32.to_be_bytes());
        let mvhd = make_box(b"mvhd", &mvhd_payload);

        let mut tkhd_payload = vec![0u8; 84];
        tkhd_payload[12..16].copy_from_slice(&1u32.to_be_bytes());
        tkhd_payload[20..24].copy_from_slice(&100u32.to_be_bytes());
        tkhd_payload[76..80].copy_from_slice(&(320u32 << 16).to_be_bytes());
        tkhd_payload[80..84].copy_from_slice(&(240u32 << 16).to_be_bytes());
        let tkhd = make_box(b"tkhd", &tkhd_payload);

        let mut mdhd_payload = vec![0u8; 24];
        mdhd_payload[12..16].copy_from_slice(&1000u32.to_be_bytes());
        mdhd_payload[16..20].copy_from_slice(&100u32.to_be_bytes());
        let mdhd = make_box(b"mdhd", &mdhd_payload);

        let mut hdlr_payload = vec![0u8; 12];
        hdlr_payload[8..12].copy_from_slice(b"vide");
        let hdlr = make_box(b"hdlr", &hdlr_payload);

        let mut avc1_payload = vec![0u8; 78];
        avc1_payload[24..26].copy_from_slice(&320u16.to_be_bytes());
        avc1_payload[26..28].copy_from_slice(&240u16.to_be_bytes());
        avc1_payload.extend(make_box(b"avcC", &[1, 2, 3]));
        let avc1 = make_box(b"avc1", &avc1_payload);

        let mut stsd_payload = vec![0u8; 8];
        stsd_payload[4..8].copy_from_slice(&1u32.to_be_bytes());
        stsd_payload.extend(avc1);
        let stsd = make_box(b"stsd", &stsd_payload);

        let mut stts_payload = vec![0u8; 16];
        stts_payload[4..8].copy_from_slice(&1u32.to_be_bytes());
        stts_payload[8..12].copy_from_slice(&1u32.to_be_bytes());
        stts_payload[12..16].copy_from_slice(&1u32.to_be_bytes());
        let stts = make_box(b"stts", &stts_payload);

        let mut stsc_payload = vec![0u8; 20];
        stsc_payload[4..8].copy_from_slice(&1u32.to_be_bytes());
        stsc_payload[8..12].copy_from_slice(&1u32.to_be_bytes());
        stsc_payload[12..16].copy_from_slice(&1u32.to_be_bytes());
        stsc_payload[16..20].copy_from_slice(&1u32.to_be_bytes());
        let stsc = make_box(b"stsc", &stsc_payload);

        let mut stsz_payload = vec![0u8; 16];
        stsz_payload[8..12].copy_from_slice(&1u32.to_be_bytes());
        stsz_payload[12..16].copy_from_slice(&4u32.to_be_bytes());
        let stsz = make_box(b"stsz", &stsz_payload);

        let mut stco_payload = vec![0u8; 12];
        stco_payload[4..8].copy_from_slice(&1u32.to_be_bytes());
        stco_payload[8..12].copy_from_slice(&chunk_offset.to_be_bytes());
        let stco = make_box(b"stco", &stco_payload);

        let mut stss_payload = vec![0u8; 12];
        stss_payload[4..8].copy_from_slice(&1u32.to_be_bytes());
        stss_payload[8..12].copy_from_slice(&1u32.to_be_bytes());
        let stss = make_box(b"stss", &stss_payload);

        let mut stbl_payload = Vec::new();
        stbl_payload.extend(stsd);
        stbl_payload.extend(stts);
        stbl_payload.extend(stsc);
        stbl_payload.extend(stsz);
        stbl_payload.extend(stco);
        stbl_payload.extend(stss);
        let stbl = make_box(b"stbl", &stbl_payload);
        let minf = make_box(b"minf", &stbl);

        let mut mdia_payload = Vec::new();
        mdia_payload.extend(mdhd);
        mdia_payload.extend(hdlr);
        mdia_payload.extend(minf);
        let mdia = make_box(b"mdia", &mdia_payload);

        let mut trak_payload = Vec::new();
        trak_payload.extend(tkhd);
        trak_payload.extend(mdia);
        let trak = make_box(b"trak", &trak_payload);

        let mut moov_payload = Vec::new();
        moov_payload.extend(mvhd);
        moov_payload.extend(trak);
        make_box(b"moov", &moov_payload)
    }

    fn build_minimal_mp4() -> Vec<u8> {
        let ftyp = make_box(
            b"ftyp",
            &[
                b'i', b's', b'o', b'm', 0, 0, 2, 0, b'i', b's', b'o', b'm', b'a', b'v', b'c', b'1',
            ],
        );

        let moov_placeholder = make_minimal_moov(0);
        let mdat_content_offset = ftyp.len() + moov_placeholder.len() + 8;
        let moov = make_minimal_moov(mdat_content_offset as u32);
        assert_eq!(moov.len(), moov_placeholder.len());

        let mut file = ftyp;
        file.extend(moov);
        file.extend(make_box(b"mdat", &[1, 2, 3, 4]));
        file
    }

    fn make_mvhd() -> Vec<u8> {
        let mut mvhd_payload = vec![0u8; 100];
        mvhd_payload[12..16].copy_from_slice(&1000u32.to_be_bytes());
        mvhd_payload[16..20].copy_from_slice(&100u32.to_be_bytes());
        make_box(b"mvhd", &mvhd_payload)
    }

    fn make_trak(
        track_id: u32,
        timescale: u32,
        handler: &[u8; 4],
        stts_entries: &[(u32, u32)],
        ctts_entries: Option<&[(u32, i64)]>,
        chunk_offset: u32,
        sample_sizes: &[u32],
    ) -> Vec<u8> {
        let mut tkhd_payload = vec![0u8; 84];
        tkhd_payload[12..16].copy_from_slice(&track_id.to_be_bytes());
        tkhd_payload[20..24].copy_from_slice(&100u32.to_be_bytes());
        let tkhd = make_box(b"tkhd", &tkhd_payload);

        let mut mdhd_payload = vec![0u8; 24];
        mdhd_payload[12..16].copy_from_slice(&timescale.to_be_bytes());
        mdhd_payload[16..20].copy_from_slice(&100u32.to_be_bytes());
        let mdhd = make_box(b"mdhd", &mdhd_payload);

        let mut hdlr_payload = vec![0u8; 12];
        hdlr_payload[8..12].copy_from_slice(handler);
        let hdlr = make_box(b"hdlr", &hdlr_payload);

        let mut avc1_payload = vec![0u8; 78];
        avc1_payload[24..26].copy_from_slice(&16u16.to_be_bytes());
        avc1_payload[26..28].copy_from_slice(&16u16.to_be_bytes());
        avc1_payload.extend(make_box(b"avcC", &[1, 2, 3]));
        let avc1 = make_box(b"avc1", &avc1_payload);

        let mut stsd_payload = vec![0u8; 8];
        stsd_payload[4..8].copy_from_slice(&1u32.to_be_bytes());
        stsd_payload.extend(avc1);
        let stsd = make_box(b"stsd", &stsd_payload);

        let mut stts_payload = Vec::new();
        stts_payload.extend_from_slice(&0u32.to_be_bytes());
        stts_payload.extend_from_slice(&(stts_entries.len() as u32).to_be_bytes());
        for &(sample_count, sample_delta) in stts_entries {
            stts_payload.extend_from_slice(&sample_count.to_be_bytes());
            stts_payload.extend_from_slice(&sample_delta.to_be_bytes());
        }
        let stts = make_box(b"stts", &stts_payload);

        let mut stsc_payload = Vec::new();
        stsc_payload.extend_from_slice(&0u32.to_be_bytes());
        stsc_payload.extend_from_slice(&1u32.to_be_bytes());
        stsc_payload.extend_from_slice(&1u32.to_be_bytes());
        stsc_payload.extend_from_slice(&(sample_sizes.len() as u32).to_be_bytes());
        stsc_payload.extend_from_slice(&1u32.to_be_bytes());
        let stsc = make_box(b"stsc", &stsc_payload);

        let mut stsz_payload = Vec::new();
        stsz_payload.extend_from_slice(&0u32.to_be_bytes());
        stsz_payload.extend_from_slice(&0u32.to_be_bytes());
        stsz_payload.extend_from_slice(&(sample_sizes.len() as u32).to_be_bytes());
        for &size in sample_sizes {
            stsz_payload.extend_from_slice(&size.to_be_bytes());
        }
        let stsz = make_box(b"stsz", &stsz_payload);

        let mut stco_payload = Vec::new();
        stco_payload.extend_from_slice(&0u32.to_be_bytes());
        stco_payload.extend_from_slice(&1u32.to_be_bytes());
        stco_payload.extend_from_slice(&chunk_offset.to_be_bytes());
        let stco = make_box(b"stco", &stco_payload);

        let mut stss_payload = Vec::new();
        stss_payload.extend_from_slice(&0u32.to_be_bytes());
        stss_payload.extend_from_slice(&(sample_sizes.len() as u32).to_be_bytes());
        for sample_number in 1..=sample_sizes.len() as u32 {
            stss_payload.extend_from_slice(&sample_number.to_be_bytes());
        }
        let stss = make_box(b"stss", &stss_payload);

        let mut stbl_payload = Vec::new();
        stbl_payload.extend(stsd);
        stbl_payload.extend(stts);
        if let Some(entries) = ctts_entries {
            let version = if entries.iter().any(|&(_, offset)| offset < 0) {
                1u8
            } else {
                0u8
            };

            let mut ctts_payload = Vec::new();
            ctts_payload.push(version);
            ctts_payload.extend_from_slice(&[0, 0, 0]);
            ctts_payload.extend_from_slice(&(entries.len() as u32).to_be_bytes());

            for &(sample_count, offset) in entries {
                ctts_payload.extend_from_slice(&sample_count.to_be_bytes());
                if version == 1 {
                    ctts_payload.extend_from_slice(&(offset as i32).to_be_bytes());
                } else {
                    ctts_payload.extend_from_slice(&(offset as u32).to_be_bytes());
                }
            }

            stbl_payload.extend(make_box(b"ctts", &ctts_payload));
        }
        stbl_payload.extend(stsc);
        stbl_payload.extend(stsz);
        stbl_payload.extend(stco);
        stbl_payload.extend(stss);
        let stbl = make_box(b"stbl", &stbl_payload);
        let minf = make_box(b"minf", &stbl);

        let mut mdia_payload = Vec::new();
        mdia_payload.extend(mdhd);
        mdia_payload.extend(hdlr);
        mdia_payload.extend(minf);
        let mdia = make_box(b"mdia", &mdia_payload);

        let mut trak_payload = Vec::new();
        trak_payload.extend(tkhd);
        trak_payload.extend(mdia);
        make_box(b"trak", &trak_payload)
    }

    fn make_moov(video_offset: u32, audio_offset: u32) -> Vec<u8> {
        let mut moov_payload = Vec::new();
        moov_payload.extend(make_mvhd());
        moov_payload.extend(make_trak(
            1,
            1000,
            b"vide",
            &[(3, 40)],
            Some(&[(3, -20)]),
            video_offset,
            &[1, 1, 1],
        ));
        moov_payload.extend(make_trak(
            2,
            48000,
            b"soun",
            &[(3, 960)],
            None,
            audio_offset,
            &[1, 1, 1],
        ));
        make_box(b"moov", &moov_payload)
    }

    fn build_interleaved_mp4() -> Vec<u8> {
        let ftyp = make_box(
            b"ftyp",
            &[
                b'i', b's', b'o', b'm', 0, 0, 2, 0, b'i', b's', b'o', b'm', b'a', b'v', b'c', b'1',
            ],
        );

        let placeholder = make_moov(0, 0);
        let mdat_start = ftyp.len() + placeholder.len() + 8;
        let moov = make_moov(mdat_start as u32, (mdat_start + 3) as u32);
        assert_eq!(moov.len(), placeholder.len());

        let mut file = ftyp;
        file.extend(moov);
        file.extend(make_box(b"mdat", &[1, 2, 3, 4, 5, 6]));
        file
    }

    fn make_fragmented_trak(track_id: u32, timescale: u32) -> Vec<u8> {
        let mut tkhd_payload = vec![0u8; 84];
        tkhd_payload[12..16].copy_from_slice(&track_id.to_be_bytes());
        tkhd_payload[20..24].copy_from_slice(&100u32.to_be_bytes());
        let tkhd = make_box(b"tkhd", &tkhd_payload);

        let mut mdhd_payload = vec![0u8; 24];
        mdhd_payload[12..16].copy_from_slice(&timescale.to_be_bytes());
        mdhd_payload[16..20].copy_from_slice(&100u32.to_be_bytes());
        let mdhd = make_box(b"mdhd", &mdhd_payload);

        let mut hdlr_payload = vec![0u8; 12];
        hdlr_payload[8..12].copy_from_slice(b"vide");
        let hdlr = make_box(b"hdlr", &hdlr_payload);

        let mut avc1_payload = vec![0u8; 78];
        avc1_payload[24..26].copy_from_slice(&16u16.to_be_bytes());
        avc1_payload[26..28].copy_from_slice(&16u16.to_be_bytes());
        avc1_payload.extend(make_box(b"avcC", &[1, 2, 3]));
        let avc1 = make_box(b"avc1", &avc1_payload);

        let mut stsd_payload = vec![0u8; 8];
        stsd_payload[4..8].copy_from_slice(&1u32.to_be_bytes());
        stsd_payload.extend(avc1);
        let stsd = make_box(b"stsd", &stsd_payload);

        // Fragmented tracks ship empty sample tables in `moov`.
        let stts = make_box(b"stts", &[0u8; 8]);
        let stsc = make_box(b"stsc", &[0u8; 8]);
        let stsz = make_box(b"stsz", &[0u8; 12]);
        let stco = make_box(b"stco", &[0u8; 8]);

        let mut stbl_payload = Vec::new();
        stbl_payload.extend(stsd);
        stbl_payload.extend(stts);
        stbl_payload.extend(stsc);
        stbl_payload.extend(stsz);
        stbl_payload.extend(stco);
        let stbl = make_box(b"stbl", &stbl_payload);
        let minf = make_box(b"minf", &stbl);

        let mut mdia_payload = Vec::new();
        mdia_payload.extend(mdhd);
        mdia_payload.extend(hdlr);
        mdia_payload.extend(minf);
        let mdia = make_box(b"mdia", &mdia_payload);

        let mut trak_payload = Vec::new();
        trak_payload.extend(tkhd);
        trak_payload.extend(mdia);
        make_box(b"trak", &trak_payload)
    }

    fn make_trex(
        track_id: u32,
        sample_duration: u32,
        sample_size: u32,
        sample_flags: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u32.to_be_bytes());
        payload.extend_from_slice(&track_id.to_be_bytes());
        payload.extend_from_slice(&1u32.to_be_bytes());
        payload.extend_from_slice(&sample_duration.to_be_bytes());
        payload.extend_from_slice(&sample_size.to_be_bytes());
        payload.extend_from_slice(&sample_flags.to_be_bytes());
        make_box(b"trex", &payload)
    }

    fn build_fragmented_mp4() -> Vec<u8> {
        let ftyp = make_box(
            b"ftyp",
            &[
                b'i', b's', b'o', b'm', 0, 0, 2, 0, b'i', b's', b'o', b'm', b'a', b'v', b'c', b'1',
            ],
        );

        let mut moov_payload = make_mvhd();
        moov_payload.extend(make_fragmented_trak(1, 1000));
        let mvex_payload = make_trex(1, 10, 5, 0x0101_0000);
        moov_payload.extend(make_box(b"mvex", &mvex_payload));
        let moov = make_box(b"moov", &moov_payload);

        let mut mfhd_payload = vec![0u8; 8];
        mfhd_payload[4..8].copy_from_slice(&1u32.to_be_bytes());
        let mfhd = make_box(b"mfhd", &mfhd_payload);

        let mut tfhd_payload = vec![0u8; 8];
        tfhd_payload[4..8].copy_from_slice(&1u32.to_be_bytes());
        let tfhd = make_box(b"tfhd", &tfhd_payload);

        let mut trun_payload = Vec::new();
        trun_payload.push(1); // Version 1: signed composition offsets.
        let trun_flags = 0x1 | 0x4 | 0x100 | 0x200 | 0x800u32;
        trun_payload.extend_from_slice(&trun_flags.to_be_bytes()[1..]);
        trun_payload.extend_from_slice(&3u32.to_be_bytes());
        trun_payload.extend_from_slice(&0i32.to_be_bytes()); // Patched below.
        trun_payload.extend_from_slice(&0x0200_0000u32.to_be_bytes());
        for &(duration, size, offset) in &[(10u32, 4u32, 0i32), (10, 4, 5), (20, 4, -3)] {
            trun_payload.extend_from_slice(&duration.to_be_bytes());
            trun_payload.extend_from_slice(&size.to_be_bytes());
            trun_payload.extend_from_slice(&offset.to_be_bytes());
        }
        let trun = make_box(b"trun", &trun_payload);

        // A second run continues after the first and relies entirely on the
        // `trex` default duration and size.
        let mut trun2_payload = Vec::new();
        trun2_payload.extend_from_slice(&0u32.to_be_bytes());
        trun2_payload.extend_from_slice(&2u32.to_be_bytes());
        let trun2 = make_box(b"trun", &trun2_payload);

        let mut traf_payload = Vec::new();
        traf_payload.extend(tfhd);
        traf_payload.extend(trun);
        traf_payload.extend(trun2);
        let traf = make_box(b"traf", &traf_payload);

        let mut moof_payload = Vec::new();
        moof_payload.extend(mfhd);
        moof_payload.extend(traf);
        let mut moof = make_box(b"moof", &moof_payload);

        // `moof` (8) + `mfhd` (16) + `traf` header (8) + `tfhd` (16) +
        // `trun` header (8) + version/flags and sample count (8).
        let data_offset_pos = 64;
        let data_offset = (moof.len() + 8) as i32;
        moof[data_offset_pos..data_offset_pos + 4].copy_from_slice(&data_offset.to_be_bytes());

        let mut file = ftyp;
        file.extend(moov);
        file.extend(moof);
        file.extend(make_box(
            b"mdat",
            &[
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
            ],
        ));
        file
    }

    #[test]
    fn creates_reader_from_empty_fails() {
        let data = vec![];
        let cursor = Cursor::new(data);
        assert!(Mp4Reader::new(cursor).is_err());
    }

    #[test]
    fn parses_minimal_container_and_track_metadata() {
        let data = build_minimal_mp4();
        let mut reader = Mp4Reader::new(Cursor::new(data)).unwrap();

        let file_type = reader.file_type().expect("ftyp should be parsed");
        assert_eq!(file_type.major_brand, *b"isom");
        assert_eq!(file_type.minor_version, 512);
        assert_eq!(file_type.compatible_brands, vec![*b"isom", *b"avc1"]);

        assert_eq!(reader.tracks().len(), 1);
        let track = &reader.tracks()[0];
        assert_eq!(track.track_id, 1);
        assert_eq!(track.track_type, TrackType::Video);
        assert_eq!(track.timescale, 1000);
        assert_eq!(track.duration, 100);
        assert_eq!(track.width, Some(320));
        assert_eq!(track.height, Some(240));
        assert_eq!(track.codec_private_data, vec![1, 2, 3]);

        assert_eq!(reader.sample_count(1), Some(1));
        let (mdat_offset, mdat_size) = reader.mdat_range().expect("mdat should be found");
        assert_eq!(mdat_size, 4);
        assert_eq!(mdat_offset + mdat_size, reader.file_size);

        let sample = reader.read_sample(1, 0).unwrap();
        assert_eq!(sample.data, vec![1, 2, 3, 4]);
        assert_eq!(sample.track_id, 1);
        assert_eq!(sample.timestamp, 0);
        assert_eq!(sample.dts, 0);
        assert_eq!(sample.pts, 0);
        assert_eq!(sample.duration, 1);
        assert!(sample.is_keyframe);
    }

    #[test]
    fn interleaves_packets_across_tracks_by_dts() {
        let data = build_interleaved_mp4();
        let mut reader = Mp4Reader::new(Cursor::new(data)).unwrap();

        let packets = reader
            .packets()
            .collect::<Mp4Result<Vec<Mp4Sample>>>()
            .unwrap();

        let summary: Vec<(u32, u64, i64, u64, u8)> = packets
            .iter()
            .map(|s| (s.track_id, s.dts, s.pts, s.duration, s.data[0]))
            .collect();

        assert_eq!(
            summary,
            vec![
                // Video track 1 uses timescale 1000; audio track 2 uses 48000.
                // Both start at DTS 0, so track ID breaks the tie.
                (1, 0, -20, 40, 1),
                (2, 0, 0, 960, 4),
                (2, 960, 960, 960, 5),
                // Video DTS 40 / 1000 s ties audio DTS 1920 / 48000 s.
                (1, 40, 20, 40, 2),
                (2, 1920, 1920, 960, 6),
                (1, 80, 60, 40, 3),
            ]
        );

        let timing = reader.sample_timing(1, 2).unwrap();
        assert_eq!(timing.dts, 80);
        assert_eq!(timing.pts, 60);
        assert_eq!(timing.duration, 40);
    }

    #[test]
    fn accepts_zero_sized_mdat_at_end_of_file() {
        let mut data = build_minimal_mp4();
        let mdat_start = data.len() - 12;
        data[mdat_start..mdat_start + 4].copy_from_slice(&0u32.to_be_bytes());
        let expected_offset = (data.len() - 4) as u64;

        let reader = Mp4Reader::new(Cursor::new(data)).unwrap();

        assert_eq!(reader.mdat_range(), Some((expected_offset, 4)));
    }

    #[test]
    fn flattens_fragmented_moof_samples_into_track_tables() {
        let data = build_fragmented_mp4();
        let mut reader = Mp4Reader::new(Cursor::new(data)).unwrap();

        assert_eq!(reader.tracks().len(), 1);
        assert_eq!(reader.sample_count(1), Some(5));

        assert_eq!(
            reader.sample_timing(1, 0).unwrap(),
            SampleTiming {
                dts: 0,
                pts: 0,
                duration: 10
            }
        );
        assert_eq!(
            reader.sample_timing(1, 1).unwrap(),
            SampleTiming {
                dts: 10,
                pts: 15,
                duration: 10
            }
        );
        assert_eq!(
            reader.sample_timing(1, 2).unwrap(),
            SampleTiming {
                dts: 20,
                pts: 17,
                duration: 20
            }
        );
        assert_eq!(
            reader.sample_timing(1, 3).unwrap(),
            SampleTiming {
                dts: 40,
                pts: 40,
                duration: 10
            }
        );
        assert_eq!(
            reader.sample_timing(1, 4).unwrap(),
            SampleTiming {
                dts: 50,
                pts: 50,
                duration: 10
            }
        );

        assert_eq!(reader.read_sample(1, 0).unwrap().data, vec![1, 2, 3, 4]);
        assert_eq!(reader.read_sample(1, 1).unwrap().data, vec![5, 6, 7, 8]);
        assert_eq!(reader.read_sample(1, 2).unwrap().data, vec![9, 10, 11, 12]);
        assert_eq!(
            reader.read_sample(1, 3).unwrap().data,
            vec![21, 22, 23, 24, 25]
        );
        assert_eq!(
            reader.read_sample(1, 4).unwrap().data,
            vec![26, 27, 28, 29, 30]
        );

        let keyframes: Vec<bool> = (0..5)
            .map(|i| reader.read_sample(1, i).unwrap().is_keyframe)
            .collect();
        assert_eq!(keyframes, vec![true, false, false, false, false]);

        let packets = reader
            .packets()
            .collect::<Mp4Result<Vec<Mp4Sample>>>()
            .unwrap();
        assert_eq!(packets.len(), 5);
        assert_eq!(
            packets.iter().map(|s| (s.dts, s.pts)).collect::<Vec<_>>(),
            vec![(0, 0), (10, 15), (20, 17), (40, 40), (50, 50)]
        );
    }

    #[test]
    fn ignores_trailing_padding_shorter_than_a_box() {
        let mut data = build_minimal_mp4();
        data.extend_from_slice(&[0u8; 4]);

        let mut reader = Mp4Reader::new(Cursor::new(data)).unwrap();
        assert_eq!(reader.sample_count(1), Some(1));
        assert_eq!(reader.read_sample(1, 0).unwrap().data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn rejects_child_box_beyond_parent() {
        let ftyp = make_box(b"ftyp", &[0u8; 8]);
        let mut moov_payload = Vec::new();
        moov_payload.extend_from_slice(&100u32.to_be_bytes());
        moov_payload.extend_from_slice(b"trak");
        let moov = make_box(b"moov", &moov_payload);

        let mut data = ftyp;
        data.extend(moov);

        assert!(matches!(
            Mp4Reader::new(Cursor::new(data)),
            Err(Mp4Error::InvalidFormat(_))
        ));
    }

    #[test]
    fn rejects_top_level_box_beyond_file() {
        let mut data = make_box(b"ftyp", &[0u8; 8]);
        data.extend_from_slice(&100u32.to_be_bytes());
        data.extend_from_slice(b"moov");

        assert!(matches!(
            Mp4Reader::new(Cursor::new(data)),
            Err(Mp4Error::InvalidFormat(_))
        ));
    }
}
