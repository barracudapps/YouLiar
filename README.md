# YouLiar 🕵️ — détecteur de mensonges de soirée

Application multiplateforme (macOS, Windows, Linux, Android, iOS) en **Rust + Tauri 2**.
Chaque joueur calibre sa voix (et sa caméra s'il le veut), puis répond à des questions
en mentant ou non. La machine rend un verdict, les autres votent, et on compte les points.

> ⚠️ C'est un **jeu**. La machine mesure des signes de stress et des écarts aux habitudes
> du joueur. Aucun algorithme ne détecte réellement le mensonge (les polygraphes non plus).

## Lancer

```bash
# Rust (si absent) : https://rustup.rs
npm install
npm run dev          # application de bureau en mode développement
npm run build        # paquet installable (.app/.dmg, .msi, .deb/.AppImage)
cargo test --workspace
```

### Mobile

```bash
npx tauri android init && npx tauri android dev   # Android Studio + NDK requis
npx tauri ios init && npx tauri ios dev           # Xcode requis (macOS)
```

* **Android** : ajouter `RECORD_AUDIO`, `CAMERA`, `INTERNET`, `ACCESS_WIFI_STATE`,
  `CHANGE_WIFI_MULTICAST_STATE` dans `gen/android/app/src/main/AndroidManifest.xml`.
* **iOS** : les textes d'autorisation micro / caméra / réseau local sont dans
  `src-tauri/Info.plist`. Pour la découverte automatique (broadcast UDP), Apple exige
  l'entitlement `com.apple.developer.networking.multicast` (sur demande). Sans lui, il
  suffit de saisir l'adresse IP affichée chez l'hôte.

## Modes de jeu

| Mode | Fonctionnement |
|---|---|
| **Même appareil** | Les joueurs se passent l'appareil. La consigne secrète s'affiche quand on la maintient appuyée. **Attribution de la voix** : la machine vérifie que c'est bien la bonne personne qui répond (empreinte vocale MFCC + hauteur). |
| **Réseau** | Un appareil héberge la partie et affiche un code à 6 chiffres et son adresse IP. Les autres la trouvent automatiquement (balise UDP) ou saisissent l'IP. Plusieurs joueurs peuvent partager un même appareil invité. |

**Connexion** : même Wi-Fi, ou **partage de connexion / point d'accès** d'un des téléphones,
ce qui remplace le Wi-Fi Direct (tous deux fournissent un réseau IP local) et fonctionne
sans internet. Le **Bluetooth** n'est pas implémenté : débit et API trop hétérogènes selon
les plateformes pour un gain faible ici. Voir « Pistes ».

**Confidentialité** : l'audio et l'image sont analysés sur l'appareil du joueur. Seuls le
verdict et les scores transitent sur le réseau. Les profils (empreinte vocale, pouls de
repos) sont stockés localement dans un fichier lisible par l'utilisateur seul (0600).

## Sécurité réseau

| Menace | Protection |
|---|---|
| Écoute du Wi-Fi (y compris par un autre joueur qui connaît le code) | Échange de clés **SPAKE2** authentifié par le code, puis **ChaCha20-Poly1305** avec une clé par connexion et par sens. La consigne secrète du sujet reste illisible. |
| Vol du code par écoute | Le code n'est jamais transmis ; une capture ne permet pas de le retrouver hors ligne. |
| Deviner le code | 6 chiffres ; 5 échecs ⇒ IP bloquée 5 min ; 20 échecs au total ⇒ nouveau code automatique. |
| Fausse partie annoncée (balise usurpée) | Un faux hôte qui ignore le code ne peut pas établir la session. |
| Messages modifiés, rejoués, réordonnés | Rejetés (authentification + compteur de nonce strict). |
| Saturation (mémoire, connexions, inondation) | Trames ≤ 256 Kio, 16 connexions max, poignée de main ≤ 10 s, 30 messages/s par connexion, files d'envoi bornées. |
| Données hostiles d'un pair (noms géants, NaN, HTML) | Bornées et validées par l'hôte (`PlayerInfo::sanitized`, `Verdict::sanitize`), échappées à l'affichage, CSP stricte. |
| Prise de place d'un joueur déconnecté | Reprise uniquement avec le jeton secret remis à l'appareil d'origine. |

Limite assumée : l'analyse se fait sur l'appareil du sujet (pour que la voix et l'image ne
le quittent pas). Un joueur qui modifierait l'application pourrait donc falsifier son propre
verdict. Pour un jeu de soirée, la confidentialité passe avant ce risque.

Une coupure (veille du téléphone, Wi-Fi instable) déclenche une **reconnexion automatique**
pendant ~35 s ; le joueur retrouve sa place dans la partie.

## Deux modes

* **🎮 Jeu** : manches structurées, consignes secrètes, interrogateur, votes, paris et points
  (détails ci-dessous).
* **🎤 Libre** : une salle d'interrogatoire ouverte, sans manches ni points. N'importe qui
  choisit un joueur et lui pose **la question qu'il veut** (ou pioche une idée), depuis
  n'importe quel appareil. La personne interrogée ment ou dit vrai à sa guise, la machine
  rend son verdict aussitôt, puis elle peut **avouer** (ce qui entraîne la machine) ou garder
  le secret. On peut rejoindre la salle en cours de route ; l'hôte la ferme avec « Terminer »,
  qui affiche toutes les questions posées.

Le mode se choisit dans le salon (par l'hôte), en local comme en réseau.

**Histoires** (mode jeu et mode libre) : 🪆 agent du KGB, 🕶️ espion de la CIA, 🎬 star de
cinéma, 🔎 meurtre au manoir, 🏴‍☠️ mutinerie à bord. Un prologue lu à voix haute ouvre la
partie, des chapitres font avancer l'intrigue (répartis sur toutes les manches, dénouement à
la dernière), les questions se jouent dans la peau de son personnage, les consignes et les
verdicts prennent le vocabulaire de l'histoire, et l'épilogue attribue un grade à chacun
selon le classement. Les textes sont dans `crates/youliar-core/src/themes.rs` ; seul
l'identifiant de l'histoire circule sur le réseau.

**Rejoindre** : scanner le QR code affiché par l'hôte (caméra de l'appareil qui rejoint,
décodage local), choisir une partie détectée sur le réseau, ou saisir l'adresse et le code.

**Rythme** : en mode jeu, le vote est limité dans le temps (15, 30 ou 60 s, ou illimité) ;
à l'échéance, la révélation est automatique. Les questions déjà posées sont mémorisées par
l'hôte pour ne pas revenir d'une soirée à l'autre (500 dernières).

**Réglages** : sons, vibrations, caméra par défaut, halo lumineux, voix et vitesse de lecture
des questions, effacement de toutes les données ; nom et avatar modifiables dans les profils.

## Déroulé (mode jeu)

1. **Calibrage** (~3 min, par joueur, conservé entre les parties) :
   test micro → caméra facultative (15 s immobile : pouls de repos, lumière) → 2 lectures
   (empreinte vocale, ton habituel) → comptage (ligne de base) → 4 vérités et 4 mensonges
   imposés, lus à voix haute par la machine.
2. **Manche** : un joueur passe sur la sellette et reçoit une consigne secrète (*MENS* /
   *DIS LA VÉRITÉ*, à maintenir appuyé pour la voir) ou choisit lui-même. Le joueur suivant
   devient **interrogateur** : il choisit parmi 3 questions ou écrit la sienne. Pendant la
   réponse, tous les appareils affichent le **polygraphe en direct** (seule l'amplitude de la
   voix circule, jamais l'audio).
3. **Vote** (avec pari facultatif « sûr de moi »), puis **révélation en trois temps** :
   l'aiguille hésite, se fige sur l'indice (0-100) avec les facteurs (« voix plus aiguë
   +1,8σ »…), puis la vérité éclate. Points :
   * vote juste : +1 (+2 si « sûr de moi ») ; vote faux « sûr de moi » : −1 ;
   * sujet : +1 par votant trompé, +2 s'il a berné la machine ;
   * interrogateur : +1 si la machine démasque le menteur (« question piège »).
4. La machine **apprend** de chaque réponse révélée (les poids du joueur sont réajustés).
5. **Fin de partie** : podium, trophées (Maître menteur, Détecteur humain, Sang-froid, Trop
   transparent, Flambeur, Question piège) et récapitulatif des manches.

Chaque profil affiche sa **carte d'identité vocale** (hauteur de voix, temps de réponse,
débit, silences, pouls de repos) et son bilan face à la machine. Sons synthétisés et
vibrations (coupables via 🔊), écran maintenu allumé pendant la partie, animations
désactivées si le système demande de réduire les animations.

## Architecture

```
crates/youliar-core/     Rust pur, testé, sans interface
  audio.rs     VAD, latence, hauteur (YIN), jitter, shimmer, pauses, débit, MFCC
  video.rs     pouls par rPPG (méthode CHROM + FFT), agitation
  model.rs     ligne de base par joueur, score logistique, apprentissage, identification du locuteur
  game.rs      machine à états de la partie, droits par appareil, vues filtrées (secrets)
  protocol.rs  messages réseau (trames bornées sur TCP, balise UDP)
  secure.rs    SPAKE2 + ChaCha20-Poly1305
src-tauri/               application Tauri : commandes, profils (JSON), sessions réseau (tokio)
ui/                      interface HTML/CSS/JS sans framework : capture micro/caméra, écrans
```

L'hôte détient l'unique état de partie qui fait foi. Une partie locale est un hôte sans
réseau : le même code sert pour les deux modes.

Ports : TCP 47820 (partie), UDP 47821 (découverte). Sur mobile, la caméra n'analyse que
5 images/s hors des mesures (30 images/s pendant la réponse du joueur) et s'arrête quand
l'application passe en arrière-plan.

## Pistes

* Bluetooth LE (via `btleplug` sur le bureau, plugin natif sur mobile) pour l'appairage sans réseau.
* QR code contenant IP + port + code.
* Détection du visage (MediaPipe) pour suivre le visage au lieu d'une zone ovale fixe, et mesurer les clignements.
* Traductions.
