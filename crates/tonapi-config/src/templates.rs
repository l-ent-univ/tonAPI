//! Modèles de fichiers pour `tonapi new`.

/// Texte laissé dans un modèle là où il faut écrire quelque chose. Refusé par la validation.
pub const PLACEHOLDER: &str = "<à remplir>";

/// Hôte laissé dans les URL d'un modèle. Refusé par la validation.
pub const PLACEHOLDER_HOST: &str = "a-remplir.invalid";

/// Vrai si le texte contient encore une valeur de modèle.
pub(crate) fn is_placeholder(text: &str) -> bool {
    text.contains(PLACEHOLDER) || text.contains(PLACEHOLDER_HOST)
}

/// `university.toml` pour une nouvelle université.
#[must_use]
pub fn university_template(id: &str) -> String {
    include_str!("../templates/university.toml").replace("{id}", id)
}

/// `establishments/<id>.toml` pour une nouvelle composante de `university`.
#[must_use]
pub fn establishment_template(university: &str, id: &str) -> String {
    include_str!("../templates/establishment.toml")
        .replace("{university}", university)
        .replace("{id}", id)
}
