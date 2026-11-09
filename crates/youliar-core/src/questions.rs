//! Banques de questions et protocole de calibrage.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Pack {
    #[default]
    Soft,
    Party,
    Deep,
    /// Toutes les questions mélangées.
    Mix,
}

/// Questions d'un paquet (le mélange réunit tous les paquets).
pub fn questions_for(pack: Pack) -> Vec<&'static str> {
    match pack {
        Pack::Mix => [Pack::Soft, Pack::Party, Pack::Deep].iter().flat_map(|p| questions(*p).iter().copied()).collect(),
        p => questions(p).to_vec(),
    }
}

pub fn questions(pack: Pack) -> &'static [&'static str] {
    match pack {
        Pack::Mix => &[],
        Pack::Soft => &[
            "As-tu déjà fait semblant de connaître une chanson ?",
            "As-tu déjà mangé un dessert avant le plat principal ?",
            "As-tu déjà menti sur ton nombre de pas ou de kilomètres courus ?",
            "As-tu déjà fait une sieste de plus de trois heures ?",
            "Sais-tu faire la roue ?",
            "As-tu déjà perdu tes clés deux fois dans la même journée ?",
            "As-tu déjà parlé à une plante ?",
            "As-tu déjà fait semblant d'aimer un cadeau ?",
            "As-tu déjà mangé quelque chose tombé par terre ?",
            "As-tu déjà chanté sous la douche cette semaine ?",
            "As-tu déjà oublié l'anniversaire d'un proche ?",
            "Sais-tu nager le crawl ?",
            "As-tu déjà pleuré devant un dessin animé ?",
            "As-tu déjà menti sur ton âge ?",
            "As-tu déjà raté un train ou un avion ?",
            "As-tu déjà parlé tout seul dans la rue ?",
            "As-tu déjà fini un pot de glace en une soirée ?",
            "Te brosses-tu toujours les dents deux fois par jour ?",
            "As-tu déjà dormi plus de 14 heures d'affilée ?",
            "Connais-tu par cœur les paroles d'une chanson ringarde ?",
            "As-tu déjà fait un vœu en soufflant une bougie qui s'est réalisé ?",
            "As-tu déjà eu peur du noir après tes 15 ans ?",
            "Quel est le dernier film que tu as vu ?",
            "Quel est ton plat préféré ?",
            "As-tu déjà gagné un concours ?",
        ],
        Pack::Party => &[
            "As-tu déjà liké une vieille photo par erreur en stalkant ?",
            "As-tu déjà menti sur ton score à un jeu vidéo ?",
            "As-tu déjà fait semblant d'avoir du réseau pour ignorer un appel ?",
            "As-tu déjà inventé un rendez-vous pour quitter une soirée ?",
            "As-tu déjà lu les messages de quelqu'un par-dessus son épaule ?",
            "As-tu déjà donné un faux numéro de téléphone ?",
            "As-tu déjà pris le dernier morceau en faisant semblant de ne pas l'avoir vu ?",
            "As-tu déjà espionné le téléphone de quelqu'un ?",
            "As-tu déjà inventé une excuse pour éviter une soirée ?",
            "As-tu déjà eu un crush sur quelqu'un dans cette pièce ?",
            "As-tu déjà regifté un cadeau ?",
            "As-tu déjà triché à un jeu de société ?",
            "As-tu déjà fait semblant d'être malade pour ne pas travailler ?",
            "As-tu déjà envoyé un message à la mauvaise personne ?",
            "As-tu déjà stalké un ex sur les réseaux sociaux ?",
            "As-tu déjà mangé la nourriture de quelqu'un dans le frigo ?",
            "As-tu déjà dit « je suis en route » alors que tu étais encore chez toi ?",
            "As-tu déjà menti dans ce jeu ?",
            "As-tu déjà fait croire que tu avais lu un livre ?",
            "As-tu déjà oublié le prénom de quelqu'un en pleine conversation ?",
            "As-tu déjà accusé quelqu'un d'autre d'une bêtise que tu avais faite ?",
            "As-tu déjà dansé seul devant un miroir cette année ?",
            "Qui, ici, te ferait le plus confiance pour garder un secret ?",
        ],
        Pack::Deep => &[
            "As-tu déjà menti pour protéger quelqu'un ?",
            "Y a-t-il quelqu'un que tu admires en secret ?",
            "As-tu déjà dit « je t'aime » sans le penser ?",
            "As-tu déjà pleuré en cachette cette année ?",
            "Y a-t-il une décision que tu regrettes vraiment ?",
            "As-tu déjà menti à tes parents sur quelque chose d'important ?",
            "As-tu déjà été jaloux d'un ami proche ?",
            "Y a-t-il quelqu'un à qui tu dois des excuses ?",
            "As-tu déjà gardé un secret qui ne t'appartenait pas ?",
            "Es-tu vraiment satisfait de ta vie en ce moment ?",
            "As-tu déjà fait semblant d'aller bien ?",
            "As-tu déjà trahi la confiance de quelqu'un ?",
            "Penses-tu être une personne honnête ?",
            "As-tu déjà eu peur de décevoir quelqu'un ici ?",
            "Quelle est la chose dont tu es le plus fier ?",
            "Y a-t-il un rêve que tu as abandonné ?",
        ],
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CalibKind {
    /// Lecture à voix haute : empreinte vocale + ligne de base.
    Voice,
    /// Parole neutre (comptage…) : ligne de base.
    Neutral,
    /// Réponse sincère imposée.
    Truth,
    /// Mensonge imposé.
    Lie,
}

#[derive(Debug, Clone, Serialize)]
pub struct CalibStep {
    pub kind: CalibKind,
    pub instruction: &'static str,
    pub prompt: &'static str,
}

/// Protocole de calibrage : alterne vérités et mensonges imposés pour que la
/// machine apprenne comment *ce* joueur réagit.
pub fn calibration_steps() -> Vec<CalibStep> {
    use CalibKind::*;
    vec![
        CalibStep { kind: Voice, instruction: "Lis cette phrase à voix haute, naturellement.", prompt: "Le soleil se lève à l'est et se couche à l'ouest, comme chaque jour." },
        CalibStep { kind: Voice, instruction: "Lis cette phrase à voix haute.", prompt: "Mon voisin promène son chien tous les matins avant le petit-déjeuner." },
        CalibStep { kind: Neutral, instruction: "Compte calmement de un à huit.", prompt: "1, 2, 3, 4, 5, 6, 7, 8" },
        CalibStep { kind: Truth, instruction: "Réponds en disant LA VÉRITÉ.", prompt: "Quel est ton prénom ?" },
        CalibStep { kind: Lie, instruction: "Réponds par un MENSONGE crédible.", prompt: "Dans quelle ville es-tu né ?" },
        CalibStep { kind: Truth, instruction: "Réponds en disant LA VÉRITÉ.", prompt: "Qu'as-tu mangé à ton dernier repas ?" },
        CalibStep { kind: Lie, instruction: "Réponds par un MENSONGE crédible.", prompt: "Quel âge as-tu ?" },
        CalibStep { kind: Truth, instruction: "Réponds en disant LA VÉRITÉ.", prompt: "De quelle couleur est le haut que tu portes ?" },
        CalibStep { kind: Lie, instruction: "Réponds par un MENSONGE crédible.", prompt: "As-tu des frères ou des sœurs ? Lesquels ?" },
        CalibStep { kind: Truth, instruction: "Réponds en disant LA VÉRITÉ.", prompt: "Quel temps fait-il dehors aujourd'hui ?" },
        CalibStep { kind: Lie, instruction: "Réponds par un MENSONGE crédible.", prompt: "Quel est ton métier ou tes études ?" },
    ]
}
