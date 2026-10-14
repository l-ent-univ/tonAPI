//! Règles de rattachement d'un étudiant à une composante.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Règles de rattachement automatique.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Affiliation {
    /// Chaque règle qui correspond ajoute son poids au score de la composante.
    #[serde(default)]
    pub rules: Vec<MatchRule>,
}

impl Affiliation {
    /// Somme des poids des règles qui correspondent à au moins un signal.
    #[must_use]
    pub fn score(&self, signals: &[Signal]) -> u32 {
        self.rules
            .iter()
            .filter(|rule| signals.iter().any(|signal| rule.matches(signal)))
            .map(|rule| u32::from(rule.weight))
            .sum()
    }
}

/// Une règle : un signal du profil et un motif. Renseigner exactement un de `contains`, `prefix`, `suffix`.
///
/// La comparaison ignore la casse, les accents et les espaces multiples ; un `?` ou `�` dans le signal
/// (accent mal encodé par le serveur) correspond à n'importe quel caractère.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct MatchRule {
    /// Signal comparé.
    pub signal: SignalSource,
    /// Le signal contient ce texte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contains: Option<String>,
    /// Le signal commence par ce texte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    /// Le signal finit par ce texte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    /// Poids de la règle. Défaut : 1.
    #[serde(default = "default_weight")]
    pub weight: u8,
}

fn default_weight() -> u8 {
    1
}

impl MatchRule {
    /// Vrai si le signal est de la bonne source et correspond au motif.
    #[must_use]
    pub fn matches(&self, signal: &Signal) -> bool {
        if signal.source != self.signal {
            return false;
        }
        let value: Vec<char> = normalize(&signal.value).chars().collect();
        if let Some(p) = &self.contains {
            let p: Vec<char> = normalize(p).chars().collect();
            return !p.is_empty() && value.windows(p.len()).any(|w| same(w, &p));
        }
        if let Some(p) = &self.prefix {
            let p: Vec<char> = normalize(p).chars().collect();
            return !p.is_empty() && value.len() >= p.len() && same(&value[..p.len()], &p);
        }
        if let Some(p) = &self.suffix {
            let p: Vec<char> = normalize(p).chars().collect();
            return !p.is_empty()
                && value.len() >= p.len()
                && same(&value[value.len() - p.len()..], &p);
        }
        false
    }

    /// Nombre de motifs renseignés (doit valoir 1).
    pub(crate) fn pattern_count(&self) -> usize {
        [&self.contains, &self.prefix, &self.suffix]
            .iter()
            .filter(|p| p.as_deref().is_some_and(|p| !normalize(p).is_empty()))
            .count()
    }
}

fn same(value: &[char], pattern: &[char]) -> bool {
    value
        .iter()
        .zip(pattern)
        .all(|(v, p)| v == p || *v == '?' || *v == '\u{FFFD}')
}

/// Source d'un signal de rattachement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SignalSource {
    /// Chemin du groupe dans l'arbre ADE (ex. `IUT Lannion › BUT MMI › MMI A2`).
    AdeGroupPath,
    /// Composante d'inscription (Mon Dossier Web, ScoDoc…).
    EnrollmentComponent,
    /// Adresse postale du profil.
    PostalAddress,
    /// Adresse ou nom d'une salle de cours.
    RoomAddress,
}

/// Valeur observée dans les données de l'étudiant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Signal {
    pub source: SignalSource,
    pub value: String,
}

/// Minuscules, sans accents, espaces regroupés.
#[must_use]
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        match c {
            'à' | 'â' | 'ä' | 'á' | 'ã' | 'å' => out.push('a'),
            'ç' => out.push('c'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'î' | 'ï' | 'í' | 'ì' => out.push('i'),
            'ô' | 'ö' | 'ó' | 'ò' | 'õ' => out.push('o'),
            'ù' | 'û' | 'ü' | 'ú' => out.push('u'),
            'ÿ' => out.push('y'),
            'œ' => out.push_str("oe"),
            'æ' => out.push_str("ae"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(source: SignalSource) -> MatchRule {
        MatchRule {
            signal: source,
            contains: None,
            prefix: None,
            suffix: None,
            weight: 1,
        }
    }

    fn signal(source: SignalSource, value: &str) -> Signal {
        Signal {
            source,
            value: value.to_owned(),
        }
    }

    #[test]
    fn normalise_casse_accents_espaces() {
        assert_eq!(normalize("  I.U.T.   de Béziers "), "i.u.t. de beziers");
        assert_eq!(normalize("Œuvre"), "oeuvre");
    }

    #[test]
    fn accent_mal_encode_correspond() {
        let mut r = rule(SignalSource::AdeGroupPath);
        r.contains = Some("MMI 1ère année".into());
        assert!(r.matches(&signal(
            SignalSource::AdeGroupPath,
            "Iutb.MMI 1?re ann?e BUT.TDB."
        )));
        assert!(r.matches(&signal(
            SignalSource::AdeGroupPath,
            "Iutb.MMI 1\u{FFFD}re ann\u{FFFD}e"
        )));
    }

    #[test]
    fn prefixe_suffixe_et_source() {
        let mut r = rule(SignalSource::PostalAddress);
        r.suffix = Some("$beziers".into());
        assert!(r.matches(&signal(
            SignalSource::PostalAddress,
            "3 place du 14 juillet$34500$BEZIERS"
        )));
        assert!(!r.matches(&signal(SignalSource::RoomAddress, "x$BEZIERS")));

        let mut r = rule(SignalSource::AdeGroupPath);
        r.prefix = Some("Iutb".into());
        assert!(r.matches(&signal(
            SignalSource::AdeGroupPath,
            "Iutb.R&T 1ère année BUT"
        )));
        assert!(!r.matches(&signal(SignalSource::AdeGroupPath, "Droit › RENNES")));
    }

    #[test]
    fn score_additionne_les_regles() {
        let mut a = rule(SignalSource::AdeGroupPath);
        a.contains = Some("iut lannion".into());
        a.weight = 3;
        let mut b = rule(SignalSource::EnrollmentComponent);
        b.contains = Some("lannion".into());
        let aff = Affiliation { rules: vec![a, b] };
        let signals = [
            signal(SignalSource::AdeGroupPath, "IUT Lannion › BUT MMI"),
            signal(SignalSource::EnrollmentComponent, "IUT de Lannion"),
        ];
        assert_eq!(aff.score(&signals), 4);
        assert_eq!(aff.score(&signals[..1]), 3);
    }
}
