use std::f32::consts::PI;

pub struct SynthesisFilterbank {
    v: [[f32; 1024]; 2],
    actual_v: [usize; 2],
}

impl SynthesisFilterbank {
    pub fn new() -> Self {
        SynthesisFilterbank {
            v: [[0.0; 1024]; 2],
            actual_v: [0; 2],
        }
    }

    pub fn synthesize(&mut self, samples: &[f32; 32], channel: usize, output: &mut [f32]) {
        assert!(channel < 2);
        assert_eq!(output.len(), 32);

        let actual = self.actual_v[channel];

        for (i, &sample) in samples.iter().enumerate() {
            self.v[channel][(actual + i) % 1024] = sample;
        }

        let mut u = [0.0f32; 512];
        for i in 0..8 {
            for j in 0..32 {
                let idx = (actual + i * 64 + j) % 1024;
                u[i * 64 + j] = self.v[channel][idx];
            }
        }

        let mut w = [0.0f32; 512];
        for i in 0..512 {
            w[i] = u[i] * Self::window_coeff(i);
        }

        for i in 0..32 {
            let mut sum = 0.0;
            for j in 0..16 {
                sum += w[j * 32 + i];
            }
            output[i] = sum;
        }

        self.actual_v[channel] = (actual + 32) % 1024;
    }

    fn window_coeff(i: usize) -> f32 {
        let n = i as f32;
        let angle = PI / 512.0 * (n + 0.5);
        angle.sin()
    }
}

impl Default for SynthesisFilterbank {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_synthesis_filterbank() {
        let fb = SynthesisFilterbank::new();
        assert_eq!(fb.actual_v[0], 0);
        assert_eq!(fb.actual_v[1], 0);
    }

    #[test]
    fn synthesizes_samples() {
        let mut fb = SynthesisFilterbank::new();
        let input = [0.0; 32];
        let mut output = [0.0; 32];

        fb.synthesize(&input, 0, &mut output);

        assert!(output.iter().all(|&x| x.abs() < 0.001));
    }

    #[test]
    fn advances_buffer_position() {
        let mut fb = SynthesisFilterbank::new();
        let input = [1.0; 32];
        let mut output = [0.0; 32];

        let initial_pos = fb.actual_v[0];
        fb.synthesize(&input, 0, &mut output);

        assert_ne!(fb.actual_v[0], initial_pos);
    }

    #[test]
    fn handles_dual_channels() {
        let mut fb = SynthesisFilterbank::new();
        let input = [0.5; 32];
        let mut output_l = [0.0; 32];
        let mut output_r = [0.0; 32];

        fb.synthesize(&input, 0, &mut output_l);
        fb.synthesize(&input, 1, &mut output_r);

        assert_ne!(fb.actual_v[0], 0);
        assert_ne!(fb.actual_v[1], 0);
    }
}
