//! JSON Schema des fichiers de configuration, pour l'éditeur (taplo) et la doc.

use serde_json::Value;

use crate::establishment::EstablishmentFile;
use crate::university::UniversityFile;

/// JSON Schema de `university.toml`.
#[must_use]
pub fn university_schema() -> Value {
    Value::from(schemars::schema_for!(UniversityFile))
}

/// JSON Schema d'un fichier `establishments/<id>.toml`.
#[must_use]
pub fn establishment_schema() -> Value {
    Value::from(schemars::schema_for!(EstablishmentFile))
}
