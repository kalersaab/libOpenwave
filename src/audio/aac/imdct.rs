// Symphonia AAC-LC decoder port.
// Copyright (c) 2019-2026 The Project Symphonia Developers.
// This file is licensed under the Mozilla Public License, v. 2.0.

use std::f32::consts::PI;

#[derive(Clone, Copy, Default)]
struct Complex {
    re: f32,
    im: f32,
}

impl Complex {
    fn new(re: f32, im: f32) -> Self {
        Complex { re, im }
    }

    fn mul(self, other: Self) -> Self {
        Complex {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }

    fn add(self, other: Self) -> Self {
        Complex {
            re: self.re + other.re,
            im: self.im + other.im,
        }
    }

    fn sub(self, other: Self) -> Self {
        Complex {
            re: self.re - other.re,
            im: self.im - other.im,
        }
    }

    fn conj(self) -> Self {
        Complex {
            re: self.re,
            im: -self.im,
        }
    }
}

struct Fft {
    n: usize,
    perm: Vec<usize>,
    twiddles: Vec<Complex>,
}

impl Fft {
    fn new(n: usize) -> Self {
        assert!(n.is_power_of_two());
        let bits = n.trailing_zeros() as usize;
        let shift = usize::BITS as usize - bits;
        let perm = (0..n).map(|i| i.reverse_bits() >> shift).collect();
        let twiddles = (0..n / 2)
            .map(|k| {
                let angle = 2.0 * PI * k as f32 / n as f32;
                Complex::new(angle.cos(), -angle.sin())
            })
            .collect();
        Fft { n, perm, twiddles }
    }

    fn fft_inplace(&self, x: &mut [Complex]) {
        assert_eq!(x.len(), self.n);
        for (i, &j) in self.perm.iter().enumerate() {
            if i < j {
                x.swap(i, j);
            }
        }

        let mut len = 2;
        while len <= self.n {
            let step = self.n / len;
            for start in (0..self.n).step_by(len) {
                for k in 0..len / 2 {
                    let w = self.twiddles[k * step];
                    let u = x[start + k];
                    let v = x[start + k + len / 2].mul(w);
                    x[start + k] = u.add(v);
                    x[start + k + len / 2] = u.sub(v);
                }
            }
            len <<= 1;
        }
    }
}

pub struct Imdct {
    n: usize,
    fft: Fft,
    scratch: Vec<Complex>,
    twiddle: Vec<Complex>,
}

impl Imdct {
    pub fn new(n: usize) -> Self {
        Imdct::new_scaled(n, 1.0)
    }

    pub fn new_scaled(n: usize, scale: f64) -> Self {
        assert!(n.is_power_of_two());
        let n2 = n / 2;
        let mut twiddle = Vec::with_capacity(n2);

        let alpha = 1.0 / 8.0
            + if scale.is_sign_positive() {
                0.0
            } else {
                n2 as f64
            };
        let pi_n = PI as f64 / n as f64;
        let sqrt_scale = scale.abs().sqrt();

        for k in 0..n2 {
            let theta = pi_n * (alpha + k as f64);
            let re = sqrt_scale * theta.cos();
            let im = sqrt_scale * theta.sin();
            twiddle.push(Complex::new(re as f32, im as f32));
        }

        let scratch = vec![Complex::default(); n2];

        Imdct {
            n,
            fft: Fft::new(n2),
            scratch,
            twiddle,
        }
    }

    pub fn imdct(&mut self, spec: &[f32], out: &mut [f32]) {
        let n = self.n;
        let n2 = n >> 1;
        let n4 = n >> 2;

        assert_eq!(spec.len(), n);
        assert_eq!(out.len(), 2 * n);

        for (i, (&w, t)) in self.twiddle.iter().zip(self.scratch.iter_mut()).enumerate() {
            let even = spec[i * 2];
            let odd = -spec[n - 1 - i * 2];

            let re = odd * w.im - even * w.re;
            let im = odd * w.re + even * w.im;
            *t = Complex::new(re, im);
        }

        self.fft.fft_inplace(&mut self.scratch);

        let (vec0, vec1) = out.split_at_mut(n2);
        let (vec1, vec2) = vec1.split_at_mut(n2);
        let (vec2, vec3) = vec2.split_at_mut(n2);

        for (i, (x, &w)) in self.scratch[..n4]
            .iter()
            .zip(self.twiddle[..n4].iter())
            .enumerate()
        {
            let val = w.mul(x.conj());
            let fi = 2 * i;
            let ri = n2 - 1 - 2 * i;

            vec0[ri] = -val.im;
            vec1[fi] = val.im;
            vec2[ri] = val.re;
            vec3[fi] = val.re;
        }

        for (i, (x, &w)) in self.scratch[n4..]
            .iter()
            .zip(self.twiddle[n4..].iter())
            .enumerate()
        {
            let val = w.mul(x.conj());
            let fi = 2 * i;
            let ri = n2 - 1 - 2 * i;

            vec0[fi] = -val.re;
            vec1[ri] = val.re;
            vec2[fi] = val.im;
            vec3[ri] = val.im;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imdct_analytical(x: &[f32], y: &mut [f32], scale: f64) {
        assert!(y.len() == 2 * x.len());
        let n_in = x.len();
        let n_out = x.len() << 1;
        let pi_2n = std::f64::consts::PI / (2 * n_out) as f64;
        for (i, item) in y.iter_mut().enumerate().take(n_out) {
            let accum: f64 = x
                .iter()
                .copied()
                .map(f64::from)
                .enumerate()
                .map(|(j, jtem)| jtem * (pi_2n * ((2 * i + 1 + n_in) * (2 * j + 1)) as f64).cos())
                .sum();
            *item = (scale * accum) as f32;
        }
    }

    #[test]
    fn verify_imdct() {
        let test: Vec<f32> = (1..=32).map(|v| v as f32).collect();
        let mut actual = [0f32; 64];
        let mut expected = [0f32; 64];
        let scale = (2.0f64 / 64.0).sqrt();
        imdct_analytical(&test, &mut expected, scale);
        let mut mdct = Imdct::new_scaled(32, scale);
        mdct.imdct(&test, &mut actual);
        for i in 0..64 {
            let delta = f64::from(actual[i]) - f64::from(expected[i]);
            assert!(delta.abs() < 0.00001);
        }
    }
}
