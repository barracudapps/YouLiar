//! Protocole réseau v2, sur TCP :
//!
//! 1. Trames binaires préfixées par leur longueur (u32 big-endian, ≤ `MAX_FRAME`).
//! 2. Poignée de main en clair : `[version u32][message SPAKE2]` dans chaque sens
//!    (voir `secure.rs` : le code de partie authentifie l'échange sans être transmis).
//! 3. Ensuite, chaque trame est un message JSON chiffré (ChaCha20-Poly1305).
//!    Une trame d'un seul octet est un refus de l'hôte (`REFUSE_*`).
//!
//! Découverte sur le réseau local par balise UDP diffusée (broadcast).

use crate::game::{Action, GameView, PlayerInfo};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 2;
/// Taille maximale d'une trame (protège la mémoire contre un pair malveillant).
pub const MAX_FRAME: usize = 256 * 1024;
/// Taille maximale d'une balise de découverte.
pub const MAX_BEACON: usize = 1024;
pub const REFUSE_AUTH: u8 = 0;
pub const REFUSE_VERSION: u8 = 1;
pub const REFUSE_BUSY: u8 = 2;
/// Longueur du code de partie.
pub const CODE_LEN: usize = 6;
pub const DEFAULT_TCP_PORT: u16 = 47820;
pub const DISCOVERY_PORT: u16 = 47821;
pub const BEACON_MAGIC: &str = "youliar";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Premier message chiffré. `resume` : jeton fourni par l'hôte lors d'une connexion
    /// précédente, pour reprendre sa place après une coupure.
    Hello { players: Vec<PlayerInfo>, resume: Option<String> },
    Action { action: Action },
    /// Niveau vocal instantané du sujet (0-1), ~10 fois/s, pour le polygraphe en direct.
    /// Seule cette amplitude circule : jamais l'audio.
    Live { level: f32 },
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Welcome { device: String, resume: String },
    State { view: Box<GameView> },
    Error { message: String },
    Kicked { reason: String },
    Live { player: String, level: f32 },
    Pong,
}

/// Annonce périodique d'une partie sur le réseau local.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Beacon {
    pub magic: String,
    pub version: u32,
    pub name: String,
    pub host_name: String,
    pub port: u16,
    pub players: usize,
    pub open: bool,
}

impl Beacon {
    /// Borne les champs d'une balise reçue (donnée non fiable).
    pub fn sanitized(mut self) -> Self {
        self.name = self.name.chars().take(40).collect();
        self.host_name = self.host_name.chars().take(60).collect();
        self.players = self.players.min(99);
        self
    }
}

/// Code de partie aléatoire à `CODE_LEN` chiffres.
pub fn new_code() -> String {
    use rand::Rng;
    let mut rng = rand::rngs::OsRng;
    (0..CODE_LEN).map(|_| char::from(b'0' + rng.gen_range(0..10u8))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let m = ClientMsg::Action { action: Action::Vote { voter: "x".into(), lie: true, sure: true } };
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("\"type\":\"action\""));
        assert_eq!(serde_json::from_str::<ClientMsg>(&s).unwrap(), m);
    }
}
