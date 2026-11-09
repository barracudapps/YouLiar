//! Canal chiffré et authentifié par le code de la partie.
//!
//! * **SPAKE2** (échange de clés authentifié par mot de passe) : le code n'est jamais
//!   transmis et une écoute passive ne permet pas de le retrouver hors ligne. Un faux
//!   hôte (balise usurpée) qui ignore le code ne peut pas établir la session.
//! * Chaque connexion obtient sa propre clé : un autre joueur qui connaît le code mais
//!   écoute le Wi-Fi ne peut pas lire la consigne secrète destinée au sujet.
//! * **ChaCha20-Poly1305**, une clé par sens, nonce = compteur strictement croissant :
//!   toute modification, réinjection ou réordonnancement d'un message est rejeté.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};

const ID_CLIENT: &[u8] = b"youliar-v2-client";
const ID_HOST: &[u8] = b"youliar-v2-host";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecureError {
    /// Code incorrect (ou message altéré).
    Auth,
    /// Compteur épuisé (2^64 messages : théorique).
    Exhausted,
}

pub enum Role {
    Client,
    Host,
}

pub struct Handshake {
    state: Spake2<Ed25519Group>,
    role: Role,
}

impl Handshake {
    /// Démarre l'échange ; renvoie le message à envoyer au pair.
    pub fn start(role: Role, code: &str) -> (Self, Vec<u8>) {
        let pw = Password::new(format!("youliar:{}", code.trim()).into_bytes());
        let (id_a, id_b) = (Identity::new(ID_CLIENT), Identity::new(ID_HOST));
        let (state, msg) = match role {
            Role::Client => Spake2::<Ed25519Group>::start_a(&pw, &id_a, &id_b),
            Role::Host => Spake2::<Ed25519Group>::start_b(&pw, &id_a, &id_b),
        };
        (Self { state, role }, msg)
    }

    /// Termine l'échange avec le message du pair. Si les codes diffèrent, les deux côtés
    /// obtiennent des clés différentes : l'erreur apparaît au premier message déchiffré.
    pub fn finish(self, peer_msg: &[u8]) -> Result<(Sealer, Opener), SecureError> {
        let shared = self.state.finish(peer_msg).map_err(|_| SecureError::Auth)?;
        let hk = Hkdf::<Sha256>::new(Some(b"youliar-v2"), &shared);
        let mut c2h = [0u8; 32];
        let mut h2c = [0u8; 32];
        hk.expand(b"client->host", &mut c2h).map_err(|_| SecureError::Auth)?;
        hk.expand(b"host->client", &mut h2c).map_err(|_| SecureError::Auth)?;
        let (send, recv) = match self.role {
            Role::Client => (c2h, h2c),
            Role::Host => (h2c, c2h),
        };
        Ok((
            Sealer { cipher: ChaCha20Poly1305::new(Key::from_slice(&send)), counter: 0 },
            Opener { cipher: ChaCha20Poly1305::new(Key::from_slice(&recv)), counter: 0 },
        ))
    }
}

fn nonce(counter: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..8].copy_from_slice(&counter.to_le_bytes());
    n
}

pub struct Sealer {
    cipher: ChaCha20Poly1305,
    counter: u64,
}

impl Sealer {
    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, SecureError> {
        let n = nonce(self.counter);
        self.counter = self.counter.checked_add(1).ok_or(SecureError::Exhausted)?;
        self.cipher.encrypt(Nonce::from_slice(&n), plaintext).map_err(|_| SecureError::Exhausted)
    }
}

pub struct Opener {
    cipher: ChaCha20Poly1305,
    counter: u64,
}

impl Opener {
    pub fn open(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, SecureError> {
        let n = nonce(self.counter);
        let pt = self.cipher.decrypt(Nonce::from_slice(&n), ciphertext).map_err(|_| SecureError::Auth)?;
        self.counter = self.counter.checked_add(1).ok_or(SecureError::Exhausted)?;
        Ok(pt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(code_c: &str, code_h: &str) -> ((Sealer, Opener), (Sealer, Opener)) {
        let (c, mc) = Handshake::start(Role::Client, code_c);
        let (h, mh) = Handshake::start(Role::Host, code_h);
        (c.finish(&mh).unwrap(), h.finish(&mc).unwrap())
    }

    #[test]
    fn same_code_communicates_both_ways() {
        let ((mut cs, mut co), (mut hs, mut ho)) = pair("123456", "123456");
        let m = cs.seal(b"bonjour").unwrap();
        assert_ne!(&m[..], b"bonjour");
        assert_eq!(ho.open(&m).unwrap(), b"bonjour");
        assert_eq!(co.open(&hs.seal(b"salut").unwrap()).unwrap(), b"salut");
    }

    #[test]
    fn wrong_code_is_rejected() {
        let ((mut cs, _), (_, mut ho)) = pair("123456", "654321");
        assert_eq!(ho.open(&cs.seal(b"x").unwrap()), Err(SecureError::Auth));
    }

    #[test]
    fn tampering_and_replay_are_rejected() {
        let ((mut cs, _), (_, mut ho)) = pair("42", "42");
        let a = cs.seal(b"a").unwrap();
        let mut bad = a.clone();
        bad[0] ^= 1;
        assert!(ho.open(&bad).is_err());
        assert!(ho.open(&a).is_ok());
        assert!(ho.open(&a).is_err(), "rejeu");
        let _skipped = cs.seal(b"b").unwrap();
        let c = cs.seal(b"c").unwrap();
        assert!(ho.open(&c).is_err(), "message sauté / réordonné");
    }

    #[test]
    fn garbage_handshake_fails_cleanly() {
        let (h, _) = Handshake::start(Role::Host, "1");
        assert!(h.finish(b"nope").is_err());
    }
}
