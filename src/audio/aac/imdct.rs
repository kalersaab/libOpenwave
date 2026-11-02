use std::f32::consts::PI;

pub struct Imdct {
    n: usize,
    _cos_tab: Vec<f32>,
    _sin_tab: Vec<f32>,
}

impl Imdct {
    pub fn new(n: usize) -> Self {
        let n2 = n / 2;
        let _n4 = n / 4;

        let mut cos_tab = vec![0.0; _n4];
        let mut sin_tab = vec![0.0; _n4];

        for i in 0.._n4 {
            let angle = PI * (i as f32 + 0.5) / (n2 as f32);
            cos_tab[i] = angle.cos();
            sin_tab[i] = angle.sin();
        }

        Imdct {
            n,
            _cos_tab: cos_tab,
            _sin_tab: sin_tab,
        }
    }

    pub fn process(&self, input: &[f32], output: &mut [f32]) {
        assert_eq!(input.len(), self.n / 2);
        assert_eq!(output.len(), self.n);

        let n = self.n;
        let n2 = n / 2;
        let _n4 = n / 4;

        for (i, output_sample) in output.iter_mut().enumerate() {
            let mut sum = 0.0;

            for (k, &input_val) in input.iter().enumerate().take(n2) {
                let angle = PI / n as f32 * (i as f32 + 0.5 + n2 as f32 / 2.0) * (k as f32 + 0.5);
                sum += input_val * angle.cos();
            }

            *output_sample = sum * (2.0 / n as f32).sqrt();
        }
    }

    pub fn process_long(&self, input: &[f32], output: &mut [f32], prev_overlap: &[f32]) {
        assert_eq!(self.n, 1024);

        let mut temp = vec![0.0; 1024];
        self.process(input, &mut temp);

        for i in 0..512 {
            output[i] = temp[i] * Self::window_long(i, false) + prev_overlap[i];
        }

        for i in 512..1024 {
            output[i] = temp[i] * Self::window_long(i, true);
        }
    }

    pub fn process_short(&self, input: &[&[f32]; 8], output: &mut [f32], prev_overlap: &[f32]) {
        assert_eq!(self.n, 128);

        let mut temp_outputs = [[0.0f32; 128]; 8];

        for (i, &inp) in input.iter().enumerate() {
            self.process(inp, &mut temp_outputs[i]);
        }

        output[..512].copy_from_slice(prev_overlap);

        for (win_idx, temp) in temp_outputs.iter().enumerate() {
            let offset = 448 + win_idx * 128;

            for i in 0..128 {
                if offset + i < 1024 {
                    output[offset + i] += temp[i] * Self::window_short(i);
                }
            }
        }
    }

    fn window_long(i: usize, second_half: bool) -> f32 {
        if !second_half {
            ((i as f32 + 0.5) * PI / 1024.0).sin()
        } else {
            ((i as f32 - 511.5) * PI / 1024.0).cos()
        }
    }

    fn window_short(i: usize) -> f32 {
        ((i as f32 + 0.5) * PI / 128.0).sin()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_imdct_1024() {
        let imdct = Imdct::new(1024);
        assert_eq!(imdct.n, 1024);
        assert_eq!(imdct._cos_tab.len(), 256);
    }

    #[test]
    fn creates_imdct_128() {
        let imdct = Imdct::new(128);
        assert_eq!(imdct.n, 128);
        assert_eq!(imdct._cos_tab.len(), 32);
    }

    #[test]
    fn processes_impulse() {
        let imdct = Imdct::new(128);
        let mut input = vec![0.0; 64];
        input[0] = 1.0;

        let mut output = vec![0.0; 128];
        imdct.process(&input, &mut output);

        assert!(!output.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn window_long_symmetric() {
        let w1 = Imdct::window_long(0, false);
        let w2 = Imdct::window_long(1023, true);

        assert!((w1 - w2).abs() < 0.001);
    }

    #[test]
    fn window_short_symmetric() {
        let w1 = Imdct::window_short(0);
        let w2 = Imdct::window_short(127);

        assert!((w1 - w2).abs() < 0.001);
    }
}
