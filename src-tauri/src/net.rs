//! Sessions de jeu : locale (un seul appareil), hôte réseau ou invité.
//!
//! Transport : TCP sur le réseau local (même Wi-Fi, partage de connexion / point
//! d'accès d'un téléphone…). Découverte automatique par balise UDP diffusée ;
//! saisie manuelle de l'IP possible (réseaux qui filtrent le broadcast).
//!
//! Sécurité (voir `youliar_core::secure` et `protocol`) :
//! * échange SPAKE2 authentifié par le code à 6 chiffres, puis chiffrement
//!   ChaCha20-Poly1305 avec une clé propre à chaque connexion ;
//! * trames de taille bornée, 16 connexions max, poignée de main limitée à 10 s ;
//! * 5 échecs de code par adresse IP ⇒ blocage 5 min ; 20 échecs au total ⇒ nouveau code ;
//! * débit de messages limité par connexion, files d'envoi bornées ;
//! * toutes les données reçues sont bornées / validées par `Game::apply`.
//!
//! Seuls les verdicts calculés circulent : la voix et l'image ne quittent jamais
//! l'appareil du joueur.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::async_runtime::JoinHandle;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{mpsc, Semaphore};
use youliar_core::game::{Action, Game, GameView, PlayerInfo, Stage, HOST_DEVICE};
use youliar_core::protocol::*;
use youliar_core::secure::{Handshake, Opener, Role, Sealer};

pub const EVT_STATE: &str = "game-state";
pub const EVT_ERROR: &str = "game-error";
pub const EVT_ENDED: &str = "session-ended";
pub const EVT_DISCOVERY: &str = "discovery";
pub const EVT_LINK: &str = "link-status";
pub const EVT_HOST_INFO: &str = "host-info";
pub const EVT_LIVE: &str = "live-level";

#[derive(Debug, Clone, Serialize)]
pub struct LiveLevel {
    pub player: String,
    pub level: f32,
}

const MAX_CONNECTIONS: usize = 16;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_MSGS_PER_SEC: u32 = 30;
const IP_MAX_FAILURES: u32 = 5;
const IP_BAN: Duration = Duration::from_secs(300);
const GLOBAL_MAX_FAILURES: usize = 20;
const OUTBOX: usize = 64;

/// Canal vers l'interface (événements Tauri en production, collecteur en test).
pub type Emit = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

/// Persistance des questions déjà posées (fichier côté application, rien en test).
pub type UsedSink = Arc<dyn Fn(&[String]) + Send + Sync>;

fn send_evt<T: Serialize>(emit: &Emit, name: &str, payload: T) {
    if let Ok(v) = serde_json::to_value(payload) {
        emit(name, v);
    }
}

fn random_hex(bytes: usize) -> String {
    use rand::RngCore;
    let mut b = vec![0u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ---------- Trames ----------

fn bad_data(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<Vec<u8>> {
    let len = r.read_u32().await? as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(bad_data("trame hors limites"));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, data: &[u8]) -> io::Result<()> {
    if data.is_empty() || data.len() > MAX_FRAME {
        return Err(bad_data("trame hors limites"));
    }
    let mut out = Vec::with_capacity(4 + data.len());
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
    w.write_all(&out).await
}

fn handshake_payload(msg: &[u8]) -> Vec<u8> {
    let mut v = PROTOCOL_VERSION.to_be_bytes().to_vec();
    v.extend_from_slice(msg);
    v
}

fn parse_handshake(frame: &[u8]) -> Option<(u32, &[u8])> {
    (frame.len() > 4).then(|| (u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]), &frame[4..]))
}

/// Moitiés d'un flux d'octets : TCP (application) ou WebSocket ponté (navigateur).
pub(crate) type Rd = Box<dyn AsyncRead + Unpin + Send>;
pub(crate) type Wr = Box<dyn AsyncWrite + Unpin + Send>;

struct SecureTx {
    w: Wr,
    sealer: Sealer,
}

impl SecureTx {
    async fn send<T: Serialize>(&mut self, msg: &T) -> io::Result<()> {
        let json = serde_json::to_vec(msg).map_err(io::Error::other)?;
        let ct = self.sealer.seal(&json).map_err(|_| bad_data("chiffrement"))?;
        write_frame(&mut self.w, &ct).await
    }
}

struct SecureRx {
    r: Rd,
    opener: Opener,
}

impl SecureRx {
    async fn recv<T: DeserializeOwned>(&mut self) -> io::Result<T> {
        let frame = read_frame(&mut self.r).await?;
        if frame.len() == 1 {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, refusal_reason(frame[0])));
        }
        let pt = self.opener.open(&frame).map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "auth"))?;
        serde_json::from_slice(&pt).map_err(|_| bad_data("message invalide"))
    }
}

fn refusal_reason(code: u8) -> &'static str {
    match code {
        REFUSE_AUTH => "Code de partie incorrect.",
        REFUSE_VERSION => "Version de YouLiar incompatible avec l'hôte.",
        _ => "L'hôte refuse la connexion (partie pleine ou trop de tentatives).",
    }
}

// ---------- Hôte ----------

#[derive(Debug, Clone, Serialize)]
pub struct HostInfo {
    pub ips: Vec<String>,
    pub port: u16,
    pub code: String,
    /// Page servie aux navigateurs (rejoindre sans installer l'application).
    pub web_port: Option<u16>,
    pub web_url: Option<String>,
    /// Empreinte du certificat HTTPS, à comparer en cas de doute.
    pub fingerprint: Option<String>,
}

impl HostInfo {
    fn new(ips: Vec<String>, port: u16) -> Self {
        Self { ips, port, code: new_code(), web_port: None, web_url: None, fingerprint: None }
    }

    /// Lien d'invitation : la page de l'hôte, avec le code et les adresses pour
    /// l'application (le fragment `#…` n'est jamais envoyé au serveur).
    pub(crate) fn refresh_url(&mut self) {
        self.web_url = match (self.web_port, self.ips.first()) {
            (Some(wp), Some(ip)) => Some(format!("https://{ip}:{wp}/#join={}&p={}&ips={}", self.code, self.port, self.ips.join(","))),
            _ => None,
        };
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum SessionInfo {
    None,
    Local,
    Host { info: HostInfo },
    Client { host: String },
}

struct Peer {
    conn: u64,
    tx: mpsc::Sender<ServerMsg>,
}

#[derive(Default)]
struct AuthGuard {
    per_ip: HashMap<IpAddr, (u32, Option<Instant>)>,
    recent: VecDeque<Instant>,
}

pub struct HostShared {
    emit: Emit,
    game: Mutex<Game>,
    peers: Mutex<HashMap<String, Peer>>,
    /// Jeton de reprise -> appareil.
    tokens: Mutex<HashMap<String, String>>,
    guard: Mutex<AuthGuard>,
    slots: Arc<Semaphore>,
    next_id: AtomicU64,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    pub(crate) info: Mutex<Option<HostInfo>>,
    /// Manche dont la révélation automatique est déjà programmée.
    vote_timer: Mutex<Option<u32>>,
    used_sink: Option<UsedSink>,
    used_mark: Mutex<(usize, Option<String>)>,
}

impl HostShared {
    fn new(emit: Emit, game: Game, info: Option<HostInfo>, used_sink: Option<UsedSink>) -> Self {
        Self {
            vote_timer: Mutex::new(None),
            used_mark: Mutex::new((game.used_questions().len(), game.used_questions().last().cloned())),
            used_sink,
            emit,
            game: Mutex::new(game),
            peers: Mutex::new(HashMap::new()),
            tokens: Mutex::new(HashMap::new()),
            guard: Mutex::new(AuthGuard::default()),
            slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            next_id: AtomicU64::new(1),
            tasks: Mutex::new(vec![]),
            info: Mutex::new(info),
        }
    }

    pub fn info(&self) -> Option<HostInfo> {
        self.info.lock().unwrap().clone()
    }

    fn code(&self) -> String {
        self.info.lock().unwrap().as_ref().map(|i| i.code.clone()).unwrap_or_default()
    }

    pub fn apply(self: &Arc<Self>, device: &str, action: Action) -> Result<(), String> {
        self.game.lock().unwrap().apply(device, action)?;
        self.after_change();
        Ok(())
    }

    /// Diffuse le nouvel état, programme la révélation automatique du vote et mémorise
    /// les questions posées.
    fn after_change(self: &Arc<Self>) {
        self.broadcast();
        let (deadline, used) = {
            let g = self.game.lock().unwrap();
            let q = g.used_questions();
            let mark = (q.len(), q.last().cloned());
            let mut last = self.used_mark.lock().unwrap();
            let changed = *last != mark;
            *last = mark;
            (g.vote_deadline(), changed.then(|| q.to_vec()))
        };
        if let (Some(sink), Some(used)) = (&self.used_sink, used) {
            sink(&used);
        }
        let Some((round, end)) = deadline else { return };
        {
            let mut scheduled = self.vote_timer.lock().unwrap();
            if *scheduled == Some(round) {
                return;
            }
            *scheduled = Some(round);
        }
        let h = Arc::clone(self);
        self.spawn(async move {
            tokio::time::sleep(end.saturating_duration_since(Instant::now())).await;
            let revealed = {
                let mut g = h.game.lock().unwrap();
                g.vote_deadline().is_some_and(|(n, _)| n == round) && g.apply(HOST_DEVICE, Action::Reveal).is_ok()
            };
            if revealed {
                h.after_change();
            }
        });
    }

    pub fn view(&self) -> GameView {
        self.game.lock().unwrap().view_for(HOST_DEVICE)
    }

    /// Relaie le niveau vocal du sujet aux autres appareils. Message jetable : jamais
    /// au prix d'une file d'envoi saturée.
    fn relay_live(&self, device: &str, level: f32) {
        let Some(player) = self.game.lock().unwrap().live_subject(device) else { return };
        let level = if level.is_finite() { level.clamp(0.0, 1.0) } else { 0.0 };
        for (dev, p) in self.peers.lock().unwrap().iter() {
            if dev != device && p.tx.capacity() > OUTBOX / 2 {
                let _ = p.tx.try_send(ServerMsg::Live { player: player.clone(), level });
            }
        }
        if device != HOST_DEVICE {
            send_evt(&self.emit, EVT_LIVE, LiveLevel { player, level });
        }
    }

    fn broadcast(&self) {
        let game = self.game.lock().unwrap();
        let mut peers = self.peers.lock().unwrap();
        // Un invité trop lent (file pleine) est déconnecté plutôt que de faire grossir la mémoire.
        peers.retain(|dev, p| p.tx.try_send(ServerMsg::State { view: Box::new(game.view_for(dev)) }).is_ok());
        send_evt(&self.emit, EVT_STATE, game.view_for(HOST_DEVICE));
    }

    pub(crate) fn spawn(&self, fut: impl std::future::Future<Output = ()> + Send + 'static) {
        let t = tauri::async_runtime::spawn(fut);
        let mut tasks = self.tasks.lock().unwrap();
        tasks.retain(|t| !t.inner().is_finished());
        tasks.push(t);
    }

    fn beacon(&self, port: u16) -> Beacon {
        let g = self.game.lock().unwrap();
        let host_name = g.players.iter().filter(|p| p.device == HOST_DEVICE).map(|p| p.info.name.clone()).collect::<Vec<_>>().join(", ");
        Beacon {
            magic: BEACON_MAGIC.into(),
            version: PROTOCOL_VERSION,
            name: g.settings.game_name.clone(),
            host_name,
            port,
            players: g.players.len(),
            // En mode libre, la salle reste ouverte aux retardataires.
            open: g.stage == Stage::Lobby || g.settings.mode == youliar_core::game::GameMode::Free,
        }
    }

    pub(crate) fn is_banned(&self, ip: IpAddr) -> bool {
        let mut g = self.guard.lock().unwrap();
        match g.per_ip.get(&ip) {
            Some((_, Some(until))) if Instant::now() < *until => true,
            Some((_, Some(_))) => {
                g.per_ip.remove(&ip);
                false
            }
            _ => false,
        }
    }

    /// Enregistre un échec d'authentification ; change le code si les essais se multiplient.
    fn auth_failed(&self, ip: IpAddr) {
        let rotate = {
            let mut g = self.guard.lock().unwrap();
            let e = g.per_ip.entry(ip).or_insert((0, None));
            e.0 += 1;
            if e.0 >= IP_MAX_FAILURES {
                e.1 = Some(Instant::now() + IP_BAN);
            }
            let now = Instant::now();
            g.recent.push_back(now);
            while g.recent.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(600)) {
                g.recent.pop_front();
            }
            let rotate = g.recent.len() >= GLOBAL_MAX_FAILURES;
            if rotate {
                g.recent.clear();
            }
            rotate
        };
        if rotate {
            let info = {
                let mut i = self.info.lock().unwrap();
                if let Some(info) = i.as_mut() {
                    info.code = new_code();
                    info.refresh_url();
                }
                i.clone()
            };
            send_evt(&self.emit, EVT_HOST_INFO, info);
            send_evt(&self.emit, EVT_ERROR, "Trop de tentatives de connexion échouées : le code de la partie a été changé.");
        }
    }

    pub fn shutdown(&self) {
        for (_, p) in self.peers.lock().unwrap().drain() {
            let _ = p.tx.try_send(ServerMsg::Kicked { reason: "L'hôte a fermé la partie.".into() });
        }
        // Laisse le temps aux messages d'adieu de partir.
        let tasks: Vec<_> = self.tasks.lock().unwrap().drain(..).collect();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            tasks.iter().for_each(|t| t.abort());
        });
    }
}

/// Adresses IPv4 locales (hors boucle locale) et adresses de diffusion associées.
fn local_ipv4() -> Vec<(Ipv4Addr, Option<Ipv4Addr>)> {
    let mut out: Vec<(Ipv4Addr, Option<Ipv4Addr>)> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v4) if !v4.ip.is_link_local() => Some((v4.ip, v4.broadcast)),
            _ => None,
        })
        .collect();
    // Les réseaux privés usuels d'abord (Wi-Fi, partage de connexion).
    out.sort_by_key(|(ip, _)| !ip.is_private());
    out.dedup_by_key(|(ip, _)| *ip);
    out
}

pub fn start_local(emit: Emit, used: Vec<String>, used_sink: Option<UsedSink>) -> Arc<HostShared> {
    let mut game = Game::default();
    game.set_used_questions(used);
    let h = Arc::new(HostShared::new(emit, game, None, used_sink));
    h.broadcast();
    h
}

pub async fn start_host(emit: Emit, game_name: String, used: Vec<String>, used_sink: Option<UsedSink>) -> Result<Arc<HostShared>, String> {
    let listener = match TcpListener::bind(("0.0.0.0", DEFAULT_TCP_PORT)).await {
        Ok(l) => l,
        Err(_) => TcpListener::bind(("0.0.0.0", 0)).await.map_err(|e| format!("Impossible d'ouvrir le port réseau : {e}"))?,
    };
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let mut game = Game::default();
    game.set_used_questions(used);
    game.apply(HOST_DEVICE, Action::UpdateSettings { settings: youliar_core::game::Settings { game_name, ..Default::default() } })?;
    let info = HostInfo::new(local_ipv4().iter().map(|(ip, _)| ip.to_string()).collect(), port);
    let h = Arc::new(HostShared::new(emit, game, Some(info), used_sink));

    let hh = h.clone();
    h.spawn(async move {
        while let Ok((stream, addr)) = listener.accept().await {
            let h2 = hh.clone();
            hh.spawn(async move { handle_peer(h2, stream, addr).await });
        }
    });
    let hh = h.clone();
    h.spawn(async move { beacon_loop(hh, port).await });
    h.broadcast();
    Ok(h)
}

async fn beacon_loop(h: Arc<HostShared>, port: u16) {
    let Ok(sock) = UdpSocket::bind(("0.0.0.0", 0)).await else { return };
    let _ = sock.set_broadcast(true);
    loop {
        if let Ok(payload) = serde_json::to_vec(&h.beacon(port)) {
            let mut targets = vec![Ipv4Addr::BROADCAST];
            targets.extend(local_ipv4().into_iter().filter_map(|(_, b)| b));
            for t in targets {
                let _ = sock.send_to(&payload, (t, DISCOVERY_PORT)).await;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Poignée de main côté hôte. Renvoie les deux moitiés chiffrées et le premier message.
async fn host_handshake(h: &HostShared, mut r: Rd, mut w: Wr, ip: IpAddr) -> Option<(SecureTx, SecureRx, ClientMsg)> {
    let frame = read_frame(&mut r).await.ok()?;
    let (version, msg) = parse_handshake(&frame)?;
    if version != PROTOCOL_VERSION {
        let _ = write_frame(&mut w, &[REFUSE_VERSION]).await;
        return None;
    }
    let (hs, out) = Handshake::start(Role::Host, &h.code());
    write_frame(&mut w, &handshake_payload(&out)).await.ok()?;
    let Ok((sealer, opener)) = hs.finish(msg) else {
        h.auth_failed(ip);
        let _ = write_frame(&mut w, &[REFUSE_AUTH]).await;
        return None;
    };
    let mut rx = SecureRx { r, opener };
    match rx.recv::<ClientMsg>().await {
        Ok(hello @ ClientMsg::Hello { .. }) => Some((SecureTx { w, sealer }, rx, hello)),
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            // Clés différentes : le code saisi est faux.
            h.auth_failed(ip);
            let mut w = w;
            let _ = write_frame(&mut w, &[REFUSE_AUTH]).await;
            None
        }
        _ => None,
    }
}

async fn handle_peer(h: Arc<HostShared>, stream: TcpStream, addr: SocketAddr) {
    let _ = stream.set_nodelay(true);
    let (r, w) = stream.into_split();
    serve_peer(h, Box::new(r), Box::new(w), addr.ip()).await;
}

/// Sert un appareil, quel que soit le transport (TCP direct, ou WebSocket ponté pour
/// un navigateur) : mêmes limites, même poignée de main chiffrée, mêmes règles.
pub(crate) async fn serve_peer(h: Arc<HostShared>, r: Rd, mut w: Wr, ip: IpAddr) {
    let Ok(_slot) = h.slots.clone().try_acquire_owned() else {
        let _ = write_frame(&mut w, &[REFUSE_BUSY]).await;
        return;
    };
    if h.is_banned(ip) {
        let _ = write_frame(&mut w, &[REFUSE_BUSY]).await;
        return;
    }
    let Ok(Some((mut tx, mut rx, hello))) = tokio::time::timeout(HANDSHAKE_TIMEOUT, host_handshake(&h, r, w, ip)).await else {
        return;
    };
    let ClientMsg::Hello { players, resume } = hello else { return };

    // Reprise après coupure, ou nouvel appareil.
    let known = resume.and_then(|t| h.tokens.lock().unwrap().get(&t).map(|d| (t, d.clone())));
    let (device, token) = match known {
        Some((t, d)) => (d, t),
        None => {
            let d = format!("d{}", h.next_id.fetch_add(1, Ordering::Relaxed));
            let t = random_hex(16);
            h.tokens.lock().unwrap().insert(t.clone(), d.clone());
            (d, t)
        }
    };
    let joined = {
        let mut g = h.game.lock().unwrap();
        if g.device_rejoined(&device) { Ok(()) } else { g.apply(&device, Action::AddPlayers { players }) }
    };
    if let Err(reason) = joined {
        let _ = tx.send(&ServerMsg::Kicked { reason }).await;
        return;
    }

    let conn = h.next_id.fetch_add(1, Ordering::Relaxed);
    let (out_tx, mut out_rx) = mpsc::channel::<ServerMsg>(OUTBOX);
    let _ = out_tx.try_send(ServerMsg::Welcome { device: device.clone(), resume: token });
    // Remplace une éventuelle connexion fantôme du même appareil.
    h.peers.lock().unwrap().insert(device.clone(), Peer { conn, tx: out_tx.clone() });
    h.after_change();

    let writer = tauri::async_runtime::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            let last = matches!(msg, ServerMsg::Kicked { .. });
            if tx.send(&msg).await.is_err() || last {
                break;
            }
        }
        let _ = tx.w.shutdown().await;
    });

    let mut window = Instant::now();
    let mut count = 0u32;
    // Les invités envoient un « ping » toutes les 5 s : 30 s de silence = appareil perdu.
    while let Ok(Ok(msg)) = tokio::time::timeout(IDLE_TIMEOUT, rx.recv::<ClientMsg>()).await {
        if window.elapsed() >= Duration::from_secs(1) {
            window = Instant::now();
            count = 0;
        }
        count += 1;
        if count > MAX_MSGS_PER_SEC {
            break; // inondation : on coupe
        }
        match msg {
            ClientMsg::Action { action } => {
                if let Err(message) = h.apply(&device, action) {
                    let _ = out_tx.try_send(ServerMsg::Error { message });
                }
            }
            ClientMsg::Live { level } => h.relay_live(&device, level),
            ClientMsg::Ping => {
                let _ = out_tx.try_send(ServerMsg::Pong);
            }
            ClientMsg::Hello { .. } => {}
        }
    }
    let current = {
        let mut peers = h.peers.lock().unwrap();
        match peers.get(&device) {
            Some(p) if p.conn != conn => false, // déjà remplacée par une reconnexion
            Some(_) => {
                peers.remove(&device);
                true
            }
            None => true,
        }
    };
    if current {
        h.game.lock().unwrap().device_left(&device);
        h.after_change();
    }
    writer.abort();
}

// ---------- Invité ----------

pub struct ClientHandle {
    tx: mpsc::Sender<ClientMsg>,
    online: Arc<AtomicBool>,
    task: JoinHandle<()>,
    pub host: String,
    pub last_view: Arc<Mutex<Option<GameView>>>,
}

pub enum Session {
    Host(Arc<HostShared>),
    Client(ClientHandle),
}

impl Session {
    pub fn info(&self) -> SessionInfo {
        match self {
            Session::Host(h) => match h.info() {
                Some(info) => SessionInfo::Host { info },
                None => SessionInfo::Local,
            },
            Session::Client(c) => SessionInfo::Client { host: c.host.clone() },
        }
    }

    pub fn act(&self, action: Action) -> Result<(), String> {
        match self {
            Session::Host(h) => h.apply(HOST_DEVICE, action),
            Session::Client(c) => {
                if !c.online.load(Ordering::Relaxed) {
                    return Err("Reconnexion à l'hôte en cours…".into());
                }
                c.tx.try_send(ClientMsg::Action { action }).map_err(|_| "Connexion perdue avec l'hôte.".to_string())
            }
        }
    }

    /// Niveau vocal du sujet local, à diffuser aux autres appareils.
    pub fn live(&self, level: f32) {
        match self {
            Session::Host(h) => h.relay_live(HOST_DEVICE, level),
            Session::Client(c) => {
                if c.online.load(Ordering::Relaxed) {
                    let _ = c.tx.try_send(ClientMsg::Live { level });
                }
            }
        }
    }

    pub fn view(&self) -> Option<GameView> {
        match self {
            Session::Host(h) => Some(h.view()),
            Session::Client(c) => c.last_view.lock().unwrap().clone(),
        }
    }

    pub fn stop(self) {
        match self {
            Session::Host(h) => h.shutdown(),
            Session::Client(c) => c.task.abort(),
        }
    }
}

struct Connected {
    tx: SecureTx,
    rx: SecureRx,
    resume: String,
}

/// Erreur de connexion : `fatal` si inutile de réessayer (code faux, version…).
struct JoinError {
    message: String,
    fatal: bool,
}

async fn connect(addr: SocketAddr, code: &str, players: &[PlayerInfo], resume: Option<String>) -> Result<Connected, JoinError> {
    let soft = |m: &str| JoinError { message: m.to_string(), fatal: false };
    let stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr))
        .await
        .map_err(|_| soft("L'hôte ne répond pas (même réseau ? pare-feu ?)."))?
        .map_err(|e| soft(&format!("Connexion impossible : {e}")))?;
    let _ = stream.set_nodelay(true);
    let (r, w) = stream.into_split();
    let (mut r, mut w): (Rd, Wr) = (Box::new(r), Box::new(w));
    let handshake = async {
        let (hs, out) = Handshake::start(Role::Client, code);
        write_frame(&mut w, &handshake_payload(&out)).await.map_err(|_| soft("Connexion interrompue."))?;
        let frame = read_frame(&mut r).await.map_err(|_| soft("L'hôte a fermé la connexion."))?;
        if frame.len() == 1 {
            return Err(JoinError { message: refusal_reason(frame[0]).into(), fatal: true });
        }
        let (_, msg) = parse_handshake(&frame).ok_or_else(|| soft("Réponse invalide de l'hôte."))?;
        let (sealer, opener) = hs.finish(msg).map_err(|_| JoinError { message: refusal_reason(REFUSE_AUTH).into(), fatal: true })?;
        let mut tx = SecureTx { w, sealer };
        let mut rx = SecureRx { r, opener };
        tx.send(&ClientMsg::Hello { players: players.to_vec(), resume }).await.map_err(|_| soft("Connexion interrompue."))?;
        match rx.recv::<ServerMsg>().await {
            Ok(ServerMsg::Welcome { resume, .. }) => Ok(Connected { tx, rx, resume }),
            Ok(ServerMsg::Kicked { reason }) => Err(JoinError { message: reason, fatal: true }),
            Ok(_) => Err(soft("Réponse inattendue de l'hôte.")),
            // Un hôte qui n'a pas pu déchiffrer notre Hello renvoie un refus d'authentification.
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => Err(JoinError { message: e.to_string(), fatal: true }),
            Err(_) => Err(soft("L'hôte n'a pas répondu.")),
        }
    };
    tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake).await.map_err(|_| soft("L'hôte n'a pas répondu."))?
}

enum Ended {
    Kicked(String),
    Lost,
    Stopped,
}

/// Fait vivre une connexion établie jusqu'à sa fin.
async fn run_connection(c: Connected, cmds: &mut mpsc::Receiver<ClientMsg>, emit: &Emit, last_view: &Mutex<Option<GameView>>) -> Ended {
    let Connected { mut tx, mut rx, .. } = c;
    // La lecture d'une trame n'est pas interruptible sans perte : elle vit dans sa propre tâche.
    let (in_tx, mut inbox) = mpsc::channel::<ServerMsg>(OUTBOX);
    let reader = tauri::async_runtime::spawn(async move {
        while let Ok(Ok(m)) = tokio::time::timeout(IDLE_TIMEOUT, rx.recv::<ServerMsg>()).await {
            if in_tx.send(m).await.is_err() {
                break;
            }
        }
    });
    let mut ping = tokio::time::interval(Duration::from_secs(5));
    let ended = loop {
        tokio::select! {
            m = inbox.recv() => match m {
                Some(ServerMsg::State { view }) => {
                    let view = *view;
                    *last_view.lock().unwrap() = Some(view.clone());
                    send_evt(emit, EVT_STATE, view);
                }
                Some(ServerMsg::Error { message }) => send_evt(emit, EVT_ERROR, message),
                Some(ServerMsg::Live { player, level }) => send_evt(emit, EVT_LIVE, LiveLevel { player, level }),
                Some(ServerMsg::Kicked { reason }) => break Ended::Kicked(reason),
                Some(_) => {}
                None => break Ended::Lost,
            },
            cmd = cmds.recv() => match cmd {
                Some(m) => if tx.send(&m).await.is_err() { break Ended::Lost },
                None => break Ended::Stopped,
            },
            _ = ping.tick() => {
                if tx.send(&ClientMsg::Ping).await.is_err() { break Ended::Lost }
            }
        }
    };
    reader.abort();
    ended
}

pub async fn join(emit: Emit, ip: String, port: u16, code: String, players: Vec<PlayerInfo>) -> Result<ClientHandle, String> {
    let addr: IpAddr = ip.trim().parse().map_err(|_| "Adresse IP invalide.".to_string())?;
    let addr = SocketAddr::new(addr, port);
    let code: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    if code.len() != CODE_LEN {
        return Err(format!("Le code de partie comporte {CODE_LEN} chiffres."));
    }
    let first = connect(addr, &code, &players, None).await.map_err(|e| e.message)?;

    let (tx, mut cmds) = mpsc::channel::<ClientMsg>(32);
    let online = Arc::new(AtomicBool::new(true));
    let last_view = Arc::new(Mutex::new(None::<GameView>));
    let (on, lv) = (online.clone(), last_view.clone());
    let task = tauri::async_runtime::spawn(async move {
        let mut resume = first.resume.clone();
        let mut conn = first;
        loop {
            match run_connection(conn, &mut cmds, &emit, &lv).await {
                Ended::Stopped => return,
                Ended::Kicked(reason) => {
                    send_evt(&emit, EVT_ENDED, reason);
                    return;
                }
                Ended::Lost => {}
            }
            // Coupure (veille du téléphone, Wi-Fi instable…) : on tente de reprendre sa place.
            on.store(false, Ordering::Relaxed);
            send_evt(&emit, EVT_LINK, "reconnecting");
            let mut next = None;
            for delay in [1u64, 2, 3, 5, 5, 5, 5, 5] {
                tokio::time::sleep(Duration::from_secs(delay)).await;
                match connect(addr, &code, &players, Some(resume.clone())).await {
                    Ok(c) => {
                        next = Some(c);
                        break;
                    }
                    Err(e) if e.fatal => {
                        send_evt(&emit, EVT_ENDED, e.message);
                        return;
                    }
                    Err(_) => {}
                }
            }
            let Some(c) = next else {
                send_evt(&emit, EVT_ENDED, "Connexion perdue avec l'hôte.");
                return;
            };
            resume = c.resume.clone();
            conn = c;
            // Vide les commandes émises pendant la coupure : elles seraient périmées.
            while cmds.try_recv().is_ok() {}
            on.store(true, Ordering::Relaxed);
            send_evt(&emit, EVT_LINK, "connected");
        }
    });
    Ok(ClientHandle { tx, online, task, host: addr.to_string(), last_view })
}

// ---------- Découverte ----------

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredGame {
    pub ip: String,
    pub port: u16,
    pub name: String,
    pub host_name: String,
    pub players: usize,
    pub open: bool,
}

/// Écoute les balises des parties du réseau local et publie la liste à l'interface.
pub fn start_discovery(emit: Emit) -> Result<JoinHandle<()>, String> {
    use socket2::{Domain, Protocol, Socket, Type};
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).map_err(|e| e.to_string())?;
    let _ = sock.set_reuse_address(true);
    #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
    let _ = sock.set_reuse_port(true);
    let _ = sock.set_broadcast(true);
    sock.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)).into())
        .map_err(|e| format!("Découverte indisponible ({e}). Utilise l'adresse IP."))?;
    sock.set_nonblocking(true).map_err(|e| e.to_string())?;
    let std_sock: std::net::UdpSocket = sock.into();

    Ok(tauri::async_runtime::spawn(async move {
        let Ok(udp) = UdpSocket::from_std(std_sock) else { return };
        let mut seen: HashMap<String, (DiscoveredGame, Instant)> = HashMap::new();
        let mut buf = vec![0u8; MAX_BEACON];
        let mut tick = tokio::time::interval(Duration::from_millis(1000));
        loop {
            tokio::select! {
                r = udp.recv_from(&mut buf) => {
                    let Ok((n, from)) = r else { continue };
                    let Ok(b) = serde_json::from_slice::<Beacon>(&buf[..n]) else { continue };
                    if b.magic != BEACON_MAGIC || b.version != PROTOCOL_VERSION { continue; }
                    // Borne le nombre de parties suivies (balises usurpées en masse).
                    if seen.len() >= 32 { continue; }
                    let b = b.sanitized();
                    let ip = from.ip().to_string();
                    let key = format!("{ip}:{}", b.port);
                    seen.insert(key, (DiscoveredGame { ip, port: b.port, name: b.name, host_name: b.host_name, players: b.players, open: b.open }, Instant::now()));
                }
                _ = tick.tick() => {
                    seen.retain(|_, (_, t)| t.elapsed() < Duration::from_secs(4));
                    let mut list: Vec<DiscoveredGame> = seen.values().map(|(g, _)| g.clone()).collect();
                    list.sort_by(|a, b| a.name.cmp(&b.name).then(a.ip.cmp(&b.ip)));
                    send_evt(&emit, EVT_DISCOVERY, list);
                }
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use youliar_core::model::Verdict;

    type Log = Arc<Mutex<Vec<(String, serde_json::Value)>>>;

    fn collector() -> (Emit, Log) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let l = log.clone();
        (Arc::new(move |n: &str, v| l.lock().unwrap().push((n.to_string(), v))), log)
    }

    fn info(id: &str) -> PlayerInfo {
        PlayerInfo { id: id.into(), name: id.into(), avatar: "x".into(), calibrated: true }
    }

    async fn wait_for(log: &Log, pred: impl Fn(&GameView) -> bool) -> GameView {
        for _ in 0..100 {
            let found = log.lock().unwrap().iter().rev().find(|(n, _)| n == EVT_STATE).and_then(|(_, v)| serde_json::from_value::<GameView>(v.clone()).ok());
            if let Some(v) = found.filter(|v| pred(v)) {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        panic!("état attendu jamais reçu");
    }

    fn local(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    #[test]
    fn host_and_client_play_a_round_over_encrypted_tcp() {
        tauri::async_runtime::block_on(async {
            let (host_emit, host_log) = collector();
            let host = start_host(host_emit, "Test".into(), vec![], None).await.unwrap();
            let info_h = host.info().unwrap();
            assert_eq!(info_h.code.len(), CODE_LEN);
            host.apply(HOST_DEVICE, Action::AddPlayers { players: vec![info("alice")] }).unwrap();

            // Mauvais code : refusé, sans que le code ne circule.
            let wrong = if info_h.code == "000000" { "111111" } else { "000000" };
            let (e, _) = collector();
            let bad = join(e, "127.0.0.1".into(), info_h.port, wrong.into(), vec![info("bob")]).await;
            assert!(matches!(bad, Err(ref m) if m.contains("Code")));

            let (cl_emit, cl_log) = collector();
            let client = join(cl_emit, "127.0.0.1".into(), info_h.port, info_h.code.clone(), vec![info("bob")]).await.unwrap();
            wait_for(&host_log, |v| v.players.len() == 2).await;
            let session = Session::Client(client);

            assert!(session.act(Action::Start).is_ok()); // envoyé…
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(host.view().stage, Stage::Lobby); // …mais refusé par l'hôte
            assert!(cl_log.lock().unwrap().iter().any(|(n, _)| n == EVT_ERROR));

            host.apply(HOST_DEVICE, Action::Start).unwrap();
            let v = wait_for(&cl_log, |v| v.stage == Stage::Asking).await;
            let round = v.round.unwrap();
            assert_eq!(round.subject, "alice");
            assert_eq!(round.lie, None, "la consigne de l'hôte ne doit pas fuiter");

            session.act(Action::ChooseQuestion { index: 1 }).unwrap(); // bob interroge
            wait_for(&cl_log, |v| v.round.as_ref().is_some_and(|r| r.question_locked)).await;
            host.apply(HOST_DEVICE, Action::StartAnswer).unwrap();
            wait_for(&cl_log, |v| v.stage == Stage::Answering).await;
            Session::Host(host.clone()).live(0.7);
            for _ in 0..50 {
                if cl_log.lock().unwrap().iter().any(|(n, v)| n == EVT_LIVE && v["player"] == "alice") { break; }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(cl_log.lock().unwrap().iter().any(|(n, v)| n == EVT_LIVE && v["player"] == "alice"), "tracé en direct relayé");
            session.live(0.5); // bob n'est pas le sujet : ignoré
            let verdict = Verdict { probability: 0.9, index: 90, label: "x".into(), is_lie: true, factors: vec![], heart_rate: None, confidence: 0.5, speaker: None };
            host.apply(HOST_DEVICE, Action::SubmitVerdict { verdict }).unwrap();
            wait_for(&cl_log, |v| v.stage == Stage::Voting).await;
            session.act(Action::Vote { voter: "bob".into(), lie: true, sure: false }).unwrap();
            let v = wait_for(&cl_log, |v| v.stage == Stage::Reveal).await;
            assert!(v.round.unwrap().verdict.is_some());

            // L'invité part : il est marqué déconnecté.
            session.stop();
            let v = wait_for(&host_log, |v| v.players.iter().any(|p| p.info.id == "bob" && !p.connected)).await;
            assert_eq!(v.stage, Stage::Reveal);
            host.shutdown();
        });
    }

    #[test]
    fn device_can_resume_its_seat_after_a_drop() {
        tauri::async_runtime::block_on(async {
            let (emit, log) = collector();
            let host = start_host(emit, "T".into(), vec![], None).await.unwrap();
            let hi = host.info().unwrap();
            host.apply(HOST_DEVICE, Action::AddPlayers { players: vec![info("alice")] }).unwrap();
            let c = connect(local(hi.port), &hi.code, &[info("bob")], None).await.ok().unwrap();
            let token = c.resume.clone();
            host.apply(HOST_DEVICE, Action::Start).unwrap();
            drop(c); // coupure brutale
            wait_for(&log, |v| v.players.iter().any(|p| p.info.id == "bob" && !p.connected)).await;
            // Un autre appareil ne peut pas prendre la place de bob sans le jeton…
            let thief = connect(local(hi.port), &hi.code, &[info("bob")], Some("faux".into())).await;
            assert!(matches!(thief, Err(JoinError { fatal: true, .. })));
            // …mais bob revient avec son jeton.
            let _c2 = connect(local(hi.port), &hi.code, &[info("bob")], Some(token)).await.ok().unwrap();
            wait_for(&log, |v| v.players.iter().any(|p| p.info.id == "bob" && p.connected)).await;
            host.shutdown();
        });
    }

    #[test]
    fn vote_is_revealed_automatically_and_questions_are_persisted() {
        tauri::async_runtime::block_on(async {
            let (emit, log) = collector();
            let saved: Arc<Mutex<Vec<String>>> = Arc::default();
            let s2 = saved.clone();
            let sink: UsedSink = Arc::new(move |q: &[String]| *s2.lock().unwrap() = q.to_vec());
            let host = start_local(emit, vec![], Some(sink));
            host.apply(HOST_DEVICE, Action::AddPlayers { players: vec![info("a"), info("b")] }).unwrap();
            let mut settings = host.view().settings;
            settings.vote_seconds = 1;
            host.apply(HOST_DEVICE, Action::UpdateSettings { settings }).unwrap();
            host.apply(HOST_DEVICE, Action::Start).unwrap();
            assert_eq!(saved.lock().unwrap().len(), 3, "les 3 questions proposées sont mémorisées");
            host.apply(HOST_DEVICE, Action::ChooseQuestion { index: 0 }).unwrap();
            host.apply(HOST_DEVICE, Action::StartAnswer).unwrap();
            let verdict = Verdict { probability: 0.9, index: 90, label: String::new(), is_lie: true, factors: vec![], heart_rate: None, confidence: 0.5, speaker: None };
            host.apply(HOST_DEVICE, Action::SubmitVerdict { verdict }).unwrap();
            assert_eq!(host.view().stage, Stage::Voting);
            // Personne ne vote : l'hôte révèle seul à l'échéance.
            wait_for(&log, |v| v.stage == Stage::Reveal).await;
            host.shutdown();
        });
    }

    #[test]
    fn hostile_peers_are_contained() {
        tauri::async_runtime::block_on(async {
            let (emit, log) = collector();
            let host = start_host(emit, "T".into(), vec![], None).await.unwrap();
            let hi = host.info().unwrap();
            // Trame annoncée géante : coupée sans allouer.
            let mut s = TcpStream::connect(local(hi.port)).await.unwrap();
            s.write_u32(u32::MAX).await.unwrap();
            let mut b = [0u8; 1];
            let n = tokio::time::timeout(Duration::from_secs(2), s.read(&mut b)).await.unwrap().unwrap_or(0);
            assert_eq!(n, 0, "connexion fermée");

            // Essais de code répétés : l'IP est bloquée.
            let wrong = if hi.code == "999999" { "888888" } else { "999999" };
            for _ in 0..IP_MAX_FAILURES {
                assert!(connect(local(hi.port), wrong, &[info("x")], None).await.is_err());
            }
            let blocked = connect(local(hi.port), &hi.code, &[info("x")], None).await;
            assert!(matches!(blocked, Err(JoinError { ref message, .. }) if message.contains("refuse")));
            assert!(host.view().players.is_empty());
            drop(log);
            host.shutdown();
        });
    }
}
