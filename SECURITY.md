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

## Audit des dépendances

```bash
cargo install cargo-audit --locked
cargo audit
```

Dernier audit (0.6.0, 454 dépendances) : **aucune vulnérabilité connue**. Le lecteur de
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
