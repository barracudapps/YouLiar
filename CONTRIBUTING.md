# Conventions

## Commits

Inspirées de l'historique d'Odoo : le diff dit *quoi*, le message dit *pourquoi*.

```
[TAG] module: short imperative summary in English

Body wrapped at 72 characters explaining WHY the change is needed:
the problem, the constraint, the alternatives rejected, the choices
made and their trade-offs.
```

| Tag | Usage |
|---|---|
| `[ADD]` | nouveau module ou composant |
| `[IMP]` | amélioration ou fonctionnalité dans un module existant |
| `[FIX]` | correctif |
| `[REF]` | refactoring sans changement de comportement |
| `[PERF]` | performance |
| `[SEC]` | durcissement de sécurité |
| `[REM]` | suppression |
| `[REV]` | revert |
| `[MOV]` | déplacement de fichiers |
| `[I18N]` | traductions |
| `[REL]` | version publiée (numéros de version, CHANGELOG, tag) |

Modules : `youliar-core` (crate d'analyse et de jeu), `app` (coque Tauri, `src-tauri/`),
`ui` (interface web), `docs`. Plusieurs modules : `[IMP] youliar-core, ui: …` ;
changement transverse : `[IMP] various: …`.

* Résumé ≤ 72 caractères, impératif, sans point final.
* Un commit = un sujet cohérent qui compile et dont les tests passent
  (`cargo test --workspace`, `cargo clippy --workspace --all-targets`).

## Versions (SemVer)

`MAJEUR.MINEUR.CORRECTIF`, une seule version pour tout le dépôt (crates Rust,
`tauri.conf.json`, `package.json`), publiée par un commit `[REL] x.y.z` et un tag annoté
`vx.y.z`.

Tant que la version est en `0.y.z` (pas encore éprouvée sur mobile en conditions réelles) :

* `0.MINEUR` : nouvelle fonctionnalité, ou changement incompatible (format des profils,
  protocole réseau) ;
* `0.y.CORRECTIF` : correctifs et améliorations compatibles.

À partir de `1.0.0` : MAJEUR pour toute incompatibilité (profils non relisibles, protocole),
MINEUR pour une fonctionnalité compatible, CORRECTIF pour un correctif.

`PROTOCOL_VERSION` (`crates/youliar-core/src/protocol.rs`) est indépendant : il augmente à
chaque changement du format échangé sur le réseau, pour que des versions incompatibles se
refusent proprement au lieu d'échouer en pleine partie.
