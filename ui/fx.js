// Effets : sons synthétisés (aucun fichier), vibrations, écran maintenu allumé.

let ctx = null;
let muted = (() => { try { return localStorage.getItem('yl.muted') === 'true'; } catch { return false; } })();
export const reducedMotion = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;

function ac() {
  if (!ctx) {
    const AC = window.AudioContext || window.webkitAudioContext;
    if (!AC) return null;
    ctx = new AC();
  }
  if (ctx.state === 'suspended') ctx.resume().catch(() => {});
  return ctx;
}

function tone(freq, dur, { type = 'sine', gain = 0.18, slideTo = null, delay = 0 } = {}) {
  const c = ac();
  if (!c) return;
  const t = c.currentTime + delay;
  const o = c.createOscillator();
  const g = c.createGain();
  o.type = type;
  o.frequency.setValueAtTime(freq, t);
  if (slideTo) o.frequency.exponentialRampToValueAtTime(slideTo, t + dur);
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(gain, t + 0.015);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  o.connect(g).connect(c.destination);
  o.start(t);
  o.stop(t + dur + 0.05);
}

const SOUNDS = {
  tap: () => tone(660, 0.06, { gain: 0.08 }),
  vote: () => { tone(520, 0.07, { gain: 0.12 }); tone(780, 0.09, { gain: 0.12, delay: 0.07 }); },
  start: () => tone(220, 0.5, { type: 'triangle', slideTo: 880, gain: 0.12 }),
  rec: () => { tone(880, 0.08, { gain: 0.1 }); tone(880, 0.08, { gain: 0.1, delay: 0.14 }); },
  suspense: () => { for (let i = 0; i < 8; i++) tone(300 + i * 40, 0.08, { type: 'square', gain: 0.04, delay: i * 0.2 }); },
  lie: () => { tone(150, 0.32, { type: 'sawtooth', slideTo: 95, gain: 0.16 }); tone(150, 0.4, { type: 'sawtooth', slideTo: 80, gain: 0.16, delay: 0.38 }); },
  truth: () => [523, 659, 784].forEach((f, i) => tone(f, 0.35, { gain: 0.12, delay: i * 0.09 })),
  fooled: () => tone(420, 0.7, { type: 'triangle', slideTo: 160, gain: 0.14 }),
  win: () => [523, 659, 784, 1046, 784, 1046].forEach((f, i) => tone(f, 0.25, { type: 'triangle', gain: 0.12, delay: i * 0.12 })),
};

export const fx = {
  get muted() { return muted; },
  toggle() {
    muted = !muted;
    try { localStorage.setItem('yl.muted', String(muted)); } catch { /* ignoré */ }
    return muted;
  },
  play(name) { if (!muted) try { SOUNDS[name]?.(); } catch { /* audio indisponible */ } },
  buzz(pattern) { try { navigator.vibrate?.(pattern); } catch { /* non pris en charge */ } },
};

// Garde l'écran allumé pendant une partie (sinon le téléphone se verrouille en pleine manche).
let lock = null;
let wanted = false;
export async function keepAwake(on) {
  wanted = on;
  try {
    if (on && !lock && navigator.wakeLock) {
      lock = await navigator.wakeLock.request('screen');
      lock.addEventListener('release', () => { lock = null; });
    } else if (!on && lock) {
      await lock.release();
      lock = null;
    }
  } catch { /* refusé ou non pris en charge */ }
}
document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'visible' && wanted) keepAwake(true);
});
