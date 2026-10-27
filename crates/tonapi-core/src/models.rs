//! Données renvoyées par l'API. Champs en camelCase, dates ISO 8601 avec fuseau.

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

/// Relevé de notes, semestre par semestre.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GradeReport {
    pub semesters: Vec<SemesterGrades>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnitGrade {
    pub code: String,
    pub name: String,
    pub average: Option<Grade>,
    pub ects: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModuleGrades {
    pub code: String,
    pub name: String,
    pub average: Option<Grade>,
    pub evaluations: Vec<Evaluation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Evaluation {
    pub id: String,
    pub name: String,
    pub date: Option<NaiveDate>,
    pub grade: Option<Grade>,
    pub coefficient: Option<f64>,
    /// Mention à la place d'une note (ABS, EXC, ATT…).
    pub status: Option<String>,
}

/// Absences et totaux.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AbsenceReport {
    pub absences: Vec<Absence>,
    pub totals: AbsenceTotals,
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
