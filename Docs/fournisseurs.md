# Fournisseurs

Un fournisseur traduit un logiciel universitaire (ADE, ScoDoc, Moodle…) vers les modèles communs
de tonAPI. Douze fournisseurs distants sont branchés dans le dispatch, plus `demo` ;
`mon-dossier-web` est accepté par la configuration mais répond `not_implemented`. Un essai réel ne
vaut que pour l'établissement où il a eu lieu, même si le logiciel est le même ailleurs.

État relevé le 01/11/2026 dans `crates/tonapi-providers/src/lib.rs` et `sso.rs` [code]. Essais réels
repris de [agents/contexte.md](agents/contexte.md) et de
[../reports/univ-montp3.md](../reports/univ-montp3.md) [vu, dates indiquées].

## Contrat

Le contrat détaillé est dans [../crates/tonapi-providers/README.md](../crates/tonapi-providers/README.md).
Points principaux :

- un module par logiciel, nommé d'après le logiciel, jamais d'après une université ;
- une analyse pure (sans réseau) testée sur des fixtures synthétiques, et une fonction de
  récupération qui passe par `Net` et enregistre les cookies avec `net.save(ctx)` ;
- pas de mot de passe hors `service_login`, qui ne le garde pas ;
- un seul nouvel essai quand le service signale une session perdue ;
- champ obligatoire absent : `upstream_changed` ; panne ou délai : `service_unavailable` ;
  fonction impossible : `not_implemented` ;
- aucune donnée de l'étudiant dans les messages d'erreur ;
- dates avec fuseau, converties dans celui de l'université ; notes non numériques jamais
  converties en zéro.

## Connexion (commune)

| Élément | Variantes | Essai réel |
|---|---|---|
| CAS de l'université (`[sso] kind = "cas"`) | `apereo`, `esup-otp-gateway`, `phpcas` | `esup-otp-gateway` avec second facteur puis appareil mémorisé : Université de Montpellier (compte de l'IUT de Béziers), 27/10/2026 [vu]. `apereo` sans second facteur : Paul-Valéry, 27/10/2026 [vu]. `phpcas` : CAS de l'IUT de Béziers pour ScoDoc, 27/10/2026 [vu] |
| Second facteur esup-otp (`[sso.mfa]`) | notification, code | Montpellier, 27/10/2026 [vu] ; Rennes jamais essayé |
| Fédération SAML (`[sso.saml]`) | Moodle `entry = "shibboleth"` | jamais essayée en réel |
| `demo` | identifiants `demo` / `demo` | sans service distant |

## Catalogue par logiciel

### Emploi du temps (ADE)

| Fournisseur | Parcours | Essai réel | Limites |
|---|---|---|---|
| `ade-ical-link` | page authentifiée qui contient le lien `direct_cal.jsp` (prose à Montpellier), puis flux iCal | Université de Montpellier (compte de l'IUT de Béziers), 27/10/2026 [vu] | `/v1/timetable/groups` répond `not_implemented` : le flux ne contient que les séances |
| `ade-myplanning` | appels GWT-RPC de MyPlanning pour les ressources de l'étudiant, puis `anonymous_cal.jsp` | Paul-Valéry, 27/10/2026, groupes compris [vu] | types GWT d'ADE 6.x ; groupes limités à ceux de l'étudiant |
| `ade-portal-rest` | API REST du portail ADE (`sso/accessToken`, `ssologin`, projets) | aucun : faux serveurs seulement [code] | format réel de la sélection personnelle [inconnu] |

### Notes et absences

| Fournisseur | Parcours | Essai réel | Limites |
|---|---|---|---|
| `scodoc-gateway` | Passerelle ScoDoc (`services/data.php`), par le CAS de l'université ou un CAS de composante (`own-cas`) | IUT de Béziers, CAS propre, 27/10/2026 [vu] | variante par le CAS de l'université jamais essayée ; second facteur d'un CAS de composante `not_implemented` ; `local-password` `not_implemented` |
| `etudiants-api` (notes) | application de scolarité, `api/etudiants/notes` | Paul-Valéry, 27/10/2026 : réponse sans semestre [vu] | format d'une note réelle [inconnu], lu dans le code client |

### Examens

| Fournisseur | Parcours | Essai réel | Limites |
|---|---|---|---|
| `etudiants-api` | `api/etudiants/calendrier` | Paul-Valéry, 27/10/2026 : liste vide [vu] | formats des dates et durées déduits du code client [déduit] |

### Devoirs (Moodle)

| Fournisseur | Parcours | Essai réel | Limites |
|---|---|---|---|
| `moodle-ajax` | entrée CAS (`authCAS=CAS`) ou Shibboleth, `sesskey` de la page, `lib/ajax/service.php` | Paul-Valéry, 27/10/2026 [vu] | entrée Shibboleth jamais essayée ; seuls les devoirs à faire |
| `moodle-webservice` | `login/token.php` avec le mot de passe local, puis web service REST | IUT de Béziers (Moodle BUT), 27/10/2026 [vu] | seulement `sign-in = local-password` : pas de jeton par le CAS (`not_implemented`) |

### Mail

| Fournisseur | Parcours | Essai réel | Limites |
|---|---|---|---|
| `zimbra-rest` | CAS puis préauthentification (`ZM_AUTH_TOKEN`), SOAP `SearchRequest` | Université de Montpellier (compte de l'IUT de Béziers), 27/10/2026 [vu] | webmail derrière une fédération SAML : `not_implemented` [code] ; 20 derniers messages |
| `bluemind` | CAS, clé de `webapp/session-infos.js`, API REST | Paul-Valéry, 27/10/2026 [vu] | boîte de réception, 20 derniers messages |

### Portail : applications, profil, composante

`apps` et `profile` utilisent `provider = "portal"` ; le logiciel vient de `[portal]`.

| Portail | Parcours | Essai réel | Limites |
|---|---|---|---|
| `tiles-portal` | client CAS du portail, `api/cas-attributes`, `api/tiles` | Université de Montpellier (compte de l'IUT de Béziers), 27/10/2026 : profil, applications, détection de la composante [vu] | logiciel non identifié ; seul portail qui fournit des signaux de composante |
| `uportal5` | `layout.json`, lien de chaque portlet dans son fragment, profil par `userinfo` ou `session.json` et `people` | Paul-Valéry, 27/10/2026, sans `userinfo` [vu] | uPortal 5 de Rennes jamais essayé ; portlets techniques présents dans la liste |
| `uportal4-prolongation` | `layout.jsp` (JSONP) | aucun : faux serveurs seulement ; l'ENT de Montpellier a changé depuis la recherche | |

Le profil peut être complété par l'application de scolarité
(`record = { kind = "etudiants-api", … }`) : numéro étudiant et formation, vus à Paul-Valéry le
27/10/2026.

### Documents

| Fournisseur | Parcours | Essai réel | Limites |
|---|---|---|---|
| `etudiants-api` | `api/etudiants/attestations`, fichier en PDF ; `certificate-url` ajouté comme lien | Paul-Valéry, 27/10/2026 : certificat en lien, aucune attestation [vu] | téléchargement d'une attestation jamais essayé |
| `ardemat` | ticket CAS demandé pour l'application sans le consommer, `api/etudiants/<ticket>`, `api/attestations/download` | IUT de Béziers, 28/10/2026 : liste 200 sans attestation [vu] | téléchargement d'un vrai PDF jamais essayé |
| `mon-dossier-web` | aucun | | `not_implemented` [code] |

### Démonstration

`demo` sert des données fictives pour chaque service de l'université `demo`.

## Options de service

`options.timeout-secs` n'est lu que par `scodoc-gateway` (défaut 20 s). `options.cache-ttl-secs`
n'est lu par aucun fournisseur. Les délais globaux des routes s'appliquent à tous
([api.md](api.md#limites)).

## Ajouter un fournisseur

1. Lire [../crates/tonapi-providers/README.md](../crates/tonapi-providers/README.md) et un module
   voisin.
2. Ajouter la variante dans `crates/tonapi-config/src/services.rs`, régénérer `schema/`.
3. Écrire l'analyse pure et ses tests, puis la récupération et un test de bout en bout contre un
   faux serveur (`crates/tonapi-providers/tests/<logiciel>_mock.rs`).
4. Brancher le bras dans `crates/tonapi-providers/src/lib.rs`.
5. Mettre à jour cette page, [couverture.md](couverture.md) et la fiche de la console
   (`console/donnees.js`).

Détails et vérifications : [developpement.md](developpement.md).
