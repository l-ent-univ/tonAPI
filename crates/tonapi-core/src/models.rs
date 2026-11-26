//! Données renvoyées par l'API. Champs en camelCase, dates ISO 8601 avec fuseau.

use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset, NaiveDate};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Métadonnées jointes à chaque réponse de données.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    /// Identifiant de l'université.
    pub university: String,
    /// Composante de l'étudiant, si connue.
    pub establishment: Option<String>,
    /// Fournisseur qui a produit les données (`ade-portal-rest`, `demo`…).
    pub provider: String,
    pub fetched_at: DateTime<FixedOffset>,
    /// Durée, en secondes, pendant laquelle le client peut resservir cette réponse depuis sa
    /// mémoire sans la redemander. Absent : ne pas la garder. Le serveur ne garde rien, et la
    /// réponse HTTP reste `no-store` : un navigateur ne l'écrit jamais dans son cache.
    pub cache_secs: Option<u32>,
}

/// Cours de l'emploi du temps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Course {
    /// Identifiant stable d'une séance (UID iCal, id d'événement ADE).
    pub id: String,
    pub title: String,
    /// Type de séance tel que fourni (CM, TD, TP, Examen…).
    pub kind: Option<String>,
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    pub rooms: Vec<String>,
    pub teachers: Vec<String>,
    pub groups: Vec<String>,
    pub description: Option<String>,
    /// Couleur de la séance telle que la donne le logiciel (ADE : `R,V,B`, ex. `120,140,160`).
    pub color: Option<String>,
    /// Activité à laquelle la séance appartient, si le logiciel la nomme (ADE : `activityId`).
    pub activity_id: Option<String>,
}

/// Groupe d'emploi du temps sélectionnable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TimetableGroup {
    pub id: String,
    pub name: String,
    /// Chemin dans l'arbre des groupes (ex. `IUT Lannion › BUT MMI › MMI A2`).
    pub path: Vec<String>,
    /// Groupe actuellement suivi par l'étudiant.
    pub selected: bool,
}

/// Note ramenée à son barème.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Grade {
    pub value: f64,
    pub out_of: f64,
}

/// Notes de la promotion pour une évaluation, une unité ou un semestre, sur le barème de la note
/// qu'elles accompagnent.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GradeStats {
    pub min: Option<f64>,
    pub mean: Option<f64>,
    pub max: Option<f64>,
}

/// Relevé de notes, semestre par semestre.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GradeReport {
    /// Semestres détaillés : le semestre courant, ou celui demandé.
    pub semesters: Vec<SemesterGrades>,
    /// Tous les semestres de l'étudiant, du plus ancien au plus récent, quand le logiciel les
    /// liste. Leur `id` se passe à `GET /v1/grades?semester=`.
    pub available: Vec<SemesterRef>,
}

/// Message d'un département à afficher avec ses relevés.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GradeNotice {
    /// Texte brut ; absent quand le département n'a rien écrit.
    pub message: Option<String>,
}

/// Répartition des notes de la promotion à une évaluation.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GradeDistribution {
    /// Nombre de notes par point entier : 21 cases, de `[0 ; 1[` à `20`. Vide si `unavailable`.
    pub counts: Vec<u32>,
    /// Nombre de notes comptées.
    pub total: u32,
    /// Barème des cases.
    pub out_of: f64,
    /// Pourquoi il n'y a pas de répartition ; absent quand il y en a une.
    pub unavailable: Option<DistributionUnavailable>,
}

/// Raison pour laquelle une répartition n'est pas donnée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DistributionUnavailable {
    /// L'établissement ne partage pas les répartitions.
    Disabled,
    /// Le relevé n'est pas publié.
    Hidden,
    /// Trop peu de notes pour ne désigner personne.
    TooFew,
}

/// Semestre dont le relevé peut être demandé.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SemesterRef {
    pub id: String,
    pub name: String,
    /// Rang du semestre dans la formation (1 à 6 en BUT).
    pub number: Option<u32>,
    /// Année universitaire telle que fournie (ex. `2026/2027`).
    pub year: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SemesterGrades {
    pub id: String,
    pub name: String,
    pub average: Option<Grade>,
    /// Rang tel que fourni (ex. `12/84`).
    pub rank: Option<String>,
    /// Unités d'enseignement (compétences en BUT).
    pub units: Vec<UnitGrade>,
    /// Ressources, SAÉ ou matières et leurs évaluations.
    pub modules: Vec<ModuleGrades>,
    /// Rang du semestre dans la formation (1 à 6 en BUT).
    pub number: Option<u32>,
    /// Année universitaire telle que fournie (ex. `2026 - 2027`).
    pub year: Option<String>,
    /// Premier et dernier jour du semestre.
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
    /// Type de relevé tel que fourni (ex. `BUT`).
    pub kind: Option<String>,
    /// Sigle de la formation.
    pub program: Option<String>,
    /// Sigle du département qui gère le semestre.
    pub department: Option<String>,
    /// `false` : l'établissement n'a pas publié ce relevé (voir `notice`). Absent : inconnu.
    pub published: Option<bool>,
    /// Message de l'établissement sur ce relevé (non publié…), en texte brut.
    pub notice: Option<String>,
    /// Date de production du relevé.
    pub updated_at: Option<DateTime<FixedOffset>>,
    /// Moyennes générales de la promotion.
    pub stats: Option<GradeStats>,
    /// Nombre d'étudiants classés dans le semestre, quand le logiciel le donne : il peut être connu
    /// alors que l'établissement masque le rang lui-même.
    pub ranked: Option<u32>,
    /// Rang dans chacun des groupes de l'étudiant.
    pub group_ranks: Vec<GroupRank>,
    /// Groupes de l'étudiant dans ce semestre (TD, TP, parcours…).
    pub groups: Vec<StudentGroup>,
    /// Crédits ECTS du semestre et crédits acquis, quand l'établissement les montre.
    pub ects: Option<f64>,
    pub ects_earned: Option<f64>,
    /// Décision de jury sur le semestre, puis sur l'année.
    pub decision: Option<JuryDecision>,
    pub year_decision: Option<JuryDecision>,
    /// Le relevé PDF de ce semestre peut être demandé (`GET /v1/grades/{id}/transcript`). Absent :
    /// inconnu pour ce semestre.
    pub transcript: Option<bool>,
    /// La répartition des notes d'une évaluation peut être demandée
    /// (`GET /v1/grades/{id}/evaluations/{evaluation}/distribution`). Absent : inconnu.
    pub distributions: Option<bool>,
}

/// Rang dans un groupe, sous la même forme que le rang du semestre.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupRank {
    /// Nom du groupe, ou son identifiant s'il n'est pas nommé.
    pub group: String,
    /// Rang tel que fourni (ex. `3/28`).
    pub rank: String,
}

/// Groupe d'étudiants.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StudentGroup {
    pub id: Option<String>,
    pub name: String,
    /// Découpage auquel le groupe appartient (ex. `TD`, `TP`, `Parcours`).
    pub partition: Option<String>,
}

/// Décision de jury.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JuryDecision {
    /// Code tel que fourni (ex. `ADM`, `AJ`, `RED`).
    pub code: String,
    pub date: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnitGrade {
    pub code: String,
    pub name: String,
    pub average: Option<Grade>,
    /// Crédits ECTS de l'unité.
    pub ects: Option<f64>,
    /// Crédits acquis, quand l'établissement les montre.
    pub ects_earned: Option<f64>,
    /// Couleur de l'unité (`#RRGGBB`).
    pub color: Option<String>,
    /// Moyennes de la promotion dans l'unité.
    pub stats: Option<GradeStats>,
    /// Rang tel que fourni (ex. `12/84`).
    pub rank: Option<String>,
    /// Points ajoutés et retirés à la moyenne de l'unité, déjà comptés dans `average`.
    pub bonus: Option<f64>,
    pub malus: Option<f64>,
    /// Unité acquise lors d'un semestre précédent et conservée.
    pub capitalized: bool,
    /// Code de la décision de jury sur l'unité (ex. `ADM`, `CMP`, `AJ`).
    pub decision: Option<String>,
    /// Modules qui comptent dans l'unité, avec leur coefficient.
    pub modules: Vec<UnitModule>,
}

/// Part d'un module dans une unité.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnitModule {
    /// Code du module (`ModuleGrades::code`).
    pub code: String,
    pub coefficient: Option<f64>,
    /// Moyenne du module dans cette unité.
    pub average: Option<Grade>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModuleGrades {
    pub code: String,
    pub name: String,
    pub average: Option<Grade>,
    pub evaluations: Vec<Evaluation>,
    /// Identifiant du module dans le logiciel.
    pub id: Option<String>,
    /// Nature du module telle que le logiciel la range (ScoDoc : `ressource` ou `sae`).
    pub kind: Option<String>,
}

/// Place d'une évaluation dans le calcul de la moyenne.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EvaluationKind {
    #[default]
    Normal,
    /// Rattrapage : remplace la moyenne des évaluations normales si elle est meilleure.
    Resit,
    /// Seconde session : remplace les évaluations de la première.
    SecondSession,
    /// Points de bonus, hors moyenne.
    Bonus,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Evaluation {
    pub id: String,
    pub name: String,
    pub date: Option<NaiveDate>,
    pub grade: Option<Grade>,
    pub coefficient: Option<f64>,
    /// Mention à la place d'une note (ABS, EXC, ATT…).
    pub status: Option<String>,
    pub kind: EvaluationKind,
    /// Poids de l'évaluation dans chaque unité (clé : `UnitGrade::code`). Vide si le logiciel ne
    /// pondère pas par unité.
    pub weights: BTreeMap<String, f64>,
    /// Notes de la promotion à cette évaluation.
    pub stats: Option<GradeStats>,
    /// Début et fin de l'épreuve, quand l'heure est connue.
    pub start: Option<DateTime<FixedOffset>>,
    pub end: Option<DateTime<FixedOffset>>,
}

/// Absences et totaux.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AbsenceReport {
    pub absences: Vec<Absence>,
    pub totals: AbsenceTotals,
    /// Retards, à part des absences : ils ne comptent pas dans `totals`.
    pub lates: Vec<Absence>,
    /// L'établissement montre ses absences à l'étudiant. Absent : inconnu.
    pub visible: Option<bool>,
    /// Unité dans laquelle l'établissement compte les absences (ScoDoc : `heure`, `demi`,
    /// `journee` ou `compte`). Absent : inconnue.
    pub unit: Option<String>,
    /// Totaux calculés par l'établissement, par unité.
    pub unit_totals: Option<AbsenceUnitTotals>,
    /// Message de l'établissement sur les absences (délai de justification…), en texte brut.
    pub notice: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Absence {
    pub id: String,
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    pub justified: bool,
    /// Matière ou ressource concernée.
    pub module: Option<String>,
    /// Identifiant de cette matière dans le logiciel (`ModuleGrades::id`), ou son libellé quand
    /// le logiciel ne donne que lui.
    pub module_id: Option<String>,
    /// Enseignant qui a relevé l'absence, tel que fourni (souvent une adresse).
    pub teacher: Option<String>,
}

/// Totaux d'absences de l'établissement, pour chaque unité qu'il fournit (clé : l'unité).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AbsenceUnitTotals {
    pub justified: BTreeMap<String, f64>,
    pub unjustified: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AbsenceTotals {
    pub justified: u32,
    pub unjustified: u32,
}

/// Devoir ou échéance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Homework {
    pub id: String,
    pub title: String,
    /// Cours ou espace d'origine.
    pub course: Option<String>,
    pub due: DateTime<FixedOffset>,
    pub description: Option<String>,
    /// Page du devoir.
    pub url: Option<String>,
    /// Rendu déposé, si l'information est disponible.
    pub submitted: Option<bool>,
    /// Type d'activité tel que le nomme le logiciel (Moodle : `assign`, `quiz`, `forum`…), s'il
    /// est connu.
    pub kind: Option<String>,
}

/// Boîte de réception.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MailInbox {
    pub unread: u32,
    pub messages: Vec<MailMessage>,
    /// Webmail, pour lire un message en entier.
    pub webmail_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MailMessage {
    pub id: String,
    pub subject: String,
    pub from: MailAddress,
    pub date: DateTime<FixedOffset>,
    pub unread: bool,
    /// Début du texte.
    pub preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MailAddress {
    pub name: Option<String>,
    pub address: String,
}

/// Épreuve du calendrier d'examens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Exam {
    /// Code de l'épreuve dans le logiciel de scolarité.
    pub id: String,
    pub title: String,
    /// Début de l'épreuve. Absent quand la date ou l'heure n'est pas encore fixée.
    pub start: Option<DateTime<FixedOffset>>,
    /// Durée en minutes.
    pub duration_minutes: Option<u32>,
    pub room: Option<String>,
    pub building: Option<String>,
    /// Période ou session d'examens (ex. `Session 1 - Semestre impair`).
    pub session: Option<String>,
    /// Enseignements concernés (`code - intitulé`).
    pub courses: Vec<String>,
    /// Consigne particulière publiée avec l'épreuve.
    pub notice: Option<String>,
}

/// Document administratif de l'étudiant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    /// Identifiant à passer à `GET /v1/documents/{id}` quand `downloadable` est vrai.
    pub id: String,
    pub title: String,
    pub kind: DocumentKind,
    /// Année universitaire de début (ex. `2025` pour 2025-2026).
    pub year: Option<String>,
    pub session: Option<String>,
    /// Le fichier est servi par `GET /v1/documents/{id}`.
    pub downloadable: bool,
    /// Page de l'établissement qui produit le document, quand l'API ne le sert pas.
    pub url: Option<String>,
}

/// Nature d'un document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DocumentKind {
    Certificate,
    SemesterTranscript,
    AnnualTranscript,
    Achievement,
}

/// Application du portail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct App {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub url: String,
    pub icon_url: Option<String>,
    pub category: Option<String>,
}

/// Profil de l'étudiant connecté.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// Identifiant de connexion.
    pub username: String,
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub student_number: Option<String>,
    /// Formation suivie (libellé de l'étape d'inscription), quand l'université la donne.
    pub program: Option<String>,
}

/// Comment la composante de l'étudiant a été choisie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum EstablishmentSource {
    /// Déduite des signaux du profil.
    Detected,
    /// Composante par défaut de l'université.
    Default,
    /// Choisie par l'étudiant.
    Student,
}

/// Composante de l'étudiant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EstablishmentChoice {
    pub id: String,
    pub name: String,
    pub source: EstablishmentSource,
    /// Confiance de la détection, entre 0 et 1.
    pub confidence: f32,
    /// Signaux qui ont servi à la détection.
    pub signals: Vec<String>,
}

/// Disponibilité d'un service pour l'étudiant ou l'université.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    /// `timetable`, `grades`…
    pub service: String,
    /// `api`, `link` ou `absent`.
    pub availability: Availability,
    /// Fournisseur, si `api`.
    pub provider: Option<String>,
    /// Page à ouvrir, si `link`.
    pub link: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum Availability {
    Api,
    Link,
    Absent,
}
