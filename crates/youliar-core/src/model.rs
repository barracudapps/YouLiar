//! Modèle propre à chaque joueur.
//!
//! * Une **ligne de base** (moyenne / écart-type de chaque mesure) est établie au
//!   calibrage à partir des réponses neutres et sincères ; une réponse est ensuite
//!   décrite par des écarts normalisés (z-scores) à cette ligne de base.
//! * Un score de type régression logistique combine ces écarts. Les poids partent
//!   d'a priori « stress » (voix plus aiguë, latence, pauses, pouls, agitation…),
//!   puis sont réajustés sur les réponses dont la vérité est connue : consignes du
//!   calibrage, puis chaque manche révélée. La machine « apprend » donc le joueur.
//!
//! ⚠️ Rien de tout cela ne détecte réellement le mensonge : on mesure des signes
//! de stress et d'écart à l'habitude. C'est un jeu.

use crate::audio::{AudioFeatures, N_MFCC};
use crate::video::VideoFeatures;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub struct FeatureDef {
    pub key: &'static str,
    pub label: &'static str,
    pub up: &'static str,
    pub down: &'static str,
    /// Poids a priori (positif : une hausse rend plus suspect).
    pub prior: f32,
    /// Écart-type minimal, pour ne pas sur-réagir à une base trop régulière.
    pub std_floor: f32,
}

pub const FEATURES: &[FeatureDef] = &[
    FeatureDef { key: "latency", label: "Temps de réponse", up: "plus long que d'habitude", down: "plus rapide que d'habitude", prior: 0.9, std_floor: 0.35 },
    FeatureDef { key: "pitch", label: "Hauteur de voix", up: "voix plus aiguë", down: "voix plus grave", prior: 0.8, std_floor: 0.7 },
    FeatureDef { key: "intonation", label: "Intonation", up: "voix plus chantante", down: "voix plus monotone", prior: 0.35, std_floor: 0.4 },
    FeatureDef { key: "jitter", label: "Tremblement de la voix", up: "voix plus tremblante", down: "voix plus stable", prior: 0.5, std_floor: 0.006 },
    FeatureDef { key: "shimmer", label: "Stabilité du volume", up: "volume plus irrégulier", down: "volume plus régulier", prior: 0.3, std_floor: 0.03 },
    FeatureDef { key: "energy", label: "Volume", up: "parle plus fort", down: "parle plus bas", prior: 0.15, std_floor: 2.0 },
    FeatureDef { key: "pauses", label: "Silences", up: "plus de blancs", down: "moins de blancs", prior: 0.6, std_floor: 0.06 },
    FeatureDef { key: "hesitations", label: "Hésitations", up: "hésite davantage", down: "hésite moins", prior: 0.5, std_floor: 0.6 },
    FeatureDef { key: "rate", label: "Débit", up: "parle plus vite", down: "parle plus lentement", prior: 0.2, std_floor: 0.5 },
    FeatureDef { key: "duration", label: "Longueur de réponse", up: "réponse plus longue", down: "réponse plus courte", prior: 0.35, std_floor: 0.5 },
    FeatureDef { key: "heart_rate", label: "Pouls (caméra)", up: "pouls plus rapide", down: "pouls plus lent", prior: 0.8, std_floor: 3.0 },
    FeatureDef { key: "motion", label: "Agitation (caméra)", up: "bouge davantage", down: "plus immobile", prior: 0.6, std_floor: 0.4 },
];

pub type Measures = BTreeMap<String, f32>;

/// Convertit les analyses brutes en mesures nommées (absentes si non disponibles).
pub fn measures(audio: &AudioFeatures, video: Option<&VideoFeatures>) -> Measures {
    let mut m = Measures::new();
    m.insert("latency".into(), audio.latency_s);
    if audio.speech_detected {
        if audio.voiced_ratio > 0.1 {
            m.insert("pitch".into(), audio.f0_mean_st);
            m.insert("intonation".into(), audio.f0_std_st);
            m.insert("jitter".into(), audio.jitter);
            m.insert("shimmer".into(), audio.shimmer);
        }
        m.insert("energy".into(), audio.energy_db);
        m.insert("pauses".into(), audio.pause_ratio);
        m.insert("hesitations".into(), audio.hesitations);
        m.insert("rate".into(), audio.speech_rate);
        m.insert("duration".into(), audio.duration_s);
    }
    if let Some(v) = video {
        if let Some(hr) = v.heart_rate.filter(|h| (30.0..=220.0).contains(h)) {
            m.insert("heart_rate".into(), hr);
        }
        if v.duration_s > 0.5 {
            m.insert("motion".into(), v.motion_mean);
        }
    }
    // Une valeur non finie (NaN…) empoisonnerait définitivement la ligne de base.
    m.retain(|_, v| v.is_finite());
    m
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Stat {
    pub mean: f32,
    pub std: f32,
    pub n: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Label {
    /// Lecture / comptage : sert à la ligne de base uniquement.
    Neutral,
    Truth,
    Lie,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sample {
    pub measures: Measures,
    pub label: Label,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct VoiceSignature {
    pub mfcc: Vec<f32>,
    pub pitch_st: f32,
    pub n: u32,
}

impl VoiceSignature {
    pub fn from_audio(a: &AudioFeatures) -> Option<Self> {
        (a.speech_detected && a.mfcc.len() == N_MFCC && a.mfcc.iter().all(|x| x.is_finite()) && a.f0_mean_st.is_finite()).then(|| Self { mfcc: a.mfcc.clone(), pitch_st: a.f0_mean_st, n: 1 })
    }

    /// Intègre une nouvelle prise (moyenne cumulée).
    pub fn merge(&mut self, a: &AudioFeatures) {
        if let Some(o) = Self::from_audio(a) {
            let w = 1.0 / (self.n + 1) as f32;
            for (s, v) in self.mfcc.iter_mut().zip(o.mfcc.iter()) {
                *s += (v - *s) * w;
            }
            self.pitch_st += (o.pitch_st - self.pitch_st) * w;
            self.n += 1;
        }
    }

    pub fn distance(&self, other: &VoiceSignature) -> f32 {
        let timbre: f32 = self.mfcc.iter().zip(&other.mfcc).map(|(a, b)| ((a - b) / 3.0).powi(2)).sum();
        (timbre + ((self.pitch_st - other.pitch_st) / 2.5).powi(2)).sqrt()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlayerProfile {
    pub id: String,
    pub name: String,
    pub avatar: String,
    pub calibrated: bool,
    pub camera_calibrated: bool,
    pub baseline: BTreeMap<String, Stat>,
    pub weights: BTreeMap<String, f32>,
    pub bias: f32,
    pub voice: Option<VoiceSignature>,
    /// Pouls de repos mesuré au calibrage caméra.
    pub resting_hr: Option<f32>,
    pub samples: Vec<Sample>,
    pub answers_learned: u32,
    #[serde(default)]
    pub stats: PlayerStats,
}

/// Bilan du joueur face à la machine, toutes parties confondues.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PlayerStats {
    pub answers: u32,
    pub lies: u32,
    /// Mensonges démasqués par la machine.
    pub lies_caught: u32,
    /// Réponses (vraies ou fausses) sur lesquelles la machine s'est trompée.
    pub machine_fooled: u32,
}

/// « Carte d'identité vocale » : la ligne de base, en unités parlantes.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct Traits {
    pub pitch_hz: Option<f32>,
    pub latency_s: Option<f32>,
    pub speech_rate: Option<f32>,
    pub pauses_pct: Option<f32>,
}

const MAX_SAMPLES: usize = 300;
const GAIN: f32 = 1.6;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Factor {
    pub key: String,
    pub label: String,
    pub description: String,
    pub z: f32,
    pub contribution: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeakerCheck {
    pub expected_id: String,
    pub best_id: String,
    pub best_name: String,
    pub matched: bool,
    pub distance: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Verdict {
    /// Probabilité de mensonge estimée par la machine (0-1).
    pub probability: f32,
    /// Indice affiché, 0-100.
    pub index: u8,
    pub label: String,
    pub is_lie: bool,
    pub factors: Vec<Factor>,
    pub heart_rate: Option<f32>,
    /// Confiance de la machine dans son propre verdict (0-1).
    pub confidence: f32,
    pub speaker: Option<SpeakerCheck>,
}

fn clip(s: &mut String, n: usize) {
    if s.chars().count() > n {
        *s = s.chars().take(n).collect();
    }
}

fn finite(x: f32, lo: f32, hi: f32) -> f32 {
    if x.is_finite() { x.clamp(lo, hi) } else { 0.0 }
}

impl Verdict {
    /// Rend cohérent et borné un verdict reçu d'un autre appareil : l'index et la
    /// décision sont recalculés depuis la probabilité, textes et listes sont tronqués.
    pub fn sanitize(&mut self) {
        self.probability = finite(self.probability, 0.0, 1.0);
        self.index = (self.probability * 100.0).round() as u8;
        self.is_lie = self.probability >= 0.5;
        self.label = match self.index {
            0..=34 => "Sincère",
            35..=49 => "Plutôt sincère",
            50..=64 => "Suspect",
            _ => "Mensonge probable",
        }
        .into();
        self.confidence = finite(self.confidence, 0.0, 1.0);
        self.heart_rate = self.heart_rate.filter(|h| h.is_finite() && (30.0..=220.0).contains(h));
        self.factors.truncate(6);
        for f in self.factors.iter_mut() {
            clip(&mut f.key, 24);
            clip(&mut f.label, 40);
            clip(&mut f.description, 80);
            f.z = finite(f.z, -10.0, 10.0);
            f.contribution = finite(f.contribution, -10.0, 10.0);
        }
        if let Some(sp) = self.speaker.as_mut() {
            clip(&mut sp.expected_id, 40);
            clip(&mut sp.best_id, 40);
            clip(&mut sp.best_name, 24);
            sp.distance = finite(sp.distance, 0.0, 1e6);
        }
    }
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

impl PlayerProfile {
    pub fn new(id: impl Into<String>, name: impl Into<String>, avatar: impl Into<String>) -> Self {
        let mut p = Self {
            id: id.into(),
            name: name.into(),
            avatar: avatar.into(),
            calibrated: false,
            camera_calibrated: false,
            baseline: BTreeMap::new(),
            weights: BTreeMap::new(),
            bias: -1.0,
            voice: None,
            resting_hr: None,
            samples: Vec::new(),
            answers_learned: 0,
            stats: PlayerStats::default(),
        };
        p.reset_weights();
        p
    }

    fn reset_weights(&mut self) {
        self.weights = FEATURES.iter().map(|f| (f.key.to_string(), f.prior)).collect();
        self.bias = -1.0;
    }

    fn z(&self, m: &Measures) -> Vec<(&'static FeatureDef, f32)> {
        FEATURES
            .iter()
            .filter_map(|f| {
                let x = m.get(f.key)?;
                let st = self.baseline.get(f.key)?;
                Some((f, ((x - st.mean) / st.std.max(f.std_floor)).clamp(-3.0, 3.0)))
            })
            .collect()
    }

    /// Logit et contributions individuelles.
    fn logit(&self, m: &Measures) -> (f32, Vec<(&'static FeatureDef, f32, f32)>, f32) {
        let zs = self.z(m);
        let norm = zs
            .iter()
            .map(|(f, _)| self.weights.get(f.key).copied().unwrap_or(f.prior).powi(2))
            .sum::<f32>()
            .sqrt()
            .max(0.5);
        let parts: Vec<_> = zs
            .iter()
            .map(|(f, z)| {
                let w = self.weights.get(f.key).copied().unwrap_or(f.prior);
                (*f, *z, GAIN * w * z / norm)
            })
            .collect();
        let s = self.bias + parts.iter().map(|p| p.2).sum::<f32>();
        (s, parts, norm)
    }

    /// Recalcule la ligne de base puis réajuste les poids sur les réponses étiquetées.
    pub fn rebuild(&mut self) {
        self.baseline.clear();
        for f in FEATURES {
            let pick = |allow_lie: bool| -> Vec<f32> {
                self.samples
                    .iter()
                    .filter(|s| allow_lie || s.label != Label::Lie)
                    .filter_map(|s| s.measures.get(f.key).copied())
                    .collect()
            };
            let mut vals = pick(false);
            if vals.len() < 2 {
                vals = pick(true);
            }
            if vals.is_empty() {
                continue;
            }
            let n = vals.len() as f32;
            let mean = vals.iter().sum::<f32>() / n;
            let var = if vals.len() > 1 { vals.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / (n - 1.0) } else { 0.0 };
            self.baseline.insert(f.key.into(), Stat { mean, std: var.sqrt().max(f.std_floor), n: vals.len() as u32 });
        }
        self.fit();
    }

    /// Régression logistique régularisée vers les a priori (peu de données !).
    fn fit(&mut self) {
        self.reset_weights();
        let labeled: Vec<(Measures, f32)> = self
            .samples
            .iter()
            .filter_map(|s| match s.label {
                Label::Truth => Some((s.measures.clone(), 0.0)),
                Label::Lie => Some((s.measures.clone(), 1.0)),
                Label::Neutral => None,
            })
            .collect();
        let has_lie = labeled.iter().any(|(_, y)| *y > 0.5);
        let has_truth = labeled.iter().any(|(_, y)| *y < 0.5);
        if !(has_lie && has_truth) {
            return;
        }
        let lambda = 2.0 / labeled.len() as f32;
        let lr = 0.15;
        for _ in 0..400 {
            let mut grad_w: BTreeMap<&str, f32> = BTreeMap::new();
            let mut grad_b = 0.0;
            for (m, y) in &labeled {
                let (s, parts, norm) = self.logit(m);
                let err = sigmoid(s) - y;
                grad_b += err;
                for (f, z, _) in parts {
                    *grad_w.entry(f.key).or_default() += err * GAIN * z / norm;
                }
            }
            let n = labeled.len() as f32;
            self.bias -= lr * (grad_b / n + 0.05 * (self.bias + 1.0));
            for f in FEATURES {
                let w = self.weights.get_mut(f.key).unwrap();
                let g = grad_w.get(f.key).copied().unwrap_or(0.0) / n + lambda * (*w - f.prior);
                *w = (*w - lr * g).clamp(-2.5, 2.5);
            }
        }
    }

    /// Termine le calibrage avec les prises guidées.
    pub fn calibrate(&mut self, samples: Vec<Sample>, voice_takes: &[AudioFeatures], resting_hr: Option<f32>) {
        self.samples = samples;
        self.voice = None;
        for a in voice_takes {
            match &mut self.voice {
                Some(v) => v.merge(a),
                None => self.voice = VoiceSignature::from_audio(a),
            }
        }
        self.resting_hr = resting_hr;
        self.camera_calibrated = resting_hr.is_some() || self.samples.iter().any(|s| s.measures.contains_key("motion"));
        self.answers_learned = 0;
        self.rebuild();
        self.calibrated = true;
    }

    pub fn traits(&self) -> Traits {
        let mean = |k: &str| self.baseline.get(k).map(|s| s.mean);
        Traits {
            pitch_hz: mean("pitch").map(|st| 55.0 * 2f32.powf(st / 12.0)),
            latency_s: mean("latency"),
            speech_rate: mean("rate"),
            pauses_pct: mean("pauses").map(|p| p * 100.0),
        }
    }

    /// Met à jour le bilan après une manche révélée.
    pub fn record_outcome(&mut self, lie: bool, machine_correct: bool) {
        self.stats.answers += 1;
        if lie {
            self.stats.lies += 1;
            if machine_correct {
                self.stats.lies_caught += 1;
            }
        }
        if !machine_correct {
            self.stats.machine_fooled += 1;
        }
    }

    /// Apprend d'une réponse dont la vérité vient d'être révélée.
    pub fn learn(&mut self, m: Measures, lie: bool) {
        self.samples.push(Sample { measures: m, label: if lie { Label::Lie } else { Label::Truth } });
        if self.samples.len() > MAX_SAMPLES {
            // On garde les prises neutres du calibrage, on oublie les plus anciennes réponses.
            if let Some(pos) = self.samples.iter().position(|s| s.label != Label::Neutral) {
                self.samples.remove(pos);
            }
        }
        self.answers_learned += 1;
        self.rebuild();
    }

    pub fn score(&self, m: &Measures) -> Verdict {
        let (s, parts, _) = self.logit(m);
        let p = sigmoid(s);
        let index = (p * 100.0).round().clamp(0.0, 100.0) as u8;
        let label = match index {
            0..=34 => "Sincère",
            35..=49 => "Plutôt sincère",
            50..=64 => "Suspect",
            _ => "Mensonge probable",
        };
        let mut factors: Vec<Factor> = parts
            .iter()
            .filter(|(_, z, _)| z.abs() >= 0.8)
            .map(|(f, z, c)| Factor {
                key: f.key.into(),
                label: f.label.into(),
                description: format!("{} ({:+.1}σ)", if *z > 0.0 { f.up } else { f.down }, z),
                z: *z,
                contribution: *c,
            })
            .collect();
        factors.sort_by(|a, b| b.contribution.abs().partial_cmp(&a.contribution.abs()).unwrap_or(std::cmp::Ordering::Equal));
        factors.truncate(4);
        let labeled = self.samples.iter().filter(|s| s.label != Label::Neutral).count();
        let confidence = 0.5 * (parts.len() as f32 / FEATURES.len() as f32) + 0.5 * (labeled.min(20) as f32 / 20.0);
        Verdict {
            probability: p,
            index,
            label: label.into(),
            is_lie: p >= 0.5,
            factors,
            heart_rate: m.get("heart_rate").copied(),
            confidence,
            speaker: None,
        }
    }
}

/// Identifie, parmi les profils calibrés, celui dont la voix ressemble le plus.
pub fn identify_speaker(audio: &AudioFeatures, expected_id: &str, candidates: &[&PlayerProfile]) -> Option<SpeakerCheck> {
    let sig = VoiceSignature::from_audio(audio)?;
    let voiced: Vec<_> = candidates.iter().filter_map(|p| p.voice.as_ref().map(|v| (p, v.distance(&sig)))).collect();
    if voiced.len() < 2 {
        return None;
    }
    let (best, dist) = voiced.iter().min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
    let expected_dist = voiced.iter().find(|(p, _)| p.id == expected_id).map(|v| v.1);
    // On ne crie à l'usurpation que si l'écart est net.
    let matched = best.id == expected_id || expected_dist.is_none_or(|d| d - dist < 0.6);
    Some(SpeakerCheck {
        expected_id: expected_id.into(),
        best_id: best.id.clone(),
        best_name: best.name.clone(),
        matched,
        distance: *dist,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(latency: f32, pitch: f32, pauses: f32) -> Measures {
        let mut x = Measures::new();
        x.insert("latency".into(), latency);
        x.insert("pitch".into(), pitch);
        x.insert("pauses".into(), pauses);
        x.insert("energy".into(), -20.0);
        x
    }

    #[test]
    fn calibrated_profile_flags_stressed_answers() {
        let mut p = PlayerProfile::new("a", "Alice", "🦊");
        let mut s = vec![];
        for i in 0..4 {
            s.push(Sample { measures: m(0.6 + 0.05 * i as f32, 30.0 + 0.2 * i as f32, 0.1), label: Label::Neutral });
            s.push(Sample { measures: m(0.7 + 0.05 * i as f32, 30.2, 0.12), label: Label::Truth });
            s.push(Sample { measures: m(1.8 + 0.1 * i as f32, 32.0, 0.3), label: Label::Lie });
        }
        p.calibrate(s, &[], None);
        let calm = p.score(&m(0.65, 30.1, 0.1));
        let tense = p.score(&m(2.0, 32.5, 0.35));
        assert!(calm.probability < 0.4, "calme {}", calm.probability);
        assert!(tense.probability > 0.7, "tendu {}", tense.probability);
        assert!(tense.factors.iter().any(|f| f.key == "latency"));
    }

    #[test]
    fn learning_shifts_weights() {
        let mut p = PlayerProfile::new("a", "A", "x");
        let base: Vec<Sample> = (0..5).map(|i| Sample { measures: m(0.8, 30.0 + 0.3 * i as f32, 0.1), label: Label::Neutral }).collect();
        p.calibrate(base, &[], None);
        // Ce joueur ment… en répondant plus vite.
        for _ in 0..6 {
            p.learn(m(0.5, 30.5, 0.1), true);
            p.learn(m(1.1, 30.5, 0.1), false);
        }
        assert!(p.weights["latency"] < 0.5, "poids {}", p.weights["latency"]);
        assert!(p.score(&m(0.5, 30.5, 0.1)).probability > p.score(&m(1.1, 30.5, 0.1)).probability);
    }

    #[test]
    fn speaker_identification() {
        let mk = |id: &str, pitch: f32, c1: f32| {
            let mut p = PlayerProfile::new(id, id, "x");
            p.voice = Some(VoiceSignature { mfcc: vec![c1; N_MFCC], pitch_st: pitch, n: 1 });
            p
        };
        let (a, b) = (mk("a", 25.0, 2.0), mk("b", 37.0, -4.0));
        let audio = AudioFeatures { speech_detected: true, f0_mean_st: 36.5, mfcc: vec![-3.5; N_MFCC], ..Default::default() };
        let check = identify_speaker(&audio, "a", &[&a, &b]).unwrap();
        assert_eq!(check.best_id, "b");
        assert!(!check.matched);
    }
}
