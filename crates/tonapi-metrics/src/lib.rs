//! Métriques du processus, servies au format texte de Prometheus par `tonapi-server` sur un port
//! interne (`TONAPI_METRICS_ADDR`).
//!
//! Les noms sont déclarés une fois dans [`METRICS`] ; une étiquette ne porte jamais d'identifiant
//! d'étudiant, de jeton ni d'URL complète : route modèle, statut, code d'erreur, université
//! (identifiant de configuration) ou hôte contacté. Au-delà de [`MAX_SERIES`] séries par
//! métrique, les nouvelles combinaisons sont comptées dans `tonapi_metrics_dropped_total` au lieu
//! de faire grossir la mémoire.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{LazyLock, Mutex};

/// Séries au plus par métrique.
pub const MAX_SERIES: usize = 1000;

/// Durées d'une requête à tonAPI, en secondes (jusqu'au délai de 130 s du second facteur).
const REQUEST_SECONDS: &[f64] = &[
    0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 130.0,
];
/// Durées d'une requête vers une université, en secondes.
const UPSTREAM_SECONDS: &[f64] = &[0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0];
/// Attente d'une place vers un hôte, en secondes (abandon à 10 s).
const WAIT_SECONDS: &[f64] = &[0.001, 0.01, 0.1, 0.5, 1.0, 2.5, 5.0, 10.0];
/// Taille d'un jeton de session, en octets (les répartiteurs coupent souvent à 4, 8 ou 16 Ko).
const TOKEN_BYTES: &[f64] = &[
    1024.0, 2048.0, 4096.0, 8192.0, 12_288.0, 16_384.0, 24_576.0, 32_768.0, 65_536.0,
];

/// Nature d'une métrique.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Counter,
    /// Histogramme avec ses bornes supérieures, croissantes.
    Histogram(&'static [f64]),
}

/// Déclaration d'une métrique.
#[derive(Debug)]
pub struct Metric {
    pub name: &'static str,
    pub help: &'static str,
    pub kind: Kind,
}

/// Toutes les métriques de tonAPI.
pub const METRICS: &[Metric] = &[
    Metric {
        name: "tonapi_requests_total",
        help: "Requêtes reçues, par route modèle et statut HTTP.",
        kind: Kind::Counter,
    },
    Metric {
        name: "tonapi_request_duration_seconds",
        help: "Durée des requêtes reçues, par route modèle.",
        kind: Kind::Histogram(REQUEST_SECONDS),
    },
    Metric {
        name: "tonapi_errors_total",
        help: "Réponses d'erreur, par code stable et université.",
        kind: Kind::Counter,
    },
    Metric {
        name: "tonapi_upstream_requests_total",
        help: "Requêtes envoyées aux universités, par hôte et classe de statut (error : pas de réponse).",
        kind: Kind::Counter,
    },
    Metric {
        name: "tonapi_upstream_duration_seconds",
        help: "Durée des requêtes envoyées aux universités, par hôte, corps compris.",
        kind: Kind::Histogram(UPSTREAM_SECONDS),
    },
    Metric {
        name: "tonapi_upstream_wait_seconds",
        help: "Attente d'une place libre vers un hôte, par file (normal, long_poll).",
        kind: Kind::Histogram(WAIT_SECONDS),
    },
    Metric {
        name: "tonapi_upstream_saturated_total",
        help: "Requêtes abandonnées faute de place vers un hôte, par hôte et file.",
        kind: Kind::Counter,
    },
    Metric {
        name: "tonapi_session_token_bytes",
        help: "Taille des jetons de session émis, en octets.",
        kind: Kind::Histogram(TOKEN_BYTES),
    },
    Metric {
        name: "tonapi_metrics_dropped_total",
        help: "Observations ignorées : trop de séries pour une métrique.",
        kind: Kind::Counter,
    },
];

type Labels = Vec<(&'static str, String)>;

#[derive(Debug, Clone)]
struct Histogram {
    /// Observations dans chaque intervalle (non cumulées), plus une case au-delà de la dernière
    /// borne.
    buckets: Vec<u64>,
    sum: f64,
    count: u64,
}

#[derive(Debug, Default)]
struct Store {
    counters: BTreeMap<&'static str, BTreeMap<Labels, u64>>,
    histograms: BTreeMap<&'static str, BTreeMap<Labels, Histogram>>,
}

static STORE: LazyLock<Mutex<Store>> = LazyLock::new(Mutex::default);

fn find(name: &str) -> Option<&'static Metric> {
    METRICS.iter().find(|m| m.name == name)
}

fn labels(pairs: &[(&'static str, &str)]) -> Labels {
    pairs.iter().map(|(k, v)| (*k, (*v).to_owned())).collect()
}

/// Ajoute 1 à un compteur.
pub fn inc(name: &'static str, pairs: &[(&'static str, &str)]) {
    let Some(Metric {
        kind: Kind::Counter,
        ..
    }) = find(name)
    else {
        debug_assert!(false, "compteur non déclaré : {name}");
        return;
    };
    let Ok(mut store) = STORE.lock() else { return };
    let series = store.counters.entry(name).or_default();
    let key = labels(pairs);
    if let Some(value) = series.get_mut(&key) {
        *value += 1;
    } else if series.len() < MAX_SERIES {
        series.insert(key, 1);
    } else {
        dropped(&mut store, name);
    }
}

/// Ajoute une observation à un histogramme.
pub fn observe(name: &'static str, pairs: &[(&'static str, &str)], value: f64) {
    let Some(Metric {
        kind: Kind::Histogram(bounds),
        ..
    }) = find(name)
    else {
        debug_assert!(false, "histogramme non déclaré : {name}");
        return;
    };
    if !value.is_finite() {
        return;
    }
    let Ok(mut store) = STORE.lock() else { return };
    let series = store.histograms.entry(name).or_default();
    let key = labels(pairs);
    if !series.contains_key(&key) && series.len() >= MAX_SERIES {
        dropped(&mut store, name);
        return;
    }
    let histogram = series.entry(key).or_insert_with(|| Histogram {
        buckets: vec![0; bounds.len() + 1],
        sum: 0.0,
        count: 0,
    });
    let slot = bounds
        .iter()
        .position(|bound| value <= *bound)
        .unwrap_or(bounds.len());
    if let Some(bucket) = histogram.buckets.get_mut(slot) {
        *bucket += 1;
    }
    histogram.sum += value;
    histogram.count += 1;
}

fn dropped(store: &mut Store, name: &'static str) {
    *store
        .counters
        .entry("tonapi_metrics_dropped_total")
        .or_default()
        .entry(vec![("metric", name.to_owned())])
        .or_default() += 1;
}

/// Toutes les séries, au format texte de Prometheus (version 0.0.4).
#[must_use]
pub fn render() -> String {
    let Ok(store) = STORE.lock() else {
        return String::new();
    };
    let mut out = String::new();
    for metric in METRICS {
        match metric.kind {
            Kind::Counter => {
                let _ = writeln!(out, "# HELP {} {}", metric.name, metric.help);
                let _ = writeln!(out, "# TYPE {} counter", metric.name);
                for (labels, value) in store.counters.get(metric.name).into_iter().flatten() {
                    let _ = writeln!(
                        out,
                        "{}{} {value}",
                        metric.name,
                        format_labels(labels, None)
                    );
                }
            }
            Kind::Histogram(bounds) => {
                let _ = writeln!(out, "# HELP {} {}", metric.name, metric.help);
                let _ = writeln!(out, "# TYPE {} histogram", metric.name);
                for (labels, histogram) in store.histograms.get(metric.name).into_iter().flatten() {
                    let mut cumulative = 0;
                    for (bound, count) in bounds.iter().zip(&histogram.buckets) {
                        cumulative += count;
                        let le = format!("{bound}");
                        let _ = writeln!(
                            out,
                            "{}_bucket{} {cumulative}",
                            metric.name,
                            format_labels(labels, Some(&le))
                        );
                    }
                    let _ = writeln!(
                        out,
                        "{}_bucket{} {}",
                        metric.name,
                        format_labels(labels, Some("+Inf")),
                        histogram.count
                    );
                    let _ = writeln!(
                        out,
                        "{}_sum{} {}",
                        metric.name,
                        format_labels(labels, None),
                        histogram.sum
                    );
                    let _ = writeln!(
                        out,
                        "{}_count{} {}",
                        metric.name,
                        format_labels(labels, None),
                        histogram.count
                    );
                }
            }
        }
    }
    out
}

fn format_labels(labels: &Labels, le: Option<&str>) -> String {
    let mut parts: Vec<String> = labels
        .iter()
        .map(|(k, v)| format!("{k}=\"{}\"", escape(v)))
        .collect();
    if let Some(le) = le {
        parts.push(format!("le=\"{le}\""));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{{{}}}", parts.join(","))
    }
}

/// Échappement des valeurs d'étiquette : `\`, `"` et saut de ligne.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// Classe d'un statut HTTP pour une étiquette (`2xx`…).
#[must_use]
pub fn status_class(status: u16) -> &'static str {
    match status {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        _ => "5xx",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noms_uniques_et_bornes_croissantes() {
        for (i, metric) in METRICS.iter().enumerate() {
            assert!(
                METRICS[i + 1..].iter().all(|m| m.name != metric.name),
                "{}",
                metric.name
            );
            if let Kind::Histogram(bounds) = metric.kind {
                assert!(bounds.windows(2).all(|w| w[0] < w[1]), "{}", metric.name);
            }
        }
    }

    #[test]
    fn compteur_et_histogramme_au_format_prometheus() {
        inc(
            "tonapi_errors_total",
            &[("code", "test_rendu"), ("university", "a\"b")],
        );
        inc(
            "tonapi_errors_total",
            &[("code", "test_rendu"), ("university", "a\"b")],
        );
        observe("tonapi_session_token_bytes", &[], 3000.0);
        observe("tonapi_session_token_bytes", &[], 1e9);
        let text = render();
        assert!(
            text.contains("tonapi_errors_total{code=\"test_rendu\",university=\"a\\\"b\"} 2"),
            "{text}"
        );
        assert!(text.contains("# TYPE tonapi_session_token_bytes histogram"));
        assert!(text.contains("tonapi_session_token_bytes_bucket{le=\"2048\"} 0"));
        assert!(text.contains("tonapi_session_token_bytes_bucket{le=\"4096\"} 1"));
        assert!(text.contains("tonapi_session_token_bytes_bucket{le=\"+Inf\"} 2"));
        assert!(text.contains("tonapi_session_token_bytes_count 2"));
    }

    #[test]
    fn series_bornees() {
        for i in 0..=MAX_SERIES {
            inc(
                "tonapi_upstream_saturated_total",
                &[("host", &format!("h{i}.test")), ("lane", "normal")],
            );
        }
        let text = render();
        assert!(text.contains(
            "tonapi_metrics_dropped_total{metric=\"tonapi_upstream_saturated_total\"} 1"
        ));
        assert!(!text.contains(&format!("h{MAX_SERIES}.test")));
    }

    #[test]
    fn classes_de_statut() {
        assert_eq!(status_class(204), "2xx");
        assert_eq!(status_class(302), "3xx");
        assert_eq!(status_class(429), "4xx");
        assert_eq!(status_class(503), "5xx");
    }
}
