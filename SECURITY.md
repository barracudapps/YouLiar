# Sécurité

## Modèle de menace

Voir la section « Sécurité réseau » du README : connexions SPAKE2 + ChaCha20-Poly1305,
limitation des tentatives, validation de toutes les données reçues, reprise de place par
jeton secret, données personnelles stockées localement en 0600.

Côté interface : CSP stricte (pas de script en ligne, pas de plugin, pas de cadre, pas de
formulaire, `base-uri 'none'`), prototypes JavaScript gelés (`freezePrototype`), échappement
de toute valeur venant d'un autre appareil, permissions Tauri réduites à l'écoute
d'événements. Les images (caméra, scanner de QR code) sont réduites dans la page ; seules
des mesures (couleur moyenne, mouvement) ou une image en niveaux de gris bornée
(≤ 1280 × 960, QR code) atteignent le cœur Rust.

## Joueurs dans un navigateur

* La page n'est servie que pendant qu'une partie réseau est hébergée, sur le réseau local, en
  HTTPS (TLS 1.2/1.3, `rustls`). Seuls les fichiers de l'interface embarqués dans
  l'application sont servis ; en-têtes `Content-Security-Policy` (avec `'wasm-unsafe-eval'`
  pour le moteur WebAssembly, connexions limitées à l'hôte), `nosniff`, `no-referrer`,
  `frame-ancestors 'none'`, `Permissions-Policy` (caméra et micro pour cette origine seulement).
* Le WebSocket est ponté vers le même service que les connexions de l'application : SPAKE2
  avec le code, chiffrement ChaCha20-Poly1305, limites de débit, de taille et de tentatives.
  64 connexions HTTP simultanées au plus.
* **Limite assumée** : le certificat est auto-signé (aucune autorité n'existe sur un réseau
  local sans Internet). Au premier accès, le navigateur avertit et le joueur l'accepte ; un
  attaquant actif sur le même réseau pourrait à ce moment présenter sa propre page.
  L'empreinte du certificat est affichée sur l'écran de l'hôte pour comparaison. Le code de
  partie, lui, ne transite jamais en clair.
* Les profils du navigateur (empreinte vocale, mesures) sont dans son stockage local ;
  « Réglages → Effacer toutes mes données » les supprime.

## Audit des dépendances

```bash
cargo install cargo-audit --locked
cargo audit
```

Dernier audit (0.8.0, 473 dépendances) : **aucune vulnérabilité connue**. Le lecteur de
QR code est en `rqrr` 0.11 : la 0.9 tirait `lru` 0.12, signalée non sûre (RUSTSEC-2026-0002). Deux
avertissements, tous deux tirés par la pile Linux de Tauri (GTK 3), hors de notre contrôle
tant que Tauri n'a pas migré :

| Avis | Crate | Nature | Impact pour YouLiar |
|---|---|---|---|
| RUSTSEC-2024-0370 | `proc-macro-error` 1.0.4 | non maintenue | macro de compilation uniquement, rien dans le binaire livré |
| RUSTSEC-2024-0429 | `glib` 0.18.5 | API non sûre (`VariantStrIter`) | Linux seulement ; YouLiar n'appelle pas cette API |

## Signaler un problème

Merci d'écrire à l'auteur (voir l'historique git) plutôt que d'ouvrir un ticket public, avec
la version (`Réglages` → bas de page) et les étapes pour reproduire.
