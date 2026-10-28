use openwave::{WavError, WavFile};

fn make_wav(channels: u16, sample_rate: u32, bits: u16, pcm: &[u8]) -> Vec<u8> {
    let block_align = channels * (bits / 8);
    let byte_rate = sample_rate * block_align as u32;
    let riff_size = 36 + pcm.len() as u32;

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_size.to_le_bytes());
    bytes.extend_from_slice(b"WAVE");

    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&bits.to_le_bytes());

    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    bytes.extend_from_slice(pcm);
    bytes
}

#[test]
fn parses_mono_16_bit_pcm() {
    let pcm = [0u8; 16_000 * 2];
    let wav = WavFile::parse(make_wav(1, 16_000, 16, &pcm)).unwrap();

    assert_eq!(wav.info.channels, 1);
    assert_eq!(wav.info.sample_rate, 16_000);
    assert_eq!(wav.info.bits_per_sample, 16);
    assert_eq!(wav.info.frame_count(), 16_000);
    assert!((wav.info.duration_seconds() - 1.0).abs() < 0.001);
    assert_eq!(wav.pcm_data().len(), pcm.len());
}

#[test]
fn parses_stereo_24_bit_pcm() {
    let pcm = [0u8; 48];
    let wav = WavFile::parse(make_wav(2, 48_000, 24, &pcm)).unwrap();

    assert_eq!(wav.info.block_align, 6);
    assert_eq!(wav.info.frame_count(), 8);
}

#[test]
fn rejects_non_riff_data() {
    assert_eq!(
        WavFile::parse(b"not a WAV file".to_vec()).unwrap_err(),
        WavError::InvalidRiff
    );
}

#[test]
fn rejects_unsupported_encoding() {
    let mut bytes = make_wav(1, 44_100, 16, &[0, 0]);
    bytes[20..22].copy_from_slice(&3u16.to_le_bytes());

    assert_eq!(
        WavFile::parse(bytes).unwrap_err(),
        WavError::UnsupportedFormat(3)
    );
}

#[test]
fn rejects_partial_pcm_frame() {
    let bytes = make_wav(2, 48_000, 16, &[0, 0, 0]);

    assert_eq!(WavFile::parse(bytes).unwrap_err(), WavError::TruncatedData);
}
