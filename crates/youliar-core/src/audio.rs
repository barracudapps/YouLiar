//! Analyse de la voix : détection d'activité vocale, latence de réponse,
//! hauteur (YIN), micro-variations (jitter / shimmer), pauses, débit,
//! timbre (centroïde spectral, MFCC pour l'empreinte vocale).

use rustfft::{num_complex::Complex32, FftPlanner};
use serde::{Deserialize, Serialize};

/// Fréquence d'échantillonnage interne.
pub const SR: u32 = 16_000;
const FRAME: usize = 400; // 25 ms
const HOP: usize = 160; // 10 ms
const HOP_S: f32 = HOP as f32 / SR as f32;
/// La hauteur est estimée sur un signal décimé à 8 kHz (≈ 4× moins de calcul,
/// largement suffisant pour une fondamentale ≤ 450 Hz).
const YIN_SR: f32 = 8_000.0;
const YIN_W: usize = 320; // fenêtre d'intégration YIN (40 ms à 8 kHz)
const F0_MIN: f32 = 70.0;
const F0_MAX: f32 = 450.0;
const N_FFT: usize = 512;
const N_MEL: usize = 26;
pub const N_MFCC: usize = 12;

/// Mesures extraites d'un enregistrement de réponse.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AudioFeatures {
    pub speech_detected: bool,
    /// Délai avant le début de la parole (s), délai de consigne inclus.
    pub latency_s: f32,
    /// Durée entre le premier et le dernier segment de parole (s).
    pub duration_s: f32,
    /// Hauteur moyenne en demi-tons au-dessus de 55 Hz.
    pub f0_mean_st: f32,
    /// Écart-type de la hauteur (demi-tons) : intonation.
    pub f0_std_st: f32,
    /// Instabilité relative de la période d'une trame voisée à l'autre.
    pub jitter: f32,
    /// Instabilité relative de l'amplitude d'une trame voisée à l'autre.
    pub shimmer: f32,
    /// Énergie moyenne de la parole (dBFS).
    pub energy_db: f32,
    /// Part de silence à l'intérieur de la réponse.
    pub pause_ratio: f32,
    /// Nombre de pauses de plus de 250 ms dans la réponse.
    pub hesitations: f32,
    /// Noyaux syllabiques par seconde de parole.
    pub speech_rate: f32,
    pub spectral_centroid: f32,
    pub voiced_ratio: f32,
    /// MFCC moyens (c1..c12) : empreinte du timbre.
    pub mfcc: Vec<f32>,
}

/// Ramène le signal à 16 kHz (moyenne glissante anti-repliement + interpolation linéaire).
pub fn resample(input: &[f32], sr: u32) -> Vec<f32> {
    if sr == SR || input.is_empty() {
        return input.to_vec();
    }
    let ratio = sr as f64 / SR as f64;
    let smoothed: Vec<f32> = if ratio > 1.0 {
        let k = ratio.round().max(1.0) as usize;
        let mut out = Vec::with_capacity(input.len());
        let mut acc = 0.0f32;
        for i in 0..input.len() {
            acc += input[i];
            if i >= k {
                acc -= input[i - k];
            }
            out.push(acc / (i + 1).min(k) as f32);
        }
        out
    } else {
        input.to_vec()
    };
    let n_out = ((input.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(n_out);
    for i in 0..n_out {
        let pos = i as f64 * ratio;
        let j = pos.floor() as usize;
        let frac = (pos - j as f64) as f32;
        let a = smoothed[j.min(smoothed.len() - 1)];
        let b = smoothed[(j + 1).min(smoothed.len() - 1)];
        out.push(a + (b - a) * frac);
    }
    out
}

fn percentile(values: &[f32], p: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((v.len() - 1) as f32 * p).round() as usize;
    v[idx]
}

fn mean(v: &[f32]) -> f32 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f32>() / v.len() as f32
    }
}

fn std_dev(v: &[f32]) -> f32 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = mean(v);
    (v.iter().map(|x| (x - m).powi(2)).sum::<f32>() / (v.len() - 1) as f32).sqrt()
}

/// Estimation de la période fondamentale (en échantillons) par l'algorithme YIN.
/// Décimation 16 kHz → 8 kHz avec un filtre binomial [1 4 6 4 1]/16.
fn decimate2(x: &[f32]) -> Vec<f32> {
    let at = |i: isize| x[i.clamp(0, x.len() as isize - 1) as usize];
    (0..x.len() / 2)
        .map(|k| {
            let i = (2 * k) as isize;
            (at(i - 2) + 4.0 * at(i - 1) + 6.0 * at(i) + 4.0 * at(i + 1) + at(i + 2)) / 16.0
        })
        .collect()
}

/// Période fondamentale (en échantillons à `YIN_SR`). `d` et `cmnd` sont des tampons réutilisés.
fn yin_period(x: &[f32], start: usize, d: &mut [f32], cmnd: &mut [f32]) -> Option<f32> {
    let min_tau = (YIN_SR / F0_MAX) as usize;
    let max_tau = (YIN_SR / F0_MIN) as usize;
    if start + YIN_W + max_tau + 1 > x.len() {
        return None;
    }
    let seg = &x[start..start + YIN_W + max_tau + 1];
    let head = &seg[..YIN_W];
    for tau in 1..=max_tau + 1 {
        // Boucle sans indexation : vectorisée par le compilateur.
        d[tau] = head.iter().zip(&seg[tau..tau + YIN_W]).map(|(a, b)| (a - b) * (a - b)).sum();
    }
    // Différence moyenne normalisée cumulée.
    cmnd.iter_mut().for_each(|c| *c = 1.0);
    let mut running = 0.0f32;
    for tau in 1..=max_tau + 1 {
        running += d[tau];
        cmnd[tau] = if running > 0.0 { d[tau] * tau as f32 / running } else { 1.0 };
    }
    let mut best: Option<usize> = None;
    let mut tau = min_tau;
    while tau <= max_tau {
        if cmnd[tau] < 0.15 {
            while tau < max_tau && cmnd[tau + 1] < cmnd[tau] {
                tau += 1;
            }
            best = Some(tau);
            break;
        }
        tau += 1;
    }
    if best.is_none() {
        let (idx, val) = (min_tau..=max_tau)
            .map(|t| (t, cmnd[t]))
            .fold((min_tau, f32::MAX), |acc, x| if x.1 < acc.1 { x } else { acc });
        if val < 0.3 {
            best = Some(idx);
        }
    }
    let t = best?;
    // Interpolation parabolique autour du minimum.
    let (a, b, c) = (cmnd[t - 1], cmnd[t], cmnd[t + 1]);
    let denom = a - 2.0 * b + c;
    let shift = if denom.abs() > 1e-9 { 0.5 * (a - c) / denom } else { 0.0 };
    Some(t as f32 + shift.clamp(-1.0, 1.0))
}

fn mel(f: f32) -> f32 {
    2595.0 * (1.0 + f / 700.0).log10()
}

fn mel_inv(m: f32) -> f32 {
    700.0 * (10f32.powf(m / 2595.0) - 1.0)
}

fn mel_filterbank() -> Vec<Vec<(usize, f32)>> {
    let n_bins = N_FFT / 2 + 1;
    let (lo, hi) = (mel(80.0), mel(7600.0));
    let pts: Vec<f32> = (0..N_MEL + 2)
        .map(|i| mel_inv(lo + (hi - lo) * i as f32 / (N_MEL + 1) as f32))
        .collect();
    let bin = |f: f32| f * N_FFT as f32 / SR as f32;
    (0..N_MEL)
        .map(|m| {
            let (l, c, r) = (bin(pts[m]), bin(pts[m + 1]), bin(pts[m + 2]));
            (0..n_bins)
                .filter_map(|k| {
                    let k_f = k as f32;
                    let w = if k_f > l && k_f <= c {
                        (k_f - l) / (c - l)
                    } else if k_f > c && k_f < r {
                        (r - k_f) / (r - c)
                    } else {
                        0.0
                    };
                    (w > 0.0).then_some((k, w))
                })
                .collect()
        })
        .collect()
}

/// Analyse un enregistrement mono. `prompt_offset_s` est le temps écoulé entre la fin
/// de la question et le début de l'enregistrement (ajouté à la latence).
pub fn analyze(samples: &[f32], sample_rate: u32, prompt_offset_s: f32) -> AudioFeatures {
    let mut x = resample(samples, sample_rate);
    let m = mean(&x);
    x.iter_mut().for_each(|s| *s -= m);
    let total_s = x.len() as f32 / SR as f32;
    if x.len() < FRAME + 2 * YIN_W {
        return AudioFeatures { latency_s: prompt_offset_s + total_s, ..Default::default() };
    }
    let n_frames = (x.len() - FRAME) / HOP + 1;

    let rms: Vec<f32> = (0..n_frames)
        .map(|i| {
            let f = &x[i * HOP..i * HOP + FRAME];
            (f.iter().map(|s| s * s).sum::<f32>() / FRAME as f32).sqrt()
        })
        .collect();
    let db: Vec<f32> = rms.iter().map(|r| 20.0 * (r + 1e-5).log10()).collect();

    // --- Détection d'activité vocale ---
    let noise = percentile(&db, 0.10);
    let peak = percentile(&db, 0.98);
    let thr = (noise + 10.0).max(-58.0).min(peak - 8.0);
    let mut speech: Vec<bool> = db.iter().map(|d| *d > thr).collect();
    if peak < -55.0 {
        speech.iter_mut().for_each(|s| *s = false);
    }
    // Supprime les rafales trop courtes (clics, souffles).
    let mut i = 0;
    while i < n_frames {
        if speech[i] {
            let start = i;
            while i < n_frames && speech[i] {
                i += 1;
            }
            if i - start < 5 {
                speech[start..i].iter_mut().for_each(|s| *s = false);
            }
        } else {
            i += 1;
        }
    }
    let onset = speech.iter().position(|s| *s);
    let offset = speech.iter().rposition(|s| *s);
    let (onset, offset) = match (onset, offset) {
        (Some(a), Some(b)) => (a, b),
        _ => return AudioFeatures { latency_s: prompt_offset_s + total_s, ..Default::default() },
    };

    // --- Pauses et hésitations ---
    let span = offset - onset + 1;
    let mut pause_frames = 0usize;
    let mut hesitations = 0usize;
    let mut run = 0usize;
    for s in &speech[onset..=offset] {
        if *s {
            if run >= 25 {
                hesitations += 1;
            }
            run = 0;
        } else {
            run += 1;
            pause_frames += 1;
        }
    }

    // --- Hauteur, jitter, shimmer ---
    let x8 = decimate2(&x);
    let half8 = (YIN_W - FRAME / 2) / 2;
    let buf_len = (YIN_SR / F0_MIN) as usize + 2;
    let (mut d_buf, mut c_buf) = (vec![0.0f32; buf_len], vec![1.0f32; buf_len]);
    let mut periods: Vec<Option<f32>> = vec![None; n_frames];
    for (idx, p) in periods.iter_mut().enumerate().take(offset + 1).skip(onset) {
        if speech[idx] {
            *p = yin_period(&x8, (idx * HOP / 2).saturating_sub(half8), &mut d_buf, &mut c_buf);
        }
    }
    let f0s: Vec<f32> = periods.iter().flatten().map(|t| YIN_SR / t).collect();
    let med = percentile(&f0s, 0.5);
    for p in periods.iter_mut() {
        if let Some(t) = *p {
            let f = YIN_SR / t;
            if f < med / 1.8 || f > med * 1.8 {
                *p = None; // erreurs d'octave
            }
        }
    }
    let st: Vec<f32> = periods.iter().flatten().map(|t| 12.0 * (YIN_SR / t / 55.0).log2()).collect();
    let voiced_periods: Vec<f32> = periods.iter().flatten().copied().collect();
    let mean_period = mean(&voiced_periods);
    let mut jit = Vec::new();
    let mut shim = Vec::new();
    for idx in 1..n_frames {
        if let (Some(a), Some(b)) = (periods[idx - 1], periods[idx]) {
            jit.push((b - a).abs());
            shim.push((rms[idx] - rms[idx - 1]).abs());
        }
    }
    let voiced_amp: Vec<f32> = (0..n_frames).filter(|i| periods[*i].is_some()).map(|i| rms[i]).collect();
    let jitter = if mean_period > 0.0 { mean(&jit) / mean_period } else { 0.0 };
    let shimmer = {
        let ma = mean(&voiced_amp);
        if ma > 0.0 { mean(&shim) / ma } else { 0.0 }
    };

    // --- Débit : pics d'énergie voisés (noyaux syllabiques) ---
    let env: Vec<f32> = (0..n_frames)
        .map(|i| {
            let lo = i.saturating_sub(2);
            let hi = (i + 3).min(n_frames);
            mean(&db[lo..hi])
        })
        .collect();
    let mut nuclei = 0usize;
    for i in onset..=offset {
        if periods[i].is_none() || env[i] < thr + 3.0 {
            continue;
        }
        let lo = i.saturating_sub(7);
        let hi = (i + 8).min(n_frames);
        if env[lo..hi].iter().any(|v| *v > env[i]) {
            continue;
        }
        let wlo = i.saturating_sub(12);
        let whi = (i + 13).min(n_frames);
        let left_min = env[wlo..=i].iter().cloned().fold(f32::MAX, f32::min);
        let right_min = env[i..whi].iter().cloned().fold(f32::MAX, f32::min);
        if env[i] - left_min.max(right_min) >= 2.5 {
            nuclei += 1;
        }
    }
    let speaking_s = ((span - pause_frames) as f32 * HOP_S).max(0.2);

    // --- Spectre : centroïde et MFCC ---
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(N_FFT);
    let fb = mel_filterbank();
    let hamming: Vec<f32> = (0..FRAME)
        .map(|n| 0.54 - 0.46 * (2.0 * std::f32::consts::PI * n as f32 / (FRAME - 1) as f32).cos())
        .collect();
    let mut centroids = Vec::new();
    let mut mfcc_sum = [0.0f32; N_MFCC];
    let mut mfcc_n = 0usize;
    let mut buf = vec![Complex32::new(0.0, 0.0); N_FFT];
    let dct: Vec<f32> = (0..N_MFCC)
        .flat_map(|c| {
            (0..N_MEL).map(move |m| {
                (std::f32::consts::PI * (c + 1) as f32 * (m as f32 + 0.5) / N_MEL as f32).cos() * (2.0 / N_MEL as f32).sqrt()
            })
        })
        .collect();
    for idx in onset..=offset {
        if !speech[idx] {
            continue;
        }
        let f = &x[idx * HOP..idx * HOP + FRAME];
        for (n, slot) in buf.iter_mut().enumerate() {
            *slot = if n < FRAME {
                let pre = if n > 0 { f[n] - 0.97 * f[n - 1] } else { f[n] };
                Complex32::new(pre * hamming[n], 0.0)
            } else {
                Complex32::new(0.0, 0.0)
            };
        }
        fft.process(&mut buf);
        let power: Vec<f32> = buf[..N_FFT / 2 + 1].iter().map(|c| c.norm_sqr()).collect();
        let total: f32 = power.iter().sum();
        if total > 0.0 {
            let c: f32 = power
                .iter()
                .enumerate()
                .map(|(k, p)| k as f32 * SR as f32 / N_FFT as f32 * p)
                .sum::<f32>()
                / total;
            centroids.push(c);
        }
        let log_mel: Vec<f32> = fb
            .iter()
            .map(|filt| (filt.iter().map(|(k, w)| power[*k] * w).sum::<f32>() + 1e-10).ln())
            .collect();
        for (c, acc) in mfcc_sum.iter_mut().enumerate() {
            *acc += log_mel.iter().zip(&dct[c * N_MEL..(c + 1) * N_MEL]).map(|(a, b)| a * b).sum::<f32>();
        }
        mfcc_n += 1;
    }
    let mfcc = mfcc_sum.iter().map(|v| v / mfcc_n.max(1) as f32).collect();

    let speech_db: Vec<f32> = (onset..=offset).filter(|i| speech[*i]).map(|i| db[i]).collect();
    let speech_count = speech_db.len().max(1);

    AudioFeatures {
        speech_detected: true,
        latency_s: prompt_offset_s + onset as f32 * HOP_S,
        duration_s: span as f32 * HOP_S,
        f0_mean_st: mean(&st),
        f0_std_st: std_dev(&st),
        jitter,
        shimmer,
        energy_db: mean(&speech_db),
        pause_ratio: pause_frames as f32 / span as f32,
        hesitations: hesitations as f32,
        speech_rate: nuclei as f32 / speaking_s,
        spectral_centroid: mean(&centroids),
        voiced_ratio: voiced_periods.len() as f32 / speech_count as f32,
        mfcc,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Voyelle synthétique : harmoniques d'une fondamentale, modulée en syllabes.
    pub fn synth_voice(f0: f32, silence_s: f32, voice_s: f32, sr: u32) -> Vec<f32> {
        let mut out = vec![0.0f32; (silence_s * sr as f32) as usize];
        let n = (voice_s * sr as f32) as usize;
        let mut phase = 0.0f32;
        for i in 0..n {
            let t = i as f32 / sr as f32;
            let f = f0 * (1.0 + 0.02 * (2.0 * std::f32::consts::PI * 3.0 * t).sin());
            phase += 2.0 * std::f32::consts::PI * f / sr as f32;
            let syll = 0.55 + 0.45 * (2.0 * std::f32::consts::PI * 4.0 * t).sin();
            let s: f32 = (1..=8).map(|h| (phase * h as f32).sin() / h as f32).sum();
            out.push(0.2 * syll * s);
        }
        out.extend(vec![0.0f32; (0.5 * sr as f32) as usize]);
        // Bruit de fond léger, déterministe.
        let mut seed = 12345u32;
        for s in out.iter_mut() {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            *s += ((seed >> 16) as f32 / 65536.0 - 0.5) * 0.002;
        }
        out
    }

    #[test]
    fn detects_latency_and_pitch() {
        let sig = synth_voice(150.0, 1.2, 2.0, 48_000);
        let f = analyze(&sig, 48_000, 0.3);
        assert!(f.speech_detected);
        assert!((f.latency_s - 1.5).abs() < 0.15, "latence {}", f.latency_s);
        let f0 = 55.0 * 2f32.powf(f.f0_mean_st / 12.0);
        assert!((f0 - 150.0).abs() < 8.0, "f0 {f0}");
        assert!(f.speech_rate > 1.5 && f.speech_rate < 7.0, "débit {}", f.speech_rate);
        assert_eq!(f.mfcc.len(), N_MFCC);
    }

    #[test]
    fn higher_voice_has_higher_pitch() {
        let low = analyze(&synth_voice(120.0, 0.5, 1.5, 16_000), 16_000, 0.0);
        let high = analyze(&synth_voice(220.0, 0.5, 1.5, 16_000), 16_000, 0.0);
        assert!(high.f0_mean_st - low.f0_mean_st > 9.0);
    }

    #[test]
    fn silence_is_not_speech() {
        let f = analyze(&vec![0.0005f32; 32_000], 16_000, 0.0);
        assert!(!f.speech_detected);
        assert!(f.latency_s >= 1.9);
    }
}

#[cfg(test)]
mod bench {
    #[test]
    #[ignore]
    fn bench_analyze_15s() {
        let sig = super::tests::synth_voice(160.0, 1.0, 14.0, 48_000);
        let t = std::time::Instant::now();
        for _ in 0..5 {
            std::hint::black_box(super::analyze(&sig, 48_000, 0.0));
        }
        eprintln!("analyse 15 s @48k : {:.1} ms", t.elapsed().as_secs_f64() * 1000.0 / 5.0);
    }
}
