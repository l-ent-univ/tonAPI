# tonAPI : vision et état du projet

Cette page évolue avec les fonctionnalités ; le code courant prime sur les annonces d'une
ancienne conversation. Les détails sont dans les pages indexées par [../README.md](../README.md).

## Pourquoi ce projet

Un étudiant consulte son planning sur ADE, ses notes sur ScoDoc, ses devoirs sur Moodle et son
courrier sur Zimbra ou BlueMind, souvent avec des parcours de connexion différents selon
l'université et la composante. Chaque application étudiante qui veut réunir ces informations
rencontre les mêmes problèmes de formats, de sessions et de rattachement.

tonAPI mutualise ce travail : des modèles communs, un fournisseur par logiciel et une
configuration par établissement. L'impact dépendra de la couverture réellement livrée et de la
qualité des intégrations. Aucun chiffre d'utilisateurs ou de partenariat n'est établi ; le
nombre de 100 000 étudiants de [../exploitation/production.md](../exploitation/production.md) est un objectif du
propriétaire.

## État réel

| Domaine | Présent | Limite à garder en tête |
|---|---|---|
| Configuration | types TOML, héritage université → composante, validation, schémas, modèles | une configuration valide ne prouve pas un accès réel ([../guide/configuration.md](../guide/configuration.md)) |
| Établissements | `demo`, `umontpellier` (4 composantes), `univ-montp3`, `univ-rennes` (2 composantes) | réels marqués `beta` ou `planned` ([../reference/couverture.md](../reference/couverture.md)) |
| API | routes `/v1`, appel groupé, OpenAPI ([../guide/api.md](../guide/api.md)) | une route peut répondre `not_implemented` |
| Session et SSO | jeton chiffré, CAS (`apereo`, `esup-otp-gateway`, `phpcas`), second facteur esup-otp, comptes de service | Rennes et SAML jamais essayés en réel |
| Exploitation | clés obligatoires hors dev, répartiteurs de confiance, `/readyz` et arrêt en trois temps, journal JSON par requête, `/metrics` interne, Dockerfile et `compose.yaml` ([../exploitation/serveur.md](../exploitation/serveur.md), [../exploitation/docker.md](../exploitation/docker.md)) | image jamais construite hors CI ; limites non partagées entre instances ; accords des DSI et RGPD ([../exploitation/production.md](../exploitation/production.md)) |

Fournisseurs branchés dans `crates/tonapi-providers/src/lib.rs` et testés contre de faux serveurs :
`ade-ical-link`, `ade-myplanning`, `ade-portal-rest`, `scodoc-gateway`, `etudiants-api`,
`moodle-ajax`, `moodle-webservice`, `zimbra-rest`, `bluemind`, `ardemat`, `portal` (uPortal 4 et 5)
et `tiles-portal`, plus `demo`. `mon-dossier-web` est accepté par la configuration et répond
`not_implemented`. Catalogue et limites : [../reference/fournisseurs.md](../reference/fournisseurs.md).

## Essais réels

Deux établissements ont été essayés avec un vrai compte : l'IUT de Béziers (Université de
Montpellier) et Paul-Valéry (`univ-montp3`). Service par service, avec la date et la preuve :
[../reference/couverture.md](../reference/couverture.md), qui fait foi. Rapport de Paul-Valéry :
[../rapports/univ-montp3.md](../rapports/univ-montp3.md).

[vu, 28/10/2026] Appel groupé avec le compte de Béziers : passage complet de la console de 3,8 s à
0,86 s après la mise en parallèle et le client HTTP partagé ; Paul-Valéry à chaud : de 1,6–1,9 s à
0,41 s. Mesures ponctuelles, pas un engagement de performance.

## Rattachement à une composante

Branché de bout en bout [code] : après la connexion, sans choix explicite de l'étudiant,
`with_detected_establishment` (`crates/tonapi-providers/src/lib.rs`) lit les signaux du portail
et applique les règles `[[affiliation.rules]]`. Deux fournisseurs donnent des signaux :
`tiles-portal` (composante d'inscription, adresse) et `ade-portal-rest` (chemin du groupe de
l'étudiant dans l'arbre ADE, lu seulement si une composante a une règle `ade-group-path`). Sans
signal, ou sans correspondance unique, la composante par défaut reste. `pick_establishment` ne sert qu'au choix de départ (choix de
l'étudiant ou composante par défaut) et à `PUT /v1/me/establishment`. Les règles sur
`room-address` sont acceptées mais aucun fournisseur ne produit ce signal.

## Session et exécution

Le jeton contient la session chiffrée AES-256-GCM ; les clés sont identifiées (`kid`) pour leur
rotation. Le mot de passe n'est pas dans `SessionData`. Cookies et jetons de service restent des
secrets même scellés dans le jeton.

Le serveur n'a pas de base de sessions étudiantes, mais il traite les données en mémoire et garde
un état de fonctionnement : configuration, clés, limiteurs, places par hôte, cache commun
([../reference/architecture.md](../reference/architecture.md#état-gardé-par-le-processus)). La déconnexion ferme la
session CAS au mieux et répond `204` ; il n'y a pas de révocation centrale des jetons.

Variables d'environnement, valeurs par défaut et rotation des clés :
[../exploitation/serveur.md](../exploitation/serveur.md). Sans `TONAPI_SESSION_KEYS`, le serveur refuse de
démarrer, sauf avec `TONAPI_ENV=dev` (clé temporaire, jetons perdus au redémarrage).

## Sources et prochain travail

La recherche technique est dans [../recherche/](../recherche/README.md) : observations datées,
certaines pistes hypothétiques, pas tenues à jour avec le code. Le catalogue de la CLI
(`tonapi providers`) décrit les variantes acceptées, pas les services réellement utilisables.

Suites possibles : essais réels à Rennes et Lannion, signaux de composante pour uPortal, notes
réelles à Paul-Valéry, actions de [../exploitation/production.md](../exploitation/production.md).
