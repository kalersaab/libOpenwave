//! Sample-table calculations.
//!
//! This module implements the timing and byte-offset rules from ISO/IEC
//! 14496-12 for `stts`, `ctts`, `stsc`, `stsz`, `stco`/`co64`, and `stss`.
//!
//! * Decode timestamps are cumulative `stts` deltas.
//! * Presentation timestamps are decode timestamps plus `ctts` composition
//!   offsets (`PTS = DTS + composition_offset`).
//! * Sample byte offsets come from mapping samples to chunks with `stsc`, then
//!   walking the chunks with `stco`/`co64` and accumulating `stsz` sizes.

use super::types::SampleTableBox;
use super::{Mp4Error, Mp4Result};

/// Timing information for a single sample, in track timescale units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleTiming {
    /// Decode timestamp (DTS).
    pub dts: u64,
    /// Presentation timestamp (PTS), which may be negative for `ctts` v1.
    pub pts: i64,
    /// Duration of the sample.
    pub duration: u64,
}

/// Compute the decode timestamp, presentation timestamp, and duration of a
/// sample from `stts` and `ctts`.
pub fn sample_timing(table: &SampleTableBox, sample_index: usize) -> Mp4Result<SampleTiming> {
    let mut dts = 0u64;
    let mut first_sample = 0usize;
    let mut sample_delta = None;

    for entry in &table.time_to_sample {
        if entry.sample_count == 0 {
            continue;
        }

        let count = entry.sample_count as usize;
        let end = first_sample
            .checked_add(count)
            .ok_or(Mp4Error::InvalidFormat("stts sample count overflow"))?;

        if sample_index < end {
            let samples_before = sample_index - first_sample;
            let delta = entry.sample_delta as u64;
            let advance = (samples_before as u64)
                .checked_mul(delta)
                .ok_or(Mp4Error::InvalidFormat("stts timestamp overflow"))?;
            dts = dts
                .checked_add(advance)
                .ok_or(Mp4Error::InvalidFormat("stts timestamp overflow"))?;
            sample_delta = Some(delta);
            break;
        }

        let advance = (count as u64)
            .checked_mul(entry.sample_delta as u64)
            .ok_or(Mp4Error::InvalidFormat("stts timestamp overflow"))?;
        dts = dts
            .checked_add(advance)
            .ok_or(Mp4Error::InvalidFormat("stts timestamp overflow"))?;
        first_sample = end;
    }

    let duration = sample_delta.ok_or(Mp4Error::InvalidFormat("stts does not cover sample"))?;

    let composition_offset = composition_offset(table, sample_index)?;
    let pts = i64::try_from(dts as i128 + composition_offset as i128)
        .map_err(|_| Mp4Error::InvalidFormat("presentation timestamp overflow"))?;

    Ok(SampleTiming { dts, pts, duration })
}

/// Return the composition offset that applies to `sample_index`.
///
/// `ctts` is allowed to omit trailing samples in some files; those samples use
/// a composition offset of zero.
fn composition_offset(table: &SampleTableBox, sample_index: usize) -> Mp4Result<i64> {
    let Some(entries) = table.composition_offsets.as_ref() else {
        return Ok(0);
    };

    let mut first_sample = 0usize;

    for entry in entries {
        if entry.sample_count == 0 {
            continue;
        }

        let end = first_sample
            .checked_add(entry.sample_count as usize)
            .ok_or(Mp4Error::InvalidFormat("ctts sample count overflow"))?;

        if sample_index < end {
            return Ok(entry.sample_offset);
        }

        first_sample = end;
    }

    Ok(0)
}

/// Compute the absolute file offset of a sample using `stsc`, `stco`/`co64`,
/// and `stsz`.
pub fn sample_offset(table: &SampleTableBox, sample_index: usize) -> Mp4Result<u64> {
    if sample_index >= table.sample_sizes.len() {
        return Err(Mp4Error::InvalidFormat("Sample index out of bounds"));
    }

    if table.sample_to_chunk.is_empty() {
        return Err(Mp4Error::InvalidFormat("Missing stsc table"));
    }

    if table.sample_to_chunk[0].first_chunk != 1 {
        return Err(Mp4Error::InvalidFormat(
            "first stsc entry must start at chunk 1",
        ));
    }

    let mut current_sample = 0usize;

    for (i, entry) in table.sample_to_chunk.iter().enumerate() {
        if entry.samples_per_chunk == 0 {
            return Err(Mp4Error::InvalidFormat("invalid stsc entry"));
        }

        let first_chunk = entry.first_chunk as usize;
        let next_chunk = if let Some(next) = table.sample_to_chunk.get(i + 1) {
            next.first_chunk as usize
        } else {
            table
                .chunk_offsets
                .len()
                .checked_add(1)
                .ok_or(Mp4Error::InvalidFormat("chunk count overflow"))?
        };

        let chunk_count = next_chunk
            .checked_sub(first_chunk)
            .ok_or(Mp4Error::InvalidFormat("invalid stsc chunk range"))?;
        let samples_per_chunk = entry.samples_per_chunk as usize;
        let samples_in_range = chunk_count
            .checked_mul(samples_per_chunk)
            .ok_or(Mp4Error::InvalidFormat("stsc sample count overflow"))?;
        let range_end = current_sample
            .checked_add(samples_in_range)
            .ok_or(Mp4Error::InvalidFormat("stsc sample count overflow"))?;

        if sample_index < range_end {
            let samples_before_target = sample_index - current_sample;
            let chunks_before_target = samples_before_target / samples_per_chunk;
            let sample_in_chunk = samples_before_target % samples_per_chunk;

            let chunk_index = (first_chunk - 1)
                .checked_add(chunks_before_target)
                .ok_or(Mp4Error::InvalidFormat("chunk index overflow"))?;

            let mut offset = *table
                .chunk_offsets
                .get(chunk_index)
                .ok_or(Mp4Error::InvalidFormat("Chunk offset not found"))?;

            let first_sample_in_chunk = current_sample
                .checked_add(
                    chunks_before_target
                        .checked_mul(samples_per_chunk)
                        .ok_or(Mp4Error::InvalidFormat("stsc sample count overflow"))?,
                )
                .ok_or(Mp4Error::InvalidFormat("stsc sample count overflow"))?;
            let sample_in_chunk_end = first_sample_in_chunk
                .checked_add(sample_in_chunk)
                .ok_or(Mp4Error::InvalidFormat("stsc sample count overflow"))?;

            let sizes = table
                .sample_sizes
                .get(first_sample_in_chunk..sample_in_chunk_end)
                .ok_or(Mp4Error::InvalidFormat("stsc references missing sample"))?;

            for &size in sizes {
                offset = offset
                    .checked_add(size as u64)
                    .ok_or(Mp4Error::InvalidFormat("sample offset overflow"))?;
            }

            return Ok(offset);
        }

        current_sample = range_end;
    }

    Err(Mp4Error::InvalidFormat("Could not calculate sample offset"))
}

/// Return whether the sample is a sync sample.
///
/// When `stss` is absent every sample is a sync sample. When it is present its
/// entries are one-based sample numbers.
pub fn is_sync_sample(table: &SampleTableBox, sample_index: usize) -> bool {
    match table.sync_samples.as_ref() {
        None => true,
        Some(samples) => {
            let sample_number = (sample_index as u64)
                .checked_add(1)
                .and_then(|n| u32::try_from(n).ok());
            sample_number
                .map(|n| samples.binary_search(&n).is_ok())
                .unwrap_or(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demux::mp4::types::{CompositionOffset, SampleToChunk, TimeToSample};

    fn table(
        stts: &[(u32, u32)],
        ctts: Option<&[(u32, i64)]>,
        stsc: &[(u32, u32)],
        chunk_offsets: &[u64],
        sample_sizes: &[u32],
    ) -> SampleTableBox {
        SampleTableBox {
            chunk_offsets: chunk_offsets.to_vec(),
            sample_sizes: sample_sizes.to_vec(),
            sample_to_chunk: stsc
                .iter()
                .map(|&(first_chunk, samples_per_chunk)| SampleToChunk {
                    first_chunk,
                    samples_per_chunk,
                    sample_description_index: 1,
                })
                .collect(),
            time_to_sample: stts
                .iter()
                .map(|&(sample_count, sample_delta)| TimeToSample {
                    sample_count,
                    sample_delta,
                })
                .collect(),
            composition_offsets: ctts.map(|entries| {
                entries
                    .iter()
                    .map(|&(sample_count, sample_offset)| CompositionOffset {
                        sample_count,
                        sample_offset,
                    })
                    .collect()
            }),
            sync_samples: None,
        }
    }

    #[test]
    fn computes_cumulative_stts_dts_and_duration() {
        let table = table(&[(2, 10), (1, 20)], None, &[(1, 3)], &[0], &[1, 1, 1]);

        assert_eq!(
            sample_timing(&table, 0).unwrap(),
            SampleTiming {
                dts: 0,
                pts: 0,
                duration: 10
            }
        );
        assert_eq!(
            sample_timing(&table, 1).unwrap(),
            SampleTiming {
                dts: 10,
                pts: 10,
                duration: 10
            }
        );
        assert_eq!(
            sample_timing(&table, 2).unwrap(),
            SampleTiming {
                dts: 20,
                pts: 20,
                duration: 20
            }
        );
        assert!(sample_timing(&table, 3).is_err());
    }

    #[test]
    fn applies_ctts_version_0_offsets_to_pts() {
        let table = table(
            &[(4, 10)],
            Some(&[(1, 5), (2, 0)]),
            &[(1, 4)],
            &[0],
            &[1, 1, 1, 1],
        );

        // Sample 0 uses the first entry's offset; samples 1 and 2 use zero;
        // sample 3 is beyond the ctts table and therefore also uses zero.
        let pts: Vec<i64> = (0..4)
            .map(|i| sample_timing(&table, i).unwrap().pts)
            .collect();
        assert_eq!(pts, vec![5, 10, 20, 30]);
    }

    #[test]
    fn applies_negative_ctts_version_1_offsets_to_pts() {
        let table = table(
            &[(2, 100)],
            Some(&[(1, -40), (1, 10)]),
            &[(1, 2)],
            &[0],
            &[1, 1],
        );

        let first = sample_timing(&table, 0).unwrap();
        assert_eq!(first.dts, 0);
        assert_eq!(first.pts, -40);

        let second = sample_timing(&table, 1).unwrap();
        assert_eq!(second.dts, 100);
        assert_eq!(second.pts, 110);
    }

    #[test]
    fn maps_samples_to_chunks_across_stsc_entries() {
        // 3 chunks:
        //   chunk 1: samples 0, 1
        //   chunk 2: samples 2, 3
        //   chunk 3: sample 4
        let table = table(
            &[(5, 10)],
            None,
            &[(1, 2), (3, 1)],
            &[100, 200, 300],
            &[10, 20, 30, 40, 50],
        );

        assert_eq!(sample_offset(&table, 0).unwrap(), 100);
        assert_eq!(sample_offset(&table, 1).unwrap(), 110);
        assert_eq!(sample_offset(&table, 2).unwrap(), 200);
        assert_eq!(sample_offset(&table, 3).unwrap(), 230);
        assert_eq!(sample_offset(&table, 4).unwrap(), 300);
    }

    #[test]
    fn supports_co64_style_large_chunk_offsets() {
        let base = u32::MAX as u64 + 123;
        let table = table(&[(2, 10)], None, &[(1, 2)], &[base], &[7, 9]);

        assert_eq!(sample_offset(&table, 0).unwrap(), base);
        assert_eq!(sample_offset(&table, 1).unwrap(), base + 7);
    }

    #[test]
    fn finds_sync_samples_with_one_based_stss_numbers() {
        let mut table = table(&[(3, 10)], None, &[(1, 3)], &[0], &[1, 1, 1]);
        table.sync_samples = Some(vec![1, 3]);

        assert!(is_sync_sample(&table, 0));
        assert!(!is_sync_sample(&table, 1));
        assert!(is_sync_sample(&table, 2));

        table.sync_samples = None;
        assert!(is_sync_sample(&table, 1));
    }

    #[test]
    fn rejects_out_of_range_and_inconsistent_tables() {
        let table = table(&[(1, 10)], None, &[(1, 2)], &[0], &[1, 1]);

        assert!(sample_offset(&table, 2).is_err());

        let missing_chunk = table_with_chunks(&[]);
        assert!(sample_offset(&missing_chunk, 0).is_err());
    }

    fn table_with_chunks(chunk_offsets: &[u64]) -> SampleTableBox {
        table(&[(1, 10)], None, &[(1, 1)], chunk_offsets, &[1])
    }
}
