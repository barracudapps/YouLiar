// Mode navigateur : l'interface est servie par l'hôte de la partie (HTTPS, réseau local).
// Ce module remplace les commandes de l'application par :
// - le moteur Rust compilé en WebAssembly (analyse, profils, calibrage, verdicts),
//   exécuté sur cet appareil : la voix et l'image ne le quittent pas ;
// - une connexion WebSocket vers l'hôte, avec le même protocole chiffré que l'application ;
// - le stockage local du navigateur pour les profils de cet appareil.

import init, * as yl from './web/youliar_web.js';

const PROFILES_KEY = 'yl.web.profiles';
// Partie en cours de cet onglet (code, joueurs, jeton de reprise) : un rechargement de la
// page, fréquent sur téléphone, reprend la place au lieu d'éjecter le joueur.
// sessionStorage : propre à l'onglet, effacé à sa fermeture.
const SESSION_KEY = 'yl.web.session';
const MAX_FRAME = 256 * 1024;
const REFUSALS = {
  0: 'Code de partie incorrect.',
  1: "Version de YouLiar incompatible avec l'hôte.",
  2: "L'hôte refuse la connexion (partie pleine ou trop de tentatives).",
};

const listeners = {};
function emit(name, payload) { (listeners[name] ?? []).forEach((cb) => cb({ payload })); }

// ---------- Profils (stockage local du navigateur) ----------

let profiles = [];
function loadProfiles() {
  try { profiles = JSON.parse(localStorage.getItem(PROFILES_KEY) ?? '[]'); } catch { profiles = []; }
}
function saveProfiles() {
  try { localStorage.setItem(PROFILES_KEY, JSON.stringify(profiles)); } catch {
    throw new Error('Stockage du navigateur plein ou désactivé : impossible d\'enregistrer le profil.');
  }
}
const summary = (p) => JSON.parse(yl.profile_summary(JSON.stringify(p)));
function profile(id) {
  const p = profiles.find((x) => x.id === id);
  if (!p) throw new Error('Profil introuvable.');
  return p;
}
function replace(p) {
  profiles = profiles.map((x) => (x.id === p.id ? p : x));
  saveProfiles();
  return summary(p);
}
const sessionMeasures = {};

// ---------- Connexion à l'hôte ----------

let conn = null; // { ws, crypto, online, view, resume, code, players, ping, closing }

function frame(bytes) {
  const out = new Uint8Array(4 + bytes.length);
  new DataView(out.buffer).setUint32(0, bytes.length);
  out.set(bytes, 4);
  return out;
}

/** Ouvre le WebSocket et mène la poignée de main ; résout une fois accueilli. */
function connect(code, players, resume) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`wss://${location.host}/ws`);
    ws.binaryType = 'arraybuffer';
    const crypto = new yl.WebSession(code);
    let buf = new Uint8Array(0);
    let stage = 'handshake';
    const fail = (msg, fatal) => { stage = 'dead'; try { ws.close(); } catch { /* déjà fermé */ } reject({ message: msg, fatal }); };
    const timer = setTimeout(() => fail("L'hôte n'a pas répondu.", false), 10000);
    const c = { ws, crypto, online: true };

    ws.onopen = () => ws.send(frame(crypto.hello()));
    ws.onerror = () => {};
    ws.onclose = () => {
      clearTimeout(timer);
      if (stage === 'run') lost(c);
      else if (stage !== 'dead') fail("Connexion impossible avec l'hôte.", false);
    };
    ws.onmessage = (ev) => {
      const chunk = new Uint8Array(ev.data);
      const merged = new Uint8Array(buf.length + chunk.length);
      merged.set(buf); merged.set(chunk, buf.length);
      buf = merged;
      while (buf.length >= 4) {
        const n = new DataView(buf.buffer, buf.byteOffset).getUint32(0);
        if (n === 0 || n > MAX_FRAME) return fail('Réponse invalide de l\'hôte.', false);
        if (buf.length < 4 + n) break;
        const f = buf.slice(4, 4 + n);
        buf = buf.slice(4 + n);
        if (f.length === 1) return fail(REFUSALS[f[0]] ?? REFUSALS[2], true);
        try {
          if (stage === 'handshake') {
            crypto.finish(f);
            ws.send(frame(crypto.seal(JSON.stringify({ type: 'hello', players, resume }))));
            stage = 'welcome';
          } else {
            const msg = JSON.parse(crypto.open(f));
            if (stage === 'welcome') {
              if (msg.type === 'kicked') return fail(msg.reason, true);
              if (msg.type !== 'welcome') return fail("Réponse inattendue de l'hôte.", false);
              clearTimeout(timer);
              stage = 'run';
              c.resume = msg.resume;
              resolve(c);
            } else {
              // Une erreur d'affichage ne doit pas couper la connexion.
              try { dispatch(c, msg); } catch (e) { console.error(e); }
            }
          }
        } catch (e) {
          return fail(String(e?.message ?? e), true);
        }
      }
    };
  });
}

function send(c, msg) {
  if (!c?.online || c.ws.readyState !== WebSocket.OPEN) throw new Error(c ? 'Reconnexion à l\'hôte en cours…' : 'Aucune partie en cours.');
  c.ws.send(frame(c.crypto.seal(JSON.stringify(msg))));
}

// L'état peut arriver dans le même paquet que l'accueil, avant que `conn` soit en place :
// il est rattaché à la liaison elle-même.
function dispatch(c, msg) {
  switch (msg.type) {
    case 'state': c.view = msg.view; emit('game-state', msg.view); break;
    case 'error': emit('game-error', msg.message); break;
    case 'live': emit('live-level', { player: msg.player, level: msg.level }); break;
    case 'kicked':
      c.closing = true;
      if (conn) conn.closing = true;
      c.ws.close();
      end(msg.reason);
      break;
    default: break;
  }
}

function remember() {
  try {
    if (conn) sessionStorage.setItem(SESSION_KEY, JSON.stringify({ code: conn.code, players: conn.players, resume: conn.resume }));
    else sessionStorage.removeItem(SESSION_KEY);
  } catch { /* stockage indisponible : pas de reprise après rechargement */ }
}

function end(reason) {
  if (!conn) return;
  clearInterval(conn.ping);
  conn = null;
  remember();
  if (reason) emit('session-ended', reason);
}

/** Coupure : on tente de reprendre sa place avec le jeton secret, comme l'application. */
async function lost(c) {
  if (!conn || conn.closing || conn.link !== c) return;
  c.online = false;
  emit('link-status', 'reconnecting');
  for (const delay of [1, 2, 3, 5, 5, 5, 5, 5]) {
    await new Promise((r) => setTimeout(r, delay * 1000));
    if (!conn || conn.closing) return;
    try {
      const next = await connect(conn.code, conn.players, conn.resume);
      conn.link = next;
      conn.resume = next.resume;
      remember();
      emit('link-status', 'connected');
      return;
    } catch (e) {
      if (e.fatal) return end(e.message);
    }
  }
  end("Connexion perdue avec l'hôte.");
}

function attach(link, code, players) {
  conn = { link, code, players, resume: link.resume, closing: false };
  conn.ping = setInterval(() => { try { send(conn.link, { type: 'ping' }); } catch { /* reconnexion en cours */ } }, 5000);
  remember();
}

/** Après un rechargement : reprend la partie de cet onglet, si l'hôte la garde encore. */
async function resumeSaved() {
  let saved;
  try { saved = JSON.parse(sessionStorage.getItem(SESSION_KEY) ?? 'null'); } catch { saved = null; }
  if (!saved?.code || !saved.resume || !Array.isArray(saved.players)) return;
  try {
    const link = await connect(saved.code, saved.players, saved.resume);
    attach(link, saved.code, saved.players);
    // L'état suit l'accueil de près : on l'attend un court instant pour afficher la partie.
    for (let i = 0; i < 20 && !link.view; i++) await new Promise((r) => setTimeout(r, 100));
  } catch {
    try { sessionStorage.removeItem(SESSION_KEY); } catch { /* ignoré */ }
  }
}

async function join({ code, profileIds }) {
  if (!profileIds?.length) throw new Error('Choisis au moins un joueur sur cet appareil.');
  code = String(code ?? '').replace(/\D/g, '');
  if (code.length !== 6) throw new Error('Le code de partie comporte 6 chiffres.');
  const players = profileIds.map((id) => profile(id)).map((p) => ({ id: p.id, name: p.name, avatar: p.avatar, calibrated: p.calibrated }));
  stop();
  let link;
  try { link = await connect(code, players, null); } catch (e) { throw new Error(e.message); }
  attach(link, code, players);
  Object.keys(sessionMeasures).forEach((k) => delete sessionMeasures[k]);
  return { mode: 'client', host: location.host };
}

function stop() {
  if (!conn) return;
  conn.closing = true;
  try { conn.link.ws.close(); } catch { /* déjà fermé */ }
  end(null);
}

// ---------- Commandes ----------

const unavailable = () => { throw new Error('Indisponible dans le navigateur : installe l\'application pour héberger une partie.'); };

const COMMANDS = {
  profiles_list: () => profiles.map(summary),
  profile_create: ({ name, avatar }) => {
    const p = JSON.parse(yl.profile_new(name, avatar));
    profiles.push(p);
    saveProfiles();
    return summary(p);
  },
  profile_update: ({ id, name, avatar }) => {
    const p = profile(id);
    const n = String(name ?? '').trim().slice(0, 24);
    if (!n || !avatar) throw new Error('Choisis un prénom et un avatar.');
    return replace({ ...p, name: n, avatar: String(avatar).slice(0, 8) });
  },
  profile_delete: ({ id }) => { profiles = profiles.filter((p) => p.id !== id); saveProfiles(); },
  player_infos: ({ ids }) => ids.map((id) => profiles.find((p) => p.id === id)).filter(Boolean)
    .map((p) => ({ id: p.id, name: p.name, avatar: p.avatar, calibrated: p.calibrated })),
  calibration_protocol: () => JSON.parse(yl.calibration_protocol()),
  analyze_audio: (bytes, opts) => {
    const h = opts?.headers ?? {};
    const samples = new Float32Array(bytes.buffer, bytes.byteOffset, Math.floor(bytes.byteLength / 4));
    return JSON.parse(yl.analyze_audio(samples, Number(h['x-sample-rate']), Number(h['x-prompt-offset'] ?? 0)));
  },
  analyze_video: ({ samples }) => JSON.parse(yl.analyze_video(JSON.stringify(samples))),
  calibration_finish: ({ profileId, takes, resting }) =>
    replace(JSON.parse(yl.profile_calibrate(JSON.stringify(profile(profileId)), JSON.stringify(takes), JSON.stringify(resting ?? null)))),
  score_answer: ({ profileId, audio, video, candidates }) => {
    const others = profiles.filter((p) => p.id === profileId || candidates?.includes(p.id));
    const session = sessionMeasures[profileId] ?? [];
    const verdict = JSON.parse(yl.profile_score(JSON.stringify(profile(profileId)), JSON.stringify(audio),
      JSON.stringify(video ?? null), JSON.stringify(others), JSON.stringify(session)));
    // Réponses de la partie en cours : corrigent la dérive du jour aux réponses suivantes.
    (sessionMeasures[profileId] ??= []).push(JSON.parse(yl.answer_measures(JSON.stringify(audio), JSON.stringify(video ?? null))));
    return verdict;
  },
  learn_answer: ({ profileId, audio, video, lie, machineCorrect }) =>
    replace(JSON.parse(yl.profile_learn(JSON.stringify(profile(profileId)), JSON.stringify(audio), JSON.stringify(video ?? null), !!lie, !!machineCorrect))),
  themes_list: () => JSON.parse(yl.themes()),
  app_version: () => yl.version(),
  // Le navigateur reçoit toujours l'interface de l'hôte : rien à mettre à jour ici.
  update_check: () => null,
  update_install: unavailable,
  wipe_data: () => { stop(); profiles = []; try { localStorage.removeItem(PROFILES_KEY); } catch { /* ignoré */ } },
  session_info: () => (conn ? { mode: 'client', host: location.host } : { mode: 'none' }),
  game_state: () => conn?.link?.view ?? null,
  session_join: join,
  session_stop: stop,
  game_action: ({ action }) => send(conn?.link, { type: 'action', action }),
  live_level: ({ level }) => { try { send(conn?.link, { type: 'live', level }); } catch { /* jetable */ } },
  edge_light: () => false, // pas de fenêtres d'écran : l'interface dessine elle-même le halo
  discovery_start: () => {},
  discovery_stop: () => {},
  session_local: unavailable,
  session_host: unavailable,
  host_qr: unavailable,
  scan_qr: () => null,
};

/** Installe l'équivalent de `window.__TAURI__` pour l'interface. */
export async function install() {
  await init();
  loadProfiles();
  await resumeSaved();
  return {
    web: true,
    core: {
      invoke: async (cmd, args, opts) => {
        const fn = COMMANDS[cmd];
        if (!fn) throw new Error(`Commande inconnue : ${cmd}`);
        try {
          return await fn(args ?? {}, opts);
        } catch (e) {
          throw typeof e === 'string' ? e : (e?.message ?? String(e));
        }
      },
    },
    event: {
      listen: async (name, cb) => {
        (listeners[name] ??= []).push(cb);
        return () => { listeners[name] = listeners[name].filter((x) => x !== cb); };
      },
    },
  };
}
