//! Analyse caméra : rythme cardiaque estimé à distance (rPPG, méthode CHROM)
//! à partir des couleurs moyennes de la zone du visage, et agitation (mouvement).
//!
//! L'interface envoie ~30 échantillons/s : couleur moyenne de la zone ovale
//! centrale et énergie de mouvement entre deux images. Aucune image ne quitte
//! l'interface.

use rustfft::{num_complex::Complex32, FftPlanner};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct VideoSample {
    /// Horodatage en secondes.
    pub t: f64,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    /// Différence moyenne de luminance avec l'image précédente (0-255).
    pub motion: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct VideoFeatures {
    /// Battements par minute, si le signal est exploitable.
    pub heart_rate: Option<f32>,
    /// Part de l'énergie du spectre portée par le pic cardiaque (0-1).
    pub hr_quality: f32,
    pub motion_mean: f32,
    pub motion_p90: f32,
    /// Luminosité moyenne de la zone du visage (0-255).
    pub luma: f32,
    pub duration_s: f32,
}

const FS: f64 = 30.0;

fn interp(samples: &[VideoSample], t: f64, ch: impl Fn(&VideoSample) -> f32, hint: &mut usize) -> f32 {
    while *hint + 1 < samples.len() && samples[*hint + 1].t < t {
        *hint += 1;
    }
    let a = &samples[*hint];
    let b = &samples[(*hint + 1).min(samples.len() - 1)];
    if b.t <= a.t {
        return ch(a);
    }
    let w = ((t - a.t) / (b.t - a.t)).clamp(0.0, 1.0) as f32;
    ch(a) + (ch(b) - ch(a)) * w
}

fn moving_mean_normalize(v: &[f32], win: usize) -> Vec<f32> {
    let n = v.len();
    let mut prefix = vec![0.0f64; n + 1];
    for i in 0..n {
        prefix[i + 1] = prefix[i] + v[i] as f64;
    }
    (0..n)
        .map(|i| {
            let lo = i.saturating_sub(win / 2);
            let hi = (i + win / 2 + 1).min(n);
            let m = ((prefix[hi] - prefix[lo]) / (hi - lo) as f64) as f32;
            if m.abs() > 1e-3 { v[i] / m - 1.0 } else { 0.0 }
        })
        .collect()
}

fn std_dev(v: &[f32]) -> f32 {
    let m = v.iter().sum::<f32>() / v.len().max(1) as f32;
    (v.iter().map(|x| (x - m).powi(2)).sum::<f32>() / v.len().max(1) as f32).sqrt()
}

/// Estime (bpm, qualité) par rPPG sur des échantillons triés.
fn heart_rate(samples: &[VideoSample]) -> (Option<f32>, f32) {
    let dur = samples.last().unwrap().t - samples[0].t;
    if dur < 6.0 || samples.len() < 60 {
        return (None, 0.0);
    }
    let n = (dur * FS) as usize;
    let t0 = samples[0].t;
    let (mut hr_, mut hg, mut hb) = (0usize, 0usize, 0usize);
    let r: Vec<f32> = (0..n).map(|i| interp(samples, t0 + i as f64 / FS, |s| s.r, &mut hr_)).collect();
    let g: Vec<f32> = (0..n).map(|i| interp(samples, t0 + i as f64 / FS, |s| s.g, &mut hg)).collect();
    let b: Vec<f32> = (0..n).map(|i| interp(samples, t0 + i as f64 / FS, |s| s.b, &mut hb)).collect();
    let win = FS as usize;
    let (rn, gn, bn) = (moving_mean_normalize(&r, win), moving_mean_normalize(&g, win), moving_mean_normalize(&b, win));
    let xs: Vec<f32> = (0..n).map(|i| 3.0 * rn[i] - 2.0 * gn[i]).collect();
    let ys: Vec<f32> = (0..n).map(|i| 1.5 * rn[i] + gn[i] - 1.5 * bn[i]).collect();
    let sy = std_dev(&ys);
    let alpha = if sy > 1e-9 { std_dev(&xs) / sy } else { 1.0 };
    let mut s: Vec<f32> = (0..n).map(|i| xs[i] - alpha * ys[i]).collect();
    let m = s.iter().sum::<f32>() / n as f32;
    s.iter_mut().for_each(|v| *v -= m);

    let n_fft = n.next_power_of_two().max(2048);
    let mut buf: Vec<Complex32> = (0..n_fft)
        .map(|i| {
            if i < n {
                let w = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos();
                Complex32::new(s[i] * w, 0.0)
            } else {
                Complex32::new(0.0, 0.0)
            }
        })
        .collect();
    FftPlanner::<f32>::new().plan_fft_forward(n_fft).process(&mut buf);
    let hz = |k: usize| k as f64 * FS / n_fft as f64;
    let band: Vec<usize> = (1..n_fft / 2).filter(|k| (0.75..=3.0).contains(&hz(*k))).collect();
    let power: Vec<f32> = band.iter().map(|k| buf[*k].norm_sqr()).collect();
    let total: f32 = power.iter().sum();
    if total <= 0.0 || power.len() < 3 {
        return (None, 0.0);
    }
    let (pi, _) = power
        .iter()
        .enumerate()
        .fold((0, f32::MIN), |acc, (i, p)| if *p > acc.1 { (i, *p) } else { acc });
    // Le pic à zéro-padding s'étale : on somme la puissance autour (±0.1 Hz).
    let spread = ((0.1 * n_fft as f64 / FS) as usize).max(1);
    let lo = pi.saturating_sub(spread);
    let hi = (pi + spread + 1).min(power.len());
    let quality = power[lo..hi].iter().sum::<f32>() / total;
    let shift = if pi > 0 && pi + 1 < power.len() {
        let (a, b, c) = (power[pi - 1], power[pi], power[pi + 1]);
        let d = a - 2.0 * b + c;
        if d.abs() > 1e-12 { (0.5 * (a - c) / d).clamp(-1.0, 1.0) } else { 0.0 }
    } else {
        0.0
    };
    let bpm = (hz(band[pi]) + shift as f64 * FS / n_fft as f64) * 60.0;
    let usable = quality >= 0.25;
    (usable.then_some(bpm as f32), quality)
}

pub fn analyze(samples: &[VideoSample]) -> VideoFeatures {
    let mut s: Vec<VideoSample> = samples
        .iter()
        .copied()
        .filter(|x| x.t.is_finite() && [x.r, x.g, x.b, x.motion].iter().all(|v| v.is_finite() && (0.0..=255.0).contains(v)))
        .collect();
    if s.len() < 2 {
        return VideoFeatures::default();
    }
    s.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    let mut motion: Vec<f32> = s.iter().map(|x| x.motion).collect();
    let motion_mean = motion.iter().sum::<f32>() / motion.len() as f32;
    motion.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let motion_p90 = motion[((motion.len() - 1) as f32 * 0.9) as usize];
    let luma = s.iter().map(|x| 0.299 * x.r + 0.587 * x.g + 0.114 * x.b).sum::<f32>() / s.len() as f32;
    let (heart_rate, hr_quality) = heart_rate(&s);
    VideoFeatures {
        heart_rate,
        hr_quality,
        motion_mean,
        motion_p90,
        luma,
        duration_s: (s.last().unwrap().t - s[0].t) as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_synthetic_pulse() {
        // Pouls à 72 bpm (1.2 Hz) noyé dans de petites variations d'éclairage, 29 fps irréguliers.
        let mut samples = Vec::new();
        let mut t = 0.0f64;
        let mut seed = 7u32;
        while t < 15.0 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let noise = ((seed >> 16) as f32 / 65536.0 - 0.5) * 0.3;
            let pulse = (2.0 * std::f64::consts::PI * 1.2 * t).sin() as f32;
            let light = 3.0 * (2.0 * std::f64::consts::PI * 0.1 * t).sin() as f32;
            samples.push(VideoSample {
                t,
                r: 150.0 + light + 0.35 * pulse + noise,
                g: 110.0 + light + 0.8 * pulse + noise,
                b: 90.0 + light + 0.2 * pulse + noise,
                motion: 1.0,
            });
            t += 1.0 / 29.0 + if seed.is_multiple_of(3) { 0.004 } else { 0.0 };
        }
        let f = analyze(&samples);
        let hr = f.heart_rate.expect("pouls détecté");
        assert!((hr - 72.0).abs() < 4.0, "bpm {hr}, qualité {}", f.hr_quality);
    }

    #[test]
    fn short_clip_has_no_heart_rate() {
        let samples: Vec<VideoSample> =
            (0..60).map(|i| VideoSample { t: i as f64 / 30.0, r: 100.0, g: 100.0, b: 100.0, motion: 2.0 }).collect();
        let f = analyze(&samples);
        assert!(f.heart_rate.is_none());
        assert!((f.motion_mean - 2.0).abs() < 1e-6);
    }
}
