import { Media, speak, Trace } from './media.js';
import { fx, keepAwake, reducedMotion } from './fx.js';
import { EdgeLight, lightControl } from './light.js';

const TAURI = window.__TAURI__;
const app = document.getElementById('app');
const backBtn = document.getElementById('back');
const badge = document.getElementById('session-badge');
const muteBtn = document.getElementById('mute');

const AVATARS = ['🦊', '🐼', '🐸', '🦁', '🐙', '🦄', '🐯', '🐵', '🐧', '🦉', '🐲', '👽', '🤖', '👻', '🎃', '🌵'];
const PACKS = { soft: '🍬 Léger', party: '🎉 Soirée', deep: '🌊 Profond', mix: '🎲 Mélange' };

const media = new Media();
const light = new EdgeLight(media, {
  invoke: (cmd, args) => (TAURI ? TAURI.core.invoke(cmd, args) : Promise.reject(new Error('no core'))),
  onAuto: () => toast('💡 Peu de lumière : halo lumineux activé'),
});
const S = {
  screen: 'home',
  profiles: [],
  view: null,
  session: { mode: 'none' },
  discovered: [],
  answers: {},        // n° de manche -> { profileId, audio, video, learned }
  revealed: {},       // n° de manche -> animation de révélation déjà jouée
  flowKey: null,      // empêche de re-dessiner l'écran pendant un enregistrement
  live: { player: null, level: 0, at: 0 },
  useCamera: pref('camera', 'true') === 'true',
};

// ---------- Utilitaires ----------

function pref(k, d) { try { return localStorage.getItem('yl.' + k) ?? d; } catch { return d; } }
function setPref(k, v) { try { localStorage.setItem('yl.' + k, v); } catch { /* stockage indisponible */ } }

const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const $ = (sel, root = app) => root.querySelector(sel);
const $$ = (sel, root = app) => [...root.querySelectorAll(sel)];
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

let toastTimer;
function toast(msg, error = false) {
  const t = document.getElementById('toast');
  t.textContent = msg;
  t.className = error ? 'error' : '';
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), error ? 4500 : 2800);
}

async function call(cmd, args, opts) {
  try {
    return await TAURI.core.invoke(cmd, args, opts);
  } catch (e) {
    toast(typeof e === 'string' ? e : e?.message ?? String(e), true);
    throw e;
  }
}

const act = (action) => call('game_action', { action }).catch(() => {});

function playerById(id) { return S.view?.players.find((p) => p.id === id); }
function pname(id) { const p = playerById(id); return p ? `${esc(p.avatar)} ${esc(p.name)}` : '?'; }
function nameOf(id) { return esc(playerById(id)?.name ?? '?'); }
const isMine = (id) => !!id && !!playerById(id)?.mine;

async function loadProfiles() { S.profiles = await call('profiles_list'); }

function go(screen, params = {}) {
  if (S.screen === 'join' && screen !== 'join') call('discovery_stop').catch(() => {});
  S.screen = screen;
  S.params = params;
  S.flowKey = null;
  backBtn.hidden = screen === 'home';
  keepAwake(screen === 'game' || screen === 'calibrate');
  render();
  window.scrollTo(0, 0);
}

function render() {
  const fn = SCREENS[S.screen] ?? SCREENS.home;
  fn(S.params ?? {});
}

let leaveArmed = 0;
backBtn.onclick = async () => {
  if (S.screen === 'game' && S.session.mode !== 'none') {
    if (Date.now() - leaveArmed > 2500) { leaveArmed = Date.now(); return toast('Appuie encore pour quitter la partie.'); }
    await leaveSession();
  }
  if (S.screen === 'calibrate') media.cancelRecording?.();
  const back = { calibrate: S.params.returnTo ?? 'profiles', rules: S.params.returnTo ?? 'home' }[S.screen] ?? 'home';
  go(back);
};

async function leaveSession() {
  await call('session_stop').catch(() => {});
  media.stop();
  S.session = { mode: 'none' }; S.view = null; S.answers = {}; S.revealed = {};
  updateBadge();
}

function updateBadge() {
  const m = S.session.mode;
  badge.hidden = m === 'none';
  badge.classList.remove('warn');
  badge.textContent = { local: '📱 Local', host: '📡 Hôte', client: '🔗 Invité' }[m] ?? '';
}

function updateMute() {
  muteBtn.textContent = fx.muted ? '🔇' : '🔊';
  muteBtn.setAttribute('aria-label', fx.muted ? 'Activer le son' : 'Couper le son');
}
muteBtn.onclick = () => { fx.toggle(); updateMute(); fx.play('tap'); };
updateMute();

// Petit retour sonore / tactile sur chaque bouton.
document.addEventListener('click', (e) => {
  if (e.target.closest('button:not(:disabled)')) { fx.play('tap'); fx.buzz(8); }
}, true);

// ---------- Composants ----------

function profilePicker(selected = [], name = 'pick') {
  if (!S.profiles.length) return `<p class="muted">Aucun joueur pour l'instant.</p>`;
  return S.profiles.map((p) => `
    <label class="check player">
      <input type="checkbox" name="${name}" value="${esc(p.id)}" ${selected.includes(p.id) ? 'checked' : ''}>
      <span class="avatar">${esc(p.avatar)}</span>
      <span class="name">${esc(p.name)}</span>
      <span class="tag">${p.calibrated ? '✓ calibré' : '<span class="warn">à calibrer</span>'}</span>
    </label>`).join('');
}
const picked = (name = 'pick') => $$(`input[name="${name}"]:checked`).map((i) => i.value);

function bindCommon() {
  $$('[data-goto]').forEach((b) => (b.onclick = () => go(b.dataset.goto, b.dataset.return ? { returnTo: b.dataset.return } : {})));
  $$('[data-light]').forEach((b) => (b.onclick = (e) => {
    e.preventDefault();
    light.setMode(b.dataset.light);
    $$('[data-light]').forEach((x) => { x.classList.toggle('sel', x === b); x.setAttribute('aria-checked', String(x === b)); });
  }));
}

const gaugeColor = (i) => (i >= 65 ? 'var(--lie)' : i >= 50 ? 'var(--warn)' : 'var(--truth)');

/** Jauge à aiguille ; l'aiguille est une rotation CSS, donc animable. */
function gauge(index, { scanning = false } = {}) {
  const deg = -90 + index * 1.8;
  return `<div class="gauge ${scanning ? 'scanning' : ''}"><svg viewBox="0 0 220 125" aria-hidden="true">
    <defs><linearGradient id="gg"><stop offset="0" stop-color="#3ddc97"/><stop offset=".5" stop-color="#ffc857"/><stop offset="1" stop-color="#ff4d6d"/></linearGradient></defs>
    <path d="M20 110 A90 90 0 0 1 200 110" fill="none" stroke="url(#gg)" stroke-width="14" stroke-linecap="round" opacity=".9"/>
    <g class="needle" style="transform: rotate(${scanning ? -90 : deg}deg)"><line x1="110" y1="110" x2="110" y2="28" stroke="#fff" stroke-width="4" stroke-linecap="round"/></g>
    <circle cx="110" cy="110" r="8" fill="#fff"/></svg>
    <div class="value" style="color:${gaugeColor(index)}">${scanning ? '…' : index}</div></div>`;
}

function describePitch(hz) {
  if (!hz) return '';
  return hz < 135 ? 'grave' : hz < 190 ? 'médium' : 'aiguë';
}

/** Carte d'identité vocale d'un profil. */
function idCard(p) {
  const t = p.traits ?? {};
  const rows = [
    t.pitch_hz && ['🎵', 'Voix', `${Math.round(t.pitch_hz)} Hz · ${describePitch(t.pitch_hz)}`],
    t.latency_s && ['⏱️', 'Temps de réponse', `${t.latency_s.toFixed(1)} s`],
    t.speech_rate && ['🗣️', 'Débit', `${t.speech_rate.toFixed(1)} syllabes/s`],
    t.pauses_pct != null && ['🤐', 'Silences', `${Math.round(t.pauses_pct)} %`],
    p.resting_hr && ['♥', 'Pouls de repos', `${Math.round(p.resting_hr)} bpm`],
  ].filter(Boolean);
  if (!rows.length) return '';
  return `<div class="idcard">${rows.map(([i, l, v]) => `<div><span>${i} ${l}</span><b>${esc(v)}</b></div>`).join('')}</div>`;
}

function statsLine(p) {
  const s = p.stats;
  if (!s?.answers) return '';
  return `${s.answers} réponse(s) · ${s.lies ? `démasqué ${s.lies_caught}/${s.lies} mensonges · ` : ''}machine bernée ${s.machine_fooled} fois`;
}

const DISCLAIMER = `<p class="disclaimer">🎲 YouLiar est un jeu. La machine mesure des signes de stress et des écarts à tes habitudes (voix, temps de réponse, pouls estimé, agitation) — aucun appareil ne détecte réellement le mensonge. Voix et image sont analysées sur l'appareil et ne sont jamais envoyées.</p>`;

// ---------- Capture d'une réponse ----------

/** Enregistre une réponse dans `box`. Renvoie { audio, video } ou null (annulé). */
async function captureAnswer(box, { prompt, readAloud, live = false }) {
  await media.resume();
  box.innerHTML = `
    <p class="center status"></p>
    <canvas class="trace"></canvas>
    <div class="row"><button class="ghost grow cancel">Annuler</button><button class="primary grow stop" hidden>J'ai fini</button></div>`;
  const status = $('.status', box);
  const trace = new Trace($('.trace', box), () => (media.recording ? media.level * 8 : 0.02));
  let cancelled = false;
  $('.cancel', box).onclick = () => { window.speechSynthesis?.cancel(); media.cancelRecording?.(); cancelled = true; };
  if (readAloud && prompt) {
    status.innerHTML = '🔊 La machine pose la question…';
    await speak(prompt);
    if (cancelled) { trace.stop(); return null; }
  }
  fx.play('rec');
  fx.buzz(30);
  status.innerHTML = '<span class="rec-dot"></span>Réponds maintenant, à voix haute';
  $('.stop', box).hidden = false;
  $('.stop', box).onclick = () => media.stopRecording?.();
  // Tracé en direct sur les autres appareils : ~10 niveaux/s, jamais d'audio.
  let lastSent = 0;
  const rec = await media.record({
    onLevel: (lvl) => {
      const now = performance.now();
      if (live && now - lastSent > 100) {
        lastSent = now;
        TAURI.core.invoke('live_level', { level: Math.min(1, lvl * 8) }).catch(() => {});
      }
    },
  });
  if (live) TAURI.core.invoke('live_level', { level: 0 }).catch(() => {});
  trace.stop();
  if (rec.cancelled || cancelled) return null;
  status.innerHTML = '<span class="pulse">🧠 Analyse en cours…</span>';
  $('.stop', box).hidden = true;
  const audio = await call('analyze_audio', new Uint8Array(rec.samples.buffer), {
    headers: { 'x-sample-rate': String(rec.sampleRate), 'x-prompt-offset': '0' },
  });
  const dur = rec.samples.length / rec.sampleRate;
  const vs = media.videoWindow(Math.max(12, dur + 4));
  const video = vs ? await call('analyze_video', { samples: vs }) : null;
  return { audio, video };
}

// ---------- Écrans ----------

const SCREENS = {};

SCREENS.home = () => {
  const first = !S.profiles.length;
  const calibrated = S.profiles.filter((p) => p.calibrated).length;
  app.innerHTML = `
    <section class="card hero">
      <div class="logo" aria-hidden="true">🕵️</div>
      <h2>Le détecteur de mensonges de soirée</h2>
      <p class="muted">Calibre ta voix, réponds aux questions… et tente de berner la machine et tes amis.</p>
      ${first ? '<button class="primary big" data-goto="profiles">Commencer : crée ton joueur</button>' : ''}
    </section>
    <nav class="menu">
      <button data-goto="local" ${calibrated ? '' : 'class="dim"'}><span class="emoji">👥</span><span>Partie sur cet appareil<small>Mode jeu ou libre · on se passe le téléphone</small></span></button>
      <button data-goto="host" ${calibrated ? '' : 'class="dim"'}><span class="emoji">📡</span><span>Créer une partie réseau<small>Mode jeu ou libre · chacun son appareil</small></span></button>
      <button data-goto="join"><span class="emoji">🔗</span><span>Rejoindre une partie<small>Découverte automatique ou adresse IP</small></span></button>
      <button data-goto="profiles"><span class="emoji">🎙️</span><span>Joueurs & calibrage<small>${S.profiles.length} profil(s) · ${calibrated} calibré(s)</small></span></button>
      <button data-goto="rules"><span class="emoji">📖</span><span>Comment jouer<small>Mode jeu, mode libre, points</small></span></button>
    </nav>
    ${DISCLAIMER}`;
  bindCommon();
};

SCREENS.rules = () => {
  app.innerHTML = `
    <h2>📖 Comment jouer</h2>
    <section class="card">
      <h3>Le principe</h3>
      <p>Chacun calibre d'abord sa voix (≈ 3 min, une seule fois). La machine apprend ton ton, ton débit, ton temps de réponse… puis comment tu réagis quand tu mens.</p>
    </section>
    <section class="card steps">
      <h3>🎮 Une manche (mode jeu)</h3>
      <div><span>🔥</span><p><b>Sur la sellette.</b> Un joueur reçoit en secret sa consigne : <span class="t-lie">MENS</span> ou <span class="t-truth">DIS LA VÉRITÉ</span> (ou la choisit, selon le réglage).</p></div>
      <div><span>🎤</span><p><b>L'interrogateur</b> (le joueur suivant) choisit parmi 3 questions, ou écrit la sienne.</p></div>
      <div><span>🎙️</span><p><b>Réponse à voix haute.</b> Tout le monde voit l'aiguille du polygraphe s'agiter en direct.</p></div>
      <div><span>🗳️</span><p><b>Vote.</b> Mensonge ou vérité ? Tu peux parier « sûr de moi » pour doubler la mise.</p></div>
      <div><span>🤖</span><p><b>Verdict</b> de la machine, puis la vérité éclate.</p></div>
    </section>
    <section class="card">
      <h3>🎤 Le mode libre</h3>
      <p>Pas de manches ni de points : c'est une <b>salle d'interrogatoire</b>. N'importe qui choisit un joueur et lui pose la question qu'il veut (ou pioche une idée). La personne interrogée ment ou dit vrai, à sa guise ; la machine rend son verdict aussitôt. Ensuite, elle peut <b>avouer</b>… ou garder le secret.</p>
    </section>
    <section class="card">
      <h3>Les points (mode jeu)</h3>
      <table class="points">
        <tr><td>Vote juste</td><td>+1</td></tr>
        <tr><td>Vote juste « sûr de moi »</td><td>+2</td></tr>
        <tr><td>Vote faux « sûr de moi »</td><td>−1</td></tr>
        <tr><td>Sujet : par joueur trompé</td><td>+1</td></tr>
        <tr><td>Sujet : machine bernée</td><td>+2</td></tr>
        <tr><td>Interrogateur : menteur démasqué par la machine</td><td>+1</td></tr>
      </table>
    </section>
    <section class="card">
      <h3>Pour une machine redoutable</h3>
      <p>• Calibre au calme, à 30-50 cm du micro, en parlant naturellement.<br>
      • Caméra : visage bien éclairé de face, dans l'ovale.<br>
      • Plus vous jouez, plus elle apprend vos tics : chaque manche révélée l'entraîne.</p>
    </section>
    ${DISCLAIMER}`;
};

SCREENS.profiles = () => {
  let avatar = AVATARS[Math.floor(Math.random() * AVATARS.length)];
  app.innerHTML = `
    <h2>Joueurs</h2>
    <section class="stack">
      ${S.profiles.map((p) => `
        <div class="card profile">
          <div class="row">
            <span class="avatar big">${esc(p.avatar)}</span>
            <div class="grow"><b>${esc(p.name)}</b><br><span class="tag">${p.calibrated
              ? `✓ calibré${p.camera_calibrated ? ' · 📷' : ''}${p.answers_learned ? ` · ${p.answers_learned} réponses apprises` : ''}`
              : '<span class="warn">à calibrer</span>'}</span></div>
            <button class="${p.calibrated ? 'ghost' : 'primary'}" data-cal="${esc(p.id)}">${p.calibrated ? 'Recalibrer' : 'Calibrer'}</button>
            <button class="ghost" data-del="${esc(p.id)}" aria-label="Supprimer ${esc(p.name)}">🗑</button>
          </div>
          ${p.calibrated ? idCard(p) : ''}
          ${statsLine(p) ? `<p class="small muted">🏁 ${statsLine(p)}</p>` : ''}
        </div>`).join('') || '<p class="muted">Aucun joueur pour l\'instant.</p>'}
    </section>
    <section class="card">
      <h3>Nouveau joueur</h3>
      <input type="text" id="pname" placeholder="Prénom" maxlength="24" autocomplete="off" aria-label="Prénom">
      <div class="avatars" role="radiogroup" aria-label="Avatar">${AVATARS.map((a) => `<button data-av="${a}" role="radio" aria-checked="${a === avatar}" class="${a === avatar ? 'sel' : ''}">${a}</button>`).join('')}</div>
      <button class="primary" id="create">Créer et calibrer</button>
    </section>`;
  $$('[data-av]').forEach((b) => (b.onclick = () => {
    avatar = b.dataset.av;
    $$('[data-av]').forEach((x) => { x.classList.toggle('sel', x === b); x.setAttribute('aria-checked', String(x === b)); });
  }));
  $('#pname').onkeydown = (e) => { if (e.key === 'Enter') $('#create').click(); };
  $('#create').onclick = async () => {
    const p = await call('profile_create', { name: $('#pname').value, avatar }).catch(() => null);
    if (!p) return;
    await loadProfiles();
    go('calibrate', { id: p.id });
  };
  $$('[data-cal]').forEach((b) => (b.onclick = () => go('calibrate', { id: b.dataset.cal })));
  let armed = null;
  $$('[data-del]').forEach((b) => (b.onclick = async () => {
    if (armed !== b.dataset.del) { armed = b.dataset.del; return toast('Appuie encore pour supprimer ce joueur.'); }
    await call('profile_delete', { id: b.dataset.del });
    await loadProfiles();
    render();
  }));
};

SCREENS.calibrate = ({ id, returnTo }) => {
  const profile = S.profiles.find((p) => p.id === id);
  if (!profile) return go('profiles');
  const W = { steps: [], i: 0, takes: [], resting: null, camera: S.useCamera };
  const head = () => `<h2>${esc(profile.avatar)} Calibrage de ${esc(profile.name)}</h2>`;

  const phases = {
    intro() {
      app.innerHTML = `${head()}
        <section class="card">
          <p>La machine va apprendre <b>ta</b> façon de parler : ta voix, ton ton habituel, ton temps de réponse… puis comment tu réagis quand tu mens.</p>
          <p class="muted small">≈ 3 minutes · endroit calme · parle normalement, à 30-50 cm de l'appareil.</p>
          <label class="check"><input type="checkbox" id="cam" ${W.camera ? 'checked' : ''}> Utiliser aussi la caméra (pouls estimé & agitation)</label>
          <p class="muted small">La caméra est facultative. Les images ne quittent pas l'appareil : seules la couleur moyenne du visage et la quantité de mouvement sont mesurées.</p>
          <button class="primary big" id="go">Commencer</button>
        </section>${DISCLAIMER}`;
      $('#go').onclick = async () => {
        W.camera = $('#cam').checked;
        S.useCamera = W.camera; setPref('camera', String(W.camera));
        try {
          await media.start({ camera: W.camera });
          await media.resume();
        } catch (e) { return toast(e.message, true); }
        if (W.camera && !media.camera) toast('Caméra indisponible : calibrage vocal uniquement.', true);
        W.camera = media.camera;
        W.steps = await call('calibration_protocol');
        show('mic');
      };
    },
    mic() {
      app.innerHTML = `${head()}
        <section class="card">
          <h3>1. Test du micro</h3>
          <p>Dis quelques mots : la jauge doit bouger.</p>
          <div class="meter" role="meter" aria-label="Niveau du micro"><div id="lvl"></div></div>
          <button class="primary" id="ok" disabled>Le micro m'entend →</button>
        </section>`;
      let heard = false;
      const tick = () => {
        if (!$('#lvl')) return;
        const v = Math.min(100, media.level * 600);
        $('#lvl').style.width = v + '%';
        if (v > 12 && !heard) { heard = true; $('#ok').disabled = false; fx.play('vote'); }
        requestAnimationFrame(tick);
      };
      tick();
      $('#ok').onclick = () => show(W.camera ? 'camera' : 'step');
    },
    camera() {
      app.innerHTML = `${head()}
        <section class="card">
          <h3>2. Calibrage caméra</h3>
          <p>Place ton visage dans l'ovale, bien éclairé de face, et reste immobile pendant 15 secondes.</p>
          <div class="camera-box" id="cbox"><div class="oval"></div></div>
          <p class="center small" id="cinfo"></p>
          ${lightControl(light.mode)}
          <div class="progress"><div id="cprog" style="width:0"></div></div>
          <div class="row"><button class="ghost grow" id="skip">Passer</button><button class="primary grow" id="start">Démarrer (15 s)</button></div>
        </section>`;
      media.attachPreview($('#cbox'));
      bindCommon();
      const info = setInterval(() => {
        const w = media.videoWindow(1);
        if (!$('#cinfo')) return clearInterval(info);
        if (!w) return;
        const l = w.reduce((a, s) => a + .299 * s.r + .587 * s.g + .114 * s.b, 0) / w.length;
        const m = w.reduce((a, s) => a + s.motion, 0) / w.length;
        const dark = light.active ? '💡 Halo allumé — si c\'est encore sombre, rapproche-toi d\'une lumière.' : light.mode === 'off' ? '💡 Trop sombre : rapproche-toi d\'une lumière.' : '💡 Un peu sombre : le halo lumineux va s\'allumer…';
        $('#cinfo').innerHTML = l < 60 ? `<span class="warn">${dark}</span>` : m > 4 ? '<span class="warn">🫨 Ça bouge beaucoup…</span>' : '✓ Image correcte';
      }, 500);
      $('#skip').onclick = () => { clearInterval(info); show('step'); };
      $('#start').onclick = async () => {
        $('#start').disabled = true; $('#skip').disabled = true;
        light.hold(true);
        const t0 = performance.now();
        await new Promise((res) => {
          const iv = setInterval(() => {
            const p = (performance.now() - t0) / 15000;
            if ($('#cprog')) $('#cprog').style.width = Math.min(100, p * 100) + '%';
            if (p >= 1) { clearInterval(iv); res(); }
          }, 200);
        });
        clearInterval(info);
        light.hold(false);
        const vs = media.videoWindow(15);
        W.resting = vs ? await call('analyze_video', { samples: vs }) : null;
        const hr = W.resting?.heart_rate;
        $('#cinfo').innerHTML = hr ? `♥ Pouls estimé : <b>${Math.round(hr)} bpm</b>` : '<span class="warn">Signal de pouls trop faible (lumière ? mouvement ?). L\'agitation sera quand même mesurée.</span>';
        $('#start').textContent = 'Continuer →';
        $('#start').disabled = false;
        $('#start').onclick = () => show('step');
      };
    },
    step() {
      if (W.i >= W.steps.length) return show('finish');
      const st = W.steps[W.i];
      const answer = st.kind === 'truth' || st.kind === 'lie';
      app.innerHTML = `${head()}
        <div class="progress" role="progressbar" aria-valuenow="${W.i}" aria-valuemax="${W.steps.length}"><div style="width:${(W.i / W.steps.length) * 100}%"></div></div>
        <section class="card">
          <p class="muted small center">Étape ${W.i + 1} / ${W.steps.length}</p>
          <p class="instruction ${st.kind}">${st.kind === 'truth' ? '😇 ' : st.kind === 'lie' ? '🤥 ' : '📖 '}${esc(st.instruction)}</p>
          <p class="question">${esc(st.prompt)}</p>
          <div id="rec"><button class="primary big full" id="ready">🎙️ ${answer ? 'Prêt, pose-moi la question' : 'Commencer'}</button></div>
        </section>
        <p class="muted small center">${answer ? 'La question sera lue à voix haute, puis réponds naturellement en une phrase.' : 'L\'enregistrement s\'arrête seul quand tu as fini.'}</p>`;
      $('#ready').onclick = async () => {
        const res = await captureAnswer($('#rec'), { prompt: st.prompt, readAloud: answer }).catch(() => null);
        if (!res) return show('step');
        if (!res.audio.speech_detected) { toast('Je n\'ai rien entendu… recommence plus près du micro.', true); return show('step'); }
        W.takes.push({ kind: st.kind, audio: res.audio, video: res.video });
        W.i++;
        fx.play('vote');
        show('step');
      };
    },
    async finish() {
      app.innerHTML = `${head()}<section class="card hero"><p class="pulse">🧠 Construction de ton profil…</p></section>`;
      try {
        await call('calibration_finish', { profileId: id, takes: W.takes, resting: W.resting });
        await loadProfiles();
        const p = S.profiles.find((x) => x.id === id);
        // Si une partie est en cours, on met à jour le joueur côté hôte.
        if (S.session.mode !== 'none' && playerById(id)) {
          const infos = await call('player_infos', { ids: [id] });
          await act({ type: 'add_players', players: infos });
        }
        fx.play('truth');
        app.innerHTML = `${head()}
          <section class="card hero">
            <div class="logo">✅</div>
            <h3>Profil calibré !</h3>
            <p class="muted small">Ta carte d'identité vocale :</p>
            ${idCard(p)}
            <p class="small muted">${W.takes.length} prises${p.has_voice ? ' · empreinte vocale enregistrée' : ''}. La machine continuera d'apprendre à chaque manche jouée.</p>
            <button class="primary big" id="done">Terminé</button>
          </section>`;
        $('#done').onclick = () => go(returnTo ?? 'profiles');
      } catch {
        app.innerHTML = `${head()}<section class="card"><p>Le calibrage n'a pas abouti.</p><button class="primary" id="again">Recommencer</button></section>`;
        $('#again').onclick = () => go('calibrate', { id, returnTo });
      }
    },
  };
  const show = (ph) => phases[ph]();
  show('intro');
};

function setupScreen({ title, intro, network }) {
  const pre = S.profiles.filter((p) => p.calibrated).map((p) => p.id).slice(0, network ? 1 : 12);
  app.innerHTML = `
    <h2>${title}</h2>
    <p class="muted">${intro}</p>
    ${network ? `<label class="field"><span>Nom de la partie</span><input type="text" id="gname" maxlength="40" value="${esc(pref('gname', 'Soirée YouLiar'))}"></label>` : ''}
    <section class="card"><h3>Joueurs sur cet appareil</h3>${profilePicker(pre)}
      <button class="ghost" data-goto="profiles">＋ Nouveau joueur</button></section>
    <label class="check"><input type="checkbox" id="cam" ${S.useCamera ? 'checked' : ''}> Utiliser la caméra de cet appareil</label>
    <div class="sticky-cta"><button class="primary big full" id="go">${network ? 'Créer la partie' : 'Ouvrir le salon'}</button></div>`;
  bindCommon();
  $('#go').onclick = async () => {
    const ids = picked();
    if (!ids.length) return toast('Choisis au moins un joueur.', true);
    S.useCamera = $('#cam').checked; setPref('camera', String(S.useCamera));
    try {
      if (network) {
        setPref('gname', $('#gname').value);
        const info = await call('session_host', { gameName: $('#gname').value || 'Soirée YouLiar' });
        S.session = { mode: 'host', info };
      } else {
        await call('session_local');
        S.session = { mode: 'local' };
      }
      updateBadge();
      await act({ type: 'add_players', players: await call('player_infos', { ids }) });
      go('game');
    } catch { /* message déjà affiché */ }
  };
}

SCREENS.local = () => setupScreen({ title: '👥 Partie sur cet appareil', intro: 'Coche les joueurs présents. Chacun se passera l\'appareil pour répondre ; la machine vérifie même que c\'est bien la bonne voix !', network: false });
SCREENS.host = () => setupScreen({ title: '📡 Créer une partie réseau', intro: 'Les autres appareils doivent être sur le même réseau : même Wi-Fi, ou connectés au partage de connexion (point d\'accès) de l\'un d\'entre vous.', network: true });

SCREENS.join = () => {
  const pre = S.profiles.filter((p) => p.calibrated).map((p) => p.id).slice(0, 1);
  app.innerHTML = `
    <h2>🔗 Rejoindre une partie</h2>
    <section class="card"><h3>Joueur(s) sur cet appareil</h3>${profilePicker(pre)}
      <button class="ghost" data-goto="profiles">＋ Nouveau joueur</button></section>
    <section class="card"><h3>Parties à proximité</h3><div id="found" class="stack"></div></section>
    <section class="card">
      <h3>Connexion</h3>
      <div class="row">
        <label class="field grow"><span>Adresse IP de l'hôte</span><input type="text" id="ip" inputmode="decimal" placeholder="192.168.1.20" value="${esc(pref('lastip', ''))}"></label>
        <label class="field" style="width:110px"><span>Port</span><input type="number" id="port" value="47820"></label>
      </div>
      <label class="field"><span>Code de la partie (6 chiffres, affiché chez l'hôte)</span><input type="text" id="code" inputmode="numeric" autocomplete="off" maxlength="6" placeholder="000000"></label>
      <p class="muted small">🔒 Le code n'est jamais envoyé sur le réseau : il sert à établir une connexion chiffrée avec l'hôte.</p>
      <label class="check"><input type="checkbox" id="cam" ${S.useCamera ? 'checked' : ''}> Utiliser la caméra de cet appareil</label>
      <button class="primary big" id="go">Rejoindre</button>
    </section>`;
  bindCommon();
  renderFound();
  call('discovery_start').catch(() => {});
  $('#code').oninput = (e) => { e.target.value = e.target.value.replace(/\D/g, ''); if (e.target.value.length === 6) $('#go').focus(); };
  $('#go').onclick = async () => {
    const ids = picked();
    if (!ids.length) return toast('Choisis au moins un joueur.', true);
    S.useCamera = $('#cam').checked; setPref('camera', String(S.useCamera));
    const ip = $('#ip').value.trim();
    setPref('lastip', ip);
    $('#go').disabled = true;
    $('#go').textContent = 'Connexion…';
    try {
      await call('session_join', { ip, port: Number($('#port').value) || 47820, code: $('#code').value.trim(), profileIds: ids });
      S.session = { mode: 'client' };
      updateBadge();
      fx.play('start');
      go('game');
    } catch {
      if ($('#go')) { $('#go').disabled = false; $('#go').textContent = 'Rejoindre'; }
    }
  };
};

function renderFound() {
  const el = $('#found');
  if (!el) return;
  el.innerHTML = S.discovered.length
    ? S.discovered.map((g, i) => `
        <button class="player ghost" data-g="${i}" ${g.open ? '' : 'disabled'}>
          <span class="avatar">📡</span>
          <span class="name">${esc(g.name)}<br><span class="tag">${esc(g.host_name || g.ip)} · ${g.players} joueur(s)${g.open ? '' : ' · déjà commencée'}</span></span>
        </button>`).join('')
    : '<p class="muted small pulse">Recherche sur le réseau local… (si rien n\'apparaît, saisis l\'adresse IP affichée chez l\'hôte)</p>';
  $$('[data-g]', el).forEach((b) => (b.onclick = () => {
    const g = S.discovered[Number(b.dataset.g)];
    $('#ip').value = g.ip; $('#port').value = g.port;
    $('#code').focus();
  }));
}

// ---------- Partie ----------

SCREENS.game = () => renderGame(true);

function renderGame(force = false) {
  const v = S.view;
  if (!v) { app.innerHTML = '<section class="card hero"><p class="pulse">Connexion…</p></section>'; return; }
  const r = v.round;
  const mineSubject = r && isMine(r.subject);
  // Ne pas interrompre l'enregistrement en cours.
  const key = `${v.stage}:${r?.number ?? 0}`;
  if (!force && S.flowKey && S.flowKey === key) return;
  S.flowKey = null;

  // Conserve la saisie en cours lors d'un rafraîchissement.
  const kept = $$('[data-keep]').map((i) => [i.id, i.value]);

  if (v.stage !== 'lobby' && v.stage !== 'finished' && !media.ready) {
    media.start({ camera: S.useCamera }).catch((e) => toast(e.message, true));
  }

  const free = v.settings.mode === 'free';
  const header = free && v.stage !== 'lobby' && v.stage !== 'finished' ? `
    <div class="row small muted free-head"><span class="grow">🎤 Mode libre${v.round_no ? ` · question n° ${v.round_no}` : ''}</span><span title="Verdicts justes de la machine (sur les aveux)">🤖 ${v.machine.correct}/${v.machine.total}</span>
      ${v.is_host ? '<button class="mini ghost" id="endfree">Terminer</button>' : ''}</div>` : r ? `
    <div class="round-head">
      <div class="dots" aria-label="Manche ${v.round_no} sur ${v.total_rounds}">${Array.from({ length: v.total_rounds }, (_, i) => `<i class="${i + 1 < v.round_no ? 'done' : i + 1 === v.round_no ? 'now' : ''}"></i>`).join('')}</div>
      <div class="row small muted"><span class="grow">Manche ${v.round_no} / ${v.total_rounds}</span><span title="Verdicts justes de la machine">🤖 ${v.machine.correct}/${v.machine.total}</span></div>
    </div>` : '';

  app.innerHTML = header + STAGES[v.stage](v, r, mineSubject);
  bindCommon();
  STAGES_BIND[v.stage]?.(v, r, mineSubject);
  $('#endfree') && ($('#endfree').onclick = () => act({ type: 'end_game' }));
  kept.forEach(([id, val]) => { const el = document.getElementById(id); if (el) el.value = val; });

  if (S.useCamera && media.camera && mineSubject && (v.stage === 'asking' || v.stage === 'answering')) {
    const mini = document.createElement('div');
    mini.className = 'camera-mini';
    app.append(mini);
    media.attachPreview(mini);
  }
}

function scoreboard(v, gains = {}) {
  const r = v.round;
  return `<section class="card"><h3>Scores</h3>${[...v.players].sort((a, b) => b.score - a.score).map((p, i) => `
    <div class="player ${p.connected ? '' : 'off'} ${r?.subject === p.id ? 'subject' : ''}">
      <span class="rank">${i + 1}</span><span class="avatar">${esc(p.avatar)}</span>
      <span class="name">${esc(p.name)}${p.mine ? ' <span class="tag">(ici)</span>' : ''}${p.connected ? '' : ' <span class="tag">déconnecté</span>'}</span>
      ${gains[p.id] ? `<span class="gain ${gains[p.id] > 0 ? 'pos' : 'neg'}">${gains[p.id] > 0 ? '+' : ''}${gains[p.id]}</span>` : ''}
      <span class="score">${p.score}</span></div>`).join('')}</section>`;
}

function secretBox(r, title, shared) {
  return `<section class="secret" id="secret" tabindex="0" role="button" aria-label="Maintenir pour voir la consigne secrète">
      <p class="muted">${title}</p>
      <div class="reveal" id="secret-txt">👁️ Maintiens appuyé pour voir</div>
      ${shared ? '<p class="small muted">Les autres, détournez le regard 👀</p>' : ''}
    </section>`;
}

const STAGES = {
  lobby(v) {
    const host = v.is_host;
    const net = S.session.mode === 'host' ? S.session.info : null;
    const inGame = new Set(v.players.map((p) => p.id));
    const addable = S.profiles.filter((p) => !inGame.has(p.id));
    const s = v.settings;
    const uncal = v.players.filter((p) => !p.calibrated);
    const ready = v.players.length && !uncal.length;
    const free = s.mode === 'free';
    return `
      <h2>${esc(s.game_name)}</h2>
      <section class="modes" role="radiogroup" aria-label="Mode">
        <button role="radio" aria-checked="${!free}" class="mode ${free ? '' : 'sel'}" data-gmode="game" ${host ? '' : 'disabled'}>
          <span>🎮</span><b>Jeu</b><small>Manches, consignes secrètes, votes et points</small></button>
        <button role="radio" aria-checked="${free}" class="mode ${free ? 'sel' : ''}" data-gmode="free" ${host ? '' : 'disabled'}>
          <span>🎤</span><b>Libre</b><small>Chacun pose les questions qu'il veut, à qui il veut</small></button>
      </section>
      ${net ? `<section class="card center">
          <p class="muted small">Code à saisir sur les autres appareils</p>
          <div class="big-code" aria-label="Code ${esc(net.code.split('').join(' '))}">${esc(net.code.slice(0, 3))}<span>${esc(net.code.slice(3))}</span></div>
          <p class="ip-list">${net.ips.map((ip) => `${esc(ip)}:${net.port}`).join('<br>') || 'Aucune adresse réseau détectée'}</p>
          <p class="muted small">Sur les autres appareils : « Rejoindre une partie ». 🔒 Connexions chiffrées : ne montre ce code qu'aux joueurs présents.</p>
        </section>` : ''}
      <section class="card"><h3>Joueurs (${v.players.length})</h3>
        <div class="chips">${v.players.map((p) => `
          <div class="chip ${p.calibrated ? '' : 'uncal'}"><span class="avatar">${esc(p.avatar)}</span><span>${esc(p.name)}</span>
            ${p.mine && !p.calibrated ? `<button class="mini primary" data-calib="${esc(p.id)}">Calibrer</button>` : ''}
            ${host || p.mine ? `<button class="mini ghost" data-rm="${esc(p.id)}" aria-label="Retirer ${esc(p.name)}">✕</button>` : ''}
          </div>`).join('')}</div>
        ${addable.length ? `<div class="row"><select id="addp" class="grow" aria-label="Joueur à ajouter">${addable.map((p) => `<option value="${esc(p.id)}">${esc(p.avatar)} ${esc(p.name)}${p.calibrated ? '' : ' (à calibrer)'}</option>`).join('')}</select><button id="addbtn">Ajouter</button></div>` : ''}
        <button class="ghost" data-goto="profiles">＋ Créer un joueur</button>
      </section>
      <section class="card"><h3>Réglages ${host ? '' : '<span class="tag">(choisis par l\'hôte)</span>'}</h3>
        ${free ? '' : `<div class="field"><span>Manches par joueur</span>
          <div class="seg" role="radiogroup">${[1, 2, 3, 4, 5].map((n) => `<button role="radio" aria-checked="${n === s.rounds_per_player}" class="${n === s.rounds_per_player ? 'sel' : ''}" data-rounds="${n}" ${host ? '' : 'disabled'}>${n}</button>`).join('')}</div></div>
        <div class="field"><span>Mentir ou pas ?</span>
          <div class="seg" role="radiogroup">
            <button role="radio" data-mode="assigned" class="${s.instruction_mode === 'assigned' ? 'sel' : ''}" ${host ? '' : 'disabled'}>🎲 Consigne secrète</button>
            <button role="radio" data-mode="free" class="${s.instruction_mode === 'free' ? 'sel' : ''}" ${host ? '' : 'disabled'}>🤔 Libre choix</button></div></div>`}
        <div class="field"><span>${free ? 'Idées de questions' : 'Questions'}</span>
          <div class="seg wrap" role="radiogroup">${Object.entries(PACKS).map(([k, l]) => `<button role="radio" data-pack="${k}" class="${s.pack === k ? 'sel' : ''}" ${host ? '' : 'disabled'}>${l}</button>`).join('')}</div></div>
        <label class="check"><input type="checkbox" id="aloud" ${s.read_aloud ? 'checked' : ''} ${host ? '' : 'disabled'}> La machine lit les questions à voix haute</label>
        <label class="check"><input type="checkbox" id="cam" ${S.useCamera ? 'checked' : ''}> Caméra sur cet appareil</label>
        ${S.useCamera ? lightControl(light.mode) : ''}
      </section>
      <button class="ghost" data-goto="rules" data-return="game">📖 Règles & points</button>
      ${uncal.length ? `<p class="warn small center">Tous les joueurs doivent être calibrés avant de commencer.</p>` : ''}
      <div class="sticky-cta">${host
        ? `<button class="primary big full" id="start" ${ready ? '' : 'disabled'}>${free ? "🎤 Ouvrir la salle d'interrogatoire" : `Lancer la partie · ${v.players.length * s.rounds_per_player} manches`}</button>`
        : '<p class="center muted pulse">En attente du lancement par l\'hôte…</p>'}</div>`;
  },

  free(v) {
    const players = v.players.filter((p) => p.connected);
    const mineP = v.players.filter((p) => p.mine);
    const target = S.freeTarget && players.some((p) => p.id === S.freeTarget) ? S.freeTarget : null;
    const recent = [...v.history].reverse().slice(0, 8);
    return `
      <section class="card">
        <h3>🎯 Qui interroger ?</h3>
        <div class="targets">${players.map((p) => `<button class="target ${p.id === target ? 'sel' : ''}" data-target="${esc(p.id)}" aria-pressed="${p.id === target}">
          <span class="avatar big">${esc(p.avatar)}</span><span>${esc(p.name)}</span></button>`).join('')}</div>
      </section>
      <section class="card">
        <h3>❓ Ta question</h3>
        <textarea id="freeq" data-keep maxlength="200" rows="3" placeholder="Pose la question que tu veux…" aria-label="Question"></textarea>
        <p class="muted small">Besoin d'inspiration ?</p>
        <div class="stack">${v.suggestions.map((q) => `<button class="choice small-choice" data-sugg="${esc(q)}">${esc(q)}</button>`).join('')}</div>
        <button class="ghost" id="moresugg">↻ Autres idées</button>
        ${mineP.length > 1 ? `<label class="field"><span>Posée par</span><select id="asker" data-keep>
          <option value="">🕶️ Anonyme</option>${mineP.map((p) => `<option value="${esc(p.id)}">${esc(p.avatar)} ${esc(p.name)}</option>`).join('')}</select></label>` : ''}
      </section>
      <div class="sticky-cta"><button class="primary big full" id="ask" ${target ? '' : 'disabled'}>🎙️ ${target ? `Interroger ${nameOf(target)}` : 'Choisis qui interroger'}</button></div>
      ${recent.length ? `<section class="card"><h3>Dernières questions</h3>${recent.map((h) => recapRow(h)).join('')}</section>` : ''}`;
  },

  asking(v, r, mine) {
    if (v.settings.mode === 'free') {
      return `
        <section class="card versus">
          <div><span class="avatar big">${esc(playerById(r.subject)?.avatar)}</span><b>${nameOf(r.subject)}</b><small>🔥 interrogé(e)</small></div>
          ${r.asker ? `<div class="vs">←</div><div><span class="avatar big">${esc(playerById(r.asker)?.avatar)}</span><b>${nameOf(r.asker)}</b><small>🎤 demande</small></div>` : ''}
        </section>
        <section class="card question-card"><p class="question">${esc(r.question)}</p></section>
        ${mine ? `<p class="center muted small">Mens ou dis la vérité : c'est toi qui décides. Tu pourras avouer après le verdict… ou pas.</p>
          <div class="sticky-cta"><button class="primary big full" id="answer">🎙️ ${nameOf(r.subject)} répond</button></div>`
          : `<p class="center muted">${nameOf(r.subject)} se prépare à répondre… observez bien !</p>`}
        <button class="ghost" id="cancelq">Annuler la question</button>`;
    }
    const free = v.settings.instruction_mode === 'free';
    const subj = playerById(r.subject);
    const shared = v.players.filter((p) => p.mine).length > 1;
    const chooser = r.asker ? isMine(r.asker) : mine;
    const who = `<section class="card versus">
        <div><span class="avatar big">${esc(subj?.avatar)}</span><b>${nameOf(r.subject)}</b><small>🔥 sur la sellette</small></div>
        ${r.asker ? `<div class="vs">VS</div><div><span class="avatar big">${esc(playerById(r.asker)?.avatar)}</span><b>${nameOf(r.asker)}</b><small>🎤 interrogateur</small></div>` : ''}
      </section>`;

    if (!r.question_locked) {
      return `${who}
        ${chooser ? `
          <section class="card">
            <h3>${r.asker ? `${nameOf(r.asker)}, choisis la question` : 'Choisis ta question'}${r.asker ? ` pour ${nameOf(r.subject)}` : ''}</h3>
            <div class="stack">${r.choices.map((q, i) => `<button class="choice" data-choose="${i}">${esc(q)}</button>`).join('')}</div>
            <button class="ghost" id="newq">↻ 3 autres questions</button>
            <div class="row"><input type="text" id="customq" data-keep maxlength="200" placeholder="…ou écris ta propre question" class="grow" aria-label="Question personnalisée"><button id="sendq">Poser</button></div>
          </section>`
        : `<section class="card center"><p class="pulse">🎤 ${nameOf(r.asker)} choisit une question pour ${nameOf(r.subject)}…</p></section>`}
        ${mine && !chooser && !free ? secretBox(r, `Consigne secrète pour ${esc(subj?.name)} — prépare-toi`, shared) : ''}`;
    }

    return `${who}
      <section class="card question-card"><p class="question">${esc(r.question)}</p></section>
      ${mine ? (free && r.lie == null ? `
          <section class="card"><p class="center"><b>${esc(subj?.name)}</b>, choisis en secret :</p>
            <div class="row"><button class="truth grow big" data-declare="false">😇 Vérité</button><button class="lie grow big" data-declare="true">🤥 Mensonge</button></div>
            ${shared ? '<p class="small muted center">Les autres, détournez le regard 👀</p>' : ''}</section>`
        : `${secretBox(r, free ? 'Ton choix secret' : `Consigne secrète pour ${esc(subj?.name)}`, shared)}
          <div class="sticky-cta"><button class="primary big full" id="answer">🎙️ ${esc(subj?.name)} est prêt(e) à répondre</button></div>`)
      : `<p class="center muted">${nameOf(r.subject)} se prépare à répondre… observez bien !</p>`}`;
  },

  answering(v, r, mine) {
    return `
      <section class="card center"><p class="muted">🎙️ Réponse de</p><h2>${pname(r.subject)}</h2></section>
      <section class="card question-card"><p class="question">${esc(r.question)}</p>
        ${mine ? '<div id="rec"></div>' : `<p class="center"><span class="rec-dot"></span>Polygraphe en direct</p><canvas class="trace" id="otrace"></canvas>
          <div class="meter live"><div id="olvl"></div></div>`}
      </section>
      ${mine ? '' : '<p class="center muted small">Observez son visage, sa voix… vous voterez juste après.</p>'}`;
  },

  voting(v, r) {
    const voters = v.players.filter((p) => p.connected && p.id !== r.subject);
    const myVoters = voters.filter((p) => p.mine);
    const done = new Set(r.voted);
    return `
      <section class="card center"><p class="muted">🗳️ À vous de juger</p><h2>${pname(r.subject)}</h2><p class="muted">« ${esc(r.question)} »</p></section>
      <section class="card center machine-sealed"><p>🤖 La machine a rendu son verdict… <b>sous scellés</b> 🔒</p></section>
      ${myVoters.length ? `<section class="card"><h3>Mensonge ou vérité ?</h3>
        ${myVoters.map((p) => done.has(p.id)
          ? `<div class="player"><span class="avatar">${esc(p.avatar)}</span><span class="name">${esc(p.name)}</span><span class="tag">✓ a voté</span></div>`
          : `<div class="vote-box"><div class="player"><span class="avatar">${esc(p.avatar)}</span><span class="name">${esc(p.name)}</span>
                <label class="bet"><input type="checkbox" data-sure="${esc(p.id)}"> 🔥 Sûr de moi <small>(+2 / −1)</small></label></div>
              <div class="row"><button class="truth grow big" data-vote="${esc(p.id)}" data-lie="false">😇 Vérité</button><button class="lie grow big" data-vote="${esc(p.id)}" data-lie="true">🤥 Mensonge</button></div></div>`).join('')}
        </section>` : '<p class="center muted pulse">Les autres délibèrent…</p>'}
      <section class="card"><div class="voters">${voters.map((p) => `<span class="${done.has(p.id) ? 'ok' : ''}" title="${esc(p.name)}">${esc(p.avatar)}</span>`).join('')}</div>
        <p class="center muted small">Votes : ${done.size} / ${voters.length}</p></section>
      ${v.is_host ? '<button class="ghost" id="force">Révéler sans attendre</button>' : ''}`;
  },

  reveal(v, r, mine) {
    if (v.settings.mode === 'free') return freeReveal(v, r, mine);
    const vd = r.verdict;
    const o = r.outcome ?? { points: {}, machine_correct: true };
    const subj = playerById(r.subject);
    const last = v.round_no >= v.total_rounds;
    const animate = !S.revealed[r.number] && !reducedMotion;
    return `
      <section class="card">
        <p class="center muted">« ${esc(r.question)} » — ${pname(r.subject)}</p>
        <p class="center muted small">Indice de mensonge de la machine</p>
        <div id="gauge">${gauge(vd.index, { scanning: animate })}</div>
        <p class="verdict-label ${animate ? 'later' : ''}" id="vlabel" style="color:${gaugeColor(vd.index)}">🤖 « ${esc(vd.label)} »</p>
      </section>
      <div class="truth-banner ${r.lie ? 'lie' : 'truth'} ${animate ? 'later' : ''}" id="tbanner">${r.lie ? '🤥 ' + esc(subj?.name) + ' MENTAIT !' : '😇 ' + esc(subj?.name) + ' disait la VÉRITÉ'}
        <small>${o.machine_correct ? '🎯 La machine avait vu juste' : `🤖💥 La machine s'est fait berner ! +2 pour ${esc(subj?.name)}`}</small></div>
      <div class="${animate ? 'later' : ''}" id="details">
        ${vd.speaker && !vd.speaker.matched ? `<p class="center warn card">🕵️ Bizarre… cette voix ressemblait plutôt à celle de ${esc(vd.speaker.best_name)} !</p>` : ''}
        <section class="card"><h3>Ce qui a trahi (ou pas) ${esc(subj?.name)}</h3>
          ${vd.factors.length ? vd.factors.map((f) => `<div class="factor"><span>${esc(f.label)}</span><span class="${f.contribution > 0 ? 'up' : 'down'}">${esc(f.description)}</span></div>`).join('') : '<p class="muted small">Aucun écart marquant par rapport à son calibrage.</p>'}
          ${vd.heart_rate ? `<p class="small">♥ Pouls estimé : ${Math.round(vd.heart_rate)} bpm</p>` : ''}
          <p class="muted small">Confiance de la machine : ${Math.round(vd.confidence * 100)} %</p>
        </section>
        ${r.votes && Object.keys(r.votes).length ? `<section class="card"><h3>Votes</h3>${Object.entries(r.votes).map(([id, vt]) => `
          <div class="player"><span class="name">${pname(id)}</span><span>${vt.lie ? '🤥' : '😇'}${vt.sure ? ' 🔥' : ''}</span><span class="${vt.lie === r.lie ? 'pos' : 'neg'}">${vt.lie === r.lie ? '✅' : '❌'}</span></div>`).join('')}</section>` : ''}
        ${scoreboard(v, o.points)}
        ${v.is_host ? `<div class="sticky-cta"><button class="primary big full" id="next">${last ? '🏆 Résultats' : 'Manche suivante →'}</button></div>` : '<p class="center muted">L\'hôte lance la manche suivante…</p>'}
      </div>`;
  },

  finished(v) {
    if (v.settings.mode === 'free') {
      const confessed = v.history.filter((h) => h.lie != null);
      return `
        <section class="card hero"><div class="logo">🎤</div><h2>Fin de l'interrogatoire</h2>
          <p class="muted">${v.history.length} question(s) posée(s) · ${confessed.length} aveu(x)</p>
          ${v.machine.total ? `<p>🤖 Sur les aveux, la machine a vu juste <b>${v.machine.correct}</b> fois sur <b>${v.machine.total}</b>.</p>` : ''}</section>
        ${v.history.length ? `<section class="card"><h3>Toutes les questions</h3>${v.history.map((h) => recapRow(h)).join('')}</section>` : ''}
        ${v.is_host ? '<button class="primary big" id="again">Retour au salon</button>' : ''}
        <button class="ghost" id="quit">Quitter</button>`;
    }
    const ranking = [...v.players].sort((a, b) => b.score - a.score);
    const acc = v.machine.total ? Math.round((v.machine.correct / v.machine.total) * 100) : 0;
    const podium = [ranking[1], ranking[0], ranking[2]];
    const aw = awards(v);
    return `
      <section class="card hero"><div class="logo">🏆</div><h2>${ranking[0] ? esc(ranking[0].name) + ' gagne !' : 'Partie terminée'}</h2>
        <div class="podium">${podium.map((p, i) => p ? `<div class="p${[2, 1, 3][i]}"><span class="avatar big">${esc(p.avatar)}</span><b>${esc(p.name)}</b><span class="score">${p.score}</span><div class="step">${[2, 1, 3][i]}</div></div>` : '<div></div>').join('')}</div>
      </section>
      ${aw.length ? `<section class="card"><h3>🎖️ Trophées</h3><div class="awards">${aw.map((a) => `<div><span>${a.icon}</span><b>${esc(a.title)}</b><small>${pname(a.player)} · ${esc(a.why)}</small></div>`).join('')}</div></section>` : ''}
      <section class="card center"><p>🤖 La machine a vu juste <b>${v.machine.correct}</b> fois sur <b>${v.machine.total}</b> (${acc} %).</p>
        <p class="muted small">${acc >= 60 ? 'Elle commence à bien vous connaître…' : 'Vous êtes de sacrés comédiens !'} Plus vous jouez, plus elle apprend vos tics.</p></section>
      ${v.history.length ? `<section class="card"><h3>Récapitulatif</h3>${v.history.map((h) => recapRow(h)).join('')}</section>` : ''}
      ${v.is_host ? '<button class="primary big" id="again">Rejouer</button>' : ''}
      <button class="ghost" id="quit">Quitter</button>`;
  },
};

/** Ligne de récapitulatif : sujet, question, indice, aveu / vérité. */
function recapRow(h) {
  const truth = h.lie == null ? '' : h.lie ? ' <span class="t-lie">a menti</span>' : ' <span class="t-truth">a dit vrai</span>';
  const mark = h.machine_correct == null ? '🤐' : h.machine_correct ? '🎯' : '💥';
  return `<div class="recap"><span class="num">${h.number}</span><div class="grow"><b>${pname(h.subject)}</b>${truth}${h.asker ? ` <small class="muted">· question de ${nameOf(h.asker)}</small>` : ''}<br><small class="muted">« ${esc(h.question)} »</small></div>
    <span title="Indice de la machine" style="color:${gaugeColor(h.index)}">${h.index}</span><span title="${h.machine_correct == null ? 'Pas d\'aveu' : 'Verdict de la machine'}">${mark}</span></div>`;
}

function freeReveal(v, r, mine) {
  const vd = r.verdict;
  const subj = playerById(r.subject);
  const animate = !S.revealed[r.number] && !reducedMotion;
  const confessed = r.lie != null;
  return `
    <section class="card">
      <p class="center muted">« ${esc(r.question)} » — ${pname(r.subject)}</p>
      <p class="center muted small">Indice de mensonge de la machine</p>
      <div id="gauge">${gauge(vd.index, { scanning: animate })}</div>
      <p class="verdict-label ${animate ? 'later' : ''}" id="vlabel" style="color:${gaugeColor(vd.index)}">🤖 « ${esc(vd.label)} »</p>
    </section>
    <div class="${animate ? 'later' : ''}" id="details">
      ${confessed ? `<div class="truth-banner ${r.lie ? 'lie' : 'truth'}">${r.lie ? '🤥 ' + esc(subj?.name) + ' avoue avoir menti !' : '😇 ' + esc(subj?.name) + ' jure avoir dit vrai'}
          <small>${r.outcome?.machine_correct ? '🎯 La machine avait vu juste' : "🤖💥 La machine s'est fait berner !"}</small></div>`
        : mine ? `<section class="card"><h3>Alors, ${esc(subj?.name)}… tu avoues ?</h3>
            <p class="muted small">Avouer aide la machine à mieux te connaître. Tu peux aussi garder le secret.</p>
            <div class="row"><button class="truth grow" data-confess="false">😇 J'ai dit vrai</button><button class="lie grow" data-confess="true">🤥 J'ai menti</button></div></section>`
        : `<p class="center muted pulse">🤐 ${nameOf(r.subject)} va-t-il/elle avouer ?</p>`}
      ${vd.speaker && !vd.speaker.matched ? `<p class="center warn card">🕵️ Bizarre… cette voix ressemblait plutôt à celle de ${esc(vd.speaker.best_name)} !</p>` : ''}
      <section class="card"><h3>Ce que la machine a remarqué</h3>
        ${vd.factors.length ? vd.factors.map((f) => `<div class="factor"><span>${esc(f.label)}</span><span class="${f.contribution > 0 ? 'up' : 'down'}">${esc(f.description)}</span></div>`).join('') : '<p class="muted small">Aucun écart marquant par rapport à son calibrage.</p>'}
        ${vd.heart_rate ? `<p class="small">♥ Pouls estimé : ${Math.round(vd.heart_rate)} bpm</p>` : ''}
        <p class="muted small">Confiance de la machine : ${Math.round(vd.confidence * 100)} %</p>
      </section>
      <div class="sticky-cta"><button class="primary big full" id="nextq">❓ Nouvelle question</button></div>
    </div>`;
}

/** Trophées de fin de partie, calculés à partir de l'historique public. */
function awards(v) {
  const t = {};
  const add = (k, id, n = 1) => { (t[k] ??= {})[id] = (t[k][id] ?? 0) + n; };
  const avg = {};
  const push = (k, id, x) => { ((avg[k] ??= {})[id] ??= []).push(x); };
  for (const h of v.history.filter((x) => x.lie != null)) {
    add('liar', h.subject, h.points[h.subject] ?? 0);
    push(h.lie ? 'cold' : 'nervous', h.subject, h.index);
    for (const [voter, vt] of Object.entries(h.votes)) {
      if (vt.lie === h.lie) add('detective', voter);
      if (vt.sure) add('gambler', voter);
    }
    if (h.asker && h.lie && h.machine_correct) add('trapper', h.asker);
  }
  const best = (m, dir = 1) => {
    const e = Object.entries(m ?? {}).filter(([, x]) => x > 0 || dir < 0);
    if (!e.length) return null;
    return e.sort((a, b) => dir * (b[1] - a[1]))[0];
  };
  const mean = (k) => Object.fromEntries(Object.entries(avg[k] ?? {}).map(([id, xs]) => [id, xs.reduce((a, b) => a + b, 0) / xs.length]));
  const out = [];
  const liar = best(t.liar); if (liar) out.push({ icon: '🎭', title: 'Maître menteur', player: liar[0], why: `${liar[1]} pts en étant sur la sellette` });
  const det = best(t.detective); if (det) out.push({ icon: '🔍', title: 'Détecteur humain', player: det[0], why: `${det[1]} vote(s) juste(s)` });
  const cold = best(mean('cold'), -1); if (cold) out.push({ icon: '🧊', title: 'Sang-froid', player: cold[0], why: `indice moyen de ${Math.round(cold[1])} en mentant` });
  const nerv = best(mean('nervous')); if (nerv && nerv[1] >= 40) out.push({ icon: '😰', title: 'Trop transparent', player: nerv[0], why: `l'air coupable même en disant vrai (${Math.round(nerv[1])})` });
  const gam = best(t.gambler); if (gam) out.push({ icon: '🎲', title: 'Flambeur', player: gam[0], why: `${gam[1]} pari(s) « sûr de moi »` });
  const trap = best(t.trapper); if (trap) out.push({ icon: '🪤', title: 'Question piège', player: trap[0], why: `${trap[1]} menteur(s) démasqué(s)` });
  return out;
}

function bindSecret(r) {
  const sec = $('#secret');
  if (!sec) return;
  const txt = $('#secret-txt');
  const show = (e) => { e.preventDefault(); txt.className = 'reveal ' + (r.lie ? 'lie' : 'truth'); txt.textContent = r.lie ? '🤥 MENS !' : '😇 DIS LA VÉRITÉ'; fx.buzz(15); };
  const hide = () => { txt.className = 'reveal'; txt.textContent = '👁️ Maintiens appuyé pour voir'; };
  sec.addEventListener('pointerdown', show);
  ['pointerup', 'pointerleave', 'pointercancel', 'blur'].forEach((ev) => sec.addEventListener(ev, hide));
  sec.addEventListener('keydown', (e) => { if (e.key === ' ' || e.key === 'Enter') show(e); });
  sec.addEventListener('keyup', hide);
  sec.addEventListener('contextmenu', (e) => e.preventDefault());
}

const STAGES_BIND = {
  free(v) {
    $$('[data-target]').forEach((b) => (b.onclick = () => { S.freeTarget = b.dataset.target; renderGame(true); }));
    $$('[data-sugg]').forEach((b) => (b.onclick = () => { $('#freeq').value = b.dataset.sugg; $('#freeq').focus(); }));
    $('#moresugg').onclick = () => act({ type: 'new_question' });
    $('#ask').onclick = () => {
      const question = $('#freeq').value.trim();
      if (!question) { toast('Écris (ou choisis) une question.', true); return $('#freeq').focus(); }
      const asker = $('#asker') ? ($('#asker').value || null) : (v.players.find((p) => p.mine && p.id !== S.freeTarget)?.id ?? null);
      $('#freeq').value = '';
      act({ type: 'ask', subject: S.freeTarget, question, asker });
    };
  },

  lobby(v) {
    $$('[data-rm]').forEach((b) => (b.onclick = () => act({ type: 'remove_player', player_id: b.dataset.rm })));
    $$('[data-calib]').forEach((b) => (b.onclick = () => go('calibrate', { id: b.dataset.calib, returnTo: 'game' })));
    $('#addbtn') && ($('#addbtn').onclick = async () => {
      const id = $('#addp').value;
      act({ type: 'add_players', players: await call('player_infos', { ids: [id] }) });
    });
    const upd = (patch) => act({ type: 'update_settings', settings: { ...v.settings, ...patch } });
    if (v.is_host) {
      $$('[data-gmode]').forEach((b) => (b.onclick = () => upd({ mode: b.dataset.gmode })));
      $$('[data-rounds]').forEach((b) => (b.onclick = () => upd({ rounds_per_player: Number(b.dataset.rounds) })));
      $$('[data-mode]').forEach((b) => (b.onclick = () => upd({ instruction_mode: b.dataset.mode })));
      $$('[data-pack]').forEach((b) => (b.onclick = () => upd({ pack: b.dataset.pack })));
      $('#aloud').onchange = () => upd({ read_aloud: $('#aloud').checked });
    }
    $('#cam').onchange = () => { S.useCamera = $('#cam').checked; setPref('camera', String(S.useCamera)); media.stop(); renderGame(true); };
    $('#start') && ($('#start').onclick = async () => {
      try { await media.start({ camera: S.useCamera }); await media.resume(); } catch (e) { return toast(e.message, true); }
      fx.play('start');
      act({ type: 'start' });
    });
  },

  asking(v, r) {
    $('#cancelq') && ($('#cancelq').onclick = () => act({ type: 'next_round' }));
    $$('[data-choose]').forEach((b) => (b.onclick = () => act({ type: 'choose_question', index: Number(b.dataset.choose) })));
    $('#newq') && ($('#newq').onclick = () => act({ type: 'new_question' }));
    if ($('#sendq')) {
      const send = () => { const t = $('#customq').value.trim(); if (t) { act({ type: 'custom_question', text: t }); $('#customq').value = ''; } };
      $('#sendq').onclick = send;
      $('#customq').onkeydown = (e) => { if (e.key === 'Enter') send(); };
    }
    $$('[data-declare]').forEach((b) => (b.onclick = () => act({ type: 'declare', lie: b.dataset.declare === 'true' })));
    bindSecret(r);
    $('#answer') && ($('#answer').onclick = async () => {
      try { await media.start({ camera: S.useCamera }); await media.resume(); } catch (e) { return toast(e.message, true); }
      act({ type: 'start_answer' });
    });
  },

  answering(v, r, mine) {
    if (!mine) {
      const c = $('#otrace');
      // Niveau reçu en direct du sujet ; à défaut, simple agitation d'attente.
      const level = () => {
        const fresh = S.live.player === r.subject && performance.now() - S.live.at < 600;
        const l = fresh ? S.live.level : 0.15 + Math.random() * 0.1;
        const m = $('#olvl');
        if (m) m.style.width = Math.round(Math.min(1, l) * 100) + '%';
        return l;
      };
      if (c) new Trace(c, level);
      return;
    }
    S.flowKey = `answering:${r.number}`;
    runAnswer(v, r);
  },

  voting() {
    $$('[data-vote]').forEach((b) => (b.onclick = () => {
      const voter = b.dataset.vote;
      const sure = !!$(`[data-sure="${CSS.escape(voter)}"]`)?.checked;
      fx.play('vote');
      fx.buzz(sure ? [20, 40, 20] : 20);
      act({ type: 'vote', voter, lie: b.dataset.lie === 'true', sure });
    }));
    $('#force') && ($('#force').onclick = () => act({ type: 'reveal' }));
  },

  reveal(v, r, mine) {
    $('#next') && ($('#next').onclick = () => act({ type: 'next_round' }));
    $('#nextq') && ($('#nextq').onclick = () => act({ type: 'next_round' }));
    $$('[data-confess]').forEach((b) => (b.onclick = () => act({ type: 'confess', lie: b.dataset.confess === 'true' })));
    // La machine apprend : on connaît maintenant la vérité de cette réponse.
    const a = S.answers[r.number];
    if (mine && a && !a.learned && r.lie != null) {
      a.learned = true;
      call('learn_answer', { profileId: a.profileId, audio: a.audio, video: a.video, lie: r.lie, machineCorrect: !!r.outcome?.machine_correct })
        .then(loadProfiles).catch(() => {});
    }
    if (!S.revealed[r.number]) {
      S.revealed[r.number] = true;
      if (reducedMotion) playOutcome(r);
      else stageReveal(r);
    }
  },

  finished(v) {
    if (!S.revealed.finished) { S.revealed.finished = true; fx.play('win'); fx.buzz([40, 60, 40, 60, 120]); }
    $('#again') && ($('#again').onclick = () => act({ type: 'back_to_lobby' }));
    $('#quit').onclick = async () => { await leaveSession(); go('home'); };
  },
};

function playOutcome(r) {
  const o = r.outcome;
  if (!o) return;
  if (!o.machine_correct) { fx.play('fooled'); fx.buzz([60, 40, 60]); } else if (r.lie) { fx.play('lie'); fx.buzz([200]); } else { fx.play('truth'); fx.buzz(40); }
}

/** Révélation en trois temps : l'aiguille hésite, se fige, puis la vérité éclate. */
async function stageReveal(r) {
  const vd = r.verdict;
  const freeMode = S.view?.settings.mode === 'free';
  fx.play('suspense');
  await wait(1700);
  const g = $('#gauge');
  if (!g) return;
  g.innerHTML = gauge(vd.index);
  $('#vlabel')?.classList.add('show');
  fx.play(vd.index >= 50 ? 'lie' : 'truth');
  await wait(1500);
  if (freeMode) { $('#details')?.classList.add('show'); return; }
  $('#tbanner')?.classList.add('show');
  playOutcome(r);
  await wait(500);
  $('#details')?.classList.add('show');
}

async function runAnswer(v, r) {
  const box = $('#rec');
  const live = S.session.mode !== 'local';
  const res = await captureAnswer(box, { prompt: r.question, readAloud: v.settings.read_aloud, live }).catch(() => null);
  if (S.view?.stage !== 'answering' || S.view.round?.number !== r.number) return;
  const retry = (msg) => {
    box.innerHTML = `${msg ? `<p class="center warn">${msg}</p>` : ''}
      <div class="row"><button class="ghost grow" id="cancel">Retour</button><button class="primary grow" id="retry">Réessayer</button></div>`;
    $('#retry', box).onclick = () => runAnswer(v, r);
    $('#cancel', box).onclick = () => { S.flowKey = null; act({ type: 'cancel_answer' }); };
  };
  if (!res) {
    S.flowKey = null;
    return act({ type: 'cancel_answer' });
  }
  if (!res.audio.speech_detected) return retry('Je n\'ai entendu aucune réponse 🤔');
  const candidates = S.view.players.filter((p) => p.mine).map((p) => p.id);
  try {
    const verdict = await call('score_answer', { profileId: r.subject, audio: res.audio, video: res.video, candidates });
    S.answers[r.number] = { profileId: r.subject, audio: res.audio, video: res.video, learned: false };
    S.flowKey = null;
    await act({ type: 'submit_verdict', verdict });
  } catch {
    retry('');
  }
}

// ---------- Démarrage ----------

async function boot() {
  if (!TAURI) {
    app.innerHTML = `<section class="card"><h3>Application native requise</h3><p class="muted">Cette interface tourne dans l'application YouLiar (Tauri). Lance <code>npm run dev</code>.</p></section>`;
    return;
  }
  const { listen } = TAURI.event;
  await listen('game-state', (e) => {
    const prev = S.view;
    S.view = e.payload;
    // Nouvelle partie / retour au salon : on oublie les réponses mémorisées.
    if (prev && prev.stage !== 'lobby' && S.view.stage === 'lobby') { S.answers = {}; S.revealed = {}; }
    // Annonces sonores des changements d'étape.
    if (prev && prev.stage !== S.view.stage) {
      if (S.view.stage === 'asking' && isMine(S.view.round?.subject)) { fx.play('start'); fx.buzz([30, 50, 30]); }
      if (S.view.stage === 'voting') fx.play('rec');
    }
    if (prev?.round && S.view.round && prev.round.voted.length < S.view.round.voted.length) fx.play('vote');
    // Mode libre : l'aveu déclenche le son du dénouement.
    if (prev?.round && prev.round.lie == null && S.view.round?.lie != null && S.view.stage === 'reveal' && S.view.settings.mode === 'free') playOutcome(S.view.round);
    if (S.screen === 'game') renderGame();
  });
  await listen('live-level', (e) => { S.live = { player: e.payload.player, level: e.payload.level, at: performance.now() }; });
  await listen('game-error', (e) => toast(e.payload, true));
  await listen('session-ended', (e) => {
    toast(e.payload, true);
    media.stop();
    S.session = { mode: 'none' }; S.view = null;
    updateBadge();
    if (S.screen === 'game') go('home');
  });
  await listen('link-status', (e) => {
    const lost = e.payload === 'reconnecting';
    badge.textContent = lost ? '⚠️ Reconnexion…' : '🔗 Invité';
    badge.classList.toggle('warn', lost);
    toast(lost ? 'Connexion perdue, reconnexion en cours…' : 'Reconnecté à la partie ✓', lost);
  });
  await listen('host-info', (e) => {
    if (S.session.mode === 'host' && e.payload) {
      S.session.info = e.payload;
      // Le code n'est affiché que dans le salon ; ne pas interrompre un enregistrement.
      if (S.screen === 'game' && S.view?.stage === 'lobby') renderGame(true);
    }
  });
  await listen('discovery', (e) => { S.discovered = e.payload; if (S.screen === 'join') renderFound(); });
  await loadProfiles().catch(() => {});
  // Reprise après rechargement de la page : la session vit côté Rust.
  const info = await call('session_info').catch(() => ({ mode: 'none' }));
  S.session = info;
  updateBadge();
  if (info.mode !== 'none') {
    S.view = await call('game_state').catch(() => null);
    go('game');
  } else {
    go('home');
  }
}

boot();
