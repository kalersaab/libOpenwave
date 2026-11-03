use std::f32::consts::PI;

pub struct Mp3Imdct {
    cos_long: Vec<Vec<f32>>,
    cos_short: Vec<Vec<f32>>,
}

impl Mp3Imdct {
    pub fn new() -> Self {
        let mut cos_long = vec![vec![0.0f32; 18]; 36];
        let mut cos_short = vec![vec![0.0f32; 6]; 12];

        for i in 0..36 {
            for k in 0..18 {
                let angle = PI / 36.0 * ((i as f32) + 9.0 + 0.5) * ((k as f32) + 0.5);
                cos_long[i][k] = angle.cos();
            }
        }

        for i in 0..12 {
            for k in 0..6 {
                let angle = PI / 12.0 * ((i as f32) + 3.0 + 0.5) * ((k as f32) + 0.5);
                cos_short[i][k] = angle.cos();
            }
        }

        Mp3Imdct {
            cos_long,
            cos_short,
        }
    }

    pub fn imdct_long(&self, input: &[f32], output: &mut [f32]) {
        assert!(input.len() >= 18);
        assert!(output.len() >= 36);

        for i in 0..36 {
            let mut sum = 0.0;
            for k in 0..18 {
                sum += input[k] * self.cos_long[i][k];
            }
            output[i] = sum;
        }
    }

    pub fn imdct_short(&self, input: &[f32], output: &mut [f32]) {
        assert!(input.len() >= 6);
        assert!(output.len() >= 12);

        for i in 0..12 {
            let mut sum = 0.0;
            for k in 0..6 {
                sum += input[k] * self.cos_short[i][k];
            }
            output[i] = sum;
        }
    }

    pub fn window(&self, samples: &mut [f32], block_type: u8) {
        match block_type {
            0 => self.window_normal(samples),
            1 => self.window_start(samples),
            2 => self.window_short(samples),
            3 => self.window_stop(samples),
            _ => self.window_normal(samples),
        }
    }

    fn window_normal(&self, samples: &mut [f32]) {
        for i in 0..36 {
            let angle = PI / 36.0 * (i as f32 + 0.5);
            samples[i] *= angle.sin();
        }
    }

    fn window_start(&self, samples: &mut [f32]) {
        for i in 0..18 {
            let angle = PI / 36.0 * (i as f32 + 0.5);
            samples[i] *= angle.sin();
        }

        for i in 0..6 {
            let angle = PI / 12.0 * (i as f32 + 0.5);
            samples[24 + i] *= angle.sin();
        }
        for i in 0..6 {
            samples[30 + i] = 0.0;
        }
    }

    fn window_short(&self, samples: &mut [f32]) {
        for i in 0..12 {
            let angle = PI / 12.0 * (i as f32 + 0.5);
            samples[i] *= angle.sin();
        }
    }

    fn window_stop(&self, samples: &mut [f32]) {
        for i in 0..6 {
            samples[i] = 0.0;
        }
        for i in 0..6 {
            let angle = PI / 12.0 * (i as f32 + 0.5);
            samples[6 + i] *= angle.sin();
        }
        for i in 0..18 {
            let angle = PI / 36.0 * (i as f32 + 0.5);
            samples[18 + i] *= angle.sin();
        }
    }
}

impl Default for Mp3Imdct {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_imdct() {
        let imdct = Mp3Imdct::new();
        assert_eq!(imdct.cos_long.len(), 36);
        assert_eq!(imdct.cos_short.len(), 12);
    }

    #[test]
    fn processes_long_block() {
        let imdct = Mp3Imdct::new();
        let input = vec![0.0; 18];
        let mut output = vec![0.0; 36];

        imdct.imdct_long(&input, &mut output);

        assert!(output.iter().all(|&x| x.abs() < 0.001));
    }

    #[test]
    fn processes_short_block() {
        let imdct = Mp3Imdct::new();
        let input = vec![0.0; 6];
        let mut output = vec![0.0; 12];

        imdct.imdct_short(&input, &mut output);

        assert!(output.iter().all(|&x| x.abs() < 0.001));
    }

    #[test]
    fn windows_normal_block() {
        let imdct = Mp3Imdct::new();
        let mut samples = vec![1.0; 36];

        imdct.window(&mut samples, 0);

        assert!(samples[0] > 0.0 && samples[0] < 1.0);
        assert!(samples[18] > 0.9);
    }
}
