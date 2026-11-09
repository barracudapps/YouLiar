//! Le cœur de YouLiar pour le navigateur (WebAssembly).
//!
//! Un joueur qui rejoint depuis la page servie par l'hôte analyse sa voix et sa caméra
//! sur son propre appareil, exactement comme l'application : seules des mesures et des
//! verdicts circulent. Les échanges avec l'hôte utilisent le même protocole chiffré
//! (SPAKE2 + ChaCha20-Poly1305), transporté par WebSocket.
//!
//! Les données passent en JSON (chaînes) pour garder une interface JavaScript simple.

use wasm_bindgen::prelude::*;
use youliar_core::audio::{self, AudioFeatures};
use youliar_core::model::{measures, CalibTake, Measures, PlayerProfile, ProfileSummary};
use youliar_core::protocol::PROTOCOL_VERSION;
use youliar_core::questions::calibration_steps;
use youliar_core::secure::{Handshake, Opener, Role, Sealer};
use youliar_core::video::{self, VideoFeatures, VideoSample};

// Erreurs en texte : wasm-bindgen les convertit en exceptions JavaScript dans le
// navigateur, et les tests natifs n'ont pas besoin d'environnement JavaScript.
type R<T> = Result<T, String>;

fn js_err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn parse<T: serde::de::DeserializeOwned>(json: &str) -> R<T> {
    serde_json::from_str(json).map_err(js_err)
}

fn json<T: serde::Serialize + ?Sized>(v: &T) -> R<String> {
    serde_json::to_string(v).map_err(js_err)
}

fn opt<T: serde::de::DeserializeOwned>(json: &str) -> R<Option<T>> {
    if json.is_empty() || json == "null" { Ok(None) } else { parse(json).map(Some) }
}

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").into()
}

#[wasm_bindgen]
pub fn themes() -> R<String> {
    json(youliar_core::themes::themes())
}

#[wasm_bindgen]
pub fn calibration_protocol() -> R<String> {
    json(&calibration_steps())
}

/// `samples` : signal mono en f32. Bornes identiques à l'application (60 s à 96 kHz).
#[wasm_bindgen]
pub fn analyze_audio(samples: Vec<f32>, sample_rate: u32, prompt_offset: f32) -> R<String> {
    if !(8_000..=192_000).contains(&sample_rate) || samples.len() > 60 * 96_000 {
        return Err(js_err("Enregistrement trop long ou invalide."));
    }
    let clean: Vec<f32> = samples.iter().map(|x| if x.is_finite() { x.clamp(-1.0, 1.0) } else { 0.0 }).collect();
    let offset = if prompt_offset.is_finite() { prompt_offset.clamp(0.0, 30.0) } else { 0.0 };
    json(&audio::analyze(&clean, sample_rate, offset))
}

#[wasm_bindgen]
pub fn analyze_video(samples_json: &str) -> R<String> {
    let samples: Vec<VideoSample> = parse(samples_json)?;
    if samples.len() > 3600 {
        return Err(js_err("Trop d'images."));
    }
    json(&video::analyze(&samples))
}

#[wasm_bindgen]
pub fn profile_new(name: &str, avatar: &str) -> R<String> {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).take(24).collect();
    let avatar: String = avatar.chars().filter(|c| !c.is_control()).take(4).collect();
    if name.is_empty() {
        return Err(js_err("Choisis un prénom."));
    }
    json(&PlayerProfile::new(format!("p{:016x}", rand::random::<u64>()), name, avatar))
}

#[wasm_bindgen]
pub fn profile_summary(profile_json: &str) -> R<String> {
    let p: PlayerProfile = parse(profile_json)?;
    json(&ProfileSummary::from(&p))
}

/// Renvoie le profil calibré (JSON).
#[wasm_bindgen]
pub fn profile_calibrate(profile_json: &str, takes_json: &str, resting_json: &str) -> R<String> {
    let mut p: PlayerProfile = parse(profile_json)?;
    let takes: Vec<CalibTake> = parse(takes_json)?;
    if takes.len() > 40 {
        return Err(js_err("Trop de prises."));
    }
    let resting: Option<VideoFeatures> = opt(resting_json)?;
    p.calibrate_from_takes(&takes, resting.as_ref()).map_err(js_err)?;
    json(&p)
}

/// `others_json` : profils de cet appareil (attribution de la voix) ; `session_json` :
/// mesures des réponses déjà données dans la partie (correction de dérive).
#[wasm_bindgen]
pub fn profile_score(profile_json: &str, audio_json: &str, video_json: &str, others_json: &str, session_json: &str) -> R<String> {
    let p: PlayerProfile = parse(profile_json)?;
    let audio: AudioFeatures = parse(audio_json)?;
    let video: Option<VideoFeatures> = opt(video_json)?;
    let others: Vec<PlayerProfile> = parse(others_json)?;
    let session: Vec<Measures> = parse(session_json)?;
    let refs: Vec<&PlayerProfile> = others.iter().take(12).collect();
    let verdict = p.verdict_for(&audio, video.as_ref(), &refs, &session[session.len().saturating_sub(50)..]).map_err(js_err)?;
    json(&verdict)
}

#[wasm_bindgen]
pub fn answer_measures(audio_json: &str, video_json: &str) -> R<String> {
    let audio: AudioFeatures = parse(audio_json)?;
    let video: Option<VideoFeatures> = opt(video_json)?;
    json(&measures(&audio, video.as_ref()))
}

/// Renvoie le profil après apprentissage (JSON).
#[wasm_bindgen]
pub fn profile_learn(profile_json: &str, audio_json: &str, video_json: &str, lie: bool, machine_correct: bool) -> R<String> {
    let mut p: PlayerProfile = parse(profile_json)?;
    let audio: AudioFeatures = parse(audio_json)?;
    let video: Option<VideoFeatures> = opt(video_json)?;
    p.learn_answer(&audio, video.as_ref(), lie, machine_correct);
    json(&p)
}

/// Canal chiffré avec l'hôte, côté navigateur. Le cadrage (longueur sur 4 octets) et le
/// transport WebSocket sont assurés en JavaScript.
#[wasm_bindgen]
pub struct WebSession {
    handshake: Option<Handshake>,
    first: Vec<u8>,
    sealer: Option<Sealer>,
    opener: Option<Opener>,
}

#[wasm_bindgen]
impl WebSession {
    #[wasm_bindgen(constructor)]
    pub fn new(code: &str) -> WebSession {
        let (hs, msg) = Handshake::start(Role::Client, code);
        let mut first = PROTOCOL_VERSION.to_be_bytes().to_vec();
        first.extend_from_slice(&msg);
        WebSession { handshake: Some(hs), first, sealer: None, opener: None }
    }

    /// Première trame à envoyer : version du protocole et message SPAKE2.
    pub fn hello(&self) -> Vec<u8> {
        self.first.clone()
    }

    /// Termine l'échange avec la réponse de l'hôte.
    pub fn finish(&mut self, frame: &[u8]) -> R<()> {
        if frame.len() <= 4 {
            return Err(js_err("Réponse invalide de l'hôte."));
        }
        let hs = self.handshake.take().ok_or_else(|| js_err("Échange déjà terminé."))?;
        let (sealer, opener) = hs.finish(&frame[4..]).map_err(|_| js_err("Code de partie incorrect."))?;
        self.sealer = Some(sealer);
        self.opener = Some(opener);
        Ok(())
    }

    pub fn seal(&mut self, json: &str) -> R<Vec<u8>> {
        self.sealer.as_mut().ok_or_else(|| js_err("Canal non établi."))?.seal(json.as_bytes()).map_err(|_| js_err("Chiffrement impossible."))
    }

    /// Déchiffre une trame ; échoue si elle a été altérée, rejouée ou si le code diffère.
    pub fn open(&mut self, frame: &[u8]) -> R<String> {
        let pt = self.opener.as_mut().ok_or_else(|| js_err("Canal non établi."))?.open(frame).map_err(|_| js_err("Code de partie incorrect."))?;
        String::from_utf8(pt).map_err(js_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use youliar_core::protocol::{ClientMsg, ServerMsg};

    #[test]
    fn profile_round_trip_through_json() {
        let p = profile_new("  Léa ", "🦊").unwrap();
        let s: serde_json::Value = serde_json::from_str(&profile_summary(&p).unwrap()).unwrap();
        assert_eq!(s["name"], "Léa");
        assert_eq!(s["calibrated"], false);
        assert!(profile_new("   ", "x").is_err());
        let steps: serde_json::Value = serde_json::from_str(&calibration_protocol().unwrap()).unwrap();
        assert_eq!(steps.as_array().unwrap().len(), 15);
    }

    #[test]
    fn web_session_speaks_the_host_protocol() {
        let mut web = WebSession::new("482913");
        let hello = web.hello();
        assert_eq!(u32::from_be_bytes(hello[..4].try_into().unwrap()), PROTOCOL_VERSION);
        // Côté hôte, avec le même code.
        let (hs, msg) = Handshake::start(Role::Host, "482913");
        let (mut host_tx, mut host_rx) = hs.finish(&hello[4..]).unwrap();
        let mut reply = PROTOCOL_VERSION.to_be_bytes().to_vec();
        reply.extend_from_slice(&msg);
        web.finish(&reply).unwrap();
        let frame = web.seal(&serde_json::to_string(&ClientMsg::Ping).unwrap()).unwrap();
        let back: ClientMsg = serde_json::from_slice(&host_rx.open(&frame).unwrap()).unwrap();
        assert_eq!(back, ClientMsg::Ping);
        let pong = host_tx.seal(serde_json::to_string(&ServerMsg::Pong).unwrap().as_bytes()).unwrap();
        assert!(web.open(&pong).unwrap().contains("pong"));
        // Code faux : refusé au premier message.
        let mut wrong = WebSession::new("000000");
        let (hs, msg) = Handshake::start(Role::Host, "482913");
        let (_, mut rx) = hs.finish(&wrong.hello()[4..]).unwrap();
        let mut reply = PROTOCOL_VERSION.to_be_bytes().to_vec();
        reply.extend_from_slice(&msg);
        wrong.finish(&reply).unwrap();
        assert!(rx.open(&wrong.seal("{}").unwrap()).is_err());
    }
}
