//! Rejoindre sans installer l'application : l'hôte sert l'interface aux navigateurs du
//! réseau local, en HTTPS (les navigateurs n'autorisent le micro et la caméra que sur
//! une origine sécurisée).
//!
//! * Certificat auto-signé, créé une fois puis conservé (0600) : chaque téléphone n'a à
//!   accepter l'avertissement du navigateur qu'une seule fois.
//! * Seuls les fichiers de l'interface embarqués dans l'application sont servis.
//! * `/ws` : WebSocket dont les messages binaires portent exactement les mêmes trames que
//!   la connexion TCP de l'application ; il est ponté vers `net::serve_peer`, donc même
//!   poignée de main SPAKE2, même chiffrement, mêmes limites. Le moteur d'analyse tourne
//!   dans le navigateur (WebAssembly) : la voix ne quitte toujours pas l'appareil.

use crate::net::{serve_peer, HostShared, Rd, Wr};
use futures_util::{SinkExt, StreamExt};
use flate2::{write::GzEncoder, Compression};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Write;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

pub const DEFAULT_WEB_PORT: u16 = 47822;
const MAX_HEAD: usize = 8 * 1024;
const MAX_HTTP_CONNECTIONS: usize = 64;
/// Un navigateur ouvre quelques connexions en parallèle (fichiers + WebSocket) : au-delà,
/// un seul appareil ne doit pas pouvoir occuper toutes les places des autres invités.
const MAX_HTTP_PER_IP: usize = 12;

/// Connexions ouvertes par adresse ; la place est rendue quand le jeton est lâché.
#[derive(Clone, Default)]
struct PerIp(Arc<Mutex<HashMap<IpAddr, usize>>>);

struct IpSlot(PerIp, IpAddr);

impl PerIp {
    fn try_take(&self, ip: IpAddr) -> Option<IpSlot> {
        let mut map = self.0.lock().ok()?;
        let n = map.entry(ip).or_default();
        if *n >= MAX_HTTP_PER_IP {
            return None;
        }
        *n += 1;
        Some(IpSlot(self.clone(), ip))
    }
}

impl Drop for IpSlot {
    fn drop(&mut self) {
        if let Ok(mut map) = self.0 .0.lock() {
            if let Some(n) = map.get_mut(&self.1) {
                *n -= 1;
                if *n == 0 {
                    map.remove(&self.1);
                }
            }
        }
    }
}

/// Versions compressées déjà calculées, par empreinte du contenu (l'interface est figée
/// dans l'application : quelques dizaines de fichiers au plus).
const MAX_GZ_CACHE: usize = 128;

/// Fichiers de l'interface : chemin → (contenu, type MIME).
pub type Assets = Arc<dyn Fn(&str) -> Option<(Vec<u8>, String)> + Send + Sync>;

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

async fn read_head<S: AsyncRead + Unpin>(stream: &mut S) -> Option<Request> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 || buf.len() + n > MAX_HEAD {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let text = std::str::from_utf8(&buf).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, path) = (first.next()?.to_string(), first.next()?.to_string());
    let headers = lines
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    Some(Request { method, path, headers })
}

fn mime_for(path: &str, fallback: &str) -> String {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "json" => "application/json",
        _ => fallback,
    }
    .to_string()
}

/// Hôte tel que vu par le navigateur, réduit aux caractères d'un nom ou d'une adresse.
fn safe_host(req: &Request) -> Option<String> {
    let h = req.header("host")?;
    h.chars().all(|c| c.is_ascii_alphanumeric() || ".:-[]".contains(c)).then(|| h.to_string())
}

async fn respond<S: AsyncWrite + Unpin>(stream: &mut S, status: &str, mime: &str, body: &[u8], extra: &str) {
    // Les fichiers de l'interface se revalident (ETag) ; tout le reste n'est jamais gardé.
    let cache = if extra.contains("ETag:") { "Cache-Control: no-cache\r\nVary: Accept-Encoding\r\n" } else { "Cache-Control: no-store\r\n" };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n\
         X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nCross-Origin-Opener-Policy: same-origin\r\n\
         Permissions-Policy: camera=(self), microphone=(self), geolocation=()\r\n{extra}{cache}Connection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body).await;
    let _ = stream.shutdown().await;
}

/// Traite une connexion HTTP(S) : fichier de l'interface, ou passage en WebSocket.
pub(crate) async fn serve_http<S>(h: Arc<HostShared>, assets: Assets, mut stream: S, ip: IpAddr)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let Ok(Some(req)) = tokio::time::timeout(Duration::from_secs(10), read_head(&mut stream)).await else { return };
    let path = req.path.split(['?', '#']).next().unwrap_or("/").to_string();

    if path == "/ws" {
        let upgrade = req.header("upgrade").is_some_and(|u| u.eq_ignore_ascii_case("websocket"));
        let Some(key) = req.header("sec-websocket-key").filter(|_| upgrade && req.method == "GET") else {
            return respond(&mut stream, "400 Bad Request", "text/plain", b"WebSocket attendu", "").await;
        };
        let accept = derive_accept_key(key.as_bytes());
        let head = format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n");
        if stream.write_all(head.as_bytes()).await.is_err() {
            return;
        }
        let config = WebSocketConfig::default().max_message_size(Some(512 * 1024)).max_frame_size(Some(512 * 1024));
        let ws = WebSocketStream::from_raw_socket(stream, Role::Server, Some(config)).await;
        return bridge(h, ws, ip).await;
    }

    if req.method != "GET" && req.method != "HEAD" {
        return respond(&mut stream, "405 Method Not Allowed", "text/plain", b"", "Allow: GET, HEAD\r\n").await;
    }
    let file = if path == "/" { "index.html".to_string() } else { path.trim_start_matches('/').to_string() };
    if file.contains("..") || file.contains('\\') || file.starts_with('.') {
        return respond(&mut stream, "404 Not Found", "text/plain", b"", "").await;
    }
    let Some((bytes, mime)) = assets(&file) else {
        return respond(&mut stream, "404 Not Found", "text/plain", b"", "").await;
    };
    let host = safe_host(&req).unwrap_or_default();
    let csp = format!(
        "Content-Security-Policy: default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self' wss://{host}; \
         img-src 'self' data: blob:; media-src 'self' blob: mediastream:; style-src 'self' 'unsafe-inline'; \
         object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\n"
    );
    let mime = mime_for(&file, &mime);
    let digest = Sha256::digest(&bytes);
    let etag = format!("\"{}\"", digest[..12].iter().map(|b| format!("{b:02x}")).collect::<String>());
    if req.header("if-none-match").is_some_and(|v| v.split(',').any(|t| t.trim() == etag)) {
        return respond(&mut stream, "304 Not Modified", &mime, b"", &format!("ETag: {etag}\r\n")).await;
    }
    let gzip = req.header("accept-encoding").is_some_and(|v| v.split(',').any(|e| e.trim().starts_with("gzip")));
    let packed = if gzip { gzipped(&etag, &mime, &bytes) } else { None };
    let mut extra = format!("{csp}ETag: {etag}\r\n");
    let body: &[u8] = match &packed {
        Some(gz) => {
            extra.push_str("Content-Encoding: gzip\r\n");
            gz
        }
        None => &bytes,
    };
    let body = if req.method == "HEAD" { &[][..] } else { body };
    respond(&mut stream, "200 OK", &mime, body, &extra).await;
}

/// Version gzip d'un fichier texte ou WebAssembly, calculée une fois : le moteur (≈ 600 Kio)
/// est ainsi envoyé environ trois fois plus vite sur un Wi-Fi chargé.
fn gzipped(etag: &str, mime: &str, bytes: &[u8]) -> Option<Arc<Vec<u8>>> {
    let compressible = mime.starts_with("text/") || ["javascript", "wasm", "json", "svg"].iter().any(|k| mime.contains(k));
    if !compressible || bytes.len() < 1024 {
        return None;
    }
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Vec<u8>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(gz) = cache.lock().ok()?.get(etag) {
        return Some(gz.clone());
    }
    let mut enc = GzEncoder::new(Vec::with_capacity(bytes.len() / 2), Compression::default());
    enc.write_all(bytes).ok()?;
    let gz = Arc::new(enc.finish().ok()?);
    if gz.len() >= bytes.len() {
        return None;
    }
    let mut map = cache.lock().ok()?;
    if map.len() >= MAX_GZ_CACHE {
        map.clear();
    }
    map.insert(etag.to_string(), gz.clone());
    Some(gz)
}

/// Relie un WebSocket au service des appareils : messages binaires ⇄ flux d'octets.
async fn bridge<S>(h: Arc<HostShared>, ws: WebSocketStream<S>, ip: IpAddr)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut ws_tx, mut ws_rx) = ws.split();
    let (near, far) = tokio::io::duplex(256 * 1024);
    let (mut near_r, mut near_w) = tokio::io::split(near);
    let upstream = tauri::async_runtime::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                Message::Binary(b) => {
                    if near_w.write_all(&b).await.is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        let _ = near_w.shutdown().await;
    });
    let downstream = tauri::async_runtime::spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        while let Ok(n) = near_r.read(&mut buf).await {
            if n == 0 || ws_tx.send(Message::Binary(buf[..n].to_vec().into())).await.is_err() {
                break;
            }
        }
        let _ = ws_tx.close().await;
    });
    let (r, w) = tokio::io::split(far);
    let (r, w): (Rd, Wr) = (Box::new(r), Box::new(w));
    serve_peer(h, r, w, ip).await;
    upstream.abort();
    // Laisse partir les derniers messages (refus, départ) avant de fermer.
    let _ = tokio::time::timeout(Duration::from_secs(2), downstream).await;
}

/// Certificat auto-signé conservé dans `dir` (créé au premier lancement).
fn tls_config(dir: Option<&PathBuf>, ips: &[String]) -> Result<(rustls::ServerConfig, String), String> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let paths = dir.map(|d| (d.join("web-cert.der"), d.join("web-key.der")));
    let stored = paths.as_ref().and_then(|(c, k)| Some((std::fs::read(c).ok()?, std::fs::read(k).ok()?)));
    let (cert, key) = match stored {
        Some(pair) => pair,
        None => {
            let mut names: Vec<String> = ips.to_vec();
            names.extend(["localhost".to_string(), "127.0.0.1".to_string()]);
            let ck = rcgen::generate_simple_self_signed(names).map_err(|e| e.to_string())?;
            let pair = (ck.cert.der().to_vec(), ck.key_pair.serialize_der());
            if let Some((c, k)) = &paths {
                let _ = crate::write_private(c, &pair.0);
                let _ = crate::write_private(k, &pair.1);
            }
            pair
        }
    };
    let digest = Sha256::digest(&cert);
    let fingerprint = digest[..8].iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":");
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(cert)], PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)))
        .map_err(|e| e.to_string())?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok((config, fingerprint))
}

/// Démarre le serveur HTTPS de l'hôte et complète les informations d'invitation.
pub async fn start(h: &Arc<HostShared>, assets: Assets, cert_dir: Option<PathBuf>) -> Result<(), String> {
    let ips = h.info().map(|i| i.ips).unwrap_or_default();
    let (config, fingerprint) = tls_config(cert_dir.as_ref(), &ips)?;
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = match TcpListener::bind(("0.0.0.0", DEFAULT_WEB_PORT)).await {
        Ok(l) => l,
        Err(_) => TcpListener::bind(("0.0.0.0", 0)).await.map_err(|e| e.to_string())?,
    };
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    if let Some(info) = h.info.lock().unwrap().as_mut() {
        info.web_port = Some(port);
        info.fingerprint = Some(fingerprint);
        info.refresh_url();
    }
    let slots = Arc::new(Semaphore::new(MAX_HTTP_CONNECTIONS));
    let per_ip = PerIp::default();
    let hh = h.clone();
    h.spawn(async move {
        while let Ok((tcp, addr)) = listener.accept().await {
            // Appareil banni pour avoir raté le code : il n'obtient même plus la page.
            if hh.is_banned(addr.ip()) {
                continue;
            }
            let Some(ip_slot) = per_ip.try_take(addr.ip()) else { continue };
            let Ok(permit) = slots.clone().try_acquire_owned() else { continue };
            let _ = tcp.set_nodelay(true);
            let (h2, acceptor, assets) = (hh.clone(), acceptor.clone(), assets.clone());
            hh.spawn(async move {
                let _permit = (permit, ip_slot);
                if let Ok(Ok(tls)) = tokio::time::timeout(Duration::from_secs(10), acceptor.accept(tcp)).await {
                    serve_http(h2, assets, tls, addr.ip()).await;
                }
            });
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{start_host, Emit};
    use std::sync::Mutex;
    use youliar_core::game::{Action, PlayerInfo, HOST_DEVICE};
    use youliar_core::protocol::{ClientMsg, ServerMsg, PROTOCOL_VERSION};
    use youliar_core::secure::{Handshake, Role as SRole};

    fn assets() -> Assets {
        Arc::new(|p: &str| match p {
            "index.html" => Some((b"<!doctype html><title>YouLiar</title>".to_vec(), "text/html".into())),
            "app.js" => Some(("console.log('YouLiar');\n".repeat(200).into_bytes(), "text/javascript".into())),
            _ => None,
        })
    }

    async fn plain_server(h: Arc<HostShared>) -> u16 {
        let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = l.local_addr().unwrap().port();
        tauri::async_runtime::spawn(async move {
            while let Ok((s, a)) = l.accept().await {
                tauri::async_runtime::spawn(serve_http(h.clone(), assets(), s, a.ip()));
            }
        });
        port
    }

    async fn get(port: u16, path: &str) -> String {
        String::from_utf8_lossy(&request(port, path, "").await).into_owned()
    }

    async fn request(port: u16, path: &str, headers: &str) -> Vec<u8> {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{headers}\r\n").as_bytes()).await.unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        out
    }

    #[test]
    fn one_device_cannot_take_every_connection_slot() {
        let per_ip = PerIp::default();
        let (a, b): (IpAddr, IpAddr) = ("192.168.1.10".parse().unwrap(), "192.168.1.11".parse().unwrap());
        let held: Vec<_> = (0..MAX_HTTP_PER_IP).map(|_| per_ip.try_take(a).unwrap()).collect();
        assert!(per_ip.try_take(a).is_none(), "plafond atteint pour cet appareil");
        assert!(per_ip.try_take(b).is_some(), "les autres invités passent toujours");
        drop(held);
        assert!(per_ip.try_take(a).is_some(), "places rendues à la fermeture");
        assert!(per_ip.0.lock().unwrap().get(&b).is_none(), "aucune entrée qui traîne");
    }

    #[test]
    fn interface_files_are_compressed_and_revalidated() {
        tauri::async_runtime::block_on(async {
            let emit: Emit = Arc::new(|_: &str, _| {});
            let host = start_host(emit, "Web".into(), vec![], None).await.unwrap();
            let port = plain_server(host).await;
            let raw = request(port, "/app.js", "Accept-Encoding: gzip, br\r\n").await;
            let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
            let head = String::from_utf8_lossy(&raw[..split]).into_owned();
            assert!(head.contains("Content-Encoding: gzip") && head.contains("Cache-Control: no-cache"), "{head}");
            let mut plain = String::new();
            std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(&raw[split + 4..]), &mut plain).unwrap();
            assert_eq!(plain, "console.log('YouLiar');\n".repeat(200));
            assert!(raw.len() < plain.len() / 4);

            let etag = head.lines().find_map(|l| l.strip_prefix("ETag: ")).unwrap().to_string();
            let again = get(port, "/app.js").await;
            assert!(!again.contains("Content-Encoding"), "sans Accept-Encoding : contenu brut");
            let cached = String::from_utf8_lossy(&request(port, "/app.js", &format!("If-None-Match: {etag}\r\n")).await).into_owned();
            assert!(cached.starts_with("HTTP/1.1 304") && cached.ends_with("\r\n\r\n"), "{cached}");
            assert!(get(port, "/nope.js").await.contains("Cache-Control: no-store"));
        });
    }

    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut f = (payload.len() as u32).to_be_bytes().to_vec();
        f.extend_from_slice(payload);
        f
    }

    #[test]
    fn serves_the_interface_and_bridges_websocket_players() {
        tauri::async_runtime::block_on(async {
            let emit: Emit = Arc::new(|_: &str, _| {});
            let host = start_host(emit, "Web".into(), vec![], None).await.unwrap();
            host.apply(HOST_DEVICE, Action::AddPlayers { players: vec![PlayerInfo { id: "a".into(), name: "Alice".into(), avatar: "x".into(), calibrated: true }] }).unwrap();
            let port = plain_server(host.clone()).await;

            let page = get(port, "/").await;
            assert!(page.starts_with("HTTP/1.1 200") && page.contains("<title>YouLiar</title>"));
            assert!(page.contains("'wasm-unsafe-eval'") && page.contains("frame-ancestors 'none'"));
            assert!(get(port, "/../secret").await.starts_with("HTTP/1.1 404"));
            assert!(get(port, "/nope.js").await.starts_with("HTTP/1.1 404"));

            // Un « navigateur » rejoint par WebSocket avec le protocole chiffré habituel.
            let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            let (mut ws, _) = tokio_tungstenite::client_async(format!("ws://127.0.0.1:{port}/ws"), tcp).await.unwrap();
            let code = host.info().unwrap().code;
            let (hs, msg) = Handshake::start(SRole::Client, &code);
            let mut hello = PROTOCOL_VERSION.to_be_bytes().to_vec();
            hello.extend_from_slice(&msg);
            ws.send(Message::Binary(frame(&hello).into())).await.unwrap();
            // Les trames peuvent arriver découpées : on réassemble le flux.
            let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
            async fn next_frame<S: AsyncRead + AsyncWrite + Unpin>(ws: &mut WebSocketStream<S>, buf: &Mutex<Vec<u8>>) -> Vec<u8> {
                loop {
                    {
                        let mut b = buf.lock().unwrap();
                        if b.len() >= 4 {
                            let n = u32::from_be_bytes(b[..4].try_into().unwrap()) as usize;
                            if b.len() >= 4 + n {
                                let f = b[4..4 + n].to_vec();
                                b.drain(..4 + n);
                                return f;
                            }
                        }
                    }
                    if let Some(Ok(Message::Binary(d))) = ws.next().await {
                        buf.lock().unwrap().extend_from_slice(&d);
                    }
                }
            }
            let reply = next_frame(&mut ws, &buf).await;
            let (mut tx, mut rx) = hs.finish(&reply[4..]).unwrap();
            let me = ClientMsg::Hello { players: vec![PlayerInfo { id: "w".into(), name: "Web".into(), avatar: "🌐".into(), calibrated: false }], resume: None };
            ws.send(Message::Binary(frame(&tx.seal(&serde_json::to_vec(&me).unwrap()).unwrap()).into())).await.unwrap();
            let welcome: ServerMsg = serde_json::from_slice(&rx.open(&next_frame(&mut ws, &buf).await).unwrap()).unwrap();
            assert!(matches!(welcome, ServerMsg::Welcome { .. }));
            for _ in 0..50 {
                if host.view().players.iter().any(|p| p.info.id == "w") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(host.view().players.iter().any(|p| p.info.id == "w"), "le joueur du navigateur est dans le salon");
            host.shutdown();
        });
    }

    #[test]
    fn certificate_is_created_once_and_reused() {
        let dir = std::env::temp_dir().join(format!("youliar-cert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (_, fp1) = tls_config(Some(&dir), &["192.168.1.20".into()]).unwrap();
        let (_, fp2) = tls_config(Some(&dir), &["10.0.0.5".into()]).unwrap();
        assert_eq!(fp1, fp2, "même certificat d'une session à l'autre");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Test de bout en bout avec un vrai navigateur : `YOULIAR_E2E=<fichier> cargo test -p youliar
/// e2e_browser -- --ignored`, puis piloter un navigateur vers le port écrit dans le fichier.
#[cfg(test)]
mod e2e {
    use super::*;
    use crate::net::{start_host, Emit};
    use youliar_core::game::{Action, PlayerInfo, HOST_DEVICE};

    #[test]
    #[ignore]
    fn e2e_browser_join() {
        tauri::async_runtime::block_on(async {
            let emit: Emit = Arc::new(|_: &str, _| {});
            let host = start_host(emit, "E2E".into(), vec![], None).await.unwrap();
            host.apply(HOST_DEVICE, Action::AddPlayers { players: vec![PlayerInfo { id: "a".into(), name: "Alice".into(), avatar: "🦊".into(), calibrated: true }] }).unwrap();
            let ui = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ui");
            let assets: Assets = Arc::new(move |p: &str| std::fs::read(ui.join(p)).ok().map(|b| (b, "application/octet-stream".into())));
            start(&host, assets, None).await.unwrap();
            let info = host.info().unwrap();
            std::fs::write(std::env::var("YOULIAR_E2E").unwrap(), format!("{}\n{}\n", info.web_port.unwrap(), info.code)).unwrap();
            for _ in 0..600 {
                if let Some(p) = host.view().players.iter().find(|p| p.info.id != "a") {
                    println!("E2E joined: {} {}", p.info.name, p.info.avatar);
                    tokio::time::sleep(Duration::from_secs(12)).await;
                    let v = host.view();
                    println!("E2E end: {} players, {} connected", v.players.len(), v.players.iter().filter(|p| p.connected).count());
                    host.shutdown();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            panic!("aucun navigateur n'a rejoint");
        });
    }
}
