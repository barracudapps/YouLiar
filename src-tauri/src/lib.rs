//! Application Tauri : commandes appelées par l'interface web.

#[cfg(desktop)]
mod edge;
mod net;

use net::{HostInfo, Session, SessionInfo};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::async_runtime::JoinHandle;
use tauri::ipc::{InvokeBody, Request};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use youliar_core::audio::{self, AudioFeatures};
use youliar_core::game::{Action, GameView, PlayerInfo};
use youliar_core::model::{self, Label, PlayerProfile, PlayerStats, Sample, Traits, Verdict};
use youliar_core::questions::{calibration_steps, CalibKind, CalibStep};
use youliar_core::video::{self, VideoFeatures, VideoSample};

#[derive(Default)]
struct AppState {
    profiles: Mutex<Vec<PlayerProfile>>,
    profiles_path: Mutex<Option<PathBuf>>,
    session: Mutex<Option<Session>>,
    discovery: Mutex<Option<JoinHandle<()>>>,
}

impl AppState {
    fn save(&self) -> Result<(), String> {
        let Some(path) = self.profiles_path.lock().unwrap().clone() else { return Ok(()) };
        let json = serde_json::to_vec_pretty(&*self.profiles.lock().unwrap()).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("json.tmp");
        write_private(&tmp, &json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    fn with_profile<T>(&self, id: &str, f: impl FnOnce(&mut PlayerProfile) -> T) -> Result<T, String> {
        let out = {
            let mut list = self.profiles.lock().unwrap();
            let p = list.iter_mut().find(|p| p.id == id).ok_or("Profil introuvable.")?;
            f(p)
        };
        self.save()?;
        Ok(out)
    }
}

/// Les profils contiennent des données personnelles (empreinte vocale, pouls) :
/// fichier lisible par l'utilisateur seul.
fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(data)?;
    f.sync_all()
}

const MAX_AUDIO_BYTES: usize = 60 * 96_000 * 4; // 60 s à 96 kHz
const MAX_VIDEO_SAMPLES: usize = 60 * 60; // 60 s à 60 i/s
const MAX_TAKES: usize = 40;

/// Ce que l'interface a besoin de savoir d'un profil (sans les données d'apprentissage).
#[derive(Serialize)]
struct ProfileSummary {
    id: String,
    name: String,
    avatar: String,
    calibrated: bool,
    camera_calibrated: bool,
    resting_hr: Option<f32>,
    answers_learned: u32,
    has_voice: bool,
    traits: Traits,
    stats: PlayerStats,
}

impl From<&PlayerProfile> for ProfileSummary {
    fn from(p: &PlayerProfile) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            avatar: p.avatar.clone(),
            calibrated: p.calibrated,
            camera_calibrated: p.camera_calibrated,
            resting_hr: p.resting_hr,
            answers_learned: p.answers_learned,
            has_voice: p.voice.is_some(),
            traits: p.traits(),
            stats: p.stats.clone(),
        }
    }
}

fn player_info(p: &PlayerProfile) -> PlayerInfo {
    PlayerInfo { id: p.id.clone(), name: p.name.clone(), avatar: p.avatar.clone(), calibrated: p.calibrated }
}

// ---------- Profils ----------

#[tauri::command]
fn profiles_list(state: State<AppState>) -> Vec<ProfileSummary> {
    state.profiles.lock().unwrap().iter().map(ProfileSummary::from).collect()
}

#[tauri::command]
async fn profile_create(state: State<'_, AppState>, name: String, avatar: String) -> Result<ProfileSummary, String> {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).take(24).collect();
    let avatar: String = avatar.chars().filter(|c| !c.is_control()).take(4).collect();
    if name.is_empty() {
        return Err("Choisis un prénom.".into());
    }
    let id = format!("p{:016x}", rand::random::<u64>());
    let p = PlayerProfile::new(id, name, avatar);
    let summary = ProfileSummary::from(&p);
    state.profiles.lock().unwrap().push(p);
    state.save()?;
    Ok(summary)
}

#[tauri::command]
async fn profile_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.profiles.lock().unwrap().retain(|p| p.id != id);
    state.save()
}

#[tauri::command]
fn player_infos(state: State<AppState>, ids: Vec<String>) -> Vec<PlayerInfo> {
    let list = state.profiles.lock().unwrap();
    ids.iter().filter_map(|id| list.iter().find(|p| &p.id == id)).map(player_info).collect()
}

// ---------- Analyse ----------

#[tauri::command]
fn calibration_protocol() -> Vec<CalibStep> {
    calibration_steps()
}

/// Corps brut : échantillons f32 little-endian. En-têtes : `x-sample-rate`, `x-prompt-offset`.
#[tauri::command]
async fn analyze_audio(request: Request<'_>) -> Result<AudioFeatures, String> {
    let InvokeBody::Raw(bytes) = request.body() else { return Err("Audio attendu en binaire.".into()) };
    if bytes.len() > MAX_AUDIO_BYTES || bytes.len() % 4 != 0 {
        return Err("Enregistrement trop long ou invalide.".into());
    }
    let header = |k: &str| request.headers().get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
    let sr: u32 = header("x-sample-rate")
        .and_then(|v| v.parse().ok())
        .filter(|sr| (8_000..=192_000).contains(sr))
        .ok_or("Fréquence d'échantillonnage invalide.")?;
    let offset: f32 = header("x-prompt-offset").and_then(|v| v.parse().ok()).filter(|o: &f32| o.is_finite()).unwrap_or(0.0).clamp(0.0, 30.0);
    let samples: Vec<f32> = bytes
        .as_chunks::<4>().0.iter()
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .map(|x| if x.is_finite() { x.clamp(-1.0, 1.0) } else { 0.0 })
        .collect();
    tauri::async_runtime::spawn_blocking(move || audio::analyze(&samples, sr, offset.max(0.0)))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn analyze_video(samples: Vec<VideoSample>) -> Result<VideoFeatures, String> {
    if samples.len() > MAX_VIDEO_SAMPLES {
        return Err("Trop d'images.".into());
    }
    tauri::async_runtime::spawn_blocking(move || video::analyze(&samples)).await.map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct CalibTake {
    kind: CalibKind,
    audio: AudioFeatures,
    video: Option<VideoFeatures>,
}

#[tauri::command]
async fn calibration_finish(state: State<'_, AppState>, profile_id: String, takes: Vec<CalibTake>, resting: Option<VideoFeatures>) -> Result<ProfileSummary, String> {
    if takes.len() > MAX_TAKES {
        return Err("Trop de prises.".into());
    }
    let answers = takes.iter().filter(|t| matches!(t.kind, CalibKind::Truth | CalibKind::Lie) && t.audio.speech_detected).count();
    if answers < 2 {
        return Err("Pas assez de réponses exploitables : recommence le calibrage dans un endroit calme.".into());
    }
    let samples: Vec<Sample> = takes
        .iter()
        .filter(|t| t.audio.speech_detected)
        .map(|t| {
            let mut m = model::measures(&t.audio, t.video.as_ref());
            let label = match t.kind {
                CalibKind::Voice | CalibKind::Neutral => {
                    // Lire ou compter n'est pas répondre : latence et durée ne sont pas comparables.
                    m.remove("latency");
                    m.remove("duration");
                    Label::Neutral
                }
                CalibKind::Truth => Label::Truth,
                CalibKind::Lie => Label::Lie,
            };
            Sample { measures: m, label }
        })
        .collect();
    let voices: Vec<AudioFeatures> = takes.iter().map(|t| t.audio.clone()).collect();
    let mut resting_hr = resting.as_ref().and_then(|v| v.heart_rate);
    if resting_hr.is_none() {
        let hrs: Vec<f32> = takes.iter().filter_map(|t| t.video.as_ref()?.heart_rate).collect();
        if !hrs.is_empty() {
            resting_hr = Some(hrs.iter().sum::<f32>() / hrs.len() as f32);
        }
    }
    state.with_profile(&profile_id, |p| {
        p.calibrate(samples, &voices, resting_hr);
        if let Some(r) = &resting {
            if r.duration_s > 3.0 {
                p.camera_calibrated = true;
            }
        }
        ProfileSummary::from(&*p)
    })
}

#[tauri::command]
async fn score_answer(
    state: State<'_, AppState>,
    profile_id: String,
    audio: AudioFeatures,
    video: Option<VideoFeatures>,
    candidates: Vec<String>,
) -> Result<Verdict, String> {
    if !audio.speech_detected {
        return Err("Aucune voix détectée : parle plus près du micro et recommence.".into());
    }
    let list = state.profiles.lock().unwrap();
    let p = list.iter().find(|p| p.id == profile_id).ok_or("Profil introuvable.")?;
    let mut verdict = p.score(&model::measures(&audio, video.as_ref()));
    let others: Vec<&PlayerProfile> = list.iter().filter(|c| c.id == profile_id || candidates.iter().take(12).any(|id| id == &c.id)).collect();
    verdict.speaker = model::identify_speaker(&audio, &profile_id, &others);
    Ok(verdict)
}

#[tauri::command]
async fn learn_answer(
    state: State<'_, AppState>,
    profile_id: String,
    audio: AudioFeatures,
    video: Option<VideoFeatures>,
    lie: bool,
    machine_correct: bool,
) -> Result<ProfileSummary, String> {
    state.with_profile(&profile_id, |p| {
        p.record_outcome(lie, machine_correct);
        p.learn(model::measures(&audio, video.as_ref()), lie);
        if let Some(v) = p.voice.as_mut() {
            v.merge(&audio);
        }
        ProfileSummary::from(&*p)
    })
}

// ---------- Sessions ----------

fn emitter(app: AppHandle) -> net::Emit {
    Arc::new(move |name: &str, payload| {
        let _ = app.emit(name, payload);
    })
}

fn replace_session(state: &AppState, s: Option<Session>) {
    let old = std::mem::replace(&mut *state.session.lock().unwrap(), s);
    if let Some(old) = old {
        old.stop();
    }
}

#[tauri::command]
fn session_local(app: AppHandle, state: State<AppState>) -> SessionInfo {
    replace_session(&state, Some(Session::Host(net::start_local(emitter(app)))));
    SessionInfo::Local
}

#[tauri::command]
async fn session_host(app: AppHandle, state: State<'_, AppState>, game_name: String) -> Result<HostInfo, String> {
    replace_session(&state, None);
    let h = net::start_host(emitter(app), game_name).await?;
    let info = h.info().ok_or("Hôte sans adresse réseau.")?;
    replace_session(&state, Some(Session::Host(h)));
    Ok(info)
}

#[tauri::command]
async fn session_join(app: AppHandle, state: State<'_, AppState>, ip: String, port: u16, code: String, profile_ids: Vec<String>) -> Result<SessionInfo, String> {
    if profile_ids.is_empty() {
        return Err("Choisis au moins un joueur sur cet appareil.".into());
    }
    let players = {
        let list = state.profiles.lock().unwrap();
        profile_ids.iter().filter_map(|id| list.iter().find(|p| &p.id == id)).map(player_info).collect::<Vec<_>>()
    };
    replace_session(&state, None);
    let c = net::join(emitter(app), ip, port, code, players).await?;
    let s = Session::Client(c);
    let info = s.info();
    replace_session(&state, Some(s));
    Ok(info)
}

#[tauri::command]
fn session_stop(state: State<AppState>) {
    replace_session(&state, None);
}

#[tauri::command]
fn session_info(state: State<AppState>) -> SessionInfo {
    state.session.lock().unwrap().as_ref().map_or(SessionInfo::None, Session::info)
}

#[tauri::command]
fn game_action(state: State<AppState>, action: Action) -> Result<(), String> {
    let guard = state.session.lock().unwrap();
    guard.as_ref().ok_or("Aucune partie en cours.")?.act(action)
}

/// Niveau vocal du sujet local (0-1), relayé aux autres appareils pour le tracé en direct.
#[tauri::command]
fn live_level(state: State<AppState>, level: f32) {
    if let Some(s) = state.session.lock().unwrap().as_ref() {
        s.live(level);
    }
}

/// Halo lumineux sur les bords de l'écran. Renvoie `false` là où il n'existe pas
/// (mobile) : l'interface dessine alors le halo elle-même.
/// Asynchrone : créer une fenêtre depuis une commande synchrone bloque sous Windows.
#[tauri::command]
async fn edge_light(app: AppHandle, on: bool, strength: f32) -> Result<bool, String> {
    #[cfg(desktop)]
    {
        if on {
            edge::on(&app, if strength.is_finite() { strength } else { 0.5 })?;
        } else {
            edge::off(&app);
        }
        Ok(true)
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, on, strength);
        Ok(false)
    }
}

#[tauri::command]
fn game_state(state: State<AppState>) -> Option<GameView> {
    state.session.lock().unwrap().as_ref().and_then(Session::view)
}

#[tauri::command]
fn discovery_start(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let mut d = state.discovery.lock().unwrap();
    if d.is_none() {
        *d = Some(net::start_discovery(emitter(app))?);
    }
    Ok(())
}

#[tauri::command]
fn discovery_stop(state: State<AppState>) {
    if let Some(t) = state.discovery.lock().unwrap().take() {
        t.abort();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| {
            let state = app.state::<AppState>();
            let path = app.path().app_data_dir()?.join("profiles.json");
            if let Ok(bytes) = std::fs::read(&path) {
                match serde_json::from_slice::<Vec<PlayerProfile>>(&bytes) {
                    Ok(list) => *state.profiles.lock().unwrap() = list,
                    Err(e) => {
                        eprintln!("profiles.json illisible ({e}), sauvegarde en .bak");
                        let _ = std::fs::rename(&path, path.with_extension("json.bak"));
                    }
                }
            }
            *state.profiles_path.lock().unwrap() = Some(path);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Les bandes du halo sont des fenêtres : sans cela, elles garderaient
            // l'application ouverte après la fermeture de la fenêtre principale.
            #[cfg(desktop)]
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                edge::off(window.app_handle());
                window.app_handle().exit(0);
            }
            #[cfg(not(desktop))]
            let _ = (window, event);
        })
        .invoke_handler(tauri::generate_handler![
            profiles_list,
            profile_create,
            profile_delete,
            player_infos,
            calibration_protocol,
            analyze_audio,
            analyze_video,
            calibration_finish,
            score_answer,
            learn_answer,
            session_local,
            session_host,
            session_join,
            session_stop,
            session_info,
            game_action,
            game_state,
            live_level,
            edge_light,
            discovery_start,
            discovery_stop,
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de YouLiar");
}
