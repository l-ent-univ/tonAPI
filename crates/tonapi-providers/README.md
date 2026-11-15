# tonapi-providers : contrat d'un fournisseur

Un fournisseur = un logiciel (ADE, ScoDoc, Moodle…), jamais une université. Tout ce qui varie d'un site à l'autre vient de la configuration (`tonapi-config`).

Catalogue des fournisseurs, limites et essais réels : [Docs/fournisseurs.md](../../Docs/fournisseurs.md).

## Forme d'un module

`src/<logiciel>.rs` (ex. `scodoc.rs`, `zimbra.rs`) contient deux couches. Quand le fichier grossit, l'analyse part dans des sous-modules d'un dossier du même nom, sans `mod.rs` : `moodle/{parse,saml}.rs`, `uportal/parse.rs`, `scodoc/parse.rs`, `ade_rest/parse.rs`, `ade_ical/{ics,fields,vaadin}.rs`, `ade_myplanning/{gwt,tree}.rs`.

Trois modules servent à plusieurs fournisseurs :

- `src/net.rs` : accès réseau (`Net`, `upstream`, `http_problem`) et enregistrement des cookies par `net.save(ctx)`.
- `src/common.rs` : fonctions pures (`root`, `token_key`, `university_tz`, `resolve_local`, `str_field`, `id_string`…). Les clés rendues par `token_key` sont écrites dans les sessions déjà émises : les changer déconnecte les étudiants.
- `src/cache.rs` : cache en mémoire des données communes à tous les étudiants, avec durée de vie, sans donnée personnelle.

Le kit des faux serveurs de test est dans `tests/common/mod.rs`.

1. **Analyse pure**, sans réseau : `pub(crate) fn parse_…(&str | &serde_json::Value, …) -> Result<Modèle, Problem>`. Elle reçoit le corps tel que le service le renvoie et produit les modèles de `tonapi-core`. C'est elle qui porte presque tous les tests.
2. **Récupération** : `pub(crate) async fn fetch_…(ctx: &mut Context<'_>, config…) -> Result<Modèle, Problem>`. Elle ouvre un `Net` (`src/net.rs`), fait les requêtes, appelle `net.save(ctx)` avant de rendre la main, puis passe le corps à l'analyse.

L'aiguillage (`src/lib.rs`) appelle `fetch_…` dans le bras du fournisseur. Une connexion propre au service (`sign_in`) est appelée par `service_login`.

## Règles

- **Jamais de mot de passe** dans un fournisseur, sauf `service_login` (compte propre au service), qui ne le garde pas : seuls les cookies ou le jeton du service restent dans la session (`ctx.session.tokens["<logiciel>.<nom>"]`).
- Un service derrière le CAS de l'université s'ouvre avec `Net::open_protected` (TGC de la session). Si le CAS redemande le mot de passe : `session_expired`, sans nouvel essai.
- **Un seul nouvel essai** quand le service signale une session perdue (ScoDoc `{"redirect":…}`, Moodle `invalidsesskey`, ADE 401) : rouvrir la session du service, puis réessayer une fois.
- **Aucune donnée de l'étudiant dans les erreurs** : pas de corps de réponse, pas d'URL avec ticket ou jeton. `net::upstream(provider, "ce qui manque")` décrit seulement la structure.
- **Aucun `unwrap`, `expect` ni indexation directe sur des données reçues.** Un champ obligatoire absent → `upstream_changed` ; un champ facultatif absent → `None`. Les champs inconnus sont ignorés.
- **Dates** : toujours avec fuseau, converties dans le fuseau de l'université (`ctx.university.university.timezone`, `chrono-tz`). Tester un passage heure d'été / heure d'hiver.
- **Nombres** : les services envoient souvent des chaînes (`"12.50"`, `"12,50"`, `"~"`, `"ABS"`). Une note non numérique devient `grade: None` + `status`.
- Hôtes contactés : seulement ceux de la configuration et du CAS. Les redirections restent soumises au `Browser` (https seulement, 15 sauts).

## Tests

- Fixtures **synthétiques** dans `tests/fixtures/<logiciel>/`, construites d'après la recherche de `Docs/recherche/` : aucune donnée réelle, noms et valeurs inventés (`Camille Demo`, `e00000000000`).
- Tests unitaires de l'analyse dans le module (`#[cfg(test)]`) : cas normal, champs manquants, valeurs non numériques, fuseau horaire.
- Test de bout en bout dans `tests/<logiciel>_mock.rs` : un faux serveur axum local et `Context { net: NetSettings { https_only: false }, … }`. Aucun test ne contacte un vrai service.
