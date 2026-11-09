//! Histoires : chaque thème habille la partie d'un récit (prologue, chapitres qui
//! avancent au fil des manches, questions à jouer « dans le rôle », vocabulaire,
//! réactions aux verdicts, grades et épilogue).
//!
//! Les textes sont statiques et identiques sur tous les appareils : seul l'identifiant
//! du thème circule sur le réseau. Les marqueurs `{subject}`, `{asker}` et `{winner}`
//! sont remplacés par les prénoms à l'affichage.

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Chapter {
    pub title: &'static str,
    pub text: &'static str,
}

#[derive(Debug, Serialize)]
pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub emoji: &'static str,
    pub tagline: &'static str,
    pub prologue: &'static str,
    /// Récit découpé en étapes, réparties sur l'ensemble des manches.
    pub chapters: &'static [Chapter],
    /// Questions à jouer dans le rôle.
    pub questions: &'static [&'static str],
    /// Mise en scène d'une manche : `{subject}` et `{asker}`.
    pub summon: &'static str,
    /// Mise en scène sans interrogateur (solo, mode libre) : `{subject}`.
    pub summon_solo: &'static str,
    pub subject_role: &'static str,
    pub asker_role: &'static str,
    /// Nom de la machine dans l'histoire.
    pub machine: &'static str,
    pub lie_order: &'static str,
    pub truth_order: &'static str,
    pub liar_caught: &'static str,
    pub liar_escaped: &'static str,
    pub honest_cleared: &'static str,
    pub honest_accused: &'static str,
    /// Grades, du plus bas au plus haut, attribués selon le classement final.
    pub ranks: &'static [&'static str],
    /// Conclusion : `{winner}`.
    pub epilogue: &'static str,
}

pub const CLASSIC: &str = "classic";

pub fn themes() -> &'static [Theme] {
    THEMES
}

pub fn theme(id: &str) -> Option<&'static Theme> {
    THEMES.iter().find(|t| t.id == id)
}

/// Chapitre de la manche `round` (1-based) sur `total` : le récit est étalé sur toute la
/// partie, son dénouement tombe sur la dernière manche.
pub fn chapter_index(theme: &Theme, round: u32, total: u32) -> Option<usize> {
    if round == 0 || theme.chapters.is_empty() {
        return None;
    }
    let n = theme.chapters.len();
    let total = total.max(round) as usize;
    Some(((round as usize - 1) * n / total).min(n - 1))
}

static THEMES: &[Theme] = &[
    Theme {
        id: "kgb",
        name: "Agent du KGB",
        emoji: "🪆",
        tagline: "Moscou, 1962. Une taupe s'est glissée dans votre cellule.",
        prologue: "Moscou, hiver 1962. Le Centre a intercepté un message chiffré : l'un d'entre vous renseigne les Américains. \
                   Personne ne quittera la Loubianka avant que la taupe soit démasquée. Camarades, le polygraphe vous attend.",
        chapters: &[
            Chapter { title: "Le message intercepté", text: "Un télégramme de Washington mentionne un nom de code : « Rossignol ». Le Centre est formel, Rossignol est dans cette pièce." },
            Chapter { title: "La boîte aux lettres morte", text: "Au parc Gorki, sous le troisième banc, une cachette a été vidée cette nuit. Quelqu'un ici connaissait l'emplacement." },
            Chapter { title: "Le bal de l'ambassade", text: "Hier soir, à l'ambassade de France, un agent de la cellule a dansé trop longtemps avec l'attaché américain. Les photographes du KGB ont tout vu." },
            Chapter { title: "Berlin, Checkpoint Charlie", text: "Un microfilm a franchi le mur caché dans une boîte de cigares. Les douaniers parlent d'un passeur au sourire nerveux." },
            Chapter { title: "Les caves de la Loubianka", text: "Le colonel perd patience. Les interrogatoires se durcissent, et chaque hésitation sera consignée dans votre dossier." },
            Chapter { title: "L'aube du verdict", text: "Le Centre s'apprête à trancher. Avant le lever du jour, quelqu'un sera décoré… et quelqu'un partira pour la Sibérie." },
        ],
        questions: &[
            "As-tu déjà rencontré un diplomate américain en dehors du service ?",
            "Où étais-tu la nuit où les plans du missile ont disparu ?",
            "Parles-tu anglais mieux que tu ne le prétends ?",
            "As-tu déjà utilisé la boîte aux lettres morte du parc Gorki ?",
            "Écoutes-tu Radio Free Europe en cachette ?",
            "Que transportais-tu dans ta valise diplomatique la semaine dernière ?",
            "As-tu déjà reçu des dollars en échange d'un service ?",
            "Ton oncle à Leningrad existe-t-il vraiment ?",
            "As-tu déjà photographié un document classé secret ?",
            "Pourquoi as-tu été vu au Bolchoï avec un inconnu ?",
            "Connais-tu le nom de code « Rossignol » ?",
            "As-tu déjà mis un micro dans le bureau d'un camarade ?",
            "Ton dernier voyage à Vienne était-il vraiment pour le travail ?",
            "As-tu déjà brûlé un message après l'avoir lu ?",
            "Possèdes-tu un passeport sous un autre nom ?",
            "Fais-tu entièrement confiance au camarade assis à ta gauche ?",
            "As-tu déjà mangé du caviar payé par l'ennemi ?",
            "Ton chat s'appelle-t-il vraiment Boris ?",
        ],
        summon: "{subject} est convoqué(e) en salle d'interrogatoire. Le camarade {asker} mène l'interrogatoire.",
        summon_solo: "{subject} est convoqué(e) seul(e) face au polygraphe du Centre.",
        subject_role: "suspect(e)",
        asker_role: "officier traitant",
        machine: "le polygraphe du Centre",
        lie_order: "TIENS TA LÉGENDE",
        truth_order: "DIS LA VÉRITÉ AU CENTRE",
        liar_caught: "La taupe est démasquée ! Le dossier part au colonel.",
        liar_escaped: "Légende tenue : le polygraphe du Centre n'y a vu que du feu.",
        honest_cleared: "Loyauté confirmée. Le Parti vous remercie, camarade.",
        honest_accused: "Le Centre soupçonne un innocent… la paranoïa gagne la cellule.",
        ranks: &["Recrue", "Lieutenant", "Capitaine", "Commandant", "Colonel", "Héros de l'Union soviétique"],
        epilogue: "Au petit matin, {winner} reçoit l'ordre de Lénine sous les applaudissements du Centre. Quant à Rossignol… certains disent qu'il chante encore.",
    },
    Theme {
        id: "cia",
        name: "Espion de la CIA",
        emoji: "🕶️",
        tagline: "Langley, 1983. Un agent double vend vos secrets à Moscou.",
        prologue: "Langley, Virginie, 1983. Trois de nos agents ont été arrêtés à Moscou en une semaine. Quelqu'un dans cette équipe travaille pour le KGB. \
                   Le directeur a ordonné un passage au détecteur pour tout le monde. Sans exception.",
        chapters: &[
            Chapter { title: "Langley, sous-sol B", text: "Les dossiers des agents arrêtés ont tous été consultés depuis ce bureau. La liste des accès tient sur une seule page." },
            Chapter { title: "Opération Hibou de nuit", text: "Une mission secrète à Berlin-Est a échoué au dernier moment. L'ennemi connaissait l'heure exacte du rendez-vous." },
            Chapter { title: "Vienne, le café Central", text: "Un agent de l'équipe a été photographié avec un homme en manteau gris. Il prétend avoir parlé de la météo." },
            Chapter { title: "Le transfuge", text: "Un colonel soviétique propose de passer à l'Ouest. Il connaît le nom de la taupe… et il ne le donnera qu'à la fin." },
            Chapter { title: "Le pont des espions", text: "Un échange de prisonniers se prépare sur le pont de Glienicke. Si la taupe parle avant, l'échange tourne au piège." },
            Chapter { title: "Le rapport final", text: "Le directeur attend votre rapport demain matin. Ce soir, chaque réponse peut briser une carrière." },
        ],
        questions: &[
            "As-tu déjà vendu une information, même anodine ?",
            "Pourquoi ton compte en banque a-t-il grossi cet été ?",
            "As-tu déjà rencontré un agent du KGB sans le signaler ?",
            "Où étais-tu pendant l'opération Hibou de nuit ?",
            "Parles-tu russe ?",
            "As-tu déjà emporté un dossier classifié chez toi ?",
            "Ton voyage à Vienne était-il vraiment des vacances ?",
            "As-tu déjà menti au détecteur lors de ton recrutement ?",
            "Qui t'a appris à crocheter une serrure ?",
            "As-tu déjà utilisé une fausse identité en dehors d'une mission ?",
            "As-tu une boîte aux lettres morte que l'Agence ne connaît pas ?",
            "Quelqu'un dans cette pièce te fait-il chanter ?",
            "As-tu déjà effacé une bande d'écoute ?",
            "Ton stylo est-il vraiment un simple stylo ?",
            "As-tu déjà trahi un collègue pour avancer ?",
            "Combien de passeports possèdes-tu ?",
            "Que fais-tu vraiment le jeudi soir ?",
            "As-tu déjà joué aux échecs avec un inconnu à Moscou ?",
        ],
        summon: "Agent {subject}, passez au détecteur. L'officier {asker} conduit l'entretien.",
        summon_solo: "Agent {subject}, passez au détecteur de Langley.",
        subject_role: "agent sous enquête",
        asker_role: "officier enquêteur",
        machine: "le détecteur de Langley",
        lie_order: "PROTÈGE TA COUVERTURE",
        truth_order: "JOUE CARTES SUR TABLE",
        liar_caught: "Couverture grillée ! La sécurité intérieure vous attend.",
        liar_escaped: "Sang-froid d'agent double : le détecteur de Langley n'a rien vu.",
        honest_cleared: "Habilitation confirmée. Retour au service actif.",
        honest_accused: "Fausse alerte : on soupçonne un agent loyal. Le doute ronge l'équipe.",
        ranks: &["Stagiaire", "Analyste", "Agent de terrain", "Chef de poste", "Directeur adjoint", "Légende de Langley"],
        epilogue: "Le rapport est signé : {winner} est promu(e) et reçoit l'étoile de l'Agence. Le transfuge, lui, a disparu avec son secret.",
    },
    Theme {
        id: "actor",
        name: "Star de cinéma",
        emoji: "🎬",
        tagline: "Hollywood. Le rôle de votre vie se joue ce soir.",
        prologue: "Hollywood, la veille du grand casting. Un réalisateur légendaire cherche son premier rôle, et il ne croit qu'une chose : un grand acteur sait mentir les yeux dans les yeux. \
                   Sa directrice de casting a apporté un drôle d'appareil. Moteur… action !",
        chapters: &[
            Chapter { title: "Le casting", text: "Une file d'attente interminable, des CV embellis et des photos retouchées. Ici, tout le monde a « déjà tourné avec Spielberg »." },
            Chapter { title: "Les essais caméra", text: "Le réalisateur veut des émotions vraies. Ou du moins, qui en ont l'air." },
            Chapter { title: "Premier jour de tournage", text: "Le plateau est en ébullition. Une rumeur circule : quelqu'un aurait menti sur son expérience." },
            Chapter { title: "Le scandale", text: "Les tabloïds publient des photos compromettantes. Les attachés de presse démentent tout… un peu trop vite." },
            Chapter { title: "Le festival de Cannes", text: "Tapis rouge, flashes et interviews pièges. Chaque réponse sera dans les journaux demain." },
            Chapter { title: "La cérémonie", text: "L'enveloppe est entre les mains de la présentatrice. Plus qu'un mensonge à tenir, ou une vérité à assumer." },
        ],
        questions: &[
            "As-tu déjà oublié ton texte en pleine scène ?",
            "As-tu vraiment fait toutes tes cascades toi-même ?",
            "As-tu déjà pleuré sur commande pour obtenir quelque chose ?",
            "Ton premier rôle était-il vraiment un arbre dans le spectacle de l'école ?",
            "As-tu déjà menti sur ton âge à un casting ?",
            "As-tu déjà eu une liaison avec un partenaire de tournage ?",
            "Sais-tu vraiment monter à cheval, comme indiqué sur ton CV ?",
            "As-tu déjà improvisé une réplique restée dans le film ?",
            "As-tu déjà fait semblant d'aimer un film d'un ami ?",
            "As-tu déjà payé quelqu'un pour applaudir à ta première ?",
            "As-tu déjà dormi dans ta loge ?",
            "Ton accent anglais est-il naturel ?",
            "As-tu déjà refusé un rôle par jalousie envers le casting ?",
            "As-tu déjà répété un discours de remerciement devant ton miroir ?",
            "As-tu déjà inventé une anecdote de tournage pour une interview ?",
            "Connais-tu vraiment le réalisateur que tu cites tout le temps ?",
            "As-tu déjà simulé un malaise pour arrêter une prise ?",
            "Tes larmes à la dernière cérémonie étaient-elles sincères ?",
        ],
        summon: "{subject} entre dans la lumière des projecteurs. {asker} dirige l'audition.",
        summon_solo: "{subject} entre seul(e) dans la lumière des projecteurs.",
        subject_role: "à l'audition",
        asker_role: "directeur·rice de casting",
        machine: "le détecteur de la directrice de casting",
        lie_order: "JOUE LA COMÉDIE",
        truth_order: "SOIS SINCÈRE",
        liar_caught: "Coupez ! Le jeu sonnait faux, la directrice de casting n'y a pas cru.",
        liar_escaped: "Performance digne d'un Oscar : même le détecteur a été ému.",
        honest_cleared: "Authentique. Le réalisateur adore les vraies émotions.",
        honest_accused: "Trop sincère pour être vrai… le plateau ne vous croit pas.",
        ranks: &["Figurant·e", "Doublure", "Second rôle", "Tête d'affiche", "Star", "Légende d'Hollywood"],
        epilogue: "Et la statuette est attribuée à… {winner} ! Sous les flashes, la star promet que son discours n'était pas répété. Personne ne la croit.",
    },
    Theme {
        id: "manor",
        name: "Meurtre au manoir",
        emoji: "🔎",
        tagline: "Lord Blackwood est mort. Vous étiez tous au manoir ce soir-là.",
        prologue: "Manoir de Blackwood, une nuit d'orage. Le maître des lieux a été retrouvé sans vie dans la bibliothèque, la porte fermée de l'intérieur. \
                   Aucun invité n'a pu quitter le domaine. L'inspecteur a installé son étrange appareil dans le grand salon.",
        chapters: &[
            Chapter { title: "Un cri dans la nuit", text: "À vingt-deux heures, un cri a traversé le manoir. Chacun prétend avoir été ailleurs." },
            Chapter { title: "La bibliothèque", text: "Un verre de porto à moitié vide, une fenêtre entrouverte et des traces de boue sur le tapis persan." },
            Chapter { title: "Le testament", text: "Le notaire révèle que Lord Blackwood comptait changer son testament… dès le lendemain matin." },
            Chapter { title: "Le poison", text: "Le médecin est formel : de l'arsenic. Quelqu'un ici savait où le jardinier rangeait ses produits." },
            Chapter { title: "La lettre anonyme", text: "Une lettre glissée sous la porte de l'inspecteur accuse l'un d'entre vous. Elle est écrite à l'encre verte." },
            Chapter { title: "Le salon, à minuit", text: "L'inspecteur réunit tout le monde au salon. Comme dans tout bon roman, le coupable est dans la pièce." },
        ],
        questions: &[
            "Où étiez-vous à vingt-deux heures hier soir ?",
            "Aviez-vous une raison d'en vouloir à Lord Blackwood ?",
            "Êtes-vous entré(e) dans la bibliothèque après le dîner ?",
            "Saviez-vous que vous figuriez dans le testament ?",
            "Connaissiez-vous l'existence du passage secret ?",
            "Avez-vous touché au carafon de porto ?",
            "Pourquoi vos chaussures étaient-elles couvertes de boue ?",
            "Avez-vous déjà acheté de l'arsenic « pour les rats » ?",
            "Entreteniez-vous une liaison avec quelqu'un du manoir ?",
            "Avez-vous entendu le cri, ou l'avez-vous poussé ?",
            "Écrivez-vous à l'encre verte ?",
            "Avez-vous des dettes de jeu ?",
            "Avez-vous vu le majordome après minuit ?",
            "Pourquoi avez-vous changé de tenue pendant la soirée ?",
            "Avez-vous déjà menacé Lord Blackwood ?",
            "La clé de la bibliothèque était-elle dans votre poche ?",
            "Avez-vous un alibi que quelqu'un peut confirmer ?",
            "Êtes-vous vraiment arrivé(e) au manoir en retard à cause de l'orage ?",
        ],
        summon: "L'inspecteur appelle {subject} au salon. {asker} pose les questions.",
        summon_solo: "L'inspecteur appelle {subject} au salon.",
        subject_role: "suspect(e)",
        asker_role: "enquêteur·rice",
        machine: "l'appareil de l'inspecteur",
        lie_order: "PROTÈGE TON SECRET",
        truth_order: "DIS CE QUE TU SAIS",
        liar_caught: "Contradiction ! L'inspecteur note quelque chose dans son carnet…",
        liar_escaped: "Alibi en béton : l'appareil de l'inspecteur reste muet.",
        honest_cleared: "Témoignage crédible. Un suspect de moins.",
        honest_accused: "Les apparences sont contre vous… et pourtant vous disiez vrai.",
        ranks: &["Suspect n°1", "Témoin douteux", "Témoin", "Alibi solide", "Au-dessus de tout soupçon", "Fin limier"],
        epilogue: "L'affaire Blackwood est close. {winner} ressort du manoir libre, et peut-être un peu plus riche. L'inspecteur, lui, n'a jamais retrouvé la clé de la bibliothèque.",
    },
    Theme {
        id: "pirate",
        name: "Mutinerie à bord",
        emoji: "🏴‍☠️",
        tagline: "Le coffre du capitaine est vide. L'équipage est à bord, la côte est loin.",
        prologue: "Mer des Caraïbes, 1715. Ce matin, le coffre du capitaine était vide. Pas une pièce d'or, pas un bijou. \
                   Personne n'a quitté le navire. Le capitaine a sorti sa vieille boussole enchantée : elle tremble, dit-on, face aux menteurs.",
        chapters: &[
            Chapter { title: "Le coffre vide", text: "La serrure n'a pas été forcée. Seuls trois membres d'équipage connaissaient la cachette de la clé." },
            Chapter { title: "La carte déchirée", text: "La carte du trésor a été coupée en deux. Une moitié manque, et quelqu'un dort dessus." },
            Chapter { title: "La tempête", text: "Pendant l'orage, quelqu'un a été vu près de la cale avec une lanterne. Les marins chuchotent." },
            Chapter { title: "L'île aux perroquets", text: "Une barque a touché terre cette nuit sans ordre du capitaine. Le sable garde des traces de pas." },
            Chapter { title: "La planche", text: "Le capitaine fait sortir la planche. Le prochain menteur démasqué ira nourrir les requins." },
            Chapter { title: "Le partage du butin", text: "Il est temps de partager l'or retrouvé. Mais le pirate le plus rusé ne dit jamais tout." },
        ],
        questions: &[
            "As-tu touché au coffre du capitaine ?",
            "Où étais-tu pendant la tempête ?",
            "As-tu déjà servi sur un navire de la Couronne ?",
            "As-tu caché de l'or dans ta botte ?",
            "Sais-tu vraiment nager ?",
            "As-tu déjà triché aux dés avec l'équipage ?",
            "Pourquoi ton perroquet répète-t-il « trésor » ?",
            "As-tu déjà vendu un compagnon pour une bouteille de rhum ?",
            "As-tu pris la barque cette nuit ?",
            "Ta jambe de bois est-elle vraiment nécessaire ?",
            "As-tu déjà vu la deuxième moitié de la carte ?",
            "As-tu bu la réserve de rhum du capitaine ?",
            "As-tu déjà eu peur d'un fantôme en mer ?",
            "As-tu promis la même part du butin à deux personnes ?",
            "Sais-tu lire la carte, ou fais-tu semblant ?",
            "As-tu un trésor enterré sur une autre île ?",
            "Ton cache-œil cache-t-il vraiment un œil ?",
            "Prépares-tu une mutinerie ?",
        ],
        summon: "{subject} est traîné(e) devant le grand mât. {asker} brandit la boussole.",
        summon_solo: "{subject} est traîné(e) devant le grand mât.",
        subject_role: "accusé(e)",
        asker_role: "quartier-maître",
        machine: "la boussole enchantée",
        lie_order: "GARDE TON SECRET DE PIRATE",
        truth_order: "PAROLE DE PIRATE",
        liar_caught: "La boussole s'affole ! Préparez la planche.",
        liar_escaped: "Menteur de haute mer : la boussole enchantée n'a pas bougé.",
        honest_cleared: "Parole de pirate tenue. Un tonneau de rhum pour l'honnête marin !",
        honest_accused: "La boussole s'est trompée… l'équipage se méfie d'un innocent.",
        ranks: &["Mousse", "Matelot", "Canonnier", "Maître d'équipage", "Second", "Capitaine"],
        epilogue: "Au couchant, l'équipage hisse {winner} au rang de capitaine. On murmure que le coffre n'a jamais été vraiment vide…",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_is_complete() {
        for t in themes() {
            assert!(t.questions.len() >= 15, "{}: questions", t.id);
            assert!(t.chapters.len() >= 4, "{}: chapitres", t.id);
            assert_eq!(t.ranks.len(), 6, "{}: grades", t.id);
            assert!(t.summon.contains("{subject}") && t.summon.contains("{asker}"), "{}", t.id);
            assert!(t.summon_solo.contains("{subject}") && !t.summon_solo.contains("{asker}"), "{}", t.id);
            assert!(t.epilogue.contains("{winner}"), "{}", t.id);
            assert!(t.questions.iter().all(|q| q.chars().count() <= 200), "{}", t.id);
            assert_ne!(t.id, CLASSIC);
        }
    }

    #[test]
    fn chapters_spread_over_the_whole_game() {
        let t = theme("kgb").unwrap();
        let n = t.chapters.len();
        assert_eq!(chapter_index(t, 1, 12), Some(0));
        assert_eq!(chapter_index(t, 12, 12), Some(n - 1), "le dénouement tombe à la dernière manche");
        let seen: Vec<_> = (1..=12).filter_map(|r| chapter_index(t, r, 12)).collect();
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "le récit avance");
        // Partie courte : on avance plus vite, sans dépasser le dernier chapitre.
        assert_eq!(chapter_index(t, 2, 2), Some(n / 2));
        assert_eq!(chapter_index(t, 0, 4), None);
    }
}
