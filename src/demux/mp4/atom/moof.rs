//! Movie fragment (`moof`) parsing.
//!
//! Fragmented MP4 files split sample data into `moof`/`mdat` pairs. Each
//! `moof` contains one `traf` per track, and each `traf` contains one or more
//! `trun` boxes describing samples. This module parses those boxes and flattens
//! the fragment samples into the regular `SampleTableBox` representation so
//! the rest of the reader does not need to know a file is fragmented.

use super::{bounded_atom_end, finish_atom};
use crate::demux::mp4::parser::*;
use crate::demux::mp4::types::*;
use crate::demux::mp4::{Mp4Error, Mp4Result};
use std::collections::HashMap;
use std::io::{Read, Seek};

/// Sample flags bit indicating that a sample is not a sync sample.
const SAMPLE_IS_NON_SYNC: u32 = 0x0001_0000;

/// A parsed `moof` box.
#[derive(Debug, Default)]
pub struct MoofBox {
    pub trafs: Vec<TrafBox>,
}

#[derive(Debug, Default)]
pub struct TrafBox {
    pub tfhd: TfhdBox,
    pub truns: Vec<TrunBox>,
}

#[derive(Debug, Default)]
pub struct TfhdBox {
    pub track_id: u32,
    pub base_data_offset: Option<u64>,
    pub sample_description_index: Option<u32>,
    pub default_sample_duration: Option<u32>,
    pub default_sample_size: Option<u32>,
    pub default_sample_flags: Option<u32>,
}

#[derive(Debug, Default)]
pub struct TrunBox {
    pub version: u8,
    pub data_offset: Option<i32>,
    pub sample_count: u32,
    pub first_sample_flags: Option<u32>,
    pub sample_durations: Vec<u32>,
    pub sample_sizes: Vec<u32>,
    pub sample_flags: Vec<u32>,
    pub composition_offsets: Vec<i64>,
}

impl MoofBox {
    pub fn parse<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<Self> {
        let start_pos = reader.stream_position()?;
        let end_pos = start_pos
            .checked_add(size)
            .ok_or(Mp4Error::InvalidFormat("moof box size overflow"))?;

        let mut moof = Self::default();

        while reader.stream_position()? < end_pos {
            let atom_start = reader.stream_position()?;
            let header = parse_box_header(reader)?;
            let atom_end = bounded_atom_end(&header, atom_start, end_pos)?;
            let content_size = header.content_size();

            match header.box_type {
                BoxType::TRAF => {
                    moof.trafs
                        .push(Self::parse_traf(reader, content_size, end_pos)?);
                }
                _ => {
                    skip_bytes(reader, content_size)?;
                }
            }

            finish_atom(reader, atom_end)?;
        }

        Ok(moof)
    }

    fn parse_traf<R: Read + Seek>(
        reader: &mut R,
        size: u64,
        parent_end: u64,
    ) -> Mp4Result<TrafBox> {
        let start_pos = reader.stream_position()?;
        let end_pos = start_pos
            .checked_add(size)
            .ok_or(Mp4Error::InvalidFormat("traf box size overflow"))?;
        if end_pos > parent_end {
            return Err(Mp4Error::InvalidFormat("traf box extends beyond moof"));
        }

        let mut traf = TrafBox::default();
        let mut has_tfhd = false;

        while reader.stream_position()? < end_pos {
            let atom_start = reader.stream_position()?;
            let header = parse_box_header(reader)?;
            let atom_end = bounded_atom_end(&header, atom_start, end_pos)?;
            let content_size = header.content_size();

            match header.box_type {
                BoxType::TFHD => {
                    traf.tfhd = Self::parse_tfhd(reader, content_size)?;
                    has_tfhd = true;
                }
                BoxType::TRUN => {
                    traf.truns.push(Self::parse_trun(reader, content_size)?);
                }
                _ => {
                    skip_bytes(reader, content_size)?;
                }
            }

            finish_atom(reader, atom_end)?;
        }

        if !has_tfhd {
            return Err(Mp4Error::InvalidFormat("traf box is missing tfhd"));
        }

        Ok(traf)
    }

    fn parse_tfhd<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<TfhdBox> {
        ensure_box_size(size, 8, "tfhd box too small")?;

        skip_bytes(reader, 1)?;
        let flags = read_u24(reader)?;

        let required = 8u64
            + u64::from(flags & 0x1 != 0) * 8
            + u64::from(flags & 0x2 != 0) * 4
            + u64::from(flags & 0x8 != 0) * 4
            + u64::from(flags & 0x10 != 0) * 4
            + u64::from(flags & 0x20 != 0) * 4;
        if required > size {
            return Err(Mp4Error::InvalidFormat(
                "tfhd optional fields exceed box size",
            ));
        }

        let track_id = read_u32(reader)?;

        let base_data_offset = if flags & 0x1 != 0 {
            Some(read_u64(reader)?)
        } else {
            None
        };

        let sample_description_index = if flags & 0x2 != 0 {
            Some(read_u32(reader)?)
        } else {
            None
        };

        let default_sample_duration = if flags & 0x8 != 0 {
            Some(read_u32(reader)?)
        } else {
            None
        };

        let default_sample_size = if flags & 0x10 != 0 {
            Some(read_u32(reader)?)
        } else {
            None
        };

        let default_sample_flags = if flags & 0x20 != 0 {
            Some(read_u32(reader)?)
        } else {
            None
        };

        Ok(TfhdBox {
            track_id,
            base_data_offset,
            sample_description_index,
            default_sample_duration,
            default_sample_size,
            default_sample_flags,
        })
    }

    fn parse_trun<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<TrunBox> {
        ensure_box_size(size, 8, "trun box too small")?;

        let version = read_u8(reader)?;
        let flags = read_u24(reader)?;

        if version > 1 {
            return Err(Mp4Error::UnsupportedFeature("Unsupported trun version"));
        }

        let has_data_offset = flags & 0x1 != 0;
        let has_first_sample_flags = flags & 0x4 != 0;
        let has_duration = flags & 0x100 != 0;
        let has_size = flags & 0x200 != 0;
        let has_flags = flags & 0x400 != 0;
        let has_composition_offset = flags & 0x800 != 0;

        if has_first_sample_flags && has_flags {
            return Err(Mp4Error::InvalidFormat(
                "trun sets both first-sample-flags and sample-flags",
            ));
        }

        let sample_count = read_u32(reader)?;

        let fields_per_sample = u64::from(has_duration)
            + u64::from(has_size)
            + u64::from(has_flags)
            + u64::from(has_composition_offset);
        let required =
            8u64 + if has_data_offset { 4 } else { 0 } + if has_first_sample_flags { 4 } else { 0 };
        let required = (sample_count as u64)
            .checked_mul(fields_per_sample * 4)
            .and_then(|fields| required.checked_add(fields))
            .ok_or(Mp4Error::InvalidFormat("trun size overflow"))?;
        if required > size {
            return Err(Mp4Error::InvalidFormat(
                "trun sample fields exceed box size",
            ));
        }

        let data_offset = if has_data_offset {
            Some(read_u32(reader)? as i32)
        } else {
            None
        };

        let first_sample_flags = if has_first_sample_flags {
            Some(read_u32(reader)?)
        } else {
            None
        };

        let mut trun = TrunBox {
            version,
            data_offset,
            sample_count,
            first_sample_flags,
            sample_durations: Vec::new(),
            sample_sizes: Vec::new(),
            sample_flags: Vec::new(),
            composition_offsets: Vec::new(),
        };

        let count = sample_count as usize;
        trun.sample_durations
            .try_reserve_exact(if has_duration { count } else { 0 })
            .map_err(|_| Mp4Error::InvalidFormat("trun sample count too large"))?;
        trun.sample_sizes
            .try_reserve_exact(if has_size { count } else { 0 })
            .map_err(|_| Mp4Error::InvalidFormat("trun sample count too large"))?;
        trun.sample_flags
            .try_reserve_exact(if has_flags { count } else { 0 })
            .map_err(|_| Mp4Error::InvalidFormat("trun sample count too large"))?;
        trun.composition_offsets
            .try_reserve_exact(if has_composition_offset { count } else { 0 })
            .map_err(|_| Mp4Error::InvalidFormat("trun sample count too large"))?;

        for _ in 0..sample_count {
            if has_duration {
                trun.sample_durations.push(read_u32(reader)?);
            }
            if has_size {
                trun.sample_sizes.push(read_u32(reader)?);
            }
            if has_flags {
                trun.sample_flags.push(read_u32(reader)?);
            }
            if has_composition_offset {
                if version == 0 {
                    trun.composition_offsets.push(read_u32(reader)? as i64);
                } else {
                    trun.composition_offsets.push(read_i32(reader)? as i64);
                }
            }
        }

        Ok(trun)
    }

    /// Flatten every track fragment in this `moof` into the track sample
    /// tables.
    ///
    /// Each `trun` is appended as one chunk. Sample durations, sizes, flags, and
    /// composition offsets fall back to the values declared by `tfhd`, then to
    /// the track defaults from `mvex`/`trex`.
    pub fn append_samples(
        &self,
        moof_start: u64,
        trex_defaults: &HashMap<u32, TrexDefaults>,
        sample_tables: &mut HashMap<u32, SampleTableBox>,
    ) -> Mp4Result<()> {
        for traf in &self.trafs {
            let track_id = traf.tfhd.track_id;
            let table = sample_tables
                .get_mut(&track_id)
                .ok_or(Mp4Error::InvalidFormat(
                    "movie fragment references an unknown track",
                ))?;

            let trex = trex_defaults.get(&track_id).copied().unwrap_or_default();
            let default_duration = traf
                .tfhd
                .default_sample_duration
                .unwrap_or(trex.sample_duration);
            let default_size = traf.tfhd.default_sample_size.unwrap_or(trex.sample_size);
            let default_flags = traf.tfhd.default_sample_flags.unwrap_or(trex.sample_flags);
            let description_index = traf
                .tfhd
                .sample_description_index
                .unwrap_or(trex.sample_description_index)
                .max(1);

            let traf_base_pos = traf.tfhd.base_data_offset.unwrap_or(moof_start);
            let mut trun_pos = traf_base_pos;

            let base_sample_count = table.sample_sizes.len();
            let mut appended = 0usize;
            let mut fragment_sync = Vec::new();
            let mut fragment_offsets = Vec::new();
            let mut has_nonzero_composition_offset = false;

            for trun in &traf.truns {
                if let Some(offset) = trun.data_offset {
                    trun_pos = apply_signed_offset(traf_base_pos, offset)?;
                }

                let sample_count = trun.sample_count as usize;
                if sample_count == 0 {
                    continue;
                }

                let first_chunk = u32::try_from(table.chunk_offsets.len() + 1)
                    .map_err(|_| Mp4Error::InvalidFormat("chunk count overflow"))?;

                table.chunk_offsets.push(trun_pos);
                table.sample_to_chunk.push(SampleToChunk {
                    first_chunk,
                    samples_per_chunk: trun.sample_count,
                    sample_description_index: description_index,
                });

                for sample_index in 0..sample_count {
                    let size = trun
                        .sample_sizes
                        .get(sample_index)
                        .copied()
                        .unwrap_or(default_size);

                    let duration = trun
                        .sample_durations
                        .get(sample_index)
                        .copied()
                        .unwrap_or(default_duration);

                    let flags = trun.sample_flags_for(sample_index, default_flags);

                    let composition_offset = trun
                        .composition_offsets
                        .get(sample_index)
                        .copied()
                        .unwrap_or(0);

                    table.sample_sizes.push(size);
                    push_time_to_sample(&mut table.time_to_sample, duration);

                    push_composition_offset(&mut fragment_offsets, composition_offset);
                    if composition_offset != 0 {
                        has_nonzero_composition_offset = true;
                    }

                    appended += 1;
                    if flags & SAMPLE_IS_NON_SYNC == 0 {
                        let sample_number = u32::try_from(base_sample_count + appended)
                            .map_err(|_| Mp4Error::InvalidFormat("sample number overflow"))?;
                        fragment_sync.push(sample_number);
                    }

                    trun_pos = trun_pos
                        .checked_add(u64::from(size))
                        .ok_or(Mp4Error::InvalidFormat("fragment sample offset overflow"))?;
                }
            }

            if appended == 0 {
                continue;
            }

            if has_nonzero_composition_offset || table.composition_offsets.is_some() {
                let mut offsets = table.composition_offsets.take().unwrap_or_default();

                if offsets.is_empty() && base_sample_count > 0 {
                    offsets.push(CompositionOffset {
                        sample_count: u32::try_from(base_sample_count)
                            .map_err(|_| Mp4Error::InvalidFormat("sample count overflow"))?,
                        sample_offset: 0,
                    });
                }

                merge_composition_offsets(&mut offsets, &fragment_offsets);
                table.composition_offsets = Some(offsets);
            }

            merge_sync_samples(
                &mut table.sync_samples,
                base_sample_count,
                appended,
                &fragment_sync,
            )?;
        }

        Ok(())
    }
}

impl TrunBox {
    fn sample_flags_for(&self, sample_index: usize, default_flags: u32) -> u32 {
        if let Some(flags) = self.sample_flags.get(sample_index) {
            return *flags;
        }

        if sample_index == 0
            && let Some(flags) = self.first_sample_flags
        {
            return flags;
        }

        default_flags
    }
}

fn apply_signed_offset(base: u64, offset: i32) -> Mp4Result<u64> {
    if offset >= 0 {
        base.checked_add(offset as u64)
    } else {
        base.checked_sub(u64::from(offset.unsigned_abs()))
    }
    .ok_or(Mp4Error::InvalidFormat("trun data offset out of range"))
}

fn push_time_to_sample(entries: &mut Vec<TimeToSample>, sample_delta: u32) {
    if let Some(last) = entries.last_mut()
        && last.sample_delta == sample_delta
    {
        last.sample_count = last.sample_count.saturating_add(1);
        return;
    }

    entries.push(TimeToSample {
        sample_count: 1,
        sample_delta,
    });
}

fn push_composition_offset(entries: &mut Vec<CompositionOffset>, sample_offset: i64) {
    if let Some(last) = entries.last_mut()
        && last.sample_offset == sample_offset
    {
        last.sample_count = last.sample_count.saturating_add(1);
        return;
    }

    entries.push(CompositionOffset {
        sample_count: 1,
        sample_offset,
    });
}

fn merge_composition_offsets(dest: &mut Vec<CompositionOffset>, src: &[CompositionOffset]) {
    for entry in src {
        if entry.sample_count == 0 {
            continue;
        }

        if let Some(last) = dest.last_mut()
            && last.sample_offset == entry.sample_offset
        {
            last.sample_count = last.sample_count.saturating_add(entry.sample_count);
            continue;
        }

        dest.push(*entry);
    }
}

fn merge_sync_samples(
    sync_samples: &mut Option<Vec<u32>>,
    base_sample_count: usize,
    appended: usize,
    fragment_sync: &[u32],
) -> Mp4Result<()> {
    let all_sync = fragment_sync.len() == appended;

    if all_sync && sync_samples.is_none() {
        // Without an explicit `stss` table every sample is a sync sample.
        return Ok(());
    }

    let samples = match sync_samples {
        Some(samples) => samples,
        None => {
            // The track previously had no `stss` table, so every existing
            // sample was implicitly a sync sample.
            let mut samples = Vec::new();
            samples
                .try_reserve(base_sample_count + fragment_sync.len())
                .map_err(|_| Mp4Error::InvalidFormat("sync sample count too large"))?;
            let existing = u32::try_from(base_sample_count)
                .map_err(|_| Mp4Error::InvalidFormat("sample count overflow"))?;
            samples.extend(1..=existing);
            *sync_samples = Some(samples);
            sync_samples.as_mut().expect("just assigned")
        }
    };

    samples.extend_from_slice(fragment_sync);

    Ok(())
}
