# CLAUDE.md

API Rust (workspace Cargo, edition 2024). Langue du projet : français (code commenté, docs, messages d'erreur).

## Commandes

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -q -p tonapi-cli -- validate            # fichiers universities/
cargo run -q -p tonapi-cli -- schema --check      # schema/ à jour
cargo run -q -p tonapi-cli -- providers           # fournisseurs et paramètres
cargo run -q -p tonapi-cli -- report check reports/*.md
cargo run -p tonapi-server                        # http://127.0.0.1:8080/docs
```

Les cinq premières doivent passer avant chaque commit ; la CI (`.github/workflows/ci.yml`) les rejoue.

## Arborescence

| Crate | Rôle |
|---|---|
| `tonapi-core` | modèles de données, erreurs `problem+json` (`ErrorCode`) |
| `tonapi-config` | types TOML, chargement, validation, JSON Schema, modèles `tonapi new` |
| `tonapi-session` | jeton de session |
| `tonapi-http` | client HTTP, cookies, formulaires |
| `tonapi-providers` | un module par logiciel ; contrat dans `crates/tonapi-providers/README.md` |
| `tonapi-server` | routes axum `/v1`, OpenAPI (utoipa), page `/docs` |
| `tonapi-cli` | binaire `tonapi` |

Autres dossiers : `universities/` (un dossier par université, `establishments/` pour les composantes), `schema/` (généré par `tonapi schema`), `Docs/` (recherche par établissement), `prompts/`, `reports/`.

## Conventions

- Lints : `clippy::pedantic` en avertissement, `unsafe_code` interdit (voir `Cargo.toml`).
- Dépendances internes déclarées dans `[workspace.dependencies]`, puis `xxx.workspace = true`.
- Après un changement de types dans `tonapi-config` : `cargo run -q -p tonapi-cli -- schema` et committer `schema/`.
- Tests sans réseau : faux serveurs axum sur `127.0.0.1:0`.
- Un fournisseur porte le nom d'un logiciel, jamais d'une université.

## Commits

- Conventional Commits en français : `feat(config): …`, `fix(server): …`, `docs: …`.
- Auteur : l'utilisateur seul. **Aucune ligne `Co-Authored-By`** ni mention de Claude.
- Ne pas pousser sans demande explicite.
