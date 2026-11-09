//! Halo lumineux sur le pourtour de l'écran, pour éclairer le visage quand la caméra
//! manque de lumière (même principe que l'anneau lumineux des visioconférences).
//!
//! Quatre fenêtres opaques fines (haut, bas, gauche, droite), toujours au premier plan
//! et transparentes aux clics, plutôt qu'une seule fenêtre plein écran transparente :
//! la transparence exige l'API privée de macOS (`macos-private-api`), incompatible avec
//! une diffusion sur l'App Store.

use std::sync::Mutex;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

const LABELS: [&str; 4] = ["edge-top", "edge-bottom", "edge-left", "edge-right"];

/// Position et taille de la fenêtre principale avant qu'on la recentre dans le halo.
static SAVED_MAIN: Mutex<Option<(PhysicalPosition<i32>, PhysicalSize<u32>)>> = Mutex::new(None);

/// Épaisseur logique du halo, de 40 px (léger) à 140 px (pleine puissance).
fn thickness(strength: f32, scale: f64) -> u32 {
    ((40.0 + 100.0 * f64::from(strength.clamp(0.0, 1.0))) * scale).round() as u32
}

/// Rectangle (x, y, largeur, hauteur) en pixels physiques.
type Rect = (i32, i32, u32, u32);

/// Les quatre bandes d'un cadre d'épaisseur `t` dans la zone `area`.
fn strips(area: Rect, t: u32) -> [Rect; 4] {
    let (x, y, w, h) = area;
    let ti = t as i32;
    [
        (x, y, w, t),
        (x, y + h as i32 - ti, w, t),
        (x, y + ti, t, h - 2 * t),
        (x + w as i32 - ti, y + ti, t, h - 2 * t),
    ]
}

/// Place une fenêtre (`outer` = taille avec barre de titre) à l'intérieur de `inner`.
/// Renvoie la nouvelle position et la réduction de taille à appliquer, ou `None` si elle
/// y tient déjà.
fn fit_inside(pos: (i32, i32), outer: (u32, u32), inner: Rect) -> Option<((i32, i32), (u32, u32))> {
    let (ix, iy, iw, ih) = inner;
    let (w, h) = (outer.0.min(iw), outer.1.min(ih));
    let x = pos.0.clamp(ix, ix + (iw - w) as i32);
    let y = pos.1.clamp(iy, iy + (ih - h) as i32);
    ((x, y) != pos || (w, h) != outer).then_some(((x, y), (outer.0 - w, outer.1 - h)))
}

pub fn off(app: &AppHandle) {
    for label in LABELS {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.close();
        }
    }
    if let (Some((pos, size)), Some(main)) = (SAVED_MAIN.lock().unwrap().take(), app.get_webview_window("main")) {
        let _ = main.set_size(size);
        let _ = main.set_position(pos);
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
    // Zone utile : sans la barre des menus ni le Dock, que les fenêtres ne peuvent pas couvrir
    // (macOS décale alors la bande du haut et le cadre se désaligne).
    let wa = monitor.work_area();
    let area: Rect = (wa.position.x, wa.position.y, wa.size.width, wa.size.height);
    let t = thickness(strength, monitor.scale_factor()).min(area.2 / 4).min(area.3 / 4);

    for (label, (x, y, w, h)) in LABELS.iter().zip(strips(area, t)) {
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
        win.set_size(PhysicalSize::new(w, h)).map_err(|e| e.to_string())?;
        win.show().map_err(|e| e.to_string())?;
    }

    // Le halo, toujours au premier plan, masquerait le haut de la fenêtre (bouton retour) :
    // on la ramène à l'intérieur du cadre, et on la remet en place à l'extinction.
    if !main.is_fullscreen().unwrap_or(false) {
        let (pos, outer, inner_size) = (
            main.outer_position().map_err(|e| e.to_string())?,
            main.outer_size().map_err(|e| e.to_string())?,
            main.inner_size().map_err(|e| e.to_string())?,
        );
        let ti = t as i32;
        let inside: Rect = (area.0 + ti, area.1 + ti, area.2 - 2 * t, area.3 - 2 * t);
        if let Some(((x, y), (shrink_w, shrink_h))) = fit_inside((pos.x, pos.y), (outer.width, outer.height), inside) {
            SAVED_MAIN.lock().unwrap().get_or_insert((pos, inner_size));
            if shrink_w > 0 || shrink_h > 0 {
                let _ = main.set_size(PhysicalSize::new(inner_size.width - shrink_w, inner_size.height - shrink_h));
            }
            let _ = main.set_position(PhysicalPosition::new(x, y));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thickness_follows_strength_and_scale() {
        assert_eq!(thickness(0.0, 1.0), 40);
        assert_eq!(thickness(1.0, 2.0), 280);
        assert_eq!(thickness(7.0, 1.0), 140, "puissance bornée");
    }

    #[test]
    fn strips_form_a_closed_frame_inside_the_work_area() {
        // Écran 3456×2234 dont la barre des menus occupe 66 px (cas réel mesuré).
        let s = strips((0, 66, 3456, 2168), 180);
        assert_eq!(s[0], (0, 66, 3456, 180));
        assert_eq!(s[1], (0, 66 + 2168 - 180, 3456, 180));
        assert_eq!(s[2], (0, 66 + 180, 180, 2168 - 360), "le côté gauche commence sous la bande du haut");
        assert_eq!(s[3].0 + s[3].2 as i32, 3456);
    }

    #[test]
    fn window_is_pulled_inside_the_frame() {
        let inner = (180, 246, 3096, 1808);
        // Fenêtre mesurée : y = 178, sous la bande du haut.
        assert_eq!(fit_inside((1298, 178), (960, 1720), inner), Some(((1298, 246), (0, 0))));
        // Trop haute pour le cadre : réduite.
        assert_eq!(fit_inside((200, 300), (960, 2000), inner), Some(((200, 246), (0, 192))));
        assert_eq!(fit_inside((500, 500), (960, 1000), inner), None);
    }
}
