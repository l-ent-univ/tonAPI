//! Modèles unifiés de tonAPI et erreurs au format RFC 9457.
//!
//! Tous les fournisseurs convertissent leurs données vers ces types : un client n'a jamais à savoir si
//! l'emploi du temps vient d'un flux iCal ou d'une API REST ADE.

mod models;
mod problem;

pub use models::*;
pub use problem::{ErrorCode, Problem};
