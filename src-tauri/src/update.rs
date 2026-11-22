//! Mises à jour : chaque tag `vx.y.z` poussé sur GitHub produit une release (voir
//! `.github/workflows/release.yml`).
//!
//! - Bureau : le greffon updater de Tauri lit `latest.json` dans la dernière release,
//!   vérifie la signature minisign avec la clé publique de `tauri.conf.json`, installe et
//!   relance l'application.
//! - Android : pas d'installation silencieuse possible hors Play Store ; on compare la
//!   dernière release à la version courante et on ouvre le lien de l'APK, qu'Android installe
//!   par-dessus (même clé de signature).
//!
//! L'interface est prévenue par l'événement `update-available` ; rien n'est installé sans
//! que le joueur le demande, pour ne jamais couper une partie.

use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// Dépôt public qui porte les releases.
#[cfg(mobile)]
const LATEST_RELEASE_API: &str = "https://api.github.com/repos/barracudapps/YouLiar/releases/latest";

/// Délai entre deux vérifications pendant que l'application reste ouverte.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
    /// `true` : installation dans l'application ; `false` : téléchargement à ouvrir.
    pub in_app: bool,
}

#[derive(Default)]
pub struct UpdateState {
    #[cfg(desktop)]
    pending: Mutex<Option<tauri_plugin_updater::Update>>,
    #[cfg(mobile)]
    pending: Mutex<Option<(UpdateInfo, String)>>,
    /// Empêche deux installations simultanées (double clic).
    installing: Mutex<bool>,
}

/// Lance la vérification au démarrage puis périodiquement. Les versions de développement
/// ne vérifient pas : elles se croiraient toujours en retard ou en avance.
pub fn spawn_checks(app: AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        // Laisser l'interface s'abonner aux événements avant d'annoncer quoi que ce soit.
        tokio::time::sleep(Duration::from_secs(5)).await;
        loop {
            match check(&app).await {
                Ok(Some(info)) => {
                    let _ = app.emit("update-available", info);
                }
                Ok(None) => {}
                Err(e) => eprintln!("vérification des mises à jour impossible : {e}"),
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

#[cfg(desktop)]
async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    let info = update.as_ref().map(|u| UpdateInfo { version: u.version.clone(), notes: u.body.clone(), in_app: true });
    *app.state::<UpdateState>().pending.lock().unwrap() = update;
    Ok(info)
}

#[cfg(mobile)]
async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    #[derive(serde::Deserialize)]
    struct Asset {
        browser_download_url: String,
    }
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
        body: Option<String>,
        #[serde(default)]
        assets: Vec<Asset>,
    }
    let release: Release = reqwest::Client::new()
        .get(LATEST_RELEASE_API)
        .header("User-Agent", "YouLiar")
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let apk = release.assets.into_iter().map(|a| a.browser_download_url).find(|u| u.ends_with(".apk"));
    let found = match (apk, newer_than(&release.tag_name, env!("CARGO_PKG_VERSION"))) {
        (Some(url), Some(version)) => Some((UpdateInfo { version, notes: release.body, in_app: false }, url)),
        _ => None,
    };
    let info = found.as_ref().map(|(i, _)| i.clone());
    *app.state::<UpdateState>().pending.lock().unwrap() = found;
    Ok(info)
}

/// Version de `tag` (`v1.2.3` ou `1.2.3`) si elle est strictement plus récente que `current`.
#[cfg_attr(not(mobile), allow(dead_code))]
fn newer_than(tag: &str, current: &str) -> Option<String> {
    let latest = semver::Version::parse(tag.trim_start_matches('v')).ok()?;
    let current = semver::Version::parse(current).ok()?;
    (latest > current).then(|| latest.to_string())
}

/// Vérification à la demande (bouton des réglages).
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    if cfg!(debug_assertions) {
        return Ok(None);
    }
    check(&app).await
}

/// Installe la mise à jour trouvée puis relance (bureau), ou ouvre l'APK (Android).
#[tauri::command]
pub async fn update_install(app: AppHandle, state: State<'_, UpdateState>) -> Result<(), String> {
    {
        let mut busy = state.installing.lock().unwrap();
        if *busy {
            return Err("Mise à jour déjà en cours.".into());
        }
        *busy = true;
    }
    let result = install(&app, &state).await;
    *state.installing.lock().unwrap() = false;
    result
}

#[cfg(desktop)]
async fn install(app: &AppHandle, state: &UpdateState) -> Result<(), String> {
    let update = state.pending.lock().unwrap().take().ok_or("Aucune mise à jour à installer.")?;
    let mut done = 0usize;
    let progress = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                done += chunk;
                if let Some(total) = total.filter(|t| *t > 0) {
                    let _ = progress.emit("update-progress", (done as f64 / total as f64).min(1.0));
                }
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    // Les halos sont des fenêtres à part : on les ferme avant de relancer.
    crate::edge::off(app);
    app.restart();
}

#[cfg(mobile)]
async fn install(app: &AppHandle, state: &UpdateState) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let url = state.pending.lock().unwrap().as_ref().map(|(_, url)| url.clone()).ok_or("Aucune mise à jour à installer.")?;
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::newer_than;

    #[test]
    fn compares_release_tags_with_the_running_version() {
        assert_eq!(newer_than("v0.11.0", "0.10.0").as_deref(), Some("0.11.0"));
        assert_eq!(newer_than("0.10.1", "0.10.0").as_deref(), Some("0.10.1"));
        assert_eq!(newer_than("v0.10.0", "0.10.0"), None);
        assert_eq!(newer_than("v0.9.9", "0.10.0"), None, "ordre numérique, pas alphabétique");
        assert_eq!(newer_than("nightly", "0.10.0"), None);
    }
}
