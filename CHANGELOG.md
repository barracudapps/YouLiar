# Journal des versions

Format inspiré de [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/), versions
[SemVer](https://semver.org/lang/fr/) (voir `CONTRIBUTING.md`).

Les versions 0.1.0 à 0.3.0 sont des étapes de développement antérieures à l'historique git,
qui commence à 0.4.0. Elles sont décrites ici pour garder la trace des choix, sans tag.

## [0.4.0] — 2026-11-09

### Ajouté
- **Mode libre** : salle d'interrogatoire où chacun choisit un joueur et lui pose la question
  qu'il veut, depuis n'importe quel appareil ; verdict immédiat ; aveu facultatif qui entraîne
  la machine ; on peut rejoindre la salle en cours ; récapitulatif de toutes les questions.
- Choix du mode (🎮 Jeu / 🎤 Libre) dans le salon.

## [0.3.0] — 2026-11-09

### Ajouté
- Interrogateur : le joueur suivant choisit parmi 3 questions ou écrit la sienne.
- Polygraphe en direct sur tous les appareils (niveau vocal uniquement, jamais l'audio).
- Paris « sûr de moi » (+2 / −1), bonus « question piège », paquet de questions « Mélange ».
- Fin de partie : podium, trophées, récapitulatif des manches.
- Révélation en trois temps, sons synthétisés, vibrations, écran maintenu allumé.
- Carte d'identité vocale et bilan par joueur, écran « Comment jouer ».

## [0.2.0] — 2026-11-09

### Sécurité
- Échange SPAKE2 authentifié par un code à 6 chiffres puis chiffrement ChaCha20-Poly1305,
  une clé par connexion (protocole réseau v2, incompatible avec v1).
- Limitation des tentatives de code, des connexions, de la taille des trames et du débit.
- Validation de toutes les données reçues d'un pair ou de l'interface ; profils en 0600.

### Performance
- Analyse de la voix 4× plus rapide (hauteur estimée à 8 kHz).
- Capture audio en AudioWorklet, commandes lourdes hors du fil principal.

### Ajouté
- Reconnexion automatique avec reprise de sa place par jeton secret.

## [0.1.0] — 2026-11-09

### Ajouté
- Calibrage par joueur (voix, ton, temps de réponse, caméra facultative).
- Analyse vocale (latence, hauteur, jitter, shimmer, pauses, débit, MFCC) et caméra (rPPG,
  agitation) ; modèle logistique par joueur qui apprend de chaque manche.
- Partie sur un seul appareil avec attribution de la voix, ou en réseau local.
