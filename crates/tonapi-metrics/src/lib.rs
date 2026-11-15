//! Métriques du processus, servies au format texte de Prometheus par `tonapi-server` sur un port
//! interne (`TONAPI_METRICS_ADDR`).
//!
//! Les noms sont déclarés une fois dans `METRICS` ; une étiquette ne porte jamais d'identifiant
//! d'étudiant, d'adresse IP, de jeton ni d'URL complète : route modèle, statut, code d'erreur,
//! université (identifiant de configuration), limiteur, résultat ou hôte contacté. Au-delà de
//! `MAX_SERIES` séries par métrique, les nouvelles combinaisons sont comptées dans
//! `tonapi_metrics_dropped_total` au lieu de faire grossir la mémoire.
//!
//! Trois natures : compteur ([`inc`]), jauge ([`set`], [`add`]) et histogramme ([`observe`]).
//! [`render`] copie l'état sous verrou puis formate hors verrou : un scrape ne retarde pas les
//! requêtes. Sous Linux, la mémoire résidente et les descripteurs de fichiers du processus sont
//! lus dans `/proc/self` à chaque scrape.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

/// Séries au plus par métrique.
pub(crate) const MAX_SERIES: usize = 1000;

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
pub(crate) enum Kind {
    Counter,
    /// Valeur qui monte et descend (requêtes en cours, entrées d'une table…).
    Gauge,
    /// Histogramme avec ses bornes supérieures, croissantes.
    Histogram(&'static [f64]),
}

/// Déclaration d'une métrique.
#[derive(Debug)]
pub(crate) struct Metric {
    pub(crate) name: &'static str,
    pub(crate) help: &'static str,
    pub(crate) kind: Kind,
}

/// Toutes les métriques de tonAPI.
pub(crate) const METRICS: &[Metric] = &[
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
        name: "tonapi_rate_limited_total",
        help: "Demandes refusées par un limiteur (ip : connexions par adresse ; login : échecs par compte ; slots : requêtes simultanées par compte).",
        kind: Kind::Counter,
    },
    Metric {
        name: "tonapi_login_total",
        help: "Demandes de connexion à une université, par résultat (success, second_factor ou code d'erreur stable).",
        kind: Kind::Counter,
    },
    Metric {
        name: "tonapi_config_reloads_total",
        help: "Rechargements de la configuration des universités, par résultat (ok, rejected, interrupted).",
        kind: Kind::Counter,
    },
    Metric {
        name: "tonapi_session_renewed_total",
        help: "Réponses qui renvoient un jeton mis à jour dans l'en-tête TonAPI-Session.",
        kind: Kind::Counter,
    },
    Metric {
        name: "tonapi_http_requests_in_flight",
        help: "Requêtes HTTP en cours de traitement.",
        kind: Kind::Gauge,
    },
    Metric {
        name: "tonapi_limiter_entries",
        help: "Entrées suivies par chaque limiteur (ip : adresses ; login : comptes ; slots : comptes avec requêtes simultanées).",
        kind: Kind::Gauge,
    },
    Metric {
        name: "tonapi_universities_loaded",
        help: "Universités actuellement chargées depuis la configuration.",
        kind: Kind::Gauge,
    },
    Metric {
        name: "process_resident_memory_bytes",
        help: "Mémoire résidente du processus, en octets (Linux seulement).",
        kind: Kind::Gauge,
    },
    Metric {
        name: "process_open_fds",
        help: "Descripteurs de fichiers ouverts par le processus (Linux seulement).",
        kind: Kind::Gauge,
    },
    Metric {
        name: "process_max_fds",
        help: "Limite souple de descripteurs de fichiers du processus (Linux seulement).",
        kind: Kind::Gauge,
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

#[derive(Debug, Clone, Default)]
struct Store {
    counters: BTreeMap<&'static str, BTreeMap<Labels, u64>>,
    gauges: BTreeMap<&'static str, BTreeMap<Labels, i64>>,
    histograms: BTreeMap<&'static str, BTreeMap<Labels, Histogram>>,
}

static STORE: LazyLock<Mutex<Store>> = LazyLock::new(Mutex::default);

/// Verrou du magasin. Un verrou empoisonné (panique d'un autre fil) n'éteint pas les métriques :
/// chaque mise à jour est un incrément isolé, l'état reste utilisable.
fn lock() -> MutexGuard<'static, Store> {
    STORE.lock().unwrap_or_else(PoisonError::into_inner)
}

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
    let mut store = lock();
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

/// Fixe la valeur d'une jauge.
pub fn set(name: &'static str, pairs: &[(&'static str, &str)], value: i64) {
    update_gauge(name, pairs, |current| *current = value);
}

/// Ajoute `delta` (négatif pour retirer) à une jauge, qui part de 0.
pub fn add(name: &'static str, pairs: &[(&'static str, &str)], delta: i64) {
    update_gauge(name, pairs, |current| {
        *current = current.saturating_add(delta);
    });
}

fn update_gauge(name: &'static str, pairs: &[(&'static str, &str)], apply: impl FnOnce(&mut i64)) {
    let Some(Metric {
        kind: Kind::Gauge, ..
    }) = find(name)
    else {
        debug_assert!(false, "jauge non déclarée : {name}");
        return;
    };
    let mut store = lock();
    let series = store.gauges.entry(name).or_default();
    let key = labels(pairs);
    if let Some(value) = series.get_mut(&key) {
        apply(value);
    } else if series.len() < MAX_SERIES {
        let mut value = 0;
        apply(&mut value);
        series.insert(key, value);
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
    let mut store = lock();
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
///
/// L'état est copié sous verrou, puis formaté hors verrou : un scrape ne bloque ni `inc`, ni
/// `set`, ni `add`, ni `observe`.
#[must_use]
pub fn render() -> String {
    let mut snapshot = lock().clone();
    for (name, value) in process_samples() {
        snapshot
            .gauges
            .entry(name)
            .or_default()
            .insert(Vec::new(), value);
    }
    format_store(&snapshot)
}

fn format_store(store: &Store) -> String {
    let mut out = String::new();
    for metric in METRICS {
        let kind = match metric.kind {
            Kind::Counter => "counter",
            Kind::Gauge => "gauge",
            Kind::Histogram(_) => "histogram",
        };
        let _ = writeln!(out, "# HELP {} {}", metric.name, metric.help);
        let _ = writeln!(out, "# TYPE {} {kind}", metric.name);
        match metric.kind {
            Kind::Counter => {
                for (labels, value) in store.counters.get(metric.name).into_iter().flatten() {
                    let _ = writeln!(
                        out,
                        "{}{} {value}",
                        metric.name,
                        format_labels(labels, None)
                    );
                }
            }
            Kind::Gauge => {
                for (labels, value) in store.gauges.get(metric.name).into_iter().flatten() {
                    let _ = writeln!(
                        out,
                        "{}{} {value}",
                        metric.name,
                        format_labels(labels, None)
                    );
                }
            }
            Kind::Histogram(bounds) => {
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

/// Mesures du processus lues dans `/proc/self` : mémoire résidente, descripteurs ouverts et leur
/// limite. Une lecture qui échoue omet simplement la mesure.
#[cfg(target_os = "linux")]
fn process_samples() -> Vec<(&'static str, i64)> {
    let mut samples = Vec::new();
    if let Some(bytes) = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| parse_resident_bytes(&text))
    {
        samples.push(("process_resident_memory_bytes", bytes));
    }
    if let Ok(entries) = std::fs::read_dir("/proc/self/fd") {
        // Le répertoire lu occupe lui-même un descripteur pendant le décompte.
        let open = entries.count().saturating_sub(1);
        samples.push(("process_open_fds", i64::try_from(open).unwrap_or(i64::MAX)));
    }
    if let Some(limit) = std::fs::read_to_string("/proc/self/limits")
        .ok()
        .and_then(|text| parse_max_fds(&text))
    {
        samples.push(("process_max_fds", limit));
    }
    samples
}

/// Hors Linux, pas de `/proc` : les métriques `process_*` restent déclarées mais sans valeur.
#[cfg(not(target_os = "linux"))]
fn process_samples() -> Vec<(&'static str, i64)> {
    Vec::new()
}

/// Mémoire résidente en octets, d'après `/proc/self/status` (ligne `VmRSS:  1234 kB`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_resident_bytes(status: &str) -> Option<i64> {
    let kib: i64 = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    kib.checked_mul(1024)
}

/// Limite souple de descripteurs, d'après `/proc/self/limits` ; `None` si elle est illimitée.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_max_fds(limits: &str) -> Option<i64> {
    limits
        .lines()
        .find_map(|line| line.strip_prefix("Max open files"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
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
    fn chaque_metrique_a_une_aide_en_francais_et_un_type() {
        let text = render();
        for metric in METRICS {
            assert!(!metric.help.trim().is_empty(), "{}", metric.name);
            assert!(
                text.contains(&format!("# HELP {} {}\n", metric.name, metric.help)),
                "{}",
                metric.name
            );
            let kind = match metric.kind {
                Kind::Counter => "counter",
                Kind::Gauge => "gauge",
                Kind::Histogram(_) => "histogram",
            };
            assert!(
                text.contains(&format!("# TYPE {} {kind}\n", metric.name)),
                "{}",
                metric.name
            );
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
    fn jauge_fixee_puis_ajustee() {
        set("tonapi_limiter_entries", &[("limiter", "test_jauge")], 5);
        add("tonapi_limiter_entries", &[("limiter", "test_jauge")], 3);
        add("tonapi_limiter_entries", &[("limiter", "test_jauge")], -2);
        add(
            "tonapi_limiter_entries",
            &[("limiter", "test_jauge_neuve")],
            -1,
        );
        let text = render();
        assert!(text.contains("# TYPE tonapi_limiter_entries gauge"));
        assert!(
            text.contains("tonapi_limiter_entries{limiter=\"test_jauge\"} 6"),
            "{text}"
        );
        assert!(
            text.contains("tonapi_limiter_entries{limiter=\"test_jauge_neuve\"} -1"),
            "{text}"
        );
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
    fn verrou_empoisonne_ne_perd_pas_les_metriques() {
        inc("tonapi_session_renewed_total", &[]);
        let result = std::thread::spawn(|| {
            let _guard = lock();
            panic!("panique volontaire du test, verrou tenu");
        })
        .join();
        assert!(result.is_err());
        assert!(STORE.is_poisoned());
        inc("tonapi_session_renewed_total", &[]);
        let text = render();
        let value = text
            .lines()
            .find_map(|line| line.strip_prefix("tonapi_session_renewed_total "))
            .and_then(|v| v.parse::<u64>().ok());
        assert!(value.is_some_and(|v| v >= 2), "{text}");
    }

    #[test]
    fn lecture_de_proc_self() {
        let status = "Name:\ttonapi\nVmRSS:\t   12345 kB\nThreads:\t4\n";
        assert_eq!(parse_resident_bytes(status), Some(12_345 * 1024));
        assert_eq!(parse_resident_bytes("Name:\ttonapi\n"), None);
        let limits = "Limit    Soft Limit    Hard Limit    Units\n\
                      Max open files            1024                 524288               files\n";
        assert_eq!(parse_max_fds(limits), Some(1024));
        let unlimited =
            "Max open files            unlimited            unlimited            files\n";
        assert_eq!(parse_max_fds(unlimited), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn processus_lu_a_chaque_scrape_sous_linux() {
        let text = render();
        let value = |name: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(&format!("{name} ")))
                .and_then(|v| v.parse::<i64>().ok())
        };
        assert!(value("process_resident_memory_bytes").is_some_and(|v| v > 0));
        assert!(value("process_open_fds").is_some_and(|v| v > 0));
    }

    #[test]
    fn classes_de_statut() {
        assert_eq!(status_class(204), "2xx");
        assert_eq!(status_class(302), "3xx");
        assert_eq!(status_class(429), "4xx");
        assert_eq!(status_class(503), "5xx");
    }
}
