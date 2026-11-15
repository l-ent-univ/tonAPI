# Développer tonAPI

Le dépôt est un workspace Cargo (édition 2024) de sept crates. Avant chaque commit, cinq
vérifications doivent passer (format, clippy, tests, validation des universités, schémas à jour) ;
les tests ne contactent jamais une vraie université. Règles communes des contributions et des
agents : [../../AGENTS.md](../../AGENTS.md).

## Prérequis

- Rust stable avec `rustfmt` et `clippy`, installé automatiquement par `rust-toolchain.toml`.
  Version minimale : 1.88 (`rust-version` du `Cargo.toml`).
- Sous Linux, les fichiers de développement d'OpenSSL pour `native-tls` [déduit : dépendance de
  `reqwest` avec `native-tls`, paquet exact selon la distribution].
- Python 3 pour servir la console en local (facultatif).

## Lancer

```bash
TONAPI_ENV=dev cargo run -p tonapi-server
```

L'API écoute sur `127.0.0.1:8080`, la documentation interactive est sur `/docs`. Avec
`TONAPI_ENV=dev`, une clé de session temporaire est créée (les jetons ne survivent pas au
redémarrage) et `universities/` est rechargé à chaque modification. Toutes les variables :
[serveur.md](../exploitation/serveur.md#variables-denvironnement).

## Vérifications avant commit

Depuis la racine, d'après [../../AGENTS.md](../../AGENTS.md) :

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -q -p tonapi-cli -- validate universities
cargo run -q -p tonapi-cli -- schema --check
cargo run -q -p tonapi-cli -- report check docs/rapports/*.md   # quand un rapport change
```

La CI (`.github/workflows/ci.yml`) lance les six, avec `--locked`. Sans accès au registre Cargo,
`--offline` fonctionne si les dépendances sont en cache. Un contrôle qui n'a pas pu tourner est
signalé avec sa cause, jamais déclaré réussi.

Le lint `unsafe_code` est interdit et `clippy::pedantic` est actif. Les dépendances internes sont
déclarées dans `[workspace.dependencies]` puis utilisées avec `xxx.workspace = true`.

## Tests

| Où | Quoi |
|---|---|
| `#[cfg(test)]` dans chaque module | analyse pure : cas normal, champ manquant, valeur non numérique, fuseau et changement d'heure |
| `crates/tonapi-providers/tests/<logiciel>_mock.rs` | fournisseur de bout en bout contre un faux serveur axum sur `127.0.0.1:0` |
| `crates/tonapi-providers/tests/detection.rs` | rattachement à une composante |
| `crates/tonapi-http/tests/cas_mock.rs` | parcours CAS et second facteur |
| `crates/tonapi-server/tests/api.rs` | routes, erreurs, limites |
| `crates/tonapi-config/tests/config.rs` | chargement et refus (`tests/fixtures/invalid/`) |

Règles :

- fixtures synthétiques uniquement (`Camille Demo`, `e00000000000`), construites d'après la
  documentation, jamais copiées d'un vrai compte ;
- aucun test n'envoie un identifiant, même fictif, à une université réelle ;
- les faux serveurs parlent HTTP : le test passe `NetSettings { https_only: false }`.

Un test réussi contre un faux serveur ne prouve pas le comportement d'un déploiement réel ; les
essais réels sont tenus à part ([couverture.md](../reference/couverture.md)).

## Ajouter un fournisseur

Contrat et forme d'un module : [../../crates/tonapi-providers/README.md](../../crates/tonapi-providers/README.md).
Étapes et catalogue : [fournisseurs.md](../reference/fournisseurs.md#ajouter-un-fournisseur). Sources de formats
existantes : [../recherche/](../recherche/README.md).

## Configuration et schémas

Après un changement des types de `crates/tonapi-config/src/` :

```bash
cargo run -q -p tonapi-cli -- schema          # réécrit schema/*.schema.json
cargo run -q -p tonapi-cli -- schema --check  # échoue si schema/ n'est plus à jour
```

Le dossier `schema/` est généré et fait partie du changement. Format des fichiers TOML :
[configuration.md](configuration.md).

## Console

`console/` est une page statique pour essayer l'API avec un compte réel : connexion et second
facteur, appel de chaque service avec fournisseur et durée, couverture, fiches des fournisseurs,
explorateur OpenAPI, codes d'erreur.

```bash
TONAPI_ENV=dev cargo run -p tonapi-server
python3 -m http.server 5173 --bind 127.0.0.1 -d console
```

Puis ouvrir <http://127.0.0.1:5173>. La console n'envoie identifiants et jetons qu'à une API en
HTTPS ou sur `127.0.0.1`/`localhost`. Les comptes mémorisés gardent leur jeton chiffré dans le
`localStorage` du navigateur, jamais le mot de passe. `console/donnees.js` (fournisseurs, essais,
erreurs) est tenu à la main : le mettre à jour avec [fournisseurs.md](../reference/fournisseurs.md) et
[couverture.md](../reference/couverture.md).

## Rapports d'établissement

Les rapports `docs/rapports/<id>.md` suivent `docs/rapports/_modele.md` et passent
`cargo run -q -p tonapi-cli -- report check docs/rapports/*.md`. Ce contrôle détecte certains motifs
sensibles ; il ne remplace pas une relecture.

## Commits

Conventional Commits en français (`feat(config): …`, `fix(server): …`, `docs: …`), avec
l'utilisateur comme seul auteur. Détails : [../../AGENTS.md](../../AGENTS.md#git-et-fin-de-tâche).
