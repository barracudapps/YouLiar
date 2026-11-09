//! Logique de partie, identique en local (un seul appareil) et en réseau : l'hôte
//! détient l'unique `Game` faisant autorité ; chaque appareil (« device ») possède
//! un ou plusieurs joueurs et reçoit une vue filtrée (les secrets restent secrets).
//!
//! Deux modes :
//! * **Jeu** (`GameMode::Game`) : manches structurées, consignes secrètes, votes, points ;
//! * **Libre** (`GameMode::Free`) : salle d'interrogatoire ouverte — n'importe qui choisit
//!   qui interroger et pose la question qu'il veut ; la machine rend son verdict tout de
//!   suite, et le joueur interrogé peut ensuite avouer (ce qui entraîne la machine).
//!
//! Déroulé d'une manche (mode Jeu) :
//! 1. `Asking`    – un joueur est sur la sellette ; le joueur suivant devient
//!    l'**interrogateur** et choisit parmi 3 questions (ou écrit la sienne). Le sujet
//!    reçoit en secret une consigne (MENS / DIS LA VÉRITÉ) ou choisit lui-même.
//! 2. `Answering` – il répond à voix haute ; son appareil analyse voix + caméra en local.
//! 3. `Voting`    – les autres votent « mensonge » ou « vérité ».
//! 4. `Reveal`    – verdict de la machine, vérité, points.

use crate::model::Verdict;
use crate::questions::{questions_for, Pack};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub const HOST_DEVICE: &str = "host";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlayerInfo {
    pub id: String,
    pub name: String,
    pub avatar: String,
    pub calibrated: bool,
}

impl PlayerInfo {
    /// Borne les champs fournis par un appareil (donnée non fiable).
    pub fn sanitized(mut self) -> Result<Self, String> {
        self.id = self.id.trim().chars().take(40).collect();
        self.name = self.name.trim().chars().filter(|c| !c.is_control()).take(24).collect();
        self.avatar = self.avatar.chars().filter(|c| !c.is_control()).take(4).collect();
        if self.id.is_empty() || self.name.is_empty() {
            return Err("Joueur invalide.".into());
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Player {
    pub info: PlayerInfo,
    pub device: String,
    pub score: i32,
    pub connected: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum InstructionMode {
    /// La machine impose secrètement de mentir ou non (50/50).
    #[default]
    Assigned,
    /// Le joueur choisit secrètement.
    Free,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GameMode {
    #[default]
    Game,
    Free,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub game_name: String,
    #[serde(default)]
    pub mode: GameMode,
    pub rounds_per_player: u32,
    pub instruction_mode: InstructionMode,
    pub pack: Pack,
    /// La machine lit la question à voix haute (synthèse vocale).
    pub read_aloud: bool,
    /// Temps de vote en secondes (0 = illimité). Au-delà, la révélation est automatique.
    #[serde(default = "default_vote_seconds")]
    pub vote_seconds: u32,
}

fn default_vote_seconds() -> u32 {
    30
}

/// Nombre maximal de questions mémorisées pour éviter les répétitions d'une soirée à l'autre.
pub const MAX_USED_QUESTIONS: usize = 500;

impl Default for Settings {
    fn default() -> Self {
        Self { game_name: "Partie YouLiar".into(), mode: GameMode::Game, rounds_per_player: 2, instruction_mode: InstructionMode::Assigned, pack: Pack::Soft, read_aloud: true, vote_seconds: default_vote_seconds() }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Lobby,
    /// Mode libre : salle d'interrogatoire, en attente d'une question.
    Free,
    Asking,
    Answering,
    Voting,
    Reveal,
    Finished,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Outcome {
    pub points: BTreeMap<String, i32>,
    pub machine_correct: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Vote {
    pub lie: bool,
    /// Pari « sûr de moi » : +2 si juste, −1 si faux.
    #[serde(default)]
    pub sure: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Round {
    pub number: u32,
    pub subject: String,
    /// Joueur qui choisit la question (absent en solo).
    pub asker: Option<String>,
    /// Questions proposées à l'interrogateur.
    pub choices: Vec<String>,
    pub question: String,
    pub question_locked: bool,
    /// Le sujet ment-il ? (secret jusqu'à la révélation)
    pub lie: Option<bool>,
    pub verdict: Option<Verdict>,
    pub votes: BTreeMap<String, Vote>,
    pub outcome: Option<Outcome>,
}

/// Résumé public d'une manche révélée (sert au récapitulatif et aux trophées).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoundRecord {
    pub number: u32,
    pub subject: String,
    pub asker: Option<String>,
    pub question: String,
    /// Inconnu en mode libre tant que le joueur n'a pas avoué.
    pub lie: Option<bool>,
    pub index: u8,
    pub machine_correct: Option<bool>,
    pub heart_rate: Option<f32>,
    pub votes: BTreeMap<String, Vote>,
    pub points: BTreeMap<String, i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MachineStats {
    pub correct: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    AddPlayers { players: Vec<PlayerInfo> },
    RemovePlayer { player_id: String },
    UpdateSettings { settings: Settings },
    Start,
    NewQuestion,
    CustomQuestion { text: String },
    ChooseQuestion { index: usize },
    /// Mode libre : poser une question à n'importe quel joueur.
    Ask { subject: String, question: String, asker: Option<String> },
    /// Mode libre : le joueur interrogé révèle s'il mentait.
    Confess { lie: bool },
    Declare { lie: bool },
    StartAnswer,
    CancelAnswer,
    SubmitVerdict { verdict: Verdict },
    Vote {
        voter: String,
        lie: bool,
        #[serde(default)]
        sure: bool,
    },
    Reveal,
    NextRound,
    EndGame,
    BackToLobby,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlayerView {
    #[serde(flatten)]
    pub info: PlayerInfo,
    pub score: i32,
    pub connected: bool,
    /// Ce joueur est contrôlé par l'appareil qui reçoit la vue.
    pub mine: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoundView {
    pub number: u32,
    pub subject: String,
    pub asker: Option<String>,
    pub choices: Vec<String>,
    pub question: String,
    pub question_locked: bool,
    /// Visible par l'appareil du sujet, et par tous à la révélation.
    pub lie: Option<bool>,
    pub verdict: Option<Verdict>,
    pub voted: Vec<String>,
    pub votes: Option<BTreeMap<String, Vote>>,
    pub outcome: Option<Outcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GameView {
    pub device: String,
    pub is_host: bool,
    pub settings: Settings,
    pub stage: Stage,
    pub players: Vec<PlayerView>,
    pub round: Option<RoundView>,
    pub round_no: u32,
    pub total_rounds: u32,
    pub machine: MachineStats,
    pub history: Vec<RoundRecord>,
    /// Mode libre : idées de questions proposées dans la salle d'interrogatoire.
    pub suggestions: Vec<String>,
    /// Temps de vote restant au moment où la vue est produite.
    #[serde(default)]
    pub vote_remaining_ms: Option<u64>,
}

pub struct Game {
    pub settings: Settings,
    pub players: Vec<Player>,
    pub stage: Stage,
    pub round: Option<Round>,
    pub round_no: u32,
    pub machine: MachineStats,
    pub history: Vec<RoundRecord>,
    suggestions: Vec<String>,
    vote_deadline: Option<Instant>,
    used: Vec<String>,
    rng: StdRng,
}

impl Default for Game {
    fn default() -> Self {
        Self::new(StdRng::from_entropy())
    }
}

type R = Result<(), String>;

fn err<T>(msg: &str) -> Result<T, String> {
    Err(msg.to_string())
}

impl Game {
    pub fn new(rng: StdRng) -> Self {
        Self { settings: Settings::default(), players: vec![], stage: Stage::Lobby, round: None, round_no: 0, machine: MachineStats::default(), history: vec![], suggestions: vec![], vote_deadline: None, used: vec![], rng }
    }

    pub fn total_rounds(&self) -> u32 {
        match self.settings.mode {
            GameMode::Game => self.settings.rounds_per_player * self.players.len() as u32,
            GameMode::Free => 0,
        }
    }

    /// Questions déjà posées (persistées par l'hôte d'une session à l'autre).
    pub fn used_questions(&self) -> &[String] {
        &self.used
    }

    pub fn set_used_questions(&mut self, mut used: Vec<String>) {
        used.retain(|q| q.chars().count() <= 200);
        let excess = used.len().saturating_sub(MAX_USED_QUESTIONS);
        used.drain(..excess);
        self.used = used;
    }

    /// Fin du vote en cours, pour que l'hôte programme la révélation automatique.
    pub fn vote_deadline(&self) -> Option<(u32, Instant)> {
        if self.stage != Stage::Voting {
            return None;
        }
        Some((self.round.as_ref()?.number, self.vote_deadline?))
    }

    fn free(&self) -> bool {
        self.settings.mode == GameMode::Free
    }

    fn player(&self, id: &str) -> Option<&Player> {
        self.players.iter().find(|p| p.info.id == id)
    }

    fn owns(&self, device: &str, player_id: &str) -> bool {
        self.player(player_id).is_some_and(|p| p.device == device)
    }

    fn subject_device(&self) -> Option<&str> {
        let r = self.round.as_ref()?;
        self.player(&r.subject).map(|p| p.device.as_str())
    }

    fn pick_question(&mut self) -> String {
        let bank = questions_for(self.settings.pack);
        let fresh: Vec<&str> = bank.iter().copied().filter(|q| !self.used.iter().any(|u| u == q)).collect();
        let q = if fresh.is_empty() {
            self.used.clear();
            bank[self.rng.gen_range(0..bank.len())]
        } else {
            fresh[self.rng.gen_range(0..fresh.len())]
        };
        self.used.push(q.to_string());
        if self.used.len() > MAX_USED_QUESTIONS {
            self.used.remove(0);
        }
        q.to_string()
    }

    fn deal_choices(&mut self) -> Vec<String> {
        (0..3).map(|_| self.pick_question()).collect()
    }

    /// L'interrogateur choisit la question ; à défaut (solo, départ), le sujet ou l'hôte.
    fn can_ask(&self, device: &str) -> bool {
        let Some(r) = self.round.as_ref() else { return false };
        device == HOST_DEVICE
            || match &r.asker {
                Some(a) => self.owns(device, a),
                None => self.owns(device, &r.subject),
            }
    }

    fn begin_round(&mut self) {
        if self.free() {
            // Retour à la salle d'interrogatoire.
            self.round = None;
            self.suggestions = self.deal_choices();
            self.stage = Stage::Free;
            return;
        }
        let active: Vec<String> = self.players.iter().filter(|p| p.connected).map(|p| p.info.id.clone()).collect();
        if active.is_empty() || self.round_no >= self.total_rounds() {
            self.stage = Stage::Finished;
            self.round = None;
            return;
        }
        self.round_no += 1;
        let idx = (self.round_no as usize - 1) % active.len();
        let subject = active[idx].clone();
        let asker = (active.len() > 1).then(|| active[(idx + 1) % active.len()].clone());
        let choices = self.deal_choices();
        // En solo, pas d'interrogateur : la première question est retenue d'office.
        let (question, question_locked) = if asker.is_none() { (choices[0].clone(), true) } else { (String::new(), false) };
        let lie = match self.settings.instruction_mode {
            InstructionMode::Assigned => Some(self.rng.gen_bool(0.5)),
            InstructionMode::Free => None,
        };
        self.round = Some(Round {
            number: self.round_no,
            subject,
            asker,
            choices,
            question,
            question_locked,
            lie,
            verdict: None,
            votes: BTreeMap::new(),
            outcome: None,
        });
        self.stage = Stage::Asking;
    }

    fn voters(&self) -> Vec<String> {
        let subject = self.round.as_ref().map(|r| r.subject.as_str()).unwrap_or("");
        self.players.iter().filter(|p| p.connected && p.info.id != subject).map(|p| p.info.id.clone()).collect()
    }

    fn check_votes_complete(&mut self) {
        if self.stage != Stage::Voting {
            return;
        }
        let voters = self.voters();
        let done = self.round.as_ref().is_some_and(|r| voters.iter().all(|v| r.votes.contains_key(v)));
        if done {
            self.reveal();
        }
    }

    fn reveal(&mut self) {
        let Some(r) = self.round.as_mut() else { return };
        let (Some(truth), Some(verdict)) = (r.lie, r.verdict.as_ref()) else { return };
        let mut outcome = Outcome { machine_correct: verdict.is_lie == truth, ..Default::default() };
        let mut add = |id: &str, pts: i32| *outcome.points.entry(id.to_string()).or_default() += pts;
        add(&r.subject, 0);
        for (voter, v) in &r.votes {
            if v.lie == truth {
                add(voter, if v.sure { 2 } else { 1 });
            } else {
                add(&r.subject, 1);
                if v.sure {
                    add(voter, -1);
                }
            }
        }
        if !outcome.machine_correct {
            add(&r.subject, 2);
        }
        // Question piège : l'interrogateur gagne 1 point si la machine démasque un menteur.
        if let Some(a) = &r.asker {
            if truth && outcome.machine_correct {
                add(a, 1);
            }
        }
        self.machine.total += 1;
        if outcome.machine_correct {
            self.machine.correct += 1;
        }
        for p in self.players.iter_mut() {
            p.score += outcome.points.get(&p.info.id).copied().unwrap_or(0);
        }
        self.history.push(RoundRecord {
            number: r.number,
            subject: r.subject.clone(),
            asker: r.asker.clone(),
            question: r.question.clone(),
            lie: Some(truth),
            index: verdict.index,
            machine_correct: Some(outcome.machine_correct),
            heart_rate: verdict.heart_rate,
            votes: r.votes.clone(),
            points: outcome.points.clone(),
        });
        r.outcome = Some(outcome);
        self.stage = Stage::Reveal;
        self.vote_deadline = None;
    }

    pub fn apply(&mut self, device: &str, action: Action) -> R {
        let host = device == HOST_DEVICE;
        let subject_dev = self.subject_device().map(str::to_string);
        let is_subject = subject_dev.as_deref() == Some(device);
        match action {
            Action::AddPlayers { players } => {
                if players.len() > 12 {
                    return err("Trop de joueurs.");
                }
                let open = self.stage == Stage::Lobby || self.free();
                for info in players {
                    let info = info.sanitized()?;
                    match self.players.iter_mut().find(|p| p.info.id == info.id) {
                        Some(p) if p.device == device => {
                            p.info = info;
                            p.connected = true;
                        }
                        Some(_) => return err("Ce profil est déjà utilisé sur un autre appareil."),
                        // En mode libre, on peut rejoindre la salle à tout moment.
                        None if open => {
                            if self.players.len() >= 12 {
                                return err("La partie est complète (12 joueurs).");
                            }
                            self.players.push(Player { info, device: device.into(), score: 0, connected: true });
                        }
                        None => return err("La partie a déjà commencé."),
                    }
                }
            }
            Action::RemovePlayer { player_id } => {
                if !(host || self.owns(device, &player_id)) {
                    return err("Action non autorisée.");
                }
                if self.stage == Stage::Lobby {
                    self.players.retain(|p| p.info.id != player_id);
                } else {
                    if let Some(p) = self.players.iter_mut().find(|p| p.info.id == player_id) {
                        p.connected = false;
                    }
                    self.after_departure();
                }
            }
            Action::UpdateSettings { settings } => {
                if !host || self.stage != Stage::Lobby {
                    return err("Seul l'hôte peut modifier les réglages, dans le salon.");
                }
                let mut s = settings;
                s.rounds_per_player = s.rounds_per_player.clamp(1, 10);
                s.vote_seconds = s.vote_seconds.min(120);
                s.game_name = s.game_name.trim().chars().filter(|c| !c.is_control()).take(40).collect();
                if s.game_name.is_empty() {
                    s.game_name = Settings::default().game_name;
                }
                self.settings = s;
            }
            Action::Start => {
                if !host {
                    return err("Seul l'hôte peut lancer la partie.");
                }
                if self.players.is_empty() {
                    return err("Ajoute au moins un joueur.");
                }
                if let Some(p) = self.players.iter().find(|p| !p.info.calibrated) {
                    return Err(format!("{} doit d'abord être calibré(e).", p.info.name));
                }
                self.round_no = 0;
                self.machine = MachineStats::default();
                self.history.clear();
                self.players.iter_mut().for_each(|p| p.score = 0);
                self.begin_round();
            }
            Action::NewQuestion if self.stage == Stage::Free => {
                self.suggestions = self.deal_choices();
            }
            Action::NewQuestion => {
                if self.stage != Stage::Asking || !self.can_ask(device) {
                    return err("Ce n'est pas à toi de choisir la question.");
                }
                let choices = self.deal_choices();
                let r = self.round.as_mut().unwrap();
                if r.asker.is_none() {
                    r.question = choices[0].clone();
                } else {
                    r.question_locked = false;
                    r.question.clear();
                }
                r.choices = choices;
            }
            Action::CustomQuestion { text } => {
                let text: String = text.trim().chars().filter(|c| !c.is_control()).take(200).collect();
                if self.stage != Stage::Asking || text.is_empty() || !self.can_ask(device) {
                    return err("Question invalide.");
                }
                let r = self.round.as_mut().unwrap();
                r.question = text;
                r.question_locked = true;
            }
            Action::Ask { subject, question, asker } => {
                if !self.free() || !matches!(self.stage, Stage::Free | Stage::Reveal) {
                    return err("On ne peut pas poser de question maintenant.");
                }
                if !self.player(&subject).is_some_and(|p| p.connected) {
                    return err("Ce joueur n'est pas là.");
                }
                if let Some(a) = &asker {
                    if !self.owns(device, a) {
                        return err("Action non autorisée.");
                    }
                }
                let question: String = question.trim().chars().filter(|c| !c.is_control()).take(200).collect();
                if question.is_empty() {
                    return err("Écris une question.");
                }
                self.round_no += 1;
                self.round = Some(Round {
                    number: self.round_no,
                    subject,
                    asker,
                    choices: vec![],
                    question,
                    question_locked: true,
                    lie: None,
                    verdict: None,
                    votes: BTreeMap::new(),
                    outcome: None,
                });
                self.stage = Stage::Asking;
            }
            Action::Confess { lie } => {
                let free = self.free();
                let Some(r) = self.round.as_mut() else { return err("Action non autorisée.") };
                if !free || self.stage != Stage::Reveal || !is_subject || r.lie.is_some() {
                    return err("Action non autorisée.");
                }
                let Some(verdict) = r.verdict.as_ref() else { return err("Action non autorisée.") };
                let correct = verdict.is_lie == lie;
                r.lie = Some(lie);
                r.outcome = Some(Outcome { points: BTreeMap::new(), machine_correct: correct });
                self.machine.total += 1;
                if correct {
                    self.machine.correct += 1;
                }
                if let Some(h) = self.history.iter_mut().rev().find(|h| h.number == r.number) {
                    h.lie = Some(lie);
                    h.machine_correct = Some(correct);
                }
            }
            Action::ChooseQuestion { index } => {
                if self.stage != Stage::Asking || !self.can_ask(device) {
                    return err("Ce n'est pas à toi de choisir la question.");
                }
                let r = self.round.as_mut().unwrap();
                let q = r.choices.get(index).cloned().ok_or("Question inconnue.")?;
                r.question = q;
                r.question_locked = true;
            }
            Action::Declare { lie } => {
                if self.stage != Stage::Asking || !is_subject || self.settings.instruction_mode != InstructionMode::Free {
                    return err("Action non autorisée.");
                }
                self.round.as_mut().unwrap().lie = Some(lie);
            }
            Action::StartAnswer => {
                if self.stage != Stage::Asking || !is_subject {
                    return err("Action non autorisée.");
                }
                let r = self.round.as_ref().unwrap();
                if !r.question_locked {
                    return err("L'interrogateur n'a pas encore choisi la question.");
                }
                if r.lie.is_none() && !self.free() {
                    return err("Choisis d'abord : vérité ou mensonge ?");
                }
                self.stage = Stage::Answering;
            }
            Action::CancelAnswer => {
                if self.stage != Stage::Answering || !(is_subject || host) {
                    return err("Action non autorisée.");
                }
                self.stage = Stage::Asking;
            }
            Action::SubmitVerdict { mut verdict } => {
                if self.stage != Stage::Answering || !is_subject {
                    return err("Action non autorisée.");
                }
                verdict.sanitize();
                if self.free() {
                    // Pas de vote en mode libre : le verdict tombe tout de suite.
                    let r = self.round.as_mut().unwrap();
                    self.history.push(RoundRecord {
                        number: r.number,
                        subject: r.subject.clone(),
                        asker: r.asker.clone(),
                        question: r.question.clone(),
                        lie: None,
                        index: verdict.index,
                        machine_correct: None,
                        heart_rate: verdict.heart_rate,
                        votes: BTreeMap::new(),
                        points: BTreeMap::new(),
                    });
                    if self.history.len() > 100 {
                        self.history.remove(0);
                    }
                    r.verdict = Some(verdict);
                    self.stage = Stage::Reveal;
                } else {
                    self.round.as_mut().unwrap().verdict = Some(verdict);
                    self.stage = Stage::Voting;
                    let secs = self.settings.vote_seconds;
                    self.vote_deadline = (secs > 0).then(|| Instant::now() + Duration::from_secs(u64::from(secs)));
                    self.check_votes_complete();
                }
            }
            Action::Vote { voter, lie, sure } => {
                if self.stage != Stage::Voting || !self.owns(device, &voter) || !self.voters().contains(&voter) {
                    return err("Vote non autorisé.");
                }
                let votes = &mut self.round.as_mut().unwrap().votes;
                if votes.contains_key(&voter) {
                    return err("Vote déjà enregistré.");
                }
                votes.insert(voter, Vote { lie, sure });
                self.check_votes_complete();
            }
            Action::Reveal => {
                if !host || self.stage != Stage::Voting {
                    return err("Action non autorisée.");
                }
                self.reveal();
            }
            Action::NextRound => {
                // Mode libre : tout le monde peut revenir à la salle (ou annuler une question).
                let allowed = if self.free() { matches!(self.stage, Stage::Asking | Stage::Reveal) } else { host && self.stage == Stage::Reveal };
                if !allowed {
                    return err("Action non autorisée.");
                }
                self.begin_round();
            }
            Action::EndGame => {
                if !host {
                    return err("Action non autorisée.");
                }
                self.stage = Stage::Finished;
                self.round = None;
            }
            Action::BackToLobby => {
                if !host {
                    return err("Action non autorisée.");
                }
                self.players.retain(|p| p.connected);
                self.stage = Stage::Lobby;
                self.round = None;
                self.round_no = 0;
            }
        }
        Ok(())
    }

    /// Sujet en train de répondre depuis `device` (pour relayer son tracé en direct).
    pub fn live_subject(&self, device: &str) -> Option<String> {
        (self.stage == Stage::Answering && self.subject_device() == Some(device)).then(|| self.round.as_ref().unwrap().subject.clone())
    }

    /// Un appareil revient après une coupure : ses joueurs redeviennent actifs.
    pub fn device_rejoined(&mut self, device: &str) -> bool {
        let mut any = false;
        for p in self.players.iter_mut().filter(|p| p.device == device) {
            p.connected = true;
            any = true;
        }
        any
    }

    /// Un appareil s'est déconnecté.
    pub fn device_left(&mut self, device: &str) {
        if self.stage == Stage::Lobby {
            self.players.retain(|p| p.device != device);
            return;
        }
        for p in self.players.iter_mut().filter(|p| p.device == device) {
            p.connected = false;
        }
        self.after_departure();
    }

    fn after_departure(&mut self) {
        let subject_gone = self
            .round
            .as_ref()
            .and_then(|r| self.player(&r.subject))
            .is_some_and(|p| !p.connected);
        let asker_gone = self
            .round
            .as_ref()
            .and_then(|r| r.asker.as_ref())
            .and_then(|a| self.player(a))
            .is_some_and(|p| !p.connected);
        if subject_gone && matches!(self.stage, Stage::Asking | Stage::Answering | Stage::Voting) {
            // (en mode libre, begin_round ramène simplement à la salle)
            self.begin_round();
        } else {
            if asker_gone {
                // Le sujet (ou l'hôte) choisira lui-même la question.
                self.round.as_mut().unwrap().asker = None;
            }
            self.check_votes_complete();
        }
    }

    pub fn view_for(&self, device: &str) -> GameView {
        let revealed = matches!(self.stage, Stage::Reveal);
        let subject_dev = self.subject_device();
        GameView {
            device: device.into(),
            is_host: device == HOST_DEVICE,
            settings: self.settings.clone(),
            stage: self.stage,
            players: self
                .players
                .iter()
                .map(|p| PlayerView { info: p.info.clone(), score: p.score, connected: p.connected, mine: p.device == device })
                .collect(),
            round: self.round.as_ref().map(|r| RoundView {
                number: r.number,
                subject: r.subject.clone(),
                asker: r.asker.clone(),
                choices: r.choices.clone(),
                question: r.question.clone(),
                question_locked: r.question_locked,
                lie: if revealed || subject_dev == Some(device) { r.lie } else { None },
                verdict: if revealed { r.verdict.clone() } else { None },
                voted: r.votes.keys().cloned().collect(),
                votes: revealed.then(|| r.votes.clone()),
                outcome: r.outcome.clone(),
            }),
            round_no: self.round_no,
            total_rounds: self.total_rounds(),
            machine: self.machine.clone(),
            history: self.history.clone(),
            suggestions: if self.stage == Stage::Free { self.suggestions.clone() } else { vec![] },
            vote_remaining_ms: self
                .vote_deadline()
                .map(|(_, end)| end.saturating_duration_since(Instant::now()).as_millis() as u64),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str) -> PlayerInfo {
        PlayerInfo { id: id.into(), name: id.to_uppercase(), avatar: "🙂".into(), calibrated: true }
    }

    fn verdict(is_lie: bool) -> Verdict {
        Verdict { probability: if is_lie { 0.8 } else { 0.2 }, index: 0, label: String::new(), is_lie, factors: vec![], heart_rate: None, confidence: 0.5, speaker: None }
    }

    #[test]
    fn network_round_with_secrets_and_scoring() {
        let mut g = Game::new(StdRng::seed_from_u64(1));
        g.apply(HOST_DEVICE, Action::AddPlayers { players: vec![info("a")] }).unwrap();
        g.apply("d2", Action::AddPlayers { players: vec![info("b"), info("c")] }).unwrap();
        assert!(g.apply("d2", Action::Start).is_err());
        g.apply(HOST_DEVICE, Action::Start).unwrap();
        assert_eq!(g.stage, Stage::Asking);
        assert_eq!(g.total_rounds(), 6);

        let r = g.round.clone().unwrap();
        assert_eq!(r.subject, "a");
        let truth = r.lie.unwrap();
        // La consigne n'est visible que de l'appareil du sujet.
        assert_eq!(g.view_for(HOST_DEVICE).round.unwrap().lie, Some(truth));
        assert_eq!(g.view_for("d2").round.unwrap().lie, None);

        assert!(g.apply("d2", Action::StartAnswer).is_err());
        // b (sur d2) est l'interrogateur : le sujet attend sa question.
        assert_eq!(r.asker.as_deref(), Some("b"));
        assert!(g.apply(HOST_DEVICE, Action::StartAnswer).is_err());
        g.apply("d2", Action::ChooseQuestion { index: 2 }).unwrap();
        assert_eq!(g.round.as_ref().unwrap().question, r.choices[2]);
        g.apply(HOST_DEVICE, Action::StartAnswer).unwrap();
        g.apply(HOST_DEVICE, Action::SubmitVerdict { verdict: verdict(!truth) }).unwrap();
        assert_eq!(g.stage, Stage::Voting);
        assert!(g.view_for("d2").round.unwrap().verdict.is_none());
        assert!(g.apply(HOST_DEVICE, Action::Vote { voter: "b".into(), lie: true, sure: false }).is_err());
        g.apply("d2", Action::Vote { voter: "b".into(), lie: truth, sure: true }).unwrap();
        assert!(g.apply("d2", Action::Vote { voter: "b".into(), lie: !truth, sure: false }).is_err(), "pas de second vote");
        g.apply("d2", Action::Vote { voter: "c".into(), lie: !truth, sure: true }).unwrap();
        assert_eq!(g.stage, Stage::Reveal);
        let o = g.round.as_ref().unwrap().outcome.clone().unwrap();
        assert!(!o.machine_correct);
        assert_eq!(o.points["a"], 3); // 1 joueur dupé + machine dupée
        assert_eq!(o.points["b"], 2); // pari gagné
        assert_eq!(o.points["c"], -1); // pari perdu
        assert_eq!(g.history.len(), 1);
        assert_eq!(g.view_for("d2").history[0].lie, Some(truth));
        assert_eq!(g.machine, MachineStats { correct: 0, total: 1 });

        g.apply(HOST_DEVICE, Action::NextRound).unwrap();
        assert_eq!(g.round.as_ref().unwrap().subject, "b");
        // Le sujet se déconnecte : on passe au suivant.
        g.device_left("d2");
        assert_eq!(g.stage, Stage::Asking);
        assert_eq!(g.round.as_ref().unwrap().subject, "a");
    }

    #[test]
    fn voting_timer_and_question_memory() {
        let mut g = Game::new(StdRng::seed_from_u64(11));
        g.settings.vote_seconds = 15;
        g.set_used_questions(questions_for(Pack::Soft).iter().skip(1).map(|q| q.to_string()).collect());
        g.apply(HOST_DEVICE, Action::AddPlayers { players: vec![info("a")] }).unwrap();
        g.apply("d2", Action::AddPlayers { players: vec![info("b")] }).unwrap();
        g.apply(HOST_DEVICE, Action::Start).unwrap();
        // Une seule question jamais posée : elle sort forcément parmi les 3 proposées.
        let fresh = questions_for(Pack::Soft)[0];
        assert!(g.round.as_ref().unwrap().choices.iter().any(|q| q == fresh));
        assert!(g.used_questions().len() <= MAX_USED_QUESTIONS);

        g.apply("d2", Action::ChooseQuestion { index: 0 }).unwrap();
        g.apply(HOST_DEVICE, Action::StartAnswer).unwrap();
        assert!(g.view_for("d2").vote_remaining_ms.is_none());
        g.apply(HOST_DEVICE, Action::SubmitVerdict { verdict: verdict(true) }).unwrap();
        let (n, _) = g.vote_deadline().expect("échéance de vote");
        assert_eq!(n, 1);
        let left = g.view_for("d2").vote_remaining_ms.unwrap();
        assert!(left > 14_000 && left <= 15_000, "{left}");
        // L'hôte révèle à l'échéance (le joueur n'a pas voté).
        g.apply(HOST_DEVICE, Action::Reveal).unwrap();
        assert_eq!(g.stage, Stage::Reveal);
        assert!(g.vote_deadline().is_none());
    }

    #[test]
    fn free_mode_anyone_asks_anything() {
        let mut g = Game::new(StdRng::seed_from_u64(5));
        g.settings.mode = GameMode::Free;
        g.apply(HOST_DEVICE, Action::AddPlayers { players: vec![info("a")] }).unwrap();
        g.apply("d2", Action::AddPlayers { players: vec![info("b")] }).unwrap();
        g.apply(HOST_DEVICE, Action::Start).unwrap();
        assert_eq!(g.stage, Stage::Free);
        assert_eq!(g.view_for("d2").suggestions.len(), 3);
        // Un retardataire peut rejoindre la salle.
        g.apply("d3", Action::AddPlayers { players: vec![info("c")] }).unwrap();

        // b (invité) interroge a (hôte) avec sa propre question.
        assert!(g.apply("d2", Action::Ask { subject: "a".into(), question: "x".into(), asker: Some("a".into()) }).is_err(), "pas au nom d'un autre");
        g.apply("d2", Action::Ask { subject: "a".into(), question: "  As-tu lu mes messages ?  ".into(), asker: Some("b".into()) }).unwrap();
        assert_eq!(g.round.as_ref().unwrap().question, "As-tu lu mes messages ?");
        assert!(g.apply("d2", Action::StartAnswer).is_err(), "seul l'interrogé répond");
        g.apply(HOST_DEVICE, Action::StartAnswer).unwrap();
        g.apply(HOST_DEVICE, Action::SubmitVerdict { verdict: verdict(true) }).unwrap();
        assert_eq!(g.stage, Stage::Reveal, "pas de vote en mode libre");
        let v = g.view_for("d3");
        assert!(v.round.as_ref().unwrap().verdict.is_some());
        assert_eq!(v.round.unwrap().lie, None, "secret tant qu'il n'avoue pas");
        assert_eq!(v.history[0].lie, None);

        assert!(g.apply("d2", Action::Confess { lie: false }).is_err());
        g.apply(HOST_DEVICE, Action::Confess { lie: true }).unwrap();
        assert!(g.apply(HOST_DEVICE, Action::Confess { lie: false }).is_err(), "aveu unique");
        assert_eq!(g.machine, MachineStats { correct: 1, total: 1 });
        assert_eq!(g.history[0].machine_correct, Some(true));

        // N'importe qui enchaîne directement avec une autre question, à n'importe qui.
        g.apply("d3", Action::Ask { subject: "b".into(), question: "Et toi ?".into(), asker: None }).unwrap();
        g.apply("d3", Action::NextRound).unwrap(); // annule : retour à la salle
        assert_eq!(g.stage, Stage::Free);
        assert!(g.players.iter().all(|p| p.score == 0), "pas de points en mode libre");
    }

    #[test]
    fn asker_bonus_and_departure() {
        let mut g = Game::new(StdRng::seed_from_u64(9));
        g.settings.instruction_mode = InstructionMode::Free;
        g.apply(HOST_DEVICE, Action::AddPlayers { players: vec![info("a")] }).unwrap();
        g.apply("d2", Action::AddPlayers { players: vec![info("b")] }).unwrap();
        g.apply(HOST_DEVICE, Action::Start).unwrap();
        // Seul l'interrogateur (ou l'hôte) choisit.
        assert!(g.apply("d3", Action::CustomQuestion { text: "x".into() }).is_err());
        g.apply("d2", Action::CustomQuestion { text: "  As-tu triché ?  ".into() }).unwrap();
        assert_eq!(g.round.as_ref().unwrap().question, "As-tu triché ?");
        g.apply(HOST_DEVICE, Action::Declare { lie: true }).unwrap();
        g.apply(HOST_DEVICE, Action::StartAnswer).unwrap();
        g.apply(HOST_DEVICE, Action::SubmitVerdict { verdict: verdict(true) }).unwrap();
        g.apply("d2", Action::Vote { voter: "b".into(), lie: false, sure: false }).unwrap();
        let o = g.round.as_ref().unwrap().outcome.clone().unwrap();
        assert_eq!(o.points["b"], 1, "question piège réussie, vote raté");
        assert_eq!(o.points["a"], 1);

        // Manche 2 : b est sujet, a interroge… puis l'interrogateur part.
        g.apply(HOST_DEVICE, Action::NextRound).unwrap();
        assert_eq!(g.round.as_ref().unwrap().asker.as_deref(), Some("a"));
        g.apply(HOST_DEVICE, Action::RemovePlayer { player_id: "a".into() }).unwrap();
        assert_eq!(g.round.as_ref().unwrap().asker, None);
        g.apply("d2", Action::ChooseQuestion { index: 0 }).unwrap();
        assert!(g.round.as_ref().unwrap().question_locked);
    }

    #[test]
    fn untrusted_inputs_are_bounded() {
        let mut g = Game::new(StdRng::seed_from_u64(3));
        let evil = PlayerInfo { id: "x".repeat(500), name: "<b>".repeat(100), avatar: "🙂".repeat(50), calibrated: true };
        g.apply("d1", Action::AddPlayers { players: vec![evil] }).unwrap();
        let p = &g.players[0].info;
        assert!(p.id.len() <= 40 && p.name.chars().count() <= 24 && p.avatar.chars().count() <= 4);
        assert!(g.apply("d1", Action::AddPlayers { players: vec![PlayerInfo { id: " ".into(), ..info("z") }] }).is_err());

        g.apply(HOST_DEVICE, Action::Start).unwrap();
        g.apply("d1", Action::StartAnswer).unwrap();
        let mut v = verdict(false);
        v.probability = f32::NAN;
        v.index = 99;
        v.factors = (0..1000).map(|_| crate::model::Factor { key: "k".into(), label: "l".repeat(999), description: String::new(), z: f32::INFINITY, contribution: 0.0 }).collect();
        g.apply("d1", Action::SubmitVerdict { verdict: v }).unwrap();
        let v = g.round.as_ref().unwrap().verdict.clone().unwrap();
        assert_eq!((v.probability, v.index, v.is_lie), (0.0, 0, false));
        assert!(v.factors.len() <= 6 && v.factors[0].label.len() <= 40 && v.factors[0].z == 0.0);
    }

    #[test]
    fn solo_free_mode_reveals_immediately() {
        let mut g = Game::new(StdRng::seed_from_u64(2));
        g.settings.instruction_mode = InstructionMode::Free;
        g.apply(HOST_DEVICE, Action::AddPlayers { players: vec![info("a")] }).unwrap();
        g.apply(HOST_DEVICE, Action::Start).unwrap();
        assert!(g.apply(HOST_DEVICE, Action::StartAnswer).is_err());
        g.apply(HOST_DEVICE, Action::Declare { lie: true }).unwrap();
        g.apply(HOST_DEVICE, Action::StartAnswer).unwrap();
        g.apply(HOST_DEVICE, Action::SubmitVerdict { verdict: verdict(true) }).unwrap();
        assert_eq!(g.stage, Stage::Reveal);
        assert_eq!(g.machine.correct, 1);
    }
}
