// Capture micro / caméra et synthèse vocale.
// Rien n'est envoyé sur le réseau : l'audio part vers le moteur Rust local,
// la caméra n'est résumée qu'en couleur moyenne du visage + mouvement.

const VIDEO_KEEP_S = 40;

export class Media {
  constructor() {
    this.stream = null;
    this.ctx = null;
    this.level = 0;          // RMS courant (0..1)
    this.recording = null;   // { chunks, onBuffer }
    this.camera = false;
    this.video = null;
    this.videoSamples = [];
    this._timer = null;
    this._fps = 30;
    // Application en arrière-plan : on cesse d'analyser les images.
    document.addEventListener('visibilitychange', () => this._restartSampler());
  }

  get ready() { return !!this.stream; }

  /** Démarre micro (+ caméra). Les appels simultanés partagent le même démarrage. */
  start(opts) {
    if (this.stream && this.camera === opts.camera) return Promise.resolve();
    if (this._starting) return this._starting;
    this._starting = this._start(opts).finally(() => { this._starting = null; });
    return this._starting;
  }

  async _start({ camera }) {
    this.stop();
    const constraints = {
      audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false, channelCount: 1 },
      video: camera ? { facingMode: 'user', width: { ideal: 640 }, height: { ideal: 480 }, frameRate: { ideal: 30 } } : false,
    };
    try {
      this.stream = await navigator.mediaDevices.getUserMedia(constraints);
    } catch (e) {
      if (camera) {
        // Caméra refusée ou absente : on continue avec le micro seul.
        this.stream = await navigator.mediaDevices.getUserMedia({ audio: constraints.audio });
        camera = false;
      } else {
        throw new Error("Accès au micro refusé. Autorise-le dans les réglages de l'appareil.");
      }
    }
    this.camera = camera;
    const AC = window.AudioContext || window.webkitAudioContext;
    this.ctx = new AC();
    const src = this.ctx.createMediaStreamSource(this.stream);
    const mute = this.ctx.createGain();
    mute.gain.value = 0;
    const onData = (data, owned) => {
      let s = 0;
      for (let i = 0; i < data.length; i++) s += data[i] * data[i];
      this.level = Math.sqrt(s / data.length);
      if (this.recording) {
        this.recording.chunks.push(owned ? data : new Float32Array(data));
        this.recording.onBuffer?.(this.level, data.length / this.ctx.sampleRate);
      }
    };
    let node;
    try {
      // AudioWorklet : la capture ne dépend plus de la fluidité de l'interface.
      await this.ctx.audioWorklet.addModule('recorder-worklet.js');
      node = new AudioWorkletNode(this.ctx, 'yl-tap', { numberOfInputs: 1, numberOfOutputs: 1, channelCount: 1 });
      node.port.onmessage = (ev) => onData(ev.data, true);
    } catch {
      // Moteurs plus anciens : ScriptProcessor (déprécié, mais universel).
      node = this.ctx.createScriptProcessor(2048, 1, 1);
      node.onaudioprocess = (ev) => onData(ev.inputBuffer.getChannelData(0), false);
    }
    src.connect(node);
    node.connect(mute);
    mute.connect(this.ctx.destination);
    this._node = node;
    if (camera) this._startVideo();
  }

  async resume() { if (this.ctx?.state === 'suspended') await this.ctx.resume(); }

  _startVideo() {
    this.video = document.createElement('video');
    this.video.muted = true;
    this.video.playsInline = true;
    this.video.setAttribute('playsinline', '');
    this.video.srcObject = this.stream;
    this.video.play().catch(() => {});
    const W = 64, H = 48;
    const canvas = document.createElement('canvas');
    canvas.width = W; canvas.height = H;
    const g = canvas.getContext('2d', { willReadFrequently: true });
    let prev = null;
    // Zone ovale centrale (visage) : mêmes proportions que le guide affiché.
    const roi = [];
    for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
      const dx = (x + .5 - W / 2) / (W * .2), dy = (y + .5 - H / 2) / (H * .35);
      if (dx * dx + dy * dy <= 1) roi.push(y * W + x);
    }
    this._sample = () => {
      if (!this.video || this.video.readyState < 2) return;
      g.drawImage(this.video, 0, 0, W, H);
      const px = g.getImageData(0, 0, W, H).data;
      let r = 0, gg = 0, b = 0;
      for (const i of roi) { r += px[i * 4]; gg += px[i * 4 + 1]; b += px[i * 4 + 2]; }
      const n = roi.length;
      const luma = new Float32Array(W * H);
      let motion = 0;
      for (let i = 0; i < W * H; i++) {
        luma[i] = .299 * px[i * 4] + .587 * px[i * 4 + 1] + .114 * px[i * 4 + 2];
        if (prev) motion += Math.abs(luma[i] - prev[i]);
      }
      motion = prev ? motion / (W * H) : 0;
      prev = luma;
      const t = performance.now() / 1000;
      this.videoSamples.push({ t, r: r / n, g: gg / n, b: b / n, motion });
      while (this.videoSamples.length && this.videoSamples[0].t < t - VIDEO_KEEP_S) this.videoSamples.shift();
    };
    this._restartSampler();
  }

  /**
   * Cadence d'analyse caméra : 30 i/s quand une mesure compte (pouls), 5 i/s sinon
   * (luminosité pour le halo) — l'analyse d'image coûte cher en batterie sur mobile.
   */
  setRate(fps) {
    if (fps === this._fps) return;
    this._fps = fps;
    this._restartSampler();
  }

  _restartSampler() {
    clearInterval(this._timer);
    this._timer = null;
    if (this._sample && !document.hidden) this._timer = setInterval(this._sample, Math.round(1000 / (this._fps || 30)));
  }

  /** Échantillons caméra des `seconds` dernières secondes (au moins `min`). */
  videoWindow(seconds, min = 11) {
    if (!this.camera) return null;
    const t = performance.now() / 1000;
    const s = this.videoSamples.filter((x) => x.t >= t - seconds);
    return s.length >= min ? s : null;
  }

  /** Attache un aperçu caméra dans `el`. */
  attachPreview(el) {
    if (!this.camera) return;
    const v = document.createElement('video');
    v.muted = true; v.playsInline = true; v.setAttribute('playsinline', '');
    v.srcObject = this.stream;
    v.play().catch(() => {});
    el.prepend(v);
  }

  /**
   * Enregistre une réponse. S'arrête seul après un silence suivant la parole,
   * ou à `maxS`. Renvoie { samples: Float32Array, sampleRate, cancelled }.
   */
  record({ maxS = 15, silenceS = 1.6, noSpeechS = 9, onLevel } = {}) {
    return new Promise((resolve) => {
      let elapsed = 0, floor = null, speaking = false, loud = 0, quiet = 0;
      const finish = (cancelled = false) => {
        const rec = this.recording;
        this.recording = null;
        const total = rec.chunks.reduce((a, c) => a + c.length, 0);
        const samples = new Float32Array(total);
        let o = 0;
        for (const c of rec.chunks) { samples.set(c, o); o += c.length; }
        resolve({ samples, sampleRate: this.ctx.sampleRate, cancelled });
      };
      this.recording = {
        chunks: [],
        onBuffer: (lvl, dt) => {
          elapsed += dt;
          onLevel?.(lvl);
          if (elapsed < .25) { floor = floor === null ? lvl : Math.min(floor, lvl); return; }
          const thr = Math.max((floor ?? 0) * 4, 0.012);
          if (lvl > thr) { loud += dt; quiet = 0; if (loud > .12) speaking = true; }
          else { quiet += dt; loud = 0; }
          if ((speaking && quiet > silenceS) || elapsed > maxS || (!speaking && elapsed > noSpeechS)) finish();
        },
      };
      this.stopRecording = () => this.recording && finish();
      this.cancelRecording = () => this.recording && finish(true);
    });
  }

  stop() {
    clearInterval(this._timer);
    this._timer = null;
    this._sample = null;
    this.recording = null;
    this.stream?.getTracks().forEach((t) => t.stop());
    this.ctx?.close().catch(() => {});
    this.stream = null; this.ctx = null; this.video = null; this.videoSamples = []; this.camera = false;
  }
}

/** Lit un texte (français) ; se résout à la fin de la lecture, ou tout de suite si indisponible. */
export function speak(text) {
  return new Promise((resolve) => {
    const synth = window.speechSynthesis;
    if (!synth || typeof SpeechSynthesisUtterance === 'undefined') return resolve(false);
    try {
      synth.cancel();
      const u = new SpeechSynthesisUtterance(text);
      u.lang = 'fr-FR';
      u.rate = 1.0;
      const voice = synth.getVoices().find((v) => v.lang?.startsWith('fr'));
      if (voice) u.voice = voice;
      let done = false;
      const end = (ok) => { if (!done) { done = true; resolve(ok); } };
      u.onend = () => end(true);
      u.onerror = () => end(false);
      setTimeout(() => end(true), 1500 + text.length * 90); // garde-fou
      synth.speak(u);
    } catch { resolve(false); }
  });
}

/** Tracé façon polygraphe qui défile. `source()` renvoie une amplitude 0..1. */
export class Trace {
  constructor(canvas, source) {
    this.c = canvas; this.source = source; this.pts = []; this.alive = true;
    const loop = () => {
      if (!this.alive || !this.c.isConnected) return;
      this.draw();
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
  }
  draw() {
    const dpr = window.devicePixelRatio || 1;
    const w = this.c.clientWidth, h = this.c.clientHeight;
    if (this.c.width !== w * dpr) { this.c.width = w * dpr; this.c.height = h * dpr; }
    const g = this.c.getContext('2d');
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    const amp = Math.min(1, this.source());
    const jitter = (Math.random() - .5) * (0.08 + amp * 1.6);
    this.pts.push(jitter);
    if (this.pts.length > w / 2) this.pts.shift();
    g.clearRect(0, 0, w, h);
    g.strokeStyle = 'rgba(255,255,255,.06)';
    for (let x = 0; x < w; x += 20) { g.beginPath(); g.moveTo(x, 0); g.lineTo(x, h); g.stroke(); }
    g.strokeStyle = '#ff5fa2';
    g.lineWidth = 2;
    g.beginPath();
    this.pts.forEach((p, i) => { const x = w - (this.pts.length - i) * 2, y = h / 2 + p * h * .45; i ? g.lineTo(x, y) : g.moveTo(x, y); });
    g.stroke();
  }
  stop() { this.alive = false; }
}
