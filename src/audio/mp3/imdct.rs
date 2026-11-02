use std::f32::consts::PI;

pub struct Mp3Imdct {
    n: usize,
}

impl Mp3Imdct {
    pub fn new(n: usize) -> Self {
        Mp3Imdct { n }
    }

    pub fn process(&self, input: &[f32], output: &mut [f32], block_type: u8) {
        match (self.n, block_type) {
            (18, 2) => self.imdct_short(input, output),
            (18, _) => self.imdct_long(input, output),
            _ => {}
        }
    }

    fn imdct_long(&self, input: &[f32], output: &mut [f32]) {
        assert_eq!(self.n, 18);
        assert_eq!(input.len(), 18);
        assert_eq!(output.len(), 36);

        for (i, output_sample) in output.iter_mut().enumerate() {
            let mut sum = 0.0;
            for (k, &input_val) in input.iter().enumerate() {
                let angle = PI / 36.0 * (i as f32 + 0.5 + 18.0) * (k as f32 + 0.5);
                sum += input_val * angle.cos();
            }
            *output_sample = sum;
        }
    }

    fn imdct_short(&self, input: &[f32], output: &mut [f32]) {
        assert_eq!(self.n, 18);
        assert_eq!(input.len(), 18);
        assert_eq!(output.len(), 12);

        for (i, output_sample) in output.iter_mut().enumerate() {
            let mut sum = 0.0;
            for (k, &input_val) in input.iter().take(6).enumerate() {
                let angle = PI / 12.0 * (i as f32 + 0.5 + 6.0) * (k as f32 + 0.5);
                sum += input_val * angle.cos();
            }
            *output_sample = sum;
        }
    }

    pub fn window(&self, samples: &mut [f32], block_type: u8) {
        match block_type {
            0 => self.window_normal(samples),
            1 => self.window_start(samples),
            2 => self.window_short(samples),
            3 => self.window_stop(samples),
            _ => {}
        }
    }

    fn window_normal(&self, samples: &mut [f32]) {
        for (i, sample) in samples.iter_mut().enumerate() {
            let angle = PI / 36.0 * (i as f32 + 0.5);
            *sample *= angle.sin();
        }
    }

    fn window_start(&self, samples: &mut [f32]) {
        for (i, sample) in samples.iter_mut().enumerate().take(18) {
            let angle = PI / 36.0 * (i as f32 + 0.5);
            *sample *= angle.sin();
        }
        for sample in samples.iter_mut().skip(18).take(6) {
            *sample *= 1.0;
        }
        for (i, sample) in samples.iter_mut().enumerate().skip(24).take(6) {
            let angle = PI / 12.0 * (i as f32 - 18.0 + 0.5);
            *sample *= angle.sin();
        }
        for sample in samples.iter_mut().skip(30).take(6) {
            *sample *= 0.0;
        }
    }

    fn window_short(&self, samples: &mut [f32]) {
        for (i, sample) in samples.iter_mut().enumerate() {
            let angle = PI / 12.0 * (i as f32 + 0.5);
            *sample *= angle.sin();
        }
    }

    fn window_stop(&self, samples: &mut [f32]) {
        for sample in samples.iter_mut().take(6) {
            *sample *= 0.0;
        }
        for (i, sample) in samples.iter_mut().enumerate().skip(6).take(6) {
            let angle = PI / 12.0 * (i as f32 - 6.0 + 0.5);
            *sample *= angle.sin();
        }
        for sample in samples.iter_mut().skip(12).take(6) {
            *sample *= 1.0;
        }
        for (i, sample) in samples.iter_mut().enumerate().skip(18).take(18) {
            let angle = PI / 36.0 * (i as f32 + 0.5);
            *sample *= angle.sin();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_imdct_18() {
        let imdct = Mp3Imdct::new(18);
        assert_eq!(imdct.n, 18);
    }

    #[test]
    fn processes_long_block() {
        let imdct = Mp3Imdct::new(18);
        let input = vec![0.0; 18];
        let mut output = vec![0.0; 36];

        imdct.process(&input, &mut output, 0);

        assert!(output.iter().all(|&x| x.abs() < 0.001));
    }

    #[test]
    fn processes_short_block() {
        let imdct = Mp3Imdct::new(18);
        let input = vec![0.0; 18];
        let mut output = vec![0.0; 12];

        imdct.process(&input, &mut output, 2);

        assert!(output.iter().all(|&x| x.abs() < 0.001));
    }

    #[test]
    fn windows_normal_block() {
        let imdct = Mp3Imdct::new(18);
        let mut samples = vec![1.0; 36];

        imdct.window(&mut samples, 0);

        assert!(samples[0] > 0.0 && samples[0] < 1.0);
        assert!(samples[18] > 0.9);
    }
}
