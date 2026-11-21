use rustfft::{num_complex::Complex, FftPlanner};

const LEVEL_FLOOR: f32 = 60.0;
const LEVEL_FULL: f32 = 2600.0;

const VIS_WIN: usize = 1024;
const VIS_HOP: usize = 480;

pub fn pcm_level(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    let rms = (sum_sq / samples.len() as f64).sqrt() as f32;
    if rms <= LEVEL_FLOOR {
        return 0.0;
    }
    ((rms - LEVEL_FLOOR) / (LEVEL_FULL - LEVEL_FLOOR)).min(1.0)
}

pub fn pcm_visemes(samples: &[i16], sr: u32) -> Vec<(f32, f32, f32)> {
    if samples.len() < VIS_WIN {
        return Vec::new();
    }

    let sr_f = sr as f32;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(VIS_WIN);

    let hanning: Vec<f32> = (0..VIS_WIN)
        .map(|i| {
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (VIS_WIN - 1) as f32).cos())
        })
        .collect();

    let freq_bin = |f: f32| -> usize { (f * VIS_WIN as f32 / sr_f).round() as usize };

    let f1_lo = (freq_bin(150.0), freq_bin(450.0));
    let f1_hi = (freq_bin(450.0), freq_bin(1100.0));
    let f2_bk = (freq_bin(600.0), freq_bin(1300.0));
    let f2_fr = (freq_bin(1700.0), freq_bin(3200.0));
    let hiss = (freq_bin(3800.0), freq_bin(8000.0));

    let mut out = Vec::new();
    let mut start = 0usize;

    while start < samples.len() {
        let hop_end = (start + VIS_HOP).min(samples.len());
        let hop_slice = &samples[start..hop_end];
        let level = pcm_level(hop_slice);

        if level <= 0.0 {
            out.push((0.0, 0.0, 0.0));
            start += VIS_HOP;
            continue;
        }

        let seg_end = (start + VIS_WIN).min(samples.len());
        let mut buf: Vec<Complex<f32>> = Vec::with_capacity(VIS_WIN);
        for i in 0..VIS_WIN {
            let s = if start + i < seg_end {
                samples[start + i] as f32
            } else {
                0.0
            };
            buf.push(Complex::new(s, 0.0));
        }

        let mean: f32 = buf.iter().map(|c| c.re).sum::<f32>() / VIS_WIN as f32;
        for (i, c) in buf.iter_mut().enumerate() {
            c.re = (c.re - mean) * hanning[i];
        }

        fft.process(&mut buf);

        let mag: Vec<f32> = buf.iter().take(VIS_WIN / 2 + 1).map(|c| c.norm()).collect();

        let band_sum = |range: (usize, usize)| -> f32 {
            let (lo, hi) = range;
            let hi = hi.min(mag.len());
            if lo >= hi {
                return 0.0;
            }
            mag[lo..hi].iter().sum()
        };

        let f1l = band_sum(f1_lo);
        let f1h = band_sum(f1_hi);
        let f2b = band_sum(f2_bk);
        let f2f = band_sum(f2_fr);
        let h_val = band_sum(hiss);

        let mut openness = f1h / (f1l + f1h + 1e-6);
        let mut width = (f2f - f2b) / (f2f + f2b + 1e-6);
        width *= (1.0 - openness).powf(0.8);
        let h = h_val / (f1l + f1h + f2b + f2f + h_val + 1e-6);
        openness *= 1.0 - 0.65 * (h * 2.5).min(1.0);

        out.push((
            level,
            openness.clamp(0.0, 1.0),
            width.clamp(-1.0, 1.0),
        ));

        start += VIS_HOP;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_returns_zero() {
        let silence = vec![0i16; 1024];
        assert_eq!(pcm_level(&silence), 0.0);
    }

    #[test]
    fn loud_signal_near_one() {
        let loud: Vec<i16> = (0..1024).map(|_| 10000).collect();
        let level = pcm_level(&loud);
        assert!(level > 0.9, "expected near 1.0, got {level}");
    }

    #[test]
    fn visemes_on_short_input() {
        let short = vec![0i16; 100];
        assert!(pcm_visemes(&short, 24000).is_empty());
    }
}
