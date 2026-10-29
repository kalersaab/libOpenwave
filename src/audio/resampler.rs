#[derive(Debug, Clone, PartialEq)]
pub enum ResamplerError {
    InvalidSampleRate,
    InvalidChannelCount,
    MisalignedSamples,
}

impl std::fmt::Display for ResamplerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSampleRate => write!(f, "sample rates must be greater than zero"),
            Self::InvalidChannelCount => write!(f, "channel count must be greater than zero"),
            Self::MisalignedSamples => write!(f, "samples do not contain complete frames"),
        }
    }
}

impl std::error::Error for ResamplerError {}

pub struct LinearResampler {
    input_rate: u32,
    output_rate: u32,
    channels: usize,
    input: Vec<f32>,
    base_frame: u64,
    next_output_frame: u64,
    finished: bool,
}

impl LinearResampler {
    pub fn new(input_rate: u32, output_rate: u32, channels: usize) -> Result<Self, ResamplerError> {
        Self::new_at(input_rate, output_rate, channels, 0, 0)
    }

    pub fn new_at(
        input_rate: u32,
        output_rate: u32,
        channels: usize,
        source_start_frame: u64,
        output_start_frame: u64,
    ) -> Result<Self, ResamplerError> {
        if input_rate == 0 || output_rate == 0 {
            return Err(ResamplerError::InvalidSampleRate);
        }

        if channels == 0 {
            return Err(ResamplerError::InvalidChannelCount);
        }

        Ok(Self {
            input_rate,
            output_rate,
            channels,
            input: Vec::new(),
            base_frame: source_start_frame,
            next_output_frame: output_start_frame,
            finished: false,
        })
    }

    pub fn push(&mut self, samples: &[f32]) -> Result<Vec<f32>, ResamplerError> {
        if self.finished {
            return Ok(Vec::new());
        }
        if !samples.len().is_multiple_of(self.channels) {
            return Err(ResamplerError::MisalignedSamples);
        }

        self.input.extend_from_slice(samples);
        self.produce(false)
    }

    pub fn finish(&mut self) -> Vec<f32> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        self.produce(true).unwrap_or_default()
    }

    fn produce(&mut self, eof: bool) -> Result<Vec<f32>, ResamplerError> {
        let available_frames = self.input.len() / self.channels;
        let end_frame = self.base_frame + available_frames as u64;
        let mut output = Vec::new();

        loop {
            let numerator = self.next_output_frame as u128 * self.input_rate as u128;
            let source_frame = (numerator / self.output_rate as u128) as u64;
            let remainder = (numerator % self.output_rate as u128) as f32;
            let fraction = remainder / self.output_rate as f32;

            if source_frame >= end_frame {
                break;
            }

            let next_frame = source_frame + 1;
            if next_frame >= end_frame && !eof {
                break;
            }

            let first_local = (source_frame - self.base_frame) as usize;
            let second_local = if next_frame < end_frame {
                (next_frame - self.base_frame) as usize
            } else {
                first_local
            };

            for channel in 0..self.channels {
                let a = self.input[first_local * self.channels + channel];
                let b = self.input[second_local * self.channels + channel];
                output.push(a + (b - a) * fraction);
            }

            self.next_output_frame += 1;
        }

        let numerator = self.next_output_frame as u128 * self.input_rate as u128;
        let next_source = (numerator / self.output_rate as u128) as u64;
        let keep_from = next_source
            .saturating_sub(self.base_frame)
            .min(available_frames as u64);

        if keep_from > 0 {
            let discard_samples = keep_from as usize * self.channels;
            self.input.drain(..discard_samples);
            self.base_frame += keep_from;
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_sample_rates() {
        assert!(LinearResampler::new(0, 48_000, 1).is_err());
        assert!(LinearResampler::new(44_100, 0, 1).is_err());
    }

    #[test]
    fn rejects_misaligned_stereo_samples() {
        let mut r = LinearResampler::new(44_100, 48_000, 2).unwrap();
        assert_eq!(
            r.push(&[0.0, 0.1, 0.2]),
            Err(ResamplerError::MisalignedSamples)
        );
    }

    #[test]
    fn preserves_stereo_channel_alignment() {
        let mut r = LinearResampler::new(2, 4, 2).unwrap();
        let mut out = r.push(&[1.0, -1.0, 3.0, -3.0]).unwrap();
        out.extend(r.finish());

        assert_eq!(out.len() % 2, 0);
        assert_eq!(out[0], 1.0);
        assert_eq!(out[1], -1.0);
        assert_eq!(out[2], 2.0);
        assert_eq!(out[3], -2.0);
    }

    #[test]
    fn chunking_does_not_change_output() {
        let input = [0.0, 1.0, 0.0, -1.0, 0.5, 0.25];
        let mut whole = LinearResampler::new(3, 5, 1).unwrap();
        let mut expected = whole.push(&input).unwrap();
        expected.extend(whole.finish());

        let mut split = LinearResampler::new(3, 5, 1).unwrap();
        let mut actual = split.push(&input[..2]).unwrap();
        actual.extend(split.push(&input[2..4]).unwrap());
        actual.extend(split.push(&input[4..]).unwrap());
        actual.extend(split.finish());

        assert_eq!(actual.len(), expected.len());
        for (a, b) in actual.iter().zip(expected.iter()) {
            assert!((a - b).abs() < 1e-6, "{a} != {b}");
        }
    }

    #[test]
    fn extreme_downsampling_produces_output() {
        let mut r = LinearResampler::new(192_000, 8_000, 1).unwrap();
        let input: Vec<f32> = (0..1920).map(|x| x as f32).collect();

        let mut out = r.push(&input).unwrap();
        out.extend(r.finish());

        assert_eq!(out.len(), 80);
        assert!(out.iter().all(|sample| sample.is_finite()));
    }
}
