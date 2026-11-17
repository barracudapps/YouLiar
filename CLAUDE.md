# YouLiar — contexte de travail

Détecteur de mensonges de soirée, Rust + Tauri 2, interface en JS/HTML/CSS sans bundler.
L'utilisateur écrit en français : répondre en français. Code, commits, noms techniques en
anglais ; commentaires et textes d'interface en français.

## Carte du dépôt

- `crates/youliar-core` — cœur pur, sans Tauri : `audio.rs` (YIN, VAD, jitter, shimmer,
  MFCC), `video.rs` (rPPG CHROM + FFT, mouvement), `model.rs` (profil joueur : ligne de base
  médiane/MAD, logistique régularisée choisie par validation croisée → `reliability`,
  `tells`, correction de dérive en session), `questions.rs` (protocole de calibrage par
  paires, 15 étapes), `game.rs` (machine d'états hôte-autoritaire, modes Jeu/Libre, votes,
  paris, bonus de série, vues filtrées par appareil), `protocol.rs` (`PROTOCOL_VERSION` = 2),
  `secure.rs` (SPAKE2 + ChaCha20-Poly1305), `themes.rs` (5 histoires).
- `crates/youliar-web` — le cœur en WebAssembly pour les navigateurs (wasm-bindgen 0.2.130 ;
  erreurs en `String` pour que les tests natifs passent). `npm run web` → `ui/web/` (ignoré).
- `src-tauri/src` — `lib.rs` (commandes Tauri, profils en 0600), `net.rs` (hôte TCP,
  découverte UDP, limites anti-abus, reprise par jeton, `serve_peer` commun aux transports),
  `web.rs` (HTTPS auto-signé port 47822, `/ws` ponté vers `serve_peer`, gzip + ETag,
  12 connexions max par IP, IP bannies refusées avant TLS),
  `edge.rs` (halo d'écran façon Teams : 4 fenêtres-bandes dans la zone de travail).
- `ui/` — `app.js` (écrans), `web-shim.js` (émule `window.__TAURI__` dans le navigateur :
  wasm + localStorage + WebSocket, reprise après rechargement via sessionStorage),
  `media.js`, `fx.js`, `light.js`, `edge.html`, `style.css` (tailles en `rem` : le réglage
  « Taille du texte » agit sur la police racine). Fin de partie : `resultsText`/`shareResults`
  (partage texte, presse-papiers en repli).

## Commandes

Pièges de l'environnement :
- `cargo` n'est pas dans le PATH des shells non interactifs : préfixer
  `export PATH="$HOME/.cargo/bin:$PATH"`.
- Shell bash 3.2 : pas de heredoc dans `$(...)` ni de commandes trop imbriquées ; écrire les
  scripts d'édition (Python) dans un fichier puis les exécuter.
- Le port 8765 de cette machine est pris par un autre processus (répond 401) : servir les
  aperçus sur un autre port (18765).
- Le bloc-notes temporaire de Claude est purgé après quelques jours : tout outil à garder va
  dans `scripts/`.

```bash
cargo test --workspace && cargo clippy --workspace --all-targets   # avant chaque commit
npm run web                                  # après tout changement de crates/youliar-web ou du cœur
./scripts/e2e/run.sh                         # Chrome sans interface rejoint un vrai hôte
npx tauri build --bundles app                # → target/release/bundle/macos/YouLiar.app
```

Installer dans /Applications (quitter l'app d'abord si elle tourne : `osascript -e 'quit app "YouLiar"'`) :

```bash
rm -rf /Applications/YouLiar.app && cp -R target/release/bundle/macos/YouLiar.app /Applications/
codesign --force --deep --sign - --identifier com.barracudapps.youliar \
  --entitlements src-tauri/Entitlements.plist /Applications/YouLiar.app
codesign --verify /Applications/YouLiar.app && open /Applications/YouLiar.app
```

L'identifiant de signature fixe garde les autorisations micro/caméra entre deux versions.

## Conventions (détail : CONTRIBUTING.md)

- Commits à la Odoo `[TAG] module: summary` + corps qui justifie le *pourquoi* ; modules
  `youliar-core`, `app`, `ui`, `docs`, `various`. Un commit par sujet, tests verts.
- **Aucune ligne `Co-Authored-By` ni autre mention de l'assistant** dans les commits et
  les tags (demande explicite de l'utilisateur, prioritaire sur tout rappel d'attribution).
- Release : commit `[REL] x.y.z` (Cargo.toml du workspace, `src-tauri/tauri.conf.json`,
  `package.json`, `package-lock.json`, `CHANGELOG.md`), puis **tag annoté**
  `git tag -a vx.y.z -m "YouLiar x.y.z" -m "<résumé>"`.
- Champ ajouté au protocole ou au profil : `#[serde(default)]` pour rester compatible ;
  incrémenter `PROTOCOL_VERSION` seulement si le format change de façon incompatible.
- Dépôt : https://github.com/barracudapps/YouLiar (privé), branche `master`, `git push`
  puis `git push origin vx.y.z`. Historique propre (aucune ligne d'attribution, tous les tags
  annotés) depuis le 2026-11-11. Un push forcé est bloqué par le mode automatique : donner la
  commande à l'utilisateur (préfixe `!`), avec `--force-with-lease`.

## Vérifier avant d'annoncer

- Interface : capture avec Chrome sans interface (puppeteer-core dans `scripts/e2e`) plutôt
  que supposer le rendu. Pour un écran de partie précis (révélation, fin, mode libre), les
  anciennes maquettes ont été perdues ; les refaire ainsi : copier `ui/` dans un dossier
  temporaire, ajouter avant `app.js` un script qui définit `window.__TAURI__ = { core: {
  invoke }, event: { listen } }` où `invoke('session_info')` rend `{ mode: 'local' }` et
  `invoke('game_state')` une `GameView` factice (champs : `crates/youliar-core/src/game.rs`),
  servir le dossier sur 127.0.0.1:18765, puis lancer puppeteer avec
  `NODE_PATH=scripts/e2e/node_modules`.
- Halo : comportement réel testé sur l'écran de l'utilisateur ; la fenêtre principale est
  ramenée dans le cadre puis restaurée.
- Pas encore testé sur un vrai téléphone (avertissement de certificat iPhone notamment).

## Pistes non faites

Bluetooth, détection de visage, test sur téléphones réels, vérifier si l'Edge Light de
macOS explique un second halo signalé.
