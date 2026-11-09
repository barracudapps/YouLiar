//! Halo lumineux sur le pourtour de l'écran, pour éclairer le visage quand la caméra
//! manque de lumière (même principe que l'anneau lumineux des visioconférences).
//!
//! Quatre fenêtres opaques fines (haut, bas, gauche, droite), toujours au premier plan
//! et transparentes aux clics, plutôt qu'une seule fenêtre plein écran transparente :
//! la transparence exige l'API privée de macOS (`macos-private-api`), incompatible avec
//! une diffusion sur l'App Store.

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

const LABELS: [&str; 4] = ["edge-top", "edge-bottom", "edge-left", "edge-right"];

/// Épaisseur logique du halo, de 40 px (léger) à 140 px (pleine puissance).
fn thickness(strength: f32, scale: f64) -> u32 {
    ((40.0 + 100.0 * f64::from(strength.clamp(0.0, 1.0))) * scale).round() as u32
}

pub fn off(app: &AppHandle) {
    for label in LABELS {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.close();
        }
    }
}

pub fn on(app: &AppHandle, strength: f32) -> Result<(), String> {
    let main = app.get_webview_window("main").ok_or("Fenêtre principale introuvable.")?;
    let monitor = main
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| main.primary_monitor().ok().flatten())
        .ok_or("Écran introuvable.")?;
    let (pos, size) = (*monitor.position(), *monitor.size());
    let t = thickness(strength, monitor.scale_factor()).min(size.width / 4).min(size.height / 4);
    let (w, h) = (size.width, size.height);
    let ti = t as i32;
    let rects = [
        (pos.x, pos.y, w, t),
        (pos.x, pos.y + h as i32 - ti, w, t),
        (pos.x, pos.y + ti, t, h - 2 * t),
        (pos.x + w as i32 - ti, pos.y + ti, t, h - 2 * t),
    ];
    for (label, (x, y, rw, rh)) in LABELS.iter().zip(rects) {
        let win = match app.get_webview_window(label) {
            Some(win) => win,
            None => {
                let win = WebviewWindowBuilder::new(app, *label, WebviewUrl::App("edge.html".into()))
                    .title("YouLiar – halo lumineux")
                    .decorations(false)
                    .resizable(false)
                    .always_on_top(true)
                    .skip_taskbar(true)
                    .shadow(false)
                    .focused(false)
                    .visible(false)
                    .visible_on_all_workspaces(true)
                    .build()
                    .map_err(|e| e.to_string())?;
                // Le halo ne doit jamais intercepter un clic destiné à ce qui est dessous.
                let _ = win.set_ignore_cursor_events(true);
                win
            }
        };
        win.set_position(PhysicalPosition::new(x, y)).map_err(|e| e.to_string())?;
        win.set_size(PhysicalSize::new(rw, rh)).map_err(|e| e.to_string())?;
        win.show().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::thickness;

    #[test]
    fn thickness_follows_strength_and_scale() {
        assert_eq!(thickness(0.0, 1.0), 40);
        assert_eq!(thickness(1.0, 2.0), 280);
        assert_eq!(thickness(7.0, 1.0), 140, "puissance bornée");
    }
}
