// Halo lumineux : s'allume quand la caméra voit un visage trop sombre.
//
// Bureau : bandes blanches sur les bords de l'écran (fenêtres gérées côté Rust).
// Mobile : l'application occupe déjà l'écran, le halo est dessiné dans l'interface.
//
// La lumière ne change jamais pendant un enregistrement ou une mesure de pouls :
// un saut d'éclairage fausserait le rythme cardiaque estimé par la caméra.

const DARK = 70;        // luminosité du visage (0-255) en dessous de laquelle on éclaire
const STILL_DARK = 60;  // halo allumé mais toujours trop sombre : on renforce
const MODES = ['auto', 'on', 'off'];

export class EdgeLight {
  constructor(media, { invoke, onAuto }) {
    this.media = media;
    this.invoke = invoke;
    this.onAuto = onAuto;
    this.mode = (() => { try { return localStorage.getItem('yl.light') ?? 'auto'; } catch { return 'auto'; } })();
    if (!MODES.includes(this.mode)) this.mode = 'auto';
    this.active = false;
    this.strength = 0;
    this.dark = 0;
    this.held = false;
    this.screenEdges = null; // inconnu tant que le cœur Rust n'a pas répondu
    this.glow = document.getElementById('edge-glow');
    setInterval(() => this.tick(), 1000);
  }

  setMode(mode) {
    this.mode = MODES.includes(mode) ? mode : 'auto';
    try { localStorage.setItem('yl.light', this.mode); } catch { /* stockage indisponible */ }
    this.tick();
  }

  /** Fige l'éclairage (mesure en cours). */
  hold(on) { this.held = on; }

  luma() {
    const w = this.media.videoWindow(1.5);
    if (!w) return null;
    return w.reduce((a, s) => a + 0.299 * s.r + 0.587 * s.g + 0.114 * s.b, 0) / w.length;
  }

  tick() {
    const luma = this.media.camera ? this.luma() : null;
    if (luma === null || this.mode === 'off') return this.apply(false, 0);
    if (this.mode === 'on') return this.apply(true, Math.max(this.strength, 0.7));
    if (this.held || this.media.recording) return;
    if (!this.active) {
      this.dark = luma < DARK ? this.dark + 1 : 0;
      if (this.dark >= 2) {
        // Plus c'est sombre, plus le halo est épais : 70 → 0.4, 20 → 1.
        this.apply(true, Math.min(1, 0.4 + (DARK - luma) / 80));
        this.onAuto?.();
      }
    } else if (luma < STILL_DARK && this.strength < 1) {
      this.apply(true, Math.min(1, this.strength + 0.15));
    }
    // Une fois allumé en mode auto, le halo reste allumé tant que la caméra tourne :
    // la lumière qu'il apporte fausserait toute décision de l'éteindre.
  }

  async apply(on, strength) {
    const s = on ? Math.round(strength * 20) / 20 : 0;
    if (on === this.active && s === this.strength) return;
    this.active = on;
    this.strength = s;
    if (!on) this.dark = 0;
    try {
      this.screenEdges = await this.invoke('edge_light', { on, strength: s });
    } catch {
      this.screenEdges = false;
    }
    // Sans bandes d'écran (mobile, ou échec), on éclaire depuis l'interface.
    const inApp = on && this.screenEdges === false;
    this.glow?.classList.toggle('on', inApp);
    this.glow?.style.setProperty('--glow', String(s));
  }
}

/** Sélecteur « Halo lumineux » réutilisable (salon, calibrage). */
export function lightControl(mode) {
  const opt = (v, l) => `<button role="radio" aria-checked="${mode === v}" class="${mode === v ? 'sel' : ''}" data-light="${v}">${l}</button>`;
  return `<div class="field"><span>💡 Halo lumineux si le visage est trop sombre</span>
    <div class="seg" role="radiogroup">${opt('auto', 'Auto')}${opt('on', 'Toujours')}${opt('off', 'Jamais')}</div></div>`;
}
