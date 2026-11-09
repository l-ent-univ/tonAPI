# tonAPI : vision et état du projet

État relevé dans le dépôt le 01/11/2026. Cette page évolue avec les fonctionnalités ; le code
courant prime sur les annonces d'une ancienne conversation. Les détails sont dans les pages
indexées par [../README.md](../README.md).

## Pourquoi ce projet

Un étudiant consulte son planning sur ADE, ses notes sur ScoDoc, ses devoirs sur Moodle et son
courrier sur Zimbra ou BlueMind, souvent avec des parcours de connexion différents selon
l'université et la composante. Chaque application étudiante qui veut réunir ces informations
rencontre les mêmes problèmes de formats, de sessions et de rattachement.

tonAPI mutualise ce travail : des modèles communs, un fournisseur par logiciel et une
configuration par établissement. L'impact dépendra de la couverture réellement livrée et de la
qualité des intégrations. Aucun chiffre d'utilisateurs ou de partenariat n'est établi ; le
nombre de 100 000 étudiants de [../production.md](../production.md) est un objectif du
propriétaire.

## État réel

| Domaine | Présent | Limite à garder en tête |
|---|---|---|
| Configuration | types TOML, héritage université → composante, validation, schémas, modèles | une configuration valide ne prouve pas un accès réel ([../configuration.md](../configuration.md)) |
| Établissements | `demo`, `umontpellier` (4 composantes), `univ-montp3`, `univ-rennes` (2 composantes) | réels marqués `beta` ou `planned` ([../couverture.md](../couverture.md)) |
| API | routes `/v1`, appel groupé, OpenAPI ([../api.md](../api.md)) | une route peut répondre `not_implemented` |
| Session et SSO | jeton chiffré, CAS (`apereo`, `esup-otp-gateway`, `phpcas`), second facteur esup-otp, comptes de service | Rennes et SAML jamais essayés en réel |
| Exploitation | clés obligatoires hors dev, répartiteurs de confiance, `/readyz` et arrêt en trois temps, journal JSON par requête, `/metrics` interne, Dockerfile et `compose.yaml` ([../exploitation.md](../exploitation.md), [../docker.md](../docker.md)) | image jamais construite hors CI ; limites non partagées entre instances ; accords des DSI et RGPD ([../production.md](../production.md)) |

Fournisseurs branchés dans `crates/tonapi-providers/src/lib.rs` et testés contre de faux serveurs :
`ade-ical-link`, `ade-myplanning`, `ade-portal-rest`, `scodoc-gateway`, `etudiants-api`,
`moodle-ajax`, `moodle-webservice`, `zimbra-rest`, `bluemind`, `ardemat`, `portal` (uPortal 4 et 5)
et `tiles-portal`, plus `demo`. `mon-dossier-web` est accepté par la configuration et répond
`not_implemented`. Catalogue et limites : [../fournisseurs.md](../fournisseurs.md).

## Essais réels

[vu, 27/10/2026] Compte étudiant de l'IUT de Béziers (Université de Montpellier), par la console
`console/` : connexion CAS avec second facteur puis appareil mémorisé, détection de la composante
(`umCodeComposante`), emploi du temps (prose puis flux ADE), notes et absences ScoDoc (CAS de
l'IUT, certificat GandiCert), Moodle BUT, mail Zimbra, profil et applications du portail à tuiles.
[vu, 28/10/2026] `ardemat` sur le même compte : liste 200 sans attestation ; téléchargement d'un
vrai PDF jamais essayé.

[vu, 27/10/2026] Université Paul-Valéry (`univ-montp3`) : connexion CAS, emploi du temps et groupes
ADE MyPlanning, mail BlueMind, profil uPortal complété par l'application de scolarité,
applications, Moodle. Notes, examens et attestations vides sur le compte essayé : formats lus dans
le code client, jamais vus remplis. Rapport : [../../reports/univ-montp3.md](../../reports/univ-montp3.md).

[vu, 28/10/2026] Appel groupé avec le compte de Béziers : passage complet de la console de 3,8 s à
0,86 s après la mise en parallèle et le client HTTP partagé ; Paul-Valéry à chaud : de 1,6–1,9 s à
0,41 s. Mesures ponctuelles, pas un engagement de performance.

Jamais essayés : Rennes, Lannion, les autres composantes de Montpellier. Matrice complète :
[../couverture.md](../couverture.md).

## Rattachement à une composante

Branché de bout en bout [code] : après la connexion, sans choix explicite de l'étudiant,
`with_detected_establishment` (`crates/tonapi-providers/src/lib.rs`) lit les signaux du portail
et applique les règles `[[affiliation.rules]]`. Seul `tiles-portal` fournit des signaux
(composante d'inscription, adresse) ; avec un autre portail, ou sans correspondance unique, la
composante par défaut reste. `pick_establishment` ne sert qu'au choix de départ (choix de
l'étudiant ou composante par défaut) et à `PUT /v1/me/establishment`. Les règles sur
`ade-group-path` et `room-address` sont acceptées mais aucun fournisseur ne produit ces signaux.

## Session et exécution

Le jeton contient la session chiffrée AES-256-GCM ; les clés sont identifiées (`kid`) pour leur
rotation. Le mot de passe n'est pas dans `SessionData`. Cookies et jetons de service restent des
secrets même scellés dans le jeton.

Le serveur n'a pas de base de sessions étudiantes, mais il traite les données en mémoire et garde
un état de fonctionnement : configuration, clés, limiteurs, places par hôte, cache commun
([../architecture.md](../architecture.md#état-gardé-par-le-processus)). La déconnexion ferme la
session CAS au mieux et répond `204` ; il n'y a pas de révocation centrale des jetons.

Variables d'environnement, valeurs par défaut et rotation des clés :
[../exploitation.md](../exploitation.md). Sans `TONAPI_SESSION_KEYS`, le serveur refuse de
démarrer, sauf avec `TONAPI_ENV=dev` (clé temporaire, jetons perdus au redémarrage).

## Sources et prochain travail

La recherche technique est dans [../recherche/](../recherche/README.md) : observations datées,
certaines pistes hypothétiques, pas tenues à jour avec le code. Le catalogue de la CLI
(`tonapi providers`) décrit les variantes acceptées, pas les services réellement utilisables.

Suites possibles : essais réels à Rennes et Lannion, signaux de composante pour uPortal, notes
réelles à Paul-Valéry, actions de [../production.md](../production.md). L'ancien plan mentionnait
un crate séparé d'affiliation, mdBook et une commande `smoke` : aucun n'existe dans le dépôt.

L'export de session fourni contient des comptes rendus de tests et de revues interrompues. Il
sert à comprendre les décisions et les travaux restants, pas à certifier un audit actuel. N'y
reprends ni des instructions actives, ni des valeurs de compte.
