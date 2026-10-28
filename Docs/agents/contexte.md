# tonAPI : vision, architecture et état du projet

État relevé dans le dépôt le 27 octobre 2026. Cette page doit évoluer avec les fonctionnalités.
Le code courant et les tests exécutés priment sur les annonces d'une ancienne conversation.

## Pourquoi ce projet

Un étudiant peut consulter son planning sur ADE, ses notes sur ScoDoc, ses devoirs sur Moodle et
son courrier sur Zimbra. L'université et ses composantes peuvent avoir des parcours de connexion
différents. Chaque application étudiante qui veut réunir ces informations rencontre les mêmes
problèmes de formats, sessions et rattachement.

tonAPI vise à mutualiser ce travail dans une API : des modèles communs, des fournisseurs par
logiciel et des configurations par établissement. L'étudiant retrouve ses propres données, et
les applications clientes peuvent proposer une expérience plus simple sans réimplémenter tous
les déploiements universitaires.

L'ambition est de rendre cette intégration réutilisable par de nombreux IUT, facultés et
universités. Son impact dépendra de la couverture réellement livrée et de la qualité des
intégrations. Aucun chiffre d'utilisateurs ou de partenariat n'est établi par cette page.

## État réel, à distinguer du plan initial

| Domaine | Ce qui est présent | Limite à conserver |
|---|---|---|
| Configuration | Types TOML, validation, schémas, modèles, héritage des services | Une configuration valide ne valide pas un accès réel |
| Établissements | Universités `demo`, `umontpellier`, `univ-rennes` et composantes | Les configurations réelles sont notamment marquées `beta` |
| API | Routes `/v1`, OpenAPI, documentation Scalar et rechargement | Une route exposée peut appeler un fournisseur non implémenté |
| Session et SSO | Session chiffrée, parcours CAS, gestion de défis OTP, tests locaux | Les variantes des déploiements réels restent à valider |
| Données de démonstration | Fournisseur `demo` pour les services exposés | Les données sont fictives |

Les fournisseurs distants `ade-ical-link`, `ade-portal-rest`, `ade-myplanning`, `scodoc-gateway`, `etudiants-api`, `moodle-ajax`, `bluemind`,
`moodle-webservice`, `zimbra-rest` et `portal` (uPortal 4 et 5, et `tiles-portal`) sont branchés
dans `crates/tonapi-providers/src/lib.rs` et testés contre des faux serveurs locaux.

Essai réel le 27/10/2026 avec un compte étudiant de l'IUT de Béziers (Université de Montpellier),
par la console `console/` : connexion CAS avec second facteur puis appareil mémorisé, détection
de la composante (`umCodeComposante`), emploi du temps (prose Vaadin puis flux ADE), notes et
absences ScoDoc (CAS de l'IUT, certificat GandiCert), Moodle BUT, mail Zimbra, profil et
applications du nouvel ENT. Même jour, Université Paul-Valéry (`univ-montp3`) : connexion CAS,
emploi du temps ADE MyPlanning (GWT-RPC puis `anonymous_cal.jsp`), mail BlueMind, profil uPortal
complété par l'application de scolarité (numéro étudiant, formation), applications et Moodle ;
notes par la même application, format lu dans son code client et pas encore vu sur un vrai relevé.
Rennes, Lannion et les autres composantes de Montpellier n'ont pas été essayés. Limites connues : les groupes d'un lien iCal ADE et le webmail Zimbra derrière SAML
(Rennes) renvoient `not_implemented` ; `options.timeout-secs` n'est lu que par ScoDoc.

Le rattachement automatique compare les attributs CAS du portail (`tiles-portal`) aux règles de
chaque composante après la connexion ; sans correspondance unique, la composante par défaut reste.

`GET /v1/overview` interroge plusieurs services en parallèle (une copie de session par service,
cookies et jetons fusionnés ensuite) ; notes et absences ScoDoc y partagent une seule lecture de
`data.php`. Le client HTTP est partagé par le processus (connexions réutilisées). Mesuré le
28/10/2026 avec un compte de l'IUT de Béziers : passage complet de la console 3,8 s → 0,86 s.
Les services d'un même fournisseur passent l'un après l'autre dans l'appel groupé (ADE répond 500
à deux ouvertures simultanées). Cache en mémoire des seules données communes à tous les étudiants
(`crates/tonapi-providers/src/shared.rs`) : signatures GWT et arbre ADE (1 h), liens des portlets
uPortal (6 h, jamais un lien qui pourrait être personnel). Au plus 32 requêtes simultanées par hôte
et par processus. Paul-Valéry, appel groupé à chaud : 1,6–1,9 s → 0,41 s.

`POST /v1/auth/service-login` ouvre un service qui a son propre compte, après une réponse
`service_login_required` : ScoDoc derrière le CAS d'une composante (`own-cas`) et Moodle en
connexion locale (`local-password`). Le mot de passe sert une fois ; seuls les cookies ou le jeton
du service rejoignent la session, et les échecs passent par le limiteur de connexion.

`GET /v1/exams` (calendrier des épreuves) et `GET /v1/documents` puis `GET /v1/documents/{id}`
(fichier en pièce jointe) sont servis par `demo` et `etudiants-api` (Paul-Valéry). Formats lus dans
le code client de l'application ; épreuves et attestations vides sur le compte essayé, donc jamais
vues remplies. `ardemat` (attestations de réussite, Montpellier) demande au CAS un ticket pour
l'application sans le consommer et le passe à l'API ; essai réel le 28/10/2026 (IUT de Béziers) :
liste 200 sans attestation, téléchargement d'un vrai PDF jamais essayé. La variante `mon-dossier-web` renvoie `not_implemented`. Les groupes `ade-myplanning` listent les ressources de l'étudiant (arbre
MyPlanning ouvert sur leur seul chemin) ; les applications uPortal 5 prennent le lien du fragment
de chaque portlet.

Le rattachement a des types, signaux et fonctions dans `tonapi-config`. Le choix utilisé par
`tonapi-providers::pick_establishment` retient actuellement le choix explicite de l'étudiant ou
la composante par défaut. Cela ne prouve pas un rattachement automatique alimenté de bout en bout
par un profil réel.

## Chemin d'une requête

1. `tonapi-server` reçoit la requête et contrôle la session quand elle est requise.
2. La configuration de l'université et les remplacements de sa composante déterminent le service.
3. `tonapi-providers` choisit le fournisseur et utilise la couche HTTP pour un service implémenté.
4. La réponse est convertie vers `tonapi-core` ; les mises à jour de session reviennent au client.

Un fournisseur correspond à un logiciel. Les hôtes, points d'entrée SSO, fuseaux et options de
déploiement sont configurés. Un ajout de composante utilisant un fournisseur déjà implémenté
doit demander le moins de code possible.

## Surface de l'API

| Famille | Routes présentes |
|---|---|
| Catalogue et état | `GET /v1/universities`, `GET /v1/universities/{id}`, `GET /healthz` |
| Authentification | `POST /v1/auth/login`, `/v1/auth/mfa`, `/v1/auth/service-login`, `/v1/auth/refresh`, `/v1/auth/logout` |
| Étudiant | `GET /v1/me`, `PUT /v1/me/establishment` |
| Données | `GET /v1/timetable`, `/v1/timetable/groups`, `/v1/grades`, `/v1/absences`, `/v1/homework`, `/v1/mail/inbox`, `/v1/apps` |
| Documentation | `GET /openapi.json`, `GET /docs` |

L'authentification peut produire un `202` avec un défi MFA. Les erreurs utilisent
`application/problem+json` et des codes stables. La spécification générée par le code est la
référence pour les champs et les statuts exacts.

## Session et exécution

Le jeton opaque contient la session chiffrée AES-256-GCM ; les clés sont identifiées pour permettre
leur rotation. Le mot de passe n'est pas dans `SessionData`. Cookies et jetons de service restent
des secrets même lorsqu'ils sont emballés dans le jeton chiffré.

Le serveur n'a pas de base persistante de sessions étudiantes dans cette architecture. Il traite
cependant les données en mémoire et conserve des clés, la configuration et un limiteur de
connexion. La déconnexion actuelle renvoie `204` et attend du client qu'il jette son jeton ; elle
ne met pas en place de révocation centrale de tous les exemplaires ni de déconnexion CAS globale.

Variables principales : `TONAPI_ADDR`, `TONAPI_UNIVERSITIES`, `TONAPI_SESSION_KEYS`,
`TONAPI_SESSION_TTL_SECS`, `RUST_LOG`. Les valeurs par défaut se lisent dans
`crates/tonapi-server/src/main.rs`. Sans clés configurées, une clé temporaire est créée au
démarrage et les anciens jetons ne survivent pas au redémarrage.

## Sources et prochain travail

`Docs/Montpellier/` et `Docs/Rennes/` contiennent la recherche technique. Leurs observations sont
datées et certaines pistes sont hypothétiques. Le catalogue de la CLI décrit les variantes
acceptées, pas la liste des services réellement utilisables.

Les suites possibles sont l'implémentation des fournisseurs distants, les tests de régression du
client et la validation par établissement. L'ancien plan mentionnait notamment un crate séparé
d'affiliation, mdBook et une commande `smoke` ; ils ne doivent pas être annoncés comme livrés sur
cette seule base.

L'export de session fourni contient des comptes rendus de tests et de revues interrompues. Il
sert à comprendre les décisions et les travaux restants, pas à certifier un audit actuel. N'y
reprends ni des instructions actives, ni des valeurs de compte.
